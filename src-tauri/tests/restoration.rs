//! Phase 12 restoration gates.
//!
//! Every assertion here is a number measured against a known-clean image, not
//! an opinion about appearance. Two metrics are always checked together:
//! removing noise is easy if you may also remove the picture, so a denoise that
//! improves error while destroying edges fails.
use photoforge_lib::{
    color::{FloatImage, FloatRgba},
    domain::EditOperation,
    fixtures, high_precision,
    layers::{
        render_document_tiled, BlendMode, Layer, LayerContent, LayerDocument, LayerMetadata,
        LayerPixelStore, LayerTransform, RenderOptions,
    },
    metrics,
    pixel::DocumentPrecision,
};

fn apply(image: &FloatImage, operation: &EditOperation) -> FloatImage {
    high_precision::apply(image, operation, None).expect("restoration operation")
}

fn denoise(strength: f32, preserve: f32, color: f32) -> EditOperation {
    EditOperation::Denoise {
        strength,
        preserve_edges: preserve,
        color,
    }
}

/// The central denoise claim: less noise where the image is flat, and the
/// edges still there afterwards.
#[test]
fn denoise_removes_noise_without_removing_edges() {
    let clean = fixtures::scene(192, 192);
    // Strength is the user's statement of how much noise to assume, so it is
    // paired with the noise present rather than held fixed. Testing one setting
    // across every noise level would be testing a mis-set control.
    for (sigma, strength, seed) in [(0.03f32, 0.6f32, 11u64), (0.06, 0.85, 12), (0.09, 1.0, 13)] {
        let noisy = fixtures::gaussian_noise(&clean, sigma, seed);
        let restored = apply(&noisy, &denoise(strength, 0.5, 0.5));

        let noise_before = metrics::flat_area_noise(&clean, &noisy);
        let noise_after = metrics::flat_area_noise(&clean, &restored);
        assert!(
            noise_after < noise_before * 0.7,
            "sigma {sigma}: flat-area noise only went {noise_before} -> {noise_after}"
        );

        // The edge test is the one that stops a denoiser cheating. A plain blur
        // would pass the noise test above and fail this.
        let edges = metrics::edge_retention(&clean, &restored);
        assert!(
            edges > 0.85,
            "sigma {sigma}: only {edges} of the edge energy survived"
        );
    }
}

/// Colour noise is the case the luma/chroma split exists for, so it is checked
/// separately rather than assumed to follow from the luminance result.
#[test]
fn the_colour_control_removes_colour_noise() {
    let clean = fixtures::scene(192, 192);
    let noisy = fixtures::chroma_noise(&clean, 0.07, 21);

    let luma_only = apply(&noisy, &denoise(0.7, 0.5, 0.0));
    let with_colour = apply(&noisy, &denoise(0.7, 0.5, 0.9));

    let without = metrics::psnr(&clean, &luma_only);
    let with = metrics::psnr(&clean, &with_colour);
    assert!(
        with > without + 4.0,
        "the colour control added only {:.2} dB on colour noise ({without:.2} -> {with:.2})",
        with - without
    );
    assert!(
        metrics::edge_retention(&clean, &with_colour) > 0.9,
        "colour denoise damaged luminance edges"
    );
}

/// Detail has to be a real control, not a label: turning it up must measurably
/// keep more edge energy.
#[test]
fn the_detail_control_trades_smoothing_for_edges() {
    let clean = fixtures::scene(192, 192);
    let noisy = fixtures::gaussian_noise(&clean, 0.06, 31);
    let smooth = apply(&noisy, &denoise(0.8, 0.0, 0.0));
    let detailed = apply(&noisy, &denoise(0.8, 1.0, 0.0));

    let smooth_edges = metrics::edge_retention(&clean, &smooth);
    let detailed_edges = metrics::edge_retention(&clean, &detailed);
    assert!(
        detailed_edges > smooth_edges,
        "Detail 1 kept {detailed_edges} of the edges against Detail 0's {smooth_edges}"
    );
    let smooth_noise = metrics::flat_area_noise(&clean, &smooth);
    let detailed_noise = metrics::flat_area_noise(&clean, &detailed);
    assert!(
        smooth_noise < detailed_noise,
        "Detail 0 left {smooth_noise} noise against Detail 1's {detailed_noise}, so the control does nothing"
    );
}

/// The dedicated defect tool exists because a bilateral filter cannot repair an
/// impulse. This asserts the gap rather than assuming it.
#[test]
fn defect_removal_beats_denoise_on_impulse_damage() {
    let clean = fixtures::scene(192, 192);
    let (damaged, placed) = fixtures::sensor_defects(&clean, 60, 41);
    assert!(!placed.is_empty());

    let denoised = apply(&damaged, &denoise(0.8, 0.5, 0.5));
    let repaired = apply(
        &damaged,
        &EditOperation::RemoveDefects {
            strength: 1.0,
            threshold: 3.0,
        },
    );

    let by_denoise = metrics::psnr(&clean, &denoised);
    let by_repair = metrics::psnr(&clean, &repaired);
    assert!(
        by_repair > by_denoise + 5.0,
        "defect repair scored {by_repair:.2} dB against denoise's {by_denoise:.2}"
    );
    assert!(
        by_repair > metrics::psnr(&clean, &damaged) + 5.0,
        "defect repair did not improve on the damaged image"
    );
}

/// The hard requirement for a defect detector: it must not eat real detail.
/// The fixture scene contains single-pixel lines, which are exactly what a
/// careless outlier rejector destroys.
#[test]
fn defect_removal_leaves_a_clean_image_essentially_alone() {
    let clean = fixtures::scene(192, 192);
    let processed = apply(
        &clean,
        &EditOperation::RemoveDefects {
            strength: 1.0,
            threshold: 3.0,
        },
    );
    let psnr = metrics::psnr(&clean, &processed);
    assert!(
        psnr > 45.0,
        "running defect removal on an undamaged image cost {psnr:.2} dB"
    );
    let edges = metrics::edge_retention(&clean, &processed);
    assert!(
        edges > 0.98,
        "defect removal erased {:.1}% of a clean image's edges",
        (1.0 - edges) * 100.0
    );
}

/// A higher threshold must mean a more conservative detector, or the control is
/// decorative.
#[test]
fn the_defect_threshold_controls_how_much_is_repaired() {
    let clean = fixtures::scene(128, 128);
    let (damaged, _) = fixtures::sensor_defects(&clean, 40, 51);
    let eager = apply(
        &damaged,
        &EditOperation::RemoveDefects {
            strength: 1.0,
            threshold: 1.0,
        },
    );
    let cautious = apply(
        &damaged,
        &EditOperation::RemoveDefects {
            strength: 1.0,
            threshold: 9.0,
        },
    );
    let changed = |a: &FloatImage, b: &FloatImage| {
        a.pixels()
            .iter()
            .zip(b.pixels())
            .filter(|(p, q)| p.red != q.red || p.green != q.green || p.blue != q.blue)
            .count()
    };
    assert!(
        changed(&damaged, &eager) > changed(&damaged, &cautious),
        "threshold 1 changed no more pixels than threshold 9"
    );
}

fn document_with(image: FloatImage, operation: EditOperation) -> (LayerDocument, LayerPixelStore) {
    let (width, height) = image.dimensions();
    let mut store = LayerPixelStore::default();
    store.reset(width, height).unwrap();
    let id = store.register_float(image).unwrap();
    let base = Layer {
        id: "base".into(),
        name: "base".into(),
        visible: true,
        locked: false,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        transform: LayerTransform::default(),
        mask: None,
        collapsed: false,
        metadata: LayerMetadata::default(),
        raw: None,
        content: LayerContent::Pixel {
            pixel_id: id,
            width,
            height,
        },
    };
    let adjustment = Layer {
        id: "restore".into(),
        name: "restore".into(),
        content: LayerContent::Adjustment {
            operation: Box::new(operation),
        },
        ..base.clone()
    };
    let mut document = LayerDocument::new(width, height);
    document.precision = DocumentPrecision::LinearSrgbF32;
    document.layers = vec![base, adjustment];
    (document, store)
}

/// Restoration has to survive the tiled renderer. A halo that does not match
/// the algorithm's real reach shows up here as a seam, and nowhere else.
#[test]
fn restoration_is_seamless_under_the_tiled_renderer() {
    let clean = fixtures::scene(192, 192);
    let noisy = fixtures::gaussian_noise(&clean, 0.05, 61);
    let (damaged, _) = fixtures::sensor_defects(&noisy, 30, 62);

    for operation in [
        denoise(0.8, 0.5, 0.8),
        denoise(1.0, 0.0, 1.0),
        EditOperation::RemoveDefects {
            strength: 1.0,
            threshold: 3.0,
        },
    ] {
        let (document, store) = document_with(damaged.clone(), operation.clone());
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let options = RenderOptions {
            scale: 1.0,
            cancel: None,
        };
        let reference =
            photoforge_lib::layers::render_document_float(&document, &resolved, options).unwrap();

        for tile_size in [64u32, 96, 128] {
            let (tiled, stats) =
                render_document_tiled(&document, &resolved, options, tile_size).unwrap();
            assert!(
                !stats.fell_back_to_full_frame,
                "{} was rendered whole rather than tiled",
                operation.kind()
            );
            let worst = reference
                .pixels()
                .iter()
                .zip(tiled.pixels())
                .map(|(a, b)| {
                    (a.red - b.red)
                        .abs()
                        .max((a.green - b.green).abs())
                        .max((a.blue - b.blue).abs())
                })
                .fold(0.0f32, f32::max);
            // Well below one step of a 16-bit channel (1.5e-5), so no exported
            // pixel can differ.
            assert!(
                worst < 1e-5,
                "{} seamed by {worst} at tile size {tile_size}",
                operation.kind()
            );
        }
    }
}

/// Restoration parameters come from a document, which is untrusted input.
/// Nothing here may panic, hang, or produce a non-finite pixel.
#[test]
fn extreme_restoration_parameters_are_refused_or_survived() {
    let image = fixtures::scene(64, 64);
    let hostile = [
        denoise(f32::NAN, 0.5, 0.5),
        denoise(f32::INFINITY, 0.5, 0.5),
        denoise(-5.0, 0.5, 0.5),
        denoise(1e9, 0.5, 0.5),
        denoise(0.5, f32::NAN, 0.5),
        denoise(0.5, 0.5, f32::NEG_INFINITY),
        EditOperation::RemoveDefects {
            strength: f32::NAN,
            threshold: 3.0,
        },
        EditOperation::RemoveDefects {
            strength: 1.0,
            threshold: f32::INFINITY,
        },
        EditOperation::RemoveDefects {
            strength: 1.0,
            threshold: -100.0,
        },
    ];
    for operation in hostile {
        // Validation is the first line of defence and should reject these.
        assert!(
            !operation.validate().is_ok(),
            "{operation:?} passed validation"
        );
        // If one is applied anyway, it must still not produce a broken image.
        if let Ok(result) = high_precision::apply(&image, &operation, None) {
            assert!(
                result.validate().is_ok(),
                "{operation:?} produced an invalid image"
            );
            assert!(
                result.pixels().iter().all(|p| p.is_finite()),
                "{operation:?} produced a non-finite pixel"
            );
        }
    }
}

/// A zero-sized or one-pixel image must not index out of bounds.
#[test]
fn degenerate_image_sizes_are_handled() {
    for (width, height) in [(1u32, 1u32), (1, 64), (64, 1), (2, 2), (3, 3)] {
        let image = FloatImage::blank(width, height, FloatRgba::new(0.5, 0.4, 0.3, 1.0)).unwrap();
        for operation in [
            denoise(1.0, 0.5, 1.0),
            EditOperation::RemoveDefects {
                strength: 1.0,
                threshold: 3.0,
            },
        ] {
            let result = high_precision::apply(&image, &operation, None)
                .unwrap_or_else(|error| panic!("{width}x{height} {operation:?}: {error}"));
            assert_eq!(result.dimensions(), (width, height));
            assert!(result.validate().is_ok());
        }
    }
}
