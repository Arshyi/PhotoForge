//! Times one high-precision operation on one full frame.
//!
//! The layered benchmarks measure whole renders, which mixes compositing with
//! the operation under test. Deciding whether an operation is worth moving to
//! another backend needs the operation on its own.
//!
//! Usage: operation_benchmark MEGAPIXELS OPERATION [PARAMETER]
//!   OPERATION  blur | sharpen | brightness | denoise | localcontrast
use photoforge_lib::{
    color::{FloatImage, FloatRgba},
    domain::EditOperation,
    high_precision,
};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let megapixels: f64 = args.get(1).map_or(Ok(24.0), |v| v.parse())?;
    let name = args.get(2).map_or("blur", String::as_str);
    let parameter: f32 = args.get(3).map_or(Ok(4.0), |v| v.parse())?;

    let height = (megapixels * 1e6 / 1.3333).sqrt() as u32;
    let width = (f64::from(height) * 1.3333) as u32;
    let mut image = FloatImage::blank(width, height, FloatRgba::TRANSPARENT)?;
    for (index, pixel) in image.pixels_mut().iter_mut().enumerate() {
        let n = (index % 251) as f32 / 251.0;
        *pixel = FloatRgba::new(n, 1.0 - n, 0.5, 0.3 + n * 0.7);
    }

    let operation = match name {
        "blur" => EditOperation::GaussianBlur { radius: parameter },
        "sharpen" => EditOperation::Sharpen {
            strength: parameter,
        },
        "brightness" => EditOperation::Brightness { amount: parameter },
        "denoise" => EditOperation::Denoise {
            strength: parameter,
            preserve_edges: 0.5,
        },
        "localcontrast" => EditOperation::LocalContrast {
            strength: parameter,
            tile_size: 32,
            clip_limit: 2.0,
        },
        _ => return Err("unknown operation".into()),
    };

    // One warm run first: the first touch of a freshly allocated frame pays for
    // page faults that have nothing to do with the operation.
    let warm = high_precision::apply(&image, &operation, None)?;
    std::hint::black_box(&warm);

    let mut best = f64::MAX;
    for _ in 0..3 {
        let started = Instant::now();
        let result = high_precision::apply(&image, &operation, None)?;
        best = best.min(started.elapsed().as_secs_f64() * 1000.0);
        std::hint::black_box(&result);
    }
    println!(
        "{}",
        serde_json::json!({
            "operation": name,
            "parameter": parameter,
            "width": width,
            "height": height,
            "megapixels": f64::from(width) * f64::from(height) / 1e6,
            "bestMs": best,
            "threads": std::thread::available_parallelism().map_or(1, usize::from).min(8),
        })
    );
    Ok(())
}
