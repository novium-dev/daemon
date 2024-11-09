#[cfg(feature = "websocket")]
use axum::extract::ws::Message;
use bitflags::bitflags;
#[cfg(feature = "docker")]
use bollard::container::Stats;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
#[schema(example = json!({"label": "Running", "healthcheck": null, "start": false, "stop": true, "kill": false}))]
pub struct PublicServerState {
    pub label: String,
    pub healthcheck: Option<String>,
    pub start: bool,
    pub stop: bool,
    pub kill: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ServerStats {
    pub cpu: f32,
    pub mem: f32,
    pub cpu_max: f32,
    pub mem_max: f32,
}

impl Default for ServerStats {
    fn default() -> Self {
        Self {
            cpu: -1f32,
            mem: -1f32,
            cpu_max: -1f32,
            mem_max: -1f32,
        }
    }
}

impl ServerStats {
    pub fn with_max(self, cpu: f32, mem: f32) -> Self {
        Self {
            cpu: self.cpu,
            mem: self.mem,
            cpu_max: cpu,
            mem_max: mem,
        }
    }
}

#[cfg(feature = "docker")]
impl From<Stats> for ServerStats {
    fn from(value: Stats) -> Self {
        let mut cpu = -1f32;
        let mut mem = -1f32;

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
                mem = usage as f32;
            }
        }
        Self {
            cpu,
            mem,
            ..Self::default()
        }
    }
}

#[allow(unused)]
pub const STATS_MAGIC: &str = "STATS!";

#[cfg(feature = "websocket")]
impl ServerStats {
    pub fn as_websocket_message(&self) -> Message {
        Message::Text(
            format!(
                "{}{}",
                STATS_MAGIC.to_owned(),
                serde_json::to_string(self).unwrap()
            )
            .to_owned(),
        )
    }
}

bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
    pub struct BackendAuthServerPermissions: i32 {
        const TERMINAL_READ = 1 << 0;
        const TERMINAL_WRITE = 1 << 1;

        const FILES_READ = 1 << 2;
        const FILES_WRITE = 1 << 3;
        const FILES_DELETE = 1 << 4;
        const FILES_CREATE = 1 << 5;
    }
}
