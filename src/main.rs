use crate::drive::{ACTION_ACK_ERROR, ACTION_RESET_INDEX, DriveFeedback};
use crate::messages::{
    AckFailureReason, ClientMessage, CoreMessage, DriveState, MotionAction, MotionCommand, SavedSetMetadata,
};
use anyhow::{Context, Result, anyhow};
use clap::Parser;
use linmot::mci::ErrorCode;
use linmot::mci::units::{Acceleration, Current, DriveTemperature, MotorTemperature, Position, Velocity};
use log::{trace, warn};
use mio::Token;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::mpsc;
use std::time::Duration;

mod drive;
#[cfg(feature = "hidapi")]
mod hid;
mod messages;
#[cfg(feature = "questdb-rs")]
mod metrics;
#[cfg(feature = "tungstenite")]
mod websocket;

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
    /// USB remote controller PID
    #[clap(long, value_parser=from_hex, default_value = "8354")]
    usb_pid: u16,
    /// Enable WebSocket server
    #[clap(short = 'w', long)]
    enable_websocket: bool,
    /// WebSocket server listen port
    #[clap(short = 'p', long, default_value = "8080")]
    websocket_port: u16,
    /// Stroke limit in millimeters
    #[clap(short, long, default_value = "360.0")]
    stroke_limit: f64,
    /// Velocity limit in meters per second
    #[clap(short, long, default_value = "2.5")]
    velocity_limit: f64,
    /// Acceleration limit in meters per second squared
    #[clap(short, long, default_value = "15.0")]
    acceleration_limit: f64,
    /// Drive loop interval in milliseconds
    #[clap(short, long, default_value = "5")]
    loop_interval: u64,
    /// Metrics table name
    #[clap(long, default_value = "puddle_stats")]
    stats_table: String,
    /// Metrics buffer count limit
    #[clap(long, default_value = "75000")]
    stats_limit: usize,
    /// Metrics buffer time in seconds
    #[clap(short = 's', long, default_value = "5")]
    stats_interval: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemLimits {
    pub position: Position,
    pub velocity: Velocity,
    pub acceleration: Acceleration,
    pub deceleration: Acceleration,
}

/// Controller identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControllerId {
    Hid,
    WebSocket(Token),
}

impl std::fmt::Display for ControllerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ControllerId::Hid => write!(f, "hid"),
            ControllerId::WebSocket(token) => write!(f, "ws-{}", token.0),
        }
    }
}

impl FromStr for ControllerId {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> anyhow::Result<Self> {
        match s {
            "hid" => Ok(ControllerId::Hid),
            _ => {
                let id = s.strip_prefix("ws-").ok_or_else(|| anyhow!("Invalid controller ID: {}", s))?;
                Ok(ControllerId::WebSocket(Token(id.parse()?)))
            }
        }
    }
}

impl serde::Serialize for ControllerId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.to_string().as_str())
    }
}

impl<'de> serde::Deserialize<'de> for ControllerId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        ControllerId::from_str(s.as_str())
            .map_err(|_| serde::de::Error::invalid_value(serde::de::Unexpected::Str(&s), &"a controller ID"))
    }
}

// TODO: See the comment on drive::DriveFeedback, think about trimming this down.
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoreState {
    pub drive_state: DriveState,
    pub active_command_index: usize,
    pub actual_position: Position,
    pub demand_position: Position,
    pub demand_velocity: Velocity,
    pub demand_acceleration: Acceleration,
    pub current_draw: Current,
    pub drive_temperature: DriveTemperature,
    pub motor_temperature: MotorTemperature,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub command_set_version: u64,
    pub write_access_holder: Option<ControllerId>,
}

/// Every inbound event the core processes, from any source.
#[derive(Debug)]
pub enum CoreEvent {
    // From WebSocket or HID threads
    Connected { controller_id: ControllerId },
    Disconnected { controller_id: ControllerId },
    Message { controller_id: ControllerId, message: ClientMessage },

    // From drive thread
    DriveStateUpdated(DriveFeedback),
}

struct CoreManager {
    limits: SystemLimits,
    drive: drive::ConnectionManager,
    hid_controller: Option<hid::Controller>,
    websocket_server: Option<websocket::Server>,
    core_state: CoreState,
    active_command_set: (u64, Vec<MotionCommand>),
    // TODO: This will be database-backed in the future
    saved_command_sets: HashMap<String, (u64, Vec<MotionCommand>)>,
}

impl CoreManager {
    fn new(
        limits: SystemLimits,
        drive: drive::ConnectionManager,
        hid_controller: Option<hid::Controller>,
        websocket_server: Option<websocket::Server>,
    ) -> Self {
        Self {
            limits,
            drive,
            hid_controller,
            websocket_server,
            core_state: CoreState::default(),
            active_command_set: (0, Vec::new()),
            saved_command_sets: HashMap::new(),
        }
    }

    fn send(&self, destination: Option<ControllerId>, message: CoreMessage) -> Result<()> {
        trace!("Sending message to {:?}: {:?}", destination, message);

        match destination {
            Some(ControllerId::Hid) => {
                if let Some(hid_controller) = &self.hid_controller {
                    hid_controller.handle_message(message)
                } else {
                    Err(anyhow!("HID controller not enabled"))
                }
            }
            Some(ControllerId::WebSocket(token)) => {
                if let Some(websocket_server) = &self.websocket_server {
                    websocket_server.send(Some(token), message)
                } else {
                    Err(anyhow!("WebSocket server not enabled"))
                }
            }
            None => {
                if let Some(hid_controller) = &self.hid_controller {
                    hid_controller.handle_message(message.clone())?;
                }
                if let Some(websocket_server) = &self.websocket_server {
                    websocket_server.send(None, message)?;
                }
                Ok(())
            }
        }
    }

    fn handle_event(&mut self, event: CoreEvent) -> Result<()> {
        trace!("Received core event: {:?}", event);

        match event {
            CoreEvent::Connected { controller_id } => self.send(
                Some(controller_id),
                CoreMessage::Connected { controller_id, limits: self.limits.clone(), state: self.core_state.clone() },
            ),
            CoreEvent::Disconnected { controller_id } => {
                if self.core_state.write_access_holder == Some(controller_id) {
                    self.core_state.write_access_holder = None;

                    {
                        let mut commands = self.drive.interface.commands.lock().unwrap();
                        commands.motion_enabled = false;
                    }

                    self.send(
                        None,
                        CoreMessage::WriteAccessChanged { holder: None, previous_holder: Some(controller_id) },
                    )
                } else {
                    // Nothing to do if not the write access holder.
                    Ok(())
                }
            }
            CoreEvent::Message { controller_id, message } => self.handle_message(controller_id, message),
            CoreEvent::DriveStateUpdated(feedback) => {
                self.core_state.drive_state = feedback.drive_state;
                self.core_state.active_command_index = feedback.active_command_index;
                self.core_state.actual_position = feedback.actual_position;
                self.core_state.demand_position = feedback.demand_position;
                self.core_state.demand_velocity = feedback.demand_velocity;
                self.core_state.demand_acceleration = feedback.demand_acceleration;
                self.core_state.current_draw = feedback.current_draw;
                self.core_state.drive_temperature = feedback.drive_temperature;
                self.core_state.motor_temperature = feedback.motor_temperature;
                self.core_state.warnings =
                    feedback.warning_flags.iter_names().map(|(name, _)| name.to_owned()).collect();
                self.core_state.error_code = match feedback.error_code {
                    ErrorCode::NoError => None,
                    // TODO: Format this correctly
                    error_code => Some(format!("{:?}", error_code)),
                };

                self.send(None, CoreMessage::State { seq: None, state: self.core_state.clone() })
            }
        }
    }

    fn handle_message(&mut self, controller_id: ControllerId, message: ClientMessage) -> Result<()> {
        match message {
            ClientMessage::RequestWriteAccess { seq } => {
                let can_take_write_access = self.core_state.drive_state == DriveState::PowerOff
                    || self.core_state.write_access_holder.is_none()
                    || controller_id == ControllerId::Hid;

                if can_take_write_access {
                    let previous_holder = self.core_state.write_access_holder.replace(controller_id);
                    if previous_holder != Some(controller_id) {
                        self.send(
                            None,
                            CoreMessage::WriteAccessChanged { holder: Some(controller_id), previous_holder },
                        )?;
                    }
                    self.send(
                        Some(controller_id),
                        CoreMessage::WriteAccessResult { seq, granted: true, holder: Some(controller_id) },
                    )
                } else {
                    self.send(
                        Some(controller_id),
                        CoreMessage::WriteAccessResult {
                            seq,
                            granted: false,
                            holder: self.core_state.write_access_holder,
                        },
                    )
                }
            }
            ClientMessage::ReleaseWriteAccess { seq } => {
                if self.core_state.write_access_holder == Some(controller_id) {
                    self.core_state.write_access_holder = None;
                    self.send(
                        None,
                        CoreMessage::WriteAccessChanged { holder: None, previous_holder: Some(controller_id) },
                    )?;
                    self.send(Some(controller_id), CoreMessage::Ack { seq, success: true, reason: None })
                } else {
                    self.send(
                        Some(controller_id),
                        CoreMessage::Ack { seq, success: false, reason: Some(AckFailureReason::NotWriter) },
                    )
                }
            }
            ClientMessage::GetState { seq } => {
                self.send(Some(controller_id), CoreMessage::State { seq: Some(seq), state: self.core_state.clone() })
            }
            ClientMessage::GetCommandSet { seq, set } => {
                if let Some(set_name) = &set {
                    if let Some((version, commands)) = self.saved_command_sets.get(set_name) {
                        self.send(
                            Some(controller_id),
                            CoreMessage::CommandSet { seq, set, version: *version, commands: commands.clone() },
                        )
                    } else {
                        self.send(
                            Some(controller_id),
                            CoreMessage::Ack { seq, success: false, reason: Some(AckFailureReason::NotFound) },
                        )
                    }
                } else {
                    let (version, commands) = self.active_command_set.clone();
                    self.send(Some(controller_id), CoreMessage::CommandSet { seq, set, version, commands })
                }
            }
            ClientMessage::UpsertCommandSet { seq, set, base_version, commands: new_commands } => {
                if let Some(set_name) = &set {
                    if let Some((version, commands)) = self.saved_command_sets.get_mut(set_name) {
                        if base_version.is_none() || base_version == Some(*version) {
                            *version += 1;
                            *commands = new_commands;
                            let version = *version;
                            self.send(Some(controller_id), CoreMessage::CommandResult { seq, success: true, version })
                        } else {
                            let version = *version;
                            self.send(Some(controller_id), CoreMessage::CommandResult { seq, success: false, version })
                        }
                    } else {
                        self.saved_command_sets.insert(set_name.clone(), (1, new_commands.clone()));

                        self.send(Some(controller_id), CoreMessage::CommandResult { seq, success: true, version: 1 })
                    }
                } else if self.core_state.write_access_holder == Some(controller_id) {
                    if base_version.is_none() || base_version == Some(self.active_command_set.0) {
                        self.active_command_set = (self.active_command_set.0 + 1, new_commands);

                        self.sync_commands_to_drive();

                        self.drive.interface.actions.send(ACTION_RESET_INDEX);

                        self.send(
                            None,
                            CoreMessage::CommandSetChanged { version: self.active_command_set.0, update: None },
                        )?;

                        self.send(
                            Some(controller_id),
                            CoreMessage::CommandResult { seq, success: true, version: self.active_command_set.0 },
                        )
                    } else {
                        self.send(
                            Some(controller_id),
                            CoreMessage::CommandResult { seq, success: false, version: self.active_command_set.0 },
                        )
                    }
                } else {
                    self.send(
                        Some(controller_id),
                        CoreMessage::Ack { seq, success: false, reason: Some(AckFailureReason::NotWriter) },
                    )
                }
            }
            ClientMessage::UpdateCommand { seq, update } => {
                if self.core_state.write_access_holder == Some(controller_id) {
                    match self.active_command_set.1.get_mut(update.index) {
                        Some(command) => {
                            if command.apply_fields(&update.fields) {
                                self.active_command_set.0 += 1;

                                self.sync_commands_to_drive();

                                self.send(
                                    None,
                                    CoreMessage::CommandSetChanged {
                                        version: self.active_command_set.0,
                                        update: Some(update),
                                    },
                                )?;
                            }

                            self.send(
                                Some(controller_id),
                                CoreMessage::CommandResult { seq, success: true, version: self.active_command_set.0 },
                            )
                        }
                        None => self.send(
                            Some(controller_id),
                            CoreMessage::Ack { seq, success: false, reason: Some(AckFailureReason::OutOfRange) },
                        ),
                    }
                } else {
                    self.send(
                        Some(controller_id),
                        CoreMessage::Ack { seq, success: false, reason: Some(AckFailureReason::NotWriter) },
                    )
                }
            }
            ClientMessage::DeleteCommandSet { seq, set, base_version } => {
                if let Some(set_name) = &set {
                    if let Some((version, _commands)) = self.saved_command_sets.get(set_name) {
                        if base_version.is_none() || base_version == Some(*version) {
                            self.saved_command_sets.remove(set_name);

                            self.send(Some(controller_id), CoreMessage::Ack { seq, success: true, reason: None })
                        } else {
                            self.send(
                                Some(controller_id),
                                CoreMessage::CommandResult { seq, success: false, version: *version },
                            )
                        }
                    } else {
                        self.send(
                            Some(controller_id),
                            CoreMessage::Ack { seq, success: false, reason: Some(AckFailureReason::NotFound) },
                        )
                    }
                } else if self.core_state.write_access_holder == Some(controller_id) {
                    if base_version.is_none() || base_version == Some(self.active_command_set.0) {
                        self.active_command_set = (self.active_command_set.0 + 1, Vec::new());

                        self.sync_commands_to_drive();

                        self.drive.interface.actions.send(ACTION_RESET_INDEX);

                        self.send(
                            None,
                            CoreMessage::CommandSetChanged { version: self.active_command_set.0, update: None },
                        )?;

                        self.send(
                            Some(controller_id),
                            CoreMessage::CommandResult { seq, success: true, version: self.active_command_set.0 },
                        )
                    } else {
                        self.send(
                            Some(controller_id),
                            CoreMessage::CommandResult { seq, success: false, version: self.active_command_set.0 },
                        )
                    }
                } else {
                    self.send(
                        Some(controller_id),
                        CoreMessage::Ack { seq, success: false, reason: Some(AckFailureReason::NotWriter) },
                    )
                }
            }
            ClientMessage::ListSavedSets { seq } => {
                self.send(
                    Some(controller_id),
                    CoreMessage::SavedSetList {
                        seq,
                        sets: self
                            .saved_command_sets
                            .iter()
                            .map(|(name, (version, _commands))| SavedSetMetadata {
                                name: name.clone(),
                                version: *version,
                                saved_at: "".to_string(), // TODO
                            })
                            .collect(),
                    },
                )
            }
            ClientMessage::SetDrivePower { seq, enabled } => {
                if self.core_state.write_access_holder == Some(controller_id) {
                    {
                        let mut commands = self.drive.interface.commands.lock().unwrap();
                        commands.power_enabled = enabled;
                    }

                    self.send(Some(controller_id), CoreMessage::Ack { seq, success: true, reason: None })
                } else {
                    self.send(
                        Some(controller_id),
                        CoreMessage::Ack { seq, success: false, reason: Some(AckFailureReason::NotWriter) },
                    )
                }
            }
            ClientMessage::SetMotionState { seq, action } => {
                if self.core_state.write_access_holder == Some(controller_id) {
                    let valid = match (action, self.core_state.drive_state) {
                        (MotionAction::Start, DriveState::Paused) => true,
                        (MotionAction::Pause, DriveState::Moving) => true,
                        (MotionAction::Resume, DriveState::Paused) => true,
                        (MotionAction::Stop, DriveState::Moving | DriveState::Paused) => true,
                        _ => false,
                    };

                    if !valid {
                        return self.send(
                            Some(controller_id),
                            CoreMessage::Ack { seq, success: false, reason: Some(AckFailureReason::InvalidState) },
                        );
                    }

                    {
                        let mut commands = self.drive.interface.commands.lock().unwrap();
                        commands.motion_enabled = match action {
                            MotionAction::Start | MotionAction::Resume => true,
                            MotionAction::Stop | MotionAction::Pause => false,
                        };
                    }

                    if action == MotionAction::Start || action == MotionAction::Stop {
                        self.drive.interface.actions.send(ACTION_RESET_INDEX);
                    }

                    self.send(Some(controller_id), CoreMessage::Ack { seq, success: true, reason: None })
                } else {
                    self.send(
                        Some(controller_id),
                        CoreMessage::Ack { seq, success: false, reason: Some(AckFailureReason::NotWriter) },
                    )
                }
            }
            ClientMessage::AcknowledgeError { seq } => {
                if self.core_state.write_access_holder == Some(controller_id) {
                    if self.core_state.drive_state != DriveState::Errored {
                        return self.send(
                            Some(controller_id),
                            CoreMessage::Ack { seq, success: false, reason: Some(AckFailureReason::InvalidState) },
                        );
                    }

                    self.drive.interface.actions.send(ACTION_ACK_ERROR);

                    self.send(Some(controller_id), CoreMessage::Ack { seq, success: true, reason: None })
                } else {
                    self.send(
                        Some(controller_id),
                        CoreMessage::Ack { seq, success: false, reason: Some(AckFailureReason::NotWriter) },
                    )
                }
            }
        }
    }

    fn sync_commands_to_drive(&mut self) {
        self.core_state.command_set_version = self.active_command_set.0;

        let mut commands = self.drive.interface.commands.lock().unwrap();
        commands.commands.clear();
        commands.commands.extend(self.active_command_set.1.iter().map(|c| MotionCommand {
            position: c.position.clamp(Position::default(), self.limits.position),
            velocity: c.velocity.clamp(Velocity::default(), self.limits.velocity),
            acceleration: c.acceleration.clamp(Acceleration::default(), self.limits.acceleration),
            deceleration: c.deceleration.clamp(Acceleration::default(), self.limits.deceleration),
        }));
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let options = Options::parse();

    let (core_sender, core_receiver) = mpsc::channel();

    let hid_controller = if options.enable_usb {
        Some(hid::Controller::new(options.usb_vid, options.usb_pid, core_sender.clone())?)
    } else {
        None
    };

    let websocket_server = if options.enable_websocket {
        Some(websocket::Server::new(options.websocket_port, core_sender.clone())?)
    } else {
        None
    };

    if hid_controller.is_none() && websocket_server.is_none() {
        return Err(anyhow!("No controller interfaces enabled"));
    }

    let limits = SystemLimits {
        position: Position::from_millimeters_f64(options.stroke_limit),
        velocity: Velocity::from_meters_per_second_f64(options.velocity_limit),
        acceleration: Acceleration::from_meters_per_second_squared_f64(options.acceleration_limit),
        deceleration: Acceleration::from_meters_per_second_squared_f64(options.acceleration_limit),
    };

    let metrics = match metrics::MetricSender::new(
        options.stats_table,
        options.stats_limit,
        Duration::from_secs(options.stats_interval),
    ) {
        Ok(metrics) => Some(metrics),
        Err(e) => {
            warn!("Metrics reporting disabled: {}", e);
            None
        }
    };

    let drive = drive::ConnectionManager::new(
        options.drive_address,
        Duration::from_millis(options.loop_interval),
        core_sender.clone(),
        metrics.map(|m| m.sender.clone()),
    );

    let mut core_manager = CoreManager::new(limits, drive, hid_controller, websocket_server);

    loop {
        let message = match core_receiver.recv() {
            Ok(message) => message,
            Err(_) => break,
        };

        core_manager.handle_event(message)?;
    }

    Ok(())
}
