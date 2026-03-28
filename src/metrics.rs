use anyhow::{Result, anyhow};
use linmot::mci::units::{Acceleration, Current, DriveTemperature, MotorTemperature, Position, Velocity};
use linmot::mci::{Command, ControlFlags, MotionCommand, StatusFlags, WarningFlags};
use linmot::udp::{Request, Response};
use log::{trace, warn};
use questdb::ingress::{Buffer, Sender, TimestampMicros};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub struct RecordCommand {
    position: Position,
    velocity: Velocity,
    acceleration: Acceleration,
    deceleration: Acceleration,
}

pub struct Record {
    timestamp: TimestampMicros,
    processing_time: Duration,
    response_time: Duration,
    active_command_index: usize,
    command: Option<RecordCommand>,
    control_flags: ControlFlags,
    status_flags: StatusFlags,
    raw_state: u16,
    actual_position: Position,
    demand_position: Position,
    demand_velocity: Velocity,
    demand_acceleration: Acceleration,
    motor_current: Current,
    warning_flags: WarningFlags,
    raw_error_code: u16,
    drive_temperature: DriveTemperature,
    motor_temperature: MotorTemperature,
}

impl Record {
    pub fn new(
        loop_duration: Duration,
        last_rtt: Duration,
        active_command_index: usize,
        request: &Request,
        response: &Response,
    ) -> Result<Self> {
        let command = match request.motion_command {
            Some(MotionCommand {
                command: Command::VaiGoToPos { target_position, maximal_velocity, acceleration, deceleration },
                ..
            }) => Some(RecordCommand {
                position: target_position,
                velocity: maximal_velocity,
                acceleration,
                deceleration,
            }),
            _ => None,
        };

        let (demand_velocity, demand_acceleration, drive_temperature, motor_temperature) =
            response.monitoring_channel.ok_or_else(|| anyhow!("Missing monitoring channel in response"))?;

        Ok(Record {
            timestamp: TimestampMicros::now(),
            processing_time: loop_duration,
            response_time: last_rtt,
            active_command_index,
            command,
            control_flags: request.control_flags.ok_or_else(|| anyhow!("Missing control flags in request"))?,
            status_flags: response.status_flags.ok_or_else(|| anyhow!("Missing status flags in response"))?,
            raw_state: response.raw_state.ok_or_else(|| anyhow!("Missing state in response"))?,
            actual_position: response.actual_position.ok_or_else(|| anyhow!("Missing actual position in response"))?,
            demand_position: response.demand_position.ok_or_else(|| anyhow!("Missing demand position in response"))?,
            demand_velocity: Velocity(demand_velocity as i32),
            demand_acceleration: Acceleration(demand_acceleration as i32),
            motor_current: response.current.ok_or_else(|| anyhow!("Missing motor current in response"))?,
            warning_flags: response.warning_flags.ok_or_else(|| anyhow!("Missing warning flags in response"))?,
            raw_error_code: response.raw_error_code.ok_or_else(|| anyhow!("Missing error code in response"))?,
            drive_temperature: DriveTemperature(drive_temperature as i16),
            motor_temperature: MotorTemperature(motor_temperature as i16),
        })
    }

    fn add_to_buffer(&self, buffer: &mut Buffer) -> Result<()> {
        buffer.column_i64("processing_time", i64::try_from(self.processing_time.as_micros())?)?;
        buffer.column_i64("response_time", i64::try_from(self.response_time.as_micros())?)?;
        buffer.column_i64("active_command", i64::try_from(self.active_command_index)?)?;

        if let Some(command) = &self.command {
            buffer.column_i64("command_position", i64::from(command.position.0))?;
            buffer.column_i64("command_velocity", i64::from(command.velocity.0))?;
            buffer.column_i64("command_acceleration", i64::from(command.acceleration.0))?;
            buffer.column_i64("command_deceleration", i64::from(command.deceleration.0))?;
        }

        buffer.column_i64("control_flags", i64::from(self.control_flags.bits()))?;
        buffer.column_i64("status_flags", i64::from(self.status_flags.bits()))?;
        buffer.column_i64("state", i64::from(self.raw_state))?;
        buffer.column_i64("actual_position", i64::from(self.actual_position.0))?;
        buffer.column_i64("demand_position", i64::from(self.demand_position.0))?;
        buffer.column_i64("demand_velocity", i64::from(self.demand_velocity.0))?;
        buffer.column_i64("demand_acceleration", i64::from(self.demand_acceleration.0))?;
        buffer.column_i64("motor_current", i64::from(self.motor_current.0))?;
        buffer.column_i64("warning_flags", i64::from(self.warning_flags.bits()))?;
        buffer.column_i64("error_code", i64::from(self.raw_error_code))?;
        buffer.column_i64("drive_temperature", i64::from(self.drive_temperature.0))?;
        buffer.column_i64("motor_temperature", i64::from(self.motor_temperature.0))?;

        // Must be last, ends the record.
        buffer.at(self.timestamp)?;

        Ok(())
    }
}

pub struct MetricSender {
    pub sender: mpsc::Sender<Record>,
}

impl MetricSender {
    pub fn new(table_name: String, flush_limit: usize, flush_interval: Duration) -> Result<Self> {
        let mut sender = Sender::from_env()?;
        let mut buffer = sender.new_buffer();
        let mut last_flush = Instant::now();
        let (queue_tx, queue_rx) = mpsc::channel();

        std::thread::spawn(move || {
            loop {
                // TODO: Set to a lower priority?

                let mut tick = || -> Result<()> {
                    let record: Record = queue_rx.recv()?;

                    if sender.must_close() {
                        warn!("Restarting metrics sender due to fatal connection error");
                        sender = Sender::from_env()?;
                        buffer = sender.new_buffer();
                        last_flush = Instant::now();
                    }

                    buffer.table(table_name.as_str())?;
                    record.add_to_buffer(&mut buffer)?;

                    if buffer.row_count() > flush_limit || last_flush.elapsed() > flush_interval {
                        trace!("Flushing {} metrics entries after {:?}", buffer.row_count(), last_flush.elapsed());

                        sender.flush(&mut buffer)?;
                        last_flush = Instant::now();
                    }

                    Ok(())
                };

                if let Err(err) = tick() {
                    warn!("Metric loop error: {}", err);
                    continue;
                }
            }
        });

        Ok(Self { sender: queue_tx })
    }
}
