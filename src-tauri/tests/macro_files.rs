//! The two commands that read and write macro files: what they refuse before they
//! touch anything, and that what they do accept round-trips.
use photoforge_lib::commands::{export_macro, import_macro};
use serde_json::json;

fn macro_text() -> String {
    json!({"kind": "photoforge-macro", "schemaVersion": 1, "macro": {"name": "M", "steps": []}})
        .to_string()
}

#[test]
fn a_macro_round_trips_through_the_commands() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .join("m.json")
        .to_string_lossy()
        .into_owned();
    let saved = export_macro(path.clone(), macro_text()).unwrap();
    assert!(saved.ends_with("m.json"));
    assert_eq!(import_macro(path).unwrap(), macro_text());
}

#[test]
fn paths_that_would_reach_beyond_the_machine_or_the_file_are_refused_unopened() {
    for bad in [
        // A network share: opening it makes the machine authenticate to someone else's server.
        r"\\attacker\share\m.json",
        "//attacker/share/m.json",
        // Device namespaces.
        r"\\.\pipe\m.json",
        r"\\?\C:\m.json",
        // An alternate data stream.
        r"C:\Users\Public\m.json:hidden",
        // Traversal, and a path that is not absolute.
        r"C:\Users\..\m.json",
        r"m.json",
        r"..\m.json",
        "",
        "C:\\Users\\Public\\m\0.json",
    ] {
        assert!(import_macro(bad.to_string()).is_err(), "imported {bad:?}");
        assert!(
            export_macro(bad.to_string(), macro_text()).is_err(),
            "exported to {bad:?}"
        );
    }
}

#[test]
fn only_a_macro_is_ever_written_or_read_whatever_the_path() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .join("x.json")
        .to_string_lossy()
        .into_owned();
    assert!(export_macro(path.clone(), r#"{"anything":"else"}"#.into()).is_err());
    assert!(!std::path::Path::new(&path).exists());
    std::fs::write(&path, r#"{"password":"hunter2"}"#).unwrap();
    assert!(import_macro(path).is_err());
}
