use super::*;
use crate::openexr::testing::{multipart, patch_compression, TestFile};
use crate::psd::ReportKind;
use exr::math::Vec2;
use exr::meta::attribute::Compression as Codec;

fn limits() -> ImportLimits {
    ImportLimits {
        store_bytes: 1 << 30,
        entry_bytes: 1 << 30,
        job_bytes: 1 << 30,
    }
}

fn import_bytes(bytes: &[u8]) -> Result<ImportedExr, AppError> {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("t.exr");
    std::fs::write(&path, bytes).unwrap();
    import_exr(&path, limits())
}

fn import_file(file: &TestFile) -> ImportedExr {
    import_bytes(&file.bytes()).expect("the file imports")
}

fn has(report: &ExrImportReport, kind: ReportKind, text: &str) -> bool {
    report
        .entries
        .iter()
        .any(|e| e.kind == kind && e.detail.contains(text))
}

fn pixel(imported: &ImportedExr, layer: usize, x: u32, y: u32) -> FloatRgba {
    imported.project.linear_pixels[layer]
        .1
        .get(x, y)
        .expect("pixel in range")
}

fn close(a: f32, b: f32, tolerance: f32) -> bool {
    (a - b).abs() <= tolerance
}

fn error_text(result: Result<ImportedExr, AppError>) -> String {
    match result {
        Ok(_) => panic!("expected the import to be refused"),
        Err(error) => error.to_string(),
    }
}

/// An RGBA file whose straight colour is `(0.1 x, 0.05 y, 0.5)` and alpha is `0.25 + 0.05 x`,
/// stored premultiplied as OpenEXR does.
fn premultiplied_rgba(width: usize, height: usize) -> TestFile {
    let alpha = |x: usize, _y: usize| 0.25 + 0.05 * x as f32;
    TestFile::new(width, height)
        .f32_channel("A", alpha)
        .f32_channel("B", move |x, y| 0.5 * alpha(x, y))
        .f32_channel("G", move |x, y| 0.05 * y as f32 * alpha(x, y))
        .f32_channel("R", move |x, y| 0.1 * x as f32 * alpha(x, y))
}

#[test]
fn a_plain_rgba_file_opens_as_one_visible_high_precision_layer() {
    let imported = import_file(&premultiplied_rgba(8, 6));
    let project = &imported.project;
    assert_eq!(
        (
            project.document.canvas_width,
            project.document.canvas_height
        ),
        (8, 6)
    );
    assert_eq!(project.document.precision, DocumentPrecision::LinearSrgbF32);
    assert_eq!(project.document.layers.len(), 1);
    assert!(project.document.layers[0].visible);
    assert_eq!(project.document.layers[0].name, "Image");
    assert!(project.pixels.is_empty());
    assert_eq!(project.linear_pixels.len(), 1);
    let report = &imported.report;
    assert_eq!(report.format, "OpenEXR");
    assert_eq!((report.width, report.height), (8, 6));
    assert_eq!(report.bit_depth, 32);
    assert_eq!(report.color_mode, "RGB");
    assert_eq!(report.layers_imported, 1);
    assert_eq!(report.count(ReportKind::NotImported), 0);
    assert_eq!(report.count(ReportKind::Approximated), 0);
}

#[test]
fn premultiplied_colour_comes_across_as_straight_alpha() {
    let imported = import_file(&premultiplied_rgba(8, 6));
    for (x, y) in [(0_u32, 0_u32), (3, 2), (7, 5)] {
        let p = pixel(&imported, 0, x, y);
        assert!(close(p.red, 0.1 * x as f32, 1e-6), "{x},{y}: {}", p.red);
        assert!(
            close(p.green, 0.05 * y as f32, 1e-6),
            "{x},{y}: {}",
            p.green
        );
        assert!(close(p.blue, 0.5, 1e-6));
        assert!(close(p.alpha, 0.25 + 0.05 * x as f32, 1e-6));
    }
}

#[test]
fn half_float_files_are_16_bit_and_keep_their_values() {
    let file = TestFile::new(4, 4)
        .f16_channel("R", |x, _| 0.5 * x as f32)
        .f16_channel("G", |_, _| 0.25)
        .f16_channel("B", |_, _| 2.0);
    let imported = import_file(&file);
    assert_eq!(imported.report.bit_depth, 16);
    let p = pixel(&imported, 0, 3, 1);
    assert_eq!((p.red, p.green, p.blue, p.alpha), (1.5, 0.25, 2.0, 1.0));
}

#[test]
fn light_brighter_than_white_and_below_black_is_kept() {
    let file = TestFile::new(2, 2)
        .f32_channel("R", |_, _| 40.0)
        .f32_channel("G", |_, _| -0.5)
        .f32_channel("B", |_, _| 1000.0);
    let imported = import_file(&file);
    let p = pixel(&imported, 0, 1, 1);
    assert_eq!((p.red, p.green, p.blue), (40.0, -0.5, 1000.0));
}

#[test]
fn a_file_without_alpha_is_opaque() {
    let file = TestFile::new(3, 3)
        .f32_channel("R", |_, _| 0.2)
        .f32_channel("G", |_, _| 0.4)
        .f32_channel("B", |_, _| 0.6);
    let imported = import_file(&file);
    assert_eq!(pixel(&imported, 0, 1, 1).alpha, 1.0);
}

#[test]
fn a_luminance_only_file_is_a_grey_picture() {
    let file = TestFile::new(4, 3).f32_channel("Y", |x, _| 0.1 * x as f32);
    let imported = import_file(&file);
    assert_eq!(imported.report.color_mode, "Grayscale");
    let p = pixel(&imported, 0, 3, 0);
    assert!(close(p.red, 0.3, 1e-6) && p.red == p.green && p.green == p.blue);
    assert!(has(&imported.report, ReportKind::Note, "luminance"));
}

#[test]
fn a_luminance_chroma_file_is_not_read_as_its_grey_half() {
    let file = TestFile::new(4, 4)
        .f32_channel("Y", |_, _| 0.5)
        .f32_channel("RY", |_, _| 0.1)
        .f32_channel("BY", |_, _| 0.1);
    let message = error_text(import_bytes(&file.bytes()));
    assert!(message.contains("luminance and chroma"), "{message}");
}

#[test]
fn the_long_channel_names_some_compositors_write_are_read() {
    let file = TestFile::new(2, 2)
        .f32_channel("red", |_, _| 0.1)
        .f32_channel("green", |_, _| 0.2)
        .f32_channel("blue", |_, _| 0.3)
        .f32_channel("alpha", |_, _| 1.0);
    let imported = import_file(&file);
    let p = pixel(&imported, 0, 0, 0);
    assert_eq!((p.red, p.green, p.blue), (0.1, 0.2, 0.3));
}

#[test]
fn a_picture_that_only_exists_under_a_prefix_is_read_and_named_for_it() {
    let file = TestFile::new(3, 3)
        .f32_channel("beauty.R", |_, _| 0.3)
        .f32_channel("beauty.G", |_, _| 0.2)
        .f32_channel("beauty.B", |_, _| 0.1);
    let imported = import_file(&file);
    assert_eq!(imported.project.document.layers[0].name, "beauty");
    assert_eq!(pixel(&imported, 0, 0, 0).red, 0.3);
}

#[test]
fn a_pass_beside_the_main_picture_is_imported_hidden_and_does_not_borrow_its_alpha() {
    let file = TestFile::new(3, 3)
        .f32_channel("A", |_, _| 0.5)
        .f32_channel("B", |_, _| 0.1)
        .f32_channel("G", |_, _| 0.1)
        .f32_channel("R", |_, _| 0.1)
        .f32_channel("diffuse.R", |_, _| 0.8)
        .f32_channel("diffuse.G", |_, _| 0.7)
        .f32_channel("diffuse.B", |_, _| 0.6);
    let imported = import_file(&file);
    let layers = &imported.project.document.layers;
    assert_eq!(layers.len(), 2);
    assert!(layers[0].visible);
    assert_eq!(layers[0].name, "Image");
    assert!(!layers[1].visible);
    assert_eq!(layers[1].name, "diffuse");
    // The pass has no alpha of its own: it is opaque, not half as covering as the main picture.
    let pass = pixel(&imported, 1, 0, 0);
    assert_eq!(
        (pass.red, pass.green, pass.blue, pass.alpha),
        (0.8, 0.7, 0.6, 1.0)
    );
    // The main picture's colour was premultiplied by 0.5.
    assert!(close(pixel(&imported, 0, 0, 0).red, 0.2, 1e-6));
    assert!(has(&imported.report, ReportKind::Note, "pass"));
}

#[test]
fn channels_that_are_not_colour_are_named_not_dropped_silently() {
    let file = TestFile::new(2, 2)
        .f32_channel("R", |_, _| 0.1)
        .f32_channel("G", |_, _| 0.1)
        .f32_channel("B", |_, _| 0.1)
        .f32_channel("Z", |_, _| 5.0)
        .u32_channel("objectId", |_, _| 7);
    let imported = import_file(&file);
    assert!(has(
        &imported.report,
        ReportKind::NotImported,
        "2 other channels"
    ));
    assert!(has(&imported.report, ReportKind::NotImported, "Z"));
    assert!(has(&imported.report, ReportKind::NotImported, "objectId"));
}

#[test]
fn a_file_with_no_colour_picture_is_refused_and_says_what_it_has() {
    let file = TestFile::new(2, 2).f32_channel("Z", |_, _| 1.0);
    let message = error_text(import_bytes(&file.bytes()));
    assert!(
        message.contains("[Z]") && message.contains("colour picture"),
        "{message}"
    );
    let ids = TestFile::new(2, 2)
        .u32_channel("R", |_, _| 1)
        .u32_channel("G", |_, _| 1)
        .u32_channel("B", |_, _| 1);
    assert!(import_bytes(&ids.bytes()).is_err());
}

#[test]
fn a_file_in_another_colour_space_is_converted_to_srgb_primaries() {
    // Pure ACEScg red, which is far outside sRGB.
    let ap1 = [
        (0.713, 0.293),
        (0.165, 0.830),
        (0.128, 0.044),
        (0.32168, 0.33767),
    ];
    let file = TestFile::new(2, 2)
        .f32_channel("R", |_, _| 1.0)
        .f32_channel("G", |_, _| 0.0)
        .f32_channel("B", |_, _| 0.0)
        .chromaticities(ap1);
    let imported = import_file(&file);
    let p = pixel(&imported, 0, 0, 0);
    // The first column of the published ACEScg-to-sRGB matrix.
    assert!(close(p.red, 1.70505, 3e-3), "{}", p.red);
    assert!(close(p.green, -0.13026, 3e-3), "{}", p.green);
    assert!(close(p.blue, -0.02400, 3e-3), "{}", p.blue);
    assert!(has(&imported.report, ReportKind::Note, "primaries"));
}

#[test]
fn rec709_chromaticities_change_nothing_and_say_nothing() {
    let rec709 = [(0.64, 0.33), (0.30, 0.60), (0.15, 0.06), (0.3127, 0.3290)];
    let file = TestFile::new(2, 2)
        .f32_channel("R", |_, _| 0.9)
        .f32_channel("G", |_, _| 0.3)
        .f32_channel("B", |_, _| 0.1)
        .chromaticities(rec709);
    let imported = import_file(&file);
    let p = pixel(&imported, 0, 0, 0);
    assert!(close(p.red, 0.9, 1e-6) && close(p.green, 0.3, 1e-6) && close(p.blue, 0.1, 1e-6));
    assert!(!has(&imported.report, ReportKind::Note, "primaries"));
}

#[test]
fn white_chromaticities_that_are_nonsense_are_read_as_rec709_and_said() {
    let nonsense = [(0.3, 0.3), (0.3, 0.3), (0.3, 0.3), (0.3127, 0.3290)];
    let file = TestFile::new(2, 2)
        .f32_channel("R", |_, _| 0.9)
        .f32_channel("G", |_, _| 0.3)
        .f32_channel("B", |_, _| 0.1)
        .chromaticities(nonsense);
    let imported = import_file(&file);
    assert!(close(pixel(&imported, 0, 0, 0).red, 0.9, 1e-6));
    assert!(has(
        &imported.report,
        ReportKind::Approximated,
        "do not describe"
    ));
}

#[test]
fn a_data_window_inside_a_larger_display_window_sits_where_the_file_puts_it() {
    let file = TestFile::new(4, 3)
        .f32_channel("R", |_, _| 0.5)
        .f32_channel("G", |_, _| 0.5)
        .f32_channel("B", |_, _| 0.5)
        .at(2, 1)
        .display(8, 6);
    let imported = import_file(&file);
    let document = &imported.project.document;
    assert_eq!((document.canvas_width, document.canvas_height), (8, 6));
    let layer = &document.layers[0];
    assert_eq!(
        (layer.transform.translate_x, layer.transform.translate_y),
        (2.0, 1.0)
    );
    assert!(matches!(
        layer.content,
        LayerContent::Pixel {
            width: 4,
            height: 3,
            ..
        }
    ));
    assert!(has(&imported.report, ReportKind::Note, "data window"));
}

#[test]
fn only_the_first_part_of_a_multipart_file_is_imported_and_the_rest_is_named() {
    let part = |name: &str, value: f32| {
        TestFile::new(3, 3)
            .named(name)
            .f32_channel("R", move |_, _| value)
            .f32_channel("G", move |_, _| value)
            .f32_channel("B", move |_, _| value)
    };
    let bytes = multipart(&[part("beauty", 0.2), part("shadow", 0.7)]);
    let imported = import_bytes(&bytes).unwrap();
    assert_eq!(imported.project.document.layers.len(), 1);
    assert_eq!(pixel(&imported, 0, 0, 0).red, 0.2);
    assert!(has(&imported.report, ReportKind::NotImported, "shadow"));
}

#[test]
fn every_codec_the_reader_has_opens_the_same_picture() {
    let codecs = [
        Codec::Uncompressed,
        Codec::RLE,
        Codec::ZIP1,
        Codec::ZIP16,
        Codec::PIZ,
        Codec::PXR24,
    ];
    for codec in codecs {
        let file = TestFile::new(40, 36)
            .compression(codec)
            .f32_channel("R", |x, _| x as f32 / 40.0)
            .f32_channel("G", |_, y| y as f32 / 36.0)
            .f32_channel("B", |x, y| ((x + y) % 5) as f32 * 0.25);
        let imported = import_file(&file);
        let p = pixel(&imported, 0, 14, 7);
        assert!(close(p.red, 14.0 / 40.0, 1e-3), "{codec:?}: {}", p.red);
        assert!(close(p.green, 7.0 / 36.0, 1e-3), "{codec:?}: {}", p.green);
        // (14 + 7) % 5 = 1 quarter.
        assert!(close(p.blue, 0.25, 1e-3), "{codec:?}: {}", p.blue);
    }
}

#[test]
fn the_lossy_b44_codecs_open_within_their_precision() {
    for codec in [Codec::B44, Codec::B44A] {
        let file = TestFile::new(32, 32)
            .compression(codec)
            .f16_channel("R", |x, _| x as f32 / 32.0)
            .f16_channel("G", |_, y| y as f32 / 32.0)
            .f16_channel("B", |_, _| 0.5);
        let imported = import_file(&file);
        let p = pixel(&imported, 0, 16, 8);
        assert!(close(p.red, 0.5, 2e-2), "{codec:?}: {}", p.red);
        assert!(close(p.green, 0.25, 2e-2), "{codec:?}: {}", p.green);
        assert!(close(p.blue, 0.5, 2e-2), "{codec:?}: {}", p.blue);
    }
}

#[test]
fn tiled_files_are_read_whole() {
    let file = TestFile::new(10, 7)
        .tiled(4)
        .compression(Codec::PIZ)
        .f32_channel("R", |x, y| (x * 10 + y) as f32)
        .f32_channel("G", |_, _| 0.0)
        .f32_channel("B", |_, _| 0.0);
    let imported = import_file(&file);
    for (x, y) in [(0_u32, 0_u32), (9, 6), (5, 3), (4, 4)] {
        assert_eq!(pixel(&imported, 0, x, y).red, (x * 10 + y) as f32);
    }
}

#[test]
fn codecs_the_reader_lacks_are_refused_by_name_from_the_header() {
    let base = TestFile::new(4, 4)
        .f32_channel("R", |_, _| 0.1)
        .f32_channel("G", |_, _| 0.1)
        .f32_channel("B", |_, _| 0.1)
        .bytes();
    for (code, name) in [(8_u8, "DWAA"), (9, "DWAB"), (10, "HTJ2K"), (11, "HTJ2K")] {
        let mut bytes = base.clone();
        patch_compression(&mut bytes, code);
        let message = error_text(import_bytes(&bytes));
        assert!(message.contains(name), "{code}: {message}");
        assert!(message.contains("cannot read"), "{code}: {message}");
    }
}

fn meta_of(file: &TestFile) -> MetaData {
    MetaData::read_from_buffered(&file.bytes()[..], false).unwrap()
}

fn small_rgb() -> TestFile {
    TestFile::new(4, 4)
        .f32_channel("R", |_, _| 0.1)
        .f32_channel("G", |_, _| 0.1)
        .f32_channel("B", |_, _| 0.1)
}

#[test]
fn deep_data_is_refused_before_anything_is_decoded() {
    let mut meta = meta_of(&small_rgb());
    meta.headers[0].deep = true;
    let message = admit(&meta, &limits()).err().unwrap().to_string();
    assert!(message.contains("deep data"), "{message}");
}

#[test]
fn a_header_that_claims_more_than_the_budget_is_refused_without_a_pixel_read() {
    let mut meta = meta_of(&small_rgb());
    meta.headers[0].layer_size = Vec2(1_000_000, 1_000_000);
    meta.headers[0].shared_attributes.display_window.size = Vec2(1_000_000, 1_000_000);
    assert!(matches!(
        admit(&meta, &limits()),
        Err(AppError::ImageTooLarge { .. })
    ));

    // Inside the dimension limits, but more pixels than one buffer may be.
    let mut meta = meta_of(&small_rgb());
    meta.headers[0].layer_size = Vec2(4_000, 4_000);
    meta.headers[0].shared_attributes.display_window.size = Vec2(4_000, 4_000);
    let tight = ImportLimits {
        store_bytes: 1 << 30,
        entry_bytes: 64 << 20,
        job_bytes: 1 << 30,
    };
    assert!(matches!(
        admit(&meta, &tight),
        Err(AppError::ResourceBudget { .. })
    ));

    // A display window that is huge around a tiny picture is a huge canvas, and is refused.
    let mut meta = meta_of(&small_rgb());
    meta.headers[0].shared_attributes.display_window.size = Vec2(100_000, 100_000);
    assert!(admit(&meta, &limits()).is_err());
}

#[test]
fn pictures_that_would_not_fit_beside_the_first_are_left_out_and_named() {
    let file = TestFile::new(8, 6)
        .f32_channel("R", |_, _| 0.1)
        .f32_channel("G", |_, _| 0.1)
        .f32_channel("B", |_, _| 0.1)
        .f32_channel("one.R", |_, _| 0.2)
        .f32_channel("one.G", |_, _| 0.2)
        .f32_channel("one.B", |_, _| 0.2)
        .f32_channel("two.R", |_, _| 0.3)
        .f32_channel("two.G", |_, _| 0.3)
        .f32_channel("two.B", |_, _| 0.3);
    let directory = tempfile::tempdir().unwrap();
    let path = file.write_to(directory.path(), "t.exr");
    let layer = 8 * 6 * 16;
    let tight = ImportLimits {
        store_bytes: layer * 2 + 10,
        entry_bytes: 1 << 30,
        job_bytes: 1 << 30,
    };
    let imported = import_exr(&path, tight).unwrap();
    assert_eq!(imported.project.document.layers.len(), 2);
    assert!(has(
        &imported.report,
        ReportKind::NotImported,
        "would not fit in memory"
    ));
    assert!(has(&imported.report, ReportKind::NotImported, "two"));
}

#[test]
fn the_number_of_pictures_is_bounded() {
    let mut file = TestFile::new(2, 2)
        .f32_channel("R", |_, _| 0.1)
        .f32_channel("G", |_, _| 0.1)
        .f32_channel("B", |_, _| 0.1);
    for index in 0..20 {
        for channel in ["R", "G", "B"] {
            file = file.f32_channel(&format!("pass{index:02}.{channel}"), |_, _| 0.5);
        }
    }
    let imported = import_file(&file);
    assert_eq!(imported.project.document.layers.len(), MAX_SETS);
    assert!(has(
        &imported.report,
        ReportKind::NotImported,
        "21 pictures"
    ));
}

#[test]
fn values_that_are_not_numbers_are_replaced_and_counted() {
    let file = TestFile::new(2, 1)
        .f32_channel("R", |x, _| if x == 0 { f32::NAN } else { f32::INFINITY })
        .f32_channel("G", |_, _| 0.5)
        .f32_channel("B", |_, _| 0.5);
    let imported = import_file(&file);
    assert_eq!(pixel(&imported, 0, 0, 0).red, 0.0);
    assert_eq!(pixel(&imported, 0, 1, 0).red, 65_504.0);
    assert!(has(&imported.report, ReportKind::Approximated, "2 pixels"));
    assert!(imported.project.linear_pixels[0].1.validate().is_ok());
}

#[test]
fn light_with_no_alpha_is_counted_and_dropped_in_a_real_file() {
    let file = TestFile::new(2, 1)
        .f32_channel("A", |x, _| if x == 0 { 0.0 } else { 1.0 })
        .f32_channel("B", |_, _| 0.5)
        .f32_channel("G", |_, _| 0.5)
        .f32_channel("R", |_, _| 0.5);
    let imported = import_file(&file);
    assert_eq!(pixel(&imported, 0, 0, 0), FloatRgba::TRANSPARENT);
    assert!(has(&imported.report, ReportKind::Approximated, "no alpha"));
}

#[test]
fn an_alpha_channel_that_is_zero_everywhere_is_ignored_and_said() {
    // The Carrots file in the OpenEXR project's own test set is like this: ACES colour, an alpha
    // channel nobody filled. It must open as a picture, not as a clear sheet.
    let file = TestFile::new(3, 2)
        .f32_channel("A", |_, _| 0.0)
        .f32_channel("B", |_, _| 0.3)
        .f32_channel("G", |_, _| 0.2)
        .f32_channel("R", |_, _| 0.1);
    let imported = import_file(&file);
    let p = pixel(&imported, 0, 1, 1);
    assert_eq!((p.red, p.green, p.blue, p.alpha), (0.1, 0.2, 0.3, 1.0));
    assert!(has(
        &imported.report,
        ReportKind::Approximated,
        "alpha was ignored"
    ));
    assert!(!has(&imported.report, ReportKind::Approximated, "no alpha"));
}

#[test]
fn square_pixels_are_assumed_and_a_different_aspect_is_said() {
    let file = small_rgb().pixel_aspect(2.0);
    let imported = import_file(&file);
    assert!(has(
        &imported.report,
        ReportKind::Approximated,
        "not square"
    ));
    let plain = import_file(&small_rgb());
    assert!(!has(&plain.report, ReportKind::Approximated, "not square"));
}

#[test]
fn what_is_not_an_exr_is_refused_and_a_missing_file_is_an_error() {
    assert!(import_bytes(b"this is not an image").is_err());
    assert!(import_bytes(&[]).is_err());
    assert!(import_bytes(&[0x76, 0x2f, 0x31, 0x01]).is_err());
    let directory = tempfile::tempdir().unwrap();
    assert!(import_exr(&directory.path().join("nope.exr"), limits()).is_err());
    assert!(matches!(
        import_exr(directory.path(), limits()),
        Err(AppError::UnsupportedImageFormat)
    ));
}

#[test]
fn every_truncation_and_changed_byte_is_an_error_or_a_picture_never_a_panic() {
    let file = TestFile::new(8, 6)
        .compression(Codec::PIZ)
        .f32_channel("R", |x, y| (x + y) as f32 * 0.1)
        .f32_channel("G", |_, _| 0.25)
        .f32_channel("B", |_, _| 0.5)
        .f32_channel("A", |_, _| 1.0);
    let bytes = file.bytes();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("t.exr");
    let mut completed = 0_u32;
    for length in 0..bytes.len() {
        std::fs::write(&path, &bytes[..length]).unwrap();
        // A truncated file is an error or, for a lucky cut, a picture; nothing else.
        let result = std::panic::catch_unwind(|| import_exr(&path, limits()));
        assert!(result.is_ok(), "panicked on a file cut to {length} bytes");
        completed += 1;
    }
    for index in 0..bytes.len() {
        for value in [0xff_u8, bytes[index] ^ 0x55] {
            let mut changed = bytes.clone();
            changed[index] = value;
            std::fs::write(&path, &changed).unwrap();
            let result = std::panic::catch_unwind(|| import_exr(&path, limits()));
            assert!(
                result.is_ok(),
                "panicked with byte {index} set to {value:#x}"
            );
            completed += 1;
        }
    }
    assert!(completed as usize > bytes.len());
}

#[test]
fn an_unreasonably_large_file_is_refused_before_it_is_read() {
    // Not a real 4 GiB file: the limit is checked against the length the file system reports,
    // which a sparse file can claim.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("huge.exr");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(MAX_EXR_BYTES + 1).unwrap();
    drop(file);
    let message = error_text(import_exr(&path, limits()));
    assert!(message.contains("largest OpenEXR file"), "{message}");
}
