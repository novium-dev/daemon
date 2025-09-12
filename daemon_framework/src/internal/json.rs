use crate::utils::MaybeUnit;
use axum::Json;
use axum::response::{IntoResponse, Response};
use http::StatusCode;
use serde::Serialize;
use serde::de::DeserializeOwned;

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
