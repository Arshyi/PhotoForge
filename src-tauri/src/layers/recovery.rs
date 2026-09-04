use super::model::LayerDocument;
use super::project::{encode_project, encode_project_typed, load_project, LoadedProject};
use crate::domain::EditOperation;
use crate::error::AppError;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::Builder as TempFileBuilder;

/// Extension for a recovery snapshot. It is deliberately different from
/// `.photoforge` so a recovery file can never be mistaken for, or silently
/// overwrite, a project the user saved themselves.
pub const RECOVERY_EXTENSION: &str = "photoforge-recovery";
/// Most snapshots kept on disk. Older ones are pruned oldest-first.
pub const MAX_RECOVERY_SNAPSHOTS: usize = 3;

/// What a recovery snapshot records besides the document itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryRecord {
    /// Absolute path of the project this snapshot belongs to, when the document
    /// has been saved at least once.
    pub project_path: Option<String>,
    /// Display name for the recovery prompt.
    pub document_name: String,
    pub saved_at: String,
    pub snapshot_path: String,
    pub bytes: u64,
}

fn recovery_directory() -> Result<PathBuf, AppError> {
    let base = std::env::var_os("LOCALAPPDATA").ok_or_else(|| {
        AppError::ProjectIo("the local application-data folder is unavailable".into())
    })?;
    let base = PathBuf::from(base);
    if !base.is_absolute()
        || base
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(AppError::ProjectIo(
            "the local application-data folder is not an absolute local path".into(),
        ));
    }
    Ok(base.join("PhotoForge").join("recovery"))
}

fn map_io(error: std::io::Error) -> AppError {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        AppError::Permission
    } else {
        AppError::ProjectIo(error.to_string())
    }
}

/// A recovery snapshot is an ordinary project container plus a small sidecar
/// describing where it came from, so recovery reuses the same validated,
/// checksummed reader rather than a second parallel format.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RecoverySidecar {
    project_path: Option<String>,
    document_name: String,
    saved_at: String,
}

fn sidecar_path(snapshot: &Path) -> PathBuf {
    snapshot.with_extension(format!("{RECOVERY_EXTENSION}.json"))
}

/// Writes a snapshot of the working document into the local recovery folder.
///
/// This never touches the user's own project file. Writing is atomic, and the
/// folder is pruned to `MAX_RECOVERY_SNAPSHOTS` afterwards.
#[allow(clippy::too_many_arguments)]
pub fn write_snapshot(
    document: &LayerDocument,
    operations: &[EditOperation],
    pixels: &[(String, &RgbaImage)],
    project_path: Option<&str>,
    document_name: &str,
    application_version: &str,
    saved_at: &str,
    directory: Option<&Path>,
) -> Result<RecoveryRecord, AppError> {
    let bytes = encode_project(
        document,
        operations,
        pixels,
        application_version,
        saved_at,
        saved_at,
    )?;
    write_snapshot_bytes(bytes, project_path, document_name, saved_at, directory)
}

#[allow(clippy::too_many_arguments)]
pub fn write_snapshot_typed(
    document: &LayerDocument,
    operations: &[EditOperation],
    pixels: &[(String, crate::pixel::PixelBuffer)],
    project_path: Option<&str>,
    document_name: &str,
    application_version: &str,
    saved_at: &str,
    directory: Option<&Path>,
) -> Result<RecoveryRecord, AppError> {
    let bytes = encode_project_typed(
        document,
        operations,
        pixels,
        application_version,
        saved_at,
        saved_at,
    )?;
    write_snapshot_bytes(bytes, project_path, document_name, saved_at, directory)
}

fn write_snapshot_bytes(
    bytes: Vec<u8>,
    project_path: Option<&str>,
    document_name: &str,
    saved_at: &str,
    directory: Option<&Path>,
) -> Result<RecoveryRecord, AppError> {
    let folder = match directory {
        Some(directory) => directory.to_path_buf(),
        None => recovery_directory()?,
    };
    fs::create_dir_all(&folder).map_err(map_io)?;

    // The stem is derived from the timestamp so snapshots sort chronologically
    // and never collide.
    let stem: String = saved_at
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect();
    let snapshot = folder.join(format!("session-{stem}.{RECOVERY_EXTENSION}"));

    let temporary = TempFileBuilder::new()
        .prefix(".photoforge-recovery-")
        .suffix(".tmp")
        .tempfile_in(&folder)
        .map_err(map_io)?;
    fs::write(temporary.path(), &bytes).map_err(map_io)?;
    temporary.as_file().sync_all().map_err(map_io)?;
    temporary
        .persist(&snapshot)
        .map_err(|error| map_io(error.error))?;

    let sidecar = RecoverySidecar {
        project_path: project_path.map(str::to_string),
        document_name: document_name.to_string(),
        saved_at: saved_at.to_string(),
    };
    let sidecar_json = serde_json::to_vec_pretty(&sidecar)
        .map_err(|error| AppError::ProjectFormat(error.to_string()))?;
    fs::write(sidecar_path(&snapshot), sidecar_json).map_err(map_io)?;

    prune(&folder)?;

    Ok(RecoveryRecord {
        project_path: sidecar.project_path,
        document_name: sidecar.document_name,
        saved_at: sidecar.saved_at,
        snapshot_path: snapshot.to_string_lossy().into_owned(),
        bytes: bytes.len() as u64,
    })
}

fn snapshot_files(folder: &Path) -> Result<Vec<PathBuf>, AppError> {
    if !folder.is_dir() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = fs::read_dir(folder)
        .map_err(map_io)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.eq_ignore_ascii_case(RECOVERY_EXTENSION))
        })
        .collect();
    files.sort();
    Ok(files)
}

fn prune(folder: &Path) -> Result<(), AppError> {
    let files = snapshot_files(folder)?;
    if files.len() <= MAX_RECOVERY_SNAPSHOTS {
        return Ok(());
    }
    for stale in &files[..files.len() - MAX_RECOVERY_SNAPSHOTS] {
        let _ = fs::remove_file(stale);
        let _ = fs::remove_file(sidecar_path(stale));
    }
    Ok(())
}

/// Lists available snapshots, newest last. A snapshot whose sidecar is missing
/// or unreadable is skipped rather than guessed at.
pub fn list_snapshots(directory: Option<&Path>) -> Result<Vec<RecoveryRecord>, AppError> {
    let folder = match directory {
        Some(directory) => directory.to_path_buf(),
        None => recovery_directory()?,
    };
    let mut records = Vec::new();
    for snapshot in snapshot_files(&folder)? {
        let Ok(json) = fs::read_to_string(sidecar_path(&snapshot)) else {
            continue;
        };
        let Ok(sidecar) = serde_json::from_str::<RecoverySidecar>(&json) else {
            continue;
        };
        let bytes = fs::metadata(&snapshot)
            .map(|value| value.len())
            .unwrap_or(0);
        records.push(RecoveryRecord {
            project_path: sidecar.project_path,
            document_name: sidecar.document_name,
            saved_at: sidecar.saved_at,
            snapshot_path: snapshot.to_string_lossy().into_owned(),
            bytes,
        });
    }
    Ok(records)
}

/// Reads one snapshot back through the ordinary project reader, so a corrupt
/// snapshot is rejected by the same checks a corrupt project is.
pub fn read_snapshot(path: &Path) -> Result<LoadedProject, AppError> {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .map_or(true, |value| {
            !value.eq_ignore_ascii_case(RECOVERY_EXTENSION)
        })
    {
        return Err(AppError::ProjectIo(format!(
            "recovery snapshots must use the .{RECOVERY_EXTENSION} extension"
        )));
    }
    // Managed Windows snapshots are canonicalized to a verbatim disk path.
    // Convert that spelling back before the public loader's device-path guard.
    load_project(Path::new(&crate::raw::presentable_path(path)))
}

/// Resolves a renderer-supplied snapshot path inside PhotoForge's own recovery
/// folder. Merely carrying the recovery extension is not enough: a compromised
/// renderer must not be able to use the recovery commands as an arbitrary local
/// file reader or deleter.
fn managed_snapshot_path(path: &Path, directory: &Path) -> Result<PathBuf, AppError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(AppError::ProjectIo(
            "recovery snapshot paths must be absolute and traversal-free".into(),
        ));
    }
    if path
        .extension()
        .and_then(|value| value.to_str())
        .map_or(true, |value| {
            !value.eq_ignore_ascii_case(RECOVERY_EXTENSION)
        })
    {
        return Err(AppError::ProjectIo(format!(
            "recovery snapshots must use the .{RECOVERY_EXTENSION} extension"
        )));
    }

    let managed_directory = fs::canonicalize(directory).map_err(map_io)?;
    let managed_path = fs::canonicalize(path).map_err(map_io)?;
    let metadata = fs::metadata(&managed_path).map_err(map_io)?;
    if !metadata.is_file() || managed_path.parent() != Some(managed_directory.as_path()) {
        return Err(AppError::ProjectIo(
            "that snapshot is outside PhotoForge's recovery folder".into(),
        ));
    }
    Ok(managed_path)
}

/// Reads a renderer-selected snapshot only after confining it to the managed
/// recovery directory.
pub fn read_managed_snapshot(path: &Path) -> Result<LoadedProject, AppError> {
    let directory = recovery_directory()?;
    let path = managed_snapshot_path(path, &directory)?;
    read_snapshot(&path)
}

/// Removes one snapshot and its sidecar. Used once the user has recovered or
/// dismissed it, and after a successful save.
pub fn discard_snapshot(path: &Path) -> Result<(), AppError> {
    if path
        .extension()
        .and_then(|value| value.to_str())
        .map_or(true, |value| {
            !value.eq_ignore_ascii_case(RECOVERY_EXTENSION)
        })
    {
        return Err(AppError::ProjectIo(
            "that path is not a recovery snapshot".into(),
        ));
    }
    let _ = fs::remove_file(sidecar_path(path));
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(map_io(error)),
    }
}

/// Deletes a renderer-selected snapshot only after confining it to the managed
/// recovery directory.
pub fn discard_managed_snapshot(path: &Path) -> Result<(), AppError> {
    let directory = recovery_directory()?;
    let path = managed_snapshot_path(path, &directory)?;
    discard_snapshot(&path)
}

/// Removes every snapshot, used when the user closes a document cleanly.
pub fn discard_all(directory: Option<&Path>) -> Result<usize, AppError> {
    let folder = match directory {
        Some(directory) => directory.to_path_buf(),
        None => recovery_directory()?,
    };
    let mut removed = 0;
    for snapshot in snapshot_files(&folder)? {
        if discard_snapshot(&snapshot).is_ok() {
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layers::model::fixtures::*;
    use image::Rgba;

    fn image(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba([10, 20, 30, 255]))
    }

    fn document() -> LayerDocument {
        let mut document = LayerDocument::new(8, 8);
        document.layers = vec![pixel_layer("base", 8, 8)];
        document
    }

    fn write(folder: &Path, saved_at: &str, project_path: Option<&str>) -> RecoveryRecord {
        let buffer = image(8, 8);
        let pixels = vec![("pxbase".to_string(), &buffer)];
        write_snapshot(
            &document(),
            &[],
            &pixels,
            project_path,
            "Untitled",
            "0.8.0",
            saved_at,
            Some(folder),
        )
        .unwrap()
    }

    #[test]
    fn a_snapshot_round_trips_through_the_project_reader() {
        let directory = tempfile::tempdir().unwrap();
        let record = write(directory.path(), "2026-09-03T10-00-00Z", None);
        assert!(record.bytes > 0);

        let restored = read_snapshot(Path::new(&record.snapshot_path)).unwrap();
        assert_eq!(restored.document, document());
        assert_eq!(restored.pixels.len(), 1);
    }

    #[test]
    fn snapshots_use_their_own_extension_so_a_project_is_never_confused_for_one() {
        let directory = tempfile::tempdir().unwrap();
        let record = write(directory.path(), "2026-09-03T10-00-00Z", None);
        assert!(record.snapshot_path.ends_with(".photoforge-recovery"));
        assert!(!record.snapshot_path.ends_with(".photoforge"));
    }

    #[test]
    fn reading_and_discarding_reject_paths_that_are_not_snapshots() {
        let directory = tempfile::tempdir().unwrap();
        let stray = directory.path().join("project.photoforge");
        fs::write(&stray, b"not a snapshot").unwrap();
        assert!(read_snapshot(&stray).is_err());
        assert!(discard_snapshot(&stray).is_err());
        // The unrelated file was left alone.
        assert!(stray.is_file());
    }

    #[test]
    fn renderer_paths_are_confined_to_the_managed_recovery_folder() {
        let managed = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let managed_record = write(managed.path(), "2026-09-03T10-00-00Z", None);
        let outside_record = write(outside.path(), "2026-09-03T10-00-01Z", None);

        let accepted =
            managed_snapshot_path(Path::new(&managed_record.snapshot_path), managed.path())
                .unwrap();
        assert_eq!(
            accepted,
            fs::canonicalize(&managed_record.snapshot_path).unwrap()
        );
        assert!(
            managed_snapshot_path(Path::new(&outside_record.snapshot_path), managed.path())
                .is_err()
        );
        assert!(
            managed_snapshot_path(Path::new("relative.photoforge-recovery"), managed.path())
                .is_err()
        );
    }

    #[test]
    fn renderer_paths_reject_nested_and_parent_traversal_spellings() {
        let managed = tempfile::tempdir().unwrap();
        let record = write(managed.path(), "2026-09-03T10-00-00Z", None);
        let nested = managed.path().join("nested");
        fs::create_dir(&nested).unwrap();
        let nested_record = write(&nested, "2026-09-03T10-00-01Z", None);

        assert!(
            managed_snapshot_path(Path::new(&nested_record.snapshot_path), managed.path()).is_err()
        );

        let traversal = nested.join("..").join(
            Path::new(&record.snapshot_path)
                .file_name()
                .expect("snapshot has a file name"),
        );
        assert!(traversal.exists());
        assert!(managed_snapshot_path(&traversal, managed.path()).is_err());
    }

    #[test]
    fn listing_reports_the_recorded_origin_and_name() {
        let directory = tempfile::tempdir().unwrap();
        write(
            directory.path(),
            "2026-09-03T10-00-00Z",
            Some("C:/work/a.photoforge"),
        );
        let records = list_snapshots(Some(directory.path())).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].project_path.as_deref(),
            Some("C:/work/a.photoforge")
        );
        assert_eq!(records[0].document_name, "Untitled");
        assert_eq!(records[0].saved_at, "2026-09-03T10-00-00Z");
    }

    #[test]
    fn an_empty_or_missing_folder_lists_nothing() {
        let directory = tempfile::tempdir().unwrap();
        assert!(list_snapshots(Some(directory.path())).unwrap().is_empty());
        assert!(list_snapshots(Some(&directory.path().join("absent")))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn older_snapshots_are_pruned_to_the_documented_ceiling() {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..(MAX_RECOVERY_SNAPSHOTS + 3) {
            write(
                directory.path(),
                &format!("2026-09-03T10-00-{index:02}Z"),
                None,
            );
        }
        let records = list_snapshots(Some(directory.path())).unwrap();
        assert_eq!(records.len(), MAX_RECOVERY_SNAPSHOTS);
        // Six were written, so the three newest survive and the three oldest go.
        let kept: Vec<&str> = records
            .iter()
            .map(|record| record.saved_at.as_str())
            .collect();
        assert_eq!(
            kept,
            vec![
                "2026-09-03T10-00-03Z",
                "2026-09-03T10-00-04Z",
                "2026-09-03T10-00-05Z"
            ]
        );
    }

    #[test]
    fn a_snapshot_without_a_readable_sidecar_is_skipped_rather_than_guessed_at() {
        let directory = tempfile::tempdir().unwrap();
        let record = write(directory.path(), "2026-09-03T10-00-00Z", None);
        fs::remove_file(sidecar_path(Path::new(&record.snapshot_path))).unwrap();
        assert!(list_snapshots(Some(directory.path())).unwrap().is_empty());
    }

    #[test]
    fn a_corrupt_snapshot_is_rejected_by_the_project_reader() {
        let directory = tempfile::tempdir().unwrap();
        let record = write(directory.path(), "2026-09-03T10-00-00Z", None);
        let path = PathBuf::from(&record.snapshot_path);
        let mut bytes = fs::read(&path).unwrap();
        let middle = bytes.len() / 2;
        bytes[middle] ^= 0xff;
        fs::write(&path, bytes).unwrap();
        assert!(read_snapshot(&path).is_err());
    }

    #[test]
    fn discarding_removes_the_snapshot_and_its_sidecar() {
        let directory = tempfile::tempdir().unwrap();
        let record = write(directory.path(), "2026-09-03T10-00-00Z", None);
        let path = PathBuf::from(&record.snapshot_path);
        discard_snapshot(&path).unwrap();
        assert!(!path.exists());
        assert!(!sidecar_path(&path).exists());
        // Discarding again is not an error.
        discard_snapshot(&path).unwrap();
    }

    #[test]
    fn discarding_all_clears_the_folder() {
        let directory = tempfile::tempdir().unwrap();
        for index in 0..2 {
            write(
                directory.path(),
                &format!("2026-09-03T10-00-{index:02}Z"),
                None,
            );
        }
        assert_eq!(discard_all(Some(directory.path())).unwrap(), 2);
        assert!(list_snapshots(Some(directory.path())).unwrap().is_empty());
    }

    #[test]
    fn writing_leaves_no_temporary_files_behind() {
        let directory = tempfile::tempdir().unwrap();
        write(directory.path(), "2026-09-03T10-00-00Z", None);
        let leftovers: Vec<_> = fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn a_snapshot_never_writes_to_the_project_it_came_from() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("original.photoforge");
        fs::write(&project, b"untouched").unwrap();
        write(
            directory.path(),
            "2026-09-03T10-00-00Z",
            Some(project.to_string_lossy().as_ref()),
        );
        assert_eq!(fs::read(&project).unwrap(), b"untouched");
    }
}
