//! What a picture that is itself a TIFF says about itself: a DNG today, a TIFF picture when one is
//! opened.
//!
//! JPEG, PNG and WebP carry an EXIF block, a small TIFF of its own, which [`super::exif`] walks
//! and cleans in place. A TIFF is the structure EXIF was copied from, so there is no block to cut
//! out: the camera's tags sit in the first directory (IFD0) among the picture's own (strip
//! offsets, and in a DNG its colour matrices, linearisation tables and CFA description), the
//! exposure in the Exif directory IFD0 points at, the place in the GPS directory, and XMP and
//! IPTC are tags of IFD0.
//!
//! So the EXIF block is **built**: a new, small TIFF holding only the descriptive tags of IFD0
//! and the Exif, GPS and Interoperability directories, each value copied byte for byte under
//! its own type and count and laid down after the directories. Nothing is interpreted, which
//! is also why nothing is invented: a tag is carried or it is not.
//!
//! What is not carried, and said so (see [`super::left_behind`]):
//!
//! * **The maker note** (EXIF 0x927C) and a DNG's private data (0xC634, where Adobe's converter
//!   keeps the camera's own). Their values hold offsets into the file they came from, so moved
//!   into a block of their own they would point at nothing.
//! * A field above [`MAX_FIELD_BYTES`], or one that does not fit the block's budget, which is
//!   kept under what a JPEG segment holds so that the block can be written into one.
//!
//! And silently: the picture's own structure and the DNG-specific tags (they describe the sensor,
//! not the photograph, and mean nothing to the file the picture is exported to).
//!
//! The file is hostile input like any other. Every offset is checked against the buffer by the
//! reader this borrows from [`super::exif`], directories are visited once, and nothing is
//! allocated from a declared size beyond the caps above.

use super::exif::{self, get_u32, order_of, pointer, read_ifd, Ifd, Order, MAX_EXIF_BYTES};
use super::{iptc, left_behind, SourceMetadata};

const TAG_XMP: u16 = 0x02BC;
const TAG_IPTC_NAA: u16 = 0x83BB;
const TAG_PHOTOSHOP: u16 = 0x8649;
const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_GPS_IFD: u16 = 0x8825;
const TAG_INTEROP_IFD: u16 = 0xA005;
const TAG_MAKER_NOTE: u16 = 0x927C;
const TAG_DNG_PRIVATE_DATA: u16 = 0xC634;

/// The tags of IFD0 that say what the picture is and who made it: description, make, model,
/// orientation, resolution, software, date, artist and copyright. The rest of IFD0 is the
/// picture's own structure.
const DESCRIPTIVE: [u16; 11] = [
    0x010E, 0x010F, 0x0110, 0x0112, 0x011A, 0x011B, 0x0128, 0x0131, 0x0132, 0x013B, 0x8298,
];

/// A field larger than this is left behind. EXIF holds 64 KiB in a JPEG segment, all told.
pub const MAX_FIELD_BYTES: usize = 16 * 1024;
/// The block as a whole stays under one JPEG segment.
pub const MAX_BLOCK_BYTES: usize = 60_000;

/// TIFF field types of the base specification; anything else has no width this code could vouch
/// for and is not copied.
const KNOWN_KINDS: std::ops::RangeInclusive<u16> = 1..=12;

fn width_of(kind: u16) -> u64 {
    match kind {
        3 | 8 => 2,
        4 | 9 | 11 => 4,
        5 | 10 | 12 => 8,
        _ => 1,
    }
}

#[derive(Debug, Clone)]
struct Field {
    tag: u16,
    kind: u16,
    count: u32,
    value: Vec<u8>,
}

/// A directory to write: the fields copied, and the directories it points at, by index.
#[derive(Debug, Default)]
struct Directory {
    fields: Vec<Field>,
    /// (pointer tag, index of the directory it names)
    links: Vec<(u16, usize)>,
}

/// What may still go into the block, and whether anything was turned away for size.
struct Budget {
    left: usize,
    oversize: bool,
}

/// The fields of `ifd` that `wanted` names, as they are in the file.
fn copy_fields(
    file: &[u8],
    ifd: &Ifd,
    wanted: impl Fn(u16) -> bool,
    budget: &mut Budget,
) -> Vec<Field> {
    let mut out: Vec<Field> = Vec::new();
    for entry in &ifd.entries {
        if !wanted(entry.tag) || !KNOWN_KINDS.contains(&entry.kind) {
            continue;
        }
        // The first of a repeated tag stands, as it does for the reader of the directory.
        if out.iter().any(|field| field.tag == entry.tag) {
            continue;
        }
        let declared = width_of(entry.kind).saturating_mul(u64::from(entry.count));
        if declared == 0 {
            continue;
        }
        let cost = 12
            + if declared > 4 {
                declared + (declared & 1)
            } else {
                0
            };
        if declared > MAX_FIELD_BYTES as u64 || cost > budget.left as u64 {
            budget.oversize = true;
            continue;
        }
        // The value lies outside the file: there is nothing to copy.
        let Some((at, len)) = entry.data else {
            continue;
        };
        let Some(value) = at.checked_add(len).and_then(|end| file.get(at..end)) else {
            continue;
        };
        budget.left -= cost as usize;
        out.push(Field {
            tag: entry.tag,
            kind: entry.kind,
            count: entry.count,
            value: value.to_vec(),
        });
    }
    out
}

/// The follows-a-pointer step: `None` when IFD0 names no directory there, or one that cannot be
/// read. A damaged place costs the place, not the rest of the block.
fn follow(file: &[u8], order: Order, from: &Ifd, tag: u16) -> Option<Ifd> {
    let at = pointer(file, order, from, tag)?;
    read_ifd(file, order, at)
}

/// Builds the block, and says what was left out of it. `None` for the block when nothing
/// descriptive was found.
fn build_exif(file: &[u8], order: Order, ifd0: &Ifd) -> (Option<Vec<u8>>, u8) {
    let mut left = 0_u8;
    let mut budget = Budget {
        left: MAX_BLOCK_BYTES,
        oversize: false,
    };
    // The 8 bytes of header, and the three pointer fields a directory may gain, are paid for first.
    budget.left = budget.left.saturating_sub(8 + 3 * 12);

    let mut directories = vec![Directory {
        fields: copy_fields(file, ifd0, |tag| DESCRIPTIVE.contains(&tag), &mut budget),
        links: Vec::new(),
    }];

    // The Exif directory: everything but the maker note and the pointers, which are rebuilt.
    if let Some(exif_ifd) = follow(file, order, ifd0, TAG_EXIF_IFD) {
        if exif_ifd.find(TAG_MAKER_NOTE).is_some() {
            left |= left_behind::MAKER_NOTE;
        }
        let fields = copy_fields(
            file,
            &exif_ifd,
            |tag| {
                !matches!(
                    tag,
                    TAG_MAKER_NOTE | TAG_INTEROP_IFD | TAG_EXIF_IFD | TAG_GPS_IFD
                )
            },
            &mut budget,
        );
        let mut directory = Directory {
            fields,
            links: Vec::new(),
        };
        let mut interop = None;
        if let Some(interop_ifd) = follow(file, order, &exif_ifd, TAG_INTEROP_IFD) {
            let fields = copy_fields(file, &interop_ifd, |_| true, &mut budget);
            if !fields.is_empty() {
                interop = Some(Directory {
                    fields,
                    links: Vec::new(),
                });
            }
        }
        let exif_index = directories.len();
        if !directory.fields.is_empty() || interop.is_some() {
            directories.push(Directory::default());
            if let Some(interop) = interop {
                let interop_index = directories.len();
                directories.push(interop);
                directory.links.push((TAG_INTEROP_IFD, interop_index));
            }
            directories[exif_index] = directory;
            directories[0].links.push((TAG_EXIF_IFD, exif_index));
        }
    }

    if let Some(gps_ifd) = follow(file, order, ifd0, TAG_GPS_IFD) {
        let fields = copy_fields(file, &gps_ifd, |_| true, &mut budget);
        if !fields.is_empty() {
            let index = directories.len();
            directories.push(Directory {
                fields,
                links: Vec::new(),
            });
            directories[0].links.push((TAG_GPS_IFD, index));
        }
    }

    if ifd0.find(TAG_DNG_PRIVATE_DATA).is_some() {
        left |= left_behind::MAKER_NOTE;
    }
    if budget.oversize {
        left |= left_behind::OVERSIZE;
    }

    let nothing = directories
        .iter()
        .all(|directory| directory.fields.is_empty() && directory.links.is_empty());
    if nothing {
        return (None, left);
    }
    let block = serialise(order, &directories).filter(|block| {
        // What was made must itself be a block the cleaner will walk, or it is not offered.
        block.len() <= MAX_EXIF_BYTES && exif::summarize(block).is_some()
    });
    (block, left)
}

fn put_u16(out: &mut Vec<u8>, order: Order, value: u16) {
    out.extend_from_slice(&match order {
        Order::Little => value.to_le_bytes(),
        Order::Big => value.to_be_bytes(),
    });
}

fn put_u32(out: &mut Vec<u8>, order: Order, value: u32) {
    out.extend_from_slice(&match order {
        Order::Little => value.to_le_bytes(),
        Order::Big => value.to_be_bytes(),
    });
}

/// Writes the directories one after another, then every value that did not fit inline, with the
/// pointers resolved. Entries are in tag order, which TIFF requires.
fn serialise(order: Order, directories: &[Directory]) -> Option<Vec<u8>> {
    enum Slot<'a> {
        Field(&'a Field),
        Link(usize),
    }
    let layout: Vec<Vec<(u16, Slot<'_>)>> = directories
        .iter()
        .map(|directory| {
            let mut entries: Vec<(u16, Slot<'_>)> = directory
                .fields
                .iter()
                .map(|field| (field.tag, Slot::Field(field)))
                .collect();
            entries.extend(
                directory
                    .links
                    .iter()
                    .map(|(tag, target)| (*tag, Slot::Link(*target))),
            );
            entries.sort_by_key(|(tag, _)| *tag);
            entries
        })
        .collect();

    let mut offsets = Vec::with_capacity(layout.len());
    let mut at = 8_usize;
    for entries in &layout {
        offsets.push(u32::try_from(at).ok()?);
        at = at.checked_add(2 + 12 * entries.len() + 4)?;
    }
    let values_start = at;

    let mut out: Vec<u8> = Vec::with_capacity(values_start + 256);
    out.extend_from_slice(match order {
        Order::Little => b"II*\0",
        Order::Big => b"MM\0*",
    });
    put_u32(&mut out, order, 8);
    let mut values: Vec<u8> = Vec::new();
    for entries in &layout {
        put_u16(&mut out, order, u16::try_from(entries.len()).ok()?);
        for (tag, slot) in entries {
            put_u16(&mut out, order, *tag);
            match slot {
                Slot::Link(target) => {
                    put_u16(&mut out, order, 4);
                    put_u32(&mut out, order, 1);
                    put_u32(&mut out, order, *offsets.get(*target)?);
                }
                Slot::Field(field) => {
                    put_u16(&mut out, order, field.kind);
                    put_u32(&mut out, order, field.count);
                    if field.value.len() <= 4 {
                        out.extend_from_slice(&field.value);
                        out.resize(out.len() + (4 - field.value.len()), 0);
                    } else {
                        let place = values_start.checked_add(values.len())?;
                        put_u32(&mut out, order, u32::try_from(place).ok()?);
                        values.extend_from_slice(&field.value);
                        if values.len() % 2 == 1 {
                            values.push(0);
                        }
                    }
                }
            }
        }
        // No next directory: a thumbnail is never carried.
        put_u32(&mut out, order, 0);
    }
    debug_assert_eq!(out.len(), values_start);
    out.extend_from_slice(&values);
    Some(out)
}

/// The value of an IFD0 tag of one of these types, as bytes.
fn value_of(file: &[u8], ifd: &Ifd, tag: u16, kinds: &[u16]) -> Option<Vec<u8>> {
    let entry = ifd.find(tag)?;
    if !kinds.contains(&entry.kind) {
        return None;
    }
    let (at, len) = entry.data?;
    file.get(at..at.checked_add(len)?).map(<[u8]>::to_vec)
}

/// The metadata of the TIFF-structured picture in `file` (the whole file, as it is on disk).
///
/// `None` when the bytes are not a TIFF this reader will walk; a file with no metadata is `Some`
/// with nothing in it. The orientation is read, not applied: whether the pixels were turned is
/// for the caller to say.
pub fn read(file: &[u8]) -> Option<SourceMetadata> {
    let order = order_of(file)?;
    let first = usize::try_from(get_u32(file, order, 4)?).ok()?;
    let ifd0 = read_ifd(file, order, first)?;

    // A packet written as ASCII carries its terminator, and some writers pad with zeros.
    let xmp = value_of(file, &ifd0, TAG_XMP, &[1, 2, 7]).map(|mut bytes| {
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        bytes
    });
    // The newsroom tag first, then the Photoshop resources that may hold the same stream.
    let iptc = [
        (TAG_IPTC_NAA, [1_u16, 2, 4, 7].as_slice()),
        (TAG_PHOTOSHOP, &[1, 7][..]),
    ]
    .into_iter()
    .find_map(|(tag, kinds)| {
        value_of(file, &ifd0, tag, kinds).and_then(|bytes| iptc::extract(&bytes))
    });

    let (block, left) = build_exif(file, order, &ifd0);
    let mut found = SourceMetadata::from_blocks(block, xmp, iptc);
    found.left_behind = left;
    Some(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{prepare, MetadataPolicy, OutputFacts};

    /// One field of a synthetic directory.
    type Item = (u16, u16, Vec<u8>);

    fn le16(value: u16) -> Vec<u8> {
        value.to_le_bytes().to_vec()
    }

    fn ascii(text: &str) -> Vec<u8> {
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(0);
        bytes
    }

    fn width(kind: u16) -> usize {
        match kind {
            3 | 8 => 2,
            4 | 9 | 11 | 13 => 4,
            5 | 10 | 12 => 8,
            _ => 1,
        }
    }

    /// A TIFF: IFD0, then each further directory (Exif, GPS, Interoperability), then every
    /// value. An entry of a pointer tag (`0x8769` and `0x8825` in IFD0, `0xA005` in the Exif
    /// directory) with a payload of one byte is replaced by the offset of its directory.
    struct Tiff {
        ifd0: Vec<Item>,
        exif: Vec<Item>,
        gps: Vec<Item>,
        interop: Vec<Item>,
        big: bool,
    }

    impl Tiff {
        fn new() -> Self {
            Self {
                ifd0: Vec::new(),
                exif: Vec::new(),
                gps: Vec::new(),
                interop: Vec::new(),
                big: false,
            }
        }

        fn build(&self) -> Vec<u8> {
            let order = if self.big { Order::Big } else { Order::Little };
            let mut ifd0 = self.ifd0.clone();
            if !self.exif.is_empty() {
                ifd0.push((TAG_EXIF_IFD, 4, vec![0]));
            }
            if !self.gps.is_empty() {
                ifd0.push((TAG_GPS_IFD, 4, vec![1]));
            }
            let mut exif = self.exif.clone();
            if !self.interop.is_empty() {
                exif.push((TAG_INTEROP_IFD, 4, vec![0]));
            }
            let dirs = [&ifd0, &exif, &self.gps, &self.interop];
            // Directory sizes first, so the pointers and the values can be placed.
            let mut dir_at = Vec::new();
            let mut at = 8_usize;
            for dir in &dirs {
                dir_at.push(at);
                if !dir.is_empty() {
                    at += 2 + 12 * dir.len() + 4;
                }
            }
            let mut values = Vec::new();
            let values_start = at;
            let mut out = Vec::new();
            out.extend_from_slice(if self.big { b"MM\0*" } else { b"II*\0" });
            put_u32(&mut out, order, 8);
            for (index, dir) in dirs.iter().enumerate() {
                if dir.is_empty() {
                    continue;
                }
                let mut sorted: Vec<&Item> = dir.iter().collect();
                sorted.sort_by_key(|(tag, _, _)| *tag);
                put_u16(&mut out, order, sorted.len() as u16);
                for (tag, kind, payload) in sorted {
                    put_u16(&mut out, order, *tag);
                    put_u16(&mut out, order, *kind);
                    let target = match (index, *tag) {
                        (0, TAG_EXIF_IFD) if payload.len() == 1 => Some(1),
                        (0, TAG_GPS_IFD) if payload.len() == 1 => Some(2),
                        (1, TAG_INTEROP_IFD) if payload.len() == 1 => Some(3),
                        _ => None,
                    };
                    if let Some(target) = target {
                        put_u32(&mut out, order, 1);
                        put_u32(&mut out, order, dir_at[target] as u32);
                        continue;
                    }
                    put_u32(&mut out, order, (payload.len() / width(*kind)) as u32);
                    if payload.len() <= 4 {
                        out.extend_from_slice(payload);
                        out.resize(out.len() + 4 - payload.len(), 0);
                    } else {
                        put_u32(&mut out, order, (values_start + values.len()) as u32);
                        values.extend_from_slice(payload);
                        if values.len() % 2 == 1 {
                            values.push(0);
                        }
                    }
                }
                put_u32(&mut out, order, 0);
            }
            assert_eq!(out.len(), values_start);
            out.extend_from_slice(&values);
            out
        }
    }

    /// A DNG-shaped file: picture structure and DNG tags beside the descriptive ones, an Exif
    /// directory with a maker note, a GPS directory, XMP and IPTC.
    fn dng_like() -> Tiff {
        let mut tiff = Tiff::new();
        tiff.ifd0 = vec![
            (0x00FE, 4, vec![1, 0, 0, 0]),
            (0x0100, 4, 256_u32.to_le_bytes().to_vec()),
            (0x0101, 4, 171_u32.to_le_bytes().to_vec()),
            (0x010F, 2, ascii("Acme")),
            (0x0110, 2, ascii("Cam-One")),
            (0x0111, 4, 9000_u32.to_le_bytes().to_vec()),
            (0x0112, 3, le16(6)),
            (0x0131, 2, ascii("Converter 9.8")),
            (0x0132, 2, ascii("2026:10:11 09:00:00")),
            (0x013B, 2, ascii("A. Photographer")),
            (
                0x02BC,
                1,
                b"<x:xmpmeta><dc:title>Pier</dc:title></x:xmpmeta>".to_vec(),
            ),
            (0x83BB, 7, vec![0x1C, 2, 80, 0, 4, b'P', b'i', b'e', b'r']),
            (0xC612, 1, vec![1, 4, 0, 0]),
            (0xC614, 2, ascii("Acme Cam-One")),
            (0xC62F, 2, ascii("SN-IFD0-77")),
            (0xC634, 1, b"Adobe\0MakN-PRIVATE-DATA-OF-THE-MAKER".to_vec()),
            (0xC621, 10, vec![7; 72]),
        ];
        tiff.exif = vec![
            (
                0x829A,
                5,
                [1_u32.to_le_bytes(), 250_u32.to_le_bytes()].concat(),
            ),
            (0x8827, 3, le16(400)),
            (0x9003, 2, ascii("2026:10:10 14:03:09")),
            (0x927C, 7, b"MAKER-NOTE-WITH-ABSOLUTE-OFFSETS".to_vec()),
            (0xA431, 2, ascii("BODY-SERIAL-42")),
            (0xA434, 2, ascii("Lens 35mm f/2")),
        ];
        tiff.interop = vec![(0x0001, 2, ascii("R98")), (0x0002, 7, b"0100".to_vec())];
        tiff.gps = vec![
            (0x0000, 1, vec![2, 3, 0, 0]),
            (0x0001, 2, ascii("N")),
            (
                0x0002,
                5,
                [
                    48_u32.to_le_bytes(),
                    1_u32.to_le_bytes(),
                    51_u32.to_le_bytes(),
                    1_u32.to_le_bytes(),
                    7_u32.to_le_bytes(),
                    1_u32.to_le_bytes(),
                ]
                .concat(),
            ),
        ];
        tiff
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }

    /// The tags of the first directory of a built block, in the order they are written.
    fn ifd0_tags(block: &[u8]) -> Vec<u16> {
        let order = order_of(block).unwrap();
        let first = get_u32(block, order, 4).unwrap() as usize;
        let ifd0 = read_ifd(block, order, first).unwrap();
        ifd0.entries.iter().map(|entry| entry.tag).collect()
    }

    fn facts() -> OutputFacts {
        OutputFacts {
            width: 100,
            height: 50,
            wide_gamut_profile: false,
        }
    }

    #[test]
    fn a_dng_like_file_gives_exif_xmp_and_iptc() {
        let file = dng_like().build();
        let found = read(&file).unwrap();
        let block = found.exif.as_ref().expect("an EXIF block was built");
        let summary = exif::summarize(block).unwrap();
        assert_eq!(summary.make.as_deref(), Some("Acme"));
        assert_eq!(summary.model.as_deref(), Some("Cam-One"));
        assert_eq!(summary.lens.as_deref(), Some("Lens 35mm f/2"));
        assert_eq!(summary.taken.as_deref(), Some("2026:10:10 14:03:09"));
        assert!(summary.has_location);
        assert!(summary.has_identifiers, "the Exif body serial number");
        assert!(!summary.has_thumbnail);
        assert_eq!(found.source_orientation, 6);
        assert!(!found.orientation_applied);
        assert!(found.xmp.as_ref().unwrap().starts_with(b"<x:xmpmeta>"));
        assert_eq!(
            found.iptc.as_ref().unwrap().as_slice(),
            [0x1C, 2, 80, 0, 4, b'P', b'i', b'e', b'r']
        );
    }

    #[test]
    fn only_descriptive_tags_and_the_exif_and_gps_directories_are_carried() {
        let file = dng_like().build();
        let found = read(&file).unwrap();
        let block = found.exif.unwrap();
        // The picture's own structure and the DNG's sensor tags are not in it.
        for foreign in [
            &b"SN-IFD0-77"[..],
            b"Acme Cam-One",
            b"PRIVATE-DATA",
            b"MAKER-NOTE",
        ] {
            assert!(
                !contains(&block, foreign),
                "{:?}",
                String::from_utf8_lossy(foreign)
            );
        }
        // The tags that are in it, by reading the block back as a directory.
        let order = order_of(&block).unwrap();
        let ifd0 = read_ifd(&block, order, 8).unwrap();
        let tags: Vec<u16> = ifd0.entries.iter().map(|entry| entry.tag).collect();
        assert_eq!(
            tags,
            vec![0x010F, 0x0110, 0x0112, 0x0131, 0x0132, 0x013B, 0x8769, 0x8825]
        );
        assert!(
            tags.windows(2).all(|pair| pair[0] < pair[1]),
            "TIFF requires entries in tag order"
        );
        let exif_ifd = read_ifd(
            &block,
            order,
            pointer(&block, order, &ifd0, 0x8769).unwrap(),
        )
        .unwrap();
        let exif_tags: Vec<u16> = exif_ifd.entries.iter().map(|entry| entry.tag).collect();
        // The Interoperability pointer is rebuilt, among the fields, in tag order.
        assert_eq!(
            exif_tags,
            vec![0x829A, 0x8827, 0x9003, 0xA005, 0xA431, 0xA434]
        );
        assert!(
            exif_tags.windows(2).all(|pair| pair[0] < pair[1]),
            "TIFF requires entries in tag order"
        );
        let interop_at = pointer(&block, order, &exif_ifd, 0xA005).unwrap();
        let interop = read_ifd(&block, order, interop_at).unwrap();
        let (at, len) = interop.find(0x0001).unwrap().data.unwrap();
        assert_eq!(&block[at..at + len], b"R98\0");
    }

    #[test]
    fn the_maker_note_and_the_private_data_are_left_behind_and_said() {
        let file = dng_like().build();
        let found = read(&file).unwrap();
        assert_eq!(
            found.left_behind & left_behind::MAKER_NOTE,
            left_behind::MAKER_NOTE
        );
        assert!(found.summary().has_maker_note);

        let (kept, report) = prepare(&found, MetadataPolicy::KeepAll, facts());
        assert!(kept.exif.is_some());
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.kind == "removed" && note.detail.contains("private notes")),
            "{report:?}"
        );
        // And the place is still in the file under keep-all, with its note.
        assert!(report
            .notes
            .iter()
            .any(|note| note.detail.contains("place")));
    }

    #[test]
    fn the_private_policy_removes_the_place_and_the_serial_numbers_from_the_built_block() {
        let file = dng_like().build();
        let found = read(&file).unwrap();
        let (prepared, report) = prepare(&found, MetadataPolicy::RemovePrivate, facts());
        let block = prepared.exif.unwrap();
        let summary = exif::summarize(&block).unwrap();
        assert!(!summary.has_location);
        assert!(!summary.has_identifiers);
        assert!(!contains(&block, b"BODY-SERIAL-42"));
        // The camera and the lens stay.
        assert_eq!(summary.model.as_deref(), Some("Cam-One"));
        assert_eq!(summary.lens.as_deref(), Some("Lens 35mm f/2"));
        assert!(report.notes.iter().any(|note| note.detail.contains("GPS")));
        // The orientation was not spent on the pixels, so it travels with them.
        assert_eq!(exif::orientation(&block), Some(6));
    }

    #[test]
    fn remove_all_still_carries_an_orientation_that_was_not_applied() {
        let file = dng_like().build();
        let found = read(&file).unwrap();
        let (prepared, _) = prepare(&found, MetadataPolicy::RemoveAll, facts());
        let block = prepared.exif.unwrap();
        assert_eq!(exif::orientation(&block), Some(6));
        assert_eq!(exif::summarize(&block).unwrap().model, None);
        assert!(prepared.xmp.is_none() && prepared.iptc.is_none());
    }

    #[test]
    fn a_big_endian_file_gives_a_big_endian_block_with_the_same_values() {
        let mut tiff = dng_like();
        // The synthetic builder writes values little-endian; a big-endian file needs its own.
        tiff.big = true;
        tiff.ifd0 = vec![
            (0x010F, 2, ascii("Acme")),
            (0x0110, 2, ascii("Cam-One")),
            (0x0112, 3, 6_u16.to_be_bytes().to_vec()),
        ];
        tiff.exif = vec![
            (0x8827, 3, 400_u16.to_be_bytes().to_vec()),
            (0x9003, 2, ascii("2026:10:10 14:03:09")),
        ];
        tiff.gps = Vec::new();
        let file = tiff.build();
        assert_eq!(&file[..4], b"MM\0*");
        let found = read(&file).unwrap();
        let block = found.exif.unwrap();
        assert_eq!(&block[..4], b"MM\0*");
        let summary = exif::summarize(&block).unwrap();
        assert_eq!(summary.model.as_deref(), Some("Cam-One"));
        assert_eq!(summary.taken.as_deref(), Some("2026:10:10 14:03:09"));
        assert_eq!(found.source_orientation, 6);
    }

    #[test]
    fn a_photoshop_resource_block_is_read_for_iptc_when_the_newsroom_tag_is_absent() {
        let stream = vec![0x1C, 2, 120, 0, 3, b'a', b'b', b'c'];
        let mut resource = Vec::new();
        resource.extend_from_slice(b"8BIM");
        resource.extend_from_slice(&0x0404_u16.to_be_bytes());
        resource.extend_from_slice(&[0, 0]);
        resource.extend_from_slice(&(stream.len() as u32).to_be_bytes());
        resource.extend_from_slice(&stream);
        resource.push(0);
        let mut tiff = Tiff::new();
        tiff.ifd0 = vec![(0x010F, 2, ascii("Acme")), (0x8649, 1, resource)];
        let found = read(&tiff.build()).unwrap();
        assert_eq!(found.iptc.unwrap().as_slice(), stream.as_slice());
    }

    #[test]
    fn an_xmp_packet_loses_its_terminator() {
        let mut tiff = Tiff::new();
        let mut packet = b"<x:xmpmeta/>".to_vec();
        packet.extend_from_slice(&[0, 0]);
        tiff.ifd0 = vec![(0x02BC, 2, packet)];
        let found = read(&tiff.build()).unwrap();
        assert_eq!(found.xmp.unwrap().as_slice(), b"<x:xmpmeta/>");
    }

    #[test]
    fn a_file_with_no_metadata_is_some_with_nothing_in_it() {
        let mut tiff = Tiff::new();
        tiff.ifd0 = vec![(0x0100, 4, 8_u32.to_le_bytes().to_vec())];
        let found = read(&tiff.build()).unwrap();
        assert!(found.is_empty());
        assert_eq!(found.source_orientation, 1);
        assert_eq!(found.left_behind, 0);
    }

    #[test]
    fn what_is_not_a_tiff_is_none() {
        assert!(read(b"").is_none());
        assert!(read(b"\xFF\xD8\xFF\xE0 a jpeg").is_none());
        assert!(read(b"II*\0").is_none());
        // BigTIFF has its own header and is not read here.
        assert!(read(b"II+\0\x08\0\0\0\x10\0\0\0\0\0\0\0").is_none());
    }

    #[test]
    fn an_oversize_field_is_left_behind_and_the_rest_is_kept() {
        let mut tiff = dng_like();
        tiff.exif.push((0x9286, 7, vec![b'x'; MAX_FIELD_BYTES + 2]));
        let found = read(&tiff.build()).unwrap();
        assert_eq!(
            found.left_behind & left_behind::OVERSIZE,
            left_behind::OVERSIZE
        );
        let block = found.exif.as_ref().unwrap();
        assert!(block.len() < MAX_FIELD_BYTES);
        assert_eq!(
            exif::summarize(block).unwrap().lens.as_deref(),
            Some("Lens 35mm f/2")
        );
        let (_, report) = prepare(&found, MetadataPolicy::KeepAll, facts());
        assert!(report
            .notes
            .iter()
            .any(|note| note.detail.contains("too large")));
    }

    #[test]
    fn many_fields_stop_at_the_budget_and_the_block_stays_small() {
        let mut tiff = Tiff::new();
        tiff.ifd0 = vec![(0x010F, 2, ascii("Acme"))];
        // 300 fields of 12 KiB each: far over a JPEG segment's worth.
        tiff.exif = (0..300_u16)
            .map(|index| (0x9000 + index, 7, vec![index as u8; 12 * 1024]))
            .collect();
        let found = read(&tiff.build()).unwrap();
        assert_eq!(
            found.left_behind & left_behind::OVERSIZE,
            left_behind::OVERSIZE
        );
        let block = found.exif.unwrap();
        assert!(block.len() <= MAX_BLOCK_BYTES, "{}", block.len());
        assert!(exif::summarize(&block).is_some());
    }

    #[test]
    fn a_pointer_outside_the_file_costs_only_that_directory() {
        let mut file = dng_like().build();
        // Find the GPS pointer in IFD0 and send it far past the end.
        let count = u16::from_le_bytes([file[8], file[9]]) as usize;
        let mut patched = false;
        for index in 0..count {
            let entry = 10 + index * 12;
            if u16::from_le_bytes([file[entry], file[entry + 1]]) == TAG_GPS_IFD {
                file[entry + 8..entry + 12].copy_from_slice(&0xFFFF_0000_u32.to_le_bytes());
                patched = true;
            }
        }
        assert!(patched);
        let found = read(&file).unwrap();
        let summary = exif::summarize(found.exif.as_ref().unwrap()).unwrap();
        assert!(!summary.has_location);
        assert_eq!(summary.model.as_deref(), Some("Cam-One"));
        assert_eq!(summary.lens.as_deref(), Some("Lens 35mm f/2"));
    }

    #[test]
    fn directories_that_point_at_each_other_do_not_loop() {
        // The Exif directory's own Exif pointer names IFD0; the pointer is not followed again.
        let mut tiff = dng_like();
        tiff.exif
            .push((TAG_EXIF_IFD, 4, 8_u32.to_le_bytes().to_vec()));
        let found = read(&tiff.build()).unwrap();
        let block = found.exif.unwrap();
        assert!(exif::summarize(&block).is_some());
        assert!(block.len() <= MAX_BLOCK_BYTES);
    }

    #[test]
    fn an_unknown_field_type_and_a_zero_count_are_not_copied() {
        let mut tiff = Tiff::new();
        tiff.ifd0 = vec![
            (0x010F, 2, ascii("Acme")),
            (0x0110, 99, vec![1, 2, 3, 4, 5, 6, 7, 8]),
        ];
        let found = read(&tiff.build()).unwrap();
        let block = found.exif.as_ref().unwrap();
        let summary = exif::summarize(block).unwrap();
        assert_eq!(summary.make.as_deref(), Some("Acme"));
        // Not "no model was read": the entry is not in the block at all.
        assert_eq!(ifd0_tags(block), vec![0x010F]);
    }

    #[test]
    fn either_place_a_maker_note_can_be_is_noticed_on_its_own() {
        let mut in_exif = Tiff::new();
        in_exif.ifd0 = vec![(0x010F, 2, ascii("Acme"))];
        in_exif.exif = vec![(0x927C, 7, b"NOTE-IN-THE-EXIF-DIRECTORY".to_vec())];
        let found = read(&in_exif.build()).unwrap();
        assert_eq!(found.left_behind, left_behind::MAKER_NOTE);

        let mut in_private_data = Tiff::new();
        in_private_data.ifd0 = vec![
            (0x010F, 2, ascii("Acme")),
            (0xC634, 1, b"Adobe\0MakN-ONLY-HERE".to_vec()),
        ];
        let found = read(&in_private_data.build()).unwrap();
        assert_eq!(found.left_behind, left_behind::MAKER_NOTE);

        let mut neither = Tiff::new();
        neither.ifd0 = vec![(0x010F, 2, ascii("Acme"))];
        assert_eq!(read(&neither.build()).unwrap().left_behind, 0);
    }

    #[test]
    fn values_sit_on_even_offsets_as_tiff_asks() {
        let file = dng_like().build();
        let block = read(&file).unwrap().exif.unwrap();
        let order = order_of(&block).unwrap();
        let ifd0 = read_ifd(&block, order, 8).unwrap();
        let exif_ifd = read_ifd(
            &block,
            order,
            pointer(&block, order, &ifd0, 0x8769).unwrap(),
        )
        .unwrap();
        let gps_ifd = read_ifd(
            &block,
            order,
            pointer(&block, order, &ifd0, 0x8825).unwrap(),
        )
        .unwrap();
        let mut stored = 0;
        for ifd in [&ifd0, &exif_ifd, &gps_ifd] {
            for entry in &ifd.entries {
                if let Some((at, len)) = entry.data {
                    if len > 4 {
                        stored += 1;
                        assert_eq!(at % 2, 0, "tag {:#06x} at {at}", entry.tag);
                    }
                }
            }
        }
        assert!(stored >= 6, "the check looked at {stored} values");
    }

    #[test]
    fn a_repeated_tag_keeps_the_first() {
        let mut tiff = Tiff::new();
        tiff.ifd0 = vec![
            (0x0110, 2, ascii("First")),
            (0x0110, 2, ascii("Second")),
            (0x010F, 2, ascii("Acme")),
        ];
        let found = read(&tiff.build()).unwrap();
        let block = found.exif.as_ref().unwrap();
        let summary = exif::summarize(block).unwrap();
        assert_eq!(summary.model.as_deref(), Some("First"));
        // One entry, not two with the second hidden behind the first.
        assert_eq!(ifd0_tags(block), vec![0x010F, 0x0110]);
    }

    /// Every prefix of a well-formed file, and every single-byte change of it, is read without a
    /// panic, and whatever block comes out is one the cleaner will walk and within its budget.
    #[test]
    fn truncated_and_corrupted_files_never_panic_and_never_give_an_unwalkable_block() {
        let file = dng_like().build();
        let check = |bytes: &[u8]| {
            if let Some(found) = read(bytes) {
                if let Some(block) = &found.exif {
                    assert!(block.len() <= MAX_BLOCK_BYTES);
                    assert!(exif::summarize(block).is_some());
                }
            }
        };
        for length in 0..=file.len() {
            check(&file[..length]);
        }
        for at in 0..file.len() {
            for value in [0x00, 0x01, 0x7F, 0x80, 0xFF] {
                let mut changed = file.clone();
                changed[at] = value;
                check(&changed);
            }
        }
    }
}
