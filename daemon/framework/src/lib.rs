pub use async_trait::async_trait;
pub use axum;
use axum::response::IntoResponse;
pub use axum_extra;
pub use daemon_framework_macros::define_error;
pub use daemon_framework_macros::error;
pub use daemon_framework_macros::service;
pub use http;
pub use paste;
pub use reqwest;
use reqwest::Url;
pub use serde;
pub use serde_json;
pub use serde_urlencoded;
pub use sqlx;
use std::any::TypeId;
use std::collections::HashMap;
use std::mem;
use std::mem::MaybeUninit;
pub use urlencoding;
pub use utoipa;
use utoipa::ToSchema;
use utoipa::openapi::schema::RefBuilder;
use utoipa::openapi::{Content, RefOr, Response, ResponsesBuilder, Schema};
pub use utoipa_axum;

pub const SUCCESS_DESCRIPTION: &str = "Success";

pub trait FrameworkDocumentedResponse {
    fn openapi(
        responses: &mut utoipa::openapi::ResponsesBuilder,
        schemas: &mut HashMap<String, RefOr<Schema>>,
    );
}

pub trait FrameworkClientImpl {
    fn client(&self) -> &reqwest::Client;

    fn base_url(&self) -> &Url;
}

#[doc(hidden)]
pub mod internal {
    use crate::MaybeUnit;
    use axum::Json;
    use axum::response::{IntoResponse, Response};
    use axum::routing::MethodRouter;
    use axum_extra::TypedHeader;
    use axum_extra::headers::{Header, HeaderMapExt};
    use http::{HeaderMap, HeaderName};
    use serde::de::{DeserializeOwned, Error};
    use serde::ser::Impossible;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::fmt::{Debug, Display, Formatter};
    use std::str::FromStr;
    use thiserror::Error;
    use utoipa::PartialSchema;
    use utoipa::openapi::path::OperationBuilder;
    use utoipa::openapi::{Object, RefOr, Schema, Type};
    use utoipa_axum::PathItemExt;
    use utoipa_axum::router::OpenApiRouter;

    pub fn error_schema(id: &str, msg: &str) -> Schema {
        let mut obj = Object::with_type(Type::Object);
        let mut error = Object::with_type(Type::Object);

        let mut id_obj = Object::with_type(Type::String);
        id_obj.example = Some(serde_json::Value::String(id.to_string()));
        error
            .properties
            .insert("id".to_string(), RefOr::T(Schema::Object(id_obj)));

        let mut msg_obj = Object::with_type(Type::String);
        msg_obj.example = Some(serde_json::Value::String(msg.to_string()));
        error
            .properties
            .insert("msg".to_string(), RefOr::T(Schema::Object(msg_obj)));

        obj.properties
            .insert("error".to_string(), RefOr::T(Schema::Object(error)));
        Schema::Object(obj)
    }

    trait TypedHeaderExt<T: Header> {
        fn inner(self) -> T;
    }

    impl<T: Header> TypedHeaderExt<T> for TypedHeader<T> {
        fn inner(self) -> T {
            self.0
        }
    }

    #[allow(private_bounds)]
    pub fn header_name<T: TypedHeaderExt<E>, E: Header>() -> &'static HeaderName {
        E::name()
    }

    #[allow(private_bounds)]
    pub fn header_insert<T: TypedHeaderExt<E>, E: Header>(value: T, map: &mut HeaderMap) {
        map.typed_insert(value.inner())
    }

    pub struct FrameworkJsonResponse<T>(pub T);

    impl<T: Serialize + MaybeUnit + 'static> IntoResponse for FrameworkJsonResponse<T> {
        fn into_response(self) -> Response {
            if T::unit().is_some() {
                ().into_response()
            } else {
                Json(self.0).into_response()
            }
        }
    }

    impl<T: DeserializeOwned + MaybeUnit + 'static> FrameworkJsonResponse<T> {
        pub async fn from_reqwest(response: reqwest::Response) -> Result<Self, reqwest::Error> {
            if let Some(unit) = T::unit() {
                Ok(FrameworkJsonResponse(unit))
            } else {
                response.json().await.map(FrameworkJsonResponse)
            }
        }
    }

    pub trait FrameworkServerImpl {
        fn routes(&self) -> OpenApiRouter
        where
            Self: Sized,
        {
            OpenApiRouter::new().routes((
                vec![("Result".to_string(), <()>::schema())],
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
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            <Self as Debug>::fmt(self, f)
        }
    }

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

        fn serialize_bool(self, v: bool) -> Result<Self::Ok, Self::Error> {
            Ok(Some(v.to_string()))
        }

        fn serialize_i8(self, v: i8) -> Result<Self::Ok, Self::Error> {
            Ok(Some(v.to_string()))
        }

        fn serialize_i16(self, v: i16) -> Result<Self::Ok, Self::Error> {
            Ok(Some(v.to_string()))
        }

        fn serialize_i32(self, v: i32) -> Result<Self::Ok, Self::Error> {
            Ok(Some(v.to_string()))
        }

        fn serialize_i64(self, v: i64) -> Result<Self::Ok, Self::Error> {
            Ok(Some(v.to_string()))
        }

        fn serialize_u8(self, v: u8) -> Result<Self::Ok, Self::Error> {
            Ok(Some(v.to_string()))
        }

        fn serialize_u16(self, v: u16) -> Result<Self::Ok, Self::Error> {
            Ok(Some(v.to_string()))
        }

        fn serialize_u32(self, v: u32) -> Result<Self::Ok, Self::Error> {
            Ok(Some(v.to_string()))
        }

        fn serialize_u64(self, v: u64) -> Result<Self::Ok, Self::Error> {
            Ok(Some(v.to_string()))
        }

        fn serialize_f32(self, v: f32) -> Result<Self::Ok, Self::Error> {
            Ok(Some(v.to_string()))
        }

        fn serialize_f64(self, v: f64) -> Result<Self::Ok, Self::Error> {
            Ok(Some(v.to_string()))
        }

        fn serialize_char(self, v: char) -> Result<Self::Ok, Self::Error> {
            Ok(Some(v.to_string()))
        }

        fn serialize_str(self, v: &str) -> Result<Self::Ok, Self::Error> {
            Ok(Some(v.to_string()))
        }

        fn serialize_bytes(self, v: &[u8]) -> Result<Self::Ok, Self::Error> {
            Ok(Some(String::from_utf8_lossy(v).to_string()))
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
                    .map_err(|_| D::Error::custom("Error not found!"))
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
        if TypeId::of::<()>() == TypeId::of::<T>() {
            Some(unsafe { MaybeUninit::zeroed().assume_init() }) // TODO: Replace with auto traits when stable, see https://github.com/rust-lang/rust/issues/13231
        } else {
            None
        }
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
    fn openapi(responses: &mut ResponsesBuilder, schemas: &mut HashMap<String, RefOr<Schema>>) {
        <Result<(), E>>::openapi(responses, schemas)
    }
}

impl<V: ToSchema + 'static, E: FrameworkDocumentedResponse + 'static> FrameworkDocumentedResponse
    for Result<V, E>
{
    fn openapi(responses: &mut ResponsesBuilder, schemas: &mut HashMap<String, RefOr<Schema>>) {
        let mut responses_owned = ResponsesBuilder::new();
        mem::swap(responses, &mut responses_owned);
        if TypeId::of::<V>() == TypeId::of::<()>() {
            responses_owned = responses_owned.response(
                "200",
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

        E::openapi(responses, schemas)
    }
}

pub struct FileResponse(pub axum::response::Response); // TODO: Add content type as parameter, when https://github.com/rust-lang/rust/issues/95174 is stable

impl IntoResponse for FileResponse {
    fn into_response(self) -> axum::response::Response {
        self.0.into_response()
    }
}

impl<E: FrameworkDocumentedResponse + 'static> FrameworkDocumentedResponse
    for Result<FileResponse, E>
{
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

        E::openapi(responses, schemas)
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
    ($base:ident $var:ident { $($tr:ident),* }) => {
        $crate::utoipa_axum::router::OpenApiRouter::new()
            $(
                .merge(<$base as $tr>::routes(&$var))
            )*
    };
}

#[macro_export]
macro_rules! framework_router_user {
    ($base:ident $var:ident { $($tr:ident),* }) => {
        $crate::utoipa_axum::router::OpenApiRouter::new()
            $(
                .merge(<$base as $tr>::routes_user(&$var))
            )*
    };
}
