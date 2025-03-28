use crate::DaemonState;
use crate::schedule::ScheduleStepResult;
use crate::server::FetchedServer;
use crate::server::start::ServerStartError;
use crate::server::status::InternalServerState;
use crate::server::stop::ServerStopError;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageError, storage_read};
use crate::utils::server;
use chrono::{TimeDelta, TimeZone, Utc};
use cron::Schedule;
use daemon_common::payload::schedule::{ScheduleRunPolicy, ScheduleStep};
use sqlx::__rt::timeout;
use std::collections::HashSet;
use std::ops::{Deref, DerefMut};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::sync::oneshot::Receiver;
use tokio::time::Instant;
use tracing::error;

pub(crate) async fn task(
    server_id: u32,
    state: DaemonState,
    policy: ScheduleRunPolicy,
    steps: Vec<ScheduleStep>,
    recv: Receiver<()>,
    schedule: Schedule,
    timezone: impl TimeZone + Copy + Send + 'static,
) {
    drop(tokio::task::spawn(async move {
        tokio::select! {
            _ = recv => {}
            _ = async move {
                loop {
                    if let Some(next) = determine_next_run(&schedule, &timezone) {
                        tokio::time::sleep_until(next).await;
                        run(server_id, state.clone(), policy, &steps).await;
                    }
                }
            } => {},
        }
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
        .map(|d| Instant::now() + d)
}

// Ensure this function is cancel-safe
async fn run(
    server_id: u32,
    state: DaemonState,
    policy: ScheduleRunPolicy,
    steps: &Vec<ScheduleStep>,
) {
    let running = match FetchedServer::state(server_id, HashSet::new(), &state.docker, None).await {
        Ok(
            InternalServerState::Running(_, _)
            | InternalServerState::Unhealthy(_, _)
            | InternalServerState::Starting(_, _),
        ) => true,
        Ok(InternalServerState::Stopped) => false,
        Ok(_) => unreachable!(),
        Err(err) => {
            error!(
                "Failed to check server state for schedule execution of server {}: {:?}",
                server_id, err
            );
            return;
        }
    };
    match (policy, running) {
        (ScheduleRunPolicy::Always, _) => {}
        (ScheduleRunPolicy::Running, true) => {}
        (ScheduleRunPolicy::Stopped, false) => {}
        _ => return,
    }
    for step in steps {
        if let ScheduleStepResult::Interrupt = match step {
            ScheduleStep::Start => start(server_id, state.clone()).await,
            ScheduleStep::Stop => stop(server_id, state.clone()).await,
            ScheduleStep::Restart => restart(server_id, state.clone()).await,
            ScheduleStep::Kill => kill(server_id, state.clone()).await,
            ScheduleStep::Sleep { time } => {
                tokio::time::sleep(*time).await;
                ScheduleStepResult::Continue
            }
            ScheduleStep::TerminalCommand { command } => {
                terminal(server_id, command, state.clone()).await
            }
            ScheduleStep::ContainerCommand { command } => {
                exec(server_id, command.clone(), state.clone()).await
            }
        } {
            break;
        }
    }
}

#[inline(always)]
async fn start(server_id: u32, state: DaemonState) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&state.conn, &state.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let (Ok(mut server_data_locked), storage) = tokio::join!(
        timeout(Duration::from_millis(100), server_data.clone().lock_owned()),
        storage_read(
            &state.config,
            state.conn.deref(),
            &state.storages,
            StorageIdentifier::Primary(server_id),
            true
        )
    ) else {
        return ScheduleStepResult::Continue;
    };
    let Err(StorageError::Timeout(_)) = storage else {
        return ScheduleStepResult::Continue;
    };
    let Ok(storage) =
        storage.inspect_err(|err| error!("Unable to acquire storage lock: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };

    // Ensure the start doesn't get cancelled
    let state = state.clone();
    tokio::task::spawn(async move {
        match server
            .start(
                state.docker.clone(),
                server_data_locked.deref_mut(),
                storage,
                server_data,
                &state,
            )
            .await
        {
            Ok(_) => ScheduleStepResult::Continue,
            Err(ServerStartError::AlreadyRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn stop(server_id: u32, state: DaemonState) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&state.conn, &state.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(server_data_locked) =
        timeout(Duration::from_millis(100), server_data.clone().lock_owned()).await
    else {
        return ScheduleStepResult::Continue;
    };

    // Ensure the stop doesn't get cancelled
    let state = state.clone();
    tokio::task::spawn(async move {
        match server.stop(&state.docker, server_data_locked).await {
            Ok(_) => ScheduleStepResult::Continue,
            Err(ServerStopError::NotRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn restart(server_id: u32, state: DaemonState) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&state.conn, &state.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(server_data_locked) =
        timeout(Duration::from_millis(100), server_data.clone().lock_owned()).await
    else {
        return ScheduleStepResult::Continue;
    };

    // Ensure the restart doesn't get cancelled
    let state = state.clone();
    tokio::task::spawn(async move {
        match server.restart(&state.docker, server_data_locked).await {
            Ok(_) => ScheduleStepResult::Continue,
            Err(ServerStopError::NotRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn kill(server_id: u32, state: DaemonState) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&state.conn, &state.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(server_data_locked) =
        timeout(Duration::from_millis(100), server_data.clone().lock_owned()).await
    else {
        return ScheduleStepResult::Continue;
    };

    // Ensure the kill doesn't get cancelled
    let state = state.clone();
    tokio::task::spawn(async move {
        match server.kill(&state.docker, server_data_locked).await {
            Ok(_) => ScheduleStepResult::Continue,
            Err(ServerStopError::NotRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn exec(server_id: u32, command: Vec<String>, state: DaemonState) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&state.conn, &state.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(mut server_data_locked) =
        timeout(Duration::from_millis(100), server_data.clone().lock_owned()).await
    else {
        return ScheduleStepResult::Continue;
    };

    // Ensure the kill doesn't get cancelled
    let state = state.clone();
    tokio::task::spawn(async move {
        match server
            .exec(
                &state.docker,
                command,
                server_data_locked.deref_mut(),
                false,
            )
            .await
        {
            Ok(_) => ScheduleStepResult::Continue,
            Err(ServerStopError::NotRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    })
    .await
    .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn terminal(server_id: u32, command: &str, state: DaemonState) -> ScheduleStepResult {
    let Ok(Some((_, server_data))) = server(&state.conn, &state.servers, server_id)
        .await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err))
    else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(server_data_locked) =
        timeout(Duration::from_millis(100), server_data.clone().lock_owned()).await
    else {
        return ScheduleStepResult::Continue;
    };

    match server_data_locked.attach_write.lock().await.deref_mut() {
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
