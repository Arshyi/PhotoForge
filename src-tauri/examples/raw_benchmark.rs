//! Reproducible RAW development benchmark.
//!
//! Run with `cargo run --release --example raw_benchmark`. Every figure is a
//! wall-clock measurement of the same code the application runs; nothing here
//! is estimated. Sensor sizes are synthetic so the benchmark is reproducible
//! anywhere, and a real camera file can be added with
//! `--path <file>` to check the synthetic figures against one.

use photoforge_lib::color::{DevelopmentParameters, WhiteBalance};
use photoforge_lib::raw::demosaic::{self, Quality};
use photoforge_lib::raw::develop::{develop_sensor, RenderScale};
use photoforge_lib::raw::dng::{self, fixtures::DngBuilder};
use std::hint::black_box;
use std::time::Instant;

fn measure<T>(name: &str, operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = operation();
    println!(
        "METRIC {name} {:.1} ms",
        started.elapsed().as_secs_f64() * 1_000.0
    );
    result
}

/// A sensor-shaped gradient with noise-like variation, so demosaicing has real
/// structure to interpolate rather than a flat field it can shortcut.
fn sensor_bytes(width: u32, height: u32) -> Vec<u8> {
    let samples: Vec<u16> = (0..u64::from(width) * u64::from(height))
        .map(|index| {
            let x = (index % u64::from(width)) as u32;
            let y = (index / u64::from(width)) as u32;
            let base = 512 + ((x / 8 + y / 8) % 3000) as u16;
            base.wrapping_add(((x.wrapping_mul(2654435761) ^ y) % 97) as u16)
        })
        .collect();
    DngBuilder::new(width, height, samples)
        .levels(vec![512, 512, 512, 514], (2, 2), 3800)
        .neutral([0.55, 1.0, 0.72])
        .matrix([0.72, 0.18, 0.10, 0.16, 0.74, 0.10, 0.09, 0.20, 0.71])
        .build()
}

fn scenario(label: &str, width: u32, height: u32) {
    let megapixels = f64::from(width) * f64::from(height) / 1e6;
    println!("--- {label} ({width}x{height}, {megapixels:.1} MP) ---");

    let bytes = measure(&format!("{label}_write_fixture"), || {
        sensor_bytes(width, height)
    });
    println!("METRIC {label}_file_bytes {}", bytes.len());

    let decoded = measure(&format!("{label}_decode"), || dng::decode(&bytes));
    let sensor = match decoded {
        Ok(sensor) => sensor,
        Err(error) => {
            // A refusal is a measurement too: PhotoForge bounds every decode at
            // the application-wide pixel ceiling, and a sensor above it is
            // declined rather than allowed to allocate.
            println!("METRIC {label}_refused 1");
            println!("NOTE {label} refused: {error}");
            println!();
            return;
        }
    };
    println!(
        "METRIC {label}_sensor_bytes {}",
        std::mem::size_of_val(sensor.data.as_slice())
    );

    let normalized = measure(&format!("{label}_normalize"), || {
        demosaic::normalize(&sensor)
    });
    println!(
        "METRIC {label}_normalized_bytes {}",
        std::mem::size_of_val(normalized.data.as_slice())
    );

    for quality in [Quality::Fast, Quality::High] {
        let rgb = measure(&format!("{label}_demosaic_{}", quality.name()), || {
            black_box(demosaic::demosaic(&normalized, quality))
        });
        println!(
            "METRIC {label}_demosaic_{}_bytes {}",
            quality.name(),
            std::mem::size_of_val(rgb.as_slice())
        );
    }

    let parameters = DevelopmentParameters::default();
    measure(&format!("{label}_initial_preview"), || {
        black_box(develop_sensor(&sensor, &parameters, RenderScale::Preview).expect("preview"))
    });

    // The interactive edits: each is a full re-development, because there is no
    // composite cache. These are the numbers a slider actually costs.
    let white_balance = DevelopmentParameters {
        white_balance: WhiteBalance::Custom {
            multipliers: [1.8, 1.0, 1.4],
        },
        ..DevelopmentParameters::default()
    };
    measure(&format!("{label}_white_balance_preview"), || {
        black_box(develop_sensor(&sensor, &white_balance, RenderScale::Preview).expect("wb"))
    });

    let exposed = DevelopmentParameters {
        exposure_ev: 0.75,
        ..DevelopmentParameters::default()
    };
    measure(&format!("{label}_exposure_preview"), || {
        black_box(develop_sensor(&sensor, &exposed, RenderScale::Preview).expect("exposure"))
    });

    let toned = DevelopmentParameters {
        contrast: 0.35,
        highlights: -0.4,
        shadows: 0.5,
        ..DevelopmentParameters::default()
    };
    measure(&format!("{label}_tone_preview"), || {
        black_box(develop_sensor(&sensor, &toned, RenderScale::Preview).expect("tone"))
    });

    let full = measure(&format!("{label}_full_render"), || {
        develop_sensor(&sensor, &parameters, RenderScale::Full).expect("full")
    });
    println!(
        "METRIC {label}_working_image_bytes {}",
        std::mem::size_of_val(full.image.pixels())
    );

    let encoded = measure(&format!("{label}_encode_png16"), || {
        full.image.encode_png16().expect("encodes")
    });
    println!("METRIC {label}_png16_bytes {}", encoded.len());

    // Peak simultaneous cost of one full render, counted rather than guessed.
    let sensor_bytes_count = std::mem::size_of_val(sensor.data.as_slice());
    let normalized_bytes = std::mem::size_of_val(normalized.data.as_slice());
    let rgb_bytes = std::mem::size_of_val(normalized.data.as_slice()) * 3;
    let working_bytes = std::mem::size_of_val(full.image.pixels());
    println!(
        "METRIC {label}_peak_pipeline_bytes {}",
        sensor_bytes_count + normalized_bytes + rgb_bytes + working_bytes + encoded.len()
    );
    println!();
}

fn main() {
    println!("PhotoForge RAW development benchmark");
    println!();

    // The three sizes the brief asks for, plus a small one for scale.
    scenario("6mp", 3000, 2000);
    scenario("12mp", 4240, 2832);
    scenario("24mp", 6000, 4000);
    scenario("45mp", 8256, 5504);

    // An optional real camera file, for checking the synthetic figures.
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument != "--path" {
            continue;
        }
        let Some(path) = arguments.next() else { break };
        let Ok(bytes) = std::fs::read(&path) else {
            println!("could not read {path}");
            continue;
        };
        println!("--- real file: {path} ---");
        println!("METRIC real_file_bytes {}", bytes.len());
        let sensor = measure("real_decode", || dng::decode(&bytes).expect("decodes"));
        println!("METRIC real_dimensions {}x{}", sensor.width, sensor.height);
        let parameters = DevelopmentParameters::default();
        measure("real_initial_preview", || {
            black_box(develop_sensor(&sensor, &parameters, RenderScale::Preview).expect("preview"))
        });
        measure("real_exposure_preview", || {
            let exposed = DevelopmentParameters {
                exposure_ev: 0.75,
                ..DevelopmentParameters::default()
            };
            black_box(develop_sensor(&sensor, &exposed, RenderScale::Preview).expect("exposure"))
        });
        let full = measure("real_full_render", || {
            develop_sensor(&sensor, &parameters, RenderScale::Full).expect("full")
        });
        let encoded = measure("real_encode_png16", || {
            full.image.encode_png16().expect("encodes")
        });
        println!("METRIC real_png16_bytes {}", encoded.len());
        println!();
    }
}
