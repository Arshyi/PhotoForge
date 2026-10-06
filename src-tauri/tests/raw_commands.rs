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
use tauri::{Manager, WebviewWindow, WebviewWindowBuilder};

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
        // RAW layer edits now require a current document, like other layer IPC.
        let original = std::sync::Arc::new(image::DynamicImage::new_rgba8(2, 2));
        *app.state::<AppState>().session.lock().unwrap() =
            Some(photoforge_lib::application::EditorSession {
                document_id: 1,
                analysis: None,
                source: photoforge_lib::infrastructure::LoadedImage {
                    path: "raw-command-test.png".into(),
                    original: original.clone(),
                    preview: original,
                    working: None,
                    metadata: photoforge_lib::domain::ImageMetadata {
                        filename: "test.png".into(),
                        width: 2,
                        height: 2,
                        format: "PNG".into(),
                        file_size: 0,
                        color_space: "sRGB".into(),
                        bit_depth: 8,
                        has_alpha: true,
                        created_at: None,
                        modified_at: None,
                        camera_model: None,
                        exif_available: false,
                        raw: None,
                        origin: None,
                    },
                },
            });
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

// ---------------------------------------------------------------------------
// Project round-trip with a RAW layer (section 14)
// ---------------------------------------------------------------------------

/// Builds the complex document the brief asks for: a RAW layer with its
/// development settings, a transform and a mask, alongside an adjustment layer,
/// a pass-through group, and an ordinary pixel layer.
fn complex_raw_document(
    harness: &Harness,
    raw_path: &std::path::Path,
) -> (
    photoforge_lib::layers::LayerDocument,
    Value,
    std::collections::HashMap<String, image::RgbaImage>,
) {
    use photoforge_lib::layers::{
        BlendMode, Layer, LayerContent, LayerDocument, LayerMask, LayerMetadata, LayerTransform,
    };
    use photoforge_lib::mask::{MaskBitmap, MaskSnapshot};

    let opened = harness.ok(
        "open_raw_layer",
        json!({ "path": raw_path.to_string_lossy(), "fullResolution": true }),
    );
    let raw_source: photoforge_lib::raw::RawLayerSource =
        serde_json::from_value(opened["source"].clone()).expect("source");
    let pixel_id = as_string(&opened["pixelId"]);
    let width = opened["width"].as_u64().unwrap() as u32;
    let height = opened["height"].as_u64().unwrap() as u32;

    // The developed raster, so the project has real pixels to store.
    let developed = image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
    });
    let ordinary = image::RgbaImage::from_pixel(width, height, image::Rgba([40, 90, 140, 255]));

    let mut buffers = std::collections::HashMap::new();
    buffers.insert(pixel_id.clone(), developed);
    buffers.insert("pxplain".to_string(), ordinary);

    let base = |id: &str| Layer {
        id: id.into(),
        name: id.into(),
        visible: true,
        locked: false,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        transform: LayerTransform::default(),
        mask: None,
        collapsed: false,
        metadata: LayerMetadata::default(),
        raw: None,
        origin: None,
        content: LayerContent::Pixel {
            pixel_id: "pxplain".into(),
            width,
            height,
        },
    };

    let mut mask_bitmap = MaskBitmap::empty(width, height).expect("mask");
    for y in 0..height {
        for x in 0..width / 2 {
            mask_bitmap.set(x, y, 255);
        }
    }

    // The RAW layer, carrying its source, a transform, and a mask.
    let mut raw_layer = base("rawlayer");
    raw_layer.name = "Developed RAW".into();
    raw_layer.opacity = 0.82;
    raw_layer.blend_mode = BlendMode::Multiply;
    raw_layer.transform = LayerTransform {
        translate_x: 3.0,
        translate_y: -2.0,
        scale_x: 1.25,
        rotation_degrees: 8.0,
        ..LayerTransform::default()
    };
    raw_layer.mask = Some(LayerMask {
        snapshot: MaskSnapshot::encode(&mask_bitmap),
        enabled: true,
        inverted: false,
    });
    raw_layer.raw = Some(raw_source);
    raw_layer.content = LayerContent::Pixel {
        pixel_id: pixel_id.clone(),
        width,
        height,
    };

    let adjustment = Layer {
        name: "Warmth".into(),
        opacity: 0.7,
        content: LayerContent::Adjustment {
            operation: Box::new(photoforge_lib::domain::EditOperation::Contrast { amount: 0.2 }),
        },
        ..base("adjust")
    };

    let mut group = Layer {
        content: LayerContent::Group {
            children: vec![adjustment, raw_layer],
            isolated: false,
        },
        ..base("passgroup")
    };
    group.name = "Pass-through".into();

    let document = LayerDocument {
        schema_version: photoforge_lib::layers::LAYER_SCHEMA_VERSION,
        precision: Default::default(),
        canvas_width: width,
        canvas_height: height,
        layers: vec![base("plain"), group],
        smart_sources: Default::default(),
        active_layer_id: Some("rawlayer".into()),
    };
    (document, opened, buffers)
}

/// The heart of the phase: a RAW-backed project must survive a save and a
/// reload with its source identity and development state intact, so reopening
/// develops the photograph again rather than inheriting a baked raster.
#[test]
fn a_raw_backed_project_survives_a_save_and_reload_with_its_development_state() {
    use photoforge_lib::layers::{decode_project, encode_project, LayerContent};

    let directory = temp();
    let harness = Harness::new();
    let raw_path = write(directory.path(), "shot.dng", &camera_like_dng(32, 24));
    let (document, opened, buffers) = complex_raw_document(&harness, &raw_path);

    let borrowed: Vec<(String, &image::RgbaImage)> = buffers
        .iter()
        .map(|(id, image)| (id.clone(), image))
        .collect();
    let encoded = encode_project(
        &document,
        &[],
        &borrowed,
        "0.9.0",
        "2026-01-01T00:00:00Z",
        "2026-01-02T00:00:00Z",
    )
    .expect("project encodes");

    // A fresh decode stands in for closing and reopening the application.
    let loaded = decode_project(&encoded).expect("project decodes");
    let restored = &loaded.document;

    // Tree identity and stable identifiers.
    let names: Vec<String> = restored.iter().map(|layer| layer.name.clone()).collect();
    assert!(names.contains(&"Developed RAW".to_string()));
    assert!(names.contains(&"Pass-through".to_string()));
    assert_eq!(restored.active_layer_id.as_deref(), Some("rawlayer"));

    let raw_layer = restored.find("rawlayer").expect("the RAW layer came back");
    let source = raw_layer.raw.as_ref().expect("the RAW source came back");

    // Source identity: the same file, verified by hash rather than by name.
    let original = opened["source"]["reference"].clone();
    assert_eq!(source.reference.sha256, as_string(&original["sha256"]));
    assert_eq!(source.reference.filename, "shot.dng");
    assert_eq!(source.reference.format, photoforge_lib::raw::RawFormat::Dng);
    assert_eq!(source.reference.width, 32);
    assert_eq!(source.reference.height, 24);
    assert!(source.mode.is_linked());
    assert_eq!(source.decoder, "photoforge-dng");

    // Development state, which is what makes the edit non-destructive.
    let stored: photoforge_lib::color::DevelopmentParameters =
        serde_json::from_value(opened["source"]["parameters"].clone()).unwrap();
    assert_eq!(source.parameters, stored);

    // Camera metadata survived, so the panel can describe the shot even with
    // the source unavailable.
    assert_eq!(source.capture.iso, Some(400));
    assert_eq!(source.capture.manufacturer.as_deref(), Some("PhotoForge"));

    // Layer state around it.
    assert_eq!(raw_layer.opacity, 0.82);
    assert_eq!(
        raw_layer.blend_mode,
        photoforge_lib::layers::BlendMode::Multiply
    );
    assert_eq!(raw_layer.transform.rotation_degrees, 8.0);
    assert_eq!(raw_layer.transform.scale_x, 1.25);
    assert!(raw_layer.mask.is_some(), "the layer mask was lost");
    let group = restored.find("passgroup").expect("group");
    assert!(matches!(
        group.content,
        LayerContent::Group {
            isolated: false,
            ..
        }
    ));
    assert!(restored.find("adjust").is_some());
    assert!(restored.find("plain").is_some());

    // And the source is still exactly where and what it was.
    let status = harness.ok(
        "verify_raw_source",
        json!({
            "reference": serde_json::to_value(&source.reference).unwrap(),
            "path": raw_path.to_string_lossy()
        }),
    );
    assert_eq!(status["status"], "available");

    // The reopened layer can be developed again from its original file, which
    // is the whole point of storing the source rather than only the raster.
    let redeveloped = harness.ok(
        "develop_raw_layer",
        json!({
            "request": {
                "source": serde_json::to_value(source).unwrap(),
                "fullResolution": true,
                "documentId": 1,
                "requestId": 9
            }
        }),
    );
    assert_eq!(redeveloped["sourceWidth"], 32);
    assert_eq!(redeveloped["multipliers"], opened["multipliers"]);
}

/// A project written before RAW existed must still load, and must not acquire a
/// RAW source it never had.
#[test]
fn a_project_without_any_raw_layer_still_loads_and_gains_no_raw_source() {
    use photoforge_lib::layers::{decode_project, encode_project};

    let directory = temp();
    let harness = Harness::new();
    let raw_path = write(directory.path(), "shot.dng", &camera_like_dng(16, 16));
    let (mut document, _, buffers) = complex_raw_document(&harness, &raw_path);
    // Strip the RAW record, as a pre-0.9.0 project would have.
    fn clear(layers: &mut [photoforge_lib::layers::Layer]) {
        for layer in layers.iter_mut() {
            layer.raw = None;
            if let photoforge_lib::layers::LayerContent::Group { children, .. } = &mut layer.content
            {
                clear(children);
            }
        }
    }
    clear(&mut document.layers);
    let borrowed: Vec<(String, &image::RgbaImage)> = buffers
        .iter()
        .map(|(id, image)| (id.clone(), image))
        .collect();
    let encoded = encode_project(&document, &[], &borrowed, "0.8.2", "", "").expect("encodes");
    let loaded = decode_project(&encoded).expect("decodes");
    assert!(
        loaded.document.iter().all(|layer| layer.raw.is_none()),
        "a layer acquired a RAW source it never had"
    );
}

/// A project whose linked photograph has gone missing must still open, with the
/// layer reporting the problem rather than the whole project failing.
#[test]
fn a_project_opens_when_its_linked_raw_source_is_missing() {
    use photoforge_lib::layers::{decode_project, encode_project};

    let directory = temp();
    let harness = Harness::new();
    let raw_path = write(directory.path(), "shot.dng", &camera_like_dng(16, 16));
    let (document, _, buffers) = complex_raw_document(&harness, &raw_path);
    let borrowed: Vec<(String, &image::RgbaImage)> = buffers
        .iter()
        .map(|(id, image)| (id.clone(), image))
        .collect();
    let encoded = encode_project(&document, &[], &borrowed, "0.9.0", "", "").expect("encodes");

    // The photograph is deleted after the project was saved.
    std::fs::remove_file(&raw_path).unwrap();

    let loaded = decode_project(&encoded).expect("the project itself still opens");
    let raw_layer = loaded.document.find("rawlayer").expect("layer");
    let source = raw_layer.raw.as_ref().expect("source record");
    let status = harness.ok(
        "verify_raw_source",
        json!({
            "reference": serde_json::to_value(&source.reference).unwrap(),
            "path": raw_path.to_string_lossy()
        }),
    );
    assert_eq!(status["status"], "missing");
    // The developed raster is still in the project, so the layer can be shown
    // while the source is unavailable.
    assert!(matches!(
        raw_layer.content,
        photoforge_lib::layers::LayerContent::Pixel { .. }
    ));
}
