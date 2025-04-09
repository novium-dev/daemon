use crate::config::StorageEntry;
use crate::server::FetchedServer;
use crate::server::status::InternalServerState;
use crate::server::stop::ServerStopError;
use crate::storage::StorageIdentifier;
use crate::utils::ChannelWriteData;
use crate::{Daemon, StorageData};
use daemon_common::server::ServerStats;
use sqlx::query;
use std::collections::HashMap;
use std::fs::File;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime};
use tokio::runtime::Handle;
use tokio::sync::broadcast::Sender;
use tokio::task::JoinSet;
use tokio::time::timeout;
use tracing::{error, warn};

pub(crate) async fn run(state: Daemon) {
    let state2 = state.clone();
    for (storage_id, storage_entry) in state.config.storage.clone() {
        let state = state2.clone();
        tokio::task::spawn_blocking(move || watch(state, storage_id, storage_entry));
    }
}

async fn resolve(
    state: &Daemon,
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
    .fetch_all(state.conn.deref())
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
        let mut storages = state.storages.lock().await; // acquire lock multiple times, because the try_exists probably takes some time
        let storage_data = storages
            .entry(identifier)
            .or_insert(StorageData {
                lock: Arc::default(),
                stats_lock: Arc::default(),
                value: Arc::new((
                    AtomicI64::new(-1),
                    AtomicI64::new(disk.map(|i| i as i64).unwrap_or(-1)),
                )),
            })
            .clone();
        drop(storages);
        let servers = state.servers.lock().await;
        let server = servers.get(&id).cloned();
        drop(servers);
        let sender = match server {
            None => None,
            Some(sender) => Some(sender.lock().await.stats_sender.clone()),
        };
        available.insert(
            identifier,
            (Some(id), path, cpu, ram, swap, storage_data, sender),
        );
    }

    Some(available)
}

pub(crate) fn watch(state: Daemon, storage_id: String, storage_entry: StorageEntry) {
    let mut last = SystemTime::UNIX_EPOCH;
    loop {
        let Some(storages) =
            Handle::current().block_on(resolve(&state, storage_id.clone(), &storage_entry))
        else {
            thread::sleep(Duration::from_secs(30));
            continue;
        };

        let mut quota = false;
        #[cfg(feature = "btrfs")]
        if let StorageEntry::Btrfs {
            quota: true,
            ref path,
            ..
        } = storage_entry
        {
            quota = true;

            let mut quotas = HashMap::new();
            if let Err(err) = super::btrfs::search_quotas(File::open(path), &mut quotas) {
                error!(
                    "Unable to retrieve quotas: {:?}. Ensure quotas are enabled",
                    err
                );
            }

            for (id, (_server_id, path, _, _, _, data, _)) in &storages {
                let subvol = match super::btrfs::get_subvol_id(File::open(path)) {
                    Ok(id) => id,
                    Err(err) => {
                        warn!("Unable to get subvolume id of {:?}: {:?}", id, err);
                        continue;
                    }
                };
                let Some(size) = quotas.get(&subvol) else {
                    warn!("Quota missing for {:?} (subvolid {})", id, subvol);
                    warn!("Retrived quotas: {:?}", quotas);
                    continue;
                };
                let size = if *size > i64::MAX as u64 {
                    -1
                } else {
                    *size as i64
                };
                data.value.0.store(size, Ordering::Relaxed);
            }
        }

        if !quota && last.elapsed().unwrap_or_default() > crate::common::STORAGE_SCAN_INTERVAL {
            last = SystemTime::now();
            for (id, (server_id, path, _, _, _, data, _)) in &storages {
                let Ok(_read_guard) = data.lock.try_read() else {
                    continue;
                };
                match fs_extra::dir::get_size(path) {
                    Ok(size) => {
                        let size = if size > i64::MAX as u64 {
                            -1
                        } else {
                            size as i64
                        };
                        data.value.0.store(size, Ordering::Relaxed);
                        let allowed = data.value.1.load(Ordering::Relaxed);
                        let exceeded = (size / 1048576) > allowed && allowed != -1;
                        if let Some(server_id) = server_id {
                            if exceeded {
                                if let Err(err) = Handle::current().block_on(query!("INSERT IGNORE INTO `server_locks` (`server`, `lock`) VALUES (?, 'storage')", server_id)
                                    .execute(state.conn.deref())) {
                                    warn!("Unable to lock server {}: {:?}", server_id, err);
                                }
                                let servers = state.servers.blocking_lock();
                                if let Some(server) = servers.get(server_id).cloned() {
                                    drop(servers);
                                    let server_id = *server_id;
                                    let docker = state.docker.clone();
                                    tokio::task::spawn(async move {
                                        let server_data = server.lock_owned().await;
                                        if let Err(err) = FetchedServer::stop(&FetchedServer {
                                            id: server_id,
                                            .. FetchedServer::default()
                                        }, &docker, server_data).await {
                                            if let ServerStopError::NotRunning = err {
                                                return;
                                            }
                                            warn!("Unable to stop server after lock {}: {:?}", server_id, err);
                                        }
                                    });
                                }
                            } else if let Err(err) = Handle::current().block_on(query!("DELETE FROM `server_locks` WHERE `server` = ? AND `lock` = 'storage'", server_id)
                                    .execute(state.conn.deref())) {
                                    warn!("Unable to unlock server {}: {:?}", server_id, err);
                                }
                        }
                    }
                    Err(err) => {
                        warn!("Failed to scan directory size for {:?}: {:?}", id, err);
                    }
                }
            }
        }

        let mut join = JoinSet::new();
        for (_id, (server_id, _path, cpu, ram, swap, data, sender)) in storages {
            let conn = state.conn.clone();
            join.spawn(async move {
                let stats_guard = timeout(Duration::from_millis(100), data.stats_lock.lock()).await;
                if stats_guard.is_ok() {
                    if let Some(stats_sender) = sender {
                        let cpu_max = cpu.map(|cpu| cpu * 100f32).unwrap_or(-1f32);
                        let ram_max = ram.map(|ram| ram as i64).unwrap_or(-1);
                        let swap_max = swap.map(|ram| ram as i64).unwrap_or(-1);

                        if !quota && server_id.is_some() {
                            let locks =
                                FetchedServer::resolve_locked(server_id.unwrap(), conn.deref())
                                    .await;
                            let _ =
                                stats_sender.send(ChannelWriteData::Status(if locks.is_empty() {
                                    InternalServerState::Stopped
                                } else {
                                    InternalServerState::Locked
                                }));
                        }
                        let _ = stats_sender.send(ChannelWriteData::Stats(
                            ServerStats::default()
                                .with_disk(data.value.0.load(Ordering::Relaxed))
                                .with_max(
                                    cpu_max,
                                    ram_max,
                                    swap_max,
                                    data.value.1.load(Ordering::Relaxed),
                                ),
                        ));
                    }
                }
            });
        }
        Handle::current().block_on(join.join_all());

        thread::sleep(Duration::from_secs(30));
    }
}
