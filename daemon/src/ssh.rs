mod sftp;

use crate::ssh::sftp::DaemonSftpSession;
use crate::storage::StorageIdentifiers;
use crate::terminal::{new_session, update_size, TerminalData};
use crate::utils::{ChannelAdapter, ChannelError, ChannelWriteData};
use crate::{DaemonState, ServerData};
use axum::async_trait;
use log::{debug, error};
use russh::server::{Auth, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId, ChannelStream, Pty};
use std::collections::HashMap;
use std::io::{self, ErrorKind};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::time::timeout;

#[derive(Clone)]
pub struct DaemonSSHServer {
    pub(crate) state: Arc<DaemonState>,
}

impl Server for DaemonSSHServer {
    type Handler = DaemonSSHHandler;

    fn new_client(&mut self, _peer_addr: Option<SocketAddr>) -> Self::Handler {
        DaemonSSHHandler {
            state: self.state.clone(),
            channels: HashMap::new(),
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
}

#[async_trait]
impl Handler for DaemonSSHHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn auth_publickey(
        &mut self,
        _user: &str,
        _public_key: &russh_keys::key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
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
        debug!("Exec");
        let str =
            String::from_utf8(data.to_vec()).map_err(|err| russh::Error::Utf8(err.utf8_error()))?;
        let server_id: u32 = str.parse().map_err(|_err| russh::Error::DecryptionError)?;

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
        debug!("Subsystem Request {}", name);

        if name == "sftp" {
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
                    let storage = StorageIdentifiers::Primary(1);
                    sftp.access.insert(storage.as_vec(), storage);

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
