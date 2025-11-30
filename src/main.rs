use anyhow::{Context, Result, anyhow};
use clap::Parser;
use linmot::mci::units::{Acceleration, Position, Velocity};
use linmot::mci::{Command, ControlFlags, ErrorCode, MotionCommand, State};
use linmot::udp::{BUFFER_SIZE, CONTROLLER_PORT, DRIVE_PORT, Request, Response, ResponseFlags};
use std::net::{Ipv4Addr, UdpSocket};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub mod linmot;
mod reader;
mod writer;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Options {
    drive_address: String,
}

fn main() -> Result<()> {
    let options = Options::parse();

    DriveConnection::new(&options.drive_address)?.start_loop().context("Failed to connect to drive")?;

    Ok(())
}

struct DriveConnection {
    socket: UdpSocket,
    buffer: [u8; BUFFER_SIZE],
    last_response: Option<Response>,
    last_state: State,
    control_flags: ControlFlags,
    acknowledge_error: bool,
    pending_command_count: Option<u8>,
    pending_motion_command: Option<Command>,
}

impl DriveConnection {
    fn new(address: &str) -> Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, CONTROLLER_PORT))?;
        socket.connect((address, DRIVE_PORT))?;

        println!("Connected to drive at {:?} from {:?}", socket.peer_addr(), socket.local_addr());

        Ok(Self {
            socket,
            buffer: [0u8; BUFFER_SIZE],
            last_response: None,
            last_state: State::NotReadyToSwitchOn,
            control_flags: ControlFlags::SWITCH_ON,
            acknowledge_error: true,
            pending_command_count: None,
            pending_motion_command: Some(Command::VaiGoToPos {
                target_position: Position::from_millimeters(150),
                maximal_velocity: Velocity::from_meters_per_second(1),
                acceleration: Acceleration::from_meters_per_second_squared(1),
                deceleration: Acceleration::from_meters_per_second_squared(1),
            }),
        })
    }

    fn loop_tick(&mut self) -> Result<()> {
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
        if let Some(Response { state: Some(state), .. }) = &self.last_response {
            if state != &self.last_state {
                println!("Transitioned from {:?} to {:?}", self.last_state, state);
                self.last_state = *state;
            }

            match state {
                State::NotReadyToSwitchOn => {
                    self.control_flags = ControlFlags::empty();
                }
                State::ReadyToSwitchOn => {
                    self.acknowledge_error = false;
                    self.control_flags = ControlFlags::SWITCH_ON;
                }
                State::Error { error_code } if self.acknowledge_error => {
                    println!("Acknowledging error: {error_code:?}");

                    self.control_flags = ControlFlags::ERROR_ACKNOWLEDGE;
                }
                State::OperationEnabled { homed: false, .. } => {
                    self.control_flags.insert(ControlFlags::HOME);
                }
                State::OperationEnabled { motion_active: false, motion_command_count, .. } => {
                    if let Some(pending_count) = self.pending_command_count
                        && pending_count == *motion_command_count
                    {
                        println!("Motion command {pending_count} complete");

                        self.pending_command_count = None;
                        // self.pending_motion_command = None;

                        // TODO: Quick oscillating test.
                        self.pending_motion_command =
                            if let Some(Command::VaiGoToPos { target_position, .. }) = self.pending_motion_command {
                                let new_target_position = if target_position < Position::from_millimeters(150) {
                                    Position::from_millimeters(200)
                                } else {
                                    Position::from_millimeters(100)
                                };

                                Some(Command::VaiGoToPos {
                                    target_position: new_target_position,
                                    maximal_velocity: Velocity::from_meters_per_second(1),
                                    acceleration: Acceleration::from_meters_per_second_squared(1),
                                    deceleration: Acceleration::from_meters_per_second_squared(1),
                                })
                            } else {
                                None
                            }
                    }

                    if let Some(pending_command) = self.pending_motion_command {
                        // TODO: We had a case where the received command count was 0, but internally it had 1 stored, and it ignored our new command.
                        let next_command_count = (motion_command_count + 2) % 0xF;
                        self.pending_command_count = Some(next_command_count);

                        request.motion_command =
                            Some(MotionCommand { count: next_command_count, command: pending_command });

                        println!("Sending motion command {next_command_count}: {pending_command:?}");
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

    fn start_loop(&mut self) -> Result<()> {
        const LOOP_INTERVAL: Duration = Duration::from_millis(5);
        const REPORT_INTERVAL: Duration = Duration::from_secs(1);

        println!("Loop interval: {LOOP_INTERVAL:?}");

        self.socket.set_read_timeout(Some(LOOP_INTERVAL / 2))?;

        let mut last_loop_report = Instant::now();
        let mut loop_duration_sum = Duration::ZERO;
        let mut loop_message_count: u32 = 0;
        let mut loop_error_count: u32 = 0;

        let mut next_tick = Instant::now() + LOOP_INTERVAL;

        loop {
            let iter_start = Instant::now();

            if let Err(error) = self.loop_tick() {
                eprintln!("Error in loop tick: {error}");
                loop_error_count += 1;
            }

            loop_message_count += 1;

            let loop_duration = iter_start.elapsed();
            loop_duration_sum += loop_duration;

            if last_loop_report.elapsed() >= REPORT_INTERVAL {
                if loop_error_count > 0 && loop_error_count == loop_message_count {
                    break Err(anyhow!("Too many errors in loop, aborting"));
                }

                let avg_loop_duration = loop_duration_sum / loop_message_count;
                println!(
                    "Average loop duration: {:?} ({:.2}% usage, {}/{} errors)",
                    avg_loop_duration,
                    (avg_loop_duration.as_secs_f64() / LOOP_INTERVAL.as_secs_f64()) * 100.0,
                    loop_error_count,
                    loop_message_count,
                );

                self.print_drive_status();

                last_loop_report = Instant::now();
                loop_duration_sum = Duration::ZERO;
                loop_message_count = 0;
                loop_error_count = 0;
            }

            // Sleep until the next tick; if overrun, report lateness and realign to the next interval boundary
            let now = Instant::now();
            if let Some(remaining) = next_tick.checked_duration_since(now) {
                sleep(remaining);
                next_tick += LOOP_INTERVAL;
            } else {
                let late_by = now.duration_since(next_tick);
                eprintln!("Late by {late_by:?}");
                next_tick = now + LOOP_INTERVAL;
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
