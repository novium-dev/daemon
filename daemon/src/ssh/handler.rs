// SPDX-License-Identifier: AGPL-3.0-only
use crate::ssh::auth::{InvalidationCacheMap, password_auth};
use crate::ssh::sftp::DaemonSftpSession;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageError, StorageLockData, storage_read_timeout};
use crate::{Daemon, DaemonExtra};
use cap_async_std::ambient_authority;
use cap_async_std::fs::Dir;
use core::sync::atomic::{AtomicU16, Ordering};
use daemon_common::node::BackendAuthServerPermissions;
use daemon_common::service::files::task::StorageTask;
use daemon_common::ssh::{DaemonBackendAuthResponse, DaemonBackendAuthServer};
use russh::keys::PublicKey;
use russh::keys::ssh_key::Fingerprint;
use russh::server::{Auth, ChannelOpenHandle, Handler, Msg, Session};
use russh::{Channel, ChannelId, Pty};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};

#[derive(Default)]
pub struct DaemonChannel {
    pub channel: Option<Channel<Msg>>,
    pub server_id: Option<u32>,
    pub cols: Option<Arc<AtomicU16>>,
    pub rows: Option<Arc<AtomicU16>>,
}

pub struct DaemonSSHHandler<T: DaemonExtra> {
    pub daemon: Daemon<T>,
    pub channels: HashMap<ChannelId, DaemonChannel>,
    pub publickey_cache: InvalidationCacheMap<(String, Fingerprint), DaemonBackendAuthResponse>,
    pub auth: Option<DaemonBackendAuthResponse>,
}

impl<T: DaemonExtra> Handler for DaemonSSHHandler<T> {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        #[cfg(feature = "ssh_debug_auth_bypass")]
        {
            use sqlx::query_scalar;
            if cfg!(feature = "ssh_debug_auth_bypass") {
                let servers: Result<Vec<u32>, sqlx::Error> =
                    query_scalar!("SELECT `id` FROM `servers`")
                        .fetch_all(&*self.daemon.conn)
                        .await;
                let mut auth =
                    Vec::with_capacity(servers.as_ref().map(Vec::len).unwrap_or_default());

                match servers {
                    Ok(servers) => {
                        for server in servers {
                            auth.push(crate::common::ssh::DaemonBackendAuthServer {
                                id: server,
                                name: server.to_string(),
                                permissions: BackendAuthServerPermissions::all(),
                            });
                        }
                    }
                    Err(err) => {
                        debug!("Failed to complete authentication: {err:?}");
                        return Err(russh::Error::IO(std::io::Error::other(err)));
                    }
                }

                self.auth = Some(auth);
                return Ok(Auth::Accept);
            }
        }
        let Some(ssh) = &self.daemon.config.ssh else {
            return Err(russh::Error::Inconsistent);
        };
        match password_auth(&self.daemon, ssh, user, password)
            .await
            .inspect_err(|err| debug!("Password authentication failed: {err:?}"))
        {
            Ok(response) => {
                self.auth = Some(response);
                Ok(Auth::Accept)
            }
            Err(err) if err.kind() == ErrorKind::PermissionDenied => Ok(Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            }),
            Err(err) => Err(russh::Error::IO(err)),
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        #[cfg(feature = "ssh_debug_auth_bypass")]
        {
            if cfg!(feature = "ssh_debug_auth_bypass") {
                return Ok(Auth::UnsupportedMethod);
            }
        }
        let Some(ssh) = &self.daemon.config.ssh else {
            return Err(russh::Error::Inconsistent);
        };
        match self
            .publickey_cache
            .get_resolved(&self.daemon, ssh, user, public_key)
            .await
            .inspect_err(|err| debug!("Publickey authentication failed: {err:?}"))
        {
            Ok(response) => {
                self.auth = Some(response);
                Ok(Auth::Accept)
            }
            Err(err) => {
                if err.kind() == ErrorKind::PermissionDenied {
                    Ok(Auth::Reject {
                        proceed_with_methods: None,
                        partial_success: false,
                    })
                } else {
                    Err(russh::Error::IO(err))
                }
            }
        }
    }

    async fn auth_publickey_offered(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        #[cfg(feature = "ssh_debug_auth_bypass")]
        {
            if cfg!(feature = "ssh_debug_auth_bypass") {
                return Ok(Auth::UnsupportedMethod);
            }
        }
        let Some(ssh) = &self.daemon.config.ssh else {
            return Err(russh::Error::Inconsistent);
        };
        match self
            .publickey_cache
            .get_resolved(&self.daemon, ssh, user, public_key)
            .await
            .inspect_err(|err| debug!("Publickey offered authentication failed: {err:?}"))
        {
            Ok(_response) => Ok(Auth::Accept),
            Err(err) => {
                if err.kind() == ErrorKind::PermissionDenied {
                    Ok(Auth::Reject {
                        proceed_with_methods: None,
                        partial_success: false,
                    })
                } else {
                    Err(russh::Error::IO(err))
                }
            }
        }
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.close(channel)
    }

    async fn window_change_request(
        &mut self,
        channel_id: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.auth.is_none() {
            return Err(russh::Error::NotAuthenticated);
        }
        debug!("Window size change {col_width}x{row_height}");

        let Ok(col_width) = u16::try_from(col_width) else {
            debug!("The column size is larger than u16::MAX");
            return session.channel_failure(channel_id);
        };
        let Ok(row_height) = u16::try_from(row_height) else {
            debug!("The row size is larger than u16::MAX");
            return session.channel_failure(channel_id);
        };

        match self.channels.get_mut(&channel_id) {
            Some(DaemonChannel {
                server_id,
                cols,
                rows,
                ..
            }) => {
                let cols = match cols {
                    None => {
                        debug!("Size is None on window size");
                        return session.channel_failure(channel_id);
                    }
                    Some(cols) => Arc::clone(cols),
                };
                let rows = match rows {
                    None => {
                        debug!("Size is None on window size");
                        return session.channel_failure(channel_id);
                    }
                    Some(rows) => Arc::clone(rows),
                };
                cols.store(col_width, Ordering::Relaxed);
                rows.store(row_height, Ordering::Relaxed);
                match server_id {
                    None => {}
                    Some(id) => {
                        let mut servers = self.daemon.servers.lock().await;
                        let server_data = Arc::clone(match servers.get(id) {
                            None => servers.entry(*id).or_default(),
                            Some(server) => server,
                        });
                        drop(servers);

                        crate::terminal::update_size(*id, &self.daemon.docker, &server_data).await;
                    }
                }
                session.channel_success(channel_id)
            }
            None => {
                debug!("Tried to access a non existent channel");
                session.channel_failure(channel_id)
            }
        }
    }

    async fn exec_request(
        &mut self,
        channel_id: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(auth) = &self.auth else {
            return Err(russh::Error::NotAuthenticated);
        };
        let server_id = String::from_utf8(data.to_vec())
            .inspect_err(|_err| debug!("Command is not utf-8 encoded"))
            .map_err(|err| russh::Error::Utf8(err.utf8_error()))?;
        debug!("Exec {server_id}");

        let server = Self::find_server(auth, &server_id);

        self.ssh_terminal(channel_id, session, server).await
    }

    async fn pty_request(
        &mut self,
        channel_id: ChannelId,
        _term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.auth.is_none() {
            return Err(russh::Error::NotAuthenticated);
        }
        debug!("PTY Request");

        let Ok(col_width) = u16::try_from(col_width) else {
            debug!("The column size is larger than u16::MAX");
            return session.channel_failure(channel_id);
        };
        let Ok(row_height) = u16::try_from(row_height) else {
            debug!("The row size is larger than u16::MAX");
            return session.channel_failure(channel_id);
        };

        if let Some(DaemonChannel { cols, rows, .. }) = self.channels.get_mut(&channel_id) {
            match cols {
                None => *cols = Some(Arc::new(AtomicU16::new(col_width))),
                Some(_) => {
                    debug!("Size is Some on pty");
                    return session.channel_failure(channel_id);
                }
            }
            match rows {
                None => *rows = Some(Arc::new(AtomicU16::new(row_height))),
                Some(_) => {
                    debug!("Size is Some on pty");
                    return session.channel_failure(channel_id);
                }
            }
        }
        session.channel_success(channel_id)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        handle: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        handle.accept().await;

        self.channels.insert(
            channel.id(),
            DaemonChannel {
                channel: Some(channel),
                ..DaemonChannel::default()
            },
        );

        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let Some(auth) = &self.auth else {
            return Err(russh::Error::NotAuthenticated);
        };
        debug!("Subsystem Request {name}");

        if name == "sftp" {
            match self.channels.get_mut(&channel_id) {
                None => {
                    debug!("Channel invalid on subsystem request");
                    session.channel_failure(channel_id)
                }
                Some(DaemonChannel { channel, .. }) => {
                    let Some(channel) = channel.take() else {
                        debug!("Channel was already taken on subsystem request");
                        return session.channel_failure(channel_id);
                    };
                    let mut sftp = DaemonSftpSession {
                        access: HashMap::new(),
                        file_handles: HashMap::new(),
                        version: None,
                    };

                    for server in auth {
                        if server
                            .permissions
                            .contains(BackendAuthServerPermissions::FILES_READ)
                        {
                            let identifier = StorageIdentifier::Primary(server.id);
                            if let Ok(StorageLockData {
                                lock,
                                storage,
                                notify,
                                ..
                            }) = storage_read_timeout::<StorageError>(
                                &self.daemon.config,
                                &*self.daemon.conn,
                                &self.daemon.storages,
                                identifier,
                                StorageTask::Sftp,
                            )
                            .await
                            {
                                let base =
                                    PathBuf::from(storage.path()).join(identifier.as_pathbuf());
                                match Dir::open_ambient_dir(&base, ambient_authority())
                                    .await
                                    .inspect_err(|err| {
                                        error!(
                                            "Failed to open storage directory: {} {err:?}",
                                            base.to_string_lossy(),
                                        );
                                    }) {
                                    Ok(storage) => {
                                        sftp.access.insert(
                                            vec!["servers".to_owned(), server.name.clone()],
                                            (
                                                Arc::new(storage),
                                                server.permissions,
                                                lock,
                                                Some(notify),
                                            ),
                                        );
                                    }
                                    Err(_err) => continue,
                                }
                            }
                        }
                        if server
                            .permissions
                            .contains(BackendAuthServerPermissions::BACKUP_DOWNLOAD)
                        {
                            let identifier = StorageIdentifier::Primary(server.id);
                            for (name, backup) in &self.daemon.config.backup {
                                if let Ok(StorageLockData { lock, .. }) =
                                    storage_read_timeout::<StorageError>(
                                        &self.daemon.config,
                                        &*self.daemon.conn,
                                        &self.daemon.storages,
                                        identifier,
                                        StorageTask::Sftp,
                                    )
                                    .await
                                {
                                    let base =
                                        PathBuf::from(backup.path()).join(identifier.as_pathbuf());
                                    if matches!(tokio::fs::try_exists(&base).await, Ok(false)) {
                                        let _ = tokio::fs::create_dir_all(&base).await;
                                    }
                                    if let Ok(storage) =
                                        Dir::open_ambient_dir(&base, ambient_authority())
                                            .await
                                            .inspect_err(|err| {
                                                error!(
                                                    "Failed to open backup directory: {} {err:?}",
                                                    base.to_string_lossy(),
                                                );
                                            })
                                    {
                                        sftp.access.insert(
                                            vec![
                                                "backups".to_owned(),
                                                name.clone(),
                                                server.name.clone(),
                                            ],
                                            (
                                                Arc::new(storage),
                                                BackendAuthServerPermissions::FILES_READ,
                                                lock,
                                                None,
                                            ),
                                        );
                                    }
                                }
                            }
                        }
                    }
                    russh_sftp::server::run(channel.into_stream(), sftp).await;
                    session.channel_success(channel_id)
                }
            }
        } else {
            debug!("Tried to access a non existent subsystem: {name:?}");
            session.channel_failure(channel_id)
        }
    }
}

impl<T: DaemonExtra> DaemonSSHHandler<T> {
    pub(crate) fn find_server(
        auth: &DaemonBackendAuthResponse,
        id: &str,
    ) -> Option<DaemonBackendAuthServer> {
        for server in auth {
            if server.name == id {
                return Some(server.clone());
            }
        }
        None
    }
}
