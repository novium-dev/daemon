use crate::server::start::ServerStartError;
use crate::server::FetchedServer;
use crate::utils::ChannelWriteData;
use crate::{DaemonState, ServerData};
use async_std::prelude::Stream;
use async_std::stream::StreamExt;
use bollard::container::{AttachContainerOptions, LogOutput, Stats, StatsOptions};
use bollard::errors::Error;
use daemon_common::server::ServerStats;
use ringbuffer::AllocRingBuffer;
use std::ops::{Add, Deref, DerefMut};
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::Sender;
use tokio::sync::{Mutex, OwnedRwLockReadGuard};
use tokio::time::Instant;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn attach(
    server_id: u32,
    state: &DaemonState,
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
        .attach_container::<String>(
            &FetchedServer::container_name(server_id),
            Some(AttachContainerOptions {
                logs: Some(true),
                stderr: Some(true),
                stdout: Some(true),
                stdin: Some(true),
                stream: Some(true),
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
    state: DaemonState,
    cpu_max: f32,
    ram_max: i64,
    swap_max: i64,
    mut result2: impl Stream<Item = Result<Stats, Error>> + Sized + Send + Unpin + 'static,
    server_data_arc: Arc<Mutex<ServerData>>,
    storage_lock: OwnedRwLockReadGuard<(), ()>,
    storage_value: Arc<(AtomicI64, AtomicI64)>,
    storage_stats_lock: Arc<Mutex<()>>,
) {
    let storage_stats_guard = storage_stats_lock.lock().await;
    let mut next = Instant::now().add(Duration::from_millis(500));
    loop {
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
                    let _ = stats_sender.send(ChannelWriteData::Stats(<Stats as Into<ServerStats>>::into(stats).with_disk(storage_value.0.load(Ordering::Relaxed)).with_max(cpu_max, ram_max, swap_max, storage_value.1.load(Ordering::Relaxed))));
                }
            }
            _elapsed = tokio::time::sleep_until(next) => {
                next = Instant::now().add(Duration::from_millis(500));
                let stats_sender = stats_sender.clone();
                let docker = state.docker.clone();
                let server_data_arc = server_data_arc.clone();
                tokio::task::spawn(async move {
                    FetchedServer::update_status(server_id, &stats_sender, &docker, &server_data_arc).await;
                });
            }
        }
    }
    drop(storage_lock);
    let mut failure = if let Ok(inspection) = state
        .docker
        .inspect_container(&FetchedServer::container_name(server_id), None)
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
        FetchedServer::update_status0(server_id, &state.docker, &server_data).await;
    }
}
