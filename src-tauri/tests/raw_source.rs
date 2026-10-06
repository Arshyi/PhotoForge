//! A camera RAW as an oversized source: how it is priced, previewed and kept.
//!
//! These cover the seams around `develop_region` — which `raw_region.rs` proves
//! bit-exact — so that the claim "a RAW region is real region decoding" is checked
//! at the places it could quietly stop being true: the planner's prices, the
//! bounded preview, and the record a layer keeps so the region can be rebuilt.
use photoforge_lib::color::{DevelopmentParameters, WhiteBalance};
use photoforge_lib::raw::develop::{
    develop_preview, develop_sensor, RenderScale, DEVELOP_BYTES_PER_PHOTOSITE,
};
use photoforge_lib::raw::dng::fixtures::DngBuilder;
use photoforge_lib::raw::dng::{decode, SEGMENT_TRANSIENT_BYTES_PER_PHOTOSITE};
use photoforge_lib::raw::{
    RawCaptureMetadata, RawError, RawFormat, RawLayerSource, RawSourceMode, RawSourceReference,
};
use photoforge_lib::resources::admission::{
    plan, AdmissionOption, ReducedDecode, RegionDecode, SourceKind, SourceProbe, Verdict,
};
use photoforge_lib::resources::memory::SystemMemory;
use photoforge_lib::resources::policy::{compute_budget, BudgetMode, ResourceLimits, GIB};
use photoforge_lib::source::dng::{self, capabilities, DngInfo};
use photoforge_lib::source::probe::probe_path;
use photoforge_lib::source::Rect;
use std::sync::atomic::{AtomicBool, Ordering};

fn texture(width: u32, height: u32) -> Vec<u16> {
    let mut state = 0x1357_9bdfu32;
    (0..width * height)
        .map(|index| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let (x, y) = (index % width, index / width);
            let ramp = ((x * 37 + y * 19) % 2048) as u16;
            let edge = if (x / 11 + y / 13) % 2 == 0 { 600 } else { 0 };
            (250 + ramp / 2 + edge + ((state >> 23) as u16) / 3).min(4000)
        })
        .collect()
}

fn dng_bytes(width: u32, height: u32, tile: Option<(u32, u32)>) -> Vec<u8> {
    let mut builder = DngBuilder::new(width, height, texture(width, height))
        .levels(vec![64, 64, 64, 64], (2, 2), 4095)
        .neutral([0.52, 1.0, 0.61])
        .matrix([1.2, -0.3, 0.1, -0.2, 1.1, 0.05, 0.02, -0.1, 0.9]);
    builder.tile_size = tile;
    builder.build()
}

fn write_dng(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

// ---------------------------------------------------------------------------------
// The planner is told what the decoder can do, and is not flattered.
// ---------------------------------------------------------------------------------

#[test]
fn a_dng_probes_as_a_segmented_region_source_with_no_reduced_copy() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_dng(dir.path(), "frame.dng", &dng_bytes(96, 64, Some((32, 32))));
    let probed = probe_path(&path).unwrap();

    assert_eq!(probed.kind, SourceKind::Dng);
    assert_eq!((probed.width, probed.height), (96, 64));
    assert!(matches!(
        probed.probe.capabilities.region,
        RegionDecode::Segments { .. }
    ));
    // Honest: no reduced *document* is offered for a RAW.
    assert_eq!(probed.probe.capabilities.reduced, ReducedDecode::None);
    // Opening whole develops the entire sensor at once, and is priced that way
    // rather than at the two bytes a photosite occupies in the file.
    assert_eq!(
        probed.probe.full_decode_bytes_per_pixel,
        DEVELOP_BYTES_PER_PHOTOSITE
    );
    assert!(probed.probe.full_decode_bytes_per_pixel > 2 * probed.probe.native_bytes_per_pixel);
}

#[test]
fn a_tiled_file_is_priced_by_its_tile_and_a_single_strip_by_the_whole_sensor() {
    let (width, height) = (512, 384);
    let tiled = DngInfo {
        width,
        height,
        bits: 12,
        tiled: true,
        segment_pixels: 64 * 64,
        segments: 48,
    };
    let strip = DngInfo {
        tiled: false,
        segment_pixels: u64::from(width) * u64::from(height),
        segments: 1,
        ..tiled.clone()
    };
    let fixed = |info: &DngInfo| match capabilities(info).region {
        RegionDecode::Segments { fixed_bytes, .. } => fixed_bytes,
        other => panic!("a DNG is priced as segments, not {other:?}"),
    };
    // The only difference between the two is the segment that is decoded whole.
    assert_eq!(
        fixed(&strip) - fixed(&tiled),
        (u64::from(width) * u64::from(height) - 64 * 64) * SEGMENT_TRANSIENT_BYTES_PER_PHOTOSITE
    );
}

/// A sensor too large for the budget has a region the planner will admit, and the
/// ceiling it names is exactly where the model says the peak crosses the budget.
#[test]
fn an_oversized_sensor_is_offered_a_region_and_never_a_reduced_copy() {
    let info = DngInfo {
        width: 12_000,
        height: 9_000,
        bits: 14,
        tiled: true,
        segment_pixels: 256 * 256,
        segments: 47 * 36,
    };
    let probe = SourceProbe {
        kind: SourceKind::Dng,
        width: 12_000,
        height: 9_000,
        native_bytes_per_pixel: 4,
        full_decode_bytes_per_pixel: DEVELOP_BYTES_PER_PHOTOSITE,
        file_bytes: 180 * 1024 * 1024,
        capabilities: capabilities(&info),
    };
    // A small machine: 108 MP of sensor cannot be developed whole.
    let budget = compute_budget(
        BudgetMode::Automatic,
        Some(SystemMemory {
            total_physical: 8 * GIB,
            available_physical: 6 * GIB,
        }),
    );
    let limits = ResourceLimits::from_budget(budget.bytes);
    let report = plan(&probe, &budget, &limits, Some(6 * GIB), 0);

    assert_eq!(report.verdict, Verdict::RegionRequired, "{report:?}");
    let region = report
        .options
        .iter()
        .find_map(|option| match option {
            AdmissionOption::OpenRegion {
                max_region_pixels, ..
            } => Some(*max_region_pixels),
            _ => None,
        })
        .expect("a region is offered");
    assert!(
        region > 1_000_000,
        "the region ceiling is useless: {region}"
    );
    assert!(region < 12_000 * 9_000);
    assert!(
        !report
            .options
            .iter()
            .any(|option| matches!(option, AdmissionOption::OpenReduced { .. })),
        "a RAW has no reduced copy and must not be offered one"
    );

    // The same sensor on a large machine opens whole.
    let big = compute_budget(
        BudgetMode::Automatic,
        Some(SystemMemory {
            total_physical: 128 * GIB,
            available_physical: 100 * GIB,
        }),
    );
    let big_limits = ResourceLimits::from_budget(big.bytes);
    let report = plan(&probe, &big, &big_limits, Some(100 * GIB), 0);
    assert!(
        matches!(
            report.verdict,
            Verdict::FullResolution | Verdict::FullResolutionWithWarning { .. }
        ),
        "{:?}",
        report.verdict
    );
}

// ---------------------------------------------------------------------------------
// The bounded preview.
// ---------------------------------------------------------------------------------

/// The preview is the sensor decimated by whole two-photosite blocks and then
/// developed. Doing that decimation by hand on a fully decoded sensor, and
/// developing it with the public entry point, must give the same pixels — which
/// proves the streamed preview read the right photosites without decoding a whole
/// sensor.
#[test]
fn the_streamed_preview_equals_a_hand_decimated_development() {
    let (width, height) = (419u32, 301u32);
    for tile in [None, Some((64, 64)), Some((48, 32))] {
        let data = dng_bytes(width, height, tile);
        let parameters = DevelopmentParameters {
            white_balance: WhiteBalance::Auto,
            ..DevelopmentParameters::default()
        };
        let preview = develop_preview(&data, &parameters, 64, None).unwrap();

        // By hand.
        let mut sensor = decode(&data).unwrap();
        let factor = width.max(height).div_ceil(64);
        let step = factor * 2;
        let (out_width, out_height) = ((width / step).max(1) * 2, (height / step).max(1) * 2);
        let mut samples = vec![0u16; (out_width * out_height) as usize];
        for y in 0..out_height {
            for x in 0..out_width {
                let sx = ((x / 2) * step + x % 2).min(width - 1);
                let sy = ((y / 2) * step + y % 2).min(height - 1);
                samples[(y * out_width + x) as usize] = sensor.sample(sx, sy);
            }
        }
        sensor.data = samples;
        sensor.width = out_width;
        sensor.height = out_height;
        sensor.active_area = (0, 0, out_width, out_height);
        sensor.default_crop = None;
        let by_hand = develop_sensor(&sensor, &parameters, RenderScale::Full).unwrap();

        assert_eq!(
            (preview.image.width(), preview.image.height()),
            (out_width, out_height),
            "tile {tile:?}"
        );
        assert!(preview.image.width().max(preview.image.height()) <= 64 + 2);
        assert_eq!(
            preview.image.pixels(),
            by_hand.image.pixels(),
            "tile {tile:?}"
        );
        // The picture it describes is the whole sensor, not the preview.
        assert_eq!(preview.source_dimensions, (width, height));
    }
}

#[test]
fn a_sensor_already_small_enough_is_previewed_whole() {
    let data = dng_bytes(48, 32, None);
    let preview = develop_preview(&data, &DevelopmentParameters::default(), 512, None).unwrap();
    assert_eq!((preview.image.width(), preview.image.height()), (48, 32));
}

#[test]
fn a_preview_can_be_cancelled_between_segments() {
    let data = dng_bytes(419, 301, Some((32, 32)));
    let cancel = AtomicBool::new(true);
    let error =
        develop_preview(&data, &DevelopmentParameters::default(), 64, Some(&cancel)).unwrap_err();
    assert_eq!(error, RawError::Cancelled);
    // And an unset flag does not interfere.
    cancel.store(false, Ordering::Release);
    assert!(develop_preview(&data, &DevelopmentParameters::default(), 64, Some(&cancel)).is_ok());
}

#[test]
fn the_preview_command_path_builds_a_picture_from_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_dng(dir.path(), "wide.dng", &dng_bytes(419, 301, Some((64, 64))));
    let picture = dng::preview(&path, 64, None).unwrap();
    assert!(picture.width().max(picture.height()) <= 66);
}

// ---------------------------------------------------------------------------------
// The record a layer keeps.
// ---------------------------------------------------------------------------------

fn layer(width: u32, height: u32, view: Option<Rect>) -> RawLayerSource {
    RawLayerSource {
        reference: RawSourceReference {
            filename: "huge.dng".into(),
            format: RawFormat::Dng,
            file_size: 200 * 1024 * 1024,
            sha256: "ab".repeat(32),
            width,
            height,
        },
        mode: RawSourceMode::Linked {
            path: "C:/photos/huge.dng".into(),
        },
        parameters: DevelopmentParameters::default(),
        decoder: "photoforge-dng".into(),
        decoder_version: "2".into(),
        capture: RawCaptureMetadata::default(),
        view,
    }
}

/// The default limit is the legacy 67 MP, so a 200 MP sensor cannot be a whole
/// layer, but a window of it can.
#[test]
fn a_sensor_beyond_the_budget_is_a_valid_layer_only_as_a_region() {
    let sensor = (20_000, 10_000);
    assert!(layer(sensor.0, sensor.1, None).validate().is_err());

    let window = Rect {
        x: 4_000,
        y: 2_000,
        width: 3_000,
        height: 2_000,
    };
    layer(sensor.0, sensor.1, Some(window)).validate().unwrap();

    // The window must be on the sensor...
    let off = Rect {
        x: 19_000,
        y: 0,
        width: 3_000,
        height: 2_000,
    };
    assert!(layer(sensor.0, sensor.1, Some(off)).validate().is_err());
    // ...and must itself fit the budget.
    let whole_ish = Rect {
        x: 0,
        y: 0,
        width: 20_000,
        height: 10_000,
    };
    assert!(layer(sensor.0, sensor.1, Some(whole_ish))
        .validate()
        .is_err());
    // A hostile sensor size is refused whatever the window.
    assert!(layer(u32::MAX, u32::MAX, Some(window)).validate().is_err());
}

#[test]
fn the_view_survives_a_round_trip_and_is_absent_for_a_whole_sensor() {
    let window = Rect {
        x: 10,
        y: 20,
        width: 300,
        height: 200,
    };
    let with = serde_json::to_value(layer(4_000, 3_000, Some(window))).unwrap();
    assert_eq!(with["view"]["x"], 10);
    let back: RawLayerSource = serde_json::from_value(with).unwrap();
    assert_eq!(back.view, Some(window));

    // A whole-sensor layer writes no `view`, so a project saved before regions
    // existed — and one saved after, of an ordinary RAW — are identical.
    let without = serde_json::to_value(layer(4_000, 3_000, None)).unwrap();
    assert!(without.get("view").is_none());
    let back: RawLayerSource = serde_json::from_value(without).unwrap();
    assert_eq!(back.view, None);

    // An unknown field is still refused: a record from a newer build is not
    // quietly read as an older one.
    let mut tampered = serde_json::to_value(layer(4_000, 3_000, None)).unwrap();
    tampered["surprise"] = serde_json::json!(1);
    assert!(serde_json::from_value::<RawLayerSource>(tampered).is_err());
}
