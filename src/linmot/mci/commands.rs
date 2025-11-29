use crate::writer::{WireWrite, Writer};
use anyhow::Result;

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Command {
    #[default]
    NoOperation,
    VaiGoToPos {
        target_position: i32,
        maximal_velocity: u32,
        acceleration: u32,
        deceleration: u32,
    },
}

impl Command {
    #[must_use]
    pub const fn id(&self) -> u16 {
        match self {
            Self::NoOperation => 0x000,
            Self::VaiGoToPos { .. } => 0x010,
        }
    }

    pub(super) fn write_parameters(&self, w: &mut Writer) -> Result<()> {
        match self {
            Self::NoOperation => {}
            Self::VaiGoToPos { target_position, maximal_velocity, acceleration, deceleration } => {
                target_position.write_to(w)?;
                maximal_velocity.write_to(w)?;
                acceleration.write_to(w)?;
                deceleration.write_to(w)?;
            }
        }

        Ok(())
    }
}
