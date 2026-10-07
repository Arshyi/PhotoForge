//! Plugins end to end, through the real IPC boundary: install a package from a file,
//! run its command on a document, see it in the preview, save and reopen the project,
//! and watch what happens when the plugin goes away.
#![cfg(feature = "plugins")]
mod common;
use common::*;
use photoforge_lib::application::AppState;
use photoforge_lib::plugins::store::{set_global, PluginRegistry};
use photoforge_lib::plugins::testing::example_package;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{WebviewWindow, WebviewWindowBuilder};

static GLOBAL: Mutex<()> = Mutex::new(());

struct Harness {
    _lock: MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
    _app: tauri::App<MockRuntime>,
    webview: WebviewWindow<MockRuntime>,
}

impl Harness {
    fn new() -> Self {
        let lock = GLOBAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        set_global(Some(Arc::new(PluginRegistry::open(
            dir.path().join("plugins"),
        ))));
        let app = photoforge_lib::register_plugin_commands(mock_builder())
            .manage(AppState::default())
            .build(mock_context(noop_assets()))
            .expect("mock application");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview");
        Self {
            _lock: lock,
            dir,
            _app: app,
            webview,
        }
    }

    fn call(&self, command: &str, body: Value) -> Result<Value, Value> {
        // `run_plugin_command` takes one argument, `request`, as the interface sends it.
        let body = if command == "run_plugin_command" {
            json!({ "request": body })
        } else {
            body
        };
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

    fn ok(&self, command: &str, body: Value) -> Value {
        self.call(command, body)
            .unwrap_or_else(|error| panic!("{command} failed: {error}"))
    }

    fn err(&self, command: &str, body: Value) -> Value {
        match self.call(command, body) {
            Ok(value) => panic!("{command} unexpectedly succeeded: {value}"),
            Err(error) => error,
        }
    }

    /// Writes an example's package to a file and returns its path.
    fn package_file(&self, name: &str) -> String {
        let bytes = example_package(
            &example_manifest_json(name),
            Some(&example_module(name)),
            Some("An example."),
        );
        let path = self.dir.path().join(format!("{name}.photoforge-plugin"));
        std::fs::write(&path, bytes).unwrap();
        path.to_string_lossy().into_owned()
    }

    /// Inspects and installs an example, granting everything it asks for (or
    /// `grant` if given). Returns the content hash.
    fn install(&self, name: &str, grant: Option<Vec<&str>>) -> String {
        let path = self.package_file(name);
        let inspection = self.ok("inspect_plugin_package", json!({ "path": path }));
        let hash = inspection["contentHash"].as_str().unwrap().to_string();
        let asked: Vec<String> = inspection["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_string())
            .collect();
        let grant: Vec<String> = grant.map_or(asked, |g| g.into_iter().map(String::from).collect());
        let report = self.ok(
            "install_plugin_package",
            json!({ "path": path, "expectedHash": hash, "grant": grant }),
        );
        assert_eq!(report["contentHash"], hash);
        assert_eq!(report["test"]["passed"], true);
        hash
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        set_global(None);
    }
}

fn png_of(preview: &Value) -> image::RgbaImage {
    use base64::Engine;
    let url = preview["previewDataUrl"].as_str().unwrap();
    let data = url.split_once(',').unwrap().1;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .unwrap();
    image::load_from_memory(&bytes).unwrap().to_rgba8()
}

const SHAPES: &str = "photoforge.example.shapes";

#[test]
fn a_package_is_inspected_installed_listed_tested_and_removed() {
    let harness = Harness::new();
    let path = harness.package_file("shapes");
    let inspection = harness.ok("inspect_plugin_package", json!({ "path": path }));
    assert_eq!(inspection["manifest"]["id"], SHAPES);
    assert!(inspection["signature"]
        .as_str()
        .unwrap()
        .starts_with("Not signed"));
    assert_eq!(inspection["runtimeAvailable"], true);
    let asked: Vec<&str> = inspection["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(asked, ["filter.pixels", "document.operations", "ui.tool"]);
    // Inspecting installed nothing.
    assert!(harness.ok("list_plugins", json!({}))["plugins"]
        .as_array()
        .unwrap()
        .is_empty());

    // The hash the person was shown is the hash that is installed; a changed file is not.
    let hash = inspection["contentHash"].as_str().unwrap();
    let wrong = harness.err(
        "install_plugin_package",
        json!({ "path": path, "expectedHash": "0".repeat(64), "grant": [] }),
    );
    assert!(wrong["message"]
        .as_str()
        .unwrap()
        .contains("changed after it was inspected"));
    harness.install("shapes", None);

    let status = harness.ok("list_plugins", json!({}));
    assert_eq!(status["runtimeAvailable"], true);
    let plugin = &status["plugins"][0];
    assert_eq!(plugin["id"], SHAPES);
    assert_eq!(plugin["enabled"], true);
    assert_eq!(plugin["activeHash"], hash);
    assert_eq!(plugin["availability"]["kind"], "available");
    assert_eq!(plugin["granted"].as_array().unwrap().len(), 3);

    let tested = harness.ok("test_plugin", json!({ "plugin": SHAPES }));
    assert_eq!(tested["passed"], true);
    assert_eq!(tested["filters"][0]["filter"], "shape");
    assert!(tested["filters"][0]["tiles"].as_u64().unwrap() > 1);

    harness.ok(
        "set_plugin_enabled",
        json!({ "plugin": SHAPES, "enabled": false }),
    );
    assert_eq!(
        harness.ok("list_plugins", json!({}))["plugins"][0]["availability"]["kind"],
        "disabled"
    );
    harness.ok(
        "set_plugin_enabled",
        json!({ "plugin": SHAPES, "enabled": true }),
    );
    harness.ok("remove_plugin", json!({ "plugin": SHAPES }));
    assert!(harness.ok("list_plugins", json!({}))["plugins"]
        .as_array()
        .unwrap()
        .is_empty());
    // Removing again, and testing what is not there, say so.
    assert_eq!(
        harness.err("remove_plugin", json!({ "plugin": SHAPES }))["code"],
        "invalid_plugin_manifest"
    );
    assert_eq!(
        harness.err("test_plugin", json!({ "plugin": SHAPES }))["code"],
        "plugin_unavailable"
    );
}

#[test]
fn a_file_that_is_not_a_plugin_package_is_not_read_as_one() {
    let harness = Harness::new();
    for (name, bytes) in [
        ("a.zip", b"PK\x05\x06".to_vec()),
        ("b.photoforge-plugin", b"not a zip".to_vec()),
    ] {
        let path = harness.dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        let error = harness.err(
            "inspect_plugin_package",
            json!({ "path": path.to_string_lossy() }),
        );
        assert_eq!(error["code"], "invalid_plugin_manifest", "{name}");
    }
    for path in [
        "relative.photoforge-plugin",
        "\\\\server\\share\\x.photoforge-plugin",
        "C:/x/../y.photoforge-plugin",
    ] {
        harness.err("inspect_plugin_package", json!({ "path": path }));
    }
}

/// Success case 5: a plugin works end to end, including Undo and save and reopen.
#[test]
fn a_plugin_command_draws_on_a_document_as_one_undoable_step_and_survives_a_save() {
    let harness = Harness::new();
    harness.install("shapes", None);
    let blank = harness.ok(
        "create_blank_layer_document",
        json!({ "width": 96, "height": 72, "requestId": 5 }),
    );
    assert_eq!(blank["document"]["layers"].as_array().unwrap().len(), 0);
    let before = blank["document"].clone();

    let result = harness.ok(
        "run_plugin_command",
        json!({
            "plugin": SHAPES, "command": "add_shape",
            "values": { "shape": 1, "size": 0.5 },
            "document": before.clone()
        }),
    );
    // One transaction: a new layer made, and the plugin's filter applied to it.
    assert_eq!(result["label"], "Add shape layer");
    assert_eq!(result["steps"].as_array().unwrap().len(), 2);
    assert_eq!(result["createdPixelIds"].as_array().unwrap().len(), 2);
    let document = result["document"].clone();
    assert_eq!(document["layers"].as_array().unwrap().len(), 1);
    assert_eq!(document["layers"][0]["name"], "Shape");
    // Undo is returning to the document that was passed in, which is untouched.
    assert_eq!(before["layers"].as_array().unwrap().len(), 0);

    // The picture is a square in the middle and clear around it.
    let preview = harness.ok(
        "render_layer_composite",
        json!({ "document": document, "operations": [], "documentId": 5, "requestId": 1 }),
    );
    assert_eq!(preview["missingPlugins"].as_array().unwrap().len(), 0);
    let picture = png_of(&preview);
    let (w, h) = picture.dimensions();
    assert_eq!(picture.get_pixel(w / 2, h / 2).0, [255, 255, 255, 255]);
    assert_eq!(picture.get_pixel(1, 1).0[3], 0);

    // Saved and reopened, the document is what it was.
    let path = harness.dir.path().join("shapes.photoforge");
    harness.ok(
        "save_layer_project",
        json!({
            "outputPath": path.to_string_lossy(), "document": document, "operations": [],
            "createdAt": "2026-10-06T00:00:00Z", "modifiedAt": "2026-10-06T00:00:00Z"
        }),
    );
    let loaded = harness.ok(
        "load_layer_project",
        json!({ "path": path.to_string_lossy(), "requestId": 6 }),
    );
    assert_eq!(loaded["document"]["layers"][0]["name"], "Shape");
    assert_eq!(
        loaded["pluginRequirements"].as_array().unwrap().len(),
        0,
        "baked pixels need no plugin"
    );
}

#[test]
fn a_plugin_adjustment_layer_is_part_of_the_document_and_missing_plugins_are_never_hidden() {
    let harness = Harness::new();
    harness.install("shapes", None);
    harness.install("solarize", None);
    let blank = harness.ok("run_plugin_command", json!({
        "plugin": SHAPES, "command": "add_shape", "values": { "shape": 0, "size": 0.8 },
        "document": harness.ok("create_blank_layer_document", json!({ "width": 96, "height": 72, "requestId": 8 }))["document"].clone()
    }));
    // A non-destructive plugin layer that inverts everything beneath it.
    let made = harness.ok("apply_transaction", json!({ "request": {
        "document": blank["document"].clone(), "label": "Add solarize",
        "steps": [{ "op": "core.plugin.add_adjustment", "params": {
            "plugin": "photoforge.example.solarize", "filter": "solarize", "parameters": { "threshold": 0.0 } } }],
        "origin": "user" } }));
    let document = made["document"].clone();
    let layer = &document["layers"][1];
    assert_eq!(layer["name"], "Solarize", "named for the filter");
    assert_eq!(layer["content"]["operation"]["type"], "plugin_filter");
    let recorded = layer["content"]["operation"]["sha256"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(recorded.len(), 64);

    // The id of the document the session holds: opening a project replaces it.
    let session = std::cell::Cell::new(8u64);
    let render = |doc: &Value, id: u64| {
        harness.ok(
            "render_layer_composite",
            json!({ "document": doc, "operations": [], "documentId": session.get(), "requestId": id }),
        )
    };
    let with = render(&document, 1);
    assert_eq!(with["missingPlugins"].as_array().unwrap().len(), 0);
    let centre = |preview: &Value| {
        let picture = png_of(preview);
        picture
            .get_pixel(picture.width() / 2, picture.height() / 2)
            .0
    };
    assert_eq!(
        centre(&with),
        [0, 0, 0, 255],
        "the white circle was inverted by the plugin layer"
    );

    // The plugin goes away.
    harness.ok(
        "remove_plugin",
        json!({ "plugin": "photoforge.example.solarize" }),
    );
    let status = harness.ok(
        "document_plugin_status",
        json!({ "document": document, "operations": [] }),
    );
    assert_eq!(status[0]["plugin"], "photoforge.example.solarize");
    assert_eq!(status[0]["sha256"], recorded);
    assert_eq!(status[0]["availability"]["kind"], "missing");
    assert!(status[0]["message"]
        .as_str()
        .unwrap()
        .contains("not installed"));

    // The preview leaves the layer out and names it. It does not pretend.
    let without = render(&document, 2);
    let missing = without["missingPlugins"].as_array().unwrap();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0]["layerName"], "Solarize");
    assert_eq!(missing[0]["plugin"], "photoforge.example.solarize");
    assert_eq!(
        centre(&without),
        [255, 255, 255, 255],
        "drawn without the plugin layer, and said so"
    );

    // Nothing that makes a file of it will go ahead.
    let export = harness.err(
        "export_layer_composite",
        json!({
        "outputPath": harness.dir.path().join("out.png").to_string_lossy(),
        "document": document, "operations": [], "profile": "lossless" }),
    );
    assert_eq!(export["code"], "plugin_unavailable");
    assert!(export["message"]
        .as_str()
        .unwrap()
        .contains("photoforge.example.solarize"));
    assert!(!harness.dir.path().join("out.png").exists());
    let flattened = harness.err("flatten_layer_document", json!({ "document": document }));
    assert_eq!(flattened["code"], "plugin_unavailable");
    let merged = harness.err("merge_layer_pixels", json!({
        "document": document, "layerIds": [document["layers"][0]["id"], document["layers"][1]["id"]] }));
    assert!(
        merged["code"] == "plugin_unavailable" || merged["code"] == "invalid_layer_document",
        "{merged}"
    );
    let transaction = harness.err(
        "apply_transaction",
        json!({ "request": {
        "document": document, "label": "Flatten",
        "steps": [{ "op": "core.document.flatten", "params": {} }], "origin": "user" } }),
    );
    assert_eq!(transaction["code"], "transaction_failed");
    assert!(transaction["message"]
        .as_str()
        .unwrap()
        .contains("photoforge.example.solarize"));

    // Saving keeps the reference, and reopening says what is needed.
    let path = harness.dir.path().join("needs-plugin.photoforge");
    harness.ok(
        "save_layer_project",
        json!({
        "outputPath": path.to_string_lossy(), "document": document, "operations": [],
        "createdAt": "2026-10-06T00:00:00Z", "modifiedAt": "2026-10-06T00:00:00Z" }),
    );
    let loaded = harness.ok(
        "load_layer_project",
        json!({ "path": path.to_string_lossy(), "requestId": 9 }),
    );
    session.set(9);
    let kept = &loaded["document"]["layers"][1]["content"]["operation"];
    assert_eq!(kept["plugin"], "photoforge.example.solarize");
    assert_eq!(
        kept["sha256"], recorded,
        "the project dropped or changed the plugin reference"
    );
    assert_eq!(kept["parameters"], json!([0.0]));
    assert_eq!(
        loaded["pluginRequirements"][0]["availability"]["kind"],
        "missing"
    );

    // A different build of the plugin under the same id is not a substitute.
    let mut manifest = example_manifest_json("solarize");
    manifest["version"] = json!("1.0.1");
    let other = harness.dir.path().join("other.photoforge-plugin");
    std::fs::write(
        &other,
        example_package(&manifest, Some(&example_module("solarize")), None),
    )
    .unwrap();
    let inspected = harness.ok(
        "inspect_plugin_package",
        json!({ "path": other.to_string_lossy() }),
    );
    harness.ok("install_plugin_package", json!({
        "path": other.to_string_lossy(), "expectedHash": inspected["contentHash"], "grant": ["filter.pixels"] }));
    let status = harness.ok(
        "document_plugin_status",
        json!({ "document": document, "operations": [] }),
    );
    assert_eq!(status[0]["availability"]["kind"], "otherVersion");
    assert_eq!(status[0]["availability"]["installedVersion"], "1.0.1");
    assert_eq!(
        centre(&render(&document, 3)),
        [255, 255, 255, 255],
        "a different version rendered the layer"
    );

    // The exact package the document names, installed again, restores the picture —
    // beside the other version, which is not the one it was made with.
    harness.install("solarize", None);
    assert_eq!(
        harness.ok(
            "document_plugin_status",
            json!({ "document": document, "operations": [] })
        )[0]["availability"]["kind"],
        "available"
    );
    assert_eq!(
        render(&document, 4)["missingPlugins"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(centre(&render(&document, 5)), [0, 0, 0, 255]);
}

#[test]
fn what_a_plugin_is_not_allowed_to_do_is_refused_at_the_point_of_use() {
    let harness = Harness::new();
    // Installed without the right to change the document.
    harness.install("shapes", Some(vec!["filter.pixels"]));
    let blank = harness.ok(
        "create_blank_layer_document",
        json!({ "width": 32, "height": 32, "requestId": 3 }),
    );
    let command = harness.err(
        "run_plugin_command",
        json!({
        "plugin": SHAPES, "command": "add_shape", "document": blank["document"].clone() }),
    );
    assert_eq!(command["code"], "plugin_unavailable");
    assert!(
        command["message"]
            .as_str()
            .unwrap()
            .contains("not been allowed"),
        "{command}"
    );
    harness.ok(
        "set_plugin_grants",
        json!({ "plugin": SHAPES, "grant": ["filter.pixels", "document.operations"] }),
    );
    harness.ok(
        "run_plugin_command",
        json!({
        "plugin": SHAPES, "command": "add_shape", "document": blank["document"].clone() }),
    );

    // Values are checked against the command's own declaration.
    for values in [
        json!({ "size": 2.0 }),
        json!({ "shape": 9 }),
        json!({ "colour": 1 }),
        json!({ "size": "big" }),
    ] {
        let error = harness.call("run_plugin_command", json!({
            "plugin": SHAPES, "command": "add_shape", "values": values, "document": blank["document"].clone() }));
        assert!(error.is_err(), "{values}");
    }
    assert!(harness
        .call(
            "run_plugin_command",
            json!({
        "plugin": SHAPES, "command": "nope", "document": blank["document"].clone() })
        )
        .is_err());
    assert!(harness
        .call(
            "run_plugin_command",
            json!({
        "plugin": "com.example.absent", "command": "x", "document": blank["document"].clone() })
        )
        .is_err());

    // A plugin's transaction may apply that plugin's filters and nobody else's, and
    // must say which plugin it is.
    harness.install("solarize", None);
    let request = |origin: &str, plugin: Value| {
        json!({ "request": {
        "document": blank["document"].clone(), "label": "Mischief", "origin": origin, "plugin": plugin,
        "steps": [
            { "op": "core.layer.add_pixel", "params": {} },
            { "op": "core.plugin.apply_filter", "params": {
                "selector": { "type": "last_created" }, "plugin": "photoforge.example.solarize", "filter": "solarize" } }] } })
    };
    let anonymous = harness.err("apply_transaction", request("plugin", Value::Null));
    assert!(
        anonymous["message"]
            .as_str()
            .unwrap()
            .contains("which plugin"),
        "{anonymous}"
    );
    let confused = harness.err("apply_transaction", request("plugin", json!(SHAPES)));
    assert_eq!(confused["code"], "transaction_failed");
    assert!(
        confused["message"]
            .as_str()
            .unwrap()
            .contains("only its own filters"),
        "{confused}"
    );
    // The same two steps, as the plugin that owns the filter, are allowed.
    harness.ok(
        "apply_transaction",
        request("plugin", json!("photoforge.example.solarize")),
    );
    // And a person may use any installed filter.
    harness.ok("apply_transaction", request("user", Value::Null));
}
