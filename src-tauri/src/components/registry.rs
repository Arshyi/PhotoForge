use crate::components::{PlannerFactory, RestorationEngineFactory};
use crate::domain::{
    ComponentConfiguration, ComponentDiagnostics, ComponentSnapshot, EngineProvider,
    EngineRegistration, PlannerProvider, PlannerRegistration,
};
use crate::error::AppError;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

const NOT_INSTALLED: &str = "Component not installed.";
const MAX_CONFIGURATION_BYTES: u64 = 32 * 1024;

pub struct ComponentRegistry {
    configuration: ComponentConfiguration,
    configuration_path: PathBuf,
    loaded_components: HashSet<String>,
    initialization_failures: Vec<String>,
    plugin_validation_errors: Vec<String>,
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("PhotoForge");
        let configuration = ComponentConfiguration {
            active_planner: PlannerProvider::Rule,
            active_engine: EngineProvider::Deterministic,
            planner_endpoint: "http://127.0.0.1:11434".into(),
            initialization_timeout_ms: 5_000,
            ollama_timeout_ms: 15_000,
            ollama_max_response_bytes: 256 * 1_024,
            ollama_selected_model: None,
            ollama_max_operations: 8,
            model_directories: vec![base.join("models").to_string_lossy().into_owned()],
            plugin_directory: base.join("plugins").to_string_lossy().into_owned(),
        };
        let loaded_components = HashSet::from([
            PlannerProvider::Rule.id().to_string(),
            EngineProvider::Deterministic.id().to_string(),
        ]);
        Self {
            configuration,
            configuration_path: base.join("components.json"),
            loaded_components,
            initialization_failures: Vec::new(),
            plugin_validation_errors: Vec::new(),
        }
    }
}

impl ComponentRegistry {
    pub fn load_persisted_configuration(&mut self) {
        if !self.configuration_path.is_file() {
            return;
        }
        let result = (|| {
            let metadata = fs::metadata(&self.configuration_path).map_err(|error| {
                AppError::InvalidComponentConfiguration(format!(
                    "could not inspect saved settings: {error}"
                ))
            })?;
            if metadata.len() > MAX_CONFIGURATION_BYTES {
                return Err(AppError::InvalidComponentConfiguration(
                    "saved component settings exceed 32 KiB".into(),
                ));
            }
            let json = fs::read_to_string(&self.configuration_path).map_err(|error| {
                AppError::InvalidComponentConfiguration(format!(
                    "could not read saved settings: {error}"
                ))
            })?;
            let configuration: ComponentConfiguration =
                serde_json::from_str(&json).map_err(|error| {
                    AppError::InvalidComponentConfiguration(format!(
                        "could not parse saved settings: {error}"
                    ))
                })?;
            self.update_configuration(configuration)
        })();
        if let Err(error) = result {
            self.record_initialization_failure(error.to_string());
        }
    }

    pub fn persist_configuration(&self) -> Result<(), AppError> {
        self.configuration.validate()?;
        let parent = self.configuration_path.parent().ok_or_else(|| {
            AppError::InvalidComponentConfiguration(
                "component settings path has no parent directory".into(),
            )
        })?;
        fs::create_dir_all(parent).map_err(|error| {
            AppError::InvalidComponentConfiguration(format!(
                "could not create the component settings directory: {error}"
            ))
        })?;
        let json = serde_json::to_string_pretty(&self.configuration).map_err(|error| {
            AppError::InvalidComponentConfiguration(format!(
                "could not serialize component settings: {error}"
            ))
        })?;
        fs::write(&self.configuration_path, json).map_err(|error| {
            AppError::InvalidComponentConfiguration(format!(
                "could not save component settings: {error}"
            ))
        })
    }

    pub fn active_planner(&self) -> PlannerProvider {
        self.configuration.active_planner
    }

    pub fn active_engine(&self) -> EngineProvider {
        self.configuration.active_engine
    }

    pub fn initialization_timeout_ms(&self) -> u64 {
        self.configuration.initialization_timeout_ms
    }

    pub fn configuration(&self) -> &ComponentConfiguration {
        &self.configuration
    }

    pub fn select_planner(&mut self, provider: PlannerProvider) -> Result<(), AppError> {
        if !matches!(provider, PlannerProvider::Rule | PlannerProvider::Ollama) {
            return Err(AppError::PlannerNotInstalled);
        }
        let planner = PlannerFactory::create(provider);
        if planner.provider() != provider {
            return Err(AppError::ComponentInitializationFailure(
                "planner factory returned the wrong provider".into(),
            ));
        }
        self.configuration.active_planner = provider;
        self.loaded_components.insert(provider.id().into());
        self.unload_inactive_optional_components();
        Ok(())
    }

    pub fn select_engine(&mut self, provider: EngineProvider) -> Result<(), AppError> {
        if provider != EngineProvider::Deterministic {
            return Err(AppError::RestorationEngineNotInstalled);
        }
        let engine = RestorationEngineFactory::create(provider);
        if engine.provider() != provider {
            return Err(AppError::ComponentInitializationFailure(
                "restoration factory returned the wrong provider".into(),
            ));
        }
        self.configuration.active_engine = provider;
        self.loaded_components.insert(provider.id().into());
        self.unload_inactive_optional_components();
        Ok(())
    }

    pub fn update_configuration(
        &mut self,
        configuration: ComponentConfiguration,
    ) -> Result<(), AppError> {
        configuration.validate()?;
        if !matches!(
            configuration.active_planner,
            PlannerProvider::Rule | PlannerProvider::Ollama
        ) {
            return Err(AppError::PlannerNotInstalled);
        }
        if configuration.active_engine != EngineProvider::Deterministic {
            return Err(AppError::RestorationEngineNotInstalled);
        }
        self.configuration = configuration;
        self.loaded_components
            .insert(PlannerProvider::Rule.id().into());
        self.loaded_components
            .insert(EngineProvider::Deterministic.id().into());
        self.unload_inactive_optional_components();
        Ok(())
    }

    pub fn record_initialization_failure(&mut self, message: impl Into<String>) {
        let message = message.into();
        if !message.trim().is_empty() && !self.initialization_failures.contains(&message) {
            self.initialization_failures.push(message);
            self.initialization_failures.truncate(32);
        }
    }

    pub fn set_plugin_validation_errors(&mut self, errors: Vec<String>) {
        self.plugin_validation_errors = errors.into_iter().take(64).collect();
    }

    pub fn snapshot(&self) -> ComponentSnapshot {
        ComponentSnapshot {
            application_version: env!("CARGO_PKG_VERSION").into(),
            planners: PlannerProvider::ALL
                .into_iter()
                .map(|provider| self.planner_registration(provider))
                .collect(),
            engines: EngineProvider::ALL
                .into_iter()
                .map(|provider| self.engine_registration(provider))
                .collect(),
            configuration: self.configuration.clone(),
        }
    }

    pub fn diagnostics(&self) -> ComponentDiagnostics {
        let snapshot = self.snapshot();
        ComponentDiagnostics {
            application_version: snapshot.application_version,
            registered_planners: snapshot
                .planners
                .iter()
                .map(|component| component.name.clone())
                .collect(),
            registered_engines: snapshot
                .engines
                .iter()
                .map(|component| component.name.clone())
                .collect(),
            loaded_components: snapshot
                .planners
                .iter()
                .filter(|component| component.loaded)
                .map(|component| component.name.clone())
                .chain(
                    snapshot
                        .engines
                        .iter()
                        .filter(|component| component.loaded)
                        .map(|component| component.name.clone()),
                )
                .collect(),
            unavailable_components: snapshot
                .planners
                .iter()
                .filter(|component| !component.installed)
                .map(|component| component.name.clone())
                .chain(
                    snapshot
                        .engines
                        .iter()
                        .filter(|component| !component.installed)
                        .map(|component| component.name.clone()),
                )
                .collect(),
            initialization_failures: self.initialization_failures.clone(),
            plugin_validation_errors: self.plugin_validation_errors.clone(),
            configuration_path: self.configuration_path.to_string_lossy().into_owned(),
        }
    }

    pub fn planner_registration(&self, provider: PlannerProvider) -> PlannerRegistration {
        let implementation = PlannerFactory::create(provider);
        let installed = matches!(provider, PlannerProvider::Rule | PlannerProvider::Ollama);
        PlannerRegistration {
            id: provider.id().into(),
            name: provider.display_name().into(),
            version: if installed {
                env!("CARGO_PKG_VERSION").into()
            } else {
                "Not installed".into()
            },
            provider: match provider {
                PlannerProvider::Rule => "PhotoForge",
                PlannerProvider::Ollama => "Ollama local API",
                PlannerProvider::OpenAi => "OpenAI (future adapter)",
                PlannerProvider::Future => "Unassigned",
            }
            .into(),
            memory_estimate_mb: match provider {
                PlannerProvider::Rule => 1,
                PlannerProvider::Ollama => 1,
                PlannerProvider::OpenAi => 32,
                PlannerProvider::Future => 0,
            },
            installed,
            loaded: self.loaded_components.contains(provider.id()),
            active: self.configuration.active_planner == provider,
            unavailable_reason: (!installed).then(|| NOT_INSTALLED.into()),
            capabilities: implementation.capabilities(),
        }
    }

    pub fn engine_registration(&self, provider: EngineProvider) -> EngineRegistration {
        let implementation = RestorationEngineFactory::create(provider);
        let installed = provider == EngineProvider::Deterministic;
        EngineRegistration {
            id: provider.id().into(),
            name: provider.display_name().into(),
            version: if installed {
                env!("CARGO_PKG_VERSION").into()
            } else {
                "Not installed".into()
            },
            provider: match provider {
                EngineProvider::Deterministic => "PhotoForge",
                EngineProvider::Onnx => "ONNX (future adapter)",
                EngineProvider::RealEsrgan => "Real-ESRGAN (future adapter)",
                EngineProvider::Future => "Unassigned",
            }
            .into(),
            memory_estimate_mb: match provider {
                EngineProvider::Deterministic => 4,
                EngineProvider::Onnx => 2_048,
                EngineProvider::RealEsrgan => 4_096,
                EngineProvider::Future => 0,
            },
            installed,
            loaded: self.loaded_components.contains(provider.id()),
            active: self.configuration.active_engine == provider,
            unavailable_reason: (!installed).then(|| NOT_INSTALLED.into()),
            capabilities: implementation.capabilities(),
        }
    }

    fn unload_inactive_optional_components(&mut self) {
        self.loaded_components.retain(|component| {
            component == PlannerProvider::Rule.id()
                || component == EngineProvider::Deterministic.id()
                || component == self.configuration.active_planner.id()
                || component == self.configuration.active_engine.id()
        });
    }

    #[cfg(test)]
    fn set_configuration_path(&mut self, path: PathBuf) {
        self.configuration_path = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_activates_only_built_in_components() {
        let registry = ComponentRegistry::default();
        assert_eq!(registry.active_planner(), PlannerProvider::Rule);
        assert_eq!(registry.active_engine(), EngineProvider::Deterministic);
        let snapshot = registry.snapshot();
        assert_eq!(
            snapshot.planners.iter().filter(|item| item.active).count(),
            1
        );
        assert_eq!(
            snapshot.engines.iter().filter(|item| item.active).count(),
            1
        );
    }

    #[test]
    fn registry_reports_all_placeholder_components() {
        let snapshot = ComponentRegistry::default().snapshot();
        assert_eq!(snapshot.planners.len(), 4);
        assert_eq!(snapshot.engines.len(), 4);
        assert_eq!(
            snapshot
                .planners
                .iter()
                .filter(|item| !item.installed)
                .count(),
            2
        );
        assert_eq!(
            snapshot
                .engines
                .iter()
                .filter(|item| !item.installed)
                .count(),
            3
        );
    }

    #[test]
    fn unavailable_components_are_visible_with_reasons() {
        let snapshot = ComponentRegistry::default().snapshot();
        assert!(snapshot
            .planners
            .iter()
            .filter(|item| !item.installed)
            .all(|item| item.unavailable_reason.as_deref() == Some(NOT_INSTALLED)));
        assert!(snapshot
            .engines
            .iter()
            .filter(|item| !item.installed)
            .all(|item| item.unavailable_reason.as_deref() == Some(NOT_INSTALLED)));
    }

    #[test]
    fn selecting_rule_planner_is_idempotent() {
        let mut registry = ComponentRegistry::default();
        registry.select_planner(PlannerProvider::Rule).unwrap();
        registry.select_planner(PlannerProvider::Rule).unwrap();
        assert_eq!(registry.active_planner(), PlannerProvider::Rule);
    }

    #[test]
    fn selecting_deterministic_engine_is_idempotent() {
        let mut registry = ComponentRegistry::default();
        registry
            .select_engine(EngineProvider::Deterministic)
            .unwrap();
        registry
            .select_engine(EngineProvider::Deterministic)
            .unwrap();
        assert_eq!(registry.active_engine(), EngineProvider::Deterministic);
    }

    #[test]
    fn selecting_ollama_adapter_succeeds_without_connecting() {
        let mut registry = ComponentRegistry::default();
        registry.select_planner(PlannerProvider::Ollama).unwrap();
        assert_eq!(registry.active_planner(), PlannerProvider::Ollama);
        assert!(
            registry
                .planner_registration(PlannerProvider::Ollama)
                .loaded
        );
    }

    #[test]
    fn selecting_unavailable_engine_fails_without_switching() {
        let mut registry = ComponentRegistry::default();
        assert!(matches!(
            registry.select_engine(EngineProvider::Onnx),
            Err(AppError::RestorationEngineNotInstalled)
        ));
        assert_eq!(registry.active_engine(), EngineProvider::Deterministic);
    }

    #[test]
    fn valid_configuration_can_be_updated() {
        let mut registry = ComponentRegistry::default();
        let mut configuration = registry.configuration().clone();
        configuration.initialization_timeout_ms = 2_000;
        configuration.model_directories = vec!["D:\\Models".into()];
        registry
            .update_configuration(configuration.clone())
            .unwrap();
        assert_eq!(registry.configuration(), &configuration);
    }

    #[test]
    fn unavailable_active_provider_is_rejected_in_configuration() {
        let mut registry = ComponentRegistry::default();
        let mut configuration = registry.configuration().clone();
        configuration.active_planner = PlannerProvider::OpenAi;
        assert!(matches!(
            registry.update_configuration(configuration),
            Err(AppError::PlannerNotInstalled)
        ));
    }

    #[test]
    fn diagnostics_report_registered_loaded_and_unavailable_components() {
        let diagnostics = ComponentRegistry::default().diagnostics();
        assert_eq!(diagnostics.registered_planners.len(), 4);
        assert_eq!(diagnostics.registered_engines.len(), 4);
        assert_eq!(diagnostics.loaded_components.len(), 2);
        assert_eq!(diagnostics.unavailable_components.len(), 5);
        assert!(diagnostics.configuration_path.ends_with("components.json"));
    }

    #[test]
    fn initialization_failures_are_deduplicated() {
        let mut registry = ComponentRegistry::default();
        registry.record_initialization_failure("Planner not installed.");
        registry.record_initialization_failure("Planner not installed.");
        assert_eq!(registry.diagnostics().initialization_failures.len(), 1);
    }

    #[test]
    fn plugin_validation_errors_replace_previous_scan() {
        let mut registry = ComponentRegistry::default();
        registry.set_plugin_validation_errors(vec!["first".into(), "second".into()]);
        assert_eq!(registry.diagnostics().plugin_validation_errors.len(), 2);
        registry.set_plugin_validation_errors(Vec::new());
        assert!(registry.diagnostics().plugin_validation_errors.is_empty());
    }

    #[test]
    fn capabilities_are_derived_from_factory_implementations() {
        let registry = ComponentRegistry::default();
        assert!(
            registry
                .planner_registration(PlannerProvider::Rule)
                .capabilities
                .offline
        );
        assert!(
            registry
                .engine_registration(EngineProvider::Deterministic)
                .capabilities
                .preserves_alpha
        );
        assert!(
            registry
                .planner_registration(PlannerProvider::Ollama)
                .capabilities
                .requires_model
        );
    }

    #[test]
    fn default_configuration_contains_only_local_paths_and_endpoint() {
        let registry = ComponentRegistry::default();
        let configuration = registry.configuration();
        assert_eq!(configuration.planner_endpoint, "http://127.0.0.1:11434");
        assert_eq!(configuration.model_directories.len(), 1);
        assert!(configuration.plugin_directory.ends_with("plugins"));
        assert!(configuration.validate().is_ok());
    }

    #[test]
    fn configuration_persists_and_loads_without_network_or_component_initialization() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("components.json");
        let mut registry = ComponentRegistry::default();
        registry.set_configuration_path(path.clone());
        let mut configuration = registry.configuration().clone();
        configuration.initialization_timeout_ms = 2_500;
        configuration.model_directories = vec!["D:\\Local Models".into()];
        registry
            .update_configuration(configuration.clone())
            .unwrap();
        registry.persist_configuration().unwrap();

        let mut restored = ComponentRegistry::default();
        restored.set_configuration_path(path);
        restored.load_persisted_configuration();
        assert_eq!(restored.configuration(), &configuration);
        assert!(restored.diagnostics().initialization_failures.is_empty());
    }

    #[test]
    fn invalid_persisted_configuration_falls_back_to_safe_defaults() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("components.json");
        fs::write(&path, r#"{"activePlanner":"ollama"}"#).unwrap();
        let mut registry = ComponentRegistry::default();
        registry.set_configuration_path(path);
        registry.load_persisted_configuration();
        assert_eq!(registry.active_planner(), PlannerProvider::Rule);
        assert_eq!(registry.active_engine(), EngineProvider::Deterministic);
        assert_eq!(registry.diagnostics().initialization_failures.len(), 1);
    }
}
