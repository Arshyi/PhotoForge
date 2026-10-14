//! High-precision implementation of the same layer tree as the legacy renderer.
use super::clip::{can_hold_clipped, clip_groups, paints, ClipGroup, Paint};
use super::composite::{decoded_mask, mask_coverage, mask_inverted, render_transform};
use super::{
    BlendMode, Layer, LayerContent, LayerDocument, LayerInterpolation, PixelSource, RenderOptions,
    MAX_GROUP_DEPTH,
};
use crate::color::{srgb_decode, srgb_encode, FloatImage, FloatRgba};
use crate::error::AppError;
use crate::image_processing::high_precision;
use crate::pixel::{DocumentPrecision, PixelBuffer};

pub fn render_document_typed(
    document: &LayerDocument,
    source: &dyn PixelSource,
    options: RenderOptions<'_>,
) -> Result<PixelBuffer, AppError> {
    render_document_typed_cached(document, source, options, None)
}

/// Renders a document, reusing unchanged tiles when a cache is supplied.
///
/// High-precision documents go through the tiled renderer: it is bounded in
/// memory and uses every core, where `render_document_float` allocates whole
/// frames on one thread. That function is still here and still the definition
/// of a correct render — the tiled path falls back to it whenever a document
/// contains an operation that cannot be evaluated on a sub-rectangle, and the
/// equivalence tests compare against it rather than against themselves.
///
/// Legacy 8-bit documents are untouched. They were the pre-0.10.0 renderer and
/// are kept working, not modernised.
pub fn render_document_typed_cached(
    document: &LayerDocument,
    source: &dyn PixelSource,
    options: RenderOptions<'_>,
    cache: Option<&super::TileCache>,
) -> Result<PixelBuffer, AppError> {
    document.validate()?;
    match document.precision {
        DocumentPrecision::LegacySrgb8 => Ok(super::render_layers(
            &document.layers,
            document.canvas_width,
            document.canvas_height,
            source,
            options,
        )?
        .into()),
        DocumentPrecision::LinearSrgbF32 => Ok(super::tiled::render_document_tiled_cached(
            document,
            source,
            options,
            super::tiles::DEFAULT_TILE_SIZE,
            0,
            cache,
        )?
        .0
        .into()),
    }
}

pub fn render_document_float(
    document: &LayerDocument,
    source: &dyn PixelSource,
    options: RenderOptions<'_>,
) -> Result<FloatImage, AppError> {
    document.validate()?;
    if let Some(prepared) = super::smart::prepare_render(document, source, options)? {
        return render_document_float(&prepared.document, &prepared.pixels, options);
    }
    if !options.scale.is_finite() || options.scale <= 0.0 || options.scale > 1.0 {
        return Err(AppError::InvalidLayerDocument(
            "render scale must be greater than zero and no larger than one".into(),
        ));
    }
    crate::resources::ResourceEstimate::render(
        document,
        options.scale,
        source.resident_bytes(),
        source.promotion_bytes(document),
    )?;
    let width = ((f64::from(document.canvas_width) * options.scale).round() as u32).max(1);
    let height = ((f64::from(document.canvas_height) * options.scale).round() as u32).max(1);
    let context = Context {
        source,
        options,
        width,
        height,
    };
    let mut canvas = FloatImage::blank(width, height, FloatRgba::TRANSPARENT)?;
    composite_onto(&mut canvas, &document.layers, &context, 1)?;
    canvas.validate()?;
    Ok(canvas)
}

struct Context<'a> {
    source: &'a dyn PixelSource,
    options: RenderOptions<'a>,
    width: u32,
    height: u32,
}

fn composite_onto(
    canvas: &mut FloatImage,
    layers: &[Layer],
    context: &Context<'_>,
    depth: usize,
) -> Result<(), AppError> {
    if depth > MAX_GROUP_DEPTH {
        return Err(AppError::LayerDepthExceeded {
            depth,
            limit: MAX_GROUP_DEPTH,
        });
    }
    for group in clip_groups(layers) {
        high_precision::check_cancel(context.options.cancel)?;
        if group.needs_buffer() {
            paint_clip_group(canvas, &group, context, depth)?;
            continue;
        }
        paint_layer(canvas, group.base, Paint::of(group.base), context, depth)?;
        // A base with no coverage of its own lets the layers above it paint as
        // ordinary ones; any other base with nothing visible clipped to it has
        // nothing more to draw.
        if !can_hold_clipped(group.base) {
            for layer in group.clipped {
                paint_layer(canvas, layer, Paint::of(layer), context, depth)?;
            }
        }
    }
    high_precision::check_cancel(context.options.cancel)
}

/// Composites one layer onto `canvas` with the given paint.
fn paint_layer(
    canvas: &mut FloatImage,
    layer: &Layer,
    paint: Paint,
    context: &Context<'_>,
    depth: usize,
) -> Result<(), AppError> {
    if !paints(layer) {
        return Ok(());
    }
    match &layer.content {
        LayerContent::SmartObject { .. } => {
            return Err(AppError::InvalidLayerDocument(
                "smart content was not prepared for rendering".into(),
            ));
        }
        LayerContent::Pixel { pixel_id, .. } => {
            let pixels = context.source.resolve_linear(pixel_id)?;
            draw(canvas, &pixels, layer, paint, context)?;
        }
        LayerContent::Shape { shape } => {
            // Delegated to the tiled renderer's routine over a region
            // covering the whole canvas. Two implementations of shape
            // drawing would be two things to keep identical, and this
            // module is the oracle the other one is checked against.
            super::tiled::draw_shape_full_frame(canvas, shape, layer, paint, context.options)?;
        }
        LayerContent::Text { text } => {
            // Delegated for the same reason as a shape: one implementation
            // of glyph drawing, exercised from both renderers.
            super::tiled::draw_text_full_frame(canvas, text, layer, paint, context.options)?;
        }
        LayerContent::Group { children, isolated } => {
            if children.is_empty() {
                return Ok(());
            }
            if *isolated || paint.in_clip_group {
                let mut group =
                    FloatImage::blank(context.width, context.height, FloatRgba::TRANSPARENT)?;
                composite_onto(&mut group, children, context, depth + 1)?;
                draw(canvas, &group, layer, paint, context)?;
            } else {
                let mut reworked = canvas.clone();
                composite_onto(&mut reworked, children, context, depth + 1)?;
                cross_fade(canvas, &reworked, layer, context)?;
            }
        }
        LayerContent::Adjustment { operation } => {
            let adjusted = high_precision::apply(canvas, operation, context.options.cancel)?;
            mix_adjustment(canvas, &adjusted, layer, context)?;
        }
    }
    Ok(())
}

/// Composites a base and the layers clipped to it as one unit; see `clip.rs`.
fn paint_clip_group(
    canvas: &mut FloatImage,
    group: &ClipGroup<'_>,
    context: &Context<'_>,
    depth: usize,
) -> Result<(), AppError> {
    let mut buffer = FloatImage::blank(context.width, context.height, FloatRgba::TRANSPARENT)?;
    paint_layer(&mut buffer, group.base, Paint::base(), context, depth)?;
    for layer in group.clipped {
        paint_layer(&mut buffer, layer, Paint::clipped(layer), context, depth)?;
    }
    lay_over(
        canvas,
        &buffer,
        group.base.opacity,
        group.base.blend_mode,
        context,
    )
}

/// Composites a canvas-sized buffer onto `canvas`, in place, at an opacity and
/// in a blend mode. The buffer's own placement has already been applied.
fn lay_over(
    canvas: &mut FloatImage,
    buffer: &FloatImage,
    opacity: f32,
    blend: BlendMode,
    context: &Context<'_>,
) -> Result<(), AppError> {
    for y in 0..context.height {
        high_precision::check_cancel(context.options.cancel)?;
        for x in 0..context.width {
            let i = (y * context.width + x) as usize;
            let mut p = buffer.pixels()[i];
            p.alpha *= opacity;
            if p.alpha <= 0.0 {
                continue;
            }
            canvas.pixels_mut()[i] = source_over(canvas.pixels()[i], p, blend);
        }
    }
    Ok(())
}

fn channels(p: FloatRgba) -> [f32; 3] {
    [p.red, p.green, p.blue]
}

/// A blend of linear light, for operands that may lie outside 0 to 1.
///
/// **Seven modes have no ceiling**: Normal, Multiply, Screen, Darken, Lighten, Difference and
/// Exclusion map the unit square into itself, so their arithmetic is the same inside it and is
/// extended to any value, and a highlight under them is kept exactly.
///
/// **Every other mode is defined on 0 to 1** (it pivots on mid-grey, or clamps its own result as
/// part of its definition, so applying it to a value above white would change what it does to
/// values below). It is applied to the operands clamped to that range, as it always was, and then
/// **what the backdrop held above white is added back**: `result = f(clamp(b), clamp(s)) +
/// max(b - 1, 0)`. So
///
/// * with both operands in range, and for anything below 0, the result is exactly what it was;
/// * a backdrop highlight is never flattened to white by these modes: it passes through, and the
///   mode still acts on the part of the picture that is in range (a neutral source leaves any
///   backdrop alone, above white too);
/// * the result is continuous at white and never lower than the clamped one;
/// * a **source** above white counts as white: it is clamped, and its excess is not added (the
///   modes disagree about what a source highlight should do, and a wrong guess is worse than none).
///
/// W3C component modes are evaluated in encoded float sRGB, never linear HSL.
pub fn blend_linear(mode: BlendMode, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    match mode {
        BlendMode::Normal => return s,
        BlendMode::Multiply => return std::array::from_fn(|i| b[i] * s[i]),
        BlendMode::Screen => return std::array::from_fn(|i| b[i] + s[i] - b[i] * s[i]),
        BlendMode::Darken => return std::array::from_fn(|i| b[i].min(s[i])),
        BlendMode::Lighten => return std::array::from_fn(|i| b[i].max(s[i])),
        BlendMode::Difference => return std::array::from_fn(|i| (b[i] - s[i]).abs()),
        BlendMode::Exclusion => {
            return std::array::from_fn(|i| b[i] + s[i] - 2.0 * b[i] * s[i]);
        }
        _ => {}
    }
    let within = |value: f32| {
        if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let (inside_b, inside_s) = (b.map(within), s.map(within));
    let blended = if mode.is_separable() {
        mode.blend(inside_b, inside_s)
    } else {
        mode.blend(inside_b.map(srgb_encode), inside_s.map(srgb_encode))
            .map(srgb_decode)
    };
    // What the backdrop held above white.
    std::array::from_fn(|i| {
        let highlight = if b[i].is_finite() {
            (b[i] - 1.0).max(0.0)
        } else {
            0.0
        };
        blended[i] + highlight
    })
}

pub fn source_over(back: FloatRgba, source: FloatRgba, mode: BlendMode) -> FloatRgba {
    let ab = back.alpha;
    let a = source.alpha;
    let ao = a + ab * (1.0 - a);
    if ao <= 0.0 {
        return FloatRgba::TRANSPARENT;
    }
    let b = channels(back);
    let s = channels(source);
    let blended = blend_linear(mode, b, s);
    let rgb: [f32; 3] = std::array::from_fn(|i| {
        ((1.0 - ab) * a * s[i] + ab * a * blended[i] + (1.0 - a) * ab * b[i]) / ao
    });
    FloatRgba::new(rgb[0], rgb[1], rgb[2], ao.clamp(0.0, 1.0))
}

/// `source` composited atop `back`: the backdrop keeps its own coverage and only
/// its colour moves, by the source's coverage and through the blend mode.
///
/// What a clipped layer does to the buffer of its clipping group; see `clip.rs`.
pub fn source_atop(back: FloatRgba, source: FloatRgba, mode: BlendMode) -> FloatRgba {
    if back.alpha <= 0.0 {
        return back;
    }
    let a = source.alpha.clamp(0.0, 1.0);
    let b = channels(back);
    let blended = blend_linear(mode, b, channels(source));
    let rgb: [f32; 3] = std::array::from_fn(|i| b[i] + a * (blended[i] - b[i]));
    FloatRgba::new(rgb[0], rgb[1], rgb[2], back.alpha)
}

/// Draws a pixel buffer through its transform and mask, with the opacity, blend
/// mode and mode of coverage in `paint`.
fn draw(
    canvas: &mut FloatImage,
    source: &FloatImage,
    layer: &Layer,
    paint: Paint,
    context: &Context<'_>,
) -> Result<(), AppError> {
    let (sw, sh) = source.dimensions();
    let transform = render_transform(&layer.transform, context.options.scale);
    let Some((x0, y0, x1, y1)) = transform
        .document_bounds(sw, sh)
        .clip_to_canvas(context.width, context.height)
    else {
        return Ok(());
    };
    let inverse = transform.inverse(sw, sh)?;
    let mask = decoded_mask(layer)?;
    for y in y0..y1 {
        high_precision::check_cancel(context.options.cancel)?;
        for x in x0..x1 {
            let (sx, sy) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
            let mut p = source.sample(
                sx,
                sy,
                layer.transform.interpolation == LayerInterpolation::Nearest,
            );
            p.alpha *= paint.opacity
                * mask_coverage(
                    mask.as_ref(),
                    mask_inverted(layer),
                    sx,
                    sy,
                    sw,
                    sh,
                    layer.transform.interpolation,
                );
            if p.alpha <= 0.0 {
                continue;
            }
            let i = (y * context.width + x) as usize;
            canvas.pixels_mut()[i] = if paint.atop {
                source_atop(canvas.pixels()[i], p, paint.blend)
            } else {
                source_over(canvas.pixels()[i], p, paint.blend)
            };
        }
    }
    Ok(())
}

fn cross_fade(
    canvas: &mut FloatImage,
    changed: &FloatImage,
    layer: &Layer,
    context: &Context<'_>,
) -> Result<(), AppError> {
    let mask = decoded_mask(layer)?;
    let inverse = render_transform(&layer.transform, context.options.scale)
        .inverse(context.width, context.height)?;
    for y in 0..context.height {
        high_precision::check_cancel(context.options.cancel)?;
        for x in 0..context.width {
            let (sx, sy) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
            let t = layer.opacity
                * mask_coverage(
                    mask.as_ref(),
                    mask_inverted(layer),
                    sx,
                    sy,
                    context.width,
                    context.height,
                    layer.transform.interpolation,
                );
            if t <= 0.0 {
                continue;
            }
            let i = (y * context.width + x) as usize;
            let a = canvas.pixels()[i];
            let b = changed.pixels()[i];
            let alpha = a.alpha + t * (b.alpha - a.alpha);
            let p = if alpha <= 0.0 {
                FloatRgba::TRANSPARENT
            } else {
                let rgb: [f32; 3] = std::array::from_fn(|c| {
                    (channels(a)[c] * a.alpha * (1.0 - t) + channels(b)[c] * b.alpha * t) / alpha
                });
                FloatRgba::new(rgb[0], rgb[1], rgb[2], alpha.clamp(0.0, 1.0))
            };
            canvas.pixels_mut()[i] = p;
        }
    }
    Ok(())
}

fn mix_adjustment(
    canvas: &mut FloatImage,
    changed: &FloatImage,
    layer: &Layer,
    context: &Context<'_>,
) -> Result<(), AppError> {
    if canvas.dimensions() != changed.dimensions() {
        return Err(AppError::InvalidLayerDocument(
            "an adjustment layer must preserve the canvas dimensions".into(),
        ));
    }
    let mask = decoded_mask(layer)?;
    let inverse = render_transform(&layer.transform, context.options.scale)
        .inverse(context.width, context.height)?;
    for y in 0..context.height {
        high_precision::check_cancel(context.options.cancel)?;
        for x in 0..context.width {
            let (sx, sy) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
            let coverage = layer.opacity
                * mask_coverage(
                    mask.as_ref(),
                    mask_inverted(layer),
                    sx,
                    sy,
                    context.width,
                    context.height,
                    layer.transform.interpolation,
                );
            if coverage <= 0.0 {
                continue;
            }
            let i = (y * context.width + x) as usize;
            let base = canvas.pixels()[i];
            let blended = blend_linear(
                layer.blend_mode,
                channels(base),
                channels(changed.pixels()[i]),
            );
            let rgb: [f32; 3] = std::array::from_fn(|c| {
                channels(base)[c] + coverage * (blended[c] - channels(base)[c])
            });
            canvas.pixels_mut()[i] = FloatRgba::new(rgb[0], rgb[1], rgb[2], base.alpha);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::{test_pixel_layer, LayerMask, LayerPixelStore};
    use crate::mask::{MaskBitmap, MaskSnapshot};
    fn fixture() -> (LayerDocument, LayerPixelStore) {
        let mut store = LayerPixelStore::default();
        store.reset(4, 4).unwrap();
        let id = store
            .register_float(
                FloatImage::blank(4, 4, FloatRgba::new(0.123456, 0.345678, 1.25, 0.5)).unwrap(),
            )
            .unwrap();
        let mut doc = LayerDocument::new(4, 4);
        doc.precision = DocumentPrecision::LinearSrgbF32;
        doc.layers.push(test_pixel_layer("raw", &id, 4, 4));
        (doc, store)
    }
    fn render(doc: &LayerDocument, store: &LayerPixelStore) -> FloatImage {
        let resolved = store.resolve(&doc.referenced_pixel_ids(), false).unwrap();
        render_document_float(doc, &resolved, RenderOptions::default()).unwrap()
    }
    #[test]
    fn single_source_keeps_hdr_fractional_samples_and_alpha() {
        let (doc, store) = fixture();
        let p = render(&doc, &store).pixels()[0];
        assert_eq!(p, FloatRgba::new(0.123456, 0.345678, 1.25, 0.5));
    }
    #[test]
    fn transparent_edges_are_interpolated_in_premultiplied_light() {
        let image = FloatImage::new(
            2,
            1,
            vec![
                FloatRgba::new(1.0, 0.0, 0.0, 1.0),
                FloatRgba::new(0.0, 100.0, 0.0, 0.0),
            ],
        )
        .unwrap();
        let p = image.sample(1.0, 0.5, false);
        assert_eq!(p, FloatRgba::new(1.0, 0.0, 0.0, 0.5));
    }
    #[test]
    fn tiny_alpha_and_all_blends_are_finite() {
        for mode in BlendMode::ALL {
            for a in [0.0, 1e-20, 0.5, 1.0] {
                let p = source_over(
                    FloatRgba::new(-0.25, 4.0, 0.5, a),
                    FloatRgba::new(2.0, 0.0, -1.0, a),
                    mode,
                );
                assert!(p.is_finite());
            }
        }
    }
    #[test]
    fn source_over_matches_linear_light_reference() {
        let p = source_over(
            FloatRgba::new(0.0, 0.0, 1.0, 1.0),
            FloatRgba::new(1.0, 0.0, 0.0, 0.5),
            BlendMode::Normal,
        );
        assert_eq!(p, FloatRgba::new(0.5, 0.0, 0.5, 1.0));
        assert_eq!(p.to_srgba8(), [188, 0, 188, 255]);
    }
    #[test]
    fn masks_and_transforms_preserve_fractional_rgb() {
        let (mut doc, store) = fixture();
        doc.layers[0].mask = Some(LayerMask {
            snapshot: MaskSnapshot::encode(
                &MaskBitmap::from_coverage(4, 4, vec![128; 16]).unwrap(),
            ),
            enabled: true,
            inverted: false,
        });
        doc.layers[0].transform.translate_x = 1.0;
        let image = render(&doc, &store);
        assert_eq!(image.get(0, 0).unwrap().alpha, 0.0);
        let p = image.get(1, 1).unwrap();
        assert!((p.red - 0.123456).abs() < 1e-6);
        assert!((p.alpha - 128.0 / 255.0 * 0.5).abs() < 1e-6);
    }
    #[test]
    fn pass_through_and_isolated_adjustments_have_distinct_backdrops() {
        let (mut doc, store) = fixture();
        let mut adjustment = test_pixel_layer("adjustment", "unused", 4, 4);
        adjustment.content = LayerContent::Adjustment {
            operation: Box::new(crate::domain::EditOperation::RawDevelopment {
                parameters: crate::color::DevelopmentParameters {
                    exposure_ev: -1.0,
                    ..Default::default()
                },
            }),
        };
        let mut group = test_pixel_layer("group", "unused", 4, 4);
        group.content = LayerContent::Group {
            children: vec![adjustment],
            isolated: false,
        };
        group.opacity = 0.5;
        doc.layers.push(group);
        let p = render(&doc, &store).pixels()[0];
        assert!((p.red - 0.123456 * 0.75).abs() < 1e-6);
        assert_eq!(p.alpha, 0.5);
        if let LayerContent::Group { isolated, .. } = &mut doc.layers[1].content {
            *isolated = true;
        }
        assert!((render(&doc, &store).pixels()[0].red - 0.123456).abs() < 1e-6);
    }
    #[test]
    fn repeated_renders_are_deterministic_and_png16_is_not_byte_expansion() {
        let (doc, store) = fixture();
        let a = render(&doc, &store);
        let b = render(&doc, &store);
        assert_eq!(a, b);
        let decoded = image::load_from_memory(&a.encode_png16().unwrap())
            .unwrap()
            .to_rgba16();
        assert!(decoded.pixels().any(|p| p[0] % 257 != 0));
    }
    #[test]
    fn cancelled_render_returns_no_partial_canvas() {
        let (doc, store) = fixture();
        let resolved = store.resolve(&doc.referenced_pixel_ids(), false).unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(true);
        assert!(matches!(
            render_document_float(
                &doc,
                &resolved,
                RenderOptions {
                    scale: 1.0,
                    cancel: Some(&cancel)
                }
            ),
            Err(AppError::RenderCancelled)
        ));
    }
}

#[cfg(test)]
mod highlight_tests {
    use super::*;

    const UNBOUNDED: [BlendMode; 7] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Difference,
        BlendMode::Exclusion,
    ];

    /// What `blend_linear` did before highlights were passed through: the mode on the clamped
    /// operands (through encoded sRGB for the component modes), with nothing added back.
    fn clamped_reference(mode: BlendMode, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
        if !mode.is_separable() {
            return mode
                .blend(b.map(srgb_encode), s.map(srgb_encode))
                .map(srgb_decode);
        }
        mode.blend(b, s)
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() <= 1e-5)
    }

    #[test]
    fn in_range_operands_give_exactly_what_every_mode_always_gave() {
        let levels = [0.0_f32, 0.07, 0.25, 0.5, 0.5001, 0.8, 1.0];
        for mode in BlendMode::ALL {
            if UNBOUNDED.contains(&mode) {
                continue;
            }
            for &r in &levels {
                for &g in &levels {
                    for &s in &levels {
                        let b = [r, g, 1.0 - r];
                        let src = [s, 1.0 - g, g];
                        assert_eq!(
                            blend_linear(mode, b, src),
                            clamped_reference(mode, b, src),
                            "{mode:?} {b:?} {src:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_backdrop_highlight_passes_through_the_modes_defined_on_zero_to_one() {
        for mode in BlendMode::ALL {
            if UNBOUNDED.contains(&mode) {
                continue;
            }
            for source in [[0.0, 0.0, 0.0], [0.3, 0.5, 0.9], [1.0, 1.0, 1.0]] {
                let backdrop = [4.0, 1.25, 0.4];
                let got = blend_linear(mode, backdrop, source);
                let inside = clamped_reference(mode, [1.0, 1.0, 0.4], source);
                let want = [inside[0] + 3.0, inside[1] + 0.25, inside[2]];
                assert!(
                    close(got, want),
                    "{mode:?} {source:?}: {got:?} against {want:?}"
                );
                // never below what the clamped mode gave: a highlight is not made darker
                assert!(got[0] >= inside[0] && got[1] >= inside[1]);
            }
        }
    }

    #[test]
    fn a_neutral_source_leaves_a_highlight_alone() {
        // Mid-grey is the neutral operand of these three, so a backdrop of any brightness is
        // returned as it was, above white included.
        for mode in [
            BlendMode::Overlay,
            BlendMode::SoftLight,
            BlendMode::HardLight,
        ] {
            let backdrop = [6.0, 0.2, 1.0];
            let got = blend_linear(mode, backdrop, [0.5, 0.5, 0.5]);
            assert!(close(got, backdrop), "{mode:?}: {got:?}");
        }
    }

    #[test]
    fn a_source_above_white_counts_as_white_under_those_modes() {
        for mode in BlendMode::ALL {
            if UNBOUNDED.contains(&mode) {
                continue;
            }
            let backdrop = [0.3, 0.7, 2.0];
            assert_eq!(
                blend_linear(mode, backdrop, [9.0, 1.5, 1.0]),
                blend_linear(mode, backdrop, [1.0, 1.0, 1.0]),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn the_seven_unbounded_modes_keep_their_highlights_exactly() {
        let (b, s) = ([4.0, 0.5, 2.0], [0.5, 3.0, 2.0]);
        assert_eq!(blend_linear(BlendMode::Normal, b, s), s);
        assert_eq!(blend_linear(BlendMode::Multiply, b, s), [2.0, 1.5, 4.0]);
        assert_eq!(
            blend_linear(BlendMode::Screen, b, s),
            [4.0 + 0.5 - 2.0, 0.5 + 3.0 - 1.5, 2.0 + 2.0 - 4.0]
        );
        assert_eq!(blend_linear(BlendMode::Darken, b, s), [0.5, 0.5, 2.0]);
        assert_eq!(blend_linear(BlendMode::Lighten, b, s), [4.0, 3.0, 2.0]);
        assert_eq!(blend_linear(BlendMode::Difference, b, s), [3.5, 2.5, 0.0]);
    }

    #[test]
    fn every_mode_stays_finite_for_extreme_operands() {
        let extremes = [
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            -3.0,
            0.0,
            1.0,
            1e30,
        ];
        for mode in BlendMode::ALL {
            for &x in &extremes {
                for &y in &extremes {
                    let out = blend_linear(mode, [x, y, 0.5], [y, x, 2.0]);
                    // the seven unbounded modes are plain arithmetic, and may be as large as the
                    // inputs; the others must never turn a finite backdrop into a non-finite
                    if !UNBOUNDED.contains(&mode) && x.is_finite() && y.is_finite() {
                        assert!(
                            out.iter().all(|v| v.is_finite()),
                            "{mode:?} {x} {y}: {out:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_picture_with_a_highlight_keeps_it_through_an_overlay_layer() {
        // Source-over of a half-grey Overlay layer, fully opaque, on a backdrop of 8.0: the
        // highlight is what it was (it used to come out as 1.0).
        let back = FloatRgba::new(8.0, 8.0, 8.0, 1.0);
        let source = FloatRgba::new(0.5, 0.5, 0.5, 1.0);
        let out = source_over(back, source, BlendMode::Overlay);
        assert!((out.red - 8.0).abs() < 1e-4 && (out.green - 8.0).abs() < 1e-4);
        // and at half coverage it is a mix of what it was and what the mode gives, still 8.0
        let half = source_over(back, FloatRgba::new(0.5, 0.5, 0.5, 0.5), BlendMode::Overlay);
        assert!((half.red - 8.0).abs() < 1e-4);
    }
}
