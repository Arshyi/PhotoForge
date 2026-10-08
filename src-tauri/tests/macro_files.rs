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

/// The paths above fail for reasons of their own (nothing is at the far end), which
/// would hide a missing guard. These would *succeed* without it: they name a real
/// folder and a real file in a way the guard refuses.
#[test]
fn a_path_that_would_otherwise_work_is_still_refused_by_the_guard() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().to_string_lossy().into_owned();
    std::fs::create_dir(directory.path().join("sub")).unwrap();
    let real = directory.path().join("real.json");
    std::fs::write(&real, macro_text()).unwrap();

    // Traversal through a folder that exists.
    let through = format!(r"{root}\sub\..\real.json");
    assert!(
        import_macro(through).is_err(),
        "a traversal path was imported"
    );
    let out = format!(r"{root}\sub\..\written.json");
    assert!(export_macro(out, macro_text()).is_err());
    assert!(
        !directory.path().join("written.json").exists(),
        "a traversal path was written to"
    );

    // A verbatim (device-namespace) spelling of a real, ordinary file.
    let verbatim = format!(r"\\?\{}", real.display());
    assert!(
        import_macro(verbatim.clone()).is_err(),
        "a verbatim path was imported"
    );
    assert!(export_macro(verbatim, macro_text()).is_err());

    // The plain spelling of the same file is fine.
    assert_eq!(
        import_macro(real.to_string_lossy().into_owned()).unwrap(),
        macro_text()
    );
}
