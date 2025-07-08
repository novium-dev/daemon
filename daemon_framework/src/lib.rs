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
#![expect(
    clippy::arbitrary_source_item_ordering,
    reason = "This order is more readable"
)]

#[cfg(feature = "ts")]
pub mod ts;

pub use async_trait::async_trait;
pub use axum;
pub use axum_extra;
pub use daemon_framework_macros::define_error;
pub use daemon_framework_macros::error;
pub use daemon_framework_macros::service;
pub use daemon_framework_macros::ts;
pub use http;
pub use paste;
pub use reqwest;
pub use serde;
pub use serde_json;
pub use serde_urlencoded;
pub use sqlx;
pub use tap;
#[cfg(feature = "ts")]
pub use ts_rs;
pub use url;
pub use urlencoding;
pub use utoipa;
pub use utoipa_axum;

use axum::body::BodyDataStream;
use axum::extract::{FromRequest, Request};
use axum::response::IntoResponse;
use bytes::Bytes;
use core::any::TypeId;
use core::mem::{self, MaybeUninit};
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use futures_util::{Stream, TryStreamExt as _};
use http::HeaderMap;
use reqwest::Url;
use serde::de::Error as _;
use serde::ser::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashMap;
use std::io;
use tokio::io::AsyncRead;
use tokio_util::io::{ReaderStream, StreamReader};
use utoipa::ToSchema;
use utoipa::openapi::schema::RefBuilder;
use utoipa::openapi::{Content, RefOr, Response, ResponsesBuilder, Schema};
use utoipa_axum::router::OpenApiRouter;

#[doc(hidden)]
pub mod internal {
    use crate::MaybeUnit;
    use axum::Json;
    use axum::response::{IntoResponse, Response};
    use axum::routing::MethodRouter;
    use axum_extra::TypedHeader;
    use axum_extra::headers::{Header, HeaderMapExt as _};
    use core::fmt::{Debug, Display, Formatter};
    use core::str::FromStr;
    use http::{HeaderMap, HeaderName, StatusCode};
    use serde::de::{DeserializeOwned, Error as _};
    use serde::ser::Impossible;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use thiserror::Error;
    use utoipa::PartialSchema as _;
    use utoipa::openapi::path::OperationBuilder;
    use utoipa::openapi::{Object, RefOr, Schema, Type};
    use utoipa_axum::PathItemExt as _;
    use utoipa_axum::router::OpenApiRouter;

    trait TypedHeaderExt<T: Header> {
        fn inner(self) -> T;
    }

    impl<T: Header> TypedHeaderExt<T> for TypedHeader<T> {
        fn inner(self) -> T {
            self.0
        }
    }

    pub struct FrameworkJsonResponse<T>(pub T);

    impl<T: Serialize + MaybeUnit + 'static> IntoResponse for FrameworkJsonResponse<T> {
        fn into_response(self) -> Response {
            if T::unit().is_some() {
                StatusCode::NO_CONTENT.into_response()
            } else {
                Json(self.0).into_response()
            }
        }
    }

    impl<T: DeserializeOwned + MaybeUnit + 'static> FrameworkJsonResponse<T> {
        pub async fn from_reqwest(response: reqwest::Response) -> Result<Self, reqwest::Error> {
            if let Some(unit) = T::unit() {
                Ok(Self(unit))
            } else {
                response.json().await.map(Self)
            }
        }
    }

    pub trait FrameworkServerImpl {
        fn routes(&self) -> OpenApiRouter
        where
            Self: Sized,
        {
            OpenApiRouter::new().routes((
                vec![("Result".to_owned(), <()>::schema())],
                {
                    let mut paths = utoipa::openapi::path::Paths::new();
                    paths.add_path_operation(
                        "",
                        vec![utoipa::openapi::HttpMethod::Get],
                        OperationBuilder::new().build(),
                    );
                    paths
                },
                MethodRouter::new().on(
                    utoipa::openapi::HttpMethod::Get.to_method_filter(),
                    || async {},
                ),
            ))
        }

        fn routes_user(&self) -> OpenApiRouter
        where
            Self: Sized,
        {
            OpenApiRouter::new()
        }
    }

    pub struct StringSerializer;

    #[derive(Debug, Error)]
    pub struct StringSerializerError(pub String);

    impl serde::ser::Error for StringSerializerError {
        fn custom<T: Display>(msg: T) -> Self {
            Self(msg.to_string())
        }
    }

    impl Display for StringSerializerError {
        fn fmt(&self, formatter: &mut Formatter<'_>) -> core::fmt::Result {
            <Self as Debug>::fmt(self, formatter)
        }
    }

    #[expect(
        clippy::unimplemented,
        reason = "Some serializer functions are not needed and therefore aren't implemented"
    )]
    impl Serializer for StringSerializer {
        type Ok = Option<String>;
        type Error = StringSerializerError;
        type SerializeSeq = Impossible<Self::Ok, Self::Error>;
        type SerializeTuple = Impossible<Self::Ok, Self::Error>;
        type SerializeTupleStruct = Impossible<Self::Ok, Self::Error>;
        type SerializeTupleVariant = Impossible<Self::Ok, Self::Error>;
        type SerializeMap = Impossible<Self::Ok, Self::Error>;
        type SerializeStruct = Impossible<Self::Ok, Self::Error>;
        type SerializeStructVariant = Impossible<Self::Ok, Self::Error>;

        fn serialize_bool(self, value: bool) -> Result<Self::Ok, Self::Error> {
            Ok(Some(value.to_string()))
        }

        fn serialize_i8(self, value: i8) -> Result<Self::Ok, Self::Error> {
            Ok(Some(value.to_string()))
        }

        fn serialize_i16(self, value: i16) -> Result<Self::Ok, Self::Error> {
            Ok(Some(value.to_string()))
        }

        fn serialize_i32(self, value: i32) -> Result<Self::Ok, Self::Error> {
            Ok(Some(value.to_string()))
        }

        fn serialize_i64(self, value: i64) -> Result<Self::Ok, Self::Error> {
            Ok(Some(value.to_string()))
        }

        fn serialize_u8(self, value: u8) -> Result<Self::Ok, Self::Error> {
            Ok(Some(value.to_string()))
        }

        fn serialize_u16(self, value: u16) -> Result<Self::Ok, Self::Error> {
            Ok(Some(value.to_string()))
        }

        fn serialize_u32(self, value: u32) -> Result<Self::Ok, Self::Error> {
            Ok(Some(value.to_string()))
        }

        fn serialize_u64(self, value: u64) -> Result<Self::Ok, Self::Error> {
            Ok(Some(value.to_string()))
        }

        fn serialize_f32(self, value: f32) -> Result<Self::Ok, Self::Error> {
            Ok(Some(value.to_string()))
        }

        fn serialize_f64(self, value: f64) -> Result<Self::Ok, Self::Error> {
            Ok(Some(value.to_string()))
        }

        fn serialize_char(self, value: char) -> Result<Self::Ok, Self::Error> {
            Ok(Some(value.to_string()))
        }

        fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
            Ok(Some(value.to_owned()))
        }

        fn serialize_bytes(self, value: &[u8]) -> Result<Self::Ok, Self::Error> {
            Ok(Some(String::from_utf8_lossy(value).to_string()))
        }

        fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
            Ok(None)
        }

        fn serialize_some<T>(self, value: &T) -> Result<Self::Ok, Self::Error>
        where
            T: ?Sized + Serialize,
        {
            value.serialize(self)
        }

        fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
            Ok(None)
        }

        fn serialize_unit_struct(self, _name: &'static str) -> Result<Self::Ok, Self::Error> {
            unimplemented!()
        }

        fn serialize_unit_variant(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
        ) -> Result<Self::Ok, Self::Error> {
            unimplemented!()
        }

        fn serialize_newtype_struct<T>(
            self,
            _name: &'static str,
            _value: &T,
        ) -> Result<Self::Ok, Self::Error>
        where
            T: ?Sized + Serialize,
        {
            unimplemented!()
        }

        fn serialize_newtype_variant<T>(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
            _value: &T,
        ) -> Result<Self::Ok, Self::Error>
        where
            T: ?Sized + Serialize,
        {
            unimplemented!()
        }

        fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
            unimplemented!()
        }

        fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple, Self::Error> {
            unimplemented!()
        }

        fn serialize_tuple_struct(
            self,
            _name: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeTupleStruct, Self::Error> {
            unimplemented!()
        }

        fn serialize_tuple_variant(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeTupleVariant, Self::Error> {
            unimplemented!()
        }

        fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
            unimplemented!()
        }

        fn serialize_struct(
            self,
            _name: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeStruct, Self::Error> {
            unimplemented!()
        }

        fn serialize_struct_variant(
            self,
            _name: &'static str,
            _variant_index: u32,
            _variant: &'static str,
            _len: usize,
        ) -> Result<Self::SerializeStructVariant, Self::Error> {
            unimplemented!()
        }
    }

    #[must_use]
    pub fn error_schema(id: &str, msg: &str) -> Schema {
        let mut obj = Object::with_type(Type::Object);
        let mut error = Object::with_type(Type::Object);

        let mut id_obj = Object::with_type(Type::String);
        id_obj.example = Some(serde_json::Value::String(id.to_owned()));
        error
            .properties
            .insert("id".to_owned(), RefOr::T(Schema::Object(id_obj)));

        let mut msg_obj = Object::with_type(Type::String);
        msg_obj.example = Some(serde_json::Value::String(msg.to_owned()));
        error
            .properties
            .insert("msg".to_owned(), RefOr::T(Schema::Object(msg_obj)));

        obj.properties
            .insert("error".to_owned(), RefOr::T(Schema::Object(error)));
        Schema::Object(obj)
    }

    #[expect(
        private_bounds,
        reason = "Trait only implemented for TypedHeader, it should be sealed"
    )]
    #[must_use]
    pub fn header_name<T: TypedHeaderExt<E>, E: Header>() -> &'static HeaderName {
        E::name()
    }

    #[expect(
        private_bounds,
        reason = "Trait only implemented for TypedHeader, it should be sealed"
    )]
    pub fn header_insert<T: TypedHeaderExt<E>, E: Header>(value: T, map: &mut HeaderMap) {
        map.typed_insert(value.inner());
    }

    pub async fn error_by_response<T: FromStr<Err = ()> + From<reqwest::Error>>(
        response: reqwest::Response,
    ) -> T {
        struct FromStrData<T: FromStr<Err = ()>>(T);
        impl<'de, T: FromStr<Err = ()>> Deserialize<'de> for FromStrData<T> {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let id = String::deserialize(deserializer)?;
                T::from_str(&id)
                    .map(FromStrData)
                    .map_err(|()| D::Error::custom("Error not found!"))
            }
        }

        #[derive(Deserialize)]
        struct ErrorContent<T> {
            id: T,
        }
        #[derive(Deserialize)]
        struct ErrorStruct<T> {
            error: ErrorContent<T>,
        }

        match response.json().await {
            Ok(ErrorStruct {
                error: ErrorContent {
                    id: FromStrData(err),
                },
            }) => err,
            Err(err) => T::from(err),
        }
    }

    #[must_use]
    pub fn from_raw<T: From<reqwest::Response>>(response: reqwest::Response) -> T {
        T::from(response)
    }
}

pub const SUCCESS_DESCRIPTION: &str = "Success";

pub trait FrameworkDocumentedResponse {
    fn error_values() -> &'static [&'static str];

    fn openapi(
        responses: &mut utoipa::openapi::ResponsesBuilder,
        schemas: &mut HashMap<String, RefOr<Schema>>,
    );
}

pub trait FrameworkClientImpl {
    fn base_url(&self) -> &Url;
    fn client(&self) -> &reqwest::Client;
}

pub trait MaybeUnit {
    fn unit() -> Option<Self>
    where
        Self: Sized;
}

impl<T: 'static> MaybeUnit for T {
    fn unit() -> Option<Self>
    where
        Self: Sized,
    {
        // SAFETY: It is certain that the current type is (), and therefore doesn't have any size. There is no undefined behaviour.
        (TypeId::of::<()>() == TypeId::of::<T>())
            .then(|| unsafe { MaybeUninit::zeroed().assume_init() })
    }
}

pub struct WebsocketResponse(pub axum::response::Response);

impl IntoResponse for WebsocketResponse {
    fn into_response(self) -> axum::response::Response {
        self.0.into_response()
    }
}

impl<E: FrameworkDocumentedResponse + 'static> FrameworkDocumentedResponse
    for Result<WebsocketResponse, E>
{
    fn error_values() -> &'static [&'static str] {
        E::error_values()
    }

    fn openapi(responses: &mut ResponsesBuilder, schemas: &mut HashMap<String, RefOr<Schema>>) {
        <Result<(), E>>::openapi(responses, schemas);
    }
}

impl<V: ToSchema + MaybeUnit + 'static, E: FrameworkDocumentedResponse + 'static>
    FrameworkDocumentedResponse for Result<V, E>
{
    fn error_values() -> &'static [&'static str] {
        E::error_values()
    }

    fn openapi(responses: &mut ResponsesBuilder, schemas: &mut HashMap<String, RefOr<Schema>>) {
        let mut responses_owned = ResponsesBuilder::new();
        mem::swap(responses, &mut responses_owned);
        if V::unit().is_some() {
            responses_owned = responses_owned.response(
                "204",
                Response::builder().description(SUCCESS_DESCRIPTION).build(),
            );
        } else {
            responses_owned = responses_owned.response(
                "200",
                Response::builder()
                    .description(SUCCESS_DESCRIPTION)
                    .content(
                        "application/json",
                        Content::new(Some(
                            RefBuilder::new().ref_location_from_schema_name(V::name()),
                        )),
                    )
                    .build(),
            );
            let mut vals = Vec::new();
            vals.push((V::name().to_string(), V::schema()));
            V::schemas(&mut vals);
            schemas.extend(vals);
        }
        *responses = responses_owned;

        E::openapi(responses, schemas);
    }
}

pub struct FileResponse(pub Option<HeaderMap>, pub DataStream); // TODO: Add content type as parameter, when https://github.com/rust-lang/rust/issues/95174 is stable

impl From<reqwest::Response> for FileResponse {
    fn from(value: reqwest::Response) -> Self {
        Self(
            Some(value.headers().clone()),
            DataStream::by_stream(value.bytes_stream().map_err(io::Error::other)),
        )
    }
}

impl IntoResponse for FileResponse {
    fn into_response(self) -> axum::response::Response {
        (self.0, self.1.into_axum()).into_response()
    }
}

impl<E: FrameworkDocumentedResponse + 'static> FrameworkDocumentedResponse
    for Result<FileResponse, E>
{
    fn error_values() -> &'static [&'static str] {
        E::error_values()
    }

    fn openapi(responses: &mut ResponsesBuilder, schemas: &mut HashMap<String, RefOr<Schema>>) {
        let mut responses_owned = ResponsesBuilder::new();
        mem::swap(responses, &mut responses_owned);
        responses_owned = responses_owned.response(
            "200",
            Response::builder()
                .description(SUCCESS_DESCRIPTION)
                .content("application/octet-stream", Content::builder().build())
                .build(),
        );
        *responses = responses_owned;

        E::openapi(responses, schemas);
    }
}

#[derive(Debug, Clone)]
pub struct JsonVec<T: Clone>(pub Vec<T>);

#[cfg(feature = "ts")]
impl<T: Clone + ts_rs::TS> ts_rs::TS for JsonVec<T> {
    type WithoutGenerics = <Vec<T> as ts_rs::TS>::WithoutGenerics;
    type OptionInnerType = Self;

    fn ident() -> String {
        <Vec<T>>::ident()
    }

    fn decl() -> String {
        <Vec<T>>::decl()
    }

    fn decl_concrete() -> String {
        <Vec<T>>::decl_concrete()
    }

    fn name() -> String {
        <Vec<T>>::name()
    }

    fn inline() -> String {
        <Vec<T>>::inline()
    }

    fn inline_flattened() -> String {
        <Vec<T>>::inline_flattened()
    }

    fn docs() -> Option<String> {
        <Vec<T>>::docs()
    }
}

impl<T: Clone> Deref for JsonVec<T> {
    type Target = Vec<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Clone> DerefMut for JsonVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T: Serialize + Clone> Serialize for JsonVec<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serde_json::to_string(&self.0)
            .map_err(S::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de, T: for<'a> Deserialize<'a> + Clone> Deserialize<'de> for JsonVec<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        serde_json::from_str(<&str>::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

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

#[cfg(feature = "ts")]
impl ts_rs::TS for DataStream {
    type WithoutGenerics = Self;
    type OptionInnerType = Self;

    fn decl() -> String {
        "type DataStream = string | Buffer | Uint8Array | Readable;".to_owned()
    }

    fn decl_concrete() -> String {
        Self::decl()
    }

    fn name() -> String {
        "string | Buffer | Uint8Array | Readable".to_owned()
    }

    fn inline() -> String {
        Self::name()
    }

    fn inline_flattened() -> String {
        Self::name()
    }

    fn docs() -> Option<String> {
        Some("/// Universal type for data streams over the network".to_owned())
    }
}

pub trait MultiRouterExtension {
    #[must_use]
    fn merge(self, other: Self) -> Self;
}

impl MultiRouterExtension for (OpenApiRouter, OpenApiRouter) {
    fn merge(self, other: Self) -> Self {
        (self.0.merge(other.0), self.1.merge(other.1))
    }
}

#[macro_export]
macro_rules! define_path_variable {
    ($name:ident $description:literal) => {
        $crate::paste::paste! {
            pub mod [< variable_path_ $name:snake >] {

                pub const DESCRIPTION: &str = $description;

            }
        }
    };
}

#[macro_export]
macro_rules! define_query_variable {
    ($name:ident $description:literal) => {
        $crate::paste::paste! {
            pub mod [< variable_query_ $name:snake >] {

                pub const DESCRIPTION: &str = $description;

            }
        }
    };
}

#[macro_export]
macro_rules! define_header_variable {
    ($name:ident $description:literal) => {
        $crate::paste::paste! {
            pub mod [< variable_header_ $name:snake >] {

                pub const DESCRIPTION: &str = $description;

            }
        }
    };
}

#[macro_export]
macro_rules! framework_router {
    ($base:tt $var:ident { $($tr:path),* $( { $($feat:literal $tr2:path),* } )* }) => {
        {
            use $crate::tap::Pipe;
            ($crate::utoipa_axum::router::OpenApiRouter::new()
                $(
                    .merge(<$base as $tr>::routes(&$var))
                )*
                $(
                    $(
                        .pipe(|router| {
                            #[cfg(feature = $feat)]
                            {
                                router.merge(<$base as $tr2>::routes(&$var))
                            }
                            #[cfg(not(feature = $feat))]
                            {
                                router
                            }
                        })
                    )*
                )*,
                $crate::utoipa_axum::router::OpenApiRouter::new()
                $(
                    .merge(<$base as $tr>::routes_user(&$var))
                )*
                $(
                    $(
                        .pipe(|router| {
                            #[cfg(feature = $feat)]
                            {
                                router.merge(<$base as $tr2>::routes_user(&$var))
                            }
                            #[cfg(not(feature = $feat))]
                            {
                                router
                            }
                        })

                    )*
                )*
            )
        }
    };
}

#[cfg(feature = "ts")]
#[macro_export]
macro_rules! framework_ts_type_export {
    ($base:ident $visitor:ident { $($tr:path),* }) => {
        {

            $(
                <$base as $tr>::visit_types(&mut $visitor);
            )*
        }
    };
}

#[cfg(feature = "ts")]
#[macro_export]
macro_rules! framework_ts_func_export {
    ($base:ident $visitor:ident { $($tr:path),* }) => {
        {

            $(
                <$base as $tr>::visit_funcs(&mut $visitor);
            )*
        }
    };
}
