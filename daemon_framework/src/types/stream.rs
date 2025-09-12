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
use axum::body::BodyDataStream;
use axum::extract::{FromRequest, Request};
use bytes::Bytes;
use core::pin::Pin;
use futures_util::Stream;
use std::io;
use tokio::io::AsyncRead;
use tokio_util::io::{ReaderStream, StreamReader};

/// Universal type for data streams over the network.
///
/// It can be received using axum and be used in a request with reqwest.
pub enum DataStream {
    Axum(BodyDataStream),
    Read(Pin<Box<dyn AsyncRead + Send + 'static>>),
    Stream(Pin<BoxedStream>),
}

type BoxedStream = Box<dyn Stream<Item = Result<Bytes, io::Error>> + Send>;
impl DataStream {
    #[must_use]
    pub fn by_read<T: AsyncRead + Send + 'static>(read: T) -> Self {
        Self::Read(Box::pin(read))
    }

    #[must_use]
    pub fn by_stream<T: Stream<Item = Result<Bytes, io::Error>> + Send + 'static>(
        stream: T,
    ) -> Self {
        Self::Stream(Box::pin(stream))
    }

    #[must_use]
    pub fn into_axum(self) -> axum::body::Body {
        match self {
            Self::Axum(stream) => axum::body::Body::from_stream(stream),
            Self::Stream(stream) => axum::body::Body::from_stream(stream),
            Self::Read(read) => axum::body::Body::from_stream(ReaderStream::new(read)),
        }
    }

    #[must_use]
    pub fn into_reqwest(self) -> reqwest::Body {
        match self {
            Self::Axum(stream) => reqwest::Body::wrap_stream(stream),
            Self::Stream(stream) => reqwest::Body::wrap_stream(stream),
            Self::Read(read) => reqwest::Body::wrap_stream(ReaderStream::new(read)),
        }
    }

    #[must_use]
    pub fn reader(self) -> StreamReader<Pin<BoxedStream>, Bytes> {
        use futures_util::TryStreamExt as _;
        StreamReader::new(match self {
            Self::Axum(stream) => Box::pin(stream.map_err(io::Error::other)),
            Self::Stream(stream) => stream,
            Self::Read(read) => Box::pin(ReaderStream::new(read)),
        })
    }
}

impl<S: Send + Sync> FromRequest<S> for DataStream {
    type Rejection = ();

    async fn from_request(req: Request, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self::Axum(req.into_body().into_data_stream()))
    }
}
