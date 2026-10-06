//! Where a layer's pixels came from, when they are only part of a larger file.
//!
//! Opening a region or a reduced copy of an image is not the same act as
//! cropping or resizing one. A crop throws the rest away and forgets it ever
//! existed; the document here keeps a record of *which* file, *which* bytes of
//! it, and *which part*, so the relationship can be inspected, checked against
//! the file as it is now, and — if the file is unchanged — used to open a
//! different region.
//!
//! The pixels themselves are still stored in the project, as for any layer. The
//! origin is provenance, not a dependency: a project with a missing source opens
//! and edits exactly as one without.
use crate::error::AppError;
use crate::infrastructure::local_path::check_local_absolute;
use crate::resources::admission::SourceKind;
use crate::resources::{HARD_MAX_CANVAS_DIMENSION, HARD_MAX_SOURCE_DIMENSION};
use serde::{Deserialize, Serialize};

pub const MAX_ORIGIN_PATH_CHARS: usize = 4096;

/// A rectangle of source pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    /// One past the last column, as `u64` so a hostile `x + width` cannot wrap.
    pub fn right(&self) -> u64 {
        u64::from(self.x) + u64::from(self.width)
    }

    pub fn bottom(&self) -> u64 {
        u64::from(self.y) + u64::from(self.height)
    }

    pub fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    /// Whether the rectangle is non-empty and lies wholly inside a
    /// `width` x `height` image.
    pub fn is_within(&self, width: u32, height: u32) -> bool {
        self.width > 0
            && self.height > 0
            && self.right() <= u64::from(width)
            && self.bottom() <= u64::from(height)
    }
}

/// The file a region or reduced copy was taken from.
///
/// Identity is the SHA-256 of the file's bytes. Size is recorded too, but is
/// only a quick filter: two different files can share a size, so a match on size
/// alone is never taken as "unchanged". Timestamps are not recorded at all,
/// because they can change without the content changing and, worse, stay the
/// same when it does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceIdentity {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub kind: SourceKind,
    pub width: u32,
    pub height: u32,
}

/// Which part of the source a layer's pixels are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "view", rename_all = "camelCase")]
pub enum SourceView {
    /// Pixel-exact: the layer is this rectangle of the source, unscaled.
    #[serde(rename_all = "camelCase")]
    Region { rect: Rect },
    /// The whole source, resampled down. The layer has *less* resolution than
    /// the file and the document says so.
    #[serde(rename_all = "camelCase")]
    Reduced { width: u32, height: u32 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceOrigin {
    pub source: SourceIdentity,
    pub view: SourceView,
}

impl SourceOrigin {
    /// Checks the record is internally consistent and agrees with the pixels it
    /// claims to describe.
    ///
    /// `layer_width` and `layer_height` are the pixel layer's own dimensions. A
    /// project that says "this is a 400x300 region" over a 1000x1000 buffer is
    /// wrong about one of them, and it is refused rather than guessed at.
    pub fn validate(&self, layer_width: u32, layer_height: u32) -> Result<(), AppError> {
        let invalid = |message: &str| AppError::InvalidLayerDocument(message.to_string());
        let source = &self.source;
        if source.path.trim().is_empty() || source.path.chars().count() > MAX_ORIGIN_PATH_CHARS {
            return Err(invalid("a source path must contain 1 to 4096 characters"));
        }
        check_local_absolute(&source.path)?;
        if source.sha256.len() != 64 || !source.sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("a source digest must be 64 hexadecimal characters"));
        }
        let max = HARD_MAX_SOURCE_DIMENSION;
        if source.width == 0
            || source.height == 0
            || u64::from(source.width) > max
            || u64::from(source.height) > max
        {
            return Err(invalid("source dimensions are outside the supported range"));
        }
        match self.view {
            SourceView::Region { rect } => {
                if !rect.is_within(source.width, source.height) {
                    return Err(invalid("the region lies outside its source"));
                }
                if rect.width > HARD_MAX_CANVAS_DIMENSION || rect.height > HARD_MAX_CANVAS_DIMENSION
                {
                    return Err(invalid("the region is larger than any canvas can be"));
                }
                if (rect.width, rect.height) != (layer_width, layer_height) {
                    return Err(invalid(
                        "the region's size does not match the layer's pixels",
                    ));
                }
            }
            SourceView::Reduced { width, height } => {
                if width == 0 || height == 0 || width > source.width || height > source.height {
                    return Err(invalid("a reduced copy cannot be larger than its source"));
                }
                // Each side is the source's side times one scale, rounded, so
                // the two sides may disagree with the source's aspect by at
                // most the rounding of one pixel on each.
                let cross = u128::from(width) * u128::from(source.height);
                let other = u128::from(height) * u128::from(source.width);
                let tolerance = u128::from(source.width.max(source.height));
                if cross.abs_diff(other) > tolerance {
                    return Err(invalid(
                        "a reduced copy must keep its source's aspect ratio",
                    ));
                }
                if (width, height) != (layer_width, layer_height) {
                    return Err(invalid(
                        "the reduced size does not match the layer's pixels",
                    ));
                }
            }
        }
        Ok(())
    }

    /// The scale a reduced copy was made at, or 1.0 for a region.
    pub fn scale(&self) -> f64 {
        match self.view {
            SourceView::Region { .. } => 1.0,
            SourceView::Reduced { width, .. } => f64::from(width) / f64::from(self.source.width),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(width: u32, height: u32) -> SourceIdentity {
        SourceIdentity {
            path: "C:\\scans\\wall.png".into(),
            sha256: "a".repeat(64),
            bytes: 1234,
            kind: SourceKind::Png,
            width,
            height,
        }
    }

    fn region(x: u32, y: u32, w: u32, h: u32) -> SourceOrigin {
        SourceOrigin {
            source: identity(12_000, 9_000),
            view: SourceView::Region {
                rect: Rect {
                    x,
                    y,
                    width: w,
                    height: h,
                },
            },
        }
    }

    #[test]
    fn a_region_that_matches_its_pixels_is_valid() {
        region(100, 200, 400, 300).validate(400, 300).unwrap();
    }

    #[test]
    fn a_region_may_touch_each_edge_of_the_source() {
        region(0, 0, 100, 100).validate(100, 100).unwrap();
        region(11_900, 8_900, 100, 100).validate(100, 100).unwrap();
        region(0, 0, 12_000, 9_000).validate(12_000, 9_000).unwrap();
    }

    #[test]
    fn a_region_outside_its_source_is_refused_without_overflow() {
        for (x, y, w, h) in [
            (11_901, 0, 100, 100),
            (0, 8_901, 100, 100),
            (u32::MAX, 0, 100, 100),
            (u32::MAX, u32::MAX, u32::MAX, u32::MAX),
            (0, 0, 0, 100),
            (0, 0, 100, 0),
        ] {
            assert!(
                region(x, y, w, h).validate(w, h).is_err(),
                "accepted {x},{y} {w}x{h}"
            );
        }
    }

    /// A project that describes one size and stores another is wrong about one.
    #[test]
    fn an_origin_that_disagrees_with_the_pixels_is_refused() {
        assert!(region(0, 0, 400, 300).validate(1000, 1000).is_err());
        let reduced = SourceOrigin {
            source: identity(12_000, 9_000),
            view: SourceView::Reduced {
                width: 1_500,
                height: 1_125,
            },
        };
        reduced.validate(1_500, 1_125).unwrap();
        assert!(reduced.validate(1_500, 1_000).is_err());
    }

    #[test]
    fn a_reduced_copy_may_not_exceed_or_distort_its_source() {
        let make = |w, h| SourceOrigin {
            source: identity(12_000, 9_000),
            view: SourceView::Reduced {
                width: w,
                height: h,
            },
        };
        assert!(
            make(13_000, 9_750).validate(13_000, 9_750).is_err(),
            "upscaled"
        );
        assert!(make(1_500, 500).validate(1_500, 500).is_err(), "distorted");
        // Rounding of one pixel on a side is allowed.
        make(1_501, 1_125).validate(1_501, 1_125).unwrap();
        assert!((make(1_500, 1_125).scale() - 0.125).abs() < 1e-9);
    }

    #[test]
    fn a_hostile_identity_is_refused() {
        let mut origin = region(0, 0, 10, 10);
        for path in [
            "",
            "relative.png",
            "C:\\a\\..\\b.png",
            "\\\\host\\share\\x.png",
        ] {
            origin.source.path = path.into();
            assert!(origin.validate(10, 10).is_err(), "accepted {path:?}");
        }
        origin.source.path = "C:\\ok.png".into();
        origin.source.sha256 = "zz".repeat(32);
        assert!(origin.validate(10, 10).is_err());
        origin.source.sha256 = "a".repeat(63);
        assert!(origin.validate(10, 10).is_err());
        origin.source.sha256 = "a".repeat(64);
        origin.source.width = 0;
        assert!(origin.validate(10, 10).is_err());
    }

    #[test]
    fn the_record_round_trips_through_json_unchanged() {
        let origin = region(5, 6, 7, 8);
        let json = serde_json::to_string(&origin).unwrap();
        assert!(json.contains("\"view\":\"region\""), "{json}");
        let back: SourceOrigin = serde_json::from_str(&json).unwrap();
        assert_eq!(back, origin);
    }
}
