//! Turning a read Photoshop document into a PhotoForge layered document, and saying what
//! did not survive.
//!
//! The rule is that nothing is dropped silently. Every layer the file holds is either
//! imported as it is, imported *approximately* (and the entry says how), or *not imported*
//! (and the entry says why). The report is part of the result, not a log.
//!
//! What maps exactly: pixel layers, their position, opacity (with fill opacity), visibility,
//! sixteen blend modes, groups (isolated and pass-through, open and closed), raster layer
//! and group masks, names (Unicode). What is approximated: text, smart-object, shape and
//! fill layers (imported as the pixels Photoshop stored for them), vector masks (as their
//! rendered coverage), clipping (kept as a live relation; see below for what differs).
//! What is not imported: layer styles, adjustment layers, blend modes PhotoForge does not
//! have (the layer is imported as Normal and listed), artboards, and everything else the
//! reader only recorded the presence of.
//!
//! # Clipping
//!
//! A layer clipped to the one beneath it is drawn only where that layer is. PhotoForge's
//! layer model has the same relation, so a clipped layer is imported *as clipped*: it
//! stays attached to its base, moving or hiding the base moves or hides it, and its pixels
//! are exactly the ones Photoshop stored.
//!
//! What does not carry over exactly: a clipped layer whose base could not be imported (a
//! fill or adjustment layer with no pixels) has nothing to attach to and is imported
//! unclipped; a pass-through group that is a clipping base is imported as an isolated one;
//! and a base with "Blend Clipped Layers as Group" switched off is still composited as one
//! group. The report names each.
//!
//! # Colour
//!
//! 8-bit files whose profile is absent or equal to sRGB stay 8-bit. 16- and 32-bit files,
//! and 8-bit files with another RGB profile, become high-precision (linear float) documents
//! and are converted through the embedded profile — nothing is truncated to 8 bits and
//! nothing is converted without being named in the report. A 32-bit file without a profile
//! is read as linear light, which is what Photoshop's 32-bit mode is.
use super::blend::{blend_from_key, describe_block_key, BlendKey};
use super::document::{ColorMode, Divider, Document, LayerRecord, Rect};
use super::pixels::{decode_channel, decode_merged_image, ChannelSpec, MergedSpec};
use super::reader::Reader;
use super::tree::{build_tree, Node};
use crate::color::{srgb_decode, FloatImage, FloatRgba};
use crate::color_management::IccRowTransform;
use crate::error::AppError;
use crate::layers::{
    BlendMode, Layer, LayerContent, LayerDocument, LayerMask, LoadedProject, MAX_GROUP_DEPTH,
    MAX_LAYERS,
};
use crate::mask::{MaskBitmap, MaskSnapshot, MAX_MASK_PIXELS};
use crate::operations::structure::new_layer;
use crate::operations::support::{Clock, SystemClock};
use crate::pixel::DocumentPrecision;
use crate::resources::HARD_MAX_CANVAS_DIMENSION;
use image::{Rgba, RgbaImage};
use serde::Serialize;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportKind {
    /// Imported, but not exactly as Photoshop shows it.
    Approximated,
    /// Left out of a document being opened.
    NotImported,
    /// Left out of a file being written.
    NotExported,
    /// Worth knowing, and neither of those.
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportEntry {
    pub kind: ReportKind,
    /// The layer it is about, if it is about one.
    pub layer: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    /// "PSD" or "PSB".
    pub format: String,
    pub width: u32,
    pub height: u32,
    pub bit_depth: u16,
    pub color_mode: String,
    /// Layers (not groups) that became layers.
    pub layers_imported: usize,
    pub groups_imported: usize,
    pub entries: Vec<ReportEntry>,
}

impl ImportReport {
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

pub struct Imported {
    pub project: LoadedProject,
    pub report: ImportReport,
}

/// How much the import may hold, from the resource manager.
#[derive(Debug, Clone, Copy)]
pub struct ImportLimits {
    /// Pixel memory for every layer buffer together.
    pub store_bytes: u64,
    /// One buffer.
    pub entry_bytes: u64,
    /// The most one job may peak at: a layer is decoded, converted and composed in memory
    /// that is not the buffer it ends as, and a layer that would not fit is left out.
    pub job_bytes: u64,
}

impl ImportLimits {
    pub fn from_resources() -> Self {
        Self {
            store_bytes: crate::resources::max_store_bytes(),
            entry_bytes: crate::resources::max_entry_bytes(),
            job_bytes: crate::resources::max_job_bytes(),
        }
    }
}

fn refuse(message: impl Into<String>) -> AppError {
    AppError::PsdFile(message.into())
}

/// Reads a PSD or PSB from `path` into a document and its report.
pub fn import_psd(path: &Path, limits: ImportLimits) -> Result<Imported, AppError> {
    let file = File::open(path).map_err(|_| AppError::Permission)?;
    let document = super::read_document(file).map_err(AppError::from)?;
    let file = File::open(path).map_err(|_| AppError::Permission)?;
    let mut reader = Reader::new(file).map_err(AppError::from)?;
    Importer::new(&document, limits)?.run(&mut reader)
}

/// A colour transform plan, decided once per file.
struct ColourPlan {
    depth: u16,
    transform: Option<IccRowTransform>,
    /// The document is high precision (linear float), not 8-bit sRGB.
    float: bool,
    /// What was decided, for the report.
    description: String,
}

/// Whether an embedded RGB profile behaves as sRGB, by transforming a few colours and
/// comparing with the sRGB transfer function. Profiles are often "sRGB IEC61966-2.1" and
/// sometimes a v4 equivalent; both pass, and a wide-gamut or gamma-2.2 profile does not.
fn is_srgb_equivalent(transform: &IccRowTransform) -> bool {
    let samples: [[f32; 3]; 8] = [
        [0.0, 0.0, 0.0],
        [1.0, 1.0, 1.0],
        [0.5, 0.5, 0.5],
        [0.25, 0.25, 0.25],
        [0.75, 0.75, 0.75],
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
    ];
    let mut encoded = Vec::new();
    for sample in samples {
        encoded.extend_from_slice(&[sample[0], sample[1], sample[2], 1.0]);
    }
    let mut linear = vec![0.0_f32; encoded.len()];
    if transform.apply(&encoded, &mut linear).is_err() {
        return false;
    }
    samples.iter().enumerate().all(|(index, sample)| {
        (0..3).all(|channel| {
            (linear[index * 4 + channel] - srgb_decode(sample[channel])).abs() < 0.004
        })
    })
}

fn plan_colour(document: &Document) -> Result<ColourPlan, AppError> {
    let depth = document.header.depth;
    let transform = match &document.resources.icc_profile {
        Some(bytes) => IccRowTransform::new(bytes).ok(),
        None => None,
    };
    let profile_present = document.resources.icc_profile.is_some();
    let srgb = transform.as_ref().is_some_and(is_srgb_equivalent);
    let (transform, description) = match (depth, transform, srgb) {
        (8, _, true) | (8, None, _) => (None, "8-bit sRGB".to_string()),
        (8, Some(transform), false) => (
            Some(transform),
            "8-bit with an embedded profile, converted to a high-precision document".to_string(),
        ),
        (16, Some(transform), false) => (
            Some(transform),
            "16-bit with an embedded profile, converted to a high-precision document".to_string(),
        ),
        (16, _, _) => (
            None,
            "16-bit sRGB, as a high-precision document".to_string(),
        ),
        (32, Some(transform), _) => (
            Some(transform),
            "32-bit, converted through its embedded profile to a high-precision document"
                .to_string(),
        ),
        (32, None, _) => (
            None,
            "32-bit without a profile, read as linear light".to_string(),
        ),
        _ => (None, "sRGB".to_string()),
    };
    let description = if profile_present && description.starts_with("8-bit sRGB") {
        "8-bit sRGB (the embedded profile is equivalent to sRGB)".to_string()
    } else {
        description
    };
    Ok(ColourPlan {
        depth,
        float: depth != 8 || transform.is_some(),
        transform,
        description,
    })
}

enum Pixels {
    Rgba8(RgbaImage),
    Float(FloatImage),
}

/// A layer mask cut to a grid, with its flags.
struct BuiltMask {
    bitmap: MaskBitmap,
    enabled: bool,
    inverted: bool,
}

struct Importer<'a> {
    document: &'a Document,
    limits: ImportLimits,
    plan: ColourPlan,
    report: ImportReport,
    clock: SystemClock,
    next_layer: u32,
    next_pixel: u32,
    bytes_used: u64,
    pixels: Vec<(String, RgbaImage)>,
    linear_pixels: Vec<(String, FloatImage)>,
}

impl<'a> Importer<'a> {
    fn new(document: &'a Document, limits: ImportLimits) -> Result<Self, AppError> {
        let header = &document.header;
        match header.mode {
            ColorMode::Rgb | ColorMode::Grayscale => {}
            other => {
                return Err(refuse(format!(
                    "This file is in {} colour mode, which PhotoForge cannot import yet. \
                     In Photoshop, choose Image > Mode > RGB Color and save it again.",
                    mode_name(other)
                )));
            }
        }
        if !matches!(header.depth, 8 | 16 | 32) {
            return Err(refuse(format!(
                "This file is {} bits per channel, which PhotoForge cannot import. \
                 In Photoshop, choose Image > Mode > 8 Bits/Channel and save it again.",
                header.depth
            )));
        }
        if header.width > HARD_MAX_CANVAS_DIMENSION || header.height > HARD_MAX_CANVAS_DIMENSION {
            return Err(AppError::ImageTooLarge {
                pixels: u64::from(header.width) * u64::from(header.height),
                limit: u64::from(HARD_MAX_CANVAS_DIMENSION) * u64::from(HARD_MAX_CANVAS_DIMENSION),
            });
        }
        let plan = plan_colour(document)?;
        let report = ImportReport {
            format: match header.version {
                super::Version::Psd => "PSD".into(),
                super::Version::Psb => "PSB".into(),
            },
            width: header.width,
            height: header.height,
            bit_depth: header.depth,
            color_mode: mode_name(header.mode).into(),
            ..ImportReport::default()
        };
        Ok(Self {
            document,
            limits,
            plan,
            report,
            clock: SystemClock,
            next_layer: 0,
            next_pixel: 0,
            bytes_used: 0,
            pixels: Vec::new(),
            linear_pixels: Vec::new(),
        })
    }

    fn run<R: std::io::Read + std::io::Seek>(
        mut self,
        reader: &mut Reader<R>,
    ) -> Result<Imported, AppError> {
        let source = self.document;
        let header = &source.header;
        let (width, height) = (header.width, header.height);
        let mut document = LayerDocument::new(width, height);
        document.precision = if self.plan.float {
            DocumentPrecision::LinearSrgbF32
        } else {
            DocumentPrecision::LegacySrgb8
        };
        self.report
            .push(ReportKind::Note, None, self.plan.description.clone());

        let records = &source.layers.records;
        if records.is_empty() {
            let layer = self.import_flat_image(reader)?;
            document.active_layer_id = Some(layer.id.clone());
            document.layers.push(layer);
            self.report.layers_imported = 1;
            self.report.push(
                ReportKind::Note,
                None,
                "The file has no layers; its flattened image was imported as one layer.",
            );
        } else {
            let tree = build_tree(records, MAX_LAYERS, MAX_GROUP_DEPTH).map_err(AppError::from)?;
            let mut layers = Vec::new();
            self.import_nodes(reader, &tree, &mut layers)?;
            if layers.is_empty() {
                // Every layer was pixel-less (adjustment layers on nothing): the flattened image
                // is the picture, so that is what is imported.
                let layer = self.import_flat_image(reader).map_err(|error| {
                    refuse(format!(
                        "None of this file's layers hold pixels, and its flattened image could not be used instead ({error})."
                    ))
                })?;
                self.report.layers_imported = 1;
                self.report.push(
                    ReportKind::Note,
                    None,
                    "None of the file's layers hold pixels; its flattened image was imported as one layer.",
                );
                layers.push(layer);
            }
            document.active_layer_id = layers.last().map(|layer| layer.id.clone());
            document.layers = layers;
        }
        document.validate()?;

        // The photograph's own EXIF, XMP and IPTC. The pixels of a Photoshop file are upright
        // whatever an orientation tag in its EXIF says, so that tag is treated as spent.
        let resources = &self.document.resources;
        let mut source_metadata = crate::metadata::SourceMetadata::from_blocks(
            resources.exif.clone(),
            resources.xmp.clone(),
            resources.iptc.clone(),
        );
        source_metadata.orientation_applied = true;
        let source_metadata = (!source_metadata.is_empty()).then_some(source_metadata);
        let now = self.clock.now();
        let project = LoadedProject {
            document,
            document_operations: Vec::new(),
            pixels: self.pixels,
            linear_pixels: self.linear_pixels,
            application_version: env!("CARGO_PKG_VERSION").into(),
            created_at: now.clone(),
            modified_at: now,
            source_metadata,
        };
        Ok(Imported {
            project,
            report: self.report,
        })
    }

    fn fresh_layer_id(&mut self) -> String {
        self.next_layer += 1;
        format!("l{}", self.next_layer)
    }

    fn fresh_pixel_id(&mut self) -> String {
        self.next_pixel += 1;
        format!("px{}", self.next_pixel)
    }

    /// Accounts for `bytes` of pixel memory, refusing the import if the budget is spent.
    fn reserve(&mut self, bytes: u64, what: &str) -> Result<(), AppError> {
        if bytes > self.limits.entry_bytes {
            return Err(AppError::ResourceBudget {
                required: bytes,
                limit: self.limits.entry_bytes,
            });
        }
        self.bytes_used = self.bytes_used.saturating_add(bytes);
        if self.bytes_used > self.limits.store_bytes {
            return Err(refuse(format!(
                "This file's layers need about {} MiB of memory ({what}), more than the {} MiB \
                 PhotoForge will hold. Merge or delete layers in Photoshop and save a smaller copy.",
                self.bytes_used / (1 << 20),
                self.limits.store_bytes / (1 << 20)
            )));
        }
        Ok(())
    }

    fn bytes_per_pixel(&self) -> u64 {
        if self.plan.float {
            16
        } else {
            4
        }
    }

    fn import_nodes<R: std::io::Read + std::io::Seek>(
        &mut self,
        reader: &mut Reader<R>,
        nodes: &[Node],
        out: &mut Vec<Layer>,
    ) -> Result<(), AppError> {
        let document = self.document;
        let clipped_node = |node: &Node| match node {
            Node::Layer(index) => document.layers.records[*index].clipped,
            Node::Group { record, .. } => document.layers.records[*record].clipped,
        };
        // Whether something imported in this stack, and not itself clipped, is there for the
        // next clipped layer to attach to.
        let mut base_present = false;
        for (position, node) in nodes.iter().enumerate() {
            let clipped = clipped_node(node);
            let followed_by_clipped = nodes.get(position + 1).is_some_and(&clipped_node);
            let record_index = match node {
                Node::Layer(index) => *index,
                Node::Group { record, .. } => *record,
            };
            let record = &document.layers.records[record_index];
            if clipped && !base_present {
                self.report.push(
                    ReportKind::Approximated,
                    Some(record.display_name()),
                    "Clipped to a layer that could not be imported, so there is nothing to clip it to; imported unclipped.",
                );
            }
            if followed_by_clipped && !record.blend_clipped_as_group {
                self.report.push(
                    ReportKind::Approximated,
                    Some(record.display_name()),
                    "\"Blend Clipped Layers as Group\" is switched off for this layer; PhotoForge composites a layer and the layers clipped to it as one group, so their blending may differ.",
                );
            }
            let imported = match node {
                Node::Layer(index) => self.import_pixel_layer(reader, *index)?,
                Node::Group {
                    record: group_index,
                    children,
                } => {
                    let mut layer = self.import_group(reader, *group_index, children)?;
                    if followed_by_clipped && layer.is_pass_through() {
                        // A pass-through group has no buffer of its own, so there is nothing
                        // for the layers clipped to it to take their shape from.
                        if let LayerContent::Group { isolated, .. } = &mut layer.content {
                            *isolated = true;
                        }
                        self.report.push(
                            ReportKind::Approximated,
                            Some(record.display_name()),
                            "Layers are clipped to this pass-through group, so it was imported as an isolated group.",
                        );
                    }
                    Some(layer)
                }
            };
            match imported {
                Some(mut layer) => {
                    if clipped && base_present {
                        layer.clipped = true;
                    }
                    if !clipped {
                        base_present = true;
                    }
                    out.push(layer);
                }
                // A base that could not be imported leaves nothing to attach to.
                None if !clipped => base_present = false,
                None => {}
            }
        }
        Ok(())
    }

    fn describe_keys(&mut self, record: &LayerRecord) {
        let name = record.display_name().to_string();
        let mut effects = false;
        let mut seen: Vec<&'static str> = Vec::new();
        for key in &record.block_keys {
            if let Some(what) = describe_block_key(key) {
                if seen.contains(&what) {
                    continue;
                }
                seen.push(what);
                match what {
                    "layer effects (styles)" => effects = true,
                    "editable text" => self.report.push(
                        ReportKind::Approximated,
                        Some(&name),
                        "Text layer imported as the pixels Photoshop stored; it is no longer editable text.",
                    ),
                    "smart object" => self.report.push(
                        ReportKind::Approximated,
                        Some(&name),
                        "Smart object imported as its rendered pixels; its source is not kept.",
                    ),
                    "vector mask" => self.report.push(
                        ReportKind::Approximated,
                        Some(&name),
                        "Vector mask imported as the pixels it produced; it is no longer editable.",
                    ),
                    "fill layer" => self.report.push(
                        ReportKind::Approximated,
                        Some(&name),
                        "Fill layer imported as its rendered pixels.",
                    ),
                    "artboard" => self.report.push(
                        ReportKind::NotImported,
                        Some(&name),
                        "Artboards are not supported; the contents were placed on the one canvas.",
                    ),
                    "linked file" => self.report.push(
                        ReportKind::Approximated,
                        Some(&name),
                        "Linked file imported as pixels; the link is not kept.",
                    ),
                    _ => {}
                }
            }
        }
        if effects {
            self.report.push(
                ReportKind::NotImported,
                Some(&name),
                "Layer styles (drop shadow, stroke, glow and so on) were not imported; the layer appears without them.",
            );
        }
    }

    /// The blend mode and pass-through flag for a record.
    fn blend_of(&mut self, record: &LayerRecord, is_group: bool) -> (BlendMode, bool) {
        let key = if is_group {
            record.divider_blend_key.unwrap_or(record.blend_key)
        } else {
            record.blend_key
        };
        match blend_from_key(&key) {
            BlendKey::Supported(mode) => (mode, false),
            BlendKey::PassThrough => {
                if is_group {
                    (BlendMode::Normal, true)
                } else {
                    (BlendMode::Normal, false)
                }
            }
            BlendKey::Unsupported(name) => {
                self.report.push(
                    ReportKind::Approximated,
                    Some(record.display_name()),
                    format!("Blend mode {name} is not available in PhotoForge; the layer was imported as Normal."),
                );
                (BlendMode::Normal, false)
            }
            BlendKey::Unknown => {
                self.report.push(
                    ReportKind::Approximated,
                    Some(record.display_name()),
                    "An unrecognised blend mode was imported as Normal.",
                );
                (BlendMode::Normal, false)
            }
        }
    }

    fn name_of(&mut self, record: &LayerRecord, fallback: &str) -> String {
        let cleaned: String = record
            .display_name()
            .chars()
            .filter(|character| !character.is_control())
            .take(120)
            .collect();
        let trimmed = cleaned.trim();
        if trimmed.is_empty() {
            fallback.to_string()
        } else {
            trimmed.to_string()
        }
    }

    fn opacity_of(record: &LayerRecord) -> f32 {
        let opacity = f32::from(record.opacity) / 255.0;
        let fill = record
            .fill_opacity
            .map_or(1.0, |fill| f32::from(fill) / 255.0);
        (opacity * fill).clamp(0.0, 1.0)
    }

    fn import_group<R: std::io::Read + std::io::Seek>(
        &mut self,
        reader: &mut Reader<R>,
        record_index: usize,
        children: &[Node],
    ) -> Result<Layer, AppError> {
        let document = self.document;
        let record = &document.layers.records[record_index];
        let mut members = Vec::new();
        self.import_nodes(reader, children, &mut members)?;
        let name = self.name_of(record, "Group");
        let (blend, pass_through) = self.blend_of(record, true);
        self.describe_keys(record);
        // Fill opacity on a group is not the same thing as opacity in Photoshop once the
        // group's layers blend with what is beneath: the picture it stores for a pass-through
        // group at 47% fill is not what a cross-fade at 47% gives. Measured on a file made
        // for the case (psd-tools' passthrough_fill_blendmode.psd, mean error 3 levels), and
        // not understood well enough to reproduce, so it is reported rather than guessed at.
        fn blends(layers: &[Layer]) -> bool {
            layers.iter().any(|layer| {
                layer.blend_mode != BlendMode::Normal
                    || layer.is_pass_through()
                    || blends(layer.children())
            })
        }
        if record.fill_opacity.is_some_and(|fill| fill < 255) && (pass_through || blends(&members))
        {
            self.report.push(
                ReportKind::Approximated,
                Some(&name),
                "This group has a fill opacity below 100% and its layers blend with what is beneath; it was imported with the fill applied as opacity, which Photoshop does not do exactly.",
            );
        }
        let id = self.fresh_layer_id();
        let now = self.clock.now();
        let mut layer = new_layer(
            &id,
            &name,
            LayerContent::Group {
                children: members,
                isolated: !pass_through,
            },
            &now,
        );
        layer.opacity = Self::opacity_of(record);
        layer.visible = record.visible();
        layer.collapsed = record.divider == Some(Divider::ClosedFolder);
        layer.blend_mode = if pass_through {
            BlendMode::Normal
        } else {
            blend
        };
        // A group's mask lives in canvas space.
        let header = &document.header;
        let canvas = Rect {
            top: 0,
            left: 0,
            bottom: header.height as i32,
            right: header.width as i32,
        };
        if let Some(mask) = self.build_mask(reader, record, canvas)? {
            layer.mask = Some(mask);
        }
        self.report.groups_imported += 1;
        Ok(layer)
    }

    /// The pixels of one layer and its mask, as a layer. `None` when nothing could be made.
    fn import_pixel_layer<R: std::io::Read + std::io::Seek>(
        &mut self,
        reader: &mut Reader<R>,
        index: usize,
    ) -> Result<Option<Layer>, AppError> {
        let document = self.document;
        let record = &document.layers.records[index];
        let name = self.name_of(record, &format!("Layer {}", index + 1));
        if record.rect.is_empty() {
            if let Some(handled) = self.import_adjustment(reader, record, &name)? {
                return Ok(handled);
            }
            self.report_pixelless(record, &name);
            return Ok(None);
        }
        let (width, height) = (record.rect.width(), record.rect.height());
        if width > u64::from(HARD_MAX_CANVAS_DIMENSION)
            || height > u64::from(HARD_MAX_CANVAS_DIMENSION)
        {
            self.report.push(
                ReportKind::NotImported,
                Some(&name),
                format!("The layer is {width} x {height} pixels, larger than PhotoForge allows; it was not imported."),
            );
            return Ok(None);
        }
        if crate::layers::validate_dimensions(width as u32, height as u32).is_err() {
            self.report.push(
                ReportKind::NotImported,
                Some(&name),
                format!("The layer is {width} x {height} pixels, more than PhotoForge's working size allows; it was not imported."),
            );
            return Ok(None);
        }
        // A layer is decoded to planes, widened to floats, cut by its mask and
        // composed, in memory that is not the buffer it ends as: about 24 bytes a pixel more.
        let pixels = width * height;
        let peak = pixels.saturating_mul(self.bytes_per_pixel() + 24);
        if peak > self.limits.job_bytes {
            self.report.push(
                ReportKind::NotImported,
                Some(&name),
                format!(
                    "The layer is {width} x {height} pixels and needs about {} MiB to import, more than the {} MiB one job may use; it was not imported.",
                    peak / (1 << 20),
                    self.limits.job_bytes / (1 << 20)
                ),
            );
            return Ok(None);
        }
        let layer_bytes = pixels * self.bytes_per_pixel();
        self.reserve(layer_bytes, &name)?;

        let header = &document.header;
        let depth = header.depth;
        let version = header.version;
        let max_plane = width * height * u64::from(depth / 8).max(1);
        let mut planes: HashMap<i16, Vec<u8>> = HashMap::new();
        for channel in &record.channels {
            // Colour (0..), transparency (-1); the mask channels are read with the mask.
            if channel.id < -1 {
                continue;
            }
            let spec = ChannelSpec {
                offset: channel.offset,
                length: channel.length,
                width,
                height,
                depth,
            };
            let plane =
                decode_channel(reader, version, &spec, max_plane).map_err(AppError::from)?;
            planes.insert(channel.id, plane);
        }
        let samples = (width * height) as usize;
        let alpha: Vec<f32> = match planes.get(&-1) {
            Some(plane) => sample_plane(plane, depth, samples, true),
            None => vec![1.0; samples],
        };
        let own_mask = self.build_mask_bitmap(reader, record, record.rect)?;

        let channel = |id: i16| planes.get(&id);
        let gray = document.header.mode == ColorMode::Grayscale;
        let red = channel(0);
        let (green, blue) = if gray {
            (red, red)
        } else {
            (channel(1), channel(2))
        };
        let (Some(red), Some(green), Some(blue)) = (red, green, blue) else {
            self.report.push(
                ReportKind::NotImported,
                Some(&name),
                "The layer is missing colour channels and was not imported.",
            );
            return Ok(None);
        };
        let pixels = self.compose(width as u32, height as u32, [red, green, blue], &alpha)?;
        let pixel_id = self.fresh_pixel_id();
        match pixels {
            Pixels::Rgba8(image) => self.pixels.push((pixel_id.clone(), image)),
            Pixels::Float(image) => self.linear_pixels.push((pixel_id.clone(), image)),
        }

        let id = self.fresh_layer_id();
        let now = self.clock.now();
        let mut layer = new_layer(
            &id,
            &name,
            LayerContent::Pixel {
                pixel_id,
                width: width as u32,
                height: height as u32,
            },
            &now,
        );
        layer.opacity = Self::opacity_of(record);
        layer.visible = record.visible();
        let (blend, _) = self.blend_of(record, false);
        layer.blend_mode = blend;
        layer.transform.translate_x = record.rect.left as f32;
        layer.transform.translate_y = record.rect.top as f32;
        if let Some(mask) = own_mask {
            layer.mask = Some(LayerMask {
                snapshot: MaskSnapshot::encode(&mask.bitmap),
                enabled: mask.enabled,
                inverted: mask.inverted,
            });
        }
        self.describe_keys(record);
        self.report.layers_imported += 1;
        Ok(Some(layer))
    }

    /// An adjustment layer this importer can hold as an adjustment layer of its own.
    ///
    /// `Ok(None)`: not one of those, and the caller reports it as it would any layer with no
    /// pixels. `Ok(Some(None))`: one of those whose settings could not be read, reported here
    /// with the reason and left out. `Ok(Some(Some(layer)))`: imported.
    fn import_adjustment<R: std::io::Read + std::io::Seek>(
        &mut self,
        reader: &mut Reader<R>,
        record: &LayerRecord,
        name: &str,
    ) -> Result<Option<Option<Layer>>, AppError> {
        let Some((key, data)) = record.adjustment.as_ref() else {
            return Ok(None);
        };
        let grayscale = self.document.header.mode == ColorMode::Grayscale;
        let converted = match super::adjustment::convert(key, data, grayscale) {
            Ok(converted) => converted,
            Err(reason) => {
                self.report.push(
                    ReportKind::NotImported,
                    Some(name),
                    format!(
                        "Adjustment layer: {reason}; the layers beneath it are shown without it."
                    ),
                );
                return Ok(Some(None));
            }
        };
        let id = self.fresh_layer_id();
        let now = self.clock.now();
        let mut layer = new_layer(
            &id,
            name,
            LayerContent::Adjustment {
                operation: Box::new(converted.operation),
            },
            &now,
        );
        layer.opacity = Self::opacity_of(record);
        layer.visible = record.visible();
        let (blend, _) = self.blend_of(record, false);
        layer.blend_mode = blend;
        // An adjustment layer's mask lives in canvas space, like a group's.
        let header = &self.document.header;
        let canvas = Rect {
            top: 0,
            left: 0,
            bottom: header.height as i32,
            right: header.width as i32,
        };
        if let Some(mask) = self.build_mask(reader, record, canvas)? {
            layer.mask = Some(mask);
        }
        if let Some(note) = converted.note {
            self.report.push(ReportKind::Approximated, Some(name), note);
        }
        self.describe_keys(record);
        self.report.layers_imported += 1;
        Ok(Some(Some(layer)))
    }

    fn report_pixelless(&mut self, record: &LayerRecord, name: &str) {
        const ADJUSTMENTS: [(&[u8; 4], &str); 18] = [
            (b"brit", "Brightness/Contrast"),
            (b"levl", "Levels"),
            (b"curv", "Curves"),
            (b"hue2", "Hue/Saturation"),
            (b"hue ", "Hue/Saturation"),
            (b"blnc", "Color Balance"),
            (b"selc", "Selective Color"),
            (b"mixr", "Channel Mixer"),
            (b"grdm", "Gradient Map"),
            (b"phfl", "Photo Filter"),
            (b"post", "Posterize"),
            (b"posz", "Posterize"),
            (b"thrs", "Threshold"),
            (b"nvrt", "Invert"),
            (b"expA", "Exposure"),
            (b"vibA", "Vibrance"),
            (b"blwh", "Black & White"),
            (b"clrL", "Color Lookup"),
        ];
        // Keys that say nothing about what a layer shows.
        const INERT: [&[u8; 4]; 14] = [
            b"luni", b"lyid", b"lclr", b"lspf", b"clbl", b"infx", b"knko", b"fxrp", b"shmd",
            b"lnsr", b"lyvr", b"sn2P", b"tsly", b"lmgm",
        ];
        let adjustment = ADJUSTMENTS
            .iter()
            .find(|(key, _)| record.has_key(key))
            .map(|(_, label)| *label);
        if let Some(label) = adjustment {
            self.report.push(
                ReportKind::NotImported,
                Some(name),
                format!("{label} adjustment layer: adjustment layers are not imported from Photoshop files, so the layers beneath it are shown without it."),
            );
        } else if record.has_key(b"TySh") {
            self.report.push(
                ReportKind::NotImported,
                Some(name),
                "Text layer: Photoshop stored no pixels for it, so it was not imported.",
            );
        } else if record.has_key(b"SoCo")
            || record.has_key(b"GdFl")
            || record.has_key(b"PtFl")
            || record.has_key(b"vmsk")
            || record.has_key(b"vsms")
        {
            self.report.push(
                ReportKind::NotImported,
                Some(name),
                "Shape or fill layer: Photoshop stored it as a vector shape and a fill, not as pixels, and PhotoForge does not read those yet, so it was not imported.",
            );
        } else if record.has_key(b"SoLd") || record.has_key(b"SoLE") || record.has_key(b"PlLd") {
            self.report.push(
                ReportKind::NotImported,
                Some(name),
                "Smart object with no stored pixels was not imported.",
            );
        } else if let Some(key) = record.block_keys.iter().find(|key| !INERT.contains(key)) {
            let key: String = key.iter().map(|byte| char::from(*byte)).collect();
            self.report.push(
                ReportKind::NotImported,
                Some(name),
                format!("A layer of a kind PhotoForge does not read ({key}) holds no pixels and was not imported."),
            );
        } else {
            self.report.push(
                ReportKind::Note,
                Some(name),
                "The layer holds no pixels and was skipped.",
            );
        }
    }

    /// A layer mask in the layer's own pixel grid, with whether it is enabled and inverted.
    fn build_mask_bitmap<R: std::io::Read + std::io::Seek>(
        &mut self,
        reader: &mut Reader<R>,
        record: &LayerRecord,
        target: Rect,
    ) -> Result<Option<BuiltMask>, AppError> {
        let Some(mask) = &record.mask else {
            return Ok(None);
        };
        // The raster mask is channel -3 when a vector mask also exists, otherwise -2.
        let (channel_id, rect, default_color, flags) =
            match (&mask.real, record_channel(record, -3)) {
                (Some(real), Some(_)) => (-3, real.rect, real.default_color, real.flags),
                _ => (-2, mask.rect, mask.default_color, mask.flags),
            };
        let Some(channel) = record
            .channels
            .iter()
            .find(|channel| channel.id == channel_id)
        else {
            return Ok(None);
        };
        // Flag bit 0: the rectangle is relative to the layer rather than the canvas.
        let (offset_x, offset_y) = if flags & 1 != 0 {
            (i64::from(record.rect.left), i64::from(record.rect.top))
        } else {
            (0, 0)
        };
        let rect = Rect {
            top: (i64::from(rect.top) + offset_y) as i32,
            left: (i64::from(rect.left) + offset_x) as i32,
            bottom: (i64::from(rect.bottom) + offset_y) as i32,
            right: (i64::from(rect.right) + offset_x) as i32,
        };
        let (target_width, target_height) = (target.width(), target.height());
        let depth = self.document.header.depth;
        // Two sizes are the file's to choose: the grid (the whole canvas for a group or an
        // adjustment layer) and the mask's own rectangle, which nothing else bounds. Both are
        // admitted here, before anything is allocated or read for them.
        //
        // The grid is a byte a pixel and may hold no more than a mask may (what
        // `MaskBitmap::from_coverage` would refuse afterwards), nor more than one job.
        let grid = target_width.saturating_mul(target_height);
        if grid > MAX_MASK_PIXELS.min(self.limits.job_bytes) {
            self.report.push(
                ReportKind::NotImported,
                Some(record.display_name()),
                format!("The layer's mask would be {target_width} x {target_height} pixels, more than PhotoForge allows for a mask; it was not imported."),
            );
            return Ok(None);
        }
        // Reading the rectangle holds the grid, the plane as the file stores it, and the
        // floats `sample_plane` widens it to (four bytes a pixel), as a layer's peak is
        // counted. That peak must fit one job.
        let rect_pixels = rect.width().saturating_mul(rect.height());
        let plane_bytes = rect_pixels.saturating_mul(u64::from(depth / 8).max(1));
        let peak = grid
            .saturating_add(plane_bytes)
            .saturating_add(rect_pixels.saturating_mul(4));
        if peak > self.limits.job_bytes {
            self.report.push(
                ReportKind::NotImported,
                Some(record.display_name()),
                format!(
                    "The layer's mask is {} x {} pixels and needs about {} MiB to import, more than the {} MiB one job may use; it was not imported.",
                    rect.width(),
                    rect.height(),
                    peak / (1 << 20),
                    self.limits.job_bytes / (1 << 20)
                ),
            );
            return Ok(None);
        }
        let mut coverage = vec![default_color; grid as usize];
        if !rect.is_empty() {
            let version = self.document.header.version;
            let spec = ChannelSpec {
                offset: channel.offset,
                length: channel.length,
                width: rect.width(),
                height: rect.height(),
                depth,
            };
            let plane =
                decode_channel(reader, version, &spec, plane_bytes).map_err(AppError::from)?;
            let values = sample_plane(&plane, depth, rect_pixels as usize, true);
            for y in 0..rect.height() as i64 {
                for x in 0..rect.width() as i64 {
                    let tx = i64::from(rect.left) + x - i64::from(target.left);
                    let ty = i64::from(rect.top) + y - i64::from(target.top);
                    if tx >= 0 && ty >= 0 && tx < target_width as i64 && ty < target_height as i64 {
                        let value = values[(y as u64 * rect.width() + x as u64) as usize];
                        coverage[(ty as u64 * target_width + tx as u64) as usize] =
                            (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                }
            }
        }
        let bitmap =
            MaskBitmap::from_coverage(target_width as u32, target_height as u32, coverage)?;
        // Bit 1: the mask is disabled. Bit 2: invert when blending (obsolete).
        Ok(Some(BuiltMask {
            bitmap,
            enabled: flags & 0b10 == 0,
            inverted: flags & 0b100 != 0,
        }))
    }

    fn build_mask<R: std::io::Read + std::io::Seek>(
        &mut self,
        reader: &mut Reader<R>,
        record: &LayerRecord,
        target: Rect,
    ) -> Result<Option<LayerMask>, AppError> {
        Ok(self
            .build_mask_bitmap(reader, record, target)?
            .map(|mask| LayerMask {
                snapshot: MaskSnapshot::encode(&mask.bitmap),
                enabled: mask.enabled,
                inverted: mask.inverted,
            }))
    }

    /// Colour planes and alpha -> a pixel buffer of the document's precision.
    fn compose(
        &mut self,
        width: u32,
        height: u32,
        colour: [&Vec<u8>; 3],
        alpha: &[f32],
    ) -> Result<Pixels, AppError> {
        let depth = self.plan.depth;
        let count = width as usize * height as usize;
        let channels: [Vec<f32>; 3] = [
            sample_plane(colour[0], depth, count, false),
            sample_plane(colour[1], depth, count, false),
            sample_plane(colour[2], depth, count, false),
        ];
        if !self.plan.float {
            let mut image = RgbaImage::new(width, height);
            for (index, pixel) in image.pixels_mut().enumerate() {
                let to_byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                *pixel = Rgba([
                    to_byte(channels[0][index]),
                    to_byte(channels[1][index]),
                    to_byte(channels[2][index]),
                    to_byte(alpha[index]),
                ]);
            }
            return Ok(Pixels::Rgba8(image));
        }
        let mut out = Vec::with_capacity(count);
        let row = width as usize;
        let mut encoded = vec![0.0_f32; row * 4];
        let mut linear = vec![0.0_f32; row * 4];
        for y in 0..height as usize {
            for x in 0..row {
                let index = y * row + x;
                encoded[x * 4] = channels[0][index];
                encoded[x * 4 + 1] = channels[1][index];
                encoded[x * 4 + 2] = channels[2][index];
                encoded[x * 4 + 3] = alpha[index].clamp(0.0, 1.0);
            }
            match &self.plan.transform {
                Some(transform) => transform.apply(&encoded, &mut linear)?,
                None if depth == 32 => linear.copy_from_slice(&encoded),
                None => {
                    for (source, dest) in encoded.chunks_exact(4).zip(linear.chunks_exact_mut(4)) {
                        dest[0] = srgb_decode(source[0].clamp(0.0, 1.0));
                        dest[1] = srgb_decode(source[1].clamp(0.0, 1.0));
                        dest[2] = srgb_decode(source[2].clamp(0.0, 1.0));
                        dest[3] = source[3];
                    }
                }
            }
            for pixel in linear.chunks_exact(4) {
                out.push(FloatRgba::new(
                    finite(pixel[0]),
                    finite(pixel[1]),
                    finite(pixel[2]),
                    finite(pixel[3]).clamp(0.0, 1.0),
                ));
            }
        }
        Ok(Pixels::Float(FloatImage::new(width, height, out)?))
    }

    /// A file with no layers: its merged image as the one layer.
    fn import_flat_image<R: std::io::Read + std::io::Seek>(
        &mut self,
        reader: &mut Reader<R>,
    ) -> Result<Layer, AppError> {
        let header = &self.document.header;
        let image = self
            .document
            .image_data
            .as_ref()
            .ok_or_else(|| refuse("The file has neither layers nor image data."))?;
        let needed: u16 = if header.mode == ColorMode::Grayscale {
            1
        } else {
            3
        };
        if header.channels < needed {
            return Err(refuse("The file's image data is missing colour channels."));
        }
        let (width, height) = (u64::from(header.width), u64::from(header.height));
        self.reserve(
            width * height * self.bytes_per_pixel(),
            "the flattened image",
        )?;
        // Every channel in the file is read (the row table covers them all); the first
        // `needed` are the colour.
        let planes = decode_merged_image(
            reader,
            header.version,
            &MergedSpec {
                offset: image.offset,
                channels: header.channels,
                width,
                height,
                depth: header.depth,
            },
            width * height * u64::from(header.depth / 8).max(1) * u64::from(header.channels),
        )
        .map_err(AppError::from)?;
        let alpha = vec![1.0_f32; (width * height) as usize];
        let (green, blue) = if needed == 1 {
            (&planes[0], &planes[0])
        } else {
            (&planes[1], &planes[2])
        };
        let pixels = self.compose(
            header.width,
            header.height,
            [&planes[0], green, blue],
            &alpha,
        )?;
        let pixel_id = self.fresh_pixel_id();
        match pixels {
            Pixels::Rgba8(image) => self.pixels.push((pixel_id.clone(), image)),
            Pixels::Float(image) => self.linear_pixels.push((pixel_id.clone(), image)),
        }
        let id = self.fresh_layer_id();
        let now = self.clock.now();
        Ok(new_layer(
            &id,
            "Background",
            LayerContent::Pixel {
                pixel_id,
                width: header.width,
                height: header.height,
            },
            &now,
        ))
    }
}

fn record_channel(record: &LayerRecord, id: i16) -> Option<()> {
    record
        .channels
        .iter()
        .find(|channel| channel.id == id)
        .map(|_| ())
}

fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

/// One plane of big-endian samples as floats. Integer depths are normalised to 0..1;
/// 32-bit samples are floats already. `clamp` limits float samples to 0..1 (alpha and masks).
fn sample_plane(plane: &[u8], depth: u16, count: usize, clamp: bool) -> Vec<f32> {
    let mut out = Vec::with_capacity(count);
    match depth {
        8 => out.extend(
            plane
                .iter()
                .take(count)
                .map(|value| f32::from(*value) / 255.0),
        ),
        16 => out.extend(
            plane
                .chunks_exact(2)
                .take(count)
                .map(|pair| f32::from(u16::from_be_bytes([pair[0], pair[1]])) / 65_535.0),
        ),
        _ => out.extend(
            plane
                .chunks_exact(4)
                .take(count)
                .map(|word| f32::from_be_bytes([word[0], word[1], word[2], word[3]])),
        ),
    }
    out.resize(count, 0.0);
    if clamp {
        for value in &mut out {
            *value = finite(*value).clamp(0.0, 1.0);
        }
    }
    out
}

fn mode_name(mode: ColorMode) -> &'static str {
    match mode {
        ColorMode::Bitmap => "Bitmap",
        ColorMode::Grayscale => "Grayscale",
        ColorMode::Indexed => "Indexed colour",
        ColorMode::Rgb => "RGB",
        ColorMode::Cmyk => "CMYK",
        ColorMode::Multichannel => "Multichannel",
        ColorMode::Duotone => "Duotone",
        ColorMode::Lab => "Lab",
    }
}
