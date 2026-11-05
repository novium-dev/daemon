// SPDX-License-Identifier: AGPL-3.0-only
use crate::server::start::ServerStartError;
use crate::server::{FetchedServer, Server};
use crate::{Daemon, DaemonExtra, ServerData};
use bollard::Docker;
use bollard::query_parameters::{
    BuildImageOptions, ListImagesOptions, PruneImagesOptions, RemoveImageOptions,
};
use cap_async_std::async_std::stream::StreamExt as _;
use core::time::Duration;
use http_body_util::Full;
use indexmap::IndexMap;
use sqlx::{Executor, MySql, Pool, query, query_scalar};
use std::collections::{HashMap, HashSet};
use tar::{Builder, Header};
use termion::clear;
use tracing::{debug, info, warn};

const IMAGE_UNUSED_DAYS: u32 = 30;
const IMAGE_CLEANUP_INTERVAL: Duration = Duration::from_hours(1);

fn normalize_tag(tag: String) -> String {
    tag.strip_prefix("localhost/")
        .map_or_else(|| tag.strip_prefix("docker.io/library/"), Some)
        .map(String::from)
        .unwrap_or(tag)
}

#[derive(Debug, Clone, Default)]
pub struct BuildImage {
    pub tag: String,
    pub dockerfile: String,
    pub args: HashMap<String, String>,
    pub parents: Vec<Self>,
}

impl Server {
    pub async fn resolve_image(
        &self,
        conn: impl Executor<'_, Database = MySql> + Copy,
        id: u64,
        tags: &HashMap<u64, String>,
    ) -> Result<BuildImage, sqlx::Error> {
        let image = query!(
            "SELECT `dockerfile`,`default_tag` FROM `images` WHERE `id` = ?",
            id
        )
        .fetch_one(conn)
        .await?;

        let ((Some(tag), _) | (None, Some(tag))) = (tags.get(&id).cloned(), image.default_tag)
        else {
            return Err(sqlx::Error::RowNotFound);
        };
        let args: HashMap<String, String> = query!(
            "SELECT `key`,`value` FROM `server_image_args` WHERE `image` = ? AND `server` = ?",
            id,
            self.id
        )
        .fetch_all(conn)
        .await?
        .into_iter()
        .map(|row| (row.key, row.value))
        .collect();
        let parent_ids: HashSet<u64> =
            query_scalar!("SELECT `parent` FROM `image_parents` WHERE `image` = ?", id)
                .fetch_all(conn)
                .await?
                .into_iter()
                .collect();
        let mut parents = Vec::with_capacity(parent_ids.len());
        for id in parent_ids {
            parents.push(Box::pin(self.resolve_image(conn, id, tags)).await?);
        }

        Ok(BuildImage {
            tag,
            dockerfile: image.dockerfile,
            args,
            parents,
        })
    }
}

impl BuildImage {
    pub fn required(&self) -> IndexMap<String, &Self> {
        let mut map = IndexMap::new();
        for parent in &self.parents {
            map.extend(parent.required());
        }
        map.insert(self.tag.clone(), self);
        map
    }
}

impl FetchedServer {
    pub async fn prepare_image(
        &self,
        state: &Daemon<impl DaemonExtra>,
        docker: &Docker,
        server_data: &ServerData,
    ) -> Result<String, ServerStartError> {
        let required: IndexMap<_, _> = self
            .build
            .required()
            .into_iter()
            .map(|(identifier, image)| {
                (
                    format!("{}:{}", crate::common::IMAGE_NAME, identifier),
                    image,
                )
            })
            .collect();

        let images = docker
            .list_images(Some(ListImagesOptions {
                filters: Some(HashMap::from([(
                    "reference".to_owned(),
                    required.keys().cloned().collect(),
                )])),
                ..ListImagesOptions::default()
            }))
            .await
            .map_err(ServerStartError::Docker)?;

        for tag in required.keys() {
            mark_image_used(&state.conn, tag).await?;
        }

        let present: HashSet<String> = images
            .into_iter()
            .filter_map(|image| {
                image
                    .repo_tags
                    .into_iter()
                    .map(normalize_tag)
                    .find(|tag| required.contains_key(tag))
            })
            .collect();

        let missing: Vec<_> = required
            .into_iter()
            .filter(|(key, _)| !present.contains(key))
            .collect();

        if !missing.is_empty() {
            Self::log(
                &server_data.sender,
                &server_data.buffer,
                "Building Image...",
            )
            .await;

            for (tag, image) in missing {
                debug!("Building image {tag} for server {}...", self.id);
                let dockerfile = {
                    let dockerfile = image.dockerfile.as_bytes();
                    let mut header = Header::new_gnu();
                    header.set_path("Dockerfile")?;
                    header.set_size(dockerfile.len() as u64);
                    header.set_cksum();
                    let mut ar = Builder::new(Vec::new());
                    ar.append(&header, dockerfile)
                        .map_err(ServerStartError::IO)?;
                    ar.finish().map_err(ServerStartError::IO)?;
                    ar.into_inner().map_err(ServerStartError::IO)?
                };
                let mut args = image.args.clone();
                if args.get("BACKEND_URL").eq(&Some(&String::new()))
                    && let Ok(backend_url) = state.config.backend_url.join("/")
                {
                    args.insert("BACKEND_URL".to_owned(), backend_url.to_string());
                }

                args.entry("CONTAINER_UID".to_owned())
                    .or_insert_with(|| crate::common::CONTAINER_DEFAULT_UID.to_string());
                args.entry("CONTAINER_GID".to_owned())
                    .or_insert_with(|| crate::common::CONTAINER_DEFAULT_GID.to_string());

                let mut image = docker.build_image(
                    BuildImageOptions {
                        buildargs: Some(args),
                        t: Some(tag),
                        rm: true,
                        ..BuildImageOptions::default()
                    },
                    None,
                    Some(http_body_util::Either::Left(Full::new(bytes::Bytes::from(
                        dockerfile,
                    )))),
                );

                let mut last_progress = Vec::new();
                let mut progress_map = HashMap::new();
                let mut sort_vec = Vec::new();
                while let Some(progress) = image.next().await {
                    let progress = progress.map_err(ServerStartError::Docker)?;

                    if let Some(stream) = progress.stream {
                        if !progress_map.is_empty() {
                            progress_map.clear();
                        }
                        Self::console(
                            &server_data.sender,
                            &server_data.buffer,
                            stream.replace('\n', "\r\n"),
                        )
                        .await;
                    // It doesn't matter if there could be two \r after this
                    } else if let Some(id) = progress.id {
                        if !progress_map.contains_key(&id) {
                            sort_vec.push(id.clone());
                        }
                        progress_map.insert(id, (progress.progress_detail, progress.status)); // TODO progress deprecated

                        let mut msg = format!(
                            "{}\r\n",
                            termion::cursor::Up(if last_progress.is_empty() {
                                0
                            } else {
                                u16::try_from(last_progress.len().saturating_add(1))
                                    .unwrap_or_default()
                            })
                        );

                        last_progress.clear();
                        for (id, (progress, status)) in sort_vec
                            .iter()
                            .map(|key| (key, progress_map.get(key)))
                            .filter_map(|(key, opt)| opt.map(|opt| (key, opt)))
                        {
                            let mut msg_intermediate = String::new();
                            msg_intermediate += clear::CurrentLine.as_ref();
                            msg_intermediate += id;
                            if let Some(status) = status {
                                msg_intermediate += ": ";
                                msg_intermediate += status;
                                if let Some(progress) = progress
                                    && let Some(current) = progress.current
                                    && let Some(total) = progress.total
                                {
                                    #[expect(clippy::integer_division)]
                                    #[expect(clippy::cast_precision_loss)]
                                    let mut percentage =
                                        ((current as f64 / total as f64) * 100f64) as i32 / 2i32;
                                    if percentage > 50i32 {
                                        percentage = 50i32;
                                    }
                                    let percentage =
                                        usize::try_from(percentage).unwrap_or_default();

                                    msg_intermediate += " [";
                                    msg_intermediate.push_str(&"=".repeat(percentage));
                                    msg_intermediate += ">";
                                    msg_intermediate
                                        .push_str(&" ".repeat(50usize.saturating_sub(percentage)));
                                    msg_intermediate += "]";
                                }
                                last_progress.push(msg_intermediate);
                            }
                        }
                        for line in &last_progress {
                            msg += line;
                            msg += "\r\n";
                        }
                        if !last_progress.is_empty() {
                            Self::console(&server_data.sender, &server_data.buffer, msg).await;
                        }
                    } else {
                        // No information about the progress, just do nothing
                    }
                }
            }
        }
        Ok(format!("{}:{}", crate::common::IMAGE_NAME, self.build.tag))
    }
}

async fn mark_image_used(conn: &Pool<MySql>, tag: &str) -> Result<(), sqlx::Error> {
    query!(
        "INSERT INTO `image_usage` (`tag`) VALUES (?) ON DUPLICATE KEY UPDATE `last_used` = CURRENT_TIMESTAMP",
        tag
    )
    .execute(conn)
    .await?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
enum ImageCleanupError {
    Docker(#[from] bollard::errors::Error),
    Sql(#[from] sqlx::Error),
}

/// Periodically removes daemon images that have not been used for [`IMAGE_UNUSED_DAYS`].
pub async fn run_cleanup(docker: &Docker, conn: &Pool<MySql>) -> ! {
    let mut interval = tokio::time::interval(IMAGE_CLEANUP_INTERVAL);
    loop {
        interval.tick().await;
        if let Err(err) = cleanup_unused_images(docker, conn).await {
            warn!("Failed to clean up unused images: {err:?}");
        }
    }
}

async fn cleanup_unused_images(
    docker: &Docker,
    conn: &Pool<MySql>,
) -> Result<(), ImageCleanupError> {
    let prefix = format!("{}:", crate::common::IMAGE_NAME);
    let present: HashSet<String> = docker
        .list_images(None::<ListImagesOptions>)
        .await?
        .into_iter()
        .flat_map(|image| image.repo_tags)
        .map(normalize_tag)
        .filter(|tag| tag.starts_with(&prefix))
        .collect();

    // Images built before usage tracking existed start their retention period now.
    for tag in &present {
        query!("INSERT IGNORE INTO `image_usage` (`tag`) VALUES (?)", tag)
            .execute(conn)
            .await?;
    }

    let stale = query_scalar!(
        "SELECT `tag` FROM `image_usage` WHERE `last_used` < NOW() - INTERVAL ? DAY",
        IMAGE_UNUSED_DAYS
    )
    .fetch_all(conn)
    .await?;

    for tag in stale {
        if present.contains(&tag) {
            match docker
                .remove_image(&tag, None::<RemoveImageOptions>, None)
                .await
            {
                Ok(_)
                | Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 404, ..
                }) => {
                    info!("Removed image {tag} (unused for {IMAGE_UNUSED_DAYS} days)");
                }
                Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 409, ..
                }) => {
                    // Still referenced by a container, so it is in use
                    mark_image_used(conn, &tag).await?;
                    continue;
                }
                Err(err) => {
                    warn!("Failed to remove unused image {tag}: {err:?}");
                    continue;
                }
            }
        }
        query!("DELETE FROM `image_usage` WHERE `tag` = ?", tag)
            .execute(conn)
            .await?;
    }

    docker
        .prune_images(Some(PruneImagesOptions {
            filters: Some(HashMap::from([(
                "dangling".to_owned(),
                vec!["true".to_owned()],
            )])),
        }))
        .await?;
    Ok(())
}
