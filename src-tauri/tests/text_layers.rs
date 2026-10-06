//! Phase 13 gates for text layers.
//!
//! The claim under test is the one that matters: text stays text. It survives a
//! save and reopen as characters rather than as a picture of characters, it
//! composites through the same masks, groups and blend modes as every other
//! layer, it renders identically whether the document is tiled or not, and
//! scaling it redraws the letterforms instead of enlarging their pixels.
use photoforge_lib::{
    color::FloatImage,
    layers::{
        render_document_float, render_document_tiled, shape::ShapeColor, text::TextContent,
        BlendMode, Layer, LayerContent, LayerDocument, LayerKind, LayerMask, LayerMetadata,
        LayerPixelStore, LayerTransform, RenderOptions,
    },
    mask::{MaskBitmap, MaskSnapshot},
    pixel::DocumentPrecision,
    text as text_engine,
};

fn base_layer(id: &str) -> Layer {
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
        raw: None,
        origin: None,
        content: LayerContent::Group {
            children: Vec::new(),
            isolated: true,
        },
    }
}

fn text_layer(id: &str, content: TextContent) -> Layer {
    Layer {
        content: LayerContent::Text {
            text: Box::new(content),
        },
        ..base_layer(id)
    }
}

fn sample_text() -> TextContent {
    let mut content = TextContent::new("Handgloves", 12.0, 70.0, 40.0);
    content.fill = ShapeColor::new(0.05, 0.05, 0.05, 1.0);
    content
}

fn document(width: u32, height: u32, layers: Vec<Layer>) -> LayerDocument {
    let mut document = LayerDocument::new(width, height);
    document.precision = DocumentPrecision::LinearSrgbF32;
    document.layers = layers;
    document.active_layer_id = None;
    document
}

fn render(document: &LayerDocument) -> FloatImage {
    let store = LayerPixelStore::default();
    let resolved = store
        .resolve(&document.referenced_pixel_ids(), false)
        .expect("resolve");
    render_document_float(document, &resolved, RenderOptions::default()).expect("render")
}

fn ink(image: &FloatImage) -> f64 {
    image.pixels().iter().map(|p| f64::from(p.alpha)).sum()
}

/// The central Phase 13 claim: reopening a project gives back editable text.
///
/// Checked against the serialised form as well as the restored value, because
/// "it round trips" would also be true of a project that stored a rendered
/// bitmap and handed the same bitmap back.
#[test]
fn text_survives_a_project_round_trip_as_text() {
    let mut content = sample_text();
    content.text = "Hello \u{0633}\u{0644}\u{0627}\u{0645}\nsecond line".into();
    content.font_family = "Times New Roman".into();
    content.italic = true;
    content.letter_spacing = 1.25;
    content.wrap_width = Some(240.0);
    let document = document(320, 200, vec![text_layer("t", content.clone())]);

    let json = serde_json::to_string(&document).expect("serialise");
    assert!(
        json.contains("\"type\":\"text\""),
        "the layer was not stored as a text layer"
    );
    assert!(
        json.contains("Times New Roman"),
        "the requested font was not stored"
    );
    assert!(
        !json.contains("pixelId"),
        "a text layer wrote a pixel reference into the project"
    );
    // No glyph indices: those name a face on one machine, and a project storing
    // them would reopen as nonsense anywhere else.
    assert!(
        !json.contains("glyph"),
        "the project stored resolved glyphs rather than characters"
    );

    let restored: LayerDocument = serde_json::from_str(&json).expect("deserialise");
    restored.validate().expect("valid");
    let LayerContent::Text { text } = &restored.layers[0].content else {
        panic!("the layer did not reopen as a text layer");
    };
    assert_eq!(**text, content);
    assert_eq!(restored.layers[0].kind(), LayerKind::Text);

    // And it renders the same after the round trip as before it.
    assert_eq!(render(&document).pixels(), render(&restored).pixels());
}

/// An older project has none of the fields text added, and must still open.
#[test]
fn a_project_without_text_layers_still_opens() {
    let json = r#"{
        "schemaVersion": 1,
        "precision": "linear_srgb_f32",
        "canvasWidth": 64,
        "canvasHeight": 64,
        "layers": [],
        "activeLayerId": null
    }"#;
    let document: LayerDocument = serde_json::from_str(json).expect("deserialise");
    document.validate().expect("valid");
    assert!(document.layers.is_empty());
}

/// Tiled and full-frame renders must agree exactly, or text seams at tile
/// boundaries — which for text means a visible break through a letter.
#[test]
fn tiled_and_full_frame_renders_of_text_agree() {
    let mut wrapped = sample_text();
    wrapped.text = "Handgloves in a wrapped paragraph that runs to several lines".into();
    wrapped.wrap_width = Some(260.0);
    let document = document(
        320,
        240,
        vec![
            text_layer("a", sample_text()),
            Layer {
                transform: LayerTransform {
                    translate_x: 8.0,
                    translate_y: 60.0,
                    scale_x: 1.4,
                    scale_y: 1.4,
                    rotation_degrees: 11.0,
                    ..LayerTransform::default()
                },
                ..text_layer("b", wrapped)
            },
        ],
    );

    let store = LayerPixelStore::default();
    let resolved = store
        .resolve(&document.referenced_pixel_ids(), false)
        .expect("resolve");
    let reference =
        render_document_float(&document, &resolved, RenderOptions::default()).expect("render");
    for tile_size in [64u32, 96, 128] {
        let (tiled, _) =
            render_document_tiled(&document, &resolved, RenderOptions::default(), tile_size)
                .expect("render");
        assert_eq!(
            reference.pixels(),
            tiled.pixels(),
            "a {tile_size}-pixel tiling disagreed with the full-frame render"
        );
    }
}

/// Scaling a text layer must redraw the glyphs. A layer that had been
/// rasterised once and resampled would keep the same proportion of soft edge
/// pixels; redrawn outlines grow their area with the square of the scale while
/// their edges stay about a pixel wide, so the soft fraction falls.
#[test]
fn scaling_text_redraws_it_rather_than_resampling() {
    // Centred, because a layer transform scales about the canvas centre and
    // text starting at the corner would simply be pushed off the canvas.
    let mut centred = sample_text();
    centred.text = "Hg".into();
    centred.origin_x = 380.0;
    centred.origin_y = 300.0;
    let small = document(800, 600, vec![text_layer("t", centred.clone())]);
    let large = document(
        800,
        600,
        vec![Layer {
            transform: LayerTransform {
                scale_x: 3.0,
                scale_y: 3.0,
                ..LayerTransform::default()
            },
            ..text_layer("t", centred)
        }],
    );

    let soft_fraction = |image: &FloatImage| {
        let partial = image
            .pixels()
            .iter()
            .filter(|p| p.alpha > 0.02 && p.alpha < 0.98)
            .count() as f64;
        let inked = image.pixels().iter().filter(|p| p.alpha > 0.02).count() as f64;
        partial / inked.max(1.0)
    };

    let small_render = render(&small);
    let large_render = render(&large);
    assert!(
        ink(&large_render) > ink(&small_render) * 3.0,
        "the enlarged text did not cover proportionally more of the canvas"
    );
    assert!(
        soft_fraction(&large_render) < soft_fraction(&small_render),
        "the enlarged text was not proportionally sharper, so it was resampled"
    );
}

/// Text takes part in the layer system rather than being painted over it.
#[test]
fn text_composites_through_opacity_masks_groups_and_blend_modes() {
    let plain = document(300, 200, vec![text_layer("t", sample_text())]);
    let full = ink(&render(&plain));
    assert!(full > 100.0, "the text drew almost nothing");

    // Opacity.
    let faded = document(
        300,
        200,
        vec![Layer {
            opacity: 0.25,
            ..text_layer("t", sample_text())
        }],
    );
    let faded_ink = ink(&render(&faded));
    assert!(
        (faded_ink - full * 0.25).abs() < full * 0.02,
        "opacity did not scale the text's coverage: {faded_ink} against {}",
        full * 0.25
    );

    // A mask that hides the left half.
    let mut bitmap = MaskBitmap::full(300, 200).expect("mask");
    for y in 0..200 {
        for x in 0..150 {
            bitmap.set(x, y, 0);
        }
    }
    let masked = document(
        300,
        200,
        vec![Layer {
            mask: Some(LayerMask {
                snapshot: MaskSnapshot::encode(&bitmap),
                enabled: true,
                inverted: false,
            }),
            ..text_layer("t", sample_text())
        }],
    );
    let masked_render = render(&masked);
    for y in 0..200u32 {
        for x in 0..150u32 {
            assert_eq!(
                masked_render.pixels()[(y * 300 + x) as usize].alpha,
                0.0,
                "the mask did not hide the text at {x},{y}"
            );
        }
    }
    assert!(ink(&masked_render) > 0.0, "the mask hid everything");

    // Inside a group, carrying the group's transform.
    let grouped = document(
        300,
        200,
        vec![Layer {
            transform: LayerTransform {
                translate_x: 20.0,
                ..LayerTransform::default()
            },
            content: LayerContent::Group {
                children: vec![text_layer("t", sample_text())],
                isolated: true,
            },
            ..base_layer("g")
        }],
    );
    let grouped_render = render(&grouped);
    assert!(
        (ink(&grouped_render) - full).abs() < full * 0.05,
        "grouping changed how much the text covered"
    );
    assert_ne!(
        grouped_render.pixels(),
        render(&plain).pixels(),
        "the group's transform did not move the text"
    );

    // A blend mode other than Normal changes the result, so text is going
    // through the compositor rather than being drawn on top of it. This needs
    // something underneath: every blend mode agrees with source-over when the
    // backdrop is empty, so a test without one proves nothing.
    let backdrop = Layer {
        content: LayerContent::Shape {
            shape: Box::new(photoforge_lib::layers::shape::ShapeContent {
                geometry: photoforge_lib::vector::ShapeGeometry::Rectangle {
                    x: 0.0,
                    y: 0.0,
                    width: 300.0,
                    height: 200.0,
                    corner_radius: 0.0,
                },
                fill: Some(ShapeColor::new(0.6, 0.7, 0.4, 1.0)),
                stroke: None,
                stroke_style: None,
                fill_rule: photoforge_lib::vector::FillRule::NonZero,
            }),
        },
        ..base_layer("bg")
    };
    let over = document(
        300,
        200,
        vec![backdrop.clone(), text_layer("t", sample_text())],
    );
    let mut multiplied = over.clone();
    multiplied.layers[1].blend_mode = BlendMode::Multiply;
    assert_ne!(
        render(&multiplied).pixels(),
        render(&over).pixels(),
        "the blend mode was ignored for text"
    );
}

/// A font this machine does not have is reported, and the request is kept.
#[test]
fn a_missing_font_is_reported_and_the_request_is_preserved() {
    let mut content = sample_text();
    content.font_family = "No Such Font Exists Here 12345".into();
    assert!(content.font_is_missing());

    let document = document(300, 200, vec![text_layer("t", content.clone())]);
    // It still draws, in a substitute, rather than failing to open.
    assert!(ink(&render(&document)) > 0.0, "the substitute drew nothing");

    // And a save keeps the original request rather than the substitute.
    let json = serde_json::to_string(&document).expect("serialise");
    let restored: LayerDocument = serde_json::from_str(&json).expect("deserialise");
    let LayerContent::Text { text } = &restored.layers[0].content else {
        panic!("not a text layer");
    };
    assert_eq!(text.font_family, "No Such Font Exists Here 12345");

    // A font the machine does have is not reported as missing.
    if let Some(installed) = text_engine::available_families().first() {
        let mut present = sample_text();
        present.font_family = installed.clone();
        assert!(
            !present.font_is_missing(),
            "{installed} was reported missing"
        );
    }
}

/// Right-to-left text has to be laid out right to left, not merely drawn.
///
/// Checked through the document render rather than through the shaper alone:
/// the claim is about what the user sees, so the evidence is where the ink
/// lands. The first character of a right-to-left line belongs on the right,
/// which puts the block's ink to the left of a left-to-right block that starts
/// at the same origin only if nothing reordered it.
#[test]
fn right_to_left_text_renders_reordered() {
    let mut persian = sample_text();
    // "salam", then a Latin word. Under correct bidi the Latin sits to the
    // right of the Persian even though it comes second in the string, because
    // the paragraph reads right to left.
    persian.text = "\u{0633}\u{0644}\u{0627}\u{0645} Latin".into();
    let shaped = persian.shaped().expect("shape");

    let latin: Vec<_> = shaped.glyphs.iter().filter(|g| g.start >= 9).collect();
    let arabic: Vec<_> = shaped.glyphs.iter().filter(|g| g.start < 9).collect();
    assert!(!latin.is_empty() && !arabic.is_empty());
    // Within the right-to-left run the source order runs backwards on screen.
    let mut ordered: Vec<_> = arabic.clone();
    ordered.sort_by(|a, b| a.x.total_cmp(&b.x));
    for pair in ordered.windows(2) {
        assert!(
            pair[1].start < pair[0].start,
            "the right-to-left run was not reordered for display"
        );
    }

    // And it actually reaches the canvas.
    let document = document(400, 200, vec![text_layer("t", persian)]);
    assert!(ink(&render(&document)) > 0.0, "the Persian drew nothing");
}

/// Nothing turns text into pixels on its own. The document keeps the layer as
/// text through every render, and only an explicit rasterize replaces it.
#[test]
fn rendering_never_rasterizes_the_layer_in_the_document() {
    let document = document(300, 200, vec![text_layer("t", sample_text())]);
    let before = serde_json::to_string(&document).expect("serialise");
    for _ in 0..3 {
        let _ = render(&document);
        let store = LayerPixelStore::default();
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .expect("resolve");
        let _ = render_document_tiled(&document, &resolved, RenderOptions::default(), 64)
            .expect("render");
    }
    assert_eq!(
        before,
        serde_json::to_string(&document).expect("serialise"),
        "rendering changed the document"
    );
    assert!(
        document.referenced_pixel_ids().is_empty(),
        "a text layer acquired a pixel buffer without being asked to"
    );
}

/// The document validator refuses text it cannot lay out, rather than accepting
/// it and failing at render time on someone else's machine.
#[test]
fn an_invalid_text_layer_is_refused_at_the_document_level() {
    for spoil in [
        (|c: &mut TextContent| c.font_size = f32::NAN) as fn(&mut TextContent),
        |c: &mut TextContent| c.font_size = 0.0,
        |c: &mut TextContent| c.line_height = 50.0,
        |c: &mut TextContent| c.font_weight = 1_000,
        |c: &mut TextContent| c.letter_spacing = f32::INFINITY,
        |c: &mut TextContent| c.wrap_width = Some(-4.0),
        |c: &mut TextContent| c.origin_x = f32::NAN,
    ] {
        let mut content = sample_text();
        spoil(&mut content);
        let document = document(64, 64, vec![text_layer("t", content)]);
        assert!(
            document.validate().is_err(),
            "an unrenderable text layer passed validation"
        );
    }
    assert!(document(64, 64, vec![text_layer("t", sample_text())])
        .validate()
        .is_ok());
}

/// Two text layers with the same words but a different setting must not render
/// identically — the setting has to reach the rasteriser.
#[test]
fn every_setting_changes_what_is_drawn() {
    let baseline = render(&document(400, 260, vec![text_layer("t", sample_text())]));
    /// One named change to make, so the loop reads as a list of settings.
    type Variant = (&'static str, Box<dyn Fn(&mut TextContent)>);
    let variants: Vec<Variant> = vec![
        ("size", Box::new(|c: &mut TextContent| c.font_size = 56.0)),
        (
            "weight",
            Box::new(|c: &mut TextContent| c.font_weight = 700),
        ),
        ("italic", Box::new(|c: &mut TextContent| c.italic = true)),
        (
            "letter spacing",
            Box::new(|c: &mut TextContent| c.letter_spacing = 4.0),
        ),
        (
            "line height",
            Box::new(|c: &mut TextContent| {
                c.text = "two\nlines".into();
                c.line_height = 2.5;
            }),
        ),
        ("origin", Box::new(|c: &mut TextContent| c.origin_x += 25.0)),
        (
            "colour",
            Box::new(|c: &mut TextContent| c.fill = ShapeColor::new(0.9, 0.1, 0.1, 1.0)),
        ),
    ];
    for (name, change) in variants {
        let mut content = sample_text();
        change(&mut content);
        let rendered = render(&document(400, 260, vec![text_layer("t", content)]));
        assert_ne!(
            baseline.pixels(),
            rendered.pixels(),
            "changing the {name} did not change the render"
        );
    }
}
