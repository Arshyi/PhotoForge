//! `.photoforge-plugin` packages as hostile input.
//!
//! Every test hands the reader something a well-behaved ZIP writer never produces,
//! and requires a refusal that says what was wrong. The package is the first thing a
//! stranger's code touches, before any WebAssembly is compiled, so this is where a
//! malicious archive has to be stopped.
mod common;
use common::*;
use photoforge_lib::plugins::package::{
    read_package, sha256_hex, write_package, MAX_ENTRIES, MAX_PACKAGE_BYTES,
};
use photoforge_lib::plugins::testing::{
    assemble, basic_manifest, local_record, manifest_for, package, RawEntry,
};
use serde_json::{json, Value};

fn module() -> Vec<u8> {
    adversarial("honest_copy")
}

fn manifest_bytes(module: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&manifest_for(module, basic_manifest("com.example.hostile"))).unwrap()
}

fn good_entries() -> Vec<RawEntry> {
    let module = module();
    vec![
        RawEntry::stored("manifest.json", &manifest_bytes(&module)),
        RawEntry::stored("plugin.wasm", &module),
    ]
}

fn refused(bytes: &[u8]) -> String {
    match read_package(bytes) {
        Ok(_) => panic!("a hostile package was accepted"),
        Err(error) => error.to_string(),
    }
}

fn refused_mentioning(bytes: &[u8], text: &str) {
    let message = refused(bytes);
    assert!(message.contains(text), "expected {text:?} in {message:?}");
}

#[test]
fn an_honest_package_loads_whether_stored_or_deflated_and_is_reproducible() {
    let module = module();
    for deflate in [false, true] {
        let bytes = package(&module, basic_manifest("com.example.honest"), deflate);
        let loaded = read_package(&bytes).unwrap();
        assert_eq!(loaded.manifest.id, "com.example.honest");
        assert_eq!(loaded.module.as_deref(), Some(module.as_slice()));
        assert_eq!(loaded.package_sha256, sha256_hex(&bytes));
        // The same inputs give the same bytes.
        assert_eq!(
            bytes,
            package(&module, basic_manifest("com.example.honest"), deflate)
        );
    }
    // The hand-assembled builder agrees with the writer.
    assert!(read_package(&assemble(&good_entries())).is_ok());

    // Documents are read; a manifest alone is a valid package.
    let manifest = json!({
        "format": "photoforge-plugin", "formatVersion": 1, "apiVersion": 1,
        "id": "com.example.panel", "name": "P", "version": "1.0.0", "publisher": "T",
        "capabilities": ["ui.panel"],
        "panels": [{ "id": "p", "title": "P", "rows": [{ "kind": "text", "text": "hi" }] }]
    });
    let json = serde_json::to_vec(&manifest).unwrap();
    let bytes = write_package(
        &[
            ("manifest.json", &json),
            ("README.md", b"# Hello"),
            ("LICENSE", b"MIT"),
        ],
        false,
    );
    let loaded = read_package(&bytes).unwrap();
    assert!(loaded.module.is_none());
    assert_eq!(loaded.readme.as_deref(), Some("# Hello"));
    assert_eq!(loaded.license.as_deref(), Some("MIT"));
}

#[test]
fn what_is_not_a_zip_is_refused() {
    for bytes in [
        Vec::new(),
        b"PK".to_vec(),
        vec![0u8; 21],
        vec![0u8; 22],
        b"MZ\x90\0 this is a program, not a package".to_vec(),
        vec![0x50, 0x4b, 0x05, 0x06]
            .into_iter()
            .chain(vec![0u8; 18])
            .collect(), // an empty archive
    ] {
        refused(&bytes);
    }
}

#[test]
fn names_cannot_escape_the_package_or_name_a_device() {
    for name in [
        "../manifest.json",
        "a/../b",
        "/etc/passwd",
        "C:/Windows/x",
        "dir\\file",
        "plugin.wasm:stream",
        "CON",
        "nul.txt",
        "com1",
        "trailing.",
        ".",
        "..",
        "a//b",
        "tab\tname",
    ] {
        let mut entries = good_entries();
        entries.push(RawEntry::stored(name, b"x"));
        refused(&assemble(&entries));
    }
    // A name that is not text.
    let mut entries = good_entries();
    entries[1].name = vec![0xff, 0xfe, b'.', b'w', b'a', b's', b'm'];
    refused(&assemble(&entries));
}

#[test]
fn the_list_of_files_is_closed() {
    for name in [
        "extra.txt",
        "payload.exe",
        "other.wasm",
        "dir/file.json",
        "readme.md.exe",
    ] {
        let mut entries = good_entries();
        entries.push(RawEntry::stored(name, b"x"));
        refused_mentioning(&assemble(&entries), "not part of a plugin package");
    }
    // The same file twice, however it is spelt.
    for twin in ["plugin.wasm", "PLUGIN.WASM", "Plugin.Wasm"] {
        let mut entries = good_entries();
        entries.push(RawEntry::stored(twin, &module()));
        refused_mentioning(&assemble(&entries), "twice");
    }
    // No manifest.
    refused_mentioning(&assemble(&good_entries()[1..]), "manifest.json");
}

#[test]
fn directories_links_encryption_and_exotic_methods_are_refused() {
    let with = |change: &dyn Fn(&mut RawEntry)| {
        let mut entries = good_entries();
        change(&mut entries[1]);
        assemble(&entries)
    };
    refused_mentioning(&with(&|e| e.name = b"plugin.wasm/".to_vec()), "directories");
    // Made on Unix (host 3) with a symlink mode in the high bits of the attributes.
    refused_mentioning(
        &with(&|e| {
            e.made_by = (3 << 8) | 20;
            e.external = 0o120777 << 16;
        }),
        "symbolic link",
    );
    refused_mentioning(
        &with(&|e| {
            e.made_by = (3 << 8) | 20;
            e.external = 0o040755 << 16;
        }),
        "regular file",
    );
    refused_mentioning(&with(&|e| e.flags = 1), "encrypted");
    refused_mentioning(&with(&|e| e.flags = 1 << 6), "encrypted");
    for method in [1u16, 6, 9, 12, 14, 93, 99] {
        refused_mentioning(&with(&|e| e.method = method), "method");
    }
}

#[test]
fn zip64_split_archives_and_comments_are_refused() {
    let with_entry = |change: &dyn Fn(&mut RawEntry)| {
        let mut entries = good_entries();
        change(&mut entries[1]);
        assemble(&entries)
    };
    refused_mentioning(&with_entry(&|e| e.compressed = 0xFFFF_FFFF), "ZIP64");
    refused_mentioning(&with_entry(&|e| e.uncompressed = 0xFFFF_FFFF), "ZIP64");
    // A ZIP64 extra field, smuggled in the entry's extra data.
    refused_mentioning(
        &with_entry(&|e| e.extra = vec![0x01, 0x00, 0x08, 0x00, 0, 0, 0, 0, 0, 0, 0, 0]),
        "ZIP64",
    );
    // A malformed extra field.
    refused_mentioning(
        &with_entry(&|e| e.extra = vec![0x99, 0x99, 0xff, 0x00]),
        "malformed",
    );
    refused_mentioning(&with_entry(&|e| e.extra = vec![0u8; 300]), "unreasonable");
    refused_mentioning(
        &with_entry(&|e| e.comment = vec![b'x'; 300]),
        "unreasonable",
    );

    let base = assemble(&good_entries());
    let eocd = base.len() - 22;
    let patch = |offset: usize, bytes: &[u8]| {
        let mut data = base.clone();
        data[eocd + offset..eocd + offset + bytes.len()].copy_from_slice(bytes);
        data
    };
    refused_mentioning(&patch(4, &[1, 0]), "split"); // disk number
    refused_mentioning(&patch(6, &[1, 0]), "split"); // directory disk
    refused_mentioning(&patch(8, &[5, 0]), "split"); // entries here != total
    let mut full = patch(8, &[0xFF, 0xFF]);
    full[eocd + 10..eocd + 12].copy_from_slice(&[0xFF, 0xFF]);
    refused_mentioning(&full, "ZIP64");
    refused_mentioning(&patch(12, &[0xFF, 0xFF, 0xFF, 0xFF]), "ZIP64");
    refused_mentioning(&patch(16, &[0xFF, 0xFF, 0xFF, 0xFF]), "ZIP64");
    refused_mentioning(&patch(20, &[4, 0]), "comment");
    // A ZIP64 locator sitting just before the end record.
    let mut with_locator = base[..eocd].to_vec();
    with_locator.extend_from_slice(&0x0706_4b50u32.to_le_bytes());
    with_locator.extend_from_slice(&[0u8; 16]);
    with_locator.extend_from_slice(&base[eocd..]);
    refused(&with_locator);
    // A comment appended after a perfectly good end record.
    let mut commented = base.clone();
    commented.extend_from_slice(b"hello");
    refused(&commented);
    // The directory does not end where the end record begins.
    refused_mentioning(&patch(12, &[1, 0, 0, 0]), "directory");
    refused_mentioning(&patch(16, &[0, 0, 0, 0]), "directory");
    // Too many entries.
    for count in [[17u8, 0], [0, 0]] {
        let mut data = patch(8, &count);
        data[eocd + 10..eocd + 12].copy_from_slice(&count);
        refused_mentioning(&data, "files");
    }
}

#[test]
fn two_copies_of_the_facts_must_agree() {
    let with = |change: &dyn Fn(&mut RawEntry)| {
        let mut entries = good_entries();
        change(&mut entries[1]);
        assemble(&entries)
    };
    // The local header names a different file than the directory does.
    refused_mentioning(
        &with(&|e| e.local_name = Some(b"other.wasm".to_vec())),
        "different file",
    );
    refused_mentioning(
        &with(&|e| e.local_name = Some(b"plugin.wasM".to_vec())),
        "different file",
    );
    // The recorded checksum, and both sizes.
    refused_mentioning(&with(&|e| e.crc ^= 1), "checksum");

    // The local header says one thing and the directory another. The builder writes
    // the same numbers to both, so the local copy is patched afterwards.
    let entries = good_entries();
    let local = 30 + entries[0].name.len() + entries[0].body.len(); // the second entry's header
    let base = assemble(&entries);
    for (offset, what) in [
        (14usize, "checksum"),
        (18, "compressed size"),
        (22, "uncompressed size"),
    ] {
        let mut data = base.clone();
        data[local + offset] ^= 1;
        let message = refused(&data);
        assert!(
            message.contains("disagrees with the directory"),
            "{what}: {message}"
        );
    }
    // Disagreement about the method or the flags, too.
    for offset in [6usize, 8] {
        let mut data = base.clone();
        data[local + offset] ^= 8;
        refused_mentioning(&data, "disagrees");
    }
    // A size that carries an entry's data into the directory.
    refused_mentioning(&with(&|e| e.compressed += 1), "directory");
}

#[test]
fn entries_may_not_overlap_each_other_or_the_directory() {
    // The second entry's directory record points at the first's local header, so two
    // names share bytes — the structure the overlapping-entries bomb is built from.
    let mut entries = good_entries();
    entries[1].offset_override = Some(0);
    refused(&assemble(&entries));
    // An entry that claims a place inside the directory.
    let mut entries = good_entries();
    let total: usize = entries
        .iter()
        .map(|e| 30 + e.name.len() + e.body.len())
        .sum();
    entries[1].offset_override = Some(total as u32 + 4);
    refused(&assemble(&entries));
    // An entry that starts beyond the end of the file.
    let mut entries = good_entries();
    entries[1].offset_override = Some(0x7fff_0000);
    refused(&assemble(&entries));
    // The real overlapping-entries construction: one entry's *data* contains another
    // entry's complete local header and data, and the second's directory record
    // points inside the first. Every name, size and checksum is individually correct,
    // so nothing but the overlap itself gives it away.
    let hidden = RawEntry::stored("LICENSE", b"MIT");
    let mut carrier = RawEntry::stored("README.md", &local_record(&hidden));
    carrier.flags = 0;
    let mut entries = good_entries();
    let before: usize = entries.iter().map(|e| local_record(e).len()).sum();
    let mut inside = hidden.clone();
    inside.offset_override = Some((before + 30 + carrier.name.len()) as u32);
    entries.push(carrier);
    entries.push(inside);
    refused_mentioning(&assemble(&entries), "overlap");
    // An entry whose data would run into the directory.
    let mut entries = good_entries();
    entries[1].compressed = entries[1].body.len() as u32 + 4096;
    entries[1].uncompressed = entries[1].compressed;
    refused(&assemble(&entries));
}

#[test]
fn compression_bombs_are_bounded_by_what_an_entry_declares() {
    // 40 MiB of zeros compresses to a few kilobytes. Declared honestly it is larger
    // than any module may be.
    let zeros = vec![0u8; 40 * 1024 * 1024];
    let honest_bomb = RawEntry::deflated("plugin.wasm", &zeros);
    assert!(honest_bomb.body.len() < 100 * 1024);
    let mut entries = good_entries();
    entries[1] = honest_bomb;
    refused_mentioning(&assemble(&entries), "larger than");

    // A smaller bomb that is under the size limit but absurdly compressed.
    let zeros = vec![0u8; 20 * 1024 * 1024];
    let mut entries = good_entries();
    entries[1] = RawEntry::deflated("plugin.wasm", &zeros);
    refused_mentioning(&assemble(&entries), "compression ratio");

    // A stream that expands past the size it declared. The declaration is small and
    // honest-looking; the data is not, and inflation stops at the declaration.
    let mut entries = good_entries();
    let big = vec![0u8; 5 * 1024 * 1024];
    let lying = RawEntry {
        uncompressed: 1000,
        crc: crc32fast::hash(&vec![0u8; 1000]),
        ..RawEntry::deflated("plugin.wasm", &big)
    };
    entries[1] = lying;
    refused_mentioning(&assemble(&entries), "declares");

    // A stream that is shorter than the size it declared.
    let mut entries = good_entries();
    let small = vec![7u8; 100];
    entries[1] = RawEntry {
        uncompressed: 5000,
        ..RawEntry::deflated("plugin.wasm", &small)
    };
    refused(&assemble(&entries));

    // A file that is itself too large to read at all.
    refused_mentioning(&vec![0u8; MAX_PACKAGE_BYTES as usize + 1], "48 MiB");
    // Too many entries, even if each is fine.
    let many: Vec<RawEntry> = (0..MAX_ENTRIES + 1)
        .map(|i| RawEntry::stored(&format!("f{i}.txt"), b"x"))
        .collect();
    refused(&assemble(&many));
}

#[test]
fn the_module_must_be_the_one_the_manifest_vouches_for_and_must_be_wasm() {
    let module = module();
    let mut tampered = module.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    // A module whose bytes differ from the checksum in the manifest.
    let mut entries = good_entries();
    entries[1] = RawEntry::stored("plugin.wasm", &tampered);
    refused_mentioning(&assemble(&entries), "checksum");
    // The manifest names a module that is not there.
    refused_mentioning(&assemble(&good_entries()[..1]), "not in the package");
    // A module that matches its checksum but is not WebAssembly.
    for junk in [
        &b"MZ\x90\0\x03\0\0\0 definitely a native executable"[..],
        b"\0asm\x02\0\0\0",
        b"short",
        b"",
    ] {
        let manifest =
            serde_json::to_vec(&manifest_for(junk, basic_manifest("com.example.hostile"))).unwrap();
        let entries = vec![
            RawEntry::stored("manifest.json", &manifest),
            RawEntry::stored("plugin.wasm", junk),
        ];
        refused_mentioning(&assemble(&entries), "not a WebAssembly module");
    }
    // A manifest with no module that nonetheless ships one.
    let manifest = json!({
        "format": "photoforge-plugin", "formatVersion": 1, "apiVersion": 1,
        "id": "com.example.nomodule", "name": "N", "version": "1.0.0", "publisher": "T",
        "capabilities": ["ui.panel"],
        "panels": [{ "id": "p", "title": "P", "rows": [{ "kind": "text", "text": "hi" }] }]
    });
    let entries = vec![
        RawEntry::stored("manifest.json", &serde_json::to_vec(&manifest).unwrap()),
        RawEntry::stored("plugin.wasm", &module),
    ];
    refused_mentioning(&assemble(&entries), "not part of a plugin package");
}

#[test]
fn a_bad_manifest_is_refused_with_its_own_reason() {
    let module = module();
    let cases: Vec<(Value, &str)> = vec![
        (
            json!({ "capabilities": ["filter.pixels", "filesystem.read"] }),
            "does not exist",
        ),
        (json!({ "capabilities": [] }), "filter.pixels"),
        (json!({ "id": "core.layer.evil" }), "id must be"),
        (json!({ "apiVersion": 9 }), "host interface"),
        (json!({ "postInstall": "run.exe" }), "unknown field"),
    ];
    for (patch, expect) in cases {
        let mut manifest = basic_manifest("com.example.bad");
        for (key, value) in patch.as_object().unwrap() {
            manifest[key] = value.clone();
        }
        let bytes = package(&module, manifest, false);
        refused_mentioning(&bytes, expect);
    }
    // A manifest that is not JSON, and one that is not UTF-8.
    for body in [&b"{ not json"[..], b"\xff\xfe\x00", b"[]", b"null"] {
        let entries = vec![RawEntry::stored("manifest.json", body)];
        refused(&assemble(&entries));
    }
    // A manifest over the size limit.
    let big = vec![b' '; 70 * 1024];
    refused(&assemble(&[RawEntry::stored("manifest.json", &big)]));
}

/// Nothing a reader is handed may make it panic, run for long or allocate without
/// bound: every prefix of a real package, and every single-byte corruption of it.
#[test]
fn truncations_and_corruptions_never_panic_and_never_yield_an_invalid_package() {
    let bytes = package(&module(), basic_manifest("com.example.fuzz"), false);
    for length in 0..bytes.len() {
        assert!(
            read_package(&bytes[..length]).is_err(),
            "a prefix of {length} bytes was accepted"
        );
    }
    let started = std::time::Instant::now();
    let mut accepted = 0;
    for position in 0..bytes.len() {
        for flip in [0x01u8, 0x80, 0xff] {
            let mut corrupt = bytes.clone();
            corrupt[position] ^= flip;
            // Either refused, or — if the flip landed somewhere that does not matter,
            // such as a timestamp — still a package that passes every check.
            if let Ok(loaded) = read_package(&corrupt) {
                accepted += 1;
                loaded.manifest.validate().unwrap();
                if let (Some(module), Some(entry)) = (&loaded.module, &loaded.manifest.entry) {
                    assert_eq!(sha256_hex(module), entry.sha256);
                }
            }
        }
    }
    assert!(started.elapsed().as_secs() < 60, "{:?}", started.elapsed());
    // Most corruptions are caught: only inert bytes (times, versions) survive.
    assert!(
        accepted < bytes.len() / 2 * 3,
        "{accepted} of {} corruptions were accepted",
        bytes.len() * 3
    );
}
