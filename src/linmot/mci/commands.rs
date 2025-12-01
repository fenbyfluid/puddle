use super::units::{Acceleration, Position, Velocity};
use crate::writer::{WireWrite, Writer};
use anyhow::Result;

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Command {
    #[default]
    NoOperation,
    VaiGoToPos {
        target_position: Position,
        maximal_velocity: Velocity,
        acceleration: Acceleration,
        deceleration: Acceleration,
    },
    VaiIncrementDemPos {
        position_increment: Position,
        maximal_velocity: Velocity,
        acceleration: Acceleration,
        deceleration: Acceleration,
    },
    VaiStop {
        deceleration: Acceleration,
    },
    PStreamWithDriveGeneratedTimeStamp {
        position: Position,
    },
    PvStreamWithDriveGeneratedTimeStamp {
        position: Position,
        velocity: Velocity,
    },
    PStreamWithDriveGeneratedTimeStampAndConfiguredPeriodTime {
        position: Position,
    },
    PvStreamWithDriveGeneratedTimeStampAndConfiguredPeriodTime {
        position: Position,
        velocity: Velocity,
    },
    PvaStreamWithDriveGeneratedTimeStamp {
        position: Position,
        velocity: Velocity,
        acceleration: Acceleration,
    },
    PvaStreamWithDriveGeneratedTimeStampAndConfiguredPeriodTime {
        position: Position,
        velocity: Velocity,
        acceleration: Acceleration,
    },
    PvaStreamWithControllerGeneratedTimeStamp {
        position: Position,
        velocity: Velocity,
        acceleration: Acceleration,
    },
    StopStream,
}

impl Command {
    #[must_use]
    pub const fn id(&self) -> u16 {
        match self {
            Self::NoOperation => 0x000,
            Self::VaiGoToPos { .. } => 0x010,
            Self::VaiIncrementDemPos { .. } => 0x011,
            Self::VaiStop { .. } => 0x017,
            Self::PStreamWithDriveGeneratedTimeStamp { .. } => 0x030,
            Self::PvStreamWithDriveGeneratedTimeStamp { .. } => 0x031,
            Self::PStreamWithDriveGeneratedTimeStampAndConfiguredPeriodTime { .. } => 0x032,
            Self::PvStreamWithDriveGeneratedTimeStampAndConfiguredPeriodTime { .. } => 0x033,
            Self::PvaStreamWithDriveGeneratedTimeStamp { .. } => 0x034,
            Self::PvaStreamWithDriveGeneratedTimeStampAndConfiguredPeriodTime { .. } => 0x035,
            Self::PvaStreamWithControllerGeneratedTimeStamp { .. } => 0x03A,
            Self::StopStream => 0x03F,
        }
    }

    pub(super) fn write_parameters(&self, w: &mut Writer) -> Result<()> {
        #[expect(clippy::match_same_arms)]
        match self {
            Self::NoOperation => {}
            Self::VaiGoToPos { target_position, maximal_velocity, acceleration, deceleration } => {
                target_position.write_to(w)?;
                maximal_velocity.write_to(w)?;
                acceleration.write_to(w)?;
                deceleration.write_to(w)?;
            }
            Self::VaiIncrementDemPos { position_increment, maximal_velocity, acceleration, deceleration } => {
                position_increment.write_to(w)?;
                maximal_velocity.write_to(w)?;
                acceleration.write_to(w)?;
                deceleration.write_to(w)?;
            }
            Self::VaiStop { deceleration } => {
                deceleration.write_to(w)?;
            }
            Self::PStreamWithDriveGeneratedTimeStamp { position } => {
                position.write_to(w)?;
            }
            Self::PvStreamWithDriveGeneratedTimeStamp { position, velocity } => {
                position.write_to(w)?;
                velocity.write_to(w)?;
            }
            Self::PStreamWithDriveGeneratedTimeStampAndConfiguredPeriodTime { position } => {
                position.write_to(w)?;
            }
            Self::PvStreamWithDriveGeneratedTimeStampAndConfiguredPeriodTime { position, velocity } => {
                position.write_to(w)?;
                velocity.write_to(w)?;
            }
            Self::PvaStreamWithDriveGeneratedTimeStamp { position, velocity, acceleration } => {
                position.write_to(w)?;
                velocity.write_to(w)?;
                acceleration.write_to(w)?;
            }
            Self::PvaStreamWithDriveGeneratedTimeStampAndConfiguredPeriodTime { position, velocity, acceleration } => {
                position.write_to(w)?;
                velocity.write_to(w)?;
                acceleration.write_to(w)?;
            }
            Self::PvaStreamWithControllerGeneratedTimeStamp { position, velocity, acceleration } => {
                position.write_to(w)?;
                velocity.write_to(w)?;
                acceleration.write_to(w)?;
            }
            Self::StopStream => {}
        }

        Ok(())
    }
}
