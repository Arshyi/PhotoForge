//! Optional, disposable on-disk storage for rendered float tiles.
//!
//! The authoritative project never depends on this directory. Every entry is
//! a self-describing, checksummed payload addressed by a fixed hexadecimal
//! filename. A damaged, truncated, old, oversized, or otherwise unexpected
//! entry is a cache miss and can be removed without affecting the document.
//!
//! The cache is opt-in. The application enables it only when
//! `PHOTOFORGE_RENDER_CACHE_DIR` is set (or a caller constructs one directly),
//! keeping the normal interactive path free of disk I/O. The directory is
//! intended to be a private PhotoForge cache location, not a project folder.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use sha2::{Digest, Sha256};

use crate::color::{FloatImage, FloatRgba};
use crate::error::AppError;

use super::cache::TileKey;

const MAGIC: &[u8; 8] = b"PFTILE01";
const VERSION: u32 = 1;
const HEADER_BYTES: usize = 8 + 4 + 4 + 4 + 8 + 32;
/// A tile should normally be 256x256 (256 KiB for f32 RGBA). This upper bound
/// prevents a hostile cache file from forcing a large allocation even when a
/// caller configured a very large disk budget.
pub const MAX_DISK_TILE_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskCacheStats {
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub refusals: u64,
    pub entries: u64,
    pub bytes: u64,
    pub capacity_bytes: u64,
}

struct Entry {
    bytes: u64,
    used: u64,
}

#[derive(Default)]
struct Inner {
    entries: HashMap<TileKey, Entry>,
    order: BTreeMap<u64, TileKey>,
    clock: u64,
    bytes: u64,
    capacity: u64,
    hits: u64,
    misses: u64,
    evictions: u64,
    refusals: u64,
}

/// A bounded, checksummed, local-only tile cache.
pub struct DiskTileCache {
    directory: PathBuf,
    inner: Mutex<Inner>,
}

impl DiskTileCache {
    /// Opens a cache directory and discards only valid-looking cache entries
    /// that fail validation. Files with unrelated names are left untouched.
    pub fn open(directory: impl AsRef<Path>, capacity_bytes: u64) -> Result<Self, AppError> {
        if capacity_bytes > 0 && capacity_bytes > MAX_DISK_TILE_BYTES.saturating_mul(16_384) {
            return Err(AppError::InvalidLayerDocument(
                "the disk render cache budget is unreasonably large".into(),
            ));
        }
        let directory = directory.as_ref().to_path_buf();
        if let Ok(metadata) = fs::symlink_metadata(&directory) {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(AppError::ProjectIo(
                    "the render cache path is not a private directory".into(),
                ));
            }
        } else {
            fs::create_dir_all(&directory).map_err(|error| {
                AppError::ProjectIo(format!("could not create the render cache: {error}"))
            })?;
        }
        let cache = Self {
            directory,
            inner: Mutex::new(Inner {
                capacity: capacity_bytes,
                ..Inner::default()
            }),
        };
        cache.index_existing()?;
        Ok(cache)
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn index_existing(&self) -> Result<(), AppError> {
        let entries = fs::read_dir(&self.directory).map_err(|error| {
            AppError::ProjectIo(format!("could not scan the render cache: {error}"))
        })?;
        for entry in entries.flatten() {
            let path = entry.path();
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            let Some(key) = key_from_path(&path) else {
                continue;
            };
            let valid = read_tile(&path).is_some();
            if !valid {
                let _ = fs::remove_file(&path);
                continue;
            }
            let bytes = metadata.len().saturating_sub(HEADER_BYTES as u64);
            if bytes == 0 || bytes > MAX_DISK_TILE_BYTES || bytes > self.lock().capacity {
                let _ = fs::remove_file(&path);
                continue;
            }
            let mut inner = self.lock();
            inner.clock = inner.clock.saturating_add(1);
            let used = inner.clock;
            inner.bytes = inner.bytes.saturating_add(bytes);
            inner.entries.insert(key, Entry { bytes, used });
            inner.order.insert(used, key);
            inner.evict_until(&self.directory, 0);
        }
        Ok(())
    }

    /// Returns a validated tile, treating all I/O and format failures as a
    /// miss. A corrupt owned entry is removed; unrelated files are untouched.
    pub fn get(&self, key: &TileKey) -> Option<FloatImage> {
        let path = self.path_for(key);
        let image = read_tile(&path);
        let mut inner = self.lock();
        match image {
            Some(image) => {
                if let Some(previous) = inner.entries.get(key).map(|entry| entry.used) {
                    inner.clock = inner.clock.saturating_add(1);
                    let used = inner.clock;
                    if let Some(entry) = inner.entries.get_mut(key) {
                        entry.used = used;
                    }
                    inner.order.remove(&previous);
                    inner.order.insert(used, *key);
                } else {
                    // An entry created after open (or by another cache object)
                    // is indexed lazily and still subject to the same budget.
                    let bytes = image_bytes(&image).unwrap_or(0);
                    if bytes == 0 || bytes > inner.capacity {
                        inner.misses = inner.misses.saturating_add(1);
                        return None;
                    }
                    inner.clock = inner.clock.saturating_add(1);
                    let used = inner.clock;
                    inner.bytes = inner.bytes.saturating_add(bytes);
                    inner.entries.insert(*key, Entry { bytes, used });
                    inner.order.insert(used, *key);
                    inner.evict_until(&self.directory, 0);
                }
                inner.hits = inner.hits.saturating_add(1);
                Some(image)
            }
            None => {
                if inner.entries.remove(key).is_some() {
                    inner.rebuild_order_and_bytes();
                }
                inner.misses = inner.misses.saturating_add(1);
                let _ = remove_owned(&path);
                None
            }
        }
    }

    /// Writes a tile using a temporary file and a replacement rename. Failure
    /// is deliberately best effort: disk cache failure must never fail a render.
    pub fn insert(&self, key: TileKey, image: &FloatImage) {
        let Some(bytes) = image_bytes(image) else {
            return;
        };
        let mut inner = self.lock();
        if bytes == 0 || bytes > MAX_DISK_TILE_BYTES || bytes > inner.capacity {
            inner.refusals = inner.refusals.saturating_add(1);
            return;
        }
        let path = self.path_for(&key);
        if let Some(existing) = inner.entries.remove(&key) {
            inner.order.remove(&existing.used);
            inner.bytes = inner.bytes.saturating_sub(existing.bytes);
        }
        inner.evict_until(&self.directory, bytes);
        if write_tile(&path, image).is_err() {
            return;
        }
        inner.clock = inner.clock.saturating_add(1);
        let used = inner.clock;
        inner.bytes = inner.bytes.saturating_add(bytes);
        inner.entries.insert(key, Entry { bytes, used });
        inner.order.insert(used, key);
    }

    pub fn clear(&self) {
        let keys: Vec<_> = self.lock().entries.keys().copied().collect();
        for key in keys {
            let _ = remove_owned(&self.path_for(&key));
        }
        let mut inner = self.lock();
        inner.entries.clear();
        inner.order.clear();
        inner.bytes = 0;
    }

    pub fn set_capacity(&self, capacity_bytes: u64) {
        let mut inner = self.lock();
        inner.capacity = capacity_bytes;
        inner.evict_until(&self.directory, 0);
    }

    pub fn stats(&self) -> DiskCacheStats {
        let inner = self.lock();
        DiskCacheStats {
            hits: inner.hits,
            misses: inner.misses,
            evictions: inner.evictions,
            refusals: inner.refusals,
            entries: inner.entries.len() as u64,
            bytes: inner.bytes,
            capacity_bytes: inner.capacity,
        }
    }

    fn path_for(&self, key: &TileKey) -> PathBuf {
        self.directory.join(format!("{}.tile", hex_key(key)))
    }
}

impl Inner {
    fn evict_until(&mut self, directory: &Path, headroom: u64) {
        while self.bytes.saturating_add(headroom) > self.capacity {
            let Some((&used, &key)) = self.order.iter().next() else {
                break;
            };
            self.order.remove(&used);
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes = self.bytes.saturating_sub(entry.bytes);
            }
            let path = directory.join(format!("{}.tile", hex_key(&key)));
            let _ = remove_owned(&path);
            self.evictions = self.evictions.saturating_add(1);
        }
    }

    fn rebuild_order_and_bytes(&mut self) {
        self.order.clear();
        self.bytes = 0;
        for (key, entry) in &self.entries {
            self.order.insert(entry.used, *key);
            self.bytes = self.bytes.saturating_add(entry.bytes);
        }
    }
}

fn image_bytes(image: &FloatImage) -> Option<u64> {
    u64::from(image.width())
        .checked_mul(u64::from(image.height()))
        .and_then(|pixels| pixels.checked_mul(16))
        .filter(|bytes| *bytes <= MAX_DISK_TILE_BYTES)
}

fn hex_key(key: &TileKey) -> String {
    let mut output = String::with_capacity(64);
    for byte in key {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn key_from_path(path: &Path) -> Option<TileKey> {
    if path.extension().and_then(|extension| extension.to_str()) != Some("tile") {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    if stem.len() != 64 || !stem.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut key = [0u8; 32];
    for (index, pair) in stem.as_bytes().chunks_exact(2).enumerate() {
        key[index] = (hex(pair[0])? << 4) | hex(pair[1])?;
    }
    Some(key)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn remove_owned(path: &Path) -> std::io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::remove_file(path),
        Ok(metadata) if metadata.is_file() => fs::remove_file(path),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn write_tile(path: &Path, image: &FloatImage) -> Result<(), AppError> {
    image
        .validate()
        .map_err(|error| AppError::ProcessingFailure(format!("invalid tile: {error}")))?;
    let bytes = image_bytes(image).ok_or(AppError::OutOfMemoryRisk)?;
    let mut payload = Vec::new();
    payload
        .try_reserve_exact(usize::try_from(bytes).map_err(|_| AppError::OutOfMemoryRisk)?)
        .map_err(|_| AppError::OutOfMemoryRisk)?;
    for pixel in image.pixels() {
        payload.extend_from_slice(&pixel.red.to_bits().to_le_bytes());
        payload.extend_from_slice(&pixel.green.to_bits().to_le_bytes());
        payload.extend_from_slice(&pixel.blue.to_bits().to_le_bytes());
        payload.extend_from_slice(&pixel.alpha.to_bits().to_le_bytes());
    }
    let checksum: [u8; 32] = Sha256::digest(&payload).into();
    let mut header = Vec::with_capacity(HEADER_BYTES);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&VERSION.to_le_bytes());
    header.extend_from_slice(&image.width().to_le_bytes());
    header.extend_from_slice(&image.height().to_le_bytes());
    header.extend_from_slice(&bytes.to_le_bytes());
    header.extend_from_slice(&checksum);

    let parent = path.parent().ok_or(AppError::ProjectIo(
        "the render cache path has no parent".into(),
    ))?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".photoforge-tile-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|error| AppError::ProjectIo(format!("could not stage a render tile: {error}")))?;
    temporary
        .write_all(&header)
        .and_then(|_| temporary.write_all(&payload))
        .and_then(|_| temporary.flush())
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|error| AppError::ProjectIo(format!("could not write a render tile: {error}")))?;
    let temporary_path = temporary.into_temp_path();
    if path.exists() {
        remove_owned(path).map_err(|error| {
            AppError::ProjectIo(format!("could not replace a render tile: {error}"))
        })?;
    }
    fs::rename(&temporary_path, path).map_err(|error| {
        AppError::ProjectIo(format!("could not publish a render tile: {error}"))
    })?;
    Ok(())
}

fn read_tile(path: &Path) -> Option<FloatImage> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return None;
    }
    if metadata.len() < HEADER_BYTES as u64
        || metadata.len() > HEADER_BYTES as u64 + MAX_DISK_TILE_BYTES
    {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.len() < HEADER_BYTES || &bytes[0..8] != MAGIC {
        return None;
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().ok()?);
    if version != VERSION {
        return None;
    }
    let width = u32::from_le_bytes(bytes[12..16].try_into().ok()?);
    let height = u32::from_le_bytes(bytes[16..20].try_into().ok()?);
    let payload_len = u64::from_le_bytes(bytes[20..28].try_into().ok()?);
    if payload_len == 0
        || payload_len > MAX_DISK_TILE_BYTES
        || payload_len as usize != bytes.len().saturating_sub(HEADER_BYTES)
    {
        return None;
    }
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(16))?;
    if expected != payload_len {
        return None;
    }
    let payload = &bytes[HEADER_BYTES..];
    let actual: [u8; 32] = Sha256::digest(payload).into();
    if actual != bytes[28..60] {
        return None;
    }
    let count = usize::try_from(payload_len / 16).ok()?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(count).ok()?;
    for chunk in payload.chunks_exact(16) {
        pixels.push(FloatRgba::new(
            f32::from_bits(u32::from_le_bytes(chunk[0..4].try_into().ok()?)),
            f32::from_bits(u32::from_le_bytes(chunk[4..8].try_into().ok()?)),
            f32::from_bits(u32::from_le_bytes(chunk[8..12].try_into().ok()?)),
            f32::from_bits(u32::from_le_bytes(chunk[12..16].try_into().ok()?)),
        ));
    }
    FloatImage::new(width, height, pixels).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(value: u8) -> TileKey {
        let mut key = [0u8; 32];
        key[0] = value;
        key
    }

    fn image(seed: f32) -> FloatImage {
        FloatImage::new(
            3,
            2,
            (0..6)
                .map(|index| FloatRgba::new(seed + index as f32 * 0.01, 0.2, 0.3, 1.0))
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn round_trip_uses_a_fixed_safe_filename() {
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskTileCache::open(directory.path(), 1024 * 1024).unwrap();
        let expected = image(0.1);
        cache.insert(key(7), &expected);
        assert_eq!(cache.get(&key(7)), Some(expected));
        assert_eq!(cache.stats().hits, 1);
        let files: Vec<_> = fs::read_dir(directory.path()).unwrap().flatten().collect();
        assert_eq!(files.len(), 1);
        assert_eq!(
            files[0].path().extension().and_then(|v| v.to_str()),
            Some("tile")
        );
        assert_eq!(files[0].file_name().to_string_lossy().len(), 69);
    }

    #[test]
    fn corruption_version_and_truncation_are_misses_and_are_removed() {
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskTileCache::open(directory.path(), 1024 * 1024).unwrap();
        let path = cache.path_for(&key(1));
        fs::write(&path, b"bad").unwrap();
        assert!(cache.get(&key(1)).is_none());
        assert!(!path.exists());

        let expected = image(0.4);
        cache.insert(key(2), &expected);
        let mut bytes = fs::read(cache.path_for(&key(2))).unwrap();
        bytes[8] = 99;
        fs::write(cache.path_for(&key(2)), &bytes[..bytes.len() - 1]).unwrap();
        assert!(cache.get(&key(2)).is_none());
    }

    #[test]
    fn disk_budget_evicts_old_entries_deterministically() {
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskTileCache::open(directory.path(), 2 * 3 * 2 * 16).unwrap();
        cache.insert(key(1), &image(0.1));
        cache.insert(key(2), &image(0.2));
        cache.insert(key(3), &image(0.3));
        assert!(cache.get(&key(1)).is_none());
        assert!(cache.get(&key(2)).is_some());
        assert!(cache.get(&key(3)).is_some());
        assert!(cache.stats().evictions > 0);
    }

    #[test]
    fn clear_does_not_remove_unowned_files() {
        let directory = tempfile::tempdir().unwrap();
        let cache = DiskTileCache::open(directory.path(), 1024 * 1024).unwrap();
        let sentinel = directory.path().join("keep-me.txt");
        fs::write(&sentinel, b"not cache data").unwrap();
        cache.insert(key(9), &image(0.9));
        cache.clear();
        assert!(sentinel.exists());
        assert!(!cache.path_for(&key(9)).exists());
    }
}
