use std::ops::{Add, Deref, DerefMut};
use std::sync::Arc;
use std::time::Duration;
use bollard::Docker;
use chrono::{TimeDelta, TimeZone, Utc};
use cron::Schedule;
use sqlx::__rt::timeout;
use tokio::sync::oneshot::Receiver;
use tokio::sync::OwnedMutexGuard;
use tokio::time::Instant;
use tracing::error;
use crate::{DaemonState, ServerData};
use crate::schedules::{ScheduleRunPolicy, ScheduleStep, ScheduleStepResult};
use crate::server::FetchedServer;
use crate::server::start::ServerStartError;
use crate::server::stop::ServerStopError;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{storage_read, StorageError};
use crate::utils::server;

pub(crate) async fn task(server_id: u32, state: Arc<DaemonState>, policy: ScheduleRunPolicy, steps: Vec<ScheduleStep>, recv: Receiver<()>, schedule: Schedule, timezone: impl TimeZone + Copy + Send + 'static) {
    drop(tokio::task::spawn(async move {
        tokio::select! {
            _ = recv => {}
            _ = async move {
                loop {
                    if let Some(next) = determine_next_run(&schedule, &timezone) {
                        tokio::time::sleep_until(next).await;
                        run(server_id, state.clone(), &steps).await;
                    }
                }
            } => {},
        }
    }));
}

#[inline(always)]
fn determine_next_run(schedule: &Schedule, timezone: &(impl TimeZone + Copy)) -> Option<Instant> {
    schedule.upcoming(*timezone).next()
        .map(|time| time.to_utc() - Utc::now())
        .as_ref()
        .map(TimeDelta::to_std)
        .map(Result::ok)
        .unwrap_or_default()
        .map(|d| Instant::now() + d)
}

// Ensure this function is cancel-safe
async fn run(server_id: u32, state: Arc<DaemonState>, steps: &Vec<ScheduleStep>) {
    for step in steps {
        if let ScheduleStepResult::Interrupt = match step {
            ScheduleStep::Start => start(server_id, state.clone()).await,
            ScheduleStep::Stop => stop(server_id, state.clone()).await,
            ScheduleStep::Restart => restart(server_id, state.clone()).await,
            ScheduleStep::Kill => kill(server_id, state.clone()).await,
            ScheduleStep::Sleep(time) => {
                tokio::time::sleep(*time).await;
                ScheduleStepResult::Continue
            },
            ScheduleStep::TerminalCommand(command) => todo!(),
            ScheduleStep::ContainerCommand(command) => todo!(),
        } {
            break;
        }
    }
}

#[inline(always)]
async fn start(server_id: u32, state: Arc<DaemonState>) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&state.conn, &state.servers, server_id).await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err)) else {
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
    let Ok(storage) = storage
        .inspect_err(|err| error!("Unable to acquire storage lock: {:?}", err)) else {
        return ScheduleStepResult::Interrupt;
    };

    // Ensure the start doesn't get cancelled
    let state = state.clone();
    tokio::task::spawn(async move {
        match server.start(state.docker.clone(), server_data_locked.deref_mut(), storage, server_data, &state).await {
            Ok(_) => ScheduleStepResult::Continue,
            Err(ServerStartError::AlreadyRunning) => ScheduleStepResult::Continue,
            Err(_) => ScheduleStepResult::Interrupt,
        }
    }).await
        .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn stop(server_id: u32, state: Arc<DaemonState>) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&state.conn, &state.servers, server_id).await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err)) else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(server_data_locked) = timeout(Duration::from_millis(100), server_data.clone().lock_owned()).await else {
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
    }).await
        .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn restart(server_id: u32, state: Arc<DaemonState>) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&state.conn, &state.servers, server_id).await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err)) else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(server_data_locked) = timeout(Duration::from_millis(100), server_data.clone().lock_owned()).await else {
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
    }).await
        .unwrap_or(ScheduleStepResult::Interrupt)
}

#[inline(always)]
async fn kill(server_id: u32, state: Arc<DaemonState>) -> ScheduleStepResult {
    let Ok(Some((server, server_data))) = server(&state.conn, &state.servers, server_id).await
        .inspect_err(|err| error!("Unable to fetch server: {:?}", err)) else {
        return ScheduleStepResult::Interrupt;
    };
    let Ok(server_data_locked) = timeout(Duration::from_millis(100), server_data.clone().lock_owned()).await else {
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
    }).await
        .unwrap_or(ScheduleStepResult::Interrupt)
}
