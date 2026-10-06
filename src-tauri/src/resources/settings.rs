//! The user's memory-budget choice, kept across sessions.
//!
//! Failure to read the setting must never stop the application starting: a
//! missing, unreadable or corrupt file means "Automatic", which is also what a
//! first run is. Failure to *write* it is reported, because the user asked for
//! something that then did not stick.
use super::memory::MemoryProbe;
use super::policy::{configure, Budget, BudgetMode};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

const FILE_NAME: &str = "resources.json";
const MAX_SETTINGS_BYTES: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    /// Bumped if the meaning of a field ever changes, so an old file is
    /// recognised as old instead of being misread.
    version: u32,
    budget: BudgetMode,
}

const VERSION: u32 = 1;

/// Where settings live: the one directory PhotoForge owns under local
/// application data.
pub fn settings_directory() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("PhotoForge")
        .join("settings")
}

pub fn settings_path() -> PathBuf {
    settings_directory().join(FILE_NAME)
}

/// Reads the saved mode from `path`, or `Automatic` if there is nothing usable.
pub fn load_mode_from(path: &Path) -> BudgetMode {
    let Ok(meta) = std::fs::metadata(path) else {
        return BudgetMode::Automatic;
    };
    // A settings file that is not tiny is not a settings file.
    if !meta.is_file() || meta.len() > MAX_SETTINGS_BYTES {
        return BudgetMode::Automatic;
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return BudgetMode::Automatic;
    };
    match serde_json::from_str::<Stored>(&text) {
        Ok(stored) if stored.version == VERSION => stored.budget,
        _ => BudgetMode::Automatic,
    }
}

pub fn load_mode() -> BudgetMode {
    load_mode_from(&settings_path())
}

/// Writes the mode atomically: a crash mid-write leaves the previous file, never
/// half of a new one.
pub fn save_mode_to(path: &Path, mode: BudgetMode) -> Result<(), AppError> {
    let directory = path
        .parent()
        .ok_or_else(|| AppError::ProcessingFailure("the settings path has no directory".into()))?;
    std::fs::create_dir_all(directory).map_err(|error| {
        AppError::ProcessingFailure(format!("could not create settings: {error}"))
    })?;
    let json = serde_json::to_string_pretty(&Stored {
        version: VERSION,
        budget: mode,
    })
    .map_err(|error| AppError::ProcessingFailure(format!("could not encode settings: {error}")))?;
    let mut staged = tempfile::NamedTempFile::new_in(directory).map_err(|error| {
        AppError::ProcessingFailure(format!("could not stage settings: {error}"))
    })?;
    staged
        .write_all(json.as_bytes())
        .and_then(|()| staged.flush())
        .map_err(|error| {
            AppError::ProcessingFailure(format!("could not write settings: {error}"))
        })?;
    staged.persist(path).map_err(|error| {
        AppError::ProcessingFailure(format!("could not save settings: {error}"))
    })?;
    Ok(())
}

pub fn save_mode(mode: BudgetMode) -> Result<(), AppError> {
    save_mode_to(&settings_path(), mode)
}

/// Installs the budget for this machine and this user's saved choice.
///
/// Called at start-up and at document-open boundaries; see the policy module on
/// why not between.
pub fn install(probe: &dyn MemoryProbe) -> Budget {
    configure(load_mode(), probe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_automatic() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            load_mode_from(&dir.path().join("nothing.json")),
            BudgetMode::Automatic
        );
    }

    #[test]
    fn a_saved_mode_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings").join("resources.json");
        for mode in [
            BudgetMode::Automatic,
            BudgetMode::Manual { bytes: 12 << 30 },
        ] {
            save_mode_to(&path, mode).unwrap();
            assert_eq!(load_mode_from(&path), mode);
        }
    }

    /// Corruption must degrade to the default, never block start-up.
    #[test]
    fn a_corrupt_or_hostile_file_falls_back_to_automatic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("resources.json");
        for content in [
            "",
            "not json",
            "{\"version\": 1}",
            "{\"version\": 99, \"budget\": {\"mode\": \"automatic\"}}",
            "{\"version\": 1, \"budget\": {\"mode\": \"manual\", \"bytes\": \"lots\"}}",
            "{\"version\": 1, \"budget\": {\"mode\": \"manual\", \"bytes\": -5}}",
        ] {
            std::fs::write(&path, content).unwrap();
            assert_eq!(load_mode_from(&path), BudgetMode::Automatic, "{content:?}");
        }
        // Oversized: a settings file is a few dozen bytes.
        std::fs::write(&path, "x".repeat(MAX_SETTINGS_BYTES as usize + 1)).unwrap();
        assert_eq!(load_mode_from(&path), BudgetMode::Automatic);
    }

    #[test]
    fn saving_replaces_the_file_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("resources.json");
        save_mode_to(&path, BudgetMode::Manual { bytes: 8 << 30 }).unwrap();
        save_mode_to(&path, BudgetMode::Automatic).unwrap();
        assert_eq!(load_mode_from(&path), BudgetMode::Automatic);
        // No staging file is left beside it.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .collect();
        assert_eq!(leftovers.len(), 1, "stray files: {leftovers:?}");
    }
}
