//! Plugins inside the render pipeline and the plugin store, tested as a whole: an
//! installed plugin's filter behaves like any adjustment (tiled, masked, cached), and
//! a plugin that cannot be had is reported, never replaced.
#![cfg(feature = "plugins")]
mod common;
use common::*;
use photoforge_lib::color::FloatImage;
use photoforge_lib::domain::EditOperation;
use photoforge_lib::error::AppError;
use photoforge_lib::high_precision::pipeline_typed;
use photoforge_lib::layers::tiles::{behavior, TileBehavior};
use photoforge_lib::layers::{
    render_document_float, render_document_tiled, render_document_tiled_cached, BlendMode, Layer,
    LayerContent, LayerDocument, LayerMetadata, LayerPixelStore, LayerTransform, RenderOptions,
    TileCache,
};
use photoforge_lib::mask::{MaskBitmap, MaskSnapshot};
use photoforge_lib::pixel::PixelBuffer;
use photoforge_lib::plugins::document::{
    ensure_available, hide_unavailable, requirements, statuses,
};
use photoforge_lib::plugins::manifest::{Capability, Locality};
use photoforge_lib::plugins::store::{set_global, Availability, PluginRegistry, Resolution};
use photoforge_lib::plugins::testing::{example_package, package};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, MutexGuard};

const NOW: &str = "2026-10-06T00:00:00.000Z";
static GLOBAL: Mutex<()> = Mutex::new(());

/// A plugin registry in a temporary directory, installed as the process-wide one for
/// the length of a test.
struct Sandbox {
    _lock: MutexGuard<'static, ()>,
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    registry: Arc<PluginRegistry>,
}

impl Sandbox {
    fn new() -> Self {
        let lock = GLOBAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("plugins");
        let registry = Arc::new(PluginRegistry::open(&root));
        set_global(Some(Arc::clone(&registry)));
        Self {
            _lock: lock,
            _dir: dir,
            root,
            registry,
        }
    }

    fn reopen(&mut self) {
        self.registry = Arc::new(PluginRegistry::open(&self.root));
        set_global(Some(Arc::clone(&self.registry)));
    }

    fn install(&self, bytes: &[u8], grant: &[Capability]) -> Result<String, AppError> {
        self.registry
            .install(bytes, None, grant, NOW)
            .map(|report| report.content_hash)
    }

    fn install_example(&self, name: &str) -> String {
        let manifest = example_manifest(name);
        self.install(
            &example_bytes(name),
            &manifest.declared_capabilities().unwrap(),
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        set_global(None);
    }
}

fn example_bytes(name: &str) -> Vec<u8> {
    example_package(
        &example_manifest_json(name),
        Some(&example_module(name)),
        Some("An example."),
    )
}

/// The operation a document records for an installed example's filter.
fn operation(sandbox: &Sandbox, id: &str, filter: &str, parameters: Vec<f64>) -> EditOperation {
    let Resolution::Available(loaded) = sandbox.registry.active(id) else {
        panic!("{id} is not available");
    };
    let (_, decl) = loaded.manifest.filter(filter).expect("the filter exists");
    EditOperation::PluginFilter {
        plugin: id.into(),
        version: loaded.manifest.version.clone(),
        sha256: loaded.content_hash.clone(),
        filter: filter.into(),
        locality: decl.locality,
        parameters,
    }
}

fn layer(id: &str, content: LayerContent) -> Layer {
    Layer {
        id: id.into(),
        name: id.into(),
        visible: true,
        locked: false,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        transform: LayerTransform::default(),
        mask: None,
        collapsed: false,
        metadata: LayerMetadata::default(),
        raw: None,
        origin: None,
        content,
    }
}

fn adjustment(id: &str, operation: EditOperation) -> Layer {
    layer(
        id,
        LayerContent::Adjustment {
            operation: Box::new(operation),
        },
    )
}

/// A float document: one pixel layer holding `pattern`, then the given layers.
fn document(width: u32, height: u32, extra: Vec<Layer>) -> (LayerDocument, LayerPixelStore) {
    let mut store = LayerPixelStore::default();
    store.reset(width, height).unwrap();
    let pixel_id = store.register_float(pattern(width, height)).unwrap();
    let mut layers = vec![layer(
        "base",
        LayerContent::Pixel {
            pixel_id,
            width,
            height,
        },
    )];
    layers.extend(extra);
    let mut document = LayerDocument::new(width, height);
    document.precision = photoforge_lib::pixel::DocumentPrecision::LinearSrgbF32;
    document.layers = layers;
    (document, store)
}

fn render(document: &LayerDocument, store: &LayerPixelStore) -> Result<FloatImage, AppError> {
    let resolved = store.resolve(&document.referenced_pixel_ids(), false)?;
    render_document_float(
        document,
        &resolved,
        RenderOptions {
            scale: 1.0,
            cancel: None,
        },
    )
}

fn max_difference(a: &FloatImage, b: &FloatImage) -> f32 {
    a.pixels()
        .iter()
        .zip(b.pixels())
        .map(|(p, q)| {
            (p.red - q.red)
                .abs()
                .max((p.green - q.green).abs())
                .max((p.blue - q.blue).abs())
                .max((p.alpha - q.alpha).abs())
        })
        .fold(0.0, f32::max)
}

fn error_text(result: Result<impl std::fmt::Debug, AppError>) -> String {
    result
        .expect_err("this should have been refused")
        .to_string()
}

// ---- the filter is an ordinary operation ------------------------------------------------

#[test]
fn a_plugin_filter_runs_in_the_pipeline_like_any_adjustment() {
    let sandbox = Sandbox::new();
    sandbox.install_example("solarize");
    let op = operation(
        &sandbox,
        "photoforge.example.solarize",
        "solarize",
        vec![0.4],
    );
    op.validate().unwrap();
    assert_eq!(op.kind(), "plugin_filter");
    assert!(op.supports_adjustment_layer() && op.supports_masking());

    let source = pattern(45, 33);
    let out = pipeline_typed(
        PixelBuffer::LinearRgbaF32(Arc::new(source.clone())),
        std::slice::from_ref(&op),
        None,
    )
    .unwrap();
    let PixelBuffer::LinearRgbaF32(out) = out else {
        panic!("float in, float out")
    };
    for (before, after) in source.pixels().iter().zip(out.pixels()) {
        let expect = |v: f32| if v > 0.4 { 1.0 - v } else { v };
        assert_eq!(after.red.to_bits(), expect(before.red).to_bits());
        assert_eq!(after.alpha.to_bits(), before.alpha.to_bits());
    }

    // Through a mask: untouched where the mask is empty, filtered where it is full.
    let mut mask = MaskBitmap::empty(45, 33).unwrap();
    for y in 0..33 {
        for x in 22..45 {
            mask.set(x, y, 255);
        }
    }
    let masked = EditOperation::Masked {
        operation: Box::new(op),
        mask: MaskSnapshot::encode(&mask),
        invert: false,
        mask_id: None,
    };
    masked.validate().unwrap();
    let PixelBuffer::LinearRgbaF32(both) = pipeline_typed(
        PixelBuffer::LinearRgbaF32(Arc::new(source.clone())),
        &[masked],
        None,
    )
    .unwrap() else {
        panic!()
    };
    assert_eq!(both.get(3, 3).unwrap(), source.get(3, 3).unwrap());
    assert_eq!(both.get(40, 3).unwrap(), out.get(40, 3).unwrap());
}

#[test]
fn what_an_operation_reaches_is_what_the_tiler_plans_for() {
    let sandbox = Sandbox::new();
    sandbox.install_example("boxblur");
    sandbox.install_example("solarize");
    let local = operation(
        &sandbox,
        "photoforge.example.boxblur",
        "box_blur",
        vec![2.0],
    );
    let point = operation(
        &sandbox,
        "photoforge.example.solarize",
        "solarize",
        vec![0.5],
    );
    // Radius 4 plus the one pixel every halo-dependent operation is given.
    assert_eq!(
        behavior(&local, 1.0),
        TileBehavior::HaloDependent { radius: 5 }
    );
    assert_eq!(behavior(&point, 1.0), TileBehavior::TileLocal);
    let EditOperation::PluginFilter {
        plugin,
        version,
        sha256,
        filter,
        parameters,
        ..
    } = point
    else {
        unreachable!()
    };
    let global = EditOperation::PluginFilter {
        plugin,
        version,
        sha256,
        filter,
        parameters,
        locality: Locality::Global,
    };
    assert_eq!(behavior(&global, 1.0), TileBehavior::Global);
}

#[test]
fn a_plugin_adjustment_layer_renders_the_same_tiled_as_whole() {
    let sandbox = Sandbox::new();
    sandbox.install_example("boxblur");
    sandbox.install_example("solarize");
    let blur = operation(
        &sandbox,
        "photoforge.example.boxblur",
        "box_blur",
        vec![3.0],
    );
    let solarize = operation(
        &sandbox,
        "photoforge.example.solarize",
        "solarize",
        vec![0.5],
    );
    let (document, store) = document(
        200,
        150,
        vec![adjustment("blur", blur), adjustment("sol", solarize)],
    );
    let resolved = store
        .resolve(&document.referenced_pixel_ids(), false)
        .unwrap();
    let options = RenderOptions {
        scale: 1.0,
        cancel: None,
    };
    let whole = render_document_float(&document, &resolved, options).unwrap();
    // The blur must actually have changed the picture, or the comparison is empty.
    let (plain, _) = {
        let (doc, store) = self::document(200, 150, vec![]);
        (render(&doc, &store).unwrap(), ())
    };
    assert!(max_difference(&whole, &plain) > 0.05);
    for tile in [64, 100, 128] {
        let (tiled, stats) = render_document_tiled(&document, &resolved, options, tile).unwrap();
        assert!(
            stats.haloed_tiles > 0,
            "tile {tile}: nothing was given a halo"
        );
        let difference = max_difference(&whole, &tiled);
        assert!(
            difference < 1e-5,
            "tile {tile}: a seam of {difference} at tile edges"
        );
    }
}

#[test]
fn the_cache_cannot_serve_one_version_or_one_setting_for_another() {
    let sandbox = Sandbox::new();
    let v1 = sandbox.install_example("solarize");
    let render_with = |op: EditOperation, cache: &TileCache| {
        let (document, store) = document(128, 96, vec![adjustment("sol", op)]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        render_document_tiled_cached(
            &document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
            64,
            1,
            Some(cache),
        )
        .unwrap()
        .0
    };
    let cache = TileCache::with_capacity(64 * 1024 * 1024);
    let first = render_with(
        operation(
            &sandbox,
            "photoforge.example.solarize",
            "solarize",
            vec![0.3],
        ),
        &cache,
    );
    let again = render_with(
        operation(
            &sandbox,
            "photoforge.example.solarize",
            "solarize",
            vec![0.3],
        ),
        &cache,
    );
    assert_eq!(
        bits(&first),
        bits(&again),
        "an unchanged document renders identically"
    );
    assert!(
        cache.stats().hits > 0,
        "the second render should have used the cache"
    );
    let other_setting = render_with(
        operation(
            &sandbox,
            "photoforge.example.solarize",
            "solarize",
            vec![0.8],
        ),
        &cache,
    );
    assert_ne!(
        bits(&first),
        bits(&other_setting),
        "a different parameter was served from the cache"
    );

    // A new build of the plugin: same id, same filter, different behaviour.
    let mut wat =
        std::fs::read_to_string(repo_root().join("plugins/examples/solarize/plugin.wat")).unwrap();
    assert!(wat.contains("(f32.gt (local.get 0) (local.get 1))"));
    wat = wat.replace(
        "(f32.gt (local.get 0) (local.get 1))",
        "(f32.lt (local.get 0) (local.get 1))",
    );
    let mut manifest = example_manifest_json("solarize");
    manifest["version"] = json!("1.1.0");
    let v2_bytes = example_package(&manifest, Some(&::wat::parse_str(&wat).unwrap()), None);
    let v2 = sandbox
        .install(&v2_bytes, &[Capability::FilterPixels])
        .unwrap();
    assert_ne!(v1, v2);
    let new_version = render_with(
        operation(
            &sandbox,
            "photoforge.example.solarize",
            "solarize",
            vec![0.3],
        ),
        &cache,
    );
    assert_ne!(
        bits(&first),
        bits(&new_version),
        "another build of the plugin was served from the cache"
    );

    // A document made with the old build still draws with the old build: versions are
    // kept, and the document names the exact one.
    let old = EditOperation::PluginFilter {
        plugin: "photoforge.example.solarize".into(),
        version: "1.0.0".into(),
        sha256: v1,
        filter: "solarize".into(),
        locality: Locality::Pointwise,
        parameters: vec![0.3],
    };
    assert_eq!(bits(&first), bits(&render_with(old, &cache)));
}

// ---- a plugin that cannot be had is reported, never replaced ----------------------------

#[test]
fn every_way_a_plugin_can_be_unavailable_fails_with_its_reason_and_never_passes_through() {
    let sandbox = Sandbox::new();
    let hash = sandbox.install_example("solarize");
    let op = operation(
        &sandbox,
        "photoforge.example.solarize",
        "solarize",
        vec![0.5],
    );
    let run = |op: &EditOperation| {
        pipeline_typed(
            PixelBuffer::LinearRgbaF32(Arc::new(pattern(8, 8))),
            std::slice::from_ref(op),
            None,
        )
    };
    run(&op).unwrap();

    // Turned off.
    sandbox
        .registry
        .set_enabled("photoforge.example.solarize", false)
        .unwrap();
    let message = error_text(run(&op));
    assert!(
        message.contains("turned off") && message.contains("photoforge.example.solarize"),
        "{message}"
    );
    sandbox
        .registry
        .set_enabled("photoforge.example.solarize", true)
        .unwrap();
    run(&op).unwrap();

    // Not allowed to do what its filters need.
    sandbox
        .registry
        .set_grants("photoforge.example.solarize", &[])
        .unwrap();
    let message = error_text(run(&op));
    assert!(message.contains("not been allowed"), "{message}");
    sandbox
        .registry
        .set_grants("photoforge.example.solarize", &[Capability::FilterPixels])
        .unwrap();
    run(&op).unwrap();

    // A different version is installed than the one the document names.
    let mut manifest = example_manifest_json("solarize");
    manifest["version"] = json!("2.0.0");
    manifest["description"] = json!("A second edition.");
    sandbox
        .install(
            &example_package(&manifest, Some(&example_module("solarize")), None),
            &[Capability::FilterPixels],
        )
        .unwrap();
    sandbox
        .registry
        .remove_version("photoforge.example.solarize", &hash)
        .unwrap();
    let message = error_text(run(&op));
    assert!(
        message.contains("2.0.0")
            && message.contains("1.0.0")
            && message.contains("different version"),
        "{message}"
    );

    // Not installed at all.
    sandbox
        .registry
        .remove("photoforge.example.solarize")
        .unwrap();
    let message = error_text(run(&op));
    assert!(message.contains("not installed"), "{message}");
    // The renderer, too, fails rather than drawing the layer without its filter.
    let (document, store) = document(16, 16, vec![adjustment("sol", op)]);
    assert!(render(&document, &store).is_err());
}

#[test]
fn a_document_with_a_missing_plugin_says_which_and_leaves_it_in_the_document() {
    let sandbox = Sandbox::new();
    sandbox.install_example("solarize");
    let op = operation(
        &sandbox,
        "photoforge.example.solarize",
        "solarize",
        vec![0.5],
    );
    let group = layer(
        "group",
        LayerContent::Group {
            children: vec![adjustment("inner", op.clone())],
            isolated: false,
        },
    );
    let (document, _store) = document(16, 16, vec![adjustment("outer", op), group]);

    let needed = requirements(&document);
    assert_eq!(
        needed.len(),
        1,
        "one plugin version, however many layers use it"
    );
    assert_eq!(needed[0].layer_ids.len(), 2);
    assert_eq!(needed[0].filters, vec!["solarize".to_string()]);
    assert!(statuses(&sandbox.registry, needed.clone())
        .iter()
        .all(|s| s.availability.is_available()));

    sandbox
        .registry
        .remove("photoforge.example.solarize")
        .unwrap();
    let status = statuses(&sandbox.registry, needed);
    assert_eq!(status[0].availability, Availability::Missing);
    assert!(status[0].message.contains("not installed"));

    // The render copy leaves both layers out and names them; the document is whole.
    let mut view = document.clone();
    let missing = hide_unavailable(&sandbox.registry, &mut view);
    let mut names: Vec<&str> = missing.iter().map(|m| m.layer_name.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["inner", "outer"]);
    assert!(!view.find("outer").unwrap().visible && !view.find("inner").unwrap().visible);
    assert!(
        document.find("outer").unwrap().visible,
        "the document itself was changed"
    );
    assert!(missing
        .iter()
        .all(|m| m.plugin == "photoforge.example.solarize" && !m.message.is_empty()));

    // Anything that would make a file of it refuses, naming the plugin and the layer.
    let message = error_text(ensure_available(&sandbox.registry, &document));
    assert!(
        message.contains("photoforge.example.solarize") && message.contains("not installed"),
        "{message}"
    );
    // ...unless the layers that need it would not be drawn anyway.
    let mut hidden = document.clone();
    hidden.layer_mut("outer").unwrap().visible = false;
    hidden.layer_mut("group").unwrap().visible = false;
    ensure_available(&sandbox.registry, &hidden).unwrap();
    let mut only_outer = document;
    only_outer.layer_mut("outer").unwrap().visible = false;
    assert!(
        ensure_available(&sandbox.registry, &only_outer).is_err(),
        "the group's child is still drawn"
    );
}

// ---- installing is all or nothing, and updates are safe ------------------------------------

fn bad_package(adversarial_name: &str, locality: Value) -> Vec<u8> {
    let mut manifest = photoforge_lib::plugins::testing::basic_manifest("com.example.rejected");
    manifest["filters"][0]["locality"] = locality;
    package(&adversarial(adversarial_name), manifest, false)
}

#[test]
fn a_plugin_that_fails_its_self_test_is_not_installed_and_leaves_no_trace() {
    let sandbox = Sandbox::new();
    sandbox.install_example("solarize");
    let before = serde_json::to_value(sandbox.registry.list()).unwrap();
    let files_before = file_count(&sandbox.root);

    let cases: Vec<(Vec<u8>, &str)> = vec![
        (
            bad_package("lying_locality", json!({"kind": "local", "radius": 1})),
            "tile edges",
        ),
        (
            bad_package("infinite_loop", json!({"kind": "pointwise"})),
            "failed",
        ),
        (
            bad_package("nan_output", json!({"kind": "pointwise"})),
            "not a number",
        ),
        (
            bad_package("unreachable", json!({"kind": "pointwise"})),
            "failed",
        ),
        (
            bad_package("reports_error", json!({"kind": "pointwise"})),
            "failed",
        ),
        (
            bad_package("memory_hog", json!({"kind": "pointwise"})),
            "failed",
        ),
    ];
    for (bytes, expect) in cases {
        let message = sandbox
            .install(&bytes, &[Capability::FilterPixels])
            .expect_err("a plugin that fails its self-test was installed")
            .to_string();
        assert!(
            message.contains("self-test") && message.contains(expect),
            "{expect}: {message}"
        );
    }
    // A module that asks for what the host does not give never gets as far as a test.
    let message = sandbox
        .install(
            &bad_package("imports_wasi", json!({"kind": "pointwise"})),
            &[Capability::FilterPixels],
        )
        .unwrap_err()
        .to_string();
    assert!(message.contains("wasi_snapshot_preview1"), "{message}");

    assert_eq!(
        before,
        serde_json::to_value(sandbox.registry.list()).unwrap(),
        "the list changed"
    );
    assert_eq!(
        files_before,
        file_count(&sandbox.root),
        "files were left behind"
    );
    // The one honest declaration of the same module passes.
    let honest = bad_package("lying_locality", json!({"kind": "local", "radius": 4}));
    sandbox
        .install(&honest, &[Capability::FilterPixels])
        .unwrap();
}

fn file_count(root: &std::path::Path) -> usize {
    walk(root)
}

fn walk(path: &std::path::Path) -> usize {
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| {
                    if entry.path().is_dir() {
                        walk(&entry.path())
                    } else {
                        1
                    }
                })
                .sum()
        })
        .unwrap_or(0)
}

#[test]
fn grants_are_what_was_chosen_and_never_more_than_was_asked_for() {
    let sandbox = Sandbox::new();
    let bytes = example_bytes("shapes");
    let inspection = sandbox.registry.inspect(&bytes).unwrap();
    assert!(inspection.signature.starts_with("Not signed"));
    let asked: Vec<&str> = inspection
        .capabilities
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(asked, ["filter.pixels", "document.operations", "ui.tool"]);
    assert!(inspection
        .capabilities
        .iter()
        .all(|c| !c.description.is_empty()));
    assert!(!inspection.already_installed && inspection.installed_version.is_none());

    // A grant the package did not ask for is refused outright.
    let error = sandbox
        .install(
            &bytes,
            &[Capability::FilterPixels, Capability::DocumentRead],
        )
        .unwrap_err();
    assert!(error.to_string().contains("did not ask for"), "{error}");
    // Granting nothing installs a plugin that cannot do anything yet.
    sandbox.install(&bytes, &[]).unwrap();
    let id = "photoforge.example.shapes";
    assert!(matches!(
        sandbox.registry.active(id),
        Resolution::Unavailable(Availability::NotGranted { .. })
    ));
    assert!(!sandbox.registry.granted(id, Capability::DocumentOperations));
    // It can be widened to what it asked for, and no further.
    assert!(sandbox
        .registry
        .set_grants(id, &[Capability::DocumentRead])
        .is_err());
    sandbox
        .registry
        .set_grants(id, &[Capability::FilterPixels])
        .unwrap();
    assert!(matches!(
        sandbox.registry.active(id),
        Resolution::Available(_)
    ));
    assert!(!sandbox.registry.granted(id, Capability::DocumentOperations));
    // And narrowed again.
    sandbox.registry.set_grants(id, &[]).unwrap();
    assert!(!sandbox.registry.granted(id, Capability::FilterPixels));
    // An update shows what it adds.
    let mut manifest = example_manifest_json("shapes");
    manifest["version"] = json!("1.2.0");
    manifest["capabilities"] = json!([
        "filter.pixels",
        "document.operations",
        "ui.tool",
        "ui.panel"
    ]);
    manifest["panels"] =
        json!([{ "id": "p", "title": "P", "rows": [{ "kind": "text", "text": "hi" }] }]);
    let update = example_package(&manifest, Some(&example_module("shapes")), None);
    let seen = sandbox.registry.inspect(&update).unwrap();
    assert_eq!(seen.installed_version.as_deref(), Some("1.0.0"));
    let added: Vec<&str> = seen
        .new_capabilities
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert!(added.contains(&"ui.panel"), "{added:?}");
}

#[test]
fn an_update_installs_beside_the_old_version_and_survives_a_restart() {
    let mut sandbox = Sandbox::new();
    let v1 = sandbox.install_example("solarize");
    let mut manifest = example_manifest_json("solarize");
    manifest["version"] = json!("1.0.1");
    manifest["description"] = json!("A patch.");
    let v2_bytes = example_package(&manifest, Some(&example_module("solarize")), None);
    let report = sandbox
        .registry
        .install(&v2_bytes, None, &[Capability::FilterPixels], NOW)
        .unwrap();
    assert_eq!(report.updated_from.as_deref(), Some("1.0.0"));
    let id = "photoforge.example.solarize";
    let v2 = report.content_hash;
    assert_ne!(v1, v2, "a changed manifest is a different plugin version");
    // Both resolve; the newer one is what new work gets.
    assert!(matches!(
        sandbox.registry.resolve(id, &v1),
        Resolution::Available(_)
    ));
    assert!(matches!(
        sandbox.registry.resolve(id, &v2),
        Resolution::Available(_)
    ));
    let Resolution::Available(active) = sandbox.registry.active(id) else {
        panic!()
    };
    assert_eq!(active.content_hash, v2);
    // The version in use cannot be removed from under the plugin.
    assert!(sandbox.registry.remove_version(id, &v2).is_err());
    // Installing the same thing again changes nothing.
    sandbox
        .registry
        .install(&v2_bytes, None, &[Capability::FilterPixels], NOW)
        .unwrap();
    let summary = &sandbox.registry.list()[0];
    assert_eq!(summary.versions.len(), 2);

    // What the person decided is remembered across a restart.
    sandbox.registry.set_enabled(id, false).unwrap();
    sandbox
        .registry
        .remember(
            id,
            "filter:solarize",
            [("threshold".to_string(), 0.25)].into(),
        )
        .unwrap();
    sandbox.reopen();
    assert!(matches!(
        sandbox.registry.resolve(id, &v2),
        Resolution::Unavailable(Availability::Disabled)
    ));
    assert_eq!(
        sandbox.registry.remembered(id, "filter:solarize")["threshold"],
        0.25
    );
    assert_eq!(sandbox.registry.list()[0].versions.len(), 2);
    sandbox.registry.set_enabled(id, true).unwrap();
    assert!(matches!(
        sandbox.registry.resolve(id, &v1),
        Resolution::Available(_)
    ));
}

#[test]
fn a_plugin_file_altered_after_install_is_refused_not_trusted() {
    let sandbox = Sandbox::new();
    let hash = sandbox.install_example("solarize");
    let id = "photoforge.example.solarize";
    let path = sandbox
        .root
        .join("store")
        .join(id)
        .join(format!("{hash}.photoforge-plugin"));
    let original = std::fs::read(&path).unwrap();

    // A byte flipped in the stored package.
    let mut flipped = original.clone();
    let middle = flipped.len() / 2;
    flipped[middle] ^= 0x55;
    std::fs::write(&path, &flipped).unwrap();
    let mut reopened = Sandbox::reopen_registry(&sandbox);
    assert!(matches!(
        reopened.resolve(id, &hash),
        Resolution::Unavailable(Availability::Damaged { .. })
    ));

    // A different, perfectly valid plugin dropped in under this name.
    std::fs::write(&path, example_bytes("border")).unwrap();
    reopened = Sandbox::reopen_registry(&sandbox);
    let Resolution::Unavailable(Availability::Damaged { reason }) = reopened.resolve(id, &hash)
    else {
        panic!("a swapped file was accepted");
    };
    assert!(reason.contains("different plugin"), "{reason}");

    // The same plugin, rebuilt with a different module, under the original's name.
    let mut manifest = example_manifest_json("solarize");
    manifest["description"] = json!("Tampered.");
    std::fs::write(
        &path,
        example_package(&manifest, Some(&example_module("solarize")), None),
    )
    .unwrap();
    reopened = Sandbox::reopen_registry(&sandbox);
    let Resolution::Unavailable(Availability::Damaged { reason }) = reopened.resolve(id, &hash)
    else {
        panic!("a modified file was accepted");
    };
    assert!(reason.contains("not what was installed"), "{reason}");

    // Put it back and it works again.
    std::fs::write(&path, original).unwrap();
    assert!(matches!(
        Sandbox::reopen_registry(&sandbox).resolve(id, &hash),
        Resolution::Available(_)
    ));
}

impl Sandbox {
    fn reopen_registry(&self) -> PluginRegistry {
        PluginRegistry::open(&self.root)
    }
}

#[test]
fn a_damaged_list_of_plugins_is_reported_and_not_silently_replaced() {
    let mut sandbox = Sandbox::new();
    sandbox.install_example("solarize");
    let state = sandbox.root.join("state.json");
    let good = std::fs::read(&state).unwrap();

    for broken in [
        &b"{ not json"[..],
        b"",
        b"{\"version\": 99, \"plugins\": {}}",
        &vec![b' '; 2 * 1024 * 1024][..],
    ] {
        std::fs::write(&state, broken).unwrap();
        sandbox.reopen();
        assert!(
            sandbox.registry.state_warning().is_some(),
            "no warning for {broken:?}"
        );
        assert!(sandbox.registry.list().is_empty());
        // Reading it did not overwrite it: the person's file is still theirs.
        assert_eq!(std::fs::read(&state).unwrap(), broken);
    }
    std::fs::write(&state, &good).unwrap();
    sandbox.reopen();
    assert!(sandbox.registry.state_warning().is_none());
    assert_eq!(sandbox.registry.list().len(), 1);
}
