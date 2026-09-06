//! Text as text.
//!
//! A text layer stores the characters the user typed, the font they asked for,
//! and how it should be set. It stores no pixels. Glyphs are shaped and
//! outlined when something needs to draw them, and the outlines go through the
//! same float coverage rasteriser vector shapes use — so a text layer scaled to
//! 400% is re-rasterised at 400%, not resampled from a 100% bitmap.
//!
//! Shaping itself is cosmic-text's: the Unicode bidirectional algorithm,
//! contextual forms, cluster handling and cross-face fallback are hard enough
//! that implementing them here would be a downgrade dressed as independence.
pub mod font;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};

use crate::error::AppError;

pub use font::{
    available_families, family_is_available, shape as shape_uncached, PositionedGlyph,
    ShapeRequest, ShapedText, TextAlign, MAX_FONT_NAME_CHARS, MAX_FONT_SIZE, MAX_TEXT_BYTES,
    MIN_FONT_SIZE,
};

/// How many distinct shaped results are kept.
///
/// Small on purpose. The cache exists so that the tiles of one frame share one
/// shaping pass, not so that every string a session ever saw stays resident.
const CACHE_CAPACITY: usize = 64;

/// Shaped results, keyed by the request that produced them.
///
/// Rendering is tiled, so a single frame asks for the same text many times from
/// several threads at once. Shaping needs `&mut FontSystem` and therefore a
/// lock; without this cache those threads would queue behind each other to
/// recompute an identical answer. With it they take the lock once, briefly, and
/// then read outlines in parallel.
static CACHE: Mutex<Vec<(u64, Arc<ShapedText>)>> = Mutex::new(Vec::new());

fn cache_key(request: &ShapeRequest<'_>) -> u64 {
    let mut hasher = DefaultHasher::new();
    request.text.hash(&mut hasher);
    request.family.hash(&mut hasher);
    // Hashed by bit pattern, so the key distinguishes values the layout
    // distinguishes. -0.0 and 0.0 hash apart, which costs one redundant entry
    // in a case that does not arise and is preferable to two different layouts
    // sharing a key.
    request.size.to_bits().hash(&mut hasher);
    request.line_height.to_bits().hash(&mut hasher);
    request.letter_spacing.to_bits().hash(&mut hasher);
    request.weight.hash(&mut hasher);
    request.italic.hash(&mut hasher);
    request.align.hash(&mut hasher);
    request.wrap_width.map(f32::to_bits).hash(&mut hasher);
    hasher.finish()
}

/// Shapes text, reusing an identical earlier result when there is one.
///
/// The cache is keyed by every input the layout depends on, so a hit is the
/// same answer shaping would have produced and not an approximation of it.
pub fn shape(request: &ShapeRequest<'_>) -> Result<Arc<ShapedText>, AppError> {
    let key = cache_key(request);
    {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(index) = cache.iter().position(|(existing, _)| *existing == key) {
            // Most recently used moves to the front, so the entries a frame is
            // actively using are the last to be evicted.
            let entry = cache.remove(index);
            let shaped = Arc::clone(&entry.1);
            cache.insert(0, entry);
            return Ok(shaped);
        }
    }

    // Deliberately shaped without holding the cache lock. Shaping takes its own
    // lock on the font system, and holding both would let a slow shape block
    // every cache hit in the frame.
    let shaped = Arc::new(font::shape(request)?);

    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if !cache.iter().any(|(existing, _)| *existing == key) {
        cache.insert(0, (key, Arc::clone(&shaped)));
        cache.truncate(CACHE_CAPACITY);
    }
    Ok(shaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(text: &str) -> ShapeRequest<'_> {
        ShapeRequest {
            text,
            family: "",
            size: 24.0,
            line_height: 1.2,
            letter_spacing: 0.0,
            weight: 400,
            italic: false,
            align: TextAlign::Start,
            wrap_width: None,
        }
    }

    /// A cache that returned something other than what shaping would have
    /// produced would be a correctness bug disguised as an optimisation.
    #[test]
    fn a_cache_hit_matches_a_fresh_shape() {
        let fresh = font::shape(&request("caching")).expect("shape");
        let first = shape(&request("caching")).expect("shape");
        let second = shape(&request("caching")).expect("shape");
        assert!(Arc::ptr_eq(&first, &second), "the second call missed");

        assert_eq!(first.glyphs.len(), fresh.glyphs.len());
        for (cached, direct) in first.glyphs.iter().zip(fresh.glyphs.iter()) {
            assert_eq!(cached.x, direct.x);
            assert_eq!(cached.y, direct.y);
            assert_eq!(cached.path.commands, direct.path.commands);
        }
    }

    /// Every field of the request has to take part in the key, or one layout
    /// gets served for another.
    #[test]
    fn every_input_changes_the_key() {
        let base = request("key");
        let mut variants = vec![base];
        let mut size = request("key");
        size.size = 25.0;
        variants.push(size);
        let mut height = request("key");
        height.line_height = 1.5;
        variants.push(height);
        let mut spacing = request("key");
        spacing.letter_spacing = 2.0;
        variants.push(spacing);
        let mut weight = request("key");
        weight.weight = 700;
        variants.push(weight);
        let mut italic = request("key");
        italic.italic = true;
        variants.push(italic);
        let mut align = request("key");
        align.align = TextAlign::Center;
        variants.push(align);
        let mut wrap = request("key");
        wrap.wrap_width = Some(40.0);
        variants.push(wrap);
        let other = request("different");
        variants.push(other);

        let keys: Vec<u64> = variants.iter().map(cache_key).collect();
        for (index, key) in keys.iter().enumerate() {
            for (other_index, other_key) in keys.iter().enumerate() {
                if index != other_index {
                    assert_ne!(key, other_key, "requests {index} and {other_index} collided");
                }
            }
        }
    }

    #[test]
    fn the_cache_stays_bounded() {
        for index in 0..CACHE_CAPACITY * 2 {
            shape(&request(&format!("bounded {index}"))).expect("shape");
        }
        let cache = CACHE.lock().expect("lock");
        assert!(cache.len() <= CACHE_CAPACITY, "the cache grew to {}", cache.len());
    }
}
