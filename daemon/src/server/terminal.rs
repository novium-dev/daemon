/*
 * Copyright (C) 2025 Markus Probst
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, version 3.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 */
use crate::server::FetchedServer;
#[cfg(not(feature = "no_tty_size_please"))]
use crate::server::stop::ServerStopError;
use crate::utils::ChannelWriteData;
#[cfg(not(feature = "no_tty_size_please"))]
use bollard::Docker;
#[cfg(not(feature = "no_tty_size_please"))]
use bollard::query_parameters::ResizeContainerTTYOptions;
use ringbuffer::AllocRingBuffer;
#[cfg(not(feature = "no_tty_size_please"))]
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

    #[cfg(not(feature = "no_tty_size_please"))]
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

    #[cfg(not(feature = "no_tty_size_please"))]
    pub(crate) async fn update_tty_size(
        id: u32,
        docker: &Docker,
        server_data: &mut crate::ServerData,
        cols: u16,
        rows: u16,
    ) -> Result<(), ServerStopError> {
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
                    w: i32::from(cols),
                    h: i32::from(rows),
                },
            )
            .await
            .map_err(ServerStopError::Docker)?;
        Ok(())
    }
}
