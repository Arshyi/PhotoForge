use super::bitmap::MaskBitmap;
use super::feather::feather;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};

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

pub fn compose(
    base: Option<&MaskBitmap>,
    incoming: &MaskBitmap,
    mode: CompositionMode,
) -> Result<MaskBitmap, AppError> {
    let Some(base) = base else {
        return Ok(match mode {
            CompositionMode::Subtract | CompositionMode::Intersect => {
                MaskBitmap::empty(incoming.width(), incoming.height())?
            }
            CompositionMode::Replace | CompositionMode::Add => incoming.clone(),
        });
    };
    if base.width() != incoming.width() || base.height() != incoming.height() {
        return Err(AppError::MaskDimensionMismatch {
            mask_width: incoming.width(),
            mask_height: incoming.height(),
            image_width: base.width(),
            image_height: base.height(),
        });
    }
    let coverage = base
        .coverage()
        .iter()
        .zip(incoming.coverage())
        .map(|(&left, &right)| match mode {
            CompositionMode::Replace => right,
            CompositionMode::Add => left.max(right),
            CompositionMode::Subtract => {
                ((u16::from(left) * u16::from(255 - right) + 127) / 255) as u8
            }
            CompositionMode::Intersect => ((u16::from(left) * u16::from(right) + 127) / 255) as u8,
        })
        .collect();
    MaskBitmap::from_coverage(base.width(), base.height(), coverage)
}

pub fn apply(
    mask: &MaskBitmap,
    operation: &MaskOperation,
    cancelled: Option<&AtomicBool>,
) -> Result<MaskBitmap, AppError> {
    match operation {
        MaskOperation::SelectAll => MaskBitmap::full(mask.width(), mask.height()),
        MaskOperation::Deselect => MaskBitmap::empty(mask.width(), mask.height()),
        MaskOperation::Invert => MaskBitmap::from_coverage(
            mask.width(),
            mask.height(),
            mask.coverage().iter().map(|value| 255 - value).collect(),
        ),
        MaskOperation::Feather { radius } => feather(mask, (*radius).min(256), cancelled),
        MaskOperation::Expand { radius } => morphology(mask, (*radius).min(256), true, cancelled),
        MaskOperation::Contract { radius } => {
            morphology(mask, (*radius).min(256), false, cancelled)
        }
        MaskOperation::Smooth { radius } => {
            let softened = feather(mask, (*radius).min(128), cancelled)?;
            Ok(contrast(&softened, 1.8))
        }
        MaskOperation::FillHoles => fill_holes(mask, cancelled),
        MaskOperation::RemoveSmallIslands { minimum_pixels } => {
            remove_small_islands(mask, (*minimum_pixels).min(10_000_000), cancelled)
        }
        MaskOperation::Border { width } => {
            let width = (*width).min(256);
            let outside = morphology(mask, width, true, cancelled)?;
            let inside = morphology(mask, width, false, cancelled)?;
            MaskBitmap::from_coverage(
                mask.width(),
                mask.height(),
                outside
                    .coverage()
                    .iter()
                    .zip(inside.coverage())
                    .map(|(&outer, &inner)| outer.saturating_sub(inner))
                    .collect(),
            )
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
                apply(
                    mask,
                    &MaskOperation::Smooth {
                        radius: (*smooth).min(128),
                    },
                    cancelled,
                )?
            };
            if *shift_edge > 0 {
                result = morphology(&result, (*shift_edge as u32).min(256), true, cancelled)?;
            } else if *shift_edge < 0 {
                result = morphology(
                    &result,
                    shift_edge.unsigned_abs().min(256),
                    false,
                    cancelled,
                )?;
            }
            if *feather_radius > 0 {
                result = feather(&result, (*feather_radius).min(256), cancelled)?;
            }
            Ok(contrast(&result, 1.0 + contrast_amount * 3.0))
        }
    }
}

fn contrast(mask: &MaskBitmap, amount: f32) -> MaskBitmap {
    let factor = amount.max(0.05);
    MaskBitmap::from_coverage(
        mask.width(),
        mask.height(),
        mask.coverage()
            .iter()
            .map(|value| {
                ((*value as f32 - 127.5) * factor + 127.5)
                    .round()
                    .clamp(0.0, 255.0) as u8
            })
            .collect(),
    )
    .expect("dimensions were already validated")
}

fn morphology(
    mask: &MaskBitmap,
    radius: u32,
    expand: bool,
    cancelled: Option<&AtomicBool>,
) -> Result<MaskBitmap, AppError> {
    if radius == 0 {
        return Ok(mask.clone());
    }
    let radius = radius as usize;
    let width = mask.width() as usize;
    let height = mask.height() as usize;
    let mut horizontal = vec![0_u8; mask.coverage().len()];
    for y in 0..height {
        check_cancelled(cancelled, y)?;
        let row = &mask.coverage()[y * width..(y + 1) * width];
        horizontal[y * width..(y + 1) * width]
            .copy_from_slice(&sliding_extreme(row, radius, expand));
    }
    let mut output = MaskBitmap::empty(mask.width(), mask.height())?;
    for x in 0..width {
        check_cancelled(cancelled, x)?;
        let column: Vec<u8> = (0..height).map(|y| horizontal[y * width + x]).collect();
        for (y, value) in sliding_extreme(&column, radius, expand)
            .into_iter()
            .enumerate()
        {
            output.set(x as u32, y as u32, value);
        }
    }
    Ok(output)
}

fn sliding_extreme(values: &[u8], radius: usize, maximum: bool) -> Vec<u8> {
    let mut output = vec![0_u8; values.len()];
    let mut deque = VecDeque::<usize>::new();
    let mut next = 0_usize;
    for (index, target) in output.iter_mut().enumerate() {
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
    output
}

fn fill_holes(mask: &MaskBitmap, cancelled: Option<&AtomicBool>) -> Result<MaskBitmap, AppError> {
    let width = mask.width() as usize;
    let height = mask.height() as usize;
    let mut outside = vec![false; mask.coverage().len()];
    let mut queue = VecDeque::new();
    for x in 0..width {
        queue_background(mask, x, 0, &mut outside, &mut queue);
        queue_background(mask, x, height - 1, &mut outside, &mut queue);
    }
    for y in 0..height {
        queue_background(mask, 0, y, &mut outside, &mut queue);
        queue_background(mask, width - 1, y, &mut outside, &mut queue);
    }
    let mut visited = 0_usize;
    while let Some((x, y)) = queue.pop_front() {
        check_cancelled(cancelled, visited)?;
        visited += 1;
        for (next_x, next_y) in neighbors4(x, y, width, height) {
            queue_background(mask, next_x, next_y, &mut outside, &mut queue);
        }
    }
    MaskBitmap::from_coverage(
        mask.width(),
        mask.height(),
        mask.coverage()
            .iter()
            .enumerate()
            .map(|(index, value)| {
                if *value < 128 && !outside[index] {
                    255
                } else {
                    *value
                }
            })
            .collect(),
    )
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
    cancelled: Option<&AtomicBool>,
) -> Result<MaskBitmap, AppError> {
    if minimum_pixels <= 1 {
        return Ok(mask.clone());
    }
    let width = mask.width() as usize;
    let height = mask.height() as usize;
    let mut output = mask.clone();
    let mut visited = vec![false; mask.coverage().len()];
    let mut counter = 0_usize;
    for y in 0..height {
        for x in 0..width {
            let start = y * width + x;
            if visited[start] || mask.coverage()[start] < 128 {
                continue;
            }
            let mut component = Vec::new();
            let mut queue = VecDeque::from([(x, y)]);
            visited[start] = true;
            while let Some((current_x, current_y)) = queue.pop_front() {
                check_cancelled(cancelled, counter)?;
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
                for index in component {
                    output.coverage_mut()[index] = 0;
                }
            }
        }
    }
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

fn check_cancelled(cancelled: Option<&AtomicBool>, counter: usize) -> Result<(), AppError> {
    if counter % 4_096 == 0 && cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(AppError::MaskCancelled)
    } else {
        Ok(())
    }
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
}
