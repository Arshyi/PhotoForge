//! Shared helpers for the plugin tests: the example and adversarial modules, and
//! images to run them on.
#![allow(dead_code)]
use photoforge_lib::color::{FloatImage, FloatRgba};
use photoforge_lib::plugins::manifest::{FilterDecl, Locality, PluginManifest};
use serde_json::Value;
use std::path::PathBuf;

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub fn wat(path: PathBuf) -> Vec<u8> {
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    wat::parse_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub fn adversarial(name: &str) -> Vec<u8> {
    wat(repo_root()
        .join("plugins/adversarial")
        .join(format!("{name}.wat")))
}

pub fn example_module(name: &str) -> Vec<u8> {
    wat(repo_root()
        .join("plugins/examples")
        .join(name)
        .join("plugin.wat"))
}

pub fn example_manifest_json(name: &str) -> Value {
    let path = repo_root()
        .join("plugins/examples")
        .join(name)
        .join("manifest.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// An example's manifest as the host sees it once the module's checksum is in.
pub fn example_manifest(name: &str) -> PluginManifest {
    let module = example_module(name);
    let value =
        photoforge_lib::plugins::testing::manifest_for(&module, example_manifest_json(name));
    PluginManifest::from_json(&value.to_string()).unwrap()
}

/// A filter declaration for the adversarial modules, which take no parameters.
pub fn plain_decl(locality: Locality) -> FilterDecl {
    FilterDecl {
        id: "main".into(),
        title: "Main".into(),
        description: String::new(),
        locality,
        deterministic: true,
        parameters: Vec::new(),
    }
}

/// A deterministic image with edges and gradients at every scale, so a pixel that
/// reads the wrong neighbour changes the answer.
pub fn pattern(width: u32, height: u32) -> FloatImage {
    let mut image = FloatImage::blank(width, height, FloatRgba::TRANSPARENT).unwrap();
    let mut state = 0x9e37_79b9u32;
    for y in 0..height {
        for x in 0..width {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (state >> 8) as f32 / (1u32 << 24) as f32;
            let ramp = ((x * 7 + y * 3) % 97) as f32 / 96.0;
            let edge = if (x / 5 + y / 6) % 2 == 0 { 0.25 } else { 0.0 };
            let alpha = if (x + y) % 11 == 0 { 0.5 } else { 1.0 };
            image.pixels_mut()[(y * width + x) as usize] = FloatRgba::new(
                (ramp * 0.6 + edge + noise * 0.1).min(1.5),
                (noise * 0.7 + edge).min(1.5),
                ((x as f32 / width as f32) * 0.8 + noise * 0.1).min(1.5),
                alpha,
            );
        }
    }
    image
}

pub fn bits(image: &FloatImage) -> Vec<[u32; 4]> {
    image
        .pixels()
        .iter()
        .map(|p| {
            [
                p.red.to_bits(),
                p.green.to_bits(),
                p.blue.to_bits(),
                p.alpha.to_bits(),
            ]
        })
        .collect()
}
