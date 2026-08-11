use image::{DynamicImage, GenericImageView, Rgba, RgbaImage};
use photoforge_lib::components::DeterministicEngine;
use photoforge_lib::domain::{EditOperation, PerspectiveCorners, RestorationEngine};
use photoforge_lib::mask::{
    apply_mask_operation, export_png, import_png, load_mask, rasterize, remap_between_chains,
    save_mask, select_magic_wand, CompositionMode, Connectivity, GeometryChain, GeometryStep,
    MaskBitmap, MaskFile, MaskMetadata, MaskOperation, MaskSnapshot, Point, SelectionShape,
    WandOptions,
};
use std::hint::black_box;
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

fn measure_samples<T>(name: &str, samples: usize, mut operation: impl FnMut() -> T) {
    if samples > 1 {
        let _ = black_box(operation());
    }
    let mut timings = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        black_box(operation());
        timings.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    timings.sort_by(f64::total_cmp);
    let median = timings[timings.len() / 2];
    println!(
        "METRIC {name} median={median:.3} ms min={:.3} ms max={:.3} ms samples={samples}",
        timings[0],
        timings[timings.len() - 1]
    );
}

fn benchmark_thumbnail(mask: &MaskBitmap, maximum_width: u32, maximum_height: u32) -> Vec<u8> {
    let scale = (maximum_width as f64 / mask.width() as f64)
        .min(maximum_height as f64 / mask.height() as f64);
    let width = ((mask.width() as f64 * scale).round() as u32).clamp(1, maximum_width);
    let height = ((mask.height() as f64 * scale).round() as u32).clamp(1, maximum_height);
    let mut output = vec![0_u8; (width * height) as usize];
    for output_y in 0..height {
        let source_top = output_y as f64 * mask.height() as f64 / height as f64;
        let source_bottom = (output_y + 1) as f64 * mask.height() as f64 / height as f64;
        for output_x in 0..width {
            let source_left = output_x as f64 * mask.width() as f64 / width as f64;
            let source_right = (output_x + 1) as f64 * mask.width() as f64 / width as f64;
            let mut weighted = 0.0;
            let mut total = 0.0;
            for source_y in
                source_top.floor() as u32..source_bottom.ceil().min(mask.height() as f64) as u32
            {
                let y_weight =
                    source_bottom.min(source_y as f64 + 1.0) - source_top.max(source_y as f64);
                for source_x in
                    source_left.floor() as u32..source_right.ceil().min(mask.width() as f64) as u32
                {
                    let x_weight =
                        source_right.min(source_x as f64 + 1.0) - source_left.max(source_x as f64);
                    let weight = x_weight * y_weight;
                    weighted += f64::from(mask.get(source_x, source_y)) * weight;
                    total += weight;
                }
            }
            output[(output_y * width + output_x) as usize] = (weighted / total.max(f64::EPSILON))
                .round()
                .clamp(0.0, 255.0)
                as u8;
        }
    }
    output
}

fn full_rectangle(width: u32, height: u32) -> MaskBitmap {
    rasterize(
        width,
        height,
        &SelectionShape::Rectangle {
            start: Point {
                x: width as f32 * 0.1,
                y: height as f32 * 0.1,
            },
            end: Point {
                x: width as f32 * 0.9,
                y: height as f32 * 0.9,
            },
        },
    )
    .unwrap()
}

fn main() {
    println!("PhotoForge mask benchmark 0.7.1");

    for (label, width, height, samples) in [
        ("1080p", 1_920, 1_080, 3),
        ("12mp", 4_000, 3_000, 2),
        ("24mp", 6_000, 4_000, 1),
    ] {
        let mask = full_rectangle(width, height);
        measure_samples(&format!("rectangle_{label}"), samples, || {
            full_rectangle(width, height)
        });
        let polygon = circle_points(500, width as f32, height as f32);
        measure_samples(&format!("polygon_500_points_{label}"), samples, || {
            rasterize(
                width,
                height,
                &SelectionShape::Polygon {
                    points: polygon.clone(),
                },
            )
            .unwrap()
        });
        let freehand_points = circle_points(180, width as f32, height as f32);
        measure_samples(&format!("freehand_180_points_{label}"), samples, || {
            rasterize(
                width,
                height,
                &SelectionShape::Freehand {
                    points: freehand_points.clone(),
                },
            )
            .unwrap()
        });
        for radius in [5, 25] {
            measure_samples(&format!("feather_radius_{radius}_{label}"), samples, || {
                apply_mask_operation(&mask, &MaskOperation::Feather { radius }, None).unwrap()
            });
        }
        measure_samples(&format!("expand_radius_24_{label}"), samples, || {
            apply_mask_operation(&mask, &MaskOperation::Expand { radius: 24 }, None).unwrap()
        });
        measure_samples(&format!("contract_radius_24_{label}"), samples, || {
            apply_mask_operation(&mask, &MaskOperation::Contract { radius: 24 }, None).unwrap()
        });
        measure_samples(&format!("refine_{label}"), samples, || {
            apply_mask_operation(
                &mask,
                &MaskOperation::Refine {
                    smooth: 3,
                    feather: 5,
                    contrast: 0.2,
                    shift_edge: -2,
                },
                None,
            )
            .unwrap()
        });
        measure_samples(&format!("thumbnail_area_average_{label}"), samples, || {
            benchmark_thumbnail(&mask, 96, 64)
        });

        let old_chain = GeometryChain::new(width, height, vec![]).unwrap();
        let crop_chain = GeometryChain::new(
            width,
            height,
            vec![GeometryStep::Crop {
                x: 0.1,
                y: 0.1,
                width: 0.8,
                height: 0.8,
            }],
        )
        .unwrap();
        measure_samples(&format!("mask_crop_transform_{label}"), samples, || {
            remap_between_chains(&mask, &old_chain, 0, &crop_chain, 1, None).unwrap()
        });
        let rotation_chain =
            GeometryChain::new(width, height, vec![GeometryStep::Rotate { degrees: 90 }]).unwrap();
        measure_samples(&format!("mask_rotate_90_{label}"), samples, || {
            remap_between_chains(&mask, &old_chain, 0, &rotation_chain, 1, None).unwrap()
        });
        let straighten_chain = GeometryChain::new(
            width,
            height,
            vec![GeometryStep::Straighten { degrees: 7.5 }],
        )
        .unwrap();
        measure_samples(&format!("mask_straighten_7_5_{label}"), samples, || {
            remap_between_chains(&mask, &old_chain, 0, &straighten_chain, 1, None).unwrap()
        });
        let perspective_chain = GeometryChain::new(
            width,
            height,
            vec![GeometryStep::Perspective {
                corners: PerspectiveCorners {
                    top_left: [0.04, 0.03],
                    top_right: [0.96, 0.01],
                    bottom_right: [0.98, 0.97],
                    bottom_left: [0.02, 0.99],
                },
            }],
        )
        .unwrap();
        measure_samples(&format!("mask_perspective_{label}"), samples, || {
            remap_between_chains(&mask, &old_chain, 0, &perspective_chain, 1, None).unwrap()
        });
        let lens_chain = GeometryChain::new(
            width,
            height,
            vec![GeometryStep::LensCorrection {
                distortion: 0.08,
                vignetting: 0.15,
                chromatic_aberration: 0.1,
            }],
        )
        .unwrap();
        measure_samples(&format!("mask_lens_distortion_{label}"), samples, || {
            remap_between_chains(&mask, &old_chain, 0, &lens_chain, 1, None).unwrap()
        });

        let single_color = RgbaImage::from_pixel(width, height, Rgba([120, 100, 80, 255]));
        measure_samples(&format!("magic_wand_single_color_{label}"), samples, || {
            select_magic_wand(
                &single_color,
                Point {
                    x: width as f32 * 0.5,
                    y: height as f32 * 0.5,
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
        println!(
            "MEMORY {label} mask_bytes={} rgba_fixture_bytes={} samples={samples}",
            mask.coverage().len(),
            single_color.as_raw().len()
        );
    }

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
    let png_path = temporary.path().join("benchmark-mask.png");
    measure("mask_png_save_6000x4000", || {
        export_png(&png_path, &rectangle).unwrap()
    });
    measure("mask_png_load_6000x4000", || import_png(&png_path).unwrap());

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
    let source_before = image.as_bytes().to_vec();
    let processed = measure("masked_brightness_4000x3000", || {
        DeterministicEngine.process(&image, &[operation]).unwrap()
    });
    let decontaminate = EditOperation::Masked {
        operation: Box::new(EditOperation::DecontaminateColors {
            enabled: true,
            strength: 0.5,
            radius: 4,
        }),
        mask: MaskSnapshot::encode(&freehand),
        invert: false,
        mask_id: Some("benchmark-refine".into()),
    };
    let decontaminated = measure("decontaminate_colors_4000x3000", || {
        DeterministicEngine
            .process(&image, &[decontaminate])
            .unwrap()
    });
    assert_eq!(decontaminated.dimensions(), image.dimensions());
    assert!(decontaminated
        .to_rgba8()
        .pixels()
        .all(|pixel| pixel.0[3] == 255));
    let export_path = temporary.path().join("masked-export.png");
    measure("masked_export_png_4000x3000", || {
        processed.save(&export_path).unwrap()
    });
    let decoded_export = image::open(&export_path).unwrap();
    assert_eq!(decoded_export.dimensions(), processed.dimensions());
    assert_eq!(image.as_bytes(), source_before.as_slice());

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
