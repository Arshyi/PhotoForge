//! Scores restoration operations against known-clean fixtures.
//!
//! Each case degrades a clean scene, restores it, and reports how close the
//! result is to the original. Two numbers matter together and neither alone:
//! `flatNoise` falls when noise is removed, `edges` falls when detail is
//! destroyed, and an algorithm that wins on one by sacrificing the other has
//! not improved anything.
//!
//! Usage: restoration_benchmark [SIZE]
use photoforge_lib::{
    color::FloatImage, domain::EditOperation, fixtures, high_precision,
    image_processing_kernels::BlurKernel, metrics,
};
use std::time::Instant;

struct Case {
    name: &'static str,
    clean: FloatImage,
    degraded: FloatImage,
    operation: EditOperation,
}

fn score(case: &Case) -> serde_json::Value {
    let started = Instant::now();
    let restored = high_precision::apply(&case.degraded, &case.operation, None)
        .expect("restoration operation");
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    serde_json::json!({
        "case": case.name,
        "operation": case.operation.kind(),
        "ms": (elapsed * 10.0).round() / 10.0,
        // Before: how bad the degraded image is. After: how bad the result is.
        // A restoration that helps moves psnr and ssim up and flatNoise down
        // without moving edges far from the degraded image's own edge score.
        "psnrBefore": round(metrics::psnr(&case.clean, &case.degraded)),
        "psnrAfter": round(metrics::psnr(&case.clean, &restored)),
        "ssimBefore": round(metrics::ssim(&case.clean, &case.degraded)),
        "ssimAfter": round(metrics::ssim(&case.clean, &restored)),
        "flatNoiseBefore": round(metrics::flat_area_noise(&case.clean, &case.degraded)),
        "flatNoiseAfter": round(metrics::flat_area_noise(&case.clean, &restored)),
        "edgesBefore": round(metrics::edge_retention(&case.clean, &case.degraded)),
        "edgesAfter": round(metrics::edge_retention(&case.clean, &restored)),
    })
}

fn round(value: f64) -> f64 {
    if value.is_finite() {
        (value * 10000.0).round() / 10000.0
    } else {
        -1.0
    }
}

fn main() {
    let size: u32 = std::env::args()
        .nth(1)
        .and_then(|v| v.parse().ok())
        .unwrap_or(256);
    let clean = fixtures::scene(size, size);

    let cases = vec![
        Case {
            name: "gaussian-noise-light",
            clean: clean.clone(),
            degraded: fixtures::gaussian_noise(&clean, 0.03, 101),
            operation: EditOperation::Denoise {
                strength: 0.5,
                preserve_edges: 0.5,
                color: 0.5,
            },
        },
        Case {
            name: "gaussian-noise-heavy",
            clean: clean.clone(),
            degraded: fixtures::gaussian_noise(&clean, 0.08, 102),
            operation: EditOperation::Denoise {
                strength: 0.8,
                preserve_edges: 0.5,
                color: 0.6,
            },
        },
        Case {
            name: "chroma-noise",
            clean: clean.clone(),
            degraded: fixtures::chroma_noise(&clean, 0.07, 103),
            operation: EditOperation::Denoise {
                strength: 0.8,
                preserve_edges: 0.5,
                color: 0.9,
            },
        },
        Case {
            name: "impulse-noise",
            clean: clean.clone(),
            degraded: fixtures::impulse_noise(&clean, 0.02, 104),
            operation: EditOperation::RemoveDefects {
                strength: 1.0,
                threshold: 3.0,
            },
        },
        Case {
            name: "block-artifacts",
            clean: clean.clone(),
            degraded: fixtures::block_artifacts(&clean, 0.8),
            operation: EditOperation::Deblock { strength: 0.7 },
        },
        Case {
            name: "defocus-blur",
            clean: clean.clone(),
            degraded: fixtures::defocus_blur(&clean, 1.5),
            // The kernel the fixture actually applied, so this measures a
            // deconvolution against a known blur rather than a guessed one.
            operation: EditOperation::Deconvolve {
                kernel: BlurKernel::Defocus { radius: 1.5 },
                iterations: 20,
                damping: 0.1,
            },
        },
        Case {
            name: "motion-blur",
            clean: clean.clone(),
            degraded: fixtures::motion_blur(&clean, 20.0, 7.0),
            operation: EditOperation::Deconvolve {
                kernel: BlurKernel::Motion {
                    angle_degrees: 20.0,
                    distance: 7.0,
                },
                iterations: 20,
                damping: 0.1,
            },
        },
        Case {
            name: "uneven-illumination",
            clean: clean.clone(),
            degraded: fixtures::uneven_illumination(&clean, 0.6),
            // Radius 96, not a small number: the falloff is image-wide, and a
            // radius sweep showed this tool only begins to help once its
            // background estimate is comparable to the scale of the defect.
            operation: EditOperation::UnevenLightingCorrection {
                strength: 0.8,
                radius: 96.0,
            },
        },
        Case {
            name: "sensor-defects",
            clean: clean.clone(),
            degraded: fixtures::sensor_defects(&clean, 60, 105).0,
            operation: EditOperation::RemoveDefects {
                strength: 1.0,
                threshold: 3.0,
            },
        },
        Case {
            name: "dust",
            clean: clean.clone(),
            degraded: fixtures::dust(&clean, 40, 106).0,
            operation: EditOperation::RemoveDefects {
                strength: 1.0,
                threshold: 3.0,
            },
        },
    ];

    let results: Vec<_> = cases.iter().map(score).collect();
    println!("{}", serde_json::json!({ "size": size, "cases": results }));
}
