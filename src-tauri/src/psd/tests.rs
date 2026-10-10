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
        BlendKey::Supported(BlendMode::LinearBurn)
    );
    assert_eq!(
        blend_from_key(b"hMix"),
        BlendKey::Supported(BlendMode::HardMix)
    );
    // The one mode Photoshop has that PhotoForge does not: a dither that cannot be reproduced.
    assert_eq!(blend_from_key(b"diss"), BlendKey::Unsupported("Dissolve"));
    assert_eq!(blend_from_key(b"????"), BlendKey::Unknown);
    // Every one of PhotoForge's twenty-six modes is reachable from some key.
    let mut reached = std::collections::HashSet::new();
    for key in [
        b"norm", b"dark", b"mul ", b"idiv", b"lite", b"scrn", b"div ", b"over", b"sLit", b"hLit",
        b"diff", b"smud", b"hue ", b"sat ", b"colr", b"lum ", b"lbrn", b"lddg", b"dkCl", b"lgCl",
        b"vLit", b"lLit", b"pLit", b"hMix", b"fsub", b"fdiv",
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
    use super::super::import::{import_psd, ImportLimits, Imported, ReportKind};
    use super::super::testing::{build, TestDoc, TestLayer};
    use crate::layers::{BlendMode, Layer, LayerContent};
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
        // Dissolve is the one: its dither is not reproducible.
        let mut layer = flat("Crumbled", (0, 0, 2, 2), [1, 1, 1]);
        layer.blend = *b"diss";
        let (_dir, path) = write("b.psd", &build(&TestDoc::rgb8(2, 2, vec![layer])));
        let imported = import_psd(&path, BIG).unwrap();
        assert_eq!(
            imported.project.document.layers[0].blend_mode,
            BlendMode::Normal
        );
        let approximated = entries(&imported.report, ReportKind::Approximated);
        assert_eq!(approximated.len(), 1);
        assert!(approximated[0].contains("Dissolve"), "{approximated:?}");
        assert!(approximated[0].starts_with("Crumbled:"), "{approximated:?}");
    }

    #[test]
    fn the_blend_modes_photoshop_adds_are_imported_as_themselves() {
        for (key, mode) in [
            (*b"lbrn", BlendMode::LinearBurn),
            (*b"lddg", BlendMode::LinearDodge),
            (*b"dkCl", BlendMode::DarkerColor),
            (*b"lgCl", BlendMode::LighterColor),
            (*b"vLit", BlendMode::VividLight),
            (*b"lLit", BlendMode::LinearLight),
            (*b"pLit", BlendMode::PinLight),
            (*b"hMix", BlendMode::HardMix),
            (*b"fsub", BlendMode::Subtract),
            (*b"fdiv", BlendMode::Divide),
        ] {
            let mut layer = flat("Mixed", (0, 0, 2, 2), [1, 1, 1]);
            layer.blend = key;
            let (_dir, path) = write("m.psd", &build(&TestDoc::rgb8(2, 2, vec![layer])));
            let imported = import_psd(&path, BIG).unwrap();
            assert_eq!(imported.project.document.layers[0].blend_mode, mode);
            assert!(entries(&imported.report, ReportKind::Approximated).is_empty());
        }
    }

    /// The pixels the importer stored for a layer.
    fn stored_pixels<'a>(imported: &'a Imported, layer: &Layer) -> &'a image::RgbaImage {
        let LayerContent::Pixel { pixel_id, .. } = &layer.content else {
            panic!("expected a pixel layer")
        };
        &imported
            .project
            .pixels
            .iter()
            .find(|(id, _)| id == pixel_id)
            .unwrap()
            .1
    }

    fn render(imported: &Imported) -> image::RgbaImage {
        let mut store = crate::layers::LayerPixelStore::default();
        let document = &imported.project.document;
        store
            .reset(document.canvas_width, document.canvas_height)
            .unwrap();
        for (id, image) in &imported.project.pixels {
            store.register_with_id(id, image.clone()).unwrap();
        }
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        crate::layers::render_document(document, &resolved, crate::layers::RenderOptions::default())
            .unwrap()
    }

    /// A layer on the left half of a 4 x 2 canvas that is opaque there and transparent on
    /// its right half.
    fn base_on_the_left() -> TestLayer {
        let mut base = flat("Base", (0, 0, 2, 4), [10, 10, 10]);
        base.channels[3] = (-1, vec![255, 255, 0, 0, 255, 255, 0, 0]);
        base
    }

    #[test]
    fn a_clipped_layer_stays_clipped_with_its_pixels_untouched() {
        let mut clipped = flat("Clipped", (0, 0, 2, 4), [200, 0, 0]);
        clipped.clipped = true;
        let (_dir, path) = write(
            "c.psd",
            &build(&TestDoc::rgb8(4, 2, vec![base_on_the_left(), clipped])),
        );
        let imported = import_psd(&path, BIG).unwrap();
        let layers = &imported.project.document.layers;
        assert!(!layers[0].clipped);
        assert!(layers[1].clipped);
        // Not baked: the layer is opaque across its whole rectangle, as Photoshop stored it.
        let image = stored_pixels(&imported, &layers[1]);
        assert_eq!(image.get_pixel(3, 1)[3], 255);
        // And nothing is reported, because nothing was approximated.
        assert!(entries(&imported.report, ReportKind::Approximated).is_empty());
        assert!(entries(&imported.report, ReportKind::NotImported).is_empty());
        // Drawn, it stops at the base.
        let picture = render(&imported);
        assert_eq!(picture.get_pixel(1, 0).0, [200, 0, 0, 255]);
        assert_eq!(picture.get_pixel(3, 0).0, [0, 0, 0, 0]);
    }

    #[test]
    fn moving_the_base_moves_what_is_clipped_to_it() {
        let mut clipped = flat("Clipped", (0, 0, 2, 4), [200, 0, 0]);
        clipped.clipped = true;
        let (_dir, path) = write(
            "m.psd",
            &build(&TestDoc::rgb8(4, 2, vec![base_on_the_left(), clipped])),
        );
        let mut imported = import_psd(&path, BIG).unwrap();
        // The base is shifted two pixels right; the clipped layer stays where it is in the
        // stack, and is still drawn only where the base is.
        imported.project.document.layers[0].transform.translate_x = 2.0;
        let picture = render(&imported);
        assert_eq!(picture.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(picture.get_pixel(1, 0).0, [0, 0, 0, 0]);
        assert_eq!(picture.get_pixel(2, 0).0, [200, 0, 0, 255]);
        assert_eq!(picture.get_pixel(3, 0).0, [200, 0, 0, 255]);
    }

    #[test]
    fn a_clipped_layer_with_no_base_is_imported_unclipped_and_says_so() {
        let mut clipped = flat("Orphan", (0, 0, 2, 2), [1, 2, 3]);
        clipped.clipped = true;
        let (_dir, path) = write("o.psd", &build(&TestDoc::rgb8(2, 2, vec![clipped])));
        let imported = import_psd(&path, BIG).unwrap();
        assert!(!imported.project.document.layers[0].clipped);
        let approximated = entries(&imported.report, ReportKind::Approximated);
        assert!(
            approximated
                .iter()
                .any(|line| line.starts_with("Orphan:") && line.contains("nothing to clip it to")),
            "{approximated:?}"
        );
    }

    #[test]
    fn layers_clipped_to_a_base_that_was_not_imported_are_unclipped_and_reported() {
        // An empty rectangle with no recognised content imports as nothing.
        let ground = flat("Ground", (0, 0, 2, 2), [9, 9, 9]);
        let nothing = TestLayer::pixel("Gone", (0, 0, 0, 0), Vec::new());
        let mut clipped = flat("Hangs", (0, 0, 2, 2), [1, 2, 3]);
        clipped.clipped = true;
        let (_dir, path) = write(
            "g.psd",
            &build(&TestDoc::rgb8(2, 2, vec![ground, nothing, clipped])),
        );
        let imported = import_psd(&path, BIG).unwrap();
        let document = &imported.project.document;
        let hangs = document.layers.iter().find(|l| l.name == "Hangs").unwrap();
        // Attached to the ground it happens to sit above, it would be wrong; it is not.
        assert!(!hangs.clipped);
        let approximated = entries(&imported.report, ReportKind::Approximated);
        assert!(
            approximated
                .iter()
                .any(|line| line.starts_with("Hangs:") && line.contains("nothing to clip it to")),
            "{approximated:?}"
        );
    }

    /// A layer with no pixels that carries one adjustment block.
    fn adjustment(name: &str, key: &[u8; 4], body: Vec<u8>) -> TestLayer {
        let mut layer = TestLayer::pixel(name, (0, 0, 0, 0), Vec::new());
        layer.extra_blocks.push((*key, body));
        layer
    }

    #[test]
    fn an_invert_layer_becomes_an_adjustment_layer_that_inverts() {
        let ground = flat("Ground", (0, 0, 2, 2), [10, 20, 30]);
        let invert = adjustment("Invert 1", b"nvrt", Vec::new());
        let (_dir, path) = write("i.psd", &build(&TestDoc::rgb8(2, 2, vec![ground, invert])));
        let imported = import_psd(&path, BIG).unwrap();
        let layers = &imported.project.document.layers;
        assert_eq!(layers.len(), 2);
        assert!(matches!(layers[1].content, LayerContent::Adjustment { .. }));
        assert_eq!(layers[1].name, "Invert 1");
        assert!(entries(&imported.report, ReportKind::NotImported).is_empty());
        assert!(entries(&imported.report, ReportKind::Approximated).is_empty());
        assert_eq!(render(&imported).get_pixel(0, 0).0, [245, 235, 225, 255]);
    }

    #[test]
    fn an_adjustment_keeps_its_opacity_visibility_blend_clipping_and_mask() {
        let ground = flat("Ground", (0, 0, 2, 4), [10, 20, 30]);
        let base = flat("Base", (0, 0, 2, 2), [100, 100, 100]);
        let mut invert = adjustment("Invert 1", b"nvrt", Vec::new());
        invert.opacity = 128;
        invert.clipped = true;
        invert.flags |= 0b10; // hidden
        invert.blend = *b"mul ";
        let (_dir, path) = write(
            "k.psd",
            &build(&TestDoc::rgb8(4, 2, vec![ground, base, invert])),
        );
        let imported = import_psd(&path, BIG).unwrap();
        let adjustment = &imported.project.document.layers[2];
        assert!(adjustment.clipped);
        assert!(!adjustment.visible);
        assert_eq!(adjustment.blend_mode, BlendMode::Multiply);
        assert!((adjustment.opacity - 128.0 / 255.0).abs() < 1e-6);
    }

    #[test]
    fn a_curves_layer_is_imported_and_says_it_was_sampled() {
        let ground = flat("Ground", (0, 0, 2, 2), [10, 20, 30]);
        // Composite curve through (0,0), (128,200), (255,255), output first.
        let mut body = vec![0, 0, 1, 0, 0, 0, 1, 0, 3];
        for (output, input) in [(0_u16, 0_u16), (200, 128), (255, 255)] {
            body.extend_from_slice(&output.to_be_bytes());
            body.extend_from_slice(&input.to_be_bytes());
        }
        let curves = adjustment("Curves 1", b"curv", body);
        let (_dir, path) = write("c1.psd", &build(&TestDoc::rgb8(2, 2, vec![ground, curves])));
        let imported = import_psd(&path, BIG).unwrap();
        assert!(matches!(
            imported.project.document.layers[1].content,
            LayerContent::Adjustment { .. }
        ));
        let approximated = entries(&imported.report, ReportKind::Approximated);
        assert!(
            approximated
                .iter()
                .any(|line| line.starts_with("Curves 1:") && line.contains("32 points")),
            "{approximated:?}"
        );
        // The mid-tone was lifted.
        let lifted = render(&imported).get_pixel(0, 0).0;
        assert!(lifted[2] > 30, "{lifted:?}");
    }

    #[test]
    fn an_adjustment_whose_settings_cannot_be_read_is_left_out_with_the_reason() {
        let ground = flat("Ground", (0, 0, 2, 2), [10, 20, 30]);
        let broken = adjustment("Levels 1", b"levl", vec![0, 9, 0, 0]);
        let (_dir, path) = write("b2.psd", &build(&TestDoc::rgb8(2, 2, vec![ground, broken])));
        let imported = import_psd(&path, BIG).unwrap();
        assert_eq!(imported.project.document.layers.len(), 1);
        let left_out = entries(&imported.report, ReportKind::NotImported);
        assert!(
            left_out
                .iter()
                .any(|line| line.starts_with("Levels 1:") && line.contains("version 9")),
            "{left_out:?}"
        );
    }

    #[test]
    fn adjustments_that_are_not_imported_are_still_reported_by_name() {
        let ground = flat("Ground", (0, 0, 2, 2), [10, 20, 30]);
        let hue = adjustment("Hue/Saturation 1", b"hue2", vec![0; 8]);
        let (_dir, path) = write("h.psd", &build(&TestDoc::rgb8(2, 2, vec![ground, hue])));
        let imported = import_psd(&path, BIG).unwrap();
        let left_out = entries(&imported.report, ReportKind::NotImported);
        assert!(
            left_out.iter().any(|line| line.contains("Hue/Saturation")),
            "{left_out:?}"
        );
    }

    #[test]
    fn a_group_fill_opacity_over_blending_layers_is_reported() {
        let ground = flat("Ground", (0, 0, 2, 2), [9, 9, 9]);
        let mut child = flat("Child", (0, 0, 2, 2), [5, 5, 5]);
        child.blend = *b"mul ";
        let mut group = TestLayer::divider("Faded", 1);
        group.extra_blocks.push((*b"iOpa", vec![120, 0, 0, 0]));
        let layers = vec![
            ground.clone(),
            TestLayer::divider("</Layer group>", 3),
            child,
            group,
        ];
        let (_dir, path) = write("fo.psd", &build(&TestDoc::rgb8(2, 2, layers)));
        let imported = import_psd(&path, BIG).unwrap();
        let approximated = entries(&imported.report, ReportKind::Approximated);
        assert!(
            approximated
                .iter()
                .any(|line| line.starts_with("Faded:") && line.contains("fill opacity")),
            "{approximated:?}"
        );
        // A fill opacity over layers that only paint is plain opacity, and says nothing.
        let mut plain = flat("Child", (0, 0, 2, 2), [5, 5, 5]);
        plain.blend = *b"norm";
        let mut group = TestLayer::divider("Faded", 1);
        group.extra_blocks.push((*b"iOpa", vec![120, 0, 0, 0]));
        let layers = vec![
            ground,
            TestLayer::divider("</Layer group>", 3),
            plain,
            group,
        ];
        let (_dir, path) = write("fo2.psd", &build(&TestDoc::rgb8(2, 2, layers)));
        let imported = import_psd(&path, BIG).unwrap();
        assert!(entries(&imported.report, ReportKind::Approximated).is_empty());
    }

    #[test]
    fn a_group_can_be_a_clipping_base() {
        let ground = flat("Ground", (0, 0, 2, 4), [9, 9, 9]);
        let child = base_on_the_left();
        let mut clipped = flat("Tint", (0, 0, 2, 4), [0, 0, 200]);
        clipped.clipped = true;
        let layers = vec![
            ground,
            TestLayer::divider("</Layer group>", 3),
            child,
            TestLayer::divider("Art", 1),
            clipped,
        ];
        let (_dir, path) = write("gb.psd", &build(&TestDoc::rgb8(4, 2, layers)));
        let imported = import_psd(&path, BIG).unwrap();
        let document = &imported.project.document;
        assert_eq!(document.layers.len(), 3);
        assert!(document.layers[2].clipped);
        assert!(entries(&imported.report, ReportKind::Approximated).is_empty());
    }

    #[test]
    fn a_pass_through_group_that_is_a_base_is_imported_isolated_and_says_so() {
        let ground = flat("Ground", (0, 0, 2, 4), [9, 9, 9]);
        let child = flat("Child", (0, 0, 2, 4), [5, 5, 5]);
        let mut through = TestLayer::divider("Through", 1);
        through.divider_blend = Some(*b"pass");
        let mut clipped = flat("Tint", (0, 0, 2, 4), [0, 0, 200]);
        clipped.clipped = true;
        let layers = vec![
            ground,
            TestLayer::divider("</Layer group>", 3),
            child,
            through,
            clipped,
        ];
        let (_dir, path) = write("pt.psd", &build(&TestDoc::rgb8(4, 2, layers)));
        let imported = import_psd(&path, BIG).unwrap();
        let group = &imported.project.document.layers[1];
        assert!(matches!(
            group.content,
            LayerContent::Group { isolated: true, .. }
        ));
        let approximated = entries(&imported.report, ReportKind::Approximated);
        assert!(
            approximated
                .iter()
                .any(|line| line.starts_with("Through:") && line.contains("isolated group")),
            "{approximated:?}"
        );
    }

    #[test]
    fn a_base_that_does_not_blend_its_clipped_layers_as_a_group_is_reported() {
        let mut base = flat("Base", (0, 0, 2, 2), [9, 9, 9]);
        base.extra_blocks.push((*b"clbl", vec![0, 0, 0, 0]));
        let mut clipped = flat("Tint", (0, 0, 2, 2), [0, 0, 200]);
        clipped.clipped = true;
        let (_dir, path) = write(
            "cl.psd",
            &build(&TestDoc::rgb8(
                2,
                2,
                vec![flat("G", (0, 0, 2, 2), [1, 1, 1]), base, clipped],
            )),
        );
        let imported = import_psd(&path, BIG).unwrap();
        let approximated = entries(&imported.report, ReportKind::Approximated);
        assert!(
            approximated
                .iter()
                .any(|line| line.starts_with("Base:")
                    && line.contains("Blend Clipped Layers as Group")),
            "{approximated:?}"
        );
        // The switch being on, the default, says nothing.
        let mut on = flat("Base", (0, 0, 2, 2), [9, 9, 9]);
        on.extra_blocks.push((*b"clbl", vec![1, 0, 0, 0]));
        let mut tint = flat("Tint", (0, 0, 2, 2), [0, 0, 200]);
        tint.clipped = true;
        let (_dir, path) = write(
            "cl2.psd",
            &build(&TestDoc::rgb8(
                2,
                2,
                vec![flat("G", (0, 0, 2, 2), [1, 1, 1]), on, tint],
            )),
        );
        let imported = import_psd(&path, BIG).unwrap();
        assert!(entries(&imported.report, ReportKind::Approximated).is_empty());
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

// ---- the exporter ---------------------------------------------------------------------------------

mod exporter {
    use super::super::export::{export_psd, key_for, key_reads_back, ExportOptions};
    use super::super::import::{import_psd, ImportLimits, Imported, ReportKind};
    use super::super::read_document;
    use crate::color::{FloatImage, FloatRgba};
    use crate::domain::{CurvePoint, CurveSet, EditOperation};
    use crate::layers::{
        render_document_typed, BlendMode, Layer, LayerContent, LayerDocument, LayerMask,
        LayerPixelStore, RenderOptions,
    };
    use crate::mask::{MaskBitmap, MaskSnapshot};
    use crate::operations::structure::new_layer;
    use crate::pixel::DocumentPrecision;
    use image::{Rgba, RgbaImage};
    use std::path::PathBuf;

    const BIG: ImportLimits = ImportLimits {
        store_bytes: 1 << 30,
        entry_bytes: 1 << 29,
        job_bytes: 1 << 31,
    };

    struct World {
        document: LayerDocument,
        store: LayerPixelStore,
    }

    impl World {
        fn eight_bit(width: u32, height: u32) -> Self {
            let mut store = LayerPixelStore::default();
            store.reset(width, height).unwrap();
            Self {
                document: LayerDocument::new(width, height),
                store,
            }
        }

        fn float(width: u32, height: u32) -> Self {
            let mut world = Self::eight_bit(width, height);
            world.document.precision = DocumentPrecision::LinearSrgbF32;
            world
        }

        /// A pixel layer of one colour, `rect` being (x, y, width, height).
        fn solid(&mut self, name: &str, rect: (i32, i32, u32, u32), rgba: [u8; 4]) -> Layer {
            let image = RgbaImage::from_fn(rect.2, rect.3, |x, y| {
                // A gradient across it, so that a misplaced pixel shows.
                Rgba([
                    rgba[0].wrapping_add((x * 7) as u8),
                    rgba[1].wrapping_add((y * 11) as u8),
                    rgba[2],
                    rgba[3],
                ])
            });
            let id = if self.document.precision == DocumentPrecision::LinearSrgbF32 {
                self.store
                    .register_float(FloatImage::from_rgba8(&image).unwrap())
                    .unwrap()
            } else {
                self.store.register(image).unwrap()
            };
            let mut layer = new_layer(
                &format!("L{}", next_id()),
                name,
                LayerContent::Pixel {
                    pixel_id: id,
                    width: rect.2,
                    height: rect.3,
                },
                "t",
            );
            layer.transform.translate_x = rect.0 as f32;
            layer.transform.translate_y = rect.1 as f32;
            layer
        }

        fn export(
            &self,
            name: &str,
            options: &ExportOptions,
        ) -> (
            super::super::export::ExportReport,
            PathBuf,
            tempfile::TempDir,
        ) {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join(name);
            let resolved = self
                .store
                .resolve(&self.document.referenced_pixel_ids(), false)
                .unwrap();
            let report = export_psd(&self.document, &resolved, &path, options).unwrap();
            (report, path, directory)
        }

        fn render(&self) -> RgbaImage {
            let resolved = self
                .store
                .resolve(&self.document.referenced_pixel_ids(), false)
                .unwrap();
            render_document_typed(&self.document, &resolved, RenderOptions::default())
                .unwrap()
                .encoded8()
                .as_ref()
                .clone()
        }
    }

    fn rendered(imported: &Imported) -> RgbaImage {
        let document = &imported.project.document;
        let mut store = LayerPixelStore::default();
        store
            .reset(document.canvas_width, document.canvas_height)
            .unwrap();
        for (id, image) in &imported.project.pixels {
            store.register_with_id(id, image.clone()).unwrap();
        }
        for (id, image) in &imported.project.linear_pixels {
            store
                .register_typed_with_id(id, image.clone().into())
                .unwrap();
        }
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        render_document_typed(document, &resolved, RenderOptions::default())
            .unwrap()
            .encoded8()
            .as_ref()
            .clone()
    }

    fn max_difference(a: &RgbaImage, b: &RgbaImage) -> u8 {
        assert_eq!(a.dimensions(), b.dimensions());
        a.pixels()
            .zip(b.pixels())
            .flat_map(|(p, q)| (0..4).map(move |c| p[c].abs_diff(q[c])))
            .max()
            .unwrap_or(0)
    }

    fn mask(width: u32, height: u32) -> LayerMask {
        let coverage: Vec<u8> = (0..width * height).map(|i| (i * 17 % 256) as u8).collect();
        LayerMask {
            snapshot: MaskSnapshot::encode(
                &MaskBitmap::from_coverage(width, height, coverage).unwrap(),
            ),
            enabled: true,
            inverted: false,
        }
    }

    fn next_id() -> u32 {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }

    fn group(name: &str, children: Vec<Layer>, isolated: bool) -> Layer {
        new_layer(
            &format!("G{}", next_id()),
            name,
            LayerContent::Group { children, isolated },
            "t",
        )
    }

    fn adjustment(name: &str, operation: EditOperation) -> Layer {
        new_layer(
            &format!("A{}", next_id()),
            name,
            LayerContent::Adjustment {
                operation: Box::new(operation),
            },
            "t",
        )
    }

    fn names(layers: &[Layer]) -> Vec<String> {
        layers.iter().map(|layer| layer.name.clone()).collect()
    }

    #[test]
    fn every_blend_mode_has_a_key_that_reads_back_as_itself() {
        let mut keys = std::collections::HashSet::new();
        for mode in BlendMode::ALL {
            assert!(key_reads_back(mode), "{mode:?}");
            assert!(keys.insert(key_for(mode)), "{mode:?} shares a key");
        }
    }

    #[test]
    fn a_layered_document_is_written_and_reads_back_the_same() {
        let mut world = World::eight_bit(8, 6);
        let ground = world.solid("Ground", (0, 0, 8, 6), [40, 60, 80, 255]);
        let mut top = world.solid("Ünï ☃ top", (2, 1, 4, 3), [200, 100, 50, 200]);
        top.opacity = 0.5;
        top.blend_mode = BlendMode::Multiply;
        top.mask = Some(mask(4, 3));
        let mut hidden = world.solid("Hidden", (1, 1, 2, 2), [255, 0, 0, 255]);
        hidden.visible = false;
        hidden.locked = true;
        let mut inner = world.solid("Inner", (0, 0, 3, 3), [10, 200, 30, 255]);
        inner.blend_mode = BlendMode::Screen;
        let mut clipped = world.solid("Clipped", (0, 0, 3, 3), [255, 255, 0, 255]);
        clipped.clipped = true;
        clipped.opacity = 0.75;
        let mut folder = group("Folder", vec![inner, clipped], true);
        folder.opacity = 0.8;
        folder.collapsed = true;
        let through = group("Through", vec![], false);
        world.document.layers = vec![ground, top, hidden, folder, through];
        let (report, path, _dir) = world.export("a.psd", &ExportOptions::default());

        assert_eq!(report.format, "PSD");
        assert_eq!(report.bit_depth, 8);
        assert_eq!(report.layers_written, 5);
        assert_eq!(report.groups_written, 2);
        assert!(report.entries.is_empty(), "{:?}", report.entries);

        let imported = import_psd(&path, BIG).unwrap();
        let back = &imported.project.document;
        assert_eq!(back.precision, DocumentPrecision::LegacySrgb8);
        assert_eq!(
            names(&back.layers),
            ["Ground", "Ünï ☃ top", "Hidden", "Folder", "Through"]
        );
        let top = &back.layers[1];
        assert!((top.opacity - 0.5).abs() < 1.0 / 255.0);
        assert_eq!(top.blend_mode, BlendMode::Multiply);
        assert!(top.mask.is_some());
        assert_eq!(
            (top.transform.translate_x, top.transform.translate_y),
            (2.0, 1.0)
        );
        assert!(!back.layers[2].visible);
        let folder = &back.layers[3];
        assert!(folder.collapsed);
        assert!((folder.opacity - 0.8).abs() < 1.0 / 255.0);
        let LayerContent::Group { children, isolated } = &folder.content else {
            panic!("expected a group")
        };
        assert!(*isolated);
        assert_eq!(names(children), ["Inner", "Clipped"]);
        assert!(children[1].clipped);
        assert!(!children[0].clipped);
        assert_eq!(children[0].blend_mode, BlendMode::Screen);
        assert!(matches!(
            back.layers[4].content,
            LayerContent::Group {
                isolated: false,
                ..
            }
        ));
        // The same picture, to a level: a file holds opacity in 255ths, and 0.5 is not one.
        assert!(max_difference(&world.render(), &rendered(&imported)) <= 1);
    }

    #[test]
    fn the_file_is_laid_out_as_photoshops_are() {
        let mut world = World::eight_bit(4, 4);
        let ground = world.solid("Ground", (0, 0, 4, 4), [1, 2, 3, 255]);
        let child = world.solid("Child", (1, 1, 2, 2), [9, 9, 9, 255]);
        world.document.layers = vec![ground, group("Folder", vec![child], true)];
        let (_report, path, _dir) = world.export("l.psd", &ExportOptions::default());
        let document = read_document(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(
            document.header.version,
            super::super::document::Version::Psd
        );
        assert_eq!((document.header.width, document.header.height), (4, 4));
        assert_eq!(document.header.depth, 8);
        // Ground, the bounding divider, Child, the folder: four records, in that order.
        let records = &document.layers.records;
        assert_eq!(records.len(), 4);
        assert_eq!(records[0].display_name(), "Ground");
        assert_eq!(records[1].display_name(), "</Layer group>");
        assert_eq!(records[2].display_name(), "Child");
        assert_eq!(records[3].display_name(), "Folder");
        // Every record has the channels Photoshop writes: transparency first, then colour.
        let ids: Vec<i16> = records[0].channels.iter().map(|c| c.id).collect();
        assert_eq!(ids, [-1, 0, 1, 2]);
        assert!(document.image_data.is_some());
        assert!(document.resources.icc_profile.is_some());
    }

    /// One layer record as the bytes say it is, found without `read_document`.
    struct RawRecord {
        blend: [u8; 4],
        blocks: Vec<([u8; 4], Vec<u8>)>,
    }

    impl RawRecord {
        fn block(&self, key: &[u8; 4]) -> Option<&Vec<u8>> {
            self.blocks.iter().find(|b| &b.0 == key).map(|b| &b.1)
        }
    }

    /// Walks the layer section of a PSD the way Photoshop does. Every record's extra data must be
    /// used up exactly by its tagged blocks, every block must start with a signature and have a
    /// length that is a multiple of four, and the channel data must fill what the records leave.
    fn raw_records(bytes: &[u8]) -> Vec<RawRecord> {
        let u16_at = |at: usize| u16::from_be_bytes([bytes[at], bytes[at + 1]]) as usize;
        let u32_at = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!(&bytes[..4], b"8BPS");
        assert_eq!(u16_at(4), 1, "this walker reads PSD, not PSB");
        let mut at = 26;
        at += 4 + u32_at(at); // colour mode data
        at += 4 + u32_at(at); // image resources
        at += 4; // length of the layer and mask information
        let info_length = u32_at(at);
        at += 4;
        let info_end = at + info_length;
        let count = i16::from_be_bytes([bytes[at], bytes[at + 1]]).unsigned_abs() as usize;
        at += 2;
        let mut records = Vec::new();
        let mut channel_bytes = 0;
        for _ in 0..count {
            at += 16; // the rectangle
            let channels = u16_at(at);
            at += 2;
            for _ in 0..channels {
                channel_bytes += u32_at(at + 2);
                at += 6;
            }
            assert_eq!(&bytes[at..at + 4], b"8BIM");
            let blend: [u8; 4] = bytes[at + 4..at + 8].try_into().unwrap();
            at += 12; // signature, blend key, opacity, clipping, flags, filler
            let extra_end = at + 4 + u32_at(at);
            at += 4;
            at += 4 + u32_at(at); // mask data
            at += 4 + u32_at(at); // blending ranges
            at += (1 + usize::from(bytes[at])).div_ceil(4) * 4; // the name, padded to four
            let mut blocks = Vec::new();
            while at < extra_end {
                assert!(
                    matches!(&bytes[at..at + 4], b"8BIM" | b"8B64"),
                    "no signature at {at}"
                );
                let key: [u8; 4] = bytes[at + 4..at + 8].try_into().unwrap();
                let length = u32_at(at + 8);
                assert_eq!(
                    length % 4,
                    0,
                    "{} holds {length} bytes",
                    String::from_utf8_lossy(&key)
                );
                blocks.push((key, bytes[at + 12..at + 12 + length].to_vec()));
                at += 12 + length;
            }
            assert_eq!(at, extra_end, "the blocks do not end where the record does");
            records.push(RawRecord { blend, blocks });
        }
        let padding = info_end - at - channel_bytes;
        assert!(
            padding < 4,
            "{padding} bytes unaccounted for after the channels"
        );
        records
    }

    #[test]
    fn every_tagged_block_is_padded_and_every_record_ends_where_it_says() {
        let mut world = World::eight_bit(6, 6);
        // Names of every length modulo four, so that every padding is exercised.
        let ground = world.solid("A", (0, 0, 6, 6), [30, 90, 150, 255]);
        let two = world.solid("Ab", (0, 0, 2, 2), [30, 90, 150, 255]);
        let mut three = world.solid("Abc", (1, 1, 2, 2), [30, 90, 150, 255]);
        three.visible = false;
        three.locked = true;
        let inner = world.solid("Ünï ☃", (0, 0, 3, 3), [10, 200, 30, 255]);
        let mut folder = group("Folder", vec![inner], true);
        folder.blend_mode = BlendMode::Multiply;
        let through = group("Through", vec![], false);
        let mut curves = adjustment(
            "Curves",
            EditOperation::Curves {
                curves: CurveSet {
                    rgb: vec![
                        CurvePoint {
                            input: 0.0,
                            output: 0.0,
                        },
                        CurvePoint {
                            input: 0.5,
                            output: 0.7,
                        },
                        CurvePoint {
                            input: 1.0,
                            output: 1.0,
                        },
                    ],
                    ..CurveSet::default()
                },
            },
        );
        curves.mask = Some(mask(6, 6));
        let levels = adjustment(
            "Levels",
            EditOperation::Levels {
                input_black: 10,
                input_white: 240,
                gamma: 1.2,
                output_black: 5,
                output_white: 250,
            },
        );
        world.document.layers = vec![ground, two, three, folder, through, curves, levels];
        let (_report, path, _dir) = world.export("raw.psd", &ExportOptions::default());
        let records = raw_records(&std::fs::read(&path).unwrap());

        // A, Ab, Abc, divider, Ünï ☃, Folder, divider, Through, Curves, Levels.
        assert_eq!(records.len(), 10);
        for (index, record) in records.iter().enumerate() {
            assert!(
                record.block(b"luni").is_some(),
                "record {index} has no name"
            );
            assert!(record.block(b"lyid").is_some(), "record {index} has no id");
        }
        let sections: Vec<usize> = (0..records.len())
            .filter(|i| records[*i].block(b"lsct").is_some())
            .collect();
        assert_eq!(sections, [3, 5, 6, 7]);
        assert_eq!(records[3].block(b"lsct").unwrap(), &3_u32.to_be_bytes());
        // A group that blends as a unit says so in both places.
        assert_eq!(records[5].blend, key_for(BlendMode::Multiply));
        let folder_section = records[5].block(b"lsct").unwrap();
        assert_eq!(folder_section.len(), 16);
        assert_eq!(&folder_section[8..12], &key_for(BlendMode::Multiply));
        // A pass-through group is 'norm' in the record and 'pass' in the section.
        assert_eq!(records[7].blend, *b"norm");
        assert_eq!(&records[7].block(b"lsct").unwrap()[8..12], b"pass");
        // Locking is kept, and the adjustments carry their own blocks.
        assert!(records[2].block(b"lspf").is_some());
        assert!(records[0].block(b"lspf").is_none());
        assert!(records[8].block(b"curv").is_some());
        assert!(records[9].block(b"levl").is_some());
    }

    #[test]
    fn inverted_and_disabled_masks_come_back_as_they_paint() {
        let mut world = World::eight_bit(6, 6);
        let ground = world.solid("Ground", (0, 0, 6, 6), [5, 5, 5, 255]);
        let mut inverted = world.solid("Inverted", (0, 0, 4, 4), [200, 40, 40, 255]);
        let mut shown = mask(4, 4);
        shown.inverted = true;
        inverted.mask = Some(shown);
        let mut disabled = world.solid("Disabled", (2, 2, 4, 4), [40, 40, 200, 255]);
        let mut off = mask(4, 4);
        off.enabled = false;
        disabled.mask = Some(off);
        world.document.layers = vec![ground, inverted, disabled];
        let (_report, path, _dir) = world.export("m.psd", &ExportOptions::default());
        let imported = import_psd(&path, BIG).unwrap();
        // What the mask does, not how it is stored: the picture is the same.
        assert!(max_difference(&world.render(), &rendered(&imported)) <= 1);
        // The inversion is in the data and not left to the reader, so that it paints the same in
        // a reader that ignores the flag; the disabled mask is still a mask, and still disabled.
        let layers = &imported.project.document.layers;
        let kept = layers[1].mask.as_ref().unwrap();
        assert!(kept.enabled && !kept.inverted);
        assert!(!layers[2].mask.as_ref().unwrap().enabled);
    }

    #[test]
    fn a_layer_partly_off_the_canvas_keeps_its_rectangle() {
        let mut world = World::eight_bit(6, 6);
        let ground = world.solid("Ground", (0, 0, 6, 6), [5, 5, 5, 255]);
        let off = world.solid("Off", (-2, -3, 5, 5), [100, 150, 200, 255]);
        world.document.layers = vec![ground, off];
        let (_report, path, _dir) = world.export("o.psd", &ExportOptions::default());
        let imported = import_psd(&path, BIG).unwrap();
        let layer = &imported.project.document.layers[1];
        assert_eq!(
            (layer.transform.translate_x, layer.transform.translate_y),
            (-2.0, -3.0)
        );
        assert_eq!(layer.pixel_dimensions(), Some((5, 5)));
        assert_eq!(max_difference(&world.render(), &rendered(&imported)), 0);
    }

    #[test]
    fn a_transformed_layer_is_baked_and_reported() {
        let mut world = World::eight_bit(12, 12);
        let ground = world.solid("Ground", (0, 0, 12, 12), [20, 20, 20, 255]);
        let mut turned = world.solid("Turned", (2, 2, 5, 4), [200, 80, 40, 255]);
        turned.transform.scale_x = 1.5;
        turned.transform.rotation_degrees = 20.0;
        turned.mask = Some(mask(5, 4));
        turned.opacity = 0.7;
        world.document.layers = vec![ground, turned];
        let (report, path, _dir) = world.export("t.psd", &ExportOptions::default());
        let approximated: Vec<_> = report
            .entries
            .iter()
            .filter(|e| e.kind == ReportKind::Approximated)
            .collect();
        assert_eq!(approximated.len(), 1);
        assert_eq!(approximated[0].layer.as_deref(), Some("Turned"));
        let imported = import_psd(&path, BIG).unwrap();
        let baked = &imported.project.document.layers[1];
        assert!(baked.transform.integer_translation().is_some());
        assert!((baked.opacity - 0.7).abs() < 1.0 / 255.0);
        // What the transform and the mask did is in the pixels. Drawn again it is the same
        // picture, to a level: the mask was sampled twice.
        let difference = max_difference(&world.render(), &rendered(&imported));
        assert!(difference <= 2, "{difference}");
    }

    #[test]
    fn adjustments_are_written_as_curves_levels_and_invert() {
        let mut world = World::eight_bit(6, 6);
        let ground = world.solid("Ground", (0, 0, 6, 6), [30, 90, 150, 255]);
        let curves = adjustment(
            "Curves",
            EditOperation::Curves {
                curves: CurveSet {
                    rgb: vec![
                        CurvePoint {
                            input: 0.0,
                            output: 0.0,
                        },
                        CurvePoint {
                            input: 0.5,
                            output: 0.7,
                        },
                        CurvePoint {
                            input: 1.0,
                            output: 1.0,
                        },
                    ],
                    ..CurveSet::default()
                },
            },
        );
        let levels = adjustment(
            "Levels",
            EditOperation::Levels {
                input_black: 10,
                input_white: 240,
                gamma: 1.2,
                output_black: 5,
                output_white: 250,
            },
        );
        let invert = adjustment(
            "Invert",
            EditOperation::Curves {
                curves: CurveSet {
                    rgb: vec![
                        CurvePoint {
                            input: 0.0,
                            output: 1.0,
                        },
                        CurvePoint {
                            input: 1.0,
                            output: 0.0,
                        },
                    ],
                    ..CurveSet::default()
                },
            },
        );
        world.document.layers = vec![ground, curves, levels, invert];
        let (report, path, _dir) = world.export("adj.psd", &ExportOptions::default());
        assert_eq!(report.layers_written, 4);
        // Only the three-point curve is not exact.
        let approximated: Vec<_> = report
            .entries
            .iter()
            .filter(|e| e.kind == ReportKind::Approximated)
            .map(|e| e.layer.clone().unwrap_or_default())
            .collect();
        assert_eq!(approximated, ["Curves"]);
        let imported = import_psd(&path, BIG).unwrap();
        let back = &imported.project.document;
        assert!(back.layers[1..]
            .iter()
            .all(|l| matches!(l.content, LayerContent::Adjustment { .. })));
        // Levels with master settings only come back as PhotoForge's Levels, the same numbers.
        let LayerContent::Adjustment { operation } = &back.layers[2].content else {
            panic!()
        };
        assert_eq!(
            **operation,
            EditOperation::Levels {
                input_black: 10,
                input_white: 240,
                gamma: 1.2,
                output_black: 5,
                output_white: 250,
            }
        );
        // The picture comes back to within the smooth curve's difference.
        let difference = max_difference(&world.render(), &rendered(&imported));
        assert!(difference <= 3, "{difference}");
    }

    #[test]
    fn a_per_channel_tone_adjustment_is_written_as_curves() {
        let mut world = World::eight_bit(6, 6);
        let ground = world.solid("Ground", (0, 0, 6, 6), [30, 90, 150, 255]);
        let brightness = adjustment("Lift", EditOperation::Brightness { amount: 0.2 });
        world.document.layers = vec![ground, brightness];
        let (report, path, _dir) = world.export("b.psd", &ExportOptions::default());
        assert_eq!(report.count(ReportKind::NotExported), 0);
        assert_eq!(report.count(ReportKind::Approximated), 1);
        let imported = import_psd(&path, BIG).unwrap();
        assert!(matches!(
            imported.project.document.layers[1].content,
            LayerContent::Adjustment { .. }
        ));
        let difference = max_difference(&world.render(), &rendered(&imported));
        assert!(difference <= 3, "{difference}");
    }

    #[test]
    fn an_adjustment_that_mixes_channels_is_left_out_and_named() {
        let mut world = World::eight_bit(6, 6);
        let ground = world.solid("Ground", (0, 0, 6, 6), [30, 90, 150, 255]);
        let blur = adjustment("Soften", EditOperation::GaussianBlur { radius: 2.0 });
        world.document.layers = vec![ground, blur];
        let (report, path, _dir) = world.export("n.psd", &ExportOptions::default());
        let left_out: Vec<_> = report
            .entries
            .iter()
            .filter(|e| e.kind == ReportKind::NotExported)
            .collect();
        assert_eq!(left_out.len(), 1);
        assert_eq!(left_out[0].layer.as_deref(), Some("Soften"));
        assert!(
            left_out[0].detail.contains("gaussian blur"),
            "{}",
            left_out[0].detail
        );
        assert_eq!(report.layers_written, 1);
        let imported = import_psd(&path, BIG).unwrap();
        assert_eq!(imported.project.document.layers.len(), 1);
    }

    #[test]
    fn edits_over_the_whole_document_are_reported() {
        let mut world = World::eight_bit(4, 4);
        let ground = world.solid("Ground", (0, 0, 4, 4), [30, 90, 150, 255]);
        world.document.layers = vec![ground];
        let options = ExportOptions {
            operations: vec![EditOperation::Brightness { amount: 0.3 }],
            ..ExportOptions::default()
        };
        let (report, path, _dir) = world.export("d.psd", &options);
        let notes: Vec<_> = report
            .entries
            .iter()
            .filter(|e| e.kind == ReportKind::NotExported)
            .collect();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].layer.is_none());
        assert!(notes[0].detail.contains("brightness"));
        // The layers are as they were; the flattened picture has the edit.
        let imported = import_psd(&path, BIG).unwrap();
        assert_eq!(max_difference(&world.render(), &rendered(&imported)), 0);
        let document = read_document(std::fs::File::open(&path).unwrap()).unwrap();
        assert!(document.image_data.is_some());
    }

    #[test]
    fn a_high_precision_document_is_written_as_16_bit_and_comes_back_close() {
        let mut world = World::float(8, 8);
        let ground = world.solid("Ground", (0, 0, 8, 8), [40, 60, 80, 255]);
        let mut top = world.solid("Top", (2, 2, 4, 4), [200, 120, 60, 180]);
        top.blend_mode = BlendMode::Overlay;
        world.document.layers = vec![ground, top];
        let (report, path, _dir) = world.export("h.psd", &ExportOptions::default());
        assert_eq!(report.bit_depth, 16);
        assert!(report.entries.is_empty(), "{:?}", report.entries);
        let document = read_document(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(document.header.depth, 16);
        let imported = import_psd(&path, BIG).unwrap();
        assert_eq!(
            imported.project.document.precision,
            DocumentPrecision::LinearSrgbF32
        );
        assert!(max_difference(&world.render(), &rendered(&imported)) <= 1);
    }

    #[test]
    fn highlights_above_white_are_clipped_at_16_bits_and_kept_at_32() {
        let mut store = LayerPixelStore::default();
        store.reset(2, 2).unwrap();
        let hot = FloatImage::blank(2, 2, FloatRgba::new(3.0, 0.5, 0.25, 1.0)).unwrap();
        let id = store.register_float(hot).unwrap();
        let mut document = LayerDocument::new(2, 2);
        document.precision = DocumentPrecision::LinearSrgbF32;
        document.layers.push(new_layer(
            "hot",
            "Hot",
            LayerContent::Pixel {
                pixel_id: id,
                width: 2,
                height: 2,
            },
            "t",
        ));
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let sixteen = directory.path().join("16.psd");
        // By default a document that holds highlights is written at 32 bits, and says so.
        let by_default = directory.path().join("auto.psd");
        let report =
            export_psd(&document, &resolved, &by_default, &ExportOptions::default()).unwrap();
        assert_eq!(report.bit_depth, 32);
        assert!(report
            .entries
            .iter()
            .any(|e| e.kind == ReportKind::Note && e.detail.contains("32-bit")));
        // Asked for 16, it clips them and says that.
        let options = ExportOptions {
            bit_depth: Some(16),
            ..ExportOptions::default()
        };
        let report = export_psd(&document, &resolved, &sixteen, &options).unwrap();
        assert!(
            report
                .entries
                .iter()
                .any(|e| e.kind == ReportKind::Approximated
                    && e.detail.contains("brighter than white")),
            "{:?}",
            report.entries
        );
        let thirty_two = directory.path().join("32.psd");
        let options = ExportOptions {
            bit_depth: Some(32),
            ..ExportOptions::default()
        };
        let report = export_psd(&document, &resolved, &thirty_two, &options).unwrap();
        assert!(report.entries.is_empty(), "{:?}", report.entries);
        assert_eq!(report.bit_depth, 32);
        let imported = import_psd(&thirty_two, BIG).unwrap();
        let back = rendered_float(&imported);
        assert!(
            (back.pixels()[0].red - 3.0).abs() < 1e-5,
            "{}",
            back.pixels()[0].red
        );
        assert!((back.pixels()[0].green - 0.5).abs() < 1e-5);
    }

    #[test]
    fn the_depth_follows_whether_colour_is_outside_white_and_black_by_more_than_rounding() {
        let depth_for = |pixel: FloatRgba| {
            let mut store = LayerPixelStore::default();
            store.reset(2, 2).unwrap();
            let id = store
                .register_float(FloatImage::blank(2, 2, pixel).unwrap())
                .unwrap();
            let mut document = LayerDocument::new(2, 2);
            document.precision = DocumentPrecision::LinearSrgbF32;
            document.layers.push(new_layer(
                "px",
                "Px",
                LayerContent::Pixel {
                    pixel_id: id,
                    width: 2,
                    height: 2,
                },
                "t",
            ));
            let resolved = store
                .resolve(&document.referenced_pixel_ids(), false)
                .unwrap();
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("d.psd");
            export_psd(&document, &resolved, &path, &ExportOptions::default()).unwrap()
        };
        // White after a profile conversion's rounding is still white: 16 bits, and no report.
        let report = depth_for(FloatRgba::new(1.0003, 0.5, -0.0002, 1.0));
        assert_eq!(report.bit_depth, 16);
        assert!(report.entries.is_empty(), "{:?}", report.entries);
        // Brighter than white by a visible amount, or below black (a colour more saturated than
        // sRGB, in linear sRGB), needs 32 bits to be kept.
        assert_eq!(depth_for(FloatRgba::new(1.05, 0.5, 0.5, 1.0)).bit_depth, 32);
        assert_eq!(
            depth_for(FloatRgba::new(0.5, 0.5, -0.05, 1.0)).bit_depth,
            32
        );
    }

    fn rendered_float(imported: &Imported) -> FloatImage {
        let document = &imported.project.document;
        let mut store = LayerPixelStore::default();
        store
            .reset(document.canvas_width, document.canvas_height)
            .unwrap();
        for (id, image) in &imported.project.pixels {
            store.register_with_id(id, image.clone()).unwrap();
        }
        for (id, image) in &imported.project.linear_pixels {
            store
                .register_typed_with_id(id, image.clone().into())
                .unwrap();
        }
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        render_document_typed(document, &resolved, RenderOptions::default())
            .unwrap()
            .linear()
            .unwrap()
            .as_ref()
            .clone()
    }

    #[test]
    fn a_large_format_file_reads_back_the_same() {
        let mut world = World::eight_bit(8, 6);
        let ground = world.solid("Ground", (0, 0, 8, 6), [40, 60, 80, 255]);
        let mut top = world.solid("Top", (2, 1, 4, 3), [200, 100, 50, 200]);
        top.mask = Some(mask(4, 3));
        world.document.layers = vec![ground, top];
        let options = ExportOptions {
            force_large_format: true,
            ..ExportOptions::default()
        };
        let (report, path, _dir) = world.export("b.psb", &options);
        assert_eq!(report.format, "PSB");
        let document = read_document(std::fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(
            document.header.version,
            super::super::document::Version::Psb
        );
        let imported = import_psd(&path, BIG).unwrap();
        assert_eq!(max_difference(&world.render(), &rendered(&imported)), 0);
        // Sixteen bits as well, which keeps its layers in a block with an eight-byte length.
        let mut float = World::float(8, 6);
        let ground = float.solid("Ground", (0, 0, 8, 6), [40, 60, 80, 255]);
        float.document.layers = vec![ground];
        let (report, path, _dir) = float.export("b16.psb", &options);
        assert_eq!((report.format.as_str(), report.bit_depth), ("PSB", 16));
        let imported = import_psd(&path, BIG).unwrap();
        assert!(max_difference(&float.render(), &rendered(&imported)) <= 1);
    }

    #[test]
    fn a_layer_that_paints_nothing_is_still_there() {
        let mut world = World::eight_bit(4, 4);
        let ground = world.solid("Ground", (0, 0, 4, 4), [30, 90, 150, 255]);
        let mut clear = world.solid("Clear", (0, 0, 4, 4), [0, 0, 0, 0]);
        clear.transform.scale_x = 2.0;
        world.document.layers = vec![ground, clear];
        let (_report, path, _dir) = world.export("c.psd", &ExportOptions::default());
        let imported = import_psd(&path, BIG).unwrap();
        // An empty layer has no pixels in the file; it comes back as nothing to import, and the
        // picture is unchanged.
        assert_eq!(max_difference(&world.render(), &rendered(&imported)), 0);
    }

    #[test]
    fn a_failed_export_leaves_what_was_there() {
        let mut world = World::eight_bit(4, 4);
        let ground = world.solid("Ground", (0, 0, 4, 4), [30, 90, 150, 255]);
        world.document.layers = vec![ground];
        let resolved = world
            .store
            .resolve(&world.document.referenced_pixel_ids(), false)
            .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing-directory").join("x.psd");
        assert!(export_psd(&world.document, &resolved, &path, &ExportOptions::default()).is_err());
        assert!(!path.exists());
        // A depth that is not a Photoshop depth is refused before anything is written.
        let odd = directory.path().join("odd.psd");
        let options = ExportOptions {
            bit_depth: Some(12),
            ..ExportOptions::default()
        };
        assert!(export_psd(&world.document, &resolved, &odd, &options).is_err());
        assert!(!odd.exists());
        // And nothing is left beside a good one.
        let good = directory.path().join("good.psd");
        export_psd(&world.document, &resolved, &good, &ExportOptions::default()).unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, ["good.psd"]);
    }
}
