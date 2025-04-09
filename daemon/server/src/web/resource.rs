use crate::Daemon;
use crate::web::sql_error_to_http;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use daemon_common::payload::resource::{ResourceDisk, ResourceResponse};
use num_traits::cast::ToPrimitive;
use sqlx::query_as;
use sqlx::types::BigDecimal;
use std::ops::Deref;
use std::thread;
use std::time::Duration;
use tracing::debug;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

#[derive(sqlx::FromRow)]
struct ResourceSummary {
    ram: Option<BigDecimal>,
    swap: Option<BigDecimal>,
}

pub(crate) fn config() -> OpenApiRouter<Daemon> {
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
    State(state): State<Daemon>,
) -> Result<Json<ResourceResponse>, StatusCode> {
    let ResourceSummary {
        ram: mem_allocated,
        swap: swap_allocated,
    } = query_as!(
        ResourceSummary,
        "SELECT SUM(`ram`) AS `ram`, SUM(`swap`) AS `swap` FROM `servers`"
    )
    .fetch_one(state.conn.deref())
    .await
    .map_err(sql_error_to_http)?;

    let resource = tokio::task::spawn_blocking(move || {
        let mut system = sysinfo::System::new_all();
        thread::sleep(Duration::from_millis(200)); // Recommended time by sysinfo
        system.refresh_cpu_all();

        let disks = sysinfo::Disks::new_with_refreshed_list();

        ResourceResponse {
            mem_total: system.total_memory(),
            mem_used: system.used_memory(),
            mem_allocated: mem_allocated
                .unwrap_or_default()
                .to_u64()
                .unwrap_or_default()
                * 1024
                * 1024,
            swap_total: system.total_swap(),
            swap_used: system.used_swap(),
            swap_allocated: swap_allocated
                .unwrap_or_default()
                .to_u64()
                .unwrap_or_default()
                * 1024u64
                * 1024,
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
    .inspect_err(|err| debug!("Failed to provide resource info: {:?}", err))
    .map_err(|_err| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(resource))
}
