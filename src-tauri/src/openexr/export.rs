//! Writing the document as an OpenEXR file.
//!
//! An OpenEXR file here holds **one picture**: the layers as PhotoForge composites them, with the
//! document's own edits over them, as scene-linear light. The working space is already linear
//! light on sRGB primaries with straight alpha and no upper bound, so nothing is tone-mapped or
//! clipped on the way out: a highlight at 40 times white is 40.0 in the file. The one change of
//! convention is OpenEXR's: colour is stored premultiplied by alpha.
//!
//! The file says what its colour means (a `chromaticities` attribute with Rec. 709's primaries),
//! so a reader that honours the attribute, and one that assumes Rec. 709 without it, agree.
use crate::color::FloatImage;
use crate::domain::EditOperation;
use crate::error::AppError;
use crate::image_processing::high_precision::pipeline_typed;
use crate::layers::{render_document_typed, LayerDocument, PixelSource, RenderOptions};
use crate::pixel::DocumentPrecision;
use crate::psd::{ReportEntry, ReportKind};
use serde::Serialize;

/// The largest value half precision holds.
const HALF_MAX: f32 = 65_504.0;

/// What the file is written with.
#[derive(Debug, Clone, Default)]
pub struct ExrOptions {
    /// Edits over the whole document (crop, rotation, resize, adjustments), applied to the
    /// composite before it is written.
    pub operations: Vec<EditOperation>,
    /// 16 for half float, 32 for single precision; `None` is single precision.
    pub bit_depth: Option<u16>,
}

/// What an export did, in the shape the Photoshop export report has, so one dialog shows both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExrExportReport {
    /// Always "OpenEXR".
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u16,
    /// "RGB" or "RGBA".
    pub color_mode: String,
    /// Always one: the composite.
    pub layers_written: usize,
    pub groups_written: usize,
    pub bytes: u64,
    pub entries: Vec<ReportEntry>,
}

impl ExrExportReport {
    fn push(&mut self, kind: ReportKind, detail: impl Into<String>) {
        self.entries.push(ReportEntry {
            kind,
            layer: None,
            detail: detail.into(),
        });
    }

    pub fn count(&self, kind: ReportKind) -> usize {
        self.entries.iter().filter(|e| e.kind == kind).count()
    }
}

/// The encoded file and what is to be said about it.
pub struct ExrExport {
    pub bytes: Vec<u8>,
    pub report: ExrExportReport,
}

fn failure() -> AppError {
    AppError::ExportFailure
}

/// Encodes `picture` (straight-alpha linear sRGB) as an OpenEXR file in memory.
///
/// `half`: samples are written as 16-bit floats, which hold ±65504 with about three decimal
/// digits; anything beyond is clamped, and the count is returned. `alpha`: write an alpha
/// channel; otherwise the file has R, G, B only and the alpha in `picture` is ignored.
pub(crate) fn encode(
    picture: &FloatImage,
    half: bool,
    alpha: bool,
) -> Result<(Vec<u8>, u64), AppError> {
    use exr::prelude::*;
    let (width, height) = picture.dimensions();
    let (width, height) = (width as usize, height as usize);
    let pixels = picture.pixels();

    let layer_attributes = LayerAttributes {
        software_name: Some(Text::from(
            format!("PhotoForge {}", env!("CARGO_PKG_VERSION")).as_str(),
        )),
        ..LayerAttributes::default()
    };
    let encoding = Encoding::SMALL_LOSSLESS;

    let mut clamped = 0_u64;
    let mut bytes = std::io::Cursor::new(Vec::new());

    // Premultiplied, as OpenEXR stores it. Values are read straight from the shared buffer by
    // position, so no second copy of the picture is made.
    let premultiplied = move |x: usize, y: usize| {
        let p = &pixels[y * width + x];
        (
            p.red * p.alpha,
            p.green * p.alpha,
            p.blue * p.alpha,
            p.alpha,
        )
    };

    macro_rules! write_with {
        ($channels:expr) => {{
            let layer = Layer::new((width, height), layer_attributes, encoding, $channels);
            let mut image = Image::from_layer(layer);
            image.attributes.chromaticities = Some(exr::meta::attribute::Chromaticities {
                red: Vec2(0.6400, 0.3300),
                green: Vec2(0.3000, 0.6000),
                blue: Vec2(0.1500, 0.0600),
                white: Vec2(0.3127, 0.3290),
            });
            image
                .write()
                .non_parallel()
                .to_buffered(&mut bytes)
                .map_err(|_| failure())?;
        }};
    }

    if half {
        // Count what half precision cannot hold, so it can be said.
        for p in pixels {
            let a = if alpha { p.alpha } else { 1.0 };
            if [p.red * a, p.green * a, p.blue * a]
                .iter()
                .any(|v| v.abs() > HALF_MAX)
            {
                clamped += 1;
            }
        }
        let to_half = |v: f32| f16::from_f32(v.clamp(-HALF_MAX, HALF_MAX));
        if alpha {
            write_with!(SpecificChannels::rgba(|Vec2(x, y): Vec2<usize>| {
                let (r, g, b, a) = premultiplied(x, y);
                (to_half(r), to_half(g), to_half(b), to_half(a))
            }));
        } else {
            write_with!(SpecificChannels::rgb(|Vec2(x, y): Vec2<usize>| {
                let p = &pixels[y * width + x];
                (to_half(p.red), to_half(p.green), to_half(p.blue))
            }));
        }
    } else if alpha {
        write_with!(SpecificChannels::rgba(|Vec2(x, y): Vec2<usize>| {
            premultiplied(x, y)
        }));
    } else {
        write_with!(SpecificChannels::rgb(|Vec2(x, y): Vec2<usize>| {
            let p = &pixels[y * width + x];
            (p.red, p.green, p.blue)
        }));
    }
    Ok((bytes.into_inner(), clamped))
}

/// Renders `document` and encodes it as OpenEXR. Never changes the document.
pub fn export_exr(
    document: &LayerDocument,
    source: &dyn PixelSource,
    options: &ExrOptions,
) -> Result<ExrExport, AppError> {
    document.validate()?;
    let depth = match options.bit_depth {
        None | Some(32) => 32,
        Some(16) => 16,
        Some(other) => {
            let message = format!(
                "{other} bits per channel is not a depth OpenEXR holds colour in; use 16 (half) or 32"
            );
            return Err(AppError::ProcessingFailure(message));
        }
    };
    let rendered = render_document_typed(document, source, RenderOptions::default())?;
    let processed = pipeline_typed(rendered, &options.operations, None)?;
    let (width, height) = processed.dimensions();
    // The file is scene-linear, so an 8-bit picture is decoded from sRGB to linear light here.
    let picture = processed.linear()?;
    let has_alpha = picture.pixels().iter().any(|p| p.alpha < 1.0);

    // Peak: the encoded file in memory, at most the uncompressed size, beside the picture.
    let channels: u64 = if has_alpha { 4 } else { 3 };
    let sample: u64 = u64::from(depth / 8);
    let file_bytes = u64::from(width) * u64::from(height) * channels * sample;
    crate::resources::ResourceEstimate::new(processed.bytes(), file_bytes, 0, file_bytes)?;

    let (bytes, clamped) = encode(&picture, depth == 16, has_alpha)?;

    let mut report = ExrExportReport {
        format: "OpenEXR".to_string(),
        width,
        height,
        bit_depth: depth,
        color_mode: if has_alpha { "RGBA" } else { "RGB" }.to_string(),
        layers_written: 1,
        groups_written: 0,
        bytes: bytes.len() as u64,
        entries: Vec::new(),
    };
    let layer_count = document.iter_all().count();
    if layer_count > 1 {
        report.push(
            ReportKind::NotExported,
            format!(
                "The document's {layer_count} layers were composited into one picture: an OpenEXR file written here holds no layers, masks or adjustments of its own."
            ),
        );
    }
    if !options.operations.is_empty() {
        report.push(
            ReportKind::Note,
            "The document's own edits (crop, rotation, resize, global adjustments) are in the picture.",
        );
    }
    if document.precision == DocumentPrecision::LegacySrgb8 {
        report.push(
            ReportKind::Note,
            "The document is 8-bit: its colour was decoded to linear light, but the file carries only the 256 levels per channel the document had.",
        );
    }
    if clamped > 0 {
        report.push(
            ReportKind::Approximated,
            format!(
                "{clamped} pixel{} held a value beyond what half precision holds (±65504) and was clamped; write 32-bit to keep it.",
                if clamped == 1 { "" } else { "s" }
            ),
        );
    }
    report.push(
        ReportKind::Note,
        format!(
            "Written as scene-linear light on Rec. 709 / sRGB primaries (stated in the file), premultiplied by alpha, {}, ZIP-compressed.",
            if depth == 16 {
                "half float (16-bit)"
            } else {
                "single precision (32-bit)"
            }
        ),
    );
    Ok(ExrExport { bytes, report })
}

#[cfg(test)]
mod tests;
