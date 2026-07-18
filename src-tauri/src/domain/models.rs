use crate::error::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EditOperation {
    Brightness { amount: f32 },
    Contrast { amount: f32 },
    Saturation { amount: f32 },
    Gamma { value: f32 },
    Grayscale,
    Sepia,
    ReflectHorizontal,
    Rotate { degrees: i32 },
    GaussianBlur { radius: f32 },
    Sharpen { strength: f32 },
}

impl EditOperation {
    pub fn validate(&self) -> Result<(), AppError> {
        let valid = match self {
            Self::Brightness { amount } => (-1.0..=1.0).contains(amount),
            Self::Contrast { amount } => (-1.0..=1.0).contains(amount),
            Self::Saturation { amount } => (-1.0..=1.0).contains(amount),
            Self::Gamma { value } => (0.2..=3.0).contains(value),
            Self::Grayscale | Self::Sepia | Self::ReflectHorizontal => true,
            Self::Rotate { degrees } => matches!(degrees.rem_euclid(360), 0 | 90 | 180 | 270),
            Self::GaussianBlur { radius } => (0.0..=20.0).contains(radius),
            Self::Sharpen { strength } => (0.0..=2.0).contains(strength),
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
