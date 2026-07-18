use super::{EditOperation, EditPlan, ImageQualityAnalysis};
use crate::error::AppError;
use std::collections::HashSet;

pub const MAX_PLAN_OPERATIONS: usize = 8;
const MAX_REQUEST_CHARS: usize = 1_000;
const MAX_WARNINGS: usize = 8;

pub trait EditPlanner {
    fn plan(&self, request: &str, analysis: &ImageQualityAnalysis) -> Result<EditPlan, AppError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RuleBasedPlanner;

impl EditPlanner for RuleBasedPlanner {
    fn plan(&self, request: &str, analysis: &ImageQualityAnalysis) -> Result<EditPlan, AppError> {
        let normalized = normalize_request(request)?;
        let wants_lighter = has_any(
            &normalized,
            &[
                "too dark",
                "underexposed",
                "brighten",
                "brighter",
                "lighter",
                "increase brightness",
                "lift shadows",
            ],
        );
        let wants_darker = has_any(
            &normalized,
            &[
                "too bright",
                "overexposed",
                "darken",
                "darker",
                "dim this",
                "reduce brightness",
            ],
        );
        if wants_lighter && wants_darker {
            return Err(AppError::InvalidPlan(
                "the request asks for conflicting brightness directions".into(),
            ));
        }

        let mut operations = Vec::new();
        let mut warnings = Vec::new();
        let mut matches = 0_u32;
        let preserve_colors = has_any(
            &normalized,
            &["without changing colors", "keep colors", "preserve colors"],
        );

        if preserve_colors
            && has_any(
                &normalized,
                &[
                    "improve without changing colors",
                    "improve while preserving colors",
                ],
            )
        {
            matches += 1;
            push_unique(
                &mut operations,
                EditOperation::LocalContrast {
                    strength: 0.22,
                    tile_size: 40,
                    clip_limit: 1.15,
                },
            );
        }

        if wants_lighter {
            matches += 1;
            if analysis.average_luminance < 0.72 {
                push_unique(
                    &mut operations,
                    EditOperation::Brightness {
                        amount: if analysis.average_luminance < 0.30 {
                            0.14
                        } else {
                            0.08
                        },
                    },
                );
            } else {
                warnings.push(
                    "The image already appears bright, so the plan does not increase global brightness."
                        .into(),
                );
                push_unique(
                    &mut operations,
                    EditOperation::LocalContrast {
                        strength: 0.24,
                        tile_size: 32,
                        clip_limit: 1.2,
                    },
                );
            }
        }

        if wants_darker {
            matches += 1;
            if analysis.average_luminance > 0.25 {
                push_unique(
                    &mut operations,
                    EditOperation::Brightness {
                        amount: if analysis.average_luminance > 0.78 {
                            -0.14
                        } else {
                            -0.08
                        },
                    },
                );
            } else {
                warnings.push(
                    "The image already appears dark, so the plan does not reduce global brightness."
                        .into(),
                );
                push_unique(
                    &mut operations,
                    EditOperation::LocalContrast {
                        strength: 0.18,
                        tile_size: 32,
                        clip_limit: 1.1,
                    },
                );
            }
        }

        let color_request = has_any(
            &normalized,
            &[
                "yellow",
                "warm cast",
                "too warm",
                "blue cast",
                "too blue",
                "too cool",
                "indoor lighting",
                "white balance",
                "colors more natural",
                "natural colors",
            ],
        );
        if color_request {
            matches += 1;
            if preserve_colors {
                warnings.push(
                    "Color correction was omitted because the request asks to preserve colors."
                        .into(),
                );
            } else {
                push_unique(
                    &mut operations,
                    EditOperation::AutoWhiteBalance {
                        strength: if analysis.estimated_color_cast.dominant == "neutral" {
                            0.38
                        } else {
                            0.66
                        },
                    },
                );
            }
        }

        let weak_contrast = has_any(
            &normalized,
            &[
                "washed out",
                "flat",
                "faded",
                "weak contrast",
                "bring out",
                "more readable",
            ],
        );
        if weak_contrast {
            matches += 1;
            push_unique(
                &mut operations,
                EditOperation::LocalContrast {
                    strength: if analysis.estimated_local_contrast < 0.04 {
                        0.46
                    } else {
                        0.32
                    },
                    tile_size: 32,
                    clip_limit: 1.35,
                },
            );
        }

        if has_any(
            &normalized,
            &["too much contrast", "harsh contrast", "reduce contrast"],
        ) {
            matches += 1;
            push_unique(&mut operations, EditOperation::Contrast { amount: -0.16 });
        }

        if has_any(
            &normalized,
            &[
                "grainy",
                "noisy",
                "reduce noise",
                "remove noise",
                "sensor noise",
            ],
        ) {
            matches += 1;
            push_unique(
                &mut operations,
                EditOperation::Denoise {
                    strength: if analysis.estimated_noise > 0.20 {
                        0.52
                    } else {
                        0.34
                    },
                    preserve_edges: 0.84,
                },
            );
        }

        if has_any(
            &normalized,
            &[
                "compression",
                "jpeg",
                "blocky",
                "blocking artifacts",
                "jpeg artifacts",
            ],
        ) {
            matches += 1;
            push_unique(&mut operations, EditOperation::Deblock { strength: 0.56 });
            push_unique(
                &mut operations,
                EditOperation::Denoise {
                    strength: 0.16,
                    preserve_edges: 0.90,
                },
            );
            warnings.push(
                "JPEG cleanup can reduce visible blocks but cannot restore discarded data.".into(),
            );
        }

        if has_any(
            &normalized,
            &["blurry", "blurred", "soft", "slight blur", "camera shake"],
        ) {
            matches += 1;
            if analysis.estimated_noise > 0.18 {
                push_unique(
                    &mut operations,
                    EditOperation::Denoise {
                        strength: 0.22,
                        preserve_edges: 0.88,
                    },
                );
            }
            push_unique(
                &mut operations,
                EditOperation::MildDeblur {
                    strength: 0.36,
                    radius: 1.2,
                },
            );
            warnings.push(
                "Mild Deblur improves captured edge contrast; it cannot recover missing information."
                    .into(),
            );
        }

        if has_any(
            &normalized,
            &["sharpen", "more detail", "crisper", "clearer edges"],
        ) {
            matches += 1;
            push_unique(
                &mut operations,
                EditOperation::EdgeAwareSharpen {
                    strength: 0.30,
                    radius: 1.0,
                    threshold: if analysis.estimated_noise > 0.15 {
                        0.055
                    } else {
                        0.035
                    },
                },
            );
            if analysis.estimated_noise > 0.15 {
                warnings.push(
                    "Sharpening is thresholded because the analysis estimates visible noise."
                        .into(),
                );
            }
        }

        if has_any(
            &normalized,
            &[
                "uneven lighting",
                "uneven illumination",
                "page shadow",
                "lighting gradient",
                "shadow across",
            ],
        ) {
            matches += 1;
            push_unique(
                &mut operations,
                EditOperation::UnevenLightingCorrection {
                    strength: 0.64,
                    radius: 44.0,
                },
            );
        }

        let document_request = has_any(
            &normalized,
            &[
                "document",
                "receipt",
                "scan",
                "reading",
                "writing",
                "handwriting",
                "notes",
                "worksheet",
                "whiteboard",
                "text",
            ],
        );
        if document_request {
            matches += 1;
            let grayscale = has_any(&normalized, &["grayscale", "black and white", "monochrome"]);
            push_unique(
                &mut operations,
                EditOperation::DocumentEnhance {
                    strength: if analysis.likely_document { 0.72 } else { 0.62 },
                    grayscale,
                },
            );
            warnings.push(
                "Document enhancement increases local contrast and may reduce photographic realism."
                    .into(),
            );
        }

        if has_any(
            &normalized,
            &["old photo", "old scan", "aged photo", "restore photo"],
        ) {
            matches += 1;
            if !preserve_colors {
                push_unique(
                    &mut operations,
                    EditOperation::AutoWhiteBalance { strength: 0.42 },
                );
            }
            push_unique(
                &mut operations,
                EditOperation::LocalContrast {
                    strength: 0.30,
                    tile_size: 40,
                    clip_limit: 1.25,
                },
            );
            push_unique(
                &mut operations,
                EditOperation::Denoise {
                    strength: 0.24,
                    preserve_edges: 0.86,
                },
            );
            push_unique(
                &mut operations,
                EditOperation::EdgeAwareSharpen {
                    strength: 0.26,
                    radius: 1.1,
                    threshold: 0.045,
                },
            );
        }

        if preserve_colors && operations.is_empty() {
            matches += 1;
            push_unique(
                &mut operations,
                EditOperation::LocalContrast {
                    strength: 0.24,
                    tile_size: 32,
                    clip_limit: 1.2,
                },
            );
            if analysis.estimated_noise > 0.12 {
                push_unique(
                    &mut operations,
                    EditOperation::Denoise {
                        strength: 0.20,
                        preserve_edges: 0.88,
                    },
                );
            }
        }

        if operations.is_empty() {
            return Err(AppError::PlannerNoMatch);
        }

        operations.sort_by_key(operation_stage);
        if operations.len() > MAX_PLAN_OPERATIONS {
            operations.truncate(MAX_PLAN_OPERATIONS);
            warnings.push(
                "The request matched more edits than the safe plan limit; lower-priority steps were omitted."
                    .into(),
            );
        }
        warnings.truncate(MAX_WARNINGS);
        let operation_explanations = operations
            .iter()
            .map(|operation| operation_explanation(operation).to_string())
            .collect();
        let plan = EditPlan {
            summary: plan_summary(&operations, document_request),
            confidence: (0.52 + matches.min(4) as f32 * 0.09).min(0.92),
            warnings,
            operations,
            operation_explanations,
        };
        validate_edit_plan(&plan)?;
        Ok(plan)
    }
}

pub fn validate_edit_plan(plan: &EditPlan) -> Result<(), AppError> {
    let summary = plan.summary.trim();
    if summary.is_empty() || summary.chars().count() > 240 {
        return Err(AppError::InvalidPlan(
            "summary must contain between 1 and 240 characters".into(),
        ));
    }
    if !plan.confidence.is_finite() || !(0.0..=1.0).contains(&plan.confidence) {
        return Err(AppError::InvalidPlan(
            "confidence must be a finite value between 0 and 1".into(),
        ));
    }
    if plan.operations.is_empty() {
        return Err(AppError::InvalidPlan(
            "the plan must contain at least one operation".into(),
        ));
    }
    if plan.operations.len() > MAX_PLAN_OPERATIONS {
        return Err(AppError::InvalidPlan(format!(
            "a guided plan may contain at most {MAX_PLAN_OPERATIONS} operations"
        )));
    }
    if plan.operation_explanations.len() != plan.operations.len() {
        return Err(AppError::InvalidPlan(
            "every operation must have one explanation".into(),
        ));
    }
    if plan.warnings.len() > MAX_WARNINGS
        || plan
            .warnings
            .iter()
            .any(|warning| warning.trim().is_empty() || warning.chars().count() > 240)
    {
        return Err(AppError::InvalidPlan(
            "warnings must be non-empty, concise, and within the supported count".into(),
        ));
    }
    if plan
        .operation_explanations
        .iter()
        .any(|explanation| explanation.trim().is_empty() || explanation.chars().count() > 240)
    {
        return Err(AppError::InvalidPlan(
            "operation explanations must be non-empty and concise".into(),
        ));
    }

    let mut seen = HashSet::new();
    let mut previous_stage = 0_u8;
    let mut has_grayscale = false;
    let mut has_saturation = false;
    let mut grayscale_document = false;
    for operation in &plan.operations {
        operation.validate()?;
        let key = operation_key(operation).ok_or_else(|| {
            AppError::InvalidPlan(
                "the planner may only use supported deterministic editing operations".into(),
            )
        })?;
        if !seen.insert(key) {
            return Err(AppError::InvalidPlan(format!(
                "duplicate or conflicting {key} operations are not supported"
            )));
        }
        let stage = operation_stage(operation);
        if stage < previous_stage {
            return Err(AppError::InvalidPlan(
                "operations are in an unsupported order; move color/lighting cleanup before detail and final color conversion"
                    .into(),
            ));
        }
        previous_stage = stage;
        has_grayscale |= matches!(operation, EditOperation::Grayscale);
        has_saturation |= matches!(operation, EditOperation::Saturation { amount } if amount.abs() > f32::EPSILON);
        grayscale_document |= matches!(
            operation,
            EditOperation::DocumentEnhance {
                grayscale: true,
                ..
            }
        );
    }
    if has_saturation && (has_grayscale || grayscale_document) {
        return Err(AppError::InvalidPlan(
            "saturation conflicts with a grayscale result".into(),
        ));
    }
    Ok(())
}

fn normalize_request(request: &str) -> Result<String, AppError> {
    let trimmed = request.trim();
    if trimmed.is_empty() {
        return Err(AppError::InvalidPlan(
            "enter a guided editing request".into(),
        ));
    }
    if trimmed.chars().count() > MAX_REQUEST_CHARS {
        return Err(AppError::InvalidPlan(format!(
            "requests may contain at most {MAX_REQUEST_CHARS} characters"
        )));
    }
    let normalized = trimmed
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || character.is_whitespace() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    Ok(normalized)
}

fn has_any(request: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| request.contains(phrase))
}

fn push_unique(operations: &mut Vec<EditOperation>, operation: EditOperation) {
    let key = operation_key(&operation);
    if !operations
        .iter()
        .any(|candidate| operation_key(candidate) == key)
    {
        operations.push(operation);
    }
}

fn operation_key(operation: &EditOperation) -> Option<&'static str> {
    match operation {
        EditOperation::Brightness { .. } => Some("brightness"),
        EditOperation::Contrast { .. } => Some("contrast"),
        EditOperation::Saturation { .. } => Some("saturation"),
        EditOperation::Grayscale => Some("grayscale"),
        EditOperation::AutoWhiteBalance { .. } => Some("auto white balance"),
        EditOperation::LocalContrast { .. } => Some("local contrast"),
        EditOperation::Denoise { .. } => Some("denoise"),
        EditOperation::Deblock { .. } => Some("JPEG cleanup"),
        EditOperation::EdgeAwareSharpen { .. } => Some("edge-aware sharpen"),
        EditOperation::MildDeblur { .. } => Some("mild deblur"),
        EditOperation::DocumentEnhance { .. } => Some("document enhance"),
        EditOperation::UnevenLightingCorrection { .. } => Some("uneven lighting"),
        _ => None,
    }
}

fn operation_stage(operation: &EditOperation) -> u8 {
    match operation {
        EditOperation::AutoWhiteBalance { .. } => 10,
        EditOperation::UnevenLightingCorrection { .. } => 20,
        EditOperation::DocumentEnhance { .. } => 25,
        EditOperation::Brightness { .. } => 30,
        EditOperation::Contrast { .. } => 35,
        EditOperation::LocalContrast { .. } => 40,
        EditOperation::Deblock { .. } => 50,
        EditOperation::Denoise { .. } => 55,
        EditOperation::MildDeblur { .. } => 60,
        EditOperation::EdgeAwareSharpen { .. } => 65,
        EditOperation::Saturation { .. } => 70,
        EditOperation::Grayscale => 75,
        _ => u8::MAX,
    }
}

fn operation_explanation(operation: &EditOperation) -> &'static str {
    match operation {
        EditOperation::Brightness { amount } if *amount < 0.0 => {
            "Reduces global luminance by a bounded amount."
        }
        EditOperation::Brightness { .. } => "Raises global luminance by a bounded amount.",
        EditOperation::Contrast { amount } if *amount < 0.0 => {
            "Softens the global difference between light and dark tones."
        }
        EditOperation::Contrast { .. } => {
            "Increases the global difference between light and dark tones."
        }
        EditOperation::Saturation { .. } => {
            "Adjusts color intensity without generating new colors."
        }
        EditOperation::Grayscale => "Converts captured colors to deterministic luminance.",
        EditOperation::AutoWhiteBalance { .. } => {
            "Uses trimmed channel statistics to reduce a broad color cast."
        }
        EditOperation::LocalContrast { .. } => {
            "Improves nearby brightness differences without globally increasing contrast."
        }
        EditOperation::Denoise { .. } => {
            "Reduces small pixel variation while weighting major edges more strongly."
        }
        EditOperation::Deblock { .. } => {
            "Softens selected 8×8 boundary discontinuities conservatively."
        }
        EditOperation::EdgeAwareSharpen { .. } => {
            "Raises contrast at captured edges while thresholding flatter regions."
        }
        EditOperation::MildDeblur { .. } => {
            "Applies bounded clarity restoration for slight softness; it does not recreate detail."
        }
        EditOperation::DocumentEnhance { .. } => {
            "Runs a fixed, documented sequence for page lighting, contrast, noise, and text edges."
        }
        EditOperation::UnevenLightingCorrection { .. } => {
            "Normalizes broad illumination variation using a low-frequency luminance estimate."
        }
        _ => "Uses an existing deterministic PhotoForge operation.",
    }
}

fn plan_summary(operations: &[EditOperation], document_request: bool) -> String {
    if document_request {
        return "Improve readability using a reviewable deterministic document plan.".into();
    }
    let labels = operations
        .iter()
        .filter_map(operation_key)
        .collect::<Vec<_>>();
    format!(
        "Apply a conservative deterministic plan using {}.",
        labels.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ColorCastEstimate;

    fn analysis() -> ImageQualityAnalysis {
        ImageQualityAnalysis {
            average_luminance: 0.48,
            luminance_spread: 0.52,
            estimated_color_cast: ColorCastEstimate {
                dominant: "neutral".into(),
                red_bias: 0.0,
                green_bias: 0.0,
                blue_bias: 0.0,
            },
            estimated_noise: 0.08,
            estimated_sharpness: 0.10,
            estimated_local_contrast: 0.08,
            edge_density: 0.12,
            white_background_ratio: 0.10,
            likely_document: false,
        }
    }

    fn plan(request: &str) -> EditPlan {
        RuleBasedPlanner.plan(request, &analysis()).unwrap()
    }

    fn has_type(plan: &EditPlan, expected: &str) -> bool {
        plan.operations
            .iter()
            .any(|operation| operation_key(operation) == Some(expected))
    }

    macro_rules! phrase_test {
        ($name:ident, $request:literal, $expected:literal) => {
            #[test]
            fn $name() {
                assert!(has_type(&plan($request), $expected));
            }
        };
    }

    phrase_test!(recognizes_too_dark, "This is too dark", "brightness");
    phrase_test!(
        recognizes_underexposed,
        "Fix an underexposed photo",
        "brightness"
    );
    phrase_test!(recognizes_brighten, "Brighten this slightly", "brightness");
    phrase_test!(recognizes_too_bright, "This is too bright", "brightness");
    phrase_test!(
        recognizes_overexposed,
        "Reduce an overexposed image",
        "brightness"
    );
    phrase_test!(recognizes_darker, "Make this darker", "brightness");
    phrase_test!(
        recognizes_yellow,
        "Remove the yellow cast",
        "auto white balance"
    );
    phrase_test!(
        recognizes_blue_cast,
        "Fix the blue cast",
        "auto white balance"
    );
    phrase_test!(
        recognizes_indoor_lighting,
        "Fix indoor lighting",
        "auto white balance"
    );
    phrase_test!(
        recognizes_washed_out,
        "This looks washed out",
        "local contrast"
    );
    phrase_test!(recognizes_flat, "The image looks flat", "local contrast");
    phrase_test!(
        recognizes_faded,
        "Improve this faded scan",
        "local contrast"
    );
    phrase_test!(recognizes_grainy, "Reduce a grainy texture", "denoise");
    phrase_test!(recognizes_noisy, "This is noisy", "denoise");
    phrase_test!(recognizes_jpeg, "Reduce JPEG artifacts", "JPEG cleanup");
    phrase_test!(
        recognizes_compression,
        "Clean compression artifacts",
        "JPEG cleanup"
    );
    phrase_test!(recognizes_blurry, "This is blurry", "mild deblur");
    phrase_test!(recognizes_soft, "The photo is soft", "mild deblur");
    phrase_test!(recognizes_sharpen, "Sharpen slightly", "edge-aware sharpen");
    phrase_test!(
        recognizes_uneven_lighting,
        "Fix uneven lighting",
        "uneven lighting"
    );
    phrase_test!(
        recognizes_document,
        "Improve this document",
        "document enhance"
    );
    phrase_test!(
        recognizes_receipt,
        "Clean up this receipt",
        "document enhance"
    );
    phrase_test!(
        recognizes_handwriting,
        "Make handwriting easier to read",
        "document enhance"
    );
    phrase_test!(
        recognizes_notes,
        "Bring out the writing in these notes",
        "document enhance"
    );
    phrase_test!(
        recognizes_old_photo,
        "Restore this old photo",
        "local contrast"
    );
    phrase_test!(
        recognizes_harsh_contrast,
        "Reduce harsh contrast",
        "contrast"
    );

    #[test]
    fn empty_request_is_rejected() {
        assert!(matches!(
            RuleBasedPlanner.plan("   ", &analysis()),
            Err(AppError::InvalidPlan(_))
        ));
    }

    #[test]
    fn excessive_request_length_is_rejected() {
        assert!(RuleBasedPlanner
            .plan(&"x".repeat(MAX_REQUEST_CHARS + 1), &analysis())
            .is_err());
    }

    #[test]
    fn unknown_request_is_not_guessed() {
        assert!(matches!(
            RuleBasedPlanner.plan("make it magical", &analysis()),
            Err(AppError::PlannerNoMatch)
        ));
    }

    #[test]
    fn conflicting_brightness_request_is_rejected() {
        assert!(matches!(
            RuleBasedPlanner.plan("brighten this but make it darker", &analysis()),
            Err(AppError::InvalidPlan(_))
        ));
    }

    #[test]
    fn comparative_brightness_language_is_recognized() {
        let planned = RuleBasedPlanner
            .plan("make this photo brighter", &analysis())
            .unwrap();
        assert!(has_type(&planned, "brightness"));
    }

    #[test]
    fn bright_analysis_prevents_brightness_increase() {
        let mut bright = analysis();
        bright.average_luminance = 0.9;
        let planned = RuleBasedPlanner.plan("too dark", &bright).unwrap();
        assert!(!has_type(&planned, "brightness"));
        assert!(has_type(&planned, "local contrast"));
    }

    #[test]
    fn dark_analysis_prevents_brightness_decrease() {
        let mut dark = analysis();
        dark.average_luminance = 0.1;
        let planned = RuleBasedPlanner.plan("make it darker", &dark).unwrap();
        assert!(!has_type(&planned, "brightness"));
        assert!(!planned.warnings.is_empty());
    }

    #[test]
    fn strong_cast_increases_white_balance_strength() {
        let mut cast = analysis();
        cast.estimated_color_cast.dominant = "warm".into();
        let planned = RuleBasedPlanner
            .plan("make colors more natural", &cast)
            .unwrap();
        assert!(matches!(
            planned.operations[0],
            EditOperation::AutoWhiteBalance { strength } if strength > 0.6
        ));
    }

    #[test]
    fn preserve_colors_omits_white_balance() {
        let planned = plan("fix indoor lighting without changing colors");
        assert!(!has_type(&planned, "auto white balance"));
        assert!(planned
            .warnings
            .iter()
            .any(|warning| warning.contains("omitted")));
    }

    #[test]
    fn improve_without_changing_colors_is_conservative() {
        let planned = plan("improve without changing colors");
        assert_eq!(planned.operations.len(), 1);
        assert!(has_type(&planned, "local contrast"));
    }

    #[test]
    fn noisy_blur_places_denoise_before_deblur() {
        let mut noisy = analysis();
        noisy.estimated_noise = 0.3;
        let planned = RuleBasedPlanner
            .plan("fix this blurry photo", &noisy)
            .unwrap();
        assert_eq!(operation_key(&planned.operations[0]), Some("denoise"));
        assert_eq!(operation_key(&planned.operations[1]), Some("mild deblur"));
    }

    #[test]
    fn document_grayscale_is_explicit() {
        let planned = plan("Improve this document in grayscale");
        assert!(matches!(
            planned.operations[0],
            EditOperation::DocumentEnhance {
                grayscale: true,
                ..
            }
        ));
    }

    #[test]
    fn likely_document_uses_stronger_document_setting() {
        let mut document = analysis();
        document.likely_document = true;
        let planned = RuleBasedPlanner
            .plan("clean this receipt", &document)
            .unwrap();
        assert!(matches!(
            planned.operations[0],
            EditOperation::DocumentEnhance { strength, .. } if strength > 0.7
        ));
    }

    #[test]
    fn every_operation_has_an_explanation() {
        let planned = plan("old noisy blurry photo");
        assert_eq!(
            planned.operations.len(),
            planned.operation_explanations.len()
        );
        assert!(planned
            .operation_explanations
            .iter()
            .all(|explanation| !explanation.is_empty()));
    }

    #[test]
    fn confidence_is_bounded_and_heuristic() {
        let planned = plan("old noisy blurry faded photo");
        assert!((0.0..=1.0).contains(&planned.confidence));
        assert!(planned.confidence < 1.0);
    }

    #[test]
    fn generated_plan_never_exceeds_operation_limit() {
        let planned =
            plan("too dark yellow washed out noisy jpeg blurry sharpen uneven lighting old photo");
        assert!(planned.operations.len() <= MAX_PLAN_OPERATIONS);
    }

    #[test]
    fn generated_plan_round_trips_through_json() {
        let planned = plan("reduce noise and sharpen slightly");
        let encoded = serde_json::to_string(&planned).unwrap();
        let decoded: EditPlan = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, planned);
    }

    fn valid_plan(operations: Vec<EditOperation>) -> EditPlan {
        let operation_explanations = operations
            .iter()
            .map(|operation| operation_explanation(operation).into())
            .collect();
        EditPlan {
            summary: "A valid deterministic plan.".into(),
            confidence: 0.8,
            warnings: Vec::new(),
            operations,
            operation_explanations,
        }
    }

    #[test]
    fn validation_accepts_a_generated_plan() {
        assert!(validate_edit_plan(&plan("reduce noise and sharpen slightly")).is_ok());
    }

    #[test]
    fn validation_rejects_nan_confidence() {
        let mut candidate = valid_plan(vec![EditOperation::Denoise {
            strength: 0.3,
            preserve_edges: 0.8,
        }]);
        candidate.confidence = f32::NAN;
        assert!(validate_edit_plan(&candidate).is_err());
    }

    #[test]
    fn validation_rejects_infinite_confidence() {
        let mut candidate = valid_plan(vec![EditOperation::Deblock { strength: 0.3 }]);
        candidate.confidence = f32::INFINITY;
        assert!(validate_edit_plan(&candidate).is_err());
    }

    #[test]
    fn validation_rejects_out_of_range_confidence() {
        let mut candidate = valid_plan(vec![EditOperation::Deblock { strength: 0.3 }]);
        candidate.confidence = 1.2;
        assert!(validate_edit_plan(&candidate).is_err());
    }

    #[test]
    fn validation_rejects_empty_summary() {
        let mut candidate = valid_plan(vec![EditOperation::Deblock { strength: 0.3 }]);
        candidate.summary.clear();
        assert!(validate_edit_plan(&candidate).is_err());
    }

    #[test]
    fn validation_rejects_empty_operations() {
        assert!(validate_edit_plan(&valid_plan(Vec::new())).is_err());
    }

    #[test]
    fn validation_rejects_too_many_operations() {
        let operations = (0..=MAX_PLAN_OPERATIONS)
            .map(|index| EditOperation::Brightness {
                amount: index as f32 / 100.0,
            })
            .collect();
        assert!(validate_edit_plan(&valid_plan(operations)).is_err());
    }

    #[test]
    fn validation_rejects_missing_explanation() {
        let mut candidate = valid_plan(vec![EditOperation::Deblock { strength: 0.3 }]);
        candidate.operation_explanations.clear();
        assert!(validate_edit_plan(&candidate).is_err());
    }

    #[test]
    fn validation_rejects_duplicate_operations() {
        let candidate = valid_plan(vec![
            EditOperation::Deblock { strength: 0.2 },
            EditOperation::Deblock { strength: 0.5 },
        ]);
        assert!(validate_edit_plan(&candidate).is_err());
    }

    #[test]
    fn validation_rejects_unsupported_operation() {
        assert!(validate_edit_plan(&valid_plan(vec![EditOperation::Sepia])).is_err());
    }

    #[test]
    fn validation_rejects_out_of_order_operations() {
        let candidate = valid_plan(vec![
            EditOperation::EdgeAwareSharpen {
                strength: 0.2,
                radius: 1.0,
                threshold: 0.04,
            },
            EditOperation::Denoise {
                strength: 0.3,
                preserve_edges: 0.8,
            },
        ]);
        assert!(validate_edit_plan(&candidate).is_err());
    }

    #[test]
    fn validation_rejects_grayscale_saturation_conflict() {
        let candidate = valid_plan(vec![
            EditOperation::Saturation { amount: 0.2 },
            EditOperation::Grayscale,
        ]);
        assert!(validate_edit_plan(&candidate).is_err());
    }

    #[test]
    fn validation_rejects_document_grayscale_saturation_conflict() {
        let candidate = valid_plan(vec![
            EditOperation::DocumentEnhance {
                strength: 0.7,
                grayscale: true,
            },
            EditOperation::Saturation { amount: 0.2 },
        ]);
        assert!(validate_edit_plan(&candidate).is_err());
    }

    #[test]
    fn validation_rejects_invalid_operation_parameter() {
        let candidate = valid_plan(vec![EditOperation::Denoise {
            strength: f32::NAN,
            preserve_edges: 0.8,
        }]);
        assert!(validate_edit_plan(&candidate).is_err());
    }

    #[test]
    fn unknown_json_operation_is_rejected() {
        let json = r#"{"summary":"x","confidence":0.5,"warnings":[],"operations":[{"type":"invent_pixels"}],"operationExplanations":["x"]}"#;
        assert!(serde_json::from_str::<EditPlan>(json).is_err());
    }

    #[test]
    fn plan_order_is_stable_across_repeated_generation() {
        let first = plan("yellow noisy blurry faded photo");
        let second = plan("yellow noisy blurry faded photo");
        assert_eq!(first, second);
    }

    #[test]
    fn punctuation_and_case_do_not_change_matching() {
        assert_eq!(
            plan("REDUCE, JPEG ARTIFACTS!"),
            plan("reduce jpeg artifacts")
        );
    }

    #[test]
    fn planner_does_not_create_generation_or_script_operations() {
        let planned = plan("improve this old photo");
        assert!(planned
            .operations
            .iter()
            .all(|operation| operation_key(operation).is_some()));
    }
}
