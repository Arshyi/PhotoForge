//! Macro files: a bounded way to read and write one exported macro.
//!
//! The interface turns a macro into text and back; this is only the file. It is
//! narrow on purpose. It reads and writes text that is a PhotoForge macro document
//! and nothing else, in files no larger than a macro may be, so it cannot be used as
//! a general way to read or overwrite files from the interface. What a macro's steps
//! mean is not decided here — the transaction engine checks every step when the macro
//! is run, and is the authority.
use crate::error::AppError;
use serde_json::Value;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// The most a macro file may hold. A hundred steps of any registered operation are
/// far smaller; this mirrors the limit the interface enforces.
pub const MAX_MACRO_FILE_BYTES: u64 = 1_000_000;
const KIND: &str = "photoforge-macro";
const SCHEMA_VERSION: u64 = 1;

fn invalid(reason: impl Into<String>) -> AppError {
    AppError::WorkflowImport(format!("macro file: {}", reason.into()))
}

/// Checks that `text` is a macro document, without interpreting its steps.
pub fn check_macro_text(text: &str) -> Result<(), AppError> {
    if text.len() as u64 > MAX_MACRO_FILE_BYTES {
        return Err(invalid(format!(
            "it is larger than the {MAX_MACRO_FILE_BYTES}-byte limit"
        )));
    }
    let value: Value =
        serde_json::from_str(text).map_err(|error| invalid(format!("not valid JSON ({error})")))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid("it is not a macro document"))?;
    if object.get("kind").and_then(Value::as_str) != Some(KIND) {
        return Err(invalid("it is not a PhotoForge macro"));
    }
    if object.get("schemaVersion").and_then(Value::as_u64) != Some(SCHEMA_VERSION) {
        return Err(invalid("it is a version this build does not read"));
    }
    if !object.get("macro").is_some_and(Value::is_object) {
        return Err(invalid("it holds no macro"));
    }
    Ok(())
}

pub fn read_macro_file(path: &Path) -> Result<String, AppError> {
    let metadata = fs::metadata(path).map_err(|error| invalid(error.to_string()))?;
    if !metadata.is_file() || metadata.len() > MAX_MACRO_FILE_BYTES {
        return Err(invalid(format!(
            "it is not a regular file within the {MAX_MACRO_FILE_BYTES}-byte limit"
        )));
    }
    // Read through a hard cap as well as checking the size above: a file that grows
    // between the check and the read is still not read past the limit.
    let mut text = String::new();
    fs::File::open(path)
        .and_then(|file| {
            file.take(MAX_MACRO_FILE_BYTES + 1)
                .read_to_string(&mut text)
        })
        .map_err(|error| invalid(error.to_string()))?;
    check_macro_text(&text)?;
    Ok(text)
}

pub fn write_macro_file(path: &Path, text: &str) -> Result<PathBuf, AppError> {
    check_macro_text(text)?;
    let json_extension = path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("json"));
    if !path.is_absolute() || !json_extension {
        return Err(invalid("an export needs an absolute .json path"));
    }
    let parent = path
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| invalid("the folder does not exist"))?;
    if path.is_dir() {
        return Err(invalid("the path is a folder"));
    }
    // Written beside the target and renamed over it, so an interrupted write leaves
    // the old file whole, not half of the new one.
    let temporary = parent.join(format!(
        ".{}.photoforge-tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("macro")
    ));
    fs::write(&temporary, text).map_err(|error| invalid(error.to_string()))?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(invalid(error.to_string()));
    }
    Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn document() -> String {
        json!({"kind": "photoforge-macro", "schemaVersion": 1, "macro": {"name": "M", "steps": []}})
            .to_string()
    }

    #[test]
    fn a_macro_round_trips_and_an_export_replaces_an_older_one() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("macro.json");
        write_macro_file(&path, &document()).unwrap();
        assert_eq!(read_macro_file(&path).unwrap(), document());
        let newer = document().replace("\"M\"", "\"Newer\"");
        write_macro_file(&path, &newer).unwrap();
        assert_eq!(read_macro_file(&path).unwrap(), newer);
        let leftovers: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            leftovers.len(),
            1,
            "a temporary file was left: {leftovers:?}"
        );
    }

    #[test]
    fn only_macro_documents_are_read_or_written() {
        let directory = tempfile::tempdir().unwrap();
        let other = directory.path().join("notes.json");
        fs::write(&other, r#"{"password":"hunter2"}"#).unwrap();
        // Not a macro: it is not handed to the interface...
        assert!(read_macro_file(&other).is_err());
        // ...and nothing that is not a macro is written, wherever it is asked to go.
        let target = directory.path().join("out.json");
        for text in [
            "not json",
            "[]",
            r#"{"kind":"something-else","schemaVersion":1,"macro":{}}"#,
            r#"{"kind":"photoforge-macro","schemaVersion":2,"macro":{}}"#,
            r#"{"kind":"photoforge-macro","schemaVersion":1}"#,
            r#"{"kind":"photoforge-macro","schemaVersion":1,"macro":5}"#,
        ] {
            assert!(write_macro_file(&target, text).is_err(), "{text}");
        }
        assert!(!target.exists());
    }

    #[test]
    fn size_is_bounded_in_both_directions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("big.json");
        let padding = "x".repeat(MAX_MACRO_FILE_BYTES as usize);
        let big =
            json!({"kind": "photoforge-macro", "schemaVersion": 1, "macro": {"pad": padding}})
                .to_string();
        assert!(write_macro_file(&path, &big).is_err());
        assert!(!path.exists());
        fs::write(&path, &big).unwrap();
        assert!(read_macro_file(&path).is_err());
    }

    #[test]
    fn the_destination_must_be_a_json_file_in_a_folder_that_exists() {
        let directory = tempfile::tempdir().unwrap();
        for bad in [
            directory.path().join("macro.exe"),
            directory.path().join("macro"),
            directory.path().join("missing").join("macro.json"),
            PathBuf::from("macro.json"),
        ] {
            assert!(write_macro_file(&bad, &document()).is_err(), "{bad:?}");
        }
        // A folder named like a file is not overwritten.
        let folder = directory.path().join("folder.json");
        fs::create_dir(&folder).unwrap();
        assert!(write_macro_file(&folder, &document()).is_err());
        assert!(folder.is_dir());
    }

    #[test]
    fn a_missing_file_or_a_folder_is_an_error_not_a_panic() {
        let directory = tempfile::tempdir().unwrap();
        assert!(read_macro_file(&directory.path().join("nothing.json")).is_err());
        assert!(read_macro_file(directory.path()).is_err());
    }
}
