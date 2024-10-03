pub(crate) mod stats;
pub(crate) mod terminal;

use crate::utils::{ChannelAdapter, ChannelError, ChannelWriteData};
use crate::DaemonState;
use axum::extract::ws::{Message, WebSocket};
use axum::http::StatusCode;
use axum::{async_trait, Error};
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use futures_util::SinkExt;
use log::debug;
use serde::Deserialize;
use std::io;
use std::io::ErrorKind;

#[derive(Deserialize)]
pub(crate) struct WebsocketTerminalQuery {
    verify: String,
    timestamp: String,
    cols: u32,
    rows: u32,
}

#[derive(Deserialize)]
pub(crate) struct WebsocketStatsQuery {
    verify: String,
    timestamp: String,
}

pub(crate) async fn authenticate(
    #[allow(unused)] server_id: u32,
    #[allow(unused)] state: &DaemonState,
    timestamp: &str,
    verify: &String,
) -> Result<(), StatusCode> {
    debug!("Authenticating WebSocket...");
    #[allow(unused)]
    let timestamp: u128 = timestamp.parse().map_err(|_err| StatusCode::BAD_REQUEST)?;
    let verify = BASE64_STANDARD
        .decode(verify.as_bytes())
        .map_err(|_err| StatusCode::BAD_REQUEST)?;
    #[cfg(feature = "websocket_debug_auth_bypass")]
    if !verify.eq("bypass".as_bytes()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    #[cfg(not(feature = "websocket_debug_auth_bypass"))]
    {
        use ed25519_dalek::{Signature, Verifier};
        use std::time::{SystemTime, UNIX_EPOCH};

        let signature_bytes: [u8; 64] =
            verify.try_into().map_err(|_err| StatusCode::BAD_REQUEST)?;
        state
            .config
            .websocket_key
            .verify(
                format!("{}-{}-{}", state.config.identifier, server_id, timestamp).as_bytes(),
                &Signature::from_bytes(&signature_bytes),
            )
            .map_err(|_err| StatusCode::UNAUTHORIZED)?;
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_err| StatusCode::BAD_REQUEST)?;
        if (timestamp + state.config.websocket_allow_duration as u128) <= unix.as_millis() {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    debug!("WebSocket authentication passed.");
    Ok(())
}

impl ChannelWriteData {
    pub fn into_websocket_message(self) -> Message {
        match self {
            ChannelWriteData::Terminal(data) => Message::Binary(data),
            ChannelWriteData::Status(state) => state.as_websocket_message(),
            ChannelWriteData::Ping => Message::Ping(Vec::new()),
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
            Some(Ok(Message::Binary(data))) => Ok(data),
            Some(Ok(Message::Text(data))) => Ok(data.into_bytes()),
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

    async fn close(self) {
        let _ = self.close().await;
    }

    fn send_ping() -> bool
    where
        Self: Sized,
    {
        true
    }
}
