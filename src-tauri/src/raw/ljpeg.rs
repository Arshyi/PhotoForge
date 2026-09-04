//! Lossless JPEG (ITU-T T.81 Annex H, SOF3) decoding for DNG.
//!
//! Adobe's DNG Converter, and every camera that writes compressed DNG, stores
//! each strip or tile as a complete lossless-JPEG stream. This is not the
//! familiar DCT JPEG: there is no transform and no quantisation, only a
//! predictor and Huffman-coded differences, so decoding is exact — the samples
//! that come out are bit-for-bit the samples the camera put in.
//!
//! CFA data is carried by encoding N adjacent sensor columns as N JPEG
//! "components", so a stream whose SOF3 declares width W with N components
//! holds W * N samples per row, interleaved component by component.
//!
//! Every length in the stream is attacker-controlled. Nothing is allocated
//! before the declared geometry is checked against a ceiling, and the bit
//! reader can never read past the end of its buffer.

/// Largest number of samples one stream may declare, which bounds allocation
/// before any decoding starts.
pub const MAX_SAMPLES: usize = 80_000_000;
/// Largest component count a lossless-JPEG scan may use.
pub const MAX_COMPONENTS: usize = 4;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LJpegError {
    #[error("the compressed strip is not a lossless JPEG stream")]
    NotLosslessJpeg,
    #[error("the compressed strip ends early: {0}")]
    Truncated(&'static str),
    #[error("the compressed strip is malformed: {0}")]
    Malformed(&'static str),
    #[error("the compressed strip declares more data than is allowed: {0}")]
    LimitExceeded(&'static str),
}

/// A decoded lossless-JPEG frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Samples in row-major order, already de-interleaved across components,
    /// so `width * components` values make up one row.
    pub samples: Vec<u16>,
    /// Samples per row, counting every component.
    pub width: usize,
    pub height: usize,
    pub components: usize,
    pub precision: u8,
}

/// A canonical JPEG Huffman table, decoded one bit at a time.
#[derive(Default, Clone)]
struct HuffmanTable {
    /// `min_code[l]` and `max_code[l]` bound the codes of length `l + 1`.
    min_code: [i32; 17],
    max_code: [i32; 17],
    /// Index into `values` of the first code of each length.
    value_index: [usize; 17],
    values: Vec<u8>,
}

impl HuffmanTable {
    fn build(counts: &[u8; 16], values: Vec<u8>) -> Self {
        let mut table = Self {
            values,
            ..Default::default()
        };
        let mut code = 0i32;
        let mut index = 0usize;
        for (length, declared) in counts.iter().enumerate() {
            table.value_index[length] = index;
            table.min_code[length] = code;
            let count = usize::from(*declared);
            code += count as i32;
            // An empty length has no valid code; -1 makes every comparison fail.
            table.max_code[length] = if count == 0 { -1 } else { code - 1 };
            index += count;
            code <<= 1;
        }
        table
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    position: usize,
    bits: u32,
    count: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            position: 0,
            bits: 0,
            count: 0,
        }
    }

    /// Reads one entropy-coded bit.
    ///
    /// `FF 00` is a stuffed literal `FF`; any other marker ends the entropy
    /// stream, at which point zero bits are supplied so a slightly short strip
    /// yields a dull image rather than an error part way through a frame.
    fn bit(&mut self) -> u32 {
        if self.count == 0 {
            let Some(byte) = self.data.get(self.position).copied() else {
                return 0;
            };
            self.position += 1;
            if byte == 0xFF {
                match self.data.get(self.position).copied() {
                    Some(0x00) => self.position += 1,
                    _ => {
                        // A real marker: stop consuming and feed zeroes.
                        self.position = self.data.len();
                        return 0;
                    }
                }
            }
            self.bits = u32::from(byte);
            self.count = 8;
        }
        self.count -= 1;
        (self.bits >> self.count) & 1
    }

    fn bits(&mut self, length: u32) -> u32 {
        let mut value = 0;
        for _ in 0..length {
            value = (value << 1) | self.bit();
        }
        value
    }

    fn decode(&mut self, table: &HuffmanTable) -> Result<u8, LJpegError> {
        let mut code = self.bit() as i32;
        for length in 0..16 {
            if table.max_code[length] >= code {
                let offset = table.value_index[length] + (code - table.min_code[length]) as usize;
                return table
                    .values
                    .get(offset)
                    .copied()
                    .ok_or(LJpegError::Malformed("Huffman code out of table"));
            }
            code = (code << 1) | self.bit() as i32;
        }
        Err(LJpegError::Malformed("Huffman code longer than 16 bits"))
    }

    /// Restarts at the next byte boundary, as a restart marker requires.
    fn restart(&mut self) {
        self.count = 0;
        while self.position + 1 < self.data.len() {
            if self.data[self.position] == 0xFF {
                let marker = self.data[self.position + 1];
                if (0xD0..=0xD7).contains(&marker) {
                    self.position += 2;
                    return;
                }
            }
            self.position += 1;
        }
        self.position = self.data.len();
    }
}

/// The signed difference a Huffman category and its extra bits encode.
fn extend(value: u32, category: u32) -> i32 {
    if category == 0 {
        return 0;
    }
    let threshold = 1i32 << (category - 1);
    let value = value as i32;
    if value < threshold {
        value - (1i32 << category) + 1
    } else {
        value
    }
}

struct Component {
    dc_table: usize,
}

/// Decodes one lossless-JPEG stream.
pub fn decode(data: &[u8]) -> Result<Frame, LJpegError> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(LJpegError::NotLosslessJpeg);
    }
    let mut position = 2usize;
    let mut tables: Vec<HuffmanTable> = vec![HuffmanTable::default(); 8];
    let mut precision = 0u8;
    let mut height = 0usize;
    let mut width = 0usize;
    let mut frame_components: Vec<usize> = Vec::new();
    let mut restart_interval = 0usize;

    loop {
        // Markers are byte-aligned and may be preceded by fill bytes.
        while position < data.len() && data[position] != 0xFF {
            position += 1;
        }
        while position < data.len() && data[position] == 0xFF {
            position += 1;
        }
        let Some(marker) = data.get(position).copied() else {
            return Err(LJpegError::Truncated("marker"));
        };
        position += 1;
        match marker {
            // SOF3: the only frame type lossless JPEG defines.
            0xC3 => {
                let segment = segment(data, position)?;
                if segment.len() < 6 {
                    return Err(LJpegError::Truncated("frame header"));
                }
                precision = segment[0];
                if !(2..=16).contains(&precision) {
                    return Err(LJpegError::Malformed("sample precision out of range"));
                }
                height = usize::from(u16::from_be_bytes([segment[1], segment[2]]));
                width = usize::from(u16::from_be_bytes([segment[3], segment[4]]));
                let count = usize::from(segment[5]);
                if count == 0 || count > MAX_COMPONENTS {
                    return Err(LJpegError::Malformed("unsupported component count"));
                }
                if segment.len() < 6 + count * 3 {
                    return Err(LJpegError::Truncated("component specification"));
                }
                frame_components = (0..count).collect();
                let samples = width
                    .checked_mul(height)
                    .and_then(|value| value.checked_mul(count))
                    .ok_or(LJpegError::LimitExceeded("frame geometry overflows"))?;
                if samples == 0 {
                    return Err(LJpegError::Malformed("frame has no samples"));
                }
                if samples > MAX_SAMPLES {
                    return Err(LJpegError::LimitExceeded("frame declares too many samples"));
                }
                position += segment.len() + 2;
            }
            // DHT: one or more Huffman tables.
            0xC4 => {
                let segment = segment(data, position)?;
                let mut offset = 0usize;
                while offset + 17 <= segment.len() {
                    let identifier = usize::from(segment[offset] & 0x0F);
                    let mut counts = [0u8; 16];
                    counts.copy_from_slice(&segment[offset + 1..offset + 17]);
                    let total: usize = counts.iter().map(|count| usize::from(*count)).sum();
                    if total > 256 || offset + 17 + total > segment.len() {
                        return Err(LJpegError::Malformed("Huffman table overruns its segment"));
                    }
                    let values = segment[offset + 17..offset + 17 + total].to_vec();
                    if identifier < tables.len() {
                        tables[identifier] = HuffmanTable::build(&counts, values);
                    }
                    offset += 17 + total;
                }
                position += segment.len() + 2;
            }
            // DRI: restart interval.
            0xDD => {
                let segment = segment(data, position)?;
                if segment.len() >= 2 {
                    restart_interval = usize::from(u16::from_be_bytes([segment[0], segment[1]]));
                }
                position += segment.len() + 2;
            }
            // SOS: the scan header, immediately followed by entropy data.
            0xDA => {
                let segment = segment(data, position)?;
                if segment.is_empty() {
                    return Err(LJpegError::Truncated("scan header"));
                }
                let count = usize::from(segment[0]);
                if count == 0 || count > MAX_COMPONENTS || segment.len() < 1 + count * 2 + 3 {
                    return Err(LJpegError::Malformed("scan header is inconsistent"));
                }
                let mut components = Vec::with_capacity(count);
                for index in 0..count {
                    let table = usize::from(segment[2 + index * 2] >> 4);
                    components.push(Component {
                        dc_table: table.min(tables.len() - 1),
                    });
                }
                let predictor = u32::from(segment[1 + count * 2]);
                let point_transform = u32::from(segment[1 + count * 2 + 2] & 0x0F);
                if frame_components.is_empty() {
                    return Err(LJpegError::Malformed("scan without a frame"));
                }
                if components.len() != frame_components.len() {
                    return Err(LJpegError::Malformed(
                        "scan and frame disagree on component count",
                    ));
                }
                if !(1..=7).contains(&predictor) {
                    return Err(LJpegError::Malformed("unsupported predictor"));
                }
                let entropy = &data[position + segment.len() + 2..];
                return scan(
                    entropy,
                    &tables,
                    &components,
                    width,
                    height,
                    precision,
                    predictor,
                    point_transform,
                    restart_interval,
                );
            }
            0xD9 => return Err(LJpegError::Truncated("stream ended before its scan")),
            // Standalone markers carry no segment.
            0x01 | 0xD0..=0xD7 => {}
            _ => {
                let segment = segment(data, position)?;
                position += segment.len() + 2;
            }
        }
    }
}

/// The payload of a marker segment, excluding its two-byte length.
fn segment(data: &[u8], position: usize) -> Result<&[u8], LJpegError> {
    let bytes = data
        .get(position..position + 2)
        .ok_or(LJpegError::Truncated("segment length"))?;
    let length = usize::from(u16::from_be_bytes([bytes[0], bytes[1]]));
    if length < 2 {
        return Err(LJpegError::Malformed("segment length below its own header"));
    }
    data.get(position + 2..position + length)
        .ok_or(LJpegError::Truncated("segment payload"))
}

#[allow(clippy::too_many_arguments)]
fn scan(
    entropy: &[u8],
    tables: &[HuffmanTable],
    components: &[Component],
    width: usize,
    height: usize,
    precision: u8,
    predictor: u32,
    point_transform: u32,
    restart_interval: usize,
) -> Result<Frame, LJpegError> {
    let count = components.len();
    let row_samples = width
        .checked_mul(count)
        .ok_or(LJpegError::LimitExceeded("row geometry overflows"))?;
    let total = row_samples
        .checked_mul(height)
        .ok_or(LJpegError::LimitExceeded("frame geometry overflows"))?;
    if total > MAX_SAMPLES {
        return Err(LJpegError::LimitExceeded("frame declares too many samples"));
    }
    let mut samples = vec![0u16; total];
    let mut reader = BitReader::new(entropy);

    // The first sample of the frame has no neighbour to predict from, so the
    // specification defines it as half of full scale.
    let seed = 1i32 << (i32::from(precision) - i32::from(point_transform as u8) - 1);
    let mut since_restart = 0usize;
    let mut restart_pending = false;

    for y in 0..height {
        for x in 0..width {
            if restart_interval > 0 && since_restart == restart_interval {
                reader.restart();
                since_restart = 0;
                restart_pending = true;
            }
            for (index, component) in components.iter().enumerate() {
                let position = y * row_samples + x * count + index;
                let table = tables
                    .get(component.dc_table)
                    .ok_or(LJpegError::Malformed("scan names a missing Huffman table"))?;
                let category = u32::from(reader.decode(table)?);
                if category > 16 {
                    return Err(LJpegError::Malformed("difference category out of range"));
                }
                // Category 16 is the escape meaning "difference of 32768".
                let difference = if category == 16 {
                    32768
                } else {
                    extend(reader.bits(category), category)
                };

                // Neighbours are per component, so each colour of the CFA
                // predicts from its own kind.
                let left = (x > 0).then(|| i32::from(samples[position - count]));
                let above = (y > 0).then(|| i32::from(samples[position - row_samples]));
                let above_left =
                    (x > 0 && y > 0).then(|| i32::from(samples[position - row_samples - count]));

                let base = if restart_pending {
                    // A restart resets the predictor to the frame seed.
                    seed
                } else {
                    match (left, above, above_left) {
                        // The first row predicts from the left only, the first
                        // column from above only, and the very first sample
                        // from the seed.
                        (None, None, _) => seed,
                        (Some(left), None, _) => left,
                        (None, Some(above), _) => above,
                        (Some(left), Some(above), above_left) => {
                            let above_left = above_left.unwrap_or(0);
                            match predictor {
                                1 => left,
                                2 => above,
                                3 => above_left,
                                4 => left + above - above_left,
                                5 => left + ((above - above_left) >> 1),
                                6 => above + ((left - above_left) >> 1),
                                _ => (left + above) >> 1,
                            }
                        }
                    }
                };
                restart_pending = false;

                // Lossless JPEG is defined modulo 2^16, so a difference that
                // runs past the end simply wraps.
                let value = (base.wrapping_add(difference)) as u16;
                samples[position] = value;
            }
            since_restart += 1;
        }
    }

    Ok(Frame {
        samples,
        width,
        height,
        components: count,
        precision,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a canonical Huffman table where category `n` has code length
    /// `n + 1`, which is simple to hand-encode in a test.
    struct Encoder {
        bits: Vec<u8>,
        current: u8,
        filled: u32,
    }

    impl Encoder {
        fn new() -> Self {
            Self {
                bits: Vec::new(),
                current: 0,
                filled: 0,
            }
        }

        fn push(&mut self, bit: u8) {
            self.current = (self.current << 1) | (bit & 1);
            self.filled += 1;
            if self.filled == 8 {
                self.bits.push(self.current);
                // Byte stuffing, so an entropy byte of FF is not a marker.
                if self.current == 0xFF {
                    self.bits.push(0x00);
                }
                self.current = 0;
                self.filled = 0;
            }
        }

        fn push_bits(&mut self, value: u32, length: u32) {
            for index in (0..length).rev() {
                self.push(((value >> index) & 1) as u8);
            }
        }

        /// Categories 0..=15 are `n` ones then a zero; category 16 is sixteen
        /// ones, which is the escape the specification reserves.
        fn category(&mut self, category: u32) {
            if category >= 16 {
                for _ in 0..16 {
                    self.push(1);
                }
                return;
            }
            for _ in 0..category {
                self.push(1);
            }
            self.push(0);
        }

        fn difference(&mut self, value: i32) {
            if value == 0 {
                self.category(0);
                return;
            }
            let magnitude = value.unsigned_abs();
            // A difference of exactly 32768 is category 16, which carries no
            // extra bits at all. Everything else spells out its magnitude.
            if magnitude == 32768 {
                self.category(16);
                return;
            }
            let category = 32 - magnitude.leading_zeros();
            self.category(category);
            let encoded = if value > 0 {
                value as u32
            } else {
                (value + (1i32 << category) - 1) as u32
            };
            self.push_bits(encoded, category);
        }

        fn finish(mut self) -> Vec<u8> {
            while self.filled != 0 {
                self.push(1);
            }
            self.bits
        }
    }

    /// Builds a lossless-JPEG stream from explicit sample values.
    fn build(
        width: usize,
        height: usize,
        components: usize,
        precision: u8,
        rows: &[Vec<i32>],
    ) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];

        // DHT: category n has code length n + 1 for n in 0..=16.
        let mut dht = vec![0x00u8];
        // Lengths 1..15 carry one code each and length 16 carries two, which is
        // exactly seventeen codes for categories 0 through 16 and a Kraft sum
        // of one.
        let mut counts = [1u8; 16];
        counts[15] = 2;
        dht.extend(counts);
        // The value list is the category for each code, in length order.
        dht.extend((0u8..=16).collect::<Vec<u8>>());
        out.extend([0xFF, 0xC4]);
        out.extend(((dht.len() + 2) as u16).to_be_bytes());
        out.extend(dht);

        // SOF3.
        let mut sof = vec![precision];
        sof.extend((height as u16).to_be_bytes());
        sof.extend((width as u16).to_be_bytes());
        sof.push(components as u8);
        for index in 0..components {
            sof.extend([index as u8 + 1, 0x11, 0x00]);
        }
        out.extend([0xFF, 0xC3]);
        out.extend(((sof.len() + 2) as u16).to_be_bytes());
        out.extend(sof);

        // SOS with predictor 1 (predict from the left).
        let mut sos = vec![components as u8];
        for index in 0..components {
            sos.extend([index as u8 + 1, 0x00]);
        }
        sos.extend([0x01, 0x00, 0x00]);
        out.extend([0xFF, 0xDA]);
        out.extend(((sos.len() + 2) as u16).to_be_bytes());
        out.extend(sos);

        let mut encoder = Encoder::new();
        let seed = 1i32 << (i32::from(precision) - 1);
        for (y, row) in rows.iter().enumerate().take(height) {
            for x in 0..width {
                for component in 0..components {
                    let index = x * components + component;
                    let value = row[index];
                    let base = if x > 0 {
                        row[index - components]
                    } else if y > 0 {
                        rows[y - 1][index]
                    } else {
                        seed
                    };
                    // Lossless JPEG differences are modulo 2^16, so a nominal
                    // difference of 65535 is encoded as -1. Without this the
                    // fixture would ask for categories the format cannot spell.
                    let delta = i32::from((((value - base) & 0xFFFF) as u16) as i16);
                    encoder.difference(delta);
                }
            }
        }
        out.extend(encoder.finish());
        out.extend([0xFF, 0xD9]);
        out
    }

    #[test]
    fn decodes_a_single_component_frame_exactly() {
        let rows = vec![vec![1000, 1010, 990, 1200], vec![1005, 1015, 985, 1190]];
        let data = build(4, 2, 1, 12, &rows);
        let frame = decode(&data).unwrap();
        assert_eq!(frame.width, 4);
        assert_eq!(frame.height, 2);
        assert_eq!(frame.components, 1);
        assert_eq!(frame.precision, 12);
        let expected: Vec<u16> = rows.concat().iter().map(|value| *value as u16).collect();
        assert_eq!(frame.samples, expected);
    }

    /// The layout DNG actually uses: two sensor columns per JPEG column.
    #[test]
    fn decodes_a_two_component_frame_interleaved_as_dng_writes_it() {
        let rows = vec![
            vec![100, 200, 110, 210, 120, 220],
            vec![105, 205, 115, 215, 125, 225],
        ];
        let data = build(3, 2, 2, 14, &rows);
        let frame = decode(&data).unwrap();
        assert_eq!(frame.components, 2);
        assert_eq!(frame.width, 3);
        // Six samples per row: component 0 and 1 alternating.
        assert_eq!(frame.samples.len(), 12);
        assert_eq!(&frame.samples[0..6], &[100, 200, 110, 210, 120, 220]);
        assert_eq!(&frame.samples[6..12], &[105, 205, 115, 215, 125, 225]);
    }

    #[test]
    fn decoding_is_exact_across_the_full_sixteen_bit_range() {
        let rows = vec![vec![0, 65535, 32768, 1], vec![65535, 0, 12345, 54321]];
        let data = build(4, 2, 1, 16, &rows);
        let frame = decode(&data).unwrap();
        let expected: Vec<u16> = rows.concat().iter().map(|value| *value as u16).collect();
        assert_eq!(frame.samples, expected, "lossless decoding lost a value");
    }

    #[test]
    fn rejects_a_stream_that_is_not_jpeg() {
        assert_eq!(decode(b"nope").unwrap_err(), LJpegError::NotLosslessJpeg);
        assert_eq!(decode(&[]).unwrap_err(), LJpegError::NotLosslessJpeg);
    }

    #[test]
    fn rejects_a_truncated_stream_without_panicking() {
        let data = build(4, 2, 1, 12, &[vec![1, 2, 3, 4], vec![5, 6, 7, 8]]);
        for length in 2..data.len() {
            // Every prefix must produce an error or a frame, never a panic.
            let _ = decode(&data[..length]);
        }
    }

    #[test]
    fn refuses_an_absurd_declared_frame_before_allocating() {
        let mut data = build(4, 2, 1, 12, &[vec![1, 2, 3, 4], vec![5, 6, 7, 8]]);
        // Find the SOF3 marker and inflate its declared dimensions.
        let position = data
            .windows(2)
            .position(|pair| pair == [0xFF, 0xC3])
            .expect("SOF3 present");
        let payload = position + 4;
        data[payload + 1..payload + 3].copy_from_slice(&65535u16.to_be_bytes());
        data[payload + 3..payload + 5].copy_from_slice(&65535u16.to_be_bytes());
        assert_eq!(
            decode(&data).unwrap_err(),
            LJpegError::LimitExceeded("frame declares too many samples")
        );
    }

    #[test]
    fn refuses_a_frame_with_no_components_or_too_many() {
        let mut data = build(2, 2, 1, 12, &[vec![1, 2], vec![3, 4]]);
        let position = data
            .windows(2)
            .position(|pair| pair == [0xFF, 0xC3])
            .unwrap();
        data[position + 4 + 5] = 0;
        assert_eq!(
            decode(&data).unwrap_err(),
            LJpegError::Malformed("unsupported component count")
        );
    }

    #[test]
    fn a_dct_jpeg_frame_type_is_not_accepted_as_lossless() {
        let mut data = build(2, 2, 1, 12, &[vec![1, 2], vec![3, 4]]);
        let position = data
            .windows(2)
            .position(|pair| pair == [0xFF, 0xC3])
            .unwrap();
        // SOF0 is baseline DCT, which this decoder must not pretend to read.
        data[position + 1] = 0xC0;
        assert!(matches!(
            decode(&data),
            Err(LJpegError::Malformed(_)) | Err(LJpegError::Truncated(_))
        ));
    }

    #[test]
    fn random_bytes_never_panic() {
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..200 {
            let mut data = vec![0xFF, 0xD8];
            for _ in 0..256 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                data.push((state >> 24) as u8);
            }
            let _ = decode(&data);
        }
    }
}
