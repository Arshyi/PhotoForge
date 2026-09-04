//! Atomic, tagged color export. PNG is written row by row, without a full-frame
//! 8/16-bit intermediate. Metadata is limited to the chosen RGB ICC profile.
use crate::color::{FloatImage, FloatRgba};
use crate::color_management::{quantize8, ColorExportOptions};
use crate::domain::ExportProfile;
use crate::error::AppError;
use image::{ExtendedColorType, ImageEncoder};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// Converts one row of working-space float pixels into PNG bytes.
///
/// Shared by the whole-image and streaming paths so an export cannot quantize
/// differently depending on how its pixels were produced. `y` matters: ordered
/// dither is a function of the pixel's position in the finished image, so a
/// band has to be told which rows of the output it holds.
fn encode_row(row: &mut Vec<u8>, pixels: &[FloatRgba], y: u32, options: &ColorExportOptions) {
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
                row.push(quantize8(c, x as u32, y, options.dither));
            }
            row.push(quantize8(p.alpha, x as u32, y, false));
        }
    }
}

/// Publishes a finished temporary file at its real path, or not at all.
fn persist_export(
    temporary: tempfile::NamedTempFile,
    path: &Path,
    _profile: ExportProfile,
) -> Result<PathBuf, AppError> {
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| AppError::ExportFailure)?;
    temporary
        .persist(path)
        .map_err(|_| AppError::ExportFailure)?;
    Ok(path.to_path_buf())
}

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
                encode_row(&mut row, pixels, y as u32, &options);
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

/// Writes a PNG whose pixels are produced a band at a time.
///
/// `save_color_image` already streams rows into the encoder, but it streams
/// them out of a float frame the caller had to build first — at 45 megapixels
/// that frame is 719 MB before a single byte is written. Here the caller is
/// handed a sink instead and pushes bands as it renders them, so nothing wider
/// than one band exists at once.
///
/// The pixel arithmetic is deliberately the same code as the whole-image path
/// (`encode_row`), because an export that streamed would otherwise be free to
/// quantize differently from one that did not. A test asserts the two produce
/// identical files.
///
/// PNG only: JPEG and WebP encoders in this build take a whole buffer, so
/// there is nothing to stream into.
// One more argument than the lint likes: it is `save_color_image`'s signature
// with the image replaced by dimensions plus a producer, and keeping the two in
// step matters more than the count.
#[allow(clippy::too_many_arguments)]
pub fn save_color_image_streaming(
    width: u32,
    height: u32,
    original: &Path,
    output: &Path,
    profile: ExportProfile,
    options: ColorExportOptions,
    cancel: Option<&AtomicBool>,
    render: impl FnOnce(
        &mut dyn FnMut(u32, &FloatImage) -> Result<(), AppError>,
    ) -> Result<(), AppError>,
) -> Result<PathBuf, AppError> {
    options.validate()?;
    let path = super::image_io::validate_output_path(original, output)?;
    let format = super::image_io::output_format(&path)?;
    if format != image::ImageFormat::Png {
        return Err(AppError::ColorPipeline(
            "streaming export is implemented for PNG only".into(),
        ));
    }
    if width == 0 || height == 0 {
        return Err(AppError::ColorPipeline("empty export".into()));
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
        let mut info = png::Info::with_size(width, height);
        info.color_type = png::ColorType::Rgba;
        info.bit_depth = if options.bit_depth == 16 {
            png::BitDepth::Sixteen
        } else {
            png::BitDepth::Eight
        };
        info.icc_profile = Some(std::borrow::Cow::Owned(icc));
        let encoder = png::Encoder::with_info(writer, info).map_err(|_| AppError::ExportFailure)?;
        let mut writer = encoder
            .write_header()
            .map_err(|_| AppError::ExportFailure)?;
        let mut stream = writer
            .stream_writer()
            .map_err(|_| AppError::ExportFailure)?;
        let mut row = Vec::with_capacity(width as usize * usize::from(options.bit_depth / 8) * 4);
        // Counted, not trusted: the encoder would accept a short file and the
        // caller is the one deciding how many rows to push.
        let mut written = 0u32;
        {
            let mut emit = |first_row: u32, band: &FloatImage| -> Result<(), AppError> {
                if band.width() != width {
                    return Err(AppError::ColorPipeline(
                        "an export band was not the width of the image".into(),
                    ));
                }
                if first_row != written {
                    return Err(AppError::ColorPipeline(
                        "export bands arrived out of order".into(),
                    ));
                }
                band.validate()?;
                for (index, pixels) in band.pixels().chunks(width as usize).enumerate() {
                    crate::image_processing::high_precision::check_cancel(cancel)?;
                    let y = first_row
                        .checked_add(index as u32)
                        .ok_or(AppError::ExportFailure)?;
                    if y >= height {
                        return Err(AppError::ColorPipeline(
                            "an export band ran past the end of the image".into(),
                        ));
                    }
                    encode_row(&mut row, pixels, y, &options);
                    stream
                        .write_all(&row)
                        .map_err(|_| AppError::ExportFailure)?;
                    written = y + 1;
                }
                Ok(())
            };
            render(&mut emit)?;
        }
        if written != height {
            return Err(AppError::ColorPipeline(
                "the export ended before every row was written".into(),
            ));
        }
        stream.finish().map_err(|_| AppError::ExportFailure)?;
        writer.finish().map_err(|_| AppError::ExportFailure)?;
    }
    persist_export(temporary, &path, profile)
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

    /// Builds a deterministic test image with values that exercise both the
    /// 16-bit quantizer and the ordered dither in the 8-bit path.
    fn streaming_fixture(width: u32, height: u32) -> FloatImage {
        let mut image = FloatImage::blank(width, height, FloatRgba::TRANSPARENT).unwrap();
        for (index, pixel) in image.pixels_mut().iter_mut().enumerate() {
            let i = index as u32;
            let (x, y) = (i % width, i / width);
            let n = |k: u32| f32::from((x * 7 + y * 13 + k * 29) as u16 % 251) / 251.0;
            *pixel = FloatRgba {
                red: n(1),
                green: n(2),
                blue: n(3),
                alpha: 0.25 + n(4) * 0.75,
            };
        }
        image
    }

    /// Streaming must be an allocation strategy, not a different export.
    ///
    /// Byte equality is the only assertion worth making here: if the streamed
    /// file merely *looked* the same, dither phase or row filtering could still
    /// differ, and the export would depend on how much memory was available.
    #[test]
    fn a_streamed_export_is_byte_identical_to_a_whole_image_export() {
        for (bit_depth, dither) in [(16u8, false), (8, true), (8, false)] {
            let (width, height) = (61u32, 47u32);
            let image = streaming_fixture(width, height);
            let folder = tempfile::tempdir().unwrap();
            let source = folder.path().join("source.png");
            std::fs::write(&source, b"sentinel").unwrap();
            let options = ColorExportOptions {
                bit_depth,
                dither,
                ..Default::default()
            };

            let whole = folder.path().join("whole.png");
            save_color_image(
                &image,
                &source,
                &whole,
                ExportProfile::Lossless,
                options,
                None,
            )
            .unwrap();

            // Deliberately ragged bands, including a final short one, so the
            // test does not silently depend on a tidy division.
            let streamed = folder.path().join("streamed.png");
            save_color_image_streaming(
                width,
                height,
                &source,
                &streamed,
                ExportProfile::Lossless,
                options,
                None,
                |emit| {
                    let mut first_row = 0;
                    for rows in [16u32, 1, 20, 10] {
                        let rows = rows.min(height - first_row);
                        let mut band = FloatImage::blank(width, rows, FloatRgba::TRANSPARENT)?;
                        let offset = first_row as usize * width as usize;
                        let count = band.pixels().len();
                        band.pixels_mut()
                            .copy_from_slice(&image.pixels()[offset..offset + count]);
                        emit(first_row, &band)?;
                        first_row += rows;
                    }
                    Ok(())
                },
            )
            .unwrap();

            assert_eq!(
                std::fs::read(&whole).unwrap(),
                std::fs::read(&streamed).unwrap(),
                "streaming changed the file at {bit_depth} bits with dither {dither}"
            );
        }
    }

    /// The encoder cannot seek backwards, so a sink that skips or repeats rows
    /// would silently write a corrupt image. It must be refused instead.
    #[test]
    fn a_streamed_export_refuses_bands_that_do_not_tile_the_image() {
        let (width, height) = (32u32, 32u32);
        let image = streaming_fixture(width, height);
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("source.png");
        std::fs::write(&source, b"sentinel").unwrap();

        let cases: Vec<(&str, Vec<(u32, u32)>)> = vec![
            ("out of order", vec![(16, 16), (0, 16)]),
            ("overlapping", vec![(0, 16), (8, 16)]),
            ("short", vec![(0, 16)]),
            ("past the end", vec![(0, 16), (16, 16), (32, 16)]),
        ];
        for (name, bands) in cases {
            let output = folder
                .path()
                .join(format!("{}.png", name.replace(' ', "-")));
            let result = save_color_image_streaming(
                width,
                height,
                &source,
                &output,
                ExportProfile::Lossless,
                ColorExportOptions::default(),
                None,
                |emit| {
                    for (first_row, rows) in bands {
                        let mut band = FloatImage::blank(width, rows, FloatRgba::TRANSPARENT)?;
                        let offset = (first_row as usize * width as usize)
                            .min(image.pixels().len().saturating_sub(band.pixels().len()));
                        let count = band.pixels().len();
                        band.pixels_mut()
                            .copy_from_slice(&image.pixels()[offset..offset + count]);
                        emit(first_row, &band)?;
                    }
                    Ok(())
                },
            );
            assert!(result.is_err(), "{name} bands were accepted");
            assert!(!output.exists(), "{name} bands left a partial file behind");
        }
    }
}
