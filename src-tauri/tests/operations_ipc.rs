//! The operation commands through the real IPC boundary: the names, the argument
//! shapes the interface sends, and the error shapes it receives.
use photoforge_lib::application::AppState;
use photoforge_lib::layers::LayerDocument;
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
        let app = photoforge_lib::register_operation_commands(mock_builder())
            .manage(AppState::default())
            .build(mock_context(noop_assets()))
            .expect("mock application");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview");
        Self { _app: app, webview }
    }

    fn call(&self, command: &str, body: Value) -> Result<Value, Value> {
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
            Err(error) => Err(error),
        }
    }
}

fn document() -> Value {
    serde_json::to_value(LayerDocument::new(8, 8)).unwrap()
}

#[test]
fn the_registry_is_listed_with_everything_the_interface_needs() {
    let harness = Harness::new();
    let specs = harness.call("list_operations", json!({})).unwrap();
    let specs = specs.as_array().unwrap();
    assert_eq!(specs.len(), 22);
    let ids: Vec<&str> = specs
        .iter()
        .map(|spec| spec["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"core.layer.set_opacity"));
    assert!(ids.contains(&"core.plugin.apply_filter"));
    for spec in specs {
        for field in [
            "id",
            "version",
            "title",
            "summary",
            "category",
            "effects",
            "lock",
            "plannerSafe",
            "needsSelection",
            "params",
        ] {
            assert!(
                spec.get(field).is_some(),
                "{} is missing {field}",
                spec["id"]
            );
        }
    }
}

#[test]
fn a_transaction_is_applied_as_the_interface_sends_it() {
    let harness = Harness::new();
    let result = harness
        .call(
            "apply_transaction",
            json!({
                "request": {
                    "document": document(),
                    "label": "Add notes",
                    "steps": [
                        { "op": "core.layer.add_group", "params": { "name": "Notes" } },
                        { "op": "core.layer.rename", "params": {
                            "selector": { "type": "last_created" }, "name": "Notes v2" } }
                    ],
                    "origin": "automation"
                }
            }),
        )
        .unwrap();
    assert_eq!(result["label"], "Add notes");
    assert_eq!(result["document"]["layers"][0]["name"], "Notes v2");
    assert_eq!(result["steps"].as_array().unwrap().len(), 2);
    assert_eq!(
        result["steps"][0]["createdLayers"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(result["revision"].as_str().unwrap().len(), 64);

    // The revision it returns is the one `layer_document_revision` computes.
    let revision = harness
        .call(
            "layer_document_revision",
            json!({ "document": result["document"].clone() }),
        )
        .unwrap();
    assert_eq!(revision, result["revision"]);
}

#[test]
fn a_failure_arrives_with_a_stable_code_and_the_step_that_caused_it() {
    let harness = Harness::new();
    let error = harness
        .call(
            "apply_transaction",
            json!({
                "request": {
                    "document": document(),
                    "label": "Bad",
                    "steps": [
                        { "op": "core.layer.add_group", "params": {} },
                        { "op": "core.layer.set_opacity",
                          "params": { "selector": { "type": "active" }, "opacity": 4.0 } }
                    ]
                }
            }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "transaction_failed");
    let message = error["message"].as_str().unwrap();
    assert!(message.contains("Step 2"), "{message}");
    assert!(message.contains("core.layer.set_opacity"), "{message}");
    assert!(message.contains("Nothing was changed"), "{message}");

    // A stale plan has its own code, so the interface can say why.
    let stale = harness
        .call(
            "apply_transaction",
            json!({
                "request": {
                    "document": document(),
                    "label": "Stale",
                    "steps": [{ "op": "core.layer.add_group", "params": {} }],
                    "expectedRevision": "0".repeat(64)
                }
            }),
        )
        .unwrap_err();
    assert_eq!(stale["code"], "stale_revision");

    // An unknown request field is refused, not ignored.
    let unknown = harness
        .call(
            "apply_transaction",
            json!({ "request": {
                "document": document(), "label": "x", "steps": [], "surprise": 1 } }),
        )
        .unwrap_err();
    assert!(unknown.to_string().contains("surprise") || unknown.to_string().contains("unknown"));
}
