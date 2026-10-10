//! Writing a layered document as a Photoshop PSD or PSB, and saying what did not survive.
//!
//! The rule is the importer's: nothing is dropped silently. A layer is written as it is, written
//! *approximately* (and the entry says how), or *left out* (and the entry says why). The report is
//! part of the result.
//!
//! What is written exactly: pixel layers placed by a whole-pixel move (position, pixels,
//! transparency), opacity, visibility, all twenty-six blend modes, groups (isolated and
//! pass-through, open and closed, nested), layer and group masks (enabled or not, inverted ones
//! with the inversion applied to the stored mask), clipping, Unicode names, and the locked flag.
//! The flattened picture stored in the file is PhotoForge's own composite.
//!
//! What is written approximately: a layer with a scale, rotation or flip is *baked* — its
//! transform is applied and the result stored as pixels; shape, text and smart-object layers are
//! rasterised, with their masks resampled where they move with the layer; a group with a transform
//! is flattened; an adjustment layer that is a curve, Levels or a per-channel tone change is stored
//! as Photoshop's Curves or Levels, with the curve reduced to 16 points.
//!
//! What is left out: adjustment layers that mix channels or look at neighbours (blur, sharpen,
//! hue/saturation, white balance, plug-in filters), and the edits that sit over the whole document
//! rather than in a layer, which are in the flattened picture but not in the layers.
//!
//! # Depth and colour
//!
//! A document kept as 8-bit sRGB is written as 8-bit with an sRGB profile. A high-precision
//! document is written as 16-bit sRGB with the profile by default, or as 32-bit float in linear
//! light, with no profile, which is what Photoshop's 32-bit mode is. Highlights above 1.0 do not fit
//! 16 bits and are clipped, which the report says.
//!
//! # Compression
//!
//! 8- and 16-bit channels are PackBits (RLE), which every Photoshop reader accepts; 32-bit channels
//! are stored raw. Files over what the 4-byte lengths of a PSD can hold, or with a side over 30,000
//! pixels, are written as PSB.
//!
//! # What was and was not checked
//!
//! The file is checked by reading it back with this program's own reader and importer, by a walker
//! in the tests that does not share the reader's leniency (signatures, block lengths a multiple of
//! four, record extents), and by an independent parser written outside the program against the
//! record, block and padding layout of files Photoshop wrote. It has not been opened in Photoshop.
use super::blend::BlendKey;
use super::import::{ReportEntry, ReportKind};
use crate::color::{srgb_decode, srgb_encode, FloatImage};
use crate::domain::{CurvePoint, CurveSet, EditOperation};
use crate::error::AppError;
use crate::layers::{
    render_document_typed, BlendMode, Layer, LayerContent, LayerDocument, LayerTransform,
    PixelSource, RenderOptions,
};
use crate::pixel::{DocumentPrecision, PixelBuffer};
use image::RgbaImage;
use serde::Serialize;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

/// How far outside 0..1 (linear light) a sample may be and still count as inside it. The rounding
/// of a profile conversion puts white at 1.0001, which is not highlights: this is under a fifth
/// of an 8-bit level in the encoded value.
const RANGE_TOLERANCE: f32 = 1.0 / 512.0;

/// Longest side of a PSD; beyond it the file is a PSB.
const PSD_MAX_SIDE: u32 = 30_000;
/// A PSD's lengths are 4 bytes. Well short of 4 GiB, so what is estimated and what is written
/// cannot disagree about which side of the line a file falls.
const PSD_MAX_BYTES: u64 = 1 << 31;

/// What the exporter is asked for.
#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// 8, 16 or 32. `None` is the document's own: 8 for 8-bit sRGB, 16 for high precision.
    pub bit_depth: Option<u16>,
    /// Edits that sit over the whole document. They are applied to the flattened picture when
    /// they keep its size, and are reported either way: the layers do not hold them.
    pub operations: Vec<EditOperation>,
    /// Writes a PSB whatever the size. For tests of the format, mostly.
    pub force_large_format: bool,
    /// The photograph's EXIF, XMP and IPTC as the policy left them, written as the file's
    /// image resources (1058, 1060, 1028).
    pub metadata: Option<crate::metadata::Prepared>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    /// "PSD" or "PSB".
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u16,
    pub color_mode: String,
    /// Layers (not groups) written, baked and rasterised ones included.
    pub layers_written: usize,
    pub groups_written: usize,
    pub bytes: u64,
    pub entries: Vec<ReportEntry>,
}

impl ExportReport {
    fn push(&mut self, kind: ReportKind, layer: Option<&str>, detail: impl Into<String>) {
        self.entries.push(ReportEntry {
            kind,
            layer: layer.map(str::to_string),
            detail: detail.into(),
        });
    }

    pub fn count(&self, kind: ReportKind) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .count()
    }
}

fn failure(message: impl Into<String>) -> AppError {
    AppError::PsdFile(message.into())
}

// ---- keys -------------------------------------------------------------------------------------

/// The key Photoshop writes for each mode. The reverse of `blend_from_key`, checked as such.
pub fn key_for(mode: BlendMode) -> [u8; 4] {
    match mode {
        BlendMode::Normal => *b"norm",
        BlendMode::Darken => *b"dark",
        BlendMode::Multiply => *b"mul ",
        BlendMode::ColorBurn => *b"idiv",
        BlendMode::LinearBurn => *b"lbrn",
        BlendMode::DarkerColor => *b"dkCl",
        BlendMode::Lighten => *b"lite",
        BlendMode::Screen => *b"scrn",
        BlendMode::ColorDodge => *b"div ",
        BlendMode::LinearDodge => *b"lddg",
        BlendMode::LighterColor => *b"lgCl",
        BlendMode::Overlay => *b"over",
        BlendMode::SoftLight => *b"sLit",
        BlendMode::HardLight => *b"hLit",
        BlendMode::VividLight => *b"vLit",
        BlendMode::LinearLight => *b"lLit",
        BlendMode::PinLight => *b"pLit",
        BlendMode::HardMix => *b"hMix",
        BlendMode::Difference => *b"diff",
        BlendMode::Exclusion => *b"smud",
        BlendMode::Subtract => *b"fsub",
        BlendMode::Divide => *b"fdiv",
        BlendMode::Hue => *b"hue ",
        BlendMode::Saturation => *b"sat ",
        BlendMode::Color => *b"colr",
        BlendMode::Luminosity => *b"lum ",
    }
}

/// Whether a key reads back as the mode it was written for. Used by the tests.
pub fn key_reads_back(mode: BlendMode) -> bool {
    matches!(super::blend::blend_from_key(&key_for(mode)), BlendKey::Supported(m) if m == mode)
}

// ---- bytes ------------------------------------------------------------------------------------

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// A length of the width the format uses for it: 4 bytes in a PSD, 8 in a PSB.
fn put_length(out: &mut Vec<u8>, value: u64, big: bool) {
    if big {
        out.extend_from_slice(&value.to_be_bytes());
    } else {
        out.extend_from_slice(&(value as u32).to_be_bytes());
    }
}

fn pad_to(out: &mut Vec<u8>, multiple: usize) {
    while !out.len().is_multiple_of(multiple) {
        out.push(0);
    }
}

/// PackBits for one row: runs of three or more equal bytes, literals between.
fn pack_bits(row: &[u8], out: &mut Vec<u8>) {
    let mut at = 0;
    while at < row.len() {
        let mut run = 1;
        while at + run < row.len() && row[at + run] == row[at] && run < 128 {
            run += 1;
        }
        if run >= 3 {
            out.push((1 - run as i16) as i8 as u8);
            out.push(row[at]);
            at += run;
        } else {
            let start = at;
            let mut count = 0;
            while at < row.len() && count < 128 {
                // A literal ends where a run of three begins.
                if at + 2 < row.len() && row[at] == row[at + 1] && row[at] == row[at + 2] {
                    break;
                }
                at += 1;
                count += 1;
            }
            out.push((count - 1) as u8);
            out.extend_from_slice(&row[start..start + count]);
        }
    }
}

/// One channel, with its compression field. `rows` rows of `row_bytes` bytes each.
///
/// 8- and 16-bit channels are PackBits; 32-bit channels are stored raw. A zero-sized channel is
/// the two bytes of a raw one.
fn encode_channel(samples: &[u8], row_bytes: usize, rows: usize, depth: u16, big: bool) -> Vec<u8> {
    if samples.is_empty() || row_bytes == 0 || rows == 0 {
        return vec![0, 0];
    }
    if depth == 32 {
        let mut out = vec![0, 0];
        out.extend_from_slice(samples);
        return out;
    }
    let mut out = vec![0, 1];
    let mut packed = Vec::with_capacity(samples.len() / 2 + rows * 2);
    let mut counts = Vec::with_capacity(rows);
    for row in samples.chunks(row_bytes).take(rows) {
        let before = packed.len();
        pack_bits(row, &mut packed);
        counts.push((packed.len() - before) as u32);
    }
    for count in counts {
        if big {
            put_u32(&mut out, count);
        } else {
            put_u16(&mut out, count as u16);
        }
    }
    out.extend_from_slice(&packed);
    out
}

// ---- pixels -----------------------------------------------------------------------------------

/// A layer's pixels in whichever form the document keeps them: straight alpha throughout.
#[derive(Clone)]
enum Pixels {
    /// sRGB-encoded 8-bit.
    Eight(Arc<RgbaImage>),
    /// Linear light, float.
    Float(Arc<FloatImage>),
}

/// What conversion to the file's depth lost, for the report.
#[derive(Default)]
struct Loss {
    /// Colour values above 1.0 that a 16-bit file cannot hold.
    highlights_clipped: bool,
}

impl Pixels {
    fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::Eight(image) => image.dimensions(),
            Self::Float(image) => image.dimensions(),
        }
    }

    fn from_buffer(buffer: &PixelBuffer) -> Self {
        match buffer {
            PixelBuffer::EncodedSrgba8(image) => Self::Eight(Arc::clone(image)),
            PixelBuffer::LinearRgbaF32(image) => Self::Float(Arc::clone(image)),
        }
    }

    /// Whether anything in the picture is not fully opaque.
    fn has_transparency(&self) -> bool {
        match self {
            Self::Eight(image) => image.pixels().any(|pixel| pixel[3] != 255),
            Self::Float(image) => image.pixels().iter().any(|pixel| pixel.alpha < 1.0),
        }
    }

    /// The smallest rectangle (x, y, width, height) holding every pixel with alpha above zero.
    fn opaque_bounds(&self) -> Option<(u32, u32, u32, u32)> {
        let (width, height) = self.dimensions();
        let alpha = |x: u32, y: u32| -> bool {
            match self {
                Self::Eight(image) => image.get_pixel(x, y)[3] > 0,
                Self::Float(image) => image.pixels()[(y * width + x) as usize].alpha > 0.0,
            }
        };
        let (mut left, mut top, mut right, mut bottom) = (width, height, 0_u32, 0_u32);
        for y in 0..height {
            for x in 0..width {
                if alpha(x, y) {
                    left = left.min(x);
                    right = right.max(x + 1);
                    top = top.min(y);
                    bottom = bottom.max(y + 1);
                }
            }
        }
        (right > left && bottom > top).then(|| (left, top, right - left, bottom - top))
    }

    /// The rectangle `(x, y, width, height)` as big-endian planes at `depth`: red, green, blue,
    /// alpha.
    fn planes(&self, rect: (u32, u32, u32, u32), depth: u16, loss: &mut Loss) -> [Vec<u8>; 4] {
        let (x0, y0, w, h) = rect;
        let bytes = usize::from(depth / 8);
        let count = w as usize * h as usize;
        let mut planes: [Vec<u8>; 4] = std::array::from_fn(|_| Vec::with_capacity(count * bytes));
        let (width, _) = self.dimensions();
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                let values: [f32; 4] = match self {
                    Self::Eight(image) => {
                        let p = image.get_pixel(x, y).0;
                        if depth == 8 {
                            for (plane, value) in planes.iter_mut().zip(p) {
                                plane.push(value);
                            }
                            continue;
                        }
                        if depth == 16 {
                            for (plane, value) in planes.iter_mut().zip(p) {
                                plane.extend_from_slice(&(u16::from(value) * 257).to_be_bytes());
                            }
                            continue;
                        }
                        // 32-bit: linear light.
                        [
                            srgb_decode(f32::from(p[0]) / 255.0),
                            srgb_decode(f32::from(p[1]) / 255.0),
                            srgb_decode(f32::from(p[2]) / 255.0),
                            f32::from(p[3]) / 255.0,
                        ]
                    }
                    Self::Float(image) => {
                        let p = image.pixels()[(y * width + x) as usize];
                        [p.red, p.green, p.blue, p.alpha]
                    }
                };
                for (index, value) in values.iter().enumerate() {
                    let value = if value.is_finite() { *value } else { 0.0 };
                    match depth {
                        8 | 16 => {
                            let encoded = if index < 3 {
                                if !(-RANGE_TOLERANCE..=1.0 + RANGE_TOLERANCE).contains(&value) {
                                    loss.highlights_clipped = true;
                                }
                                srgb_encode(value.clamp(0.0, 1.0))
                            } else {
                                value.clamp(0.0, 1.0)
                            };
                            if depth == 8 {
                                planes[index].push((encoded * 255.0).round() as u8);
                            } else {
                                planes[index].extend_from_slice(
                                    &((encoded * 65_535.0).round() as u16).to_be_bytes(),
                                );
                            }
                        }
                        _ => planes[index].extend_from_slice(&value.to_be_bytes()),
                    }
                }
            }
        }
        planes
    }
}

/// Mask coverage (0–255) as a big-endian plane at `depth`.
fn mask_plane(coverage: &[u8], depth: u16) -> Vec<u8> {
    match depth {
        8 => coverage.to_vec(),
        16 => coverage
            .iter()
            .flat_map(|value| (u16::from(*value) * 257).to_be_bytes())
            .collect(),
        _ => coverage
            .iter()
            .flat_map(|value| (f32::from(*value) / 255.0).to_be_bytes())
            .collect(),
    }
}

// ---- adjustments ------------------------------------------------------------------------------

/// A curve as the list of (input, output) points on the 0–255 scale, 16 at most.
type Points = Vec<(u16, u16)>;

fn is_identity_points(points: &[CurvePoint]) -> bool {
    points.len() == 2
        && points[0].input == 0.0
        && points[0].output == 0.0
        && points[1].input == 1.0
        && points[1].output == 1.0
}

/// PhotoForge's curve (straight lines between its points) as 256 outputs.
fn lut_of(points: &[CurvePoint]) -> [f32; 256] {
    let mut lut = [0.0_f32; 256];
    for (index, entry) in lut.iter_mut().enumerate() {
        let input = index as f32 / 255.0;
        let pair = points
            .windows(2)
            .find(|pair| input <= pair[1].input)
            .unwrap_or(&points[points.len().saturating_sub(2)..]);
        let span = (pair[1].input - pair[0].input).max(f32::EPSILON);
        let amount = ((input - pair[0].input) / span).clamp(0.0, 1.0);
        *entry =
            (pair[0].output + (pair[1].output - pair[0].output) * amount).clamp(0.0, 1.0) * 255.0;
    }
    lut
}

/// A 256-entry mapping reduced to at most 16 points that Photoshop's curve — a natural cubic
/// spline through them — follows: the ends, then each time the point where the spline through
/// those so far is furthest from the mapping, until all are within `tolerance` levels.
///
/// Fitting the spline, not the straight lines between points, is what keeps a kink (the top of a
/// brightness lift, the bend of a three-point curve) from being rounded off.
fn reduce(lut: &[f32; 256], tolerance: f32) -> Points {
    let mut chosen: Vec<usize> = vec![0, 255];
    while chosen.len() < 16 {
        let points: Vec<(f32, f32)> = chosen
            .iter()
            .map(|index| (*index as f32, lut[*index].round().clamp(0.0, 255.0)))
            .collect();
        let spline = super::adjustment::Spline::new(points);
        let mut worst = (0.0_f32, 0_usize);
        for (index, value) in lut.iter().enumerate() {
            if chosen.contains(&index) {
                continue;
            }
            let error = (value - spline.at(index as f32)).abs();
            if error > worst.0 {
                worst = (error, index);
            }
        }
        if worst.0 <= tolerance {
            break;
        }
        chosen.push(worst.1);
        chosen.sort_unstable();
    }
    chosen
        .into_iter()
        .map(|index| (index as u16, lut[index].round().clamp(0.0, 255.0) as u16))
        .collect()
}

fn curve_block(curves: &[(usize, Points)]) -> Vec<u8> {
    let mut mask = 0_u32;
    for (index, _) in curves {
        mask |= 1 << index;
    }
    let mut out = vec![0];
    put_u16(&mut out, 1);
    put_u32(&mut out, mask);
    let mut sorted: Vec<&(usize, Points)> = curves.iter().collect();
    sorted.sort_by_key(|(index, _)| *index);
    for (_, points) in &sorted {
        put_u16(&mut out, points.len() as u16);
        for (input, output) in points.iter() {
            // The output value comes first.
            put_u16(&mut out, *output);
            put_u16(&mut out, *input);
        }
    }
    // The extended form Photoshop also writes: the same curves again, each with its channel.
    out.extend_from_slice(b"Crv ");
    put_u16(&mut out, 4);
    put_u32(&mut out, sorted.len() as u32);
    for (index, points) in &sorted {
        put_u16(&mut out, *index as u16);
        put_u16(&mut out, points.len() as u16);
        for (input, output) in points.iter() {
            put_u16(&mut out, *output);
            put_u16(&mut out, *input);
        }
    }
    out
}

fn levels_block(
    input_black: u8,
    input_white: u8,
    gamma: f32,
    output_black: u8,
    output_white: u8,
) -> Vec<u8> {
    let mut out = Vec::new();
    put_u16(&mut out, 2);
    let neutral = [0_u16, 255, 0, 255, 100];
    let master = [
        u16::from(input_black),
        u16::from(input_white),
        u16::from(output_black),
        u16::from(output_white),
        (gamma * 100.0).round().clamp(10.0, 999.0) as u16,
    ];
    for index in 0..29 {
        for value in if index == 0 { master } else { neutral } {
            put_u16(&mut out, value);
        }
    }
    // The extra levels Photoshop writes after the twenty-nine: a tag, a version, a count, and
    // the remaining slots at their defaults.
    out.extend_from_slice(b"Lvls");
    put_u16(&mut out, 3);
    put_u16(&mut out, 62);
    for _ in 0..33 {
        for value in neutral {
            put_u16(&mut out, value);
        }
    }
    out
}

/// A block key and body, and a sentence for the report when it is not exact.
type AdjustmentBlock = ([u8; 4], Vec<u8>, Option<String>);

/// What an adjustment becomes in the file. `Err` is the reason it cannot be written.
fn adjustment_block(operation: &EditOperation) -> Result<AdjustmentBlock, String> {
    if let EditOperation::Levels {
        input_black,
        input_white,
        gamma,
        output_black,
        output_white,
    } = operation
    {
        return Ok((
            *b"levl",
            levels_block(
                *input_black,
                *input_white,
                *gamma,
                *output_black,
                *output_white,
            ),
            None,
        ));
    }
    let set: CurveSet = match operation {
        EditOperation::Curves { curves } => curves.clone(),
        // What a plug-in does is the plug-in's; there is nothing here to try it with.
        EditOperation::PluginFilter { .. } => {
            return Err("a plug-in filter layer has no Photoshop equivalent".to_string())
        }
        other => match as_curve_set(other) {
            Some(set) => set,
            None => {
                return Err(format!(
                    "a {} adjustment layer has no Photoshop equivalent PhotoForge can write",
                    other.kind().replace('_', " ")
                ))
            }
        },
    };
    let derived = !matches!(operation, EditOperation::Curves { .. });
    let (rgb, red, green, blue) = (&set.rgb, &set.red, &set.green, &set.blue);
    // An inversion is Photoshop's Invert.
    if red.len() == 2
        && green.len() == 2
        && blue.len() == 2
        && is_identity_points(red)
        && is_identity_points(green)
        && is_identity_points(blue)
        && rgb.len() == 2
        && rgb[0].input == 0.0
        && rgb[0].output == 1.0
        && rgb[1].input == 1.0
        && rgb[1].output == 0.0
    {
        return Ok((*b"nvrt", Vec::new(), None));
    }
    let channel_set = [red, green, blue];
    let any_channel = channel_set.iter().any(|points| !is_identity_points(points));
    let composite_set = !is_identity_points(rgb);
    let mut curves: Vec<(usize, Points)> = Vec::new();
    let mut inexact = derived;
    let points_of = |points: &[CurvePoint]| -> (Points, bool) {
        if points.len() == 2 {
            // A straight line is exactly two points.
            let lut = lut_of(points);
            return (reduce(&lut, 0.5), false);
        }
        (reduce(&lut_of(points), 0.75), true)
    };
    if composite_set && any_channel {
        // PhotoForge applies the composite curve first and a channel's second; Photoshop the
        // other way about. Each channel gets the pair composed, and the composite is dropped.
        let composite = lut_of(rgb);
        for (index, points) in channel_set.iter().enumerate() {
            let own = lut_of(points);
            let mut lut = [0.0_f32; 256];
            for (input, entry) in lut.iter_mut().enumerate() {
                let through_composite = composite[input].round().clamp(0.0, 255.0) as usize;
                *entry = own[through_composite];
            }
            curves.push((index + 1, reduce(&lut, 0.75)));
        }
        inexact = true;
    } else if composite_set {
        let (points, approximated) = points_of(rgb);
        curves.push((0, points));
        inexact |= approximated;
    } else {
        for (index, points) in channel_set.iter().enumerate() {
            if !is_identity_points(points) {
                let (reduced, approximated) = points_of(points);
                curves.push((index + 1, reduced));
                inexact |= approximated;
            }
        }
    }
    if curves.is_empty() {
        // An identity: kept as a composite curve through its two ends, so the layer is still there.
        curves.push((0, vec![(0, 0), (255, 255)]));
    }
    let note = inexact.then(|| {
        "Written as Photoshop Curves: the curve reduced to at most 16 points, which Photoshop joins with a smooth curve rather than straight lines."
            .to_string()
    });
    Ok((*b"curv", curve_block(&curves), note))
}

/// An adjustment that acts on each channel by itself, as curves, found by trying it. `None` when
/// it mixes channels or looks at neighbours.
fn as_curve_set(operation: &EditOperation) -> Option<CurveSet> {
    if !operation.supports_adjustment_layer() {
        return None;
    }
    // Greys: the mapping of each channel for the 256 inputs.
    let grey = RgbaImage::from_fn(256, 1, |x, _| image::Rgba([x as u8, x as u8, x as u8, 255]));
    let mapped = crate::image_processing::apply_operation(&grey, operation).ok()?;
    if mapped.dimensions() != (256, 1) {
        return None;
    }
    // Colours that differ by channel: each output channel must be the grey mapping of its own
    // input, whatever the other two are.
    let mixed = RgbaImage::from_fn(256, 1, |x, _| {
        image::Rgba([
            (255 - x) as u8,
            ((x * 7) % 256) as u8,
            ((x * 13 + 5) % 256) as u8,
            255,
        ])
    });
    let mixed_out = crate::image_processing::apply_operation(&mixed, operation).ok()?;
    if mixed_out.dimensions() != (256, 1) {
        return None;
    }
    for x in 0..256_u32 {
        let input = mixed.get_pixel(x, 0).0;
        let output = mixed_out.get_pixel(x, 0).0;
        for channel in 0..3 {
            let expected = mapped.get_pixel(u32::from(input[channel]), 0)[channel];
            if expected != output[channel] {
                return None;
            }
        }
        if output[3] != 255 {
            return None;
        }
    }
    let curve = |channel: usize| -> Vec<CurvePoint> {
        let mut lut = [0.0_f32; 256];
        for (x, entry) in lut.iter_mut().enumerate() {
            *entry = f32::from(mapped.get_pixel(x as u32, 0)[channel]);
        }
        reduce(&lut, 0.5)
            .into_iter()
            .map(|(input, output)| CurvePoint {
                input: f32::from(input) / 255.0,
                output: f32::from(output) / 255.0,
            })
            .collect()
    };
    Some(CurveSet {
        rgb: vec![
            CurvePoint {
                input: 0.0,
                output: 0.0,
            },
            CurvePoint {
                input: 1.0,
                output: 1.0,
            },
        ],
        red: curve(0),
        green: curve(1),
        blue: curve(2),
    })
}

// ---- layers -----------------------------------------------------------------------------------

/// One record of the file, with its channel data already encoded.
struct OutLayer {
    name: String,
    rect: (i32, i32, i32, i32),
    /// `(channel id, encoded data with its compression field)`.
    channels: Vec<(i16, Vec<u8>)>,
    blend: [u8; 4],
    opacity: u8,
    clipped: bool,
    flags: u8,
    mask: Option<OutMask>,
    blocks: Vec<([u8; 4], Vec<u8>)>,
}

struct OutMask {
    rect: (i32, i32, i32, i32),
    default: u8,
    flags: u8,
}

fn rect_of(x: i64, y: i64, w: u32, h: u32) -> (i32, i32, i32, i32) {
    (
        y as i32,
        x as i32,
        (y + i64::from(h)) as i32,
        (x + i64::from(w)) as i32,
    )
}

struct Writer<'a> {
    document: &'a LayerDocument,
    source: &'a dyn PixelSource,
    depth: u16,
    big: bool,
    float: bool,
    layers: Vec<OutLayer>,
    report: ExportReport,
    next_id: u32,
    loss: Loss,
    /// Bytes the channel data has taken so far, for the budget.
    encoded_bytes: u64,
    budget: u64,
}

impl Writer<'_> {
    fn fresh_id(&mut self) -> u32 {
        self.next_id += 1;
        self.next_id
    }

    fn pascal(name: &str) -> Vec<u8> {
        let mut bytes: Vec<u8> = name
            .chars()
            .map(|c| {
                if c.is_ascii() && !c.is_control() {
                    c as u8
                } else {
                    b'?'
                }
            })
            .take(31)
            .collect();
        bytes.insert(0, bytes.len() as u8);
        bytes
    }

    fn unicode_block(name: &str) -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        let mut body = Vec::new();
        put_u32(&mut body, units.len() as u32);
        for unit in &units {
            put_u16(&mut body, *unit);
        }
        body
    }

    fn account(&mut self, bytes: usize) -> Result<(), AppError> {
        self.encoded_bytes += bytes as u64;
        if self.encoded_bytes > self.budget {
            return Err(AppError::ResourceBudget {
                required: self.encoded_bytes,
                limit: self.budget,
            });
        }
        Ok(())
    }

    /// The four channels of a layer with no pixels, as Photoshop writes them.
    fn empty_channels() -> Vec<(i16, Vec<u8>)> {
        [-1_i16, 0, 1, 2]
            .into_iter()
            .map(|id| (id, vec![0, 0]))
            .collect()
    }

    fn name_blocks(&mut self, layer: &Layer) -> Vec<([u8; 4], Vec<u8>)> {
        let mut blocks = vec![(*b"luni", Self::unicode_block(&layer.name))];
        let id = self.fresh_id();
        blocks.push((*b"lyid", id.to_be_bytes().to_vec()));
        if layer.locked {
            blocks.push((*b"lspf", 0x8000_0000_u32.to_be_bytes().to_vec()));
        }
        blocks
    }

    fn opacity_byte(layer: &Layer) -> u8 {
        (layer.opacity.clamp(0.0, 1.0) * 255.0).round() as u8
    }

    fn flags(layer: &Layer, pixel_less: bool) -> u8 {
        let mut flags = 0x08;
        if !layer.visible {
            flags |= 0x02;
        }
        if pixel_less {
            flags |= 0x10;
        }
        flags
    }

    /// The mask of a layer, in canvas space or in the layer's own, as stored data.
    fn mask_data(
        &mut self,
        layer: &Layer,
        rect: (i32, i32, i32, i32),
    ) -> Result<Option<(OutMask, Vec<u8>)>, AppError> {
        let Some(mask) = &layer.mask else {
            return Ok(None);
        };
        let bitmap = mask.snapshot.decode()?;
        let (width, height) = (bitmap.width(), bitmap.height());
        let mut coverage = bitmap.coverage().to_vec();
        // Photoshop's own inversion flag is not honoured by every reader, so the inversion is
        // applied to the stored mask.
        if mask.inverted {
            for value in &mut coverage {
                *value = 255 - *value;
            }
        }
        let plane = mask_plane(&coverage, self.depth);
        let row_bytes = width as usize * usize::from(self.depth / 8);
        let data = encode_channel(&plane, row_bytes, height as usize, self.depth, self.big);
        self.account(data.len())?;
        Ok(Some((
            OutMask {
                rect,
                default: 0,
                flags: if mask.enabled { 0 } else { 0x02 },
            },
            data,
        )))
    }

    fn push_pixel_layer(
        &mut self,
        layer: &Layer,
        pixels: &Pixels,
        region: (u32, u32, u32, u32),
        position: (i64, i64),
        mask: Option<(OutMask, Vec<u8>)>,
    ) -> Result<(), AppError> {
        let (_, _, w, h) = region;
        let planes = pixels.planes(region, self.depth, &mut self.loss);
        let row_bytes = w as usize * usize::from(self.depth / 8);
        let mut channels = Vec::new();
        for (id, plane) in [(-1_i16, 3), (0, 0), (1, 1), (2, 2)] {
            let data = encode_channel(&planes[plane], row_bytes, h as usize, self.depth, self.big);
            self.account(data.len())?;
            channels.push((id, data));
        }
        let rect = rect_of(position.0, position.1, w, h);
        let mut out_mask = None;
        if let Some((mask, data)) = mask {
            channels.push((-2, data));
            out_mask = Some(mask);
        }
        let blocks = self.name_blocks(layer);
        self.layers.push(OutLayer {
            name: layer.name.clone(),
            rect,
            channels,
            blend: key_for(layer.blend_mode),
            opacity: Self::opacity_byte(layer),
            clipped: layer.clipped,
            flags: Self::flags(layer, false),
            mask: out_mask,
            blocks,
        });
        self.report.layers_written += 1;
        Ok(())
    }

    /// A pixel-less record: an adjustment, a divider or a layer that paints nothing.
    fn push_pixel_less(
        &mut self,
        layer: &Layer,
        blend: [u8; 4],
        mut blocks: Vec<([u8; 4], Vec<u8>)>,
        mask: Option<(OutMask, Vec<u8>)>,
    ) {
        let mut channels = Self::empty_channels();
        let mut out_mask = None;
        if let Some((mask, data)) = mask {
            channels.push((-2, data));
            out_mask = Some(mask);
        }
        let mut all = self.name_blocks(layer);
        all.append(&mut blocks);
        self.layers.push(OutLayer {
            name: layer.name.clone(),
            rect: (0, 0, 0, 0),
            channels,
            blend,
            opacity: Self::opacity_byte(layer),
            clipped: layer.clipped,
            flags: Self::flags(layer, true),
            mask: out_mask,
            blocks: all,
        });
    }

    fn canvas_rect(&self) -> (i32, i32, i32, i32) {
        (
            0,
            0,
            self.document.canvas_height as i32,
            self.document.canvas_width as i32,
        )
    }

    /// Renders one layer alone — its own transform, content and children, but not its mask,
    /// opacity or blend mode — at canvas size.
    fn isolated(&self, layer: &Layer) -> Result<Pixels, AppError> {
        let mut alone = layer.clone();
        alone.visible = true;
        alone.opacity = 1.0;
        alone.blend_mode = BlendMode::Normal;
        alone.mask = None;
        alone.clipped = false;
        if let LayerContent::Group { isolated, .. } = &mut alone.content {
            // A pass-through group on a blank backdrop is an isolated one.
            *isolated = true;
        }
        let mut document =
            LayerDocument::new(self.document.canvas_width, self.document.canvas_height);
        document.precision = self.document.precision;
        document.smart_sources = self.document.smart_sources.clone();
        document.layers.push(alone);
        let buffer = render_document_typed(&document, self.source, RenderOptions::default())?;
        Ok(Pixels::from_buffer(&buffer))
    }

    /// The layer's mask as the renderer samples it: at each canvas pixel, through the inverse of
    /// the layer's transform, in the space the mask lives in. Coverage at canvas size, with the
    /// mask's inversion applied.
    fn moved_mask(&self, layer: &Layer) -> Result<Option<Vec<u8>>, AppError> {
        let Some(mask) = &layer.mask else {
            return Ok(None);
        };
        let bitmap = mask.snapshot.decode()?;
        let (space_width, space_height) = (bitmap.width(), bitmap.height());
        let (canvas_width, canvas_height) =
            (self.document.canvas_width, self.document.canvas_height);
        let transform = crate::layers::render_transform(&layer.transform, 1.0);
        // The mask is in the layer's own space for a pixel layer and a smart object, and in the
        // canvas's for a group; its own dimensions are that space's in every case.
        let inverse = transform.inverse(space_width, space_height)?;
        let mut coverage = Vec::with_capacity(canvas_width as usize * canvas_height as usize);
        for y in 0..canvas_height {
            for x in 0..canvas_width {
                let (sx, sy) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
                let value = crate::layers::mask_coverage(
                    Some(&bitmap),
                    mask.inverted,
                    sx,
                    sy,
                    space_width,
                    space_height,
                    layer.transform.interpolation,
                );
                coverage.push((value.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        Ok(Some(coverage))
    }

    /// A baked or rasterised layer: its content rendered at canvas size, cropped to what it
    /// paints, with its mask (if any) carried the same way or taken from canvas space.
    fn push_rasterised(
        &mut self,
        layer: &Layer,
        mask_in_canvas_space: bool,
    ) -> Result<(), AppError> {
        let rendered = self.isolated(layer)?;
        let Some(bounds) = rendered.opaque_bounds() else {
            // It paints nothing: an empty layer, so that it is still there to be filled.
            self.push_pixel_less(layer, key_for(layer.blend_mode), Vec::new(), None);
            self.report.layers_written += 1;
            return Ok(());
        };
        let (x, y, w, h) = bounds;
        let mask = if layer.mask.is_none() {
            None
        } else if mask_in_canvas_space {
            // Canvas-space masks stay where they are.
            self.mask_data(layer, self.canvas_rect())?
        } else {
            let moved = self.moved_mask(layer)?.unwrap_or_default();
            let (cw, _) = rendered.dimensions();
            let mut cropped = Vec::with_capacity(w as usize * h as usize);
            for row in y..y + h {
                let start = (row * cw + x) as usize;
                cropped.extend_from_slice(&moved[start..start + w as usize]);
            }
            let plane = mask_plane(&cropped, self.depth);
            let row_bytes = w as usize * usize::from(self.depth / 8);
            let data = encode_channel(&plane, row_bytes, h as usize, self.depth, self.big);
            self.account(data.len())?;
            let enabled = layer.mask.as_ref().is_some_and(|m| m.enabled);
            Some((
                OutMask {
                    rect: rect_of(i64::from(x), i64::from(y), w, h),
                    default: 0,
                    flags: if enabled { 0 } else { 0x02 },
                },
                data,
            ))
        };
        self.push_pixel_layer(layer, &rendered, bounds, (i64::from(x), i64::from(y)), mask)
    }

    fn write_layer(&mut self, layer: &Layer) -> Result<(), AppError> {
        let name = layer.name.clone();
        match &layer.content {
            LayerContent::Pixel {
                pixel_id,
                width,
                height,
            } => {
                let identity_placement = integer_placement(&layer.transform);
                match identity_placement {
                    Some((dx, dy)) => {
                        let pixels = if self.float {
                            Pixels::Float(self.source.resolve_linear(pixel_id)?)
                        } else {
                            Pixels::Eight(self.source.resolve(pixel_id)?)
                        };
                        let (w, h) = pixels.dimensions();
                        if (w, h) != (*width, *height) {
                            return Err(failure(format!(
                                "the pixels of {name} are {w} x {h}, not the {width} x {height} the layer says"
                            )));
                        }
                        let rect = rect_of(dx, dy, w, h);
                        let mask = self.mask_data(layer, rect)?;
                        self.push_pixel_layer(layer, &pixels, (0, 0, w, h), (dx, dy), mask)
                    }
                    None => {
                        self.report.push(
                            ReportKind::Approximated,
                            Some(&name),
                            "The layer is scaled, rotated, flipped or placed between pixels; its transform was applied and the result stored as pixels, so the transform can no longer be changed.",
                        );
                        self.push_rasterised(layer, false)
                    }
                }
            }
            LayerContent::Shape { .. } => {
                self.report.push(
                    ReportKind::Approximated,
                    Some(&name),
                    "Shape layer written as the pixels it draws; it is no longer an editable shape in the file.",
                );
                self.push_rasterised(layer, true)
            }
            LayerContent::Text { .. } => {
                self.report.push(
                    ReportKind::Approximated,
                    Some(&name),
                    "Text layer written as the pixels it draws; it is no longer editable text in the file.",
                );
                self.push_rasterised(layer, true)
            }
            LayerContent::SmartObject { .. } => {
                self.report.push(
                    ReportKind::Approximated,
                    Some(&name),
                    "Smart object written as its rendered pixels; the source is not in the file.",
                );
                // A smart object's mask is in the space of its source, and moves with it.
                self.push_rasterised(layer, false)
            }
            LayerContent::Adjustment { operation } => {
                match adjustment_block(operation) {
                    Ok((key, body, note)) => {
                        if let Some(note) = note {
                            self.report
                                .push(ReportKind::Approximated, Some(&name), note);
                        }
                        let mask = self.mask_data(layer, self.canvas_rect())?;
                        self.push_pixel_less(
                            layer,
                            key_for(layer.blend_mode),
                            vec![(key, body)],
                            mask,
                        );
                        self.report.layers_written += 1;
                    }
                    Err(reason) => {
                        self.report.push(
                            ReportKind::NotExported,
                            Some(&name),
                            format!("{reason}; it is not in the file, and the layers beneath it are shown without it there."),
                        );
                    }
                }
                Ok(())
            }
            LayerContent::Group { children, isolated } => {
                if !layer.transform.is_identity() {
                    self.report.push(
                        ReportKind::Approximated,
                        Some(&name),
                        "The group has a transform; it was flattened into one layer with the transform applied.",
                    );
                    return self.push_rasterised(layer, false);
                }
                // The bounding divider, the children, then the folder record that names the group.
                let divider_blocks = vec![(*b"lsct", 3_u32.to_be_bytes().to_vec())];
                let ghost = Layer {
                    name: "</Layer group>".to_string(),
                    mask: None,
                    clipped: false,
                    opacity: 1.0,
                    visible: true,
                    locked: false,
                    ..layer.clone()
                };
                self.push_pixel_less(&ghost, *b"norm", divider_blocks, None);
                for child in children {
                    self.write_layer(child)?;
                }
                // Photoshop's own record says Normal for a pass-through group and its section
                // divider says `pass`; for any other group both say the mode.
                let key = if *isolated {
                    key_for(layer.blend_mode)
                } else {
                    *b"pass"
                };
                let record_key = if *isolated { key } else { *b"norm" };
                let mut section = Vec::new();
                put_u32(&mut section, if layer.collapsed { 2 } else { 1 });
                section.extend_from_slice(b"8BIM");
                section.extend_from_slice(&key);
                put_u32(&mut section, 0);
                let mask = self.mask_data(layer, self.canvas_rect())?;
                self.push_pixel_less(layer, record_key, vec![(*b"lsct", section)], mask);
                self.report.groups_written += 1;
                Ok(())
            }
        }
    }
}

/// A whole-pixel move with nothing else: the one placement a Photoshop layer holds as it is.
fn integer_placement(transform: &LayerTransform) -> Option<(i64, i64)> {
    transform.integer_translation()
}

// ---- the file ---------------------------------------------------------------------------------

fn layer_info(layers: &[OutLayer], depth: u16, big: bool, merged_has_alpha: bool) -> Vec<u8> {
    let _ = depth;
    let mut records = Vec::new();
    let mut data = Vec::new();
    for layer in layers {
        let (top, left, bottom, right) = layer.rect;
        for value in [top, left, bottom, right] {
            put_i32(&mut records, value);
        }
        put_u16(&mut records, layer.channels.len() as u16);
        for (id, bytes) in &layer.channels {
            records.extend_from_slice(&id.to_be_bytes());
            put_length(&mut records, bytes.len() as u64, big);
        }
        records.extend_from_slice(b"8BIM");
        records.extend_from_slice(&layer.blend);
        records.push(layer.opacity);
        records.push(u8::from(layer.clipped));
        records.push(layer.flags);
        records.push(0);

        let mut extra = Vec::new();
        match &layer.mask {
            None => put_u32(&mut extra, 0),
            Some(mask) => {
                put_u32(&mut extra, 20);
                let (top, left, bottom, right) = mask.rect;
                for value in [top, left, bottom, right] {
                    put_i32(&mut extra, value);
                }
                extra.push(mask.default);
                extra.push(mask.flags);
                extra.extend_from_slice(&[0, 0]);
            }
        }
        // Blending ranges: the defaults Photoshop writes, composite and four channels.
        put_u32(&mut extra, 40);
        for _ in 0..10 {
            extra.extend_from_slice(&[0, 0, 0xff, 0xff]);
        }
        let pascal = Writer::pascal(&layer.name);
        extra.extend_from_slice(&pascal);
        pad_to(&mut extra, 4);
        for (key, body) in &layer.blocks {
            // The length of a block counts its padding, which is to a multiple of four: a
            // reader that skips a block by its length must land on the next signature.
            let mut body = body.clone();
            pad_to(&mut body, 4);
            extra.extend_from_slice(b"8BIM");
            extra.extend_from_slice(key);
            put_u32(&mut extra, body.len() as u32);
            extra.extend_from_slice(&body);
        }
        put_u32(&mut records, extra.len() as u32);
        records.extend_from_slice(&extra);
        for (_, bytes) in &layer.channels {
            data.extend_from_slice(bytes);
        }
    }
    // A negative count says the first alpha channel of the flattened picture is transparency.
    let count = layers.len() as i16;
    let mut out = (if merged_has_alpha { -count } else { count })
        .to_be_bytes()
        .to_vec();
    out.extend_from_slice(&records);
    out.extend_from_slice(&data);
    out
}

/// The flattened picture's channels, compressed the way the layers' are, in one block with the
/// row counts of all channels first.
fn merged_image(planes: &[Vec<u8>], width: u32, height: u32, depth: u16, big: bool) -> Vec<u8> {
    let row_bytes = width as usize * usize::from(depth / 8);
    if depth == 32 {
        let mut out = vec![0, 0];
        for plane in planes {
            out.extend_from_slice(plane);
        }
        return out;
    }
    let mut out = vec![0, 1];
    let mut counts = Vec::new();
    let mut packed = Vec::new();
    for plane in planes {
        for row in plane.chunks(row_bytes.max(1)).take(height as usize) {
            let before = packed.len();
            pack_bits(row, &mut packed);
            counts.push((packed.len() - before) as u32);
        }
    }
    for count in counts {
        if big {
            put_u32(&mut out, count);
        } else {
            put_u16(&mut out, count as u16);
        }
    }
    out.extend_from_slice(&packed);
    out
}

/// Whether any pixel layer holds colour a 16-bit sRGB file cannot: above white, or below black
/// (which is what a colour more saturated than sRGB becomes in linear sRGB).
fn holds_out_of_range_colour(
    document: &LayerDocument,
    source: &dyn PixelSource,
) -> Result<bool, AppError> {
    for layer in document.iter_all() {
        if let LayerContent::Pixel { pixel_id, .. } = &layer.content {
            let image = source.resolve_linear(pixel_id)?;
            let bright = image.pixels().iter().any(|p| {
                p.alpha > 0.0
                    && [p.red, p.green, p.blue].iter().any(|v| {
                        v.is_finite() && !(-RANGE_TOLERANCE..=1.0 + RANGE_TOLERANCE).contains(v)
                    })
            });
            if bright {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Exports `document` to `path`. Never changes the document; writes the file only if every part
/// of it was made, by writing beside it and then renaming.
pub fn export_psd(
    document: &LayerDocument,
    source: &dyn PixelSource,
    path: &Path,
    options: &ExportOptions,
) -> Result<ExportReport, AppError> {
    document.validate()?;
    let float = document.precision == DocumentPrecision::LinearSrgbF32;
    let depth = match options.bit_depth {
        Some(depth @ (8 | 16 | 32)) => depth,
        Some(other) => {
            return Err(failure(format!(
                "{other} bits per channel is not a depth Photoshop files use"
            )))
        }
        None => {
            if !float {
                8
            } else if holds_out_of_range_colour(document, source)? {
                // Colour outside 0..1 does not fit 16 bits; 32-bit float keeps it.
                32
            } else {
                16
            }
        }
    };
    let (width, height) = (document.canvas_width, document.canvas_height);

    // The most this can take, counted before anything is made: every layer's pixels once, as
    // written, and the flattened picture.
    let bytes_per_sample = u64::from(depth / 8);
    let mut raw = u64::from(width) * u64::from(height) * 4 * bytes_per_sample;
    let mut widest = width.max(height);
    for layer in document.iter_all() {
        if let Some((w, h)) = layer.pixel_dimensions() {
            raw += u64::from(w) * u64::from(h) * 4 * bytes_per_sample;
            widest = widest.max(w).max(h);
        }
    }
    let budget = crate::resources::max_job_bytes();
    if raw > budget {
        return Err(AppError::ResourceBudget {
            required: raw,
            limit: budget,
        });
    }
    let big = options.force_large_format || widest > PSD_MAX_SIDE || raw > PSD_MAX_BYTES;

    let mut writer = Writer {
        document,
        source,
        depth,
        big,
        float,
        layers: Vec::new(),
        report: ExportReport {
            format: if big { "PSB" } else { "PSD" }.to_string(),
            width,
            height,
            bit_depth: depth,
            color_mode: "RGB".to_string(),
            ..ExportReport::default()
        },
        next_id: 0,
        loss: Loss::default(),
        encoded_bytes: 0,
        budget,
    };
    if options.bit_depth.is_none() && float && depth == 32 {
        writer.report.push(
            ReportKind::Note,
            None,
            "Written as 32-bit because the document holds colour a 16-bit file cannot keep: brighter than white, or more saturated than sRGB.",
        );
    }
    for layer in &document.layers {
        writer.write_layer(layer)?;
    }

    // The flattened picture: the layers as PhotoForge composites them, and the edits that sit over
    // them when they keep its size.
    let rendered = render_document_typed(document, source, RenderOptions::default())?;
    let mut picture = rendered;
    if !options.operations.is_empty() {
        let keeps_size = options
            .operations
            .iter()
            .all(|operation| operation.supports_adjustment_layer());
        if keeps_size {
            picture = crate::image_processing::high_precision::pipeline_typed(
                picture,
                &options.operations,
                None,
            )?;
        }
        let kinds: Vec<String> = options
            .operations
            .iter()
            .map(|operation| operation.kind().replace('_', " "))
            .collect();
        writer.report.push(
            ReportKind::NotExported,
            None,
            format!(
                "Edits over the whole document ({}) are not in the layers.{}",
                kinds.join(", "),
                if keeps_size {
                    " They are in the flattened picture the file also holds."
                } else {
                    " They change the picture's size or shape, so they are in neither."
                }
            ),
        );
    }
    let merged = Pixels::from_buffer(&picture);
    if merged.dimensions() != (width, height) {
        return Err(failure(
            "the flattened picture is not the size of the canvas",
        ));
    }
    let has_alpha = merged.has_transparency();
    let mut loss = std::mem::take(&mut writer.loss);
    let planes = merged.planes((0, 0, width, height), depth, &mut loss);
    let channel_count: u16 = if has_alpha { 4 } else { 3 };
    let merged_planes: Vec<Vec<u8>> = if has_alpha {
        vec![
            planes[0].clone(),
            planes[1].clone(),
            planes[2].clone(),
            planes[3].clone(),
        ]
    } else {
        vec![planes[0].clone(), planes[1].clone(), planes[2].clone()]
    };
    if loss.highlights_clipped {
        writer.report.push(
            ReportKind::Approximated,
            None,
            format!("Colour brighter than white, or more saturated than sRGB, was clipped: a {depth}-bit file holds colour from black to white. Export as 32-bit to keep it."),
        );
    }

    // Assemble.
    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    put_u16(&mut out, if big { 2 } else { 1 });
    out.extend_from_slice(&[0; 6]);
    put_u16(&mut out, channel_count);
    put_u32(&mut out, height);
    put_u32(&mut out, width);
    put_u16(&mut out, depth);
    put_u16(&mut out, 3); // RGB
    put_u32(&mut out, 0); // colour mode data

    // Image resources: the sRGB profile, for the encoded depths.
    let mut resources = Vec::new();
    if depth != 32 {
        let icc = crate::color_management::RgbColorSpace::Srgb.icc_bytes()?;
        resources.extend_from_slice(b"8BIM");
        put_u16(&mut resources, 1039);
        resources.extend_from_slice(&[0, 0]);
        put_u32(&mut resources, icc.len() as u32);
        resources.extend_from_slice(&icc);
        if icc.len() % 2 == 1 {
            resources.push(0);
        }
    }
    if let Some(metadata) = &options.metadata {
        for (id, block) in [
            (1058_u16, &metadata.exif),
            (1060, &metadata.xmp),
            (1028, &metadata.iptc),
        ] {
            let Some(block) = block else { continue };
            resources.extend_from_slice(b"8BIM");
            put_u16(&mut resources, id);
            resources.extend_from_slice(&[0, 0]);
            put_u32(&mut resources, block.len() as u32);
            resources.extend_from_slice(block);
            if block.len() % 2 == 1 {
                resources.push(0);
            }
        }
    }
    put_u32(&mut out, resources.len() as u32);
    out.extend_from_slice(&resources);

    // Layer and mask information. A 16- or 32-bit document keeps its layers in a tagged block.
    let info = layer_info(&writer.layers, depth, big, has_alpha);
    let mut section = Vec::new();
    if depth == 8 {
        let mut padded = info;
        pad_to(&mut padded, 2);
        put_length(&mut section, padded.len() as u64, big);
        section.extend_from_slice(&padded);
        put_u32(&mut section, 0); // global layer mask info
    } else {
        put_length(&mut section, 0, big);
        put_u32(&mut section, 0);
        let key: &[u8; 4] = if depth == 16 { b"Lr16" } else { b"Lr32" };
        let mut padded = info;
        pad_to(&mut padded, 4);
        section.extend_from_slice(b"8BIM");
        section.extend_from_slice(key);
        // The layer blocks of a PSB carry an 8-byte length.
        put_length(&mut section, padded.len() as u64, big);
        section.extend_from_slice(&padded);
    }
    put_length(&mut out, section.len() as u64, big);
    out.extend_from_slice(&section);

    // Image data.
    out.extend_from_slice(&merged_image(&merged_planes, width, height, depth, big));

    if !big && out.len() as u64 >= (1 << 32) - 1 {
        return Err(failure(
            "the file would be larger than a PSD can hold; export it with a smaller canvas or as a PSB",
        ));
    }
    write_beside_then_rename(path, &out)?;
    writer.report.bytes = out.len() as u64;
    Ok(writer.report)
}

/// Writes next to `path` and renames, so a failed or interrupted export leaves what was there.
fn write_beside_then_rename(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let directory = path.parent().filter(|p| !p.as_os_str().is_empty());
    let temporary = match directory {
        Some(directory) => directory.join(format!(
            ".{}.photoforge-export",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("export")
        )),
        None => Path::new(".photoforge-export").to_path_buf(),
    };
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(AppError::ExportFailure);
    }
    Ok(())
}
