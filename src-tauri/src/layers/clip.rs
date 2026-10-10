//! Clipping: a layer that paints only where the layer beneath it does.
//!
//! A clipped layer takes its shape from its *base* — the nearest unclipped layer
//! below it among its siblings. The base and every layer clipped to it are
//! composited as one unit:
//!
//! 1. the base is drawn, through its own transform and mask, into a buffer of its
//!    own, at full opacity and in Normal mode;
//! 2. each clipped layer is drawn into that buffer through its own transform,
//!    mask, opacity and blend mode, **atop** what is already there — it changes
//!    colour where the base has coverage and leaves the buffer's alpha alone, so
//!    nothing a clipped layer paints can extend past the base, and a half
//!    transparent base stays half transparent;
//! 3. the buffer is composited onto the layers below with the *base's* opacity and
//!    blend mode.
//!
//! This is Photoshop's "blend clipped layers as group" behaviour, which is the
//! default and what a file that does not say otherwise means. The base's blend
//! mode therefore applies to the whole group, and an adjustment layer clipped to
//! a layer changes only that layer and whatever is clipped beneath it.
//!
//! Three renderers implement this — the 8-bit one, the float full-frame one and
//! the tiled one — and each does it in its own terms. What they share is this
//! file: which layers belong together, and how a layer is to be painted.

use super::{BlendMode, Layer, LayerContent};

/// How a layer's pixels combine with what is already beneath them.
///
/// Taken from the layer for an ordinary layer. A clipping group paints its base
/// and its clipped layers with values that differ from the layer's own, which is
/// why the renderers' drawing routines take this instead of reading the layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Paint {
    pub opacity: f32,
    pub blend: BlendMode,
    /// Paint atop the destination: change its colour where it has coverage and
    /// keep its alpha, instead of adding coverage of the layer's own.
    pub atop: bool,
    /// The layer is being painted into a clipping group's buffer. A pass-through
    /// group has no backdrop of its own there to pass through to, so it paints as
    /// an isolated one.
    pub in_clip_group: bool,
}

impl Paint {
    pub fn of(layer: &Layer) -> Self {
        Self {
            opacity: layer.opacity,
            blend: layer.blend_mode,
            atop: false,
            in_clip_group: false,
        }
    }

    /// The base of a clipping group, drawn into the group's own buffer. Its
    /// opacity and blend mode belong to the group as a whole, and are applied
    /// when the buffer is composited.
    pub fn base() -> Self {
        Self {
            opacity: 1.0,
            blend: BlendMode::Normal,
            atop: false,
            in_clip_group: true,
        }
    }

    /// A layer clipped to the base: its own opacity and blend mode, atop.
    pub fn clipped(layer: &Layer) -> Self {
        Self {
            opacity: layer.opacity,
            blend: layer.blend_mode,
            atop: true,
            in_clip_group: true,
        }
    }
}

/// Whether the layer contributes to a render at all.
pub(super) fn paints(layer: &Layer) -> bool {
    layer.visible && layer.opacity > 0.0
}

/// Whether layers clipped to this one can take their shape from it.
///
/// An adjustment layer has no coverage of its own — it changes the colour of
/// whatever is beneath it — so there is nothing for a layer clipped to it to
/// take a shape from. Such layers paint as ordinary ones.
pub(super) fn can_hold_clipped(layer: &Layer) -> bool {
    !matches!(layer.content, LayerContent::Adjustment { .. })
}

/// One base and the layers clipped to it, bottom to top.
pub(super) struct ClipGroup<'a> {
    pub base: &'a Layer,
    pub clipped: &'a [Layer],
}

impl ClipGroup<'_> {
    /// Whether the unit must be composited through a buffer of its own. A hidden
    /// base hides everything clipped to it, and clipped layers that all paint
    /// nothing leave the base to be drawn exactly as it would have been alone.
    pub fn needs_buffer(&self) -> bool {
        paints(self.base) && can_hold_clipped(self.base) && self.clipped.iter().any(paints)
    }
}

/// Splits one stack of siblings into clipping groups.
///
/// The first layer of a stack is always a base, whatever it says about itself:
/// there is nothing beneath it to clip to. A layer flagged clipped after that
/// joins the group of the nearest unclipped layer below it, hidden ones included —
/// hiding a clipped layer must not re-attach the layers above it to some other
/// base.
pub(super) fn clip_groups(layers: &[Layer]) -> impl Iterator<Item = ClipGroup<'_>> {
    let mut rest = layers;
    std::iter::from_fn(move || {
        let (base, above) = rest.split_first()?;
        let run = above.iter().take_while(|layer| layer.clipped).count();
        let (clipped, remaining) = above.split_at(run);
        rest = remaining;
        Some(ClipGroup { base, clipped })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::test_pixel_layer;

    fn layer(id: &str, clipped: bool) -> Layer {
        let mut layer = test_pixel_layer(id, "p", 2, 2);
        layer.clipped = clipped;
        layer
    }

    fn shape(layers: &[Layer]) -> Vec<(String, Vec<String>)> {
        clip_groups(layers)
            .map(|group| {
                (
                    group.base.id.clone(),
                    group.clipped.iter().map(|layer| layer.id.clone()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn unclipped_layers_are_each_their_own_group() {
        let layers = [layer("a", false), layer("b", false)];
        assert_eq!(
            shape(&layers),
            vec![("a".into(), vec![]), ("b".into(), vec![])]
        );
    }

    #[test]
    fn clipped_layers_join_the_nearest_unclipped_one_below() {
        let layers = [
            layer("a", false),
            layer("b", false),
            layer("c", true),
            layer("d", true),
            layer("e", false),
        ];
        assert_eq!(
            shape(&layers),
            vec![
                ("a".into(), vec![]),
                ("b".into(), vec!["c".into(), "d".into()]),
                ("e".into(), vec![]),
            ]
        );
    }

    #[test]
    fn the_first_layer_is_a_base_even_when_it_is_flagged_clipped() {
        let layers = [layer("a", true), layer("b", true)];
        assert_eq!(shape(&layers), vec![("a".into(), vec!["b".into()])]);
    }

    #[test]
    fn a_hidden_clipped_layer_does_not_reattach_the_ones_above_it() {
        let mut hidden = layer("c", true);
        hidden.visible = false;
        let layers = [layer("a", false), hidden, layer("d", true)];
        assert_eq!(
            shape(&layers),
            vec![("a".into(), vec!["c".into(), "d".into()])]
        );
    }

    #[test]
    fn a_group_needs_a_buffer_only_when_something_visible_is_clipped_to_a_visible_base() {
        let base = layer("a", false);
        let mut hidden = layer("b", true);
        hidden.visible = false;
        let lone = [base.clone(), hidden.clone()];
        assert!(!clip_groups(&lone).next().unwrap().needs_buffer());
        let with_visible = [base.clone(), hidden, layer("c", true)];
        assert!(clip_groups(&with_visible).next().unwrap().needs_buffer());
        let mut invisible_base = base;
        invisible_base.visible = false;
        let under_hidden = [invisible_base, layer("c", true)];
        assert!(!clip_groups(&under_hidden).next().unwrap().needs_buffer());
    }

    #[test]
    fn an_adjustment_layer_cannot_be_a_base() {
        let mut base = layer("a", false);
        base.content = LayerContent::Adjustment {
            operation: Box::new(crate::domain::EditOperation::RawDevelopment {
                parameters: crate::color::DevelopmentParameters::default(),
            }),
        };
        let layers = [base, layer("b", true)];
        assert!(!clip_groups(&layers).next().unwrap().needs_buffer());
    }

    mod rendering {
        use super::*;
        use crate::color::{FloatImage, FloatRgba};
        use crate::domain::EditOperation;
        use crate::layers::composite::testing::{solid, MapSource};
        use crate::layers::model::fixtures::{
            adjustment_layer, document, group_layer, pass_through_group, pixel_layer,
        };
        use crate::layers::{
            render_document, render_document_float, LayerDocument, LayerPixelStore, RenderOptions,
        };
        use image::{Rgba, RgbaImage};

        const RED: [u8; 4] = [255, 0, 0, 255];
        const GREEN: [u8; 4] = [0, 255, 0, 255];
        const BLUE: [u8; 4] = [0, 0, 255, 255];
        const CLEAR: [u8; 4] = [0, 0, 0, 0];

        fn canvas(layers: Vec<Layer>) -> LayerDocument {
            let mut document = document(layers);
            document.canvas_width = 4;
            document.canvas_height = 2;
            document
        }

        fn clipped(mut layer: Layer) -> Layer {
            layer.clipped = true;
            layer
        }

        /// A 4 x 2 image whose left half is `colour` and whose right half is
        /// fully transparent.
        fn left_half(colour: [u8; 4]) -> RgbaImage {
            RgbaImage::from_fn(4, 2, |x, _| if x < 2 { Rgba(colour) } else { Rgba(CLEAR) })
        }

        fn render(document: &LayerDocument, source: &MapSource) -> RgbaImage {
            render_document(document, source, RenderOptions::default()).unwrap()
        }

        fn source(images: &[(&str, RgbaImage)]) -> MapSource {
            let mut source = MapSource::default();
            for (id, image) in images {
                source.insert(&format!("px{id}"), image.clone());
            }
            source
        }

        #[test]
        fn a_clipped_layer_paints_only_where_its_base_has_coverage() {
            let document = canvas(vec![
                pixel_layer("ground", 4, 2),
                pixel_layer("base", 4, 2),
                clipped(pixel_layer("ink", 4, 2)),
            ]);
            let source = source(&[
                ("ground", solid(4, 2, GREEN)),
                ("base", left_half(RED)),
                ("ink", solid(4, 2, BLUE)),
            ]);
            let image = render(&document, &source);
            assert_eq!(image.get_pixel(0, 0).0, BLUE);
            assert_eq!(image.get_pixel(1, 1).0, BLUE);
            // Beyond the base the ink does not reach, and the ground shows.
            assert_eq!(image.get_pixel(2, 0).0, GREEN);
            assert_eq!(image.get_pixel(3, 1).0, GREEN);
        }

        #[test]
        fn without_the_flag_the_same_layer_paints_everywhere() {
            let document = canvas(vec![
                pixel_layer("ground", 4, 2),
                pixel_layer("base", 4, 2),
                pixel_layer("ink", 4, 2),
            ]);
            let source = source(&[
                ("ground", solid(4, 2, GREEN)),
                ("base", left_half(RED)),
                ("ink", solid(4, 2, BLUE)),
            ]);
            assert_eq!(render(&document, &source).get_pixel(3, 0).0, BLUE);
        }

        #[test]
        fn a_half_transparent_base_stays_half_transparent() {
            // The layer above is fully opaque. Taking its coverage from the base
            // means the pair is still half transparent, not three quarters.
            let document = canvas(vec![
                pixel_layer("base", 4, 2),
                clipped(pixel_layer("ink", 4, 2)),
            ]);
            let source = source(&[
                ("base", solid(4, 2, [255, 0, 0, 128])),
                ("ink", solid(4, 2, BLUE)),
            ]);
            let image = render(&document, &source);
            assert_eq!(image.get_pixel(1, 1).0, [0, 0, 255, 128]);
        }

        #[test]
        fn the_bases_opacity_and_blend_mode_apply_to_the_whole_group() {
            let mut base = pixel_layer("base", 4, 2);
            base.blend_mode = BlendMode::Multiply;
            base.opacity = 0.5;
            let document = canvas(vec![
                pixel_layer("ground", 4, 2),
                base,
                clipped(pixel_layer("ink", 4, 2)),
            ]);
            let source = source(&[
                ("ground", solid(4, 2, [255, 255, 255, 255])),
                ("base", solid(4, 2, RED)),
                ("ink", solid(4, 2, BLUE)),
            ]);
            // The ink is blue, so the group is blue; multiplied onto white at
            // half opacity that is the midpoint of white and blue. Had the
            // group's blend mode and opacity applied to the base alone, the
            // ink would paint over it untouched and the pixel would be blue.
            assert_eq!(
                render(&document, &source).get_pixel(0, 0).0,
                [128, 128, 255, 255]
            );
        }

        #[test]
        fn a_clipped_layer_keeps_its_own_opacity() {
            let mut ink = clipped(pixel_layer("ink", 4, 2));
            ink.opacity = 0.5;
            let document = canvas(vec![pixel_layer("base", 4, 2), ink]);
            let source = source(&[("base", solid(4, 2, RED)), ("ink", solid(4, 2, BLUE))]);
            assert_eq!(
                render(&document, &source).get_pixel(0, 0).0,
                [128, 0, 128, 255]
            );
        }

        #[test]
        fn a_clipped_adjustment_changes_only_what_it_is_clipped_to() {
            let document = canvas(vec![
                pixel_layer("ground", 4, 2),
                pixel_layer("base", 4, 2),
                clipped(adjustment_layer("grey", EditOperation::Grayscale)),
            ]);
            let source = source(&[
                ("ground", solid(4, 2, [200, 50, 50, 255])),
                ("base", left_half([50, 200, 50, 255])),
            ]);
            let image = render(&document, &source);
            let on_base = image.get_pixel(0, 0).0;
            assert_eq!(on_base[0], on_base[1]);
            assert_eq!(on_base[1], on_base[2]);
            // The ground beside the base is untouched. An unclipped adjustment
            // would have turned it grey as well.
            assert_eq!(image.get_pixel(3, 0).0, [200, 50, 50, 255]);

            let mut unclipped = document.clone();
            unclipped.layers[2].clipped = false;
            let beside = render(&unclipped, &source).get_pixel(3, 0).0;
            assert_eq!(beside[0], beside[1]);
        }

        #[test]
        fn a_hidden_base_hides_what_is_clipped_to_it() {
            let mut base = pixel_layer("base", 4, 2);
            base.visible = false;
            let document = canvas(vec![
                pixel_layer("ground", 4, 2),
                base,
                clipped(pixel_layer("ink", 4, 2)),
            ]);
            let source = source(&[
                ("ground", solid(4, 2, GREEN)),
                ("base", solid(4, 2, RED)),
                ("ink", solid(4, 2, BLUE)),
            ]);
            assert_eq!(render(&document, &source).get_pixel(0, 0).0, GREEN);
        }

        #[test]
        fn a_clipped_layer_with_nothing_beneath_it_paints_as_an_ordinary_layer() {
            let document = canvas(vec![clipped(pixel_layer("ink", 4, 2))]);
            let source = source(&[("ink", solid(4, 2, BLUE))]);
            assert_eq!(render(&document, &source).get_pixel(2, 1).0, BLUE);
        }

        #[test]
        fn an_adjustment_cannot_be_a_base_so_what_is_clipped_to_it_paints_ordinarily() {
            let document = canvas(vec![
                pixel_layer("ground", 4, 2),
                adjustment_layer("grey", EditOperation::Grayscale),
                clipped(pixel_layer("ink", 4, 2)),
            ]);
            let source = source(&[("ground", solid(4, 2, GREEN)), ("ink", solid(4, 2, BLUE))]);
            assert_eq!(render(&document, &source).get_pixel(1, 1).0, BLUE);
        }

        #[test]
        fn a_group_can_be_a_base_and_can_be_clipped() {
            let document = canvas(vec![
                pixel_layer("ground", 4, 2),
                group_layer("cut-out", vec![pixel_layer("shape", 4, 2)]),
                clipped(group_layer("tint", vec![pixel_layer("wash", 4, 2)])),
            ]);
            let source = source(&[
                ("ground", solid(4, 2, GREEN)),
                ("shape", left_half(RED)),
                ("wash", solid(4, 2, BLUE)),
            ]);
            let image = render(&document, &source);
            assert_eq!(image.get_pixel(0, 0).0, BLUE);
            assert_eq!(image.get_pixel(3, 0).0, GREEN);
        }

        #[test]
        fn a_pass_through_group_that_is_a_base_paints_as_an_isolated_one() {
            // Pass-through means "see the backdrop". Inside a clipping group
            // there is no backdrop to see: the group is its own buffer.
            let document = canvas(vec![
                pixel_layer("ground", 4, 2),
                pass_through_group(
                    "through",
                    vec![adjustment_layer("grey", EditOperation::Grayscale)],
                ),
                clipped(pixel_layer("ink", 4, 2)),
            ]);
            let source = source(&[
                ("ground", solid(4, 2, [200, 50, 50, 255])),
                ("ink", solid(4, 2, BLUE)),
            ]);
            // Nothing in the base has coverage, so nothing of the ink shows, and
            // the grey adjustment had no backdrop to act on.
            assert_eq!(
                render(&document, &source).get_pixel(0, 0).0,
                [200, 50, 50, 255]
            );
        }

        #[test]
        fn clipping_is_per_stack() {
            // The clipped layer is the first of its group, so it has no base
            // there, whatever sits beneath the group.
            let document = canvas(vec![
                pixel_layer("ground", 4, 2),
                group_layer("inside", vec![clipped(pixel_layer("ink", 4, 2))]),
            ]);
            let source = source(&[("ground", left_half(RED)), ("ink", solid(4, 2, BLUE))]);
            assert_eq!(render(&document, &source).get_pixel(3, 0).0, BLUE);
        }

        // The same behaviour in the high-precision renderer, in linear light.

        fn float_document(layers: Vec<Layer>) -> LayerDocument {
            let mut document = canvas(layers);
            document.precision = crate::pixel::DocumentPrecision::LinearSrgbF32;
            document
        }

        fn flat(colour: FloatRgba) -> FloatImage {
            FloatImage::blank(4, 2, colour).unwrap()
        }

        fn float_left_half(colour: FloatRgba) -> FloatImage {
            let pixels = (0..8)
                .map(|index| {
                    if index % 4 < 2 {
                        colour
                    } else {
                        FloatRgba::TRANSPARENT
                    }
                })
                .collect();
            FloatImage::new(4, 2, pixels).unwrap()
        }

        /// Registers each image under the identifier the fixture layers use and
        /// renders the document with the full-frame float renderer.
        fn float_render(document: &LayerDocument, images: Vec<(&str, FloatImage)>) -> FloatImage {
            let mut store = LayerPixelStore::default();
            store.reset(4, 2).unwrap();
            for (id, image) in images {
                store
                    .register_typed_with_id(
                        &format!("px{id}"),
                        crate::pixel::PixelBuffer::LinearRgbaF32(image.into()),
                    )
                    .unwrap();
            }
            let resolved = store
                .resolve(&document.referenced_pixel_ids(), false)
                .unwrap();
            render_document_float(document, &resolved, RenderOptions::default()).unwrap()
        }

        fn close(actual: FloatRgba, expected: FloatRgba) {
            let pairs = [
                (actual.red, expected.red),
                (actual.green, expected.green),
                (actual.blue, expected.blue),
                (actual.alpha, expected.alpha),
            ];
            for (a, e) in pairs {
                assert!((a - e).abs() < 1e-6, "{actual:?} is not {expected:?}");
            }
        }

        #[test]
        fn float_clipped_layers_paint_only_where_the_base_has_coverage() {
            let document = float_document(vec![
                pixel_layer("ground", 4, 2),
                pixel_layer("base", 4, 2),
                clipped(pixel_layer("ink", 4, 2)),
            ]);
            let image = float_render(
                &document,
                vec![
                    ("ground", flat(FloatRgba::new(0.0, 1.0, 0.0, 1.0))),
                    ("base", float_left_half(FloatRgba::new(1.0, 0.0, 0.0, 1.0))),
                    ("ink", flat(FloatRgba::new(0.0, 0.0, 1.0, 1.0))),
                ],
            );
            close(image.get(0, 0).unwrap(), FloatRgba::new(0.0, 0.0, 1.0, 1.0));
            close(image.get(3, 1).unwrap(), FloatRgba::new(0.0, 1.0, 0.0, 1.0));
        }

        #[test]
        fn float_a_half_transparent_base_stays_half_transparent() {
            let document = float_document(vec![
                pixel_layer("base", 4, 2),
                clipped(pixel_layer("ink", 4, 2)),
            ]);
            let image = float_render(
                &document,
                vec![
                    ("base", flat(FloatRgba::new(1.0, 0.0, 0.0, 0.5))),
                    ("ink", flat(FloatRgba::new(0.0, 0.0, 1.0, 1.0))),
                ],
            );
            close(image.get(2, 0).unwrap(), FloatRgba::new(0.0, 0.0, 1.0, 0.5));
        }

        #[test]
        fn float_the_bases_opacity_and_blend_mode_apply_to_the_whole_group() {
            let mut base = pixel_layer("base", 4, 2);
            base.blend_mode = BlendMode::Multiply;
            base.opacity = 0.5;
            let document = float_document(vec![
                pixel_layer("ground", 4, 2),
                base,
                clipped(pixel_layer("ink", 4, 2)),
            ]);
            let image = float_render(
                &document,
                vec![
                    ("ground", flat(FloatRgba::new(1.0, 1.0, 1.0, 1.0))),
                    ("base", flat(FloatRgba::new(1.0, 0.0, 0.0, 1.0))),
                    ("ink", flat(FloatRgba::new(0.0, 0.0, 1.0, 1.0))),
                ],
            );
            close(image.get(0, 0).unwrap(), FloatRgba::new(0.5, 0.5, 1.0, 1.0));
        }

        #[test]
        fn float_a_clipped_adjustment_changes_only_what_it_is_clipped_to() {
            let document = float_document(vec![
                pixel_layer("ground", 4, 2),
                pixel_layer("base", 4, 2),
                clipped(adjustment_layer("grey", EditOperation::Grayscale)),
            ]);
            let image = float_render(
                &document,
                vec![
                    ("ground", flat(FloatRgba::new(0.6, 0.1, 0.1, 1.0))),
                    ("base", float_left_half(FloatRgba::new(0.1, 0.6, 0.1, 1.0))),
                ],
            );
            let on_base = image.get(0, 0).unwrap();
            assert!((on_base.red - on_base.green).abs() < 1e-6);
            assert!((on_base.green - on_base.blue).abs() < 1e-6);
            close(image.get(3, 0).unwrap(), FloatRgba::new(0.6, 0.1, 0.1, 1.0));
        }
    }

    #[test]
    fn the_flag_round_trips_and_is_absent_when_false() {
        let plain = layer("a", false);
        let json = serde_json::to_string(&plain).unwrap();
        assert!(!json.contains("clipped"), "{json}");
        let flagged = layer("b", true);
        let json = serde_json::to_string(&flagged).unwrap();
        assert!(json.contains("\"clipped\":true"), "{json}");
        let back: Layer = serde_json::from_str(&json).unwrap();
        assert!(back.clipped);
        // A project written before the field existed has no key at all.
        let mut value: serde_json::Value = serde_json::to_value(&plain).unwrap();
        value.as_object_mut().unwrap().remove("clipped");
        let old: Layer = serde_json::from_value(value).unwrap();
        assert!(!old.clipped);
    }

    #[test]
    fn paint_for_a_base_is_normal_and_whole_and_for_a_clipped_layer_its_own() {
        let mut layer = layer("a", true);
        layer.opacity = 0.4;
        layer.blend_mode = BlendMode::Multiply;
        assert_eq!(
            Paint::base(),
            Paint {
                opacity: 1.0,
                blend: BlendMode::Normal,
                atop: false,
                in_clip_group: true
            }
        );
        assert_eq!(
            Paint::clipped(&layer),
            Paint {
                opacity: 0.4,
                blend: BlendMode::Multiply,
                atop: true,
                in_clip_group: true
            }
        );
        assert!(!Paint::of(&layer).atop);
    }
}
