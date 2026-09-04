use crate::error::AppError;
use serde::{Deserialize, Serialize};

/// Smallest and largest per-axis scale a layer transform may request. The bound
/// keeps an inverse mapping numerically stable and stops a malformed project
/// from asking the compositor to resample a layer into an unbounded region.
pub const MIN_LAYER_SCALE: f32 = 1.0 / 64.0;
pub const MAX_LAYER_SCALE: f32 = 64.0;
/// Largest absolute translation, in document pixels, a transform may request.
pub const MAX_LAYER_TRANSLATION: f32 = 1_000_000.0;

/// How the compositor samples a transformed layer.
///
/// Bilinear is the default and the only behaviour earlier releases had, so a
/// project written before 0.8.2 loads with it. Nearest neighbour exists for
/// hard-edged artwork, where interpolating across a boundary is exactly the
/// wrong answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerInterpolation {
    #[default]
    Bilinear,
    Nearest,
}

/// A non-destructive per-layer placement.
///
/// The transform maps the layer's own pixel grid into document space. Flip,
/// scale, and rotation are applied around the layer's centre, then the layer is
/// translated. Pixels are never resampled until the compositor renders, so
/// repeated edits do not accumulate resampling loss.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerTransform {
    pub translate_x: f32,
    pub translate_y: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub rotation_degrees: f32,
    pub flip_horizontal: bool,
    pub flip_vertical: bool,
    /// Absent in projects written before 0.8.2, which sampled bilinearly.
    #[serde(default)]
    pub interpolation: LayerInterpolation,
}

impl Default for LayerTransform {
    fn default() -> Self {
        Self {
            translate_x: 0.0,
            translate_y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation_degrees: 0.0,
            flip_horizontal: false,
            flip_vertical: false,
            interpolation: LayerInterpolation::Bilinear,
        }
    }
}

impl LayerTransform {
    pub fn validate(&self) -> Result<(), AppError> {
        let finite = [
            self.translate_x,
            self.translate_y,
            self.scale_x,
            self.scale_y,
            self.rotation_degrees,
        ]
        .iter()
        .all(|value| value.is_finite());
        if !finite {
            return Err(AppError::InvalidLayerTransform(
                "transform values must be finite".into(),
            ));
        }
        if self.translate_x.abs() > MAX_LAYER_TRANSLATION
            || self.translate_y.abs() > MAX_LAYER_TRANSLATION
        {
            return Err(AppError::InvalidLayerTransform(format!(
                "translation must stay within +/-{MAX_LAYER_TRANSLATION} document pixels"
            )));
        }
        let scale_in_range = |value: f32| {
            let magnitude = value.abs();
            (MIN_LAYER_SCALE..=MAX_LAYER_SCALE).contains(&magnitude)
        };
        if !scale_in_range(self.scale_x) || !scale_in_range(self.scale_y) {
            return Err(AppError::InvalidLayerTransform(format!(
                "scale magnitude must stay between {MIN_LAYER_SCALE} and {MAX_LAYER_SCALE}"
            )));
        }
        if self.rotation_degrees.abs() > 360.0 {
            return Err(AppError::InvalidLayerTransform(
                "rotation must stay within +/-360 degrees".into(),
            ));
        }
        Ok(())
    }

    pub fn is_identity(&self) -> bool {
        self.translate_x == 0.0
            && self.translate_y == 0.0
            && self.scale_x == 1.0
            && self.scale_y == 1.0
            && self.rotation_degrees == 0.0
            && !self.flip_horizontal
            && !self.flip_vertical
    }

    /// True when the transform is a whole-pixel move with no resampling, which
    /// lets the compositor copy rows instead of sampling.
    pub fn integer_translation(&self) -> Option<(i64, i64)> {
        if self.scale_x != 1.0
            || self.scale_y != 1.0
            || self.rotation_degrees != 0.0
            || self.flip_horizontal
            || self.flip_vertical
        {
            return None;
        }
        if self.translate_x.fract() != 0.0 || self.translate_y.fract() != 0.0 {
            return None;
        }
        Some((self.translate_x as i64, self.translate_y as i64))
    }

    fn effective_scale(&self) -> (f32, f32) {
        (
            if self.flip_horizontal {
                -self.scale_x
            } else {
                self.scale_x
            },
            if self.flip_vertical {
                -self.scale_y
            } else {
                self.scale_y
            },
        )
    }

    /// Maps a point in layer pixel space to document pixel space.
    pub fn forward(&self, point: (f32, f32), width: u32, height: u32) -> (f32, f32) {
        let (centre_x, centre_y) = (width as f32 / 2.0, height as f32 / 2.0);
        let (scale_x, scale_y) = self.effective_scale();
        let x = (point.0 - centre_x) * scale_x;
        let y = (point.1 - centre_y) * scale_y;
        let radians = self.rotation_degrees.to_radians();
        let (sin, cos) = radians.sin_cos();
        (
            x * cos - y * sin + centre_x + self.translate_x,
            x * sin + y * cos + centre_y + self.translate_y,
        )
    }

    /// Maps a point in document pixel space back to layer pixel space.
    ///
    /// Returns an error only when the transform is singular, which validation
    /// already prevents; callers resolve it once per render rather than per
    /// pixel.
    pub fn inverse(&self, width: u32, height: u32) -> Result<InverseTransform, AppError> {
        self.validate()?;
        let (scale_x, scale_y) = self.effective_scale();
        if scale_x == 0.0 || scale_y == 0.0 {
            return Err(AppError::InvalidLayerTransform(
                "a zero scale cannot be inverted".into(),
            ));
        }
        let radians = self.rotation_degrees.to_radians();
        let (sin, cos) = radians.sin_cos();
        Ok(InverseTransform {
            centre_x: width as f32 / 2.0,
            centre_y: height as f32 / 2.0,
            translate_x: self.translate_x,
            translate_y: self.translate_y,
            inverse_scale_x: 1.0 / scale_x,
            inverse_scale_y: 1.0 / scale_y,
            sin,
            cos,
        })
    }

    /// Axis-aligned bounding box of the transformed layer in document space,
    /// returned as inclusive-exclusive integer pixel bounds.
    pub fn document_bounds(&self, width: u32, height: u32) -> LayerBounds {
        let corners = [
            self.forward((0.0, 0.0), width, height),
            self.forward((width as f32, 0.0), width, height),
            self.forward((width as f32, height as f32), width, height),
            self.forward((0.0, height as f32), width, height),
        ];
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for (x, y) in corners {
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
        LayerBounds {
            min_x: min_x.floor() as i64 - 1,
            min_y: min_y.floor() as i64 - 1,
            max_x: max_x.ceil() as i64 + 1,
            max_y: max_y.ceil() as i64 + 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayerBounds {
    pub min_x: i64,
    pub min_y: i64,
    pub max_x: i64,
    pub max_y: i64,
}

impl LayerBounds {
    /// Intersects the layer bounds with a canvas, returning the region of the
    /// canvas the layer can actually touch. `None` means the layer lies wholly
    /// outside the canvas and contributes nothing.
    pub fn clip_to_canvas(
        &self,
        canvas_width: u32,
        canvas_height: u32,
    ) -> Option<(u32, u32, u32, u32)> {
        let min_x = self.min_x.max(0);
        let min_y = self.min_y.max(0);
        let max_x = self.max_x.min(i64::from(canvas_width));
        let max_y = self.max_y.min(i64::from(canvas_height));
        if min_x >= max_x || min_y >= max_y {
            return None;
        }
        Some((min_x as u32, min_y as u32, max_x as u32, max_y as u32))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InverseTransform {
    centre_x: f32,
    centre_y: f32,
    translate_x: f32,
    translate_y: f32,
    inverse_scale_x: f32,
    inverse_scale_y: f32,
    sin: f32,
    cos: f32,
}

impl InverseTransform {
    /// Maps a document-space point back into layer pixel space.
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        let x = x - self.centre_x - self.translate_x;
        let y = y - self.centre_y - self.translate_y;
        let unrotated_x = x * self.cos + y * self.sin;
        let unrotated_y = -x * self.sin + y * self.cos;
        (
            unrotated_x * self.inverse_scale_x + self.centre_x,
            unrotated_y * self.inverse_scale_y + self.centre_y,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(left: f32, right: f32) -> bool {
        (left - right).abs() < 1e-3
    }

    /// The sampling mode is a new key inside the layer tree, which does not
    /// deny unknown fields. That is what lets a 0.8.2 project open in 0.8.0 and
    /// 0.8.1: those releases skip the key and draw the layer bilinearly, which
    /// is exactly what they always did. Pinning it here keeps a future
    /// deny_unknown_fields from breaking that quietly.
    #[test]
    fn an_unknown_transform_key_is_ignored_so_older_releases_can_still_read_a_project() {
        let json = r#"{"translateX":1.0,"translateY":2.0,"scaleX":1.0,"scaleY":1.0,
            "rotationDegrees":0.0,"flipHorizontal":false,"flipVertical":false,
            "someFutureField":42}"#;
        let transform: LayerTransform = serde_json::from_str(json).expect("unknown keys skipped");
        assert_eq!(transform.translate_x, 1.0);
        assert_eq!(transform.interpolation, LayerInterpolation::Bilinear);
    }

    #[test]
    fn identity_transform_is_recognized_and_round_trips() {
        let transform = LayerTransform::default();
        assert!(transform.is_identity());
        assert_eq!(transform.integer_translation(), Some((0, 0)));
        let mapped = transform.forward((7.0, 9.0), 20, 20);
        assert!(close(mapped.0, 7.0) && close(mapped.1, 9.0));
    }

    #[test]
    fn forward_and_inverse_are_mutual_for_every_supported_component() {
        let transform = LayerTransform {
            translate_x: 12.5,
            translate_y: -7.25,
            scale_x: 1.75,
            scale_y: 0.6,
            rotation_degrees: 33.0,
            flip_horizontal: true,
            flip_vertical: false,
            interpolation: LayerInterpolation::Bilinear,
        };
        let inverse = transform.inverse(40, 24).unwrap();
        for point in [(0.0, 0.0), (12.0, 5.0), (40.0, 24.0), (3.5, 21.75)] {
            let forward = transform.forward(point, 40, 24);
            let back = inverse.apply(forward.0, forward.1);
            assert!(
                close(back.0, point.0) && close(back.1, point.1),
                "{point:?} -> {forward:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn integer_translation_is_only_reported_for_whole_pixel_moves() {
        let mut transform = LayerTransform {
            translate_x: 4.0,
            translate_y: -3.0,
            ..LayerTransform::default()
        };
        assert_eq!(transform.integer_translation(), Some((4, -3)));
        transform.translate_x = 4.5;
        assert_eq!(transform.integer_translation(), None);
        transform.translate_x = 4.0;
        transform.rotation_degrees = 1.0;
        assert_eq!(transform.integer_translation(), None);
        transform.rotation_degrees = 0.0;
        transform.flip_horizontal = true;
        assert_eq!(transform.integer_translation(), None);
    }

    #[test]
    fn flips_mirror_around_the_layer_centre() {
        let transform = LayerTransform {
            flip_horizontal: true,
            ..LayerTransform::default()
        };
        let mapped = transform.forward((0.0, 5.0), 10, 10);
        assert!(close(mapped.0, 10.0) && close(mapped.1, 5.0));

        let transform = LayerTransform {
            flip_vertical: true,
            interpolation: LayerInterpolation::Bilinear,
            ..LayerTransform::default()
        };
        let mapped = transform.forward((5.0, 0.0), 10, 10);
        assert!(close(mapped.0, 5.0) && close(mapped.1, 10.0));
    }

    #[test]
    fn ninety_degree_rotation_maps_corners_predictably() {
        let transform = LayerTransform {
            rotation_degrees: 90.0,
            ..LayerTransform::default()
        };
        let mapped = transform.forward((0.0, 0.0), 10, 10);
        assert!(close(mapped.0, 10.0) && close(mapped.1, 0.0));
    }

    #[test]
    fn document_bounds_track_translation_and_scale() {
        let transform = LayerTransform {
            translate_x: 100.0,
            translate_y: 50.0,
            ..LayerTransform::default()
        };
        let bounds = transform.document_bounds(20, 10);
        assert_eq!(bounds.min_x, 99);
        assert_eq!(bounds.min_y, 49);
        assert_eq!(bounds.max_x, 121);
        assert_eq!(bounds.max_y, 61);

        let scaled = LayerTransform {
            scale_x: 2.0,
            scale_y: 2.0,
            ..LayerTransform::default()
        };
        let bounds = scaled.document_bounds(20, 10);
        assert_eq!(bounds.min_x, -11);
        assert_eq!(bounds.max_x, 31);
    }

    #[test]
    fn bounds_clip_against_the_canvas_and_reject_layers_that_miss_it() {
        let bounds = LayerBounds {
            min_x: -50,
            min_y: -50,
            max_x: 30,
            max_y: 20,
        };
        assert_eq!(bounds.clip_to_canvas(100, 100), Some((0, 0, 30, 20)));

        let outside = LayerBounds {
            min_x: 500,
            min_y: 500,
            max_x: 600,
            max_y: 600,
        };
        assert_eq!(outside.clip_to_canvas(100, 100), None);

        let negative = LayerBounds {
            min_x: -600,
            min_y: -600,
            max_x: -100,
            max_y: -100,
        };
        assert_eq!(negative.clip_to_canvas(100, 100), None);
    }

    #[test]
    fn validation_rejects_non_finite_singular_and_oversized_transforms() {
        let cases = [
            LayerTransform {
                translate_x: f32::NAN,
                ..LayerTransform::default()
            },
            LayerTransform {
                translate_y: f32::INFINITY,
                ..LayerTransform::default()
            },
            LayerTransform {
                scale_x: 0.0,
                ..LayerTransform::default()
            },
            LayerTransform {
                scale_y: 1_000.0,
                ..LayerTransform::default()
            },
            LayerTransform {
                scale_x: MIN_LAYER_SCALE / 2.0,
                ..LayerTransform::default()
            },
            LayerTransform {
                rotation_degrees: 720.0,
                ..LayerTransform::default()
            },
            LayerTransform {
                translate_x: MAX_LAYER_TRANSLATION * 2.0,
                ..LayerTransform::default()
            },
        ];
        for transform in cases {
            assert!(
                matches!(
                    transform.validate(),
                    Err(AppError::InvalidLayerTransform(_))
                ),
                "{transform:?} should be rejected"
            );
            assert!(transform.inverse(10, 10).is_err());
        }
    }

    #[test]
    fn negative_scales_remain_invertible() {
        let transform = LayerTransform {
            scale_x: -1.5,
            scale_y: 2.0,
            ..LayerTransform::default()
        };
        transform.validate().unwrap();
        let inverse = transform.inverse(16, 16).unwrap();
        let forward = transform.forward((3.0, 4.0), 16, 16);
        let back = inverse.apply(forward.0, forward.1);
        assert!(close(back.0, 3.0) && close(back.1, 4.0));
    }

    #[test]
    fn transforms_round_trip_through_camel_case_json() {
        let transform = LayerTransform {
            translate_x: 3.0,
            translate_y: -4.0,
            scale_x: 1.25,
            scale_y: 0.75,
            rotation_degrees: 15.0,
            flip_horizontal: true,
            flip_vertical: true,
            interpolation: LayerInterpolation::Bilinear,
        };
        let json = serde_json::to_string(&transform).unwrap();
        assert!(json.contains("translateX"));
        assert!(json.contains("flipHorizontal"));
        assert_eq!(
            serde_json::from_str::<LayerTransform>(&json).unwrap(),
            transform
        );
    }
}
