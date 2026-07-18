use crate::error::AppError;
use serde::{Deserialize, Serialize};

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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageMetadata {
    pub filename: String,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub file_size: u64,
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
}
