use super::*;
use crate::color::FloatRgba;
use crate::layers::{LayerContent, LayerPixelStore, LoadedProject, ResolvedPixels};
use crate::openexr::import::import_exr;
use crate::openexr::testing::TestFile;
use crate::operations::structure::new_layer;
use crate::pixel::PixelBuffer;
use crate::psd::ImportLimits;
use exr::meta::MetaData;

fn limits() -> ImportLimits {
    ImportLimits {
        store_bytes: 1 << 30,
        entry_bytes: 1 << 30,
        job_bytes: 1 << 30,
    }
}

/// A document of float pixel layers, as the importer builds it, and the pixels behind it.
fn float_document(file: &TestFile) -> (LayerDocument, ResolvedPixels) {
    let directory = tempfile::tempdir().unwrap();
    let path = file.write_to(directory.path(), "in.exr");
    let imported = import_exr(&path, limits()).unwrap();
    resolve(imported.project)
}

fn resolve(project: LoadedProject) -> (LayerDocument, ResolvedPixels) {
    let mut store = LayerPixelStore::default();
    store
        .reset(
            project.document.canvas_width,
            project.document.canvas_height,
        )
        .unwrap();
    for (id, image) in &project.linear_pixels {
        store
            .register_typed_with_id(id, PixelBuffer::from(image.clone()))
            .unwrap();
    }
    for (id, image) in &project.pixels {
        store
            .register_typed_with_id(id, PixelBuffer::from(image.clone()))
            .unwrap();
    }
    let resolved = store
        .resolve(&project.document.referenced_pixel_ids(), false)
        .unwrap();
    (project.document, resolved)
}

fn read_back(bytes: &[u8]) -> crate::openexr::ImportedExr {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("out.exr");
    std::fs::write(&path, bytes).unwrap();
    import_exr(&path, limits()).unwrap()
}

fn pixel(imported: &crate::openexr::ImportedExr, x: u32, y: u32) -> FloatRgba {
    imported.project.linear_pixels[0].1.get(x, y).unwrap()
}

fn has(report: &ExrExportReport, kind: ReportKind, text: &str) -> bool {
    report
        .entries
        .iter()
        .any(|e| e.kind == kind && e.detail.contains(text))
}

fn gradient(width: usize, height: usize) -> TestFile {
    let alpha = |x: usize, _y: usize| 0.25 + 0.05 * x as f32;
    TestFile::new(width, height)
        .f32_channel("A", alpha)
        .f32_channel("B", move |x, y| 0.5 * alpha(x, y) * (1.0 + y as f32))
        .f32_channel("G", move |x, y| 0.05 * y as f32 * alpha(x, y))
        .f32_channel("R", move |x, y| 8.0 * x as f32 * alpha(x, y))
}

#[test]
fn a_picture_written_and_read_back_is_the_same_picture() {
    let (document, resolved) = float_document(&gradient(8, 6));
    let exported = export_exr(&document, &resolved, &ExrOptions::default()).unwrap();
    assert_eq!(exported.report.format, "OpenEXR");
    assert_eq!(exported.report.bit_depth, 32);
    assert_eq!(exported.report.color_mode, "RGBA");
    assert_eq!(exported.report.bytes, exported.bytes.len() as u64);
    let back = read_back(&exported.bytes);
    assert_eq!(back.project.document.canvas_width, 8);
    for y in 0..6 {
        for x in 0..8 {
            let wanted = {
                let a = 0.25 + 0.05 * x as f32;
                FloatRgba::new(8.0 * x as f32, 0.05 * y as f32, 0.5 * (1.0 + y as f32), a)
            };
            let got = pixel(&back, x, y);
            for (g, w) in [
                (got.red, wanted.red),
                (got.green, wanted.green),
                (got.blue, wanted.blue),
                (got.alpha, wanted.alpha),
            ] {
                assert!(
                    (g - w).abs() <= 1e-4 * w.abs().max(1.0),
                    "{x},{y}: {got:?} against {wanted:?}"
                );
            }
        }
    }
}

#[test]
fn colour_is_premultiplied_on_disk_as_openexr_stores_it() {
    let (document, resolved) = float_document(&gradient(4, 2));
    let exported = export_exr(&document, &resolved, &ExrOptions::default()).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("raw.exr");
    std::fs::write(&path, &exported.bytes).unwrap();
    let image = exr::prelude::read_first_rgba_layer_from_file(
        &path,
        |resolution, _| {
            vec![(0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32); resolution.width() * resolution.height()]
        },
        |pixels: &mut Vec<(f32, f32, f32, f32)>, position, (r, g, b, a): (f32, f32, f32, f32)| {
            pixels[position.y() * 4 + position.x()] = (r, g, b, a)
        },
    )
    .unwrap();
    let raw = &image.layer_data.channel_data.pixels;
    // Pixel (3, 1): straight colour (24, 0.05, 1.0), alpha 0.4.
    let (r, g, b, a) = raw[4 + 3];
    assert!((a - 0.4).abs() < 1e-6);
    assert!((r - 24.0 * 0.4).abs() < 1e-4, "{r}");
    assert!((g - 0.05 * 0.4).abs() < 1e-6, "{g}");
    assert!((b - 1.0 * 0.4).abs() < 1e-5, "{b}");
}

#[test]
fn the_file_states_the_primaries_its_colour_means() {
    let (document, resolved) = float_document(&gradient(4, 2));
    let exported = export_exr(&document, &resolved, &ExrOptions::default()).unwrap();
    let meta = MetaData::read_from_buffered(&exported.bytes[..], false).unwrap();
    let chromaticities = meta.headers[0]
        .shared_attributes
        .chromaticities
        .expect("the file carries a chromaticities attribute");
    let primaries = crate::openexr::colour::Primaries {
        red: (chromaticities.red.x(), chromaticities.red.y()),
        green: (chromaticities.green.x(), chromaticities.green.y()),
        blue: (chromaticities.blue.x(), chromaticities.blue.y()),
        white: (chromaticities.white.x(), chromaticities.white.y()),
    };
    assert!(primaries.is_rec709());
    let software = meta.headers[0]
        .own_attributes
        .software_name
        .as_ref()
        .unwrap();
    assert!(software.to_string().starts_with("PhotoForge "));
}

#[test]
fn an_opaque_picture_has_no_alpha_channel_and_a_translucent_one_does() {
    let opaque = TestFile::new(4, 4)
        .f32_channel("R", |_, _| 0.5)
        .f32_channel("G", |_, _| 0.5)
        .f32_channel("B", |_, _| 0.5);
    let (document, resolved) = float_document(&opaque);
    let exported = export_exr(&document, &resolved, &ExrOptions::default()).unwrap();
    assert_eq!(exported.report.color_mode, "RGB");
    let meta = MetaData::read_from_buffered(&exported.bytes[..], false).unwrap();
    let names: Vec<String> = meta.headers[0]
        .channels
        .list
        .iter()
        .map(|c| c.name.to_string())
        .collect();
    assert_eq!(names, ["B", "G", "R"]);

    let (document, resolved) = float_document(&gradient(4, 4));
    let exported = export_exr(&document, &resolved, &ExrOptions::default()).unwrap();
    let meta = MetaData::read_from_buffered(&exported.bytes[..], false).unwrap();
    assert_eq!(meta.headers[0].channels.list.len(), 4);
}

#[test]
fn half_precision_halves_the_samples_and_clamps_what_it_cannot_hold() {
    let file = TestFile::new(4, 1)
        .f32_channel("R", |x, _| [0.5, 1000.0, 70_000.0, -2.0][x])
        .f32_channel("G", |_, _| 0.25)
        .f32_channel("B", |_, _| 0.125);
    let (document, resolved) = float_document(&file);
    let single = export_exr(&document, &resolved, &ExrOptions::default()).unwrap();
    let half = export_exr(
        &document,
        &resolved,
        &ExrOptions {
            bit_depth: Some(16),
            ..ExrOptions::default()
        },
    )
    .unwrap();
    assert_eq!(half.report.bit_depth, 16);
    assert!(half.bytes.len() < single.bytes.len());
    let back = read_back(&half.bytes);
    assert_eq!(back.report.bit_depth, 16);
    assert_eq!(pixel(&back, 0, 0).red, 0.5);
    // 1000 is between half-precision neighbours 1000 and 1000.5 at its spacing of 0.5.
    assert!((pixel(&back, 1, 0).red - 1000.0).abs() <= 0.5);
    assert_eq!(pixel(&back, 2, 0).red, 65_504.0);
    assert_eq!(pixel(&back, 3, 0).red, -2.0);
    assert!(has(&half.report, ReportKind::Approximated, "1 pixel held"));
    // The file itself, read raw and not through the importer (which turns an infinity into 65,504
    // and would hide a clamp that was never made): the sample is 65,504, not infinity.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("half.exr");
    std::fs::write(&path, &half.bytes).unwrap();
    let raw = exr::prelude::read_first_rgba_layer_from_file(
        &path,
        |resolution, _| {
            vec![(0.0_f32, 0.0_f32, 0.0_f32, 0.0_f32); resolution.width() * resolution.height()]
        },
        |pixels: &mut Vec<(f32, f32, f32, f32)>, position, value: (f32, f32, f32, f32)| {
            pixels[position.y() * 4 + position.x()] = value
        },
    )
    .unwrap();
    assert_eq!(raw.layer_data.channel_data.pixels[2].0, 65_504.0);
    // Single precision keeps 70,000 and says nothing of the sort.
    let back = read_back(&single.bytes);
    assert_eq!(pixel(&back, 2, 0).red, 70_000.0);
    assert!(!has(
        &single.report,
        ReportKind::Approximated,
        "half precision"
    ));
}

#[test]
fn a_depth_openexr_does_not_hold_colour_in_is_refused() {
    let (document, resolved) = float_document(&gradient(4, 4));
    for depth in [8_u16, 24, 64] {
        let result = export_exr(
            &document,
            &resolved,
            &ExrOptions {
                bit_depth: Some(depth),
                ..ExrOptions::default()
            },
        );
        assert!(result.is_err(), "{depth}");
    }
}

#[test]
fn several_layers_are_composited_into_one_picture_and_that_is_said() {
    let file = TestFile::new(3, 3)
        .f32_channel("R", |_, _| 0.1)
        .f32_channel("G", |_, _| 0.1)
        .f32_channel("B", |_, _| 0.1)
        .f32_channel("top.R", |_, _| 0.9)
        .f32_channel("top.G", |_, _| 0.6)
        .f32_channel("top.B", |_, _| 0.3);
    let directory = tempfile::tempdir().unwrap();
    let path = file.write_to(directory.path(), "in.exr");
    let mut imported = import_exr(&path, limits()).unwrap();
    // The pass is imported hidden; switch it on and it covers the first.
    imported.project.document.layers[1].visible = true;
    let (document, resolved) = resolve(imported.project);
    let exported = export_exr(&document, &resolved, &ExrOptions::default()).unwrap();
    assert!(has(
        &exported.report,
        ReportKind::NotExported,
        "2 layers were composited"
    ));
    assert_eq!(exported.report.layers_written, 1);
    let back = read_back(&exported.bytes);
    let p = pixel(&back, 1, 1);
    assert!(
        (p.red - 0.9).abs() < 1e-5 && (p.green - 0.6).abs() < 1e-5 && (p.blue - 0.3).abs() < 1e-5
    );
}

#[test]
fn a_hidden_layer_is_not_in_the_file() {
    let file = TestFile::new(2, 2)
        .f32_channel("R", |_, _| 0.1)
        .f32_channel("G", |_, _| 0.1)
        .f32_channel("B", |_, _| 0.1)
        .f32_channel("top.R", |_, _| 0.9)
        .f32_channel("top.G", |_, _| 0.9)
        .f32_channel("top.B", |_, _| 0.9);
    let (document, resolved) = float_document(&file);
    assert!(!document.layers[1].visible);
    let exported = export_exr(&document, &resolved, &ExrOptions::default()).unwrap();
    let back = read_back(&exported.bytes);
    assert!((pixel(&back, 0, 0).red - 0.1).abs() < 1e-6);
}

#[test]
fn an_8_bit_document_is_decoded_to_linear_light_and_says_what_it_carries() {
    let mut document = LayerDocument::new(2, 1);
    let mut image = image::RgbaImage::new(2, 1);
    image.put_pixel(0, 0, image::Rgba([188, 0, 255, 255]));
    image.put_pixel(1, 0, image::Rgba([255, 255, 255, 255]));
    let layer = new_layer(
        "l1",
        "Photo",
        LayerContent::Pixel {
            pixel_id: "px1".to_string(),
            width: 2,
            height: 1,
        },
        "2026-01-01T00:00:00Z",
    );
    document.layers.push(layer);
    let project = LoadedProject {
        document,
        document_operations: Vec::new(),
        pixels: vec![("px1".to_string(), image)],
        linear_pixels: Vec::new(),
        application_version: "test".into(),
        created_at: String::new(),
        modified_at: String::new(),
        source_metadata: None,
    };
    let (document, resolved) = resolve(project);
    let exported = export_exr(&document, &resolved, &ExrOptions::default()).unwrap();
    assert!(has(&exported.report, ReportKind::Note, "8-bit"));
    let back = read_back(&exported.bytes);
    let p = pixel(&back, 0, 0);
    // sRGB 188 / 255 is 0.5029 in linear light.
    assert!((p.red - 0.5029).abs() < 1e-3, "{}", p.red);
    assert_eq!((p.green, p.blue, p.alpha), (0.0, 1.0, 1.0));
    assert_eq!(pixel(&back, 1, 0).red, 1.0);
}

#[test]
fn edits_over_the_document_are_in_the_picture_and_said() {
    let (document, resolved) = float_document(&gradient(4, 4));
    let plain = export_exr(&document, &resolved, &ExrOptions::default()).unwrap();
    let edited = export_exr(
        &document,
        &resolved,
        &ExrOptions {
            operations: vec![EditOperation::Brightness { amount: 0.5 }],
            bit_depth: None,
        },
    )
    .unwrap();
    assert!(has(&edited.report, ReportKind::Note, "own edits"));
    assert!(!has(&plain.report, ReportKind::Note, "own edits"));
    let a = read_back(&plain.bytes);
    let b = read_back(&edited.bytes);
    assert_ne!(pixel(&a, 2, 2), pixel(&b, 2, 2));
}

#[test]
fn the_document_is_not_changed_by_an_export() {
    let (document, resolved) = float_document(&gradient(4, 4));
    let before = document.clone();
    export_exr(&document, &resolved, &ExrOptions::default()).unwrap();
    assert_eq!(
        serde_json::to_value(&document).unwrap(),
        serde_json::to_value(&before).unwrap()
    );
}

#[test]
fn a_grey_gradient_in_a_document_survives_every_codec_the_writer_uses() {
    // The writer always uses ZIP; this pins that a large smooth picture is still smaller than
    // the raw samples, and reads back bit for bit.
    let file = TestFile::new(64, 64)
        .f32_channel("R", |x, y| (x + y) as f32 / 128.0)
        .f32_channel("G", |x, y| (x + y) as f32 / 128.0)
        .f32_channel("B", |x, y| (x + y) as f32 / 128.0);
    let (document, resolved) = float_document(&file);
    let exported = export_exr(&document, &resolved, &ExrOptions::default()).unwrap();
    assert!(exported.bytes.len() < 64 * 64 * 3 * 4);
    let back = read_back(&exported.bytes);
    for (x, y) in [(0_u32, 0_u32), (63, 63), (10, 50)] {
        assert_eq!(pixel(&back, x, y).red, (x + y) as f32 / 128.0);
    }
}
