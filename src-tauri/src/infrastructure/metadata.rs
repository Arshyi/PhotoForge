use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_EXIF_SCAN_BYTES: u64 = 8 * 1024 * 1024;

pub fn file_time(value: Result<SystemTime, std::io::Error>) -> Option<String> {
    value
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| format!("{}Z", duration.as_secs()))
}

pub fn camera_model(path: &Path) -> Option<String> {
    if fs::metadata(path).ok()?.len() > MAX_EXIF_SCAN_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    let tiff = jpeg_exif_tiff(&bytes)?;
    tiff_ascii_tag(tiff, 0x0110)
}

fn jpeg_exif_tiff(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.get(..2)? != [0xff, 0xd8] {
        return None;
    }
    let mut offset = 2;
    while offset + 4 <= bytes.len() {
        if bytes[offset] != 0xff {
            return None;
        }
        let marker = bytes[offset + 1];
        offset += 2;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if matches!(marker, 0x01 | 0xd0..=0xd7) {
            continue;
        }
        let length = u16::from_be_bytes([*bytes.get(offset)?, *bytes.get(offset + 1)?]) as usize;
        if length < 2 || offset + length > bytes.len() {
            return None;
        }
        let payload = &bytes[offset + 2..offset + length];
        if marker == 0xe1 && payload.starts_with(b"Exif\0\0") {
            return Some(&payload[6..]);
        }
        offset += length;
    }
    None
}

fn tiff_ascii_tag(tiff: &[u8], wanted_tag: u16) -> Option<String> {
    let little = match tiff.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let read_u16 = |offset: usize| -> Option<u16> {
        let bytes = [*tiff.get(offset)?, *tiff.get(offset + 1)?];
        Some(if little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    };
    let read_u32 = |offset: usize| -> Option<u32> {
        let bytes = [
            *tiff.get(offset)?,
            *tiff.get(offset + 1)?,
            *tiff.get(offset + 2)?,
            *tiff.get(offset + 3)?,
        ];
        Some(if little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    };
    if read_u16(2)? != 42 {
        return None;
    }
    let directory = read_u32(4)? as usize;
    let count = read_u16(directory)? as usize;
    for index in 0..count {
        let entry = directory + 2 + index * 12;
        if read_u16(entry)? != wanted_tag || read_u16(entry + 2)? != 2 {
            continue;
        }
        let length = read_u32(entry + 4)? as usize;
        if length == 0 || length > 256 {
            return None;
        }
        let value_offset = if length <= 4 {
            entry + 8
        } else {
            read_u32(entry + 8)? as usize
        };
        let value = tiff.get(value_offset..value_offset + length)?;
        return std::str::from_utf8(value)
            .ok()
            .map(|text| text.trim_end_matches('\0').trim().to_string())
            .filter(|text| !text.is_empty());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jpeg_with_model(model: &[u8]) -> Vec<u8> {
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II");
        tiff.extend_from_slice(&42_u16.to_le_bytes());
        tiff.extend_from_slice(&8_u32.to_le_bytes());
        tiff.extend_from_slice(&1_u16.to_le_bytes());
        tiff.extend_from_slice(&0x0110_u16.to_le_bytes());
        tiff.extend_from_slice(&2_u16.to_le_bytes());
        tiff.extend_from_slice(&(model.len() as u32).to_le_bytes());
        tiff.extend_from_slice(&26_u32.to_le_bytes());
        tiff.extend_from_slice(&0_u32.to_le_bytes());
        tiff.extend_from_slice(model);
        let mut payload = b"Exif\0\0".to_vec();
        payload.extend(tiff);
        let mut jpeg = vec![0xff, 0xd8, 0xff, 0xe1];
        jpeg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
        jpeg.extend(payload);
        jpeg.extend_from_slice(&[0xff, 0xd9]);
        jpeg
    }

    #[test]
    fn parses_camera_model_from_exif_app1() {
        let jpeg = jpeg_with_model(b"PhotoForge Camera\0");
        let tiff = jpeg_exif_tiff(&jpeg).unwrap();
        assert_eq!(
            tiff_ascii_tag(tiff, 0x0110).as_deref(),
            Some("PhotoForge Camera")
        );
    }

    #[test]
    fn rejects_truncated_exif_without_panicking() {
        for length in 0..20 {
            assert!(jpeg_exif_tiff(&jpeg_with_model(b"Test\0")[..length]).is_none());
        }
    }

    #[test]
    fn rejects_non_jpeg_data() {
        assert!(jpeg_exif_tiff(b"not a jpeg").is_none());
    }

    #[test]
    fn file_time_formats_epoch_seconds() {
        assert_eq!(file_time(Ok(UNIX_EPOCH)), Some("0Z".into()));
    }
}
