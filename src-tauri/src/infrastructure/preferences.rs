use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    pub preview_max_dimension: u32,
    pub default_export_format: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            preview_max_dimension: 1_600,
            default_export_format: "png".into(),
        }
    }
}

impl Preferences {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_as_json() {
        let preferences = Preferences::default();
        let json = preferences.to_json().unwrap();
        assert_eq!(Preferences::from_json(&json).unwrap(), preferences);
    }
}
