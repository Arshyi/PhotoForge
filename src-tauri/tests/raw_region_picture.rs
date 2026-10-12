//! A region of a camera RAW is chosen on the picture the editor shows: the part of the sensor the
//! camera declares (its default crop inside the active area), turned upright by its orientation
//! tag. The rectangle is the picture's own, it is mapped back to the photosites it covers, and the
//! region is developed and turned the way a whole open is.
//!
//! The synthetic sensor is 32 x 24 with the active area (top 2, left 4, bottom 22, right 30) and a
//! default crop of 20 x 12 at (2, 2) inside it, so the picture is the rectangle (6, 4, 20, 12) of
//! the sensor, 20 x 12 upright for orientations 1 to 4 and 12 x 20 for 5 to 8.
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

/// A gradient sensor with the picture of the module comment declared.
fn framed(orientation: u16) -> Vec<u8> {
    sensor(WIDTH, HEIGHT, orientation, [2, 4, 22, 30], [2, 2, 20, 12])
}

fn sensor(width: u32, height: u32, orientation: u16, area: [u32; 4], crop: [u32; 4]) -> Vec<u8> {
    let samples: Vec<u16> = (0..width * height)
        .map(|index| 300 + ((index % width) * 61 + (index / width) * 37) as u16 % 2500)
        .collect();
    DngBuilder::new(width, height, samples)
        .levels(vec![0], (1, 1), 4095)
        .neutral([0.55, 1.0, 0.72])
        .matrix([0.72, 0.18, 0.10, 0.16, 0.74, 0.10, 0.09, 0.20, 0.71])
        .orientation(orientation)
        .entry(50829, 4, longs(&area))
        .entry(50719, 4, longs(&[crop[0], crop[1]]))
        .entry(50720, 4, longs(&[crop[2], crop[3]]))
        .build()
}

fn write(directory: &std::path::Path, name: &str, bytes: &[u8]) -> String {
    let path = directory.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

fn export(
    harness: &Harness,
    directory: &std::path::Path,
    name: &str,
    source: &Value,
) -> image::ImageBuffer<image::Rgba<u16>, Vec<u16>> {
    let output = directory.join(name);
    harness.ok(
        "export_raw_layer_png16",
        json!({ "outputPath": output.to_string_lossy(), "source": source }),
    );
    image::open(&output)
        .expect("the export decodes")
        .to_rgba16()
}

#[test]
fn a_region_of_the_picture_is_that_window_of_the_whole_picture_for_every_orientation() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    // Odd offset and size on purpose; inside both 20 x 12 and 12 x 20.
    let (x, y, width, height) = (3_u32, 1_u32, 7_u32, 5_u32);
    for orientation in 1..=8_u16 {
        let path = write(
            directory.path(),
            &format!("frame-{orientation}.dng"),
            &framed(orientation),
        );
        let whole_open = harness.ok(
            "open_raw_image",
            json!({ "path": path, "requestId": 100 + u64::from(orientation) }),
        );
        let (picture_width, picture_height) = if orientation >= 5 { (12, 20) } else { (20, 12) };
        assert_eq!(whole_open["metadata"]["width"], picture_width);
        assert_eq!(whole_open["metadata"]["height"], picture_height);
        let whole = export(
            &harness,
            directory.path(),
            &format!("whole-{orientation}.png"),
            &whole_open["source"],
        );
        assert_eq!(whole.dimensions(), (picture_width, picture_height));

        let region_open = harness.ok(
            "open_raw_image",
            json!({
                "path": path,
                "requestId": 200 + u64::from(orientation),
                "region": { "x": x, "y": y, "width": width, "height": height },
            }),
        );
        // The document is the rectangle chosen, the way up the picture shows it.
        assert_eq!(region_open["metadata"]["width"], width);
        assert_eq!(region_open["metadata"]["height"], height);
        let view = &region_open["source"]["view"];
        // Recorded as photosites of the sensor, inside the picture (6, 4, 20, 12) on it.
        let (view_x, view_y) = (view["x"].as_u64().unwrap(), view["y"].as_u64().unwrap());
        let (view_w, view_h) = (
            view["width"].as_u64().unwrap(),
            view["height"].as_u64().unwrap(),
        );
        assert!(view_x >= 6 && view_y >= 4 && view_x + view_w <= 26 && view_y + view_h <= 16);
        assert_eq!(view_w * view_h, u64::from(width * height));
        if orientation == 1 {
            assert!(region_open["source"].get("orientationApplied").is_none());
        } else {
            assert_eq!(region_open["source"]["orientationApplied"], orientation);
        }

        let region = export(
            &harness,
            directory.path(),
            &format!("region-{orientation}.png"),
            &region_open["source"],
        );
        assert_eq!(
            region.dimensions(),
            (width, height),
            "orientation {orientation}"
        );
        for row in 0..height {
            for column in 0..width {
                assert_eq!(
                    region.get_pixel(column, row),
                    whole.get_pixel(x + column, y + row),
                    "orientation {orientation} ({column}, {row})"
                );
            }
        }
    }
}

#[test]
fn developing_a_region_layer_again_keeps_its_size_and_its_turn() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "portrait.dng", &framed(6));
    let harness = Harness::new();
    let opened = harness.ok(
        "open_raw_image",
        json!({
            "path": path,
            "requestId": 4,
            "region": { "x": 2, "y": 3, "width": 8, "height": 5 },
        }),
    );
    let source = opened["source"].clone();
    assert_eq!(source["orientationApplied"], 6);
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
        // A region is developed at full resolution whatever was asked: it is the rectangle.
        assert_eq!(again["width"], 8, "fullResolution={full_resolution}");
        assert_eq!(again["height"], 5);
        assert_eq!(again["source"]["orientationApplied"], 6);
        assert_eq!(again["source"]["view"], source["view"]);
        assert_eq!(again["sourceWidth"], WIDTH);
    }
}

#[test]
fn a_region_layer_recorded_before_regions_were_turned_is_developed_as_it_was() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "portrait.dng", &framed(6));
    let harness = Harness::new();
    let opened = harness.ok(
        "open_raw_image",
        json!({
            "path": path,
            "requestId": 5,
            "region": { "x": 2, "y": 3, "width": 8, "height": 5 },
        }),
    );
    // What an earlier build wrote for a region: a rectangle of the sensor and no turn.
    let mut source = opened["source"].clone();
    source.as_object_mut().unwrap().remove("orientationApplied");
    // The photosites it covers are 5 wide and 8 tall; a quarter turn is what made them 8 by 5.
    assert_eq!(source["view"]["width"], 5);
    assert_eq!(source["view"]["height"], 8);

    let again = harness.ok(
        "develop_raw_layer",
        json!({
            "request": { "source": source, "documentId": 5, "requestId": 50 }
        }),
    );
    assert_eq!(again["width"], 5);
    assert_eq!(again["height"], 8);
    assert!(again["source"].get("orientationApplied").is_none());

    let out = directory.path().join("old.png");
    harness.ok(
        "export_raw_layer_png16",
        json!({ "outputPath": out.to_string_lossy(), "source": source }),
    );
    assert_eq!(
        image::GenericImageView::dimensions(&image::open(&out).unwrap()),
        (5, 8)
    );
}

#[test]
fn a_region_that_is_not_on_the_picture_is_refused() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "portrait.dng", &framed(6));
    let harness = Harness::new();
    // The upright picture is 12 wide and 20 tall. 16 is inside the sensor (32) and the sensor
    // turned (24), but not the picture.
    for (x, y, width, height) in [(8, 0, 8, 5), (0, 16, 4, 8), (0, 0, 13, 5), (0, 0, 0, 5)] {
        let outcome = harness.call(
            "open_raw_image",
            json!({
                "path": path,
                "requestId": 7,
                "region": { "x": x, "y": y, "width": width, "height": height },
            }),
        );
        assert!(
            outcome.is_err(),
            "({x}, {y}, {width}, {height}): {outcome:?}"
        );
    }
    // The whole picture is on it.
    let whole = harness.ok(
        "open_raw_image",
        json!({
            "path": path,
            "requestId": 8,
            "region": { "x": 0, "y": 0, "width": 12, "height": 20 },
        }),
    );
    assert_eq!(whole["metadata"]["width"], 12);
    assert_eq!(whole["metadata"]["height"], 20);
    assert_eq!(
        whole["source"]["view"],
        json!({ "x": 6, "y": 4, "width": 20, "height": 12 })
    );
}

#[test]
fn the_probe_reports_the_picture_it_is_chosen_on() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    for (orientation, expected) in [
        (1_u16, (20, 12)),
        (6, (12, 20)),
        (3, (20, 12)),
        (8, (12, 20)),
    ] {
        let path = write(
            directory.path(),
            &format!("p-{orientation}.dng"),
            &framed(orientation),
        );
        let probed = harness.ok("probe_image_source", json!({ "path": path }));
        assert_eq!(probed["kind"], "dng");
        assert_eq!(
            (
                probed["width"].as_u64().unwrap(),
                probed["height"].as_u64().unwrap()
            ),
            expected,
            "orientation {orientation}"
        );
    }
    // A file that declares no picture and no turn is as large as its sensor.
    let plain = DngBuilder::new(WIDTH, HEIGHT, vec![1000; (WIDTH * HEIGHT) as usize])
        .levels(vec![0], (1, 1), 4095)
        .build();
    let path = write(directory.path(), "plain.dng", &plain);
    let probed = harness.ok("probe_image_source", json!({ "path": path }));
    assert_eq!(probed["width"], WIDTH);
    assert_eq!(probed["height"], HEIGHT);
}

#[test]
fn the_preview_a_region_is_chosen_on_is_the_picture() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::new();

    // Small enough to be developed whole: the preview is the picture to the pixel.
    let path = write(directory.path(), "small.dng", &framed(6));
    let preview = harness.ok(
        "source_preview_image",
        json!({ "path": path, "maxEdge": 256 }),
    );
    assert_eq!(preview["width"], 12);
    assert_eq!(preview["height"], 20);

    // Large enough to be decimated: cut to the picture in proportion, and turned. The picture is
    // 320 x 240 at (30, 20) on a 400 x 300 sensor, so upright (a quarter turn) it is 240 x 320.
    let big = sensor(400, 300, 6, [10, 20, 290, 380], [10, 10, 320, 240]);
    let path = write(directory.path(), "big.dng", &big);
    let probed = harness.ok("probe_image_source", json!({ "path": path }));
    assert_eq!(probed["width"], 240);
    assert_eq!(probed["height"], 320);
    let preview = harness.ok(
        "source_preview_image",
        json!({ "path": path, "maxEdge": 64 }),
    );
    let (width, height) = (
        preview["width"].as_f64().unwrap(),
        preview["height"].as_f64().unwrap(),
    );
    assert!(width < height, "portrait: {width} x {height}");
    assert!(width >= 16.0 && height >= 16.0, "{width} x {height}");
    // The preview has the picture's proportions (240 / 320), to the rounding of a small image.
    let ratio = width / height;
    assert!((ratio - 0.75).abs() < 0.06, "{width} x {height}: {ratio}");
}

#[test]
fn where_a_region_lies_on_the_picture_is_found_from_its_photosites() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(directory.path(), "portrait.dng", &framed(6));
    let harness = Harness::new();
    let chosen = json!({ "x": 2, "y": 3, "width": 8, "height": 5 });
    let opened = harness.ok(
        "open_raw_image",
        json!({ "path": path, "requestId": 9, "region": chosen }),
    );
    let view = opened["source"]["view"].clone();

    // From the photosites the layer records back to what was chosen on the picture.
    let back = harness.ok(
        "raw_region_on_picture",
        json!({ "path": path, "view": view }),
    );
    assert_eq!(back, chosen);

    // A region of the sensor that takes in the masked border has no place on the picture.
    let border = harness.ok(
        "raw_region_on_picture",
        json!({ "path": path, "view": { "x": 0, "y": 0, "width": 10, "height": 10 } }),
    );
    assert_eq!(border, Value::Null);
    // Nor does one off the sensor.
    let off = harness.ok(
        "raw_region_on_picture",
        json!({ "path": path, "view": { "x": 30, "y": 20, "width": 10, "height": 10 } }),
    );
    assert_eq!(off, Value::Null);
}
