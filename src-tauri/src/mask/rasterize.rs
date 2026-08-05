use super::bitmap::MaskBitmap;
use super::geometry::{simplify_path, Point, SelectionShape};
use crate::error::AppError;

const SAMPLES: [(f32, f32); 16] = [
    (0.125, 0.125),
    (0.375, 0.125),
    (0.625, 0.125),
    (0.875, 0.125),
    (0.125, 0.375),
    (0.375, 0.375),
    (0.625, 0.375),
    (0.875, 0.375),
    (0.125, 0.625),
    (0.375, 0.625),
    (0.625, 0.625),
    (0.875, 0.625),
    (0.125, 0.875),
    (0.375, 0.875),
    (0.625, 0.875),
    (0.875, 0.875),
];

pub fn rasterize(width: u32, height: u32, shape: &SelectionShape) -> Result<MaskBitmap, AppError> {
    shape.validate()?;
    match shape {
        SelectionShape::Rectangle { start, end } => rectangle(width, height, *start, *end),
        SelectionShape::Ellipse { start, end } => ellipse(width, height, *start, *end),
        SelectionShape::Polygon { points } => polygon(width, height, points),
        SelectionShape::Freehand { points } => polygon(width, height, &simplify_path(points, 0.35)),
        SelectionShape::Brush {
            points,
            diameter,
            hardness,
            opacity,
        } => brush(width, height, points, *diameter, *hardness, *opacity),
    }
}

fn rectangle(width: u32, height: u32, start: Point, end: Point) -> Result<MaskBitmap, AppError> {
    let left = start.x.min(end.x).clamp(0.0, width as f32);
    let right = start.x.max(end.x).clamp(0.0, width as f32);
    let top = start.y.min(end.y).clamp(0.0, height as f32);
    let bottom = start.y.max(end.y).clamp(0.0, height as f32);
    let mut mask = MaskBitmap::empty(width, height)?;
    for y in top.floor().max(0.0) as u32..bottom.ceil().min(height as f32) as u32 {
        let overlap_y = (bottom.min(y as f32 + 1.0) - top.max(y as f32)).clamp(0.0, 1.0);
        for x in left.floor().max(0.0) as u32..right.ceil().min(width as f32) as u32 {
            let overlap_x = (right.min(x as f32 + 1.0) - left.max(x as f32)).clamp(0.0, 1.0);
            mask.set(x, y, (overlap_x * overlap_y * 255.0).round() as u8);
        }
    }
    Ok(mask)
}

fn ellipse(width: u32, height: u32, start: Point, end: Point) -> Result<MaskBitmap, AppError> {
    let left = start.x.min(end.x).clamp(0.0, width as f32);
    let right = start.x.max(end.x).clamp(0.0, width as f32);
    let top = start.y.min(end.y).clamp(0.0, height as f32);
    let bottom = start.y.max(end.y).clamp(0.0, height as f32);
    let radius_x = (right - left) / 2.0;
    let radius_y = (bottom - top) / 2.0;
    let center_x = left + radius_x;
    let center_y = top + radius_y;
    sample_shape(width, height, [left, top, right, bottom], |x, y| {
        let dx = (x - center_x) / radius_x;
        let dy = (y - center_y) / radius_y;
        dx * dx + dy * dy <= 1.0
    })
}

fn polygon(width: u32, height: u32, points: &[Point]) -> Result<MaskBitmap, AppError> {
    let bounds = points.iter().fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |[left, top, right, bottom], point| {
            [
                left.min(point.x),
                top.min(point.y),
                right.max(point.x),
                bottom.max(point.y),
            ]
        },
    );
    let left = bounds[0].floor().clamp(0.0, width as f32) as u32;
    let top = bounds[1].floor().clamp(0.0, height as f32) as u32;
    let right = bounds[2].ceil().clamp(0.0, width as f32) as u32;
    let bottom = bounds[3].ceil().clamp(0.0, height as f32) as u32;
    let mut mask = MaskBitmap::empty(width, height)?;
    let mut counts = vec![0_u8; (right - left) as usize];
    for y in top..bottom {
        counts.fill(0);
        for sample_y_index in 0..4 {
            let sample_y = y as f32 + (sample_y_index as f32 + 0.5) / 4.0;
            let mut intersections = Vec::with_capacity(points.len());
            let mut previous = points[points.len() - 1];
            for &current in points {
                if (current.y > sample_y) != (previous.y > sample_y) {
                    intersections.push(
                        (previous.x - current.x) * (sample_y - current.y)
                            / (previous.y - current.y)
                            + current.x,
                    );
                }
                previous = current;
            }
            intersections.sort_by(f32::total_cmp);
            for pair in intersections.chunks_exact(2) {
                let interval_left = pair[0].clamp(left as f32, right as f32);
                let interval_right = pair[1].clamp(left as f32, right as f32);
                let first_x = interval_left.floor().max(left as f32) as u32;
                let last_x = interval_right.ceil().min(right as f32) as u32;
                for x in first_x..last_x {
                    let hits = (0..4)
                        .filter(|sample_x_index| {
                            let sample_x = x as f32 + (*sample_x_index as f32 + 0.5) / 4.0;
                            sample_x >= interval_left && sample_x < interval_right
                        })
                        .count() as u8;
                    counts[(x - left) as usize] += hits;
                }
            }
        }
        for (offset, count) in counts.iter().enumerate() {
            mask.set(
                left + offset as u32,
                y,
                ((u16::from(*count) * 255 + 8) / 16) as u8,
            );
        }
    }
    Ok(mask)
}

fn sample_shape<F>(
    width: u32,
    height: u32,
    bounds: [f32; 4],
    inside: F,
) -> Result<MaskBitmap, AppError>
where
    F: Fn(f32, f32) -> bool,
{
    let mut mask = MaskBitmap::empty(width, height)?;
    let left = bounds[0].floor().clamp(0.0, width as f32) as u32;
    let top = bounds[1].floor().clamp(0.0, height as f32) as u32;
    let right = bounds[2].ceil().clamp(0.0, width as f32) as u32;
    let bottom = bounds[3].ceil().clamp(0.0, height as f32) as u32;
    for y in top..bottom {
        for x in left..right {
            let hits = SAMPLES
                .iter()
                .filter(|(offset_x, offset_y)| inside(x as f32 + offset_x, y as f32 + offset_y))
                .count();
            mask.set(
                x,
                y,
                ((hits * 255 + SAMPLES.len() / 2) / SAMPLES.len()) as u8,
            );
        }
    }
    Ok(mask)
}

fn brush(
    width: u32,
    height: u32,
    points: &[Point],
    diameter: f32,
    hardness: f32,
    opacity: f32,
) -> Result<MaskBitmap, AppError> {
    let mut mask = MaskBitmap::empty(width, height)?;
    let radius = diameter / 2.0;
    let spacing = (diameter * 0.18).max(0.5);
    if points.len() == 1 {
        paint_dab(&mut mask, points[0], radius, hardness, opacity);
        return Ok(mask);
    }
    for pair in points.windows(2) {
        let dx = pair[1].x - pair[0].x;
        let dy = pair[1].y - pair[0].y;
        let distance = dx.hypot(dy);
        let steps = (distance / spacing).ceil().max(1.0) as usize;
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            paint_dab(
                &mut mask,
                Point {
                    x: pair[0].x + dx * t,
                    y: pair[0].y + dy * t,
                },
                radius,
                hardness,
                opacity,
            );
        }
    }
    Ok(mask)
}

fn paint_dab(mask: &mut MaskBitmap, center: Point, radius: f32, hardness: f32, opacity: f32) {
    let left = (center.x - radius - 1.0).floor().max(0.0) as u32;
    let right = (center.x + radius + 1.0).ceil().min(mask.width() as f32) as u32;
    let top = (center.y - radius - 1.0).floor().max(0.0) as u32;
    let bottom = (center.y + radius + 1.0).ceil().min(mask.height() as f32) as u32;
    let hard_radius = radius * hardness;
    for y in top..bottom {
        for x in left..right {
            let distance = (x as f32 + 0.5 - center.x).hypot(y as f32 + 0.5 - center.y);
            if distance > radius {
                continue;
            }
            let falloff = if distance <= hard_radius || hard_radius >= radius {
                1.0
            } else {
                1.0 - (distance - hard_radius) / (radius - hard_radius)
            };
            let incoming = (255.0 * opacity * falloff).round() as u8;
            mask.set(x, y, mask.get(x, y).max(incoming));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_clamps_to_bounds_and_prevents_zero_area() {
        let mask = rasterize(
            3,
            3,
            &SelectionShape::Rectangle {
                start: Point { x: -4.0, y: -4.0 },
                end: Point { x: 2.0, y: 2.0 },
            },
        )
        .unwrap();
        assert_eq!(mask.get(0, 0), 255);
        assert_eq!(mask.get(2, 2), 0);
        assert!(rasterize(
            3,
            3,
            &SelectionShape::Rectangle {
                start: Point { x: 1.0, y: 1.0 },
                end: Point { x: 1.0, y: 2.0 },
            }
        )
        .is_err());
    }

    #[test]
    fn ellipse_has_partial_antialiased_edges() {
        let mask = rasterize(
            5,
            5,
            &SelectionShape::Ellipse {
                start: Point { x: 0.25, y: 0.25 },
                end: Point { x: 4.75, y: 4.75 },
            },
        )
        .unwrap();
        assert_eq!(mask.get(2, 2), 255);
        assert!(mask
            .coverage()
            .iter()
            .any(|value| (1..=254).contains(value)));
    }

    #[test]
    fn self_intersecting_polygon_is_deterministic() {
        let shape = SelectionShape::Polygon {
            points: vec![
                Point { x: 0.0, y: 0.0 },
                Point { x: 4.0, y: 4.0 },
                Point { x: 0.0, y: 4.0 },
                Point { x: 4.0, y: 0.0 },
            ],
        };
        assert_eq!(
            rasterize(4, 4, &shape).unwrap(),
            rasterize(4, 4, &shape).unwrap()
        );
    }

    #[test]
    fn rapid_brush_segment_has_no_centerline_gaps() {
        let mask = rasterize(
            100,
            20,
            &SelectionShape::Brush {
                points: vec![Point { x: 5.0, y: 10.0 }, Point { x: 95.0, y: 10.0 }],
                diameter: 8.0,
                hardness: 1.0,
                opacity: 1.0,
            },
        )
        .unwrap();
        assert!((5..95).all(|x| mask.get(x, 10) == 255));
    }
}
