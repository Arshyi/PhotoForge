use super::bitmap::MaskBitmap;
use super::geometry::{simplify_path, Point, ResolvedBrushSample, SelectionShape};
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

// A brush stroke may contain up to 20,000 valid points or resolved samples.
// Per-segment limits alone still permit billions of interpolated dabs, so cap
// both interpolation overhead and conservative dab bounding-box pixel visits.
const MAX_BRUSH_DABS: u64 = 1_000_000;
const MAX_BRUSH_PIXEL_WORK: u64 = 128_000_000;

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
        SelectionShape::ResolvedBrush { samples, hardness } => {
            resolved_brush(width, height, samples, *hardness)
        }
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
    let steps_by_segment = brush_plan(width, height, points, diameter)?;
    let mut mask = MaskBitmap::empty(width, height)?;
    let radius = diameter / 2.0;
    if points.len() == 1 {
        paint_dab(&mut mask, points[0], radius, hardness, opacity);
        return Ok(mask);
    }
    for (pair, &steps) in points.windows(2).zip(&steps_by_segment) {
        let dx = pair[1].x - pair[0].x;
        let dy = pair[1].y - pair[0].y;
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

fn brush_plan(
    width: u32,
    height: u32,
    points: &[Point],
    diameter: f32,
) -> Result<Vec<usize>, AppError> {
    let mut aggregate_dabs = 0_u64;
    let mut aggregate_pixel_work = 0_u64;
    if points.len() == 1 {
        add_brush_work(
            width,
            height,
            1,
            diameter,
            &mut aggregate_dabs,
            &mut aggregate_pixel_work,
        )?;
        return Ok(Vec::new());
    }

    let spacing = (diameter * 0.18).max(0.5);
    let maximum_steps = 2.0 * (f64::from(width) + f64::from(height) + 4_096.0);
    let mut steps_by_segment = Vec::with_capacity(points.len().saturating_sub(1));
    for pair in points.windows(2) {
        let distance = (pair[1].x - pair[0].x).hypot(pair[1].y - pair[0].y);
        let requested_steps = (distance / spacing).ceil().max(1.0);
        if !requested_steps.is_finite() || f64::from(requested_steps) > maximum_steps {
            return Err(AppError::InvalidMask(
                "brush segment exceeds the bounded rasterization span".into(),
            ));
        }
        let steps_u64 = requested_steps as u64;
        add_brush_work(
            width,
            height,
            steps_u64.saturating_add(1),
            diameter,
            &mut aggregate_dabs,
            &mut aggregate_pixel_work,
        )?;
        let steps = usize::try_from(steps_u64)
            .map_err(|_| AppError::InvalidMask("brush step count exceeds this platform".into()))?;
        steps_by_segment.push(steps);
    }
    Ok(steps_by_segment)
}

fn resolved_brush(
    width: u32,
    height: u32,
    samples: &[ResolvedBrushSample],
    hardness: f32,
) -> Result<MaskBitmap, AppError> {
    let steps_by_segment = resolved_brush_plan(width, height, samples)?;
    let mut mask = MaskBitmap::empty(width, height)?;
    if samples.len() == 1 {
        paint_resolved_dab(&mut mask, samples[0], hardness);
        return Ok(mask);
    }

    for (pair, &steps) in samples.windows(2).zip(&steps_by_segment) {
        let start = pair[0];
        let end = pair[1];
        let dx = end.x - start.x;
        let dy = end.y - start.y;
        for step in 0..=steps {
            let t = step as f32 / steps as f32;
            paint_resolved_dab(
                &mut mask,
                ResolvedBrushSample {
                    x: start.x + dx * t,
                    y: start.y + dy * t,
                    diameter: start.diameter + (end.diameter - start.diameter) * t,
                    opacity: start.opacity + (end.opacity - start.opacity) * t,
                },
                hardness,
            );
        }
    }
    Ok(mask)
}

fn resolved_brush_plan(
    width: u32,
    height: u32,
    samples: &[ResolvedBrushSample],
) -> Result<Vec<usize>, AppError> {
    let mut aggregate_dabs = 0_u64;
    let mut aggregate_pixel_work = 0_u64;
    if samples.len() == 1 {
        add_brush_work(
            width,
            height,
            1,
            samples[0].diameter,
            &mut aggregate_dabs,
            &mut aggregate_pixel_work,
        )?;
        return Ok(Vec::new());
    }

    let mut steps_by_segment = Vec::with_capacity(samples.len().saturating_sub(1));
    for pair in samples.windows(2) {
        let start = pair[0];
        let end = pair[1];
        let distance = (end.x - start.x).hypot(end.y - start.y);
        if !distance.is_finite() {
            return Err(AppError::InvalidMask(
                "resolved brush segment length is invalid".into(),
            ));
        }
        let spacing = (start.diameter.min(end.diameter) * 0.18).max(0.5);
        let requested_steps = (f64::from(distance) / f64::from(spacing)).ceil().max(1.0);
        let maximum_steps = 2.0 * (f64::from(width) + f64::from(height) + 4_096.0);
        if !requested_steps.is_finite() || requested_steps > maximum_steps {
            return Err(AppError::InvalidMask(
                "resolved brush segment exceeds the bounded rasterization span".into(),
            ));
        }
        let steps_u64 = requested_steps as u64;
        add_brush_work(
            width,
            height,
            steps_u64.saturating_add(1),
            start.diameter.max(end.diameter),
            &mut aggregate_dabs,
            &mut aggregate_pixel_work,
        )?;
        let steps = usize::try_from(steps_u64).map_err(|_| {
            AppError::InvalidMask("resolved brush step count exceeds this platform".into())
        })?;
        steps_by_segment.push(steps);
    }
    Ok(steps_by_segment)
}

fn add_brush_work(
    width: u32,
    height: u32,
    dabs: u64,
    maximum_diameter: f32,
    aggregate_dabs: &mut u64,
    aggregate_pixel_work: &mut u64,
) -> Result<(), AppError> {
    *aggregate_dabs = aggregate_dabs
        .checked_add(dabs)
        .ok_or_else(brush_work_error)?;
    if *aggregate_dabs > MAX_BRUSH_DABS {
        return Err(brush_work_error());
    }

    // paint_dab visits the clipped integer bounding box from radius + 1px
    // padding. ceil(diameter) + 3 is a conservative maximum side length.
    let maximum_side = maximum_diameter.ceil() as u64 + 3;
    let bounding_width = u64::from(width).min(maximum_side);
    let bounding_height = u64::from(height).min(maximum_side);
    let pixels_per_dab = bounding_width
        .checked_mul(bounding_height)
        .ok_or_else(brush_work_error)?;
    let added_work = pixels_per_dab
        .checked_mul(dabs)
        .ok_or_else(brush_work_error)?;
    *aggregate_pixel_work = aggregate_pixel_work
        .checked_add(added_work)
        .ok_or_else(brush_work_error)?;
    if *aggregate_pixel_work > MAX_BRUSH_PIXEL_WORK {
        return Err(brush_work_error());
    }
    Ok(())
}

fn brush_work_error() -> AppError {
    AppError::InvalidMask(format!(
        "brush exceeds aggregate rasterization limits ({MAX_BRUSH_DABS} dabs or {MAX_BRUSH_PIXEL_WORK} conservative pixel visits)"
    ))
}

fn paint_resolved_dab(mask: &mut MaskBitmap, sample: ResolvedBrushSample, hardness: f32) {
    paint_dab(
        mask,
        sample.point(),
        sample.diameter / 2.0,
        hardness,
        sample.opacity,
    );
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

    #[test]
    fn resolved_uniform_samples_match_legacy_brush_exactly() {
        let points = vec![
            Point { x: 5.5, y: 8.5 },
            Point { x: 34.5, y: 12.5 },
            Point { x: 58.5, y: 8.5 },
        ];
        let legacy = rasterize(
            64,
            24,
            &SelectionShape::Brush {
                points: points.clone(),
                diameter: 10.0,
                hardness: 0.65,
                opacity: 0.7,
            },
        )
        .unwrap();
        let resolved = rasterize(
            64,
            24,
            &SelectionShape::ResolvedBrush {
                samples: points
                    .into_iter()
                    .map(|point| ResolvedBrushSample {
                        x: point.x,
                        y: point.y,
                        diameter: 10.0,
                        opacity: 0.7,
                    })
                    .collect(),
                hardness: 0.65,
            },
        )
        .unwrap();
        assert_eq!(resolved, legacy);
    }

    #[test]
    fn resolved_brush_interpolates_size_and_opacity_conservatively() {
        let mask = rasterize(
            48,
            32,
            &SelectionShape::ResolvedBrush {
                samples: vec![
                    ResolvedBrushSample {
                        x: 10.5,
                        y: 15.5,
                        diameter: 4.0,
                        opacity: 0.25,
                    },
                    ResolvedBrushSample {
                        x: 30.5,
                        y: 15.5,
                        diameter: 12.0,
                        opacity: 1.0,
                    },
                ],
                hardness: 1.0,
            },
        )
        .unwrap();

        assert!((64..=254).contains(&mask.get(10, 15)));
        assert!((65..=254).contains(&mask.get(20, 15)));
        assert_eq!(mask.get(30, 15), 255);
        assert_eq!(mask.get(10, 20), 0);
        assert_eq!(mask.get(30, 20), 255);

        let conservative = rasterize(
            48,
            32,
            &SelectionShape::ResolvedBrush {
                samples: vec![
                    ResolvedBrushSample {
                        x: 10.5,
                        y: 15.5,
                        diameter: 4.0,
                        opacity: 0.4,
                    },
                    ResolvedBrushSample {
                        x: 30.5,
                        y: 15.5,
                        diameter: 12.0,
                        opacity: 0.4,
                    },
                ],
                hardness: 1.0,
            },
        )
        .unwrap();
        assert_eq!(conservative.coverage().iter().copied().max(), Some(102));
    }

    #[test]
    fn resolved_single_point_uses_effective_values() {
        let mask = rasterize(
            24,
            24,
            &SelectionShape::ResolvedBrush {
                samples: vec![ResolvedBrushSample {
                    x: 10.5,
                    y: 10.5,
                    diameter: 6.0,
                    opacity: 0.4,
                }],
                hardness: 1.0,
            },
        )
        .unwrap();
        assert_eq!(mask.get(10, 10), 102);
        assert_eq!(mask.coverage().iter().copied().max(), Some(102));
        assert_eq!(mask.get(0, 0), 0);
    }

    #[test]
    fn resolved_variable_segment_has_no_centerline_gaps_and_replays_exactly() {
        let shape = SelectionShape::ResolvedBrush {
            samples: vec![
                ResolvedBrushSample {
                    x: 5.5,
                    y: 10.5,
                    diameter: 2.0,
                    opacity: 1.0,
                },
                ResolvedBrushSample {
                    x: 95.5,
                    y: 10.5,
                    diameter: 8.0,
                    opacity: 1.0,
                },
            ],
            hardness: 1.0,
        };
        let first = rasterize(104, 24, &shape).unwrap();
        let replayed = rasterize(104, 24, &shape).unwrap();
        assert_eq!(first, replayed);
        assert!((5..=95).all(|x| first.get(x, 10) == 255));
    }

    #[test]
    fn resolved_brush_rejects_aggregate_large_dab_pixel_work_before_painting() {
        let samples: Vec<_> = (0..128)
            .map(|index| ResolvedBrushSample {
                x: if index % 2 == 0 { 0.0 } else { 4_095.0 },
                y: 2_048.0,
                diameter: 2_048.0,
                opacity: 1.0,
            })
            .collect();
        let shape = SelectionShape::ResolvedBrush {
            samples: samples.clone(),
            hardness: 0.5,
        };
        shape.validate().unwrap();
        assert!(matches!(
            resolved_brush_plan(4_096, 4_096, &samples),
            Err(AppError::InvalidMask(message)) if message.contains("aggregate rasterization limits")
        ));
        assert!(matches!(
            rasterize(4_096, 4_096, &shape),
            Err(AppError::InvalidMask(message)) if message.contains("aggregate rasterization limits")
        ));
    }

    #[test]
    fn legacy_brush_rejects_finite_hostile_spans_before_step_conversion() {
        let points = vec![Point { x: -1.0e30, y: 0.0 }, Point { x: 1.0e30, y: 0.0 }];
        let shape = SelectionShape::Brush {
            points: points.clone(),
            diameter: 1.0,
            hardness: 1.0,
            opacity: 1.0,
        };
        shape.validate().unwrap();
        assert!(matches!(
            brush_plan(64, 64, &points, 1.0),
            Err(AppError::InvalidMask(message)) if message.contains("bounded rasterization span")
        ));
        assert!(matches!(
            rasterize(64, 64, &shape),
            Err(AppError::InvalidMask(message)) if message.contains("bounded rasterization span")
        ));
    }

    #[test]
    fn resolved_brush_rejects_aggregate_dab_count_for_alternating_far_points() {
        let samples: Vec<_> = (0..128)
            .map(|index| ResolvedBrushSample {
                x: if index % 2 == 0 { 0.0 } else { 4_095.0 },
                y: 0.5,
                diameter: 1.0,
                opacity: 0.5,
            })
            .collect();
        assert!(matches!(
            resolved_brush_plan(4_096, 1, &samples),
            Err(AppError::InvalidMask(message)) if message.contains("aggregate rasterization limits")
        ));
    }

    #[test]
    fn resolved_brush_aggregate_caps_preserve_normal_deterministic_strokes() {
        let samples: Vec<_> = (0..100)
            .map(|index| ResolvedBrushSample {
                x: 4.5 + index as f32 * 4.0,
                y: 32.5 + (index % 3) as f32,
                diameter: 24.0,
                opacity: 0.75,
            })
            .collect();
        let shape = SelectionShape::ResolvedBrush {
            samples: samples.clone(),
            hardness: 0.7,
        };
        assert_eq!(resolved_brush_plan(420, 72, &samples).unwrap().len(), 99);
        let first = rasterize(420, 72, &shape).unwrap();
        let replayed = rasterize(420, 72, &shape).unwrap();
        assert_eq!(first, replayed);
        assert!(first.coverage().contains(&191));
    }
}
