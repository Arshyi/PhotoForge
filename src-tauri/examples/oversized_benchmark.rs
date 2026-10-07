//! What opening part of a very large source costs, per format, against opening all of
//! it. One scenario per process, and the fixture is made by a separate process, so the
//! reported peak working set is the decode's and not the generator's.
//!
//! ```text
//! oversized_benchmark make   FORMAT WIDTH HEIGHT PATH     FORMAT: png | jpeg | webp | dng
//! oversized_benchmark run    PATH MODE [REGION_EDGE]      MODE: full | region_top | region_middle | region_bottom | reduced | probe
//!                                                               | open_full | open_region_middle | open_reduced   (the application's real open path)
//! ```
//!
//! `full` is the cost of opening the whole file as a conventional document would have
//! to decode it, at the decoder's native depth. `region_*` is a square window of
//! `REGION_EDGE` pixels (default 2000). `reduced` is a copy at a quarter of each
//! dimension. "Peak" is the process's peak working set, which is what Windows reports
//! and includes the image libraries' own transient buffers.
use photoforge_lib::color::DevelopmentParameters;
use photoforge_lib::raw::develop::{develop_bytes, develop_region, RenderScale};
use photoforge_lib::raw::dng::fixtures::DngBuilder;
use photoforge_lib::resources::memory::{MemoryProbe, OsProbe};
use photoforge_lib::source::open::{open_selection, OpenSelection};
use photoforge_lib::source::probe::{probe_path, report};
use photoforge_lib::source::{jpeg, png, webp, Rect};
use std::path::Path;
use std::time::Instant;

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

fn peak() -> u64 {
    OsProbe
        .process()
        .map_or(0, |process| process.peak_working_set)
}

fn private() -> u64 {
    OsProbe.process().map_or(0, |process| process.private_bytes)
}

fn make(format: &str, width: u32, height: u32, path: &Path) {
    match format {
        "png" | "jpeg" | "webp" | "webpa" => {
            // Smooth structure plus noise: neither flat (which compresses to nothing and
            // flatters every format) nor pure noise (which defeats the codecs).
            let mut image = image::RgbImage::new(width, height);
            let mut state = 0x9e37_79b9u32;
            for (x, y, pixel) in image.enumerate_pixels_mut() {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (state >> 28) as u8;
                let base = ((x / 7 + y / 5) % 180) as u8;
                *pixel = image::Rgb([
                    base.saturating_add(noise),
                    ((x * 255 / width) as u8).saturating_add(noise),
                    ((y * 255 / height) as u8).wrapping_add(noise),
                ]);
            }
            match format {
                "png" => image.save(path).unwrap(),
                "jpeg" => {
                    let file = std::fs::File::create(path).unwrap();
                    image::codecs::jpeg::JpegEncoder::new_with_quality(
                        std::io::BufWriter::new(file),
                        90,
                    )
                    .encode_image(&image)
                    .unwrap();
                }
                "webpa" => {
                    // The same picture with a varying alpha channel.
                    let mut rgba = image::DynamicImage::ImageRgb8(image).to_rgba8();
                    for (x, y, pixel) in rgba.enumerate_pixels_mut() {
                        pixel[3] = 128 + ((x ^ y) % 127) as u8;
                    }
                    let file = std::fs::File::create(path).unwrap();
                    image::codecs::webp::WebPEncoder::new_lossless(std::io::BufWriter::new(file))
                        .encode(
                            rgba.as_raw(),
                            width,
                            height,
                            image::ExtendedColorType::Rgba8,
                        )
                        .unwrap();
                }
                _ => {
                    let file = std::fs::File::create(path).unwrap();
                    image::codecs::webp::WebPEncoder::new_lossless(std::io::BufWriter::new(file))
                        .encode(
                            image.as_raw(),
                            width,
                            height,
                            image::ExtendedColorType::Rgb8,
                        )
                        .unwrap();
                }
            }
        }
        "dng" => {
            let samples: Vec<u16> = (0..u64::from(width) * u64::from(height))
                .map(|index| {
                    let x = (index % u64::from(width)) as u32;
                    let y = (index / u64::from(width)) as u32;
                    (600 + ((x / 8 + y / 8) % 3000) as u16)
                        .wrapping_add(((x.wrapping_mul(2_654_435_761) ^ y) % 97) as u16)
                })
                .collect();
            let mut builder = DngBuilder::new(width, height, samples)
                .levels(vec![512, 512, 512, 514], (2, 2), 4095)
                .neutral([0.55, 1.0, 0.72])
                .matrix([0.72, 0.18, 0.10, 0.16, 0.74, 0.10, 0.09, 0.20, 0.71]);
            builder.tile_size = Some((256, 256));
            std::fs::write(path, builder.build()).unwrap();
        }
        other => panic!("unknown format {other}"),
    }
    println!(
        "made {} ({width}x{height}) {:.1} MiB",
        path.display(),
        mib(std::fs::metadata(path).unwrap().len())
    );
}

fn run(path: &Path, mode: &str, edge: u32) {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let file_mib = mib(std::fs::metadata(path).unwrap().len());
    let (width, height, label) = if extension == "dng" {
        let bytes = std::fs::read(path).unwrap();
        let sensor = photoforge_lib::raw::dng::decode(&bytes).unwrap();
        (sensor.width, sensor.height, "dng")
    } else {
        let probed = probe_path(path).unwrap();
        (
            probed.probe.width as u32,
            probed.probe.height as u32,
            match extension.as_str() {
                "png" => "png",
                "jpg" | "jpeg" => "jpeg",
                _ => "webp",
            },
        )
    };
    let edge = edge.min(width).min(height);
    let rect_at = |y: u32| Rect {
        x: (width - edge) / 2,
        y,
        width: edge,
        height: edge,
    };
    let (rect, mode_name) = match mode {
        "region_top" => (Some(rect_at(0)), "region_top"),
        "region_middle" => (Some(rect_at((height - edge) / 2)), "region_middle"),
        "region_bottom" => (Some(rect_at(height - edge)), "region_bottom"),
        _ => (None, mode),
    };
    // The application's real open path, against the planner's own prediction for it:
    // decode, the 16-byte-per-pixel float working copy, the preview, and the hash of the
    // file, all of it, which is what a person's click costs.
    if mode.starts_with("open_") && label != "dng" {
        let probed = probe_path(path).unwrap();
        let planned = report(&probed, 0);
        let (selection, predicted) = match mode {
            "open_full" => (None, planned.full_peak_bytes),
            "open_region_middle" => {
                let rect = rect_at((height - edge) / 2);
                let cost = planned.region_cost.expect("this format can open a region");
                (
                    Some(OpenSelection::Region(rect)),
                    u64::try_from(cost.peak(u128::from(rect.pixels()))).unwrap_or(u64::MAX),
                )
            }
            _ => {
                let (w, h) = ((width / 4).max(1), (height / 4).max(1));
                (
                    Some(OpenSelection::Reduced {
                        width: w,
                        height: h,
                    }),
                    0,
                )
            }
        };
        let before = private();
        let started = Instant::now();
        let produced = match selection {
            None => {
                let loaded = photoforge_lib::infrastructure::load_image(path).unwrap();
                format!("{}x{}", loaded.metadata.width, loaded.metadata.height)
            }
            Some(selection) => {
                let loaded = open_selection(path, selection, 0, None).unwrap();
                format!("{}x{}", loaded.metadata.width, loaded.metadata.height)
            }
        };
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        println!(
            "format={label} source={width}x{height} file_mib={file_mib:.0} mode={mode} produced={produced} ms={elapsed:.0}              peak_ws_mib={:.0} private_before_mib={:.0} predicted_peak_mib={}",
            mib(peak()),
            mib(before),
            if predicted == 0 { "n/a".to_string() } else { format!("{:.0}", mib(predicted)) }
        );
        return;
    }
    let before = private();
    let started = Instant::now();
    let produced = match (label, mode) {
        (_, "probe") => probe_path(path)
            .map(|p| format!("{}x{}", p.probe.width, p.probe.height))
            .unwrap(),
        ("dng", "full") => {
            let bytes = std::fs::read(path).unwrap();
            let developed =
                develop_bytes(&bytes, &DevelopmentParameters::default(), RenderScale::Full)
                    .unwrap();
            format!("{}x{}", developed.image.width(), developed.image.height())
        }
        ("dng", m) if m.starts_with("region") => {
            let bytes = std::fs::read(path).unwrap();
            let region =
                develop_region(&bytes, rect.unwrap(), &DevelopmentParameters::default()).unwrap();
            format!("{}x{}", region.image.width(), region.image.height())
        }
        ("dng", _) => "no reduced copy is offered for a DNG".to_string(),
        (_, "full") => {
            let reader = image::ImageReader::open(path)
                .unwrap()
                .with_guessed_format()
                .unwrap();
            let decoded = reader.decode().unwrap();
            format!("{}x{}", decoded.width(), decoded.height())
        }
        (format, m) if m.starts_with("region") => {
            let rect = rect.unwrap();
            let image = match format {
                "png" => png::decode_region(path, rect, None).map(|r| r.image),
                "jpeg" => jpeg::decode_region(path, rect, None).map(|r| r.image),
                _ => webp::decode_region(path, rect, None).map(|r| r.image),
            }
            .unwrap();
            format!("{}x{}", image.width(), image.height())
        }
        (format, "reduced") => {
            let (w, h) = ((width / 4).max(1), (height / 4).max(1));
            let image = match format {
                "png" => png::decode_reduced(path, w, h, None),
                "jpeg" => jpeg::decode_reduced(path, w, h, None),
                _ => webp::decode_reduced(path, w, h, None),
            }
            .unwrap();
            format!("{}x{}", image.width(), image.height())
        }
        other => panic!("unknown mode {other:?}"),
    };
    let elapsed = started.elapsed().as_secs_f64() * 1000.0;
    println!(
        "format={label} source={width}x{height} file_mib={file_mib:.0} mode={mode_name} produced={produced} \
         ms={elapsed:.0} peak_ws_mib={:.0} private_before_mib={:.0}",
        mib(peak()),
        mib(before)
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("make") if args.len() == 5 => make(
            &args[1],
            args[2].parse().unwrap(),
            args[3].parse().unwrap(),
            Path::new(&args[4]),
        ),
        Some("run") if args.len() >= 3 => run(
            Path::new(&args[1]),
            &args[2],
            args.get(3).map_or(2000, |v| v.parse().unwrap()),
        ),
        _ => {
            eprintln!("usage: oversized_benchmark make FORMAT W H PATH | run PATH MODE [EDGE]");
            std::process::exit(2);
        }
    }
}
