//! What text costs: font discovery once, then shaping and rasterising.
//!
//! Discovery is the number that decides the architecture — it is paid once, on
//! first use, and it is far too slow to repeat while somebody is typing.
use std::time::Instant;

use photoforge_lib::layers::shape::ShapeColor;
use photoforge_lib::layers::text::{render_text, TextContent};
use photoforge_lib::text;

fn main() {
    let started = Instant::now();
    let families = text::available_families();
    let discovery = started.elapsed();
    println!(
        "font discovery: {} families in {:.0} ms",
        families.len(),
        discovery.as_secs_f64() * 1000.0
    );

    let started = Instant::now();
    let _ = text::family_is_available("Arial");
    println!(
        "availability check: {:.3} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );

    let mut content = TextContent::new(
        "Handgloves quickly jumping over the lazy dog, twice",
        20.0,
        80.0,
        32.0,
    );
    content.fill = ShapeColor::new(0.0, 0.0, 0.0, 1.0);
    content.wrap_width = Some(700.0);

    // Cold: the first shape of this exact request.
    let started = Instant::now();
    let shaped = content.shaped().expect("shape");
    println!(
        "shape (cold): {:.2} ms for {} glyphs",
        started.elapsed().as_secs_f64() * 1000.0,
        shaped.glyphs.len()
    );

    // Warm: what every tile of a frame after the first actually pays.
    let started = Instant::now();
    for _ in 0..1000 {
        let _ = content.shaped().expect("shape");
    }
    println!(
        "shape (cached): {:.4} ms each",
        started.elapsed().as_secs_f64() / 1000.0 * 1000.0
    );

    for (label, width, height) in [
        ("one 256 tile", 256u32, 256u32),
        ("full 1920x1080", 1920, 1080),
    ] {
        let started = Instant::now();
        for _ in 0..20 {
            let _ = render_text(&content, |p| p, 0, 0, width, height).expect("render");
        }
        println!(
            "rasterise {label}: {:.2} ms each",
            started.elapsed().as_secs_f64() / 20.0 * 1000.0
        );
    }
}
