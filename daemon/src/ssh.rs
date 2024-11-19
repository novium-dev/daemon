use std::collections::HashMap;
use std::hash::Hash;
use std::io::{self, ErrorKind};
use std::net::SocketAddr;
use std::ops::DerefMut;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(feature = "sftp")]
use crate::ssh::sftp::DaemonSftpSession;
#[cfg(feature = "sftp")]
use crate::storage::StorageIdentifier;
use crate::terminal::{new_session, update_size, TerminalData};
use crate::utils::{storage_read, ChannelAdapter, ChannelError, ChannelWriteData};
use crate::{DaemonState, ServerData};
use axum::{async_trait, http};
use cap_async_std::ambient_authority;
use cap_async_std::fs::Dir;
use daemon_common::server::BackendAuthServerPermissions;
use daemon_common::ssh::DaemonBackendAuthResponse;
use russh::server::{Auth, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId, ChannelStream, Pty};
use russh_keys::key::PublicKey;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{debug, error};

#[cfg(feature = "sftp")]
pub(crate) mod sftp;

#[derive(Clone)]
pub struct DaemonSSHServer {
    pub(crate) state: Arc<DaemonState>,
    pub(crate) cache_invalidation: Duration,
}

impl Server for DaemonSSHServer {
    type Handler = DaemonSSHHandler;

    fn new_client(&mut self, _peer_addr: Option<SocketAddr>) -> Self::Handler {
        DaemonSSHHandler {
            state: self.state.clone(),
            channels: HashMap::new(),
            publickey_cache: InvalidationCacheMap::new(self.cache_invalidation),
            auth: None,
        }
    }

    fn handle_session_error(&mut self, error: <Self::Handler as Handler>::Error) {
        if let russh::Error::IO(ref err) = error {
            if matches!(
                err.kind(),
                ErrorKind::UnexpectedEof | ErrorKind::ConnectionReset
            ) {
                return;
            }
        }
        error!("Session error: {:?}", error);
    }
}

pub(crate) type DaemonChannel = (
    Option<Channel<Msg>>,
    Option<u32>,
    Option<Arc<AtomicU32>>,
    Option<Arc<AtomicU32>>,
);

pub struct DaemonSSHHandler {
    state: Arc<DaemonState>,
    channels: HashMap<ChannelId, DaemonChannel>,
    publickey_cache: InvalidationCacheMap<(String, String), DaemonBackendAuthResponse>,
    auth: Option<DaemonBackendAuthResponse>,
}

struct InvalidationCacheMap<T: Clone + Hash + Eq, E: Clone>(HashMap<T, (Instant, E)>, Duration);

#[allow(unused)]
impl<T: Clone + Hash + Eq, E: Clone> InvalidationCacheMap<T, E> {
    pub(crate) fn new(invalidation: Duration) -> Self {
        Self(HashMap::new(), invalidation)
    }

    pub(crate) fn insert(&mut self, key: T, val: E) {
        self.update();
        self.0.insert(key, (Instant::now(), val));
    }

    pub(crate) fn get(&mut self, key: &T) -> Option<&mut E> {
        self.update();
        self.0.get_mut(key).map(|t| &mut t.1)
    }

    pub(crate) fn remove(&mut self, key: &T) {
        self.0.remove(key);
    }

    pub(crate) fn update(&mut self) {
        for (key, (time, _val)) in self.0.clone() {
            if time.elapsed() >= self.1 {
                self.0.remove(&key);
            }
        }
    }
}

async fn password_auth(
    state: &DaemonState,
    user: &str,
    password: &str,
) -> Result<DaemonBackendAuthResponse, io::Error> {
    let mut url = state
        .config
        .ssh
        .auth_url
        .join("/api/internal/auth/password")
        .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
    url.query_pairs_mut()
        .append_pair("node", &state.config.identifier);

    state
        .client
        .get(url)
        .basic_auth(user, Some(password))
        .send()
        .await
        .inspect_err(|err| debug!("SSH Authentication failed (password): {:?}", err))
        .map_err(|err| match err {
            err if matches!(err.status(), Some(http::StatusCode::UNAUTHORIZED)) => {
                io::Error::new(ErrorKind::PermissionDenied, err)
            }
            err => io::Error::new(ErrorKind::Other, err),
        })?
        .json()
        .await
        .map_err(|err| io::Error::new(ErrorKind::Other, err))
}

async fn publickey_auth(
    state: &DaemonState,
    user: &str,
    public_key: &PublicKey,
) -> Result<DaemonBackendAuthResponse, io::Error> {
    let mut url = state
        .config
        .ssh
        .auth_url
        .join("/api/internal/auth/publickey")
        .map_err(|err| io::Error::new(ErrorKind::Other, err))?;
    url.query_pairs_mut()
        .append_pair("node", &state.config.identifier);

    state
        .client
        .get(url)
        .basic_auth(user, Some(public_key.fingerprint()))
        .send()
        .await
        .inspect_err(|err| debug!("SSH Authentication failed (publickey): {:?}", err))
        .map_err(|err| io::Error::new(ErrorKind::Other, err))?
        .error_for_status()
        .inspect_err(|err| debug!("SSH Authentication failed (publickey): {:?}", err))
        .map_err(|err| match err {
            err if matches!(err.status(), Some(http::StatusCode::UNAUTHORIZED)) => {
                io::Error::new(ErrorKind::PermissionDenied, err)
            }
            err => io::Error::new(ErrorKind::Other, err),
        })?
        .json()
        .await
        .map_err(|err| io::Error::new(ErrorKind::Other, err))
}

impl InvalidationCacheMap<(String, String), DaemonBackendAuthResponse> {
    pub(crate) async fn get_resolved(
        &mut self,
        state: &DaemonState,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<DaemonBackendAuthResponse, io::Error> {
        match self.get(&(user.to_string(), public_key.fingerprint())) {
            None => publickey_auth(state, user, public_key).await,
            Some(response) => Ok(response.clone()),
        }
    }
}

#[async_trait]
impl Handler for DaemonSSHHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        #[cfg(feature = "ssh_debug_auth_bypass")]
        {
            use crate::schema::servers;
            use diesel::{QueryDsl, SelectableHelper};
            use diesel_async::RunQueryDsl;
            use std::ops::DerefMut;

            if cfg!(feature = "ssh_debug_auth_bypass") {
                let conn = self.state.conn.clone();
                let mut conn = conn.lock().await;
                let servers: Result<Vec<crate::schema::Server>, diesel::result::Error> =
                    servers::dsl::servers
                        .select(crate::schema::Server::as_select())
                        .load(conn.deref_mut())
                        .await;
                let mut auth =
                    Vec::with_capacity(servers.as_ref().map(Vec::len).unwrap_or_default());

                match servers {
                    Ok(servers) => {
                        for server in servers {
                            auth.push(crate::common::ssh::DaemonBackendAuthServer {
                                id: server.id,
                                name: server.id.to_string(),
                                permissions: BackendAuthServerPermissions::all(),
                            });
                        }
                    }
                    Err(err) => {
                        debug!("Failed to complete authentication: {:?}", err);
                        return Err(russh::Error::IO(io::Error::new(ErrorKind::Other, err)));
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
                }),
                err => Err(russh::Error::IO(err)),
            },
        }
    }

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &russh_keys::key::PublicKey,
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
        session.close(channel);
        Ok(())
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
                        session.channel_failure(channel_id);
                        return Ok(());
                    }
                    Some(cols) => cols.clone(),
                };
                let rows = match rows {
                    None => {
                        debug!("Size is None on window size");
                        session.channel_failure(channel_id);
                        return Ok(());
                    }
                    Some(rows) => rows.clone(),
                };
                cols.store(col_width, Ordering::Relaxed);
                rows.store(row_height, Ordering::Relaxed);
                match id {
                    None => {
                        debug!("Server is None on window size");
                        session.channel_failure(channel_id);
                    }
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
                        session.channel_success(channel_id);
                    }
                }
            }
            None => {
                debug!("Tried to access a non existent channel");
                session.channel_failure(channel_id);
            }
        }
        Ok(())
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
        let str = String::from_utf8(data.to_vec())
            .inspect_err(|_err| debug!("Command is not utf-8 encoded"))
            .map_err(|err| russh::Error::Utf8(err.utf8_error()))?;
        debug!("Exec {}", str);
        let server_id: u32 = str
            .parse()
            .inspect_err(|_err| debug!("Command is not parsable as server id to u32"))
            .map_err(|_err| russh::Error::DecryptionError)?;

        let auth = self.auth.as_ref().unwrap();
        let mut allowed = 0u8;
        for server in auth {
            if server.id == server_id {
                if server
                    .permissions
                    .contains(BackendAuthServerPermissions::TERMINAL_READ)
                {
                    if server
                        .permissions
                        .contains(BackendAuthServerPermissions::TERMINAL_WRITE)
                    {
                        allowed = 2;
                    } else {
                        allowed = 1;
                    }
                }
                break;
            }
        }
        if allowed == 0 {
            debug!("Terminal access denied. No permission");
            session.channel_failure(channel_id);
            return Ok(());
        }

        let mut servers = self.state.servers.lock().await;
        let server_data_arc = match servers.get(&server_id) {
            None => {
                servers.insert(server_id, Arc::new(Mutex::new(ServerData::default())));
                servers.get(&server_id).unwrap()
            }
            Some(server) => server,
        };
        let server = timeout(Duration::from_millis(100), server_data_arc.lock()).await;
        match server {
            Ok(server_data) => {
                let sender = server_data.sender.clone();
                let attach_write = server_data.attach_write.clone();

                match self.channels.get_mut(&channel_id) {
                    None => {
                        debug!("Tried to access a non existent channel");
                        session.channel_failure(channel_id);
                        Ok(())
                    }
                    Some((channel, id, cols, rows)) => {
                        *id = Some(server_id);
                        let channel = channel.take();
                        if channel.is_none() {
                            debug!("Channel already taken");
                            session.channel_failure(channel_id);
                            return Ok(());
                        }
                        let cols = match cols {
                            None => {
                                debug!("Size is None on exec");
                                session.channel_failure(channel_id);
                                return Ok(());
                            }
                            Some(cols) => cols.clone(),
                        };
                        let rows = match rows {
                            None => {
                                debug!("Size is None on exec");
                                session.channel_failure(channel_id);
                                return Ok(());
                            }
                            Some(rows) => rows.clone(),
                        };
                        drop(server_data);
                        tokio::task::spawn(new_session(
                            server_id,
                            (channel.unwrap().into_stream(), [0u8; 2048]),
                            TerminalData {
                                sender,
                                attach_write,
                                cols,
                                rows,
                                write: allowed == 2,
                            },
                            self.state.docker.clone(),
                            server_data_arc.clone(),
                        ));
                        session.channel_success(channel_id);
                        Ok(())
                    }
                }
            }
            Err(_timeout) => {
                debug!("Timed out on locking server data");
                Err(russh::Error::IO(io::Error::new(
                    ErrorKind::TimedOut,
                    "Timed out locking server.",
                )))
            }
        }
    }

    async fn pty_request(
        &mut self,
        channel_id: ChannelId,
        _term: &str,
        col_width: u32,
        _row_height: u32,
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
                    session.channel_failure(channel_id);
                    return Ok(());
                }
            }
            match rows {
                None => *rows = Some(Arc::new(AtomicU32::new(col_width))),
                Some(_) => {
                    debug!("Size is Some on pty");
                    session.channel_failure(channel_id);
                    return Ok(());
                }
            }
        }
        session.channel_success(channel_id);
        Ok(())
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
                    session.channel_failure(channel_id);
                }
                Some((channel, _id, _cols, _rows)) => {
                    let channel = channel.take();
                    if channel.is_none() {
                        debug!("Channel was already taken on subsystem request");
                        session.channel_failure(channel_id);
                        return Ok(());
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
                            if let Ok((_path, lock, _, storage)) = storage_read(
                                &self.state.config,
                                self.state.conn.lock().await.deref_mut(),
                                &self.state.storages,
                                identifier,
                                true,
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
                    }

                    session.channel_success(channel_id);
                    russh_sftp::server::run(channel.unwrap().into_stream(), sftp).await;
                }
            }
        } else {
            debug!("Tried to access a non existent subsystem: {:?}", name);
            session.channel_failure(channel_id);
        }
        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter<Result<usize, io::Error>, Result<(), io::Error>, Result<(), io::Error>>
    for (ChannelStream<Msg>, [u8; 2048])
{
    async fn read(&mut self) -> Result<usize, io::Error> {
        self.0.read(&mut self.1).await
    }

    fn read_transform(&mut self, data: Result<usize, io::Error>) -> Result<Vec<u8>, ChannelError>
    where
        Self: Sized,
    {
        match data {
            Ok(size) => Ok(self.1[0..size].to_vec()),
            Err(err) if err.kind() == ErrorKind::UnexpectedEof => Err(ChannelError::Closed),
            Err(err) => Err(ChannelError::IO(err)),
        }
    }

    async fn write(&mut self, data: ChannelWriteData) -> Result<(), io::Error> {
        if let ChannelWriteData::Terminal(data) = data {
            self.0.write_all(&data).await
        } else {
            Ok(())
        }
    }

    fn write_transform(&mut self, data: Result<(), io::Error>) -> Result<(), ChannelError>
    where
        Self: Sized,
    {
        data.map_err(ChannelError::IO)
    }

    async fn flush(&mut self) -> Result<(), io::Error> {
        self.0.flush().await
    }

    async fn close(mut self) {
        let _ = self.0.shutdown().await;
    }

    fn send_ping() -> bool
    where
        Self: Sized,
    {
        false
    }
}
