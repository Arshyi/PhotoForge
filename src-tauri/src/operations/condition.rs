//! Conditions: the only decision an automation can make.
//!
//! A step may carry a condition, and is skipped when it does not hold. That is the
//! whole of it. There is no `else`, no loop, no variable and no expression: a
//! condition is one of a short list of questions about the document *as it is when
//! the step is reached* — after the steps before it have run — and it is answered
//! with yes or no.
//!
//! "As it is when the step is reached" is the point. A condition evaluated once at the
//! start would answer for a document the earlier steps have since changed, so
//! "if there is a layer called Sky, dim it" could dim a layer an earlier step had
//! already deleted. The engine asks the question of the working copy at each step, in
//! the dry run and again in the real one, so both skip the same steps.
use crate::error::AppError;
use crate::layers::workflow::{resolve, LayerSelector};
use crate::layers::LayerDocument;
use serde::{Deserialize, Serialize};

/// How deeply `not` may be nested. Enough to say what is meant, too little to be a
/// program.
pub const MAX_CONDITION_DEPTH: usize = 2;
pub const MAX_LAYER_COUNT: usize = 512;

const LAYER_KINDS: [&str; 6] = [
    "pixel",
    "group",
    "adjustment",
    "shape",
    "text",
    "smart_object",
];
const PRECISIONS: [&str; 2] = ["linear_srgb_f32", "legacy_srgb8"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Condition {
    /// A layer the selector names exists. A selector that names nothing, or names
    /// something ambiguous, is "no".
    LayerExists { selector: LayerSelector },
    /// The document has at least this many layers, groups included.
    LayerCountAtLeast { count: usize },
    /// The selected layer is of this kind (`pixel`, `group`, `adjustment`, `shape`,
    /// `text`, `smart_object`). With nothing selected, "no".
    ActiveLayerKind { layer_kind: String },
    /// The document has this precision (`linear_srgb_f32` or `legacy_srgb8`).
    Precision { precision: String },
    /// The opposite of the condition inside.
    Not { condition: Box<Condition> },
}

fn invalid(reason: impl Into<String>) -> AppError {
    AppError::InvalidOperation(format!("condition: {}", reason.into()))
}

impl Condition {
    pub fn validate(&self) -> Result<(), AppError> {
        self.validate_at(0)
    }

    fn validate_at(&self, depth: usize) -> Result<(), AppError> {
        match self {
            Self::LayerExists { selector } => {
                if let LayerSelector::Id { id } = selector {
                    if id.is_empty() || id.len() > 64 {
                        return Err(invalid("a layer id must contain 1 to 64 characters"));
                    }
                }
                if let LayerSelector::Name { name } = selector {
                    if name.trim().is_empty() || name.len() > 120 {
                        return Err(invalid("a layer name must contain 1 to 120 characters"));
                    }
                }
                Ok(())
            }
            Self::LayerCountAtLeast { count } => {
                if *count == 0 || *count > MAX_LAYER_COUNT {
                    Err(invalid(format!(
                        "a layer count must be 1 to {MAX_LAYER_COUNT}"
                    )))
                } else {
                    Ok(())
                }
            }
            Self::ActiveLayerKind { layer_kind } => {
                if LAYER_KINDS.contains(&layer_kind.as_str()) {
                    Ok(())
                } else {
                    Err(invalid(format!("{layer_kind:?} is not a kind of layer")))
                }
            }
            Self::Precision { precision } => {
                if PRECISIONS.contains(&precision.as_str()) {
                    Ok(())
                } else {
                    Err(invalid(format!(
                        "{precision:?} is not a document precision"
                    )))
                }
            }
            Self::Not { condition } => {
                if depth >= MAX_CONDITION_DEPTH {
                    return Err(invalid(format!(
                        "not may be nested at most {MAX_CONDITION_DEPTH} deep"
                    )));
                }
                condition.validate_at(depth + 1)
            }
        }
    }

    /// Every layer selector the condition names.
    pub fn selectors(&self) -> Vec<&LayerSelector> {
        match self {
            Self::LayerExists { selector } => vec![selector],
            Self::Not { condition } => condition.selectors(),
            _ => Vec::new(),
        }
    }

    /// Whether it holds for `document`, with `last_created` as the layer the last step
    /// that made one made.
    pub fn holds(&self, document: &LayerDocument, last_created: Option<&str>) -> bool {
        match self {
            Self::LayerExists { selector } => resolve(document, selector, last_created).is_ok(),
            Self::LayerCountAtLeast { count } => document.layer_count() >= *count,
            Self::ActiveLayerKind { layer_kind } => document
                .active_layer_id
                .as_deref()
                .and_then(|id| document.find(id))
                .is_some_and(|layer| layer.kind().id() == layer_kind),
            Self::Precision { precision } => {
                let actual = match document.precision {
                    crate::pixel::DocumentPrecision::LinearSrgbF32 => "linear_srgb_f32",
                    crate::pixel::DocumentPrecision::LegacySrgb8 => "legacy_srgb8",
                };
                actual == precision
            }
            Self::Not { condition } => !condition.holds(document, last_created),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::LayerContent;
    use crate::operations::structure::new_layer;

    fn document() -> LayerDocument {
        let mut document = LayerDocument::new(8, 8);
        document.layers = vec![
            new_layer(
                "sky",
                "Sky",
                LayerContent::Pixel {
                    pixel_id: "px1".into(),
                    width: 8,
                    height: 8,
                },
                "t",
            ),
            new_layer(
                "notes",
                "Notes",
                LayerContent::Group {
                    children: vec![],
                    isolated: true,
                },
                "t",
            ),
        ];
        document.active_layer_id = Some("sky".into());
        document
    }

    fn exists(name: &str) -> Condition {
        Condition::LayerExists {
            selector: LayerSelector::Name { name: name.into() },
        }
    }

    #[test]
    fn each_condition_answers_about_the_document() {
        let doc = document();
        assert!(exists("Sky").holds(&doc, None));
        assert!(!exists("Water").holds(&doc, None));
        assert!(Condition::LayerCountAtLeast { count: 2 }.holds(&doc, None));
        assert!(!Condition::LayerCountAtLeast { count: 3 }.holds(&doc, None));
        let kind = |value: &str| Condition::ActiveLayerKind {
            layer_kind: value.into(),
        };
        assert!(kind("pixel").holds(&doc, None));
        assert!(!kind("group").holds(&doc, None));
        let precision = |value: &str| Condition::Precision {
            precision: value.into(),
        };
        assert!(precision("legacy_srgb8").holds(&doc, None));
        assert!(!precision("linear_srgb_f32").holds(&doc, None));
        assert!(Condition::Not {
            condition: Box::new(exists("Water"))
        }
        .holds(&doc, None));
        // With nothing selected the kind question is "no".
        let mut none = document();
        none.active_layer_id = None;
        assert!(!kind("pixel").holds(&none, None));
    }

    #[test]
    fn an_ambiguous_or_missing_selector_is_a_no() {
        let mut doc = document();
        doc.layers[1].name = "Sky".into();
        assert!(
            !exists("Sky").holds(&doc, None),
            "two layers are called Sky"
        );
        let last = Condition::LayerExists {
            selector: LayerSelector::LastCreated,
        };
        assert!(!last.holds(&doc, None));
        assert!(last.holds(&doc, Some("notes")));
        assert!(!last.holds(&doc, Some("gone")));
    }

    #[test]
    fn conditions_are_small_closed_and_bounded() {
        for bad in [
            Condition::LayerCountAtLeast { count: 0 },
            Condition::LayerCountAtLeast { count: 513 },
            Condition::ActiveLayerKind {
                layer_kind: "photo".into(),
            },
            Condition::Precision {
                precision: "float64".into(),
            },
            Condition::LayerExists {
                selector: LayerSelector::Id { id: String::new() },
            },
            Condition::LayerExists {
                selector: LayerSelector::Name {
                    name: "x".repeat(121),
                },
            },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        let not = |inner: Condition| Condition::Not {
            condition: Box::new(inner),
        };
        assert!(not(not(exists("a"))).validate().is_ok());
        assert!(
            not(not(not(exists("a")))).validate().is_err(),
            "three deep is a program"
        );
    }

    #[test]
    fn the_wire_form_is_strict() {
        let parsed: Condition =
            serde_json::from_str(r#"{"kind":"layer_count_at_least","count":3}"#).unwrap();
        assert_eq!(parsed, Condition::LayerCountAtLeast { count: 3 });
        let parsed: Condition = serde_json::from_str(
            r#"{"kind":"not","condition":{"kind":"active_layer_kind","layerKind":"text"}}"#,
        )
        .unwrap();
        assert!(matches!(parsed, Condition::Not { .. }));
        for bad in [
            r#"{"kind":"layer_count_at_least","count":3,"extra":1}"#,
            r#"{"kind":"eval","code":"1+1"}"#,
            r#"{"kind":"layer_count_at_least"}"#,
        ] {
            assert!(serde_json::from_str::<Condition>(bad).is_err(), "{bad}");
        }
    }
}
