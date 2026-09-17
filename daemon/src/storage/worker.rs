// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::StorageEntry;
use crate::server::FetchedServer;
use crate::server::status::InternalServerState;
use crate::server::stop::ServerStopError;
use crate::storage::StorageIdentifier;
use crate::utils::ChannelWriteData;
use crate::{Daemon, DaemonExtra, StorageData};
use core::sync::atomic::{AtomicI64, Ordering};
use core::time::Duration;
use daemon_common::service::files::task::StorageTask;
use daemon_common::service::server::ServerStats;
use daemon_common::service::server::task::ServerTask;
use sqlx::query;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::SystemTime;
use tokio::runtime::Handle;
use tokio::sync::broadcast::Sender;
use tokio::sync::broadcast::error::TryRecvError;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tracing::warn;

pub async fn run(daemon: Daemon<impl DaemonExtra>) {
    for (storage_id, storage_entry) in daemon.config.storage.clone() {
        let mut server_task = daemon.server_task_cancellable.lock().await;
        let daemon = daemon.clone();
        server_task.spawn_blocking(move || watch(&daemon, &storage_id, &storage_entry));
    }
}

async fn resolve(
    daemon: &Daemon<impl DaemonExtra>,
    storage_id: String,
    storage_entry: &StorageEntry,
) -> Option<
    HashMap<
        StorageIdentifier,
        (
            Option<u32>,
            PathBuf,
            Option<f32>,
            Option<u32>,
            Option<u32>,
            StorageData,
            Option<Arc<Sender<ChannelWriteData>>>,
        ),
    >,
> {
    type ResourceInfo = (Option<f32>, Option<u32>, Option<u32>, Option<u64>);
    let servers: HashMap<u32, ResourceInfo> = match query!(
        "SELECT `id`, `cpu`, `ram`, `swap`, `disk` FROM `servers` WHERE `storage` = ?",
        &storage_id
    )
    .fetch_all(&*daemon.conn)
    .await
    {
        Ok(servers) => servers
            .into_iter()
            .map(|row| (row.id, (row.cpu, row.ram, row.swap, row.disk)))
            .collect(),
        Err(err) => {
            warn!("Failed to retrieve servers list: {:?}", err);
            tokio::time::sleep(Duration::from_secs(30)).await;
            return None;
        }
    };
    let mut available = HashMap::new();
    for (id, (cpu, ram, swap, disk)) in servers {
        let identifier = StorageIdentifier::Primary(id);
        let path = PathBuf::from(storage_entry.path()).join(identifier.as_pathbuf());

        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            continue;
        }
        let mut storages = daemon.storages.lock().await; // acquire lock multiple times, because the try_exists probably takes some time
        let storage_data = storages
            .entry(identifier)
            .or_insert_with(|| StorageData {
                lock: Arc::default(),
                stats_lock: Arc::default(),
                value: Arc::new((
                    AtomicI64::new(-1),
                    AtomicI64::new(
                        disk.filter(|i| *i <= i64::MAX.unsigned_abs())
                            .map_or(-1, u64::cast_signed),
                    ),
                )),
                notify: Sender::new(32),
                tokens: Arc::default(),
            })
            .clone();
        drop(storages);
        let servers = daemon.servers.lock().await;
        let server = servers.get(&id).cloned();
        drop(servers);
        let sender = server.map_or_else(|| None, |server| Some(Arc::clone(&server.stats_sender)));
        available.insert(
            identifier,
            (Some(id), path, cpu, ram, swap, storage_data, sender),
        );
    }

    Some(available)
}

pub fn watch(daemon: &Daemon<impl DaemonExtra>, storage_id: &str, storage_entry: &StorageEntry) {
    let mut last = SystemTime::UNIX_EPOCH;
    let mut recv = daemon.shutdown_channel.subscribe();
    while recv.try_recv() == Err(TryRecvError::Empty) {
        let Some(storages) =
            Handle::current().block_on(resolve(daemon, storage_id.to_owned(), storage_entry))
        else {
            thread::sleep(Duration::from_secs(30));
            continue;
        };

        #[cfg_attr(
            not(feature = "btrfs"),
            expect(unused_mut, reason = "only used currently with btrfs enabled")
        )]
        let mut quota = false;
        #[cfg(feature = "btrfs")]
        if let StorageEntry::Btrfs {
            quota: true, path, ..
        } = storage_entry
        {
            use std::fs::File;
            use tracing::error;
            quota = true;

            let mut quotas = HashMap::new();
            if let Err(err) = super::btrfs::search_quotas(File::open(path), &mut quotas) {
                error!("Unable to retrieve quotas: {err:?}. Ensure quotas are enabled");
            }

            for (id, (_server_id, path, _, _, _, data, _)) in &storages {
                let subvol = match super::btrfs::get_subvol_id(File::open(path)) {
                    Ok(id) => id,
                    Err(err) => {
                        warn!("Unable to get subvolume id of {id:?}: {err:?}");
                        continue;
                    }
                };
                let Some(size) = quotas.get(&subvol) else {
                    warn!("Quota missing for {id:?} (subvolid {subvol})");
                    warn!("Retrived quotas: {quotas:?}");
                    continue;
                };
                let size = i64::try_from(*size).unwrap_or(-1);
                data.value.0.store(size, Ordering::Relaxed);
            }
        }

        if !quota && last.elapsed().unwrap_or_default() > crate::common::STORAGE_SCAN_INTERVAL {
            last = SystemTime::now();
            for (id, (server_id, path, _, _, _, data, _)) in &storages {
                let Ok(_read_guard) = data.lock.try_read(StorageTask::Scan) else {
                    continue;
                };
                match fs_extra::dir::get_size(path) {
                    Ok(size_unsigned) => {
                        let size = if size_unsigned > i64::MAX.unsigned_abs() {
                            -1
                        } else {
                            size_unsigned.cast_signed()
                        };
                        data.value.0.store(size, Ordering::Relaxed);
                        let allowed = data.value.1.load(Ordering::Relaxed);
                        let exceeded = allowed != -1
                            && size != -1
                            && u128::from(size_unsigned)
                                > u128::from(allowed.unsigned_abs()) * 0x0010_0000; // MiB -> bytes
                        if let Some(server_id) = *server_id {
                            if exceeded {
                                if let Err(err) = Handle::current().block_on(query!("INSERT IGNORE INTO `server_locks` (`server`, `lock`) VALUES (?, 'storage')", server_id)
                                    .execute(&*daemon.conn)) {
                                    warn!("Unable to lock server {server_id}: {err:?}");
                                }
                                let servers = daemon.servers.blocking_lock();
                                if let Some(server_data) = servers.get(&server_id).cloned() {
                                    drop(servers);
                                    let docker = Arc::clone(&daemon.docker);
                                    tokio::task::spawn(async move {
                                        let lock = server_data.lock.read(ServerTask::Stop).await;
                                        if let Err(err) = FetchedServer::stop(&FetchedServer {
                                            id: server_id,
                                            .. FetchedServer::default()
                                        }, &docker, &server_data, &lock).await {
                                            if matches!(err, ServerStopError::NotRunning) {
                                                return;
                                            }
                                            warn!("Unable to stop server after lock {server_id}: {err:?}");
                                        }
                                    });
                                }
                            } else if let Err(err) = Handle::current().block_on(query!("DELETE FROM `server_locks` WHERE `server` = ? AND `lock` = 'storage'", server_id)
                                    .execute(&*daemon.conn)) {
                                    warn!("Unable to unlock server {server_id}: {err:?}");
                            } else {
                                // Sucessfully unlocked the server
                            }
                        }
                    }
                    Err(err) => {
                        warn!("Failed to scan directory size for {id:?}: {err:?}");
                    }
                }
            }
        }

        let mut join = JoinSet::new();
        for (_id, (server_id, _path, cpu_max, ram_max, swap_max, data, sender)) in storages {
            let conn = Arc::clone(&daemon.conn);
            join.spawn(async move {
                let stats_guard = timeout(Duration::from_millis(100), data.stats_lock.lock()).await;
                if stats_guard.is_ok()
                    && let Some(stats_sender) = sender
                {
                    if let Some(server_id) = server_id
                        && !quota
                    {
                        let locks = FetchedServer::resolve_locked(server_id, &conn).await;
                        let _ = stats_sender.send(ChannelWriteData::Status(if locks.is_empty() {
                            InternalServerState::Stopped
                        } else {
                            InternalServerState::Locked
                        }));
                    }
                    let disk = data.value.0.load(Ordering::Relaxed);
                    let disk_max = data.value.1.load(Ordering::Relaxed);
                    let _ = stats_sender.send(ChannelWriteData::Stats(
                        ServerStats::default()
                            .with_disk(
                                (disk != -1).then(|| crate::utils::to_mib(disk.unsigned_abs())),
                            )
                            .with_max(
                                cpu_max,
                                ram_max,
                                swap_max,
                                (disk_max != -1).then(|| disk_max.unsigned_abs()),
                            ),
                    ));
                }
            });
        }
        Handle::current().block_on(join.join_all());

        thread::sleep(Duration::from_secs(30));
    }
}
