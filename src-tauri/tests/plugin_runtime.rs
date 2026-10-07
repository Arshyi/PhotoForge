//! The plugin runtime, run for real: the example plugins produce what they claim, a
//! tiled filter equals a whole one, and every kind of misbehaving module is
//! contained with a clear reason.
//!
//! The modules are WebAssembly text in `plugins/examples` and `plugins/adversarial`,
//! compiled here by the `wat` crate — a test dependency, not a production one.
#![cfg(feature = "plugins")]
mod common;
use common::*;
use photoforge_lib::color::{FloatImage, FloatRgba};
use photoforge_lib::plugins::filter::{apply, check_parameters, plan_tiles, verify_locality};
use photoforge_lib::plugins::job::TileJob;
use photoforge_lib::plugins::limits::{self, call_limits, CallLimits, MIB};
use photoforge_lib::plugins::manifest::Locality;
use photoforge_lib::plugins::{PluginError, Runtime};
use photoforge_lib::source::Rect;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::new().expect("the runtime starts"))
}

const ALLOWANCE: u64 = 256 * MIB;

/// Runs the example filter `name` with `parameters` over `source`, tiled.
fn run_example(name: &str, parameters: &[f64], source: &FloatImage, tile: u32) -> FloatImage {
    let manifest = example_manifest(name);
    let compiled = runtime().compile(&example_module(name)).unwrap();
    apply(
        runtime(),
        &compiled,
        &manifest.filters[0],
        0,
        parameters,
        source,
        ALLOWANCE,
        tile,
        None,
    )
    .unwrap()
    .image
}

fn run_adversarial(
    name: &str,
    locality: Locality,
    image: &FloatImage,
) -> Result<FloatImage, PluginError> {
    let compiled = runtime().compile(&adversarial(name))?;
    apply(
        runtime(),
        &compiled,
        &plain_decl(locality),
        0,
        &[],
        image,
        ALLOWANCE,
        16,
        None,
    )
    .map(|run| run.image)
}

fn same(a: &FloatImage, b: &FloatImage) -> bool {
    bits(a) == bits(b)
}

// ---- the examples do what they say ---------------------------------------------------

#[test]
fn solarize_inverts_the_channels_above_the_threshold_and_keeps_alpha() {
    let source = pattern(37, 29);
    let out = run_example("solarize", &[0.5], &source, 16);
    for (before, after) in source.pixels().iter().zip(out.pixels()) {
        let expect = |v: f32| if v > 0.5 { 1.0 - v } else { v };
        assert_eq!(after.red.to_bits(), expect(before.red).to_bits());
        assert_eq!(after.green.to_bits(), expect(before.green).to_bits());
        assert_eq!(after.blue.to_bits(), expect(before.blue).to_bits());
        assert_eq!(after.alpha.to_bits(), before.alpha.to_bits());
    }
}

#[test]
fn the_border_covers_exactly_the_outer_pixels_and_nothing_else() {
    let source = pattern(40, 30);
    let out = run_example("border", &[5.0, 1.0, 0.0, 0.5], &source, 16);
    for y in 0..30u32 {
        for x in 0..40u32 {
            let p = out.get(x, y).unwrap();
            let edge = x < 5 || y < 5 || x >= 35 || y >= 25;
            if edge {
                assert_eq!(
                    (p.red, p.green, p.blue, p.alpha),
                    (1.0, 0.0, 0.5, 1.0),
                    "({x},{y})"
                );
            } else {
                assert_eq!(p, source.get(x, y).unwrap(), "({x},{y}) was changed");
            }
        }
    }
    // Zero width is the identity.
    assert!(same(
        &run_example("border", &[0.0, 1.0, 0.0, 0.5], &source, 16),
        &source
    ));
}

#[test]
fn channel_swap_permutes_the_channels() {
    let source = pattern(20, 11);
    let orders: [[usize; 3]; 5] = [[2, 1, 0], [1, 0, 2], [2, 0, 1], [0, 2, 1], [1, 2, 0]];
    for (choice, order) in orders.iter().enumerate() {
        let out = run_example("channel_swap", &[choice as f64], &source, 8);
        for (before, after) in source.pixels().iter().zip(out.pixels()) {
            let channels = [before.red, before.green, before.blue];
            assert_eq!(after.red.to_bits(), channels[order[0]].to_bits());
            assert_eq!(after.green.to_bits(), channels[order[1]].to_bits());
            assert_eq!(after.blue.to_bits(), channels[order[2]].to_bits());
            assert_eq!(after.alpha.to_bits(), before.alpha.to_bits());
        }
    }
}

#[test]
fn the_shape_generator_draws_a_centred_shape_and_ignores_the_input() {
    let blank = FloatImage::blank(64, 48, FloatRgba::new(0.2, 0.4, 0.6, 1.0)).unwrap();
    let busy = pattern(64, 48);
    for (shape, inside, outside) in [
        (0.0, (32, 24), (2, 2)),   // circle: centre in, corner out
        (1.0, (32, 24), (2, 2)),   // square
        (2.0, (32, 24), (40, 40)), // diamond: a far diagonal point is out
    ] {
        let a = run_example("shapes", &[shape, 0.6], &blank, 16);
        let b = run_example("shapes", &[shape, 0.6], &busy, 16);
        assert!(
            same(&a, &b),
            "shape {shape}: the result depends on the input"
        );
        assert_eq!(
            a.get(inside.0, inside.1).unwrap(),
            FloatRgba::new(1.0, 1.0, 1.0, 1.0)
        );
        assert_eq!(
            a.get(outside.0, outside.1).unwrap(),
            FloatRgba::new(0.0, 0.0, 0.0, 0.0)
        );
    }
    // A bigger size covers more.
    let covered = |size: f64| {
        run_example("shapes", &[0.0, size], &blank, 16)
            .pixels()
            .iter()
            .filter(|p| p.alpha > 0.5)
            .count()
    };
    assert!(covered(0.9) > covered(0.3) && covered(0.3) > covered(0.1));
}

/// The reference the box blur is compared against, in the same order of operations.
fn reference_box_blur(source: &FloatImage, radius: i64) -> FloatImage {
    let (w, h) = (source.width() as i64, source.height() as i64);
    let mut out = FloatImage::blank(w as u32, h as u32, FloatRgba::TRANSPARENT).unwrap();
    for y in 0..h {
        for x in 0..w {
            let (mut r, mut g, mut b, mut a, mut n) = (0f32, 0f32, 0f32, 0f32, 0u32);
            for ny in (y - radius).max(0)..=(y + radius).min(h - 1) {
                for nx in (x - radius).max(0)..=(x + radius).min(w - 1) {
                    let p = source.get(nx as u32, ny as u32).unwrap();
                    r += p.red;
                    g += p.green;
                    b += p.blue;
                    a += p.alpha;
                    n += 1;
                }
            }
            let n = n as f32;
            out.pixels_mut()[(y * w + x) as usize] = FloatRgba::new(r / n, g / n, b / n, a / n);
        }
    }
    out
}

#[test]
fn the_box_blur_matches_a_reference_and_does_not_show_tile_edges() {
    let source = pattern(53, 41);
    for radius in [1, 2, 4] {
        let reference = reference_box_blur(&source, radius);
        // Whole image, and several tilings that put edges in awkward places.
        for tile in [1000, 64, 16, 7, 3] {
            let out = run_example("boxblur", &[radius as f64], &source, tile);
            assert!(same(&out, &reference), "radius {radius}, tile {tile}");
        }
    }
}

// ---- locality is verified, not believed ------------------------------------------------

#[test]
fn honest_declarations_pass_the_locality_check_and_a_lie_is_caught() {
    let source = pattern(48, 36);
    // Pointwise and local examples, run whole and tiled.
    for name in ["solarize", "border", "channel_swap", "shapes", "boxblur"] {
        let manifest = example_manifest(name);
        let compiled = runtime().compile(&example_module(name)).unwrap();
        let report = verify_locality(
            runtime(),
            &compiled,
            &manifest.filters[0],
            0,
            &source,
            ALLOWANCE,
            16,
        )
        .unwrap();
        assert!(report.honest, "{name}: {report:?}");
    }
    // The same blur declaring one pixel of reach while using four.
    let compiled = runtime().compile(&adversarial("lying_locality")).unwrap();
    let mut decl = plain_decl(Locality::Local { radius: 1 });
    decl.parameters = Vec::new();
    let report = verify_locality(runtime(), &compiled, &decl, 0, &source, ALLOWANCE, 16);
    // `lying_locality` takes no parameters, so it blurs by its own constant 4.
    let report = report.unwrap();
    assert!(
        !report.honest,
        "a filter that reads beyond its declared radius was passed"
    );
    assert!(report.differing_pixels > 0);
    assert!(report.first_difference.is_some());
    // Declaring the truth makes it pass.
    let truthful = plain_decl(Locality::Local { radius: 4 });
    assert!(
        verify_locality(runtime(), &compiled, &truthful, 0, &source, ALLOWANCE, 16)
            .unwrap()
            .honest
    );
}

#[test]
fn the_same_input_gives_the_same_bits_every_time_and_no_state_survives_a_call() {
    let source = pattern(33, 21);
    let first = run_example("boxblur", &[3.0], &source, 8);
    let second = run_example("boxblur", &[3.0], &source, 8);
    assert!(same(&first, &second));
    // Instances are fresh, so a plugin cannot carry anything from tile to tile: the
    // honest copy gives back its input exactly however many tiles there were.
    let out = run_adversarial("honest_copy", Locality::Pointwise, &source).unwrap();
    assert!(same(&out, &source));
}

// ---- every way to misbehave is contained --------------------------------------------------

fn expect_error(name: &str, check: impl FnOnce(&PluginError)) {
    let image = pattern(16, 16);
    let started = Instant::now();
    let error = run_adversarial(name, Locality::Pointwise, &image)
        .expect_err(&format!("{name} was allowed to succeed"));
    check(&error);
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "{name} took {:?}",
        started.elapsed()
    );
}

#[test]
fn work_that_never_ends_is_stopped_by_fuel() {
    expect_error("infinite_loop", |e| {
        assert_eq!(*e, PluginError::OutOfFuel, "{e}")
    });
    // Including in its start function, before any filter is called.
    expect_error("start_loop", |e| {
        assert_eq!(*e, PluginError::OutOfFuel, "{e}")
    });
}

#[test]
fn memory_is_capped_including_the_size_a_module_declares() {
    expect_error("memory_hog", |e| match e {
        PluginError::Trap(message) => assert!(message.contains("more than"), "{message}"),
        other => panic!("{other:?}"),
    });
    expect_error("huge_initial_memory", |e| match e {
        PluginError::Trap(message) => assert!(message.contains("MiB"), "{message}"),
        other => panic!("{other:?}"),
    });
}

#[test]
fn traps_and_runaway_recursion_are_reported_not_fatal() {
    for name in ["unreachable", "reads_out_of_bounds", "deep_recursion"] {
        expect_error(name, |e| {
            assert!(matches!(e, PluginError::Trap(_)), "{name}: {e}")
        });
    }
    expect_error("reports_error", |e| {
        assert_eq!(*e, PluginError::Reported { code: 7 })
    });
}

#[test]
fn an_allocator_that_lies_is_caught_before_anything_is_written() {
    for name in ["bad_alloc_pointer", "zero_alloc_pointer"] {
        expect_error(name, |e| match e {
            PluginError::Trap(message) => {
                assert!(message.contains("pf_alloc"), "{name}: {message}")
            }
            other => panic!("{name}: {other:?}"),
        });
    }
}

#[test]
fn output_that_is_not_a_picture_is_refused_not_repaired() {
    expect_error("nan_output", |e| match e {
        PluginError::BadOutput(m) => assert!(m.contains("not a number"), "{m}"),
        other => panic!("{other:?}"),
    });
    expect_error("infinite_output", |e| {
        assert!(matches!(e, PluginError::BadOutput(_)), "{e}")
    });
    expect_error("alpha_out_of_range", |e| match e {
        PluginError::BadOutput(m) => assert!(m.contains("alpha"), "{m}"),
        other => panic!("{other:?}"),
    });
}

#[test]
fn a_module_that_asks_for_anything_the_host_does_not_give_is_refused_at_compile_time() {
    for (name, mention) in [
        ("imports_wasi", "wasi_snapshot_preview1.fd_write"),
        ("imports_unknown_host_function", "photoforge.read_file"),
        ("log_with_wrong_type", "photoforge.log"),
        ("imported_memory", "env.memory"),
    ] {
        let error = runtime()
            .compile(&adversarial(name))
            .err()
            .unwrap_or_else(|| panic!("{name} compiled"));
        match &error {
            PluginError::Module(message) => assert!(message.contains(mention), "{name}: {message}"),
            other => panic!("{name}: {other:?}"),
        }
    }
    // What a module must offer.
    for (name, mention) in [
        ("missing_filter_export", "pf_filter"),
        ("filter_with_wrong_signature", "pf_filter"),
        ("empty_module", "memory"),
    ] {
        let error = runtime()
            .compile(&adversarial(name))
            .err()
            .unwrap_or_else(|| panic!("{name} compiled"));
        match &error {
            PluginError::Module(message) => assert!(message.contains(mention), "{name}: {message}"),
            other => panic!("{name}: {other:?}"),
        }
    }
    // Garbage that is not a module at all.
    assert!(matches!(
        runtime().compile(b"MZ this is not wasm"),
        Err(PluginError::Module(_))
    ));
    assert!(matches!(
        runtime().compile(&[]),
        Err(PluginError::Module(_))
    ));
    assert!(matches!(
        runtime().compile(b"\0asm\x01\0\0\0\xff\xff\xff"),
        Err(PluginError::Module(_))
    ));
}

#[test]
fn a_module_built_for_another_interface_version_is_refused() {
    expect_error("wrong_abi_version", |e| match e {
        PluginError::Module(m) => assert!(m.contains("interface"), "{m}"),
        other => panic!("{other:?}"),
    });
}

#[test]
fn guest_text_is_bounded_and_cannot_fail_a_render() {
    let image = pattern(8, 8);
    let compiled = runtime().compile(&adversarial("log_flood")).unwrap();
    let job = |compiled: &_| {
        let input = vec![0u8; 8 * 8 * 16];
        let rect = Rect {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        };
        runtime().call(
            compiled,
            &TileJob {
                filter_index: 0,
                parameters: &[],
                image_width: 8,
                image_height: 8,
                input_rect: rect,
                input: &input,
                output_rect: rect,
            },
            call_limits(ALLOWANCE, 64),
            None,
        )
    };
    let result = job(&compiled).unwrap();
    assert_eq!(result.logs.len(), limits::LOG_LINES_PER_CALL);
    assert!(result
        .logs
        .iter()
        .all(|line| line.len() <= limits::LOG_LINE_BYTES + 8));
    // Out-of-range pointers and lengths are dropped, not an error.
    let compiled = runtime().compile(&adversarial("log_out_of_range")).unwrap();
    assert!(job(&compiled).unwrap().logs.is_empty());
    let _ = image;
}

// ---- time, cancellation and size -----------------------------------------------------------

fn tiny_job(width: u32, height: u32) -> (Vec<u8>, Rect) {
    (
        vec![0u8; (width * height * 16) as usize],
        Rect {
            x: 0,
            y: 0,
            width,
            height,
        },
    )
}

#[test]
fn a_call_that_outlasts_its_time_is_stopped_even_with_fuel_to_spare() {
    let compiled = runtime().compile(&adversarial("infinite_loop")).unwrap();
    let (input, rect) = tiny_job(4, 4);
    let started = Instant::now();
    let error = runtime()
        .call(
            &compiled,
            &TileJob {
                filter_index: 0,
                parameters: &[],
                image_width: 4,
                image_height: 4,
                input_rect: rect,
                input: &input,
                output_rect: rect,
            },
            CallLimits {
                memory_bytes: ALLOWANCE,
                fuel: u64::MAX / 2,
                time: Duration::from_millis(300),
            },
            None,
        )
        .err()
        .unwrap();
    assert_eq!(error, PluginError::TimedOut);
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(250),
        "stopped early: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "stopped late: {elapsed:?}"
    );
}

#[test]
fn a_running_filter_can_be_cancelled() {
    let compiled = runtime().compile(&adversarial("infinite_loop")).unwrap();
    let (input, rect) = tiny_job(4, 4);
    let flag = Arc::new(AtomicBool::new(false));
    let setter = {
        let flag = Arc::clone(&flag);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            flag.store(true, Ordering::Release);
        })
    };
    let started = Instant::now();
    let error = runtime()
        .call(
            &compiled,
            &TileJob {
                filter_index: 0,
                parameters: &[],
                image_width: 4,
                image_height: 4,
                input_rect: rect,
                input: &input,
                output_rect: rect,
            },
            CallLimits {
                memory_bytes: ALLOWANCE,
                fuel: u64::MAX / 2,
                time: Duration::from_secs(60),
            },
            Some(&flag),
        )
        .err()
        .unwrap();
    setter.join().unwrap();
    assert_eq!(error, PluginError::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn cancelling_through_the_filter_api_stops_between_and_within_tiles() {
    let compiled = runtime().compile(&adversarial("infinite_loop")).unwrap();
    let cancel = AtomicBool::new(true);
    let image = pattern(64, 64);
    // Cancelled before the first tile: no work is done at all.
    let started = Instant::now();
    let error = apply(
        runtime(),
        &compiled,
        &plain_decl(Locality::Pointwise),
        0,
        &[],
        &image,
        ALLOWANCE,
        16,
        Some(&cancel),
    )
    .err()
    .unwrap();
    assert_eq!(error, PluginError::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_whole_image_filter_that_cannot_fit_is_refused_before_it_starts() {
    let compiled = runtime().compile(&example_module("boxblur")).unwrap();
    let mut decl = example_manifest("boxblur").filters[0].clone();
    decl.locality = Locality::Global;
    // 4000 x 3000 pixels twice over is about 366 MiB, more than 256 MiB.
    let image = FloatImage::blank(4000, 3000, FloatRgba::new(0.5, 0.5, 0.5, 1.0)).unwrap();
    let started = Instant::now();
    let error = apply(
        runtime(),
        &compiled,
        &decl,
        0,
        &[1.0],
        &image,
        ALLOWANCE,
        256,
        None,
    )
    .err()
    .unwrap();
    match &error {
        PluginError::Refused(message) => {
            assert!(message.contains("whole image"), "{message}");
            assert!(message.contains("MiB"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "admission must not run the plugin"
    );
    // The same image as tiles is admitted: memory follows the tile, not the image.
    let tiled = example_manifest("boxblur").filters[0].clone();
    assert!(limits::call_memory_required(264 * 264, 256 * 256, 1) < ALLOWANCE);
    assert!(plan_tiles(tiled.locality, 4000, 3000, 256).len() > 100);
}

#[test]
fn parameters_are_checked_against_the_declaration_before_the_module_sees_them() {
    let manifest = example_manifest("border");
    let decl = &manifest.filters[0];
    check_parameters(decl, &[8.0, 0.0, 0.0, 0.0]).unwrap();
    for bad in [
        vec![8.0, 0.0, 0.0],           // too few
        vec![8.0, 0.0, 0.0, 0.0, 1.0], // too many
        vec![201.0, 0.0, 0.0, 0.0],    // out of range
        vec![-1.0, 0.0, 0.0, 0.0],
        vec![2.5, 0.0, 0.0, 0.0], // not an integer
        vec![8.0, 1.5, 0.0, 0.0],
        vec![f64::NAN, 0.0, 0.0, 0.0],
        vec![8.0, f64::INFINITY, 0.0, 0.0],
    ] {
        assert!(
            matches!(check_parameters(decl, &bad), Err(PluginError::Refused(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn a_call_that_asks_for_more_memory_than_it_may_have_is_refused_by_the_runtime_too() {
    let compiled = runtime().compile(&adversarial("honest_copy")).unwrap();
    let (input, rect) = tiny_job(64, 64);
    let error = runtime()
        .call(
            &compiled,
            &TileJob {
                filter_index: 0,
                parameters: &[],
                image_width: 64,
                image_height: 64,
                input_rect: rect,
                input: &input,
                output_rect: rect,
            },
            CallLimits {
                memory_bytes: 9 * MIB,
                fuel: 1_000_000,
                time: Duration::from_secs(5),
            },
            None,
        )
        .err();
    // 2 x 64 x 64 x 16 is 128 KiB, which fits in 9 MiB with 8 MiB of overhead...
    assert!(error.is_none(), "{error:?}");
    let error = runtime()
        .call(
            &compiled,
            &TileJob {
                filter_index: 0,
                parameters: &[],
                image_width: 64,
                image_height: 64,
                input_rect: rect,
                input: &input,
                output_rect: rect,
            },
            CallLimits {
                memory_bytes: 8 * MIB,
                fuel: 1_000_000,
                time: Duration::from_secs(5),
            },
            None,
        )
        .err()
        .unwrap();
    // ...and does not in 8 MiB.
    assert!(matches!(error, PluginError::Refused(_)), "{error:?}");
}
