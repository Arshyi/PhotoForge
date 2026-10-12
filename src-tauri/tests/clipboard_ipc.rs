//! Copying and pasting through the real command boundary: the names, the arguments the interface
//! sends, what comes back, and the refusals.
//!
//! The refusals happen before the clipboard is touched, so they run always. Everything that puts a
//! picture on, or takes one from, the real clipboard replaces what the person has there and runs
//! only with `PHOTOFORGE_CLIPBOARD_TESTS=1`.
use photoforge_lib::application::AppState;
use photoforge_lib::layers::{LayerContent, LayerDocument, ShapeColor, ShapeContent};
use photoforge_lib::mask::{MaskBitmap, MaskSnapshot};
use photoforge_lib::operations::structure::new_layer;
use photoforge_lib::vector::{FillRule, ShapeGeometry};
use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

static CLIPBOARD: Mutex<()> = Mutex::new(());

fn real_clipboard_enabled() -> bool {
    std::env::var("PHOTOFORGE_CLIPBOARD_TESTS").is_ok_and(|v| v == "1")
}

struct Harness {
    webview: WebviewWindow<MockRuntime>,
    _app: tauri::App<MockRuntime>,
}

impl Harness {
    fn new() -> Self {
        let app = photoforge_lib::commands::register_layer_commands(mock_builder())
            .manage(AppState::default())
            .build(mock_context(noop_assets()))
            .expect("mock application");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview");
        Self { webview, _app: app }
    }

    fn call(&self, cmd: &str, body: Value) -> Result<Value, Value> {
        let response = tauri::test::get_ipc_response(
            &self.webview,
            InvokeRequest {
                cmd: cmd.into(),
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
            Err(error) => Err(error),
        }
    }
}

/// A 40 × 30 canvas with two shapes: a red rectangle at (10, 5) of 20 × 10, and a blue one
/// elsewhere.
fn document() -> Value {
    let rectangle = |id: &str, name: &str, x: f32, y: f32, w: f32, h: f32, colour: ShapeColor| {
        new_layer(
            id,
            name,
            LayerContent::Shape {
                shape: Box::new(ShapeContent {
                    geometry: ShapeGeometry::Rectangle {
                        x,
                        y,
                        width: w,
                        height: h,
                        corner_radius: 0.0,
                    },
                    fill: Some(colour),
                    stroke: None,
                    stroke_style: None,
                    fill_rule: FillRule::NonZero,
                }),
            },
            "t",
        )
    };
    let mut document = LayerDocument::new(40, 30);
    document.layers.push(rectangle(
        "red",
        "Red",
        10.0,
        5.0,
        20.0,
        10.0,
        ShapeColor::new(1.0, 0.0, 0.0, 1.0),
    ));
    document.layers.push(rectangle(
        "blue",
        "Blue",
        0.0,
        20.0,
        8.0,
        6.0,
        ShapeColor::new(0.0, 0.0, 1.0, 1.0),
    ));
    document.precision = photoforge_lib::pixel::DocumentPrecision::LinearSrgbF32;
    serde_json::to_value(&document).unwrap()
}

fn selection(width: u32, height: u32, covered: &[(u32, u32)]) -> Value {
    let mut mask = MaskBitmap::empty(width, height).unwrap();
    for &(x, y) in covered {
        mask.coverage_mut()[(y * width + x) as usize] = 255;
    }
    serde_json::to_value(MaskSnapshot::encode(&mask)).unwrap()
}

#[test]
fn what_cannot_be_copied_is_refused_before_the_clipboard_is_touched() {
    let harness = Harness::new();
    // A layer that is not in the document.
    let error = harness
        .call(
            "copy_to_clipboard",
            json!({ "document": document(), "operations": [], "layerId": "nope", "selection": null }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "layer_not_found", "{error}");

    // A selection of another size than the picture.
    let error = harness
        .call(
            "copy_to_clipboard",
            json!({
                "document": document(),
                "operations": [],
                "layerId": "red",
                "selection": selection(10, 10, &[(1, 1)]),
            }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "mask_dimension_mismatch", "{error}");

    // An empty selection has nothing to copy.
    let error = harness
        .call(
            "copy_to_clipboard",
            json!({
                "document": document(),
                "operations": [],
                "layerId": null,
                "selection": selection(40, 30, &[]),
            }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "clipboard", "{error}");
    assert!(error["message"]
        .as_str()
        .unwrap()
        .contains("selection is empty"));

    // A document that is not valid.
    let mut bad = document();
    bad["canvasWidth"] = json!(0);
    assert!(harness
        .call(
            "copy_to_clipboard",
            json!({ "document": bad, "operations": [], "layerId": null, "selection": null }),
        )
        .is_err());
}

#[test]
fn a_layer_is_copied_trimmed_and_a_picture_pastes_back_as_a_layer_and_as_a_document() {
    if !real_clipboard_enabled() {
        eprintln!("PHOTOFORGE_CLIPBOARD_TESTS is not 1: this test would replace the clipboard, so it does not run");
        return;
    }
    let _lock = CLIPBOARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let harness = Harness::new();

    // Copy the red layer: trimmed to its rectangle.
    let copied = harness
        .call(
            "copy_to_clipboard",
            json!({ "document": document(), "operations": [], "layerId": "red", "selection": null }),
        )
        .expect("the copy succeeds");
    assert_eq!(copied["scope"], "layer");
    assert_eq!(
        (copied["width"].as_u64(), copied["height"].as_u64()),
        (Some(20), Some(10))
    );
    assert_eq!(copied["selection"], false);
    assert_eq!(copied["formats"], json!(["PNG", "CF_DIBV5"]));
    let pasted = photoforge_lib::clipboard::read_image(1 << 20).unwrap();
    assert_eq!(pasted.image.dimensions(), (20, 10));
    assert_eq!(pasted.image.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(pasted.image.get_pixel(19, 9).0, [255, 0, 0, 255]);

    // Paste it into the session as a pixel buffer for a new layer.
    let layer = harness
        .call("paste_clipboard_image", json!({}))
        .expect("the paste succeeds");
    assert_eq!(
        (layer["width"].as_u64(), layer["height"].as_u64()),
        (Some(20), Some(10))
    );
    assert!(layer["pixelId"].as_str().unwrap().len() > 1);
    assert_eq!(layer["filename"], "Pasted image");
    assert_eq!(layer["format"], "png");
    assert_eq!(
        harness.call("clipboard_has_image", json!({})).unwrap(),
        json!(true)
    );

    // Open it as a document.
    let opened = harness
        .call("open_clipboard_image", json!({ "requestId": 1 }))
        .expect("the document opens");
    assert_eq!(opened["isCurrent"], true);
    assert_eq!(
        (
            opened["canvasWidth"].as_u64(),
            opened["canvasHeight"].as_u64()
        ),
        (Some(20), Some(10))
    );
    assert_eq!(opened["metadata"]["filename"], "Pasted image");
    assert_eq!(opened["document"]["layers"].as_array().unwrap().len(), 1);
    assert_eq!(opened["document"]["layers"][0]["content"]["type"], "pixel");
}

#[test]
fn a_selection_limits_the_merged_picture() {
    if !real_clipboard_enabled() {
        eprintln!("PHOTOFORGE_CLIPBOARD_TESTS is not 1: this test would replace the clipboard, so it does not run");
        return;
    }
    let _lock = CLIPBOARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let harness = Harness::new();
    // Select a 3 × 2 block inside the red rectangle (which spans x 10..30, y 5..15).
    let block: Vec<(u32, u32)> = (12..15).flat_map(|x| (6..8).map(move |y| (x, y))).collect();
    let copied = harness
        .call(
            "copy_to_clipboard",
            json!({
                "document": document(),
                "operations": [],
                "layerId": null,
                "selection": selection(40, 30, &block),
            }),
        )
        .expect("the copy succeeds");
    assert_eq!(copied["scope"], "merged");
    assert_eq!(copied["selection"], true);
    assert_eq!(
        (copied["width"].as_u64(), copied["height"].as_u64()),
        (Some(3), Some(2))
    );
    let pasted = photoforge_lib::clipboard::read_image(1 << 20).unwrap();
    assert_eq!(pasted.image.dimensions(), (3, 2));
    assert!(pasted.image.pixels().all(|p| p.0 == [255, 0, 0, 255]));
    // The merged picture, whole: the canvas, with the blue shape in it.
    let whole = harness
        .call(
            "copy_to_clipboard",
            json!({ "document": document(), "operations": [], "layerId": null, "selection": null }),
        )
        .unwrap();
    assert_eq!(
        (whole["width"].as_u64(), whole["height"].as_u64()),
        (Some(40), Some(30))
    );
    let pasted = photoforge_lib::clipboard::read_image(1 << 20).unwrap();
    assert_eq!(pasted.image.get_pixel(20, 10).0, [255, 0, 0, 255]);
    assert_eq!(pasted.image.get_pixel(3, 22).0, [0, 0, 255, 255]);
    assert_eq!(pasted.image.get_pixel(35, 28).0[3], 0);
}
