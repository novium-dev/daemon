use crate::DaemonState;
use crate::network::{Network, NetworkError};
use crate::web::sql_error_to_http;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use daemon_common::payload::IdResponse;
use daemon_common::payload::network::NetworkPayload;
use sqlx::Connection;
use std::ops::Deref;
use tracing::error;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) fn config() -> OpenApiRouter<DaemonState> {
    OpenApiRouter::new()
        .routes(routes!(create_network))
        .routes(routes!(delete_network))
}

#[utoipa::path(put,
    path = "/network",
    tag = "network",
    description = "Creates a new network",
    responses(
        (status = 200, description = "Network was deleted successfully", body = IdResponse, content_type = "application/json"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid"),
        (status = 400, description = "Invalid network")
    ),
    params(
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
async fn create_network(
    State(state): State<DaemonState>,
    Json(NetworkPayload { ipv4, ipv6 }): Json<NetworkPayload>,
) -> Result<Json<IdResponse>, StatusCode> {
    let mut conn = state.conn.acquire().await.map_err(sql_error_to_http)?;
    let network = conn
        .transaction(move |tx| {
            Box::pin(async move { Network::create(&state.docker, tx, ipv4, ipv6).await })
        })
        .await
        .inspect_err(|err| error!("Failed to create network: {:?}", err))
        .map_err(NetworkError::into_http)?;
    Ok(Json(IdResponse { id: network.id }))
}

#[utoipa::path(delete,
    path = "/network/{network_id}",
    tag = "network",
    description = "Deletes an existing network",
    responses(
        (status = 200, description = "Network was deleted successfully"),
        (status = 404, description = "Network does not exist"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("network_id" = u32, description = "The id of the network"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
async fn delete_network(
    State(state): State<DaemonState>,
    Path((network_id,)): Path<(u32,)>,
) -> Result<(), StatusCode> {
    let Some(network) = Network::resolve(state.conn.deref(), network_id)
        .await
        .inspect_err(|err| error!("Failed to resolve network: {:?}", err))
        .map_err(NetworkError::into_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    let mut conn = state.conn.acquire().await.map_err(sql_error_to_http)?;
    conn.transaction(move |tx| {
        Box::pin(async move {
            network.delete(&state.docker, tx).await?;
            Ok(())
        })
    })
    .await
    .inspect_err(|err| error!("Failed to delete network: {:?}", err))
    .map_err(NetworkError::into_http)?;

    Ok(())
}
