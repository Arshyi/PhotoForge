//! Making OpenEXR files for tests, with exactly the channels, sample types, windows and attributes
//! a test needs. The files are written by the same crate that reads them, which is why the
//! round-trip tests are also checked against an independent reader (see docs/openexr.md).
use exr::prelude::*;
use std::path::{Path, PathBuf};

/// The samples of one channel, row-major.
#[derive(Clone)]
pub enum Samples {
    F32(Vec<f32>),
    /// Held as f32 and written as half.
    F16(Vec<f32>),
    U32(Vec<u32>),
}

pub struct TestFile {
    pub width: usize,
    pub height: usize,
    pub channels: Vec<(String, Samples)>,
    pub compression: Compression,
    pub tiles: Option<usize>,
    pub layer_name: Option<String>,
    /// The position of the data window; the display window is (0, 0, display).
    pub position: (i32, i32),
    pub display: Option<(usize, usize)>,
    /// Red, green, blue, white; `None` writes no attribute (Rec. 709 by convention).
    pub chromaticities: Option<[(f32, f32); 4]>,
    pub pixel_aspect: f32,
}

impl TestFile {
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            width,
            height,
            channels: Vec::new(),
            compression: Compression::ZIP16,
            tiles: None,
            layer_name: None,
            position: (0, 0),
            display: None,
            chromaticities: None,
            pixel_aspect: 1.0,
        }
    }

    /// A channel whose value at `(x, y)` is `value(x, y)`, as single precision.
    pub fn f32_channel(mut self, name: &str, value: impl Fn(usize, usize) -> f32) -> Self {
        let samples = self.samples(value);
        self.channels
            .push((name.to_string(), Samples::F32(samples)));
        self
    }

    /// The same, written as half precision.
    pub fn f16_channel(mut self, name: &str, value: impl Fn(usize, usize) -> f32) -> Self {
        let samples = self.samples(value);
        self.channels
            .push((name.to_string(), Samples::F16(samples)));
        self
    }

    pub fn u32_channel(mut self, name: &str, value: impl Fn(usize, usize) -> u32) -> Self {
        let mut samples = Vec::with_capacity(self.width * self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                samples.push(value(x, y));
            }
        }
        self.channels
            .push((name.to_string(), Samples::U32(samples)));
        self
    }

    fn samples(&self, value: impl Fn(usize, usize) -> f32) -> Vec<f32> {
        let mut samples = Vec::with_capacity(self.width * self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                samples.push(value(x, y));
            }
        }
        samples
    }

    pub fn compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    pub fn tiled(mut self, tile: usize) -> Self {
        self.tiles = Some(tile);
        self
    }

    pub fn named(mut self, name: &str) -> Self {
        self.layer_name = Some(name.to_string());
        self
    }

    pub fn at(mut self, x: i32, y: i32) -> Self {
        self.position = (x, y);
        self
    }

    pub fn display(mut self, width: usize, height: usize) -> Self {
        self.display = Some((width, height));
        self
    }

    pub fn chromaticities(mut self, values: [(f32, f32); 4]) -> Self {
        self.chromaticities = Some(values);
        self
    }

    pub fn pixel_aspect(mut self, aspect: f32) -> Self {
        self.pixel_aspect = aspect;
        self
    }

    fn layer(&self) -> Layer<AnyChannels<FlatSamples>> {
        let mut list: SmallVec<[AnyChannel<FlatSamples>; 4]> = SmallVec::new();
        for (name, samples) in &self.channels {
            let flat = match samples {
                Samples::F32(values) => FlatSamples::F32(values.clone()),
                Samples::F16(values) => {
                    FlatSamples::F16(values.iter().map(|v| f16::from_f32(*v)).collect())
                }
                Samples::U32(values) => FlatSamples::U32(values.clone()),
            };
            list.push(AnyChannel::new(name.as_str(), flat));
        }
        let mut attributes = match &self.layer_name {
            Some(name) => LayerAttributes::named(name.as_str()),
            None => LayerAttributes::default(),
        };
        attributes.layer_position = Vec2(self.position.0, self.position.1);
        let encoding = Encoding {
            compression: self.compression,
            blocks: match self.tiles {
                Some(size) => Blocks::Tiles(Vec2(size, size)),
                None => Blocks::ScanLines,
            },
            line_order: if self.tiles.is_some() {
                LineOrder::Unspecified
            } else {
                LineOrder::Increasing
            },
        };
        Layer::new(
            (self.width, self.height),
            attributes,
            encoding,
            AnyChannels::sort(list),
        )
    }

    fn attributes(&self) -> ImageAttributes {
        let (display_width, display_height) = self.display.unwrap_or((self.width, self.height));
        let mut attributes = ImageAttributes::new(IntegerBounds::new(
            Vec2(0, 0),
            Vec2(display_width, display_height),
        ));
        attributes.pixel_aspect = self.pixel_aspect;
        if let Some([r, g, b, w]) = self.chromaticities {
            attributes.chromaticities = Some(exr::meta::attribute::Chromaticities {
                red: Vec2(r.0, r.1),
                green: Vec2(g.0, g.1),
                blue: Vec2(b.0, b.1),
                white: Vec2(w.0, w.1),
            });
        }
        attributes
    }

    pub fn bytes(&self) -> Vec<u8> {
        let image = Image::empty(self.attributes()).with_layer(self.layer());
        let mut out = std::io::Cursor::new(Vec::new());
        image
            .write()
            .non_parallel()
            .to_buffered(&mut out)
            .expect("the test file is writable");
        out.into_inner()
    }

    pub fn write_to(&self, directory: &Path, name: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, self.bytes()).unwrap();
        path
    }
}

/// A multi-part file: each `TestFile` becomes one part, named.
pub fn multipart(parts: &[TestFile]) -> Vec<u8> {
    let mut layers: SmallVec<[Layer<AnyChannels<FlatSamples>>; 2]> = SmallVec::new();
    for part in parts {
        layers.push(part.layer());
    }
    let image = Image::from_layers(parts[0].attributes(), layers);
    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write()
        .non_parallel()
        .to_buffered(&mut out)
        .expect("the multipart file is writable");
    out.into_inner()
}

/// Overwrites the one-byte `compression` attribute of a file's first header.
pub fn patch_compression(bytes: &mut [u8], code: u8) {
    let key = b"compression\0compression\0";
    let at = bytes
        .windows(key.len())
        .position(|window| window == key)
        .expect("the file has a compression attribute");
    // name\0 type\0 size(i32 = 1) value(u8)
    let value = at + key.len() + 4;
    assert_eq!(bytes[at + key.len()], 1);
    bytes[value] = code;
}

/// Overwrites the tile size of a tiled file's first header (its `tiles` attribute).
pub fn patch_tile_size(bytes: &mut [u8], width: u32, height: u32) {
    let key = b"tiles\0tiledesc\0";
    let at = bytes
        .windows(key.len())
        .position(|window| window == key)
        .expect("the file has a tiles attribute");
    // name\0 type\0 size(i32 = 9) x(u32) y(u32) mode(u8)
    let value = at + key.len() + 4;
    assert_eq!(bytes[at + key.len()], 9);
    bytes[value..value + 4].copy_from_slice(&width.to_le_bytes());
    bytes[value + 4..value + 8].copy_from_slice(&height.to_le_bytes());
}

/// The length of the first header of a file: its attributes and the byte that ends them.
pub fn header_len(bytes: &[u8]) -> usize {
    let mut at = 8; // the magic number and the version
    while bytes[at] != 0 {
        // name\0 type\0 size(i32) value
        for _ in 0..2 {
            at += bytes[at..].iter().position(|&byte| byte == 0).unwrap() + 1;
        }
        let size = i32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        at += 4 + size as usize;
    }
    at + 1
}

/// One attribute as a file stores it.
pub fn attribute(name: &[u8], kind: &[u8], value: &[u8]) -> Vec<u8> {
    let mut out = [name, &[0], kind, &[0]].concat();
    out.extend_from_slice(&(value.len() as i32).to_le_bytes());
    out.extend_from_slice(value);
    out
}

/// Adds attributes, as `attribute` makes them, to the header of a file with one part, and moves
/// the positions of its blocks by as much, so that it is still a file the reader can read. (The
/// first position in the offset table is taken to be the lowest, as the writer makes it.)
pub fn add_header_attributes(bytes: &[u8], attributes: &[u8]) -> Vec<u8> {
    let end = header_len(bytes);
    let first = u64::from_le_bytes(bytes[end..end + 8].try_into().unwrap()) as usize;
    let mut out = bytes[..end - 1].to_vec();
    out.extend_from_slice(attributes);
    out.extend_from_slice(&bytes[end - 1..]);
    let table = end - 1 + attributes.len() + 1;
    for entry in (0..(first - end) / 8).map(|entry| table + 8 * entry) {
        let moved =
            u64::from_le_bytes(out[entry..entry + 8].try_into().unwrap()) + attributes.len() as u64;
        out[entry..entry + 8].copy_from_slice(&moved.to_le_bytes());
    }
    out
}

/// Replaces the stored bytes of the one block of a file whose picture is a single tile, keeping
/// the header: a block the writer would never make, as a hostile file has.
pub fn replace_single_block(bytes: &[u8], stored: &[u8]) -> Vec<u8> {
    // The offset table is one u64, the block's own position, and the block follows it.
    let mut starts = (8..bytes.len()).filter(|&at| bytes[at - 8..at] == (at as u64).to_le_bytes());
    let start = starts.next().expect("the file has an offset table");
    assert!(starts.next().is_none(), "the block position is ambiguous");
    // x, y and level of the tile (four i32) are kept; the i32 size and the bytes are replaced.
    assert!(bytes.len() > start + 20, "the file has a block");
    let mut out = bytes[..start + 16].to_vec();
    out.extend_from_slice(&(stored.len() as i32).to_le_bytes());
    out.extend_from_slice(stored);
    out
}

mod heap {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    thread_local! {
        // Plain counters without a destructor, so that touching them in the allocator is safe at
        // any point in a thread's life, and per thread, so that tests running beside this one
        // are not counted.
        static HELD: Cell<isize> = const { Cell::new(0) };
        static PEAK: Cell<isize> = const { Cell::new(0) };
    }

    fn grow(bytes: usize) {
        let held = HELD.get() + bytes as isize;
        HELD.set(held);
        if held > PEAK.get() {
            PEAK.set(held);
        }
    }

    struct Counting;

    // SAFETY: every call is forwarded to the system allocator unchanged; only sizes are counted.
    // (`realloc` is the default one, an allocation, a copy and a free, which counts the old and
    // the new block together as an allocator that cannot resize in place holds them.)
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let pointer = System.alloc(layout);
            if !pointer.is_null() {
                grow(layout.size());
            }
            pointer
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            System.dealloc(pointer, layout);
            HELD.set(HELD.get() - layout.size() as isize);
        }
    }

    #[global_allocator]
    static ALLOCATOR: Counting = Counting;

    /// Runs `work` and returns its result with the most heap this thread held at once above where
    /// it began.
    pub fn peak_heap<T>(work: impl FnOnce() -> T) -> (T, u64) {
        let base = HELD.get();
        PEAK.set(base);
        let result = work();
        (result, (PEAK.get() - base).max(0) as u64)
    }
}

pub use heap::peak_heap;
