// SPDX-License-Identifier: AGPL-3.0-only
use crate::DaemonExtra;
use crate::ssh::handler::{DaemonChannel, DaemonSSHHandler};
use crate::terminal::{TerminalData, TerminalSessionOrigin, new_session};
use crate::utils::{ChannelAdapter, ChannelError, ChannelWriteData};
use async_trait::async_trait;
use daemon_common::node::BackendAuthServerPermissions;
use daemon_common::ssh::DaemonBackendAuthServer;
use russh::server::{Handle, Msg, Session};
use russh::{ChannelId, ChannelStream};
use std::io::{self, ErrorKind};
use std::sync::Arc;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tracing::debug;

/// Ctrl+C. Detaches the SSH session instead of being sent to the server
const DETACH_BYTE: u8 = 0x03;

pub(crate) struct SshTerminal {
    stream: ChannelStream<Msg>,
    buffer: [u8; 2048],
    handle: Handle,
    channel_id: ChannelId,
}

impl<T: DaemonExtra> DaemonSSHHandler<T> {
    pub(crate) async fn ssh_terminal(
        &mut self,
        channel_id: ChannelId,
        session: &mut Session,
        server: Option<DaemonBackendAuthServer>,
    ) -> Result<(), russh::Error> {
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

        let server_data = Arc::clone(
            self.daemon
                .servers
                .lock()
                .await
                .entry(internal_server_id)
                .or_default(),
        );
        let sender = Arc::clone(&server_data.sender);
        let attach_write = Arc::clone(&server_data.attach_write);

        match self.channels.get_mut(&channel_id) {
            None => {
                debug!("Tried to access a non existent channel");
                session.channel_failure(channel_id)
            }
            Some(DaemonChannel {
                channel,
                server_id,
                cols,
                rows,
            }) => {
                *server_id = Some(internal_server_id);
                let Some(channel) = channel.take() else {
                    debug!("Channel already taken");
                    return session.channel_failure(channel_id);
                };
                let cols = match cols {
                    None => {
                        debug!("Size is None on exec");
                        return session.channel_failure(channel_id);
                    }
                    Some(cols) => Arc::clone(cols),
                };
                let rows = match rows {
                    None => {
                        debug!("Size is None on exec");
                        return session.channel_failure(channel_id);
                    }
                    Some(rows) => Arc::clone(rows),
                };
                tokio::task::spawn(new_session(
                    internal_server_id,
                    SshTerminal {
                        stream: channel.into_stream(),
                        buffer: [0u8; 2048],
                        handle: session.handle(),
                        channel_id,
                    },
                    TerminalData {
                        sender,
                        attach_write,
                        cols,
                        rows,
                        origin: TerminalSessionOrigin::Ssh,
                        write,
                    },
                    Arc::clone(&self.daemon.docker),
                    Arc::clone(&server_data),
                ));
                session.channel_success(channel_id)
            }
        }
    }
}

#[async_trait]
impl ChannelAdapter<Result<usize, io::Error>, Result<(), io::Error>, Result<(), io::Error>>
    for SshTerminal
{
    async fn read(&mut self) -> Result<usize, io::Error> {
        self.stream.read(&mut self.buffer).await
    }

    fn read_transform(&mut self, data: Result<usize, io::Error>) -> Result<Vec<u8>, ChannelError>
    where
        Self: Sized,
    {
        match data {
            Ok(size) => {
                let data = self.buffer.get(0..size).ok_or_else(|| {
                    ChannelError::IO(io::Error::other("Bug: Size larger than buffer"))
                })?;
                if data.contains(&DETACH_BYTE) {
                    debug!("Detaching SSH terminal session");
                    return Err(ChannelError::Closed);
                }
                Ok(data.to_vec())
            }
            Err(err) if err.kind() == ErrorKind::UnexpectedEof => Err(ChannelError::Closed),
            Err(err) => Err(ChannelError::IO(err)),
        }
    }

    async fn write(&mut self, data: ChannelWriteData) -> Result<(), io::Error> {
        if let ChannelWriteData::Terminal(data) = data {
            self.stream.write_all(&data).await
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
        self.stream.flush().await
    }

    async fn close(mut self) {
        let _ = self.stream.shutdown().await;
        let _ = self.handle.exit_status_request(self.channel_id, 0).await;
        let _ = self.handle.close(self.channel_id).await;
    }

    fn send_ping() -> bool
    where
        Self: Sized,
    {
        false
    }
}
