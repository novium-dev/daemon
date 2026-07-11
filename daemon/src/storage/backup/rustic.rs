// SPDX-License-Identifier: AGPL-3.0-only
//! Integration of deduplicated backups via the [`rustic_core`] library.
//!
//! Each server gets its own rustic repository located at `<backup.path>/<storage id>`.
//! Every backup is stored as a snapshot inside that repository, which allows rustic to
//! deduplicate data across all backups of the same server. Because the daemon identifies
//! backups by a millisecond timestamp, that timestamp is stored as a tag on the snapshot
//! (see [`ID_TAG_PREFIX`]) so a backup can later be looked up, renamed, restored or deleted.

use crate::storage::archive::TarSandboxedAppendExt as _;
use crate::storage::archive::compression::ArchiveCompression;
use daemon_common::service::DataStream;
use rustic_backend::BackendOptions;
use rustic_core::repofile::SnapshotFile;
use rustic_core::{
    BackupOptions, ConfigOptions, Credentials, KeyOptions, LocalDestination, LsOptions, PathList,
    PruneOptions, Repository, RepositoryBackends, RepositoryOptions, RestoreOptions,
    SnapshotOptions,
};
use std::collections::HashSet;
use std::io;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use tokio_util::io::SyncIoBridge;
use tracing::error;

/// Prefix of the snapshot tag that stores the daemon backup id (a millisecond timestamp).
const ID_TAG_PREFIX: &str = "daemon-id=";

/// Metadata describing a single rustic snapshot in a form understood by the daemon.
pub struct RusticSnapshotInfo {
    pub time: i64,
    pub size: Option<u64>,
    pub name: Option<String>,
}

fn rustic_err(err: impl core::fmt::Display) -> io::Error {
    io::Error::other(format!("rustic error: {err}"))
}

fn to_str(path: &Path) -> Result<&str, io::Error> {
    path.to_str()
        .ok_or_else(|| io::Error::other("path is not valid UTF-8"))
}

fn backends(repo_path: &Path) -> Result<RepositoryBackends, io::Error> {
    BackendOptions::default()
        .repository(to_str(repo_path)?)
        .to_backends()
        .map_err(rustic_err)
}

/// A rustic repository stores its config in a `config` file at its root once initialized.
fn is_initialized(repo_path: &Path) -> bool {
    repo_path.join("config").exists()
}

/// Reads the daemon backup id that was stored as a tag on the snapshot, if present.
fn snapshot_id(snapshot: &SnapshotFile) -> Option<i64> {
    snapshot
        .tags
        .iter()
        .find_map(|tag| tag.strip_prefix(ID_TAG_PREFIX))
        .and_then(|id| id.parse().ok())
}

/// The logical size of the files contained in the snapshot (matches restore size).
fn snapshot_size(snapshot: &SnapshotFile) -> Option<u64> {
    snapshot
        .summary
        .as_ref()
        .map(|summary| summary.total_bytes_processed)
}

/// Creates a new snapshot of `source` inside the server's rustic repository, initializing
/// the repository if it does not exist yet. Returns the size backed up by the snapshot.
///
/// Blocking: run inside [`tokio::task::spawn_blocking`].
pub fn create(
    repo_path: &Path,
    password: String,
    source: &Path,
    backup_id: i64,
    name: Option<String>,
) -> Result<Option<u64>, io::Error> {
    let backends = backends(repo_path)?;
    let credentials = Credentials::password(password);
    let repo = Repository::new(&RepositoryOptions::default(), &backends).map_err(rustic_err)?;
    let repo = if is_initialized(repo_path) {
        repo.open(&credentials).map_err(rustic_err)?
    } else {
        repo.init(
            &credentials,
            &KeyOptions::default(),
            &ConfigOptions::default(),
        )
        .map_err(rustic_err)?
    }
    .to_indexed_ids()
    .map_err(rustic_err)?;

    let source = PathList::from_string(to_str(source)?)
        .map_err(rustic_err)?
        .sanitize()
        .map_err(rustic_err)?;
    let snapshot = SnapshotOptions::default()
        .add_tags(&format!("{ID_TAG_PREFIX}{backup_id}"))
        .map_err(rustic_err)?
        .description(name)
        .to_snapshot()
        .map_err(rustic_err)?;

    let snapshot = repo
        .backup(&BackupOptions::default(), &source, snapshot)
        .map_err(rustic_err)?;
    Ok(snapshot_size(&snapshot))
}

/// Lists all snapshots of the server's rustic repository. Returns an empty list if the
/// repository has not been initialized yet.
///
/// Blocking: run inside [`tokio::task::spawn_blocking`].
pub fn list(repo_path: &Path, password: String) -> Result<Vec<RusticSnapshotInfo>, io::Error> {
    if !is_initialized(repo_path) {
        return Ok(Vec::new());
    }
    let repo = open(repo_path, password)?;
    let snapshots = repo.get_all_snapshots().map_err(rustic_err)?;
    Ok(snapshots
        .into_iter()
        .filter_map(|snapshot| {
            snapshot_id(&snapshot).map(|time| RusticSnapshotInfo {
                time,
                size: snapshot_size(&snapshot),
                name: snapshot.description.clone(),
            })
        })
        .collect())
}

/// Deletes the snapshot with the given backup id and prunes the repository to reclaim space.
///
/// Blocking: run inside [`tokio::task::spawn_blocking`].
pub fn delete(repo_path: &Path, password: String, backup_id: i64) -> Result<(), io::Error> {
    let repo = open(repo_path, password)?;
    let snapshot = find(repo.get_all_snapshots().map_err(rustic_err)?, backup_id)?;
    repo.delete_snapshots(&[snapshot.id]).map_err(rustic_err)?;

    let prune_options = PruneOptions::default();
    let plan = repo.prune_plan(&prune_options).map_err(rustic_err)?;
    repo.prune(&prune_options, plan).map_err(rustic_err)?;
    Ok(())
}

/// Sets (or clears with `None`) the human readable name of the snapshot. The name is stored
/// in the snapshot description; because the daemon identifies the backup by its id tag,
/// rewriting the snapshot with an updated description keeps the backup id stable.
///
/// Blocking: run inside [`tokio::task::spawn_blocking`].
pub fn set_name(
    repo_path: &Path,
    password: String,
    backup_id: i64,
    name: Option<String>,
) -> Result<(), io::Error> {
    let repo = open(repo_path, password)?;
    let mut snapshot = find(repo.get_all_snapshots().map_err(rustic_err)?, backup_id)?;
    let old_id = snapshot.id;
    snapshot.description = name;
    // The id of a snapshot is derived from its content, so saving produces a new snapshot;
    // replace the old one to avoid duplicates.
    repo.save_snapshots(vec![snapshot]).map_err(rustic_err)?;
    repo.delete_snapshots(&[old_id]).map_err(rustic_err)?;
    Ok(())
}

/// Restores the snapshot with the given backup id into `destination`.
///
/// Blocking: run inside [`tokio::task::spawn_blocking`].
pub fn restore(
    repo_path: &Path,
    password: String,
    backup_id: i64,
    destination: &Path,
) -> Result<(), io::Error> {
    let repo = Repository::new(&RepositoryOptions::default(), &backends(repo_path)?)
        .map_err(rustic_err)?
        .open(&Credentials::password(password))
        .map_err(rustic_err)?
        .to_indexed()
        .map_err(rustic_err)?;

    let snapshot = find(repo.get_all_snapshots().map_err(rustic_err)?, backup_id)?;
    let node = repo
        .node_from_snapshot_path(&snapshot.id.to_string(), |_| true)
        .map_err(rustic_err)?;
    let entries = repo.ls(&node, &LsOptions::default()).map_err(rustic_err)?;

    let dest =
        LocalDestination::new(to_str(destination)?, true, !node.is_dir()).map_err(rustic_err)?;
    let options = RestoreOptions::default();
    let plan = repo
        .prepare_restore(&options, entries.clone(), &dest, false)
        .map_err(rustic_err)?;
    repo.restore(plan, &options, entries, &dest)
        .map_err(rustic_err)?;
    Ok(())
}

fn open(
    repo_path: &Path,
    password: String,
) -> Result<Repository<rustic_core::OpenStatus>, io::Error> {
    Repository::new(&RepositoryOptions::default(), &backends(repo_path)?)
        .map_err(rustic_err)?
        .open(&Credentials::password(password))
        .map_err(rustic_err)
}

fn find(snapshots: Vec<SnapshotFile>, backup_id: i64) -> Result<SnapshotFile, io::Error> {
    snapshots
        .into_iter()
        .find(|snapshot| snapshot_id(snapshot) == Some(backup_id))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "backup not found"))
}

/// Streams the contents of `directory` as an uncompressed tar and removes the directory once
/// the stream has been fully produced. Used to serve a rustic backup that was first restored
/// into a temporary directory.
///
/// Cancel safe
#[expect(
    clippy::unused_async,
    reason = "ensure it only gets called in an async context"
)]
pub async fn tar_stream_and_cleanup(directory: PathBuf) -> DataStream {
    let (sender, receiver) = tokio::io::duplex(1024 * 1024);
    tokio::task::spawn_blocking(move || {
        let result = (|| {
            let encoder = ArchiveCompression::None.create_encoder(SyncIoBridge::new(sender))?;
            let mut archive = tar::Builder::new(encoder);
            archive.follow_symlinks(false);
            let (archive, ()) = archive.append_dir_all_sandboxed_blocking(
                &directory,
                "",
                "/",
                HashSet::new(),
                (),
                true,
                false,
            )?;
            archive.into_inner()?.finish()?.flush()?;
            Ok::<(), io::Error>(())
        })();
        if let Err(err) = &result {
            error!("Failed to stream rustic backup (tar): {err:?}");
        }
        if let Err(err) = std::fs::remove_dir_all(&directory) {
            error!("Failed to clean up rustic restore directory: {err:?}");
        }
    });
    DataStream::by_read(receiver)
}
