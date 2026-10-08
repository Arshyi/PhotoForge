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

/// Reads an exported macro. The text is returned for the interface to check and keep;
/// nothing in it is run.
#[tauri::command]
pub fn import_macro(path: String) -> Result<String, AppError> {
    // The same guard every other path the interface hands over goes through: a network
    // share, a device name or a stream is refused before anything is opened.
    let checked = crate::infrastructure::local_path::validated_local_path(&path)?;
    crate::infrastructure::read_macro_file(&checked)
}

/// Writes a macro the interface has turned into text. Only a macro document is
/// written, to a `.json` file in a folder that exists.
#[tauri::command]
pub fn export_macro(path: String, text: String) -> Result<String, AppError> {
    let checked = crate::infrastructure::local_path::validated_local_path(&path)?;
    crate::infrastructure::write_macro_file(&checked, &text)
        .map(|saved| saved.to_string_lossy().into_owned())
}
