use crate::application::AppState;
use crate::components::{validate_ollama_plan, OllamaClient};
use crate::domain::{
    ComponentConfiguration, EditPlanner, ImageQualityAnalysis, OllamaConnectionResult,
    OllamaDiagnostics, OllamaModelDiscoveryResult, OllamaPlanResult, PlanValidationReport,
    PlannerComparisonEntry, PlannerComparisonResult, RulePlanner,
};
use crate::error::AppError;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tauri::State;

#[tauri::command]
pub async fn test_ollama_connection(
    state: State<'_, AppState>,
) -> Result<OllamaConnectionResult, AppError> {
    let configuration = configuration(&state)?;
    let client = client(&configuration)?;
    let started = Instant::now();
    match client.version().await {
        Ok(version) => {
            let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
            update_diagnostics(&state, |diagnostics| {
                diagnostics.connected = true;
                diagnostics.last_error = None;
                diagnostics.last_response_time_ms = Some(elapsed);
                diagnostics.connection_latency_ms = Some(elapsed);
                diagnostics.model_selected = configuration.ollama_selected_model.clone();
            })?;
            Ok(OllamaConnectionResult {
                connected: true,
                message: format!("Connected to local Ollama {version}."),
                version,
                response_time_ms: elapsed,
            })
        }
        Err(error) => {
            record_error(&state, &error)?;
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn refresh_ollama_models(
    state: State<'_, AppState>,
) -> Result<OllamaModelDiscoveryResult, AppError> {
    let configuration = configuration(&state)?;
    let client = client(&configuration)?;
    let started = Instant::now();
    match client.models().await {
        Ok(models) => {
            let elapsed = started.elapsed().as_secs_f64() * 1_000.0;
            update_diagnostics(&state, |diagnostics| {
                diagnostics.connected = true;
                diagnostics.last_error = None;
                diagnostics.last_response_time_ms = Some(elapsed);
                diagnostics.model_selected = configuration.ollama_selected_model.clone();
            })?;
            let message = if models.is_empty() {
                "No compatible local models found.".into()
            } else {
                format!("Found {} installed local model(s).", models.len())
            };
            Ok(OllamaModelDiscoveryResult {
                models,
                message,
                response_time_ms: elapsed.round() as u64,
            })
        }
        Err(error) => {
            record_error(&state, &error)?;
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn generate_ollama_plan(
    request: String,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<OllamaPlanResult, AppError> {
    state
        .latest_plan_request
        .store(request_id, Ordering::Release);
    let _permit = state
        .plan_gate
        .try_lock()
        .map_err(|_| AppError::PlannerBusy)?;
    let (analysis, configuration) = analysis_and_configuration(&state, document_id)?;
    let model = configuration
        .ollama_selected_model
        .clone()
        .filter(|value| !value.trim().is_empty())
        .ok_or(AppError::OllamaModelMissing)?;
    let client = client(&configuration)?;
    let total_started = Instant::now();
    let generation_started = Instant::now();
    let generation = tokio::select! {
        result = client.generate(&model, &request, &analysis, configuration.ollama_max_operations) => result,
        _ = wait_for_cancellation(&state, document_id, request_id) => Err(AppError::PlannerCancellation),
    };
    let generation_time_ms = generation_started.elapsed().as_secs_f64() * 1_000.0;
    let generation = match generation {
        Ok(generation) => generation,
        Err(AppError::PlannerCancellation) => {
            record_cancellation(&state)?;
            return Err(AppError::PlannerCancellation);
        }
        Err(error) => {
            record_error(&state, &error)?;
            return Err(error);
        }
    };
    ensure_current(&state, document_id, request_id)?;
    match validate_ollama_plan(&generation.response, configuration.ollama_max_operations) {
        Ok((plan, report)) => {
            ensure_current(&state, document_id, request_id)?;
            let validation_time_ms = report.validation_time_ms;
            let total_time_ms = total_started.elapsed().as_secs_f64() * 1_000.0;
            update_diagnostics(&state, |diagnostics| {
                diagnostics.connected = true;
                diagnostics.last_error = None;
                diagnostics.last_response_time_ms = Some(total_time_ms);
                diagnostics.generation_latency_ms = Some(generation_time_ms);
                diagnostics.validation_latency_ms = Some(validation_time_ms);
                diagnostics.model_selected = Some(model.clone());
                diagnostics.successful_plans += 1;
            })?;
            Ok(OllamaPlanResult {
                plan: Some(plan),
                document_id,
                request_id,
                model: generation.model,
                generation_time_ms,
                validation_time_ms,
                total_time_ms,
                is_current: true,
                error: None,
                validation_report: report,
            })
        }
        Err(failure) => {
            let total_time_ms = total_started.elapsed().as_secs_f64() * 1_000.0;
            let validation_time_ms = failure.report.validation_time_ms;
            let message = failure.error.to_string();
            update_diagnostics(&state, |diagnostics| {
                diagnostics.connected = true;
                diagnostics.last_error = Some(message.clone());
                diagnostics.last_response_time_ms = Some(total_time_ms);
                diagnostics.generation_latency_ms = Some(generation_time_ms);
                diagnostics.validation_latency_ms = Some(validation_time_ms);
                diagnostics.model_selected = Some(model.clone());
                diagnostics.validation_failures += 1;
                diagnostics.rejected_plans += 1;
            })?;
            Ok(OllamaPlanResult {
                plan: None,
                document_id,
                request_id,
                model: generation.model,
                generation_time_ms,
                validation_time_ms,
                total_time_ms,
                is_current: true,
                error: Some(message),
                validation_report: *failure.report,
            })
        }
    }
}

#[tauri::command]
pub fn cancel_ollama_plan(request_id: u64, state: State<'_, AppState>) -> Result<(), AppError> {
    let current = state.latest_plan_request.load(Ordering::Acquire);
    if current == request_id {
        state
            .latest_plan_request
            .store(request_id.saturating_add(1), Ordering::Release);
    }
    Ok(())
}

#[tauri::command]
pub fn validate_ollama_json(raw_json: String, maximum_operations: usize) -> PlanValidationReport {
    match validate_ollama_plan(&raw_json, maximum_operations) {
        Ok((_, report)) => report,
        Err(failure) => *failure.report,
    }
}

#[tauri::command]
pub async fn compare_planners(
    request: String,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<PlannerComparisonResult, AppError> {
    state
        .latest_plan_request
        .store(request_id, Ordering::Release);
    let _permit = state
        .plan_gate
        .try_lock()
        .map_err(|_| AppError::PlannerBusy)?;
    let (analysis, configuration) = analysis_and_configuration(&state, document_id)?;
    let total_started = Instant::now();
    let rule_started = Instant::now();
    let rule_result = RulePlanner.create_plan(&request, &analysis);
    let rule_time = rule_started.elapsed().as_secs_f64() * 1_000.0;
    let rule = match rule_result {
        Ok(plan) => PlannerComparisonEntry {
            provider: "Rule".into(),
            plan: Some(plan),
            execution_time_ms: rule_time,
            error: None,
        },
        Err(error) => PlannerComparisonEntry {
            provider: "Rule".into(),
            plan: None,
            execution_time_ms: rule_time,
            error: Some(error.to_string()),
        },
    };
    let model = configuration
        .ollama_selected_model
        .clone()
        .filter(|value| !value.trim().is_empty());
    let ollama_started = Instant::now();
    let (ollama, validation_report) = if let Some(model) = model {
        let client = client(&configuration)?;
        let generation = tokio::select! {
            result = client.generate(&model, &request, &analysis, configuration.ollama_max_operations) => result,
            _ = wait_for_cancellation(&state, document_id, request_id) => Err(AppError::PlannerCancellation),
        };
        match generation {
            Ok(generation) => match validate_ollama_plan(
                &generation.response,
                configuration.ollama_max_operations,
            ) {
                Ok((plan, report)) => (
                    PlannerComparisonEntry {
                        provider: "Ollama".into(),
                        plan: Some(plan),
                        execution_time_ms: ollama_started.elapsed().as_secs_f64() * 1_000.0,
                        error: None,
                    },
                    Some(report),
                ),
                Err(failure) => (
                    {
                        let message = failure.error.to_string();
                        update_diagnostics(&state, |diagnostics| {
                            diagnostics.connected = true;
                            diagnostics.last_error = Some(message.clone());
                            diagnostics.validation_failures += 1;
                            diagnostics.rejected_plans += 1;
                        })?;
                        PlannerComparisonEntry {
                            provider: "Ollama".into(),
                            plan: None,
                            execution_time_ms: ollama_started.elapsed().as_secs_f64() * 1_000.0,
                            error: Some(message),
                        }
                    },
                    Some(*failure.report),
                ),
            },
            Err(AppError::PlannerCancellation) => {
                record_cancellation(&state)?;
                return Err(AppError::PlannerCancellation);
            }
            Err(error) => {
                record_error(&state, &error)?;
                (
                    PlannerComparisonEntry {
                        provider: "Ollama".into(),
                        plan: None,
                        execution_time_ms: ollama_started.elapsed().as_secs_f64() * 1_000.0,
                        error: Some(error.to_string()),
                    },
                    None,
                )
            }
        }
    } else {
        update_diagnostics(&state, |diagnostics| {
            diagnostics.last_error = Some(AppError::OllamaModelMissing.to_string());
        })?;
        (
            PlannerComparisonEntry {
                provider: "Ollama".into(),
                plan: None,
                execution_time_ms: 0.0,
                error: Some(AppError::OllamaModelMissing.to_string()),
            },
            None,
        )
    };
    ensure_current(&state, document_id, request_id)?;
    let total_time_ms = total_started.elapsed().as_secs_f64() * 1_000.0;
    update_diagnostics(&state, |diagnostics| {
        diagnostics.rule_planner_latency_ms = Some(rule_time);
        diagnostics.comparison_latency_ms = Some(total_time_ms);
        diagnostics.generation_latency_ms = Some(ollama.execution_time_ms);
    })?;
    Ok(PlannerComparisonResult {
        rule,
        ollama,
        validation_report,
        total_time_ms,
    })
}

#[tauri::command]
pub fn get_ollama_diagnostics(state: State<'_, AppState>) -> Result<OllamaDiagnostics, AppError> {
    state
        .ollama_diagnostics
        .lock()
        .map(|diagnostics| diagnostics.clone())
        .map_err(|_| AppError::ComponentInitializationFailure("diagnostics are unavailable".into()))
}

fn configuration(state: &AppState) -> Result<ComponentConfiguration, AppError> {
    state
        .components
        .lock()
        .map(|registry| registry.configuration().clone())
        .map_err(|_| AppError::ComponentInitializationFailure("registry is unavailable".into()))
}

fn analysis_and_configuration(
    state: &AppState,
    document_id: u64,
) -> Result<(ImageQualityAnalysis, ComponentConfiguration), AppError> {
    let analysis = state
        .session
        .lock()
        .map_err(|_| AppError::AnalysisUnavailable)?
        .as_ref()
        .filter(|session| session.document_id == document_id)
        .and_then(|session| session.analysis.clone())
        .ok_or(AppError::AnalysisUnavailable)?;
    Ok((analysis, configuration(state)?))
}

fn client(configuration: &ComponentConfiguration) -> Result<OllamaClient, AppError> {
    OllamaClient::new(
        &configuration.planner_endpoint,
        configuration.ollama_timeout_ms,
        configuration.ollama_max_response_bytes,
    )
}

async fn wait_for_cancellation(state: &AppState, document_id: u64, request_id: u64) {
    loop {
        if state.latest_plan_request.load(Ordering::Acquire) != request_id
            || state.pending_open_request.load(Ordering::Acquire) != 0
            || state
                .session
                .lock()
                .ok()
                .and_then(|session| session.as_ref().map(|session| session.document_id))
                != Some(document_id)
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn ensure_current(state: &AppState, document_id: u64, request_id: u64) -> Result<(), AppError> {
    let current = state.latest_plan_request.load(Ordering::Acquire) == request_id
        && state.pending_open_request.load(Ordering::Acquire) == 0
        && state
            .session
            .lock()
            .ok()
            .and_then(|session| session.as_ref().map(|session| session.document_id))
            == Some(document_id);
    if current {
        Ok(())
    } else {
        record_cancellation(state)?;
        Err(AppError::PlannerCancellation)
    }
}

fn update_diagnostics(
    state: &AppState,
    update: impl FnOnce(&mut OllamaDiagnostics),
) -> Result<(), AppError> {
    let mut diagnostics = state.ollama_diagnostics.lock().map_err(|_| {
        AppError::ComponentInitializationFailure("diagnostics are unavailable".into())
    })?;
    update(&mut diagnostics);
    Ok(())
}

fn record_error(state: &AppState, error: &AppError) -> Result<(), AppError> {
    update_diagnostics(state, |diagnostics| {
        diagnostics.connected = matches!(
            error,
            AppError::OllamaJsonParse
                | AppError::OllamaHttpStatus(_)
                | AppError::OllamaSchemaValidation(_)
                | AppError::OllamaResponseTooLarge { .. }
                | AppError::UnsupportedPlannerVersion
        );
        diagnostics.last_error = Some(error.to_string());
    })
}

fn record_cancellation(state: &AppState) -> Result<(), AppError> {
    update_diagnostics(state, |diagnostics| {
        diagnostics.last_error = Some(AppError::PlannerCancellation.to_string());
        diagnostics.cancelled_plans += 1;
    })
}
