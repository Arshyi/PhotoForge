//! IPTC-IIM: the older newsroom metadata (captions, keywords, credit, place) that photo software
//! still writes next to XMP.
//!
//! A stream of datasets, each `0x1C`, a record number, a dataset number, a length and the data.
//! Reading is bounded by the stream; removing rebuilds the stream without the datasets named, so
//! nothing of them is left.

/// A stream larger than this is not carried.
pub const MAX_IPTC_BYTES: usize = 256 * 1024;

/// Record 2 datasets that name a place: content location code and name, city, sub-location,
/// state or province, country code and country name.
const LOCATION_DATASETS: [u8; 7] = [26, 27, 90, 92, 95, 100, 101];

struct Dataset<'a> {
    record: u8,
    number: u8,
    whole: &'a [u8],
}

/// Walks the stream. `None` when it is not a well-formed run of datasets.
fn datasets(stream: &[u8]) -> Option<Vec<Dataset<'_>>> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < stream.len() {
        if *stream.get(at)? != 0x1C {
            return None;
        }
        let record = *stream.get(at + 1)?;
        let number = *stream.get(at + 2)?;
        let first = u16::from_be_bytes([*stream.get(at + 3)?, *stream.get(at + 4)?]);
        let (header, length) = if first & 0x8000 == 0 {
            (5_usize, usize::from(first))
        } else {
            // Extended: the low bits say how many bytes of length follow.
            let width = usize::from(first & 0x7FFF);
            if width == 0 || width > 4 {
                return None;
            }
            let mut length = 0_usize;
            for index in 0..width {
                length = length
                    .checked_mul(256)?
                    .checked_add(usize::from(*stream.get(at + 5 + index)?))?;
            }
            (5 + width, length)
        };
        let end = at.checked_add(header)?.checked_add(length)?;
        let whole = stream.get(at..end)?;
        found.push(Dataset {
            record,
            number,
            whole,
        });
        at = end;
        if found.len() > 10_000 {
            return None;
        }
    }
    Some(found)
}

/// The IIM stream out of what a decoder hands back, which for JPEG is the whole Photoshop image
/// resource block (`8BIM`, resource 0x0404, name, size, data) and elsewhere may be the stream
/// itself. `None` when it is neither.
pub fn extract(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.first() == Some(&0x1C) {
        return datasets(bytes).map(|_| bytes.to_vec());
    }
    let mut at = 0;
    while at + 12 <= bytes.len() {
        if bytes.get(at..at + 4)? != b"8BIM" {
            return None;
        }
        let id = u16::from_be_bytes([bytes[at + 4], bytes[at + 5]]);
        let name = usize::from(bytes[at + 6]);
        // The name is a Pascal string, padded so that the length byte and name are even.
        let name_total = (1 + name).div_ceil(2) * 2;
        let size_at = at.checked_add(6)?.checked_add(name_total)?;
        let size = u32::from_be_bytes(bytes.get(size_at..size_at + 4)?.try_into().ok()?) as usize;
        let data_at = size_at + 4;
        let data = bytes.get(data_at..data_at.checked_add(size)?)?;
        if id == 0x0404 {
            return datasets(data).map(|_| data.to_vec());
        }
        at = data_at + size + (size & 1);
    }
    None
}

/// Whether the stream names a place.
pub fn has_location(stream: &[u8]) -> bool {
    datasets(stream).is_some_and(|sets| {
        sets.iter()
            .any(|set| set.record == 2 && LOCATION_DATASETS.contains(&set.number))
    })
}

/// Whether the stream is one this reader understands.
pub fn is_readable(stream: &[u8]) -> bool {
    datasets(stream).is_some()
}

/// The stream without the datasets that name a place. `None` when it cannot be read, in which
/// case it should not be carried.
pub fn without_location(stream: &[u8]) -> Option<Vec<u8>> {
    let sets = datasets(stream)?;
    let mut out = Vec::with_capacity(stream.len());
    for set in sets {
        if set.record == 2 && LOCATION_DATASETS.contains(&set.number) {
            continue;
        }
        out.extend_from_slice(set.whole);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dataset(record: u8, number: u8, text: &str) -> Vec<u8> {
        let mut bytes = vec![0x1C, record, number];
        bytes.extend_from_slice(&(text.len() as u16).to_be_bytes());
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }

    fn sample() -> Vec<u8> {
        let mut stream = Vec::new();
        stream.extend(dataset(2, 0, "\0\x04"));
        stream.extend(dataset(2, 25, "harbour"));
        stream.extend(dataset(2, 90, "Vancouver"));
        stream.extend(dataset(2, 92, "Granville Island"));
        stream.extend(dataset(2, 101, "Canada"));
        stream.extend(dataset(2, 116, "(c) A. Photographer"));
        stream
    }

    #[test]
    fn a_stream_is_read_and_its_place_found() {
        let stream = sample();
        assert!(is_readable(&stream));
        assert!(has_location(&stream));
        assert!(!has_location(&dataset(2, 25, "harbour")));
    }

    #[test]
    fn the_place_is_removed_and_the_rest_is_exactly_what_it_was() {
        let stream = sample();
        let cleaned = without_location(&stream).unwrap();
        let text = String::from_utf8_lossy(&cleaned).to_string();
        assert!(!text.contains("Vancouver"));
        assert!(!text.contains("Granville"));
        assert!(!text.contains("Canada"));
        assert!(text.contains("harbour"));
        assert!(text.contains("(c) A. Photographer"));
        assert!(!has_location(&cleaned));
        let expected: Vec<u8> = [
            dataset(2, 0, "\0\x04"),
            dataset(2, 25, "harbour"),
            dataset(2, 116, "(c) A. Photographer"),
        ]
        .concat();
        assert_eq!(cleaned, expected);
    }

    #[test]
    fn the_stream_is_found_inside_a_photoshop_resource_block_or_bare() {
        let stream = sample();
        let mut block = b"8BIM".to_vec();
        block.extend_from_slice(&0x0404_u16.to_be_bytes());
        block.extend_from_slice(&[0, 0]);
        block.extend_from_slice(&(stream.len() as u32).to_be_bytes());
        block.extend_from_slice(&stream);
        if stream.len() % 2 == 1 {
            block.push(0);
        }
        assert_eq!(extract(&block), Some(stream.clone()));
        assert_eq!(extract(&stream), Some(stream));
        assert_eq!(extract(b"8BIM\x04\x03\0\0\0\0\0\0"), None);
        assert_eq!(extract(b"hello, this is not it"), None);
    }

    #[test]
    fn a_stream_that_is_not_one_is_refused() {
        assert!(without_location(b"hello").is_none());
        assert!(without_location(&[0x1C, 2, 90, 0, 9, b'x']).is_none());
        assert!(without_location(&[0x1C, 2, 90, 0x80, 0x00]).is_none());
        assert!(without_location(&[]).is_some());
    }

    #[test]
    fn no_truncation_and_no_changed_byte_makes_it_panic() {
        let stream = sample();
        for len in 0..stream.len() {
            let _ = without_location(&stream[..len]);
            let _ = has_location(&stream[..len]);
        }
        for at in 0..stream.len() {
            let mut changed = stream.clone();
            changed[at] = 0xFF;
            let _ = without_location(&changed);
        }
    }
}
