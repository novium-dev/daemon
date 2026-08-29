// SPDX-License-Identifier: AGPL-3.0-only
use super::{Check, CheckError, IntegrityContext};
use crate::config::{BackupEntry, StorageEntry};
use async_trait::async_trait;
use daemon_common::integrity::{IntegrityCategory, IntegrityFinding};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;
use tokio::fs;

pub(super) fn writable(path: &Path) -> bool {
    let Ok(raw) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: access only reads the nul terminated buffer we own and returns a status code
    unsafe { libc::access(raw.as_ptr(), libc::W_OK | libc::X_OK) == 0i32 }
}

async fn check_directory(label: &str, name: &str, path: &str) -> Vec<IntegrityFinding> {
    let target = Path::new(path);
    match fs::metadata(target).await {
        Err(err) => vec![
            IntegrityFinding::error(format!(
                "The {label} directory of {name} is unusable: {err}"
            ))
            .with_subject(path.to_owned()),
        ],
        Ok(metadata) if !metadata.is_dir() => vec![
            IntegrityFinding::error(format!(
                "The {label} path of {name} exists but is not a directory"
            ))
            .with_subject(path.to_owned()),
        ],
        Ok(_metadata) if !writable(target) => vec![
            IntegrityFinding::error(format!(
                "The {label} directory of {name} is not writable by the daemon"
            ))
            .with_subject(path.to_owned()),
        ],
        Ok(_metadata) => Vec::new(),
    }
}

pub struct ConfigPaths;

#[async_trait]
impl Check for ConfigPaths {
    fn id(&self) -> &'static str {
        "config.paths"
    }

    fn title(&self) -> &'static str {
        "Configured paths exist and are usable"
    }

    fn description(&self) -> &'static str {
        "Every storage, backup storage and bind mount in the daemon config must point at an \
         existing, writable directory. A storage configured as btrfs must really live on a btrfs \
         filesystem, with quotas enabled where the config asks for them."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Configuration
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let mut findings = Vec::new();

        for (name, storage) in &ctx.config.storage {
            findings.extend(check_directory("storage", name, storage.path()).await);
            match storage {
                StorageEntry::Directory { .. } => {}
                #[cfg(feature = "btrfs")]
                StorageEntry::Btrfs { path, quota, .. } => {
                    findings.extend(check_btrfs("storage", name, path, *quota));
                }
            }
        }

        for (name, backup) in &ctx.config.backup {
            findings.extend(check_directory("backup storage", name, backup.path()).await);
            match backup {
                BackupEntry::Archive { .. } => {}
                #[cfg(feature = "btrfs")]
                BackupEntry::Btrfs { path, quota, .. } => {
                    findings.extend(check_btrfs("backup storage", name, path, *quota));
                }
            }
        }

        for mount in &ctx.config.mounts {
            if !Path::new(&mount.src).exists() {
                findings.push(
                    IntegrityFinding::error(
                        "A configured bind mount source does not exist; servers using it will \
                         fail to start"
                            .to_owned(),
                    )
                    .with_subject(mount.src.clone())
                    .with_detail("destination", mount.dest.clone())
                    .with_detail("mode", mount.mode.clone()),
                );
            }
        }

        if ctx.config.backup.is_empty() {
            findings.push(IntegrityFinding::warning(
                "No backup storage is configured on this node".to_owned(),
            ));
        }

        Ok(findings)
    }
}

#[cfg(feature = "btrfs")]
fn check_btrfs(label: &str, name: &str, path: &str, quota: bool) -> Vec<IntegrityFinding> {
    use crate::storage::btrfs;
    use std::collections::HashMap;

    let mut findings = Vec::new();

    if let Err(err) = btrfs::get_subvol_id(std::fs::File::open(path)) {
        findings.push(
            IntegrityFinding::error(format!(
                "The {label} {name} is configured as btrfs, but the path does not answer btrfs \
                 ioctls: {err}"
            ))
            .with_subject(path.to_owned()),
        );
        return findings;
    }

    if quota {
        let mut quotas = HashMap::new();
        if let Err(err) = btrfs::search_quotas(std::fs::File::open(path), &mut quotas) {
            findings.push(
                IntegrityFinding::error(format!(
                    "The {label} {name} asks for quotas, but they could not be read: {err}. \
                     Enable them with `btrfs quota enable`"
                ))
                .with_subject(path.to_owned()),
            );
        }
    }

    findings
}
