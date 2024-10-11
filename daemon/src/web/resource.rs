use axum::http::StatusCode;
use axum::Json;
use daemon_common::payload::resource::ResourceResponse;

pub(crate) async fn get() -> Result<Json<ResourceResponse>, StatusCode> {
    Ok(Json(ResourceResponse {}))
}
