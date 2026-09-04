//! Checked admission estimates for CPU decode and rendering, before allocation.
use crate::error::AppError;
use serde::Serialize;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, MutexGuard, TryLockError};

static CPU_JOB: Mutex<()> = Mutex::new(());
/// Serialize full-frame workers, including batch, with cancellable admission.
/// Call only in blocking workers, before taking any session/store lock.
pub fn acquire_job(cancel: Option<&AtomicBool>) -> Result<MutexGuard<'static, ()>, AppError> {
    loop {
        crate::image_processing::high_precision::check_cancel(cancel)?;
        match CPU_JOB.try_lock() {
            Ok(guard) => return Ok(guard),
            Err(TryLockError::Poisoned(_)) => {
                return Err(AppError::ProcessingFailure(
                    "CPU job gate is unavailable".into(),
                ))
            }
            Err(TryLockError::WouldBlock) => {
                std::thread::sleep(std::time::Duration::from_millis(20))
            }
        }
    }
}

pub const MAX_WORKING_IMAGE_BYTES: u64 = 1_073_741_824;
pub const MAX_JOB_BYTES: u64 = 4_294_967_296;
pub const MAX_DIMENSION: u32 = 20_000;
/// All working pixels occupy 16 bytes. This bound follows from a byte budget.
pub const MAX_WORKING_PIXELS: u64 = MAX_WORKING_IMAGE_BYTES / 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceEstimate {
    pub source_bytes: u64,
    pub scratch_bytes: u64,
    pub mask_bytes: u64,
    pub output_bytes: u64,
    pub estimated_peak_bytes: u64,
}

pub fn checked_pixels(width: u32, height: u32) -> Result<u64, AppError> {
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(AppError::OutOfMemoryRisk)?;
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || pixels > MAX_WORKING_PIXELS
    {
        return Err(AppError::ImageTooLarge {
            pixels,
            limit: MAX_WORKING_PIXELS,
        });
    }
    Ok(pixels)
}

impl ResourceEstimate {
    pub fn pipeline(
        width: u32,
        height: u32,
        operations: &[crate::domain::EditOperation],
        resident: u64,
    ) -> Result<Self, AppError> {
        let n = checked_pixels(width, height)?;
        let frames = operations
            .iter()
            .map(crate::image_processing::high_precision::scratch_frames)
            .max()
            .unwrap_or(0);
        Self::new(
            resident,
            n.checked_mul(16)
                .and_then(|n| n.checked_mul(frames))
                .ok_or(AppError::OutOfMemoryRisk)?,
            0,
            n * 8 + 40_960_000,
        )
    }
    pub fn new(
        source_bytes: u64,
        scratch_bytes: u64,
        mask_bytes: u64,
        output_bytes: u64,
    ) -> Result<Self, AppError> {
        let total = source_bytes
            .checked_add(scratch_bytes)
            .and_then(|n| n.checked_add(mask_bytes))
            .and_then(|n| n.checked_add(output_bytes))
            .ok_or(AppError::OutOfMemoryRisk)?;
        if total > MAX_JOB_BYTES {
            return Err(AppError::ResourceBudget {
                required: total,
                limit: MAX_JOB_BYTES,
            });
        }
        Ok(Self {
            source_bytes,
            scratch_bytes,
            mask_bytes,
            output_bytes,
            estimated_peak_bytes: total,
        })
    }
    /// Sensor, demosaic, camera transform, development copy and display buffers.
    pub fn decode(width: u32, height: u32, compressed_bytes: u64) -> Result<Self, AppError> {
        let n = checked_pixels(width, height)?;
        Self::new(
            compressed_bytes,
            n.checked_mul(52).ok_or(AppError::OutOfMemoryRisk)?,
            0,
            n * 4 + 40_960_000,
        )
    }
    pub fn render(
        document: &crate::layers::LayerDocument,
        scale: f64,
        resident: u64,
        promotion_bytes: u64,
    ) -> Result<Self, AppError> {
        let w = ((f64::from(document.canvas_width) * scale).round() as u32).max(1);
        let h = ((f64::from(document.canvas_height) * scale).round() as u32).max(1);
        let n = checked_pixels(w, h)?;
        let mut max_depth = 0_u64;
        let mut adjustment_frames = 0_u64;
        let mut masks = 0_u64;
        let mut stack: Vec<_> = document.layers.iter().map(|l| (l, 0_u64)).collect();
        while let Some((layer, depth)) = stack.pop() {
            if !layer.visible || layer.opacity <= 0.0 {
                continue;
            }
            if let Some(mask) = &layer.mask {
                masks = masks
                    .checked_add(u64::from(mask.snapshot.width) * u64::from(mask.snapshot.height))
                    .ok_or(AppError::OutOfMemoryRisk)?;
            }
            match &layer.content {
                crate::layers::LayerContent::Group { children, .. } => {
                    max_depth = max_depth.max(depth + 1);
                    stack.extend(children.iter().map(|l| (l, depth + 1)));
                }
                crate::layers::LayerContent::Adjustment { operation } => {
                    let frames = crate::image_processing::high_precision::scratch_frames(operation);
                    adjustment_frames = adjustment_frames.max(frames);
                }
                _ => {}
            }
        }
        // Canvas + one promoted encoded source + the deepest live groups and
        // operation scratch. Reserve PNG16-sized output even for preview.
        let scratch = n
            .checked_mul(16)
            .and_then(|b| b.checked_mul(1 + max_depth + adjustment_frames))
            .and_then(|b| b.checked_add(promotion_bytes))
            .ok_or(AppError::OutOfMemoryRisk)?;
        Self::new(resident, scratch, masks, n * 8 + 40_960_000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflow_and_decompression_bombs_fail_before_allocation() {
        assert!(checked_pixels(u32::MAX, u32::MAX).is_err());
        assert!(ResourceEstimate::new(u64::MAX, 1, 0, 0).is_err());
        assert!(checked_pixels(20000, 20000).is_err());
    }
    #[test]
    fn legitimate_45_and_61_mp_dimensions_have_explicit_byte_costs() {
        for (w, h) in [(8256, 5504), (9504, 6336)] {
            let estimate = ResourceEstimate::decode(w, h, 100_000_000).unwrap();
            assert!(estimate.estimated_peak_bytes < MAX_JOB_BYTES);
            assert!(estimate.scratch_bytes > 2_000_000_000);
        }
    }
    #[test]
    fn excessive_live_buffers_are_rejected_even_when_dimensions_are_legal() {
        assert!(matches!(
            ResourceEstimate::new(3_000_000_000, 2_000_000_000, 0, 0),
            Err(AppError::ResourceBudget { .. })
        ));
    }
}
