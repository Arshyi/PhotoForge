use crate::application::AppState;
use crate::components::{
    initialize_with_timeout, measure_component_overhead, PlannerFactory, RestorationEngineFactory,
};
use crate::domain::{
    ComponentConfiguration, ComponentDiagnostics, ComponentPerformanceMetrics, ComponentSnapshot,
    EngineProvider, ModelDiscoveryResult, PlannerProvider, PluginManifest, PluginScanResult,
};
use crate::error::AppError;
use crate::infrastructure::{discover_local_models, scan_plugin_manifests};
use std::path::PathBuf;
use std::str::FromStr;
use tauri::State;

#[tauri::command]
pub fn get_component_snapshot(state: State<'_, AppState>) -> Result<ComponentSnapshot, AppError> {
    state
        .components
        .lock()
        .map_err(|_| AppError::ComponentInitializationFailure("registry is unavailable".into()))
        .map(|registry| registry.snapshot())
}

#[tauri::command]
pub fn get_component_diagnostics(
    state: State<'_, AppState>,
) -> Result<ComponentDiagnostics, AppError> {
    state
        .components
        .lock()
        .map_err(|_| AppError::ComponentInitializationFailure("registry is unavailable".into()))
        .map(|registry| registry.diagnostics())
}

#[tauri::command]
pub fn measure_component_performance(
    samples: u32,
    state: State<'_, AppState>,
) -> Result<ComponentPerformanceMetrics, AppError> {
    state
        .components
        .lock()
        .map_err(|_| AppError::ComponentInitializationFailure("registry is unavailable".into()))
        .map(|registry| measure_component_overhead(&registry, samples))
}

#[tauri::command]
pub async fn select_planner_provider(
    provider: String,
    state: State<'_, AppState>,
) -> Result<ComponentSnapshot, AppError> {
    let provider = PlannerProvider::from_str(&provider)?;
    let timeout_ms = state
        .components
        .lock()
        .map_err(|_| AppError::ComponentInitializationFailure("registry is unavailable".into()))?
        .initialization_timeout_ms();
    let result = initialize_with_timeout(timeout_ms, async move {
        if !matches!(provider, PlannerProvider::Rule | PlannerProvider::Ollama) {
            return Err(AppError::PlannerNotInstalled);
        }
        let planner = PlannerFactory::create(provider);
        if planner.provider() != provider {
            return Err(AppError::ComponentInitializationFailure(
                "planner factory returned the wrong provider".into(),
            ));
        }
        Ok(provider)
    })
    .await;
    let mut registry = state
        .components
        .lock()
        .map_err(|_| AppError::ComponentInitializationFailure("registry is unavailable".into()))?;
    match result {
        Ok(provider) => {
            registry.select_planner(provider)?;
            registry.persist_configuration()?;
            Ok(registry.snapshot())
        }
        Err(error) => {
            registry.record_initialization_failure(error.to_string());
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn select_restoration_engine(
    provider: String,
    state: State<'_, AppState>,
) -> Result<ComponentSnapshot, AppError> {
    let provider = EngineProvider::from_str(&provider)?;
    let timeout_ms = state
        .components
        .lock()
        .map_err(|_| AppError::ComponentInitializationFailure("registry is unavailable".into()))?
        .initialization_timeout_ms();
    let result = initialize_with_timeout(timeout_ms, async move {
        if provider != EngineProvider::Deterministic {
            return Err(AppError::RestorationEngineNotInstalled);
        }
        let engine = RestorationEngineFactory::create(provider);
        if engine.provider() != provider {
            return Err(AppError::ComponentInitializationFailure(
                "restoration factory returned the wrong provider".into(),
            ));
        }
        Ok(provider)
    })
    .await;
    let mut registry = state
        .components
        .lock()
        .map_err(|_| AppError::ComponentInitializationFailure("registry is unavailable".into()))?;
    match result {
        Ok(provider) => {
            registry.select_engine(provider)?;
            registry.persist_configuration()?;
            Ok(registry.snapshot())
        }
        Err(error) => {
            registry.record_initialization_failure(error.to_string());
            Err(error)
        }
    }
}

#[tauri::command]
pub fn update_component_configuration(
    configuration: ComponentConfiguration,
    state: State<'_, AppState>,
) -> Result<ComponentSnapshot, AppError> {
    let mut registry = state
        .components
        .lock()
        .map_err(|_| AppError::ComponentInitializationFailure("registry is unavailable".into()))?;
    registry.update_configuration(configuration)?;
    registry.persist_configuration()?;
    Ok(registry.snapshot())
}

#[tauri::command]
pub async fn discover_models(state: State<'_, AppState>) -> Result<ModelDiscoveryResult, AppError> {
    let (directories, timeout_ms) = {
        let registry = state.components.lock().map_err(|_| {
            AppError::ComponentInitializationFailure("registry is unavailable".into())
        })?;
        (
            registry.configuration().model_directories.clone(),
            registry.initialization_timeout_ms(),
        )
    };
    initialize_with_timeout(timeout_ms, async move {
        tauri::async_runtime::spawn_blocking(move || discover_local_models(&directories))
            .await
            .map_err(|_| AppError::ModelDiscoveryFailure("discovery worker stopped".into()))?
    })
    .await
}

#[tauri::command]
pub async fn scan_plugins(state: State<'_, AppState>) -> Result<PluginScanResult, AppError> {
    let (directory, timeout_ms) = {
        let registry = state.components.lock().map_err(|_| {
            AppError::ComponentInitializationFailure("registry is unavailable".into())
        })?;
        (
            PathBuf::from(&registry.configuration().plugin_directory),
            registry.initialization_timeout_ms(),
        )
    };
    let result = initialize_with_timeout(timeout_ms, async move {
        tauri::async_runtime::spawn_blocking(move || scan_plugin_manifests(&directory))
            .await
            .map_err(|_| AppError::InvalidPluginManifest("plugin scan worker stopped".into()))?
    })
    .await?;
    let errors = result
        .records
        .iter()
        .filter_map(|record| record.error.clone())
        .collect();
    state
        .components
        .lock()
        .map_err(|_| AppError::ComponentInitializationFailure("registry is unavailable".into()))?
        .set_plugin_validation_errors(errors);
    Ok(result)
}

#[tauri::command]
pub fn validate_plugin_manifest(json: String) -> Result<PluginManifest, AppError> {
    PluginManifest::from_json(&json)
}
