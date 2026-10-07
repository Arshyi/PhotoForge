//! Reading a `.photoforge-plugin` package, which is a ZIP file, as hostile input.
//!
//! A package arrives from a stranger, so the reader trusts none of it: not the
//! sizes, not the names, not the offsets, not the directory. It is a deliberately
//! small reader rather than a general ZIP library, because everything a general one
//! supports and a plugin does not need is surface a malicious archive can aim at:
//!
//! * only **stored** and **deflated** entries, never anything else;
//! * no encryption, no ZIP64, no split archives, no archive comment;
//! * no entry may be a symbolic link or a directory, and names are a closed set of
//!   plain relative paths with no traversal, no drive, no device name, no stream;
//! * a **closed list of files**: the manifest, the module the manifest names, and
//!   an optional README and licence. Anything else is refused, not skipped;
//! * every offset and size is checked before it is used, in 64-bit arithmetic, and
//!   entry data may not overlap itself, the directory or the end record (which is
//!   how the overlapping-entries "zip bomb" works);
//! * inflation is bounded by the size the entry declared, which is itself bounded,
//!   so a few kilobytes cannot become gigabytes; the result must be exactly that
//!   size and match the recorded CRC-32;
//! * the module's SHA-256 must equal the one the manifest states, and it must look
//!   like a WebAssembly module before anything tries to compile it.
//!
//! The reader allocates in proportion to what the limits below allow and no more.
use super::manifest::{validate_package_path, PluginManifest};
use crate::error::AppError;
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub const MAX_PACKAGE_BYTES: u64 = 48 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 16;
pub const MAX_MODULE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_DOCUMENT_BYTES: u64 = 256 * 1024;
pub const MAX_TOTAL_UNCOMPRESSED: u64 = 40 * 1024 * 1024;
/// A deflate stream cannot honestly exceed about 1,030:1; this is a margin above
/// that for tiny entries and a hard stop for large ones.
pub const MAX_COMPRESSION_RATIO: u64 = 1000;
pub const MANIFEST_NAME: &str = "manifest.json";
pub const ALLOWED_DOCUMENTS: [&str; 3] = ["README.md", "LICENSE", "LICENSE.txt"];

const SIG_EOCD: u32 = 0x0605_4b50;
const SIG_CENTRAL: u32 = 0x0201_4b50;
const SIG_LOCAL: u32 = 0x0403_4b50;
const SIG_ZIP64_LOCATOR: u32 = 0x0706_4b50;
const EOCD_LEN: usize = 22;
const CENTRAL_LEN: usize = 46;
const LOCAL_LEN: usize = 30;
const FLAG_DATA_DESCRIPTOR: u16 = 1 << 3;
const FLAG_UTF8: u16 = 1 << 11;

fn refuse(reason: impl Into<String>) -> AppError {
    AppError::InvalidPluginManifest(format!(
        "the package is not safe to read: {}",
        reason.into()
    ))
}

fn u16_at(data: &[u8], at: usize) -> Result<u16, AppError> {
    data.get(at..at + 2)
        .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
        .ok_or_else(|| refuse("it ends in the middle of a record"))
}

fn u32_at(data: &[u8], at: usize) -> Result<u32, AppError> {
    data.get(at..at + 4)
        .map(|bytes| u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        .ok_or_else(|| refuse("it ends in the middle of a record"))
}

#[derive(Debug, Clone)]
struct Entry {
    name: String,
    method: u16,
    flags: u16,
    crc: u32,
    compressed: u64,
    uncompressed: u64,
    /// Where the entry's data begins, once the local header has been read.
    data_start: u64,
}

/// What a package holds, after every check.
#[derive(Debug, Clone)]
pub struct LoadedPackage {
    pub manifest: PluginManifest,
    /// The manifest exactly as it was stored.
    pub manifest_json: String,
    /// The WebAssembly module, if the plugin has one.
    pub module: Option<Vec<u8>>,
    pub readme: Option<String>,
    pub license: Option<String>,
    /// SHA-256 of the package file itself, to tell two downloads apart.
    pub package_sha256: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Locates the end record and returns `(directory offset, directory size, entries)`.
fn read_end_record(data: &[u8]) -> Result<(u64, u64, usize), AppError> {
    if data.len() < EOCD_LEN {
        return Err(refuse("it is too small to be a ZIP file"));
    }
    let eocd = data.len() - EOCD_LEN;
    // A package carries no archive comment, so the end record is exactly the last
    // 22 bytes. Searching for it would let a comment hide a second, fake one.
    if u32_at(data, eocd)? != SIG_EOCD {
        return Err(refuse(
            "it does not end with a ZIP end record (or has a comment)",
        ));
    }
    if eocd >= 20 && u32_at(data, eocd - 20)? == SIG_ZIP64_LOCATOR {
        return Err(refuse("ZIP64 archives are not accepted"));
    }
    let (disk, directory_disk) = (u16_at(data, eocd + 4)?, u16_at(data, eocd + 6)?);
    let (here, total) = (u16_at(data, eocd + 8)?, u16_at(data, eocd + 10)?);
    let (size, offset) = (u32_at(data, eocd + 12)?, u32_at(data, eocd + 16)?);
    if u16_at(data, eocd + 20)? != 0 {
        return Err(refuse("an archive comment is not allowed"));
    }
    if disk != 0 || directory_disk != 0 || here != total {
        return Err(refuse("split archives are not accepted"));
    }
    if total == 0xFFFF || size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF {
        return Err(refuse("ZIP64 archives are not accepted"));
    }
    if usize::from(total) == 0 || usize::from(total) > MAX_ENTRIES {
        return Err(refuse(format!("it must hold 1 to {MAX_ENTRIES} files")));
    }
    // The directory must end exactly where the end record begins.
    if u64::from(offset) + u64::from(size) != eocd as u64 {
        return Err(refuse(
            "its directory does not end where the end record begins",
        ));
    }
    Ok((u64::from(offset), u64::from(size), usize::from(total)))
}

fn read_central_directory(
    data: &[u8],
    offset: u64,
    size: u64,
    count: usize,
) -> Result<Vec<(Entry, u64)>, AppError> {
    let directory = data
        .get(offset as usize..(offset + size) as usize)
        .ok_or_else(|| refuse("its directory lies outside the file"))?;
    let mut entries = Vec::with_capacity(count);
    let mut at = 0usize;
    for _ in 0..count {
        if u32_at(directory, at)? != SIG_CENTRAL {
            return Err(refuse("a directory record is damaged"));
        }
        let made_by = u16_at(directory, at + 4)?;
        let flags = u16_at(directory, at + 8)?;
        let method = u16_at(directory, at + 10)?;
        let crc = u32_at(directory, at + 16)?;
        let compressed = u32_at(directory, at + 20)?;
        let uncompressed = u32_at(directory, at + 24)?;
        let name_len = usize::from(u16_at(directory, at + 28)?);
        let extra_len = usize::from(u16_at(directory, at + 30)?);
        let comment_len = usize::from(u16_at(directory, at + 32)?);
        let disk = u16_at(directory, at + 34)?;
        let external = u32_at(directory, at + 38)?;
        let local = u32_at(directory, at + 42)?;
        let name_end = at + CENTRAL_LEN + name_len;
        let record_end = name_end + extra_len + comment_len;
        let name_bytes = directory
            .get(at + CENTRAL_LEN..name_end)
            .ok_or_else(|| refuse("a file name runs past the directory"))?;
        let extra = directory
            .get(name_end..name_end + extra_len)
            .ok_or_else(|| refuse("an extra field runs past the directory"))?;
        if record_end > directory.len() {
            return Err(refuse("a directory record runs past the directory"));
        }
        if flags & !(FLAG_DATA_DESCRIPTOR | FLAG_UTF8) != 0 {
            return Err(refuse(
                "an entry is encrypted or uses an unsupported feature",
            ));
        }
        if method != 0 && method != 8 {
            return Err(refuse(format!(
                "an entry uses compression method {method}; only stored and deflate are accepted"
            )));
        }
        if disk != 0 {
            return Err(refuse("split archives are not accepted"));
        }
        if compressed == 0xFFFF_FFFF || uncompressed == 0xFFFF_FFFF || local == 0xFFFF_FFFF {
            return Err(refuse("ZIP64 archives are not accepted"));
        }
        if comment_len > 256 || extra_len > 256 {
            return Err(refuse(
                "an entry carries an unreasonable amount of extra data",
            ));
        }
        // Walk the extra fields, so a ZIP64 block cannot be smuggled in them.
        let mut cursor = 0usize;
        while cursor < extra.len() {
            let id = u16_at(extra, cursor)?;
            let length = usize::from(u16_at(extra, cursor + 2)?);
            if id == 0x0001 {
                return Err(refuse("ZIP64 archives are not accepted"));
            }
            cursor += 4 + length;
            if cursor > extra.len() {
                return Err(refuse("an extra field is malformed"));
            }
        }
        // The high half of the external attributes is the Unix mode when the entry
        // was made on Unix. A symbolic link there is a way to name a file the
        // package does not contain.
        if made_by >> 8 == 3 {
            let mode = external >> 16;
            if mode & 0o170000 == 0o120000 {
                return Err(refuse("an entry is a symbolic link"));
            }
            if mode & 0o170000 != 0 && mode & 0o170000 != 0o100000 {
                return Err(refuse("an entry is not a regular file"));
            }
        }
        let name = std::str::from_utf8(name_bytes)
            .map_err(|_| refuse("a file name is not valid text"))?
            .to_string();
        if name.ends_with('/') {
            return Err(refuse("directories are not accepted"));
        }
        validate_package_path(&name).map_err(|error| refuse(error.to_string()))?;
        entries.push((
            Entry {
                name,
                method,
                flags,
                crc,
                compressed: u64::from(compressed),
                uncompressed: u64::from(uncompressed),
                data_start: 0,
            },
            u64::from(local),
        ));
        at = record_end;
    }
    if at != directory.len() {
        return Err(refuse("its directory has data after the last record"));
    }
    Ok(entries)
}

/// Reads and checks the local header of an entry, and returns where its data starts.
fn locate_data(
    data: &[u8],
    entry: &Entry,
    local: u64,
    directory_start: u64,
) -> Result<u64, AppError> {
    let at = local as usize;
    if local
        .checked_add(LOCAL_LEN as u64)
        .is_none_or(|end| end > directory_start)
    {
        return Err(refuse("an entry starts outside the file's data area"));
    }
    if u32_at(data, at)? != SIG_LOCAL {
        return Err(refuse("a local header is damaged"));
    }
    let flags = u16_at(data, at + 6)?;
    let method = u16_at(data, at + 8)?;
    let name_len = usize::from(u16_at(data, at + 26)?);
    let extra_len = usize::from(u16_at(data, at + 28)?);
    // Both copies of the facts about an entry must agree, or a tool that reads one
    // and a tool that reads the other would see different archives.
    if method != entry.method || flags != entry.flags {
        return Err(refuse("a local header disagrees with the directory"));
    }
    let name = data
        .get(at + LOCAL_LEN..at + LOCAL_LEN + name_len)
        .ok_or_else(|| refuse("a local header runs past the file"))?;
    if name != entry.name.as_bytes() {
        return Err(refuse(
            "a local header names a different file than the directory",
        ));
    }
    if entry.flags & FLAG_DATA_DESCRIPTOR == 0 {
        let crc = u32_at(data, at + 14)?;
        let compressed = u64::from(u32_at(data, at + 18)?);
        let uncompressed = u64::from(u32_at(data, at + 22)?);
        if crc != entry.crc || compressed != entry.compressed || uncompressed != entry.uncompressed
        {
            return Err(refuse("a local header disagrees with the directory"));
        }
    }
    if extra_len > 256 {
        return Err(refuse(
            "an entry carries an unreasonable amount of extra data",
        ));
    }
    let start = local + LOCAL_LEN as u64 + name_len as u64 + extra_len as u64;
    let end = start
        .checked_add(entry.compressed)
        .ok_or_else(|| refuse("an entry's size overflows"))?;
    if end > directory_start {
        return Err(refuse("an entry's data runs into the directory"));
    }
    Ok(start)
}

fn inflate(data: &[u8], entry: &Entry) -> Result<Vec<u8>, AppError> {
    let start = entry.data_start as usize;
    let stored = data
        .get(start..start + entry.compressed as usize)
        .ok_or_else(|| refuse("an entry's data lies outside the file"))?;
    let bytes = match entry.method {
        0 => {
            if entry.compressed != entry.uncompressed {
                return Err(refuse("a stored entry's sizes disagree"));
            }
            stored.to_vec()
        }
        _ => {
            miniz_oxide::inflate::decompress_to_vec_with_limit(stored, entry.uncompressed as usize)
                .map_err(|_| {
                    refuse(format!(
                        "{} does not decompress to the size it declares",
                        entry.name
                    ))
                })?
        }
    };
    if bytes.len() as u64 != entry.uncompressed {
        return Err(refuse(format!(
            "{} is not the size it declares",
            entry.name
        )));
    }
    if crc32fast::hash(&bytes) != entry.crc {
        return Err(refuse(format!(
            "{} does not match its checksum",
            entry.name
        )));
    }
    Ok(bytes)
}

/// Reads a package from memory, applying every check above and the manifest's own.
pub fn read_package(data: &[u8]) -> Result<LoadedPackage, AppError> {
    if data.len() as u64 > MAX_PACKAGE_BYTES {
        return Err(refuse("it is larger than 48 MiB"));
    }
    let (offset, size, count) = read_end_record(data)?;
    let mut entries = read_central_directory(data, offset, size, count)?;

    // Names: no duplicates, compared as Windows compares them.
    let mut seen = HashSet::new();
    for (entry, _) in &entries {
        if !seen.insert(entry.name.to_ascii_lowercase()) {
            return Err(refuse(format!("{} appears twice", entry.name)));
        }
    }

    let mut total = 0u64;
    for (entry, _) in &entries {
        let limit =
            if entry.name == MANIFEST_NAME || ALLOWED_DOCUMENTS.contains(&entry.name.as_str()) {
                if entry.name == MANIFEST_NAME {
                    super::manifest::MAX_MANIFEST_BYTES as u64
                } else {
                    MAX_DOCUMENT_BYTES
                }
            } else {
                MAX_MODULE_BYTES
            };
        if entry.uncompressed > limit {
            return Err(refuse(format!(
                "{} is larger than {limit} bytes",
                entry.name
            )));
        }
        if entry.uncompressed > (1 << 20)
            && entry.uncompressed > entry.compressed.saturating_mul(MAX_COMPRESSION_RATIO)
        {
            return Err(refuse(format!(
                "{} has a suspicious compression ratio",
                entry.name
            )));
        }
        total += entry.uncompressed;
    }
    if total > MAX_TOTAL_UNCOMPRESSED {
        return Err(refuse("its files add up to more than 40 MiB"));
    }

    // Where every entry's bytes are, and that none of it overlaps anything else.
    let mut regions: Vec<(u64, u64)> = Vec::with_capacity(entries.len());
    for (entry, local) in &mut entries {
        let start = locate_data(data, entry, *local, offset)?;
        entry.data_start = start;
        regions.push((*local, start + entry.compressed));
    }
    regions.sort_unstable();
    if regions.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(refuse("two entries overlap"));
    }

    // The manifest first: it says which module belongs.
    let manifest_entry = entries
        .iter()
        .map(|(entry, _)| entry)
        .find(|entry| entry.name == MANIFEST_NAME)
        .ok_or_else(|| refuse("it has no manifest.json"))?;
    let manifest_bytes = inflate(data, manifest_entry)?;
    let manifest_json = String::from_utf8(manifest_bytes)
        .map_err(|_| refuse("manifest.json is not valid UTF-8"))?;
    let manifest = PluginManifest::from_json(&manifest_json)?;

    // The closed list of files.
    let module_path = manifest.entry.as_ref().map(|entry| entry.path.as_str());
    for (entry, _) in &entries {
        let allowed = entry.name == MANIFEST_NAME
            || ALLOWED_DOCUMENTS.contains(&entry.name.as_str())
            || module_path == Some(entry.name.as_str());
        if !allowed {
            return Err(refuse(format!(
                "it contains {}, which is not part of a plugin package",
                entry.name
            )));
        }
    }

    let module = match &manifest.entry {
        None => None,
        Some(reference) => {
            let entry = entries
                .iter()
                .map(|(entry, _)| entry)
                .find(|entry| entry.name == reference.path)
                .ok_or_else(|| {
                    refuse(format!(
                        "the module {} is not in the package",
                        reference.path
                    ))
                })?;
            let bytes = inflate(data, entry)?;
            if sha256_hex(&bytes) != reference.sha256 {
                return Err(refuse(
                    "the module does not match the checksum in the manifest",
                ));
            }
            // `\0asm` and version 1: anything else is not a module, whatever its name.
            if bytes.len() < 8 || bytes[..4] != *b"\0asm" || bytes[4..8] != [1, 0, 0, 0] {
                return Err(refuse("the module is not a WebAssembly module"));
            }
            Some(bytes)
        }
    };

    let document = |name: &str| -> Result<Option<String>, AppError> {
        match entries
            .iter()
            .map(|(entry, _)| entry)
            .find(|entry| entry.name == name)
        {
            None => Ok(None),
            Some(entry) => {
                let text = String::from_utf8(inflate(data, entry)?)
                    .map_err(|_| refuse(format!("{name} is not valid UTF-8")))?;
                Ok(Some(text))
            }
        }
    };
    let license = match document("LICENSE")? {
        Some(text) => Some(text),
        None => document("LICENSE.txt")?,
    };
    Ok(LoadedPackage {
        manifest,
        manifest_json,
        module,
        readme: document("README.md")?,
        license,
        package_sha256: sha256_hex(data),
    })
}

/// Writes a package: the tool the examples are built with, and the fixtures.
///
/// Entries are written in the order given, with a fixed timestamp so the same input
/// gives the same bytes. Not a general ZIP writer: no comment, no extra fields.
pub fn write_package(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut directory = Vec::new();
    for (name, bytes) in files {
        let crc = crc32fast::hash(bytes);
        let (method, body): (u16, Vec<u8>) = if deflate {
            (8, miniz_oxide::deflate::compress_to_vec(bytes, 6))
        } else {
            (0, bytes.to_vec())
        };
        let local = out.len() as u32;
        out.extend_from_slice(&SIG_LOCAL.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // time
        out.extend_from_slice(&0x0021u16.to_le_bytes()); // date: 1980-01-01
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&body);

        directory.extend_from_slice(&SIG_CENTRAL.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&method.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0x0021u16.to_le_bytes());
        directory.extend_from_slice(&crc.to_le_bytes());
        directory.extend_from_slice(&(body.len() as u32).to_le_bytes());
        directory.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        directory.extend_from_slice(&(name.len() as u16).to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u32.to_le_bytes());
        directory.extend_from_slice(&local.to_le_bytes());
        directory.extend_from_slice(name.as_bytes());
    }
    let directory_offset = out.len() as u32;
    out.extend_from_slice(&directory);
    out.extend_from_slice(&SIG_EOCD.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    out.extend_from_slice(&directory_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}
