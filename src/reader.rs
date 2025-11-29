use anyhow::{Result, anyhow};

/// Simple cursor-based little-endian reader with bounds checking
pub struct Reader<'a> {
    buf: &'a [u8],
    idx: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, idx: 0 }
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.idx + n > self.buf.len() {
            return Err(anyhow!(
                "buffer underflow while parsing (needed {}, have {})",
                n,
                self.buf.len().saturating_sub(self.idx)
            ));
        }

        let s = &self.buf[self.idx..self.idx + n];
        self.idx += n;

        Ok(s)
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_bytes(1)?[0])
    }

    pub fn read_u16_le(&mut self) -> Result<u16> {
        let b = self.read_bytes(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn read_i16_le(&mut self) -> Result<i16> {
        Ok(self.read_u16_le()? as i16)
    }

    pub fn read_u32_le(&mut self) -> Result<u32> {
        let b = self.read_bytes(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn read_i32_le(&mut self) -> Result<i32> {
        Ok(self.read_u32_le()? as i32)
    }
}

/// Trait for types that can be deserialized from a `Reader`.
pub trait WireRead: Sized {
    fn read_from(r: &mut Reader) -> Result<Self>;
}

impl WireRead for u8 {
    fn read_from(r: &mut Reader) -> Result<Self> {
        r.read_u8()
    }
}

impl WireRead for u16 {
    fn read_from(r: &mut Reader) -> Result<Self> {
        r.read_u16_le()
    }
}

impl WireRead for i16 {
    fn read_from(r: &mut Reader) -> Result<Self> {
        r.read_i16_le()
    }
}

impl WireRead for u32 {
    fn read_from(r: &mut Reader) -> Result<Self> {
        r.read_u32_le()
    }
}

impl WireRead for i32 {
    fn read_from(r: &mut Reader) -> Result<Self> {
        r.read_i32_le()
    }
}

impl<T: WireRead> WireRead for (T, T, T, T) {
    fn read_from(r: &mut Reader) -> Result<Self> {
        Ok((T::read_from(r)?, T::read_from(r)?, T::read_from(r)?, T::read_from(r)?))
    }
}
