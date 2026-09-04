//! Black-level normalisation and Bayer demosaicing.
//!
//! Two algorithms are offered and both are deterministic: the same sensor data
//! always produces the same pixels, on any machine, in any order.
//!
//! * `Quality::Fast` is bilinear interpolation. It is cheap enough to run on
//!   every preview and is correct — it simply blurs detail near edges.
//! * `Quality::High` is the gradient-corrected method of Malvar, He, and Cutler
//!   (Microsoft Research, 2004). It costs a 5x5 neighbourhood per pixel and
//!   measurably reduces the colour fringing bilinear leaves on edges, which is
//!   why it is called high quality here rather than as a matter of taste.
//!
//! Nothing in this module invents detail. Both algorithms interpolate the
//! samples the sensor actually recorded.

use super::dng::{CfaColor, CfaPattern, SensorImage};

/// Which demosaic algorithm to run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    /// Bilinear: used for interactive previews.
    Fast,
    /// Malvar-He-Cutler: used for full renders and export.
    #[default]
    High,
}

impl Quality {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Fast => "bilinear",
            Self::High => "malvar-he-cutler",
        }
    }
}

/// Sensor samples after black subtraction, in scene-linear units where 1.0 is
/// the sensor's white level. Values above 1.0 are not clipped: a highlight that
/// exceeded the nominal white point is still real data, and clipping it here
/// would throw away the recovery headroom the whole pipeline exists to keep.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedCfa {
    pub data: Vec<f32>,
    pub width: u32,
    pub height: u32,
    pub cfa: CfaPattern,
}

impl NormalizedCfa {
    #[inline]
    pub fn at(&self, x: i64, y: i64) -> f32 {
        // Edges clamp to the nearest real photosite, which keeps the CFA phase
        // correct only for even offsets; callers that need the right colour use
        // `mirror` instead.
        let x = x.clamp(0, i64::from(self.width) - 1) as usize;
        let y = y.clamp(0, i64::from(self.height) - 1) as usize;
        self.data[y * self.width as usize + x]
    }

    /// Reflects a coordinate back inside the sensor.
    ///
    /// Reflection about either edge changes a coordinate by an even amount, so
    /// the reflected photosite keeps the colour of the one that was asked for
    /// and the CFA phase stays intact. Clamping instead would flip the phase on
    /// odd offsets and tint the border.
    #[inline]
    fn mirror(&self, x: i64, y: i64) -> f32 {
        let x = reflect(x, i64::from(self.width));
        let y = reflect(y, i64::from(self.height));
        self.data[y as usize * self.width as usize + x as usize]
    }
}

/// Folds a coordinate back into `0..length`, preserving its parity.
#[inline]
fn reflect(value: i64, length: i64) -> i64 {
    if length <= 1 {
        return 0;
    }
    let mut value = value;
    // Two folds cover the +/-2 reach of the widest kernel here; the clamp is a
    // backstop so a degenerate length can never leave the range.
    for _ in 0..2 {
        if value < 0 {
            value = -value;
        }
        if value >= length {
            value = 2 * (length - 1) - value;
        }
    }
    value.clamp(0, length - 1)
}

/// Subtracts the per-position black level and scales to the white level.
///
/// Each CFA position may carry its own black level, because sensor amplifiers
/// differ channel to channel; using a single average would leave a colour cast
/// in the shadows that no white balance can remove.
pub fn normalize(sensor: &SensorImage) -> NormalizedCfa {
    let width = sensor.width;
    let height = sensor.height;
    let mut data = vec![0.0f32; sensor.data.len()];
    for y in 0..height {
        for x in 0..width {
            let index = y as usize * width as usize + x as usize;
            let raw = f32::from(sensor.data[index]);
            let black = sensor.black_level[CfaPattern::cell_index(x, y)];
            let range = sensor.white_level - black;
            // Validation already refused a non-positive range, but a decoder
            // change must not be able to turn that into a division by zero.
            let value = if range > 0.0 {
                (raw - black) / range
            } else {
                0.0
            };
            // Below-black samples are sensor noise, not negative light.
            data[index] = if value.is_finite() {
                value.max(0.0)
            } else {
                0.0
            };
        }
    }
    NormalizedCfa {
        data,
        width,
        height,
        cfa: sensor.cfa,
    }
}

/// Scales each CFA channel by its white-balance multiplier before interpolation.
///
/// Balancing before demosaicing is deliberate: interpolating channels that are
/// still unbalanced mixes a strong green into a weak red and leaves colour
/// fringes on edges that no later correction can undo.
pub fn apply_white_balance(cfa: &mut NormalizedCfa, multipliers: [f32; 3]) {
    let safe: Vec<f32> = multipliers
        .iter()
        .map(|value| {
            if value.is_finite() && *value > 0.0 {
                *value
            } else {
                1.0
            }
        })
        .collect();
    for y in 0..cfa.height {
        for x in 0..cfa.width {
            let index = y as usize * cfa.width as usize + x as usize;
            let gain = match cfa.cfa.color_at(x, y) {
                CfaColor::Red => safe[0],
                CfaColor::Green => safe[1],
                CfaColor::Blue => safe[2],
            };
            cfa.data[index] *= gain;
        }
    }
}

/// Interpolates a full RGB triple at every photosite.
///
/// Returns interleaved RGB in scene-linear units, still in the camera's own
/// colour space.
pub fn demosaic(cfa: &NormalizedCfa, quality: Quality) -> Vec<[f32; 3]> {
    match quality {
        Quality::Fast => bilinear(cfa),
        Quality::High => malvar(cfa),
    }
}

/// Whether the red photosites neighbouring this green one lie left and right
/// rather than above and below.
#[inline]
fn red_is_horizontal(cfa: &NormalizedCfa, x: i64, y: i64) -> bool {
    // The pattern repeats every two photosites, so a negative or oversized
    // coordinate still names the right cell once it is folded into range.
    let neighbour = ((x + 1).rem_euclid(2)) as u32;
    let row = (y.rem_euclid(2)) as u32;
    cfa.cfa.color_at(neighbour, row) == CfaColor::Red
}

fn bilinear(cfa: &NormalizedCfa) -> Vec<[f32; 3]> {
    let width = i64::from(cfa.width);
    let height = i64::from(cfa.height);
    let mut out = vec![[0.0f32; 3]; cfa.data.len()];
    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let colour = cfa.cfa.color_at(x as u32, y as u32);
            // The four diagonal neighbours, the four orthogonal ones, and the
            // sample itself are all the bilinear kernel needs.
            let centre = cfa.mirror(x, y);
            let horizontal = 0.5 * (cfa.mirror(x - 1, y) + cfa.mirror(x + 1, y));
            let vertical = 0.5 * (cfa.mirror(x, y - 1) + cfa.mirror(x, y + 1));
            let orthogonal = 0.25
                * (cfa.mirror(x - 1, y)
                    + cfa.mirror(x + 1, y)
                    + cfa.mirror(x, y - 1)
                    + cfa.mirror(x, y + 1));
            let diagonal = 0.25
                * (cfa.mirror(x - 1, y - 1)
                    + cfa.mirror(x + 1, y - 1)
                    + cfa.mirror(x - 1, y + 1)
                    + cfa.mirror(x + 1, y + 1));

            out[index] = match colour {
                CfaColor::Red => [centre, orthogonal, diagonal],
                CfaColor::Blue => [diagonal, orthogonal, centre],
                CfaColor::Green => {
                    // On a green photosite red lies along one axis and blue
                    // along the other. `color_at` is pure modular arithmetic,
                    // so asking about the neighbour is safe even at an edge.
                    if red_is_horizontal(cfa, x, y) {
                        [horizontal, centre, vertical]
                    } else {
                        [vertical, centre, horizontal]
                    }
                }
            };
        }
    }
    out
}

/// Malvar-He-Cutler gradient-corrected interpolation.
///
/// The correction terms are the Laplacian of the known channel, which is what
/// lets the interpolation follow an edge instead of averaging across it.
fn malvar(cfa: &NormalizedCfa) -> Vec<[f32; 3]> {
    let width = i64::from(cfa.width);
    let height = i64::from(cfa.height);
    let mut out = vec![[0.0f32; 3]; cfa.data.len()];

    for y in 0..height {
        for x in 0..width {
            let index = (y * width + x) as usize;
            let s = |dx: i64, dy: i64| cfa.mirror(x + dx, y + dy);
            let centre = s(0, 0);

            // The four Laplacian-style correction terms the paper defines.
            let cross = s(-1, 0) + s(1, 0) + s(0, -1) + s(0, 1);
            let far = s(-2, 0) + s(2, 0) + s(0, -2) + s(0, 2);
            let diagonal = s(-1, -1) + s(1, -1) + s(-1, 1) + s(1, 1);
            let horizontal_pair = s(-1, 0) + s(1, 0);
            let vertical_pair = s(0, -1) + s(0, 1);
            let far_horizontal = s(-2, 0) + s(2, 0);
            let far_vertical = s(0, -2) + s(0, 2);

            // Green at a red or blue photosite.
            let green_here = 0.125 * (4.0 * centre + 2.0 * cross - far);
            // The opposite colour at a red or blue photosite.
            let opposite = 0.125 * (6.0 * centre + 2.0 * diagonal - 1.5 * far);
            // A red or blue value on a green photosite. The two kernels are
            // transposes of one another: whichever axis carries the colour gets
            // the strong +4 terms, the far samples on that axis are subtracted,
            // and the diagonals carry -1. Each sums to 8, so a flat field comes
            // through at unit gain.
            let along_row = 0.125
                * (5.0 * centre + 4.0 * horizontal_pair - far_horizontal + 0.5 * far_vertical
                    - diagonal);
            let along_column = 0.125
                * (5.0 * centre + 4.0 * vertical_pair - far_vertical + 0.5 * far_horizontal
                    - diagonal);

            out[index] = match cfa.cfa.color_at(x as u32, y as u32) {
                CfaColor::Red => [centre, green_here, opposite],
                CfaColor::Blue => [opposite, green_here, centre],
                CfaColor::Green => {
                    if red_is_horizontal(cfa, x, y) {
                        [along_row, centre, along_column]
                    } else {
                        [along_column, centre, along_row]
                    }
                }
            };
        }
    }
    out
}

/// Converts camera RGB into linear sRGB with the matrix the file supplied.
///
/// Without a matrix the camera values are passed through unchanged and the
/// caller is expected to say so rather than claim the result is colour managed.
pub fn apply_camera_matrix(pixels: &mut [[f32; 3]], matrix: &[[f32; 3]; 3]) {
    for pixel in pixels.iter_mut() {
        let red = matrix[0][0] * pixel[0] + matrix[0][1] * pixel[1] + matrix[0][2] * pixel[2];
        let green = matrix[1][0] * pixel[0] + matrix[1][1] * pixel[1] + matrix[1][2] * pixel[2];
        let blue = matrix[2][0] * pixel[0] + matrix[2][1] * pixel[1] + matrix[2][2] * pixel[2];
        // A matrix can drive a channel slightly negative on saturated colours.
        // Negatives are not light, but the highlight side is left alone so
        // recovery headroom survives.
        *pixel = [
            if red.is_finite() { red.max(0.0) } else { 0.0 },
            if green.is_finite() {
                green.max(0.0)
            } else {
                0.0
            },
            if blue.is_finite() { blue.max(0.0) } else { 0.0 },
        ];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::dng::fixtures::DngBuilder;

    fn cfa_from(width: u32, height: u32, pattern: CfaPattern, data: Vec<f32>) -> NormalizedCfa {
        NormalizedCfa {
            data,
            width,
            height,
            cfa: pattern,
        }
    }

    /// A flat scene: every photosite of every colour records the same level, so
    /// a correct demosaic must return exactly that level everywhere.
    fn flat(width: u32, height: u32, value: f32, pattern: CfaPattern) -> NormalizedCfa {
        cfa_from(
            width,
            height,
            pattern,
            vec![value; (width * height) as usize],
        )
    }

    #[test]
    fn normalisation_maps_black_to_zero_and_white_to_one() {
        let samples = vec![512u16, 8192, 16383, 512, 16383, 8192, 512, 1000, 16383];
        let data = DngBuilder::new(3, 3, samples)
            .levels(vec![512], (1, 1), 16383)
            .build();
        let sensor = crate::raw::dng::decode(&data).unwrap();
        let normalized = normalize(&sensor);
        assert_eq!(normalized.data[0], 0.0, "black did not map to zero");
        assert!(
            (normalized.data[2] - 1.0).abs() < 1e-6,
            "white did not map to one: {}",
            normalized.data[2]
        );
        assert!(normalized.data.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn normalisation_uses_the_black_level_of_each_cfa_position() {
        // Every photosite records its own black level, so a correct
        // normalisation returns zero everywhere despite the differing values.
        // cell_index is (y % 2) * 2 + (x % 2), so the levels 100/200/300/400
        // belong to positions (0,0), (1,0), (0,1), (1,1) and repeat from there.
        let samples = vec![
            100u16, 200, 100, 200, 300, 400, 300, 400, 100, 200, 100, 200,
        ];
        let data = DngBuilder::new(4, 3, samples)
            .levels(vec![100, 200, 300, 400], (2, 2), 4095)
            .build();
        let sensor = crate::raw::dng::decode(&data).unwrap();
        let normalized = normalize(&sensor);
        for (index, value) in normalized.data.iter().enumerate() {
            assert_eq!(*value, 0.0, "position {index} kept a black offset: {value}");
        }
    }

    /// A highlight above the nominal white level is real captured signal. If
    /// normalisation clipped it, exposure could never bring it back, which is
    /// the whole reason for developing from RAW.
    #[test]
    fn normalisation_keeps_headroom_above_the_white_level() {
        let samples = vec![5000u16; 4];
        let data = DngBuilder::new(2, 2, samples)
            .levels(vec![0], (1, 1), 4000)
            .build();
        let sensor = crate::raw::dng::decode(&data).unwrap();
        let normalized = normalize(&sensor);
        assert!(
            normalized.data[0] > 1.0,
            "a highlight above white was clipped to {}",
            normalized.data[0]
        );
        assert!((normalized.data[0] - 1.25).abs() < 1e-6);
    }

    #[test]
    fn white_balance_scales_each_channel_by_its_own_multiplier() {
        let mut cfa = flat(4, 4, 0.5, CfaPattern::RGGB);
        apply_white_balance(&mut cfa, [2.0, 1.0, 3.0]);
        // RGGB: (0,0) red, (1,0) green, (0,1) green, (1,1) blue.
        assert_eq!(cfa.data[0], 1.0);
        assert_eq!(cfa.data[1], 0.5);
        assert_eq!(cfa.data[4], 0.5);
        assert_eq!(cfa.data[5], 1.5);
    }

    #[test]
    fn a_nonsense_white_balance_multiplier_is_ignored_rather_than_applied() {
        let mut cfa = flat(2, 2, 0.5, CfaPattern::RGGB);
        apply_white_balance(&mut cfa, [f32::NAN, 0.0, -1.0]);
        assert!(cfa.data.iter().all(|value| (*value - 0.5).abs() < 1e-9));
    }

    /// The strongest correctness check available without a reference image: a
    /// uniform scene must demosaic to that exact uniform colour, for every
    /// Bayer layout and both algorithms. Any phase error in the CFA handling
    /// shows up immediately as a checkerboard.
    #[test]
    fn a_flat_field_demosaics_to_a_flat_neutral_image_in_every_layout() {
        for pattern in [
            CfaPattern::RGGB,
            CfaPattern::BGGR,
            CfaPattern::GRBG,
            CfaPattern::GBRG,
        ] {
            for quality in [Quality::Fast, Quality::High] {
                let cfa = flat(8, 8, 0.4, pattern);
                let rgb = demosaic(&cfa, quality);
                for (index, pixel) in rgb.iter().enumerate() {
                    for (channel, value) in pixel.iter().enumerate() {
                        assert!(
                            (value - 0.4).abs() < 1e-5,
                            "{} {:?}: pixel {index} channel {channel} is {value}",
                            quality.name(),
                            pattern.name()
                        );
                    }
                }
            }
        }
    }

    /// A scene where each channel is flat but at a different level must come
    /// back with those three levels everywhere: this is what proves the
    /// demosaic assigns each interpolated value to the right channel.
    #[test]
    fn a_flat_colour_demosaics_to_that_colour_in_every_layout() {
        for pattern in [
            CfaPattern::RGGB,
            CfaPattern::BGGR,
            CfaPattern::GRBG,
            CfaPattern::GBRG,
        ] {
            let (width, height) = (10u32, 10u32);
            let mut data = vec![0.0f32; (width * height) as usize];
            for y in 0..height {
                for x in 0..width {
                    data[(y * width + x) as usize] = match pattern.color_at(x, y) {
                        CfaColor::Red => 0.8,
                        CfaColor::Green => 0.5,
                        CfaColor::Blue => 0.2,
                    };
                }
            }
            let cfa = cfa_from(width, height, pattern, data);
            for quality in [Quality::Fast, Quality::High] {
                let rgb = demosaic(&cfa, quality);
                // Interior pixels only: the edges reflect and are checked by
                // the flat-field test above.
                for y in 2..height - 2 {
                    for x in 2..width - 2 {
                        let pixel = rgb[(y * width + x) as usize];
                        assert!(
                            (pixel[0] - 0.8).abs() < 1e-4
                                && (pixel[1] - 0.5).abs() < 1e-4
                                && (pixel[2] - 0.2).abs() < 1e-4,
                            "{} {}: ({x},{y}) is {pixel:?}, expected [0.8, 0.5, 0.2]",
                            quality.name(),
                            pattern.name()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_known_channel_is_never_altered_by_interpolation() {
        // Whatever the sensor measured must survive demosaicing untouched.
        let (width, height) = (8u32, 8u32);
        let data: Vec<f32> = (0..width * height)
            .map(|index| (index % 17) as f32 / 17.0)
            .collect();
        let cfa = cfa_from(width, height, CfaPattern::RGGB, data.clone());
        for quality in [Quality::Fast, Quality::High] {
            let rgb = demosaic(&cfa, quality);
            for y in 0..height {
                for x in 0..width {
                    let index = (y * width + x) as usize;
                    let channel = match CfaPattern::RGGB.color_at(x, y) {
                        CfaColor::Red => 0,
                        CfaColor::Green => 1,
                        CfaColor::Blue => 2,
                    };
                    assert!(
                        (rgb[index][channel] - data[index]).abs() < 1e-6,
                        "{}: measured sample at ({x},{y}) changed",
                        quality.name()
                    );
                }
            }
        }
    }

    /// Samples a known RGB image through a CFA, the way a sensor would.
    fn mosaic(rgb: &[[f32; 3]], width: u32, height: u32, pattern: CfaPattern) -> NormalizedCfa {
        let mut data = vec![0.0f32; (width * height) as usize];
        for y in 0..height {
            for x in 0..width {
                let index = (y * width + x) as usize;
                data[index] = match pattern.color_at(x, y) {
                    CfaColor::Red => rgb[index][0],
                    CfaColor::Green => rgb[index][1],
                    CfaColor::Blue => rgb[index][2],
                };
            }
        }
        cfa_from(width, height, pattern, data)
    }

    /// A scene with real structure whose channels are correlated, as a
    /// photograph's are: shared luminance detail carrying a modest colour
    /// difference. Gradient correction rests on exactly that correlation — it
    /// borrows the sharp channel's Laplacian to place the interpolated one — so
    /// a benchmark of it has to use a scene where the assumption holds. On an
    /// artificial image whose channels are unrelated, bilinear does better, and
    /// the comparison would say nothing about photographs.
    fn scene(width: u32, height: u32) -> Vec<[f32; 3]> {
        (0..width * height)
            .map(|index| {
                let x = (index % width) as f32;
                let y = (index / width) as f32;
                let dx = x - width as f32 * 0.5;
                let dy = y - height as f32 * 0.5;
                // Shared detail: a diagonal edge, a disc, and a fine ramp.
                let mut luma = if x + y > width as f32 { 0.72 } else { 0.24 };
                if dx * dx + dy * dy < (width as f32 * 0.22).powi(2) {
                    luma = 0.88;
                }
                luma += 0.06 * ((x * 0.5).sin() + (y * 0.35).cos());
                let luma = luma.clamp(0.02, 0.98);
                // A gentle, slowly varying colour cast on top of that detail.
                let warm = 0.04 * (x / width as f32);
                [
                    (luma + warm).clamp(0.0, 1.0),
                    luma,
                    (luma - warm * 0.5).clamp(0.0, 1.0),
                ]
            })
            .collect()
    }

    /// Mean squared reconstruction error against the image the sensor saw.
    fn reconstruction_error(quality: Quality, pattern: CfaPattern) -> f64 {
        let (width, height) = (64u32, 64u32);
        let truth = scene(width, height);
        let cfa = mosaic(&truth, width, height, pattern);
        let restored = demosaic(&cfa, quality);
        let mut total = 0.0f64;
        let mut count = 0u64;
        // Interior only, so the comparison is about interpolation rather than
        // about how each algorithm handles a border.
        for y in 3..height - 3 {
            for x in 3..width - 3 {
                let index = (y * width + x) as usize;
                for channel in 0..3 {
                    let error = f64::from(restored[index][channel] - truth[index][channel]);
                    total += error * error;
                    count += 1;
                }
            }
        }
        total / count as f64
    }

    /// The reason `High` is called high quality rather than merely named so:
    /// it reconstructs a known image more accurately than bilinear does, on
    /// every Bayer layout. This is the standard way demosaic algorithms are
    /// compared — mosaic an image you already know, restore it, and measure.
    #[test]
    fn gradient_correction_reconstructs_a_known_scene_better_than_bilinear() {
        for pattern in [
            CfaPattern::RGGB,
            CfaPattern::BGGR,
            CfaPattern::GRBG,
            CfaPattern::GBRG,
        ] {
            let fast = reconstruction_error(Quality::Fast, pattern);
            let high = reconstruction_error(Quality::High, pattern);
            assert!(
                high < fast,
                "{}: gradient correction was not more accurate ({high:.6} vs {fast:.6})",
                pattern.name()
            );
        }
    }

    /// A guard against the two paths silently becoming the same code.
    #[test]
    fn the_two_algorithms_produce_different_results_on_real_structure() {
        let (width, height) = (32u32, 32u32);
        let cfa = mosaic(&scene(width, height), width, height, CfaPattern::RGGB);
        assert_ne!(demosaic(&cfa, Quality::Fast), demosaic(&cfa, Quality::High));
    }

    #[test]
    fn demosaicing_is_deterministic() {
        let cfa = flat(16, 16, 0.33, CfaPattern::GRBG);
        for quality in [Quality::Fast, Quality::High] {
            assert_eq!(demosaic(&cfa, quality), demosaic(&cfa, quality));
        }
    }

    #[test]
    fn a_one_pixel_sensor_does_not_panic() {
        for pattern in [CfaPattern::RGGB, CfaPattern::BGGR] {
            for quality in [Quality::Fast, Quality::High] {
                let cfa = flat(1, 1, 0.5, pattern);
                assert_eq!(demosaic(&cfa, quality).len(), 1);
            }
        }
        // A two-by-one strip exercises the mirroring on a degenerate axis.
        let cfa = flat(2, 1, 0.25, CfaPattern::RGGB);
        assert_eq!(demosaic(&cfa, Quality::High).len(), 2);
    }

    #[test]
    fn the_camera_matrix_is_applied_and_negatives_are_not_light() {
        let mut pixels = vec![[0.5f32, 0.5, 0.5], [1.0, 0.0, 0.0]];
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        apply_camera_matrix(&mut pixels, &identity);
        assert_eq!(pixels[0], [0.5, 0.5, 0.5]);

        let mut pixels = vec![[1.0f32, 0.0, 0.0]];
        // A matrix row that would drive a channel negative.
        let matrix = [[1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
        apply_camera_matrix(&mut pixels, &matrix);
        assert_eq!(pixels[0][1], 0.0, "a negative channel was kept as light");
    }

    #[test]
    fn the_camera_matrix_preserves_highlight_headroom() {
        let mut pixels = vec![[1.4f32, 1.4, 1.4]];
        let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        apply_camera_matrix(&mut pixels, &identity);
        assert!(
            pixels[0][0] > 1.0,
            "the matrix clipped a recoverable highlight"
        );
    }
}
