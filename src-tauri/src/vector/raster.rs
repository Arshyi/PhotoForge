//! Rasterising vector geometry to floating-point coverage.
//!
//! # Why this is written rather than taken from a crate
//!
//! tiny-skia is the obvious candidate and its mask is 8-bit: "we're using just
//! a simple 8bit alpha mask". PhotoForge composites in linear f32 precisely to
//! avoid quantisation of this kind, so an 8-bit coverage buffer would put a
//! 1/255 step on the edge of every shape in an application built to not have
//! one. Coverage here is f32 and feeds the float compositor directly.
//!
//! # How it works
//!
//! Signed-area accumulation, the technique font rasterisers use. Each line
//! segment deposits its exact contribution to the pixels it crosses into an
//! accumulation buffer; a running sum along each scanline then turns those
//! contributions into coverage. The antialiasing is analytic — the exact area
//! of the pixel the shape covers — not sampled, so there is no supersampling
//! parameter to get wrong.
//!
//! # Region rendering
//!
//! The tiled renderer asks for sub-rectangles, and a running sum that started
//! at the left edge of a tile would lose the winding accumulated to its left.
//! So the accumulator spans the *path*, with an origin that does not depend on
//! the rectangle asked for, and only the requested columns are kept. Because
//! the sum then adds the same terms in the same order every time, a tile is
//! bit-identical to the same region of a full frame — asserted by test, not
//! assumed.
use super::path::VectorPath;

/// A rectangle of f32 coverage in [0, 1].
pub struct CoverageMask {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub coverage: Vec<f32>,
}

impl CoverageMask {
    pub fn at(&self, x: i32, y: i32) -> f32 {
        if x < self.x || y < self.y {
            return 0.0;
        }
        let (dx, dy) = ((x - self.x) as u32, (y - self.y) as u32);
        if dx >= self.width || dy >= self.height {
            return 0.0;
        }
        self.coverage[(dy * self.width + dx) as usize]
    }
}

/// How the interior of a path is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FillRule {
    /// A point is inside when the signed crossing count is not zero. The usual
    /// choice, and what a self-overlapping star looks solid under.
    #[default]
    NonZero,
    /// A point is inside when the crossing count is odd, so overlaps punch
    /// holes.
    EvenOdd,
}

/// Rasterises flattened subpaths into coverage for `region`.
///
/// `region` is in device pixels. Subpaths are device-space polylines; a subpath
/// that is not closed is closed implicitly, because coverage is an area and an
/// open outline does not bound one.
pub fn rasterize(
    subpaths: &[Vec<(f32, f32)>],
    region_x: i32,
    region_y: i32,
    region_width: u32,
    region_height: u32,
    rule: FillRule,
) -> CoverageMask {
    let empty = || CoverageMask {
        x: region_x,
        y: region_y,
        width: region_width,
        height: region_height,
        coverage: vec![0.0; (region_width as usize) * (region_height as usize)],
    };
    if region_width == 0 || region_height == 0 {
        return empty();
    }

    // The accumulator must span every edge the path has, not merely the region.
    //
    // Starting it at the region's left edge would lose the winding accumulated
    // to the left; ending it at the region's right edge would mean clamping
    // edges that lie further right, and clamping an edge *moves* it. An
    // earlier version did exactly that, and a region whose boundary happened to
    // fall on an ellipse's edge disagreed with the full-frame render by 0.003 —
    // small, and still a seam.
    let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
    for subpath in subpaths {
        for (x, _) in subpath {
            if x.is_finite() {
                min_x = min_x.min(*x);
                max_x = max_x.max(*x);
            }
        }
    }
    if min_x == f32::MAX {
        return empty();
    }
    // Both ends depend only on the path, never on the region. That is what
    // makes a tile *bit-identical* to the same part of a full frame rather than
    // merely close: the running sum then adds exactly the same terms in exactly
    // the same order whatever rectangle was asked for. Deriving the left edge
    // from the region instead left a 6e-6 disagreement — invisible, and still
    // not the same number.
    let start_x = (min_x.floor() as i64 - 1).clamp(-(1 << 30), 1 << 30) as i32;
    // Two extra columns: a segment landing on the last pixel writes to the one
    // after it, and the running sum needs somewhere to put it.
    let end_x = (max_x.ceil() as i64 + 2).clamp(start_x as i64 + 1, 1 << 30);
    let accumulator_width = (end_x - start_x as i64 + 1).max(1) as usize;
    let Some(cells) = accumulator_width.checked_mul(region_height as usize) else {
        return empty();
    };
    // The accumulator is now sized purely by the path, so this cap is the only
    // thing between a pathological path and a pathological allocation. Sixty-
    // four million cells is far more than any canvas needs and small enough to
    // refuse safely.
    if cells > 64 * 1024 * 1024 {
        return empty();
    }
    let mut accumulator = vec![0.0f32; cells];

    for subpath in subpaths {
        if subpath.len() < 2 {
            continue;
        }
        for window in subpath.windows(2) {
            accumulate(
                &mut accumulator,
                accumulator_width,
                region_height as usize,
                start_x,
                region_y,
                window[0],
                window[1],
            );
        }
        // Implicit close.
        let (first, last) = (subpath[0], subpath[subpath.len() - 1]);
        if first != last {
            accumulate(
                &mut accumulator,
                accumulator_width,
                region_height as usize,
                start_x,
                region_y,
                last,
                first,
            );
        }
    }

    // Running sum along each row turns contributions into winding, and the fill
    // rule turns winding into coverage.
    //
    // Columns of the region that lie left of the path stay zero: there is no
    // winding out there to accumulate, and reaching for them would mean giving
    // the accumulator a region-dependent origin again.
    let mut coverage = vec![0.0f32; (region_width as usize) * (region_height as usize)];
    let offset = region_x as i64 - start_x as i64;
    for row in 0..region_height as usize {
        let base = row * accumulator_width;
        let mut sum = 0.0f32;
        for column in 0..accumulator_width {
            sum += accumulator[base + column];
            let target = column as i64 - offset;
            if target < 0 {
                continue;
            }
            let target = target as usize;
            if target >= region_width as usize {
                break;
            }
            coverage[row * region_width as usize + target] = match rule {
                FillRule::NonZero => sum.abs().min(1.0),
                FillRule::EvenOdd => {
                    // Fold the winding into [0,2) and take the tent, so 1 is
                    // solid and 0 and 2 are both empty.
                    let folded = sum.abs() % 2.0;
                    if folded > 1.0 {
                        2.0 - folded
                    } else {
                        folded
                    }
                }
            };
        }
    }

    CoverageMask {
        x: region_x,
        y: region_y,
        width: region_width,
        height: region_height,
        coverage,
    }
}

/// Deposits one segment's exact area contribution into the accumulator.
///
/// Adapted from the standard signed-area technique: for each scanline the
/// segment crosses, the area to the right of the segment within each pixel is
/// added, with a sign from the direction of travel.
#[allow(clippy::too_many_arguments)]
fn accumulate(
    accumulator: &mut [f32],
    width: usize,
    height: usize,
    start_x: i32,
    start_y: i32,
    p0: (f32, f32),
    p1: (f32, f32),
) {
    if !p0.0.is_finite() || !p0.1.is_finite() || !p1.0.is_finite() || !p1.1.is_finite() {
        return;
    }
    // Into accumulator-local coordinates.
    let p0 = (p0.0 - start_x as f32, p0.1 - start_y as f32);
    let p1 = (p1.0 - start_x as f32, p1.1 - start_y as f32);

    if (p0.1 - p1.1).abs() < 1e-9 {
        return; // Horizontal segments contribute no area.
    }
    let (direction, top, bottom) = if p0.1 < p1.1 {
        (1.0f32, p0, p1)
    } else {
        (-1.0f32, p1, p0)
    };
    let dxdy = (bottom.0 - top.0) / (bottom.1 - top.1);
    if !dxdy.is_finite() {
        return;
    }

    let first_row = top.1.floor().max(0.0) as usize;
    let last_row = (bottom.1.ceil().max(0.0) as usize).min(height);

    for row in first_row..last_row {
        let row_top = (row as f32).max(top.1);
        let row_bottom = ((row + 1) as f32).min(bottom.1);
        let dy = row_bottom - row_top;
        if dy <= 0.0 {
            continue;
        }
        // Evaluated from the line equation rather than carried forward from the
        // previous row. Stepping incrementally makes the result depend on which
        // row the loop started at, so a tile whose top edge sat lower than the
        // full frame's accumulated a different rounding and disagreed by 6e-6.
        let x = top.0 + (row_top - top.1) * dxdy;
        let x_next = top.0 + (row_bottom - top.1) * dxdy;
        let signed = dy * direction;
        let (left, right) = if x < x_next { (x, x_next) } else { (x_next, x) };
        // The accumulator is sized to span the whole path, so this clamp is a
        // guard against pathological input rather than part of the geometry.
        // Clamping an edge that is genuinely inside the path would move it.
        let limit = (width - 2) as f32;
        let left = left.clamp(0.0, limit);
        let right = right.clamp(0.0, limit);
        let base = row * width;

        let left_floor = left.floor();
        let left_index = left_floor as usize;
        let right_ceil = right.ceil();
        let right_index = right_ceil as usize;

        if right_index <= left_index + 1 {
            // The segment stays within one pixel column on this scanline.
            let mid = 0.5 * (left + right) - left_floor;
            accumulator[base + left_index] += signed * (1.0 - mid);
            accumulator[base + left_index + 1] += signed * mid;
        } else {
            let inverse = (right - left).recip();
            let left_fraction = left - left_floor;
            let first_area = 0.5 * inverse * (1.0 - left_fraction) * (1.0 - left_fraction);
            let right_fraction = right - right_ceil + 1.0;
            let last_area = 0.5 * inverse * right_fraction * right_fraction;
            accumulator[base + left_index] += signed * first_area;
            if right_index == left_index + 2 {
                accumulator[base + left_index + 1] += signed * (1.0 - first_area - last_area);
            } else {
                let second = inverse * (1.5 - left_fraction);
                accumulator[base + left_index + 1] += signed * (second - first_area);
                for column in left_index + 2..right_index - 1 {
                    accumulator[base + column] += signed * inverse;
                }
                let before_last = second + (right_index - left_index - 3) as f32 * inverse;
                accumulator[base + right_index - 1] += signed * (1.0 - before_last - last_area);
            }
            accumulator[base + right_index] += signed * last_area;
        }
    }
}

/// Rasterises a path directly, flattening it first.
pub fn rasterize_path(
    path: &VectorPath,
    tolerance: f32,
    region_x: i32,
    region_y: i32,
    region_width: u32,
    region_height: u32,
    rule: FillRule,
) -> CoverageMask {
    let subpaths = path.flatten(tolerance);
    rasterize(
        &subpaths,
        region_x,
        region_y,
        region_width,
        region_height,
        rule,
    )
}

#[cfg(test)]
mod tests {
    use super::super::path::ShapeGeometry;
    use super::*;

    fn total_coverage(mask: &CoverageMask) -> f64 {
        mask.coverage.iter().map(|v| f64::from(*v)).sum()
    }

    /// Coverage is an area, so a rasterised shape's total coverage has to equal
    /// its geometric area. This is the strongest single check on the
    /// rasteriser: it catches sign errors, double counting and missing edges.
    #[test]
    fn total_coverage_equals_the_geometric_area() {
        let square = ShapeGeometry::Rectangle {
            x: 10.0,
            y: 10.0,
            width: 40.0,
            height: 30.0,
            corner_radius: 0.0,
        };
        let mask = rasterize_path(&square.to_path(), 0.05, 0, 0, 64, 64, FillRule::NonZero);
        assert!(
            (total_coverage(&mask) - 1200.0).abs() < 0.5,
            "square covered {}",
            total_coverage(&mask)
        );

        let circle = ShapeGeometry::Ellipse {
            cx: 32.0,
            cy: 32.0,
            rx: 20.0,
            ry: 20.0,
        };
        let mask = rasterize_path(&circle.to_path(), 0.02, 0, 0, 64, 64, FillRule::NonZero);
        let expected = std::f64::consts::PI * 400.0;
        assert!(
            (total_coverage(&mask) - expected).abs() < 2.0,
            "circle covered {} against {expected}",
            total_coverage(&mask)
        );
    }

    /// A half-covered pixel must read as half, or the antialiasing is not
    /// analytic and edges will band.
    #[test]
    fn a_half_covered_pixel_reads_as_half() {
        // A rectangle whose right edge falls exactly down the middle of x=5.
        let path = ShapeGeometry::Rectangle {
            x: 0.0,
            y: 0.0,
            width: 5.5,
            height: 8.0,
            corner_radius: 0.0,
        }
        .to_path();
        let mask = rasterize_path(&path, 0.05, 0, 0, 10, 10, FillRule::NonZero);
        for y in 1..7 {
            assert!(
                (mask.at(4, y) - 1.0).abs() < 1e-4,
                "pixel 4 of row {y} was {}",
                mask.at(4, y)
            );
            assert!(
                (mask.at(5, y) - 0.5).abs() < 1e-3,
                "the half-covered pixel of row {y} was {}",
                mask.at(5, y)
            );
            assert!(mask.at(6, y) < 1e-4, "pixel 6 of row {y} leaked");
        }
    }

    /// The property the tiled renderer depends on: a region render must equal
    /// the same region of a full-frame render, exactly.
    #[test]
    fn a_region_is_identical_to_that_part_of_the_whole() {
        let shapes = [
            ShapeGeometry::Ellipse {
                cx: 60.0,
                cy: 45.0,
                rx: 40.0,
                ry: 28.0,
            },
            ShapeGeometry::Star {
                cx: 64.0,
                cy: 64.0,
                outer_radius: 50.0,
                inner_radius: 18.0,
                points: 7,
                rotation_degrees: 12.0,
            },
            ShapeGeometry::Rectangle {
                x: 12.5,
                y: 9.25,
                width: 77.0,
                height: 61.5,
                corner_radius: 14.0,
            },
        ];
        for shape in shapes {
            let path = shape.to_path();
            let whole = rasterize_path(&path, 0.02, 0, 0, 128, 128, FillRule::NonZero);
            for (rx, ry, rw, rh) in [
                (0, 0, 32, 32),
                (32, 32, 32, 32),
                (96, 0, 32, 128),
                (61, 47, 7, 5),
            ] {
                let region = rasterize_path(&path, 0.02, rx, ry, rw, rh, FillRule::NonZero);
                for y in 0..rh as i32 {
                    for x in 0..rw as i32 {
                        let expected = whole.at(rx + x, ry + y);
                        let actual = region.at(rx + x, ry + y);
                        // Exactly equal, not merely close: the accumulator's
                        // origin does not depend on the region, so the same
                        // sum is computed either way.
                        assert!(
                            expected == actual,
                            "{shape:?} at ({},{}) whole {expected} region {actual}",
                            rx + x,
                            ry + y
                        );
                    }
                }
            }
        }
    }

    /// Geometry outside the requested region must not appear inside it, and
    /// must not index out of the accumulator.
    #[test]
    fn geometry_outside_the_region_is_clipped_safely() {
        let path = ShapeGeometry::Rectangle {
            x: -5000.0,
            y: -5000.0,
            width: 20.0,
            height: 20.0,
            corner_radius: 0.0,
        }
        .to_path();
        let mask = rasterize_path(&path, 0.05, 0, 0, 32, 32, FillRule::NonZero);
        assert_eq!(total_coverage(&mask), 0.0);

        // And a shape enormously larger than the region fills it completely.
        let huge = ShapeGeometry::Rectangle {
            x: -5000.0,
            y: -5000.0,
            width: 10_000.0,
            height: 10_000.0,
            corner_radius: 0.0,
        }
        .to_path();
        let mask = rasterize_path(&huge, 0.05, 100, 100, 16, 16, FillRule::NonZero);
        assert!(
            (total_coverage(&mask) - 256.0).abs() < 0.01,
            "a covering shape gave {}",
            total_coverage(&mask)
        );
    }

    /// The two fill rules must actually differ where a path overlaps itself.
    #[test]
    fn the_fill_rules_differ_on_a_self_overlapping_path() {
        // A five-pointed star drawn as a single crossing polyline overlaps in
        // the middle, which is where the rules disagree.
        use super::super::path::{PathCommand, VectorPath};
        let mut commands = Vec::new();
        for step in 0..5 {
            let angle =
                std::f32::consts::TAU * (step as f32 * 2.0) / 5.0 - std::f32::consts::FRAC_PI_2;
            let (x, y) = (64.0 + angle.cos() * 50.0, 64.0 + angle.sin() * 50.0);
            commands.push(if step == 0 {
                PathCommand::MoveTo { x, y }
            } else {
                PathCommand::LineTo { x, y }
            });
        }
        commands.push(PathCommand::Close);
        let path = VectorPath::new(commands);

        let nonzero = rasterize_path(&path, 0.02, 0, 0, 128, 128, FillRule::NonZero);
        let evenodd = rasterize_path(&path, 0.02, 0, 0, 128, 128, FillRule::EvenOdd);
        // Even-odd punches out the pentagon in the middle, so it covers less.
        assert!(
            total_coverage(&nonzero) > total_coverage(&evenodd) * 1.2,
            "nonzero {} against even-odd {}",
            total_coverage(&nonzero),
            total_coverage(&evenodd)
        );
        // And the centre is the point they disagree about.
        assert!(nonzero.at(64, 64) > 0.9, "nonzero centre was empty");
        assert!(evenodd.at(64, 64) < 0.1, "even-odd centre was filled");
    }

    /// Nothing here may panic on hostile geometry.
    #[test]
    fn hostile_geometry_does_not_panic() {
        use super::super::path::{PathCommand, VectorPath};
        let cases = vec![
            VectorPath::new(vec![]),
            VectorPath::new(vec![PathCommand::MoveTo { x: 0.0, y: 0.0 }]),
            VectorPath::new(vec![
                PathCommand::MoveTo {
                    x: f32::NAN,
                    y: 0.0,
                },
                PathCommand::LineTo { x: 10.0, y: 10.0 },
                PathCommand::Close,
            ]),
            VectorPath::new(vec![
                PathCommand::MoveTo {
                    x: f32::INFINITY,
                    y: f32::NEG_INFINITY,
                },
                PathCommand::LineTo { x: 5.0, y: 5.0 },
                PathCommand::Close,
            ]),
            VectorPath::new(vec![
                PathCommand::MoveTo { x: 1e9, y: 1e9 },
                PathCommand::LineTo { x: -1e9, y: -1e9 },
                PathCommand::Close,
            ]),
        ];
        for path in cases {
            let mask = rasterize_path(&path, 0.05, 0, 0, 32, 32, FillRule::NonZero);
            assert_eq!(mask.coverage.len(), 32 * 32);
            assert!(
                mask.coverage
                    .iter()
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
                "produced coverage outside [0,1] for {path:?}"
            );
        }
    }

    /// A zero-sized region is a legitimate request and must not allocate or
    /// index anything.
    #[test]
    fn an_empty_region_is_empty() {
        let path = ShapeGeometry::Ellipse {
            cx: 10.0,
            cy: 10.0,
            rx: 5.0,
            ry: 5.0,
        }
        .to_path();
        let mask = rasterize_path(&path, 0.05, 0, 0, 0, 0, FillRule::NonZero);
        assert!(mask.coverage.is_empty());
    }
}
