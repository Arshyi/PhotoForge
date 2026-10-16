//! Makes before and after pictures with PhotoForge's own engine, for talks and reports.
//!
//! Usage: `cargo run --release --example demo_before_after -- <recipe.json> <output-dir>`
//!
//! A recipe names one input picture and a list of steps. Every step goes through the code the
//! application runs: an `op` is any typed `EditOperation` (the same JSON a workflow or a plan
//! carries) applied by the float pipeline; a `scale` is the layer transform of the Transform panel
//! rendered by the compositor (bilinear or nearest, nothing else is claimed); a `select` is a
//! colour-range selection, feathered, and an operation applied inside it as a masked operation.
//!
//! Nothing here is a model or a trained upscaler: PhotoForge has no resize filter beyond the
//! layer transform's two interpolations, and this tool does not add one.
//!
//! Output, in `<output-dir>`: `<name>_before.png`, `<name>_after.png` and `<name>.json` (the
//! recipe's steps, the sizes, the time each step took and any measurements). `prepare` steps are
//! applied to both pictures (cropping a screenshot's window frame away, for example) and
//! `before_steps` to the "before" alone (for instance a nearest-neighbour scale, so a before and
//! an after of different sizes can be compared at one size).
use image::RgbaImage;
use photoforge_lib::color::FloatImage;
use photoforge_lib::domain::EditOperation;
use photoforge_lib::error::AppError;
use photoforge_lib::high_precision;
use photoforge_lib::layers::{
    render_layers, BlendMode, Layer, LayerContent, LayerInterpolation, LayerMetadata,
    LayerTransform, PixelSource, RenderOptions,
};
use photoforge_lib::mask::{feather, select_color_range, ColorRangeOptions, MaskSnapshot, Point};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Deserialize)]
struct Recipe {
    name: String,
    input: String,
    #[serde(default)]
    prepare: Vec<Step>,
    steps: Vec<Step>,
    #[serde(default)]
    before_steps: Vec<Step>,
    /// Regions `(x, y, width, height)` of the prepared picture whose mean and standard deviation of
    /// luminance are reported before and after. Only meaningful when the steps keep the size.
    #[serde(default)]
    measure: Vec<Measure>,
}

#[derive(Debug, Deserialize)]
struct Measure {
    label: String,
    region: [u32; 4],
}

#[derive(Debug, Clone, Copy, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum Interpolation {
    #[default]
    Bilinear,
    Nearest,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Step {
    Op {
        operation: EditOperation,
    },
    Scale {
        factor: f32,
        #[serde(default)]
        interpolation: Interpolation,
    },
    Select {
        /// Sample points `[x, y]` in the picture as it is at this step.
        samples: Vec<[f32; 2]>,
        tolerance: f32,
        #[serde(default = "half")]
        luminance: f32,
        #[serde(default = "half")]
        hue: f32,
        #[serde(default = "half")]
        saturation: f32,
        #[serde(default)]
        feather: u32,
        #[serde(default)]
        invert: bool,
        operation: EditOperation,
    },
}

fn half() -> f32 {
    0.5
}

struct MapSource(HashMap<String, Arc<RgbaImage>>);

impl PixelSource for MapSource {
    fn resolve(&self, pixel_id: &str) -> Result<Arc<RgbaImage>, AppError> {
        self.0
            .get(pixel_id)
            .cloned()
            .ok_or_else(|| AppError::LayerPixelsMissing(pixel_id.to_string()))
    }
}

/// Scales a picture by rendering it as one layer, transformed, on a canvas of the new size: the
/// path the Transform panel's Scale fields take.
fn scale(
    image: &FloatImage,
    factor: f32,
    interpolation: Interpolation,
) -> Result<FloatImage, AppError> {
    if !factor.is_finite() || factor <= 0.0 {
        return Err(AppError::InvalidOperation(
            "a scale factor must be positive".into(),
        ));
    }
    let (width, height) = image.dimensions();
    let canvas_width = ((width as f32) * factor).round().max(1.0) as u32;
    let canvas_height = ((height as f32) * factor).round().max(1.0) as u32;
    let mut buffers = HashMap::new();
    buffers.insert("source".to_string(), Arc::new(image.to_rgba8()));
    let layer = Layer {
        id: "layer".into(),
        name: "layer".into(),
        visible: true,
        locked: false,
        opacity: 1.0,
        blend_mode: BlendMode::Normal,
        transform: LayerTransform {
            // Scaled about its own centre, then moved so its top-left corner is the canvas's.
            translate_x: (canvas_width as f32 - width as f32) / 2.0,
            translate_y: (canvas_height as f32 - height as f32) / 2.0,
            scale_x: canvas_width as f32 / width as f32,
            scale_y: canvas_height as f32 / height as f32,
            rotation_degrees: 0.0,
            flip_horizontal: false,
            flip_vertical: false,
            interpolation: match interpolation {
                Interpolation::Bilinear => LayerInterpolation::Bilinear,
                Interpolation::Nearest => LayerInterpolation::Nearest,
            },
        },
        mask: None,
        collapsed: false,
        clipped: false,
        metadata: LayerMetadata::default(),
        raw: None,
        origin: None,
        content: LayerContent::Pixel {
            pixel_id: "source".into(),
            width,
            height,
        },
    };
    let rendered = render_layers(
        &[layer],
        canvas_width,
        canvas_height,
        &MapSource(buffers),
        RenderOptions::default(),
    )?;
    FloatImage::from_rgba8(&rendered).map_err(|error| AppError::ColorPipeline(error.to_string()))
}

fn run_step(image: FloatImage, step: &Step) -> Result<(FloatImage, String), AppError> {
    match step {
        Step::Op { operation } => {
            let label = serde_json::to_string(operation).unwrap_or_default();
            Ok((high_precision::apply(&image, operation, None)?, label))
        }
        Step::Scale {
            factor,
            interpolation,
        } => {
            let label = format!("scale x{factor} ({interpolation:?})");
            Ok((scale(&image, *factor, *interpolation)?, label))
        }
        Step::Select {
            samples,
            tolerance,
            luminance,
            hue,
            saturation,
            feather: radius,
            invert,
            operation,
        } => {
            let points: Vec<Point> = samples
                .iter()
                .map(|[x, y]| Point { x: *x, y: *y })
                .collect();
            let mut mask = select_color_range(
                &image.to_rgba8(),
                &points,
                ColorRangeOptions {
                    tolerance: *tolerance,
                    luminance_sensitivity: *luminance,
                    hue_sensitivity: *hue,
                    saturation_sensitivity: *saturation,
                },
                None,
            )?;
            if *radius > 0 {
                mask = feather(&mask, *radius, None)?;
            }
            let covered = mask.coverage().iter().filter(|value| **value > 127).count();
            let label = format!(
                "select by colour ({} sample(s), tolerance {tolerance}, {:.1}% of the picture) then {}",
                samples.len(),
                100.0 * covered as f64 / mask.coverage().len().max(1) as f64,
                serde_json::to_string(operation).unwrap_or_default()
            );
            let masked = EditOperation::Masked {
                operation: Box::new(operation.clone()),
                mask: MaskSnapshot::encode(&mask),
                invert: *invert,
                mask_id: None,
            };
            Ok((high_precision::apply(&image, &masked, None)?, label))
        }
    }
}

fn run_steps(
    mut image: FloatImage,
    steps: &[Step],
    log: &mut Vec<serde_json::Value>,
    phase: &str,
) -> Result<FloatImage, AppError> {
    for step in steps {
        let started = Instant::now();
        let (next, label) = run_step(image, step)?;
        image = next;
        log.push(serde_json::json!({
            "phase": phase,
            "step": label,
            "ms": (started.elapsed().as_secs_f64() * 1000.0 * 10.0).round() / 10.0,
            "size": [image.width(), image.height()],
        }));
    }
    Ok(image)
}

/// Mean and standard deviation of Rec. 709 luminance of the encoded 8-bit values, 0 to 255.
fn luminance_stats(image: &FloatImage, region: [u32; 4]) -> Option<(f64, f64)> {
    let [x, y, w, h] = region;
    let rendered = image.to_rgba8();
    if w == 0 || h == 0 || x + w > rendered.width() || y + h > rendered.height() {
        return None;
    }
    let mut values = Vec::with_capacity((w * h) as usize);
    for row in y..y + h {
        for column in x..x + w {
            let p = rendered.get_pixel(column, row).0;
            values.push(
                0.2126 * f64::from(p[0]) + 0.7152 * f64::from(p[1]) + 0.0722 * f64::from(p[2]),
            );
        }
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64;
    Some((mean, variance.sqrt()))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(recipe_path), Some(out_dir)) = (args.next(), args.next()) else {
        eprintln!("usage: demo_before_after <recipe.json> <output-dir>");
        std::process::exit(2);
    };
    let recipe: Recipe =
        serde_json::from_slice(&std::fs::read(&recipe_path).expect("read the recipe"))
            .expect("the recipe is not valid JSON for this tool");
    let out_dir = PathBuf::from(out_dir);
    std::fs::create_dir_all(&out_dir).expect("create the output directory");

    let decoded = image::open(&recipe.input).expect("open the input picture");
    let mut log = Vec::new();
    let started = Instant::now();
    let source = FloatImage::from_dynamic(&decoded).expect("convert the input to float");
    let prepared = run_steps(source, &recipe.prepare, &mut log, "prepare").expect("prepare steps");
    let before = run_steps(prepared.clone(), &recipe.before_steps, &mut log, "before")
        .expect("before steps");
    let after = run_steps(prepared.clone(), &recipe.steps, &mut log, "after").expect("steps");

    let save = |image: &FloatImage, suffix: &str| -> PathBuf {
        let path = out_dir.join(format!("{}_{suffix}.png", recipe.name));
        image.to_rgba8().save(&path).expect("write a picture");
        path
    };
    let before_path = save(&before, "before");
    let after_path = save(&after, "after");

    let measurements: Vec<serde_json::Value> = recipe
        .measure
        .iter()
        .filter_map(|m| {
            let (b_mean, b_std) = luminance_stats(&prepared, m.region)?;
            let (a_mean, a_std) = luminance_stats(&after, m.region)?;
            Some(serde_json::json!({
                "label": m.label, "region": m.region,
                "before": { "mean": round(b_mean), "std": round(b_std) },
                "after": { "mean": round(a_mean), "std": round(a_std) },
            }))
        })
        .collect();
    let report = serde_json::json!({
        "name": recipe.name,
        "input": Path::new(&recipe.input).file_name().map(|n| n.to_string_lossy().into_owned()),
        "beforeSize": [before.width(), before.height()],
        "afterSize": [after.width(), after.height()],
        "totalMs": (started.elapsed().as_secs_f64() * 1000.0).round(),
        "steps": log,
        "measurements": measurements,
    });
    std::fs::write(
        out_dir.join(format!("{}.json", recipe.name)),
        serde_json::to_vec_pretty(&report).expect("encode the report"),
    )
    .expect("write the report");
    println!(
        "{}: {}x{} -> {}x{}  ({} , {})",
        recipe.name,
        before.width(),
        before.height(),
        after.width(),
        after.height(),
        before_path.display(),
        after_path.display()
    );
}

fn round(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
