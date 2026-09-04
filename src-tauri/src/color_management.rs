//! Named RGB spaces and bounded ICC transforms. The working space is linear
//! sRGB/D65, not a monitor profile. Matrix data: W3C CSS Color 4 section 19.
use crate::color::{srgb_decode, srgb_encode, FloatImage, FloatRgba};
use crate::error::AppError;
use moxcms::{
    ColorProfile, DataColorSpace, Layout, ParsingOptions, ToneReprCurve, TransformOptions,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RgbColorSpace {
    #[default]
    Srgb,
    DisplayP3,
    AdobeRgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColorExportOptions {
    #[serde(default)]
    pub color_space: RgbColorSpace,
    pub bit_depth: u8,
    #[serde(default)]
    pub dither: bool,
}
impl Default for ColorExportOptions {
    fn default() -> Self {
        Self {
            color_space: RgbColorSpace::Srgb,
            bit_depth: 16,
            dither: false,
        }
    }
}
impl ColorExportOptions {
    pub fn validate(&self) -> Result<(), AppError> {
        if !matches!(self.bit_depth, 8 | 16) {
            return Err(AppError::ColorPipeline(
                "output bit depth must be 8 or 16".into(),
            ));
        }
        Ok(())
    }
}

type Matrix = [[f64; 3]; 3];
const SRGB_XYZ: Matrix = [
    [506752.0 / 1228815.0, 87881.0 / 245763.0, 12673.0 / 70218.0],
    [87098.0 / 409605.0, 175762.0 / 245763.0, 12673.0 / 175545.0],
    [7918.0 / 409605.0, 87881.0 / 737289.0, 1001167.0 / 1053270.0],
];
const XYZ_SRGB: Matrix = [
    [12831.0 / 3959.0, -329.0 / 214.0, -1974.0 / 3959.0],
    [
        -851781.0 / 878810.0,
        1648619.0 / 878810.0,
        36519.0 / 878810.0,
    ],
    [705.0 / 12673.0, -2585.0 / 12673.0, 705.0 / 667.0],
];
const P3_XYZ: Matrix = [
    [
        608311.0 / 1250200.0,
        189793.0 / 714400.0,
        198249.0 / 1000160.0,
    ],
    [
        35783.0 / 156275.0,
        247089.0 / 357200.0,
        198249.0 / 2500400.0,
    ],
    [0.0, 32229.0 / 714400.0, 5220557.0 / 5000800.0],
];
const XYZ_P3: Matrix = [
    [
        446124.0 / 178915.0,
        -333277.0 / 357830.0,
        -72051.0 / 178915.0,
    ],
    [-14852.0 / 17905.0, 63121.0 / 35810.0, 423.0 / 17905.0],
    [11844.0 / 330415.0, -50337.0 / 660830.0, 316169.0 / 330415.0],
];
const ADOBE_XYZ: Matrix = [
    [
        573536.0 / 994567.0,
        263643.0 / 1420810.0,
        187206.0 / 994567.0,
    ],
    [
        591459.0 / 1989134.0,
        6239551.0 / 9945670.0,
        374412.0 / 4972835.0,
    ],
    [
        53769.0 / 1989134.0,
        351524.0 / 4972835.0,
        4929758.0 / 4972835.0,
    ],
];
const XYZ_ADOBE: Matrix = [
    [
        1829569.0 / 896150.0,
        -506331.0 / 896150.0,
        -308931.0 / 896150.0,
    ],
    [
        -851781.0 / 878810.0,
        1648619.0 / 878810.0,
        36519.0 / 878810.0,
    ],
    [
        16779.0 / 1248040.0,
        -147721.0 / 1248040.0,
        1266979.0 / 1248040.0,
    ],
];

fn multiply(m: Matrix, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|i| {
        (m[i][0] * f64::from(v[0]) + m[i][1] * f64::from(v[1]) + m[i][2] * f64::from(v[2])) as f32
    })
}

impl RgbColorSpace {
    pub fn name(self) -> &'static str {
        match self {
            Self::Srgb => "sRGB",
            Self::DisplayP3 => "Display P3",
            Self::AdobeRgb => "Adobe RGB (1998)",
        }
    }
    pub fn decode(self, value: f32) -> f32 {
        match self {
            Self::AdobeRgb => value.signum() * value.abs().powf(563.0 / 256.0),
            _ => srgb_decode(value),
        }
    }
    pub fn encode(self, value: f32) -> f32 {
        match self {
            Self::AdobeRgb => value.signum() * value.abs().powf(256.0 / 563.0),
            _ => srgb_encode(value),
        }
    }
    pub fn to_working(self, encoded: [f32; 3]) -> [f32; 3] {
        let linear = encoded.map(|v| self.decode(v));
        match self {
            Self::Srgb => linear,
            Self::DisplayP3 => multiply(XYZ_SRGB, multiply(P3_XYZ, linear)),
            Self::AdobeRgb => multiply(XYZ_SRGB, multiply(ADOBE_XYZ, linear)),
        }
    }
    pub fn from_working(self, linear: [f32; 3]) -> [f32; 3] {
        let result = match self {
            Self::Srgb => linear,
            Self::DisplayP3 => multiply(XYZ_P3, multiply(SRGB_XYZ, linear)),
            Self::AdobeRgb => multiply(XYZ_ADOBE, multiply(SRGB_XYZ, linear)),
        };
        result.map(|v| self.encode(v))
    }
    pub fn profile(self) -> ColorProfile {
        let mut p = match self {
            Self::Srgb => ColorProfile::new_srgb(),
            Self::DisplayP3 => ColorProfile::new_display_p3(),
            Self::AdobeRgb => ColorProfile::new_adobe_rgb(),
        };
        // Matrix/TRC tags are authoritative. In particular do not emit a CICP
        // cinema-P3 identifier for the Display P3/D65 profile.
        p.cicp = None;
        p
    }
    pub fn icc_bytes(self) -> Result<Vec<u8>, AppError> {
        self.profile().encode().map_err(cms_error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhitePoint {
    D50,
    D65,
}
pub fn adapt_xyz(xyz: [f32; 3], from: WhitePoint, to: WhitePoint) -> [f32; 3] {
    if from == to {
        return xyz;
    }
    let matrix = if from == WhitePoint::D65 {
        [
            [
                1.0479297925449969,
                0.022946870601609652,
                -0.05019226628920524,
            ],
            [
                0.02962780877005599,
                0.9904344267538799,
                -0.017073799063418826,
            ],
            [
                -0.009243040646204504,
                0.015055191490298152,
                0.7518742814281371,
            ],
        ]
    } else {
        [
            [0.955473421488075, -0.02309845494876471, 0.06325924320057072],
            [
                -0.0283697093338637,
                1.0099953980813041,
                0.021041441191917323,
            ],
            [
                0.012314014864481998,
                -0.020507649298898964,
                1.330365926242124,
            ],
        ]
    };
    multiply(matrix, xyz)
}

pub const MAX_ICC_BYTES: usize = 4 * 1024 * 1024;
fn cms_error(error: moxcms::CmsError) -> AppError {
    AppError::ColorPipeline(format!("RGB color profile could not be processed: {error}"))
}

pub fn parse_rgb_profile(bytes: &[u8]) -> Result<ColorProfile, AppError> {
    if bytes.len() < 132 || bytes.len() >= MAX_ICC_BYTES {
        return Err(AppError::ColorPipeline(
            "ICC profile size is outside the supported bounds".into(),
        ));
    }
    let tags = u32::from_be_bytes(bytes[128..132].try_into().expect("checked ICC header"));
    if tags > 256 {
        return Err(AppError::ColorPipeline(
            "ICC profile has too many tags".into(),
        ));
    }
    let profile = ColorProfile::new_from_slice_with_options(
        bytes,
        ParsingOptions {
            max_profile_size: MAX_ICC_BYTES,
            max_allowed_clut_size: 1_048_576,
            max_allowed_trc_size: 16_384,
        },
    )
    .map_err(cms_error)?;
    if profile.color_space != DataColorSpace::Rgb {
        return Err(AppError::ColorPipeline(
            "only RGB ICC profiles are supported".into(),
        ));
    }
    Ok(profile)
}

pub fn linear_profile() -> ColorProfile {
    let mut profile = RgbColorSpace::Srgb.profile();
    profile.red_trc = Some(ToneReprCurve::Lut(Vec::new()));
    profile.green_trc = profile.red_trc.clone();
    profile.blue_trc = profile.red_trc.clone();
    profile
}

pub fn import_icc(image: &image::DynamicImage, bytes: &[u8]) -> Result<FloatImage, AppError> {
    let input = parse_rgb_profile(bytes)?;
    let options = TransformOptions {
        prefer_fixed_point: false,
        allow_use_cicp_transfer: false,
        allow_extended_range_rgb_xyz: true,
        ..Default::default()
    };
    let transform = input
        .create_transform_f32(Layout::Rgba, &linear_profile(), Layout::Rgba, options)
        .map_err(cms_error)?;
    let encoded = image.to_rgba32f();
    let mut result = FloatImage::blank(image.width(), image.height(), FloatRgba::TRANSPARENT)?;
    let row_size = image.width() as usize * 4;
    let mut row = vec![0.0_f32; row_size];
    for (source, dest) in encoded
        .as_raw()
        .chunks_exact(row_size)
        .zip(result.pixels_mut().chunks_mut(image.width() as usize))
    {
        transform.transform(source, &mut row).map_err(cms_error)?;
        for ((input, p), output) in source.chunks_exact(4).zip(dest).zip(row.chunks_exact(4)) {
            *p = FloatRgba::new(output[0], output[1], output[2], input[3]);
        }
    }
    result.validate()?;
    Ok(result)
}

/// Ordered 4x4 dither, exactly reproducible, never applied to alpha or endpoints.
pub fn quantize8(value: f32, x: u32, y: u32, dither: bool) -> u8 {
    const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    let delta = if dither && value > 0.0 && value < 1.0 {
        (f32::from(BAYER[(y % 4) as usize][(x % 4) as usize]) + 0.5) / 16.0 - 0.5
    } else {
        0.0
    };
    (value.mul_add(255.0, delta).round().clamp(0.0, 255.0)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    fn close(a: [f32; 3], b: [f32; 3], t: f32) {
        for i in 0..3 {
            assert!((a[i] - b[i]).abs() < t, "{a:?} != {b:?}");
        }
    }
    #[test]
    fn named_spaces_round_trip_extended_and_reference_colors() {
        for space in [
            RgbColorSpace::Srgb,
            RgbColorSpace::DisplayP3,
            RgbColorSpace::AdobeRgb,
        ] {
            for color in [
                [0.0; 3],
                [1.0; 3],
                [0.5; 3],
                [0.2, 0.6, 0.8],
                [-0.1, 1.3, 0.02],
            ] {
                close(space.from_working(space.to_working(color)), color, 2e-5);
            }
        }
    }
    #[test]
    fn p3_red_has_expected_out_of_srgb_coordinates() {
        close(
            RgbColorSpace::DisplayP3.to_working([1.0, 0.0, 0.0]),
            [1.2249402, -0.04205695, -0.019637555],
            2e-6,
        );
    }
    #[test]
    fn adobe_rgb_gamma_is_563_over_256() {
        assert!((RgbColorSpace::AdobeRgb.decode(0.5) - 0.21775553).abs() < 1e-6);
    }
    #[test]
    fn bradford_maps_daylight_whites_and_round_trips() {
        let d65 = [0.3127 / 0.329, 1.0, (1.0 - 0.3127 - 0.329) / 0.329];
        let d50 = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];
        close(adapt_xyz(d65, WhitePoint::D65, WhitePoint::D50), d50, 2e-6);
        close(
            adapt_xyz(
                adapt_xyz([0.2, 0.4, 0.7], WhitePoint::D65, WhitePoint::D50),
                WhitePoint::D50,
                WhitePoint::D65,
            ),
            [0.2, 0.4, 0.7],
            2e-6,
        );
    }
    #[test]
    fn generated_icc_profiles_are_deterministic_and_rgb() {
        for s in [
            RgbColorSpace::Srgb,
            RgbColorSpace::DisplayP3,
            RgbColorSpace::AdobeRgb,
        ] {
            let bytes = s.icc_bytes().unwrap();
            assert_eq!(bytes, s.icc_bytes().unwrap());
            assert_eq!(
                parse_rgb_profile(&bytes).unwrap().color_space,
                DataColorSpace::Rgb
            );
        }
    }
    #[test]
    fn malformed_profiles_fail_without_panics() {
        for n in [0, 12, 128, 132, 2048] {
            let bytes = vec![0xff; n];
            assert!(parse_rgb_profile(&bytes).is_err());
        }
        assert!(parse_rgb_profile(&vec![0; MAX_ICC_BYTES]).is_err());
    }
    #[test]
    fn embedded_profiles_transform_native_16bit_samples_and_keep_alpha() {
        let original =
            image::ImageBuffer::from_pixel(1, 1, image::Rgba([23001_u16, 32013, 42003, 51001]));
        let encoded = [23001.0 / 65535.0, 32013.0 / 65535.0, 42003.0 / 65535.0];
        for space in [
            RgbColorSpace::Srgb,
            RgbColorSpace::DisplayP3,
            RgbColorSpace::AdobeRgb,
        ] {
            let output = import_icc(
                &image::DynamicImage::ImageRgba16(original.clone()),
                &space.icc_bytes().unwrap(),
            )
            .unwrap();
            let p = output.pixels()[0];
            close([p.red, p.green, p.blue], space.to_working(encoded), 3e-4);
            assert_eq!(p.alpha, 51001.0 / 65535.0);
        }
    }
    #[test]
    fn dither_is_bounded_repeatable_and_preserves_black_and_white() {
        let a: Vec<_> = (0..16)
            .map(|i| quantize8(0.5, i % 4, i / 4, true))
            .collect();
        let b: Vec<_> = (0..16)
            .map(|i| quantize8(0.5, i % 4, i / 4, true))
            .collect();
        assert_eq!(a, b);
        assert!(a.contains(&127) && a.contains(&128));
        assert_eq!(quantize8(0.0, 0, 0, true), 0);
        assert_eq!(quantize8(1.0, 0, 0, true), 255);
    }
}
