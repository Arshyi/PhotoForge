//! The TIFF reader and writer against files written, and read, by programs that are not
//! PhotoForge.
//!
//! No TIFF file is committed. Point `PHOTOFORGE_TIFF_CORPUS` at a directory made by
//! `make_tiff_corpus.py` (tifffile writes each file and says what the RGBA values must be) and
//! this reads every one, and compares. Without the variable the tests do nothing, and say so.
//!
//! With `PHOTOFORGE_TIFF_DUMP` also set, what PhotoForge writes is left there (a picture of every
//! kind it writes, with and without metadata) so an independent reader can be asked about it.
use photoforge_lib::color::FloatRgba;
use photoforge_lib::pixel::DocumentPrecision;
use photoforge_lib::psd::ImportLimits;
use photoforge_lib::tiff::import_tiff;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Case {
    name: String,
    file: String,
    #[serde(default)]
    height: u32,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    high: bool,
    #[serde(default)]
    tolerance: f64,
    #[serde(default)]
    note: String,
    /// A word the refusal of this file must contain.
    #[serde(default)]
    refused: Option<String>,
}

fn corpus() -> Option<(PathBuf, Vec<Case>)> {
    let root = PathBuf::from(std::env::var_os("PHOTOFORGE_TIFF_CORPUS")?);
    let manifest = std::fs::read_to_string(root.join("manifest.json")).ok()?;
    Some((root, serde_json::from_str(&manifest).ok()?))
}

fn expected(root: &std::path::Path, case: &Case) -> Vec<[f32; 4]> {
    let bytes = std::fs::read(root.join(format!("{}.expect.f32", case.name))).unwrap();
    assert_eq!(
        bytes.len(),
        case.width as usize * case.height as usize * 16,
        "{}",
        case.name
    );
    bytes
        .chunks_exact(16)
        .map(|pixel| {
            let value = |at: usize| {
                f32::from_le_bytes([pixel[at], pixel[at + 1], pixel[at + 2], pixel[at + 3]])
            };
            [value(0), value(4), value(8), value(12)]
        })
        .collect()
}

#[test]
fn every_file_of_the_corpus_reads_as_the_writer_meant() {
    let Some((root, cases)) = corpus() else {
        eprintln!("skipped: set PHOTOFORGE_TIFF_CORPUS to run real-file checks");
        return;
    };
    let mut worst = 0.0_f64;
    for case in &cases {
        let path = root.join(&case.file);
        if let Some(word) = &case.refused {
            let error = import_tiff(&path, ImportLimits::from_resources())
                .err()
                .unwrap_or_else(|| panic!("{}: was read, and should have been refused", case.name));
            assert!(
                error
                    .to_string()
                    .to_lowercase()
                    .contains(&word.to_lowercase()),
                "{}: refused as {error}, which does not name {word}",
                case.name
            );
            continue;
        }
        let imported = import_tiff(&path, ImportLimits::from_resources())
            .unwrap_or_else(|error| panic!("{}: {error}", case.name));
        let want = expected(&root, case);
        let document = &imported.project.document;
        assert_eq!(
            (document.canvas_width, document.canvas_height),
            (case.width, case.height),
            "{}: size",
            case.name
        );
        assert_eq!(
            document.precision == DocumentPrecision::LinearSrgbF32,
            case.high,
            "{}: precision",
            case.name
        );
        let got: Vec<[f32; 4]> = if case.high {
            let image = &imported.project.linear_pixels[0].1;
            image
                .pixels()
                .iter()
                .map(|p: &FloatRgba| [p.red, p.green, p.blue, p.alpha])
                .collect()
        } else {
            let image = &imported.project.pixels[0].1;
            image
                .as_raw()
                .chunks_exact(4)
                .map(|p| {
                    [
                        f32::from(p[0]) / 255.0,
                        f32::from(p[1]) / 255.0,
                        f32::from(p[2]) / 255.0,
                        f32::from(p[3]) / 255.0,
                    ]
                })
                .collect()
        };
        assert_eq!(got.len(), want.len(), "{}", case.name);
        for (index, (g, w)) in got.iter().zip(&want).enumerate() {
            for channel in 0..4 {
                let difference = f64::from((g[channel] - w[channel]).abs());
                worst = worst.max(difference);
                assert!(
                    difference <= case.tolerance,
                    "{}: pixel {index} channel {channel}: read {} where {} was written (tolerance {})",
                    case.name,
                    g[channel],
                    w[channel],
                    case.tolerance
                );
            }
        }
        match case.note.as_str() {
            "pages" => assert!(
                imported
                    .report
                    .entries
                    .iter()
                    .any(|entry| entry.detail.contains("other image")),
                "{}: the second page was not mentioned",
                case.name
            ),
            "metadata" => {
                let metadata = imported.project.source_metadata.as_ref().unwrap();
                assert!(metadata.xmp.is_some() && metadata.iptc.is_some());
                let summary = metadata.summary();
                assert_eq!(summary.make.as_deref(), Some("Acme"));
                assert_eq!(summary.model.as_deref(), Some("Cam-One"));
            }
            _ => {}
        }
    }
    eprintln!("{} files read; worst difference {worst:e}", cases.len());
}

/// What PhotoForge writes, left for another program to read. Needs `PHOTOFORGE_TIFF_DUMP`.
///
/// Each file comes with `<name>.raw` (the samples it must hold, interleaved, 8 bits or 16-bit
/// little-endian) and `<name>.json` (what else it must say).
#[test]
fn what_is_written_is_left_for_an_independent_reader() {
    use photoforge_lib::metadata::Prepared;
    use photoforge_lib::tiff::{write, Compression, Spec};
    use std::io::Cursor;

    let Some(dump) = std::env::var_os("PHOTOFORGE_TIFF_DUMP").map(PathBuf::from) else {
        eprintln!("skipped: set PHOTOFORGE_TIFF_DUMP to leave files for an independent reader");
        return;
    };
    std::fs::create_dir_all(&dump).unwrap();
    let p3 = photoforge_lib::color_management::RgbColorSpace::DisplayP3
        .icc_bytes()
        .unwrap();
    std::fs::write(dump.join("p3.icc"), &p3).unwrap();

    // A block with the camera, the exposure, the lens, a place and a serial number.
    fn entry(tag: u16, kind: u16, count: u32, value: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&kind.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
        bytes
    }
    let mut block = Vec::new();
    block.extend_from_slice(b"II*\0");
    block.extend_from_slice(&8_u32.to_le_bytes());
    // IFD0 at 8: make, model, orientation, Exif and GPS pointers (5 entries, 66 bytes) -> 74.
    block.extend_from_slice(&5_u16.to_le_bytes());
    block.extend_from_slice(&entry(0x010F, 2, 5, 170));
    block.extend_from_slice(&entry(0x0110, 2, 8, 176));
    block.extend_from_slice(&entry(0x0112, 3, 1, 1));
    block.extend_from_slice(&entry(0x8769, 4, 1, 74));
    block.extend_from_slice(&entry(0x8825, 4, 1, 134));
    block.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(block.len(), 74);
    // Exif at 74: exposure, ISO, date, serial, lens (5 entries, 66 bytes) -> 140? (74 + 66 = 140)
    block.extend_from_slice(&5_u16.to_le_bytes());
    block.extend_from_slice(&entry(0x829A, 5, 1, 184));
    block.extend_from_slice(&entry(0x8827, 3, 1, 400));
    block.extend_from_slice(&entry(0x9003, 2, 20, 192));
    block.extend_from_slice(&entry(0xA431, 2, 12, 212));
    block.extend_from_slice(&entry(0xA434, 2, 14, 224));
    block.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(block.len(), 140);
    // GPS at 134 was wrong by design above; fix the pointer now that the layout is known.
    block[8 + 2 + 4 * 12 + 8..8 + 2 + 4 * 12 + 12].copy_from_slice(&140_u32.to_le_bytes());
    // GPS at 140: ref, latitude (2 entries, 30 bytes) -> 170.
    block.extend_from_slice(&2_u16.to_le_bytes());
    block.extend_from_slice(&entry(0x0001, 2, 2, u32::from_le_bytes([b'N', 0, 0, 0])));
    block.extend_from_slice(&entry(0x0002, 5, 3, 238));
    block.extend_from_slice(&0_u32.to_le_bytes());
    assert_eq!(block.len(), 170);
    block.extend_from_slice(b"Acme\0\0"); // 170..176
    block.extend_from_slice(b"Cam-One\0"); // 176..184
    block.extend_from_slice(&1_u32.to_le_bytes()); // 184: 1/250
    block.extend_from_slice(&250_u32.to_le_bytes());
    block.extend_from_slice(b"2026:10:10 14:03:09\0"); // 192..212
    block.extend_from_slice(b"SERIAL-0042\0"); // 212..224
    block.extend_from_slice(b"Lens 35mm f/2\0\0"); // 224..238 (14 bytes + pad)
    block.truncate(238);
    block.extend_from_slice(&[
        48, 0, 0, 0, 1, 0, 0, 0, 51, 0, 0, 0, 1, 0, 0, 0, 7, 0, 0, 0, 1, 0, 0, 0,
    ]);
    let xmp = b"<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Harbour at dusk</dc:title></rdf:Description></rdf:RDF></x:xmpmeta>".to_vec();
    let iptc = vec![0x1C, 2, 120, 0, 6, b'h', b'a', b'r', b'b', b'o', b'u', b'r'];
    let prepared = Prepared {
        exif: Some(block),
        xmp: Some(xmp),
        iptc: Some(iptc),
    };

    struct Case<'a> {
        name: &'a str,
        width: u32,
        height: u32,
        bits: u16,
        alpha: bool,
        compression: Compression,
        icc: Option<&'a [u8]>,
        metadata: bool,
    }
    let cases = [
        Case {
            name: "rgb8_deflate",
            width: 61,
            height: 47,
            bits: 8,
            alpha: false,
            compression: Compression::Deflate,
            icc: None,
            metadata: false,
        },
        Case {
            name: "rgb8_none",
            width: 61,
            height: 47,
            bits: 8,
            alpha: false,
            compression: Compression::None,
            icc: None,
            metadata: false,
        },
        Case {
            name: "rgba8_deflate",
            width: 33,
            height: 29,
            bits: 8,
            alpha: true,
            compression: Compression::Deflate,
            icc: None,
            metadata: false,
        },
        Case {
            name: "rgb16_p3",
            width: 40,
            height: 31,
            bits: 16,
            alpha: false,
            compression: Compression::Deflate,
            icc: Some(&p3),
            metadata: false,
        },
        Case {
            name: "rgba16_none",
            width: 25,
            height: 19,
            bits: 16,
            alpha: true,
            compression: Compression::None,
            icc: None,
            metadata: false,
        },
        Case {
            name: "wide_strips",
            width: 900,
            height: 200,
            bits: 8,
            alpha: false,
            compression: Compression::Deflate,
            icc: None,
            metadata: false,
        },
        Case {
            name: "metadata",
            width: 20,
            height: 14,
            bits: 8,
            alpha: false,
            compression: Compression::Deflate,
            icc: None,
            metadata: true,
        },
    ];
    for case in &cases {
        let samples = if case.alpha { 4 } else { 3 };
        let value = |x: u32, y: u32, c: u32| -> u32 {
            let max = if case.bits == 8 { 255 } else { 65535 };
            (x * 37 + y * 101 + c * 4001 + x * y) % (max + 1)
        };
        let mut raw = Vec::new();
        let mut out = Cursor::new(Vec::new());
        let spec = Spec {
            width: case.width,
            height: case.height,
            bits: case.bits,
            alpha: case.alpha,
            compression: case.compression,
            icc: case.icc,
            metadata: case.metadata.then_some(&prepared),
        };
        write(&mut out, &spec, |y, row| {
            for x in 0..case.width {
                for c in 0..samples {
                    let v = value(x, y, c);
                    if case.bits == 8 {
                        row.push(v as u8);
                        raw.push(v as u8);
                    } else {
                        row.extend_from_slice(&(v as u16).to_le_bytes());
                        raw.extend_from_slice(&(v as u16).to_le_bytes());
                    }
                }
            }
            Ok(())
        })
        .unwrap();
        std::fs::write(dump.join(format!("{}.tif", case.name)), out.into_inner()).unwrap();
        std::fs::write(dump.join(format!("{}.raw", case.name)), &raw).unwrap();
        let account = serde_json::json!({
            "width": case.width, "height": case.height, "bits": case.bits, "samples": samples,
            "alpha": case.alpha, "deflate": matches!(case.compression, Compression::Deflate),
            "icc": case.icc.is_some(), "metadata": case.metadata,
        });
        std::fs::write(
            dump.join(format!("{}.json", case.name)),
            account.to_string(),
        )
        .unwrap();
    }
}
