use crate::server::image::BuildImage;
use crate::server::network::{AssignedNetwork, Port};
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use daemon_common::payload::server::ServerFileWrite;
use sqlx::{query, query_as, query_scalar, Executor, MySql};
use std::collections::{HashMap, HashSet};
use std::io;
use std::io::ErrorKind;

pub(crate) mod exec;
pub(crate) mod image;
pub(crate) mod network;
pub(crate) mod start;
pub(crate) mod status;
pub(crate) mod stop;
pub(crate) mod terminal;
pub(crate) mod watcher;

#[derive(Debug, Clone, Default)]
pub(crate) struct FetchedServer {
    pub(crate) id: u32,

    pub(crate) build: BuildImage,
    pub(crate) backup_exclude: HashSet<String>,
    pub(crate) files_write: HashMap<String, ServerFileWrite>,

    pub(crate) cpu: Option<f32>,
    pub(crate) ram: Option<u32>,
    pub(crate) swap: Option<u32>,
    pub(crate) disk: Option<u64>,

    pub(crate) cap_add: HashSet<String>,
    pub(crate) cap_drop: HashSet<String>,

    pub(crate) uid: Option<u16>,
    pub(crate) gid: Option<u16>,

    pub(crate) ports: Vec<Port>,
    pub(crate) storage: String,
    pub(crate) internet: bool,
    #[allow(dead_code)]
    pub(crate) ipv6_suffix: Option<u64>,

    pub(crate) networks: Vec<AssignedNetwork>,
}

impl Server {
    pub(crate) async fn fetched(
        self,
        conn: impl Executor<'_, Database = MySql> + Copy,
    ) -> Result<FetchedServer, sqlx::Error> {
        let Some(image) = self.image else {
            return Err(sqlx::Error::ColumnDecode {
                index: "image".to_string(),
                source: io::Error::new(ErrorKind::Other, "image is null").into(),
            });
        };
        let tags = query!(
            "SELECT `image`,`tag` FROM `server_image_tags` WHERE `server` = ?",
            self.id
        )
        .fetch_all(conn)
        .await?
        .into_iter()
        .map(|row| (row.image, row.tag))
        .collect();
        let build = self.resolve_image(conn, image, &tags).await?;

        let backup_exclude: HashSet<String> = query_scalar!(
            "SELECT `exclude` FROM `server_backup_exclude` WHERE `server` = ?",
            self.id
        )
        .fetch_all(conn)
        .await?
        .into_iter()
        .collect();

        let ports: Vec<Port> = query_as!(
            Port,
            "SELECT `internal_port`,`external_port`,`external_ip`,`protocol` FROM `server_ports` WHERE `server` = ?",
            self.id
        )
        .fetch_all(conn)
        .await?;

        let cap_add: HashSet<String> = query_scalar!(
            "SELECT `capability` FROM `server_capability_add` WHERE `server` = ?",
            self.id
        )
        .fetch_all(conn)
        .await?
        .into_iter()
        .collect();

        let cap_drop: HashSet<String> = query_scalar!(
            "SELECT `capability` FROM `server_capability_drop` WHERE `server` = ?",
            self.id
        )
        .fetch_all(conn)
        .await?
        .into_iter()
        .collect();

        let files_write_entries = query!(
            "SELECT `file`,`filetype`,`key` AS `key: Option<String>`,`value` FROM `server_file_writes` WHERE `server` = ?",
            self.id
        )
        .fetch_all(conn)
        .await?;

        let mut files_write: HashMap<String, ServerFileWrite> = HashMap::new();
        for entry in files_write_entries {
            match entry.filetype.as_str() {
                "properties" => {
                    if !files_write.contains_key(&entry.file) {
                        files_write.insert(
                            entry.file.clone(),
                            ServerFileWrite::PROPERTIES {
                                content: HashMap::new(),
                            },
                        );
                    }
                    if let ServerFileWrite::PROPERTIES { content } =
                        files_write.get_mut(&entry.file).unwrap()
                    {
                        content.insert(
                            entry
                                .key
                                .ok_or(sqlx::Error::Decode(Box::new(io::Error::new(
                                    ErrorKind::Other,
                                    "key is null!",
                                ))))?,
                            entry.value,
                        );
                    }
                }
                "write" => {
                    files_write.insert(
                        entry.file.clone(),
                        ServerFileWrite::WRITE {
                            data: BASE64_STANDARD
                                .decode(entry.value)
                                .map_err(Box::new)
                                .map_err(|err| sqlx::Error::Decode(err))?,
                        },
                    );
                }
                "json" => {
                    if !files_write.contains_key(&entry.file) {
                        files_write.insert(
                            entry.file.clone(),
                            ServerFileWrite::JSON {
                                content: HashMap::new(),
                            },
                        );
                    }
                    if let ServerFileWrite::JSON { content } =
                        files_write.get_mut(&entry.file).unwrap()
                    {
                        content.insert(
                            entry
                                .key
                                .ok_or(sqlx::Error::Decode(Box::new(io::Error::new(
                                    ErrorKind::Other,
                                    "key is null!",
                                ))))?,
                            serde_json::from_str(&entry.value)
                                .map_err(|err| sqlx::Error::Decode(Box::new(err)))?,
                        );
                    }
                }
                "yaml" => {
                    if !files_write.contains_key(&entry.file) {
                        files_write.insert(
                            entry.file.clone(),
                            ServerFileWrite::YAML {
                                content: HashMap::new(),
                            },
                        );
                    }
                    if let ServerFileWrite::YAML { content } =
                        files_write.get_mut(&entry.file).unwrap()
                    {
                        content.insert(
                            entry
                                .key
                                .ok_or(sqlx::Error::Decode(Box::new(io::Error::new(
                                    ErrorKind::Other,
                                    "key is null!",
                                ))))?,
                            serde_yaml::from_str(&entry.value)
                                .map_err(|err| sqlx::Error::Decode(Box::new(err)))?,
                        );
                    }
                }
                _ => {}
            }
        }

        let networks = query_as!(AssignedNetwork, "SELECT `networks`.`id` as `network_id`,`networks`.`ipv4` as `network_ipv4`,`networks`.`ipv6` as `network_ipv6`,`server_networks`.`ipv4` as `server_ipv4`,`server_networks`.`ipv6` as `server_ipv6` FROM `server_networks` INNER JOIN `networks` ON `networks`.`id` = `server_networks`.`network` WHERE `server_networks`.`server` = ?", self.id)
            .fetch_all(conn)
            .await?;

        Ok(FetchedServer {
            id: self.id,
            build,
            backup_exclude,
            files_write,

            cpu: self.cpu,
            ram: self.ram,
            swap: self.swap,
            disk: self.disk,

            uid: self.uid,
            gid: self.gid,

            ports,

            cap_add,
            cap_drop,

            storage: self.storage,
            internet: self.internet,
            ipv6_suffix: self.ipv6_suffix,

            networks,
        })
    }
}

#[derive(sqlx::FromRow)]
pub(crate) struct Server {
    pub(crate) id: u32,

    pub(crate) image: Option<u64>,

    pub(crate) cpu: Option<f32>,  // in Threads
    pub(crate) ram: Option<u32>,  // in MiB
    pub(crate) swap: Option<u32>, // in MiB
    pub(crate) disk: Option<u64>, // in MiB

    pub(crate) storage: String,
    pub(crate) internet: bool,
    pub(crate) ipv6_suffix: Option<u64>,

    pub(crate) uid: Option<u16>,
    pub(crate) gid: Option<u16>,
}

impl FetchedServer {
    pub(crate) fn container_name(id: u32) -> String {
        format!("{}{}", crate::common::CONTAINER_PREFIX, id)
    }
}
