use anyhow::{Result, anyhow};

/// Bounded little-endian writer over a preallocated buffer.
pub struct Writer<'a> {
    buf: &'a mut [u8],
    idx: usize,
}

impl<'a> Writer<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, idx: 0 }
    }

    pub fn pos(&self) -> usize {
        self.idx
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.idx)
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        let needed = bytes.len();

        if self.idx + needed > self.buf.len() {
            return Err(anyhow!("buffer overflow while serializing (need {}, have {})", needed, self.remaining()));
        }

        let end = self.idx + needed;
        self.buf[self.idx..end].copy_from_slice(bytes);
        self.idx = end;

        Ok(())
    }

    pub fn write_u8(&mut self, v: u8) -> Result<()> {
        self.write_bytes(&[v])
    }

    pub fn write_u16_le(&mut self, v: u16) -> Result<()> {
        self.write_bytes(&v.to_le_bytes())
    }

    pub fn write_i16_le(&mut self, v: i16) -> Result<()> {
        self.write_bytes(&v.to_le_bytes())
    }

    pub fn write_u32_le(&mut self, v: u32) -> Result<()> {
        self.write_bytes(&v.to_le_bytes())
    }

    pub fn write_i32_le(&mut self, v: i32) -> Result<()> {
        self.write_bytes(&v.to_le_bytes())
    }
}

/// Trait for types that can serialize themselves to a preallocated buffer via `Writer`.
pub trait WireWrite {
    fn write_to(&self, w: &mut Writer) -> Result<()>;
}

impl WireWrite for u8 {
    fn write_to(&self, w: &mut Writer) -> Result<()> {
        w.write_u8(*self)
    }
}

impl WireWrite for u16 {
    fn write_to(&self, w: &mut Writer) -> Result<()> {
        w.write_u16_le(*self)
    }
}

impl WireWrite for i16 {
    fn write_to(&self, w: &mut Writer) -> Result<()> {
        w.write_i16_le(*self)
    }
}

impl WireWrite for u32 {
    fn write_to(&self, w: &mut Writer) -> Result<()> {
        w.write_u32_le(*self)
    }
}

impl WireWrite for i32 {
    fn write_to(&self, w: &mut Writer) -> Result<()> {
        w.write_i32_le(*self)
    }
}
