use super::*;
use crate::openexr::testing::{
    add_header_attributes, attribute, header_len, multipart, patch_compression, patch_tile_size,
    peak_heap, replace_single_block, TestFile,
};
use crate::psd::ReportKind;
use crate::resources::policy::MIN_BUDGET_BYTES;
use exr::math::{RoundingMode, Vec2};
use exr::meta::attribute::{Compression as Codec, LevelMode, TileDescription};

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
fn a_tiny_file_cannot_name_a_block_the_budget_cannot_hold() {
    // The header of a 4 x 4 picture says its tiles are 2^29 pixels square. The reader would set
    // aside the 2 GiB a block may claim before it read a byte of the block, from a file of a few
    // hundred bytes. (The block in the file is real and small; the claim is in the header.)
    let mut bytes = small_rgb().tiled(4).bytes();
    assert!(import_bytes(&bytes).is_ok());
    patch_tile_size(&mut bytes, 1 << 29, 1 << 29);
    let message = error_text(import_bytes(&bytes));
    assert!(message.contains("blocks of up to 2047 MiB"), "{message}");
    assert!(message.contains("1024 MiB"), "{message}");

    // A tile larger than the picture is something writers do, and costs what the picture does.
    assert!(import_bytes(&small_rgb().tiled(64).bytes()).is_ok());
}

#[test]
fn channels_nobody_reads_count_toward_what_a_block_needs() {
    // 64 x 16 pixels, ZIP16 (16 lines to a block, so one block is the picture): red, green and
    // blue and 61 others, all single precision, 64 x 4 = 256 bytes a pixel. The picture is
    // 64 x 16 x 16 = 16 KiB. A block is 256 x 1,024 = 256 KiB, read into a buffer of its own and
    // then decoded: four more of them, a row of 64 pixels and the codecs' tables.
    let mut file = TestFile::new(64, 16)
        .f32_channel("R", |_, _| 0.1)
        .f32_channel("G", |_, _| 0.2)
        .f32_channel("B", |_, _| 0.3);
    for index in 0..61 {
        file = file.f32_channel(&format!("aov{index:02}"), |_, _| 0.5);
    }
    let (picture, block) = (64 * 16 * 16, 256 * 64 * 16);
    let needed = picture + block + 4 * block + 64 * 16 + DECODER_FIXED_BYTES;
    let limits_with = |job_bytes| ImportLimits {
        store_bytes: 1 << 30,
        entry_bytes: 1 << 30,
        job_bytes,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = file.write_to(directory.path(), "t.exr");
    // Exactly enough opens, with the other channels named; one byte less is refused.
    let imported = import_exr(&path, limits_with(needed)).unwrap();
    assert!(has(
        &imported.report,
        ReportKind::NotImported,
        "61 other channels"
    ));
    let message = error_text(import_exr(&path, limits_with(needed - 1)));
    assert!(message.contains("decoding the file's blocks"), "{message}");
    // One block alone may not be more than one buffer may be.
    let narrow = ImportLimits {
        entry_bytes: block - 1,
        ..limits_with(1 << 30)
    };
    let message = error_text(import_exr(&path, narrow));
    assert!(message.contains("blocks of up to"), "{message}");
}

#[test]
fn a_block_the_reader_may_be_sent_to_by_another_part_is_costed_too() {
    // The part a block belongs to is read from the block itself, so a part nobody imports can
    // still be the one whose block is read.
    let parts = multipart(&[small_rgb().named("beauty"), small_rgb().named("other")]);
    let mut meta = MetaData::read_from_buffered(&parts[..], false).unwrap();
    assert!(admit(&meta, &limits()).is_ok());
    meta.headers[1].blocks = BlockDescription::Tiles(TileDescription {
        tile_size: Vec2(1 << 29, 1 << 29),
        level_mode: LevelMode::Singular,
        rounding_mode: RoundingMode::Down,
    });
    assert!(admit(&meta, &limits()).is_err());
    // A deep part is never decoded, so its blocks are not costed.
    meta.headers[1].deep = true;
    assert!(admit(&meta, &limits()).is_ok());
}

#[test]
fn the_block_arithmetic_is_the_readers_and_cannot_overflow() {
    // The largest block the reader will take is its own `max_block_byte_size`.
    for file in [
        TestFile::new(10, 7).compression(Codec::PIZ),
        TestFile::new(10, 7).tiled(4),
        TestFile::new(3, 40).compression(Codec::ZIP1),
    ] {
        let meta = meta_of(
            &file
                .f32_channel("R", |_, _| 0.1)
                .f32_channel("G", |_, _| 0.1)
                .f16_channel("B", |_, _| 0.1),
        );
        let header = &meta.headers[0];
        assert_eq!(
            block_memory(header).buffer,
            header.max_block_byte_size() as u64
        );
    }
    // Every field at its most: saturates, and is refused, instead of wrapping to a small number.
    let mut meta = meta_of(&small_rgb().tiled(4));
    meta.headers[0].channels.bytes_per_pixel = usize::MAX;
    meta.headers[0].blocks = BlockDescription::Tiles(TileDescription {
        tile_size: Vec2(u32::MAX as usize, u32::MAX as usize),
        level_mode: LevelMode::Singular,
        rounding_mode: RoundingMode::Down,
    });
    assert_eq!(block_memory(&meta.headers[0]).peak, u64::MAX);
    assert!(admit(&meta, &limits()).is_err());
}

/// A small file whose first header is `length` bytes long, padded by one unused text attribute.
fn with_header_of(length: usize) -> Vec<u8> {
    let base = small_rgb().bytes();
    // name\0 type\0 size(i32) value
    let fill = length - header_len(&base) - b"pad\0string\0".len() - 4;
    let bytes = add_header_attributes(&base, &attribute(b"pad", b"string", &vec![b'x'; fill]));
    assert_eq!(header_len(&bytes), length);
    bytes
}

#[test]
fn a_header_longer_than_the_cap_is_refused_before_it_is_parsed() {
    let cap = MAX_HEADER_BYTES as usize;
    // A header of exactly the cap is read whole: a long manifest is a legitimate thing to carry.
    let imported = import_bytes(&with_header_of(cap)).expect("a header of the cap opens");
    assert_eq!(imported.report.layers_imported, 1);
    // One byte more and it is refused, naming the header, not as a file that is cut short.
    let message = error_text(import_bytes(&with_header_of(cap + 1)));
    assert!(message.contains("header is longer than"), "{message}");
    assert!(!message.contains("can be read"), "{message}");
}

#[test]
fn a_header_of_many_tiny_attributes_costs_the_cap_and_not_the_file() {
    // 20,000 attributes of ten bytes (a three-byte name, a one-byte type, a size of nought) are a
    // 200 KB file that the reader holds as 16 MB: a table entry of about 340 bytes each, and the
    // table doubling.
    let mut attributes = Vec::new();
    for index in 0..20_000_usize {
        let name = [
            1 + (index / 62_500) as u8,
            1 + (index / 250 % 250) as u8,
            1 + (index % 250) as u8,
        ];
        attributes.extend(attribute(&name, b"q", &[]));
    }
    let bytes = add_header_attributes(&small_rgb().bytes(), &attributes);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("t.exr");
    std::fs::write(&path, &bytes).unwrap();

    let cap = 64 << 10;
    let (uncapped, whole) = peak_heap(|| read_header(&path, u64::MAX));
    assert!(uncapped.is_ok());
    drop(uncapped);
    assert!(whole > 128 * cap, "{whole}");
    let (capped, held) = peak_heap(|| read_header(&path, cap));
    let message = capped.expect_err("cut at the cap").to_string();
    assert!(message.contains("header is longer than"), "{message}");
    assert!(held < 128 * cap, "{held}");
    // What the real cap allows is 127 times itself at the most, measured (66 MB in the first read
    // and 132 MB across an import): half of the smallest budget a machine can be set to is left.
    const { assert!(MAX_HEADER_BYTES * 128 <= MIN_BUDGET_BYTES / 2) };
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
fn a_block_is_decoded_beside_the_pictures_already_read() {
    // Three pictures of 8 x 6 (768 bytes each) in nine single-precision channels: 36 bytes a
    // pixel, and with ZIP16 a block of 16 x 8 pixels is declared and 8 x 6 are there. A block
    // is read as 36 x 128 = 4,608 bytes and decoded as 36 x 48 = 1,728, held four times over,
    // with a row of 8 pixels and the codecs' tables.
    let mut file = TestFile::new(8, 6)
        .f32_channel("R", |_, _| 0.1)
        .f32_channel("G", |_, _| 0.1)
        .f32_channel("B", |_, _| 0.1);
    for prefix in ["one", "two"] {
        for channel in ["R", "G", "B"] {
            file = file.f32_channel(&format!("{prefix}.{channel}"), |_, _| 0.2);
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let path = file.write_to(directory.path(), "t.exr");
    let block = 4_608 + 4 * 1_728 + 8 * 16 + DECODER_FIXED_BYTES;
    // Room for two pictures and a block: the third is left out, though the store holds it.
    let two = ImportLimits {
        store_bytes: 1 << 30,
        entry_bytes: 1 << 30,
        job_bytes: 2 * 768 + block,
    };
    let imported = import_exr(&path, two).unwrap();
    assert_eq!(imported.project.document.layers.len(), 2);
    assert!(has(
        &imported.report,
        ReportKind::NotImported,
        "would not fit in memory"
    ));
    assert!(has(&imported.report, ReportKind::NotImported, "two"));
    // Not even one picture and a block: refused whole.
    let none = ImportLimits {
        job_bytes: 768 + block - 1,
        ..two
    };
    assert!(import_exr(&path, none).is_err());
}

/// A picture of one tile of run-length data, four single-precision channels: 16 bytes a pixel,
/// so the picture and its block are each 65 x 65 x 16 = 67,600 bytes. (65 rather than 64 because
/// a decoded block of 64 times that is then just over a power of two, which is when the buffer
/// that grows by doubling holds the most beside it.)
const TILE: usize = 65;
const BLOCK: u64 = (TILE * TILE * 16) as u64;

fn run_length_tile() -> TestFile {
    TestFile::new(TILE, TILE)
        .compression(Codec::RLE)
        .tiled(TILE)
        .f32_channel("R", |_, _| 0.1)
        .f32_channel("G", |_, _| 0.2)
        .f32_channel("B", |_, _| 0.3)
        .f32_channel("A", |_, _| 1.0)
}

fn limits_of_job(job_bytes: u64) -> ImportLimits {
    ImportLimits {
        store_bytes: 1 << 30,
        entry_bytes: 1 << 30,
        job_bytes,
    }
}

#[test]
fn a_run_length_block_is_costed_at_what_its_runs_can_expand_to() {
    // The block is read into 67,600 bytes and its runs may make 64 times that (a run of 128 bytes
    // is stored as two), in a buffer grown to twice, with the reordering copy: three times 64
    // blocks, beside the pixels of an earlier block, a row of 65 pixels and the codecs' tables.
    let row = TILE as u64 * 16;
    let needed = BLOCK + BLOCK + 3 * 64 * BLOCK + BLOCK + row + DECODER_FIXED_BYTES;
    let directory = tempfile::tempdir().unwrap();
    let path = run_length_tile().write_to(directory.path(), "t.exr");
    assert!(import_exr(&path, limits_of_job(needed)).is_ok());
    let message = error_text(import_exr(&path, limits_of_job(needed - 1)));
    assert!(message.contains("decoding the file's blocks"), "{message}");

    // The same block in a codec that is given the size it may produce is not multiplied.
    let zip = run_length_tile().compression(Codec::ZIP16);
    let path = zip.write_to(directory.path(), "z.exr");
    let needed = BLOCK + BLOCK + 4 * BLOCK + row + DECODER_FIXED_BYTES;
    assert!(import_exr(&path, limits_of_job(needed)).is_ok());
    assert!(import_exr(&path, limits_of_job(needed - 1)).is_err());
}

#[test]
fn runs_that_step_over_the_size_of_a_block_are_held_within_what_was_admitted() {
    // A tile of stored bytes that are a one-byte run and then runs of 128. The reader stops when
    // the output is exactly the size it expects, and these never land on it (they total
    // 1 + 128 k, and a block is an even number of bytes), so it decodes to the end of the stored
    // bytes: 64 times as many, before it finds that they are the wrong size. The file is 67 KB.
    let file = run_length_tile();
    let stored = BLOCK - 2; // not the block's size, or it would be taken as raw pixels
    let mut runs = vec![0x00, 0x00];
    for _ in 1..stored / 2 {
        runs.extend_from_slice(&[0x7F, 0x00]);
    }
    let hostile = replace_single_block(&file.bytes(), &runs);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("t.exr");
    std::fs::write(&path, &hostile).unwrap();

    let admitted = BLOCK + block_memory(&meta_of(&file).headers[0]).peak;
    let (result, held) = peak_heap(|| import_exr(&path, limits()));
    let message = error_text(result);
    assert!(message.contains("decoded"), "{message}");
    // That the runs were expanded, and into more than the two copies (the output and the
    // reordering buffer) a doubling buffer would hold if it were exact: this is the case that
    // needs the three.
    assert!(held > 2 * 64 * stored, "{held} bytes held");
    assert!(held <= admitted, "held {held}, admitted {admitted}");

    // At a budget below what the file can make the reader hold, it is refused before any of it
    // is read.
    let (result, held) = peak_heap(|| import_exr(&path, limits_of_job(admitted - 1)));
    let message = error_text(result);
    assert!(message.contains("decoding the file's blocks"), "{message}");
    assert!(held < stored, "the block was read: {held} bytes held");
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
