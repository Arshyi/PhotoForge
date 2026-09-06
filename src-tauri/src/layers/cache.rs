//! A bounded, evicting cache of rendered tiles.
//!
//! # What makes a cached tile reusable
//!
//! A tile may be reused only when everything that could have changed its pixels
//! is unchanged. That is a stronger claim than "the layer I edited is somewhere
//! else", so the key is derived rather than asserted: each layer is digested
//! once, an *influence region* is computed for it, and a tile's key is the
//! digest of every layer whose influence reaches that tile, in document order.
//!
//! Influence is deliberately pessimistic. An adjustment layer influences the
//! whole canvas. A group influences the transform of the union of its children.
//! A pixel layer influences its transformed bounds grown by the render halo.
//! Being wrong in this direction costs a re-render; being wrong in the other
//! direction shows the user stale pixels, so nothing here narrows a region on
//! an argument that has not been tested.
//!
//! # Bounds are enforced, not advertised
//!
//! The cache holds a byte budget, and inserting past it evicts least-recently
//! used entries until the new entry fits. A single entry larger than the whole
//! budget is refused rather than evicting everything to store it. `stats()`
//! reports what actually happened, including refusals.
//!
//! Cached tiles are rendered image content and are treated as private user
//! data. The default cache is memory-only; an explicit environment setting or
//! constructor may add the disposable, checksummed disk tier in
//! [`super::disk_cache`].
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use sha2::{Digest, Sha256};

use super::composite::render_transform;
use super::disk_cache::DiskTileCache;
use super::tiles::Region;
use super::{Layer, LayerContent, LayerDocument};
use crate::color::FloatImage;
use crate::error::AppError;

/// Default in-memory budget for rendered tiles.
pub const DEFAULT_CACHE_BYTES: u64 = 256 * 1024 * 1024;

/// A cache key. Wide enough that a collision is not a practical concern, which
/// matters more here than usual: a collision would serve the wrong pixels.
pub type TileKey = [u8; 32];

/// What the cache actually did, for reporting and for tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    /// Entries removed to stay inside the budget.
    pub evictions: u64,
    /// Entries refused because one tile alone exceeded the budget.
    pub refusals: u64,
    pub entries: u64,
    pub bytes: u64,
    pub capacity_bytes: u64,
    /// Entries spilled to the optional local disk cache.
    pub disk_entries: u64,
    pub disk_bytes: u64,
    pub disk_capacity_bytes: u64,
}

impl CacheStats {
    /// Share of lookups that were served from the cache, in [0,1].
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

struct Entry {
    image: Arc<FloatImage>,
    bytes: u64,
    used: u64,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<TileKey, Entry>,
    /// Use counter to key, so the least recently used entry is the first one.
    order: BTreeMap<u64, TileKey>,
    clock: u64,
    bytes: u64,
    capacity: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
    refusals: u64,
}

/// A bounded, least-recently-used cache of rendered tiles.
pub struct TileCache {
    inner: Mutex<Inner>,
    disk: Option<Arc<DiskTileCache>>,
}

impl Default for TileCache {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_CACHE_BYTES)
    }
}

impl TileCache {
    pub fn with_capacity(capacity_bytes: u64) -> Self {
        Self {
            inner: Mutex::new(Inner {
                capacity: capacity_bytes,
                ..Inner::default()
            }),
            disk: None,
        }
    }

    /// Creates a cache with an explicitly supplied disposable disk tier.
    /// Disk-cache construction is fallible so callers can disable it without
    /// making opening an image fail; the normal application constructor uses
    /// [`Self::from_environment`] for that reason.
    pub fn with_disk_capacity(
        capacity_bytes: u64,
        directory: impl AsRef<std::path::Path>,
        disk_capacity_bytes: u64,
    ) -> Result<Self, AppError> {
        Ok(Self {
            inner: Mutex::new(Inner {
                capacity: capacity_bytes,
                ..Inner::default()
            }),
            disk: Some(Arc::new(DiskTileCache::open(
                directory,
                disk_capacity_bytes,
            )?)),
        })
    }

    /// Enables the disk tier only when explicitly requested. This keeps the
    /// default build local and quiet while still providing a supported
    /// out-of-core spill path for large-document deployments and benchmarks.
    pub fn from_environment() -> Self {
        let directory = std::env::var_os("PHOTOFORGE_RENDER_CACHE_DIR")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                let enabled = std::env::var_os("PHOTOFORGE_ENABLE_DISK_CACHE")
                    .map(|value| value != "0" && !value.is_empty())
                    .unwrap_or(false);
                if enabled {
                    std::env::var_os("LOCALAPPDATA")
                        .map(std::path::PathBuf::from)
                        .map(|path| path.join("PhotoForge").join("render-cache"))
                } else {
                    None
                }
            });
        let Some(directory) = directory else {
            return Self::with_capacity(DEFAULT_CACHE_BYTES);
        };
        let disk_capacity = std::env::var("PHOTOFORGE_RENDER_CACHE_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(DEFAULT_CACHE_BYTES);
        Self::with_disk_capacity(DEFAULT_CACHE_BYTES, directory, disk_capacity)
            .unwrap_or_else(|_| Self::with_capacity(DEFAULT_CACHE_BYTES))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn get(&self, key: &TileKey) -> Option<Arc<FloatImage>> {
        {
            let mut inner = self.lock();
            inner.clock += 1;
            let clock = inner.clock;
            if let Some(entry) = inner.entries.get_mut(key) {
                let previous = entry.used;
                entry.used = clock;
                let image = Arc::clone(&entry.image);
                inner.order.remove(&previous);
                inner.order.insert(clock, *key);
                inner.hits += 1;
                return Some(image);
            }
        }

        // A disk hit is still a cache hit to the caller. Promote it to memory
        // so the next request avoids disk I/O, while keeping the memory tier's
        // byte budget authoritative.
        if let Some(disk) = &self.disk {
            if let Some(image) = disk.get(key) {
                let image = Arc::new(image);
                let mut inner = self.lock();
                insert_memory(&mut inner, *key, Arc::clone(&image));
                inner.hits += 1;
                return Some(image);
            }
        }
        self.lock().misses += 1;
        None
    }

    /// Stores a tile, evicting least-recently-used entries to stay in budget.
    ///
    /// Storing is best effort by design: a tile the cache cannot afford is
    /// simply not stored, because a render that fails for want of cache space
    /// would be worse than one that is merely slower.
    pub fn insert(&self, key: TileKey, image: Arc<FloatImage>) {
        let bytes = u64::from(image.width()) * u64::from(image.height()) * 16;
        {
            let mut inner = self.lock();
            if bytes > inner.capacity {
                inner.refusals += 1;
            } else {
                insert_memory(&mut inner, key, Arc::clone(&image));
            }
        }
        if let Some(disk) = &self.disk {
            disk.insert(key, &image);
        }
    }

    /// Drops everything. Used when a document closes, and by the interface's
    /// explicit "clear render cache" action.
    pub fn clear(&self) {
        let mut inner = self.lock();
        inner.entries.clear();
        inner.order.clear();
        inner.bytes = 0;
        drop(inner);
        if let Some(disk) = &self.disk {
            disk.clear();
        }
    }

    /// Changes the budget, evicting immediately if the new one is smaller.
    pub fn set_capacity(&self, capacity_bytes: u64) {
        let mut inner = self.lock();
        inner.capacity = capacity_bytes;
        inner.evict_until(0);
    }

    pub fn stats(&self) -> CacheStats {
        let inner = self.lock();
        let (disk_entries, disk_bytes, disk_capacity_bytes) = self
            .disk
            .as_ref()
            .map(|disk| {
                let stats = disk.stats();
                (stats.entries, stats.bytes, stats.capacity_bytes)
            })
            .unwrap_or_default();
        CacheStats {
            hits: inner.hits,
            misses: inner.misses,
            evictions: inner.evictions,
            refusals: inner.refusals,
            entries: inner.entries.len() as u64,
            bytes: inner.bytes,
            capacity_bytes: inner.capacity,
            disk_entries,
            disk_bytes,
            disk_capacity_bytes,
        }
    }
}

fn insert_memory(inner: &mut Inner, key: TileKey, image: Arc<FloatImage>) {
    let bytes = u64::from(image.width()) * u64::from(image.height()) * 16;
    if bytes > inner.capacity {
        inner.refusals += 1;
        return;
    }
    if let Some(existing) = inner.entries.remove(&key) {
        inner.order.remove(&existing.used);
        inner.bytes -= existing.bytes;
    }
    inner.evict_until(bytes);
    inner.clock += 1;
    let used = inner.clock;
    inner.bytes += bytes;
    inner.entries.insert(key, Entry { image, bytes, used });
    inner.order.insert(used, key);
}

impl Inner {
    /// Evicts until `headroom` more bytes would still fit inside the budget.
    fn evict_until(&mut self, headroom: u64) {
        while self.bytes + headroom > self.capacity {
            let Some((&used, &key)) = self.order.iter().next() else {
                // Nothing left to evict; the caller checked the entry fits, so
                // this only happens when the budget is zero.
                break;
            };
            self.order.remove(&used);
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes -= entry.bytes;
            }
            self.evictions += 1;
        }
    }
}

/// Where in the document a layer can change the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Influence {
    /// Nowhere: the layer is hidden, fully transparent, or off canvas.
    Nowhere,
    /// A rectangle of the render canvas.
    Within(Region),
    /// The whole canvas. Adjustments, and anything that could not be bounded.
    Everywhere,
}

impl Influence {
    fn reaches(self, tile: &Region) -> bool {
        match self {
            Self::Nowhere => false,
            Self::Everywhere => true,
            Self::Within(region) => !region.intersect(tile).is_empty(),
        }
    }
}

struct LayerPrint {
    digest: TileKey,
    influence: Influence,
}

/// A document reduced to what a tile key needs: one digest per layer, and where
/// that layer can reach.
///
/// Built once per render and then asked for a key per tile, so the expensive
/// part — serialising and hashing the layer tree — happens once rather than
/// once per tile.
pub struct DocumentFingerprint {
    base: TileKey,
    layers: Vec<LayerPrint>,
}

impl DocumentFingerprint {
    /// `halo` is the render halo in canvas pixels: a layer can influence tiles
    /// that far beyond its own bounds, because a neighbourhood operation above
    /// it will read across the boundary.
    pub fn new(
        document: &LayerDocument,
        source: &dyn super::PixelSource,
        scale: f64,
        tile_size: u32,
        halo: u32,
    ) -> Result<Self, AppError> {
        let (canvas_width, canvas_height) = super::tiled::render_dimensions(document, scale);
        let canvas = Region::whole(canvas_width, canvas_height);
        let mut base = Sha256::new();
        // Everything that changes what a tile means, rather than what is in it.
        base.update(b"photoforge.tile.v2");
        base.update(canvas_width.to_le_bytes());
        base.update(canvas_height.to_le_bytes());
        base.update(scale.to_bits().to_le_bytes());
        base.update(tile_size.to_le_bytes());
        base.update(halo.to_le_bytes());
        base.update((document.precision as u8).to_le_bytes());
        let base = base.finalize().into();

        let layers = document
            .layers
            .iter()
            .map(|layer| {
                Ok(LayerPrint {
                    digest: digest_layer(layer, source)?,
                    influence: influence_of(layer, source, scale, halo, &canvas),
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok(Self { base, layers })
    }

    /// The key for one tile: the document's own identity, then the index and
    /// digest of every layer that can reach this tile, in document order.
    ///
    /// Indices are included so that reordering or removing a layer changes the
    /// keys of the tiles below it, and the layer count is included so that a
    /// document whose extra layers all miss this tile is still a different
    /// document from one without them.
    pub fn tile_key(&self, tile: &Region) -> TileKey {
        let mut hasher = Sha256::new();
        hasher.update(self.base);
        hasher.update(tile.x.to_le_bytes());
        hasher.update(tile.y.to_le_bytes());
        hasher.update(tile.width.to_le_bytes());
        hasher.update(tile.height.to_le_bytes());
        hasher.update((self.layers.len() as u64).to_le_bytes());
        for (index, layer) in self.layers.iter().enumerate() {
            if layer.influence.reaches(tile) {
                hasher.update((index as u64).to_le_bytes());
                hasher.update(layer.digest);
            }
        }
        hasher.finalize().into()
    }
}

/// Digests everything about a layer that could change a pixel.
///
/// Serialisation is the whole layer, not a hand-picked field list, so a new
/// field added to `Layer` later cannot be silently left out of the key.
pub(crate) fn digest_layer(
    layer: &Layer,
    source: &dyn super::PixelSource,
) -> Result<TileKey, AppError> {
    let encoded = serde_json::to_vec(layer)
        .map_err(|_| AppError::InvalidLayerDocument("a layer could not be digested".into()))?;
    let mut hasher = Sha256::new();
    hasher.update(&encoded);
    digest_visible_sources(layer, source, &mut hasher)?;
    Ok(hasher.finalize().into())
}

/// Adds the immutable content behind every visible pixel reference. Layer
/// identifiers are document-local and can be reused after a store reset, so
/// hashing JSON alone is not sufficient to prevent cross-document stale hits.
fn digest_visible_sources(
    layer: &Layer,
    source: &dyn super::PixelSource,
    hasher: &mut Sha256,
) -> Result<(), AppError> {
    if !layer.visible || layer.opacity <= 0.0 {
        return Ok(());
    }
    match &layer.content {
        LayerContent::Shape { shape } => {
            // A shape's identity is entirely in its own description: geometry,
            // paint and rule. There is no external buffer to fingerprint, so
            // serialising it is both necessary and sufficient.
            hasher.update(b"shape-content");
            let encoded = serde_json::to_vec(shape.as_ref()).map_err(|_| {
                AppError::InvalidLayerDocument("a shape could not be digested".into())
            })?;
            hasher.update((encoded.len() as u64).to_le_bytes());
            hasher.update(&encoded);
        }
        LayerContent::Pixel { pixel_id, .. } => {
            hasher.update(b"pixel-source");
            hasher.update((pixel_id.len() as u64).to_le_bytes());
            hasher.update(pixel_id.as_bytes());
            let digest = match source.cache_fingerprint(pixel_id) {
                Some(digest) => digest,
                None => fingerprint_resolved_source(source, pixel_id)?,
            };
            hasher.update(digest);
        }
        LayerContent::Group { children, .. } => {
            for child in children {
                digest_visible_sources(child, source, hasher)?;
            }
        }
        LayerContent::Adjustment { .. } => {}
    }
    Ok(())
}

/// Safe fallback for custom `PixelSource` implementations. Production
/// `ResolvedPixels` supplies a digest computed once at registration time; a
/// source that does not can still use the cache without weakening its key.
fn fingerprint_resolved_source(
    source: &dyn super::PixelSource,
    pixel_id: &str,
) -> Result<TileKey, AppError> {
    let image = source.resolve_linear(pixel_id)?;
    let mut hasher = Sha256::new();
    hasher.update(b"photoforge.resolved-pixel.v1");
    hasher.update(image.width().to_le_bytes());
    hasher.update(image.height().to_le_bytes());
    for pixel in image.pixels() {
        hasher.update(pixel.red.to_bits().to_le_bytes());
        hasher.update(pixel.green.to_bits().to_le_bytes());
        hasher.update(pixel.blue.to_bits().to_le_bytes());
        hasher.update(pixel.alpha.to_bits().to_le_bytes());
    }
    Ok(hasher.finalize().into())
}

/// The region of the canvas a layer can change.
fn influence_of(
    layer: &Layer,
    source: &dyn super::PixelSource,
    scale: f64,
    halo: u32,
    canvas: &Region,
) -> Influence {
    if !layer.visible || layer.opacity <= 0.0 {
        return Influence::Nowhere;
    }
    let transform = render_transform(&layer.transform, scale);
    match &layer.content {
        // An adjustment reads and rewrites everything beneath it in its scope.
        // Bounding that properly would mean tracking what is beneath it, which
        // is exactly the kind of narrowing this module refuses to guess at.
        LayerContent::Adjustment { .. } => Influence::Everywhere,
        LayerContent::Shape { shape } => {
            // A shape reaches exactly as far as it paints, which is what makes
            // moving one invalidate the tiles it left and the tiles it arrived
            // at, and nothing else.
            let Some((min_x, min_y, max_x, max_y)) = shape.untransformed_bounds() else {
                return Influence::Nowhere;
            };
            let scaled = |v: f32| (f64::from(v) * scale) as f32;
            let corners = [
                (scaled(min_x), scaled(min_y)),
                (scaled(max_x), scaled(min_y)),
                (scaled(min_x), scaled(max_y)),
                (scaled(max_x), scaled(max_y)),
            ];
            let (mut low_x, mut low_y) = (f32::MAX, f32::MAX);
            let (mut high_x, mut high_y) = (f32::MIN, f32::MIN);
            for (x, y) in corners {
                let (fx, fy) = transform.forward((x, y), canvas.width, canvas.height);
                if !fx.is_finite() || !fy.is_finite() {
                    return Influence::Everywhere;
                }
                low_x = low_x.min(fx);
                low_y = low_y.min(fy);
                high_x = high_x.max(fx);
                high_y = high_y.max(fy);
            }
            let x = low_x.floor().max(0.0) as u32;
            let y = low_y.floor().max(0.0) as u32;
            let right = (high_x.ceil().max(0.0) as u64 + 1).min(u64::from(canvas.width));
            let bottom = (high_y.ceil().max(0.0) as u64 + 1).min(u64::from(canvas.height));
            if u64::from(x) >= right || u64::from(y) >= bottom {
                return Influence::Nowhere;
            }
            Influence::Within(
                Region::new(
                    x,
                    y,
                    (right - u64::from(x)) as u32,
                    (bottom - u64::from(y)) as u32,
                )
                .expanded(halo, canvas),
            )
        }
        LayerContent::Pixel { pixel_id, .. } => {
            // The renderer bounds this layer by the resolved buffer's size. If
            // that size is unknown the layer could be larger than the canvas
            // and reach anywhere, so the honest answer is the whole canvas.
            let Some((width, height)) = source.dimensions(pixel_id) else {
                return Influence::Everywhere;
            };
            let bounds = transform.document_bounds(width, height);
            match bounds.clip_to_canvas(canvas.width, canvas.height) {
                None => Influence::Nowhere,
                Some((x0, y0, x1, y1)) => {
                    Influence::Within(Region::new(x0, y0, x1 - x0, y1 - y0).expanded(halo, canvas))
                }
            }
        }
        LayerContent::Group { children, .. } => {
            let mut union: Option<Region> = None;
            for child in children {
                match influence_of(child, source, scale, halo, canvas) {
                    Influence::Nowhere => continue,
                    Influence::Everywhere => return Influence::Everywhere,
                    Influence::Within(region) => {
                        union = Some(match union {
                            None => region,
                            Some(existing) => bounding(&existing, &region),
                        });
                    }
                }
            }
            let Some(union) = union else {
                return Influence::Nowhere;
            };
            if transform.is_identity() {
                return Influence::Within(union.expanded(halo, canvas));
            }
            // The group's buffer is sampled through its transform, so a child
            // far from a tile in the group's own coordinates can still land on
            // it. Map the corners forward and take the bounding box.
            let corners = [
                (union.x as f32, union.y as f32),
                (union.right() as f32, union.y as f32),
                (union.x as f32, union.bottom() as f32),
                (union.right() as f32, union.bottom() as f32),
            ];
            let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
            let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
            for (x, y) in corners {
                let (fx, fy) = transform.forward((x, y), canvas.width, canvas.height);
                if !fx.is_finite() || !fy.is_finite() {
                    return Influence::Everywhere;
                }
                min_x = min_x.min(fx);
                min_y = min_y.min(fy);
                max_x = max_x.max(fx);
                max_y = max_y.max(fy);
            }
            let x = min_x.floor().max(0.0) as u32;
            let y = min_y.floor().max(0.0) as u32;
            let right = (max_x.ceil().max(0.0) as u64 + 1).min(u64::from(canvas.width));
            let bottom = (max_y.ceil().max(0.0) as u64 + 1).min(u64::from(canvas.height));
            if u64::from(x) >= right || u64::from(y) >= bottom {
                return Influence::Nowhere;
            }
            Influence::Within(
                Region::new(
                    x,
                    y,
                    (right - u64::from(x)) as u32,
                    (bottom - u64::from(y)) as u32,
                )
                .expanded(halo, canvas),
            )
        }
    }
}

/// The smallest rectangle containing both.
fn bounding(a: &Region, b: &Region) -> Region {
    let x = a.x.min(b.x);
    let y = a.y.min(b.y);
    let right = a.right().max(b.right());
    let bottom = a.bottom().max(b.bottom());
    Region::new(
        x,
        y,
        (right - u64::from(x)) as u32,
        (bottom - u64::from(y)) as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::FloatRgba;

    fn tile(bytes: u64) -> Arc<FloatImage> {
        // 16 bytes per pixel, so a one-pixel-tall image of n/16 pixels.
        let width = (bytes / 16) as u32;
        Arc::new(FloatImage::blank(width, 1, FloatRgba::TRANSPARENT).unwrap())
    }

    fn key(n: u8) -> TileKey {
        let mut key = [0u8; 32];
        key[0] = n;
        key
    }

    /// The budget is the point. A cache that merely counted bytes and kept
    /// going would be a memory leak with statistics.
    #[test]
    fn the_byte_budget_is_actually_enforced() {
        let cache = TileCache::with_capacity(1000);
        for n in 0..20 {
            cache.insert(key(n), tile(160));
            let stats = cache.stats();
            assert!(
                stats.bytes <= stats.capacity_bytes,
                "held {} bytes against a {} byte budget",
                stats.bytes,
                stats.capacity_bytes
            );
        }
        let stats = cache.stats();
        assert!(stats.evictions > 0, "nothing was ever evicted");
        assert_eq!(stats.entries * 160, stats.bytes);
    }

    /// Least *recently used*, not least recently inserted: reading an entry has
    /// to protect it, or a cache under pressure evicts exactly what is in use.
    #[test]
    fn eviction_takes_the_least_recently_used_entry() {
        let cache = TileCache::with_capacity(320);
        cache.insert(key(1), tile(160));
        cache.insert(key(2), tile(160));
        // Touching 1 makes 2 the oldest.
        assert!(cache.get(&key(1)).is_some());
        cache.insert(key(3), tile(160));
        assert!(
            cache.get(&key(1)).is_some(),
            "the touched entry was evicted"
        );
        assert!(cache.get(&key(2)).is_none(), "the stale entry survived");
        assert!(cache.get(&key(3)).is_some());
    }

    /// A tile bigger than the whole budget must not empty the cache trying to
    /// fit, and must be reported rather than silently dropped.
    #[test]
    fn an_oversized_tile_is_refused_without_evicting_everything() {
        let cache = TileCache::with_capacity(320);
        cache.insert(key(1), tile(160));
        cache.insert(key(9), tile(640));
        let stats = cache.stats();
        assert_eq!(stats.refusals, 1);
        assert!(
            cache.get(&key(1)).is_some(),
            "a refusal evicted a good entry"
        );
        assert!(cache.get(&key(9)).is_none());
    }

    /// Re-inserting a key must replace it, not double-count its bytes.
    #[test]
    fn replacing_an_entry_does_not_leak_its_bytes() {
        let cache = TileCache::with_capacity(1000);
        cache.insert(key(1), tile(160));
        cache.insert(key(1), tile(320));
        let stats = cache.stats();
        assert_eq!(stats.entries, 1);
        assert_eq!(stats.bytes, 320);
    }

    /// Lowering the budget has to take effect immediately, not at the next
    /// insert, or a user reducing it would see no memory returned.
    #[test]
    fn shrinking_the_budget_evicts_at_once() {
        let cache = TileCache::with_capacity(1600);
        for n in 0..10 {
            cache.insert(key(n), tile(160));
        }
        assert_eq!(cache.stats().bytes, 1600);
        cache.set_capacity(320);
        let stats = cache.stats();
        assert!(stats.bytes <= 320, "shrinking left {} bytes", stats.bytes);
        assert_eq!(stats.capacity_bytes, 320);
    }

    /// A zero budget is a legitimate setting: caching off, nothing retained.
    #[test]
    fn a_zero_budget_stores_nothing() {
        let cache = TileCache::with_capacity(0);
        cache.insert(key(1), tile(160));
        assert_eq!(cache.stats().entries, 0);
        assert_eq!(cache.stats().bytes, 0);
        assert!(cache.get(&key(1)).is_none());
    }

    #[test]
    fn clearing_releases_everything_but_keeps_the_budget() {
        let cache = TileCache::with_capacity(1000);
        cache.insert(key(1), tile(160));
        cache.clear();
        let stats = cache.stats();
        assert_eq!((stats.entries, stats.bytes), (0, 0));
        assert_eq!(stats.capacity_bytes, 1000);
    }

    #[test]
    fn statistics_count_what_happened() {
        let cache = TileCache::with_capacity(1000);
        assert_eq!(cache.stats().hit_rate(), 0.0);
        cache.insert(key(1), tile(160));
        assert!(cache.get(&key(1)).is_some());
        assert!(cache.get(&key(2)).is_none());
        let stats = cache.stats();
        assert_eq!((stats.hits, stats.misses), (1, 1));
        assert!((stats.hit_rate() - 0.5).abs() < 1e-9);
    }
}
