use super::*;
use crate::color::srgb_decode;
use crate::color_management::RgbColorSpace;
use crate::tiff::testing::{ramp, Tiff};
use std::io::Cursor;

fn limits() -> ImportLimits {
    ImportLimits {
        store_bytes: 1 << 30,
        entry_bytes: 1 << 30,
        job_bytes: 1 << 30,
    }
}

fn read(bytes: Vec<u8>) -> Result<ImportedTiff, AppError> {
    import_from(Cursor::new(bytes), limits())
}

fn refusal(bytes: Vec<u8>) -> String {
    match read(bytes) {
        Ok(_) => panic!("the file was read"),
        Err(error) => error.to_string(),
    }
}

fn eight(imported: &ImportedTiff) -> &image::RgbaImage {
    &imported.project.pixels[0].1
}

fn float(imported: &ImportedTiff) -> &FloatImage {
    &imported.project.linear_pixels[0].1
}

fn high(imported: &ImportedTiff) -> bool {
    imported.project.document.precision == DocumentPrecision::LinearSrgbF32
}

fn notes(imported: &ImportedTiff) -> String {
    imported
        .report
        .entries
        .iter()
        .map(|entry| entry.detail.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Rewrites the first value of tag `tag` in the first directory of a little-endian file.
fn patch(bytes: &mut [u8], tag: u16, value: u32) {
    let directory = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let count = u16::from_le_bytes([bytes[directory], bytes[directory + 1]]) as usize;
    for index in 0..count {
        let at = directory + 2 + index * 12;
        if u16::from_le_bytes([bytes[at], bytes[at + 1]]) == tag {
            let kind = u16::from_le_bytes([bytes[at + 2], bytes[at + 3]]);
            if kind == 3 {
                bytes[at + 8..at + 10].copy_from_slice(&(value as u16).to_le_bytes());
            } else {
                bytes[at + 8..at + 12].copy_from_slice(&value.to_le_bytes());
            }
            return;
        }
    }
    panic!("no tag {tag}");
}

#[test]
fn an_eight_bit_rgb_picture_stays_eight_bit_and_exact_in_either_byte_order() {
    for big in [false, true] {
        let data = ramp(13, 9, 3, 255.0);
        let mut tiff = Tiff::rgb8(13, 9, data.clone());
        tiff.big = big;
        let imported = read(tiff.build()).unwrap();
        assert!(!high(&imported));
        assert_eq!(imported.project.document.canvas_width, 13);
        let image = eight(&imported);
        for y in 0..9_u32 {
            for x in 0..13_u32 {
                let at = (y * 13 + x) as usize * 3;
                let want = [data[at] as u8, data[at + 1] as u8, data[at + 2] as u8, 255];
                assert_eq!(image.get_pixel(x, y).0, want, "({x}, {y}) big={big}");
            }
        }
        assert_eq!(imported.report.format, "TIFF");
        assert_eq!(imported.report.bit_depth, 8);
        assert_eq!(imported.report.layers_imported, 1);
        assert!(notes(&imported).contains("8-bit sRGB"));
    }
}

#[test]
fn every_layout_compression_and_predictor_reads_back_exactly() {
    let mut files = 0;
    for bits in [8_u16, 16] {
        for compression in [1_u16, 5, 8, 32946, 32773] {
            for predictor in [1_u16, 2] {
                for planar in [false, true] {
                    for tile in [None, Some((16_u32, 16_u32))] {
                        for big in [false, true] {
                            let max = if bits == 8 { 255.0 } else { 65535.0 };
                            let data = ramp(37, 21, 3, max);
                            let mut tiff = Tiff::rgb8(37, 21, data.clone());
                            tiff.bits = bits;
                            tiff.compression = compression;
                            tiff.predictor = predictor;
                            tiff.planar = planar;
                            tiff.tile = tile;
                            tiff.big = big;
                            tiff.rows_per_strip = 5;
                            let label = format!(
                                "{bits}-bit compression {compression} predictor {predictor} \
                                 planar {planar} tile {tile:?} big {big}"
                            );
                            let imported = read(tiff.build())
                                .unwrap_or_else(|error| panic!("{label}: {error}"));
                            files += 1;
                            for y in 0..21_u32 {
                                for x in 0..37_u32 {
                                    let at = (y * 37 + x) as usize * 3;
                                    if bits == 8 {
                                        let want = [
                                            data[at] as u8,
                                            data[at + 1] as u8,
                                            data[at + 2] as u8,
                                            255,
                                        ];
                                        assert_eq!(
                                            eight(&imported).get_pixel(x, y).0,
                                            want,
                                            "{label} ({x}, {y})"
                                        );
                                    } else {
                                        let p = float(&imported).pixels()[(y * 37 + x) as usize];
                                        for (got, v) in [
                                            (p.red, data[at]),
                                            (p.green, data[at + 1]),
                                            (p.blue, data[at + 2]),
                                        ] {
                                            let want = srgb_decode((v / 65535.0) as f32);
                                            assert!(
                                                (got - want).abs() < 1e-6,
                                                "{label} ({x}, {y}): {got} vs {want}"
                                            );
                                        }
                                        assert_eq!(p.alpha, 1.0);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    assert_eq!(files, 160);
}

#[test]
fn sixteen_bit_without_a_profile_is_read_as_srgb_in_a_high_precision_document() {
    let mut tiff = Tiff::rgb8(2, 1, vec![0.0, 32768.0, 65535.0, 65535.0, 0.0, 16384.0]);
    tiff.bits = 16;
    let imported = read(tiff.build()).unwrap();
    assert!(high(&imported));
    assert!(imported.project.pixels.is_empty());
    let p = float(&imported).pixels();
    assert!((p[0].green - srgb_decode(32768.0 / 65535.0)).abs() < 1e-6);
    assert_eq!(p[0].red, 0.0);
    assert!((p[1].red - 1.0).abs() < 1e-6);
    assert!(notes(&imported).contains("read as sRGB"));
}

#[test]
fn floating_point_is_linear_light_and_values_beyond_one_are_kept() {
    for (bits, big, predictor, compression) in [
        (32_u16, false, 1_u16, 1_u16),
        (32, true, 1, 1),
        (32, false, 3, 8),
        (32, true, 3, 8),
        (16, false, 1, 1),
        (16, true, 3, 5),
        (64, false, 1, 1),
        (64, false, 3, 8),
    ] {
        let values: Vec<f64> = (0..5 * 4 * 3)
            .map(|index| f64::from(index % 40) / 16.0 - 1.0)
            .collect();
        let mut tiff = Tiff::rgb8(5, 4, values.clone());
        tiff.bits = bits;
        tiff.float = true;
        tiff.big = big;
        tiff.predictor = predictor;
        tiff.compression = compression;
        tiff.rows_per_strip = 3;
        let label = format!("{bits}-bit float big {big} predictor {predictor}");
        let imported = read(tiff.build()).unwrap_or_else(|error| panic!("{label}: {error}"));
        assert!(high(&imported), "{label}");
        let p = float(&imported).pixels();
        for (index, pixel) in p.iter().enumerate() {
            assert_eq!(pixel.red, values[index * 3] as f32, "{label} pixel {index}");
            assert_eq!(pixel.green, values[index * 3 + 1] as f32, "{label}");
            assert_eq!(pixel.blue, values[index * 3 + 2] as f32, "{label}");
        }
        assert!(
            p.iter().any(|pixel| pixel.red > 1.0 || pixel.blue > 1.0),
            "values above one were clipped"
        );
        assert!(
            p.iter().any(|pixel| pixel.red < 0.0),
            "negatives were clipped"
        );
        assert!(notes(&imported).contains("linear light"), "{label}");
    }
}

#[test]
fn grey_white_is_zero_bilevel_and_palette_pictures_read() {
    // Grey, 8-bit.
    let mut grey = Tiff::rgb8(3, 1, vec![0.0, 128.0, 255.0]);
    grey.samples = 1;
    grey.photometric = 1;
    let imported = read(grey.build()).unwrap();
    assert_eq!(eight(&imported).get_pixel(1, 0).0, [128, 128, 128, 255]);
    assert_eq!(imported.report.color_mode, "Grayscale");

    // White is zero: inverted, and said.
    let mut inverted = Tiff::rgb8(3, 1, vec![0.0, 100.0, 255.0]);
    inverted.samples = 1;
    inverted.photometric = 0;
    let imported = read(inverted.build()).unwrap();
    assert_eq!(eight(&imported).get_pixel(0, 0).0, [255, 255, 255, 255]);
    assert_eq!(eight(&imported).get_pixel(1, 0).0, [155, 155, 155, 255]);
    assert!(notes(&imported).contains("inverted"));

    // 1, 2 and 4 bits a sample, rows padded to the byte.
    for bits in [1_u16, 2, 4] {
        let max = f64::from((1_u32 << bits) - 1);
        let width = 11_u32;
        let data: Vec<f64> = (0..width * 3)
            .map(|i| f64::from(i % (max as u32 + 1)))
            .collect();
        let mut tiff = Tiff::rgb8(width, 3, data.clone());
        tiff.bits = bits;
        tiff.samples = 1;
        tiff.photometric = 1;
        let imported = read(tiff.build()).unwrap();
        for y in 0..3_u32 {
            for x in 0..width {
                let v = data[(y * width + x) as usize] / max;
                let want = (v * 255.0).round() as u8;
                assert_eq!(
                    eight(&imported).get_pixel(x, y).0,
                    [want, want, want, 255],
                    "{bits}-bit ({x}, {y})"
                );
            }
        }
    }

    // A palette of four colours at 2 bits.
    let mut palette = Tiff::rgb8(4, 1, vec![0.0, 1.0, 2.0, 3.0]);
    palette.bits = 2;
    palette.samples = 1;
    palette.photometric = 3;
    // 16-bit entries, as TIFF stores them: the mid-values show the scaling, not just the ends.
    palette.palette = vec![[0, 0, 0], [65535, 0, 0], [0, 32768, 0], [0, 0, 65535]];
    let imported = read(palette.build()).unwrap();
    assert!(!high(&imported));
    assert_eq!(eight(&imported).get_pixel(1, 0).0, [255, 0, 0, 255]);
    assert_eq!(eight(&imported).get_pixel(2, 0).0, [0, 128, 0, 255]);
    assert_eq!(eight(&imported).get_pixel(3, 0).0, [0, 0, 255, 255]);
    assert!(notes(&imported).contains("palette"));
}

#[test]
fn alpha_is_straightened_kept_or_ignored_as_the_file_says() {
    // Unassociated (2): kept as it is.
    let mut straight = Tiff::rgb8(1, 1, vec![200.0, 100.0, 50.0, 128.0]);
    straight.samples = 4;
    straight.extra_samples = vec![2];
    let imported = read(straight.build()).unwrap();
    assert_eq!(eight(&imported).get_pixel(0, 0).0, [200, 100, 50, 128]);

    // Associated (1): the colour is divided by alpha.
    let mut premultiplied = Tiff::rgb8(2, 1, vec![100.0, 50.0, 25.0, 128.0, 9.0, 9.0, 9.0, 0.0]);
    premultiplied.samples = 4;
    premultiplied.extra_samples = vec![1];
    let imported = read(premultiplied.build()).unwrap();
    let p = eight(&imported).get_pixel(0, 0).0;
    assert_eq!(p[3], 128);
    // 100 * 255 / 128 = 199.2; 50 -> 99.6; 25 -> 49.8
    assert_eq!([p[0], p[1], p[2]], [199, 100, 50]);
    // Alpha zero has no colour to divide.
    assert_eq!(eight(&imported).get_pixel(1, 0).0, [0, 0, 0, 0]);

    // Unspecified (0): not alpha. The picture is opaque and the channel is said to be left out.
    let mut unspecified = Tiff::rgb8(1, 1, vec![10.0, 20.0, 30.0, 7.0]);
    unspecified.samples = 4;
    unspecified.extra_samples = vec![0];
    let imported = read(unspecified.build()).unwrap();
    assert_eq!(eight(&imported).get_pixel(0, 0).0, [10, 20, 30, 255]);
    assert_eq!(imported.report.count(ReportKind::NotImported), 1);
    assert!(notes(&imported).contains("extra channel"));

    // Grey with alpha, 16-bit: alpha is not put through the colour curve.
    let mut grey = Tiff::rgb8(1, 1, vec![65535.0, 16384.0]);
    grey.samples = 2;
    grey.photometric = 1;
    grey.bits = 16;
    grey.extra_samples = vec![2];
    let imported = read(grey.build()).unwrap();
    let p = float(&imported).pixels()[0];
    assert!((p.red - 1.0).abs() < 1e-6);
    assert!((p.alpha - 16384.0 / 65535.0).abs() < 1e-6);
}

#[test]
fn all_eight_orientations_turn_the_picture_upright() {
    // 3 wide, 2 tall: 1 2 3 / 4 5 6, each a grey of 40 * its number.
    let expected: [(u16, u32, u32, [[u8; 3]; 3]); 8] = [
        (1, 3, 2, [[1, 2, 3], [4, 5, 6], [0, 0, 0]]),
        (2, 3, 2, [[3, 2, 1], [6, 5, 4], [0, 0, 0]]),
        (3, 3, 2, [[6, 5, 4], [3, 2, 1], [0, 0, 0]]),
        (4, 3, 2, [[4, 5, 6], [1, 2, 3], [0, 0, 0]]),
        (5, 2, 3, [[1, 4, 0], [2, 5, 0], [3, 6, 0]]),
        (6, 2, 3, [[4, 1, 0], [5, 2, 0], [6, 3, 0]]),
        (7, 2, 3, [[6, 3, 0], [5, 2, 0], [4, 1, 0]]),
        (8, 2, 3, [[3, 6, 0], [2, 5, 0], [1, 4, 0]]),
    ];
    for (orientation, width, height, rows) in expected {
        let data: Vec<f64> = (1..=6).map(|n| f64::from(n * 40)).collect();
        let mut tiff = Tiff::rgb8(3, 2, data);
        tiff.samples = 1;
        tiff.photometric = 1;
        tiff.orientation = Some(orientation);
        let imported = read(tiff.build()).unwrap();
        assert_eq!(
            (
                imported.project.document.canvas_width,
                imported.project.document.canvas_height
            ),
            (width, height),
            "orientation {orientation}"
        );
        for y in 0..height {
            for x in 0..width {
                let want = rows[y as usize][x as usize] * 40;
                assert_eq!(
                    eight(&imported).get_pixel(x, y).0[0],
                    want,
                    "orientation {orientation} ({x}, {y})"
                );
            }
        }
        assert_eq!(imported.report.width, width);
        let said = notes(&imported).contains("turned upright");
        assert_eq!(said, orientation != 1, "orientation {orientation}");
    }
}

#[test]
fn a_profile_that_is_srgb_keeps_eight_bit_and_another_converts() {
    let srgb = RgbColorSpace::Srgb.icc_bytes().unwrap();
    let p3 = RgbColorSpace::DisplayP3.icc_bytes().unwrap();
    let data = vec![255.0, 0.0, 0.0, 128.0, 128.0, 128.0];

    let mut tagged = Tiff::rgb8(2, 1, data.clone());
    tagged.icc = Some(srgb);
    let imported = read(tagged.build()).unwrap();
    assert!(!high(&imported));
    assert!(notes(&imported).contains("equivalent to sRGB"));
    assert_eq!(eight(&imported).get_pixel(0, 0).0, [255, 0, 0, 255]);

    let mut wide = Tiff::rgb8(2, 1, data);
    wide.icc = Some(p3);
    let imported = read(wide.build()).unwrap();
    assert!(high(&imported));
    let p = float(&imported).pixels();
    // Display P3's red is outside sRGB: more than full red once taken to sRGB's primaries.
    assert!(p[0].red > 1.1, "{:?}", p[0]);
    // Its grey is the same grey (the transfer function is sRGB's).
    assert!(
        (p[1].red - srgb_decode(128.0 / 255.0)).abs() < 0.01,
        "{:?}",
        p[1]
    );
    assert!(notes(&imported).contains("embedded profile"));

    // A profile that is not a profile is named, and the picture is read as sRGB.
    let mut broken = Tiff::rgb8(1, 1, vec![1.0, 2.0, 3.0]);
    broken.icc = Some(vec![0xAB; 40]);
    let imported = read(broken.build()).unwrap();
    assert!(!high(&imported));
    assert!(notes(&imported).contains("could not be read"));
}

#[test]
fn what_cannot_be_read_is_refused_by_name() {
    let base = || Tiff::rgb8(4, 4, ramp(4, 4, 3, 255.0));

    let mut cmyk = base();
    cmyk.samples = 4;
    cmyk.photometric = 5;
    cmyk.data = ramp(4, 4, 4, 255.0);
    assert!(refusal(cmyk.build()).contains("CMYK"));

    let mut ycbcr = base();
    ycbcr.photometric = 6;
    assert!(refusal(ycbcr.build()).contains("YCbCr"));

    let mut mosaic = base();
    mosaic.samples = 1;
    mosaic.photometric = 32803;
    mosaic.data = ramp(4, 4, 1, 255.0);
    assert!(refusal(mosaic.build()).contains("RAW"));

    // The writer cannot compress with JPEG, so name the code in a valid file.
    let mut jpeg = base().build();
    patch(&mut jpeg, 259, 7);
    let message = refusal(jpeg);
    assert!(message.contains("JPEG"), "{message}");

    // One sample, so the depth is in the entry itself where `patch` can reach it.
    let mut one = base();
    one.samples = 1;
    one.photometric = 1;
    one.data = ramp(4, 4, 1, 255.0);
    let mut odd_depth = one.build();
    patch(&mut odd_depth, 258, 12);
    assert!(refusal(odd_depth).contains("12-bit"));

    let mut lsb = base();
    lsb.fill_order = Some(2);
    assert!(refusal(lsb.build()).contains("least significant"));

    let mut signed = base();
    signed.extra_tags.push((
        339,
        3,
        [2_u16, 2, 2].iter().flat_map(|v| v.to_le_bytes()).collect(),
    ));
    let message = refusal(signed.build());
    assert!(message.to_lowercase().contains("signed"), "{message}");

    let mut unknown = base();
    unknown.photometric = 99;
    assert!(refusal(unknown.build()).contains("99"));

    let big_tiff = b"II+\0\x08\0\x08\0\0\0\0\0\0\0\0\0".to_vec();
    assert!(refusal(big_tiff).contains("BigTIFF"));

    assert!(refusal(b"GIF89a, not a TIFF at all".to_vec()).contains("not a TIFF"));
    assert!(refusal(Vec::new()).contains("too short"));

    let mut empty = base().build();
    patch(&mut empty, 256, 0);
    assert!(refusal(empty).contains("no area"));
}

#[test]
fn other_pages_and_photoshop_layers_are_reported_not_imported() {
    let mut paged = Tiff::rgb8(4, 4, ramp(4, 4, 3, 255.0));
    paged.with_thumbnail = true;
    let imported = read(paged.build()).unwrap();
    assert_eq!(imported.project.document.canvas_width, 4);
    assert!(notes(&imported).contains("1 other image"));
    assert_eq!(imported.report.count(ReportKind::NotImported), 1);

    let mut layered = Tiff::rgb8(4, 4, ramp(4, 4, 3, 255.0));
    layered.extra_tags.push((37724, 7, vec![1, 2, 3, 4, 5, 6]));
    let imported = read(layered.build()).unwrap();
    assert!(notes(&imported).contains("Photoshop with its layers"));

    // A first directory that is itself the reduced copy is not the picture.
    let mut reduced = Tiff::rgb8(4, 4, ramp(4, 4, 3, 255.0));
    reduced.new_subfile_type = Some(1);
    assert!(refusal(reduced.build()).contains("no full-resolution picture"));
}

#[test]
fn the_cameras_metadata_is_carried_and_the_orientation_is_spent() {
    let xmp = b"<x:xmpmeta><dc:title>Pier</dc:title></x:xmpmeta>".to_vec();
    let iptc = vec![0x1C, 2, 120, 0, 4, b'P', b'i', b'e', b'r'];
    let text = |value: &str| {
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        bytes
    };
    let mut tiff = Tiff::rgb8(3, 2, ramp(3, 2, 3, 255.0));
    tiff.orientation = Some(6);
    tiff.extra_tags = vec![
        (271, 2, text("Acme")),
        (272, 2, text("Cam-One")),
        (315, 2, text("A. Photographer")),
        (700, 1, xmp.clone()),
        (33723, 7, iptc.clone()),
    ];
    let imported = read(tiff.build()).unwrap();
    let metadata = imported.project.source_metadata.as_ref().unwrap();
    assert!(metadata.orientation_applied);
    assert_eq!(metadata.source_orientation, 6);
    assert_eq!(
        metadata.xmp.as_deref().map(Vec::as_slice),
        Some(xmp.as_slice())
    );
    assert_eq!(
        metadata.iptc.as_deref().map(Vec::as_slice),
        Some(iptc.as_slice())
    );
    let summary = metadata.summary();
    assert_eq!(summary.make.as_deref(), Some("Acme"));
    assert_eq!(summary.model.as_deref(), Some("Cam-One"));
    assert!(summary.orientation_applied);

    // And a big-endian file builds a big-endian block.
    let mut be = Tiff::rgb8(3, 2, ramp(3, 2, 3, 255.0));
    be.big = true;
    be.extra_tags = vec![(271, 2, text("Acme")), (274, 3, vec![0, 3])];
    let imported = read(be.build()).unwrap();
    let block = imported
        .project
        .source_metadata
        .as_ref()
        .unwrap()
        .exif
        .as_ref()
        .unwrap();
    assert_eq!(&block[..4], b"MM\0*");
}

#[test]
fn sizes_are_admitted_before_anything_is_allocated() {
    // A small file that claims a very large picture.
    let mut huge = Tiff::rgb8(4, 4, ramp(4, 4, 3, 255.0)).build();
    patch(&mut huge, 256, 40_000);
    patch(&mut huge, 257, 40_000);
    let message = refusal(huge.clone());
    assert!(
        message.contains("memory budget")
            || message.contains("too large")
            || message.contains("larger"),
        "{message}"
    );

    // And one that is only large against a small budget.
    let medium = Tiff::rgb8(200, 200, ramp(200, 200, 3, 255.0)).build();
    let small = ImportLimits {
        store_bytes: 1 << 30,
        entry_bytes: 100_000,
        job_bytes: 1 << 30,
    };
    let error = import_from(Cursor::new(medium), small).err().unwrap();
    assert!(error.to_string().contains("memory budget"), "{error}");

    // A table of strips far longer than the file.
    let mut strips = Tiff::rgb8(4, 4, ramp(4, 4, 3, 255.0)).build();
    patch(&mut strips, 278, 1);
    let message = refusal(strips);
    assert!(
        message.contains("fewer strips") || message.contains("declares"),
        "{message}"
    );
}

#[test]
fn a_truncated_strip_is_an_error_and_not_a_band_of_zeros() {
    let mut tiff = Tiff::rgb8(32, 32, ramp(32, 32, 3, 255.0));
    tiff.compression = 8;
    tiff.rows_per_strip = 8;
    let bytes = tiff.build();
    // Cut inside the pixel data (the directory is after it, so cut and lose the directory:
    // the file is refused either way, but never read as a picture with a hole).
    for length in [9, 100, bytes.len() / 3, bytes.len() / 2, bytes.len() - 1] {
        assert!(read(bytes[..length].to_vec()).is_err(), "{length}");
    }
}

#[test]
fn truncation_and_changed_bytes_never_panic() {
    let mut tiff = Tiff::rgb8(19, 13, ramp(19, 13, 3, 65535.0));
    tiff.bits = 16;
    tiff.compression = 5;
    tiff.predictor = 2;
    tiff.tile = Some((16, 16));
    tiff.extra_tags = vec![(700, 1, b"<x/>".to_vec()), (271, 2, b"Acme\0".to_vec())];
    let bytes = tiff.build();
    for length in 0..=bytes.len() {
        let _ = read(bytes[..length].to_vec());
    }
    for at in 0..bytes.len() {
        for value in [0x00_u8, 0x7F, 0xFF] {
            let mut changed = bytes.clone();
            changed[at] = value;
            if let Ok(imported) = read(changed) {
                imported.project.document.validate().unwrap();
            }
        }
    }
}

/// Sets every element of the (external) LONG array of `tag` in the first directory.
fn patch_longs(bytes: &mut [u8], tag: u16, value: u32) {
    let directory = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let count = u16::from_le_bytes([bytes[directory], bytes[directory + 1]]) as usize;
    for index in 0..count {
        let at = directory + 2 + index * 12;
        if u16::from_le_bytes([bytes[at], bytes[at + 1]]) == tag {
            let elements =
                u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
                    as usize;
            let offset =
                u32::from_le_bytes([bytes[at + 8], bytes[at + 9], bytes[at + 10], bytes[at + 11]])
                    as usize;
            for element in 0..elements {
                bytes[offset + element * 4..offset + element * 4 + 4]
                    .copy_from_slice(&value.to_le_bytes());
            }
            return;
        }
    }
    panic!("no tag {tag}");
}

#[test]
fn strips_that_overlap_are_refused_instead_of_being_read_again_and_again() {
    let mut tiff = Tiff::rgb8(8, 64, ramp(8, 64, 3, 255.0));
    tiff.rows_per_strip = 1;
    tiff.compression = 8;
    let mut bytes = tiff.build();
    // Every one of the 64 strips claims the whole of a 64 KiB.
    patch_longs(&mut bytes, 279, 60_000);
    let message = refusal(bytes);
    assert!(
        message.contains("declare more bytes than the file holds"),
        "{message}"
    );
}

#[test]
fn a_tile_of_four_billion_by_four_billion_is_a_refusal_and_not_an_overflow() {
    let mut tiff = Tiff::rgb8(40, 40, ramp(40, 40, 3, 255.0));
    tiff.tile = Some((16, 16));
    let mut bytes = tiff.build();
    patch(&mut bytes, 322, 4_000_000_000);
    patch(&mut bytes, 323, 4_000_000_000);
    let message = refusal(bytes);
    assert!(
        message.contains("memory budget") || message.contains("fewer strips"),
        "{message}"
    );
}

#[test]
fn values_that_are_not_numbers_are_replaced_and_counted() {
    let data = vec![f64::NAN, 0.5, f64::INFINITY, f64::NEG_INFINITY, 0.25, 1.0];
    let mut tiff = Tiff::rgb8(2, 1, data);
    tiff.bits = 32;
    tiff.float = true;
    let imported = read(tiff.build()).unwrap();
    let p = float(&imported).pixels();
    assert_eq!(p[0].red, 0.0, "not-a-number became zero");
    assert_eq!(
        p[0].blue,
        f32::MAX,
        "infinity became the largest finite value"
    );
    assert_eq!(p[1].red, f32::MIN);
    assert!(p
        .iter()
        .all(|pixel| pixel.red.is_finite() && pixel.blue.is_finite()));
    assert_eq!(imported.report.count(ReportKind::Approximated), 1);
    assert!(notes(&imported).contains("3 samples held a value that is not a number"));
}

#[test]
fn a_makers_private_data_in_the_first_directory_is_said_to_be_left_behind() {
    let mut tiff = Tiff::rgb8(2, 2, ramp(2, 2, 3, 255.0));
    tiff.extra_tags = vec![
        (271, 2, b"Acme\0".to_vec()),
        (0xC634, 1, b"Adobe\0MakN-private".to_vec()),
    ];
    let imported = read(tiff.build()).unwrap();
    let metadata = imported.project.source_metadata.unwrap();
    assert_eq!(
        metadata.left_behind & crate::metadata::left_behind::MAKER_NOTE,
        crate::metadata::left_behind::MAKER_NOTE
    );
    assert!(metadata.summary().has_maker_note);
}
