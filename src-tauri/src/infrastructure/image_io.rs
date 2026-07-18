use crate::domain::ImageMetadata;
use crate::error::AppError;
use base64::{engine::general_purpose::STANDARD, Engine};
use image::{DynamicImage, ImageFormat, ImageReader};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_PIXELS: u64 = 120_000_000;
const MAX_FILE_BYTES: u64 = 750 * 1024 * 1024;
const PREVIEW_MAX_DIMENSION: u32 = 1_600;

pub struct LoadedImage {
    pub path: PathBuf,
    pub original: Arc<DynamicImage>,
    pub preview: Arc<DynamicImage>,
    pub metadata: ImageMetadata,
}

pub fn load_image(path: &Path) -> Result<LoadedImage, AppError> {
    let canonical_path = fs::canonicalize(path).map_err(map_io_error)?;
    let file_metadata = fs::metadata(&canonical_path).map_err(map_io_error)?;
    if !file_metadata.is_file() {
        return Err(AppError::UnsupportedImageFormat);
    }
    if file_metadata.len() > MAX_FILE_BYTES {
        return Err(AppError::OutOfMemoryRisk);
    }

    let (width, height) = image::image_dimensions(&canonical_path).map_err(map_image_error)?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_PIXELS {
        return Err(AppError::ImageTooLarge {
            pixels,
            limit: MAX_PIXELS,
        });
    }

    let reader = ImageReader::open(&canonical_path)
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

    let decoded = reader.decode().map_err(map_image_error)?;
    let preview = if width > PREVIEW_MAX_DIMENSION || height > PREVIEW_MAX_DIMENSION {
        decoded.thumbnail(PREVIEW_MAX_DIMENSION, PREVIEW_MAX_DIMENSION)
    } else {
        decoded.clone()
    };

    let filename = canonical_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image")
        .to_string();
    let metadata = ImageMetadata {
        filename,
        width,
        height,
        format: format_name(format).to_string(),
        file_size: file_metadata.len(),
    };

    Ok(LoadedImage {
        path: canonical_path,
        original: Arc::new(decoded),
        preview: Arc::new(preview),
        metadata,
    })
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
    let safe_path = validate_output_path(original_path, output_path)?;
    let format = output_format(&safe_path)?;
    image
        .save_with_format(&safe_path, format)
        .map_err(map_export_error)?;
    Ok(safe_path)
}

fn validate_output_path(original_path: &Path, output_path: &Path) -> Result<PathBuf, AppError> {
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

fn output_format(path: &Path) -> Result<ImageFormat, AppError> {
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
}
