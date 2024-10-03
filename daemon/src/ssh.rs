#[cfg(feature = "sftp")]
pub(crate) mod sftp;

#[cfg(feature = "sftp")]
use crate::ssh::sftp::DaemonSftpSession;
#[cfg(feature = "sftp")]
use crate::storage::StorageIdentifier;
use crate::terminal::{new_session, update_size, TerminalData};
use crate::utils::{ChannelAdapter, ChannelError, ChannelWriteData};
use crate::{DaemonState, ServerData};
use axum::{async_trait, http};
use bitflags::bitflags;
use log::{debug, error};
use russh::server::{Auth, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId, ChannelStream, Pty};
use russh_keys::key::PublicKey;
use serde::Deserialize;
use std::collections::HashMap;
use std::hash::Hash;
use std::io::{self, ErrorKind};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::time::timeout;

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

pub(crate) type PanelAuthResponse = Vec<PanelAuthServer>;
#[derive(Deserialize, Clone)]
pub(crate) struct PanelAuthServer {
    pub(crate) id: u32,
    pub(crate) permissions: PanelAuthServerPermissions,
}

bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
    pub struct PanelAuthServerPermissions: i32 {
        const TERMINAL_READ = 1 << 0;
        const TERMINAL_WRITE = 1 << 1;

        const SFTP_READ = 1 << 2;
        const SFTP_WRITE = 1 << 3;
        const SFTP_DELETE = 1 << 4;
        const SFTP_CREATE = 1 << 5;
    }
}

pub struct DaemonSSHHandler {
    state: Arc<DaemonState>,
    channels: HashMap<ChannelId, DaemonChannel>,
    publickey_cache: InvalidationCacheMap<(String, String), PanelAuthResponse>,
    auth: Option<PanelAuthResponse>,
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
) -> Result<PanelAuthResponse, io::Error> {
    state
        .client
        .get(format!(
            "{}/api/auth/password?node={}",
            state.config.ssh_auth_url, state.config.identifier
        ))
        .basic_auth(user, Some(password))
        .send()
        .await
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
) -> Result<PanelAuthResponse, io::Error> {
    state
        .client
        .get(format!(
            "{}/api/auth/publickey?node={}",
            state.config.ssh_auth_url, state.config.identifier
        ))
        .basic_auth(user, Some(public_key.fingerprint()))
        .send()
        .await
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

impl InvalidationCacheMap<(String, String), PanelAuthResponse> {
    pub(crate) async fn get_resolved(
        &mut self,
        state: &DaemonState,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<PanelAuthResponse, io::Error> {
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
            use diesel::SelectableHelper;
            use diesel::{QueryDsl, RunQueryDsl};
            use std::ops::DerefMut;

            if cfg!(feature = "ssh_debug_auth_bypass") {
                let mut auth = Vec::new();
                let conn = self.state.conn.clone();
                let servers: Result<Vec<crate::schema::Server>, diesel::result::Error> =
                    tokio::task::spawn_blocking(move || {
                        let mut conn = conn.blocking_lock();
                        let server = servers::dsl::servers
                            .select(crate::schema::Server::as_select())
                            .load(conn.deref_mut())?;
                        Ok(server)
                    })
                    .await
                    .map_err(russh::Error::Join)?;

                match servers {
                    Ok(servers) => {
                        for server in servers {
                            auth.push(PanelAuthServer {
                                id: server.id,
                                permissions: PanelAuthServerPermissions::all(),
                            });
                        }
                    }
                    Err(err) => {
                        return Err(russh::Error::IO(io::Error::new(ErrorKind::Other, err)));
                    }
                }

                self.auth = Some(auth);
                return Ok(Auth::Accept);
            }
        }
        match password_auth(&self.state, user, password).await {
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
        debug!("Window size change");
        match self.channels.get_mut(&channel_id) {
            Some((_channel, id, cols, rows)) => {
                let cols = match cols {
                    None => {
                        session.channel_failure(channel_id);
                        return Ok(());
                    }
                    Some(cols) => cols.clone(),
                };
                let rows = match rows {
                    None => {
                        session.channel_failure(channel_id);
                        return Ok(());
                    }
                    Some(rows) => rows.clone(),
                };
                cols.store(col_width, Ordering::Relaxed);
                rows.store(row_height, Ordering::Relaxed);
                match id {
                    None => {
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
        debug!("Exec");
        let str =
            String::from_utf8(data.to_vec()).map_err(|err| russh::Error::Utf8(err.utf8_error()))?;
        let server_id: u32 = str.parse().map_err(|_err| russh::Error::DecryptionError)?;

        let auth = self.auth.as_ref().unwrap();
        let mut allowed = 0u8;
        for server in auth {
            if server.id == server_id {
                if server
                    .permissions
                    .contains(PanelAuthServerPermissions::TERMINAL_READ)
                {
                    if server
                        .permissions
                        .contains(PanelAuthServerPermissions::TERMINAL_WRITE)
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
                        session.channel_failure(channel_id);
                        Ok(())
                    }
                    Some((channel, id, cols, rows)) => {
                        *id = Some(server_id);
                        let channel = channel.take();
                        if channel.is_none() {
                            session.channel_failure(channel_id);
                            return Ok(());
                        }
                        let cols = match cols {
                            None => {
                                session.channel_failure(channel_id);
                                return Ok(());
                            }
                            Some(cols) => cols.clone(),
                        };
                        let rows = match rows {
                            None => {
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
            Err(_timeout) => Err(russh::Error::IO(io::Error::new(
                ErrorKind::TimedOut,
                "Timed out locking server.",
            ))),
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
                    session.channel_failure(channel_id);
                    return Ok(());
                }
            }
            match rows {
                None => *rows = Some(Arc::new(AtomicU32::new(col_width))),
                Some(_) => {
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
                    session.channel_failure(channel_id);
                }
                Some((channel, _id, _cols, _rows)) => {
                    let channel = channel.take();
                    if channel.is_none() {
                        session.channel_failure(channel_id);
                        return Ok(());
                    }
                    let mut sftp = DaemonSftpSession {
                        access: HashMap::new(),
                        file_handles: HashMap::new(),
                        state: self.state.clone(),
                        version: None,
                    };

                    for server in self.auth.as_ref().unwrap() {
                        if server
                            .permissions
                            .contains(PanelAuthServerPermissions::SFTP_READ)
                        {
                            let identifier = StorageIdentifier::Primary(server.id);
                            sftp.access
                                .insert(identifier.as_vec(), (identifier, server.permissions));
                        }
                    }

                    session.channel_success(channel_id);
                    russh_sftp::server::run(channel.unwrap().into_stream(), sftp).await;
                }
            }
        } else {
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
