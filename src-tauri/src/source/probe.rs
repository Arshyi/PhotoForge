//! Reading a file's header and deciding, before any pixel is decoded, whether it
//! can be opened and how.
//!
//! This is the front door for every image open. Nothing here allocates in
//! proportion to the image: the headers of all three formats are read without
//! decoding a sample, so a hostile claim of a million pixels a side is learned
//! about, and refused, for the price of reading a few hundred bytes.
use super::{jpeg, png, webp};
use crate::error::AppError;
use crate::resources::admission::{
    plan, AdmissionReport, DecodeCapabilities, ReducedDecode, Refusal, RegionDecode, Shortfall,
    SourceKind, SourceProbe, Verdict,
};
use crate::resources::memory::{MemoryProbe, OsProbe};
use crate::resources::policy::GIB;
use crate::resources::{self};
use image::{ImageFormat, ImageReader};
use std::fs;
use std::path::{Path, PathBuf};

/// A file larger than this is not a photograph PhotoForge is going to read. The
/// planner, not this number, decides what fits; this only stops absurd inputs
/// from reaching it.
pub const HARD_MAX_SOURCE_FILE_BYTES: u64 = 32 * GIB;

/// Everything learned from a header.
#[derive(Debug, Clone)]
pub struct Probed {
    /// The canonical path. Used to open the file; not the path that is stored,
    /// because on Windows canonicalising can turn a mapped drive into a UNC path.
    pub path: PathBuf,
    pub file_bytes: u64,
    pub kind: SourceKind,
    pub width: u32,
    pub height: u32,
    pub has_icc: bool,
    pub bit_depth: u8,
    pub has_alpha: bool,
    pub probe: SourceProbe,
}

pub fn image_format_of(kind: SourceKind) -> ImageFormat {
    match kind {
        SourceKind::Png => ImageFormat::Png,
        SourceKind::Jpeg => ImageFormat::Jpeg,
        SourceKind::WebP => ImageFormat::WebP,
        // A DNG is read by the RAW pipeline, which has its own entry points.
        SourceKind::Dng => ImageFormat::Tiff,
    }
}

/// Reads the header of the file at `path`.
pub fn probe_path(path: &Path) -> Result<Probed, AppError> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            AppError::Permission
        } else {
            AppError::DecodeFailure
        }
    })?;
    let metadata = fs::metadata(&canonical).map_err(|_| AppError::DecodeFailure)?;
    if !metadata.is_file() {
        return Err(AppError::UnsupportedImageFormat);
    }
    if metadata.len() > HARD_MAX_SOURCE_FILE_BYTES {
        return Err(AppError::OutOfMemoryRisk);
    }
    let format = ImageReader::open(&canonical)
        .map_err(|_| AppError::DecodeFailure)?
        .with_guessed_format()
        .map_err(|_| AppError::DecodeFailure)?
        .format()
        .ok_or(AppError::UnsupportedImageFormat)?;

    let (kind, width, height, native_bpp, has_icc, bit_depth, has_alpha, capabilities) =
        match format {
            ImageFormat::Png => {
                let info = png::read_info(&canonical)?;
                let bpp = info.layout.bytes_per_pixel() as u64;
                // Row streaming works for a non-interlaced file only. Adam7 rows
                // arrive in pass order, so the frame is assembled first.
                let (region, reduced) = if info.interlaced {
                    (
                        RegionDecode::TransientFull {
                            bytes_per_pixel: bpp,
                        },
                        ReducedDecode::TransientFull {
                            bytes_per_pixel: bpp,
                        },
                    )
                } else {
                    (RegionDecode::Rows, ReducedDecode::Rows)
                };
                (
                    SourceKind::Png,
                    info.width,
                    info.height,
                    bpp,
                    info.icc.is_some(),
                    if info.layout.is_16_bit() { 16 } else { 8 },
                    info.layout.has_alpha(),
                    DecodeCapabilities { region, reduced },
                )
            }
            ImageFormat::Jpeg => {
                let info = jpeg::read_info(&canonical)?;
                (
                    SourceKind::Jpeg,
                    info.width,
                    info.height,
                    jpeg::WHOLE_FRAME_BYTES_PER_PIXEL,
                    info.icc.is_some(),
                    8,
                    false,
                    DecodeCapabilities {
                        region: RegionDecode::TransientFull {
                            bytes_per_pixel: jpeg::WHOLE_FRAME_BYTES_PER_PIXEL,
                        },
                        reduced: ReducedDecode::DctScaled,
                    },
                )
            }
            ImageFormat::WebP => {
                let info = webp::read_info(&canonical)?;
                let bpp = if info.has_alpha { 4 } else { 3 };
                (
                    SourceKind::WebP,
                    info.width,
                    info.height,
                    bpp,
                    info.icc.is_some(),
                    8,
                    info.has_alpha,
                    DecodeCapabilities {
                        region: RegionDecode::TransientFull {
                            bytes_per_pixel: bpp,
                        },
                        reduced: ReducedDecode::TransientFull {
                            bytes_per_pixel: bpp,
                        },
                    },
                )
            }
            _ => return Err(AppError::UnsupportedImageFormat),
        };

    let probe = SourceProbe {
        kind,
        width: u64::from(width),
        height: u64::from(height),
        native_bytes_per_pixel: native_bpp,
        file_bytes: metadata.len(),
        capabilities,
    };
    Ok(Probed {
        path: canonical,
        file_bytes: metadata.len(),
        kind,
        width,
        height,
        has_icc,
        bit_depth,
        has_alpha,
        probe,
    })
}

/// Plans against an explicit budget and limits, and the memory free right now.
pub fn report_with(
    probed: &Probed,
    resident: u64,
    budget: &crate::resources::Budget,
    limits: &crate::resources::ResourceLimits,
) -> AdmissionReport {
    let available = OsProbe.system().map(|system| system.available_physical);
    plan(&probed.probe, budget, limits, available, resident)
}

/// Plans against the budget in force and the memory free right now.
///
/// `resident` is what the session already holds and will keep; pass zero when the
/// new source replaces the open document.
pub fn report(probed: &Probed, resident: u64) -> AdmissionReport {
    let budget = resources::budget();
    let limits = resources::limits();
    let available = OsProbe.system().map(|system| system.available_physical);
    plan(&probed.probe, &budget, &limits, available, resident)
}

/// Succeeds only if the source can be opened whole as a conventional document.
///
/// The error says what kind of refusal it was, because "too big", "damaged" and
/// "not enough free memory at the moment" call for different responses.
pub fn require_full_admission(probed: &Probed) -> Result<(), AppError> {
    let report = report(probed, 0);
    let pixels = probed.probe.pixels().min(u128::from(u64::MAX)) as u64;
    match report.verdict {
        Verdict::FullResolution | Verdict::FullResolutionWithWarning { .. } => Ok(()),
        Verdict::Unsafe {
            refusal: Refusal::ImpossibleExpansion { .. } | Refusal::Empty,
        } => Err(AppError::CorruptImage),
        Verdict::InsufficientResources {
            shortfall: Shortfall::Momentary,
        } => Err(AppError::OutOfMemoryRisk),
        Verdict::Unsafe { .. }
        | Verdict::RegionRequired
        | Verdict::ReducedCopyRecommended
        | Verdict::InsufficientResources { .. } => Err(AppError::ImageTooLarge {
            pixels,
            limit: resources::max_working_pixels(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufWriter;

    fn write_png(path: &Path, width: u32, height: u32) {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = ::png::Encoder::new(BufWriter::new(file), width, height);
        encoder.set_color(::png::ColorType::Rgb);
        encoder.set_depth(::png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer
            .write_image_data(&vec![120u8; (width * height * 3) as usize])
            .unwrap();
    }

    #[test]
    fn a_png_header_becomes_a_streaming_probe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        write_png(&path, 64, 48);
        let probed = probe_path(&path).unwrap();
        assert_eq!(probed.kind, SourceKind::Png);
        assert_eq!((probed.width, probed.height), (64, 48));
        assert_eq!(probed.probe.capabilities.region, RegionDecode::Rows);
        assert_eq!(probed.probe.capabilities.reduced, ReducedDecode::Rows);
        assert_eq!(probed.probe.native_bytes_per_pixel, 3);
    }

    /// JPEG has no region decode and the probe must say so rather than flatter it.
    #[test]
    fn a_jpeg_is_priced_as_a_whole_frame_region_and_a_dct_scaled_reduction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.jpg");
        image::RgbImage::new(32, 32).save(&path).unwrap();
        let probed = probe_path(&path).unwrap();
        assert_eq!(probed.kind, SourceKind::Jpeg);
        assert!(matches!(
            probed.probe.capabilities.region,
            RegionDecode::TransientFull { bytes_per_pixel: 3 }
        ));
        assert_eq!(probed.probe.capabilities.reduced, ReducedDecode::DctScaled);
    }

    #[test]
    fn a_normal_image_is_admitted_whole() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        write_png(&path, 640, 480);
        let probed = probe_path(&path).unwrap();
        require_full_admission(&probed).unwrap();
    }

    #[test]
    fn a_file_that_is_not_an_image_is_refused_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "hello").unwrap();
        assert!(probe_path(&path).is_err());
        assert!(probe_path(&dir.path().join("missing.png")).is_err());
        assert!(probe_path(dir.path()).is_err(), "a directory");
    }

    /// Case 4: a header that claims an absurd size is refused from the header
    /// alone, with nothing allocated and no pixel decoded.
    #[test]
    fn a_hostile_claimed_size_is_refused_from_the_header_alone() {
        // 100,000 x 100,000 RGB in a file of a few hundred bytes.
        fn crc32(bytes: &[u8]) -> u32 {
            let mut crc = !0u32;
            for byte in bytes {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        let mut file = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&100_000u32.to_be_bytes());
        ihdr.extend_from_slice(&100_000u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        // An IDAT must be present: the decoder reads headers up to the first one.
        let idat = [0x78u8, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01];
        for (kind, data) in [
            (b"IHDR", ihdr.as_slice()),
            (b"IDAT", &idat[..]),
            (b"IEND", &[][..]),
        ] {
            file.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend_from_slice(data);
            file.extend_from_slice(&body);
            file.extend_from_slice(&crc32(&body).to_be_bytes());
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bomb.png");
        std::fs::write(&path, &file).unwrap();

        let started = std::time::Instant::now();
        let probed = probe_path(&path).unwrap();
        assert_eq!((probed.width, probed.height), (100_000, 100_000));
        assert!(started.elapsed().as_millis() < 500);
        // 30 GB of raster in a 60-byte file is not a PNG.
        assert!(matches!(
            report(&probed, 0).verdict,
            Verdict::Unsafe {
                refusal: Refusal::ImpossibleExpansion { .. }
            }
        ));
        assert!(require_full_admission(&probed).is_err());
    }
}
