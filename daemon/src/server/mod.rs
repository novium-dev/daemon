/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
use crate::server::image::BuildImage;
use crate::server::network::{AssignedNetwork, Port};
use base64::Engine as _;
use base64::prelude::BASE64_STANDARD;
use core::fmt::{Display, Formatter};
use daemon_common::service::server::ServerFileWrite;
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::{Database, Decode, Encode, Executor, MySql, Pool, query, query_as, query_scalar};
use std::collections::{HashMap, HashSet};
use std::io;

pub mod exec;
pub mod image;
pub mod network;
pub mod start;
pub mod status;
pub mod stop;
pub mod terminal;
pub mod watcher;

#[derive(Debug, Clone, Default)]
pub struct FetchedServer {
    pub id: u32,

    pub build: BuildImage,
    pub backup_exclude: HashSet<String>,
    pub files_write: HashMap<String, ServerFileWrite>,

    pub cpu: Option<f32>,
    pub ram: Option<u32>,
    pub swap: Option<u32>,
    pub disk: Option<u64>,

    pub cap_add: HashSet<String>,
    pub cap_drop: HashSet<String>,

    pub environment: HashMap<String, String>,

    pub uid: Option<u16>,
    pub gid: Option<u16>,

    pub stop_command: Option<String>,
    pub stop_timeout: u16,

    pub locks: HashSet<String>,
    pub ports: Vec<Port>,
    pub storage: String,
    pub internet: bool,
    #[expect(dead_code, reason = "not implemented yet, but present in the db")]
    pub ipv6_suffix: Option<u64>,

    pub networks: Vec<AssignedNetwork>,
}

#[derive(Debug, Copy, Clone, Ord, PartialOrd, PartialEq, Eq)]
pub enum ServerLock {
    Storage,
}

impl Server {
    pub async fn fetched(
        self,
        conn: impl Executor<'_, Database = MySql> + Copy,
    ) -> Result<FetchedServer, sqlx::Error> {
        let Some(image) = self.image else {
            return Err(sqlx::Error::ColumnDecode {
                index: "image".to_owned(),
                source: io::Error::other("image is null").into(),
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

        let locks: HashSet<String> = query_scalar!(
            "SELECT `lock` FROM `server_locks` WHERE `server` = ?",
            self.id
        )
        .fetch_all(conn)
        .await?
        .into_iter()
        .collect();

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
            "SELECT `internal_port` AS `internal`,`external_port` AS `external`,`external_ip`,`protocol` FROM `server_ports` WHERE `server` = ?",
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

        let environment: HashMap<String, String> = query!(
            "SELECT `key`, `value` FROM `server_environment` WHERE `server` = ?",
            self.id
        )
        .fetch_all(conn)
        .await?
        .into_iter()
        .map(|row| (row.key, row.value))
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
                    if let ServerFileWrite::JavaProperties { content } = files_write
                        .entry(entry.file.clone())
                        .or_insert_with(|| ServerFileWrite::JavaProperties {
                            content: HashMap::new(),
                        })
                    {
                        content.insert(
                            entry
                                .key
                                .ok_or(sqlx::Error::Decode(Box::new(io::Error::other(
                                    "key is null!",
                                ))))?,
                            entry.value,
                        );
                    }
                }
                "write" => {
                    files_write.insert(
                        entry.file.clone(),
                        ServerFileWrite::Write {
                            data: BASE64_STANDARD
                                .decode(entry.value)
                                .map_err(Box::new)
                                .map_err(|err| sqlx::Error::Decode(err))?,
                        },
                    );
                }
                "json" => {
                    if let ServerFileWrite::Json { set } = files_write
                        .entry(entry.file.clone())
                        .or_insert_with(|| ServerFileWrite::Json { set: Vec::new() })
                    {
                        set.push(
                            serde_json::from_str(&entry.value)
                                .map_err(|err| sqlx::Error::Decode(Box::new(err)))?,
                        );
                    }
                }
                "yaml" => {
                    if let ServerFileWrite::Yaml { set } = files_write
                        .entry(entry.file.clone())
                        .or_insert_with(|| ServerFileWrite::Yaml { set: Vec::new() })
                    {
                        set.push(
                            serde_json::from_str(&entry.value)
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

            stop_command: self.stop_command,
            stop_timeout: self.stop_timeout,

            locks,

            ports,

            cap_add,
            cap_drop,

            environment,

            storage: self.storage,
            internet: self.internet,
            ipv6_suffix: self.ipv6_suffix,

            networks,
        })
    }
}

#[derive(sqlx::FromRow)]
pub struct Server {
    pub id: u32,

    pub image: Option<u64>,

    pub cpu: Option<f32>,  // in Threads
    pub ram: Option<u32>,  // in MiB
    pub swap: Option<u32>, // in MiB
    pub disk: Option<u64>, // in MiB

    pub storage: String,
    pub internet: bool,
    pub ipv6_suffix: Option<u64>,

    pub uid: Option<u16>,
    pub gid: Option<u16>,

    pub stop_command: Option<String>,
    pub stop_timeout: u16,
}

impl FetchedServer {
    pub fn container_name(id: u32) -> String {
        format!("{}{}", crate::common::CONTAINER_PREFIX, id)
    }

    pub async fn resolve_locked(id: u32, conn: &Pool<MySql>) -> HashSet<String> {
        if let Ok(value) = query_scalar!("SELECT `lock` FROM `server_locks` WHERE `server` = ?", id)
            .fetch_all(conn)
            .await
        {
            return value.into_iter().collect();
        }
        HashSet::new()
    }
}

impl sqlx::Type<MySql> for ServerLock {
    fn type_info() -> <sqlx::MySql as sqlx::Database>::TypeInfo {
        <&str as sqlx::Type<MySql>>::type_info()
    }

    fn compatible(ty: &<sqlx::MySql as sqlx::Database>::TypeInfo) -> bool {
        <&str as sqlx::Type<MySql>>::compatible(ty)
    }
}

impl<'q> sqlx::Encode<'q, MySql> for ServerLock {
    fn encode_by_ref(
        &self,
        buf: &mut <MySql as Database>::ArgumentBuffer<'q>,
    ) -> Result<IsNull, BoxDynError> {
        let str = self.to_string();
        <&str as Encode<'_, MySql>>::encode_by_ref(&&*str, buf)
    }

    fn produces(&self) -> Option<<sqlx::MySql as sqlx::Database>::TypeInfo> {
        <&str as Encode<'_, MySql>>::produces(&&*self.to_string())
    }

    fn size_hint(&self) -> usize {
        <&str as Encode<'_, MySql>>::size_hint(&&*self.to_string())
    }
}

impl<'r> sqlx::Decode<'r, MySql> for ServerLock {
    fn decode(value: <MySql as Database>::ValueRef<'r>) -> Result<Self, BoxDynError> {
        let value = <&str as Decode<'r, MySql>>::decode(value)?;
        Ok(Self::by_name(String::from(value)))
    }
}

impl ServerLock {
    pub fn by_name(_str: String) -> Self {
        Self::Storage
    }
}

impl Display for ServerLock {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("storage")
    }
}
