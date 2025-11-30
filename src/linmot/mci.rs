use crate::reader::{Reader, WireRead};
use crate::writer::{WireWrite, Writer};
use anyhow::{Result, anyhow};
use bitflags::bitflags;

mod commands;
pub mod units;

pub use commands::Command;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct ControlFlags: u16 {
        const SWITCH_ON = 1 << 0;
        const VOLTAGE_ENABLE = 1 << 1;
        const QUICK_STOP_DISABLE = 1 << 2;
        const ENABLE_OPERATION = 1 << 3;
        const ABORT_DISABLE = 1 << 4;
        const FREEZE_DISABLE = 1 << 5;
        const GO_TO_POSITION = 1 << 6;
        const ERROR_ACKNOWLEDGE = 1 << 7;
        const JOG_MOVE_POSITIVE = 1 << 8;
        const JOG_MOVE_NEGATIVE = 1 << 9;
        const SPECIAL_MODE = 1 << 10;
        const HOME = 1 << 11;
        const CLEARANCE_CHECK = 1 << 12;
        const GO_TO_INITIAL_POSITION = 1 << 13;
        const _RESERVED_14 = 1 << 14;
        const PHASE_SEARCH = 1 << 15;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct StatusFlags: u16 {
        const OPERATION_ENABLED = 1 << 0;
        const SWITCH_ON_ACTIVE = 1 << 1;
        const ENABLE_OPERATION = 1 << 2;
        const ERROR = 1 << 3;
        const VOLTAGE_ENABLE = 1 << 4;
        const QUICK_STOP_DISABLE = 1 << 5;
        const SWITCH_ON_LOCKED = 1 << 6;
        const WARNING = 1 << 7;
        const EVENT_HANDLER_ACTIVE = 1 << 8;
        const SPECIAL_MOTION_ACTIVE = 1 << 9;
        const IN_TARGET_POSITION = 1 << 10;
        const HOMED = 1 << 11;
        const FATAL_ERROR = 1 << 12;
        const MOTION_ACTIVE = 1 << 13;
        const RANGE_INDICATOR_1 = 1 << 14;
        const RANGE_INDICATOR_2 = 1 << 15;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct WarningFlags: u16 {
        const MOTOR_HOT_SENSOR = 1 << 0;
        const MOTOR_SHORT_TIME_OVERLOAD = 1 << 1;
        const MOTOR_SUPPLY_VOLTAGE_LOW = 1 << 2;
        const MOTOR_SUPPLY_VOLTAGE_HIGH = 1 << 3;
        const POSITION_LAG_ALWAYS = 1 << 4;
        const _RESERVED_5 = 1 << 5;
        const DRIVE_HOT = 1 << 6;
        const MOTOR_NOT_HOMED = 1 << 7;
        const PTC_SENSOR_1_HOT = 1 << 8;
        const PTC_SENSOR_2_HOT = 1 << 9;
        const REGENERATIVE_TEMP_OVERLOAD = 1 << 10;
        const SPEED_LAG_ALWAYS = 1 << 11;
        const POSITION_SENSOR = 1 << 12;
        const _RESERVED_13 = 1 << 13;
        const INTERFACE_WARN_FLAG = 1 << 14;
        const APPLICATION_WARN_FLAG = 1 << 15;
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ErrorCode {
    #[default]
    NoError,
    LogicSupplyTooLow,
    LogicSupplyTooHigh,
    MotorSupplyTooLow,
    MotorSupplyTooHigh,
    MinPositionUndershot,
    MaxPositionOvershot,
    PositionLagAlwaysTooBig,
    MotorHotSensor,
    MotorSliderMissing,
    MotorShortTimeOverload,
    MotorCommunicationLost,
    NotHomed,
    UnknownMotionCommand,
    PvtBufferOverflow,
    PvtBufferUnderflow,
    PvtControllerTooFast,
    PvtControllerTooSlow,
    MotionCommandInWrongState,
    Unknown(u16),
}

impl From<u16> for ErrorCode {
    fn from(e: u16) -> Self {
        match e {
            0x00 => Self::NoError,
            0x01 => Self::LogicSupplyTooLow,
            0x02 => Self::LogicSupplyTooHigh,
            0x03 => Self::MotorSupplyTooLow,
            0x04 => Self::MotorSupplyTooHigh,
            0x07 => Self::MinPositionUndershot,
            0x08 => Self::MaxPositionOvershot,
            0x0B => Self::PositionLagAlwaysTooBig,
            0x20 => Self::MotorHotSensor,
            0x22 => Self::MotorSliderMissing,
            0x23 => Self::MotorShortTimeOverload,
            0x45 => Self::MotorCommunicationLost,
            0x80 => Self::NotHomed,
            0x81 => Self::UnknownMotionCommand,
            0x82 => Self::PvtBufferOverflow,
            0x83 => Self::PvtBufferUnderflow,
            0x84 => Self::PvtControllerTooFast,
            0x85 => Self::PvtControllerTooSlow,
            0x86 => Self::MotionCommandInWrongState,
            _ => Self::Unknown(e),
        }
    }
}

impl From<u8> for ErrorCode {
    fn from(e: u8) -> Self {
        Self::from(u16::from(e))
    }
}

impl WireRead for ErrorCode {
    fn read_from(r: &mut Reader) -> Result<Self> {
        Ok(u16::read_from(r)?.into())
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    NotReadyToSwitchOn,
    SwitchOnDisabled,
    ReadyToSwitchOn,
    SetupError {
        error_code: ErrorCode,
    },
    Error {
        error_code: ErrorCode,
    },
    HardwareTests,
    ReadyToOperate,
    OperationEnabled {
        motion_command_count: u8,
        event_handler: bool,
        motion_active: bool,
        in_target_position: bool,
        homed: bool,
    },
    Homing {
        finished: bool,
    },
    ClearanceCheck {
        finished: bool,
    },
    GoingToInitialPosition {
        finished: bool,
    },
    Aborting,
    Freezing,
    QuickStop,
    GoingToPosition {
        finished: bool,
    },
    JoggingPositive {
        finished: bool,
    },
    JoggingNegative {
        finished: bool,
    },
    Linearizing,
    PhaseSearch,
    SpecialMode,
    BrakeDelay,
    Unknown {
        main_state: u8,
        sub_state: u8,
    },
}

impl WireRead for State {
    fn read_from(r: &mut Reader) -> Result<Self> {
        let sub_state = u8::read_from(r)?;
        let main_state = u8::read_from(r)?;

        Ok(match main_state {
            0 => Self::NotReadyToSwitchOn,
            1 => Self::SwitchOnDisabled,
            2 => Self::ReadyToSwitchOn,
            3 => Self::SetupError { error_code: sub_state.into() },
            4 => Self::Error { error_code: sub_state.into() },
            5 => Self::HardwareTests,
            6 => Self::ReadyToOperate,
            8 => Self::OperationEnabled {
                motion_command_count: sub_state & 0b1111,
                event_handler: (sub_state & (1 << 4)) != 0,
                motion_active: (sub_state & (1 << 5)) != 0,
                in_target_position: (sub_state & (1 << 6)) != 0,
                homed: (sub_state & (1 << 7)) != 0,
            },
            9 => Self::Homing { finished: sub_state == 0xF },
            10 => Self::ClearanceCheck { finished: sub_state == 0xF },
            11 => Self::GoingToInitialPosition { finished: sub_state == 0xF },
            12 => Self::Aborting,
            13 => Self::Freezing,
            14 => Self::QuickStop,
            15 => Self::GoingToPosition { finished: sub_state == 0xF },
            16 => Self::JoggingPositive { finished: sub_state == 0xF },
            17 => Self::JoggingNegative { finished: sub_state == 0xF },
            18 => Self::Linearizing,
            19 => Self::PhaseSearch,
            20 => Self::SpecialMode,
            21 => Self::BrakeDelay,
            _ => Self::Unknown { main_state, sub_state },
        })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MotionCommand {
    pub count: u8,
    pub command: Command,
}

impl WireWrite for MotionCommand {
    fn write_to(&self, w: &mut Writer) -> Result<()> {
        let header = (self.command.id() << 4) | u16::from(self.count & 0xF);

        let before = w.pos();

        header.write_to(w)?;
        self.command.write_parameters(w)?;

        // Header + parameters must fit into 32 bytes
        let len = w.pos() - before;
        if len > 32 {
            return Err(anyhow!("motion command parameters too large: {len} bytes (max 32)"));
        }

        // Pad the remainder of the 32-byte command block with zeros
        let pad_len = 32 - len;
        if pad_len > 0 {
            let zeros = [0u8; 32];
            w.write_bytes(&zeros[..pad_len])?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{units::*, *};

    #[test]
    fn test_motion_command_write_to() {
        let command = MotionCommand {
            count: 0,
            command: Command::VaiGoToPos {
                target_position: Position::from_millimeters(10),
                maximal_velocity: Velocity::from_meters_per_second(1),
                acceleration: Acceleration::from_meters_per_second_squared(10),
                deceleration: Acceleration::from_meters_per_second_squared(10),
            },
        };

        let mut buffer = [0u8; 32];
        let mut writer = Writer::new(&mut buffer);

        command.write_to(&mut writer).unwrap();

        // These expected bytes were taken from the LinUDP documentation
        assert_eq!(
            &buffer[..],
            &[
                0x00, 0x01, 0xA0, 0x86, 0x01, 0x00, 0x40, 0x42, 0x0F, 0x00, 0x40, 0x42, 0x0F, 0x00, 0x40, 0x42, 0x0F,
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00
            ]
        );
    }
}
