//! Decodes a RAW file and reports what PhotoForge made of it.
//!
//! Development aid for checking the decoder against real camera files, which
//! are too large to commit. Run with:
//! `cargo run --release --example raw_probe -- <file> [more files...]`

use photoforge_lib::color::DevelopmentParameters;
use photoforge_lib::raw::develop::{develop_sensor, RenderScale};
use photoforge_lib::raw::{demosaic, dng};
use std::time::Instant;

fn main() {
    for path in std::env::args().skip(1) {
        println!("=== {path} ===");
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) => {
                println!("  read failed: {error}");
                continue;
            }
        };
        println!("  file bytes: {}", bytes.len());

        let started = Instant::now();
        let sensor = match dng::decode(&bytes) {
            Ok(sensor) => sensor,
            Err(error) => {
                println!("  DECODE REFUSED: {error}");
                continue;
            }
        };
        let decode_ms = started.elapsed().as_secs_f64() * 1000.0;
        println!(
            "  decoded {}x{} in {decode_ms:.1} ms",
            sensor.width, sensor.height
        );
        println!(
            "  {} bits, CFA {}, black {:?}, white {}",
            sensor.bits_per_sample,
            sensor.cfa.name(),
            sensor.black_level,
            sensor.white_level
        );
        println!(
            "  camera: {:?} {:?} iso={:?} lens={:?}",
            sensor.metadata.manufacturer,
            sensor.metadata.model,
            sensor.metadata.iso,
            sensor.metadata.lens
        );
        println!(
            "  shutter={:?} aperture={:?} focal={:?} taken={:?}",
            sensor.metadata.shutter_speed_seconds,
            sensor.metadata.aperture,
            sensor.metadata.focal_length_mm,
            sensor.metadata.capture_time
        );
        println!(
            "  orientation={} active_area={:?} crop={:?}",
            sensor.orientation, sensor.active_area, sensor.default_crop
        );
        println!(
            "  as-shot neutral: {:?}  colour matrix: {}",
            sensor.as_shot_neutral,
            if sensor.camera_to_srgb.is_some() {
                "present"
            } else {
                "absent"
            }
        );

        let minimum = sensor.data.iter().copied().min().unwrap_or(0);
        let maximum = sensor.data.iter().copied().max().unwrap_or(0);
        let sum: u64 = sensor.data.iter().map(|value| u64::from(*value)).sum();
        println!(
            "  samples: min {minimum}, max {maximum}, mean {:.1}",
            sum as f64 / sensor.data.len() as f64
        );
        // A checksum of the sensor plane, so two encodings of the same shot can
        // be compared byte for byte.
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for value in &sensor.data {
            hash ^= u64::from(*value);
            hash = hash.wrapping_mul(0x1000_0000_01b3);
        }
        println!("  sensor plane fnv1a64: {hash:016x}");

        for scale in [RenderScale::Preview, RenderScale::Full] {
            let started = Instant::now();
            match develop_sensor(&sensor, &DevelopmentParameters::default(), scale) {
                Ok(developed) => {
                    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
                    let pixels = developed.image.pixels();
                    let mean: f64 = pixels.iter().map(|p| f64::from(p.green)).sum::<f64>()
                        / pixels.len() as f64;
                    let over: usize = pixels.iter().filter(|p| p.green > 1.0).count();
                    println!(
                        "  {:?}: {}x{} in {elapsed:.1} ms, demosaic {}, wb {:?}, managed {}, mean G {mean:.4}, above-white {over}",
                        scale,
                        developed.image.width(),
                        developed.image.height(),
                        developed.demosaic.name(),
                        developed.multipliers,
                        developed.color_managed,
                    );
                    let _ = demosaic::Quality::default();
                }
                Err(error) => println!("  {scale:?} development failed: {error}"),
            }
        }
    }
}
