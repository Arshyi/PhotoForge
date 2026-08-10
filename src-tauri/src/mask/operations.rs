use super::bitmap::MaskBitmap;
use super::feather::feather_with_progress;
use super::progress::MaskWorkContext;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CompositionMode {
    #[default]
    Replace,
    Add,
    Subtract,
    Intersect,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MaskOperation {
    SelectAll,
    Deselect,
    Invert,
    Feather {
        radius: u32,
    },
    Expand {
        radius: u32,
    },
    Contract {
        radius: u32,
    },
    Smooth {
        radius: u32,
    },
    FillHoles,
    RemoveSmallIslands {
        minimum_pixels: u32,
    },
    Border {
        width: u32,
    },
    Refine {
        smooth: u32,
        feather: u32,
        contrast: f32,
        shift_edge: i32,
    },
}

pub(crate) fn work_units(operation: &MaskOperation, pixels: u64) -> u64 {
    let feather_passes = |radius: u32| if radius == 0 { 1_u64 } else { 2 };
    let morphology_passes = |radius: u32| if radius == 0 { 1_u64 } else { 2 };
    let passes = match operation {
        MaskOperation::SelectAll | MaskOperation::Deselect | MaskOperation::Invert => 1,
        MaskOperation::Feather { radius } => feather_passes((*radius).min(256)),
        MaskOperation::Expand { radius } | MaskOperation::Contract { radius } => {
            morphology_passes((*radius).min(256))
        }
        MaskOperation::Smooth { radius } => feather_passes((*radius).min(128)) + 1,
        MaskOperation::FillHoles | MaskOperation::RemoveSmallIslands { .. } => 1,
        MaskOperation::Border { width } => 2 * morphology_passes((*width).min(256)) + 1,
        MaskOperation::Refine {
            smooth,
            feather,
            shift_edge,
            ..
        } => {
            let smooth_passes = if *smooth == 0 {
                0
            } else {
                feather_passes((*smooth).min(128)) + 1
            };
            let shift_passes = if *shift_edge == 0 { 0 } else { 2 };
            let feather_passes = if *feather == 0 { 0 } else { 2 };
            smooth_passes + shift_passes + feather_passes + 1
        }
    };
    pixels.saturating_mul(passes)
}

pub fn compose(
    base: Option<&MaskBitmap>,
    incoming: &MaskBitmap,
    mode: CompositionMode,
) -> Result<MaskBitmap, AppError> {
    compose_with_progress(
        base,
        incoming,
        mode,
        MaskWorkContext::cancellation_only(None),
    )
}

pub(crate) fn compose_with_progress(
    base: Option<&MaskBitmap>,
    incoming: &MaskBitmap,
    mode: CompositionMode,
    context: MaskWorkContext<'_>,
) -> Result<MaskBitmap, AppError> {
    let pixels = incoming.coverage().len() as u64;
    let Some(base) = base else {
        context.check_cancelled()?;
        let result = match mode {
            CompositionMode::Subtract | CompositionMode::Intersect => {
                MaskBitmap::empty(incoming.width(), incoming.height())?
            }
            CompositionMode::Replace | CompositionMode::Add => incoming.clone(),
        };
        context.report("compose_pixels", pixels, pixels)?;
        return Ok(result);
    };
    if base.width() != incoming.width() || base.height() != incoming.height() {
        return Err(AppError::MaskDimensionMismatch {
            mask_width: incoming.width(),
            mask_height: incoming.height(),
            image_width: base.width(),
            image_height: base.height(),
        });
    }
    let mut coverage = Vec::with_capacity(incoming.coverage().len());
    for (index, (&left, &right)) in base.coverage().iter().zip(incoming.coverage()).enumerate() {
        if index % 4_096 == 0 {
            context.report("compose_pixels", index as u64, pixels)?;
        }
        coverage.push(match mode {
            CompositionMode::Replace => right,
            CompositionMode::Add => left.max(right),
            CompositionMode::Subtract => {
                ((u16::from(left) * u16::from(255 - right) + 127) / 255) as u8
            }
            CompositionMode::Intersect => ((u16::from(left) * u16::from(right) + 127) / 255) as u8,
        });
    }
    context.report("compose_pixels", pixels, pixels)?;
    MaskBitmap::from_coverage(base.width(), base.height(), coverage)
}

pub fn apply(
    mask: &MaskBitmap,
    operation: &MaskOperation,
    cancelled: Option<&AtomicBool>,
) -> Result<MaskBitmap, AppError> {
    apply_with_progress(
        mask,
        operation,
        MaskWorkContext::cancellation_only(cancelled),
    )
}

pub(crate) fn apply_with_progress(
    mask: &MaskBitmap,
    operation: &MaskOperation,
    context: MaskWorkContext<'_>,
) -> Result<MaskBitmap, AppError> {
    let pixels = mask.coverage().len() as u64;
    match operation {
        MaskOperation::SelectAll => {
            context.check_cancelled()?;
            let result = MaskBitmap::full(mask.width(), mask.height())?;
            context.report("select_all_pixels", pixels, pixels)?;
            Ok(result)
        }
        MaskOperation::Deselect => {
            context.check_cancelled()?;
            let result = MaskBitmap::empty(mask.width(), mask.height())?;
            context.report("deselect_pixels", pixels, pixels)?;
            Ok(result)
        }
        MaskOperation::Invert => invert(mask, context),
        MaskOperation::Feather { radius } => {
            feather_with_progress(mask, (*radius).min(256), context)
        }
        MaskOperation::Expand { radius } => morphology(
            mask,
            (*radius).min(256),
            true,
            context,
            "expand_horizontal",
            "expand_vertical",
        ),
        MaskOperation::Contract { radius } => morphology(
            mask,
            (*radius).min(256),
            false,
            context,
            "contract_horizontal",
            "contract_vertical",
        ),
        MaskOperation::Smooth { radius } => {
            let softened = feather_with_progress(mask, (*radius).min(128), context)?;
            contrast(&softened, 1.8, context, "smooth_contrast")
        }
        MaskOperation::FillHoles => fill_holes(mask, context),
        MaskOperation::RemoveSmallIslands { minimum_pixels } => {
            remove_small_islands(mask, (*minimum_pixels).min(10_000_000), context)
        }
        MaskOperation::Border { width } => {
            let width = (*width).min(256);
            let outside = morphology(
                mask,
                width,
                true,
                context,
                "border_outer_horizontal",
                "border_outer_vertical",
            )?;
            let inside = morphology(
                mask,
                width,
                false,
                context,
                "border_inner_horizontal",
                "border_inner_vertical",
            )?;
            let mut coverage = Vec::with_capacity(mask.coverage().len());
            for (index, (&outer, &inner)) in
                outside.coverage().iter().zip(inside.coverage()).enumerate()
            {
                if index % 4_096 == 0 {
                    context.report("border_combine", index as u64, pixels)?;
                }
                coverage.push(outer.saturating_sub(inner));
            }
            context.report("border_combine", pixels, pixels)?;
            MaskBitmap::from_coverage(mask.width(), mask.height(), coverage)
        }
        MaskOperation::Refine {
            smooth,
            feather: feather_radius,
            contrast: contrast_amount,
            shift_edge,
        } => {
            if !contrast_amount.is_finite() || !(-1.0..=1.0).contains(contrast_amount) {
                return Err(AppError::InvalidMask(
                    "refine contrast must be between -1 and 1".into(),
                ));
            }
            let mut result = if *smooth == 0 {
                mask.clone()
            } else {
                apply_with_progress(
                    mask,
                    &MaskOperation::Smooth {
                        radius: (*smooth).min(128),
                    },
                    context,
                )?
            };
            if *shift_edge > 0 {
                result = morphology(
                    &result,
                    (*shift_edge as u32).min(256),
                    true,
                    context,
                    "refine_expand_horizontal",
                    "refine_expand_vertical",
                )?;
            } else if *shift_edge < 0 {
                result = morphology(
                    &result,
                    shift_edge.unsigned_abs().min(256),
                    false,
                    context,
                    "refine_contract_horizontal",
                    "refine_contract_vertical",
                )?;
            }
            if *feather_radius > 0 {
                result = feather_with_progress(&result, (*feather_radius).min(256), context)?;
            }
            contrast(
                &result,
                1.0 + contrast_amount * 3.0,
                context,
                "refine_contrast",
            )
        }
    }
}

fn invert(mask: &MaskBitmap, context: MaskWorkContext<'_>) -> Result<MaskBitmap, AppError> {
    let pixels = mask.coverage().len() as u64;
    let mut coverage = Vec::with_capacity(mask.coverage().len());
    for (index, value) in mask.coverage().iter().enumerate() {
        if index % 4_096 == 0 {
            context.report("invert_pixels", index as u64, pixels)?;
        }
        coverage.push(255 - value);
    }
    context.report("invert_pixels", pixels, pixels)?;
    MaskBitmap::from_coverage(mask.width(), mask.height(), coverage)
}

fn contrast(
    mask: &MaskBitmap,
    amount: f32,
    context: MaskWorkContext<'_>,
    phase: &str,
) -> Result<MaskBitmap, AppError> {
    let factor = amount.max(0.05);
    let pixels = mask.coverage().len() as u64;
    let mut coverage = Vec::with_capacity(mask.coverage().len());
    for (index, value) in mask.coverage().iter().enumerate() {
        if index % 4_096 == 0 {
            context.report(phase, index as u64, pixels)?;
        }
        coverage.push(
            ((*value as f32 - 127.5) * factor + 127.5)
                .round()
                .clamp(0.0, 255.0) as u8,
        );
    }
    context.report(phase, pixels, pixels)?;
    MaskBitmap::from_coverage(mask.width(), mask.height(), coverage)
}

fn morphology(
    mask: &MaskBitmap,
    radius: u32,
    expand: bool,
    context: MaskWorkContext<'_>,
    horizontal_phase: &str,
    vertical_phase: &str,
) -> Result<MaskBitmap, AppError> {
    let pixels = mask.coverage().len() as u64;
    if radius == 0 {
        context.report(horizontal_phase, pixels, pixels)?;
        return Ok(mask.clone());
    }
    let radius = radius as usize;
    let width = mask.width() as usize;
    let height = mask.height() as usize;
    let mut horizontal = vec![0_u8; mask.coverage().len()];
    for y in 0..height {
        if y % 16 == 0 {
            context.report(horizontal_phase, (y * width) as u64, pixels)?;
        }
        let row = &mask.coverage()[y * width..(y + 1) * width];
        horizontal[y * width..(y + 1) * width]
            .copy_from_slice(&sliding_extreme(row, radius, expand, context)?);
    }
    context.report(horizontal_phase, pixels, pixels)?;
    let mut output = MaskBitmap::empty(mask.width(), mask.height())?;
    for x in 0..width {
        if x % 16 == 0 {
            context.report(vertical_phase, (x * height) as u64, pixels)?;
        }
        let mut column = Vec::with_capacity(height);
        for y in 0..height {
            if y % 4_096 == 0 {
                context.check_cancelled()?;
            }
            column.push(horizontal[y * width + x]);
        }
        for (y, value) in sliding_extreme(&column, radius, expand, context)?
            .into_iter()
            .enumerate()
        {
            if y % 4_096 == 0 {
                context.check_cancelled()?;
            }
            output.set(x as u32, y as u32, value);
        }
    }
    context.report(vertical_phase, pixels, pixels)?;
    Ok(output)
}

fn sliding_extreme(
    values: &[u8],
    radius: usize,
    maximum: bool,
    context: MaskWorkContext<'_>,
) -> Result<Vec<u8>, AppError> {
    let mut output = vec![0_u8; values.len()];
    let mut deque = VecDeque::<usize>::new();
    let mut next = 0_usize;
    for (index, target) in output.iter_mut().enumerate() {
        if index % 4_096 == 0 {
            context.check_cancelled()?;
        }
        let end = (index + radius + 1).min(values.len());
        while next < end {
            while deque.back().is_some_and(|candidate| {
                if maximum {
                    values[*candidate] <= values[next]
                } else {
                    values[*candidate] >= values[next]
                }
            }) {
                deque.pop_back();
            }
            deque.push_back(next);
            next += 1;
        }
        let start = index.saturating_sub(radius);
        while deque.front().is_some_and(|candidate| *candidate < start) {
            deque.pop_front();
        }
        *target = values[*deque.front().expect("sliding window is non-empty")];
    }
    Ok(output)
}

fn fill_holes(mask: &MaskBitmap, context: MaskWorkContext<'_>) -> Result<MaskBitmap, AppError> {
    let width = mask.width() as usize;
    let height = mask.height() as usize;
    let mut outside = vec![false; mask.coverage().len()];
    let mut queue = VecDeque::new();
    for x in 0..width {
        if x % 4_096 == 0 {
            context.check_cancelled()?;
        }
        queue_background(mask, x, 0, &mut outside, &mut queue);
        queue_background(mask, x, height - 1, &mut outside, &mut queue);
    }
    for y in 0..height {
        if y % 4_096 == 0 {
            context.check_cancelled()?;
        }
        queue_background(mask, 0, y, &mut outside, &mut queue);
        queue_background(mask, width - 1, y, &mut outside, &mut queue);
    }
    let mut visited = 0_usize;
    while let Some((x, y)) = queue.pop_front() {
        if visited % 4_096 == 0 {
            // The reachable exterior size is data-dependent, so this phase is
            // deliberately phase-only rather than a fabricated percentage.
            context.report("fill_holes_flood", 0, 0)?;
        }
        visited += 1;
        for (next_x, next_y) in neighbors4(x, y, width, height) {
            queue_background(mask, next_x, next_y, &mut outside, &mut queue);
        }
    }
    let pixels = mask.coverage().len() as u64;
    let mut coverage = Vec::with_capacity(mask.coverage().len());
    for (index, value) in mask.coverage().iter().enumerate() {
        if index % 4_096 == 0 {
            context.report("fill_holes_finalize", index as u64, pixels)?;
        }
        coverage.push(if *value < 128 && !outside[index] {
            255
        } else {
            *value
        });
    }
    context.report("fill_holes_finalize", pixels, pixels)?;
    MaskBitmap::from_coverage(mask.width(), mask.height(), coverage)
}

fn queue_background(
    mask: &MaskBitmap,
    x: usize,
    y: usize,
    visited: &mut [bool],
    queue: &mut VecDeque<(usize, usize)>,
) {
    let index = y * mask.width() as usize + x;
    if !visited[index] && mask.coverage()[index] < 128 {
        visited[index] = true;
        queue.push_back((x, y));
    }
}

fn remove_small_islands(
    mask: &MaskBitmap,
    minimum_pixels: u32,
    context: MaskWorkContext<'_>,
) -> Result<MaskBitmap, AppError> {
    let pixels = mask.coverage().len() as u64;
    if minimum_pixels <= 1 {
        context.report("remove_islands_scan", pixels, pixels)?;
        return Ok(mask.clone());
    }
    let width = mask.width() as usize;
    let height = mask.height() as usize;
    let mut output = mask.clone();
    let mut visited = vec![false; mask.coverage().len()];
    let mut counter = 0_usize;
    for y in 0..height {
        if y % 16 == 0 {
            context.report("remove_islands_scan", (y * width) as u64, pixels)?;
        }
        for x in 0..width {
            if x % 4_096 == 0 {
                context.check_cancelled()?;
            }
            let start = y * width + x;
            if visited[start] || mask.coverage()[start] < 128 {
                continue;
            }
            let mut component = Vec::new();
            let mut queue = VecDeque::from([(x, y)]);
            visited[start] = true;
            while let Some((current_x, current_y)) = queue.pop_front() {
                if counter % 4_096 == 0 {
                    context.check_cancelled()?;
                }
                counter += 1;
                component.push(current_y * width + current_x);
                for (next_x, next_y) in neighbors4(current_x, current_y, width, height) {
                    let index = next_y * width + next_x;
                    if !visited[index] && mask.coverage()[index] >= 128 {
                        visited[index] = true;
                        queue.push_back((next_x, next_y));
                    }
                }
            }
            if component.len() < minimum_pixels as usize {
                for (component_index, index) in component.into_iter().enumerate() {
                    if component_index % 4_096 == 0 {
                        context.check_cancelled()?;
                    }
                    output.coverage_mut()[index] = 0;
                }
            }
        }
    }
    context.report("remove_islands_scan", pixels, pixels)?;
    Ok(output)
}

fn neighbors4(x: usize, y: usize, width: usize, height: usize) -> Vec<(usize, usize)> {
    let mut neighbors = Vec::with_capacity(4);
    if x > 0 {
        neighbors.push((x - 1, y));
    }
    if x + 1 < width {
        neighbors.push((x + 1, y));
    }
    if y > 0 {
        neighbors.push((x, y - 1));
    }
    if y + 1 < height {
        neighbors.push((x, y + 1));
    }
    neighbors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_modes_use_coverage_math() {
        let base = MaskBitmap::from_coverage(1, 1, vec![128]).unwrap();
        let incoming = MaskBitmap::from_coverage(1, 1, vec![128]).unwrap();
        assert_eq!(
            compose(Some(&base), &incoming, CompositionMode::Replace)
                .unwrap()
                .get(0, 0),
            128
        );
        assert_eq!(
            compose(Some(&base), &incoming, CompositionMode::Add)
                .unwrap()
                .get(0, 0),
            128
        );
        assert_eq!(
            compose(Some(&base), &incoming, CompositionMode::Subtract)
                .unwrap()
                .get(0, 0),
            64
        );
        assert_eq!(
            compose(Some(&base), &incoming, CompositionMode::Intersect)
                .unwrap()
                .get(0, 0),
            64
        );
    }

    #[test]
    fn invert_expand_and_contract_respect_boundaries() {
        let mut mask = MaskBitmap::empty(5, 5).unwrap();
        mask.set(2, 2, 255);
        assert_eq!(
            apply(&mask, &MaskOperation::Invert, None)
                .unwrap()
                .get(2, 2),
            0
        );
        let expanded = apply(&mask, &MaskOperation::Expand { radius: 1 }, None).unwrap();
        assert_eq!(expanded.get(1, 1), 255);
        let contracted = apply(&expanded, &MaskOperation::Contract { radius: 1 }, None).unwrap();
        assert_eq!(contracted.get(2, 2), 255);
    }

    #[test]
    fn holes_and_small_islands_are_classified() {
        let mut mask = MaskBitmap::full(7, 7).unwrap();
        mask.set(3, 3, 0);
        assert_eq!(
            apply(&mask, &MaskOperation::FillHoles, None)
                .unwrap()
                .get(3, 3),
            255
        );
        let mut islands = MaskBitmap::empty(7, 7).unwrap();
        islands.set(1, 1, 255);
        for y in 3..6 {
            for x in 3..6 {
                islands.set(x, y, 255);
            }
        }
        let cleaned = apply(
            &islands,
            &MaskOperation::RemoveSmallIslands { minimum_pixels: 4 },
            None,
        )
        .unwrap();
        assert_eq!(cleaned.get(1, 1), 0);
        assert_eq!(cleaned.get(4, 4), 255);
    }

    #[test]
    fn large_pixel_operation_reports_intermediate_work_and_cancels_by_chunk() {
        use std::sync::Mutex;

        let mask = MaskBitmap::full(10_000, 1).unwrap();
        let reports = Mutex::new(Vec::new());
        let callback = |phase: &str, completed: u64, total: u64| {
            reports
                .lock()
                .unwrap()
                .push((phase.to_owned(), completed, total));
            Ok(())
        };
        apply_with_progress(
            &mask,
            &MaskOperation::Invert,
            MaskWorkContext::new(None, Some(&callback)),
        )
        .unwrap();
        let reports = reports.into_inner().unwrap();
        assert!(reports
            .iter()
            .any(|(_, completed, total)| *completed > 0 && *completed < *total));
        assert!(reports
            .iter()
            .all(|(_, completed, total)| completed <= total));
        assert!(reports.iter().all(|(_, _, total)| *total == 10_000));

        let cancelled = AtomicBool::new(false);
        let cancel_callback = |_: &str, completed: u64, _: u64| {
            if completed >= 4_096 {
                cancelled.store(true, std::sync::atomic::Ordering::Release);
            }
            Ok(())
        };
        assert!(matches!(
            apply_with_progress(
                &mask,
                &MaskOperation::Invert,
                MaskWorkContext::new(Some(&cancelled), Some(&cancel_callback)),
            ),
            Err(AppError::MaskCancelled)
        ));
    }
}
