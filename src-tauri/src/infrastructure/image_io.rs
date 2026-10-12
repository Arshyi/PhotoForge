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
    /// What the source file carried (EXIF, XMP, IPTC), kept with the document so that an export
    /// can write it back under the policy the person chose. `None` for a document that never
    /// had a file of its own (a blank canvas, a RAW development).
    pub source_metadata: Option<Arc<crate::metadata::SourceMetadata>>,
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
    let mut source_metadata = crate::metadata::scan::read(&canonical_path);
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(map_image_error)?;
    // Phone and camera files are stored in the sensor's orientation with a tag saying how to turn
    // them; every other viewer applies it, so the document does too, and the tag is then spent.
    if let Some(metadata) = source_metadata.as_mut() {
        if let Some(turn) = image::metadata::Orientation::from_exif(metadata.source_orientation)
            .filter(|turn| *turn != image::metadata::Orientation::NoTransforms)
        {
            decoded.apply_orientation(turn);
            metadata.orientation_applied = true;
        }
    }
    assemble_loaded(
        canonical_path,
        &file_metadata,
        format,
        decoded,
        icc,
        None,
        source_metadata,
    )
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
    source_metadata: Option<crate::metadata::SourceMetadata>,
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
        source_metadata.as_ref(),
    );
    Ok(LoadedImage {
        path: canonical_path,
        original: Arc::new(decoded),
        preview: Arc::new(preview),
        metadata,
        working,
        source_metadata: source_metadata.map(Arc::new),
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
    source_metadata: Option<crate::metadata::SourceMetadata>,
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
        source_metadata.as_ref(),
    );
    Ok(LoadedImage {
        path: canonical_path,
        original: Arc::new(original),
        preview: Arc::new(preview),
        metadata,
        working: Some(Arc::new(reduced)),
        source_metadata: source_metadata.map(Arc::new),
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
    source_metadata: Option<&crate::metadata::SourceMetadata>,
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
        source_metadata: source_metadata
            .filter(|metadata| !metadata.is_empty())
            .map(|metadata| metadata.summary()),
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
    save_image_with_metadata(image, original_path, output_path, profile, None)
}

/// As [`save_image_with_profile`], writing the metadata of `plan` into the file and recording in
/// its report what the file had no place for.
pub fn save_image_with_metadata(
    image: &DynamicImage,
    original_path: &Path,
    output_path: &Path,
    profile: ExportProfile,
    plan: Option<&mut crate::metadata::Plan>,
) -> Result<PathBuf, AppError> {
    let safe_path = validate_output_path(original_path, output_path)?;
    let format = output_format(&safe_path)?;
    let (width, height) = image.dimensions();
    // Encoded in memory so that the metadata can be inserted; the file is made only once, whole.
    let mut encoded = Vec::new();
    let writer = &mut encoded;

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
    if let Some(plan) = plan {
        encoded = embed_metadata(format, encoded, plan)?;
    }
    let file = fs::File::create(&safe_path).map_err(map_export_io_error)?;
    let mut writer = BufWriter::new(file);
    std::io::Write::write_all(&mut writer, &encoded).map_err(map_export_io_error)?;
    std::io::Write::flush(&mut writer).map_err(map_export_io_error)?;
    Ok(safe_path)
}

/// Inserts a plan's metadata into a finished file of `format`.
pub(crate) fn embed_metadata(
    format: ImageFormat,
    encoded: Vec<u8>,
    plan: &mut crate::metadata::Plan,
) -> Result<Vec<u8>, AppError> {
    let kind = match format {
        ImageFormat::Jpeg => crate::metadata::container::Format::Jpeg,
        ImageFormat::Png => crate::metadata::container::Format::Png,
        ImageFormat::WebP => crate::metadata::container::Format::WebP,
        _ => return Ok(encoded),
    };
    crate::metadata::embed_into(kind, encoded, &plan.prepared, &mut plan.report)
}

/// Writes a high-precision development result as a true 16-bit RGBA PNG.
/// `FloatImage` has already validated dimensions and finite values; this
/// function only owns the output-path policy and the PNG boundary.
pub fn save_float_png16(
    image: &crate::color::FloatImage,
    original_path: &Path,
    output_path: &Path,
) -> Result<PathBuf, AppError> {
    save_float_png16_with_metadata(image, original_path, output_path, None)
}

/// As [`save_float_png16`], with the metadata of `plan`.
pub fn save_float_png16_with_metadata(
    image: &crate::color::FloatImage,
    original_path: &Path,
    output_path: &Path,
    plan: Option<&mut crate::metadata::Plan>,
) -> Result<PathBuf, AppError> {
    let safe_path = validate_output_path(original_path, output_path)?;
    if output_format(&safe_path)? != ImageFormat::Png {
        return Err(AppError::InvalidOutputPath);
    }
    let mut encoded = Vec::new();
    PngEncoder::new(&mut encoded)
        .write_image(
            &image.to_rgba16_native_bytes(),
            image.width(),
            image.height(),
            ExtendedColorType::Rgba16,
        )
        .map_err(map_export_error)?;
    if let Some(plan) = plan {
        encoded = embed_metadata(ImageFormat::Png, encoded, plan)?;
    }
    let file = fs::File::create(&safe_path).map_err(map_export_io_error)?;
    let mut writer = BufWriter::new(file);
    std::io::Write::write_all(&mut writer, &encoded).map_err(map_export_io_error)?;
    std::io::Write::flush(&mut writer).map_err(map_export_io_error)?;
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

    // A document that was not read from a file (a clipboard picture, a new blank document) has
    // no source to protect; resolving its empty path would fail as if the source could not be
    // read. Everything about the destination is still checked.
    let original = if has_no_source_file(original_path) {
        None
    } else {
        Some(fs::canonicalize(original_path).map_err(map_io_error)?)
    };
    let output = resolve_output_path(output_path)?;

    if original
        .as_deref()
        .is_some_and(|original| paths_equal(original, &output))
    {
        return Err(AppError::InvalidOutputPath);
    }
    Ok(output)
}

/// Whether `source` says that the document has no file of its own: an empty path, which is how a
/// picture pasted from the clipboard and a new blank document are staged.
fn has_no_source_file(source: &Path) -> bool {
    source.as_os_str().is_empty()
}

/// Where a write to `output_path` would land, with symbolic links, `..` segments and short names
/// resolved: the file itself when it exists, otherwise its folder (which must exist) joined to
/// its name.
fn resolve_output_path(output_path: &Path) -> Result<PathBuf, AppError> {
    if output_path.exists() {
        if !output_path.is_file() {
            return Err(AppError::InvalidOutputPath);
        }
        fs::canonicalize(output_path).map_err(map_io_error)
    } else {
        let parent = output_path.parent().ok_or(AppError::InvalidOutputPath)?;
        let canonical_parent = fs::canonicalize(parent).map_err(map_io_error)?;
        if !canonical_parent.is_dir() {
            return Err(AppError::InvalidOutputPath);
        }
        Ok(canonical_parent.join(output_path.file_name().ok_or(AppError::InvalidOutputPath)?))
    }
}

/// Refuses an export that would replace the file the open document was read from.
///
/// This is the rule [`validate_output_path`] applies to the raster formats, for the exports that
/// do not go through it (Photoshop, SVG and OpenEXR). The destination is compared with `source`
/// after both are resolved, so a relative spelling, a symbolic link, `..` segments, a short name
/// or another case (where the file system ignores case) of the source is still the source, and
/// on Unix so is a hard link to it. The refusal is the raster one: it asks for a different file
/// name and names no path.
///
/// An empty `source` is a document that was not read from a file (a picture pasted from the
/// clipboard, a new blank document): there is nothing to protect, and that is not an error. A
/// destination that does not exist yet is not the source, and nothing is when the source itself
/// can no longer be reached (an unplugged drive): the export then replaces nothing the document
/// came from.
pub(crate) fn refuse_to_replace_source(source: &Path, destination: &Path) -> Result<(), AppError> {
    if has_no_source_file(source) {
        return Ok(());
    }
    let Ok(source) = fs::canonicalize(source) else {
        return Ok(());
    };
    if !destination.exists() {
        return Ok(());
    }
    // What cannot be resolved is not a file this export would replace; the export meets the same
    // fault when it writes, and reports it in its own words.
    let Ok(destination) = resolve_output_path(destination) else {
        return Ok(());
    };
    if paths_equal(&source, &destination) || is_same_file(&source, &destination) {
        return Err(AppError::InvalidOutputPath);
    }
    Ok(())
}

/// Whether two existing paths are one file under two names (a hard link), which no spelling of
/// the path shows. Only Unix reports a file's identity through the standard library; elsewhere
/// the resolved paths that [`paths_equal`] compares are all there is.
#[cfg(unix)]
fn is_same_file(left: &Path, right: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(left), fs::metadata(right)) {
        (Ok(left), Ok(right)) => left.dev() == right.dev() && left.ino() == right.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn is_same_file(_left: &Path, _right: &Path) -> bool {
    false
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

    /// What the destination checks say about `output` for a document read from `real_source`, and
    /// for one with no source file (an empty path): they must say the same, whatever it is.
    fn verdicts(real_source: &Path, output: &Path) -> (String, String) {
        (
            format!("{:?}", validate_output_path(real_source, output)),
            format!("{:?}", validate_output_path(Path::new(""), output)),
        )
    }

    #[test]
    fn a_document_with_no_source_file_exports_to_a_valid_new_path() {
        let directory = tempfile::tempdir().unwrap();
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(3, 2, Rgba([9, 80, 200, 255])));
        for name in ["new.png", "new.jpg", "new.jpeg", "new.webp", "SHOUT.PNG"] {
            let output = directory.path().join(name);
            let resolved = validate_output_path(Path::new(""), &output).unwrap();
            assert_eq!(resolved.file_name(), output.file_name());
            let written = save_image(&image, Path::new(""), &output).unwrap();
            assert_eq!(written, resolved);
            let decoded = image::open(&written).unwrap_or_else(|_| panic!("{name} decodes"));
            assert_eq!(decoded.dimensions(), (3, 2), "{name}");
        }
        // Replacing a file that is not the source is as allowed as it is with a source.
        let existing = directory.path().join("existing.png");
        fs::write(&existing, b"old").unwrap();
        save_image(&image, Path::new(""), &existing).unwrap();
        assert_eq!(image::open(&existing).unwrap().dimensions(), (3, 2));
    }

    #[test]
    fn every_raster_saver_accepts_a_document_with_no_source_file() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("float.png");
        let float =
            crate::color::FloatImage::blank(4, 3, crate::color::FloatRgba::TRANSPARENT).unwrap();
        save_float_png16(&float, Path::new(""), &output).unwrap();
        assert_eq!(image::open(&output).unwrap().dimensions(), (4, 3));
    }

    #[test]
    fn a_document_with_no_source_file_still_has_its_destination_checked() {
        let directory = tempfile::tempdir().unwrap();
        let real = directory.path().join("original.png");
        RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255]))
            .save(&real)
            .unwrap();
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])));
        let folder = directory.path().join("a folder.png");
        fs::create_dir(&folder).unwrap();

        // An extension nothing is written in, a folder that is not there, a folder where a file
        // should be, and a destination that is not a full path.
        let bad_extension = directory.path().join("notes.txt");
        let no_extension = directory.path().join("picture");
        for output in [&bad_extension, &no_extension] {
            assert!(matches!(
                save_image(&image, Path::new(""), output),
                Err(AppError::InvalidOutputPath)
            ));
            assert!(!output.exists());
        }
        let missing_parent = directory.path().join("no/such/folder/new.png");
        assert!(save_image(&image, Path::new(""), &missing_parent).is_err());
        assert!(!directory.path().join("no").exists());
        assert!(matches!(
            save_image(&image, Path::new(""), &folder),
            Err(AppError::InvalidOutputPath)
        ));
        for relative in ["new.png", "./new.png", ""] {
            assert!(matches!(
                validate_output_path(Path::new(""), Path::new(relative)),
                Err(AppError::InvalidOutputPath)
            ));
        }
        // A file where the parent folder should be is no folder either.
        let under_a_file = real.join("new.png");
        assert!(save_image(&image, Path::new(""), &under_a_file).is_err());

        // Each of them is refused in the very words a document with a source gets.
        for output in [&missing_parent, &folder, &under_a_file] {
            let (with_source, without) = verdicts(&real, output);
            assert!(with_source.starts_with("Err("), "{output:?}: {with_source}");
            assert_eq!(with_source, without, "{output:?}");
        }
    }

    #[test]
    fn a_source_file_is_still_protected_when_the_document_has_one() {
        let directory = tempfile::tempdir().unwrap();
        let real = directory.path().join("original.png");
        RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255]))
            .save(&real)
            .unwrap();
        let before = fs::read(&real).unwrap();
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([9, 9, 9, 255])));
        // The file itself, and the same file under another spelling.
        let respelled = directory.path().join(".").join("original.png");
        for output in [&real, &respelled] {
            assert!(matches!(
                save_image(&image, &real, output),
                Err(AppError::InvalidOutputPath)
            ));
        }
        assert_eq!(fs::read(&real).unwrap(), before);
        // Another file is another matter.
        let other = directory.path().join("other.png");
        save_image(&image, &real, &other).unwrap();
        assert_eq!(fs::read(&real).unwrap(), before);
    }

    /// A folder holding `scene.exr` (any bytes will do: the guard compares files, not contents).
    fn folder_with_source() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("scene.exr");
        fs::write(&source, b"the original").unwrap();
        (directory, source)
    }

    #[test]
    fn a_source_is_never_replaced_whatever_its_spelling() {
        let (directory, source) = folder_with_source();
        fs::create_dir(directory.path().join("sub")).unwrap();
        #[allow(unused_mut)] // only the Unix aliases below add to it
        let mut aliases = vec![
            source.clone(),
            directory.path().join("sub").join("..").join("scene.exr"),
            directory.path().join(".").join("scene.exr"),
        ];
        #[cfg(unix)]
        {
            let link = directory.path().join("link.exr");
            std::os::unix::fs::symlink(&source, &link).unwrap();
            let folder = directory.path().join("folder-link");
            std::os::unix::fs::symlink(directory.path(), &folder).unwrap();
            // A path that climbs out of the folder and back in by its own name.
            let name = directory.path().file_name().unwrap();
            let climbing = directory.path().join("..").join(name).join("scene.exr");
            aliases.extend([link, folder.join("scene.exr"), climbing]);
        }
        for alias in &aliases {
            assert!(
                matches!(
                    refuse_to_replace_source(&source, alias),
                    Err(AppError::InvalidOutputPath)
                ),
                "{} was not refused",
                alias.display()
            );
        }
        // And the other way round: the source may be the one spelled indirectly.
        let indirect = directory.path().join("sub").join("..").join("scene.exr");
        assert!(matches!(
            refuse_to_replace_source(&indirect, &source),
            Err(AppError::InvalidOutputPath)
        ));
        assert_eq!(fs::read(&source).unwrap(), b"the original");
    }

    #[cfg(unix)]
    #[test]
    fn a_relative_spelling_of_the_source_is_still_the_source() {
        let (_directory, source) = folder_with_source();
        // The source relative to the folder the tests run in, as `../../tmp/x/scene.exr`.
        let here = std::env::current_dir().unwrap();
        let mut relative = PathBuf::new();
        for _ in here
            .components()
            .filter(|c| matches!(c, std::path::Component::Normal(_)))
        {
            relative.push("..");
        }
        relative.push(source.strip_prefix("/").unwrap());
        assert!(relative.is_relative());
        assert!(matches!(
            refuse_to_replace_source(&source, &relative),
            Err(AppError::InvalidOutputPath)
        ));
        assert!(matches!(
            refuse_to_replace_source(&relative, &source),
            Err(AppError::InvalidOutputPath)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_hard_link_to_the_source_is_the_source() {
        let (directory, source) = folder_with_source();
        let hard = directory.path().join("hard.exr");
        fs::hard_link(&source, &hard).unwrap();
        assert!(matches!(
            refuse_to_replace_source(&source, &hard),
            Err(AppError::InvalidOutputPath)
        ));
        assert!(matches!(
            refuse_to_replace_source(&hard, &source),
            Err(AppError::InvalidOutputPath)
        ));
    }

    #[cfg(windows)]
    #[test]
    fn another_case_of_the_source_is_the_source() {
        let (directory, source) = folder_with_source();
        let shouted = directory.path().join("SCENE.EXR");
        assert!(matches!(
            refuse_to_replace_source(&source, &shouted),
            Err(AppError::InvalidOutputPath)
        ));
    }

    #[test]
    fn another_file_is_not_the_source() {
        let (directory, source) = folder_with_source();
        // Beside it, existing or not, and in another folder.
        let beside = directory.path().join("other.exr");
        fs::write(&beside, b"somebody else's").unwrap();
        let nested = directory.path().join("sub");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("scene.exr"), b"same name, other folder").unwrap();
        for other in [
            beside,
            directory.path().join("new.exr"),
            directory.path().join("scene.exr.psd"),
            nested.join("scene.exr"),
            nested.join("new.exr"),
        ] {
            assert!(
                refuse_to_replace_source(&source, &other).is_ok(),
                "{} was refused",
                other.display()
            );
        }
        // A destination that cannot be written for another reason is that export's to report.
        assert!(
            refuse_to_replace_source(&source, &directory.path().join("no/such/new.exr")).is_ok()
        );
        assert!(refuse_to_replace_source(&source, directory.path()).is_ok());
        assert!(refuse_to_replace_source(&source, Path::new("")).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_another_file_is_not_the_source() {
        let (directory, source) = folder_with_source();
        let other = directory.path().join("other.exr");
        fs::write(&other, b"somebody else's").unwrap();
        let link = directory.path().join("link.exr");
        std::os::unix::fs::symlink(&other, &link).unwrap();
        let dangling = directory.path().join("dangling.exr");
        std::os::unix::fs::symlink(directory.path().join("gone.exr"), &dangling).unwrap();
        assert!(refuse_to_replace_source(&source, &link).is_ok());
        assert!(refuse_to_replace_source(&source, &dangling).is_ok());
    }

    #[test]
    fn a_document_with_no_source_file_has_nothing_to_protect() {
        let directory = tempfile::tempdir().unwrap();
        let existing = directory.path().join("scene.exr");
        fs::write(&existing, b"somebody's file").unwrap();
        for destination in [existing, directory.path().join("new.exr")] {
            assert!(refuse_to_replace_source(Path::new(""), &destination).is_ok());
        }
    }

    #[test]
    fn a_source_that_is_gone_cannot_be_replaced() {
        let (directory, source) = folder_with_source();
        let elsewhere = directory.path().join("elsewhere.exr");
        fs::write(&elsewhere, b"another file").unwrap();
        fs::remove_file(&source).unwrap();
        assert!(refuse_to_replace_source(&source, &elsewhere).is_ok());
        // The name it had is free to be used again.
        assert!(refuse_to_replace_source(&source, &source).is_ok());
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
