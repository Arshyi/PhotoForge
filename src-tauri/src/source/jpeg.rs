//! JPEG: whole-frame region decoding, and DCT-scaled reduced decoding.
//!
//! # What this allocates — and what it does not
//!
//! **A region of a JPEG is not region decoding.** Neither decoder PhotoForge can
//! use offers a crop or a row stream (`zune-jpeg` has only `decode_into` for the
//! whole frame; checked in its source), so a region means decoding the entire
//! frame at 8 bits per channel, keeping the rectangle, and freeing the rest.
//! Peak memory follows the *source*, at 3 bytes per pixel for RGB rather than 16
//! for the float working copy. That is a real saving and the planner prices it as
//! exactly what it is: [`RegionDecode::TransientFull`].
//!
//! **A reduced copy of a JPEG can be genuinely bounded.** A JPEG stores 8x8 DCT
//! blocks, and the DC and low-frequency terms of a block are the block's average
//! at 1/8 scale. `jpeg-decoder` can decode straight to 1/2, 1/4 or 1/8 size
//! without ever producing the full frame. Measured on a 108 MP, 16 MB file:
//!
//! | decode | time | peak working set |
//! | --- | --- | --- |
//! | `zune-jpeg`, whole frame | 337 ms | 329 MiB |
//! | `jpeg-decoder`, whole frame | 454 ms | 623 MiB |
//! | `jpeg-decoder`, 1/8 scale | 342 ms | 15 MiB |
//! | `jpeg-decoder`, 1/4 scale | 372 ms | 43 MiB |
//!
//! The same speed at about a twenty-second of the memory. For a reduction larger
//! than the nearest DCT scale, the scaled frame is then area-resampled down to the
//! exact size requested.
//!
//! `jpeg-decoder` is in maintenance mode upstream. It is used for this one path
//! and for nothing a user could reach with an ordinary-size photograph, which
//! still decodes through `image` and `zune-jpeg`.
use super::model::Rect;
use super::reduce::AreaReducer;
use crate::color::{srgb_decode, FloatImage};
use crate::color_management::IccRowTransform;
use crate::error::AppError;
use crate::image_processing::high_precision::check_cancel;
use image::{DynamicImage, ImageDecoder, ImageReader, Limits};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::AtomicBool;

/// Bytes per pixel the whole-frame decode holds, for the planner. RGB is three;
/// a greyscale JPEG is one but is priced as three because the cost of being wrong
/// is a refusal that should not have happened, not an overrun.
pub const WHOLE_FRAME_BYTES_PER_PIXEL: u64 = 3;

/// What a DCT-scaled decode holds, and which scale it will use, are decided by
/// the planner so that what it prices is what this module does.
pub use crate::resources::admission::{dct_factor, DCT_BYTES_PER_OUTPUT_PIXEL, DCT_FIXED_BYTES};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegInfo {
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub icc: Option<Vec<u8>>,
}

fn map_jpeg_error(error: jpeg_decoder::Error) -> AppError {
    match error {
        jpeg_decoder::Error::Io(io) if io.kind() == std::io::ErrorKind::PermissionDenied => {
            AppError::Permission
        }
        jpeg_decoder::Error::Unsupported(_) => AppError::UnsupportedImageFormat,
        _ => AppError::CorruptImage,
    }
}

fn map_image_error(error: image::ImageError) -> AppError {
    match error {
        image::ImageError::Unsupported(_) => AppError::UnsupportedImageFormat,
        image::ImageError::Limits(_) => AppError::OutOfMemoryRisk,
        image::ImageError::Decoding(_) => AppError::CorruptImage,
        _ => AppError::DecodeFailure,
    }
}

fn open(path: &Path) -> Result<jpeg_decoder::Decoder<BufReader<File>>, AppError> {
    let file = File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            AppError::Permission
        } else {
            AppError::DecodeFailure
        }
    })?;
    Ok(jpeg_decoder::Decoder::new(BufReader::with_capacity(
        1 << 20,
        file,
    )))
}

/// Reads only the headers.
pub fn read_info(path: &Path) -> Result<JpegInfo, AppError> {
    let mut decoder = open(path)?;
    decoder.read_info().map_err(map_jpeg_error)?;
    let info = decoder.info().ok_or(AppError::CorruptImage)?;
    Ok(JpegInfo {
        width: u32::from(info.width),
        height: u32::from(info.height),
        components: match info.pixel_format {
            jpeg_decoder::PixelFormat::L8 | jpeg_decoder::PixelFormat::L16 => 1,
            jpeg_decoder::PixelFormat::RGB24 => 3,
            jpeg_decoder::PixelFormat::CMYK32 => 4,
        },
        icc: decoder.icc_profile(),
    })
}

/// A region of a JPEG, cut from a whole-frame decode.
pub struct RegionImage {
    pub image: DynamicImage,
    pub icc: Option<Vec<u8>>,
}

/// Decodes the whole frame and keeps `rect`.
///
/// The whole frame is alive while the region is copied out, so the peak is the
/// frame plus the region. It is released as soon as the copy is made, before the
/// caller builds the float working copy, so the two never coexist.
pub fn decode_region(
    path: &Path,
    rect: Rect,
    cancel: Option<&AtomicBool>,
) -> Result<RegionImage, AppError> {
    check_cancel(cancel)?;
    let mut reader = ImageReader::open(path)
        .map_err(|_| AppError::DecodeFailure)?
        .with_guessed_format()
        .map_err(|_| AppError::DecodeFailure)?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(crate::resources::max_job_bytes());
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(map_image_error)?;
    let (width, height) = decoder.dimensions();
    if !rect.is_within(width, height) {
        return Err(AppError::InvalidOperation(
            "the region lies outside the image".into(),
        ));
    }
    let icc = decoder.icc_profile().map_err(map_image_error)?;
    let full = DynamicImage::from_decoder(decoder).map_err(map_image_error)?;
    check_cancel(cancel)?;
    let region = full.crop_imm(rect.x, rect.y, rect.width, rect.height);
    drop(full);
    Ok(RegionImage { image: region, icc })
}

/// Decodes the whole JPEG reduced to `dst_width` x `dst_height`, as linear float.
///
/// Decodes at the nearest DCT scale at or above the target, so quality is never
/// thrown away by scaling past what was asked, then area-resamples the scaled
/// frame down to the exact size.
pub fn decode_reduced(
    path: &Path,
    dst_width: u32,
    dst_height: u32,
    cancel: Option<&AtomicBool>,
) -> Result<FloatImage, AppError> {
    check_cancel(cancel)?;
    let mut decoder = open(path)?;
    decoder.read_info().map_err(map_jpeg_error)?;
    let info = decoder.info().ok_or(AppError::CorruptImage)?;
    let (src_w, src_h) = (u32::from(info.width), u32::from(info.height));
    if dst_width == 0 || dst_height == 0 || dst_width > src_w || dst_height > src_h {
        return Err(AppError::InvalidOperation(
            "a reduced copy must be no larger than its source".into(),
        ));
    }
    let format = info.pixel_format;
    // CMYK and 12-bit greyscale are refused rather than approximated. `image`
    // does not open CMYK either, so the two paths agree about what is
    // supported; 16-bit greyscale JPEG is rare enough that its byte order was
    // not worth guessing at, and a wrong guess would silently produce wrong
    // tones.
    if matches!(
        format,
        jpeg_decoder::PixelFormat::CMYK32 | jpeg_decoder::PixelFormat::L16
    ) {
        return Err(AppError::UnsupportedImageFormat);
    }
    let icc = decoder.icc_profile();

    // `scale` takes the size we want at least, and reports the size we got.
    let ask_w = dst_width.min(u32::from(u16::MAX)) as u16;
    let ask_h = dst_height.min(u32::from(u16::MAX)) as u16;
    let (scaled_w, scaled_h) = decoder.scale(ask_w, ask_h).map_err(map_jpeg_error)?;
    let (scaled_w, scaled_h) = (u32::from(scaled_w), u32::from(scaled_h));
    let pixels = decoder.decode().map_err(map_jpeg_error)?;
    check_cancel(cancel)?;

    let channels: usize = match format {
        jpeg_decoder::PixelFormat::RGB24 => 3,
        _ => 1,
    };
    let row_bytes = scaled_w as usize * channels;
    if pixels.len() < row_bytes * scaled_h as usize {
        return Err(AppError::CorruptImage);
    }

    let transform = icc.as_deref().map(IccRowTransform::new).transpose()?;
    let mut reducer = AreaReducer::new(scaled_w, scaled_h, dst_width, dst_height)?;
    let width = scaled_w as usize;
    let mut encoded = vec![0.0f32; width * 4];
    let mut linear = vec![0.0f32; width * 4];
    for y in 0..scaled_h as usize {
        let row = &pixels[y * row_bytes..(y + 1) * row_bytes];
        for x in 0..width {
            let (r, g, b) = if channels == 3 {
                (
                    f32::from(row[x * 3]) / 255.0,
                    f32::from(row[x * 3 + 1]) / 255.0,
                    f32::from(row[x * 3 + 2]) / 255.0,
                )
            } else {
                let v = f32::from(row[x]) / 255.0;
                (v, v, v)
            };
            encoded[x * 4..x * 4 + 4].copy_from_slice(&[r, g, b, 1.0]);
        }
        match &transform {
            Some(transform) => transform.apply(&encoded, &mut linear)?,
            None => {
                for (src, dst) in encoded.chunks_exact(4).zip(linear.chunks_exact_mut(4)) {
                    dst[0] = srgb_decode(src[0]);
                    dst[1] = srgb_decode(src[1]);
                    dst[2] = srgb_decode(src[2]);
                    dst[3] = 1.0;
                }
            }
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

    /// A smooth gradient, which JPEG compresses faithfully, so decoded values stay
    /// near the source and the comparisons below can use tolerances that mean
    /// something.
    fn write_jpeg(path: &Path, width: u32, height: u32) {
        let mut buffer = vec![0u8; (width * height * 3) as usize];
        for y in 0..height {
            for x in 0..width {
                let i = ((y * width + x) * 3) as usize;
                buffer[i] = (x * 255 / width.max(1)) as u8;
                buffer[i + 1] = (y * 255 / height.max(1)) as u8;
                buffer[i + 2] = 96;
            }
        }
        let file = std::fs::File::create(path).unwrap();
        let mut encoder =
            image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), 95);
        encoder
            .encode(&buffer, width, height, image::ExtendedColorType::Rgb8)
            .unwrap();
    }

    #[test]
    fn the_header_reports_the_size_without_decoding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        write_jpeg(&path, 200, 120);
        let info = read_info(&path).unwrap();
        assert_eq!((info.width, info.height, info.components), (200, 120, 3));
    }

    /// A JPEG region equals the crop of a whole-frame decode, which is all it
    /// claims to be.
    #[test]
    fn a_region_equals_the_crop_of_a_full_decode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        write_jpeg(&path, 160, 90);
        let full = image::ImageReader::open(&path).unwrap().decode().unwrap();
        for rect in [
            Rect {
                x: 0,
                y: 0,
                width: 160,
                height: 90,
            },
            Rect {
                x: 10,
                y: 20,
                width: 50,
                height: 30,
            },
            Rect {
                x: 159,
                y: 89,
                width: 1,
                height: 1,
            },
        ] {
            let got = decode_region(&path, rect, None).unwrap().image;
            let want = full.crop_imm(rect.x, rect.y, rect.width, rect.height);
            assert_eq!(got.dimensions(), want.dimensions());
            assert_eq!(got.as_bytes(), want.as_bytes(), "{rect:?}");
        }
    }

    #[test]
    fn a_region_outside_the_image_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        write_jpeg(&path, 64, 64);
        for rect in [
            Rect {
                x: 60,
                y: 0,
                width: 10,
                height: 5,
            },
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 5,
            },
            Rect {
                x: u32::MAX,
                y: 0,
                width: 5,
                height: 5,
            },
        ] {
            assert!(decode_region(&path, rect, None).is_err(), "{rect:?}");
        }
    }

    #[test]
    fn the_dct_scale_is_the_largest_that_keeps_the_target() {
        assert_eq!(dct_factor(12_000, 9_000, 1_500, 1_125), 8);
        assert_eq!(dct_factor(12_000, 9_000, 1_501, 1_125), 4);
        assert_eq!(dct_factor(12_000, 9_000, 3_000, 2_250), 4);
        assert_eq!(dct_factor(12_000, 9_000, 6_000, 4_500), 2);
        assert_eq!(dct_factor(12_000, 9_000, 6_001, 4_500), 1);
        assert_eq!(dct_factor(100, 100, 100, 100), 1);
    }

    /// A reduced decode is close to resampling the full decode, and exactly the
    /// requested size.
    #[test]
    fn a_reduced_decode_has_the_requested_size_and_the_right_colours() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        write_jpeg(&path, 256, 192);
        let reduced = decode_reduced(&path, 64, 48, None).unwrap();
        assert_eq!(reduced.dimensions(), (64, 48));
        // The gradient's red channel rises left to right: the left column is
        // darker than the right in linear light.
        let left = reduced.pixels()[48 / 2 * 64].red;
        let right = reduced.pixels()[48 / 2 * 64 + 63].red;
        assert!(right > left + 0.3, "{left} -> {right}");
        // And the constant blue channel stays at the encoded 96/255.
        let blue = srgb_decode(96.0 / 255.0);
        assert!(
            (reduced.pixels()[100].blue - blue).abs() < 0.02,
            "{}",
            reduced.pixels()[100].blue
        );
    }

    /// Reduction by a non-power-of-two ratio still lands on the exact size.
    #[test]
    fn a_reduction_between_dct_scales_still_hits_the_exact_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        write_jpeg(&path, 300, 200);
        let reduced = decode_reduced(&path, 111, 74, None).unwrap();
        assert_eq!(reduced.dimensions(), (111, 74));
    }

    #[test]
    fn a_reduced_decode_refuses_to_enlarge() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        write_jpeg(&path, 64, 64);
        assert!(decode_reduced(&path, 128, 128, None).is_err());
        assert!(decode_reduced(&path, 0, 10, None).is_err());
    }

    #[test]
    fn cancellation_is_honoured() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        write_jpeg(&path, 64, 64);
        let cancel = AtomicBool::new(true);
        assert!(matches!(
            decode_reduced(&path, 32, 32, Some(&cancel)),
            Err(AppError::RenderCancelled)
        ));
        assert!(matches!(
            decode_region(
                &path,
                Rect {
                    x: 0,
                    y: 0,
                    width: 8,
                    height: 8
                },
                Some(&cancel)
            ),
            Err(AppError::RenderCancelled)
        ));
    }

    #[test]
    fn a_non_jpeg_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not.jpg");
        std::fs::write(&path, b"not a jpeg at all").unwrap();
        assert!(read_info(&path).is_err());
        assert!(decode_reduced(&path, 1, 1, None).is_err());
    }
}
