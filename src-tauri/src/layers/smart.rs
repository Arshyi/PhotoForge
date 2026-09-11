//! Smart objects: content composed at its own size, placed by a transform.
//!
//! # What makes this a smart object rather than a filename
//!
//! A pixel layer scaled to 15% and back to 100% is ruined: the intermediate
//! resampling threw away the data, and no later transform can recover it. A
//! smart object keeps its content at its native size and applies the layer's
//! transform to a *fresh* composite of that content every render, so the
//! scaling is never composed onto a damaged raster. Scale it down, scale it
//! back, and the result is the original.
//!
//! That property is what the name means. A raster layer that merely remembers
//! which file it came from is not a smart object, because the second transform
//! still lands on the remains of the first.
//!
//! # Sources and instances
//!
//! The content lives in the document's source registry, and layers refer to it
//! by identifier. Two layers naming the same source are two instances of one
//! thing: editing the source changes both, while each keeps its own transform,
//! opacity, blend mode and mask. That is the distinction between the source and
//! an instance of it, and it is why the content is not stored on the layer.
//!
//! # Links
//!
//! A source may additionally be linked to a file on disk. The link records the
//! path and the SHA-256 of the bytes read from it. That digest is what lets
//! PhotoForge say the file is missing or has changed rather than assuming it is
//! still what it was, and the check hashes the file again rather than trusting
//! a timestamp.
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::error::AppError;

use super::model::{valid_identifier, Layer, MAX_LAYER_ID_CHARS};

/// How many smart sources one document may hold.
pub const MAX_SMART_SOURCES: usize = 512;
/// How deeply smart objects may nest inside each other's content.
///
/// A source's content may itself contain smart objects. Each level is a whole
/// extra composite at native size, so the cost compounds; four is enough for a
/// badge inside a card inside a page and well short of anything pathological.
pub const MAX_SMART_DEPTH: usize = 4;
/// Longest linked path kept, in characters.
pub const MAX_LINK_PATH_CHARS: usize = 4_096;
/// Largest native size a source may declare, per side.
pub const MAX_SOURCE_EDGE: u32 = 32_768;
/// Sum of native smart composite buffers retained by one render. Native-size
/// fallback is deliberately bounded; it is not an out-of-core source renderer.
pub const MAX_SMART_COMPOSITE_BYTES: u64 = 536_870_912;

/// A file a smart source was read from.
///
/// The digest is of the file's bytes, not of the composed result, so it answers
/// exactly one question: is the file on disk still the one this content came
/// from?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartLink {
    pub path: String,
    /// Lowercase hexadecimal SHA-256 of the file's bytes when it was read.
    pub digest: String,
    pub bytes: u64,
}

impl SmartLink {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.path.trim().is_empty() || self.path.chars().count() > MAX_LINK_PATH_CHARS {
            return Err(AppError::InvalidLayerDocument(format!(
                "a linked path must contain 1 to {MAX_LINK_PATH_CHARS} characters"
            )));
        }
        // Projects store links as inert metadata; they never authorise a
        // network connection or a path traversal. Only explicit local absolute
        // file paths may be checked/relinked by the command layer.
        let normalised = self.path.replace('\\', "/");
        let drive_absolute = normalised.as_bytes().get(1) == Some(&b':')
            && normalised
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
            && normalised.as_bytes().get(2) == Some(&b'/');
        let unix_absolute = normalised.starts_with('/') && !normalised.starts_with("//");
        if self.path.contains('\0')
            || normalised.starts_with("//")
            || (!drive_absolute && !unix_absolute)
            || normalised
                .split('/')
                .any(|part| part == ".." || part == ".")
            || normalised.get(2..).is_some_and(|tail| tail.contains(':'))
        {
            return Err(AppError::InvalidLayerDocument(
                "smart links require an absolute local file path without traversal or network paths".into(),
            ));
        }
        if self.digest.len() != 64 || !self.digest.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AppError::InvalidLayerDocument(
                "a linked source digest must be 64 hexadecimal characters".into(),
            ));
        }
        Ok(())
    }
}

/// Whether a linked file is still what the project expects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LinkState {
    /// Not linked to anything; the content lives in the project.
    Embedded,
    /// The file is present and its bytes still hash to the recorded digest.
    Available,
    /// The path does not exist, or cannot be read.
    Missing,
    /// The file is present but its bytes hash to something else.
    Changed,
}

/// Content composed at its own size, shared by every instance of it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartSource {
    /// The size the content is composed at, before any instance's transform.
    pub width: u32,
    pub height: u32,
    /// The content itself: an ordinary layer stack, composited at native size.
    ///
    /// A stack rather than a single buffer, because "edit contents" has to mean
    /// something. A source holding one flattened image would be a raster layer
    /// wearing a different name.
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// The file this content came from, when it came from one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<SmartLink>,
}

impl SmartSource {
    pub fn new(width: u32, height: u32, layers: Vec<Layer>) -> Self {
        Self {
            width,
            height,
            layers,
            link: None,
        }
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_SOURCE_EDGE
            || self.height > MAX_SOURCE_EDGE
        {
            return Err(AppError::InvalidLayerDocument(format!(
                "a smart source must be between 1 and {MAX_SOURCE_EDGE} pixels on each side"
            )));
        }
        crate::resources::checked_pixels(self.width, self.height)?;
        if let Some(link) = &self.link {
            link.validate()?;
        }
        Ok(())
    }

    /// Whether this source is linked to a file rather than only embedded.
    pub fn is_linked(&self) -> bool {
        self.link.is_some()
    }
}

/// What a smart object layer holds: which source it is an instance of.
///
/// Deliberately just the reference. Everything else about the appearance —
/// placement, opacity, blend mode, mask — is the layer's, so two instances of
/// one source can differ in all of those while sharing the content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartObjectContent {
    pub source_id: String,
}

impl SmartObjectContent {
    pub fn new(source_id: impl Into<String>) -> Self {
        Self {
            source_id: source_id.into(),
        }
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if !valid_identifier(&self.source_id) {
            return Err(AppError::InvalidLayerDocument(format!(
                "smart source identifiers must contain 1 to {MAX_LAYER_ID_CHARS} characters from A-Z, a-z, 0-9, hyphen, or underscore"
            )));
        }
        Ok(())
    }
}

/// The document's smart sources, keyed by identifier.
pub type SmartSources = HashMap<String, SmartSource>;

/// Checks that every smart object resolves, and that none of them contains
/// itself at any depth.
///
/// Recursion has to be refused rather than merely bounded: a source that
/// contains an instance of itself is not deep, it is infinite, and stopping at
/// a depth limit would render some arbitrary number of copies and call that the
/// document. The two failures are reported differently for that reason.
pub fn validate_sources(layers: &[Layer], sources: &SmartSources) -> Result<(), AppError> {
    if sources.len() > MAX_SMART_SOURCES {
        return Err(AppError::InvalidLayerDocument(format!(
            "a document may hold at most {MAX_SMART_SOURCES} smart sources"
        )));
    }
    let mut composite_bytes = 0_u64;
    for (id, source) in sources {
        if !valid_identifier(id) {
            return Err(AppError::InvalidLayerDocument(
                "a smart source identifier was not a valid identifier".into(),
            ));
        }
        source.validate()?;
        composite_bytes = composite_bytes
            .checked_add(u64::from(source.width) * u64::from(source.height) * 16)
            .ok_or(AppError::OutOfMemoryRisk)?;
    }
    if composite_bytes > MAX_SMART_COMPOSITE_BYTES {
        return Err(AppError::ResourceBudget {
            required: composite_bytes,
            limit: MAX_SMART_COMPOSITE_BYTES,
        });
    }
    // Every stack in the document, including the ones inside sources, has to
    // resolve. A source referenced by nothing is still checked, because it can
    // be placed later and a project that only fails once used is worse.
    let root_references = stack_references(layers)?;
    let dependencies: HashMap<_, _> = sources
        .iter()
        .map(|(id, source)| Ok((id.as_str(), stack_references(&source.layers)?)))
        .collect::<Result<_, AppError>>()?;
    let mut memo = HashMap::new();
    for id in root_references
        .into_iter()
        .chain(sources.keys().map(String::as_str))
    {
        check_source(id, &dependencies, &mut Vec::new(), &mut memo)?;
    }
    Ok(())
}

fn stack_references(layers: &[Layer]) -> Result<Vec<&str>, AppError> {
    use super::model::LayerContent;
    let mut references = Vec::new();
    let mut stack: Vec<_> = layers.iter().map(|layer| (layer, 1)).collect();
    let mut count = 0;
    while let Some((layer, depth)) = stack.pop() {
        count += 1;
        if depth > super::model::MAX_GROUP_DEPTH || count > super::model::MAX_LAYERS {
            return Err(AppError::InvalidLayerDocument(
                "smart source layer tree exceeds its bounded depth or layer count".into(),
            ));
        }
        match &layer.content {
            LayerContent::SmartObject { smart } => {
                references.push(smart.source_id.as_str());
            }
            LayerContent::Group { children, .. } => {
                stack.extend(children.iter().map(|child| (child, depth + 1)))
            }
            LayerContent::Pixel { .. }
            | LayerContent::Adjustment { .. }
            | LayerContent::Shape { .. }
            | LayerContent::Text { .. } => {}
        }
    }
    Ok(references)
}

fn check_source<'a>(
    id: &'a str,
    dependencies: &HashMap<&'a str, Vec<&'a str>>,
    open: &mut Vec<&'a str>,
    memo: &mut HashMap<&'a str, usize>,
) -> Result<usize, AppError> {
    if open.contains(&id) {
        return Err(AppError::InvalidLayerDocument(format!(
            "smart source '{id}' contains itself"
        )));
    }
    if open.len() >= MAX_SMART_DEPTH {
        return Err(AppError::InvalidLayerDocument(format!(
            "smart objects may nest at most {MAX_SMART_DEPTH} deep"
        )));
    }
    let children = dependencies.get(id).ok_or_else(|| {
        AppError::InvalidLayerDocument(format!("smart source '{id}' is missing from the document"))
    })?;
    let depth = if let Some(depth) = memo.get(id) {
        *depth
    } else {
        open.push(id);
        let mut deepest = 0;
        for child in children {
            deepest = deepest.max(check_source(child, dependencies, open, memo)?);
        }
        open.pop();
        let depth = deepest + 1;
        memo.insert(id, depth);
        depth
    };
    if open.len() + depth > MAX_SMART_DEPTH {
        return Err(AppError::InvalidLayerDocument(format!(
            "smart objects may nest at most {MAX_SMART_DEPTH} deep"
        )));
    }
    Ok(depth)
}

/// Every pixel identifier a source's content refers to, so the store resolves
/// them alongside the document's own.
pub fn referenced_pixel_ids(sources: &SmartSources, out: &mut std::collections::HashSet<String>) {
    use super::model::LayerContent;
    let mut stack: Vec<&Layer> = sources.values().flat_map(|s| s.layers.iter()).collect();
    while let Some(layer) = stack.pop() {
        if let LayerContent::Pixel { pixel_id, .. } = &layer.content {
            out.insert(pixel_id.clone());
        }
        stack.extend(layer.children());
    }
}

/// Native-size source fallback prepared once per render, not once per tile or
/// instance. The edited document is never changed: lowering exists only for
/// the lifetime of the render. All instances of a source share one immutable
/// composite. Nested sources are resolved in dependency order.
pub(super) struct PreparedRender<'a> {
    pub document: super::LayerDocument,
    pub pixels: PreparedPixels<'a>,
    pub native_composite_bytes: u64,
    pub peak_source_bytes: u64,
}

pub(super) struct PreparedPixels<'a> {
    base: &'a dyn super::PixelSource,
    frames: HashMap<String, std::sync::Arc<crate::color::FloatImage>>,
    digests: HashMap<String, [u8; 32]>,
    bytes: u64,
}

impl super::PixelSource for PreparedPixels<'_> {
    fn resolve(&self, id: &str) -> Result<std::sync::Arc<image::RgbaImage>, AppError> {
        match self.frames.get(id) {
            Some(frame) => Ok(std::sync::Arc::new(frame.to_rgba8())),
            None => self.base.resolve(id),
        }
    }

    fn resolve_linear(
        &self,
        id: &str,
    ) -> Result<std::sync::Arc<crate::color::FloatImage>, AppError> {
        match self.frames.get(id) {
            Some(frame) => Ok(std::sync::Arc::clone(frame)),
            None => self.base.resolve_linear(id),
        }
    }

    fn resolve_native_linear(
        &self,
        id: &str,
    ) -> Result<std::sync::Arc<crate::color::FloatImage>, AppError> {
        match self.frames.get(id) {
            Some(frame) => Ok(std::sync::Arc::clone(frame)),
            None => self.base.resolve_native_linear(id),
        }
    }

    fn dimensions(&self, id: &str) -> Option<(u32, u32)> {
        self.frames
            .get(id)
            .map(|frame| frame.dimensions())
            .or_else(|| self.base.dimensions(id))
    }

    fn cache_fingerprint(&self, id: &str) -> Option<[u8; 32]> {
        self.digests
            .get(id)
            .copied()
            .or_else(|| self.base.cache_fingerprint(id))
    }

    fn resident_bytes(&self) -> u64 {
        self.base.resident_bytes().saturating_add(self.bytes)
    }

    fn promotion_bytes(&self, document: &super::LayerDocument) -> u64 {
        // Deliberately conservative for caller-owned encoded sources.
        self.base.promotion_bytes(document)
    }
}

impl<'a> PreparedPixels<'a> {
    fn new(base: &'a dyn super::PixelSource) -> Self {
        Self {
            base,
            frames: HashMap::new(),
            digests: HashMap::new(),
            bytes: 0,
        }
    }

    fn insert(&mut self, id: String, frame: std::sync::Arc<crate::color::FloatImage>) {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"photoforge.smart-native.v1");
        hash.update(frame.width().to_le_bytes());
        hash.update(frame.height().to_le_bytes());
        for pixel in frame.pixels() {
            for channel in [pixel.red, pixel.green, pixel.blue, pixel.alpha] {
                hash.update(channel.to_bits().to_le_bytes());
            }
        }
        self.bytes += frame.pixels().len() as u64 * 16;
        self.digests.insert(id.clone(), hash.finalize().into());
        self.frames.insert(id, frame);
    }
}

/// Prepare native smart composites with an explicit conservative byte budget.
/// The outer renderer still tiles/streams its output, but smart source stacks
/// currently use the full-frame CPU oracle. This distinction is exposed in
/// `TiledStats`; no claim of out-of-core smart content is made.
pub(super) fn prepare_render<'a>(
    document: &super::LayerDocument,
    source: &'a dyn super::PixelSource,
    options: super::RenderOptions<'_>,
) -> Result<Option<PreparedRender<'a>>, AppError> {
    use super::model::LayerContent;
    use crate::color::FloatImage;
    use crate::resources::ResourceEstimate;
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    if document.smart_sources.is_empty() {
        return Ok(None);
    }
    document.validate()?;
    if !options.scale.is_finite() || options.scale <= 0.0 || options.scale > 1.0 {
        return Err(AppError::InvalidLayerDocument(
            "render scale must be greater than zero and no larger than one".into(),
        ));
    }
    crate::image_processing::high_precision::check_cancel(options.cancel)?;

    let mut pending = stack_references(&document.layers)?;
    let mut required = HashSet::new();
    while let Some(id) = pending.pop() {
        if required.insert(id) {
            pending.extend(stack_references(&document.smart_sources[id].layers)?);
        }
    }
    // Deterministic identifiers, disjoint from every caller-owned pixel id.
    let mut reserved: HashSet<String> = document.referenced_pixel_ids().into_iter().collect();
    let mut source_ids: Vec<_> = required.into_iter().collect();
    source_ids.sort_unstable();
    let mut aliases = HashMap::new();
    let mut serial = 0_u64;
    for id in &source_ids {
        loop {
            let candidate = format!("smart_render_{serial}");
            serial += 1;
            if reserved.insert(candidate.clone()) {
                aliases.insert((*id).to_string(), candidate);
                break;
            }
        }
    }

    let mut native_references = HashMap::new();
    let mut native_composite_bytes = 0_u64;
    let mut peak_source_bytes = 0;
    let mut source_documents = HashMap::new();
    for id in &source_ids {
        let smart = &document.smart_sources[*id];
        native_composite_bytes += u64::from(smart.width) * u64::from(smart.height) * 16;
        let mut source_document = super::LayerDocument::new(smart.width, smart.height);
        source_document.precision = crate::pixel::DocumentPrecision::LinearSrgbF32;
        source_document.layers = smart.layers.clone();
        for layer in source_document.iter() {
            if let LayerContent::Pixel {
                pixel_id,
                width,
                height,
            } = &layer.content
            {
                if native_references
                    .insert(pixel_id.clone(), (*width, *height))
                    .is_some_and(|existing| existing != (*width, *height))
                {
                    return Err(AppError::InvalidLayerDocument(
                        "a shared smart pixel source has inconsistent native dimensions".into(),
                    ));
                }
            }
        }
        peak_source_bytes = peak_source_bytes
            .max(ResourceEstimate::render(&source_document, 1.0, 0, 0)?.estimated_peak_bytes);
        lower_stack(
            &mut source_document.layers,
            &document.smart_sources,
            &aliases,
        )?;
        source_documents.insert((*id).to_string(), source_document);
    }
    let native_pixel_bytes: u64 = native_references
        .values()
        .map(|(w, h)| u64::from(*w) * u64::from(*h) * 16)
        .sum();
    let root_estimate = ResourceEstimate::render(
        document,
        options.scale,
        source.resident_bytes(),
        source.promotion_bytes(document),
    )?;
    // Native pixels (including any encoded promotions), retained composites,
    // source work, root work and preview copies are all reserved before the
    // first source allocation. Double-counting shared float input is safe.
    ResourceEstimate::new(
        root_estimate.estimated_peak_bytes,
        native_pixel_bytes
            .checked_add(native_composite_bytes.saturating_mul(2))
            .and_then(|bytes| bytes.checked_add(peak_source_bytes))
            .ok_or(AppError::OutOfMemoryRisk)?,
        0,
        0,
    )?;

    let mut native = PreparedPixels::new(source);
    for (id, dimensions) in native_references {
        crate::image_processing::high_precision::check_cancel(options.cancel)?;
        let frame = source.resolve_native_linear(&id)?;
        if frame.dimensions() != dimensions {
            return Err(AppError::InvalidLayerDocument(format!(
                "smart source pixels '{id}' must resolve at their original dimensions"
            )));
        }
        native.insert(id, frame);
    }
    let mut completed = HashSet::new();
    while completed.len() < source_ids.len() {
        let mut progressed = false;
        for id in &source_ids {
            if completed.contains(*id) {
                continue;
            }
            if stack_references(&document.smart_sources[*id].layers)?
                .iter()
                .any(|child| !completed.contains(child))
            {
                continue;
            }
            let frame = super::linear::render_document_float(
                &source_documents[*id],
                &native,
                super::RenderOptions {
                    scale: 1.0,
                    cancel: options.cancel,
                },
            )?;
            native.insert(aliases[*id].clone(), Arc::new(frame));
            completed.insert(*id);
            progressed = true;
        }
        if !progressed {
            return Err(AppError::InvalidLayerDocument(
                "smart source dependency graph could not be resolved".into(),
            ));
        }
    }
    let mut pixels = PreparedPixels::new(source);
    for id in source_ids {
        let alias = &aliases[id];
        let frame = &native.frames[alias];
        let scaled = (
            ((f64::from(frame.width()) * options.scale).round() as u32).max(1),
            ((f64::from(frame.height()) * options.scale).round() as u32).max(1),
        );
        let frame: Arc<FloatImage> = if scaled == frame.dimensions() {
            Arc::clone(frame)
        } else {
            Arc::new(frame.resized(scaled.0, scaled.1)?)
        };
        pixels.insert(alias.clone(), frame);
    }
    let mut lowered = document.clone();
    lower_stack(&mut lowered.layers, &document.smart_sources, &aliases)?;
    lowered.smart_sources.clear();
    Ok(Some(PreparedRender {
        document: lowered,
        pixels,
        native_composite_bytes,
        peak_source_bytes,
    }))
}

fn lower_stack(
    layers: &mut [Layer],
    sources: &SmartSources,
    aliases: &HashMap<String, String>,
) -> Result<(), AppError> {
    use super::model::LayerContent;
    for layer in layers {
        match &mut layer.content {
            LayerContent::SmartObject { smart } => {
                let source = &sources[&smart.source_id];
                let pixel_id = aliases
                    .get(&smart.source_id)
                    .ok_or_else(|| {
                        AppError::InvalidLayerDocument("unprepared smart source".into())
                    })?
                    .clone();
                layer.content = LayerContent::Pixel {
                    pixel_id,
                    width: source.width,
                    height: source.height,
                };
            }
            LayerContent::Group { children, .. } => lower_stack(children, sources, aliases)?,
            LayerContent::Pixel { .. }
            | LayerContent::Adjustment { .. }
            | LayerContent::Text { .. }
            | LayerContent::Shape { .. } => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{FloatImage, FloatRgba};
    use crate::layers::model::LayerContent;
    use crate::layers::test_pixel_layer;
    use crate::layers::{LayerDocument, LayerMask, LayerPixelStore, RenderOptions, TileCache};
    use crate::mask::{MaskBitmap, MaskSnapshot};
    use crate::pixel::DocumentPrecision;

    fn smart_layer(id: &str, source_id: &str) -> Layer {
        Layer {
            content: LayerContent::SmartObject {
                smart: Box::new(SmartObjectContent::new(source_id)),
            },
            ..test_pixel_layer(id, "unused", 1, 1)
        }
    }

    fn sources(entries: Vec<(&str, SmartSource)>) -> SmartSources {
        entries
            .into_iter()
            .map(|(id, source)| (id.to_string(), source))
            .collect()
    }

    #[test]
    fn a_resolving_document_validates() {
        let registry = sources(vec![(
            "s1",
            SmartSource::new(64, 64, vec![test_pixel_layer("inner", "px1", 64, 64)]),
        )]);
        validate_sources(&[smart_layer("a", "s1")], &registry).expect("valid");
    }

    /// Two layers naming one source are two instances of it, which is a
    /// legitimate and important arrangement rather than a duplicate.
    #[test]
    fn several_instances_may_share_one_source() {
        let registry = sources(vec![(
            "s1",
            SmartSource::new(64, 64, vec![test_pixel_layer("inner", "px1", 64, 64)]),
        )]);
        validate_sources(
            &[
                smart_layer("a", "s1"),
                smart_layer("b", "s1"),
                smart_layer("c", "s1"),
            ],
            &registry,
        )
        .expect("valid");
    }

    #[test]
    fn a_missing_source_is_refused() {
        let error = validate_sources(&[smart_layer("a", "absent")], &SmartSources::new())
            .expect_err("should refuse");
        assert!(
            error.to_string().contains("absent"),
            "the error did not name the missing source: {error}"
        );
    }

    /// A source containing an instance of itself is infinite, not deep.
    #[test]
    fn direct_recursion_is_refused() {
        let registry = sources(vec![(
            "s1",
            SmartSource::new(64, 64, vec![smart_layer("self", "s1")]),
        )]);
        let error = validate_sources(&[smart_layer("a", "s1")], &registry).expect_err("refuse");
        assert!(
            error.to_string().contains("contains itself"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn indirect_recursion_is_refused() {
        let registry = sources(vec![
            ("s1", SmartSource::new(64, 64, vec![smart_layer("x", "s2")])),
            ("s2", SmartSource::new(64, 64, vec![smart_layer("y", "s3")])),
            ("s3", SmartSource::new(64, 64, vec![smart_layer("z", "s1")])),
        ]);
        let error = validate_sources(&[smart_layer("a", "s1")], &registry).expect_err("refuse");
        assert!(
            error.to_string().contains("contains itself"),
            "unexpected error: {error}"
        );
    }

    /// Recursion through a group inside a source is still recursion.
    #[test]
    fn recursion_hidden_inside_a_group_is_refused() {
        let nested = Layer {
            content: LayerContent::Group {
                children: vec![smart_layer("deep", "s1")],
                isolated: true,
            },
            ..test_pixel_layer("g", "unused", 1, 1)
        };
        let registry = sources(vec![("s1", SmartSource::new(64, 64, vec![nested]))]);
        assert!(validate_sources(&[smart_layer("a", "s1")], &registry).is_err());
    }

    /// A source that nothing places is still checked, because a project that
    /// only fails once the layer is used is worse than one that fails on open.
    #[test]
    fn an_unplaced_source_is_still_checked() {
        let registry = sources(vec![(
            "s1",
            SmartSource::new(64, 64, vec![smart_layer("dangling", "gone")]),
        )]);
        assert!(validate_sources(&[], &registry).is_err());
    }

    #[test]
    fn nesting_beyond_the_limit_is_refused() {
        // A chain one level deeper than the limit allows.
        let mut entries = Vec::new();
        for level in 0..=MAX_SMART_DEPTH {
            let next = format!("s{}", level + 1);
            let id = format!("s{level}");
            entries.push((id, SmartSource::new(64, 64, vec![smart_layer("n", &next)])));
        }
        let last = format!("s{}", MAX_SMART_DEPTH + 1);
        entries.push((
            last,
            SmartSource::new(64, 64, vec![test_pixel_layer("leaf", "px1", 64, 64)]),
        ));
        let registry: SmartSources = entries.into_iter().collect();
        let error = validate_sources(&[smart_layer("a", "s0")], &registry).expect_err("refuse");
        assert!(
            error.to_string().contains("nest at most"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_chain_within_the_limit_is_accepted() {
        let mut entries = Vec::new();
        for level in 0..MAX_SMART_DEPTH - 1 {
            let next = format!("s{}", level + 1);
            entries.push((
                format!("s{level}"),
                SmartSource::new(64, 64, vec![smart_layer("n", &next)]),
            ));
        }
        entries.push((
            format!("s{}", MAX_SMART_DEPTH - 1),
            SmartSource::new(64, 64, vec![test_pixel_layer("leaf", "px1", 64, 64)]),
        ));
        let registry: SmartSources = entries.into_iter().collect();
        validate_sources(&[smart_layer("a", "s0")], &registry).expect("valid");
    }

    #[test]
    fn hostile_sources_are_refused() {
        assert!(SmartSource::new(0, 64, vec![]).validate().is_err());
        assert!(SmartSource::new(64, 0, vec![]).validate().is_err());
        assert!(SmartSource::new(MAX_SOURCE_EDGE + 1, 64, vec![])
            .validate()
            .is_err());

        let mut linked = SmartSource::new(64, 64, vec![]);
        linked.link = Some(SmartLink {
            path: String::new(),
            digest: "a".repeat(64),
            bytes: 1,
        });
        assert!(linked.validate().is_err(), "an empty path was accepted");

        linked.link = Some(SmartLink {
            path: "C:/photos/logo.png".into(),
            digest: "not a digest".into(),
            bytes: 1,
        });
        assert!(linked.validate().is_err(), "a bad digest was accepted");

        linked.link = Some(SmartLink {
            path: "C:/photos/logo.png".into(),
            digest: "A1".repeat(32),
            bytes: 1,
        });
        assert!(linked.validate().is_ok());
    }

    #[test]
    fn source_content_contributes_its_pixel_references() {
        let registry = sources(vec![
            (
                "s1",
                SmartSource::new(64, 64, vec![test_pixel_layer("inner", "px1", 64, 64)]),
            ),
            (
                "s2",
                SmartSource::new(
                    64,
                    64,
                    vec![Layer {
                        content: LayerContent::Group {
                            children: vec![test_pixel_layer("deep", "px2", 64, 64)],
                            isolated: true,
                        },
                        ..test_pixel_layer("g", "unused", 1, 1)
                    }],
                ),
            ),
        ]);
        let mut ids = std::collections::HashSet::new();
        referenced_pixel_ids(&registry, &mut ids);
        assert!(ids.contains("px1"), "a source's own pixels were not listed");
        assert!(
            ids.contains("px2"),
            "pixels nested in a source's group were not listed"
        );
    }

    fn render_fixture() -> (LayerDocument, LayerPixelStore) {
        let mut store = LayerPixelStore::default();
        store.reset(20, 12).unwrap();
        let mut pixels = FloatImage::blank(7, 5, FloatRgba::TRANSPARENT).unwrap();
        for (i, pixel) in pixels.pixels_mut().iter_mut().enumerate() {
            *pixel = FloatRgba::new(
                (i % 7) as f32 * 0.27,
                -0.05,
                0.123456,
                (i % 5 + 1) as f32 / 5.0,
            );
        }
        let id = store.register_float(pixels).unwrap();
        let mut document = LayerDocument::new(20, 12);
        document.precision = DocumentPrecision::LinearSrgbF32;
        document.smart_sources.insert(
            "source".into(),
            SmartSource::new(7, 5, vec![test_pixel_layer("source-pixels", &id, 7, 5)]),
        );
        let mut left = smart_layer("left", "source");
        left.transform.translate_x = 1.25;
        left.transform.translate_y = 0.5;
        left.transform.rotation_degrees = 13.0;
        let mut right = smart_layer("right", "source");
        right.transform.translate_x = 10.0;
        right.transform.translate_y = 4.0;
        right.transform.scale_x = 1.2;
        right.opacity = 0.7;
        let mut mask = MaskBitmap::full(7, 5).unwrap();
        mask.set(3, 2, 64);
        right.mask = Some(LayerMask {
            snapshot: MaskSnapshot::encode(&mask),
            enabled: true,
            inverted: false,
        });
        document.layers = vec![left, right];
        (document, store)
    }

    #[test]
    fn native_shared_instances_match_tiled_and_streamed_float_without_mutating_document() {
        let (document, store) = render_fixture();
        let before = document.clone();
        let source = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let expected =
            crate::layers::render_document_float(&document, &source, RenderOptions::default())
                .unwrap();
        let (tiled, stats) = crate::layers::render_document_tiled_with_threads(
            &document,
            &source,
            RenderOptions::default(),
            64,
            3,
        )
        .unwrap();
        assert_eq!(expected.pixels(), tiled.pixels());
        assert!(stats.smart_source_full_frame_fallback);
        assert!(!stats.fell_back_to_full_frame);
        assert_eq!(stats.smart_source_composite_bytes, 7 * 5 * 16);
        let mut streamed = Vec::new();
        let stream_stats = crate::layers::render_document_streaming(
            &document,
            &source,
            RenderOptions::default(),
            64,
            3,
            &mut |_, band| {
                streamed.extend_from_slice(band.pixels());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(streamed, expected.pixels());
        assert_eq!(
            stream_stats.smart_source_composite_bytes,
            stats.smart_source_composite_bytes
        );
        assert_eq!(document, before);
    }

    #[test]
    fn stable_source_ids_invalidate_warm_tiles_after_content_edits() {
        let (mut document, store) = render_fixture();
        let source = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let cache = TileCache::with_capacity(1024 * 1024);
        let render = |document: &LayerDocument| {
            crate::layers::render_document_tiled_cached(
                document,
                &source,
                RenderOptions::default(),
                64,
                2,
                Some(&cache),
            )
            .unwrap()
        };
        let (first, _) = render(&document);
        assert!(render(&document).1.cached_tiles > 0);
        document.smart_sources.get_mut("source").unwrap().layers[0].opacity = 0.4;
        let (edited, stats) = render(&document);
        assert!(stats.cached_tiles < stats.tiles);
        assert_ne!(edited.pixels(), first.pixels());
        let truth =
            crate::layers::render_document_float(&document, &source, RenderOptions::default())
                .unwrap();
        assert_eq!(edited.pixels(), truth.pixels());
    }

    #[test]
    fn nested_source_with_neighbourhood_operation_matches_independent_native_composite() {
        let (mut document, store) = render_fixture();
        let mut adjustment = test_pixel_layer("blur", "unused", 1, 1);
        adjustment.content = LayerContent::Adjustment {
            operation: Box::new(crate::domain::EditOperation::GaussianBlur { radius: 1.0 }),
        };
        document.smart_sources.insert(
            "outer".into(),
            SmartSource::new(9, 7, vec![smart_layer("nested", "source"), adjustment]),
        );
        document.layers = vec![smart_layer("placed-outer", "outer")];
        let source = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let expected =
            crate::layers::render_document_float(&document, &source, RenderOptions::default())
                .unwrap();
        let (actual, stats) =
            crate::layers::render_document_tiled(&document, &source, RenderOptions::default(), 64)
                .unwrap();
        assert_eq!(expected.pixels(), actual.pixels());
        assert_eq!(stats.smart_source_composite_bytes, (7 * 5 + 9 * 7) * 16);
        assert_eq!(
            stats.halo, 0,
            "native source blur is baked before outer tiling"
        );
    }

    #[test]
    fn scaling_an_instance_down_and_back_preserves_original_samples() {
        let (mut document, store) = render_fixture();
        document.layers.truncate(1);
        document.layers[0].transform = Default::default();
        let source = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let render = |document: &LayerDocument| {
            crate::layers::render_document_float(document, &source, RenderOptions::default())
                .unwrap()
        };
        let original = render(&document);
        document.layers[0].transform.scale_x = 0.15;
        document.layers[0].transform.scale_y = 0.15;
        assert_ne!(render(&document).pixels(), original.pixels());
        document.layers[0].transform = Default::default();
        assert_eq!(render(&document).pixels(), original.pixels());
        assert_eq!(original.pixels()[1].red, 0.27);
        assert!((original.pixels()[1].green + 0.05).abs() < 1e-6);
    }

    #[test]
    fn document_validation_checks_source_ids_aggregate_layers_masks_and_bytes() {
        let (mut document, _) = render_fixture();
        document.smart_sources.get_mut("source").unwrap().layers[0].id = "left".into();
        assert!(matches!(
            document.validate(),
            Err(AppError::DuplicateLayerId(_))
        ));
        document.smart_sources.get_mut("source").unwrap().layers[0].id = "source-pixels".into();
        document.layers[1].mask.as_mut().unwrap().snapshot =
            MaskSnapshot::encode(&MaskBitmap::full(20, 12).unwrap());
        assert!(matches!(
            document.validate(),
            Err(AppError::MaskDimensionMismatch { .. })
        ));
        document.layers[1].mask = None;
        document.smart_sources.get_mut("source").unwrap().layers = (0..crate::layers::MAX_LAYERS)
            .map(|i| test_pixel_layer(&format!("inner-{i}"), "same-pixels", 1, 1))
            .collect();
        assert!(matches!(
            document.validate(),
            Err(AppError::TooManyLayers { .. })
        ));
        let huge = sources(vec![("large", SmartSource::new(6000, 6000, vec![]))]);
        assert!(matches!(
            validate_sources(&[], &huge),
            Err(AppError::ResourceBudget { .. })
        ));
    }

    #[test]
    fn linked_paths_refuse_network_traversal_and_alternate_streams() {
        for path in [
            "../secret",
            "C:/photos/../secret",
            "C:/photos/./secret",
            "//server/share/a.png",
            "\\\\server\\share\\a.png",
            "C:/a.png:stream",
            "https://host/a.png",
            "C:/a\0.png",
        ] {
            assert!(
                SmartLink {
                    path: path.into(),
                    digest: "a".repeat(64),
                    bytes: 1
                }
                .validate()
                .is_err(),
                "accepted {path:?}"
            );
        }
    }
}
