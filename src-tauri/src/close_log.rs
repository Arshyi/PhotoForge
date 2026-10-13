//! A small, local, bounded log of what happened when the window was asked to close.
//!
//! The close path has a single authority: once the interface registers a close-requested
//! listener, Tauri vetoes every close itself and closes the window only if the interface lets it.
//! That is what keeps unsaved work from being lost, and it is also what a window that does not
//! close looks like when the page, for whatever reason, does not answer. It was seen once in the
//! packaged window with a document that was failing to render, and could not be reproduced on
//! demand, so this records what it cannot yet explain: when a request arrived, what the page
//! decided, whether the window went away, and, if it did not, that it was still open three seconds
//! later. Those lines tell "the page never answered" from "the page answered and the window stayed".
//!
//! The file holds timestamps and fixed phrases only: no paths, no names, nothing about a document.
//! It lives beside the settings under `PhotoForge\diagnostics`, is cut back when it passes
//! [`MAX_BYTES`], and is never sent anywhere.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{Manager, Runtime, WebviewWindow, WindowEvent};

/// The log is cut back to nothing before a write that would take it past this.
pub const MAX_BYTES: u64 = 32 * 1024;

/// How long after a request the window may still be open before that is written down.
const STILL_OPEN_AFTER: Duration = Duration::from_secs(3);

/// The decisions the page may report. Anything else is recorded as `unrecognised`, so a page can
/// not put text of its own into the file.
const DECISIONS: [&str; 4] = ["close", "prompt-busy", "prompt-dirty", "failed"];

pub fn log_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("PhotoForge")
        .join("diagnostics")
        .join("close.log")
}

/// Appends `line`, stamped with seconds since the Unix epoch, to `path`. Failure to write is
/// ignored: a diagnostic must never be the reason the window stays.
pub fn append_to(path: &Path, line: &str) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let _ = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if fs::metadata(path).is_ok_and(|meta| meta.len() > MAX_BYTES) {
            fs::write(path, b"")?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{seconds} {line}")
    })();
}

fn append(line: &str) {
    append_to(&log_path(), line);
}

/// What the page decided about a close request, as one of [`DECISIONS`].
#[tauri::command]
pub fn note_close_decision(decision: String) {
    if DECISIONS.contains(&decision.as_str()) {
        append(&format!("the page decided: {decision}"));
    } else {
        append("the page decided: unrecognised");
    }
}

/// Records close requests and the destruction of `window`, and notes a window that is still open
/// a few seconds after a request.
pub fn watch<R: Runtime>(window: &WebviewWindow<R>) {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { .. } => {
            append("close requested");
            let app = app.clone();
            let label = label.clone();
            std::thread::spawn(move || {
                std::thread::sleep(STILL_OPEN_AFTER);
                if app.get_webview_window(&label).is_some() {
                    append("the window is still open 3 s after the request");
                }
            });
        }
        WindowEvent::Destroyed => append("the window was destroyed"),
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(path: &Path) -> Vec<String> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| line.split_once(' ').unwrap().1.to_string())
            .collect()
    }

    #[test]
    fn a_line_is_stamped_and_appended_creating_the_directory() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("diagnostics").join("close.log");
        append_to(&path, "close requested");
        append_to(&path, "the window was destroyed");
        assert_eq!(
            lines(&path),
            ["close requested", "the window was destroyed"]
        );
        let first = fs::read_to_string(&path).unwrap();
        let stamp = first.split_whitespace().next().unwrap();
        assert!(stamp.parse::<u64>().unwrap() > 1_600_000_000);
    }

    #[test]
    fn the_file_is_cut_back_rather_than_growing_without_bound() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("close.log");
        fs::write(&path, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
        append_to(&path, "close requested");
        assert_eq!(lines(&path), ["close requested"]);
        assert!(fs::metadata(&path).unwrap().len() < 200);
    }

    #[test]
    fn a_file_that_cannot_be_written_is_not_an_error() {
        // A directory where the file should be: opening it for append fails, and nothing panics.
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("close.log");
        fs::create_dir(&path).unwrap();
        append_to(&path, "close requested");
    }

    #[test]
    fn only_the_known_decisions_are_written_as_given() {
        assert!(DECISIONS.contains(&"close"));
        assert!(DECISIONS.contains(&"prompt-busy"));
        assert!(DECISIONS.contains(&"prompt-dirty"));
        assert!(!DECISIONS.contains(&"C:\\Users\\someone\\secret.png"));
    }
}
