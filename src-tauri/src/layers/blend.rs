use serde::{Deserialize, Serialize};

/// Separable and non-separable blend functions.
///
/// Every mode implements the blend function `B(Cb, Cs)` from the W3C compositing
/// model on straight (unassociated) sRGB-encoded channel values in `0.0..=1.0`.
/// Compositing happens in the encoded space PhotoForge already stores, not in a
/// linear-light space; see `docs/compositing.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    SoftLight,
    HardLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
}

impl BlendMode {
    pub const ALL: [Self; 16] = [
        Self::Normal,
        Self::Multiply,
        Self::Screen,
        Self::Overlay,
        Self::Darken,
        Self::Lighten,
        Self::ColorDodge,
        Self::ColorBurn,
        Self::SoftLight,
        Self::HardLight,
        Self::Difference,
        Self::Exclusion,
        Self::Hue,
        Self::Saturation,
        Self::Color,
        Self::Luminosity,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Multiply => "multiply",
            Self::Screen => "screen",
            Self::Overlay => "overlay",
            Self::Darken => "darken",
            Self::Lighten => "lighten",
            Self::ColorDodge => "color_dodge",
            Self::ColorBurn => "color_burn",
            Self::SoftLight => "soft_light",
            Self::HardLight => "hard_light",
            Self::Difference => "difference",
            Self::Exclusion => "exclusion",
            Self::Hue => "hue",
            Self::Saturation => "saturation",
            Self::Color => "color",
            Self::Luminosity => "luminosity",
        }
    }

    /// Non-separable modes mix all three channels together and cannot be
    /// evaluated one channel at a time.
    pub const fn is_separable(self) -> bool {
        !matches!(
            self,
            Self::Hue | Self::Saturation | Self::Color | Self::Luminosity
        )
    }

    /// Applies `B(Cb, Cs)` to a backdrop and source triple.
    ///
    /// Inputs are clamped to `0.0..=1.0` first, so a non-finite or out-of-range
    /// channel can never produce NaN, infinity, or an out-of-gamut result.
    pub fn blend(self, backdrop: [f32; 3], source: [f32; 3]) -> [f32; 3] {
        let backdrop = sanitize(backdrop);
        let source = sanitize(source);
        let blended = if self.is_separable() {
            [
                self.blend_channel(backdrop[0], source[0]),
                self.blend_channel(backdrop[1], source[1]),
                self.blend_channel(backdrop[2], source[2]),
            ]
        } else {
            self.blend_non_separable(backdrop, source)
        };
        sanitize(blended)
    }

    fn blend_channel(self, backdrop: f32, source: f32) -> f32 {
        match self {
            Self::Normal => source,
            Self::Multiply => backdrop * source,
            Self::Screen => screen(backdrop, source),
            Self::Overlay => hard_light(source, backdrop),
            Self::Darken => backdrop.min(source),
            Self::Lighten => backdrop.max(source),
            Self::ColorDodge => color_dodge(backdrop, source),
            Self::ColorBurn => color_burn(backdrop, source),
            Self::SoftLight => soft_light(backdrop, source),
            Self::HardLight => hard_light(backdrop, source),
            Self::Difference => (backdrop - source).abs(),
            Self::Exclusion => backdrop + source - 2.0 * backdrop * source,
            Self::Hue | Self::Saturation | Self::Color | Self::Luminosity => source,
        }
    }

    fn blend_non_separable(self, backdrop: [f32; 3], source: [f32; 3]) -> [f32; 3] {
        match self {
            Self::Hue => set_luminosity(
                set_saturation(source, saturation(backdrop)),
                luminosity(backdrop),
            ),
            Self::Saturation => set_luminosity(
                set_saturation(backdrop, saturation(source)),
                luminosity(backdrop),
            ),
            Self::Color => set_luminosity(source, luminosity(backdrop)),
            Self::Luminosity => set_luminosity(backdrop, luminosity(source)),
            _ => source,
        }
    }
}

fn sanitize(color: [f32; 3]) -> [f32; 3] {
    [
        clamp_unit(color[0]),
        clamp_unit(color[1]),
        clamp_unit(color[2]),
    ]
}

pub(crate) fn clamp_unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn screen(backdrop: f32, source: f32) -> f32 {
    backdrop + source - backdrop * source
}

fn hard_light(backdrop: f32, source: f32) -> f32 {
    if source <= 0.5 {
        backdrop * (2.0 * source)
    } else {
        screen(backdrop, 2.0 * source - 1.0)
    }
}

fn color_dodge(backdrop: f32, source: f32) -> f32 {
    if backdrop <= 0.0 {
        0.0
    } else if source >= 1.0 {
        1.0
    } else {
        (backdrop / (1.0 - source)).min(1.0)
    }
}

fn color_burn(backdrop: f32, source: f32) -> f32 {
    if backdrop >= 1.0 {
        1.0
    } else if source <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - backdrop) / source).min(1.0)
    }
}

fn soft_light(backdrop: f32, source: f32) -> f32 {
    if source <= 0.5 {
        backdrop - (1.0 - 2.0 * source) * backdrop * (1.0 - backdrop)
    } else {
        let d = if backdrop <= 0.25 {
            ((16.0 * backdrop - 12.0) * backdrop + 4.0) * backdrop
        } else {
            backdrop.sqrt()
        };
        backdrop + (2.0 * source - 1.0) * (d - backdrop)
    }
}

fn luminosity(color: [f32; 3]) -> f32 {
    0.3 * color[0] + 0.59 * color[1] + 0.11 * color[2]
}

fn saturation(color: [f32; 3]) -> f32 {
    let maximum = color[0].max(color[1]).max(color[2]);
    let minimum = color[0].min(color[1]).min(color[2]);
    maximum - minimum
}

fn clip_color(color: [f32; 3]) -> [f32; 3] {
    let lum = luminosity(color);
    let minimum = color[0].min(color[1]).min(color[2]);
    let maximum = color[0].max(color[1]).max(color[2]);
    let mut result = color;
    if minimum < 0.0 {
        let denominator = lum - minimum;
        if denominator.abs() > f32::EPSILON {
            for channel in &mut result {
                *channel = lum + (*channel - lum) * lum / denominator;
            }
        } else {
            result = [lum; 3];
        }
    }
    if maximum > 1.0 {
        let denominator = maximum - lum;
        if denominator.abs() > f32::EPSILON {
            for channel in &mut result {
                *channel = lum + (*channel - lum) * (1.0 - lum) / denominator;
            }
        } else {
            result = [lum; 3];
        }
    }
    result
}

fn set_luminosity(color: [f32; 3], target: f32) -> [f32; 3] {
    let delta = target - luminosity(color);
    clip_color([color[0] + delta, color[1] + delta, color[2] + delta])
}

fn set_saturation(color: [f32; 3], target: f32) -> [f32; 3] {
    let mut indices = [0_usize, 1, 2];
    indices.sort_by(|left, right| {
        color[*left]
            .partial_cmp(&color[*right])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let (minimum, middle, maximum) = (indices[0], indices[1], indices[2]);
    let mut result = [0.0_f32; 3];
    if color[maximum] > color[minimum] {
        result[middle] =
            (color[middle] - color[minimum]) * target / (color[maximum] - color[minimum]);
        result[maximum] = target;
    }
    result[minimum] = 0.0;
    result
}

/// Composites a source pixel over a backdrop pixel with straight (unassociated)
/// alpha, following the W3C `source-over` formula extended by a blend function:
///
/// ```text
/// ao = as + ab * (1 - as)
/// Co = ((1 - ab) * as * Cs + ab * as * B(Cb, Cs) + (1 - as) * ab * Cb) / ao
/// ```
///
/// The premultiplied numerator is divided back out so the result stays straight
/// alpha. A fully transparent result returns transparent black rather than a
/// division by zero, which is what keeps transparent edges free of halos.
pub fn composite_pixel(backdrop: [f32; 4], source: [f32; 4], mode: BlendMode) -> [f32; 4] {
    let backdrop_alpha = clamp_unit(backdrop[3]);
    let source_alpha = clamp_unit(source[3]);
    let backdrop_color = sanitize([backdrop[0], backdrop[1], backdrop[2]]);
    let source_color = sanitize([source[0], source[1], source[2]]);

    let output_alpha = source_alpha + backdrop_alpha * (1.0 - source_alpha);
    if output_alpha <= 0.0 {
        return [0.0, 0.0, 0.0, 0.0];
    }

    let blended = mode.blend(backdrop_color, source_color);
    let mut output = [0.0_f32; 4];
    for channel in 0..3 {
        let premultiplied = (1.0 - backdrop_alpha) * source_alpha * source_color[channel]
            + backdrop_alpha * source_alpha * blended[channel]
            + (1.0 - source_alpha) * backdrop_alpha * backdrop_color[channel];
        output[channel] = clamp_unit(premultiplied / output_alpha);
    }
    output[3] = output_alpha;
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() < 1e-5
    }

    fn assert_color(actual: [f32; 3], expected: [f32; 3]) {
        for channel in 0..3 {
            assert!(
                close(actual[channel], expected[channel]),
                "channel {channel}: {actual:?} != {expected:?}"
            );
        }
    }

    #[test]
    fn separable_modes_match_published_formulas() {
        let backdrop = [0.2, 0.5, 0.8];
        let source = [0.6, 0.4, 0.1];
        assert_color(BlendMode::Normal.blend(backdrop, source), source);
        assert_color(
            BlendMode::Multiply.blend(backdrop, source),
            [0.12, 0.2, 0.08],
        );
        assert_color(BlendMode::Screen.blend(backdrop, source), [0.68, 0.7, 0.82]);
        assert_color(BlendMode::Darken.blend(backdrop, source), [0.2, 0.4, 0.1]);
        assert_color(BlendMode::Lighten.blend(backdrop, source), [0.6, 0.5, 0.8]);
        assert_color(
            BlendMode::Difference.blend(backdrop, source),
            [0.4, 0.1, 0.7],
        );
        assert_color(
            BlendMode::Exclusion.blend(backdrop, source),
            [0.56, 0.5, 0.74],
        );
    }

    #[test]
    fn overlay_and_hard_light_are_transposes() {
        let backdrop = [0.25, 0.5, 0.75];
        let source = [0.6, 0.3, 0.9];
        let overlay = BlendMode::Overlay.blend(backdrop, source);
        let hard_light = BlendMode::HardLight.blend(source, backdrop);
        assert_color(overlay, hard_light);
        // Overlay(0.25, 0.6) uses the multiply branch on the backdrop: 2*0.25*0.6.
        assert!(close(overlay[0], 0.3));
        // Overlay(0.75, 0.9) uses the screen branch: 1 - 2*(1-0.75)*(1-0.9).
        assert!(close(overlay[2], 0.95));
    }

    #[test]
    fn dodge_and_burn_handle_their_singular_endpoints() {
        assert_color(
            BlendMode::ColorDodge.blend([0.0, 0.5, 0.5], [0.5, 1.0, 0.0]),
            [0.0, 1.0, 0.5],
        );
        assert!(close(
            BlendMode::ColorDodge.blend([0.25, 0.0, 0.0], [0.5, 0.0, 0.0])[0],
            0.5
        ));
        assert_color(
            BlendMode::ColorBurn.blend([1.0, 0.5, 0.5], [0.5, 0.0, 1.0]),
            [1.0, 0.0, 0.5],
        );
        assert!(close(
            BlendMode::ColorBurn.blend([0.5, 0.0, 0.0], [0.5, 0.0, 0.0])[0],
            0.0
        ));
    }

    #[test]
    fn soft_light_uses_both_documented_branches() {
        // Source below 0.5 darkens: Cb - (1 - 2*Cs) * Cb * (1 - Cb).
        assert!(close(
            BlendMode::SoftLight.blend([0.5, 0.0, 0.0], [0.25, 0.0, 0.0])[0],
            0.375
        ));
        // Source above 0.5 with a bright backdrop uses the sqrt branch.
        let bright = BlendMode::SoftLight.blend([0.64, 0.0, 0.0], [1.0, 0.0, 0.0])[0];
        assert!(close(bright, 0.64 + (0.8 - 0.64)));
        // Source above 0.5 with a dark backdrop uses the polynomial branch.
        let dark = BlendMode::SoftLight.blend([0.16, 0.0, 0.0], [1.0, 0.0, 0.0])[0];
        let d = ((16.0 * 0.16_f32 - 12.0) * 0.16 + 4.0) * 0.16;
        assert!(close(dark, 0.16 + (d - 0.16)));
    }

    #[test]
    fn neutral_operands_are_identities() {
        let backdrop = [0.3, 0.6, 0.9];
        assert_color(BlendMode::Multiply.blend(backdrop, [1.0; 3]), backdrop);
        assert_color(BlendMode::Screen.blend(backdrop, [0.0; 3]), backdrop);
        assert_color(BlendMode::Overlay.blend(backdrop, [0.5; 3]), backdrop);
        assert_color(BlendMode::SoftLight.blend(backdrop, [0.5; 3]), backdrop);
        assert_color(BlendMode::HardLight.blend(backdrop, [0.5; 3]), backdrop);
        assert_color(BlendMode::Difference.blend(backdrop, [0.0; 3]), backdrop);
        assert_color(BlendMode::Exclusion.blend(backdrop, [0.0; 3]), backdrop);
    }

    #[test]
    fn non_separable_modes_move_the_expected_component() {
        let backdrop = [0.2, 0.4, 0.6];
        let source = [0.9, 0.1, 0.3];

        let luminosity_result = BlendMode::Luminosity.blend(backdrop, source);
        assert!(close(luminosity(luminosity_result), luminosity(source)));

        let color_result = BlendMode::Color.blend(backdrop, source);
        assert!(close(luminosity(color_result), luminosity(backdrop)));

        let saturation_result = BlendMode::Saturation.blend(backdrop, source);
        assert!(close(luminosity(saturation_result), luminosity(backdrop)));
        assert!(close(saturation(saturation_result), saturation(source)));

        let hue_result = BlendMode::Hue.blend(backdrop, source);
        assert!(close(luminosity(hue_result), luminosity(backdrop)));
        assert!(close(saturation(hue_result), saturation(backdrop)));
    }

    #[test]
    fn gray_sources_leave_saturation_modes_neutral() {
        let backdrop = [0.2, 0.5, 0.8];
        // A grey source carries no hue and no saturation, so Color keeps the
        // backdrop's luminosity and returns exactly that grey — not the
        // source's own level.
        let expected = 0.3 * 0.2 + 0.59 * 0.5 + 0.11 * 0.8;
        assert_color(BlendMode::Color.blend(backdrop, [0.5; 3]), [expected; 3]);
        let desaturated = BlendMode::Saturation.blend(backdrop, [0.4; 3]);
        assert!(close(saturation(desaturated), 0.0));
    }

    #[test]
    fn every_mode_stays_inside_the_unit_range_for_extreme_inputs() {
        let samples = [0.0_f32, 0.25, 0.5, 0.75, 1.0];
        for mode in BlendMode::ALL {
            for &br in &samples {
                for &bg in &samples {
                    for &sr in &samples {
                        for &sg in &samples {
                            let result = mode.blend([br, bg, 0.5], [sr, sg, 0.5]);
                            for channel in result {
                                assert!(
                                    channel.is_finite() && (0.0..=1.0).contains(&channel),
                                    "{} produced {channel}",
                                    mode.id()
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn non_finite_and_out_of_range_inputs_are_clamped_rather_than_propagated() {
        for mode in BlendMode::ALL {
            let result = mode.blend(
                [f32::NAN, f32::INFINITY, -5.0],
                [2.0, f32::NEG_INFINITY, 0.5],
            );
            for channel in result {
                assert!(channel.is_finite() && (0.0..=1.0).contains(&channel));
            }
        }
        let composited = composite_pixel(
            [f32::NAN, 0.5, 0.5, f32::INFINITY],
            [0.5, f32::NAN, 0.5, -1.0],
            BlendMode::Overlay,
        );
        for channel in composited {
            assert!(channel.is_finite() && (0.0..=1.0).contains(&channel));
        }
    }

    #[test]
    fn opaque_source_replaces_the_backdrop_in_normal_mode() {
        let result = composite_pixel(
            [0.1, 0.2, 0.3, 1.0],
            [0.7, 0.8, 0.9, 1.0],
            BlendMode::Normal,
        );
        assert_color([result[0], result[1], result[2]], [0.7, 0.8, 0.9]);
        assert!(close(result[3], 1.0));
    }

    #[test]
    fn translucent_source_over_opaque_backdrop_interpolates_linearly() {
        let result = composite_pixel(
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 0.5],
            BlendMode::Normal,
        );
        assert_color([result[0], result[1], result[2]], [0.5, 0.5, 0.5]);
        assert!(close(result[3], 1.0));
    }

    #[test]
    fn transparent_backdrop_keeps_the_source_color_unmodified_by_blending() {
        for mode in BlendMode::ALL {
            let result = composite_pixel([0.9, 0.1, 0.4, 0.0], [0.2, 0.7, 0.3, 1.0], mode);
            assert_color([result[0], result[1], result[2]], [0.2, 0.7, 0.3]);
            assert!(close(result[3], 1.0));
        }
    }

    #[test]
    fn transparent_over_transparent_stays_fully_transparent_black() {
        let result = composite_pixel(
            [0.4, 0.4, 0.4, 0.0],
            [0.9, 0.9, 0.9, 0.0],
            BlendMode::Multiply,
        );
        assert_eq!(result, [0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn translucent_over_translucent_matches_the_porter_duff_alpha() {
        let result = composite_pixel(
            [1.0, 0.0, 0.0, 0.5],
            [0.0, 0.0, 1.0, 0.5],
            BlendMode::Normal,
        );
        // ao = 0.5 + 0.5 * 0.5 = 0.75; the source contributes 0.5/0.75 of the result.
        assert!(close(result[3], 0.75));
        assert!(close(result[2], 2.0 / 3.0));
        assert!(close(result[0], 1.0 / 3.0));
    }

    #[test]
    fn a_fully_transparent_source_never_changes_the_backdrop() {
        for mode in BlendMode::ALL {
            let backdrop = [0.3, 0.6, 0.2, 0.8];
            let result = composite_pixel(backdrop, [0.9, 0.1, 0.5, 0.0], mode);
            assert_color([result[0], result[1], result[2]], [0.3, 0.6, 0.2]);
            assert!(close(result[3], 0.8));
        }
    }

    #[test]
    fn mode_identifiers_are_unique_and_round_trip_through_json() {
        let mut identifiers: Vec<&str> = BlendMode::ALL.iter().map(|mode| mode.id()).collect();
        identifiers.sort_unstable();
        let count = identifiers.len();
        identifiers.dedup();
        assert_eq!(identifiers.len(), count);
        for mode in BlendMode::ALL {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(json, format!("\"{}\"", mode.id()));
            assert_eq!(serde_json::from_str::<BlendMode>(&json).unwrap(), mode);
        }
    }

    #[test]
    fn unknown_blend_modes_are_rejected_by_deserialization() {
        assert!(serde_json::from_str::<BlendMode>("\"dissolve\"").is_err());
    }
}
