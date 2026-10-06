//! The RAW flow for a sensor that may be too large to develop whole, driven
//! through the real IPC boundary: probe, preview, open (whole or as a region),
//! re-develop, export.
//!
//! The headline property is the same one `raw_region.rs` proves at the library
//! level, now through the commands a user's clicks reach: a region opened on its
//! own, then exported, is **identical** to the same rectangle of an export of the
//! whole sensor.
//!
//! Some tests install a small memory budget, which is process-wide state, so every
//! test here takes one lock and restores the defaults when it finishes.
use photoforge_lib::application::AppState;
use photoforge_lib::raw::dng::fixtures::DngBuilder;
use photoforge_lib::resources::{self, BudgetMode};
use serde_json::{json, Value};
use std::sync::{Mutex, MutexGuard};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

const MIB: u64 = 1 << 20;

static LIMITS: Mutex<()> = Mutex::new(());

/// Holds the lock and puts the default limits back however the test ends.
struct Isolated(#[allow(dead_code)] MutexGuard<'static, ()>);

impl Isolated {
    fn new() -> Self {
        let guard = LIMITS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        resources::reset();
        Self(guard)
    }
}

impl Drop for Isolated {
    fn drop(&mut self) {
        resources::reset();
    }
}

struct Harness {
    _app: tauri::App<MockRuntime>,
    webview: WebviewWindow<MockRuntime>,
}

impl Harness {
    fn new() -> Self {
        let app = photoforge_lib::register_raw_flow_commands(mock_builder())
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

fn sensor(width: u32, height: u32, tile: Option<(u32, u32)>) -> Vec<u8> {
    let mut state = 0x0bad_cafeu32;
    let samples: Vec<u16> = (0..width * height)
        .map(|index| {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let (x, y) = (index % width, index / width);
            let ramp = ((x * 31 + y * 17) % 1900) as u16;
            let edge = if (x / 10 + y / 12) % 2 == 0 { 650 } else { 0 };
            (300 + ramp / 2 + edge + ((state >> 23) as u16) / 3).min(3700)
        })
        .collect();
    let mut builder = DngBuilder::new(width, height, samples)
        .levels(vec![512, 512, 512, 514], (2, 2), 3800)
        .neutral([0.55, 1.0, 0.72])
        .matrix([0.72, 0.18, 0.10, 0.16, 0.74, 0.10, 0.09, 0.20, 0.71]);
    builder.tile_size = tile;
    builder.build()
}

fn write(directory: &std::path::Path, name: &str, bytes: &[u8]) -> String {
    let path = directory.join(name);
    std::fs::write(&path, bytes).expect("writes fixture");
    path.to_string_lossy().into_owned()
}

fn rect(x: u32, y: u32, width: u32, height: u32) -> Value {
    json!({ "x": x, "y": y, "width": width, "height": height })
}

fn export(
    harness: &Harness,
    directory: &std::path::Path,
    name: &str,
    source: &Value,
) -> image::DynamicImage {
    let output = directory.join(name);
    harness.ok(
        "export_raw_layer_png16",
        json!({ "outputPath": output.to_string_lossy(), "source": source }),
    );
    image::open(&output).expect("the export decodes")
}

#[test]
fn a_dng_is_probed_as_a_region_capable_source() {
    let _isolated = Isolated::new();
    let directory = tempfile::tempdir().unwrap();
    let path = write(
        directory.path(),
        "frame.dng",
        &sensor(130, 98, Some((32, 32))),
    );
    let harness = Harness::new();

    let probed = harness.ok("probe_image_source", json!({ "path": path }));
    assert_eq!(probed["kind"], "dng");
    assert_eq!(probed["width"], 130);
    assert_eq!(probed["height"], 98);
    assert_eq!(probed["report"]["verdict"]["kind"], "fullResolution");
    // The planner is told the truth about this decoder.
    let region = &probed["report"]["regionCost"];
    assert!(region["openingPerPixel"].as_u64().unwrap() >= 34 + 16);
    assert!(
        probed["report"]["outOfCore"]
            .as_str()
            .unwrap()
            .contains("not available"),
        "out-of-core must not be claimed"
    );
}

/// The central claim, through the commands.
#[test]
fn a_region_opened_on_its_own_is_the_crop_of_the_whole_sensor() {
    let _isolated = Isolated::new();
    let directory = tempfile::tempdir().unwrap();
    for tile in [None, Some((32, 32)), Some((48, 16))] {
        let path = write(directory.path(), "frame.dng", &sensor(130, 98, tile));
        let harness = Harness::new();

        let whole = harness.ok("open_raw_image", json!({ "path": path, "requestId": 1 }));
        assert!(whole["source"].get("view").is_none(), "{tile:?}");
        let whole_png =
            export(&harness, directory.path(), "whole.png", &whole["source"]).to_rgba16();

        // Awkward on purpose: an odd origin, an odd size, touching or not touching
        // the edges.
        for window in [
            rect(33, 17, 41, 29),
            rect(0, 0, 25, 25),
            rect(101, 71, 29, 27),
        ] {
            let opened = harness.ok(
                "open_raw_image",
                json!({ "path": path, "requestId": 2, "region": window }),
            );
            let (x, y, w, h) = (
                window["x"].as_u64().unwrap() as u32,
                window["y"].as_u64().unwrap() as u32,
                window["width"].as_u64().unwrap() as u32,
                window["height"].as_u64().unwrap() as u32,
            );
            assert_eq!(opened["metadata"]["width"], w);
            assert_eq!(opened["metadata"]["height"], h);
            // The layer says which part of which sensor it is.
            assert_eq!(opened["source"]["view"], window);
            assert_eq!(opened["source"]["reference"]["width"], 130);
            assert_eq!(opened["source"]["reference"]["height"], 98);
            assert_eq!(
                opened["source"]["reference"]["sha256"], whole["source"]["reference"]["sha256"],
                "the identity is that of the file"
            );
            // The white balance written back is the whole sensor's, not the window's.
            assert_eq!(
                opened["source"]["parameters"]["whiteBalance"],
                whole["source"]["parameters"]["whiteBalance"],
                "{tile:?} {window}"
            );

            let region_png =
                export(&harness, directory.path(), "region.png", &opened["source"]).to_rgba16();
            assert_eq!((region_png.width(), region_png.height()), (w, h));
            for row in 0..h {
                for column in 0..w {
                    assert_eq!(
                        region_png.get_pixel(column, row),
                        whole_png.get_pixel(x + column, y + row),
                        "{tile:?} {window} at ({column},{row})"
                    );
                }
            }
        }
    }
}

#[test]
fn an_oversized_sensor_opens_only_as_a_region_on_a_small_machine() {
    let _isolated = Isolated::new();
    let directory = tempfile::tempdir().unwrap();
    // 4096 x 3072 is 12.6 MP of sensor: more than a 512 MiB budget can develop.
    let path = write(
        directory.path(),
        "big.dng",
        &sensor(4096, 3072, Some((256, 256))),
    );
    // The budget a user chose is read from the settings directory, so a test
    // chooses one the same way: by pointing that directory at a temporary one.
    let settings_home = tempfile::tempdir().unwrap();
    std::env::set_var("LOCALAPPDATA", settings_home.path());
    resources::settings::save_mode(BudgetMode::Manual { bytes: 512 * MIB }).unwrap();
    let harness = Harness::new();

    let probed = harness.ok("probe_image_source", json!({ "path": path }));
    assert_eq!(
        probed["report"]["verdict"]["kind"], "regionRequired",
        "{probed}"
    );
    let options = probed["report"]["options"].as_array().unwrap();
    let ceiling = options
        .iter()
        .find(|option| option["kind"] == "openRegion")
        .expect("a region is offered")["maxRegionPixels"]
        .as_u64()
        .unwrap();
    assert!(ceiling >= 500_000, "a useless ceiling: {ceiling}");
    assert!(ceiling < 4096 * 3072);
    assert!(
        options.iter().all(|option| option["kind"] != "openReduced"),
        "a RAW is not offered a reduced copy"
    );

    // Whole is refused — by the planner, not by running out of memory.
    harness.err("open_raw_image", json!({ "path": path, "requestId": 1 }));

    // A region inside the ceiling opens.
    let side = (ceiling as f64).sqrt() as u32 / 2;
    let ok = harness.ok(
        "open_raw_image",
        json!({ "path": path, "requestId": 2, "region": rect(1000, 700, side, side) }),
    );
    assert_eq!(ok["metadata"]["width"], side);
    assert_eq!(ok["source"]["reference"]["width"], 4096);

    // A region beyond the ceiling is refused, as is one off the sensor, an empty
    // one, and one whose far edge would wrap a 32-bit sum.
    harness.err(
        "open_raw_image",
        json!({ "path": path, "requestId": 3, "region": rect(0, 0, 4096, 3072) }),
    );
    harness.err(
        "open_raw_image",
        json!({ "path": path, "requestId": 4, "region": rect(4000, 3000, 200, 200) }),
    );
    harness.err(
        "open_raw_image",
        json!({ "path": path, "requestId": 5, "region": rect(10, 10, 0, 50) }),
    );
    harness.err(
        "open_raw_image",
        json!({ "path": path, "requestId": 6, "region": rect(u32::MAX - 5, 0, 100, 100) }),
    );

    // The preview of the whole sensor is bounded and small.
    let preview = harness.ok(
        "source_preview_image",
        json!({ "path": path, "maxEdge": 512 }),
    );
    assert!(preview["width"].as_u64().unwrap() <= 514);
    assert!(preview["height"].as_u64().unwrap() <= 514);
    assert!(preview["dataUrl"]
        .as_str()
        .unwrap()
        .starts_with("data:image/"));

    // The same file with the budget left to the machine opens whole.
    resources::settings::save_mode(BudgetMode::Automatic).unwrap();
    let probed = harness.ok("probe_image_source", json!({ "path": path }));
    assert!(
        matches!(
            probed["report"]["verdict"]["kind"].as_str().unwrap(),
            "fullResolution" | "fullResolutionWithWarning"
        ),
        "{probed}"
    );
}

#[test]
fn a_region_layer_is_redeveloped_as_the_same_rectangle() {
    let _isolated = Isolated::new();
    let directory = tempfile::tempdir().unwrap();
    let path = write(
        directory.path(),
        "frame.dng",
        &sensor(130, 98, Some((32, 32))),
    );
    let harness = Harness::new();
    let window = rect(40, 20, 50, 36);
    let opened = harness.ok(
        "open_raw_image",
        json!({ "path": path, "requestId": 7, "region": window }),
    );
    let source = opened["source"].clone();

    for full_resolution in [false, true] {
        let mut parameters = source["parameters"].clone();
        parameters["exposureEv"] = json!(0.8);
        let redeveloped = harness.ok(
            "develop_raw_layer",
            json!({
                "request": {
                    "source": source,
                    "parameters": parameters,
                    "fullResolution": full_resolution,
                    "documentId": 7,
                    "requestId": 100 + u64::from(full_resolution)
                }
            }),
        );
        // A region layer is the rectangle, at full resolution, whatever the
        // interface asked for; a decimated preview would not line up with it.
        assert_eq!(redeveloped["width"], 50, "fullResolution={full_resolution}");
        assert_eq!(redeveloped["height"], 36);
        assert_eq!(redeveloped["source"]["view"], window);
        assert_eq!(redeveloped["sourceWidth"], 130);
    }
}

#[test]
fn a_changed_file_is_not_silently_rebound_to_a_region() {
    let _isolated = Isolated::new();
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "frame.dng", &sensor(130, 98, None));
    let harness = Harness::new();
    let opened = harness.ok(
        "open_raw_image",
        json!({ "path": path, "requestId": 9, "region": rect(10, 10, 40, 40) }),
    );
    // Replace the file with a different photograph of the same size.
    let mut altered = sensor(130, 98, None);
    let last = altered.len() - 1;
    altered[last] ^= 0xFF;
    std::fs::write(&path, altered).unwrap();

    let message = harness.err(
        "develop_raw_layer",
        json!({
            "request": {
                "source": opened["source"].clone(),
                "fullResolution": true,
                "documentId": 9,
                "requestId": 10
            }
        }),
    );
    assert!(message.contains("not the photograph"), "{message}");
}

/// A project that holds a region of a sensor keeps *which* region, and rebuilds the
/// same rectangle from the same file when it is opened again.
#[test]
fn a_region_backed_project_survives_a_save_and_reload() {
    use photoforge_lib::layers::{
        decode_project, encode_project, BlendMode, Layer, LayerContent, LayerDocument,
        LayerMetadata, LayerTransform,
    };

    let _isolated = Isolated::new();
    let directory = tempfile::tempdir().unwrap();
    let path = write(
        directory.path(),
        "frame.dng",
        &sensor(130, 98, Some((32, 32))),
    );
    let harness = Harness::new();
    let window = rect(33, 17, 41, 29);
    let opened = harness.ok(
        "open_raw_image",
        json!({ "path": path, "requestId": 11, "region": window }),
    );
    let source: photoforge_lib::raw::RawLayerSource =
        serde_json::from_value(opened["source"].clone()).unwrap();
    let pixel_id = opened["backgroundPixelId"].as_str().unwrap().to_string();

    let layer = |view_matches_raster: bool| {
        let mut source = source.clone();
        if !view_matches_raster {
            // A raster whose size the source could not reproduce.
            source.view = Some(photoforge_lib::source::Rect {
                x: 33,
                y: 17,
                width: 40,
                height: 29,
            });
        }
        Layer {
            id: "region".into(),
            name: "RAW region".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            transform: LayerTransform::default(),
            mask: None,
            collapsed: false,
            metadata: LayerMetadata::default(),
            raw: Some(source),
            origin: None,
            content: LayerContent::Pixel {
                pixel_id: pixel_id.clone(),
                width: 41,
                height: 29,
            },
        }
    };
    let document = |layer: Layer| LayerDocument {
        schema_version: photoforge_lib::layers::LAYER_SCHEMA_VERSION,
        precision: Default::default(),
        canvas_width: 41,
        canvas_height: 29,
        layers: vec![layer],
        smart_sources: Default::default(),
        active_layer_id: Some("region".into()),
    };
    let raster = image::RgbaImage::from_pixel(41, 29, image::Rgba([10, 20, 30, 255]));
    let buffers = vec![(pixel_id.clone(), &raster)];

    let encoded = encode_project(
        &document(layer(true)),
        &[],
        &buffers,
        "0.14.0",
        "2026-10-06T00:00:00Z",
        "2026-10-06T00:00:00Z",
    )
    .expect("a consistent region layer saves");
    let restored = decode_project(&encoded).expect("and loads");
    let back = restored
        .document
        .find("region")
        .unwrap()
        .raw
        .clone()
        .unwrap();
    assert_eq!(back.view, source.view);
    assert_eq!(back.reference, source.reference);

    // The reloaded layer rebuilds the very same rectangle from the file.
    let again = harness.ok(
        "develop_raw_layer",
        json!({
            "request": {
                "source": serde_json::to_value(&back).unwrap(),
                "fullResolution": true,
                "documentId": 11,
                "requestId": 12
            }
        }),
    );
    assert_eq!(again["width"], 41);
    assert_eq!(again["height"], 29);
    assert_eq!(again["source"]["view"], window);

    // A raster that is not the size of its region is refused outright, rather than
    // saved as something the source could never reproduce.
    assert!(encode_project(
        &document(layer(false)),
        &[],
        &buffers,
        "0.14.0",
        "2026-10-06T00:00:00Z",
        "2026-10-06T00:00:00Z",
    )
    .is_err());
}
