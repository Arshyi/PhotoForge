use super::composite::PixelSource;
use super::model::validate_dimensions;
use crate::color::FloatImage;
use crate::error::AppError;
use crate::pixel::PixelBuffer;
use image::{imageops, RgbaImage};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;

/// Longest edge a preview buffer may have, matching the existing preview
/// ceiling in `infrastructure::image_io` so a layered preview and a flat
/// preview are the same size.
pub const PREVIEW_MAX_DIMENSION: u32 = 1_600;
/// Total full-resolution pixel memory one document's layers may hold. Previews
/// are counted too. Exceeding it fails the edit rather than the process.
pub const MAX_STORE_BYTES: u64 = 1_073_741_824;
/// Largest number of distinct pixel buffers a document may hold.
pub const MAX_STORE_BUFFERS: usize = 1_024;

struct StoredPixels {
    full: PixelBuffer,
    preview: PixelBuffer,
    full_fingerprint: [u8; 32],
    preview_fingerprint: [u8; 32],
}

impl StoredPixels {
    fn bytes(&self) -> u64 {
        let shared = match (&self.full, &self.preview) {
            (PixelBuffer::EncodedSrgba8(a), PixelBuffer::EncodedSrgba8(b)) => Arc::ptr_eq(a, b),
            (PixelBuffer::LinearRgbaF32(a), PixelBuffer::LinearRgbaF32(b)) => Arc::ptr_eq(a, b),
            _ => false,
        };
        self.full.bytes() + if shared { 0 } else { self.preview.bytes() }
    }
}

/// Immutable pixel buffers for the open layered document.
///
/// Buffers are content-independent and never mutated in place, so several
/// layers may share one identifier. Duplicating a layer therefore costs one
/// `Arc` clone rather than a pixel copy, and any edit that changes pixels
/// registers a new buffer instead of overwriting the old one — the copy-on-write
/// discipline that keeps undo cheap.
pub struct LayerPixelStore {
    buffers: HashMap<String, StoredPixels>,
    next_pixel: u64,
    canvas_width: u32,
    canvas_height: u32,
    preview_width: u32,
    preview_height: u32,
}

impl Default for LayerPixelStore {
    fn default() -> Self {
        Self {
            buffers: HashMap::new(),
            next_pixel: 0,
            canvas_width: 1,
            canvas_height: 1,
            preview_width: 1,
            preview_height: 1,
        }
    }
}

/// Preview dimensions for a canvas, preserving aspect ratio within the preview
/// ceiling.
pub fn preview_dimensions(width: u32, height: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= PREVIEW_MAX_DIMENSION || longest == 0 {
        return (width.max(1), height.max(1));
    }
    let scale = f64::from(PREVIEW_MAX_DIMENSION) / f64::from(longest);
    (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    )
}

impl LayerPixelStore {
    /// Drops every buffer and rebinds the store to a new canvas. Opening an
    /// image or loading a project always resets, so buffers never leak between
    /// documents.
    pub fn reset(&mut self, canvas_width: u32, canvas_height: u32) -> Result<(), AppError> {
        validate_dimensions(canvas_width, canvas_height)?;
        let (preview_width, preview_height) = preview_dimensions(canvas_width, canvas_height);
        self.buffers.clear();
        self.next_pixel = 0;
        self.canvas_width = canvas_width;
        self.canvas_height = canvas_height;
        self.preview_width = preview_width;
        self.preview_height = preview_height;
        Ok(())
    }

    pub fn canvas(&self) -> (u32, u32) {
        (self.canvas_width, self.canvas_height)
    }

    /// Ratio between preview buffers and full-resolution buffers. The
    /// compositor scales layer translations by exactly this value so a preview
    /// is a faithful miniature of the export.
    pub fn preview_scale(&self) -> f64 {
        f64::from(self.preview_width) / f64::from(self.canvas_width.max(1))
    }

    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    pub fn total_bytes(&self) -> u64 {
        self.buffers.values().map(StoredPixels::bytes).sum()
    }

    pub fn contains(&self, pixel_id: &str) -> bool {
        self.buffers.contains_key(pixel_id)
    }

    fn next_identifier(&mut self) -> String {
        self.next_pixel = self.next_pixel.saturating_add(1);
        format!("px{}", self.next_pixel)
    }

    fn scaled_preview(&self, image: &PixelBuffer) -> Result<PixelBuffer, AppError> {
        let scale = self.preview_scale();
        if scale >= 1.0 {
            return Ok(image.clone());
        }
        let dimensions = image.dimensions();
        let width = ((f64::from(dimensions.0) * scale).round() as u32).max(1);
        let height = ((f64::from(dimensions.1) * scale).round() as u32).max(1);
        if (width, height) == image.dimensions() {
            return Ok(image.clone());
        }
        Ok(match image {
            PixelBuffer::EncodedSrgba8(image) => {
                imageops::thumbnail(image.as_ref(), width, height).into()
            }
            PixelBuffer::LinearRgbaF32(image) => image.resized(width, height)?.into(),
        })
    }

    fn insert(&mut self, id: String, image: PixelBuffer) -> Result<String, AppError> {
        let (width, height) = image.dimensions();
        validate_dimensions(width, height)?;
        if let PixelBuffer::LinearRgbaF32(image) = &image {
            image.validate()?;
        }
        if !self.buffers.contains_key(&id) && self.buffers.len() >= MAX_STORE_BUFFERS {
            return Err(AppError::OutOfMemoryRisk);
        }
        let replaced = self
            .buffers
            .get(&id)
            .map(StoredPixels::bytes)
            .unwrap_or_default();
        let preview_bytes = if self.preview_scale() < 1.0 {
            let w = ((f64::from(width) * self.preview_scale()).round() as u64).max(1);
            let h = ((f64::from(height) * self.preview_scale()).round() as u64).max(1);
            w.checked_mul(h)
                .and_then(|n| {
                    n.checked_mul(match image {
                        PixelBuffer::EncodedSrgba8(_) => 4,
                        PixelBuffer::LinearRgbaF32(_) => 16,
                    })
                })
                .ok_or(AppError::OutOfMemoryRisk)?
        } else {
            0
        };
        let incoming = image
            .bytes()
            .checked_add(preview_bytes)
            .ok_or(AppError::OutOfMemoryRisk)?;
        let projected = self
            .total_bytes()
            .saturating_sub(replaced)
            .saturating_add(incoming);
        if projected > MAX_STORE_BYTES {
            return Err(AppError::OutOfMemoryRisk);
        }
        let preview = self.scaled_preview(&image)?;
        let full_fingerprint = fingerprint(&image);
        let preview_fingerprint = if same_buffer(&image, &preview) {
            full_fingerprint
        } else {
            fingerprint(&preview)
        };
        let stored = StoredPixels {
            full: image,
            preview,
            full_fingerprint,
            preview_fingerprint,
        };
        self.buffers.insert(id.clone(), stored);
        Ok(id)
    }

    /// Registers a buffer under a freshly generated identifier.
    pub fn register(&mut self, image: RgbaImage) -> Result<String, AppError> {
        self.register_typed(image.into())
    }

    pub fn register_float(&mut self, image: FloatImage) -> Result<String, AppError> {
        self.register_typed(image.into())
    }

    pub fn register_typed(&mut self, image: PixelBuffer) -> Result<String, AppError> {
        let id = self.next_identifier();
        self.insert(id, image)
    }

    /// Registers a buffer under a caller-supplied identifier, used when loading
    /// a project so the saved tree's references stay valid.
    pub fn register_with_id(&mut self, id: &str, image: RgbaImage) -> Result<(), AppError> {
        self.register_typed_with_id(id, image.into())
    }

    pub fn register_typed_with_id(&mut self, id: &str, image: PixelBuffer) -> Result<(), AppError> {
        if id.is_empty()
            || id.len() > 64
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(AppError::InvalidLayerDocument(
                "pixel buffer identifiers use the same character set as layer identifiers".into(),
            ));
        }
        // Keep generated identifiers from colliding with restored ones.
        if let Some(numeric) = id
            .strip_prefix("px")
            .and_then(|rest| rest.parse::<u64>().ok())
        {
            self.next_pixel = self.next_pixel.max(numeric);
        }
        self.insert(id.to_string(), image).map(|_| ())
    }

    pub fn full(&self, pixel_id: &str) -> Result<Arc<RgbaImage>, AppError> {
        Ok(self.full_typed(pixel_id)?.encoded8())
    }

    pub fn full_typed(&self, pixel_id: &str) -> Result<PixelBuffer, AppError> {
        self.buffers
            .get(pixel_id)
            .map(|stored| stored.full.clone())
            .ok_or_else(|| AppError::LayerPixelsMissing(pixel_id.to_string()))
    }

    /// Clones the `Arc` handles a render needs so the caller can release the
    /// session lock before doing any pixel work.
    pub fn resolve(&self, pixel_ids: &[String], preview: bool) -> Result<ResolvedPixels, AppError> {
        let mut resolved = HashMap::with_capacity(pixel_ids.len());
        let mut native = HashMap::with_capacity(pixel_ids.len());
        let mut fingerprints = HashMap::with_capacity(pixel_ids.len());
        for id in pixel_ids {
            let stored = self
                .buffers
                .get(id)
                .ok_or_else(|| AppError::LayerPixelsMissing(id.clone()))?;
            let buffer = if preview {
                stored.preview.clone()
            } else {
                stored.full.clone()
            };
            let fingerprint = if preview {
                stored.preview_fingerprint
            } else {
                stored.full_fingerprint
            };
            resolved.insert(id.clone(), buffer);
            // Smart sources composite in their own native coordinate space,
            // even when their instances are being drawn in a small preview.
            // These are shared Arc handles, never another copy of the pixels.
            native.insert(id.clone(), stored.full.clone());
            fingerprints.insert(id.clone(), fingerprint);
        }
        Ok(ResolvedPixels {
            buffers: resolved,
            native,
            fingerprints,
            resident_bytes: self.total_bytes(),
        })
    }

    /// Drops every buffer not named in `keep`. Callers pass the union of the
    /// live document's references and everything still reachable through undo
    /// history, so an undone merge can still be redone.
    pub fn retain(&mut self, keep: &[String]) -> usize {
        let before = self.buffers.len();
        self.buffers
            .retain(|id, _| keep.iter().any(|kept| kept == id));
        before - self.buffers.len()
    }

    pub fn clear(&mut self) {
        self.buffers.clear();
    }
}

/// A render-time view over resolved buffers. Holding one keeps the pixel data
/// alive without holding the session lock.
pub struct ResolvedPixels {
    buffers: HashMap<String, PixelBuffer>,
    native: HashMap<String, PixelBuffer>,
    fingerprints: HashMap<String, [u8; 32]>,
    resident_bytes: u64,
}

impl ResolvedPixels {
    pub fn len(&self) -> usize {
        self.buffers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffers.is_empty()
    }
}

impl PixelSource for ResolvedPixels {
    fn cache_fingerprint(&self, pixel_id: &str) -> Option<[u8; 32]> {
        self.fingerprints.get(pixel_id).copied()
    }

    fn dimensions(&self, pixel_id: &str) -> Option<(u32, u32)> {
        self.buffers.get(pixel_id).map(PixelBuffer::dimensions)
    }
    fn promotion_bytes(&self, _document: &super::LayerDocument) -> u64 {
        self.buffers
            .values()
            .filter_map(|buffer| match buffer {
                PixelBuffer::EncodedSrgba8(image) => {
                    Some(u64::from(image.width()) * u64::from(image.height()) * 16)
                }
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }
    fn resolve(&self, pixel_id: &str) -> Result<Arc<RgbaImage>, AppError> {
        self.buffers
            .get(pixel_id)
            .map(PixelBuffer::encoded8)
            .ok_or_else(|| AppError::LayerPixelsMissing(pixel_id.to_string()))
    }

    fn resolve_linear(&self, pixel_id: &str) -> Result<Arc<FloatImage>, AppError> {
        self.buffers
            .get(pixel_id)
            .ok_or_else(|| AppError::LayerPixelsMissing(pixel_id.to_string()))?
            .linear()
    }

    fn resolve_native_linear(&self, pixel_id: &str) -> Result<Arc<FloatImage>, AppError> {
        self.native
            .get(pixel_id)
            .ok_or_else(|| AppError::LayerPixelsMissing(pixel_id.to_string()))?
            .linear()
    }

    fn resident_bytes(&self) -> u64 {
        self.resident_bytes
    }
}

fn same_buffer(a: &PixelBuffer, b: &PixelBuffer) -> bool {
    match (a, b) {
        (PixelBuffer::EncodedSrgba8(a), PixelBuffer::EncodedSrgba8(b)) => Arc::ptr_eq(a, b),
        (PixelBuffer::LinearRgbaF32(a), PixelBuffer::LinearRgbaF32(b)) => Arc::ptr_eq(a, b),
        _ => false,
    }
}

/// Hashes the stored representation once, when an immutable buffer enters the
/// store. Different representations may deliberately get different digests;
/// equality must imply equal rendered content, while extra misses are safe.
fn fingerprint(buffer: &PixelBuffer) -> [u8; 32] {
    let mut hasher = Sha256::new();
    let (width, height) = buffer.dimensions();
    hasher.update(b"photoforge.pixel.v1");
    hasher.update(width.to_le_bytes());
    hasher.update(height.to_le_bytes());
    match buffer {
        PixelBuffer::EncodedSrgba8(image) => {
            hasher.update([0]);
            hasher.update(image.as_raw());
        }
        PixelBuffer::LinearRgbaF32(image) => {
            hasher.update([1]);
            for pixel in image.pixels() {
                hasher.update(pixel.red.to_bits().to_le_bytes());
                hasher.update(pixel.green.to_bits().to_le_bytes());
                hasher.update(pixel.blue.to_bits().to_le_bytes());
                hasher.update(pixel.alpha.to_bits().to_le_bytes());
            }
        }
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn solid(width: u32, height: u32, value: u8) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([value, value, value, 255]))
    }

    fn store(width: u32, height: u32) -> LayerPixelStore {
        let mut store = LayerPixelStore::default();
        store.reset(width, height).unwrap();
        store
    }

    #[test]
    fn preview_dimensions_preserve_aspect_ratio_within_the_ceiling() {
        assert_eq!(preview_dimensions(800, 600), (800, 600));
        assert_eq!(preview_dimensions(3_200, 2_400), (1_600, 1_200));
        assert_eq!(preview_dimensions(2_400, 3_200), (1_200, 1_600));
        assert_eq!(preview_dimensions(6_000, 4_000), (1_600, 1_067));
        let (width, height) = preview_dimensions(20_000, 1);
        assert_eq!(width, 1_600);
        assert!(height >= 1);
    }

    #[test]
    fn a_small_canvas_renders_previews_at_full_scale() {
        let store = store(800, 600);
        assert!((store.preview_scale() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_large_canvas_reports_a_proportional_preview_scale() {
        let store = store(3_200, 2_400);
        assert!((store.preview_scale() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn registering_a_buffer_returns_a_stable_unique_identifier() {
        let mut store = store(64, 64);
        let first = store.register(solid(64, 64, 10)).unwrap();
        let second = store.register(solid(64, 64, 20)).unwrap();
        assert_ne!(first, second);
        assert!(store.contains(&first));
        assert!(store.contains(&second));
        assert_eq!(store.buffer_count(), 2);
    }

    #[test]
    fn previews_are_generated_at_the_canvas_scale_for_every_layer_size() {
        let mut store = store(3_200, 2_400);
        let id = store.register(solid(800, 400, 5)).unwrap();
        let resolved = store.resolve(std::slice::from_ref(&id), true).unwrap();
        let preview = resolved.resolve(&id).unwrap();
        assert_eq!(preview.dimensions(), (400, 200));

        let full = store.resolve(std::slice::from_ref(&id), false).unwrap();
        assert_eq!(full.resolve(&id).unwrap().dimensions(), (800, 400));
    }

    #[test]
    fn resolving_a_missing_buffer_fails_closed() {
        let store = store(64, 64);
        assert!(matches!(
            store.resolve(&["ghost".to_string()], false),
            Err(AppError::LayerPixelsMissing(_))
        ));
        assert!(matches!(
            store.full("ghost"),
            Err(AppError::LayerPixelsMissing(_))
        ));
    }

    #[test]
    fn several_layers_can_share_one_buffer_without_copying_pixels() {
        let mut store = store(64, 64);
        let id = store.register(solid(64, 64, 7)).unwrap();
        let resolved = store.resolve(&[id.clone(), id.clone()], false).unwrap();
        let first = resolved.resolve(&id).unwrap();
        let second = resolved.resolve(&id).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(store.buffer_count(), 1);
    }

    #[test]
    fn resetting_releases_every_buffer_and_rebinds_the_canvas() {
        let mut store = store(64, 64);
        store.register(solid(64, 64, 1)).unwrap();
        store.reset(128, 64).unwrap();
        assert_eq!(store.buffer_count(), 0);
        assert_eq!(store.canvas(), (128, 64));
        assert_eq!(store.total_bytes(), 0);
    }

    #[test]
    fn retain_drops_only_unreferenced_buffers() {
        let mut store = store(64, 64);
        let kept = store.register(solid(16, 16, 1)).unwrap();
        let dropped = store.register(solid(16, 16, 2)).unwrap();
        let removed = store.retain(std::slice::from_ref(&kept));
        assert_eq!(removed, 1);
        assert!(store.contains(&kept));
        assert!(!store.contains(&dropped));
    }

    #[test]
    fn total_bytes_counts_full_and_preview_buffers() {
        let mut store = store(64, 64);
        store.register(solid(16, 16, 1)).unwrap();
        // A full-scale preview shares the immutable source rather than doubling it.
        assert_eq!(store.total_bytes(), 16 * 16 * 4);
    }

    #[test]
    fn registering_beyond_the_memory_ceiling_is_rejected() {
        let mut store = store(4_000, 4_000);
        let mut registered = 0;
        loop {
            match store.register(solid(4_000, 4_000, 3)) {
                Ok(_) => {
                    registered += 1;
                    assert!(registered < 64, "the memory ceiling was never reached");
                }
                Err(AppError::OutOfMemoryRisk) => break,
                Err(error) => panic!("unexpected error: {error}"),
            }
        }
        assert!(store.total_bytes() <= MAX_STORE_BYTES);
    }

    #[test]
    fn registering_beyond_the_buffer_count_ceiling_is_rejected() {
        let mut store = store(8, 8);
        for _ in 0..MAX_STORE_BUFFERS {
            store.register(solid(1, 1, 1)).unwrap();
        }
        assert!(matches!(
            store.register(solid(1, 1, 1)),
            Err(AppError::OutOfMemoryRisk)
        ));
    }

    #[test]
    fn restored_identifiers_are_validated_and_never_collide_with_generated_ones() {
        let mut store = store(64, 64);
        store.register_with_id("px7", solid(8, 8, 1)).unwrap();
        let generated = store.register(solid(8, 8, 2)).unwrap();
        assert_eq!(generated, "px8");

        assert!(store.register_with_id("../escape", solid(8, 8, 1)).is_err());
        assert!(store.register_with_id("", solid(8, 8, 1)).is_err());
        assert!(store
            .register_with_id(&"a".repeat(65), solid(8, 8, 1))
            .is_err());
    }

    #[test]
    fn oversized_buffers_are_rejected_before_allocation_bookkeeping() {
        let mut store = store(64, 64);
        let tiny = RgbaImage::new(0, 0);
        assert!(store.register(tiny).is_err());
        assert_eq!(store.buffer_count(), 0);
    }

    #[test]
    fn an_invalid_canvas_cannot_be_bound() {
        let mut store = LayerPixelStore::default();
        assert!(store.reset(0, 100).is_err());
        assert!(store.reset(30_000, 30_000).is_err());
    }

    #[test]
    fn resolved_views_report_their_size() {
        let mut store = store(64, 64);
        let id = store.register(solid(8, 8, 1)).unwrap();
        let resolved = store.resolve(&[id], false).unwrap();
        assert_eq!(resolved.len(), 1);
        assert!(!resolved.is_empty());
        assert!(store.resolve(&[], false).unwrap().is_empty());
    }
}
