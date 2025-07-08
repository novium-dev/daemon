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
use crate::utils::{ChannelAdapter, ChannelError, ChannelWriteData};
use async_trait::async_trait;
use axum::Error;
use axum::extract::ws::{Message, WebSocket};
use bytes::Bytes;
use futures_util::SinkExt;
use std::io;

impl ChannelWriteData {
    pub fn into_websocket_message(self) -> Message {
        match self {
            Self::Terminal(data) => Message::Binary(data.into()),
            Self::Status(state) => state.as_websocket_message(),
            Self::Ping => Message::Ping(Bytes::new()),
            Self::Stats(stats) => stats.as_websocket_message(),
        }
    }
}

#[async_trait]
impl ChannelAdapter<Option<Result<Message, Error>>, Result<(), Error>, Result<(), Error>>
    for WebSocket
{
    async fn read(&mut self) -> Option<Result<Message, Error>> {
        let message = self.recv().await;
        if let Some(Ok(Message::Ping(ref data))) = message {
            let _ = self.send(Message::Pong(data.clone())).await;
        }
        message
    }

    fn read_transform(
        &mut self,
        data: Option<Result<Message, Error>>,
    ) -> Result<Vec<u8>, ChannelError>
    where
        Self: Sized,
    {
        match data {
            Some(Ok(Message::Binary(data))) => Ok(data.into()),
            Some(Ok(Message::Text(data))) => Ok(data.to_string().into_bytes()),
            Some(Ok(Message::Close(_))) | None => Err(ChannelError::Closed),
            Some(Ok(_)) => Ok(Vec::new()),
            Some(Err(err)) => Err(ChannelError::IO(io::Error::other(err))),
        }
    }

    async fn write(&mut self, data: ChannelWriteData) -> Result<(), Error> {
        self.send(data.into_websocket_message()).await
    }

    fn write_transform(&mut self, data: Result<(), Error>) -> Result<(), ChannelError>
    where
        Self: Sized,
    {
        data.map_err(|err| ChannelError::IO(io::Error::other(err)))
    }

    async fn flush(&mut self) -> Result<(), Error> {
        SinkExt::flush(self).await
    }

    async fn close(mut self) {
        let _ = self.send(Message::Close(None)).await;
    }

    fn send_ping() -> bool
    where
        Self: Sized,
    {
        true
    }
}
