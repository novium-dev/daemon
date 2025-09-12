use core::str::FromStr;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};
use utoipa::openapi::{Object, RefOr, Schema, Type};

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
