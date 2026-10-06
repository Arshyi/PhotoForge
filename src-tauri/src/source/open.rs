//! Opening a region, a reduced copy, or a bounded preview of a source.
//!
//! Each of these is gated by the same planner as a whole-file open, so the answer
//! to "may I open this part?" is the one the interface was shown, not a second
//! opinion that could disagree at the boundary.
//!
//! # The file must not change underneath a read
//!
//! A source is identified by the SHA-256 of its bytes, taken before decoding. If
//! the file were replaced between the hash and the decode, the stored identity
//! would describe one file and the pixels another. So the file's length and
//! modification time are sampled before and after, and a difference is an error.
//! Timestamps are used here only as a race detector; they are never part of the
//! identity, because they can change without the content changing and stay
//! put when it does.
use super::model::{Rect, SourceIdentity, SourceOrigin, SourceView};
use super::probe::{image_format_of, probe_path, report, Probed, HARD_MAX_SOURCE_FILE_BYTES};
use super::{dng, jpeg, png, webp};
use crate::color::FloatImage;
use crate::error::AppError;
use crate::infrastructure::image_io::{
    assemble_loaded, assemble_reduced, encode_preview, LoadedImage, SourceTraits,
};
use crate::infrastructure::local_path::{check_local_absolute, hash_file};
use crate::resources::admission::{AdmissionOption, AdmissionReport, SourceKind};
use image::DynamicImage;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;

/// What part of a source to open.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OpenSelection {
    Region(Rect),
    /// The whole source resampled to this size.
    Reduced {
        width: u32,
        height: u32,
    },
}

/// The size a reduced copy at `scale` has: each side the source's side times the
/// scale, rounded, and never below one pixel.
pub fn reduced_dimensions(source_width: u32, source_height: u32, scale: f64) -> (u32, u32) {
    let side = |length: u32| ((f64::from(length) * scale).round() as u32).clamp(1, length);
    (side(source_width), side(source_height))
}

/// A file's length and modification time, to detect it changing mid-read.
fn fingerprint(path: &Path) -> Result<(u64, Option<SystemTime>), AppError> {
    let metadata = fs::metadata(path).map_err(|_| AppError::DecodeFailure)?;
    Ok((metadata.len(), metadata.modified().ok()))
}

/// The path that will be stored for a source: absolute as supplied, not
/// canonical, so that a mapped network drive keeps its letter rather than turning
/// into a UNC path that a stored origin could not hold.
fn storable_path(path: &Path) -> Result<String, AppError> {
    let absolute = std::path::absolute(path).map_err(|_| {
        AppError::InvalidOperation("the file's path could not be made absolute".into())
    })?;
    let text = absolute.to_string_lossy().into_owned();
    check_local_absolute(&text).map_err(|_| {
        AppError::InvalidOperation(
            "a region or reduced copy can only keep its source relationship for a file on a \
             local drive or a mapped drive letter, not a network path"
                .into(),
        )
    })?;
    Ok(text)
}

fn identity_of(probed: &Probed, stored_path: String, sha256: String) -> SourceIdentity {
    SourceIdentity {
        path: stored_path,
        sha256,
        bytes: probed.file_bytes,
        kind: probed.kind,
        width: probed.width,
        height: probed.height,
    }
}

/// Refuses a region the planner would not have offered.
///
/// This is the same check whatever the format, so a camera RAW and a PNG are held
/// to one rule: the rectangle lies on the source, and it is no larger than the
/// planner's own answer for this source on this machine.
pub fn gate_region(probed: &Probed, planned: &AdmissionReport, rect: Rect) -> Result<(), AppError> {
    if !rect.is_within(probed.width, probed.height) {
        return Err(AppError::InvalidOperation(
            "the region lies outside the image".into(),
        ));
    }
    let allowed = planned.options.iter().find_map(|option| match option {
        AdmissionOption::OpenRegion {
            max_region_pixels, ..
        } => Some(*max_region_pixels),
        _ => None,
    });
    // A source that opens whole may also be opened by region: the planner
    // offers OpenFull, not OpenRegion, in that case, so the whole-file
    // ceiling applies.
    let limit = allowed.unwrap_or_else(crate::resources::max_working_pixels);
    if allowed.is_none()
        && !planned
            .options
            .iter()
            .any(|option| matches!(option, AdmissionOption::OpenFull { .. }))
    {
        return Err(AppError::ImageTooLarge {
            pixels: rect.pixels(),
            limit,
        });
    }
    if rect.pixels() > limit {
        return Err(AppError::ImageTooLarge {
            pixels: rect.pixels(),
            limit,
        });
    }
    Ok(())
}

/// Opens the part of `path` named by `selection` as a document-ready image, with
/// its origin recorded.
pub fn open_selection(
    path: &Path,
    selection: OpenSelection,
    resident: u64,
    cancel: Option<&AtomicBool>,
) -> Result<LoadedImage, AppError> {
    let probed = probe_path(path)?;
    let planned = report(&probed, resident);
    let stored_path = storable_path(path)?;

    // Gate on the same report the interface showed.
    match selection {
        OpenSelection::Region(rect) => gate_region(&probed, &planned, rect)?,
        OpenSelection::Reduced { width, height } => {
            if width == 0 || height == 0 || width > probed.width || height > probed.height {
                return Err(AppError::InvalidOperation(
                    "a reduced copy must be no larger than its source".into(),
                ));
            }
            let scale = f64::from(width) / f64::from(probed.width);
            let max_scale = planned.options.iter().find_map(|option| match option {
                AdmissionOption::OpenReduced { max_scale, .. } => Some(*max_scale),
                _ => None,
            });
            // Rounding of each side may put the requested scale a hair above the
            // boundary the planner found; allow one pixel's worth.
            let slack = 1.0 / f64::from(probed.width);
            if max_scale.is_none_or(|limit| scale > limit + slack) {
                return Err(AppError::ImageTooLarge {
                    pixels: u64::from(width) * u64::from(height),
                    limit: crate::resources::max_working_pixels(),
                });
            }
        }
    }

    let before = fingerprint(&probed.path)?;
    let (sha256, bytes) = hash_file(&probed.path, HARD_MAX_SOURCE_FILE_BYTES)?;
    if bytes != probed.file_bytes {
        return Err(AppError::ProjectIo(
            "the file changed while it was being read".into(),
        ));
    }
    let metadata = fs::metadata(&probed.path).map_err(|_| AppError::DecodeFailure)?;
    let format = image_format_of(probed.kind);

    let loaded = match selection {
        OpenSelection::Region(rect) => {
            let (decoded, icc): (DynamicImage, Option<Vec<u8>>) = match probed.kind {
                SourceKind::Png => {
                    let r = png::decode_region(&probed.path, rect, cancel)?;
                    (r.image, r.icc)
                }
                SourceKind::Jpeg => {
                    let r = jpeg::decode_region(&probed.path, rect, cancel)?;
                    (r.image, r.icc)
                }
                SourceKind::WebP => {
                    let r = webp::decode_region(&probed.path, rect, cancel)?;
                    (r.image, r.icc)
                }
                SourceKind::Dng => return Err(AppError::UnsupportedImageFormat),
            };
            let origin = SourceOrigin {
                source: identity_of(&probed, stored_path, sha256),
                view: SourceView::Region { rect },
            };
            assemble_loaded(
                probed.path.clone(),
                &metadata,
                format,
                decoded,
                icc,
                Some(origin),
            )?
        }
        OpenSelection::Reduced { width, height } => {
            let reduced: FloatImage = match probed.kind {
                SourceKind::Png => png::decode_reduced(&probed.path, width, height, cancel)?,
                SourceKind::Jpeg => jpeg::decode_reduced(&probed.path, width, height, cancel)?,
                SourceKind::WebP => webp::decode_reduced(&probed.path, width, height, cancel)?,
                SourceKind::Dng => return Err(AppError::UnsupportedImageFormat),
            };
            let origin = SourceOrigin {
                source: identity_of(&probed, stored_path, sha256),
                view: SourceView::Reduced { width, height },
            };
            assemble_reduced(
                probed.path.clone(),
                &metadata,
                format,
                reduced,
                SourceTraits {
                    had_icc: probed.has_icc,
                    bit_depth: probed.bit_depth,
                    has_alpha: probed.has_alpha,
                },
                origin,
            )?
        }
    };

    // If the file moved under the read, what was hashed is not what was decoded.
    if fingerprint(&probed.path)? != before {
        return Err(AppError::ProjectIo(
            "the file changed while it was being read".into(),
        ));
    }
    Ok(loaded)
}

/// A bounded picture of a whole source, for choosing a region on.
pub struct SourcePreview {
    pub data_url: String,
    pub width: u32,
    pub height: u32,
}

/// The longest edge a preview may have.
pub const MAX_PREVIEW_EDGE: u32 = 2048;

/// Builds a preview of the whole source no larger than `max_edge` on a side.
///
/// It uses the cheapest decode the format offers — row streaming for PNG, DCT
/// scaling for JPEG — and is gated by the planner like any other open, so the
/// preview itself can never be the thing that exhausts memory. If the planner
/// cannot offer a reduced decode within budget, there is no preview, and the
/// caller says so rather than falling back to decoding the source whole.
pub fn source_preview(
    path: &Path,
    max_edge: u32,
    cancel: Option<&AtomicBool>,
) -> Result<SourcePreview, AppError> {
    let probed = probe_path(path)?;
    let edge = max_edge.clamp(64, MAX_PREVIEW_EDGE);
    let longest = probed.width.max(probed.height);
    let scale = (f64::from(edge) / f64::from(longest)).min(1.0);
    let (width, height) = reduced_dimensions(probed.width, probed.height, scale);

    // A DNG's preview is not a reduced *document*, so the reduced-copy option does
    // not gate it. It is built a segment at a time and is as large as the picture
    // and one segment, which the region the user will then choose has to fit in
    // anyway; developing it still passes through the job-bytes check.
    if probed.kind == SourceKind::Dng {
        let picture = dng::preview(&probed.path, edge, cancel)?;
        let (width, height) = (picture.width(), picture.height());
        return Ok(SourcePreview {
            data_url: encode_preview(&DynamicImage::ImageRgba8(picture.to_rgba8()))?,
            width,
            height,
        });
    }

    let planned = report(&probed, 0);
    let max_scale = planned.options.iter().find_map(|option| match option {
        AdmissionOption::OpenReduced { max_scale, .. } => Some(*max_scale),
        _ => None,
    });
    // A source small enough to open whole has no reduced option when it is
    // small enough that "reducing" it is moot; a preview of it is just itself.
    let whole = planned
        .options
        .iter()
        .any(|option| matches!(option, AdmissionOption::OpenFull { .. }));
    if !whole && max_scale.is_none_or(|limit| scale > limit) {
        return Err(AppError::OutOfMemoryRisk);
    }

    let reduced = match probed.kind {
        SourceKind::Png => png::decode_reduced(&probed.path, width, height, cancel)?,
        SourceKind::Jpeg => jpeg::decode_reduced(&probed.path, width, height, cancel)?,
        SourceKind::WebP => webp::decode_reduced(&probed.path, width, height, cancel)?,
        SourceKind::Dng => return Err(AppError::UnsupportedImageFormat),
    };
    let data_url = encode_preview(&DynamicImage::ImageRgba8(reduced.to_rgba8()))?;
    Ok(SourcePreview {
        data_url,
        width,
        height,
    })
}

/// Whether the file at `origin`'s path is still the one the origin describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OriginState {
    /// Present, and its bytes hash to the recorded digest.
    Available,
    /// The path does not exist or cannot be read.
    Missing,
    /// Present, but its bytes hash to something else.
    Changed,
}

/// Checks `origin` against the file as it is now, by hashing it again.
///
/// Never by timestamp or size alone: either can match while the content differs.
pub fn check_origin(origin: &SourceOrigin) -> Result<OriginState, AppError> {
    let path: PathBuf =
        crate::infrastructure::local_path::validated_local_path(&origin.source.path)?;
    if !path.is_file() {
        return Ok(OriginState::Missing);
    }
    match hash_file(&path, HARD_MAX_SOURCE_FILE_BYTES) {
        Ok((digest, _)) if digest.eq_ignore_ascii_case(&origin.source.sha256) => {
            Ok(OriginState::Available)
        }
        Ok(_) => Ok(OriginState::Changed),
        Err(_) => Ok(OriginState::Missing),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use std::io::BufWriter;

    fn write_png(path: &Path, width: u32, height: u32) {
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = ::png::Encoder::new(BufWriter::new(file), width, height);
        encoder.set_color(::png::ColorType::Rgb);
        encoder.set_depth(::png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let mut data = vec![0u8; (width * height * 3) as usize];
        for y in 0..height {
            for x in 0..width {
                let i = ((y * width + x) * 3) as usize;
                data[i] = (x % 251) as u8;
                data[i + 1] = (y % 241) as u8;
                data[i + 2] = ((x + y) % 239) as u8;
            }
        }
        writer.write_image_data(&data).unwrap();
    }

    #[test]
    fn reduced_dimensions_round_each_side_and_never_vanish() {
        assert_eq!(reduced_dimensions(1000, 500, 0.5), (500, 250));
        assert_eq!(reduced_dimensions(1001, 501, 0.5), (501, 251));
        assert_eq!(reduced_dimensions(10, 10, 0.001), (1, 1));
        assert_eq!(reduced_dimensions(10, 10, 1.0), (10, 10));
    }

    /// The whole point: a region open gives the pixels of that area of the source,
    /// carries the right origin, and the origin validates against the pixels.
    #[test]
    fn a_region_open_matches_the_source_and_records_where_it_came_from() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        write_png(&path, 120, 90);
        let rect = Rect {
            x: 20,
            y: 10,
            width: 50,
            height: 40,
        };

        let loaded = open_selection(&path, OpenSelection::Region(rect), 0, None).unwrap();
        assert_eq!(loaded.original.dimensions(), (50, 40));
        assert_eq!((loaded.metadata.width, loaded.metadata.height), (50, 40));

        let full = image::ImageReader::open(&path).unwrap().decode().unwrap();
        let want = full.crop_imm(rect.x, rect.y, rect.width, rect.height);
        assert_eq!(loaded.original.as_bytes(), want.as_bytes());

        let origin = loaded
            .metadata
            .origin
            .as_ref()
            .expect("an origin was recorded");
        assert_eq!(origin.view, SourceView::Region { rect });
        assert_eq!((origin.source.width, origin.source.height), (120, 90));
        assert_eq!(origin.source.kind, SourceKind::Png);
        origin
            .validate(50, 40)
            .expect("the origin disagrees with its own pixels");
        let (digest, bytes) = hash_file(&path, 1 << 30).unwrap();
        assert_eq!(origin.source.sha256, digest);
        assert_eq!(origin.source.bytes, bytes);
        // The stored path must be one a project may hold.
        check_local_absolute(&origin.source.path).unwrap();
    }

    #[test]
    fn a_reduced_open_has_the_requested_size_and_an_origin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        write_png(&path, 200, 100);
        let loaded = open_selection(
            &path,
            OpenSelection::Reduced {
                width: 50,
                height: 25,
            },
            0,
            None,
        )
        .unwrap();
        assert_eq!(loaded.original.dimensions(), (50, 25));
        let working = loaded
            .working
            .as_ref()
            .expect("a reduced copy is linear float");
        assert_eq!(working.dimensions(), (50, 25));
        let origin = loaded.metadata.origin.as_ref().unwrap();
        assert_eq!(
            origin.view,
            SourceView::Reduced {
                width: 50,
                height: 25
            }
        );
        origin.validate(50, 25).unwrap();
        assert!((origin.scale() - 0.25).abs() < 1e-9);
    }

    #[test]
    fn a_region_or_reduction_that_does_not_fit_the_source_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        write_png(&path, 60, 40);
        for rect in [
            Rect {
                x: 50,
                y: 0,
                width: 20,
                height: 10,
            },
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 10,
            },
            Rect {
                x: u32::MAX,
                y: u32::MAX,
                width: u32::MAX,
                height: u32::MAX,
            },
        ] {
            assert!(
                open_selection(&path, OpenSelection::Region(rect), 0, None).is_err(),
                "{rect:?}"
            );
        }
        for (w, h) in [(120, 80), (0, 5), (61, 40)] {
            assert!(
                open_selection(
                    &path,
                    OpenSelection::Reduced {
                        width: w,
                        height: h
                    },
                    0,
                    None
                )
                .is_err(),
                "{w}x{h}"
            );
        }
    }

    /// The whole oversized-image story, end to end, at the default budget: a 70 MP
    /// PNG is too big to open whole, the planner offers a region, and the region
    /// opens with exactly the pixels it should.
    #[test]
    fn an_oversized_png_is_refused_whole_and_opens_by_region() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.png");
        // 8,400 x 8,400 = 70.6 MP, over the 67.1 MP default ceiling, and flat so
        // the file stays small.
        let (width, height) = (8_400u32, 8_400u32);
        {
            let file = std::fs::File::create(&path).unwrap();
            let mut encoder = ::png::Encoder::new(BufWriter::new(file), width, height);
            encoder.set_color(::png::ColorType::Rgb);
            encoder.set_depth(::png::BitDepth::Eight);
            // Encoding 70 MP is the cost of this test, so spend as little as
            // possible on compressing it; decoding is unaffected.
            encoder.set_compression(::png::Compression::Fast);
            encoder.set_filter(::png::Filter::NoFilter);
            let mut writer = encoder.write_header().unwrap();
            let mut stream = writer.stream_writer().unwrap();
            // Only 241 distinct rows exist (the green channel is `y % 241`), so
            // build each once rather than filling 70 million pixels one at a time.
            let rows: Vec<Vec<u8>> = (0..241u32)
                .map(|g| {
                    let mut row = vec![0u8; width as usize * 3];
                    for (x, pixel) in row.chunks_exact_mut(3).enumerate() {
                        pixel[0] = (x % 251) as u8;
                        pixel[1] = g as u8;
                        pixel[2] = 77;
                    }
                    row
                })
                .collect();
            for y in 0..height {
                std::io::Write::write_all(&mut stream, &rows[(y % 241) as usize]).unwrap();
            }
            stream.finish().unwrap();
        }

        // Whole-file open is refused...
        let probed = probe_path(&path).unwrap();
        assert!(crate::source::probe::require_full_admission(&probed).is_err());
        let planned = report(&probed, 0);
        assert_eq!(
            planned.verdict,
            crate::resources::admission::Verdict::RegionRequired
        );
        let max_region = planned
            .options
            .iter()
            .find_map(|o| match o {
                AdmissionOption::OpenRegion {
                    max_region_pixels, ..
                } => Some(*max_region_pixels),
                _ => None,
            })
            .expect("a region is on offer");
        assert!(
            max_region >= 4_000_000,
            "the offered region is only {max_region} pixels"
        );

        // ...and a region of it opens, with the right pixels.
        let rect = Rect {
            x: 4_000,
            y: 5_000,
            width: 1_500,
            height: 1_200,
        };
        let loaded = open_selection(&path, OpenSelection::Region(rect), 0, None).unwrap();
        assert_eq!(loaded.original.dimensions(), (1_500, 1_200));
        let rgb = loaded.original.to_rgb8();
        for &(dx, dy) in &[(0u32, 0u32), (1_499, 1_199), (700, 500)] {
            let (x, y) = (rect.x + dx, rect.y + dy);
            assert_eq!(
                rgb.get_pixel(dx, dy).0,
                [(x % 251) as u8, (y % 241) as u8, 77]
            );
        }
        // A region larger than the offered maximum is refused even though it is
        // inside the image.
        let too_big = Rect {
            x: 0,
            y: 0,
            width: 8_400,
            height: 8_000,
        };
        assert!(open_selection(&path, OpenSelection::Region(too_big), 0, None).is_err());
    }

    #[test]
    fn the_preview_is_bounded_and_keeps_the_aspect_ratio() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.png");
        write_png(&path, 600, 200);
        let preview = source_preview(&path, 150, None).unwrap();
        assert_eq!((preview.width, preview.height), (150, 50));
        assert!(preview.data_url.starts_with("data:image/"));
        // A source smaller than the request is not enlarged.
        let small = source_preview(&path, 2048, None).unwrap();
        assert_eq!((small.width, small.height), (600, 200));
    }

    #[test]
    fn an_origin_is_available_missing_or_changed_by_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        write_png(&path, 30, 30);
        let rect = Rect {
            x: 0,
            y: 0,
            width: 10,
            height: 10,
        };
        let loaded = open_selection(&path, OpenSelection::Region(rect), 0, None).unwrap();
        let origin = loaded.metadata.origin.unwrap();
        assert_eq!(check_origin(&origin).unwrap(), OriginState::Available);

        // Rewriting the file with different content changes the digest even though
        // the dimensions and name are the same.
        write_png(&path, 30, 31);
        assert_eq!(check_origin(&origin).unwrap(), OriginState::Changed);

        std::fs::remove_file(&path).unwrap();
        assert_eq!(check_origin(&origin).unwrap(), OriginState::Missing);
    }

    /// Identical size and timestamps would not change the digest, which is why
    /// identity is the digest.
    #[test]
    fn content_not_size_decides_whether_a_source_changed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        write_png(&path, 30, 30);
        let origin = open_selection(
            &path,
            OpenSelection::Region(Rect {
                x: 0,
                y: 0,
                width: 5,
                height: 5,
            }),
            0,
            None,
        )
        .unwrap()
        .metadata
        .origin
        .unwrap();
        let mut bytes = std::fs::read(&path).unwrap();
        // Flip one bit in the middle of the file: same length, different content.
        let middle = bytes.len() / 2;
        bytes[middle] ^= 1;
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), origin.source.bytes);
        assert_eq!(check_origin(&origin).unwrap(), OriginState::Changed);
    }
}
