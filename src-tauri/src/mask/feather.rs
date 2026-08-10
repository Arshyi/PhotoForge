use super::bitmap::MaskBitmap;
use super::progress::MaskWorkContext;
use crate::error::AppError;
use std::sync::atomic::AtomicBool;

pub fn feather(
    mask: &MaskBitmap,
    radius: u32,
    cancelled: Option<&AtomicBool>,
) -> Result<MaskBitmap, AppError> {
    feather_with_progress(mask, radius, MaskWorkContext::cancellation_only(cancelled))
}

pub(crate) fn feather_with_progress(
    mask: &MaskBitmap,
    radius: u32,
    context: MaskWorkContext<'_>,
) -> Result<MaskBitmap, AppError> {
    let pixels = mask.coverage().len() as u64;
    if radius == 0 {
        context.report("feather_copy", pixels, pixels)?;
        return Ok(mask.clone());
    }
    let radius = radius.min(256) as usize;
    let width = mask.width() as usize;
    let height = mask.height() as usize;
    let mut horizontal = vec![0_u8; mask.coverage().len()];
    for y in 0..height {
        if y % 16 == 0 {
            context.report("feather_horizontal", (y * width) as u64, pixels)?;
        }
        let row = &mask.coverage()[y * width..(y + 1) * width];
        let mut prefix = vec![0_u32; width + 1];
        for (index, value) in row.iter().enumerate() {
            if index % 4_096 == 0 {
                context.check_cancelled()?;
            }
            prefix[index + 1] = prefix[index] + u32::from(*value);
        }
        for x in 0..width {
            if x % 4_096 == 0 {
                context.check_cancelled()?;
            }
            let start = x.saturating_sub(radius);
            let end = (x + radius + 1).min(width);
            horizontal[y * width + x] =
                ((prefix[end] - prefix[start]) / (end - start) as u32) as u8;
        }
    }
    context.report("feather_horizontal", pixels, pixels)?;

    let mut output = MaskBitmap::empty(mask.width(), mask.height())?;
    for x in 0..width {
        if x % 16 == 0 {
            context.report("feather_vertical", (x * height) as u64, pixels)?;
        }
        let mut prefix = vec![0_u32; height + 1];
        for y in 0..height {
            if y % 4_096 == 0 {
                context.check_cancelled()?;
            }
            prefix[y + 1] = prefix[y] + u32::from(horizontal[y * width + x]);
        }
        for y in 0..height {
            if y % 4_096 == 0 {
                context.check_cancelled()?;
            }
            let start = y.saturating_sub(radius);
            let end = (y + radius + 1).min(height);
            output.set(
                x as u32,
                y as u32,
                ((prefix[end] - prefix[start]) / (end - start) as u32) as u8,
            );
        }
    }
    context.report("feather_vertical", pixels, pixels)?;
    Ok(output)
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

    #[test]
    fn cancellation_is_checked_inside_extreme_aspect_ratio_rows_and_columns() {
        for mask in [
            MaskBitmap::full(8_192, 1).unwrap(),
            MaskBitmap::full(1, 8_192).unwrap(),
        ] {
            let cancelled = AtomicBool::new(false);
            let callback = |_: &str, _: u64, _: u64| {
                cancelled.store(true, std::sync::atomic::Ordering::Release);
                Ok(())
            };
            assert!(matches!(
                feather_with_progress(
                    &mask,
                    10,
                    MaskWorkContext::new(Some(&cancelled), Some(&callback)),
                ),
                Err(AppError::MaskCancelled)
            ));
        }
    }
}
