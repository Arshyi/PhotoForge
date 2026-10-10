//! The structure of a PSD or PSB: header, resources, layer records. No pixel is read here.
//!
//! Written against Adobe's published file-format specification. Every length is checked
//! against the section that contains it before a buffer is made for it (see `reader`), and
//! the number of things a file may declare — layer records, resource blocks, tagged blocks
//! per layer — is capped, so a hostile file costs at most a bounded amount of work.
use super::error::PsdError;
use super::reader::Reader;
use std::io::{Read, Seek};

pub const MAX_DIMENSION_PSD: u32 = 30_000;
pub const MAX_DIMENSION_PSB: u32 = 300_000;
pub const MAX_CHANNELS: u16 = 56;
/// Records in the layer section. A group takes two (its opening and closing divider), so
/// this is not the layer limit — `layers::MAX_LAYERS` is, and is applied by the tree builder.
pub const MAX_LAYER_RECORDS: u64 = 4_096;
pub const MAX_ICC_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_RESOURCE_BLOCKS: usize = 4_096;
pub const MAX_TAGGED_BLOCKS: usize = 512;
const MAX_NAME_UNITS: u64 = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    Psd,
    Psb,
}

impl Version {
    /// Whether lengths that are 4 bytes in a PSD are 8 in this file.
    pub fn is_big(self) -> bool {
        self == Self::Psb
    }

    pub fn max_dimension(self) -> u32 {
        match self {
            Self::Psd => MAX_DIMENSION_PSD,
            Self::Psb => MAX_DIMENSION_PSB,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Bitmap,
    Grayscale,
    Indexed,
    Rgb,
    Cmyk,
    Multichannel,
    Duotone,
    Lab,
}

impl ColorMode {
    fn from_u16(value: u16) -> Option<Self> {
        Some(match value {
            0 => Self::Bitmap,
            1 => Self::Grayscale,
            2 => Self::Indexed,
            3 => Self::Rgb,
            4 => Self::Cmyk,
            7 => Self::Multichannel,
            8 => Self::Duotone,
            9 => Self::Lab,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub version: Version,
    pub channels: u16,
    pub width: u32,
    pub height: u32,
    pub depth: u16,
    pub mode: ColorMode,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Resolution {
    pub horizontal_ppi: f32,
    pub vertical_ppi: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceBlock {
    pub id: u16,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Resources {
    pub icc_profile: Option<Vec<u8>>,
    /// The photograph's EXIF block (resource 1058), XMP packet (1060) and IPTC stream (1028),
    /// when each is within the size this code carries.
    pub exif: Option<Vec<u8>>,
    pub xmp: Option<Vec<u8>>,
    pub iptc: Option<Vec<u8>>,
    pub resolution: Option<Resolution>,
    /// Every block's id and size, so a report can say what was present and not carried over.
    pub blocks: Vec<ResourceBlock>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub top: i32,
    pub left: i32,
    pub bottom: i32,
    pub right: i32,
}

impl Rect {
    pub fn width(&self) -> u64 {
        (i64::from(self.right) - i64::from(self.left)).max(0) as u64
    }

    pub fn height(&self) -> u64 {
        (i64::from(self.bottom) - i64::from(self.top)).max(0) as u64
    }

    pub fn is_empty(&self) -> bool {
        self.width() == 0 || self.height() == 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInfo {
    /// 0, 1, 2 colour; -1 transparency; -2 user mask; -3 real user mask.
    pub id: i16,
    /// Bytes of channel image data, including its two-byte compression field.
    pub length: u64,
    /// Absolute offset of the compression field in the file.
    pub offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskData {
    pub rect: Rect,
    pub default_color: u8,
    pub flags: u8,
    /// The second mask of a layer that has both a raster and a vector mask (channel id -3 is
    /// the "real" user mask and has its own rectangle).
    pub real: Option<RealMask>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealMask {
    pub rect: Rect,
    pub default_color: u8,
    pub flags: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divider {
    /// A group's own record, drawn open in Photoshop's panel.
    OpenFolder,
    ClosedFolder,
    /// The record that marks where a group's members end (it comes first in the file).
    BoundingSection,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerRecord {
    pub rect: Rect,
    pub channels: Vec<ChannelInfo>,
    pub blend_key: [u8; 4],
    pub opacity: u8,
    /// True for a layer clipped to the one beneath it.
    pub clipped: bool,
    pub flags: u8,
    pub name: String,
    pub unicode_name: Option<String>,
    pub mask: Option<MaskData>,
    pub divider: Option<Divider>,
    /// The blend key a group carries in its section-divider block (its own record says
    /// `pass` or `norm` for compatibility).
    pub divider_blend_key: Option<[u8; 4]>,
    pub layer_id: Option<u32>,
    /// The "fill opacity" (`iOpa`), which multiplies the layer's opacity for a layer with
    /// no effects.
    pub fill_opacity: Option<u8>,
    /// "Blend clipped layers as group" (`clbl`), which Photoshop switches on unless the file
    /// says otherwise. Off means the layers clipped to this one blend with what is beneath
    /// the whole clipping group individually, not as one.
    pub blend_clipped_as_group: bool,
    /// The settings block of an adjustment layer this reader knows how to hand on (`curv`,
    /// `levl`, `nvrt`), with its key. Larger blocks are not kept.
    pub adjustment: Option<([u8; 4], Vec<u8>)>,
    /// Every additional-information key the layer carries, in file order.
    pub block_keys: Vec<[u8; 4]>,
}

impl LayerRecord {
    /// Bit 1 of the flags is set when the layer is *hidden* (the specification's wording is
    /// the other way round; Photoshop's files are as here).
    pub fn visible(&self) -> bool {
        self.flags & 0b10 == 0
    }

    pub fn display_name(&self) -> &str {
        self.unicode_name.as_deref().unwrap_or(&self.name)
    }

    pub fn has_key(&self, key: &[u8; 4]) -> bool {
        self.block_keys.iter().any(|candidate| candidate == key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LayerSection {
    pub records: Vec<LayerRecord>,
    /// The layer count was negative: the first alpha channel holds the merged image's
    /// transparency.
    pub merged_alpha: bool,
    /// Where the records came from when not the plain layer-info block (`Lr16`, `Lr32`, `Layr`).
    pub from_block: Option<[u8; 4]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    /// Absolute offset of the compression field of the merged image.
    pub offset: u64,
    pub compression: u16,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub header: Header,
    pub resources: Resources,
    pub layers: LayerSection,
    /// Keys of the document-level additional information blocks.
    pub global_block_keys: Vec<[u8; 4]>,
    pub image_data: Option<ImageData>,
    pub file_len: u64,
}

const SIG_8BIM: [u8; 4] = *b"8BIM";
const SIG_8B64: [u8; 4] = *b"8B64";

/// Keys whose length is 8 bytes in a PSB.
fn is_long_key(key: &[u8; 4]) -> bool {
    matches!(
        key,
        b"LMsk"
            | b"Lr16"
            | b"Lr32"
            | b"Layr"
            | b"Mt16"
            | b"Mt32"
            | b"Mtrn"
            | b"Alph"
            | b"FMsk"
            | b"lnk2"
            | b"FEid"
            | b"FXid"
            | b"PxSD"
            | b"cinf"
    )
}

pub fn read_document<R: Read + Seek>(source: R) -> Result<Document, PsdError> {
    let mut reader = Reader::new(source)?;
    let file_len = reader.len();
    let header = read_header(&mut reader)?;
    let big = header.version.is_big();

    // Colour mode data: a palette or duotone curves. Not needed to find the layers.
    let mode_len = u64::from(reader.u32(file_len, "the colour mode data length")?);
    reader.skip(mode_len, file_len, "the colour mode data")?;

    let resources_len = u64::from(reader.u32(file_len, "the image resources length")?);
    let resources_end = reader
        .position()
        .checked_add(resources_len)
        .filter(|end| *end <= file_len)
        .ok_or(PsdError::Truncated {
            what: "the image resources",
        })?;
    let resources = read_resources(&mut reader, resources_end)?;
    reader.seek_to(resources_end, "the image resources")?;

    let section_len = reader.length(big, file_len, "the layer and mask section length")?;
    let section_end = reader
        .position()
        .checked_add(section_len)
        .filter(|end| *end <= file_len)
        .ok_or(PsdError::Truncated {
            what: "the layer and mask section",
        })?;

    let mut layers = LayerSection::default();
    let mut global_block_keys = Vec::new();
    if section_len > 0 {
        let (parsed, keys) = read_layer_and_mask_section(&mut reader, &header, section_end)?;
        layers = parsed;
        global_block_keys = keys;
    }
    reader.seek_to(section_end, "the layer and mask section")?;

    // The merged image: only its compression field is read here.
    let image_data = reader
        .u16(file_len, "the merged image compression")
        .ok()
        .map(|compression| ImageData {
            offset: section_end,
            compression,
        });

    Ok(Document {
        header,
        resources,
        layers,
        global_block_keys,
        image_data,
        file_len,
    })
}

fn read_header<R: Read + Seek>(reader: &mut Reader<R>) -> Result<Header, PsdError> {
    let end = reader.len();
    let signature = reader.tag(end, "the signature")?;
    if &signature != b"8BPS" {
        return Err(PsdError::NotPhotoshop);
    }
    let version = match reader.u16(end, "the version")? {
        1 => Version::Psd,
        2 => Version::Psb,
        other => return Err(PsdError::UnsupportedVersion(other)),
    };
    reader.skip(6, end, "the reserved bytes")?;
    let channels = reader.u16(end, "the channel count")?;
    let height = reader.u32(end, "the height")?;
    let width = reader.u32(end, "the width")?;
    let depth = reader.u16(end, "the depth")?;
    let mode_value = reader.u16(end, "the colour mode")?;

    if channels == 0 || channels > MAX_CHANNELS {
        return Err(PsdError::Limit {
            what: "channels",
            value: u64::from(channels),
            limit: u64::from(MAX_CHANNELS),
        });
    }
    let max = version.max_dimension();
    for (value, what) in [(width, "width"), (height, "height")] {
        if value == 0 || value > max {
            return Err(PsdError::Limit {
                what,
                value: u64::from(value),
                limit: u64::from(max),
            });
        }
    }
    if !matches!(depth, 1 | 8 | 16 | 32) {
        return Err(PsdError::Unsupported {
            what: format!("a bit depth of {depth}"),
        });
    }
    let mode = ColorMode::from_u16(mode_value).ok_or_else(|| PsdError::Unsupported {
        what: format!("colour mode {mode_value}"),
    })?;
    Ok(Header {
        version,
        channels,
        width,
        height,
        depth,
        mode,
    })
}

fn read_resources<R: Read + Seek>(reader: &mut Reader<R>, end: u64) -> Result<Resources, PsdError> {
    let mut resources = Resources::default();
    while reader.position() + 12 <= end {
        if resources.blocks.len() >= MAX_RESOURCE_BLOCKS {
            return Err(PsdError::Limit {
                what: "image resource blocks",
                value: resources.blocks.len() as u64 + 1,
                limit: MAX_RESOURCE_BLOCKS as u64,
            });
        }
        let signature = reader.tag(end, "an image resource signature")?;
        if signature != SIG_8BIM {
            // Not a block this reader knows how to step over; what was found so far stands.
            break;
        }
        let id = reader.u16(end, "an image resource id")?;
        // A Pascal string padded so that its total length is even.
        let name_len = u64::from(reader.u8(end, "an image resource name")?);
        let name_total = 1 + name_len;
        reader.skip(name_len + (name_total % 2), end, "an image resource name")?;
        let length = u64::from(reader.u32(end, "an image resource size")?);
        let data_start = reader.position();
        let data_end = data_start
            .checked_add(length)
            .filter(|data_end| *data_end <= end)
            .ok_or(PsdError::Truncated {
                what: "an image resource",
            })?;
        match id {
            1039 => {
                if length <= MAX_ICC_BYTES {
                    resources.icc_profile =
                        Some(reader.bytes(length, MAX_ICC_BYTES, end, "the ICC profile")?);
                }
            }
            1058 if length <= crate::metadata::exif::MAX_EXIF_BYTES as u64 => {
                resources.exif = Some(reader.bytes(
                    length,
                    crate::metadata::exif::MAX_EXIF_BYTES as u64,
                    end,
                    "the EXIF block",
                )?);
            }
            1060 if length <= crate::metadata::xmp::MAX_XMP_BYTES as u64 => {
                resources.xmp = Some(reader.bytes(
                    length,
                    crate::metadata::xmp::MAX_XMP_BYTES as u64,
                    end,
                    "the XMP packet",
                )?);
            }
            1028 if length <= crate::metadata::iptc::MAX_IPTC_BYTES as u64 => {
                resources.iptc = Some(reader.bytes(
                    length,
                    crate::metadata::iptc::MAX_IPTC_BYTES as u64,
                    end,
                    "the IPTC block",
                )?);
            }
            1005 if length >= 16 => {
                let horizontal = reader.u32(end, "the resolution")?;
                reader.skip(4, end, "the resolution")?;
                let vertical = reader.u32(end, "the resolution")?;
                // Fixed-point 16.16, pixels per inch.
                resources.resolution = Some(Resolution {
                    horizontal_ppi: horizontal as f32 / 65_536.0,
                    vertical_ppi: vertical as f32 / 65_536.0,
                });
            }
            _ => {}
        }
        resources.blocks.push(ResourceBlock { id, length });
        // The data is padded to an even length.
        let next = data_end + (length % 2);
        reader.seek_to(next.min(end), "an image resource")?;
    }
    Ok(resources)
}

fn read_layer_and_mask_section<R: Read + Seek>(
    reader: &mut Reader<R>,
    header: &Header,
    section_end: u64,
) -> Result<(LayerSection, Vec<[u8; 4]>), PsdError> {
    let big = header.version.is_big();
    // Layer info: its own length, then the records and their channel data.
    let info_len = reader.length(big, section_end, "the layer info length")?;
    let info_start = reader.position();
    let info_end = info_start
        .checked_add(info_len)
        .filter(|end| *end <= section_end)
        .ok_or(PsdError::Truncated {
            what: "the layer info",
        })?;
    let mut layers = LayerSection::default();
    if info_len > 0 {
        layers = read_layer_records(reader, header, info_end)?;
    }
    reader.seek_to(info_end, "the layer info")?;

    // Global layer mask info: skipped by its length.
    if reader.position() + 4 <= section_end {
        let mask_len = u64::from(reader.u32(section_end, "the global layer mask length")?);
        reader.skip(mask_len, section_end, "the global layer mask info")?;
    }

    // Additional information at document level. For 16- and 32-bit files this is where
    // the layers are: `Lr16` / `Lr32` (and `Layr` in some files).
    let mut keys = Vec::new();
    while reader.position() + 12 <= section_end {
        if keys.len() >= MAX_TAGGED_BLOCKS {
            return Err(PsdError::Limit {
                what: "document-level blocks",
                value: keys.len() as u64 + 1,
                limit: MAX_TAGGED_BLOCKS as u64,
            });
        }
        let signature = reader.tag(section_end, "a block signature")?;
        if signature != SIG_8BIM && signature != SIG_8B64 {
            break;
        }
        let key = reader.tag(section_end, "a block key")?;
        let length = reader.length(big && is_long_key(&key), section_end, "a block length")?;
        let data_start = reader.position();
        let data_end = data_start
            .checked_add(length)
            .filter(|end| *end <= section_end)
            .ok_or(PsdError::Truncated { what: "a block" })?;
        keys.push(key);
        if layers.records.is_empty() && matches!(&key, b"Lr16" | b"Lr32" | b"Layr") {
            // The block's data is the layer info without its own length field.
            let mut parsed = read_layer_records(reader, header, data_end)?;
            parsed.from_block = Some(key);
            layers = parsed;
        }
        reader.seek_to(data_end, "a block")?;
        let next = next_block_start(reader, data_end, section_end);
        reader.seek_to(next, "a block")?;
    }
    Ok((layers, keys))
}

/// Where the next block begins after data that ends at `data_end`. Writers pad blocks by up
/// to three bytes, relative to different things, so rather than guess an alignment the
/// next signature is looked for in the next four positions. A pad is zeros and a signature
/// starts with `8`, so the first position that holds one is the answer; none means the
/// region is finished.
fn next_block_start<R: Read + Seek>(reader: &mut Reader<R>, data_end: u64, region_end: u64) -> u64 {
    for skip in 0..=3_u64 {
        let candidate = data_end + skip;
        if candidate >= region_end {
            return region_end;
        }
        if reader.seek_to(candidate, "a block").is_ok() {
            if let Some(tag) = reader.peek_tag(region_end) {
                if tag == SIG_8BIM || tag == SIG_8B64 {
                    return candidate;
                }
            }
        }
    }
    data_end.min(region_end)
}

fn read_layer_records<R: Read + Seek>(
    reader: &mut Reader<R>,
    header: &Header,
    info_end: u64,
) -> Result<LayerSection, PsdError> {
    let big = header.version.is_big();
    let count = reader.i16(info_end, "the layer count")?;
    let merged_alpha = count < 0;
    let records = i32::from(count).unsigned_abs() as u64;
    if records > MAX_LAYER_RECORDS {
        return Err(PsdError::Limit {
            what: "layer records",
            value: records,
            limit: MAX_LAYER_RECORDS,
        });
    }
    // A record is at least 34 bytes, so a count the section cannot hold is refused before
    // any record is read.
    let minimum = records.saturating_mul(34);
    if minimum > info_end.saturating_sub(reader.position()) {
        return Err(PsdError::Truncated {
            what: "the layer records",
        });
    }
    let mut parsed = Vec::with_capacity(records as usize);
    for _ in 0..records {
        parsed.push(read_record(reader, big, info_end)?);
    }
    // Channel image data follows every record, in record order, channel by channel.
    let mut offset = reader.position();
    for record in &mut parsed {
        for channel in &mut record.channels {
            channel.offset = offset;
            offset = offset
                .checked_add(channel.length)
                .filter(|next| *next <= info_end)
                .ok_or(PsdError::Truncated {
                    what: "the layer channel data",
                })?;
        }
    }
    Ok(LayerSection {
        records: parsed,
        merged_alpha,
        from_block: None,
    })
}

fn read_rect<R: Read + Seek>(
    reader: &mut Reader<R>,
    limit: u64,
    what: &'static str,
) -> Result<Rect, PsdError> {
    Ok(Rect {
        top: reader.i32(limit, what)?,
        left: reader.i32(limit, what)?,
        bottom: reader.i32(limit, what)?,
        right: reader.i32(limit, what)?,
    })
}

fn read_record<R: Read + Seek>(
    reader: &mut Reader<R>,
    big: bool,
    info_end: u64,
) -> Result<LayerRecord, PsdError> {
    let rect = read_rect(reader, info_end, "a layer rectangle")?;
    let channel_count = reader.u16(info_end, "a layer's channel count")?;
    if channel_count > MAX_CHANNELS {
        return Err(PsdError::Limit {
            what: "channels in one layer",
            value: u64::from(channel_count),
            limit: u64::from(MAX_CHANNELS),
        });
    }
    let mut channels = Vec::with_capacity(usize::from(channel_count));
    for _ in 0..channel_count {
        let id = reader.i16(info_end, "a channel id")?;
        let length = reader.length(big, info_end, "a channel length")?;
        channels.push(ChannelInfo {
            id,
            length,
            offset: 0,
        });
    }
    let signature = reader.tag(info_end, "a blend mode signature")?;
    if signature != SIG_8BIM {
        return Err(PsdError::Malformed {
            what: "a layer record is not followed by a blend mode signature",
        });
    }
    let blend_key = reader.tag(info_end, "a blend mode key")?;
    let opacity = reader.u8(info_end, "a layer opacity")?;
    let clipping = reader.u8(info_end, "a layer clipping flag")?;
    let flags = reader.u8(info_end, "a layer flags byte")?;
    reader.skip(1, info_end, "a layer filler")?;

    let extra_len = u64::from(reader.u32(info_end, "a layer's extra data length")?);
    let extra_start = reader.position();
    let extra_end = extra_start
        .checked_add(extra_len)
        .filter(|end| *end <= info_end)
        .ok_or(PsdError::Truncated {
            what: "a layer's extra data",
        })?;

    // Layer mask / adjustment data.
    let mask_len = u64::from(reader.u32(extra_end, "a layer mask length")?);
    let mask_start = reader.position();
    let mask_end = mask_start
        .checked_add(mask_len)
        .filter(|end| *end <= extra_end)
        .ok_or(PsdError::Truncated {
            what: "a layer mask",
        })?;
    let mut mask = None;
    if mask_len >= 18 {
        let mask_rect = read_rect(reader, mask_end, "a layer mask rectangle")?;
        let default_color = reader.u8(mask_end, "a layer mask default colour")?;
        let mask_flags = reader.u8(mask_end, "a layer mask flags")?;
        // A block of 36 bytes or more carries the real mask's flags, background and
        // rectangle right after the first mask's; a 20-byte block ends with two padding bytes.
        // (Real Photoshop files put these before the mask parameters, which is not the order
        // the specification's table suggests: `layer_mask_data.psd` in psd-tools' fixtures.)
        let mut real = None;
        if mask_len >= 36 {
            let real_flags = reader.u8(mask_end, "the real mask flags")?;
            let real_default = reader.u8(mask_end, "the real mask background")?;
            let real_rect = read_rect(reader, mask_end, "the real mask rectangle")?;
            real = Some(RealMask {
                rect: real_rect,
                default_color: real_default,
                flags: real_flags,
            });
        }
        // Bit 4: mask parameters follow (a presence byte, then a density byte and/or an
        // 8-byte feather for the user and vector mask). Not needed to place the pixels, so
        // they are stepped over; whatever remains in the block is padding.
        if mask_flags & 0b1_0000 != 0 && mask_end.saturating_sub(reader.position()) >= 1 {
            let present = reader.u8(mask_end, "the layer mask parameters")?;
            for bit in 0..4_u8 {
                if present & (1 << bit) != 0 {
                    // Bits 0 and 2 are a one-byte density, bits 1 and 3 an eight-byte feather.
                    let size = if bit % 2 == 0 { 1 } else { 8 };
                    reader.skip(size, mask_end, "the layer mask parameters")?;
                }
            }
        }
        mask = Some(MaskData {
            rect: mask_rect,
            default_color,
            flags: mask_flags,
            real,
        });
    } else if mask_len != 0 {
        return Err(PsdError::Malformed {
            what: "a layer mask block is too short",
        });
    }
    reader.seek_to(mask_end, "a layer mask")?;

    // Blending ranges: not used.
    let ranges_len = u64::from(reader.u32(extra_end, "the blending ranges length")?);
    reader.skip(ranges_len, extra_end, "the blending ranges")?;

    // The layer name: a Pascal string padded to a multiple of 4 bytes.
    let name_len = u64::from(reader.u8(extra_end, "a layer name")?);
    let name_bytes = reader.bytes(name_len, 255, extra_end, "a layer name")?;
    let padding = (4 - (1 + name_len) % 4) % 4;
    reader.skip(padding, extra_end, "a layer name padding")?;
    let name: String = name_bytes.iter().map(|byte| char::from(*byte)).collect();

    let mut record = LayerRecord {
        rect,
        channels,
        blend_key,
        opacity,
        clipped: clipping != 0,
        flags,
        name,
        unicode_name: None,
        mask,
        divider: None,
        divider_blend_key: None,
        layer_id: None,
        fill_opacity: None,
        blend_clipped_as_group: true,
        adjustment: None,
        block_keys: Vec::new(),
    };
    read_layer_blocks(reader, big, extra_end, &mut record)?;
    reader.seek_to(extra_end, "a layer's extra data")?;
    Ok(record)
}

fn read_layer_blocks<R: Read + Seek>(
    reader: &mut Reader<R>,
    big: bool,
    end: u64,
    record: &mut LayerRecord,
) -> Result<(), PsdError> {
    while reader.position() + 12 <= end {
        if record.block_keys.len() >= MAX_TAGGED_BLOCKS {
            return Err(PsdError::Limit {
                what: "additional information blocks in one layer",
                value: record.block_keys.len() as u64 + 1,
                limit: MAX_TAGGED_BLOCKS as u64,
            });
        }
        let signature = reader.tag(end, "a layer block signature")?;
        if signature != SIG_8BIM && signature != SIG_8B64 {
            break;
        }
        let key = reader.tag(end, "a layer block key")?;
        let length = reader.length(big && is_long_key(&key), end, "a layer block length")?;
        let data_start = reader.position();
        let data_end = data_start
            .checked_add(length)
            .filter(|data_end| *data_end <= end)
            .ok_or(PsdError::Truncated {
                what: "a layer block",
            })?;
        record.block_keys.push(key);
        match &key {
            b"luni" if length >= 4 => {
                let units = u64::from(reader.u32(data_end, "a Unicode layer name")?);
                if units <= MAX_NAME_UNITS && units * 2 <= length - 4 {
                    let raw = reader.bytes(
                        units * 2,
                        MAX_NAME_UNITS * 2,
                        data_end,
                        "a Unicode layer name",
                    )?;
                    let wide: Vec<u16> = raw
                        .chunks_exact(2)
                        .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                        .collect();
                    let text = String::from_utf16_lossy(&wide);
                    record.unicode_name = Some(text.trim_end_matches('\0').to_string());
                }
            }
            b"lsct" | b"lsdk" if length >= 4 => {
                let kind = reader.u32(data_end, "a section divider type")?;
                record.divider = Some(match kind {
                    1 => Divider::OpenFolder,
                    2 => Divider::ClosedFolder,
                    3 => Divider::BoundingSection,
                    _ => Divider::Other,
                });
                if length >= 12 {
                    let signature = reader.tag(data_end, "a section divider signature")?;
                    if signature == SIG_8BIM {
                        record.divider_blend_key =
                            Some(reader.tag(data_end, "a section divider blend mode")?);
                    }
                }
            }
            b"lyid" if length >= 4 => {
                record.layer_id = Some(reader.u32(data_end, "a layer id")?);
            }
            b"iOpa" if length >= 1 => {
                record.fill_opacity = Some(reader.u8(data_end, "a fill opacity")?);
            }
            b"clbl" if length >= 1 => {
                record.blend_clipped_as_group = reader.u8(data_end, "a clipping blend flag")? != 0;
            }
            b"curv" | b"levl" | b"nvrt" | b"expA"
                if length <= super::adjustment::MAX_BLOCK_BYTES && record.adjustment.is_none() =>
            {
                let data = reader.bytes(
                    length,
                    super::adjustment::MAX_BLOCK_BYTES,
                    data_end,
                    "an adjustment layer's settings",
                )?;
                record.adjustment = Some((key, data));
            }
            _ => {}
        }
        reader.seek_to(data_end, "a layer block")?;
        let next = next_block_start(reader, data_end, end);
        reader.seek_to(next, "a layer block")?;
    }
    Ok(())
}
