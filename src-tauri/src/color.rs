//! High-precision colour and photographic-development primitives.
//!
//! The legacy editor deliberately keeps its established `RgbaImage`/sRGB
//! boundary.  Phase 9 work uses this module as an explicit opt-in boundary:
//! encoded source samples are decoded once into straight-alpha, linear-sRGB
//! `f32` values; development and future compositing operate on those values;
//! display and export encode and clamp only at the edge.  The representation
//! intentionally permits negative and above-white intermediates so exposure
//! and tone operations do not clip prematurely.

use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder, RgbaImage};
use serde::{Deserialize, Serialize};

const MAX_FLOAT_PIXELS: u64 = crate::resources::MAX_WORKING_PIXELS;
const MAX_FLOAT_DIMENSION: u32 = 20_000;
const SRGB_DECODE_BREAK: f32 = 0.04045;
const SRGB_ENCODE_BREAK: f32 = 0.003_130_8;
const SRGB_ALPHA_EPSILON: f32 = 1.0e-6;

/// Errors raised before a high-precision buffer is allowed to allocate or be
/// converted to an output format.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ColorPipelineError {
    #[error("the float image dimensions are invalid")]
    InvalidDimensions,
    #[error("the float image contains an invalid pixel count")]
    InvalidPixelCount,
    #[error("a colour value is not finite")]
    NonFiniteValue,
    #[error("alpha must be between zero and one")]
    InvalidAlpha,
    #[error("the image allocation exceeds available resources")]
    Allocation,
    #[error("development parameters are invalid: {0}")]
    InvalidParameters(String),
    #[error("PNG encoding failed: {0}")]
    Encoding(String),
}

/// The colour encoding at an image boundary.  Only sRGB is transformed in
/// this phase; other spaces are retained as metadata and rejected by callers
/// until a real ICC transform is available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorEncoding {
    Srgb,
    LinearSrgb,
    DisplayP3,
    AdobeRgb,
    EmbeddedIcc,
}

/// The working space used by the Phase 9 development primitives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WorkingColorSpace {
    /// Linear-light sRGB primaries, with an unbounded float transfer domain.
    #[default]
    LinearSrgb,
}

/// Straight-alpha linear RGB plus alpha.  RGB is deliberately not clamped;
/// values below zero and above one are useful between source decoding and
/// output quantisation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "gpu", derive(bytemuck::Pod, bytemuck::Zeroable))]
#[serde(rename_all = "camelCase")]
// Fixed layout so a buffer of these can be handed to a GPU, or read back from
// one, without copying field by field. Four f32 in declared order, no padding.
#[repr(C)]
pub struct FloatRgba {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl FloatRgba {
    pub const BLACK: Self = Self {
        red: 0.0,
        green: 0.0,
        blue: 0.0,
        alpha: 1.0,
    };

    pub const TRANSPARENT: Self = Self {
        red: 0.0,
        green: 0.0,
        blue: 0.0,
        alpha: 0.0,
    };

    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    /// Decodes an ordinary 8-bit sRGB pixel into linear RGB.  Alpha is a
    /// coverage value and is therefore not gamma transformed.
    pub fn from_srgba8(pixel: [u8; 4]) -> Self {
        Self::new(
            srgb_decode(f32::from(pixel[0]) / 255.0),
            srgb_decode(f32::from(pixel[1]) / 255.0),
            srgb_decode(f32::from(pixel[2]) / 255.0),
            f32::from(pixel[3]) / 255.0,
        )
    }

    /// Decodes a native-endian 16-bit sRGB pixel into linear RGB.
    pub fn from_srgba16(pixel: [u16; 4]) -> Self {
        Self::new(
            srgb_decode(f32::from(pixel[0]) / 65_535.0),
            srgb_decode(f32::from(pixel[1]) / 65_535.0),
            srgb_decode(f32::from(pixel[2]) / 65_535.0),
            f32::from(pixel[3]) / 65_535.0,
        )
    }

    /// Encodes to display-referred 8-bit sRGB.  Out-of-range and negative
    /// working values are clipped only here.
    pub fn to_srgba8(self) -> [u8; 4] {
        [
            quantize_u8(srgb_encode(self.red)),
            quantize_u8(srgb_encode(self.green)),
            quantize_u8(srgb_encode(self.blue)),
            quantize_u8(self.alpha),
        ]
    }

    /// Encodes to display-referred 16-bit sRGB.  This is a genuine 16-bit
    /// quantisation of the float result, not an expansion of an 8-bit value.
    pub fn to_srgba16(self) -> [u16; 4] {
        [
            quantize_u16(srgb_encode(self.red)),
            quantize_u16(srgb_encode(self.green)),
            quantize_u16(srgb_encode(self.blue)),
            quantize_u16(self.alpha),
        ]
    }

    pub fn is_finite(self) -> bool {
        [self.red, self.green, self.blue, self.alpha]
            .iter()
            .all(|value| value.is_finite())
    }

    pub fn luminance(self) -> f32 {
        self.red * 0.2126 + self.green * 0.7152 + self.blue * 0.0722
    }
}

/// A bounded, row-major high-precision image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloatImage {
    width: u32,
    height: u32,
    pixels: Vec<FloatRgba>,
}

impl FloatImage {
    pub fn new(
        width: u32,
        height: u32,
        pixels: Vec<FloatRgba>,
    ) -> Result<Self, ColorPipelineError> {
        let count = checked_pixel_count(width, height)?;
        if usize::try_from(count).map_err(|_| ColorPipelineError::InvalidPixelCount)?
            != pixels.len()
        {
            return Err(ColorPipelineError::InvalidPixelCount);
        }
        if pixels.iter().any(|pixel| !pixel.is_finite()) {
            return Err(ColorPipelineError::NonFiniteValue);
        }
        if pixels
            .iter()
            .any(|pixel| !(0.0..=1.0).contains(&pixel.alpha))
        {
            return Err(ColorPipelineError::InvalidAlpha);
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    pub fn blank(width: u32, height: u32, fill: FloatRgba) -> Result<Self, ColorPipelineError> {
        let count = checked_pixel_count(width, height)?;
        if !fill.is_finite() {
            return Err(ColorPipelineError::NonFiniteValue);
        }
        let count = usize::try_from(count).map_err(|_| ColorPipelineError::InvalidPixelCount)?;
        let mut pixels = Vec::new();
        pixels
            .try_reserve_exact(count)
            .map_err(|_| ColorPipelineError::Allocation)?;
        pixels.resize(count, fill);
        Self::new(width, height, pixels)
    }

    pub fn from_rgba8(image: &RgbaImage) -> Result<Self, ColorPipelineError> {
        checked_pixel_count(image.width(), image.height())?;
        let pixels = image
            .pixels()
            .map(|pixel| FloatRgba::from_srgba8(pixel.0))
            .collect();
        Self::new(image.width(), image.height(), pixels)
    }

    /// Retain native source precision; float DynamicImage channels are encoded
    /// sRGB here, matching the ordinary decoder's explicit input contract.
    pub fn from_dynamic(image: &image::DynamicImage) -> Result<Self, ColorPipelineError> {
        checked_pixel_count(image.width(), image.height())?;
        match image {
            image::DynamicImage::ImageRgba8(rgba) => Self::from_rgba8(rgba),
            _ => {
                let encoded = image.to_rgba32f();
                Self::new(
                    image.width(),
                    image.height(),
                    encoded
                        .pixels()
                        .map(|p| {
                            FloatRgba::new(
                                srgb_decode(p[0]),
                                srgb_decode(p[1]),
                                srgb_decode(p[2]),
                                p[3],
                            )
                        })
                        .collect(),
                )
            }
        }
    }

    pub fn validate(&self) -> Result<(), ColorPipelineError> {
        let count = checked_pixel_count(self.width, self.height)?;
        if count as usize != self.pixels.len() {
            return Err(ColorPipelineError::InvalidPixelCount);
        }
        if self.pixels.iter().any(|p| !p.is_finite()) {
            return Err(ColorPipelineError::NonFiniteValue);
        }
        if self.pixels.iter().any(|p| !(0.0..=1.0).contains(&p.alpha)) {
            return Err(ColorPipelineError::InvalidAlpha);
        }
        Ok(())
    }

    /// Pixel centres are at index + 0.5. Interpolate associated color so fully
    /// transparent neighbors cannot contribute hidden RGB to an edge.
    pub fn sample(&self, x: f32, y: f32, nearest: bool) -> FloatRgba {
        if !x.is_finite()
            || !y.is_finite()
            || x < 0.0
            || y < 0.0
            || x >= self.width as f32
            || y >= self.height as f32
        {
            return FloatRgba::TRANSPARENT;
        }
        if nearest {
            return self
                .get(x as u32, y as u32)
                .unwrap_or(FloatRgba::TRANSPARENT);
        }
        let sx = x - 0.5;
        let sy = y - 0.5;
        let bx = sx.floor() as i64;
        let by = sy.floor() as i64;
        let fx = sx - sx.floor();
        let fy = sy - sy.floor();
        if fx == 0.0 && fy == 0.0 {
            return self
                .get(bx as u32, by as u32)
                .unwrap_or(FloatRgba::TRANSPARENT);
        }
        let mut sum = [0.0_f32; 4];
        for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
            for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
                let weight = wx * wy;
                if weight <= 0.0 {
                    continue;
                }
                if let Some(p) = self.get((bx + dx) as u32, (by + dy) as u32) {
                    let a = p.alpha * weight;
                    sum[0] += p.red * a;
                    sum[1] += p.green * a;
                    sum[2] += p.blue * a;
                    sum[3] += a;
                }
            }
        }
        if sum[3] <= 0.0 {
            FloatRgba::TRANSPARENT
        } else {
            FloatRgba::new(
                sum[0] / sum[3],
                sum[1] / sum[3],
                sum[2] / sum[3],
                sum[3].clamp(0.0, 1.0),
            )
        }
    }

    pub fn resized(&self, width: u32, height: u32) -> Result<Self, ColorPipelineError> {
        let mut result = Self::blank(width, height, FloatRgba::TRANSPARENT)?;
        for y in 0..height {
            for x in 0..width {
                let sx = ((x as f32 + 0.5) * self.width as f32 / width as f32)
                    .clamp(0.5, self.width as f32 - 0.5);
                let sy = ((y as f32 + 0.5) * self.height as f32 / height as f32)
                    .clamp(0.5, self.height as f32 - 0.5);
                result.pixels[(y * width + x) as usize] = self.sample(sx, sy, false);
            }
        }
        Ok(result)
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn pixels(&self) -> &[FloatRgba] {
        &self.pixels
    }

    pub fn pixels_mut(&mut self) -> &mut [FloatRgba] {
        &mut self.pixels
    }

    pub fn get(&self, x: u32, y: u32) -> Option<FloatRgba> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = usize::try_from(y)
            .ok()?
            .checked_mul(usize::try_from(self.width).ok()?)?
            .checked_add(usize::try_from(x).ok()?)?;
        self.pixels.get(index).copied()
    }

    pub fn to_rgba8(&self) -> RgbaImage {
        let bytes = self
            .pixels
            .iter()
            .flat_map(|pixel| pixel.to_srgba8())
            .collect::<Vec<_>>();
        // The invariant was checked on construction, so this cannot fail.
        RgbaImage::from_raw(self.width, self.height, bytes).expect("validated float image size")
    }

    /// Returns native-endian channel bytes suitable for `PngEncoder`'s
    /// `Rgba16` path.
    pub fn to_rgba16_native_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(self.pixels.len().saturating_mul(8));
        for pixel in &self.pixels {
            for channel in pixel.to_srgba16() {
                bytes.extend_from_slice(&channel.to_ne_bytes());
            }
        }
        bytes
    }

    /// Encodes the developed float image as true 16-bit RGBA PNG.
    pub fn encode_png16(&self) -> Result<Vec<u8>, ColorPipelineError> {
        let mut bytes = Vec::new();
        PngEncoder::new(&mut bytes)
            .write_image(
                &self.to_rgba16_native_bytes(),
                self.width,
                self.height,
                ExtendedColorType::Rgba16,
            )
            .map_err(|error| ColorPipelineError::Encoding(error.to_string()))?;
        Ok(bytes)
    }
}

fn checked_pixel_count(width: u32, height: u32) -> Result<u64, ColorPipelineError> {
    if width == 0 || height == 0 || width > MAX_FLOAT_DIMENSION || height > MAX_FLOAT_DIMENSION {
        return Err(ColorPipelineError::InvalidDimensions);
    }
    let count = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(ColorPipelineError::InvalidPixelCount)?;
    if count > MAX_FLOAT_PIXELS {
        return Err(ColorPipelineError::InvalidPixelCount);
    }
    Ok(count)
}

/// IEC 61966-2-1 sRGB transfer to linear light.  The sign-preserving extension
/// keeps mathematical intermediates useful without changing nominal [0, 1]
/// behaviour.
pub fn srgb_decode(value: f32) -> f32 {
    if !value.is_finite() {
        return value;
    }
    let sign = value.signum();
    let magnitude = value.abs();
    sign * if magnitude <= SRGB_DECODE_BREAK {
        magnitude / 12.92
    } else {
        ((magnitude + 0.055) / 1.055).powf(2.4)
    }
}

/// IEC 61966-2-1 linear-light transfer to sRGB, extended by sign for values
/// outside the display gamut.  Output encoders clamp the result afterwards.
pub fn srgb_encode(value: f32) -> f32 {
    if !value.is_finite() {
        return value;
    }
    let sign = value.signum();
    let magnitude = value.abs();
    sign * if magnitude <= SRGB_ENCODE_BREAK {
        magnitude * 12.92
    } else {
        1.055 * magnitude.powf(1.0 / 2.4) - 0.055
    }
}

fn quantize_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn quantize_u16(value: f32) -> u16 {
    (value.clamp(0.0, 1.0) * 65_535.0).round() as u16
}

/// White-balance modes kept separate from ordinary adjustment layers.  The
/// temperature control is intentionally normalised rather than labelled in
/// Kelvin: this phase does not have a calibrated camera/illuminant model and
/// therefore must not imply false precision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum WhiteBalance {
    AsShot { multipliers: [f32; 3] },
    Auto,
    TemperatureTint { temperature: f32, tint: f32 },
    Custom { multipliers: [f32; 3] },
}

impl Default for WhiteBalance {
    fn default() -> Self {
        Self::AsShot {
            multipliers: [1.0, 1.0, 1.0],
        }
    }
}

/// Non-destructive RAW/development controls.  Values are applied to a linear
/// working image and remain distinct from legacy 8-bit `EditOperation`s.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DevelopmentParameters {
    pub white_balance: WhiteBalance,
    /// Exposure in photographic stops: +1.0 doubles linear intensity.
    pub exposure_ev: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
}

impl Default for DevelopmentParameters {
    fn default() -> Self {
        Self {
            white_balance: WhiteBalance::default(),
            exposure_ev: 0.0,
            contrast: 0.0,
            highlights: 0.0,
            shadows: 0.0,
            whites: 0.0,
            blacks: 0.0,
        }
    }
}

impl DevelopmentParameters {
    pub fn validate(&self) -> Result<(), ColorPipelineError> {
        let valid_scalar = [
            self.exposure_ev,
            self.contrast,
            self.highlights,
            self.shadows,
            self.whites,
            self.blacks,
        ]
        .iter()
        .all(|value| value.is_finite());
        if !valid_scalar {
            return Err(ColorPipelineError::InvalidParameters(
                "development values must be finite".into(),
            ));
        }
        if !(-8.0..=8.0).contains(&self.exposure_ev) {
            return Err(ColorPipelineError::InvalidParameters(
                "exposure must be between -8 and +8 EV".into(),
            ));
        }
        if [
            self.contrast,
            self.highlights,
            self.shadows,
            self.whites,
            self.blacks,
        ]
        .iter()
        .any(|value| !(-1.0..=1.0).contains(value))
        {
            return Err(ColorPipelineError::InvalidParameters(
                "tone controls must be between -1 and +1".into(),
            ));
        }
        match &self.white_balance {
            WhiteBalance::AsShot { multipliers } | WhiteBalance::Custom { multipliers } => {
                if multipliers
                    .iter()
                    .any(|value| !value.is_finite() || !(0.01..=16.0).contains(value))
                {
                    return Err(ColorPipelineError::InvalidParameters(
                        "white-balance multipliers must be finite and between 0.01 and 16".into(),
                    ));
                }
            }
            WhiteBalance::Auto => {}
            WhiteBalance::TemperatureTint { temperature, tint } => {
                if !temperature.is_finite()
                    || !tint.is_finite()
                    || !(-1.0..=1.0).contains(temperature)
                    || !(-1.0..=1.0).contains(tint)
                {
                    return Err(ColorPipelineError::InvalidParameters(
                        "temperature and tint must be finite and between -1 and +1".into(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// Applies a deterministic photographic development pass in-place.  No
/// operation clips RGB; the display/export conversion is the first clipping
/// boundary.  White balance uses channel multipliers, exposure uses `2^EV`,
/// contrast pivots around middle grey, and the remaining controls use smooth
/// luminance weights to avoid hard halos.
pub fn apply_development(
    image: &mut FloatImage,
    parameters: &DevelopmentParameters,
) -> Result<(), ColorPipelineError> {
    parameters.validate()?;
    let white_balance = resolve_white_balance(image, &parameters.white_balance);
    let exposure = 2.0_f32.powf(parameters.exposure_ev);

    for pixel in image.pixels_mut() {
        pixel.red *= white_balance[0] * exposure;
        pixel.green *= white_balance[1] * exposure;
        pixel.blue *= white_balance[2] * exposure;

        let luminance = pixel.luminance();
        let contrast_luminance = (luminance - 0.18) * (1.0 + parameters.contrast) + 0.18;
        let contrast_scale = if luminance.abs() > SRGB_ALPHA_EPSILON {
            contrast_luminance / luminance
        } else {
            1.0 + parameters.contrast
        };
        pixel.red *= contrast_scale;
        pixel.green *= contrast_scale;
        pixel.blue *= contrast_scale;

        let adjusted_luminance = pixel.luminance();
        let highlight_weight = smoothstep(0.5, 1.0, adjusted_luminance.max(0.0));
        let shadow_weight = 1.0 - smoothstep(0.0, 0.5, adjusted_luminance.max(0.0));
        let white_weight = smoothstep(0.75, 1.0, adjusted_luminance.max(0.0));
        let black_weight = 1.0 - smoothstep(0.0, 0.25, adjusted_luminance.max(0.0));
        let highlight_scale = 1.0 + parameters.highlights * 0.35 * highlight_weight;
        let shadow_scale = 1.0 + parameters.shadows * 0.25 * shadow_weight;
        let additive =
            parameters.whites * 0.20 * white_weight + parameters.blacks * 0.20 * black_weight;
        pixel.red = pixel.red * highlight_scale * shadow_scale + additive;
        pixel.green = pixel.green * highlight_scale * shadow_scale + additive;
        pixel.blue = pixel.blue * highlight_scale * shadow_scale + additive;
    }
    Ok(())
}

fn resolve_white_balance(image: &FloatImage, white_balance: &WhiteBalance) -> [f32; 3] {
    match white_balance {
        WhiteBalance::AsShot { multipliers } | WhiteBalance::Custom { multipliers } => *multipliers,
        WhiteBalance::TemperatureTint { temperature, tint } => {
            // A deliberately modest, normalised control.  Calibrated Kelvin
            // conversion belongs with a real camera profile/ICC backend.
            [
                (1.0 + temperature * 0.25).clamp(0.5, 1.5),
                (1.0 + tint * 0.08).clamp(0.5, 1.5),
                (1.0 - temperature * 0.25).clamp(0.5, 1.5),
            ]
        }
        WhiteBalance::Auto => {
            let mut sums = [0.0_f64; 3];
            let mut count = 0_u64;
            for pixel in image.pixels() {
                if pixel.alpha <= 0.0 {
                    continue;
                }
                sums[0] += f64::from(pixel.red.max(0.0));
                sums[1] += f64::from(pixel.green.max(0.0));
                sums[2] += f64::from(pixel.blue.max(0.0));
                count += 1;
            }
            if count == 0 {
                return [1.0, 1.0, 1.0];
            }
            let means = [
                (sums[0] / count as f64) as f32,
                (sums[1] / count as f64) as f32,
                (sums[2] / count as f64) as f32,
            ];
            let target = (means[0] + means[1] + means[2]) / 3.0;
            [
                safe_ratio(target, means[0]),
                safe_ratio(target, means[1]),
                safe_ratio(target, means[2]),
            ]
        }
    }
}

fn safe_ratio(numerator: f32, denominator: f32) -> f32 {
    if denominator.abs() <= SRGB_ALPHA_EPSILON {
        1.0
    } else {
        (numerator / denominator).clamp(0.25, 4.0)
    }
}

fn smoothstep(edge0: f32, edge1: f32, value: f32) -> f32 {
    if edge1 <= edge0 {
        return f32::from(value >= edge1);
    }
    let t = ((value - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A bounded RGB/luminance histogram for the linear development stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FloatHistogram {
    pub red: Vec<u64>,
    pub green: Vec<u64>,
    pub blue: Vec<u64>,
    pub luminance: Vec<u64>,
    pub clipped_highlights: u64,
    pub clipped_shadows: u64,
    pub sampled_pixels: u64,
}

impl Default for FloatHistogram {
    fn default() -> Self {
        Self {
            red: vec![0; 256],
            green: vec![0; 256],
            blue: vec![0; 256],
            luminance: vec![0; 256],
            clipped_highlights: 0,
            clipped_shadows: 0,
            sampled_pixels: 0,
        }
    }
}

pub fn histogram(image: &FloatImage, max_samples: usize) -> FloatHistogram {
    let mut result = FloatHistogram::default();
    let stride = (image.pixels().len() / max_samples.max(1)).max(1);
    for pixel in image.pixels().iter().step_by(stride) {
        result.sampled_pixels += 1;
        add_channel(
            &mut result.red,
            pixel.red,
            &mut result.clipped_highlights,
            &mut result.clipped_shadows,
        );
        add_channel(
            &mut result.green,
            pixel.green,
            &mut result.clipped_highlights,
            &mut result.clipped_shadows,
        );
        add_channel(
            &mut result.blue,
            pixel.blue,
            &mut result.clipped_highlights,
            &mut result.clipped_shadows,
        );
        add_channel(
            &mut result.luminance,
            pixel.luminance(),
            &mut result.clipped_highlights,
            &mut result.clipped_shadows,
        );
    }
    result
}

fn add_channel(bins: &mut [u64], value: f32, highlights: &mut u64, shadows: &mut u64) {
    if value > 1.0 {
        *highlights += 1;
    }
    if value < 0.0 {
        *shadows += 1;
    }
    let index = (value.clamp(0.0, 1.0) * 255.0).round() as usize;
    bins[index.min(255)] += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: f32, expected: f32, tolerance: f32) {
        assert!(
            (actual - expected).abs() <= tolerance,
            "{actual} vs {expected}"
        );
    }

    #[test]
    fn srgb_reference_points_decode_and_encode() {
        close(srgb_decode(0.0), 0.0, 1.0e-7);
        close(srgb_decode(1.0), 1.0, 1.0e-6);
        close(srgb_decode(0.5), 0.21404114, 1.0e-6);
        close(srgb_encode(0.0), 0.0, 1.0e-7);
        close(srgb_encode(1.0), 1.0, 1.0e-6);
        close(srgb_encode(0.21404114), 0.5, 1.0e-6);
    }

    #[test]
    fn transfer_round_trip_preserves_nominal_values() {
        for index in 0..=1000 {
            let value = index as f32 / 1000.0;
            close(srgb_encode(srgb_decode(value)), value, 2.0e-6);
        }
    }

    #[test]
    fn transfer_preserves_signed_extended_intermediates() {
        close(srgb_decode(-0.5), -srgb_decode(0.5), 1.0e-6);
        close(srgb_encode(1.5), 1.1941764, 2.0e-5);
        close(srgb_encode(-0.25), -srgb_encode(0.25), 1.0e-6);
    }

    #[test]
    fn eight_and_sixteen_bit_boundaries_use_alpha_without_gamma() {
        let pixel = FloatRgba::from_srgba8([128, 64, 255, 32]);
        assert_eq!(pixel.to_srgba8(), [128, 64, 255, 32]);
        assert_eq!(
            FloatRgba::from_srgba16([65_535, 0, 32_768, 16_384]).to_srgba16(),
            [65_535, 0, 32_768, 16_384]
        );
    }

    #[test]
    fn float_image_rejects_bad_dimensions_and_non_finite_pixels() {
        assert_eq!(
            FloatImage::new(0, 1, Vec::new()),
            Err(ColorPipelineError::InvalidDimensions)
        );
        assert_eq!(
            FloatImage::new(1, 1, vec![FloatRgba::new(f32::NAN, 0.0, 0.0, 1.0)]),
            Err(ColorPipelineError::NonFiniteValue)
        );
        assert_eq!(
            FloatImage::new(2, 2, vec![FloatRgba::BLACK; 3]),
            Err(ColorPipelineError::InvalidPixelCount)
        );
    }

    #[test]
    fn exposure_is_stop_based_and_does_not_clip_float_values() {
        let mut image = FloatImage::new(1, 1, vec![FloatRgba::new(0.25, 0.5, 1.0, 1.0)]).unwrap();
        let parameters = DevelopmentParameters {
            exposure_ev: 1.0,
            ..DevelopmentParameters::default()
        };
        apply_development(&mut image, &parameters).unwrap();
        let pixel = image.get(0, 0).unwrap();
        close(pixel.red, 0.5, 1.0e-6);
        close(pixel.green, 1.0, 1.0e-6);
        close(pixel.blue, 2.0, 1.0e-6);
    }

    #[test]
    fn auto_white_balance_is_deterministic_and_ignores_transparent_pixels() {
        let mut image = FloatImage::new(
            2,
            1,
            vec![
                FloatRgba::new(0.5, 0.25, 0.25, 1.0),
                FloatRgba::new(100.0, 0.0, 0.0, 0.0),
            ],
        )
        .unwrap();
        let parameters = DevelopmentParameters {
            white_balance: WhiteBalance::Auto,
            ..DevelopmentParameters::default()
        };
        apply_development(&mut image, &parameters).unwrap();
        let balanced = image.get(0, 0).unwrap();
        close(balanced.red, balanced.green, 1.0e-6);
        close(balanced.green, balanced.blue, 1.0e-6);
        assert_eq!(balanced.alpha, 1.0);
    }

    #[test]
    fn histogram_is_bounded_and_reports_out_of_range_channels() {
        let image = FloatImage::new(
            2,
            1,
            vec![FloatRgba::new(-0.2, 0.2, 1.2, 1.0), FloatRgba::BLACK],
        )
        .unwrap();
        let histogram = histogram(&image, 1);
        assert_eq!(histogram.sampled_pixels, 1);
        assert!(histogram.clipped_highlights > 0);
        assert!(histogram.clipped_shadows > 0);
    }

    #[test]
    fn development_parameters_round_trip_as_a_separate_wire_contract() {
        let parameters = DevelopmentParameters {
            white_balance: WhiteBalance::TemperatureTint {
                temperature: 0.25,
                tint: -0.1,
            },
            exposure_ev: -1.5,
            highlights: -0.4,
            ..DevelopmentParameters::default()
        };
        let json = serde_json::to_string(&parameters).unwrap();
        let decoded: DevelopmentParameters = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, parameters);
        assert!(json.contains("exposureEv"));
        assert!(json.contains("temperatureTint"));
    }

    #[test]
    fn png16_is_real_high_bit_output() {
        let image = FloatImage::new(1, 1, vec![FloatRgba::new(0.21404114, 0.5, 1.0, 1.0)]).unwrap();
        let png = image.encode_png16().unwrap();
        let decoded = image::load_from_memory(&png).unwrap();
        assert_eq!(decoded.color(), image::ColorType::Rgba16);
        assert_eq!(
            decoded.to_rgba16().get_pixel(0, 0).0,
            [32_768, 48_192, 65_535, 65_535]
        );
    }
}
