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
