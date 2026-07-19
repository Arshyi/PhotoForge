use super::EditOperation;
use crate::error::AppError;
use image::DynamicImage;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerProvider {
    Rule,
    Ollama,
    OpenAi,
    Future,
}

impl PlannerProvider {
    pub const ALL: [Self; 4] = [Self::Rule, Self::Ollama, Self::OpenAi, Self::Future];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Rule => "rule",
            Self::Ollama => "ollama",
            Self::OpenAi => "open_ai",
            Self::Future => "future",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Rule => "Rule Planner",
            Self::Ollama => "Ollama Planner",
            Self::OpenAi => "OpenAI Planner",
            Self::Future => "Future Planner",
        }
    }
}

impl fmt::Display for PlannerProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.id())
    }
}

impl FromStr for PlannerProvider {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "rule" | "rule_planner" => Ok(Self::Rule),
            "ollama" | "ollama_planner" => Ok(Self::Ollama),
            "open_ai" | "openai" | "openai_planner" => Ok(Self::OpenAi),
            "future" | "future_planner" => Ok(Self::Future),
            _ => Err(AppError::InvalidComponentConfiguration(
                "unknown planner provider".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineProvider {
    Deterministic,
    Onnx,
    RealEsrgan,
    Future,
}

impl EngineProvider {
    pub const ALL: [Self; 4] = [
        Self::Deterministic,
        Self::Onnx,
        Self::RealEsrgan,
        Self::Future,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Deterministic => "deterministic",
            Self::Onnx => "onnx",
            Self::RealEsrgan => "real_esrgan",
            Self::Future => "future",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Deterministic => "Deterministic Engine",
            Self::Onnx => "ONNX Restoration",
            Self::RealEsrgan => "Real-ESRGAN",
            Self::Future => "Future Engine",
        }
    }
}

impl fmt::Display for EngineProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.id())
    }
}

impl FromStr for EngineProvider {
    type Err = AppError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "deterministic" | "deterministic_engine" => Ok(Self::Deterministic),
            "onnx" | "onnx_restoration" => Ok(Self::Onnx),
            "real_esrgan" | "realesrgan" | "esrgan" => Ok(Self::RealEsrgan),
            "future" | "future_engine" => Ok(Self::Future),
            _ => Err(AppError::InvalidComponentConfiguration(
                "unknown restoration engine provider".into(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannerCapabilities {
    pub supports_guided_editing: bool,
    pub supports_reasoning: bool,
    pub requires_model: bool,
    pub offline: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorationCapabilities {
    pub supports_restoration: bool,
    pub supports_neural_models: bool,
    pub requires_model: bool,
    pub offline: bool,
    pub preserves_alpha: bool,
    pub max_input_megapixels: f32,
}

pub trait RestorationEngine: Send + Sync {
    fn provider(&self) -> EngineProvider;

    fn capabilities(&self) -> RestorationCapabilities;

    fn process(
        &self,
        image: &DynamicImage,
        operations: &[EditOperation],
    ) -> Result<DynamicImage, AppError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannerRegistration {
    pub id: String,
    pub name: String,
    pub version: String,
    pub provider: String,
    pub memory_estimate_mb: u32,
    pub installed: bool,
    pub loaded: bool,
    pub active: bool,
    pub unavailable_reason: Option<String>,
    pub capabilities: PlannerCapabilities,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineRegistration {
    pub id: String,
    pub name: String,
    pub version: String,
    pub provider: String,
    pub memory_estimate_mb: u32,
    pub installed: bool,
    pub loaded: bool,
    pub active: bool,
    pub unavailable_reason: Option<String>,
    pub capabilities: RestorationCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentConfiguration {
    pub active_planner: PlannerProvider,
    pub active_engine: EngineProvider,
    pub planner_endpoint: String,
    pub initialization_timeout_ms: u64,
    #[serde(default = "default_ollama_timeout_ms")]
    pub ollama_timeout_ms: u64,
    #[serde(default = "default_ollama_max_response_bytes")]
    pub ollama_max_response_bytes: u64,
    #[serde(default)]
    pub ollama_selected_model: Option<String>,
    #[serde(default = "default_ollama_max_operations")]
    pub ollama_max_operations: usize,
    pub model_directories: Vec<String>,
    pub plugin_directory: String,
}

impl ComponentConfiguration {
    pub fn validate(&self) -> Result<(), AppError> {
        let endpoint = self.planner_endpoint.trim();
        if endpoint.is_empty() || endpoint.len() > 2_048 {
            return Err(AppError::InvalidComponentConfiguration(
                "planner endpoint must contain 1 to 2,048 characters".into(),
            ));
        }
        if !is_local_loopback_endpoint(endpoint) {
            return Err(AppError::InvalidComponentConfiguration(
                "Ollama endpoints must use an explicit local loopback address".into(),
            ));
        }
        if !(100..=30_000).contains(&self.initialization_timeout_ms) {
            return Err(AppError::InvalidComponentConfiguration(
                "initialization timeout must be between 100 and 30,000 milliseconds".into(),
            ));
        }
        if !(250..=120_000).contains(&self.ollama_timeout_ms) {
            return Err(AppError::InvalidComponentConfiguration(
                "Ollama timeout must be between 250 and 120,000 milliseconds".into(),
            ));
        }
        if !(1_024..=2_097_152).contains(&self.ollama_max_response_bytes) {
            return Err(AppError::InvalidComponentConfiguration(
                "Ollama maximum response size must be between 1 KiB and 2 MiB".into(),
            ));
        }
        if self.ollama_max_operations == 0 || self.ollama_max_operations > 8 {
            return Err(AppError::InvalidComponentConfiguration(
                "Ollama may generate between one and eight operations".into(),
            ));
        }
        if self.ollama_selected_model.as_ref().is_some_and(|model| {
            model.trim().is_empty()
                || model.chars().count() > 200
                || model.chars().any(char::is_control)
        }) {
            return Err(AppError::InvalidComponentConfiguration(
                "selected Ollama model names must contain 1 to 200 printable characters".into(),
            ));
        }
        if self.model_directories.len() > 8 {
            return Err(AppError::InvalidComponentConfiguration(
                "at most eight model directories may be configured".into(),
            ));
        }
        if self
            .model_directories
            .iter()
            .any(|directory| directory.trim().is_empty() || directory.len() > 1_024)
        {
            return Err(AppError::InvalidComponentConfiguration(
                "model directory paths must contain 1 to 1,024 characters".into(),
            ));
        }
        if self.plugin_directory.trim().is_empty() || self.plugin_directory.len() > 1_024 {
            return Err(AppError::InvalidComponentConfiguration(
                "plugin directory path must contain 1 to 1,024 characters".into(),
            ));
        }
        Ok(())
    }
}

pub const fn default_ollama_timeout_ms() -> u64 {
    15_000
}

pub const fn default_ollama_max_response_bytes() -> u64 {
    256 * 1_024
}

pub const fn default_ollama_max_operations() -> usize {
    8
}

fn is_local_loopback_endpoint(endpoint: &str) -> bool {
    let Some(remainder) = endpoint.strip_prefix("http://") else {
        return false;
    };
    let authority = remainder.split(['/', '?', '#']).next().unwrap_or_default();
    ["localhost", "127.0.0.1", "[::1]"]
        .into_iter()
        .any(|host| valid_loopback_authority(authority, host))
}

fn valid_loopback_authority(authority: &str, host: &str) -> bool {
    if authority == host {
        return true;
    }
    authority
        .strip_prefix(&format!("{host}:"))
        .and_then(|port| port.parse::<u16>().ok())
        .is_some_and(|port| port > 0)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentSnapshot {
    pub application_version: String,
    pub planners: Vec<PlannerRegistration>,
    pub engines: Vec<EngineRegistration>,
    pub configuration: ComponentConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDiagnostics {
    pub application_version: String,
    pub registered_planners: Vec<String>,
    pub registered_engines: Vec<String>,
    pub loaded_components: Vec<String>,
    pub unavailable_components: Vec<String>,
    pub initialization_failures: Vec<String>,
    pub plugin_validation_errors: Vec<String>,
    pub configuration_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentActionResult {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentPerformanceMetrics {
    pub samples: u32,
    pub registry_lookup_average_ns: u64,
    pub planner_dispatch_average_ns: u64,
    pub component_factory_average_ns: u64,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMetadata {
    pub name: String,
    pub path: String,
    pub format: String,
    pub file_size_bytes: u64,
    pub memory_estimate_mb: u64,
    pub supported_tasks: Vec<String>,
    pub expected_input: String,
    pub expected_input_size: Option<Vec<u32>>,
    pub expected_output: String,
    pub compatible: bool,
    pub unavailable_reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDiscoveryResult {
    pub searched_directories: Vec<String>,
    pub models: Vec<ModelMetadata>,
    pub message: String,
    pub processing_time_ms: u128,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configuration() -> ComponentConfiguration {
        ComponentConfiguration {
            active_planner: PlannerProvider::Rule,
            active_engine: EngineProvider::Deterministic,
            planner_endpoint: "http://localhost:11434".into(),
            initialization_timeout_ms: 5_000,
            ollama_timeout_ms: default_ollama_timeout_ms(),
            ollama_max_response_bytes: default_ollama_max_response_bytes(),
            ollama_selected_model: None,
            ollama_max_operations: default_ollama_max_operations(),
            model_directories: vec!["models".into()],
            plugin_directory: "plugins".into(),
        }
    }

    #[test]
    fn planner_provider_parses_all_supported_ids() {
        assert_eq!(
            PlannerProvider::from_str("rule").unwrap(),
            PlannerProvider::Rule
        );
        assert_eq!(
            PlannerProvider::from_str("OLLAMA").unwrap(),
            PlannerProvider::Ollama
        );
        assert_eq!(
            PlannerProvider::from_str("openai").unwrap(),
            PlannerProvider::OpenAi
        );
        assert_eq!(
            PlannerProvider::from_str("future_planner").unwrap(),
            PlannerProvider::Future
        );
    }

    #[test]
    fn planner_provider_rejects_unknown_ids() {
        assert!(PlannerProvider::from_str("cloud_magic").is_err());
    }

    #[test]
    fn engine_provider_parses_all_supported_ids() {
        assert_eq!(
            EngineProvider::from_str("deterministic").unwrap(),
            EngineProvider::Deterministic
        );
        assert_eq!(
            EngineProvider::from_str("onnx").unwrap(),
            EngineProvider::Onnx
        );
        assert_eq!(
            EngineProvider::from_str("esrgan").unwrap(),
            EngineProvider::RealEsrgan
        );
        assert_eq!(
            EngineProvider::from_str("future_engine").unwrap(),
            EngineProvider::Future
        );
    }

    #[test]
    fn engine_provider_rejects_unknown_ids() {
        assert!(EngineProvider::from_str("python").is_err());
    }

    #[test]
    fn provider_ids_and_display_names_are_stable() {
        assert_eq!(PlannerProvider::Rule.id(), "rule");
        assert_eq!(PlannerProvider::OpenAi.display_name(), "OpenAI Planner");
        assert_eq!(EngineProvider::RealEsrgan.id(), "real_esrgan");
        assert_eq!(
            EngineProvider::Deterministic.display_name(),
            "Deterministic Engine"
        );
    }

    #[test]
    fn configuration_accepts_local_loopback_endpoint() {
        assert!(configuration().validate().is_ok());
        let mut ipv4 = configuration();
        ipv4.planner_endpoint = "http://127.0.0.1:11434".into();
        assert!(ipv4.validate().is_ok());
    }

    #[test]
    fn configuration_rejects_remote_endpoint() {
        let mut candidate = configuration();
        candidate.planner_endpoint = "https://example.com".into();
        assert!(candidate.validate().is_err());
    }

    #[test]
    fn configuration_rejects_loopback_prefix_spoofing() {
        for endpoint in [
            "http://localhost.example.com:11434",
            "http://127.0.0.1.example.com",
            "http://localhost@remote.example",
            "https://localhost:11434",
            "http://localhost:70000",
        ] {
            let mut candidate = configuration();
            candidate.planner_endpoint = endpoint.into();
            assert!(candidate.validate().is_err(), "accepted {endpoint}");
        }
    }

    #[test]
    fn configuration_rejects_empty_endpoint() {
        let mut candidate = configuration();
        candidate.planner_endpoint.clear();
        assert!(candidate.validate().is_err());
    }

    #[test]
    fn configuration_rejects_short_timeout() {
        let mut candidate = configuration();
        candidate.initialization_timeout_ms = 99;
        assert!(candidate.validate().is_err());
    }

    #[test]
    fn configuration_rejects_long_timeout() {
        let mut candidate = configuration();
        candidate.initialization_timeout_ms = 30_001;
        assert!(candidate.validate().is_err());
    }

    #[test]
    fn configuration_rejects_too_many_model_directories() {
        let mut candidate = configuration();
        candidate.model_directories = (0..9).map(|index| format!("models-{index}")).collect();
        assert!(candidate.validate().is_err());
    }

    #[test]
    fn configuration_rejects_empty_model_directory() {
        let mut candidate = configuration();
        candidate.model_directories = vec![" ".into()];
        assert!(candidate.validate().is_err());
    }

    #[test]
    fn configuration_rejects_empty_plugin_directory() {
        let mut candidate = configuration();
        candidate.plugin_directory.clear();
        assert!(candidate.validate().is_err());
    }

    #[test]
    fn providers_round_trip_through_json() {
        let json = serde_json::to_string(&PlannerProvider::OpenAi).unwrap();
        assert_eq!(
            serde_json::from_str::<PlannerProvider>(&json).unwrap(),
            PlannerProvider::OpenAi
        );
        let json = serde_json::to_string(&EngineProvider::RealEsrgan).unwrap();
        assert_eq!(
            serde_json::from_str::<EngineProvider>(&json).unwrap(),
            EngineProvider::RealEsrgan
        );
    }

    #[test]
    fn configuration_round_trips_through_json() {
        let candidate = configuration();
        let json = serde_json::to_string(&candidate).unwrap();
        assert_eq!(
            serde_json::from_str::<ComponentConfiguration>(&json).unwrap(),
            candidate
        );
    }
}
