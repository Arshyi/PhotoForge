use image::{DynamicImage, Rgba, RgbaImage};
use photoforge_lib::components::DeterministicEngine;
use photoforge_lib::domain::{EditOperation, RestorationEngine};
use photoforge_lib::mask::{
    apply_mask_operation, load_mask, rasterize, save_mask, select_magic_wand, CompositionMode,
    Connectivity, MaskBitmap, MaskFile, MaskMetadata, MaskOperation, MaskSnapshot, Point,
    SelectionShape, WandOptions,
};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn measure<T>(name: &str, operation: impl FnOnce() -> T) -> T {
    let started = Instant::now();
    let result = operation();
    println!(
        "METRIC {name} {:.3} ms",
        started.elapsed().as_secs_f64() * 1_000.0
    );
    result
}

fn circle_points(count: usize, width: f32, height: f32) -> Vec<Point> {
    (0..count)
        .map(|index| {
            let angle = std::f32::consts::TAU * index as f32 / count as f32;
            Point {
                x: width * 0.5 + angle.cos() * width * 0.36,
                y: height * 0.5 + angle.sin() * height * 0.36,
            }
        })
        .collect()
}

fn main() {
    println!("PhotoForge mask benchmark 0.7.0");

    let rectangle = measure("rectangle_6000x4000", || {
        rasterize(
            6_000,
            4_000,
            &SelectionShape::Rectangle {
                start: Point {
                    x: 300.25,
                    y: 220.5,
                },
                end: Point {
                    x: 5_700.75,
                    y: 3_760.5,
                },
            },
        )
        .unwrap()
    });

    let freehand_points = circle_points(180, 4_000.0, 3_000.0);
    let freehand = measure("freehand_180_points_4000x3000", || {
        rasterize(
            4_000,
            3_000,
            &SelectionShape::Freehand {
                points: freehand_points,
            },
        )
        .unwrap()
    });

    let polygon_points = circle_points(500, 1_920.0, 1_080.0);
    measure("polygon_500_points_1920x1080", || {
        rasterize(
            1_920,
            1_080,
            &SelectionShape::Polygon {
                points: polygon_points,
            },
        )
        .unwrap()
    });

    let brush_points: Vec<Point> = (0..500)
        .map(|index| Point {
            x: 100.0 + index as f32 * 7.5,
            y: 1_500.0 + (index as f32 * 0.08).sin() * 600.0,
        })
        .collect();
    measure("brush_500_points_4000x3000", || {
        rasterize(
            4_000,
            3_000,
            &SelectionShape::Brush {
                points: brush_points,
                diameter: 64.0,
                hardness: 0.75,
                opacity: 0.8,
            },
        )
        .unwrap()
    });

    for radius in [4, 16, 64] {
        measure(&format!("feather_radius_{radius}_4000x3000"), || {
            apply_mask_operation(&freehand, &MaskOperation::Feather { radius }, None).unwrap()
        });
    }
    measure("expand_radius_24_4000x3000", || {
        apply_mask_operation(&freehand, &MaskOperation::Expand { radius: 24 }, None).unwrap()
    });
    measure("contract_radius_24_4000x3000", || {
        apply_mask_operation(&freehand, &MaskOperation::Contract { radius: 24 }, None).unwrap()
    });

    let single_color = RgbaImage::from_pixel(6_000, 4_000, Rgba([120, 100, 80, 255]));
    measure("magic_wand_single_color_6000x4000", || {
        select_magic_wand(
            &single_color,
            Point {
                x: 3_000.0,
                y: 2_000.0,
            },
            WandOptions {
                tolerance: 0.12,
                connectivity: Connectivity::Eight,
                anti_alias: true,
                contiguous: true,
            },
            None,
        )
        .unwrap()
    });

    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("benchmark.photoforge-mask.json");
    let document = MaskFile::new(
        "benchmark".into(),
        "Benchmark".into(),
        MaskSnapshot::encode(&rectangle),
        MaskMetadata::default(),
    );
    measure("mask_save_6000x4000", || {
        save_mask(&path, &document).unwrap()
    });
    measure("mask_load_6000x4000", || load_mask(&path).unwrap());

    let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
        4_000,
        3_000,
        Rgba([80, 90, 100, 255]),
    ));
    let operation = EditOperation::Masked {
        operation: Box::new(EditOperation::Brightness { amount: 0.15 }),
        mask: MaskSnapshot::encode(&freehand),
        invert: false,
        mask_id: Some("benchmark".into()),
    };
    measure("masked_brightness_4000x3000", || {
        DeterministicEngine.process(&image, &[operation]).unwrap()
    });

    let cancellation_mask = MaskBitmap::full(6_000, 4_000).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let worker = std::thread::spawn(move || {
        apply_mask_operation(
            &cancellation_mask,
            &MaskOperation::Feather { radius: 128 },
            Some(worker_cancelled.as_ref()),
        )
    });
    std::thread::sleep(Duration::from_millis(10));
    let signal = Instant::now();
    cancelled.store(true, std::sync::atomic::Ordering::Release);
    let result = worker.join().unwrap();
    println!(
        "METRIC cancellation_acknowledgement {:.3} ms ({})",
        signal.elapsed().as_secs_f64() * 1_000.0,
        if result.is_err() {
            "cancelled"
        } else {
            "completed"
        }
    );

    println!(
        "MEMORY raw_mask_bytes={} rgba_fixture_bytes={} queue_is_bounded_by_pixels=true composition_mode={:?}",
        rectangle.coverage().len(),
        single_color.as_raw().len(),
        CompositionMode::Replace
    );
}
