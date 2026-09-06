//! Shape layer content: what a vector layer stores, and how it is drawn.
//!
//! A shape layer keeps geometry, not pixels. It is drawn by rasterising its
//! path directly into the rectangle being rendered, which is why it can be
//! scaled and rotated without ever accumulating resampling damage: there is no
//! source raster to resample.
use serde::{Deserialize, Serialize};

use crate::color::{FloatImage, FloatRgba};
use crate::error::AppError;
use crate::vector::{
    rasterize, stroke_outlines, FillRule, ShapeGeometry, StrokeStyle, MAX_STROKE_WIDTH,
};

/// How finely curves are flattened before rasterising, in device pixels.
///
/// Fixed rather than derived from the transform, so the same shape produces the
/// same geometry whichever rectangle asked for it. A tile that flattened more
/// coarsely than its neighbour would seam.
pub const FLATTEN_TOLERANCE: f32 = 0.05;

/// A solid colour, in the document's linear working space.
///
/// Straight alpha, like every other colour in the document, so a half
/// transparent fill means what it means everywhere else.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeColor {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl ShapeColor {
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    pub fn is_valid(&self) -> bool {
        [self.red, self.green, self.blue, self.alpha]
            .iter()
            .all(|v| v.is_finite())
            && (0.0..=1.0).contains(&self.alpha)
            // RGB may sit outside [0,1] — the working space is not gamut
            // limited — but not so far outside that it is obviously junk.
            && [self.red, self.green, self.blue]
                .iter()
                .all(|v| v.abs() <= 64.0)
    }

    const fn to_rgba(self) -> FloatRgba {
        FloatRgba::new(self.red, self.green, self.blue, self.alpha)
    }
}

/// Everything a shape layer holds.
///
/// Fill and stroke are both optional and independent: a shape with neither
/// draws nothing, which is a legitimate state while the user is deciding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShapeContent {
    pub geometry: ShapeGeometry,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<ShapeColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<ShapeColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_style: Option<StrokeStyle>,
    #[serde(default)]
    pub fill_rule: FillRule,
}

impl ShapeContent {
    pub fn validate(&self) -> Result<(), AppError> {
        self.geometry.validate()?;
        for colour in [self.fill.as_ref(), self.stroke.as_ref()]
            .into_iter()
            .flatten()
        {
            if !colour.is_valid() {
                return Err(AppError::InvalidLayerDocument(
                    "a shape colour was not finite or was outside the supported range".into(),
                ));
            }
        }
        if let Some(style) = &self.stroke_style {
            if !style.is_valid() {
                return Err(AppError::InvalidLayerDocument(format!(
                    "a shape stroke width must be above zero and at most {MAX_STROKE_WIDTH}"
                )));
            }
        }
        Ok(())
    }

    /// Whether anything would be drawn at all.
    pub fn draws_anything(&self) -> bool {
        let fills = self.fill.is_some_and(|c| c.alpha > 0.0) && self.geometry.is_closed();
        let strokes = self.stroke.is_some_and(|c| c.alpha > 0.0)
            && self.stroke_style.is_some_and(|s| s.is_valid());
        fills || strokes
    }

    /// The document-space bounds this shape can paint into, before the layer
    /// transform is applied.
    ///
    /// Conservative: a stroke reaches half its width beyond the path, and a
    /// miter join can reach further still, so the miter limit is included.
    pub fn untransformed_bounds(&self) -> Option<(f32, f32, f32, f32)> {
        let (min_x, min_y, max_x, max_y) = self.geometry.to_path().bounds(FLATTEN_TOLERANCE)?;
        let mut margin = 0.0f32;
        if let (Some(colour), Some(style)) = (self.stroke, self.stroke_style) {
            if colour.alpha > 0.0 && style.is_valid() {
                // A miter can reach `limit` half-widths from the joint.
                margin = style.width * 0.5 * style.miter_limit.max(1.0);
            }
        }
        Some((
            min_x - margin,
            min_y - margin,
            max_x + margin,
            max_y + margin,
        ))
    }
}

/// Draws a shape into `canvas`, which represents `region` of the render canvas.
///
/// `to_device` maps a point in document space to a pixel in the render canvas,
/// carrying the render scale and the layer's transform. Applying the transform
/// to the *geometry* rather than resampling a rendered raster is what keeps a
/// shape sharp however many times it is scaled and rotated.
pub fn render_shape<F>(
    content: &ShapeContent,
    to_device: F,
    region_x: i32,
    region_y: i32,
    region_width: u32,
    region_height: u32,
) -> Result<Option<(FloatImage, Vec<f32>)>, AppError>
where
    F: Fn((f32, f32)) -> (f32, f32),
{
    if !content.draws_anything() || region_width == 0 || region_height == 0 {
        return Ok(None);
    }
    let subpaths: Vec<Vec<(f32, f32)>> = content
        .geometry
        .to_path()
        .flatten(FLATTEN_TOLERANCE)
        .into_iter()
        .map(|subpath| subpath.into_iter().map(&to_device).collect())
        .collect();
    if subpaths.is_empty() {
        return Ok(None);
    }

    let mut colours = FloatImage::blank(region_width, region_height, FloatRgba::TRANSPARENT)?;
    let mut coverage = vec![0.0f32; (region_width as usize) * (region_height as usize)];

    // Fill first, then stroke over it, which is the order every drawing
    // application uses and the one a user expects when both are set.
    /// One paint pass: a colour, the outlines to cover with it, and the rule
    /// deciding what counts as inside.
    type PaintPass = (FloatRgba, Vec<Vec<(f32, f32)>>, FillRule);
    let mut layers: Vec<PaintPass> = Vec::new();
    if let Some(fill) = content.fill {
        if fill.alpha > 0.0 && content.geometry.is_closed() {
            layers.push((fill.to_rgba(), subpaths.clone(), content.fill_rule));
        }
    }
    if let (Some(colour), Some(style)) = (content.stroke, content.stroke_style) {
        if colour.alpha > 0.0 && style.is_valid() {
            // The stroke is generated in device space, so its width is in
            // device pixels and a scaled-up shape gets a proportionally
            // scaled stroke — which is what "scale the shape" means.
            let scaled = device_stroke_style(&style, &to_device);
            let outlines = stroke_outlines(&subpaths, &scaled);
            if !outlines.is_empty() {
                layers.push((colour.to_rgba(), outlines, FillRule::NonZero));
            }
        }
    }

    for (colour, outlines, rule) in layers {
        let mask = rasterize(
            &outlines,
            region_x,
            region_y,
            region_width,
            region_height,
            rule,
        );
        for (index, alpha) in mask.coverage.iter().enumerate() {
            let incoming = alpha * colour.alpha;
            if incoming <= 0.0 {
                continue;
            }
            // Source-over in premultiplied linear, then back to straight alpha,
            // which is the same arithmetic the compositor uses for everything
            // else.
            let existing = colours.pixels()[index];
            let out_alpha = incoming + existing.alpha * (1.0 - incoming);
            let mix = |over: f32, under: f32| {
                if out_alpha <= 0.0 {
                    0.0
                } else {
                    (over * incoming + under * existing.alpha * (1.0 - incoming)) / out_alpha
                }
            };
            colours.pixels_mut()[index] = FloatRgba::new(
                mix(colour.red, existing.red),
                mix(colour.green, existing.green),
                mix(colour.blue, existing.blue),
                out_alpha,
            );
            coverage[index] = out_alpha;
        }
    }

    Ok(Some((colours, coverage)))
}

/// The stroke width in device pixels, given the mapping into device space.
///
/// Measured from the mapping rather than assumed, so a shape scaled by its
/// layer transform gets a stroke scaled with it. The average of the two axes is
/// used because a single width cannot express an anisotropic scale, and that
/// limitation is recorded in `docs/vector-layers.md`.
fn device_stroke_style<F>(style: &StrokeStyle, to_device: &F) -> StrokeStyle
where
    F: Fn((f32, f32)) -> (f32, f32),
{
    let origin = to_device((0.0, 0.0));
    let along_x = to_device((1.0, 0.0));
    let along_y = to_device((0.0, 1.0));
    let length = |a: (f32, f32), b: (f32, f32)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        (dx * dx + dy * dy).sqrt()
    };
    let scale = (length(origin, along_x) + length(origin, along_y)) * 0.5;
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    StrokeStyle {
        width: (style.width * scale).clamp(0.01, MAX_STROKE_WIDTH),
        ..*style
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(red: f32, green: f32, blue: f32) -> ShapeColor {
        ShapeColor::new(red, green, blue, 1.0)
    }

    fn square(size: f32) -> ShapeContent {
        ShapeContent {
            geometry: ShapeGeometry::Rectangle {
                x: 10.0,
                y: 10.0,
                width: size,
                height: size,
                corner_radius: 0.0,
            },
            fill: Some(solid(1.0, 0.0, 0.0)),
            stroke: None,
            stroke_style: None,
            fill_rule: FillRule::NonZero,
        }
    }

    #[test]
    fn a_filled_shape_paints_its_colour_and_covers_its_area() {
        let content = square(40.0);
        let (colours, coverage) = render_shape(&content, |p| p, 0, 0, 64, 64)
            .unwrap()
            .unwrap();
        let covered: f64 = coverage.iter().map(|v| f64::from(*v)).sum();
        assert!(
            (covered - 1600.0).abs() < 1.0,
            "covered {covered} of an expected 1600"
        );
        let centre = colours.get(30, 30).unwrap();
        assert!((centre.red - 1.0).abs() < 1e-6);
        assert!(centre.green.abs() < 1e-6);
        assert!((centre.alpha - 1.0).abs() < 1e-6);
        // Outside the shape nothing was written.
        assert_eq!(colours.get(2, 2).unwrap().alpha, 0.0);
    }

    /// A shape with nothing set draws nothing rather than something surprising.
    #[test]
    fn a_shape_with_no_paint_draws_nothing() {
        let mut content = square(40.0);
        content.fill = None;
        assert!(!content.draws_anything());
        assert!(render_shape(&content, |p| p, 0, 0, 64, 64)
            .unwrap()
            .is_none());

        // A fully transparent fill is the same as no fill.
        content.fill = Some(ShapeColor::new(1.0, 0.0, 0.0, 0.0));
        assert!(!content.draws_anything());

        // A line has no interior, so a fill alone paints nothing.
        let line = ShapeContent {
            geometry: ShapeGeometry::Line {
                x1: 0.0,
                y1: 0.0,
                x2: 10.0,
                y2: 10.0,
            },
            fill: Some(solid(1.0, 1.0, 1.0)),
            stroke: None,
            stroke_style: None,
            fill_rule: FillRule::NonZero,
        };
        assert!(!line.draws_anything());
    }

    /// A stroke over a fill paints the stroke colour on the boundary.
    #[test]
    fn a_stroke_paints_over_its_fill() {
        let content = ShapeContent {
            geometry: ShapeGeometry::Rectangle {
                x: 16.0,
                y: 16.0,
                width: 32.0,
                height: 32.0,
                corner_radius: 0.0,
            },
            fill: Some(solid(1.0, 0.0, 0.0)),
            stroke: Some(solid(0.0, 0.0, 1.0)),
            stroke_style: Some(StrokeStyle {
                width: 4.0,
                ..StrokeStyle::default()
            }),
            fill_rule: FillRule::NonZero,
        };
        let (colours, _) = render_shape(&content, |p| p, 0, 0, 64, 64)
            .unwrap()
            .unwrap();
        // Deep inside: the fill.
        let inside = colours.get(32, 32).unwrap();
        assert!(inside.red > 0.99 && inside.blue < 0.01, "{inside:?}");
        // On the edge: the stroke.
        let edge = colours.get(16, 32).unwrap();
        assert!(edge.blue > 0.99 && edge.red < 0.01, "{edge:?}");
    }

    /// The transform is applied to geometry, so scaling up does not blur: the
    /// edge stays as sharp as it was.
    #[test]
    fn scaling_transforms_geometry_rather_than_pixels() {
        let content = square(10.0);
        // Four times larger, drawn directly at that size.
        let (colours, coverage) =
            render_shape(&content, |(x, y)| (x * 4.0, y * 4.0), 0, 0, 256, 256)
                .unwrap()
                .unwrap();
        let covered: f64 = coverage.iter().map(|v| f64::from(*v)).sum();
        assert!(
            (covered - 1600.0).abs() < 2.0,
            "a 10px square scaled 4x covered {covered}, expected 1600"
        );
        // The edge is one pixel wide, not four: nothing was resampled.
        let mut soft = 0;
        for y in 0..256 {
            let value = colours.get(200, y).unwrap().alpha;
            if value > 0.001 && value < 0.999 {
                soft += 1;
            }
        }
        assert!(soft <= 2, "the scaled edge was {soft} pixels of gradient");
    }

    /// A stroke has to scale with the shape, or a scaled-up shape draws a
    /// hairline.
    #[test]
    fn a_stroke_scales_with_its_shape() {
        let content = ShapeContent {
            geometry: ShapeGeometry::Line {
                x1: 4.0,
                y1: 16.0,
                x2: 28.0,
                y2: 16.0,
            },
            fill: None,
            stroke: Some(solid(1.0, 1.0, 1.0)),
            stroke_style: Some(StrokeStyle {
                width: 2.0,
                ..StrokeStyle::default()
            }),
            fill_rule: FillRule::NonZero,
        };
        let area = |scale: f32| {
            let (_, coverage) = render_shape(
                &content,
                |(x, y)| (x * scale, y * scale),
                0,
                0,
                (64.0 * scale) as u32,
                (64.0 * scale) as u32,
            )
            .unwrap()
            .unwrap();
            coverage.iter().map(|v| f64::from(*v)).sum::<f64>()
        };
        let single = area(1.0);
        let double = area(2.0);
        // Twice as long and twice as wide is four times the area.
        assert!(
            (double / single - 4.0).abs() < 0.2,
            "scaling 2x changed stroke area by {}x",
            double / single
        );
    }

    #[test]
    fn hostile_shape_content_is_refused() {
        let cases = vec![
            ShapeContent {
                geometry: ShapeGeometry::Rectangle {
                    x: f32::NAN,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                    corner_radius: 0.0,
                },
                fill: Some(solid(1.0, 1.0, 1.0)),
                stroke: None,
                stroke_style: None,
                fill_rule: FillRule::NonZero,
            },
            ShapeContent {
                fill: Some(ShapeColor::new(f32::NAN, 0.0, 0.0, 1.0)),
                ..square(10.0)
            },
            ShapeContent {
                fill: Some(ShapeColor::new(0.0, 0.0, 0.0, 5.0)),
                ..square(10.0)
            },
            ShapeContent {
                stroke: Some(solid(1.0, 1.0, 1.0)),
                stroke_style: Some(StrokeStyle {
                    width: -1.0,
                    ..StrokeStyle::default()
                }),
                ..square(10.0)
            },
            ShapeContent {
                stroke: Some(solid(1.0, 1.0, 1.0)),
                stroke_style: Some(StrokeStyle {
                    width: f32::INFINITY,
                    ..StrokeStyle::default()
                }),
                ..square(10.0)
            },
        ];
        for content in cases {
            assert!(content.validate().is_err(), "{content:?} was accepted");
        }
    }

    /// Bounds have to cover everything the shape paints, or the cache will
    /// invalidate the wrong region and leave debris on the canvas.
    #[test]
    fn bounds_cover_everything_the_shape_paints() {
        let content = ShapeContent {
            geometry: ShapeGeometry::Rectangle {
                x: 40.0,
                y: 40.0,
                width: 40.0,
                height: 40.0,
                corner_radius: 0.0,
            },
            fill: Some(solid(1.0, 0.0, 0.0)),
            stroke: Some(solid(0.0, 1.0, 0.0)),
            stroke_style: Some(StrokeStyle {
                width: 12.0,
                ..StrokeStyle::default()
            }),
            fill_rule: FillRule::NonZero,
        };
        let (min_x, min_y, max_x, max_y) = content.untransformed_bounds().unwrap();
        let (_, coverage) = render_shape(&content, |p| p, 0, 0, 160, 160)
            .unwrap()
            .unwrap();
        for y in 0..160u32 {
            for x in 0..160u32 {
                if coverage[(y * 160 + x) as usize] <= 0.0 {
                    continue;
                }
                let (fx, fy) = (x as f32, y as f32);
                assert!(
                    fx >= min_x - 1.0
                        && fx <= max_x + 1.0
                        && fy >= min_y - 1.0
                        && fy <= max_y + 1.0,
                    "painted ({x},{y}) outside bounds ({min_x},{min_y})-({max_x},{max_y})"
                );
            }
        }
    }
}
