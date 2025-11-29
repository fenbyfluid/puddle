use crate::reader::{Reader, WireRead};
use crate::writer::{WireWrite, Writer};
use anyhow::{Result, anyhow};
use bitflags::bitflags;

pub static CONTROL_MASTER_PORT: u16 = 0xA0B0;
pub static CONTROL_DRIVE_PORT: u16 = 0xC0D0;
pub static CONTROL_BUFFER_SIZE: usize = 64;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct RequestFlags: u32 {
        const CONTROL_FLAGS = 1 << 0;
        const MOTION_COMMAND = 1 << 1;
        const REALTIME_CONFIGURATION = 1 << 2;
        const _ = !0;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct ResponseFlags: u32 {
        const STATUS_FLAGS = 1 << 0;
        const STATE = 1 << 1;
        const ACTUAL_POSITION = 1 << 2;
        const DEMAND_POSITION = 1 << 3;
        const CURRENT = 1 << 4;
        const WARNING_FLAGS = 1 << 5;
        const ERROR_CODE = 1 << 6;
        const MONITORING_CHANNEL = 1 << 7;
        const REALTIME_CONFIGURATION = 1 << 8;
        const _ = !0;
    }

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
    PvtMasterTooFast,
    PvtMasterTooSlow,
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
            0x84 => Self::PvtMasterTooFast,
            0x85 => Self::PvtMasterTooSlow,
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
    fn id(&self) -> u16 {
        match self {
            Self::NoOperation => 0x000,
            Self::VaiGoToPos { .. } => 0x010,
        }
    }

    fn write_parameters(&self, w: &mut Writer) -> Result<()> {
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

#[derive(Debug, Default, Clone, Copy)]
pub struct MotionCommand {
    pub count: u8,
    pub command: Command,
}

impl WireWrite for MotionCommand {
    fn write_to(&self, w: &mut Writer) -> Result<()> {
        let header = (self.command.id() << 4) | (self.count & 0xF) as u16;
        header.write_to(w)?;

        let before = w.pos();
        self.command.write_parameters(w)?;
        let after = w.pos();

        let parameters_len = after - before;
        if parameters_len > 30 {
            // Header (2) + parameters must fit into 32 bytes
            return Err(anyhow!("motion command parameters too large: {} bytes (max 30)", parameters_len));
        }

        // Pad the remainder of the 32-byte command block with zeros
        let pad_len = 32 - 2 - parameters_len;
        if pad_len > 0 {
            let zeros = [0u8; 32];
            w.write_bytes(&zeros[..pad_len])?;
        }

        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RealtimeConfiguration {
    pub command: u16,
    pub params: [u16; 3],
}

impl WireWrite for RealtimeConfiguration {
    fn write_to(&self, w: &mut Writer) -> Result<()> {
        self.command.write_to(w)?;

        for v in &self.params {
            v.write_to(w)?;
        }

        Ok(())
    }
}

impl WireRead for RealtimeConfiguration {
    fn read_from(r: &mut Reader) -> Result<Self> {
        let command = u16::read_from(r)?;

        let mut params = [0u16; _];
        for p in &mut params {
            *p = u16::read_from(r)?;
        }

        Ok(RealtimeConfiguration { command, params })
    }
}

#[derive(Debug, Default, Clone)]
pub struct Request {
    pub control_flags: Option<ControlFlags>,
    pub motion_command: Option<MotionCommand>,
    pub realtime_configuration: Option<RealtimeConfiguration>,
    pub response_flags: ResponseFlags,
}

impl Request {
    fn flags(&self) -> RequestFlags {
        let mut f = RequestFlags::empty();

        if self.control_flags.is_some() {
            f |= RequestFlags::CONTROL_FLAGS;
        }

        if self.motion_command.is_some() {
            f |= RequestFlags::MOTION_COMMAND;
        }

        if self.realtime_configuration.is_some() {
            f |= RequestFlags::REALTIME_CONFIGURATION;
        }

        f
    }

    pub(crate) fn to_wire(&self, out: &mut [u8]) -> Result<usize> {
        let mut w = Writer::new(out);

        self.flags().bits().write_to(&mut w)?;
        self.response_flags.bits().write_to(&mut w)?;

        if let Some(cw) = self.control_flags {
            cw.bits().write_to(&mut w)?;
        }

        if let Some(mc) = &self.motion_command {
            mc.write_to(&mut w)?;
        }

        if let Some(rtc) = &self.realtime_configuration {
            rtc.write_to(&mut w)?;
        }

        Ok(w.pos())
    }
}

#[derive(Debug, Default, Clone)]
pub struct Response {
    pub status_flags: Option<StatusFlags>,
    pub state: Option<State>,
    pub actual_position: Option<i32>,
    pub demand_position: Option<i32>,
    pub current: Option<i16>,
    pub warning_flags: Option<WarningFlags>,
    pub error_code: Option<ErrorCode>,
    pub monitoring_channel: Option<(u32, u32, u32, u32)>,
    pub realtime_configuration: Option<RealtimeConfiguration>,
}

impl Response {
    pub(crate) fn from_wire(buf: &[u8]) -> Result<Self> {
        let mut rd = Reader::new(buf);

        let request_flags = RequestFlags::from_bits_truncate(rd.read_u32_le()?);
        let mut response_flags = ResponseFlags::from_bits_truncate(rd.read_u32_le()?);

        // The response only includes the realtime configuration if it was also specified in the request
        if !request_flags.contains(RequestFlags::REALTIME_CONFIGURATION) {
            response_flags.remove(ResponseFlags::REALTIME_CONFIGURATION);
        }

        Ok(Self {
            status_flags: Self::read_opt(&mut rd, response_flags, ResponseFlags::STATUS_FLAGS)?
                .map(StatusFlags::from_bits_truncate),
            state: Self::read_opt(&mut rd, response_flags, ResponseFlags::STATE)?,
            actual_position: Self::read_opt(&mut rd, response_flags, ResponseFlags::ACTUAL_POSITION)?,
            demand_position: Self::read_opt(&mut rd, response_flags, ResponseFlags::DEMAND_POSITION)?,
            current: Self::read_opt(&mut rd, response_flags, ResponseFlags::CURRENT)?,
            warning_flags: Self::read_opt(&mut rd, response_flags, ResponseFlags::WARNING_FLAGS)?
                .map(WarningFlags::from_bits_truncate),
            error_code: Self::read_opt(&mut rd, response_flags, ResponseFlags::ERROR_CODE)?,
            monitoring_channel: Self::read_opt(&mut rd, response_flags, ResponseFlags::MONITORING_CHANNEL)?,
            realtime_configuration: Self::read_opt(&mut rd, response_flags, ResponseFlags::REALTIME_CONFIGURATION)?,
        })
    }

    fn read_opt<T: WireRead>(rd: &mut Reader, flags: ResponseFlags, flag: ResponseFlags) -> Result<Option<T>> {
        if flags.contains(flag) { Ok(Some(T::read_from(rd)?)) } else { Ok(None) }
    }
}
