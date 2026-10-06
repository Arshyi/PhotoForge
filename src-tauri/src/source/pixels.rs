//! Decoded rows, in the layouts the `image` crate would have produced.
//!
//! A region or reduced copy has to come out of the same colour pipeline as a
//! whole-file open, so the pixels are delivered as the same `DynamicImage`
//! variants a full decode returns, and the same conversion to linear float runs
//! over them. Doing the conversion differently for streamed sources would give
//! the region of a file different colours from the same area of a full open,
//! which would make "the region matches the full render" untrue before any
//! editing began.
use crate::error::AppError;
use image::{DynamicImage, ImageBuffer};

/// How the samples of one decoded row are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowLayout {
    Gray8,
    GrayAlpha8,
    Rgb8,
    Rgba8,
    Gray16,
    GrayAlpha16,
    Rgb16,
    Rgba16,
}

impl RowLayout {
    pub fn channels(self) -> usize {
        match self {
            Self::Gray8 | Self::Gray16 => 1,
            Self::GrayAlpha8 | Self::GrayAlpha16 => 2,
            Self::Rgb8 | Self::Rgb16 => 3,
            Self::Rgba8 | Self::Rgba16 => 4,
        }
    }

    pub fn is_16_bit(self) -> bool {
        matches!(
            self,
            Self::Gray16 | Self::GrayAlpha16 | Self::Rgb16 | Self::Rgba16
        )
    }

    pub fn bytes_per_pixel(self) -> usize {
        self.channels() * if self.is_16_bit() { 2 } else { 1 }
    }

    pub fn has_alpha(self) -> bool {
        matches!(
            self,
            Self::GrayAlpha8 | Self::Rgba8 | Self::GrayAlpha16 | Self::Rgba16
        )
    }
}

/// A growing region image, row by row.
///
/// The buffer is reserved up front with `try_reserve_exact`, so a request that
/// the admission planner let through but the allocator cannot honour is an error
/// the user can read, not an abort that closes the application.
pub struct RegionBuffer {
    layout: RowLayout,
    width: u32,
    height: u32,
    rows: u32,
    bytes8: Vec<u8>,
    samples16: Vec<u16>,
}

impl RegionBuffer {
    pub fn new(layout: RowLayout, width: u32, height: u32) -> Result<Self, AppError> {
        let samples = (width as usize)
            .checked_mul(height as usize)
            .and_then(|pixels| pixels.checked_mul(layout.channels()))
            .ok_or(AppError::OutOfMemoryRisk)?;
        let (mut bytes8, mut samples16) = (Vec::new(), Vec::new());
        if layout.is_16_bit() {
            samples16
                .try_reserve_exact(samples)
                .map_err(|_| AppError::OutOfMemoryRisk)?;
        } else {
            bytes8
                .try_reserve_exact(samples)
                .map_err(|_| AppError::OutOfMemoryRisk)?;
        }
        Ok(Self {
            layout,
            width,
            height,
            rows: 0,
            bytes8,
            samples16,
        })
    }

    /// Appends `self.width` pixels of `row`, starting at pixel `x`.
    pub fn push(&mut self, row: &[u8], x: u32) -> Result<(), AppError> {
        if self.rows >= self.height {
            return Err(AppError::DecodeFailure);
        }
        let bpp = self.layout.bytes_per_pixel();
        let start = x as usize * bpp;
        let end = start + self.width as usize * bpp;
        let slice = row.get(start..end).ok_or(AppError::CorruptImage)?;
        if self.layout.is_16_bit() {
            self.samples16.extend(
                slice
                    .chunks_exact(2)
                    .map(|pair| u16::from_be_bytes([pair[0], pair[1]])),
            );
        } else {
            self.bytes8.extend_from_slice(slice);
        }
        self.rows += 1;
        Ok(())
    }

    pub fn is_complete(&self) -> bool {
        self.rows == self.height
    }

    pub fn finish(self) -> Result<DynamicImage, AppError> {
        if !self.is_complete() {
            return Err(AppError::CorruptImage);
        }
        let (w, h) = (self.width, self.height);
        let image = match self.layout {
            RowLayout::Gray8 => {
                ImageBuffer::from_raw(w, h, self.bytes8).map(DynamicImage::ImageLuma8)
            }
            RowLayout::GrayAlpha8 => {
                ImageBuffer::from_raw(w, h, self.bytes8).map(DynamicImage::ImageLumaA8)
            }
            RowLayout::Rgb8 => {
                ImageBuffer::from_raw(w, h, self.bytes8).map(DynamicImage::ImageRgb8)
            }
            RowLayout::Rgba8 => {
                ImageBuffer::from_raw(w, h, self.bytes8).map(DynamicImage::ImageRgba8)
            }
            RowLayout::Gray16 => {
                ImageBuffer::from_raw(w, h, self.samples16).map(DynamicImage::ImageLuma16)
            }
            RowLayout::GrayAlpha16 => {
                ImageBuffer::from_raw(w, h, self.samples16).map(DynamicImage::ImageLumaA16)
            }
            RowLayout::Rgb16 => {
                ImageBuffer::from_raw(w, h, self.samples16).map(DynamicImage::ImageRgb16)
            }
            RowLayout::Rgba16 => {
                ImageBuffer::from_raw(w, h, self.samples16).map(DynamicImage::ImageRgba16)
            }
        };
        image.ok_or(AppError::CorruptImage)
    }
}

/// One decoded row as encoded RGBA `f32` in `0..=1`, matching what
/// `DynamicImage::to_rgba32f` gives for the same samples.
pub fn row_to_encoded_rgba(layout: RowLayout, row: &[u8], width: usize, out: &mut [f32]) {
    let scale = if layout.is_16_bit() { 65535.0 } else { 255.0 };
    let sample = |index: usize| -> f32 {
        if layout.is_16_bit() {
            f32::from(u16::from_be_bytes([row[index * 2], row[index * 2 + 1]]))
        } else {
            f32::from(row[index])
        }
    };
    let channels = layout.channels();
    for x in 0..width {
        let base = x * channels;
        let (r, g, b, a) = match layout.channels() {
            1 => {
                let v = sample(base);
                (v, v, v, scale)
            }
            2 => {
                let v = sample(base);
                (v, v, v, sample(base + 1))
            }
            3 => (sample(base), sample(base + 1), sample(base + 2), scale),
            _ => (
                sample(base),
                sample(base + 1),
                sample(base + 2),
                sample(base + 3),
            ),
        };
        let o = &mut out[x * 4..x * 4 + 4];
        o[0] = r / scale;
        o[1] = g / scale;
        o[2] = b / scale;
        o[3] = a / scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_report_their_geometry() {
        assert_eq!(RowLayout::Rgba16.bytes_per_pixel(), 8);
        assert_eq!(RowLayout::Gray8.bytes_per_pixel(), 1);
        assert_eq!(RowLayout::GrayAlpha16.channels(), 2);
        assert!(RowLayout::Rgba8.has_alpha() && !RowLayout::Rgb8.has_alpha());
    }

    #[test]
    fn a_region_buffer_assembles_rows_and_refuses_extras() {
        let mut buffer = RegionBuffer::new(RowLayout::Rgb8, 2, 2).unwrap();
        // A 4-pixel-wide source row; take pixels 1 and 2.
        let row: Vec<u8> = (0..12).collect();
        buffer.push(&row, 1).unwrap();
        buffer.push(&row, 1).unwrap();
        assert!(
            buffer.push(&row, 1).is_err(),
            "a third row into a two-row region"
        );
        let image = buffer.finish().unwrap().to_rgb8();
        assert_eq!(image.as_raw()[..6], [3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn a_region_outside_the_row_is_a_corrupt_image_not_a_panic() {
        let mut buffer = RegionBuffer::new(RowLayout::Gray8, 4, 1).unwrap();
        assert!(buffer.push(&[1, 2, 3], 0).is_err());
    }

    #[test]
    fn an_incomplete_region_cannot_be_finished() {
        let buffer = RegionBuffer::new(RowLayout::Gray8, 2, 2).unwrap();
        assert!(buffer.finish().is_err());
    }

    #[test]
    fn sixteen_bit_samples_are_read_big_endian() {
        let mut buffer = RegionBuffer::new(RowLayout::Gray16, 2, 1).unwrap();
        buffer.push(&[0x12, 0x34, 0xAB, 0xCD], 0).unwrap();
        let image = buffer.finish().unwrap().to_luma16();
        assert_eq!(image.as_raw(), &vec![0x1234, 0xABCD]);
    }

    /// The streamed conversion has to agree with the library's own.
    #[test]
    fn row_conversion_matches_to_rgba32f() {
        let cases: Vec<(RowLayout, DynamicImage)> = vec![
            (
                RowLayout::Rgb8,
                DynamicImage::ImageRgb8(
                    ImageBuffer::from_raw(2, 1, vec![10, 20, 30, 250, 128, 0]).unwrap(),
                ),
            ),
            (
                RowLayout::GrayAlpha16,
                DynamicImage::ImageLumaA16(
                    ImageBuffer::from_raw(2, 1, vec![1000, 65535, 40000, 20000]).unwrap(),
                ),
            ),
            (
                RowLayout::Rgba16,
                DynamicImage::ImageRgba16(
                    ImageBuffer::from_raw(1, 1, vec![65535, 0, 32768, 500]).unwrap(),
                ),
            ),
            (
                RowLayout::Gray8,
                DynamicImage::ImageLuma8(ImageBuffer::from_raw(3, 1, vec![0, 128, 255]).unwrap()),
            ),
        ];
        for (layout, image) in cases {
            // Re-encode the image's samples into the byte layout a PNG row has.
            let raw: Vec<u8> = match &image {
                DynamicImage::ImageRgb8(i) => i.as_raw().clone(),
                DynamicImage::ImageLuma8(i) => i.as_raw().clone(),
                DynamicImage::ImageLumaA16(i) => {
                    i.as_raw().iter().flat_map(|v| v.to_be_bytes()).collect()
                }
                DynamicImage::ImageRgba16(i) => {
                    i.as_raw().iter().flat_map(|v| v.to_be_bytes()).collect()
                }
                _ => unreachable!(),
            };
            let width = image.width() as usize;
            let mut out = vec![0.0f32; width * 4];
            row_to_encoded_rgba(layout, &raw, width, &mut out);
            let expected = image.to_rgba32f();
            for (got, want) in out.iter().zip(expected.as_raw()) {
                assert!((got - want).abs() < 1e-6, "{layout:?}: {got} vs {want}");
            }
        }
    }
}
