//! The Photoshop exporter against real files: each is imported, written back out, imported again,
//! and the two documents compared as structures and as pictures.
//!
//! Set `PHOTOFORGE_PSD_SAMPLES` to a file or directory of `.psd` / `.psb` files (none are
//! committed). Where the first import reported nothing approximated or left out, the second must be
//! the same document: the same layers in the same order with the same names, and the same picture
//! to within a level. Elsewhere the difference is printed and not asserted, because an adjustment
//! the importer sampled is sampled again.
use photoforge_lib::layers::{
    render_document_typed, Layer, LayerContent, LayerDocument, LayerPixelStore, RenderOptions,
};
use photoforge_lib::psd::{export_psd, import_psd, ExportOptions, ImportLimits, ReportKind};
use std::path::PathBuf;

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

fn limits() -> ImportLimits {
    ImportLimits {
        store_bytes: 8 << 30,
        entry_bytes: 4 << 30,
        job_bytes: 16 << 30,
    }
}

fn outline(layers: &[Layer], out: &mut Vec<String>, depth: usize) {
    for layer in layers {
        let kind = match &layer.content {
            LayerContent::Pixel { .. } => "pixel",
            LayerContent::Group { isolated: true, .. } => "group",
            LayerContent::Group {
                isolated: false, ..
            } => "group-pass",
            LayerContent::Adjustment { .. } => "adjustment",
            LayerContent::Shape { .. } => "shape",
            LayerContent::Text { .. } => "text",
            LayerContent::SmartObject { .. } => "smart",
        };
        out.push(format!(
            "{}{} {:?} {:?} v{} c{} m{}",
            "  ".repeat(depth),
            kind,
            layer.name,
            layer.blend_mode,
            u8::from(layer.visible),
            u8::from(layer.clipped),
            u8::from(layer.mask.is_some()),
        ));
        if let LayerContent::Group { children, .. } = &layer.content {
            outline(children, out, depth + 1);
        }
    }
}

fn render(imported: photoforge_lib::psd::Imported) -> (LayerDocument, image::RgbaImage) {
    let project = imported.project;
    let document = project.document;
    let mut store = LayerPixelStore::default();
    store
        .reset(document.canvas_width, document.canvas_height)
        .unwrap();
    for (id, image) in project.pixels {
        store.register_with_id(&id, image).unwrap();
    }
    for (id, image) in project.linear_pixels {
        store.register_typed_with_id(&id, image.into()).unwrap();
    }
    let resolved = store
        .resolve(&document.referenced_pixel_ids(), false)
        .unwrap();
    let rendered = render_document_typed(&document, &resolved, RenderOptions::default()).unwrap();
    (document, rendered.encoded8().as_ref().clone())
}

#[test]
fn real_photoshop_files_survive_being_written_and_read_again() {
    let files = samples();
    if files.is_empty() {
        eprintln!("PHOTOFORGE_PSD_SAMPLES is not set; nothing to check");
        return;
    }
    let directory = tempfile::tempdir().unwrap();
    let (mut checked, mut refused, mut exact, mut close, mut differ) = (0, 0, 0, 0, 0);
    let mut problems = Vec::new();
    let mut reports: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut total_bytes = 0_u64;
    let mut slowest = (0_u128, String::new());
    for path in files {
        let name = path.to_string_lossy().replace('\\', "/");
        let name = name
            .rsplit("psd_files/")
            .next()
            .unwrap_or(&name)
            .to_string();
        let Ok(first) = import_psd(&path, limits()) else {
            refused += 1;
            continue;
        };
        let clean = first.report.count(ReportKind::Approximated) == 0
            && first.report.count(ReportKind::NotImported) == 0;
        let mut before_outline = Vec::new();
        outline(&first.project.document.layers, &mut before_outline, 0);
        let (_, before) = render(first);

        let output = directory.path().join("again.psd");
        let started = std::time::Instant::now();
        // Export needs the pixels of the first import, which `render` consumed: import again.
        let again = import_psd(&path, limits()).unwrap();
        let document = again.project.document.clone();
        let mut store = LayerPixelStore::default();
        store
            .reset(document.canvas_width, document.canvas_height)
            .unwrap();
        for (id, image) in again.project.pixels {
            store.register_with_id(&id, image).unwrap();
        }
        for (id, image) in again.project.linear_pixels {
            store.register_typed_with_id(&id, image.into()).unwrap();
        }
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let report = match export_psd(&document, &resolved, &output, &ExportOptions::default()) {
            Ok(report) => report,
            Err(error) => {
                problems.push(format!("{name}: export failed: {error}"));
                continue;
            }
        };
        let elapsed = started.elapsed().as_millis();
        if elapsed > slowest.0 {
            slowest = (elapsed, name.clone());
        }
        total_bytes += report.bytes;
        for entry in &report.entries {
            *reports
                .entry(format!("{:?}: {}", entry.kind, entry.detail))
                .or_default() += 1;
        }
        let second = match import_psd(&output, limits()) {
            Ok(second) => second,
            Err(error) => {
                problems.push(format!(
                    "{name}: the file written could not be read back: {error}"
                ));
                continue;
            }
        };
        let (written, after) = render(second);
        let mut after_outline = Vec::new();
        outline(&written.layers, &mut after_outline, 0);
        checked += 1;

        if before.dimensions() != after.dimensions() {
            problems.push(format!("{name}: the picture changed size"));
            continue;
        }
        let (mut sum, mut max, mut count) = (0_u64, 0_u8, 0_u64);
        for (p, q) in before.pixels().zip(after.pixels()) {
            for channel in 0..3 {
                // Only where there is a picture: beneath transparency the colour is whatever.
                if p[3] == 0 && q[3] == 0 {
                    continue;
                }
                let d = p[channel].abs_diff(q[channel]);
                sum += u64::from(d);
                max = max.max(d);
                count += 1;
            }
            let d = p[3].abs_diff(q[3]);
            sum += u64::from(d);
            max = max.max(d);
            count += 1;
        }
        let mean = sum as f64 / count.max(1) as f64;
        if max == 0 {
            exact += 1;
        } else if mean < 1.0 {
            close += 1;
        } else {
            differ += 1;
        }
        let same_structure = before_outline == after_outline;
        println!(
            "{} {name}: {} layers, {} bytes, {} ms; mean {mean:.3} max {max}{}",
            if same_structure { "OK  " } else { "DIFF" },
            before_outline.len(),
            report.bytes,
            elapsed,
            if same_structure {
                ""
            } else {
                "; the layers are not the same"
            },
        );
        if clean && !same_structure {
            problems.push(format!(
                "{name}: a clean import came back with different layers"
            ));
        }
        if clean && mean >= 1.0 {
            problems.push(format!(
                "{name}: a clean import came back {mean:.2} levels different"
            ));
        }
    }
    println!(
        "\n{checked} files written and read again ({refused} refused on import): \
         {exact} identical, {close} within a level on average, {differ} further; \
         {} MiB written; slowest {} ms ({})",
        total_bytes >> 20,
        slowest.0,
        slowest.1
    );
    println!("what the exports said, by how many files:");
    let mut lines: Vec<_> = reports.into_iter().collect();
    lines.sort_by_key(|line| std::cmp::Reverse(line.1));
    for (line, count) in lines.iter().take(20) {
        println!("  {count:4}  {}", &line[..line.len().min(150)]);
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
