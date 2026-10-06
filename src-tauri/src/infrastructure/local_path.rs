//! Paths that a project may remember and PhotoForge may later be asked to open.
//!
//! A project stores a path to a linked file or to the source of a region. Reading
//! one back must be inert: opening a project can never cause a read, a network
//! connection or a device to be touched. So a stored path is only a description,
//! and it becomes something PhotoForge acts on solely when the user explicitly
//! asks to check, relink or reopen it, at which point it goes through
//! [`validated_local_path`].
//!
//! This one function exists so that smart-object links and source origins cannot
//! drift apart in what they refuse.
use crate::error::AppError;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// Whether `path` is acceptable to store: absolute, local, and free of traversal.
///
/// Pure string checks, no filesystem access, so it is safe to run on a path read
/// from an untrusted project.
pub fn check_local_absolute(path: &str) -> Result<(), AppError> {
    let normalised = path.replace('\\', "/");
    let drive_absolute = normalised.as_bytes().get(1) == Some(&b':')
        && normalised
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        && normalised.as_bytes().get(2) == Some(&b'/');
    let unix_absolute = normalised.starts_with('/') && !normalised.starts_with("//");
    if path.contains('\0')
        || normalised.starts_with("//")
        || (!drive_absolute && !unix_absolute)
        || normalised
            .split('/')
            .any(|part| part == ".." || part == ".")
        // A colon anywhere past the drive letter is an alternate data stream.
        || normalised.get(2..).is_some_and(|tail| tail.contains(':'))
    {
        return Err(AppError::InvalidLayerDocument(
            "a stored file path must be absolute and local, without traversal, network or stream syntax"
                .into(),
        ));
    }
    Ok(())
}

/// Turns a stored path into one PhotoForge may open, or refuses.
///
/// Stricter than [`check_local_absolute`]: it also refuses DOS device names,
/// which Windows resolves even under an otherwise ordinary drive path.
pub fn validated_local_path(path: &str) -> Result<PathBuf, AppError> {
    check_local_absolute(path)?;
    let result = PathBuf::from(path);
    if !result.is_absolute()
        || path.starts_with("\\\\")
        || path.starts_with("//")
        || path.contains("://")
        || path.contains('\0')
        || result.components().any(|part| part == Component::ParentDir)
    {
        return Err(AppError::ProjectIo(
            "a stored path must be absolute and local, with no parent traversal or device or network prefix"
                .into(),
        ));
    }
    for part in path.split(['\\', '/']).skip(1) {
        let stem = part
            .split('.')
            .next()
            .unwrap_or("")
            .trim_end_matches([' ', '.'])
            .to_ascii_uppercase();
        if matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err(AppError::ProjectIo(
                "a stored path cannot name a device".into(),
            ));
        }
    }
    Ok(result)
}

/// The SHA-256 of a file's bytes and how many there were, in lowercase hex.
///
/// Reads through a single open handle and stops once `limit` bytes have been
/// seen, so a file that another process is growing cannot make this run on
/// forever or report a size it did not read.
pub fn hash_file(path: &Path, limit: u64) -> Result<(String, u64), AppError> {
    let mut file =
        std::fs::File::open(path).map_err(|error| AppError::ProjectIo(error.to_string()))?;
    let metadata = file
        .metadata()
        .map_err(|error| AppError::ProjectIo(error.to_string()))?;
    if !metadata.is_file() {
        return Err(AppError::ProjectIo("the path is not a regular file".into()));
    }
    hash_reader(&mut file, limit)
}

pub fn hash_reader(reader: &mut impl Read, limit: u64) -> Result<(String, u64), AppError> {
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| AppError::ProjectIo(error.to_string()))?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or(AppError::OutOfMemoryRisk)?;
        if total > limit {
            return Err(AppError::ProjectTooLarge {
                bytes: total,
                limit,
            });
        }
        hasher.update(&buffer[..count]);
    }
    Ok((format!("{:x}", hasher.finalize()), total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_absolute_paths_are_accepted() {
        for path in [
            "C:\\Users\\Someone\\Pictures\\scan.png",
            "C:/photos/a b/scan.png",
            "/home/someone/scan.png",
        ] {
            check_local_absolute(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        }
    }

    #[test]
    fn hostile_paths_are_refused_without_touching_the_disk() {
        for path in [
            "",
            "relative/scan.png",
            "..\\..\\scan.png",
            "C:\\photos\\..\\secret.png",
            "C:\\photos\\.\\scan.png",
            "\\\\server\\share\\scan.png",
            "//server/share/scan.png",
            "C:\\photos\\scan.png:hidden",
            "C:\\photos\\scan.png\0.txt",
            "http://example.com/scan.png",
        ] {
            assert!(check_local_absolute(path).is_err(), "accepted {path:?}");
        }
    }

    #[test]
    fn device_names_are_refused_when_a_path_is_about_to_be_opened() {
        for path in [
            "C:\\photos\\CON",
            "C:\\photos\\nul.png",
            "C:\\photos\\com1.txt",
            "C:\\photos\\LPT9",
            "C:\\photos\\AUX .png",
        ] {
            assert!(validated_local_path(path).is_err(), "opened {path:?}");
        }
        assert!(validated_local_path("C:\\photos\\console.png").is_ok());
    }

    #[test]
    fn hashing_matches_a_known_digest_and_counts_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.bin");
        std::fs::write(&path, b"abc").unwrap();
        let (digest, bytes) = hash_file(&path, 1024).unwrap();
        assert_eq!(
            digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(bytes, 3);
    }

    #[test]
    fn hashing_stops_at_the_limit_instead_of_reading_on() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.bin");
        std::fs::write(&path, vec![0u8; 5_000_000]).unwrap();
        assert!(matches!(
            hash_file(&path, 1_000_000),
            Err(AppError::ProjectTooLarge { .. })
        ));
    }

    #[test]
    fn hashing_refuses_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        assert!(hash_file(dir.path(), 1024).is_err());
    }
}
