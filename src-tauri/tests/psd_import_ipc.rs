//! Opening a Photoshop document through the real command boundary: the name, the arguments the
//! interface sends, the document and report that come back, and the refusals.
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

struct LayerSpec {
    name: &'static str,
    /// top, left, bottom, right
    rect: (i32, i32, i32, i32),
    rgb: [u8; 3],
    opacity: u8,
}

/// A small, valid PSD: raw RGB, 8 bits, the layers given, and a flattened image of mid grey.
fn psd(width: u32, height: u32, mode: u16, layers: &[LayerSpec]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&1_u16.to_be_bytes());
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(&3_u16.to_be_bytes());
    out.extend_from_slice(&height.to_be_bytes());
    out.extend_from_slice(&width.to_be_bytes());
    out.extend_from_slice(&8_u16.to_be_bytes());
    out.extend_from_slice(&mode.to_be_bytes());
    out.extend_from_slice(&0_u32.to_be_bytes()); // colour mode data
    out.extend_from_slice(&0_u32.to_be_bytes()); // image resources

    let mut info = Vec::new();
    let mut data = Vec::new();
    info.extend_from_slice(&(layers.len() as i16).to_be_bytes());
    for layer in layers {
        let (top, left, bottom, right) = layer.rect;
        let area = ((right - left) * (bottom - top)) as usize;
        for value in [top, left, bottom, right] {
            info.extend_from_slice(&value.to_be_bytes());
        }
        info.extend_from_slice(&4_u16.to_be_bytes());
        for id in [0_i16, 1, 2, -1] {
            info.extend_from_slice(&id.to_be_bytes());
            info.extend_from_slice(&((2 + area) as u32).to_be_bytes());
        }
        info.extend_from_slice(b"8BIM");
        info.extend_from_slice(b"norm");
        info.extend_from_slice(&[layer.opacity, 0, 0, 0]);
        let mut extra = Vec::new();
        extra.extend_from_slice(&0_u32.to_be_bytes()); // mask
        extra.extend_from_slice(&0_u32.to_be_bytes()); // blending ranges
        extra.push(layer.name.len() as u8);
        extra.extend_from_slice(layer.name.as_bytes());
        while (1 + layer.name.len() + (extra.len() - 9 - layer.name.len())) % 4 != 0 {
            extra.push(0);
        }
        info.extend_from_slice(&(extra.len() as u32).to_be_bytes());
        info.extend_from_slice(&extra);
        for value in [layer.rgb[0], layer.rgb[1], layer.rgb[2], 255] {
            data.extend_from_slice(&0_u16.to_be_bytes());
            data.extend(std::iter::repeat_n(value, area));
        }
    }
    info.extend_from_slice(&data);
    if info.len() % 2 == 1 {
        info.push(0);
    }
    let mut section = Vec::new();
    section.extend_from_slice(&(info.len() as u32).to_be_bytes());
    section.extend_from_slice(&info);
    section.extend_from_slice(&0_u32.to_be_bytes()); // global layer mask info
    out.extend_from_slice(&(section.len() as u32).to_be_bytes());
    out.extend_from_slice(&section);

    out.extend_from_slice(&0_u16.to_be_bytes());
    out.extend(std::iter::repeat_n(128_u8, (width * height * 3) as usize));
    out
}

fn write(directory: &tempfile::TempDir, name: &str, bytes: &[u8]) -> String {
    let path = directory.path().join(name);
    std::fs::write(&path, bytes).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn a_layered_document_opens_as_an_unsaved_layered_document_with_its_report() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(
        &directory,
        "poster.psd",
        &psd(
            16,
            12,
            3,
            &[
                LayerSpec {
                    name: "Sky",
                    rect: (0, 0, 12, 16),
                    rgb: [10, 20, 200],
                    opacity: 255,
                },
                LayerSpec {
                    name: "Sun",
                    rect: (2, 3, 6, 9),
                    rgb: [250, 240, 30],
                    opacity: 128,
                },
            ],
        ),
    );
    let harness = Harness::new();
    let result = harness
        .call(
            "import_psd_document",
            json!({ "path": path, "requestId": 1 }),
        )
        .expect("the import succeeds");

    let project = &result["project"];
    assert_eq!(project["isCurrent"], true);
    assert_eq!(project["canvasWidth"], 16);
    assert_eq!(project["canvasHeight"], 12);
    assert_eq!(project["metadata"]["format"], "PSD");
    assert_eq!(project["metadata"]["filename"], "poster.psd");

    let layers = project["document"]["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0]["name"], "Sky");
    assert_eq!(layers[1]["name"], "Sun");
    let sun_opacity = layers[1]["opacity"].as_f64().unwrap();
    assert!((sun_opacity - 128.0 / 255.0).abs() < 1e-3, "{sun_opacity}");
    assert_eq!(layers[1]["transform"]["translateX"], 3.0);
    assert_eq!(layers[1]["transform"]["translateY"], 2.0);
    assert_eq!(project["document"]["precision"], "legacy_srgb8");

    let report = &result["report"];
    assert_eq!(report["format"], "PSD");
    assert_eq!(report["layersImported"], 2);
    // Nothing was approximated or left out in this file; the report says what it did decide.
    let kinds: Vec<&str> = report["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.iter().all(|kind| *kind == "note"), "{kinds:?}");
}

#[test]
fn a_file_in_a_colour_mode_the_importer_does_not_read_is_refused_with_what_to_do() {
    let directory = tempfile::tempdir().unwrap();
    // Mode 4 is CMYK.
    let path = write(
        &directory,
        "print.psd",
        &psd(
            8,
            8,
            4,
            &[LayerSpec {
                name: "A",
                rect: (0, 0, 8, 8),
                rgb: [1, 2, 3],
                opacity: 255,
            }],
        ),
    );
    let harness = Harness::new();
    let error = harness
        .call(
            "import_psd_document",
            json!({ "path": path, "requestId": 1 }),
        )
        .expect_err("refused");
    let text = error.to_string();
    assert!(text.contains("CMYK"), "{text}");
    assert!(text.contains("RGB"), "{text}");
}

#[test]
fn something_that_is_not_a_photoshop_file_is_refused_and_a_missing_file_is_an_error() {
    let directory = tempfile::tempdir().unwrap();
    let path = write(
        &directory,
        "not.psd",
        b"GIF89a this is not a Photoshop file",
    );
    let harness = Harness::new();
    let error = harness
        .call(
            "import_psd_document",
            json!({ "path": path, "requestId": 1 }),
        )
        .expect_err("refused");
    assert!(error.to_string().contains("not a Photoshop"), "{error}");

    let missing = directory
        .path()
        .join("missing.psd")
        .to_string_lossy()
        .into_owned();
    assert!(harness
        .call(
            "import_psd_document",
            json!({ "path": missing, "requestId": 2 })
        )
        .is_err());
}
