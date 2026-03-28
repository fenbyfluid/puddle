use crate::messages::DriveState;
use mio::Token;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use units::{Acceleration, Current, DriveTemperature, MotorTemperature, Position, Velocity};

pub mod messages;

pub use linmot::mci::units;

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

    fn from_str(s: &str) -> Result<Self, anyhow::Error> {
        match s {
            "hid" => Ok(ControllerId::Hid),
            _ => {
                let id = s.strip_prefix("ws-").ok_or_else(|| anyhow::anyhow!("Invalid controller ID: {}", s))?;
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
