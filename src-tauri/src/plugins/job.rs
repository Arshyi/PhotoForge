//! What one call to a plugin's filter is given and returns. Shared by the real runtime
//! and by the stand-in a build without the `plugins` feature carries.
use crate::source::Rect;

/// One call: the pixels in, where they sit, and what to produce.
pub struct TileJob<'a> {
    pub filter_index: u32,
    pub parameters: &'a [f64],
    pub image_width: u32,
    pub image_height: u32,
    /// The window of the image the module is given, and its pixels as RGBA `f32`.
    pub input_rect: Rect,
    pub input: &'a [u8],
    /// The rectangle of the image the module must produce.
    pub output_rect: Rect,
}

pub struct TileResult {
    /// `output_rect` pixels as RGBA `f32`, little-endian.
    pub output: Vec<u8>,
    pub logs: Vec<String>,
    pub fuel_used: u64,
}
