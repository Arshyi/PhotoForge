mod components;
mod models;
mod ollama;
mod pipeline;
mod planner;
mod plugins;

pub use components::{
    ComponentActionResult, ComponentConfiguration, ComponentDiagnostics,
    ComponentPerformanceMetrics, ComponentSnapshot, EngineProvider, EngineRegistration,
    ModelDiscoveryResult, ModelMetadata, PlannerCapabilities, PlannerProvider, PlannerRegistration,
    RestorationCapabilities, RestorationEngine,
};

pub use models::{
    AnalysisResult, ColorCastEstimate, EditOperation, EditPlan, ExportResult, ImageMetadata,
    ImageQualityAnalysis, OpenImageResult, PlanResult, PreviewResult,
};
pub use ollama::{
    OllamaConnectionResult, OllamaDiagnostics, OllamaModel, OllamaModelDiscoveryResult,
    OllamaPlanResult, PlanValidationReport, PlannerComparisonEntry, PlannerComparisonResult,
};
pub use pipeline::EditPipeline;
pub(crate) use planner::operation_explanation;
pub use planner::{
    validate_edit_plan, EditPlanner, RuleBasedPlanner, RulePlanner, MAX_PLAN_OPERATIONS,
};
pub use plugins::{
    PluginManifest, PluginManifestRecord, PluginScanResult, PluginType, MAX_PLUGIN_MANIFESTS,
    MAX_PLUGIN_MANIFEST_BYTES,
};

pub type ImageAnalysis = ImageQualityAnalysis;
