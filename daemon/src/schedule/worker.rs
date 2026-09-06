// SPDX-License-Identifier: AGPL-3.0-only
use crate::schedule::ScheduleStepResult;
use crate::server::FetchedServer;
use crate::server::start::ServerStartError;
use crate::server::status::InternalServerState;
use crate::server::stop::ServerStopError;
use crate::storage::StorageIdentifier;
use crate::storage::backup;
use crate::storage::sync::{StorageError, storage_read_timeout};
use crate::utils::server;
use crate::{Daemon, DaemonExtra, ServerData};
use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use core::time::Duration;
use cron::Schedule;
use daemon_common::service::backup::BackupRetention;
use daemon_common::service::files::task::StorageTask;
use daemon_common::service::schedule::{ScheduleRunPolicy, ScheduleStep};
use daemon_common::service::server::task::ServerTask;
use sqlx::__rt::timeout;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::io::AsyncWriteExt as _;
use tokio::time::Instant;
use tracing::{error, info};

pub async fn task(
    server_id: u32,
    server_data: Arc<ServerData>,
    daemon: Daemon<impl DaemonExtra>,
    policy: ScheduleRunPolicy,
    steps: Vec<ScheduleStep>,
    schedule: Schedule,
    timezone: impl TimeZone + Copy + Send + 'static,
) {
    drop(tokio::task::spawn(async move {
        while let Some(next) = determine_next_run(&schedule, &timezone) {
            tokio::time::sleep_until(next).await;
            run(daemon.clone(), server_id, &server_data, policy, &steps).await;
        }
    }));
}

/// Runs a schedule's steps once, right now.
///
/// The run policy is deliberately not consulted: a trigger is somebody asking
/// for these steps explicitly, so the guard that keeps unattended runs off a
/// server in the wrong state does not apply.
pub fn trigger(
    server_id: u32,
    server_data: Arc<ServerData>,
    daemon: Daemon<impl DaemonExtra>,
    steps: Vec<ScheduleStep>,
) {
    drop(tokio::task::spawn(async move {
        run(
            daemon,
            server_id,
            &server_data,
            ScheduleRunPolicy::Always,
            &steps,
        )
        .await;
    }));
}

#[inline(always)]
fn determine_next_run(schedule: &Schedule, timezone: &(impl TimeZone + Copy)) -> Option<Instant> {
    schedule
        .upcoming(*timezone)
        .next()
        .as_ref()
        .map(DateTime::to_utc)
        .map(|time| time.signed_duration_since(Utc::now()))
        .as_ref()
        .map(TimeDelta::to_std)
        .map(Result::ok)
        .unwrap_or_default()
        .map(|duration| Instant::now().checked_add(duration))
        .unwrap_or_default()
}

// Ensure this function is cancel-safe
async fn run(
    daemon: Daemon<impl DaemonExtra>,
    server_id: u32,
    server_data: &ServerData,
    policy: ScheduleRunPolicy,
    steps: &Vec<ScheduleStep>,
) {
    let lock = server_data.lock.read(ServerTask::ScheduleTaskRun).await;
    let running = match FetchedServer::state(server_id, &HashSet::new(), &daemon.docker, server_data).await {
        Ok(
            InternalServerState::Running(_, _)
            | InternalServerState::Unhealthy(_, _)
            | InternalServerState::Starting(_, _)
            | InternalServerState::Restarting(_)
            | InternalServerState::Stopping(_)
        ) => true,
        Ok(InternalServerState::Stopped | /* Technically not possible */ InternalServerState::Locked | /* Technically not possible */ InternalServerState::Preparing) => false,
        Err(err) => {
            error!(
                "Failed to check server state for schedule execution of server {server_id}: {err:?}"
            );
            return;
        }
    };
    match (policy, running) {
        (ScheduleRunPolicy::Always, _)
        | (ScheduleRunPolicy::Running, true)
        | (ScheduleRunPolicy::Stopped, false) => {}
        _ => return,
    }
    drop(lock);
    for step in steps {
        if ScheduleStepResult::Interrupt
            == match step {
                ScheduleStep::Start => start(daemon.clone(), server_id).await,
                ScheduleStep::Stop => stop(daemon.clone(), server_id).await,
                ScheduleStep::Restart => restart(daemon.clone(), server_id).await,
                ScheduleStep::Kill => kill(daemon.clone(), server_id).await,
                ScheduleStep::Sleep { time } => {
                    tokio::time::sleep(Duration::from_secs(*time)).await;
                    ScheduleStepResult::Continue
                }
                ScheduleStep::TerminalCommand { command } => {
                    terminal(daemon.clone(), server_id, command).await
                }
                ScheduleStep::ContainerCommand { command } => {
                    exec(daemon.clone(), server_id, command.clone()).await
                }
                ScheduleStep::Backup { storage, retention } => {
                    backup(daemon.clone(), server_id, storage.clone(), *retention).await
                }
                ScheduleStep::Prune { storage, retention } => {
                    prune(daemon.clone(), server_id, storage.clone(), *retention).await;
                    ScheduleStepResult::Continue
                }
            }
        {
            break;
        }
    }
}

#[inline(always)]
async fn start(daemon: Daemon<impl DaemonExtra>, server_id: u32) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&daemon.conn, &daemon.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let (Ok(mut lock), storage) = tokio::join!(
        timeout(
            Duration::from_millis(100),
            Arc::clone(&server_data.lock).write_owned(ServerTask::Start)
        ),
        storage_read_timeout::<StorageError>(
            &daemon.config,
            &*daemon.conn,
            &daemon.storages,
            StorageIdentifier::Primary(server_id),
            StorageTask::ServerRunning,
        )
    ) else {
        return ScheduleStepResult::Continue;
    };
    if matches!(storage, Err(StorageError::Timeout)) {
        return ScheduleStepResult::Continue;
    }
    let Ok(storage) = storage.inspect_err(|err| error!("Unable to acquire storage lock: {err:?}"))
    else {
        return ScheduleStepResult::Interrupt;
    };

    // Ensure the start doesn't get cancelled
    let state = daemon.clone();
    tokio::task::spawn(async move {
        match server
            .start(
                Arc::clone(&state.docker),
                server_data,
                &mut lock,
                storage,
                &state,
            )
            .await
        {
            Ok(()) | Err(ServerStartError::AlreadyRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn stop(daemon: Daemon<impl DaemonExtra>, server_id: u32) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&daemon.conn, &daemon.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {err:?}"))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(lock) = timeout(
        Duration::from_millis(100),
        Arc::clone(&server_data.lock).read_owned(ServerTask::Stop),
    )
    .await
    else {
        return ScheduleStepResult::Continue;
    };

    // Ensure the stop doesn't get cancelled
    let state = daemon.clone();
    tokio::task::spawn(async move {
        match server.stop(&state.docker, &server_data, &lock).await {
            Ok(()) | Err(ServerStopError::NotRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn restart(daemon: Daemon<impl DaemonExtra>, server_id: u32) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&daemon.conn, &daemon.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(lock) = timeout(
        Duration::from_millis(100),
        Arc::clone(&server_data.lock).read_owned(ServerTask::Restart),
    )
    .await
    else {
        return ScheduleStepResult::Continue;
    };

    // Ensure the restart doesn't get cancelled
    let state = daemon.clone();
    tokio::task::spawn(async move {
        match server.restart(&state.docker, &server_data, &lock).await {
            Ok(()) | Err(ServerStopError::NotRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn kill(daemon: Daemon<impl DaemonExtra>, server_id: u32) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&daemon.conn, &daemon.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(lock) = timeout(
        Duration::from_millis(100),
        Arc::clone(&server_data.lock).read_owned(ServerTask::Kill),
    )
    .await
    else {
        return ScheduleStepResult::Continue;
    };

    // Ensure the kill doesn't get cancelled
    let state = daemon.clone();
    tokio::task::spawn(async move {
        match server.kill(&state.docker, &server_data, &lock).await {
            Ok(()) | Err(ServerStopError::NotRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

/// Creates a backup and, when the step carries a retention policy, applies that
/// policy afterwards.
///
/// A failed backup interrupts the schedule. A failed prune does not, since the
/// backup itself did land and the next run prunes again.
///
/// Cancel safe
#[inline(always)]
async fn backup(
    daemon: Daemon<impl DaemonExtra>,
    server_id: u32,
    storage: Option<String>,
    retention: Option<BackupRetention>,
) -> ScheduleStepResult {
    // Ensure neither the backup nor the prune that follows it gets cancelled
    // half way through when the schedule is removed or updated.
    tokio::task::spawn(async move {
        let id = StorageIdentifier::Primary(server_id);
        let created = match storage.as_ref() {
            Some(storage) => {
                backup::create_backup::<StorageError>(&daemon, id, storage, None).await
            }
            None => backup::create_backup_order::<StorageError>(&daemon, id, None).await,
        };
        match created {
            Ok(backup_id) => info!("Schedule created backup {backup_id} of server {server_id}"),
            Err(err) => {
                error!("Schedule failed to create a backup of server {server_id}: {err:?}");
                return ScheduleStepResult::Interrupt;
            }
        }

        if let Some(retention) = retention {
            prune_logged(&daemon, server_id, storage.as_ref(), retention).await;
        }

        ScheduleStepResult::Continue
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

/// Applies a retention policy to a server's backups.
///
/// Cancel safe
#[inline(always)]
async fn prune(
    daemon: Daemon<impl DaemonExtra>,
    server_id: u32,
    storage: Option<String>,
    retention: BackupRetention,
) {
    // Ensure a prune that has started deleting is not cancelled part way.
    if let Err(err) = tokio::task::spawn(async move {
        prune_logged(&daemon, server_id, storage.as_ref(), retention).await;
    })
    .await
    {
        error!("Prune task of server {server_id} panicked: {err:?}");
    }
}

/// Applies a retention policy, reporting the outcome to the log.
///
/// A prune that fails is logged and forgotten: the next run applies the same
/// policy again, so nothing is lost by letting the schedule carry on.
///
/// Not cancel safe, run it inside a task of its own.
async fn prune_logged(
    daemon: &Daemon<impl DaemonExtra>,
    server_id: u32,
    storage: Option<&String>,
    retention: BackupRetention,
) {
    match backup::prune_backups::<StorageError>(
        daemon,
        StorageIdentifier::Primary(server_id),
        storage,
        retention,
        false,
    )
    .await
    {
        Ok(entries) => {
            let removed = entries.iter().filter(|entry| !entry.keep).count();
            info!(
                "Retention policy removed {removed} of {} backups of server {server_id}",
                entries.len()
            );
        }
        Err(err) => {
            error!("Schedule failed to prune the backups of server {server_id}: {err:?}");
        }
    }
}

#[inline(always)]
async fn exec(
    daemon: Daemon<impl DaemonExtra>,
    server_id: u32,
    command: Vec<String>,
) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&daemon.conn, &daemon.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {err:?}"))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(lock) = timeout(
        Duration::from_millis(100),
        Arc::clone(&server_data.lock).read_owned(ServerTask::ContainerCommand),
    )
    .await
    else {
        return ScheduleStepResult::Continue;
    };

    // Ensure the kill doesn't get cancelled
    let state = daemon.clone();
    tokio::task::spawn(async move {
        match server
            .exec(&state.docker, command, &server_data, &lock, false)
            .await
        {
            Ok(_) | Err(ServerStopError::NotRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn terminal(
    daemon: Daemon<impl DaemonExtra>,
    server_id: u32,
    command: &str,
) -> ScheduleStepResult {
    let Ok(Some((_, server_data))) = server(&daemon.conn, &daemon.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {err:?}"))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(lock) = timeout(
        Duration::from_millis(100),
        server_data.lock.read(ServerTask::ScheduleTerminalCommand),
    )
    .await
    else {
        return ScheduleStepResult::Continue;
    };

    match &mut *server_data.attach_write.lock().await {
        None => ScheduleStepResult::Interrupt,
        Some(write) => {
            let mut content = "\x08".repeat(4096);
            content += command;
            content += "\n";
            let _ = write.write_all(content.as_bytes()).await;
            let _ = write.flush().await;
            drop(lock);
            ScheduleStepResult::Continue
        }
    }
}
