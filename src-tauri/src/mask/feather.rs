use super::bitmap::MaskBitmap;
use crate::error::AppError;
use std::sync::atomic::{AtomicBool, Ordering};

pub fn feather(
    mask: &MaskBitmap,
    radius: u32,
    cancelled: Option<&AtomicBool>,
) -> Result<MaskBitmap, AppError> {
    if radius == 0 {
        return Ok(mask.clone());
    }
    let radius = radius.min(256) as usize;
    let width = mask.width() as usize;
    let height = mask.height() as usize;
    let mut horizontal = vec![0_u8; mask.coverage().len()];
    for y in 0..height {
        check_cancelled(cancelled, y)?;
        let row = &mask.coverage()[y * width..(y + 1) * width];
        let mut prefix = vec![0_u32; width + 1];
        for (index, value) in row.iter().enumerate() {
            prefix[index + 1] = prefix[index] + u32::from(*value);
        }
        for x in 0..width {
            let start = x.saturating_sub(radius);
            let end = (x + radius + 1).min(width);
            horizontal[y * width + x] =
                ((prefix[end] - prefix[start]) / (end - start) as u32) as u8;
        }
    }

    let mut output = MaskBitmap::empty(mask.width(), mask.height())?;
    for x in 0..width {
        check_cancelled(cancelled, x)?;
        let mut prefix = vec![0_u32; height + 1];
        for y in 0..height {
            prefix[y + 1] = prefix[y] + u32::from(horizontal[y * width + x]);
        }
        for y in 0..height {
            let start = y.saturating_sub(radius);
            let end = (y + radius + 1).min(height);
            output.set(
                x as u32,
                y as u32,
                ((prefix[end] - prefix[start]) / (end - start) as u32) as u8,
            );
        }
    }
    Ok(output)
}

fn check_cancelled(cancelled: Option<&AtomicBool>, counter: usize) -> Result<(), AppError> {
    if counter % 64 == 0 && cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(AppError::MaskCancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feather_creates_partial_coverage_and_preserves_dimensions() {
        let mut mask = MaskBitmap::empty(9, 9).unwrap();
        mask.set(4, 4, 255);
        let result = feather(&mask, 2, None).unwrap();
        assert_eq!((result.width(), result.height()), (9, 9));
        assert!(result
            .coverage()
            .iter()
            .any(|value| (1..=254).contains(value)));
    }

    #[test]
    fn cancellation_is_acknowledged() {
        let mask = MaskBitmap::full(128, 128).unwrap();
        let cancelled = AtomicBool::new(true);
        assert!(matches!(
            feather(&mask, 10, Some(&cancelled)),
            Err(AppError::MaskCancelled)
        ));
    }
}
