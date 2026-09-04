use super::blend::BlendMode;
use super::model::{LayerDocument, LayerKind};
use crate::domain::EditOperation;
use crate::error::AppError;
use serde::{Deserialize, Serialize};

/// Workflow schema version that understands layer steps. Version 1 files carry
/// no layer steps and are still accepted unchanged.
pub const LAYER_WORKFLOW_SCHEMA_VERSION: u32 = 2;
/// Largest number of layer steps one workflow may carry.
pub const MAX_LAYER_WORKFLOW_STEPS: usize = 100;

// Serde's internally tagged unit visitor accepts surplus fields even when the
// enum denies them. Route fieldless variants through a strict empty struct.
fn deserialize_empty_variant<'de, D>(deserializer: D) -> Result<(), D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    Empty::deserialize(deserializer).map(|_| ())
}

/// How a workflow step names the layer it acts on.
///
/// Selectors are deterministic by construction: an identifier is exact, `Active`
/// follows the document's own selection, `LastCreated` refers to the layer the
/// preceding step made, and `Name` resolves only when exactly one layer carries
/// that name. Anything ambiguous or missing fails the replay rather than
/// silently retargeting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayerSelector {
    Id {
        id: String,
    },
    #[serde(deserialize_with = "deserialize_empty_variant")]
    Active,
    #[serde(deserialize_with = "deserialize_empty_variant")]
    LastCreated,
    Name {
        name: String,
    },
    #[serde(deserialize_with = "deserialize_empty_variant")]
    Bottom,
    #[serde(deserialize_with = "deserialize_empty_variant")]
    Top,
}

impl LayerSelector {
    /// Whether a planner is allowed to emit this selector.
    ///
    /// A planner never sees the document's layer identifiers, so allowing an
    /// `Id` selector from planner output would let it invent one. Restricting
    /// planners to the relative selectors makes fabrication impossible rather
    /// than merely discouraged.
    pub const fn is_planner_safe(&self) -> bool {
        matches!(self, Self::Active | Self::LastCreated)
    }

    fn validate(&self) -> Result<(), AppError> {
        match self {
            Self::Id { id } => {
                if id.is_empty() || id.len() > 64 {
                    return Err(AppError::WorkflowValidation(
                        "layer selector identifiers must contain 1 to 64 characters".into(),
                    ));
                }
                Ok(())
            }
            Self::Name { name } => {
                if name.trim().is_empty() || name.len() > 120 {
                    return Err(AppError::WorkflowValidation(
                        "layer selector names must contain 1 to 120 characters".into(),
                    ));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// One layer-aware workflow step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayerWorkflowStep {
    SelectLayer {
        selector: LayerSelector,
    },
    SetVisibility {
        selector: LayerSelector,
        visible: bool,
    },
    SetOpacity {
        selector: LayerSelector,
        opacity: f32,
    },
    SetBlendMode {
        selector: LayerSelector,
        #[serde(rename = "blendMode")]
        blend_mode: BlendMode,
    },
    CreateAdjustmentLayer {
        operation: Box<EditOperation>,
        #[serde(default)]
        name: Option<String>,
    },
    ApplyToLayer {
        selector: LayerSelector,
        operations: Vec<EditOperation>,
    },
    CreateMaskFromSelection {
        selector: LayerSelector,
    },
    MergeDown {
        selector: LayerSelector,
    },
    #[serde(deserialize_with = "deserialize_empty_variant")]
    Flatten,
    #[serde(deserialize_with = "deserialize_empty_variant")]
    ExportComposite,
}

impl LayerWorkflowStep {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::SelectLayer { .. } => "select_layer",
            Self::SetVisibility { .. } => "set_visibility",
            Self::SetOpacity { .. } => "set_opacity",
            Self::SetBlendMode { .. } => "set_blend_mode",
            Self::CreateAdjustmentLayer { .. } => "create_adjustment_layer",
            Self::ApplyToLayer { .. } => "apply_to_layer",
            Self::CreateMaskFromSelection { .. } => "create_mask_from_selection",
            Self::MergeDown { .. } => "merge_down",
            Self::Flatten => "flatten",
            Self::ExportComposite => "export_composite",
        }
    }

    fn selector(&self) -> Option<&LayerSelector> {
        match self {
            Self::SelectLayer { selector }
            | Self::SetVisibility { selector, .. }
            | Self::SetOpacity { selector, .. }
            | Self::SetBlendMode { selector, .. }
            | Self::ApplyToLayer { selector, .. }
            | Self::CreateMaskFromSelection { selector }
            | Self::MergeDown { selector } => Some(selector),
            _ => None,
        }
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if let Some(selector) = self.selector() {
            selector.validate()?;
        }
        match self {
            Self::SetOpacity { opacity, .. } => {
                if !opacity.is_finite() || !(0.0..=1.0).contains(opacity) {
                    return Err(AppError::WorkflowValidation(
                        "layer opacity must be a finite value between 0 and 1".into(),
                    ));
                }
            }
            Self::CreateAdjustmentLayer { operation, name } => {
                if !operation.supports_adjustment_layer() {
                    return Err(AppError::UnsupportedAdjustmentLayer(
                        operation.kind().to_string(),
                    ));
                }
                operation.validate()?;
                if name
                    .as_ref()
                    .is_some_and(|value| value.trim().is_empty() || value.len() > 120)
                {
                    return Err(AppError::WorkflowValidation(
                        "layer names must contain 1 to 120 characters".into(),
                    ));
                }
            }
            Self::ApplyToLayer { operations, .. } => {
                if operations.is_empty() {
                    return Err(AppError::WorkflowValidation(
                        "apply_to_layer requires at least one operation".into(),
                    ));
                }
                for operation in operations {
                    // Geometry changes the canvas, so it belongs to the document
                    // pipeline rather than to a single layer.
                    if !operation.supports_adjustment_layer() {
                        return Err(AppError::WorkflowValidation(format!(
                            "{} cannot be applied to a single layer",
                            operation.kind()
                        )));
                    }
                    operation.validate()?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Whether a planner may emit this step at all.
    pub fn is_planner_safe(&self) -> bool {
        if matches!(
            self,
            Self::Flatten | Self::MergeDown { .. } | Self::ExportComposite
        ) {
            // Destructive and output steps are the user's decision, never a
            // suggestion the planner can make on its own.
            return false;
        }
        self.selector().is_none_or(LayerSelector::is_planner_safe)
    }
}

pub fn validate_layer_steps(steps: &[LayerWorkflowStep]) -> Result<(), AppError> {
    if steps.len() > MAX_LAYER_WORKFLOW_STEPS {
        return Err(AppError::WorkflowValidation(format!(
            "a workflow may contain at most {MAX_LAYER_WORKFLOW_STEPS} layer steps"
        )));
    }
    let mut created = false;
    for step in steps {
        step.validate()?;
        if let Some(LayerSelector::LastCreated) = step.selector() {
            if !created {
                return Err(AppError::WorkflowValidation(
                    "a step referenced the last created layer before any step created one".into(),
                ));
            }
        }
        if matches!(step, LayerWorkflowStep::CreateAdjustmentLayer { .. }) {
            created = true;
        }
    }
    Ok(())
}

/// Validates planner-proposed steps, rejecting anything that could name a layer
/// the planner cannot know about.
pub fn validate_planner_layer_steps(steps: &[LayerWorkflowStep]) -> Result<(), AppError> {
    validate_layer_steps(steps)?;
    for step in steps {
        if !step.is_planner_safe() {
            return Err(AppError::InvalidPlan(format!(
                "a planner may not propose the {} step",
                step.kind()
            )));
        }
    }
    Ok(())
}

/// Resolves a selector against a document, failing closed.
pub fn resolve(
    document: &LayerDocument,
    selector: &LayerSelector,
    last_created: Option<&str>,
) -> Result<String, AppError> {
    let found = match selector {
        LayerSelector::Id { id } => document.find(id).map(|layer| layer.id.clone()),
        LayerSelector::Active => document
            .active_layer_id
            .as_ref()
            .and_then(|id| document.find(id))
            .map(|layer| layer.id.clone()),
        LayerSelector::LastCreated => last_created
            .and_then(|id| document.find(id))
            .map(|layer| layer.id.clone()),
        LayerSelector::Bottom => document.layers.first().map(|layer| layer.id.clone()),
        LayerSelector::Top => document.layers.last().map(|layer| layer.id.clone()),
        LayerSelector::Name { name } => {
            let matches: Vec<&str> = document
                .iter()
                .filter(|layer| layer.name == *name)
                .map(|layer| layer.id.as_str())
                .collect();
            if matches.len() > 1 {
                return Err(AppError::WorkflowValidation(format!(
                    "{} layers are named {name}; a workflow selector must be unambiguous",
                    matches.len()
                )));
            }
            matches.first().map(|id| (*id).to_string())
        }
    };

    found.ok_or_else(|| {
        AppError::WorkflowValidation(format!(
            "this workflow needs a layer that this document does not have ({})",
            describe(selector)
        ))
    })
}

fn describe(selector: &LayerSelector) -> String {
    match selector {
        LayerSelector::Id { id } => format!("layer {id}"),
        LayerSelector::Active => "the selected layer".into(),
        LayerSelector::LastCreated => "the layer created by an earlier step".into(),
        LayerSelector::Name { name } => format!("a layer named {name}"),
        LayerSelector::Bottom => "the bottom layer".into(),
        LayerSelector::Top => "the top layer".into(),
    }
}

/// Checks that every selector in a workflow resolves against this document
/// before any of it runs, so a replay never half-applies.
pub fn plan_against(
    document: &LayerDocument,
    steps: &[LayerWorkflowStep],
) -> Result<Vec<String>, AppError> {
    validate_layer_steps(steps)?;
    let mut resolved = Vec::new();
    // A create step introduces a layer this pass cannot see, so once one has
    // run, later `LastCreated` selectors are accepted without resolution here.
    let mut pending_created = false;
    for step in steps {
        if matches!(step, LayerWorkflowStep::CreateAdjustmentLayer { .. }) {
            pending_created = true;
            resolved.push(String::new());
            continue;
        }
        match step.selector() {
            Some(LayerSelector::LastCreated) if pending_created => resolved.push(String::new()),
            Some(selector) => resolved.push(resolve(document, selector, None)?),
            None => resolved.push(String::new()),
        }
    }
    Ok(resolved)
}

/// Whether a step needs a pixel layer specifically.
pub fn requires_pixel_layer(step: &LayerWorkflowStep) -> bool {
    matches!(
        step,
        LayerWorkflowStep::ApplyToLayer { .. } | LayerWorkflowStep::MergeDown { .. }
    )
}

/// Confirms a resolved layer is the kind a step needs.
pub fn check_kind(
    document: &LayerDocument,
    step: &LayerWorkflowStep,
    layer_id: &str,
) -> Result<(), AppError> {
    if !requires_pixel_layer(step) {
        return Ok(());
    }
    let layer = document
        .find(layer_id)
        .ok_or_else(|| AppError::LayerNotFound(layer_id.to_string()))?;
    if layer.kind() != LayerKind::Pixel {
        return Err(AppError::WorkflowValidation(format!(
            "{} needs a pixel layer, but {} is a {}",
            step.kind(),
            layer.name,
            layer.kind().id()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::model::fixtures::*;

    fn document() -> LayerDocument {
        let mut document = LayerDocument::new(16, 16);
        document.layers = vec![
            pixel_layer("bottom", 16, 16),
            group_layer("group", vec![pixel_layer("child", 8, 8)]),
            pixel_layer("top", 16, 16),
        ];
        document.active_layer_id = Some("top".into());
        document
    }

    #[test]
    fn identifier_selectors_resolve_exactly() {
        let document = document();
        let selector = LayerSelector::Id { id: "child".into() };
        assert_eq!(resolve(&document, &selector, None).unwrap(), "child");
    }

    #[test]
    fn relative_selectors_resolve_against_the_document() {
        let document = document();
        assert_eq!(
            resolve(&document, &LayerSelector::Active, None).unwrap(),
            "top"
        );
        assert_eq!(
            resolve(&document, &LayerSelector::Bottom, None).unwrap(),
            "bottom"
        );
        assert_eq!(
            resolve(&document, &LayerSelector::Top, None).unwrap(),
            "top"
        );
    }

    #[test]
    fn a_missing_layer_fails_closed_rather_than_targeting_another() {
        let document = document();
        let selector = LayerSelector::Id {
            id: "absent".into(),
        };
        let error = resolve(&document, &selector, None).unwrap_err();
        assert!(matches!(error, AppError::WorkflowValidation(_)));
        assert!(error.to_string().contains("does not have"));
    }

    #[test]
    fn an_ambiguous_name_is_rejected_instead_of_picking_one() {
        let mut document = document();
        document.layers[0].name = "Shared".into();
        document.layers[2].name = "Shared".into();
        let selector = LayerSelector::Name {
            name: "Shared".into(),
        };
        assert!(resolve(&document, &selector, None).is_err());
    }

    #[test]
    fn a_unique_name_resolves() {
        let mut document = document();
        document.layers[0].name = "Sky".into();
        let selector = LayerSelector::Name { name: "Sky".into() };
        assert_eq!(resolve(&document, &selector, None).unwrap(), "bottom");
    }

    #[test]
    fn an_empty_document_resolves_nothing() {
        let document = LayerDocument::new(8, 8);
        assert!(resolve(&document, &LayerSelector::Top, None).is_err());
        assert!(resolve(&document, &LayerSelector::Active, None).is_err());
    }

    #[test]
    fn a_stale_active_selection_fails_closed() {
        let mut document = document();
        document.active_layer_id = None;
        assert!(resolve(&document, &LayerSelector::Active, None).is_err());
    }

    #[test]
    fn last_created_resolves_only_after_something_created_a_layer() {
        let steps = vec![LayerWorkflowStep::SetOpacity {
            selector: LayerSelector::LastCreated,
            opacity: 0.5,
        }];
        assert!(validate_layer_steps(&steps).is_err());

        let steps = vec![
            LayerWorkflowStep::CreateAdjustmentLayer {
                operation: Box::new(EditOperation::Grayscale),
                name: None,
            },
            LayerWorkflowStep::SetOpacity {
                selector: LayerSelector::LastCreated,
                opacity: 0.5,
            },
        ];
        validate_layer_steps(&steps).unwrap();
    }

    #[test]
    fn planning_resolves_every_selector_before_anything_runs() {
        let document = document();
        let steps = vec![
            LayerWorkflowStep::SelectLayer {
                selector: LayerSelector::Id { id: "top".into() },
            },
            LayerWorkflowStep::SetOpacity {
                selector: LayerSelector::Bottom,
                opacity: 0.4,
            },
        ];
        assert_eq!(
            plan_against(&document, &steps).unwrap(),
            vec!["top", "bottom"]
        );

        let broken = vec![
            LayerWorkflowStep::SelectLayer {
                selector: LayerSelector::Id { id: "top".into() },
            },
            LayerWorkflowStep::SetOpacity {
                selector: LayerSelector::Id {
                    id: "absent".into(),
                },
                opacity: 0.4,
            },
        ];
        assert!(plan_against(&document, &broken).is_err());
    }

    #[test]
    fn planning_accepts_a_layer_a_previous_step_will_create() {
        let document = document();
        let steps = vec![
            LayerWorkflowStep::CreateAdjustmentLayer {
                operation: Box::new(EditOperation::Contrast { amount: 0.2 }),
                name: Some("Punch".into()),
            },
            LayerWorkflowStep::SetOpacity {
                selector: LayerSelector::LastCreated,
                opacity: 0.6,
            },
        ];
        assert!(plan_against(&document, &steps).is_ok());
    }

    #[test]
    fn opacity_and_names_are_validated() {
        for opacity in [1.5_f32, -0.1, f32::NAN] {
            let step = LayerWorkflowStep::SetOpacity {
                selector: LayerSelector::Active,
                opacity,
            };
            assert!(step.validate().is_err());
        }
        let step = LayerWorkflowStep::CreateAdjustmentLayer {
            operation: Box::new(EditOperation::Grayscale),
            name: Some("   ".into()),
        };
        assert!(step.validate().is_err());
    }

    #[test]
    fn geometry_operations_are_rejected_for_layer_steps() {
        let step = LayerWorkflowStep::CreateAdjustmentLayer {
            operation: Box::new(EditOperation::Rotate { degrees: 90 }),
            name: None,
        };
        assert!(matches!(
            step.validate(),
            Err(AppError::UnsupportedAdjustmentLayer(_))
        ));

        let step = LayerWorkflowStep::ApplyToLayer {
            selector: LayerSelector::Active,
            operations: vec![EditOperation::ReflectHorizontal],
        };
        assert!(matches!(
            step.validate(),
            Err(AppError::WorkflowValidation(_))
        ));
    }

    #[test]
    fn apply_to_layer_needs_at_least_one_operation() {
        let step = LayerWorkflowStep::ApplyToLayer {
            selector: LayerSelector::Active,
            operations: Vec::new(),
        };
        assert!(step.validate().is_err());
    }

    #[test]
    fn steps_beyond_the_ceiling_are_rejected() {
        let steps: Vec<LayerWorkflowStep> = (0..=MAX_LAYER_WORKFLOW_STEPS)
            .map(|_| LayerWorkflowStep::Flatten)
            .collect();
        assert!(validate_layer_steps(&steps).is_err());
    }

    #[test]
    fn a_step_needing_pixels_rejects_a_group_or_adjustment_layer() {
        let document = document();
        let step = LayerWorkflowStep::ApplyToLayer {
            selector: LayerSelector::Id { id: "group".into() },
            operations: vec![EditOperation::Grayscale],
        };
        assert!(check_kind(&document, &step, "group").is_err());
        assert!(check_kind(&document, &step, "top").is_ok());

        let harmless = LayerWorkflowStep::SetVisibility {
            selector: LayerSelector::Id { id: "group".into() },
            visible: false,
        };
        assert!(check_kind(&document, &harmless, "group").is_ok());
    }

    #[test]
    fn planners_may_only_use_selectors_they_cannot_fabricate() {
        assert!(LayerSelector::Active.is_planner_safe());
        assert!(LayerSelector::LastCreated.is_planner_safe());
        assert!(!LayerSelector::Id {
            id: "made-up".into()
        }
        .is_planner_safe());
        assert!(!LayerSelector::Name { name: "Sky".into() }.is_planner_safe());
        assert!(!LayerSelector::Top.is_planner_safe());
    }

    #[test]
    fn planner_output_containing_an_identifier_is_rejected() {
        let steps = vec![LayerWorkflowStep::SetOpacity {
            selector: LayerSelector::Id {
                id: "invented".into(),
            },
            opacity: 0.5,
        }];
        assert!(matches!(
            validate_planner_layer_steps(&steps),
            Err(AppError::InvalidPlan(_))
        ));
    }

    #[test]
    fn planner_output_may_not_flatten_merge_or_export() {
        for step in [
            LayerWorkflowStep::Flatten,
            LayerWorkflowStep::MergeDown {
                selector: LayerSelector::Active,
            },
            LayerWorkflowStep::ExportComposite,
        ] {
            assert!(matches!(
                validate_planner_layer_steps(&[step]),
                Err(AppError::InvalidPlan(_))
            ));
        }
    }

    #[test]
    fn a_planner_may_propose_an_adjustment_layer_with_a_mask_and_opacity() {
        // The example from the phase brief: create a curves adjustment layer,
        // mask it with the current selection, and reduce its opacity.
        let steps = vec![
            LayerWorkflowStep::CreateAdjustmentLayer {
                operation: Box::new(EditOperation::Curves {
                    curves: crate::domain::CurveSet::default(),
                }),
                name: Some("Curves".into()),
            },
            LayerWorkflowStep::CreateMaskFromSelection {
                selector: LayerSelector::LastCreated,
            },
            LayerWorkflowStep::SetOpacity {
                selector: LayerSelector::LastCreated,
                opacity: 0.6,
            },
        ];
        validate_planner_layer_steps(&steps).unwrap();
    }

    #[test]
    fn steps_round_trip_through_tagged_json() {
        let steps = vec![
            LayerWorkflowStep::SelectLayer {
                selector: LayerSelector::Active,
            },
            LayerWorkflowStep::SetBlendMode {
                selector: LayerSelector::Top,
                blend_mode: BlendMode::Multiply,
            },
            LayerWorkflowStep::CreateAdjustmentLayer {
                operation: Box::new(EditOperation::Brightness { amount: 0.2 }),
                name: Some("Lift".into()),
            },
            LayerWorkflowStep::Flatten,
        ];
        let json = serde_json::to_string(&steps).unwrap();
        assert!(json.contains("select_layer"));
        assert!(json.contains("blendMode"));
        assert_eq!(
            serde_json::from_str::<Vec<LayerWorkflowStep>>(&json).unwrap(),
            steps
        );
    }

    #[test]
    fn unknown_step_and_selector_types_are_rejected() {
        assert!(
            serde_json::from_str::<LayerWorkflowStep>(r#"{"type":"delete_everything"}"#).is_err()
        );
        assert!(serde_json::from_str::<LayerSelector>(r#"{"type":"any"}"#).is_err());
    }

    #[test]
    fn selectors_reject_unknown_fields_including_unit_variants() {
        for json in [
            r#"{"type":"id","id":"layer","fallback":"active"}"#,
            r#"{"type":"name","name":"Sky","index":0}"#,
            r#"{"type":"active","id":"different-layer"}"#,
            r#"{"type":"last_created","unknown":true}"#,
            r#"{"type":"top","unknown":true}"#,
            r#"{"type":"bottom","unknown":true}"#,
        ] {
            assert!(
                serde_json::from_str::<LayerSelector>(json).is_err(),
                "accepted {json}"
            );
        }
    }

    #[test]
    fn steps_reject_unknown_fields_before_workflow_validation() {
        for json in [
            r#"{"type":"set_opacity","selector":{"type":"active"},"opacity":0.5,"mask":"ignored"}"#,
            r#"{"type":"select_layer","selector":{"type":"active","id":"ignored"}}"#,
            r#"{"type":"create_adjustment_layer","operation":{"type":"grayscale"},"layerId":"ignored"}"#,
            r#"{"type":"flatten","visibleOnly":false}"#,
            r#"{"type":"export_composite","outputPath":"ignored.png"}"#,
        ] {
            assert!(
                serde_json::from_str::<LayerWorkflowStep>(json).is_err(),
                "accepted {json}"
            );
        }
    }
}
