// SPDX-License-Identifier: AGPL-3.0-only
use super::storage::{PRIMARY_DIR, entries, known_servers, subvolume_finding};
use super::{Check, CheckError, IntegrityContext};
use crate::config::BackupEntry;
use crate::storage::archive::compression::ArchiveCompression;
use crate::storage::archive::format::ArchiveFormat;
use async_trait::async_trait;
use daemon_common::integrity::{IntegrityCategory, IntegrityFinding};
use std::io::{self, BufReader, Read as _};
use std::path::{Path, PathBuf};

const TAIL_LEN: usize = 1024;
const CHUNK_LEN: usize = 0x0001_0000;

struct ServerBackups {
    storage: String,
    server: u32,
    dir: PathBuf,
}

async fn server_dirs(
    ctx: &IntegrityContext,
    archive_only: bool,
) -> Result<Vec<(ServerBackups, BackupEntry)>, CheckError> {
    let mut out = Vec::new();

    for (name, backup) in &ctx.config.backup {
        if archive_only && !matches!(backup, BackupEntry::Archive { .. }) {
            continue;
        }
        let root = PathBuf::from(backup.path()).join(PRIMARY_DIR);
        let Some(servers) = entries(&root).await? else {
            continue;
        };
        for (entry, path, is_dir) in servers {
            let Ok(server) = entry.parse::<u32>() else {
                continue;
            };
            if !is_dir {
                continue;
            }
            out.push((
                ServerBackups {
                    storage: name.clone(),
                    server,
                    dir: path,
                },
                backup.clone(),
            ));
        }
    }

    Ok(out)
}

pub struct BackupNaming;

#[async_trait]
impl Check for BackupNaming {
    fn id(&self) -> &'static str {
        "backup.naming"
    }

    fn title(&self) -> &'static str {
        "Backup folders are named after live servers"
    }

    fn description(&self) -> &'static str {
        "A backup storage may only hold the primary directory with one numerically named folder \
         per server, and each of those may only hold backups named after their timestamp. \
         Backups of a server that no longer exists are never cleaned up on their own."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Backup
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let known = known_servers(ctx).await?;
        let mut findings = Vec::new();

        for (name, backup) in &ctx.config.backup {
            let root = PathBuf::from(backup.path());
            let Some(top) = entries(&root).await? else {
                continue;
            };

            for (entry, path, _is_dir) in top {
                if entry != PRIMARY_DIR {
                    findings.push(
                        IntegrityFinding::warning(format!(
                            "Backup storage {name} holds an entry the daemon never creates"
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
                let Ok(server) = entry.parse::<u32>() else {
                    findings.push(
                        IntegrityFinding::warning(format!(
                            "Backup storage {name} holds an entry that is not named after a \
                             server id"
                        ))
                        .with_subject(subject)
                        .with_detail("storage", name.clone()),
                    );
                    continue;
                };
                if !is_dir {
                    findings.push(
                        IntegrityFinding::warning(format!(
                            "Backup storage {name} holds a file where only server directories \
                             belong"
                        ))
                        .with_subject(subject)
                        .with_detail("storage", name.clone()),
                    );
                    continue;
                }
                if !known.contains(&server) {
                    findings.push(
                        IntegrityFinding::warning(format!(
                            "Backup storage {name} still holds the backups of server {server}, \
                             which no longer exists in the daemon database"
                        ))
                        .with_subject(subject)
                        .with_detail("storage", name.clone())
                        .with_detail("server", server.to_string()),
                    );
                    continue;
                }

                check_backup_names(&mut findings, name, server, &path, backup).await?;
            }
        }

        Ok(findings)
    }
}

async fn check_backup_names(
    findings: &mut Vec<IntegrityFinding>,
    storage: &str,
    server: u32,
    dir: &Path,
    backup: &BackupEntry,
) -> Result<(), CheckError> {
    let Some(backups) = entries(dir).await? else {
        return Ok(());
    };

    for (entry, path, is_dir) in backups {
        let subject = path.to_string_lossy().into_owned();
        let named = match backup {
            BackupEntry::Archive { compression, .. } => {
                entry
                    .strip_suffix(&format!(".tar{}", compression.ext()))
                    .is_some_and(|stem| stem.parse::<i64>().is_ok())
                    && !is_dir
            }
            #[cfg(feature = "btrfs")]
            BackupEntry::Btrfs { .. } => entry.parse::<i64>().is_ok() && is_dir,
        };
        if !named {
            findings.push(
                IntegrityFinding::warning(format!(
                    "Backup storage {storage} holds an entry for server {server} that does not \
                     follow the daemon's backup naming; it is invisible to the panel"
                ))
                .with_subject(subject)
                .with_detail("storage", storage.to_owned())
                .with_detail("server", server.to_string()),
            );
        }
    }

    Ok(())
}

pub struct BackupCompression;

#[async_trait]
impl Check for BackupCompression {
    fn id(&self) -> &'static str {
        "backup.compression"
    }

    fn title(&self) -> &'static str {
        "Archive backups match the configured compression"
    }

    fn description(&self) -> &'static str {
        "Every archive backup must carry the extension of the compression its storage is \
         configured with, and the file's own magic bytes must agree. Backups written before the \
         compression was changed are no longer listed and can never be restored."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Backup
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let dirs = server_dirs(ctx, true).await?;
        let mut findings = Vec::new();

        for (target, backup) in dirs {
            let BackupEntry::Archive { compression, .. } = backup else {
                continue;
            };
            let expected = format!(".tar{}", compression.ext());
            let Some(backups) = entries(&target.dir).await? else {
                continue;
            };

            for (entry, path, is_dir) in backups {
                if is_dir {
                    continue;
                }
                let subject = path.to_string_lossy().into_owned();

                if !entry.ends_with(&expected) {
                    findings.push(
                        IntegrityFinding::error(format!(
                            "Backup of server {} in storage {} is not stored with the configured \
                             compression ({expected}); it will never be listed or restored",
                            target.server, target.storage
                        ))
                        .with_subject(subject.clone())
                        .with_detail("storage", target.storage.clone())
                        .with_detail("server", target.server.to_string())
                        .with_detail("expected", expected.clone()),
                    );
                    continue;
                }

                let probe = path.clone();
                let detected = tokio::task::spawn_blocking(move || {
                    ArchiveFormat::detect_blocking(BufReader::new(std::fs::File::open(probe)?))
                })
                .await
                .map_err(io::Error::from)??;

                match detected {
                    Some(ArchiveFormat::Tar(actual)) if actual.ext() == compression.ext() => {}
                    Some(ArchiveFormat::Tar(actual)) => findings.push(
                        IntegrityFinding::error(format!(
                            "Backup of server {} in storage {} is named {expected} but is \
                             actually compressed with {}",
                            target.server,
                            target.storage,
                            display_ext(actual.ext())
                        ))
                        .with_subject(subject)
                        .with_detail("storage", target.storage.clone())
                        .with_detail("server", target.server.to_string())
                        .with_detail("detected", display_ext(actual.ext())),
                    ),
                    Some(ArchiveFormat::Zip) | None => findings.push(
                        IntegrityFinding::error(format!(
                            "Backup of server {} in storage {} is not a tar archive the daemon \
                             can read",
                            target.server, target.storage
                        ))
                        .with_subject(subject)
                        .with_detail("storage", target.storage.clone())
                        .with_detail("server", target.server.to_string()),
                    ),
                }
            }
        }

        Ok(findings)
    }
}

fn display_ext(ext: &str) -> String {
    if ext.is_empty() {
        "no compression".to_owned()
    } else {
        ext.to_owned()
    }
}

pub struct BackupTrailer;

#[async_trait]
impl Check for BackupTrailer {
    fn id(&self) -> &'static str {
        "backup.trailer"
    }

    fn title(&self) -> &'static str {
        "Archive backups end with a complete tar trailer"
    }

    fn description(&self) -> &'static str {
        "A finished tar ends with two zeroed 512 byte blocks. A backup that stops short was \
         interrupted or truncated and will fail halfway through a restore. This reads every \
         archive backup in full."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Backup
    }

    fn deep(&self) -> bool {
        true
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let dirs = server_dirs(ctx, true).await?;
        let mut findings = Vec::new();

        for (target, backup) in dirs {
            let BackupEntry::Archive { compression, .. } = backup else {
                continue;
            };
            let expected = format!(".tar{}", compression.ext());
            let Some(backups) = entries(&target.dir).await? else {
                continue;
            };

            for (entry, path, is_dir) in backups {
                if is_dir || !entry.ends_with(&expected) {
                    continue;
                }
                let subject = path.to_string_lossy().into_owned();
                let probe = path.clone();
                let outcome =
                    tokio::task::spawn_blocking(move || verify_trailer(&probe, compression))
                        .await
                        .map_err(io::Error::from)?;

                match outcome {
                    Ok(true) => {}
                    Ok(false) => findings.push(
                        IntegrityFinding::error(format!(
                            "Backup of server {} in storage {} has no tar end of archive marker; \
                             it is truncated",
                            target.server, target.storage
                        ))
                        .with_subject(subject)
                        .with_detail("storage", target.storage.clone())
                        .with_detail("server", target.server.to_string()),
                    ),
                    Err(err) => findings.push(
                        IntegrityFinding::error(format!(
                            "Backup of server {} in storage {} could not be read to its end: \
                             {err}",
                            target.server, target.storage
                        ))
                        .with_subject(subject)
                        .with_detail("storage", target.storage.clone())
                        .with_detail("server", target.server.to_string()),
                    ),
                }
            }
        }

        Ok(findings)
    }
}

fn verify_trailer(path: &Path, compression: ArchiveCompression) -> Result<bool, io::Error> {
    let file = BufReader::new(std::fs::File::open(path)?);
    let mut reader = compression.create_decoder(file)?;

    let mut chunk = vec![0u8; CHUNK_LEN];
    let mut tail: Vec<u8> = Vec::with_capacity(TAIL_LEN);
    let mut total = 0u64;

    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0usize {
            break;
        }
        total = total.saturating_add(read as u64);
        let Some(slice) = chunk.get(..read) else {
            return Err(io::Error::other("short read outside of the buffer"));
        };
        tail.extend_from_slice(slice);
        let excess = tail.len().saturating_sub(TAIL_LEN);
        if excess > 0usize {
            tail.drain(..excess);
        }
    }

    if tail.len() < TAIL_LEN || total.checked_rem(512u64) != Some(0u64) {
        return Ok(false);
    }

    Ok(tail.iter().all(|byte| *byte == 0u8))
}

pub struct BackupSubvolume;

#[async_trait]
impl Check for BackupSubvolume {
    fn id(&self) -> &'static str {
        "backup.subvolume"
    }

    fn title(&self) -> &'static str {
        "Backups on btrfs are subvolumes"
    }

    fn description(&self) -> &'static str {
        "Snapshot backups are created as btrfs subvolumes. A plain directory in their place means \
         the backup was copied with cp instead of snapshotted, so its size cannot be accounted \
         for and restoring from it does not behave like a snapshot."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Backup
    }

    #[cfg(not(feature = "btrfs"))]
    async fn run(&self, _ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        Err(CheckError::Skipped(
            "This daemon was built without btrfs support".to_owned(),
        ))
    }

    #[cfg(feature = "btrfs")]
    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let mut findings = Vec::new();
        let mut checked = 0usize;

        for (name, backup) in &ctx.config.backup {
            let BackupEntry::Btrfs { path, .. } = backup else {
                continue;
            };
            checked = checked.saturating_add(1usize);

            let root = PathBuf::from(path).join(PRIMARY_DIR);
            let Some(servers) = entries(&root).await? else {
                continue;
            };

            for (entry, server_dir, is_dir) in servers {
                let Ok(server) = entry.parse::<u32>() else {
                    continue;
                };
                if !is_dir {
                    continue;
                }
                let Some(backups) = entries(&server_dir).await? else {
                    continue;
                };
                for (backup_entry, target, backup_is_dir) in backups {
                    if !backup_is_dir || backup_entry.parse::<i64>().is_err() {
                        continue;
                    }
                    subvolume_finding(
                        &mut findings,
                        &target,
                        &format!("Backup {backup_entry} of server {server} in storage {name}"),
                    )
                    .await;
                }
            }
        }

        if checked == 0usize {
            return Err(CheckError::Skipped(
                "No backup storage on this node is configured as btrfs".to_owned(),
            ));
        }

        Ok(findings)
    }
}
