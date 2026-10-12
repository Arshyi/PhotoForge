//! The directories of a TIFF file, read by seeking.
//!
//! Every offset, count and length in the file is a claim from a stranger. Nothing is allocated
//! from a declared size before it has been checked against the file's own length and a fixed cap,
//! a directory is visited once, and a chain of them is walked no further than [`MAX_IFDS`].
//! Reading is by seeking because the directory of a TIFF is as often after its pixels as before
//! them, and a photograph of a hundred megapixels is not held in memory to find its width.
//!
//! Only classic TIFF (32-bit offsets, magic 42) is read. BigTIFF is named and refused.
use crate::error::AppError;
use std::collections::{BTreeMap, HashSet};
use std::io::{Read, Seek, SeekFrom};

/// The most directories walked in one chain.
pub const MAX_IFDS: usize = 64;
/// The most entries one directory may declare.
pub const MAX_ENTRIES: usize = 4096;
/// The most bytes one tag's value may take: an ICC profile or an offset table.
pub const MAX_VALUE_BYTES: u64 = 32 << 20;
/// The most strips or tiles a picture may be stored in.
pub const MAX_SEGMENTS: u64 = 1 << 22;

fn refuse(message: impl Into<String>) -> AppError {
    AppError::foreign_file(message.into())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    Little,
    Big,
}

impl Order {
    pub fn u16(self, bytes: [u8; 2]) -> u16 {
        match self {
            Self::Little => u16::from_le_bytes(bytes),
            Self::Big => u16::from_be_bytes(bytes),
        }
    }

    pub fn u32(self, bytes: [u8; 4]) -> u32 {
        match self {
            Self::Little => u32::from_le_bytes(bytes),
            Self::Big => u32::from_be_bytes(bytes),
        }
    }
}

/// Bytes in one value of a field type, or `None` for a type this reader does not know.
pub fn kind_width(kind: u16) -> Option<u64> {
    match kind {
        1 | 2 | 6 | 7 => Some(1),
        3 | 8 => Some(2),
        4 | 9 | 11 | 13 => Some(4),
        5 | 10 | 12 => Some(8),
        _ => None,
    }
}

/// One directory entry, as it lies in the file: the value is not read until it is asked for.
#[derive(Debug, Clone)]
pub struct Entry {
    pub tag: u16,
    pub kind: u16,
    pub count: u32,
    /// The four bytes of the value field: the value itself when it fits, otherwise an offset.
    field: [u8; 4],
}

#[derive(Debug, Clone, Default)]
pub struct Directory {
    pub entries: BTreeMap<u16, Entry>,
}

impl Directory {
    pub fn get(&self, tag: u16) -> Option<&Entry> {
        self.entries.get(&tag)
    }

    pub fn has(&self, tag: u16) -> bool {
        self.entries.contains_key(&tag)
    }
}

/// A TIFF file opened for reading.
pub struct TiffFile<R> {
    reader: R,
    length: u64,
    order: Order,
    first: u32,
}

impl<R: Read + Seek> TiffFile<R> {
    /// Checks the header. `Err` names what the file is when it is not a classic TIFF.
    pub fn open(mut reader: R) -> Result<Self, AppError> {
        let length = reader
            .seek(SeekFrom::End(0))
            .map_err(|_| AppError::DecodeFailure)?;
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|_| AppError::DecodeFailure)?;
        let mut header = [0_u8; 8];
        if length < 8 || reader.read_exact(&mut header).is_err() {
            return Err(refuse("this file is too short to be a TIFF"));
        }
        let order = match &header[..2] {
            b"II" => Order::Little,
            b"MM" => Order::Big,
            _ => return Err(refuse("this file is not a TIFF (no byte-order mark)")),
        };
        match order.u16([header[2], header[3]]) {
            42 => {}
            43 => {
                return Err(refuse(
                    "this is a BigTIFF file (64-bit offsets); only classic TIFF is read",
                ))
            }
            _ => return Err(refuse("this file is not a TIFF (wrong version number)")),
        }
        let first = order.u32([header[4], header[5], header[6], header[7]]);
        Ok(Self {
            reader,
            length,
            order,
            first,
        })
    }

    pub fn order(&self) -> Order {
        self.order
    }

    pub fn length(&self) -> u64 {
        self.length
    }

    /// `length` bytes at `offset`, when they lie inside the file and within `cap`.
    pub fn read_at(&mut self, offset: u64, length: u64, cap: u64) -> Result<Vec<u8>, AppError> {
        if length > cap {
            return Err(refuse(format!(
                "a part of the file declares {length} bytes where at most {cap} are read"
            )));
        }
        let end = offset
            .checked_add(length)
            .ok_or_else(|| refuse("an offset in the file overflows"))?;
        if end > self.length {
            return Err(refuse(
                "the file ends inside a structure that declares more data (it is truncated)",
            ));
        }
        self.reader
            .seek(SeekFrom::Start(offset))
            .map_err(|_| AppError::DecodeFailure)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length as usize)
            .map_err(|_| AppError::OutOfMemoryRisk)?;
        bytes.resize(length as usize, 0);
        self.reader
            .read_exact(&mut bytes)
            .map_err(|_| refuse("the file could not be read to the end of a structure"))?;
        Ok(bytes)
    }

    fn directory_at(&mut self, offset: u64) -> Result<(Directory, u32), AppError> {
        let head = self.read_at(offset, 2, 2)?;
        let count = usize::from(self.order.u16([head[0], head[1]]));
        if count > MAX_ENTRIES {
            return Err(refuse(format!(
                "a directory declares {count} entries; at most {MAX_ENTRIES} are read"
            )));
        }
        let body_len = 2 + 12 * count as u64 + 4;
        let body = self.read_at(offset, body_len, body_len)?;
        let mut directory = Directory::default();
        for index in 0..count {
            let at = 2 + index * 12;
            let tag = self.order.u16([body[at], body[at + 1]]);
            let kind = self.order.u16([body[at + 2], body[at + 3]]);
            let value_count =
                self.order
                    .u32([body[at + 4], body[at + 5], body[at + 6], body[at + 7]]);
            // A type this reader does not know has no width, so its value cannot be located;
            // the entry is left out and the rest of the directory stays readable.
            if kind_width(kind).is_none() {
                continue;
            }
            directory.entries.entry(tag).or_insert(Entry {
                tag,
                kind,
                count: value_count,
                field: [body[at + 8], body[at + 9], body[at + 10], body[at + 11]],
            });
        }
        let end = 2 + 12 * count;
        let next = self
            .order
            .u32([body[end], body[end + 1], body[end + 2], body[end + 3]]);
        Ok((directory, next))
    }

    /// The directories of the first chain, in order. A chain that loops, or that is longer than
    /// [`MAX_IFDS`], is cut there; a directory that cannot be read ends the chain and is not an
    /// error unless it is the first.
    pub fn directories(&mut self) -> Result<Vec<Directory>, AppError> {
        let mut found = Vec::new();
        let mut seen = HashSet::new();
        let mut next = self.first;
        while next != 0 && found.len() < MAX_IFDS && seen.insert(next) {
            match self.directory_at(u64::from(next)) {
                Ok((directory, following)) => {
                    found.push(directory);
                    next = following;
                }
                Err(error) if found.is_empty() => return Err(error),
                Err(_) => break,
            }
        }
        if found.is_empty() {
            return Err(refuse("the file holds no image directory"));
        }
        Ok(found)
    }

    /// The directory at `offset`, without following its chain.
    pub fn directory(&mut self, offset: u32) -> Result<Directory, AppError> {
        self.directory_at(u64::from(offset))
            .map(|(directory, _)| directory)
    }

    /// The bytes of an entry's value.
    pub fn value_bytes(&mut self, entry: &Entry) -> Result<Vec<u8>, AppError> {
        let width = kind_width(entry.kind).ok_or_else(|| refuse("an unknown field type"))?;
        let size = width
            .checked_mul(u64::from(entry.count))
            .ok_or_else(|| refuse("a value's size overflows"))?;
        if size <= 4 {
            return Ok(entry.field[..size as usize].to_vec());
        }
        let offset = u64::from(self.order.u32(entry.field));
        self.read_at(offset, size, MAX_VALUE_BYTES)
    }

    /// An integer-valued entry as unsigned 32-bit values (BYTE, SHORT, LONG and their signed
    /// forms, the signed ones clamped at zero).
    pub fn u32s(&mut self, entry: &Entry) -> Result<Vec<u32>, AppError> {
        let bytes = self.value_bytes(entry)?;
        let order = self.order;
        Ok(match entry.kind {
            1 | 7 => bytes.iter().map(|b| u32::from(*b)).collect(),
            6 => bytes
                .iter()
                .map(|b| u32::from((*b as i8).max(0) as u8))
                .collect(),
            3 => bytes
                .chunks_exact(2)
                .map(|c| u32::from(order.u16([c[0], c[1]])))
                .collect(),
            8 => bytes
                .chunks_exact(2)
                .map(|c| u32::from((order.u16([c[0], c[1]]) as i16).max(0) as u16))
                .collect(),
            4 | 13 => bytes
                .chunks_exact(4)
                .map(|c| order.u32([c[0], c[1], c[2], c[3]]))
                .collect(),
            9 => bytes
                .chunks_exact(4)
                .map(|c| (order.u32([c[0], c[1], c[2], c[3]]) as i32).max(0) as u32)
                .collect(),
            _ => return Err(refuse("a value that should be an integer is not")),
        })
    }

    /// The first integer of an entry, if it has one.
    pub fn first_u32(&mut self, directory: &Directory, tag: u16) -> Result<Option<u32>, AppError> {
        match directory.get(tag) {
            Some(entry) if entry.count > 0 => Ok(self.u32s(entry)?.first().copied()),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn little(entries: &[(u16, u16, u32, u32)]) -> Vec<u8> {
        let mut file = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
        file.extend((entries.len() as u16).to_le_bytes());
        for (tag, kind, count, value) in entries {
            file.extend(tag.to_le_bytes());
            file.extend(kind.to_le_bytes());
            file.extend(count.to_le_bytes());
            file.extend(value.to_le_bytes());
        }
        file.extend(0_u32.to_le_bytes());
        file
    }

    #[test]
    fn a_header_and_a_directory_are_read() {
        let file = little(&[(256, 4, 1, 640), (257, 3, 1, 480)]);
        let mut tiff = TiffFile::open(Cursor::new(file)).unwrap();
        assert_eq!(tiff.order(), Order::Little);
        let directories = tiff.directories().unwrap();
        assert_eq!(directories.len(), 1);
        let width = directories[0].get(256).unwrap().clone();
        assert_eq!(tiff.u32s(&width).unwrap(), vec![640]);
        assert_eq!(tiff.first_u32(&directories[0], 257).unwrap(), Some(480));
        assert_eq!(tiff.first_u32(&directories[0], 999).unwrap(), None);
    }

    #[test]
    fn what_is_not_a_classic_tiff_is_named() {
        let name = |bytes: &[u8]| {
            TiffFile::open(Cursor::new(bytes.to_vec()))
                .err()
                .unwrap()
                .to_string()
        };
        assert!(name(b"II+\0\x08\0\0\0").contains("BigTIFF"));
        assert!(name(b"XX*\0\x08\0\0\0").contains("not a TIFF"));
        assert!(name(b"II").contains("too short"));
        assert!(name(b"II\x2B\x01\x08\0\0\0").contains("version"));
    }

    #[test]
    fn a_loop_in_the_chain_is_cut_and_a_long_chain_is_bounded() {
        // The first directory's next pointer names itself.
        let mut file = little(&[(256, 4, 1, 4)]);
        let next_at = 8 + 2 + 12;
        file[next_at..next_at + 4].copy_from_slice(&8_u32.to_le_bytes());
        let mut tiff = TiffFile::open(Cursor::new(file)).unwrap();
        assert_eq!(tiff.directories().unwrap().len(), 1);
    }

    #[test]
    fn a_value_past_the_end_of_the_file_is_refused_before_it_is_allocated() {
        let file = little(&[(273, 4, 1_000_000, 0xFFFF_FF00)]);
        let mut tiff = TiffFile::open(Cursor::new(file)).unwrap();
        let directory = tiff.directories().unwrap().remove(0);
        let entry = directory.get(273).unwrap().clone();
        let error = tiff.u32s(&entry).unwrap_err().to_string();
        assert!(error.contains("declares"), "{error}");
    }

    #[test]
    fn an_absurd_entry_count_is_refused() {
        let mut file = little(&[(256, 4, 1, 4)]);
        file[8..10].copy_from_slice(&u16::MAX.to_le_bytes());
        let mut tiff = TiffFile::open(Cursor::new(file)).unwrap();
        assert!(tiff.directories().is_err());
    }

    #[test]
    fn signed_values_do_not_wrap() {
        let file = little(&[(256, 9, 1, (-5_i32) as u32)]);
        let mut tiff = TiffFile::open(Cursor::new(file)).unwrap();
        let directory = tiff.directories().unwrap().remove(0);
        let entry = directory.get(256).unwrap().clone();
        assert_eq!(tiff.u32s(&entry).unwrap(), vec![0]);
    }

    #[test]
    fn an_unknown_field_type_leaves_the_rest_of_the_directory_readable() {
        let file = little(&[(256, 4, 1, 4), (300, 99, 1, 0), (257, 4, 1, 8)]);
        let mut tiff = TiffFile::open(Cursor::new(file)).unwrap();
        let directory = tiff.directories().unwrap().remove(0);
        assert!(!directory.has(300));
        assert!(directory.has(257));
    }
}
