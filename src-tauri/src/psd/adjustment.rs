//! Photoshop adjustment layers that PhotoForge can hold as adjustment layers of its own.
//!
//! An adjustment layer in a PSD stores its settings in an additional-information block (`curv`,
//! `levl`, `nvrt`, ...) and no pixels. These are decoded here, with every length checked against
//! the block, and turned into the PhotoForge operation that does the same thing. A block that is
//! malformed, or that holds something PhotoForge cannot say, is *refused with a reason* and the
//! importer reports the layer as left out; nothing here panics on a bad block.
//!
//! What maps:
//!
//! * **Invert** (`nvrt`) becomes a curve, exactly.
//! * **Levels** (`levl`) becomes PhotoForge's Levels when only the master (RGB) levels are set,
//!   which is the same arithmetic; otherwise a curve per channel, sampled, because PhotoForge's
//!   Levels is one set of values for all three channels.
//! * **Curves** (`curv`) becomes PhotoForge's curves. Photoshop draws a smooth curve through the
//!   points; PhotoForge joins its points with straight lines, so the smooth curve is sampled at up
//!   to 32 points and the report says so.
//!
//! Two facts here came from Photoshop's own pictures rather than from the documentation, which
//! does not state them: a channel's curve or levels is applied **before** the composite RGB one,
//! and the curve through the points is a **natural cubic spline** (with two other candidates
//! tried, the mean error against the stored picture was 9 and 14 levels against 0.6). See
//! `docs/phase-15-1d-results.md`.
use crate::color::{srgb_decode, srgb_encode};
use crate::domain::{CurvePoint, CurveSet, EditOperation};

/// Largest block this module will read. Real ones are a few hundred bytes.
pub const MAX_BLOCK_BYTES: u64 = 64 * 1024;

/// An adjustment that could be expressed, and what did not carry over exactly.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    pub operation: EditOperation,
    /// A sentence for the report when the result is not exactly Photoshop's.
    pub note: Option<String>,
}

/// Which adjustment keys this module reads.
pub fn handles(key: &[u8; 4]) -> bool {
    matches!(key, b"curv" | b"levl" | b"nvrt" | b"expA")
}

/// Decodes one adjustment block. `Err` carries the reason it could not be imported.
///
/// `grayscale`: the document is greyscale. Its one channel sits in the first channel slot.
pub fn convert(key: &[u8; 4], data: &[u8], grayscale: bool) -> Result<Converted, String> {
    match key {
        b"nvrt" => Ok(Converted {
            operation: EditOperation::Curves {
                curves: CurveSet {
                    rgb: vec![
                        CurvePoint {
                            input: 0.0,
                            output: 1.0,
                        },
                        CurvePoint {
                            input: 1.0,
                            output: 0.0,
                        },
                    ],
                    ..CurveSet::default()
                },
            },
            note: None,
        }),
        b"levl" => levels(data, grayscale),
        b"curv" => curves(data, grayscale),
        b"expA" => exposure(data, grayscale),
        _ => Err("this kind of adjustment layer is not imported".to_string()),
    }
}

// ---- reading ----------------------------------------------------------------------------------

struct Bytes<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Bytes<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    fn u8(&mut self) -> Result<u8, String> {
        let value = *self.data.get(self.at).ok_or_else(truncated)?;
        self.at += 1;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, String> {
        let pair = self.data.get(self.at..self.at + 2).ok_or_else(truncated)?;
        self.at += 2;
        Ok(u16::from_be_bytes([pair[0], pair[1]]))
    }

    fn u32(&mut self) -> Result<u32, String> {
        let word = self.data.get(self.at..self.at + 4).ok_or_else(truncated)?;
        self.at += 4;
        Ok(u32::from_be_bytes([word[0], word[1], word[2], word[3]]))
    }
}

fn truncated() -> String {
    "the adjustment's settings are cut short".to_string()
}

// ---- a smooth curve through points --------------------------------------------------------------

/// A natural cubic spline through control points on the 0–255 scale, clamped to 0–255.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Spline {
    /// `(input, output)`, strictly ascending in input, at least two.
    points: Vec<(f32, f32)>,
    /// The second derivative at each point; zero at both ends.
    second: Vec<f32>,
}

impl Spline {
    pub(crate) fn new(points: Vec<(f32, f32)>) -> Self {
        let n = points.len();
        let mut second = vec![0.0_f32; n];
        if n > 2 {
            // The tridiagonal system for the interior second derivatives (Thomas algorithm).
            let h: Vec<f32> = points
                .windows(2)
                .map(|pair| pair[1].0 - pair[0].0)
                .collect();
            let delta: Vec<f32> = points
                .windows(2)
                .zip(&h)
                .map(|(pair, h)| (pair[1].1 - pair[0].1) / h)
                .collect();
            let mut diagonal = vec![0.0_f32; n];
            let mut right = vec![0.0_f32; n];
            for i in 1..n - 1 {
                diagonal[i] = 2.0 * (h[i - 1] + h[i]);
                right[i] = 6.0 * (delta[i] - delta[i - 1]);
            }
            for i in 2..n - 1 {
                let w = h[i - 1] / diagonal[i - 1];
                diagonal[i] -= w * h[i - 1];
                right[i] -= w * right[i - 1];
            }
            second[n - 2] = right[n - 2] / diagonal[n - 2];
            for i in (1..n - 2).rev() {
                second[i] = (right[i] - h[i] * second[i + 1]) / diagonal[i];
            }
        }
        Self { points, second }
    }

    pub(crate) fn at(&self, x: f32) -> f32 {
        let n = self.points.len();
        if x <= self.points[0].0 {
            return self.points[0].1.clamp(0.0, 255.0);
        }
        if x >= self.points[n - 1].0 {
            return self.points[n - 1].1.clamp(0.0, 255.0);
        }
        let i = self
            .points
            .windows(2)
            .position(|pair| x <= pair[1].0)
            .unwrap_or(n - 2);
        let (x0, y0) = self.points[i];
        let (x1, y1) = self.points[i + 1];
        let h = x1 - x0;
        let a = (x1 - x) / h;
        let b = (x - x0) / h;
        let value = a * y0
            + b * y1
            + ((a * a * a - a) * self.second[i] + (b * b * b - b) * self.second[i + 1]) * h * h
                / 6.0;
        value.clamp(0.0, 255.0)
    }
}

/// `function` sampled as PhotoForge's straight-line point list: the control inputs given, and
/// enough evenly spaced ones to keep a smooth curve's shape, 32 at most, with 0 and 255 always
/// present.
fn sample(function: impl Fn(f32) -> f32, controls: &[f32]) -> Vec<CurvePoint> {
    let mut inputs: Vec<f32> = vec![0.0, 255.0];
    inputs.extend(
        controls
            .iter()
            .copied()
            .filter(|v| (0.0..=255.0).contains(v)),
    );
    let tidy = |inputs: &mut Vec<f32>| {
        inputs.sort_by(f32::total_cmp);
        inputs.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    };
    tidy(&mut inputs);
    let mut grid = 24_usize;
    while grid > 0 && inputs.len() + grid > 32 {
        grid -= 1;
    }
    for step in 1..=grid {
        inputs.push(255.0 * step as f32 / (grid + 1) as f32);
    }
    tidy(&mut inputs);
    inputs.truncate(32);
    // Truncation keeps the lowest inputs; the top of the scale must still be there.
    if let Some(last) = inputs.last_mut() {
        *last = 255.0;
    }
    inputs
        .into_iter()
        .map(|input| CurvePoint {
            input: input / 255.0,
            output: (function(input) / 255.0).clamp(0.0, 1.0),
        })
        .collect()
}

fn identity() -> Vec<CurvePoint> {
    vec![
        CurvePoint {
            input: 0.0,
            output: 0.0,
        },
        CurvePoint {
            input: 1.0,
            output: 1.0,
        },
    ]
}

// ---- Curves -----------------------------------------------------------------------------------

fn curves(data: &[u8], grayscale: bool) -> Result<Converted, String> {
    let mut bytes = Bytes::new(data);
    let _padding = bytes.u8()?;
    let version = bytes.u16()?;
    if version != 1 && version != 4 {
        return Err(format!("curves settings of version {version} are not read"));
    }
    let mask = bytes.u32()?;
    // Bit 0 is the composite curve, bits 1 to 3 red, green and blue. Higher bits are channels of
    // other colour modes, which an RGB document does not have.
    if mask & !0b1111 != 0 {
        return Err("the curves adjust channels this document does not have".to_string());
    }
    let mut set: [Option<Vec<(f32, f32)>>; 4] = [None, None, None, None];
    for (index, slot) in set.iter_mut().enumerate() {
        if mask & (1 << index) == 0 {
            continue;
        }
        let count = usize::from(bytes.u16()?);
        if !(2..=19).contains(&count) {
            return Err(format!("a curve with {count} points is not read"));
        }
        let mut points = Vec::with_capacity(count);
        for _ in 0..count {
            // The output value comes first.
            let output = f32::from(bytes.u16()?);
            let input = f32::from(bytes.u16()?);
            if input > 255.0 || output > 255.0 {
                return Err("a curve point lies outside 0 to 255".to_string());
            }
            points.push((input, output));
        }
        if points.windows(2).any(|pair| pair[0].0 >= pair[1].0) {
            return Err("a curve's points are not in increasing order".to_string());
        }
        *slot = Some(points);
    }
    // A greyscale document's one channel is the first channel slot, and stands for all three of
    // the RGB channels PhotoForge keeps it as.
    if grayscale {
        set[2] = set[1].clone();
        set[3] = set[1].clone();
    }
    let splines: Vec<Option<Spline>> = set
        .iter()
        .map(|points| points.clone().map(Spline::new))
        .collect();
    let controls = |points: &Option<Vec<(f32, f32)>>| -> Vec<f32> {
        points
            .as_ref()
            .map(|points| points.iter().map(|(input, _)| *input).collect())
            .unwrap_or_default()
    };
    let from_points = |index: usize| -> Vec<CurvePoint> {
        match (&splines[index], &set[index]) {
            // Two points are a straight line, flat beyond them: four exact points, no sampling.
            (Some(spline), Some(points)) if points.len() == 2 => {
                let mut inputs = vec![0.0, points[0].0, points[1].0, 255.0];
                inputs.dedup_by(|a, b| (*a - *b).abs() < 0.5);
                inputs
                    .into_iter()
                    .map(|input| CurvePoint {
                        input: input / 255.0,
                        output: spline.at(input) / 255.0,
                    })
                    .collect()
            }
            (Some(spline), _) => sample(|x| spline.at(x), &controls(&set[index])),
            _ => identity(),
        }
    };
    let composite = &splines[0];
    let any_channel = splines[1..].iter().any(Option::is_some);
    let composed = composite.is_some() && any_channel;
    let operation = if composed {
        // Photoshop applies a channel's curve and then the composite one; PhotoForge applies the
        // composite first. Each channel's pair is composed into one curve for it.
        let channel_curve = |index: usize| -> Vec<CurvePoint> {
            let own = &splines[index];
            let mut points = controls(&set[index]);
            points.extend(controls(&set[0]));
            let function = |x: f32| {
                let through_channel = own.as_ref().map_or(x, |own| own.at(x));
                composite
                    .as_ref()
                    .map_or(through_channel, |c| c.at(through_channel))
            };
            sample(function, &points)
        };
        EditOperation::Curves {
            curves: CurveSet {
                rgb: identity(),
                red: channel_curve(1),
                green: channel_curve(2),
                blue: channel_curve(3),
            },
        }
    } else {
        EditOperation::Curves {
            curves: CurveSet {
                rgb: from_points(0),
                red: from_points(1),
                green: from_points(2),
                blue: from_points(3),
            },
        }
    };
    // A curve through two points is a straight line, which PhotoForge's are: exact. Anything
    // with more points is a smooth curve and is sampled.
    let straight = set
        .iter()
        .all(|points| points.as_ref().is_none_or(|points| points.len() == 2));
    Ok(Converted {
        operation,
        note: (!straight || composed).then(|| {
            "Curves were imported by sampling Photoshop's smooth curve at up to 32 points and joining them with straight lines; the result is within a level or two of Photoshop's.".to_string()
        }),
    })
}

// ---- Exposure ---------------------------------------------------------------------------------

/// Exposure, offset and gamma correction. Photoshop computes them on linear light even in an
/// 8-bit sRGB document, so the mapping is evaluated that way and handed on as a curve.
fn exposure(data: &[u8], grayscale: bool) -> Result<Converted, String> {
    let mut bytes = Bytes::new(data);
    let version = bytes.u16()?;
    if version != 1 {
        return Err(format!(
            "exposure settings of version {version} are not read"
        ));
    }
    let stops = f32::from_bits(bytes.u32()?);
    let offset = f32::from_bits(bytes.u32()?);
    let gamma = f32::from_bits(bytes.u32()?);
    if !(stops.is_finite() && offset.is_finite() && gamma.is_finite())
        || stops.abs() > 20.0
        || offset.abs() > 1.0
        || !(0.01..=10.0).contains(&gamma)
    {
        return Err("an exposure value lies outside its range".to_string());
    }
    // In a greyscale document Photoshop's picture matched neither sRGB nor a 2.2 power nor the
    // encoded values (mean error 11, 11 and 30 levels), so it is left out rather than guessed.
    if grayscale {
        return Err(
            "exposure in a greyscale document is not imported: its transfer curve is not known well enough".to_string(),
        );
    }
    let scale = stops.exp2();
    let function = |x: f32| {
        let linear = srgb_decode((x / 255.0).clamp(0.0, 1.0)) * scale + offset;
        let corrected = if linear >= 0.0 {
            linear.powf(1.0 / gamma)
        } else {
            -(-linear).powf(1.0 / gamma)
        };
        srgb_encode(corrected.clamp(0.0, 1.0)) * 255.0
    };
    // Steep near black in the encoded scale, so the dark end gets the points.
    let controls = [2.0, 4.0, 6.0, 9.0, 13.0, 18.0, 25.0, 34.0, 48.0, 64.0, 96.0];
    Ok(Converted {
        operation: EditOperation::Curves {
            curves: CurveSet {
                rgb: sample(function, &controls),
                ..CurveSet::default()
            },
        },
        note: Some(
            "Exposure was imported as a curve sampled at up to 32 points of what Photoshop computes in linear light; the result is within a level or two of Photoshop's."
                .to_string(),
        ),
    })
}

// ---- Levels -----------------------------------------------------------------------------------

/// One levels record: input floor and ceiling, output floor and ceiling, and the midtone.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LevelsRecord {
    input_floor: f32,
    input_ceiling: f32,
    output_floor: f32,
    output_ceiling: f32,
    gamma: f32,
}

impl LevelsRecord {
    const NEUTRAL: Self = Self {
        input_floor: 0.0,
        input_ceiling: 255.0,
        output_floor: 0.0,
        output_ceiling: 255.0,
        gamma: 1.0,
    };

    fn is_default(&self) -> bool {
        *self == Self::NEUTRAL
    }

    fn apply(&self, value: f32) -> f32 {
        let span = self.input_ceiling - self.input_floor;
        let normal = if span <= 0.0 {
            f32::from(value >= self.input_ceiling)
        } else {
            ((value - self.input_floor) / span).clamp(0.0, 1.0)
        };
        self.output_floor
            + (self.output_ceiling - self.output_floor) * normal.powf(1.0 / self.gamma)
    }
}

fn levels(data: &[u8], grayscale: bool) -> Result<Converted, String> {
    let mut bytes = Bytes::new(data);
    let version = bytes.u16()?;
    if version != 2 {
        return Err(format!("levels settings of version {version} are not read"));
    }
    // Twenty-nine records follow the version with no count of their own: the composite, red,
    // green and blue, then slots for the channels of other colour modes.
    let mut records = Vec::with_capacity(4);
    for index in 0..29 {
        let record = LevelsRecord {
            input_floor: f32::from(bytes.u16()?),
            input_ceiling: f32::from(bytes.u16()?),
            output_floor: f32::from(bytes.u16()?),
            output_ceiling: f32::from(bytes.u16()?),
            gamma: f32::from(bytes.u16()?) / 100.0,
        };
        if index < 4 {
            if record.gamma <= 0.0
                || record.input_floor > 255.0
                || record.input_ceiling > 255.0
                || record.output_floor > 255.0
                || record.output_ceiling > 255.0
            {
                return Err("a levels value lies outside its range".to_string());
            }
            records.push(record);
        }
    }
    let master = records[0];
    // A greyscale document's one channel is the first channel slot.
    let channels = if grayscale {
        [records[1]; 3]
    } else {
        [records[1], records[2], records[3]]
    };
    // Master only, and values PhotoForge's own Levels can hold: the same arithmetic.
    if channels.iter().all(LevelsRecord::is_default)
        && master.input_floor < master.input_ceiling
        && master.output_floor <= master.output_ceiling
        && (0.1..=10.0).contains(&master.gamma)
    {
        return Ok(Converted {
            operation: EditOperation::Levels {
                input_black: master.input_floor as u8,
                input_white: master.input_ceiling as u8,
                gamma: master.gamma,
                output_black: master.output_floor as u8,
                output_white: master.output_ceiling as u8,
            },
            note: None,
        });
    }
    // Otherwise a curve per channel: the channel's own levels, then the master's.
    let curve_for = |channel: LevelsRecord| -> Vec<CurvePoint> {
        let controls = [
            master.input_floor,
            master.input_ceiling,
            channel.input_floor,
            channel.input_ceiling,
            master.output_floor,
            master.output_ceiling,
        ];
        sample(|x| master.apply(channel.apply(x)), &controls)
    };
    Ok(Converted {
        operation: EditOperation::Curves {
            curves: CurveSet {
                rgb: identity(),
                red: curve_for(channels[0]),
                green: curve_for(channels[1]),
                blue: curve_for(channels[2]),
            },
        },
        note: Some(
            "Levels with settings for individual channels, or a master PhotoForge's Levels cannot hold, were imported as curves sampled at up to 32 points; the result is within a level or two of Photoshop's."
                .to_string(),
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn be16(out: &mut Vec<u8>, value: u16) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    /// A `curv` block with the given curves (composite, red, green, blue), each a list of
    /// `(input, output)`.
    fn curv(curves: [Option<&[(u16, u16)]>; 4]) -> Vec<u8> {
        let mut out = vec![0, 0, 1];
        let mask: u32 = curves
            .iter()
            .enumerate()
            .map(|(i, c)| u32::from(c.is_some()) << i)
            .sum();
        out.extend_from_slice(&mask.to_be_bytes());
        for curve in curves.into_iter().flatten() {
            be16(&mut out, curve.len() as u16);
            for (input, output) in curve {
                be16(&mut out, *output);
                be16(&mut out, *input);
            }
        }
        out
    }

    fn levl(records: &[[u16; 5]]) -> Vec<u8> {
        let mut out = Vec::new();
        be16(&mut out, 2);
        for index in 0..29 {
            let record = records.get(index).copied().unwrap_or([0, 255, 0, 255, 100]);
            for value in record {
                be16(&mut out, value);
            }
        }
        out
    }

    fn curves_of(converted: &Converted) -> &CurveSet {
        match &converted.operation {
            EditOperation::Curves { curves } => curves,
            other => panic!("expected curves, got {other:?}"),
        }
    }

    fn at(points: &[CurvePoint], input: f32) -> f32 {
        let pair = points
            .windows(2)
            .find(|pair| input <= pair[1].input)
            .unwrap_or(&points[points.len() - 2..]);
        let t = (input - pair[0].input) / (pair[1].input - pair[0].input);
        pair[0].output + (pair[1].output - pair[0].output) * t
    }

    #[test]
    fn a_real_curves_block_is_read_with_output_first() {
        // "Curves 2" of psd-tools' adjustments/curves_rgb.psd: composite and blue, with its
        // trailing extended data cut off.
        // The block is 7 bytes of header, then the composite curve of four points (output,
        // input): (3, 26), (84, 55), (170, 171), (255, 255); then the blue one of three:
        // (0, 0), (110, 167), (166, 255).
        let bytes: Vec<u8> = vec![
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x09, 0x00, 0x04, 0x00, 0x03, 0x00, 0x1a, 0x00,
            0x54, 0x00, 0x37, 0x00, 0xaa, 0x00, 0xab, 0x00, 0xff, 0x00, 0xff, 0x00, 0x03, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x6e, 0x00, 0xa7, 0x00, 0xa6, 0x00, 0xff,
        ];
        let converted = convert(b"curv", &bytes, false).unwrap();
        let curves = curves_of(&converted);
        // The composite curve and a channel's own are composed per channel, so red, which has no
        // curve of its own, shows the composite: input 26 maps to output 3, input 171 to 170.
        assert!((at(&curves.red, 26.0 / 255.0) - 3.0 / 255.0).abs() < 0.01);
        assert!((at(&curves.red, 171.0 / 255.0) - 170.0 / 255.0).abs() < 0.01);
        // Blue goes through its own curve first: 167 -> 110, then the composite takes 110
        // (between its points (55, 84) and (171, 170)) to about 150.
        let blue = at(&curves.blue, 167.0 / 255.0) * 255.0;
        assert!((140.0..160.0).contains(&blue), "{blue}");
        // A smooth curve was sampled, and the result says so.
        assert!(converted.note.is_some());
        assert!(curves.red.len() > 4 && curves.red.len() <= 32);
    }

    #[test]
    fn a_curve_through_two_points_is_exact() {
        let bytes = curv([Some(&[(0, 0), (255, 255)]), None, None, None]);
        let converted = convert(b"curv", &bytes, false).unwrap();
        assert!(converted.note.is_none());
        assert_eq!(curves_of(&converted).rgb.len(), 2);
        // Lowering only the top is a straight line from the origin.
        let bytes = curv([Some(&[(0, 0), (255, 128)]), None, None, None]);
        let converted = convert(b"curv", &bytes, false).unwrap();
        assert!(converted.note.is_none());
        let rgb = &curves_of(&converted).rgb;
        assert!((at(rgb, 0.5) - 0.25).abs() < 0.01);
        // A curve through two inner points is flat beyond them, said exactly in four points.
        let bytes = curv([Some(&[(64, 64), (192, 192)]), None, None, None]);
        let converted = convert(b"curv", &bytes, false).unwrap();
        assert!(converted.note.is_none());
        let rgb = &curves_of(&converted).rgb;
        assert_eq!(rgb.len(), 4);
        assert!((rgb[0].output - 64.0 / 255.0).abs() < 1e-4);
        assert!((rgb[3].output - 192.0 / 255.0).abs() < 1e-4);
    }

    #[test]
    fn a_channel_curve_is_applied_before_the_composite_one() {
        // Composite: halve. Red: lift the floor to 64. Channel first, input 0.4 becomes about
        // 0.65 and then 0.33; composite first it would be 0.2 and then 0.4.
        let composite: &[(u16, u16)] = &[(0, 0), (255, 128)];
        let red: &[(u16, u16)] = &[(0, 64), (255, 255)];
        let bytes = curv([Some(composite), Some(red), None, None]);
        let converted = convert(b"curv", &bytes, false).unwrap();
        let curves = curves_of(&converted);
        let x = 0.4;
        let channel_first = ((64.0 + (255.0 - 64.0) * x) / 255.0) * 128.0 / 255.0;
        assert!(
            (at(&curves.red, x) - channel_first).abs() < 0.01,
            "{}",
            at(&curves.red, x)
        );
        // The green channel has no curve of its own and sees only the composite.
        assert!((at(&curves.green, x) - x * 128.0 / 255.0).abs() < 0.01);
        assert_eq!(curves.rgb, identity());
    }

    #[test]
    fn exposure_is_computed_in_linear_light_and_handed_on_as_a_curve() {
        // Exposure +1 stop, no offset, gamma 1: 2^1 in linear light. Mid grey (sRGB 188) is
        // 0.5 linear, so it doubles to 1.0: white.
        let mut bytes = vec![0, 1];
        for value in [1.0_f32, 0.0, 1.0] {
            bytes.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        bytes.extend_from_slice(&[0, 0]);
        let converted = convert(b"expA", &bytes, false).unwrap();
        let rgb = &curves_of(&converted).rgb;
        assert!(rgb.len() <= 32);
        assert!(at(rgb, 188.0 / 255.0) > 0.98);
        // Black stays black with no offset, and a quarter of linear light (sRGB 137) goes to half
        // (sRGB 188).
        assert!(at(rgb, 0.0) < 0.01);
        assert!((at(rgb, 137.0 / 255.0) - 188.0 / 255.0).abs() < 0.03);
        assert!(converted.note.is_some());
        // Out-of-range settings and a greyscale document are refused with a reason.
        let mut absurd = vec![0, 1];
        for value in [100.0_f32, 0.0, 1.0] {
            absurd.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        assert!(convert(b"expA", &absurd, false).is_err());
        assert!(convert(b"expA", &bytes, true).is_err());
    }

    #[test]
    fn invert_is_exact() {
        let converted = convert(b"nvrt", &[], false).unwrap();
        assert!(converted.note.is_none());
        let rgb = &curves_of(&converted).rgb;
        assert_eq!((rgb[0].output, rgb[1].output), (1.0, 0.0));
    }

    #[test]
    fn master_levels_become_photoforges_levels() {
        let bytes = levl(&[[48, 135, 74, 141, 150]]);
        let converted = convert(b"levl", &bytes, false).unwrap();
        assert!(converted.note.is_none());
        assert_eq!(
            converted.operation,
            EditOperation::Levels {
                input_black: 48,
                input_white: 135,
                gamma: 1.5,
                output_black: 74,
                output_white: 141,
            }
        );
    }

    #[test]
    fn channel_levels_become_curves_with_the_channel_first() {
        // Master: input 64..192 stretched to 0..255. Red: output ceiling 128.
        let bytes = levl(&[[64, 192, 0, 255, 100], [0, 255, 0, 128, 100]]);
        let converted = convert(b"levl", &bytes, false).unwrap();
        assert!(converted.note.is_some());
        let curves = curves_of(&converted);
        // Red at 0.5 (127.5): the channel halves it to 64, which the master puts at its floor, 0.
        // Master first it would be 126.5 and then 63.5, a quarter. The two orders differ here.
        assert!(at(&curves.red, 0.5) < 0.02, "{}", at(&curves.red, 0.5));
        // Green sees only the master: 127.5 is just under halfway between 64 and 192.
        assert!((at(&curves.green, 0.5) - 0.496).abs() < 0.02);
    }

    #[test]
    fn a_greyscale_documents_one_channel_is_the_composite() {
        let bytes = levl(&[[0, 255, 0, 255, 100], [39, 217, 25, 255, 152]]);
        let converted = convert(b"levl", &bytes, true).unwrap();
        // The channel's own levels, applied to all three of PhotoForge's channels.
        let curves = curves_of(&converted);
        assert_eq!(curves.red, curves.green);
        assert_eq!(curves.green, curves.blue);
        assert!(at(&curves.red, 0.5) > 0.0);
        let bytes = curv([None, Some(&[(0, 0), (100, 200), (255, 255)]), None, None]);
        let converted = convert(b"curv", &bytes, true).unwrap();
        let curves = curves_of(&converted);
        assert_eq!(curves.red, curves.blue);
        assert!(at(&curves.green, 100.0 / 255.0) > 0.7);
    }

    #[test]
    fn malformed_blocks_are_refused_not_panicked_on() {
        let good = curv([Some(&[(0, 0), (100, 50), (255, 255)]), None, None, None]);
        for cut in 0..good.len() {
            let _ = convert(b"curv", &good[..cut], false);
        }
        let good = levl(&[[0, 255, 0, 255, 100]]);
        for cut in 0..good.len() {
            let _ = convert(b"levl", &good[..cut], false);
        }
        // Points that are out of order, a count that is absurd, an unknown version, a channel an
        // RGB document does not have.
        let reversed = curv([Some(&[(100, 0), (50, 255)]), None, None, None]);
        assert!(convert(b"curv", &reversed, false).is_err());
        let mut absurd = curv([Some(&[(0, 0), (255, 255)]), None, None, None]);
        absurd[7] = 0xff;
        absurd[8] = 0xff;
        assert!(convert(b"curv", &absurd, false).is_err());
        let mut version = good.clone();
        version[1] = 9;
        assert!(convert(b"levl", &version, false).is_err());
        let mut mask = curv([Some(&[(0, 0), (255, 255)]), None, None, None]);
        mask[6] |= 0b10000;
        assert!(convert(b"curv", &mask, false).is_err());
    }

    #[test]
    fn a_smooth_curve_stays_in_range_and_in_32_points() {
        let points: &[(u16, u16)] = &[(0, 255), (40, 0), (90, 255), (140, 0), (200, 255), (255, 0)];
        let converted = convert(b"curv", &curv([Some(points), None, None, None]), false).unwrap();
        let rgb = &curves_of(&converted).rgb;
        assert!(rgb.len() <= 32);
        assert_eq!(rgb[0].input, 0.0);
        assert_eq!(rgb.last().unwrap().input, 1.0);
        assert!(rgb.iter().all(|p| (0.0..=1.0).contains(&p.output)));
        assert!(rgb.windows(2).all(|pair| pair[0].input < pair[1].input));
    }
}
