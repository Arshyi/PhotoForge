//! The OpenEXR import and export against real files, when some are provided.
//!
//! No EXR file is committed to the repository. Point `PHOTOFORGE_EXR_SAMPLES` at a directory of
//! `.exr` files (the OpenEXR project's own `openexr-images`, for instance) and this imports every
//! one, and exports every imported picture, printing one line per file. Without the variable the
//! test does nothing, and says so.
//!
//! With `PHOTOFORGE_EXR_DUMP` also set, what was imported is written there as raw little-endian
//! `f32` RGBA (`<name>.<layer>.f32`) with a `<name>.json` account, and what was exported as
//! `<name>.out.exr` (single precision) and `<name>.out16.exr` (half), so an independent reader can
//! be asked whether they say what the reference implementation says.
use photoforge_lib::layers::{LayerContent, LayerPixelStore};
use photoforge_lib::openexr::{export_exr, import_exr, ExrOptions};
use photoforge_lib::pixel::PixelBuffer;
use photoforge_lib::psd::ImportLimits;
use serde_json::json;
use std::path::{Path, PathBuf};

fn samples() -> Vec<PathBuf> {
    let Some(root) = std::env::var_os("PHOTOFORGE_EXR_SAMPLES") else {
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
        if path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("exr"))
        {
            found.push(path);
        }
    }
    found.sort();
    found
}

fn dump_directory() -> Option<PathBuf> {
    std::env::var_os("PHOTOFORGE_EXR_DUMP").map(PathBuf::from)
}

fn examine(path: &Path) -> Result<String, String> {
    let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
    let dump = dump_directory();
    let limits = ImportLimits::from_resources();
    let imported = match import_exr(path, limits) {
        Ok(imported) => imported,
        Err(error) => {
            if let Some(dump) = &dump {
                let account = json!({ "status": "refused", "message": error.to_string() });
                std::fs::write(dump.join(format!("{stem}.json")), account.to_string())
                    .map_err(|e| e.to_string())?;
            }
            return Ok(format!("refused: {error}"));
        }
    };
    let project = &imported.project;
    let document = &project.document;
    let mut layers = Vec::new();
    for (index, layer) in document.layers.iter().enumerate() {
        let LayerContent::Pixel {
            pixel_id,
            width,
            height,
        } = &layer.content
        else {
            continue;
        };
        let (_, image) = project
            .linear_pixels
            .iter()
            .find(|(id, _)| id == pixel_id)
            .ok_or("a pixel layer without pixels")?;
        if let Some(dump) = &dump {
            let mut bytes = Vec::with_capacity(image.pixels().len() * 16);
            for pixel in image.pixels() {
                for value in [pixel.red, pixel.green, pixel.blue, pixel.alpha] {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
            std::fs::write(dump.join(format!("{stem}.{index}.f32")), bytes)
                .map_err(|e| e.to_string())?;
        }
        layers.push(json!({
            "name": layer.name,
            "visible": layer.visible,
            "width": width,
            "height": height,
            "translateX": layer.transform.translate_x,
            "translateY": layer.transform.translate_y,
        }));
    }

    // Export what was imported, in both precisions.
    let mut store = LayerPixelStore::default();
    store
        .reset(document.canvas_width, document.canvas_height)
        .map_err(|e| e.to_string())?;
    for (id, image) in &project.linear_pixels {
        store
            .register_typed_with_id(id, PixelBuffer::from(image.clone()))
            .map_err(|e| e.to_string())?;
    }
    let resolved = store
        .resolve(&document.referenced_pixel_ids(), false)
        .map_err(|e| e.to_string())?;
    let single =
        export_exr(document, &resolved, &ExrOptions::default()).map_err(|e| e.to_string())?;
    let half = export_exr(
        document,
        &resolved,
        &ExrOptions {
            bit_depth: Some(16),
            ..ExrOptions::default()
        },
    )
    .map_err(|e| e.to_string())?;
    if let Some(dump) = &dump {
        std::fs::write(dump.join(format!("{stem}.out.exr")), &single.bytes)
            .map_err(|e| e.to_string())?;
        std::fs::write(dump.join(format!("{stem}.out16.exr")), &half.bytes)
            .map_err(|e| e.to_string())?;
        let account = json!({
            "status": "ok",
            "canvas": [document.canvas_width, document.canvas_height],
            "bitDepth": imported.report.bit_depth,
            "colorMode": imported.report.color_mode,
            "layers": layers,
            "entries": imported.report.entries.iter().map(|e| json!({
                "kind": format!("{:?}", e.kind),
                "layer": e.layer,
                "detail": e.detail,
            })).collect::<Vec<_>>(),
            "exportBytes": single.bytes.len(),
            "exportHalfBytes": half.bytes.len(),
            "exportColorMode": single.report.color_mode,
        });
        std::fs::write(dump.join(format!("{stem}.json")), account.to_string())
            .map_err(|e| e.to_string())?;
    }
    Ok(format!(
        "{}x{} {}-bit {}, {} layer(s); {} report entries; wrote {} / {} bytes",
        document.canvas_width,
        document.canvas_height,
        imported.report.bit_depth,
        imported.report.color_mode,
        layers.len(),
        imported.report.entries.len(),
        single.bytes.len(),
        half.bytes.len()
    ))
}

#[test]
fn real_exr_files_import_and_export() {
    let files = samples();
    if files.is_empty() {
        eprintln!("PHOTOFORGE_EXR_SAMPLES is not set (or holds no .exr): nothing to examine");
        return;
    }
    if let Some(dump) = dump_directory() {
        std::fs::create_dir_all(dump).unwrap();
    }
    let mut failures = Vec::new();
    let mut opened = 0;
    let mut refused = 0;
    for path in &files {
        let name = path.file_name().unwrap().to_string_lossy();
        match examine(path) {
            Ok(line) => {
                if line.starts_with("refused") {
                    refused += 1;
                } else {
                    opened += 1;
                }
                println!("{name}: {line}");
            }
            Err(error) => {
                println!("{name}: FAILED {error}");
                failures.push(format!("{name}: {error}"));
            }
        }
    }
    println!(
        "{} files: {opened} imported and exported, {refused} refused, {} failed",
        files.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{failures:#?}");
}
