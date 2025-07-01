use crate::server::FetchedServer;
#[cfg(not(feature = "no_tty_size_please"))]
use crate::server::stop::ServerStopError;
use crate::utils::ChannelWriteData;
#[cfg(not(feature = "no_tty_size_please"))]
use bollard::Docker;
use bollard::query_parameters::ResizeContainerTTYOptions;
use ringbuffer::AllocRingBuffer;
#[cfg(not(feature = "no_tty_size_please"))]
use std::collections::HashSet;
#[cfg(not(feature = "no_tty_size_please"))]
use std::io;
use termion::color;
use termion::color::Fg;
use tokio::sync::broadcast::Sender;

impl FetchedServer {
    pub(crate) fn prefix() -> String {
        format!("{}[Daemon] ", Fg(color::Yellow))
    }

    pub(crate) fn console(
        sender: &Sender<ChannelWriteData>,
        buffer: &mut AllocRingBuffer<u8>,
        msg: String,
    ) {
        let bytes = msg.into_bytes();
        buffer.extend(bytes.clone());
        let _ = sender.send(ChannelWriteData::Terminal(bytes));
    }

    pub(crate) fn log(
        sender: &Sender<ChannelWriteData>,
        buffer: &mut AllocRingBuffer<u8>,
        msg: &str,
    ) {
        Self::console(
            sender,
            buffer,
            format!("\r{}{}{}\r\n", Self::prefix(), msg, Fg(color::Reset)),
        );
    }

    #[cfg(not(feature = "no_tty_size_please"))]
    pub(crate) async fn update_exec_tty_size(
        id: String,
        docker: &Docker,
        cols: u32,
        rows: u32,
    ) -> Result<(), ServerStopError> {
        if cols > u16::MAX as u32 || rows > u16::MAX as u32 {
            return Err(ServerStopError::Docker(bollard::errors::Error::IOError {
                err: io::Error::other("Integer overflow"),
            }));
        }
        docker
            .resize_exec(
                &id,
                bollard::exec::ResizeExecOptions {
                    width: cols as u16,
                    height: rows as u16,
                },
            )
            .await
            .map_err(ServerStopError::Docker)
    }

    #[cfg(not(feature = "no_tty_size_please"))]
    pub(crate) async fn update_tty_size(
        id: u32,
        docker: &Docker,
        server_data: &mut crate::ServerData,
        cols: u32,
        rows: u32,
    ) -> Result<(), ServerStopError> {
        if cols > u16::MAX as u32 || rows > u16::MAX as u32 {
            return Err(ServerStopError::Docker(bollard::errors::Error::IOError {
                err: io::Error::other("Integer overflow"),
            }));
        }
        server_data.size = (cols, rows);
        if !Self::state(id, HashSet::new(), docker, Some(server_data))
            .await
            .map_err(ServerStopError::Docker)?
            .stop()
        {
            return Err(ServerStopError::NotRunning);
        }
        docker
            .resize_container_tty(
                &Self::container_name(id),
                ResizeContainerTTYOptions {
                    w: cols as i32,
                    h: rows as i32,
                },
            )
            .await
            .map_err(ServerStopError::Docker)?;
        Ok(())
    }
}
