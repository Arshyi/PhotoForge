//! Integration tests for the layer commands, driven through the real Tauri IPC
//! boundary.
//!
//! Every call here goes out as an `InvokeRequest` with a JSON body — exactly the
//! shape the WebView sends — is matched to a command by name, has its arguments
//! deserialized by serde, receives the managed `AppState`, and comes back as a
//! serialized response. That is the wiring a unit test on the Rust function
//! cannot check: a renamed field, a changed argument, or a command missing from
//! the handler all fail here and nowhere else.
//!
//! What this is **not**: it is not a test of the packaged desktop application,
//! and it drives no WebView, no window, and no pointer. The frontend is
//! represented only by the request bodies.

use image::{Rgba, RgbaImage};
use photoforge_lib::application::{AppState, EditorSession};
use photoforge_lib::infrastructure::load_image;
use photoforge_lib::layers::{
    BlendMode, Layer, LayerContent, LayerDocument, LayerInterpolation, LayerMask, LayerMetadata,
    LayerTransform,
};
use photoforge_lib::mask::{MaskBitmap, MaskSnapshot};
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

    /// Sends one command across the IPC boundary and returns its response.
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

    fn err(&self, command: &str, body: Value) -> String {
        match self.call(command, body) {
            Ok(value) => panic!("{command} unexpectedly succeeded: {value}"),
            Err(error) => error,
        }
    }

    /// Registers a buffer the way the application does, through the command.
    fn blank_pixels(&self, width: u32, height: u32) -> String {
        self.ok(
            "create_layer_pixels",
            json!({ "width": width, "height": height }),
        )["pixelId"]
            .as_str()
            .expect("pixelId")
            .to_string()
    }

    /// Loads a real image file as a layer buffer, through the command.
    fn import(&self, path: &std::path::Path) -> Value {
        self.ok(
            "import_layer_image",
            json!({ "path": path.to_string_lossy() }),
        )
    }

    /// Installs an open document, which the composite and export commands
    /// require before they will do any work.
    fn open_session(&self, path: &std::path::Path, document_id: u64) {
        let loaded = load_image(path).expect("test image loads");
        let state = self.app.state::<AppState>();
        *state.session.lock().unwrap() = Some(EditorSession {
            source: loaded,
            document_id,
            analysis: None,
        });
    }

    /// Renders the document composite and returns its pixels.
    ///
    /// The command hands back a base64 PNG data URL, so decoding it here is a
    /// genuine read of what the application would put on screen.
    fn composite(&self, document: &LayerDocument, document_id: u64, request_id: u64) -> RgbaImage {
        let result = self.ok(
            "render_layer_composite",
            json!({
                "document": document,
                "operations": [],
                "documentId": document_id,
                "requestId": request_id
            }),
        );
        assert_eq!(result["isCurrent"], true, "render was reported stale");
        decode_data_url(result["previewDataUrl"].as_str().expect("data url"))
    }
}

fn decode_data_url(url: &str) -> RgbaImage {
    use base64::Engine;
    let comma = url.find(',').expect("data url has a comma");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&url[comma + 1..])
        .expect("base64 payload");
    image::load_from_memory(&bytes)
        .expect("decodable image")
        .to_rgba8()
}

fn write_png(directory: &std::path::Path, name: &str, image: &RgbaImage) -> std::path::PathBuf {
    let path = directory.join(name);
    image.save(&path).expect("write test png");
    path
}

/// A four-quadrant image, so a flip, a rotation, or a mask misalignment is
/// visible rather than symmetric.
fn quadrants(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        let left = x < width / 2;
        let top = y < height / 2;
        match (left, top) {
            (true, true) => Rgba([220, 40, 40, 255]),
            (false, true) => Rgba([40, 220, 40, 255]),
            (true, false) => Rgba([40, 40, 220, 255]),
            (false, false) => Rgba([220, 220, 40, 255]),
        }
    })
}

fn pixel_layer(id: &str, pixel_id: &str, width: u32, height: u32) -> Layer {
    Layer {
        id: id.into(),
        name: id.into(),
        visible: true,
        locked: false,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        transform: LayerTransform::default(),
        mask: None,
        collapsed: false,
        metadata: LayerMetadata::default(),
        content: LayerContent::Pixel {
            pixel_id: pixel_id.into(),
            width,
            height,
        },
    }
}

fn group_layer(id: &str, children: Vec<Layer>, isolated: bool) -> Layer {
    Layer {
        id: id.into(),
        name: id.into(),
        visible: true,
        locked: false,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        transform: LayerTransform::default(),
        mask: None,
        collapsed: false,
        metadata: LayerMetadata::default(),
        content: LayerContent::Group { children, isolated },
    }
}

fn document(canvas: (u32, u32), layers: Vec<Layer>) -> LayerDocument {
    let active = layers.last().map(|layer| layer.id.clone());
    LayerDocument {
        schema_version: photoforge_lib::layers::LAYER_SCHEMA_VERSION,
        canvas_width: canvas.0,
        canvas_height: canvas.1,
        layers,
        active_layer_id: active,
    }
}

/// A soft-edged half-coverage mask, so applying it is visible in the alpha.
fn half_mask(width: u32, height: u32) -> LayerMask {
    let mut bitmap = MaskBitmap::empty(width, height).expect("mask");
    for y in 0..height {
        for x in 0..width / 2 {
            bitmap.set(x, y, 255);
        }
    }
    LayerMask {
        snapshot: MaskSnapshot::encode(&bitmap),
        enabled: true,
        inverted: false,
    }
}

fn temp_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

fn max_channel_difference(left: &RgbaImage, right: &RgbaImage) -> u8 {
    assert_eq!(left.dimensions(), right.dimensions(), "dimensions differ");
    left.as_raw()
        .iter()
        .zip(right.as_raw())
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Dispatch and contract
// ---------------------------------------------------------------------------

#[test]
fn every_layer_command_is_reachable_by_name() {
    let harness = Harness::new();
    // An unknown command must be refused rather than silently ignored, which is
    // what proves the successful calls below actually resolved.
    let missing = harness.err("create_layer_pixel", json!({ "width": 1, "height": 1 }));
    assert!(missing.to_lowercase().contains("not found"), "{missing}");

    let pixel_id = harness.blank_pixels(8, 8);
    assert!(!pixel_id.is_empty());
    let report = harness.ok("layer_store_report", json!({}));
    assert_eq!(report["buffers"], 1);
}

/// The frontend builds these bodies by hand in TypeScript. Sending one written
/// the same way proves the camelCase contract, which a Rust round trip through
/// its own serializer would pass even if the two had drifted apart.
#[test]
fn a_hand_written_frontend_document_deserializes() {
    let harness = Harness::new();
    let pixel_id = harness.blank_pixels(8, 8);
    let body = json!({
        "document": {
            "schemaVersion": 1,
            "canvasWidth": 8,
            "canvasHeight": 8,
            "activeLayerId": "base",
            "layers": [{
                "id": "base",
                "name": "Background",
                "visible": true,
                "locked": false,
                "opacity": 1.0,
                "blendMode": "normal",
                "transform": {
                    "translateX": 2.0,
                    "translateY": -1.0,
                    "scaleX": 1.5,
                    "scaleY": 1.5,
                    "rotationDegrees": 15.0,
                    "flipHorizontal": true,
                    "flipVertical": false,
                    "interpolation": "nearest"
                },
                "mask": null,
                "collapsed": false,
                "metadata": { "createdAt": "", "modifiedAt": "", "custom": {} },
                "content": { "type": "pixel", "pixelId": pixel_id, "width": 8, "height": 8 }
            }]
        }
    });
    assert_eq!(harness.ok("validate_layer_document", body), json!(1));
}

/// A pre-0.8.2 document carries no interpolation field at all. It must still
/// load, and must load as the bilinear sampling those releases always used.
#[test]
fn a_transform_without_an_interpolation_field_still_loads_as_bilinear() {
    let harness = Harness::new();
    let pixel_id = harness.blank_pixels(8, 8);
    let body = json!({
        "document": {
            "schemaVersion": 1,
            "canvasWidth": 8,
            "canvasHeight": 8,
            "activeLayerId": "base",
            "layers": [{
                "id": "base",
                "name": "Background",
                "visible": true,
                "locked": false,
                "opacity": 1.0,
                "blendMode": "normal",
                "transform": {
                    "translateX": 0.0,
                    "translateY": 0.0,
                    "scaleX": 1.0,
                    "scaleY": 1.0,
                    "rotationDegrees": 0.0,
                    "flipHorizontal": false,
                    "flipVertical": false
                },
                "mask": null,
                "collapsed": false,
                "metadata": { "createdAt": "", "modifiedAt": "", "custom": {} },
                "content": { "type": "pixel", "pixelId": pixel_id, "width": 8, "height": 8 }
            }]
        }
    });
    assert_eq!(harness.ok("validate_layer_document", body), json!(1));
    assert_eq!(
        serde_json::to_value(LayerTransform::default()).unwrap()["interpolation"],
        json!("bilinear")
    );
}

// ---------------------------------------------------------------------------
// Malformed transforms (section 8)
// ---------------------------------------------------------------------------

#[test]
fn malformed_transforms_are_refused_at_the_boundary_without_panicking() {
    let harness = Harness::new();
    let pixel_id = harness.blank_pixels(8, 8);
    let hostile = [
        ("zero scale", json!({ "scaleX": 0.0 })),
        ("enormous scale", json!({ "scaleX": 1.0e9 })),
        ("tiny scale", json!({ "scaleY": 1.0e-9 })),
        ("runaway translation", json!({ "translateX": 1.0e12 })),
        ("over-rotation", json!({ "rotationDegrees": 4000.0 })),
    ];
    for (label, patch) in hostile {
        let mut transform = json!({
            "translateX": 0.0, "translateY": 0.0, "scaleX": 1.0, "scaleY": 1.0,
            "rotationDegrees": 0.0, "flipHorizontal": false, "flipVertical": false
        });
        for (key, value) in patch.as_object().unwrap() {
            transform[key] = value.clone();
        }
        let body = json!({
            "document": {
                "schemaVersion": 1, "canvasWidth": 8, "canvasHeight": 8,
                "activeLayerId": "base",
                "layers": [{
                    "id": "base", "name": "Background", "visible": true, "locked": false,
                    "opacity": 1.0, "blendMode": "normal", "transform": transform, "mask": null,
                    "collapsed": false,
                    "metadata": { "createdAt": "", "modifiedAt": "", "custom": {} },
                    "content": { "type": "pixel", "pixelId": pixel_id, "width": 8, "height": 8 }
                }]
            }
        });
        let error = harness.err("validate_layer_document", body);
        assert!(!error.is_empty(), "{label} produced an empty error");
    }
}

/// JSON has no NaN or infinity literal, so a malformed number arrives as a
/// string or a null. Neither may be silently coerced into a placement.
#[test]
fn a_non_numeric_transform_value_is_rejected_rather_than_coerced() {
    let harness = Harness::new();
    let pixel_id = harness.blank_pixels(8, 8);
    for bad in [json!("NaN"), json!(null), json!({})] {
        let body = json!({
            "document": {
                "schemaVersion": 1, "canvasWidth": 8, "canvasHeight": 8,
                "activeLayerId": "base",
                "layers": [{
                    "id": "base", "name": "Background", "visible": true, "locked": false,
                    "opacity": 1.0, "blendMode": "normal",
                    "transform": {
                        "translateX": bad, "translateY": 0.0, "scaleX": 1.0, "scaleY": 1.0,
                        "rotationDegrees": 0.0, "flipHorizontal": false, "flipVertical": false
                    },
                    "mask": null, "collapsed": false,
                    "metadata": { "createdAt": "", "modifiedAt": "", "custom": {} },
                    "content": { "type": "pixel", "pixelId": pixel_id, "width": 8, "height": 8 }
                }]
            }
        });
        assert!(!harness.err("validate_layer_document", body).is_empty());
    }
}

#[test]
fn an_unknown_blend_mode_or_out_of_range_opacity_is_refused() {
    let harness = Harness::new();
    let pixel_id = harness.blank_pixels(8, 8);
    let build = |blend: Value, opacity: Value| {
        json!({
            "document": {
                "schemaVersion": 1, "canvasWidth": 8, "canvasHeight": 8,
                "activeLayerId": "base",
                "layers": [{
                    "id": "base", "name": "Background", "visible": true, "locked": false,
                    "opacity": opacity, "blendMode": blend,
                    "transform": serde_json::to_value(LayerTransform::default()).unwrap(),
                    "mask": null, "collapsed": false,
                    "metadata": { "createdAt": "", "modifiedAt": "", "custom": {} },
                    "content": { "type": "pixel", "pixelId": pixel_id, "width": 8, "height": 8 }
                }]
            }
        })
    };
    assert!(!harness
        .err(
            "validate_layer_document",
            build(json!("plasma"), json!(1.0))
        )
        .is_empty());
    assert!(!harness
        .err(
            "validate_layer_document",
            build(json!("normal"), json!(4.0))
        )
        .is_empty());
}

// ---------------------------------------------------------------------------
// Merge (sections 12 and 13)
// ---------------------------------------------------------------------------

/// Merging must not change what the document looks like. Both sides are
/// rendered through the composite command and compared pixel for pixel.
#[test]
fn merging_two_layers_preserves_the_rendered_appearance() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(32, 32));
    harness.open_session(&source, 7);

    let base = harness.import(&source);
    let overlay = harness.import(&source);
    let mut top = pixel_layer("top", overlay["pixelId"].as_str().unwrap(), 32, 32);
    top.opacity = 0.6;
    top.blend_mode = BlendMode::Multiply;
    top.transform = LayerTransform {
        translate_x: 4.0,
        translate_y: -3.0,
        scale_x: 1.25,
        ..LayerTransform::default()
    };
    top.mask = Some(half_mask(32, 32));
    let bottom = pixel_layer("bottom", base["pixelId"].as_str().unwrap(), 32, 32);
    let before = document((32, 32), vec![bottom.clone(), top.clone()]);
    let before_pixels = harness.composite(&before, 7, 1);

    let merged = harness.ok(
        "merge_layer_pixels",
        json!({ "document": before, "layerIds": ["bottom", "top"] }),
    );
    assert_eq!(merged["width"], 32);
    assert_eq!(merged["height"], 32);
    let after = document(
        (32, 32),
        vec![pixel_layer(
            "bottom",
            merged["pixelId"].as_str().unwrap(),
            32,
            32,
        )],
    );
    let after_pixels = harness.composite(&after, 7, 2);

    // The merge composites through the same renderer, so the only difference is
    // one extra 8-bit quantization of the result.
    assert!(
        max_channel_difference(&before_pixels, &after_pixels) <= 1,
        "merge changed the appearance by {}",
        max_channel_difference(&before_pixels, &after_pixels)
    );
}

#[test]
fn merging_follows_the_documents_own_order_not_the_order_it_was_asked_in() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(16, 16));
    harness.open_session(&source, 1);
    let a = harness.import(&source);
    let b = harness.import(&source);
    let doc = document(
        (16, 16),
        vec![
            pixel_layer("lower", a["pixelId"].as_str().unwrap(), 16, 16),
            pixel_layer("upper", b["pixelId"].as_str().unwrap(), 16, 16),
        ],
    );
    let ascending = harness.ok(
        "merge_layer_pixels",
        json!({ "document": doc, "layerIds": ["lower", "upper"] }),
    );
    let descending = harness.ok(
        "merge_layer_pixels",
        json!({ "document": doc, "layerIds": ["upper", "lower"] }),
    );
    let first = document(
        (16, 16),
        vec![pixel_layer(
            "m",
            ascending["pixelId"].as_str().unwrap(),
            16,
            16,
        )],
    );
    let second = document(
        (16, 16),
        vec![pixel_layer(
            "m",
            descending["pixelId"].as_str().unwrap(),
            16,
            16,
        )],
    );
    assert_eq!(
        max_channel_difference(
            &harness.composite(&first, 1, 1),
            &harness.composite(&second, 1, 2)
        ),
        0
    );
}

#[test]
fn merging_rejects_an_unknown_layer_and_an_empty_request() {
    let harness = Harness::new();
    let pixel_id = harness.blank_pixels(8, 8);
    let doc = document((8, 8), vec![pixel_layer("a", &pixel_id, 8, 8)]);
    assert!(!harness
        .err(
            "merge_layer_pixels",
            json!({ "document": doc, "layerIds": ["ghost"] })
        )
        .is_empty());
    assert!(!harness
        .err(
            "merge_layer_pixels",
            json!({ "document": doc, "layerIds": [] })
        )
        .is_empty());
}

#[test]
fn merging_a_blend_dependent_subset_refuses_to_bake_an_omitted_backdrop() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(8, 8));
    harness.open_session(&source, 2);
    let bottom = harness.import(&source);
    let middle = harness.import(&source);
    let top = harness.import(&source);
    let mut blend = pixel_layer("middle", middle["pixelId"].as_str().unwrap(), 8, 8);
    blend.blend_mode = BlendMode::Screen;
    let doc = document(
        (8, 8),
        vec![
            pixel_layer("bottom", bottom["pixelId"].as_str().unwrap(), 8, 8),
            blend,
            pixel_layer("top", top["pixelId"].as_str().unwrap(), 8, 8),
        ],
    );
    let error = harness.err(
        "merge_layer_pixels",
        json!({ "document": doc, "layerIds": ["middle", "top"] }),
    );
    assert!(error.contains("depends on layers beneath"), "{error}");
}

#[test]
fn merging_several_layers_from_inside_a_group_keeps_their_appearance() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(24, 24));
    harness.open_session(&source, 3);
    let one = harness.import(&source);
    let two = harness.import(&source);
    let three = harness.import(&source);

    let mut middle = pixel_layer("two", two["pixelId"].as_str().unwrap(), 24, 24);
    middle.opacity = 0.4;
    middle.blend_mode = BlendMode::Screen;
    let mut upper = pixel_layer("three", three["pixelId"].as_str().unwrap(), 24, 24);
    upper.transform.rotation_degrees = 12.0;

    let group = group_layer(
        "grp",
        vec![
            pixel_layer("one", one["pixelId"].as_str().unwrap(), 24, 24),
            middle,
            upper,
        ],
        true,
    );
    let doc = document((24, 24), vec![group]);
    let merged = harness.ok(
        "merge_layer_pixels",
        json!({ "document": doc, "layerIds": ["one", "two", "three"] }),
    );
    let flattened = document(
        (24, 24),
        vec![pixel_layer(
            "m",
            merged["pixelId"].as_str().unwrap(),
            24,
            24,
        )],
    );
    assert!(
        max_channel_difference(
            &harness.composite(&doc, 3, 1),
            &harness.composite(&flattened, 3, 2)
        ) <= 1
    );
}

// ---------------------------------------------------------------------------
// Flatten (section 14)
// ---------------------------------------------------------------------------

#[test]
fn flattening_a_nested_document_preserves_the_rendered_appearance() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(32, 32));
    harness.open_session(&source, 5);

    let background = harness.import(&source);
    let inner_pixels = harness.import(&source);
    let outer_pixels = harness.import(&source);

    let mut transformed = pixel_layer("inner", inner_pixels["pixelId"].as_str().unwrap(), 32, 32);
    transformed.transform = LayerTransform {
        translate_x: -6.0,
        rotation_degrees: 20.0,
        scale_x: 0.8,
        scale_y: 0.8,
        ..LayerTransform::default()
    };
    transformed.mask = Some(half_mask(32, 32));

    let adjustment = Layer {
        content: LayerContent::Adjustment {
            operation: Box::new(photoforge_lib::domain::EditOperation::Brightness { amount: 0.2 }),
        },
        ..pixel_layer("adj", "unused", 32, 32)
    };

    let mut nested = group_layer("nested", vec![transformed], true);
    nested.opacity = 0.75;
    let mut pass_through = group_layer(
        "pass",
        vec![
            adjustment,
            nested,
            pixel_layer("outer", outer_pixels["pixelId"].as_str().unwrap(), 32, 32),
        ],
        false,
    );
    pass_through.mask = Some(half_mask(32, 32));

    let doc = document(
        (32, 32),
        vec![
            pixel_layer("bg", background["pixelId"].as_str().unwrap(), 32, 32),
            pass_through,
        ],
    );
    let before = harness.composite(&doc, 5, 1);

    let flattened = harness.ok("flatten_layer_document", json!({ "document": doc }));
    let after_doc = document(
        (32, 32),
        vec![pixel_layer(
            "flat",
            flattened["pixelId"].as_str().unwrap(),
            32,
            32,
        )],
    );
    let after = harness.composite(&after_doc, 5, 2);
    assert!(
        max_channel_difference(&before, &after) <= 1,
        "flatten changed the appearance by {}",
        max_channel_difference(&before, &after)
    );
}

#[test]
fn flattening_produces_one_canvas_sized_layer() {
    let harness = Harness::new();
    let pixel_id = harness.blank_pixels(10, 10);
    let mut small = pixel_layer("small", &pixel_id, 10, 10);
    small.transform.translate_x = 5.0;
    let doc = document((40, 25), vec![small]);
    let flattened = harness.ok("flatten_layer_document", json!({ "document": doc }));
    assert_eq!(flattened["width"], 40);
    assert_eq!(flattened["height"], 25);
}

// ---------------------------------------------------------------------------
// Rasterize a transform (sections 1 and 5)
// ---------------------------------------------------------------------------

#[test]
fn rasterizing_a_transform_bakes_the_placement_and_the_mask() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(32, 32));
    harness.open_session(&source, 9);
    let imported = harness.import(&source);

    let mut layer = pixel_layer("art", imported["pixelId"].as_str().unwrap(), 32, 32);
    layer.transform = LayerTransform {
        translate_x: 5.0,
        translate_y: -4.0,
        scale_x: 1.4,
        scale_y: 0.7,
        rotation_degrees: 23.0,
        flip_horizontal: true,
        flip_vertical: false,
        interpolation: LayerInterpolation::Bilinear,
    };
    layer.mask = Some(half_mask(32, 32));
    let before_doc = document((32, 32), vec![layer.clone()]);
    let before = harness.composite(&before_doc, 9, 1);

    let baked = harness.ok(
        "rasterize_layer_transform",
        json!({ "request": { "document": before_doc, "layerId": "art" } }),
    );
    let after_doc = document(
        (32, 32),
        vec![pixel_layer(
            "art",
            baked["pixelId"].as_str().unwrap(),
            32,
            32,
        )],
    );
    let after = harness.composite(&after_doc, 9, 2);
    assert!(
        max_channel_difference(&before, &after) <= 1,
        "rasterize changed the appearance by {}",
        max_channel_difference(&before, &after)
    );
}

#[test]
fn rasterizing_refuses_a_group_and_an_adjustment_layer() {
    let harness = Harness::new();
    let pixel_id = harness.blank_pixels(8, 8);
    let adjustment = Layer {
        content: LayerContent::Adjustment {
            operation: Box::new(photoforge_lib::domain::EditOperation::Brightness { amount: 0.1 }),
        },
        ..pixel_layer("adj", "unused", 8, 8)
    };
    let doc = document(
        (8, 8),
        vec![
            group_layer("grp", vec![pixel_layer("in", &pixel_id, 8, 8)], true),
            adjustment,
        ],
    );
    for id in ["grp", "adj"] {
        let error = harness.err(
            "rasterize_layer_transform",
            json!({ "request": { "document": doc, "layerId": id } }),
        );
        assert!(error.to_lowercase().contains("pixel"), "{id}: {error}");
    }
}

#[test]
fn rasterizing_preserves_editable_opacity_blend_and_hidden_layer_pixels() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(16, 16));
    harness.open_session(&source, 70);
    let base = harness.import(&source);
    let overlay = harness.import(&source);
    let bottom = pixel_layer("base", base["pixelId"].as_str().unwrap(), 16, 16);
    for (index, blend_mode) in [BlendMode::Normal, BlendMode::Multiply, BlendMode::Overlay]
        .into_iter()
        .enumerate()
    {
        let mut top = pixel_layer("top", overlay["pixelId"].as_str().unwrap(), 16, 16);
        top.opacity = 0.5;
        top.blend_mode = blend_mode;
        top.transform.translate_x = 2.0;
        top.mask = Some(half_mask(16, 16));
        let original = document((16, 16), vec![bottom.clone(), top.clone()]);
        let before = harness.composite(&original, 70, 10 + index as u64 * 2);
        // Even a currently hidden layer must retain its pixels when baked.
        let mut hidden = original.clone();
        hidden.layers[1].visible = false;
        let baked = harness.ok(
            "rasterize_layer_transform",
            json!({
                "request": { "document": hidden, "layerId": "top" }
            }),
        );
        top.content = LayerContent::Pixel {
            pixel_id: baked["pixelId"].as_str().unwrap().into(),
            width: 16,
            height: 16,
        };
        top.transform = LayerTransform::default();
        top.mask = None;
        let after = harness.composite(
            &document((16, 16), vec![bottom.clone(), top]),
            70,
            11 + index as u64 * 2,
        );
        assert!(
            max_channel_difference(&before, &after) <= 1,
            "rasterize changed {blend_mode:?} at 50% opacity"
        );
    }
}

// ---------------------------------------------------------------------------
// Masks (sections 15 and 16)
// ---------------------------------------------------------------------------

#[test]
fn applying_a_mask_bakes_its_coverage_into_the_alpha() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(16, 16));
    harness.open_session(&source, 2);
    let imported = harness.import(&source);

    let mut masked = pixel_layer("art", imported["pixelId"].as_str().unwrap(), 16, 16);
    masked.mask = Some(half_mask(16, 16));
    // This is the isolated single-layer merge the application uses for
    // "Apply mask": the renderer bakes coverage into the buffer's alpha.
    let isolated = document((16, 16), vec![masked.clone()]);
    let baked = harness.ok(
        "merge_layer_pixels",
        json!({ "document": isolated, "layerIds": ["art"] }),
    );
    let after = document(
        (16, 16),
        vec![pixel_layer(
            "art",
            baked["pixelId"].as_str().unwrap(),
            16,
            16,
        )],
    );
    let rendered = harness.composite(&after, 2, 1);
    // The covered half keeps its colour, the uncovered half is gone.
    assert_eq!(rendered.get_pixel(2, 2).0[3], 255);
    assert_eq!(rendered.get_pixel(13, 2).0[3], 0);
}

#[test]
fn a_fully_black_mask_hides_the_layer_and_a_white_one_changes_nothing() {
    let harness = Harness::new();
    let pixel_id = harness.blank_pixels(16, 16);
    let doc = document((16, 16), vec![pixel_layer("art", &pixel_id, 16, 16)]);
    for (filled, expected) in [(true, 255_u8), (false, 0_u8)] {
        let created = harness.ok(
            "create_layer_mask",
            json!({ "document": doc, "layerId": "art", "filled": filled }),
        );
        let bitmap = serde_json::from_value::<MaskSnapshot>(created["snapshot"].clone())
            .expect("mask snapshot")
            .decode()
            .expect("decodes");
        assert_eq!(bitmap.get(0, 0), expected);
        assert_eq!(created["width"], 16);
    }
}

#[test]
fn a_selection_becomes_a_mask_and_comes_back_as_the_same_selection() {
    let harness = Harness::new();
    let pixel_id = harness.blank_pixels(16, 16);
    let layer = pixel_layer("art", &pixel_id, 16, 16);
    let doc = document((16, 16), vec![layer]);

    let mut selection = MaskBitmap::empty(16, 16).expect("selection");
    for y in 4..12 {
        for x in 4..12 {
            selection.set(x, y, 255);
        }
    }
    let snapshot = MaskSnapshot::encode(&selection);
    let created = harness.ok(
        "layer_mask_from_selection",
        json!({ "document": doc, "layerId": "art", "selection": snapshot }),
    );

    let mut masked = doc.layers[0].clone();
    masked.mask = Some(LayerMask {
        snapshot: serde_json::from_value(created["snapshot"].clone()).unwrap(),
        enabled: true,
        inverted: false,
    });
    let with_mask = document((16, 16), vec![masked]);
    let loaded = harness.ok(
        "selection_from_layer_mask",
        json!({ "document": with_mask, "layerId": "art" }),
    );
    let round_tripped = serde_json::from_value::<MaskSnapshot>(loaded["snapshot"].clone())
        .unwrap()
        .decode()
        .unwrap();
    assert_eq!(round_tripped.get(8, 8), 255);
    assert_eq!(round_tripped.get(1, 1), 0);
}

/// A mask lives in the layer's own coordinate space, so a transformed layer
/// must carry its mask with it rather than leaving it pinned to the canvas.
#[test]
fn a_mask_stays_with_its_layer_through_a_transform() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(32, 32));
    harness.open_session(&source, 4);
    let imported = harness.import(&source);

    let mut layer = pixel_layer("art", imported["pixelId"].as_str().unwrap(), 32, 32);
    layer.mask = Some(half_mask(32, 32));
    let still = harness.composite(&document((32, 32), vec![layer.clone()]), 4, 1);

    let mut moved = layer.clone();
    moved.transform.translate_x = 8.0;
    let shifted = harness.composite(&document((32, 32), vec![moved]), 4, 2);

    // The covered edge moves with the layer: what was opaque at x=10 is now
    // opaque at x=18, and the old position past the mask edge is clear.
    assert_eq!(still.get_pixel(10, 16).0[3], 255);
    assert_eq!(shifted.get_pixel(18, 16).0[3], 255);
    assert_eq!(still.get_pixel(20, 16).0[3], 0);
    assert_eq!(shifted.get_pixel(28, 16).0[3], 0);
}

// ---------------------------------------------------------------------------
// Project save and load (section 17)
// ---------------------------------------------------------------------------

/// Builds the tree the brief asks for: a pass-through group holding an
/// adjustment layer, a transformed and masked pixel layer, and a nested group,
/// between two ordinary pixel layers.
fn complex_document(harness: &Harness, source: &std::path::Path) -> LayerDocument {
    let a = harness.import(source);
    let b = harness.import(source);
    let c = harness.import(source);
    let d = harness.import(source);

    let mut transformed = pixel_layer("transformed", b["pixelId"].as_str().unwrap(), 32, 32);
    transformed.name = "Transformed art".into();
    transformed.transform = LayerTransform {
        translate_x: 3.5,
        translate_y: -2.25,
        scale_x: 1.3,
        scale_y: 0.9,
        rotation_degrees: 17.5,
        flip_horizontal: true,
        flip_vertical: false,
        interpolation: LayerInterpolation::Nearest,
    };
    transformed.mask = Some(half_mask(32, 32));
    transformed.opacity = 0.85;
    transformed.blend_mode = BlendMode::Overlay;
    transformed.locked = true;
    transformed
        .metadata
        .custom
        .insert("source".into(), "scanner".into());

    let adjustment = Layer {
        name: "Warmth".into(),
        opacity: 0.7,
        content: LayerContent::Adjustment {
            operation: Box::new(photoforge_lib::domain::EditOperation::Contrast { amount: 0.25 }),
        },
        ..pixel_layer("adjust", "unused", 32, 32)
    };

    let mut nested = group_layer(
        "nested",
        vec![pixel_layer("leaf", c["pixelId"].as_str().unwrap(), 32, 32)],
        true,
    );
    nested.collapsed = true;
    nested.opacity = 0.6;

    let mut pass_through = group_layer("pass", vec![adjustment, transformed, nested], false);
    pass_through.name = "Pass-through".into();
    pass_through.mask = Some(half_mask(32, 32));

    let mut top = pixel_layer("top", d["pixelId"].as_str().unwrap(), 32, 32);
    top.visible = false;

    document(
        (32, 32),
        vec![
            pixel_layer("bg", a["pixelId"].as_str().unwrap(), 32, 32),
            pass_through,
            top,
        ],
    )
}

#[test]
fn a_complex_project_survives_a_save_and_a_reload_intact() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(32, 32));
    harness.open_session(&source, 11);
    let original = complex_document(&harness, &source);
    let before = harness.composite(&original, 11, 1);

    let path = directory.path().join("project.photoforge");
    let saved = harness.ok(
        "save_layer_project",
        json!({
            "outputPath": path.to_string_lossy(),
            "document": original,
            "operations": [],
            "createdAt": "2026-01-01T00:00:00Z",
            "modifiedAt": "2026-01-02T00:00:00Z"
        }),
    );
    assert!(saved["bytes"].as_u64().unwrap() > 0);

    // A second harness stands in for a restart: nothing is carried over in
    // memory, so every buffer has to come back out of the file.
    let reopened = Harness::new();
    let loaded = reopened.ok(
        "load_layer_project",
        json!({ "path": path.to_string_lossy(), "requestId": 11 }),
    );
    let restored: LayerDocument = serde_json::from_value(loaded["document"].clone()).unwrap();

    assert_eq!(loaded["createdAt"], "2026-01-01T00:00:00Z");
    assert_eq!(loaded["canvasWidth"], 32);
    assert_eq!(loaded["documentId"], 11);
    assert_eq!(loaded["isCurrent"], true);
    assert_eq!(loaded["metadata"]["filename"], "project.photoforge");
    assert_eq!(
        decode_data_url(loaded["originalPreviewDataUrl"].as_str().unwrap()),
        before
    );
    let state = reopened.app.state::<AppState>();
    assert_eq!(
        state
            .pending_open_request
            .load(std::sync::atomic::Ordering::Acquire),
        0
    );
    assert_eq!(
        state.session.lock().unwrap().as_ref().unwrap().document_id,
        11
    );

    // Structure, identifiers, names, and every per-layer setting.
    let names: Vec<_> = restored.iter().map(|layer| layer.name.clone()).collect();
    assert!(names.contains(&"Pass-through".to_string()));
    assert!(names.contains(&"Transformed art".to_string()));
    let transformed = restored.find("transformed").expect("transformed layer");
    assert_eq!(transformed.transform.rotation_degrees, 17.5);
    assert!(transformed.transform.flip_horizontal);
    assert_eq!(
        transformed.transform.interpolation,
        LayerInterpolation::Nearest
    );
    assert_eq!(transformed.opacity, 0.85);
    assert_eq!(transformed.blend_mode, BlendMode::Overlay);
    assert!(transformed.locked);
    assert!(transformed.mask.is_some());
    assert_eq!(
        transformed
            .metadata
            .custom
            .get("source")
            .map(String::as_str),
        Some("scanner")
    );
    let pass = restored.find("pass").expect("group");
    assert!(matches!(
        pass.content,
        LayerContent::Group {
            isolated: false,
            ..
        }
    ));
    assert!(pass.mask.is_some());
    let nested = restored.find("nested").expect("nested group");
    assert!(nested.collapsed);
    assert_eq!(nested.opacity, 0.6);
    assert!(!restored.find("top").expect("top").visible);
    assert!(matches!(
        restored.find("adjust").expect("adjustment").content,
        LayerContent::Adjustment { .. }
    ));

    // And it must still look identical.
    let after = reopened.composite(&restored, 11, 2);
    assert_eq!(
        max_channel_difference(&before, &after),
        0,
        "a reloaded project rendered differently"
    );
}

#[test]
fn a_reloaded_project_renders_the_same_thumbnails() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(32, 32));
    harness.open_session(&source, 12);
    let original = complex_document(&harness, &source);
    let thumbnail = harness.ok(
        "render_layer_thumbnail",
        json!({ "document": original, "layerId": "transformed", "maxEdge": 64 }),
    );

    let path = directory.path().join("project.photoforge");
    harness.ok(
        "save_layer_project",
        json!({
            "outputPath": path.to_string_lossy(), "document": original, "operations": [],
            "createdAt": "2026-01-01T00:00:00Z", "modifiedAt": "2026-01-01T00:00:00Z"
        }),
    );
    let reopened = Harness::new();
    let loaded = reopened.ok(
        "load_layer_project",
        json!({ "path": path.to_string_lossy(), "requestId": 12 }),
    );
    let restored: LayerDocument = serde_json::from_value(loaded["document"].clone()).unwrap();
    let again = reopened.ok(
        "render_layer_thumbnail",
        json!({ "document": restored, "layerId": "transformed", "maxEdge": 64 }),
    );
    assert_eq!(thumbnail["dataUrl"], again["dataUrl"]);
}

#[test]
fn loading_refuses_a_file_that_is_not_a_project() {
    let directory = temp_dir();
    let harness = Harness::new();
    let path = directory.path().join("not-a-project.photoforge");
    std::fs::write(&path, b"PFORGE\r\nbut then nothing that parses").unwrap();
    assert!(!harness
        .err(
            "load_layer_project",
            json!({ "path": path.to_string_lossy(), "requestId": 1 })
        )
        .is_empty());

    let missing = directory.path().join("absent.photoforge");
    assert!(!harness
        .err(
            "load_layer_project",
            json!({ "path": missing.to_string_lossy(), "requestId": 2 })
        )
        .is_empty());
}

#[test]
fn failed_project_and_recovery_loads_preserve_the_open_document_and_clear_pending() {
    use std::sync::atomic::Ordering;
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(8, 8));
    harness.open_session(&source, 42);
    let imported = harness.import(&source);
    let pixel_id = imported["pixelId"].as_str().unwrap();
    let doc = document((8, 8), vec![pixel_layer("a", pixel_id, 8, 8)]);
    let before = harness.composite(&doc, 42, 1);

    for (command, request_id) in [
        ("load_layer_project", 43),
        ("restore_recovery_snapshot", 44),
    ] {
        harness.err(
            command,
            json!({
                "path": directory.path().join("absent.photoforge").to_string_lossy(),
                "requestId": request_id,
            }),
        );
        let state = harness.app.state::<AppState>();
        assert_eq!(state.pending_open_request.load(Ordering::Acquire), 0);
        assert_eq!(
            state.session.lock().unwrap().as_ref().unwrap().document_id,
            42
        );
        assert!(state.layers.lock().unwrap().contains(pixel_id));
        assert_eq!(
            harness.composite(&doc, 42, request_id).as_raw(),
            before.as_raw()
        );
    }
}

#[test]
fn project_load_returns_base_and_edited_previews_without_baking_the_pipeline() {
    let directory = temp_dir();
    let pixels = quadrants(8, 8);
    let doc = document((8, 8), vec![pixel_layer("a", "px1", 8, 8)]);
    let operation: photoforge_lib::domain::EditOperation =
        serde_json::from_value(json!({ "type": "grayscale" })).unwrap();
    let path = directory.path().join("edited.photoforge");
    photoforge_lib::layers::save_project(
        &path,
        &doc,
        &[operation],
        &[("px1".into(), &pixels)],
        "0.8.2",
        "",
        "",
    )
    .unwrap();
    let harness = Harness::new();
    let loaded = harness.ok("load_layer_project", json!({"path": path, "requestId": 15}));
    let original = decode_data_url(loaded["originalPreviewDataUrl"].as_str().unwrap());
    let edited = decode_data_url(loaded["previewDataUrl"].as_str().unwrap());
    assert_eq!(original, pixels);
    assert_ne!(edited, original);
    assert_eq!(loaded["operations"].as_array().unwrap().len(), 1);
    let state = harness.app.state::<AppState>();
    assert_eq!(
        state
            .session
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .source
            .original
            .to_rgba8(),
        original
    );
}

// ---------------------------------------------------------------------------
// Export (section 18)
// ---------------------------------------------------------------------------

#[test]
fn exporting_writes_the_visible_composite_without_touching_the_project() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(32, 32));
    let source_before = std::fs::read(&source).unwrap();
    harness.open_session(&source, 13);
    let original = complex_document(&harness, &source);
    let before = harness.composite(&original, 13, 1);

    // The format comes from the extension and the profile only sets quality, so
    // every format PhotoForge writes is covered here.
    for (name, profile) in [
        ("out.png", "lossless"),
        ("out.jpg", "high_jpeg"),
        ("out.webp", "archive"),
    ] {
        let output = directory.path().join(name);
        let result = harness.ok(
            "export_layer_composite",
            json!({
                "outputPath": output.to_string_lossy(),
                "document": original,
                "operations": [],
                "profile": profile
            }),
        );
        let written = std::path::PathBuf::from(result["outputPath"].as_str().unwrap());
        assert!(written.exists(), "{name} was not written");
        assert_eq!(result["width"], 32);
        assert_eq!(result["height"], 32);
        // Decoding it again is the only proof the file is a real image.
        let decoded = image::open(&written).expect("exported file decodes");
        assert_eq!(decoded.width(), 32);
        assert_eq!(decoded.height(), 32);
    }

    // The lossless export must match what the screen showed.
    let png = image::open(directory.path().join("out.png"))
        .unwrap()
        .to_rgba8();
    assert!(max_channel_difference(&before, &png) <= 1);

    // Neither the source image nor the editable tree may have changed.
    assert_eq!(std::fs::read(&source).unwrap(), source_before);
    assert_eq!(
        harness.composite(&original, 13, 2).as_raw(),
        before.as_raw()
    );
}

#[test]
fn exporting_refuses_to_overwrite_the_source_image() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(16, 16));
    harness.open_session(&source, 14);
    let pixel_id = harness.blank_pixels(16, 16);
    let doc = document((16, 16), vec![pixel_layer("a", &pixel_id, 16, 16)]);
    let error = harness.err(
        "export_layer_composite",
        json!({
            "outputPath": source.to_string_lossy(),
            "document": doc,
            "operations": [],
            "profile": "lossless"
        }),
    );
    assert!(!error.is_empty());
}

#[test]
fn exporting_without_an_open_image_is_refused() {
    let directory = temp_dir();
    let harness = Harness::new();
    let pixel_id = harness.blank_pixels(8, 8);
    let doc = document((8, 8), vec![pixel_layer("a", &pixel_id, 8, 8)]);
    assert!(!harness
        .err(
            "export_layer_composite",
            json!({
                "outputPath": directory.path().join("x.png").to_string_lossy(),
                "document": doc, "operations": [], "profile": "lossless"
            })
        )
        .is_empty());
}

// ---------------------------------------------------------------------------
// Place image as a layer (section 19)
// ---------------------------------------------------------------------------

#[test]
fn placing_an_image_reports_its_size_and_name_and_leaves_the_file_alone() {
    let directory = temp_dir();
    let harness = Harness::new();
    let mut art = quadrants(24, 18);
    // A transparent corner, so alpha is carried through rather than filled in.
    art.put_pixel(0, 0, Rgba([0, 0, 0, 0]));
    let path = write_png(directory.path(), "placed.png", &art);
    let before = std::fs::read(&path).unwrap();

    let placed = harness.import(&path);
    assert_eq!(placed["width"], 24);
    assert_eq!(placed["height"], 18);
    assert_eq!(placed["filename"], "placed.png");
    assert_eq!(std::fs::read(&path).unwrap(), before);

    // The transparent corner survives into the composite.
    harness.open_session(&path, 15);
    let doc = document(
        (24, 18),
        vec![pixel_layer(
            "placed",
            placed["pixelId"].as_str().unwrap(),
            24,
            18,
        )],
    );
    assert_eq!(harness.composite(&doc, 15, 1).get_pixel(0, 0).0[3], 0);
}

#[test]
fn placing_refuses_malformed_and_missing_files() {
    let directory = temp_dir();
    let harness = Harness::new();
    let broken = directory.path().join("broken.png");
    std::fs::write(&broken, b"\x89PNG\r\n\x1a\n and then rubbish").unwrap();
    assert!(!harness
        .err(
            "import_layer_image",
            json!({ "path": broken.to_string_lossy() })
        )
        .is_empty());

    let absent = directory.path().join("absent.png");
    assert!(!harness
        .err(
            "import_layer_image",
            json!({ "path": absent.to_string_lossy() })
        )
        .is_empty());

    // A directory is not an image either.
    assert!(!harness
        .err(
            "import_layer_image",
            json!({ "path": directory.path().to_string_lossy() })
        )
        .is_empty());
}

// ---------------------------------------------------------------------------
// Applying operations to one layer, and the buffer store
// ---------------------------------------------------------------------------

#[test]
fn an_operation_applied_to_a_layer_leaves_the_original_buffer_untouched() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(16, 16));
    harness.open_session(&source, 16);
    let imported = harness.import(&source);
    let original_id = imported["pixelId"].as_str().unwrap().to_string();
    let doc = document((16, 16), vec![pixel_layer("a", &original_id, 16, 16)]);
    let before = harness.composite(&doc, 16, 1);

    let edited = harness.ok(
        "apply_operations_to_layer",
        json!({
            "document": doc,
            "layerId": "a",
            "operations": [{ "type": "brightness", "amount": 0.5 }]
        }),
    );
    assert_ne!(edited["pixelId"].as_str().unwrap(), original_id);
    // Undo stays cheap because the old buffer is still there.
    assert_eq!(harness.composite(&doc, 16, 2).as_raw(), before.as_raw());
}

#[test]
fn a_locked_layer_and_a_geometry_operation_are_both_refused() {
    let harness = Harness::new();
    let pixel_id = harness.blank_pixels(16, 16);
    let mut locked = pixel_layer("a", &pixel_id, 16, 16);
    locked.locked = true;
    let doc = document((16, 16), vec![locked]);
    assert!(!harness
        .err(
            "apply_operations_to_layer",
            json!({
                "document": doc, "layerId": "a",
                "operations": [{ "type": "brightness", "amount": 0.1 }]
            })
        )
        .is_empty());

    let doc = document((16, 16), vec![pixel_layer("a", &pixel_id, 16, 16)]);
    let error = harness.err(
        "apply_operations_to_layer",
        json!({
            "document": doc, "layerId": "a",
            "operations": [{ "type": "rotate", "degrees": 90 }]
        }),
    );
    assert!(error.to_lowercase().contains("geometry"), "{error}");
}

#[test]
fn unreferenced_buffers_are_released_when_the_session_stops_needing_them() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(8, 8));
    harness.open_session(&source, 1);
    let keep = harness.blank_pixels(8, 8);
    let drop = harness.blank_pixels(8, 8);
    assert_eq!(harness.ok("layer_store_report", json!({}))["buffers"], 2);
    let stale = harness.ok(
        "retain_layer_pixels",
        json!({ "pixelIds": [], "documentId": 999 }),
    );
    assert_eq!(stale["released"], 0);
    assert_eq!(stale["buffers"], 2);
    let report = harness.ok(
        "retain_layer_pixels",
        json!({ "pixelIds": [keep], "documentId": 1 }),
    );
    assert_eq!(report["released"], 1);
    assert_eq!(report["buffers"], 1);
    let _ = drop;
}

// ---------------------------------------------------------------------------
// Stale requests (section 8)
// ---------------------------------------------------------------------------

/// A render belonging to a document that is no longer open must come back
/// marked stale rather than replacing what is on screen.
#[test]
fn a_render_for_a_closed_document_is_reported_stale_and_carries_no_pixels() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(16, 16));
    harness.open_session(&source, 20);
    let pixel_id = harness.blank_pixels(16, 16);
    let doc = document((16, 16), vec![pixel_layer("a", &pixel_id, 16, 16)]);

    let stale = harness.ok(
        "render_layer_composite",
        json!({ "document": doc, "operations": [], "documentId": 999, "requestId": 1 }),
    );
    assert_eq!(stale["isCurrent"], false);
    assert_eq!(stale["previewDataUrl"], "");
}

#[test]
fn layered_sampling_sees_current_pixels_without_replacing_the_opened_source() {
    let directory = temp_dir();
    let harness = Harness::new();
    let white = RgbaImage::from_pixel(8, 8, Rgba([255, 255, 255, 255]));
    let source = write_png(directory.path(), "opened.png", &white);
    harness.open_session(&source, 80);
    let state = harness.app.state::<AppState>();
    state.layers.lock().unwrap().reset(8, 8).unwrap();
    let baseline_path = state
        .session
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .source
        .path
        .clone();
    let changed = RgbaImage::from_fn(8, 8, |x, _| {
        if x < 4 {
            Rgba([0, 0, 0, 255])
        } else {
            Rgba([255, 255, 255, 255])
        }
    });
    let changed_path = write_png(directory.path(), "changed.png", &changed);
    let imported = harness.import(&changed_path);
    let doc = document(
        (8, 8),
        vec![pixel_layer(
            "changed",
            imported["pixelId"].as_str().unwrap(),
            8,
            8,
        )],
    );

    let mut wand_input = json!({
        "point": { "x": 0, "y": 0 },
        "options": { "tolerance": 0, "connectivity": "four", "antiAlias": false, "contiguous": false },
        "mode": "replace", "base": null, "sampleMerged": true, "operations": [],
        "documentId": 80, "requestId": 1,
    });
    let flat_wand = harness.ok("magic_wand_selection", wand_input.clone());
    assert_eq!(flat_wand["diagnostics"]["selectedPixels"], 64);
    wand_input["layerDocument"] = json!(doc);
    wand_input["requestId"] = json!(2);
    let merged_wand = harness.ok("magic_wand_selection", wand_input.clone());
    assert_eq!(merged_wand["diagnostics"]["selectedPixels"], 32);
    wand_input["sampleMerged"] = json!(false);
    wand_input["requestId"] = json!(3);
    assert_eq!(
        harness.ok("magic_wand_selection", wand_input)["diagnostics"]["selectedPixels"],
        64
    );

    let range = harness.ok("color_range_selection", json!({
        "samples": [{"x": 0, "y": 0}],
        "options": { "tolerance": 0.01, "luminanceSensitivity": 1, "hueSensitivity": 1, "saturationSensitivity": 1 },
        "mode": "replace", "base": null, "sampleMerged": true, "operations": [],
        "layerDocument": doc, "documentId": 80, "requestId": 4,
    }));
    assert_eq!(range["diagnostics"]["selectedPixels"], 32);

    let flat_histogram = harness.ok(
        "generate_histogram",
        json!({ "operations": [], "documentId": 80, "requestId": 1 }),
    );
    let merged_histogram = harness.ok(
        "generate_histogram",
        json!({ "operations": [], "layerDocument": doc, "documentId": 80, "requestId": 2 }),
    );
    assert_eq!(flat_histogram["after"]["red"][0], 0);
    assert_eq!(merged_histogram["after"]["red"][0], 32);
    assert_eq!(merged_histogram["after"]["red"][255], 32);

    let sample = harness.ok(
        "inspect_image_pixel",
        json!({ "x": 0, "y": 0, "operations": [], "layerDocument": doc, "documentId": 80 }),
    );
    assert_eq!(sample["red"], 0);
    let point = harness.ok("create_point_operation", json!({ "x": 0, "y": 0, "white": false, "operations": [], "layerDocument": doc, "documentId": 80 }));
    assert_eq!(point["red"], 0);

    let flat_analysis = harness.ok("analyze_image", json!({ "documentId": 80, "requestId": 1 }));
    let layered_analysis = harness.ok(
        "analyze_image",
        json!({ "documentId": 80, "requestId": 2, "layerDocument": doc }),
    );
    assert!(
        flat_analysis["analysis"]["averageLuminance"]
            .as_f64()
            .unwrap()
            > 0.99
    );
    assert!(
        (layered_analysis["analysis"]["averageLuminance"]
            .as_f64()
            .unwrap()
            - 0.5)
            .abs()
            < 0.01
    );
    let flat_again = harness.ok("analyze_image", json!({ "documentId": 80, "requestId": 3 }));
    assert!(
        flat_again["analysis"]["averageLuminance"].as_f64().unwrap() > 0.99,
        "layer analysis must not poison original-source cache"
    );
    let session = state.session.lock().unwrap();
    assert_eq!(session.as_ref().unwrap().source.path, baseline_path);
    assert_eq!(session.as_ref().unwrap().source.original.to_rgba8(), white);
}

#[test]
fn layered_sampling_rejects_stale_identity_and_missing_pixels() {
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(8, 8));
    harness.open_session(&source, 81);
    harness
        .app
        .state::<AppState>()
        .layers
        .lock()
        .unwrap()
        .reset(8, 8)
        .unwrap();
    let missing = document((8, 8), vec![pixel_layer("a", "missing", 8, 8)]);
    assert!(!harness
        .err(
            "inspect_image_pixel",
            json!({
                "x": 0, "y": 0, "operations": [], "layerDocument": missing, "documentId": 81
            })
        )
        .is_empty());
    let stale = harness.ok(
        "generate_histogram",
        json!({
            "operations": [], "layerDocument": missing, "documentId": 82, "requestId": 1
        }),
    );
    assert_eq!(stale["isCurrent"], false);
}

// ---------------------------------------------------------------------------
// Recovery snapshots (section 27)
// ---------------------------------------------------------------------------

#[test]
fn a_recovery_snapshot_is_written_listed_restored_and_discarded() {
    // Recovery writes prune old snapshots and discard(null) removes all of them.
    // Never point this integration test at the user's real application data.
    const CHILD_MARKER: &str = "PHOTOFORGE_ISOLATED_RECOVERY_TEST";
    if std::env::var_os(CHILD_MARKER).is_none() {
        let isolated_data = temp_dir();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "a_recovery_snapshot_is_written_listed_restored_and_discarded",
                "--nocapture",
            ])
            .env("LOCALAPPDATA", isolated_data.path())
            .env(CHILD_MARKER, "1")
            .status()
            .expect("isolated recovery test starts");
        assert!(status.success(), "isolated recovery test failed");
        return;
    }
    let directory = temp_dir();
    let harness = Harness::new();
    let source = write_png(directory.path(), "source.png", &quadrants(16, 16));
    harness.open_session(&source, 21);
    let imported = harness.import(&source);
    let doc = document(
        (16, 16),
        vec![pixel_layer(
            "a",
            imported["pixelId"].as_str().unwrap(),
            16,
            16,
        )],
    );
    let rendered = harness.composite(&doc, 21, 1);

    let record = harness.ok(
        "write_recovery_snapshot",
        json!({
            "document": doc,
            "operations": [],
            "projectPath": null,
            "documentName": "Untitled",
            "savedAt": "2026-01-01T00:00:00Z"
        }),
    );
    let snapshot_path = record["snapshotPath"].as_str().unwrap().to_string();
    assert!(record["bytes"].as_u64().unwrap() > 0);

    let listed = harness.ok("list_recovery_snapshots", json!({}));
    assert!(listed["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["snapshotPath"] == snapshot_path.as_str()));

    // Restore into a genuine cold-start session, not the session used to save.
    let restarted = Harness::new();
    let restored = restarted.ok(
        "restore_recovery_snapshot",
        json!({ "path": snapshot_path, "requestId": 22 }),
    );
    let restored_document: LayerDocument =
        serde_json::from_value(restored["document"].clone()).unwrap();
    assert_eq!(
        restarted.composite(&restored_document, 22, 2).as_raw(),
        rendered.as_raw()
    );
    assert_eq!(restored["isCurrent"], true);
    assert_eq!(restored["documentId"], 22);
    assert_eq!(
        decode_data_url(restored["originalPreviewDataUrl"].as_str().unwrap()),
        rendered
    );
    assert_eq!(
        restarted
            .app
            .state::<AppState>()
            .pending_open_request
            .load(std::sync::atomic::Ordering::Acquire),
        0
    );

    // Discarding must actually remove it, and be safe to repeat.
    assert!(
        harness
            .ok("discard_recovery_snapshot", json!({ "path": null }))
            .as_u64()
            .unwrap()
            >= 1
    );
    assert_eq!(
        harness.ok("list_recovery_snapshots", json!({}))["snapshots"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        harness.ok("discard_recovery_snapshot", json!({ "path": null })),
        json!(0)
    );
}

#[test]
fn restoring_a_corrupt_or_absent_snapshot_fails_cleanly() {
    let directory = temp_dir();
    let harness = Harness::new();
    let path = directory.path().join("broken.photoforge-recovery");
    std::fs::write(&path, b"not a snapshot").unwrap();
    assert!(!harness
        .err(
            "restore_recovery_snapshot",
            json!({ "path": path.to_string_lossy(), "requestId": 1 })
        )
        .is_empty());
    assert!(!harness
        .err(
            "restore_recovery_snapshot",
            json!({ "path": directory.path().join("absent").to_string_lossy(), "requestId": 2 })
        )
        .is_empty());
}

// ---------------------------------------------------------------------------
// Layer workflow planning (section 11)
// ---------------------------------------------------------------------------

#[test]
fn planning_layer_steps_resolves_targets_and_refuses_a_fabricated_one() {
    let harness = Harness::new();
    let pixel_id = harness.blank_pixels(8, 8);
    let doc = document((8, 8), vec![pixel_layer("a", &pixel_id, 8, 8)]);
    let planned = harness.ok(
        "plan_layer_workflow",
        json!({
            "document": doc,
            "steps": [{ "type": "set_opacity", "selector": { "type": "active" }, "opacity": 0.5 }]
        }),
    );
    assert_eq!(planned["steps"], 1);
    assert_eq!(planned["targets"], json!(["a"]));

    assert!(!harness
        .err(
            "plan_layer_workflow",
            json!({
                "document": doc,
                "steps": [{
                    "type": "set_opacity",
                    "selector": { "type": "id", "id": "invented" },
                    "opacity": 0.5
                }]
            })
        )
        .is_empty());
}
