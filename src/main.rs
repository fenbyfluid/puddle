use crate::reader::{Reader, WireRead};
use anyhow::{Context, Result, anyhow};
use clap::Parser;
use linmot::mci::units::{Acceleration, Position, Velocity};
use linmot::mci::{Command, ControlFlags, ErrorCode, MotionCommand, State};
use linmot::udp::{BUFFER_SIZE, CONTROLLER_PORT, DRIVE_PORT, Request, Response, ResponseFlags};
use questdb::ingress::{Buffer, Sender, TimestampMicros};
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub mod linmot;
mod reader;
mod writer;

fn from_hex(s: &str) -> Result<u16> {
    u16::from_str_radix(s, 16).with_context(|| format!("Invalid hex value: {}", s))
}

#[derive(Parser, Clone, Debug)]
#[command(version, about, long_about = None)]
struct Options {
    /// Drive controller hostname or IP address
    drive_address: String,
    /// Connect to USB remote controller
    #[clap(short = 'u', long)]
    enable_usb: bool,
    /// USB remote controller VID
    #[clap(long, value_parser=from_hex, default_value = "303A")]
    usb_vid: u16,
    // TODO: Eventually, use our custom PID
    /// USB remote controller PID
    #[clap(short = 'p', long, value_parser=from_hex, default_value = "4005")]
    usb_pid: u16,
    /// Stroke Limit
    #[clap(short, long, default_value = "360.0")]
    stroke_limit: f64,
    /// Velocity Limit
    #[clap(short, long, default_value = "2.5")]
    velocity_limit: f64,
    /// Acceleration Limit
    #[clap(short, long, default_value = "15.0")]
    acceleration_limit: f64,
    /// Loop interval in milliseconds
    #[clap(short, long, default_value = "5")]
    loop_interval: u64,
    /// Report interval in milliseconds
    #[clap(short, long, default_value = "10000")]
    report_interval: u64,
}

fn main() -> Result<()> {
    let options = Options::parse();

    let stroke_params = Arc::new(Mutex::new(StrokeParams::new()));
    let last_response = Arc::new(Mutex::new(None));

    #[cfg(feature = "hidapi")]
    if options.enable_usb {
        let options = options.clone();
        let stroke_params = stroke_params.clone();
        let last_response = last_response.clone();
        std::thread::spawn(move || {
            run_hidapi_loop(&options, stroke_params, last_response).unwrap();
        });
    }

    {
        let stroke_params = stroke_params.clone();
        std::thread::spawn(move || {
            run_input_loop(stroke_params);
        });
    }

    loop {
        let stroke_params = stroke_params.clone();
        let last_response = last_response.clone();

        let mut connection = match DriveConnection::new(&options, stroke_params, last_response) {
            Ok(connection) => connection,
            Err(e) => {
                eprintln!("Failed to connect to drive: {e}");
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        if let Err(e) = connection.start_loop() {
            eprintln!("Drive connection failed: {e}");
        }

        std::thread::sleep(Duration::from_secs(1));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StrokeParams {
    enabled: bool,
    stopped: bool,
    start: Position,
    end: Position,
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
            enabled: false,
            stopped: false,
            start: Position::from_millimeters(0),
            end: Position::from_millimeters(0),
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

// TODO: This is for the USB HID API, consider implementing a more specific trait.
impl WireRead for StrokeParams {
    fn read_from(r: &mut Reader) -> Result<Self> {
        let flags = r.read_u8()?;

        Ok(Self {
            enabled: (flags & 0x01) != 0,
            stopped: (flags & 0x02) != 0,
            start: Position::read_from(r)?,
            end: Position::read_from(r)?,
            direction_change_tolerance: Position::read_from(r)?,
            forwards_velocity: Velocity::read_from(r)?,
            forwards_acceleration: Acceleration::read_from(r)?,
            forwards_deceleration: Acceleration::read_from(r)?,
            backwards_velocity: Velocity::read_from(r)?,
            backwards_acceleration: Acceleration::read_from(r)?,
            backwards_deceleration: Acceleration::read_from(r)?,
        })
    }
}

#[cfg(feature = "hidapi")]
fn run_hidapi_loop(
    options: &Options,
    stroke_params: Arc<Mutex<StrokeParams>>,
    last_response: Arc<Mutex<Option<Response>>>,
) -> Result<()> {
    use hidapi::HidApi;

    let mut api = HidApi::new()?;

    loop {
        api.reset_devices()?;
        api.add_devices(options.usb_vid, options.usb_pid)?;

        if !api.device_list().any(|d| d.vendor_id() == options.usb_vid && d.product_id() == options.usb_pid) {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }

        if let Err(r) = run_hid_device_loop(&api, options, stroke_params.clone(), last_response.clone()) {
            println!("Error in USB HID device loop: {}", r);
            std::thread::sleep(Duration::from_secs(1));
            continue;
        }
    }
}

#[cfg(feature = "hidapi")]
fn run_hid_device_loop(
    api: &hidapi::HidApi,
    options: &Options,
    stroke_params: Arc<Mutex<StrokeParams>>,
    last_response: Arc<Mutex<Option<Response>>>,
) -> Result<()> {
    let device = api.open(options.usb_vid, options.usb_pid)?;

    println!("USB device connected: {:#?}", &device);

    let feature_report: Vec<u8> = [0x01]
        .into_iter()
        .chain(Position::from_millimeters_f64(options.stroke_limit).0.to_le_bytes())
        .chain(Velocity::from_meters_per_second_f64(options.velocity_limit).0.to_le_bytes())
        .chain(Acceleration::from_meters_per_second_squared_f64(options.acceleration_limit).0.to_le_bytes())
        .collect();

    device.send_feature_report(&feature_report)?;

    let mut buffer = [0u8; 64];

    loop {
        let response_report: Vec<u8> = {
            let last_response = last_response.lock().unwrap();

            match last_response.as_ref() {
                Some(Response {
                    status_flags: Some(status_flags),
                    state: Some(state),
                    actual_position: Some(actual_position),
                    demand_position: Some(demand_position),
                    current: Some(current),
                    warning_flags: Some(warning_flags),
                    error_code: Some(error_code),
                    ..
                }) => [0x01, 0x01]
                    .into_iter()
                    .chain(status_flags.bits().to_le_bytes())
                    .chain(u16::to_le_bytes((*state).into()))
                    .chain(actual_position.0.to_le_bytes())
                    .chain(demand_position.0.to_le_bytes())
                    .chain(current.0.to_le_bytes())
                    .chain(warning_flags.bits().to_le_bytes())
                    .chain(u16::to_le_bytes((*error_code).into()))
                    .collect(),
                _ => {
                    vec![0x01, 0x00]
                }
            }
        };

        device.write(&response_report)?;

        // Use a timeout so we write the params even if we're not getting any data.
        let read_count = device.read_timeout(&mut buffer, 1000)?;
        if read_count == 0 {
            continue;
        }

        // println!("USB HID: {:02x?}", &buffer[..read_count]);

        let mut reader = Reader::new(&buffer);

        let report_id = reader.read_u8()?;
        if report_id != 1 {
            continue;
        }

        let new_stroke_params = StrokeParams::read_from(&mut reader)?;

        {
            let mut stroke_params = stroke_params.lock().unwrap();

            if *stroke_params == new_stroke_params {
                continue;
            }

            // println!("{:#?}", new_stroke_params);

            // TODO: We may also want to sync the local params back to the USB device.
            *stroke_params = new_stroke_params;
        }
    }
}

fn run_input_loop(stroke_params: Arc<Mutex<StrokeParams>>) {
    let mut input = String::new();

    loop {
        input.clear();
        let bytes_read = std::io::stdin().read_line(&mut input).unwrap();
        if bytes_read == 0 {
            // If no bytes were read, we've hit EOF.
            break;
        }

        let (command, value) = match input.split_once(' ') {
            Some((command, value)) => (command, value.trim_end().parse().ok()),
            None => (input.trim_end(), None),
        };

        let mut stroke_params = stroke_params.lock().unwrap();

        match (command, value) {
            ("h", _) => {
                println!("Available commands:");
                println!("   p = Toggle power (hard stop)");
                println!("   f = Toggle soft stop");
                println!("   r = Reset parameters to default");
                println!("   s = Set stroke start position in mm");
                println!("   e = Set stroke start position in mm");
                println!("  sl = Set stroke length in mm");
                println!("  el = Set stroke length in mm");
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
            ("p", _) => {
                stroke_params.enabled = !stroke_params.enabled;
            }
            ("f", _) => {
                stroke_params.stopped = !stroke_params.stopped;
            }
            ("r", _) => {
                *stroke_params = StrokeParams {
                    enabled: stroke_params.enabled,
                    stopped: stroke_params.stopped,
                    ..StrokeParams::new()
                }
            }
            ("s", Some(v)) => stroke_params.start = Position::from_millimeters_f64(v),
            ("e", Some(v)) => stroke_params.end = Position::from_millimeters_f64(v),
            ("sl", Some(v)) => stroke_params.end = stroke_params.start + Position::from_millimeters_f64(v),
            ("el", Some(v)) => stroke_params.start = stroke_params.end - Position::from_millimeters_f64(v),
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

        println!("{:#?}", *stroke_params);
    }
}

struct StrokeLimits {
    stroke_limit: Position,
    velocity_limit: Velocity,
    acceleration_limit: Acceleration,
}

struct DriveConnection {
    socket: UdpSocket,
    loop_interval: Duration,
    report_interval: Duration,
    buffer: [u8; BUFFER_SIZE],
    last_response: Arc<Mutex<Option<Response>>>,
    last_state: State,
    control_flags: ControlFlags,
    acknowledge_error: bool,
    moving_forwards: bool,
    in_range: bool,
    stroke_limits: StrokeLimits,
    stroke_params: Arc<Mutex<StrokeParams>>,
    #[cfg(feature = "questdb-rs")]
    metrics: Option<DriveMetrics>,
}

impl DriveConnection {
    fn new(
        options: &Options,
        stroke_params: Arc<Mutex<StrokeParams>>,
        last_response: Arc<Mutex<Option<Response>>>,
    ) -> Result<Self> {
        println!("Connecting to drive at {}:{}...", options.drive_address, DRIVE_PORT);

        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, CONTROLLER_PORT))?;

        socket.connect((options.drive_address.as_str(), DRIVE_PORT))?;

        let loop_interval = Duration::from_millis(options.loop_interval);
        socket.set_read_timeout(Some(loop_interval * 10))?;

        let mut buffer = [0u8; BUFFER_SIZE];

        // Send a packet to check the drive is responding.
        {
            let request = Request::default();
            let to_send = request.to_wire(&mut buffer).context("Failed to serialize request")?;
            socket.send(&buffer[..to_send])?;

            let received = socket.recv(&mut buffer)?;
            let _response = Response::from_wire(&buffer[..received])?;
        }

        println!("Connected to drive at {:?} from {:?}", socket.peer_addr().unwrap(), socket.local_addr().unwrap());

        #[cfg(feature = "questdb-rs")]
        let metrics = DriveMetrics::new((1000 / options.loop_interval) as usize)
            .inspect_err(|e| {
                eprintln!("Failed to create metrics reporter: {e}");
            })
            .ok();

        Ok(Self {
            socket,
            loop_interval,
            report_interval: Duration::from_millis(options.report_interval),
            buffer,
            last_response,
            last_state: State::NotReadyToSwitchOn,
            control_flags: ControlFlags::empty(),
            acknowledge_error: true,
            moving_forwards: false,
            in_range: false,
            stroke_limits: StrokeLimits {
                stroke_limit: Position::from_millimeters_f64(options.stroke_limit),
                velocity_limit: Velocity::from_meters_per_second_f64(options.velocity_limit),
                acceleration_limit: Acceleration::from_meters_per_second_squared_f64(options.acceleration_limit),
            },
            stroke_params,
            #[cfg(feature = "questdb-rs")]
            metrics,
        })
    }

    fn get_motion_command_for_stroke_params(
        limits: &StrokeLimits,
        params: &StrokeParams,
        moving_forwards: &mut bool,
        demand_position: &Position,
        in_range: &mut bool,
        demand_velocity: &Velocity,
    ) -> Command {
        let start_position = params.start.min(limits.stroke_limit);
        let end_position = params.end.min(limits.stroke_limit);

        // TODO: There's an issue with the motor overshooting if deceleration is reduced during active deceleration.
        //       Until we can rework the control loop, work around it by stopping on overshoot, to let our next command correct it.
        // TODO: We're still seeing occasional overshoots with this, we're going to need to model the VAI completely.
        if *in_range {
            if *demand_velocity > Velocity::from_meters_per_second(0) {
                if *demand_position >= end_position + params.direction_change_tolerance {
                    *in_range = false;
                    *moving_forwards = false;
                    return Command::VaiStop { deceleration: Acceleration::from_meters_per_second_squared(10) };
                }
            } else {
                if *demand_position <= start_position - params.direction_change_tolerance {
                    *in_range = false;
                    *moving_forwards = true;
                    return Command::VaiStop { deceleration: Acceleration::from_meters_per_second_squared(10) };
                }
            }
        } else if *demand_position >= start_position && *demand_position <= end_position {
            *in_range = true;
        }

        if *moving_forwards {
            if params.stopped {
                return Command::VaiStop { deceleration: params.forwards_deceleration.min(limits.acceleration_limit) };
            }

            if *demand_position >= end_position - params.direction_change_tolerance {
                *moving_forwards = false;
            }

            Command::VaiGoToPos {
                target_position: end_position,
                maximal_velocity: params.forwards_velocity.min(limits.velocity_limit),
                acceleration: params.forwards_acceleration.min(limits.acceleration_limit),
                deceleration: params.forwards_deceleration.min(limits.acceleration_limit),
            }
        } else {
            if params.stopped {
                return Command::VaiStop { deceleration: params.backwards_deceleration.min(limits.acceleration_limit) };
            }

            if *demand_position <= start_position + params.direction_change_tolerance {
                *moving_forwards = true;
            }

            Command::VaiGoToPos {
                target_position: start_position,
                maximal_velocity: params.backwards_velocity.min(limits.velocity_limit),
                acceleration: params.backwards_acceleration.min(limits.acceleration_limit),
                deceleration: params.backwards_deceleration.min(limits.acceleration_limit),
            }
        }
    }

    fn loop_tick(&mut self) -> Result<()> {
        let mut request = Request {
            response_flags: ResponseFlags::STATUS_FLAGS
                | ResponseFlags::STATE
                | ResponseFlags::ACTUAL_POSITION
                | ResponseFlags::DEMAND_POSITION
                | ResponseFlags::CURRENT
                | ResponseFlags::WARNING_FLAGS
                | ResponseFlags::ERROR_CODE
                | ResponseFlags::MONITORING_CHANNEL,
            ..Default::default()
        };

        let last_response = { self.last_response.lock().unwrap().clone() };

        // TODO: We currently have several control bits forced in the parameter configuration,
        //       re-evaluate if we want to implement the full state machine instead.
        if let Some(Response {
            state: Some(state),
            demand_position: Some(demand_position),
            monitoring_channel: Some((demand_velocity, ..)),
            ..
        }) = last_response.as_ref()
        {
            if *state != self.last_state {
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

                    let stroke_params = self.stroke_params.lock().unwrap();

                    if stroke_params.enabled {
                        self.control_flags = ControlFlags::SWITCH_ON;
                    }
                }
                State::Error { error_code } if self.acknowledge_error => {
                    println!("Acknowledging error: {error_code:?}");

                    // TODO: Clearing a fatal error (e.g. slider missing) requires a drive reset.
                    self.control_flags = ControlFlags::ERROR_ACKNOWLEDGE;
                }
                State::OperationEnabled { homed: false, .. } => {
                    self.control_flags.insert(ControlFlags::HOME);
                }
                State::OperationEnabled { homed: true, motion_command_count, .. } => {
                    let next_command_count = (motion_command_count.wrapping_add(1)) & 0xF;

                    let stroke_params = self.stroke_params.lock().unwrap();

                    if !stroke_params.enabled {
                        self.control_flags.remove(ControlFlags::SWITCH_ON);
                    } else {
                        let command = Self::get_motion_command_for_stroke_params(
                            &self.stroke_limits,
                            &stroke_params,
                            &mut self.moving_forwards,
                            demand_position,
                            &mut self.in_range,
                            &Velocity(i32::from_ne_bytes(demand_velocity.to_ne_bytes())),
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

        #[cfg(feature = "questdb-rs")]
        if let Some(metrics) = &mut self.metrics {
            metrics.record(&request, &response);
        }

        {
            *self.last_response.lock().unwrap() = Some(response);
        }

        Ok(())
    }

    fn start_loop(&mut self) -> Result<()> {
        let mut last_loop_report = Instant::now();
        let mut loop_duration_sum = Duration::ZERO;
        let mut loop_duration_min = Duration::MAX;
        let mut loop_duration_max = Duration::ZERO;
        let mut loop_message_count: usize = 0;
        let mut loop_error_history = Vec::new();

        let mut next_tick = Instant::now() + self.loop_interval;

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

            if last_loop_report.elapsed() >= self.report_interval {
                // println!();

                // TODO: Print the error history in a compact format
                let avg_loop_duration = loop_duration_sum / (loop_message_count as u32);
                println!(
                    "Timing statistics: {:?} average, {:?} min, {:?} max, {:.2}% usage ({:.2}% peak), {}/{} errors",
                    avg_loop_duration,
                    loop_duration_min,
                    loop_duration_max,
                    (avg_loop_duration.as_secs_f64() / self.loop_interval.as_secs_f64()) * 100.0,
                    (loop_duration_max.as_secs_f64() / self.loop_interval.as_secs_f64()) * 100.0,
                    loop_error_history.len(),
                    loop_message_count,
                );

                self.print_drive_status();

                // {
                //     let stroke_params = self.stroke_params.lock().unwrap();
                //     println!("{:#?}", *stroke_params);
                // }

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
                next_tick += self.loop_interval;
            } else {
                let late_by = now.duration_since(next_tick);
                eprintln!("Late by {late_by:?}");
                next_tick = now + self.loop_interval;
            }
        }
    }

    fn print_drive_status(&self) {
        let last_response = self.last_response.lock().unwrap();
        let Some(response) = last_response.as_ref() else {
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

        if let (Some(warning_flags), Some(error_code)) = (&response.warning_flags, &response.error_code) {
            if !warning_flags.is_empty() || *error_code != ErrorCode::NoError {
                println!("Warnings: {warning_flags:?}, Error: {error_code:?}");
            }
        }
    }
}

#[cfg(feature = "questdb-rs")]
struct DriveMetrics {
    flush_count: usize,
    sender: Sender,
    buffer: Buffer,
}

#[cfg(feature = "questdb-rs")]
impl DriveMetrics {
    fn new(flush_count: usize) -> Result<Self> {
        let sender = Sender::from_env()?;
        let buffer = sender.new_buffer();

        Ok(Self { flush_count, sender, buffer })
    }

    fn record(&mut self, request: &Request, response: &Response) {
        self.buffer.table("linmot_stats").unwrap();

        if let Some(control_flags) = request.control_flags {
            self.buffer.column_i64("control_flags", control_flags.bits() as i64).unwrap();
        }

        if let Some(motion_command) = &request.motion_command {
            self.buffer.column_i64("motion_command_id", motion_command.command.id() as i64).unwrap();

            if let Command::VaiGoToPos { target_position, .. } = &motion_command.command {
                self.buffer.column_f64("target_position", target_position.0 as f64).unwrap();
            }
        }

        // TODO: Consider packing status_flags+state+warning_flags+error_code into a single column
        // TODO: Instead, pre-create the table on the QuestDB side to specify smaller storage data types

        if let Some(status_flags) = response.status_flags {
            self.buffer.column_i64("status_flags", status_flags.bits() as i64).unwrap();
        }

        if let Some(state) = response.state {
            self.buffer.column_i64("state", u16::from(state) as i64).unwrap();
        }

        if let Some(actual_position) = response.actual_position {
            self.buffer.column_i64("actual_position", actual_position.0 as i64).unwrap();
        }

        if let Some(demand_position) = response.demand_position {
            self.buffer.column_i64("demand_position", demand_position.0 as i64).unwrap();
        }

        if let Some(current) = response.current {
            self.buffer.column_i64("current", current.0 as i64).unwrap();
        }

        if let Some(warning_flags) = response.warning_flags {
            self.buffer.column_i64("warning_flags", warning_flags.bits() as i64).unwrap();
        }

        if let Some(error_code) = response.error_code {
            self.buffer.column_i64("error_code", u16::from(error_code) as i64).unwrap();
        }

        // TODO: Get the current config during startup to verify these are setup correctly in the drive.
        if let Some((a, b, c, d)) = response.monitoring_channel {
            let demand_velocity = i32::from_ne_bytes(a.to_ne_bytes());
            self.buffer.column_i64("demand_velocity", demand_velocity as i64).unwrap();
            let demand_acceleration = i32::from_ne_bytes(b.to_ne_bytes());
            self.buffer.column_i64("demand_acceleration", demand_acceleration as i64).unwrap();
            let drive_temp = c as u16;
            self.buffer.column_i64("drive_temp", drive_temp as i64).unwrap();
            let motor_temp = d as u16;
            self.buffer.column_i64("motor_temp", motor_temp as i64).unwrap();
        }

        self.buffer.at(TimestampMicros::now()).unwrap();

        if self.buffer.row_count() >= self.flush_count {
            // TODO: We may want to move flushing to a separate thread, which'll involve moving this entire struct.
            // TODO: Figure out our error handling needs here.
            self.sender.flush(&mut self.buffer).unwrap();
        }
    }
}
