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
        overshoot_margin: Position,
        hard_deceleration_min: Acceleration,
        hard_deceleration_max: Acceleration,
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
                    overshoot_margin,
                    hard_deceleration_min,
                    hard_deceleration_max,
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
    overshoot_margin: Position,
    hard_deceleration_min: Acceleration,
    hard_deceleration_max: Acceleration,
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
    active_command_has_approached: bool,
    // The approach direction of the current command: sign(current_target - previous_target).
    // None on the first command or after a reset. When None, clamp_deceleration falls back
    // to the velocity/displacement sign heuristic.
    active_approach_direction: Option<i32>,
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
        overshoot_margin: Position,
        hard_deceleration_min: Acceleration,
        hard_deceleration_max: Acceleration,
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
            overshoot_margin,
            hard_deceleration_min,
            hard_deceleration_max,
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
            active_command_has_approached: false,
            active_approach_direction: None,
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
            log_state_change(&mut self.last_state, state);

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
            self.active_command_has_approached = false;
            self.active_approach_direction = None;
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

        let next_command_count = motion_command_count.wrapping_add(1) & 0xF;

        if self.input_commands.is_empty() {
            self.next_motion_command = Some(MciMotionCommand {
                count: next_command_count,
                command: Command::VaiStop { deceleration: self.hard_deceleration_min },
            });
            return Ok(());
        }

        self.active_command_index = self.active_command_index % self.input_commands.len();
        let input_command = self.input_commands.get(self.active_command_index).unwrap();
        let current_target = input_command.position;

        // Determine the approach direction for this command: the direction from the
        // previous waypoint to this one. This is used by clamp_deceleration to
        // correctly identify overshoots vs. normal mid-stroke reversal states.
        //
        // On the first command (or after a reset), we fall back to None, which
        // causes clamp_deceleration to use the velocity/displacement heuristic.
        let approach_direction = self.active_approach_direction;

        // Arrival detection: have we reached the current target waypoint?
        //
        // Three cases, in order:
        //
        // 1. Exactly at target: always reached.
        //
        // 2. Within the overshoot margin AND we have previously been observed moving
        //    toward this target (has_approached): reached if now stationary or moving
        //    away (the VAI has begun parking or reversing onto the target).
        //    The has_approached guard prevents immediately declaring arrival when a new
        //    target happens to start within the margin of the current position, which
        //    would cause the drive to never move toward it.
        //
        // 3. Otherwise (outside the margin, or not yet committed): use predicted-
        //    crossing detection. This handles the normal full-speed approach where the
        //    drive will cross the target between this tick and the next.
        //    Note: this can fail to fire when the drive decelerates to a stop just
        //    short of the target, in which case case 2 takes over on subsequent ticks
        //    once has_approached is set and the VAI begins its final parking move.
        let target_reached = {
            let displacement = current_target.0 as i64 - demand_position.0 as i64;
            let dist = displacement.unsigned_abs();
            let margin = self.overshoot_margin.0.unsigned_abs() as u64;
            let v = demand_velocity.0 as i64;

            let moving_toward = match approach_direction {
                Some(dir) if dir != 0 => (dir > 0 && v > 0) || (dir < 0 && v < 0),
                _ => (displacement > 0 && v > 0) || (displacement < 0 && v < 0),
            };

            // Record that we've committed to this command (observed moving toward target).
            if moving_toward {
                self.active_command_has_approached = true;
            }

            if dist == 0 {
                true
            } else if dist <= margin && self.active_command_has_approached {
                // Within arrival zone and we've committed to this target:
                // reached if stopped (v==0) or moving away (VAI parking/reversing).
                // We don't wait for moving_away to become true when already stopped,
                // as that would add an unnecessary extra tick of latency.
                v == 0 || !moving_toward
            } else {
                // Outside margin, or not yet committed: only reached by predicted crossing.
                let next_position =
                    predict_position(demand_position, demand_velocity, demand_acceleration, self.interval);
                let distance_after = current_target.0 as i64 - next_position.0 as i64;
                (displacement > 0 && distance_after <= 0) || (displacement < 0 && distance_after >= 0)
            }
        };

        let (clamp_target, clamp_approach_direction) = if target_reached {
            let prev_index = (self.active_command_index + self.input_commands.len() - 1) % self.input_commands.len();
            let prev_target = self.input_commands[prev_index].position;

            // Advance to next command.
            self.active_command_index = (self.active_command_index + 1) % self.input_commands.len();
            self.active_command_has_approached = false;
            let next_command = self.input_commands.get(self.active_command_index).unwrap();

            // The new approach direction is from current_target toward the next target.
            let new_dir = (next_command.position.0 - current_target.0).signum();
            self.active_approach_direction = Some(new_dir);

            // For the clamp this tick, protect against overshooting the target we just
            // reached, using the approach direction we just completed.
            let completed_dir = (current_target.0 - prev_target.0).signum();
            (current_target, Some(completed_dir))
        } else {
            (current_target, approach_direction)
        };

        // Re-fetch after possible advance.
        let input_command = self.input_commands.get(self.active_command_index).unwrap();

        if !self.motion_enabled {
            let deceleration = clamp_deceleration(
                demand_position,
                demand_velocity,
                clamp_target,
                self.hard_deceleration_min,
                clamp_approach_direction,
                self.overshoot_margin,
                self.hard_deceleration_min,
                self.hard_deceleration_max,
            );

            self.next_motion_command =
                Some(MciMotionCommand { count: next_command_count, command: Command::VaiStop { deceleration } });

            return Ok(());
        }

        let deceleration = clamp_deceleration(
            demand_position,
            demand_velocity,
            clamp_target,
            input_command.deceleration,
            clamp_approach_direction,
            self.overshoot_margin,
            self.hard_deceleration_min,
            self.hard_deceleration_max,
        );

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

/// Computes a safe deceleration value to send to the drive's built-in VAI engine.
///
/// The VAI uses deceleration as its primary stopping parameter. The safety
/// requirement is that the drive must not overshoot `target_position` beyond
/// `overshoot_margin` in the approach direction. This function raises
/// `requested_deceleration` if it would be insufficient to stop within that
/// bound, and returns it unchanged otherwise.
///
/// `approach_direction` is the sign of `(current_target - previous_target)`:
/// positive if the drive approached from below, negative if from above. This
/// is used to correctly distinguish a genuine overshoot from a mid-reversal
/// state where the drive is still on the near side of the target but momentarily
/// carrying velocity away from it (e.g. immediately after a direction change).
/// When `None` (first command, after a reset), the function falls back to using
/// the sign of `(velocity, displacement)` as a conservative heuristic.
///
/// When past the target in the approach direction, only the remaining margin is
/// available as stopping room. The hard-stop deceleration applied scales linearly
/// between `hard_decel_min` (at zero overshoot past the margin) and `hard_decel_max`
/// (at overshoot equal to or exceeding the margin), giving a proportional response
/// rather than a step to maximum deceleration.
///
/// Drive feedback is delayed by one cycle, so the position and velocity used
/// here reflect the state from the previous cycle. This is accounted for by
/// the margin; the function itself makes no additional correction for the delay.
pub fn clamp_deceleration(
    demand_position: Position,
    demand_velocity: Velocity,
    target_position: Position,
    requested_deceleration: Acceleration,
    approach_direction: Option<i32>,
    overshoot_margin: Position,
    hard_decel_min: Acceleration,
    hard_decel_max: Acceleration,
) -> Acceleration {
    let displacement = target_position.0 as i64 - demand_position.0 as i64;
    let margin = overshoot_margin.0.unsigned_abs() as u64;
    let v = demand_velocity.0 as i64;
    let v_sq = (v * v) as u64;

    // Determine whether the drive is moving past the target in the approach direction.
    // With a known approach direction this is unambiguous. Without one, we use the
    // sign of (velocity, displacement): moving away = velocity and displacement oppose.
    let past_target = match approach_direction {
        Some(dir) if dir != 0 => {
            // Past if we've gone beyond the target in the approach direction.
            // Approach dir > 0: drive came from below, past if displacement < 0 (overshot above).
            // Approach dir < 0: drive came from above, past if displacement > 0 (overshot below).
            (dir > 0 && displacement < 0) || (dir < 0 && displacement > 0)
        }
        _ => {
            // Fallback: past if velocity and displacement oppose (moving away from target).
            (v > 0 && displacement < 0) || (v < 0 && displacement > 0)
        }
    };

    let stopping_budget = if past_target {
        let distance_past = displacement.unsigned_abs();

        if distance_past >= margin {
            // Compute a hard deceleration scaled by how far past the margin we are.
            // At distance_past == margin: hard_decel_min.
            // At distance_past >= 2*margin (or margin == 0): hard_decel_max.
            let excess = distance_past - margin;
            let scale_denom = margin.max(1);
            let t = (excess * 1000 / scale_denom).min(1000);
            let min = hard_decel_min.0 as u64;
            let max = hard_decel_max.0 as u64;
            let scaled = min + (max - min) * t / 1000;

            // This may happen harmlessly if the command parameters are changed.
            // We don't know if we're actually in the deceleration phase at the time this is called.
            warn!(
                "Recovering from overshoot of {:?}, applying {:?} deceleration",
                Position(excess as i32),
                Acceleration(scaled as i32),
            );

            return Acceleration(scaled.min(i32::MAX as u64) as i32);
        }

        margin - distance_past
    } else {
        // Only grant the overshoot margin when the requested deceleration is already
        // sufficient to stop within displacement+margin — i.e. the drive will actually
        // arrive near the target and the VAI needs slack to manage its curve.
        // If d_req is so low that it can't stop within that budget, withhold the margin
        // and clamp to the target itself, avoiding an unnecessarily generous budget for
        // a clearly insufficient deceleration.
        let base = displacement.unsigned_abs();
        let with_margin = base + margin;
        let d_req = requested_deceleration.0 as u64;
        if d_req > 0 && v_sq <= 2 * d_req * with_margin { with_margin } else { base }
    };

    if v == 0 || stopping_budget == 0 {
        return requested_deceleration;
    }

    let d_req = requested_deceleration.0 as u64;
    if d_req > 0 && v_sq <= 2 * d_req * stopping_budget {
        return requested_deceleration;
    }

    let d_min = (v_sq + 2 * stopping_budget - 1) / (2 * stopping_budget);
    Acceleration(d_min.min(i32::MAX as u64) as i32)
}

fn log_state_change(old_state: &mut State, new_state: State) {
    fn normalized_eq(a: &State, b: &State) -> bool {
        match (a, b) {
            (State::OperationEnabled { homed: h1, .. }, State::OperationEnabled { homed: h2, .. }) => h1 == h2,
            _ => a == b,
        }
    }

    fn format_state(state: &State) -> String {
        match state {
            State::OperationEnabled { homed, .. } => {
                format!("OperationEnabled {{ homed: {homed}, .. }}")
            }
            _ => format!("{state:?}"),
        }
    }

    if !normalized_eq(old_state, &new_state) {
        info!("Drive state changed: {} -> {}", format_state(old_state), format_state(&new_state));
    }

    *old_state = new_state;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    // Shorthand for the common no-approach-direction case used in most tests.
    fn clamp(
        demand_position: Position,
        demand_velocity: Velocity,
        target_position: Position,
        requested_deceleration: Acceleration,
        overshoot_margin: Position,
    ) -> Acceleration {
        clamp_deceleration(
            demand_position,
            demand_velocity,
            target_position,
            requested_deceleration,
            None,
            overshoot_margin,
            Acceleration::from_meters_per_second_squared(10),
            Acceleration::from_meters_per_second_squared(100),
        )
    }

    fn clamp_with_dir(
        demand_position: Position,
        demand_velocity: Velocity,
        target_position: Position,
        requested_deceleration: Acceleration,
        approach_direction: i32,
        overshoot_margin: Position,
    ) -> Acceleration {
        clamp_deceleration(
            demand_position,
            demand_velocity,
            target_position,
            requested_deceleration,
            Some(approach_direction),
            overshoot_margin,
            Acceleration::from_meters_per_second_squared(10),
            Acceleration::from_meters_per_second_squared(100),
        )
    }

    fn assert_position_mm(result: Position, expected_mm: f64) {
        let expected_units = (expected_mm * 10_000.0).round() as i32;
        let tolerance = 1;
        assert!(
            (result.0 - expected_units).abs() <= tolerance,
            "expected ~{expected_mm}mm ({expected_units} units), got {} units ({:.4}mm)",
            result.0,
            result.0 as f64 / 10_000.0,
        );
    }

    // --- predict_position tests ---

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
        let p = predict_position(
            Position::from_millimeters(-100),
            Velocity::from_millimeters_per_second(-500),
            Acceleration::from_millimeters_per_second_squared(-2000),
            Duration::from_millis(10),
        );
        assert_position_mm(p, -105.1);
    }

    // --- clamp_deceleration tests ---

    #[test]
    fn sufficient_deceleration_unchanged() {
        // 1 m/s, 50mm to target, 10 m/s² -> stops in exactly 50mm.
        let result = clamp(
            Position::from_millimeters(50),
            Velocity::from_meters_per_second(1),
            Position::from_millimeters(100),
            Acceleration::from_meters_per_second_squared(10),
            Position::ZERO,
        );
        assert_eq!(result, Acceleration::from_meters_per_second_squared(10));
    }

    #[test]
    fn insufficient_deceleration_clamped() {
        // 1 m/s, 50mm to target, 5 m/s² requested. Needs 100mm, clamp to 10 m/s².
        let result = clamp(
            Position::from_millimeters(50),
            Velocity::from_meters_per_second(1),
            Position::from_millimeters(100),
            Acceleration::from_meters_per_second_squared(5),
            Position::ZERO,
        );
        assert_eq!(result, Acceleration::from_meters_per_second_squared(10));
    }

    #[test]
    fn overshoot_margin_avoids_clamp() {
        // 1 m/s, 50mm to target, 5 m/s² (needs 100mm). 50mm margin -> 100mm total, fine.
        let result = clamp(
            Position::from_millimeters(50),
            Velocity::from_meters_per_second(1),
            Position::from_millimeters(100),
            Acceleration::from_meters_per_second_squared(5),
            Position::from_millimeters(50),
        );
        assert_eq!(result, Acceleration::from_meters_per_second_squared(5));
    }

    #[test]
    fn negative_velocity_clamped() {
        // -1 m/s at 100mm, target 50mm (50mm away), 5 m/s² requested.
        let result = clamp(
            Position::from_millimeters(100),
            Velocity::from_meters_per_second(-1),
            Position::from_millimeters(50),
            Acceleration::from_meters_per_second_squared(5),
            Position::ZERO,
        );
        assert_eq!(result, Acceleration::from_meters_per_second_squared(10));
    }

    #[test]
    fn past_target_at_margin_boundary_applies_min_hard_decel() {
        // Overshot by exactly 10mm, margin is 10mm. distance_past == margin -> hard_decel_min.
        let result = clamp(
            Position::from_millimeters(110),
            Velocity::from_meters_per_second(1),
            Position::from_millimeters(100),
            Acceleration::from_meters_per_second_squared(10),
            Position::from_millimeters(10),
        );
        assert_eq!(result, Acceleration::from_meters_per_second_squared(10));
    }

    #[test]
    fn past_target_double_margin_applies_max_hard_decel() {
        // Overshot by 20mm, margin is 10mm. excess == margin -> t == 1000 -> hard_decel_max.
        let result = clamp(
            Position::from_millimeters(120),
            Velocity::from_meters_per_second(1),
            Position::from_millimeters(100),
            Acceleration::from_meters_per_second_squared(10),
            Position::from_millimeters(10),
        );
        assert_eq!(result, Acceleration::from_meters_per_second_squared(100));
    }

    #[test]
    fn past_target_within_margin_clamped() {
        // Overshot by 5mm, margin is 10mm. 5mm remaining to stop.
        // v=1 m/s, budget=5mm -> d_min = 1^2 / (2 * 0.005) = 100 m/s².
        let result = clamp(
            Position::from_millimeters(105),
            Velocity::from_meters_per_second(1),
            Position::from_millimeters(100),
            Acceleration::from_meters_per_second_squared(10),
            Position::from_millimeters(10),
        );
        assert_eq!(result, Acceleration::from_meters_per_second_squared(100));
    }

    #[test]
    fn direction_reversal_benign_without_approach_dir() {
        // At 50mm, target 100mm, moving away (v=-1). No approach direction known.
        // Treated as past target (heuristic), budget = margin - 50mm.
        // With 100mm margin, budget = 50mm. d_min = 1^2 / (2*0.05) = 10 m/s². Fine.
        let result = clamp(
            Position::from_millimeters(50),
            Velocity::from_meters_per_second(-1),
            Position::from_millimeters(100),
            Acceleration::from_meters_per_second_squared(10),
            Position::from_millimeters(100),
        );
        assert_eq!(result, Acceleration::from_meters_per_second_squared(10));
    }

    #[test]
    fn direction_reversal_benign_with_approach_dir() {
        // Same geometry, but we know the approach is from below (dir=+1).
        // Drive is at 50mm < target 100mm: displacement > 0, approach > 0 -> NOT past target.
        // Budget = 50mm + 100mm = 150mm. 10 m/s² easily sufficient.
        let result = clamp_with_dir(
            Position::from_millimeters(50),
            Velocity::from_meters_per_second(-1),
            Position::from_millimeters(100),
            Acceleration::from_meters_per_second_squared(10),
            1, // approaching from below
            Position::from_millimeters(100),
        );
        assert_eq!(result, Acceleration::from_meters_per_second_squared(10));
    }

    #[test]
    fn approach_dir_prevents_false_overshoot_on_command_advance() {
        // Simulates the previously-failing case: drive just reached 150mm (A->B complete),
        // index has advanced to B->C (target 0mm), velocity still +1 m/s (moving away from 0mm).
        // Approach direction for the new B->C leg is -1 (from 150mm down to 0mm).
        // Displacement = 0 - 150 = -150mm, approach dir = -1.
        // past_target check: dir < 0 && displacement > 0 -> false. Not past target.
        // Budget = 150mm + margin. No hard stop.
        let result = clamp_with_dir(
            Position::from_millimeters(150),
            Velocity::from_meters_per_second(1),
            Position::from_millimeters(0),
            Acceleration::from_meters_per_second_squared(5),
            -1, // approaching from above (B=150 -> C=0)
            Position::from_millimeters(2),
        );
        // Budget = 150mm + 2mm = 152mm, v=1 m/s, d_min = 1/(2*0.152) ≈ 3.3 m/s² -> 5 m/s² is fine
        assert_eq!(result, Acceleration::from_meters_per_second_squared(5));
    }

    #[test]
    fn approach_dir_detects_genuine_overshoot_in_new_direction() {
        // Same B->C leg (target 0mm, approach -1), but drive has already passed 0mm
        // and is now at -5mm still moving at +1 m/s (wrong direction for B->C leg,
        // and past the target in the approach direction).
        // displacement = 0 - (-5) = +5mm > 0, approach dir = -1: past_target = true.
        // distance_past = 5mm, margin = 10mm -> budget = 5mm.
        // d_min = 1/(2*0.005) = 100 m/s².
        let result = clamp_with_dir(
            Position::from_millimeters(-5),
            Velocity::from_meters_per_second(1),
            Position::from_millimeters(0),
            Acceleration::from_meters_per_second_squared(5),
            -1,
            Position::from_millimeters(10),
        );
        assert_eq!(result, Acceleration::from_meters_per_second_squared(100));
    }

    #[test]
    fn triangle_motion_middle_waypoint_not_false_overshoot() {
        // Triangle: 0 -> 75 -> 150. Drive approaching 75mm from below (dir=+1),
        // at 70mm with v=+1 m/s. Not past target, normal approach.
        let result = clamp_with_dir(
            Position::from_millimeters(70),
            Velocity::from_meters_per_second(1),
            Position::from_millimeters(75),
            Acceleration::from_meters_per_second_squared(5),
            1,
            Position::from_millimeters(2),
        );
        // Budget = 5mm + 2mm = 7mm. d_min = 1/(2*0.007) ≈ 71.4 m/s². Must clamp.
        assert!(result.0 > Acceleration::from_meters_per_second_squared(5).0);
    }

    #[test]
    fn zigzag_motion_no_false_overshoot_at_intermediate() {
        // Zig-zag: 0 -> 75 -> 50 -> 100. Tracking toward 50mm, approach dir = -1
        // (from 75mm down to 50mm). Drive at 60mm, v=-1 m/s (moving toward 50mm).
        // Not past target. Budget = 10mm + 2mm = 12mm.
        let result = clamp_with_dir(
            Position::from_millimeters(60),
            Velocity::from_meters_per_second(-1),
            Position::from_millimeters(50),
            Acceleration::from_meters_per_second_squared(5),
            -1,
            Position::from_millimeters(2),
        );
        // d_min = 1/(2*0.012) ≈ 41.7 m/s². Must clamp.
        assert!(result.0 > Acceleration::from_meters_per_second_squared(5).0);
        // But should NOT be a hard-stop value.
        assert!(result.0 < Acceleration::from_meters_per_second_squared(100).0);
    }
}
