//! Reducing an image as its rows arrive.
//!
//! A reduced copy of a source too large to hold has to be built without ever
//! holding the source, so the resampler consumes one source row at a time and
//! emits a destination row the moment it is complete. Memory is the destination
//! image plus a few rows, whatever the source's height.
//!
//! # The filter
//!
//! Exact area averaging: every destination pixel is the mean of the source over
//! the rectangle it covers, with partial pixels at the edges weighted by how much
//! of them lies inside. For *reduction* this is the right filter, not a
//! compromise: it is a true box filter at the destination's own pitch, so it
//! cannot alias the way point sampling does, and it conserves energy — the sum of
//! the image is preserved, which a filter with negative lobes does not guarantee.
//!
//! The geometry is done in integers. A source row `r` occupies `[r*dh, (r+1)*dh)`
//! in units of `1/dh` of a source row, and destination row `k` occupies
//! `[k*sh, (k+1)*sh)` in the same units, so the overlap of the two is an exact
//! integer and the weight is that integer over `sh`. No accumulated floating
//! point position can drift across a hundred million rows.
//!
//! # What it expects
//!
//! Rows of **premultiplied linear** `f32` RGBA. Averaging must happen in linear
//! light, because averaging gamma-encoded values darkens every edge; and in
//! premultiplied form, because otherwise a transparent pixel's colour leaks into
//! its neighbours. The caller converts; the result is returned premultiplied and
//! [`AreaReducer::finish`] un-premultiplies it.
use crate::color::{FloatImage, FloatRgba};
use crate::error::AppError;

/// Precomputed horizontal weights for one destination column.
struct Span {
    first: u32,
    weights_start: usize,
    weights_len: usize,
}

pub struct AreaReducer {
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    spans: Vec<Span>,
    weights: Vec<f64>,
    /// Source rows consumed so far.
    rows_in: u32,
    /// Destination rows emitted so far.
    rows_out: u32,
    /// Accumulators for destination row `rows_out`, and for the one after it.
    current: Vec<f64>,
    following: Vec<f64>,
    /// One source row reduced horizontally.
    scratch: Vec<f64>,
    /// The result, built in place: premultiplied while rows arrive, straight when
    /// [`AreaReducer::finish`] has gone over it. One buffer, because a second one the
    /// size of the result would double the peak of the very thing this exists to bound.
    output: FloatImage,
}

impl AreaReducer {
    pub fn new(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Result<Self, AppError> {
        if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 || dst_w > src_w || dst_h > src_h {
            return Err(AppError::InvalidOperation(
                "a reduced copy must be non-empty and no larger than its source".into(),
            ));
        }
        crate::resources::checked_pixels(dst_w, dst_h)?;

        let (sw, dw) = (u64::from(src_w), u64::from(dst_w));
        let mut spans = Vec::with_capacity(dst_w as usize);
        let mut weights = Vec::with_capacity((src_w + dst_w) as usize + 2);
        for i in 0..dw {
            // Destination column `i` covers [i*sw, (i+1)*sw) in 1/dw units; source
            // column `j` covers [j*dw, (j+1)*dw).
            let lo = i * sw;
            let hi = (i + 1) * sw;
            let first = lo / dw;
            let last = (hi - 1) / dw;
            let start = weights.len();
            for j in first..=last {
                let overlap = hi.min((j + 1) * dw) - lo.max(j * dw);
                weights.push(overlap as f64 / sw as f64);
            }
            spans.push(Span {
                first: first as u32,
                weights_start: start,
                weights_len: weights.len() - start,
            });
        }

        let row = dst_w as usize * 4;
        let output = FloatImage::blank(dst_w, dst_h, FloatRgba::TRANSPARENT)?;
        Ok(Self {
            src_w,
            src_h,
            dst_w,
            dst_h,
            spans,
            weights,
            rows_in: 0,
            rows_out: 0,
            current: vec![0.0; row],
            following: vec![0.0; row],
            scratch: vec![0.0; row],
            output,
        })
    }

    /// Consumes the next source row: `src_w * 4` premultiplied linear samples.
    pub fn push_row(&mut self, row: &[f32]) -> Result<(), AppError> {
        if row.len() != self.src_w as usize * 4 {
            return Err(AppError::InvalidOperation(
                "a source row had the wrong width".into(),
            ));
        }
        if self.rows_in >= self.src_h {
            return Err(AppError::InvalidOperation(
                "more rows were supplied than the source has".into(),
            ));
        }

        // Horizontal pass.
        for (column, span) in self.spans.iter().enumerate() {
            let mut acc = [0.0f64; 4];
            let weights = &self.weights[span.weights_start..span.weights_start + span.weights_len];
            let base = span.first as usize * 4;
            for (offset, weight) in weights.iter().enumerate() {
                let p = &row[base + offset * 4..base + offset * 4 + 4];
                acc[0] += f64::from(p[0]) * weight;
                acc[1] += f64::from(p[1]) * weight;
                acc[2] += f64::from(p[2]) * weight;
                acc[3] += f64::from(p[3]) * weight;
            }
            self.scratch[column * 4..column * 4 + 4].copy_from_slice(&acc);
        }

        // Vertical pass. This source row lies in destination row `k` and, if it
        // straddles a boundary, in `k + 1`.
        let (sh, dh) = (u64::from(self.src_h), u64::from(self.dst_h));
        let r = u64::from(self.rows_in);
        let lo = r * dh;
        let hi = (r + 1) * dh;
        let k = lo / sh;
        let boundary = (k + 1) * sh;
        let in_first = hi.min(boundary) - lo;
        let in_second = hi - hi.min(boundary);
        debug_assert_eq!(k, u64::from(self.rows_out));
        let w_first = in_first as f64 / sh as f64;
        for (acc, value) in self.current.iter_mut().zip(&self.scratch) {
            *acc += value * w_first;
        }
        if in_second > 0 {
            let w_second = in_second as f64 / sh as f64;
            for (acc, value) in self.following.iter_mut().zip(&self.scratch) {
                *acc += value * w_second;
            }
        }
        self.rows_in += 1;

        // The destination row is complete once the source has reached its end.
        if hi >= boundary {
            self.emit();
        }
        Ok(())
    }

    fn emit(&mut self) {
        let width = self.dst_w as usize;
        let start = self.rows_out as usize * width;
        for (out, value) in self.output.pixels_mut()[start..start + width]
            .iter_mut()
            .zip(self.current.chunks_exact(4))
        {
            *out = FloatRgba::new(
                value[0] as f32,
                value[1] as f32,
                value[2] as f32,
                value[3] as f32,
            );
        }
        std::mem::swap(&mut self.current, &mut self.following);
        self.following.iter_mut().for_each(|v| *v = 0.0);
        self.rows_out += 1;
    }

    /// Source rows still expected.
    pub fn rows_remaining(&self) -> u32 {
        self.src_h - self.rows_in
    }

    /// Completes the reduction and returns straight-alpha linear pixels.
    pub fn finish(self) -> Result<FloatImage, AppError> {
        if self.rows_out != self.dst_h {
            return Err(AppError::InvalidOperation(
                "the source ended before every row was supplied".into(),
            ));
        }
        let mut image = self.output;
        for pixel in image.pixels_mut() {
            let alpha = pixel.alpha.clamp(0.0, 1.0);
            *pixel = if alpha > 0.0 {
                FloatRgba::new(
                    pixel.red / alpha,
                    pixel.green / alpha,
                    pixel.blue / alpha,
                    alpha,
                )
            } else {
                FloatRgba::TRANSPARENT
            };
        }
        Ok(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(
        src_w: u32,
        src_h: u32,
        dst_w: u32,
        dst_h: u32,
        f: impl Fn(u32, u32) -> [f32; 4],
    ) -> FloatImage {
        let mut reducer = AreaReducer::new(src_w, src_h, dst_w, dst_h).unwrap();
        for y in 0..src_h {
            let mut row = Vec::new();
            for x in 0..src_w {
                row.extend_from_slice(&f(x, y));
            }
            reducer.push_row(&row).unwrap();
        }
        reducer.finish().unwrap()
    }

    #[test]
    fn a_flat_image_stays_flat_at_any_ratio() {
        for (sw, sh, dw, dh) in [
            (100, 100, 10, 10),
            (101, 97, 13, 7),
            (64, 64, 63, 63),
            (10, 10, 1, 1),
        ] {
            let image = run(sw, sh, dw, dh, |_, _| [0.25, 0.5, 0.75, 1.0]);
            for pixel in image.pixels() {
                assert!(
                    (pixel.red - 0.25).abs() < 1e-5,
                    "{sw}x{sh}->{dw}x{dh}: {pixel:?}"
                );
                assert!((pixel.green - 0.5).abs() < 1e-5);
                assert!((pixel.blue - 0.75).abs() < 1e-5);
                assert!((pixel.alpha - 1.0).abs() < 1e-5);
            }
        }
    }

    /// Two-to-one is a plain 2x2 mean, which can be checked by hand.
    #[test]
    fn halving_is_the_mean_of_each_two_by_two() {
        let image = run(4, 2, 2, 1, |x, _| {
            let v = x as f32; // columns 0,1,2,3
            [v, v, v, 1.0]
        });
        // (0+1)/2 and (2+3)/2, each averaged over two identical rows.
        assert!((image.pixels()[0].red - 0.5).abs() < 1e-6);
        assert!((image.pixels()[1].red - 2.5).abs() < 1e-6);
    }

    /// Energy conservation: the mean of the image is unchanged by reduction, at
    /// ratios that are not integers, where a sloppy filter would lose or gain.
    #[test]
    fn the_mean_of_the_image_is_preserved() {
        let pattern = |x: u32, y: u32| {
            let v = (((x * 37 + y * 91) % 255) as f32) / 255.0;
            [v, 1.0 - v, (v * 0.5), 1.0]
        };
        for (sw, sh, dw, dh) in [(97, 89, 31, 29), (200, 150, 67, 51), (13, 11, 5, 3)] {
            let source = run(sw, sh, sw, sh, pattern);
            let reduced = run(sw, sh, dw, dh, pattern);
            let mean = |image: &FloatImage| {
                image.pixels().iter().map(|p| f64::from(p.red)).sum::<f64>()
                    / image.pixels().len() as f64
            };
            assert!(
                (mean(&source) - mean(&reduced)).abs() < 1e-4,
                "{sw}x{sh}->{dw}x{dh}: {} vs {}",
                mean(&source),
                mean(&reduced)
            );
        }
    }

    /// Premultiplied averaging: a transparent pixel's colour must not leak into
    /// its opaque neighbour.
    #[test]
    fn a_transparent_pixel_does_not_tint_its_neighbour() {
        // Left pixel opaque red; right pixel fully transparent *green*. Premultiplied,
        // the transparent one contributes nothing.
        let image = run(2, 1, 1, 1, |x, _| {
            if x == 0 {
                [1.0, 0.0, 0.0, 1.0]
            } else {
                [0.0, 0.0, 0.0, 0.0]
            }
        });
        let p = image.pixels()[0];
        assert!((p.alpha - 0.5).abs() < 1e-6);
        assert!((p.red - 1.0).abs() < 1e-5, "red {}", p.red);
        assert!(p.green.abs() < 1e-5, "green leaked: {}", p.green);
    }

    #[test]
    fn rows_arrive_and_are_emitted_incrementally() {
        let mut reducer = AreaReducer::new(8, 8, 2, 2).unwrap();
        let row = vec![0.5f32; 8 * 4];
        for _ in 0..4 {
            reducer.push_row(&row).unwrap();
        }
        assert_eq!(
            reducer.rows_out, 1,
            "the first destination row was not emitted after its source rows"
        );
        assert_eq!(reducer.rows_remaining(), 4);
    }

    #[test]
    fn misuse_is_an_error_not_a_panic() {
        assert!(AreaReducer::new(0, 5, 1, 1).is_err());
        assert!(AreaReducer::new(10, 10, 20, 5).is_err(), "upscaling");
        let mut reducer = AreaReducer::new(4, 4, 2, 2).unwrap();
        assert!(reducer.push_row(&[0.0; 3]).is_err(), "wrong width");
        for _ in 0..4 {
            reducer.push_row(&[0.0; 16]).unwrap();
        }
        assert!(reducer.push_row(&[0.0; 16]).is_err(), "too many rows");
        let short = AreaReducer::new(4, 4, 2, 2).unwrap();
        assert!(short.finish().is_err(), "an unfinished source");
    }

    /// Hostile sizes must fail before any buffer exists.
    #[test]
    fn an_enormous_destination_is_refused_before_allocating() {
        assert!(AreaReducer::new(u32::MAX / 2, u32::MAX / 2, u32::MAX / 2, u32::MAX / 2).is_err());
    }
}
