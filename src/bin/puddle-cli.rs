// DISCLAIMER:
// This is an example / test of the Puddle WebSocket API.
// This code was LLM-generated.

use anyhow::Result;
use clap::Parser;
use puddle::messages::{
    ClientMessage, CommandUpdate, CoreMessage, DriveState, MotionAction, MotionCommand, MotionCommandFields,
};
use puddle::units::{Acceleration, Position, Velocity};
use puddle::{ControllerId, CoreState, SystemLimits};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, LineGauge, Paragraph};
use signal_hook::consts::signal::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::{Message, connect};

#[derive(Parser, Clone, Debug)]
#[command(version, about, long_about = None)]
struct Options {
    /// WebSocket server port to connect to
    #[clap(short = 'p', long, default_value = "8080")]
    websocket_port: u16,
    /// WebSocket server host
    #[clap(short = 'H', long, default_value = "localhost")]
    websocket_host: String,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let options = Options::parse();

    let mut terminal = ratatui::init();
    let app_result = App::new(options).run(&mut terminal);
    ratatui::restore();

    app_result
}

struct App {
    inputs: [i32; 8],
    input_index: usize,
    limits: Option<SystemLimits>,
    state: Option<CoreState>,
    my_id: Option<ControllerId>,
    exit: Arc<AtomicBool>,
    tx: mpsc::Sender<Option<ClientMessage>>,
    rx: mpsc::Receiver<CoreMessage>,
    seq: u64,
    last_key: Option<(KeyCode, Instant)>,
    repeat_count: u32,
    ws_handle: Option<thread::JoinHandle<()>>,
}

impl App {
    pub fn new(options: Options) -> Self {
        let (tx_to_ws, rx_from_ui) = mpsc::channel();
        let (tx_to_ui, rx_from_ws) = mpsc::channel::<CoreMessage>();

        let ws_url = format!("ws://{}:{}/", options.websocket_host, options.websocket_port);
        let ws_handle = thread::spawn(move || {
            let mut ws = match connect(ws_url) {
                Ok((ws, _)) => ws,
                Err(e) => {
                    log::error!("Failed to connect to WebSocket: {}", e);
                    return;
                }
            };

            match ws.get_mut() {
                tungstenite::stream::MaybeTlsStream::Plain(s) => s.set_nonblocking(true).ok(),
                _ => None,
            };

            'outer: loop {
                // Read from WS
                loop {
                    match ws.read() {
                        Ok(Message::Text(text)) => {
                            if let Ok(msg) = serde_json::from_str::<CoreMessage>(&text) {
                                tx_to_ui.send(msg).ok();
                            }
                        }
                        Ok(Message::Binary(bin)) => {
                            if let Ok(msg) = serde_json::from_slice::<CoreMessage>(&bin) {
                                tx_to_ui.send(msg).ok();
                            }
                        }
                        Ok(Message::Close(_)) => {
                            break 'outer;
                        }
                        Err(tungstenite::Error::Io(ref e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            break;
                        }
                        Err(e) => {
                            log::error!("WS read error: {}", e);
                            break 'outer;
                        }
                        _ => {}
                    }
                }

                // Read from UI
                while let Ok(msg) = rx_from_ui.try_recv() {
                    match msg {
                        Some(msg) => {
                            let text = serde_json::to_string(&msg).unwrap();
                            if let Err(e) = ws.send(Message::Text(text.into())) {
                                log::error!("WS send error: {}", e);
                                break 'outer;
                            }
                        }
                        None => {
                            ws.close(None).ok();
                            // Wait a bit for close to flush or just exit
                            let _ = ws.flush();
                            break 'outer;
                        }
                    }
                }

                thread::sleep(Duration::from_millis(10));
            }
        });

        let exit = Arc::new(AtomicBool::new(false));
        if let Err(e) = signal_hook::flag::register(SIGINT, Arc::clone(&exit)) {
            log::error!("Failed to register SIGINT: {}", e);
        }
        if let Err(e) = signal_hook::flag::register(SIGTERM, Arc::clone(&exit)) {
            log::error!("Failed to register SIGTERM: {}", e);
        }

        Self {
            inputs: [0; 8],
            input_index: 0,
            limits: None,
            state: None,
            my_id: None,
            exit,
            tx: tx_to_ws,
            rx: rx_from_ws,
            seq: 1,
            last_key: None,
            repeat_count: 0,
            ws_handle: Some(ws_handle),
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.exit.load(Ordering::SeqCst) {
            while let Ok(msg) = self.rx.try_recv() {
                self.handle_core_message(msg)?;
            }

            terminal.draw(|frame| self.draw(frame))?;

            match event::poll(Duration::from_millis(10)) {
                Ok(true) => {
                    if let Event::Key(key_event) = event::read()? {
                        if key_event.kind == KeyEventKind::Press || key_event.kind == KeyEventKind::Repeat {
                            self.handle_key_event(key_event)?;
                        }
                    }
                }
                Ok(false) => {}
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    // Signal received, loop will check exit flag next
                }
                Err(e) => return Err(e.into()),
            }
        }
        // Cleanup WS
        self.tx.send(None).ok();
        if let Some(handle) = self.ws_handle.take() {
            let _ = handle.join();
        }
        Ok(())
    }

    fn handle_core_message(&mut self, msg: CoreMessage) -> Result<()> {
        match msg {
            CoreMessage::Connected { controller_id, limits, state } => {
                self.my_id = Some(controller_id);
                self.limits = Some(limits);
                self.state = Some(state);
            }
            CoreMessage::State { state, .. } => {
                self.state = Some(state);
            }
            CoreMessage::WriteAccessChanged { holder, .. } => {
                if let Some(state) = &mut self.state {
                    state.write_access_holder = holder;
                }
            }
            CoreMessage::WriteAccessResult { granted, holder, .. } => {
                if let Some(state) = &mut self.state {
                    state.write_access_holder = holder;
                }
                if granted {
                    // Send 2 commands to the active set
                    let cmd = MotionCommand {
                        position: Position(self.inputs[0]),
                        velocity: Velocity(self.inputs[1]),
                        acceleration: Acceleration(self.inputs[2]),
                        deceleration: Acceleration(self.inputs[3]),
                    };
                    let cmd2 = MotionCommand {
                        position: Position(self.inputs[4]),
                        velocity: Velocity(self.inputs[5]),
                        acceleration: Acceleration(self.inputs[6]),
                        deceleration: Acceleration(self.inputs[7]),
                    };
                    let seq = self.next_seq();
                    self.send(ClientMessage::UpsertCommandSet {
                        seq,
                        set: None,
                        base_version: None,
                        commands: vec![cmd, cmd2],
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn next_seq(&mut self) -> u64 {
        let s = self.seq;
        self.seq += 1;
        s
    }

    fn send(&self, msg: ClientMessage) {
        self.tx.send(Some(msg)).ok();
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) -> Result<()> {
        let now = Instant::now();
        let _is_repeat = if let Some((last_code, last_time)) = self.last_key {
            if last_code == key_event.code && now.duration_since(last_time) < Duration::from_millis(150) {
                self.repeat_count += 1;
                true
            } else {
                self.repeat_count = 0;
                false
            }
        } else {
            self.repeat_count = 0;
            false
        };
        self.last_key = Some((key_event.code, now));

        let large_step = self.repeat_count > 5 || key_event.kind == KeyEventKind::Repeat;

        let has_write_access = if let (Some(state), Some(my_id)) = (&self.state, &self.my_id) {
            state.write_access_holder == Some(*my_id)
        } else {
            false
        };

        match key_event.code {
            KeyCode::Char('q') => self.exit.store(true, Ordering::SeqCst),
            KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.exit.store(true, Ordering::SeqCst);
            }
            KeyCode::Up => {
                if self.input_index > 0 {
                    self.input_index -= 1;
                }
            }
            KeyCode::Down => {
                if self.input_index < 7 {
                    self.input_index += 1;
                }
            }
            KeyCode::Left => {
                if has_write_access {
                    self.update_input(-1, large_step);
                }
            }
            KeyCode::Right => {
                if has_write_access {
                    self.update_input(1, large_step);
                }
            }
            KeyCode::Char('w') => {
                let seq = self.next_seq();
                self.send(ClientMessage::RequestWriteAccess { seq });
            }
            KeyCode::Char('p') => {
                if has_write_access {
                    if let Some(state) = &self.state {
                        let enabled = state.drive_state == DriveState::PowerOff;
                        let seq = self.next_seq();
                        self.send(ClientMessage::SetDrivePower { seq, enabled });
                    }
                }
            }
            KeyCode::Char('m') => {
                if has_write_access {
                    if let Some(state) = &self.state {
                        let action = match state.drive_state {
                            DriveState::Moving => MotionAction::Pause,
                            DriveState::Paused => MotionAction::Resume,
                            _ => MotionAction::Start,
                        };
                        let seq = self.next_seq();
                        self.send(ClientMessage::SetMotionState { seq, action });
                    }
                }
            }
            KeyCode::Char('a') => {
                if has_write_access {
                    let seq = self.next_seq();
                    self.send(ClientMessage::AcknowledgeError { seq });
                }
            }
            KeyCode::Char('r') => {
                if has_write_access {
                    self.reset_inputs();
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn reset_inputs(&mut self) {
        self.inputs = [0; 8];

        // Send update to server if we have write access
        if let Some(state) = &self.state {
            if state.write_access_holder.is_some() {
                let cmd = MotionCommand {
                    position: Position(0),
                    velocity: Velocity(0),
                    acceleration: Acceleration(0),
                    deceleration: Acceleration(0),
                };
                let seq = self.next_seq();
                self.send(ClientMessage::UpsertCommandSet {
                    seq,
                    set: None,
                    base_version: None,
                    commands: vec![cmd.clone(), cmd],
                });
            }
        }
    }

    fn update_input(&mut self, delta: i32, large: bool) {
        // Double check write access just in case
        if let (Some(state), Some(my_id)) = (&self.state, &self.my_id) {
            if state.write_access_holder != Some(*my_id) {
                return;
            }
        } else {
            return;
        }

        let limit = if let Some(limits) = &self.limits {
            match self.input_index % 4 {
                0 => limits.position.0,
                1 => limits.velocity.0,
                2 => limits.acceleration.0,
                3 => limits.deceleration.0,
                _ => unreachable!(),
            }
        } else {
            i32::MAX
        };

        // Refined steps: Position (0.1mm vs 1mm), Velocity (1mm/s vs 10mm/s), Accel (0.1m/s^2 vs 1m/s^2)
        let step = match self.input_index % 4 {
            0 => {
                if large {
                    10_000
                } else {
                    1_000
                }
            }
            1 => {
                if large {
                    10_000
                } else {
                    1_000
                }
            }
            2 => {
                if large {
                    10_000
                } else {
                    1_000
                }
            }
            3 => {
                if large {
                    10_000
                } else {
                    1_000
                }
            }
            _ => unreachable!(),
        };

        self.inputs[self.input_index] = (self.inputs[self.input_index] + delta * step).clamp(0, limit);

        // Relative checks: cmd0 > cmd1 should push cmd1 up to match, and vice-versa for decreasing cmd1 < cmd0.
        // cmd0 is index 0, cmd1 is index 4.
        let mut affected_indices = vec![self.input_index];
        if self.input_index == 0 && self.inputs[0] > self.inputs[4] {
            self.inputs[4] = self.inputs[0];
            affected_indices.push(4);
        } else if self.input_index == 4 && self.inputs[4] < self.inputs[0] {
            self.inputs[0] = self.inputs[4];
            affected_indices.push(0);
        }

        // Send updates to server if we have write access
        if let Some(state) = &self.state {
            if state.write_access_holder.is_some() {
                for &idx in &affected_indices {
                    let mut fields = MotionCommandFields::default();
                    match idx % 4 {
                        0 => fields.position = Some(Position(self.inputs[idx])),
                        1 => fields.velocity = Some(Velocity(self.inputs[idx])),
                        2 => fields.acceleration = Some(Acceleration(self.inputs[idx])),
                        3 => fields.deceleration = Some(Acceleration(self.inputs[idx])),
                        _ => unreachable!(),
                    }

                    let seq = self.next_seq();
                    self.send(ClientMessage::UpdateCommand { seq, update: CommandUpdate { index: idx / 4, fields } });
                }
            }
        }
    }

    fn draw(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(0), Constraint::Length(60)])
            .split(frame.area());

        self.draw_left(frame, chunks[0]);
        self.draw_right(frame, chunks[1]);
    }

    fn draw_left(&self, frame: &mut Frame, area: Rect) {
        let mut status_height = 0;
        if let Some(state) = &self.state {
            if !state.warnings.is_empty() || state.error_code.is_some() {
                status_height = 4;
            }
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(status_height)])
            .split(area);

        self.draw_inputs(frame, chunks[0]);
        if status_height > 0 {
            self.draw_status(frame, chunks[1]);
        }
    }

    fn draw_inputs(&self, frame: &mut Frame, area: Rect) {
        let labels = [
            "Start Position",
            "Start Velocity",
            "Start Acceleration",
            "Start Deceleration",
            "  End Position",
            "  End Velocity",
            "  End Acceleration",
            "  End Deceleration",
        ];

        let block = Block::bordered().title(" Inputs (Up/Down Select, Left/Right Adjust) ");
        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        let input_height = if inner_area.height >= 16 { 2 } else { 1 };
        let constraints = vec![Constraint::Length(input_height); 8];
        let chunks = Layout::default().direction(Direction::Vertical).constraints(constraints).split(inner_area);

        for i in 0..8 {
            let is_selected = i == self.input_index;
            let label_prefix = if is_selected { "▶ " } else { "  " };

            let (filled_style, unfilled_style) = if is_selected {
                (
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    Style::default().fg(Color::Rgb(64, 64, 0)), // Dim yellow for track
                )
            } else {
                (
                    Style::default().fg(Color::Cyan),
                    Style::default().fg(Color::Rgb(0, 32, 32)), // Dim cyan for track
                )
            };

            let (limit_info, val_str) = if let Some(limits) = &self.limits {
                let limit = match i % 4 {
                    0 => limits.position.0,
                    1 => limits.velocity.0,
                    2 => limits.acceleration.0,
                    3 => limits.deceleration.0,
                    _ => unreachable!(),
                };

                let ratio = if limit > 0 { (self.inputs[i] as f64 / limit as f64).clamp(0.0, 1.0) } else { 0.0 };

                let val_str = match i % 4 {
                    0 => format!("{:?}", Position(self.inputs[i])),
                    1 => format!("{:?}", Velocity(self.inputs[i])),
                    2 => format!("{:?}", Acceleration(self.inputs[i])),
                    3 => format!("{:?}", Acceleration(self.inputs[i])),
                    _ => unreachable!(),
                };

                let limit_str = match i % 4 {
                    0 => format!("{:?}", Position(limit)),
                    1 => format!("{:?}", Velocity(limit)),
                    2 => format!("{:?}", Acceleration(limit)),
                    3 => format!("{:?}", Acceleration(limit)),
                    _ => unreachable!(),
                };

                (Some((ratio, limit_str)), val_str)
            } else {
                (None, format!("{}", self.inputs[i]))
            };

            let label_text = format!("{}{}", label_prefix, labels[i]);

            if input_height == 1 {
                // 1-line mode: [Label (22)] [Value (12)] [Gauge (flexible)] [Limit (12)]
                let row_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Length(22),
                        Constraint::Length(12),
                        Constraint::Min(0),
                        Constraint::Length(12),
                    ])
                    .split(chunks[i]);

                frame.render_widget(Paragraph::new(label_text).style(filled_style), row_chunks[0]);
                frame.render_widget(Paragraph::new(val_str).style(filled_style), row_chunks[1]);

                if let Some((ratio, limit_str)) = limit_info {
                    let gauge = LineGauge::default()
                        .filled_style(filled_style)
                        .unfilled_style(unfilled_style)
                        .ratio(ratio)
                        .line_set(symbols::line::THICK);
                    frame.render_widget(gauge, row_chunks[2]);
                    frame.render_widget(Paragraph::new(limit_str).alignment(Alignment::Right), row_chunks[3]);
                }
            } else {
                // 2-line mode
                let lines = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(1), Constraint::Length(1)])
                    .split(chunks[i]);

                let l1_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Length(22), Constraint::Min(0)])
                    .split(lines[0]);

                frame.render_widget(Paragraph::new(label_text).style(filled_style), l1_chunks[0]);
                frame.render_widget(Paragraph::new(val_str).style(filled_style), l1_chunks[1]);

                if let Some((ratio, limit_str)) = limit_info {
                    let l2_chunks = Layout::default()
                        .direction(Direction::Horizontal)
                        .constraints([Constraint::Min(0), Constraint::Length(12)])
                        .split(lines[1]);

                    let gauge = LineGauge::default()
                        .filled_style(filled_style)
                        .unfilled_style(unfilled_style)
                        .ratio(ratio)
                        .line_set(symbols::line::THICK);
                    frame.render_widget(gauge, l2_chunks[0]);
                    frame.render_widget(Paragraph::new(limit_str).alignment(Alignment::Right), l2_chunks[1]);
                }
            }
        }
    }

    fn draw_status(&self, frame: &mut Frame, area: Rect) {
        if let Some(state) = &self.state {
            let mut status_lines = Vec::new();
            if let Some(error) = &state.error_code {
                status_lines.push(Line::from(vec![
                    Span::styled("ERROR: ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                    Span::styled(error, Style::default().fg(Color::Red)),
                ]));
            }
            if !state.warnings.is_empty() {
                status_lines.push(Line::from(vec![
                    Span::styled("WARNINGS: ", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                    Span::styled(state.warnings.join(", "), Style::default().fg(Color::Yellow)),
                ]));
            }

            if !status_lines.is_empty() {
                let block = Block::bordered().title(" Status ");
                let paragraph = Paragraph::new(status_lines).block(block).wrap(ratatui::widgets::Wrap { trim: true });
                frame.render_widget(paragraph, area);
            }
        }
    }

    fn draw_right(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered().title(" Drive State ");
        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        if let Some(state) = &self.state {
            let holder = match state.write_access_holder {
                Some(h) => {
                    let mut s = h.to_string();
                    if Some(h) == self.my_id {
                        s.push_str(" (Me)");
                    }
                    s
                }
                None => "None".to_string(),
            };

            let col1_text = format!(
                "  Drive State: {:?}\n   Actual Pos: {:?}\n   Demand Pos: {:?}\n     Velocity: {:?}\n        Accel: {:?}",
                state.drive_state,
                state.actual_position,
                state.demand_position,
                state.demand_velocity,
                state.demand_acceleration
            );

            let col2_text = format!(
                "  Active Cmd: {}\n     Current: {:?}\n  Drive Temp: {:?}\n  Motor Temp: {:?}\n      Holder: {}",
                state.active_command_index,
                state.current_draw,
                state.drive_temperature,
                state.motor_temperature,
                holder
            );

            let controls_text = format!(
                " [W] Write Access   [P] Toggle Power   [R] Reset Inputs\n \
                 [M] Motion Toggle  [A] Ack Error      [Q] Quit"
            );

            if inner_area.width > 50 {
                let main_chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Min(0), Constraint::Length(4)])
                    .split(inner_area);

                let telemetry_chunks = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(main_chunks[0]);

                frame.render_widget(Paragraph::new(col1_text), telemetry_chunks[0]);
                frame.render_widget(Paragraph::new(col2_text), telemetry_chunks[1]);
                frame.render_widget(
                    Paragraph::new(controls_text).block(Block::default().borders(Borders::TOP)),
                    main_chunks[1],
                );
            } else {
                let state_text = format!("{}\n{}\n\n{}", col1_text, col2_text, controls_text);
                frame.render_widget(Paragraph::new(state_text).wrap(ratatui::widgets::Wrap { trim: true }), inner_area);
            }
        } else {
            frame.render_widget(Paragraph::new("Connecting..."), inner_area);
        }
    }
}
