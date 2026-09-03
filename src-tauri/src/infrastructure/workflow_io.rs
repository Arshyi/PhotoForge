use crate::domain::WorkflowDocument;
use crate::error::AppError;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_WORKFLOW_BYTES: u64 = 64 * 1024 * 1024;

pub fn parse_workflow_json(json: &str) -> Result<WorkflowDocument, AppError> {
    if json.len() as u64 > MAX_WORKFLOW_BYTES {
        return Err(AppError::WorkflowImport(
            "workflow exceeds the 64 MiB import limit".into(),
        ));
    }
    let document: WorkflowDocument =
        serde_json::from_str(json).map_err(|error| AppError::WorkflowImport(error.to_string()))?;
    document.validate()?;
    Ok(document)
}

pub fn load_workflow(path: &Path) -> Result<WorkflowDocument, AppError> {
    let metadata =
        fs::metadata(path).map_err(|error| AppError::WorkflowImport(error.to_string()))?;
    if !metadata.is_file() || metadata.len() > MAX_WORKFLOW_BYTES {
        return Err(AppError::WorkflowImport(
            "workflow file is not a regular JSON file within the 64 MiB limit".into(),
        ));
    }
    let json =
        fs::read_to_string(path).map_err(|error| AppError::WorkflowImport(error.to_string()))?;
    parse_workflow_json(&json)
}

pub fn save_workflow(path: &Path, document: &WorkflowDocument) -> Result<PathBuf, AppError> {
    document.validate()?;
    if !path.is_absolute()
        || path
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            != Some("json".into())
    {
        return Err(AppError::WorkflowValidation(
            "workflow exports require an absolute .json path".into(),
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        AppError::WorkflowValidation("workflow export has no parent folder".into())
    })?;
    if !parent.is_dir() {
        return Err(AppError::WorkflowValidation(
            "workflow export folder does not exist".into(),
        ));
    }
    let json = serde_json::to_vec_pretty(document)
        .map_err(|error| AppError::WorkflowValidation(error.to_string()))?;
    if json.len() as u64 > MAX_WORKFLOW_BYTES {
        return Err(AppError::WorkflowValidation(
            "workflow exceeds the 64 MiB export limit".into(),
        ));
    }
    let temporary = path.with_extension("json.photoforge-tmp");
    fs::write(&temporary, json).map_err(|error| AppError::WorkflowValidation(error.to_string()))?;
    if path.exists() {
        fs::remove_file(path).map_err(|error| AppError::WorkflowValidation(error.to_string()))?;
    }
    fs::rename(&temporary, path)
        .map_err(|error| AppError::WorkflowValidation(error.to_string()))?;
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{EditOperation, Workflow, WORKFLOW_SCHEMA_VERSION};

    fn document() -> WorkflowDocument {
        WorkflowDocument {
            schema_version: WORKFLOW_SCHEMA_VERSION,
            workflow: Workflow {
                id: "one".into(),
                name: "One".into(),
                description: String::new(),
                folder: String::new(),
                favorite: false,
                operations: vec![EditOperation::Grayscale],
                layer_steps: Vec::new(),
                created_at: String::new(),
                updated_at: String::new(),
            },
        }
    }

    #[test]
    fn import_and_export_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workflow.json");
        save_workflow(&path, &document()).unwrap();
        assert_eq!(load_workflow(&path).unwrap(), document());
    }

    #[test]
    fn import_rejects_malformed_json() {
        assert!(matches!(
            parse_workflow_json("{"),
            Err(AppError::WorkflowImport(_))
        ));
    }

    #[test]
    fn import_rejects_unsupported_schema() {
        for version in [0, 3, 99] {
            let json = serde_json::to_string(&WorkflowDocument {
                schema_version: version,
                ..document()
            })
            .unwrap();
            assert!(
                matches!(
                    parse_workflow_json(&json),
                    Err(AppError::WorkflowValidation(_))
                ),
                "schema version {version} should be rejected"
            );
        }
    }

    #[test]
    fn import_still_accepts_a_phase_six_and_seven_schema_one_workflow() {
        // A version 1 file has no layer steps and must load untouched.
        let json = serde_json::to_string(&WorkflowDocument {
            schema_version: 1,
            ..document()
        })
        .unwrap();
        let parsed = parse_workflow_json(&json).unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert!(parsed.workflow.layer_steps.is_empty());
        assert_eq!(parsed.workflow.operations.len(), 1);
    }

    #[test]
    fn a_schema_one_workflow_carrying_layer_steps_is_rejected() {
        let mut document = document();
        document.schema_version = 1;
        document.workflow.layer_steps = vec![crate::layers::LayerWorkflowStep::Flatten];
        let json = serde_json::to_string(&document).unwrap();
        assert!(matches!(
            parse_workflow_json(&json),
            Err(AppError::WorkflowValidation(_))
        ));
    }

    #[test]
    fn a_schema_two_workflow_round_trips_with_its_layer_steps() {
        let mut source = document();
        source.workflow.layer_steps = vec![
            crate::layers::LayerWorkflowStep::CreateAdjustmentLayer {
                operation: Box::new(EditOperation::Contrast { amount: 0.2 }),
                name: Some("Punch".into()),
            },
            crate::layers::LayerWorkflowStep::SetOpacity {
                selector: crate::layers::LayerSelector::LastCreated,
                opacity: 0.6,
            },
        ];
        let json = serde_json::to_string(&source).unwrap();
        let parsed = parse_workflow_json(&json).unwrap();
        assert_eq!(parsed.workflow.layer_steps, source.workflow.layer_steps);
    }

    #[test]
    fn a_workflow_of_only_layer_steps_is_valid() {
        let mut source = document();
        source.workflow.operations = Vec::new();
        source.workflow.layer_steps = vec![crate::layers::LayerWorkflowStep::Flatten];
        let json = serde_json::to_string(&source).unwrap();
        assert!(parse_workflow_json(&json).is_ok());
    }

    #[test]
    fn a_workflow_that_does_nothing_at_all_is_rejected() {
        let mut source = document();
        source.workflow.operations = Vec::new();
        source.workflow.layer_steps = Vec::new();
        let json = serde_json::to_string(&source).unwrap();
        assert!(matches!(
            parse_workflow_json(&json),
            Err(AppError::WorkflowValidation(_))
        ));
    }

    #[test]
    fn export_requires_json_extension() {
        let directory = tempfile::tempdir().unwrap();
        assert!(save_workflow(&directory.path().join("workflow.txt"), &document()).is_err());
    }

    #[test]
    fn import_rejects_oversized_payload() {
        let json = " ".repeat((MAX_WORKFLOW_BYTES + 1) as usize);
        assert!(matches!(
            parse_workflow_json(&json),
            Err(AppError::WorkflowImport(_))
        ));
    }

    #[test]
    fn save_replaces_existing_file_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("workflow.json");
        fs::write(&path, "old").unwrap();
        save_workflow(&path, &document()).unwrap();
        assert!(fs::read_to_string(path).unwrap().contains("schemaVersion"));
    }
}
