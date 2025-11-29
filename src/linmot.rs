use crate::reader::{Reader, WireRead};
use crate::writer::{WireWrite, Writer};
use bitflags::bitflags;
use anyhow::Result;

pub static CONTROL_MASTER_PORT: u16 = 0xA0B0;
pub static CONTROL_DRIVE_PORT: u16 = 0xC0D0;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct RequestFlags: u32 {
        const CONTROL_WORD = 1 << 0;
        const MC_INTERFACE = 1 << 1;
        const REALTIME_CONFIGURATION = 1 << 2;
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct ResponseFlags: u32 {
        const STATUS_WORD = 1 << 0;
        const STATE_VAR = 1 << 1;
        const ACTUAL_POSITION = 1 << 2;
        const DEMAND_POSITION = 1 << 3;
        const CURRENT = 1 << 4;
        const WARN_WORD = 1 << 5;
        const ERROR_CODE = 1 << 6;
        const MONITORING_CHANNEL = 1 << 7;
        const REALTIME_CONFIGURATION = 1 << 8;
    }
}

#[derive(Debug, Default, Clone)]
pub struct Request {
    pub control_word: Option<u16>,
    pub motion_command: Option<MotionCommand>,
    pub realtime_configuration: Option<RealtimeConfiguration>,
    pub response_flags: ResponseFlags,
}

#[derive(Debug, Default, Clone)]
pub struct Response {
    pub status_word: Option<u16>,
    pub state_var: Option<StateVar>,
    pub actual_position: Option<i32>,
    pub demand_position: Option<i32>,
    pub current: Option<i16>,
    pub warn_word: Option<u16>,
    pub error_code: Option<u16>,
    pub monitoring_channel: Option<(u32, u32, u32, u32)>,
    pub realtime_configuration: Option<RealtimeConfiguration>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MotionCommand {
    pub command: u16,
    pub params: [u16; 15],
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RealtimeConfiguration {
    pub command: u16,
    pub params: [u16; 3],
}

#[derive(Debug, Default, Clone, Copy)]
pub struct StateVar {
    pub sub_state: u8,
    pub main_state: u8,
}

impl Request {
    fn flags(&self) -> RequestFlags {
        let mut f = RequestFlags::empty();

        if self.control_word.is_some() {
            f |= RequestFlags::CONTROL_WORD;
        }

        if self.motion_command.is_some() {
            f |= RequestFlags::MC_INTERFACE;
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

        if let Some(cw) = self.control_word {
            cw.write_to(&mut w)?;
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

impl Response {
    pub(crate) fn from_wire(buf: &[u8]) -> Result<Self> {
        let mut rd = Reader::new(buf);

        let _request_flags = RequestFlags::from_bits_truncate(rd.read_u32_le()?);
        let response_flags = ResponseFlags::from_bits_truncate(rd.read_u32_le()?);

        Ok(Self {
            status_word: Self::read_opt(&mut rd, response_flags, ResponseFlags::STATUS_WORD)?,
            state_var: Self::read_opt(&mut rd, response_flags, ResponseFlags::STATE_VAR)?,
            actual_position: Self::read_opt(&mut rd, response_flags, ResponseFlags::ACTUAL_POSITION)?,
            demand_position: Self::read_opt(&mut rd, response_flags, ResponseFlags::DEMAND_POSITION)?,
            current: Self::read_opt(&mut rd, response_flags, ResponseFlags::CURRENT)?,
            warn_word: Self::read_opt(&mut rd, response_flags, ResponseFlags::WARN_WORD)?,
            error_code: Self::read_opt(&mut rd, response_flags, ResponseFlags::ERROR_CODE)?,
            monitoring_channel: Self::read_opt(&mut rd, response_flags, ResponseFlags::MONITORING_CHANNEL)?,
            realtime_configuration: Self::read_opt(&mut rd, response_flags, ResponseFlags::REALTIME_CONFIGURATION)?,
        })
    }

    fn read_opt<T: WireRead>(rd: &mut Reader, flags: ResponseFlags, flag: ResponseFlags) -> Result<Option<T>> {
        if flags.contains(flag) {
            Ok(Some(T::read_from(rd)?))
        } else {
            Ok(None)
        }
    }
}

impl WireWrite for MotionCommand {
    fn write_to(&self, w: &mut Writer) -> Result<()> {
        self.command.write_to(w)?;

        for v in &self.params {
            v.write_to(w)?;
        }

        Ok(())
    }
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

impl WireRead for StateVar {
    fn read_from(r: &mut Reader) -> Result<Self> {
        Ok(StateVar {
            sub_state: u8::read_from(r)?,
            main_state: u8::read_from(r)?,
        })
    }
}

impl WireRead for (u32, u32, u32, u32) {
    fn read_from(r: &mut Reader) -> Result<Self> {
        Ok((
            u32::read_from(r)?,
            u32::read_from(r)?,
            u32::read_from(r)?,
            u32::read_from(r)?,
        ))
    }
}
