//! PNG: genuine row-streaming region and reduced decoding.
//!
//! # What this allocates
//!
//! For a non-interlaced PNG, **memory follows the region, not the source.** The
//! `png` crate delivers one row at a time; rows above the region are inflated and
//! dropped, the region's rows are copied into the output, and decoding stops at
//! the region's last row. Peak memory is the region at its native depth plus one
//! source row. Time still runs down to the region's last row, because deflate
//! cannot be entered in the middle — a region near the bottom of a very tall
//! image costs as long as decoding the image.
//!
//! For an **interlaced** PNG the rows arrive in Adam7 pass order, which cannot be
//! cropped as it streams, so the frame is assembled first and the region cut from
//! it. That is a transient whole-frame decode, not region decoding, and the
//! capability reported for interlaced files says so.
//!
//! # Equality with a full decode
//!
//! The decoder is configured as `image` configures its own (`EXPAND`: palette,
//! low bit depth and tRNS become ordinary 8-bit channels, 16-bit stays 16-bit)
//! and returns the same `DynamicImage` variants, so a region is bit-identical to
//! the same rectangle of a full decode. The tests assert exactly that, per
//! layout, against the `image` crate's own decoder.
use super::model::Rect;
use super::pixels::{row_to_encoded_rgba, RegionBuffer, RowLayout};
use super::reduce::AreaReducer;
use crate::color::{srgb_decode, FloatImage};
use crate::color_management::IccRowTransform;
use crate::error::AppError;
use crate::image_processing::high_precision::check_cancel;
use image::DynamicImage;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::OnceLock;

/// How often a long decode looks at the cancel flag, in rows.
const CANCEL_EVERY_ROWS: u32 = 64;

/// The decoder's own allocation ceiling (ancillary chunks, zlib window).
const DECODER_LIMIT_BYTES: usize = 128 * 1024 * 1024;

/// What the header says, plus what the decoder will deliver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngInfo {
    pub width: u32,
    pub height: u32,
    /// The layout rows arrive in after palette and bit-depth expansion.
    pub layout: RowLayout,
    pub interlaced: bool,
    pub icc: Option<Vec<u8>>,
}

type PngReader = png::Reader<BufReader<File>>;

fn map_png_error(error: png::DecodingError) -> AppError {
    match error {
        png::DecodingError::IoError(io) => match io.kind() {
            std::io::ErrorKind::PermissionDenied => AppError::Permission,
            // A file that ends early is a damaged image, not a broken disk.
            std::io::ErrorKind::UnexpectedEof => AppError::CorruptImage,
            _ => AppError::DecodeFailure,
        },
        png::DecodingError::LimitsExceeded => AppError::OutOfMemoryRisk,
        png::DecodingError::Parameter(_) => AppError::DecodeFailure,
        png::DecodingError::Format(_) => AppError::CorruptImage,
    }
}

fn open_reader(path: &Path) -> Result<PngReader, AppError> {
    let file = File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            AppError::Permission
        } else {
            AppError::DecodeFailure
        }
    })?;
    let mut decoder = png::Decoder::new(BufReader::with_capacity(1 << 20, file));
    decoder.set_transformations(png::Transformations::EXPAND);
    decoder.set_limits(png::Limits {
        bytes: DECODER_LIMIT_BYTES,
    });
    decoder.read_info().map_err(map_png_error)
}

fn layout_of(color: png::ColorType, depth: png::BitDepth) -> Result<RowLayout, AppError> {
    use png::{BitDepth::*, ColorType::*};
    Ok(match (color, depth) {
        (Grayscale, Eight) => RowLayout::Gray8,
        (GrayscaleAlpha, Eight) => RowLayout::GrayAlpha8,
        (Rgb, Eight) => RowLayout::Rgb8,
        (Rgba, Eight) => RowLayout::Rgba8,
        (Grayscale, Sixteen) => RowLayout::Gray16,
        (GrayscaleAlpha, Sixteen) => RowLayout::GrayAlpha16,
        (Rgb, Sixteen) => RowLayout::Rgb16,
        (Rgba, Sixteen) => RowLayout::Rgba16,
        // After EXPAND nothing else should be delivered; anything that is is a
        // format PhotoForge does not know how to read.
        _ => return Err(AppError::UnsupportedImageFormat),
    })
}

fn info_of(reader: &PngReader) -> Result<PngInfo, AppError> {
    let (color, depth) = reader.output_color_type();
    let info = reader.info();
    Ok(PngInfo {
        width: info.width,
        height: info.height,
        layout: layout_of(color, depth)?,
        interlaced: info.interlaced,
        icc: info.icc_profile.as_ref().map(|profile| profile.to_vec()),
    })
}

/// Reads only the header chunks.
pub fn read_info(path: &Path) -> Result<PngInfo, AppError> {
    info_of(&open_reader(path)?)
}

/// Rows top to bottom, whether they stream or were assembled first.
enum Rows {
    // Boxed: the reader is large and the other variant is a few words.
    Streaming(Box<PngReader>),
    Assembled {
        data: Vec<u8>,
        line: usize,
        next: usize,
        height: usize,
    },
}

impl Rows {
    fn open(path: &Path) -> Result<(Self, PngInfo), AppError> {
        let mut reader = open_reader(path)?;
        let info = info_of(&reader)?;
        if !info.interlaced {
            return Ok((Self::Streaming(Box::new(reader)), info));
        }
        // Interlaced: assemble the frame, then walk it. This is the transient
        // whole-frame decode the capability report prices.
        let size = reader
            .output_buffer_size()
            .ok_or(AppError::OutOfMemoryRisk)?;
        let mut data = Vec::new();
        data.try_reserve_exact(size)
            .map_err(|_| AppError::OutOfMemoryRisk)?;
        data.resize(size, 0);
        reader.next_frame(&mut data).map_err(map_png_error)?;
        let line = reader
            .output_line_size(info.width)
            .ok_or(AppError::CorruptImage)?;
        Ok((
            Self::Assembled {
                data,
                line,
                next: 0,
                height: info.height as usize,
            },
            info,
        ))
    }

    fn next_row(&mut self) -> Result<Option<&[u8]>, AppError> {
        match self {
            Self::Streaming(reader) => Ok(reader
                .next_row()
                .map_err(map_png_error)?
                .map(|row| row.data())),
            Self::Assembled {
                data,
                line,
                next,
                height,
            } => {
                if *next >= *height {
                    return Ok(None);
                }
                let start = *next * *line;
                *next += 1;
                Ok(data.get(start..start + *line))
            }
        }
    }
}

/// A region of a PNG, in the layout a full decode would have produced.
pub struct RegionImage {
    pub image: DynamicImage,
    pub icc: Option<Vec<u8>>,
}

/// Decodes `rect` of the PNG at `path`.
pub fn decode_region(
    path: &Path,
    rect: Rect,
    cancel: Option<&AtomicBool>,
) -> Result<RegionImage, AppError> {
    let (mut rows, info) = Rows::open(path)?;
    if !rect.is_within(info.width, info.height) {
        return Err(AppError::InvalidOperation(
            "the region lies outside the image".into(),
        ));
    }
    let mut buffer = RegionBuffer::new(info.layout, rect.width, rect.height)?;
    let mut y = 0u32;
    while let Some(row) = rows.next_row()? {
        if y >= rect.y {
            buffer.push(row, rect.x)?;
        }
        y += 1;
        // The last row of the region is the last row needed. Nothing below it
        // is inflated, which is what makes a region from the top of a tall image
        // cheap.
        if u64::from(y) >= rect.bottom() {
            break;
        }
        if y.is_multiple_of(CANCEL_EVERY_ROWS) {
            check_cancel(cancel)?;
        }
    }
    Ok(RegionImage {
        image: buffer.finish()?,
        icc: info.icc,
    })
}

fn srgb_lut8() -> &'static [f32; 256] {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| {
        let mut table = [0.0f32; 256];
        for (value, slot) in table.iter_mut().enumerate() {
            *slot = srgb_decode(value as f32 / 255.0);
        }
        table
    })
}

fn srgb_lut16() -> &'static [f32] {
    static LUT: OnceLock<Vec<f32>> = OnceLock::new();
    LUT.get_or_init(|| {
        (0..65536u32)
            .map(|v| srgb_decode(v as f32 / 65535.0))
            .collect()
    })
}

/// Converts one row to premultiplied linear RGBA, ready for the reducer.
///
/// Tables rather than `powf` per sample: a reduced copy of a six-hundred
/// megapixel source converts nearly two billion samples.
fn row_to_premultiplied_linear(
    layout: RowLayout,
    row: &[u8],
    width: usize,
    icc: Option<&IccRowTransform>,
    encoded: &mut [f32],
    linear: &mut [f32],
) -> Result<(), AppError> {
    if let Some(transform) = icc {
        row_to_encoded_rgba(layout, row, width, encoded);
        transform.apply(encoded, linear)?;
    } else {
        let channels = layout.channels();
        let sixteen = layout.is_16_bit();
        let (lut8, lut16) = (srgb_lut8(), srgb_lut16());
        let sample = |index: usize| -> (f32, f32) {
            // (linear value, alpha-style plain value)
            if sixteen {
                let raw = usize::from(u16::from_be_bytes([row[index * 2], row[index * 2 + 1]]));
                (lut16[raw], raw as f32 / 65535.0)
            } else {
                let raw = usize::from(row[index]);
                (lut8[raw], raw as f32 / 255.0)
            }
        };
        for x in 0..width {
            let base = x * channels;
            let o = &mut linear[x * 4..x * 4 + 4];
            match channels {
                1 => {
                    let v = sample(base).0;
                    o.copy_from_slice(&[v, v, v, 1.0]);
                }
                2 => {
                    let v = sample(base).0;
                    o.copy_from_slice(&[v, v, v, sample(base + 1).1]);
                }
                3 => o.copy_from_slice(&[
                    sample(base).0,
                    sample(base + 1).0,
                    sample(base + 2).0,
                    1.0,
                ]),
                _ => o.copy_from_slice(&[
                    sample(base).0,
                    sample(base + 1).0,
                    sample(base + 2).0,
                    sample(base + 3).1,
                ]),
            }
        }
    }
    for pixel in linear.chunks_exact_mut(4) {
        let alpha = pixel[3];
        pixel[0] *= alpha;
        pixel[1] *= alpha;
        pixel[2] *= alpha;
    }
    Ok(())
}

/// Decodes the whole PNG reduced to `dst_width` x `dst_height`, as linear float.
///
/// The source is never held: rows are converted, folded into the reducer and
/// dropped. For an interlaced file the assembled frame is the exception, which
/// is why its capability is priced differently.
pub fn decode_reduced(
    path: &Path,
    dst_width: u32,
    dst_height: u32,
    cancel: Option<&AtomicBool>,
) -> Result<FloatImage, AppError> {
    let (mut rows, info) = Rows::open(path)?;
    let mut reducer = AreaReducer::new(info.width, info.height, dst_width, dst_height)?;
    let icc = info.icc.as_deref().map(IccRowTransform::new).transpose()?;
    let width = info.width as usize;
    let mut encoded = vec![0.0f32; width * 4];
    let mut linear = vec![0.0f32; width * 4];
    let mut y = 0u32;
    while let Some(row) = rows.next_row()? {
        row_to_premultiplied_linear(
            info.layout,
            row,
            width,
            icc.as_ref(),
            &mut encoded,
            &mut linear,
        )?;
        reducer.push_row(&linear)?;
        y += 1;
        if y.is_multiple_of(CANCEL_EVERY_ROWS) {
            check_cancel(cancel)?;
        }
    }
    reducer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::GenericImageView;
    use std::io::Write;

    /// Writes a PNG of a given colour type and depth with a deterministic pattern.
    fn write_png(
        path: &Path,
        width: u32,
        height: u32,
        color: png::ColorType,
        depth: png::BitDepth,
        palette: Option<Vec<u8>>,
        trns: Option<Vec<u8>>,
    ) {
        let file = File::create(path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
        encoder.set_color(color);
        encoder.set_depth(depth);
        if let Some(palette) = palette {
            encoder.set_palette(palette);
        }
        if let Some(trns) = trns {
            encoder.set_trns(trns);
        }
        let mut writer = encoder.write_header().unwrap();
        let channels = match color {
            png::ColorType::Grayscale | png::ColorType::Indexed => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
        };
        let bits = depth as usize * channels;
        let row_bytes = (width as usize * bits).div_ceil(8);
        let mut data = vec![0u8; row_bytes * height as usize];
        let mut state = 0x1234_5678u32;
        for byte in &mut data {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *byte = (state >> 24) as u8;
        }
        // Indexed images must only reference real palette entries. Only 8-bit
        // indexed is exercised, and the palettes used have four entries.
        if color == png::ColorType::Indexed {
            for byte in &mut data {
                *byte %= 4;
            }
        }
        writer.write_image_data(&data).unwrap();
    }

    fn reference(path: &Path) -> DynamicImage {
        image::ImageReader::open(path)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
    }

    fn same_pixels(a: &DynamicImage, b: &DynamicImage) -> bool {
        a.dimensions() == b.dimensions()
            && std::mem::discriminant(a) == std::mem::discriminant(b)
            && a.as_bytes() == b.as_bytes()
    }

    /// The claim the whole module rests on: a region is exactly the crop of a
    /// full decode, for every pixel layout.
    #[test]
    fn a_region_is_bit_identical_to_the_crop_of_a_full_decode() {
        use png::{BitDepth::*, ColorType::*};
        let dir = tempfile::tempdir().unwrap();
        let rgb_palette: Vec<u8> = vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 200, 200, 10];
        /// One fixture: a name, colour type, depth, palette and transparency.
        type Fixture = (
            &'static str,
            png::ColorType,
            png::BitDepth,
            Option<Vec<u8>>,
            Option<Vec<u8>>,
        );
        let layouts: Vec<Fixture> = vec![
            ("gray8", Grayscale, Eight, None, None),
            ("gray16", Grayscale, Sixteen, None, None),
            ("gray4", Grayscale, Four, None, None),
            ("gray2", Grayscale, Two, None, None),
            ("gray1", Grayscale, One, None, None),
            ("ga8", GrayscaleAlpha, Eight, None, None),
            ("ga16", GrayscaleAlpha, Sixteen, None, None),
            ("rgb8", Rgb, Eight, None, None),
            ("rgb16", Rgb, Sixteen, None, None),
            ("rgba8", Rgba, Eight, None, None),
            ("rgba16", Rgba, Sixteen, None, None),
            ("indexed8", Indexed, Eight, Some(rgb_palette.clone()), None),
            (
                "indexed8_trns",
                Indexed,
                Eight,
                Some(rgb_palette.clone()),
                Some(vec![255, 128, 0, 64]),
            ),
            ("gray8_trns", Grayscale, Eight, None, Some(vec![0, 77])),
            (
                "rgb8_trns",
                Rgb,
                Eight,
                None,
                Some(vec![0, 10, 0, 20, 0, 30]),
            ),
        ];
        for (name, color, depth, palette, trns) in layouts {
            let path = dir.path().join(format!("{name}.png"));
            write_png(&path, 53, 41, color, depth, palette, trns);
            let full = reference(&path);
            for rect in [
                Rect {
                    x: 0,
                    y: 0,
                    width: 53,
                    height: 41,
                },
                Rect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                Rect {
                    x: 52,
                    y: 40,
                    width: 1,
                    height: 1,
                },
                Rect {
                    x: 7,
                    y: 5,
                    width: 20,
                    height: 17,
                },
                Rect {
                    x: 33,
                    y: 0,
                    width: 20,
                    height: 41,
                },
                Rect {
                    x: 0,
                    y: 30,
                    width: 53,
                    height: 11,
                },
            ] {
                let got = decode_region(&path, rect, None).unwrap().image;
                let want = full.crop_imm(rect.x, rect.y, rect.width, rect.height);
                assert!(
                    same_pixels(&got, &want),
                    "{name}: region {rect:?} differs from the crop of a full decode \
                     ({:?} vs {:?})",
                    got.color(),
                    want.color()
                );
            }
        }
    }

    /// A region from the top of the image must not need the data below it. A file
    /// cut off partway decodes a top region and fails a bottom one — which is only
    /// true if decoding genuinely stops at the region's last row.
    #[test]
    fn decoding_stops_at_the_regions_last_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tall.png");
        write_png(
            &path,
            64,
            4000,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            None,
            None,
        );
        let whole = std::fs::read(&path).unwrap();
        // Cut the file in half: the IEND and the second half of the pixels go.
        let cut = dir.path().join("cut.png");
        std::fs::write(&cut, &whole[..whole.len() / 2]).unwrap();

        let top = Rect {
            x: 4,
            y: 10,
            width: 30,
            height: 20,
        };
        let got = decode_region(&cut, top, None).expect("a region above the cut should decode");
        let want = reference(&path).crop_imm(top.x, top.y, top.width, top.height);
        assert!(same_pixels(&got.image, &want));

        let bottom = Rect {
            x: 0,
            y: 3900,
            width: 30,
            height: 50,
        };
        assert!(
            decode_region(&cut, bottom, None).is_err(),
            "a region below the cut decoded from data that is not there"
        );
    }

    /// A header may claim any size. Only the region is allocated, so a tiny region
    /// of a file that claims to be enormous costs a tiny allocation and then fails
    /// on the missing data.
    #[test]
    fn a_huge_claimed_size_costs_only_the_region() {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut crc = !0u32;
            for byte in bytes {
                crc ^= u32::from(*byte);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        (crc >> 1) ^ 0xEDB8_8320
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        }
        fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut body = kind.to_vec();
            body.extend_from_slice(data);
            out.extend_from_slice(&body);
            out.extend_from_slice(&crc32(&body).to_be_bytes());
        }
        let mut file = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&60_000u32.to_be_bytes());
        ihdr.extend_from_slice(&60_000u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 0, 0, 0, 0]); // 8-bit grayscale, no interlace
        chunk(&mut file, b"IHDR", &ihdr);
        // Four bytes of zlib-wrapped nothing: the header is present, the data is not.
        chunk(
            &mut file,
            b"IDAT",
            &[0x78, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
        );
        chunk(&mut file, b"IEND", &[]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bomb.png");
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&file)
            .unwrap();

        // A header-only read reports the claimed size without decoding anything.
        let info = read_info(&path).unwrap();
        assert_eq!((info.width, info.height), (60_000, 60_000));
        // A 10x10 region allocates 100 bytes and then fails on the missing rows.
        let started = std::time::Instant::now();
        let result = decode_region(
            &path,
            Rect {
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            },
            None,
        );
        assert!(
            result.is_err(),
            "decoded a region from a file with no pixel data"
        );
        assert!(
            started.elapsed().as_secs() < 5,
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_region_outside_the_image_is_refused_before_decoding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.png");
        write_png(
            &path,
            20,
            20,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            None,
            None,
        );
        for rect in [
            Rect {
                x: 15,
                y: 0,
                width: 10,
                height: 5,
            },
            Rect {
                x: 0,
                y: 19,
                width: 5,
                height: 2,
            },
            Rect {
                x: u32::MAX,
                y: 0,
                width: 5,
                height: 5,
            },
            Rect {
                x: 0,
                y: 0,
                width: 0,
                height: 5,
            },
        ] {
            assert!(decode_region(&path, rect, None).is_err(), "{rect:?}");
        }
    }

    #[test]
    fn a_non_png_is_refused_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not.png");
        std::fs::write(&path, b"this is not an image").unwrap();
        assert!(read_info(&path).is_err());
        assert!(read_info(&dir.path().join("missing.png")).is_err());
    }

    /// Identity reduction must equal the library's own conversion to linear float,
    /// or a reduced copy and a full open would disagree about colour.
    #[test]
    fn reducing_at_scale_one_matches_the_full_open_colour_pipeline() {
        let dir = tempfile::tempdir().unwrap();
        for (name, color, depth) in [
            ("rgb8", png::ColorType::Rgb, png::BitDepth::Eight),
            ("rgb16", png::ColorType::Rgb, png::BitDepth::Sixteen),
            ("gray8", png::ColorType::Grayscale, png::BitDepth::Eight),
        ] {
            let path = dir.path().join(format!("{name}.png"));
            write_png(&path, 31, 23, color, depth, None, None);
            let want = FloatImage::from_dynamic(&reference(&path)).unwrap();
            let got = decode_reduced(&path, 31, 23, None).unwrap();
            for (a, b) in got.pixels().iter().zip(want.pixels()) {
                assert!(
                    (a.red - b.red).abs() < 1e-6
                        && (a.green - b.green).abs() < 1e-6
                        && (a.blue - b.blue).abs() < 1e-6,
                    "{name}: {a:?} vs {b:?}"
                );
            }
        }
    }

    #[test]
    fn a_reduced_decode_is_the_area_mean_of_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flat.png");
        // A flat mid-grey: any correct reduction is the same flat grey.
        let file = File::create(&path).unwrap();
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), 64, 48);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&vec![128u8; 64 * 48 * 3]).unwrap();
        drop(writer);
        let reduced = decode_reduced(&path, 16, 12, None).unwrap();
        assert_eq!(reduced.dimensions(), (16, 12));
        let linear = srgb_decode(128.0 / 255.0);
        assert!(reduced
            .pixels()
            .iter()
            .all(|p| (p.red - linear).abs() < 1e-5));
    }

    /// A real Adam7 file, generated outside Rust so the encoder and decoder
    /// cannot share a bug. Interlaced rows arrive in pass order, so the frame is
    /// assembled; the region must still equal the crop of a full decode, and the
    /// header must say the file is interlaced so the planner prices it as such.
    #[test]
    fn an_interlaced_png_decodes_to_the_same_region() {
        const ADAM7: &str = "89504e470d0a1a0a0000000d494844520000000d0000000b08020000015cd7a0a0000001ce4944415478da01c3013cfe000000008828400018e808a0104800441410cc3c90005cfc18e42498000c7404508814949c44d8b09400220a04661e24aa3264002e7e08729228b6a668003af20c7e062cc21a6c00063a022844064a4e126c58268e6242b06c66d276920012ae0634b80a56c21678cc2a9ad646bce06adeea96001e220a402c0e62361a84402ea64a4ac8546eea5e9a00110501330f09551919772331992d51bb377900173f0339490b5b531b7d5d339f6753c1717b001d79053f830d618d1d839735a5a155c7ab7d0023b30745bd0f67c71f89d137abdb57cde57f0029ed094bf7116d01218f0b39b11559d31f81002f270b513113733b2395453bb74f5bd9598300031d01142202252705362c0a47311158361a693b257a40328b45419c4a52ad4f65be547acf5991000957031a5c042b61073c660c4d6b135e701c6f7527807a34917f43a28454b38967c48e7cd59393000f9105209606319b0942a00e53a51564aa1e75af2986b43697b945a8be56b9c369cac87edbcd950015cb0726d00837d50b48da1059df176ae4207be92b8cee389df347aef858bffd6bd00280e10797001b05092c0a0a3d0f0d4e14125f1919701e2281232d92283aa32d49b4325ac5376dd63c82e74199883d9dd6c80beb280000000049454e44ae426082";
        let bytes: Vec<u8> = (0..ADAM7.len() / 2)
            .map(|i| u8::from_str_radix(&ADAM7[i * 2..i * 2 + 2], 16).unwrap())
            .collect();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("adam7.png");
        std::fs::write(&path, &bytes).unwrap();

        let info = read_info(&path).unwrap();
        assert!(info.interlaced, "the fixture is not interlaced");
        assert_eq!((info.width, info.height), (13, 11));

        // The expected pixels come from the formula the fixture was built with.
        let expected = |x: u32, y: u32| {
            [
                (x * 17 + y * 3) % 256,
                (x * 5 + y * 29) % 256,
                (x * x + y) % 256,
            ]
        };
        let rect = Rect {
            x: 3,
            y: 2,
            width: 7,
            height: 6,
        };
        let got = decode_region(&path, rect, None).unwrap().image.to_rgb8();
        for dy in 0..rect.height {
            for dx in 0..rect.width {
                let want = expected(rect.x + dx, rect.y + dy);
                let pixel = got.get_pixel(dx, dy).0;
                assert_eq!(
                    [
                        u32::from(pixel[0]),
                        u32::from(pixel[1]),
                        u32::from(pixel[2])
                    ],
                    want,
                    "pixel {},{}",
                    rect.x + dx,
                    rect.y + dy
                );
            }
        }
        // And against the library's own decoder.
        let full = reference(&path);
        assert!(same_pixels(
            &decode_region(&path, rect, None).unwrap().image,
            &full.crop_imm(rect.x, rect.y, rect.width, rect.height)
        ));
    }

    #[test]
    fn cancellation_stops_a_long_decode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("long.png");
        write_png(
            &path,
            32,
            2000,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            None,
            None,
        );
        let cancel = AtomicBool::new(true);
        let result = decode_region(
            &path,
            Rect {
                x: 0,
                y: 1900,
                width: 8,
                height: 8,
            },
            Some(&cancel),
        );
        assert!(
            matches!(result, Err(AppError::RenderCancelled)),
            "{:?}",
            result.err()
        );
    }
}
