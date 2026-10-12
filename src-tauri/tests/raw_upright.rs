//! A DNG developed whole is turned upright from its orientation tag; a region is chosen on the
//! upright picture and is turned with it.
//!
//! The synthetic sensors carry a bright block in the sensor's top-left corner and are otherwise
//! flat, so where the block lands says which way the picture was turned: the eight EXIF values put
//! the sensor's first row and first column at the top-left, top-right, bottom-right, bottom-left
//! (1 to 4) and, for the four that exchange the axes (5 to 8), at the same four corners.
use photoforge_lib::application::AppState;
use photoforge_lib::color::DevelopmentParameters;
use photoforge_lib::metadata::{exif, scan};
use photoforge_lib::raw::develop::{develop_bytes, develop_bytes_picture, RenderScale};
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

/// A flat sensor with a 4 × 4 block, three times as bright, in its top-left corner.
fn dng(orientation: u16) -> Vec<u8> {
    let mut samples = vec![1000_u16; (WIDTH * HEIGHT) as usize];
    for y in 0..4 {
        for x in 0..4 {
            samples[(y * WIDTH + x) as usize] = 3000;
        }
    }
    DngBuilder::new(WIDTH, HEIGHT, samples)
        .levels(vec![0], (1, 1), 4095)
        .orientation(orientation)
        .build()
}

/// Which corner the sensor's top-left block should be in after the turn, as (right, bottom).
fn corner(orientation: u16) -> (bool, bool) {
    match orientation {
        1 | 5 => (false, false),
        2 | 6 => (true, false),
        3 | 7 => (true, true),
        _ => (false, true),
    }
}

fn brightness(image: &photoforge_lib::color::FloatImage, x: u32, y: u32) -> f32 {
    image.get(x, y).expect("inside the picture").green
}

#[test]
fn the_eight_orientations_put_the_sensors_corner_where_exif_says() {
    let parameters = DevelopmentParameters::default();
    for orientation in 1..=8_u16 {
        let upright = develop_bytes_picture(&dng(orientation), &parameters, RenderScale::Full)
            .unwrap_or_else(|error| panic!("{orientation}: {error}"));
        let (width, height) = upright.image.dimensions();
        let swapped = orientation >= 5;
        assert_eq!(
            (width, height),
            if swapped {
                (HEIGHT, WIDTH)
            } else {
                (WIDTH, HEIGHT)
            },
            "orientation {orientation}"
        );
        // The sensor's own size is still said as the sensor has it.
        assert_eq!(upright.source_dimensions, (WIDTH, HEIGHT));

        let (right, bottom) = corner(orientation);
        let at = |far: bool, size: u32| if far { size - 2 } else { 1 };
        let block = brightness(&upright.image, at(right, width), at(bottom, height));
        assert!(block > 0.6, "orientation {orientation}: block {block}");
        // The other three corners are the flat field.
        for (other_right, other_bottom) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            if (other_right, other_bottom) == (right, bottom) {
                continue;
            }
            let flat = brightness(
                &upright.image,
                at(other_right, width),
                at(other_bottom, height),
            );
            assert!(
                flat < 0.4,
                "orientation {orientation}: corner ({other_right}, {other_bottom}) is {flat}"
            );
        }
    }
}

#[test]
fn an_upright_sensor_is_unchanged_and_the_sensors_own_way_up_is_still_available() {
    let parameters = DevelopmentParameters::default();
    let data = dng(1);
    let own = develop_bytes(&data, &parameters, RenderScale::Full).unwrap();
    let upright = develop_bytes_picture(&data, &parameters, RenderScale::Full).unwrap();
    assert_eq!(own.image, upright.image);

    // A sideways one, in the sensor's own orientation, is the same picture before the turn.
    let sideways = dng(6);
    let own = develop_bytes(&sideways, &parameters, RenderScale::Full).unwrap();
    assert_eq!(own.image.dimensions(), (WIDTH, HEIGHT));
    let turned = develop_bytes_picture(&sideways, &parameters, RenderScale::Full).unwrap();
    assert_eq!(turned.image.dimensions(), (HEIGHT, WIDTH));
    let (width, height, pixels) = own.image.into_parts();
    let by_hand = photoforge_lib::orientation::turn(pixels, 6, width, height).unwrap();
    assert_eq!(turned.image.pixels(), by_hand.as_slice());
}

fn write(directory: &std::path::Path, name: &str, bytes: &[u8]) -> String {
    let path = directory.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn a_portrait_dng_opens_as_a_portrait_and_exports_upright() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "portrait.dng", &dng(6));
    let harness = Harness::new();
    let opened = harness.ok("open_raw_image", json!({ "path": path, "requestId": 1 }));

    // The document is the upright picture: 24 wide and 32 tall.
    assert_eq!(opened["metadata"]["width"], HEIGHT);
    assert_eq!(opened["metadata"]["height"], WIDTH);
    // The sensor is still described as it is, and the layer says which turn it was given.
    assert_eq!(opened["source"]["reference"]["width"], WIDTH);
    assert_eq!(opened["source"]["reference"]["height"], HEIGHT);
    assert_eq!(opened["source"]["orientationApplied"], 6);
    // The tag is spent, and the Metadata card says the picture was turned.
    let summary = &opened["metadata"]["sourceMetadata"];
    assert_eq!(summary["orientation"], 6);
    assert_eq!(summary["orientationApplied"], true);

    // Exported, the file says upright, whatever the policy.
    for policy in ["keep_all", "remove_private", "remove_all"] {
        let out = directory.path().join(format!("out-{policy}.jpg"));
        harness.ok(
            "export_image",
            json!({
                "outputPath": out.to_string_lossy(),
                "operations": [],
                "metadataPolicy": policy,
            }),
        );
        let image = image::open(&out).unwrap();
        assert_eq!(
            image::GenericImageView::dimensions(&image),
            (HEIGHT, WIDTH),
            "{policy}"
        );
        let found = scan::read(&out).unwrap();
        match found.exif.as_deref() {
            Some(block) => assert_eq!(
                exif::summarize(block).unwrap().orientation,
                1,
                "{policy}: the picture is upright and says so"
            ),
            None => assert_eq!(policy, "remove_all"),
        }
    }
}

#[test]
fn developing_the_layer_again_keeps_its_size_and_its_turn() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "portrait.dng", &dng(8));
    let harness = Harness::new();
    let opened = harness.ok("open_raw_image", json!({ "path": path, "requestId": 4 }));
    let source = opened["source"].clone();
    assert_eq!(source["orientationApplied"], 8);

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
                    "documentId": 4,
                    "requestId": 40 + u64::from(full_resolution),
                }
            }),
        );
        // Upright size, so the document's masks and transform still fit it.
        assert_eq!(again["width"], HEIGHT, "fullResolution={full_resolution}");
        assert_eq!(again["height"], WIDTH);
        assert_eq!(again["source"]["orientationApplied"], 8);
        assert_eq!(again["sourceWidth"], WIDTH);
    }

    // The export of the layer is the upright picture too.
    let out = directory.path().join("layer.png");
    harness.ok(
        "export_raw_layer_png16",
        json!({ "outputPath": out.to_string_lossy(), "source": source }),
    );
    assert_eq!(
        image::GenericImageView::dimensions(&image::open(&out).unwrap()),
        (HEIGHT, WIDTH)
    );
}

#[test]
fn a_layer_recorded_before_the_turn_existed_is_developed_as_it_was() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "portrait.dng", &dng(6));
    let harness = Harness::new();
    let opened = harness.ok("open_raw_image", json!({ "path": path, "requestId": 5 }));
    // What an earlier build wrote: no record of a turn.
    let mut source = opened["source"].clone();
    source.as_object_mut().unwrap().remove("orientationApplied");

    let again = harness.ok(
        "develop_raw_layer",
        json!({
            "request": {
                "source": source,
                "fullResolution": true,
                "documentId": 5,
                "requestId": 50,
            }
        }),
    );
    // The raster keeps the shape the project stored: the sensor's own, not turned.
    assert_eq!(again["width"], WIDTH);
    assert_eq!(again["height"], HEIGHT);
    assert!(again["source"].get("orientationApplied").is_none());
}

#[test]
fn a_region_is_chosen_on_the_upright_picture_and_is_turned_with_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "portrait.dng", &dng(6));
    let harness = Harness::new();
    // The upright picture is 24 wide and 32 tall; this is 20 by 10 of it.
    let window = json!({ "x": 2, "y": 2, "width": 20, "height": 10 });
    let opened = harness.ok(
        "open_raw_image",
        json!({ "path": path, "requestId": 6, "region": window }),
    );
    // The document is the rectangle that was chosen, the way up the picture shows.
    assert_eq!(opened["metadata"]["width"], 20);
    assert_eq!(opened["metadata"]["height"], 10);
    // The layer records the photosites it is (a quarter turn exchanges the axes: 10 wide and 20
    // tall on the sensor) and the turn it was given.
    assert_eq!(
        opened["source"]["view"],
        json!({ "x": 2, "y": 2, "width": 10, "height": 20 })
    );
    assert_eq!(opened["source"]["orientationApplied"], 6);
    // So the tag is spent for it as it is for a whole sensor.
    let summary = &opened["metadata"]["sourceMetadata"];
    assert_eq!(summary["orientation"], 6);
    assert_eq!(summary["orientationApplied"], true);

    let out = directory.path().join("region.jpg");
    harness.ok(
        "export_image",
        json!({
            "outputPath": out.to_string_lossy(),
            "operations": [],
            "metadataPolicy": "remove_all",
        }),
    );
    // Nothing is written that could turn it again.
    let block = scan::read(&out).and_then(|found| found.exif);
    let orientation = block
        .as_deref()
        .and_then(|bytes| exif::summarize(bytes))
        .map(|summary| summary.orientation);
    assert!(
        orientation.is_none_or(|value| value == 1),
        "{orientation:?}"
    );
}

#[test]
fn a_raw_layer_added_to_a_document_is_upright_and_says_so() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "portrait.dng", &dng(3));
    let harness = Harness::new();
    // A document to add it to.
    harness.ok(
        "open_raw_image",
        json!({ "path": write(directory.path(), "base.dng", &dng(1)), "requestId": 7 }),
    );
    let layer = harness.ok(
        "open_raw_layer",
        json!({ "path": path, "fullResolution": true }),
    );
    // A half turn keeps the size, and the layer records it.
    assert_eq!(layer["width"], WIDTH);
    assert_eq!(layer["height"], HEIGHT);
    assert_eq!(layer["source"]["orientationApplied"], 3);

    let portrait = write(directory.path(), "tall.dng", &dng(6));
    let tall = harness.ok(
        "open_raw_layer",
        json!({ "path": portrait, "fullResolution": true }),
    );
    assert_eq!(tall["width"], HEIGHT);
    assert_eq!(tall["height"], WIDTH);
    assert_eq!(tall["source"]["orientationApplied"], 6);
}

/// A project that records a turn is a version an earlier build refuses by name (it would
/// otherwise stop at an unknown field), and one that records none is the version it always was.
#[test]
fn a_project_that_records_a_turn_is_version_three_and_one_that_does_not_is_version_two() {
    use photoforge_lib::layers::{
        decode_project, encode_project, BlendMode, Layer, LayerContent, LayerDocument,
        LayerMetadata, LayerTransform,
    };

    for (orientation, version, expected_turn) in [(6_u16, 3_u32, 6_u64), (1, 2, 1)] {
        let directory = tempfile::tempdir().unwrap();
        let path = write(directory.path(), "frame.dng", &dng(orientation));
        let harness = Harness::new();
        let opened = harness.ok("open_raw_image", json!({ "path": path, "requestId": 3 }));
        let source: photoforge_lib::raw::RawLayerSource =
            serde_json::from_value(opened["source"].clone()).unwrap();
        assert_eq!(u64::from(source.orientation_applied), expected_turn);
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
            raw: Some(source),
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
        // The container: 8 bytes of magic, then the format version.
        let found = u32::from_le_bytes([encoded[8], encoded[9], encoded[10], encoded[11]]);
        assert_eq!(found, version, "orientation {orientation}");
        let restored = decode_project(&encoded).expect("and loads");
        let back = restored.document.layers[0].raw.as_ref().unwrap();
        assert_eq!(u64::from(back.orientation_applied), expected_turn);
    }
}
