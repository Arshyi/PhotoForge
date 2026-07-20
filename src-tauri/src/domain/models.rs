use crate::error::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurvePoint {
    pub input: f32,
    pub output: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurveSet {
    pub rgb: Vec<CurvePoint>,
    pub red: Vec<CurvePoint>,
    pub green: Vec<CurvePoint>,
    pub blue: Vec<CurvePoint>,
}

impl Default for CurveSet {
    fn default() -> Self {
        let identity = vec![
            CurvePoint {
                input: 0.0,
                output: 0.0,
            },
            CurvePoint {
                input: 1.0,
                output: 1.0,
            },
        ];
        Self {
            rgb: identity.clone(),
            red: identity.clone(),
            green: identity.clone(),
            blue: identity,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HslAdjustment {
    pub hue: f32,
    pub saturation: f32,
    pub lightness: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HslSettings {
    pub master: HslAdjustment,
    pub red: HslAdjustment,
    pub yellow: HslAdjustment,
    pub green: HslAdjustment,
    pub cyan: HslAdjustment,
    pub blue: HslAdjustment,
    pub magenta: HslAdjustment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CropOverlay {
    #[default]
    None,
    RuleOfThirds,
    GoldenRatio,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerspectiveCorners {
    pub top_left: [f32; 2],
    pub top_right: [f32; 2],
    pub bottom_right: [f32; 2],
    pub bottom_left: [f32; 2],
}

impl Default for PerspectiveCorners {
    fn default() -> Self {
        Self {
            top_left: [0.0, 0.0],
            top_right: [1.0, 0.0],
            bottom_right: [1.0, 1.0],
            bottom_left: [0.0, 1.0],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SelectiveColorAdjustment {
    pub cyan: f32,
    pub magenta: f32,
    pub yellow: f32,
    pub black: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EditOperation {
    Brightness {
        amount: f32,
    },
    Contrast {
        amount: f32,
    },
    Saturation {
        amount: f32,
    },
    Gamma {
        value: f32,
    },
    Grayscale,
    Sepia,
    ReflectHorizontal,
    Rotate {
        degrees: i32,
    },
    GaussianBlur {
        radius: f32,
    },
    Sharpen {
        strength: f32,
    },
    AutoWhiteBalance {
        strength: f32,
    },
    LocalContrast {
        strength: f32,
        tile_size: u32,
        clip_limit: f32,
    },
    Denoise {
        strength: f32,
        preserve_edges: f32,
    },
    Deblock {
        strength: f32,
    },
    EdgeAwareSharpen {
        strength: f32,
        radius: f32,
        threshold: f32,
    },
    MildDeblur {
        strength: f32,
        radius: f32,
    },
    DocumentEnhance {
        strength: f32,
        grayscale: bool,
    },
    UnevenLightingCorrection {
        strength: f32,
        radius: f32,
    },
    Curves {
        curves: CurveSet,
    },
    Levels {
        input_black: u8,
        input_white: u8,
        gamma: f32,
        output_black: u8,
        output_white: u8,
    },
    WhitePoint {
        red: u8,
        green: u8,
        blue: u8,
    },
    BlackPoint {
        red: u8,
        green: u8,
        blue: u8,
    },
    Crop {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        aspect_ratio: Option<String>,
        overlay: CropOverlay,
    },
    Straighten {
        degrees: f32,
    },
    Perspective {
        corners: PerspectiveCorners,
    },
    LensCorrection {
        distortion: f32,
        vignetting: f32,
        chromatic_aberration: f32,
    },
    Hsl {
        settings: HslSettings,
    },
    TemperatureTint {
        temperature: f32,
        tint: f32,
    },
    SelectiveColor {
        target_hue: f32,
        width: f32,
        adjustment: SelectiveColorAdjustment,
    },
}

impl EditOperation {
    pub fn validate(&self) -> Result<(), AppError> {
        let exceeds_resource_limit = match self {
            Self::LocalContrast { tile_size, .. } => *tile_size > 128,
            Self::EdgeAwareSharpen { radius, .. } => radius.is_finite() && *radius > 4.0,
            Self::MildDeblur { radius, .. } => radius.is_finite() && *radius > 3.0,
            Self::UnevenLightingCorrection { radius, .. } => radius.is_finite() && *radius > 96.0,
            _ => false,
        };
        if exceeds_resource_limit {
            return Err(AppError::RestorationResourceLimit);
        }

        let valid = match self {
            Self::Brightness { amount } => (-1.0..=1.0).contains(amount),
            Self::Contrast { amount } => (-1.0..=1.0).contains(amount),
            Self::Saturation { amount } => (-1.0..=1.0).contains(amount),
            Self::Gamma { value } => (0.2..=3.0).contains(value),
            Self::Grayscale | Self::Sepia | Self::ReflectHorizontal => true,
            Self::Rotate { degrees } => matches!(degrees.rem_euclid(360), 0 | 90 | 180 | 270),
            Self::GaussianBlur { radius } => (0.0..=20.0).contains(radius),
            Self::Sharpen { strength } => (0.0..=2.0).contains(strength),
            Self::AutoWhiteBalance { strength }
            | Self::Deblock { strength }
            | Self::DocumentEnhance { strength, .. } => (0.0..=1.0).contains(strength),
            Self::LocalContrast {
                strength,
                tile_size,
                clip_limit,
            } => {
                (0.0..=1.0).contains(strength)
                    && (8..=128).contains(tile_size)
                    && (0.5..=4.0).contains(clip_limit)
            }
            Self::Denoise {
                strength,
                preserve_edges,
            } => (0.0..=1.0).contains(strength) && (0.0..=1.0).contains(preserve_edges),
            Self::EdgeAwareSharpen {
                strength,
                radius,
                threshold,
            } => {
                (0.0..=2.0).contains(strength)
                    && (0.5..=4.0).contains(radius)
                    && (0.0..=0.25).contains(threshold)
            }
            Self::MildDeblur { strength, radius } => {
                (0.0..=1.0).contains(strength) && (0.5..=3.0).contains(radius)
            }
            Self::UnevenLightingCorrection { strength, radius } => {
                (0.0..=1.0).contains(strength) && (4.0..=96.0).contains(radius)
            }
            Self::Curves { curves } => validate_curve_set(curves),
            Self::Levels {
                input_black,
                input_white,
                gamma,
                output_black,
                output_white,
            } => {
                input_black < input_white
                    && output_black <= output_white
                    && gamma.is_finite()
                    && (0.1..=10.0).contains(gamma)
            }
            Self::WhitePoint { red, green, blue } => *red > 0 && *green > 0 && *blue > 0,
            Self::BlackPoint { .. } => true,
            Self::Crop {
                x,
                y,
                width,
                height,
                aspect_ratio,
                ..
            } => {
                [*x, *y, *width, *height]
                    .iter()
                    .all(|value| value.is_finite())
                    && *x >= 0.0
                    && *y >= 0.0
                    && *width > 0.0
                    && *height > 0.0
                    && *x + *width <= 1.000_001
                    && *y + *height <= 1.000_001
                    && aspect_ratio
                        .as_ref()
                        .map_or(true, |ratio| ratio.len() <= 32)
            }
            Self::Straighten { degrees } => degrees.is_finite() && (-45.0..=45.0).contains(degrees),
            Self::Perspective { corners } => validate_corners(corners),
            Self::LensCorrection {
                distortion,
                vignetting,
                chromatic_aberration,
            } => {
                distortion.is_finite()
                    && vignetting.is_finite()
                    && chromatic_aberration.is_finite()
                    && (-1.0..=1.0).contains(distortion)
                    && (-1.0..=1.0).contains(vignetting)
                    && (-1.0..=1.0).contains(chromatic_aberration)
            }
            Self::Hsl { settings } => validate_hsl(settings),
            Self::TemperatureTint { temperature, tint } => {
                temperature.is_finite()
                    && tint.is_finite()
                    && (-1.0..=1.0).contains(temperature)
                    && (-1.0..=1.0).contains(tint)
            }
            Self::SelectiveColor {
                target_hue,
                width,
                adjustment,
            } => {
                target_hue.is_finite()
                    && width.is_finite()
                    && (0.0..=360.0).contains(target_hue)
                    && (1.0..=180.0).contains(width)
                    && [
                        adjustment.cyan,
                        adjustment.magenta,
                        adjustment.yellow,
                        adjustment.black,
                    ]
                    .iter()
                    .all(|value| value.is_finite() && (-1.0..=1.0).contains(value))
            }
        };

        if valid {
            Ok(())
        } else {
            Err(AppError::InvalidOperation(format!(
                "parameter is outside the supported range for {self:?}"
            )))
        }
    }
}

fn validate_curve(points: &[CurvePoint]) -> bool {
    (2..=32).contains(&points.len())
        && points.iter().all(|point| {
            point.input.is_finite()
                && point.output.is_finite()
                && (0.0..=1.0).contains(&point.input)
                && (0.0..=1.0).contains(&point.output)
        })
        && points.windows(2).all(|pair| pair[0].input < pair[1].input)
        && points.first().is_some_and(|point| point.input == 0.0)
        && points.last().is_some_and(|point| point.input == 1.0)
}

fn validate_curve_set(curves: &CurveSet) -> bool {
    validate_curve(&curves.rgb)
        && validate_curve(&curves.red)
        && validate_curve(&curves.green)
        && validate_curve(&curves.blue)
}

fn validate_hsl(settings: &HslSettings) -> bool {
    [
        settings.master,
        settings.red,
        settings.yellow,
        settings.green,
        settings.cyan,
        settings.blue,
        settings.magenta,
    ]
    .iter()
    .all(|adjustment| {
        adjustment.hue.is_finite()
            && adjustment.saturation.is_finite()
            && adjustment.lightness.is_finite()
            && (-180.0..=180.0).contains(&adjustment.hue)
            && (-1.0..=1.0).contains(&adjustment.saturation)
            && (-1.0..=1.0).contains(&adjustment.lightness)
    })
}

fn validate_corners(corners: &PerspectiveCorners) -> bool {
    let points = [
        corners.top_left,
        corners.top_right,
        corners.bottom_right,
        corners.bottom_left,
    ];
    points
        .iter()
        .flatten()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
        && corners.top_left[0] < corners.top_right[0]
        && corners.bottom_left[0] < corners.bottom_right[0]
        && corners.top_left[1] < corners.bottom_left[1]
        && corners.top_right[1] < corners.bottom_right[1]
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageMetadata {
    pub filename: String,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub file_size: u64,
    pub color_space: String,
    pub bit_depth: u8,
    pub has_alpha: bool,
    pub created_at: Option<String>,
    pub modified_at: Option<String>,
    pub camera_model: Option<String>,
    pub exif_available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenImageResult {
    pub metadata: ImageMetadata,
    pub original_preview_data_url: String,
    pub preview_data_url: String,
    pub processing_time_ms: f64,
    pub document_id: u64,
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewResult {
    pub preview_data_url: String,
    pub request_id: u64,
    pub processing_time_ms: f64,
    pub is_current: bool,
    pub operation_count: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub output_path: String,
    pub width: u32,
    pub height: u32,
    pub processing_time_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColorCastEstimate {
    pub dominant: String,
    pub red_bias: f32,
    pub green_bias: f32,
    pub blue_bias: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageQualityAnalysis {
    pub average_luminance: f32,
    pub luminance_spread: f32,
    pub estimated_color_cast: ColorCastEstimate,
    pub estimated_noise: f32,
    pub estimated_sharpness: f32,
    pub estimated_local_contrast: f32,
    pub edge_density: f32,
    pub white_background_ratio: f32,
    pub likely_document: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisResult {
    pub analysis: Option<ImageQualityAnalysis>,
    pub document_id: u64,
    pub request_id: u64,
    pub processing_time_ms: f64,
    pub is_current: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditPlan {
    pub summary: String,
    pub confidence: f32,
    pub warnings: Vec<String>,
    pub operations: Vec<EditOperation>,
    pub operation_explanations: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanResult {
    pub plan: Option<EditPlan>,
    pub document_id: u64,
    pub request_id: u64,
    pub processing_time_ms: f64,
    pub is_current: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restoration_operations_round_trip_through_tagged_json() {
        let operations = vec![
            EditOperation::AutoWhiteBalance { strength: 0.7 },
            EditOperation::LocalContrast {
                strength: 0.6,
                tile_size: 32,
                clip_limit: 1.5,
            },
            EditOperation::Denoise {
                strength: 0.4,
                preserve_edges: 0.8,
            },
            EditOperation::Deblock { strength: 0.5 },
            EditOperation::EdgeAwareSharpen {
                strength: 0.7,
                radius: 1.5,
                threshold: 0.03,
            },
            EditOperation::MildDeblur {
                strength: 0.4,
                radius: 1.2,
            },
            EditOperation::DocumentEnhance {
                strength: 0.8,
                grayscale: true,
            },
            EditOperation::UnevenLightingCorrection {
                strength: 0.75,
                radius: 32.0,
            },
        ];
        let json = serde_json::to_string(&operations).unwrap();
        let decoded: Vec<EditOperation> = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, operations);
        assert!(json.contains("auto_white_balance"));
        assert!(json.contains("document_enhance"));
    }

    macro_rules! valid_operation_test {
        ($name:ident, $operation:expr) => {
            #[test]
            fn $name() {
                $operation.validate().unwrap();
            }
        };
    }

    macro_rules! invalid_operation_test {
        ($name:ident, $operation:expr) => {
            #[test]
            fn $name() {
                assert!(matches!(
                    $operation.validate(),
                    Err(AppError::InvalidOperation(_))
                ));
            }
        };
    }

    valid_operation_test!(
        accepts_identity_curves,
        EditOperation::Curves {
            curves: CurveSet::default()
        }
    );
    valid_operation_test!(
        accepts_levels_bounds,
        EditOperation::Levels {
            input_black: 0,
            input_white: 255,
            gamma: 1.0,
            output_black: 0,
            output_white: 255
        }
    );
    valid_operation_test!(
        accepts_white_point,
        EditOperation::WhitePoint {
            red: 200,
            green: 210,
            blue: 220
        }
    );
    valid_operation_test!(
        accepts_black_point,
        EditOperation::BlackPoint {
            red: 0,
            green: 4,
            blue: 8
        }
    );
    valid_operation_test!(
        accepts_normalized_crop,
        EditOperation::Crop {
            x: 0.1,
            y: 0.2,
            width: 0.8,
            height: 0.7,
            aspect_ratio: Some("16:9".into()),
            overlay: CropOverlay::RuleOfThirds
        }
    );
    valid_operation_test!(
        accepts_straighten_limit,
        EditOperation::Straighten { degrees: -45.0 }
    );
    valid_operation_test!(
        accepts_identity_perspective,
        EditOperation::Perspective {
            corners: PerspectiveCorners::default()
        }
    );
    valid_operation_test!(
        accepts_lens_correction_bounds,
        EditOperation::LensCorrection {
            distortion: -1.0,
            vignetting: 1.0,
            chromatic_aberration: 0.25
        }
    );
    valid_operation_test!(
        accepts_neutral_hsl,
        EditOperation::Hsl {
            settings: HslSettings::default()
        }
    );
    valid_operation_test!(
        accepts_temperature_tint_bounds,
        EditOperation::TemperatureTint {
            temperature: 1.0,
            tint: -1.0
        }
    );
    valid_operation_test!(
        accepts_selective_color_bounds,
        EditOperation::SelectiveColor {
            target_hue: 360.0,
            width: 180.0,
            adjustment: SelectiveColorAdjustment {
                cyan: -1.0,
                magenta: 1.0,
                yellow: 0.0,
                black: 0.5
            }
        }
    );

    invalid_operation_test!(
        rejects_unsorted_curve_points,
        EditOperation::Curves {
            curves: CurveSet {
                rgb: vec![
                    CurvePoint {
                        input: 0.0,
                        output: 0.0
                    },
                    CurvePoint {
                        input: 0.7,
                        output: 0.7
                    },
                    CurvePoint {
                        input: 0.6,
                        output: 1.0
                    }
                ],
                ..CurveSet::default()
            }
        }
    );
    invalid_operation_test!(
        rejects_levels_reversed_input,
        EditOperation::Levels {
            input_black: 240,
            input_white: 10,
            gamma: 1.0,
            output_black: 0,
            output_white: 255
        }
    );
    invalid_operation_test!(
        rejects_zero_white_point_channel,
        EditOperation::WhitePoint {
            red: 0,
            green: 100,
            blue: 100
        }
    );
    invalid_operation_test!(
        rejects_crop_outside_canvas,
        EditOperation::Crop {
            x: 0.5,
            y: 0.0,
            width: 0.6,
            height: 1.0,
            aspect_ratio: None,
            overlay: CropOverlay::None
        }
    );
    invalid_operation_test!(
        rejects_excessive_straighten,
        EditOperation::Straighten { degrees: 45.1 }
    );
    invalid_operation_test!(
        rejects_crossed_perspective_corners,
        EditOperation::Perspective {
            corners: PerspectiveCorners {
                top_left: [0.8, 0.0],
                top_right: [0.2, 0.0],
                ..PerspectiveCorners::default()
            }
        }
    );
    invalid_operation_test!(
        rejects_excessive_lens_distortion,
        EditOperation::LensCorrection {
            distortion: 1.1,
            vignetting: 0.0,
            chromatic_aberration: 0.0
        }
    );
    invalid_operation_test!(
        rejects_non_finite_hsl,
        EditOperation::Hsl {
            settings: HslSettings {
                master: HslAdjustment {
                    hue: f32::NAN,
                    ..Default::default()
                },
                ..Default::default()
            }
        }
    );
    invalid_operation_test!(
        rejects_excessive_temperature,
        EditOperation::TemperatureTint {
            temperature: 1.01,
            tint: 0.0
        }
    );
    invalid_operation_test!(
        rejects_zero_selective_width,
        EditOperation::SelectiveColor {
            target_hue: 0.0,
            width: 0.0,
            adjustment: SelectiveColorAdjustment::default()
        }
    );

    #[test]
    fn professional_operations_round_trip_through_tagged_json() {
        let operations = vec![
            EditOperation::Curves {
                curves: CurveSet::default(),
            },
            EditOperation::Levels {
                input_black: 4,
                input_white: 250,
                gamma: 1.1,
                output_black: 2,
                output_white: 252,
            },
            EditOperation::Perspective {
                corners: PerspectiveCorners::default(),
            },
            EditOperation::TemperatureTint {
                temperature: 0.2,
                tint: -0.1,
            },
        ];
        let json = serde_json::to_string(&operations).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<EditOperation>>(&json).unwrap(),
            operations
        );
        assert!(json.contains("perspective"));
    }
}
