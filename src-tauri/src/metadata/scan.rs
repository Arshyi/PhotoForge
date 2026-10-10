//! Finding the metadata of a file on disk without decoding a pixel.
//!
//! Used for every way a photograph is opened, whole, by region or as a reduced copy, so that
//! all three come out with the same metadata: the region and reduced paths do not go through a
//! decoder that offers it. Each container is walked by its own headers with seeks, reading only
//! the segments that matter and never more than the sizes this code carries, so the cost is a
//! few kilobytes whatever the size of the file.
//!
//! Every length is a claim and is bounded before a buffer is made for it.

use super::{exif, iptc, xmp, SourceMetadata};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAX_SEGMENTS: usize = 4096;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
const XMP_JPEG_NAMESPACE: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
const XMP_PNG_KEYWORD: &[u8] = b"XML:com.adobe.xmp";

#[derive(Default)]
struct Blocks {
    exif: Option<Vec<u8>>,
    xmp: Option<Vec<u8>>,
    iptc: Option<Vec<u8>>,
}

/// The metadata of the JPEG, PNG or WebP at `path`. `None` when it is none of those or cannot
/// be read; a file with no metadata is `Some` with nothing in it.
pub fn read(path: &Path) -> Option<SourceMetadata> {
    let mut file = File::open(path).ok()?;
    scan(&mut file)
}

pub fn scan<R: Read + Seek>(reader: &mut R) -> Option<SourceMetadata> {
    let mut head = [0_u8; 12];
    reader.read_exact(&mut head).ok()?;
    reader.seek(SeekFrom::Start(0)).ok()?;
    let blocks = if head[..2] == [0xFF, 0xD8] {
        jpeg(reader)
    } else if head[..8] == PNG_SIGNATURE {
        png(reader)
    } else if &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        webp(reader)
    } else {
        return None;
    };
    Some(SourceMetadata::from_blocks(
        blocks.exif,
        blocks.xmp,
        blocks.iptc,
    ))
}

fn take<R: Read>(reader: &mut R, length: usize) -> Option<Vec<u8>> {
    let mut bytes = vec![0_u8; length];
    reader.read_exact(&mut bytes).ok()?;
    Some(bytes)
}

fn skip<R: Seek>(reader: &mut R, length: u64) -> Option<()> {
    let length = i64::try_from(length).ok()?;
    reader.seek(SeekFrom::Current(length)).ok().map(|_| ())
}

fn jpeg<R: Read + Seek>(reader: &mut R) -> Blocks {
    let mut blocks = Blocks::default();
    if reader.seek(SeekFrom::Start(2)).is_err() {
        return blocks;
    }
    for _ in 0..MAX_SEGMENTS {
        let mut byte = [0_u8; 1];
        if reader.read_exact(&mut byte).is_err() || byte[0] != 0xFF {
            break;
        }
        // Fill bytes: any number of 0xFF before the marker.
        let marker = loop {
            if reader.read_exact(&mut byte).is_err() {
                return blocks;
            }
            if byte[0] != 0xFF {
                break byte[0];
            }
        };
        // Markers without a length: standalone, restart, and the byte-stuffed zero.
        if marker == 0x00 || marker == 0x01 || (0xD0..=0xD8).contains(&marker) {
            continue;
        }
        // Start of scan or end of image: the metadata, if any, is behind us.
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let mut length = [0_u8; 2];
        if reader.read_exact(&mut length).is_err() {
            break;
        }
        let length = usize::from(u16::from_be_bytes(length));
        if length < 2 {
            break;
        }
        let payload = length - 2;
        match marker {
            0xE1 | 0xED if payload > 0 => {
                let Some(bytes) = take(reader, payload) else {
                    break;
                };
                if marker == 0xE1 {
                    if blocks.exif.is_none() && bytes.starts_with(b"Exif\0\0") {
                        blocks.exif = Some(bytes[6..].to_vec());
                    } else if blocks.xmp.is_none() && bytes.starts_with(XMP_JPEG_NAMESPACE) {
                        blocks.xmp = Some(bytes[XMP_JPEG_NAMESPACE.len()..].to_vec());
                    }
                } else if blocks.iptc.is_none() && bytes.starts_with(b"Photoshop 3.0\0") {
                    blocks.iptc = iptc::extract(&bytes[14..]);
                }
            }
            _ => {
                if skip(reader, payload as u64).is_none() {
                    break;
                }
            }
        }
    }
    blocks
}

fn png<R: Read + Seek>(reader: &mut R) -> Blocks {
    let mut blocks = Blocks::default();
    if reader.seek(SeekFrom::Start(8)).is_err() {
        return blocks;
    }
    for _ in 0..MAX_SEGMENTS * 16 {
        let mut header = [0_u8; 8];
        if reader.read_exact(&mut header).is_err() {
            break;
        }
        let length = u64::from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]));
        let kind = [header[4], header[5], header[6], header[7]];
        match &kind {
            b"IEND" => break,
            b"eXIf" if length as usize <= exif::MAX_EXIF_BYTES && blocks.exif.is_none() => {
                let Some(bytes) = take(reader, length as usize) else {
                    break;
                };
                blocks.exif = Some(bytes);
                if skip(reader, 4).is_none() {
                    break;
                }
            }
            b"iTXt" if length as usize <= xmp::MAX_XMP_BYTES + 256 && blocks.xmp.is_none() => {
                let Some(bytes) = take(reader, length as usize) else {
                    break;
                };
                blocks.xmp = itxt_xmp(&bytes);
                if skip(reader, 4).is_none() {
                    break;
                }
            }
            _ => {
                if skip(reader, length + 4).is_none() {
                    break;
                }
            }
        }
    }
    blocks
}

/// The text of an `iTXt` chunk whose keyword is the XMP one.
fn itxt_xmp(data: &[u8]) -> Option<Vec<u8>> {
    let rest = data.strip_prefix(XMP_PNG_KEYWORD)?.strip_prefix(&[0])?;
    let (&flag, rest) = rest.split_first()?;
    let (&method, rest) = rest.split_first()?;
    // Language tag and translated keyword, each NUL-terminated.
    let after_language = rest.iter().position(|b| *b == 0)? + 1;
    let rest = rest.get(after_language..)?;
    let after_translated = rest.iter().position(|b| *b == 0)? + 1;
    let text = rest.get(after_translated..)?;
    match (flag, method) {
        (0, _) => Some(text.to_vec()),
        (1, 0) => {
            miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(text, xmp::MAX_XMP_BYTES).ok()
        }
        _ => None,
    }
}

fn webp<R: Read + Seek>(reader: &mut R) -> Blocks {
    let mut blocks = Blocks::default();
    if reader.seek(SeekFrom::Start(12)).is_err() {
        return blocks;
    }
    for _ in 0..MAX_SEGMENTS {
        let mut header = [0_u8; 8];
        if reader.read_exact(&mut header).is_err() {
            break;
        }
        let length = u64::from(u32::from_le_bytes([
            header[4], header[5], header[6], header[7],
        ]));
        let padded = length + (length & 1);
        match &header[..4] {
            b"EXIF" if length as usize <= exif::MAX_EXIF_BYTES + 6 && blocks.exif.is_none() => {
                let Some(bytes) = take(reader, length as usize) else {
                    break;
                };
                blocks.exif = Some(bytes);
                if skip(reader, padded - length).is_none() {
                    break;
                }
            }
            b"XMP " if length as usize <= xmp::MAX_XMP_BYTES && blocks.xmp.is_none() => {
                let Some(bytes) = take(reader, length as usize) else {
                    break;
                };
                blocks.xmp = Some(bytes);
                if skip(reader, padded - length).is_none() {
                    break;
                }
            }
            _ => {
                if skip(reader, padded).is_none() {
                    break;
                }
            }
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::container::{embed, Embedded, Format};
    use image::codecs::jpeg::JpegEncoder;
    use image::codecs::png::PngEncoder;
    use image::codecs::webp::WebPEncoder;
    use image::{ExtendedColorType, ImageEncoder};
    use std::io::Cursor;

    fn encoded(format: Format) -> Vec<u8> {
        let pixels: Vec<u8> = (0..9 * 7 * 4).map(|i| (i * 7) as u8).collect();
        let mut bytes = Vec::new();
        match format {
            Format::Jpeg => {
                let rgb: Vec<u8> = pixels.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
                JpegEncoder::new_with_quality(&mut bytes, 90)
                    .encode(&rgb, 9, 7, ExtendedColorType::Rgb8)
                    .unwrap();
            }
            Format::Png => PngEncoder::new(&mut bytes)
                .write_image(&pixels, 9, 7, ExtendedColorType::Rgba8)
                .unwrap(),
            Format::WebP => WebPEncoder::new_lossless(&mut bytes)
                .write_image(&pixels, 9, 7, ExtendedColorType::Rgba8)
                .unwrap(),
        }
        bytes
    }

    fn exif_block() -> Vec<u8> {
        exif::orientation_only(6)
    }

    const XMP: &[u8] = b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>";
    const IPTC: &[u8] = &[0x1C, 2, 25, 0, 3, b'a', b'b', b'c'];

    #[test]
    fn each_container_gives_back_what_was_put_into_it() {
        for format in [Format::Jpeg, Format::Png, Format::WebP] {
            let exif = exif_block();
            let (with, _) = embed(
                format,
                encoded(format),
                &Embedded {
                    exif: Some(&exif),
                    xmp: Some(XMP),
                    iptc: Some(IPTC),
                },
            )
            .unwrap();
            let found = scan(&mut Cursor::new(with)).unwrap();
            assert_eq!(found.exif.as_deref(), Some(&exif), "{format:?}");
            assert_eq!(
                found.xmp.as_deref().map(Vec::as_slice),
                Some(XMP),
                "{format:?}"
            );
            assert_eq!(found.source_orientation, 6, "{format:?}");
            if format == Format::Jpeg {
                assert_eq!(found.iptc.as_deref().map(Vec::as_slice), Some(IPTC));
            } else {
                assert!(found.iptc.is_none());
            }
        }
    }

    #[test]
    fn a_file_with_no_metadata_is_found_to_have_none() {
        for format in [Format::Jpeg, Format::Png, Format::WebP] {
            let found = scan(&mut Cursor::new(encoded(format))).unwrap();
            assert!(found.is_empty(), "{format:?}");
            assert_eq!(found.source_orientation, 1);
        }
    }

    #[test]
    fn what_is_not_an_image_is_not_scanned() {
        assert!(scan(&mut Cursor::new(b"just some text, not an image".to_vec())).is_none());
        assert!(scan(&mut Cursor::new(Vec::new())).is_none());
    }

    #[test]
    fn a_declared_length_far_beyond_the_file_costs_nothing() {
        // A PNG whose first chunk claims four gigabytes.
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(&u32::MAX.to_be_bytes());
        bytes.extend_from_slice(b"eXIf");
        let found = scan(&mut Cursor::new(bytes)).unwrap();
        assert!(found.is_empty());
        // A JPEG whose APP1 claims 65,533 bytes in a file of twenty.
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1, 0xFF, 0xFF];
        jpeg.extend_from_slice(b"Exif\0\0II*\0");
        let found = scan(&mut Cursor::new(jpeg)).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn no_truncation_makes_it_panic() {
        for format in [Format::Jpeg, Format::Png, Format::WebP] {
            let exif = exif_block();
            let (with, _) = embed(
                format,
                encoded(format),
                &Embedded {
                    exif: Some(&exif),
                    xmp: Some(XMP),
                    iptc: Some(IPTC),
                },
            )
            .unwrap();
            for len in 0..with.len() {
                let _ = scan(&mut Cursor::new(with[..len].to_vec()));
            }
        }
    }

    #[test]
    fn a_compressed_png_xmp_packet_is_inflated_within_the_limit() {
        let compressed = miniz_oxide::deflate::compress_to_vec_zlib(XMP, 6);
        let mut data = XMP_PNG_KEYWORD.to_vec();
        data.extend_from_slice(&[0, 1, 0, 0, 0]);
        data.extend_from_slice(&compressed);
        assert_eq!(itxt_xmp(&data).as_deref(), Some(XMP));
        // A bomb: a few hundred bytes that inflate past the limit are refused, not expanded.
        let bomb = miniz_oxide::deflate::compress_to_vec_zlib(&vec![b'a'; 4 << 20], 6);
        let mut data = XMP_PNG_KEYWORD.to_vec();
        data.extend_from_slice(&[0, 1, 0, 0, 0]);
        data.extend_from_slice(&bomb);
        assert!(itxt_xmp(&data).is_none());
    }
}
