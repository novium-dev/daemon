// SPDX-License-Identifier: AGPL-3.0-only
#[cfg(feature = "service")]
use std::io;
#[cfg(feature = "service")]
use tokio_util::io::SyncIoBridge;
use url::Url;

#[cfg(feature = "docker")]
pub async fn identify_container_by_pid(
    docker: &bollard::Docker,
    pid: i32,
) -> Result<u32, std::io::Error> {
    use bollard::query_parameters::InspectContainerOptions;
    use std::io;
    use tokio::fs::File;
    use tokio::io::AsyncReadExt as _;

    let regex = lazy_regex::regex!("docker-([a-z0-9]{64}).scope");

    let mut file = File::open(format!("/proc/{pid}/cgroup")).await?;
    let mut buf = String::new();
    file.read_to_string(&mut buf).await?;
    let captures = regex.captures(&buf);
    match captures {
        None => Err(io::Error::other("Not in a docker cgroup")),
        Some(captures) => {
            let capture = &captures[1];
            let container = match docker
                .inspect_container(capture, None::<InspectContainerOptions>)
                .await
            {
                Ok(container) => container,
                Err(err) => return Err(io::Error::other(err)),
            };
            match container.name.as_deref() {
                None => Err(io::Error::other("Container unknown (name missing)")),
                Some(mut name) => {
                    if let Some(new_name) = name.strip_prefix('/') {
                        name = new_name;
                    }

                    let Some(stripped_name) = name.strip_prefix(crate::CONTAINER_PREFIX) else {
                        return Err(io::Error::other(format!(
                            "Container unknown (not starting with prefix: {name:?})"
                        )));
                    };
                    stripped_name.parse().map_err(io::Error::other)
                }
            }
        }
    }
}

pub fn base_url(url: &mut Url) {
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
}

/// Replaces the origin (scheme, host and port) of `url` with that of `base`, keeping the path and
/// query intact. Used to rebase signed URLs onto a node's public address.
#[must_use]
pub fn rebase_origin(mut url: Url, base: &Url) -> Url {
    let _ = url.set_scheme(base.scheme());
    let _ = url.set_host(base.host_str());
    let _ = url.set_port(base.port());
    url
}

#[cfg(feature = "service")]
pub trait JsonExt {
    fn to_yaml(self) -> serde_yaml_ng::Value;

    fn from_yaml(yaml: serde_yaml_ng::Value) -> Self;
}

#[cfg(feature = "service")]
const JSON_MAGIC: &str = "{DAEMON_JSON_MAGIC}!";

#[cfg(feature = "service")]
impl JsonExt for serde_json::Value {
    fn to_yaml(self) -> serde_yaml_ng::Value {
        match self {
            Self::Null => serde_yaml_ng::Value::Null,
            Self::Bool(value) => serde_yaml_ng::Value::Bool(value),
            Self::Number(value) => match (value.as_u64(), value.as_i64(), value.as_f64()) {
                (Some(value), _, _) => {
                    serde_yaml_ng::Value::Number(serde_yaml_ng::Number::from(value))
                }
                (_, Some(value), _) => {
                    serde_yaml_ng::Value::Number(serde_yaml_ng::Number::from(value))
                }
                (_, _, value) => serde_yaml_ng::Value::Number(serde_yaml_ng::Number::from(
                    value.unwrap_or_default(),
                )),
            },
            Self::String(value) => serde_yaml_ng::Value::String(value),
            Self::Array(value) => {
                serde_yaml_ng::Value::Sequence(value.into_iter().map(Self::to_yaml).collect())
            }
            Self::Object(value) => {
                if let Some(Self::String(tag)) = value.get(&format!("{JSON_MAGIC}_tag"))
                    && let Some(untagged) = value.get("value")
                    && value.len() == 2
                {
                    let tag = serde_yaml_ng::value::Tag::new(tag);
                    serde_yaml_ng::Value::Tagged(Box::new(serde_yaml_ng::value::TaggedValue {
                        tag,
                        value: untagged.clone().to_yaml(),
                    }))
                } else {
                    let mut map = serde_yaml_ng::Mapping::with_capacity(value.len());
                    for (key, val) in value {
                        let val = val.to_yaml();
                        if let Some(stripped_key) = key.strip_prefix(JSON_MAGIC) {
                            let Ok(key_json) = serde_json::from_str::<Self>(stripped_key) else {
                                map.insert(serde_yaml_ng::Value::String(key), val);
                                continue;
                            };
                            let key = key_json.to_yaml();
                            map.insert(key, val);
                            continue;
                        }
                        map.insert(serde_yaml_ng::Value::String(key), val);
                    }
                    serde_yaml_ng::Value::Mapping(map)
                }
            }
        }
    }

    fn from_yaml(yaml: serde_yaml_ng::Value) -> Self {
        use serde_yaml_ng::Value;
        match yaml {
            Value::Null => Self::Null,
            Value::Bool(value) => Self::Bool(value),
            Value::Number(value) => match (value.as_u64(), value.as_i64(), value.as_f64()) {
                (Some(value), _, _) => Self::Number(serde_json::Number::from(value)),
                (_, Some(value), _) => Self::Number(serde_json::Number::from(value)),
                (_, _, value) => Self::Number(
                    serde_json::Number::from_f64(value.unwrap_or_default())
                        .unwrap_or_else(|| serde_json::Number::from(0u64)),
                ),
            },
            Value::String(value) => Self::String(value),
            Value::Sequence(seq) => Self::Array(seq.into_iter().map(Self::from_yaml).collect()),
            Value::Mapping(map) => Self::Object(
                map.into_iter()
                    .map(|(key, val)| {
                        let key_json = Self::from_yaml(key);
                        let key = if let Self::String(str) = key_json {
                            str
                        } else {
                            format!(
                                "{JSON_MAGIC}{}",
                                serde_json::to_string(&key_json).unwrap_or_default()
                            )
                        };
                        (key, Self::from_yaml(val))
                    })
                    .collect(),
            ),
            Value::Tagged(tagged) => {
                let mut obj = serde_json::Map::with_capacity(2);
                obj.insert(
                    format!("{JSON_MAGIC}_tag"),
                    Self::String(tagged.tag.to_string()),
                );
                obj.insert("value".to_owned(), Self::from_yaml(tagged.value));
                Self::Object(obj)
            }
        }
    }
}

#[cfg(feature = "service")]
pub(crate) async fn io_sync<
    F: FnOnce(&mut SyncIoBridge<R>) -> Result<V, E> + Send + 'static,
    V: Send + 'static,
    E: Send + From<io::Error> + 'static,
    R: Unpin + Send + 'static,
>(
    func: F,
    io: R,
) -> Result<(R, V), E> {
    tokio::task::spawn_blocking(move || {
        let mut read = SyncIoBridge::new(io);
        let val = func(&mut read)?;
        Ok((read.into_inner(), val))
    })
    .await
    .map_err(io::Error::from)?
}

#[cfg(feature = "service")]
pub(crate) async fn io_sync_seek_truncate<
    F: FnOnce(&mut SyncIoBridge<R>) -> Result<V, E> + Send + 'static,
    V: Send + 'static,
    E: Send + From<io::Error> + 'static,
    R: tokio::io::AsyncSeek + Truncate + Unpin + Send + 'static,
>(
    func: F,
    mut io: R,
) -> Result<(R, V), E> {
    use tokio::io::AsyncSeekExt as _;
    io.seek(io::SeekFrom::Start(0)).await?;
    io.truncate().await?;
    tokio::task::spawn_blocking(move || {
        let mut read = SyncIoBridge::new(io);
        let val = func(&mut read)?;
        Ok((read.into_inner(), val))
    })
    .await
    .map_err(io::Error::from)?
}

#[cfg(feature = "service")]
pub(crate) trait Truncate {
    async fn truncate(&self) -> Result<(), io::Error>;
}

#[cfg(feature = "service")]
impl Truncate for tokio::fs::File {
    async fn truncate(&self) -> Result<(), io::Error> {
        self.set_len(0).await
    }
}
