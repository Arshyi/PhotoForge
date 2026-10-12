//! A DNG's EXIF, XMP and IPTC through the real command boundary: a file is opened as the
//! application opens a RAW, exported under each policy, and the file that comes back is read
//! again.
//!
//! The DNGs here are synthetic, written with the same directory structure a converter writes:
//! the camera's tags in the first directory beside the picture's own and the DNG's, the
//! exposure in an Exif directory, the place in a GPS directory, XMP and IPTC as tags.
use image::GenericImageView;
use photoforge_lib::application::AppState;
use photoforge_lib::metadata::{exif, iptc, scan};
use photoforge_lib::raw::dng::fixtures::DngBuilder;
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
        let app = photoforge_lib::register_raw_flow_commands(mock_builder())
            .manage(AppState::default())
            .build(mock_context(noop_assets()))
            .expect("mock application");
        let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .expect("mock webview");
        Self { app, webview }
    }

    fn call(&self, command: &str, body: Value) -> Result<Value, String> {
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
            Err(error) => Err(error.to_string()),
        }
    }

    fn ok(&self, command: &str, body: Value) -> Value {
        self.call(command, body)
            .unwrap_or_else(|error| panic!("{command} failed: {error}"))
    }
}

const XMP: &[u8] = b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF \
    xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description \
    xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
    xmlns:aux=\"http://ns.adobe.com/exif/1.0/aux/\" aux:SerialNumber=\"BODY-SERIAL-42\">\
    <dc:title>Harbour at dusk</dc:title></rdf:Description></rdf:RDF></x:xmpmeta>";
const IPTC: [u8; 12] = [0x1C, 2, 25, 0, 7, b'h', b'a', b'r', b'b', b'o', b'u', b'r'];

fn text(value: &str) -> Vec<u8> {
    let mut bytes = value.as_bytes().to_vec();
    bytes.push(0);
    bytes
}

fn rational(numerator: u32, denominator: u32) -> Vec<u8> {
    [numerator.to_le_bytes(), denominator.to_le_bytes()].concat()
}

fn samples(width: u32, height: u32) -> Vec<u16> {
    (0..width * height)
        .map(|index| 300 + ((index * 37) % 2000) as u16)
        .collect()
}

/// A camera-shaped DNG: orientation 6, a place, a maker note and serial numbers.
fn dng_with_metadata() -> Vec<u8> {
    let mut builder = DngBuilder::new(24, 16, samples(24, 16))
        .levels(vec![512, 512, 512, 514], (2, 2), 3800)
        .neutral([0.55, 1.0, 0.72])
        .matrix([0.72, 0.18, 0.10, 0.16, 0.74, 0.10, 0.09, 0.20, 0.71])
        .orientation(6)
        .entry(0x02BC, 1, XMP.to_vec())
        .entry(0x83BB, 7, IPTC.to_vec())
        .entry(0x0131, 2, text("Converter 9.8"))
        .entry(0x013B, 2, text("A. Photographer"))
        .entry(0xC62F, 2, text("SN-IFD0-77"))
        .entry(0xC634, 1, b"Adobe\0MakN-PRIVATE-DATA-OF-THE-MAKER".to_vec())
        .exif_entry(0x829A, 5, rational(1, 250))
        .exif_entry(0x8827, 3, 400_u16.to_le_bytes().to_vec())
        .exif_entry(0x9003, 2, text("2026:10:10 14:03:09"))
        .exif_entry(0x927C, 7, b"MAKER-NOTE-WITH-ABSOLUTE-OFFSETS".to_vec())
        .exif_entry(0xA431, 2, text("BODY-SERIAL-42"))
        .exif_entry(0xA434, 2, text("Lens 35mm f/2"))
        .gps_entry(0x0001, 2, text("N"))
        .gps_entry(
            0x0002,
            5,
            [rational(48, 1), rational(51, 1), rational(7, 1)].concat(),
        );
    builder.make = "Acme".into();
    builder.model = "Cam-One".into();
    builder.build()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn open(harness: &Harness, directory: &std::path::Path, bytes: &[u8]) -> (String, Value) {
    let path = directory.join("frame.dng");
    std::fs::write(&path, bytes).unwrap();
    let path = path.to_string_lossy().into_owned();
    let opened = harness.ok("open_raw_image", json!({ "path": path, "requestId": 1 }));
    (path, opened)
}

fn export(harness: &Harness, out: &std::path::Path, policy: &str) -> Value {
    harness.ok(
        "export_image",
        json!({
            "outputPath": out.to_string_lossy(),
            "operations": [],
            "metadataPolicy": policy,
        }),
    )
}

#[test]
fn opening_a_dng_reads_what_the_camera_wrote() {
    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    let (_, opened) = open(&harness, directory.path(), &dng_with_metadata());

    let summary = &opened["metadata"]["sourceMetadata"];
    assert_eq!(summary["hasExif"], true, "{summary}");
    assert_eq!(summary["hasXmp"], true);
    assert_eq!(summary["hasIptc"], true);
    assert_eq!(summary["make"], "Acme");
    assert_eq!(summary["model"], "Cam-One");
    assert_eq!(summary["lens"], "Lens 35mm f/2");
    assert_eq!(summary["taken"], "2026:10:10 14:03:09");
    assert_eq!(summary["hasLocation"], true);
    assert_eq!(summary["hasMakerNote"], true);
    assert_eq!(summary["hasIdentifiers"], true);
    assert_eq!(summary["hasThumbnail"], false);
    // Development leaves the sensor's own way up, so the tag is not spent.
    assert_eq!(summary["orientation"], 6);
    assert_eq!(summary["orientationApplied"], false);

    // The session holds it, so the exports below can use it.
    let state = harness.app.state::<AppState>();
    let session = state.session.lock().unwrap();
    let source = session.as_ref().unwrap().source.source_metadata.clone();
    let source = source.expect("the document holds the source metadata");
    assert_eq!(source.xmp.as_deref().map(Vec::as_slice), Some(XMP));
    assert_eq!(source.iptc.as_deref().map(Vec::as_slice), Some(&IPTC[..]));
}

#[test]
fn the_default_export_removes_the_place_the_serial_numbers_and_an_xmp_that_names_one() {
    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    open(&harness, directory.path(), &dng_with_metadata());

    let out = directory.path().join("shared.jpg");
    let result = export(&harness, &out, "remove_private");
    assert_eq!(result["metadata"]["policy"], "remove_private");
    let notes = result["metadata"]["notes"].as_array().unwrap();
    let said = |needle: &str, kind: &str| {
        notes
            .iter()
            .any(|n| n["kind"] == kind && n["detail"].as_str().unwrap().contains(needle))
    };
    assert!(said("GPS", "removed"), "{notes:?}");
    assert!(said("private notes", "removed"), "{notes:?}");
    assert!(said("serial numbers", "removed"), "{notes:?}");
    assert!(said("XMP, because it", "removed"), "{notes:?}");

    let found = scan::read(&out).unwrap();
    let block = found.exif.as_deref().expect("an EXIF block is written");
    let summary = exif::summarize(block).unwrap();
    assert_eq!(summary.make.as_deref(), Some("Acme"));
    assert_eq!(summary.lens.as_deref(), Some("Lens 35mm f/2"));
    assert!(!summary.has_location);
    assert!(!summary.has_identifiers);
    // The pixels were not turned, so the tag goes with them.
    assert_eq!(summary.orientation, 6);
    assert!(found.xmp.is_none(), "the packet names a serial number");
    assert!(iptc::is_readable(found.iptc.as_deref().unwrap()));
    let bytes = std::fs::read(&out).unwrap();
    for gone in [
        &b"BODY-SERIAL-42"[..],
        b"SN-IFD0-77",
        b"MAKER-NOTE",
        b"PRIVATE-DATA",
    ] {
        assert!(
            !contains(&bytes, gone),
            "{} is in the file",
            String::from_utf8_lossy(gone)
        );
    }
}

#[test]
fn keeping_everything_keeps_what_can_be_moved_and_says_what_cannot() {
    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    open(&harness, directory.path(), &dng_with_metadata());

    let out = directory.path().join("archive.jpg");
    let result = export(&harness, &out, "keep_all");
    let notes = result["metadata"]["notes"].as_array().unwrap();
    // Not carried even here, and said.
    assert!(
        notes
            .iter()
            .any(|n| n["kind"] == "removed"
                && n["detail"].as_str().unwrap().contains("private notes")),
        "{notes:?}"
    );

    let found = scan::read(&out).unwrap();
    let summary = exif::summarize(found.exif.as_deref().unwrap()).unwrap();
    assert!(summary.has_location);
    assert!(summary.has_identifiers);
    assert!(!summary.has_maker_note, "a maker note cannot be moved");
    assert_eq!(summary.taken.as_deref(), Some("2026:10:10 14:03:09"));
    assert_eq!(found.xmp.as_deref().map(Vec::as_slice), Some(XMP));
    assert_eq!(found.iptc.as_deref().map(Vec::as_slice), Some(&IPTC[..]));
    let bytes = std::fs::read(&out).unwrap();
    assert!(!contains(&bytes, b"MAKER-NOTE"));
    assert!(!contains(&bytes, b"PRIVATE-DATA"));
}

#[test]
fn removing_everything_still_writes_the_orientation_the_pixels_did_not_spend() {
    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    let (_, opened) = open(&harness, directory.path(), &dng_with_metadata());

    let out = directory.path().join("bare.jpg");
    export(&harness, &out, "remove_all");
    let found = scan::read(&out).unwrap();
    assert!(found.xmp.is_none() && found.iptc.is_none());
    let summary = exif::summarize(found.exif.as_deref().unwrap()).unwrap();
    assert_eq!(summary.orientation, 6);
    assert_eq!(summary.make, None);
    assert!(!summary.has_location);
    // The picture is the developed sensor, at its own size.
    let (width, height) = image::open(&out).unwrap().dimensions();
    assert_eq!(
        (width, height),
        (
            opened["metadata"]["width"].as_u64().unwrap() as u32,
            opened["metadata"]["height"].as_u64().unwrap() as u32
        )
    );
}

#[test]
fn a_region_opened_on_its_own_carries_the_same_metadata() {
    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    let bytes = dng_with_metadata();
    let (path, whole) = open(&harness, directory.path(), &bytes);
    let region = harness.ok(
        "open_raw_image",
        json!({
            "path": path,
            "requestId": 2,
            "region": { "x": 4, "y": 2, "width": 12, "height": 8 },
        }),
    );
    assert_eq!(region["metadata"]["width"], 12);
    assert_eq!(
        region["metadata"]["sourceMetadata"],
        whole["metadata"]["sourceMetadata"]
    );
}

#[test]
fn a_dng_with_no_exif_directory_still_has_its_make_model_and_orientation() {
    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    let bytes = DngBuilder::new(8, 8, samples(8, 8))
        .levels(vec![0], (1, 1), 4095)
        .build();
    let (_, opened) = open(&harness, directory.path(), &bytes);
    let summary = &opened["metadata"]["sourceMetadata"];
    assert_eq!(summary["make"], "PhotoForge");
    assert_eq!(summary["hasXmp"], false);
    assert_eq!(summary["hasLocation"], false);
    assert_eq!(summary["orientation"], 1);
}

#[test]
fn a_damaged_exif_pointer_costs_the_directory_and_not_the_open() {
    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    // A pointer entry of its own, aimed far past the end of the file.
    let bytes = DngBuilder::new(8, 8, samples(8, 8))
        .levels(vec![0], (1, 1), 4095)
        .entry(0x8769, 4, 0xFFFF_0000_u32.to_le_bytes().to_vec())
        .entry(0x02BC, 1, b"<x:xmpmeta/>".to_vec())
        .build();
    let (_, opened) = open(&harness, directory.path(), &bytes);
    let summary = &opened["metadata"]["sourceMetadata"];
    assert_eq!(summary["make"], "PhotoForge");
    assert_eq!(summary["hasXmp"], true);
    assert_eq!(summary["lens"], Value::Null);
}

/// The same through a real converter's file, when `PHOTOFORGE_RAW_FIXTURES` names the directory
/// (see `scripts/fetch-raw-fixtures.ps1`); skipped otherwise. When `PHOTOFORGE_METADATA_DUMP`
/// also names a directory the three exports are left there for another program to read.
#[test]
fn a_real_dng_exports_with_its_metadata_under_each_policy() {
    let Ok(fixtures) = std::env::var("PHOTOFORGE_RAW_FIXTURES") else {
        eprintln!("skipped: set PHOTOFORGE_RAW_FIXTURES to run real-file checks");
        return;
    };
    let source = std::path::Path::new(&fixtures).join("5G4A9394-uncompressed.DNG");
    if !source.is_file() {
        eprintln!("skipped: {} is not there", source.display());
        return;
    }
    let harness = Harness::new();
    let opened = harness.ok(
        "open_raw_image",
        json!({ "path": source.to_string_lossy(), "requestId": 1 }),
    );
    let summary = &opened["metadata"]["sourceMetadata"];
    assert_eq!(summary["make"], "Canon", "{summary}");
    assert_eq!(summary["model"], "Canon EOS 5D Mark III");
    assert_eq!(summary["hasXmp"], true);
    assert_eq!(summary["hasMakerNote"], true);
    assert_eq!(summary["hasIdentifiers"], true);
    assert_eq!(summary["hasLocation"], false);

    let out_dir = match std::env::var("PHOTOFORGE_METADATA_DUMP") {
        Ok(directory) => std::path::PathBuf::from(directory),
        Err(_) => tempfile::tempdir().unwrap().keep(),
    };
    std::fs::create_dir_all(&out_dir).unwrap();
    for policy in ["keep_all", "remove_private", "remove_all"] {
        let out = out_dir.join(format!("export-{policy}.jpg"));
        let result = export(&harness, &out, policy);
        let found = scan::read(&out).unwrap();
        match policy {
            "keep_all" => {
                let exif = exif::summarize(found.exif.as_deref().unwrap()).unwrap();
                assert_eq!(exif.model.as_deref(), Some("Canon EOS 5D Mark III"));
                assert!(exif.has_identifiers);
                assert_eq!(found.xmp.as_deref().map(Vec::len), Some(6238));
            }
            "remove_private" => {
                let exif = exif::summarize(found.exif.as_deref().unwrap()).unwrap();
                assert_eq!(exif.model.as_deref(), Some("Canon EOS 5D Mark III"));
                assert!(!exif.has_identifiers);
                assert!(found.xmp.is_none());
            }
            _ => {
                assert!(found.xmp.is_none() && found.iptc.is_none());
                assert!(
                    found.exif.is_none(),
                    "upright, so nothing at all is written"
                );
            }
        }
        assert!(
            result["metadata"]["notes"].is_array(),
            "{policy}: {}",
            result["metadata"]
        );
    }
}

/// The path the interface exports by: the layered composite of the document the RAW opened as.
#[test]
fn the_layered_export_of_a_raw_document_carries_the_metadata_too() {
    use photoforge_lib::layers::{LayerContent, LayerDocument};
    use photoforge_lib::operations::structure::new_layer;

    let harness = Harness::new();
    let directory = tempfile::tempdir().unwrap();
    let (_, opened) = open(&harness, directory.path(), &dng_with_metadata());
    let width = opened["metadata"]["width"].as_u64().unwrap() as u32;
    let height = opened["metadata"]["height"].as_u64().unwrap() as u32;
    let pixel_id = opened["backgroundPixelId"].as_str().unwrap().to_string();
    let mut document = LayerDocument::new(width, height);
    document.layers.push(new_layer(
        "layer-1",
        "Photograph",
        LayerContent::Pixel {
            pixel_id,
            width,
            height,
        },
        "t",
    ));

    let out = directory.path().join("layered.jpg");
    let result = harness.ok(
        "export_layer_composite",
        json!({
            "outputPath": out.to_string_lossy(),
            "document": serde_json::to_value(&document).unwrap(),
            "operations": [],
            "profile": "lossless",
            "metadataPolicy": "remove_private",
        }),
    );
    let notes = result["metadata"]["notes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .any(|n| n["kind"] == "removed" && n["detail"].as_str().unwrap().contains("GPS")),
        "{notes:?}"
    );
    let found = scan::read(&out).unwrap();
    let summary = exif::summarize(found.exif.as_deref().unwrap()).unwrap();
    assert_eq!(summary.make.as_deref(), Some("Acme"));
    assert!(!summary.has_location && !summary.has_identifiers);
    assert_eq!(summary.orientation, 6);
}
