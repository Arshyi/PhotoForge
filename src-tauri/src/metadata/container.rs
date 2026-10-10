//! Putting metadata into a finished JPEG, PNG or WebP, and nothing else about the file.
//!
//! The `image` encoders can write an EXIF block and an ICC profile, but not XMP or IPTC, and not
//! one code path for all three. The encoded bytes are therefore taken as they are and the
//! metadata is inserted as the container's own segments or chunks. The pixels are not touched:
//! the tests decode the file again and compare.
//!
//! What cannot be stored is returned in [`Placed`] as `false`, never dropped quietly: a JPEG
//! segment holds 64 KiB, so a larger EXIF block or XMP packet is reported as not stored, and PNG
//! has no standard place for IPTC.

/// What is to be inserted.
#[derive(Debug, Clone, Copy, Default)]
pub struct Embedded<'a> {
    /// A TIFF block, without the "Exif\0\0" prefix.
    pub exif: Option<&'a [u8]>,
    pub xmp: Option<&'a [u8]>,
    /// An IPTC-IIM stream.
    pub iptc: Option<&'a [u8]>,
}

/// What was stored.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Placed {
    pub exif: bool,
    pub xmp: bool,
    pub iptc: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    WebP,
}

const XMP_JPEG_NAMESPACE: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const XMP_PNG_KEYWORD: &[u8] = b"XML:com.adobe.xmp";

/// Inserts `parts` into `encoded`. `Err` means the bytes were not a file of that format.
pub fn embed(
    format: Format,
    encoded: Vec<u8>,
    parts: &Embedded<'_>,
) -> Result<(Vec<u8>, Placed), &'static str> {
    if parts.exif.is_none() && parts.xmp.is_none() && parts.iptc.is_none() {
        return Ok((encoded, Placed::default()));
    }
    match format {
        Format::Jpeg => jpeg(encoded, parts),
        Format::Png => png(encoded, parts),
        Format::WebP => webp(encoded, parts),
    }
}

fn jpeg(encoded: Vec<u8>, parts: &Embedded<'_>) -> Result<(Vec<u8>, Placed), &'static str> {
    if encoded.get(..2) != Some(&[0xFF, 0xD8]) {
        return Err("not a JPEG");
    }
    // After the start of image and the run of application segments the encoder wrote (JFIF, an
    // ICC profile), before the tables.
    let mut at = 2;
    while at + 4 <= encoded.len() && encoded[at] == 0xFF {
        let marker = encoded[at + 1];
        if !(0xE0..=0xEF).contains(&marker) && marker != 0xFE {
            break;
        }
        let length = usize::from(u16::from_be_bytes([encoded[at + 2], encoded[at + 3]]));
        if length < 2 || at + 2 + length > encoded.len() {
            return Err("a JPEG segment runs past the end of the file");
        }
        at += 2 + length;
    }
    let mut placed = Placed::default();
    let mut inserted = Vec::new();
    let mut segment = |marker: u8, payload: Vec<u8>| -> bool {
        if payload.len() + 2 > usize::from(u16::MAX) {
            return false;
        }
        inserted.extend_from_slice(&[0xFF, marker]);
        inserted.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        inserted.extend_from_slice(&payload);
        true
    };
    if let Some(exif) = parts.exif {
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend_from_slice(exif);
        placed.exif = segment(0xE1, payload);
    }
    if let Some(xmp) = parts.xmp {
        let mut payload = XMP_JPEG_NAMESPACE.to_vec();
        payload.extend_from_slice(xmp);
        placed.xmp = segment(0xE1, payload);
    }
    if let Some(iptc) = parts.iptc {
        // A Photoshop image resource block: "8BIM", resource 0x0404 (IPTC-NAA), an empty name,
        // the size, the data, padded to an even length.
        let mut payload = b"Photoshop 3.0\0".to_vec();
        payload.extend_from_slice(b"8BIM");
        payload.extend_from_slice(&0x0404_u16.to_be_bytes());
        payload.extend_from_slice(&[0, 0]);
        payload.extend_from_slice(&(iptc.len() as u32).to_be_bytes());
        payload.extend_from_slice(iptc);
        if iptc.len() % 2 == 1 {
            payload.push(0);
        }
        placed.iptc = segment(0xED, payload);
    }
    let mut out = Vec::with_capacity(encoded.len() + inserted.len());
    out.extend_from_slice(&encoded[..at]);
    out.extend_from_slice(&inserted);
    out.extend_from_slice(&encoded[at..]);
    Ok((out, placed))
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(kind);
    hasher.update(data);
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&hasher.finalize().to_be_bytes());
}

fn png(encoded: Vec<u8>, parts: &Embedded<'_>) -> Result<(Vec<u8>, Placed), &'static str> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if encoded.get(..8) != Some(&SIGNATURE) {
        return Err("not a PNG");
    }
    // Before the first IDAT, so that every reader meets it before the image data.
    let mut at = 8;
    loop {
        let length = u32::from_be_bytes(
            encoded
                .get(at..at + 4)
                .ok_or("a PNG chunk runs past the end of the file")?
                .try_into()
                .map_err(|_| "a PNG chunk runs past the end of the file")?,
        ) as usize;
        let kind = encoded
            .get(at + 4..at + 8)
            .ok_or("a PNG chunk runs past the end of the file")?;
        if kind == b"IDAT" {
            break;
        }
        if kind == b"IEND" {
            return Err("a PNG with no image data");
        }
        at = at
            .checked_add(12)
            .and_then(|value| value.checked_add(length))
            .filter(|value| *value <= encoded.len())
            .ok_or("a PNG chunk runs past the end of the file")?;
    }
    let mut inserted = Vec::new();
    let mut placed = Placed::default();
    if let Some(exif) = parts.exif {
        png_chunk(&mut inserted, b"eXIf", exif);
        placed.exif = true;
    }
    if let Some(xmp) = parts.xmp {
        // iTXt: keyword, NUL, compression flag, method, language tag and translated keyword (both
        // empty, each NUL-terminated), then the text uncompressed, as XMP requires.
        let mut data = XMP_PNG_KEYWORD.to_vec();
        data.extend_from_slice(&[0, 0, 0, 0, 0]);
        data.extend_from_slice(xmp);
        png_chunk(&mut inserted, b"iTXt", &data);
        placed.xmp = true;
    }
    // IPTC has no chunk of its own in PNG; XMP is where a PNG keeps it.
    let mut out = Vec::with_capacity(encoded.len() + inserted.len());
    out.extend_from_slice(&encoded[..at]);
    out.extend_from_slice(&inserted);
    out.extend_from_slice(&encoded[at..]);
    Ok((out, placed))
}

struct Chunk<'a> {
    kind: [u8; 4],
    data: &'a [u8],
}

fn riff_chunks(webp: &[u8]) -> Result<Vec<Chunk<'_>>, &'static str> {
    if webp.get(..4) != Some(b"RIFF") || webp.get(8..12) != Some(b"WEBP") {
        return Err("not a WebP");
    }
    let declared = u32::from_le_bytes(webp[4..8].try_into().map_err(|_| "not a WebP")?) as usize;
    let end = declared
        .checked_add(8)
        .filter(|end| *end <= webp.len())
        .ok_or("a WebP whose size is larger than the file")?;
    let mut chunks = Vec::new();
    let mut at = 12;
    while at + 8 <= end {
        let kind: [u8; 4] = webp[at..at + 4].try_into().map_err(|_| "not a WebP")?;
        let length =
            u32::from_le_bytes(webp[at + 4..at + 8].try_into().map_err(|_| "not a WebP")?) as usize;
        let data_end = at
            .checked_add(8)
            .and_then(|value| value.checked_add(length))
            .filter(|value| *value <= end)
            .ok_or("a WebP chunk runs past the end of the file")?;
        chunks.push(Chunk {
            kind,
            data: &webp[at + 8..data_end],
        });
        at = data_end + (length & 1);
    }
    Ok(chunks)
}

fn write_riff_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(kind);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

/// The canvas size and whether there is alpha, read from the image chunk of a simple WebP.
fn webp_canvas(chunk: &Chunk<'_>) -> Option<(u32, u32, bool)> {
    match &chunk.kind {
        b"VP8L" => {
            let d = chunk.data;
            if d.len() < 5 || d[0] != 0x2F {
                return None;
            }
            let bits = u32::from_le_bytes([d[1], d[2], d[3], d[4]]);
            Some((
                (bits & 0x3FFF) + 1,
                ((bits >> 14) & 0x3FFF) + 1,
                bits >> 28 & 1 == 1,
            ))
        }
        b"VP8 " => {
            let d = chunk.data;
            if d.len() < 10 || d[3..6] != [0x9D, 0x01, 0x2A] {
                return None;
            }
            let width = u32::from(u16::from_le_bytes([d[6], d[7]]) & 0x3FFF);
            let height = u32::from(u16::from_le_bytes([d[8], d[9]]) & 0x3FFF);
            Some((width, height, false))
        }
        _ => None,
    }
}

fn webp(encoded: Vec<u8>, parts: &Embedded<'_>) -> Result<(Vec<u8>, Placed), &'static str> {
    let chunks = riff_chunks(&encoded)?;
    let first = chunks.first().ok_or("a WebP with no chunks")?;
    let mut body = Vec::new();
    let mut placed = Placed::default();
    let has = |wanted: &[u8; 4]| chunks.iter().any(|chunk| &chunk.kind == wanted);
    if &first.kind == b"VP8X" {
        if first.data.len() != 10 {
            return Err("a WebP header of the wrong size");
        }
        let mut header = first.data.to_vec();
        if parts.exif.is_some() && !has(b"EXIF") {
            header[0] |= 0x08;
            placed.exif = true;
        }
        if parts.xmp.is_some() && !has(b"XMP ") {
            header[0] |= 0x04;
            placed.xmp = true;
        }
        write_riff_chunk(&mut body, b"VP8X", &header);
        for chunk in chunks.iter().skip(1) {
            write_riff_chunk(&mut body, &chunk.kind, chunk.data);
        }
    } else {
        let (width, height, alpha) = webp_canvas(first).ok_or("a WebP of a kind not handled")?;
        let mut header = [0_u8; 10];
        header[0] = u8::from(alpha) << 4;
        if parts.exif.is_some() {
            header[0] |= 0x08;
            placed.exif = true;
        }
        if parts.xmp.is_some() {
            header[0] |= 0x04;
            placed.xmp = true;
        }
        header[4..7].copy_from_slice(&(width - 1).to_le_bytes()[..3]);
        header[7..10].copy_from_slice(&(height - 1).to_le_bytes()[..3]);
        write_riff_chunk(&mut body, b"VP8X", &header);
        for chunk in &chunks {
            write_riff_chunk(&mut body, &chunk.kind, chunk.data);
        }
    }
    if placed.exif {
        write_riff_chunk(&mut body, b"EXIF", parts.exif.unwrap_or_default());
    }
    if placed.xmp {
        write_riff_chunk(&mut body, b"XMP ", parts.xmp.unwrap_or_default());
    }
    // IPTC has no chunk in WebP; XMP is where it goes.
    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    Ok((out, placed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::jpeg::{JpegDecoder, JpegEncoder};
    use image::codecs::png::{PngDecoder, PngEncoder};
    use image::codecs::webp::{WebPDecoder, WebPEncoder};
    use image::{ExtendedColorType, ImageDecoder, ImageEncoder, RgbaImage};
    use std::io::Cursor;

    fn picture() -> RgbaImage {
        RgbaImage::from_fn(9, 7, |x, y| {
            image::Rgba([(x * 25) as u8, (y * 30) as u8, (x * y) as u8, 255])
        })
    }

    fn jpeg_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        let rgb = image::DynamicImage::ImageRgba8(picture()).to_rgb8();
        JpegEncoder::new_with_quality(&mut bytes, 90)
            .encode(rgb.as_raw(), 9, 7, ExtendedColorType::Rgb8)
            .unwrap();
        bytes
    }

    fn png_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(picture().as_raw(), 9, 7, ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }

    fn webp_bytes() -> Vec<u8> {
        let mut bytes = Vec::new();
        WebPEncoder::new_lossless(&mut bytes)
            .write_image(picture().as_raw(), 9, 7, ExtendedColorType::Rgba8)
            .unwrap();
        bytes
    }

    const EXIF: &[u8] = b"II*\0\x08\0\0\0\0\0\0\0\0\0";
    const XMP: &[u8] =
        b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><dc:title>Harbour</dc:title></x:xmpmeta>";
    const IPTC: &[u8] = &[0x1C, 2, 25, 0, 7, b'h', b'a', b'r', b'b', b'o', b'u', b'r'];

    fn parts() -> Embedded<'static> {
        Embedded {
            exif: Some(EXIF),
            xmp: Some(XMP),
            iptc: Some(IPTC),
        }
    }

    #[test]
    fn a_jpeg_carries_exif_xmp_and_iptc_and_decodes_the_same_picture() {
        let plain = jpeg_bytes();
        let (with, placed) = embed(Format::Jpeg, plain.clone(), &parts()).unwrap();
        assert_eq!(
            placed,
            Placed {
                exif: true,
                xmp: true,
                iptc: true
            }
        );
        let mut decoder = JpegDecoder::new(Cursor::new(&with)).unwrap();
        assert_eq!(decoder.exif_metadata().unwrap().as_deref(), Some(EXIF));
        assert_eq!(decoder.xmp_metadata().unwrap().as_deref(), Some(XMP));
        // The decoder hands back the Photoshop resource block; the stream is inside it.
        let block = decoder.iptc_metadata().unwrap().unwrap();
        assert_eq!(
            crate::metadata::iptc::extract(&block).as_deref(),
            Some(IPTC)
        );
        let a = image::load_from_memory(&plain).unwrap().to_rgba8();
        let b = image::load_from_memory(&with).unwrap().to_rgba8();
        assert_eq!(a, b);
    }

    #[test]
    fn a_jpeg_segment_that_does_not_fit_is_reported_not_stored() {
        let big = vec![b'x'; 70_000];
        let (with, placed) = embed(
            Format::Jpeg,
            jpeg_bytes(),
            &Embedded {
                exif: Some(EXIF),
                xmp: Some(&big),
                iptc: None,
            },
        )
        .unwrap();
        assert!(placed.exif && !placed.xmp);
        assert!(image::load_from_memory(&with).is_ok());
    }

    #[test]
    fn a_png_carries_exif_and_xmp_and_says_iptc_has_no_place() {
        let plain = png_bytes();
        let (with, placed) = embed(Format::Png, plain.clone(), &parts()).unwrap();
        assert_eq!(
            placed,
            Placed {
                exif: true,
                xmp: true,
                iptc: false
            }
        );
        let mut decoder = PngDecoder::new(Cursor::new(&with)).unwrap();
        assert_eq!(decoder.exif_metadata().unwrap().as_deref(), Some(EXIF));
        assert_eq!(decoder.xmp_metadata().unwrap().as_deref(), Some(XMP));
        let a = image::load_from_memory(&plain).unwrap().to_rgba8();
        let b = image::load_from_memory(&with).unwrap().to_rgba8();
        assert_eq!(a, b);
    }

    #[test]
    fn a_webp_carries_exif_and_xmp_and_decodes_the_same_picture() {
        let plain = webp_bytes();
        let (with, placed) = embed(Format::WebP, plain.clone(), &parts()).unwrap();
        assert_eq!(
            placed,
            Placed {
                exif: true,
                xmp: true,
                iptc: false
            }
        );
        let mut decoder = WebPDecoder::new(Cursor::new(&with)).unwrap();
        assert_eq!(decoder.exif_metadata().unwrap().as_deref(), Some(EXIF));
        assert_eq!(decoder.xmp_metadata().unwrap().as_deref(), Some(XMP));
        assert_eq!(decoder.dimensions(), (9, 7));
        let a = image::load_from_memory(&plain).unwrap().to_rgba8();
        let b = image::load_from_memory(&with).unwrap().to_rgba8();
        assert_eq!(a, b);
    }

    #[test]
    fn nothing_to_embed_returns_the_file_untouched() {
        let plain = png_bytes();
        let (same, placed) = embed(Format::Png, plain.clone(), &Embedded::default()).unwrap();
        assert_eq!(same, plain);
        assert_eq!(placed, Placed::default());
    }

    #[test]
    fn bytes_of_the_wrong_kind_are_refused() {
        assert!(embed(Format::Jpeg, png_bytes(), &parts()).is_err());
        assert!(embed(Format::Png, jpeg_bytes(), &parts()).is_err());
        assert!(embed(Format::WebP, jpeg_bytes(), &parts()).is_err());
    }

    #[test]
    fn truncated_files_are_refused_or_handled_and_never_panic() {
        for bytes in [jpeg_bytes(), png_bytes(), webp_bytes()] {
            let format = if bytes.starts_with(&[0xFF, 0xD8]) {
                Format::Jpeg
            } else if bytes.starts_with(&[0x89]) {
                Format::Png
            } else {
                Format::WebP
            };
            for len in 0..bytes.len().min(300) {
                let _ = embed(format, bytes[..len].to_vec(), &parts());
            }
        }
    }
}
