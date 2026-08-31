// SPDX-License-Identifier: AGPL-3.0-only
use super::{Check, CheckError, IntegrityContext};
use async_trait::async_trait;
use daemon_common::integrity::{IntegrityCategory, IntegrityFinding};
use sqlx::query_scalar;
use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use tokio::fs;

pub(super) const PRIMARY_DIR: &str = "primary";

pub(super) async fn known_servers(ctx: &IntegrityContext) -> Result<HashSet<u32>, CheckError> {
    Ok(query_scalar!(r#"SELECT `id` AS `id: u32` FROM `servers`"#)
        .fetch_all(&*ctx.conn)
        .await?
        .into_iter()
        .collect())
}

pub(super) async fn entries(
    dir: &Path,
) -> Result<Option<Vec<(String, PathBuf, bool)>>, CheckError> {
    let mut read_dir = match fs::read_dir(dir).await {
        Ok(read_dir) => read_dir,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };

    let mut out = Vec::new();
    while let Some(entry) = read_dir.next_entry().await? {
        let metadata = entry.metadata().await?;
        out.push((
            entry.file_name().to_string_lossy().into_owned(),
            entry.path(),
            metadata.is_dir(),
        ));
    }
    Ok(Some(out))
}

pub struct StorageNaming;

#[async_trait]
impl Check for StorageNaming {
    fn id(&self) -> &'static str {
        "storage.naming"
    }

    fn title(&self) -> &'static str {
        "Server storage folders are named after live servers"
    }

    fn description(&self) -> &'static str {
        "A storage root may only hold the primary directory, and that directory may only hold one \
         numerically named folder per server. Anything else was put there by hand, and a folder \
         whose server no longer exists is leftover data still occupying disk."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Storage
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let known = known_servers(ctx).await?;
        let mut findings = Vec::new();

        for (name, storage) in &ctx.config.storage {
            let root = PathBuf::from(storage.path());
            let Some(top) = entries(&root).await? else {
                continue;
            };

            for (entry, path, _is_dir) in top {
                if entry != PRIMARY_DIR {
                    findings.push(
                        IntegrityFinding::warning(format!(
                            "Storage {name} holds an entry the daemon never creates"
                        ))
                        .with_subject(path.to_string_lossy().into_owned())
                        .with_detail("storage", name.clone()),
                    );
                }
            }

            let Some(servers) = entries(&root.join(PRIMARY_DIR)).await? else {
                continue;
            };

            for (entry, path, is_dir) in servers {
                let subject = path.to_string_lossy().into_owned();
                if !is_dir {
                    findings.push(
                        IntegrityFinding::warning(format!(
                            "Storage {name} holds a file where only server directories belong"
                        ))
                        .with_subject(subject)
                        .with_detail("storage", name.clone()),
                    );
                    continue;
                }
                let Ok(id) = entry.parse::<u32>() else {
                    findings.push(
                        IntegrityFinding::warning(format!(
                            "Storage {name} holds a directory that is not named after a server id"
                        ))
                        .with_subject(subject)
                        .with_detail("storage", name.clone()),
                    );
                    continue;
                };
                if !known.contains(&id) {
                    findings.push(
                        IntegrityFinding::warning(format!(
                            "Storage {name} still holds the data of server {id}, which no longer \
                             exists in the daemon database"
                        ))
                        .with_subject(subject)
                        .with_detail("storage", name.clone())
                        .with_detail("server", id.to_string()),
                    );
                }
            }
        }

        Ok(findings)
    }
}

pub struct StorageSubvolume;

#[async_trait]
impl Check for StorageSubvolume {
    fn id(&self) -> &'static str {
        "storage.subvolume"
    }

    fn title(&self) -> &'static str {
        "Server storages on btrfs are subvolumes"
    }

    fn description(&self) -> &'static str {
        "The daemon creates every server directory on a btrfs storage as its own subvolume. A \
         plain directory in its place means the data was copied instead of snapshotted, which \
         breaks quotas and every snapshot based backup taken from it."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Storage
    }

    #[cfg(not(feature = "btrfs"))]
    async fn run(&self, _ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        Err(CheckError::Skipped(
            "This daemon was built without btrfs support".to_owned(),
        ))
    }

    #[cfg(feature = "btrfs")]
    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        use crate::config::StorageEntry;

        let mut findings = Vec::new();
        let mut checked = 0usize;

        for (name, storage) in &ctx.config.storage {
            let StorageEntry::Btrfs { path, .. } = storage else {
                continue;
            };
            checked = checked.saturating_add(1usize);

            let Some(servers) = entries(&PathBuf::from(path).join(PRIMARY_DIR)).await? else {
                continue;
            };

            for (entry, target, is_dir) in servers {
                if !is_dir || entry.parse::<u32>().is_err() {
                    continue;
                }
                subvolume_finding(
                    &mut findings,
                    &target,
                    &format!("Server storage {entry} on storage {name}"),
                )
                .await;
            }
        }

        if checked == 0usize {
            return Err(CheckError::Skipped(
                "No storage on this node is configured as btrfs".to_owned(),
            ));
        }

        Ok(findings)
    }
}

/// A btrfs subvolume is always inode 256 at its root, so a plain directory in
/// its place is visible from `stat` alone.
#[cfg(feature = "btrfs")]
pub(super) async fn subvolume_finding(
    findings: &mut Vec<IntegrityFinding>,
    path: &Path,
    label: &str,
) {
    use crate::storage::btrfs::sys::BTRFS_FIRST_FREE_OBJECTID;
    use std::os::unix::fs::MetadataExt as _;

    match fs::symlink_metadata(path).await {
        Err(err) => findings.push(
            IntegrityFinding::error(format!("{label} could not be inspected: {err}"))
                .with_subject(path.to_string_lossy().into_owned()),
        ),
        Ok(metadata) if metadata.ino() != u64::from(BTRFS_FIRST_FREE_OBJECTID) => findings.push(
            IntegrityFinding::error(format!(
                "{label} is a plain directory, not a subvolume of its own. It was most likely \
                 copied instead of snapshotted"
            ))
            .with_subject(path.to_string_lossy().into_owned()),
        ),
        Ok(_metadata) => {}
    }
}
