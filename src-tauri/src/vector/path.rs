//! Vector geometry: a path model, the semantic shapes that generate one, and
//! the flattening that turns curves into line segments.
//!
//! Shapes keep their meaning. A rounded rectangle stores a corner radius and a
//! star stores a point count, rather than being flattened to anonymous
//! coordinates the moment they are created — that is what lets the user edit
//! the shape afterwards instead of editing its debris.
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Largest number of commands a single path may hold.
///
/// Project files are untrusted and a path is a list the file chooses the length
/// of, so it needs a ceiling. Large enough for hand-drawn work, small enough
/// that flattening one cannot exhaust memory.
pub const MAX_PATH_COMMANDS: usize = 20_000;
/// Largest coordinate magnitude accepted, in document pixels.
///
/// Comfortably outside any canvas the document model allows, and finite, which
/// is the part that matters: an infinity here becomes a NaN in the rasteriser.
pub const MAX_COORDINATE: f32 = 1_000_000.0;
/// Most sides or points a generated polygon or star may have.
pub const MAX_POLYGON_SIDES: u32 = 512;

/// One step of a path.
///
/// Quadratic curves are deliberately absent: they are converted to cubics when
/// a path is built, so the renderer has one curve type to reason about rather
/// than two.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PathCommand {
    MoveTo {
        x: f32,
        y: f32,
    },
    LineTo {
        x: f32,
        y: f32,
    },
    CubicTo {
        c1x: f32,
        c1y: f32,
        c2x: f32,
        c2y: f32,
        x: f32,
        y: f32,
    },
    Close,
}

impl PathCommand {
    /// Every coordinate this command carries.
    fn coordinates(&self) -> [f32; 6] {
        match *self {
            Self::MoveTo { x, y } | Self::LineTo { x, y } => [x, y, 0.0, 0.0, 0.0, 0.0],
            Self::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => [c1x, c1y, c2x, c2y, x, y],
            Self::Close => [0.0; 6],
        }
    }

    fn is_finite_and_bounded(&self) -> bool {
        self.coordinates()
            .iter()
            .all(|value| value.is_finite() && value.abs() <= MAX_COORDINATE)
    }
}

/// A sequence of subpaths.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorPath {
    pub commands: Vec<PathCommand>,
}

impl VectorPath {
    pub fn new(commands: Vec<PathCommand>) -> Self {
        Self { commands }
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.commands.len() > MAX_PATH_COMMANDS {
            return Err(AppError::InvalidLayerDocument(format!(
                "a path may hold at most {MAX_PATH_COMMANDS} commands"
            )));
        }
        if !self.commands.iter().all(PathCommand::is_finite_and_bounded) {
            return Err(AppError::InvalidLayerDocument(
                "a path coordinate was not finite or was outside the supported range".into(),
            ));
        }
        Ok(())
    }

    /// Line segments approximating this path, in document coordinates.
    ///
    /// `tolerance` is the largest distance a flattened segment may sit from the
    /// true curve, in the same units as the coordinates. Each returned vector is
    /// one closed or open subpath.
    pub fn flatten(&self, tolerance: f32) -> Vec<Vec<(f32, f32)>> {
        let tolerance = tolerance.clamp(0.01, 10.0);
        let mut subpaths: Vec<Vec<(f32, f32)>> = Vec::new();
        let mut current: Vec<(f32, f32)> = Vec::new();
        let mut cursor = (0.0f32, 0.0f32);
        let mut start = (0.0f32, 0.0f32);

        for command in &self.commands {
            match *command {
                PathCommand::MoveTo { x, y } => {
                    if current.len() > 1 {
                        subpaths.push(std::mem::take(&mut current));
                    } else {
                        current.clear();
                    }
                    cursor = (x, y);
                    start = cursor;
                    current.push(cursor);
                }
                PathCommand::LineTo { x, y } => {
                    if current.is_empty() {
                        current.push(cursor);
                    }
                    cursor = (x, y);
                    current.push(cursor);
                }
                PathCommand::CubicTo {
                    c1x,
                    c1y,
                    c2x,
                    c2y,
                    x,
                    y,
                } => {
                    if current.is_empty() {
                        current.push(cursor);
                    }
                    flatten_cubic(
                        cursor,
                        (c1x, c1y),
                        (c2x, c2y),
                        (x, y),
                        tolerance,
                        &mut current,
                    );
                    cursor = (x, y);
                }
                PathCommand::Close => {
                    if current.len() > 1 {
                        if current.first() != current.last() {
                            current.push(start);
                        }
                        subpaths.push(std::mem::take(&mut current));
                    } else {
                        current.clear();
                    }
                    cursor = start;
                }
            }
        }
        if current.len() > 1 {
            subpaths.push(current);
        }
        subpaths
    }

    /// The tight bounding box of the flattened path, or `None` when empty.
    pub fn bounds(&self, tolerance: f32) -> Option<(f32, f32, f32, f32)> {
        let mut bounds: Option<(f32, f32, f32, f32)> = None;
        for subpath in self.flatten(tolerance) {
            for (x, y) in subpath {
                bounds = Some(match bounds {
                    None => (x, y, x, y),
                    Some((min_x, min_y, max_x, max_y)) => {
                        (min_x.min(x), min_y.min(y), max_x.max(x), max_y.max(y))
                    }
                });
            }
        }
        bounds
    }
}

/// Subdivides a cubic until it is flat enough, appending points to `out`.
///
/// Flatness is measured as the distance of the control points from the chord,
/// which is the standard conservative test: if both controls lie within
/// `tolerance` of the line, so does the whole curve.
fn flatten_cubic(
    p0: (f32, f32),
    p1: (f32, f32),
    p2: (f32, f32),
    p3: (f32, f32),
    tolerance: f32,
    out: &mut Vec<(f32, f32)>,
) {
    // Depth-limited rather than recursive without bound: a degenerate curve
    // with coincident control points can otherwise subdivide forever.
    fn recurse(
        p0: (f32, f32),
        p1: (f32, f32),
        p2: (f32, f32),
        p3: (f32, f32),
        tolerance: f32,
        depth: u32,
        out: &mut Vec<(f32, f32)>,
    ) {
        let (dx, dy) = (p3.0 - p0.0, p3.1 - p0.1);
        let d1 = ((p1.0 - p3.0) * dy - (p1.1 - p3.1) * dx).abs();
        let d2 = ((p2.0 - p3.0) * dy - (p2.1 - p3.1) * dx).abs();
        let sum = d1 + d2;
        if depth >= 16 || sum * sum <= tolerance * (dx * dx + dy * dy) {
            out.push(p3);
            return;
        }
        let mid = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
        let p01 = mid(p0, p1);
        let p12 = mid(p1, p2);
        let p23 = mid(p2, p3);
        let p012 = mid(p01, p12);
        let p123 = mid(p12, p23);
        let centre = mid(p012, p123);
        recurse(p0, p01, p012, centre, tolerance, depth + 1, out);
        recurse(centre, p123, p23, p3, tolerance, depth + 1, out);
    }
    recurse(p0, p1, p2, p3, tolerance, 0, out);
}

/// A shape that keeps its meaning.
///
/// A rectangle is stored as a rectangle, not as four points, so that changing
/// its corner radius later is editing a parameter rather than reconstructing
/// geometry from its remains.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ShapeGeometry {
    Rectangle {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        /// Zero for a square corner. Clamped to half the shorter side.
        #[serde(default)]
        corner_radius: f32,
    },
    Ellipse {
        cx: f32,
        cy: f32,
        rx: f32,
        ry: f32,
    },
    Line {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
    },
    Polygon {
        cx: f32,
        cy: f32,
        radius: f32,
        sides: u32,
        #[serde(default)]
        rotation_degrees: f32,
    },
    Star {
        cx: f32,
        cy: f32,
        outer_radius: f32,
        inner_radius: f32,
        points: u32,
        #[serde(default)]
        rotation_degrees: f32,
    },
    /// Arbitrary geometry, for the pen tool and for shapes that have been
    /// converted away from their parametric form.
    Path {
        path: VectorPath,
    },
}

/// The circle-to-cubic constant: four curves with control points this far along
/// the tangents approximate a circle to within about 0.02%.
const KAPPA: f32 = 0.552_284_8;

impl ShapeGeometry {
    pub fn validate(&self) -> Result<(), AppError> {
        let finite = |values: &[f32]| {
            values
                .iter()
                .all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)
        };
        let bad = |what: &str| {
            Err(AppError::InvalidLayerDocument(format!(
                "a shape's {what} was not finite or was outside the supported range"
            )))
        };
        match self {
            Self::Rectangle {
                x,
                y,
                width,
                height,
                corner_radius,
            } => {
                if !finite(&[*x, *y, *width, *height, *corner_radius]) {
                    return bad("rectangle");
                }
                if *width <= 0.0 || *height <= 0.0 || *corner_radius < 0.0 {
                    return bad("rectangle size");
                }
            }
            Self::Ellipse { cx, cy, rx, ry } => {
                if !finite(&[*cx, *cy, *rx, *ry]) {
                    return bad("ellipse");
                }
                if *rx <= 0.0 || *ry <= 0.0 {
                    return bad("ellipse radius");
                }
            }
            Self::Line { x1, y1, x2, y2 } => {
                if !finite(&[*x1, *y1, *x2, *y2]) {
                    return bad("line");
                }
            }
            Self::Polygon {
                cx,
                cy,
                radius,
                sides,
                rotation_degrees,
            } => {
                if !finite(&[*cx, *cy, *radius, *rotation_degrees]) {
                    return bad("polygon");
                }
                if *radius <= 0.0 || !(3..=MAX_POLYGON_SIDES).contains(sides) {
                    return bad("polygon size");
                }
            }
            Self::Star {
                cx,
                cy,
                outer_radius,
                inner_radius,
                points,
                rotation_degrees,
            } => {
                if !finite(&[*cx, *cy, *outer_radius, *inner_radius, *rotation_degrees]) {
                    return bad("star");
                }
                if *outer_radius <= 0.0
                    || *inner_radius <= 0.0
                    || *inner_radius > *outer_radius
                    || !(3..=MAX_POLYGON_SIDES).contains(points)
                {
                    return bad("star size");
                }
            }
            Self::Path { path } => path.validate()?,
        }
        Ok(())
    }

    /// The path this shape describes.
    pub fn to_path(&self) -> VectorPath {
        match self {
            Self::Rectangle {
                x,
                y,
                width,
                height,
                corner_radius,
            } => rectangle_path(*x, *y, *width, *height, *corner_radius),
            Self::Ellipse { cx, cy, rx, ry } => ellipse_path(*cx, *cy, *rx, *ry),
            Self::Line { x1, y1, x2, y2 } => VectorPath::new(vec![
                PathCommand::MoveTo { x: *x1, y: *y1 },
                PathCommand::LineTo { x: *x2, y: *y2 },
            ]),
            Self::Polygon {
                cx,
                cy,
                radius,
                sides,
                rotation_degrees,
            } => radial_path(*cx, *cy, &[*radius], *sides, *rotation_degrees),
            Self::Star {
                cx,
                cy,
                outer_radius,
                inner_radius,
                points,
                rotation_degrees,
            } => radial_path(
                *cx,
                *cy,
                &[*outer_radius, *inner_radius],
                *points,
                *rotation_degrees,
            ),
            Self::Path { path } => path.clone(),
        }
    }

    /// Whether this shape encloses an area that can be filled.
    ///
    /// A line does not, which is why a line with only a fill and no stroke draws
    /// nothing rather than something surprising.
    pub const fn is_closed(&self) -> bool {
        !matches!(self, Self::Line { .. })
    }
}

fn rectangle_path(x: f32, y: f32, width: f32, height: f32, radius: f32) -> VectorPath {
    let radius = radius.min(width * 0.5).min(height * 0.5).max(0.0);
    let (right, bottom) = (x + width, y + height);
    if radius <= 0.0 {
        return VectorPath::new(vec![
            PathCommand::MoveTo { x, y },
            PathCommand::LineTo { x: right, y },
            PathCommand::LineTo {
                x: right,
                y: bottom,
            },
            PathCommand::LineTo { x, y: bottom },
            PathCommand::Close,
        ]);
    }
    let c = radius * KAPPA;
    VectorPath::new(vec![
        PathCommand::MoveTo { x: x + radius, y },
        PathCommand::LineTo {
            x: right - radius,
            y,
        },
        PathCommand::CubicTo {
            c1x: right - radius + c,
            c1y: y,
            c2x: right,
            c2y: y + radius - c,
            x: right,
            y: y + radius,
        },
        PathCommand::LineTo {
            x: right,
            y: bottom - radius,
        },
        PathCommand::CubicTo {
            c1x: right,
            c1y: bottom - radius + c,
            c2x: right - radius + c,
            c2y: bottom,
            x: right - radius,
            y: bottom,
        },
        PathCommand::LineTo {
            x: x + radius,
            y: bottom,
        },
        PathCommand::CubicTo {
            c1x: x + radius - c,
            c1y: bottom,
            c2x: x,
            c2y: bottom - radius + c,
            x,
            y: bottom - radius,
        },
        PathCommand::LineTo { x, y: y + radius },
        PathCommand::CubicTo {
            c1x: x,
            c1y: y + radius - c,
            c2x: x + radius - c,
            c2y: y,
            x: x + radius,
            y,
        },
        PathCommand::Close,
    ])
}

fn ellipse_path(cx: f32, cy: f32, rx: f32, ry: f32) -> VectorPath {
    let (ox, oy) = (rx * KAPPA, ry * KAPPA);
    VectorPath::new(vec![
        PathCommand::MoveTo { x: cx, y: cy - ry },
        PathCommand::CubicTo {
            c1x: cx + ox,
            c1y: cy - ry,
            c2x: cx + rx,
            c2y: cy - oy,
            x: cx + rx,
            y: cy,
        },
        PathCommand::CubicTo {
            c1x: cx + rx,
            c1y: cy + oy,
            c2x: cx + ox,
            c2y: cy + ry,
            x: cx,
            y: cy + ry,
        },
        PathCommand::CubicTo {
            c1x: cx - ox,
            c1y: cy + ry,
            c2x: cx - rx,
            c2y: cy + oy,
            x: cx - rx,
            y: cy,
        },
        PathCommand::CubicTo {
            c1x: cx - rx,
            c1y: cy - oy,
            c2x: cx - ox,
            c2y: cy - ry,
            x: cx,
            y: cy - ry,
        },
        PathCommand::Close,
    ])
}

/// A closed path stepping through `radii` in turn around `count` positions.
///
/// One radius gives a regular polygon; two alternating radii give a star.
fn radial_path(cx: f32, cy: f32, radii: &[f32], count: u32, rotation_degrees: f32) -> VectorPath {
    let steps = count as usize * radii.len();
    let mut commands = Vec::with_capacity(steps + 2);
    let rotation = rotation_degrees.to_radians();
    for step in 0..steps {
        // Starting at the top, which is what a user drawing a star expects.
        let angle = rotation - std::f32::consts::FRAC_PI_2
            + std::f32::consts::TAU * step as f32 / steps as f32;
        let radius = radii[step % radii.len()];
        let (x, y) = (cx + angle.cos() * radius, cy + angle.sin() * radius);
        commands.push(if step == 0 {
            PathCommand::MoveTo { x, y }
        } else {
            PathCommand::LineTo { x, y }
        });
    }
    commands.push(PathCommand::Close);
    VectorPath::new(commands)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area_of(points: &[(f32, f32)]) -> f32 {
        // Shoelace, absolute.
        let mut sum = 0.0;
        for index in 0..points.len() {
            let (x0, y0) = points[index];
            let (x1, y1) = points[(index + 1) % points.len()];
            sum += x0 * y1 - x1 * y0;
        }
        (sum * 0.5).abs()
    }

    /// A flattened circle has to enclose the area a circle encloses, or the
    /// curve approximation is wrong in a way no visual check would catch.
    #[test]
    fn a_flattened_ellipse_has_the_right_area() {
        let shape = ShapeGeometry::Ellipse {
            cx: 0.0,
            cy: 0.0,
            rx: 100.0,
            ry: 100.0,
        };
        let subpaths = shape.to_path().flatten(0.05);
        assert_eq!(subpaths.len(), 1);
        let area = area_of(&subpaths[0]);
        let expected = std::f32::consts::PI * 100.0 * 100.0;
        assert!(
            (area - expected).abs() / expected < 0.001,
            "flattened area {area} against {expected}"
        );
    }

    #[test]
    fn a_rectangle_flattens_to_its_own_corners() {
        let shape = ShapeGeometry::Rectangle {
            x: 10.0,
            y: 20.0,
            width: 30.0,
            height: 40.0,
            corner_radius: 0.0,
        };
        let subpaths = shape.to_path().flatten(0.1);
        assert_eq!(subpaths.len(), 1);
        let area = area_of(&subpaths[0]);
        assert!((area - 1200.0).abs() < 0.5, "area {area}");
        let bounds = shape.to_path().bounds(0.1).unwrap();
        assert_eq!(bounds, (10.0, 20.0, 40.0, 60.0));
    }

    /// A corner radius must actually round the corner, and must be clamped
    /// rather than allowed to invert the shape.
    #[test]
    fn corner_radius_rounds_and_is_clamped() {
        let square = ShapeGeometry::Rectangle {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            corner_radius: 0.0,
        };
        let rounded = ShapeGeometry::Rectangle {
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            corner_radius: 20.0,
        };
        let square_area = area_of(&square.to_path().flatten(0.05)[0]);
        let rounded_area = area_of(&rounded.to_path().flatten(0.05)[0]);
        // Four quarter-circles replace four corners: 400 - 100*pi lost.
        let expected_loss = 4.0 * 400.0 - std::f32::consts::PI * 400.0;
        assert!(
            ((square_area - rounded_area) - expected_loss).abs() < 20.0,
            "lost {} against an expected {expected_loss}",
            square_area - rounded_area
        );

        // A radius larger than the shape becomes a stadium, not an inversion.
        let over = ShapeGeometry::Rectangle {
            x: 0.0,
            y: 0.0,
            width: 40.0,
            height: 100.0,
            corner_radius: 500.0,
        };
        let bounds = over.to_path().bounds(0.05).unwrap();
        assert!(bounds.0 >= -0.01 && bounds.2 <= 40.01, "bounds {bounds:?}");
    }

    #[test]
    fn a_star_alternates_between_its_two_radii() {
        let star = ShapeGeometry::Star {
            cx: 0.0,
            cy: 0.0,
            outer_radius: 50.0,
            inner_radius: 20.0,
            points: 5,
            rotation_degrees: 0.0,
        };
        let subpaths = star.to_path().flatten(0.05);
        let points = &subpaths[0];
        // Ten vertices plus the closing repeat.
        assert!(points.len() >= 10, "only {} points", points.len());
        let radii: Vec<f32> = points[..10]
            .iter()
            .map(|(x, y)| (x * x + y * y).sqrt())
            .collect();
        for (index, radius) in radii.iter().enumerate() {
            let expected = if index % 2 == 0 { 50.0 } else { 20.0 };
            assert!(
                (radius - expected).abs() < 0.01,
                "vertex {index} at radius {radius}, expected {expected}"
            );
        }
    }

    /// Project files are untrusted, so every hostile shape must be refused.
    #[test]
    fn hostile_geometry_is_refused() {
        let cases = vec![
            ShapeGeometry::Rectangle {
                x: f32::NAN,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                corner_radius: 0.0,
            },
            ShapeGeometry::Rectangle {
                x: 0.0,
                y: 0.0,
                width: f32::INFINITY,
                height: 10.0,
                corner_radius: 0.0,
            },
            ShapeGeometry::Rectangle {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 10.0,
                corner_radius: 0.0,
            },
            ShapeGeometry::Rectangle {
                x: 1e12,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                corner_radius: 0.0,
            },
            ShapeGeometry::Ellipse {
                cx: 0.0,
                cy: 0.0,
                rx: -5.0,
                ry: 10.0,
            },
            ShapeGeometry::Polygon {
                cx: 0.0,
                cy: 0.0,
                radius: 10.0,
                sides: 2,
                rotation_degrees: 0.0,
            },
            ShapeGeometry::Polygon {
                cx: 0.0,
                cy: 0.0,
                radius: 10.0,
                sides: 100_000,
                rotation_degrees: 0.0,
            },
            ShapeGeometry::Star {
                cx: 0.0,
                cy: 0.0,
                outer_radius: 10.0,
                inner_radius: 50.0,
                points: 5,
                rotation_degrees: 0.0,
            },
            ShapeGeometry::Path {
                path: VectorPath::new(vec![PathCommand::MoveTo {
                    x: f32::NAN,
                    y: 0.0,
                }]),
            },
            ShapeGeometry::Path {
                path: VectorPath::new(vec![
                    PathCommand::LineTo { x: 0.0, y: 0.0 };
                    MAX_PATH_COMMANDS + 1
                ]),
            },
        ];
        for shape in cases {
            assert!(shape.validate().is_err(), "{shape:?} was accepted");
        }
    }

    /// A degenerate curve must not subdivide forever.
    #[test]
    fn a_degenerate_curve_terminates() {
        let path = VectorPath::new(vec![
            PathCommand::MoveTo { x: 0.0, y: 0.0 },
            PathCommand::CubicTo {
                c1x: 0.0,
                c1y: 0.0,
                c2x: 0.0,
                c2y: 0.0,
                x: 0.0,
                y: 0.0,
            },
            PathCommand::Close,
        ]);
        let subpaths = path.flatten(0.05);
        let total: usize = subpaths.iter().map(Vec::len).sum();
        assert!(
            total < 200_000,
            "a degenerate curve produced {total} points"
        );
    }

    /// An open subpath stays open, and a closed one closes.
    #[test]
    fn closing_a_subpath_returns_to_its_start() {
        let path = VectorPath::new(vec![
            PathCommand::MoveTo { x: 0.0, y: 0.0 },
            PathCommand::LineTo { x: 10.0, y: 0.0 },
            PathCommand::LineTo { x: 10.0, y: 10.0 },
            PathCommand::Close,
        ]);
        let subpaths = path.flatten(0.05);
        assert_eq!(subpaths.len(), 1);
        assert_eq!(subpaths[0].first(), subpaths[0].last());

        let open = VectorPath::new(vec![
            PathCommand::MoveTo { x: 0.0, y: 0.0 },
            PathCommand::LineTo { x: 10.0, y: 0.0 },
        ]);
        let subpaths = open.flatten(0.05);
        assert_eq!(subpaths.len(), 1);
        assert_ne!(subpaths[0].first(), subpaths[0].last());
    }
}
