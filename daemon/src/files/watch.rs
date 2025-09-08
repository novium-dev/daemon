use crate::config::ArchiveCompression;
use crate::files::{IntoFileEntry as _, storage_path_root};
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageLockData, storage_read_timeout};
use crate::{Daemon, DaemonExtra, StorageNotification};
use axum::extract::WebSocketUpgrade;
use axum::extract::ws::{Message, Utf8Bytes, WebSocket};
use bytes::Bytes;
use cap_async_std::ambient_authority;
use cap_async_std::fs::{Dir, MetadataExt as _};
use core::time::Duration;
use daemon_common::service::WebsocketResponse;
use daemon_common::service::error_storage_failure::FromStorageFailure;
use daemon_common::service::error_storage_mutex_locked::FromStorageMutexLocked;
use daemon_common::service::error_storage_not_found::FromStorageNotFound;
use daemon_common::service::files::{
    FileEntry, FilesWatchCommand, FilesWatchResponse, FilesWatchResponseEntry,
    FilesWatchWorkDirectoryChangedReason,
};
use futures_util::SinkExt as _;
use indexmap::IndexMap;
use std::io;
use std::io::ErrorKind;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use tokio::sync::broadcast::Receiver;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::Instant;
use tracing::{debug, error, warn};

pub async fn watch_files<
    E: From<sqlx::Error>
        + From<io::Error>
        + FromStorageNotFound
        + FromStorageMutexLocked
        + FromStorageFailure,
>(
    daemon: &Daemon<impl DaemonExtra>,
    ws: WebSocketUpgrade,
    id: StorageIdentifier,
    path: Option<String>,
) -> Result<WebsocketResponse, E> {
    let StorageLockData {
        notify, storage, ..
    } = storage_read_timeout::<E>(&daemon.config, &*daemon.conn, &daemon.storages, id).await?;
    let (storage, _) = storage_path_root(id, &storage).await?;
    let recv = notify.subscribe();
    Ok(WebsocketResponse(ws.on_upgrade(move |socket| {
        handle_websocket(socket, recv, storage, path)
    })))
}

async fn handle_websocket(
    mut socket: WebSocket,
    mut recv: Receiver<StorageNotification>,
    root: Dir,
    path: Option<String>,
) {
    let mut last_sent = Instant::now();
    let mut ping_sent = false;

    let requested_path = path.unwrap_or_else(|| ".".to_owned());

    let mut initial_data = Vec::with_capacity(2);

    let (mut current_fd, mut current_path) = match root.open_dir(requested_path).await {
        Ok(path) => {
            let current_path = match resolve_current_path(&root, &path).await {
                Ok(path) => path,
                Err(err) => {
                    error!(
                        "Closing connection. Unable to resolve path of requested directory: {err:?}"
                    );
                    drop(err);
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            };
            initial_data.push(FilesWatchResponseEntry::WorkDirectoryChanged {
                reason: FilesWatchWorkDirectoryChangedReason::Update,
                path: current_path.to_string_lossy().to_string(),
            });
            (path, current_path)
        }
        Err(err) if err.kind() == ErrorKind::NotFound || err.kind() == ErrorKind::NotADirectory => {
            initial_data.push(FilesWatchResponseEntry::WorkDirectoryChanged {
                reason: FilesWatchWorkDirectoryChangedReason::Delete,
                path: "/".to_owned(),
            });
            (root.clone(), PathBuf::from("/"))
        }
        Err(err) => {
            error!("Closing connection. Unable to open requested directory: {err:?}");
            drop(err);
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };

    let mut current_ino = match current_fd.dir_metadata().await {
        Ok(meta) => meta.ino(),
        Err(err) => {
            error!("Closing connection. Unable to resolve inode: {err:?}");
            drop(err);
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    let mut current_list = match resolve_list(&current_fd, &current_path).await {
        Ok(list) => list,
        Err(err) => {
            error!("Closing connection. Unable to resolve directory contents: {err:?}");
            drop(err);
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    initial_data.push(FilesWatchResponseEntry::Reset {
        data: current_list.values().cloned().collect(),
    });
    let initial_response = FilesWatchResponse {
        token: String::new(),
        data: initial_data,
    };

    if let Err(err) = socket
        .send(Message::Text(Utf8Bytes::from(
            serde_json::to_string(&initial_response).expect("Unable to serialize watch response"),
        )))
        .await
    {
        warn!("Channel disconnected with error: {err:?}");
        drop(err);
        let _ = socket.send(Message::Close(None)).await;
        return;
    }
    let _ = socket.flush().await;
    drop(initial_response);

    loop {
        if Instant::now().duration_since(last_sent) > Duration::from_secs(10) {
            if ping_sent {
                debug!("Closing connection. Files watch session timed out");
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
            let _ = socket.send(Message::Ping(Bytes::new())).await;
            ping_sent = true;
            last_sent = Instant::now();
        }
        #[expect(
            clippy::integer_division_remainder_used,
            reason = "probably caused by tokio macro"
        )]
        {
            tokio::select! {
                _timeout = tokio::time::sleep(Duration::from_secs(10)) => {}
                stdout = recv.recv() => {
                    match stdout {
                        Ok(data) => {
                            let data = match handle_notification(&root, data, &mut current_fd, &mut current_ino, &mut current_path, &mut current_list).await {
                                Ok(data) => data,
                                Err(err) => {
                                    error!("Closing connection. Unable to handle notification: {err:?}");
                                    drop(err);
                                    let _ = socket.send(Message::Close(None)).await;
                                    return;
                                }
                            };
                            if !data.is_empty() {
                                let response = FilesWatchResponse {
                                    data,
                                    token: String::new(),
                                };
                                if let Err(err) = socket.send(Message::Text(Utf8Bytes::from(serde_json::to_string(&response).expect("Unable to serialize watch response")))).await {
                                    warn!("Channel disconnected with error: {err:?}");
                                    drop(err);
                                    let _ = socket.send(Message::Close(None)).await;
                                    return;
                                }
                                let _ = socket.flush().await;
                            }

                        }
                        Err(RecvError::Lagged(_)) => {},
                        Err(RecvError::Closed) => {
                            error!("Closing connection. Internal Sender disconnected");
                            let _ = socket.send(Message::Close(None)).await;
                            return;
                        }
                    }
                }
                message = socket.recv() => {
                    match message {
                        Some(Ok(message)) => {

                            last_sent = Instant::now();
                            ping_sent = false;

                            match message {
                                Message::Ping(data) => {
                                    let _ = socket.send(Message::Ping(data)).await;
                                    let _ = socket.flush().await;
                                },
                                Message::Text(text) => {
                                    if let Ok(command) = serde_json::from_str(text.as_str()) {
                                        match handle_command(&root, command, &mut current_fd, &mut current_ino, &mut current_path, &mut current_list).await {
                                            Ok(data) => {
                                                if !data.is_empty() {
                                                    let response = FilesWatchResponse {
                                                        data,
                                                        token: String::new(),
                                                    };
                                                    if let Err(err) = socket.send(Message::Text(Utf8Bytes::from(serde_json::to_string(&response).expect("Unable to serialize watch responses")))).await {
                                                        warn!("Channel disconnected with error: {err:?}");
                                                        drop(err);
                                                        let _ = socket.send(Message::Close(None)).await;
                                                        return;
                                                    }
                                                    let _ = socket.flush().await;
                                                }
                                            },
                                            Err(err) => {
                                                error!("Closing connection. Unable to handle command: {err:?}");
                                                drop(err);
                                                let _ = socket.send(Message::Close(None)).await;
                                                return;
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        },
                        Some(Err(err)) => {
                            warn!("Channel disconnected with error: {err:?}");
                            drop(err);
                            let _ = socket.send(Message::Close(None)).await;
                            return;
                        },
                        None => {
                            let _ = socket.send(Message::Close(None)).await;
                            return;
                        }
                    }
                }
            };
        };
    }
}

async fn handle_notification(
    root: &Dir,
    notification: StorageNotification,
    current_fd: &mut Dir,
    current_ino: &mut u64,
    current_path: &mut PathBuf,
    current_list: &mut IndexMap<u64, FileEntry>,
) -> Result<Vec<FilesWatchResponseEntry>, io::Error> {
    let mut responses = Vec::new();
    match notification {
        StorageNotification::Rename {
            from_parent_inode,
            to_parent_inode,
            inode,
            ..
        } => {
            let reload = if inode_in_path(root, inode, current_fd, current_ino).await? {
                if let Ok(path) = resolve_current_path(root, current_fd).await {
                    responses.push(FilesWatchResponseEntry::WorkDirectoryChanged {
                        reason: FilesWatchWorkDirectoryChangedReason::Update,
                        path: path.to_string_lossy().to_string(),
                    });
                    *current_path = path;
                    false
                } else {
                    let (reload, responses_result) =
                        handle_reload_work_directory(root, current_fd, current_ino, current_path)
                            .await?;
                    responses.extend(responses_result);
                    if reload {
                        responses.extend(
                            handle_reload(current_fd, current_path, current_list, true).await?,
                        );
                    }
                    reload
                }
            } else {
                false
            };
            if (*current_ino == from_parent_inode || *current_ino == to_parent_inode) && !reload {
                handle_reload(current_fd, current_path, current_list, false).await?;
            }
        }
        StorageNotification::Update {
            parent_inode,
            inode,
        } => {
            if parent_inode == Some(*current_ino)
                || inode.is_some_and(|ino| current_list.contains_key(&ino))
            {
                handle_reload(current_fd, current_path, current_list, false).await?;
            }
        }
        StorageNotification::Create { parent_inode, .. } => {
            if parent_inode == *current_ino {
                handle_reload(current_fd, current_path, current_list, false).await?;
            }
        }
        StorageNotification::Delete {
            parent_inode, dir, ..
        } => {
            let reload = if dir {
                let (reload, responses_result) =
                    handle_reload_work_directory(root, current_fd, current_ino, current_path)
                        .await?;
                responses.extend(responses_result);
                if reload {
                    responses
                        .extend(handle_reload(current_fd, current_path, current_list, true).await?);
                }
                reload
            } else {
                false
            };
            if parent_inode == *current_ino && !reload {
                handle_reload(current_fd, current_path, current_list, false).await?;
            }
        }
        StorageNotification::RenameFrom { parent_inode, .. } => {
            let (reload, responses_result) =
                handle_reload_work_directory(root, current_fd, current_ino, current_path).await?;
            responses.extend(responses_result);
            if reload {
                responses
                    .extend(handle_reload(current_fd, current_path, current_list, true).await?);
            }
            if parent_inode == Some(*current_ino) && !reload {
                handle_reload(current_fd, current_path, current_list, false).await?;
            }
        }
    }
    Ok(responses)
}

async fn handle_command(
    root: &Dir,
    command: FilesWatchCommand,
    current_fd: &mut Dir,
    current_ino: &mut u64,
    current_path: &mut PathBuf,
    current_list: &mut IndexMap<u64, FileEntry>,
) -> Result<Vec<FilesWatchResponseEntry>, io::Error> {
    Ok(match command {
        FilesWatchCommand::Reload => {
            let (reload, mut responses) =
                handle_reload_work_directory(root, current_fd, current_ino, current_path).await?;

            responses.extend(handle_reload(current_fd, current_path, current_list, reload).await?);

            responses
        }
        FilesWatchCommand::Fresh => {
            vec![FilesWatchResponseEntry::Reset {
                data: current_list.values().cloned().collect(),
            }]
        }
        FilesWatchCommand::WorkDirectory { path } => {
            match handle_work_directory_change(
                root,
                path,
                current_fd,
                current_ino,
                current_path,
                current_list,
            )
            .await
            {
                Ok(responses) => responses,
                Err(err)
                    if err.kind() == ErrorKind::NotFound
                        || err.kind() == ErrorKind::NotADirectory =>
                {
                    Vec::new()
                }
                Err(err) => return Err(err),
            }
        }
    })
}

async fn inode_in_path(
    root: &Dir,
    inode: u64,
    current_fd: &Dir,
    current_ino: &u64,
) -> Result<bool, io::Error> {
    let mut tmp_ino = *current_ino;
    let mut handle = current_fd.clone();

    let root_ino = root.dir_metadata().await?.ino();
    loop {
        if inode == tmp_ino {
            break Ok(true);
        }
        if root_ino == tmp_ino {
            break Ok(false);
        }
        handle = handle.open_parent_dir(ambient_authority()).await?;
        tmp_ino = handle.dir_metadata().await?.ino();
    }
}

async fn handle_work_directory_change(
    root: &Dir,
    requested_path: String,
    current_fd: &mut Dir,
    current_ino: &mut u64,
    current_path: &mut PathBuf,
    current_list: &mut IndexMap<u64, FileEntry>,
) -> Result<Vec<FilesWatchResponseEntry>, io::Error> {
    if requested_path.starts_with('/') {
        *current_fd = root.open_dir(requested_path).await?;
    } else {
        let mut path = &*requested_path;
        let mut dir = current_fd.clone();

        let root_ino = root.dir_metadata().await?.ino();
        let root_path = root.canonicalize(".").await?;

        while path.starts_with("../") || path.eq("..") {
            if path.eq("..") || path.eq("../") {
                path = ".";
            } else if let Some(new_path) = path.strip_prefix("../") {
                path = new_path;
            } else {
                unreachable!()
            }

            let dir_ino = dir.dir_metadata().await?.ino();
            if root_ino == dir_ino {
                return Err(io::Error::new(ErrorKind::NotFound, "root_ino == dir_ino"));
            }
            dir = dir.open_parent_dir(ambient_authority()).await?;
            let dir_path = dir.canonicalize(".").await?;
            if dir_path.strip_prefix(&root_path).is_err() {
                return Err(io::Error::new(
                    ErrorKind::NotFound,
                    "dir_path.strip_prefix(root_path).is_err()",
                ));
            }
        }

        *current_fd = dir.open_dir(path).await?;
    }

    *current_ino = current_fd.dir_metadata().await?.ino();
    *current_path = resolve_current_path(root, current_fd).await?;

    let mut responses = vec![FilesWatchResponseEntry::WorkDirectoryChanged {
        path: current_path.to_string_lossy().to_string(),
        reason: FilesWatchWorkDirectoryChangedReason::Changed,
    }];
    responses.extend(handle_reload(current_fd, current_path, current_list, true).await?);

    Ok(responses)
}

async fn resolve_current_path(root: &Dir, current_fd: &Dir) -> Result<PathBuf, io::Error> {
    let path = current_fd.canonicalize(".").await?;
    process_path(root, path.as_ref()).await
}

async fn resolve_list(
    current_fd: &Dir,
    current_path: &Path,
) -> Result<IndexMap<u64, FileEntry>, io::Error> {
    let readdir = current_fd
        .read_dir(".")
        .await?
        .collect::<Result<Vec<_>, io::Error>>()?;
    let mut map = IndexMap::with_capacity(readdir.len());
    for entry in readdir {
        let metadata = entry.metadata()?;

        let mut link_path = None;
        let mut archive = false;

        let relative_path = current_path.join(entry.file_name());
        let file_name = entry.file_name().to_string_lossy().to_string();
        if metadata.is_symlink() {
            link_path = Some(
                current_fd
                    .read_link(PathBuf::from(entry.file_name()))
                    .await
                    .inspect_err(|err| error!("Failed to list files (watch, read_link): {err:?}"))?
                    .to_string_lossy()
                    .to_string(),
            );
        } else {
            archive = tokio::task::spawn_blocking(move || {
                let file = entry.open()?;
                let fd: OwnedFd = file.into_std().into();
                let file = std::fs::File::from(fd);
                if let Ok(Some(_)) = ArchiveCompression::detect_blocking(file) {
                    Ok::<_, io::Error>(true)
                } else {
                    Ok(false)
                }
            })
            .await
            .inspect_err(|err| error!("Failed to list files (watch, detect): {err:?}"))
            .map_err(io::Error::from)?
            .inspect_err(|err| error!("Failed to list files (watch, detect): {err:?}"))?;
        }

        map.insert(
            metadata.ino(),
            metadata.into_fileentry(
                file_name,
                relative_path.to_string_lossy().to_string(),
                link_path,
                archive,
            ),
        );
    }
    Ok(map)
}

async fn handle_reload(
    current_fd: &Dir,
    current_path: &Path,
    current_list: &mut IndexMap<u64, FileEntry>,
    fresh: bool,
) -> Result<Vec<FilesWatchResponseEntry>, io::Error> {
    let list = resolve_list(current_fd, current_path).await?;
    if fresh {
        Ok(vec![FilesWatchResponseEntry::Reset {
            data: list.into_values().collect(),
        }])
    } else {
        let mut entries = Vec::new();

        for (index, (ino_new, entry_new)) in list.clone().into_iter().enumerate() {
            if let Some((ino_current, entry_current)) = current_list.get_index_mut(index) {
                if ino_new == *ino_current {
                    if !entry_new.eq(entry_current) {
                        entries.push(FilesWatchResponseEntry::Modify {
                            index,
                            entry: entry_new.clone(),
                        });
                        *entry_current = entry_new;
                    }
                    continue;
                }
                if !list.contains_key(ino_current) {
                    entries.push(FilesWatchResponseEntry::Remove { index });
                    current_list.shift_remove_index(index);
                }
                if let Some(old_index) = current_list.get_index_of(&ino_new) {
                    entries.push(FilesWatchResponseEntry::Move {
                        index: old_index,
                        offset: i64::try_from(index)
                            .map_err(io::Error::other)?
                            .checked_sub(i64::try_from(old_index).map_err(io::Error::other)?)
                            .expect("index - old_index underflowed"),
                    });
                    current_list.move_index(old_index, index);
                    continue;
                }
            }

            current_list.shift_insert(index, ino_new, entry_new);
        }

        Ok(entries)
    }
}

async fn handle_reload_work_directory(
    root: &Dir,
    current_fd: &mut Dir,
    current_ino: &mut u64,
    current_path: &mut PathBuf,
) -> Result<(bool, Vec<FilesWatchResponseEntry>), io::Error> {
    let mut responses = Vec::new();

    let reload = match current_fd.canonicalize(".").await {
        Ok(path) => {
            if let Ok(current) = process_path(root, path.as_ref()).await {
                if !current.eq(current_path) {
                    responses.push(FilesWatchResponseEntry::WorkDirectoryChanged {
                        path: current.to_string_lossy().to_string(),
                        reason: FilesWatchWorkDirectoryChangedReason::Update,
                    });
                    *current_path = current;
                }
                false
            } else {
                // could be triggered by cross-server rename in sftp
                responses.extend(
                    handle_delete_work_directory(root, current_fd, current_ino, current_path)
                        .await?,
                );

                true
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            responses.extend(
                handle_delete_work_directory(root, current_fd, current_ino, current_path).await?,
            );

            true
        }
        Err(err) => return Err(err),
    };

    Ok((reload, responses))
}

async fn handle_delete_work_directory(
    root: &Dir,
    current_fd: &mut Dir,
    current_ino: &mut u64,
    current_path: &mut PathBuf,
) -> Result<Vec<FilesWatchResponseEntry>, io::Error> {
    let root_ino = root.dir_metadata().await?.ino();

    let parent = if let Ok(parent) = current_fd.open_parent_dir(ambient_authority()).await {
        let metadata = parent.dir_metadata().await?;
        let path = parent.canonicalize(".").await?;
        let current = process_path(root, path.as_ref()).await?;

        Some((parent, metadata.ino(), current))
    } else {
        None
    };

    if root_ino == *current_ino || current_path.as_os_str().eq("/") || parent.is_none() {
        *current_fd = root.clone();
        *current_ino = root_ino;
        *current_path = PathBuf::from("/");

        Ok(vec![FilesWatchResponseEntry::WorkDirectoryChanged {
            path: "/".to_owned(),
            reason: FilesWatchWorkDirectoryChangedReason::Delete,
        }])
    } else if let Some((fd, ino, path)) = parent {
        *current_fd = fd;
        *current_ino = ino;
        current_path.clone_from(&path);
        Ok(vec![FilesWatchResponseEntry::WorkDirectoryChanged {
            path: path.to_string_lossy().to_string(),
            reason: FilesWatchWorkDirectoryChangedReason::Delete,
        }])
    } else {
        unreachable!()
    }
}

async fn process_path(root: &Dir, path: &Path) -> Result<PathBuf, io::Error> {
    let prefix = root.canonicalize(".").await?;
    let current = path.strip_prefix(prefix).map_err(io::Error::other)?;
    if current.as_os_str().is_empty() {
        Ok(PathBuf::from("/"))
    } else {
        Ok(PathBuf::from(current))
    }
}
