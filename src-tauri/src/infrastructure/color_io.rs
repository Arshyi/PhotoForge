//! Atomic, tagged color export. PNG is written row by row, without a full-frame
//! 8/16-bit intermediate. Metadata is limited to the chosen RGB ICC profile.
use crate::color::FloatImage;
use crate::color_management::{quantize8, ColorExportOptions};
use crate::domain::ExportProfile;
use crate::error::AppError;
use image::{ExtendedColorType, ImageEncoder};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

pub fn save_color_image(
    image: &FloatImage,
    original: &Path,
    output: &Path,
    profile: ExportProfile,
    options: ColorExportOptions,
    cancel: Option<&AtomicBool>,
) -> Result<PathBuf, AppError> {
    options.validate()?;
    image.validate()?;
    let path = super::image_io::validate_output_path(original, output)?;
    let format = super::image_io::output_format(&path)?;
    if options.bit_depth == 16 && format != image::ImageFormat::Png {
        return Err(AppError::ColorPipeline("16-bit export requires PNG".into()));
    }
    let parent = path.parent().ok_or(AppError::InvalidOutputPath)?;
    let temporary = tempfile::Builder::new()
        .prefix(".photoforge-color-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|_| AppError::ExportFailure)?;
    let icc = options.color_space.icc_bytes()?;
    {
        let writer = BufWriter::new(temporary.as_file());
        if format == image::ImageFormat::Png {
            let mut info = png::Info::with_size(image.width(), image.height());
            info.color_type = png::ColorType::Rgba;
            info.bit_depth = if options.bit_depth == 16 {
                png::BitDepth::Sixteen
            } else {
                png::BitDepth::Eight
            };
            info.icc_profile = Some(std::borrow::Cow::Owned(icc));
            let encoder =
                png::Encoder::with_info(writer, info).map_err(|_| AppError::ExportFailure)?;
            let mut writer = encoder
                .write_header()
                .map_err(|_| AppError::ExportFailure)?;
            let mut stream = writer
                .stream_writer()
                .map_err(|_| AppError::ExportFailure)?;
            let mut row =
                Vec::with_capacity(image.width() as usize * usize::from(options.bit_depth / 8) * 4);
            for (y, pixels) in image.pixels().chunks(image.width() as usize).enumerate() {
                crate::image_processing::high_precision::check_cancel(cancel)?;
                row.clear();
                for (x, p) in pixels.iter().enumerate() {
                    let rgb = options.color_space.from_working([p.red, p.green, p.blue]);
                    if options.bit_depth == 16 {
                        for c in [rgb[0], rgb[1], rgb[2], p.alpha] {
                            row.extend_from_slice(
                                &((c.clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes(),
                            );
                        }
                    } else {
                        for c in rgb {
                            row.push(quantize8(c, x as u32, y as u32, options.dither));
                        }
                        row.push(quantize8(p.alpha, x as u32, y as u32, false));
                    }
                }
                stream
                    .write_all(&row)
                    .map_err(|_| AppError::ExportFailure)?;
            }
            stream.finish().map_err(|_| AppError::ExportFailure)?;
            writer.finish().map_err(|_| AppError::ExportFailure)?;
        } else {
            let jpeg = format == image::ImageFormat::Jpeg;
            let channels = if jpeg { 3 } else { 4 };
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(image.pixels().len() * channels)
                .map_err(|_| AppError::OutOfMemoryRisk)?;
            for (y, row) in image.pixels().chunks(image.width() as usize).enumerate() {
                crate::image_processing::high_precision::check_cancel(cancel)?;
                for (x, p) in row.iter().enumerate() {
                    let mut linear = [p.red, p.green, p.blue];
                    if jpeg {
                        linear = linear.map(|c| c * p.alpha + 1.0 - p.alpha);
                    }
                    for c in options.color_space.from_working(linear) {
                        bytes.push(quantize8(c, x as u32, y as u32, options.dither));
                    }
                    if !jpeg {
                        bytes.push(quantize8(p.alpha, 0, 0, false));
                    }
                }
            }
            if jpeg {
                let quality = match profile {
                    ExportProfile::Web => 82,
                    ExportProfile::Print | ExportProfile::HighJpeg => 95,
                    _ => 90,
                };
                let mut encoder =
                    image::codecs::jpeg::JpegEncoder::new_with_quality(writer, quality);
                encoder
                    .set_icc_profile(icc)
                    .map_err(|_| AppError::ExportFailure)?;
                encoder
                    .write_image(
                        &bytes,
                        image.width(),
                        image.height(),
                        ExtendedColorType::Rgb8,
                    )
                    .map_err(|_| AppError::ExportFailure)?;
            } else {
                let mut encoder = image::codecs::webp::WebPEncoder::new_lossless(writer);
                encoder
                    .set_icc_profile(icc)
                    .map_err(|_| AppError::ExportFailure)?;
                encoder
                    .write_image(
                        &bytes,
                        image.width(),
                        image.height(),
                        ExtendedColorType::Rgba8,
                    )
                    .map_err(|_| AppError::ExportFailure)?;
            }
        }
    }
    crate::image_processing::high_precision::check_cancel(cancel)?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| AppError::ExportFailure)?;
    temporary
        .persist(&path)
        .map_err(|_| AppError::ExportFailure)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{srgb_decode, FloatRgba};
    use crate::color_management::RgbColorSpace;
    use image::ImageDecoder;
    #[test]
    fn png16_retains_precision_and_embeds_the_transformed_space() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.png");
        std::fs::write(&source, b"protected").unwrap();
        let image = FloatImage::blank(
            3,
            2,
            FloatRgba::new(srgb_decode(0.30001), 0.44444, 0.9999, 0.7),
        )
        .unwrap();
        for space in [
            RgbColorSpace::Srgb,
            RgbColorSpace::DisplayP3,
            RgbColorSpace::AdobeRgb,
        ] {
            let path = directory.path().join(format!("{space:?}.png"));
            save_color_image(
                &image,
                &source,
                &path,
                ExportProfile::Lossless,
                ColorExportOptions {
                    color_space: space,
                    ..Default::default()
                },
                None,
            )
            .unwrap();
            let mut decoder = image::ImageReader::open(&path)
                .unwrap()
                .into_decoder()
                .unwrap();
            let profile = decoder.icc_profile().unwrap().unwrap();
            assert_eq!(profile, space.icc_bytes().unwrap());
            assert_eq!(decoder.color_type(), image::ColorType::Rgba16);
            let encoded = image::DynamicImage::from_decoder(decoder).unwrap();
            assert!(encoded.to_rgba16().pixels().any(|p| p[0] % 257 != 0));
            let roundtrip = crate::color_management::import_icc(&encoded, &profile).unwrap();
            assert!((roundtrip.pixels()[0].red - image.pixels()[0].red).abs() < 5e-4);
        }
        assert_eq!(std::fs::read(source).unwrap(), b"protected");
    }
    #[test]
    fn cancelled_export_keeps_existing_destination() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.png");
        let target = directory.path().join("target.png");
        std::fs::write(&source, b"source").unwrap();
        std::fs::write(&target, b"keep").unwrap();
        let cancel = AtomicBool::new(true);
        let image = FloatImage::blank(2, 2, FloatRgba::BLACK).unwrap();
        assert!(save_color_image(
            &image,
            &source,
            &target,
            ExportProfile::Lossless,
            Default::default(),
            Some(&cancel)
        )
        .is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"keep");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2);
    }
}
