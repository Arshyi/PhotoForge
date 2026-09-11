//! High-precision implementation of the same layer tree as the legacy renderer.
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
    for layer in layers {
        high_precision::check_cancel(context.options.cancel)?;
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        match &layer.content {
            LayerContent::SmartObject { .. } => {
                return Err(AppError::InvalidLayerDocument(
                    "smart content was not prepared for rendering".into(),
                ));
            }
            LayerContent::Pixel { pixel_id, .. } => {
                let pixels = context.source.resolve_linear(pixel_id)?;
                draw(canvas, &pixels, layer, context)?;
            }
            LayerContent::Shape { shape } => {
                // Delegated to the tiled renderer's routine over a region
                // covering the whole canvas. Two implementations of shape
                // drawing would be two things to keep identical, and this
                // module is the oracle the other one is checked against.
                super::tiled::draw_shape_full_frame(canvas, shape, layer, context.options)?;
            }
            LayerContent::Text { text } => {
                // Delegated for the same reason as a shape: one implementation
                // of glyph drawing, exercised from both renderers.
                super::tiled::draw_text_full_frame(canvas, text, layer, context.options)?;
            }
            LayerContent::Group { children, isolated } => {
                if children.is_empty() {
                    continue;
                }
                if *isolated {
                    let mut group =
                        FloatImage::blank(context.width, context.height, FloatRgba::TRANSPARENT)?;
                    composite_onto(&mut group, children, context, depth + 1)?;
                    draw(canvas, &group, layer, context)?;
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
    }
    high_precision::check_cancel(context.options.cancel)
}

fn channels(p: FloatRgba) -> [f32; 3] {
    [p.red, p.green, p.blue]
}

/// Extended linear arithmetic is defined for unbounded modes. SDR-only modes
/// explicitly bound their blend operands (not the whole image) to [0,1].
/// W3C component modes are evaluated in encoded float sRGB, never linear HSL.
pub fn blend_linear(mode: BlendMode, b: [f32; 3], s: [f32; 3]) -> [f32; 3] {
    if !mode.is_separable() {
        return mode
            .blend(b.map(srgb_encode), s.map(srgb_encode))
            .map(srgb_decode);
    }
    match mode {
        BlendMode::Normal => s,
        BlendMode::Multiply => std::array::from_fn(|i| b[i] * s[i]),
        BlendMode::Screen => std::array::from_fn(|i| b[i] + s[i] - b[i] * s[i]),
        BlendMode::Darken => std::array::from_fn(|i| b[i].min(s[i])),
        BlendMode::Lighten => std::array::from_fn(|i| b[i].max(s[i])),
        BlendMode::Difference => std::array::from_fn(|i| (b[i] - s[i]).abs()),
        BlendMode::Exclusion => std::array::from_fn(|i| b[i] + s[i] - 2.0 * b[i] * s[i]),
        _ => mode.blend(b, s),
    }
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

fn draw(
    canvas: &mut FloatImage,
    source: &FloatImage,
    layer: &Layer,
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
            p.alpha *= layer.opacity
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
            canvas.pixels_mut()[i] = source_over(canvas.pixels()[i], p, layer.blend_mode);
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
