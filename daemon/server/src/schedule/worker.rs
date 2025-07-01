use crate::Daemon;
use crate::schedule::ScheduleStepResult;
use crate::server::FetchedServer;
use crate::server::start::ServerStartError;
use crate::server::status::InternalServerState;
use crate::server::stop::ServerStopError;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageError, storage_read_timeout};
use crate::utils::server;
use chrono::{TimeDelta, TimeZone, Utc};
use cron::Schedule;
use daemon_common::service::schedule::{ScheduleRunPolicy, ScheduleStep};
use sqlx::__rt::timeout;
use std::collections::HashSet;
use std::sync::Arc;
use core::time::Duration;
use tokio::io::AsyncWriteExt as _;
use tokio::time::Instant;
use tracing::error;

pub async fn task(
    server_id: u32,
    daemon: Daemon,
    policy: ScheduleRunPolicy,
    steps: Vec<ScheduleStep>,
    schedule: Schedule,
    timezone: impl TimeZone + Copy + Send + 'static,
) {
    drop(tokio::task::spawn(async move {
        loop {
            if let Some(next) = determine_next_run(&schedule, &timezone) {
                tokio::time::sleep_until(next).await;
                run(daemon.clone(), server_id, policy, &steps).await;
            }
        };
    }));
}

#[inline(always)]
fn determine_next_run(schedule: &Schedule, timezone: &(impl TimeZone + Copy)) -> Option<Instant> {
    schedule
        .upcoming(*timezone)
        .next()
        .map(|time| time.to_utc() - Utc::now())
        .as_ref()
        .map(TimeDelta::to_std)
        .map(Result::ok)
        .unwrap_or_default()
        .map(|duration| Instant::now().checked_add(duration))
        .unwrap_or_default()
}

// Ensure this function is cancel-safe
async fn run(daemon: Daemon, server_id: u32, policy: ScheduleRunPolicy, steps: &Vec<ScheduleStep>) {
    let running = match FetchedServer::state(server_id, HashSet::new(), &daemon.docker, None).await {
        Ok(
            InternalServerState::Running(_, _)
            | InternalServerState::Unhealthy(_, _)
            | InternalServerState::Starting(_, _)
            // Technically not possible
            | InternalServerState::Restarting(_)
            | InternalServerState::Stopping(_)
        ) => true,
        Ok(InternalServerState::Stopped | /* Technically not possible */ InternalServerState::Locked) => false,
        Err(err) => {
            error!(
                "Failed to check server state for schedule execution of server {server_id}: {err:?}"
            );
            return;
        }
    };
    match (policy, running) {
        (ScheduleRunPolicy::Always, _) | (ScheduleRunPolicy::Running, true) | (ScheduleRunPolicy::Stopped, false) => {}
        _ => return,
    }
    for step in steps {
        if ScheduleStepResult::Interrupt == match step {
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
        } {
            break;
        }
    }
}

#[inline(always)]
async fn start(daemon: Daemon, server_id: u32) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&daemon.conn, &daemon.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let (Ok(mut server_data_locked), storage) = tokio::join!(
        timeout(Duration::from_millis(100), Arc::clone(&server_data).lock_owned()),
        storage_read_timeout::<StorageError>(
            &daemon.config,
            &*daemon.conn,
            &daemon.storages,
            StorageIdentifier::Primary(server_id),
        )
    ) else {
        return ScheduleStepResult::Continue;
    };
    let Err(StorageError::Timeout) = storage else {
        return ScheduleStepResult::Continue;
    };
    let Ok(storage) =
        storage.inspect_err(|err| error!("Unable to acquire storage lock: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };

    // Ensure the start doesn't get cancelled
    let state = daemon.clone();
    tokio::task::spawn(async move {
        match server
            .start(
                Arc::clone(&state.docker),
                &mut server_data_locked,
                storage,
                server_data,
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
async fn stop(daemon: Daemon, server_id: u32) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&daemon.conn, &daemon.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(server_data_locked) =
        timeout(Duration::from_millis(100), Arc::clone(&server_data).lock_owned()).await
    else {
        return ScheduleStepResult::Continue;
    };

    // Ensure the stop doesn't get cancelled
    let state = daemon.clone();
    tokio::task::spawn(async move {
        match server.stop(&state.docker, server_data_locked).await {
            Ok(()) | Err(ServerStopError::NotRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn restart(daemon: Daemon, server_id: u32) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&daemon.conn, &daemon.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(server_data_locked) =
        timeout(Duration::from_millis(100), Arc::clone(&server_data).lock_owned()).await
    else {
        return ScheduleStepResult::Continue;
    };

    // Ensure the restart doesn't get cancelled
    let state = daemon.clone();
    tokio::task::spawn(async move {
        match server.restart(&state.docker, server_data_locked).await {
            Ok(()) | Err(ServerStopError::NotRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn kill(daemon: Daemon, server_id: u32) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&daemon.conn, &daemon.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(server_data_locked) =
        timeout(Duration::from_millis(100), Arc::clone(&server_data).lock_owned()).await
    else {
        return ScheduleStepResult::Continue;
    };

    // Ensure the kill doesn't get cancelled
    let state = daemon.clone();
    tokio::task::spawn(async move {
        match server.kill(&state.docker, server_data_locked).await {
            Ok(()) | Err(ServerStopError::NotRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn exec(daemon: Daemon, server_id: u32, command: Vec<String>) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&daemon.conn, &daemon.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(server_data_locked) =
        timeout(Duration::from_millis(100), Arc::clone(&server_data).lock_owned()).await
    else {
        return ScheduleStepResult::Continue;
    };

    // Ensure the kill doesn't get cancelled
    let state = daemon.clone();
    tokio::task::spawn(async move {
        match server
            .exec(
                &state.docker,
                command,
                &server_data_locked,
                false,
            )
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
async fn terminal(daemon: Daemon, server_id: u32, command: &str) -> ScheduleStepResult {
    let Ok(Some((_, server_data))) = server(&daemon.conn, &daemon.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(server_data_locked) =
        timeout(Duration::from_millis(100), Arc::clone(&server_data).lock_owned()).await
    else {
        return ScheduleStepResult::Continue;
    };

    match &mut *server_data_locked.attach_write.lock().await {
        None => ScheduleStepResult::Interrupt,
        Some(write) => {
            let mut content = "\x08".repeat(4096);
            content += command;
            content += "\n";
            let _ = write.write_all(content.as_bytes()).await;
            let _ = write.flush().await;
            ScheduleStepResult::Continue
        }
    }
}
