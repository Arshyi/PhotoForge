//! A reader that cannot be talked into reading, or allocating, more than the file holds.
//!
//! Every read names the absolute offset it must stay below (`limit`): the end of the
//! section being parsed, not merely the end of the file. A length field is therefore
//! checked against the bytes that actually remain *before* any buffer is made for it, so
//! a header that claims four gigabytes costs a comparison and an error.
use super::error::PsdError;
use std::io::{Read, Seek, SeekFrom};

pub struct Reader<R> {
    inner: R,
    pos: u64,
    len: u64,
}

impl<R: Read + Seek> Reader<R> {
    pub fn new(mut inner: R) -> Result<Self, PsdError> {
        let len = inner.seek(SeekFrom::End(0))?;
        inner.seek(SeekFrom::Start(0))?;
        Ok(Self { inner, pos: 0, len })
    }

    /// The size of the whole file.
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn position(&self) -> u64 {
        self.pos
    }

    pub fn seek_to(&mut self, pos: u64, what: &'static str) -> Result<(), PsdError> {
        if pos > self.len {
            return Err(PsdError::Truncated { what });
        }
        self.inner.seek(SeekFrom::Start(pos))?;
        self.pos = pos;
        Ok(())
    }

    /// Moves forward `count` bytes, which must lie before `limit`.
    pub fn skip(&mut self, count: u64, limit: u64, what: &'static str) -> Result<(), PsdError> {
        let target = self
            .pos
            .checked_add(count)
            .filter(|target| *target <= limit.min(self.len))
            .ok_or(PsdError::Truncated { what })?;
        self.seek_to(target, what)
    }

    fn fill(&mut self, buffer: &mut [u8], limit: u64, what: &'static str) -> Result<(), PsdError> {
        let end = self
            .pos
            .checked_add(buffer.len() as u64)
            .filter(|end| *end <= limit.min(self.len))
            .ok_or(PsdError::Truncated { what })?;
        self.inner
            .read_exact(buffer)
            .map_err(|_| PsdError::Truncated { what })?;
        self.pos = end;
        Ok(())
    }

    pub fn u8(&mut self, limit: u64, what: &'static str) -> Result<u8, PsdError> {
        let mut buffer = [0_u8; 1];
        self.fill(&mut buffer, limit, what)?;
        Ok(buffer[0])
    }

    pub fn u16(&mut self, limit: u64, what: &'static str) -> Result<u16, PsdError> {
        let mut buffer = [0_u8; 2];
        self.fill(&mut buffer, limit, what)?;
        Ok(u16::from_be_bytes(buffer))
    }

    pub fn i16(&mut self, limit: u64, what: &'static str) -> Result<i16, PsdError> {
        let mut buffer = [0_u8; 2];
        self.fill(&mut buffer, limit, what)?;
        Ok(i16::from_be_bytes(buffer))
    }

    pub fn u32(&mut self, limit: u64, what: &'static str) -> Result<u32, PsdError> {
        let mut buffer = [0_u8; 4];
        self.fill(&mut buffer, limit, what)?;
        Ok(u32::from_be_bytes(buffer))
    }

    pub fn i32(&mut self, limit: u64, what: &'static str) -> Result<i32, PsdError> {
        let mut buffer = [0_u8; 4];
        self.fill(&mut buffer, limit, what)?;
        Ok(i32::from_be_bytes(buffer))
    }

    pub fn u64(&mut self, limit: u64, what: &'static str) -> Result<u64, PsdError> {
        let mut buffer = [0_u8; 8];
        self.fill(&mut buffer, limit, what)?;
        Ok(u64::from_be_bytes(buffer))
    }

    pub fn tag(&mut self, limit: u64, what: &'static str) -> Result<[u8; 4], PsdError> {
        let mut buffer = [0_u8; 4];
        self.fill(&mut buffer, limit, what)?;
        Ok(buffer)
    }

    /// A length that is 4 bytes in a PSD and 8 in a PSB.
    pub fn length(&mut self, big: bool, limit: u64, what: &'static str) -> Result<u64, PsdError> {
        if big {
            self.u64(limit, what)
        } else {
            Ok(u64::from(self.u32(limit, what)?))
        }
    }

    /// `count` bytes, refused if they do not lie before `limit` or if `count` exceeds `max`.
    /// Nothing is allocated for a refused count.
    pub fn bytes(
        &mut self,
        count: u64,
        max: u64,
        limit: u64,
        what: &'static str,
    ) -> Result<Vec<u8>, PsdError> {
        if count > max {
            return Err(PsdError::Limit {
                what,
                value: count,
                limit: max,
            });
        }
        let available = limit.min(self.len).saturating_sub(self.pos);
        if count > available {
            return Err(PsdError::Truncated { what });
        }
        let mut buffer = vec![0_u8; count as usize];
        self.fill(&mut buffer, limit, what)?;
        Ok(buffer)
    }

    /// Looks at the next four bytes without consuming them (`None` if fewer remain).
    pub fn peek_tag(&mut self, limit: u64) -> Option<[u8; 4]> {
        let start = self.pos;
        let tag = self.tag(limit, "a signature").ok();
        let _ = self.seek_to(start, "a signature");
        tag
    }
}
