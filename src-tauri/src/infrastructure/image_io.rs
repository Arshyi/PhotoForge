use crate::domain::{ExportProfile, ImageMetadata};
use crate::error::AppError;
use base64::{engine::general_purpose::STANDARD, Engine};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPEncoder;
use image::{
    DynamicImage, ExtendedColorType, GenericImageView, ImageDecoder, ImageEncoder, ImageFormat,
    ImageReader, Limits, Rgb, RgbImage,
};
use std::fs;
use std::io::{BufWriter, Cursor};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::{camera_model, file_time};

/// The edge ceiling is structural; the pixel and byte ceilings come from the
/// resource budget, so they are functions rather than constants.
const MAX_DIMENSION: u32 = crate::resources::HARD_MAX_CANVAS_DIMENSION;

fn max_pixels() -> u64 {
    crate::resources::max_working_pixels()
}

fn max_decoded_bytes() -> u64 {
    crate::resources::max_working_image_bytes()
}
const PREVIEW_MAX_DIMENSION: u32 = 1_600;
const JPEG_QUALITY: u8 = 90;

pub struct LoadedImage {
    pub path: PathBuf,
    pub original: Arc<DynamicImage>,
    pub preview: Arc<DynamicImage>,
    pub metadata: ImageMetadata,
    /// Present for high-bit-depth or ICC sources. The byte preview is never
    /// substituted for these authoritative samples in a linear document.
    pub working: Option<Arc<crate::color::FloatImage>>,
}

pub fn load_image(path: &Path) -> Result<LoadedImage, AppError> {
    let canonical_path = fs::canonicalize(path).map_err(map_io_error)?;
    let file_metadata = fs::metadata(&canonical_path).map_err(map_io_error)?;
    if !file_metadata.is_file() {
        return Err(AppError::UnsupportedImageFormat);
    }

    // One arbiter. The planner reads the header, prices opening this source as a
    // conventional document against the budget in force, and either admits it or
    // says why not. Anything it does not admit whole is a job for the region or
    // reduced-copy paths, not for this one.
    let probed = crate::source::probe::probe_path(&canonical_path)?;
    crate::source::probe::require_full_admission(&probed)?;
    let (width, height) = (probed.width, probed.height);
    validate_dimensions(width, height)?;

    let mut reader = ImageReader::open(&canonical_path)
        .map_err(map_io_error)?
        .with_guessed_format()
        .map_err(map_io_error)?;
    let format = reader.format().ok_or(AppError::UnsupportedImageFormat)?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err(AppError::UnsupportedImageFormat);
    }

    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(max_decoded_bytes());
    reader.limits(limits);

    let mut decoder = reader.into_decoder().map_err(map_image_error)?;
    if decoder.total_bytes() > max_decoded_bytes() {
        return Err(AppError::OutOfMemoryRisk);
    }
    let icc = decoder.icc_profile().map_err(map_image_error)?;
    let decoded = DynamicImage::from_decoder(decoder).map_err(map_image_error)?;
    assemble_loaded(canonical_path, &file_metadata, format, decoded, icc, None)
}

/// Everything after the pixels are decoded: colour management, the preview and
/// the metadata.
///
/// Shared by the whole-file open and by region opens, deliberately. A region has
/// to come out of exactly the colour pipeline a full open uses, or "the region
/// matches the same area of a full render" would be untrue before any editing
/// began. `origin` is set only when what was decoded is part of a larger file.
pub(crate) fn assemble_loaded(
    canonical_path: PathBuf,
    file_metadata: &fs::Metadata,
    format: ImageFormat,
    decoded: DynamicImage,
    icc: Option<Vec<u8>>,
    origin: Option<crate::source::SourceOrigin>,
) -> Result<LoadedImage, AppError> {
    let (width, height) = decoded.dimensions();
    let color = decoded.color();
    let working = if let Some(profile) = &icc {
        Some(Arc::new(crate::color_management::import_icc(
            &decoded, profile,
        )?))
    } else if color.bits_per_pixel() / u16::from(color.channel_count()) > 8 {
        Some(Arc::new(crate::color::FloatImage::from_dynamic(&decoded)?))
    } else {
        None
    };
    let preview = if let Some(working) = &working {
        let (pw, ph) = crate::layers::preview_dimensions(width, height);
        DynamicImage::ImageRgba8(working.resized(pw, ph)?.to_rgba8())
    } else if width > PREVIEW_MAX_DIMENSION || height > PREVIEW_MAX_DIMENSION {
        decoded.thumbnail(PREVIEW_MAX_DIMENSION, PREVIEW_MAX_DIMENSION)
    } else {
        decoded.clone()
    };

    let metadata = metadata_for(
        &canonical_path,
        file_metadata,
        format,
        (width, height),
        icc.is_some(),
        (color.bits_per_pixel() / u16::from(color.channel_count())).min(255) as u8,
        color.has_alpha(),
        origin,
    );
    Ok(LoadedImage {
        path: canonical_path,
        original: Arc::new(decoded),
        preview: Arc::new(preview),
        metadata,
        working,
    })
}

/// What the source file said about its own pixels, which a reduced copy no
/// longer carries in its samples but must still report.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SourceTraits {
    pub had_icc: bool,
    pub bit_depth: u8,
    pub has_alpha: bool,
}

/// A reduced copy, already in linear float: it was resampled in linear light, so
/// converting it again would be wrong, and the float image is authoritative.
pub(crate) fn assemble_reduced(
    canonical_path: PathBuf,
    file_metadata: &fs::Metadata,
    format: ImageFormat,
    reduced: crate::color::FloatImage,
    traits: SourceTraits,
    origin: crate::source::SourceOrigin,
) -> Result<LoadedImage, AppError> {
    let (width, height) = reduced.dimensions();
    let (pw, ph) = crate::layers::preview_dimensions(width, height);
    let preview = DynamicImage::ImageRgba8(reduced.resized(pw, ph)?.to_rgba8());
    // The 8-bit original the document keeps alongside, encoded from the float
    // image so the two agree.
    let original = DynamicImage::ImageRgba8(reduced.to_rgba8());
    let metadata = metadata_for(
        &canonical_path,
        file_metadata,
        format,
        (width, height),
        traits.had_icc,
        traits.bit_depth,
        traits.has_alpha,
        Some(origin),
    );
    Ok(LoadedImage {
        path: canonical_path,
        original: Arc::new(original),
        preview: Arc::new(preview),
        metadata,
        working: Some(Arc::new(reduced)),
    })
}

#[allow(clippy::too_many_arguments)]
fn metadata_for(
    canonical_path: &Path,
    file_metadata: &fs::Metadata,
    format: ImageFormat,
    (width, height): (u32, u32),
    had_icc: bool,
    bit_depth: u8,
    has_alpha: bool,
    origin: Option<crate::source::SourceOrigin>,
) -> ImageMetadata {
    let filename = canonical_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image")
        .to_string();
    let camera_model = camera_model(canonical_path);
    ImageMetadata {
        filename,
        width,
        height,
        format: format_name(format).to_string(),
        file_size: file_metadata.len(),
        color_space: if had_icc {
            "Embedded RGB ICC → linear sRGB".to_string()
        } else {
            "sRGB".to_string()
        },
        bit_depth,
        has_alpha,
        created_at: file_time(file_metadata.created()),
        modified_at: file_time(file_metadata.modified()),
        exif_available: camera_model.is_some(),
        camera_model,
        raw: None,
        origin,
    }
}

pub fn encode_preview(image: &DynamicImage) -> Result<String, AppError> {
    let mut bytes = Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, ImageFormat::Png)
        .map_err(|_| AppError::ProcessingFailure("preview encoding failed".into()))?;
    Ok(format!(
        "data:image/png;base64,{}",
        STANDARD.encode(bytes.into_inner())
    ))
}

pub fn save_image(
    image: &DynamicImage,
    original_path: &Path,
    output_path: &Path,
) -> Result<PathBuf, AppError> {
    save_image_with_profile(image, original_path, output_path, ExportProfile::Lossless)
}

pub fn save_image_with_profile(
    image: &DynamicImage,
    original_path: &Path,
    output_path: &Path,
    profile: ExportProfile,
) -> Result<PathBuf, AppError> {
    let safe_path = validate_output_path(original_path, output_path)?;
    let format = output_format(&safe_path)?;
    let file = fs::File::create(&safe_path).map_err(map_export_io_error)?;
    let writer = BufWriter::new(file);
    let (width, height) = image.dimensions();

    match format {
        ImageFormat::Png => {
            let rgba = image.to_rgba8();
            PngEncoder::new(writer)
                .write_image(rgba.as_raw(), width, height, ExtendedColorType::Rgba8)
                .map_err(map_export_error)?;
        }
        ImageFormat::Jpeg => {
            let rgb = flatten_alpha_on_white(image);
            let quality = match profile {
                ExportProfile::Web => 82,
                ExportProfile::Print | ExportProfile::HighJpeg => 95,
                _ => JPEG_QUALITY,
            };
            JpegEncoder::new_with_quality(writer, quality)
                .encode(rgb.as_raw(), width, height, ExtendedColorType::Rgb8)
                .map_err(map_export_error)?;
        }
        ImageFormat::WebP => {
            let rgba = image.to_rgba8();
            WebPEncoder::new_lossless(writer)
                .encode(rgba.as_raw(), width, height, ExtendedColorType::Rgba8)
                .map_err(map_export_error)?;
        }
        _ => return Err(AppError::InvalidOutputPath),
    }
    Ok(safe_path)
}

/// Writes a high-precision development result as a true 16-bit RGBA PNG.
/// `FloatImage` has already validated dimensions and finite values; this
/// function only owns the output-path policy and the PNG boundary.
pub fn save_float_png16(
    image: &crate::color::FloatImage,
    original_path: &Path,
    output_path: &Path,
) -> Result<PathBuf, AppError> {
    let safe_path = validate_output_path(original_path, output_path)?;
    if output_format(&safe_path)? != ImageFormat::Png {
        return Err(AppError::InvalidOutputPath);
    }
    let file = fs::File::create(&safe_path).map_err(map_export_io_error)?;
    let writer = BufWriter::new(file);
    PngEncoder::new(writer)
        .write_image(
            &image.to_rgba16_native_bytes(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba16,
        )
        .map_err(map_export_error)?;
    Ok(safe_path)
}

fn flatten_alpha_on_white(image: &DynamicImage) -> RgbImage {
    let rgba = image.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let pixel = rgba.get_pixel(x, y);
        let alpha = u16::from(pixel[3]);
        let blend = |channel: u8| -> u8 {
            (((u16::from(channel) * alpha) + (255 * (255 - alpha)) + 127) / 255) as u8
        };
        Rgb([blend(pixel[0]), blend(pixel[1]), blend(pixel[2])])
    })
}

fn validate_dimensions(width: u32, height: u32) -> Result<u64, AppError> {
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(AppError::ImageTooLarge {
            pixels: u64::from(width).saturating_mul(u64::from(height)),
            limit: max_pixels(),
        });
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(AppError::OutOfMemoryRisk)?;
    if pixels > max_pixels() {
        return Err(AppError::ImageTooLarge {
            pixels,
            limit: max_pixels(),
        });
    }
    Ok(pixels)
}

pub(super) fn validate_output_path(
    original_path: &Path,
    output_path: &Path,
) -> Result<PathBuf, AppError> {
    if !output_path.is_absolute() || output_path.file_name().is_none() {
        return Err(AppError::InvalidOutputPath);
    }

    let original = fs::canonicalize(original_path).map_err(map_io_error)?;
    let output = if output_path.exists() {
        if !output_path.is_file() {
            return Err(AppError::InvalidOutputPath);
        }
        fs::canonicalize(output_path).map_err(map_io_error)?
    } else {
        let parent = output_path.parent().ok_or(AppError::InvalidOutputPath)?;
        let canonical_parent = fs::canonicalize(parent).map_err(map_io_error)?;
        if !canonical_parent.is_dir() {
            return Err(AppError::InvalidOutputPath);
        }
        canonical_parent.join(output_path.file_name().ok_or(AppError::InvalidOutputPath)?)
    };

    if paths_equal(&original, &output) {
        return Err(AppError::InvalidOutputPath);
    }
    Ok(output)
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

pub(super) fn output_format(path: &Path) -> Result<ImageFormat, AppError> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Ok(ImageFormat::Png),
        Some("jpg" | "jpeg") => Ok(ImageFormat::Jpeg),
        Some("webp") => Ok(ImageFormat::WebP),
        _ => Err(AppError::InvalidOutputPath),
    }
}

fn format_name(format: ImageFormat) -> &'static str {
    match format {
        ImageFormat::Png => "PNG",
        ImageFormat::Jpeg => "JPEG",
        ImageFormat::WebP => "WebP",
        _ => "Unsupported",
    }
}

fn map_io_error(error: std::io::Error) -> AppError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        AppError::Permission
    } else {
        AppError::DecodeFailure
    }
}

fn map_image_error(error: image::ImageError) -> AppError {
    match error {
        image::ImageError::Unsupported(_) => AppError::UnsupportedImageFormat,
        image::ImageError::Limits(_) => AppError::OutOfMemoryRisk,
        image::ImageError::IoError(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            AppError::Permission
        }
        image::ImageError::Decoding(_) => AppError::CorruptImage,
        _ => AppError::DecodeFailure,
    }
}

fn map_export_error(error: image::ImageError) -> AppError {
    match error {
        image::ImageError::IoError(error)
            if error.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            AppError::Permission
        }
        _ => AppError::ExportFailure,
    }
}

fn map_export_io_error(error: std::io::Error) -> AppError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        AppError::Permission
    } else {
        AppError::ExportFailure
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    #[test]
    fn rejects_unsupported_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes.txt");
        fs::write(&path, b"not an image").unwrap();
        assert!(matches!(
            load_image(&path),
            Err(AppError::UnsupportedImageFormat) | Err(AppError::DecodeFailure)
        ));
    }

    #[test]
    fn prevents_export_over_the_original() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.png");
        let image = RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255]));
        image.save(&original).unwrap();

        assert!(matches!(
            validate_output_path(&original, &original),
            Err(AppError::InvalidOutputPath)
        ));
    }

    #[test]
    fn rejects_unsafe_export_extensions() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original.png");
        let image = RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255]));
        image.save(&original).unwrap();
        let output = directory.path().join("output.exe");

        assert!(matches!(
            save_image(&DynamicImage::ImageRgba8(image), &original, &output),
            Err(AppError::InvalidOutputPath)
        ));
    }

    #[test]
    fn rejects_dimensions_that_exceed_the_memory_budget() {
        assert!(matches!(
            validate_dimensions(50_000, 50_000),
            Err(AppError::ImageTooLarge { .. })
        ));
        assert!(matches!(
            validate_dimensions(10_000, 7_000),
            Err(AppError::ImageTooLarge { .. })
        ));
        assert_eq!(validate_dimensions(8_000, 5_000).unwrap(), 40_000_000);
        assert_eq!(validate_dimensions(9504, 6336).unwrap(), 60_217_344);
    }

    #[test]
    fn preserves_alpha_in_png_and_webp_exports() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("source.png");
        let image = RgbaImage::from_pixel(2, 1, Rgba([20, 40, 60, 73]));
        image.save(&original).unwrap();
        let dynamic = DynamicImage::ImageRgba8(image);

        for extension in ["png", "webp"] {
            let output = directory.path().join(format!("alpha output.{extension}"));
            save_image(&dynamic, &original, &output).unwrap();
            let decoded = image::open(output).unwrap().to_rgba8();
            assert_eq!(decoded.get_pixel(0, 0)[3], 73);
        }
    }

    #[test]
    fn jpeg_export_flattens_alpha_onto_white() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("source.png");
        let image = RgbaImage::from_pixel(8, 8, Rgba([255, 0, 0, 0]));
        image.save(&original).unwrap();
        let output = directory.path().join("flattened image.jpg");

        save_image(&DynamicImage::ImageRgba8(image), &original, &output).unwrap();
        let pixel = image::open(output).unwrap().to_rgb8().get_pixel(4, 4).0;
        assert!(pixel.iter().all(|channel| *channel >= 245));
    }

    #[test]
    fn loads_supported_images_from_unicode_and_space_paths() {
        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("space folder");
        fs::create_dir(&nested).unwrap();
        let path = nested.join("résumé_日本語.png");
        RgbaImage::from_pixel(3, 2, Rgba([1, 2, 3, 4]))
            .save(&path)
            .unwrap();

        let loaded = load_image(&path).unwrap();
        assert_eq!((loaded.metadata.width, loaded.metadata.height), (3, 2));
        assert_eq!(loaded.metadata.filename, "résumé_日本語.png");
    }

    #[test]
    fn loads_png_jpeg_webp_and_grayscale_inputs() {
        let directory = tempfile::tempdir().unwrap();
        let rgb = image::RgbImage::from_fn(9, 7, |x, y| {
            image::Rgb([(x * 17) as u8, (y * 23) as u8, 91])
        });

        for (extension, expected_format) in [("png", "PNG"), ("jpg", "JPEG"), ("webp", "WebP")] {
            let path = directory.path().join(format!("fixture.{extension}"));
            rgb.save(&path).unwrap();
            let loaded = load_image(&path).unwrap();
            assert_eq!((loaded.metadata.width, loaded.metadata.height), (9, 7));
            assert_eq!(loaded.metadata.format, expected_format);
        }

        let grayscale_path = directory.path().join("grayscale.png");
        image::GrayImage::from_pixel(5, 4, image::Luma([123]))
            .save(&grayscale_path)
            .unwrap();
        assert_eq!(load_image(&grayscale_path).unwrap().metadata.format, "PNG");
    }

    #[test]
    fn rejects_empty_truncated_and_renamed_invalid_inputs() {
        let directory = tempfile::tempdir().unwrap();
        let empty = directory.path().join("empty.jpg");
        fs::write(&empty, []).unwrap();
        assert!(load_image(&empty).is_err());

        let truncated = directory.path().join("truncated.jpg");
        fs::write(&truncated, [0xff, 0xd8, 0xff, 0xe0, 0x00]).unwrap();
        assert!(load_image(&truncated).is_err());

        let renamed = directory.path().join("not-really-an-image.png");
        fs::write(&renamed, b"plain text with an image extension").unwrap();
        assert!(load_image(&renamed).is_err());
    }

    #[test]
    fn bounds_preview_dimensions_and_keeps_the_full_resolution_source() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wide.png");
        RgbaImage::from_pixel(2_400, 1_200, Rgba([4, 5, 6, 255]))
            .save(&path)
            .unwrap();

        let loaded = load_image(&path).unwrap();
        assert_eq!(loaded.original.dimensions(), (2_400, 1_200));
        assert_eq!(loaded.preview.dimensions(), (1_600, 800));
    }

    #[test]
    fn export_preserves_dimensions_and_does_not_modify_the_source() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.png");
        RgbaImage::from_pixel(13, 17, Rgba([11, 22, 33, 44]))
            .save(&source)
            .unwrap();
        let before = fs::read(&source).unwrap();

        for extension in ["png", "jpg", "webp"] {
            let output = directory.path().join(format!("export.{extension}"));
            save_image(&image::open(&source).unwrap(), &source, &output).unwrap();
            assert_eq!(image::image_dimensions(output).unwrap(), (13, 17));
        }
        assert_eq!(fs::read(&source).unwrap(), before);
    }

    #[test]
    fn high_precision_png_export_preserves_float_quantisation_and_source() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source.png");
        RgbaImage::from_pixel(1, 1, Rgba([128, 64, 32, 255]))
            .save(&source)
            .unwrap();
        let before = fs::read(&source).unwrap();
        let image = crate::color::FloatImage::new(
            1,
            1,
            vec![crate::color::FloatRgba::new(0.21404114, 0.5, 1.0, 1.0)],
        )
        .unwrap();
        let output = directory.path().join("developed.png");
        save_float_png16(&image, &source, &output).unwrap();
        let decoded = image::open(&output).unwrap();
        assert_eq!(decoded.color(), image::ColorType::Rgba16);
        assert_eq!(
            decoded.to_rgba16().get_pixel(0, 0).0,
            [32_768, 48_192, 65_535, 65_535]
        );
        assert_eq!(fs::read(&source).unwrap(), before);
    }

    #[test]
    #[allow(clippy::permissions_set_readonly_false)]
    fn opens_a_read_only_source() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("read-only-source.png");
        RgbaImage::from_pixel(2, 2, Rgba([1, 2, 3, 255]))
            .save(&path)
            .unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();

        assert_eq!(load_image(&path).unwrap().metadata.width, 2);

        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).unwrap();
    }
}
