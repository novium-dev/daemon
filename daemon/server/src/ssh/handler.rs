use crate::ssh::auth::{InvalidationCacheMap, password_auth};
use crate::ssh::sftp::DaemonSftpSession;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageError, StorageLockData, storage_read_timeout};
use crate::sync::Mutex;
use crate::terminal::update_size;
use crate::{Daemon, ServerData};
use cap_async_std::ambient_authority;
use cap_async_std::fs::Dir;
use daemon_common::server::BackendAuthServerPermissions;
use daemon_common::ssh::{DaemonBackendAuthResponse, DaemonBackendAuthServer};
use russh::keys::{HashAlg, PublicKey};
use russh::server::{Auth, Handler, Msg, Session};
use russh::{Channel, ChannelId, Pty};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tracing::{debug, error};

pub(crate) type DaemonChannel = (
    Option<Channel<Msg>>,
    Option<u32>,
    Option<Arc<AtomicU32>>,
    Option<Arc<AtomicU32>>,
);

pub struct DaemonSSHHandler {
    pub(crate) state: Arc<Daemon>,
    pub(crate) channels: HashMap<ChannelId, DaemonChannel>,
    pub(crate) publickey_cache: InvalidationCacheMap<
        (String, [u8; HashAlg::Sha512.digest_size()]),
        DaemonBackendAuthResponse,
    >,
    pub(crate) auth: Option<DaemonBackendAuthResponse>,
}

impl Handler for DaemonSSHHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        #[cfg(feature = "ssh_debug_auth_bypass")]
        {
            use sqlx::query_scalar;
            if cfg!(feature = "ssh_debug_auth_bypass") {
                let conn = self.state.conn.clone();
                let servers: Result<Vec<u32>, sqlx::Error> =
                    query_scalar!("SELECT `id` FROM `servers`")
                        .fetch_all(conn.deref())
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
                        debug!("Failed to complete authentication: {:?}", err);
                        return Err(russh::Error::IO(std::io::Error::new(ErrorKind::Other, err)));
                    }
                }

                self.auth = Some(auth);
                return Ok(Auth::Accept);
            }
        }
        match password_auth(&self.state, user, password)
            .await
            .inspect_err(|err| debug!("Password authentication failed: {:?}", err))
        {
            Ok(response) => {
                self.auth = Some(response);
                Ok(Auth::Accept)
            }
            Err(err) => match err {
                err if err.kind() == ErrorKind::PermissionDenied => Ok(Auth::Reject {
                    proceed_with_methods: None,
                    partial_success: false,
                }),
                err => Err(russh::Error::IO(err)),
            },
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
        match self
            .publickey_cache
            .get_resolved(&self.state, user, public_key)
            .await
            .inspect_err(|err| debug!("Publickey authentication failed: {:?}", err))
        {
            Ok(response) => {
                self.auth = Some(response);
                Ok(Auth::Accept)
            }
            Err(err) => match err {
                err if err.kind() == ErrorKind::PermissionDenied => Ok(Auth::Reject {
                    proceed_with_methods: None,
                    partial_success: false,
                }),
                err => Err(russh::Error::IO(err)),
            },
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
        match self
            .publickey_cache
            .get_resolved(&self.state, user, public_key)
            .await
            .inspect_err(|err| debug!("Publickey offered authentication failed: {:?}", err))
        {
            Ok(_response) => Ok(Auth::Accept),
            Err(err) => match err {
                err if err.kind() == ErrorKind::PermissionDenied => Ok(Auth::Reject {
                    proceed_with_methods: None,
                    partial_success: false,
                }),
                err => Err(russh::Error::IO(err)),
            },
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
        debug!("Window size change {}x{}", col_width, row_height);
        match self.channels.get_mut(&channel_id) {
            Some((_channel, id, cols, rows)) => {
                let cols = match cols {
                    None => {
                        debug!("Size is None on window size");
                        return session.channel_failure(channel_id);
                    }
                    Some(cols) => cols.clone(),
                };
                let rows = match rows {
                    None => {
                        debug!("Size is None on window size");
                        return session.channel_failure(channel_id);
                    }
                    Some(rows) => rows.clone(),
                };
                cols.store(col_width, Ordering::Relaxed);
                rows.store(row_height, Ordering::Relaxed);
                match id {
                    None => Ok(()),
                    Some(id) => {
                        let mut servers = self.state.servers.lock().await;
                        let server_data_arc = match servers.get(id) {
                            None => {
                                servers.insert(*id, Arc::new(Mutex::new(ServerData::default())));
                                servers.get(id).unwrap()
                            }
                            Some(server) => server,
                        };
                        update_size(*id, &self.state.docker, server_data_arc.clone()).await;
                        session.channel_success(channel_id)
                    }
                }
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
        if self.auth.is_none() {
            return Err(russh::Error::NotAuthenticated);
        }
        let server_id = String::from_utf8(data.to_vec())
            .inspect_err(|_err| debug!("Command is not utf-8 encoded"))
            .map_err(|err| russh::Error::Utf8(err.utf8_error()))?;
        debug!("Exec {}", server_id);

        #[cfg(feature = "ssh_control")]
        if let Some(server_id) = server_id.strip_prefix("control ") {
            return self
                .ssh_control(channel_id, session, server_id.to_string())
                .await;
        }

        self.ssh_terminal(channel_id, session, server_id).await
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
        if let Some((_channel, _id, cols, rows)) = self.channels.get_mut(&channel_id) {
            match cols {
                None => *cols = Some(Arc::new(AtomicU32::new(col_width))),
                Some(_) => {
                    debug!("Size is Some on pty");
                    return session.channel_failure(channel_id);
                }
            }
            match rows {
                None => *rows = Some(Arc::new(AtomicU32::new(row_height))),
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
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        self.channels
            .insert(channel.id(), (Some(channel), None, None, None));
        Ok(true)
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.auth.is_none() {
            return Err(russh::Error::NotAuthenticated);
        }
        debug!("Subsystem Request {}", name);

        if name == "sftp" && cfg!(feature = "sftp") {
            #[cfg(feature = "sftp")]
            match self.channels.get_mut(&channel_id) {
                None => {
                    debug!("Channel invalid on subsystem request");
                    session.channel_failure(channel_id)
                }
                Some((channel, _id, _cols, _rows)) => {
                    let channel = channel.take();
                    if channel.is_none() {
                        debug!("Channel was already taken on subsystem request");
                        return session.channel_failure(channel_id);
                    }
                    let mut sftp = DaemonSftpSession {
                        access: HashMap::new(),
                        file_handles: HashMap::new(),
                        version: None,
                    };

                    for server in self.auth.as_ref().unwrap() {
                        if server
                            .permissions
                            .contains(BackendAuthServerPermissions::FILES_READ)
                        {
                            let identifier = StorageIdentifier::Primary(server.id);
                            if let Ok(StorageLockData { lock, storage, .. }) =
                                storage_read_timeout::<StorageError>(
                                    &self.state.config,
                                    self.state.conn.deref(),
                                    &self.state.storages,
                                    identifier,
                                )
                                .await
                            {
                                let base =
                                    PathBuf::from(storage.path()).join(identifier.as_pathbuf());
                                match Dir::open_ambient_dir(&base, ambient_authority())
                                    .await
                                    .inspect_err(|err| {
                                        error!(
                                            "Failed to open storage directory: {} {:?}",
                                            base.to_string_lossy(),
                                            err
                                        )
                                    }) {
                                    Ok(storage) => {
                                        sftp.access.insert(
                                            vec!["servers".to_string(), server.name.clone()],
                                            (Arc::new(storage), server.permissions, lock),
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
                            for (name, backup) in &self.state.config.backup {
                                if let Ok(StorageLockData { lock, .. }) =
                                    storage_read_timeout::<StorageError>(
                                        &self.state.config,
                                        self.state.conn.deref(),
                                        &self.state.storages,
                                        identifier,
                                    )
                                    .await
                                {
                                    let base =
                                        PathBuf::from(backup.path()).join(identifier.as_pathbuf());
                                    if let Ok(false) = tokio::fs::try_exists(&base).await {
                                        let _ = tokio::fs::create_dir_all(&base).await;
                                    }
                                    match Dir::open_ambient_dir(&base, ambient_authority())
                                        .await
                                        .inspect_err(|err| {
                                            error!(
                                                "Failed to open backup directory: {} {:?}",
                                                base.to_string_lossy(),
                                                err
                                            )
                                        }) {
                                        Ok(storage) => {
                                            sftp.access.insert(
                                                vec![
                                                    "backups".to_string(),
                                                    name.clone(),
                                                    server.name.clone(),
                                                ],
                                                (
                                                    Arc::new(storage),
                                                    BackendAuthServerPermissions::FILES_READ,
                                                    lock,
                                                ),
                                            );
                                        }
                                        Err(_err) => continue,
                                    }
                                }
                            }
                        }
                    }
                    russh_sftp::server::run(channel.unwrap().into_stream(), sftp).await;
                    session.channel_success(channel_id)
                }
            }
        } else {
            debug!("Tried to access a non existent subsystem: {:?}", name);
            session.channel_failure(channel_id)
        }
    }
}

impl DaemonSSHHandler {
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
