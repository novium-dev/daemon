// SPDX-License-Identifier: AGPL-3.0-only
use crate::config::StorageEntry;
use crate::server::FetchedServer;
use crate::server::status::InternalServerState;
use crate::server::stop::ServerStopError;
use crate::storage::StorageIdentifier;
use crate::storage::scan::ScanError;
use crate::utils::ChannelWriteData;
use crate::{Daemon, DaemonExtra, StorageData};
use core::sync::atomic::{AtomicI64, Ordering};
use core::time::Duration;
use daemon_common::service::server::ServerStats;
use daemon_common::service::server::task::ServerTask;
use rayon::iter::{IntoParallelRefIterator as _, ParallelIterator as _};
use sqlx::query;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use std::{io, thread};
use tokio::runtime::Handle;
use tokio::sync::broadcast::Sender;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tracing::{error, warn};

pub struct ResolvedStorage {
    server_id: Option<u32>,
    pub path: PathBuf,
    cpu_max: Option<f32>,
    ram_max: Option<u32>,
    swap_max: Option<u32>,
    data: StorageData,
    sender: Option<Arc<Sender<ChannelWriteData>>>,
}

pub struct ApplyQuotaToken<'a>(PhantomData<fn(&'a ()) -> &'a ()>);

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
) -> Option<HashMap<StorageIdentifier, ResolvedStorage>> {
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
    for (id, (cpu_max, ram_max, swap_max, disk_max)) in servers {
        let identifier = StorageIdentifier::Primary(id);
        let path = Path::new(&storage_entry.path).join(identifier.as_pathbuf());

        if !tokio::fs::try_exists(&path).await.unwrap_or(false) {
            continue;
        }
        let mut storages = daemon.storages.lock().await; // acquire lock multiple times, because the try_exists probably takes some time
        let data = storages
            .entry(identifier)
            .or_insert_with(|| StorageData {
                lock: Arc::default(),
                stats_lock: Arc::default(),
                value: Arc::new((
                    AtomicI64::new(-1),
                    AtomicI64::new(
                        disk_max
                            .filter(|i| *i <= i64::MAX.unsigned_abs())
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
            ResolvedStorage {
                server_id: Some(id),
                path,
                cpu_max,
                ram_max,
                swap_max,
                data,
                sender,
            },
        );
    }

    Some(available)
}

pub fn watch(daemon: &Daemon<impl DaemonExtra>, storage_id: &str, storage_entry: &StorageEntry) {
    let mut last = SystemTime::UNIX_EPOCH;
    while !daemon.shutdown_token.is_cancelled() {
        let Some(storages) =
            Handle::current().block_on(resolve(daemon, storage_id.to_owned(), storage_entry))
        else {
            thread::sleep(Duration::from_secs(30));
            continue;
        };

        let quota = matches!(
            storage_entry
                .backend
                .quota_usage(Path::new(&storage_entry.path), &storages)
                .inspect_err(|err| error!("Unable to retrieve quotas: {err:?}")),
            Err(_) | Ok(Some(_))
        );

        if !quota && last.elapsed().unwrap_or_default() > crate::common::STORAGE_SCAN_INTERVAL {
            last = SystemTime::now();
            let sizes = super::scan::scan_all(
                &storages
                    .iter()
                    .map(|(id, ResolvedStorage { path, data, .. })| {
                        (*id, path.clone(), Arc::clone(&data.lock))
                    })
                    .collect::<Vec<_>>(),
            );
            for (id, result) in sizes {
                let Some(ResolvedStorage {
                    server_id, data, ..
                }) = storages.get(&id)
                else {
                    continue;
                };
                match result {
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
                    Err(ScanError::Interrupted) => {
                        // Storage is used exclusively, retry on the next scan
                    }
                    Err(ScanError::IO(err)) => {
                        warn!("Failed to scan directory size for {id:?}: {err:?}");
                    }
                }
            }
        }

        let mut join = JoinSet::new();
        for (
            _id,
            ResolvedStorage {
                server_id,
                cpu_max,
                ram_max,
                swap_max,
                data,
                sender,
                ..
            },
        ) in storages
        {
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

pub fn apply_quotas<
    'a,
    F: Fn(StorageIdentifier, &'a ResolvedStorage) -> Result<u64, io::Error> + Send + Sync,
>(
    storages: &'a HashMap<StorageIdentifier, ResolvedStorage>,
    get: F,
) -> ApplyQuotaToken<'a> {
    storages
        .par_iter()
        .for_each(|(id, storage)| match get(*id, storage) {
            Ok(size) => {
                let size = i64::try_from(size).unwrap_or(-1);
                storage.data.value.0.store(size, Ordering::Relaxed);
            }
            Err(err) => {
                warn!("Unable to get quota for {id:?}: {err:?}");
            }
        });

    ApplyQuotaToken(PhantomData)
}
