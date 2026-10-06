//! Phase 13 gates for vector shape layers.
//!
//! The claim under test is that a shape is a first-class document object: it
//! composites through the same masks, groups and blend modes as everything
//! else, it renders identically whether the document is tiled or not, and it
//! survives a transform without being resampled.
use photoforge_lib::{
    color::{FloatImage, FloatRgba},
    layers::{
        render_document_float, render_document_tiled, shape::ShapeColor, shape::ShapeContent,
        BlendMode, Layer, LayerContent, LayerDocument, LayerMask, LayerMetadata, LayerPixelStore,
        LayerTransform, RenderOptions,
    },
    mask::{MaskBitmap, MaskSnapshot},
    pixel::DocumentPrecision,
    vector::{FillRule, ShapeGeometry, StrokeStyle},
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

fn shape_layer(id: &str, content: ShapeContent) -> Layer {
    Layer {
        content: LayerContent::Shape {
            shape: Box::new(content),
        },
        ..base_layer(id)
    }
}

fn filled(geometry: ShapeGeometry, colour: ShapeColor) -> ShapeContent {
    ShapeContent {
        geometry,
        fill: Some(colour),
        stroke: None,
        stroke_style: None,
        fill_rule: FillRule::NonZero,
    }
}

fn document(width: u32, height: u32, layers: Vec<Layer>) -> LayerDocument {
    let mut document = LayerDocument::new(width, height);
    document.precision = DocumentPrecision::LinearSrgbF32;
    document.layers = layers;
    document
}

fn render_both(document: &LayerDocument, tile: u32) -> (FloatImage, FloatImage) {
    let store = LayerPixelStore::default();
    let resolved = store
        .resolve(&document.referenced_pixel_ids(), false)
        .expect("resolve");
    let options = RenderOptions {
        scale: 1.0,
        cancel: None,
    };
    let whole = render_document_float(document, &resolved, options).expect("full frame");
    let (tiled, stats) = render_document_tiled(document, &resolved, options, tile).expect("tiled");
    assert!(
        !stats.fell_back_to_full_frame,
        "a shape document was rendered whole instead of tiled"
    );
    (whole, tiled)
}

/// The central integration claim: tiling a shape changes nothing at all.
///
/// Exactly equal, not merely close. The rasteriser's accumulator is
/// region-independent by construction, so any difference here would be a bug in
/// how the renderer passes regions rather than rounding.
#[test]
fn a_shape_renders_identically_whether_tiled_or_not() {
    let shapes = vec![
        shape_layer(
            "circle",
            filled(
                ShapeGeometry::Ellipse {
                    cx: 90.0,
                    cy: 70.0,
                    rx: 60.0,
                    ry: 40.0,
                },
                ShapeColor::new(0.9, 0.2, 0.1, 1.0),
            ),
        ),
        shape_layer(
            "star",
            ShapeContent {
                geometry: ShapeGeometry::Star {
                    cx: 120.0,
                    cy: 120.0,
                    outer_radius: 70.0,
                    inner_radius: 26.0,
                    points: 7,
                    rotation_degrees: 13.0,
                },
                fill: Some(ShapeColor::new(0.1, 0.4, 0.9, 0.8)),
                stroke: Some(ShapeColor::new(1.0, 1.0, 0.2, 1.0)),
                stroke_style: Some(StrokeStyle {
                    width: 5.0,
                    ..StrokeStyle::default()
                }),
                fill_rule: FillRule::NonZero,
            },
        ),
        shape_layer(
            "rounded",
            filled(
                ShapeGeometry::Rectangle {
                    x: 20.5,
                    y: 30.25,
                    width: 150.0,
                    height: 90.0,
                    corner_radius: 22.0,
                },
                ShapeColor::new(0.2, 0.8, 0.3, 0.6),
            ),
        ),
    ];
    let document = document(224, 192, shapes);
    for tile in [64u32, 96, 128] {
        let (whole, tiled) = render_both(&document, tile);
        assert_eq!(
            whole.pixels(),
            tiled.pixels(),
            "tile size {tile} changed a shape render"
        );
    }
}

/// A shape has to obey the layer properties every other layer obeys.
#[test]
fn a_shape_participates_in_opacity_masks_and_blend_modes() {
    let geometry = ShapeGeometry::Rectangle {
        x: 20.0,
        y: 20.0,
        width: 80.0,
        height: 80.0,
        corner_radius: 0.0,
    };
    let opaque = document(
        128,
        128,
        vec![shape_layer(
            "square",
            filled(geometry.clone(), ShapeColor::new(1.0, 0.0, 0.0, 1.0)),
        )],
    );
    let (whole, _) = render_both(&opaque, 64);
    assert!((whole.get(60, 60).unwrap().alpha - 1.0).abs() < 1e-6);

    // Opacity.
    let mut half = opaque.clone();
    half.layers[0].opacity = 0.5;
    let (whole, tiled) = render_both(&half, 64);
    assert_eq!(whole.pixels(), tiled.pixels());
    assert!(
        (whole.get(60, 60).unwrap().alpha - 0.5).abs() < 1e-5,
        "opacity was ignored: {:?}",
        whole.get(60, 60)
    );

    // Mask: covering the left half only.
    let mut masked = opaque.clone();
    let mut bitmap = MaskBitmap::full(128, 128).unwrap();
    for y in 0..128u32 {
        for x in 64..128u32 {
            bitmap.set(x, y, 0);
        }
    }
    masked.layers[0].mask = Some(LayerMask {
        snapshot: MaskSnapshot::encode(&bitmap),
        enabled: true,
        inverted: false,
    });
    let (whole, tiled) = render_both(&masked, 64);
    assert_eq!(whole.pixels(), tiled.pixels());
    assert!(
        whole.get(40, 60).unwrap().alpha > 0.99,
        "the masked-in half did not paint"
    );
    assert!(
        whole.get(80, 60).unwrap().alpha < 0.01,
        "the masked-out half painted anyway"
    );

    // Blend mode over a backdrop.
    let backdrop = shape_layer(
        "backdrop",
        filled(
            ShapeGeometry::Rectangle {
                x: 0.0,
                y: 0.0,
                width: 128.0,
                height: 128.0,
                corner_radius: 0.0,
            },
            ShapeColor::new(0.5, 0.5, 0.5, 1.0),
        ),
    );
    let mut top = shape_layer(
        "multiply",
        filled(geometry, ShapeColor::new(0.5, 0.5, 0.5, 1.0)),
    );
    top.blend_mode = BlendMode::Multiply;
    let blended = document(128, 128, vec![backdrop, top]);
    let (whole, tiled) = render_both(&blended, 64);
    assert_eq!(whole.pixels(), tiled.pixels());
    let inside = whole.get(60, 60).unwrap();
    assert!(
        (inside.red - 0.25).abs() < 1e-4,
        "multiply gave {inside:?}, expected 0.25"
    );
}

/// Shapes inside groups, isolated and pass-through, nested.
#[test]
fn a_shape_composites_correctly_inside_groups() {
    let inner = shape_layer(
        "inner",
        filled(
            ShapeGeometry::Ellipse {
                cx: 64.0,
                cy: 64.0,
                rx: 40.0,
                ry: 40.0,
            },
            ShapeColor::new(0.2, 0.7, 0.9, 1.0),
        ),
    );
    for isolated in [true, false] {
        let mut group = base_layer("group");
        group.content = LayerContent::Group {
            children: vec![inner.clone()],
            isolated,
        };
        group.opacity = 0.75;
        if !isolated {
            group.blend_mode = BlendMode::Normal;
        }
        let mut nested = base_layer("outer");
        nested.content = LayerContent::Group {
            children: vec![group],
            isolated: true,
        };
        let document = document(128, 128, vec![nested]);
        let (whole, tiled) = render_both(&document, 64);
        assert_eq!(
            whole.pixels(),
            tiled.pixels(),
            "a shape in an isolated={isolated} group differed when tiled"
        );
        assert!(
            whole.get(64, 64).unwrap().alpha > 0.7,
            "the grouped shape did not paint"
        );
    }
}

/// The point of a vector layer: repeated transforms sample the geometry, not a
/// rendered raster, so nothing degrades.
#[test]
fn repeated_transforms_do_not_degrade_a_shape() {
    // A circle, so there is always a genuinely antialiased edge to measure. An
    // axis-aligned rectangle on integer coordinates has none at all, which
    // makes it useless for telling a sharp edge from a smeared one.
    let content = filled(
        ShapeGeometry::Ellipse {
            cx: 64.0,
            cy: 64.0,
            rx: 24.0,
            ry: 24.0,
        },
        ShapeColor::new(1.0, 1.0, 1.0, 1.0),
    );
    // The fraction of painted pixels that are partly covered. For geometry
    // re-rasterised at its final size this tracks perimeter over area, so it
    // *falls* as the shape grows. For a resampled raster it stays put or rises,
    // because the softness is baked into the pixels being magnified.
    let soft_fraction = |transform: LayerTransform| {
        let mut layer = shape_layer("circle", content.clone());
        layer.transform = transform;
        let document = document(256, 256, vec![layer]);
        let (whole, _) = render_both(&document, 64);
        let painted = whole.pixels().iter().filter(|p| p.alpha > 0.01).count();
        let soft = whole
            .pixels()
            .iter()
            .filter(|p| p.alpha > 0.01 && p.alpha < 0.99)
            .count();
        assert!(painted > 100, "the shape barely painted: {painted} pixels");
        soft as f64 / painted as f64
    };

    let untouched = soft_fraction(LayerTransform::default());
    assert!(
        untouched > 0.0,
        "a circle produced no antialiased edge at all"
    );

    // A full rotation returns the shape to where it started, and must return
    // the edge with it.
    let rotated = soft_fraction(LayerTransform {
        rotation_degrees: 360.0,
        ..LayerTransform::default()
    });
    assert!(
        (rotated - untouched).abs() < untouched * 0.5,
        "a full rotation changed the soft-edge fraction from {untouched} to {rotated}"
    );

    // Scaled up, the shape is drawn again at the new size: four times the area
    // but only twice the perimeter, so the soft fraction must roughly halve. A
    // magnified raster would keep it, or worse.
    let enlarged = soft_fraction(LayerTransform {
        scale_x: 2.0,
        scale_y: 2.0,
        ..LayerTransform::default()
    });
    assert!(
        enlarged < untouched * 0.75,
        "scaling 2x left the soft-edge fraction at {enlarged} against {untouched},          which is what resampling rather than re-rasterising looks like"
    );
}

/// Shape layers must survive a project round trip as shapes.
#[test]
fn a_shape_survives_serialisation_as_a_shape() {
    let content = ShapeContent {
        geometry: ShapeGeometry::Star {
            cx: 60.0,
            cy: 60.0,
            outer_radius: 40.0,
            inner_radius: 18.0,
            points: 6,
            rotation_degrees: 22.5,
        },
        fill: Some(ShapeColor::new(0.3, 0.6, 0.9, 0.85)),
        stroke: Some(ShapeColor::new(0.0, 0.0, 0.0, 1.0)),
        stroke_style: Some(StrokeStyle {
            width: 3.5,
            ..StrokeStyle::default()
        }),
        fill_rule: FillRule::EvenOdd,
    };
    let layer = shape_layer("star", content.clone());
    let encoded = serde_json::to_string(&layer).expect("encode");
    let decoded: Layer = serde_json::from_str(&encoded).expect("decode");
    assert_eq!(layer, decoded);
    match decoded.content {
        LayerContent::Shape { shape } => {
            assert_eq!(*shape, content, "the shape changed across a round trip");
            // Still parametric, not a bag of points.
            assert!(matches!(
                shape.geometry,
                ShapeGeometry::Star { points: 6, .. }
            ));
        }
        other => panic!("a shape layer decoded as {other:?}"),
    }
}

/// A document holding a hostile shape must be refused, not rendered.
#[test]
fn a_document_with_hostile_geometry_is_refused() {
    let layer = shape_layer(
        "bad",
        filled(
            ShapeGeometry::Rectangle {
                x: f32::NAN,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                corner_radius: 0.0,
            },
            ShapeColor::new(1.0, 0.0, 0.0, 1.0),
        ),
    );
    let document = document(64, 64, vec![layer]);
    assert!(
        document.validate().is_err(),
        "a document with a NaN shape validated"
    );
}

/// An empty shape is a legitimate state and must render as nothing rather than
/// failing.
#[test]
fn a_shape_with_no_paint_renders_as_nothing() {
    let content = ShapeContent {
        geometry: ShapeGeometry::Rectangle {
            x: 10.0,
            y: 10.0,
            width: 40.0,
            height: 40.0,
            corner_radius: 0.0,
        },
        fill: None,
        stroke: None,
        stroke_style: None,
        fill_rule: FillRule::NonZero,
    };
    let document = document(64, 64, vec![shape_layer("empty", content)]);
    let (whole, tiled) = render_both(&document, 64);
    assert_eq!(whole.pixels(), tiled.pixels());
    assert!(
        whole.pixels().iter().all(|p| p.alpha == 0.0),
        "an unpainted shape wrote pixels"
    );
    assert!(whole.pixels().iter().all(|p: &FloatRgba| p.is_finite()));
}
