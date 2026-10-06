//! One scenario per process, so the OS peak is an actual scenario high-water mark.
//! Usage: precision_benchmark WIDTH HEIGHT raw|single|adjustment|mask|multi|transform|preview|export|legacy
use photoforge_lib::{
    color::DevelopmentParameters,
    domain::{EditOperation, ExportProfile},
    layers::*,
    pixel::DocumentPrecision,
    raw::{
        develop::{develop_sensor, RenderScale},
        dng::{self, fixtures::DngBuilder},
    },
};
use std::time::Instant;

#[cfg(windows)]
fn memory() -> Option<(usize, usize)> {
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
    (ok != 0).then_some((counters.values[0], counters.values[7]))
}
#[cfg(not(windows))]
fn memory() -> Option<(usize, usize)> {
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
        origin: None,
        content: LayerContent::Pixel {
            pixel_id: pixel.into(),
            width: w,
            height: h,
        },
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let w: u32 = args.get(1).ok_or("width")?.parse()?;
    let h: u32 = args.get(2).ok_or("height")?.parse()?;
    let scenario = args.get(3).ok_or("scenario")?;
    let total = Instant::now();
    let samples = (0..u64::from(w) * u64::from(h))
        .map(|i| 512 + ((i * 37 + i / u64::from(w) * 13) % 3000) as u16)
        .collect();
    let bytes = DngBuilder::new(w, h, samples)
        .levels(vec![512], (1, 1), 4000)
        .build();
    let started = Instant::now();
    let sensor = dng::decode(&bytes)?;
    let decode_ms = started.elapsed().as_secs_f64() * 1000.0;
    drop(bytes);
    let started = Instant::now();
    let developed = develop_sensor(
        &sensor,
        &DevelopmentParameters::default(),
        RenderScale::Full,
    )?;
    let development_ms = started.elapsed().as_secs_f64() * 1000.0;
    drop(sensor);
    let mut store = LayerPixelStore::default();
    store.reset(w, h)?;
    let id = if scenario == "legacy" {
        store.register(developed.image.to_rgba8())?
    } else {
        store.register_float(developed.image)?
    };
    let mut doc = LayerDocument::new(w, h);
    doc.precision = if scenario == "legacy" {
        DocumentPrecision::LegacySrgb8
    } else {
        DocumentPrecision::LinearSrgbF32
    };
    doc.layers.push(layer("raw", &id, w, h));
    match scenario.as_str() {
        "adjustment" => {
            let mut node = layer("exposure", "unused", w, h);
            node.content = LayerContent::Adjustment {
                operation: Box::new(EditOperation::RawDevelopment {
                    parameters: DevelopmentParameters {
                        exposure_ev: 0.25,
                        ..Default::default()
                    },
                }),
            };
            doc.layers.push(node);
        }
        "mask" => {
            let mut mask = photoforge_lib::mask::MaskBitmap::full(w, h)?;
            for y in 0..h {
                for x in 0..w / 2 {
                    mask.set(x, y, 128);
                }
            }
            doc.layers[0].mask = Some(LayerMask {
                snapshot: photoforge_lib::mask::MaskSnapshot::encode(&mask),
                enabled: true,
                inverted: false,
            });
        }
        "multi" => {
            for index in 0..2 {
                let mut node = layer(&format!("duplicate{index}"), &id, w, h);
                node.opacity = 0.25;
                node.blend_mode = BlendMode::Screen;
                doc.layers.push(node);
            }
        }
        "transform" => {
            doc.layers[0].transform.translate_x = 0.25;
            doc.layers[0].transform.rotation_degrees = 0.1;
        }
        "raw" | "single" | "preview" | "export" | "legacy" => {}
        _ => return Err("unknown scenario".into()),
    }
    let mut render_ms = 0.0;
    let mut export_ms = 0.0;
    let mut output_bytes = 0;
    if scenario != "raw" {
        let preview = scenario == "preview";
        let resolved = store.resolve(&doc.referenced_pixel_ids(), preview)?;
        let started = Instant::now();
        let result = render_document_typed(
            &doc,
            &resolved,
            RenderOptions {
                scale: if preview { store.preview_scale() } else { 1.0 },
                cancel: None,
            },
        )?;
        render_ms = started.elapsed().as_secs_f64() * 1000.0;
        if scenario == "export" {
            let folder = tempfile::tempdir()?;
            let destination = folder.path().join("precision.png");
            std::fs::write(
                folder.path().join("input.dng"),
                b"benchmark source sentinel",
            )?;
            let started = Instant::now();
            photoforge_lib::infrastructure::save_color_image(
                result.linear()?.as_ref(),
                &folder.path().join("input.dng"),
                &destination,
                ExportProfile::Lossless,
                Default::default(),
                None,
            )?;
            export_ms = started.elapsed().as_secs_f64() * 1000.0;
            output_bytes = std::fs::metadata(destination)?.len();
        }
        std::hint::black_box(result);
    }
    let peak = memory();
    println!(
        "{}",
        serde_json::json!({"scenario":scenario,"width":w,"height":h,"megapixels":f64::from(w)*f64::from(h)/1e6,"layers":doc.layers.len(),"decodeMs":decode_ms,"developmentMs":development_ms,"renderMs":render_ms,"exportMs":export_ms,"totalMs":total.elapsed().as_secs_f64()*1000.0,"peakWorkingSetBytes":peak.map(|p|p.0),"peakPagefileBytes":peak.map(|p|p.1),"storeBytes":store.total_bytes(),"outputBytes":output_bytes,"source":"synthetic CFA DNG; multi shares one immutable source"})
    );
    Ok(())
}
