//! Commands for the operation registry and its transactions.
use crate::application::AppState;
use crate::error::AppError;
use crate::layers::LayerDocument;
use crate::operations::{
    document_revision, execute, operation_specs, OperationSpec, TransactionRequest,
    TransactionResult,
};
use tauri::State;

/// Runs a list of operations as one transaction: all of them, or none.
///
/// The result is one document. Committing it is one entry in the undo history.
#[tauri::command]
pub async fn apply_transaction(
    request: TransactionRequest,
    state: State<'_, AppState>,
) -> Result<TransactionResult, AppError> {
    // Transactions are serialised with the other layer operations that read or
    // register pixel buffers, so two cannot interleave their journals.
    let _permit = state.layer_gate.lock().await;
    execute(&state, request).await
}

/// What a transaction would do, without doing it: every check and a dry run of every
/// step, saying which steps' conditions hold. Produces nothing and changes nothing.
#[tauri::command]
pub async fn plan_transaction(
    request: TransactionRequest,
    state: State<'_, AppState>,
) -> Result<Vec<crate::operations::StepReport>, AppError> {
    let _permit = state.layer_gate.lock().await;
    crate::operations::plan(&state, &request).await
}

/// Every operation that may edit a document, for the interface to draw its
/// command palette and automation editor from.
#[tauri::command]
pub fn list_operations() -> Vec<OperationSpec> {
    operation_specs()
}

/// The revision of a document, to name in a plan made against it.
#[tauri::command]
pub async fn layer_document_revision(document: LayerDocument) -> Result<String, AppError> {
    document.validate()?;
    document_revision(&document)
}
