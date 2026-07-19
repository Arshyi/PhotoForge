use super::EditPlan;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OllamaModel {
    pub name: String,
    pub size_bytes: u64,
    pub modified_at: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaModelDiscoveryResult {
    pub models: Vec<OllamaModel>,
    pub message: String,
    pub response_time_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaConnectionResult {
    pub connected: bool,
    pub message: String,
    pub version: String,
    pub response_time_ms: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanValidationReport {
    pub valid: bool,
    pub original_response: String,
    pub validated_response: Option<String>,
    pub rejected_fields: Vec<String>,
    pub errors: Vec<String>,
    pub validation_time_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaPlanResult {
    pub plan: Option<EditPlan>,
    pub document_id: u64,
    pub request_id: u64,
    pub model: String,
    pub generation_time_ms: f64,
    pub validation_time_ms: f64,
    pub total_time_ms: f64,
    pub is_current: bool,
    pub error: Option<String>,
    pub validation_report: PlanValidationReport,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannerComparisonEntry {
    pub provider: String,
    pub plan: Option<EditPlan>,
    pub execution_time_ms: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannerComparisonResult {
    pub rule: PlannerComparisonEntry,
    pub ollama: PlannerComparisonEntry,
    pub validation_report: Option<PlanValidationReport>,
    pub total_time_ms: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OllamaDiagnostics {
    pub connected: bool,
    pub last_error: Option<String>,
    pub last_response_time_ms: Option<f64>,
    pub connection_latency_ms: Option<f64>,
    pub generation_latency_ms: Option<f64>,
    pub validation_latency_ms: Option<f64>,
    pub rule_planner_latency_ms: Option<f64>,
    pub comparison_latency_ms: Option<f64>,
    pub model_selected: Option<String>,
    pub planner_version: String,
    pub validation_failures: u64,
    pub rejected_plans: u64,
    pub successful_plans: u64,
    pub cancelled_plans: u64,
    pub local_client_memory_estimate_mb: u64,
    pub memory_note: String,
}

impl Default for OllamaDiagnostics {
    fn default() -> Self {
        Self {
            connected: false,
            last_error: None,
            last_response_time_ms: None,
            connection_latency_ms: None,
            generation_latency_ms: None,
            validation_latency_ms: None,
            rule_planner_latency_ms: None,
            comparison_latency_ms: None,
            model_selected: None,
            planner_version: env!("CARGO_PKG_VERSION").into(),
            validation_failures: 0,
            rejected_plans: 0,
            successful_plans: 0,
            cancelled_plans: 0,
            local_client_memory_estimate_mb: 1,
            memory_note: "The Ollama process is external; PhotoForge retains no model weights."
                .into(),
        }
    }
}
