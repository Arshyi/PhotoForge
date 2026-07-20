use super::EditOperation;
use crate::error::AppError;
use serde::{Deserialize, Serialize};

pub const WORKFLOW_SCHEMA_VERSION: u32 = 1;
pub const MAX_WORKFLOW_OPERATIONS: usize = 200;
pub const MAX_BATCH_FILES: usize = 10_000;
pub const MAX_BATCH_WORKERS: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistogramChannels {
    pub red: Vec<u64>,
    pub green: Vec<u64>,
    pub blue: Vec<u64>,
    pub luminance: Vec<u64>,
    pub shadow_clipping: u64,
    pub highlight_clipping: u64,
    pub pixel_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistogramResult {
    pub before: HistogramChannels,
    pub after: HistogramChannels,
    pub document_id: u64,
    pub request_id: u64,
    pub processing_time_ms: f64,
    pub is_current: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PixelInspection {
    pub x: u32,
    pub y: u32,
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
    pub hue: f32,
    pub saturation: f32,
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workflow {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub favorite: bool,
    pub operations: Vec<EditOperation>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

impl Workflow {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.id.trim().is_empty() || self.id.len() > 128 {
            return Err(AppError::WorkflowValidation(
                "id must contain 1 to 128 characters".into(),
            ));
        }
        if self.name.trim().is_empty() || self.name.len() > 120 {
            return Err(AppError::WorkflowValidation(
                "name must contain 1 to 120 characters".into(),
            ));
        }
        if self.folder.len() > 120 || self.description.len() > 1_000 {
            return Err(AppError::WorkflowValidation(
                "folder or description exceeds its local storage limit".into(),
            ));
        }
        if self.operations.is_empty() || self.operations.len() > MAX_WORKFLOW_OPERATIONS {
            return Err(AppError::WorkflowValidation(format!(
                "workflows require 1 to {MAX_WORKFLOW_OPERATIONS} operations"
            )));
        }
        for operation in &self.operations {
            operation.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowDocument {
    pub schema_version: u32,
    pub workflow: Workflow,
}

impl WorkflowDocument {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.schema_version != WORKFLOW_SCHEMA_VERSION {
            return Err(AppError::WorkflowValidation(format!(
                "unsupported workflow schema version {}; expected {}",
                self.schema_version, WORKFLOW_SCHEMA_VERSION
            )));
        }
        self.workflow.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ExportProfile {
    Web,
    Print,
    Archive,
    #[default]
    Lossless,
    HighJpeg,
    MaximumCompression,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchOptions {
    pub input_folder: String,
    pub output_folder: String,
    pub filename_template: String,
    pub recursive: bool,
    pub overwrite: bool,
    pub workers: usize,
    pub export_profile: ExportProfile,
    #[serde(default)]
    pub dry_run: bool,
}

impl BatchOptions {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.input_folder.trim().is_empty() || self.output_folder.trim().is_empty() {
            return Err(AppError::BatchFailure(
                "input and output folders are required".into(),
            ));
        }
        if !(1..=MAX_BATCH_WORKERS).contains(&self.workers) {
            return Err(AppError::BatchFailure(format!(
                "worker count must be between 1 and {MAX_BATCH_WORKERS}"
            )));
        }
        if self.filename_template.trim().is_empty()
            || self.filename_template.len() > 180
            || self.filename_template.contains(['/', '\\'])
        {
            return Err(AppError::BatchFailure(
                "filename template is empty, too long, or contains a path separator".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchState {
    Idle,
    Discovering,
    Running,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchFailureRecord {
    pub input_path: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchStatus {
    pub batch_id: u64,
    pub state: BatchState,
    pub discovered: usize,
    pub completed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub current_file: Option<String>,
    pub estimated_remaining_ms: Option<u64>,
    pub elapsed_ms: u64,
    pub failures: Vec<BatchFailureRecord>,
    pub log_path: Option<String>,
}

impl Default for BatchStatus {
    fn default() -> Self {
        Self {
            batch_id: 0,
            state: BatchState::Idle,
            discovered: 0,
            completed: 0,
            skipped: 0,
            failed: 0,
            current_file: None,
            estimated_remaining_ms: None,
            elapsed_ms: 0,
            failures: Vec::new(),
            log_path: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchPreview {
    pub discovered: usize,
    pub sample_outputs: Vec<String>,
    pub estimated_time_ms: u64,
    pub skipped_existing: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceLayout {
    pub schema_version: u32,
    pub name: String,
    pub left_panel_width: u16,
    pub right_panel_width: u16,
    pub collapsed_sections: Vec<String>,
    pub active_panel: String,
    pub high_contrast: bool,
    pub ui_scale: f32,
}

impl WorkspaceLayout {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.schema_version != 1 {
            return Err(AppError::WorkspaceLoading(
                "unsupported workspace schema version".into(),
            ));
        }
        if self.name.trim().is_empty()
            || !(180..=800).contains(&self.left_panel_width)
            || !(240..=900).contains(&self.right_panel_width)
            || !(0.75..=2.0).contains(&self.ui_scale)
        {
            return Err(AppError::WorkspaceLoading(
                "workspace dimensions or scale are invalid".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutBinding {
    pub action: String,
    pub keys: String,
}

pub fn validate_shortcuts(bindings: &[ShortcutBinding]) -> Result<(), AppError> {
    let mut seen = std::collections::HashMap::<String, String>::new();
    for binding in bindings {
        if binding.action.trim().is_empty() || binding.keys.trim().is_empty() {
            return Err(AppError::ShortcutConflict(
                "shortcut actions and keys cannot be empty".into(),
            ));
        }
        let normalized = binding.keys.to_ascii_lowercase().replace(' ', "");
        if let Some(action) = seen.insert(normalized.clone(), binding.action.clone()) {
            return Err(AppError::ShortcutConflict(format!(
                "{} and {} both use {}",
                action, binding.action, binding.keys
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workflow() -> Workflow {
        Workflow {
            id: "restore-scan".into(),
            name: "Restore Old Scan".into(),
            description: "Local deterministic workflow".into(),
            folder: "Restoration".into(),
            favorite: true,
            operations: vec![EditOperation::Brightness { amount: 0.1 }],
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    #[test]
    fn valid_workflow_round_trips() {
        let document = WorkflowDocument {
            schema_version: WORKFLOW_SCHEMA_VERSION,
            workflow: workflow(),
        };
        let json = serde_json::to_string_pretty(&document).unwrap();
        let decoded: WorkflowDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, document);
        decoded.validate().unwrap();
    }

    #[test]
    fn unknown_workflow_versions_are_rejected() {
        let document = WorkflowDocument {
            schema_version: 99,
            workflow: workflow(),
        };
        assert!(matches!(
            document.validate(),
            Err(AppError::WorkflowValidation(_))
        ));
    }

    #[test]
    fn future_unknown_document_fields_are_ignored() {
        let json = r#"{"schemaVersion":1,"futureField":true,"workflow":{"id":"x","name":"X","operations":[{"type":"grayscale"}]}}"#;
        let document: WorkflowDocument = serde_json::from_str(json).unwrap();
        document.validate().unwrap();
    }

    #[test]
    fn workflow_requires_operations() {
        let mut value = workflow();
        value.operations.clear();
        assert!(value.validate().is_err());
    }

    #[test]
    fn workflow_rejects_invalid_operations() {
        let mut value = workflow();
        value.operations = vec![EditOperation::Gamma { value: 0.0 }];
        assert!(value.validate().is_err());
    }

    #[test]
    fn batch_options_bound_workers() {
        for workers in [0, MAX_BATCH_WORKERS + 1, usize::MAX] {
            let options = BatchOptions {
                input_folder: "in".into(),
                output_folder: "out".into(),
                filename_template: "{name}".into(),
                recursive: false,
                overwrite: false,
                workers,
                export_profile: ExportProfile::Lossless,
                dry_run: false,
            };
            assert!(options.validate().is_err());
        }
    }

    #[test]
    fn batch_options_accept_supported_worker_counts() {
        for workers in 1..=MAX_BATCH_WORKERS {
            let options = BatchOptions {
                input_folder: "in".into(),
                output_folder: "out".into(),
                filename_template: "{name}-{index}".into(),
                recursive: false,
                overwrite: false,
                workers,
                export_profile: ExportProfile::Web,
                dry_run: true,
            };
            options.validate().unwrap();
        }
    }

    #[test]
    fn shortcut_conflicts_are_case_insensitive() {
        let bindings = vec![
            ShortcutBinding {
                action: "open".into(),
                keys: "Ctrl+O".into(),
            },
            ShortcutBinding {
                action: "other".into(),
                keys: "ctrl+o".into(),
            },
        ];
        assert!(matches!(
            validate_shortcuts(&bindings),
            Err(AppError::ShortcutConflict(_))
        ));
    }

    #[test]
    fn unique_shortcuts_pass() {
        validate_shortcuts(&[
            ShortcutBinding {
                action: "open".into(),
                keys: "Ctrl+O".into(),
            },
            ShortcutBinding {
                action: "save".into(),
                keys: "Ctrl+S".into(),
            },
        ])
        .unwrap();
    }

    #[test]
    fn workspace_bounds_are_enforced() {
        let valid = WorkspaceLayout {
            schema_version: 1,
            name: "Editing".into(),
            left_panel_width: 240,
            right_panel_width: 360,
            collapsed_sections: vec![],
            active_panel: "tools".into(),
            high_contrast: false,
            ui_scale: 1.0,
        };
        valid.validate().unwrap();
        let mut invalid = valid;
        invalid.ui_scale = 4.0;
        assert!(invalid.validate().is_err());
    }
}
