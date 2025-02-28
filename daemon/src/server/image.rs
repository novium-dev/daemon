use crate::server::start::ServerStartError;
use crate::server::{FetchedServer, Server};
use crate::{DaemonState, ServerData};
use async_std::stream::StreamExt;
use bollard::image::{BuildImageOptions, ListImagesOptions};
use bollard::Docker;
use indexmap::IndexMap;
use ringbuffer::AllocRingBuffer;
use sqlx::{query, query_scalar, Executor, MySql};
use std::collections::{HashMap, HashSet};
use tar::{Builder, Header};
use termion::clear;

#[derive(Debug, Clone, Default)]
pub(crate) struct BuildImage {
    pub(crate) tag: String,
    pub(crate) dockerfile: String,
    pub(crate) args: HashMap<String, String>,
    pub(crate) parents: Vec<BuildImage>,
}

impl Server {
    pub(crate) async fn resolve_image(
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

        let tag = match (tags.get(&id).cloned(), image.default_tag) {
            (Some(tag), _) => tag,
            (None, Some(tag)) => tag,
            (None, None) => return Err(sqlx::Error::RowNotFound),
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
    pub fn required(&self) -> IndexMap<String, &BuildImage> {
        let mut map = IndexMap::new();
        for parent in &self.parents {
            map.extend(parent.required());
        }
        map.insert(self.tag.clone(), self);
        map
    }
}

impl FetchedServer {
    pub(crate) async fn prepare_image(
        &self,
        state: &DaemonState,
        docker: &Docker,
        buffer: &mut AllocRingBuffer<u8>,
        server_data: &mut ServerData,
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
                filters: HashMap::from([(
                    "reference".to_owned(),
                    required.keys().cloned().collect(),
                )]),
                ..ListImagesOptions::default()
            }))
            .await
            .map_err(ServerStartError::Docker)?;

        let present: HashSet<String> = images
            .into_iter()
            .filter_map(|image| {
                image
                    .repo_tags
                    .into_iter()
                    .find(|tag| required.contains_key(tag))
            })
            .collect();

        let missing: Vec<_> = required
            .into_iter()
            .filter(|(key, _)| !present.contains(key))
            .collect();

        if !missing.is_empty() {
            Self::log(&server_data.sender, buffer, "Building Image...");

            for (tag, image) in missing {
                let dockerfile = {
                    let dockerfile = image.dockerfile.as_bytes();
                    let mut header = Header::new_gnu();
                    header.set_path("Dockerfile").unwrap();
                    header.set_size(dockerfile.len() as u64);
                    header.set_cksum();
                    let mut ar = Builder::new(Vec::new());
                    ar.append(&header, dockerfile)
                        .map_err(ServerStartError::IO)?;
                    ar.finish().map_err(ServerStartError::IO)?;
                    ar.into_inner().map_err(ServerStartError::IO)?
                };
                let mut args = image.args.clone();
                if args.get("BACKEND_URL").eq(&Some(&String::new())) {
                    args.insert(
                        "BACKEND_URL".to_string(),
                        state.config.backend_url.join("/").unwrap().to_string(),
                    );
                }

                let mut image = docker.build_image(
                    BuildImageOptions {
                        buildargs: args,
                        t: tag,
                        rm: true,
                        ..BuildImageOptions::default()
                    },
                    None,
                    Some(bytes::Bytes::from(dockerfile)),
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
                        Self::console(&server_data.sender, buffer, stream.replace('\n', "\r\n"));
                    // It doesn't matter if there could be two \r after this
                    } else if let Some(id) = progress.id {
                        if !progress_map.contains_key(&id) {
                            sort_vec.push(id.clone());
                        }
                        progress_map.insert(id, (progress.progress, progress.status));

                        let mut msg = format!(
                            "{}\r\n",
                            termion::cursor::Up(if last_progress.is_empty() {
                                0
                            } else {
                                last_progress.len() as u16 + 1
                            })
                        );

                        last_progress.clear();
                        for (id, (progress, status)) in sort_vec
                            .iter()
                            .map(|key| (key, progress_map.get(key)))
                            .filter(|(_, opt)| opt.is_some())
                            .map(|(key, opt)| (key, opt.unwrap()))
                        {
                            let mut msg_intermediate = String::new();
                            msg_intermediate += clear::CurrentLine.as_ref();
                            msg_intermediate += id;
                            if let Some(status) = status {
                                msg_intermediate += ": ";
                                msg_intermediate += status;
                                if let Some(progress) = progress {
                                    msg_intermediate += " ";
                                    msg_intermediate += progress;
                                }
                                last_progress.push(msg_intermediate);
                            }
                        }
                        for line in &last_progress {
                            msg += line;
                            msg += "\r\n";
                        }
                        if !last_progress.is_empty() {
                            Self::console(&server_data.sender, buffer, msg);
                        }
                    }
                }
            }
        }
        Ok(format!("{}:{}", crate::common::IMAGE_NAME, self.build.tag))
    }
}
