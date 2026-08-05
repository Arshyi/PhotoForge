use crate::error::AppError;

pub const MAX_MASK_PIXELS: u64 = 100_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaskBitmap {
    width: u32,
    height: u32,
    coverage: Vec<u8>,
}

impl MaskBitmap {
    pub fn empty(width: u32, height: u32) -> Result<Self, AppError> {
        let length = checked_length(width, height)?;
        Ok(Self {
            width,
            height,
            coverage: vec![0; length],
        })
    }

    pub fn full(width: u32, height: u32) -> Result<Self, AppError> {
        let length = checked_length(width, height)?;
        Ok(Self {
            width,
            height,
            coverage: vec![u8::MAX; length],
        })
    }

    pub fn from_coverage(width: u32, height: u32, coverage: Vec<u8>) -> Result<Self, AppError> {
        let expected = checked_length(width, height)?;
        if coverage.len() != expected {
            return Err(AppError::InvalidMask(format!(
                "coverage length {} does not match {} x {} dimensions",
                coverage.len(),
                width,
                height
            )));
        }
        Ok(Self {
            width,
            height,
            coverage,
        })
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    pub fn coverage(&self) -> &[u8] {
        &self.coverage
    }

    pub fn coverage_mut(&mut self) -> &mut [u8] {
        &mut self.coverage
    }

    pub fn get(&self, x: u32, y: u32) -> u8 {
        self.coverage[(y as usize * self.width as usize) + x as usize]
    }

    pub fn set(&mut self, x: u32, y: u32, value: u8) {
        let index = y as usize * self.width as usize + x as usize;
        self.coverage[index] = value;
    }

    pub fn resample_to(&self, width: u32, height: u32) -> Result<Self, AppError> {
        if self.width == width && self.height == height {
            return Ok(self.clone());
        }
        checked_length(width, height)?;
        let source_ratio = self.width as f64 / self.height as f64;
        let target_ratio = width as f64 / height as f64;
        if (source_ratio - target_ratio).abs() > 0.002 {
            return Err(AppError::MaskDimensionMismatch {
                mask_width: self.width,
                mask_height: self.height,
                image_width: width,
                image_height: height,
            });
        }

        let mut output = Self::empty(width, height)?;
        let scale_x = self.width as f64 / width as f64;
        let scale_y = self.height as f64 / height as f64;
        for y in 0..height {
            let source_y = ((y as f64 + 0.5) * scale_y - 0.5).clamp(0.0, self.height as f64 - 1.0);
            let y0 = source_y.floor() as u32;
            let y1 = (y0 + 1).min(self.height - 1);
            let fy = source_y - y0 as f64;
            for x in 0..width {
                let source_x =
                    ((x as f64 + 0.5) * scale_x - 0.5).clamp(0.0, self.width as f64 - 1.0);
                let x0 = source_x.floor() as u32;
                let x1 = (x0 + 1).min(self.width - 1);
                let fx = source_x - x0 as f64;
                let top = self.get(x0, y0) as f64 * (1.0 - fx) + self.get(x1, y0) as f64 * fx;
                let bottom = self.get(x0, y1) as f64 * (1.0 - fx) + self.get(x1, y1) as f64 * fx;
                output.set(x, y, (top * (1.0 - fy) + bottom * fy).round() as u8);
            }
        }
        Ok(output)
    }
}

pub fn checked_length(width: u32, height: u32) -> Result<usize, AppError> {
    if width == 0 || height == 0 {
        return Err(AppError::InvalidMask(
            "mask dimensions must be greater than zero".into(),
        ));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| AppError::InvalidMask("mask dimensions overflow".into()))?;
    if pixels > MAX_MASK_PIXELS {
        return Err(AppError::MaskTooLarge {
            pixels,
            limit: MAX_MASK_PIXELS,
        });
    }
    usize::try_from(pixels).map_err(|_| AppError::OutOfMemoryRisk)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_dimensions_and_coverage_length() {
        assert!(MaskBitmap::empty(0, 2).is_err());
        assert!(MaskBitmap::from_coverage(2, 2, vec![0; 3]).is_err());
        assert!(MaskBitmap::empty(2, 2).is_ok());
    }

    #[test]
    fn resampling_preserves_corner_coverage() {
        let mask = MaskBitmap::from_coverage(2, 2, vec![0, 255, 255, 0]).unwrap();
        let resized = mask.resample_to(4, 4).unwrap();
        assert_eq!(resized.get(0, 0), 0);
        assert_eq!(resized.get(3, 0), 255);
        assert_eq!(resized.get(0, 3), 255);
        assert_eq!(resized.get(3, 3), 0);
    }

    #[test]
    fn resampling_rejects_changed_aspect_ratio() {
        let mask = MaskBitmap::empty(4, 2).unwrap();
        assert!(matches!(
            mask.resample_to(2, 2),
            Err(AppError::MaskDimensionMismatch { .. })
        ));
    }
}
