//! A photograph's EXIF, XMP and IPTC through the real command boundary: a file is opened as the
//! application opens one, a layered export is asked for under each policy, and the file that
//! comes back is read again.
use image::GenericImageView;
use photoforge_lib::application::{AppState, EditorSession};
use photoforge_lib::infrastructure::load_image;
use photoforge_lib::layers::{LayerContent, LayerDocument};
use photoforge_lib::metadata::{container, exif, iptc, scan};
use photoforge_lib::operations::structure::new_layer;
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

    /// Opens `path` as the application does and returns a one-layer document that shows it.
    fn open(&self, path: &std::path::Path) -> Value {
        let loaded = load_image(path).unwrap();
        let (width, height) = loaded.original.dimensions();
        let state = self.app.state::<AppState>();
        let mut store = state.layers.lock().unwrap();
        store.reset(width, height).unwrap();
        let id = store.register(loaded.original.to_rgba8()).unwrap();
        let mut document = LayerDocument::new(width, height);
        document.layers.push(new_layer(
            "layer-1",
            "Photograph",
            LayerContent::Pixel {
                pixel_id: id,
                width,
                height,
            },
            "t",
        ));
        *state.session.lock().unwrap() = Some(EditorSession {
            source: loaded,
            document_id: 1,
            analysis: None,
        });
        serde_json::to_value(&document).unwrap()
    }
}

/// A little-endian EXIF block with a make, an orientation of 6 and a GPS directory.
fn exif_with_a_place() -> Vec<u8> {
    fn entry(tag: u16, kind: u16, count: u32, value: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
        bytes
    }
    // Header 8; IFD0 (3 entries) at 8..50; GPS IFD (2 entries) at 50..80; values from 80.
    let mut block = Vec::new();
    block.extend_from_slice(b"II*\0");
    block.extend_from_slice(&8_u32.to_le_bytes());
    block.extend_from_slice(&3_u16.to_le_bytes());
    block.extend_from_slice(&entry(0x010F, 2, 5, 80));
    block.extend_from_slice(&entry(0x0112, 3, 1, 6));
    block.extend_from_slice(&entry(0x8825, 4, 1, 50));
    block.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(block.len(), 50);
    block.extend_from_slice(&2_u16.to_le_bytes());
    block.extend_from_slice(&entry(0x0001, 2, 2, u32::from_le_bytes([b'N', 0, 0, 0])));
    block.extend_from_slice(&entry(0x0002, 5, 3, 85));
    block.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(block.len(), 80);
    block.extend_from_slice(b"Acme\0");
    // Latitude 48 51 7, as three rationals.
    block.extend_from_slice(&[
        48, 0, 0, 0, 1, 0, 0, 0, 51, 0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 1, 0, 0, 0,
    ]);
    block
}

fn photograph(dir: &std::path::Path) -> std::path::PathBuf {
    use image::codecs::jpeg::JpegEncoder;
    use image::{ExtendedColorType, ImageEncoder};
    let rgb: Vec<u8> = (0..32 * 16)
        .flat_map(|i| [(i % 32 * 8) as u8, (i / 32 * 16) as u8, 40])
        .collect();
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, 90)
        .write_image(&rgb, 32, 16, ExtendedColorType::Rgb8)
        .unwrap();
    let exif = exif_with_a_place();
    let xmp = b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><dc:title>Harbour</dc:title></x:xmpmeta>";
    let iim = [0x1C, 2, 25, 0, 7, b'h', b'a', b'r', b'b', b'o', b'u', b'r'];
    let (with, _) = container::embed(
        container::Format::Jpeg,
        encoded,
        &container::Embedded {
            exif: Some(&exif),
            xmp: Some(xmp),
            iptc: Some(&iim),
        },
    )
    .unwrap();
    let path = dir.join("phone.jpg");
    std::fs::write(&path, with).unwrap();
    path
}

fn export(
    harness: &Harness,
    document: &Value,
    out: &std::path::Path,
    policy: Option<&str>,
) -> Value {
    let mut body = json!({
        "outputPath": out.to_string_lossy(),
        "document": document,
        "operations": [],
        "profile": "lossless",
    });
    if let Some(policy) = policy {
        body["metadataPolicy"] = json!(policy);
    }
    harness
        .call("export_layer_composite", body)
        .expect("the export succeeds")
}

#[test]
fn an_export_removes_the_place_by_default_and_says_so() {
    let harness = Harness::new();
    let dir = tempfile::tempdir().unwrap();
    let source = photograph(dir.path());
    let document = harness.open(&source);

    let out = dir.path().join("shared.jpg");
    let result = export(&harness, &document, &out, None);
    assert_eq!(result["metadata"]["policy"], "remove_private");
    let notes = result["metadata"]["notes"].as_array().unwrap();
    assert!(notes
        .iter()
        .any(|n| n["kind"] == "removed" && n["detail"].as_str().unwrap().contains("place")));

    let found = scan::read(&out).unwrap();
    let summary = exif::summarize(found.exif.as_deref().unwrap()).unwrap();
    assert!(!summary.has_location);
    assert_eq!(summary.make.as_deref(), Some("Acme"));
    // Turned upright on open, so the tag says upright and the picture is a portrait.
    assert_eq!(summary.orientation, 1);
    assert_eq!(image::open(&out).unwrap().dimensions(), (16, 32));
    assert!(found.xmp.is_some());
    assert!(iptc::is_readable(found.iptc.as_deref().unwrap()));
    // Nowhere in the bytes.
    let bytes = std::fs::read(&out).unwrap();
    assert!(!bytes
        .windows(12)
        .any(|v| v == [48, 0, 0, 0, 1, 0, 0, 0, 51, 0, 0, 0]));
}

#[test]
fn keeping_everything_and_removing_everything_are_both_honoured() {
    let harness = Harness::new();
    let dir = tempfile::tempdir().unwrap();
    let document = harness.open(&photograph(dir.path()));

    let kept = dir.path().join("archive.jpg");
    export(&harness, &document, &kept, Some("keep_all"));
    let found = scan::read(&kept).unwrap();
    let summary = exif::summarize(found.exif.as_deref().unwrap()).unwrap();
    assert!(summary.has_location);
    assert_eq!(summary.orientation, 1);

    let bare = dir.path().join("bare.jpg");
    let result = export(&harness, &document, &bare, Some("remove_all"));
    assert!(scan::read(&bare).unwrap().is_empty());
    let notes = result["metadata"]["notes"].as_array().unwrap();
    assert!(notes
        .iter()
        .any(|n| n["detail"].as_str().unwrap().starts_with("EXIF")));
    assert!(notes.iter().all(|n| n["kind"] != "kept"));
}

#[test]
fn a_document_that_never_had_a_file_has_nothing_to_remove_and_says_nothing() {
    let harness = Harness::new();
    let dir = tempfile::tempdir().unwrap();
    let source = photograph(dir.path());
    let document = harness.open(&source);
    // As after "new document": the session has no source file's metadata.
    {
        let state = harness.app.state::<AppState>();
        let mut session = state.session.lock().unwrap();
        session.as_mut().unwrap().source.source_metadata = None;
    }
    let out = dir.path().join("fresh.jpg");
    let result = export(&harness, &document, &out, Some("keep_all"));
    assert!(result["metadata"]["notes"].as_array().unwrap().is_empty());
    assert!(scan::read(&out).unwrap().is_empty());
}

#[test]
fn a_policy_that_does_not_exist_is_refused() {
    let harness = Harness::new();
    let dir = tempfile::tempdir().unwrap();
    let document = harness.open(&photograph(dir.path()));
    let out = dir.path().join("x.jpg");
    let error = harness
        .call(
            "export_layer_composite",
            json!({
                "outputPath": out.to_string_lossy(),
                "document": document,
                "operations": [],
                "profile": "lossless",
                "metadataPolicy": "keep_some",
            }),
        )
        .unwrap_err();
    assert!(!out.exists(), "{error}");
}
