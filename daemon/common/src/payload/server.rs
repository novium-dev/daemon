use crate::service::error_storage_quota_reached::FromStorageQuotaReached;
use crc64::crc64;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_yaml::Mapping;
use std::collections::{HashMap, HashSet};
use std::io;
use std::io::{ErrorKind, Seek, SeekFrom};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tracing::{debug, error, warn};

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct ServerPayload {
    pub storage: Option<String>,
    pub resource: Option<ServerResourcePayload>,
    pub build: Option<ServerBuildPayload>,
    pub ports: Option<Vec<ServerPortEntryPayload>>,
    pub network: Option<ServerNetworkPayload>,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ServerResourcePayload {
    pub cpu: Option<f32>,
    pub ram: Option<u32>,
    pub swap: Option<u32>,
    pub disk: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ServerBuildPayload {
    pub image: ServerBuildImage,

    pub backup_exclude: HashSet<String>,
    pub files_write: HashMap<String, ServerFileWrite>,

    pub cap_add: Vec<String>,
    pub cap_drop: Vec<String>,

    pub environment: HashMap<String, String>,

    pub uid: Option<u16>,
    pub gid: Option<u16>,

    pub stop_command: Option<String>,
    pub stop_timeout: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[schema(no_recursion)]
pub struct ServerBuildImage {
    pub dockerfile: String,
    pub default_tag: Option<String>,
    pub tag: Option<String>,
    pub args: HashMap<String, String>,
    pub parents: Vec<ServerBuildImage>,
    pub base: Option<usize>,
}

impl ServerBuildImage {
    pub fn id(&self) -> u64 {
        crc64(0, self.dockerfile().as_bytes())
    }

    pub fn used_tag(&self) -> Option<String> {
        match (&self.default_tag, &self.tag) {
            (_, Some(tag)) | (Some(tag), _) => Some(tag.clone()),
            _ => None,
        }
    }

    pub fn dockerfile(&self) -> String {
        match self.base {
            Some(base) => {
                let Some(parent) = self.parents.get(base) else {
                    return self.dockerfile.clone();
                };
                let Some(tag) = parent.used_tag() else {
                    return self.dockerfile.clone();
                };
                if self.dockerfile.contains("{BASE}") {
                    self.dockerfile
                        .replace("{BASE}", &format!("{}:{}", crate::IMAGE_NAME, tag))
                } else {
                    format!("FROM {}:{}\n{}", crate::IMAGE_NAME, tag, self.dockerfile)
                }
            }
            None => self.dockerfile.clone(),
        }
    }

    pub fn all(&self) -> Vec<&ServerBuildImage> {
        let mut vec = Vec::with_capacity(1);
        for entries in &self.parents {
            vec.extend(entries.all());
        }
        vec.push(self);
        vec
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ServerNetworkPayload {
    pub internet_access: bool,
    pub networks: Vec<NetworkAttachment>,
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct NetworkAttachment {
    pub network: u32,
    pub ipv4: u8,
    pub ipv6: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "filetype")]
pub enum ServerFileWrite {
    #[serde(rename = "write")]
    WRITE { data: Vec<u8> },
    #[serde(rename = "properties")]
    PROPERTIES { content: HashMap<String, String> },
    #[serde(rename = "json")]
    JSON {
        content: HashMap<String, serde_json::Value>,
    },
    #[serde(rename = "yaml")]
    YAML {
        content: HashMap<String, serde_yaml::Value>,
    },
}

impl ServerFileWrite {
    pub async fn apply<E: From<io::Error> + FromStorageQuotaReached + Send + 'static>(
        &self,
        mut file: tokio::fs::File,
    ) -> Result<(), E> {
        match self {
            ServerFileWrite::WRITE { data } => {
                file.set_len(0)
                    .await
                    .inspect_err(|err| error!("Failed to truncate file: {:?}", err))?;
                file.seek(SeekFrom::Start(0))
                    .await
                    .inspect_err(|err| error!("Failed to seek file: {:?}", err))?;
                file.write_all(data)
                    .await
                    .inspect_err(|err| {
                        if err.kind() == ErrorKind::QuotaExceeded {
                            debug!("Failed to write data: {:?}", err)
                        } else {
                            error!("Failed to write data: {:?}", err)
                        }
                    })
                    .map_err(|err| {
                        if err.kind() == ErrorKind::QuotaExceeded {
                            <E as FromStorageQuotaReached>::from()
                        } else {
                            <E as From<io::Error>>::from(err)
                        }
                    })?;
            }
            ServerFileWrite::PROPERTIES { content } => {
                let content = content.clone();
                let mut file = file.into_std().await;
                tokio::task::spawn_blocking(move || {
                    let mut current = java_properties::read(&mut file)
                        .inspect_err(|err| debug!("Failed to read properties file: {:?}", err))
                        .unwrap_or_default();
                    for (key, val) in content {
                        current.insert(key, val);
                    }

                    file.set_len(0)
                        .inspect_err(|err| error!("Failed to truncate file: {:?}", err))?;
                    file.seek(SeekFrom::Start(0))
                        .inspect_err(|err| error!("Failed to seek file: {:?}", err))?;
                    java_properties::write(&mut file, &current)
                        .inspect_err(|err| warn!("Failed to write properties: {:?}", err)) // only warning, Potential QUOTA_EXCEEDED
                        .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                    Ok::<_, E>(())
                })
                .await
                .inspect_err(|err| error!("Failed to write properties (task): {:?}", err))
                .map_err(|err| io::Error::new(ErrorKind::Other, err))??;
            }
            ServerFileWrite::JSON { content } => {
                let mut file = file.into_std().await;
                let content = content.clone();
                let (mut value, mut file) =
                    tokio::task::spawn_blocking(move || match serde_json::from_reader(&mut file) {
                        Ok(val) => Ok::<_, E>((val, file)),
                        Err(err) if err.is_io() => {
                            let kind = err.io_error_kind().unwrap();
                            if kind == ErrorKind::QuotaExceeded {
                                debug!("Failed to read json file: {:?}", err);
                                Err(<E as FromStorageQuotaReached>::from())
                            } else {
                                error!("Failed to read json file: {:?}", err);
                                Err(<E as From<io::Error>>::from(io::Error::new(
                                    err.io_error_kind().unwrap(),
                                    err,
                                )))
                            }
                        }
                        Err(_err) => Ok((Value::Object(Map::new()), file)),
                    })
                    .await
                    .inspect_err(|err| error!("Failed to read json (task): {:?}", err))
                    .map_err(|err| io::Error::new(ErrorKind::Other, err))??;

                for (key, val) in content {
                    let components: Vec<String> = key.split(".").map(String::from).collect();
                    let mut current = &mut value;
                    for comp in components {
                        match &current {
                            Value::Array(array) => {
                                let index = comp
                                    .parse::<usize>()
                                    .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
                                if index < array.len() {
                                    current = &mut current.as_array_mut().unwrap()[index];
                                } else {
                                    while index >= current.as_array_mut().unwrap().len() {
                                        current.as_array_mut().unwrap().push(
                                            serde_json::Value::Object(serde_json::Map::new()),
                                        );
                                    }
                                    current = &mut current.as_array_mut().unwrap()[index];
                                }
                            }
                            Value::Object(obj) => {
                                if obj.contains_key(&comp) {
                                    current =
                                        current.as_object_mut().unwrap().get_mut(&comp).unwrap();
                                } else {
                                    current.as_object_mut().unwrap().insert(
                                        comp.clone(),
                                        serde_json::Value::Object(serde_json::Map::new()),
                                    );
                                    current =
                                        current.as_object_mut().unwrap().get_mut(&comp).unwrap();
                                }
                            }
                            _ => {
                                *current = serde_json::Value::Object(Map::new());
                                if current.as_object_mut().unwrap().contains_key(&comp) {
                                    current =
                                        current.as_object_mut().unwrap().get_mut(&comp).unwrap();
                                } else {
                                    current.as_object_mut().unwrap().insert(
                                        comp.clone(),
                                        serde_json::Value::Object(serde_json::Map::new()),
                                    );
                                    current =
                                        current.as_object_mut().unwrap().get_mut(&comp).unwrap();
                                }
                            }
                        }
                    }
                    *current = val;
                }
                tokio::task::spawn_blocking(move || {
                    file.set_len(0)
                        .inspect_err(|err| error!("Failed to truncate file: {:?}", err))?;
                    file.seek(SeekFrom::Start(0))
                        .inspect_err(|err| error!("Failed to seek file: {:?}", err))?;
                    serde_json::to_writer_pretty(&mut file, &value)
                        .inspect_err(|err| warn!("Failed to write json: {:?}", err))
                        .map_err(|err| io::Error::new(ErrorKind::Other, err))
                })
                .await
                .inspect_err(|err| error!("Failed to write json (task): {:?}", err))
                .map_err(|err| io::Error::new(ErrorKind::Other, err))??;
            }
            ServerFileWrite::YAML { content } => {
                let mut file = file.into_std().await;
                let content = content.clone();
                let (mut value, mut file) = tokio::task::spawn_blocking(move || {
                    serde_yaml::from_reader(&mut file)
                        .inspect_err(|err| debug!("Failed to read yaml: {:?}", err))
                        .map_err(|err| {
                            <E as From<io::Error>>::from(io::Error::new(ErrorKind::Other, err))
                        })
                        .map(|val: serde_yaml::Value| (val, file))
                })
                .await
                .inspect_err(|err| error!("Failed to read yaml (task): {:?}", err))
                .map_err(|err| io::Error::new(ErrorKind::Other, err))??;

                for (key, val) in content {
                    let components: Vec<String> = key.split(".").map(String::from).collect();
                    let mut current = &mut value;
                    for comp in components {
                        match &current {
                            serde_yaml::Value::Sequence(array) => {
                                let index = comp.parse::<usize>().map_err(|err| {
                                    <E as From<io::Error>>::from(io::Error::new(
                                        ErrorKind::Other,
                                        err,
                                    ))
                                })?;
                                if index < array.len() {
                                    current = &mut current.as_sequence_mut().unwrap()[index];
                                } else {
                                    while index >= current.as_sequence_mut().unwrap().len() {
                                        current.as_sequence_mut().unwrap().push(
                                            serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
                                        );
                                    }
                                    current = &mut current.as_sequence_mut().unwrap()[index];
                                }
                            }
                            serde_yaml::Value::Mapping(obj) => {
                                if obj.contains_key(&comp) {
                                    current =
                                        current.as_mapping_mut().unwrap().get_mut(&comp).unwrap();
                                } else {
                                    current.as_mapping_mut().unwrap().insert(
                                        serde_yaml::Value::String(comp.clone()),
                                        serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
                                    );
                                    current =
                                        current.as_mapping_mut().unwrap().get_mut(&comp).unwrap();
                                }
                            }
                            _ => {
                                *current = serde_yaml::Value::Mapping(Mapping::new());
                                if current.as_mapping_mut().unwrap().contains_key(&comp) {
                                    current =
                                        current.as_mapping_mut().unwrap().get_mut(&comp).unwrap();
                                } else {
                                    current.as_mapping_mut().unwrap().insert(
                                        serde_yaml::Value::String(comp.clone()),
                                        serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
                                    );
                                    current =
                                        current.as_mapping_mut().unwrap().get_mut(&comp).unwrap();
                                }
                            }
                        }
                    }
                    *current = val;
                }
                tokio::task::spawn_blocking(move || {
                    file.set_len(0)
                        .inspect_err(|err| error!("Failed to truncate file: {:?}", err))?;
                    file.seek(SeekFrom::Start(0))
                        .inspect_err(|err| error!("Failed to seek file: {:?}", err))?;
                    serde_yaml::to_writer(&mut file, &value)
                        .inspect_err(|err| warn!("Failed to write yaml: {:?}", err))
                        .map_err(|err| io::Error::new(ErrorKind::Other, err))
                })
                .await
                .map_err(|err| io::Error::new(ErrorKind::Other, err))??;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ServerPortEntryPayload {
    pub internal_port: u16,
    pub external_port: u16,
    pub external_ip: String,
    pub protocol: String,
}
