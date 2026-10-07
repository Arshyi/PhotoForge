//! What a plugin filter costs against the native equivalent, what installed plugins
//! cost at start-up, and how well the memory estimate predicts the real peak. One
//! scenario per process, so the reported peak working set is a real high-water mark
//! and not a shared one.
//!
//! ```text
//! plugin_benchmark filter WIDTH HEIGHT KIND [REPEATS]
//!   KIND  native_contrast | plugin_solarize | native_blur | plugin_boxblur1 | plugin_boxblur4
//! plugin_benchmark startup COUNT
//! ```
//!
//! `native_contrast` and `plugin_solarize` are both pointwise, one arithmetic step per
//! channel, so their difference is what the sandbox and the copy in and out cost.
//! `native_blur` (Gaussian) and `plugin_boxblur*` (box) are *different algorithms*: the
//! comparison shows what a neighbourhood filter costs in each, not that the plugin does
//! the same work.
use photoforge_lib::color::{FloatImage, FloatRgba};
use photoforge_lib::domain::EditOperation;
use photoforge_lib::high_precision::pipeline_typed;
use photoforge_lib::pixel::PixelBuffer;
use photoforge_lib::plugins::manifest::{Capability, PluginManifest};
use photoforge_lib::plugins::store::{set_global, PluginRegistry, Resolution};
use photoforge_lib::plugins::testing::{example_package, manifest_for};
use photoforge_lib::resources::memory::{MemoryProbe, OsProbe};
use photoforge_lib::resources::ResourceEstimate;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

const NOW: &str = "2026-10-06T00:00:00.000Z";

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins/examples")
}

fn example(name: &str) -> (serde_json::Value, Vec<u8>) {
    let directory = examples().join(name);
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(directory.join("manifest.json")).unwrap())
            .unwrap();
    let module = wat::parse_str(std::fs::read_to_string(directory.join("plugin.wat")).unwrap())
        .expect("the example module is valid");
    (manifest, module)
}

fn package(name: &str, id_suffix: Option<usize>) -> (Vec<u8>, PluginManifest) {
    let (mut manifest, module) = example(name);
    if let Some(index) = id_suffix {
        manifest["id"] = format!("benchmark.copy{index:04}").into();
    }
    let full = manifest_for(&module, manifest);
    let parsed = PluginManifest::from_json(&full.to_string()).unwrap();
    (example_package(&full, Some(&module), None), parsed)
}

fn image(width: u32, height: u32) -> FloatImage {
    let mut image = FloatImage::blank(width, height, FloatRgba::TRANSPARENT).unwrap();
    let mut state = 0x9e37_79b9u32;
    for pixel in image.pixels_mut() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let value = (state >> 8) as f32 / (1u32 << 24) as f32;
        *pixel = FloatRgba::new(value, value * 0.5, 1.0 - value, 1.0);
    }
    image
}

fn private_bytes() -> u64 {
    OsProbe.process().map_or(0, |process| process.private_bytes)
}

fn peak_bytes() -> u64 {
    OsProbe
        .process()
        .map_or(0, |process| process.peak_working_set)
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn filter(width: u32, height: u32, kind: &str, repeats: usize) {
    // The default budget is derived from this machine; a large image needs the real one,
    // so the measurement is of what a person would get, not of a special setting.
    let budget = photoforge_lib::resources::configure(
        photoforge_lib::resources::BudgetMode::Automatic,
        &OsProbe,
    );
    let directory = tempfile::tempdir().unwrap();
    let registry = Arc::new(PluginRegistry::open(directory.path().join("plugins")));
    set_global(Some(Arc::clone(&registry)));

    let operation = match kind {
        "native_contrast" => EditOperation::Contrast { amount: 0.25 },
        "native_blur" => EditOperation::GaussianBlur { radius: 2.0 },
        "plugin_solarize" | "plugin_boxblur1" | "plugin_boxblur4" => {
            let (name, filter_id, parameters) = match kind {
                "plugin_solarize" => ("solarize", "solarize", vec![0.5]),
                "plugin_boxblur1" => ("boxblur", "box_blur", vec![1.0]),
                _ => ("boxblur", "box_blur", vec![4.0]),
            };
            let (bytes, manifest) = package(name, None);
            let report = registry
                .install(
                    &bytes,
                    None,
                    &manifest.declared_capabilities().unwrap(),
                    NOW,
                )
                .expect("the example installs");
            let loaded = match registry.resolve(&manifest.id, &report.content_hash) {
                Resolution::Available(loaded) => loaded,
                Resolution::Unavailable(why) => panic!("{}", why.describe(&manifest.id, "1.0.0")),
            };
            let (_, decl) = loaded.manifest.filter(filter_id).unwrap();
            EditOperation::PluginFilter {
                plugin: manifest.id.clone(),
                version: manifest.version.clone(),
                sha256: loaded.content_hash.clone(),
                filter: filter_id.into(),
                locality: decl.locality,
                parameters,
            }
        }
        other => panic!("unknown kind {other}"),
    };

    let source = Arc::new(image(width, height));
    let source_bytes = u64::from(width) * u64::from(height) * 16;
    let predicted = ResourceEstimate::pipeline(
        width,
        height,
        std::slice::from_ref(&operation),
        source_bytes,
    )
    .map(|estimate| estimate.estimated_peak_bytes);
    let before = private_bytes();
    let mut times = Vec::new();
    for _ in 0..repeats {
        // A shared handle, not a copy: whatever the pipeline has to copy, it copies inside the timing.
        let input = PixelBuffer::LinearRgbaF32(Arc::clone(&source));
        let start = Instant::now();
        let output = pipeline_typed(input, std::slice::from_ref(&operation), None);
        let elapsed = start.elapsed();
        match output {
            Ok(_) => times.push(elapsed.as_secs_f64() * 1000.0),
            Err(error) => {
                println!("kind={kind} {width}x{height} REFUSED {error}");
                return;
            }
        }
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = times[times.len() / 2];
    let megapixels = f64::from(width) * f64::from(height) / 1e6;
    println!(
        "kind={kind} size={width}x{height} mp={megapixels:.1} runs={repeats} min_ms={:.0} median_ms={median:.0} mp_per_s={:.1} \
         peak_ws_mib={:.0} private_before_mib={:.0} budget_mib={:.0} predicted_peak_mib={}",
        times[0],
        megapixels / (median / 1000.0),
        mib(peak_bytes()),
        mib(before),
        mib(budget.bytes),
        predicted.map_or("refused".to_string(), |bytes| format!("{:.0}", mib(bytes))),
    );
}

fn startup(count: usize) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("plugins");
    let baseline = private_bytes();
    let mut install_ms = Vec::new();
    {
        let registry = PluginRegistry::open(&root);
        for index in 0..count {
            let (bytes, manifest) = package("solarize", Some(index));
            let grants: Vec<Capability> = manifest.declared_capabilities().unwrap();
            let start = Instant::now();
            registry
                .install(&bytes, None, &grants, NOW)
                .expect("a copy installs");
            install_ms.push(start.elapsed().as_secs_f64() * 1000.0);
        }
    }
    // What a launch costs: opening a store that already holds `count` plugins, then using one.
    let before_open = private_bytes();
    let start = Instant::now();
    let registry = PluginRegistry::open(&root);
    let open_ms = start.elapsed().as_secs_f64() * 1000.0;
    let listed_start = Instant::now();
    let listed = registry.list();
    let list_ms = listed_start.elapsed().as_secs_f64() * 1000.0;
    let first_use_ms = if count > 0 {
        let start = Instant::now();
        let id = format!("benchmark.copy{:04}", count - 1);
        let resolved = registry.active(&id);
        assert!(
            matches!(resolved, Resolution::Available(_)),
            "the last plugin resolves"
        );
        start.elapsed().as_secs_f64() * 1000.0
    } else {
        0.0
    };
    let mean_install = if install_ms.is_empty() {
        0.0
    } else {
        install_ms.iter().sum::<f64>() / install_ms.len() as f64
    };
    println!(
        "startup plugins={count} listed={} open_ms={open_ms:.1} list_ms={list_ms:.1} first_use_ms={first_use_ms:.1} \
         mean_install_ms={mean_install:.1} private_growth_mib={:.1} (baseline {:.0} MiB)",
        listed.len(),
        mib(private_bytes().saturating_sub(before_open)),
        mib(baseline),
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("filter") if args.len() >= 5 => filter(
            args[1].parse().unwrap(),
            args[2].parse().unwrap(),
            &args[3],
            args.get(4).map_or(3, |value| value.parse().unwrap()),
        ),
        Some("startup") if args.len() == 2 => startup(args[1].parse().unwrap()),
        _ => {
            eprintln!("usage: plugin_benchmark filter W H KIND [REPEATS] | startup COUNT");
            std::process::exit(2);
        }
    }
}
