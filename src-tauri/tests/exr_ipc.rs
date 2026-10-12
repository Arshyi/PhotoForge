//! OpenEXR through the real command boundary: the names, the arguments the interface sends, the
//! document, file and reports that come back, and the refusals.
use photoforge_lib::application::AppState;
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

    fn open(&self, path: &std::path::Path, request_id: u64) -> Result<Value, Value> {
        self.call(
            "import_exr_document",
            json!({ "path": path.to_string_lossy(), "requestId": request_id }),
        )
    }

    fn export(
        &self,
        path: &std::path::Path,
        document: &Value,
        depth: Option<u16>,
    ) -> Result<Value, Value> {
        let mut body = json!({
            "outputPath": path.to_string_lossy(),
            "document": document,
            "operations": [],
        });
        if let Some(depth) = depth {
            body["bitDepth"] = json!(depth);
        }
        self.call("export_exr_document", body)
    }
}

/// A 12 × 8 render: a ramp of scene-linear colour with values above one, and a soft alpha edge,
/// stored premultiplied as OpenEXR stores it.
fn write_render(path: &std::path::Path) {
    exr::prelude::write_rgba_file(path, 12, 8, |x, y| {
        let alpha = if x < 2 { 0.5_f32 } else { 1.0 };
        (
            4.0 * x as f32 / 11.0 * alpha,
            y as f32 / 7.0 * alpha,
            0.25 * alpha,
            alpha,
        )
    })
    .unwrap();
}

#[test]
fn an_exr_opens_as_an_unsaved_high_precision_document_with_its_report() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("render.exr");
    write_render(&path);
    let harness = Harness::new();
    let result = harness.open(&path, 1).expect("the import succeeds");

    let project = &result["project"];
    assert_eq!(project["isCurrent"], true);
    assert_eq!(project["canvasWidth"], 12);
    assert_eq!(project["canvasHeight"], 8);
    assert_eq!(project["metadata"]["format"], "OpenEXR");
    assert_eq!(project["metadata"]["filename"], "render.exr");
    assert_eq!(project["document"]["precision"], "linear_srgb_f32");
    let layers = project["document"]["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0]["content"]["type"], "pixel");
    assert_eq!(layers[0]["visible"], true);

    let report = &result["report"];
    assert_eq!(report["format"], "OpenEXR");
    assert_eq!(report["bitDepth"], 32);
    assert_eq!(report["colorMode"], "RGB");
    assert_eq!(report["layersImported"], 1);
    // The one note says what convention the picture was read in.
    let entries = report["entries"].as_array().unwrap();
    assert!(entries.iter().all(|e| e["kind"] == "note"));
    assert!(entries
        .iter()
        .any(|e| e["detail"].as_str().unwrap().contains("scene-linear")));
}

#[test]
fn what_goes_in_comes_out_the_same_through_both_commands() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("render.exr");
    write_render(&input);
    let harness = Harness::new();
    let opened = harness.open(&input, 1).unwrap();
    let document = opened["project"]["document"].clone();

    let output = directory.path().join("again.exr");
    let result = harness
        .export(&output, &document, None)
        .expect("the export succeeds");
    assert_eq!(result["outputPath"], output.to_string_lossy().as_ref());
    assert_eq!(result["report"]["format"], "OpenEXR");
    assert_eq!(result["report"]["bitDepth"], 32);
    assert_eq!(result["report"]["colorMode"], "RGBA");
    assert_eq!(
        result["report"]["bytes"].as_u64().unwrap(),
        std::fs::metadata(&output).unwrap().len()
    );
    // There is no place for a photograph's EXIF in an EXR, and the result carries that account.
    assert!(result["metadata"]["policy"].is_string());
    // Nothing else was left beside the files.
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);

    // The picture in the new file is the picture in the old one, premultiplied both times.
    let read = |path: &std::path::Path| {
        exr::prelude::read_first_rgba_layer_from_file(
            path,
            |resolution, _| {
                vec![(0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32); resolution.width() * resolution.height()]
            },
            |pixels: &mut Vec<(f32, f32, f32, f32)>,
             position,
             (r, g, b, a): (f32, f32, f32, f32)| {
                pixels[position.y() * 12 + position.x()] = (r, g, b, a)
            },
        )
        .unwrap()
        .layer_data
        .channel_data
        .pixels
    };
    let (before, after) = (read(&input), read(&output));
    assert_eq!(before.len(), after.len());
    for (index, (b, a)) in before.iter().zip(&after).enumerate() {
        for (value_in, value_out) in [(b.0, a.0), (b.1, a.1), (b.2, a.2), (b.3, a.3)] {
            assert!(
                (value_in - value_out).abs() < 1e-4,
                "pixel {index}: {b:?} became {a:?}"
            );
        }
    }
}

#[test]
fn half_precision_is_a_choice_of_the_export() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("render.exr");
    write_render(&input);
    let harness = Harness::new();
    let opened = harness.open(&input, 1).unwrap();
    let document = opened["project"]["document"].clone();
    let single = directory.path().join("single.exr");
    let half = directory.path().join("half.exr");
    harness.export(&single, &document, Some(32)).unwrap();
    let result = harness.export(&half, &document, Some(16)).unwrap();
    assert_eq!(result["report"]["bitDepth"], 16);
    assert!(std::fs::metadata(&half).unwrap().len() < std::fs::metadata(&single).unwrap().len());
    let error = harness
        .export(&directory.path().join("eight.exr"), &document, Some(8))
        .unwrap_err();
    assert!(error.to_string().contains("OpenEXR"), "{error}");
    assert!(!directory.path().join("eight.exr").exists());
}

#[test]
fn a_name_that_is_not_exr_or_not_absolute_is_refused_and_nothing_is_written() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("render.exr");
    write_render(&input);
    let harness = Harness::new();
    let document = harness.open(&input, 1).unwrap()["project"]["document"].clone();
    let before = std::fs::read_dir(directory.path()).unwrap().count();
    for name in ["out.png", "out", "out.exr.txt"] {
        let error = harness
            .export(&directory.path().join(name), &document, None)
            .unwrap_err();
        assert_eq!(error["code"], "invalid_output_path", "{name}: {error}");
    }
    let error = harness
        .call(
            "export_exr_document",
            json!({ "outputPath": "out.exr", "document": document, "operations": [] }),
        )
        .unwrap_err();
    assert_eq!(error["code"], "invalid_output_path");
    let missing = directory.path().join("no-such-folder").join("out.exr");
    let error = harness.export(&missing, &document, None).unwrap_err();
    assert_eq!(error["code"], "invalid_output_path");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), before);
}

#[test]
fn an_existing_file_is_replaced_whole_and_an_invalid_document_leaves_it() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("render.exr");
    write_render(&input);
    let harness = Harness::new();
    let document = harness.open(&input, 1).unwrap()["project"]["document"].clone();
    let output = directory.path().join("out.exr");
    std::fs::write(&output, b"what was here").unwrap();
    let mut bad = document.clone();
    bad["canvasWidth"] = json!(0);
    assert!(harness.export(&output, &bad, None).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"what was here");
    harness.export(&output, &document, None).unwrap();
    assert!(std::fs::read(&output)
        .unwrap()
        .starts_with(&[0x76, 0x2f, 0x31, 0x01]));
}

#[test]
fn the_file_the_document_was_opened_from_is_never_replaced_by_its_export() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("render.exr");
    write_render(&input);
    let original = std::fs::read(&input).unwrap();
    let harness = Harness::new();
    let document = harness.open(&input, 1).unwrap()["project"]["document"].clone();

    // The same name, and the other ways of spelling the same file.
    std::fs::create_dir(directory.path().join("sub")).unwrap();
    #[allow(unused_mut)]
    let mut aliases = vec![
        input.clone(),
        directory.path().join("sub").join("..").join("render.exr"),
    ];
    #[cfg(unix)]
    {
        let link = directory.path().join("link.exr");
        std::os::unix::fs::symlink(&input, &link).unwrap();
        let folder = directory.path().join("folder-link");
        std::os::unix::fs::symlink(directory.path(), &folder).unwrap();
        let hard = directory.path().join("hard.exr");
        std::fs::hard_link(&input, &hard).unwrap();
        aliases.extend([link, folder.join("render.exr"), hard]);
    }
    let entries = std::fs::read_dir(directory.path()).unwrap().count();
    for alias in &aliases {
        let error = harness.export(alias, &document, None).unwrap_err();
        assert_eq!(
            error["code"],
            "invalid_output_path",
            "{}: {error}",
            alias.display()
        );
        // It says what to do, and names no path.
        let message = error["message"].as_str().unwrap_or_default();
        assert!(message.contains("different"), "{error}");
        assert!(!message.contains("render.exr"), "{error}");
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
    harness
        .export(&directory.path().join("again.exr"), &document, None)
        .expect("a new name is allowed");
    let taken = directory.path().join("taken.exr");
    std::fs::write(&taken, b"an older export").unwrap();
    harness
        .export(&taken, &document, None)
        .expect("another existing file is replaced");
    assert!(std::fs::read(&taken)
        .unwrap()
        .starts_with(&[0x76, 0x2f, 0x31, 0x01]));
    assert_eq!(std::fs::read(&input).unwrap(), original);
}

#[test]
fn a_document_with_no_source_file_can_be_exported() {
    // A new document and a pasted picture have no file behind them: their source path is empty.
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    let blank = harness
        .call(
            "create_blank_layer_document",
            json!({ "width": 8, "height": 6, "requestId": 1 }),
        )
        .expect("a new document");
    let output = directory.path().join("blank.exr");
    harness
        .export(&output, &blank["document"], None)
        .expect("a document with no source file exports");
    assert!(std::fs::read(&output)
        .unwrap()
        .starts_with(&[0x76, 0x2f, 0x31, 0x01]));
}

#[test]
fn what_is_not_a_readable_exr_is_refused_and_a_missing_file_is_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::new();
    let text = directory.path().join("words.exr");
    std::fs::write(&text, b"not an image at all").unwrap();
    assert!(harness.open(&text, 1).is_err());
    assert!(harness.open(&directory.path().join("nope.exr"), 1).is_err());

    // A real file whose header says it is compressed with a codec the reader lacks is refused by
    // name, before any pixel is read.
    let render = directory.path().join("render.exr");
    write_render(&render);
    let mut bytes = std::fs::read(&render).unwrap();
    let key = b"compression\0compression\0";
    let at = bytes.windows(key.len()).position(|w| w == key).unwrap();
    bytes[at + key.len() + 4] = 8; // DWAA
    let dwaa = directory.path().join("dwaa.exr");
    std::fs::write(&dwaa, &bytes).unwrap();
    let error = harness.open(&dwaa, 1).unwrap_err();
    assert!(error.to_string().contains("DWAA"), "{error}");
}
