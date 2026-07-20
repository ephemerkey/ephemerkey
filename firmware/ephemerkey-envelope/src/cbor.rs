//! Minimal CBOR (RFC 8949) encoder/decoder for the fixed envelope profile.
//! Deliberately dependency-free and no-alloc: encoders write into caller
//! buffers, decoders return slices into the input. Only the major types the
//! envelope uses (uint, negint, bstr, tstr, array, map) are supported.

use crate::Error;

pub struct Enc<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> Enc<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        Enc { buf, pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.pos
    }

    fn head(&mut self, major: u8, value: u64) -> Result<(), Error> {
        let m = major << 5;
        if value < 24 {
            self.push(m | value as u8)
        } else if value <= 0xff {
            self.push(m | 24)?;
            self.push(value as u8)
        } else if value <= 0xffff {
            self.push(m | 25)?;
            self.extend(&(value as u16).to_be_bytes())
        } else if value <= 0xffff_ffff {
            self.push(m | 26)?;
            self.extend(&(value as u32).to_be_bytes())
        } else {
            self.push(m | 27)?;
            self.extend(&value.to_be_bytes())
        }
    }

    fn push(&mut self, b: u8) -> Result<(), Error> {
        *self.buf.get_mut(self.pos).ok_or(Error::BufferTooSmall)? = b;
        self.pos += 1;
        Ok(())
    }

    fn extend(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let end = self.pos + bytes.len();
        self.buf
            .get_mut(self.pos..end)
            .ok_or(Error::BufferTooSmall)?
            .copy_from_slice(bytes);
        self.pos = end;
        Ok(())
    }

    pub fn uint(&mut self, v: u64) -> Result<(), Error> {
        self.head(0, v)
    }

    pub fn int(&mut self, v: i64) -> Result<(), Error> {
        if v >= 0 {
            self.head(0, v as u64)
        } else {
            self.head(1, (-1 - v) as u64)
        }
    }

    pub fn bstr(&mut self, b: &[u8]) -> Result<(), Error> {
        self.head(2, b.len() as u64)?;
        self.extend(b)
    }

    pub fn tstr(&mut self, s: &str) -> Result<(), Error> {
        self.head(3, s.len() as u64)?;
        self.extend(s.as_bytes())
    }

    pub fn array(&mut self, len: u64) -> Result<(), Error> {
        self.head(4, len)
    }

    pub fn map(&mut self, len: u64) -> Result<(), Error> {
        self.head(5, len)
    }

    /// A CBOR boolean (major 7: 0xf5 true / 0xf4 false). Config policy fields
    /// use these; the decoder has no reader, but `Dec::skip` consumes them.
    pub fn bool(&mut self, v: bool) -> Result<(), Error> {
        self.extend(&[if v { 0xf5 } else { 0xf4 }])
    }

    /// Reserve a bstr of exactly `len` bytes; returns its position range for
    /// the caller to fill (used for encrypt-in-place).
    pub fn bstr_reserve(&mut self, len: usize) -> Result<core::ops::Range<usize>, Error> {
        self.head(2, len as u64)?;
        let start = self.pos;
        let end = start + len;
        if end > self.buf.len() {
            return Err(Error::BufferTooSmall);
        }
        self.pos = end;
        Ok(start..end)
    }
}

pub struct Dec<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Dec<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Dec { buf, pos: 0 }
    }

    fn byte(&mut self) -> Result<u8, Error> {
        let b = *self.buf.get(self.pos).ok_or(Error::Truncated)?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos + n;
        let s = self.buf.get(self.pos..end).ok_or(Error::Truncated)?;
        self.pos = end;
        Ok(s)
    }

    fn head(&mut self) -> Result<(u8, u64), Error> {
        let b = self.byte()?;
        let major = b >> 5;
        let info = b & 0x1f;
        let value = match info {
            0..=23 => info as u64,
            24 => self.byte()? as u64,
            25 => u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as u64,
            26 => u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as u64,
            27 => u64::from_be_bytes(self.take(8)?.try_into().unwrap()),
            _ => return Err(Error::Malformed), // indefinite lengths unsupported
        };
        Ok((major, value))
    }

    pub fn uint(&mut self) -> Result<u64, Error> {
        match self.head()? {
            (0, v) => Ok(v),
            _ => Err(Error::Malformed),
        }
    }

    pub fn int(&mut self) -> Result<i64, Error> {
        match self.head()? {
            (0, v) => i64::try_from(v).map_err(|_| Error::Malformed),
            (1, v) => Ok(-1 - i64::try_from(v).map_err(|_| Error::Malformed)?),
            _ => Err(Error::Malformed),
        }
    }

    pub fn bstr(&mut self) -> Result<&'a [u8], Error> {
        match self.head()? {
            (2, len) => self.take(len as usize),
            _ => Err(Error::Malformed),
        }
    }

    pub fn tstr(&mut self) -> Result<&'a str, Error> {
        match self.head()? {
            (3, len) => core::str::from_utf8(self.take(len as usize)?)
                .map_err(|_| Error::Malformed),
            _ => Err(Error::Malformed),
        }
    }

    pub fn array(&mut self) -> Result<u64, Error> {
        match self.head()? {
            (4, len) => Ok(len),
            _ => Err(Error::Malformed),
        }
    }

    pub fn map(&mut self) -> Result<u64, Error> {
        match self.head()? {
            (5, len) => Ok(len),
            _ => Err(Error::Malformed),
        }
    }

    /// Skip one complete item (for unknown map keys — forward compatibility).
    pub fn skip(&mut self) -> Result<(), Error> {
        self.skip_depth(8)
    }

    fn skip_depth(&mut self, depth: u8) -> Result<(), Error> {
        if depth == 0 {
            return Err(Error::Malformed);
        }
        let (major, value) = self.head()?;
        match major {
            0 | 1 | 7 => Ok(()),
            2 | 3 => self.take(value as usize).map(|_| ()),
            4 => {
                for _ in 0..value {
                    self.skip_depth(depth - 1)?;
                }
                Ok(())
            }
            5 => {
                for _ in 0..value {
                    self.skip_depth(depth - 1)?;
                    self.skip_depth(depth - 1)?;
                }
                Ok(())
            }
            6 => self.skip_depth(depth - 1), // tag: skip the tagged item
            _ => Err(Error::Malformed),
        }
    }
}
