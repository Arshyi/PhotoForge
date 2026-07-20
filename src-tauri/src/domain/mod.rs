mod components;
mod models;
mod ollama;
mod pipeline;
mod planner;
mod plugins;
mod professional;

pub use components::{
    ComponentActionResult, ComponentConfiguration, ComponentDiagnostics,
    ComponentPerformanceMetrics, ComponentSnapshot, EngineProvider, EngineRegistration,
    ModelDiscoveryResult, ModelMetadata, PlannerCapabilities, PlannerProvider, PlannerRegistration,
    RestorationCapabilities, RestorationEngine,
};

pub use models::{
    AnalysisResult, ColorCastEstimate, CropOverlay, CurvePoint, CurveSet, EditOperation, EditPlan,
    ExportResult, HslAdjustment, HslSettings, ImageMetadata, ImageQualityAnalysis, OpenImageResult,
    PerspectiveCorners, PlanResult, PreviewResult, SelectiveColorAdjustment,
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
pub use professional::{
    validate_shortcuts, BatchFailureRecord, BatchOptions, BatchPreview, BatchState, BatchStatus,
    ExportProfile, HistogramChannels, HistogramResult, PixelInspection, ShortcutBinding, Workflow,
    WorkflowDocument, WorkspaceLayout, MAX_BATCH_FILES, MAX_BATCH_WORKERS, MAX_WORKFLOW_OPERATIONS,
    WORKFLOW_SCHEMA_VERSION,
};

pub type ImageAnalysis = ImageQualityAnalysis;
