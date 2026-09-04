//! Tiled versus full-frame rendering, one scenario per process so the reported
//! peak working set is a real high-water mark rather than a shared one.
//!
//! Usage: tiled_benchmark WIDTH HEIGHT SCENARIO MODE [TILE_SIZE]
//!   SCENARIO  stack | blur | group | global
//!   MODE      full | tiled | tiled1
//!
//! `tiled1` is the tiled renderer pinned to one thread. Comparing full against
//! tiled1 isolates what tiling itself costs; comparing tiled1 against tiled
//! isolates what the scheduler wins. Reporting only `full` against `tiled`
//! would credit tiling with the whole difference.
use photoforge_lib::{
    color::{FloatImage, FloatRgba},
    domain::EditOperation,
    layers::*,
    pixel::DocumentPrecision,
};
use std::time::Instant;

#[cfg(windows)]
fn peak_bytes() -> Option<usize> {
    #[repr(C)]
    struct Counters {
        cb: u32,
        page_faults: u32,
        values: [usize; 9],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, bytes: u32) -> i32;
    }
    let mut counters = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        page_faults: 0,
        values: [0; 9],
    };
    // SAFETY: only queries this process, with a correctly sized writable struct.
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<Counters>() as u32,
        )
    };
    (ok != 0).then_some(counters.values[0])
}
#[cfg(not(windows))]
fn peak_bytes() -> Option<usize> {
    None
}

fn layer(id: &str, pixel: &str, w: u32, h: u32) -> Layer {
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
        raw: None,
        content: LayerContent::Pixel {
            pixel_id: pixel.into(),
            width: w,
            height: h,
        },
    }
}

/// A deterministic image, so every run of every mode composites the same data.
fn image(w: u32, h: u32, seed: u32) -> Result<FloatImage, Box<dyn std::error::Error>> {
    let mut image = FloatImage::blank(w, h, FloatRgba::TRANSPARENT)?;
    let width = u64::from(w);
    for (index, pixel) in image.pixels_mut().iter_mut().enumerate() {
        let i = index as u64;
        let (x, y) = (i % width, i / width);
        let n = |k: u64| ((x * 7 + y * 13 + k * 31 + u64::from(seed) * 17) % 251) as f32 / 251.0;
        *pixel = FloatRgba {
            red: n(1),
            green: n(2),
            blue: n(3),
            alpha: 0.35 + n(4) * 0.65,
        };
    }
    Ok(image)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let w: u32 = args.get(1).ok_or("width")?.parse()?;
    let h: u32 = args.get(2).ok_or("height")?.parse()?;
    let scenario = args.get(3).ok_or("scenario")?.as_str();
    let mode = args.get(4).ok_or("mode")?.as_str();
    let tile_size: u32 = args.get(5).map_or(Ok(256), |v| v.parse())?;

    let mut store = LayerPixelStore::default();
    store.reset(w, h)?;
    // One immutable source shared by every layer, as precision_benchmark does.
    // Two 45 MP float sources are 1.4 GB and exceed the store's 1 GiB ceiling,
    // so registering per-layer sources would change the document between sizes
    // and make the size comparison meaningless.
    let base = store.register_float(image(w, h, 1)?)?;
    let over = base.clone();

    let mut doc = LayerDocument::new(w, h);
    doc.precision = DocumentPrecision::LinearSrgbF32;
    doc.layers.push(layer("base", &base, w, h));
    match scenario {
        "stack" => {
            let mut top = layer("over", &over, w, h);
            top.blend_mode = BlendMode::Screen;
            top.opacity = 0.6;
            doc.layers.push(top);
            let mut third = layer("third", &base, w, h);
            third.blend_mode = BlendMode::Overlay;
            third.opacity = 0.4;
            doc.layers.push(third);
        }
        "blur" => {
            let mut node = layer("blur", "unused", w, h);
            node.content = LayerContent::Adjustment {
                operation: Box::new(EditOperation::GaussianBlur { radius: 4.0 }),
            };
            doc.layers.push(node);
        }
        "group" => {
            let mut inner = layer("inner", &over, w, h);
            inner.blend_mode = BlendMode::Multiply;
            let mut group = layer("group", "unused", w, h);
            group.content = LayerContent::Group {
                children: vec![inner],
                isolated: true,
            };
            group.transform = LayerTransform {
                translate_x: 12.0,
                rotation_degrees: 3.0,
                ..LayerTransform::default()
            };
            group.opacity = 0.8;
            doc.layers.push(group);
        }
        // A big base plus one small layer: the case a tile cache exists for,
        // and the one where a whole-frame renderer does the most wasted work.
        "spot" => {
            let mut spot = layer("spot", &over, w, h);
            spot.transform = LayerTransform {
                translate_x: 40.0,
                translate_y: 40.0,
                scale_x: 0.05,
                scale_y: 0.05,
                ..LayerTransform::default()
            };
            doc.layers.push(spot);
        }
        // Deliberately untileable: shows the honest cost of the fallback rather
        // than hiding it by only benchmarking favourable documents.
        "global" => {
            let mut node = layer("deblock", "unused", w, h);
            node.content = LayerContent::Adjustment {
                operation: Box::new(EditOperation::AutoWhiteBalance { strength: 0.7 }),
            };
            doc.layers.push(node);
        }
        _ => return Err("unknown scenario".into()),
    }

    let resolved = store.resolve(&doc.referenced_pixel_ids(), false)?;
    let options = RenderOptions {
        scale: 1.0,
        cancel: None,
    };
    // Export modes write a real PNG16 so the memory figure includes the
    // encoder, not just the renderer.
    let folder = tempfile::tempdir()?;
    let source_path = folder.path().join("input.dng");
    std::fs::write(&source_path, b"benchmark source sentinel")?;
    let destination = folder.path().join("out.png");
    let export_options = photoforge_lib::color_management::ColorExportOptions {
        bit_depth: 16,
        ..Default::default()
    };

    let started = Instant::now();
    let mut second_ms = 0.0f64;
    let mut cache_stats: Option<photoforge_lib::layers::CacheStats> = None;
    let mut checksum = 0.0f64;
    // Sampled sparsely and identically in every mode: the point is to prove the
    // modes agree, and a streamed export never holds the frame to check.
    let mut accumulate = |first_row: u32, band: &photoforge_lib::color::FloatImage| {
        for (index, p) in band.pixels().iter().enumerate() {
            if (first_row as usize * band.width() as usize + index) % 4099 == 0 {
                checksum +=
                    f64::from(p.red) + f64::from(p.green) + f64::from(p.blue) + f64::from(p.alpha);
            }
        }
    };
    let stats = match mode {
        "full" | "tiled" | "tiled1" | "export" => {
            let (result, stats) = match mode {
                "full" | "export" => (
                    render_document_float(&doc, &resolved, options)?,
                    TiledStats::default(),
                ),
                "tiled" => render_document_tiled(&doc, &resolved, options, tile_size)?,
                _ => render_document_tiled_with_threads(&doc, &resolved, options, tile_size, 1)?,
            };
            accumulate(0, &result);
            if mode == "export" {
                photoforge_lib::infrastructure::save_color_image(
                    &result,
                    &source_path,
                    &destination,
                    photoforge_lib::domain::ExportProfile::Lossless,
                    export_options,
                    None,
                )?;
            }
            std::hint::black_box(&result);
            stats
        }
        // Renders twice through one cache and times the second: what the user
        // waits for when nothing has changed.
        "warm" | "nudge" => {
            let cache = TileCache::with_capacity(1024 * 1024 * 1024);
            let first =
                render_document_tiled_cached(&doc, &resolved, options, tile_size, 0, Some(&cache))?;
            std::hint::black_box(&first);
            if mode == "nudge" {
                // Move the small layer a few pixels, as dragging it would.
                if let Some(last) = doc.layers.last_mut() {
                    last.transform.translate_x += 3.0;
                }
            }
            let started_second = Instant::now();
            let (result, stats) =
                render_document_tiled_cached(&doc, &resolved, options, tile_size, 0, Some(&cache))?;
            second_ms = started_second.elapsed().as_secs_f64() * 1000.0;
            cache_stats = Some(cache.stats());
            accumulate(0, &result);
            std::hint::black_box(&result);
            stats
        }
        "stream" => {
            let mut streamed = TiledStats::default();
            photoforge_lib::infrastructure::save_color_image_streaming(
                w,
                h,
                &source_path,
                &destination,
                photoforge_lib::domain::ExportProfile::Lossless,
                export_options,
                None,
                |emit| {
                    streamed = render_document_streaming(
                        &doc,
                        &resolved,
                        options,
                        tile_size,
                        0,
                        &mut |first_row, band| {
                            accumulate(first_row, band);
                            emit(first_row, band)
                        },
                    )?;
                    Ok(())
                },
            )?;
            streamed
        }
        _ => return Err("unknown mode".into()),
    };
    let render_ms = started.elapsed().as_secs_f64() * 1000.0;
    let output_bytes = std::fs::metadata(&destination)
        .map(|m| m.len())
        .unwrap_or(0);
    let frame_bytes = u64::from(w) * u64::from(h) * 16;
    println!(
        "{}",
        serde_json::json!({
            "scenario": scenario,
            "mode": mode,
            "width": w,
            "height": h,
            "megapixels": f64::from(w) * f64::from(h) / 1e6,
            "tileSize": tile_size,
            "renderMs": render_ms,
            "tiles": stats.tiles,
            "haloedTiles": stats.haloed_tiles,
            "halo": stats.halo,
            "fellBackToFullFrame": stats.fell_back_to_full_frame,
            "peakRegionBytes": stats.peak_region_bytes,
            "frameBytes": frame_bytes,
            "workers": stats.workers,
            "secondRenderMs": second_ms,
            "cachedTiles": stats.cached_tiles,
            "cache": cache_stats,
            "outputBytes": output_bytes,
            "peakWorkingSetBytes": peak_bytes(),
            "storeBytes": store.total_bytes(),
            // Compared across modes: identical output is what makes the timings
            // comparable at all.
            "checksum": checksum
        })
    );
    Ok(())
}
