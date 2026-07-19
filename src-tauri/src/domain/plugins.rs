use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Component, Path};

pub const MAX_PLUGIN_MANIFEST_BYTES: usize = 65_536;
pub const MAX_PLUGIN_MANIFESTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginType {
    Planner,
    RestorationEngine,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub schema_version: u32,
    pub name: String,
    pub version: String,
    #[serde(rename = "type")]
    pub plugin_type: PluginType,
    pub provider: String,
    pub entry: String,
    pub memory_estimate_mb: u32,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

impl PluginManifest {
    pub fn from_json(json: &str) -> Result<Self, AppError> {
        if json.len() > MAX_PLUGIN_MANIFEST_BYTES {
            return Err(AppError::InvalidPluginManifest(
                "manifest exceeds the 64 KiB limit".into(),
            ));
        }
        let manifest: Self = serde_json::from_str(json).map_err(|error| {
            AppError::InvalidPluginManifest(format!("manifest JSON is invalid: {error}"))
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.schema_version != 1 {
            return Err(AppError::InvalidPluginManifest(
                "schemaVersion must be 1".into(),
            ));
        }
        if self.name.trim().is_empty() || self.name.len() > 80 {
            return Err(AppError::InvalidPluginManifest(
                "name must contain 1 to 80 characters".into(),
            ));
        }
        if !valid_version(&self.version) {
            return Err(AppError::InvalidPluginManifest(
                "version must use numeric semantic-version segments".into(),
            ));
        }
        if self.provider.trim().is_empty()
            || self.provider.len() > 80
            || !self
                .provider
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
        {
            return Err(AppError::InvalidPluginManifest(
                "provider must be a bounded identifier".into(),
            ));
        }
        validate_entry(&self.entry)?;
        if self.memory_estimate_mb > 262_144 {
            return Err(AppError::InvalidPluginManifest(
                "memoryEstimateMb exceeds the 256 GiB metadata limit".into(),
            ));
        }
        if self.capabilities.len() > 32 {
            return Err(AppError::InvalidPluginManifest(
                "at most 32 capabilities may be declared".into(),
            ));
        }
        let mut unique = HashSet::new();
        for capability in &self.capabilities {
            if capability.is_empty()
                || capability.len() > 64
                || !capability
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
            {
                return Err(AppError::InvalidPluginManifest(
                    "capabilities must be bounded identifiers".into(),
                ));
            }
            if !unique.insert(capability.to_ascii_lowercase()) {
                return Err(AppError::InvalidPluginManifest(
                    "capabilities must not contain duplicates".into(),
                ));
            }
        }
        Ok(())
    }
}

fn valid_version(version: &str) -> bool {
    let segments: Vec<_> = version.split('.').collect();
    (2..=3).contains(&segments.len())
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment.len() <= 6
                && segment.chars().all(|character| character.is_ascii_digit())
        })
}

fn validate_entry(entry: &str) -> Result<(), AppError> {
    if entry.trim().is_empty() || entry.len() > 260 {
        return Err(AppError::InvalidPluginManifest(
            "entry must contain 1 to 260 characters".into(),
        ));
    }
    let path = Path::new(entry);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(AppError::InvalidPluginManifest(
            "entry must be a relative path without parent traversal".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifestRecord {
    pub manifest_path: String,
    pub valid: bool,
    pub manifest: Option<PluginManifest>,
    pub error: Option<String>,
    pub execution_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginScanResult {
    pub directory: String,
    pub records: Vec<PluginManifestRecord>,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_json() -> String {
        serde_json::json!({
            "schemaVersion": 1,
            "name": "Example Planner",
            "version": "1.0.0",
            "type": "planner",
            "provider": "example-local",
            "entry": "adapters/example-planner.disabled",
            "memoryEstimateMb": 128,
            "capabilities": ["guided_editing", "offline"]
        })
        .to_string()
    }

    #[test]
    fn parses_a_valid_manifest() {
        let manifest = PluginManifest::from_json(&valid_json()).unwrap();
        assert_eq!(manifest.name, "Example Planner");
        assert_eq!(manifest.plugin_type, PluginType::Planner);
    }

    #[test]
    fn parses_restoration_engine_type() {
        let json = valid_json().replace("\"planner\"", "\"restoration_engine\"");
        assert_eq!(
            PluginManifest::from_json(&json).unwrap().plugin_type,
            PluginType::RestorationEngine
        );
    }

    #[test]
    fn rejects_unknown_fields() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["execute"] = serde_json::Value::Bool(true);
        assert!(PluginManifest::from_json(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_unknown_schema_version() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["schemaVersion"] = 2.into();
        assert!(PluginManifest::from_json(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_empty_name() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["name"] = "".into();
        assert!(PluginManifest::from_json(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_long_name() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["name"] = "x".repeat(81).into();
        assert!(PluginManifest::from_json(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_non_numeric_version() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["version"] = "one.two".into();
        assert!(PluginManifest::from_json(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_too_many_version_segments() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["version"] = "1.2.3.4".into();
        assert!(PluginManifest::from_json(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_invalid_provider_identifier() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["provider"] = "provider with spaces".into();
        assert!(PluginManifest::from_json(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_absolute_entry() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["entry"] = "C:\\plugins\\adapter.dll".into();
        assert!(PluginManifest::from_json(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_parent_traversal_entry() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["entry"] = "../adapter.dll".into();
        assert!(PluginManifest::from_json(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_excessive_memory_estimate() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["memoryEstimateMb"] = 262_145.into();
        assert!(PluginManifest::from_json(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_duplicate_capabilities() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["capabilities"] = serde_json::json!(["offline", "OFFLINE"]);
        assert!(PluginManifest::from_json(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_invalid_capability_identifier() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["capabilities"] = serde_json::json!(["runs arbitrary code"]);
        assert!(PluginManifest::from_json(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_more_than_32_capabilities() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["capabilities"] = serde_json::Value::Array(
            (0..33)
                .map(|index| format!("capability_{index}").into())
                .collect(),
        );
        assert!(PluginManifest::from_json(&value.to_string()).is_err());
    }

    #[test]
    fn rejects_oversized_json_before_parsing() {
        assert!(PluginManifest::from_json(&" ".repeat(MAX_PLUGIN_MANIFEST_BYTES + 1)).is_err());
    }

    #[test]
    fn missing_capabilities_default_to_empty() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value.as_object_mut().unwrap().remove("capabilities");
        assert!(PluginManifest::from_json(&value.to_string())
            .unwrap()
            .capabilities
            .is_empty());
    }
}
