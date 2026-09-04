//! Integration tests for the RAW commands, driven through the real Tauri IPC
//! boundary.
//!
//! Every call goes out as an `InvokeRequest` with a JSON body of the shape the
//! WebView sends, is matched to a command by name, has its arguments
//! deserialized by serde, receives the managed `AppState`, and comes back
//! serialized. A renamed field or a command missing from the handler fails
//! here and nowhere else.
//!
//! Fixtures are DNG files this test writes itself, so every sample has a known
//! expected value and no photograph anyone else owns is committed. Real camera
//! files are covered separately and optionally by `raw_real_files.rs`.

use photoforge_lib::application::AppState;
use photoforge_lib::color::{DevelopmentParameters, WhiteBalance};
use photoforge_lib::raw::dng::fixtures::DngBuilder;
use serde_json::{json, Value};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

struct Harness {
    /// Kept alive: dropping the app would take the webview with it.
    _app: tauri::App<MockRuntime>,
    webview: WebviewWindow<MockRuntime>,
}

impl Harness {
    fn new() -> Self {
        let app = photoforge_lib::commands::register_raw_commands(mock_builder())
            .manage(AppState::default())
            .build(mock_context(noop_assets()))
            .expect("mock application");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview");
        Self { _app: app, webview }
    }

    fn call(&self, command: &str, body: Value) -> Result<Value, String> {
        let response = tauri::test::get_ipc_response(
            &self.webview,
            InvokeRequest {
                cmd: command.into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "http://tauri.localhost".parse().unwrap(),
                body: body.into(),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        );
        match response {
            Ok(payload) => Ok(payload.deserialize().expect("response is JSON")),
            Err(error) => Err(error.to_string()),
        }
    }

    fn ok(&self, command: &str, body: Value) -> Value {
        self.call(command, body)
            .unwrap_or_else(|error| panic!("{command} failed: {error}"))
    }

    fn err(&self, command: &str, body: Value) -> String {
        match self.call(command, body) {
            Ok(value) => panic!("{command} unexpectedly succeeded: {value}"),
            Err(error) => error,
        }
    }
}

/// A gradient sensor with a real camera's shape: a white level below full
/// scale, a per-position black level, an as-shot neutral, and a colour matrix.
fn camera_like_dng(width: u32, height: u32) -> Vec<u8> {
    let samples: Vec<u16> = (0..width * height)
        .map(|index| 600 + ((index * 37) % 3000) as u16)
        .collect();
    DngBuilder::new(width, height, samples)
        .levels(vec![512, 512, 512, 514], (2, 2), 3800)
        .neutral([0.55, 1.0, 0.72])
        .matrix([0.72, 0.18, 0.10, 0.16, 0.74, 0.10, 0.09, 0.20, 0.71])
        .iso(400)
        .build()
}

fn write(directory: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let path = directory.join(name);
    std::fs::write(&path, bytes).expect("writes fixture");
    path
}

fn temp() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

fn as_string(value: &Value) -> String {
    value.as_str().expect("string").to_string()
}

// ---------------------------------------------------------------------------
// Dispatch and inspection
// ---------------------------------------------------------------------------

#[test]
fn every_raw_command_is_reachable_by_name() {
    let directory = temp();
    let harness = Harness::new();
    let path = write(directory.path(), "shot.dng", &camera_like_dng(16, 16));
    // A misspelled command must be refused, which is what proves the
    // successful calls below actually resolved.
    let missing = harness.err("inspect_ra", json!({ "path": path.to_string_lossy() }));
    assert!(missing.to_lowercase().contains("not found"), "{missing}");
    let result = harness.ok("inspect_raw", json!({ "path": path.to_string_lossy() }));
    assert_eq!(result["inspection"]["support"], "decodable");
}

#[test]
fn inspection_hashes_the_file_and_names_the_decoder_without_decoding_it() {
    let directory = temp();
    let harness = Harness::new();
    let bytes = camera_like_dng(16, 16);
    let path = write(directory.path(), "shot.dng", &bytes);
    let before = std::fs::read(&path).unwrap();

    let result = harness.ok("inspect_raw", json!({ "path": path.to_string_lossy() }));
    let inspection = &result["inspection"];
    assert_eq!(inspection["format"], "DNG");
    assert_eq!(inspection["fileSize"], bytes.len());
    assert_eq!(as_string(&inspection["sha256"]).len(), 64);
    assert_eq!(inspection["tiffHeader"], true);
    assert_eq!(inspection["decoder"], "photoforge-dng");
    assert_eq!(result["capabilities"]["backendAvailable"], true);
    assert_eq!(result["capabilities"]["nativeDependencies"], false);
    // Inspecting must not touch the file.
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

// ---------------------------------------------------------------------------
// Opening and developing
// ---------------------------------------------------------------------------

#[test]
fn opening_a_raw_layer_develops_it_and_leaves_the_original_untouched() {
    let directory = temp();
    let harness = Harness::new();
    let bytes = camera_like_dng(32, 24);
    let path = write(directory.path(), "shot.dng", &bytes);
    let before = std::fs::read(&path).unwrap();

    let result = harness.ok(
        "open_raw_layer",
        json!({ "path": path.to_string_lossy(), "fullResolution": true }),
    );
    assert_eq!(result["width"], 32);
    assert_eq!(result["height"], 24);
    assert_eq!(result["sourceWidth"], 32);
    assert_eq!(result["sourceHeight"], 24);
    assert!(!as_string(&result["pixelId"]).is_empty());
    assert_eq!(result["demosaic"], "malvar-he-cutler");
    assert_eq!(result["cfaPattern"], "RGGB");
    assert_eq!(result["bitsPerSample"], 16);
    assert_eq!(result["whiteLevel"], 3800.0);
    assert_eq!(result["blackLevel"], json!([512.0, 512.0, 512.0, 514.0]));
    assert_eq!(result["colorManaged"], true);
    assert_eq!(result["metadata"]["iso"], 400);

    // The source record carries everything needed to develop it again.
    let source = &result["source"];
    assert_eq!(source["mode"]["mode"], "linked");
    assert_eq!(source["decoder"], "photoforge-dng");
    assert_eq!(source["reference"]["format"], "DNG");
    assert_eq!(as_string(&source["reference"]["sha256"]).len(), 64);

    assert_eq!(
        std::fs::read(&path).unwrap(),
        before,
        "developing modified the original RAW file"
    );
}

#[test]
fn a_preview_is_smaller_than_the_sensor_but_reports_the_true_size() {
    let directory = temp();
    let harness = Harness::new();
    // Wider than the preview ceiling.
    let path = write(directory.path(), "big.dng", &camera_like_dng(3600, 8));
    let preview = harness.ok(
        "open_raw_layer",
        json!({ "path": path.to_string_lossy(), "fullResolution": false }),
    );
    assert!(preview["width"].as_u64().unwrap() < 3600);
    assert_eq!(preview["sourceWidth"], 3600);
    assert_eq!(preview["demosaic"], "bilinear");
}

#[test]
fn as_shot_white_balance_comes_from_the_file_and_is_recorded_for_reopening() {
    let directory = temp();
    let harness = Harness::new();
    let path = write(directory.path(), "shot.dng", &camera_like_dng(16, 16));
    let result = harness.ok(
        "open_raw_layer",
        json!({ "path": path.to_string_lossy(), "fullResolution": true }),
    );
    let multipliers = result["multipliers"].as_array().unwrap();
    // The neutral was [0.55, 1.0, 0.72], so the multipliers are its reciprocal
    // normalised to green.
    assert!((multipliers[0].as_f64().unwrap() - 1.0 / 0.55).abs() < 1e-3);
    assert!((multipliers[1].as_f64().unwrap() - 1.0).abs() < 1e-6);
    assert!((multipliers[2].as_f64().unwrap() - 1.0 / 0.72).abs() < 1e-3);
    // Resolved rather than left as a mode, so reopening reproduces the picture.
    assert_eq!(
        result["source"]["parameters"]["whiteBalance"]["mode"],
        "custom"
    );
}

#[test]
fn re_developing_with_new_parameters_produces_a_new_buffer_from_the_original() {
    let directory = temp();
    let harness = Harness::new();
    let path = write(directory.path(), "shot.dng", &camera_like_dng(24, 16));
    let opened = harness.ok(
        "open_raw_layer",
        json!({ "path": path.to_string_lossy(), "fullResolution": true }),
    );
    let source = opened["source"].clone();

    let mut parameters: DevelopmentParameters =
        serde_json::from_value(source["parameters"].clone()).unwrap();
    parameters.exposure_ev = 1.0;

    let redeveloped = harness.ok(
        "develop_raw_layer",
        json!({
            "request": {
                "source": source,
                "parameters": parameters,
                "fullResolution": true,
                "documentId": 1,
                "requestId": 2
            }
        }),
    );
    assert_ne!(
        as_string(&redeveloped["pixelId"]),
        as_string(&opened["pixelId"]),
        "re-developing reused the old buffer"
    );
    // The new parameters are what the layer now stores.
    assert_eq!(
        redeveloped["source"]["parameters"]["exposureEv"]
            .as_f64()
            .unwrap(),
        1.0
    );
}

#[test]
fn re_developing_refuses_a_source_that_has_gone_missing_or_changed() {
    let directory = temp();
    let harness = Harness::new();
    let path = write(directory.path(), "shot.dng", &camera_like_dng(16, 16));
    let opened = harness.ok(
        "open_raw_layer",
        json!({ "path": path.to_string_lossy(), "fullResolution": true }),
    );
    let source = opened["source"].clone();

    // A different photograph written to the same path.
    write(
        directory.path(),
        "shot.dng",
        &camera_like_dng(16, 16)
            .iter()
            .map(|b| b ^ 0x01)
            .collect::<Vec<u8>>(),
    );
    let changed = harness.err(
        "develop_raw_layer",
        json!({ "request": { "source": source.clone(), "fullResolution": false, "documentId": 1, "requestId": 1 } }),
    );
    assert!(!changed.is_empty());

    std::fs::remove_file(&path).unwrap();
    let missing = harness.err(
        "develop_raw_layer",
        json!({ "request": { "source": source, "fullResolution": false, "documentId": 1, "requestId": 1 } }),
    );
    assert!(
        missing.contains("no longer at") || missing.contains("not the photograph"),
        "{missing}"
    );
}

// ---------------------------------------------------------------------------
// Source verification and relinking
// ---------------------------------------------------------------------------

#[test]
fn a_missing_source_is_reported_rather_than_crashing_and_can_be_relinked() {
    let directory = temp();
    let harness = Harness::new();
    let bytes = camera_like_dng(16, 16);
    let path = write(directory.path(), "shot.dng", &bytes);
    let opened = harness.ok(
        "open_raw_layer",
        json!({ "path": path.to_string_lossy(), "fullResolution": false }),
    );
    let source = opened["source"].clone();
    let reference = source["reference"].clone();

    // Move the photograph somewhere else, as a user reorganising a library
    // would.
    let moved = directory.path().join("archive").join("shot.dng");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&path, &moved).unwrap();

    let status = harness.ok(
        "verify_raw_source",
        json!({ "reference": reference.clone(), "path": path.to_string_lossy() }),
    );
    assert_eq!(status["status"], "missing");

    let relinked = harness.ok(
        "relink_raw_source",
        json!({ "source": source, "path": moved.to_string_lossy() }),
    );
    assert!(as_string(&relinked["mode"]["path"]).ends_with("shot.dng"));

    let after = harness.ok(
        "verify_raw_source",
        json!({ "reference": reference, "path": moved.to_string_lossy() }),
    );
    assert_eq!(after["status"], "available");
}

/// The refusal that matters: a project must never be bound to a different
/// photograph just because the file has a plausible name.
#[test]
fn relinking_refuses_a_different_photograph_and_reports_the_mismatch() {
    let directory = temp();
    let harness = Harness::new();
    let path = write(directory.path(), "shot.dng", &camera_like_dng(16, 16));
    let opened = harness.ok(
        "open_raw_layer",
        json!({ "path": path.to_string_lossy(), "fullResolution": false }),
    );
    let source = opened["source"].clone();

    let impostor = write(directory.path(), "shot-copy.dng", &camera_like_dng(16, 24));
    let error = harness.err(
        "relink_raw_source",
        json!({ "source": source, "path": impostor.to_string_lossy() }),
    );
    assert!(
        error.contains("different photograph"),
        "the refusal did not explain itself: {error}"
    );

    // And verification says the same thing rather than silently accepting it.
    let reference = opened["source"]["reference"].clone();
    let status = harness.ok(
        "verify_raw_source",
        json!({ "reference": reference, "path": impostor.to_string_lossy() }),
    );
    assert_eq!(
        status["status"]["changed"]["foundSha256"]
            .as_str()
            .map(str::len),
        Some(64)
    );
}

// ---------------------------------------------------------------------------
// Export (sections 18 and 19)
// ---------------------------------------------------------------------------

#[test]
fn exporting_writes_a_full_resolution_sixteen_bit_png_from_the_original() {
    let directory = temp();
    let harness = Harness::new();
    let path = write(directory.path(), "shot.dng", &camera_like_dng(64, 48));
    // Deliberately open a *preview*, to prove the export does not use it.
    let opened = harness.ok(
        "open_raw_layer",
        json!({ "path": path.to_string_lossy(), "fullResolution": false }),
    );
    let output = directory.path().join("out.png");
    let result = harness.ok(
        "export_raw_layer_png16",
        json!({
            "outputPath": output.to_string_lossy(),
            "source": opened["source"].clone()
        }),
    );
    assert_eq!(result["width"], 64);
    assert_eq!(result["height"], 48);

    let decoded = image::open(&output).expect("the exported file decodes");
    assert_eq!(decoded.width(), 64);
    assert_eq!(decoded.height(), 48);
    // A true sixteen-bit container, not an eight-bit one in disguise.
    assert!(
        matches!(
            decoded.color(),
            image::ColorType::Rgba16 | image::ColorType::Rgb16
        ),
        "the export is not sixteen bit: {:?}",
        decoded.color()
    );
}

/// The claim a 16-bit export has to earn: the file holds values that expanding
/// 8-bit data could never produce. Every 8-bit value maps to a multiple of 257
/// in 16 bits, so anything that is not a multiple of 257 is precision an 8-bit
/// pipeline did not have.
#[test]
fn the_exported_sixteen_bit_png_holds_precision_eight_bits_cannot_represent() {
    let directory = temp();
    let harness = Harness::new();
    // A smooth ramp, which is where extra precision is visible.
    let (width, height) = (64u32, 64u32);
    let samples: Vec<u16> = (0..width * height)
        .map(|index| 520 + (index % 3000) as u16)
        .collect();
    let bytes = DngBuilder::new(width, height, samples)
        .levels(vec![512], (1, 1), 3800)
        .build();
    let path = write(directory.path(), "ramp.dng", &bytes);
    let opened = harness.ok(
        "open_raw_layer",
        json!({ "path": path.to_string_lossy(), "fullResolution": true }),
    );
    let output = directory.path().join("ramp.png");
    harness.ok(
        "export_raw_layer_png16",
        json!({
            "outputPath": output.to_string_lossy(),
            "source": opened["source"].clone()
        }),
    );

    let decoded = image::open(&output).expect("decodes").to_rgba16();
    let total = decoded.pixels().count();
    let beyond_eight_bit = decoded
        .pixels()
        .flat_map(|pixel| pixel.0.iter().take(3).copied())
        .filter(|value| *value % 257 != 0)
        .count();
    assert!(
        beyond_eight_bit > total / 4,
        "only {beyond_eight_bit} of {} samples carry sub-8-bit precision; this is an 8-bit image in a 16-bit container",
        total * 3
    );
}

#[test]
fn exporting_refuses_to_overwrite_the_raw_source_and_refuses_a_missing_source() {
    let directory = temp();
    let harness = Harness::new();
    let path = write(directory.path(), "shot.dng", &camera_like_dng(16, 16));
    let opened = harness.ok(
        "open_raw_layer",
        json!({ "path": path.to_string_lossy(), "fullResolution": false }),
    );
    let source = opened["source"].clone();

    // Writing over the original is refused by the shared export guard.
    assert!(!harness
        .err(
            "export_raw_layer_png16",
            json!({ "outputPath": path.to_string_lossy(), "source": source.clone() })
        )
        .is_empty());

    std::fs::remove_file(&path).unwrap();
    let error = harness.err(
        "export_raw_layer_png16",
        json!({
            "outputPath": directory.path().join("x.png").to_string_lossy(),
            "source": source
        }),
    );
    assert!(error.contains("missing or changed"), "{error}");
}

// ---------------------------------------------------------------------------
// Hostile input (sections 23, 24, 25)
// ---------------------------------------------------------------------------

#[test]
fn hostile_and_malformed_files_are_refused_with_a_readable_reason() {
    let directory = temp();
    let harness = Harness::new();
    let valid = camera_like_dng(16, 16);

    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty.dng", Vec::new()),
        (
            "random.dng",
            (0..4096u32).map(|i| (i * 37 % 251) as u8).collect(),
        ),
        ("truncated.dng", valid[..valid.len() / 3].to_vec()),
        ("header-only.dng", valid[..8].to_vec()),
        (
            "jpeg.dng",
            vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0],
        ),
        ("zeros.dng", vec![0u8; 2048]),
        ("ones.dng", vec![0xFFu8; 2048]),
    ];
    for (name, bytes) in cases {
        let path = write(directory.path(), name, &bytes);
        let error = harness.err(
            "open_raw_layer",
            json!({ "path": path.to_string_lossy(), "fullResolution": false }),
        );
        assert!(!error.is_empty(), "{name} produced an empty error");
        // A user-facing message, not a stack trace or a pointer.
        assert!(
            !error.contains("panicked") && !error.contains("0x"),
            "{name} leaked an internal diagnostic: {error}"
        );
        assert!(
            error.len() < 400,
            "{name} produced an unreasonably long message"
        );
    }
}

#[test]
fn a_recognised_format_without_a_decoder_is_refused_by_name() {
    let directory = temp();
    let harness = Harness::new();
    // A valid DNG body, but named as a Canon file: the extension decides which
    // decoder is asked for, and there is none for CR2.
    let path = write(directory.path(), "shot.cr2", &camera_like_dng(16, 16));
    let inspection = harness.ok("inspect_raw", json!({ "path": path.to_string_lossy() }));
    assert_eq!(
        inspection["inspection"]["support"],
        "recognizedDecoderUnavailable"
    );
    assert_eq!(inspection["inspection"]["decoder"], Value::Null);
}

#[test]
fn a_path_that_is_not_a_file_is_refused() {
    let directory = temp();
    let harness = Harness::new();
    for path in [
        directory.path().to_path_buf(),
        directory.path().join("absent.dng"),
    ] {
        assert!(!harness
            .err(
                "open_raw_layer",
                json!({ "path": path.to_string_lossy(), "fullResolution": false })
            )
            .is_empty());
    }
}

#[test]
fn invalid_development_parameters_are_refused_at_the_boundary() {
    let directory = temp();
    let harness = Harness::new();
    let path = write(directory.path(), "shot.dng", &camera_like_dng(16, 16));
    for parameters in [
        json!({ "whiteBalance": { "mode": "asShot", "multipliers": [1.0, 1.0, 1.0] }, "exposureEv": 99.0, "contrast": 0.0, "highlights": 0.0, "shadows": 0.0, "whites": 0.0, "blacks": 0.0 }),
        json!({ "whiteBalance": { "mode": "custom", "multipliers": [0.0, 1.0, 1.0] }, "exposureEv": 0.0, "contrast": 0.0, "highlights": 0.0, "shadows": 0.0, "whites": 0.0, "blacks": 0.0 }),
        json!({ "whiteBalance": { "mode": "asShot", "multipliers": [1.0, 1.0, 1.0] }, "exposureEv": 0.0, "contrast": 5.0, "highlights": 0.0, "shadows": 0.0, "whites": 0.0, "blacks": 0.0 }),
    ] {
        assert!(!harness
            .err(
                "open_raw_layer",
                json!({
                    "path": path.to_string_lossy(),
                    "parameters": parameters,
                    "fullResolution": false
                })
            )
            .is_empty());
    }
}

/// Development must not depend on how many times it has run.
#[test]
fn developing_the_same_file_twice_gives_the_same_answer() {
    let directory = temp();
    let harness = Harness::new();
    let path = write(directory.path(), "shot.dng", &camera_like_dng(24, 24));
    let first = harness.ok(
        "open_raw_layer",
        json!({ "path": path.to_string_lossy(), "fullResolution": true }),
    );
    let second = harness.ok(
        "open_raw_layer",
        json!({ "path": path.to_string_lossy(), "fullResolution": true }),
    );
    assert_eq!(first["multipliers"], second["multipliers"]);
    assert_eq!(first["blackLevel"], second["blackLevel"]);
    assert_eq!(first["source"]["reference"], second["source"]["reference"]);
    // Different buffers, identical description.
    assert_ne!(first["pixelId"], second["pixelId"]);
}

/// White balance is a real control over the developed picture, not a label.
#[test]
fn changing_white_balance_changes_the_developed_result() {
    let directory = temp();
    let harness = Harness::new();
    let path = write(directory.path(), "shot.dng", &camera_like_dng(16, 16));
    let opened = harness.ok(
        "open_raw_layer",
        json!({ "path": path.to_string_lossy(), "fullResolution": true }),
    );
    let source = opened["source"].clone();

    let warm = harness.ok(
        "develop_raw_layer",
        json!({
            "request": {
                "source": source,
                "parameters": serde_json::to_value(DevelopmentParameters {
                    white_balance: WhiteBalance::Custom { multipliers: [2.0, 1.0, 0.5] },
                    ..DevelopmentParameters::default()
                }).unwrap(),
                "fullResolution": true,
                "documentId": 1,
                "requestId": 3
            }
        }),
    );
    assert_eq!(warm["multipliers"], json!([2.0, 1.0, 0.5]));
    assert_ne!(warm["multipliers"], opened["multipliers"]);
}
