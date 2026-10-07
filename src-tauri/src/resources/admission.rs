//! Deciding what to do with a source that may not fit.
//!
//! A conventional PhotoForge document holds its image as 16-byte-per-pixel
//! linear float. A file can therefore be perfectly decodable and still not be
//! openable that way on a given machine: a 100 MP JPEG is 400 MB of ordinary
//! bytes and 1.6 GB of working float. The old behaviour was one error,
//! `ImageTooLarge`, for both "this is not an image we can read" and "we can read
//! it, but not like that".
//!
//! The planner keeps them apart. It takes a [`SourceProbe`] — what the header
//! says, and what the decoder for that format can honestly do — and a budget, and
//! returns a verdict plus the alternatives that genuinely exist.
//!
//! # Decoder capability is stated, not assumed
//!
//! Whether a region can be cut out of a source cheaply depends on the decoder,
//! and the difference is the whole point of this module:
//!
//! * [`RegionDecode::Segments`] / [`RegionDecode::Rows`] — memory follows the
//!   *region*. This is real region decoding.
//! * [`RegionDecode::TransientFull`] — the whole frame is decoded at its native
//!   depth, the region is kept and the rest freed. Peak memory follows the
//!   *source*, at a few bytes per pixel instead of sixteen. That is a real saving
//!   and it is **not** region decoding, and the planner prices it as what it is.
//!
//! A decoder that allocates the whole source and then crops is never reported as
//! having region support.
use super::policy::{Budget, ResourceLimits, BYTES_PER_WORKING_PIXEL, HARD_MAX_SOURCE_DIMENSION};
use serde::{Deserialize, Serialize};

/// Fixed overhead of a render (output encode buffers, caches warm-up), carried
/// over from the original estimate so open-time figures stay comparable.
const RENDER_FIXED_BYTES: u128 = 40_960_000;

/// What every decoder holds besides its pixels: the codec's own buffers, a reader's
/// buffer (the PNG reader's alone is 1 MiB), the resampler's weights and a few rows.
/// Measured at under 2 MiB on every decoder; priced at 4.
pub const DECODER_FIXED_BYTES: u128 = 4 * 1024 * 1024;

/// What opening a source builds beside the document itself: the bounded preview, as a
/// float copy and as an 8-bit one, 20 bytes for each of at most 1600 x 1600 pixels.
/// `tests::the_preview_allowance_is_the_previews_size` ties it to the real bound.
pub const OPEN_PREVIEW_BYTES: u128 = 1_600 * 1_600 * 20;

/// A document whose peak would exceed this share of the budget is admitted with
/// a warning rather than silently.
const WARN_PERCENT: u128 = 50;

/// Tightest ratio of raw raster bytes to file bytes that PNG's compression can
/// produce. Deflate cannot exceed 1032:1; the slack covers container overhead.
/// A header claiming more than this cannot be telling the truth.
const PNG_MAX_EXPANSION: u128 = 1_100;

/// A generous ceiling for baseline and progressive JPEG, whose smallest coded
/// 8x8 block is a couple of bits.
const JPEG_MAX_EXPANSION: u128 = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SourceKind {
    Png,
    Jpeg,
    WebP,
    Dng,
}

/// How a decoder can deliver a window of the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RegionDecode {
    /// Decodes row by row and keeps only the window. Memory is the window plus
    /// one row; time still runs down to the window's last row.
    Rows,
    /// Decodes only the tiles or strips that intersect the window.
    ///
    /// `fixed_bytes` is what is held whatever the window: one decoded segment at a
    /// time (the whole sensor, for a file stored as a single strip) and the margin
    /// a window carries beyond its region. `bytes_per_pixel` is what the decoder
    /// and everything developed from the window hold for each window pixel.
    #[serde(rename_all = "camelCase")]
    Segments {
        fixed_bytes: u64,
        bytes_per_pixel: u64,
    },
    /// Decodes the entire frame at native depth, keeps the window and frees the
    /// rest. Peak memory follows the source.
    #[serde(rename_all = "camelCase")]
    TransientFull { bytes_per_pixel: u64 },
    /// No way to produce a window.
    None,
}

/// How a decoder can deliver a smaller copy of the whole source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ReducedDecode {
    /// Decodes row by row, reducing as it goes. Memory is the reduced image
    /// plus a few rows.
    Rows,
    /// Decodes the entire frame at native depth, then resamples in bands.
    #[serde(rename_all = "camelCase")]
    TransientFull {
        bytes_per_pixel: u64,
    },
    /// Decodes straight to 1/2, 1/4 or 1/8 size in the DCT domain, so the full
    /// frame is never produced. Memory follows the *scaled* frame.
    DctScaled,
    None,
}

/// Measured on a 108 MP JPEG: `jpeg-decoder` holds about eight bytes per
/// *output* pixel at a DCT scale (15 MiB for 1.69 MP, 43 MiB for 6.75 MP), and
/// about the same at full scale (623 MiB for 108 MP).
pub const DCT_BYTES_PER_OUTPUT_PIXEL: u128 = 8;
pub const DCT_FIXED_BYTES: u128 = 16 * 1024 * 1024;

/// Extra bytes the row-streaming reducer holds per source *column*: one row as
/// encoded and as linear `f32` RGBA, plus its f64 weights.
pub const ROW_REDUCER_BYTES_PER_COLUMN: u128 = 48;

/// The DCT scale denominator a decode to `dst` pixels will use: the largest of
/// 8, 4, 2 that still yields at least the requested size, else 1.
pub fn dct_factor(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> u32 {
    for factor in [8u32, 4, 2] {
        if src_w.div_ceil(factor) >= dst_w && src_h.div_ceil(factor) >= dst_h {
            return factor;
        }
    }
    1
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeCapabilities {
    pub region: RegionDecode,
    pub reduced: ReducedDecode,
}

/// What is known about a source from its header, before any pixel is decoded.
///
/// Dimensions are `u64` so that a hostile header's value is carried without being
/// truncated into something plausible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceProbe {
    pub kind: SourceKind,
    pub width: u64,
    pub height: u64,
    /// Bytes per pixel the decoder produces natively (for example 4 for RGBA8).
    pub native_bytes_per_pixel: u64,
    /// Bytes per pixel the decoder holds while the *whole* source is decoded for
    /// a full open. Usually the native depth; a camera RAW holds the sensor
    /// samples, the normalised plane, the demosaiced colour and the float image
    /// together, which is far more than the two bytes a photosite is stored in.
    pub full_decode_bytes_per_pixel: u64,
    pub file_bytes: u64,
    pub capabilities: DecodeCapabilities,
}

impl SourceProbe {
    pub fn pixels(&self) -> u128 {
        u128::from(self.width) * u128::from(self.height)
    }
}

/// Why a source was refused outright, as opposed to merely being too large.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Refusal {
    /// A dimension is zero.
    Empty,
    /// A dimension is beyond what any coordinate in the application can hold.
    ImplausibleDimensions { width: u64, height: u64 },
    /// The header claims more raster than the file's compression could possibly
    /// hold, so the dimensions are not honest.
    ImpossibleExpansion { claimed_bytes: u64, file_bytes: u64 },
}

/// Why a verdict leans on the machine's current state rather than its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Shortfall {
    /// The budget is enough, but free memory right now is not. Closing other
    /// applications would change the answer; it is not a limit of PhotoForge.
    Momentary,
    /// The budget itself is too small for this.
    Budget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Verdict {
    /// Opens as an ordinary full-resolution document with room to spare.
    FullResolution,
    /// Opens, but will use a large share of the budget.
    #[serde(rename_all = "camelCase")]
    FullResolutionWithWarning { peak_percent_of_budget: u32 },
    /// Too large to open whole; a bounded region can be opened instead.
    RegionRequired,
    /// Too large to open whole, and a region cannot be decoded cheaply, but a
    /// reduced copy can.
    ReducedCopyRecommended,
    /// Nothing bounded works on this machine right now.
    #[serde(rename_all = "camelCase")]
    InsufficientResources { shortfall: Shortfall },
    /// Refused outright.
    Unsafe { refusal: Refusal },
}

/// One thing the user may do about the source, with its cost.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AdmissionOption {
    #[serde(rename_all = "camelCase")]
    OpenFull { peak_bytes: u64 },
    #[serde(rename_all = "camelCase")]
    OpenRegion {
        /// Most pixels a region may hold.
        max_region_pixels: u64,
        /// What decoding it costs beyond the region's own working memory.
        decode: RegionDecode,
        /// Peak when the region is the largest allowed.
        peak_bytes_at_max: u64,
    },
    #[serde(rename_all = "camelCase")]
    OpenReduced {
        /// Largest scale (0..1) whose result fits.
        max_scale: f64,
        decode: ReducedDecode,
    },
}

/// Why full out-of-core admission is not offered.
///
/// It appears in every report so the interface can say so, rather than leaving
/// a missing option unexplained.
pub const OUT_OF_CORE_UNAVAILABLE: &str =
    "Open Full Resolution (Out-of-Core) is not available: the pixel store, the layer \
     compositor's source access and project persistence each hold complete buffers, so \
     decoding in tiles would only move the allocation. See docs/oversized-images.md.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionReport {
    pub verdict: Verdict,
    pub options: Vec<AdmissionOption>,
    /// Peak bytes of opening the whole source as a conventional document.
    pub full_peak_bytes: u64,
    /// Coefficients for pricing a region, when the decoder can produce one.
    pub region_cost: Option<RegionCost>,
    /// The budget the decision was made against.
    pub budget_bytes: u64,
    /// Free memory at the moment of the decision.
    pub available_bytes: Option<u64>,
    pub out_of_core: String,
}

/// Peak bytes of opening `pixels` pixels as a conventional float document, with
/// a decode that holds `decode_bytes` while the working copy is built.
///
/// Two phases, and the larger governs. *Opening* holds the decoded native frame
/// and the float copy together while one is converted into the other. *Editing*
/// holds the float image, a canvas frame of the same size, the encoded original
/// the document retains, and the output buffer.
fn document_peak(pixels: u128, native_bpp: u128, file_bytes: u128, decode_bytes: u128) -> u128 {
    let working = pixels * BYTES_PER_WORKING_PIXEL as u128;
    let opening = file_bytes + decode_bytes + working;
    let editing = working            // the float image
        + working                    // a canvas frame to composite into
        + pixels * native_bpp        // the original the document keeps
        + pixels * 8                 // 16-bit output
        + RENDER_FIXED_BYTES;
    opening.max(editing)
}

fn saturate(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

/// The region peak as two affine functions of the region's pixel count.
///
/// Peak is the larger of building the working copy (the decode, plus the float
/// image) and editing it (the float image, a canvas frame, the retained original
/// and the output). Both grow linearly in the region, with a fixed part from the
/// file and the decoder, so the whole model reduces to four numbers.
///
/// Exposing the numbers rather than only the verdict is what lets an interface
/// price a rectangle while it is being dragged without carrying a second copy of
/// the model that could drift from this one. The planner's own region figures are
/// computed from this type, so there is one formula.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionCost {
    pub opening_fixed: u64,
    pub opening_per_pixel: u64,
    pub editing_fixed: u64,
    pub editing_per_pixel: u64,
}

impl RegionCost {
    pub fn peak(&self, region_pixels: u128) -> u128 {
        let opening =
            u128::from(self.opening_fixed) + u128::from(self.opening_per_pixel) * region_pixels;
        let editing =
            u128::from(self.editing_fixed) + u128::from(self.editing_per_pixel) * region_pixels;
        opening.max(editing)
    }
}

/// The cost coefficients for opening a region of `probe`, or `None` if its
/// decoder cannot produce a region at all.
pub fn region_cost(probe: &SourceProbe) -> Option<RegionCost> {
    let native = u128::from(probe.native_bytes_per_pixel);
    let file = u128::from(probe.file_bytes);
    let working = BYTES_PER_WORKING_PIXEL as u128;
    // What the decoder holds while the working copy is being built: a fixed
    // part, and a part that grows with the region.
    let (decode_fixed, decode_per_pixel) = match probe.capabilities.region {
        RegionDecode::Rows => (
            u128::from(probe.width) * native + DECODER_FIXED_BYTES,
            native,
        ),
        RegionDecode::Segments {
            fixed_bytes,
            bytes_per_pixel,
        } => (u128::from(fixed_bytes), u128::from(bytes_per_pixel)),
        RegionDecode::TransientFull { bytes_per_pixel } => (
            probe.pixels() * u128::from(bytes_per_pixel) + DECODER_FIXED_BYTES,
            0,
        ),
        RegionDecode::None => return None,
    };
    Some(RegionCost {
        opening_fixed: saturate(file + decode_fixed + OPEN_PREVIEW_BYTES),
        opening_per_pixel: saturate(decode_per_pixel + working),
        editing_fixed: saturate(RENDER_FIXED_BYTES),
        editing_per_pixel: saturate(working * 2 + native + 8),
    })
}

/// Decides what to do with a source.
///
/// `available` is free memory right now, or `None` if it could not be read.
/// `resident` is what the session already holds and will keep — pass zero when
/// the new source replaces the open document.
pub fn plan(
    probe: &SourceProbe,
    budget: &Budget,
    limits: &ResourceLimits,
    available: Option<u64>,
    resident: u64,
) -> AdmissionReport {
    let refuse = |refusal: Refusal| AdmissionReport {
        verdict: Verdict::Unsafe { refusal },
        options: Vec::new(),
        full_peak_bytes: 0,
        region_cost: None,
        budget_bytes: budget.bytes,
        available_bytes: available,
        out_of_core: OUT_OF_CORE_UNAVAILABLE.into(),
    };

    if probe.width == 0 || probe.height == 0 {
        return refuse(Refusal::Empty);
    }
    if probe.width > HARD_MAX_SOURCE_DIMENSION || probe.height > HARD_MAX_SOURCE_DIMENSION {
        return refuse(Refusal::ImplausibleDimensions {
            width: probe.width,
            height: probe.height,
        });
    }

    let pixels = probe.pixels();
    let native = u128::from(probe.native_bytes_per_pixel);
    let raw_bytes = pixels * native;
    let expansion_cap = match probe.kind {
        SourceKind::Png => Some(PNG_MAX_EXPANSION),
        SourceKind::Jpeg => Some(JPEG_MAX_EXPANSION),
        // Lossless WebP and DNG have no bound tight enough to claim as proof.
        SourceKind::WebP | SourceKind::Dng => None,
    };
    if let Some(cap) = expansion_cap {
        if raw_bytes > u128::from(probe.file_bytes).max(1) * cap {
            return refuse(Refusal::ImpossibleExpansion {
                claimed_bytes: saturate(raw_bytes),
                file_bytes: probe.file_bytes,
            });
        }
    }

    let file = u128::from(probe.file_bytes);
    let budget_bytes = u128::from(budget.bytes).saturating_sub(u128::from(resident));
    let available_bytes = available.map(u128::from);

    // ---- Whole source as a conventional document.
    let decode_full = pixels * u128::from(probe.full_decode_bytes_per_pixel);
    let full_peak = document_peak(pixels, native, file, decode_full);
    let within_pixel_ceiling = pixels <= u128::from(limits.working_pixels());
    let full_fits_budget = within_pixel_ceiling && full_peak <= budget_bytes;
    let full_fits_now = available_bytes.is_none_or(|free| full_peak <= free);

    // ---- A region of it.
    let region_option = region_option(probe, limits, budget_bytes, available_bytes);
    // ---- A reduced copy of it.
    let reduced_option = reduced_option(probe, limits, budget_bytes, available_bytes);

    let mut options = Vec::new();
    let verdict = if full_fits_budget && full_fits_now {
        options.push(AdmissionOption::OpenFull {
            peak_bytes: saturate(full_peak),
        });
        // Even a source that fits whole may be better opened bounded if it
        // would dominate the budget.
        let percent = (full_peak * 100)
            .checked_div(budget_bytes)
            .map_or(100, |value| value as u32);
        if full_peak * 100 > budget_bytes * WARN_PERCENT {
            Verdict::FullResolutionWithWarning {
                peak_percent_of_budget: percent,
            }
        } else {
            Verdict::FullResolution
        }
    } else if full_fits_budget && !full_fits_now {
        // Fits PhotoForge's budget but not the machine as it is at this moment.
        Verdict::InsufficientResources {
            shortfall: Shortfall::Momentary,
        }
    } else if region_option.is_some() {
        Verdict::RegionRequired
    } else if reduced_option.is_some() {
        Verdict::ReducedCopyRecommended
    } else {
        Verdict::InsufficientResources {
            shortfall: Shortfall::Budget,
        }
    };

    options.extend(region_option);
    options.extend(reduced_option);

    AdmissionReport {
        verdict,
        options,
        full_peak_bytes: saturate(full_peak),
        region_cost: region_cost(probe),
        budget_bytes: budget.bytes,
        available_bytes: available,
        out_of_core: OUT_OF_CORE_UNAVAILABLE.into(),
    }
}

fn region_option(
    probe: &SourceProbe,
    limits: &ResourceLimits,
    budget_bytes: u128,
    available: Option<u128>,
) -> Option<AdmissionOption> {
    let cost = region_cost(probe)?;
    let ceiling = budget_bytes.min(available.unwrap_or(u128::MAX));
    let pixel_cap = u128::from(limits.working_pixels()).min(probe.pixels());

    // Largest region whose total peak fits. Peak is monotonic in region size, so
    // a binary search finds the boundary without a closed form for each decoder.
    if pixel_cap == 0 || cost.peak(1) > ceiling {
        return None;
    }
    let (mut low, mut high) = (1u128, pixel_cap);
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if cost.peak(mid) <= ceiling {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    Some(AdmissionOption::OpenRegion {
        max_region_pixels: saturate(low),
        decode: probe.capabilities.region,
        peak_bytes_at_max: saturate(cost.peak(low)),
    })
}

/// Bytes held while a reduced copy at `scale` (0..1, per side) is produced, not
/// counting the reduced image itself.
///
/// It is a function of the scale for DCT decoding, where the decode scale is a step
/// function of it. Public so that the accuracy gate (`tests/estimate_accuracy.rs`)
/// prices a decode with the planner's own formula rather than a copy of it.
pub fn reduced_transient_bytes(probe: &SourceProbe, scale: f64) -> Option<u128> {
    let native = u128::from(probe.native_bytes_per_pixel);
    let src_w = u32::try_from(probe.width).unwrap_or(u32::MAX);
    let src_h = u32::try_from(probe.height).unwrap_or(u32::MAX);
    match probe.capabilities.reduced {
        ReducedDecode::Rows => Some(
            u128::from(probe.width) * (native + ROW_REDUCER_BYTES_PER_COLUMN) + DECODER_FIXED_BYTES,
        ),
        ReducedDecode::TransientFull { bytes_per_pixel } => {
            Some(probe.pixels() * u128::from(bytes_per_pixel) + DECODER_FIXED_BYTES)
        }
        ReducedDecode::DctScaled => {
            let dst_w = (f64::from(src_w) * scale).ceil() as u32;
            let dst_h = (f64::from(src_h) * scale).ceil() as u32;
            let factor = dct_factor(src_w, src_h, dst_w.max(1), dst_h.max(1));
            let scaled = u128::from(src_w.div_ceil(factor)) * u128::from(src_h.div_ceil(factor));
            Some(scaled * DCT_BYTES_PER_OUTPUT_PIXEL + DCT_FIXED_BYTES)
        }
        ReducedDecode::None => None,
    }
}

fn reduced_option(
    probe: &SourceProbe,
    limits: &ResourceLimits,
    budget_bytes: u128,
    available: Option<u128>,
) -> Option<AdmissionOption> {
    let file = u128::from(probe.file_bytes);
    let native = u128::from(probe.native_bytes_per_pixel);
    let ceiling = budget_bytes.min(available.unwrap_or(u128::MAX));

    // Bytes held while the reduced image is produced, as a function of the scale asked for.
    let transient = |scale: f64, _reduced: u128| reduced_transient_bytes(probe, scale);
    let peak_at = |scale: f64| -> Option<u128> {
        let reduced = ((probe.pixels() as f64) * scale * scale).ceil() as u128;
        if reduced == 0 || reduced > u128::from(limits.working_pixels()) {
            return None;
        }
        Some(document_peak(
            reduced,
            native,
            file,
            transient(scale, reduced)? + OPEN_PREVIEW_BYTES,
        ))
    };
    peak_at(1e-9)?;
    // Peak grows with scale, so the largest fitting scale is a boundary.
    let (mut low, mut high) = (0.0f64, 1.0f64);
    for _ in 0..60 {
        let mid = (low + high) / 2.0;
        if peak_at(mid).is_some_and(|peak| peak <= ceiling) {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low > 0.0).then_some(AdmissionOption::OpenReduced {
        max_scale: low,
        decode: probe.capabilities.reduced,
    })
}

#[cfg(test)]
mod tests {
    use super::super::memory::SystemMemory;
    use super::super::policy::{compute_budget, BudgetMode, GIB};
    use super::*;

    /// The planner's allowance for the preview must be the preview's real bound.
    #[test]
    fn the_preview_allowance_is_the_previews_size() {
        let edge = u128::from(crate::layers::PREVIEW_MAX_DIMENSION);
        assert_eq!(OPEN_PREVIEW_BYTES, edge * edge * 20);
    }

    fn budget_of(total_gib: u64, available_gib: u64) -> (Budget, ResourceLimits) {
        let budget = compute_budget(
            BudgetMode::Automatic,
            Some(SystemMemory {
                total_physical: total_gib * GIB,
                available_physical: available_gib * GIB,
            }),
        );
        (budget, ResourceLimits::from_budget(budget.bytes))
    }

    fn png(width: u64, height: u64, file_bytes: u64, region: RegionDecode) -> SourceProbe {
        SourceProbe {
            kind: SourceKind::Png,
            width,
            height,
            native_bytes_per_pixel: 4,
            full_decode_bytes_per_pixel: 4,
            file_bytes,
            capabilities: DecodeCapabilities {
                region,
                reduced: ReducedDecode::Rows,
            },
        }
    }

    fn run(probe: &SourceProbe, machine: (u64, u64)) -> AdmissionReport {
        let (budget, limits) = budget_of(machine.0, machine.1);
        plan(probe, &budget, &limits, Some(machine.1 * GIB), 0)
    }

    /// Case 1: an ordinary photograph opens, with no warning and one option.
    #[test]
    fn a_normal_image_is_admitted_without_fuss() {
        let report = run(&png(4000, 3000, 6_000_000, RegionDecode::Rows), (16, 12));
        assert_eq!(report.verdict, Verdict::FullResolution);
        assert!(matches!(
            report.options[0],
            AdmissionOption::OpenFull { .. }
        ));
    }

    /// The same source, judged against two machines, gets two different answers.
    /// That is the whole point of replacing a constant with a budget.
    #[test]
    fn the_same_source_is_judged_by_the_machine_it_is_on() {
        // 150 MP, 8-bit.
        let probe = png(15_000, 10_000, 90_000_000, RegionDecode::Rows);
        let large = run(&probe, (128, 103));
        let small = run(&probe, (8, 4));
        assert!(
            matches!(
                large.verdict,
                Verdict::FullResolution | Verdict::FullResolutionWithWarning { .. }
            ),
            "a 128 GiB machine refused 150 MP: {:?}",
            large.verdict
        );
        assert_eq!(small.verdict, Verdict::RegionRequired);
    }

    /// Case 10: a constrained machine is offered region and reduced options,
    /// and only those that exist.
    #[test]
    fn an_oversized_source_on_a_small_machine_offers_bounded_options() {
        let probe = png(20_000, 15_000, 120_000_000, RegionDecode::Rows);
        let report = run(&probe, (8, 4));
        assert_eq!(report.verdict, Verdict::RegionRequired);
        assert!(report
            .options
            .iter()
            .any(|o| matches!(o, AdmissionOption::OpenRegion { .. })));
        assert!(report
            .options
            .iter()
            .any(|o| matches!(o, AdmissionOption::OpenReduced { .. })));
        assert!(
            !report
                .options
                .iter()
                .any(|o| matches!(o, AdmissionOption::OpenFull { .. })),
            "a source that does not fit was offered whole"
        );
        assert!(report.out_of_core.contains("not available"));
    }

    /// A decoder that allocates the whole source first is priced for that, and
    /// is not credited with region savings it does not have.
    ///
    /// The source is chosen so the transient decode is most of the budget. A
    /// region needs `file + transient + 16 * region` bytes while it is being
    /// built, so a transient decoder's region shrinks as the source grows
    /// whereas a row decoder's stays at the per-image ceiling.
    #[test]
    fn a_decoder_without_region_support_is_not_credited_with_it() {
        // 625 MP at 4 bytes is a 2.5 GB transient decode, against a budget of
        // about 2.6 GiB on an 8 GiB machine with 4 GiB free.
        let rows = png(25_000, 25_000, 120_000_000, RegionDecode::Rows);
        let transient = png(
            25_000,
            25_000,
            120_000_000,
            RegionDecode::TransientFull { bytes_per_pixel: 4 },
        );
        let cheap = run(&rows, (8, 4));
        let costly = run(&transient, (8, 4));
        let region_of = |report: &AdmissionReport| {
            report.options.iter().find_map(|o| match o {
                AdmissionOption::OpenRegion {
                    max_region_pixels, ..
                } => Some(*max_region_pixels),
                _ => None,
            })
        };
        let rows_max = region_of(&cheap).expect("a row decoder offers a region");
        let transient_max = region_of(&costly).expect("a transient decoder still fits one");
        assert!(
            transient_max < rows_max,
            "transient-full ({transient_max}) was not priced below row decoding ({rows_max})"
        );
    }

    /// When the transient whole-frame decode alone exceeds the budget, a region
    /// from that decoder is not offered, rather than being offered and then
    /// failing.
    #[test]
    fn a_transient_decoder_is_not_offered_when_the_whole_frame_cannot_fit() {
        // 2 GP at 4 bytes = 8 GB transient, on a machine budgeted at ~2.6 GiB.
        let probe = png(
            50_000,
            40_000,
            1_800_000_000,
            RegionDecode::TransientFull { bytes_per_pixel: 4 },
        );
        let mut probe = probe;
        probe.capabilities.reduced = ReducedDecode::None;
        let report = run(&probe, (8, 4));
        assert_eq!(
            report.verdict,
            Verdict::InsufficientResources {
                shortfall: Shortfall::Budget
            }
        );
        assert!(report.options.is_empty());
    }

    #[test]
    fn a_source_with_no_bounded_decode_at_all_is_not_offered_anything_false() {
        let mut probe = png(30_000, 30_000, 400_000_000, RegionDecode::None);
        probe.capabilities.reduced = ReducedDecode::None;
        let report = run(&probe, (8, 4));
        assert!(report.options.is_empty());
        assert!(matches!(
            report.verdict,
            Verdict::InsufficientResources { .. }
        ));
    }

    /// Case 4: hostile metadata is refused by arithmetic, with nothing allocated.
    #[test]
    fn hostile_dimensions_are_refused_without_overflow() {
        for (w, h) in [(u64::MAX, u64::MAX), (u64::MAX, 1), (1 << 40, 1 << 40)] {
            let report = run(&png(w, h, 1000, RegionDecode::Rows), (128, 100));
            assert!(
                matches!(
                    report.verdict,
                    Verdict::Unsafe {
                        refusal: Refusal::ImplausibleDimensions { .. }
                    }
                ),
                "{w}x{h} was not refused: {:?}",
                report.verdict
            );
            assert!(report.options.is_empty());
        }
        let report = run(&png(0, 100, 1000, RegionDecode::Rows), (16, 12));
        assert_eq!(
            report.verdict,
            Verdict::Unsafe {
                refusal: Refusal::Empty
            }
        );
    }

    /// A header that claims more raster than PNG's compression can hold is
    /// lying, and that is provable rather than guessed.
    #[test]
    fn a_png_that_claims_the_impossible_is_refused() {
        // 100 MP RGBA is 400 MB of raster in a 10 KB file: 40,000:1.
        let report = run(&png(10_000, 10_000, 10_000, RegionDecode::Rows), (128, 100));
        assert!(matches!(
            report.verdict,
            Verdict::Unsafe {
                refusal: Refusal::ImpossibleExpansion { .. }
            }
        ));
        // A genuinely compressible one passes: a flat 100 MP PNG is about 400 KB.
        let flat = run(
            &png(10_000, 10_000, 400_000, RegionDecode::Rows),
            (128, 100),
        );
        assert!(!matches!(flat.verdict, Verdict::Unsafe { .. }));
    }

    /// A fit by budget that does not fit the machine right now is reported as
    /// momentary, because closing something else would change the answer.
    #[test]
    fn a_shortfall_that_depends_on_free_memory_is_labelled_momentary() {
        let probe = png(8_000, 6_000, 20_000_000, RegionDecode::Rows);
        let (budget, limits) = budget_of(64, 60);
        let report = plan(&probe, &budget, &limits, Some(500 * 1024 * 1024), 0);
        assert_eq!(
            report.verdict,
            Verdict::InsufficientResources {
                shortfall: Shortfall::Momentary
            }
        );
    }

    /// Opening is allowed up to the per-image ceiling, a quarter of the budget in
    /// float bytes, and at that ceiling an open costs about 69% of the budget. So
    /// a source in the upper part of the allowed range must be warned about, and
    /// the figure must be the real percentage.
    #[test]
    fn a_source_that_dominates_the_budget_is_admitted_with_a_warning() {
        let (budget, limits) = budget_of(16, 12);
        // 125 MP: well inside the pixel ceiling, over half the budget to open.
        let probe = png(12_500, 10_000, 80_000_000, RegionDecode::Rows);
        let report = plan(&probe, &budget, &limits, Some(12 * GIB), 0);
        match report.verdict {
            Verdict::FullResolutionWithWarning {
                peak_percent_of_budget,
            } => {
                let expected =
                    (u128::from(report.full_peak_bytes) * 100 / u128::from(budget.bytes)) as u32;
                assert_eq!(peak_percent_of_budget, expected);
                assert!((50..=100).contains(&peak_percent_of_budget));
            }
            other => panic!("expected a warning, got {other:?}"),
        }
        // And a source well under half the budget is not warned about.
        let small = png(6_000, 4_000, 12_000_000, RegionDecode::Rows);
        let quiet = plan(&small, &budget, &limits, Some(12 * GIB), 0);
        assert_eq!(quiet.verdict, Verdict::FullResolution);
    }

    /// What the session already holds counts against a new addition.
    #[test]
    fn resident_memory_reduces_what_may_be_added() {
        let probe = png(6_000, 4_000, 12_000_000, RegionDecode::Rows);
        let (budget, limits) = budget_of(16, 12);
        let empty = plan(&probe, &budget, &limits, Some(12 * GIB), 0);
        let loaded = plan(
            &probe,
            &budget,
            &limits,
            Some(12 * GIB),
            budget.bytes - 100_000_000,
        );
        assert!(matches!(
            empty.verdict,
            Verdict::FullResolution | Verdict::FullResolutionWithWarning { .. }
        ));
        assert_ne!(loaded.verdict, Verdict::FullResolution);
    }

    /// The largest region reported must itself be admissible, and one pixel more
    /// must not be: the figure is a boundary, not an estimate.
    #[test]
    fn the_maximum_region_is_exactly_the_boundary() {
        let probe = png(30_000, 20_000, 200_000_000, RegionDecode::Rows);
        let (budget, limits) = budget_of(16, 12);
        let report = plan(&probe, &budget, &limits, Some(12 * GIB), 0);
        let (max_pixels, peak) = report
            .options
            .iter()
            .find_map(|o| match o {
                AdmissionOption::OpenRegion {
                    max_region_pixels,
                    peak_bytes_at_max,
                    ..
                } => Some((*max_region_pixels, *peak_bytes_at_max)),
                _ => None,
            })
            .expect("region option");
        assert!(
            peak <= budget.bytes,
            "the stated maximum exceeds the budget"
        );
        assert!(max_pixels <= limits.working_pixels());
        // Probe the model directly one step past the maximum.
        let native = 4u128;
        let over = u128::from(max_pixels) + 1;
        let overhead = u128::from(probe.width) * native + over * native;
        let peak_over = document_peak(over, native, u128::from(probe.file_bytes), overhead);
        let pixel_cap = u128::from(limits.working_pixels());
        assert!(
            peak_over > u128::from(budget.bytes)
                || over > pixel_cap
                || max_pixels as u128 == pixel_cap,
            "a larger region than the reported maximum would still have fit"
        );
    }

    /// The coefficients the interface evaluates must be exactly the model the
    /// planner uses, for every decoder. Otherwise dragging a rectangle could say
    /// "supported" for a region the planner would refuse.
    #[test]
    fn the_region_cost_coefficients_are_the_planners_own_model() {
        let kinds = [
            RegionDecode::Rows,
            RegionDecode::Segments {
                fixed_bytes: 1_000_000,
                bytes_per_pixel: 36,
            },
            RegionDecode::TransientFull { bytes_per_pixel: 3 },
        ];
        for region in kinds {
            let probe = png(20_000, 15_000, 120_000_000, region);
            let cost = region_cost(&probe).expect("this decoder produces a region");
            let native = u128::from(probe.native_bytes_per_pixel);
            for pixels in [1u128, 1_000, 4_000_000, 50_000_000, 300_000_000] {
                let overhead = region_decode_overhead_for_test(&probe, pixels);
                let model = document_peak(pixels, native, u128::from(probe.file_bytes), overhead);
                assert_eq!(cost.peak(pixels), model, "{region:?} at {pixels} pixels");
            }
        }
        assert!(region_cost(&png(10, 10, 100, RegionDecode::None)).is_none());
    }

    /// The decode overhead written out independently of `region_cost`, so the test
    /// above compares two derivations of the same model rather than one with itself.
    fn region_decode_overhead_for_test(probe: &SourceProbe, region_pixels: u128) -> u128 {
        let native = u128::from(probe.native_bytes_per_pixel);
        match probe.capabilities.region {
            RegionDecode::Rows => {
                u128::from(probe.width) * native
                    + DECODER_FIXED_BYTES
                    + OPEN_PREVIEW_BYTES
                    + region_pixels * native
            }
            RegionDecode::Segments {
                fixed_bytes,
                bytes_per_pixel,
            } => {
                u128::from(fixed_bytes)
                    + OPEN_PREVIEW_BYTES
                    + region_pixels * u128::from(bytes_per_pixel)
            }
            RegionDecode::TransientFull { bytes_per_pixel } => {
                probe.pixels() * u128::from(bytes_per_pixel)
                    + DECODER_FIXED_BYTES
                    + OPEN_PREVIEW_BYTES
            }
            RegionDecode::None => unreachable!(),
        }
    }

    #[test]
    fn a_reduced_copy_scale_actually_fits() {
        let probe = png(40_000, 30_000, 900_000_000, RegionDecode::None);
        let (budget, limits) = budget_of(8, 4);
        let report = plan(&probe, &budget, &limits, Some(4 * GIB), 0);
        let scale = report
            .options
            .iter()
            .find_map(|o| match o {
                AdmissionOption::OpenReduced { max_scale, .. } => Some(*max_scale),
                _ => None,
            })
            .expect("reduced option");
        assert!(scale > 0.0 && scale < 1.0);
        let reduced = (probe.pixels() as f64 * scale * scale) as u128;
        assert!(reduced <= u128::from(limits.working_pixels()));
        assert_eq!(report.verdict, Verdict::ReducedCopyRecommended);
    }
}
