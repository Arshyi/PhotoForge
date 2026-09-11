use super::blend::BlendMode;
use super::transform::LayerTransform;
use crate::domain::EditOperation;
use crate::error::AppError;
use crate::mask::MaskSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::HashSet;

/// Version of the in-memory and on-disk layer tree schema.
pub const LAYER_SCHEMA_VERSION: u32 = 1;
/// Largest number of layers, counting every nested child, a document may hold.
pub const MAX_LAYERS: usize = 512;
/// Largest nesting depth. The root stack is depth 1, so a layer inside two
/// groups sits at depth 3. Validation enforces this before any traversal runs,
/// which keeps the compositor's group recursion provably bounded.
pub const MAX_GROUP_DEPTH: usize = 16;
pub const MAX_LAYER_ID_CHARS: usize = 64;
pub const MAX_LAYER_NAME_CHARS: usize = 120;
pub const MAX_LAYER_METADATA_ENTRIES: usize = 16;
pub const MAX_LAYER_METADATA_VALUE_CHARS: usize = 512;
pub const MAX_TIMESTAMP_CHARS: usize = 64;
/// Largest canvas the layer document model accepts, matching the existing
/// decode ceiling in `infrastructure::image_io`.
pub const MAX_CANVAS_DIMENSION: u32 = 20_000;
pub const MAX_CANVAS_PIXELS: u64 = crate::resources::MAX_WORKING_PIXELS;

/// Groups are isolated unless a project says otherwise, so a document written
/// before pass-through existed restores with exactly its original appearance.
const fn default_isolated() -> bool {
    true
}

pub(crate) fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LAYER_ID_CHARS
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Which kind of content a layer carries.
///
/// Adding a kind means adding a variant here and a branch in the compositor,
/// without changing the tree, mask, transform, or project container. Vector
/// shapes, text and smart objects arrived that way in 0.13.0.
///
/// Every match over this enum is exhaustive on purpose. An unknown variant read
/// from a project must fail the document rather than be reinterpreted as pixels,
/// which is what a catch-all arm would quietly do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayerContent {
    /// Raster content. `pixel_id` refers to an immutable buffer held by the
    /// session pixel store; the tree itself never carries pixels.
    Pixel {
        #[serde(rename = "pixelId")]
        pixel_id: String,
        width: u32,
        height: u32,
    },
    Group {
        #[serde(default)]
        children: Vec<Layer>,
        /// Whether the group composites against its own transparent buffer.
        ///
        /// Isolated (the default, and what every pre-0.8.0 project restores as)
        /// keeps a group's adjustment layers from reaching the backdrop beneath
        /// it. Pass-through lets the children see and modify that backdrop, so
        /// an adjustment inside the group also affects layers below it.
        #[serde(default = "default_isolated")]
        isolated: bool,
    },
    /// Parametric content. The stored `EditOperation` is the same validated
    /// type the destructive pipeline uses, so an adjustment layer never bakes
    /// pixels and always recomputes from its parameters.
    Adjustment { operation: Box<EditOperation> },
    /// Vector content. Geometry, fill and stroke, rasterised on demand into
    /// whatever rectangle is being rendered — never stored as pixels, so it
    /// survives any number of transforms without resampling damage.
    Shape {
        #[serde(flatten)]
        shape: Box<super::shape::ShapeContent>,
    },
    /// Editable text. The characters, the font asked for and the setting —
    /// shaped and outlined on demand, never stored as pixels or as glyph
    /// indices, so the words stay words after a save and reopen.
    Text {
        #[serde(flatten)]
        text: Box<super::text::TextContent>,
    },
    /// An instance of content held in the document's smart source registry.
    ///
    /// The layer holds only the reference. The content is composed at its own
    /// native size and this layer's transform is applied to that composite
    /// afresh on every render, which is what lets an instance be scaled down
    /// and back up without loss — and what separates a smart object from a
    /// raster layer that merely remembers a filename.
    SmartObject {
        #[serde(flatten)]
        smart: Box<super::smart::SmartObjectContent>,
    },
}

impl LayerContent {
    pub const fn kind(&self) -> LayerKind {
        match self {
            Self::Shape { .. } => LayerKind::Shape,
            Self::Text { .. } => LayerKind::Text,
            Self::SmartObject { .. } => LayerKind::SmartObject,
            Self::Pixel { .. } => LayerKind::Pixel,
            Self::Group { .. } => LayerKind::Group,
            Self::Adjustment { .. } => LayerKind::Adjustment,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayerKind {
    Pixel,
    Group,
    Adjustment,
    Shape,
    Text,
    SmartObject,
}

impl LayerKind {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Pixel => "pixel",
            Self::Group => "group",
            Self::Adjustment => "adjustment",
            Self::Shape => "shape",
            Self::Text => "text",
            Self::SmartObject => "smart_object",
        }
    }
}

/// A layer mask, stored in the layer's own pixel space so it moves, scales,
/// rotates, and flips with the layer for free. The coverage bitmap is the
/// canonical Phase 7 `MaskSnapshot`; layers introduce no second mask format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerMask {
    pub snapshot: MaskSnapshot,
    pub enabled: bool,
    #[serde(default)]
    pub inverted: bool,
}

impl LayerMask {
    pub fn validate(&self) -> Result<(), AppError> {
        self.snapshot.decode().map(|_| ())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LayerMetadata {
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub modified_at: String,
    #[serde(default)]
    pub custom: BTreeMap<String, String>,
}

impl LayerMetadata {
    fn validate(&self) -> Result<(), AppError> {
        if self.created_at.len() > MAX_TIMESTAMP_CHARS
            || self.modified_at.len() > MAX_TIMESTAMP_CHARS
        {
            return Err(AppError::InvalidLayerDocument(
                "layer timestamps exceed their length limit".into(),
            ));
        }
        if self.custom.len() > MAX_LAYER_METADATA_ENTRIES {
            return Err(AppError::InvalidLayerDocument(format!(
                "layer metadata is limited to {MAX_LAYER_METADATA_ENTRIES} entries"
            )));
        }
        for (key, value) in &self.custom {
            if key.is_empty()
                || key.len() > MAX_LAYER_ID_CHARS
                || value.len() > MAX_LAYER_METADATA_VALUE_CHARS
            {
                return Err(AppError::InvalidLayerDocument(
                    "layer metadata entry exceeds its length limit".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    pub id: String,
    pub name: String,
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    pub opacity: f32,
    #[serde(default)]
    pub blend_mode: BlendMode,
    #[serde(default)]
    pub transform: LayerTransform,
    #[serde(default)]
    pub mask: Option<LayerMask>,
    #[serde(default)]
    pub collapsed: bool,
    #[serde(default)]
    pub metadata: LayerMetadata,
    /// The camera file this layer was developed from, when it came from one.
    ///
    /// The pixel buffer a RAW layer points at is a cache of one development;
    /// this record and its parameters are what actually define the layer, so a
    /// project reopened later can develop it again rather than inheriting a
    /// baked raster. Absent on every layer that did not come from a RAW file,
    /// and absent in projects written before 0.9.0.
    #[serde(default)]
    pub raw: Option<crate::raw::RawLayerSource>,
    pub content: LayerContent,
}

impl Layer {
    pub fn kind(&self) -> LayerKind {
        self.content.kind()
    }

    pub fn children(&self) -> &[Layer] {
        match &self.content {
            LayerContent::Group { children, .. } => children,
            _ => &[],
        }
    }

    pub fn children_mut(&mut self) -> Option<&mut Vec<Layer>> {
        match &mut self.content {
            LayerContent::Group { children, .. } => Some(children),
            _ => None,
        }
    }

    /// True for a group whose children composite against the backdrop beneath
    /// it rather than against a transparent buffer of their own.
    pub fn is_pass_through(&self) -> bool {
        matches!(
            &self.content,
            LayerContent::Group {
                isolated: false,
                ..
            }
        )
    }

    /// The layer's own pixel dimensions, which need not match the canvas.
    pub fn pixel_dimensions(&self) -> Option<(u32, u32)> {
        match &self.content {
            LayerContent::Pixel { width, height, .. } => Some((*width, *height)),
            _ => None,
        }
    }

    pub fn pixel_id(&self) -> Option<&str> {
        match &self.content {
            LayerContent::Pixel { pixel_id, .. } => Some(pixel_id.as_str()),
            _ => None,
        }
    }

    fn validate_self(&self) -> Result<(), AppError> {
        if !valid_identifier(&self.id) {
            return Err(AppError::InvalidLayerDocument(format!(
                "layer identifiers must contain 1 to {MAX_LAYER_ID_CHARS} characters from A-Z, a-z, 0-9, hyphen, or underscore"
            )));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > MAX_LAYER_NAME_CHARS {
            return Err(AppError::InvalidLayerDocument(format!(
                "layer names must contain 1 to {MAX_LAYER_NAME_CHARS} characters"
            )));
        }
        if !self.opacity.is_finite() || !(0.0..=1.0).contains(&self.opacity) {
            return Err(AppError::InvalidLayerDocument(
                "layer opacity must be a finite value between 0 and 1".into(),
            ));
        }
        self.transform.validate()?;
        self.metadata.validate()?;
        if let Some(raw) = &self.raw {
            raw.validate()
                .map_err(|error| AppError::InvalidLayerDocument(error.to_string()))?;
            if !matches!(self.content, LayerContent::Pixel { .. }) {
                return Err(AppError::InvalidLayerDocument(
                    "only pixel layers can carry a RAW source".into(),
                ));
            }
        }

        match &self.content {
            LayerContent::Shape { shape } => shape.validate()?,
            LayerContent::Text { text } => text.validate()?,
            LayerContent::SmartObject { smart } => smart.validate()?,
            LayerContent::Pixel {
                pixel_id,
                width,
                height,
            } => {
                if !valid_identifier(pixel_id) {
                    return Err(AppError::InvalidLayerDocument(
                        "pixel buffer identifiers use the same character set as layer identifiers"
                            .into(),
                    ));
                }
                validate_dimensions(*width, *height)?;
            }
            LayerContent::Group { isolated, .. } => {
                // Pass-through *is* the group's blend behaviour, so a
                // pass-through group carrying a second blend mode would be
                // ambiguous. Rejecting it keeps the model unambiguous rather
                // than silently picking one meaning.
                if !isolated && self.blend_mode != BlendMode::Normal {
                    return Err(AppError::InvalidLayerDocument(
                        "a pass-through group must use the Normal blend mode".into(),
                    ));
                }
            }
            LayerContent::Adjustment { operation } => {
                if !operation.supports_adjustment_layer() {
                    return Err(AppError::UnsupportedAdjustmentLayer(
                        operation.kind().to_string(),
                    ));
                }
                operation.validate()?;
            }
        }

        if let Some(mask) = &self.mask {
            let decoded = mask.snapshot.decode()?;
            if let Some((width, height)) = self.pixel_dimensions() {
                if (decoded.width(), decoded.height()) != (width, height) {
                    return Err(AppError::MaskDimensionMismatch {
                        mask_width: decoded.width(),
                        mask_height: decoded.height(),
                        image_width: width,
                        image_height: height,
                    });
                }
            }
        }
        Ok(())
    }
}

pub fn validate_dimensions(width: u32, height: u32) -> Result<(), AppError> {
    if width == 0 || height == 0 {
        return Err(AppError::InvalidLayerDocument(
            "layer dimensions must be greater than zero".into(),
        ));
    }
    if width > MAX_CANVAS_DIMENSION || height > MAX_CANVAS_DIMENSION {
        return Err(AppError::InvalidLayerDocument(format!(
            "layer dimensions must not exceed {MAX_CANVAS_DIMENSION} pixels per side"
        )));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| AppError::InvalidLayerDocument("layer dimensions overflow".into()))?;
    if pixels > MAX_CANVAS_PIXELS {
        return Err(AppError::ImageTooLarge {
            pixels,
            limit: MAX_CANVAS_PIXELS,
        });
    }
    Ok(())
}

/// A complete layered document: canvas geometry plus the root layer stack.
///
/// `layers[0]` is the bottom of the stack and is composited first. The Layers
/// panel reverses this for display, matching the convention users expect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerDocument {
    pub schema_version: u32,
    #[serde(default)]
    pub precision: crate::pixel::DocumentPrecision,
    pub canvas_width: u32,
    pub canvas_height: u32,
    #[serde(default)]
    pub layers: Vec<Layer>,
    /// Content that smart object layers are instances of, keyed by identifier.
    ///
    /// Held on the document rather than on the layers so that several instances
    /// genuinely share one thing: editing the source changes every instance,
    /// while each instance keeps its own transform, opacity, blend mode and
    /// mask. Absent in projects written before 0.13.0, which had none.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub smart_sources: super::smart::SmartSources,
    #[serde(default)]
    pub active_layer_id: Option<String>,
}

impl LayerDocument {
    pub fn new(canvas_width: u32, canvas_height: u32) -> Self {
        Self {
            schema_version: LAYER_SCHEMA_VERSION,
            precision: crate::pixel::DocumentPrecision::default(),
            canvas_width,
            canvas_height,
            layers: Vec::new(),
            smart_sources: super::smart::SmartSources::new(),
            active_layer_id: None,
        }
    }

    /// Validates the whole document iteratively, so a malformed or hostile tree
    /// cannot exhaust the stack before its depth is checked.
    pub fn validate(&self) -> Result<(), AppError> {
        if self.schema_version != LAYER_SCHEMA_VERSION {
            return Err(AppError::UnsupportedLayerSchema(self.schema_version));
        }
        validate_dimensions(self.canvas_width, self.canvas_height)?;

        let mut seen: HashSet<&str> = HashSet::new();
        let mut count = 0_usize;
        let mut stack: Vec<(&Layer, usize, u32, u32)> = self
            .layers
            .iter()
            .rev()
            .map(|layer| (layer, 1, self.canvas_width, self.canvas_height))
            .collect();
        // Source trees participate in the same identifier and layer-count
        // budget. Validate all groups iteratively before traversing the smart
        // reference graph, including sources that are not currently placed.
        for source in self.smart_sources.values() {
            source.validate()?;
            stack.extend(
                source
                    .layers
                    .iter()
                    .rev()
                    .map(|layer| (layer, 1, source.width, source.height)),
            );
        }
        while let Some((layer, depth, canvas_width, canvas_height)) = stack.pop() {
            if depth > MAX_GROUP_DEPTH {
                return Err(AppError::LayerDepthExceeded {
                    depth,
                    limit: MAX_GROUP_DEPTH,
                });
            }
            count += 1;
            if count > MAX_LAYERS {
                return Err(AppError::TooManyLayers {
                    count,
                    limit: MAX_LAYERS,
                });
            }
            if !seen.insert(layer.id.as_str()) {
                return Err(AppError::DuplicateLayerId(layer.id.clone()));
            }
            layer.validate_self()?;
            // A group or adjustment layer has no pixels of its own, so its mask
            // lives in canvas space and has to match the canvas exactly.
            if layer.kind() != LayerKind::Pixel {
                let (mask_width, mask_height) = match &layer.content {
                    LayerContent::SmartObject { smart } => self
                        .smart_sources
                        .get(&smart.source_id)
                        .map_or((canvas_width, canvas_height), |s| (s.width, s.height)),
                    _ => (canvas_width, canvas_height),
                };
                if let Some(mask) = &layer.mask {
                    if (mask.snapshot.width, mask.snapshot.height) != (mask_width, mask_height) {
                        return Err(AppError::MaskDimensionMismatch {
                            mask_width: mask.snapshot.width,
                            mask_height: mask.snapshot.height,
                            image_width: mask_width,
                            image_height: mask_height,
                        });
                    }
                }
            }
            for child in layer.children().iter().rev() {
                stack.push((child, depth + 1, canvas_width, canvas_height));
            }
        }

        // Every smart object has to resolve, and no source may contain itself
        // at any depth. Checked after the tree so that a malformed layer is
        // reported as a malformed layer rather than as a broken reference.
        super::smart::validate_sources(&self.layers, &self.smart_sources)?;
        if let Some(active) = &self.active_layer_id {
            if !self.contains(active) {
                return Err(AppError::LayerNotFound(active.clone()));
            }
        }
        Ok(())
    }

    pub fn layer_count(&self) -> usize {
        let mut count = 0;
        let mut stack: Vec<&Layer> = self.layers.iter().collect();
        while let Some(layer) = stack.pop() {
            count += 1;
            stack.extend(layer.children());
        }
        count
    }

    /// Depth-first, bottom-to-top enumeration of every layer in the tree.
    pub fn iter(&self) -> impl Iterator<Item = &Layer> {
        let mut ordered = Vec::new();
        let mut stack: Vec<&Layer> = self.layers.iter().rev().collect();
        while let Some(layer) = stack.pop() {
            ordered.push(layer);
            for child in layer.children().iter().rev() {
                stack.push(child);
            }
        }
        ordered.into_iter()
    }

    pub fn find(&self, id: &str) -> Option<&Layer> {
        self.iter().find(|layer| layer.id == id)
    }

    /// Every stored layer, including each smart source tree exactly once.
    /// Main-document editing uses `iter`; persistence and resource accounting
    /// use this iterator so shared instances do not duplicate source content.
    pub fn iter_all(&self) -> impl Iterator<Item = &Layer> {
        let mut ordered: Vec<&Layer> = self.iter().collect();
        let mut source_ids: Vec<_> = self.smart_sources.keys().collect();
        source_ids.sort_unstable();
        for id in source_ids {
            let mut stack: Vec<&Layer> = self.smart_sources[id].layers.iter().rev().collect();
            while let Some(layer) = stack.pop() {
                ordered.push(layer);
                stack.extend(layer.children().iter().rev());
            }
        }
        ordered.into_iter()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.find(id).is_some()
    }

    /// Every pixel buffer identifier the document currently references.
    pub fn referenced_pixel_ids(&self) -> Vec<String> {
        let mut set: HashSet<String> = self
            .iter()
            .filter_map(|layer| layer.pixel_id().map(str::to_string))
            .collect();
        // Content inside a smart source is not in the tree, and it still has to
        // be resolved or every instance of that source renders blank.
        super::smart::referenced_pixel_ids(&self.smart_sources, &mut set);
        let mut ids: Vec<String> = set.into_iter().collect();
        ids.sort_unstable();
        ids
    }

    /// The layer a root-relative index path points at.
    pub fn layer_at(&self, path: &[usize]) -> Option<&Layer> {
        let mut layers = self.layers.as_slice();
        let mut current = None;
        for step in path {
            let layer = layers.get(*step)?;
            current = Some(layer);
            layers = layer.children();
        }
        current
    }

    /// The index path from the root stack to `id`, used by move and reorder.
    /// The search is iterative so an unvalidated tree cannot exhaust the stack.
    pub fn path_to(&self, id: &str) -> Option<Vec<usize>> {
        let mut stack: Vec<Vec<usize>> = (0..self.layers.len()).rev().map(|i| vec![i]).collect();
        while let Some(path) = stack.pop() {
            let layer = self.layer_at(&path)?;
            if layer.id == id {
                return Some(path);
            }
            for index in (0..layer.children().len()).rev() {
                let mut child = path.clone();
                child.push(index);
                stack.push(child);
            }
        }
        None
    }

    /// True when `candidate` is `ancestor` itself or sits anywhere beneath it.
    /// Move operations consult this to make a group-into-its-own-descendant
    /// move impossible, which is what keeps the tree acyclic.
    pub fn is_self_or_descendant(&self, ancestor: &str, candidate: &str) -> bool {
        let Some(layer) = self.find(ancestor) else {
            return false;
        };
        if ancestor == candidate {
            return true;
        }
        let mut stack: Vec<&Layer> = layer.children().iter().collect();
        while let Some(current) = stack.pop() {
            if current.id == candidate {
                return true;
            }
            stack.extend(current.children());
        }
        false
    }

    fn siblings_mut(&mut self, parent: Option<&str>) -> Result<&mut Vec<Layer>, AppError> {
        match parent {
            None => Ok(&mut self.layers),
            Some(parent_id) => {
                let path = self
                    .path_to(parent_id)
                    .ok_or_else(|| AppError::LayerNotFound(parent_id.to_string()))?;
                let mut layers = &mut self.layers;
                for index in path {
                    layers = layers[index].children_mut().ok_or_else(|| {
                        AppError::InvalidLayerDocument(format!("{parent_id} is not a group layer"))
                    })?;
                }
                Ok(layers)
            }
        }
    }

    /// Inserts `layer` under `parent` at `index`, clamping the index to the
    /// sibling count. Rejects duplicate identifiers and unknown parents.
    pub fn insert(
        &mut self,
        layer: Layer,
        parent: Option<&str>,
        index: usize,
    ) -> Result<(), AppError> {
        let mut incoming: HashSet<&str> = HashSet::new();
        let mut stack = vec![&layer];
        while let Some(current) = stack.pop() {
            if !incoming.insert(current.id.as_str()) {
                return Err(AppError::DuplicateLayerId(current.id.clone()));
            }
            if self.contains(&current.id) {
                return Err(AppError::DuplicateLayerId(current.id.clone()));
            }
            stack.extend(current.children());
        }
        let siblings = self.siblings_mut(parent)?;
        let position = index.min(siblings.len());
        siblings.insert(position, layer);
        self.validate()
    }

    /// Removes `id` and its subtree, returning the detached layer.
    pub fn remove(&mut self, id: &str) -> Result<Layer, AppError> {
        let path = self
            .path_to(id)
            .ok_or_else(|| AppError::LayerNotFound(id.to_string()))?;
        let (parent_path, index) = path.split_at(path.len() - 1);
        let index = index[0];
        let mut layers = &mut self.layers;
        for step in parent_path {
            layers = layers[*step]
                .children_mut()
                .ok_or_else(|| AppError::InvalidLayerDocument("broken layer path".into()))?;
        }
        let removed = layers.remove(index);
        if self.active_layer_id.as_deref() == Some(id) {
            self.active_layer_id = None;
        }
        Ok(removed)
    }

    /// Moves `id` under `parent` at `index`. A group may never become a child
    /// of itself or of one of its own descendants.
    pub fn move_layer(
        &mut self,
        id: &str,
        parent: Option<&str>,
        index: usize,
    ) -> Result<(), AppError> {
        if !self.contains(id) {
            return Err(AppError::LayerNotFound(id.to_string()));
        }
        if let Some(parent_id) = parent {
            if !self.contains(parent_id) {
                return Err(AppError::LayerNotFound(parent_id.to_string()));
            }
            if self.is_self_or_descendant(id, parent_id) {
                return Err(AppError::LayerCycle {
                    layer: id.to_string(),
                    parent: parent_id.to_string(),
                });
            }
            if self
                .find(parent_id)
                .is_some_and(|layer| layer.kind() != LayerKind::Group)
            {
                return Err(AppError::InvalidLayerDocument(format!(
                    "{parent_id} is not a group layer"
                )));
            }
        }
        let active = self.active_layer_id.clone();
        let layer = self.remove(id)?;
        let result = self.insert(layer, parent, index);
        self.active_layer_id = active;
        result
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    use super::*;

    pub fn pixel_layer(id: &str, width: u32, height: u32) -> Layer {
        Layer {
            id: id.to_string(),
            name: id.to_string(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            transform: LayerTransform::default(),
            mask: None,
            collapsed: false,
            metadata: LayerMetadata::default(),
            raw: None,
            content: LayerContent::Pixel {
                pixel_id: format!("px{id}"),
                width,
                height,
            },
        }
    }

    pub fn pass_through_group(id: &str, children: Vec<Layer>) -> Layer {
        let mut layer = group_layer(id, children);
        layer.content = LayerContent::Group {
            children: layer.children().to_vec(),
            isolated: false,
        };
        layer
    }

    pub fn group_layer(id: &str, children: Vec<Layer>) -> Layer {
        Layer {
            id: id.to_string(),
            name: id.to_string(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            transform: LayerTransform::default(),
            mask: None,
            collapsed: false,
            metadata: LayerMetadata::default(),
            raw: None,
            content: LayerContent::Group {
                children,
                isolated: true,
            },
        }
    }

    pub fn adjustment_layer(id: &str, operation: EditOperation) -> Layer {
        Layer {
            id: id.to_string(),
            name: id.to_string(),
            visible: true,
            locked: false,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            transform: LayerTransform::default(),
            mask: None,
            collapsed: false,
            metadata: LayerMetadata::default(),
            raw: None,
            content: LayerContent::Adjustment {
                operation: Box::new(operation),
            },
        }
    }

    pub fn document(layers: Vec<Layer>) -> LayerDocument {
        LayerDocument {
            schema_version: LAYER_SCHEMA_VERSION,
            precision: Default::default(),
            canvas_width: 16,
            canvas_height: 16,
            layers,
            smart_sources: Default::default(),
            active_layer_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn a_new_document_validates_and_reports_no_layers() {
        let document = LayerDocument::new(64, 32);
        document.validate().unwrap();
        assert_eq!(document.layer_count(), 0);
        assert_eq!(document.schema_version, LAYER_SCHEMA_VERSION);
    }

    #[test]
    fn identifiers_stay_stable_across_reordering() {
        let mut document = document(vec![pixel_layer("a", 16, 16), pixel_layer("b", 16, 16)]);
        document.move_layer("a", None, 1).unwrap();
        let ordered: Vec<&str> = document.iter().map(|layer| layer.id.as_str()).collect();
        assert_eq!(ordered, vec!["b", "a"]);
        assert!(document.find("a").is_some());
    }

    #[test]
    fn root_order_places_index_zero_at_the_bottom() {
        let document = document(vec![
            pixel_layer("bottom", 16, 16),
            pixel_layer("top", 16, 16),
        ]);
        assert_eq!(document.layers[0].id, "bottom");
        let ordered: Vec<&str> = document.iter().map(|layer| layer.id.as_str()).collect();
        assert_eq!(ordered, vec!["bottom", "top"]);
    }

    #[test]
    fn duplicate_identifiers_are_rejected_by_validation_and_insertion() {
        let collided = document(vec![pixel_layer("a", 16, 16), pixel_layer("a", 16, 16)]);
        assert!(matches!(
            collided.validate(),
            Err(AppError::DuplicateLayerId(_))
        ));

        let mut existing = document(vec![pixel_layer("a", 16, 16)]);
        assert!(matches!(
            existing.insert(pixel_layer("a", 16, 16), None, 0),
            Err(AppError::DuplicateLayerId(_))
        ));
    }

    #[test]
    fn inserting_a_group_whose_children_collide_is_rejected() {
        let mut document = document(vec![pixel_layer("a", 16, 16)]);
        let incoming = group_layer("g", vec![pixel_layer("a", 16, 16)]);
        assert!(matches!(
            document.insert(incoming, None, 0),
            Err(AppError::DuplicateLayerId(_))
        ));
        assert_eq!(document.layer_count(), 1);
    }

    #[test]
    fn nested_groups_traverse_depth_first_from_the_bottom() {
        let document = document(vec![
            pixel_layer("base", 16, 16),
            group_layer(
                "outer",
                vec![
                    pixel_layer("inner-bottom", 16, 16),
                    group_layer("nested", vec![pixel_layer("deep", 16, 16)]),
                ],
            ),
        ]);
        document.validate().unwrap();
        let ordered: Vec<&str> = document.iter().map(|layer| layer.id.as_str()).collect();
        assert_eq!(
            ordered,
            vec!["base", "outer", "inner-bottom", "nested", "deep"]
        );
        assert_eq!(document.layer_count(), 5);
    }

    #[test]
    fn a_group_cannot_become_a_child_of_itself_or_its_descendant() {
        let mut document = document(vec![group_layer(
            "outer",
            vec![group_layer("inner", vec![pixel_layer("leaf", 16, 16)])],
        )]);
        assert!(matches!(
            document.move_layer("outer", Some("outer"), 0),
            Err(AppError::LayerCycle { .. })
        ));
        assert!(matches!(
            document.move_layer("outer", Some("inner"), 0),
            Err(AppError::LayerCycle { .. })
        ));
        // The rejected moves left the tree untouched.
        document.validate().unwrap();
        assert_eq!(document.layer_count(), 3);
        assert!(document.path_to("leaf").is_some());
    }

    #[test]
    fn moving_into_and_out_of_a_group_preserves_every_layer() {
        let mut document = document(vec![
            pixel_layer("free", 16, 16),
            group_layer("g", vec![pixel_layer("member", 16, 16)]),
        ]);
        document.move_layer("free", Some("g"), 0).unwrap();
        assert_eq!(document.find("g").unwrap().children().len(), 2);
        assert_eq!(document.layers.len(), 1);

        document.move_layer("free", None, 0).unwrap();
        assert_eq!(document.find("g").unwrap().children().len(), 1);
        assert_eq!(document.layers.len(), 2);
        assert_eq!(document.layers[0].id, "free");
        document.validate().unwrap();
    }

    #[test]
    fn moving_a_whole_group_carries_its_children() {
        let mut document = document(vec![
            group_layer("target", vec![]),
            group_layer("source", vec![pixel_layer("child", 16, 16)]),
        ]);
        document.move_layer("source", Some("target"), 0).unwrap();
        document.validate().unwrap();
        assert_eq!(document.layer_count(), 3);
        assert_eq!(document.path_to("child").unwrap(), vec![0, 0, 0]);
    }

    #[test]
    fn moving_onto_a_non_group_parent_is_rejected() {
        let mut document = document(vec![pixel_layer("a", 16, 16), pixel_layer("b", 16, 16)]);
        assert!(document.move_layer("a", Some("b"), 0).is_err());
        assert_eq!(document.layers.len(), 2);
    }

    #[test]
    fn missing_parents_and_layers_fail_without_mutating_the_tree() {
        let mut document = document(vec![pixel_layer("a", 16, 16)]);
        assert!(matches!(
            document.move_layer("a", Some("ghost"), 0),
            Err(AppError::LayerNotFound(_))
        ));
        assert!(matches!(
            document.move_layer("ghost", None, 0),
            Err(AppError::LayerNotFound(_))
        ));
        assert!(matches!(
            document.remove("ghost"),
            Err(AppError::LayerNotFound(_))
        ));
        assert!(matches!(
            document.insert(pixel_layer("b", 16, 16), Some("ghost"), 0),
            Err(AppError::LayerNotFound(_))
        ));
        assert_eq!(document.layer_count(), 1);
    }

    #[test]
    fn insert_indices_are_clamped_to_the_sibling_count() {
        let mut document = document(vec![pixel_layer("a", 16, 16)]);
        document.insert(pixel_layer("b", 16, 16), None, 99).unwrap();
        assert_eq!(document.layers[1].id, "b");
    }

    #[test]
    fn removing_a_layer_clears_a_stale_active_selection() {
        let mut document = document(vec![pixel_layer("a", 16, 16)]);
        document.active_layer_id = Some("a".into());
        document.remove("a").unwrap();
        assert!(document.active_layer_id.is_none());
        document.validate().unwrap();
    }

    #[test]
    fn an_active_layer_that_does_not_exist_is_rejected() {
        let mut document = document(vec![pixel_layer("a", 16, 16)]);
        document.active_layer_id = Some("ghost".into());
        assert!(matches!(
            document.validate(),
            Err(AppError::LayerNotFound(_))
        ));
    }

    #[test]
    fn group_depth_beyond_the_limit_is_rejected() {
        let mut layer = pixel_layer("leaf", 8, 8);
        for index in 0..MAX_GROUP_DEPTH {
            layer = group_layer(&format!("g{index}"), vec![layer]);
        }
        let document = document(vec![layer]);
        assert!(matches!(
            document.validate(),
            Err(AppError::LayerDepthExceeded { .. })
        ));
    }

    #[test]
    fn group_depth_at_the_limit_is_accepted() {
        let mut layer = pixel_layer("leaf", 8, 8);
        for index in 0..(MAX_GROUP_DEPTH - 1) {
            layer = group_layer(&format!("g{index}"), vec![layer]);
        }
        document(vec![layer]).validate().unwrap();
    }

    #[test]
    fn layer_counts_beyond_the_limit_are_rejected() {
        let layers: Vec<Layer> = (0..=MAX_LAYERS)
            .map(|index| pixel_layer(&format!("l{index}"), 4, 4))
            .collect();
        assert!(matches!(
            document(layers).validate(),
            Err(AppError::TooManyLayers { .. })
        ));
    }

    #[test]
    fn invalid_opacity_names_and_dimensions_are_rejected() {
        let mut layer = pixel_layer("a", 16, 16);
        layer.opacity = 1.5;
        assert!(document(vec![layer]).validate().is_err());

        let mut layer = pixel_layer("a", 16, 16);
        layer.opacity = f32::NAN;
        assert!(document(vec![layer]).validate().is_err());

        let mut layer = pixel_layer("a", 16, 16);
        layer.name = "   ".into();
        assert!(document(vec![layer]).validate().is_err());

        let mut layer = pixel_layer("a", 16, 16);
        layer.id = "has space".into();
        assert!(document(vec![layer]).validate().is_err());

        let mut layer = pixel_layer("a", 16, 16);
        layer.content = LayerContent::Pixel {
            pixel_id: "px".into(),
            width: 0,
            height: 16,
        };
        assert!(document(vec![layer]).validate().is_err());
    }

    #[test]
    fn oversized_layer_dimensions_are_rejected_before_allocation() {
        let mut layer = pixel_layer("a", 16, 16);
        layer.content = LayerContent::Pixel {
            pixel_id: "px".into(),
            width: 30_000,
            height: 30_000,
        };
        assert!(document(vec![layer]).validate().is_err());

        let mut layer = pixel_layer("a", 16, 16);
        layer.content = LayerContent::Pixel {
            pixel_id: "px".into(),
            width: 19_000,
            height: 19_000,
        };
        assert!(matches!(
            document(vec![layer]).validate(),
            Err(AppError::ImageTooLarge { .. })
        ));
    }

    #[test]
    fn transform_validation_runs_for_every_layer() {
        let mut layer = pixel_layer("a", 16, 16);
        layer.transform.scale_x = f32::NAN;
        assert!(matches!(
            document(vec![layer]).validate(),
            Err(AppError::InvalidLayerTransform(_))
        ));
    }

    #[test]
    fn unsupported_schema_versions_are_rejected() {
        let mut document = document(vec![pixel_layer("a", 16, 16)]);
        document.schema_version = LAYER_SCHEMA_VERSION + 1;
        assert!(matches!(
            document.validate(),
            Err(AppError::UnsupportedLayerSchema(_))
        ));
    }

    #[test]
    fn adjustment_layers_accept_parametric_operations_and_reject_geometry() {
        let allowed = adjustment_layer("adj", EditOperation::Brightness { amount: 0.2 });
        document(vec![allowed]).validate().unwrap();

        for operation in [
            EditOperation::Rotate { degrees: 90 },
            EditOperation::Crop {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                aspect_ratio: None,
                overlay: crate::domain::CropOverlay::None,
            },
            EditOperation::ReflectHorizontal,
            EditOperation::DecontaminateColors {
                enabled: true,
                strength: 0.5,
                radius: 4,
            },
        ] {
            let layer = adjustment_layer("adj", operation);
            assert!(matches!(
                document(vec![layer]).validate(),
                Err(AppError::UnsupportedAdjustmentLayer(_))
            ));
        }
    }

    #[test]
    fn adjustment_parameters_are_validated_like_pipeline_operations() {
        let layer = adjustment_layer("adj", EditOperation::Gamma { value: 0.0 });
        assert!(matches!(
            document(vec![layer]).validate(),
            Err(AppError::InvalidOperation(_))
        ));
    }

    #[test]
    fn referenced_pixel_ids_are_deduplicated_and_sorted() {
        let mut first = pixel_layer("a", 8, 8);
        first.content = LayerContent::Pixel {
            pixel_id: "shared".into(),
            width: 8,
            height: 8,
        };
        let mut second = pixel_layer("b", 8, 8);
        second.content = LayerContent::Pixel {
            pixel_id: "shared".into(),
            width: 8,
            height: 8,
        };
        let document = document(vec![
            first,
            second,
            group_layer("g", vec![pixel_layer("c", 8, 8)]),
            adjustment_layer("adj", EditOperation::Sepia),
        ]);
        assert_eq!(
            document.referenced_pixel_ids(),
            vec!["pxc".to_string(), "shared".to_string()]
        );
    }

    #[test]
    fn documents_round_trip_through_camel_case_json() {
        let document = document(vec![
            pixel_layer("base", 16, 16),
            group_layer(
                "g",
                vec![adjustment_layer(
                    "adj",
                    EditOperation::Contrast { amount: 0.2 },
                )],
            ),
        ]);
        let json = serde_json::to_string(&document).unwrap();
        assert!(json.contains("schemaVersion"));
        assert!(json.contains("canvasWidth"));
        assert!(json.contains("pixelId"));
        assert!(json.contains("blendMode"));
        let decoded: LayerDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, document);
        decoded.validate().unwrap();
    }

    #[test]
    fn metadata_limits_are_enforced() {
        let mut layer = pixel_layer("a", 16, 16);
        for index in 0..(MAX_LAYER_METADATA_ENTRIES + 1) {
            layer
                .metadata
                .custom
                .insert(format!("key{index}"), "value".into());
        }
        assert!(document(vec![layer]).validate().is_err());

        let mut layer = pixel_layer("a", 16, 16);
        layer
            .metadata
            .custom
            .insert("key".into(), "x".repeat(MAX_LAYER_METADATA_VALUE_CHARS + 1));
        assert!(document(vec![layer]).validate().is_err());
    }

    #[test]
    fn a_layer_mask_must_match_its_own_pixel_dimensions() {
        use crate::mask::MaskBitmap;
        let mut layer = pixel_layer("a", 16, 16);
        layer.mask = Some(LayerMask {
            snapshot: MaskSnapshot::encode(&MaskBitmap::full(8, 8).unwrap()),
            enabled: true,
            inverted: false,
        });
        assert!(matches!(
            document(vec![layer]).validate(),
            Err(AppError::MaskDimensionMismatch { .. })
        ));

        let mut layer = pixel_layer("a", 16, 16);
        layer.mask = Some(LayerMask {
            snapshot: MaskSnapshot::encode(&MaskBitmap::full(16, 16).unwrap()),
            enabled: true,
            inverted: false,
        });
        document(vec![layer]).validate().unwrap();
    }

    #[test]
    fn a_corrupt_mask_payload_is_rejected_rather_than_decoded() {
        use crate::mask::MaskBitmap;
        let mut snapshot = MaskSnapshot::encode(&MaskBitmap::full(16, 16).unwrap());
        snapshot.checksum = "fnv1a64:0000000000000000".into();
        let mut layer = pixel_layer("a", 16, 16);
        layer.mask = Some(LayerMask {
            snapshot,
            enabled: true,
            inverted: false,
        });
        assert!(matches!(
            document(vec![layer]).validate(),
            Err(AppError::InvalidMask(_))
        ));
    }

    #[test]
    fn deeply_nested_json_is_rejected_by_the_parser_before_it_reaches_validation() {
        let mut json = String::new();
        for _ in 0..200 {
            json.push_str(
                "{\"id\":\"g\",\"name\":\"g\",\"visible\":true,\"opacity\":1.0,\"content\":{\"type\":\"group\",\"children\":[",
            );
        }
        json.push_str(
            "{\"id\":\"l\",\"name\":\"l\",\"visible\":true,\"opacity\":1.0,\"content\":{\"type\":\"pixel\",\"pixelId\":\"p\",\"width\":4,\"height\":4}}",
        );
        for _ in 0..200 {
            json.push_str("]}}");
        }
        assert!(serde_json::from_str::<Layer>(&json).is_err());
    }

    #[test]
    fn find_and_path_lookups_agree_for_nested_layers() {
        let document = document(vec![
            pixel_layer("a", 8, 8),
            group_layer("g", vec![pixel_layer("b", 8, 8), pixel_layer("c", 8, 8)]),
        ]);
        assert_eq!(document.path_to("a").unwrap(), vec![0]);
        assert_eq!(document.path_to("g").unwrap(), vec![1]);
        assert_eq!(document.path_to("c").unwrap(), vec![1, 1]);
        assert!(document.path_to("missing").is_none());
        assert_eq!(document.find("c").unwrap().name, "c");
    }

    #[test]
    fn descendant_checks_cover_self_children_and_unrelated_layers() {
        let document = document(vec![
            group_layer("g", vec![group_layer("h", vec![pixel_layer("leaf", 8, 8)])]),
            pixel_layer("other", 8, 8),
        ]);
        assert!(document.is_self_or_descendant("g", "g"));
        assert!(document.is_self_or_descendant("g", "h"));
        assert!(document.is_self_or_descendant("g", "leaf"));
        assert!(!document.is_self_or_descendant("g", "other"));
        assert!(!document.is_self_or_descendant("missing", "leaf"));
    }
}
