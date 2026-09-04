//! Objective image-quality metrics, and the synthetic degradations they judge.
//!
//! Restoration is the one area where "it looks better" is the easiest claim to
//! make and the hardest to justify. Everything here exists so that a change to
//! a restoration algorithm is accepted or rejected by a number measured against
//! a *known clean* image, not by an assertion.
//!
//! The degradations start from a clean fixture, so the original is available as
//! ground truth. That is what makes error, edge retention and artifact
//! reduction measurable at all. It is also the limit of the method: a synthetic
//! Gaussian noise field is not a real sensor, and a block-quantised fixture is
//! not a real JPEG. These metrics say an algorithm behaves correctly on a
//! degradation it was given; they do not say it will delight a photographer.
use crate::color::{FloatImage, FloatRgba};

/// Mean squared error over RGB, weighted by neither alpha nor luminance.
///
/// Alpha is excluded because the restoration operations here do not change it,
/// and including a constant channel would flatter every score equally.
pub fn mse(a: &FloatImage, b: &FloatImage) -> f64 {
    assert_eq!(a.dimensions(), b.dimensions(), "metrics need equal sizes");
    let mut total = 0.0f64;
    for (p, q) in a.pixels().iter().zip(b.pixels()) {
        for (x, y) in [(p.red, q.red), (p.green, q.green), (p.blue, q.blue)] {
            let d = f64::from(x - y);
            total += d * d;
        }
    }
    total / (a.pixels().len() as f64 * 3.0)
}

/// Peak signal-to-noise ratio in decibels against a peak of 1.0.
///
/// Returns `f64::INFINITY` for identical images. PSNR is reported because it is
/// standard and comparable, not because it is a good judge of appearance: it
/// rewards blurring away detail, so it is never the only metric used here.
pub fn psnr(a: &FloatImage, b: &FloatImage) -> f64 {
    let error = mse(a, b);
    if error <= 0.0 {
        return f64::INFINITY;
    }
    10.0 * (1.0 / error).log10()
}

/// Structural similarity on the luminance channel, over 8x8 windows.
///
/// This is the standard formulation: local means, variances and covariance with
/// the usual C1/C2 stabilisers, averaged over windows. It is computed on
/// luminance rather than per channel because SSIM is a structural measure and
/// running it three times on correlated channels would triple-count structure.
pub fn ssim(a: &FloatImage, b: &FloatImage) -> f64 {
    assert_eq!(a.dimensions(), b.dimensions(), "metrics need equal sizes");
    const WINDOW: u32 = 8;
    const C1: f64 = 0.01 * 0.01;
    const C2: f64 = 0.03 * 0.03;
    let (width, height) = a.dimensions();
    if width < WINDOW || height < WINDOW {
        return f64::NAN;
    }
    let luma = |image: &FloatImage, x: u32, y: u32| -> f64 {
        f64::from(image.get(x, y).map_or(0.0, FloatRgba::luminance))
    };
    let mut total = 0.0f64;
    let mut windows = 0u64;
    for wy in (0..=height - WINDOW).step_by(WINDOW as usize) {
        for wx in (0..=width - WINDOW).step_by(WINDOW as usize) {
            let (mut sum_a, mut sum_b) = (0.0f64, 0.0f64);
            for y in wy..wy + WINDOW {
                for x in wx..wx + WINDOW {
                    sum_a += luma(a, x, y);
                    sum_b += luma(b, x, y);
                }
            }
            let n = f64::from(WINDOW * WINDOW);
            let (mean_a, mean_b) = (sum_a / n, sum_b / n);
            let (mut var_a, mut var_b, mut covariance) = (0.0f64, 0.0f64, 0.0f64);
            for y in wy..wy + WINDOW {
                for x in wx..wx + WINDOW {
                    let (da, db) = (luma(a, x, y) - mean_a, luma(b, x, y) - mean_b);
                    var_a += da * da;
                    var_b += db * db;
                    covariance += da * db;
                }
            }
            // Sample variance, so the estimate is unbiased.
            let (var_a, var_b, covariance) =
                (var_a / (n - 1.0), var_b / (n - 1.0), covariance / (n - 1.0));
            let numerator = (2.0 * mean_a * mean_b + C1) * (2.0 * covariance + C2);
            let denominator = (mean_a * mean_a + mean_b * mean_b + C1) * (var_a + var_b + C2);
            total += numerator / denominator;
            windows += 1;
        }
    }
    if windows == 0 {
        f64::NAN
    } else {
        total / windows as f64
    }
}

/// How much of the reference's edge energy survives in `actual`, in [0, ~1].
///
/// A denoiser can always win on MSE by erasing everything, so error alone
/// cannot decide whether a denoise is good. This measures gradient magnitude
/// where the *clean* image actually has edges, and asks how much of it is left.
/// One means the edges are as strong as the original; below one means they were
/// smoothed away; comfortably above one means the operation is manufacturing
/// contrast that was not there.
pub fn edge_retention(reference: &FloatImage, actual: &FloatImage) -> f64 {
    assert_eq!(
        reference.dimensions(),
        actual.dimensions(),
        "metrics need equal sizes"
    );
    let (width, height) = reference.dimensions();
    if width < 3 || height < 3 {
        return f64::NAN;
    }
    let gradient = |image: &FloatImage, x: u32, y: u32| -> f64 {
        let at = |dx: i32, dy: i32| -> f64 {
            let sx = (x as i32 + dx).clamp(0, width as i32 - 1) as u32;
            let sy = (y as i32 + dy).clamp(0, height as i32 - 1) as u32;
            f64::from(image.get(sx, sy).map_or(0.0, FloatRgba::luminance))
        };
        // Sobel, so a single noisy pixel does not read as an edge.
        let gx = at(1, -1) + 2.0 * at(1, 0) + at(1, 1) - at(-1, -1) - 2.0 * at(-1, 0) - at(-1, 1);
        let gy = at(-1, 1) + 2.0 * at(0, 1) + at(1, 1) - at(-1, -1) - 2.0 * at(0, -1) - at(1, -1);
        (gx * gx + gy * gy).sqrt()
    };
    let (mut reference_energy, mut actual_energy) = (0.0f64, 0.0f64);
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let r = gradient(reference, x, y);
            // Only where the clean image genuinely has an edge. Counting flat
            // areas would let surviving noise masquerade as retained detail.
            if r > 0.05 {
                reference_energy += r;
                actual_energy += gradient(actual, x, y);
            }
        }
    }
    if reference_energy <= 0.0 {
        f64::NAN
    } else {
        actual_energy / reference_energy
    }
}

/// Residual noise in the regions the clean image says are flat.
///
/// The companion to `edge_retention`: together they catch the two ways a
/// denoiser cheats, by leaving the noise or by destroying the picture.
pub fn flat_area_noise(reference: &FloatImage, actual: &FloatImage) -> f64 {
    assert_eq!(
        reference.dimensions(),
        actual.dimensions(),
        "metrics need equal sizes"
    );
    let (width, height) = reference.dimensions();
    if width < 3 || height < 3 {
        return f64::NAN;
    }
    let mut total = 0.0f64;
    let mut counted = 0u64;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            // A 3x3 neighbourhood of the clean image with no variation.
            let centre = reference.get(x, y).map_or(0.0, FloatRgba::luminance);
            let flat = (-1..=1).all(|dy| {
                (-1..=1).all(|dx| {
                    let sx = (x as i32 + dx) as u32;
                    let sy = (y as i32 + dy) as u32;
                    (reference.get(sx, sy).map_or(0.0, FloatRgba::luminance) - centre).abs() < 0.002
                })
            });
            if !flat {
                continue;
            }
            let d = f64::from(actual.get(x, y).map_or(0.0, FloatRgba::luminance) - centre);
            total += d * d;
            counted += 1;
        }
    }
    if counted == 0 {
        f64::NAN
    } else {
        (total / counted as f64).sqrt()
    }
}

/// A small deterministic generator, so every fixture is reproducible from a
/// seed and a failure can be re-created exactly.
pub struct Noise(u64);

impl Noise {
    pub const fn new(seed: u64) -> Self {
        // Never zero: xorshift is stuck there.
        Self(seed | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Uniform in [0, 1).
    pub fn unit(&mut self) -> f32 {
        (self.next() >> 11) as f32 / (1u64 << 53) as f32
    }

    /// Standard normal, via Box-Muller. Gaussian sensor noise is the case the
    /// denoisers are actually tuned against, so it is generated properly rather
    /// than approximated by summing uniforms.
    pub fn normal(&mut self) -> f32 {
        let u1 = self.unit().max(f32::MIN_POSITIVE);
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: u32, height: u32, value: f32) -> FloatImage {
        FloatImage::blank(width, height, FloatRgba::new(value, value, value, 1.0)).unwrap()
    }

    /// A metric that does not report perfection for identical input is not
    /// measuring what it claims to.
    #[test]
    fn identical_images_score_perfectly() {
        let image = flat(32, 32, 0.4);
        assert_eq!(mse(&image, &image), 0.0);
        assert!(psnr(&image, &image).is_infinite());
        assert!((ssim(&image, &image) - 1.0).abs() < 1e-9);
    }

    /// PSNR has a known closed form for a constant offset, so the
    /// implementation can be checked against arithmetic rather than intuition.
    #[test]
    fn psnr_matches_its_definition_for_a_known_offset() {
        let a = flat(16, 16, 0.5);
        let b = flat(16, 16, 0.6);
        // Every channel differs by 0.1, so the MSE is 0.01 and
        // PSNR = 10*log10(1/0.01) = 20 dB. The tolerance is f32-sized because
        // the inputs are f32: 0.6f32 - 0.5f32 is 0.10000002, not 0.1, which
        // moves the MSE by about 5e-9.
        assert!((mse(&a, &b) - 0.01).abs() < 1e-7);
        assert!((psnr(&a, &b) - 20.0).abs() < 1e-5);
    }

    /// SSIM must respond to structure, not just to error magnitude. A shifted
    /// copy has the same intensities and different structure.
    #[test]
    fn ssim_falls_when_structure_changes() {
        let mut a = FloatImage::blank(32, 32, FloatRgba::new(0.2, 0.2, 0.2, 1.0)).unwrap();
        let mut b = a.clone();
        for y in 0..32u32 {
            for x in 0..32u32 {
                let stripe_a = if (x / 4) % 2 == 0 { 0.8 } else { 0.2 };
                let stripe_b = if ((x + 4) / 4) % 2 == 0 { 0.8 } else { 0.2 };
                a.pixels_mut()[(y * 32 + x) as usize] =
                    FloatRgba::new(stripe_a, stripe_a, stripe_a, 1.0);
                b.pixels_mut()[(y * 32 + x) as usize] =
                    FloatRgba::new(stripe_b, stripe_b, stripe_b, 1.0);
            }
        }
        let shifted = ssim(&a, &b);
        assert!(
            shifted < 0.1,
            "an inverted stripe pattern scored {shifted}, so SSIM is not seeing structure"
        );
    }

    /// The point of `edge_retention`: blurring must score below one, and doing
    /// nothing must score one.
    #[test]
    fn edge_retention_distinguishes_blur_from_no_change() {
        let mut clean = FloatImage::blank(32, 32, FloatRgba::new(0.1, 0.1, 0.1, 1.0)).unwrap();
        for y in 0..32u32 {
            for x in 16..32u32 {
                clean.pixels_mut()[(y * 32 + x) as usize] = FloatRgba::new(0.9, 0.9, 0.9, 1.0);
            }
        }
        assert!((edge_retention(&clean, &clean) - 1.0).abs() < 1e-9);

        // Soften the edge column by averaging across it.
        let mut blurred = clean.clone();
        for y in 0..32u32 {
            for x in [15u32, 16] {
                blurred.pixels_mut()[(y * 32 + x) as usize] = FloatRgba::new(0.5, 0.5, 0.5, 1.0);
            }
        }
        let retained = edge_retention(&clean, &blurred);
        assert!(
            retained < 0.95,
            "softening the only edge still scored {retained}"
        );
    }

    /// `flat_area_noise` must see noise in flat regions and ignore edges.
    #[test]
    fn flat_area_noise_measures_only_flat_regions() {
        let clean = flat(32, 32, 0.5);
        assert_eq!(flat_area_noise(&clean, &clean), 0.0);
        let mut noisy = clean.clone();
        let mut rng = Noise::new(7);
        for pixel in noisy.pixels_mut() {
            let n = rng.normal() * 0.05;
            *pixel = FloatRgba::new(0.5 + n, 0.5 + n, 0.5 + n, 1.0);
        }
        let measured = flat_area_noise(&clean, &noisy);
        // Luminance of equal RGB is the channel value, so the residual should
        // land near the injected sigma.
        assert!(
            (0.02..0.09).contains(&measured),
            "injected sigma 0.05 measured as {measured}"
        );
    }

    /// The generator has to be reproducible or a failing fixture cannot be
    /// re-created, and roughly correct or the noise fixtures mean nothing.
    #[test]
    fn the_noise_generator_is_reproducible_and_roughly_normal() {
        let mut a = Noise::new(42);
        let mut b = Noise::new(42);
        for _ in 0..100 {
            assert_eq!(a.normal(), b.normal());
        }
        let mut rng = Noise::new(3);
        let samples: Vec<f64> = (0..20_000).map(|_| f64::from(rng.normal())).collect();
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        let variance =
            samples.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / samples.len() as f64;
        assert!(mean.abs() < 0.05, "mean {mean} is not near zero");
        assert!(
            (variance - 1.0).abs() < 0.1,
            "variance {variance} is not near one"
        );
    }
}
