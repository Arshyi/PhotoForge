//! A DNG developed whole is cut to the picture the camera says it holds: the default crop inside
//! its active area, without the masked border around the image. A region is chosen on that
//! picture, and is made of the photosites it covers (`raw_region_picture.rs`).
//!
//! The synthetic sensor is 32 × 24 with the active area (top 2, left 4, bottom 22, right 30) and a
//! default crop of 20 × 12 at (2, 2) inside it, so the picture is the rectangle (6, 4, 20, 12) of
//! the sensor.
use photoforge_lib::application::AppState;
use photoforge_lib::raw::dng::fixtures::DngBuilder;
use serde_json::{json, Value};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

const WIDTH: u32 = 32;
const HEIGHT: u32 = 24;

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
}

fn longs(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// A gradient sensor; `declared` says whether the active area and default crop are written.
fn dng(orientation: u16, declared: bool) -> Vec<u8> {
    let samples: Vec<u16> = (0..WIDTH * HEIGHT)
        .map(|index| 300 + ((index % WIDTH) * 61 + (index / WIDTH) * 37) as u16 % 2500)
        .collect();
    let mut builder = DngBuilder::new(WIDTH, HEIGHT, samples)
        .levels(vec![0], (1, 1), 4095)
        .orientation(orientation);
    if declared {
        builder = builder
            .entry(50829, 4, longs(&[2, 4, 22, 30]))
            .entry(50719, 4, longs(&[2, 2]))
            .entry(50720, 4, longs(&[20, 12]));
    }
    builder.build()
}

fn write(directory: &std::path::Path, name: &str, bytes: &[u8]) -> String {
    let path = directory.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

const PICTURE: [u32; 4] = [6, 4, 20, 12];

fn assert_picture(crop: &Value) {
    assert_eq!(
        [
            crop["x"].as_u64().unwrap() as u32,
            crop["y"].as_u64().unwrap() as u32,
            crop["width"].as_u64().unwrap() as u32,
            crop["height"].as_u64().unwrap() as u32,
        ],
        PICTURE
    );
}

#[test]
fn a_whole_sensor_opens_as_the_picture_the_camera_declares() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "frame.dng", &dng(1, true));
    let harness = Harness::new();
    let opened = harness.ok("open_raw_image", json!({ "path": path, "requestId": 1 }));
    // 20 × 12 of the 32 × 24 sensor.
    assert_eq!(opened["metadata"]["width"], 20);
    assert_eq!(opened["metadata"]["height"], 12);
    assert_picture(&opened["source"]["crop"]);
    // The sensor is still described as it is.
    assert_eq!(opened["source"]["reference"]["width"], WIDTH);
    assert_eq!(opened["source"]["reference"]["height"], HEIGHT);
    assert!(opened["source"].get("view").is_none());

    // The export is the picture: its size, and nothing of the border.
    let out = directory.path().join("picture.jpg");
    harness.ok(
        "export_image",
        json!({ "outputPath": out.to_string_lossy(), "operations": [], "metadataPolicy": "remove_all" }),
    );
    assert_eq!(
        image::GenericImageView::dimensions(&image::open(&out).unwrap()),
        (20, 12)
    );
}

#[test]
fn the_cut_comes_before_the_turn() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "portrait.dng", &dng(6, true));
    let harness = Harness::new();
    let opened = harness.ok("open_raw_image", json!({ "path": path, "requestId": 2 }));
    // The 20 × 12 picture, turned a quarter: 12 × 20.
    assert_eq!(opened["metadata"]["width"], 12);
    assert_eq!(opened["metadata"]["height"], 20);
    // The record is in the sensor's coordinates, whichever way the picture was then turned.
    assert_picture(&opened["source"]["crop"]);
    assert_eq!(opened["source"]["orientationApplied"], 6);
}

#[test]
fn developing_the_layer_again_cuts_the_same_rectangle() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "frame.dng", &dng(8, true));
    let harness = Harness::new();
    let opened = harness.ok("open_raw_image", json!({ "path": path, "requestId": 3 }));
    let source = opened["source"].clone();

    for full_resolution in [false, true] {
        let mut parameters = source["parameters"].clone();
        parameters["exposureEv"] = json!(0.5);
        let again = harness.ok(
            "develop_raw_layer",
            json!({
                "request": {
                    "source": source,
                    "parameters": parameters,
                    "fullResolution": full_resolution,
                    "documentId": 3,
                    "requestId": 30 + u64::from(full_resolution),
                }
            }),
        );
        // 12 × 20: the picture, turned by its orientation.
        assert_eq!(again["width"], 12, "fullResolution={full_resolution}");
        assert_eq!(again["height"], 20);
        assert_picture(&again["source"]["crop"]);
    }

    let out = directory.path().join("layer.png");
    harness.ok(
        "export_raw_layer_png16",
        json!({ "outputPath": out.to_string_lossy(), "source": source }),
    );
    assert_eq!(
        image::GenericImageView::dimensions(&image::open(&out).unwrap()),
        (12, 20)
    );
}

#[test]
fn a_layer_recorded_before_the_cut_existed_is_the_whole_sensor_as_it_was() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "frame.dng", &dng(1, true));
    let harness = Harness::new();
    let opened = harness.ok("open_raw_image", json!({ "path": path, "requestId": 4 }));
    // What an earlier build wrote: the whole sensor, with no record of a cut.
    let mut source = opened["source"].clone();
    source.as_object_mut().unwrap().remove("crop");

    let again = harness.ok(
        "develop_raw_layer",
        json!({
            "request": {
                "source": source,
                "fullResolution": true,
                "documentId": 4,
                "requestId": 40,
            }
        }),
    );
    assert_eq!(again["width"], WIDTH);
    assert_eq!(again["height"], HEIGHT);
    assert!(again["source"].get("crop").is_none());
}

#[test]
fn a_region_is_chosen_on_the_picture_and_recorded_as_the_photosites_it_is() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "frame.dng", &dng(1, true));
    let harness = Harness::new();
    // The top-left corner of the picture, which is (6, 4) on the sensor.
    let opened = harness.ok(
        "open_raw_image",
        json!({
            "path": path,
            "requestId": 5,
            "region": { "x": 0, "y": 0, "width": 12, "height": 10 },
        }),
    );
    assert_eq!(opened["metadata"]["width"], 12);
    assert_eq!(opened["metadata"]["height"], 10);
    // The region is the cut, so the layer records no crop of its own, only where it is.
    assert!(opened["source"].get("crop").is_none());
    assert_eq!(
        opened["source"]["view"],
        json!({ "x": 6, "y": 4, "width": 12, "height": 10 })
    );

    // The picture is 20 x 12, so this rectangle (as chosen on a sensor 32 wide) is not on it.
    let outside = harness.call(
        "open_raw_image",
        json!({
            "path": path,
            "requestId": 6,
            "region": { "x": 10, "y": 0, "width": 12, "height": 10 },
        }),
    );
    assert!(outside.is_err(), "{outside:?}");
}

#[test]
fn a_sensor_that_declares_no_picture_is_developed_whole() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "plain.dng", &dng(1, false));
    let harness = Harness::new();
    let opened = harness.ok("open_raw_image", json!({ "path": path, "requestId": 6 }));
    assert_eq!(opened["metadata"]["width"], WIDTH);
    assert_eq!(opened["metadata"]["height"], HEIGHT);
    assert!(opened["source"].get("crop").is_none());
}

#[test]
fn a_raw_layer_added_to_a_document_is_cut_and_says_so() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    harness.ok(
        "open_raw_image",
        json!({ "path": write(directory.path(), "base.dng", &dng(1, false)), "requestId": 7 }),
    );
    let layer = harness.ok(
        "open_raw_layer",
        json!({ "path": write(directory.path(), "framed.dng", &dng(1, true)), "fullResolution": true }),
    );
    assert_eq!(layer["width"], 20);
    assert_eq!(layer["height"], 12);
    assert_picture(&layer["source"]["crop"]);
}

#[test]
fn a_record_that_cannot_be_true_is_refused_not_developed() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "frame.dng", &dng(1, true));
    let harness = Harness::new();
    let opened = harness.ok("open_raw_image", json!({ "path": path, "requestId": 8 }));

    let refused = |crop: Value, view: Option<Value>| {
        let mut source = opened["source"].clone();
        source["crop"] = crop;
        if let Some(view) = view {
            source["view"] = view;
        }
        harness
            .call(
                "develop_raw_layer",
                json!({
                    "request": {
                        "source": source,
                        "fullResolution": true,
                        "documentId": 8,
                        "requestId": 80,
                    }
                }),
            )
            .expect_err("the record is refused")
    };
    // Outside the sensor, and empty.
    let message = refused(json!({ "x": 20, "y": 0, "width": 40, "height": 10 }), None);
    assert!(message.contains("outside the sensor"), "{message}");
    assert!(refused(json!({ "x": 0, "y": 0, "width": 0, "height": 10 }), None).contains("outside"));
    // A region is not cut.
    let message = refused(
        json!({ "x": 0, "y": 0, "width": 8, "height": 8 }),
        Some(json!({ "x": 0, "y": 0, "width": 8, "height": 8 })),
    );
    assert!(message.contains("not cut"), "{message}");
}

/// A project that records a cut is a version an earlier build refuses by name, and the record
/// comes back from a reload.
#[test]
fn a_project_that_records_a_cut_is_version_three_and_keeps_it() {
    use photoforge_lib::layers::{
        decode_project, encode_project, BlendMode, Layer, LayerContent, LayerDocument,
        LayerMetadata, LayerTransform,
    };

    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    for (declared, version) in [(true, 3_u32), (false, 2)] {
        let path = write(directory.path(), "frame.dng", &dng(1, declared));
        let opened = harness.ok("open_raw_image", json!({ "path": path, "requestId": 9 }));
        let source: photoforge_lib::raw::RawLayerSource =
            serde_json::from_value(opened["source"].clone()).unwrap();
        assert_eq!(source.crop.is_some(), declared);
        let width = opened["metadata"]["width"].as_u64().unwrap() as u32;
        let height = opened["metadata"]["height"].as_u64().unwrap() as u32;
        let pixel_id = opened["backgroundPixelId"].as_str().unwrap().to_string();
        let layer = Layer {
            id: "raw".into(),
            name: "RAW".into(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            transform: LayerTransform::default(),
            mask: None,
            collapsed: false,
            clipped: false,
            metadata: LayerMetadata::default(),
            raw: Some(source.clone()),
            origin: None,
            content: LayerContent::Pixel {
                pixel_id: pixel_id.clone(),
                width,
                height,
            },
        };
        let document = LayerDocument {
            schema_version: photoforge_lib::layers::LAYER_SCHEMA_VERSION,
            precision: Default::default(),
            canvas_width: width,
            canvas_height: height,
            layers: vec![layer],
            smart_sources: Default::default(),
            active_layer_id: Some("raw".into()),
        };
        let raster = image::RgbaImage::from_pixel(width, height, image::Rgba([9, 8, 7, 255]));
        let encoded = encode_project(
            &document,
            &[],
            &[(pixel_id, &raster)],
            "0.14.0",
            "2026-10-12T00:00:00Z",
            "2026-10-12T00:00:00Z",
        )
        .expect("the document saves");
        let found = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
        assert_eq!(found, version, "declared {declared}");
        let restored = decode_project(&encoded).expect("and loads");
        let back = restored.document.layers[0].raw.as_ref().unwrap();
        assert_eq!(back.crop, source.crop);
    }
}
