use crate::domain::{
    ModelDiscoveryResult, ModelMetadata, PluginManifest, PluginManifestRecord, PluginScanResult,
    MAX_PLUGIN_MANIFESTS, MAX_PLUGIN_MANIFEST_BYTES,
};
use crate::error::AppError;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

const MAX_DISCOVERY_DIRECTORIES: usize = 8;
const MAX_DISCOVERED_MODELS: usize = 256;

pub fn scan_plugin_manifests(directory: &Path) -> Result<PluginScanResult, AppError> {
    let directory_text = directory.to_string_lossy().into_owned();
    if !directory.exists() {
        return Ok(PluginScanResult {
            directory: directory_text,
            records: Vec::new(),
            message: "Plugin directory not found. No plugins were loaded or executed.".into(),
        });
    }
    if !directory.is_dir() {
        return Err(AppError::InvalidPluginManifest(
            "plugin scan path must be a directory".into(),
        ));
    }

    let mut paths = fs::read_dir(directory)
        .map_err(|error| AppError::InvalidPluginManifest(error.to_string()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths.truncate(MAX_PLUGIN_MANIFESTS);

    let records = paths
        .into_iter()
        .map(|path| validate_manifest_path(&path))
        .collect::<Vec<_>>();
    let valid = records.iter().filter(|record| record.valid).count();
    let invalid = records.len() - valid;
    Ok(PluginScanResult {
        directory: directory_text,
        records,
        message: format!(
            "Validated {valid} manifest(s); {invalid} invalid. Plugin execution is disabled in Phase 4."
        ),
    })
}

fn validate_manifest_path(path: &Path) -> PluginManifestRecord {
    let manifest_path = path.to_string_lossy().into_owned();
    match fs::metadata(path)
        .map_err(|error| AppError::InvalidPluginManifest(error.to_string()))
        .and_then(|metadata| {
            if metadata.len() > MAX_PLUGIN_MANIFEST_BYTES as u64 {
                Err(AppError::InvalidPluginManifest(
                    "manifest exceeds the 64 KiB limit".into(),
                ))
            } else {
                Ok(())
            }
        })
        .and_then(|()| {
            fs::read_to_string(path)
                .map_err(|error| AppError::InvalidPluginManifest(error.to_string()))
        })
        .and_then(|json| PluginManifest::from_json(&json))
    {
        Ok(manifest) => PluginManifestRecord {
            manifest_path,
            valid: true,
            manifest: Some(manifest),
            error: None,
            execution_allowed: false,
        },
        Err(error) => PluginManifestRecord {
            manifest_path,
            valid: false,
            manifest: None,
            error: Some(error.to_string()),
            execution_allowed: false,
        },
    }
}

pub fn discover_local_models(directories: &[String]) -> Result<ModelDiscoveryResult, AppError> {
    let started = Instant::now();
    if directories.len() > MAX_DISCOVERY_DIRECTORIES {
        return Err(AppError::ModelDiscoveryFailure(
            "at most eight directories may be searched".into(),
        ));
    }
    let mut models = Vec::new();
    for directory in directories {
        if models.len() >= MAX_DISCOVERED_MODELS {
            break;
        }
        let path = PathBuf::from(directory);
        if !path.is_dir() {
            continue;
        }
        let mut entries = fs::read_dir(&path)
            .map_err(|error| AppError::ModelDiscoveryFailure(error.to_string()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|candidate| candidate.is_file() && recognized_model(candidate))
            .collect::<Vec<_>>();
        entries.sort();
        for candidate in entries {
            if models.len() >= MAX_DISCOVERED_MODELS {
                break;
            }
            models.push(model_metadata(&candidate)?);
        }
    }

    Ok(ModelDiscoveryResult {
        searched_directories: directories.to_vec(),
        message: if models.is_empty() {
            "No compatible models found.".into()
        } else {
            format!(
                "Found {} model file(s). No inference runtime is installed, so none were loaded.",
                models.len()
            )
        },
        models,
        processing_time_ms: started.elapsed().as_millis(),
    })
}

fn recognized_model(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "onnx" | "safetensors" | "pth" | "pt" | "bin"
            )
        })
}

fn model_metadata(path: &Path) -> Result<ModelMetadata, AppError> {
    let metadata =
        fs::metadata(path).map_err(|error| AppError::ModelDiscoveryFailure(error.to_string()))?;
    let file_size_bytes = metadata.len();
    let format = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("unknown")
        .to_ascii_uppercase();
    let memory_estimate_mb =
        file_size_bytes.saturating_mul(2).saturating_add(1_048_575) / 1_048_576;
    Ok(ModelMetadata {
        name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Unnamed model")
            .into(),
        path: path.to_string_lossy().into_owned(),
        format,
        file_size_bytes,
        memory_estimate_mb,
        supported_tasks: vec!["Future restoration adapter metadata".into()],
        expected_input: "Engine-defined image tensor; not loaded in Phase 4".into(),
        expected_input_size: None,
        expected_output: "Engine-defined image tensor; not produced in Phase 4".into(),
        compatible: false,
        unavailable_reason: "No compatible restoration engine is installed.".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "photoforge-component-test-{}-{}",
                std::process::id(),
                NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn valid_manifest() -> String {
        serde_json::json!({
            "schemaVersion": 1,
            "name": "Example Planner",
            "version": "1.0.0",
            "type": "planner",
            "provider": "example",
            "entry": "adapters/example.disabled",
            "memoryEstimateMb": 64,
            "capabilities": ["offline"]
        })
        .to_string()
    }

    #[test]
    fn missing_plugin_directory_is_safe() {
        let result =
            scan_plugin_manifests(Path::new("definitely-missing-photoforge-directory")).unwrap();
        assert!(result.records.is_empty());
        assert!(result
            .message
            .contains("No plugins were loaded or executed"));
    }

    #[test]
    fn valid_plugin_manifest_is_reported_but_never_executable() {
        let directory = TestDirectory::new();
        fs::write(directory.0.join("planner.json"), valid_manifest()).unwrap();
        let result = scan_plugin_manifests(&directory.0).unwrap();
        assert_eq!(result.records.len(), 1);
        assert!(result.records[0].valid);
        assert!(!result.records[0].execution_allowed);
    }

    #[test]
    fn invalid_plugin_manifest_is_retained_as_diagnostic() {
        let directory = TestDirectory::new();
        fs::write(directory.0.join("invalid.json"), "{not-json").unwrap();
        let result = scan_plugin_manifests(&directory.0).unwrap();
        assert_eq!(result.records.len(), 1);
        assert!(!result.records[0].valid);
        assert!(result.records[0].error.is_some());
    }

    #[test]
    fn oversized_plugin_manifest_is_rejected_before_content_is_read() {
        let directory = TestDirectory::new();
        fs::write(
            directory.0.join("oversized.json"),
            vec![b' '; MAX_PLUGIN_MANIFEST_BYTES + 1],
        )
        .unwrap();
        let result = scan_plugin_manifests(&directory.0).unwrap();
        assert_eq!(result.records.len(), 1);
        assert!(!result.records[0].valid);
        assert!(result.records[0]
            .error
            .as_deref()
            .is_some_and(|error| error.contains("64 KiB")));
    }

    #[test]
    fn plugin_scan_ignores_non_json_files() {
        let directory = TestDirectory::new();
        fs::write(directory.0.join("adapter.dll"), b"not executed").unwrap();
        assert!(scan_plugin_manifests(&directory.0)
            .unwrap()
            .records
            .is_empty());
    }

    #[test]
    fn model_discovery_reports_exact_empty_message() {
        let directory = TestDirectory::new();
        let result = discover_local_models(&[directory.0.to_string_lossy().into_owned()]).unwrap();
        assert!(result.models.is_empty());
        assert_eq!(result.message, "No compatible models found.");
    }

    #[test]
    fn model_discovery_reads_metadata_without_loading_model() {
        let directory = TestDirectory::new();
        fs::write(directory.0.join("restore.onnx"), vec![0_u8; 1_024]).unwrap();
        let result = discover_local_models(&[directory.0.to_string_lossy().into_owned()]).unwrap();
        assert_eq!(result.models.len(), 1);
        assert_eq!(result.models[0].format, "ONNX");
        assert_eq!(result.models[0].file_size_bytes, 1_024);
        assert_eq!(result.models[0].expected_input_size, None);
        assert!(!result.models[0].compatible);
        assert!(result.models[0]
            .unavailable_reason
            .contains("No compatible"));
    }

    #[test]
    fn model_discovery_ignores_unrecognized_files() {
        let directory = TestDirectory::new();
        fs::write(directory.0.join("notes.txt"), b"not a model").unwrap();
        assert!(
            discover_local_models(&[directory.0.to_string_lossy().into_owned()])
                .unwrap()
                .models
                .is_empty()
        );
    }

    #[test]
    fn model_discovery_rejects_more_than_eight_directories() {
        let directories = (0..9)
            .map(|index| format!("models-{index}"))
            .collect::<Vec<_>>();
        assert!(discover_local_models(&directories).is_err());
    }
}
