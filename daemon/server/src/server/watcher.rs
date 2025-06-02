use crate::server::FetchedServer;
use crate::server::start::ServerStartError;
use crate::sync::Mutex;
use crate::utils::ChannelWriteData;
use crate::{Daemon, ServerData};
use async_std::prelude::Stream;
use async_std::stream::StreamExt;
use bollard::container::LogOutput;
use bollard::errors::Error;
use bollard::models::ContainerStatsResponse;
use bollard::query_parameters::{AttachContainerOptions, InspectContainerOptions, StatsOptions};
use daemon_common::service::server::ServerStats;
use ringbuffer::AllocRingBuffer;
use std::collections::HashSet;
use std::ops::{Add, Deref, DerefMut};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;
use tokio::sync::OwnedRwLockReadGuard;
use tokio::sync::broadcast::Sender;
use tokio::time::Instant;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn attach(
    server_id: u32,
    state: &Daemon,
    cpu_max: f32,
    ram_max: i64,
    swap_max: i64,
    server_data: &mut ServerData,
    server_data_arc: Arc<Mutex<ServerData>>,
    storage_lock: OwnedRwLockReadGuard<(), ()>,
    storage_value: Arc<(AtomicI64, AtomicI64)>,
    storage_stats_lock: Arc<Mutex<()>>,
) -> Result<(), ServerStartError> {
    let result = state
        .docker
        .attach_container(
            &FetchedServer::container_name(server_id),
            Some(AttachContainerOptions {
                logs: true,
                stderr: true,
                stdout: true,
                stdin: true,
                stream: true,
                ..AttachContainerOptions::default()
            }),
        )
        .await
        .map_err(ServerStartError::Docker)?;
    *server_data.attach_write.lock().await = Some(result.input);

    let result2 = state.docker.stats(
        &FetchedServer::container_name(server_id),
        Some(StatsOptions {
            stream: true,
            one_shot: false,
        }),
    );

    let sender = server_data.sender.deref().clone();
    let stats_sender = server_data.stats_sender.deref().clone();
    let buffer = server_data.buffer.clone();
    let state = state.clone();

    tokio::task::spawn(watch(
        server_id,
        result.output,
        sender,
        stats_sender,
        buffer,
        state,
        cpu_max,
        ram_max,
        swap_max,
        result2,
        server_data_arc,
        storage_lock,
        storage_value,
        storage_stats_lock,
    ));
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn watch(
    server_id: u32,
    mut output: Pin<Box<dyn Stream<Item = Result<LogOutput, Error>> + Send>>,
    sender: Sender<ChannelWriteData>,
    stats_sender: Sender<ChannelWriteData>,
    buffer: Arc<Mutex<AllocRingBuffer<u8>>>,
    state: Daemon,
    cpu_max: f32,
    ram_max: i64,
    swap_max: i64,
    mut result2: impl Stream<Item = Result<ContainerStatsResponse, Error>>
    + Sized
    + Send
    + Unpin
    + 'static,
    server_data_arc: Arc<Mutex<ServerData>>,
    storage_lock: OwnedRwLockReadGuard<(), ()>,
    storage_value: Arc<(AtomicI64, AtomicI64)>,
    storage_stats_lock: Arc<Mutex<()>>,
) {
    let start = Instant::now();
    let mut reset_counter = false;
    let storage_stats_guard = storage_stats_lock.lock().await;
    let mut next = Instant::now().add(Duration::from_millis(500));
    loop {
        if start.elapsed() > Duration::from_secs(60) && !reset_counter {
            let mut server_data = server_data_arc.lock().await;
            server_data.failure_counter = 0;
            reset_counter = true;
        }
        tokio::select! {
            log = output.next() => {
                match log {
                    Some(Ok(log)) => {
                        let mut buffer = buffer.lock().await;
                        FetchedServer::console(&sender, buffer.deref_mut(), log.to_string())
                    },
                    Some(Err(_err)) => return,
                    None => break,
                }
            }
            stat = result2.next() => {
                if let Some(Ok(stats)) = stat {
                    let _ = stats_sender.send(ChannelWriteData::Stats(<ContainerStatsResponse as Into<ServerStats>>::into(stats).with_disk(storage_value.0.load(Ordering::Relaxed)).with_max(cpu_max, ram_max, swap_max, storage_value.1.load(Ordering::Relaxed))));
                }
            }
            _elapsed = tokio::time::sleep_until(next) => {
                next = Instant::now().add(Duration::from_millis(500));
                let stats_sender = stats_sender.clone();
                let docker = state.docker.clone();
                let server_data_arc = server_data_arc.clone();
                tokio::task::spawn(async move {
                    FetchedServer::update_status(server_id, HashSet::new(), &stats_sender, &docker, &server_data_arc).await;
                });
            }
        }
    }
    drop(storage_lock);
    let mut failure = if let Ok(inspection) = state
        .docker
        .inspect_container(
            &FetchedServer::container_name(server_id),
            None::<InspectContainerOptions>,
        )
        .await
    {
        inspection
            .state
            .unwrap_or_default()
            .exit_code
            .map(|code| code != 0)
            .unwrap_or_default()
    } else {
        false
    };
    let mut server_data = server_data_arc.lock().await;
    failure &= !server_data.stopping;
    server_data.stopping = false;

    let mut failure_exceeded = false;
    if !server_data.restart && failure && start.elapsed() <= Duration::from_secs(60) {
        server_data.failure_counter += 1;
        if server_data.failure_counter >= 3 {
            failure = false;
            failure_exceeded = true;
            server_data.failure_counter = 0;
        }
    }
    server_data.restart |= failure;

    let do_restart = server_data.restart;
    if do_restart {
        server_data.restart = false;
        tokio::task::spawn(FetchedServer::queue_start(
            server_id,
            state.clone(),
            Duration::from_secs(1),
        ));
    }
    let mut buffer = buffer.lock().await;
    FetchedServer::console(&server_data.sender, buffer.deref_mut(), "\n".to_owned());
    FetchedServer::log(&server_data.sender, buffer.deref_mut(), "Server stopped!");
    if failure_exceeded {
        FetchedServer::log(
            &server_data.sender,
            buffer.deref_mut(),
            "Server crashed 3 times in a row. Server will not restart",
        );
    }
    drop(buffer);
    let _ = stats_sender.send(ChannelWriteData::Stats(
        ServerStats::default()
            .with_disk(storage_value.0.load(Ordering::Relaxed))
            .with_max(
                cpu_max,
                ram_max,
                swap_max,
                storage_value.1.load(Ordering::Relaxed),
            ),
    ));
    drop(storage_stats_guard);
    if !do_restart {
        tokio::time::sleep(Duration::from_secs(1)).await;
        FetchedServer::update_status0(
            server_id,
            FetchedServer::resolve_locked(server_id, &state.conn).await,
            &state.docker,
            &server_data,
        )
        .await;
    }
}
