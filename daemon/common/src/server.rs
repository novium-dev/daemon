use axum::extract::ws::Message;
use bitflags::bitflags;
#[cfg(feature = "docker")]
use bollard::container::Stats;
use daemon_framework::ts;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"label": "Running", "healthcheck": null, "start": false, "stop": true, "kill": false, "time": 0}))]
#[ts]
pub struct PublicServerState {
    pub label: String,
    pub healthcheck: Option<String>,
    pub start: bool,
    pub stop: bool,
    pub kill: bool,
    pub time: Option<i64>,
}

#[derive(Copy, Clone, Serialize, Deserialize)]
pub struct ServerStats {
    pub cpu: f32,
    pub ram: f32,
    pub swap: f32,
    pub disk: i64,
    pub cpu_max: f32,
    pub ram_max: i64,
    pub swap_max: i64,
    pub disk_max: i64,
}

impl Default for ServerStats {
    fn default() -> Self {
        Self {
            cpu: -1f32,
            ram: -1f32,
            swap: -1f32,
            disk: -1,
            cpu_max: -1f32,
            ram_max: -1,
            swap_max: -1,
            disk_max: -1,
        }
    }
}

impl ServerStats {
    pub fn with_max(self, cpu: f32, ram: i64, swap: i64, disk: i64) -> Self {
        Self {
            cpu_max: cpu,
            ram_max: ram,
            swap_max: swap,
            disk_max: disk,
            ..self
        }
    }

    pub fn with_disk(self, disk: i64) -> Self {
        Self { disk, ..self }
    }
}

#[cfg(feature = "docker")]
impl From<Stats> for ServerStats {
    fn from(value: Stats) -> Self {
        let mut cpu = -1f32;
        let mut ram = -1f32;
        let swap = -1f32;

        let system_delta = value.cpu_stats.system_cpu_usage.unwrap_or_default() as f64
            - value.precpu_stats.system_cpu_usage.unwrap_or_default() as f64;
        let cpu_delta = value.cpu_stats.cpu_usage.total_usage as f64
            - value.precpu_stats.cpu_usage.total_usage as f64;

        if value.cpu_stats.online_cpus.is_some() && system_delta > 0f64 && cpu_delta > 0f64 {
            cpu = (((cpu_delta / system_delta)
                * value.cpu_stats.online_cpus.unwrap_or_default() as f64
                * 10000f64)
                .round()
                / 100f64) as f32;
        }

        if let Some(usage) = value.memory_stats.usage {
            let mut usage = usage as f64;
            usage /= 1024f64;
            usage /= 1024f64;
            usage *= 100f64;
            usage = usage.round();
            usage /= 100f64;
            if usage <= f32::MAX as f64 {
                ram = usage as f32;
            }
        }

        Self {
            cpu,
            ram,
            swap,
            ..Self::default()
        }
    }
}

impl ServerStats {
    pub fn as_websocket_message(&self) -> Message {
        const STATS_MAGIC: &str = "STATS!";
        Message::Text(
            format!(
                "{}{}",
                STATS_MAGIC.to_owned(),
                serde_json::to_string(self).unwrap()
            )
            .into(),
        )
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct BackendAuthServerPermissions: i32 {
        const TERMINAL_READ = 1 << 0;
        const TERMINAL_WRITE = 1 << 1;

        const FILES_READ = 1 << 2;
        const FILES_WRITE = 1 << 3;

        const CONTROL_VIEW = 1 << 6;
        const CONTROL_POWER = 1 << 7;

        const BACKUP_DOWNLOAD = 1 << 8;
    }
}
