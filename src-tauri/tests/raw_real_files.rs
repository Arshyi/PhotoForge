//! Decoder checks against real camera RAW files.
//!
//! Real camera files are tens of megabytes, so they are not committed. These
//! tests look for a directory named by `PHOTOFORGE_RAW_FIXTURES` and do nothing
//! when it is absent, which is the normal case: **`cargo test` stays entirely
//! offline and never downloads anything.**
//!
//! To run them, fetch the CC0 samples once with
//! `scripts/fetch-raw-fixtures.ps1` and point the variable at the directory.
//! See `docs/raw-development.md`.

use photoforge_lib::color::DevelopmentParameters;
use photoforge_lib::raw::develop::{develop_sensor, RenderScale};
use photoforge_lib::raw::dng;
use std::path::PathBuf;

/// The fixture directory, or `None` when the tests should be skipped.
fn fixtures() -> Option<PathBuf> {
    let value = std::env::var("PHOTOFORGE_RAW_FIXTURES").ok()?;
    let path = PathBuf::from(value);
    path.is_dir().then_some(path)
}

fn read(name: &str) -> Option<Vec<u8>> {
    let path = fixtures()?.join(name);
    std::fs::read(path).ok()
}

/// A checksum over the sensor plane, so two encodings of one photograph can be
/// compared exactly.
fn plane_checksum(samples: &[u16]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for value in samples {
        hash ^= u64::from(*value);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
    }
    hash
}

const LOSSLESS: &str = "5G4A9394-compressed-lossless.DNG";
const UNCOMPRESSED: &str = "5G4A9394-uncompressed.DNG";
const LOSSY: &str = "5G4A9394-compressed-lossy.DNG";

/// The strongest check available: the same photograph, written by Adobe's DNG
/// Converter both uncompressed and with lossless JPEG, must decode to
/// bit-identical sensor data. One file exercises the packed-sample reader and
/// the other the entropy decoder, and they can only agree if both are right.
#[test]
fn the_lossless_and_uncompressed_encodings_of_one_photograph_decode_identically() {
    let (Some(lossless), Some(uncompressed)) = (read(LOSSLESS), read(UNCOMPRESSED)) else {
        eprintln!("skipped: set PHOTOFORGE_RAW_FIXTURES to run real-file checks");
        return;
    };
    let a = dng::decode(&lossless).expect("lossless DNG decodes");
    let b = dng::decode(&uncompressed).expect("uncompressed DNG decodes");

    assert_eq!((a.width, a.height), (b.width, b.height));
    assert_eq!(
        plane_checksum(&a.data),
        plane_checksum(&b.data),
        "the two encodings of the same photograph produced different sensor data"
    );
    assert_eq!(a.data, b.data, "a sample differs between the two encodings");
    assert_eq!(a.black_level, b.black_level);
    assert_eq!(a.white_level, b.white_level);
    assert_eq!(a.cfa, b.cfa);
    assert_eq!(a.as_shot_neutral, b.as_shot_neutral);
    assert_eq!(a.metadata, b.metadata);
}

/// The values below are what this Canon file actually contains, read out of
/// the file rather than assumed. They pin the tag reading against a real
/// camera's choices — a white level of 15000 rather than a power of two, and a
/// black level that differs on one CFA position.
#[test]
fn a_canon_file_reports_its_own_sensor_description() {
    let Some(bytes) = read(LOSSLESS) else {
        eprintln!("skipped: set PHOTOFORGE_RAW_FIXTURES to run real-file checks");
        return;
    };
    let sensor = dng::decode(&bytes).expect("decodes");

    assert_eq!((sensor.width, sensor.height), (5920, 3950));
    assert_eq!(sensor.bits_per_sample, 16);
    assert_eq!(sensor.cfa, dng::CfaPattern::RGGB);
    // Canon records a white level well below full scale, so a decoder that
    // assumed 65535 would develop this file far too dark.
    assert_eq!(sensor.white_level, 15000.0);
    // Three positions sit at 2047 and one at 2048; averaging them would leave a
    // colour cast in the shadows.
    assert_eq!(sensor.black_level, [2047.0, 2047.0, 2047.0, 2048.0]);
    assert_eq!(sensor.active_area, (122, 80, 5796, 3870));
    assert_eq!(sensor.default_crop, Some((18, 16, 5760, 3840)));
    assert_eq!(sensor.orientation, 1);

    assert_eq!(sensor.metadata.manufacturer.as_deref(), Some("Canon"));
    assert_eq!(
        sensor.metadata.model.as_deref(),
        Some("Canon EOS 5D Mark III")
    );
    assert_eq!(
        sensor.metadata.lens.as_deref(),
        Some("EF70-200mm f/2.8L IS II USM")
    );
    assert_eq!(sensor.metadata.iso, Some(200));
    assert_eq!(sensor.metadata.aperture, Some(2.8));
    assert_eq!(sensor.metadata.focal_length_mm, Some(70.0));
    assert_eq!(
        sensor.metadata.capture_time.as_deref(),
        Some("2017:01:05 13:52:55")
    );
    sensor.metadata.validate().expect("metadata is well formed");

    let neutral = sensor.as_shot_neutral.expect("as-shot white balance");
    assert!((neutral[0] - 0.606_276).abs() < 1e-5);
    assert!((neutral[1] - 1.0).abs() < 1e-6);
    assert!((neutral[2] - 0.461_885).abs() < 1e-5);
    assert!(sensor.camera_to_srgb.is_some(), "no colour matrix was read");
}

/// A photograph exposed to the right has samples above the nominal white level.
/// They must survive decoding and development, because recovering them is the
/// reason to develop from RAW at all.
#[test]
fn a_real_photograph_keeps_its_highlight_headroom() {
    let Some(bytes) = read(LOSSLESS) else {
        eprintln!("skipped: set PHOTOFORGE_RAW_FIXTURES to run real-file checks");
        return;
    };
    let sensor = dng::decode(&bytes).expect("decodes");
    let maximum = sensor.data.iter().copied().max().unwrap_or(0);
    assert!(
        f32::from(maximum) > sensor.white_level,
        "this file has no highlight headroom to test with"
    );
    // Samples below the black level are read noise and are real too.
    let minimum = sensor.data.iter().copied().min().unwrap_or(u16::MAX);
    assert!(f32::from(minimum) < sensor.black_level[0]);

    let developed = develop_sensor(
        &sensor,
        &DevelopmentParameters::default(),
        RenderScale::Full,
    )
    .expect("develops");
    let above: usize = developed
        .image
        .pixels()
        .iter()
        .filter(|pixel| pixel.green > 1.0)
        .count();
    assert!(
        above > 0,
        "development clipped every highlight above the white level"
    );

    // One stop down must bring them back, which an 8-bit path could not do.
    let darker = DevelopmentParameters {
        exposure_ev: -1.0,
        ..DevelopmentParameters::default()
    };
    let recovered = develop_sensor(&sensor, &darker, RenderScale::Full).expect("develops");
    let still_above: usize = recovered
        .image
        .pixels()
        .iter()
        .filter(|pixel| pixel.green > 1.0)
        .count();
    assert!(
        still_above < above,
        "reducing exposure recovered nothing: {still_above} of {above}"
    );
}

/// Lossy DNG stores three demosaiced samples per pixel rather than a CFA, so it
/// is a different thing entirely. It must be refused by name, not decoded into
/// nonsense.
#[test]
fn a_lossy_dng_is_refused_with_a_reason_rather_than_misread() {
    let Some(bytes) = read(LOSSY) else {
        eprintln!("skipped: set PHOTOFORGE_RAW_FIXTURES to run real-file checks");
        return;
    };
    let error = dng::decode(&bytes).expect_err("a lossy DNG must not decode as CFA data");
    let message = error.to_string();
    assert!(
        message.contains("not supported"),
        "the refusal did not explain itself: {message}"
    );
}

/// The preview path must agree with the full render on a real photograph, or
/// the user is editing something other than what they will export.
#[test]
fn a_preview_of_a_real_photograph_matches_its_full_render() {
    let Some(bytes) = read(LOSSLESS) else {
        eprintln!("skipped: set PHOTOFORGE_RAW_FIXTURES to run real-file checks");
        return;
    };
    let sensor = dng::decode(&bytes).expect("decodes");
    let parameters = DevelopmentParameters::default();
    let preview = develop_sensor(&sensor, &parameters, RenderScale::Preview).expect("preview");
    let full = develop_sensor(&sensor, &parameters, RenderScale::Full).expect("full");

    assert!(preview.image.width() < full.image.width());
    assert_eq!(preview.multipliers, full.multipliers);
    assert_eq!(preview.color_managed, full.color_managed);
    assert_eq!(preview.source_dimensions, full.source_dimensions);

    let mean = |image: &photoforge_lib::color::FloatImage| -> f64 {
        let pixels = image.pixels();
        pixels
            .iter()
            .map(|pixel| f64::from(pixel.green))
            .sum::<f64>()
            / pixels.len() as f64
    };
    let (preview_mean, full_mean) = (mean(&preview.image), mean(&full.image));
    assert!(
        (preview_mean - full_mean).abs() < 0.02,
        "the preview is not the same picture: {preview_mean:.4} vs {full_mean:.4}"
    );
}

/// Decoding must never modify the file it read.
#[test]
fn decoding_leaves_the_source_file_untouched() {
    let Some(directory) = fixtures() else {
        eprintln!("skipped: set PHOTOFORGE_RAW_FIXTURES to run real-file checks");
        return;
    };
    let path = directory.join(LOSSLESS);
    let Ok(before) = std::fs::metadata(&path) else {
        return;
    };
    let bytes = std::fs::read(&path).expect("reads");
    let digest_before = plane_checksum(
        &bytes
            .iter()
            .take(4096)
            .map(|byte| u16::from(*byte))
            .collect::<Vec<u16>>(),
    );
    let _ = dng::decode(&bytes).expect("decodes");
    let after = std::fs::metadata(&path).expect("still exists");
    assert_eq!(before.len(), after.len());
    let bytes_after = std::fs::read(&path).expect("reads");
    let digest_after = plane_checksum(
        &bytes_after
            .iter()
            .take(4096)
            .map(|byte| u16::from(*byte))
            .collect::<Vec<u16>>(),
    );
    assert_eq!(digest_before, digest_after);
}

/// What a real converter's file says about itself, through the metadata reader. The values are
/// what the file contains, read out of it by a separate program (see
/// `C:\PFBuild\p15\tools\tiff_dump.py` in the working notes) and not assumed: a Canon 5D Mark III
/// frame written by Adobe DNG Converter, with its serial numbers in the Exif directory, an
/// artist, an XMP packet that names the body's serial number, no place and no IPTC.
///
/// When `PHOTOFORGE_METADATA_DUMP` names a directory the blocks are written there, so another
/// program can read the same bytes and be compared.
#[test]
fn a_real_converters_file_gives_its_exif_and_xmp() {
    use photoforge_lib::metadata::{exif, left_behind, prepare, xmp, MetadataPolicy, OutputFacts};
    let mut found_any = false;
    for name in [UNCOMPRESSED, LOSSLESS, LOSSY] {
        let Some(bytes) = read(name) else {
            continue;
        };
        found_any = true;
        let found = photoforge_lib::metadata::tiff::read(&bytes).expect("a TIFF");
        let block = found.exif.as_deref().expect("an EXIF block was built");
        let summary = exif::summarize(block).expect("the block is walkable");
        assert_eq!(summary.make.as_deref(), Some("Canon"), "{name}");
        assert_eq!(summary.model.as_deref(), Some("Canon EOS 5D Mark III"));
        assert_eq!(summary.lens.as_deref(), Some("EF70-200mm f/2.8L IS II USM"));
        assert_eq!(summary.taken.as_deref(), Some("2017:01:05 13:52:55"));
        assert_eq!(summary.orientation, 1);
        assert!(!summary.has_location);
        assert!(!summary.has_thumbnail);
        assert!(summary.has_identifiers, "body and lens serial numbers");
        // Adobe's converter keeps the camera's maker note in DNGPrivateData, which is left.
        assert_eq!(found.left_behind, left_behind::MAKER_NOTE, "{name}");
        assert!(found.summary().has_maker_note);
        assert!(found.iptc.is_none());
        let packet = found.xmp.as_deref().expect("an XMP packet");
        assert_eq!(packet.len(), 6238, "{name}");
        assert!(xmp::inspect(packet).identity, "aux:SerialNumber");
        // The block is small: descriptive tags only, nothing of the sensor.
        assert!(block.len() < 2048, "{} bytes", block.len());

        // And the policies, on the real thing.
        let facts = OutputFacts {
            width: 1000,
            height: 667,
            wide_gamut_profile: false,
        };
        let (private, report) = prepare(&found, MetadataPolicy::RemovePrivate, facts);
        assert!(private.xmp.is_none(), "names the body's serial number");
        let cleaned = exif::summarize(private.exif.as_deref().unwrap()).unwrap();
        assert!(!cleaned.has_identifiers);
        assert_eq!(cleaned.model.as_deref(), Some("Canon EOS 5D Mark III"));
        assert!(report
            .notes
            .iter()
            .any(|note| note.detail.contains("serial number")));
        let (kept, _) = prepare(&found, MetadataPolicy::KeepAll, facts);
        assert_eq!(kept.xmp.as_deref().map(<[u8]>::len), Some(6238));

        if let Ok(directory) = std::env::var("PHOTOFORGE_METADATA_DUMP") {
            let directory = PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let stem = name.trim_end_matches(".DNG");
            std::fs::write(directory.join(format!("{stem}.exif.bin")), block).unwrap();
            std::fs::write(directory.join(format!("{stem}.xmp.xml")), packet).unwrap();
        }
    }
    if !found_any {
        eprintln!("skipped: set PHOTOFORGE_RAW_FIXTURES to run real-file checks");
    }
}

/// A real photograph, with its orientation tag changed to each of the eight values, is turned
/// the way that value says: the development of the changed file is the unchanged file's
/// development turned by the same rule, and it has the shape that rule gives.
///
/// The Canon sample is upright (orientation 1), so the tag is rewritten in a copy; nothing else
/// of the file is touched. Preview scale keeps it quick; the turn does not depend on the scale.
#[test]
fn a_real_photograph_is_turned_by_each_orientation_value() {
    use photoforge_lib::orientation;
    use photoforge_lib::raw::develop::{develop_bytes, develop_bytes_picture};

    let Some(original) = read(LOSSLESS) else {
        eprintln!("skipped: set PHOTOFORGE_RAW_FIXTURES to run real-file checks");
        return;
    };
    let with_orientation = |value: u16| -> Vec<u8> {
        let mut bytes = original.clone();
        assert_eq!(&bytes[..2], b"II", "a little-endian file");
        let directory = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        let count = u16::from_le_bytes([bytes[directory], bytes[directory + 1]]) as usize;
        for index in 0..count {
            let at = directory + 2 + index * 12;
            if u16::from_le_bytes([bytes[at], bytes[at + 1]]) == 274 {
                bytes[at + 8..at + 10].copy_from_slice(&value.to_le_bytes());
                return bytes;
            }
        }
        panic!("the first directory has no orientation tag");
    };
    let parameters = DevelopmentParameters::default();
    let upright = develop_bytes(&original, &parameters, RenderScale::Preview).unwrap();
    let (width, height) = upright.image.dimensions();
    let pixels = upright.image.pixels().to_vec();
    // With PHOTOFORGE_UPRIGHT_DUMP set, every picture is left as raw little-endian f32 RGBA
    // (`<name>.f32` and `<name>.json` with the size), for a reader that is not this program.
    let dump = std::env::var_os("PHOTOFORGE_UPRIGHT_DUMP").map(PathBuf::from);
    let leave = |name: String, image: &photoforge_lib::color::FloatImage| {
        let Some(directory) = &dump else {
            return;
        };
        std::fs::create_dir_all(directory).unwrap();
        let mut raw = Vec::new();
        for pixel in image.pixels() {
            for value in [pixel.red, pixel.green, pixel.blue, pixel.alpha] {
                raw.extend_from_slice(&value.to_le_bytes());
            }
        }
        std::fs::write(directory.join(format!("{name}.f32")), raw).unwrap();
        let (w, h) = image.dimensions();
        std::fs::write(
            directory.join(format!("{name}.json")),
            format!("{{\"width\": {w}, \"height\": {h}}}"),
        )
        .unwrap();
    };
    leave("orientation-1".to_string(), &upright.image);
    for value in 2..=8_u16 {
        let turned =
            develop_bytes_picture(&with_orientation(value), &parameters, RenderScale::Preview)
                .unwrap();
        let want = orientation::oriented_size(value as u8, width, height);
        assert_eq!(turned.image.dimensions(), want, "orientation {value}");
        let by_rule = orientation::turn(pixels.clone(), value as u8, width, height).unwrap();
        assert!(
            turned.image.pixels() == by_rule.as_slice(),
            "orientation {value}: a pixel differs from the turn of the unchanged file"
        );
        assert_eq!(turned.source_dimensions, upright.source_dimensions);
        leave(format!("orientation-{value}"), &turned.image);
    }
}

/// The Canon sample declares an active area of 5796 × 3870 at (122, 80) on its 5920 × 3950 sensor
/// and a default crop of 5760 × 3840 at (18, 16) inside it, which is what the camera is
/// specified to produce: the picture is the rectangle (140, 96, 5760, 3840). The numbers were
/// read out of the file by a separate parser (`tools/tiff_dump.py` in the working notes) and
/// agree with the camera's published image size.
///
/// Developed whole, the picture is that rectangle, and a window of it is exactly what developing
/// that window of the sensor on its own gives.
#[test]
fn a_real_photograph_is_cut_to_the_picture_its_camera_declares() {
    use photoforge_lib::raw::develop::{
        develop_region, develop_sensor, develop_sensor_picture, RenderScale,
    };
    use photoforge_lib::source::Rect;

    let Some(bytes) = read(LOSSLESS) else {
        eprintln!("skipped: set PHOTOFORGE_RAW_FIXTURES to run real-file checks");
        return;
    };
    let sensor = dng::decode(&bytes).expect("decodes");
    assert_eq!((sensor.width, sensor.height), (5920, 3950));
    assert_eq!(sensor.active_area, (122, 80, 5796, 3870));
    assert_eq!(sensor.default_crop, Some((18, 16, 5760, 3840)));
    let picture = Rect {
        x: 140,
        y: 96,
        width: 5760,
        height: 3840,
    };
    assert_eq!(sensor.picture_rect(), Some(picture));

    let parameters = DevelopmentParameters::default();
    // A preview is cut in proportion.
    let whole = develop_sensor(&sensor, &parameters, RenderScale::Preview).unwrap();
    let shown = develop_sensor_picture(&sensor, &parameters, RenderScale::Preview).unwrap();
    let (preview_width, preview_height) = whole.image.dimensions();
    let (cut_width, cut_height) = shown.image.dimensions();
    let ratio = |cut: u32, whole: u32, of: f64| (f64::from(cut) - f64::from(whole) * of).abs();
    assert!(
        ratio(cut_width, preview_width, 5760.0 / 5920.0) <= 1.0,
        "{cut_width}"
    );
    assert!(
        ratio(cut_height, preview_height, 3840.0 / 3950.0) <= 1.0,
        "{cut_height}"
    );

    // Whole and at full scale: 5760 × 3840, and a window of it is the window developed alone.
    let full = develop_sensor_picture(&sensor, &parameters, RenderScale::Full).unwrap();
    assert_eq!(full.image.dimensions(), (5760, 3840));
    let window = Rect {
        x: 140 + 1000,
        y: 96 + 700,
        width: 48,
        height: 48,
    };
    let alone = develop_region(&bytes, window, &parameters).unwrap();
    for row in 0..48_u32 {
        for column in 0..48_u32 {
            assert_eq!(
                full.image.get(1000 + column, 700 + row),
                alone.image.get(column, row),
                "({column}, {row})"
            );
        }
    }
}

/// A region chosen on a real photograph's upright picture is made of exactly the photosites that
/// turn into it, and is that window of the picture: for each of the eight orientation values (the
/// tag rewritten in a copy), the rectangle chosen is mapped to the sensor, developed on its own
/// and turned, and every one of its pixels is the picture's own pixel at the place the turn
/// carried it from.
///
/// One full development stands for all eight (the turn of it is checked against Pillow in
/// `a_real_photograph_is_turned_by_each_orientation_value`); each region is developed from its own
/// copy of the file, so nothing of the whole development is reused for the windows.
#[test]
fn a_region_chosen_on_a_real_photographs_upright_picture_is_that_window_of_it() {
    use photoforge_lib::orientation;
    use photoforge_lib::raw::develop::{develop_region, develop_sensor_picture};
    use photoforge_lib::source::Rect;

    let Some(original) = read(LOSSLESS) else {
        eprintln!("skipped: set PHOTOFORGE_RAW_FIXTURES to run real-file checks");
        return;
    };
    let with_orientation = |value: u16| -> Vec<u8> {
        let mut bytes = original.clone();
        let directory = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        let count = u16::from_le_bytes([bytes[directory], bytes[directory + 1]]) as usize;
        for index in 0..count {
            let at = directory + 2 + index * 12;
            if u16::from_le_bytes([bytes[at], bytes[at + 1]]) == 274 {
                bytes[at + 8..at + 10].copy_from_slice(&value.to_le_bytes());
                return bytes;
            }
        }
        panic!("the first directory has no orientation tag");
    };

    let parameters = DevelopmentParameters::default();
    let sensor = dng::decode(&original).expect("decodes");
    let picture = develop_sensor_picture(&sensor, &parameters, RenderScale::Full).unwrap();
    assert_eq!(picture.image.dimensions(), (5760, 3840));
    drop(sensor);

    for value in 1..=8_u16 {
        let bytes = with_orientation(value);
        let geometry = dng::inspect_geometry(&bytes).expect("the header reads");
        assert_eq!(u16::from(geometry.orientation), value);
        assert_eq!(
            geometry.rect,
            Rect {
                x: 140,
                y: 96,
                width: 5760,
                height: 3840
            }
        );
        let (presented_width, presented_height) = geometry.presented_size();
        assert_eq!(
            (presented_width, presented_height),
            orientation::oriented_size(value as u8, 5760, 3840)
        );

        // A window well inside the picture at an odd offset, and one against its far corner.
        for chosen in [
            Rect {
                x: 1001,
                y: 703,
                width: 48,
                height: 40,
            },
            Rect {
                x: presented_width - 33,
                y: presented_height - 21,
                width: 33,
                height: 21,
            },
        ] {
            let rect = geometry.to_sensor(&chosen).expect("on the picture");
            let region = develop_region(&bytes, rect, &parameters)
                .unwrap()
                .turned_upright(value)
                .unwrap();
            assert_eq!(
                region.image.dimensions(),
                (chosen.width, chosen.height),
                "orientation {value}"
            );
            // Every photosite of the sensor rectangle lands inside the chosen window, on the
            // pixel that holds it, and that pixel is the picture's own.
            let (local_x, local_y) = (rect.x - 140, rect.y - 96);
            for row in 0..rect.height {
                for column in 0..rect.width {
                    let (px, py) = (local_x + column, local_y + row);
                    let (dx, dy) = orientation::place(value as u8, 5760, 3840, px, py);
                    assert!(
                        dx >= chosen.x
                            && dx < chosen.x + chosen.width
                            && dy >= chosen.y
                            && dy < chosen.y + chosen.height,
                        "orientation {value}: ({px}, {py}) lands outside the window"
                    );
                    assert_eq!(
                        region.image.get(dx - chosen.x, dy - chosen.y),
                        picture.image.get(px, py),
                        "orientation {value}: ({px}, {py})"
                    );
                }
            }
        }
    }
}
