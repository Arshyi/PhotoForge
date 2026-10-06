//! Phase 11 end-to-end gates: the tiled and streaming paths must be an
//! allocation strategy, never a second renderer with its own answers.
//!
//! The unit tests in `layers::tiled` compare renders in memory. These compare
//! whole exported files, because that is what a user actually receives, and a
//! difference in dither phase or row ordering would survive an in-memory check
//! of the composite alone.
use photoforge_lib::{
    color::{FloatImage, FloatRgba},
    color_management::{ColorExportOptions, RgbColorSpace},
    domain::{EditOperation, ExportProfile},
    infrastructure::{save_color_image, save_color_image_streaming},
    layers::*,
    pixel::DocumentPrecision,
};

fn layer(id: &str, pixel: &str, w: u32, h: u32) -> Layer {
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
        content: LayerContent::Pixel {
            pixel_id: pixel.into(),
            width: w,
            height: h,
        },
    }
}

fn source(w: u32, h: u32, seed: u32) -> FloatImage {
    let mut image = FloatImage::blank(w, h, FloatRgba::TRANSPARENT).unwrap();
    let width = u64::from(w);
    for (index, pixel) in image.pixels_mut().iter_mut().enumerate() {
        let i = index as u64;
        let (x, y) = (i % width, i / width);
        let n = |k: u64| ((x * 7 + y * 13 + k * 31 + u64::from(seed) * 17) % 251) as f32 / 251.0;
        *pixel = FloatRgba {
            red: n(1),
            green: n(2),
            blue: n(3),
            alpha: 0.3 + n(4) * 0.7,
        };
    }
    image
}

/// A document with enough going on that a tiling mistake has somewhere to hide:
/// a blend mode, a transform, a group and a neighbourhood operation.
fn document(w: u32, h: u32, store: &mut LayerPixelStore) -> LayerDocument {
    let base = store.register_float(source(w, h, 1)).unwrap();
    let mut doc = LayerDocument::new(w, h);
    doc.precision = DocumentPrecision::LinearSrgbF32;
    doc.layers.push(layer("base", &base, w, h));

    let mut inner = layer("inner", &base, w, h);
    inner.blend_mode = BlendMode::Multiply;
    inner.opacity = 0.7;
    let mut group = layer("group", "unused", w, h);
    group.content = LayerContent::Group {
        children: vec![inner],
        isolated: true,
    };
    group.transform = LayerTransform {
        translate_x: 9.5,
        rotation_degrees: 2.0,
        ..LayerTransform::default()
    };
    doc.layers.push(group);

    let mut blur = layer("blur", "unused", w, h);
    blur.content = LayerContent::Adjustment {
        operation: Box::new(EditOperation::GaussianBlur { radius: 3.0 }),
    };
    doc.layers.push(blur);
    doc
}

/// The whole point of Phase 11, stated as a file comparison: an export that
/// never held the frame must be the same file as one that did.
#[test]
fn a_streamed_export_is_the_same_file_as_a_whole_frame_export() {
    let (w, h) = (700u32, 480u32);
    let mut store = LayerPixelStore::default();
    store.reset(w, h).unwrap();
    let doc = document(w, h, &mut store);
    let resolved = store.resolve(&doc.referenced_pixel_ids(), false).unwrap();
    let options = RenderOptions {
        scale: 1.0,
        cancel: None,
    };

    let folder = tempfile::tempdir().unwrap();
    let input = folder.path().join("input.png");
    std::fs::write(&input, b"sentinel").unwrap();

    for (bit_depth, dither, space) in [
        (16u8, false, RgbColorSpace::Srgb),
        (8, true, RgbColorSpace::Srgb),
        (16, false, RgbColorSpace::DisplayP3),
    ] {
        let export = ColorExportOptions {
            bit_depth,
            dither,
            color_space: space,
        };
        let whole_path = folder
            .path()
            .join(format!("whole-{bit_depth}-{dither}.png"));
        let whole = render_document_float(&doc, &resolved, options).unwrap();
        save_color_image(
            &whole,
            &input,
            &whole_path,
            ExportProfile::Lossless,
            export,
            None,
        )
        .unwrap();

        let streamed_path = folder
            .path()
            .join(format!("streamed-{bit_depth}-{dither}.png"));
        save_color_image_streaming(
            w,
            h,
            &input,
            &streamed_path,
            ExportProfile::Lossless,
            export,
            None,
            |emit| render_document_streaming(&doc, &resolved, options, 128, 0, emit).map(|_| ()),
        )
        .unwrap();

        assert_eq!(
            std::fs::read(&whole_path).unwrap(),
            std::fs::read(&streamed_path).unwrap(),
            "a streamed {bit_depth}-bit export differed from the whole-frame export"
        );
    }
}

/// Tile size is a memory/throughput knob. If it changed the exported file it
/// would be a correctness setting, and could not be tuned freely.
#[test]
fn the_tile_size_does_not_change_the_exported_file() {
    let (w, h) = (512u32, 384u32);
    let mut store = LayerPixelStore::default();
    store.reset(w, h).unwrap();
    let doc = document(w, h, &mut store);
    let resolved = store.resolve(&doc.referenced_pixel_ids(), false).unwrap();
    let options = RenderOptions {
        scale: 1.0,
        cancel: None,
    };
    let folder = tempfile::tempdir().unwrap();
    let input = folder.path().join("input.png");
    std::fs::write(&input, b"sentinel").unwrap();

    let mut reference: Option<Vec<u8>> = None;
    for tile_size in [64u32, 100, 128, 256, 1024] {
        let path = folder.path().join(format!("tile-{tile_size}.png"));
        save_color_image_streaming(
            w,
            h,
            &input,
            &path,
            ExportProfile::Lossless,
            ColorExportOptions {
                bit_depth: 16,
                ..Default::default()
            },
            None,
            |emit| {
                render_document_streaming(&doc, &resolved, options, tile_size, 0, emit).map(|_| ())
            },
        )
        .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        match &reference {
            None => reference = Some(bytes),
            Some(expected) => assert_eq!(
                expected, &bytes,
                "tile size {tile_size} changed the exported file"
            ),
        }
    }
}

/// The CPU reference renderer stays the oracle: the tiled path is checked
/// against it, not the other way round, and not against itself.
#[test]
fn the_tiled_renderer_still_agrees_with_the_reference_renderer() {
    let (w, h) = (400u32, 300u32);
    let mut store = LayerPixelStore::default();
    store.reset(w, h).unwrap();
    let doc = document(w, h, &mut store);
    let resolved = store.resolve(&doc.referenced_pixel_ids(), false).unwrap();
    let options = RenderOptions {
        scale: 1.0,
        cancel: None,
    };
    let reference = render_document_float(&doc, &resolved, options).unwrap();
    let (tiled, stats) = render_document_tiled(&doc, &resolved, options, 64).unwrap();
    assert!(!stats.fell_back_to_full_frame, "the document was not tiled");
    assert!(stats.haloed_tiles > 0, "the blur was given no halo");

    let worst = reference
        .pixels()
        .iter()
        .zip(tiled.pixels())
        .map(|(a, b)| {
            (a.red - b.red)
                .abs()
                .max((a.green - b.green).abs())
                .max((a.blue - b.blue).abs())
                .max((a.alpha - b.alpha).abs())
        })
        .fold(0.0f32, f32::max);
    // Not bit-identical: a haloed blur sums the same taps in a different order.
    // The bound is far below one step of a 16-bit channel (1.5e-5).
    assert!(
        worst < 1e-5,
        "the tiled renderer differed from the reference by {worst}"
    );
}
