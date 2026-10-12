//! The Photoshop importer against real files, with Photoshop's own flattened image as the oracle.
//!
//! Set `PHOTOFORGE_PSD_SAMPLES` to a file or directory of `.psd` / `.psb` files (none are
//! committed). Each file is imported, the resulting document is rendered by PhotoForge's own
//! compositor, and the result is compared with the merged image Photoshop stored in the file.
//! Where the importer reports nothing approximated or left out, the two must agree closely;
//! where it does report something, the difference is printed and not asserted.
use photoforge_lib::color::srgb_encode;
use photoforge_lib::error::AppError;
use photoforge_lib::layers::{render_document_typed, LayerPixelStore, RenderOptions};
use photoforge_lib::psd::{
    decode_merged_image, import_psd, read_document, ColorMode, ImportLimits, MergedSpec, Reader,
    ReportKind,
};
use std::fs::File;
use std::path::{Path, PathBuf};

fn samples() -> Vec<PathBuf> {
    let Some(root) = std::env::var_os("PHOTOFORGE_PSD_SAMPLES") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut pending = vec![PathBuf::from(root)];
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

struct Comparison {
    compared_pixels: u64,
    mean_abs: f64,
    within_2: f64,
    within_8: f64,
    max: u8,
}

/// The merged image as 8-bit sRGB RGB, or `None` if the file cannot supply one.
fn merged_rgb8(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let document = read_document(File::open(path).ok()?).ok()?;
    let header = &document.header;
    let image = document.image_data.as_ref()?;
    let needed: u16 = if header.mode == ColorMode::Grayscale {
        1
    } else {
        3
    };
    if header.channels < needed {
        return None;
    }
    let (width, height) = (u64::from(header.width), u64::from(header.height));
    let mut reader = Reader::new(File::open(path).ok()?).ok()?;
    let planes = decode_merged_image(
        &mut reader,
        header.version,
        &MergedSpec {
            offset: image.offset,
            channels: header.channels,
            width,
            height,
            depth: header.depth,
        },
        1 << 31,
    )
    .ok()?;
    let count = (width * height) as usize;
    let mut out = Vec::with_capacity(count * 3);
    let sample = |plane: &Vec<u8>, index: usize| -> u8 {
        match header.depth {
            8 => plane[index],
            16 => {
                let value = u16::from_be_bytes([plane[index * 2], plane[index * 2 + 1]]);
                (f32::from(value) / 65_535.0 * 255.0).round() as u8
            }
            _ => {
                let word = &plane[index * 4..index * 4 + 4];
                let linear = f32::from_be_bytes([word[0], word[1], word[2], word[3]]);
                (srgb_encode(linear.clamp(0.0, 1.0)) * 255.0).round() as u8
            }
        }
    };
    for index in 0..count {
        if needed == 1 {
            let value = sample(&planes[0], index);
            out.extend_from_slice(&[value, value, value]);
        } else {
            out.extend_from_slice(&[
                sample(&planes[0], index),
                sample(&planes[1], index),
                sample(&planes[2], index),
            ]);
        }
    }
    Some((header.width, header.height, out))
}

fn compare(path: &Path) -> Result<(photoforge_lib::psd::ImportReport, Option<Comparison>), String> {
    let imported = import_psd(
        path,
        ImportLimits {
            store_bytes: 8 << 30,
            entry_bytes: 4 << 30,
            job_bytes: 16 << 30,
        },
    )
    .map_err(|error| error.to_string())?;
    let project = imported.project;
    let document = project.document;
    let (width, height) = (document.canvas_width, document.canvas_height);
    let mut store = LayerPixelStore::default();
    store
        .reset(width, height)
        .map_err(|error| error.to_string())?;
    for (id, image) in project.pixels {
        store
            .register_with_id(&id, image)
            .map_err(|error| error.to_string())?;
    }
    for (id, image) in project.linear_pixels {
        store
            .register_typed_with_id(&id, image.into())
            .map_err(|error| error.to_string())?;
    }
    document.validate().map_err(|error| error.to_string())?;
    let resolved = store
        .resolve(&document.referenced_pixel_ids(), false)
        .map_err(|error| error.to_string())?;
    let rendered = render_document_typed(&document, &resolved, RenderOptions::default())
        .map_err(|error| error.to_string())?;
    let ours = rendered.encoded8();

    let Some((mw, mh, merged)) = merged_rgb8(path) else {
        return Ok((imported.report, None));
    };
    if (mw, mh) != (width, height) {
        return Err(format!(
            "merged image is {mw}x{mh}, canvas is {width}x{height}"
        ));
    }
    let (mut count, mut sum, mut close, mut near, mut max) = (0_u64, 0_u64, 0_u64, 0_u64, 0_u8);
    for (index, pixel) in ours.pixels().enumerate() {
        // Only where PhotoForge's picture is opaque: elsewhere the merged image's colour is
        // whatever Photoshop kept behind transparency, which is not a comparison.
        if pixel[3] < 250 {
            continue;
        }
        let mut worst = 0_u8;
        for channel in 0..3 {
            let diff = pixel[channel].abs_diff(merged[index * 3 + channel]);
            sum += u64::from(diff);
            worst = worst.max(diff);
        }
        if worst > 8
            && std::env::var("PHOTOFORGE_PSD_DEBUG")
                .is_ok_and(|needle| path.to_string_lossy().contains(&needle))
            && count < 6
        {
            println!(
                "  debug {}: pixel ({}, {}) ours {:?} merged {:?}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                index as u32 % width,
                index as u32 / width,
                pixel.0,
                &merged[index * 3..index * 3 + 3]
            );
        }
        count += 1;
        if worst <= 2 {
            close += 1;
        }
        if worst <= 8 {
            near += 1;
        }
        max = max.max(worst);
    }
    let comparison = (count > 0).then(|| Comparison {
        compared_pixels: count,
        mean_abs: sum as f64 / (count * 3) as f64,
        within_2: close as f64 / count as f64,
        within_8: near as f64 / count as f64,
        max,
    });
    Ok((imported.report, comparison))
}

#[test]
fn real_photoshop_files_import_and_match_photoshops_own_flattened_image() {
    let files = samples();
    if files.is_empty() {
        eprintln!(
            "PHOTOFORGE_PSD_SAMPLES is not set (or empty): no real Photoshop files to import"
        );
        return;
    }
    let (mut imported, mut refused, mut failed) = (0, 0, Vec::new());
    let (mut clean, mut clean_close, mut clean_bad) = (0, 0, Vec::new());
    let (mut dirty, mut dirty_close) = (0, 0);
    let mut reasons: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    // Files whose own stored flattened image is not the picture of their layers (see below).
    const STALE_FLATTENED: [&str; 1] = ["layers-minimal/pixel-layer.psd"];
    for path in &files {
        let name = path
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        let name = name
            .rsplit("psd_files/")
            .next()
            .unwrap_or(&name)
            .to_string();
        // Files in a colour mode or depth the importer names as unsupported must say so.
        let header = read_document(File::open(path).unwrap()).unwrap().header;
        let expect_refusal = !matches!(header.mode, ColorMode::Rgb | ColorMode::Grayscale)
            || !matches!(header.depth, 8 | 16 | 32);
        match compare(path) {
            Ok((report, comparison)) => {
                if expect_refusal {
                    failed.push(format!("{name}: imported although its mode is unsupported"));
                    continue;
                }
                imported += 1;
                let approximated = report.count(ReportKind::Approximated);
                let left_out = report.count(ReportKind::NotImported);
                for entry in &report.entries {
                    if entry.kind == ReportKind::Note {
                        continue;
                    }
                    let mut text = entry.detail.clone();
                    // Fold the parts that vary per layer.
                    for cut in [" Blend mode", " is not available"] {
                        let _ = cut;
                    }
                    if let Some(index) = text.find(" is not available") {
                        text = format!("Blend mode ... {}", &text[index..]);
                    }
                    text.truncate(110);
                    *reasons
                        .entry(format!("{:?}: {text}", entry.kind))
                        .or_default() += 1;
                }
                let summary = match &comparison {
                    Some(c) => format!(
                        "mean |diff| {:.2}, {:.1}% within 2, {:.1}% within 8, max {}, {} px",
                        c.mean_abs,
                        c.within_2 * 100.0,
                        c.within_8 * 100.0,
                        c.max,
                        c.compared_pixels
                    ),
                    None => "no flattened image to compare".to_string(),
                };
                println!(
                    "OK   {name}: {}x{} {}-bit, {} layers + {} groups, {approximated} approximated, {left_out} left out; {summary}",
                    report.width, report.height, report.bit_depth, report.layers_imported, report.groups_imported
                );
                if let Some(c) = comparison {
                    if approximated + left_out == 0 {
                        clean += 1;
                        if c.mean_abs <= 2.0 && c.within_8 >= 0.97 {
                            clean_close += 1;
                        } else if STALE_FLATTENED.contains(&name.as_str()) {
                            println!("     (known: this file's stored flattened image is blank)");
                        } else {
                            clean_bad.push(format!("{name}: {summary}"));
                        }
                    } else {
                        dirty += 1;
                        if c.mean_abs <= 2.0 && c.within_8 >= 0.97 {
                            dirty_close += 1;
                        }
                    }
                }
            }
            Err(reason) => {
                // A refusal the importer chose is fine for an unsupported file only.
                if expect_refusal {
                    refused += 1;
                    println!("REFUSED {name}: {reason}");
                } else {
                    println!("FAIL {name}: {reason}");
                    failed.push(format!("{name}: {reason}"));
                }
            }
        }
    }
    println!(
        "\n{} files: {imported} imported, {refused} refused (unsupported mode or depth), {} failed",
        files.len(),
        failed.len()
    );
    println!("imports reporting nothing approximated or left out: {clean}, of which close to Photoshop's image: {clean_close}");
    println!("imports reporting approximations: {dirty}, of which still close: {dirty_close}");
    for bad in &clean_bad {
        println!("CLEAN BUT DIFFERENT {bad}");
    }
    let mut ranked: Vec<_> = reasons.into_iter().collect();
    ranked.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    println!(
        "
what the reports said, by how many layers:"
    );
    for (reason, count) in ranked.iter().take(40) {
        println!("  {count:>5}  {reason}");
    }
    assert!(
        clean_bad.is_empty(),
        "imports that reported nothing but differ from Photoshop's image: {clean_bad:?}"
    );
    let _ = AppError::DecodeFailure;
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
