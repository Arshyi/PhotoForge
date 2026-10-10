//! The PSD reader against real Photoshop files, when some are provided.
//!
//! No Photoshop file is committed to the repository (their licences are not ours to carry).
//! Point `PHOTOFORGE_PSD_SAMPLES` at a directory of `.psd` / `.psb` files (or at one file) and
//! this reads every one, decodes every channel of every layer that is RGB or greyscale at 8, 16 or
//! 32 bits, and prints what it found. Without the variable the test does nothing, and says so.
//!
//! Expectations, per file: the structure reads; every channel of a supported mode decodes to
//! exactly the size its rectangle declares; the group tree builds. A file in a mode the
//! reader names as unsupported (CMYK, Lab, …) must be reported as such rather than failing.
use photoforge_lib::psd::{
    build_tree, decode_channel, read_document, ChannelSpec, ColorMode, PsdError, Reader,
};
use std::fs::File;
use std::path::{Path, PathBuf};

fn samples() -> Vec<PathBuf> {
    let Some(root) = std::env::var_os("PHOTOFORGE_PSD_SAMPLES") else {
        return Vec::new();
    };
    let root = PathBuf::from(root);
    let mut found = Vec::new();
    let mut pending = vec![root];
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&path) {
                pending.extend(entries.flatten().map(|entry| entry.path()));
            }
            continue;
        }
        let is_psd = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("psd") || ext.eq_ignore_ascii_case("psb"));
        if is_psd {
            found.push(path);
        }
    }
    found.sort();
    found
}

fn key_text(key: &[u8; 4]) -> String {
    key.iter().map(|byte| char::from(*byte)).collect()
}

fn examine(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|error| format!("open: {error}"))?;
    let document = read_document(file).map_err(|error| format!("structure: {error}"))?;
    let header = &document.header;
    let mut line = format!(
        "{:?} {}x{} {}-bit {:?}, {} channels; {} records{}",
        header.version,
        header.width,
        header.height,
        header.depth,
        header.mode,
        header.channels,
        document.layers.records.len(),
        document
            .layers
            .from_block
            .map(|key| format!(" (in {})", key_text(&key)))
            .unwrap_or_default(),
    );

    let tree = build_tree(&document.layers.records, 4_096, 64)
        .map_err(|error| format!("{line}\n  tree: {error}"))?;
    line.push_str(&format!("; {} top-level nodes", tree.len()));

    let mut blends = std::collections::BTreeMap::<String, usize>::new();
    let mut keys = std::collections::BTreeMap::<String, usize>::new();
    for record in &document.layers.records {
        *blends.entry(key_text(&record.blend_key)).or_default() += 1;
        for key in &record.block_keys {
            *keys.entry(key_text(key)).or_default() += 1;
        }
    }
    line.push_str(&format!(
        "\n  blend keys: {blends:?}\n  block keys: {keys:?}"
    ));
    line.push_str(&format!(
        "\n  resources: {} blocks, ICC {}",
        document.resources.blocks.len(),
        document
            .resources
            .icc_profile
            .as_ref()
            .map_or("none".to_string(), |icc| format!("{} bytes", icc.len()))
    ));

    let readable = matches!(header.mode, ColorMode::Rgb | ColorMode::Grayscale)
        && matches!(header.depth, 8 | 16 | 32);
    if !readable {
        line.push_str("\n  pixels: not decoded (mode or depth outside what the importer reads)");
        return Ok(line);
    }
    let file = File::open(path).map_err(|error| format!("reopen: {error}"))?;
    let mut reader = Reader::new(file).map_err(|error| error.to_string())?;
    let (mut decoded, mut unsupported) = (0_u32, 0_u32);
    for record in &document.layers.records {
        for channel in &record.channels {
            let (width, height) = if channel.id <= -2 {
                match &record.mask {
                    Some(mask) if channel.id == -3 => match &mask.real {
                        Some(real) => (real.rect.width(), real.rect.height()),
                        None => (mask.rect.width(), mask.rect.height()),
                    },
                    Some(mask) => (mask.rect.width(), mask.rect.height()),
                    None => continue,
                }
            } else {
                (record.rect.width(), record.rect.height())
            };
            if width == 0 || height == 0 {
                continue;
            }
            let spec = ChannelSpec {
                offset: channel.offset,
                length: channel.length,
                width,
                height,
                depth: header.depth,
            };
            match decode_channel(&mut reader, header.version, &spec, 1 << 30) {
                Ok(bytes) => {
                    let expected = width * height * u64::from(header.depth / 8);
                    if bytes.len() as u64 != expected {
                        return Err(format!(
                            "{line}\n  channel {} of '{}' decoded to {} bytes, expected {expected}",
                            channel.id,
                            record.display_name(),
                            bytes.len()
                        ));
                    }
                    if header.depth == 32 {
                        // Samples are big-endian floats: a wrong byte order or plane layout shows
                        // up at once as NaN, infinity or absurd magnitudes.
                        let bad = bytes
                            .chunks_exact(4)
                            .map(|word| f32::from_be_bytes([word[0], word[1], word[2], word[3]]))
                            .filter(|value| !value.is_finite() || value.abs() > 1.0e6)
                            .count();
                        if bad > 0 {
                            return Err(format!(
                                "{line}
  channel {} of '{}': {bad} implausible float samples",
                                channel.id,
                                record.display_name()
                            ));
                        }
                    }
                    decoded += 1;
                }
                Err(PsdError::Unsupported { .. }) => unsupported += 1,
                Err(error) => {
                    return Err(format!(
                        "{line}\n  channel {} of '{}': {error}",
                        channel.id,
                        record.display_name()
                    ));
                }
            }
        }
    }
    line.push_str(&format!(
        "\n  channels decoded: {decoded}; refused as unsupported: {unsupported}"
    ));
    Ok(line)
}

#[test]
fn real_photoshop_files_read_when_provided() {
    let files = samples();
    if files.is_empty() {
        eprintln!("PHOTOFORGE_PSD_SAMPLES is not set (or empty): no real Photoshop files to read");
        return;
    }
    let mut failures = Vec::new();
    for path in &files {
        let name = path
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let name = name
            .rsplit("psd_files/")
            .next()
            .unwrap_or(&name)
            .to_string();
        match examine(path) {
            Ok(report) => println!("OK   {name}: {report}"),
            Err(reason) => {
                println!("FAIL {name}: {reason}");
                failures.push(format!("{name}: {reason}"));
            }
        }
    }
    println!("{} file(s), {} failed", files.len(), failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
