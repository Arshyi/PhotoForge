//! Writing an SVG through the real command boundary: the name, the arguments the interface
//! sends, the file and report that come back, and the refusals.
use photoforge_lib::application::AppState;
use photoforge_lib::layers::{LayerContent, LayerDocument, ShapeColor, ShapeContent};
use photoforge_lib::operations::structure::new_layer;
use photoforge_lib::vector::{FillRule, ShapeGeometry};
use serde_json::{json, Value};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

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

    fn call(&self, body: Value) -> Result<Value, Value> {
        let response = tauri::test::get_ipc_response(
            &self.webview,
            InvokeRequest {
                cmd: "export_svg_document".into(),
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

fn document() -> Value {
    let mut document = LayerDocument::new(120, 80);
    document.layers.push(new_layer(
        "shape-1",
        "Sky box",
        LayerContent::Shape {
            shape: Box::new(ShapeContent {
                geometry: ShapeGeometry::Rectangle {
                    x: 10.0,
                    y: 10.0,
                    width: 60.0,
                    height: 30.0,
                    corner_radius: 4.0,
                },
                fill: Some(ShapeColor::new(0.0, 0.2, 0.8, 1.0)),
                stroke: None,
                stroke_style: None,
                fill_rule: FillRule::NonZero,
            }),
        },
        "t",
    ));
    document.layers.push(new_layer(
        "pixels-1",
        "Photo",
        LayerContent::Pixel {
            pixel_id: "px-1".into(),
            width: 4,
            height: 4,
        },
        "t",
    ));
    serde_json::to_value(&document).unwrap()
}

#[test]
fn shape_layers_are_written_and_what_was_left_out_is_named() {
    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("art.svg");
    let result = harness
        .call(json!({
            "outputPath": path.to_string_lossy(),
            "document": document(),
            "operations": [],
        }))
        .expect("the export succeeds");
    assert_eq!(result["outputPath"], path.to_string_lossy().as_ref());
    let report = &result["report"];
    assert_eq!(report["format"], "SVG");
    assert_eq!(
        (report["width"].as_u64(), report["height"].as_u64()),
        (Some(120), Some(80))
    );
    assert_eq!(report["layersWritten"], 1);
    let entries = report["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["kind"], "not_exported");
    assert_eq!(entries[0]["layer"], "Photo");
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("<?xml"));
    assert!(text.contains("<rect x=\"10\" y=\"10\" width=\"60\" height=\"30\" rx=\"4\" ry=\"4\""));
    assert!(text.contains("<title>Sky box</title>"));
    assert_eq!(
        report["bytes"].as_u64().unwrap(),
        std::fs::metadata(&path).unwrap().len()
    );
    // Nothing else was left beside it.
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn a_name_that_is_not_svg_or_not_absolute_is_refused_and_nothing_is_written() {
    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    for name in ["art.png", "art", "art.svg.txt"] {
        let path = directory.path().join(name);
        let error = harness
            .call(json!({
                "outputPath": path.to_string_lossy(),
                "document": document(),
                "operations": [],
            }))
            .unwrap_err();
        assert_eq!(error["code"], "invalid_output_path", "{name}: {error}");
    }
    let relative = harness
        .call(json!({ "outputPath": "art.svg", "document": document(), "operations": [] }))
        .unwrap_err();
    assert_eq!(relative["code"], "invalid_output_path");
    let missing = directory.path().join("no-such-folder").join("art.svg");
    let error = harness
        .call(json!({
            "outputPath": missing.to_string_lossy(),
            "document": document(),
            "operations": [],
        }))
        .unwrap_err();
    assert_eq!(error["code"], "invalid_output_path");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn an_existing_file_is_replaced_whole_and_an_invalid_document_leaves_it() {
    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("art.svg");
    std::fs::write(&path, b"what was here").unwrap();
    // A canvas the model does not allow cannot be written.
    let mut bad = document();
    bad["canvasWidth"] = json!(0);
    let result = harness.call(json!({
        "outputPath": path.to_string_lossy(),
        "document": bad,
        "operations": [],
    }));
    assert!(result.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"what was here");
    harness
        .call(json!({
            "outputPath": path.to_string_lossy(),
            "document": document(),
            "operations": [],
        }))
        .unwrap();
    assert!(std::fs::read_to_string(&path).unwrap().contains("<svg"));
}

#[test]
fn edits_over_the_whole_document_are_reported_not_applied() {
    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("art.svg");
    let result = harness
        .call(json!({
            "outputPath": path.to_string_lossy(),
            "document": document(),
            "operations": [{ "type": "grayscale" }],
        }))
        .unwrap();
    let entries = result["report"]["entries"].as_array().unwrap();
    assert!(entries.iter().any(
        |e| e["kind"] == "not_exported" && e["detail"].as_str().unwrap().contains("grayscale")
    ));
}
