use super::bitmap::MaskBitmap;
use super::progress::{MaskWorkContext, IO_PROGRESS_CHUNK_PIXELS};
use crate::error::AppError;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaskDiagnostics {
    pub width: u32,
    pub height: u32,
    pub selected_pixels: u64,
    pub fully_selected_pixels: u64,
    pub average_coverage: f64,
    pub bounds: Option<[u32; 4]>,
    pub memory_bytes: u64,
}

pub fn inspect(mask: &MaskBitmap) -> MaskDiagnostics {
    inspect_with_progress(mask, MaskWorkContext::new(None, None))
        .expect("mask diagnostics without cancellation is infallible")
}

pub(crate) fn inspect_with_progress(
    mask: &MaskBitmap,
    context: MaskWorkContext<'_>,
) -> Result<MaskDiagnostics, AppError> {
    let mut selected = 0_u64;
    let mut fully_selected = 0_u64;
    let mut coverage_sum = 0_u64;
    let mut left = mask.width();
    let mut top = mask.height();
    let mut right = 0_u32;
    let mut bottom = 0_u32;
    let total = mask.coverage().len() as u64;
    context.report("inspect_mask_pixels", 0, total)?;
    let mut completed = 0_u64;
    for y in 0..mask.height() {
        for x in 0..mask.width() {
            let value = mask.get(x, y);
            coverage_sum += u64::from(value);
            if value > 0 {
                selected += 1;
                fully_selected += u64::from(value == 255);
                left = left.min(x);
                top = top.min(y);
                right = right.max(x);
                bottom = bottom.max(y);
            }
            completed += 1;
            if completed % IO_PROGRESS_CHUNK_PIXELS == 0 || completed == total {
                context.report("inspect_mask_pixels", completed, total)?;
            }
        }
    }
    Ok(MaskDiagnostics {
        width: mask.width(),
        height: mask.height(),
        selected_pixels: selected,
        fully_selected_pixels: fully_selected,
        average_coverage: coverage_sum as f64 / mask.coverage().len() as f64 / 255.0,
        bounds: (selected > 0).then_some([left, top, right + 1, bottom + 1]),
        memory_bytes: mask.coverage().len() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_area_and_bounds() {
        let mut mask = MaskBitmap::empty(4, 4).unwrap();
        mask.set(1, 2, 255);
        mask.set(2, 2, 128);
        let diagnostics = inspect(&mask);
        assert_eq!(diagnostics.selected_pixels, 2);
        assert_eq!(diagnostics.fully_selected_pixels, 1);
        assert_eq!(diagnostics.bounds, Some([1, 2, 3, 3]));
        assert_eq!(diagnostics.memory_bytes, 16);
    }
}
