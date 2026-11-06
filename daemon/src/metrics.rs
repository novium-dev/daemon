// SPDX-License-Identifier: AGPL-3.0-only
//! Prometheus exporter for server, node and group resource usage.
use crate::config::MetricsConfig;
use crate::storage::StorageIdentifier;
use crate::utils::from_mib;
use crate::{Daemon, DaemonExtra};
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse as _, Response};
use axum::routing::get;
use axum_server::Handle;
use core::fmt::Write as _;
use core::net::SocketAddr;
use core::sync::atomic::Ordering;
use daemon_common::service::resource::{ResourceService as _, ResourceSubset, StorageUsageEntry};
use sqlx::query;
use std::collections::HashMap;
use tokio::task::JoinSet;
use tracing::{error, info};

const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

impl<T: DaemonExtra> Daemon<T> {
    /// Spawns a plain HTTP listener per configured address serving `/metrics`.
    pub(crate) fn serve_metrics(
        &self,
        config: &MetricsConfig,
        handle: &Handle<SocketAddr>,
        tasks: &mut JoinSet<()>,
    ) {
        let router = Router::new()
            .route("/metrics", get(metrics_handler::<T>))
            .with_state((self.clone(), config.token.clone()));

        for listener in config.listeners.iter().copied() {
            let handle = handle.clone();
            let router = router.clone();
            info!("Serving prometheus metrics on http://{listener}/metrics");
            tasks.spawn(async move {
                let _ = axum_server::bind(listener)
                    .handle(handle)
                    .serve(router.into_make_service())
                    .await
                    .inspect_err(|err| error!("Failed to serve metrics server: {err:?}"));
            });
        }
    }

    async fn render_metrics(&self) -> Result<String, anyhow::Error> {
        let mut out = String::with_capacity(16 * 1024);

        let servers = self.collect_servers().await?;
        write_servers(&mut out, &servers);

        let resources = self
            .resource_info()
            .await
            .map_err(|err| anyhow::anyhow!("resource info failed: {err:?}"))?;

        write_family(
            &mut out,
            "novium_node_cpu_count",
            "Number of CPU cores on the node.",
            [(String::new(), f64::from(resources.cpu_count))],
        );
        write_family(
            &mut out,
            "novium_node_memory_total_bytes",
            "Total memory of the node.",
            [(String::new(), bytes(resources.mem_total))],
        );
        write_family(
            &mut out,
            "novium_node_swap_total_bytes",
            "Total swap of the node.",
            [(String::new(), bytes(resources.swap_total))],
        );

        // `system` is the whole node, `servers` is the slice all servers run in.
        let node = [
            (labels(&[("scope", "system")]), &resources.system),
            (labels(&[("scope", "servers")]), &resources.server),
        ];
        write_subsets(&mut out, "novium_node", "node", &node);

        let mut groups: Vec<_> = resources.groups.iter().collect();
        groups.sort_unstable_by(|left, right| left.0.cmp(right.0));
        let groups: Vec<_> = groups
            .into_iter()
            .map(|(name, subset)| (labels(&[("group", name)]), subset))
            .collect();
        write_subsets(&mut out, "novium_group", "group", &groups);

        let mut storages: Vec<(String, &StorageUsageEntry)> = Vec::new();
        for (kind, entries) in [
            ("server", &resources.storage.server),
            ("backup", &resources.storage.backup),
        ] {
            let mut entries: Vec<_> = entries.iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            storages.extend(entries.into_iter().map(|(id, entry)| {
                (
                    labels(&[("type", kind), ("storage", id), ("path", &entry.path)]),
                    entry,
                )
            }));
        }
        write_family(
            &mut out,
            "novium_node_storage_total_bytes",
            "Total size of the filesystem backing a storage.",
            storages
                .iter()
                .map(|(labels, entry)| (labels.clone(), bytes(entry.total))),
        );
        write_family(
            &mut out,
            "novium_node_storage_free_bytes",
            "Free space of the filesystem backing a storage.",
            storages
                .iter()
                .map(|(labels, entry)| (labels.clone(), bytes(entry.free))),
        );
        write_family(
            &mut out,
            "novium_node_storage_available_bytes",
            "Space available to unprivileged users on the filesystem backing a storage.",
            storages
                .iter()
                .map(|(labels, entry)| (labels.clone(), bytes(entry.available))),
        );

        Ok(out)
    }

    async fn collect_servers(&self) -> Result<Vec<ServerSample>, sqlx::Error> {
        let rows = query!("SELECT `id`, `cpu`, `ram`, `swap`, `disk`, `group` FROM `servers`")
            .fetch_all(&*self.conn)
            .await
            .inspect_err(|err| error!("Failed to fetch servers for metrics: {err:?}"))?;

        let stats: HashMap<u32, _> = self
            .servers
            .lock()
            .await
            .iter()
            .map(|(id, data)| (*id, **data.last_stats.load()))
            .collect();

        let disks: HashMap<u32, (i64, i64)> = self
            .storages
            .lock()
            .await
            .iter()
            .map(|(id, data)| match id {
                StorageIdentifier::Primary(server) => (
                    *server,
                    (
                        data.value.0.load(Ordering::Relaxed),
                        data.value.1.load(Ordering::Relaxed),
                    ),
                ),
            })
            .collect();

        let mut servers: Vec<_> = rows
            .into_iter()
            .map(|row| {
                let stats = stats.get(&row.id).copied().unwrap_or_default();
                let (disk_used, disk_limit) = disks.get(&row.id).copied().unwrap_or((-1, -1));
                ServerSample {
                    labels: labels(&[
                        ("server", &row.id.to_string()),
                        ("group", row.group.as_deref().unwrap_or_default()),
                    ]),
                    cpu: stats.cpu.unwrap_or_default(),
                    cpu_limit: row.cpu.map(f64::from),
                    ram: bytes(stats.ram.unwrap_or_default()),
                    ram_limit: row.ram.map(bytes),
                    swap: bytes(stats.swap.unwrap_or_default()),
                    swap_limit: row.swap.map(bytes),
                    disk: (disk_used >= 0).then(|| unsigned(disk_used.unsigned_abs())),
                    disk_limit: (disk_limit >= 0)
                        .then(|| bytes(disk_limit.unsigned_abs()))
                        .or_else(|| row.disk.map(bytes)),
                }
            })
            .collect();
        servers.sort_unstable_by(|left, right| left.labels.cmp(&right.labels));
        Ok(servers)
    }
}

struct ServerSample {
    labels: String,
    cpu: f64,
    cpu_limit: Option<f64>,
    ram: f64,
    ram_limit: Option<f64>,
    swap: f64,
    swap_limit: Option<f64>,
    disk: Option<f64>,
    disk_limit: Option<f64>,
}

type ServerField = fn(&ServerSample) -> Option<f64>;

const SERVER_METRICS: [(&str, &str, ServerField); 8] = [
    (
        "novium_server_cpu_usage_cores",
        "CPU usage of the server in cores.",
        |sample| Some(sample.cpu),
    ),
    (
        "novium_server_cpu_limit_cores",
        "CPU limit of the server in cores.",
        |sample| sample.cpu_limit,
    ),
    (
        "novium_server_memory_used_bytes",
        "Memory used by the server.",
        |sample| Some(sample.ram),
    ),
    (
        "novium_server_memory_limit_bytes",
        "Memory limit of the server.",
        |sample| sample.ram_limit,
    ),
    (
        "novium_server_swap_used_bytes",
        "Swap used by the server.",
        |sample| Some(sample.swap),
    ),
    (
        "novium_server_swap_limit_bytes",
        "Swap limit of the server.",
        |sample| sample.swap_limit,
    ),
    (
        "novium_server_disk_used_bytes",
        "Disk space used by the server.",
        |sample| sample.disk,
    ),
    (
        "novium_server_disk_limit_bytes",
        "Disk limit of the server.",
        |sample| sample.disk_limit,
    ),
];

fn write_servers(out: &mut String, servers: &[ServerSample]) {
    for (name, help, field) in SERVER_METRICS {
        write_family(
            out,
            name,
            help,
            servers
                .iter()
                .filter_map(|sample| Some((sample.labels.clone(), field(sample)?))),
        );
    }
}

type SubsetField = fn(&ResourceSubset) -> Option<f64>;

const SUBSET_METRICS: [(&str, &str, SubsetField); 12] = [
    (
        "cpu_usage_cores",
        "CPU usage of the {} in cores.",
        |subset| Some(subset.cpu_used),
    ),
    (
        "cpu_allocated_cores",
        "CPU allocated to servers of the {} in cores.",
        |subset| Some(subset.cpu_allocated),
    ),
    (
        "cpu_limit_cores",
        "CPU limit of the {} in cores.",
        |subset| subset.cpu_limit,
    ),
    (
        "cpu_limit_effective_cores",
        "Effective CPU limit of the {} in cores, including parent limits.",
        |subset| subset.cpu_limit_effective,
    ),
    ("memory_used_bytes", "Memory used by the {}.", |subset| {
        Some(bytes(subset.mem_used))
    }),
    (
        "memory_allocated_bytes",
        "Memory allocated to servers of the {}.",
        |subset| Some(bytes(subset.mem_allocated)),
    ),
    ("memory_limit_bytes", "Memory limit of the {}.", |subset| {
        subset.mem_limit.map(bytes)
    }),
    (
        "memory_limit_effective_bytes",
        "Effective memory limit of the {}, including parent limits.",
        |subset| subset.mem_limit_effective.map(bytes),
    ),
    ("swap_used_bytes", "Swap used by the {}.", |subset| {
        Some(bytes(subset.swap_used))
    }),
    (
        "swap_allocated_bytes",
        "Swap allocated to servers of the {}.",
        |subset| Some(bytes(subset.swap_allocated)),
    ),
    ("swap_limit_bytes", "Swap limit of the {}.", |subset| {
        subset.swap_limit.map(bytes)
    }),
    (
        "swap_limit_effective_bytes",
        "Effective swap limit of the {}, including parent limits.",
        |subset| subset.swap_limit_effective.map(bytes),
    ),
];

fn write_subsets(
    out: &mut String,
    prefix: &str,
    subject: &str,
    subsets: &[(String, &ResourceSubset)],
) {
    for (suffix, help, field) in SUBSET_METRICS {
        write_family(
            out,
            &format!("{prefix}_{suffix}"),
            &help.replace("{}", subject),
            subsets
                .iter()
                .filter_map(|(labels, subset)| Some((labels.clone(), field(subset)?))),
        );
    }
}

/// Writes one gauge family. Families without samples are omitted.
fn write_family(
    out: &mut String,
    name: &str,
    help: &str,
    samples: impl IntoIterator<Item = (String, f64)>,
) {
    let mut samples = samples.into_iter().peekable();
    if samples.peek().is_none() {
        return;
    }
    let _ = writeln!(out, "# HELP {name} {help}");
    let _ = writeln!(out, "# TYPE {name} gauge");
    for (labels, value) in samples {
        let _ = writeln!(out, "{name}{labels} {value}");
    }
}

fn labels(pairs: &[(&str, &str)]) -> String {
    let mut out = String::from("{");
    for (index, (key, value)) in pairs.iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        out.push_str(key);
        out.push_str("=\"");
        for char in value.chars() {
            match char {
                '\\' => out.push_str("\\\\"),
                '"' => out.push_str("\\\""),
                '\n' => out.push_str("\\n"),
                other => out.push(other),
            }
        }
        out.push('"');
    }
    out.push('}');
    out
}

/// Converts MiB to bytes.
fn bytes(mib: impl Into<u64>) -> f64 {
    unsigned(from_mib(mib))
}

#[expect(clippy::cast_precision_loss, reason = "metrics don't have to be exact")]
const fn unsigned(value: u64) -> f64 {
    value as f64
}

/// Compares in constant time to not leak the token through timing.
fn token_matches(expected: &str, given: &str) -> bool {
    expected.len() == given.len()
        && expected
            .bytes()
            .zip(given.bytes())
            .fold(0u8, |acc, (left, right)| acc | (left ^ right))
            == 0
}

async fn metrics_handler<T: DaemonExtra>(
    State((daemon, token)): State<(Daemon<T>, Option<String>)>,
    headers: HeaderMap,
) -> Response {
    if let Some(token) = &token {
        let given = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if !given.is_some_and(|given| token_matches(token, given)) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }

    match daemon.render_metrics().await {
        Ok(body) => ([(header::CONTENT_TYPE, CONTENT_TYPE)], body).into_response(),
        Err(err) => {
            error!("Failed to render metrics: {err:?}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn labels_escape() {
        assert_eq!(
            labels(&[("a", "x"), ("b", "q\"\\\n")]),
            "{a=\"x\",b=\"q\\\"\\\\\\n\"}"
        );
    }

    #[test]
    fn token() {
        assert!(token_matches("secret", "secret"));
        assert!(!token_matches("secret", "secreT"));
        assert!(!token_matches("secret", "secret2"));
    }

    #[test]
    fn family() {
        let mut out = String::new();
        write_family(&mut out, "m", "help", []);
        assert_eq!(out, "");
        write_family(&mut out, "m", "help", [(labels(&[("k", "v")]), 1.5f64)]);
        assert_eq!(out, "# HELP m help\n# TYPE m gauge\nm{k=\"v\"} 1.5\n");
    }
}
