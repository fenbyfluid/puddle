use anyhow::{Context, Result, anyhow};
use clap::Parser;
use linmot::mci::units::{Acceleration, Position, Velocity};
use linmot::mci::{Command, ControlFlags, ErrorCode, MotionCommand, State};
use linmot::udp::{BUFFER_SIZE, CONTROLLER_PORT, DRIVE_PORT, Request, Response, ResponseFlags};
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub mod linmot;
mod reader;
mod writer;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Options {
    /// Drive controller hostname or IP address
    drive_address: String,
    /// Loop interval in milliseconds
    #[clap(short, long, default_value = "5")]
    loop_interval: u64,
    /// Report interval in milliseconds
    #[clap(short, long, default_value = "1000")]
    report_interval: u64,
}

fn main() -> Result<()> {
    let options = Options::parse();

    let (stroke_params_sender, stroke_params_receiver) = mpsc::channel();

    std::thread::spawn(move || {
        run_input_loop(stroke_params_sender);
    });

    DriveConnection::new(&options.drive_address, stroke_params_receiver)?
        .start_loop(Duration::from_millis(options.loop_interval), Duration::from_millis(options.report_interval))
        .context("Failed to connect to drive")?;

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrokeMode {
    Uncontrolled,
    Stopped,
    Active,
}

#[derive(Debug, Clone)]
struct StrokeParams {
    mode: StrokeMode,
    start: Position,
    length: Position,
    direction_change_tolerance: Position,
    forwards_velocity: Velocity,
    forwards_acceleration: Acceleration,
    forwards_deceleration: Acceleration,
    backwards_velocity: Velocity,
    backwards_acceleration: Acceleration,
    backwards_deceleration: Acceleration,
}

impl StrokeParams {
    const fn new() -> Self {
        Self {
            mode: StrokeMode::Uncontrolled,
            start: Position::from_millimeters(0),
            length: Position::from_millimeters(0),
            direction_change_tolerance: Position::from_millimeters(1),
            forwards_velocity: Velocity::from_meters_per_second(1),
            forwards_acceleration: Acceleration::from_meters_per_second_squared(1),
            forwards_deceleration: Acceleration::from_meters_per_second_squared(1),
            backwards_velocity: Velocity::from_meters_per_second(1),
            backwards_acceleration: Acceleration::from_meters_per_second_squared(1),
            backwards_deceleration: Acceleration::from_meters_per_second_squared(1),
        }
    }
}

fn run_input_loop(stroke_params_sender: mpsc::Sender<StrokeParams>) {
    let mut input = String::new();
    let mut stroke_params = StrokeParams::new();

    loop {
        input.clear();
        std::io::stdin().read_line(&mut input).unwrap();

        let (command, value) = match input.split_once(' ') {
            Some((command, value)) => (command, value.trim_end().parse().ok()),
            None => (input.trim_end(), None),
        };

        match (command, value) {
            ("h", _) => {
                println!("Available commands:");
                println!("   p = Toggle power (hard stop)");
                println!("   f = Toggle soft stop");
                println!("   r = Reset parameters to default");
                println!("   s = Set stroke start position in mm");
                println!("   l = Set stroke length in mm");
                println!("   t = Set direction change tolerance in mm");
                println!("   v = Set velocity in m/s");
                println!("   a = Set acceleration in m/s²");
                println!("  fv = Set forwards velocity in m/s");
                println!("  fa = Set forwards acceleration in m/s²");
                println!("  fd = Set forwards deceleration in m/s²");
                println!("  bv = Set backwards velocity in m/s");
                println!("  ba = Set backwards acceleration in m/s²");
                println!("  bd = Set backwards deceleration in m/s²");
            }
            ("f", _) => {
                stroke_params.mode = match stroke_params.mode {
                    StrokeMode::Active => StrokeMode::Stopped,
                    StrokeMode::Stopped => StrokeMode::Active,
                    mode => mode,
                }
            }
            ("r", _) => stroke_params = StrokeParams { mode: stroke_params.mode, ..StrokeParams::new() },
            ("p", _) => {
                stroke_params.mode = match stroke_params.mode {
                    StrokeMode::Uncontrolled => StrokeMode::Active,
                    _ => StrokeMode::Uncontrolled,
                }
            }
            ("s", Some(v)) => stroke_params.start = Position::from_millimeters_f64(v),
            ("l", Some(v)) => stroke_params.length = Position::from_millimeters_f64(v),
            ("t", Some(v)) => stroke_params.direction_change_tolerance = Position::from_millimeters_f64(v),
            ("v", Some(v)) => {
                stroke_params.forwards_velocity = Velocity::from_meters_per_second_f64(v);
                stroke_params.backwards_velocity = stroke_params.forwards_velocity;
            }
            ("a", Some(v)) => {
                stroke_params.forwards_acceleration = Acceleration::from_meters_per_second_squared_f64(v);
                stroke_params.forwards_deceleration = stroke_params.forwards_acceleration;
                stroke_params.backwards_acceleration = stroke_params.forwards_acceleration;
                stroke_params.backwards_deceleration = stroke_params.backwards_acceleration;
            }
            ("fv", Some(v)) => stroke_params.forwards_velocity = Velocity::from_meters_per_second_f64(v),
            ("fa", Some(v)) => {
                stroke_params.forwards_acceleration = Acceleration::from_meters_per_second_squared_f64(v)
            }
            ("fd", Some(v)) => {
                stroke_params.forwards_deceleration = Acceleration::from_meters_per_second_squared_f64(v)
            }
            ("bv", Some(v)) => stroke_params.backwards_velocity = Velocity::from_meters_per_second_f64(v),
            ("ba", Some(v)) => {
                stroke_params.backwards_acceleration = Acceleration::from_meters_per_second_squared_f64(v)
            }
            ("bd", Some(v)) => {
                stroke_params.backwards_deceleration = Acceleration::from_meters_per_second_squared_f64(v)
            }
            _ => {
                println!("Unknown command or missing value, use 'h' for help");
                continue;
            }
        }

        stroke_params_sender.send(stroke_params.clone()).unwrap();
    }
}

struct DriveConnection {
    socket: UdpSocket,
    buffer: [u8; BUFFER_SIZE],
    last_response: Option<Response>,
    last_state: State,
    control_flags: ControlFlags,
    acknowledge_error: bool,
    moving_forwards: bool,
    stroke_params: StrokeParams,
    stroke_params_receiver: mpsc::Receiver<StrokeParams>,
}

impl DriveConnection {
    fn new(address: &str, stroke_params_receiver: mpsc::Receiver<StrokeParams>) -> Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, CONTROLLER_PORT))?;
        socket.connect((address, DRIVE_PORT))?;

        println!("Connected to drive at {:?} from {:?}", socket.peer_addr(), socket.local_addr());

        Ok(Self {
            socket,
            buffer: [0u8; BUFFER_SIZE],
            last_response: None,
            last_state: State::NotReadyToSwitchOn,
            control_flags: ControlFlags::empty(),
            acknowledge_error: true,
            moving_forwards: false,
            stroke_params: StrokeParams::new(),
            stroke_params_receiver,
        })
    }

    fn get_motion_command_for_stroke_params(
        params: &StrokeParams,
        moving_forwards: &mut bool,
        demand_position: &Position,
    ) -> Command {
        if *moving_forwards {
            if params.mode == StrokeMode::Stopped {
                return Command::VaiStop { deceleration: params.forwards_deceleration };
            }

            if *demand_position >= (params.start + params.length) - params.direction_change_tolerance {
                *moving_forwards = false;
            }

            Command::VaiGoToPos {
                target_position: params.start + params.length,
                maximal_velocity: params.forwards_velocity,
                acceleration: params.forwards_acceleration,
                deceleration: params.forwards_deceleration,
            }
        } else {
            if params.mode == StrokeMode::Stopped {
                return Command::VaiStop { deceleration: params.backwards_deceleration };
            }

            if *demand_position <= params.start + params.direction_change_tolerance {
                *moving_forwards = true;
            }

            Command::VaiGoToPos {
                target_position: params.start,
                maximal_velocity: params.backwards_velocity,
                acceleration: params.backwards_acceleration,
                deceleration: params.backwards_deceleration,
            }
        }
    }

    fn loop_tick(&mut self) -> Result<()> {
        // Check for new stroke parameters — keep the latest if multiple are pending
        while let Ok(new_params) = self.stroke_params_receiver.try_recv() {
            self.stroke_params = new_params;
        }

        let mut request = Request {
            response_flags: ResponseFlags::STATUS_FLAGS
                | ResponseFlags::STATE
                | ResponseFlags::ACTUAL_POSITION
                | ResponseFlags::DEMAND_POSITION
                | ResponseFlags::CURRENT
                | ResponseFlags::WARNING_FLAGS
                | ResponseFlags::ERROR_CODE,
            ..Default::default()
        };

        // TODO: We currently have several control bits forced in the parameter configuration,
        //       re-evaluate if we want to implement the full state machine instead.
        if let Some(Response { state: Some(state), demand_position: Some(demand_position), .. }) = &self.last_response {
            if state != &self.last_state {
                // TODO: Figure out how to ignore OperationEnabled->OperationEnabled transitions if only motion_command_count changed.
                // println!("Transitioned from {:?} to {:?}", self.last_state, state);
                self.last_state = *state;
            }

            match state {
                State::NotReadyToSwitchOn => {
                    self.control_flags = ControlFlags::empty();
                }
                State::ReadyToSwitchOn => {
                    // We only acknowledge an error once on startup to get the drive into a stable state.
                    // Require user confirmation before acknowledging any drive errors during operation.
                    self.acknowledge_error = false;

                    if self.stroke_params.mode != StrokeMode::Uncontrolled {
                        self.control_flags = ControlFlags::SWITCH_ON;
                    }
                }
                State::Error { error_code } if self.acknowledge_error => {
                    println!("Acknowledging error: {error_code:?}");

                    self.control_flags = ControlFlags::ERROR_ACKNOWLEDGE;
                }
                State::OperationEnabled { homed: false, .. } => {
                    self.control_flags.insert(ControlFlags::HOME);
                }
                State::OperationEnabled { homed: true, motion_command_count, .. } => {
                    let next_command_count = (motion_command_count.wrapping_add(1)) & 0xF;

                    if self.stroke_params.mode == StrokeMode::Uncontrolled {
                        self.control_flags.remove(ControlFlags::SWITCH_ON);
                    } else {
                        let command = Self::get_motion_command_for_stroke_params(
                            &self.stroke_params,
                            &mut self.moving_forwards,
                            demand_position,
                        );

                        request.motion_command = Some(MotionCommand { count: next_command_count, command });
                    }
                }
                State::Homing { finished: true } => {
                    self.control_flags.remove(ControlFlags::HOME);
                }
                _ => {}
            }
        }

        request.control_flags = Some(self.control_flags);

        let to_send = request.to_wire(&mut self.buffer).context("Failed to serialize request")?;

        self.socket.send(&self.buffer[..to_send])?;

        let received = self.socket.recv(&mut self.buffer)?;

        // TODO: Extend this error type to include the raw bytes that were received
        let response = Response::from_wire(&self.buffer[..received])?;

        self.last_response = Some(response);

        Ok(())
    }

    fn start_loop(&mut self, loop_interval: Duration, report_interval: Duration) -> Result<()> {
        self.socket.set_read_timeout(Some(loop_interval / 2))?;

        let mut last_loop_report = Instant::now();
        let mut loop_duration_sum = Duration::ZERO;
        let mut loop_duration_min = Duration::MAX;
        let mut loop_duration_max = Duration::ZERO;
        let mut loop_message_count: usize = 0;
        let mut loop_error_history = Vec::new();

        let mut next_tick = Instant::now() + loop_interval;

        loop {
            let iter_start = Instant::now();

            if let Err(error) = self.loop_tick() {
                // TODO: Print the error if it's not just a read timeout
                loop_error_history.push(error);
            }

            loop_message_count += 1;

            let loop_duration = iter_start.elapsed();
            loop_duration_sum += loop_duration;
            loop_duration_min = loop_duration_min.min(loop_duration);
            loop_duration_max = loop_duration_max.max(loop_duration);

            if last_loop_report.elapsed() >= report_interval {
                println!();

                // TODO: Print the error history in a compact format
                let avg_loop_duration = loop_duration_sum / (loop_message_count as u32);
                println!(
                    "Timing statistics: {:?} average, {:?} min, {:?} max, {:.2}% usage ({:.2}% peak), {}/{} errors",
                    avg_loop_duration,
                    loop_duration_min,
                    loop_duration_max,
                    (avg_loop_duration.as_secs_f64() / loop_interval.as_secs_f64()) * 100.0,
                    (loop_duration_max.as_secs_f64() / loop_interval.as_secs_f64()) * 100.0,
                    loop_error_history.len(),
                    loop_message_count,
                );

                self.print_drive_status();

                println!("{:#?}", self.stroke_params);

                if !loop_error_history.is_empty() && loop_error_history.len() == loop_message_count {
                    break Err(anyhow!("Too many errors in loop, aborting"));
                }

                last_loop_report = Instant::now();
                loop_duration_sum = Duration::ZERO;
                loop_duration_min = Duration::MAX;
                loop_duration_max = Duration::ZERO;
                loop_message_count = 0;
                loop_error_history.clear();
            }

            // Sleep until the next tick; if overrun, report lateness and realign to the next interval boundary
            let now = Instant::now();
            if let Some(remaining) = next_tick.checked_duration_since(now) {
                std::thread::sleep(remaining);
                next_tick += loop_interval;
            } else {
                let late_by = now.duration_since(next_tick);
                eprintln!("Late by {late_by:?}");
                next_tick = now + loop_interval;
            }
        }
    }

    fn print_drive_status(&self) {
        let Some(response) = &self.last_response else {
            return;
        };

        if let (Some(status_flags), Some(state)) = (&response.status_flags, &response.state) {
            println!("State: {state:?}, Status: {status_flags:?}");
        }

        if let (Some(actual_position), Some(demand_position), Some(current)) =
            (&response.actual_position, &response.demand_position, &response.current)
        {
            println!("Actual Pos.: {actual_position:?}, Desired Pos.: {demand_position:?}, Current: {current:?}");
        }

        if let (Some(warning_flags), Some(error_code)) = (&response.warning_flags, &response.error_code)
            && (!warning_flags.is_empty() || *error_code != ErrorCode::NoError)
        {
            println!("Warnings: {warning_flags:?}, Error: {error_code:?}");
        }
    }
}
