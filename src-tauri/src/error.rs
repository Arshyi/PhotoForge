use serde::ser::{SerializeStruct, Serializer};
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("This file is not a supported PNG, JPEG, or WebP image.")]
    UnsupportedImageFormat,
    #[error("The image appears to be corrupt or incomplete.")]
    CorruptImage,
    #[error("This image is too large to process safely ({pixels} pixels; limit {limit}).")]
    ImageTooLarge { pixels: u64, limit: u64 },
    #[error("PhotoForge could not decode this image.")]
    DecodeFailure,
    #[error("PhotoForge could not process this edit pipeline: {0}")]
    ProcessingFailure(String),
    #[error("PhotoForge could not analyze this image. Try reopening it.")]
    AnalysisFailure,
    #[error("Image analysis is still being prepared. Try the guided request again in a moment.")]
    AnalysisUnavailable,
    #[error("PhotoForge could not match that request to supported deterministic edits. Try a suggested request or name the lighting, color, noise, sharpness, JPEG, or document problem.")]
    PlannerNoMatch,
    #[error("Invalid guided edit plan: {0}")]
    InvalidPlan(String),
    #[error("This restoration setting requires too many resources. Choose a smaller radius or tile size.")]
    RestorationResourceLimit,
    #[error("PhotoForge could not export the edited image.")]
    ExportFailure,
    #[error("Invalid edit: {0}")]
    InvalidOperation(String),
    #[error("Choose a valid output path that is different from the original image.")]
    InvalidOutputPath,
    #[error("PhotoForge does not have permission to access that location.")]
    Permission,
    #[error("This image may require more memory than is safely available.")]
    OutOfMemoryRisk,
    #[error("Open an image before applying edits or exporting.")]
    NoImageOpen,
}

impl AppError {
    fn code(&self) -> &'static str {
        match self {
            Self::UnsupportedImageFormat => "unsupported_image_format",
            Self::CorruptImage => "corrupt_image",
            Self::ImageTooLarge { .. } => "image_too_large",
            Self::DecodeFailure => "decode_failure",
            Self::ProcessingFailure(_) => "processing_failure",
            Self::AnalysisFailure => "analysis_failure",
            Self::AnalysisUnavailable => "analysis_unavailable",
            Self::PlannerNoMatch => "planner_no_match",
            Self::InvalidPlan(_) => "invalid_plan",
            Self::RestorationResourceLimit => "restoration_resource_limit",
            Self::ExportFailure => "export_failure",
            Self::InvalidOperation(_) => "invalid_operation",
            Self::InvalidOutputPath => "invalid_output_path",
            Self::Permission => "permission_error",
            Self::OutOfMemoryRisk => "out_of_memory_risk",
            Self::NoImageOpen => "no_image_open",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}
