//! Builds the example plugins in `plugins/examples/` into `.photoforge-plugin`
//! packages.
//!
//! ```text
//! cargo run --example build_example_plugins -- [output directory]
//! ```
//!
//! Each example directory holds a `manifest.json`, an optional `plugin.wat` (the
//! module, as WebAssembly text) and a `README.md`. The output is deterministic, and
//! the packages are not part of the installer: they are for people writing plugins to
//! read, install and change.
use photoforge_lib::plugins::package::{read_package, sha256_hex};
use photoforge_lib::plugins::testing::example_package;
use std::path::PathBuf;

fn main() {
    let out: PathBuf = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("target/example-plugins"), PathBuf::from);
    let examples = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins/examples");
    std::fs::create_dir_all(&out).expect("create the output directory");
    let mut directories: Vec<PathBuf> = std::fs::read_dir(&examples)
        .expect("read plugins/examples")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    directories.sort();
    for directory in directories {
        let manifest: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(directory.join("manifest.json")).expect("manifest.json"),
        )
        .expect("manifest.json is JSON");
        let module = std::fs::read_to_string(directory.join("plugin.wat"))
            .ok()
            .map(|text| wat::parse_str(&text).expect("plugin.wat is valid WebAssembly text"));
        let readme = std::fs::read_to_string(directory.join("README.md")).ok();
        let bytes = example_package(&manifest, module.as_deref(), readme.as_deref());
        // Read it back through the same reader an installer uses.
        let loaded = read_package(&bytes).expect("the package it built is valid");
        let name = format!(
            "{}-{}.photoforge-plugin",
            loaded.manifest.id, loaded.manifest.version
        );
        std::fs::write(out.join(&name), &bytes).expect("write the package");
        println!(
            "{name}  {} bytes  sha256 {}",
            bytes.len(),
            sha256_hex(&bytes)
        );
    }
}
