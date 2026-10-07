//! Builders for packages, honest and hostile, used by tests and by the tool that
//! builds the example plugins. Not part of the editor's behaviour.
//!
//! The hostile builder writes ZIP structures a well-behaved writer never would —
//! a name that disagrees with its local header, an entry that points into the
//! directory, a field set to the ZIP64 marker — because the reader's job is to
//! refuse exactly those, and the only way to know it does is to hand it each one.
use super::package::{sha256_hex, write_package};
use serde_json::{json, Value};

/// One entry, with every field the reader checks open to being wrong.
#[derive(Debug, Clone)]
pub struct RawEntry {
    pub name: Vec<u8>,
    /// The name written in the local header, if it should differ.
    pub local_name: Option<Vec<u8>>,
    /// The bytes as they sit in the file (already compressed, for method 8).
    pub body: Vec<u8>,
    pub method: u16,
    pub flags: u16,
    pub crc: u32,
    pub compressed: u32,
    pub uncompressed: u32,
    pub made_by: u16,
    pub external: u32,
    pub extra: Vec<u8>,
    pub comment: Vec<u8>,
    /// Where the directory says the local header is, if not where it really is.
    pub offset_override: Option<u32>,
}

impl RawEntry {
    pub fn stored(name: &str, data: &[u8]) -> Self {
        Self {
            name: name.as_bytes().to_vec(),
            local_name: None,
            body: data.to_vec(),
            method: 0,
            flags: 0,
            crc: crc32fast::hash(data),
            compressed: data.len() as u32,
            uncompressed: data.len() as u32,
            made_by: 20,
            external: 0,
            extra: Vec::new(),
            comment: Vec::new(),
            offset_override: None,
        }
    }

    pub fn deflated(name: &str, data: &[u8]) -> Self {
        let body = miniz_oxide::deflate::compress_to_vec(data, 6);
        Self {
            method: 8,
            compressed: body.len() as u32,
            body,
            ..Self::stored(name, data)
        }
    }
}

/// The bytes of an entry's local header and data, as they sit in the file.
pub fn local_record(entry: &RawEntry) -> Vec<u8> {
    let local_name = entry.local_name.as_ref().unwrap_or(&entry.name);
    let mut out = Vec::new();
    out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&entry.flags.to_le_bytes());
    out.extend_from_slice(&entry.method.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0x0021u16.to_le_bytes());
    out.extend_from_slice(&entry.crc.to_le_bytes());
    out.extend_from_slice(&entry.compressed.to_le_bytes());
    out.extend_from_slice(&entry.uncompressed.to_le_bytes());
    out.extend_from_slice(&(local_name.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(local_name);
    out.extend_from_slice(&entry.body);
    out
}

/// Assembles entries into a ZIP file exactly as described, faults included.
pub fn assemble(entries: &[RawEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut directory = Vec::new();
    for entry in entries {
        let offset = out.len() as u32;
        out.extend_from_slice(&local_record(entry));

        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        directory.extend_from_slice(&entry.made_by.to_le_bytes());
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&entry.flags.to_le_bytes());
        directory.extend_from_slice(&entry.method.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0x0021u16.to_le_bytes());
        directory.extend_from_slice(&entry.crc.to_le_bytes());
        directory.extend_from_slice(&entry.compressed.to_le_bytes());
        directory.extend_from_slice(&entry.uncompressed.to_le_bytes());
        directory.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        directory.extend_from_slice(&(entry.extra.len() as u16).to_le_bytes());
        directory.extend_from_slice(&(entry.comment.len() as u16).to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&entry.external.to_le_bytes());
        directory.extend_from_slice(&entry.offset_override.unwrap_or(offset).to_le_bytes());
        directory.extend_from_slice(&entry.name);
        directory.extend_from_slice(&entry.extra);
        directory.extend_from_slice(&entry.comment);
    }
    let directory_offset = out.len() as u32;
    out.extend_from_slice(&directory);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(directory.len() as u32).to_le_bytes());
    out.extend_from_slice(&directory_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

/// A manifest with the module's checksum filled in.
pub fn manifest_for(module: &[u8], mut manifest: Value) -> Value {
    manifest["entry"] = json!({ "path": "plugin.wasm", "sha256": sha256_hex(module) });
    manifest
}

/// A minimal valid manifest for a plugin with one pointwise filter.
pub fn basic_manifest(id: &str) -> Value {
    json!({
        "format": "photoforge-plugin", "formatVersion": 1, "apiVersion": 1,
        "id": id, "name": "Test plugin", "version": "1.0.0", "publisher": "Tests",
        "description": "A plugin written by a test.", "license": "MIT",
        "capabilities": ["filter.pixels"],
        "filters": [{ "id": "main", "title": "Main", "locality": { "kind": "pointwise" } }]
    })
}

/// A well-formed package holding `module` and a manifest built from `manifest`.
pub fn package(module: &[u8], manifest: Value, deflate: bool) -> Vec<u8> {
    let manifest = manifest_for(module, manifest);
    let json = serde_json::to_vec_pretty(&manifest).expect("manifest serialises");
    write_package(
        &[("manifest.json", &json), ("plugin.wasm", module)],
        deflate,
    )
}

/// A package with no module, for a plugin that is only commands and panels.
pub fn package_without_module(manifest: &Value) -> Vec<u8> {
    let json = serde_json::to_vec_pretty(manifest).expect("manifest serialises");
    write_package(&[("manifest.json", &json)], false)
}

/// The package an example directory becomes: its manifest (with the module's
/// checksum filled in when there is a module), the module, and the README.
///
/// Deterministic: the same inputs give the same bytes, so a package's hash can be
/// compared across machines and builds. WebAssembly text is turned into a binary
/// module by the caller, because the text parser is a test dependency only.
pub fn example_package(manifest: &Value, module: Option<&[u8]>, readme: Option<&str>) -> Vec<u8> {
    let mut manifest = manifest.clone();
    match module {
        Some(module) => manifest = manifest_for(module, manifest),
        None => {
            manifest
                .as_object_mut()
                .expect("a manifest is an object")
                .remove("entry");
        }
    }
    let json = serde_json::to_vec_pretty(&manifest).expect("manifest serialises");
    let mut files: Vec<(&str, &[u8])> = vec![("manifest.json", &json)];
    if let Some(module) = module {
        files.push(("plugin.wasm", module));
    }
    if let Some(readme) = readme {
        files.push(("README.md", readme.as_bytes()));
    }
    write_package(&files, true)
}
