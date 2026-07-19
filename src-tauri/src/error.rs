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
    #[error("Planner not installed.")]
    PlannerNotInstalled,
    #[error("Ollama refused the local connection. Start Ollama and try Test Connection again.")]
    OllamaConnectionRefused,
    #[error("The local Ollama request timed out. Increase the timeout or choose a smaller model.")]
    OllamaTimeout,
    #[error("The configured local Ollama host could not be reached. Check the endpoint and local service.")]
    OllamaHostUnreachable,
    #[error("The Ollama endpoint is invalid. Use an explicit local address such as http://127.0.0.1:11434.")]
    InvalidPlannerEndpoint,
    #[error(
        "The selected Ollama model is not installed. Refresh Models and choose an installed model."
    )]
    OllamaModelMissing,
    #[error("Ollama returned malformed JSON or non-UTF-8 data.")]
    OllamaJsonParse,
    #[error("The local Ollama server returned HTTP status {0}.")]
    OllamaHttpStatus(u16),
    #[error("Ollama plan validation failed: {0}")]
    OllamaSchemaValidation(String),
    #[error("The Ollama planning request was cancelled.")]
    PlannerCancellation,
    #[error("The local planner is already handling another request. Cancel it or wait for it to finish.")]
    PlannerBusy,
    #[error("Ollama returned more than the configured {limit} byte response limit.")]
    OllamaResponseTooLarge { limit: u64 },
    #[error("The local Ollama server reported an unsupported planner response version.")]
    UnsupportedPlannerVersion,
    #[error("Restoration engine not installed.")]
    RestorationEngineNotInstalled,
    #[error("Invalid component configuration: {0}")]
    InvalidComponentConfiguration(String),
    #[error("Component initialization timed out.")]
    ComponentInitializationTimeout,
    #[error("Component initialization failed: {0}")]
    ComponentInitializationFailure(String),
    #[error("Invalid plugin manifest: {0}")]
    InvalidPluginManifest(String),
    #[error("Local model discovery failed: {0}")]
    ModelDiscoveryFailure(String),
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
            Self::PlannerNotInstalled => "planner_not_installed",
            Self::OllamaConnectionRefused => "ollama_connection_refused",
            Self::OllamaTimeout => "ollama_timeout",
            Self::OllamaHostUnreachable => "ollama_host_unreachable",
            Self::InvalidPlannerEndpoint => "invalid_planner_endpoint",
            Self::OllamaModelMissing => "ollama_model_missing",
            Self::OllamaJsonParse => "ollama_json_parse_error",
            Self::OllamaHttpStatus(_) => "ollama_http_status",
            Self::OllamaSchemaValidation(_) => "ollama_schema_validation_failure",
            Self::PlannerCancellation => "planner_cancellation",
            Self::PlannerBusy => "planner_busy",
            Self::OllamaResponseTooLarge { .. } => "ollama_response_too_large",
            Self::UnsupportedPlannerVersion => "unsupported_planner_version",
            Self::RestorationEngineNotInstalled => "restoration_engine_not_installed",
            Self::InvalidComponentConfiguration(_) => "invalid_component_configuration",
            Self::ComponentInitializationTimeout => "component_initialization_timeout",
            Self::ComponentInitializationFailure(_) => "component_initialization_failure",
            Self::InvalidPluginManifest(_) => "invalid_plugin_manifest",
            Self::ModelDiscoveryFailure(_) => "model_discovery_failure",
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
