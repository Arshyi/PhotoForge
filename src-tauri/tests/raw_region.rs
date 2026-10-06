//! Gates for developing a region of a RAW file.
//!
//! The claim under test is strict: a region developed on its own is **bit-for-bit
//! identical** to the same rectangle cut from a development of the whole sensor.
//! Not close, not within a tolerance: identical. Anything weaker would let an
//! edge artifact through, because a demosaic that read the wrong neighbour at a
//! window boundary is wrong by a small amount that a tolerance would forgive.
//!
//! The sensor is a deterministic textured pattern, not noise and not a flat field.
//! A flat field develops flat whatever the context, so it could not tell a correct
//! margin from a missing one; texture makes a missing neighbour show.
use photoforge_lib::color::{DevelopmentParameters, FloatImage, WhiteBalance};
use photoforge_lib::raw::demosaic::Quality;
use photoforge_lib::raw::develop::{
    demosaic_margin, develop_bytes, develop_region, region_window, RenderScale,
};
use photoforge_lib::raw::dng::fixtures::DngBuilder;
use photoforge_lib::raw::dng::{decode, decode_window, inspect_layout};
use photoforge_lib::source::Rect;

const WIDTH: u32 = 67;
const HEIGHT: u32 = 53;

/// A pattern with edges and gradients at every scale, so every photosite differs
/// from its neighbours and a wrong neighbour changes the answer.
fn pattern() -> Vec<u16> {
    let mut samples = Vec::with_capacity((WIDTH * HEIGHT) as usize);
    let mut state = 0x2468_ace1u32;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let noise = (state >> 22) as u16; // 0..1023
            let ramp = ((x * 41 + y * 23) % 2048) as u16;
            let edge = if (x / 9 + y / 7) % 2 == 0 { 700 } else { 0 };
            samples.push((300 + ramp / 2 + edge + noise / 3).min(4000));
        }
    }
    samples
}

fn dng(tile: Option<(u32, u32)>) -> Vec<u8> {
    let mut builder = DngBuilder::new(WIDTH, HEIGHT, pattern())
        .levels(vec![64, 64, 64, 64], (2, 2), 4095)
        .neutral([0.52, 1.0, 0.61])
        .matrix([1.2, -0.3, 0.1, -0.2, 1.1, 0.05, 0.02, -0.1, 0.9]);
    builder.tile_size = tile;
    builder.build()
}

fn parameters(balance: WhiteBalance) -> DevelopmentParameters {
    DevelopmentParameters {
        white_balance: balance,
        exposure_ev: 0.3,
        contrast: 0.2,
        highlights: -0.1,
        shadows: 0.15,
        ..DevelopmentParameters::default()
    }
}

fn crop(image: &FloatImage, rect: &Rect) -> Vec<[f32; 4]> {
    let mut out = Vec::new();
    for y in rect.y..rect.y + rect.height {
        for x in rect.x..rect.x + rect.width {
            let p = image.get(x, y).unwrap();
            out.push([p.red, p.green, p.blue, p.alpha]);
        }
    }
    out
}

fn pixels(image: &FloatImage) -> Vec<[f32; 4]> {
    image
        .pixels()
        .iter()
        .map(|p| [p.red, p.green, p.blue, p.alpha])
        .collect()
}

/// Every awkward place a rectangle can sit.
fn rectangles() -> Vec<Rect> {
    let r = |x, y, width, height| Rect {
        x,
        y,
        width,
        height,
    };
    vec![
        r(0, 0, WIDTH, HEIGHT),         // the whole sensor
        r(0, 0, 1, 1),                  // the first photosite
        r(WIDTH - 1, HEIGHT - 1, 1, 1), // the last photosite
        r(0, HEIGHT - 1, 1, 1),
        r(WIDTH - 1, 0, 1, 1),
        r(1, 1, 1, 1),                      // odd origin, one photosite
        r(3, 5, 20, 17),                    // odd origin, interior
        r(2, 4, 20, 17),                    // even origin, interior
        r(0, 0, 30, 20),                    // against the top-left corner
        r(WIDTH - 30, HEIGHT - 20, 30, 20), // against the bottom-right corner
        r(0, 20, 12, 8),                    // against the left edge only
        r(30, 0, 12, 8),                    // against the top edge only
        r(WIDTH - 12, 17, 12, 9),           // against the right edge only
        r(13, HEIGHT - 8, 12, 8),           // against the bottom edge only
        r(7, 9, 2, 2),                      // two by two, odd origin
        r(16, 16, 16, 16),                  // exactly on tile boundaries
        r(15, 15, 18, 18),                  // straddling them
        r(5, 3, 1, 40),                     // a one-photosite-wide column
        r(4, 7, 55, 1),                     // a one-photosite-high row
    ]
}

fn assert_region_matches(data: &[u8], balance: WhiteBalance, label: &str) {
    let settings = parameters(balance);
    let full = develop_bytes(data, &settings, RenderScale::Full).expect("full development");
    for rect in rectangles() {
        let region = develop_region(data, rect, &settings)
            .unwrap_or_else(|error| panic!("{label}: region {rect:?} failed: {error}"));
        assert_eq!(
            (region.image.width(), region.image.height()),
            (rect.width, rect.height),
            "{label}: wrong size for {rect:?}"
        );
        let got = pixels(&region.image);
        let want = crop(&full.image, &rect);
        assert_eq!(
            got, want,
            "{label}: region {rect:?} differs from the same rectangle of a full development"
        );
        // The numbers actually applied must agree too, or the interface would show
        // the user a balance the region did not use.
        assert_eq!(
            region.multipliers, full.multipliers,
            "{label}: gains differ for {rect:?}"
        );
        assert_eq!(region.source_dimensions, (WIDTH, HEIGHT));
    }
}

/// The headline gate, under every white-balance mode, in strips and in tiles.
#[test]
fn a_region_is_bit_identical_to_the_crop_of_a_full_development() {
    for (layout, tile) in [
        ("strips", None),
        ("8x8 tiles", Some((8, 8))),
        ("16x16 tiles", Some((16, 16))),
    ] {
        let data = dng(tile);
        for (name, balance) in [
            (
                "as shot",
                WhiteBalance::AsShot {
                    multipliers: [1.0; 3],
                },
            ),
            (
                "custom",
                WhiteBalance::Custom {
                    multipliers: [1.9, 1.0, 1.4],
                },
            ),
            (
                "temperature and tint",
                WhiteBalance::TemperatureTint {
                    temperature: 0.2,
                    tint: -0.1,
                },
            ),
        ] {
            assert_region_matches(&data, balance, &format!("{layout}, {name}"));
        }
    }
}

/// Auto balance is a statistic over the whole sensor. A region cannot compute it
/// from itself, so this is the case that fails if the gains are taken from the
/// window instead of the sensor.
#[test]
fn auto_white_balance_comes_from_the_whole_sensor_not_the_window() {
    for (layout, tile) in [("strips", None), ("tiles", Some((8, 8)))] {
        let data = dng(tile);
        assert_region_matches(&data, WhiteBalance::Auto, &format!("{layout}, auto"));
    }
    // And it really is a whole-image statistic: a small region taken from a part
    // of the sensor with a different cast would balance differently on its own.
    let data = dng(Some((8, 8)));
    let settings = parameters(WhiteBalance::Auto);
    let whole = develop_bytes(&data, &settings, RenderScale::Full).unwrap();
    let corner = develop_region(
        &data,
        Rect {
            x: 0,
            y: 0,
            width: 8,
            height: 8,
        },
        &settings,
    )
    .unwrap();
    assert_eq!(corner.multipliers, whole.multipliers);
}

/// The window decode itself: the samples it returns are the samples of the full
/// decode at the same photosites, for every layout.
#[test]
fn a_window_decode_returns_the_samples_of_the_full_decode() {
    for tile in [None, Some((8, 8)), Some((16, 16)), Some((20, 12))] {
        let data = dng(tile);
        let full = decode(&data).unwrap();
        for window in [
            Rect {
                x: 0,
                y: 0,
                width: WIDTH,
                height: HEIGHT,
            },
            Rect {
                x: 4,
                y: 2,
                width: 30,
                height: 25,
            },
            Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
            Rect {
                x: 64,
                y: 50,
                width: 3,
                height: 3,
            },
            Rect {
                x: 16,
                y: 16,
                width: 16,
                height: 16,
            },
        ] {
            let got = decode_window(&data, window).unwrap();
            assert_eq!((got.width, got.height), (window.width, window.height));
            for y in 0..window.height {
                for x in 0..window.width {
                    assert_eq!(
                        got.sample(x, y),
                        full.sample(window.x + x, window.y + y),
                        "tile {tile:?}, window {window:?}, at {x},{y}"
                    );
                }
            }
        }
    }
}

/// A tiled file is read in pieces: tiles outside the window are never decoded.
/// Truncating the file proves it. Tiles are stored in order, so cutting the file
/// removes the later ones; a window in the first tile still decodes, and decoding
/// the whole sensor does not.
#[test]
fn tiles_outside_the_window_are_never_read() {
    let data = dng(Some((16, 16)));
    // Cut away the second half of the payload (and with it the tail tiles).
    let cut = data.len() * 6 / 10;
    // Not so much that the directory goes with it: the structure sits before the
    // pixel data in this writer, and the layout must still parse.
    let truncated = &data[..cut];
    assert!(
        inspect_layout(truncated).is_ok(),
        "the directory did not survive the cut"
    );
    assert!(decode(truncated).is_err(), "a truncated file decoded whole");
    let window = Rect {
        x: 0,
        y: 0,
        width: 12,
        height: 12,
    };
    let got = decode_window(truncated, window)
        .expect("a window inside the first tile should not need the tiles that were cut");
    let full = decode(&data).unwrap();
    for y in 0..12 {
        for x in 0..12 {
            assert_eq!(got.sample(x, y), full.sample(x, y));
        }
    }
}

/// The margin is what makes the edges right. With none, the region differs from
/// the crop — which is the evidence that the margin is doing real work.
#[test]
fn the_demosaic_margin_is_load_bearing() {
    let data = dng(Some((8, 8)));
    let settings = parameters(WhiteBalance::AsShot {
        multipliers: [1.0; 3],
    });
    let full = develop_bytes(&data, &settings, RenderScale::Full).unwrap();
    let rect = Rect {
        x: 21,
        y: 19,
        width: 14,
        height: 11,
    };
    // Develop only the rectangle's own sensor window, with no context around it.
    let bare = decode_window(
        &data,
        Rect {
            x: 20,
            y: 18,
            width: 16,
            height: 12,
        },
    )
    .unwrap();
    let naive =
        photoforge_lib::raw::develop::develop_sensor(&bare, &settings, RenderScale::Full).unwrap();
    let mut differing = 0usize;
    for dy in 0..rect.height {
        for dx in 0..rect.width {
            let from_full = full.image.get(rect.x + dx, rect.y + dy).unwrap();
            let from_naive = naive.image.get(rect.x - 20 + dx, rect.y - 18 + dy).unwrap();
            if from_full != from_naive {
                differing += 1;
            }
        }
    }
    assert!(
        differing > 0,
        "developing without context produced the same pixels, so this pattern cannot show a missing margin"
    );
    // And the real region, with its margin, is exact.
    let region = develop_region(&data, rect, &settings).unwrap();
    assert_eq!(pixels(&region.image), crop(&full.image, &rect));
}

#[test]
fn the_window_grows_by_the_margin_clips_to_the_sensor_and_starts_even() {
    let margin = demosaic_margin(Quality::High);
    assert_eq!(margin, 2);
    // Interior, odd origin: grown by the margin, then moved out to even.
    let window = region_window(
        &Rect {
            x: 21,
            y: 19,
            width: 14,
            height: 11,
        },
        WIDTH,
        HEIGHT,
        Quality::High,
    );
    assert_eq!((window.x, window.y), (18, 16));
    assert_eq!(window.x % 2, 0);
    assert_eq!(window.y % 2, 0);
    assert!(window.right() >= 21 + 14 + 2);
    assert!(window.bottom() >= 19 + 11 + 2);
    // At the corners it clips instead of leaving the sensor.
    let corner = region_window(
        &Rect {
            x: 0,
            y: 0,
            width: 5,
            height: 5,
        },
        WIDTH,
        HEIGHT,
        Quality::High,
    );
    assert_eq!((corner.x, corner.y), (0, 0));
    let far = region_window(
        &Rect {
            x: WIDTH - 3,
            y: HEIGHT - 3,
            width: 3,
            height: 3,
        },
        WIDTH,
        HEIGHT,
        Quality::High,
    );
    assert_eq!(far.right(), u64::from(WIDTH));
    assert_eq!(far.bottom(), u64::from(HEIGHT));
}

#[test]
fn hostile_requests_are_refused_cleanly() {
    let data = dng(Some((8, 8)));
    let settings = parameters(WhiteBalance::default());
    for rect in [
        Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 5,
        },
        Rect {
            x: 60,
            y: 0,
            width: 20,
            height: 5,
        },
        Rect {
            x: 0,
            y: 50,
            width: 5,
            height: 20,
        },
        Rect {
            x: u32::MAX,
            y: u32::MAX,
            width: u32::MAX,
            height: u32::MAX,
        },
    ] {
        assert!(develop_region(&data, rect, &settings).is_err(), "{rect:?}");
    }
    // A window must keep the Bayer phase.
    assert!(decode_window(
        &data,
        Rect {
            x: 1,
            y: 0,
            width: 10,
            height: 10
        }
    )
    .is_err());
    assert!(decode_window(
        &data,
        Rect {
            x: 0,
            y: 3,
            width: 10,
            height: 10
        }
    )
    .is_err());
    assert!(decode_window(
        &data,
        Rect {
            x: 0,
            y: 0,
            width: 100,
            height: 10
        }
    )
    .is_err());
    // Not a DNG at all.
    assert!(develop_region(
        b"not a raw file",
        Rect {
            x: 0,
            y: 0,
            width: 1,
            height: 1
        },
        &settings
    )
    .is_err());
}

/// The layout is reported without decoding a sample.
#[test]
fn the_layout_describes_how_the_sensor_is_divided() {
    let tiled = inspect_layout(&dng(Some((16, 16)))).unwrap();
    assert!(tiled.tiled);
    assert_eq!((tiled.segment_width, tiled.segment_height), (16, 16));
    assert_eq!(tiled.segment_count(), 5 * 4);
    assert_eq!((tiled.width, tiled.height), (WIDTH, HEIGHT));
    let strips = inspect_layout(&dng(None)).unwrap();
    assert!(!strips.tiled);
    assert_eq!(strips.segment_width, WIDTH);
}
