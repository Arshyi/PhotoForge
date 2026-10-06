//! Deterministic Phase 10 reference tests, not packaged/GUI evidence.
use photoforge_lib::{
    color::{DevelopmentParameters, FloatImage, FloatRgba},
    domain::{EditOperation, ExportProfile},
    high_precision,
    layers::*,
    pixel::{DocumentPrecision, PixelBuffer},
};
use std::sync::Arc;

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
fn fixture() -> (LayerDocument, LayerPixelStore, FloatImage) {
    let image = FloatImage::new(
        4,
        4,
        (0..16)
            .map(|i| FloatRgba::new(0.123456 + i as f32 * 0.00003, 0.3, 1.25, 0.7501))
            .collect(),
    )
    .unwrap();
    let mut store = LayerPixelStore::default();
    store.reset(4, 4).unwrap();
    let id = store.register_float(image.clone()).unwrap();
    let mut doc = LayerDocument::new(4, 4);
    doc.precision = DocumentPrecision::LinearSrgbF32;
    doc.layers.push(layer("source", &id, 4, 4));
    (doc, store, image)
}

#[test]
fn saved_float_samples_are_bit_exact_and_mixed_precision_is_explicit() {
    let (mut doc, mut store, original) = fixture();
    let ordinary = store
        .register(image::RgbaImage::from_pixel(
            4,
            4,
            image::Rgba([30, 70, 90, 40]),
        ))
        .unwrap();
    doc.layers.push(layer("ordinary", &ordinary, 4, 4));
    let pixels: Vec<_> = doc
        .referenced_pixel_ids()
        .iter()
        .map(|id| (id.clone(), store.full_typed(id).unwrap()))
        .collect();
    let bytes = encode_project_typed(&doc, &[], &pixels, "test", "fixed", "fixed").unwrap();
    assert_eq!(
        bytes,
        encode_project_typed(&doc, &[], &pixels, "test", "fixed", "fixed").unwrap()
    );
    let loaded = decode_project(&bytes).unwrap();
    assert_eq!(loaded.linear_pixels[0].1, original);
    assert_eq!(loaded.pixels.len(), 1);
    assert_eq!(loaded.document, doc);
    let mut changed = bytes;
    let last = changed.len() - 1;
    changed[last] ^= 1;
    assert!(decode_project(&changed).is_err());
}

#[test]
fn float_recovery_is_bit_exact() {
    let (doc, store, original) = fixture();
    let dir = tempfile::tempdir().unwrap();
    let pixels = vec![(
        doc.referenced_pixel_ids()[0].clone(),
        store.full_typed(&doc.referenced_pixel_ids()[0]).unwrap(),
    )];
    let record = write_recovery_snapshot_typed(
        &doc,
        &[],
        &pixels,
        None,
        "float",
        "test",
        "fixed",
        Some(dir.path()),
    )
    .unwrap();
    let loaded = read_recovery_snapshot(std::path::Path::new(&record.snapshot_path)).unwrap();
    assert_eq!(loaded.linear_pixels[0].1, original);
}

#[test]
fn source_mask_adjustment_group_png16_retains_more_than_eight_bits() {
    let (mut doc, store, _) = fixture();
    let mask = photoforge_lib::mask::MaskBitmap::from_coverage(4, 4, vec![128; 16]).unwrap();
    doc.layers[0].mask = Some(LayerMask {
        snapshot: photoforge_lib::mask::MaskSnapshot::encode(&mask),
        enabled: true,
        inverted: false,
    });
    let mut adjustment = layer("adjustment", "unused", 4, 4);
    adjustment.content = LayerContent::Adjustment {
        operation: Box::new(EditOperation::RawDevelopment {
            parameters: DevelopmentParameters {
                exposure_ev: -1.0,
                ..Default::default()
            },
        }),
    };
    let mut group = layer("group", "unused", 4, 4);
    group.content = LayerContent::Group {
        children: vec![adjustment],
        isolated: false,
    };
    doc.layers.push(group);
    let resolved = store.resolve(&doc.referenced_pixel_ids(), false).unwrap();
    let result = render_document_float(&doc, &resolved, RenderOptions::default()).unwrap();
    assert!((result.pixels()[0].red - 0.123456 * 0.5).abs() < 1e-6);
    assert!((result.pixels()[0].alpha - 0.7501 * 128.0 / 255.0).abs() < 1e-6);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("result.png");
    std::fs::write(dir.path().join("source.dng"), b"protected fixture source").unwrap();
    photoforge_lib::infrastructure::save_color_image(
        &result,
        &dir.path().join("source.dng"),
        &path,
        ExportProfile::Lossless,
        Default::default(),
        None,
    )
    .unwrap();
    let png = image::open(&path).unwrap();
    assert_eq!(png.color(), image::ColorType::Rgba16);
    assert!(png
        .to_rgba16()
        .pixels()
        .any(|p| p.0.iter().any(|v| v % 257 != 0)));
    let reopened = photoforge_lib::infrastructure::load_image(&path).unwrap();
    assert!(reopened.working.is_some());
}

#[test]
fn native_png16_input_and_geometry_keep_distinct_nearby_samples() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("native.png");
    image::ImageBuffer::<image::Rgba<u16>, _>::from_raw(
        2,
        1,
        vec![10001, 20002, 30003, 65535, 10002, 20003, 30004, 32768],
    )
    .unwrap()
    .save(&path)
    .unwrap();
    let loaded = photoforge_lib::infrastructure::load_image(&path).unwrap();
    let source = loaded.working.unwrap();
    assert_ne!(source.pixels()[0].red, source.pixels()[1].red);
    let rotated =
        high_precision::apply(&source, &EditOperation::Rotate { degrees: 90 }, None).unwrap();
    assert_eq!(rotated.dimensions(), (1, 2));
    assert_eq!(rotated.pixels(), source.pixels());
}

#[test]
fn point_operations_do_not_introduce_byte_quantization() {
    let (_, _, image) = fixture();
    for operation in [
        EditOperation::Brightness { amount: 0.03 },
        EditOperation::Contrast { amount: 0.1 },
        EditOperation::Saturation { amount: 0.1 },
        EditOperation::Gamma { value: 1.1 },
        EditOperation::Grayscale,
        EditOperation::Sepia,
        EditOperation::TemperatureTint {
            temperature: 0.1,
            tint: 0.1,
        },
        EditOperation::Curves {
            curves: Default::default(),
        },
    ] {
        let result = high_precision::apply(&image, &operation, None).unwrap();
        assert!(result.pixels().iter().all(|p| p.alpha == 0.7501));
        assert_ne!(
            result.pixels()[0].red,
            result.pixels()[1].red,
            "{}",
            operation.kind()
        );
        assert_ne!(
            result,
            FloatImage::from_rgba8(&result.to_rgba8()).unwrap(),
            "{}",
            operation.kind()
        );
    }
}

#[test]
fn spatial_filters_ignore_hidden_rgb_when_sampling_edges() {
    let a = FloatImage::new(
        2,
        1,
        vec![
            FloatRgba::new(0.3, 0.1, 0.2, 1.0),
            FloatRgba::new(0.0, 100.0, 0.0, 0.0),
        ],
    )
    .unwrap();
    let b = FloatImage::new(2, 1, vec![a.pixels()[0], FloatRgba::TRANSPARENT]).unwrap();
    let blur = EditOperation::GaussianBlur { radius: 1.0 };
    assert_eq!(
        high_precision::apply(&a, &blur, None).unwrap(),
        high_precision::apply(&b, &blur, None).unwrap()
    );
}

#[test]
fn empty_typed_pipeline_shares_the_source_and_slider_edits_do_not_mutate_it() {
    let (_, _, image) = fixture();
    let original = Arc::new(image.clone());
    let typed =
        high_precision::pipeline_typed(PixelBuffer::LinearRgbaF32(original.clone()), &[], None)
            .unwrap();
    if let PixelBuffer::LinearRgbaF32(shared) = typed {
        assert!(Arc::ptr_eq(&shared, &original));
    } else {
        panic!("lost float type")
    }
    for amount in [-0.2, 0.1, 0.2] {
        let _ =
            high_precision::apply(&original, &EditOperation::Brightness { amount }, None).unwrap();
    }
    assert_eq!(*original, image);
}

#[test]
fn processing_locality_is_explicit_and_cancellation_is_not_partial_success() {
    assert_eq!(
        high_precision::locality(&EditOperation::Brightness { amount: 0.1 }),
        high_precision::OperationLocality::TileLocal
    );
    assert_eq!(
        high_precision::locality(&EditOperation::GaussianBlur { radius: 2.0 }),
        high_precision::OperationLocality::HaloDependent
    );
    assert_eq!(
        high_precision::locality(&EditOperation::AutoWhiteBalance { strength: 1.0 }),
        high_precision::OperationLocality::Global
    );
    let (_, _, source) = fixture();
    let cancel = std::sync::atomic::AtomicBool::new(true);
    assert!(matches!(
        high_precision::apply(&source, &EditOperation::Grayscale, Some(&cancel)),
        Err(photoforge_lib::error::AppError::RenderCancelled)
    ));
}
