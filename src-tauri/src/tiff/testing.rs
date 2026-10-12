//! A TIFF writer for the tests: any byte order, depth, layout, compression and predictor the
//! reader takes, so that what is read can be compared with what was meant to be written.
//!
//! It is deliberately written apart from the reader's code (and from the exporter's), and the
//! real files in `tests/tiff_real_files.rs` are written by programs that are neither.
use super::codec::testing::{lzw_encode, packbits_encode};

pub struct Tiff {
    pub width: u32,
    pub height: u32,
    pub big: bool,
    pub bits: u16,
    pub float: bool,
    pub samples: usize,
    pub photometric: u16,
    pub planar: bool,
    pub compression: u16,
    pub predictor: u16,
    pub rows_per_strip: u32,
    pub tile: Option<(u32, u32)>,
    pub extra_samples: Vec<u16>,
    pub palette: Vec<[u16; 3]>,
    pub orientation: Option<u16>,
    pub icc: Option<Vec<u8>>,
    pub fill_order: Option<u16>,
    pub new_subfile_type: Option<u32>,
    /// width × height × samples values, interleaved. Integers as their full-scale value, floats
    /// as themselves.
    pub data: Vec<f64>,
    /// (tag, field type, value bytes in the file's byte order)
    pub extra_tags: Vec<(u16, u16, Vec<u8>)>,
    /// A second, reduced picture after the first, as a camera writes its thumbnail.
    pub with_thumbnail: bool,
}

impl Tiff {
    pub fn rgb8(width: u32, height: u32, data: Vec<f64>) -> Self {
        Self {
            width,
            height,
            big: false,
            bits: 8,
            float: false,
            samples: 3,
            photometric: 2,
            planar: false,
            compression: 1,
            predictor: 1,
            rows_per_strip: height,
            tile: None,
            extra_samples: Vec::new(),
            palette: Vec::new(),
            orientation: None,
            icc: None,
            fill_order: None,
            new_subfile_type: None,
            data,
            extra_tags: Vec::new(),
            with_thumbnail: false,
        }
    }

    fn put16(&self, value: u16) -> [u8; 2] {
        if self.big {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        }
    }

    fn put32(&self, value: u32) -> [u8; 4] {
        if self.big {
            value.to_be_bytes()
        } else {
            value.to_le_bytes()
        }
    }

    fn shorts(&self, values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| self.put16(*v)).collect()
    }

    fn longs(&self, values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| self.put32(*v)).collect()
    }

    /// One sample as bytes in the file's order.
    fn sample_bytes(&self, value: f64, out: &mut Vec<u8>) {
        match (self.float, self.bits) {
            (false, 8) => out.push(value as u8),
            (false, 16) => out.extend_from_slice(&self.put16(value as u16)),
            (false, 32) => out.extend_from_slice(&self.put32(value as u32)),
            (true, 16) => out.extend_from_slice(&self.put16(f32_to_half(value as f32))),
            (true, 32) => out.extend_from_slice(&self.put32((value as f32).to_bits())),
            (true, 64) => {
                let bits = value.to_bits();
                let (high, low) = ((bits >> 32) as u32, bits as u32);
                if self.big {
                    out.extend_from_slice(&high.to_be_bytes());
                    out.extend_from_slice(&low.to_be_bytes());
                } else {
                    out.extend_from_slice(&low.to_le_bytes());
                    out.extend_from_slice(&high.to_le_bytes());
                }
            }
            _ => panic!("the test writer does not write this depth"),
        }
    }

    /// The rows of one segment as bytes, before the predictor: `group` samples per pixel,
    /// beginning at sample `first`, over the pixels (x0.., y0..) of the picture, padded with
    /// zeros to `stored_width × stored_height`.
    #[allow(clippy::too_many_arguments)]
    fn rows(
        &self,
        first: usize,
        group: usize,
        x0: u32,
        y0: u32,
        width: u32,
        height: u32,
        stored_width: u32,
        stored_height: u32,
    ) -> Vec<Vec<u8>> {
        let mut rows = Vec::new();
        for row in 0..stored_height {
            let mut bytes = Vec::new();
            if self.bits < 8 {
                let bits = usize::from(self.bits);
                let mut accumulator = 0_u16;
                let mut held = 0_usize;
                for column in 0..stored_width {
                    let inside = column < width && row < height;
                    let value = if inside {
                        let index = ((y0 + row) as usize * self.width as usize
                            + (x0 + column) as usize)
                            * self.samples
                            + first;
                        self.data[index] as u16
                    } else {
                        0
                    };
                    accumulator = (accumulator << bits) | value;
                    held += bits;
                    if held == 8 {
                        bytes.push(accumulator as u8);
                        accumulator = 0;
                        held = 0;
                    }
                }
                if held > 0 {
                    bytes.push((accumulator << (8 - held)) as u8);
                }
            } else {
                for column in 0..stored_width {
                    for sample in 0..group {
                        let inside = column < width && row < height;
                        let value = if inside {
                            let index = ((y0 + row) as usize * self.width as usize
                                + (x0 + column) as usize)
                                * self.samples
                                + first
                                + sample;
                            self.data[index]
                        } else {
                            0.0
                        };
                        self.sample_bytes(value, &mut bytes);
                    }
                }
            }
            rows.push(bytes);
        }
        rows
    }

    fn predict(&self, row: &mut [u8], group: usize) {
        match self.predictor {
            2 => {
                let unit = usize::from(self.bits / 8);
                let words = row.len() / unit;
                for index in (group..words).rev() {
                    match unit {
                        1 => {
                            row[index] = row[index].wrapping_sub(row[index - group]);
                        }
                        2 => {
                            let read = |at: usize| -> u16 {
                                let pair = [row[at * 2], row[at * 2 + 1]];
                                if self.big {
                                    u16::from_be_bytes(pair)
                                } else {
                                    u16::from_le_bytes(pair)
                                }
                            };
                            let difference = read(index).wrapping_sub(read(index - group));
                            let bytes = self.put16(difference);
                            row[index * 2] = bytes[0];
                            row[index * 2 + 1] = bytes[1];
                        }
                        4 => {
                            let read = |at: usize| -> u32 {
                                let quad = [
                                    row[at * 4],
                                    row[at * 4 + 1],
                                    row[at * 4 + 2],
                                    row[at * 4 + 3],
                                ];
                                if self.big {
                                    u32::from_be_bytes(quad)
                                } else {
                                    u32::from_le_bytes(quad)
                                }
                            };
                            let difference = read(index).wrapping_sub(read(index - group));
                            row[index * 4..index * 4 + 4].copy_from_slice(&self.put32(difference));
                        }
                        _ => panic!("no predictor 2 at this depth"),
                    }
                }
            }
            3 => {
                let unit = usize::from(self.bits / 8);
                let values = row.len() / unit;
                // Most significant byte of every value first, whatever the file's order.
                let mut shuffled = vec![0_u8; row.len()];
                for value in 0..values {
                    let mut bytes = row[value * unit..(value + 1) * unit].to_vec();
                    if !self.big {
                        bytes.reverse();
                    }
                    for (plane, byte) in bytes.iter().enumerate() {
                        shuffled[plane * values + value] = *byte;
                    }
                }
                for index in (group..shuffled.len()).rev() {
                    shuffled[index] = shuffled[index].wrapping_sub(shuffled[index - group]);
                }
                row.copy_from_slice(&shuffled);
            }
            _ => {}
        }
    }

    fn compress(&self, bytes: &[u8]) -> Vec<u8> {
        match self.compression {
            1 => bytes.to_vec(),
            5 => lzw_encode(bytes),
            8 | 32946 => miniz_oxide::deflate::compress_to_vec_zlib(bytes, 6),
            32773 => packbits_encode(bytes),
            other => panic!("the test writer does not compress with {other}"),
        }
    }

    /// The file.
    pub fn build(&self) -> Vec<u8> {
        let mut out = vec![0_u8; 8];
        out[..2].copy_from_slice(if self.big { b"MM" } else { b"II" });
        out[2..4].copy_from_slice(&self.put16(42));

        let planes = if self.planar { self.samples } else { 1 };
        let group = if self.planar { 1 } else { self.samples };
        let mut offsets: Vec<u32> = Vec::new();
        let mut counts: Vec<u32> = Vec::new();
        let mut push = |out: &mut Vec<u8>, bytes: Vec<u8>| {
            offsets.push(out.len() as u32);
            counts.push(bytes.len() as u32);
            out.extend_from_slice(&bytes);
        };
        for plane in 0..planes {
            let first = if self.planar { plane } else { 0 };
            match self.tile {
                None => {
                    let rps = self.rows_per_strip.clamp(1, self.height);
                    let mut y = 0;
                    while y < self.height {
                        let height = rps.min(self.height - y);
                        let mut rows =
                            self.rows(first, group, 0, y, self.width, height, self.width, height);
                        let mut bytes = Vec::new();
                        for row in &mut rows {
                            self.predict(row, group);
                            bytes.extend_from_slice(row);
                        }
                        let compressed = self.compress(&bytes);
                        push(&mut out, compressed);
                        y += height;
                    }
                }
                Some((tile_width, tile_height)) => {
                    let mut y = 0;
                    while y < self.height {
                        let mut x = 0;
                        while x < self.width {
                            let mut rows = self.rows(
                                first,
                                group,
                                x,
                                y,
                                tile_width.min(self.width - x),
                                tile_height.min(self.height - y),
                                tile_width,
                                tile_height,
                            );
                            let mut bytes = Vec::new();
                            for row in &mut rows {
                                self.predict(row, group);
                                bytes.extend_from_slice(row);
                            }
                            let compressed = self.compress(&bytes);
                            push(&mut out, compressed);
                            x += tile_width;
                        }
                        y += tile_height;
                    }
                }
            }
            if out.len() % 2 == 1 {
                out.push(0);
            }
        }

        // The directory, with its external values after it.
        struct Entry {
            tag: u16,
            kind: u16,
            bytes: Vec<u8>,
        }
        let width_of = |kind: u16| match kind {
            3 | 8 => 2,
            4 | 9 | 11 => 4,
            5 | 10 | 12 => 8,
            _ => 1,
        };
        let mut entries: Vec<Entry> = Vec::new();
        let mut add =
            |tag: u16, kind: u16, bytes: Vec<u8>| entries.push(Entry { tag, kind, bytes });
        if let Some(subfile) = self.new_subfile_type {
            add(254, 4, self.longs(&[subfile]));
        }
        add(256, 4, self.longs(&[self.width]));
        add(257, 4, self.longs(&[self.height]));
        add(258, 3, self.shorts(&vec![self.bits; self.samples]));
        add(259, 3, self.shorts(&[self.compression]));
        add(262, 3, self.shorts(&[self.photometric]));
        if let Some(fill) = self.fill_order {
            add(266, 3, self.shorts(&[fill]));
        }
        if self.tile.is_none() {
            add(273, 4, self.longs(&offsets));
            add(
                278,
                4,
                self.longs(&[self.rows_per_strip.clamp(1, self.height)]),
            );
            add(279, 4, self.longs(&counts));
        } else {
            let (tile_width, tile_height) = self.tile.unwrap_or((16, 16));
            add(322, 4, self.longs(&[tile_width]));
            add(323, 4, self.longs(&[tile_height]));
            add(324, 4, self.longs(&offsets));
            add(325, 4, self.longs(&counts));
        }
        if let Some(orientation) = self.orientation {
            add(274, 3, self.shorts(&[orientation]));
        }
        add(277, 3, self.shorts(&[self.samples as u16]));
        add(284, 3, self.shorts(&[if self.planar { 2 } else { 1 }]));
        if self.predictor != 1 {
            add(317, 3, self.shorts(&[self.predictor]));
        }
        if !self.palette.is_empty() {
            let entries_count = self.palette.len();
            let mut map = Vec::new();
            for channel in 0..3 {
                for colour in &self.palette {
                    map.push(colour[channel]);
                }
            }
            assert_eq!(map.len(), entries_count * 3);
            add(320, 3, self.shorts(&map));
        }
        if !self.extra_samples.is_empty() {
            add(338, 3, self.shorts(&self.extra_samples));
        }
        if self.float {
            add(339, 3, self.shorts(&vec![3; self.samples]));
        }
        if let Some(profile) = &self.icc {
            add(34675, 7, profile.clone());
        }
        for (tag, kind, bytes) in &self.extra_tags {
            add(*tag, *kind, bytes.clone());
        }
        entries.sort_by_key(|entry| entry.tag);

        let directory_at = out.len() as u32;
        let directory_len = 2 + entries.len() * 12 + 4;
        let thumbnail_at = if self.with_thumbnail {
            // After this directory's values: found below.
            0
        } else {
            0
        };
        let _ = thumbnail_at;
        let mut values: Vec<u8> = Vec::new();
        let values_start = directory_at as usize + directory_len;
        let mut directory = Vec::new();
        directory.extend_from_slice(&self.put16(entries.len() as u16));
        for entry in &entries {
            directory.extend_from_slice(&self.put16(entry.tag));
            directory.extend_from_slice(&self.put16(entry.kind));
            directory
                .extend_from_slice(&self.put32((entry.bytes.len() / width_of(entry.kind)) as u32));
            if entry.bytes.len() <= 4 {
                let mut inline = entry.bytes.clone();
                inline.resize(4, 0);
                directory.extend_from_slice(&inline);
            } else {
                directory.extend_from_slice(&self.put32((values_start + values.len()) as u32));
                values.extend_from_slice(&entry.bytes);
                if values.len() % 2 == 1 {
                    values.push(0);
                }
            }
        }
        let next_at = if self.with_thumbnail {
            (values_start + values.len()) as u32
        } else {
            0
        };
        directory.extend_from_slice(&self.put32(next_at));
        out.extend_from_slice(&directory);
        out.extend_from_slice(&values);
        if self.with_thumbnail {
            // A one-pixel reduced-resolution picture, so the walk has a second directory.
            let data_at = out.len() as u32 + 2 + 7 * 12 + 4;
            let mut thumbnail = Vec::new();
            thumbnail.extend_from_slice(&self.put16(7));
            let mut put = |tag: u16, kind: u16, value: u32| {
                thumbnail.extend_from_slice(&self.put16(tag));
                thumbnail.extend_from_slice(&self.put16(kind));
                thumbnail.extend_from_slice(&self.put32(1));
                if kind == 3 {
                    thumbnail.extend_from_slice(&self.put16(value as u16));
                    thumbnail.extend_from_slice(&[0, 0]);
                } else {
                    thumbnail.extend_from_slice(&self.put32(value));
                }
            };
            put(254, 4, 1);
            put(256, 4, 1);
            put(257, 4, 1);
            put(262, 3, 1);
            put(273, 4, data_at);
            put(277, 3, 1);
            put(279, 4, 1);
            thumbnail.extend_from_slice(&self.put32(0));
            thumbnail.push(0x80);
            out.extend_from_slice(&thumbnail);
        }
        out[4..8].copy_from_slice(&self.put32(directory_at));
        out
    }
}

/// Half-precision encoding for the values the tests use (normal numbers and zero).
fn f32_to_half(value: f32) -> u16 {
    if value == 0.0 {
        return if value.is_sign_negative() { 0x8000 } else { 0 };
    }
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xFF) as i32 - 127 + 15;
    let fraction = ((bits >> 13) & 0x03FF) as u16;
    assert!((1..31).contains(&exponent), "outside what the tests write");
    sign | ((exponent as u16) << 10) | fraction
}

/// A synthetic picture whose every pixel differs, so a transposed or offset read shows at once.
pub fn ramp(width: u32, height: u32, samples: usize, max: f64) -> Vec<f64> {
    let mut data = Vec::new();
    for y in 0..height {
        for x in 0..width {
            for s in 0..samples {
                let value = ((x * 7 + y * 13 + s as u32 * 29) % 251) as f64 / 250.0 * max;
                data.push(value.round());
            }
        }
    }
    data
}
