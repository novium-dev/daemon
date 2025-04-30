use crate::server::FetchedServer;
use crate::server::status::InternalServerState;
use crate::ssh::handler::DaemonSSHHandler;
use crate::storage::StorageIdentifier;
use crate::storage::sync::{StorageError, storage_read};
use crate::sync::Mutex;
use crate::utils::{ChannelWriteData, SyncIOBridgeExt};
use crate::{Daemon, ServerData, utils};
use chrono::{TimeZone, Utc};
use crossterm::terminal::ClearType;
use daemon_common::server::{BackendAuthServerPermissions, ServerStats};
use ratatui::backend::{Backend, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Constraint, Direction, Flex, Layout, Margin, Position, Rect, Size};
use ratatui::style::{Color, Style, Stylize};
use ratatui::symbols::Marker;
use ratatui::text::Text;
use ratatui::widgets::{Axis, Block, BorderType, Borders, Chart, Dataset, GraphType};
use ratatui::{Frame, Terminal};
use ringbuffer::{ConstGenericRingBuffer, RingBuffer};
use russh::server::{Msg, Session};
use russh::{ChannelId, ChannelStream};
use std::cmp::{max, min};
use std::io::{ErrorKind, Write};
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use std::{io, mem};
use tokio::io::{AsyncReadExt, AsyncWrite};
use tokio::runtime::Handle;
use tokio::sync::broadcast::error::TryRecvError;
use tokio_util::io::SyncIoBridge;
use tracing::{debug, error, warn};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum Event {
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

impl DaemonSSHHandler {
    pub(crate) async fn ssh_control(
        &mut self,
        channel_id: ChannelId,
        session: &mut Session,
        server_id: String,
    ) -> Result<(), russh::Error> {
        let auth = self.auth.as_ref().unwrap();

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
        let mut servers = self.state.servers.lock().await;
        let server_data_arc = match servers.get(&internal_server_id) {
            None => {
                servers.insert(
                    internal_server_id,
                    Arc::new(Mutex::new(ServerData::default())),
                );
                servers.get(&internal_server_id).unwrap()
            }
            Some(server) => server,
        }
        .clone();
        drop(servers);

        match self.channels.get_mut(&channel_id) {
            None => {
                debug!("Tried to access a non existent channel");
                session.channel_failure(channel_id)
            }
            Some((channel, _id, cols, rows)) => {
                let channel = channel.take();
                if channel.is_none() {
                    debug!("Channel already taken");
                    return session.channel_failure(channel_id);
                }
                let cols = match cols {
                    None => {
                        debug!("Size is None on exec");
                        return session.channel_failure(channel_id);
                    }
                    Some(cols) => cols.clone(),
                };
                let rows = match rows {
                    None => {
                        debug!("Size is None on exec");
                        return session.channel_failure(channel_id);
                    }
                    Some(rows) => rows.clone(),
                };

                let state = self.state.clone();
                tokio::task::spawn_blocking(move || {
                    new_control_session(
                        internal_server_id,
                        server_id,
                        state,
                        power,
                        channel.unwrap().into_stream(),
                        server_data_arc,
                        cols,
                        rows,
                    )
                });
                session.channel_success(channel_id)
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn new_control_session(
    server_id: u32,
    name: String,
    state: Arc<Daemon>,
    power: bool,
    mut stream: ChannelStream<Msg>,
    arc: Arc<Mutex<ServerData>>,
    cols: Arc<AtomicU32>,
    rows: Arc<AtomicU32>,
) {
    let mut cpu = ConstGenericRingBuffer::<f32, 120>::new();
    let mut ram = ConstGenericRingBuffer::<f32, 120>::new();
    let mut swap = ConstGenericRingBuffer::<f32, 120>::new();
    cpu.extend([-100.0; 120]);
    ram.extend([-100.0; 120]);
    swap.extend([-100.0; 120]);

    let mut current_stat = ServerStats::default();

    let mut buf = [0u8; 1024];
    let mut ring1 = ConstGenericRingBuffer::<u8, 3>::new();
    ring1.extend([0u8, 0u8, 0u8]);
    let mut ring2 = ConstGenericRingBuffer::<u8, 6>::new();
    ring2.extend([0u8, 0u8, 0u8, 0u8, 0u8, 0u8]);

    let mut events = Vec::new();

    let mut selected = 0u8;

    let conn = state.conn.clone();
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
    let mut last_cols = u32::MAX;
    let mut last_rows = u32::MAX;

    let mut recv = {
        let data = arc.blocking_lock();
        data.stats_sender.subscribe()
    };

    let _ = terminal.backend_mut().0.write_all(b"\x1B[?1000h");

    let mut last_second = 0;
    loop {
        let cols = cols.load(Ordering::Relaxed);
        let rows = rows.load(Ordering::Relaxed);
        if cols > u16::MAX as u32 {
            continue;
        }
        if rows > u16::MAX as u32 {
            continue;
        }

        if last_cols != cols || last_rows != rows {
            terminal.backend_mut().1 = cols as u16;
            terminal.backend_mut().2 = rows as u16;

            let _ = terminal.resize(Rect::new(0, 0, cols as u16, rows as u16));
            last_cols = cols;
            last_rows = rows;
            let _ = terminal.backend_mut().0.write_all(
                crossterm::terminal::Clear(ClearType::Purge)
                    .to_string()
                    .as_bytes(),
            );
        }

        let state2 = server_state.clone();

        let width = cols as u16;
        let height = rows as u16;
        let name = name.clone();

        let cpu2 = cpu.clone();
        let ram2 = ram.clone();
        let swap2 = swap.clone();

        let _ = terminal.draw(move |frame| {
            if width < 32 || height < 12 {
                return;
            }

            frame.render_widget(Text::raw(&name).centered(), Rect::new(0, 0, width, 1));

            let state_border = Block::new()
                .borders(Borders::ALL)
                .title_top("State")
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(Color::White));

            let mut state_text = state2.to_string();
            if let Some(time) = state2.since() {
                let time = Utc.timestamp_millis_opt(time).unwrap();
                let duration = Utc::now().signed_duration_since(time);
                state_text += &format!(
                    " ({:02}:{:02}:{:02}:{:02})",
                    duration.num_days(),
                    duration.num_hours() - (duration.num_days() * 24),
                    duration.num_minutes() - (duration.num_hours() * 60),
                    duration.num_seconds() - (duration.num_minutes() * 60)
                );
            }

            frame.render_widget(state_text, Rect::new(2, 2, width - 4, 1));
            frame.render_widget(state_border, Rect::new(1, 1, width - 2, 3));

            let power_border = Block::new()
                .borders(Borders::ALL)
                .title_top("Power")
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(Color::White));
            frame.render_widget(power_border, Rect::new(1, 4, width - 2, 5));

            for (pos, index, label, enabled, area) in calc_buttons(state2, power, width) {
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
                frame.render_widget(text, area.inner(Margin::new(1, 1)));
            }

            draw_stats(frame, width, height, &cpu2, &ram2, &swap2, &current_stat);
        });

        if Handle::current().block_on(read(
            state.clone(),
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
            width,
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
                    cpu.push(stats.cpu);
                    ram.push(stats.ram);
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
            cpu.push(current_stat.cpu);
            ram.push(current_stat.ram);
        }
    }
}

fn calc_buttons(
    state: InternalServerState,
    power: bool,
    width: u16,
) -> Vec<(u16, u8, &'static str, bool, Rect)> {
    let mut index = 0;
    [
        ("Start", state.start() && power),
        ("Stop", state.stop() && power),
        ("Restart", state.stop() && power),
        ("Kill", state.kill_show() && power),
    ]
    .into_iter()
    .enumerate()
    .map(|(pos, (label, enabled))| {
        let pos = pos as u16;
        let natural_width = ((width - 4) / 4) - 4;
        let width = min(54, natural_width);
        let offset = (natural_width - width) / 2;
        let height = 3;

        let area = Rect::new(
            2 + (pos * natural_width) + (pos * 4) + 2 + offset,
            5,
            width,
            height,
        );
        index += enabled as u8;
        (pos, index - enabled as u8, label, enabled, area)
    })
    .collect()
}

fn draw_stats(
    frame: &mut Frame,
    width: u16,
    height: u16,
    cpu: &ConstGenericRingBuffer<f32, 120>,
    ram: &ConstGenericRingBuffer<f32, 120>,
    swap: &ConstGenericRingBuffer<f32, 120>,
    current_stat: &ServerStats,
) {
    if height >= 20 {
        draw_chart_stats(frame, width, height, cpu, ram, swap, current_stat);
    } else {
        draw_compact_stats(frame, width, current_stat);
    }
}

fn draw_compact_stats(frame: &mut Frame, width: u16, current_stat: &ServerStats) {
    let stats_border = Block::new()
        .borders(Borders::ALL)
        .title_top("Stats")
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::White));

    let stats_area = Rect::new(1, 9, width - 2, 3);

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
    .split(stats_area.inner(Margin::new(1, 1)));

    frame.render_widget(stats_border, stats_area);

    let cpu_current_str = if current_stat.cpu == -1f32 {
        "-".to_string()
    } else {
        format!("{:.2} %", current_stat.cpu)
    };
    let cpu_max_str = if current_stat.cpu_max == -1f32 {
        "∞".to_string()
    } else {
        format!("{:.2} %", current_stat.cpu_max)
    };
    let cpu_text = Text::raw(format!("CPU: {} / {}", cpu_current_str, cpu_max_str));

    let mem_current_str = if current_stat.ram == -1f32 {
        "-".to_string()
    } else {
        format!("{:.2} MiB", current_stat.ram)
    };
    let mem_max_str = if current_stat.ram_max == -1 {
        "∞".to_string()
    } else {
        format!("{:.2} MiB", current_stat.ram_max)
    };
    let mem_text = Text::raw(format!("RAM: {} / {}", mem_current_str, mem_max_str));

    let swap_current_str = if current_stat.swap == -1f32 {
        "-".to_string()
    } else {
        format!("{:.2} MiB", current_stat.swap)
    };
    let swap_max_str = if current_stat.swap_max == -1 {
        "∞".to_string()
    } else {
        format!("{:.2} MiB", current_stat.swap_max)
    };
    let swap_text = Text::raw(format!("Swap: {} / {}", swap_current_str, swap_max_str));

    let disk_current_str = if current_stat.disk == -1 {
        "-".to_string()
    } else {
        let disk = current_stat.disk as f32 / 1048576.0;
        if disk > 1024.0 {
            format!("{:.2} GiB", disk / 1024.0)
        } else {
            format!("{:.2} MiB", disk)
        }
    };
    let disk_max_str = if current_stat.disk_max == -1 {
        "∞".to_string()
    } else if current_stat.disk_max > 1024 {
        format!("{:.2} GiB", current_stat.disk_max as f32 / 1024.0)
    } else {
        format!("{:.2} MiB", current_stat.disk_max)
    };
    let disk_text = Text::raw(format!("Disk: {} / {}", disk_current_str, disk_max_str));

    frame.render_widget(cpu_text, stats[0]);
    frame.render_widget(mem_text, stats[1]);
    frame.render_widget(swap_text, stats[2]);
    frame.render_widget(disk_text, stats[3]);
}

fn draw_chart_stats(
    frame: &mut Frame,
    width: u16,
    height: u16,
    cpu: &ConstGenericRingBuffer<f32, 120>,
    ram: &ConstGenericRingBuffer<f32, 120>,
    swap: &ConstGenericRingBuffer<f32, 120>,
    current_stat: &ServerStats,
) {
    let stats_border = Block::new()
        .borders(Borders::ALL)
        .title_top("Stats")
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(Color::White));

    let stats_area = Rect::new(1, 9, width - 2, min(24, height - 10));

    let stats = Layout::new(
        Direction::Horizontal,
        [Constraint::Percentage(50), Constraint::Fill(1)],
    )
    .flex(Flex::SpaceAround)
    .split(stats_area.inner(Margin::new(1, 1)));

    frame.render_widget(stats_border, stats_area);

    draw_cpu_chart(frame, cpu, current_stat, stats[0]);
    draw_mem_chart(frame, ram, swap, current_stat, stats[1]);
}

fn draw_cpu_chart(
    frame: &mut Frame,
    cpu: &ConstGenericRingBuffer<f32, 120>,
    current_stat: &ServerStats,
    mut area: Rect,
) {
    let cpu_peak = current_stat
        .cpu_max
        .max(cpu.iter().copied().fold(0.0, |a, b| a.max(b)))
        .max(50.0);
    let cpu_peak_divided_by_50 = (cpu_peak / 50.0) as u16;
    let mut cpu_peak_multiplied = cpu_peak_divided_by_50 * 50;
    if cpu_peak > cpu_peak_multiplied as f32 {
        cpu_peak_multiplied += 50;
    }

    let cpu_data: Vec<_> = cpu
        .iter()
        .skip(90)
        .copied()
        .enumerate()
        .map(|(index, cpu)| (index as f64, cpu as f64))
        .collect();
    let cpu = Chart::new(vec![
        Dataset::default()
            .fg(Color::Yellow)
            .graph_type(GraphType::Line)
            .marker(Marker::Braille)
            .data(&cpu_data),
    ])
    .x_axis(Axis::default().bounds([0.0, 30.0]).labels(["30s", "0s"]))
    .y_axis(
        Axis::default()
            .bounds([0.0, cpu_peak_multiplied as f64])
            .labels(["0%".into(), format!("{}%", cpu_peak_multiplied)]),
    )
    .block(
        Block::bordered()
            .title_top("CPU")
            .border_type(BorderType::Plain)
            .border_style(Style::new().fg(Color::White)),
    );
    let cpu_current_str = if current_stat.cpu == -1f32 {
        "-".to_string()
    } else {
        format!("{:.2} %", current_stat.cpu)
    };
    let cpu_max_str = if current_stat.cpu_max == -1f32 {
        "∞".to_string()
    } else {
        format!("{:.2} %", current_stat.cpu_max)
    };
    let cpu_text =
        Text::raw(format!("CPU: {} / {}", cpu_current_str, cpu_max_str)).fg(Color::Yellow);
    let cpu_text_block = Block::bordered().border_style(Style::new().fg(Color::White));

    frame.render_widget(cpu, area);
    area.x += 1;
    area.y += 1;
    area.height = 1;

    let cpu_text_width = cpu_text.width() as u16;
    let cpu_text_x = (area.x + area.width) - (2 + cpu_text_width) - 2;
    frame.render_widget(
        cpu_text_block,
        Rect::new(cpu_text_x, area.y, 2 + cpu_text_width, 3),
    );
    frame.render_widget(
        cpu_text,
        Rect::new(cpu_text_x + 1, area.y + 1, cpu_text_width, 1),
    );
}

fn draw_mem_chart(
    frame: &mut Frame,
    ram: &ConstGenericRingBuffer<f32, 120>,
    swap: &ConstGenericRingBuffer<f32, 120>,
    current_stat: &ServerStats,
    mut area: Rect,
) {
    let mem_peak = (current_stat.ram_max as f32)
        .max(ram.iter().copied().fold(0.0, |a, b| a.max(b)))
        .max(swap.iter().copied().fold(0.0, |a, b| a.max(b)))
        .max(512.0);
    let mut mem_peak_power_of_2 = 0u16;
    while ((1 << mem_peak_power_of_2) as f32) < mem_peak {
        mem_peak_power_of_2 += 1;
    }

    let ram_color = Color::Yellow;
    let swap_color = Color::Magenta;

    let ram_data: Vec<_> = ram
        .iter()
        .skip(90)
        .copied()
        .enumerate()
        .map(|(index, mem)| (index as f64, mem as f64))
        .collect();
    let swap_data: Vec<_> = swap
        .iter()
        .skip(90)
        .copied()
        .enumerate()
        .map(|(index, mem)| (index as f64, mem as f64))
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
    .x_axis(Axis::default().bounds([0.0, 30.0]).labels(["30s", "0s"]))
    .y_axis(
        Axis::default()
            .bounds([0.0, (1 << mem_peak_power_of_2) as f64])
            .labels(["0M".into(), format!("{}M", 1 << mem_peak_power_of_2)]),
    )
    .block(
        Block::bordered()
            .title_top("Memory")
            .border_type(BorderType::Plain)
            .border_style(Style::new().fg(Color::White)),
    );
    let ram_current_str = if current_stat.ram == -1f32 {
        "-".to_string()
    } else {
        format!("{:.2} MiB", current_stat.ram)
    };
    let ram_max_str = if current_stat.ram_max == -1 {
        "∞".to_string()
    } else {
        format!("{:.2} MiB", current_stat.ram_max)
    };
    let swap_current_str = if current_stat.swap == -1f32 {
        "-".to_string()
    } else {
        format!("{:.2} MiB", current_stat.swap)
    };
    let swap_max_str = if current_stat.swap_max == -1 {
        "∞".to_string()
    } else {
        format!("{:.2} MiB", current_stat.swap_max)
    };

    let ram_text = Text::raw(format!("RAM: {} / {}", ram_current_str, ram_max_str)).fg(ram_color);
    let swap_text =
        Text::raw(format!("SWP: {} / {}", swap_current_str, swap_max_str)).fg(swap_color);
    let mem_text_block = Block::bordered().border_style(Style::new().fg(Color::White));

    frame.render_widget(mem, area);
    area.x += 1;
    area.y += 1;
    area.height = 1;

    let cpu_text_width = max(ram_text.width() as u16, swap_text.width() as u16);
    let cpu_text_x = (area.x + area.width) - (2 + cpu_text_width) - 2;
    frame.render_widget(
        mem_text_block,
        Rect::new(cpu_text_x, area.y, 2 + cpu_text_width, 4),
    );
    frame.render_widget(
        ram_text,
        Rect::new(cpu_text_x + 1, area.y + 1, cpu_text_width, 1),
    );
    frame.render_widget(
        swap_text,
        Rect::new(cpu_text_x + 1, area.y + 2, cpu_text_width, 1),
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
        let r: &mut CrosstermBackendCopy<&mut SyncIoBridge<T>> =
            unsafe { mem::transmute(&mut self.0) };
        r.writer.inner_mut()
    }
}

#[allow(clippy::too_many_arguments)]
async fn read(
    state: Arc<Daemon>,
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
        let size = start as u8 + stop as u8 + restart as u8 + kill as u8;

        let mut block = false;
        if *selected >= size {
            block = true;
            while *selected >= size && size != 0 {
                *selected -= 1;
            }
        }
        if block {
            events.clear();
            return false;
        }

        // Start - stop - restart - kill
        for event in events.drain(..) {
            match event {
                Event::ArrowLeft if *selected > 0 => *selected -= 1,
                Event::ArrowRight if (*selected + 1) < size => *selected += 1,
                Event::Enter if *selected < size => {
                    launch_selected(
                        state.clone(),
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
                    for (_, index, _, enabled, area) in
                        calc_buttons(server_state.clone(), power, width)
                    {
                        if area.contains(Position::new(x, y)) && enabled {
                            launch_selected(
                                state.clone(),
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
    state: Arc<Daemon>,
    server_id: u32,
    selected: u8,
    (start, stop, restart, kill): (bool, bool, bool, bool),
) {
    // Determine the index of the buttons
    // (normal addition with overflow does result in a panic in debug builds)
    let start = if start { 0u8 } else { 255 };
    let stop = if stop {
        start.overflowing_add(1).0
    } else {
        255
    };
    let restart = if restart {
        max(start.overflowing_add(1).0, stop.overflowing_add(1).0)
    } else {
        255
    };
    let kill = if kill {
        max(
            max(start.overflowing_add(1).0, stop.overflowing_add(1).0),
            restart.overflowing_add(1).0,
        )
    } else {
        255
    };

    let (server, server_data) = match utils::server(&state.conn, &state.servers, server_id).await {
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
    let state = state.clone();
    tokio::task::spawn(async move {
        let server_data_arc = server_data.clone();
        let Ok(mut server_data) =
            tokio::time::timeout(Duration::from_millis(100), server_data.lock_owned()).await
        else {
            return;
        };

        if selected == start {
            let Ok(storage) = storage_read::<StorageError>(
                &state.config,
                state.conn.deref(),
                &state.storages,
                StorageIdentifier::Primary(server_id),
            )
            .await
            else {
                return;
            };
            if let Err(err) = server
                .start(
                    state.docker.clone(),
                    server_data.deref_mut(),
                    storage,
                    server_data_arc,
                    &state,
                )
                .await
            {
                warn!("Failed to start server using ssh (control): {:?}", err);
            }
        } else if selected == stop {
            if let Err(err) = server.stop(&state.docker, server_data).await {
                warn!("Failed to stop server using ssh (control): {:?}", err);
            }
        } else if selected == restart {
            if let Err(err) = server.restart(&state.docker, server_data).await {
                warn!("Failed to stop server using ssh (control): {:?}", err);
            }
        } else if selected == kill {
            if let Err(err) = server.kill(&state.docker, server_data).await {
                warn!("Failed to kill server using ssh (control): {:?}", err);
            }
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
        let data = &buf[0..size];
        for b in data {
            ring1.push(*b);
            ring2.push(*b);
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

                let button = (cb & 0b0000_0011) | ((cb & 0b1100_0000) >> 4); // 1 = left
                let dragging = cb & 0b0010_0000 == 0b0010_0000;

                let x = u16::from(cx.saturating_sub(32)) - 1;
                let y = u16::from(cy.saturating_sub(32)) - 1;

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
