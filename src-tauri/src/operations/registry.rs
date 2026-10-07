//! The registry: every operation that may edit a document, by name.
//!
//! An operation is identified by a namespaced id (`core.layer.set_opacity`) and a
//! version. Ids are the stable thing: a recorded macro, a plugin's request and a
//! planner's output name operations by id, so renaming one would break stored
//! work and is therefore a version bump with the old id kept, never an edit.
//!
//! The registry is a closed set in this build. A plugin does **not** add
//! operations to it. A plugin contributes a *filter* that one registered
//! operation, `core.plugin.apply_filter`, runs with the plugin's declared
//! authority; and it contributes *commands* that issue registered operations like
//! anyone else. Keeping the document-editing vocabulary closed is what lets every
//! validator, every automation editor and every planner know all of it.
//!
//! Parameters are typed structs with `deny_unknown_fields`, so a misspelt or
//! surplus parameter is an error rather than silently ignored.
use super::model::OperationCall;
use crate::domain::EditOperation;
use crate::error::AppError;
use crate::layers::workflow::LayerSelector;
use crate::layers::BlendMode;
use serde::{Deserialize, Serialize};

/// What an operation touches beyond the layer tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Effects {
    /// The tree only. Nothing outside the document value changes.
    None,
    /// Registers pixel buffers in the session store. A transaction journals them
    /// and discards them if it fails.
    Pixels,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    Layer,
    Adjustment,
    Pixels,
    Document,
    Plugin,
}

/// Whether a lock on the target blocks the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LockPolicy {
    /// Never: the operation is how a lock is changed, or does not act on the layer.
    Ignored,
    /// An automation, planner, plugin or batch run is stopped by a lock; a person
    /// acting directly is not.
    AutomationOnly,
    /// Everyone is stopped: the operation rewrites the layer's pixels or removes it.
    Always,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ParamKind {
    Selector,
    Selectors,
    OptionalSelector,
    #[serde(rename_all = "camelCase")]
    Text {
        max_chars: usize,
    },
    Bool,
    #[serde(rename_all = "camelCase")]
    Number {
        min: f64,
        max: f64,
    },
    #[serde(rename_all = "camelCase")]
    Integer {
        min: i64,
        max: i64,
    },
    BlendMode,
    EditOperation,
    EditOperations,
    Json,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamSpec {
    pub name: &'static str,
    pub kind: ParamKind,
    pub required: bool,
    pub description: &'static str,
}

fn param(
    name: &'static str,
    kind: ParamKind,
    required: bool,
    description: &'static str,
) -> ParamSpec {
    ParamSpec {
        name,
        kind,
        required,
        description,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationSpec {
    pub id: &'static str,
    pub version: u32,
    pub title: &'static str,
    pub summary: &'static str,
    pub category: Category,
    pub effects: Effects,
    pub lock: LockPolicy,
    /// Whether a planner may propose it. Destructive and structural operations
    /// are decisions, not suggestions.
    pub planner_safe: bool,
    pub needs_selection: bool,
    pub params: Vec<ParamSpec>,
}

/// Every registered operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Select,
    SetVisible,
    SetLocked,
    SetOpacity,
    SetBlendMode,
    Rename,
    SetCollapsed,
    Move,
    Delete,
    Duplicate,
    Group,
    Ungroup,
    ResetTransform,
    AddGroup,
    AddAdjustment,
    AddPixel,
    ApplyEdit,
    MaskFromSelection,
    MergeDown,
    Flatten,
    ApplyPluginFilter,
    AddPluginAdjustment,
}

impl OperationKind {
    pub const ALL: [OperationKind; 22] = [
        Self::Select,
        Self::SetVisible,
        Self::SetLocked,
        Self::SetOpacity,
        Self::SetBlendMode,
        Self::Rename,
        Self::SetCollapsed,
        Self::Move,
        Self::Delete,
        Self::Duplicate,
        Self::Group,
        Self::Ungroup,
        Self::ResetTransform,
        Self::AddGroup,
        Self::AddAdjustment,
        Self::AddPixel,
        Self::ApplyEdit,
        Self::MaskFromSelection,
        Self::MergeDown,
        Self::Flatten,
        Self::ApplyPluginFilter,
        Self::AddPluginAdjustment,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Select => "core.layer.select",
            Self::SetVisible => "core.layer.set_visible",
            Self::SetLocked => "core.layer.set_locked",
            Self::SetOpacity => "core.layer.set_opacity",
            Self::SetBlendMode => "core.layer.set_blend_mode",
            Self::Rename => "core.layer.rename",
            Self::SetCollapsed => "core.layer.set_collapsed",
            Self::Move => "core.layer.move",
            Self::Delete => "core.layer.delete",
            Self::Duplicate => "core.layer.duplicate",
            Self::Group => "core.layer.group",
            Self::Ungroup => "core.layer.ungroup",
            Self::ResetTransform => "core.layer.reset_transform",
            Self::AddGroup => "core.layer.add_group",
            Self::AddAdjustment => "core.layer.add_adjustment",
            Self::AddPixel => "core.layer.add_pixel",
            Self::ApplyEdit => "core.layer.apply_edit",
            Self::MaskFromSelection => "core.layer.mask_from_selection",
            Self::MergeDown => "core.layer.merge_down",
            Self::Flatten => "core.document.flatten",
            Self::ApplyPluginFilter => "core.plugin.apply_filter",
            Self::AddPluginAdjustment => "core.plugin.add_adjustment",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }

    pub fn spec(self) -> OperationSpec {
        let selector = || {
            param(
                "selector",
                ParamKind::Selector,
                true,
                "The layer to act on: by identifier, name, or relative to the document.",
            )
        };
        let base = |title: &'static str,
                    summary: &'static str,
                    category: Category,
                    effects: Effects,
                    lock: LockPolicy,
                    planner_safe: bool,
                    params: Vec<ParamSpec>| OperationSpec {
            id: self.id(),
            version: 1,
            title,
            summary,
            category,
            effects,
            lock,
            planner_safe,
            needs_selection: false,
            params,
        };
        use Category::{Adjustment, Document, Layer, Pixels, Plugin};
        use Effects::{None as NoEffect, Pixels as PixelEffect};
        use LockPolicy::{Always, AutomationOnly, Ignored};
        match self {
            Self::Select => base(
                "Select layer",
                "Makes a layer the active one.",
                Layer,
                NoEffect,
                Ignored,
                true,
                vec![selector()],
            ),
            Self::SetVisible => base(
                "Show or hide layer",
                "Sets whether a layer is drawn.",
                Layer,
                NoEffect,
                AutomationOnly,
                true,
                vec![
                    selector(),
                    param(
                        "visible",
                        ParamKind::Bool,
                        true,
                        "Whether the layer is drawn.",
                    ),
                ],
            ),
            Self::SetLocked => base(
                "Lock or unlock layer",
                "Sets whether a layer is protected from automation.",
                Layer,
                NoEffect,
                Ignored,
                false,
                vec![
                    selector(),
                    param(
                        "locked",
                        ParamKind::Bool,
                        true,
                        "Whether the layer is locked.",
                    ),
                ],
            ),
            Self::SetOpacity => base(
                "Set layer opacity",
                "Sets how opaque a layer is, from 0 to 1.",
                Layer,
                NoEffect,
                AutomationOnly,
                true,
                vec![
                    selector(),
                    param(
                        "opacity",
                        ParamKind::Number { min: 0.0, max: 1.0 },
                        true,
                        "Opacity from 0 (transparent) to 1 (opaque).",
                    ),
                ],
            ),
            Self::SetBlendMode => base(
                "Set blend mode",
                "Sets how a layer combines with what is beneath it.",
                Layer,
                NoEffect,
                AutomationOnly,
                true,
                vec![
                    selector(),
                    param("blendMode", ParamKind::BlendMode, true, "The blend mode."),
                ],
            ),
            Self::Rename => base(
                "Rename layer",
                "Changes a layer's name.",
                Layer,
                NoEffect,
                AutomationOnly,
                false,
                vec![
                    selector(),
                    param(
                        "name",
                        ParamKind::Text { max_chars: 120 },
                        true,
                        "The new name.",
                    ),
                ],
            ),
            Self::SetCollapsed => base(
                "Collapse or expand group",
                "Sets whether a group is shown collapsed in the Layers panel.",
                Layer,
                NoEffect,
                Ignored,
                false,
                vec![
                    selector(),
                    param(
                        "collapsed",
                        ParamKind::Bool,
                        true,
                        "Whether the group is collapsed.",
                    ),
                ],
            ),
            Self::Move => base(
                "Move layer",
                "Moves a layer to a position in a stack, optionally inside a group.",
                Layer,
                NoEffect,
                AutomationOnly,
                false,
                vec![
                    selector(),
                    param(
                        "parent",
                        ParamKind::OptionalSelector,
                        false,
                        "The group to move it into; absent for the top level.",
                    ),
                    param(
                        "index",
                        ParamKind::Integer { min: 0, max: 511 },
                        true,
                        "Position among its new siblings, 0 being the bottom.",
                    ),
                ],
            ),
            Self::Delete => base(
                "Delete layer",
                "Removes a layer and everything inside it.",
                Layer,
                NoEffect,
                Always,
                false,
                vec![selector()],
            ),
            Self::Duplicate => base(
                "Duplicate layer",
                "Copies a layer and everything inside it, just above the original.",
                Layer,
                NoEffect,
                AutomationOnly,
                false,
                vec![selector()],
            ),
            Self::Group => base(
                "Group layers",
                "Wraps layers that share a parent in a new group.",
                Layer,
                NoEffect,
                AutomationOnly,
                false,
                vec![
                    param(
                        "selectors",
                        ParamKind::Selectors,
                        true,
                        "The layers to group.",
                    ),
                    param(
                        "name",
                        ParamKind::Text { max_chars: 120 },
                        false,
                        "The group's name; \"Group\" if absent.",
                    ),
                ],
            ),
            Self::Ungroup => base(
                "Ungroup",
                "Replaces a group with the layers inside it.",
                Layer,
                NoEffect,
                AutomationOnly,
                false,
                vec![selector()],
            ),
            Self::ResetTransform => base(
                "Reset layer transform",
                "Returns a layer's placement, scale and rotation to the identity.",
                Layer,
                NoEffect,
                AutomationOnly,
                false,
                vec![selector()],
            ),
            Self::AddGroup => base(
                "New group",
                "Adds an empty group at the top of the stack.",
                Layer,
                NoEffect,
                Ignored,
                false,
                vec![param(
                    "name",
                    ParamKind::Text { max_chars: 120 },
                    false,
                    "The group's name; \"Group\" if absent.",
                )],
            ),
            Self::AddAdjustment => base(
                "New adjustment layer",
                "Adds a non-destructive adjustment at the top of the stack.",
                Adjustment,
                NoEffect,
                Ignored,
                true,
                vec![
                    param(
                        "operation",
                        ParamKind::EditOperation,
                        true,
                        "The adjustment the layer applies.",
                    ),
                    param(
                        "name",
                        ParamKind::Text { max_chars: 120 },
                        false,
                        "The layer's name; \"Adjustment\" if absent.",
                    ),
                ],
            ),
            Self::AddPixel => base(
                "New pixel layer",
                "Adds an empty, transparent pixel layer the size of the canvas.",
                Pixels,
                PixelEffect,
                Ignored,
                false,
                vec![param(
                    "name",
                    ParamKind::Text { max_chars: 120 },
                    false,
                    "The layer's name; \"Layer\" if absent.",
                )],
            ),
            Self::ApplyEdit => base(
                "Apply edits to layer",
                "Applies adjustments to a pixel layer's pixels, producing new pixels.",
                Pixels,
                PixelEffect,
                Always,
                true,
                vec![
                    selector(),
                    param(
                        "operations",
                        ParamKind::EditOperations,
                        true,
                        "The edits to apply, in order.",
                    ),
                ],
            ),
            Self::MaskFromSelection => OperationSpec {
                needs_selection: true,
                ..base(
                    "Mask from selection",
                    "Gives a layer a mask made from the current selection.",
                    Layer,
                    NoEffect,
                    Always,
                    true,
                    vec![selector()],
                )
            },
            Self::MergeDown => base(
                "Merge down",
                "Merges a pixel layer into the layer beneath it.",
                Pixels,
                PixelEffect,
                Always,
                false,
                vec![selector()],
            ),
            Self::Flatten => base(
                "Flatten document",
                "Merges every layer into a single background layer.",
                Document,
                PixelEffect,
                Always,
                false,
                Vec::new(),
            ),
            Self::ApplyPluginFilter => base(
                "Apply plugin filter",
                "Runs a plugin's filter on a pixel layer, producing new pixels.",
                Plugin,
                PixelEffect,
                Always,
                false,
                vec![
                    selector(),
                    param(
                        "plugin",
                        ParamKind::Text { max_chars: 64 },
                        true,
                        "The plugin's id.",
                    ),
                    param(
                        "filter",
                        ParamKind::Text { max_chars: 64 },
                        true,
                        "The filter's id within the plugin.",
                    ),
                    param(
                        "parameters",
                        ParamKind::Json,
                        false,
                        "The filter's parameters by name, as the plugin declared them.",
                    ),
                ],
            ),
            Self::AddPluginAdjustment => base(
                "New plugin adjustment layer",
                "Adds a non-destructive adjustment layer that runs a plugin's filter.",
                Plugin,
                NoEffect,
                Ignored,
                false,
                vec![
                    param(
                        "plugin",
                        ParamKind::Text { max_chars: 64 },
                        true,
                        "The plugin's id.",
                    ),
                    param(
                        "filter",
                        ParamKind::Text { max_chars: 64 },
                        true,
                        "The filter's id within the plugin.",
                    ),
                    param(
                        "parameters",
                        ParamKind::Json,
                        false,
                        "The filter's parameters by name, as the plugin declared them.",
                    ),
                    param(
                        "name",
                        ParamKind::Text { max_chars: 120 },
                        false,
                        "The layer's name; the filter's title if absent.",
                    ),
                ],
            ),
        }
    }
}

/// The specs of every operation, for the interface to draw its palette and its
/// automation editor from.
pub fn operation_specs() -> Vec<OperationSpec> {
    OperationKind::ALL
        .into_iter()
        .map(OperationKind::spec)
        .collect()
}

// ---- typed parameters -----------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectorParams {
    pub selector: LayerSelector,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisibleParams {
    pub selector: LayerSelector,
    pub visible: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LockedParams {
    pub selector: LayerSelector,
    pub locked: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OpacityParams {
    pub selector: LayerSelector,
    pub opacity: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BlendParams {
    pub selector: LayerSelector,
    pub blend_mode: BlendMode,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RenameParams {
    pub selector: LayerSelector,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CollapsedParams {
    pub selector: LayerSelector,
    pub collapsed: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MoveParams {
    pub selector: LayerSelector,
    #[serde(default)]
    pub parent: Option<LayerSelector>,
    pub index: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GroupParams {
    pub selectors: Vec<LayerSelector>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NameParams {
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdjustmentParams {
    pub operation: Box<EditOperation>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApplyEditParams {
    pub selector: LayerSelector,
    pub operations: Vec<EditOperation>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginFilterParams {
    pub selector: LayerSelector,
    pub plugin: String,
    pub filter: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginAdjustmentParams {
    pub plugin: String,
    pub filter: String,
    #[serde(default)]
    pub parameters: serde_json::Value,
    #[serde(default)]
    pub name: Option<String>,
}

/// A call whose parameters have been parsed.
#[derive(Debug, Clone)]
pub enum Operation {
    Select(SelectorParams),
    SetVisible(VisibleParams),
    SetLocked(LockedParams),
    SetOpacity(OpacityParams),
    SetBlendMode(BlendParams),
    Rename(RenameParams),
    SetCollapsed(CollapsedParams),
    Move(MoveParams),
    Delete(SelectorParams),
    Duplicate(SelectorParams),
    Group(GroupParams),
    Ungroup(SelectorParams),
    ResetTransform(SelectorParams),
    AddGroup(NameParams),
    AddAdjustment(AdjustmentParams),
    AddPixel(NameParams),
    ApplyEdit(ApplyEditParams),
    MaskFromSelection(SelectorParams),
    MergeDown(SelectorParams),
    Flatten,
    ApplyPluginFilter(PluginFilterParams),
    AddPluginAdjustment(PluginAdjustmentParams),
}

fn parse<T: for<'de> Deserialize<'de>>(call: &OperationCall) -> Result<T, AppError> {
    // An absent `params` is an empty object, so an operation with no parameters
    // can be written either way.
    let value = if call.params.is_null() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        call.params.clone()
    };
    serde_json::from_value(value)
        .map_err(|error| AppError::InvalidOperation(format!("{}: {error}", call.op)))
}

impl Operation {
    /// Turns a call into a typed operation, or refuses it.
    pub fn parse(call: &OperationCall) -> Result<Self, AppError> {
        let kind = OperationKind::from_id(&call.op)
            .ok_or_else(|| AppError::UnknownOperation(call.op.clone()))?;
        Ok(match kind {
            OperationKind::Select => Self::Select(parse(call)?),
            OperationKind::SetVisible => Self::SetVisible(parse(call)?),
            OperationKind::SetLocked => Self::SetLocked(parse(call)?),
            OperationKind::SetOpacity => Self::SetOpacity(parse(call)?),
            OperationKind::SetBlendMode => Self::SetBlendMode(parse(call)?),
            OperationKind::Rename => Self::Rename(parse(call)?),
            OperationKind::SetCollapsed => Self::SetCollapsed(parse(call)?),
            OperationKind::Move => Self::Move(parse(call)?),
            OperationKind::Delete => Self::Delete(parse(call)?),
            OperationKind::Duplicate => Self::Duplicate(parse(call)?),
            OperationKind::Group => Self::Group(parse(call)?),
            OperationKind::Ungroup => Self::Ungroup(parse(call)?),
            OperationKind::ResetTransform => Self::ResetTransform(parse(call)?),
            OperationKind::AddGroup => Self::AddGroup(parse(call)?),
            OperationKind::AddAdjustment => Self::AddAdjustment(parse(call)?),
            OperationKind::AddPixel => Self::AddPixel(parse(call)?),
            OperationKind::ApplyEdit => Self::ApplyEdit(parse(call)?),
            OperationKind::MaskFromSelection => Self::MaskFromSelection(parse(call)?),
            OperationKind::MergeDown => Self::MergeDown(parse(call)?),
            OperationKind::Flatten => {
                let _: EmptyParams = parse(call)?;
                Self::Flatten
            }
            OperationKind::ApplyPluginFilter => Self::ApplyPluginFilter(parse(call)?),
            OperationKind::AddPluginAdjustment => Self::AddPluginAdjustment(parse(call)?),
        })
    }

    pub fn kind(&self) -> OperationKind {
        match self {
            Self::Select(_) => OperationKind::Select,
            Self::SetVisible(_) => OperationKind::SetVisible,
            Self::SetLocked(_) => OperationKind::SetLocked,
            Self::SetOpacity(_) => OperationKind::SetOpacity,
            Self::SetBlendMode(_) => OperationKind::SetBlendMode,
            Self::Rename(_) => OperationKind::Rename,
            Self::SetCollapsed(_) => OperationKind::SetCollapsed,
            Self::Move(_) => OperationKind::Move,
            Self::Delete(_) => OperationKind::Delete,
            Self::Duplicate(_) => OperationKind::Duplicate,
            Self::Group(_) => OperationKind::Group,
            Self::Ungroup(_) => OperationKind::Ungroup,
            Self::ResetTransform(_) => OperationKind::ResetTransform,
            Self::AddGroup(_) => OperationKind::AddGroup,
            Self::AddAdjustment(_) => OperationKind::AddAdjustment,
            Self::AddPixel(_) => OperationKind::AddPixel,
            Self::ApplyEdit(_) => OperationKind::ApplyEdit,
            Self::MaskFromSelection(_) => OperationKind::MaskFromSelection,
            Self::MergeDown(_) => OperationKind::MergeDown,
            Self::Flatten => OperationKind::Flatten,
            Self::ApplyPluginFilter(_) => OperationKind::ApplyPluginFilter,
            Self::AddPluginAdjustment(_) => OperationKind::AddPluginAdjustment,
        }
    }

    /// Every selector the operation names, so the engine can apply the planner's
    /// restriction to all of them and not only to the first.
    pub fn selectors(&self) -> Vec<&LayerSelector> {
        match self {
            Self::Select(p)
            | Self::Delete(p)
            | Self::Duplicate(p)
            | Self::Ungroup(p)
            | Self::ResetTransform(p)
            | Self::MaskFromSelection(p)
            | Self::MergeDown(p) => vec![&p.selector],
            Self::SetVisible(p) => vec![&p.selector],
            Self::SetLocked(p) => vec![&p.selector],
            Self::SetOpacity(p) => vec![&p.selector],
            Self::SetBlendMode(p) => vec![&p.selector],
            Self::Rename(p) => vec![&p.selector],
            Self::SetCollapsed(p) => vec![&p.selector],
            Self::Move(p) => std::iter::once(&p.selector)
                .chain(p.parent.as_ref())
                .collect(),
            Self::Group(p) => p.selectors.iter().collect(),
            Self::ApplyEdit(p) => vec![&p.selector],
            Self::ApplyPluginFilter(p) => vec![&p.selector],
            Self::AddGroup(_)
            | Self::AddAdjustment(_)
            | Self::AddPixel(_)
            | Self::AddPluginAdjustment(_)
            | Self::Flatten => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyParams {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_kind_has_a_unique_namespaced_id_that_round_trips() {
        let mut seen = std::collections::HashSet::new();
        for kind in OperationKind::ALL {
            let id = kind.id();
            assert!(seen.insert(id), "{id} is registered twice");
            assert_eq!(OperationKind::from_id(id), Some(kind));
            // `core.` is the first-party namespace; a plugin may never be in it,
            // and nothing here is anywhere else.
            assert!(id.starts_with("core."), "{id}");
            let parts: Vec<&str> = id.split('.').collect();
            assert!(
                parts.len() == 3 && parts.iter().all(|part| !part.is_empty()),
                "{id}"
            );
            assert!(id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'.' || byte == b'_'));
            assert_eq!(kind.spec().id, id);
            assert!(kind.spec().version >= 1);
        }
        assert_eq!(OperationKind::ALL.len(), 22);
    }

    #[test]
    fn an_unknown_id_is_not_an_operation() {
        let call = OperationCall {
            op: "core.layer.explode".into(),
            params: json!({}),
        };
        assert!(matches!(
            Operation::parse(&call),
            Err(AppError::UnknownOperation(_))
        ));
        let spoof = OperationCall {
            op: "plugin.evil.set_opacity".into(),
            params: json!({}),
        };
        assert!(matches!(
            Operation::parse(&spoof),
            Err(AppError::UnknownOperation(_))
        ));
    }

    #[test]
    fn parameters_are_strict() {
        let good = OperationCall {
            op: "core.layer.set_opacity".into(),
            params: json!({"selector": {"type": "active"}, "opacity": 0.5}),
        };
        assert!(matches!(
            Operation::parse(&good),
            Ok(Operation::SetOpacity(_))
        ));
        // A surplus parameter is an error, not ignored.
        let surplus = OperationCall {
            op: "core.layer.set_opacity".into(),
            params: json!({"selector": {"type": "active"}, "opacity": 0.5, "extra": 1}),
        };
        assert!(Operation::parse(&surplus).is_err());
        // A missing one is an error.
        let missing = OperationCall {
            op: "core.layer.set_opacity".into(),
            params: json!({"selector": {"type": "active"}}),
        };
        assert!(Operation::parse(&missing).is_err());
        // A parameter of the wrong type is an error.
        let wrong = OperationCall {
            op: "core.layer.set_opacity".into(),
            params: json!({"selector": {"type": "active"}, "opacity": "half"}),
        };
        assert!(Operation::parse(&wrong).is_err());
    }

    #[test]
    fn an_operation_with_no_parameters_may_omit_them_but_not_invent_them() {
        let bare = OperationCall {
            op: "core.document.flatten".into(),
            params: serde_json::Value::Null,
        };
        assert!(matches!(Operation::parse(&bare), Ok(Operation::Flatten)));
        let empty = OperationCall {
            op: "core.document.flatten".into(),
            params: json!({}),
        };
        assert!(Operation::parse(&empty).is_ok());
        let invented = OperationCall {
            op: "core.document.flatten".into(),
            params: json!({"keep": "all"}),
        };
        assert!(Operation::parse(&invented).is_err());
    }

    #[test]
    fn destructive_operations_are_never_planner_safe() {
        for kind in [
            OperationKind::Delete,
            OperationKind::MergeDown,
            OperationKind::Flatten,
            OperationKind::ApplyPluginFilter,
            OperationKind::AddPluginAdjustment,
            OperationKind::SetLocked,
        ] {
            assert!(!kind.spec().planner_safe, "{}", kind.id());
        }
        // The ones the layer workflow already let a planner propose stay allowed.
        for kind in [
            OperationKind::Select,
            OperationKind::SetOpacity,
            OperationKind::SetBlendMode,
            OperationKind::AddAdjustment,
            OperationKind::ApplyEdit,
            OperationKind::MaskFromSelection,
        ] {
            assert!(kind.spec().planner_safe, "{}", kind.id());
        }
    }

    #[test]
    fn specs_serialise_for_the_interface() {
        let specs = operation_specs();
        let json = serde_json::to_value(&specs).unwrap();
        assert_eq!(json.as_array().unwrap().len(), 22);
        let opacity = json
            .as_array()
            .unwrap()
            .iter()
            .find(|spec| spec["id"] == "core.layer.set_opacity")
            .unwrap();
        assert_eq!(opacity["params"][1]["kind"]["kind"], "number");
        assert_eq!(opacity["lock"], "automationOnly");
    }
}
