//! WebP: whole-frame region and reduced decoding.
//!
//! `image-webp` decodes the entire frame into memory; there is no crop, no row
//! stream and no scaled decode. A region or reduced copy therefore holds the whole
//! frame while it is produced, and the planner prices it as
//! [`RegionDecode::TransientFull`] / [`ReducedDecode::TransientFull`] at
//! [`whole_frame_bytes_per_pixel`].
//!
//! The format itself bounds this: WebP dimensions are limited to 16,383 pixels a
//! side, so the whole-frame transient cannot exceed about 1 GiB however hostile
//! the header.
use super::model::Rect;
use super::reduce::AreaReducer;
use crate::color::{srgb_decode, FloatImage};
use crate::color_management::IccRowTransform;
use crate::error::AppError;
use crate::image_processing::high_precision::check_cancel;
use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// Bytes per pixel the whole-frame decode holds at its peak, measured.
///
/// With an alpha channel the decoder fills one RGBA frame: four bytes. **Without**
/// one it still decodes to RGBA and then converts to RGB while the RGBA frame is
/// alive, so both coexist: seven. (A 96 MP RGB file peaked at 645 MiB, 7.05 bytes a
/// pixel with the file buffer; the same picture with alpha at 371 MiB, 4.1.) The
/// first version of the planner priced RGB at three, which was the size of the
/// *result* and under-priced the decode by more than half; `tests/estimate_accuracy.rs`
/// now fails if a decoder holds more than the planner says it does.
pub const fn whole_frame_bytes_per_pixel(has_alpha: bool) -> u64 {
    if has_alpha {
        4
    } else {
        7
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebpInfo {
    pub width: u32,
    pub height: u32,
    pub has_alpha: bool,
    pub icc: Option<Vec<u8>>,
}

fn map_image_error(error: image::ImageError) -> AppError {
    match error {
        image::ImageError::Unsupported(_) => AppError::UnsupportedImageFormat,
        image::ImageError::Limits(_) => AppError::OutOfMemoryRisk,
        image::ImageError::Decoding(_) => AppError::CorruptImage,
        image::ImageError::IoError(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            AppError::Permission
        }
        _ => AppError::DecodeFailure,
    }
}

fn decoder(path: &Path) -> Result<Box<dyn ImageDecoder + '_>, AppError> {
    let mut reader = ImageReader::open(path)
        .map_err(|_| AppError::DecodeFailure)?
        .with_guessed_format()
        .map_err(|_| AppError::DecodeFailure)?;
    if reader.format() != Some(image::ImageFormat::WebP) {
        return Err(AppError::UnsupportedImageFormat);
    }
    let mut limits = Limits::default();
    limits.max_alloc = Some(crate::resources::max_job_bytes());
    reader.limits(limits);
    Ok(Box::new(reader.into_decoder().map_err(map_image_error)?))
}

/// Reads only the header.
pub fn read_info(path: &Path) -> Result<WebpInfo, AppError> {
    let mut decoder = decoder(path)?;
    let (width, height) = decoder.dimensions();
    let has_alpha = decoder.color_type().has_alpha();
    let icc = decoder.icc_profile().map_err(map_image_error)?;
    Ok(WebpInfo {
        width,
        height,
        has_alpha,
        icc,
    })
}

fn decode_whole(path: &Path) -> Result<(DynamicImage, Option<Vec<u8>>), AppError> {
    let mut decoder = decoder(path)?;
    let icc = decoder.icc_profile().map_err(map_image_error)?;
    let image = DynamicImage::from_decoder(decoder).map_err(map_image_error)?;
    Ok((image, icc))
}

pub struct RegionImage {
    pub image: DynamicImage,
    pub icc: Option<Vec<u8>>,
}

/// Decodes the whole frame and keeps `rect`, releasing the frame afterwards.
pub fn decode_region(
    path: &Path,
    rect: Rect,
    cancel: Option<&AtomicBool>,
) -> Result<RegionImage, AppError> {
    check_cancel(cancel)?;
    let (full, icc) = decode_whole(path)?;
    if !rect.is_within(full.width(), full.height()) {
        return Err(AppError::InvalidOperation(
            "the region lies outside the image".into(),
        ));
    }
    check_cancel(cancel)?;
    let region = full.crop_imm(rect.x, rect.y, rect.width, rect.height);
    drop(full);
    Ok(RegionImage { image: region, icc })
}

/// Decodes the whole WebP reduced to `dst_width` x `dst_height`, as linear float.
pub fn decode_reduced(
    path: &Path,
    dst_width: u32,
    dst_height: u32,
    cancel: Option<&AtomicBool>,
) -> Result<FloatImage, AppError> {
    check_cancel(cancel)?;
    let (full, icc) = decode_whole(path)?;
    let (width, height) = (full.width(), full.height());
    let mut reducer = AreaReducer::new(width, height, dst_width, dst_height)?;
    let transform = icc.as_deref().map(IccRowTransform::new).transpose()?;
    // WebP is always 8-bit, delivered as L8, La8, Rgb8 or Rgba8.
    let (channels, bytes): (usize, &[u8]) = match &full {
        DynamicImage::ImageLuma8(i) => (1, i.as_raw()),
        DynamicImage::ImageLumaA8(i) => (2, i.as_raw()),
        DynamicImage::ImageRgb8(i) => (3, i.as_raw()),
        DynamicImage::ImageRgba8(i) => (4, i.as_raw()),
        _ => return Err(AppError::UnsupportedImageFormat),
    };
    let w = width as usize;
    let mut encoded = vec![0.0f32; w * 4];
    let mut linear = vec![0.0f32; w * 4];
    for y in 0..height as usize {
        let row = &bytes[y * w * channels..(y + 1) * w * channels];
        for x in 0..w {
            let base = x * channels;
            let (r, g, b, a) = match channels {
                1 => (row[base], row[base], row[base], 255),
                2 => (row[base], row[base], row[base], row[base + 1]),
                3 => (row[base], row[base + 1], row[base + 2], 255),
                _ => (row[base], row[base + 1], row[base + 2], row[base + 3]),
            };
            encoded[x * 4..x * 4 + 4].copy_from_slice(&[
                f32::from(r) / 255.0,
                f32::from(g) / 255.0,
                f32::from(b) / 255.0,
                f32::from(a) / 255.0,
            ]);
        }
        match &transform {
            Some(transform) => transform.apply(&encoded, &mut linear)?,
            None => {
                for (src, dst) in encoded.chunks_exact(4).zip(linear.chunks_exact_mut(4)) {
                    dst[0] = srgb_decode(src[0]);
                    dst[1] = srgb_decode(src[1]);
                    dst[2] = srgb_decode(src[2]);
                    dst[3] = src[3];
                }
            }
        }
        for pixel in linear.chunks_exact_mut(4) {
            let alpha = pixel[3];
            pixel[0] *= alpha;
            pixel[1] *= alpha;
            pixel[2] *= alpha;
        }
        reducer.push_row(&linear)?;
        if y.is_multiple_of(64) {
            check_cancel(cancel)?;
        }
    }
    reducer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;

    fn write_webp(path: &Path, width: u32, height: u32, alpha: bool) {
        let file = std::fs::File::create(path).unwrap();
        let encoder = image::codecs::webp::WebPEncoder::new_lossless(std::io::BufWriter::new(file));
        if alpha {
            let mut buffer = vec![0u8; (width * height * 4) as usize];
            for y in 0..height {
                for x in 0..width {
                    let i = ((y * width + x) * 4) as usize;
                    buffer[i] = (x * 7 % 256) as u8;
                    buffer[i + 1] = (y * 11 % 256) as u8;
                    buffer[i + 2] = 90;
                    buffer[i + 3] = 200;
                }
            }
            encoder
                .encode(&buffer, width, height, image::ExtendedColorType::Rgba8)
                .unwrap();
        } else {
            let mut buffer = vec![0u8; (width * height * 3) as usize];
            for y in 0..height {
                for x in 0..width {
                    let i = ((y * width + x) * 3) as usize;
                    buffer[i] = (x * 7 % 256) as u8;
                    buffer[i + 1] = (y * 11 % 256) as u8;
                    buffer[i + 2] = 90;
                }
            }
            encoder
                .encode(&buffer, width, height, image::ExtendedColorType::Rgb8)
                .unwrap();
        }
    }

    #[test]
    fn the_header_reports_size_and_alpha() {
        let dir = tempfile::tempdir().unwrap();
        let rgb = dir.path().join("rgb.webp");
        let rgba = dir.path().join("rgba.webp");
        write_webp(&rgb, 40, 30, false);
        write_webp(&rgba, 40, 30, true);
        let a = read_info(&rgb).unwrap();
        let b = read_info(&rgba).unwrap();
        assert_eq!((a.width, a.height, a.has_alpha), (40, 30, false));
        assert!(b.has_alpha);
    }

    /// Lossless WebP round-trips exactly, so a region must equal the crop of the
    /// image that was encoded.
    #[test]
    fn a_region_equals_the_crop_of_the_whole_frame() {
        let dir = tempfile::tempdir().unwrap();
        for (name, alpha) in [("rgb", false), ("rgba", true)] {
            let path = dir.path().join(format!("{name}.webp"));
            write_webp(&path, 64, 48, alpha);
            let full = image::ImageReader::open(&path).unwrap().decode().unwrap();
            let rect = Rect {
                x: 9,
                y: 4,
                width: 30,
                height: 20,
            };
            let got = decode_region(&path, rect, None).unwrap().image;
            let want = full.crop_imm(rect.x, rect.y, rect.width, rect.height);
            assert_eq!(got.dimensions(), want.dimensions());
            assert_eq!(got.as_bytes(), want.as_bytes(), "{name}");
        }
    }

    #[test]
    fn a_region_outside_the_frame_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.webp");
        write_webp(&path, 32, 32, false);
        assert!(decode_region(
            &path,
            Rect {
                x: 30,
                y: 0,
                width: 5,
                height: 5
            },
            None
        )
        .is_err());
        assert!(decode_region(
            &path,
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 5
            },
            None
        )
        .is_err());
    }

    #[test]
    fn a_reduced_decode_has_the_requested_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.webp");
        write_webp(&path, 80, 60, true);
        let reduced = decode_reduced(&path, 20, 15, None).unwrap();
        assert_eq!(reduced.dimensions(), (20, 15));
        // Alpha was 200/255 everywhere, so it survives reduction.
        assert!(reduced
            .pixels()
            .iter()
            .all(|p| (p.alpha - 200.0 / 255.0).abs() < 1e-4));
    }

    #[test]
    fn a_png_is_not_mistaken_for_a_webp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        image::RgbImage::new(4, 4).save(&path).unwrap();
        assert!(read_info(&path).is_err());
    }

    #[test]
    fn cancellation_is_honoured() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.webp");
        write_webp(&path, 32, 32, false);
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            decode_reduced(&path, 8, 8, Some(&cancel)),
            Err(AppError::RenderCancelled)
        ));
    }
}
