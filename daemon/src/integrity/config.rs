// SPDX-License-Identifier: AGPL-3.0-only
use super::{Check, CheckError, IntegrityContext};
use crate::config::{BackupEntry, StorageEntry};
use async_trait::async_trait;
use daemon_common::integrity::{IntegrityCategory, IntegrityFinding};
use rustix::fs::{Access, access};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use tokio::fs;

pub(super) async fn writable(path: &Path) -> bool {
    let owned = path.to_path_buf();
    tokio::task::spawn_blocking(move || access(&owned, Access::WRITE_OK | Access::EXEC_OK).is_ok())
        .await
        .unwrap_or(false)
}

async fn nearest_existing(target: &Path) -> Option<PathBuf> {
    for ancestor in target.ancestors().skip(1usize) {
        if ancestor.as_os_str().is_empty() {
            continue;
        }
        if let Ok(metadata) = fs::symlink_metadata(ancestor).await {
            return metadata.is_dir().then(|| ancestor.to_path_buf());
        }
    }
    None
}

async fn check_directory(
    findings: &mut Vec<IntegrityFinding>,
    label: &str,
    name: &str,
    path: &str,
) {
    let target = Path::new(path);
    match fs::symlink_metadata(target).await {
        Err(err) if err.kind() == ErrorKind::NotFound => match nearest_existing(target).await {
            Some(parent) if writable(&parent).await => {}
            _ => findings.push(
                IntegrityFinding::error(format!(
                    "The {label} directory of {name} does not exist and cannot be created; no \
                         parent of it is a directory the daemon may write to"
                ))
                .with_subject(path),
            ),
        },
        Err(err) => findings.push(
            IntegrityFinding::error(format!(
                "The {label} directory of {name} is unusable: {err}"
            ))
            .with_subject(path),
        ),
        Ok(metadata) if !metadata.is_dir() => findings.push(
            IntegrityFinding::error(format!(
                "The {label} path of {name} exists but is not a directory"
            ))
            .with_subject(path),
        ),
        Ok(_metadata) => {
            if !writable(target).await {
                findings.push(
                    IntegrityFinding::error(format!(
                        "The {label} directory of {name} is not writable by the daemon"
                    ))
                    .with_subject(path),
                );
            }
        }
    }
}

pub struct ConfigPaths;

#[async_trait]
impl Check for ConfigPaths {
    fn id(&self) -> &'static str {
        "config.paths"
    }

    fn title(&self) -> &'static str {
        "Configured paths are usable"
    }

    fn description(&self) -> &'static str {
        "Storage and backup directories are created on first use, so they may still be missing; \
         what matters is that the daemon could create them and write to them. A storage \
         configured as btrfs must really live on a btrfs filesystem, with quotas enabled where \
         the config asks for them, one configured as ext4 on ext4 with project quotas \
         enabled, and every configured bind mount source must already exist."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Configuration
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let mut findings = Vec::new();

        for (name, storage) in &ctx.config.storage {
            check_directory(&mut findings, "storage", name, storage.path()).await;
            match storage {
                StorageEntry::Directory { .. } => {}
                #[cfg(feature = "btrfs")]
                StorageEntry::Btrfs { path, quota, .. } => {
                    check_btrfs(&mut findings, "storage", name, path, *quota).await;
                }
                #[cfg(target_os = "linux")]
                StorageEntry::Ext4 { path } => {
                    check_ext4(&mut findings, name, path).await;
                }
            }
        }

        for (name, backup) in &ctx.config.backup {
            check_directory(&mut findings, "backup storage", name, backup.path()).await;
            match backup {
                BackupEntry::Archive { .. } => {}
                #[cfg(feature = "btrfs")]
                BackupEntry::Btrfs { path, quota, .. } => {
                    check_btrfs(&mut findings, "backup storage", name, path, *quota).await;
                }
            }
        }

        for mount in &ctx.config.mounts {
            if fs::symlink_metadata(&mount.src).await.is_err() {
                findings.push(
                    IntegrityFinding::error(
                        "A configured bind mount source does not exist; servers using it will \
                         fail to start",
                    )
                    .with_subject(mount.src.clone())
                    .with_detail("destination", mount.dest.clone())
                    .with_detail("mode", mount.mode.clone()),
                );
            }
        }

        if ctx.config.backup.is_empty() {
            findings.push(IntegrityFinding::warning(
                "No backup storage is configured on this node",
            ));
        }

        Ok(findings)
    }
}

#[cfg(target_os = "linux")]
async fn check_ext4(findings: &mut Vec<IntegrityFinding>, name: &str, path: &str) {
    use crate::storage::ext4;

    if !fs::symlink_metadata(path)
        .await
        .is_ok_and(|metadata| metadata.is_dir())
    {
        return;
    }

    let owned = PathBuf::from(path);
    let problem = match tokio::task::spawn_blocking(move || ext4::check(&owned)).await {
        Ok(Ok(())) => return,
        Ok(Err(problem)) => problem,
        Err(err) => format!("could not be probed: {err}"),
    };
    findings.push(
        IntegrityFinding::error(format!(
            "The storage {name} is configured as ext4, but {problem}"
        ))
        .with_subject(path),
    );
}

#[cfg(feature = "btrfs")]
async fn check_btrfs(
    findings: &mut Vec<IntegrityFinding>,
    label: &str,
    name: &str,
    path: &str,
    quota: bool,
) {
    use crate::storage::btrfs;
    use std::collections::HashMap;

    if !fs::symlink_metadata(path)
        .await
        .is_ok_and(|metadata| metadata.is_dir())
    {
        return;
    }

    let owned = path.to_owned();
    let probe = tokio::task::spawn_blocking(move || {
        btrfs::get_subvol_id(std::fs::File::open(&owned)).map(|_id| ())
    })
    .await;

    match probe {
        Ok(Err(err)) => {
            findings.push(
                IntegrityFinding::error(format!(
                    "The {label} {name} is configured as btrfs, but the path does not answer \
                     btrfs ioctls: {err}"
                ))
                .with_subject(path),
            );
            return;
        }
        Err(err) => {
            findings.push(
                IntegrityFinding::error(format!(
                    "The {label} {name} could not be probed for btrfs: {err}"
                ))
                .with_subject(path),
            );
            return;
        }
        Ok(Ok(())) => {}
    }

    if quota {
        let owned = path.to_owned();
        let quotas = tokio::task::spawn_blocking(move || {
            let mut quotas = HashMap::new();
            btrfs::search_quotas(std::fs::File::open(&owned), &mut quotas).map(|()| quotas)
        })
        .await;

        if let Ok(Err(err)) = quotas {
            findings.push(
                IntegrityFinding::error(format!(
                    "The {label} {name} asks for quotas, but they could not be read: {err}. \
                     Enable them with `btrfs quota enable`"
                ))
                .with_subject(path),
            );
        }
    }
}
