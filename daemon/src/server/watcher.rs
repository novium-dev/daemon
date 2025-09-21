// SPDX-License-Identifier: AGPL-3.0-only
use crate::server::FetchedServer;
use crate::server::start::ServerStartError;
use crate::sync::Mutex;
use crate::utils::ChannelWriteData;
use crate::{Daemon, DaemonExtra, ServerData};
use bollard::container::LogOutput;
use bollard::errors::Error;
use bollard::models::ContainerStatsResponse;
use bollard::query_parameters::{AttachContainerOptions, InspectContainerOptions, StatsOptions};
use cap_async_std::async_std::prelude::Stream;
use cap_async_std::async_std::stream::StreamExt as _;
use core::ops::Add as _;
use core::pin::Pin;
use core::sync::atomic::{AtomicI64, Ordering};
use core::time::Duration;
use daemon_common::service::server::ServerStats;
use ringbuffer::AllocRingBuffer;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::OwnedRwLockReadGuard;
use tokio::sync::broadcast::Sender;
use tokio::time::Instant;

#[expect(clippy::too_many_arguments)]
pub async fn attach(
    daemon: &Daemon<impl DaemonExtra>,
    server_id: u32,
    cpu_max: Option<f32>,
    ram_max: Option<u32>,
    swap_max: Option<u32>,
    server_data: &ServerData,
    server_data_arc: Arc<Mutex<ServerData>>,
    storage_lock: OwnedRwLockReadGuard<(), ()>,
    storage_value: Arc<(AtomicI64, AtomicI64)>,
    storage_stats_lock: Arc<Mutex<()>>,
) -> Result<(), ServerStartError> {
    let result = daemon
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

    let result2 = daemon.docker.stats(
        &FetchedServer::container_name(server_id),
        Some(StatsOptions {
            stream: true,
            one_shot: false,
        }),
    );

    let sender = (*server_data.sender).clone();
    let stats_sender = (*server_data.stats_sender).clone();
    let buffer = Arc::clone(&server_data.buffer);
    let daemon = daemon.clone();

    tokio::task::spawn(watch(
        daemon,
        server_id,
        result.output,
        sender,
        stats_sender,
        buffer,
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

#[expect(clippy::too_many_arguments)]
pub async fn watch(
    daemon: Daemon<impl DaemonExtra>,
    server_id: u32,
    mut output: Pin<Box<dyn Stream<Item = Result<LogOutput, Error>> + Send>>,
    sender: Sender<ChannelWriteData>,
    stats_sender: Sender<ChannelWriteData>,
    buffer: Arc<Mutex<AllocRingBuffer<u8>>>,
    cpu_max: Option<f32>,
    ram_max: Option<u32>,
    swap_max: Option<u32>,
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
            drop(server_data);
            reset_counter = true;
        }
        tokio::select! {
            log = output.next() => {
                match log {
                    Some(Ok(log)) => {
                        let mut buffer = buffer.lock().await;
                        FetchedServer::console(&sender, &mut buffer, log.to_string());
                    },
                    Some(Err(_err)) => return,
                    None => break,
                }
            }
            stats = result2.next() => {
                if let Some(Ok(stats)) = stats {
                    let disk = storage_value.0.load(Ordering::Relaxed);
                    let disk_max = storage_value.1.load(Ordering::Relaxed);
                    let _ = stats_sender.send(ChannelWriteData::Stats(<ContainerStatsResponse as Into<ServerStats>>::into(stats).with_disk(if disk == -1 { None } else { Some(disk.unsigned_abs()) }).with_max(cpu_max, ram_max, swap_max, if disk_max == -1 { None } else { Some(disk_max.unsigned_abs()) })));
                }
            }
            _elapsed = tokio::time::sleep_until(next) => {
                next = Instant::now().add(Duration::from_millis(500));
                let stats_sender = stats_sender.clone();
                let docker = Arc::clone(&daemon.docker);
                let server_data_arc = Arc::clone(&server_data_arc);
                tokio::task::spawn(async move {
                    FetchedServer::update_status(server_id, HashSet::new(), &stats_sender, &docker, &server_data_arc).await;
                });
            }
        }
    }
    drop(storage_lock);
    let mut failure = if let Ok(inspection) = daemon
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
            .is_some_and(|code| code != 0)
    } else {
        false
    };
    let mut server_data = server_data_arc.lock().await;
    failure &= !server_data.stopping;
    server_data.stopping = false;

    let mut failure_exceeded = false;
    if !server_data.restart && failure && start.elapsed() <= Duration::from_secs(60) {
        server_data.failure_counter = server_data.failure_counter.saturating_add(1);
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
            daemon.clone(),
            server_id,
            Duration::from_secs(1),
        ));
    }
    let mut buffer = buffer.lock().await;
    FetchedServer::console(&server_data.sender, &mut buffer, "\n".to_owned());
    FetchedServer::log(&server_data.sender, &mut buffer, "Server stopped!");
    if failure_exceeded {
        FetchedServer::log(
            &server_data.sender,
            &mut buffer,
            "Server crashed 3 times in a row. Server will not restart",
        );
    }
    drop(buffer);
    let disk = storage_value.0.load(Ordering::Relaxed);
    let disk_max = storage_value.1.load(Ordering::Relaxed);
    let _ = stats_sender.send(ChannelWriteData::Stats(
        ServerStats::default()
            .with_disk(if disk == -1 {
                None
            } else {
                Some(disk.unsigned_abs())
            })
            .with_max(
                cpu_max,
                ram_max,
                swap_max,
                if disk_max == -1 {
                    None
                } else {
                    Some(disk_max.unsigned_abs())
                },
            ),
    ));
    drop(storage_stats_guard);
    if !do_restart {
        tokio::time::sleep(Duration::from_secs(1)).await;
        FetchedServer::update_status0(
            server_id,
            FetchedServer::resolve_locked(server_id, &daemon.conn).await,
            &daemon.docker,
            &server_data,
        )
        .await;
    }
    drop(server_data);
}
