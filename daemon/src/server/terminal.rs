// SPDX-License-Identifier: AGPL-3.0-only
use crate::server::FetchedServer;
use crate::server::stop::ServerStopError;
use crate::utils::ChannelWriteData;
use bollard::Docker;
use ringbuffer::AllocRingBuffer;
use std::collections::HashSet;
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

    pub(crate) async fn update_exec_tty_size(
        id: String,
        docker: &Docker,
        cols: u16,
        rows: u16,
    ) -> Result<(), ServerStopError> {
        docker
            .resize_exec(
                &id,
                bollard::exec::ResizeExecOptions {
                    width: cols,
                    height: rows,
                },
            )
            .await
            .map_err(ServerStopError::Docker)
    }

    pub(crate) async fn update_tty_size(
        id: u32,
        docker: &Docker,
        server_data: &crate::ServerData,
        cols: u16,
        rows: u16,
    ) -> Result<(), ServerStopError> {
        use bollard::query_parameters::ResizeContainerTTYOptions;
        use core::sync::atomic::Ordering;

        server_data.size.0.store(cols, Ordering::Relaxed);
        server_data.size.1.store(rows, Ordering::Relaxed);
        if !Self::state(id, &HashSet::new(), docker, server_data)
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
                    w: i32::from(cols),
                    h: i32::from(rows),
                },
            )
            .await
            .map_err(ServerStopError::Docker)?;
        Ok(())
    }
}
