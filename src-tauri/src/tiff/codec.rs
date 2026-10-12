//! The compression schemes a TIFF strip or tile may use, and the predictors that go with them.
//!
//! Each decoder is told how many bytes the segment must expand to (its rows, from the picture's
//! own width and depth) and never produces more than that, whatever the stream says: a strip that
//! claims to be a kilobyte and expands to a gigabyte is a bomb, and the bound is the defence.
//! A stream that ends short is an error and not a segment of zeros.
use super::structure::Order;

/// Compression codes this reader decodes.
pub const NONE: u16 = 1;
pub const LZW: u16 = 5;
pub const DEFLATE: u16 = 8;
pub const DEFLATE_OLD: u16 = 32946;
pub const PACKBITS: u16 = 32773;

/// A name for a compression code, for a refusal.
pub fn compression_name(code: u16) -> String {
    match code {
        2 => "CCITT modified Huffman RLE".to_string(),
        3 => "CCITT Group 3 fax".to_string(),
        4 => "CCITT Group 4 fax".to_string(),
        6 => "old-style JPEG".to_string(),
        7 => "JPEG".to_string(),
        34712 => "JPEG 2000".to_string(),
        34925 => "LZMA".to_string(),
        34926 => "Zstandard".to_string(),
        50001 => "WebP".to_string(),
        34661 => "JBIG".to_string(),
        other => format!("compression {other}"),
    }
}

pub fn is_supported(code: u16) -> bool {
    matches!(code, NONE | LZW | DEFLATE | DEFLATE_OLD | PACKBITS)
}

/// Expands one segment to exactly `expected` bytes.
pub fn decompress(code: u16, input: &[u8], expected: usize) -> Result<Vec<u8>, &'static str> {
    match code {
        NONE => {
            if input.len() < expected {
                return Err("a strip is shorter than the rows it holds");
            }
            Ok(input[..expected].to_vec())
        }
        PACKBITS => packbits(input, expected),
        LZW => lzw(input, expected),
        DEFLATE | DEFLATE_OLD => deflate(input, expected),
        _ => Err("a compression this reader does not decode"),
    }
}

fn packbits(input: &[u8], expected: usize) -> Result<Vec<u8>, &'static str> {
    let mut out = Vec::new();
    out.try_reserve_exact(expected)
        .map_err(|_| "the picture is too large to decode")?;
    let mut at = 0;
    while out.len() < expected {
        let Some(&control) = input.get(at) else {
            return Err("a PackBits strip ends before its rows are complete");
        };
        at += 1;
        let control = control as i8;
        if control >= 0 {
            let count = control as usize + 1;
            let Some(run) = input.get(at..at + count) else {
                return Err("a PackBits strip ends inside a literal run");
            };
            out.extend_from_slice(run);
            at += count;
        } else if control != -128 {
            let count = 1 - control as isize;
            let Some(&byte) = input.get(at) else {
                return Err("a PackBits strip ends inside a repeat");
            };
            at += 1;
            out.resize(out.len() + count as usize, byte);
        }
    }
    out.truncate(expected);
    Ok(out)
}

fn deflate(input: &[u8], expected: usize) -> Result<Vec<u8>, &'static str> {
    use miniz_oxide::inflate::{decompress_to_vec_zlib_with_limit, TINFLStatus};
    match decompress_to_vec_zlib_with_limit(input, expected) {
        Ok(mut bytes) => {
            if bytes.len() < expected {
                return Err("a Deflate strip expands to fewer bytes than its rows hold");
            }
            bytes.truncate(expected);
            Ok(bytes)
        }
        // The limit is met and more follows: the rows are complete, and the rest is padding.
        Err(error)
            if error.status == TINFLStatus::HasMoreOutput && error.output.len() >= expected =>
        {
            let mut bytes = error.output;
            bytes.truncate(expected);
            Ok(bytes)
        }
        Err(_) => Err("a Deflate strip is damaged"),
    }
}

/// TIFF LZW: most significant bit first, codes of 9 to 12 bits, clear code 256, end code 257,
/// and the code width grows one code before the table is full ("early change").
fn lzw(input: &[u8], expected: usize) -> Result<Vec<u8>, &'static str> {
    const CLEAR: u16 = 256;
    const END: u16 = 257;
    const TABLE: usize = 4096;
    // Files written before TIFF 5.0 packed the bits the other way round; they begin 00 01.
    if input.len() >= 2 && input[0] == 0 && input[1] & 1 == 1 {
        return Err("old-style LZW (from before TIFF 5.0) is not read");
    }
    let mut out = Vec::new();
    out.try_reserve_exact(expected)
        .map_err(|_| "the picture is too large to decode")?;
    let mut prefix = vec![0_u16; TABLE];
    let mut suffix = vec![0_u8; TABLE];
    let mut first = vec![0_u8; TABLE];
    let mut length = vec![0_u16; TABLE];
    for literal in 0..256 {
        suffix[literal] = literal as u8;
        first[literal] = literal as u8;
        length[literal] = 1;
    }
    let mut next = 258_usize;
    let mut width = 9_u32;
    let mut buffer = 0_u32;
    let mut held = 0_u32;
    let mut at = 0_usize;
    let mut previous: Option<u16> = None;

    'codes: while out.len() < expected {
        while held < width {
            let Some(&byte) = input.get(at) else {
                break 'codes;
            };
            buffer = (buffer << 8) | u32::from(byte);
            at += 1;
            held += 8;
        }
        let code = ((buffer >> (held - width)) & ((1 << width) - 1)) as u16;
        held -= width;
        buffer &= (1 << held) - 1;
        if code == END {
            break;
        }
        if code == CLEAR {
            next = 258;
            width = 9;
            previous = None;
            continue;
        }
        // The string this code stands for, written at the end of the output.
        let emit =
            |out: &mut Vec<u8>, code: usize, length: &[u16], prefix: &[u16], suffix: &[u8]| {
                let size = usize::from(length[code]);
                let start = out.len();
                out.resize(start + size, 0);
                let mut index = start + size;
                let mut current = code;
                loop {
                    index -= 1;
                    out[index] = suffix[current];
                    if length[current] == 1 {
                        break;
                    }
                    current = usize::from(prefix[current]);
                }
            };
        match previous {
            None => {
                if code >= 256 {
                    return Err("an LZW strip does not begin with a literal");
                }
                out.push(code as u8);
            }
            Some(before) => {
                let before = usize::from(before);
                let code = usize::from(code);
                if code < next {
                    emit(&mut out, code, &length, &prefix, &suffix);
                    if next < TABLE {
                        prefix[next] = before as u16;
                        suffix[next] = first[code];
                        first[next] = first[before];
                        length[next] = length[before] + 1;
                        next += 1;
                    }
                } else if code == next && next < TABLE {
                    // The string just defined is used before it is finished: it is the
                    // previous one followed by its own first byte.
                    prefix[next] = before as u16;
                    suffix[next] = first[before];
                    first[next] = first[before];
                    length[next] = length[before] + 1;
                    next += 1;
                    emit(&mut out, code, &length, &prefix, &suffix);
                } else {
                    return Err("an LZW strip names a code that is not in its table");
                }
            }
        }
        previous = Some(code);
        if next + 1 >= (1 << width) && width < 12 {
            width += 1;
        }
    }
    if out.len() < expected {
        return Err("an LZW strip ends before its rows are complete");
    }
    out.truncate(expected);
    Ok(out)
}

/// Undoes horizontal differencing (predictor 2) on one row of `samples` interleaved channels.
pub fn undo_horizontal(row: &mut [u8], samples: usize, bits: u16, order: Order) {
    match bits {
        8 => {
            for index in samples..row.len() {
                row[index] = row[index].wrapping_add(row[index - samples]);
            }
        }
        16 => {
            let words = row.len() / 2;
            for index in samples..words {
                let at = index * 2;
                let before = index - samples;
                let current = order.u16([row[at], row[at + 1]]);
                let earlier = order.u16([row[before * 2], row[before * 2 + 1]]);
                let sum = current.wrapping_add(earlier);
                let bytes = match order {
                    Order::Little => sum.to_le_bytes(),
                    Order::Big => sum.to_be_bytes(),
                };
                row[at] = bytes[0];
                row[at + 1] = bytes[1];
            }
        }
        32 => {
            let words = row.len() / 4;
            for index in samples..words {
                let at = index * 4;
                let before = (index - samples) * 4;
                let current = order.u32([row[at], row[at + 1], row[at + 2], row[at + 3]]);
                let earlier = order.u32([
                    row[before],
                    row[before + 1],
                    row[before + 2],
                    row[before + 3],
                ]);
                let sum = current.wrapping_add(earlier);
                let bytes = match order {
                    Order::Little => sum.to_le_bytes(),
                    Order::Big => sum.to_be_bytes(),
                };
                row[at..at + 4].copy_from_slice(&bytes);
            }
        }
        _ => {}
    }
}

/// Undoes the floating-point predictor (predictor 3) on one row: the bytes of each value are
/// stored most significant first, plane by plane, and differenced. The result holds each value
/// big-endian, whatever the file's byte order, which is what Adobe's note specifies.
pub fn undo_floating_point(row: &mut [u8], samples: usize, bytes_per_sample: usize) {
    let stride = samples;
    if stride == 0 || bytes_per_sample == 0 {
        return;
    }
    for index in stride..row.len() {
        row[index] = row[index].wrapping_add(row[index - stride]);
    }
    let values = row.len() / bytes_per_sample;
    let shuffled = row.to_vec();
    for value in 0..values {
        for byte in 0..bytes_per_sample {
            row[value * bytes_per_sample + byte] = shuffled[byte * values + value];
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    /// A TIFF LZW encoder (MSB first, early change), the other half of the decoder under test.
    pub fn lzw_encode(input: &[u8]) -> Vec<u8> {
        use std::collections::HashMap;
        struct Bits {
            out: Vec<u8>,
            buffer: u32,
            held: u32,
        }
        impl Bits {
            fn put(&mut self, code: u32, width: u32) {
                self.buffer = (self.buffer << width) | code;
                self.held += width;
                while self.held >= 8 {
                    self.out.push((self.buffer >> (self.held - 8)) as u8);
                    self.held -= 8;
                    self.buffer &= (1 << self.held) - 1;
                }
            }
        }
        let mut bits = Bits {
            out: Vec::new(),
            buffer: 0,
            held: 0,
        };
        let mut table: HashMap<(u32, u8), u32> = HashMap::new();
        let mut next = 258_u32;
        let mut width = 9_u32;
        bits.put(256, width);
        let mut current: Option<u32> = None;
        for &byte in input {
            match current {
                None => current = Some(u32::from(byte)),
                Some(prefix) => {
                    if let Some(&code) = table.get(&(prefix, byte)) {
                        current = Some(code);
                    } else {
                        bits.put(prefix, width);
                        table.insert((prefix, byte), next);
                        next += 1;
                        // Grow one code early, as the decoder does.
                        if next + 1 > (1 << width) && width < 12 {
                            width += 1;
                        }
                        if next >= 4094 {
                            bits.put(256, width);
                            table.clear();
                            next = 258;
                            width = 9;
                        }
                        current = Some(u32::from(byte));
                    }
                }
            }
        }
        if let Some(code) = current {
            bits.put(code, width);
            next += 1;
            if next + 1 > (1 << width) && width < 12 {
                width += 1;
            }
        }
        bits.put(257, width);
        if bits.held > 0 {
            bits.put(0, 8 - bits.held);
        }
        bits.out
    }

    pub fn packbits_encode(input: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < input.len() {
            let mut run = 1;
            while at + run < input.len() && input[at + run] == input[at] && run < 128 {
                run += 1;
            }
            if run >= 2 {
                out.push((1 - run as isize) as i8 as u8);
                out.push(input[at]);
                at += run;
            } else {
                let start = at;
                while at < input.len()
                    && at - start < 128
                    && !(at + 1 < input.len() && input[at] == input[at + 1])
                {
                    at += 1;
                }
                if at == start {
                    at += 1;
                }
                out.push((at - start - 1) as u8);
                out.extend_from_slice(&input[start..at]);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{lzw_encode, packbits_encode};
    use super::*;

    fn sample(len: usize) -> Vec<u8> {
        // Runs, a ramp and noise, so the table fills and resets.
        let mut state = 0x1234_5678_u32;
        (0..len)
            .map(|index| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                match (index / 300) % 3 {
                    0 => 7,
                    1 => (index % 251) as u8,
                    _ => (state >> 24) as u8,
                }
            })
            .collect()
    }

    #[test]
    fn lzw_round_trips_data_that_fills_and_resets_the_table() {
        for len in [1, 2, 10, 500, 5_000, 70_000] {
            let data = sample(len);
            let encoded = lzw_encode(&data);
            assert_eq!(lzw(&encoded, len).unwrap(), data, "{len} bytes");
        }
    }

    #[test]
    fn lzw_reads_the_string_of_the_tiff_specification() {
        // The spec's own example, "-----A---B": 256 45 258 258 65 259 66 257 at 9 bits.
        let codes = [256_u32, 45, 258, 258, 65, 259, 66, 257];
        let mut bits = 0_u64;
        let mut held = 0;
        let mut out = Vec::new();
        for code in codes {
            bits = (bits << 9) | u64::from(code);
            held += 9;
            while held >= 8 {
                out.push((bits >> (held - 8)) as u8);
                held -= 8;
                bits &= (1 << held) - 1;
            }
        }
        if held > 0 {
            out.push((bits << (8 - held)) as u8);
        }
        assert_eq!(lzw(&out, 10).unwrap(), b"-----A---B");
    }

    #[test]
    fn lzw_is_bounded_and_refuses_what_is_wrong() {
        let data = vec![9_u8; 100_000];
        let encoded = lzw_encode(&data);
        // Asked for fewer bytes than it holds, it stops there.
        assert_eq!(lzw(&encoded, 1_000).unwrap(), vec![9_u8; 1_000]);
        // Asked for more than it holds, it is an error and not zeros.
        assert!(lzw(&encoded, 200_000).is_err());
        assert!(lzw(&[0x00, 0x01, 0x02], 4)
            .unwrap_err()
            .contains("old-style"));
        // A code past the table.
        assert!(lzw(&[0x80, 0x00, 0x00, 0x00], 4).is_err());
        assert!(lzw(&[], 1).is_err());
    }

    #[test]
    fn packbits_round_trips_and_is_bounded() {
        for len in [1, 2, 127, 128, 129, 1_000, 9_999] {
            let data = sample(len);
            assert_eq!(packbits(&packbits_encode(&data), len).unwrap(), data);
        }
        // The no-op byte is skipped.
        assert_eq!(packbits(&[0x80, 0x00, 0x41], 1).unwrap(), b"A");
        // A repeat of 128 asked for 3 bytes gives 3.
        assert_eq!(packbits(&[0x81, 0x41], 3).unwrap(), b"AAA");
        assert!(packbits(&[0x05, 1, 2], 6).is_err());
        assert!(packbits(&[], 1).is_err());
    }

    #[test]
    fn deflate_round_trips_and_a_bomb_costs_only_the_rows() {
        let data = sample(20_000);
        let packed = miniz_oxide::deflate::compress_to_vec_zlib(&data, 6);
        assert_eq!(deflate(&packed, data.len()).unwrap(), data);
        let zeros = vec![0_u8; 50_000_000];
        let bomb = miniz_oxide::deflate::compress_to_vec_zlib(&zeros, 9);
        assert!(bomb.len() < 100_000);
        assert_eq!(deflate(&bomb, 1_000).unwrap(), vec![0_u8; 1_000]);
        assert!(deflate(&packed, data.len() + 1).is_err());
        assert!(deflate(b"not a zlib stream", 10).is_err());
    }

    #[test]
    fn none_requires_the_rows_to_be_there() {
        assert_eq!(decompress(NONE, &[1, 2, 3, 4], 3).unwrap(), vec![1, 2, 3]);
        assert!(decompress(NONE, &[1, 2], 3).is_err());
        assert!(decompress(7, &[1], 1).is_err());
    }

    #[test]
    fn horizontal_differencing_is_undone_for_each_depth_and_order() {
        // 3 channels, 4 pixels: differences of a ramp.
        let mut eight = vec![10, 20, 30, 1, 1, 1, 1, 1, 1, 255, 255, 255];
        undo_horizontal(&mut eight, 3, 8, Order::Little);
        assert_eq!(eight, vec![10, 20, 30, 11, 21, 31, 12, 22, 32, 11, 21, 31]);

        let mut sixteen_le = vec![0x00, 0x01, 0xFF, 0x00, 0x01, 0x00, 0xFF, 0xFF];
        undo_horizontal(&mut sixteen_le, 1, 16, Order::Little);
        // 0x0100, 0x0100 + 0x00FF, + 0x0001, + 0xFFFF
        assert_eq!(
            sixteen_le,
            vec![0x00, 0x01, 0xFF, 0x01, 0x00, 0x02, 0xFF, 0x01]
        );

        let mut sixteen_be = vec![0x01, 0x00, 0x00, 0xFF];
        undo_horizontal(&mut sixteen_be, 1, 16, Order::Big);
        assert_eq!(sixteen_be, vec![0x01, 0x00, 0x01, 0xFF]);

        let mut thirty_two = vec![1, 0, 0, 0, 2, 0, 0, 0];
        undo_horizontal(&mut thirty_two, 1, 32, Order::Little);
        assert_eq!(thirty_two, vec![1, 0, 0, 0, 3, 0, 0, 0]);
    }

    #[test]
    fn the_floating_point_predictor_is_undone() {
        // Three values, one channel: encode by hand as the writer does, then decode.
        let values = [1.0_f32, 2.5, -0.75];
        let mut planes = Vec::new();
        for byte in 0..4 {
            for value in values {
                planes.push(value.to_be_bytes()[byte]);
            }
        }
        let mut differenced = planes.clone();
        for index in (1..differenced.len()).rev() {
            differenced[index] = differenced[index].wrapping_sub(differenced[index - 1]);
        }
        undo_floating_point(&mut differenced, 1, 4);
        let read: Vec<f32> = differenced
            .chunks_exact(4)
            .map(|chunk| f32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect();
        assert_eq!(read, values);
    }
}
