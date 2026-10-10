//! EXIF, read and cleaned as the TIFF structure it is.
//!
//! An EXIF block is a small TIFF file: a header, then linked directories (IFDs) of tagged entries
//! whose values are inline when they fit in four bytes and elsewhere in the block when they do
//! not. Every offset and count in it is a claim from a file this program did not write, so each
//! is checked against the block's length before it is followed, directories are visited once,
//! and nothing here allocates from a declared size.
//!
//! Cleaning works **in place**. The block keeps its layout, so offsets inside vendor-specific
//! data that this reader does not understand stay valid, and what is removed is overwritten with
//! zeros where it lies, not merely unlinked: an entry whose bytes are still in the file is not
//! removed. An entry is deleted by moving the entries after it up and shrinking the count.
//!
//! When a block cannot be walked safely, cleaning refuses (`None`) and the caller drops the whole
//! block, because a block that cannot be understood cannot be promised to be free of anything.

use std::collections::HashSet;

/// A block larger than this is not carried at all.
pub const MAX_EXIF_BYTES: usize = 1 << 20;
const MAX_ENTRIES: usize = 4096;
const MAX_CHAIN: usize = 8;

const TAG_IMAGE_WIDTH: u16 = 0x0100;
const TAG_IMAGE_LENGTH: u16 = 0x0101;
const TAG_MAKE: u16 = 0x010F;
const TAG_MODEL: u16 = 0x0110;
const TAG_ORIENTATION: u16 = 0x0112;
const TAG_STRIP_OFFSETS: u16 = 0x0111;
const TAG_STRIP_BYTE_COUNTS: u16 = 0x0117;
const TAG_THUMBNAIL_OFFSET: u16 = 0x0201;
const TAG_THUMBNAIL_LENGTH: u16 = 0x0202;
const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_GPS_IFD: u16 = 0x8825;
const TAG_DATE_TIME_ORIGINAL: u16 = 0x9003;
const TAG_MAKER_NOTE: u16 = 0x927C;
const TAG_COLOUR_SPACE: u16 = 0xA001;
const TAG_PIXEL_X: u16 = 0xA002;
const TAG_PIXEL_Y: u16 = 0xA003;
const TAG_LENS_MODEL: u16 = 0xA434;
/// Who owns the camera, and the serial numbers of body and lens: they identify a person or a
/// device, not a picture.
const IDENTIFYING_TAGS: [u16; 5] = [0xA430, 0xA431, 0xA435, 0xA420, 0xC62F];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Order {
    Little,
    Big,
}

fn order_of(block: &[u8]) -> Option<Order> {
    match block.get(..4)? {
        b"II*\0" => Some(Order::Little),
        b"MM\0*" => Some(Order::Big),
        _ => None,
    }
}

fn get_u16(block: &[u8], order: Order, at: usize) -> Option<u16> {
    let bytes: [u8; 2] = block.get(at..at.checked_add(2)?)?.try_into().ok()?;
    Some(match order {
        Order::Little => u16::from_le_bytes(bytes),
        Order::Big => u16::from_be_bytes(bytes),
    })
}

fn get_u32(block: &[u8], order: Order, at: usize) -> Option<u32> {
    let bytes: [u8; 4] = block.get(at..at.checked_add(4)?)?.try_into().ok()?;
    Some(match order {
        Order::Little => u32::from_le_bytes(bytes),
        Order::Big => u32::from_be_bytes(bytes),
    })
}

fn put_u16(block: &mut [u8], order: Order, at: usize, value: u16) -> Option<()> {
    let bytes = match order {
        Order::Little => value.to_le_bytes(),
        Order::Big => value.to_be_bytes(),
    };
    block
        .get_mut(at..at.checked_add(2)?)?
        .copy_from_slice(&bytes);
    Some(())
}

fn put_u32(block: &mut [u8], order: Order, at: usize, value: u32) -> Option<()> {
    let bytes = match order {
        Order::Little => value.to_le_bytes(),
        Order::Big => value.to_be_bytes(),
    };
    block
        .get_mut(at..at.checked_add(4)?)?
        .copy_from_slice(&bytes);
    Some(())
}

fn kind_size(kind: u16) -> u64 {
    match kind {
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 | 16..=18 => 8,
        // BYTE, ASCII, SBYTE, UNDEFINED, and what is not known: a byte each, as TIFF says.
        _ => 1,
    }
}

#[derive(Debug, Clone)]
struct Entry {
    /// Where the 12 bytes of the entry start.
    at: usize,
    tag: u16,
    kind: u16,
    count: u32,
    /// Where the value is and how long it is, when that lies inside the block.
    data: Option<(usize, usize)>,
}

#[derive(Debug, Clone)]
struct Ifd {
    at: usize,
    entries: Vec<Entry>,
    next_at: usize,
    next: u32,
}

impl Ifd {
    fn find(&self, tag: u16) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.tag == tag)
    }

    /// Bytes from the count to the end of the next-directory pointer.
    fn span(&self) -> usize {
        self.next_at + 4 - self.at
    }
}

fn read_ifd(block: &[u8], order: Order, at: usize) -> Option<Ifd> {
    let count = usize::from(get_u16(block, order, at)?);
    if count > MAX_ENTRIES {
        return None;
    }
    let first = at.checked_add(2)?;
    let next_at = first.checked_add(count.checked_mul(12)?)?;
    if next_at.checked_add(4)? > block.len() {
        return None;
    }
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let entry_at = first + index * 12;
        let tag = get_u16(block, order, entry_at)?;
        let kind = get_u16(block, order, entry_at + 2)?;
        let count = get_u32(block, order, entry_at + 4)?;
        let size = kind_size(kind).checked_mul(u64::from(count))?;
        let data = if size == 0 {
            None
        } else if size <= 4 {
            Some((entry_at + 8, size as usize))
        } else if size > MAX_EXIF_BYTES as u64 {
            None
        } else {
            let offset = get_u32(block, order, entry_at + 8)? as usize;
            let end = offset.checked_add(size as usize)?;
            (end <= block.len()).then_some((offset, size as usize))
        };
        entries.push(Entry {
            at: entry_at,
            tag,
            kind,
            count,
            data,
        });
    }
    let next = get_u32(block, order, next_at)?;
    Some(Ifd {
        at,
        entries,
        next_at,
        next,
    })
}

/// A value that is one offset: the pointer entries (Exif, GPS).
fn pointer(block: &[u8], order: Order, ifd: &Ifd, tag: u16) -> Option<usize> {
    let entry = ifd.find(tag)?;
    if entry.count != 1 || !matches!(entry.kind, 4 | 13) {
        return None;
    }
    let value = get_u32(block, order, entry.at + 8)? as usize;
    (value != 0).then_some(value)
}

struct Layout {
    order: Order,
    ifd0: Ifd,
    exif: Option<Ifd>,
    gps: Option<Ifd>,
}

fn layout(block: &[u8]) -> Option<Layout> {
    if block.len() > MAX_EXIF_BYTES {
        return None;
    }
    let order = order_of(block)?;
    let first = get_u32(block, order, 4)? as usize;
    let ifd0 = read_ifd(block, order, first)?;
    // A pointer that is present and cannot be followed is not a block to vouch for.
    let follow = |tag: u16| -> Option<Option<Ifd>> {
        match ifd0.find(tag) {
            None => Some(None),
            Some(_) => {
                let at = pointer(block, order, &ifd0, tag)?;
                Some(Some(read_ifd(block, order, at)?))
            }
        }
    };
    let exif = follow(TAG_EXIF_IFD)?;
    let gps = follow(TAG_GPS_IFD)?;
    Some(Layout {
        order,
        ifd0,
        exif,
        gps,
    })
}

fn ascii(block: &[u8], entry: &Entry) -> Option<String> {
    let (at, len) = entry.data?;
    if entry.kind != 2 {
        return None;
    }
    let bytes = block.get(at..at.checked_add(len)?)?;
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    let text: String = String::from_utf8_lossy(&bytes[..end])
        .chars()
        .filter(|c| !c.is_control())
        .take(100)
        .collect();
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// What an EXIF block says, for the interface and for deciding what an export would drop.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub make: Option<String>,
    pub model: Option<String>,
    pub lens: Option<String>,
    /// When the picture was taken, as the camera wrote it ("2026:10:10 14:03:09").
    pub taken: Option<String>,
    /// 1 to 8; 1 when the block says nothing.
    pub orientation: u8,
    pub has_location: bool,
    pub has_thumbnail: bool,
    pub has_maker_note: bool,
    /// An owner's name or a serial number.
    pub has_identifiers: bool,
    /// The picture size the block records (Exif PixelXDimension and PixelYDimension).
    pub pixel_dimensions: Option<(u32, u32)>,
}

/// Reads the block. `None` when it is not a TIFF structure this reader will walk.
pub fn summarize(block: &[u8]) -> Option<Summary> {
    let layout = layout(block)?;
    let order = layout.order;
    let text = |ifd: &Ifd, tag: u16| ifd.find(tag).and_then(|entry| ascii(block, entry));
    let orientation = layout
        .ifd0
        .find(TAG_ORIENTATION)
        .and_then(|entry| get_u16(block, order, entry.at + 8))
        .filter(|value| (1..=8).contains(value))
        .unwrap_or(1) as u8;
    let exif = layout.exif.as_ref();
    let has_identifiers = IDENTIFYING_TAGS.iter().any(|tag| {
        exif.is_some_and(|ifd| ifd.find(*tag).is_some()) || layout.ifd0.find(*tag).is_some()
    });
    let has_thumbnail = layout.ifd0.next != 0;
    let number = |ifd: &Ifd, tag: u16| -> Option<u32> {
        let entry = ifd.find(tag)?;
        if entry.count != 1 {
            return None;
        }
        match entry.kind {
            3 => get_u16(block, order, entry.at + 8).map(u32::from),
            4 => get_u32(block, order, entry.at + 8),
            _ => None,
        }
    };
    let pixel_dimensions =
        exif.and_then(|ifd| Some((number(ifd, TAG_PIXEL_X)?, number(ifd, TAG_PIXEL_Y)?)));
    Some(Summary {
        make: text(&layout.ifd0, TAG_MAKE),
        model: text(&layout.ifd0, TAG_MODEL),
        lens: exif.and_then(|ifd| text(ifd, TAG_LENS_MODEL)),
        taken: exif.and_then(|ifd| text(ifd, TAG_DATE_TIME_ORIGINAL)),
        orientation,
        has_location: layout
            .gps
            .as_ref()
            .is_some_and(|gps| !gps.entries.is_empty()),
        has_thumbnail,
        has_maker_note: exif.is_some_and(|ifd| ifd.find(TAG_MAKER_NOTE).is_some()),
        has_identifiers,
        pixel_dimensions,
    })
}

/// The orientation the block names (1 to 8), if it names one. Only the first directory is read,
/// so a block with a damaged place or maker note still turns a picture upright.
pub fn orientation(block: &[u8]) -> Option<u8> {
    if block.len() > MAX_EXIF_BYTES {
        return None;
    }
    let order = order_of(block)?;
    let first = get_u32(block, order, 4)? as usize;
    let ifd0 = read_ifd(block, order, first)?;
    let entry = ifd0.find(TAG_ORIENTATION)?;
    let value = get_u16(block, order, entry.at + 8)?;
    (1..=8).contains(&value).then_some(value as u8)
}

/// A block holding nothing but an orientation, for a file that is written without the rest and
/// whose pixels could not be turned upright.
pub fn orientation_only(orientation: u8) -> Vec<u8> {
    let mut block = Vec::with_capacity(26);
    block.extend_from_slice(b"II*\0");
    block.extend_from_slice(&8_u32.to_le_bytes());
    block.extend_from_slice(&1_u16.to_le_bytes());
    block.extend_from_slice(&TAG_ORIENTATION.to_le_bytes());
    block.extend_from_slice(&3_u16.to_le_bytes());
    block.extend_from_slice(&1_u32.to_le_bytes());
    block.extend_from_slice(&u16::from(orientation).to_le_bytes());
    block.extend_from_slice(&[0, 0]);
    block.extend_from_slice(&0_u32.to_le_bytes());
    block
}

/// What a clean was asked to do.
#[derive(Debug, Clone, Copy, Default)]
pub struct CleanSpec {
    /// Remove the location, the maker note, and the owner and serial numbers.
    pub private: bool,
    /// The orientation to write, when the pixels have already been turned upright.
    pub orientation: Option<u8>,
    /// The dimensions of the picture the block now travels with.
    pub dimensions: Option<(u32, u32)>,
    /// Say the colour space is not sRGB (tag 0xFFFF), because the file carries another profile.
    pub colour_space_uncalibrated: bool,
}

/// What a clean took out.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Removed {
    pub location: bool,
    pub thumbnail: bool,
    pub maker_note: bool,
    pub identifiers: bool,
}

#[derive(Debug, Clone)]
pub struct Cleaned {
    pub bytes: Vec<u8>,
    pub removed: Removed,
}

fn zero(block: &mut [u8], at: usize, len: usize) -> Option<()> {
    block
        .get_mut(at..at.checked_add(len)?)?
        .iter_mut()
        .for_each(|byte| *byte = 0);
    Some(())
}

/// Overwrites a directory and every value it points at, so nothing it held is left in the block.
fn erase_ifd(block: &mut [u8], ifd: &Ifd) -> Option<()> {
    for entry in &ifd.entries {
        if let Some((at, len)) = entry.data {
            // Inline values live in the entry itself and are erased with it.
            if at != entry.at + 8 {
                zero(block, at, len)?;
            }
        }
    }
    zero(block, ifd.at, ifd.span())
}

/// Removes every entry whose tag is in `tags` from the directory at `at`, erasing the values.
fn delete_tags(block: &mut [u8], order: Order, at: usize, tags: &[u16]) -> Option<usize> {
    let mut deleted = 0;
    loop {
        let ifd = read_ifd(block, order, at)?;
        let Some(index) = ifd
            .entries
            .iter()
            .position(|entry| tags.contains(&entry.tag))
        else {
            return Some(deleted);
        };
        let entry = ifd.entries[index].clone();
        if let Some((value_at, len)) = entry.data {
            if value_at != entry.at + 8 {
                zero(block, value_at, len)?;
            }
        }
        // Move what follows (the later entries and the next-directory pointer) up by one entry.
        let from = entry.at + 12;
        let to = entry.at;
        let end = ifd.next_at + 4;
        block.copy_within(from..end, to);
        zero(block, end - 12, 12)?;
        let count = (ifd.entries.len() - 1) as u16;
        put_u16(block, order, at, count)?;
        deleted += 1;
    }
}

/// Sets a SHORT-or-LONG entry that is one value, in place. `Ok(false)` when there is no entry.
fn set_number(block: &mut [u8], order: Order, at: usize, tag: u16, value: u32) -> Option<bool> {
    let ifd = read_ifd(block, order, at)?;
    let Some(entry) = ifd.find(tag).cloned() else {
        return Some(false);
    };
    if entry.count != 1 {
        return Some(false);
    }
    match entry.kind {
        3 if value <= u32::from(u16::MAX) => {
            put_u16(block, order, entry.at + 8, value as u16)?;
            // The rest of the four-byte field is padding.
            zero(block, entry.at + 10, 2)?;
        }
        3 | 4 => {
            put_u16(block, order, entry.at + 2, 4)?;
            put_u32(block, order, entry.at + 8, value)?;
        }
        _ => return Some(false),
    }
    Some(true)
}

/// Cleans a block for a picture that has been edited. `None` when the block cannot be walked
/// safely, in which case it should not be carried at all.
pub fn clean(block: &[u8], spec: &CleanSpec) -> Option<Cleaned> {
    let layout = layout(block)?;
    let order = layout.order;
    let ifd0_at = layout.ifd0.at;
    let exif_at = layout.exif.as_ref().map(|ifd| ifd.at);
    let mut out = block.to_vec();
    let mut removed = Removed::default();

    // The thumbnail is always removed: it is a small copy of the picture as it was, and the
    // picture has been edited (a crop leaves the whole of the original in it).
    let mut seen = HashSet::new();
    let mut at = layout.ifd0.next as usize;
    while at != 0 && seen.len() < MAX_CHAIN && seen.insert(at) {
        let ifd = read_ifd(&out, order, at)?;
        // An uncompressed thumbnail is held in strips; one strip is handled, more are not.
        if ifd.find(TAG_STRIP_OFFSETS).is_some() {
            let strips = ifd.find(TAG_STRIP_OFFSETS)?;
            if strips.count != 1 {
                return None;
            }
            let at_value = get_u32(&out, order, strips.at + 8)? as usize;
            let length =
                ifd.find(TAG_STRIP_BYTE_COUNTS)
                    .and_then(|entry| get_u32(&out, order, entry.at + 8))? as usize;
            zero(&mut out, at_value, length)?;
        }
        if let (Some(offset), Some(length)) = (
            ifd.find(TAG_THUMBNAIL_OFFSET),
            ifd.find(TAG_THUMBNAIL_LENGTH),
        ) {
            let offset = get_u32(&out, order, offset.at + 8)? as usize;
            let length = get_u32(&out, order, length.at + 8)? as usize;
            zero(&mut out, offset, length)?;
        }
        let next = ifd.next as usize;
        erase_ifd(&mut out, &ifd)?;
        removed.thumbnail = true;
        at = next;
    }
    if layout.ifd0.next != 0 {
        let ifd0 = read_ifd(&out, order, ifd0_at)?;
        put_u32(&mut out, order, ifd0.next_at, 0)?;
    }

    if spec.private {
        if let Some(gps) = &layout.gps {
            if !gps.entries.is_empty() {
                removed.location = true;
            }
            erase_ifd(&mut out, gps)?;
            // The pointer is deleted as well, so the block does not name a directory of zeros.
            delete_tags(&mut out, order, ifd0_at, &[TAG_GPS_IFD])?;
        }
        if let Some(exif_at) = exif_at {
            removed.maker_note = delete_tags(&mut out, order, exif_at, &[TAG_MAKER_NOTE])? > 0;
            removed.identifiers |= delete_tags(&mut out, order, exif_at, &IDENTIFYING_TAGS)? > 0;
        }
        removed.identifiers |= delete_tags(&mut out, order, ifd0_at, &IDENTIFYING_TAGS)? > 0;
    }

    if let Some(value) = spec.orientation {
        set_number(&mut out, order, ifd0_at, TAG_ORIENTATION, u32::from(value))?;
    }
    if let Some((width, height)) = spec.dimensions {
        if let Some(exif_at) = exif_at {
            set_number(&mut out, order, exif_at, TAG_PIXEL_X, width)?;
            set_number(&mut out, order, exif_at, TAG_PIXEL_Y, height)?;
        }
        set_number(&mut out, order, ifd0_at, TAG_IMAGE_WIDTH, width)?;
        set_number(&mut out, order, ifd0_at, TAG_IMAGE_LENGTH, height)?;
    }
    if spec.colour_space_uncalibrated {
        if let Some(exif_at) = exif_at {
            set_number(&mut out, order, exif_at, TAG_COLOUR_SPACE, 0xFFFF)?;
        }
    }

    // What was made must itself be a block this reader will walk, or it is not offered.
    summarize(&out)?;
    Some(Cleaned {
        bytes: out,
        removed,
    })
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::*;

    /// Builds a little-endian EXIF block the way a camera does: IFD0, then the Exif and GPS
    /// directories and their values, then IFD1 with a JPEG thumbnail.
    pub(crate) struct Fixture;

    pub(crate) const THUMBNAIL: &[u8] = b"\xff\xd8THUMBNAIL-OF-THE-WHOLE-PICTURE\xff\xd9";
    pub(crate) const MAKER_NOTE: &[u8] = b"SECRET-MAKER-NOTE-WITH-A-PREVIEW";
    pub(crate) const SERIAL: &[u8] = b"SERIAL-0042\0";

    fn entry(tag: u16, kind: u16, count: u32, value: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
        bytes
    }

    fn short(tag: u16, value: u16) -> Vec<u8> {
        entry(tag, 3, 1, u32::from(value))
    }

    impl Fixture {
        pub(crate) fn build() -> Vec<u8> {
            // Layout, fixed so offsets can be written down: header 8; IFD0 at 8 (7 entries =
            // 2 + 84 + 4 = 90) ends at 98; Exif IFD at 98 (7 entries = 90) ends at 188; GPS IFD
            // at 188 (3 entries = 42) ends at 230; IFD1 at 230 (3 entries = 42) ends at 272;
            // values follow.
            let (ifd0, exif, gps, ifd1) = (8_u32, 98_u32, 188_u32, 230_u32);
            let mut values = Vec::new();
            let base = 272_u32;
            let mut place = |data: &[u8]| -> u32 {
                let at = base + values.len() as u32;
                values.extend_from_slice(data);
                at
            };
            let make = place(b"Acme\0");
            let model = place(b"Cam-One\0");
            let date = place(b"2026:10:10 14:03:09\0");
            let lens = place(b"Lens 35mm f/2\0");
            let note = place(MAKER_NOTE);
            let serial = place(SERIAL);
            let latitude = place(&[
                48, 0, 0, 0, 1, 0, 0, 0, 51, 0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 1, 0, 0, 0,
            ]);
            let thumb = place(THUMBNAIL);

            let mut block = Vec::new();
            block.extend_from_slice(b"II*\0");
            block.extend_from_slice(&ifd0.to_le_bytes());
            // IFD0: make, model, orientation, Exif pointer, GPS pointer, x/y resolution omitted,
            // ImageWidth/Length, in tag order.
            block.extend_from_slice(&7_u16.to_le_bytes());
            block.extend_from_slice(&entry(TAG_IMAGE_WIDTH, 4, 1, 4000));
            block.extend_from_slice(&entry(TAG_IMAGE_LENGTH, 4, 1, 3000));
            block.extend_from_slice(&entry(TAG_MAKE, 2, 5, make));
            block.extend_from_slice(&entry(TAG_MODEL, 2, 8, model));
            block.extend_from_slice(&short(TAG_ORIENTATION, 6));
            block.extend_from_slice(&entry(TAG_EXIF_IFD, 4, 1, exif));
            block.extend_from_slice(&entry(TAG_GPS_IFD, 4, 1, gps));
            block.extend_from_slice(&ifd1.to_le_bytes());
            assert_eq!(block.len() as u32, exif);
            // Exif IFD.
            block.extend_from_slice(&7_u16.to_le_bytes());
            block.extend_from_slice(&entry(TAG_DATE_TIME_ORIGINAL, 2, 20, date));
            block.extend_from_slice(&entry(TAG_MAKER_NOTE, 7, MAKER_NOTE.len() as u32, note));
            block.extend_from_slice(&short(TAG_COLOUR_SPACE, 1));
            block.extend_from_slice(&entry(TAG_PIXEL_X, 4, 1, 4000));
            block.extend_from_slice(&entry(TAG_PIXEL_Y, 4, 1, 3000));
            block.extend_from_slice(&entry(0xA431, 2, SERIAL.len() as u32, serial));
            block.extend_from_slice(&entry(TAG_LENS_MODEL, 2, 14, lens));
            block.extend_from_slice(&0_u32.to_le_bytes());
            assert_eq!(block.len() as u32, gps);
            // GPS IFD: version, latitude ref, latitude.
            block.extend_from_slice(&3_u16.to_le_bytes());
            block.extend_from_slice(&entry(0x0000, 1, 4, u32::from_le_bytes([2, 3, 0, 0])));
            block.extend_from_slice(&entry(0x0001, 2, 2, u32::from_le_bytes([b'N', 0, 0, 0])));
            block.extend_from_slice(&entry(0x0002, 5, 3, latitude));
            block.extend_from_slice(&0_u32.to_le_bytes());
            assert_eq!(block.len() as u32, ifd1);
            // IFD1: compression, thumbnail offset, length.
            block.extend_from_slice(&3_u16.to_le_bytes());
            block.extend_from_slice(&short(0x0103, 6));
            block.extend_from_slice(&entry(TAG_THUMBNAIL_OFFSET, 4, 1, thumb));
            block.extend_from_slice(&entry(TAG_THUMBNAIL_LENGTH, 4, 1, THUMBNAIL.len() as u32));
            block.extend_from_slice(&0_u32.to_le_bytes());
            assert_eq!(block.len() as u32, base);
            block.extend_from_slice(&values);
            block
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    #[test]
    fn a_block_is_summarised() {
        let block = Fixture::build();
        let summary = summarize(&block).unwrap();
        assert_eq!(summary.make.as_deref(), Some("Acme"));
        assert_eq!(summary.model.as_deref(), Some("Cam-One"));
        assert_eq!(summary.lens.as_deref(), Some("Lens 35mm f/2"));
        assert_eq!(summary.taken.as_deref(), Some("2026:10:10 14:03:09"));
        assert_eq!(summary.orientation, 6);
        assert!(summary.has_location);
        assert!(summary.has_thumbnail);
        assert!(summary.has_maker_note);
        assert!(summary.has_identifiers);
        assert_eq!(orientation(&block), Some(6));
    }

    #[test]
    fn the_thumbnail_is_always_removed_and_nothing_else_is() {
        let block = Fixture::build();
        let cleaned = clean(&block, &CleanSpec::default()).unwrap();
        assert!(cleaned.removed.thumbnail);
        assert!(!cleaned.removed.location);
        assert!(!contains(&cleaned.bytes, THUMBNAIL));
        assert!(!contains(&cleaned.bytes, b"THUMBNAIL"));
        // Everything else is where it was.
        assert!(contains(&cleaned.bytes, MAKER_NOTE));
        assert!(contains(&cleaned.bytes, SERIAL));
        let summary = summarize(&cleaned.bytes).unwrap();
        assert!(!summary.has_thumbnail);
        assert!(summary.has_location);
        assert_eq!(summary.model.as_deref(), Some("Cam-One"));
        assert_eq!(cleaned.bytes.len(), block.len());
    }

    #[test]
    fn private_removes_the_place_the_maker_note_and_the_serial_numbers_from_the_bytes() {
        let block = Fixture::build();
        let cleaned = clean(
            &block,
            &CleanSpec {
                private: true,
                ..CleanSpec::default()
            },
        )
        .unwrap();
        assert_eq!(
            cleaned.removed,
            Removed {
                location: true,
                thumbnail: true,
                maker_note: true,
                identifiers: true
            }
        );
        // Not unlinked: gone. The latitude's rationals, the note, the serial, the thumbnail.
        assert!(!contains(
            &cleaned.bytes,
            &[48, 0, 0, 0, 1, 0, 0, 0, 51, 0, 0, 0]
        ));
        assert!(!contains(&cleaned.bytes, b"SECRET-MAKER-NOTE"));
        assert!(!contains(&cleaned.bytes, b"SERIAL-0042"));
        assert!(!contains(&cleaned.bytes, b"THUMBNAIL"));
        let summary = summarize(&cleaned.bytes).unwrap();
        assert!(!summary.has_location && !summary.has_maker_note && !summary.has_identifiers);
        // What describes the picture stays.
        assert_eq!(summary.make.as_deref(), Some("Acme"));
        assert_eq!(summary.lens.as_deref(), Some("Lens 35mm f/2"));
        assert_eq!(summary.taken.as_deref(), Some("2026:10:10 14:03:09"));
        assert_eq!(summary.orientation, 6);
    }

    #[test]
    fn orientation_dimensions_and_colour_space_are_rewritten_in_place() {
        let block = Fixture::build();
        let cleaned = clean(
            &block,
            &CleanSpec {
                orientation: Some(1),
                dimensions: Some((3000, 4000)),
                colour_space_uncalibrated: true,
                ..CleanSpec::default()
            },
        )
        .unwrap();
        assert_eq!(orientation(&cleaned.bytes), Some(1));
        let layout = layout(&cleaned.bytes).unwrap();
        let exif = layout.exif.unwrap();
        let value = |tag| {
            let e = exif.find(tag).unwrap();
            get_u32(&cleaned.bytes, layout.order, e.at + 8).unwrap()
        };
        assert_eq!(value(TAG_PIXEL_X), 3000);
        assert_eq!(value(TAG_PIXEL_Y), 4000);
        assert_eq!(value(TAG_COLOUR_SPACE) & 0xFFFF, 0xFFFF);
        let width = layout.ifd0.find(TAG_IMAGE_WIDTH).unwrap();
        assert_eq!(
            get_u32(&cleaned.bytes, layout.order, width.at + 8),
            Some(3000)
        );
    }

    #[test]
    fn a_short_value_that_no_longer_fits_becomes_a_long() {
        let block = Fixture::build();
        let mut with_short = block.clone();
        // Turn PixelXDimension into a SHORT, then ask for 70,000.
        let layout0 = layout(&with_short).unwrap();
        let e = layout0
            .exif
            .as_ref()
            .unwrap()
            .find(TAG_PIXEL_X)
            .unwrap()
            .clone();
        put_u16(&mut with_short, Order::Little, e.at + 2, 3).unwrap();
        let cleaned = clean(
            &with_short,
            &CleanSpec {
                dimensions: Some((70_000, 10)),
                ..CleanSpec::default()
            },
        )
        .unwrap();
        let layout1 = layout(&cleaned.bytes).unwrap();
        let e = layout1.exif.as_ref().unwrap().find(TAG_PIXEL_X).unwrap();
        assert_eq!(e.kind, 4);
        assert_eq!(
            get_u32(&cleaned.bytes, Order::Little, e.at + 8),
            Some(70_000)
        );
    }

    #[test]
    fn a_big_endian_block_is_read_and_cleaned_too() {
        // II -> MM by building the smallest block by hand.
        let mut block = Vec::new();
        block.extend_from_slice(b"MM\0*");
        block.extend_from_slice(&8_u32.to_be_bytes());
        block.extend_from_slice(&1_u16.to_be_bytes());
        block.extend_from_slice(&TAG_ORIENTATION.to_be_bytes());
        block.extend_from_slice(&3_u16.to_be_bytes());
        block.extend_from_slice(&1_u32.to_be_bytes());
        block.extend_from_slice(&[0, 8, 0, 0]);
        block.extend_from_slice(&0_u32.to_be_bytes());
        assert_eq!(orientation(&block), Some(8));
        let cleaned = clean(
            &block,
            &CleanSpec {
                orientation: Some(1),
                ..CleanSpec::default()
            },
        )
        .unwrap();
        assert_eq!(orientation(&cleaned.bytes), Some(1));
    }

    #[test]
    fn an_orientation_only_block_is_a_block() {
        let block = orientation_only(6);
        assert_eq!(orientation(&block), Some(6));
        assert!(summarize(&block).is_some());
    }

    #[test]
    fn what_cannot_be_walked_is_refused_not_guessed() {
        assert!(summarize(b"").is_none());
        assert!(summarize(b"II*\0").is_none());
        assert!(summarize(b"not exif at all").is_none());
        // A GPS pointer that points past the end: a block that names a place it cannot show.
        let mut block = Fixture::build();
        let at = layout(&block).unwrap().ifd0.find(TAG_GPS_IFD).unwrap().at + 8;
        block[at..at + 4].copy_from_slice(&0xFFFF_FF00_u32.to_le_bytes());
        assert!(summarize(&block).is_none());
        assert!(clean(&block, &CleanSpec::default()).is_none());
    }

    #[test]
    fn a_directory_that_points_at_itself_does_not_loop() {
        let mut block = Fixture::build();
        let ifd0 = layout(&block).unwrap().ifd0;
        // IFD0's next-directory pointer names IFD0.
        block[ifd0.next_at..ifd0.next_at + 4].copy_from_slice(&8_u32.to_le_bytes());
        // It is walked once and left alone; the point is that it returns.
        let _ = clean(&block, &CleanSpec::default());
        let _ = summarize(&block);
    }

    #[test]
    fn no_truncation_and_no_changed_byte_makes_it_panic() {
        let block = Fixture::build();
        for len in 0..block.len() {
            let _ = summarize(&block[..len]);
            let _ = clean(
                &block[..len],
                &CleanSpec {
                    private: true,
                    orientation: Some(1),
                    dimensions: Some((1, 1)),
                    ..CleanSpec::default()
                },
            );
        }
        for at in 0..block.len() {
            for value in [0x00, 0x01, 0x7F, 0x80, 0xFF] {
                let mut changed = block.clone();
                changed[at] = value;
                if let Some(summary) = summarize(&changed) {
                    assert!((1..=8).contains(&summary.orientation));
                }
                if let Some(cleaned) = clean(
                    &changed,
                    &CleanSpec {
                        private: true,
                        ..CleanSpec::default()
                    },
                ) {
                    // Whatever came out is itself a block this reader will walk.
                    assert!(summarize(&cleaned.bytes).is_some());
                }
            }
        }
    }
}
