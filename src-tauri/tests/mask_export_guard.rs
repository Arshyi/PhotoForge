//! A mask is never written over the file the open document came from, through the real command
//! boundary. (It was possible before Phase 15: the mask exports wrote to any path they were given.)
use photoforge_lib::application::AppState;
use photoforge_lib::mask::{MaskBitmap, MaskFile, MaskMetadata, MaskSnapshot};
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

fn snapshot() -> MaskSnapshot {
    let mut bitmap = MaskBitmap::empty(4, 4).unwrap();
    bitmap.coverage_mut()[5] = 255;
    MaskSnapshot::encode(&bitmap)
}

fn mask() -> Value {
    serde_json::to_value(snapshot()).unwrap()
}

fn mask_file() -> Value {
    serde_json::to_value(MaskFile::new(
        "m".into(),
        "m".into(),
        snapshot(),
        MaskMetadata::default(),
    ))
    .unwrap()
}

#[test]
fn a_mask_is_not_written_over_the_file_the_document_was_opened_from() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("shot.exr");
    exr::prelude::write_rgba_file(&source, 4, 4, |_, _| (0.5_f32, 0.5_f32, 0.5_f32, 1.0_f32))
        .unwrap();
    let before = std::fs::read(&source).unwrap();
    let harness = Harness::new();
    let opened = harness
        .call(
            "import_exr_document",
            json!({ "path": source.to_string_lossy(), "requestId": 1 }),
        )
        .expect("the document opens");
    let document_id = opened["project"]["documentId"].as_u64().unwrap();

    // Over the source itself, by name and by a longer spelling of the name.
    let longer = directory.path().join(".").join("shot.exr");
    for (n, target) in [&source, &longer].into_iter().enumerate() {
        for command in ["export_mask_png", "export_mask_file"] {
            let body = if command == "export_mask_png" {
                json!({ "path": target.to_string_lossy(), "mask": mask(),
                        "documentId": document_id, "requestId": 10 + n })
            } else {
                json!({ "path": target.to_string_lossy(),
                        "document": mask_file(),
                        "documentId": document_id, "requestId": 10 + n })
            };
            let error = harness.call(command, body).unwrap_err();
            assert_eq!(error["code"], "invalid_output_path", "{command}: {error}");
        }
    }
    assert_eq!(std::fs::read(&source).unwrap(), before);

    // Another file is another matter.
    let other = directory.path().join("mask.png");
    harness
        .call(
            "export_mask_png",
            json!({ "path": other.to_string_lossy(), "mask": mask(),
                    "documentId": document_id, "requestId": 20 }),
        )
        .expect("a mask is written to another file");
    assert!(other.exists());
    assert_eq!(std::fs::read(&source).unwrap(), before);
}
