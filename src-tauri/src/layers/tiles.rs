//! Tile geometry, operation locality, and halo rules.
//!
//! This module carries no pixels. It answers three questions the tiled renderer
//! asks before it allocates anything:
//!
//! * which rectangles make up a document,
//! * how far outside a rectangle an operation reads, and
//! * whether an operation can be evaluated on a rectangle at all.
//!
//! Every rectangle here is in *render space* — document pixels multiplied by
//! the render scale — because that is the space the compositor works in. All
//! arithmetic is checked: a tile index times a tile size is attacker-influenced
//! once a project file can choose the canvas size.

use crate::domain::EditOperation;
use crate::error::AppError;
use crate::image_processing::high_precision::{locality, OperationLocality};
use crate::layers::{Layer, LayerContent, LayerDocument};

/// Tile sizes the renderer will accept. A tile has to be large enough that
/// per-tile overhead is amortised and small enough that its intermediates stay
/// far below a full frame; the default is chosen by benchmark, not by taste.
pub const MIN_TILE_SIZE: u32 = 64;
pub const MAX_TILE_SIZE: u32 = 2048;

/// Default edge length of a square tile.
///
/// Measured on this repository's benchmark: 256 gave the best wall-clock across
/// 12, 24 and 60 MP documents, with 128 losing to per-tile overhead and 512
/// losing to larger halo re-computation on neighbourhood operations. See
/// `docs/tiled-rendering.md`.
pub const DEFAULT_TILE_SIZE: u32 = 256;

/// Largest halo, in render pixels, a single operation may request.
///
/// A halo is re-rendered for every tile that needs it, so an unbounded halo
/// would silently turn a tiled render back into a full-frame one — and worse,
/// into many overlapping full-frame ones.
pub const MAX_HALO: u32 = 64;

/// A rectangle of the render canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Region {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// The whole canvas.
    pub const fn whole(width: u32, height: u32) -> Self {
        Self::new(0, 0, width, height)
    }

    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// Exclusive right and bottom edges, widened to `u64` so a region at the
    /// far edge of a maximum-sized canvas cannot wrap.
    pub const fn right(&self) -> u64 {
        self.x as u64 + self.width as u64
    }

    pub const fn bottom(&self) -> u64 {
        self.y as u64 + self.height as u64
    }

    pub const fn pixel_count(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// Bytes one `FloatRgba` buffer of this size occupies.
    pub const fn float_bytes(&self) -> u64 {
        self.pixel_count() * 16
    }

    pub fn intersect(&self, other: &Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if u64::from(x) >= right || u64::from(y) >= bottom {
            return Self::new(x, y, 0, 0);
        }
        Self::new(
            x,
            y,
            (right - u64::from(x)) as u32,
            (bottom - u64::from(y)) as u32,
        )
    }

    pub fn contains(&self, other: &Self) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    /// Grows the region by `halo` on every side, then clips it to `bounds`.
    ///
    /// Growing before clipping is what keeps a tile at the canvas edge correct:
    /// it simply gets a smaller halo on the outside edges, rather than sampling
    /// pixels that do not exist.
    pub fn expanded(&self, halo: u32, bounds: &Self) -> Self {
        let x = self.x.saturating_sub(halo);
        let y = self.y.saturating_sub(halo);
        let right = self.right().saturating_add(u64::from(halo));
        let bottom = self.bottom().saturating_add(u64::from(halo));
        Self::new(
            x,
            y,
            (right - u64::from(x)).min(u32::MAX as u64) as u32,
            (bottom - u64::from(y)).min(u32::MAX as u64) as u32,
        )
        .intersect(bounds)
    }

    /// Where `inner` sits inside this region, as an origin offset.
    pub fn offset_of(&self, inner: &Self) -> Option<(u32, u32)> {
        if !self.contains(inner) {
            return None;
        }
        Some((inner.x - self.x, inner.y - self.y))
    }
}

/// Walks a canvas as a grid of tiles, including the partial ones at the right
/// and bottom edges.
#[derive(Debug, Clone, Copy)]
pub struct TileGrid {
    pub canvas: Region,
    pub tile_size: u32,
}

impl TileGrid {
    pub fn new(width: u32, height: u32, tile_size: u32) -> Result<Self, AppError> {
        if !(MIN_TILE_SIZE..=MAX_TILE_SIZE).contains(&tile_size) {
            return Err(AppError::InvalidLayerDocument(format!(
                "tile size must be between {MIN_TILE_SIZE} and {MAX_TILE_SIZE}"
            )));
        }
        if width == 0 || height == 0 {
            return Err(AppError::InvalidLayerDocument(
                "a tile grid needs a canvas with area".into(),
            ));
        }
        Ok(Self {
            canvas: Region::whole(width, height),
            tile_size,
        })
    }

    pub const fn columns(&self) -> u32 {
        self.canvas.width.div_ceil(self.tile_size)
    }

    pub const fn rows(&self) -> u32 {
        self.canvas.height.div_ceil(self.tile_size)
    }

    pub fn count(&self) -> u64 {
        u64::from(self.columns()) * u64::from(self.rows())
    }

    /// The tile at a grid coordinate, already clipped so edge tiles are partial
    /// rather than running past the canvas.
    pub fn tile(&self, column: u32, row: u32) -> Option<Region> {
        if column >= self.columns() || row >= self.rows() {
            return None;
        }
        // Checked because a hostile canvas size and a large index must not wrap
        // into a rectangle that overlaps a different part of the image.
        let x = u64::from(column).checked_mul(u64::from(self.tile_size))?;
        let y = u64::from(row).checked_mul(u64::from(self.tile_size))?;
        if x >= u64::from(self.canvas.width) || y >= u64::from(self.canvas.height) {
            return None;
        }
        let region =
            Region::new(x as u32, y as u32, self.tile_size, self.tile_size).intersect(&self.canvas);
        (!region.is_empty()).then_some(region)
    }

    pub fn tiles(&self) -> impl Iterator<Item = Region> + '_ {
        (0..self.rows())
            .flat_map(move |row| (0..self.columns()).filter_map(move |c| self.tile(c, row)))
    }

    /// Tiles ordered so the ones overlapping `focus` come first.
    ///
    /// Interactive rendering uses this so the part of the picture a user is
    /// looking at resolves before the rest of the canvas.
    pub fn tiles_prioritized(&self, focus: Option<Region>) -> Vec<Region> {
        let mut tiles: Vec<Region> = self.tiles().collect();
        if let Some(focus) = focus {
            tiles.sort_by_key(|tile| {
                // Stable: visible tiles keep their scan order among themselves.
                u8::from(tile.intersect(&focus).is_empty())
            });
        }
        tiles
    }
}

/// How an operation reads relative to the rectangle it is asked to produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileBehavior {
    /// Reads only the pixel it writes.
    TileLocal,
    /// Reads a bounded neighbourhood; the region must be grown by `radius`.
    HaloDependent { radius: u32 },
    /// Depends on the whole image and cannot be evaluated on a sub-rectangle.
    Global,
}

impl TileBehavior {
    pub const fn halo(&self) -> u32 {
        match self {
            Self::HaloDependent { radius } => *radius,
            _ => 0,
        }
    }

    pub const fn is_global(&self) -> bool {
        matches!(self, Self::Global)
    }
}

/// The neighbourhood an operation reads, in canvas pixels.
///
/// These radii are read off the implementations in
/// `image_processing::high_precision`, not guessed from parameter names. Two
/// of them are easy to get wrong:
///
/// * The blur family's `radius` parameter is a **sigma**; the kernel actually
///   reaches `ceil(sigma * 3)` pixels. A halo sized from the parameter alone
///   would be three times too small and would show as tile seams.
/// * `Deblock` filters on an **absolute 8-pixel grid anchored at the image
///   origin**, so its result depends on where the buffer starts. That cannot be
///   reproduced on a shifted sub-rectangle at all, so it is global rather than
///   halo-dependent.
///
/// No scale factor is applied: adjustments run on the render canvas, which is
/// already scaled, so a sigma of 3 means 3 canvas pixels at any render scale.
///
/// An operation whose halo would exceed `MAX_HALO` is reported as `Global`.
/// Clamping the halo instead would silently produce seams, and re-rendering a
/// border wider than a tile would cost more than rendering the frame once.
pub fn behavior(operation: &EditOperation, _scale: f64) -> TileBehavior {
    // The blur is separable but each pass reaches the same distance, so the
    // dependency is `kernel` pixels in x and in y, not their sum.
    let from_sigma = |sigma: f32| -> u64 {
        if !sigma.is_finite() || sigma <= 0.0 {
            return 0;
        }
        (f64::from(sigma) * 3.0).ceil() as u64
    };
    let radius: u64 = match operation {
        EditOperation::Masked { operation, .. } => return behavior(operation, _scale),
        // Origin-anchored, therefore not reproducible on a sub-rectangle.
        // Still global, and for the same reason as before: the 8-pixel grid is
        // anchored to the image origin, so a tile that does not start on a
        // multiple of eight would filter the wrong columns.
        EditOperation::Deblock { .. } => return TileBehavior::Global,
        EditOperation::GaussianBlur { radius } => from_sigma(*radius),
        // `sharpen` blurs with a fixed sigma of 1.2 before subtracting.
        EditOperation::Sharpen { .. } => from_sigma(1.2),
        EditOperation::EdgeAwareSharpen { radius, .. }
        | EditOperation::MildDeblur { radius, .. } => from_sigma(*radius),
        // Read off the implementation rather than restated here: `denoise`
        // filters chroma over a wider window than luma, so the chroma radius is
        // the one that sets the halo. A stale constant here is exactly how a
        // seam appears after an algorithm is improved.
        EditOperation::Denoise {
            strength, color, ..
        } => u64::from(
            crate::image_processing::high_precision::denoise_radius(*strength).max(
                crate::image_processing::high_precision::denoise_chroma_radius(*strength, *color),
            ),
        ),
        // Each Richardson-Lucy iteration convolves forward and back, so a
        // pixel's dependency grows by twice the kernel reach every time. This
        // reaches MAX_HALO quickly and is then declared global rather than
        // clamped, which is the only honest option: a clamped halo here would
        // seam in proportion to how much sharpening the user asked for.
        EditOperation::Deconvolve {
            kernel, iterations, ..
        } => u64::from(kernel.reach()) * 2 * u64::from(*iterations),
        // A plugin states its own reach, which its installer verified by running
        // the filter whole and in tiles and comparing.
        EditOperation::PluginFilter {
            locality: crate::plugins::manifest::Locality::Local { radius },
            ..
        } => u64::from(*radius),
        // `remove_defects` reads the eight surrounding pixels and nothing else.
        EditOperation::RemoveDefects { .. } => 1,
        // `local_luma` is a box filter of exactly this radius.
        EditOperation::LocalContrast { tile_size, .. } => u64::from((*tile_size / 2).max(1)),
        EditOperation::UnevenLightingCorrection { radius, .. } => {
            if radius.is_finite() && *radius > 0.0 {
                (*radius as u64).max(1)
            } else {
                1
            }
        }
        EditOperation::DecontaminateColors { radius, .. } => u64::from(*radius),
        _ => match locality(operation) {
            OperationLocality::TileLocal => 0,
            OperationLocality::Global => return TileBehavior::Global,
            // `locality` calls it halo-dependent but this function has no rule
            // for it: refuse to tile rather than invent a border width.
            OperationLocality::HaloDependent => return TileBehavior::Global,
        },
    };
    if matches!(locality(operation), OperationLocality::Global) {
        return TileBehavior::Global;
    }
    if radius == 0 {
        return TileBehavior::TileLocal;
    }
    // One extra pixel covers the bilinear tap either side of a sampled centre.
    let radius = radius + 1;
    if radius > u64::from(MAX_HALO) {
        return TileBehavior::Global;
    }
    TileBehavior::HaloDependent {
        radius: radius as u32,
    }
}

/// What tiling a whole document would require.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DocumentTiling {
    /// Largest end-to-end dependency radius in the render graph.
    ///
    /// Sequential neighbourhood adjustments accumulate. Isolated sibling
    /// branches take the maximum of their radii instead, because neither reads
    /// the other's intermediate.
    pub halo: u32,
    /// True when some operation cannot be evaluated on a sub-rectangle.
    pub has_global: bool,
}

impl DocumentTiling {
    pub const fn is_tileable(&self) -> bool {
        !self.has_global
    }
}

/// Inspects a layer tree for the halo and global operations it contains.
pub fn document_tiling(document: &LayerDocument, scale: f64) -> DocumentTiling {
    let dependency = inspect_stack(&document.layers, scale);
    let halo = dependency.through.max(dependency.independent);
    DocumentTiling {
        halo,
        has_global: dependency.has_global || halo > MAX_HALO,
    }
}

/// A stack transforms an incoming backdrop dependency `h` into
/// `max(h + through, independent)`. Keeping both terms is what distinguishes
/// sequential pass-through work (which accumulates) from independent isolated
/// branches (which only take a maximum).
#[derive(Debug, Clone, Copy, Default)]
struct StackDependency {
    through: u32,
    independent: u32,
    has_global: bool,
}

fn inspect_stack(layers: &[Layer], scale: f64) -> StackDependency {
    let mut result = StackDependency::default();
    for layer in layers {
        // A hidden or fully transparent layer contributes nothing and is not
        // evaluated, so its halo must not force every tile to grow.
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        match &layer.content {
            LayerContent::Adjustment { operation } => {
                let behavior = behavior(operation, scale);
                let radius = behavior.halo();
                result.through = result.through.saturating_add(radius);
                result.independent = result.independent.saturating_add(radius);
                result.has_global |= behavior.is_global();
            }
            LayerContent::Group { children, isolated } => {
                let child = inspect_stack(children, scale);
                result.has_global |= child.has_global;
                if *isolated {
                    let child_halo = child.through.max(child.independent);
                    result.independent = result.independent.max(child_halo);
                } else {
                    // A pass-through group starts from the current backdrop,
                    // so its through dependency composes with the parent stack.
                    result.independent = result
                        .independent
                        .saturating_add(child.through)
                        .max(child.independent);
                    result.through = result.through.saturating_add(child.through);
                }
            }
            // Shapes and text are rasterised directly into the rectangle
            // being rendered and read no neighbouring pixels, so they need no
            // halo — the same as a pixel layer.
            LayerContent::Shape { .. }
            | LayerContent::Text { .. }
            | LayerContent::Pixel { .. }
            | LayerContent::SmartObject { .. } => {}
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{DevelopmentParameters, WhiteBalance};

    #[test]
    fn a_grid_covers_every_pixel_exactly_once() {
        // A canvas that is not a multiple of the tile size, so the right and
        // bottom edges are partial.
        let grid = TileGrid::new(700, 300, 256).unwrap();
        assert_eq!((grid.columns(), grid.rows()), (3, 2));
        assert_eq!(grid.count(), 6);

        let mut covered = vec![0u8; 700 * 300];
        for tile in grid.tiles() {
            for y in tile.y..tile.y + tile.height {
                for x in tile.x..tile.x + tile.width {
                    covered[(y as usize) * 700 + x as usize] += 1;
                }
            }
        }
        assert!(
            covered.iter().all(|count| *count == 1),
            "tiles overlapped or left a gap"
        );
    }

    #[test]
    fn edge_tiles_are_partial_rather_than_running_past_the_canvas() {
        let grid = TileGrid::new(700, 300, 256).unwrap();
        let last = grid.tile(2, 1).unwrap();
        assert_eq!(last, Region::new(512, 256, 188, 44));
        assert_eq!(last.right(), 700);
        assert_eq!(last.bottom(), 300);
        assert!(grid.tile(3, 0).is_none());
        assert!(grid.tile(0, 2).is_none());
    }

    #[test]
    fn a_single_pixel_canvas_produces_one_single_pixel_tile() {
        let grid = TileGrid::new(1, 1, 256).unwrap();
        let tiles: Vec<_> = grid.tiles().collect();
        assert_eq!(tiles, vec![Region::new(0, 0, 1, 1)]);
    }

    #[test]
    fn a_one_pixel_wide_canvas_still_tiles_down_its_height() {
        let grid = TileGrid::new(1, 600, 256).unwrap();
        assert_eq!((grid.columns(), grid.rows()), (1, 3));
        let tiles: Vec<_> = grid.tiles().collect();
        assert_eq!(tiles.len(), 3);
        assert_eq!(tiles[2], Region::new(0, 512, 1, 88));
    }

    #[test]
    fn a_grid_refuses_an_unreasonable_tile_size_or_an_empty_canvas() {
        assert!(TileGrid::new(100, 100, 0).is_err());
        assert!(TileGrid::new(100, 100, 32).is_err());
        assert!(TileGrid::new(100, 100, 4096).is_err());
        assert!(TileGrid::new(0, 100, 256).is_err());
        assert!(TileGrid::new(100, 0, 256).is_err());
    }

    /// A maximum-sized canvas with a far-edge tile index must not wrap into a
    /// rectangle describing a different part of the image.
    #[test]
    fn tile_indexing_is_checked_at_extreme_coordinates() {
        let grid = TileGrid::new(u32::MAX, 4, MAX_TILE_SIZE).unwrap();
        let last_column = grid.columns() - 1;
        let tile = grid.tile(last_column, 0).expect("the last column exists");
        assert!(tile.right() <= u64::from(u32::MAX));
        assert!(tile.x <= tile.right() as u32);
        assert!(grid.tile(u32::MAX, 0).is_none());
        // The count is computed in u64, so an enormous grid does not overflow.
        assert!(grid.count() > 0);
    }

    #[test]
    fn regions_intersect_and_report_emptiness() {
        let a = Region::new(10, 10, 100, 100);
        assert_eq!(
            a.intersect(&Region::new(50, 50, 100, 100)),
            Region::new(50, 50, 60, 60)
        );
        assert!(a.intersect(&Region::new(500, 500, 10, 10)).is_empty());
        assert!(a.intersect(&Region::new(0, 0, 5, 5)).is_empty());
        assert_eq!(a.intersect(&a), a);
    }

    #[test]
    fn expansion_adds_a_halo_and_clips_it_to_the_canvas() {
        let canvas = Region::whole(1000, 1000);
        let middle = Region::new(500, 500, 100, 100);
        assert_eq!(middle.expanded(8, &canvas), Region::new(492, 492, 116, 116));

        // A tile at the origin gets a halo only on the inside edges.
        let corner = Region::new(0, 0, 100, 100);
        assert_eq!(corner.expanded(8, &canvas), Region::new(0, 0, 108, 108));

        // And one at the far corner is clipped rather than extended past it.
        let far = Region::new(900, 900, 100, 100);
        assert_eq!(far.expanded(8, &canvas), Region::new(892, 892, 108, 108));
    }

    #[test]
    fn expansion_cannot_overflow_at_the_edge_of_the_coordinate_space() {
        let canvas = Region::whole(u32::MAX, 8);
        let edge = Region::new(u32::MAX - 4, 0, 4, 8);
        let grown = edge.expanded(MAX_HALO, &canvas);
        assert!(grown.right() <= u64::from(u32::MAX));
        assert!(!grown.is_empty());
    }

    #[test]
    fn a_region_reports_where_a_tile_sits_inside_it() {
        let expanded = Region::new(492, 492, 116, 116);
        let tile = Region::new(500, 500, 100, 100);
        assert_eq!(expanded.offset_of(&tile), Some((8, 8)));
        // A tile that is not inside has no offset rather than a wrapped one.
        assert_eq!(expanded.offset_of(&Region::new(0, 0, 10, 10)), None);
    }

    #[test]
    fn tile_local_operations_ask_for_no_halo() {
        for operation in [
            EditOperation::Brightness { amount: 0.2 },
            EditOperation::Contrast { amount: 0.1 },
            EditOperation::Grayscale,
            EditOperation::Rotate { degrees: 90 },
        ] {
            assert_eq!(
                behavior(&operation, 1.0),
                TileBehavior::TileLocal,
                "{operation:?}"
            );
        }
    }

    #[test]
    fn neighbourhood_operations_ask_for_a_halo_from_their_own_radius() {
        let small = behavior(&EditOperation::GaussianBlur { radius: 2.0 }, 1.0);
        let large = behavior(&EditOperation::GaussianBlur { radius: 12.0 }, 1.0);
        assert_eq!(small, TileBehavior::HaloDependent { radius: 7 });
        assert_eq!(large, TileBehavior::HaloDependent { radius: 37 });
        // A bigger blur must ask for more, or the seam test would pass by luck.
        assert!(large.halo() > small.halo());
    }

    /// The blur parameter is a sigma and the kernel reaches three sigma, so the
    /// halo must follow the kernel rather than the parameter. A halo of four
    /// for a sigma of three would leave a five-pixel seam on every tile edge.
    #[test]
    fn a_blur_halo_follows_the_kernel_rather_than_the_sigma() {
        assert_eq!(
            behavior(&EditOperation::GaussianBlur { radius: 3.0 }, 1.0),
            TileBehavior::HaloDependent { radius: 10 }
        );
        assert_eq!(
            behavior(&EditOperation::Sharpen { strength: 0.5 }, 1.0),
            TileBehavior::HaloDependent { radius: 5 }
        );
    }

    /// An operation that would need a border wider than the cap is refused
    /// rather than clamped, because a clamped halo produces seams silently.
    #[test]
    fn an_operation_needing_more_than_the_cap_is_reported_global() {
        let wide = behavior(
            &EditOperation::LocalContrast {
                strength: 0.5,
                tile_size: 512,
                clip_limit: 2.0,
            },
            1.0,
        );
        assert!(wide.is_global(), "a 256-pixel box filter was tiled anyway");
        // A narrow one is still tileable.
        let narrow = behavior(
            &EditOperation::LocalContrast {
                strength: 0.5,
                tile_size: 32,
                clip_limit: 2.0,
            },
            1.0,
        );
        assert_eq!(narrow, TileBehavior::HaloDependent { radius: 17 });
    }

    /// Deblock filters on an eight-pixel grid measured from the image origin,
    /// so a shifted sub-rectangle would filter different pixels.
    #[test]
    fn an_origin_anchored_operation_is_global_rather_than_haloed() {
        assert!(behavior(&EditOperation::Deblock { strength: 0.5 }, 1.0).is_global());
    }

    #[test]
    fn a_zero_radius_blur_is_treated_as_tile_local() {
        assert_eq!(
            behavior(&EditOperation::GaussianBlur { radius: 0.0 }, 1.0),
            TileBehavior::TileLocal
        );
    }

    #[test]
    fn statistical_operations_are_global_and_say_so() {
        assert!(behavior(&EditOperation::AutoWhiteBalance { strength: 0.5 }, 1.0).is_global());
        assert!(behavior(
            &EditOperation::RawDevelopment {
                parameters: DevelopmentParameters {
                    white_balance: WhiteBalance::Auto,
                    ..Default::default()
                }
            },
            1.0
        )
        .is_global());
        // The same operation with explicit multipliers is not global.
        assert!(!behavior(
            &EditOperation::RawDevelopment {
                parameters: DevelopmentParameters::default()
            },
            1.0
        )
        .is_global());
    }

    #[test]
    fn a_masked_operation_inherits_the_behaviour_of_what_it_wraps() {
        let mask =
            crate::mask::MaskSnapshot::encode(&crate::mask::MaskBitmap::full(4, 4).expect("mask"));
        let masked = EditOperation::Masked {
            operation: Box::new(EditOperation::GaussianBlur { radius: 4.0 }),
            mask,
            invert: false,
            mask_id: None,
        };
        assert_eq!(behavior(&masked, 1.0).halo(), 13);
    }

    fn adjustment(id: &str, operation: EditOperation) -> Layer {
        let mut layer = crate::layers::test_pixel_layer(id, "unused", 256, 256);
        layer.content = LayerContent::Adjustment {
            operation: Box::new(operation),
        };
        layer
    }

    #[test]
    fn sequential_neighbourhood_adjustments_accumulate_their_halos() {
        let mut document = LayerDocument::new(256, 256);
        document.layers = vec![
            crate::layers::test_pixel_layer("base", "px1", 256, 256),
            adjustment("blur-1", EditOperation::GaussianBlur { radius: 2.0 }),
            adjustment("blur-2", EditOperation::GaussianBlur { radius: 2.0 }),
        ];
        // Each operation needs seven pixels. Taking only the maximum would
        // leave the second blur reading a first-blur border computed without
        // all of its own source pixels.
        assert_eq!(document_tiling(&document, 1.0).halo, 14);
    }

    #[test]
    fn isolated_sibling_branches_take_the_maximum_not_the_sum() {
        let isolated = |id: &str, pixel_id: &str| {
            let mut group = crate::layers::test_pixel_layer(id, "unused", 256, 256);
            group.content = LayerContent::Group {
                children: vec![
                    crate::layers::test_pixel_layer(&format!("{id}-base"), pixel_id, 256, 256),
                    adjustment(
                        &format!("{id}-blur"),
                        EditOperation::GaussianBlur { radius: 2.0 },
                    ),
                ],
                isolated: true,
            };
            group
        };
        let mut document = LayerDocument::new(256, 256);
        document.layers = vec![isolated("left", "px1"), isolated("right", "px2")];
        assert_eq!(document_tiling(&document, 1.0).halo, 7);
    }
}
