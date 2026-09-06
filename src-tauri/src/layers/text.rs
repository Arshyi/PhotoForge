//! Text layer content: what a text layer stores, and how it is drawn.
//!
//! A text layer stores the characters, the font requested, and the setting.
//! Never pixels, and never glyph indices — those belong to a particular font on
//! a particular machine, and a project that stored them would render as garbage
//! on the next machine. Reopening a project re-shapes the text, so editing it
//! after a save is the same operation as editing it before one.
//!
//! Glyph outlines go through the same rasteriser vector shapes use. The one
//! difference is where flattening happens: a glyph's control points are mapped
//! into device space first and flattened there, so the curve tolerance is
//! measured in the pixels actually being drawn. Text is read at every zoom
//! level, and flattening in document space would coarsen exactly when the user
//! has zoomed in to look closely.
use serde::{Deserialize, Serialize};

use crate::color::{FloatImage, FloatRgba};
use crate::error::AppError;
use crate::layers::shape::ShapeColor;
use crate::text::{
    self, PositionedGlyph, ShapeRequest, ShapedText, TextAlign, MAX_FONT_NAME_CHARS, MAX_FONT_SIZE,
    MAX_TEXT_BYTES, MIN_FONT_SIZE,
};
use crate::vector::{
    rasterize, stroke_outlines, FillRule, LineCap, LineJoin, PathCommand, StrokeStyle, VectorPath,
    MAX_STROKE_WIDTH,
};

/// Curve flattening tolerance for glyphs, in device pixels.
///
/// A constant, and applied after the mapping into device space, so the
/// flattened outline depends only on the text and the transform — never on
/// which tile asked for it. Two tiles that flattened differently would seam
/// down the middle of a letter.
pub const DEVICE_FLATTEN_TOLERANCE: f32 = 0.05;

/// The narrowest wrapping width accepted, in document pixels.
///
/// Below this, wrapping cannot place even one glyph per line and layout
/// degenerates.
pub const MIN_WRAP_WIDTH: f32 = 1.0;
/// The widest wrapping width accepted.
pub const MAX_WRAP_WIDTH: f32 = 200_000.0;
/// How far a text block's origin may sit from the canvas.
pub const MAX_TEXT_ORIGIN: f32 = 1_000_000.0;
/// Font weights, as CSS numbers.
pub const MIN_FONT_WEIGHT: u16 = 100;
pub const MAX_FONT_WEIGHT: u16 = 900;
/// Letter spacing bound, in document pixels, either direction.
pub const MAX_LETTER_SPACING: f32 = 1_000.0;

/// Everything a text layer holds.
///
/// `font_family` is what the user asked for and stays what the user asked for.
/// If the machine does not have it, the text is drawn in something else and
/// reported as missing — the request is never rewritten to the substitute,
/// because reopening the project on a machine that does have the font must
/// restore the intended setting rather than a record of one machine's gap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextContent {
    pub text: String,
    /// The requested family, or empty for the system's default sans-serif.
    #[serde(default)]
    pub font_family: String,
    pub font_size: f32,
    #[serde(default = "default_weight")]
    pub font_weight: u16,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub align: TextAlign,
    /// Line advance as a multiple of the font size.
    #[serde(default = "default_line_height")]
    pub line_height: f32,
    #[serde(default)]
    pub letter_spacing: f32,
    /// Where the text block's first line begins, in document coordinates.
    pub origin_x: f32,
    pub origin_y: f32,
    /// The wrapping width for area text, or `None` for point text on one line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_width: Option<f32>,
    pub fill: ShapeColor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<ShapeColor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_style: Option<StrokeStyle>,
}

fn default_weight() -> u16 {
    400
}

fn default_line_height() -> f32 {
    1.2
}

impl TextContent {
    /// A plain block of text at a point, in the default face.
    pub fn new(text: impl Into<String>, origin_x: f32, origin_y: f32, font_size: f32) -> Self {
        Self {
            text: text.into(),
            font_family: String::new(),
            font_size,
            font_weight: default_weight(),
            italic: false,
            align: TextAlign::Start,
            line_height: default_line_height(),
            letter_spacing: 0.0,
            origin_x,
            origin_y,
            wrap_width: None,
            fill: ShapeColor::new(0.0, 0.0, 0.0, 1.0),
            stroke: None,
            stroke_style: None,
        }
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.text.len() > MAX_TEXT_BYTES {
            return Err(AppError::InvalidLayerDocument(format!(
                "a text layer may hold at most {MAX_TEXT_BYTES} bytes"
            )));
        }
        if self.font_family.chars().count() > MAX_FONT_NAME_CHARS {
            return Err(AppError::InvalidLayerDocument(format!(
                "a font family name may be at most {MAX_FONT_NAME_CHARS} characters"
            )));
        }
        if !self.font_size.is_finite() || !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&self.font_size)
        {
            return Err(AppError::InvalidLayerDocument(format!(
                "a font size must be between {MIN_FONT_SIZE} and {MAX_FONT_SIZE}"
            )));
        }
        if !(MIN_FONT_WEIGHT..=MAX_FONT_WEIGHT).contains(&self.font_weight) {
            return Err(AppError::InvalidLayerDocument(format!(
                "a font weight must be between {MIN_FONT_WEIGHT} and {MAX_FONT_WEIGHT}"
            )));
        }
        if !self.line_height.is_finite() || !(0.1..=10.0).contains(&self.line_height) {
            return Err(AppError::InvalidLayerDocument(
                "a line height must be between 0.1 and 10 times the font size".into(),
            ));
        }
        if !self.letter_spacing.is_finite() || self.letter_spacing.abs() > MAX_LETTER_SPACING {
            return Err(AppError::InvalidLayerDocument(format!(
                "letter spacing must be within {MAX_LETTER_SPACING} pixels"
            )));
        }
        if !self.origin_x.is_finite()
            || !self.origin_y.is_finite()
            || self.origin_x.abs() > MAX_TEXT_ORIGIN
            || self.origin_y.abs() > MAX_TEXT_ORIGIN
        {
            return Err(AppError::InvalidLayerDocument(
                "a text origin was not finite or was implausibly far from the canvas".into(),
            ));
        }
        if let Some(width) = self.wrap_width {
            if !width.is_finite() || !(MIN_WRAP_WIDTH..=MAX_WRAP_WIDTH).contains(&width) {
                return Err(AppError::InvalidLayerDocument(format!(
                    "a text wrapping width must be between {MIN_WRAP_WIDTH} and {MAX_WRAP_WIDTH}"
                )));
            }
        }
        for colour in [Some(&self.fill), self.stroke.as_ref()]
            .into_iter()
            .flatten()
        {
            if !colour.is_valid() {
                return Err(AppError::InvalidLayerDocument(
                    "a text colour was not finite or was outside the supported range".into(),
                ));
            }
        }
        if let Some(style) = &self.stroke_style {
            if !style.is_valid() {
                return Err(AppError::InvalidLayerDocument(format!(
                    "a text stroke width must be above zero and at most {MAX_STROKE_WIDTH}"
                )));
            }
        }
        Ok(())
    }

    /// Whether anything would be drawn at all.
    pub fn draws_anything(&self) -> bool {
        if self.text.trim().is_empty() {
            return false;
        }
        let fills = self.fill.alpha > 0.0;
        let strokes = self.stroke.is_some_and(|c| c.alpha > 0.0)
            && self.stroke_style.is_some_and(|s| s.is_valid());
        fills || strokes
    }

    fn request(&self) -> ShapeRequest<'_> {
        ShapeRequest {
            text: &self.text,
            family: &self.font_family,
            size: self.font_size,
            line_height: self.line_height,
            letter_spacing: self.letter_spacing,
            weight: self.font_weight,
            italic: self.italic,
            align: self.align,
            wrap_width: self.wrap_width,
        }
    }

    /// Lays the text out, reusing an identical earlier layout when there is one.
    pub fn shaped(&self) -> Result<std::sync::Arc<ShapedText>, AppError> {
        text::shape(&self.request())
    }

    /// Whether the requested family is installed on this machine.
    ///
    /// Reported rather than repaired: the caller tells the user their font is
    /// missing, and the project goes on asking for it.
    pub fn font_is_missing(&self) -> bool {
        !self.font_family.is_empty() && !text::family_is_available(&self.font_family)
    }

    /// The document-space bounds the text can paint into, before the layer
    /// transform.
    ///
    /// Measured from the glyph outlines rather than from the line boxes,
    /// because ink routinely leaves the line box: descenders, accents, italic
    /// overhang and swashes all do. A bound taken from the line box would clip
    /// them.
    pub fn untransformed_bounds(&self) -> Result<Option<(f32, f32, f32, f32)>, AppError> {
        if !self.draws_anything() {
            return Ok(None);
        }
        let shaped = self.shaped()?;
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        for glyph in &shaped.glyphs {
            let Some((min_x, min_y, max_x, max_y)) = glyph.path.bounds(DEVICE_FLATTEN_TOLERANCE)
            else {
                continue;
            };
            let (min_x, max_x) = (min_x + glyph.x, max_x + glyph.x);
            let (min_y, max_y) = (min_y + glyph.y, max_y + glyph.y);
            bounds = Some(match bounds {
                None => (min_x, min_y, max_x, max_y),
                Some(existing) => (
                    existing.0.min(min_x),
                    existing.1.min(min_y),
                    existing.2.max(max_x),
                    existing.3.max(max_y),
                ),
            });
        }
        let Some((min_x, min_y, max_x, max_y)) = bounds else {
            return Ok(None);
        };
        // A stroke reaches half its width beyond the outline, and a miter join
        // further still.
        let mut margin = 0.0f32;
        if let (Some(colour), Some(style)) = (self.stroke, self.stroke_style) {
            if colour.alpha > 0.0 && style.is_valid() {
                margin = style.width * 0.5 * style.miter_limit.max(1.0);
            }
        }
        Ok(Some((
            self.origin_x + min_x - margin,
            self.origin_y + min_y - margin,
            self.origin_x + max_x + margin,
            self.origin_y + max_y + margin,
        )))
    }
}

/// Maps a path's control points through `f`.
///
/// Exact for the transforms PhotoForge supports, which are affine: an affine
/// map sends a cubic's control polygon to the transformed cubic's control
/// polygon, so mapping four points is the whole of transforming the curve. That
/// equivalence is what lets flattening be deferred until after the mapping.
fn map_path<F>(path: &VectorPath, f: &F) -> VectorPath
where
    F: Fn((f32, f32)) -> (f32, f32),
{
    let commands = path
        .commands
        .iter()
        .map(|command| match *command {
            PathCommand::MoveTo { x, y } => {
                let (x, y) = f((x, y));
                PathCommand::MoveTo { x, y }
            }
            PathCommand::LineTo { x, y } => {
                let (x, y) = f((x, y));
                PathCommand::LineTo { x, y }
            }
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                let (c1x, c1y) = f((c1x, c1y));
                let (c2x, c2y) = f((c2x, c2y));
                let (x, y) = f((x, y));
                PathCommand::CubicTo {
                    c1x,
                    c1y,
                    c2x,
                    c2y,
                    x,
                    y,
                }
            }
            PathCommand::Close => PathCommand::Close,
        })
        .collect();
    VectorPath::new(commands)
}

/// The glyph outlines of one shaped block, in device space.
fn device_outlines<F>(
    shaped: &ShapedText,
    content: &TextContent,
    to_device: &F,
) -> Vec<Vec<(f32, f32)>>
where
    F: Fn((f32, f32)) -> (f32, f32),
{
    let mut outlines = Vec::new();
    for glyph in &shaped.glyphs {
        if glyph.path.commands.is_empty() {
            continue;
        }
        let place = |(x, y): (f32, f32)| {
            to_device((
                content.origin_x + glyph.x + x,
                content.origin_y + glyph.y + y,
            ))
        };
        let device = map_path(&glyph.path, &place);
        outlines.extend(device.flatten(DEVICE_FLATTEN_TOLERANCE));
    }
    outlines
}

/// Draws text into `canvas`, which represents `region` of the render canvas.
///
/// `to_device` maps document space to a pixel in the render canvas, carrying
/// the render scale and the layer transform. The glyphs are rasterised at that
/// scale, so enlarging a text layer re-draws the letterforms rather than
/// enlarging their pixels.
pub fn render_text<F>(
    content: &TextContent,
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
    let shaped = content.shaped()?;
    let outlines = device_outlines(&shaped, content, &to_device);
    if outlines.is_empty() {
        return Ok(None);
    }

    let mut colours = FloatImage::blank(region_width, region_height, FloatRgba::TRANSPARENT)?;
    let mut coverage = vec![0.0f32; (region_width as usize) * (region_height as usize)];

    /// One paint pass: a colour, the outlines to cover with it, and the rule
    /// deciding what counts as inside.
    type PaintPass = (FloatRgba, Vec<Vec<(f32, f32)>>, FillRule);
    let mut passes: Vec<PaintPass> = Vec::new();
    if content.fill.alpha > 0.0 {
        // Non-zero, always. Font outlines are drawn with counters wound against
        // their contours, which is exactly what the non-zero rule reads; even-
        // odd would instead fill the inside of an 'o' and hollow out the
        // overlap where a script face joins two strokes.
        passes.push((to_rgba(content.fill), outlines.clone(), FillRule::NonZero));
    }
    if let (Some(colour), Some(style)) = (content.stroke, content.stroke_style) {
        if colour.alpha > 0.0 && style.is_valid() {
            let scaled = device_stroke_style(&style, &to_device);
            let stroked = stroke_outlines(&outlines, &scaled);
            if !stroked.is_empty() {
                passes.push((to_rgba(colour), stroked, FillRule::NonZero));
            }
        }
    }

    for (colour, paths, rule) in passes {
        let mask = rasterize(
            &paths,
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
            // which is the arithmetic the compositor uses for everything else.
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

fn to_rgba(colour: ShapeColor) -> FloatRgba {
    FloatRgba::new(colour.red, colour.green, colour.blue, colour.alpha)
}

/// The stroke width in device pixels, given the mapping into device space.
///
/// Measured from the mapping rather than assumed, so a text layer scaled by its
/// transform gets a stroke scaled with it.
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

/// Where a caret sits for a byte offset into the text, in document space.
///
/// Returns the caret's top and its height. A byte offset says nothing about a
/// position once the bidirectional algorithm has reordered a line — the caret
/// for a character inside an Arabic run belongs wherever that run put it — so
/// the position is looked up through the glyph that came from those bytes.
pub fn caret_position(
    content: &TextContent,
    shaped: &ShapedText,
    byte_offset: usize,
) -> Option<(f32, f32, f32)> {
    let height = content.font_size * content.line_height;
    if shaped.glyphs.is_empty() {
        return Some((content.origin_x, content.origin_y, height));
    }
    let containing = shaped
        .glyphs
        .iter()
        .find(|g| byte_offset >= g.start && byte_offset < g.end);
    let (glyph, trailing) = match containing {
        Some(glyph) => (glyph, false),
        None => {
            // An offset past the last character sits at the trailing edge of
            // the glyph that ends the text.
            let last = shaped
                .glyphs
                .iter()
                .max_by_key(|g: &&PositionedGlyph| g.end)?;
            if byte_offset < last.end {
                return None;
            }
            (last, true)
        }
    };
    // The caret sits at the edge the character starts from, which is its left
    // edge in a left-to-right run and its right edge in a right-to-left one.
    // Placing it at `glyph.x` regardless would put the caret on the far side of
    // every Arabic, Hebrew and Persian character.
    let leading = if glyph.rtl {
        glyph.x + glyph.advance
    } else {
        glyph.x
    };
    let x = if trailing {
        if glyph.rtl {
            glyph.x
        } else {
            glyph.x + glyph.advance
        }
    } else {
        leading
    };
    Some((
        content.origin_x + x,
        content.origin_y + glyph.y - content.font_size,
        height,
    ))
}

/// A sensible stroke for text, when the user turns one on.
pub fn default_text_stroke() -> StrokeStyle {
    StrokeStyle {
        width: 1.0,
        cap: LineCap::Round,
        join: LineJoin::Round,
        miter_limit: 4.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(point: (f32, f32)) -> (f32, f32) {
        point
    }

    fn sample() -> TextContent {
        TextContent::new("Hello", 20.0, 60.0, 32.0)
    }

    fn total_coverage(coverage: &[f32]) -> f64 {
        coverage.iter().map(|v| f64::from(*v)).sum()
    }

    #[test]
    fn text_renders_ink() {
        let content = sample();
        let (_, coverage) = render_text(&content, identity, 0, 0, 256, 128)
            .expect("render")
            .expect("some ink");
        assert!(
            total_coverage(&coverage) > 50.0,
            "the text covered almost nothing: {}",
            total_coverage(&coverage)
        );
    }

    /// The property the tiled renderer depends on: a tile must equal the same
    /// window of the whole frame, exactly. Anything less is a seam.
    #[test]
    fn a_region_matches_the_same_window_of_the_whole_frame() {
        let content = sample();
        let (whole_colours, whole) = render_text(&content, identity, 0, 0, 256, 128)
            .expect("render")
            .expect("ink");
        let (region_colours, region) = render_text(&content, identity, 64, 32, 96, 64)
            .expect("render")
            .expect("ink");
        for y in 0..64u32 {
            for x in 0..96u32 {
                let region_index = (y * 96 + x) as usize;
                let whole_index = ((y + 32) * 256 + (x + 64)) as usize;
                assert_eq!(
                    region[region_index], whole[whole_index],
                    "coverage differs at {x},{y}"
                );
                assert_eq!(
                    region_colours.pixels()[region_index],
                    whole_colours.pixels()[whole_index],
                    "colour differs at {x},{y}"
                );
            }
        }
    }

    /// Scaling a text layer must re-draw the letterforms, not enlarge pixels.
    /// Enlarged pixels would keep the same proportion of soft edge; redrawn
    /// outlines grow their ink with the square of the scale while their edges
    /// stay one pixel wide, so the soft fraction falls.
    #[test]
    fn scaling_redraws_rather_than_resamples() {
        let content = sample();
        let (_, small) = render_text(&content, identity, 0, 0, 256, 128)
            .expect("render")
            .expect("ink");
        let scaled = |(x, y): (f32, f32)| (x * 4.0, y * 4.0);
        let (_, large) = render_text(&content, scaled, 0, 0, 1024, 512)
            .expect("render")
            .expect("ink");

        let ratio = total_coverage(&large) / total_coverage(&small);
        assert!(
            (12.0..20.0).contains(&ratio),
            "four times the scale should be about sixteen times the ink, but was {ratio}"
        );

        let soft = |coverage: &[f32]| {
            let partial = coverage.iter().filter(|v| **v > 0.01 && **v < 0.99).count() as f64;
            let inked = coverage.iter().filter(|v| **v > 0.01).count() as f64;
            partial / inked
        };
        assert!(
            soft(&large) < soft(&small),
            "the enlarged text was not proportionally sharper, so it was resampled"
        );
    }

    #[test]
    fn bounds_cover_every_inked_pixel() {
        let content = sample();
        let (min_x, min_y, max_x, max_y) = content
            .untransformed_bounds()
            .expect("bounds")
            .expect("some bounds");
        let (_, coverage) = render_text(&content, identity, 0, 0, 256, 128)
            .expect("render")
            .expect("ink");
        for y in 0..128u32 {
            for x in 0..256u32 {
                if coverage[(y * 256 + x) as usize] <= 0.001 {
                    continue;
                }
                let (px, py) = (x as f32, y as f32);
                assert!(
                    px >= min_x - 1.0 && px <= max_x + 1.0,
                    "ink at x={px} lies outside the bounds {min_x}..{max_x}"
                );
                assert!(
                    py >= min_y - 1.0 && py <= max_y + 1.0,
                    "ink at y={py} lies outside the bounds {min_y}..{max_y}"
                );
            }
        }
    }

    /// A descender has to be inside the bounds; taking the bounds from the line
    /// box rather than from the ink is the classic way to clip one.
    #[test]
    fn bounds_include_descenders() {
        let mut content = sample();
        content.text = "gyp".into();
        let (_, _, _, max_y) = content
            .untransformed_bounds()
            .expect("bounds")
            .expect("some bounds");
        let mut flat = content.clone();
        flat.text = "xxx".into();
        let (_, _, _, flat_max_y) = flat
            .untransformed_bounds()
            .expect("bounds")
            .expect("some bounds");
        assert!(
            max_y > flat_max_y + 1.0,
            "descenders did not extend the bounds ({max_y} against {flat_max_y})"
        );
    }

    #[test]
    fn a_stroke_widens_the_bounds_and_adds_ink() {
        let plain = sample();
        let mut stroked = sample();
        stroked.stroke = Some(ShapeColor::new(1.0, 0.0, 0.0, 1.0));
        stroked.stroke_style = Some(StrokeStyle {
            width: 3.0,
            ..default_text_stroke()
        });

        let plain_bounds = plain.untransformed_bounds().expect("b").expect("b");
        let stroked_bounds = stroked.untransformed_bounds().expect("b").expect("b");
        assert!(stroked_bounds.0 < plain_bounds.0 && stroked_bounds.2 > plain_bounds.2);

        let (_, plain_coverage) = render_text(&plain, identity, 0, 0, 256, 128)
            .expect("render")
            .expect("ink");
        let (_, stroked_coverage) = render_text(&stroked, identity, 0, 0, 256, 128)
            .expect("render")
            .expect("ink");
        assert!(
            total_coverage(&stroked_coverage) > total_coverage(&plain_coverage) * 1.2,
            "the stroke added no ink"
        );
    }

    #[test]
    fn empty_and_invisible_text_draw_nothing() {
        let mut empty = sample();
        empty.text = "   ".into();
        assert!(!empty.draws_anything());
        assert!(render_text(&empty, identity, 0, 0, 64, 64)
            .expect("render")
            .is_none());

        let mut invisible = sample();
        invisible.fill = ShapeColor::new(0.0, 0.0, 0.0, 0.0);
        assert!(!invisible.draws_anything());
    }

    #[test]
    fn hostile_content_is_refused() {
        let mut content = sample();
        content.font_size = f32::NAN;
        assert!(content.validate().is_err());

        content = sample();
        content.font_weight = 1200;
        assert!(content.validate().is_err());

        content = sample();
        content.line_height = 0.0;
        assert!(content.validate().is_err());

        content = sample();
        content.letter_spacing = f32::INFINITY;
        assert!(content.validate().is_err());

        content = sample();
        content.origin_x = 5e9;
        assert!(content.validate().is_err());

        content = sample();
        content.wrap_width = Some(0.0);
        assert!(content.validate().is_err());

        content = sample();
        content.text = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(content.validate().is_err());

        content = sample();
        content.font_family = "f".repeat(MAX_FONT_NAME_CHARS + 1);
        assert!(content.validate().is_err());

        assert!(sample().validate().is_ok());
    }

    /// Text must survive a round trip through the project format as text, not
    /// as a picture of text.
    #[test]
    fn content_round_trips_through_serde_unchanged() {
        let mut content = sample();
        content.text = "Hello \u{0633}\u{0644}\u{0627}\u{0645}\nsecond line".into();
        content.font_family = "Some Font".into();
        content.italic = true;
        content.align = TextAlign::Center;
        content.wrap_width = Some(300.0);
        content.letter_spacing = 1.5;
        content.stroke = Some(ShapeColor::new(1.0, 0.0, 0.0, 0.5));
        content.stroke_style = Some(default_text_stroke());

        let json = serde_json::to_string(&content).expect("serialise");
        assert!(
            json.contains("Some Font"),
            "the requested font was not stored"
        );
        assert!(
            json.contains("\\u0633") || json.contains('\u{0633}'),
            "the text itself was not stored"
        );
        let restored: TextContent = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(restored, content);
    }

    /// A missing font is reported, and the request is kept verbatim.
    #[test]
    fn a_missing_font_is_reported_without_rewriting_the_request() {
        let mut content = sample();
        content.font_family = "No Such Font Exists Here 12345".into();
        assert!(content.font_is_missing());
        assert!(render_text(&content, identity, 0, 0, 256, 128)
            .expect("render")
            .is_some());
        assert_eq!(content.font_family, "No Such Font Exists Here 12345");
    }

    #[test]
    fn the_caret_follows_the_text() {
        let content = sample();
        let shaped = content.shaped().expect("shape");
        let start = caret_position(&content, &shaped, 0).expect("caret");
        let middle = caret_position(&content, &shaped, 3).expect("caret");
        let end = caret_position(&content, &shaped, content.text.len()).expect("caret");
        assert!(
            start.0 < middle.0 && middle.0 < end.0,
            "the caret did not advance"
        );
        assert!(start.2 > 0.0, "the caret had no height");

        let empty = TextContent::new("", 10.0, 20.0, 24.0);
        let shaped = empty.shaped().expect("shape");
        let caret = caret_position(&empty, &shaped, 0).expect("caret");
        assert_eq!((caret.0, caret.1), (10.0, 20.0));
    }

    /// In a right-to-left run the caret has to move leftwards as the offset
    /// grows, and sit on the right edge of the first character rather than its
    /// left. A caret that used the glyph's x regardless would land on the far
    /// side of every Persian letter.
    #[test]
    fn the_caret_runs_backwards_through_right_to_left_text() {
        let mut content = sample();
        // "salam", four letters at byte offsets 0, 2, 4 and 6.
        content.text = "\u{0633}\u{0644}\u{0627}\u{0645}".into();
        let shaped = content.shaped().expect("shape");
        assert!(
            shaped.glyphs.iter().all(|g| g.rtl),
            "the run was not right to left"
        );

        let positions: Vec<f32> = [0usize, 2, 4, 6]
            .iter()
            .map(|offset| caret_position(&content, &shaped, *offset).expect("caret").0)
            .collect();
        for pair in positions.windows(2) {
            assert!(
                pair[1] < pair[0],
                "the caret advanced rightwards through right-to-left text: {positions:?}"
            );
        }

        // The caret for the first character sits at the right edge of its
        // glyph, which is the rightmost point of the line.
        let rightmost = shaped
            .glyphs
            .iter()
            .map(|g| g.x + g.advance)
            .fold(f32::MIN, f32::max);
        assert!(
            (positions[0] - (content.origin_x + rightmost)).abs() < 0.001,
            "the caret did not start at the right edge of the line"
        );

        // And the end of the text is at the left edge.
        let end = caret_position(&content, &shaped, content.text.len())
            .expect("caret")
            .0;
        assert!(end < positions[3], "the caret did not finish on the left");
    }

    /// Mapping control points through an affine transform and then flattening
    /// must agree with flattening and then mapping — that equivalence is the
    /// whole justification for deferring the flattening.
    #[test]
    fn mapping_before_flattening_agrees_with_mapping_after() {
        let content = sample();
        let shaped = content.shaped().expect("shape");
        let glyph = shaped
            .glyphs
            .iter()
            .find(|g| !g.path.commands.is_empty())
            .expect("a glyph with an outline");
        let transform = |(x, y): (f32, f32)| (x * 2.0 + 5.0, y * 2.0 - 3.0);

        let mapped_then_flat = map_path(&glyph.path, &transform).flatten(0.001);
        let flat_then_mapped: Vec<Vec<(f32, f32)>> = glyph
            .path
            .flatten(0.0005)
            .into_iter()
            .map(|subpath| subpath.into_iter().map(transform).collect())
            .collect();

        assert_eq!(mapped_then_flat.len(), flat_then_mapped.len());
        // Both approximate the same curve, so compare enclosed area rather than
        // point lists: the two flattenings choose different point counts.
        let area = |subpaths: &[Vec<(f32, f32)>]| -> f64 {
            subpaths
                .iter()
                .map(|points| {
                    let mut sum = 0.0f64;
                    for index in 0..points.len() {
                        let (x0, y0) = points[index];
                        let (x1, y1) = points[(index + 1) % points.len()];
                        sum += f64::from(x0) * f64::from(y1) - f64::from(x1) * f64::from(y0);
                    }
                    sum / 2.0
                })
                .sum()
        };
        let (a, b) = (area(&mapped_then_flat), area(&flat_then_mapped));
        assert!(
            (a - b).abs() < a.abs().max(1.0) * 0.001,
            "the two orders disagreed: {a} against {b}"
        );
    }
}
