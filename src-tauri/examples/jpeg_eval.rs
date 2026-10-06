//! Evidence for the JPEG decoding strategy: three decoders, one large file, each
//! run in its own process so the peak working set is not contaminated.
//!
//!   jpeg_eval make <path> <w> <h>      write a test JPEG
//!   jpeg_eval zune <path>              full decode through `image` (zune-jpeg)
//!   jpeg_eval jd <path>                full decode through jpeg-decoder
//!   jpeg_eval jd-scaled <path> <w> <h> scaled decode through jpeg-decoder
use photoforge_lib::resources::memory::{MemoryProbe, OsProbe};
use std::time::Instant;

fn peak_mib() -> f64 {
    OsProbe
        .process()
        .map_or(0.0, |p| p.peak_working_set as f64 / 1048576.0)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let started = Instant::now();
    match args[1].as_str() {
        "make" => {
            let (w, h): (u32, u32) = (args[3].parse().unwrap(), args[4].parse().unwrap());
            // A smooth gradient with fine noise, so the file is a realistic size.
            let mut buffer = vec![0u8; (w * h * 3) as usize];
            let mut seed = 12345u32;
            for y in 0..h {
                for x in 0..w {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    let n = (seed >> 28) as u8;
                    let i = ((y * w + x) * 3) as usize;
                    buffer[i] = ((x * 255 / w) as u8).saturating_add(n);
                    buffer[i + 1] = ((y * 255 / h) as u8).saturating_add(n);
                    buffer[i + 2] = (((x + y) * 255 / (w + h)) as u8).saturating_add(n);
                }
            }
            let file = std::fs::File::create(&args[2]).unwrap();
            let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                std::io::BufWriter::new(file),
                85,
            );
            encoder
                .encode(&buffer, w, h, image::ExtendedColorType::Rgb8)
                .unwrap();
            println!(
                "wrote {} ({} bytes)",
                args[2],
                std::fs::metadata(&args[2]).unwrap().len()
            );
        }
        "zune" => {
            let image = image::ImageReader::open(&args[2])
                .unwrap()
                .with_guessed_format()
                .unwrap()
                .decode()
                .unwrap();
            println!(
                "zune full: {}x{} in {:.0} ms, peak working set {:.0} MiB",
                image.width(),
                image.height(),
                started.elapsed().as_secs_f64() * 1000.0,
                peak_mib()
            );
        }
        "jd" => {
            let mut decoder = jpeg_decoder::Decoder::new(std::io::BufReader::new(
                std::fs::File::open(&args[2]).unwrap(),
            ));
            let pixels = decoder.decode().unwrap();
            let info = decoder.info().unwrap();
            println!(
                "jpeg-decoder full: {}x{} ({} bytes) in {:.0} ms, peak working set {:.0} MiB",
                info.width,
                info.height,
                pixels.len(),
                started.elapsed().as_secs_f64() * 1000.0,
                peak_mib()
            );
        }
        "jd-scaled" => {
            let (rw, rh): (u16, u16) = (args[3].parse().unwrap(), args[4].parse().unwrap());
            let mut decoder = jpeg_decoder::Decoder::new(std::io::BufReader::new(
                std::fs::File::open(&args[2]).unwrap(),
            ));
            decoder.read_info().unwrap();
            let (sw, sh) = decoder.scale(rw, rh).unwrap();
            let pixels = decoder.decode().unwrap();
            println!("jpeg-decoder scaled to ask {rw}x{rh}: got {sw}x{sh} ({} bytes) in {:.0} ms, peak working set {:.0} MiB", pixels.len(), started.elapsed().as_secs_f64() * 1000.0, peak_mib());
        }
        other => panic!("unknown mode {other}"),
    }
}
