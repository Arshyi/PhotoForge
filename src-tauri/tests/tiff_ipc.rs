//! TIFF through the real command boundary: opened as a document with its report, written by the
//! layered export, and refused by name when it is something else.
use photoforge_lib::application::AppState;
use photoforge_lib::metadata::Prepared;
use photoforge_lib::psd::ImportLimits;
use photoforge_lib::tiff::{import_tiff, write, Compression, Spec};
use serde_json::{json, Value};
use std::io::Cursor;
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
            "import_tiff_document",
            json!({ "path": path.to_string_lossy(), "requestId": request_id }),
        )
    }

    fn export(
        &self,
        path: &std::path::Path,
        document: &Value,
        color: Option<Value>,
        policy: Option<&str>,
    ) -> Result<Value, Value> {
        let mut body = json!({
            "outputPath": path.to_string_lossy(),
            "document": document,
            "operations": [],
            "profile": "lossless",
        });
        if let Some(color) = color {
            body["color"] = color;
        }
        if let Some(policy) = policy {
            body["metadataPolicy"] = json!(policy);
        }
        self.call("export_layer_composite", body)
    }
}

const WIDTH: u32 = 12;
const HEIGHT: u32 = 8;

/// A TIFF made by the writer, with a camera's metadata: make, model, orientation 1 and a place.
fn picture(path: &std::path::Path, bits: u16) {
    fn entry(tag: u16, kind: u16, count: u32, value: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
        bytes
    }
    // IFD0 at 8 (make, model, orientation, GPS pointer), the GPS directory at 62, values at 92.
    let mut block = Vec::new();
    block.extend_from_slice(b"II*\0");
    block.extend_from_slice(&8_u32.to_le_bytes());
    block.extend_from_slice(&4_u16.to_le_bytes());
    block.extend_from_slice(&entry(0x010F, 2, 5, 92));
    block.extend_from_slice(&entry(0x0110, 2, 8, 98));
    block.extend_from_slice(&entry(0x0112, 3, 1, 1));
    block.extend_from_slice(&entry(0x8825, 4, 1, 62));
    block.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(block.len(), 62);
    block.extend_from_slice(&2_u16.to_le_bytes());
    block.extend_from_slice(&entry(0x0001, 2, 2, u32::from_le_bytes([b'N', 0, 0, 0])));
    block.extend_from_slice(&entry(0x0002, 5, 3, 106));
    block.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(block.len(), 92);
    block.extend_from_slice(b"Acme\0\0");
    block.extend_from_slice(b"Cam-One\0");
    assert_eq!(block.len(), 106);
    block.extend_from_slice(&[
        48, 0, 0, 0, 1, 0, 0, 0, 51, 0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 1, 0, 0, 0,
    ]);
    let prepared = Prepared {
        exif: Some(block),
        xmp: Some(b"<x:xmpmeta><dc:title>Harbour</dc:title></x:xmpmeta>".to_vec()),
        iptc: None,
    };
    let mut out = Cursor::new(Vec::new());
    write(
        &mut out,
        &Spec {
            width: WIDTH,
            height: HEIGHT,
            bits,
            alpha: false,
            compression: Compression::Deflate,
            icc: None,
            metadata: Some(&prepared),
        },
        |y, row| {
            for x in 0..WIDTH {
                for channel in 0..3_u32 {
                    let value = (x * 20 + y * 7 + channel * 50) % 256;
                    if bits == 8 {
                        row.push(value as u8);
                    } else {
                        row.extend_from_slice(&((value as u16) * 257).to_le_bytes());
                    }
                }
            }
            Ok(())
        },
    )
    .unwrap();
    std::fs::write(path, out.into_inner()).unwrap();
}

#[test]
fn a_tiff_opens_as_an_unsaved_document_with_its_report_and_metadata() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("harbour.tif");
    picture(&path, 8);
    let harness = Harness::new();
    let result = harness.open(&path, 1).expect("the import succeeds");

    let project = &result["project"];
    assert_eq!(project["isCurrent"], true);
    assert_eq!(project["canvasWidth"], WIDTH);
    assert_eq!(project["canvasHeight"], HEIGHT);
    assert_eq!(project["metadata"]["format"], "TIFF");
    assert_eq!(project["metadata"]["filename"], "harbour.tif");
    assert_eq!(project["document"]["precision"], "legacy_srgb8");
    assert_eq!(project["document"]["layers"].as_array().unwrap().len(), 1);

    let report = &result["report"];
    assert_eq!(report["format"], "TIFF");
    assert_eq!(report["bitDepth"], 8);
    assert_eq!(report["colorMode"], "RGB");
    assert_eq!(report["layersImported"], 1);

    // The camera's metadata came with the document, and says what it holds.
    let summary = &project["metadata"]["sourceMetadata"];
    assert_eq!(summary["hasExif"], true, "{summary}");
    assert_eq!(summary["hasXmp"], true);
    assert_eq!(summary["make"], "Acme");
    assert_eq!(summary["model"], "Cam-One");
    assert_eq!(summary["hasLocation"], true);
}

#[test]
fn a_sixteen_bit_tiff_is_a_high_precision_document() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("deep.tiff");
    picture(&path, 16);
    let harness = Harness::new();
    let result = harness.open(&path, 1).unwrap();
    assert_eq!(
        result["project"]["document"]["precision"],
        "linear_srgb_f32"
    );
    assert_eq!(result["report"]["bitDepth"], 16);
}

#[test]
fn the_layered_export_writes_a_tiff_that_reads_back_as_the_document_was() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("in.tif");
    picture(&input, 8);
    let harness = Harness::new();
    let opened = harness.open(&input, 1).unwrap();
    let document = opened["project"]["document"].clone();

    // 8-bit, by the plain path: the picture and its metadata, under the default policy.
    let output = directory.path().join("out.tif");
    let result = harness
        .export(&output, &document, None, None)
        .expect("the export succeeds");
    assert_eq!(result["width"], WIDTH);
    assert_eq!(result["metadata"]["policy"], "remove_private");
    let notes = result["metadata"]["notes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .any(|n| n["kind"] == "removed" && n["detail"].as_str().unwrap().contains("GPS")),
        "{notes:?}"
    );
    let first = import_tiff(&input, ImportLimits::from_resources()).unwrap();
    let again = import_tiff(&output, ImportLimits::from_resources()).unwrap();
    assert_eq!(first.project.pixels[0].1, again.project.pixels[0].1);
    let metadata = again.project.source_metadata.unwrap().summary();
    assert_eq!(metadata.make.as_deref(), Some("Acme"));
    assert!(!metadata.has_location, "the default removes the place");

    // The same document, kept whole and in 16-bit Display P3: the file says which profile.
    let wide = directory.path().join("wide.tif");
    harness
        .export(
            &wide,
            &document,
            Some(json!({ "colorSpace": "display_p3", "bitDepth": 16 })),
            Some("keep_all"),
        )
        .expect("the 16-bit export succeeds");
    let wide_read = import_tiff(&wide, ImportLimits::from_resources()).unwrap();
    assert_eq!(wide_read.report.bit_depth, 16);
    assert!(wide_read
        .report
        .entries
        .iter()
        .any(|entry| entry.detail.contains("embedded profile")));
    assert!(
        wide_read
            .project
            .source_metadata
            .unwrap()
            .summary()
            .has_location
    );
    // Taken through P3 and back through its profile, the picture is the picture.
    let original = &first.project.pixels[0].1;
    let back = &wide_read.project.linear_pixels[0].1;
    let srgb_decode = |c: u8| {
        let c = f32::from(c) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    for (index, pixel) in back.pixels().iter().enumerate() {
        let source = original.get_pixel((index as u32) % WIDTH, (index as u32) / WIDTH);
        for (got, want) in [
            (pixel.red, source[0]),
            (pixel.green, source[1]),
            (pixel.blue, source[2]),
        ] {
            assert!(
                (got - srgb_decode(want)).abs() < 0.01,
                "pixel {index}: {got} vs {}",
                srgb_decode(want)
            );
        }
    }
}

#[test]
fn an_export_never_replaces_the_tiff_it_was_opened_from() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("in.tif");
    picture(&input, 8);
    let before = std::fs::read(&input).unwrap();
    let harness = Harness::new();
    let opened = harness.open(&input, 1).unwrap();
    let document = opened["project"]["document"].clone();
    assert!(harness.export(&input, &document, None, None).is_err());
    assert_eq!(std::fs::read(&input).unwrap(), before);
}

#[test]
fn what_is_not_a_readable_tiff_is_refused_by_name() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::new();

    let png = directory.path().join("not.tif");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n and nothing else").unwrap();
    let error = harness.open(&png, 1).unwrap_err().to_string();
    assert!(error.contains("not a TIFF"), "{error}");

    // CMYK: a header and a directory that says so.
    let mut cmyk = vec![b'I', b'I', 42, 0, 8, 0, 0, 0];
    let entries: [(u16, u16, u32, u32); 5] = [
        (256, 4, 1, 4),
        (257, 4, 1, 4),
        (262, 3, 1, 5),
        (273, 4, 1, 200),
        (277, 3, 1, 4),
    ];
    cmyk.extend((entries.len() as u16).to_le_bytes());
    for (tag, kind, count, value) in entries {
        cmyk.extend(tag.to_le_bytes());
        cmyk.extend(kind.to_le_bytes());
        cmyk.extend(count.to_le_bytes());
        cmyk.extend(value.to_le_bytes());
    }
    cmyk.extend(0_u32.to_le_bytes());
    let path = directory.path().join("press.tif");
    std::fs::write(&path, cmyk).unwrap();
    let error = harness.open(&path, 2).unwrap_err().to_string();
    assert!(error.contains("CMYK"), "{error}");

    let missing = directory.path().join("absent.tif");
    assert!(harness.open(&missing, 3).is_err());
}
