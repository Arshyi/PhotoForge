//! The system clipboard, for pictures.
//!
//! **Copy** puts one picture on the clipboard in the two forms programs actually read: the
//! registered `PNG` format (lossless, straight alpha: what Photoshop, Office, browsers, GIMP and
//! Paint.NET take first) and `CF_DIBV5` (what the rest take; Windows itself synthesises `CF_DIB`
//! and `CF_BITMAP` from it for programs that ask for those). **Paste** reads the clipboard in the
//! order `PNG`, `CF_DIBV5`, `CF_DIB`.
//!
//! The clipboard is a hostile-input surface and a shared one. What is read is bounded before it
//! is decoded (bytes, then pixels), what is written replaces the clipboard whole (so a program
//! reading it never sees half a copy), and the clipboard is held open only while the bytes move.
//!
//! Windows only. On other systems every operation is an error that says so.
pub mod dib;
#[cfg(windows)]
mod win;

use image::RgbaImage;

/// Why a clipboard operation did not happen. The text is for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardError {
    /// Another program is holding the clipboard.
    Busy,
    /// There is no picture on the clipboard.
    NoImage,
    /// The picture is larger than a bound.
    TooLarge { bytes: u64, limit: u64 },
    /// What is there could not be read as a picture.
    Unreadable(String),
    /// The operating system refused.
    Windows(String),
    /// This is not Windows.
    Unavailable,
}

impl std::fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Busy => write!(
                f,
                "The clipboard is in use by another program. Try again in a moment."
            ),
            Self::NoImage => write!(f, "The clipboard does not hold a picture."),
            Self::TooLarge { bytes, limit } => write!(
                f,
                "The picture on the clipboard is {bytes} bytes, more than the {limit} PhotoForge reads."
            ),
            Self::Unreadable(why) => write!(f, "The picture on the clipboard could not be read: {why}."),
            Self::Windows(why) => write!(f, "Windows refused the clipboard: {why}"),
            Self::Unavailable => write!(f, "The clipboard is available on Windows only."),
        }
    }
}

impl std::error::Error for ClipboardError {}

/// Which form of the picture was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardFormat {
    Png,
    DibV5,
    Dib,
}

/// What a copy put on the clipboard.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Written {
    pub width: u32,
    pub height: u32,
    /// The forms written, by name.
    pub formats: Vec<&'static str>,
    /// How many bytes the PNG form took.
    pub png_bytes: usize,
}

/// A picture read from the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pasted {
    pub image: RgbaImage,
    pub format: ClipboardFormat,
}

/// Whether the `CF_DIBV5` this program writes repeats the three colour masks as a table between
/// the header and the pixels, as Windows' own does.
const DIBV5_MASK_TABLE: bool = false;

/// Encodes a picture as PNG (RGBA, 8 bits).
pub fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, ClipboardError> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    // Fast compression: a copy should not make the person wait, and what reads it decodes either.
    PngEncoder::new_with_quality(&mut bytes, CompressionType::Fast, FilterType::Adaptive)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| ClipboardError::Unreadable(error.to_string()))?;
    Ok(bytes)
}

/// Decodes a PNG to 8-bit RGBA, refusing one larger than `max_pixels` from its header, before
/// anything is allocated for it.
pub fn decode_png(bytes: &[u8], max_pixels: u64) -> Result<RgbaImage, ClipboardError> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 33 || &bytes[..8] != SIGNATURE || &bytes[12..16] != b"IHDR" {
        return Err(ClipboardError::Unreadable("it is not a PNG".into()));
    }
    let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 {
        return Err(ClipboardError::Unreadable("the PNG has no pixels".into()));
    }
    if pixels > max_pixels {
        return Err(ClipboardError::TooLarge {
            bytes: pixels.saturating_mul(4),
            limit: max_pixels.saturating_mul(4),
        });
    }
    // The decoder's own allocation limit is its default (512 MiB), which is below what the pixel
    // bound admits; it is set from the same bound instead: the picture as 8-bit RGBA, up to three
    // copies of it (a 16-bit source, the conversion, the row buffers), and a margin.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    limits.max_alloc = Some(
        max_pixels
            .saturating_mul(12)
            .saturating_add(64 * 1024 * 1024),
    );
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(bytes), image::ImageFormat::Png);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .map_err(|error| ClipboardError::Unreadable(error.to_string()))?;
    Ok(decoded.to_rgba8())
}

/// Replaces the clipboard with `image`, as `PNG` and `CF_DIBV5`.
///
/// Everything is made, and its memory allocated, before the clipboard is touched, so that running
/// out of memory costs the copy and not what was on the clipboard.
///
/// The picture is also marked as not for upload to the cloud clipboard (`CanUploadToCloudClipboard`
/// of 0), which Windows honours when the person has clipboard sync on: a picture from an editor
/// that sends nothing anywhere is not sent by the system either. It stays in the local clipboard
/// history, which is the person's own.
#[cfg(windows)]
pub fn write_image(image: &RgbaImage) -> Result<Written, ClipboardError> {
    let png = encode_png(image)?;
    let dib = dib::encode_v5(image, DIBV5_MASK_TABLE);
    let png_staged = win::stage(&png)?;
    let dib_staged = win::stage(&dib)?;
    let no_cloud = win::stage(&0_u32.to_le_bytes())?;
    let clipboard = win::OpenClipboardGuard::open()?;
    clipboard.clear()?;
    clipboard.set(win::png_format(), png_staged)?;
    clipboard.set(win::dibv5_format(), dib_staged)?;
    clipboard.set(win::cloud_upload_format(), no_cloud)?;
    Ok(Written {
        width: image.width(),
        height: image.height(),
        formats: vec!["PNG", "CF_DIBV5"],
        png_bytes: png.len(),
    })
}

/// Replaces the clipboard with `image` as `CF_DIBV5` alone, with or without a mask table: for the
/// tests that ask Windows' own readers how they take it, which a clipboard with a `PNG` form
/// would hide.
#[cfg(windows)]
#[doc(hidden)]
pub fn write_dibv5_only(image: &RgbaImage, mask_table: bool) -> Result<(), ClipboardError> {
    let dib = win::stage(&dib::encode_v5(image, mask_table))?;
    let clipboard = win::OpenClipboardGuard::open()?;
    clipboard.clear()?;
    clipboard.set(win::dibv5_format(), dib)
}

/// Reads the picture on the clipboard: `PNG` first, then `CF_DIBV5`, then `CF_DIB`.
#[cfg(windows)]
pub fn read_image(max_pixels: u64) -> Result<Pasted, ClipboardError> {
    let mut last_problem: Option<ClipboardError> = None;
    // Each form is tried in turn; a form that is there but unreadable falls through to the next,
    // and the first problem is what is reported if none works.
    for (format, kind) in [
        (win::png_format(), ClipboardFormat::Png),
        (win::dibv5_format(), ClipboardFormat::DibV5),
        (win::dib_format(), ClipboardFormat::Dib),
    ] {
        // The clipboard is open only while the bytes are copied out of it. Decoding a large
        // picture takes a while, and a program that wants the clipboard (or a copy of this
        // one's) should not wait for that.
        let bytes = {
            let clipboard = win::OpenClipboardGuard::open()?;
            clipboard.get(format)?
        };
        let Some(bytes) = bytes else { continue };
        let decoded = match kind {
            ClipboardFormat::Png => decode_png(&bytes, max_pixels),
            _ => dib::decode(&bytes, max_pixels).map_err(|error| match error {
                dib::DibError::TooLarge { pixels, limit } => ClipboardError::TooLarge {
                    bytes: pixels.saturating_mul(4),
                    limit: limit.saturating_mul(4),
                },
                other => ClipboardError::Unreadable(other.to_string()),
            }),
        };
        match decoded {
            Ok(image) => {
                return Ok(Pasted {
                    image,
                    format: kind,
                })
            }
            Err(error) => {
                last_problem.get_or_insert(error);
            }
        }
    }
    Err(last_problem.unwrap_or(ClipboardError::NoImage))
}

/// Whether the clipboard holds something that reads as a picture (in any of the three forms).
#[cfg(windows)]
pub fn has_image() -> bool {
    let png_format = win::png_format();
    match win::OpenClipboardGuard::open() {
        Ok(clipboard) => {
            clipboard.has(png_format)
                || clipboard.has(win::dibv5_format())
                || clipboard.has(win::dib_format())
        }
        Err(_) => false,
    }
}

#[cfg(not(windows))]
pub fn write_image(_image: &RgbaImage) -> Result<Written, ClipboardError> {
    Err(ClipboardError::Unavailable)
}

#[cfg(not(windows))]
pub fn read_image(_max_pixels: u64) -> Result<Pasted, ClipboardError> {
    Err(ClipboardError::Unavailable)
}

#[cfg(not(windows))]
pub fn has_image() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn sample() -> RgbaImage {
        let mut image = RgbaImage::new(5, 3);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = Rgba([(x * 50) as u8, (y * 80) as u8, 200, (255 - x * 40) as u8]);
        }
        image
    }

    #[test]
    fn a_png_round_trips_with_its_alpha() {
        let image = sample();
        let bytes = encode_png(&image).unwrap();
        assert_eq!(decode_png(&bytes, 100).unwrap(), image);
    }

    #[test]
    fn a_png_larger_than_the_limit_is_refused_from_its_header() {
        let bytes = encode_png(&sample()).unwrap();
        assert!(matches!(
            decode_png(&bytes, 14),
            Err(ClipboardError::TooLarge { .. })
        ));
        // A header that claims a huge picture over a few bytes of data.
        let mut forged = bytes.clone();
        forged[16..20].copy_from_slice(&0x7FFF_FFFF_u32.to_be_bytes());
        forged[20..24].copy_from_slice(&0x7FFF_FFFF_u32.to_be_bytes());
        assert!(matches!(
            decode_png(&forged, 1 << 24),
            Err(ClipboardError::TooLarge { .. })
        ));
        // The largest the header can say: the byte counts do not overflow on the way to the
        // refusal (a debug build would have panicked).
        forged[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
        forged[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(
            decode_png(&forged, 1 << 24),
            Err(ClipboardError::TooLarge { .. })
        ));
    }

    #[test]
    fn what_is_not_a_png_is_unreadable_not_a_panic() {
        assert!(matches!(
            decode_png(b"not a png at all, not at all, not even close", 100),
            Err(ClipboardError::Unreadable(_))
        ));
        assert!(decode_png(&[], 100).is_err());
        let bytes = encode_png(&sample()).unwrap();
        for length in 0..bytes.len() {
            let _ = decode_png(&bytes[..length], 100);
        }
    }

    #[test]
    fn messages_are_for_a_person() {
        assert_eq!(
            ClipboardError::NoImage.to_string(),
            "The clipboard does not hold a picture."
        );
        assert!(ClipboardError::Busy.to_string().contains("another program"));
    }
}
