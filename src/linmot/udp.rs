use super::mci::{ControlFlags, ErrorCode, MotionCommand, State, StatusFlags, WarningFlags};
use crate::reader::{Reader, WireRead};
use crate::writer::{WireWrite, Writer};
use anyhow::Result;
use bitflags::bitflags;

pub const MASTER_PORT: u16 = 0xA0B0;
pub const DRIVE_PORT: u16 = 0xC0D0;
pub const BUFFER_SIZE: usize = 64;

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

    /// Serializes the request into the provided output buffer.
    ///
    /// # Errors
    /// Returns an error if the output buffer is too small to fit the encoded request.
    pub fn to_wire(&self, out: &mut [u8]) -> Result<usize> {
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
    /// Parses a response from the provided input buffer.
    ///
    /// # Errors
    /// Returns an error if the buffer is too small or contains invalid data for a response.
    pub fn from_wire(buf: &[u8]) -> Result<Self> {
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
        if flags.contains(flag) { T::read_from(rd).map(Some) } else { Ok(None) }
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

        let mut params = [0u16; 3];
        for p in &mut params {
            *p = u16::read_from(r)?;
        }

        Ok(Self { command, params })
    }
}
