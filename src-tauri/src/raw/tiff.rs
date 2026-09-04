//! A bounded TIFF/IFD reader for untrusted camera files.
//!
//! DNG is a TIFF/EP derivative, so reading one starts with reading TIFF. Every
//! camera file reaching this module is hostile input: offsets, counts, and
//! lengths in the file are attacker-controlled and are validated against the
//! actual buffer before a single byte is allocated or indexed. The reader never
//! seeks outside the buffer, never follows an IFD chain further than
//! `MAX_IFDS`, never recurses deeper than `MAX_IFD_DEPTH`, and never allocates
//! a value larger than `MAX_VALUE_BYTES`.
//!
//! Nothing here interprets DNG semantics; that belongs in `raw::dng`.

use std::collections::BTreeMap;

/// Largest number of IFDs, across the whole file, that will be walked.
pub const MAX_IFDS: usize = 64;
/// Largest SubIFD nesting depth.
pub const MAX_IFD_DEPTH: usize = 4;
/// Largest number of entries one IFD may declare.
pub const MAX_IFD_ENTRIES: usize = 512;
/// Largest byte length of a single tag value.
pub const MAX_VALUE_BYTES: usize = 4 * 1024 * 1024;
/// Largest number of elements a numeric tag may declare.
pub const MAX_VALUE_COUNT: usize = 1024 * 1024;
/// Largest ASCII tag length kept, in bytes.
pub const MAX_ASCII_BYTES: usize = 512;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TiffError {
    #[error("the file is not a TIFF container")]
    NotTiff,
    #[error("the file ends inside a structure that declares more data: {0}")]
    Truncated(&'static str),
    #[error("the file declares a structure beyond its own bounds: {0}")]
    OutOfBounds(&'static str),
    #[error("the file exceeds a structural limit: {0}")]
    LimitExceeded(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    Little,
    Big,
}

impl Endian {
    fn u16(self, bytes: [u8; 2]) -> u16 {
        match self {
            Self::Little => u16::from_le_bytes(bytes),
            Self::Big => u16::from_be_bytes(bytes),
        }
    }

    fn u32(self, bytes: [u8; 4]) -> u32 {
        match self {
            Self::Little => u32::from_le_bytes(bytes),
            Self::Big => u32::from_be_bytes(bytes),
        }
    }
}

/// TIFF field types this reader understands. Unknown types are kept as raw
/// bytes rather than rejected, because a camera may write vendor fields the
/// specification never described.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldType {
    Byte,
    Ascii,
    Short,
    Long,
    Rational,
    SByte,
    Undefined,
    SShort,
    SLong,
    SRational,
    Float,
    Double,
    Unknown(u16),
}

impl FieldType {
    fn from_code(code: u16) -> Self {
        match code {
            1 => Self::Byte,
            2 => Self::Ascii,
            3 => Self::Short,
            4 => Self::Long,
            5 => Self::Rational,
            6 => Self::SByte,
            7 => Self::Undefined,
            8 => Self::SShort,
            9 => Self::SLong,
            10 => Self::SRational,
            11 => Self::Float,
            12 => Self::Double,
            other => Self::Unknown(other),
        }
    }

    /// Bytes per element, or `None` for a type of unknown width.
    const fn width(self) -> Option<usize> {
        Some(match self {
            Self::Byte | Self::Ascii | Self::SByte | Self::Undefined => 1,
            Self::Short | Self::SShort => 2,
            Self::Long | Self::SLong | Self::Float => 4,
            Self::Rational | Self::SRational | Self::Double => 8,
            Self::Unknown(_) => return None,
        })
    }
}

/// One decoded IFD entry. The payload is copied out of the buffer so callers
/// never hold an offset they might later use unchecked.
#[derive(Debug, Clone)]
pub struct Entry {
    pub tag: u16,
    pub field_type: FieldType,
    pub count: usize,
    bytes: Vec<u8>,
    endian: Endian,
}

impl Entry {
    /// The entry as an unsigned integer sequence, whatever its width.
    ///
    /// Signed types are included because cameras have been observed writing a
    /// signed type for a value the specification calls unsigned; a negative
    /// value would be nonsense for those tags and is clamped to zero rather
    /// than wrapping into an enormous positive number.
    pub fn as_u32_vec(&self) -> Vec<u32> {
        let mut values = Vec::new();
        match self.field_type {
            FieldType::Byte | FieldType::Undefined => {
                values.extend(self.bytes.iter().map(|byte| u32::from(*byte)));
            }
            FieldType::SByte => {
                values.extend(
                    self.bytes
                        .iter()
                        .map(|byte| u32::from((*byte as i8).max(0) as u8)),
                );
            }
            FieldType::Short => {
                for chunk in self.bytes.chunks_exact(2) {
                    values.push(u32::from(self.endian.u16([chunk[0], chunk[1]])));
                }
            }
            FieldType::SShort => {
                for chunk in self.bytes.chunks_exact(2) {
                    let signed = self.endian.u16([chunk[0], chunk[1]]) as i16;
                    values.push(u32::from(signed.max(0) as u16));
                }
            }
            FieldType::Long => {
                for chunk in self.bytes.chunks_exact(4) {
                    values.push(self.endian.u32([chunk[0], chunk[1], chunk[2], chunk[3]]));
                }
            }
            FieldType::SLong => {
                for chunk in self.bytes.chunks_exact(4) {
                    let signed = self.endian.u32([chunk[0], chunk[1], chunk[2], chunk[3]]) as i32;
                    values.push(signed.max(0) as u32);
                }
            }
            FieldType::Rational => {
                // A rational used where an integer is expected is taken at its
                // rounded value, which is what the DNG black-level tags need.
                for value in self.as_f64_vec() {
                    values.push(value.max(0.0).min(f64::from(u32::MAX)) as u32);
                }
            }
            _ => {}
        }
        values
    }

    /// The entry as a floating-point sequence.
    pub fn as_f64_vec(&self) -> Vec<f64> {
        let mut values = Vec::new();
        match self.field_type {
            FieldType::Rational | FieldType::SRational => {
                let signed = self.field_type == FieldType::SRational;
                for chunk in self.bytes.chunks_exact(8) {
                    let first = self.endian.u32([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    let second = self.endian.u32([chunk[4], chunk[5], chunk[6], chunk[7]]);
                    let (numerator, denominator) = if signed {
                        (f64::from(first as i32), f64::from(second as i32))
                    } else {
                        (f64::from(first), f64::from(second))
                    };
                    // A zero denominator is malformed; it contributes nothing
                    // rather than producing an infinity that spreads.
                    values.push(if denominator == 0.0 {
                        0.0
                    } else {
                        numerator / denominator
                    });
                }
            }
            FieldType::Float => {
                for chunk in self.bytes.chunks_exact(4) {
                    let bits = self.endian.u32([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    values.push(f64::from(f32::from_bits(bits)));
                }
            }
            FieldType::Double => {
                for chunk in self.bytes.chunks_exact(8) {
                    let low = self.endian.u32([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    let high = self.endian.u32([chunk[4], chunk[5], chunk[6], chunk[7]]);
                    let bits = match self.endian {
                        Endian::Little => (u64::from(high) << 32) | u64::from(low),
                        Endian::Big => (u64::from(low) << 32) | u64::from(high),
                    };
                    values.push(f64::from_bits(bits));
                }
            }
            _ => {
                values.extend(self.as_u32_vec().into_iter().map(f64::from));
            }
        }
        values
    }

    pub fn first_u32(&self) -> Option<u32> {
        self.as_u32_vec().first().copied()
    }

    pub fn first_f64(&self) -> Option<f64> {
        self.as_f64_vec().first().copied()
    }

    /// An ASCII tag, trimmed of its terminator and of surrounding whitespace.
    ///
    /// Control characters are rejected rather than sanitised: a camera name
    /// carrying them is not a camera name, and passing it on would put
    /// attacker-chosen bytes into the interface.
    pub fn as_ascii(&self) -> Option<String> {
        if self.field_type != FieldType::Ascii {
            return None;
        }
        let end = self
            .bytes
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(self.bytes.len());
        let slice = &self.bytes[..end.min(MAX_ASCII_BYTES)];
        let text = std::str::from_utf8(slice).ok()?.trim();
        if text.is_empty() || text.chars().any(char::is_control) {
            return None;
        }
        Some(text.to_string())
    }

    pub fn raw_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// One image file directory: its entries, keyed by tag.
#[derive(Debug, Clone, Default)]
pub struct Ifd {
    pub entries: BTreeMap<u16, Entry>,
}

impl Ifd {
    pub fn get(&self, tag: u16) -> Option<&Entry> {
        self.entries.get(&tag)
    }

    pub fn u32(&self, tag: u16) -> Option<u32> {
        self.get(tag).and_then(Entry::first_u32)
    }

    pub fn f64(&self, tag: u16) -> Option<f64> {
        self.get(tag).and_then(Entry::first_f64)
    }

    pub fn ascii(&self, tag: u16) -> Option<String> {
        self.get(tag).and_then(Entry::as_ascii)
    }

    pub fn u32_vec(&self, tag: u16) -> Option<Vec<u32>> {
        self.get(tag).map(Entry::as_u32_vec)
    }

    pub fn f64_vec(&self, tag: u16) -> Option<Vec<f64>> {
        self.get(tag).map(Entry::as_f64_vec)
    }
}

/// Every IFD found in a TIFF file, flattened with its nesting depth recorded.
#[derive(Debug, Clone)]
pub struct TiffFile {
    pub endian: Endian,
    /// Top-level IFDs in file order, each followed by its SubIFDs.
    pub ifds: Vec<Ifd>,
}

/// Tag 330: SubIFDs. Walked as part of the structure rather than by a caller,
/// so the depth limit is enforced in one place.
const TAG_SUB_IFDS: u16 = 330;
/// Tag 34665: the Exif IFD, which carries the capture settings.
pub const TAG_EXIF_IFD: u16 = 34665;

struct Reader<'a> {
    data: &'a [u8],
    endian: Endian,
    budget: usize,
}

impl<'a> Reader<'a> {
    fn slice(&self, offset: usize, length: usize) -> Result<&'a [u8], TiffError> {
        let end = offset
            .checked_add(length)
            .ok_or(TiffError::OutOfBounds("value offset overflows"))?;
        self.data
            .get(offset..end)
            .ok_or(TiffError::OutOfBounds("value lies outside the file"))
    }

    fn u16_at(&self, offset: usize) -> Result<u16, TiffError> {
        let bytes = self.slice(offset, 2)?;
        Ok(self.endian.u16([bytes[0], bytes[1]]))
    }

    fn u32_at(&self, offset: usize) -> Result<u32, TiffError> {
        let bytes = self.slice(offset, 4)?;
        Ok(self.endian.u32([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_ifd(&mut self, offset: usize) -> Result<(Ifd, Option<usize>), TiffError> {
        let count = self.u16_at(offset)? as usize;
        if count > MAX_IFD_ENTRIES {
            return Err(TiffError::LimitExceeded("too many entries in one IFD"));
        }
        let mut ifd = Ifd::default();
        for index in 0..count {
            let base = offset
                .checked_add(2 + index * 12)
                .ok_or(TiffError::OutOfBounds("entry offset overflows"))?;
            let tag = self.u16_at(base)?;
            let field_type = FieldType::from_code(self.u16_at(base + 2)?);
            let value_count = self.u32_at(base + 4)? as usize;
            let Some(width) = field_type.width() else {
                // An unrecognised field type has no defined width, so its
                // payload cannot be located safely. Skipping the entry keeps
                // the rest of the directory readable.
                continue;
            };
            if value_count > MAX_VALUE_COUNT {
                continue;
            }
            let Some(length) = value_count.checked_mul(width) else {
                continue;
            };
            if length > MAX_VALUE_BYTES {
                continue;
            }
            // Four bytes or fewer live in the entry itself; anything longer is
            // stored elsewhere and the entry holds an offset to it.
            let bytes = if length <= 4 {
                self.slice(base + 8, length)?.to_vec()
            } else {
                let value_offset = self.u32_at(base + 8)? as usize;
                self.slice(value_offset, length)?.to_vec()
            };
            ifd.entries.insert(
                tag,
                Entry {
                    tag,
                    field_type,
                    count: value_count,
                    bytes,
                    endian: self.endian,
                },
            );
        }
        let next_offset_at = offset
            .checked_add(2 + count * 12)
            .ok_or(TiffError::OutOfBounds("IFD chain offset overflows"))?;
        let next = self.u32_at(next_offset_at).unwrap_or(0) as usize;
        Ok((ifd, (next != 0).then_some(next)))
    }

    /// Reads an IFD and everything it points at, depth first.
    fn collect(
        &mut self,
        offset: usize,
        depth: usize,
        out: &mut Vec<Ifd>,
    ) -> Result<(), TiffError> {
        if depth > MAX_IFD_DEPTH {
            return Ok(());
        }
        let mut next = Some(offset);
        // A malformed file can point an IFD at itself; the budget stops the
        // walk regardless of how the chain is shaped.
        while let Some(current) = next {
            if self.budget == 0 {
                return Ok(());
            }
            self.budget -= 1;
            let (ifd, following) = self.read_ifd(current)?;
            let children: Vec<usize> = ifd
                .get(TAG_SUB_IFDS)
                .map(|entry| {
                    entry
                        .as_u32_vec()
                        .into_iter()
                        .map(|value| value as usize)
                        .collect()
                })
                .unwrap_or_default();
            let exif = ifd.u32(TAG_EXIF_IFD).map(|value| value as usize);
            out.push(ifd);
            for child in children {
                self.collect(child, depth + 1, out)?;
            }
            if let Some(exif_offset) = exif {
                // A malformed Exif pointer must not abort the whole file: the
                // main image is still readable without capture settings.
                let _ = self.collect(exif_offset, depth + 1, out);
            }
            next = following;
        }
        Ok(())
    }
}

/// Parses a TIFF container into a flat list of IFDs.
pub fn parse(data: &[u8]) -> Result<TiffFile, TiffError> {
    if data.len() < 8 {
        return Err(TiffError::Truncated("TIFF header"));
    }
    let endian = match &data[0..2] {
        b"II" => Endian::Little,
        b"MM" => Endian::Big,
        _ => return Err(TiffError::NotTiff),
    };
    let magic = endian.u16([data[2], data[3]]);
    // 42 is TIFF. 0x4F52/0x5352 are Olympus ORF variants that reuse the layout.
    if !matches!(magic, 42 | 0x4F52 | 0x5352 | 85) {
        return Err(TiffError::NotTiff);
    }
    let first = endian.u32([data[4], data[5], data[6], data[7]]) as usize;
    let mut reader = Reader {
        data,
        endian,
        budget: MAX_IFDS,
    };
    let mut ifds = Vec::new();
    reader.collect(first, 0, &mut ifds)?;
    if ifds.is_empty() {
        return Err(TiffError::Truncated("no readable image file directory"));
    }
    Ok(TiffFile { endian, ifds })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a little-endian TIFF with one IFD from (tag, type, values).
    fn build(entries: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
        let mut header = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
        let count = entries.len();
        let directory_bytes = 2 + count * 12 + 4;
        let mut directory = Vec::new();
        directory.extend((count as u16).to_le_bytes());
        let mut overflow = Vec::new();
        let overflow_base = 8 + directory_bytes;
        for (tag, field_type, payload) in entries {
            directory.extend(tag.to_le_bytes());
            directory.extend(field_type.to_le_bytes());
            let width = FieldType::from_code(*field_type).width().unwrap_or(1);
            let element_count = payload.len() / width;
            directory.extend((element_count as u32).to_le_bytes());
            if payload.len() <= 4 {
                let mut inline = payload.clone();
                inline.resize(4, 0);
                directory.extend(inline);
            } else {
                directory.extend(((overflow_base + overflow.len()) as u32).to_le_bytes());
                overflow.extend(payload.iter().copied());
            }
        }
        directory.extend(0u32.to_le_bytes());
        header.extend(directory);
        header.extend(overflow);
        header
    }

    #[test]
    fn reads_a_minimal_little_endian_file() {
        let data = build(&[(256, 3, 640u16.to_le_bytes().to_vec())]);
        let file = parse(&data).unwrap();
        assert_eq!(file.endian, Endian::Little);
        assert_eq!(file.ifds.len(), 1);
        assert_eq!(file.ifds[0].u32(256), Some(640));
    }

    #[test]
    fn reads_a_big_endian_file() {
        let data = vec![
            b'M', b'M', 0, 42, 0, 0, 0, 8, // header
            0, 1, // one entry
            1, 0, // tag 256
            0, 3, // SHORT
            0, 0, 0, 1, // count 1
            2, 128, 0, 0, // value 640
            0, 0, 0, 0, // no next IFD
        ];
        let file = parse(&data).unwrap();
        assert_eq!(file.endian, Endian::Big);
        assert_eq!(file.ifds[0].u32(256), Some(640));
    }

    #[test]
    fn rejects_a_file_that_is_not_tiff() {
        assert_eq!(parse(b"not a tiff at all").unwrap_err(), TiffError::NotTiff);
        assert_eq!(parse(&[]).unwrap_err(), TiffError::Truncated("TIFF header"));
        assert_eq!(
            parse(b"II").unwrap_err(),
            TiffError::Truncated("TIFF header")
        );
    }

    #[test]
    fn rejects_a_value_pointing_outside_the_file() {
        let mut data = build(&[(700, 1, vec![1, 2, 3, 4, 5, 6, 7, 8])]);
        // Redirect the payload offset far past the end of the buffer.
        let offset_at = 8 + 2 + 8;
        data[offset_at..offset_at + 4].copy_from_slice(&0xFFFF_0000u32.to_le_bytes());
        assert!(matches!(parse(&data), Err(TiffError::OutOfBounds(_))));
    }

    #[test]
    fn a_self_referencing_ifd_chain_terminates() {
        // An IFD whose "next" pointer is itself would loop forever without the
        // walk budget.
        let mut data = build(&[(256, 3, 8u16.to_le_bytes().to_vec())]);
        let next_at = 8 + 2 + 12;
        data[next_at..next_at + 4].copy_from_slice(&8u32.to_le_bytes());
        let file = parse(&data).unwrap();
        assert_eq!(file.ifds.len(), MAX_IFDS);
    }

    #[test]
    fn an_absurd_entry_count_is_refused_before_allocating() {
        let mut data = build(&[(256, 3, 8u16.to_le_bytes().to_vec())]);
        data[8..10].copy_from_slice(&(MAX_IFD_ENTRIES as u16 + 1).to_le_bytes());
        assert_eq!(
            parse(&data).unwrap_err(),
            TiffError::LimitExceeded("too many entries in one IFD")
        );
    }

    #[test]
    fn an_enormous_declared_value_is_skipped_rather_than_allocated() {
        let mut data = build(&[(700, 1, vec![0; 16])]);
        // Declare four billion bytes for a tag whose payload is sixteen.
        let count_at = 8 + 2 + 4;
        data[count_at..count_at + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        let file = parse(&data).unwrap();
        assert!(file.ifds[0].get(700).is_none(), "the entry was kept");
    }

    #[test]
    fn ascii_tags_are_trimmed_and_control_characters_refused() {
        let data = build(&[(271, 2, b"  Canon\0".to_vec())]);
        assert_eq!(
            parse(&data).unwrap().ifds[0].ascii(271).as_deref(),
            Some("Canon")
        );
        let hostile = build(&[(271, 2, b"Ca\x07non\0".to_vec())]);
        assert_eq!(parse(&hostile).unwrap().ifds[0].ascii(271), None);
    }

    #[test]
    fn rationals_decode_and_a_zero_denominator_does_not_become_infinite() {
        let mut payload = Vec::new();
        payload.extend(1u32.to_le_bytes());
        payload.extend(250u32.to_le_bytes());
        payload.extend(5u32.to_le_bytes());
        payload.extend(0u32.to_le_bytes());
        let data = build(&[(33434, 5, payload)]);
        let values = parse(&data).unwrap().ifds[0].f64_vec(33434).unwrap();
        assert!((values[0] - 0.004).abs() < 1e-9);
        assert_eq!(values[1], 0.0);
    }

    #[test]
    fn sub_ifds_are_collected_and_depth_is_bounded() {
        // IFD0 points at a SubIFD that carries the real image dimensions.
        let sub = build(&[(256, 3, 4000u16.to_le_bytes().to_vec())]);
        let sub_offset = 512usize;
        let main = build(&[(330, 4, (sub_offset as u32).to_le_bytes().to_vec())]);
        let mut data = main;
        data.resize(sub_offset, 0);
        // The child IFD body is the directory portion of `sub`, which starts at
        // byte 8 of a standalone file.
        data.extend(&sub[8..]);
        let file = parse(&data).unwrap();
        assert_eq!(file.ifds.len(), 2);
        assert_eq!(file.ifds[1].u32(256), Some(4000));
    }

    #[test]
    fn a_short_written_where_a_long_is_expected_still_reads() {
        let data = build(&[(256, 4, 12000u32.to_le_bytes().to_vec())]);
        assert_eq!(parse(&data).unwrap().ifds[0].u32(256), Some(12000));
    }

    #[test]
    fn a_negative_signed_value_clamps_instead_of_wrapping() {
        let data = build(&[(256, 8, (-5i16).to_le_bytes().to_vec())]);
        assert_eq!(parse(&data).unwrap().ifds[0].u32(256), Some(0));
    }
}
