//! Opening an SVG through the real command boundary: the name, the arguments the interface sends,
//! the document and report that come back, and the refusals.
use photoforge_lib::application::AppState;
use serde_json::{json, Value};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

struct Harness {
    _app: tauri::App<MockRuntime>,
    webview: WebviewWindow<MockRuntime>,
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
        Self { _app: app, webview }
    }

    fn open(&self, path: &str, request_id: u64) -> Result<Value, Value> {
        let response = tauri::test::get_ipc_response(
            &self.webview,
            InvokeRequest {
                cmd: "import_svg_document".into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "http://tauri.localhost".parse().unwrap(),
                body: json!({ "path": path, "requestId": request_id }).into(),
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

fn write(directory: &tempfile::TempDir, name: &str, text: &str) -> String {
    let path = directory.path().join(name);
    std::fs::write(&path, text).unwrap();
    path.to_string_lossy().into_owned()
}

const ART: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80" viewBox="0 0 120 80">
  <title>Poster</title>
  <rect id="Sky" x="0" y="0" width="120" height="50" fill="#2a6fdb"/>
  <g id="Sun" opacity="0.8">
    <circle cx="90" cy="20" r="12" fill="#ffd400" stroke="#c07a00" stroke-width="2"/>
  </g>
  <path id="Hill" d="M0 80 C30 40 70 40 120 80 Z" fill="#2f9e44"/>
  <text x="5" y="10">Hello</text>
</svg>"##;

#[test]
fn an_svg_opens_as_an_unsaved_document_of_shape_layers_with_its_report() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(&directory, "poster.svg", ART);
    let harness = Harness::new();
    let result = harness.open(&path, 1).expect("the import succeeds");

    let project = &result["project"];
    assert_eq!(project["isCurrent"], true);
    assert_eq!(project["canvasWidth"], 120);
    assert_eq!(project["canvasHeight"], 80);
    assert_eq!(project["metadata"]["format"], "SVG");
    assert_eq!(project["metadata"]["filename"], "poster.svg");
    // High precision: shapes are drawn in float.
    assert_eq!(project["document"]["precision"], "linear_srgb_f32");

    let layers = project["document"]["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 3);
    assert_eq!(layers[0]["name"], "Sky");
    assert_eq!(layers[0]["content"]["type"], "shape");
    assert_eq!(layers[0]["content"]["geometry"]["type"], "rectangle");
    assert_eq!(layers[1]["name"], "Sun");
    assert_eq!(layers[1]["content"]["type"], "group");
    assert_eq!(layers[2]["content"]["geometry"]["type"], "path");

    let report = &result["report"];
    assert_eq!(report["format"], "SVG");
    assert_eq!(report["layersImported"], 3);
    assert_eq!(report["groupsImported"], 1);
    let entries = report["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["kind"], "not_imported");
    assert!(entries[0]["detail"].as_str().unwrap().contains("Text"));
}

#[test]
fn what_is_not_a_readable_svg_is_refused_and_a_missing_file_is_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    for (name, text) in [
        ("html.svg", "<html><body>no</body></html>"),
        ("text.svg", "just some words"),
        (
            "dtd.svg",
            r#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "b">]><svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>"#,
        ),
        (
            "sizeless.svg",
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="1" height="1"/></svg>"#,
        ),
    ] {
        let path = write(&directory, name, text);
        assert!(harness.open(&path, 1).is_err(), "{name} was opened");
    }
    let missing = directory.path().join("nope.svg");
    assert!(harness.open(&missing.to_string_lossy(), 1).is_err());
}
