use crate::udp::writer::{WireWrite, WriteError, Writer};
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

impl std::fmt::Display for WarningFlags {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let names: Vec<_> = self
            .iter()
            .map(|flag| match flag {
                WarningFlags::MOTOR_HOT_SENSOR => "Motor hot sensor".to_owned(),
                WarningFlags::MOTOR_SHORT_TIME_OVERLOAD => "Motor short time overload".to_owned(),
                WarningFlags::MOTOR_SUPPLY_VOLTAGE_LOW => "Motor supply voltage low".to_owned(),
                WarningFlags::MOTOR_SUPPLY_VOLTAGE_HIGH => "Motor supply voltage high".to_owned(),
                WarningFlags::POSITION_LAG_ALWAYS => "Position lag always".to_owned(),
                WarningFlags::DRIVE_HOT => "Drive hot".to_owned(),
                WarningFlags::MOTOR_NOT_HOMED => "Motor not homed".to_owned(),
                WarningFlags::PTC_SENSOR_1_HOT => "PTC sensor 1 hot".to_owned(),
                WarningFlags::PTC_SENSOR_2_HOT => "PTC sensor 2 hot".to_owned(),
                WarningFlags::REGENERATIVE_TEMP_OVERLOAD => "Regenerative temp overload".to_owned(),
                WarningFlags::SPEED_LAG_ALWAYS => "Speed lag always".to_owned(),
                WarningFlags::POSITION_SENSOR => "Position sensor".to_owned(),
                WarningFlags::INTERFACE_WARN_FLAG => "Interface warn flag".to_owned(),
                WarningFlags::APPLICATION_WARN_FLAG => "Application warn flag".to_owned(),
                unknown => format!("{:?}", unknown),
            })
            .collect();

        write!(f, "{}", names.join(", "))
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
    LessCalcTimeC0,
    LessCalcTimeC1,
    LessCalcTimeC2,
    LessCalcTimeC3,
    Unknown(u16),
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorCode::NoError => write!(f, "No error"),
            ErrorCode::LogicSupplyTooLow => write!(f, "Logic supply too low"),
            ErrorCode::LogicSupplyTooHigh => write!(f, "Logic supply too high"),
            ErrorCode::MotorSupplyTooLow => write!(f, "Motor supply too low"),
            ErrorCode::MotorSupplyTooHigh => write!(f, "Motor supply too high"),
            ErrorCode::MinPositionUndershot => write!(f, "Min position undershot"),
            ErrorCode::MaxPositionOvershot => write!(f, "Max position overshot"),
            ErrorCode::PositionLagAlwaysTooBig => write!(f, "Position lag always too big"),
            ErrorCode::MotorHotSensor => write!(f, "Motor hot sensor"),
            ErrorCode::MotorSliderMissing => write!(f, "Motor slider missing"),
            ErrorCode::MotorShortTimeOverload => write!(f, "Motor short time overload"),
            ErrorCode::MotorCommunicationLost => write!(f, "Motor communication lost"),
            ErrorCode::NotHomed => write!(f, "Not homed"),
            ErrorCode::UnknownMotionCommand => write!(f, "Unknown motion command"),
            ErrorCode::PvtBufferOverflow => write!(f, "PVT buffer overflow"),
            ErrorCode::PvtBufferUnderflow => write!(f, "PVT buffer underflow"),
            ErrorCode::PvtControllerTooFast => write!(f, "PVT controller too fast"),
            ErrorCode::PvtControllerTooSlow => write!(f, "PVT controller too slow"),
            ErrorCode::MotionCommandInWrongState => write!(f, "Motion command in wrong state"),
            ErrorCode::LessCalcTimeC0 => write!(f, "Less calc time C0"),
            ErrorCode::LessCalcTimeC1 => write!(f, "Less calc time C1"),
            ErrorCode::LessCalcTimeC2 => write!(f, "Less calc time C2"),
            ErrorCode::LessCalcTimeC3 => write!(f, "Less calc time C3"),
            ErrorCode::Unknown(e) => write!(f, "Unknown error code {}", e),
        }
    }
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
            0x90 => Self::LessCalcTimeC0,
            0x91 => Self::LessCalcTimeC1,
            0x92 => Self::LessCalcTimeC2,
            0x93 => Self::LessCalcTimeC3,
            _ => Self::Unknown(e),
        }
    }
}

impl From<u8> for ErrorCode {
    fn from(e: u8) -> Self {
        Self::from(u16::from(e))
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

// TODO: Consider making this TryFrom and have an unknown state return an error.
impl From<u16> for State {
    fn from(val: u16) -> Self {
        let sub_state = (val & 0xFF) as u8;
        let main_state = (val >> 8) as u8;

        match main_state {
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
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct MotionCommand {
    pub count: u8,
    pub command: Command,
}

impl WireWrite for MotionCommand {
    fn write_to(&self, w: &mut Writer) -> Result<(), WriteError> {
        let header = (self.command.id() << 4) | u16::from(self.count & 0xF);

        let before = w.pos();

        header.write_to(w)?;
        self.command.write_parameters(w)?;

        // Header + parameters must fit into 32 bytes
        let length = w.pos() - before;
        if length > 32 {
            return Err(WriteError::TooManyParameters { length });
        }

        // Pad the remainder of the 32-byte command block with zeros
        let padding_length = 32 - length;
        if padding_length > 0 {
            let zeros = [0u8; 32];
            w.write_bytes(&zeros[..padding_length])?;
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
