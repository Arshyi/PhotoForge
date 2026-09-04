//! The RAW development graph.
//!
//! One ordered pipeline turns camera bytes into a working image. The order is
//! fixed here, in code, rather than emerging from the order a user happens to
//! touch the controls:
//!
//! ```text
//!   RAW bytes
//!     -> decode sensor + metadata      (raw::dng)
//!     -> subtract black, scale to white (demosaic::normalize)
//!     -> white balance on the CFA       (demosaic::apply_white_balance)
//!     -> demosaic                       (demosaic::demosaic)
//!     -> camera RGB to linear sRGB      (demosaic::apply_camera_matrix)
//!     -> exposure, contrast, tone       (color::apply_development)
//!     -> working FloatImage             (linear, unclipped)
//! ```
//!
//! Everything above the last step is scene-linear and unclipped: a highlight
//! brighter than the nominal white level survives all the way to the display
//! transform, which is the only reason exposure can pull it back.
//!
//! White balance is applied *before* demosaicing on purpose. Interpolating
//! channels that are still unbalanced mixes a strong green into a weak red and
//! leaves colour fringes on edges that nothing downstream can undo.

use super::demosaic::{self, Quality};
use super::dng;
use super::{RawError, RAW_MAX_PIXELS};
use crate::color::{
    apply_development, ColorPipelineError, DevelopmentParameters, FloatImage, FloatRgba,
    WhiteBalance,
};

/// How large a preview may get before it is decimated.
///
/// A 45-megapixel sensor cannot be demosaiced from scratch on every pointer
/// move, so interactive work runs on a reduced sensor read and only the final
/// render uses every photosite.
pub const PREVIEW_MAX_EDGE: u32 = 1_600;

/// What to produce: an interactive preview or the finished picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderScale {
    /// Decimate the sensor so the longest edge is at most `PREVIEW_MAX_EDGE`,
    /// and demosaic with the fast algorithm.
    Preview,
    /// Every photosite, best algorithm.
    Full,
}

impl RenderScale {
    const fn quality(self) -> Quality {
        match self {
            // Bilinear on a decimated sensor is indistinguishable at preview
            // size and many times cheaper.
            Self::Preview => Quality::Fast,
            Self::Full => Quality::High,
        }
    }
}

/// What a development produced, alongside what was actually used to produce it.
#[derive(Debug, Clone)]
pub struct DevelopedRaw {
    pub image: FloatImage,
    /// The white-balance multipliers actually applied, whatever the mode asked
    /// for, so the interface can show the user the real numbers.
    pub multipliers: [f32; 3],
    /// Whether a camera colour matrix was available and applied.
    pub color_managed: bool,
    pub demosaic: Quality,
    /// Sensor dimensions before any preview decimation.
    pub source_dimensions: (u32, u32),
}

/// Resolves a white-balance mode into concrete channel multipliers.
///
/// `AsShot` uses the camera's own recorded neutral when the file carries one.
/// A file without that data cannot be developed "as shot", and this says so by
/// falling back to neutral rather than inventing a number.
pub fn resolve_multipliers(
    balance: &WhiteBalance,
    sensor: &dng::SensorImage,
    cfa: &demosaic::NormalizedCfa,
) -> [f32; 3] {
    let normalise = |values: [f32; 3]| -> [f32; 3] {
        // Green is the reference channel: scaling all three so green is 1.0
        // keeps overall exposure unchanged when only the tint moves.
        let green = if values[1].is_finite() && values[1] > 0.0 {
            values[1]
        } else {
            1.0
        };
        let mut out = [1.0f32; 3];
        for (index, value) in values.iter().enumerate() {
            let scaled = value / green;
            out[index] = if scaled.is_finite() {
                scaled.clamp(0.01, 16.0)
            } else {
                1.0
            };
        }
        out
    };

    match balance {
        WhiteBalance::AsShot { multipliers } => {
            // The camera's neutral is what the sensor must be divided by, so
            // the multipliers are its reciprocal.
            match sensor.as_shot_neutral {
                Some(neutral) => normalise([1.0 / neutral[0], 1.0 / neutral[1], 1.0 / neutral[2]]),
                None => normalise(*multipliers),
            }
        }
        WhiteBalance::Custom { multipliers } => normalise(*multipliers),
        WhiteBalance::Auto => normalise(auto_multipliers(cfa)),
        WhiteBalance::TemperatureTint { temperature, tint } => {
            // Temperature and tint are offsets from the as-shot neutral rather
            // than absolute Kelvin: the file gives no illuminant to anchor an
            // absolute scale to, and pretending otherwise would be a made-up
            // number on the interface.
            let base = sensor
                .as_shot_neutral
                .map(|neutral| [1.0 / neutral[0], 1.0 / neutral[1], 1.0 / neutral[2]])
                .unwrap_or([1.0, 1.0, 1.0]);
            let warm = 1.0 + temperature * 0.5;
            let cool = 1.0 - temperature * 0.5;
            let magenta = 1.0 + tint * 0.5;
            normalise([base[0] * warm, base[1] / magenta, base[2] * cool])
        }
    }
}

/// Grey-world estimate: the multipliers that make the average of each channel
/// equal. Simple, deterministic, and honest about being an estimate.
fn auto_multipliers(cfa: &demosaic::NormalizedCfa) -> [f32; 3] {
    let mut sums = [0.0f64; 3];
    let mut counts = [0u64; 3];
    for y in 0..cfa.height {
        for x in 0..cfa.width {
            let value = cfa.data[y as usize * cfa.width as usize + x as usize];
            // Saturated photosites carry no colour information.
            if !(0.001..=0.99).contains(&value) {
                continue;
            }
            let channel = match cfa.cfa.color_at(x, y) {
                dng::CfaColor::Red => 0,
                dng::CfaColor::Green => 1,
                dng::CfaColor::Blue => 2,
            };
            sums[channel] += f64::from(value);
            counts[channel] += 1;
        }
    }
    let mut averages = [0.0f32; 3];
    for index in 0..3 {
        averages[index] = if counts[index] > 0 {
            (sums[index] / counts[index] as f64) as f32
        } else {
            0.0
        };
    }
    // With no usable sample in a channel there is nothing to balance towards.
    if averages.iter().any(|value| *value <= 0.0) {
        return [1.0, 1.0, 1.0];
    }
    [averages[1] / averages[0], 1.0, averages[1] / averages[2]]
}

/// Decimates a CFA plane by whole two-pixel blocks, preserving its phase.
///
/// Stepping by an even factor keeps every sampled photosite on the same colour
/// it had before, so the reduced plane is still a valid Bayer image and can be
/// demosaiced by the same code.
fn decimate(cfa: &demosaic::NormalizedCfa, factor: u32) -> demosaic::NormalizedCfa {
    if factor <= 1 {
        return cfa.clone();
    }
    let step = factor.max(1) * 2;
    let width = (cfa.width / step).max(1) * 2;
    let height = (cfa.height / step).max(1) * 2;
    let mut data = vec![0.0f32; (width * height) as usize];
    for y in 0..height {
        for x in 0..width {
            // Each output 2x2 block is copied from one input 2x2 block, so the
            // colour at every position is unchanged.
            let source_x = (x / 2) * step + (x % 2);
            let source_y = (y / 2) * step + (y % 2);
            let source_x = source_x.min(cfa.width - 1);
            let source_y = source_y.min(cfa.height - 1);
            data[(y * width + x) as usize] = cfa.data[(source_y * cfa.width + source_x) as usize];
        }
    }
    demosaic::NormalizedCfa {
        data,
        width,
        height,
        cfa: cfa.cfa,
    }
}

/// The decimation factor that brings a sensor under the preview ceiling.
fn preview_factor(width: u32, height: u32) -> u32 {
    let longest = width.max(height);
    if longest <= PREVIEW_MAX_EDGE {
        return 1;
    }
    (longest.div_ceil(PREVIEW_MAX_EDGE)).max(1)
}

/// Runs the whole development graph on an already decoded sensor image.
pub fn develop_sensor(
    sensor: &dng::SensorImage,
    parameters: &DevelopmentParameters,
    scale: RenderScale,
) -> Result<DevelopedRaw, RawError> {
    parameters
        .validate()
        .map_err(|error| RawError::InvalidMetadata(error.to_string()))?;

    // 1. Black level and normalisation.
    let mut cfa = demosaic::normalize(sensor);

    // 2. Preview decimation, before any per-pixel work that would be thrown
    //    away. The multipliers are estimated on the same data the render uses.
    if scale == RenderScale::Preview {
        let factor = preview_factor(cfa.width, cfa.height);
        if factor > 1 {
            cfa = decimate(&cfa, factor);
        }
    }

    // 3. White balance, on the CFA, before interpolation.
    let multipliers = resolve_multipliers(&parameters.white_balance, sensor, &cfa);
    demosaic::apply_white_balance(&mut cfa, multipliers);

    // 4. Demosaic.
    let quality = scale.quality();
    let mut rgb = demosaic::demosaic(&cfa, quality);

    // 5. Camera colour space to linear sRGB.
    let color_managed = match sensor.camera_to_srgb {
        Some(matrix) => {
            demosaic::apply_camera_matrix(&mut rgb, &matrix);
            true
        }
        None => false,
    };

    // 6. Into the shared float image, then the photographic controls.
    let pixels: Vec<FloatRgba> = rgb
        .iter()
        .map(|pixel| FloatRgba {
            red: pixel[0],
            green: pixel[1],
            blue: pixel[2],
            alpha: 1.0,
        })
        .collect();
    let mut image = FloatImage::new(cfa.width, cfa.height, pixels)
        .map_err(|error: ColorPipelineError| RawError::InvalidMetadata(error.to_string()))?;
    apply_development(&mut image, parameters)
        .map_err(|error| RawError::InvalidMetadata(error.to_string()))?;

    Ok(DevelopedRaw {
        image,
        multipliers,
        color_managed,
        demosaic: quality,
        source_dimensions: (sensor.width, sensor.height),
    })
}

/// Decodes and develops a RAW buffer in one step.
pub fn develop_bytes(
    data: &[u8],
    parameters: &DevelopmentParameters,
    scale: RenderScale,
) -> Result<DevelopedRaw, RawError> {
    if data.len() as u64 > super::RAW_MAX_FILE_BYTES {
        return Err(RawError::FileTooLarge);
    }
    let sensor = dng::decode(data)?;
    let pixels = u64::from(sensor.width) * u64::from(sensor.height);
    if pixels > RAW_MAX_PIXELS {
        return Err(RawError::FileTooLarge);
    }
    develop_sensor(&sensor, parameters, scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::raw::dng::fixtures::DngBuilder;

    /// A sensor whose photosites all sit at the same fraction of full scale, so
    /// the developed result should be a flat neutral image.
    fn flat_dng(width: u32, height: u32, level: u16, white: u32) -> Vec<u8> {
        DngBuilder::new(width, height, vec![level; (width * height) as usize])
            .levels(vec![0], (1, 1), white)
            .build()
    }

    fn parameters() -> DevelopmentParameters {
        DevelopmentParameters::default()
    }

    #[test]
    fn develops_a_flat_sensor_into_a_flat_linear_image() {
        let data = flat_dng(16, 16, 2048, 4095);
        let developed = develop_bytes(&data, &parameters(), RenderScale::Full).unwrap();
        assert_eq!(developed.image.width(), 16);
        assert_eq!(developed.source_dimensions, (16, 16));
        let expected = 2048.0 / 4095.0;
        for pixel in developed.image.pixels() {
            for value in [pixel.red, pixel.green, pixel.blue] {
                assert!(
                    (value - expected).abs() < 1e-4,
                    "expected {expected}, got {value}"
                );
            }
            assert_eq!(pixel.alpha, 1.0);
        }
    }

    /// The point of a RAW pipeline: a highlight above the nominal white level
    /// is still there after development, and exposure can bring it back.
    #[test]
    fn a_highlight_above_white_survives_and_can_be_recovered() {
        // Every photosite is 25% above the declared white level.
        let data = flat_dng(8, 8, 5000, 4000);
        let developed = develop_bytes(&data, &parameters(), RenderScale::Full).unwrap();
        let value = developed.image.pixels()[0].green;
        assert!(
            value > 1.0,
            "the highlight was clipped during development: {value}"
        );

        // Pulling exposure down by one stop must bring it back below white,
        // which is only possible because nothing clipped it on the way.
        let mut darker = parameters();
        darker.exposure_ev = -1.0;
        let recovered = develop_bytes(&data, &darker, RenderScale::Full).unwrap();
        let recovered_value = recovered.image.pixels()[0].green;
        assert!(
            recovered_value < 1.0,
            "exposure could not recover the highlight: {recovered_value}"
        );
        assert!((recovered_value - value * 0.5).abs() < 1e-4);
    }

    /// The same scene developed through an 8-bit path first would have lost the
    /// headroom entirely; this pins the difference.
    #[test]
    fn eight_bit_quantisation_would_have_destroyed_that_headroom() {
        let data = flat_dng(4, 4, 5000, 4000);
        let developed = develop_bytes(&data, &parameters(), RenderScale::Full).unwrap();
        let linear = developed.image.pixels()[0].green;
        // What an early 8-bit conversion would have kept.
        let through_eight_bit = (linear.min(1.0) * 255.0).round() / 255.0;
        let mut darker = parameters();
        darker.exposure_ev = -1.0;
        let recovered = develop_bytes(&data, &darker, RenderScale::Full)
            .unwrap()
            .image
            .pixels()[0]
            .green;
        assert!(
            (recovered - through_eight_bit * 0.5).abs() > 0.05,
            "the linear path recovered no more than an 8-bit path would have"
        );
    }

    #[test]
    fn as_shot_white_balance_uses_the_camera_neutral_from_the_file() {
        let samples = vec![1000u16; 64];
        let data = DngBuilder::new(8, 8, samples)
            .levels(vec![0], (1, 1), 4095)
            .neutral([0.5, 1.0, 0.8])
            .build();
        let developed = develop_bytes(&data, &parameters(), RenderScale::Full).unwrap();
        // The neutral is what the camera divides by, so red is multiplied by
        // two relative to green.
        assert!((developed.multipliers[0] - 2.0).abs() < 1e-3);
        assert!((developed.multipliers[1] - 1.0).abs() < 1e-6);
        assert!((developed.multipliers[2] - 1.25).abs() < 1e-3);
    }

    #[test]
    fn as_shot_falls_back_to_neutral_when_the_file_carries_no_white_balance() {
        let data = flat_dng(8, 8, 1000, 4095);
        let developed = develop_bytes(&data, &parameters(), RenderScale::Full).unwrap();
        assert_eq!(developed.multipliers, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn custom_white_balance_overrides_the_camera_and_is_normalised_to_green() {
        let data = DngBuilder::new(8, 8, vec![1000u16; 64])
            .levels(vec![0], (1, 1), 4095)
            .neutral([0.5, 1.0, 0.8])
            .build();
        let mut params = parameters();
        params.white_balance = WhiteBalance::Custom {
            multipliers: [4.0, 2.0, 1.0],
        };
        let developed = develop_bytes(&data, &params, RenderScale::Full).unwrap();
        assert_eq!(developed.multipliers, [2.0, 1.0, 0.5]);
    }

    #[test]
    fn auto_white_balance_equalises_the_channel_averages() {
        // A scene with a strong red cast: red photosites read high.
        let (width, height) = (16u32, 16u32);
        let mut samples = vec![0u16; (width * height) as usize];
        for y in 0..height {
            for x in 0..width {
                let value = match dng::CfaPattern::RGGB.color_at(x, y) {
                    dng::CfaColor::Red => 2000,
                    dng::CfaColor::Green => 1000,
                    dng::CfaColor::Blue => 500,
                };
                samples[(y * width + x) as usize] = value;
            }
        }
        let data = DngBuilder::new(width, height, samples)
            .levels(vec![0], (1, 1), 4095)
            .build();
        let mut params = parameters();
        params.white_balance = WhiteBalance::Auto;
        let developed = develop_bytes(&data, &params, RenderScale::Full).unwrap();
        // Grey world: red is halved, blue is doubled, green is the reference.
        assert!((developed.multipliers[0] - 0.5).abs() < 1e-3);
        assert!((developed.multipliers[1] - 1.0).abs() < 1e-6);
        assert!((developed.multipliers[2] - 2.0).abs() < 1e-3);

        // And the developed image is then close to neutral.
        let pixel = developed.image.pixels()[(height / 2 * width + width / 2) as usize];
        assert!((pixel.red - pixel.green).abs() < 0.02);
        assert!((pixel.blue - pixel.green).abs() < 0.02);
    }

    #[test]
    fn exposure_is_a_power_of_two_in_linear_light() {
        let data = flat_dng(8, 8, 1000, 4095);
        let base = develop_bytes(&data, &parameters(), RenderScale::Full).unwrap();
        let mut brighter = parameters();
        brighter.exposure_ev = 1.0;
        let lifted = develop_bytes(&data, &brighter, RenderScale::Full).unwrap();
        let ratio = lifted.image.pixels()[0].green / base.image.pixels()[0].green;
        assert!((ratio - 2.0).abs() < 1e-4, "one stop scaled by {ratio}");
    }

    #[test]
    fn a_colour_matrix_is_applied_and_reported() {
        let data = flat_dng(8, 8, 1000, 4095);
        let plain = develop_bytes(&data, &parameters(), RenderScale::Full).unwrap();
        assert!(!plain.color_managed, "a matrix was claimed but none exists");

        let with_matrix = DngBuilder::new(8, 8, vec![1000u16; 64])
            .levels(vec![0], (1, 1), 4095)
            .matrix([0.7, 0.2, 0.1, 0.2, 0.7, 0.1, 0.1, 0.2, 0.7])
            .build();
        let managed = develop_bytes(&with_matrix, &parameters(), RenderScale::Full).unwrap();
        assert!(managed.color_managed);
        // The matrix rows are normalised for white, so a neutral sensor signal
        // stays neutral after conversion.
        let pixel = managed.image.pixels()[0];
        assert!((pixel.red - pixel.green).abs() < 1e-3);
        assert!((pixel.blue - pixel.green).abs() < 1e-3);
    }

    #[test]
    fn a_preview_is_smaller_and_uses_the_fast_algorithm() {
        // A sensor well above the preview ceiling.
        let (width, height) = (4000u32, 40u32);
        let data = DngBuilder::new(width, height, vec![1200u16; (width * height) as usize])
            .levels(vec![0], (1, 1), 4095)
            .build();
        let preview = develop_bytes(&data, &parameters(), RenderScale::Preview).unwrap();
        assert!(
            preview.image.width() < width,
            "the preview was not decimated: {}",
            preview.image.width()
        );
        assert_eq!(preview.demosaic, Quality::Fast);
        // The full render keeps every photosite and the better algorithm.
        let full = develop_bytes(&data, &parameters(), RenderScale::Full).unwrap();
        assert_eq!(full.image.width(), width);
        assert_eq!(full.demosaic, Quality::High);
        // Both report the true sensor size regardless of what they rendered.
        assert_eq!(preview.source_dimensions, (width, height));
        assert_eq!(full.source_dimensions, (width, height));
    }

    /// A preview must look like the picture, not merely be smaller.
    #[test]
    fn a_preview_of_a_flat_scene_matches_the_full_render() {
        let (width, height) = (3200u32, 32u32);
        let data = DngBuilder::new(width, height, vec![2000u16; (width * height) as usize])
            .levels(vec![0], (1, 1), 4095)
            .neutral([0.5, 1.0, 0.8])
            .build();
        let preview = develop_bytes(&data, &parameters(), RenderScale::Preview).unwrap();
        let full = develop_bytes(&data, &parameters(), RenderScale::Full).unwrap();
        assert_eq!(preview.multipliers, full.multipliers);
        let preview_pixel = preview.image.pixels()[preview.image.pixels().len() / 2];
        let full_pixel = full.image.pixels()[full.image.pixels().len() / 2];
        assert!((preview_pixel.red - full_pixel.red).abs() < 1e-3);
        assert!((preview_pixel.green - full_pixel.green).abs() < 1e-3);
    }

    #[test]
    fn a_small_sensor_is_not_decimated_at_all() {
        let data = flat_dng(64, 48, 1000, 4095);
        let preview = develop_bytes(&data, &parameters(), RenderScale::Preview).unwrap();
        assert_eq!((preview.image.width(), preview.image.height()), (64, 48));
    }

    #[test]
    fn development_is_deterministic() {
        let data = DngBuilder::new(16, 16, (0..256).map(|i| (i * 13 % 4000) as u16).collect())
            .levels(vec![100], (1, 1), 4095)
            .neutral([0.6, 1.0, 0.9])
            .matrix([0.8, 0.1, 0.1, 0.1, 0.8, 0.1, 0.1, 0.1, 0.8])
            .build();
        let first = develop_bytes(&data, &parameters(), RenderScale::Full).unwrap();
        let second = develop_bytes(&data, &parameters(), RenderScale::Full).unwrap();
        assert_eq!(first.image, second.image);
    }

    #[test]
    fn invalid_development_parameters_are_refused() {
        let data = flat_dng(8, 8, 1000, 4095);
        let mut params = parameters();
        params.exposure_ev = f32::NAN;
        assert!(develop_bytes(&data, &params, RenderScale::Full).is_err());
        params.exposure_ev = 99.0;
        assert!(develop_bytes(&data, &params, RenderScale::Full).is_err());
    }

    #[test]
    fn a_corrupt_or_empty_buffer_is_refused_without_panicking() {
        for data in [
            Vec::new(),
            b"not a raw file at all".to_vec(),
            vec![0xFF; 4096],
        ] {
            assert!(develop_bytes(&data, &parameters(), RenderScale::Full).is_err());
        }
    }

    #[test]
    fn every_developed_value_is_finite() {
        let data = DngBuilder::new(16, 16, (0..256).map(|i| (i * 251 % 4096) as u16).collect())
            .levels(vec![64], (1, 1), 4095)
            .neutral([0.4, 1.0, 0.7])
            .matrix([1.2, -0.2, 0.0, -0.1, 1.1, 0.0, 0.0, -0.3, 1.3])
            .build();
        let mut params = parameters();
        params.exposure_ev = 2.5;
        params.highlights = -0.8;
        params.shadows = 0.9;
        params.contrast = 0.6;
        let developed = develop_bytes(&data, &params, RenderScale::Full).unwrap();
        for pixel in developed.image.pixels() {
            assert!(
                pixel.red.is_finite() && pixel.green.is_finite() && pixel.blue.is_finite(),
                "a non-finite value reached the working image: {pixel:?}"
            );
        }
    }
}
