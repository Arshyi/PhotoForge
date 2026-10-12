//! Reading a bounded subset of SVG as shape layers.
//!
//! An SVG is text from a stranger, and a careless reader is a network client, a script host and an
//! entity-expansion bomb at once. This one is none of them: the document is parsed by a parser that
//! refuses a DTD and stops at a node limit, an external reference is never followed, a script is
//! never run, and everything the subset does not cover is **left out and named in the report**,
//! never guessed at. What is read is shapes and groups: rectangles, circles, ellipses, lines,
//! polylines, polygons and paths, with fill, stroke, opacity and transforms.
//!
//! A transform is baked into the geometry where the layer model cannot hold it (a rotation, a skew),
//! and kept as a plain move or axis-aligned scale otherwise, so that a rectangle stays a rectangle
//! the person can still edit.
use crate::color::srgb_decode;
use crate::error::AppError;
use crate::layers::{
    validate_dimensions, Layer, LayerContent, LayerDocument, LoadedProject, ShapeColor,
    ShapeContent, MAX_GROUP_DEPTH, MAX_LAYERS,
};
use crate::operations::structure::new_layer;
use crate::operations::support::{Clock, SystemClock};
use crate::pixel::DocumentPrecision;
use crate::psd::{ReportEntry, ReportKind};
use crate::vector::{
    FillRule, LineCap, LineJoin, PathCommand, ShapeGeometry, StrokeStyle, VectorPath,
    MAX_PATH_COMMANDS,
};
use roxmltree::{Document, Node, NodeId, ParsingOptions};
use serde::Serialize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

/// The largest SVG file read.
pub const MAX_SVG_BYTES: u64 = 8 * 1024 * 1024;
const MAX_NODES: u32 = 200_000;
/// The deepest nesting of elements read, the root `<svg>` included. The parser and the walk both
/// recurse once per level, and the node limit alone would let a file ask for 200,000 levels. Groups
/// past `MAX_GROUP_DEPTH` are flattened, not refused, so a real drawing may nest well beyond it;
/// 16 times that is far past any drawing, and it takes under half of a worker thread's 2 MiB stack
/// in an unoptimised build (about 3.6 KiB a level, measured), where the test for it runs.
const MAX_ELEMENT_DEPTH: usize = 256;
/// The largest edge of the canvas an SVG may ask for, in pixels.
const MAX_EDGE: f64 = 16_384.0;

/// What an import did, in the shape the Photoshop import report has, so one dialog shows both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SvgImportReport {
    /// Always "SVG".
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub layers_imported: usize,
    pub groups_imported: usize,
    pub entries: Vec<ReportEntry>,
}

impl SvgImportReport {
    pub fn count(&self, kind: ReportKind) -> usize {
        self.entries.iter().filter(|e| e.kind == kind).count()
    }
}

/// An imported document, as the project loader would hand it over.
pub struct ImportedSvg {
    pub project: LoadedProject,
    pub report: SvgImportReport,
}

// ---------------------------------------------------------------------------------------------
// Affine transforms
// ---------------------------------------------------------------------------------------------

/// `x' = a x + c y + e`, `y' = b x + d y + f`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Affine {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    e: f64,
    f: f64,
}

impl Affine {
    const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// `self` applied after `other`: the point goes through `other` first.
    fn then_after(self, other: Self) -> Self {
        Self {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }

    fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    fn is_finite(&self) -> bool {
        [self.a, self.b, self.c, self.d, self.e, self.f]
            .iter()
            .all(|v| v.is_finite())
    }

    /// No rotation and no skew: axis-aligned scales and a move, which a rectangle survives.
    fn is_axis_aligned(&self) -> bool {
        self.b.abs() < 1e-9 && self.c.abs() < 1e-9
    }

    /// The factor a length is scaled by on average (the geometric mean of the two axes).
    fn mean_scale(&self) -> f64 {
        (self.a * self.d - self.b * self.c).abs().sqrt()
    }

    fn is_isotropic(&self) -> bool {
        let sx = (self.a * self.a + self.b * self.b).sqrt();
        let sy = (self.c * self.c + self.d * self.d).sqrt();
        (sx - sy).abs() <= 1e-6 * sx.max(sy).max(1.0)
    }
}

/// A scanner over the characters of a list of numbers or a path.
struct Scanner<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            at: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip_separators(&mut self) {
        while let Some(byte) = self.peek() {
            if byte.is_ascii_whitespace() || byte == b',' {
                self.at += 1;
            } else {
                break;
            }
        }
    }

    fn at_end(&mut self) -> bool {
        self.skip_separators();
        self.at >= self.bytes.len()
    }

    /// A number: sign, digits, a point, digits, an exponent.
    fn number(&mut self) -> Option<f64> {
        self.skip_separators();
        let start = self.at;
        let mut at = self.at;
        let bytes = self.bytes;
        if matches!(bytes.get(at), Some(b'+' | b'-')) {
            at += 1;
        }
        let mut digits = 0;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
            digits += 1;
        }
        if bytes.get(at) == Some(&b'.') {
            at += 1;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
                digits += 1;
            }
        }
        if digits == 0 {
            return None;
        }
        if matches!(bytes.get(at), Some(b'e' | b'E')) {
            let mut look = at + 1;
            if matches!(bytes.get(look), Some(b'+' | b'-')) {
                look += 1;
            }
            if bytes.get(look).is_some_and(u8::is_ascii_digit) {
                while bytes.get(look).is_some_and(u8::is_ascii_digit) {
                    look += 1;
                }
                at = look;
            }
        }
        let text = std::str::from_utf8(&bytes[start..at]).ok()?;
        self.at = at;
        text.parse::<f64>().ok().filter(|v| v.is_finite())
    }

    /// An arc flag: one character, 0 or 1, with no separator needed after it.
    fn flag(&mut self) -> Option<bool> {
        self.skip_separators();
        match self.peek() {
            Some(b'0') => {
                self.at += 1;
                Some(false)
            }
            Some(b'1') => {
                self.at += 1;
                Some(true)
            }
            _ => None,
        }
    }
}

/// A transform list: `translate(10 20) rotate(45) matrix(…)`.
fn parse_transform(text: &str) -> Option<Affine> {
    let mut result = Affine::IDENTITY;
    let mut rest = text.trim();
    while !rest.is_empty() {
        let open = rest.find('(')?;
        let close = rest.find(')')?;
        if close < open {
            return None;
        }
        let name = rest[..open].trim();
        let mut scanner = Scanner::new(&rest[open + 1..close]);
        let mut values = Vec::new();
        while !scanner.at_end() {
            values.push(scanner.number()?);
        }
        let op = match (name, values.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Affine {
                a: *a,
                b: *b,
                c: *c,
                d: *d,
                e: *e,
                f: *f,
            },
            ("translate", [x]) => Affine {
                e: *x,
                ..Affine::IDENTITY
            },
            ("translate", [x, y]) => Affine {
                e: *x,
                f: *y,
                ..Affine::IDENTITY
            },
            ("scale", [s]) => Affine {
                a: *s,
                d: *s,
                ..Affine::IDENTITY
            },
            ("scale", [x, y]) => Affine {
                a: *x,
                d: *y,
                ..Affine::IDENTITY
            },
            ("rotate", [angle]) => rotation(*angle, 0.0, 0.0),
            ("rotate", [angle, cx, cy]) => rotation(*angle, *cx, *cy),
            ("skewX", [angle]) => Affine {
                c: angle.to_radians().tan(),
                ..Affine::IDENTITY
            },
            ("skewY", [angle]) => Affine {
                b: angle.to_radians().tan(),
                ..Affine::IDENTITY
            },
            _ => return None,
        };
        // A later operation is applied to the point first.
        result = result.then_after(op);
        rest = rest[close + 1..].trim_start_matches([' ', '\t', '\r', '\n', ',']);
    }
    result.is_finite().then_some(result)
}

fn rotation(degrees: f64, cx: f64, cy: f64) -> Affine {
    let (sin, cos) = degrees.to_radians().sin_cos();
    Affine {
        a: cos,
        b: sin,
        c: -sin,
        d: cos,
        e: cx - cos * cx + sin * cy,
        f: cy - sin * cx - cos * cy,
    }
}

// ---------------------------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------------------------

/// An SVG arc as cubic Béziers, in at most quarter-turn pieces (SVG 1.1, appendix F.6).
#[allow(clippy::too_many_arguments)]
fn arc_to_cubics(
    from: (f64, f64),
    mut rx: f64,
    mut ry: f64,
    rotation_degrees: f64,
    large: bool,
    sweep: bool,
    to: (f64, f64),
) -> Vec<[f64; 6]> {
    if from == to {
        return Vec::new();
    }
    rx = rx.abs();
    ry = ry.abs();
    if rx < 1e-9 || ry < 1e-9 {
        // A degenerate arc is a straight line.
        return vec![[from.0, from.1, to.0, to.1, to.0, to.1]];
    }
    let (sin_phi, cos_phi) = rotation_degrees.to_radians().sin_cos();
    let dx = (from.0 - to.0) / 2.0;
    let dy = (from.1 - to.1) / 2.0;
    let x1 = cos_phi * dx + sin_phi * dy;
    let y1 = -sin_phi * dx + cos_phi * dy;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        let scale = lambda.sqrt();
        rx *= scale;
        ry *= scale;
    }
    let numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let mut factor = if denominator.abs() < 1e-12 {
        0.0
    } else {
        (numerator / denominator).max(0.0).sqrt()
    };
    if large == sweep {
        factor = -factor;
    }
    let cxp = factor * rx * y1 / ry;
    let cyp = -factor * ry * x1 / rx;
    let cx = cos_phi * cxp - sin_phi * cyp + (from.0 + to.0) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (from.1 + to.1) / 2.0;
    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| {
        let dot = ux * vx + uy * vy;
        let length = (ux * ux + uy * uy).sqrt() * (vx * vx + vy * vy).sqrt();
        let mut value = (dot / length).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            value = -value;
        }
        value
    };
    let theta1 = angle(1.0, 0.0, (x1 - cxp) / rx, (y1 - cyp) / ry);
    let mut delta = angle(
        (x1 - cxp) / rx,
        (y1 - cyp) / ry,
        (-x1 - cxp) / rx,
        (-y1 - cyp) / ry,
    );
    if !sweep && delta > 0.0 {
        delta -= std::f64::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f64::consts::TAU;
    }
    let pieces = (delta.abs() / (std::f64::consts::FRAC_PI_2 + 1e-9))
        .ceil()
        .max(1.0) as usize;
    let step = delta / pieces as f64;
    let t = 4.0 / 3.0 * (step / 4.0).tan();
    let point = |theta: f64| -> (f64, f64, f64, f64) {
        let (s, c) = theta.sin_cos();
        // The point and its derivative, in the unrotated ellipse frame, then rotated and moved.
        let (px, py) = (rx * c, ry * s);
        let (dx, dy) = (-rx * s, ry * c);
        (
            cos_phi * px - sin_phi * py + cx,
            sin_phi * px + cos_phi * py + cy,
            cos_phi * dx - sin_phi * dy,
            sin_phi * dx + cos_phi * dy,
        )
    };
    let mut out = Vec::with_capacity(pieces);
    let mut theta = theta1;
    for _ in 0..pieces {
        let (x0, y0, dx0, dy0) = point(theta);
        let (x2, y2, dx2, dy2) = point(theta + step);
        out.push([
            x0 + t * dx0,
            y0 + t * dy0,
            x2 - t * dx2,
            y2 - t * dy2,
            x2,
            y2,
        ]);
        theta += step;
    }
    // The last piece ends exactly where the arc was asked to.
    if let Some(last) = out.last_mut() {
        last[4] = to.0;
        last[5] = to.1;
    }
    out
}

/// Path data as commands in user space, up to the first error. `Err` carries what was read.
fn parse_path_data(data: &str) -> (Vec<PathCommand>, bool) {
    let mut scanner = Scanner::new(data);
    let mut commands: Vec<PathCommand> = Vec::new();
    let (mut x, mut y) = (0.0_f64, 0.0_f64);
    let (mut start_x, mut start_y) = (0.0_f64, 0.0_f64);
    // The last control point, reflected by a smooth curve.
    let mut last_cubic: Option<(f64, f64)> = None;
    let mut last_quad: Option<(f64, f64)> = None;
    let mut command: Option<u8> = None;
    let mut first = true;
    let mut ok = true;
    // After a close, a drawing command carries on from the start of the subpath it closed.
    let mut after_close = false;

    macro_rules! num {
        () => {
            match scanner.number() {
                Some(value) => value,
                None => {
                    ok = false;
                    break;
                }
            }
        };
    }

    loop {
        if scanner.at_end() {
            break;
        }
        if commands.len() > MAX_PATH_COMMANDS {
            ok = false;
            break;
        }
        let byte = scanner.peek().unwrap_or(0);
        if byte.is_ascii_alphabetic() {
            command = Some(byte);
            scanner.at += 1;
        } else if command.is_none() {
            ok = false;
            break;
        } else if matches!(command, Some(b'Z' | b'z')) {
            // Numbers after a close are an error.
            ok = false;
            break;
        }
        let Some(cmd) = command else { break };
        if first && !matches!(cmd, b'M' | b'm') {
            ok = false;
            break;
        }
        first = false;
        let relative = cmd.is_ascii_lowercase();
        let (ox, oy) = if relative { (x, y) } else { (0.0, 0.0) };
        if after_close && !matches!(cmd, b'M' | b'm' | b'Z' | b'z') {
            commands.push(PathCommand::MoveTo {
                x: start_x as f32,
                y: start_y as f32,
            });
        }
        after_close = matches!(cmd, b'Z' | b'z');
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let nx = num!() + ox;
                let ny = num!() + oy;
                commands.push(PathCommand::MoveTo {
                    x: nx as f32,
                    y: ny as f32,
                });
                (x, y) = (nx, ny);
                (start_x, start_y) = (nx, ny);
                // Further pairs are implicit line-tos, relative if the command was.
                command = Some(if relative { b'l' } else { b'L' });
                last_cubic = None;
                last_quad = None;
            }
            b'L' => {
                let nx = num!() + ox;
                let ny = num!() + oy;
                commands.push(PathCommand::LineTo {
                    x: nx as f32,
                    y: ny as f32,
                });
                (x, y) = (nx, ny);
                last_cubic = None;
                last_quad = None;
            }
            b'H' => {
                let nx = num!() + ox;
                commands.push(PathCommand::LineTo {
                    x: nx as f32,
                    y: y as f32,
                });
                x = nx;
                last_cubic = None;
                last_quad = None;
            }
            b'V' => {
                let ny = num!() + oy;
                commands.push(PathCommand::LineTo {
                    x: x as f32,
                    y: ny as f32,
                });
                y = ny;
                last_cubic = None;
                last_quad = None;
            }
            b'C' => {
                let c1 = (num!() + ox, num!() + oy);
                let c2 = (num!() + ox, num!() + oy);
                let end = (num!() + ox, num!() + oy);
                commands.push(cubic(c1, c2, end));
                last_cubic = Some(c2);
                last_quad = None;
                (x, y) = end;
            }
            b'S' => {
                let c1 = match last_cubic {
                    Some((px, py)) => (2.0 * x - px, 2.0 * y - py),
                    None => (x, y),
                };
                let c2 = (num!() + ox, num!() + oy);
                let end = (num!() + ox, num!() + oy);
                commands.push(cubic(c1, c2, end));
                last_cubic = Some(c2);
                last_quad = None;
                (x, y) = end;
            }
            b'Q' => {
                let control = (num!() + ox, num!() + oy);
                let end = (num!() + ox, num!() + oy);
                commands.push(quad_to_cubic((x, y), control, end));
                last_quad = Some(control);
                last_cubic = None;
                (x, y) = end;
            }
            b'T' => {
                let control = match last_quad {
                    Some((px, py)) => (2.0 * x - px, 2.0 * y - py),
                    None => (x, y),
                };
                let end = (num!() + ox, num!() + oy);
                commands.push(quad_to_cubic((x, y), control, end));
                last_quad = Some(control);
                last_cubic = None;
                (x, y) = end;
            }
            b'A' => {
                let rx = num!();
                let ry = num!();
                let rotation = num!();
                let Some(large) = scanner.flag() else {
                    ok = false;
                    break;
                };
                let Some(sweep) = scanner.flag() else {
                    ok = false;
                    break;
                };
                let end = (num!() + ox, num!() + oy);
                for piece in arc_to_cubics((x, y), rx, ry, rotation, large, sweep, end) {
                    commands.push(cubic(
                        (piece[0], piece[1]),
                        (piece[2], piece[3]),
                        (piece[4], piece[5]),
                    ));
                }
                (x, y) = end;
                last_cubic = None;
                last_quad = None;
            }
            b'Z' => {
                commands.push(PathCommand::Close);
                (x, y) = (start_x, start_y);
                last_cubic = None;
                last_quad = None;
            }
            _ => {
                ok = false;
                break;
            }
        }
    }
    (commands, ok)
}

fn cubic(c1: (f64, f64), c2: (f64, f64), end: (f64, f64)) -> PathCommand {
    PathCommand::CubicTo {
        c1x: c1.0 as f32,
        c1y: c1.1 as f32,
        c2x: c2.0 as f32,
        c2y: c2.1 as f32,
        x: end.0 as f32,
        y: end.1 as f32,
    }
}

fn quad_to_cubic(from: (f64, f64), control: (f64, f64), end: (f64, f64)) -> PathCommand {
    let c1 = (
        from.0 + 2.0 / 3.0 * (control.0 - from.0),
        from.1 + 2.0 / 3.0 * (control.1 - from.1),
    );
    let c2 = (
        end.0 + 2.0 / 3.0 * (control.0 - end.0),
        end.1 + 2.0 / 3.0 * (control.1 - end.1),
    );
    cubic(c1, c2, end)
}

/// A path's commands, every point sent through `matrix`.
fn transformed(commands: &[PathCommand], matrix: &Affine) -> Vec<PathCommand> {
    let point = |x: f32, y: f32| {
        let (px, py) = matrix.apply(f64::from(x), f64::from(y));
        (px as f32, py as f32)
    };
    commands
        .iter()
        .map(|command| match *command {
            PathCommand::MoveTo { x, y } => {
                let (x, y) = point(x, y);
                PathCommand::MoveTo { x, y }
            }
            PathCommand::LineTo { x, y } => {
                let (x, y) = point(x, y);
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
                let (c1x, c1y) = point(c1x, c1y);
                let (c2x, c2y) = point(c2x, c2y);
                let (x, y) = point(x, y);
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
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Colours and styles
// ---------------------------------------------------------------------------------------------

/// A colour as written: encoded sRGB, 0 to 1, with alpha.
type Srgba = [f32; 4];

const NAMED: [(&str, u32); 40] = [
    ("black", 0x000000),
    ("white", 0xffffff),
    ("red", 0xff0000),
    ("green", 0x008000),
    ("blue", 0x0000ff),
    ("yellow", 0xffff00),
    ("cyan", 0x00ffff),
    ("aqua", 0x00ffff),
    ("magenta", 0xff00ff),
    ("fuchsia", 0xff00ff),
    ("gray", 0x808080),
    ("grey", 0x808080),
    ("silver", 0xc0c0c0),
    ("maroon", 0x800000),
    ("olive", 0x808000),
    ("lime", 0x00ff00),
    ("teal", 0x008080),
    ("navy", 0x000080),
    ("purple", 0x800080),
    ("orange", 0xffa500),
    ("pink", 0xffc0cb),
    ("brown", 0xa52a2a),
    ("gold", 0xffd700),
    ("indigo", 0x4b0082),
    ("violet", 0xee82ee),
    ("turquoise", 0x40e0d0),
    ("tan", 0xd2b48c),
    ("khaki", 0xf0e68c),
    ("crimson", 0xdc143c),
    ("coral", 0xff7f50),
    ("salmon", 0xfa8072),
    ("skyblue", 0x87ceeb),
    ("steelblue", 0x4682b4),
    ("royalblue", 0x4169e1),
    ("darkgray", 0xa9a9a9),
    ("darkgrey", 0xa9a9a9),
    ("lightgray", 0xd3d3d3),
    ("lightgrey", 0xd3d3d3),
    ("darkgreen", 0x006400),
    ("darkblue", 0x00008b),
];

fn parse_colour(text: &str) -> Option<Srgba> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    if lower == "transparent" {
        return Some([0.0, 0.0, 0.0, 0.0]);
    }
    if let Some(hex) = lower.strip_prefix('#') {
        let digit = |c: char| c.to_digit(16).map(|d| d as f32);
        let chars: Vec<char> = hex.chars().collect();
        let value = |i: usize, j: usize| -> Option<f32> {
            Some((digit(chars[i])? * 16.0 + digit(chars[j])?) / 255.0)
        };
        return match chars.len() {
            3 | 4 => {
                let channel = |i: usize| digit(chars[i]).map(|d| d * 17.0 / 255.0);
                Some([
                    channel(0)?,
                    channel(1)?,
                    channel(2)?,
                    if chars.len() == 4 { channel(3)? } else { 1.0 },
                ])
            }
            6 | 8 => Some([
                value(0, 1)?,
                value(2, 3)?,
                value(4, 5)?,
                if chars.len() == 8 { value(6, 7)? } else { 1.0 },
            ]),
            _ => None,
        };
    }
    if let Some(body) = lower
        .strip_prefix("rgba(")
        .or_else(|| lower.strip_prefix("rgb("))
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let parts: Vec<&str> = body
            .split([',', ' ', '/'])
            .filter(|part| !part.is_empty())
            .collect();
        if parts.len() != 3 && parts.len() != 4 {
            return None;
        }
        let channel = |part: &str| -> Option<f32> {
            match part.strip_suffix('%') {
                Some(percent) => percent.parse::<f32>().ok().map(|v| v / 100.0),
                None => part.parse::<f32>().ok().map(|v| v / 255.0),
            }
            .map(|v| v.clamp(0.0, 1.0))
        };
        let alpha = match parts.get(3) {
            Some(part) => match part.strip_suffix('%') {
                Some(percent) => percent.parse::<f32>().ok()? / 100.0,
                None => part.parse::<f32>().ok()?,
            }
            .clamp(0.0, 1.0),
            None => 1.0,
        };
        return Some([
            channel(parts[0])?,
            channel(parts[1])?,
            channel(parts[2])?,
            alpha,
        ]);
    }
    NAMED
        .iter()
        .find(|(name, _)| *name == lower)
        .map(|(_, rgb)| {
            [
                ((rgb >> 16) & 0xff) as f32 / 255.0,
                ((rgb >> 8) & 0xff) as f32 / 255.0,
                (rgb & 0xff) as f32 / 255.0,
                1.0,
            ]
        })
}

#[derive(Debug, Clone, PartialEq)]
enum Paint {
    None,
    Colour(Srgba),
}

#[derive(Debug, Clone)]
struct Style {
    fill: Paint,
    stroke: Paint,
    fill_opacity: f32,
    stroke_opacity: f32,
    stroke_width: f32,
    cap: LineCap,
    join: LineJoin,
    miter_limit: f32,
    fill_rule: FillRule,
    colour: Srgba,
    visible: bool,
}

impl Default for Style {
    fn default() -> Self {
        // SVG's initial values: a black fill, no stroke, a one-unit mitred stroke.
        Self {
            fill: Paint::Colour([0.0, 0.0, 0.0, 1.0]),
            stroke: Paint::None,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            stroke_width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 4.0,
            fill_rule: FillRule::NonZero,
            colour: [0.0, 0.0, 0.0, 1.0],
            visible: true,
        }
    }
}

/// What a property said that the importer could not use, to be reported once.
#[derive(Debug, Default)]
struct Unused {
    messages: Vec<String>,
}

/// The declarations of a node: its presentation attributes, then its `style` attribute, which
/// wins.
fn declarations(node: Node<'_, '_>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    const PROPERTIES: [&str; 17] = [
        "fill",
        "stroke",
        "fill-opacity",
        "stroke-opacity",
        "stroke-width",
        "stroke-linecap",
        "stroke-linejoin",
        "stroke-miterlimit",
        "fill-rule",
        "color",
        "display",
        "visibility",
        "opacity",
        "stroke-dasharray",
        "clip-path",
        "mask",
        "filter",
    ];
    for attribute in node.attributes() {
        if PROPERTIES.contains(&attribute.name()) {
            out.push((
                attribute.name().to_string(),
                attribute.value().trim().to_string(),
            ));
        }
    }
    if let Some(style) = node.attribute("style") {
        for declaration in style.split(';') {
            if let Some((name, value)) = declaration.split_once(':') {
                let value = value.trim().trim_end_matches("!important").trim();
                out.push((name.trim().to_ascii_lowercase(), value.to_string()));
            }
        }
    }
    out
}

/// A node's own opacity (not inherited), visibility and the style it passes down.
struct Resolved {
    style: Style,
    opacity: f32,
    display: bool,
}

fn resolve(
    node: Node<'_, '_>,
    parent: &Style,
    gradient_colour: &dyn Fn(&str) -> Option<Srgba>,
    unused: &mut Unused,
) -> Resolved {
    let mut style = parent.clone();
    let mut opacity = 1.0_f32;
    let mut display = true;
    // `color` first, so that `currentColor` in the same element sees it.
    let all = declarations(node);
    for (name, value) in &all {
        if name == "color" {
            if let Some(colour) = parse_colour(value) {
                style.colour = colour;
            }
        }
    }
    let paint = |value: &str, current: Srgba, unused: &mut Unused| -> Option<Paint> {
        let value = value.trim();
        if value == "none" {
            return Some(Paint::None);
        }
        if value.eq_ignore_ascii_case("currentcolor") {
            return Some(Paint::Colour(current));
        }
        if let Some(reference) = value
            .strip_prefix("url(")
            .and_then(|rest| rest.split(')').next())
        {
            let id = reference
                .trim()
                .trim_matches(['"', '\''])
                .trim_start_matches('#');
            return match gradient_colour(id) {
                Some(colour) => {
                    unused.messages.push(
                        "A gradient or pattern fill was replaced by its first colour.".to_string(),
                    );
                    Some(Paint::Colour(colour))
                }
                None => {
                    unused.messages.push(
                        "A fill that refers to something not in the file was left out.".to_string(),
                    );
                    Some(Paint::None)
                }
            };
        }
        parse_colour(value).map(Paint::Colour)
    };
    for (name, value) in &all {
        let number = || value.parse::<f32>().ok().filter(|v| v.is_finite());
        match name.as_str() {
            "fill" => {
                if let Some(parsed) = paint(value, style.colour, unused) {
                    style.fill = parsed;
                }
            }
            "stroke" => {
                if let Some(parsed) = paint(value, style.colour, unused) {
                    style.stroke = parsed;
                }
            }
            "fill-opacity" => {
                if let Some(v) = number() {
                    style.fill_opacity = v.clamp(0.0, 1.0);
                }
            }
            "stroke-opacity" => {
                if let Some(v) = number() {
                    style.stroke_opacity = v.clamp(0.0, 1.0);
                }
            }
            "opacity" => {
                if let Some(v) = number() {
                    opacity = v.clamp(0.0, 1.0);
                }
            }
            "stroke-width" => {
                if let Some(v) = parse_length(value, 0.0) {
                    style.stroke_width = v.max(0.0);
                }
            }
            "stroke-linecap" => match value.as_str() {
                "butt" => style.cap = LineCap::Butt,
                "round" => style.cap = LineCap::Round,
                "square" => style.cap = LineCap::Square,
                _ => {}
            },
            "stroke-linejoin" => match value.as_str() {
                "miter" | "miter-clip" | "arcs" => style.join = LineJoin::Miter,
                "round" => style.join = LineJoin::Round,
                "bevel" => style.join = LineJoin::Bevel,
                _ => {}
            },
            "stroke-miterlimit" => {
                if let Some(v) = number() {
                    style.miter_limit = v.max(1.0);
                }
            }
            "fill-rule" => match value.as_str() {
                "evenodd" => style.fill_rule = FillRule::EvenOdd,
                "nonzero" => style.fill_rule = FillRule::NonZero,
                _ => {}
            },
            "display" => display = value != "none",
            "visibility" => match value.as_str() {
                "hidden" | "collapse" => style.visible = false,
                "visible" => style.visible = true,
                _ => {}
            },
            "stroke-dasharray" if value != "none" && !value.is_empty() => unused
                .messages
                .push("A dashed stroke is imported as a solid one.".to_string()),
            "clip-path" | "mask" | "filter" if value != "none" => unused.messages.push(format!(
                "A {name} was not applied; the shape is imported without it."
            )),
            _ => {}
        }
    }
    Resolved {
        style,
        opacity,
        display,
    }
}

/// A length in user units. A percentage is of `reference` (0 when there is none).
fn parse_length(text: &str, reference: f64) -> Option<f32> {
    let text = text.trim();
    let split = text
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')))
        .unwrap_or(text.len());
    // "1e3" is a number; "1em" has an e that is a unit, so back off when the number is not valid.
    let (number, unit) = text.split_at(split);
    let value: f64 = number.parse().ok().or_else(|| {
        let trimmed = number.trim_end_matches(['e', 'E']);
        trimmed.parse().ok()
    })?;
    let factor = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "px" => 1.0,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        "mm" => 96.0 / 25.4,
        "cm" => 96.0 / 2.54,
        "in" => 96.0,
        "%" => reference / 100.0,
        _ => return None,
    };
    let result = value * factor;
    result.is_finite().then_some(result as f32)
}

// ---------------------------------------------------------------------------------------------
// The walk
// ---------------------------------------------------------------------------------------------

struct Importer<'a, 'input> {
    /// Every element that has an `id`, the first one in document order when several share it,
    /// found in one pass so that a `url(#id)` is a lookup and not a scan of the whole file.
    ids: HashMap<&'a str, Node<'a, 'input>>,
    /// What a gradient element stands for as a paint, worked out the first time it is asked for.
    gradients: RefCell<HashMap<NodeId, Option<Srgba>>>,
    /// The colour of the first `<stop>` child of an element, worked out the first time.
    stops: RefCell<HashMap<NodeId, Option<Srgba>>>,
    report: SvgImportReport,
    layers: usize,
    counter: usize,
    /// The stroke width and other lengths are measured in user units of the root.
    canvas: (f64, f64),
}

impl Importer<'_, '_> {
    fn note(&mut self, kind: ReportKind, layer: Option<&str>, detail: impl Into<String>) {
        self.report.entries.push(ReportEntry {
            kind,
            layer: layer.map(str::to_string),
            detail: detail.into(),
        });
    }

    fn next_id(&mut self) -> String {
        self.counter += 1;
        format!("svg-{}", self.counter)
    }

    fn gradient_colour(&self, id: &str) -> Option<Srgba> {
        let target = *self.ids.get(id)?;
        if !matches!(
            target.tag_name().name(),
            "linearGradient" | "radialGradient"
        ) {
            return None;
        }
        if let Some(known) = self.gradients.borrow().get(&target.id()) {
            return *known;
        }
        // A gradient may inherit its stops through an `href`; one hop is followed, inside the file.
        // Only one, so a chain of them or a loop of them ends at the second gradient.
        let colour = self.first_stop(target).or_else(|| {
            let href = target
                .attribute("href")
                .or_else(|| target.attribute(("http://www.w3.org/1999/xlink", "href")))?;
            let inner = *self.ids.get(href.strip_prefix('#')?)?;
            self.first_stop(inner)
        });
        self.gradients.borrow_mut().insert(target.id(), colour);
        colour
    }

    /// The colour of the first `<stop>` of `holder`, if it has one. Many gradients may inherit from
    /// one holder, and many paints may name one gradient, so each holder is read once.
    fn first_stop(&self, holder: Node<'_, '_>) -> Option<Srgba> {
        if let Some(known) = self.stops.borrow().get(&holder.id()) {
            return *known;
        }
        let colour = holder
            .children()
            .find(|c| c.has_tag_name("stop"))
            .map(|stop| {
                let mut colour = stop
                    .attribute("stop-color")
                    .and_then(parse_colour)
                    .or_else(|| {
                        stop.attribute("style").and_then(|style| {
                            style
                                .split(';')
                                .filter_map(|d| d.split_once(':'))
                                .find(|(n, _)| n.trim() == "stop-color")
                                .and_then(|(_, v)| parse_colour(v))
                        })
                    })
                    .unwrap_or([0.0, 0.0, 0.0, 1.0]);
                if let Some(opacity) = stop
                    .attribute("stop-opacity")
                    .and_then(|v| v.parse::<f32>().ok())
                {
                    colour[3] *= opacity.clamp(0.0, 1.0);
                }
                colour
            });
        self.stops.borrow_mut().insert(holder.id(), colour);
        colour
    }

    fn walk(
        &mut self,
        node: Node<'_, '_>,
        parent_style: &Style,
        parent_matrix: &Affine,
        depth: usize,
        out: &mut Vec<Layer>,
    ) {
        // `check_nesting` has already refused a file nested this deep; this keeps the recursion
        // below bounded on its own account, whatever the parser let through.
        if depth >= MAX_ELEMENT_DEPTH {
            self.note(
                ReportKind::NotImported,
                None,
                format!("Elements nested more than {MAX_ELEMENT_DEPTH} deep were not imported."),
            );
            return;
        }
        for child in node.children().filter(Node::is_element) {
            if self.layers >= MAX_LAYERS {
                self.note(
                    ReportKind::NotImported,
                    None,
                    format!(
                        "The file has more than {MAX_LAYERS} shapes; the rest were not imported."
                    ),
                );
                return;
            }
            let tag = child.tag_name().name();
            let id_name = child.attribute("id").map(str::to_string);
            let label = || id_name.clone().unwrap_or_else(|| tag.to_string());
            match tag {
                // Containers and metadata that draw nothing themselves.
                "defs" | "title" | "desc" | "metadata" | "symbol" | "marker" | "clipPath"
                | "mask" | "linearGradient" | "radialGradient" | "pattern" | "stop" => {}
                "g" | "a" => {
                    self.group(child, parent_style, parent_matrix, depth, out, id_name);
                }
                "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon" | "path" => {
                    self.shape(child, tag, parent_style, parent_matrix, out, id_name);
                }
                "svg" => self.note(
                    ReportKind::NotImported,
                    Some(&label()),
                    "A nested <svg> was not imported.",
                ),
                "use" => self.note(
                    ReportKind::NotImported,
                    Some(&label()),
                    "A <use> of reusable content was not imported.",
                ),
                "text" | "tspan" | "textPath" => self.note(
                    ReportKind::NotImported,
                    Some(&label()),
                    "Text was not imported: an SVG's text needs fonts it does not carry.",
                ),
                "image" => self.note(
                    ReportKind::NotImported,
                    Some(&label()),
                    "An image was not imported; no file or address in an SVG is ever fetched.",
                ),
                "script" => self.note(
                    ReportKind::NotImported,
                    Some(&label()),
                    "A script was not run and is not in the document.",
                ),
                "foreignObject" => self.note(
                    ReportKind::NotImported,
                    Some(&label()),
                    "A <foreignObject> was not imported.",
                ),
                "style" => self.note(
                    ReportKind::Approximated,
                    None,
                    "A style sheet was not applied; only styles written on the shapes themselves were.",
                ),
                other => self.note(
                    ReportKind::NotImported,
                    Some(&label()),
                    format!("<{other}> is not an element this importer reads."),
                ),
            }
        }
    }

    fn group(
        &mut self,
        node: Node<'_, '_>,
        parent_style: &Style,
        parent_matrix: &Affine,
        depth: usize,
        out: &mut Vec<Layer>,
        id_name: Option<String>,
    ) {
        let mut unused = Unused::default();
        let lookup = |id: &str| self.gradient_colour(id);
        let resolved = resolve(node, parent_style, &lookup, &mut unused);
        let name = id_name.unwrap_or_else(|| "Group".to_string());
        for message in unused.messages {
            self.note(ReportKind::Approximated, Some(&name), message);
        }
        if !resolved.display {
            return;
        }
        let matrix = match node.attribute("transform") {
            None => *parent_matrix,
            Some(text) => match parse_transform(text) {
                Some(local) => parent_matrix.then_after(local),
                None => {
                    self.note(
                        ReportKind::Approximated,
                        Some(&name),
                        "A transform that could not be read was ignored.",
                    );
                    *parent_matrix
                }
            },
        };
        let mut children = Vec::new();
        // Past the nesting limit the children are placed in the parent instead.
        let nested = depth + 1 < MAX_GROUP_DEPTH;
        let target: &mut Vec<Layer> = if nested { &mut children } else { out };
        if !nested {
            self.note(
                ReportKind::Approximated,
                Some(&name),
                "Groups nested more deeply than PhotoForge allows were flattened into their parent.",
            );
        }
        self.walk(node, &resolved.style, &matrix, depth + 1, target);
        if !nested || children.is_empty() {
            return;
        }
        let id = self.next_id();
        let mut layer = new_layer(
            &id,
            &clamp_name(&name),
            LayerContent::Group {
                children,
                isolated: true,
            },
            "svg",
        );
        layer.opacity = resolved.opacity;
        layer.visible = resolved.style.visible;
        self.report.groups_imported += 1;
        out.push(layer);
    }

    fn shape(
        &mut self,
        node: Node<'_, '_>,
        tag: &str,
        parent_style: &Style,
        parent_matrix: &Affine,
        out: &mut Vec<Layer>,
        id_name: Option<String>,
    ) {
        let mut unused = Unused::default();
        let lookup = |id: &str| self.gradient_colour(id);
        let resolved = resolve(node, parent_style, &lookup, &mut unused);
        let kind_name = match tag {
            "rect" => "Rectangle",
            "circle" => "Circle",
            "ellipse" => "Ellipse",
            "line" => "Line",
            "polyline" => "Polyline",
            "polygon" => "Polygon",
            _ => "Path",
        };
        let name = id_name.unwrap_or_else(|| kind_name.to_string());
        for message in &unused.messages {
            self.note(ReportKind::Approximated, Some(&name), message.clone());
        }
        if !resolved.display {
            return;
        }
        let matrix = match node.attribute("transform") {
            None => *parent_matrix,
            Some(text) => match parse_transform(text) {
                Some(local) => parent_matrix.then_after(local),
                None => {
                    self.note(
                        ReportKind::Approximated,
                        Some(&name),
                        "A transform that could not be read was ignored.",
                    );
                    *parent_matrix
                }
            },
        };
        let reference = self.canvas.0.max(self.canvas.1);
        let length = |attribute: &str, of: f64| -> Option<f64> {
            node.attribute(attribute)
                .map(|text| parse_length(text, of).map(f64::from))
                .unwrap_or(Some(0.0))
        };
        let (cx, cy) = (self.canvas.0, self.canvas.1);
        let geometry: Option<ShapeGeometry> = match tag {
            "rect" => {
                let (x, y) = (length("x", cx), length("y", cy));
                let (w, h) = (length("width", cx), length("height", cy));
                match (x, y, w, h) {
                    (Some(x), Some(y), Some(w), Some(h)) if w > 0.0 && h > 0.0 => {
                        let rx = length("rx", cx).unwrap_or(0.0);
                        let ry = length("ry", cy).unwrap_or(0.0);
                        let radius = match (rx > 0.0, ry > 0.0) {
                            (true, true) => rx.min(ry),
                            (true, false) => rx,
                            (false, true) => ry,
                            _ => 0.0,
                        };
                        if rx > 0.0 && ry > 0.0 && (rx - ry).abs() > 1e-6 {
                            self.note(
                                ReportKind::Approximated,
                                Some(&name),
                                "Elliptical corners were imported as round ones of the smaller radius.",
                            );
                        }
                        self.rect(&matrix, x, y, w, h, radius)
                    }
                    _ => None,
                }
            }
            "circle" => {
                let r = length("r", reference);
                match (length("cx", cx), length("cy", cy), r) {
                    (Some(cx), Some(cy), Some(r)) if r > 0.0 => self.ellipse(&matrix, cx, cy, r, r),
                    _ => None,
                }
            }
            "ellipse" => {
                match (
                    length("cx", cx),
                    length("cy", cy),
                    length("rx", cx),
                    length("ry", cy),
                ) {
                    (Some(cx), Some(cy), Some(rx), Some(ry)) if rx > 0.0 && ry > 0.0 => {
                        self.ellipse(&matrix, cx, cy, rx, ry)
                    }
                    _ => None,
                }
            }
            "line" => match (
                length("x1", cx),
                length("y1", cy),
                length("x2", cx),
                length("y2", cy),
            ) {
                (Some(x1), Some(y1), Some(x2), Some(y2)) => {
                    let (a, b) = (matrix.apply(x1, y1), matrix.apply(x2, y2));
                    Some(ShapeGeometry::Line {
                        x1: a.0 as f32,
                        y1: a.1 as f32,
                        x2: b.0 as f32,
                        y2: b.1 as f32,
                    })
                }
                _ => None,
            },
            "polyline" | "polygon" => {
                let mut scanner = Scanner::new(node.attribute("points").unwrap_or(""));
                let mut points = Vec::new();
                while !scanner.at_end() {
                    match (scanner.number(), scanner.number()) {
                        (Some(x), Some(y)) => points.push((x, y)),
                        _ => break,
                    }
                    // A file chooses how long this list is; the model's limit is the ceiling.
                    if points.len() > MAX_PATH_COMMANDS {
                        break;
                    }
                }
                if points.len() > MAX_PATH_COMMANDS {
                    self.note(
                        ReportKind::NotImported,
                        Some(&name),
                        format!("The shape has more than {MAX_PATH_COMMANDS} points."),
                    );
                    None
                } else if points.len() < 2 {
                    None
                } else {
                    let mut commands = Vec::with_capacity(points.len() + 1);
                    for (index, (x, y)) in points.iter().enumerate() {
                        let (px, py) = matrix.apply(*x, *y);
                        if index == 0 {
                            commands.push(PathCommand::MoveTo {
                                x: px as f32,
                                y: py as f32,
                            });
                        } else {
                            commands.push(PathCommand::LineTo {
                                x: px as f32,
                                y: py as f32,
                            });
                        }
                    }
                    if tag == "polygon" {
                        commands.push(PathCommand::Close);
                    }
                    Some(ShapeGeometry::Path {
                        path: VectorPath::new(commands),
                    })
                }
            }
            _ => {
                let data = node.attribute("d").unwrap_or("");
                let (commands, ok) = parse_path_data(data);
                if !ok {
                    self.note(
                        ReportKind::Approximated,
                        Some(&name),
                        "The path data has an error; the path is drawn up to it.",
                    );
                }
                if commands.len() > MAX_PATH_COMMANDS {
                    self.note(
                        ReportKind::NotImported,
                        Some(&name),
                        format!("The path has more than {MAX_PATH_COMMANDS} segments."),
                    );
                    None
                } else if commands
                    .iter()
                    .any(|c| matches!(c, PathCommand::MoveTo { .. }))
                    && commands.len() >= 2
                {
                    Some(ShapeGeometry::Path {
                        path: VectorPath::new(transformed(&commands, &matrix)),
                    })
                } else {
                    None
                }
            }
        };
        let Some(geometry) = geometry else { return };

        // Colours: the document works in linear light, an SVG's colours are encoded sRGB.
        let to_colour = |c: Srgba, extra_opacity: f32| {
            ShapeColor::new(
                srgb_decode(c[0]),
                srgb_decode(c[1]),
                srgb_decode(c[2]),
                (c[3] * extra_opacity).clamp(0.0, 1.0),
            )
        };
        let fill = match &resolved.style.fill {
            Paint::Colour(c) => Some(to_colour(*c, resolved.style.fill_opacity)),
            Paint::None => None,
        };
        let stroke_width = f64::from(resolved.style.stroke_width) * matrix.mean_scale();
        let stroke = match &resolved.style.stroke {
            Paint::Colour(c) if stroke_width > 0.0 => {
                Some(to_colour(*c, resolved.style.stroke_opacity))
            }
            _ => None,
        };
        let stroke_style = stroke.map(|_| StrokeStyle {
            width: (stroke_width as f32).clamp(0.01, crate::vector::MAX_STROKE_WIDTH),
            cap: resolved.style.cap,
            join: resolved.style.join,
            miter_limit: resolved.style.miter_limit,
        });
        if stroke.is_some() && !matrix.is_isotropic() {
            self.note(
                ReportKind::Approximated,
                Some(&name),
                "The shape is scaled differently across and down; its stroke was imported at the average of the two.",
            );
        }
        let content = ShapeContent {
            geometry,
            fill,
            stroke,
            stroke_style,
            fill_rule: resolved.style.fill_rule,
        };
        if content.validate().is_err() {
            self.note(
                ReportKind::NotImported,
                Some(&name),
                "The shape is outside the range PhotoForge can hold.",
            );
            return;
        }
        let id = self.next_id();
        let mut layer = new_layer(
            &id,
            &clamp_name(&name),
            LayerContent::Shape {
                shape: Box::new(content),
            },
            "svg",
        );
        layer.opacity = resolved.opacity;
        layer.visible = resolved.style.visible;
        self.layers += 1;
        self.report.layers_imported += 1;
        out.push(layer);
    }

    /// A rectangle, kept a rectangle when the transform allows it.
    fn rect(
        &self,
        matrix: &Affine,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        radius: f64,
    ) -> Option<ShapeGeometry> {
        if matrix.is_axis_aligned() && (matrix.a.abs() - matrix.d.abs()).abs() < 1e-9 {
            let (x0, y0) = matrix.apply(x, y);
            let (x1, y1) = matrix.apply(x + w, y + h);
            let scale = matrix.a.abs();
            return Some(ShapeGeometry::Rectangle {
                x: x0.min(x1) as f32,
                y: y0.min(y1) as f32,
                width: (x1 - x0).abs() as f32,
                height: (y1 - y0).abs() as f32,
                corner_radius: (radius * scale) as f32,
            });
        }
        // A rotation, a skew or an uneven scale: the rectangle becomes its outline.
        let base = ShapeGeometry::Rectangle {
            x: x as f32,
            y: y as f32,
            width: w as f32,
            height: h as f32,
            corner_radius: radius as f32,
        }
        .to_path();
        Some(ShapeGeometry::Path {
            path: VectorPath::new(transformed(&base.commands, matrix)),
        })
    }

    fn ellipse(
        &self,
        matrix: &Affine,
        cx: f64,
        cy: f64,
        rx: f64,
        ry: f64,
    ) -> Option<ShapeGeometry> {
        if matrix.is_axis_aligned() {
            let (px, py) = matrix.apply(cx, cy);
            return Some(ShapeGeometry::Ellipse {
                cx: px as f32,
                cy: py as f32,
                rx: (rx * matrix.a.abs()) as f32,
                ry: (ry * matrix.d.abs()) as f32,
            });
        }
        let base = ShapeGeometry::Ellipse {
            cx: cx as f32,
            cy: cy as f32,
            rx: rx as f32,
            ry: ry as f32,
        }
        .to_path();
        Some(ShapeGeometry::Path {
            path: VectorPath::new(transformed(&base.commands, matrix)),
        })
    }
}

fn clamp_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).take(100).collect();
    if cleaned.trim().is_empty() {
        "Shape".to_string()
    } else {
        cleaned
    }
}

/// Refuses text whose elements nest more than `MAX_ELEMENT_DEPTH` deep, without parsing it.
///
/// The parser recurses once per open element and has no depth limit of its own (only the node
/// limit), so a few thousand `<g>` abort the whole process with a stack overflow before any of this
/// importer's code runs. This reads the markup the way the parser does, skipping comments, CDATA,
/// processing instructions and quoted attribute values, and counts open elements. Text the parser
/// would refuse anyway is not this function's business: it stops where the parser would.
fn check_nesting(text: &str) -> Result<(), AppError> {
    let bytes = text.as_bytes();
    // The first place at or after `from` where `needle` starts.
    let find = |from: usize, needle: &[u8]| -> Option<usize> {
        let tail = bytes.get(from..)?;
        tail.windows(needle.len())
            .position(|window| window == needle)
            .map(|at| from + at)
    };
    let mut depth = 0_usize;
    let mut at = 0;
    while let Some(offset) = bytes[at..].iter().position(|byte| *byte == b'<') {
        at += offset;
        let rest = &bytes[at..];
        let next = if rest.starts_with(b"<!--") {
            find(at + 4, b"-->").map(|end| end + 3)
        } else if rest.starts_with(b"<![CDATA[") {
            find(at + 9, b"]]>").map(|end| end + 3)
        } else if rest.starts_with(b"<?") {
            find(at + 2, b"?>").map(|end| end + 2)
        } else if rest.starts_with(b"<!") {
            // A DTD or a stray declaration: the parser refuses it.
            return Ok(());
        } else if rest.starts_with(b"</") {
            depth = depth.saturating_sub(1);
            find(at + 2, b">").map(|end| end + 1)
        } else {
            // A start tag runs to the first `>` outside a quoted value; `/>` closes it at once.
            depth += 1;
            if depth > MAX_ELEMENT_DEPTH {
                return Err(AppError::foreign_file(format!(
                    "the SVG nests elements more than {MAX_ELEMENT_DEPTH} deep"
                )));
            }
            let mut quote = None;
            let mut end = None;
            for (index, byte) in bytes.iter().enumerate().skip(at + 1) {
                match (quote, *byte) {
                    (Some(open), byte) if byte == open => quote = None,
                    (Some(_), _) => {}
                    (None, b'"' | b'\'') => quote = Some(*byte),
                    (None, b'>') => {
                        end = Some(index);
                        break;
                    }
                    (None, _) => {}
                }
            }
            end.map(|end| {
                if bytes[end - 1] == b'/' {
                    depth -= 1;
                }
                end + 1
            })
        };
        match next {
            Some(next) => at = next,
            // Unterminated: the parser refuses it.
            None => return Ok(()),
        }
    }
    Ok(())
}

/// Reads `text` as SVG.
pub fn import_svg_text(text: &str) -> Result<ImportedSvg, AppError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    check_nesting(text)?;
    let options = ParsingOptions {
        allow_dtd: false,
        nodes_limit: MAX_NODES,
    };
    let document = Document::parse_with_options(text, options).map_err(|error| {
        AppError::foreign_file(format!("the file is not an SVG that can be read: {error}"))
    })?;
    let root = document.root_element();
    if root.tag_name().name() != "svg" {
        return Err(AppError::foreign_file(
            "the file is XML but its root is not <svg>".into(),
        ));
    }

    // The canvas: width and height, or the view box.
    let view_box: Option<(f64, f64, f64, f64)> = root.attribute("viewBox").and_then(|text| {
        let mut scanner = Scanner::new(text);
        let values = [
            scanner.number()?,
            scanner.number()?,
            scanner.number()?,
            scanner.number()?,
        ];
        (values[2] > 0.0 && values[3] > 0.0).then_some((values[0], values[1], values[2], values[3]))
    });
    let size = |attribute: &str, reference: Option<f64>| -> Option<f64> {
        root.attribute(attribute)
            .and_then(|text| parse_length(text, reference.unwrap_or(0.0)))
            .map(f64::from)
            .filter(|v| *v > 0.0)
    };
    let width = size("width", view_box.map(|v| v.2)).or(view_box.map(|v| v.2));
    let height = size("height", view_box.map(|v| v.3)).or(view_box.map(|v| v.3));
    let (Some(width), Some(height)) = (width, height) else {
        return Err(AppError::foreign_file(
            "the SVG has neither a width and height nor a view box, so it has no size".into(),
        ));
    };
    if width > MAX_EDGE || height > MAX_EDGE {
        return Err(AppError::foreign_file(format!(
            "the SVG is {width:.0} × {height:.0} pixels; the largest edge read is {MAX_EDGE:.0}"
        )));
    }
    let (canvas_width, canvas_height) = (
        width.round().max(1.0) as u32,
        height.round().max(1.0) as u32,
    );
    validate_dimensions(canvas_width, canvas_height)?;
    if u64::from(canvas_width) * u64::from(canvas_height) > crate::resources::max_working_pixels() {
        return Err(AppError::ImageTooLarge {
            pixels: u64::from(canvas_width) * u64::from(canvas_height),
            limit: crate::resources::max_working_pixels(),
        });
    }

    let mut ids = HashMap::new();
    for element in document.descendants().filter(Node::is_element) {
        if let Some(id) = element.attribute("id") {
            ids.entry(id).or_insert(element);
        }
    }
    let mut importer = Importer {
        ids,
        gradients: RefCell::default(),
        stops: RefCell::default(),
        report: SvgImportReport {
            format: "SVG".to_string(),
            width: canvas_width,
            height: canvas_height,
            ..SvgImportReport::default()
        },
        layers: 0,
        counter: 0,
        canvas: (f64::from(canvas_width), f64::from(canvas_height)),
    };

    // The view box mapped onto the canvas, as `preserveAspectRatio` says (the default is meet,
    // centred; `none` stretches; slice is imported as meet and said).
    let mut root_matrix = Affine::IDENTITY;
    if let Some((vx, vy, vw, vh)) = view_box {
        let (sx, sy) = (f64::from(canvas_width) / vw, f64::from(canvas_height) / vh);
        let aspect = root
            .attribute("preserveAspectRatio")
            .unwrap_or("xMidYMid meet");
        if aspect.trim() == "none" {
            root_matrix = Affine {
                a: sx,
                d: sy,
                e: -vx * sx,
                f: -vy * sy,
                ..Affine::IDENTITY
            };
        } else {
            if aspect.contains("slice") {
                importer.note(
                    ReportKind::Approximated,
                    None,
                    "preserveAspectRatio \"slice\" was imported as \"meet\": the whole view box is shown.",
                );
            }
            let scale = sx.min(sy);
            let (tx, ty) = (
                (f64::from(canvas_width) - vw * scale) / 2.0,
                (f64::from(canvas_height) - vh * scale) / 2.0,
            );
            root_matrix = Affine {
                a: scale,
                d: scale,
                e: tx - vx * scale,
                f: ty - vy * scale,
                ..Affine::IDENTITY
            };
        }
    }

    let mut layers = Vec::new();
    let root_style = Style::default();
    let mut unused = Unused::default();
    let lookup = |id: &str| importer.gradient_colour(id);
    let resolved_root = resolve(root, &root_style, &lookup, &mut unused);
    importer.walk(root, &resolved_root.style, &root_matrix, 0, &mut layers);

    let mut canvas = LayerDocument::new(canvas_width, canvas_height);
    canvas.precision = DocumentPrecision::LinearSrgbF32;
    canvas.active_layer_id = layers.last().map(|layer| layer.id.clone());
    canvas.layers = layers;
    canvas.validate()?;
    if importer.report.layers_imported == 0 {
        importer.note(
            ReportKind::Note,
            None,
            "The file has no shapes this importer reads, so the document is an empty canvas.",
        );
    }
    let now = SystemClock.now();
    let report = importer.report;
    Ok(ImportedSvg {
        project: LoadedProject {
            document: canvas,
            document_operations: Vec::new(),
            pixels: Vec::new(),
            linear_pixels: Vec::new(),
            application_version: env!("CARGO_PKG_VERSION").into(),
            created_at: now.clone(),
            modified_at: now,
            source_metadata: None,
        },
        report,
    })
}

/// Reads the SVG file at `path`. Nothing in it is fetched or run.
pub fn import_svg(path: &Path) -> Result<ImportedSvg, AppError> {
    let metadata = std::fs::metadata(path).map_err(|_| AppError::DecodeFailure)?;
    if !metadata.is_file() {
        return Err(AppError::UnsupportedImageFormat);
    }
    if metadata.len() > MAX_SVG_BYTES {
        return Err(AppError::foreign_file(format!(
            "the SVG is {} bytes; the largest file read is {MAX_SVG_BYTES}",
            metadata.len()
        )));
    }
    let bytes = std::fs::read(path).map_err(|_| AppError::DecodeFailure)?;
    let text = std::str::from_utf8(&bytes).map_err(|_| {
        AppError::foreign_file("the SVG is not UTF-8 text (compressed .svgz is not read)".into())
    })?;
    import_svg_text(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::{render_document_typed, LayerPixelStore, LayerTransform, RenderOptions};
    use crate::svg::{export_svg, SvgOptions};

    fn import(svg: &str) -> ImportedSvg {
        import_svg_text(svg).unwrap()
    }

    fn shapes(imported: &ImportedSvg) -> Vec<&ShapeContent> {
        fn collect<'a>(layers: &'a [Layer], out: &mut Vec<&'a ShapeContent>) {
            for layer in layers {
                match &layer.content {
                    LayerContent::Shape { shape } => out.push(shape),
                    LayerContent::Group { children, .. } => collect(children, out),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        collect(&imported.project.document.layers, &mut out);
        out
    }

    fn has(report: &SvgImportReport, kind: ReportKind, text: &str) -> bool {
        report
            .entries
            .iter()
            .any(|e| e.kind == kind && e.detail.contains(text))
    }

    fn rendered(document: &LayerDocument) -> image::RgbaImage {
        let mut store = LayerPixelStore::default();
        store
            .reset(document.canvas_width, document.canvas_height)
            .unwrap();
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        render_document_typed(document, &resolved, RenderOptions::default())
            .unwrap()
            .encoded8()
            .as_ref()
            .clone()
    }

    const BASIC: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 200 100">
        <rect x="10" y="10" width="80" height="40" rx="6" fill="#ff0000"/>
        <circle cx="150" cy="50" r="30" fill="rgb(0, 128, 0)" stroke="#0000ff" stroke-width="4"/>
        <ellipse cx="50" cy="75" rx="30" ry="12" style="fill:#808080;opacity:0.5"/>
        <line x1="0" y1="0" x2="200" y2="100" stroke="black"/>
        <polygon points="100,10 120,40 80,40" fill="gold"/>
    </svg>"##;

    #[test]
    fn shapes_come_across_as_shapes_with_their_colours_in_linear_light() {
        let imported = import(BASIC);
        assert_eq!((imported.report.width, imported.report.height), (200, 100));
        let found = shapes(&imported);
        assert_eq!(found.len(), 5);
        match &found[0].geometry {
            ShapeGeometry::Rectangle {
                x,
                y,
                width,
                height,
                corner_radius,
            } => assert_eq!(
                (*x, *y, *width, *height, *corner_radius),
                (10.0, 10.0, 80.0, 40.0, 6.0)
            ),
            other => panic!("{other:?}"),
        }
        let red = found[0].fill.unwrap();
        assert_eq!(
            (red.red, red.green, red.blue, red.alpha),
            (1.0, 0.0, 0.0, 1.0)
        );
        // sRGB 128 is linear 0.2158.
        let grey = found[2].fill.unwrap();
        assert!((grey.red - 0.2158).abs() < 1e-3, "{}", grey.red);
        let ring = found[1];
        assert_eq!(ring.stroke_style.unwrap().width, 4.0);
        let blue = ring.stroke.unwrap();
        assert_eq!((blue.red, blue.green, blue.blue), (0.0, 0.0, 1.0));
        // Opacity from `style` is the layer's.
        assert_eq!(imported.project.document.layers[2].opacity, 0.5);
        assert!(matches!(found[3].geometry, ShapeGeometry::Line { .. }));
        assert!(
            imported.report.entries.is_empty(),
            "{:?}",
            imported.report.entries
        );
    }

    #[test]
    fn the_view_box_is_mapped_onto_the_canvas() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 100 50">
            <rect x="10" y="10" width="20" height="10"/></svg>"#;
        let imported = import(svg);
        match &shapes(&imported)[0].geometry {
            ShapeGeometry::Rectangle {
                x,
                y,
                width,
                height,
                ..
            } => assert_eq!((*x, *y, *width, *height), (20.0, 20.0, 40.0, 20.0)),
            other => panic!("{other:?}"),
        }
        // Without a width and height the view box is the size.
        let bare = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 32"><rect width="5" height="5"/></svg>"#;
        let imported = import(bare);
        assert_eq!((imported.report.width, imported.report.height), (64, 32));
        // Units.
        let inches = r#"<svg xmlns="http://www.w3.org/2000/svg" width="1in" height="0.5in"/>"#;
        let imported = import(inches);
        assert_eq!((imported.report.width, imported.report.height), (96, 48));
    }

    #[test]
    fn a_move_or_an_axis_aligned_scale_keeps_a_rectangle_a_rectangle_and_a_rotation_bakes() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <g transform="translate(5 7) scale(2)"><rect x="1" y="2" width="10" height="5"/></g>
            <rect x="0" y="0" width="10" height="10" transform="rotate(45 5 5)"/>
        </svg>"#;
        let imported = import(svg);
        let found = shapes(&imported);
        match &found[0].geometry {
            ShapeGeometry::Rectangle {
                x,
                y,
                width,
                height,
                ..
            } => assert_eq!((*x, *y, *width, *height), (7.0, 11.0, 20.0, 10.0)),
            other => panic!("{other:?}"),
        }
        // The rotated square is its own outline now.
        assert!(matches!(found[1].geometry, ShapeGeometry::Path { .. }));
    }

    #[test]
    fn style_overrides_attributes_and_groups_pass_their_style_down() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="50" height="50">
            <g fill="#00ff00" stroke="#000000" stroke-width="3" opacity="0.4">
              <rect width="10" height="10" style="fill:#0000ff"/>
              <circle cx="30" cy="30" r="5" fill="currentColor" color="#ff0000"/>
            </g></svg>"##;
        let imported = import(svg);
        let found = shapes(&imported);
        let blue = found[0].fill.unwrap();
        assert_eq!((blue.red, blue.green, blue.blue), (0.0, 0.0, 1.0));
        assert_eq!(found[0].stroke_style.unwrap().width, 3.0);
        let red = found[1].fill.unwrap();
        assert_eq!((red.red, red.green, red.blue), (1.0, 0.0, 0.0));
        // The group's opacity is the group's.
        assert_eq!(imported.project.document.layers[0].opacity, 0.4);
        assert_eq!(imported.report.groups_imported, 1);
    }

    #[test]
    fn what_the_subset_does_not_cover_is_named_and_never_fetched_or_run() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100">
            <style>.a { fill: red }</style>
            <defs><linearGradient id="g"><stop offset="0" stop-color="#ff8800"/><stop offset="1" stop-color="#000"/></linearGradient></defs>
            <rect id="gradient" width="10" height="10" fill="url(#g)" stroke="#000" stroke-dasharray="4 2"/>
            <rect id="clipped" width="10" height="10" clip-path="url(#nothing)"/>
            <text x="5" y="5">hello</text>
            <image href="http://example.com/a.png" width="10" height="10"/>
            <use href="#gradient"/>
            <script>alert(1)</script>
            <foreignObject width="10" height="10"/>
        </svg>"##;
        let imported = import(svg);
        let report = &imported.report;
        assert_eq!(report.layers_imported, 2);
        assert!(has(report, ReportKind::Approximated, "first colour"));
        assert!(has(report, ReportKind::Approximated, "dashed stroke"));
        assert!(has(report, ReportKind::Approximated, "style sheet"));
        assert!(has(report, ReportKind::Approximated, "clip-path"));
        for what in [
            "Text was not imported",
            "An image was not imported",
            "<use>",
            "script",
            "foreignObject",
        ] {
            assert!(
                has(report, ReportKind::NotImported, what),
                "{what}: {:?}",
                report.entries
            );
        }
        // The gradient became its first colour: sRGB #ff8800.
        let fill = shapes(&imported)[0].fill.unwrap();
        assert_eq!(fill.red, 1.0);
        assert!((fill.green - srgb_decode(f32::from(0x88_u8) / 255.0)).abs() < 1e-4);
    }

    #[test]
    fn a_dtd_and_an_entity_bomb_are_refused() {
        let doctype = r#"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY a "x">]><svg xmlns="http://www.w3.org/2000/svg" width="5" height="5"/>"#;
        assert!(import_svg_text(doctype).is_err());
        let mut bomb = String::from("<?xml version=\"1.0\"?><!DOCTYPE lolz [<!ENTITY lol \"lol\">");
        for level in 1..8 {
            let previous = if level == 1 {
                "&lol;".to_string()
            } else {
                format!("&lol{};", level - 1)
            };
            bomb.push_str(&format!("<!ENTITY lol{level} \"{}\">", previous.repeat(10)));
        }
        bomb.push_str(
            "]><svg xmlns=\"http://www.w3.org/2000/svg\" width=\"5\" height=\"5\"><title>&lol7;</title></svg>",
        );
        assert!(import_svg_text(&bomb).is_err());
    }

    #[test]
    fn size_and_shape_count_are_bounded() {
        let huge = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100000" height="100000"/>"#;
        assert!(import_svg_text(huge).is_err());
        let sizeless =
            r#"<svg xmlns="http://www.w3.org/2000/svg"><rect width="5" height="5"/></svg>"#;
        assert!(import_svg_text(sizeless).is_err());
        let mut many =
            String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64">"#);
        for i in 0..(MAX_LAYERS + 40) {
            many.push_str(&format!(
                r#"<rect x="{}" y="1" width="2" height="2"/>"#,
                i % 60
            ));
        }
        many.push_str("</svg>");
        let imported = import_svg_text(&many).unwrap();
        assert_eq!(imported.report.layers_imported, MAX_LAYERS);
        assert!(has(&imported.report, ReportKind::NotImported, "more than"));
        // Not an SVG at all.
        assert!(import_svg_text("<html/>").is_err());
        assert!(import_svg_text("not xml").is_err());
    }

    #[test]
    fn a_shape_too_long_or_too_far_for_the_model_is_left_out_not_clipped_or_crashed_on() {
        // More segments than a path may hold.
        let mut long = String::from("M0 0");
        for i in 0..(MAX_PATH_COMMANDS + 50) {
            long.push_str(&format!(" L{} {}", i % 90, (i * 7) % 90));
        }
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><path d="{long}"/><rect width="5" height="5"/></svg>"#
        );
        let imported = import_svg_text(&svg).unwrap();
        assert_eq!(imported.report.layers_imported, 1);
        assert!(has(&imported.report, ReportKind::NotImported, "segments"));
        // The same through a polyline.
        let mut points = String::new();
        for i in 0..(MAX_PATH_COMMANDS + 50) {
            points.push_str(&format!("{},{} ", i % 90, (i * 3) % 90));
        }
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><polyline points="{points}"/></svg>"#
        );
        let imported = import_svg_text(&svg).unwrap();
        assert_eq!(imported.report.layers_imported, 0);
        assert!(has(&imported.report, ReportKind::NotImported, "points"));
        // Coordinates no float can hold.
        let far = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <rect x="1e300" y="0" width="5" height="5"/>
            <path d="M0 0 L1e308 1e308"/>
            <circle cx="5" cy="5" r="1e39"/>
        </svg>"#;
        let imported = import_svg_text(far).unwrap();
        assert_eq!(
            imported.report.layers_imported, 0,
            "{:?}",
            imported.report.entries
        );
        // A scale that compounds past anything finite.
        let mut compounding =
            String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">"#);
        for _ in 0..12 {
            compounding.push_str(r#"<g transform="scale(100000)">"#);
        }
        compounding.push_str(r#"<rect width="1" height="1"/>"#);
        for _ in 0..12 {
            compounding.push_str("</g>");
        }
        compounding.push_str("</svg>");
        let imported = import_svg_text(&compounding).unwrap();
        assert_eq!(imported.report.layers_imported, 0);
        imported.project.document.validate().unwrap();
    }

    #[test]
    fn a_childs_transform_is_applied_before_its_groups_and_stroke_widths_scale_with_both() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <g transform="translate(10 0)">
              <rect x="1" y="0" width="4" height="4" transform="scale(2)" stroke="#000" stroke-width="3" fill="none"/>
            </g></svg>"##;
        let imported = import(svg);
        let found = shapes(&imported);
        match &found[0].geometry {
            ShapeGeometry::Rectangle { x, width, .. } => {
                // Scaled first (1 -> 2, 4 -> 8), then moved by the group (2 -> 12).
                assert_eq!((*x, *width), (12.0, 8.0));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(found[0].stroke_style.unwrap().width, 6.0);
    }

    #[test]
    fn none_is_no_paint_and_the_default_is_a_black_fill_with_no_stroke() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40">
            <rect width="10" height="10" fill="none" stroke="#f00"/>
            <rect width="10" height="10"/>
            <rect width="10" height="10" stroke="#00f" stroke-width="0"/>
        </svg>"##;
        let imported = import(svg);
        let found = shapes(&imported);
        assert!(found[0].fill.is_none() && found[0].stroke.is_some());
        let default = found[1].fill.unwrap();
        assert_eq!(
            (default.red, default.green, default.blue, default.alpha),
            (0.0, 0.0, 0.0, 1.0)
        );
        assert!(found[1].stroke.is_none());
        // A zero-width stroke draws nothing.
        assert!(found[2].stroke.is_none());
    }

    #[test]
    fn a_view_box_of_another_shape_is_fitted_and_centred() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 100 100">
            <rect x="0" y="0" width="100" height="100"/></svg>"#;
        let imported = import(svg);
        match &shapes(&imported)[0].geometry {
            ShapeGeometry::Rectangle {
                x,
                y,
                width,
                height,
                ..
            } => assert_eq!((*x, *y, *width, *height), (50.0, 0.0, 100.0, 100.0)),
            other => panic!("{other:?}"),
        }
        // An origin that is not zero is moved to the corner.
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="-50 -50 100 100">
            <rect x="-50" y="-50" width="10" height="10"/></svg>"#;
        let imported = import(svg);
        match &shapes(&imported)[0].geometry {
            ShapeGeometry::Rectangle { x, y, .. } => assert_eq!((*x, *y), (0.0, 0.0)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn nesting_past_the_limit_is_flattened_not_refused() {
        let mut svg =
            String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20">"#);
        for _ in 0..(MAX_GROUP_DEPTH + 6) {
            svg.push_str("<g>");
        }
        svg.push_str(r#"<rect width="4" height="4"/>"#);
        for _ in 0..(MAX_GROUP_DEPTH + 6) {
            svg.push_str("</g>");
        }
        svg.push_str("</svg>");
        let imported = import_svg_text(&svg).unwrap();
        assert_eq!(imported.report.layers_imported, 1);
        assert!(has(&imported.report, ReportKind::Approximated, "flattened"));
        imported.project.document.validate().unwrap();
    }

    fn path_points(commands: &[PathCommand]) -> Vec<(f32, f32)> {
        VectorPath::new(commands.to_vec())
            .flatten(0.01)
            .into_iter()
            .flatten()
            .collect()
    }

    #[test]
    fn path_data_covers_every_command_and_its_shorthands() {
        let (commands, ok) = parse_path_data("M10 10 h20 v20 H10 Z m5 5 l10 0 10 10 z");
        assert!(ok);
        // After a close, a relative move is from the start of the subpath it closed.
        assert!(matches!(commands[0], PathCommand::MoveTo { x, y } if (x, y) == (10.0, 10.0)));
        assert!(matches!(commands[4], PathCommand::Close));
        assert!(matches!(commands[5], PathCommand::MoveTo { x, y } if (x, y) == (15.0, 15.0)));
        // Smooth curves reflect the previous control point.
        let (commands, ok) = parse_path_data("M0 0 C 10 0 20 10 20 20 S 30 40 40 40");
        assert!(ok);
        match commands[2] {
            PathCommand::CubicTo { c1x, c1y, .. } => assert_eq!((c1x, c1y), (20.0, 30.0)),
            other => panic!("{other:?}"),
        }
        let (commands, ok) = parse_path_data("M0 0 Q 10 20 20 0 T 40 0");
        assert!(ok);
        assert_eq!(commands.len(), 3);
        // Numbers with exponents.
        let (commands, ok) = parse_path_data("M1e1,1E1 L2.5e1 .5e1");
        assert!(ok, "{commands:?}");
        assert!(matches!(commands[0], PathCommand::MoveTo { x, y } if (x, y) == (10.0, 10.0)));
        assert!(matches!(commands[1], PathCommand::LineTo { x, y } if (x, y) == (25.0, 5.0)));
        // A drawing command after a close starts from the start of the closed subpath.
        let (commands, ok) = parse_path_data("M5 5 L10 5 L10 10 Z L20 20");
        assert!(ok);
        assert!(matches!(commands[4], PathCommand::MoveTo { x, y } if (x, y) == (5.0, 5.0)));
    }

    #[test]
    fn an_arc_is_a_circle_arc_through_its_endpoints() {
        let (commands, ok) = parse_path_data("M0 0 A10 10 0 0 1 20 0");
        assert!(ok);
        let points = path_points(&commands);
        let end = points.last().unwrap();
        assert!((end.0 - 20.0).abs() < 1e-3 && end.1.abs() < 1e-3);
        for (x, y) in &points {
            let distance = ((x - 10.0).powi(2) + y.powi(2)).sqrt();
            assert!(
                (distance - 10.0).abs() < 0.05,
                "({x}, {y}) is {distance} from the centre"
            );
        }
        // The sweep flag picks the side: sweep 1 goes through y = -10 in SVG's y-down space.
        assert!(points.iter().any(|(_, y)| (*y + 10.0).abs() < 0.1));
        // An arc with a radius too small is scaled up to reach, not dropped.
        let (commands, ok) = parse_path_data("M0 0 a1 1 0 1 0 30 0");
        assert!(ok);
        assert!(!commands.is_empty());
        // Flags written without separators.
        let (commands, ok) = parse_path_data("M0 0a5 5 0 0110 0");
        assert!(ok);
        assert!(!commands.is_empty());
    }

    #[test]
    fn an_error_in_path_data_keeps_what_came_before_it() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="50" height="50">
            <path d="M0 0 L10 10 L 20 oops 30" fill="none" stroke="#000"/></svg>"##;
        let imported = import(svg);
        assert_eq!(imported.report.layers_imported, 1);
        assert!(has(
            &imported.report,
            ReportKind::Approximated,
            "path data has an error"
        ));
    }

    #[test]
    fn colours_are_read_in_each_form() {
        for (text, expected) in [
            ("#f00", [1.0, 0.0, 0.0, 1.0]),
            ("#ff0000", [1.0, 0.0, 0.0, 1.0]),
            ("#ff000080", [1.0, 0.0, 0.0, 128.0 / 255.0]),
            ("rgb(255,0,0)", [1.0, 0.0, 0.0, 1.0]),
            ("rgb(100%, 0%, 0%)", [1.0, 0.0, 0.0, 1.0]),
            ("rgba(0,0,255,0.5)", [0.0, 0.0, 1.0, 0.5]),
            ("RED", [1.0, 0.0, 0.0, 1.0]),
            ("transparent", [0.0, 0.0, 0.0, 0.0]),
        ] {
            let got = parse_colour(text).unwrap_or_else(|| panic!("{text}"));
            for (g, e) in got.iter().zip(expected) {
                assert!((g - e).abs() < 1e-3, "{text}: {got:?}");
            }
        }
        assert!(parse_colour("notacolour").is_none());
        assert!(parse_colour("#12").is_none());
    }

    #[test]
    fn transform_lists_compose_the_way_svg_says() {
        let m = parse_transform("translate(10 0) scale(2)").unwrap();
        // Scale first, then move.
        assert_eq!(m.apply(1.0, 1.0), (12.0, 2.0));
        let r = parse_transform("rotate(90)").unwrap();
        let (x, y) = r.apply(1.0, 0.0);
        assert!(x.abs() < 1e-9 && (y - 1.0).abs() < 1e-9);
        let about = parse_transform("rotate(180 5 5)").unwrap();
        let (x, y) = about.apply(0.0, 0.0);
        assert!((x - 10.0).abs() < 1e-9 && (y - 10.0).abs() < 1e-9);
        assert!(parse_transform("wobble(3)").is_none());
        assert!(parse_transform("scale(1 2 3)").is_none());
    }

    #[test]
    fn what_this_program_writes_it_reads_back_as_the_same_picture() {
        // The exporter's own output, through the importer, rendered, against the original.
        let mut document = LayerDocument::new(160, 100);
        document.precision = DocumentPrecision::LinearSrgbF32;
        let make = |id: &str, geometry: ShapeGeometry, fill: Option<ShapeColor>| {
            new_layer(
                id,
                id,
                LayerContent::Shape {
                    shape: Box::new(ShapeContent {
                        geometry,
                        fill,
                        stroke: Some(ShapeColor::new(0.0, 0.0, 0.0, 1.0)),
                        stroke_style: Some(StrokeStyle {
                            width: 3.0,
                            cap: LineCap::Round,
                            join: LineJoin::Round,
                            miter_limit: 4.0,
                        }),
                        fill_rule: FillRule::NonZero,
                    }),
                },
                "t",
            )
        };
        let mut moved = make(
            "r",
            ShapeGeometry::Rectangle {
                x: 20.0,
                y: 20.0,
                width: 60.0,
                height: 30.0,
                corner_radius: 5.0,
            },
            Some(ShapeColor::new(0.8, 0.1, 0.05, 1.0)),
        );
        moved.transform = LayerTransform {
            rotation_degrees: 20.0,
            scale_x: 1.2,
            scale_y: 1.2,
            translate_x: 15.0,
            translate_y: -5.0,
            ..LayerTransform::default()
        };
        let mut faded = make(
            "e",
            ShapeGeometry::Ellipse {
                cx: 110.0,
                cy: 50.0,
                rx: 30.0,
                ry: 20.0,
            },
            Some(ShapeColor::new(0.05, 0.3, 0.8, 0.7)),
        );
        faded.opacity = 0.8;
        let star = make(
            "s",
            ShapeGeometry::Star {
                cx: 50.0,
                cy: 75.0,
                outer_radius: 20.0,
                inner_radius: 8.0,
                points: 5,
                rotation_degrees: -90.0,
            },
            Some(ShapeColor::new(0.9, 0.7, 0.05, 1.0)),
        );
        document.layers = vec![moved, faded, star];
        let exported = export_svg(&document, &SvgOptions::default()).unwrap();
        let imported = import(&exported.text);
        assert_eq!(shapes(&imported).len(), 3);
        let before = rendered(&document);
        let after = rendered(&imported.project.document);
        assert_eq!(before.dimensions(), after.dimensions());
        let mut worst = 0.0_f32;
        let mut over = 0;
        for (p, q) in before.pixels().zip(after.pixels()) {
            let a = f32::from(p[3]) / 255.0;
            let b = f32::from(q[3]) / 255.0;
            for c in 0..3 {
                let d = (f32::from(p[c]) * a - f32::from(q[c]) * b).abs();
                worst = worst.max(d);
                if d > 24.0 {
                    over += 1;
                }
            }
        }
        // Edges anti-alias differently after the rotation is baked; the interiors are the same.
        let total = before.pixels().len() * 3;
        assert!(
            over * 200 < total,
            "{over} of {total} samples differ by more than 24 (worst {worst})"
        );
    }

    #[test]
    fn no_truncation_and_no_changed_byte_makes_it_panic() {
        let svg = BASIC.as_bytes();
        for len in 0..svg.len() {
            if let Ok(text) = std::str::from_utf8(&svg[..len]) {
                let _ = import_svg_text(text);
            }
        }
        for at in (0..svg.len()).step_by(3) {
            for value in [b'<', b'>', b'"', b'0', b'-', b' ', 0x00, b'#'] {
                let mut changed = svg.to_vec();
                changed[at] = value;
                if let Ok(text) = std::str::from_utf8(&changed) {
                    let _ = import_svg_text(text);
                }
            }
        }
    }

    #[test]
    fn a_file_that_is_too_large_or_not_text_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("big.svg");
        std::fs::write(&big, vec![b' '; (MAX_SVG_BYTES + 1) as usize]).unwrap();
        assert!(import_svg(&big).is_err());
        let binary = dir.path().join("bin.svg");
        std::fs::write(&binary, [0xff, 0xfe, 0x00, 0x3c]).unwrap();
        assert!(import_svg(&binary).is_err());
        assert!(import_svg(dir.path()).is_err());
        assert!(import_svg(&dir.path().join("missing.svg")).is_err());
    }

    // -----------------------------------------------------------------------------------------
    // Hostile nesting and many references
    // -----------------------------------------------------------------------------------------

    /// Runs `work` on a thread with the given stack and gives up on it after `seconds`, so that a
    /// regression is a failed test and not a test run that never ends.
    fn on_a_thread<T: Send + 'static>(
        stack: usize,
        seconds: u64,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .stack_size(stack)
            .spawn(move || {
                let _ = sender.send(work());
            })
            .unwrap();
        receiver
            .recv_timeout(std::time::Duration::from_secs(seconds))
            .unwrap_or_else(|error| panic!("the import did not finish: {error:?}"))
    }

    /// A 20 x 20 SVG whose body is `inner`.
    fn svg_with(inner: &str) -> String {
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20">{inner}</svg>"#)
    }

    /// `levels` nested `open` elements around a rectangle; `close` ends one.
    fn nested(open: &str, close: &str, levels: usize) -> String {
        svg_with(&format!(
            r#"{}<rect width="4" height="4"/>{}"#,
            open.repeat(levels),
            close.repeat(levels)
        ))
    }

    const WORKER_STACK: usize = 2 * 1024 * 1024;

    #[test]
    fn nesting_that_would_overflow_the_parsers_stack_is_refused_unread() {
        // A few thousand levels is a few kilobytes of text. The parser recurses once per open
        // element and the node limit is the only thing that stops it, so on a worker's 2 MiB stack
        // this aborted the process. The stack here is large enough for the old code to finish, so
        // that the old code fails the assertion instead of taking the test run down with it.
        for (open, close) in [
            ("<g>", "</g>"),
            ("<a>", "</a>"),
            ("<defs>", "</defs>"),
            ("<x>", "</x>"),
        ] {
            let svg = nested(open, close, 5000);
            let result = on_a_thread(64 * 1024 * 1024, 10, move || {
                import_svg_text(&svg).map(|_| ())
            });
            let error = result.expect_err(open).to_string();
            assert!(
                error.contains("nests elements more than"),
                "{open}: {error}"
            );
        }
    }

    #[test]
    fn a_file_at_the_depth_limit_imports_on_a_workers_stack_and_one_level_more_is_refused() {
        // `<svg>` and the rectangle count as levels too.
        let at_the_limit = nested("<g>", "</g>", MAX_ELEMENT_DEPTH - 2);
        let imported = on_a_thread(WORKER_STACK, 10, move || {
            import_svg_text(&at_the_limit).unwrap()
        });
        assert_eq!(imported.report.layers_imported, 1);
        imported.project.document.validate().unwrap();
        let one_more = nested("<g>", "</g>", MAX_ELEMENT_DEPTH - 1);
        assert!(import_svg_text(&one_more).is_err());
        // A start tag that closes itself is still a level while it is read.
        let leaf = svg_with(&format!(
            "{}<g/>{}",
            "<g>".repeat(MAX_ELEMENT_DEPTH - 2),
            "</g>".repeat(MAX_ELEMENT_DEPTH - 2)
        ));
        assert!(import_svg_text(&leaf).is_ok());
        let leaf = svg_with(&format!(
            "{}<g/>{}",
            "<g>".repeat(MAX_ELEMENT_DEPTH - 1),
            "</g>".repeat(MAX_ELEMENT_DEPTH - 1)
        ));
        assert!(import_svg_text(&leaf).is_err());
    }

    #[test]
    fn nesting_is_counted_the_way_the_parser_reads_markup() {
        let deep = "<g>".repeat(MAX_ELEMENT_DEPTH + 10);
        // Markup inside a comment, CDATA, a processing instruction or text is not an element, and
        // siblings and a quoted `>` or `/>` do not stack.
        let quiet = svg_with(&format!(
            "<!-- {deep} --><title><![CDATA[{deep}]]></title><?note {deep}?>{}<g a=\"/>\" b='>'/><g></g >",
            "<g/>".repeat(10_000)
        ));
        assert!(import_svg_text(&quiet).is_ok());
        // But a quoted `/>` does not close a tag that is still open.
        let quoted = nested(r#"<g title="/>">"#, "</g>", MAX_ELEMENT_DEPTH);
        assert!(import_svg_text(&quoted).is_err());
        let quoted = nested(r#"<g title='>'>"#, "</g>", MAX_ELEMENT_DEPTH);
        assert!(import_svg_text(&quoted).is_err());
        // Depth comes back down after a close, so a long file of shallow groups is fine.
        let flat = svg_with(&"<g><g><g></g></g></g>".repeat(5_000));
        assert!(import_svg_text(&flat).is_ok());
        // Text the parser refuses is still refused, whatever the scan made of it.
        for broken in [
            "<svg",
            "<svg width=\"5",
            "<svg><!-- ",
            "<svg><![CDATA[",
            "<svg><?x ",
            "<!DOCTYPE svg><svg/>",
            "</g></g></g>",
        ] {
            assert!(import_svg_text(broken).is_err(), "{broken}");
        }
    }

    #[test]
    fn the_walk_stops_on_its_own_at_the_depth_limit() {
        // Deeper than the limit but shallow enough for the parser, which `import_svg_text` would
        // have refused first; the walk must still not follow it down.
        let svg = nested("<g>", "</g>", MAX_ELEMENT_DEPTH + 30);
        let document = Document::parse_with_options(
            &svg,
            ParsingOptions {
                allow_dtd: false,
                nodes_limit: MAX_NODES,
            },
        )
        .unwrap();
        let mut importer = Importer {
            ids: HashMap::new(),
            gradients: RefCell::default(),
            stops: RefCell::default(),
            report: SvgImportReport::default(),
            layers: 0,
            counter: 0,
            canvas: (20.0, 20.0),
        };
        let mut layers = Vec::new();
        importer.walk(
            document.root_element(),
            &Style::default(),
            &Affine::IDENTITY,
            0,
            &mut layers,
        );
        assert!(has(
            &importer.report,
            ReportKind::NotImported,
            "nested more than"
        ));
        assert_eq!(importer.layers, 0);
    }

    #[test]
    fn ten_thousand_paint_references_do_not_each_scan_the_file() {
        // Each reference to an id that is not there used to read every node of the file, so
        // references times nodes was well over a billion steps for a file of under a megabyte.
        let svg = svg_with(&format!(
            r##"<defs><linearGradient id="a"><stop offset="0" stop-color="#f80"/></linearGradient></defs>{}{}"##,
            "<!---->".repeat(150_000),
            r##"<g fill="url(#a)" stroke="url(#missing)"/>"##.repeat(10_000)
        ));
        let imported = on_a_thread(WORKER_STACK, 5, move || import_svg_text(&svg).unwrap());
        let count = |text: &str| {
            imported
                .report
                .entries
                .iter()
                .filter(|entry| entry.detail.contains(text))
                .count()
        };
        assert_eq!(count("first colour"), 10_000);
        assert_eq!(count("not in the file"), 10_000);
    }

    #[test]
    fn many_gradients_that_share_a_heavy_holder_read_it_once() {
        // The holder has thousands of children before its first stop, and every gradient takes its
        // stops from it by `href`: finding the stop again for each paint would be a scan apiece.
        let svg = svg_with(&format!(
            r##"<defs><linearGradient id="a">{}<stop stop-color="#f80"/></linearGradient>{}</defs>{}"##,
            "<x/>".repeat(20_000),
            (0..20_000)
                .map(|i| format!(r##"<linearGradient id="g{i}" href="#a"/>"##))
                .collect::<String>(),
            (0..20_000)
                .map(|i| format!(r##"<g fill="url(#g{i})"/>"##))
                .collect::<String>(),
        ));
        let imported = on_a_thread(WORKER_STACK, 5, move || import_svg_text(&svg).unwrap());
        let found = imported
            .report
            .entries
            .iter()
            .filter(|entry| entry.detail.contains("first colour"))
            .count();
        assert_eq!(found, 20_000);
    }

    #[test]
    fn a_paint_names_the_first_element_with_its_id_and_follows_one_href_only() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100">
            <defs>
              <linearGradient id="base"><stop offset="0" stop-color="#ff0000" stop-opacity="0.5"/></linearGradient>
              <linearGradient id="one" href="#base"/>
              <radialGradient id="legacy" xlink:href="#base"/>
              <linearGradient id="two" href="#one"/>
              <linearGradient id="loop-a" href="#loop-b"/>
              <linearGradient id="loop-b" href="#loop-a"/>
              <linearGradient id="itself" href="#itself"/>
              <linearGradient id="dup"><stop stop-color="#00ff00"/></linearGradient>
              <linearGradient id="dup"><stop stop-color="#0000ff"/></linearGradient>
              <linearGradient id="empty"/>
            </defs>
            <rect id="r-base" width="1" height="1" fill="url(#base)"/>
            <rect id="r-one" width="1" height="1" fill="url(#one)"/>
            <rect id="r-legacy" width="1" height="1" fill="url('#legacy')"/>
            <rect id="r-two" width="1" height="1" fill="url(#two)"/>
            <rect id="r-loop" width="1" height="1" fill="url(#loop-a)"/>
            <rect id="r-itself" width="1" height="1" fill="url(#itself)"/>
            <rect id="r-dup" width="1" height="1" fill="url(#dup)"/>
            <rect id="r-empty" width="1" height="1" fill="url(#empty)"/>
            <rect id="r-shape" width="1" height="1" fill="url(#r-base)"/>
            <rect id="r-missing" width="1" height="1" fill="url(#nothing)"/>
            <rect id="r-again" width="1" height="1" fill="url(#one)"/>
        </svg>"##;
        let imported = import(svg);
        let fills: Vec<Option<ShapeColor>> =
            shapes(&imported).iter().map(|shape| shape.fill).collect();
        let red_at_half = Some(ShapeColor::new(1.0, 0.0, 0.0, 0.5));
        assert_eq!(
            fills,
            vec![
                red_at_half,                               // base
                red_at_half,                               // one: one hop to base
                red_at_half,                               // legacy: the xlink spelling
                None,                                      // two: a second hop is not followed
                None,                                      // a loop ends after one hop
                None,                                      // so does a gradient that names itself
                Some(ShapeColor::new(0.0, 1.0, 0.0, 1.0)), // dup: the first element with the id
                None,                                      // no stops
                None,                                      // the id of a rectangle
                None,                                      // no such id
                red_at_half,                               // asked a second time, answered the same
            ]
        );
    }
}
