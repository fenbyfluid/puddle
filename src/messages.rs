use crate::{ControllerId, CoreState, SystemLimits};
use linmot::mci::units::{Acceleration, Position, Velocity};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Core domain types
// ---------------------------------------------------------------------------

/// Drive state machine states.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveState {
    #[default]
    Disconnected,
    PowerOff,
    Preparing,
    Paused,
    Moving,
    Errored,
}

/// A single motion command in a command set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MotionCommand {
    pub position: Position,
    pub velocity: Velocity,
    pub acceleration: Acceleration,
    pub deceleration: Acceleration,
}

impl MotionCommand {
    pub fn apply_fields(&mut self, fields: &MotionCommandFields) -> bool {
        let mut changed = false;
        if let Some(position) = fields.position {
            if self.position != position {
                self.position = position;
                changed = true;
            }
        }
        if let Some(velocity) = fields.velocity {
            if self.velocity != velocity {
                self.velocity = velocity;
                changed = true;
            }
        }
        if let Some(acceleration) = fields.acceleration {
            if self.acceleration != acceleration {
                self.acceleration = acceleration;
                changed = true;
            }
        }
        if let Some(deceleration) = fields.deceleration {
            if self.deceleration != deceleration {
                self.deceleration = deceleration;
                changed = true;
            }
        }
        changed
    }
}

/// Fields of a motion command that may be partially updated.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MotionCommandFields {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Position>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub velocity: Option<Velocity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub acceleration: Option<Acceleration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deceleration: Option<Acceleration>,
}

/// Metadata for a saved command set, as returned in listings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSetMetadata {
    pub name: String,
    pub version: u64,
    pub saved_at: String,
}

/// Identifies the target command set for operations.
///
/// `None` refers to the active command set (requires writer for mutations).
/// `Some(name)` refers to a saved command set by name.
pub type CommandSetId = Option<String>;

/// Motion control action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MotionAction {
    Start,
    Pause,
    Resume,
    Stop,
}

/// Describes what changed in the active command set, for delta broadcasts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandUpdate {
    pub index: usize,
    pub fields: MotionCommandFields,
}

// ---------------------------------------------------------------------------
// Client → Core messages
// ---------------------------------------------------------------------------

/// All messages that a client (WebSocket or internal HID UI manager) can
/// send to the core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    RequestWriteAccess {
        seq: u64,
    },
    ReleaseWriteAccess {
        seq: u64,
    },
    GetState {
        seq: u64,
    },
    GetCommandSet {
        seq: u64,
        set: CommandSetId,
    },
    UpsertCommandSet {
        seq: u64,
        set: CommandSetId,
        #[serde(skip_serializing_if = "Option::is_none")]
        base_version: Option<u64>,
        commands: Vec<MotionCommand>,
    },
    UpdateCommand {
        seq: u64,
        #[serde(flatten)]
        update: CommandUpdate,
    },
    DeleteCommandSet {
        seq: u64,
        set: CommandSetId,
        #[serde(skip_serializing_if = "Option::is_none")]
        base_version: Option<u64>,
    },
    ListSavedSets {
        seq: u64,
    },
    SetDrivePower {
        seq: u64,
        enabled: bool,
    },
    SetMotionState {
        seq: u64,
        action: MotionAction,
    },
    AcknowledgeError {
        seq: u64,
    },
}

// ---------------------------------------------------------------------------
// Core → Client messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AckFailureReason {
    NotWriter,
    NotFound,
    OutOfRange,
    InvalidState,
}

/// All messages that the core can send to a client.
///
/// Messages with `seq: Option<u64>` use `Some(n)` when responding to a
/// client request (echoing the client's seq) and `None` when broadcast.
/// Serde serializes `None` by omitting the field, matching the protocol
/// convention that broadcasts have no `seq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CoreMessage {
    /// Sent on WebSocket connection open.
    Connected { controller_id: ControllerId, limits: SystemLimits, state: CoreState },

    /// Generic success/failure response.
    Ack {
        seq: u64,
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<AckFailureReason>,
    },

    /// Response to write access request.
    WriteAccessResult { seq: u64, granted: bool, holder: Option<ControllerId> },

    /// Command set contents (response to get_command_set).
    CommandSet { seq: u64, set: CommandSetId, version: u64, commands: Vec<MotionCommand> },

    /// Result of a command set mutation or set-level operation.
    CommandResult { seq: u64, success: bool, version: u64 },

    /// List of saved command sets.
    SavedSetList { seq: u64, sets: Vec<SavedSetMetadata> },

    /// Real-time system state and telemetry.
    ///
    /// Used both as a response to `get_state` (with seq) and as a
    /// continuous broadcast (without seq).
    State {
        #[serde(skip_serializing_if = "Option::is_none")]
        seq: Option<u64>,
        #[serde(flatten)]
        state: CoreState,
    },

    /// Broadcast: active command set was modified.
    CommandSetChanged {
        version: u64,
        #[serde(flatten, skip_serializing_if = "Option::is_none")]
        update: Option<CommandUpdate>,
    },

    /// Broadcast: designated writer changed.
    WriteAccessChanged { holder: Option<ControllerId>, previous_holder: Option<ControllerId> },
}
