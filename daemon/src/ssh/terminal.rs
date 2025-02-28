use crate::ServerData;
use crate::ssh::handler::DaemonSSHHandler;
use crate::sync::Mutex;
use crate::terminal::{TerminalData, new_session};
use crate::utils::{ChannelAdapter, ChannelError, ChannelWriteData};
use async_trait::async_trait;
use daemon_common::server::BackendAuthServerPermissions;
use russh::server::{Msg, Session};
use russh::{ChannelId, ChannelStream};
use sqlx::__rt::timeout;
use std::io::{self, ErrorKind};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::debug;

impl DaemonSSHHandler {
    pub(crate) async fn ssh_terminal(
        &mut self,
        channel_id: ChannelId,
        session: &mut Session,
        server_id: String,
    ) -> Result<(), russh::Error> {
        let auth = self.auth.as_ref().unwrap();

        let server = Self::find_server(auth, &server_id);
        let (internal_server_id, write) = match server {
            None => {
                debug!("Terminal access denied. Server not found");
                return session.channel_failure(channel_id);
            }
            Some(server) => {
                if !server
                    .permissions
                    .contains(BackendAuthServerPermissions::TERMINAL_READ)
                {
                    debug!("Terminal access denied. No permission");
                    return session.channel_failure(channel_id);
                }
                (
                    server.id,
                    server
                        .permissions
                        .contains(BackendAuthServerPermissions::TERMINAL_WRITE),
                )
            }
        };

        let mut servers = self.state.servers.lock().await;
        let server_data_arc = match servers.get(&internal_server_id) {
            None => {
                servers.insert(
                    internal_server_id,
                    Arc::new(Mutex::new(ServerData::default())),
                );
                servers.get(&internal_server_id).unwrap()
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
                        session.channel_failure(channel_id)
                    }
                    Some((channel, id, cols, rows)) => {
                        *id = Some(internal_server_id);
                        let channel = channel.take();
                        if channel.is_none() {
                            debug!("Channel already taken");
                            return session.channel_failure(channel_id);
                        }
                        let cols = match cols {
                            None => {
                                debug!("Size is None on exec");
                                return session.channel_failure(channel_id);
                            }
                            Some(cols) => cols.clone(),
                        };
                        let rows = match rows {
                            None => {
                                debug!("Size is None on exec");
                                return session.channel_failure(channel_id);
                            }
                            Some(rows) => rows.clone(),
                        };
                        drop(server_data);
                        tokio::task::spawn(new_session(
                            internal_server_id,
                            (channel.unwrap().into_stream(), [0u8; 2048]),
                            TerminalData {
                                sender,
                                attach_write,
                                cols,
                                rows,
                                write,
                            },
                            self.state.docker.clone(),
                            server_data_arc.clone(),
                        ));
                        session.channel_success(channel_id)
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
