use crate::schema::*;
use crate::web::diesel_error_to_http;
use crate::DaemonState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use daemon_common::payload::resource::{ResourceDisk, ResourceResponse};
use diesel::QueryDsl;
use diesel_async::RunQueryDsl;
use std::thread;
use std::time::Duration;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) fn config() -> OpenApiRouter<DaemonState> {
    OpenApiRouter::new().routes(routes!(resource_get))
}

#[utoipa::path(get,
    path = "/resource",
    tag = "resource",
    description = "Gets resource information",
    responses(
        (status = 200, description = "Ok", body = ResourceResponse, content_type = "application/json"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn resource_get(
    State(state): State<DaemonState>,
) -> Result<Json<ResourceResponse>, StatusCode> {
    let mut conn = state.conn.lock().await;
    let mem_allocated: Option<u64> = servers::dsl::servers
        .select(diesel::dsl::sum(servers::dsl::ram))
        .get_result(&mut conn)
        .await
        .map_err(diesel_error_to_http)?;
    let swap_allocated: Option<u64> = servers::dsl::servers
        .select(diesel::dsl::sum(servers::dsl::swap))
        .get_result(&mut conn)
        .await
        .map_err(diesel_error_to_http)?;

    let resource = tokio::task::spawn_blocking(move || {
        let mut system = sysinfo::System::new_all();
        thread::sleep(Duration::from_millis(200)); // Recommended time by sysinfo
        system.refresh_cpu_all();

        let disks = sysinfo::Disks::new_with_refreshed_list();

        ResourceResponse {
            mem_total: system.total_memory(),
            mem_used: system.used_memory(),
            mem_allocated: mem_allocated.unwrap_or_default() * 1024 * 1024,
            swap_total: system.total_swap(),
            swap_used: system.used_swap(),
            swap_allocated: swap_allocated.unwrap_or_default() * 1024 * 1024,
            cpu_count: system.cpus().len() as u32,
            cpu_used: system
                .cpus()
                .iter()
                .map(|item| item.cpu_usage() / 100.0)
                .sum(),
            disks: disks
                .into_iter()
                .map(|disk| ResourceDisk {
                    total: disk.total_space(),
                    available: disk.available_space(),
                    mountpoint: disk.mount_point().to_string_lossy().to_string(),
                    readonly: disk.is_read_only(),
                    filesystem: disk.file_system().to_string_lossy().to_string(),
                })
                .collect(),
        }
    })
    .await
    .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(resource))
}
