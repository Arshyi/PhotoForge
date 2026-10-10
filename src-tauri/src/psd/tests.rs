//! Tests for the PSD reader. The files here are written by `testing.rs`, from the same reading
//! of the specification as the reader: they show the reader is consistent with that reading
//! and that it is bounded, not that real Photoshop output parses.
use super::blend::{blend_from_key, BlendKey};
use super::document::{Divider, Version};
use super::testing::{build, TestDoc, TestLayer};
use super::{
    build_tree, decode_channel, read_document, ChannelSpec, ColorMode, Node, PsdError, Reader,
};
use crate::layers::BlendMode;
use std::io::Cursor;

/// A channel of `width` x `height` samples with runs and noise, so RLE has both kinds of packet.
fn pattern(width: usize, height: usize, depth: u16, seed: u8) -> Vec<u8> {
    let bytes_per = usize::from(depth / 8).max(1);
    let mut out = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let value = if x < width / 2 {
                seed.wrapping_add(y as u8)
            } else {
                seed.wrapping_mul(7).wrapping_add((x * 31 + y * 17) as u8)
            };
            for byte in 0..bytes_per {
                out.push(value.wrapping_add(byte as u8));
            }
        }
    }
    out
}

fn rgb_layer(name: &str, rect: (i32, i32, i32, i32), depth: u16, seed: u8) -> TestLayer {
    let width = (rect.3 - rect.1) as usize;
    let height = (rect.2 - rect.0) as usize;
    TestLayer::pixel(
        name,
        rect,
        vec![
            pattern(width, height, depth, seed),
            pattern(width, height, depth, seed.wrapping_add(1)),
            pattern(width, height, depth, seed.wrapping_add(2)),
        ],
    )
}

fn decode_all(bytes: &[u8]) {
    // Whatever the file says, reading it and decoding every channel returns, with a bounded
    // output, and never panics. Errors are expected and fine.
    let Ok(document) = read_document(Cursor::new(bytes)) else {
        return;
    };
    let Ok(mut reader) = Reader::new(Cursor::new(bytes)) else {
        return;
    };
    for record in &document.layers.records {
        for channel in &record.channels {
            let _ = decode_channel(
                &mut reader,
                document.header.version,
                &ChannelSpec {
                    offset: channel.offset,
                    length: channel.length,
                    width: record.rect.width(),
                    height: record.rect.height(),
                    depth: document.header.depth,
                },
                1 << 20,
            );
        }
    }
}

fn sample_document(version: u16, compression: u16, depth: u16) -> (TestDoc, Vec<u8>) {
    let mut first = rgb_layer("Background", (0, 0, 5, 7), depth, 10);
    first.compression = compression;
    let mut second = rgb_layer("Sky", (1, 2, 4, 6), depth, 80);
    second.compression = compression;
    second.opacity = 128;
    second.blend = *b"mul ";
    second.flags = 0b10; // hidden
    second.unicode_name = Some("Ciel ☁".to_string());
    second.clipped = true;
    second.mask = Some(((1, 2, 4, 6), 255, 0));
    second.layer_id = Some(7);
    let mut doc = TestDoc::rgb8(7, 5, vec![first, second]);
    doc.version = version;
    doc.depth = depth;
    doc.resolution_ppi = Some(300.0);
    doc.icc = Some(vec![1, 2, 3, 4, 5]);
    if depth == 16 {
        doc.layers_in_block = Some(*b"Lr16");
    }
    let bytes = build(&doc);
    (doc, bytes)
}

#[test]
fn reads_the_header_resources_and_layer_records() {
    let (_, bytes) = sample_document(1, 0, 8);
    let document = read_document(Cursor::new(&bytes)).unwrap();
    assert_eq!(document.header.version, Version::Psd);
    assert_eq!(
        (
            document.header.width,
            document.header.height,
            document.header.depth
        ),
        (7, 5, 8)
    );
    assert_eq!(document.header.mode, ColorMode::Rgb);
    assert_eq!(
        document.resources.icc_profile.as_deref(),
        Some(&[1, 2, 3, 4, 5][..])
    );
    let resolution = document.resources.resolution.unwrap();
    assert!((resolution.horizontal_ppi - 300.0).abs() < 0.01);

    let records = &document.layers.records;
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].name, "Background");
    assert!(records[0].visible());
    assert_eq!(records[0].opacity, 255);
    assert_eq!(records[0].rect.width(), 7);
    assert_eq!(records[0].channels.len(), 3);

    let sky = &records[1];
    assert_eq!(sky.name, "Sky");
    assert_eq!(sky.display_name(), "Ciel ☁");
    assert!(!sky.visible());
    assert_eq!(sky.opacity, 128);
    assert_eq!(&sky.blend_key, b"mul ");
    assert!(sky.clipped);
    assert_eq!(sky.layer_id, Some(7));
    let mask = sky.mask.as_ref().unwrap();
    assert_eq!((mask.default_color, mask.flags), (255, 0));
    assert_eq!((mask.rect.width(), mask.rect.height()), (4, 3));
    assert!(sky.has_key(b"luni"));
    assert!(document.image_data.is_some());
}

#[test]
fn every_compression_decodes_to_the_original_samples_at_both_depths_and_versions() {
    for version in [1_u16, 2] {
        for depth in [8_u16, 16] {
            for compression in 0_u16..=3 {
                let (doc, bytes) = sample_document(version, compression, depth);
                let document = read_document(Cursor::new(&bytes))
                    .unwrap_or_else(|error| panic!("v{version} d{depth} c{compression}: {error}"));
                let mut reader = Reader::new(Cursor::new(&bytes)).unwrap();
                for (record, layer) in document.layers.records.iter().zip(&doc.layers) {
                    for (channel, (_, expected)) in record.channels.iter().zip(&layer.channels) {
                        let got = decode_channel(
                            &mut reader,
                            document.header.version,
                            &ChannelSpec {
                                offset: channel.offset,
                                length: channel.length,
                                width: record.rect.width(),
                                height: record.rect.height(),
                                depth,
                            },
                            1 << 20,
                        )
                        .unwrap_or_else(|error| {
                            panic!(
                                "v{version} d{depth} c{compression} {}: {error}",
                                record.name
                            )
                        });
                        assert_eq!(
                            &got, expected,
                            "v{version} d{depth} c{compression} {}",
                            record.name
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_psb_reads_its_eight_byte_lengths() {
    let (_, bytes) = sample_document(2, 1, 8);
    let document = read_document(Cursor::new(&bytes)).unwrap();
    assert_eq!(document.header.version, Version::Psb);
    assert_eq!(document.layers.records.len(), 2);
}

#[test]
fn sixteen_bit_layers_are_found_in_the_document_level_block() {
    let (_, bytes) = sample_document(1, 2, 16);
    let document = read_document(Cursor::new(&bytes)).unwrap();
    assert_eq!(document.layers.from_block, Some(*b"Lr16"));
    assert_eq!(document.layers.records.len(), 2);
    assert!(document.global_block_keys.contains(b"Lr16"));
}

#[test]
fn groups_become_a_tree_bottom_first() {
    // File order, bottom to top: the group's end marker, its two members, the group's own record,
    // then a layer above it.
    let layers = vec![
        TestLayer::divider("</Layer group>", 3),
        rgb_layer("Leaf", (0, 0, 2, 2), 8, 1),
        rgb_layer("Stem", (0, 0, 2, 2), 8, 2),
        {
            let mut group = TestLayer::divider("Plant", 1);
            group.opacity = 200;
            group.blend = *b"pass";
            group.divider_blend = Some(*b"pass");
            group
        },
        rgb_layer("Top", (0, 0, 2, 2), 8, 3),
    ];
    let doc = TestDoc::rgb8(2, 2, layers);
    let document = read_document(Cursor::new(build(&doc))).unwrap();
    let records = &document.layers.records;
    assert_eq!(records[0].divider, Some(Divider::BoundingSection));
    assert_eq!(records[3].divider, Some(Divider::OpenFolder));
    assert_eq!(records[3].divider_blend_key, Some(*b"pass"));

    let tree = build_tree(records, 512, 16).unwrap();
    assert_eq!(tree.len(), 2);
    match &tree[0] {
        Node::Group { record, children } => {
            assert_eq!(records[*record].name, "Plant");
            assert_eq!(
                children.iter().map(Node::record).collect::<Vec<_>>(),
                vec![1, 2]
            );
        }
        other => panic!("expected a group, got {other:?}"),
    }
    assert_eq!(tree[1], Node::Layer(4));
}

#[test]
fn malformed_and_oversized_trees_are_refused() {
    let stray_end = vec![TestLayer::divider("Group", 1)];
    let doc = TestDoc::rgb8(1, 1, stray_end);
    let document = read_document(Cursor::new(build(&doc))).unwrap();
    assert!(matches!(
        build_tree(&document.layers.records, 512, 16),
        Err(PsdError::Malformed { .. })
    ));

    let unclosed = vec![TestLayer::divider("</Layer group>", 3)];
    let doc = TestDoc::rgb8(1, 1, unclosed);
    let document = read_document(Cursor::new(build(&doc))).unwrap();
    assert!(matches!(
        build_tree(&document.layers.records, 512, 16),
        Err(PsdError::Malformed { .. })
    ));

    // Nested deeper than allowed.
    let mut deep = Vec::new();
    for _ in 0..5 {
        deep.push(TestLayer::divider("</Layer group>", 3));
    }
    for _ in 0..5 {
        deep.push(TestLayer::divider("G", 1));
    }
    let doc = TestDoc::rgb8(1, 1, deep);
    let document = read_document(Cursor::new(build(&doc))).unwrap();
    assert!(build_tree(&document.layers.records, 512, 8).is_ok());
    assert!(matches!(
        build_tree(&document.layers.records, 512, 4),
        Err(PsdError::Limit {
            what: "group nesting depth",
            ..
        })
    ));

    // More layers than allowed.
    let many: Vec<TestLayer> = (0..10)
        .map(|n| rgb_layer(&format!("L{n}"), (0, 0, 1, 1), 8, 0))
        .collect();
    let doc = TestDoc::rgb8(1, 1, many);
    let document = read_document(Cursor::new(build(&doc))).unwrap();
    assert!(matches!(
        build_tree(&document.layers.records, 9, 16),
        Err(PsdError::Limit { what: "layers", .. })
    ));
}

#[test]
fn blend_keys_map_to_the_modes_photoforge_has_and_say_so_for_the_rest() {
    assert_eq!(
        blend_from_key(b"mul "),
        BlendKey::Supported(BlendMode::Multiply)
    );
    assert_eq!(
        blend_from_key(b"idiv"),
        BlendKey::Supported(BlendMode::ColorBurn)
    );
    assert_eq!(
        blend_from_key(b"div "),
        BlendKey::Supported(BlendMode::ColorDodge)
    );
    assert_eq!(
        blend_from_key(b"smud"),
        BlendKey::Supported(BlendMode::Exclusion)
    );
    assert_eq!(blend_from_key(b"pass"), BlendKey::PassThrough);
    assert_eq!(
        blend_from_key(b"lbrn"),
        BlendKey::Unsupported("Linear Burn")
    );
    assert_eq!(blend_from_key(b"hMix"), BlendKey::Unsupported("Hard Mix"));
    assert_eq!(blend_from_key(b"????"), BlendKey::Unknown);
    // Every one of PhotoForge's sixteen modes is reachable from some key.
    let mut reached = std::collections::HashSet::new();
    for key in [
        b"norm", b"dark", b"mul ", b"idiv", b"lite", b"scrn", b"div ", b"over", b"sLit", b"hLit",
        b"diff", b"smud", b"hue ", b"sat ", b"colr", b"lum ",
    ] {
        if let BlendKey::Supported(mode) = blend_from_key(key) {
            reached.insert(mode);
        }
    }
    assert_eq!(reached.len(), BlendMode::ALL.len());
}

// ---- refusals, and what is refused before anything is allocated ----------------------------------

fn header_patch(bytes: &mut [u8], offset: usize, value: &[u8]) {
    bytes[offset..offset + value.len()].copy_from_slice(value);
}

#[test]
fn a_header_that_is_not_a_photoshop_header_is_refused() {
    let (_, good) = sample_document(1, 0, 8);
    let mut wrong_signature = good.clone();
    header_patch(&mut wrong_signature, 0, b"GIF8");
    assert_eq!(
        read_document(Cursor::new(&wrong_signature)).unwrap_err(),
        PsdError::NotPhotoshop
    );

    let mut wrong_version = good.clone();
    header_patch(&mut wrong_version, 4, &3_u16.to_be_bytes());
    assert_eq!(
        read_document(Cursor::new(&wrong_version)).unwrap_err(),
        PsdError::UnsupportedVersion(3)
    );

    // Height is at 14, width at 18, depth at 22, mode at 24, channels at 12.
    let mut too_tall = good.clone();
    header_patch(&mut too_tall, 14, &30_001_u32.to_be_bytes());
    assert!(matches!(
        read_document(Cursor::new(&too_tall)).unwrap_err(),
        PsdError::Limit { what: "height", .. }
    ));

    let mut zero_width = good.clone();
    header_patch(&mut zero_width, 18, &0_u32.to_be_bytes());
    assert!(matches!(
        read_document(Cursor::new(&zero_width)).unwrap_err(),
        PsdError::Limit { what: "width", .. }
    ));

    let mut odd_depth = good.clone();
    header_patch(&mut odd_depth, 22, &12_u16.to_be_bytes());
    assert!(matches!(
        read_document(Cursor::new(&odd_depth)).unwrap_err(),
        PsdError::Unsupported { .. }
    ));

    let mut unknown_mode = good.clone();
    header_patch(&mut unknown_mode, 24, &5_u16.to_be_bytes());
    assert!(matches!(
        read_document(Cursor::new(&unknown_mode)).unwrap_err(),
        PsdError::Unsupported { .. }
    ));

    let mut many_channels = good;
    header_patch(&mut many_channels, 12, &57_u16.to_be_bytes());
    assert!(matches!(
        read_document(Cursor::new(&many_channels)).unwrap_err(),
        PsdError::Limit {
            what: "channels",
            ..
        }
    ));
}

#[test]
fn a_psb_may_be_taller_than_a_psd_may() {
    let (_, mut bytes) = sample_document(2, 0, 8);
    header_patch(&mut bytes, 14, &100_000_u32.to_be_bytes());
    let document = read_document(Cursor::new(&bytes)).unwrap();
    assert_eq!(document.header.height, 100_000);
    header_patch(&mut bytes, 14, &300_001_u32.to_be_bytes());
    assert!(read_document(Cursor::new(&bytes)).is_err());
}

#[test]
fn a_declared_layer_count_the_file_cannot_hold_is_refused_before_any_record_is_made() {
    let (_, good) = sample_document(1, 0, 8);
    // The layer count is the first two bytes inside the layer info, which starts after
    // 26 (header) + 4 + 4 + resources + 4 (section length) + 4 (info length).
    let document = read_document(Cursor::new(&good)).unwrap();
    let resources_len: usize = 4 + document
        .resources
        .blocks
        .iter()
        .map(|block| 12 + block.length as usize + (block.length as usize % 2))
        .sum::<usize>();
    let count_at = 26 + 4 + resources_len + 4 + 4;
    let declared = i16::from_be_bytes([good[count_at], good[count_at + 1]]);
    assert_eq!(declared, 2, "the offset arithmetic found the layer count");

    let mut hold = good.clone();
    header_patch(&mut hold, count_at, &4_000_i16.to_be_bytes());
    assert!(matches!(
        read_document(Cursor::new(&hold)).unwrap_err(),
        PsdError::Truncated { .. }
    ));
    let mut over = good;
    header_patch(&mut over, count_at, &(-30_000_i16).to_be_bytes());
    assert!(matches!(
        read_document(Cursor::new(&over)).unwrap_err(),
        PsdError::Limit {
            what: "layer records",
            ..
        }
    ));
}

#[test]
fn declared_dimensions_that_would_overflow_or_exceed_the_ceiling_are_refused_unread() {
    let (_, bytes) = sample_document(1, 0, 8);
    let document = read_document(Cursor::new(&bytes)).unwrap();
    let channel = &document.layers.records[0].channels[0];
    let mut reader = Reader::new(Cursor::new(&bytes)).unwrap();
    for (width, height) in [(u64::MAX / 2, 4), (1 << 40, 1 << 40), (100_000, 100_000)] {
        let result = decode_channel(
            &mut reader,
            document.header.version,
            &ChannelSpec {
                offset: channel.offset,
                length: channel.length,
                width,
                height,
                depth: 8,
            },
            1 << 20,
        );
        assert!(
            matches!(
                result,
                Err(PsdError::Limit { .. }) | Err(PsdError::Malformed { .. })
            ),
            "{width}x{height}: {result:?}"
        );
    }
}

#[test]
fn rle_that_lies_about_its_rows_is_refused() {
    // A hand-made RLE channel: 2 rows of 4 bytes whose row table claims more than exists.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&1_u16.to_be_bytes()); // RLE
    bytes.extend_from_slice(&200_u16.to_be_bytes());
    bytes.extend_from_slice(&200_u16.to_be_bytes());
    bytes.extend_from_slice(&[3, 1, 2, 3, 4]);
    let length = bytes.len() as u64;
    let mut reader = Reader::new(Cursor::new(bytes)).unwrap();
    let result = decode_channel(
        &mut reader,
        Version::Psd,
        &ChannelSpec {
            offset: 0,
            length,
            width: 4,
            height: 2,
            depth: 8,
        },
        1 << 20,
    );
    assert!(
        matches!(result, Err(PsdError::Malformed { .. })),
        "{result:?}"
    );

    // A PackBits row that unpacks beyond its width.
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&2_u16.to_be_bytes()); // one row, two packed bytes
    bytes.extend_from_slice(&[(1 - 9_i16) as i8 as u8, 7]); // repeat 9 times into a 4-wide row
    let length = bytes.len() as u64;
    let mut reader = Reader::new(Cursor::new(bytes)).unwrap();
    let result = decode_channel(
        &mut reader,
        Version::Psd,
        &ChannelSpec {
            offset: 0,
            length,
            width: 4,
            height: 1,
            depth: 8,
        },
        1 << 20,
    );
    assert!(
        matches!(result, Err(PsdError::Malformed { .. })),
        "{result:?}"
    );
}

#[test]
fn a_zip_bomb_is_stopped_at_the_declared_size() {
    // Two megabytes of zeros deflate to a couple of kilobytes. The channel declares 16 bytes.
    let payload = miniz_oxide::deflate::compress_to_vec_zlib(&vec![0_u8; 2 << 20], 9);
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&2_u16.to_be_bytes());
    bytes.extend_from_slice(&payload);
    let length = bytes.len() as u64;
    let mut reader = Reader::new(Cursor::new(bytes)).unwrap();
    let result = decode_channel(
        &mut reader,
        Version::Psd,
        &ChannelSpec {
            offset: 0,
            length,
            width: 4,
            height: 4,
            depth: 8,
        },
        1 << 20,
    );
    assert!(
        matches!(result, Err(PsdError::Malformed { .. })),
        "{result:?}"
    );
}

#[test]
fn zip_prediction_at_32_bits_round_trips() {
    // A float channel: 5 x 3 samples, distinct bytes in every position.
    let width = 5_usize;
    let height = 3_usize;
    let mut samples = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let value = (x as f32 + 1.0) * 0.25 + y as f32 * 0.5;
            samples.extend_from_slice(&value.to_be_bytes());
        }
    }
    let bytes = super::testing::encode_channel(&samples, width, height, 32, 3, 1);
    let length = bytes.len() as u64;
    let mut reader = Reader::new(Cursor::new(bytes)).unwrap();
    let decoded = decode_channel(
        &mut reader,
        Version::Psd,
        &ChannelSpec {
            offset: 0,
            length,
            width: width as u64,
            height: height as u64,
            depth: 32,
        },
        1 << 20,
    )
    .unwrap();
    assert_eq!(decoded, samples);
}

// ---- sweeps ---------------------------------------------------------------------------------------

fn rich_document() -> Vec<u8> {
    let mut layers = vec![TestLayer::divider("</Layer group>", 3)];
    let mut a = rgb_layer("A", (0, 0, 3, 5), 8, 5);
    a.compression = 1;
    a.mask = Some(((0, 0, 3, 5), 255, 0));
    a.unicode_name = Some("Aé".into());
    layers.push(a);
    let mut b = rgb_layer("B", (1, 1, 4, 6), 8, 9);
    b.compression = 2;
    layers.push(b);
    layers.push(TestLayer::divider("Group", 2));
    let mut c = rgb_layer("C", (0, 0, 5, 7), 8, 11);
    c.compression = 3;
    c.blend = *b"over";
    layers.push(c);
    let mut doc = TestDoc::rgb8(7, 5, layers);
    doc.icc = Some((0..40).collect());
    doc.resolution_ppi = Some(72.0);
    build(&doc)
}

#[test]
fn every_prefix_of_a_valid_file_returns_without_panicking() {
    let bytes = rich_document();
    assert!(read_document(Cursor::new(&bytes)).is_ok());
    for end in 0..bytes.len() {
        decode_all(&bytes[..end]);
    }
    // A prefix is refused cleanly: the whole file is needed for the layer section.
    assert!(read_document(Cursor::new(&bytes[..bytes.len() / 2])).is_err());
}

#[test]
fn single_byte_mutations_of_a_valid_file_never_panic() {
    let bytes = rich_document();
    let mut runs = 0_u32;
    let mut accepted = 0_u32;
    for index in 0..bytes.len() {
        for mask in [0xFF_u8, 0x80, 0x01, 0x7F, 0x10] {
            let mut mutated = bytes.clone();
            mutated[index] ^= mask;
            if read_document(Cursor::new(&mutated)).is_ok() {
                accepted += 1;
            }
            decode_all(&mutated);
            runs += 1;
        }
    }
    // The sweep is a real one, not an empty loop, and some mutations are survivable (pixel
    // bytes), which shows the checks are not simply refusing everything.
    assert!(runs as usize >= bytes.len() * 5);
    assert!(accepted > 0);
    println!("mutation sweep: {runs} mutated files, {accepted} still read");
}

#[test]
fn eight_byte_aligned_garbage_after_a_valid_header_never_panics() {
    // Deterministic pseudo-random bodies behind a valid header, in both versions.
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for round in 0..400 {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"8BPS");
        bytes.extend_from_slice(&(1 + (round % 2) as u16).to_be_bytes());
        bytes.extend_from_slice(&[0; 6]);
        bytes.extend_from_slice(&3_u16.to_be_bytes());
        bytes.extend_from_slice(&64_u32.to_be_bytes());
        bytes.extend_from_slice(&64_u32.to_be_bytes());
        bytes.extend_from_slice(&8_u16.to_be_bytes());
        bytes.extend_from_slice(&3_u16.to_be_bytes());
        let length = (next() % 600) as usize;
        for _ in 0..length {
            bytes.push(next() as u8);
        }
        decode_all(&bytes);
    }
}

// ---- the importer ---------------------------------------------------------------------------------

mod importer {
    use super::super::import::{import_psd, ImportLimits, ReportKind};
    use super::super::testing::{build, TestDoc, TestLayer};
    use crate::layers::{BlendMode, LayerContent};
    use crate::pixel::DocumentPrecision;
    use std::path::PathBuf;

    const BIG: ImportLimits = ImportLimits {
        store_bytes: 1 << 30,
        entry_bytes: 1 << 29,
        job_bytes: 1 << 31,
    };

    fn write(name: &str, bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        (directory, path)
    }

    /// A layer of one flat colour, fully opaque inside its rectangle.
    fn flat(name: &str, rect: (i32, i32, i32, i32), rgb: [u8; 3]) -> TestLayer {
        let area = ((rect.3 - rect.1) * (rect.2 - rect.0)) as usize;
        TestLayer::pixel(
            name,
            rect,
            vec![
                vec![rgb[0]; area],
                vec![rgb[1]; area],
                vec![rgb[2]; area],
                vec![255; area],
            ],
        )
    }

    fn entries(report: &super::super::import::ImportReport, kind: ReportKind) -> Vec<String> {
        report
            .entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .map(|entry| {
                format!(
                    "{}: {}",
                    entry.layer.clone().unwrap_or_default(),
                    entry.detail
                )
            })
            .collect()
    }

    #[test]
    fn layers_keep_position_opacity_visibility_blend_and_pixels() {
        let base = flat("Base", (0, 0, 6, 8), [10, 20, 30]);
        let mut top = flat("Top", (1, 2, 4, 6), [200, 100, 50]);
        top.opacity = 128;
        top.blend = *b"mul ";
        top.flags = 0b10; // hidden
        let (_dir, path) = write("a.psd", &build(&TestDoc::rgb8(8, 6, vec![base, top])));
        let imported = import_psd(&path, BIG).unwrap();
        let document = &imported.project.document;
        assert_eq!(document.precision, DocumentPrecision::LegacySrgb8);
        assert_eq!(document.layers.len(), 2);
        let top = &document.layers[1];
        assert_eq!(top.name, "Top");
        assert_eq!(
            (top.transform.translate_x, top.transform.translate_y),
            (2.0, 1.0)
        );
        assert!((top.opacity - 128.0 / 255.0).abs() < 1e-6);
        assert_eq!(top.blend_mode, BlendMode::Multiply);
        assert!(!top.visible);
        let LayerContent::Pixel {
            pixel_id,
            width,
            height,
        } = &top.content
        else {
            panic!("expected a pixel layer")
        };
        assert_eq!((*width, *height), (4, 3));
        let (_, image) = imported
            .project
            .pixels
            .iter()
            .find(|(id, _)| id == pixel_id)
            .unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [200, 100, 50, 255]);
        assert!(entries(&imported.report, ReportKind::Approximated).is_empty());
        assert!(entries(&imported.report, ReportKind::NotImported).is_empty());
    }

    #[test]
    fn fill_opacity_multiplies_the_opacity() {
        let mut layer = flat("Glass", (0, 0, 2, 2), [1, 2, 3]);
        layer.opacity = 255;
        layer.extra_blocks.push((*b"iOpa", vec![128, 0, 0, 0]));
        let (_dir, path) = write("f.psd", &build(&TestDoc::rgb8(2, 2, vec![layer])));
        let imported = import_psd(&path, BIG).unwrap();
        let opacity = imported.project.document.layers[0].opacity;
        assert!((opacity - 128.0 / 255.0).abs() < 1e-6, "{opacity}");
    }

    #[test]
    fn groups_keep_their_flags_and_become_a_tree() {
        let mut group = TestLayer::divider("Scene", 2); // a closed folder
        group.opacity = 200;
        group.divider_blend = Some(*b"pass");
        group.blend = *b"pass";
        let layers = vec![
            TestLayer::divider("</Layer group>", 3),
            flat("Leaf", (0, 0, 2, 2), [9, 9, 9]),
            group,
            flat("Above", (0, 0, 2, 2), [5, 5, 5]),
        ];
        let (_dir, path) = write("g.psd", &build(&TestDoc::rgb8(2, 2, layers)));
        let imported = import_psd(&path, BIG).unwrap();
        let document = &imported.project.document;
        assert_eq!(document.layers.len(), 2);
        let group = &document.layers[0];
        assert_eq!(group.name, "Scene");
        assert!(group.collapsed);
        assert!((group.opacity - 200.0 / 255.0).abs() < 1e-6);
        match &group.content {
            LayerContent::Group { children, isolated } => {
                assert!(!isolated, "pass-through is not isolated");
                assert_eq!(children.len(), 1);
                assert_eq!(children[0].name, "Leaf");
            }
            other => panic!("expected a group, got {other:?}"),
        }
        assert_eq!(group.blend_mode, BlendMode::Normal);
        assert_eq!(imported.report.groups_imported, 1);
        assert_eq!(imported.report.layers_imported, 2);
    }

    #[test]
    fn a_layer_mask_is_cut_to_the_layer_grid_with_the_default_outside() {
        // The layer sits at canvas (left 2, top 1), 4 x 4. The mask rectangle is in canvas
        // coordinates: left 3, top 2, 2 x 2, which is (1, 1) to (3, 3) of the layer's own grid.
        let mut layer = flat("Masked", (1, 2, 5, 6), [100, 100, 100]);
        layer.mask = Some(((2, 3, 4, 5), 0, 0));
        layer.channels.push((-2, vec![200; 4]));
        let (_dir, path) = write("m.psd", &build(&TestDoc::rgb8(8, 6, vec![layer])));
        let imported = import_psd(&path, BIG).unwrap();
        let mask = imported.project.document.layers[0].mask.as_ref().unwrap();
        assert!(mask.enabled && !mask.inverted);
        let bitmap = mask.snapshot.decode().unwrap();
        assert_eq!((bitmap.width(), bitmap.height()), (4, 4));
        assert_eq!(bitmap.get(0, 0), 0);
        assert_eq!(bitmap.get(1, 1), 200);
        assert_eq!(bitmap.get(2, 2), 200);
        assert_eq!(bitmap.get(3, 3), 0);
        assert_eq!(bitmap.get(0, 1), 0);
    }

    #[test]
    fn a_disabled_mask_stays_disabled() {
        let mut layer = flat("Masked", (0, 0, 2, 2), [1, 1, 1]);
        layer.mask = Some(((0, 0, 2, 2), 255, 0b10));
        layer.channels.push((-2, vec![0; 4]));
        let (_dir, path) = write("d.psd", &build(&TestDoc::rgb8(2, 2, vec![layer])));
        let imported = import_psd(&path, BIG).unwrap();
        assert!(
            !imported.project.document.layers[0]
                .mask
                .as_ref()
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn a_blend_mode_photoforge_lacks_becomes_normal_and_is_reported() {
        let mut layer = flat("Burnt", (0, 0, 2, 2), [1, 1, 1]);
        layer.blend = *b"lbrn";
        let (_dir, path) = write("b.psd", &build(&TestDoc::rgb8(2, 2, vec![layer])));
        let imported = import_psd(&path, BIG).unwrap();
        assert_eq!(
            imported.project.document.layers[0].blend_mode,
            BlendMode::Normal
        );
        let approximated = entries(&imported.report, ReportKind::Approximated);
        assert_eq!(approximated.len(), 1);
        assert!(approximated[0].contains("Linear Burn"), "{approximated:?}");
        assert!(approximated[0].starts_with("Burnt:"), "{approximated:?}");
    }

    #[test]
    fn a_clipped_layer_is_cut_by_its_base_coverage_and_says_so() {
        // The base is opaque on its left half only; the clipped layer is opaque everywhere.
        let mut base = flat("Base", (0, 0, 2, 4), [10, 10, 10]);
        let alpha = vec![255, 255, 0, 0, 255, 255, 0, 0];
        base.channels[3] = (-1, alpha);
        let mut clipped = flat("Clipped", (0, 0, 2, 4), [200, 0, 0]);
        clipped.clipped = true;
        let (_dir, path) = write("c.psd", &build(&TestDoc::rgb8(4, 2, vec![base, clipped])));
        let imported = import_psd(&path, BIG).unwrap();
        let LayerContent::Pixel { pixel_id, .. } = &imported.project.document.layers[1].content
        else {
            panic!("expected a pixel layer")
        };
        let (_, image) = imported
            .project
            .pixels
            .iter()
            .find(|(id, _)| id == pixel_id)
            .unwrap();
        assert_eq!(image.get_pixel(0, 0)[3], 255);
        assert_eq!(image.get_pixel(1, 0)[3], 255);
        assert_eq!(image.get_pixel(2, 0)[3], 0);
        assert_eq!(image.get_pixel(3, 1)[3], 0);
        let approximated = entries(&imported.report, ReportKind::Approximated);
        assert!(
            approximated
                .iter()
                .any(|line| line.starts_with("Clipped:") && line.contains("baked")),
            "{approximated:?}"
        );
    }

    #[test]
    fn sixteen_bit_files_become_high_precision_documents() {
        let count = 4;
        let mut samples = Vec::new();
        let mut alpha = Vec::new();
        for _ in 0..count {
            samples.extend_from_slice(&0x8000_u16.to_be_bytes());
            alpha.extend_from_slice(&0xFFFF_u16.to_be_bytes());
        }
        let layer = TestLayer::pixel(
            "Deep",
            (0, 0, 2, 2),
            vec![samples.clone(), samples.clone(), samples, alpha],
        );
        let mut doc = TestDoc::rgb8(2, 2, vec![layer]);
        doc.depth = 16;
        doc.layers_in_block = Some(*b"Lr16");
        let (_dir, path) = write("deep.psd", &build(&doc));
        let imported = import_psd(&path, BIG).unwrap();
        assert_eq!(
            imported.project.document.precision,
            DocumentPrecision::LinearSrgbF32
        );
        assert!(imported.project.pixels.is_empty());
        let (_, image) = &imported.project.linear_pixels[0];
        let pixel = image.pixels()[0];
        // 0x8000 / 0xFFFF is 0.50001 encoded, which is 0.2140 in linear light.
        assert!((pixel.red - 0.214).abs() < 0.002, "{}", pixel.red);
        assert!((pixel.alpha - 1.0).abs() < 1e-6);
    }

    #[test]
    fn an_import_that_would_not_fit_is_refused_with_the_numbers() {
        let layer = flat("Large", (0, 0, 64, 64), [1, 2, 3]);
        let (_dir, path) = write("l.psd", &build(&TestDoc::rgb8(64, 64, vec![layer])));
        let error = import_psd(
            &path,
            ImportLimits {
                store_bytes: 1_000,
                entry_bytes: 1 << 20,
                job_bytes: 1 << 30,
            },
        )
        .err()
        .expect("refused");
        assert!(error.to_string().contains("memory"), "{error}");
    }

    #[test]
    fn modes_and_depths_that_are_not_read_are_refused_with_what_to_do() {
        let layer = flat("A", (0, 0, 2, 2), [1, 2, 3]);
        let mut cmyk = TestDoc::rgb8(2, 2, vec![layer]);
        cmyk.mode = 4;
        let (_dir, path) = write("cmyk.psd", &build(&cmyk));
        let error = import_psd(&path, BIG).err().expect("refused").to_string();
        assert!(
            error.contains("CMYK") && error.contains("RGB Color"),
            "{error}"
        );

        let mut bitmap = TestDoc::rgb8(2, 2, Vec::new());
        bitmap.depth = 1;
        bitmap.mode = 0;
        let (_dir, path) = write("bitmap.psd", &build(&bitmap));
        let error = import_psd(&path, BIG).err().expect("refused").to_string();
        assert!(
            error.contains("Bitmap") || error.contains("bits"),
            "{error}"
        );
    }

    #[test]
    fn a_file_with_no_layers_imports_its_flattened_image() {
        let mut doc = TestDoc::rgb8(2, 1, Vec::new());
        // Planar: red plane [10, 20], green [30, 40], blue [50, 60].
        doc.merged = vec![10, 20, 30, 40, 50, 60];
        let (_dir, path) = write("flat.psd", &build(&doc));
        let imported = import_psd(&path, BIG).unwrap();
        assert_eq!(imported.project.document.layers.len(), 1);
        assert_eq!(imported.project.document.layers[0].name, "Background");
        let (_, image) = &imported.project.pixels[0];
        assert_eq!(image.get_pixel(0, 0).0, [10, 30, 50, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [20, 40, 60, 255]);
    }

    #[test]
    fn an_adjustment_layer_is_reported_by_name_and_the_picture_below_is_kept() {
        let base = flat("Photo", (0, 0, 2, 2), [90, 90, 90]);
        let mut adjustment = TestLayer::pixel("Curves 1", (0, 0, 0, 0), Vec::new());
        adjustment.extra_blocks.push((*b"curv", vec![0; 8]));
        let (_dir, path) = write(
            "adj.psd",
            &build(&TestDoc::rgb8(2, 2, vec![base, adjustment])),
        );
        let imported = import_psd(&path, BIG).unwrap();
        assert_eq!(imported.project.document.layers.len(), 1);
        let left_out = entries(&imported.report, ReportKind::NotImported);
        assert_eq!(left_out.len(), 1);
        assert!(
            left_out[0].starts_with("Curves 1:") && left_out[0].contains("Curves"),
            "{left_out:?}"
        );
    }

    #[test]
    fn layer_effects_and_text_are_reported_even_when_pixels_come_across() {
        let mut styled = flat("Styled", (0, 0, 2, 2), [1, 1, 1]);
        styled.extra_blocks.push((*b"lfx2", vec![0; 8]));
        let mut title = flat("Title", (0, 0, 2, 2), [2, 2, 2]);
        title.extra_blocks.push((*b"TySh", vec![0; 8]));
        let (_dir, path) = write("s.psd", &build(&TestDoc::rgb8(2, 2, vec![styled, title])));
        let imported = import_psd(&path, BIG).unwrap();
        let left_out = entries(&imported.report, ReportKind::NotImported);
        assert!(left_out
            .iter()
            .any(|line| line.starts_with("Styled:") && line.contains("Layer styles")));
        let approximated = entries(&imported.report, ReportKind::Approximated);
        assert!(approximated
            .iter()
            .any(|line| line.starts_with("Title:") && line.contains("Text")));
        assert_eq!(imported.project.document.layers.len(), 2);
    }
}
