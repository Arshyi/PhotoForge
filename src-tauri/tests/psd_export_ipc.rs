//! Writing a Photoshop file through the real command boundary: the name, the arguments the
//! interface sends, the file and report that come back, and the refusals.
use photoforge_lib::application::AppState;
use photoforge_lib::layers::{LayerContent, LayerDocument};
use photoforge_lib::operations::structure::new_layer;
use photoforge_lib::psd::{import_psd, ImportLimits};
use serde_json::{json, Value};
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{Manager, WebviewWindow, WebviewWindowBuilder};

struct Harness {
    app: tauri::App<MockRuntime>,
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
        Self { app, webview }
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

    /// A canvas one pixel high and wider than a PSD can be, with one layer across it.
    fn wide_document(&self) -> Value {
        let state = self.app.state::<AppState>();
        let mut store = state.layers.lock().unwrap();
        store.reset(30_001, 1).unwrap();
        let mut document = LayerDocument::new(30_001, 1);
        let id = store
            .register(image::RgbaImage::from_pixel(
                30_001,
                1,
                image::Rgba([9, 8, 7, 255]),
            ))
            .unwrap();
        document.layers.push(new_layer(
            "layer-1",
            "Strip",
            LayerContent::Pixel {
                pixel_id: id,
                width: 30_001,
                height: 1,
            },
            "t",
        ));
        serde_json::to_value(&document).unwrap()
    }

    /// Registers an image as a layer buffer and returns the document that shows it.
    fn document(&self, layers: &[(&str, (u32, u32), [u8; 4])]) -> Value {
        let state = self.app.state::<AppState>();
        let mut store = state.layers.lock().unwrap();
        store.reset(16, 12).unwrap();
        let mut document = LayerDocument::new(16, 12);
        for (name, (width, height), rgba) in layers {
            let id = store
                .register(image::RgbaImage::from_pixel(
                    *width,
                    *height,
                    image::Rgba(*rgba),
                ))
                .unwrap();
            document.layers.push(new_layer(
                &format!("layer-{}", document.layers.len() + 1),
                name,
                LayerContent::Pixel {
                    pixel_id: id,
                    width: *width,
                    height: *height,
                },
                "t",
            ));
        }
        serde_json::to_value(&document).unwrap()
    }
}

const BIG: ImportLimits = ImportLimits {
    store_bytes: 1 << 30,
    entry_bytes: 1 << 29,
    job_bytes: 1 << 31,
};

#[test]
fn a_layered_document_is_written_as_a_photoshop_file_with_its_report() {
    let harness = Harness::new();
    let mut document = harness.document(&[
        ("Sky", (16, 12), [10, 20, 200, 255]),
        ("Sun", (6, 4), [250, 240, 30, 200]),
    ]);
    // An adjustment layer the file cannot hold, to have something in the report.
    document["layers"].as_array_mut().unwrap().push(json!({
        "id": "blur-1", "name": "Soften", "visible": true, "locked": false, "opacity": 1.0,
        "blendMode": "normal", "collapsed": false,
        "metadata": { "createdAt": "t", "modifiedAt": "t", "custom": {} },
        "content": { "type": "adjustment", "operation": { "type": "gaussian_blur", "radius": 2.0 } }
    }));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("poster.psd");
    let result = harness
        .call(
            "export_psd_document",
            json!({
                "outputPath": path.to_string_lossy(),
                "document": document,
                "operations": [],
                "bitDepth": null,
            }),
        )
        .expect("the export succeeds");
    assert_eq!(result["outputPath"], path.to_string_lossy().as_ref());
    let report = &result["report"];
    assert_eq!(report["format"], "PSD");
    assert_eq!(report["width"], 16);
    assert_eq!(report["bitDepth"], 8);
    assert_eq!(report["layersWritten"], 2);
    let entries = report["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["kind"], "not_exported");
    assert_eq!(entries[0]["layer"], "Soften");
    assert!(path.exists());
    assert_eq!(
        report["bytes"].as_u64().unwrap(),
        std::fs::metadata(&path).unwrap().len()
    );

    let imported = import_psd(&path, BIG).unwrap();
    let names: Vec<_> = imported
        .project
        .document
        .layers
        .iter()
        .map(|l| l.name.clone())
        .collect();
    assert_eq!(names, ["Sky", "Sun"]);
}

#[test]
fn a_psb_name_asks_for_the_large_format() {
    let harness = Harness::new();
    let document = harness.document(&[("Sky", (16, 12), [10, 20, 200, 255])]);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("big.psb");
    let result = harness
        .call(
            "export_psd_document",
            json!({
                "outputPath": path.to_string_lossy(),
                "document": document,
                "operations": [],
                "bitDepth": 16,
            }),
        )
        .unwrap();
    assert_eq!(result["report"]["format"], "PSB");
    assert_eq!(result["report"]["bitDepth"], 16);
    assert!(path.exists());
}

#[test]
fn a_name_that_is_not_a_photoshop_name_is_refused_and_nothing_is_written() {
    let harness = Harness::new();
    let document = harness.document(&[("Sky", (16, 12), [10, 20, 200, 255])]);
    let directory = tempfile::tempdir().unwrap();
    for name in ["picture.png", "picture", "picture.psd.txt"] {
        let path = directory.path().join(name);
        let error = harness
            .call(
                "export_psd_document",
                json!({
                    "outputPath": path.to_string_lossy(),
                    "document": document,
                    "operations": [],
                    "bitDepth": null,
                }),
            )
            .unwrap_err();
        assert_eq!(error["code"], "invalid_output_path", "{name}: {error}");
        assert!(!path.exists());
    }
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[test]
fn a_depth_photoshop_does_not_use_is_refused() {
    let harness = Harness::new();
    let document = harness.document(&[("Sky", (16, 12), [10, 20, 200, 255])]);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("x.psd");
    let error = harness
        .call(
            "export_psd_document",
            json!({
                "outputPath": path.to_string_lossy(),
                "document": document,
                "operations": [],
                "bitDepth": 12,
            }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "psd_file");
    assert!(!path.exists());
}

#[test]
fn an_existing_file_survives_a_failed_export() {
    let harness = Harness::new();
    // A document that names a buffer the session does not hold cannot be written.
    let mut document = harness.document(&[("Sky", (16, 12), [10, 20, 200, 255])]);
    document["layers"][0]["content"]["pixelId"] = json!("px-that-is-not-there");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("keep.psd");
    std::fs::write(&path, b"what was here").unwrap();
    let result = harness.call(
        "export_psd_document",
        json!({
            "outputPath": path.to_string_lossy(),
            "document": document,
            "operations": [],
            "bitDepth": null,
        }),
    );
    assert!(result.is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"what was here");
}

#[test]
fn a_document_too_wide_for_a_psd_is_written_as_a_psb_and_named_as_one() {
    let harness = Harness::new();
    let document = harness.wide_document();
    let directory = tempfile::tempdir().unwrap();
    let asked = directory.path().join("strip.psd");
    let result = harness
        .call(
            "export_psd_document",
            json!({
                "outputPath": asked.to_string_lossy(),
                "document": document,
                "operations": [],
                "bitDepth": null,
            }),
        )
        .expect("the export succeeds");
    let written = directory.path().join("strip.psb");
    assert_eq!(result["outputPath"], written.to_string_lossy().as_ref());
    assert_eq!(result["report"]["format"], "PSB");
    assert!(written.exists());
    assert!(!asked.exists());
    let entries = result["report"]["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|e| e["kind"] == "note" && e["detail"].as_str().unwrap().contains(".psb")),
        "{entries:?}"
    );
    let imported = import_psd(&written, BIG).unwrap();
    assert_eq!(imported.project.document.canvas_width, 30_001);
}

#[test]
fn the_psb_name_is_not_taken_from_another_file() {
    let harness = Harness::new();
    let document = harness.wide_document();
    let directory = tempfile::tempdir().unwrap();
    let asked = directory.path().join("strip.psd");
    let taken = directory.path().join("strip.psb");
    std::fs::write(&taken, b"somebody else's file").unwrap();
    let result = harness
        .call(
            "export_psd_document",
            json!({
                "outputPath": asked.to_string_lossy(),
                "document": document,
                "operations": [],
                "bitDepth": null,
            }),
        )
        .expect("the export succeeds");
    // The large format under the name that was asked for, and the other file as it was.
    assert_eq!(result["outputPath"], asked.to_string_lossy().as_ref());
    assert_eq!(result["report"]["format"], "PSB");
    assert_eq!(std::fs::read(&taken).unwrap(), b"somebody else's file");
    let entries = result["report"]["entries"].as_array().unwrap();
    assert!(
        entries.iter().any(
            |e| e["kind"] == "note" && e["detail"].as_str().unwrap().contains("already exists")
        ),
        "{entries:?}"
    );
}

#[test]
fn the_file_the_document_was_opened_from_is_never_replaced_by_its_export() {
    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("poster.psd");
    let export = |path: &std::path::Path, document: &Value| {
        harness.call(
            "export_psd_document",
            json!({
                "outputPath": path.to_string_lossy(),
                "document": document,
                "operations": [],
                "bitDepth": null,
            }),
        )
    };
    // A real file to open: one this exporter wrote, before any document was open.
    let made = harness.document(&[
        ("Sky", (16, 12), [10, 20, 200, 255]),
        ("Sun", (6, 4), [250, 240, 30, 200]),
    ]);
    export(&input, &made).expect("the first export succeeds");
    let original = std::fs::read(&input).unwrap();
    let opened = harness
        .call(
            "import_psd_document",
            json!({ "path": input.to_string_lossy(), "requestId": 1 }),
        )
        .expect("the import succeeds");
    let document = opened["project"]["document"].clone();

    // The same name, and the other ways of spelling the same file.
    std::fs::create_dir(directory.path().join("sub")).unwrap();
    #[allow(unused_mut)]
    let mut aliases = vec![
        input.clone(),
        directory.path().join("sub").join("..").join("poster.psd"),
    ];
    #[cfg(unix)]
    {
        let link = directory.path().join("link.psd");
        std::os::unix::fs::symlink(&input, &link).unwrap();
        let folder = directory.path().join("folder-link");
        std::os::unix::fs::symlink(directory.path(), &folder).unwrap();
        let hard = directory.path().join("hard.psd");
        std::fs::hard_link(&input, &hard).unwrap();
        aliases.extend([link, folder.join("poster.psd"), hard]);
    }
    let entries = std::fs::read_dir(directory.path()).unwrap().count();
    for alias in &aliases {
        let error = export(alias, &document).unwrap_err();
        assert_eq!(
            error["code"],
            "invalid_output_path",
            "{}: {error}",
            alias.display()
        );
        // It says what to do, and names no path.
        let message = error["message"].as_str().unwrap_or_default();
        assert!(message.contains("different"), "{error}");
        assert!(!message.contains("poster.psd"), "{error}");
        assert_eq!(
            std::fs::read(&input).unwrap(),
            original,
            "{}",
            alias.display()
        );
    }
    // Nothing was written beside it either, not even a temporary file.
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        entries
    );

    // Another name in the same folder is fine, whether it is new or an older file of its own.
    export(&directory.path().join("again.psd"), &document).expect("a new name is allowed");
    let taken = directory.path().join("taken.psd");
    std::fs::write(&taken, b"an older export").unwrap();
    export(&taken, &document).expect("another existing file is replaced");
    assert!(std::fs::read(&taken).unwrap().starts_with(b"8BPS"));
    assert_eq!(std::fs::read(&input).unwrap(), original);
}

#[test]
fn a_document_with_no_source_file_can_be_exported() {
    // A new document and a pasted picture have no file behind them: their source path is empty.
    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    let blank = harness
        .call(
            "create_blank_layer_document",
            json!({ "width": 8, "height": 6, "requestId": 1 }),
        )
        .expect("a new document");
    for name in ["blank.psd", "blank.psb"] {
        let output = directory.path().join(name);
        harness
            .call(
                "export_psd_document",
                json!({
                    "outputPath": output.to_string_lossy(),
                    "document": blank["document"],
                    "operations": [],
                    "bitDepth": null,
                }),
            )
            .expect("a document with no source file exports");
        assert!(std::fs::read(&output).unwrap().starts_with(b"8BPS"));
    }
}
