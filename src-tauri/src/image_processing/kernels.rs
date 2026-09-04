//! Point-spread functions, and the convolution that applies them.
//!
//! These live in production code rather than in the test fixtures on purpose.
//! A deconvolution can only be judged against the kernel that actually blurred
//! the image, so the fixtures build their blur from these same functions. If
//! the two ever diverged, every deconvolution test would be measuring the wrong
//! thing and still passing.
use crate::color::{FloatImage, FloatRgba};
use crate::error::AppError;

/// A blur that a deconvolution can try to reverse.
///
/// Deliberately a closed set of shapes with named parameters. Blind estimation
/// of an unknown kernel is a different and much harder problem, and nothing
/// here attempts it or claims to.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum BlurKernel {
    /// Out-of-focus blur: a disc with a one-pixel soft edge.
    Defocus { radius: f32 },
    /// Linear camera or subject movement.
    Motion { angle_degrees: f32, distance: f32 },
    /// General softness, and the shape most lens blur is approximated by.
    Gaussian { sigma: f32 },
}

/// Largest kernel reach accepted, in pixels either side.
///
/// A kernel is applied twice per deconvolution iteration, so an unbounded reach
/// would be an unbounded amount of work and an unbounded tile halo.
pub const MAX_KERNEL_REACH: f32 = 16.0;

impl BlurKernel {
    pub fn validate(&self) -> bool {
        match self {
            Self::Defocus { radius } => {
                radius.is_finite() && (0.5..=MAX_KERNEL_REACH).contains(radius)
            }
            Self::Motion {
                angle_degrees,
                distance,
            } => {
                angle_degrees.is_finite()
                    && (-360.0..=360.0).contains(angle_degrees)
                    && distance.is_finite()
                    && (1.0..=(MAX_KERNEL_REACH * 2.0)).contains(distance)
            }
            Self::Gaussian { sigma } => {
                sigma.is_finite() && (0.2..=(MAX_KERNEL_REACH / 3.0)).contains(sigma)
            }
        }
    }

    /// How far the kernel reaches from its centre, in pixels.
    pub fn reach(&self) -> u32 {
        match self {
            Self::Defocus { radius } => radius.max(0.0).ceil() as u32 + 1,
            Self::Motion { distance, .. } => (distance.max(0.0) / 2.0).ceil() as u32,
            Self::Gaussian { sigma } => (sigma.max(0.0) * 3.0).ceil() as u32,
        }
    }

    /// The normalised taps, as `(dx, dy, weight)`.
    pub fn taps(&self) -> Vec<(i32, i32, f32)> {
        let mut taps = match self {
            Self::Defocus { radius } => defocus_taps(*radius),
            Self::Motion {
                angle_degrees,
                distance,
            } => motion_taps(*angle_degrees, *distance),
            Self::Gaussian { sigma } => gaussian_taps(*sigma),
        };
        let total: f32 = taps.iter().map(|(_, _, w)| *w).sum();
        if total > 0.0 {
            for tap in &mut taps {
                tap.2 /= total;
            }
        }
        taps
    }
}

fn defocus_taps(radius: f32) -> Vec<(i32, i32, f32)> {
    let reach = radius.clamp(0.0, MAX_KERNEL_REACH).ceil().max(1.0) as i32;
    let mut taps = Vec::new();
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let distance = ((dx * dx + dy * dy) as f32).sqrt();
            // A disc with a one-pixel soft edge: what a point actually spreads
            // into when the lens is out of focus, rather than a Gaussian.
            let weight = if distance <= radius {
                1.0
            } else if distance <= radius + 1.0 {
                radius + 1.0 - distance
            } else {
                0.0
            };
            if weight > 0.0 {
                taps.push((dx, dy, weight));
            }
        }
    }
    taps
}

fn motion_taps(angle_degrees: f32, distance: f32) -> Vec<(i32, i32, f32)> {
    let steps = distance.clamp(0.0, MAX_KERNEL_REACH * 2.0).round().max(1.0) as i32;
    let radians = angle_degrees.to_radians();
    let (dx, dy) = (radians.cos(), radians.sin());
    let mut taps: Vec<(i32, i32, f32)> = Vec::new();
    for step in -(steps / 2)..=(steps / 2) {
        let x = (dx * step as f32).round() as i32;
        let y = (dy * step as f32).round() as i32;
        match taps.iter_mut().find(|(tx, ty, _)| *tx == x && *ty == y) {
            Some((_, _, weight)) => *weight += 1.0,
            None => taps.push((x, y, 1.0)),
        }
    }
    taps
}

fn gaussian_taps(sigma: f32) -> Vec<(i32, i32, f32)> {
    let sigma = sigma.clamp(0.01, MAX_KERNEL_REACH);
    let reach = (sigma * 3.0).ceil().max(1.0) as i32;
    let mut taps = Vec::new();
    for dy in -reach..=reach {
        for dx in -reach..=reach {
            let weight = (-((dx * dx + dy * dy) as f32) / (2.0 * sigma * sigma)).exp();
            if weight > 1e-6 {
                taps.push((dx, dy, weight));
            }
        }
    }
    taps
}

/// Convolves RGB with normalised `taps`, clamping at the border.
///
/// Alpha is carried through untouched: these kernels model optics, and optics
/// do not change how opaque a pixel is.
pub fn convolve(image: &FloatImage, taps: &[(i32, i32, f32)]) -> Result<FloatImage, AppError> {
    let (width, height) = image.dimensions();
    let mut out = image.clone();
    let pixels = image.pixels();
    let stride = width as usize;
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let mut sum = [0.0f32; 3];
            for (dx, dy, weight) in taps {
                let sx = (x + dx).clamp(0, width as i32 - 1) as usize;
                let sy = (y + dy).clamp(0, height as i32 - 1) as usize;
                let p = pixels[sy * stride + sx];
                sum[0] += p.red * weight;
                sum[1] += p.green * weight;
                sum[2] += p.blue * weight;
            }
            let index = y as usize * stride + x as usize;
            out.pixels_mut()[index] = FloatRgba::new(sum[0], sum[1], sum[2], pixels[index].alpha);
        }
    }
    Ok(out)
}

/// The transpose (mirrored) kernel, which is what the Richardson-Lucy update
/// needs for its back-projection step.
pub fn transpose(taps: &[(i32, i32, f32)]) -> Vec<(i32, i32, f32)> {
    taps.iter().map(|(dx, dy, w)| (-dx, -dy, *w)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kernel_is_normalised_and_bounded() {
        for kernel in [
            BlurKernel::Defocus { radius: 3.0 },
            BlurKernel::Motion {
                angle_degrees: 30.0,
                distance: 9.0,
            },
            BlurKernel::Gaussian { sigma: 2.0 },
        ] {
            assert!(kernel.validate(), "{kernel:?} failed its own validation");
            let taps = kernel.taps();
            assert!(!taps.is_empty());
            let total: f32 = taps.iter().map(|(_, _, w)| *w).sum();
            assert!(
                (total - 1.0).abs() < 1e-4,
                "{kernel:?} sums to {total}, so it would change the image's brightness"
            );
            let reach = kernel.reach() as i32;
            assert!(
                taps.iter()
                    .all(|(dx, dy, _)| dx.abs() <= reach && dy.abs() <= reach),
                "{kernel:?} has taps beyond its declared reach of {reach}"
            );
        }
    }

    /// Reach is what the tiled renderer grows a tile by, so it must never
    /// understate the kernel.
    #[test]
    fn reach_never_understates_the_taps() {
        for radius in [0.5f32, 1.0, 2.5, 4.0, 8.0, 16.0] {
            let kernel = BlurKernel::Defocus { radius };
            let reach = kernel.reach() as i32;
            for (dx, dy, _) in kernel.taps() {
                assert!(dx.abs() <= reach && dy.abs() <= reach, "radius {radius}");
            }
        }
        for distance in [1.0f32, 5.0, 12.0, 32.0] {
            let kernel = BlurKernel::Motion {
                angle_degrees: 45.0,
                distance,
            };
            let reach = kernel.reach() as i32;
            for (dx, dy, _) in kernel.taps() {
                assert!(
                    dx.abs() <= reach && dy.abs() <= reach,
                    "distance {distance}"
                );
            }
        }
    }

    #[test]
    fn hostile_kernel_parameters_are_refused() {
        for kernel in [
            BlurKernel::Defocus { radius: f32::NAN },
            BlurKernel::Defocus { radius: 0.0 },
            BlurKernel::Defocus { radius: 1e9 },
            BlurKernel::Gaussian {
                sigma: f32::INFINITY,
            },
            BlurKernel::Gaussian { sigma: -1.0 },
            BlurKernel::Motion {
                angle_degrees: f32::NAN,
                distance: 5.0,
            },
            BlurKernel::Motion {
                angle_degrees: 0.0,
                distance: 1e6,
            },
        ] {
            assert!(!kernel.validate(), "{kernel:?} was accepted");
        }
    }

    /// Convolving with a mirrored kernel twice must be symmetric, and a
    /// symmetric kernel must be its own transpose.
    #[test]
    fn transpose_mirrors_the_kernel() {
        let taps = BlurKernel::Gaussian { sigma: 1.5 }.taps();
        let mirrored = transpose(&taps);
        for (dx, dy, w) in &taps {
            let found = mirrored
                .iter()
                .find(|(mx, my, _)| *mx == -dx && *my == -dy)
                .expect("every tap has a mirror");
            assert!((found.2 - w).abs() < 1e-9);
        }
    }

    /// A convolution must not change overall brightness, or a deconvolution
    /// built on it would drift.
    #[test]
    fn convolution_preserves_average_brightness() {
        let image =
            FloatImage::blank(48, 48, FloatRgba::new(0.4, 0.5, 0.6, 1.0)).expect("blank image");
        let blurred = convolve(&image, &BlurKernel::Defocus { radius: 2.0 }.taps()).unwrap();
        for pixel in blurred.pixels() {
            assert!((pixel.red - 0.4).abs() < 1e-5);
            assert!((pixel.green - 0.5).abs() < 1e-5);
            assert!((pixel.blue - 0.6).abs() < 1e-5);
            assert_eq!(pixel.alpha, 1.0);
        }
    }
}
