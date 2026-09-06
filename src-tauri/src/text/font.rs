//! System font discovery, and the glyph outlines it yields.
//!
//! # Discovery happens once
//!
//! Enumerating the installed fonts took 871 ms and found 397 faces on the
//! machine this was written on. That is far too slow to repeat while somebody
//! is typing, so the font system is built once on first use and shared.
//!
//! # Fonts are not bundled
//!
//! PhotoForge reads the fonts already installed on the machine and ships none
//! of its own. Copying a font out of Windows and into an installer would be
//! redistributing someone else's licensed work.
//!
//! # Outlines, not bitmaps
//!
//! swash will happily rasterise a glyph, and hands back an 8-bit alpha mask.
//! PhotoForge takes the outline instead and rasterises it with the same float
//! coverage rasteriser vector shapes use. That keeps text and shapes on one
//! implementation — so one region-independence guarantee covers both — and
//! keeps the coverage in f32 rather than quantised to 1/255.
use std::sync::{Mutex, MutexGuard, OnceLock};

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Style, Weight};

use crate::error::AppError;
use crate::vector::{PathCommand, VectorPath};

/// Longest text a single layer may hold, in bytes.
///
/// Project files are untrusted and shaping cost grows with length, so the text
/// a document can ask to lay out needs a ceiling.
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
/// Largest font size accepted, in document pixels.
pub const MAX_FONT_SIZE: f32 = 2_000.0;
/// Smallest font size that still produces geometry.
pub const MIN_FONT_SIZE: f32 = 0.5;
/// Longest font family name kept.
pub const MAX_FONT_NAME_CHARS: usize = 120;

/// The shared font system.
///
/// Behind a mutex because cosmic-text needs `&mut` to shape — it caches
/// per-font data as it goes — and behind a `OnceLock` because discovery is
/// expensive and its result does not change while the application runs.
fn font_system() -> MutexGuard<'static, FontSystem> {
    static SYSTEM: OnceLock<Mutex<FontSystem>> = OnceLock::new();
    SYSTEM
        .get_or_init(|| Mutex::new(FontSystem::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Family names installed on this machine, sorted, for the font picker.
pub fn available_families() -> Vec<String> {
    let system = font_system();
    let mut names: Vec<String> = system
        .db()
        .faces()
        .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Whether a family is installed under exactly this name.
///
/// Used to tell the user their project asked for a font this machine does not
/// have, rather than silently drawing it in something else.
pub fn family_is_available(family: &str) -> bool {
    let system = font_system();
    let present = system
        .db()
        .faces()
        .any(|face| face.families.iter().any(|(name, _)| name == family));
    present
}

/// One glyph, positioned and outlined.
pub struct PositionedGlyph {
    /// The glyph's outline in font units already scaled to the requested size,
    /// with the origin at the glyph's own origin and y increasing downwards.
    pub path: VectorPath,
    /// Where the glyph's origin sits in the text block's coordinate space.
    pub x: f32,
    pub y: f32,
    /// How far this glyph advances the pen, in the same space.
    pub advance: f32,
    /// The byte range of the source text this glyph came from.
    ///
    /// Kept because a glyph's position says nothing about which character it
    /// came from once the bidirectional algorithm has reordered a line, and
    /// caret placement, hit testing and selection all need that mapping.
    pub start: usize,
    pub end: usize,
    /// Which visual line the glyph sits on, counting from zero.
    pub line: usize,
}

/// The result of laying out a piece of text.
pub struct ShapedText {
    pub glyphs: Vec<PositionedGlyph>,
    /// The block's extent, in its own coordinate space.
    pub width: f32,
    pub height: f32,
    /// True when the requested family was not installed and something else was
    /// used. The project keeps the original request either way.
    pub fell_back: bool,
}

/// How text is aligned inside its box.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "camelCase")]
pub enum TextAlign {
    #[default]
    Start,
    Center,
    End,
    Justified,
}

impl TextAlign {
    const fn to_cosmic(self) -> cosmic_text::Align {
        match self {
            Self::Start => cosmic_text::Align::Left,
            Self::Center => cosmic_text::Align::Center,
            Self::End => cosmic_text::Align::Right,
            Self::Justified => cosmic_text::Align::Justified,
        }
    }
}

/// Everything shaping needs.
pub struct ShapeRequest<'a> {
    pub text: &'a str,
    pub family: &'a str,
    pub size: f32,
    pub line_height: f32,
    pub letter_spacing: f32,
    pub weight: u16,
    pub italic: bool,
    pub align: TextAlign,
    /// The wrapping width, or `None` for a single unwrapped line.
    pub wrap_width: Option<f32>,
}

/// Lays out text and returns its glyph outlines.
///
/// Shaping is cosmic-text's: it applies the Unicode bidirectional algorithm,
/// picks contextual forms, and falls back across faces for characters the
/// requested family cannot draw. PhotoForge does not position code points
/// itself, which is the only way Arabic, Persian and Devanagari come out right.
pub fn shape(request: &ShapeRequest<'_>) -> Result<ShapedText, AppError> {
    if request.text.len() > MAX_TEXT_BYTES {
        return Err(AppError::InvalidLayerDocument(format!(
            "a text layer may hold at most {MAX_TEXT_BYTES} bytes"
        )));
    }
    if !request.size.is_finite() || !(MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&request.size) {
        return Err(AppError::InvalidLayerDocument(
            "the font size is outside the supported range".into(),
        ));
    }
    if !request.line_height.is_finite() || !(0.1..=10.0).contains(&request.line_height) {
        return Err(AppError::InvalidLayerDocument(
            "the line height is outside the supported range".into(),
        ));
    }

    let fell_back = !request.family.is_empty() && !family_is_available(request.family);
    let mut system = font_system();
    let metrics = Metrics::new(request.size, request.size * request.line_height);
    let mut buffer = Buffer::new(&mut system, metrics);
    buffer.set_size(request.wrap_width, None);

    let family = if request.family.is_empty() || fell_back {
        // Deliberately the generic sans-serif rather than a named font: the
        // project keeps what was asked for, and substituting a specific face
        // here would make the substitution look like a choice.
        Family::SansSerif
    } else {
        Family::Name(request.family)
    };
    let attrs = Attrs::new()
        .family(family)
        .weight(Weight(request.weight))
        .style(if request.italic {
            Style::Italic
        } else {
            Style::Normal
        })
        .letter_spacing(request.letter_spacing);
    buffer.set_text(
        request.text,
        &attrs,
        Shaping::Advanced,
        Some(request.align.to_cosmic()),
    );
    buffer.shape_until_scroll(&mut system, false);

    let mut context = swash::scale::ScaleContext::new();
    let mut glyphs = Vec::new();
    let (mut width, mut height) = (0.0f32, 0.0f32);
    for (line, run) in buffer.layout_runs().enumerate() {
        width = width.max(run.line_w);
        height = height.max(run.line_top + run.line_height);
        for glyph in run.glyphs.iter() {
            let Some(font) = system.get_font(glyph.font_id, glyph.font_weight) else {
                continue;
            };
            let mut scaler = context
                .builder(font.as_swash())
                .size(glyph.font_size)
                // Unhinted, because hinting snaps outlines to a pixel grid that
                // only exists at one scale — and a text layer here can be
                // transformed to any scale after the fact.
                .hint(false)
                .build();
            let Some(outline) = scaler.scale_outline(glyph.glyph_id) else {
                continue;
            };
            glyphs.push(PositionedGlyph {
                path: outline_to_path(&outline),
                x: glyph.x,
                y: run.line_y + glyph.y,
                advance: glyph.w,
                start: glyph.start,
                end: glyph.end,
                line,
            });
        }
    }
    Ok(ShapedText {
        glyphs,
        width,
        height,
        fell_back,
    })
}

/// Converts a swash outline into PhotoForge's path model.
///
/// swash reports y increasing upwards, as font formats do; the document has y
/// increasing downwards, so every y is negated here rather than at each use.
fn outline_to_path(outline: &swash::scale::outline::Outline) -> VectorPath {
    use swash::zeno::{Command, PathData};
    let mut commands = Vec::new();
    for command in outline.path().commands() {
        match command {
            Command::MoveTo(p) => commands.push(PathCommand::MoveTo { x: p.x, y: -p.y }),
            Command::LineTo(p) => commands.push(PathCommand::LineTo { x: p.x, y: -p.y }),
            Command::CurveTo(c1, c2, p) => commands.push(PathCommand::CubicTo {
                c1x: c1.x,
                c1y: -c1.y,
                c2x: c2.x,
                c2y: -c2.y,
                x: p.x,
                y: -p.y,
            }),
            Command::QuadTo(control, p) => {
                // Raised to a cubic so the renderer has one curve type. The
                // control points sit two thirds of the way from each endpoint
                // to the quadratic's control, which is exact rather than an
                // approximation.
                let start = last_point(&commands);
                commands.push(PathCommand::CubicTo {
                    c1x: start.0 + 2.0 / 3.0 * (control.x - start.0),
                    c1y: start.1 + 2.0 / 3.0 * (-control.y - start.1),
                    c2x: p.x + 2.0 / 3.0 * (control.x - p.x),
                    c2y: -p.y + 2.0 / 3.0 * (-control.y - -p.y),
                    x: p.x,
                    y: -p.y,
                });
            }
            Command::Close => commands.push(PathCommand::Close),
        }
    }
    VectorPath::new(commands)
}

fn last_point(commands: &[PathCommand]) -> (f32, f32) {
    match commands.last() {
        Some(PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y }) => (*x, *y),
        Some(PathCommand::CubicTo { x, y, .. }) => (*x, *y),
        _ => (0.0, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(text: &'a str, family: &'a str) -> ShapeRequest<'a> {
        ShapeRequest {
            text,
            family,
            size: 32.0,
            line_height: 1.25,
            letter_spacing: 0.0,
            weight: 400,
            italic: false,
            align: TextAlign::Start,
            wrap_width: None,
        }
    }

    /// Discovery has to find something, or every other text test is vacuous.
    #[test]
    fn the_system_has_fonts() {
        let families = available_families();
        assert!(
            families.len() > 5,
            "only {} font families were discovered",
            families.len()
        );
    }

    #[test]
    fn latin_text_produces_one_glyph_per_letter() {
        let shaped = shape(&request("Hello", "")).expect("shape");
        assert_eq!(shaped.glyphs.len(), 5);
        assert!(shaped.width > 0.0 && shaped.height > 0.0);
        // Left to right.
        for pair in shaped.glyphs.windows(2) {
            assert!(pair[1].x >= pair[0].x, "latin text was not laid out forwards");
        }
        // Every glyph produced real geometry.
        assert!(shaped.glyphs.iter().all(|g| !g.path.commands.is_empty()));
    }

    /// Glyphs left to right, whatever order shaping happened to report them in.
    fn visual_order(shaped: &ShapedText) -> Vec<&PositionedGlyph> {
        let mut glyphs: Vec<&PositionedGlyph> = shaped.glyphs.iter().collect();
        glyphs.sort_by(|a, b| a.x.total_cmp(&b.x));
        glyphs
    }

    /// The claim that must not be made lightly: Persian is *shaped*, not merely
    /// rendered.
    ///
    /// Two independent things have to be true. The line must be reordered, so
    /// the first character typed ends up rightmost — checked against source
    /// byte offsets rather than glyph order, because the order glyphs are
    /// reported in is a layout detail and not the claim. And the letters must
    /// join, so the connected forms differ from the isolated ones.
    #[test]
    fn persian_is_shaped_right_to_left_with_contextual_forms() {
        // "salam"
        let joined = shape(&request("\u{0633}\u{0644}\u{0627}\u{0645}", "")).expect("shape");
        assert_eq!(joined.glyphs.len(), 4, "expected four Persian glyphs");
        let ordered = visual_order(&joined);
        for pair in ordered.windows(2) {
            assert!(
                pair[1].start < pair[0].start,
                "the leftmost glyph came from an earlier byte than the one to its right, \
                 so the line was not reordered right to left"
            );
        }

        // The same letters separated by spaces cannot join, so their outlines
        // must differ from the joined ones.
        let isolated = shape(&request("\u{0633} \u{0644} \u{0627} \u{0645}", "")).expect("shape");
        let joined_shapes: Vec<usize> =
            joined.glyphs.iter().map(|g| g.path.commands.len()).collect();
        let isolated_shapes: Vec<usize> = isolated
            .glyphs
            .iter()
            .filter(|g| !g.path.commands.is_empty())
            .map(|g| g.path.commands.len())
            .collect();
        assert_ne!(
            joined_shapes, isolated_shapes,
            "joined and isolated Persian produced identical outlines, so no contextual \
             substitution happened"
        );
    }

    /// Mixed direction has to obey the bidirectional algorithm, not just
    /// concatenate runs.
    #[test]
    fn mixed_direction_text_is_reordered() {
        // "Hi " occupies bytes 0..3; the Persian follows.
        let shaped = shape(&request("Hi \u{0633}\u{0644}\u{0627}\u{0645}", "")).expect("shape");
        let ordered = visual_order(&shaped);
        let latin: Vec<&PositionedGlyph> = ordered
            .iter()
            .copied()
            .filter(|g| g.start < 3 && !g.path.commands.is_empty())
            .collect();
        let arabic: Vec<&PositionedGlyph> =
            ordered.iter().copied().filter(|g| g.start >= 3).collect();
        assert_eq!(latin.len(), 2, "the Latin prefix did not produce two glyphs");
        assert_eq!(arabic.len(), 4, "the Persian did not produce four glyphs");

        // The paragraph reads left to right, so the Latin sits to the left of
        // the Persian.
        assert!(
            latin.iter().all(|l| arabic.iter().all(|a| l.x < a.x)),
            "the Persian was not placed after the Latin"
        );
        // Latin runs forwards.
        for pair in latin.windows(2) {
            assert!(pair[1].start > pair[0].start, "the Latin run was reversed");
        }
        // And the embedded Persian runs backwards inside it.
        for pair in arabic.windows(2) {
            assert!(
                pair[1].start < pair[0].start,
                "the embedded Persian run was not reversed for display"
            );
        }
    }

    #[test]
    fn cjk_and_combining_marks_survive_shaping() {
        let cjk = shape(&request("\u{4F60}\u{597D}", "")).expect("shape");
        assert_eq!(cjk.glyphs.len(), 2, "CJK did not produce two glyphs");

        // e + combining acute + combining dot below: one base, marks attached.
        let combining = shape(&request("e\u{0301}\u{0323}", "")).expect("shape");
        assert!(
            combining.glyphs.len() >= 2,
            "combining marks were dropped rather than attached"
        );
    }

    /// A missing font must be reported, not silently swapped.
    #[test]
    fn a_missing_family_is_reported_and_still_renders() {
        let shaped = shape(&request("Hello", "No Such Font Exists Here 12345")).expect("shape");
        assert!(shaped.fell_back, "a missing family was not reported");
        assert_eq!(shaped.glyphs.len(), 5, "the fallback drew nothing");

        let present = available_families();
        if let Some(installed) = present.first() {
            let shaped = shape(&request("Hello", installed)).expect("shape");
            assert!(!shaped.fell_back, "{installed} was reported as missing");
        }
    }

    #[test]
    fn hostile_text_parameters_are_refused() {
        let huge = "x".repeat(MAX_TEXT_BYTES + 1);
        assert!(shape(&request(&huge, "")).is_err(), "an oversized string was shaped");

        for size in [f32::NAN, f32::INFINITY, 0.0, -10.0, 1e9] {
            let mut candidate = request("Hi", "");
            candidate.size = size;
            assert!(shape(&candidate).is_err(), "font size {size} was accepted");
        }
        for line_height in [f32::NAN, 0.0, 1e6] {
            let mut candidate = request("Hi", "");
            candidate.line_height = line_height;
            assert!(
                shape(&candidate).is_err(),
                "line height {line_height} was accepted"
            );
        }
    }

    /// Empty text is a legitimate state — a caret with nothing typed yet.
    #[test]
    fn empty_text_shapes_to_nothing_without_failing() {
        let shaped = shape(&request("", "")).expect("shape");
        assert!(shaped.glyphs.is_empty());
    }

    /// Every outline must be finite, or the rasteriser inherits a NaN.
    #[test]
    fn outlines_are_finite() {
        let shaped = shape(&request("Ag\u{0633}\u{4F60}", "")).expect("shape");
        for glyph in &shaped.glyphs {
            assert!(glyph.x.is_finite() && glyph.y.is_finite());
            glyph.path.validate().expect("a glyph outline was not valid");
        }
    }
}
