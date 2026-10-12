//! Decoding one channel's image data: raw, PackBits (RLE), and zlib with or without prediction.
//!
//! The output is the channel's samples, row after row, big-endian, exactly
//! `rows * ceil(width * depth / 8)` bytes. The size is computed from the *declared*
//! dimensions with checked arithmetic and compared with the caller's ceiling before
//! anything is allocated; the stream is then required to fill it exactly and no more.
use super::document::Version;
use super::error::PsdError;
use super::reader::Reader;
use std::io::{Read, Seek};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Raw,
    Rle,
    Zip,
    ZipPrediction,
}

impl Compression {
    pub fn from_u16(value: u16) -> Option<Self> {
        Some(match value {
            0 => Self::Raw,
            1 => Self::Rle,
            2 => Self::Zip,
            3 => Self::ZipPrediction,
            _ => return None,
        })
    }
}

/// Bytes in one row of one channel at `depth` bits per sample.
pub fn row_bytes(width: u64, depth: u16) -> Option<u64> {
    width
        .checked_mul(u64::from(depth))
        .map(|bits| bits.div_ceil(8))
}

/// Where a channel's data is and how big its samples are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelSpec {
    /// Absolute offset of the compression field.
    pub offset: u64,
    /// Bytes of data, the compression field included.
    pub length: u64,
    pub width: u64,
    pub height: u64,
    pub depth: u16,
}

/// Decodes the channel `spec` describes.
///
/// `max_output` bounds the decoded size; a channel whose declared dimensions would exceed
/// it is refused without reading its data.
pub fn decode_channel<R: Read + Seek>(
    reader: &mut Reader<R>,
    version: Version,
    spec: &ChannelSpec,
    max_output: u64,
) -> Result<Vec<u8>, PsdError> {
    let ChannelSpec {
        offset,
        length,
        width,
        height,
        depth,
    } = *spec;
    if length < 2 {
        return Err(PsdError::Malformed {
            what: "a channel has no compression field",
        });
    }
    let end = offset
        .checked_add(length)
        .filter(|end| *end <= reader.len())
        .ok_or(PsdError::Truncated {
            what: "a channel's image data",
        })?;
    let row = row_bytes(width, depth).ok_or(PsdError::Malformed {
        what: "a channel's row size overflows",
    })?;
    let total = row.checked_mul(height).ok_or(PsdError::Malformed {
        what: "a channel's size overflows",
    })?;
    if total > max_output {
        return Err(PsdError::Limit {
            what: "a decoded channel in bytes",
            value: total,
            limit: max_output,
        });
    }
    reader.seek_to(offset, "a channel's image data")?;
    let compression_value = reader.u16(end, "a channel's compression")?;
    let compression =
        Compression::from_u16(compression_value).ok_or_else(|| PsdError::Unsupported {
            what: format!("channel compression {compression_value}"),
        })?;
    let payload = length - 2;
    let rows = usize::try_from(height).map_err(|_| PsdError::Malformed {
        what: "a channel's height",
    })?;

    match compression {
        Compression::Raw => {
            if payload < total {
                return Err(PsdError::Truncated {
                    what: "raw channel data",
                });
            }
            reader.bytes(total, max_output, end, "raw channel data")
        }
        Compression::Rle => decode_rle(reader, version, end, payload, row, rows, total),
        Compression::Zip | Compression::ZipPrediction => {
            let packed = reader.bytes(payload, payload, end, "compressed channel data")?;
            let mut out =
                miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(&packed, total as usize)
                    .map_err(|_| PsdError::Malformed {
                        what: "channel data that does not inflate to its declared size",
                    })?;
            if out.len() as u64 != total {
                return Err(PsdError::Malformed {
                    what: "channel data that inflates to the wrong size",
                });
            }
            if compression == Compression::ZipPrediction {
                undo_prediction(&mut out, row as usize, depth)?;
            }
            Ok(out)
        }
    }
}

fn decode_rle<R: Read + Seek>(
    reader: &mut Reader<R>,
    version: Version,
    end: u64,
    payload: u64,
    row: u64,
    rows: usize,
    total: u64,
) -> Result<Vec<u8>, PsdError> {
    let count_width: u64 = if version.is_big() { 4 } else { 2 };
    let table = count_width
        .checked_mul(rows as u64)
        .filter(|table| *table <= payload)
        .ok_or(PsdError::Truncated {
            what: "the row length table of an RLE channel",
        })?;
    let mut counts = Vec::with_capacity(rows);
    let mut declared: u64 = 0;
    for _ in 0..rows {
        let count = if version.is_big() {
            u64::from(reader.u32(end, "an RLE row length")?)
        } else {
            u64::from(reader.u16(end, "an RLE row length")?)
        };
        declared = declared.saturating_add(count);
        counts.push(count);
    }
    if declared > payload - table {
        return Err(PsdError::Malformed {
            what: "RLE rows that are longer than the channel",
        });
    }
    let mut out = Vec::with_capacity(total as usize);
    for count in counts {
        let packed = reader.bytes(count, count, end, "an RLE row")?;
        unpack_bits_row(&packed, row as usize, &mut out)?;
    }
    Ok(out)
}

/// One PackBits row, which must unpack to exactly `row` bytes.
fn unpack_bits_row(packed: &[u8], row: usize, out: &mut Vec<u8>) -> Result<(), PsdError> {
    let start = out.len();
    let mut at = 0;
    while at < packed.len() && out.len() - start < row {
        let control = packed[at] as i8;
        at += 1;
        match control {
            0..=127 => {
                let run = control as usize + 1;
                let end = at
                    .checked_add(run)
                    .filter(|end| *end <= packed.len())
                    .ok_or(PsdError::Malformed {
                        what: "a PackBits literal run that runs off its row",
                    })?;
                if out.len() - start + run > row {
                    return Err(PsdError::Malformed {
                        what: "a PackBits row that unpacks beyond its width",
                    });
                }
                out.extend_from_slice(&packed[at..end]);
                at = end;
            }
            -127..=-1 => {
                let run = 1 - isize::from(control);
                let byte = *packed.get(at).ok_or(PsdError::Malformed {
                    what: "a PackBits repeat with no byte",
                })?;
                at += 1;
                let run = run as usize;
                if out.len() - start + run > row {
                    return Err(PsdError::Malformed {
                        what: "a PackBits row that unpacks beyond its width",
                    });
                }
                out.resize(out.len() + run, byte);
            }
            -128 => {}
        }
    }
    if out.len() - start != row {
        return Err(PsdError::Malformed {
            what: "a PackBits row that unpacks to the wrong width",
        });
    }
    Ok(())
}

/// Undoes the horizontal prediction Photoshop applies before zlib: each sample is stored as
/// the difference from the one to its left. At 32 bits the four bytes of every sample of a
/// row are first gathered into four planes (all the most significant bytes, then the next,
/// and so on) and the byte-wise differences run across that whole row, so the row is
/// summed and then interleaved back into big-endian samples.
fn undo_prediction(data: &mut [u8], row: usize, depth: u16) -> Result<(), PsdError> {
    match depth {
        8 => {
            for line in data.chunks_mut(row) {
                for index in 1..line.len() {
                    line[index] = line[index].wrapping_add(line[index - 1]);
                }
            }
            Ok(())
        }
        16 => {
            for line in data.chunks_mut(row) {
                let mut previous: u16 = 0;
                for pair in line.chunks_exact_mut(2) {
                    let value = u16::from_be_bytes([pair[0], pair[1]]).wrapping_add(previous);
                    pair.copy_from_slice(&value.to_be_bytes());
                    previous = value;
                }
            }
            Ok(())
        }
        32 => {
            let width = row / 4;
            let mut samples = vec![0_u8; row];
            for line in data.chunks_mut(row) {
                if line.len() != row {
                    return Err(PsdError::Malformed {
                        what: "a 32-bit prediction row that is not whole",
                    });
                }
                for index in 1..row {
                    line[index] = line[index].wrapping_add(line[index - 1]);
                }
                for x in 0..width {
                    for plane in 0..4 {
                        samples[x * 4 + plane] = line[plane * width + x];
                    }
                }
                line.copy_from_slice(&samples);
            }
            Ok(())
        }
        _ => Err(PsdError::Unsupported {
            what: format!("ZIP prediction at {depth} bits per sample"),
        }),
    }
}

/// Where the merged image is and what shape it has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergedSpec {
    /// Absolute offset of the compression field.
    pub offset: u64,
    /// Every channel in the file: the row table covers them all.
    pub channels: u16,
    pub width: u64,
    pub height: u64,
    pub depth: u16,
}

/// The merged ("flattened") image: every channel for the whole canvas, channel after
/// channel. Raw and PackBits are read; zlib is refused (Photoshop does not write it here,
/// and a stream per channel with no lengths cannot be told apart safely).
///
/// Returns one buffer per channel, each `width * height` samples at `depth` bits.
pub fn decode_merged_image<R: Read + Seek>(
    reader: &mut Reader<R>,
    version: Version,
    spec: &MergedSpec,
    max_output: u64,
) -> Result<Vec<Vec<u8>>, PsdError> {
    let MergedSpec {
        offset,
        channels,
        width,
        height,
        depth,
    } = *spec;
    let file_end = reader.len();
    let row = row_bytes(width, depth).ok_or(PsdError::Malformed {
        what: "the merged image's row size overflows",
    })?;
    let plane = row.checked_mul(height).ok_or(PsdError::Malformed {
        what: "the merged image's size overflows",
    })?;
    let total = plane
        .checked_mul(u64::from(channels))
        .ok_or(PsdError::Malformed {
            what: "the merged image's size overflows",
        })?;
    if total > max_output {
        return Err(PsdError::Limit {
            what: "the merged image in bytes",
            value: total,
            limit: max_output,
        });
    }
    reader.seek_to(offset, "the merged image")?;
    let compression_value = reader.u16(file_end, "the merged image compression")?;
    match Compression::from_u16(compression_value) {
        Some(Compression::Raw) => {
            let mut out = Vec::with_capacity(channels as usize);
            for _ in 0..channels {
                out.push(reader.bytes(plane, max_output, file_end, "the merged image")?);
            }
            Ok(out)
        }
        Some(Compression::Rle) => {
            let rows = usize::try_from(height).map_err(|_| PsdError::Malformed {
                what: "the merged image's height",
            })?;
            let counts_total = u64::from(channels) * height;
            let count_width: u64 = if version.is_big() { 4 } else { 2 };
            let table = counts_total
                .checked_mul(count_width)
                .filter(|table| offset + 2 + *table <= file_end)
                .ok_or(PsdError::Truncated {
                    what: "the merged image row table",
                })?;
            let _ = table;
            let mut counts = Vec::with_capacity(counts_total as usize);
            for _ in 0..counts_total {
                counts.push(if version.is_big() {
                    u64::from(reader.u32(file_end, "a merged image row length")?)
                } else {
                    u64::from(reader.u16(file_end, "a merged image row length")?)
                });
            }
            let mut out = Vec::with_capacity(channels as usize);
            let mut index = 0;
            for _ in 0..channels {
                let mut data = Vec::with_capacity(plane as usize);
                for _ in 0..rows {
                    let packed = reader.bytes(
                        counts[index],
                        counts[index],
                        file_end,
                        "a merged image row",
                    )?;
                    index += 1;
                    unpack_bits_row(&packed, row as usize, &mut data)?;
                }
                out.push(data);
            }
            Ok(out)
        }
        Some(_) => Err(PsdError::Unsupported {
            what: "a merged image compressed with zlib".into(),
        }),
        None => Err(PsdError::Unsupported {
            what: format!("merged image compression {compression_value}"),
        }),
    }
}
