//! Reproducible Phase 8 layer and compositing benchmark.
//!
//! Run with `cargo run --release --example layer_benchmark`. Every figure is a
//! wall-clock measurement of the same deterministic compositor the application
//! uses; nothing here is estimated.

use image::{Rgba, RgbaImage};
use photoforge_lib::domain::EditOperation;
use photoforge_lib::error::AppError;
use photoforge_lib::layers::{
    decode_project, encode_project, render_layers, BlendMode, Layer, LayerContent, LayerDocument,
    LayerMask, LayerMetadata, LayerPixelStore, LayerTransform, PixelSource, RenderOptions,
    LAYER_SCHEMA_VERSION,
};
use photoforge_lib::mask::{MaskBitmap, MaskSnapshot};
use std::collections::HashMap;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

struct MapSource(HashMap<String, Arc<RgbaImage>>);

impl PixelSource for MapSource {
    fn resolve(&self, pixel_id: &str) -> Result<Arc<RgbaImage>, AppError> {
        self.0
            .get(pixel_id)
            .cloned()
            .ok_or_else(|| AppError::LayerPixelsMissing(pixel_id.to_string()))
    }
}

fn measure<T>(name: &str, operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = operation();
    println!(
        "METRIC {name} {:.3} ms",
        started.elapsed().as_secs_f64() * 1_000.0
    );
    result
}

fn gradient(width: u32, height: u32, seed: u8) -> RgbaImage {
    let mut image = RgbaImage::new(width, height);
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        *pixel = Rgba([
            ((x + u32::from(seed)) % 256) as u8,
            ((y + u32::from(seed) * 3) % 256) as u8,
            ((x + y) % 256) as u8,
            if seed == 0 { 255 } else { 200 },
        ]);
    }
    image
}

fn pixel_layer(id: &str, pixel_id: &str, width: u32, height: u32) -> Layer {
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
        content: LayerContent::Pixel {
            pixel_id: pixel_id.into(),
            width,
            height,
        },
    }
}

fn group_layer(id: &str, children: Vec<Layer>) -> Layer {
    Layer {
        content: LayerContent::Group {
            children,
            isolated: true,
        },
        ..pixel_layer(id, "unused", 1, 1)
    }
}

fn adjustment_layer(id: &str, operation: EditOperation) -> Layer {
    Layer {
        content: LayerContent::Adjustment {
            operation: Box::new(operation),
        },
        ..pixel_layer(id, "unused", 1, 1)
    }
}

fn document(width: u32, height: u32, layers: Vec<Layer>) -> LayerDocument {
    LayerDocument {
        schema_version: LAYER_SCHEMA_VERSION,
        canvas_width: width,
        canvas_height: height,
        layers,
        active_layer_id: None,
    }
}

fn build(width: u32, height: u32, count: usize) -> (LayerDocument, MapSource) {
    let mut buffers = HashMap::new();
    let mut layers = Vec::new();
    for index in 0..count {
        let pixel_id = format!("px{index}");
        buffers.insert(
            pixel_id.clone(),
            Arc::new(gradient(width, height, index as u8)),
        );
        let mut layer = pixel_layer(&format!("l{index}"), &pixel_id, width, height);
        if index > 0 {
            layer.opacity = 0.75;
            layer.blend_mode = BlendMode::Multiply;
        }
        layers.push(layer);
    }
    (document(width, height, layers), MapSource(buffers))
}

fn render(document: &LayerDocument, source: &MapSource, scale: f64) -> RgbaImage {
    render_layers(
        &document.layers,
        document.canvas_width,
        document.canvas_height,
        source,
        RenderOptions {
            scale,
            cancel: None,
        },
    )
    .expect("render succeeds")
}

fn scenario(label: &str, width: u32, height: u32, count: usize) {
    let (document, source) = build(width, height, count);
    let preview_scale = f64::from(1_600.min(width.max(height))) / f64::from(width.max(height));

    measure(&format!("{label}_full_render"), || {
        black_box(render(&document, &source, 1.0))
    });
    measure(&format!("{label}_preview_render"), || {
        black_box(render(&document, &source, preview_scale))
    });

    // Toggling visibility on the top layer and re-rendering is the interactive
    // path a user feels most often.
    let mut hidden = document.clone();
    if let Some(top) = hidden.layers.last_mut() {
        top.visible = false;
    }
    measure(&format!("{label}_visibility_toggle_preview"), || {
        black_box(render(&hidden, &source, preview_scale))
    });

    let mut faded = document.clone();
    if let Some(top) = faded.layers.last_mut() {
        top.opacity = 0.42;
    }
    measure(&format!("{label}_opacity_change_preview"), || {
        black_box(render(&faded, &source, preview_scale))
    });

    let mut reordered = document.clone();
    if reordered.layers.len() > 1 {
        let moved = reordered.layers.remove(0);
        reordered.layers.push(moved);
    }
    measure(&format!("{label}_reorder_preview"), || {
        black_box(render(&reordered, &source, preview_scale))
    });

    measure(&format!("{label}_flatten_full"), || {
        black_box(render(&document, &source, 1.0))
    });
}

fn main() {
    println!("PhotoForge layer benchmark");

    for (label, width, height) in [
        ("4000x3000", 4_000_u32, 3_000_u32),
        ("6000x4000", 6_000, 4_000),
    ] {
        for count in [1_usize, 10] {
            scenario(&format!("{label}_{count}layers"), width, height, count);
        }
    }

    // Layer-count scaling on a smaller canvas, so 50 and 100 layer documents
    // stay inside the store's documented memory ceiling.
    for count in [1_usize, 10, 50, 100] {
        scenario(&format!("1920x1080_{count}layers"), 1_920, 1_080, count);
    }

    // Nested groups and adjustment layers.
    let (base, source) = build(1_920, 1_080, 6);
    let grouped = document(
        1_920,
        1_080,
        vec![
            base.layers[0].clone(),
            group_layer(
                "outer",
                vec![
                    base.layers[1].clone(),
                    group_layer(
                        "inner",
                        vec![base.layers[2].clone(), base.layers[3].clone()],
                    ),
                ],
            ),
            adjustment_layer("adj1", EditOperation::Brightness { amount: 0.12 }),
            adjustment_layer("adj2", EditOperation::Contrast { amount: 0.18 }),
            adjustment_layer(
                "adj3",
                EditOperation::LocalContrast {
                    strength: 0.3,
                    tile_size: 32,
                    clip_limit: 1.4,
                },
            ),
        ],
    );
    measure("1920x1080_nested_groups_and_adjustments_full", || {
        black_box(render(&grouped, &source, 1.0))
    });

    // Large masks on every layer.
    let (mut masked, mask_source) = build(1_920, 1_080, 10);
    let mut bitmap = MaskBitmap::empty(1_920, 1_080).expect("mask allocates");
    for y in 0..1_080 {
        for x in 0..1_920 {
            bitmap.set(x, y, ((x + y) % 256) as u8);
        }
    }
    let snapshot = measure("mask_snapshot_encode_1920x1080", || {
        MaskSnapshot::encode(&bitmap)
    });
    for layer in &mut masked.layers {
        layer.mask = Some(LayerMask {
            snapshot: snapshot.clone(),
            enabled: true,
            inverted: false,
        });
    }
    measure("1920x1080_10layers_all_masked_full", || {
        black_box(render(&masked, &mask_source, 1.0))
    });

    // Project container round trip.
    let (project, project_source) = build(1_920, 1_080, 10);
    let borrowed: Vec<(String, &RgbaImage)> = project
        .referenced_pixel_ids()
        .into_iter()
        .map(|id| {
            let image = project_source.0.get(&id).expect("buffer exists").as_ref();
            (id, image)
        })
        .collect();
    let encoded = measure("project_save_1920x1080_10layers", || {
        encode_project(&project, &[], &borrowed, "0.8.0", "", "").expect("encodes")
    });
    println!(
        "METRIC project_bytes_1920x1080_10layers {} bytes",
        encoded.len()
    );
    measure("project_load_1920x1080_10layers", || {
        black_box(decode_project(&encoded).expect("decodes"))
    });

    // Pixel store registration, which generates the preview buffer.
    let mut store = LayerPixelStore::default();
    store.reset(6_000, 4_000).expect("canvas binds");
    measure("store_register_6000x4000", || {
        store
            .register(gradient(6_000, 4_000, 1))
            .expect("registers")
    });
    println!(
        "METRIC store_bytes_after_one_6000x4000 {} bytes",
        store.total_bytes()
    );
}
