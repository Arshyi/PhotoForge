use super::*;
use crate::color::srgb_decode;
use crate::metadata::exif::fixture::{Fixture, MAKER_NOTE, SERIAL};
use crate::metadata::Prepared;
use crate::pixel::DocumentPrecision;
use crate::psd::ImportLimits;
use crate::tiff::import::import_from;
use crate::tiff::structure::TiffFile;
use std::io::Cursor;

fn limits() -> ImportLimits {
    ImportLimits {
        store_bytes: 1 << 30,
        entry_bytes: 1 << 30,
        job_bytes: 1 << 30,
    }
}

/// Writes a `width × height` picture whose samples are `value(x, y, channel)`.
fn encode(
    width: u32,
    height: u32,
    bits: u16,
    alpha: bool,
    compression: Compression,
    metadata: Option<&Prepared>,
    value: impl Fn(u32, u32, usize) -> u16,
) -> (Vec<u8>, Written) {
    let samples = if alpha { 4 } else { 3 };
    let mut out = Cursor::new(Vec::new());
    let spec = Spec {
        width,
        height,
        bits,
        alpha,
        compression,
        icc: None,
        metadata,
    };
    let written = write(&mut out, &spec, |y, row| {
        for x in 0..width {
            for channel in 0..samples {
                let v = value(x, y, channel);
                if bits == 8 {
                    row.push(v as u8);
                } else {
                    row.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
        Ok(())
    })
    .unwrap();
    (out.into_inner(), written)
}

#[test]
fn what_is_written_reads_back_exactly_at_each_depth_with_and_without_alpha() {
    for compression in [Compression::None, Compression::Deflate] {
        for alpha in [false, true] {
            // 8 bits.
            let value = |x: u32, y: u32, c: usize| ((x * 7 + y * 13 + c as u32 * 29) % 256) as u16;
            let (bytes, written) = encode(41, 33, 8, alpha, compression, None, value);
            assert_eq!(written.bytes as usize, bytes.len());
            let imported = import_from(Cursor::new(bytes), limits()).unwrap();
            assert_eq!(
                imported.project.document.precision,
                DocumentPrecision::LegacySrgb8
            );
            let image = &imported.project.pixels[0].1;
            for y in 0..33 {
                for x in 0..41 {
                    let want = [
                        value(x, y, 0) as u8,
                        value(x, y, 1) as u8,
                        value(x, y, 2) as u8,
                        if alpha { value(x, y, 3) as u8 } else { 255 },
                    ];
                    assert_eq!(
                        image.get_pixel(x, y).0,
                        want,
                        "{compression:?} {alpha} ({x}, {y})"
                    );
                }
            }

            // 16 bits.
            let value16 =
                |x: u32, y: u32, c: usize| ((x * 1031 + y * 4099 + c as u32 * 7919) % 65536) as u16;
            let (bytes, _) = encode(41, 33, 16, alpha, compression, None, value16);
            let imported = import_from(Cursor::new(bytes), limits()).unwrap();
            assert_eq!(
                imported.project.document.precision,
                DocumentPrecision::LinearSrgbF32
            );
            let pixels = imported.project.linear_pixels[0].1.pixels();
            for y in 0..33_u32 {
                for x in 0..41_u32 {
                    let p = pixels[(y * 41 + x) as usize];
                    let want = srgb_decode(f32::from(value16(x, y, 0)) / 65535.0);
                    assert!(
                        (p.red - want).abs() < 1e-6,
                        "{compression:?} {alpha} ({x}, {y})"
                    );
                    if alpha {
                        let a = f32::from(value16(x, y, 3)) / 65535.0;
                        assert!((p.alpha - a).abs() < 1e-6);
                    } else {
                        assert_eq!(p.alpha, 1.0);
                    }
                }
            }
        }
    }
}

#[test]
fn the_directory_says_what_the_file_is() {
    let (bytes, _) = encode(30, 40, 16, true, Compression::Deflate, None, |x, y, c| {
        (x + y + c as u32) as u16
    });
    let mut tiff = TiffFile::open(Cursor::new(bytes)).unwrap();
    let directories = tiff.directories().unwrap();
    assert_eq!(directories.len(), 1);
    let directory = &directories[0];
    let tags: Vec<u16> = directory.entries.keys().copied().collect();
    assert!(tags.windows(2).all(|pair| pair[0] < pair[1]));
    let first = |tiff: &mut TiffFile<Cursor<Vec<u8>>>, tag| tiff.first_u32(directory, tag).unwrap();
    assert_eq!(first(&mut tiff, 256), Some(30));
    assert_eq!(first(&mut tiff, 257), Some(40));
    assert_eq!(first(&mut tiff, 259), Some(8), "Deflate");
    assert_eq!(first(&mut tiff, 262), Some(2), "RGB");
    assert_eq!(first(&mut tiff, 277), Some(4));
    assert_eq!(first(&mut tiff, 284), Some(1), "chunky");
    assert_eq!(first(&mut tiff, 317), Some(2), "horizontal differencing");
    assert_eq!(first(&mut tiff, 338), Some(2), "unassociated alpha");
    let bits = tiff.u32s(directory.get(258).unwrap()).unwrap();
    assert_eq!(bits, vec![16, 16, 16, 16]);

    // Uncompressed carries no predictor.
    let (bytes, _) = encode(5, 5, 8, false, Compression::None, None, |_, _, _| 0);
    let mut tiff = TiffFile::open(Cursor::new(bytes)).unwrap();
    let directory = tiff.directories().unwrap().remove(0);
    assert!(!directory.has(317));
    assert!(!directory.has(338));
}

#[test]
fn a_wide_picture_is_in_several_strips_of_about_a_quarter_megabyte() {
    let (bytes, _) = encode(1000, 300, 8, false, Compression::None, None, |x, y, c| {
        ((x + y + c as u32) % 256) as u16
    });
    let mut tiff = TiffFile::open(Cursor::new(bytes)).unwrap();
    let directory = tiff.directories().unwrap().remove(0);
    let rows = tiff.first_u32(&directory, 278).unwrap().unwrap();
    // 3000 bytes a row: 262,144 / 3000 = 87 rows.
    assert_eq!(rows, 87);
    let offsets = tiff.u32s(directory.get(273).unwrap()).unwrap();
    assert_eq!(offsets.len(), 300_usize.div_ceil(87));
}

fn camera_metadata() -> Prepared {
    Prepared {
        exif: Some(Fixture::build()),
        xmp: Some(b"<x:xmpmeta><dc:title>Pier</dc:title></x:xmpmeta>".to_vec()),
        iptc: Some(vec![0x1C, 2, 120, 0, 4, b'P', b'i', b'e', b'r']),
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn the_cameras_metadata_goes_where_a_tiff_keeps_it() {
    let prepared = camera_metadata();
    let (bytes, written) = encode(
        4,
        3,
        8,
        false,
        Compression::Deflate,
        Some(&prepared),
        |x, y, _| (x * 10 + y) as u16,
    );
    assert!(written.placed.exif && written.placed.xmp && written.placed.iptc);
    assert!(written.maker_note_dropped, "the fixture has a maker note");
    // The maker note and the thumbnail are not in the file; the serial number (kept under this
    // policy) and the camera are.
    assert!(!contains(&bytes, MAKER_NOTE));
    assert!(!contains(&bytes, b"THUMBNAIL"));
    assert!(contains(&bytes, SERIAL));

    let imported = import_from(Cursor::new(bytes), limits()).unwrap();
    let metadata = imported.project.source_metadata.as_ref().unwrap();
    let summary = metadata.summary();
    assert_eq!(summary.make.as_deref(), Some("Acme"));
    assert_eq!(summary.model.as_deref(), Some("Cam-One"));
    assert_eq!(summary.lens.as_deref(), Some("Lens 35mm f/2"));
    assert_eq!(summary.taken.as_deref(), Some("2026:10:10 14:03:09"));
    assert!(
        summary.has_location,
        "the place is carried under this policy"
    );
    assert!(summary.has_identifiers);
    assert!(!summary.has_maker_note);
    assert!(!summary.has_thumbnail);
    assert_eq!(
        metadata.xmp.as_deref().map(Vec::as_slice),
        prepared.xmp.as_deref()
    );
    assert_eq!(
        metadata.iptc.as_deref().map(Vec::as_slice),
        prepared.iptc.as_deref()
    );
    // The fixture's orientation (6) is written, and so this reader turned the picture.
    assert_eq!(metadata.source_orientation, 6);
    assert_eq!(imported.project.document.canvas_width, 3);
    assert_eq!(imported.project.document.canvas_height, 4);
}

#[test]
fn a_big_endian_block_is_written_with_every_value_in_the_files_order() {
    // IFD0 at 8 (make "Acme", orientation 1, Exif pointer) and the Exif directory at 50 (exposure
    // 1/250, ISO 400, three shorts), then the values: written big-endian, as a phone writes it.
    fn entry(tag: u16, kind: u16, count: u32, value: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&tag.to_be_bytes());
        bytes.extend_from_slice(&kind.to_be_bytes());
        bytes.extend_from_slice(&count.to_be_bytes());
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes
    }
    let mut block = Vec::new();
    block.extend_from_slice(b"MM\0*");
    block.extend_from_slice(&8_u32.to_be_bytes());
    block.extend_from_slice(&3_u16.to_be_bytes());
    block.extend_from_slice(&entry(0x010F, 2, 5, 92));
    block.extend_from_slice(&entry(0x0112, 3, 1, 1 << 16));
    block.extend_from_slice(&entry(0x8769, 4, 1, 50));
    block.extend_from_slice(&0_u32.to_be_bytes());
    assert_eq!(block.len(), 50);
    block.extend_from_slice(&3_u16.to_be_bytes());
    block.extend_from_slice(&entry(0x829A, 5, 1, 98));
    block.extend_from_slice(&entry(0x8827, 3, 1, 400 << 16));
    block.extend_from_slice(&entry(0xA432, 3, 3, 106));
    block.extend_from_slice(&0_u32.to_be_bytes());
    assert_eq!(block.len(), 92);
    block.extend_from_slice(b"Acme\0\0");
    block.extend_from_slice(&1_u32.to_be_bytes());
    block.extend_from_slice(&250_u32.to_be_bytes());
    for value in [1_u16, 2, 3] {
        block.extend_from_slice(&value.to_be_bytes());
    }
    let prepared = Prepared {
        exif: Some(block),
        ..Prepared::default()
    };
    let (bytes, written) = encode(
        2,
        2,
        8,
        false,
        Compression::None,
        Some(&prepared),
        |_, _, _| 7,
    );
    assert!(written.placed.exif);

    let imported = import_from(Cursor::new(bytes), limits()).unwrap();
    let metadata = imported.project.source_metadata.unwrap();
    assert_eq!(metadata.summary().make.as_deref(), Some("Acme"));
    let split = crate::metadata::tiff::split(metadata.exif.as_ref().unwrap()).unwrap();
    let field = |tag: u16| {
        split
            .exif
            .iter()
            .find(|field| field.tag == tag)
            .and_then(|field| field.value.clone())
            .unwrap()
    };
    assert_eq!(field(0x8827), 400_u16.to_le_bytes().to_vec());
    assert_eq!(
        field(0x829A),
        [1_u32.to_le_bytes(), 250_u32.to_le_bytes()].concat()
    );
    assert_eq!(
        field(0xA432),
        [1_u16, 2, 3]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<u8>>()
    );
}

#[test]
fn an_icc_profile_is_embedded_byte_for_byte() {
    let profile = crate::color_management::RgbColorSpace::DisplayP3
        .icc_bytes()
        .unwrap();
    let mut out = Cursor::new(Vec::new());
    let spec = Spec {
        width: 3,
        height: 2,
        bits: 8,
        alpha: false,
        compression: Compression::Deflate,
        icc: Some(&profile),
        metadata: None,
    };
    write(&mut out, &spec, |_, row| {
        row.extend_from_slice(&[255, 0, 0, 0, 255, 0, 0, 0, 255]);
        Ok(())
    })
    .unwrap();
    let bytes = out.into_inner();
    let mut tiff = TiffFile::open(Cursor::new(bytes.clone())).unwrap();
    let directory = tiff.directories().unwrap().remove(0);
    let stored = tiff.value_bytes(directory.get(34675).unwrap()).unwrap();
    assert_eq!(stored, profile);
    // And read back, the P3 red is more than sRGB can hold.
    let imported = import_from(Cursor::new(bytes), limits()).unwrap();
    assert_eq!(
        imported.project.document.precision,
        DocumentPrecision::LinearSrgbF32
    );
    assert!(imported.project.linear_pixels[0].1.pixels()[0].red > 1.1);
}

#[test]
fn a_picture_the_size_of_nothing_is_not_written() {
    let mut out = Cursor::new(Vec::new());
    for (width, height, bits) in [(0, 4, 8), (4, 0, 8), (4, 4, 12)] {
        let spec = Spec {
            width,
            height,
            bits,
            alpha: false,
            compression: Compression::None,
            icc: None,
            metadata: None,
        };
        assert!(write(&mut out, &spec, |_, _| Ok(())).is_err());
    }
    // A row of the wrong length is a defect in the caller, not a ragged picture.
    let spec = Spec {
        width: 4,
        height: 4,
        bits: 8,
        alpha: false,
        compression: Compression::None,
        icc: None,
        metadata: None,
    };
    let mut out = Cursor::new(Vec::new());
    assert!(write(&mut out, &spec, |_, row| {
        row.extend_from_slice(&[0; 5]);
        Ok(())
    })
    .is_err());
}
