use crate::DaemonState;
use crate::utils::{ChannelAdapter, ChannelError, ChannelWriteData};
use async_trait::async_trait;
use axum::Error;
use axum::extract::ws::{Message, WebSocket};
use bytes::Bytes;
use futures_util::SinkExt;
use serde::Deserialize;
use std::io;
use std::io::ErrorKind;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) mod exec;
pub(crate) mod stats;
pub(crate) mod terminal;

#[derive(Deserialize)]
pub(crate) struct WebsocketTerminalQuery {
    verify: String,
    timestamp: String,
    cols: u32,
    rows: u32,
    write: bool,
}

impl ChannelWriteData {
    pub fn into_websocket_message(self) -> Message {
        match self {
            ChannelWriteData::Terminal(data) => Message::Binary(data.into()),
            ChannelWriteData::Status(state) => state.as_websocket_message(),
            ChannelWriteData::Ping => Message::Ping(Bytes::new()),
            ChannelWriteData::Stats(stats) => stats.as_websocket_message(),
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
            None => Err(ChannelError::Closed),
            Some(Ok(Message::Binary(data))) => Ok(data.into()),
            Some(Ok(Message::Text(data))) => Ok(data.to_string().into_bytes()),
            Some(Ok(Message::Close(_))) => Err(ChannelError::Closed),
            Some(Ok(_)) => Ok(Vec::new()),
            Some(Err(err)) => Err(ChannelError::IO(io::Error::new(ErrorKind::Other, err))),
        }
    }

    async fn write(&mut self, data: ChannelWriteData) -> Result<(), Error> {
        self.send(data.into_websocket_message()).await
    }

    fn write_transform(&mut self, data: Result<(), Error>) -> Result<(), ChannelError>
    where
        Self: Sized,
    {
        data.map_err(|err| ChannelError::IO(io::Error::new(ErrorKind::Other, err)))
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

pub(crate) fn user_config() -> OpenApiRouter<DaemonState> {
    OpenApiRouter::new()
        .routes(routes!(terminal::websocket))
        .routes(routes!(stats::websocket))
        .routes(routes!(exec::websocket))
}
