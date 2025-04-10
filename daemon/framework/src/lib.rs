pub use daemon_framework_macros::define_error;
pub use daemon_framework_macros::error;
pub use daemon_framework_macros::service;
use std::any::TypeId;
use std::collections::HashMap;
use std::mem;

pub use async_trait::async_trait;
pub use axum;
use axum::response::IntoResponse;
pub use axum_extra;
pub use http;
pub use paste;
pub use serde;
pub use serde_json;
pub use sqlx;
pub use utoipa;
pub use utoipa_axum;

use utoipa::ToSchema;
use utoipa::openapi::{Content, RefOr, Response, ResponsesBuilder, Schema};
use utoipa::openapi::schema::RefBuilder;

pub const SUCCESS_DESCRIPTION: &str = "Success";

pub trait FrameworkDocumentedResponse {
    fn openapi(responses: &mut utoipa::openapi::ResponsesBuilder, schemas: &mut HashMap<String, RefOr<Schema>>);
}

pub trait FrameworkClientImpl {
    fn client(&self) -> &reqwest::Client;
}

#[doc(hidden)]
pub mod internal {
    use axum::Json;
    use axum::response::{IntoResponse, Response};
    use axum::routing::MethodRouter;
    use axum_extra::TypedHeader;
    use axum_extra::headers::Header;
    use http::HeaderName;
    use serde::Serialize;
    use std::any::TypeId;
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

    trait TypedHeaderExt<T: Header> {}

    impl<T: Header> TypedHeaderExt<T> for TypedHeader<T> {}

    #[allow(private_bounds)]
    pub fn header_name<T: TypedHeaderExt<E>, E: Header>() -> &'static HeaderName {
        E::name()
    }

    pub struct FrameworkJsonResponse<T>(pub T);

    impl<T: Serialize + 'static> IntoResponse for FrameworkJsonResponse<T> {
        fn into_response(self) -> Response {
            if TypeId::of::<T>() == TypeId::of::<()>() {
                ().into_response()
            } else {
                Json(self.0).into_response()
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

impl<V: ToSchema + 'static, E: FrameworkDocumentedResponse + 'static>
    FrameworkDocumentedResponse for Result<V, E>
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
                    .content("application/json", Content::new(Some(RefBuilder::new().ref_location_from_schema_name(V::name()))))
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
