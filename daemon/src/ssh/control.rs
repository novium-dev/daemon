// SPDX-License-Identifier: AGPL-3.0-only
use crate::server::FetchedServer;
use crate::server::status::InternalServerState;
use crate::ssh::handler::{DaemonChannel, DaemonSSHHandler};
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageError, storage_read_timeout};
use crate::utils::{ChannelWriteData, SyncIOBridgeExt, from_mib};
use crate::{Daemon, DaemonExtra, ServerData, utils};
use chrono::{TimeZone as _, Utc};
use core::cmp::{max, min};
use core::fmt::Write as _;
use core::sync::atomic::{AtomicU16, Ordering};
use core::time::Duration;
use crossterm::terminal::ClearType;
use daemon_common::node::BackendAuthServerPermissions;
use daemon_common::service::files::task::StorageTask;
use daemon_common::service::server::ServerStats;
use daemon_common::service::server::task::ServerTask;
use num_traits::{FromPrimitive as _, ToPrimitive as _};
use ratatui::backend::{Backend, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Constraint, Direction, Flex, Layout, Margin, Position, Rect, Size};
use ratatui::style::{Color, Style, Stylize as _};
use ratatui::symbols::Marker;
use ratatui::text::Text;
use ratatui::widgets::{Axis, Block, BorderType, Borders, Chart, Dataset, GraphType};
use ratatui::{Frame, Terminal};
use ringbuffer::{ConstGenericRingBuffer, RingBuffer as _};
use russh::server::{Msg, Session};
use russh::{ChannelId, ChannelStream};
use std::io;
use std::io::{ErrorKind, Write};
use std::sync::Arc;
use tokio::io::{AsyncReadExt as _, AsyncWrite};
use tokio::runtime::Handle;
use tokio::sync::broadcast::error::TryRecvError;
use tokio_util::io::SyncIoBridge;
use tracing::{debug, error, warn};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Event {
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Enter,
    Mouse {
        button: u8,
        dragging: bool,
        x: u16,
        y: u16,
    },
}

impl<T: DaemonExtra> DaemonSSHHandler<T> {
    pub(crate) async fn ssh_control(
        &mut self,
        channel_id: ChannelId,
        session: &mut Session,
        server_id: String,
    ) -> Result<(), russh::Error> {
        let Some(auth) = &self.auth else {
            return Err(russh::Error::NotAuthenticated);
        };

        let server = Self::find_server(auth, &server_id);
        let (internal_server_id, power) = match server {
            None => {
                debug!("Control access denied. Server not found");
                return session.channel_failure(channel_id);
            }
            Some(server) => {
                if !server
                    .permissions
                    .contains(BackendAuthServerPermissions::CONTROL_VIEW)
                {
                    debug!("Control access denied. No permission");
                    return session.channel_failure(channel_id);
                }
                (
                    server.id,
                    server
                        .permissions
                        .contains(BackendAuthServerPermissions::CONTROL_POWER),
                )
            }
        };
        let mut servers = self.daemon.servers.lock().await;
        let server_data_arc = Arc::clone(match servers.get(&internal_server_id) {
            None => servers.entry(internal_server_id).or_default(),
            Some(server) => server,
        });
        drop(servers);

        match self.channels.get_mut(&channel_id) {
            None => {
                debug!("Tried to access a non existent channel");
                session.channel_failure(channel_id)
            }
            Some(DaemonChannel {
                channel,
                cols,
                rows,
                ..
            }) => {
                let Some(channel) = channel.take() else {
                    debug!("Channel was already taken");
                    return session.channel_failure(channel_id);
                };
                let cols = match cols {
                    None => {
                        debug!("Size is None on exec");
                        return session.channel_failure(channel_id);
                    }
                    Some(cols) => Arc::clone(cols),
                };
                let rows = match rows {
                    None => {
                        debug!("Size is None on exec");
                        return session.channel_failure(channel_id);
                    }
                    Some(rows) => Arc::clone(rows),
                };

                let daemon = self.daemon.clone();
                tokio::task::spawn_blocking(move || {
                    new_control_session(
                        &daemon,
                        internal_server_id,
                        &server_id,
                        power,
                        channel.into_stream(),
                        &server_data_arc,
                        &cols,
                        &rows,
                    );
                });
                session.channel_success(channel_id)
            }
        }
    }
}

#[expect(clippy::too_many_arguments)]
fn new_control_session(
    daemon: &Daemon<impl DaemonExtra>,
    server_id: u32,
    name: &str,
    power: bool,
    mut stream: ChannelStream<Msg>,
    server_data: &ServerData,
    cols: &AtomicU16,
    rows: &AtomicU16,
) {
    let mut cpu = ConstGenericRingBuffer::<Option<f64>, 120>::new();
    let mut ram = ConstGenericRingBuffer::<Option<u64>, 120>::new();
    let mut swap = ConstGenericRingBuffer::<Option<u64>, 120>::new();
    cpu.extend([None; 120]);
    ram.extend([None; 120]);
    swap.extend([None; 120]);

    let mut current_stat = ServerStats::default();

    let mut buf = [0u8; 1024];
    let mut ring1 = ConstGenericRingBuffer::<u8, 3>::new();
    ring1.extend([0u8, 0u8, 0u8]);
    let mut ring2 = ConstGenericRingBuffer::<u8, 6>::new();
    ring2.extend([0u8, 0u8, 0u8, 0u8, 0u8, 0u8]);

    let mut events = Vec::new();

    let mut selected = 0u8;

    let conn = Arc::clone(&daemon.conn);
    let mut server_state: InternalServerState = if Handle::current()
        .block_on(async move { FetchedServer::resolve_locked(server_id, &conn).await })
        .is_empty()
    {
        InternalServerState::Stopped
    } else {
        InternalServerState::Locked
    };

    let mut bridge = SyncIoBridge::new(&mut stream);
    let backend = CustomSizedCrosstermBackendWrapper(CrosstermBackend::new(&mut bridge), 1, 1);

    let Ok(mut terminal) = Terminal::new(backend) else {
        return;
    };
    let mut last_cols = u16::MAX;
    let mut last_rows = u16::MAX;

    let mut recv = server_data.stats_sender.subscribe();

    let _ = terminal.backend_mut().0.write_all(b"\x1B[?1000h");

    let mut last_second = 0;
    loop {
        let cols = cols.load(Ordering::Relaxed);
        let rows = rows.load(Ordering::Relaxed);

        if last_cols != cols || last_rows != rows {
            terminal.backend_mut().1 = cols;
            terminal.backend_mut().2 = rows;

            let _ = terminal.resize(Rect::new(0, 0, cols, rows));
            last_cols = cols;
            last_rows = rows;
            let _ = terminal.backend_mut().0.write_all(
                crossterm::terminal::Clear(ClearType::Purge)
                    .to_string()
                    .as_bytes(),
            );
        }

        let state2 = server_state.clone();

        let name = name.to_owned();

        let cpu2 = cpu.clone();
        let ram2 = ram.clone();
        let swap2 = swap.clone();

        #[expect(
            clippy::arithmetic_side_effects,
            reason = "there are proper bounds checks, overflow/underflow should never occur"
        )]
        let _ = terminal.draw(move |frame| {
            if cols < 32u16 || rows < 12u16 {
                return;
            }

            frame.render_widget(
                Text::raw(&name).centered(),
                Rect::new(0u16, 0u16, cols, 1u16),
            );

            let state_border = Block::new()
                .borders(Borders::ALL)
                .title_top("State")
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(Color::White));

            let mut state_text = state2.to_string();
            if let Some(time) = state2.since() {
                let time = Utc.timestamp_millis_opt(time).unwrap();
                let duration = Utc::now().signed_duration_since(time);
                let _ = write!(
                    &mut state_text,
                    " ({:02}:{:02}:{:02}:{:02})",
                    duration.num_days(),
                    duration.num_hours() - duration.num_days().saturating_mul(24i64),
                    duration.num_minutes() - duration.num_hours().saturating_mul(60i64),
                    duration.num_seconds() - duration.num_minutes().saturating_mul(60i64)
                );
            }

            frame.render_widget(state_text, Rect::new(2u16, 2u16, cols - 4u16, 1u16));
            frame.render_widget(state_border, Rect::new(1u16, 1u16, cols - 2u16, 3u16));

            let power_border = Block::new()
                .borders(Borders::ALL)
                .title_top("Power")
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(Color::White));
            frame.render_widget(power_border, Rect::new(1u16, 4u16, cols - 2u16, 5u16));

            for (pos, index, label, enabled, area) in calc_buttons(&state2, power, cols) {
                let style = match (pos, enabled, selected == index) {
                    (0, true, true) => Style::new().fg(Color::Green),

                    (1 | 3, true, true) => Style::new().fg(Color::Red),

                    (2, true, true) => Style::new().fg(Color::Yellow),

                    (_, false, _) => Style::new().fg(Color::DarkGray),
                    _ => Style::new().fg(Color::White),
                };

                let border = Block::new()
                    .border_style(style)
                    .border_type(BorderType::Plain)
                    .borders(Borders::ALL);

                let text = Text::raw(label).style(style).centered();

                frame.render_widget(border, area);
                frame.render_widget(text, area.inner(Margin::new(1u16, 1u16)));
            }

            draw_stats(frame, cols, rows, &cpu2, &ram2, &swap2, &current_stat);
        });

        if Handle::current().block_on(read(
            daemon.clone(),
            server_id,
            server_state.clone(),
            terminal.backend_mut().inner_mut(),
            &mut buf,
            &mut ring1,
            &mut ring2,
            &mut events,
            power,
            &mut selected,
            (
                server_state.start(),
                server_state.stop(),
                server_state.stop(),
                server_state.kill_show(),
            ),
            cols,
        )) {
            break;
        }

        let second = Utc::now()
            .signed_duration_since(Utc.timestamp_millis_opt(0).unwrap())
            .num_seconds();

        let mut got_stats = false;
        match recv.try_recv() {
            Ok(ChannelWriteData::Stats(stats)) => {
                got_stats = true;
                if last_second != second {
                    last_second = second;
                    cpu.enqueue(stats.cpu);
                    ram.enqueue(stats.ram);
                }
                current_stat = stats;
            }
            Ok(ChannelWriteData::Status(status)) => {
                server_state = status;
            }
            Ok(_) | Err(TryRecvError::Empty) => {}
            Err(err) => {
                warn!(
                    "Failed to receive stats and state data from server: {:?}",
                    err
                );
            }
        }
        if !got_stats && server_state.start() && last_second != second {
            last_second = second;
            cpu.enqueue(current_stat.cpu);
            ram.enqueue(current_stat.ram);
        }
    }
}

fn calc_buttons(
    state: &InternalServerState,
    power: bool,
    width: u16,
) -> Vec<(u16, u8, &'static str, bool, Rect)> {
    let mut index = 0u8;
    [
        ("Start", state.start() && power),
        ("Stop", state.stop() && power),
        ("Restart", state.stop() && power),
        ("Kill", state.kill_show() && power),
    ]
    .into_iter()
    .enumerate()
    .filter_map(|(pos, (label, enabled))| {
        u8::try_from(pos)
            .map(|pos| (u16::from(pos), (label, enabled)))
            .ok()
    })
    .map(|(pos, (label, enabled))| {
        let natural_width = width.saturating_sub(4).saturating_div(4).saturating_sub(4);
        let width = min(54, natural_width);
        let offset = natural_width.wrapping_sub(width).saturating_div(2);
        let height = 3;

        let area = Rect::new(
            pos.saturating_mul(natural_width)
                .saturating_add(2)
                .saturating_add(pos.saturating_mul(4))
                .saturating_add(2)
                .saturating_add(offset),
            5,
            width,
            height,
        );
        let old_index = index;
        index = index.saturating_add(u8::from(enabled));
        (pos, old_index, label, enabled, area)
    })
    .collect()
}

#[inline(always)]
fn draw_stats(
    frame: &mut Frame,
    width: u16,
    height: u16,
    cpu: &ConstGenericRingBuffer<Option<f64>, 120>,
    ram: &ConstGenericRingBuffer<Option<u64>, 120>,
    swap: &ConstGenericRingBuffer<Option<u64>, 120>,
    current_stat: &ServerStats,
) {
    debug_assert!(width >= 32);
    debug_assert!(height >= 12);
    if height >= 20 {
        draw_chart_stats(frame, width, height, cpu, ram, swap, current_stat);
    } else {
        draw_compact_stats(frame, width, current_stat);
    }
}

#[inline(always)]
#[expect(clippy::arithmetic_side_effects, reason = "width is bounds checked")]
fn draw_compact_stats(frame: &mut Frame, width: u16, current_stat: &ServerStats) {
    debug_assert!(width >= 32);

    let stats_border = Block::new()
        .borders(Borders::ALL)
        .title_top("Stats")
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::White));

    let stats_area = Rect::new(1u16, 9u16, width - 2u16, 3u16);

    let stats = Layout::new(
        Direction::Horizontal,
        [
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Fill(1),
        ],
    )
    .flex(Flex::SpaceAround)
    .split(stats_area.inner(Margin::new(1u16, 1u16)));
    let mut stats = stats.iter();

    frame.render_widget(stats_border, stats_area);

    let cpu_current_str = current_stat
        .cpu
        .map_or_else(|| "-".to_owned(), |cpu| format!("{:.2} %", cpu * 100.0f64));
    let cpu_max_str = current_stat.cpu_max.map_or_else(
        || "\u{221e}".to_owned(),
        |cpu_max| format!("{:.2} %", cpu_max * 100.0f32),
    );
    let cpu_text = Text::raw(format!("CPU: {cpu_current_str} / {cpu_max_str}"));

    let ram_current_str = current_stat
        .ram
        .as_ref()
        .map(u64::to_f64)
        .unwrap_or_default()
        .map_or_else(
            || "-".to_owned(),
            |ram| format!("{:.2} MiB", ram / 1_048_576f64),
        );
    let ram_max_str = current_stat
        .ram_max
        .map_or_else(|| "\u{221e}".to_owned(), |ram_max| format!("{ram_max} MiB"));
    let mem_text = Text::raw(format!("RAM: {ram_current_str} / {ram_max_str}"));

    let swap_current_str = current_stat
        .swap
        .as_ref()
        .map(u64::to_f64)
        .unwrap_or_default()
        .map_or_else(
            || "-".to_owned(),
            |swap| format!("{:.2} MiB", swap / 1_048_576f64),
        );
    let swap_max_str = current_stat.swap_max.map_or_else(
        || "\u{221e}".to_owned(),
        |swap_max| format!("{swap_max} MiB"),
    );
    let swap_text = Text::raw(format!("Swap: {swap_current_str} / {swap_max_str}"));

    let disk_current_str = current_stat
        .disk
        .as_ref()
        .map(u64::to_f64)
        .unwrap_or_default()
        .map_or_else(
            || "-".to_owned(),
            |disk| {
                let disk = disk / 1_048_576.0f64;
                if disk > 1024.0f64 {
                    format!("{:.2} GiB", disk / 1024.0f64)
                } else {
                    format!("{disk:.2} MiB")
                }
            },
        );
    let disk_max_str = current_stat.disk_max.map_or_else(
        || "\u{221e}".to_owned(),
        |disk_max| {
            if disk_max > 1024u64 {
                disk_max.to_f64().map_or_else(
                    || "-".to_owned(),
                    |disk_max| format!("{:.2} GiB", disk_max / 1024.0f64),
                )
            } else {
                format!("{disk_max:.2} MiB")
            }
        },
    );
    let disk_text = Text::raw(format!("Disk: {disk_current_str} / {disk_max_str}"));

    if let Some(area) = stats.next() {
        frame.render_widget(cpu_text, *area);
    }
    if let Some(area) = stats.next() {
        frame.render_widget(mem_text, *area);
    }
    if let Some(area) = stats.next() {
        frame.render_widget(swap_text, *area);
    }
    if let Some(area) = stats.next() {
        frame.render_widget(disk_text, *area);
    }
}

#[inline(always)]
fn draw_chart_stats(
    frame: &mut Frame,
    width: u16,
    height: u16,
    cpu: &ConstGenericRingBuffer<Option<f64>, 120>,
    ram: &ConstGenericRingBuffer<Option<u64>, 120>,
    swap: &ConstGenericRingBuffer<Option<u64>, 120>,
    current_stat: &ServerStats,
) {
    debug_assert!(width >= 32);
    debug_assert!(height >= 20);
    let stats_border = Block::new()
        .borders(Borders::ALL)
        .title_top("Stats")
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::White));

    let stats_area = Rect::new(
        1,
        9,
        width.saturating_sub(2),
        min(24, height.saturating_sub(10)),
    );

    let stats = Layout::new(
        Direction::Horizontal,
        [Constraint::Percentage(50), Constraint::Fill(1)],
    )
    .flex(Flex::SpaceAround)
    .split(stats_area.inner(Margin::new(1, 1)));
    let mut stats = stats.iter();

    frame.render_widget(stats_border, stats_area);

    if let Some(area) = stats.next() {
        draw_cpu_chart(frame, cpu, current_stat, *area);
    }
    if let Some(area) = stats.next() {
        draw_mem_chart(frame, ram, swap, current_stat, *area);
    }
}

#[inline(always)]
fn draw_cpu_chart(
    frame: &mut Frame,
    cpu: &ConstGenericRingBuffer<Option<f64>, 120>,
    current_stat: &ServerStats,
    mut area: Rect,
) {
    let cpu_peak = current_stat
        .cpu_max
        .map_or(50.0f64, f64::from)
        .max(cpu.iter().copied().flatten().fold(0.0f64, f64::max))
        .max(50.0f64);
    let cpu_peak_divided_by_50 = u16::from_f64(cpu_peak * 0.02f64).unwrap_or(2);
    let mut cpu_peak_multiplied = cpu_peak_divided_by_50.saturating_mul(50u16);
    if cpu_peak > f64::from(cpu_peak_multiplied) {
        cpu_peak_multiplied = cpu_peak_multiplied.saturating_add(50u16);
    }

    let cpu_data: Vec<_> = cpu
        .iter()
        .skip(90usize)
        .copied()
        .enumerate()
        .filter_map(|(index, mem)| index.to_f64().map(|index| (index, mem)))
        .map(|(index, cpu)| (index, cpu.unwrap_or(-1f64)))
        .collect();
    let cpu = Chart::new(vec![
        Dataset::default()
            .fg(Color::Yellow)
            .graph_type(GraphType::Line)
            .marker(Marker::Braille)
            .data(&cpu_data),
    ])
    .x_axis(
        Axis::default()
            .bounds([0.0f64, 30.0f64])
            .labels(["30s", "0s"]),
    )
    .y_axis(
        Axis::default()
            .bounds([0.0f64, f64::from(cpu_peak_multiplied)])
            .labels(["0%".into(), format!("{cpu_peak_multiplied}%")]),
    )
    .block(
        Block::bordered()
            .title_top("CPU")
            .border_type(BorderType::Plain)
            .border_style(Style::new().fg(Color::White)),
    );
    let cpu_current_str = current_stat
        .cpu
        .map_or_else(|| "-".to_owned(), |cpu| format!("{:.2} %", cpu * 100.0f64));
    let cpu_max_str = current_stat.cpu_max.map_or_else(
        || "\u{221e}".to_owned(),
        |cpu_max| format!("{:.2} %", cpu_max * 100.0f32),
    );
    let cpu_text = Text::raw(format!("CPU: {cpu_current_str} / {cpu_max_str}")).fg(Color::Yellow);
    let cpu_text_block = Block::bordered().border_style(Style::new().fg(Color::White));

    frame.render_widget(cpu, area);
    area.x = area.x.saturating_add(1);
    area.y = area.y.saturating_add(1);
    area.height = 1;

    let cpu_text_width = cpu_text.width() as u16;
    let cpu_text_x = area
        .x
        .saturating_add(area.width)
        .saturating_sub(4)
        .saturating_sub(cpu_text_width);
    frame.render_widget(
        cpu_text_block,
        Rect::new(cpu_text_x, area.y, cpu_text_width.saturating_add(2), 3),
    );
    frame.render_widget(
        cpu_text,
        Rect::new(
            cpu_text_x.saturating_add(1),
            area.y.saturating_add(1),
            cpu_text_width,
            1,
        ),
    );
}

#[inline(always)]
fn draw_mem_chart(
    frame: &mut Frame,
    ram: &ConstGenericRingBuffer<Option<u64>, 120>,
    swap: &ConstGenericRingBuffer<Option<u64>, 120>,
    current_stat: &ServerStats,
    mut area: Rect,
) {
    let mem_peak = u64::from(current_stat.ram_max.unwrap_or(512u32))
        .max(
            ram.iter()
                .copied()
                .flatten()
                .fold(0, |a, b| from_mib(a).max(b)),
        )
        .max(
            swap.iter()
                .copied()
                .flatten()
                .fold(0, |a, b| from_mib(a).max(b)),
        )
        .max(512u64);
    let mut mem_peak_power_of_2 = 0u16;
    while u64::from(1u16 << mem_peak_power_of_2) < mem_peak {
        mem_peak_power_of_2 = mem_peak_power_of_2.saturating_add(1);
    }
    let peak = f64::from(1u16 << mem_peak_power_of_2);

    let ram_color = Color::Yellow;
    let swap_color = Color::Magenta;

    let ram_data: Vec<_> = ram
        .iter()
        .skip(90usize)
        .copied()
        .enumerate()
        .filter_map(|(index, mem)| index.to_f64().map(|index| (index, mem)))
        .map(|(index, mem)| (index, mem.map_or(-1f64, |mem| mem.to_f64().unwrap_or(peak))))
        .collect();
    let swap_data: Vec<_> = swap
        .iter()
        .skip(90)
        .copied()
        .enumerate()
        .filter_map(|(index, mem)| index.to_f64().map(|index| (index, mem)))
        .map(|(index, mem)| (index, mem.map_or(-1f64, |mem| mem.to_f64().unwrap_or(peak))))
        .collect();

    let mem = Chart::new(vec![
        Dataset::default()
            .fg(ram_color)
            .graph_type(GraphType::Line)
            .marker(Marker::Braille)
            .data(&ram_data),
        Dataset::default()
            .fg(swap_color)
            .graph_type(GraphType::Line)
            .marker(Marker::Braille)
            .data(&swap_data),
    ])
    .x_axis(
        Axis::default()
            .bounds([0.0f64, 30.0f64])
            .labels(["30s", "0s"]),
    )
    .y_axis(
        Axis::default()
            .bounds([0.0f64, peak])
            .labels(["0M".into(), format!("{}M", 1u16 << mem_peak_power_of_2)]),
    )
    .block(
        Block::bordered()
            .title_top("Memory")
            .border_type(BorderType::Plain)
            .border_style(Style::new().fg(Color::White)),
    );
    let ram_current_str = current_stat
        .ram
        .as_ref()
        .map(u64::to_f64)
        .unwrap_or_default()
        .map_or_else(
            || "-".to_owned(),
            |ram| format!("{:.2} MiB", ram / 1_048_576f64),
        );
    let ram_max_str = current_stat
        .ram_max
        .map_or_else(|| "\u{221e}".to_owned(), |ram_max| format!("{ram_max} MiB"));

    let swap_current_str = current_stat
        .swap
        .as_ref()
        .map(u64::to_f64)
        .unwrap_or_default()
        .map_or_else(
            || "-".to_owned(),
            |swap| format!("{:.2} MiB", swap / 1_048_576f64),
        );
    let swap_max_str = current_stat.swap_max.map_or_else(
        || "\u{221e}".to_owned(),
        |swap_max| format!("{swap_max} MiB"),
    );

    let ram_text = Text::raw(format!("RAM: {ram_current_str} / {ram_max_str}")).fg(ram_color);
    let swap_text = Text::raw(format!("SWP: {swap_current_str} / {swap_max_str}")).fg(swap_color);
    let mem_text_block = Block::bordered().border_style(Style::new().fg(Color::White));

    frame.render_widget(mem, area);
    area.x = area.x.saturating_add(1);
    area.y = area.y.saturating_add(1);
    area.height = 1;

    let cpu_text_width = max(ram_text.width() as u16, swap_text.width() as u16);
    let cpu_text_x = area
        .x
        .saturating_add(area.width)
        .saturating_sub(4)
        .saturating_sub(cpu_text_width);
    frame.render_widget(
        mem_text_block,
        Rect::new(cpu_text_x, area.y, cpu_text_width.saturating_add(2), 4),
    );
    frame.render_widget(
        ram_text,
        Rect::new(
            cpu_text_x.saturating_add(1),
            area.y.saturating_add(1),
            cpu_text_width,
            1,
        ),
    );
    frame.render_widget(
        swap_text,
        Rect::new(
            cpu_text_x.saturating_add(1),
            area.y.saturating_add(2),
            cpu_text_width,
            1,
        ),
    );
}

#[derive(Debug, Default, Clone, Eq, PartialEq, Hash)]
pub struct CrosstermBackendCopy<W: Write> {
    /// The writer used to send commands to the terminal.
    writer: W,
}

struct CustomSizedCrosstermBackendWrapper<W: Write>(CrosstermBackend<W>, u16, u16);

impl<W: Write> Write for CustomSizedCrosstermBackendWrapper<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        <CrosstermBackend<W> as Write>::write(&mut self.0, buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        <CrosstermBackend<W> as Write>::flush(&mut self.0)
    }
}

impl<W: Write> Backend for CustomSizedCrosstermBackendWrapper<W> {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        <CrosstermBackend<W> as Backend>::draw(&mut self.0, content)
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        <CrosstermBackend<W> as Backend>::append_lines(&mut self.0, n)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        <CrosstermBackend<W> as Backend>::hide_cursor(&mut self.0)
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        <CrosstermBackend<W> as Backend>::show_cursor(&mut self.0)
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        <CrosstermBackend<W> as Backend>::get_cursor_position(&mut self.0)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        <CrosstermBackend<W> as Backend>::set_cursor_position(&mut self.0, position)
    }

    fn clear(&mut self) -> io::Result<()> {
        <CrosstermBackend<W> as Backend>::clear(&mut self.0)
    }

    fn clear_region(&mut self, clear_type: ratatui::backend::ClearType) -> io::Result<()> {
        <CrosstermBackend<W> as Backend>::clear_region(&mut self.0, clear_type)
    }

    fn size(&self) -> io::Result<Size> {
        Ok(Size {
            width: self.1,
            height: self.2,
        })
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: self.size()?,
            pixels: Size::ZERO,
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        <CrosstermBackend<W> as Backend>::flush(&mut self.0)
    }
}

impl<T: AsyncWrite + Unpin> SyncIOBridgeExt<T>
    for CustomSizedCrosstermBackendWrapper<&mut SyncIoBridge<T>>
{
    fn inner_mut(&mut self) -> &mut T {
        let backend_raw_copy =
            core::ptr::from_mut(&mut self.0).cast::<CrosstermBackendCopy<&mut SyncIoBridge<T>>>();
        // SAFETY: As long as the struct matches if the upstream version, there is no undefined behaviour
        unsafe { &mut *backend_raw_copy }.writer.inner_mut()
    }
}

#[expect(clippy::too_many_arguments)]
async fn read(
    daemon: Daemon<impl DaemonExtra>,
    server_id: u32,
    server_state: InternalServerState,
    stream: &mut ChannelStream<Msg>,
    buf: &mut [u8; 1024],
    ring1: &mut ConstGenericRingBuffer<u8, 3>,
    ring2: &mut ConstGenericRingBuffer<u8, 6>,
    events: &mut Vec<Event>,
    power: bool,
    selected: &mut u8,
    (start, stop, restart, kill): (bool, bool, bool, bool),
    width: u16,
) -> bool {
    if let Ok(Err(err)) = tokio::time::timeout(
        Duration::from_millis(25),
        read_inner(stream, buf, ring1, ring2, events),
    )
    .await
    {
        debug!("Failed reading control terminal: {:?}", err);
        return true;
    }

    if power {
        let size = [start, stop, restart, kill].into_iter().map(u8::from).sum();

        let mut block = false;
        if *selected >= size {
            block = true;
            while *selected >= size
                && let Some(selected_checked) = selected.checked_sub(1)
            {
                *selected = selected_checked;
            }
        }
        if block {
            events.clear();
            return false;
        }

        // Start - stop - restart - kill
        for event in events.drain(..) {
            match event {
                Event::ArrowLeft if *selected > 0 => *selected = selected.saturating_sub(1),
                Event::ArrowRight if (selected.saturating_add(1)) < size => {
                    *selected = selected.saturating_add(1);
                }
                Event::Enter if *selected < size => {
                    launch_selected(
                        daemon.clone(),
                        server_id,
                        *selected,
                        (start, stop, restart, kill),
                    )
                    .await;
                }
                Event::Mouse {
                    button: 0,
                    dragging: false,
                    x,
                    y,
                } => {
                    for (_, index, _, enabled, area) in calc_buttons(&server_state, power, width) {
                        if area.contains(Position::new(x, y)) && enabled {
                            launch_selected(
                                daemon.clone(),
                                server_id,
                                index,
                                (start, stop, restart, kill),
                            )
                            .await;
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
    } else {
        events.clear();
    }

    false
}

async fn launch_selected(
    daemon: Daemon<impl DaemonExtra>,
    server_id: u32,
    selected: u8,
    (start, stop, restart, kill): (bool, bool, bool, bool),
) {
    // Determine the index of the buttons
    // (normal addition with overflow does result in a panic in debug builds)
    let start = if start { 0u8 } else { 255 };
    let stop = if stop { start.wrapping_add(1) } else { 255 };
    let restart = if restart {
        max(start.wrapping_add(1), stop.wrapping_add(1))
    } else {
        255
    };
    let kill = if kill {
        max(
            max(start.wrapping_add(1), stop.wrapping_add(1)),
            restart.wrapping_add(1),
        )
    } else {
        255
    };

    let (server, server_data) = match utils::server(&daemon.conn, &daemon.servers, server_id).await
    {
        Ok(Some((server, server_data))) => (server, server_data),
        Ok(None) => {
            warn!(
                "Server, that is open in ssh (control), does not exist anymore: {}",
                server_id
            );
            return;
        }
        Err(err) => {
            error!("Failed to select server: {:?}", err);
            return;
        }
    };
    tokio::task::spawn(async move {
        if selected == start {
            let Ok(mut lock) = tokio::time::timeout(
                Duration::from_millis(100),
                Arc::clone(&server_data.lock).write_owned(ServerTask::Start),
            )
            .await
            else {
                return;
            };
            let Ok(storage) = storage_read_timeout::<StorageError>(
                &daemon.config,
                &*daemon.conn,
                &daemon.storages,
                StorageIdentifier::Primary(server_id),
                StorageTask::ServerRunning,
            )
            .await
            else {
                return;
            };
            if let Err(err) = server
                .start(
                    Arc::clone(&daemon.docker),
                    server_data,
                    &mut lock,
                    storage,
                    &daemon,
                )
                .await
            {
                warn!("Failed to start server using ssh (control): {err:?}");
            }
        } else if selected == stop {
            let Ok(lock) = tokio::time::timeout(
                Duration::from_millis(100),
                server_data.lock.read(ServerTask::Stop),
            )
            .await
            else {
                return;
            };
            if let Err(err) = server.stop(&daemon.docker, &server_data, &lock).await {
                warn!("Failed to stop server using ssh (control): {err:?}");
            }
        } else if selected == restart {
            let Ok(lock) = tokio::time::timeout(
                Duration::from_millis(100),
                server_data.lock.read(ServerTask::Restart),
            )
            .await
            else {
                return;
            };
            if let Err(err) = server.restart(&daemon.docker, &server_data, &lock).await {
                warn!("Failed to stop server using ssh (control): {err:?}");
            }
        } else if selected == kill {
            let Ok(lock) = tokio::time::timeout(
                Duration::from_millis(100),
                server_data.lock.read(ServerTask::Kill),
            )
            .await
            else {
                return;
            };
            if let Err(err) = server.kill(&daemon.docker, &server_data, &lock).await {
                warn!("Failed to kill server using ssh (control): {err:?}");
            }
        } else {
            // Nothing is selected, so nothing to do
        }
    });
}

// -> [27, 91, 67]
// <- [27, 91, 68]
// ^ [27, 91, 65]
// v [27, 91, 66]
// \r [13]

async fn read_inner(
    stream: &mut ChannelStream<Msg>,
    buf: &mut [u8; 1024],
    ring1: &mut ConstGenericRingBuffer<u8, 3>,
    ring2: &mut ConstGenericRingBuffer<u8, 6>,
    events: &mut Vec<Event>,
) -> Result<(), io::Error> {
    loop {
        let size = stream.read(buf).await?;
        if size == 0 {
            return Err(io::Error::new(ErrorKind::UnexpectedEof, "Size is zero"));
        }
        let data = buf.get(0..size).ok_or_else(|| {
            io::Error::new(ErrorKind::InvalidData, "Read size larger than buffer")
        })?;
        for byte in data {
            ring1.enqueue(*byte);
            ring2.enqueue(*byte);
            match (ring1[0], ring1[1], ring1[2]) {
                (27, 91, 67) => events.push(Event::ArrowRight),
                (27, 91, 68) => events.push(Event::ArrowLeft),
                (27, 91, 65) => events.push(Event::ArrowUp),
                (27, 91, 66) => events.push(Event::ArrowDown),
                (_, _, 13) => events.push(Event::Enter),
                _ => {}
            }
            if let (27, 91, 77, cb, cx, cy) =
                (ring2[0], ring2[1], ring2[2], ring2[3], ring2[4], ring2[5])
            {
                let Some(cb) = cb.checked_sub(32) else {
                    continue;
                };

                let button = (cb & 0b0000_0011u8) | ((cb & 0b1100_0000u8) >> 4u8); // 1 = left
                let dragging = cb & 0b0010_0000 == 0b0010_0000;

                let x = u16::from(cx.saturating_sub(32)).saturating_sub(1);
                let y = u16::from(cy.saturating_sub(32)).saturating_sub(1);

                events.push(Event::Mouse {
                    button,
                    dragging,
                    x,
                    y,
                });
            }
        }
    }
}
