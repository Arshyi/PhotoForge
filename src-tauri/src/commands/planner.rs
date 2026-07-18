use crate::application::AppState;
use crate::domain::{validate_edit_plan, EditPlan, EditPlanner, PlanResult, RuleBasedPlanner};
use crate::error::AppError;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tauri::State;

fn stale_plan(document_id: u64, request_id: u64) -> PlanResult {
    PlanResult {
        plan: None,
        document_id,
        request_id,
        processing_time_ms: 0.0,
        is_current: false,
    }
}

#[tauri::command]
pub async fn generate_edit_plan(
    request: String,
    document_id: u64,
    request_id: u64,
    state: State<'_, AppState>,
) -> Result<PlanResult, AppError> {
    state
        .latest_plan_request
        .store(request_id, Ordering::Release);
    let _plan_permit = state.plan_gate.lock().await;

    if state.latest_plan_request.load(Ordering::Acquire) != request_id
        || state.pending_open_request.load(Ordering::Acquire) != 0
    {
        return Ok(stale_plan(document_id, request_id));
    }

    let analysis = {
        let session = state
            .session
            .lock()
            .map_err(|_| AppError::AnalysisUnavailable)?;
        let session = session.as_ref().ok_or(AppError::NoImageOpen)?;
        if session.document_id != document_id {
            return Ok(stale_plan(document_id, request_id));
        }
        session
            .analysis
            .clone()
            .ok_or(AppError::AnalysisUnavailable)?
    };

    let started = Instant::now();
    let plan = RuleBasedPlanner.plan(&request, &analysis)?;
    let is_current = state.latest_plan_request.load(Ordering::Acquire) == request_id
        && state.pending_open_request.load(Ordering::Acquire) == 0
        && state
            .session
            .lock()
            .map_err(|_| AppError::AnalysisUnavailable)?
            .as_ref()
            .is_some_and(|session| session.document_id == document_id);

    if !is_current {
        return Ok(stale_plan(document_id, request_id));
    }

    Ok(PlanResult {
        plan: Some(plan),
        document_id,
        request_id,
        processing_time_ms: started.elapsed().as_secs_f64() * 1_000.0,
        is_current: true,
    })
}

#[tauri::command]
pub fn validate_guided_plan(plan: EditPlan) -> Result<EditPlan, AppError> {
    validate_edit_plan(&plan)?;
    Ok(plan)
}
