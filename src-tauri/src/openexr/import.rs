//! Reading an OpenEXR file as a layered document.
//!
//! An EXR is scene-linear light, usually half or single precision, with premultiplied alpha and an
//! optional statement of which primaries its RGB means. PhotoForge's high-precision documents are
//! linear light on sRGB primaries with straight alpha, so what is read is *converted*, never
//! reinterpreted: the colour goes through a primaries matrix if the file says it is not Rec. 709,
//! and is divided by its alpha.
//!
//! The file is a stranger's, so it is admitted from its header before any pixel is decoded: the
//! sizes are checked against the resource budget, deep data and the codecs the reader does not
//! have are refused by name, and nothing that would need an allocation the budget does not allow
//! is begun.
//!
//! Everything the picture carries that PhotoForge has no place for (depth, other parts of a
//! multi-part file, a subsampled luminance/chroma image) is left out **and named**.
use super::colour::{Primaries, REC709};
use crate::color::{FloatImage, FloatRgba};
use crate::error::AppError;
use crate::layers::{
    validate_dimensions, Layer, LayerContent, LayerDocument, LoadedProject, MAX_LAYERS,
    MAX_LAYER_NAME_CHARS,
};
use crate::operations::structure::new_layer;
use crate::operations::support::{Clock, SystemClock};
use crate::pixel::DocumentPrecision;
use crate::psd::{ImportLimits, ReportEntry, ReportKind};
use exr::meta::attribute::{Compression, SampleType, Text};
use exr::meta::header::Header;
use exr::meta::MetaData;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// The largest file read. A legitimate multi-layer render can be large; a file past this is
/// refused before anything else is looked at.
pub const MAX_EXR_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// The most colour sets (a main picture and its passes) imported as layers from one file.
const MAX_SETS: usize = 16;
/// Where an infinite sample is put: the largest finite half-precision value, so a pixel that was
/// "infinitely bright" is bright, not a hole.
const HALF_MAX: f32 = 65_504.0;

/// What an import did, in the shape the Photoshop import report has, so one dialog shows both.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExrImportReport {
    /// Always "OpenEXR".
    pub format: String,
    pub width: u32,
    pub height: u32,
    /// 16 (half) or 32 (single precision), as the file stores its colour.
    pub bit_depth: u16,
    /// "RGB" or "Grayscale".
    pub color_mode: String,
    pub layers_imported: usize,
    pub groups_imported: usize,
    pub entries: Vec<ReportEntry>,
}

impl ExrImportReport {
    fn push(&mut self, kind: ReportKind, layer: Option<&str>, detail: impl Into<String>) {
        self.entries.push(ReportEntry {
            kind,
            layer: layer.map(str::to_string),
            detail: detail.into(),
        });
    }

    pub fn count(&self, kind: ReportKind) -> usize {
        self.entries.iter().filter(|e| e.kind == kind).count()
    }
}

/// An imported document, as the project loader would hand it over.
pub struct ImportedExr {
    pub project: LoadedProject,
    pub report: ExrImportReport,
}

fn refuse(message: impl Into<String>) -> AppError {
    AppError::foreign_file(message.into())
}

/// How the channels of a set are named: OpenEXR's `R G B A`, or the long names some compositors
/// write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Naming {
    Short,
    Long,
}

impl Naming {
    fn red(self) -> &'static str {
        match self {
            Naming::Short => "R",
            Naming::Long => "red",
        }
    }
    fn green(self) -> &'static str {
        match self {
            Naming::Short => "G",
            Naming::Long => "green",
        }
    }
    fn blue(self) -> &'static str {
        match self {
            Naming::Short => "B",
            Naming::Long => "blue",
        }
    }
    fn alpha(self) -> &'static str {
        match self {
            Naming::Short => "A",
            Naming::Long => "alpha",
        }
    }
}

/// One picture inside the file: a prefix (empty for the main picture) and the channels that make
/// it, found by name.
#[derive(Debug, Clone, PartialEq)]
struct ColourSet {
    /// "" or e.g. "diffuse" / "ViewLayer.Combined".
    prefix: String,
    grey: bool,
    has_alpha: bool,
    /// The channels, as the file names them. Kept as the file's own `Text`, never round-tripped
    /// through a string.
    red: Text,
    green: Text,
    blue: Text,
    alpha: Option<Text>,
    sample_type: SampleType,
}

/// What the header says, decided before a pixel is read.
struct Admission {
    /// The header the sets come from.
    header_index: usize,
    sets: Vec<ColourSet>,
    width: usize,
    height: usize,
    /// Data window position relative to the display window.
    offset: (i64, i64),
    canvas: (u32, u32),
    primaries: Primaries,
    bit_depth: u16,
    layer_name: Option<String>,
    notes: Vec<(ReportKind, String)>,
}

fn text(channel: &Text) -> String {
    channel.to_string()
}

fn split_prefix(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(at) => (&name[..at], &name[at + 1..]),
        None => ("", name),
    }
}

/// The codecs the reader does not have, by name; `None` for every one it does.
fn compression_unsupported(compression: Compression) -> Option<&'static str> {
    match compression {
        Compression::DWAA(_) => Some("DWAA"),
        Compression::DWAB(_) => Some("DWAB"),
        Compression::HTJ2K32 => Some("HTJ2K (32 lines)"),
        Compression::HTJ2K256 => Some("HTJ2K (256 lines)"),
        _ => None,
    }
}

/// One channel of a header, as the set finder sees it.
struct Found {
    full: Text,
    sample_type: SampleType,
    /// Sampled at every pixel (not subsampled).
    dense: bool,
}

/// The complete colour sets in one header, main picture first. What was found but cannot be read
/// is said in `skipped`.
fn find_sets(header: &Header, skipped: &mut Vec<String>) -> Vec<ColourSet> {
    // prefix -> base name -> channel
    let mut by_prefix: BTreeMap<String, BTreeMap<String, Found>> = BTreeMap::new();
    for channel in &header.channels.list {
        let full = text(&channel.name);
        let (prefix, base) = split_prefix(&full);
        by_prefix.entry(prefix.to_string()).or_default().insert(
            base.to_string(),
            Found {
                full: channel.name.clone(),
                sample_type: channel.sample_type,
                dense: channel.sampling.x() == 1 && channel.sampling.y() == 1,
            },
        );
    }
    let mut sets = Vec::new();
    for (prefix, channels) in &by_prefix {
        let label = if prefix.is_empty() {
            "the main picture".to_string()
        } else {
            format!("\"{prefix}\"")
        };
        // A luminance/chroma image keeps its colour in RY and BY, at half resolution: a "Y" set
        // read alone would be the picture without its colour, so it is not read at all.
        if channels.contains_key("RY") && channels.contains_key("BY") {
            skipped.push(format!(
                "{label} is stored as luminance and chroma (Y, RY, BY), which PhotoForge does not read."
            ));
            continue;
        }
        let has = |naming: Naming| {
            [naming.red(), naming.green(), naming.blue()]
                .iter()
                .all(|base| channels.contains_key(*base))
        };
        let naming = if has(Naming::Short) {
            Some(Naming::Short)
        } else if has(Naming::Long) {
            Some(Naming::Long)
        } else {
            None
        };
        let (grey, red, green, blue, alpha_base) = match naming {
            Some(naming) => (
                false,
                &channels[naming.red()],
                &channels[naming.green()],
                &channels[naming.blue()],
                naming.alpha(),
            ),
            None => match channels.get("Y") {
                Some(y) => (true, y, y, y, "A"),
                None => continue,
            },
        };
        let alpha = channels.get(alpha_base);
        if !(red.dense && green.dense && blue.dense && alpha.is_none_or(|a| a.dense)) {
            skipped.push(format!(
                "{label} is stored subsampled, which PhotoForge does not read."
            ));
            continue;
        }
        if red.sample_type != green.sample_type || green.sample_type != blue.sample_type {
            skipped.push(format!(
                "{label} stores its red, green and blue with different sample types, which PhotoForge does not read."
            ));
            continue;
        }
        if red.sample_type == SampleType::U32 {
            skipped.push(format!(
                "{label} holds 32-bit integers (identifiers, not light), so it was not read as a picture."
            ));
            continue;
        }
        sets.push(ColourSet {
            prefix: prefix.clone(),
            grey,
            has_alpha: alpha.is_some(),
            red: red.full.clone(),
            green: green.full.clone(),
            blue: blue.full.clone(),
            alpha: alpha.map(|a| a.full.clone()),
            sample_type: red.sample_type,
        });
    }
    // The main picture (the empty prefix) first, then the rest in name order: the map's order.
    sets
}

/// A layer name from the file's own text: control characters replaced, no longer than the model
/// allows, never empty.
fn layer_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .take(MAX_LAYER_NAME_CHARS)
        .collect();
    if cleaned.trim().is_empty() {
        "Image".to_string()
    } else {
        cleaned
    }
}

fn display_name(set: &ColourSet, header_name: Option<&str>) -> String {
    layer_name(if !set.prefix.is_empty() {
        &set.prefix
    } else {
        header_name.unwrap_or("")
    })
}

/// Decides, from the header alone, what will be read and whether it can be.
fn admit(meta: &MetaData, limits: &ImportLimits) -> Result<Admission, AppError> {
    let mut notes: Vec<(ReportKind, String)> = Vec::new();
    if meta.headers.is_empty() {
        return Err(refuse("the file has no image in it"));
    }
    let mut chosen: Option<(usize, Vec<ColourSet>)> = None;
    let mut refusals: Vec<String> = Vec::new();
    let mut compression_refused: Option<(&'static str, String)> = None;
    let mut deep_parts = 0_usize;

    for (index, header) in meta.headers.iter().enumerate() {
        let part = header
            .own_attributes
            .layer_name
            .as_ref()
            .map(text)
            .map(|name| format!("\"{}\"", layer_name(&name)))
            .unwrap_or_else(|| format!("part {}", index + 1));
        if header.deep {
            deep_parts += 1;
            notes.push((
                ReportKind::NotImported,
                format!("{part} holds deep data (several samples at each pixel), which PhotoForge does not read."),
            ));
            continue;
        }
        if let Some(codec) = compression_unsupported(header.compression) {
            if compression_refused.is_none() {
                compression_refused = Some((codec, part.clone()));
            }
            notes.push((
                ReportKind::NotImported,
                format!("{part} is compressed with {codec}, which PhotoForge cannot read yet."),
            ));
            continue;
        }
        let mut skipped = Vec::new();
        let sets = find_sets(header, &mut skipped);
        if chosen.is_some() {
            // A later part: named, not read.
            if !sets.is_empty() {
                notes.push((
                    ReportKind::NotImported,
                    format!(
                        "{part} is another part of the file, with {} picture{}; only the first part with a picture is imported.",
                        sets.len(),
                        if sets.len() == 1 { "" } else { "s" }
                    ),
                ));
            }
            continue;
        }
        if sets.is_empty() {
            let names: Vec<String> = header
                .channels
                .list
                .iter()
                .map(|channel| layer_name(&text(&channel.name)))
                .collect();
            refusals.push(format!(
                "{part} has channels [{}] but none make a colour picture (R, G, B or Y).",
                names.join(", ")
            ));
            // Why the nearest candidates were not used, if there were any.
            refusals.extend(skipped);
            continue;
        }
        for line in skipped {
            notes.push((ReportKind::NotImported, line));
        }
        chosen = Some((index, sets));
    }

    let Some((header_index, mut sets)) = chosen else {
        if let Some((codec, part)) = compression_refused {
            return Err(refuse(format!(
                "{part} is compressed with {codec}, which PhotoForge cannot read yet. \
                 Save the file again with ZIP, PIZ or no compression."
            )));
        }
        if deep_parts == meta.headers.len() {
            return Err(refuse(
                "the file holds deep data (several samples at each pixel), which PhotoForge does not read",
            ));
        }
        return Err(refuse(if refusals.is_empty() {
            "the file holds no colour picture PhotoForge can read".to_string()
        } else {
            refusals.join(" ")
        }));
    };

    if sets.len() > MAX_SETS.min(MAX_LAYERS - 1) {
        let keep = MAX_SETS.min(MAX_LAYERS - 1);
        let dropped: Vec<String> = sets[keep..]
            .iter()
            .map(|set| display_name(set, None))
            .collect();
        notes.push((
            ReportKind::NotImported,
            format!(
                "The file has {} pictures; the first {keep} were imported and these were not: {}.",
                sets.len(),
                dropped.join(", ")
            ),
        ));
        sets.truncate(keep);
    }

    let header = &meta.headers[header_index];
    let data = header.data_window();
    let display = header.shared_attributes.display_window;
    let (width, height) = (data.size.x(), data.size.y());
    if width == 0 || height == 0 {
        return Err(refuse("the picture in the file is empty"));
    }
    let (canvas_width, canvas_height) = (display.size.x(), display.size.y());
    let canvas = if canvas_width == 0 || canvas_height == 0 {
        // No display window to speak of: the canvas is the picture.
        (width, height)
    } else {
        (canvas_width, canvas_height)
    };
    let hard = u64::from(crate::resources::HARD_MAX_CANVAS_DIMENSION);
    for (w, h) in [
        (width as u64, height as u64),
        (canvas.0 as u64, canvas.1 as u64),
    ] {
        if w > hard || h > hard {
            return Err(AppError::ImageTooLarge {
                pixels: w.saturating_mul(h),
                limit: hard * hard,
            });
        }
    }
    let (canvas_w, canvas_h) = (canvas.0 as u32, canvas.1 as u32);
    validate_dimensions(width as u32, height as u32)?;
    validate_dimensions(canvas_w, canvas_h)?;
    let pixels = width as u64 * height as u64;
    let canvas_pixels = u64::from(canvas_w) * u64::from(canvas_h);
    let working = crate::resources::max_working_pixels();
    if pixels > working || canvas_pixels > working {
        return Err(AppError::ImageTooLarge {
            pixels: pixels.max(canvas_pixels),
            limit: working,
        });
    }
    // One layer buffer is the picture's pixels at 16 bytes each; the raw samples are converted in
    // place, so that is also the peak for one set.
    let layer_bytes = pixels * 16;
    if layer_bytes > limits.entry_bytes {
        return Err(AppError::ResourceBudget {
            required: layer_bytes,
            limit: limits.entry_bytes,
        });
    }
    if layer_bytes > limits.store_bytes || layer_bytes > limits.job_bytes {
        return Err(refuse(format!(
            "the picture is {width} × {height} pixels and needs about {} MiB to import, more than the {} MiB PhotoForge will hold",
            layer_bytes / (1 << 20),
            limits.store_bytes.min(limits.job_bytes) / (1 << 20)
        )));
    }
    // Further sets that would not fit beside the first are left out, named.
    let mut fitting = sets.len();
    while fitting > 1 && layer_bytes * fitting as u64 > limits.store_bytes {
        fitting -= 1;
    }
    if fitting < sets.len() {
        let dropped: Vec<String> = sets[fitting..]
            .iter()
            .map(|set| display_name(set, None))
            .collect();
        notes.push((
            ReportKind::NotImported,
            format!(
                "These pictures would not fit in memory beside the others and were not imported: {}.",
                dropped.join(", ")
            ),
        ));
        sets.truncate(fitting);
    }

    let offset = (
        i64::from(data.position.x()) - i64::from(display.position.x()),
        i64::from(data.position.y()) - i64::from(display.position.y()),
    );
    let primaries = header
        .shared_attributes
        .chromaticities
        .map(|c| Primaries {
            red: (c.red.x(), c.red.y()),
            green: (c.green.x(), c.green.y()),
            blue: (c.blue.x(), c.blue.y()),
            white: (c.white.x(), c.white.y()),
        })
        .unwrap_or(REC709);

    let bit_depth = if sets.iter().all(|s| s.sample_type == SampleType::F16) {
        16
    } else {
        32
    };
    let aspect = header.shared_attributes.pixel_aspect;
    if aspect.is_finite() && (aspect - 1.0).abs() > 1e-3 {
        notes.push((
            ReportKind::Approximated,
            format!("The pixels are not square (aspect {aspect:.3}); they are shown square."),
        ));
    }
    Ok(Admission {
        header_index,
        sets,
        width,
        height,
        offset,
        canvas: (canvas_w, canvas_h),
        primaries,
        bit_depth,
        layer_name: header.own_attributes.layer_name.as_ref().map(text),
        notes,
    })
}

/// What turning premultiplied scene-linear samples into straight-alpha document colour found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Findings {
    /// Pixels with a sample that was not a number, or was infinite.
    pub non_finite: u64,
    /// Pixels with colour but no alpha: light added to the picture, which straight alpha cannot
    /// hold.
    pub emissive: u64,
    /// Pixels whose alpha was outside 0..1.
    pub alpha_clamped: u64,
    /// The alpha channel was zero at every pixel over colour that is there: a channel written and
    /// never filled. It was ignored, and the picture is opaque.
    pub alpha_ignored: bool,
}

/// Premultiplied → straight, non-finite samples replaced, alpha clamped, then the primaries.
///
/// `raw` holds the samples as the file had them; `has_alpha` says whether the alpha is the file's
/// own (otherwise the picture is opaque); `matrix` is `None` for Rec. 709.
pub(crate) fn finish(
    raw: &mut [FloatRgba],
    has_alpha: bool,
    matrix: Option<&[[f32; 3]; 3]>,
) -> Findings {
    let mut findings = Findings::default();
    // An alpha channel with no coverage anywhere, over colour that is there, is a file that wrote
    // the channel and never filled it (a conversion from a camera, say). Taken at its word the
    // picture would be invisible, so the channel is ignored and the picture is opaque.
    let has_alpha = if has_alpha
        && raw
            .iter()
            .all(|p| p.alpha.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater))
        && raw
            .iter()
            .any(|p| p.red != 0.0 || p.green != 0.0 || p.blue != 0.0)
    {
        findings.alpha_ignored = true;
        false
    } else {
        has_alpha
    };
    for pixel in raw.iter_mut() {
        let mut channels = [pixel.red, pixel.green, pixel.blue, pixel.alpha];
        let mut bad = false;
        for (index, value) in channels.iter_mut().enumerate() {
            // The alpha of a picture without one is the reader's default, not data.
            if index == 3 && !has_alpha {
                continue;
            }
            if value.is_nan() {
                *value = 0.0;
                bad = true;
            } else if value.is_infinite() {
                *value = if *value > 0.0 { HALF_MAX } else { -HALF_MAX };
                bad = true;
            }
        }
        if bad {
            findings.non_finite += 1;
        }
        let [mut r, mut g, mut b, a] = channels;
        let mut alpha = 1.0_f32;
        if has_alpha {
            if !(0.0..=1.0).contains(&a) {
                findings.alpha_clamped += 1;
            }
            alpha = a.clamp(0.0, 1.0);
            if a > 0.0 {
                // Divided by the alpha as written, so alpha above one keeps the colour it had.
                r /= a;
                g /= a;
                b /= a;
            } else {
                if r != 0.0 || g != 0.0 || b != 0.0 {
                    findings.emissive += 1;
                }
                r = 0.0;
                g = 0.0;
                b = 0.0;
            }
        }
        if let Some(m) = matrix {
            let (nr, ng, nb) = (
                m[0][0] * r + m[0][1] * g + m[0][2] * b,
                m[1][0] * r + m[1][1] * g + m[1][2] * b,
                m[2][0] * r + m[2][1] * g + m[2][2] * b,
            );
            r = nr;
            g = ng;
            b = nb;
        }
        let finite = |v: f32| {
            if v.is_finite() {
                v
            } else if v > 0.0 {
                f32::MAX
            } else if v < 0.0 {
                f32::MIN
            } else {
                0.0
            }
        };
        *pixel = FloatRgba::new(finite(r), finite(g), finite(b), alpha);
    }
    findings
}

/// Reads one colour set into a buffer of raw samples, as the file has them.
fn read_set(
    path: &Path,
    set: &ColourSet,
    width: usize,
    height: usize,
) -> Result<Vec<FloatRgba>, AppError> {
    use exr::prelude::*;
    // The reader looks the channels up by their full names, in the first part that has them all.
    // A set with no alpha asks for a name no file has, so that a *different* set's alpha is not
    // picked up by mistake; its default of one is what is used (and `finish` ignores the alpha of
    // a set that has none, besides).
    let no_alpha = Text::from("photoforge.no-alpha");
    let alpha_name = set.alpha.clone().unwrap_or(no_alpha);
    let pixels = if set.grey {
        read()
            .no_deep_data()
            .largest_resolution_level()
            .specific_channels()
            .required(set.red.clone())
            .optional(alpha_name, 1.0_f32)
            .collect_pixels(
                move |_resolution, _channels| vec![FloatRgba::TRANSPARENT; width * height],
                move |pixels: &mut Vec<FloatRgba>, position, (y, a): (f32, f32)| {
                    pixels[position.y() * width + position.x()] = FloatRgba::new(y, y, y, a);
                },
            )
            .first_valid_layer()
            .all_attributes()
            .from_file(path)
            .map(|image| image.layer_data.channel_data.pixels)
    } else {
        read()
            .no_deep_data()
            .largest_resolution_level()
            .specific_channels()
            .required(set.red.clone())
            .required(set.green.clone())
            .required(set.blue.clone())
            .optional(alpha_name, 1.0_f32)
            .collect_pixels(
                move |_resolution, _channels| vec![FloatRgba::TRANSPARENT; width * height],
                move |pixels: &mut Vec<FloatRgba>, position, (r, g, b, a): (f32, f32, f32, f32)| {
                    pixels[position.y() * width + position.x()] = FloatRgba::new(r, g, b, a);
                },
            )
            .first_valid_layer()
            .all_attributes()
            .from_file(path)
            .map(|image| image.layer_data.channel_data.pixels)
    };
    pixels.map_err(|error| refuse(format!("the OpenEXR file could not be decoded: {error}")))
}

/// Runs a step of the reader so that a panic in it on a hostile file is a refusal, not a crash of
/// the program. The reader is written not to panic (and every truncation and changed byte of a
/// file is tested not to); this is for the one we have not found.
fn guarded<T>(what: &str, step: impl FnOnce() -> Result<T, AppError>) -> Result<T, AppError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(step)) {
        Ok(result) => result,
        Err(_) => Err(refuse(format!(
            "the OpenEXR file could not be {what} safely: the reader stopped on it"
        ))),
    }
}

fn plural(count: u64) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

/// Reads an OpenEXR file into a document and its report.
pub fn import_exr(path: &Path, limits: ImportLimits) -> Result<ImportedExr, AppError> {
    let metadata = std::fs::metadata(path).map_err(|_| AppError::DecodeFailure)?;
    if !metadata.is_file() {
        return Err(AppError::UnsupportedImageFormat);
    }
    if metadata.len() > MAX_EXR_BYTES {
        return Err(refuse(format!(
            "the file is {} bytes; the largest OpenEXR file read is {MAX_EXR_BYTES}",
            metadata.len()
        )));
    }
    // The header first. This reads attributes and channel lists only; no pixel is decoded until
    // what the header claims has been checked against the budget.
    let meta = guarded("read", || {
        MetaData::read_from_file(path, false).map_err(|error| {
            refuse(format!(
                "the file is not an OpenEXR image that can be read: {error}"
            ))
        })
    })?;
    let admission = admit(&meta, &limits)?;
    let header = &meta.headers[admission.header_index];

    let mut report = ExrImportReport {
        format: "OpenEXR".to_string(),
        width: admission.canvas.0,
        height: admission.canvas.1,
        bit_depth: admission.bit_depth,
        color_mode: if admission.sets.iter().all(|set| set.grey) {
            "Grayscale"
        } else {
            "RGB"
        }
        .to_string(),
        ..ExrImportReport::default()
    };
    for (kind, detail) in &admission.notes {
        report.push(*kind, None, detail.clone());
    }

    let matrix = if admission.primaries.is_rec709() {
        None
    } else {
        match admission.primaries.to_working_space() {
            Some(matrix) => {
                report.push(
                    ReportKind::Note,
                    None,
                    format!(
                        "The file's colour primaries are not Rec. 709/sRGB (red {:.3}, {:.3}; white {:.4}, {:.4}). The colour was converted to sRGB's primaries with the white point adapted (Bradford); colour outside sRGB is kept in the high-precision document.",
                        admission.primaries.red.0,
                        admission.primaries.red.1,
                        admission.primaries.white.0,
                        admission.primaries.white.1
                    ),
                );
                Some(matrix)
            }
            None => {
                report.push(
                    ReportKind::Approximated,
                    None,
                    "The file's colour primaries do not describe a colour space, so the colour was read as Rec. 709/sRGB.",
                );
                None
            }
        }
    };

    let now = SystemClock.now();
    let mut document = LayerDocument::new(admission.canvas.0, admission.canvas.1);
    document.precision = DocumentPrecision::LinearSrgbF32;
    let mut layers: Vec<Layer> = Vec::new();
    let mut linear_pixels: Vec<(String, FloatImage)> = Vec::new();
    let mut totals = Findings::default();

    for (index, set) in admission.sets.iter().enumerate() {
        let mut raw = guarded("decoded", || {
            read_set(path, set, admission.width, admission.height)
        })?;
        let name = display_name(set, admission.layer_name.as_deref());
        let found = finish(&mut raw, set.has_alpha, matrix.as_ref());
        totals.non_finite += found.non_finite;
        totals.emissive += found.emissive;
        totals.alpha_clamped += found.alpha_clamped;
        totals.alpha_ignored |= found.alpha_ignored;
        let image = FloatImage::new(admission.width as u32, admission.height as u32, raw)?;
        let pixel_id = format!("px{}", index + 1);
        linear_pixels.push((pixel_id.clone(), image));
        let mut layer = new_layer(
            &format!("l{}", index + 1),
            &name,
            LayerContent::Pixel {
                pixel_id,
                width: admission.width as u32,
                height: admission.height as u32,
            },
            &now,
        );
        layer.transform.translate_x = admission.offset.0 as f32;
        layer.transform.translate_y = admission.offset.1 as f32;
        // The first picture is the picture; the rest are passes, there to be switched on.
        layer.visible = index == 0;
        layers.push(layer);
        if set.grey {
            report.push(
                ReportKind::Note,
                Some(&name),
                "A single luminance channel; it was imported as a grey picture.",
            );
        }
        if index > 0 {
            report.push(
                ReportKind::Note,
                Some(&name),
                "A further picture in the same file (a pass); it is imported hidden.",
            );
        }
    }
    report.layers_imported = layers.len();

    // Everything in the header that was not part of a colour set, named once.
    let mut used: Vec<&Text> = Vec::new();
    for set in &admission.sets {
        used.push(&set.red);
        used.push(&set.green);
        used.push(&set.blue);
        if let Some(alpha) = &set.alpha {
            used.push(alpha);
        }
    }
    let leftover: Vec<String> = header
        .channels
        .list
        .iter()
        .filter(|channel| !used.contains(&&channel.name))
        .map(|channel| layer_name(&text(&channel.name)))
        .collect();
    if !leftover.is_empty() {
        let shown: Vec<String> = leftover.iter().take(12).cloned().collect();
        report.push(
            ReportKind::NotImported,
            None,
            format!(
                "{} other channel{} not read as colour: {}{}.",
                leftover.len(),
                plural(leftover.len() as u64),
                shown.join(", "),
                if leftover.len() > shown.len() {
                    ", …"
                } else {
                    ""
                }
            ),
        );
    }
    if totals.non_finite > 0 {
        report.push(
            ReportKind::Approximated,
            None,
            format!(
                "{} pixel{} held a value that is not a number or is infinite; not-a-number became zero and infinity became the brightest finite half-float value.",
                totals.non_finite,
                plural(totals.non_finite)
            ),
        );
    }
    if totals.alpha_ignored {
        report.push(
            ReportKind::Approximated,
            None,
            "The file's alpha channel is zero at every pixel while the colour is not, so the alpha was ignored and the picture is opaque.",
        );
    }
    if totals.alpha_clamped > 0 {
        report.push(
            ReportKind::Approximated,
            None,
            format!(
                "{} pixel{} had alpha outside 0 to 1, which was clamped.",
                totals.alpha_clamped,
                plural(totals.alpha_clamped)
            ),
        );
    }
    if totals.emissive > 0 {
        report.push(
            ReportKind::Approximated,
            None,
            format!(
                "{} pixel{} carry light with no alpha (added to the picture, not covering it); a document with straight alpha cannot hold that, so the light was dropped.",
                totals.emissive,
                plural(totals.emissive)
            ),
        );
    }
    let data = header.data_window();
    let display = header.shared_attributes.display_window;
    if admission.offset != (0, 0)
        || (data.size.x(), data.size.y()) != (display.size.x(), display.size.y())
    {
        report.push(
            ReportKind::Note,
            None,
            format!(
                "The picture's data window ({} × {} at {}, {}) is not the display window ({} × {}); the canvas is the display window and the picture sits where the file puts it.",
                data.size.x(),
                data.size.y(),
                data.position.x(),
                data.position.y(),
                display.size.x(),
                display.size.y()
            ),
        );
    }
    report.push(
        ReportKind::Note,
        None,
        format!(
            "Read as scene-linear light with premultiplied alpha, stored as {}, into a high-precision document.",
            if admission.bit_depth == 16 {
                "half float (16-bit)"
            } else {
                "single precision (32-bit)"
            }
        ),
    );

    document.active_layer_id = layers.first().map(|layer| layer.id.clone());
    document.layers = layers;
    document.validate()?;

    let project = LoadedProject {
        document,
        document_operations: Vec::new(),
        pixels: Vec::new(),
        linear_pixels,
        application_version: env!("CARGO_PKG_VERSION").into(),
        created_at: now.clone(),
        modified_at: now,
        source_metadata: None,
    };
    Ok(ImportedExr { project, report })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod finish_tests {
    use super::*;

    fn run(pixels: &[[f32; 4]], has_alpha: bool) -> (Vec<FloatRgba>, Findings) {
        let mut raw: Vec<FloatRgba> = pixels
            .iter()
            .map(|p| FloatRgba::new(p[0], p[1], p[2], p[3]))
            .collect();
        let found = finish(&mut raw, has_alpha, None);
        (raw, found)
    }

    #[test]
    fn premultiplied_colour_is_divided_by_its_alpha() {
        let (out, found) = run(&[[0.25, 0.125, 0.0625, 0.5]], true);
        assert_eq!(out[0], FloatRgba::new(0.5, 0.25, 0.125, 0.5));
        assert_eq!(found, Findings::default());
    }

    #[test]
    fn a_file_without_alpha_is_left_as_it_is_and_opaque() {
        // Alpha reads as its default of one; no division, whatever the sample says.
        let (out, found) = run(&[[3.0, 0.5, -0.25, 0.5]], false);
        assert_eq!(out[0], FloatRgba::new(3.0, 0.5, -0.25, 1.0));
        assert_eq!(found, Findings::default());
    }

    #[test]
    fn colour_brighter_than_white_and_below_black_survives() {
        let (out, _) = run(&[[40.0, -0.5, 1000.0, 1.0]], true);
        assert_eq!(out[0], FloatRgba::new(40.0, -0.5, 1000.0, 1.0));
    }

    #[test]
    fn light_with_no_alpha_is_counted_and_dropped() {
        let (out, found) = run(
            &[
                [2.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 0.0, 0.0],
                [0.5, 0.5, 0.5, 1.0],
            ],
            true,
        );
        assert_eq!(out[0], FloatRgba::new(0.0, 0.0, 0.0, 0.0));
        assert_eq!(out[1], FloatRgba::new(0.0, 0.0, 0.0, 0.0));
        assert_eq!(out[2], FloatRgba::new(0.5, 0.5, 0.5, 1.0));
        assert_eq!(found.emissive, 1);
        assert!(!found.alpha_ignored);
    }

    #[test]
    fn an_alpha_channel_that_was_never_filled_is_ignored_not_obeyed() {
        // Colour everywhere, alpha zero everywhere: not a transparent picture, a channel nobody
        // wrote to.
        let (out, found) = run(&[[0.25, 0.5, 0.75, 0.0], [1.0, 2.0, 3.0, 0.0]], true);
        assert_eq!(out[0], FloatRgba::new(0.25, 0.5, 0.75, 1.0));
        assert_eq!(out[1], FloatRgba::new(1.0, 2.0, 3.0, 1.0));
        assert!(found.alpha_ignored);
        assert_eq!(found.emissive, 0);
        // A picture with nothing in it at all stays as it is: empty and transparent.
        let (out, found) = run(&[[0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0]], true);
        assert_eq!(out[0], FloatRgba::new(0.0, 0.0, 0.0, 0.0));
        assert!(!found.alpha_ignored);
    }

    #[test]
    fn alpha_outside_zero_to_one_is_clamped_and_counted() {
        let (out, found) = run(&[[2.0, 2.0, 2.0, 2.0], [0.5, 0.5, 0.5, -0.25]], true);
        // Above one: the colour is divided by the alpha written, then alpha is one.
        assert_eq!(out[0], FloatRgba::new(1.0, 1.0, 1.0, 1.0));
        // Below zero: no coverage, so no colour.
        assert_eq!(out[1], FloatRgba::new(0.0, 0.0, 0.0, 0.0));
        assert_eq!(found.alpha_clamped, 2);
    }

    #[test]
    fn samples_that_are_not_numbers_are_replaced_and_counted() {
        let (out, found) = run(
            &[
                [f32::NAN, 0.5, 0.5, 1.0],
                [f32::INFINITY, f32::NEG_INFINITY, 0.0, 1.0],
                [0.1, 0.2, 0.3, f32::NAN],
            ],
            true,
        );
        assert_eq!(out[0], FloatRgba::new(0.0, 0.5, 0.5, 1.0));
        assert_eq!(out[1], FloatRgba::new(65_504.0, -65_504.0, 0.0, 1.0));
        // A not-a-number alpha is no coverage.
        assert_eq!(out[2], FloatRgba::new(0.0, 0.0, 0.0, 0.0));
        assert_eq!(found.non_finite, 3);
        assert!(out.iter().all(|p| p.is_finite()));
    }

    #[test]
    fn a_tiny_alpha_cannot_make_the_colour_infinite() {
        let (out, _) = run(&[[1.0e30, 1.0e30, 1.0e30, 1.0e-30]], true);
        assert!(out[0].is_finite());
    }

    #[test]
    fn the_primaries_matrix_is_applied_to_straight_colour() {
        let mut raw = vec![FloatRgba::new(0.5, 0.25, 0.125, 0.5)];
        let matrix = [[0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]];
        finish(&mut raw, true, Some(&matrix));
        // Straight colour is (1, 0.5, 0.25); the matrix rotates the channels.
        assert_eq!(raw[0], FloatRgba::new(0.5, 0.25, 1.0, 0.5));
    }

    #[test]
    fn a_panic_in_the_reader_is_a_refusal_not_a_crash() {
        let result: Result<(), AppError> = guarded("decoded", || panic!("a reader bug"));
        let message = result.unwrap_err().to_string();
        assert!(message.contains("could not be decoded safely"), "{message}");
        // A step that does not panic is untouched.
        assert_eq!(guarded("read", || Ok(7)).unwrap(), 7);
        assert!(guarded::<u8>("read", || Err(refuse("no"))).is_err());
    }

    #[test]
    fn names_from_a_file_are_made_safe() {
        assert_eq!(layer_name("beauty"), "beauty");
        assert_eq!(layer_name(""), "Image");
        assert_eq!(layer_name("  \u{1}\u{2} "), "  __ ");
        let long = "x".repeat(500);
        assert_eq!(layer_name(&long).chars().count(), MAX_LAYER_NAME_CHARS);
    }
}
