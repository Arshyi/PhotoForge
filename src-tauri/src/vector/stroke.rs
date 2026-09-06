//! Turning a stroke into an area the rasteriser can fill.
//!
//! A stroke is not a primitive here. Each segment contributes a quadrilateral
//! of the requested width, each joint contributes a join shape and each open
//! end contributes a cap, and the whole collection is filled with the non-zero
//! rule so the overlaps merge into one outline. That is simpler and more robust
//! than computing a single offset outline, which has to reason about
//! self-intersection at every concave joint, and it produces the same picture.
use serde::{Deserialize, Serialize};

/// How an open end of a stroke is finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineCap {
    /// Stops exactly at the endpoint.
    #[default]
    Butt,
    /// A half-disc beyond the endpoint.
    Round,
    /// A half-square beyond the endpoint.
    Square,
}

/// How two segments meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LineJoin {
    /// A disc at the joint. Never spikes, whatever the angle.
    #[default]
    Round,
    /// A triangle across the outside of the joint.
    Bevel,
    /// Extended to a point, falling back to bevel past the miter limit — which
    /// is what stops a nearly-doubled-back joint growing a spike the length of
    /// the document.
    Miter,
}

/// Everything needed to draw a stroke.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrokeStyle {
    pub width: f32,
    #[serde(default)]
    pub cap: LineCap,
    #[serde(default)]
    pub join: LineJoin,
    #[serde(default = "default_miter_limit")]
    pub miter_limit: f32,
}

fn default_miter_limit() -> f32 {
    4.0
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self {
            width: 1.0,
            cap: LineCap::default(),
            join: LineJoin::default(),
            miter_limit: default_miter_limit(),
        }
    }
}

/// Largest stroke width accepted, in device pixels.
///
/// A stroke wider than any canvas is not a stroke, and its generated geometry
/// would be proportional to the width.
pub const MAX_STROKE_WIDTH: f32 = 4_096.0;

impl StrokeStyle {
    pub fn is_valid(&self) -> bool {
        self.width.is_finite()
            && self.width > 0.0
            && self.width <= MAX_STROKE_WIDTH
            && self.miter_limit.is_finite()
            && (1.0..=100.0).contains(&self.miter_limit)
    }
}

/// How many segments approximate a full circle in a round join or cap.
///
/// Fixed rather than adaptive so that the geometry a stroke generates depends
/// only on its own parameters — the tiled renderer needs the same outline
/// whichever rectangle asked for it.
const ARC_SEGMENTS: usize = 24;

/// Ensures a polygon winds anticlockwise, reversing it if not.
///
/// This is what makes the union work. Under the non-zero rule two overlapping
/// polygons merge only if they wind the same way; wound oppositely they cancel,
/// and the overlap becomes a hole. A round cap generated the right disc in the
/// right place and contributed exactly nothing, because the disc ran one way
/// and the segment quad the other.
fn wind_consistently(mut polygon: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    let mut twice_area = 0.0f32;
    for index in 0..polygon.len() {
        let (x0, y0) = polygon[index];
        let (x1, y1) = polygon[(index + 1) % polygon.len()];
        twice_area += x0 * y1 - x1 * y0;
    }
    if twice_area < 0.0 {
        polygon.reverse();
    }
    polygon
}

/// Builds fillable outlines covering the stroke of `subpaths`.
///
/// Every returned polygon is wound the same way, so filling them together with
/// the non-zero rule yields their union rather than their exclusive-or.
pub fn stroke_outlines(subpaths: &[Vec<(f32, f32)>], style: &StrokeStyle) -> Vec<Vec<(f32, f32)>> {
    if !style.is_valid() {
        return Vec::new();
    }
    let half = style.width * 0.5;
    let mut outlines: Vec<Vec<(f32, f32)>> = Vec::new();

    for subpath in subpaths {
        // Consecutive duplicates carry no direction and would produce a
        // zero-length normal.
        let points: Vec<(f32, f32)> = dedupe(subpath);
        if points.len() < 2 {
            // A subpath that is a single point still draws a dot under a round
            // cap, and nothing under the others.
            if points.len() == 1 && style.cap == LineCap::Round {
                outlines.push(disc(points[0], half));
            }
            continue;
        }
        let closed = points.first() == points.last() && points.len() > 2;
        let effective: &[(f32, f32)] = if closed {
            &points[..points.len() - 1]
        } else {
            &points
        };

        let count = effective.len();
        let last_segment = if closed { count } else { count - 1 };
        for index in 0..last_segment {
            let a = effective[index];
            let b = effective[(index + 1) % count];
            let Some((nx, ny)) = normal(a, b) else {
                continue;
            };
            let (ox, oy) = (nx * half, ny * half);
            // Extended ends for a square cap, but only at the true ends of an
            // open subpath.
            let (a, b) = if style.cap == LineCap::Square && !closed {
                let (dx, dy) = direction(a, b);
                (
                    if index == 0 {
                        (a.0 - dx * half, a.1 - dy * half)
                    } else {
                        a
                    },
                    if index == last_segment - 1 {
                        (b.0 + dx * half, b.1 + dy * half)
                    } else {
                        b
                    },
                )
            } else {
                (a, b)
            };
            outlines.push(vec![
                (a.0 + ox, a.1 + oy),
                (b.0 + ox, b.1 + oy),
                (b.0 - ox, b.1 - oy),
                (a.0 - ox, a.1 - oy),
            ]);
        }

        // Joins at every interior vertex, and at the seam of a closed subpath.
        let joint_range: Vec<usize> = if closed {
            (0..count).collect()
        } else {
            (1..count - 1).collect()
        };
        for index in joint_range {
            let previous = effective[(index + count - 1) % count];
            let current = effective[index];
            let next = effective[(index + 1) % count];
            outlines.extend(join_outline(previous, current, next, half, style));
        }

        // Caps on the ends of an open subpath. A square cap was handled by
        // extending the segment above, and a butt cap adds nothing.
        if !closed && style.cap == LineCap::Round {
            outlines.push(disc(effective[0], half));
            outlines.push(disc(effective[count - 1], half));
        }
    }
    outlines.into_iter().map(wind_consistently).collect()
}

fn dedupe(points: &[(f32, f32)]) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(points.len());
    for point in points {
        if !point.0.is_finite() || !point.1.is_finite() {
            continue;
        }
        match out.last() {
            Some(last) if distance_squared(*last, *point) < 1e-12 => {}
            _ => out.push(*point),
        }
    }
    out
}

fn distance_squared(a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    dx * dx + dy * dy
}

fn direction(a: (f32, f32), b: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length = (dx * dx + dy * dy).sqrt();
    if length <= 0.0 {
        (0.0, 0.0)
    } else {
        (dx / length, dy / length)
    }
}

/// The unit normal of the segment, or `None` when it has no length.
fn normal(a: (f32, f32), b: (f32, f32)) -> Option<(f32, f32)> {
    let (dx, dy) = direction(a, b);
    if dx == 0.0 && dy == 0.0 {
        None
    } else {
        Some((-dy, dx))
    }
}

fn disc(centre: (f32, f32), radius: f32) -> Vec<(f32, f32)> {
    (0..ARC_SEGMENTS)
        .map(|step| {
            let angle = std::f32::consts::TAU * step as f32 / ARC_SEGMENTS as f32;
            (
                centre.0 + angle.cos() * radius,
                centre.1 + angle.sin() * radius,
            )
        })
        .collect()
}

fn join_outline(
    previous: (f32, f32),
    current: (f32, f32),
    next: (f32, f32),
    half: f32,
    style: &StrokeStyle,
) -> Vec<Vec<(f32, f32)>> {
    let (Some(n0), Some(n1)) = (normal(previous, current), normal(current, next)) else {
        return Vec::new();
    };
    match style.join {
        LineJoin::Round => vec![disc(current, half)],
        LineJoin::Bevel => vec![bevel(current, n0, n1, half)],
        LineJoin::Miter => {
            // The half-angle between the segments decides how far the point
            // reaches. Past the limit a miter becomes a spike, so it degrades
            // to a bevel — which is what the limit is for.
            let cos_theta = (n0.0 * n1.0 + n0.1 * n1.1).clamp(-1.0, 1.0);
            let half_angle_cos = ((1.0 + cos_theta) * 0.5).max(0.0).sqrt();
            if half_angle_cos <= 1e-4 || 1.0 / half_angle_cos > style.miter_limit {
                return vec![bevel(current, n0, n1, half)];
            }
            // Bisector of the two outward normals, extended to the miter point.
            let (bx, by) = (n0.0 + n1.0, n0.1 + n1.1);
            let length = (bx * bx + by * by).sqrt();
            if length <= 1e-6 {
                return vec![bevel(current, n0, n1, half)];
            }
            let scale = half / half_angle_cos;
            let (ux, uy) = (bx / length, by / length);
            let side = |sign: f32| {
                vec![
                    current,
                    (
                        current.0 + n0.0 * half * sign,
                        current.1 + n0.1 * half * sign,
                    ),
                    (current.0 + ux * scale * sign, current.1 + uy * scale * sign),
                    (
                        current.0 + n1.0 * half * sign,
                        current.1 + n1.1 * half * sign,
                    ),
                ]
            };
            // Both sides: only the outer one has area to fill, and the inner
            // one lies inside the segment quads where it changes nothing.
            vec![side(1.0), side(-1.0)]
        }
    }
}

fn bevel(current: (f32, f32), n0: (f32, f32), n1: (f32, f32), half: f32) -> Vec<(f32, f32)> {
    vec![
        current,
        (current.0 + n0.0 * half, current.1 + n0.1 * half),
        (current.0 + n1.0 * half, current.1 + n1.1 * half),
    ]
}

#[cfg(test)]
mod tests {
    use super::super::raster::{rasterize, FillRule};
    use super::*;

    fn coverage_of(outlines: &[Vec<(f32, f32)>], size: u32) -> f64 {
        let mask = rasterize(outlines, 0, 0, size, size, FillRule::NonZero);
        mask.coverage.iter().map(|v| f64::from(*v)).sum()
    }

    /// A straight stroke covers length times width. This is the single
    /// strongest check: it catches a wrong half-width, a doubled quad and a
    /// missing segment at once.
    #[test]
    fn a_straight_stroke_covers_length_times_width() {
        let line = vec![vec![(20.0f32, 32.0f32), (80.0, 32.0)]];
        for width in [1.0f32, 4.0, 9.0] {
            let style = StrokeStyle {
                width,
                cap: LineCap::Butt,
                ..StrokeStyle::default()
            };
            let outlines = stroke_outlines(&line, &style);
            let covered = coverage_of(&outlines, 128);
            let expected = f64::from(60.0 * width);
            assert!(
                (covered - expected).abs() / expected < 0.02,
                "width {width} covered {covered} against {expected}"
            );
        }
    }

    /// Caps must differ from one another in exactly the way their names claim.
    #[test]
    fn caps_add_the_area_they_describe() {
        let line = vec![vec![(20.0f32, 32.0f32), (80.0, 32.0)]];
        let width = 10.0;
        let area = |cap: LineCap| {
            coverage_of(
                &stroke_outlines(
                    &line,
                    &StrokeStyle {
                        width,
                        cap,
                        ..StrokeStyle::default()
                    },
                ),
                128,
            )
        };
        let butt = area(LineCap::Butt);
        let round = area(LineCap::Round);
        let square = area(LineCap::Square);
        // Square adds a full width x half-width block at each end.
        assert!(
            (square - butt - f64::from(width * width)).abs() < 2.0,
            "square {square} against butt {butt}"
        );
        // Round adds a disc: two half-discs of radius width/2.
        let disc_area = f64::from(std::f32::consts::PI * (width * 0.5) * (width * 0.5));
        assert!(
            (round - butt - disc_area).abs() < 2.0,
            "round {round} against butt {butt} plus {disc_area}"
        );
        assert!(round < square, "a round cap covered more than a square one");
    }

    /// A closed stroke has no ends, so no cap may appear.
    #[test]
    fn a_closed_stroke_has_no_caps() {
        let square = vec![vec![
            (20.0f32, 20.0f32),
            (80.0, 20.0),
            (80.0, 80.0),
            (20.0, 80.0),
            (20.0, 20.0),
        ]];
        let butt = coverage_of(
            &stroke_outlines(
                &square,
                &StrokeStyle {
                    width: 6.0,
                    cap: LineCap::Butt,
                    ..StrokeStyle::default()
                },
            ),
            128,
        );
        let round = coverage_of(
            &stroke_outlines(
                &square,
                &StrokeStyle {
                    width: 6.0,
                    cap: LineCap::Round,
                    ..StrokeStyle::default()
                },
            ),
            128,
        );
        assert!(
            (butt - round).abs() < 0.5,
            "the cap style changed a closed stroke: {butt} against {round}"
        );
        // Perimeter 240 times width 6, less the four corners counted twice.
        assert!(
            (butt - 1440.0).abs() < 120.0,
            "a closed square stroke covered {butt}"
        );
    }

    /// The miter limit exists to stop a spike. A nearly doubled-back joint must
    /// fall back to a bevel rather than reaching across the canvas.
    #[test]
    fn the_miter_limit_prevents_a_spike() {
        let spike = vec![vec![(10.0f32, 60.0f32), (60.0, 59.0), (10.0, 58.0)]];
        let style = StrokeStyle {
            width: 6.0,
            join: LineJoin::Miter,
            miter_limit: 4.0,
            ..StrokeStyle::default()
        };
        let outlines = stroke_outlines(&spike, &style);
        let furthest = outlines
            .iter()
            .flatten()
            .map(|(x, _)| *x)
            .fold(f32::MIN, f32::max);
        assert!(
            furthest < 80.0,
            "the miter reached x={furthest}, far past the joint at x=60"
        );
    }

    /// Nothing here may produce geometry from nonsense.
    #[test]
    fn hostile_strokes_produce_nothing_rather_than_panicking() {
        let line = vec![vec![(10.0f32, 10.0f32), (50.0, 50.0)]];
        for style in [
            StrokeStyle {
                width: 0.0,
                ..StrokeStyle::default()
            },
            StrokeStyle {
                width: -5.0,
                ..StrokeStyle::default()
            },
            StrokeStyle {
                width: f32::NAN,
                ..StrokeStyle::default()
            },
            StrokeStyle {
                width: f32::INFINITY,
                ..StrokeStyle::default()
            },
            StrokeStyle {
                width: 1e9,
                ..StrokeStyle::default()
            },
            StrokeStyle {
                width: 4.0,
                miter_limit: f32::NAN,
                join: LineJoin::Miter,
                ..StrokeStyle::default()
            },
        ] {
            assert!(!style.is_valid(), "{style:?} passed validation");
            assert!(
                stroke_outlines(&line, &style).is_empty(),
                "{style:?} produced geometry"
            );
        }

        // Degenerate input under a valid style must also be safe.
        let degenerate = vec![
            vec![],
            vec![(5.0f32, 5.0f32)],
            vec![(5.0, 5.0), (5.0, 5.0), (5.0, 5.0)],
            vec![(f32::NAN, 0.0), (10.0, 10.0)],
        ];
        let style = StrokeStyle {
            width: 3.0,
            ..StrokeStyle::default()
        };
        let outlines = stroke_outlines(&degenerate, &style);
        for point in outlines.iter().flatten() {
            assert!(
                point.0.is_finite() && point.1.is_finite(),
                "produced a non-finite stroke point"
            );
        }
    }

    /// The generated outline may not depend on anything but the stroke itself,
    /// or a tile would stroke differently from the whole frame.
    #[test]
    fn the_same_stroke_always_generates_the_same_outline() {
        let path = vec![vec![(10.0f32, 10.0f32), (40.0, 70.0), (90.0, 25.0)]];
        let style = StrokeStyle {
            width: 7.0,
            cap: LineCap::Round,
            join: LineJoin::Miter,
            miter_limit: 4.0,
        };
        let first = stroke_outlines(&path, &style);
        let second = stroke_outlines(&path, &style);
        assert_eq!(first, second);
    }
}
