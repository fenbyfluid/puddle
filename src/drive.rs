use crate::CoreEvent;
use crate::metrics::Record;
use anyhow::Result;
use linmot::mci::units::{Acceleration, Current, DriveTemperature, MotorTemperature, Position, Velocity};
use linmot::mci::{Command, ControlFlags, ErrorCode, MotionCommand as MciMotionCommand, State, WarningFlags};
use linmot::udp::{BUFFER_SIZE, CONTROLLER_PORT, DRIVE_PORT, Request, Response, ResponseFlags};
use log::{error, info, trace, warn};
use puddle::messages::{DriveState, MotionCommand as CoreMotionCommand};
use std::net::{Ipv4Addr, UdpSocket};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

// Control handle held by both threads.
// TODO: Consider flattening this, the separate actions isn't as important now, and we'd like some
//       more consistent handling. One specific goal is that controller disconnection should be a
//       controlled stop and then power off once the motion is complete.
#[derive(Clone, Default)]
pub struct DriveInterface {
    pub commands: Arc<Mutex<DriveCommands>>,
    pub actions: Arc<DriveActions>,
}

// Core->Drive communication
#[derive(Debug, Clone, Default)]
pub struct DriveCommands {
    pub power_enabled: bool,
    pub motion_enabled: bool,
    pub commands: Vec<CoreMotionCommand>,
}

pub const ACTION_RESET_INDEX: u8 = 1 << 0;
pub const ACTION_ACK_ERROR: u8 = 1 << 1;

// Core->Drive one-off actions
#[derive(Default)]
pub struct DriveActions {
    pending: AtomicU8,
}

impl DriveActions {
    /// Core thread: set action bits. Multiple calls accumulate.
    pub fn send(&self, bits: u8) {
        self.pending.fetch_or(bits, Ordering::Release);
    }

    /// Drive thread: atomically read and clear all pending actions.
    pub fn take(&self) -> u8 {
        self.pending.swap(0, Ordering::AcqRel)
    }
}

// Drive->Core communication
#[derive(Debug, Clone, Default)]
pub struct DriveFeedback {
    // TODO: Keep this small as possible - it's only what controllers need, not the raw metrics
    //       We've currently got just about everything shoved into CoreState, which may make for
    //       nicer real-time graphs in the UI than looping via QuestDB (which was admittedly quite
    //       delayed), but it's really not all absolutely required.
    pub drive_state: DriveState,
    pub active_command_index: usize,
    pub actual_position: Position,
    pub demand_position: Position,
    pub demand_velocity: Velocity,
    pub demand_acceleration: Acceleration,
    pub current_draw: Current,
    pub warning_flags: WarningFlags,
    pub error_code: ErrorCode,
    pub drive_temperature: DriveTemperature,
    pub motor_temperature: MotorTemperature,
}

pub struct ConnectionManager {
    pub interface: DriveInterface,
}

impl ConnectionManager {
    pub fn new(
        address: String,
        interval: Duration,
        core_sender: mpsc::Sender<CoreEvent>,
        metrics_sender: Option<mpsc::Sender<Record>>,
    ) -> Self {
        let interface = DriveInterface::default();

        let connection_interface = interface.clone();

        std::thread::spawn(move || {
            if let Err(e) = thread_priority::set_thread_priority_and_policy(
                thread_priority::thread_native_id(),
                thread_priority::ThreadPriority::Max,
                thread_priority::ThreadSchedulePolicy::Realtime(
                    thread_priority::RealtimeThreadSchedulePolicy::RoundRobin,
                ),
            ) {
                warn!("Failed to set thread priority: {}", e);
            }

            if let Some(cores) = core_affinity::get_core_ids() {
                if !core_affinity::set_for_current(cores[0]) {
                    warn!("Could not set affinity to core 0");
                }
            } else {
                warn!("Could not get core list, not setting affinity");
            }

            let mut retry_time = Duration::from_secs(1);

            loop {
                // Reset the feedback state on each connection attempt.
                let _ = core_sender.send(CoreEvent::DriveStateUpdated(DriveFeedback::default()));

                let mut connection = match Connection::new(
                    &address,
                    interval,
                    core_sender.clone(),
                    metrics_sender.clone(),
                    connection_interface.clone(),
                ) {
                    Ok(connection) => connection,
                    Err(e) => {
                        error!("Failed to connect to drive: {}, trying again in {:?}", e, retry_time);
                        std::thread::sleep(retry_time);
                        retry_time = (retry_time * 10).min(Duration::from_secs(30));
                        continue;
                    }
                };

                if let Err(e) = connection.run_loop() {
                    error!("Error in drive loop: {}", e);
                }
            }
        });

        Self { interface }
    }
}

pub struct Connection {
    interval: Duration,
    core_sender: mpsc::Sender<CoreEvent>,
    metrics_sender: Option<mpsc::Sender<Record>>,
    interface: DriveInterface,
    socket: UdpSocket,
    buffer: [u8; BUFFER_SIZE],
    last_rtt: Option<Duration>,
    control_flags: ControlFlags,
    next_motion_command: Option<MciMotionCommand>,
    power_enabled: bool,
    motion_enabled: bool,
    active_command_index: usize,
    input_commands: Vec<CoreMotionCommand>,
    last_request: Request,
    last_response: Response,
    last_command_index: usize,
    last_state: State,
}

impl Connection {
    pub fn new(
        address: &str,
        interval: Duration,
        core_sender: mpsc::Sender<CoreEvent>,
        metrics_sender: Option<mpsc::Sender<Record>>,
        interface: DriveInterface,
    ) -> Result<Self> {
        info!("Connecting to drive at {}:{}...", address, DRIVE_PORT);

        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, CONTROLLER_PORT))?;

        socket.connect((address, DRIVE_PORT))?;
        socket.set_read_timeout(Some(interval / 2))?;

        let mut connection = Self {
            interval,
            core_sender,
            metrics_sender,
            interface,
            socket,
            buffer: [0u8; BUFFER_SIZE],
            last_rtt: None,
            control_flags: ControlFlags::empty(),
            next_motion_command: None,
            power_enabled: false,
            motion_enabled: false,
            active_command_index: 0,
            input_commands: Vec::new(),
            last_request: Request::default(),
            last_response: Response::default(),
            last_command_index: 0,
            last_state: State::NotReadyToSwitchOn,
        };

        // Send a packet to check the drive is responding.
        connection.send_request(&Request::default())?;

        info!("Connected to drive at {:?} from {:?}", connection.socket.peer_addr()?, connection.socket.local_addr()?);

        // TODO: Send a number of RealtimeConfiguration commands to check the monitoring channels configuration.

        // TODO: Validate other drive configuration too, such as the forced control flags.

        Ok(connection)
    }

    fn send_request(&mut self, request: &Request) -> Result<Response> {
        let now = Instant::now();
        self.last_rtt = None;

        trace!("Sending request: {:?}", request);

        let to_send = request.to_wire(&mut self.buffer)?;
        self.socket.send(&self.buffer[..to_send])?;

        let received = self.socket.recv(&mut self.buffer)?;
        let response = Response::from_wire(&self.buffer[..received])?;

        trace!("Received response: {:?}", response);

        self.last_rtt = Some(now.elapsed());

        Ok(response)
    }

    fn run_loop(&mut self) -> Result<()> {
        loop {
            let start = Instant::now();
            self.loop_tick()?;

            self.record_metrics(start.elapsed());

            let next = start + self.interval;
            let now = Instant::now();
            if let Some(sleep_time) = next.checked_duration_since(now) {
                // Use sleep_until when it is stabilized.
                std::thread::sleep(sleep_time);
            } else {
                warn!("Drive loop running slow! Late by {:?}", now.duration_since(next));
                // TODO: Do we need to consider anything other than just immediately going again?
                //       We used to wait for the next interval, but that had longer gaps between
                //       commands. Our motion prediction may be very off with this approach?
                continue;
            }
        }
    }

    fn loop_tick(&mut self) -> Result<()> {
        // 1. Send the current computed state to the drive

        let request = Request {
            control_flags: Some(self.control_flags),
            motion_command: self.next_motion_command.take(),
            realtime_configuration: None,
            response_flags: ResponseFlags::all(),
        };

        self.last_response = self.send_request(&request)?;
        self.last_request = request;
        self.last_command_index = self.active_command_index;

        // 2. Send feedback to the core

        if let (
            Some(state),
            Some(actual_position),
            Some(demand_position),
            Some(current),
            Some(warning_flags),
            Some(error_code),
            Some((demand_velocity, demand_acceleration, drive_temperature, motor_temperature)),
        ) = (
            self.last_response.state(),
            self.last_response.actual_position,
            self.last_response.demand_position,
            self.last_response.current,
            self.last_response.warning_flags,
            self.last_response.error_code(),
            self.last_response.monitoring_channel,
        ) {
            if should_log_state_change(self.last_state, state) {
                info!("Drive state changed: {:?} -> {:?}", self.last_state, state);
                self.last_state = state;
            }

            let feedback = DriveFeedback {
                drive_state: match state {
                    State::ReadyToSwitchOn => DriveState::Off,
                    State::OperationEnabled { homed: true, .. } => {
                        if self.motion_enabled {
                            DriveState::Moving
                        } else {
                            DriveState::Paused
                        }
                    }
                    State::Error { .. } => DriveState::Errored,
                    _ => DriveState::Preparing,
                },
                active_command_index: self.active_command_index,
                actual_position,
                demand_position,
                demand_velocity: Velocity(demand_velocity as i32),
                demand_acceleration: Acceleration(demand_acceleration as i32),
                current_draw: current,
                warning_flags,
                error_code,
                drive_temperature: DriveTemperature(drive_temperature as i16),
                motor_temperature: MotorTemperature(motor_temperature as i16),
            };

            self.core_sender.send(CoreEvent::DriveStateUpdated(feedback))?;
        }

        // 3. Read any new instructions from the core

        let actions = self.interface.actions.take();
        if (actions & ACTION_RESET_INDEX) != 0 {
            self.active_command_index = 0;
        }
        if (actions & ACTION_ACK_ERROR) != 0 {
            self.control_flags.insert(ControlFlags::ERROR_ACKNOWLEDGE);
        }

        // If we can't take the lock, we'll just try again next time
        if let Ok(shared) = self.interface.commands.try_lock() {
            self.power_enabled = shared.power_enabled;
            self.motion_enabled = shared.motion_enabled;
            self.input_commands.clone_from(&shared.commands);
        }

        // 4. Compute the next motion command
        self.compute_next_request()?;

        Ok(())
    }

    fn compute_next_request(&mut self) -> Result<()> {
        // If we don't have a valid response, clear all control flags
        let Some(state) = self.last_response.state() else {
            self.control_flags = ControlFlags::empty();

            return Ok(());
        };

        match state {
            State::NotReadyToSwitchOn => {
                self.control_flags = ControlFlags::empty();
            }
            State::ReadyToSwitchOn => {
                if self.power_enabled {
                    self.control_flags = ControlFlags::SWITCH_ON;
                }
            }
            State::OperationEnabled { homed: false, .. } => {
                self.control_flags.insert(ControlFlags::HOME);
            }
            State::Homing { finished: true } => {
                self.control_flags.remove(ControlFlags::HOME);
            }
            _ => {}
        }

        if !self.power_enabled {
            self.control_flags.remove(ControlFlags::SWITCH_ON);

            return Ok(());
        }

        // Control flags are set, now compute the next motion command
        let (
            State::OperationEnabled { homed: true, motion_command_count, .. },
            Some(demand_position),
            Some((demand_velocity, demand_acceleration, _, _)),
        ) = (state, self.last_response.demand_position, self.last_response.monitoring_channel)
        else {
            return Ok(());
        };

        let demand_velocity = Velocity(demand_velocity as i32);
        let demand_acceleration = Acceleration(demand_acceleration as i32);
        let next_position = predict_position(demand_position, demand_velocity, demand_acceleration, self.interval);

        let next_command_count = motion_command_count.wrapping_add(1) & 0xF;

        // TODO: Where do we want to get this from?
        let hard_deceleration = Acceleration::from_meters_per_second_squared(5);

        if !self.motion_enabled || self.input_commands.is_empty() {
            self.next_motion_command = Some(MciMotionCommand {
                count: next_command_count,
                command: Command::VaiStop { deceleration: hard_deceleration },
            });

            return Ok(());
        }

        self.active_command_index = self.active_command_index % self.input_commands.len();
        let mut input_command = self.input_commands.get(self.active_command_index).unwrap();

        // Check if we've reached the target by seeing if the predicted position
        // has crossed to the far side of the target relative to where we are now.
        let target_reached = {
            let pos = demand_position;
            let target = input_command.position;
            let predicted = next_position;
            let distance_now = target - pos;

            if distance_now == Position::ZERO {
                // Already at target
                true
            } else {
                // Have we crossed the target? Sign of (target - predicted) differs
                // from sign of (target - pos), meaning we've passed it.
                let distance_after = target - predicted;
                (distance_now > Position::ZERO && distance_after <= Position::ZERO)
                    || (distance_now < Position::ZERO && distance_after >= Position::ZERO)
            }
        };

        if target_reached {
            self.active_command_index = (self.active_command_index + 1) % self.input_commands.len();
            input_command = self.input_commands.get(self.active_command_index).unwrap();
        }

        let deceleration = if target_reached {
            input_command.deceleration
        } else {
            clamp_deceleration(demand_position, demand_velocity, input_command.position, input_command.deceleration)
                .unwrap_or(hard_deceleration)
        };

        self.next_motion_command = Some(MciMotionCommand {
            count: next_command_count,
            command: Command::VaiGoToPos {
                target_position: input_command.position,
                maximal_velocity: input_command.velocity,
                acceleration: input_command.acceleration,
                deceleration,
            },
        });

        Ok(())
    }

    fn record_metrics(&self, loop_duration: Duration) {
        let Some(sender) = &self.metrics_sender else {
            return;
        };

        match Record::new(
            loop_duration,
            self.last_rtt.unwrap_or_default(),
            self.last_command_index,
            &self.last_request,
            &self.last_response,
        ) {
            Ok(record) => {
                if let Err(e) = sender.send(record) {
                    error!("Error sending metrics record: {}", e);
                }
            }
            Err(e) => {
                error!("Error creating metrics record: {}", e);
            }
        }
    }
}

pub fn predict_position(
    position: Position,
    velocity: Velocity,
    acceleration: Acceleration,
    duration: Duration,
) -> Position {
    let t_us = duration.as_micros() as i64;

    let p0 = position.0 as i64;
    let v0 = velocity.0 as i64;
    let a = acceleration.0 as i64;

    let vt = v0 * t_us * 10 / 1_000_000;
    let at2 = a * t_us * t_us * 50 / 1_000_000_000_000;

    Position((p0 + vt + at2) as i32)
}

/// Computes the minimum deceleration needed to avoid overshooting the
/// target position given the current state.
///
/// If the requested deceleration is sufficient, returns it unchanged.
/// Otherwise returns the minimum deceleration that stops at or before
/// the target.
///
/// Returns `None` if the target is already behind the current position
/// (overshoot has already occurred, or remaining distance is zero with
/// nonzero velocity). The caller must handle this (e.g., emergency stop
/// or reversal).
pub fn clamp_deceleration(
    actual_position: Position,
    velocity: Velocity,
    target_position: Position,
    requested_deceleration: Acceleration,
) -> Option<Acceleration> {
    let v = velocity.0 as i64;

    if v == 0 {
        return Some(requested_deceleration);
    }

    let pos = actual_position.0 as i64;
    let target = target_position.0 as i64;

    // Remaining distance in the direction of travel
    let remaining = if v > 0 { target - pos } else { pos - target };

    if remaining <= 0 {
        return None;
    }

    let remaining = remaining as u64;
    let v_abs = v.unsigned_abs();

    // Stopping distance at requested deceleration: v² / (2d)
    let d = requested_deceleration.0 as i64;
    if d > 0 {
        let stopping_distance = v_abs * v_abs / (2 * d as u64);
        if stopping_distance <= remaining {
            return Some(requested_deceleration);
        }
    }

    // Minimum deceleration: d_min = v² / (2 * remaining)
    // Ceiling division to ensure we don't undershoot the deceleration
    let d_min = (v_abs * v_abs + 2 * remaining - 1) / (2 * remaining);

    Some(Acceleration(d_min as i32))
}

fn should_log_state_change(old_state: State, new_state: State) -> bool {
    fn normalize_state(state: State) -> State {
        match state {
            State::OperationEnabled {
                motion_command_count: _,
                event_handler,
                motion_active,
                in_target_position,
                homed,
            } => State::OperationEnabled {
                motion_command_count: 0,
                event_handler,
                motion_active,
                in_target_position,
                homed,
            },
            other => other,
        }
    }

    normalize_state(old_state) != normalize_state(new_state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn assert_position_mm(result: Position, expected_mm: f64) {
        let expected_units = (expected_mm * 10_000.0).round() as i32;
        let tolerance = 1; // 1 unit = 0.1 μm
        assert!(
            (result.0 - expected_units).abs() <= tolerance,
            "expected ~{expected_mm}mm ({expected_units} units), got {} units ({:.4}mm)",
            result.0,
            result.0 as f64 / 10_000.0,
        );
    }

    #[test]
    fn zero_duration_returns_original_position() {
        let p = predict_position(
            Position::from_millimeters(100),
            Velocity::from_millimeters_per_second(500),
            Acceleration::from_meters_per_second_squared(10),
            Duration::ZERO,
        );
        assert_eq!(p.0, Position::from_millimeters(100).0);
    }

    #[test]
    fn constant_velocity_only() {
        // 100mm, 500mm/s, 0 accel, 10ms → 100 + 5 = 105mm
        let p = predict_position(
            Position::from_millimeters(100),
            Velocity::from_millimeters_per_second(500),
            Acceleration::from_millimeters_per_second_squared(0),
            Duration::from_millis(10),
        );
        assert_position_mm(p, 105.0);
    }

    #[test]
    fn acceleration_from_rest() {
        // 0mm, 0 velocity, 1 m/s², 1s → ½ * 1 * 1 = 500mm
        let p = predict_position(
            Position::from_millimeters(0),
            Velocity::from_millimeters_per_second(0),
            Acceleration::from_meters_per_second_squared(1),
            Duration::from_secs(1),
        );
        assert_position_mm(p, 500.0);
    }

    #[test]
    fn velocity_and_acceleration_combined() {
        // 50mm, 200mm/s, 5 m/s², 5ms
        // vt = 200 * 0.005 = 1mm
        // ½at² = ½ * 5000 * 0.000025 = 0.0625mm
        let p = predict_position(
            Position::from_millimeters(50),
            Velocity::from_millimeters_per_second(200),
            Acceleration::from_millimeters_per_second_squared(5000),
            Duration::from_millis(5),
        );
        assert_position_mm(p, 51.0625);
    }

    #[test]
    fn negative_velocity_and_deceleration() {
        // -100mm, -500mm/s, -2 m/s², 10ms
        // vt = -500 * 0.01 = -5mm
        // ½at² = ½ * -2000 * 0.0001 = -0.1mm
        let p = predict_position(
            Position::from_millimeters(-100),
            Velocity::from_millimeters_per_second(-500),
            Acceleration::from_millimeters_per_second_squared(-2000),
            Duration::from_millis(10),
        );
        assert_position_mm(p, -105.1);
    }

    #[test]
    fn sufficient_deceleration_unchanged() {
        // 1 m/s, 50mm remaining, 10 m/s² → stops in exactly 50mm
        let result = clamp_deceleration(
            Position::from_millimeters(50),
            Velocity::from_meters_per_second(1),
            Position::from_millimeters(100),
            Acceleration::from_meters_per_second_squared(10),
        );
        assert_eq!(result, Some(Acceleration::from_meters_per_second_squared(10)));
    }

    #[test]
    fn insufficient_deceleration_clamped() {
        // 1 m/s, 50mm remaining, 5 m/s² → needs 100mm, clamped to 10 m/s²
        let result = clamp_deceleration(
            Position::from_millimeters(50),
            Velocity::from_meters_per_second(1),
            Position::from_millimeters(100),
            Acceleration::from_meters_per_second_squared(5),
        );
        assert_eq!(result, Some(Acceleration::from_meters_per_second_squared(10)));
    }

    #[test]
    fn negative_velocity_clamped() {
        // -1 m/s at 100mm, target 50mm, 5 m/s² → same physics, opposite direction
        let result = clamp_deceleration(
            Position::from_millimeters(100),
            Velocity::from_meters_per_second(-1),
            Position::from_millimeters(50),
            Acceleration::from_meters_per_second_squared(5),
        );
        assert_eq!(result, Some(Acceleration::from_meters_per_second_squared(10)));
    }

    #[test]
    fn past_target_returns_none() {
        let result = clamp_deceleration(
            Position::from_millimeters(110),
            Velocity::from_meters_per_second(1),
            Position::from_millimeters(100),
            Acceleration::from_meters_per_second_squared(10),
        );
        assert_eq!(result, None);
    }
}
