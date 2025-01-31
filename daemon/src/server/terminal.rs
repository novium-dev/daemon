use crate::server::stop::ServerStopError;
use crate::server::FetchedServer;
use crate::utils::ChannelWriteData;
use crate::ServerData;
use bollard::container::ResizeContainerTtyOptions;
use bollard::exec::ResizeExecOptions;
use bollard::Docker;
use ringbuffer::AllocRingBuffer;
use std::collections::HashSet;
use std::io;
use std::io::ErrorKind;
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

    #[allow(dead_code)]
    pub(crate) async fn update_exec_tty_size(
        id: String,
        docker: &Docker,
        cols: u32,
        rows: u32,
    ) -> Result<(), ServerStopError> {
        if cols > u16::MAX as u32 || rows > u16::MAX as u32 {
            return Err(ServerStopError::Docker(bollard::errors::Error::IOError {
                err: io::Error::new(ErrorKind::Other, "Integer overflow"),
            }));
        }
        docker
            .resize_exec(
                &id,
                ResizeExecOptions {
                    width: cols as u16,
                    height: rows as u16,
                },
            )
            .await
            .map_err(ServerStopError::Docker)
    }

    pub(crate) async fn update_tty_size(
        id: u32,
        docker: &Docker,
        server_data: &mut ServerData,
        cols: u32,
        rows: u32,
    ) -> Result<(), ServerStopError> {
        if cols > u16::MAX as u32 || rows > u16::MAX as u32 {
            return Err(ServerStopError::Docker(bollard::errors::Error::IOError {
                err: io::Error::new(ErrorKind::Other, "Integer overflow"),
            }));
        }
        server_data.size = (cols, rows);
        if !Self::state(id, HashSet::new(), docker, server_data)
            .await
            .map_err(ServerStopError::Docker)?
            .stop()
        {
            return Err(ServerStopError::NotRunning);
        }
        docker
            .resize_container_tty(
                &Self::container_name(id),
                ResizeContainerTtyOptions {
                    width: cols as u16,
                    height: rows as u16,
                },
            )
            .await
            .map_err(ServerStopError::Docker)?;
        Ok(())
    }
}
