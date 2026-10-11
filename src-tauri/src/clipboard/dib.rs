//! Windows device-independent bitmaps (`CF_DIB`, `CF_DIBV5`) as pure functions on bytes.
//!
//! The clipboard is a hostile-input surface: whatever program last copied something decided what
//! these bytes are. Nothing here touches the clipboard or trusts a size the bytes claim; every
//! length is checked against the data that is there and against a pixel limit before a buffer is
//! made, and every function is a total function of its input (tests cut and change a sample at
//! every position).
//!
//! What is read: 1, 4, 8, 16, 24 and 32 bits per pixel, uncompressed (`BI_RGB`) or with colour
//! masks (`BI_BITFIELDS`, `BI_ALPHABITFIELDS`), bottom-up or top-down, with a palette where the
//! depth has one, in a `BITMAPINFOHEADER` or a V4 / V5 header. RLE-compressed bitmaps, JPEG and
//! PNG payloads inside a DIB, and OS/2 core headers are named and refused.
use image::{Rgba, RgbaImage};

const BI_RGB: u32 = 0;
const BI_RLE8: u32 = 1;
const BI_RLE4: u32 = 2;
const BI_BITFIELDS: u32 = 3;
const BI_JPEG: u32 = 4;
const BI_PNG: u32 = 5;
const BI_ALPHABITFIELDS: u32 = 6;

/// The size of a `BITMAPV5HEADER`.
pub const V5_HEADER_BYTES: usize = 124;
/// `LCS_sRGB`: the four characters "sRGB".
const LCS_SRGB: u32 = 0x7352_4742;
/// `LCS_GM_IMAGES`: colorimetric-preserving rendering for pictures.
const LCS_GM_IMAGES: u32 = 4;

/// Why a bitmap was not read. The text is for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DibError {
    Truncated(&'static str),
    UnsupportedHeader(u32),
    UnsupportedCompression(u32),
    UnsupportedDepth(u16),
    BadDimensions,
    TooLarge { pixels: u64, limit: u64 },
    BadMasks,
}

impl std::fmt::Display for DibError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated(what) => write!(f, "the bitmap on the clipboard is cut short ({what})"),
            Self::UnsupportedHeader(size) => write!(
                f,
                "the bitmap on the clipboard has a {size}-byte header, which is not one PhotoForge reads"
            ),
            Self::UnsupportedCompression(code) => write!(
                f,
                "the bitmap on the clipboard is compressed ({}), which PhotoForge does not read",
                match *code {
                    BI_RLE8 => "RLE8",
                    BI_RLE4 => "RLE4",
                    BI_JPEG => "JPEG",
                    BI_PNG => "PNG",
                    _ => "an unknown method",
                }
            ),
            Self::UnsupportedDepth(bits) => write!(
                f,
                "the bitmap on the clipboard has {bits} bits per pixel, which PhotoForge does not read"
            ),
            Self::BadDimensions => write!(f, "the bitmap on the clipboard has an impossible size"),
            Self::TooLarge { pixels, limit } => write!(
                f,
                "the bitmap on the clipboard is {pixels} pixels, more than the {limit} PhotoForge works with"
            ),
            Self::BadMasks => write!(f, "the bitmap on the clipboard has colour masks that make no sense"),
        }
    }
}

impl std::error::Error for DibError {}

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn i32_at(bytes: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

/// Encodes `image` as a packed `CF_DIBV5`: a 124-byte header, then 32-bit BGRA rows, bottom-up,
/// with colour masks and an alpha mask, straight (not premultiplied) alpha, tagged sRGB.
///
/// `mask_table`: whether the three colour masks are repeated, as a table of three DWORDs, between
/// the header and the pixels. The V5 header carries the masks itself, but Windows' own synthesised
/// `CF_DIBV5` has the table too (see `decode`), so it is a choice that the clipboard tests make
/// by asking Windows' own readers.
pub fn encode_v5(image: &RgbaImage, mask_table: bool) -> Vec<u8> {
    let (width, height) = image.dimensions();
    let size_image = width as usize * height as usize * 4;
    let mut out = Vec::with_capacity(V5_HEADER_BYTES + 12 + size_image);
    out.extend_from_slice(&(V5_HEADER_BYTES as u32).to_le_bytes()); // bV5Size
    out.extend_from_slice(&(width as i32).to_le_bytes()); // bV5Width
    out.extend_from_slice(&(height as i32).to_le_bytes()); // bV5Height: positive, bottom-up
    out.extend_from_slice(&1_u16.to_le_bytes()); // bV5Planes
    out.extend_from_slice(&32_u16.to_le_bytes()); // bV5BitCount
    out.extend_from_slice(&BI_BITFIELDS.to_le_bytes()); // bV5Compression
    out.extend_from_slice(&(size_image as u32).to_le_bytes()); // bV5SizeImage
    out.extend_from_slice(&0_i32.to_le_bytes()); // XPelsPerMeter
    out.extend_from_slice(&0_i32.to_le_bytes()); // YPelsPerMeter
    out.extend_from_slice(&0_u32.to_le_bytes()); // ClrUsed
    out.extend_from_slice(&0_u32.to_le_bytes()); // ClrImportant
    out.extend_from_slice(&0x00FF_0000_u32.to_le_bytes()); // RedMask
    out.extend_from_slice(&0x0000_FF00_u32.to_le_bytes()); // GreenMask
    out.extend_from_slice(&0x0000_00FF_u32.to_le_bytes()); // BlueMask
    out.extend_from_slice(&0xFF00_0000_u32.to_le_bytes()); // AlphaMask
    out.extend_from_slice(&LCS_SRGB.to_le_bytes()); // bV5CSType
    out.extend_from_slice(&[0_u8; 36]); // bV5Endpoints (CIEXYZTRIPLE)
    out.extend_from_slice(&[0_u8; 12]); // gamma red, green, blue
    out.extend_from_slice(&LCS_GM_IMAGES.to_le_bytes()); // bV5Intent
    out.extend_from_slice(&[0_u8; 12]); // profile data, profile size, reserved
    debug_assert_eq!(out.len(), V5_HEADER_BYTES);
    if mask_table {
        out.extend_from_slice(&0x00FF_0000_u32.to_le_bytes());
        out.extend_from_slice(&0x0000_FF00_u32.to_le_bytes());
        out.extend_from_slice(&0x0000_00FF_u32.to_le_bytes());
    }
    for y in (0..height).rev() {
        for x in 0..width {
            let Rgba([r, g, b, a]) = *image.get_pixel(x, y);
            out.extend_from_slice(&[b, g, r, a]);
        }
    }
    out
}

/// How to read one channel out of a 16- or 32-bit pixel.
#[derive(Debug, Clone, Copy)]
struct Channel {
    mask: u32,
    shift: u32,
    /// The channel's largest value.
    max: u32,
}

impl Channel {
    fn new(mask: u32) -> Option<Self> {
        if mask == 0 {
            return None;
        }
        let shift = mask.trailing_zeros();
        let max = mask >> shift;
        // A run of contiguous ones: the next value up is a power of two.
        if max & (max + 1) != 0 {
            return None;
        }
        Some(Self { mask, shift, max })
    }

    fn value(self, pixel: u32) -> u8 {
        let raw = (pixel & self.mask) >> self.shift;
        if self.max == 255 {
            raw as u8
        } else {
            // Wide masks (10, 12, 16 bits) are scaled to 8 bits, rounding to nearest.
            ((u64::from(raw) * 255 + u64::from(self.max) / 2) / u64::from(self.max)) as u8
        }
    }
}

/// Reads a packed DIB (`CF_DIB` or `CF_DIBV5`) into an RGBA image.
///
/// `max_pixels` bounds the picture before any pixel buffer exists.
pub fn decode(bytes: &[u8], max_pixels: u64) -> Result<RgbaImage, DibError> {
    let header_size = u32_at(bytes, 0).ok_or(DibError::Truncated("header"))?;
    if !matches!(header_size, 40 | 52 | 56 | 108 | 124) {
        return Err(DibError::UnsupportedHeader(header_size));
    }
    let header_size = header_size as usize;
    if bytes.len() < header_size {
        return Err(DibError::Truncated("header"));
    }
    let width = i32_at(bytes, 4).ok_or(DibError::Truncated("header"))?;
    let raw_height = i32_at(bytes, 8).ok_or(DibError::Truncated("header"))?;
    let planes = u16_at(bytes, 12).ok_or(DibError::Truncated("header"))?;
    let bit_count = u16_at(bytes, 14).ok_or(DibError::Truncated("header"))?;
    let compression = u32_at(bytes, 16).ok_or(DibError::Truncated("header"))?;
    let colours_used = u32_at(bytes, 32).ok_or(DibError::Truncated("header"))?;

    if planes != 1 || width <= 0 || raw_height == 0 || raw_height == i32::MIN {
        return Err(DibError::BadDimensions);
    }
    let top_down = raw_height < 0;
    let width = width as u32;
    let height = raw_height.unsigned_abs();
    let pixels = u64::from(width) * u64::from(height);
    if pixels > max_pixels {
        return Err(DibError::TooLarge {
            pixels,
            limit: max_pixels,
        });
    }
    if !matches!(bit_count, 1 | 4 | 8 | 16 | 24 | 32) {
        return Err(DibError::UnsupportedDepth(bit_count));
    }
    if !matches!(compression, BI_RGB | BI_BITFIELDS | BI_ALPHABITFIELDS) {
        return Err(DibError::UnsupportedCompression(compression));
    }
    if matches!(compression, BI_BITFIELDS | BI_ALPHABITFIELDS) && !matches!(bit_count, 16 | 32) {
        return Err(DibError::BadMasks);
    }

    // Where the masks, the palette and the pixels are.
    let mut cursor = header_size;
    let mut masks = [0_u32; 4];
    let has_mask_fields = matches!(compression, BI_BITFIELDS | BI_ALPHABITFIELDS);
    if header_size >= 56 {
        // A V3, V4 or V5 header carries its masks inside it.
        for (index, slot) in masks.iter_mut().enumerate() {
            *slot = u32_at(bytes, 40 + index * 4).ok_or(DibError::Truncated("masks"))?;
        }
    } else if header_size == 52 {
        for (index, slot) in masks.iter_mut().take(3).enumerate() {
            *slot = u32_at(bytes, 40 + index * 4).ok_or(DibError::Truncated("masks"))?;
        }
    } else if has_mask_fields {
        // A plain BITMAPINFOHEADER keeps three masks (four for BI_ALPHABITFIELDS) after it.
        let count = if compression == BI_ALPHABITFIELDS {
            4
        } else {
            3
        };
        for (index, slot) in masks.iter_mut().take(count).enumerate() {
            *slot = u32_at(bytes, cursor + index * 4).ok_or(DibError::Truncated("masks"))?;
        }
        cursor += count * 4;
    }

    // Windows' own synthesised CF_DIBV5 (made from a device-dependent bitmap) keeps the table of
    // three masks after a header that already has them. It is told by the three DWORDs after the
    // header being those masks, which no pixel data of an image that has them in its header would
    // be by accident.
    if has_mask_fields
        && header_size >= 56
        && bytes.len() >= cursor + 12
        && (0..3).all(|index| u32_at(bytes, cursor + index * 4) == Some(masks[index]))
        && masks[..3].iter().all(|mask| *mask != 0)
    {
        cursor += 12;
    }

    let palette_entries = if bit_count <= 8 {
        let all = 1_usize << bit_count;
        match colours_used as usize {
            0 => all,
            used if used <= all => used,
            _ => return Err(DibError::BadDimensions),
        }
    } else {
        0
    };
    let palette_start = cursor;
    cursor = cursor
        .checked_add(palette_entries * 4)
        .ok_or(DibError::BadDimensions)?;
    if bytes.len() < cursor {
        return Err(DibError::Truncated("palette"));
    }

    let bits_per_row = u64::from(width) * u64::from(bit_count);
    let stride = bits_per_row.div_ceil(32) * 4;
    let needed = stride
        .checked_mul(u64::from(height))
        .ok_or(DibError::BadDimensions)?;
    let available = (bytes.len() - cursor) as u64;
    if available < needed {
        return Err(DibError::Truncated("pixels"));
    }
    let data = &bytes[cursor..];
    let stride = stride as usize;

    // Channel layout for 16- and 32-bit pixels.
    let channels: Option<[Option<Channel>; 4]> = match bit_count {
        16 | 32 => {
            if has_mask_fields || (header_size >= 52 && masks[..3].iter().any(|m| *m != 0)) {
                let channel = |mask: u32| Channel::new(mask);
                let (r, g, b) = (channel(masks[0]), channel(masks[1]), channel(masks[2]));
                if r.is_none() || g.is_none() || b.is_none() {
                    return Err(DibError::BadMasks);
                }
                // The alpha mask counts only when the compression says masks are in use, or the
                // header is one whose alpha field is defined (V4 / V5).
                let alpha_defined = compression == BI_ALPHABITFIELDS || header_size >= 108;
                let a = if alpha_defined {
                    channel(masks[3])
                } else {
                    None
                };
                Some([r, g, b, a])
            } else if bit_count == 16 {
                // BI_RGB at 16 bits is 5-5-5.
                Some([
                    Channel::new(0x7C00),
                    Channel::new(0x03E0),
                    Channel::new(0x001F),
                    None,
                ])
            } else {
                // BI_RGB at 32 bits is B, G, R and an unused byte.
                Some([
                    Channel::new(0x00FF_0000),
                    Channel::new(0x0000_FF00),
                    Channel::new(0x0000_00FF),
                    None,
                ])
            }
        }
        _ => None,
    };

    let palette: Vec<[u8; 3]> = (0..palette_entries)
        .map(|index| {
            let at = palette_start + index * 4;
            // RGBQUAD is B, G, R, reserved.
            [bytes[at + 2], bytes[at + 1], bytes[at]]
        })
        .collect();

    let mut image = RgbaImage::new(width, height);
    let mut any_alpha = false;
    for row in 0..height {
        let source_row = if top_down { row } else { height - 1 - row };
        let line = &data[source_row as usize * stride..][..stride];
        for x in 0..width {
            let pixel = match bit_count {
                1 | 4 | 8 => {
                    let bits = u32::from(bit_count);
                    let bit_index = x * bits;
                    let byte = line[(bit_index / 8) as usize];
                    let shift = 8 - bits - (bit_index % 8);
                    let index = ((u32::from(byte) >> shift) & ((1 << bits) - 1)) as usize;
                    let [r, g, b] = palette.get(index).copied().unwrap_or([0, 0, 0]);
                    Rgba([r, g, b, 255])
                }
                24 => {
                    let at = x as usize * 3;
                    Rgba([line[at + 2], line[at + 1], line[at], 255])
                }
                16 => {
                    let at = x as usize * 2;
                    let value = u32::from(u16::from_le_bytes([line[at], line[at + 1]]));
                    let [r, g, b, a] = channels.expect("16-bit pixels have channels");
                    let (r, g, b) = (r.unwrap(), g.unwrap(), b.unwrap());
                    let alpha = a.map_or(255, |a| a.value(value));
                    any_alpha |= alpha != 0 && a.is_some();
                    Rgba([r.value(value), g.value(value), b.value(value), alpha])
                }
                32 => {
                    let at = x as usize * 4;
                    let value =
                        u32::from_le_bytes([line[at], line[at + 1], line[at + 2], line[at + 3]]);
                    let [r, g, b, a] = channels.expect("32-bit pixels have channels");
                    let (r, g, b) = (r.unwrap(), g.unwrap(), b.unwrap());
                    let alpha = a.map_or(255, |a| a.value(value));
                    any_alpha |= alpha != 0 && a.is_some();
                    Rgba([r.value(value), g.value(value), b.value(value), alpha])
                }
                _ => unreachable!("depth was checked"),
            };
            image.put_pixel(x, row, pixel);
        }
    }
    // An alpha channel that is zero at every pixel was never filled (a great many programs write
    // a 32-bit bitmap and leave the fourth byte zero). Taken at its word the picture would be
    // invisible, so it is opaque.
    let has_alpha_channel = channels.is_some_and(|c| c[3].is_some());
    if has_alpha_channel && !any_alpha {
        for pixel in image.pixels_mut() {
            pixel.0[3] = 255;
        }
    }
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(
        size: u32,
        width: i32,
        height: i32,
        bits: u16,
        compression: u32,
        colours_used: u32,
    ) -> Vec<u8> {
        let mut h = vec![0_u8; size as usize];
        h[0..4].copy_from_slice(&size.to_le_bytes());
        h[4..8].copy_from_slice(&width.to_le_bytes());
        h[8..12].copy_from_slice(&height.to_le_bytes());
        h[12..14].copy_from_slice(&1_u16.to_le_bytes());
        h[14..16].copy_from_slice(&bits.to_le_bytes());
        h[16..20].copy_from_slice(&compression.to_le_bytes());
        h[32..36].copy_from_slice(&colours_used.to_le_bytes());
        h
    }

    fn sample() -> RgbaImage {
        let mut image = RgbaImage::new(3, 2);
        image.put_pixel(0, 0, Rgba([255, 0, 0, 255]));
        image.put_pixel(1, 0, Rgba([0, 255, 0, 128]));
        image.put_pixel(2, 0, Rgba([0, 0, 255, 0]));
        image.put_pixel(0, 1, Rgba([10, 20, 30, 255]));
        image.put_pixel(1, 1, Rgba([200, 100, 50, 77]));
        image.put_pixel(2, 1, Rgba([255, 255, 255, 255]));
        image
    }

    #[test]
    fn a_v5_bitmap_encodes_and_decodes_to_the_same_picture() {
        let image = sample();
        let bytes = encode_v5(&image, false);
        assert_eq!(bytes.len(), V5_HEADER_BYTES + 3 * 2 * 4);
        assert_eq!(decode(&bytes, 1 << 20).unwrap(), image);
    }

    #[test]
    fn a_mask_table_after_the_header_is_skipped_and_not_read_as_pixels() {
        // What Windows synthesises for a CF_DIBV5: the header, a table of the three masks, then
        // the pixels. It reads as the same picture as one without the table.
        let image = sample();
        let with_table = encode_v5(&image, true);
        assert_eq!(with_table.len(), V5_HEADER_BYTES + 12 + 3 * 2 * 4);
        assert_eq!(decode(&with_table, 100).unwrap(), image);
        assert_eq!(decode(&encode_v5(&image, false), 100).unwrap(), image);
    }

    #[test]
    fn the_header_is_what_windows_documents() {
        let bytes = encode_v5(&sample(), false);
        assert_eq!(u32_at(&bytes, 0), Some(124));
        assert_eq!(i32_at(&bytes, 4), Some(3));
        assert_eq!(i32_at(&bytes, 8), Some(2)); // positive: bottom-up
        assert_eq!(u16_at(&bytes, 12), Some(1));
        assert_eq!(u16_at(&bytes, 14), Some(32));
        assert_eq!(u32_at(&bytes, 16), Some(BI_BITFIELDS));
        assert_eq!(u32_at(&bytes, 20), Some(24));
        assert_eq!(u32_at(&bytes, 40), Some(0x00FF_0000));
        assert_eq!(u32_at(&bytes, 44), Some(0x0000_FF00));
        assert_eq!(u32_at(&bytes, 48), Some(0x0000_00FF));
        assert_eq!(u32_at(&bytes, 52), Some(0xFF00_0000));
        assert_eq!(u32_at(&bytes, 56), Some(0x7352_4742));
        // The first pixel stored is the bottom-left one: (10, 20, 30) as B, G, R, A.
        assert_eq!(&bytes[124..128], &[30, 20, 10, 255]);
    }

    #[test]
    fn a_24_bit_bitmap_pads_its_rows_to_four_bytes() {
        // 3 pixels wide: 9 bytes of pixels + 3 of padding per row; two rows, bottom-up.
        let mut bytes = header(40, 3, 2, 24, BI_RGB, 0);
        // Bottom row (image row 1): red, green, blue as B G R.
        bytes.extend_from_slice(&[0, 0, 255, 0, 255, 0, 255, 0, 0, 9, 9, 9]);
        // Top row (image row 0): white, black, grey.
        bytes.extend_from_slice(&[255, 255, 255, 0, 0, 0, 128, 128, 128, 9, 9, 9]);
        let image = decode(&bytes, 100).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [255, 255, 255, 255]);
        assert_eq!(image.get_pixel(2, 0).0, [128, 128, 128, 255]);
        assert_eq!(image.get_pixel(0, 1).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(1, 1).0, [0, 255, 0, 255]);
        assert_eq!(image.get_pixel(2, 1).0, [0, 0, 255, 255]);
    }

    #[test]
    fn a_negative_height_is_a_top_down_bitmap() {
        let mut bytes = header(40, 1, -2, 24, BI_RGB, 0);
        bytes.extend_from_slice(&[0, 0, 255, 0]); // top: red
        bytes.extend_from_slice(&[255, 0, 0, 0]); // bottom: blue
        let image = decode(&bytes, 100).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(0, 1).0, [0, 0, 255, 255]);
    }

    #[test]
    fn a_32_bit_bitmap_without_alpha_is_opaque_whatever_the_fourth_byte_holds() {
        let mut bytes = header(40, 2, 1, 32, BI_RGB, 0);
        bytes.extend_from_slice(&[10, 20, 30, 0, 40, 50, 60, 7]);
        let image = decode(&bytes, 100).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [30, 20, 10, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [60, 50, 40, 255]);
    }

    #[test]
    fn an_alpha_channel_nobody_filled_is_ignored() {
        // A V5 bitmap whose alpha mask is declared and whose alpha is zero everywhere.
        let image = {
            let mut image = sample();
            for pixel in image.pixels_mut() {
                pixel.0[3] = 0;
            }
            image
        };
        let decoded = decode(&encode_v5(&image, false), 100).unwrap();
        assert!(decoded.pixels().all(|p| p.0[3] == 255));
        assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
        // One pixel with coverage and the channel is real, zeros included.
        let mut partly = image.clone();
        partly.put_pixel(0, 0, Rgba([255, 0, 0, 1]));
        let decoded = decode(&encode_v5(&partly, false), 100).unwrap();
        assert_eq!(decoded.get_pixel(0, 0).0[3], 1);
        assert_eq!(decoded.get_pixel(1, 0).0[3], 0);
    }

    #[test]
    fn an_8_bit_bitmap_reads_through_its_palette() {
        // 2 x 2, palette of three, rows padded to four bytes.
        let mut bytes = header(40, 2, 2, 8, BI_RGB, 3);
        bytes.extend_from_slice(&[0, 0, 255, 0]); // index 0: red (B, G, R, _)
        bytes.extend_from_slice(&[0, 255, 0, 0]); // index 1: green
        bytes.extend_from_slice(&[255, 0, 0, 0]); // index 2: blue
        bytes.extend_from_slice(&[2, 1, 0, 0]); // bottom row
        bytes.extend_from_slice(&[0, 2, 0, 0]); // top row
        let image = decode(&bytes, 100).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [0, 0, 255, 255]);
        assert_eq!(image.get_pixel(0, 1).0, [0, 0, 255, 255]);
        assert_eq!(image.get_pixel(1, 1).0, [0, 255, 0, 255]);
    }

    #[test]
    fn a_1_bit_bitmap_reads_bits_left_to_right() {
        let mut bytes = header(40, 3, 1, 1, BI_RGB, 2);
        bytes.extend_from_slice(&[0, 0, 0, 0]); // 0: black
        bytes.extend_from_slice(&[255, 255, 255, 0]); // 1: white
        bytes.extend_from_slice(&[0b1010_0000, 0, 0, 0]);
        let image = decode(&bytes, 100).unwrap();
        assert_eq!(image.get_pixel(0, 0).0[0], 255);
        assert_eq!(image.get_pixel(1, 0).0[0], 0);
        assert_eq!(image.get_pixel(2, 0).0[0], 255);
    }

    #[test]
    fn a_16_bit_565_bitmap_with_masks_after_the_header() {
        let mut bytes = header(40, 2, 1, 16, BI_BITFIELDS, 0);
        bytes.extend_from_slice(&0xF800_u32.to_le_bytes());
        bytes.extend_from_slice(&0x07E0_u32.to_le_bytes());
        bytes.extend_from_slice(&0x001F_u32.to_le_bytes());
        bytes.extend_from_slice(&0xF800_u16.to_le_bytes()); // pure red
        bytes.extend_from_slice(&0x07E0_u16.to_le_bytes()); // pure green
        let image = decode(&bytes, 100).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [0, 255, 0, 255]);
    }

    #[test]
    fn what_cannot_be_read_is_named() {
        let mut rle = header(40, 2, 2, 8, BI_RLE8, 0);
        rle.extend_from_slice(&[0; 32]);
        assert_eq!(
            decode(&rle, 100),
            Err(DibError::UnsupportedCompression(BI_RLE8))
        );
        let mut deep = header(40, 2, 2, 48, BI_RGB, 0);
        deep.extend_from_slice(&[0; 32]);
        assert_eq!(decode(&deep, 100), Err(DibError::UnsupportedDepth(48)));
        let mut os2 = vec![0_u8; 12];
        os2[0..4].copy_from_slice(&12_u32.to_le_bytes());
        assert_eq!(decode(&os2, 100), Err(DibError::UnsupportedHeader(12)));
        assert!(matches!(decode(&[], 100), Err(DibError::Truncated(_))));
    }

    #[test]
    fn sizes_are_checked_against_the_data_and_the_limit_before_any_buffer_exists() {
        // A header that claims two billion pixels over four bytes of data.
        let claims = header(40, i32::MAX, i32::MAX, 32, BI_RGB, 0);
        assert!(matches!(
            decode(&claims, 1 << 24),
            Err(DibError::TooLarge { .. })
        ));
        // Small enough for the limit, but not the data that is there.
        let mut short = header(40, 100, 100, 24, BI_RGB, 0);
        short.extend_from_slice(&[0; 40]);
        assert_eq!(decode(&short, 1 << 24), Err(DibError::Truncated("pixels")));
        // Impossible dimensions.
        for (w, h) in [(0, 5), (-3, 5), (5, 0), (5, i32::MIN)] {
            let bytes = header(40, w, h, 24, BI_RGB, 0);
            assert_eq!(
                decode(&bytes, 100),
                Err(DibError::BadDimensions),
                "{w} x {h}"
            );
        }
        // A palette that claims more entries than the depth has.
        let bytes = header(40, 1, 1, 8, BI_RGB, 300);
        assert!(decode(&bytes, 100).is_err());
    }

    #[test]
    fn masks_that_are_not_runs_of_bits_are_refused() {
        let mut bytes = header(40, 1, 1, 32, BI_BITFIELDS, 0);
        bytes.extend_from_slice(&0x0000_0505_u32.to_le_bytes());
        bytes.extend_from_slice(&0x0000_FF00_u32.to_le_bytes());
        bytes.extend_from_slice(&0x0000_00FF_u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        assert_eq!(decode(&bytes, 100), Err(DibError::BadMasks));
        let mut zero = header(40, 1, 1, 32, BI_BITFIELDS, 0);
        zero.extend_from_slice(&[0; 12]);
        zero.extend_from_slice(&[0; 4]);
        assert_eq!(decode(&zero, 100), Err(DibError::BadMasks));
    }

    #[test]
    fn no_truncation_and_no_changed_byte_makes_it_panic() {
        let samples = [
            encode_v5(&sample(), false),
            {
                let mut b = header(40, 3, 2, 24, BI_RGB, 0);
                b.extend_from_slice(&[7; 24]);
                b
            },
            {
                let mut b = header(40, 2, 2, 8, BI_RGB, 2);
                b.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]);
                b.extend_from_slice(&[1, 0, 0, 0, 0, 1, 0, 0]);
                b
            },
        ];
        for bytes in &samples {
            for length in 0..bytes.len() {
                let _ = decode(&bytes[..length], 1 << 16);
            }
            for index in 0..bytes.len() {
                for value in [0x00_u8, 0x7f, 0xff, bytes[index] ^ 0x55] {
                    let mut changed = bytes.clone();
                    changed[index] = value;
                    let _ = decode(&changed, 1 << 16);
                }
            }
        }
    }
}
