//! Region-addressable high-precision renderer.
//!
//! This produces the same picture as `layers::linear`, but for an arbitrary
//! rectangle of the canvas rather than the whole of it. That single change is
//! what makes the renderer memory-bounded: an isolated group, a pass-through
//! backdrop and an adjustment intermediate are all allocated at the size of the
//! rectangle being produced, not at the size of the document.
//!
//! `layers::linear` is deliberately left untouched. It remains the correctness
//! oracle: the equivalence tests below render the same document both ways and
//! require the results to agree, which is what catches an origin, clipping or
//! halo mistake in this file.
//!
//! Coordinates: a *document* coordinate is a pixel of the full render canvas
//! (the document scaled by `options.scale`). A *local* coordinate is an index
//! into the rectangle being produced. Every sampling decision — transforms,
//! masks, adjustment placement — is made in document coordinates, so a tile
//! cannot depend on where its own rectangle happens to start.

use std::sync::Arc;

use super::cache::{DocumentFingerprint, TileCache};
use super::composite::{
    decoded_mask, mask_coverage, mask_inverted, render_transform, MAX_RENDER_THREADS,
};
use super::linear::{blend_linear, source_over};
use super::tiles::{document_tiling, DocumentTiling, Region, TileGrid, DEFAULT_TILE_SIZE};
use super::{
    Layer, LayerContent, LayerDocument, LayerInterpolation, PixelSource, RenderOptions,
    MAX_GROUP_DEPTH,
};
use crate::color::{FloatImage, FloatRgba};
use crate::error::AppError;
use crate::image_processing::high_precision;

/// How a tiled render was actually carried out, for benchmarks and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TiledStats {
    pub tiles: u64,
    /// Tiles whose rectangle had to be grown for a neighbourhood operation.
    pub haloed_tiles: u64,
    pub halo: u32,
    /// True when the document contained an operation that cannot be tiled and
    /// the whole frame was rendered instead.
    pub fell_back_to_full_frame: bool,
    /// Largest single intermediate this render allocated, in bytes.
    pub peak_region_bytes: u64,
    /// Worker threads the scheduler actually used.
    pub workers: u32,
    /// Tiles this render took from the cache instead of computing.
    pub cached_tiles: u64,
}

struct Context<'a> {
    source: &'a dyn PixelSource,
    options: RenderOptions<'a>,
    /// The full render canvas. Transforms on groups and adjustment layers are
    /// defined against this, never against the rectangle being produced.
    canvas_width: u32,
    canvas_height: u32,
    region: Region,
}

impl Context<'_> {
    /// Turns a local index into the document coordinate it represents.
    #[inline]
    const fn document_x(&self, x: u32) -> u32 {
        self.region.x + x
    }

    #[inline]
    const fn document_y(&self, y: u32) -> u32 {
        self.region.y + y
    }
}

pub(super) fn render_dimensions(document: &LayerDocument, scale: f64) -> (u32, u32) {
    (
        ((f64::from(document.canvas_width) * scale).round() as u32).max(1),
        ((f64::from(document.canvas_height) * scale).round() as u32).max(1),
    )
}

fn validate_scale(scale: f64) -> Result<(), AppError> {
    if !scale.is_finite() || scale <= 0.0 || scale > 1.0 {
        return Err(AppError::InvalidLayerDocument(
            "render scale must be greater than zero and no larger than one".into(),
        ));
    }
    Ok(())
}

/// Renders one rectangle of the canvas at the region's own size.
pub fn render_region(
    document: &LayerDocument,
    source: &dyn PixelSource,
    options: RenderOptions<'_>,
    region: Region,
) -> Result<FloatImage, AppError> {
    document.validate()?;
    validate_scale(options.scale)?;
    let (canvas_width, canvas_height) = render_dimensions(document, options.scale);
    let clipped = region.intersect(&Region::whole(canvas_width, canvas_height));
    if clipped != region || region.is_empty() {
        return Err(AppError::InvalidLayerDocument(
            "a render region must be a non-empty rectangle inside the canvas".into(),
        ));
    }
    let context = Context {
        source,
        options,
        canvas_width,
        canvas_height,
        region,
    };
    let mut canvas = FloatImage::blank(region.width, region.height, FloatRgba::TRANSPARENT)?;
    composite_onto(&mut canvas, &document.layers, &context, 1)?;
    canvas.validate()?;
    Ok(canvas)
}

/// Renders a whole document by composing tiles.
///
/// The output is identical to the full-frame renderer within the tolerance the
/// equivalence tests document, but no intermediate larger than one expanded
/// tile is ever allocated — unless the document contains an operation that
/// cannot be evaluated on a sub-rectangle, in which case this says so in the
/// returned statistics and renders the frame whole.
pub fn render_document_tiled(
    document: &LayerDocument,
    source: &dyn PixelSource,
    options: RenderOptions<'_>,
    tile_size: u32,
) -> Result<(FloatImage, TiledStats), AppError> {
    render_document_tiled_with_threads(document, source, options, tile_size, 0)
}

/// Renders a whole document by composing tiles, with an explicit thread bound.
///
/// `max_threads` of 0 means "choose from the machine". Any other value is
/// clamped to [`MAX_RENDER_THREADS`]. The result does not depend on it: work is
/// split so that no two threads write the same output pixel and no thread reads
/// another's output, so every schedule produces the same bytes. A test asserts
/// that rather than leaving it as an intention.
pub fn render_document_tiled_with_threads(
    document: &LayerDocument,
    source: &dyn PixelSource,
    options: RenderOptions<'_>,
    tile_size: u32,
    max_threads: usize,
) -> Result<(FloatImage, TiledStats), AppError> {
    render_document_tiled_cached(document, source, options, tile_size, max_threads, None)
}

/// Renders a whole document by composing tiles, reusing any that are unchanged.
///
/// A cache can only make this faster, never different: a tile is reused only
/// when every layer that can reach it is byte-for-byte the same, and the tests
/// check that against uncached renders of randomly mutated documents rather
/// than trusting the key derivation.
pub fn render_document_tiled_cached(
    document: &LayerDocument,
    source: &dyn PixelSource,
    options: RenderOptions<'_>,
    tile_size: u32,
    max_threads: usize,
    cache: Option<&TileCache>,
) -> Result<(FloatImage, TiledStats), AppError> {
    let Some((plan, mut stats)) = Plan::new(document, source, options, tile_size, cache)? else {
        // A statistical operation reads the whole image by definition. Saying
        // so and rendering the frame whole is honest; quietly producing tiles
        // that each normalised themselves would be wrong.
        return Ok((
            super::linear::render_document_float(document, source, options)?,
            full_frame_stats(document, options),
        ));
    };
    let width = plan.canvas.width;
    let mut output = FloatImage::blank(width, plan.canvas.height, FloatRgba::TRANSPARENT)?;
    let source = &MemoSource::new(source);
    let workers = worker_count(plan.grid.rows() as usize, max_threads);
    stats.workers = workers as u32;

    // Work is split by whole tile rows so each worker owns a contiguous slice
    // of the output and no two ever touch the same pixel. That is what makes
    // the parallel result identical to the sequential one rather than merely
    // usually identical.
    let band_pixels = tile_size as usize * width as usize;
    if workers <= 1 {
        for (index, band) in output.pixels_mut().chunks_mut(band_pixels).enumerate() {
            plan.render_band(source, index as u32, band)?;
        }
    } else {
        // A fixed pool of `workers` threads pulling bands from a shared queue,
        // not a thread per band: a 60 MP document has around thirty tile rows,
        // and thirty OS threads competing for eight cores is slower than eight
        // threads doing the same work. Pulling rather than partitioning up
        // front also keeps the pool busy when tiles differ in cost, which they
        // do whenever layers do not cover the whole canvas.
        let failure = std::sync::Mutex::new(None::<AppError>);
        let queue = std::sync::Mutex::new(
            output
                .pixels_mut()
                .chunks_mut(band_pixels)
                .enumerate()
                .collect::<Vec<_>>()
                .into_iter(),
        );
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let (queue, failure, plan) = (&queue, &failure, &plan);
                scope.spawn(move || loop {
                    // Stop pulling once any band has failed or the render was
                    // cancelled; the remaining work is discarded.
                    if lock(failure).is_some() {
                        break;
                    }
                    let Some((index, band)) = lock(queue).next() else {
                        break;
                    };
                    // Suppressed inside the pool, so a large tile cannot fan
                    // out again and multiply the thread count.
                    let outcome = high_precision::without_nested_parallelism(|| {
                        plan.render_band(source, index as u32, band)
                    });
                    if let Err(error) = outcome {
                        let mut slot = lock(failure);
                        if slot.is_none() {
                            *slot = Some(error);
                        }
                        break;
                    }
                });
            }
        });
        if let Some(error) = failure.into_inner().unwrap_or_else(|e| e.into_inner()) {
            return Err(error);
        }
    }
    high_precision::check_cancel(options.cancel)?;
    output.validate()?;
    stats.cached_tiles = plan.hits.load(std::sync::atomic::Ordering::Relaxed);
    Ok((output, stats))
}

/// Renders a document one horizontal band at a time, never holding the frame.
///
/// Each band is passed to `emit` in top-to-bottom order and dropped straight
/// afterwards, so peak memory is one band plus the tiles in flight rather than
/// the whole output. That is what lets an export be larger than the memory
/// available to hold it.
///
/// An untileable document is still rendered whole — there is nothing else it
/// could honestly do — emitted as a single band, and reported as a fallback.
pub fn render_document_streaming(
    document: &LayerDocument,
    source: &dyn PixelSource,
    options: RenderOptions<'_>,
    tile_size: u32,
    max_threads: usize,
    emit: &mut dyn FnMut(u32, &FloatImage) -> Result<(), AppError>,
) -> Result<TiledStats, AppError> {
    render_document_streaming_cached(
        document,
        source,
        options,
        tile_size,
        max_threads,
        None,
        emit,
    )
}

/// Streams a document band by band, reusing cached tiles where it can.
#[allow(clippy::too_many_arguments)]
pub fn render_document_streaming_cached(
    document: &LayerDocument,
    source: &dyn PixelSource,
    options: RenderOptions<'_>,
    tile_size: u32,
    max_threads: usize,
    cache: Option<&TileCache>,
    emit: &mut dyn FnMut(u32, &FloatImage) -> Result<(), AppError>,
) -> Result<TiledStats, AppError> {
    let Some((plan, mut stats)) = Plan::new(document, source, options, tile_size, cache)? else {
        let image = super::linear::render_document_float(document, source, options)?;
        emit(0, &image)?;
        return Ok(full_frame_stats(document, options));
    };
    let source = &MemoSource::new(source);
    // Bands must be emitted in order, so the parallelism has to come from
    // inside a band rather than from running bands concurrently.
    let workers = worker_count(plan.grid.columns() as usize, max_threads);
    stats.workers = workers as u32;
    let width = plan.canvas.width;
    for row in 0..plan.grid.rows() {
        let rows = plan.grid.tile(0, row).map_or(0, |tile| tile.height);
        if rows == 0 {
            continue;
        }
        let mut band = FloatImage::blank(width, rows, FloatRgba::TRANSPARENT)?;
        plan.render_band_parallel(source, row, band.pixels_mut(), workers)?;
        band.validate()?;
        emit(row * plan.grid.tile_size, &band)?;
    }
    high_precision::check_cancel(options.cancel)?;
    stats.cached_tiles = plan.hits.load(std::sync::atomic::Ordering::Relaxed);
    Ok(stats)
}

/// What the statistics say when a document could not be tiled at all.
fn full_frame_stats(document: &LayerDocument, options: RenderOptions<'_>) -> TiledStats {
    let (width, height) = render_dimensions(document, options.scale);
    TiledStats {
        halo: document_tiling(document, options.scale).halo,
        tiles: 1,
        haloed_tiles: 0,
        fell_back_to_full_frame: true,
        peak_region_bytes: Region::whole(width, height).float_bytes(),
        workers: 1,
        cached_tiles: 0,
    }
}

/// Locks a mutex, accepting a poisoned one rather than panicking a worker.
fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

/// Everything a tiled render needs that does not depend on where pixels go.
///
/// Shared by the full-frame and streaming entry points so both decompose a
/// document identically; only the destination differs.
struct Plan<'a> {
    document: &'a LayerDocument,
    options: RenderOptions<'a>,
    grid: TileGrid,
    tiling: DocumentTiling,
    canvas: Region,
    /// Present only when this render may reuse tiles. The fingerprint is built
    /// once here rather than per tile, because hashing the layer tree is the
    /// expensive half and the tile coordinates are the cheap half.
    cache: Option<(&'a TileCache, DocumentFingerprint)>,
    /// Tiles served from the cache, counted across workers.
    hits: std::sync::atomic::AtomicU64,
}

impl<'a> Plan<'a> {
    /// Returns `None` when the document cannot be tiled at all.
    fn new(
        document: &'a LayerDocument,
        source: &dyn PixelSource,
        options: RenderOptions<'a>,
        tile_size: u32,
        cache: Option<&'a TileCache>,
    ) -> Result<Option<(Self, TiledStats)>, AppError> {
        document.validate()?;
        validate_scale(options.scale)?;
        let (width, height) = render_dimensions(document, options.scale);
        let tiling = document_tiling(document, options.scale);
        if !tiling.is_tileable() {
            return Ok(None);
        }
        let canvas = Region::whole(width, height);
        let grid = TileGrid::new(width, height, tile_size)?;
        let stats = TiledStats {
            halo: tiling.halo,
            tiles: grid.count(),
            haloed_tiles: grid
                .tiles()
                .filter(|tile| tile.expanded(tiling.halo, &canvas) != *tile)
                .count() as u64,
            peak_region_bytes: grid
                .tiles()
                .map(|tile| tile.expanded(tiling.halo, &canvas).float_bytes())
                .max()
                .unwrap_or(0),
            fell_back_to_full_frame: false,
            workers: 1,
            cached_tiles: 0,
        };
        let cache = cache
            .map(|cache| {
                DocumentFingerprint::new(document, source, options.scale, tile_size, tiling.halo)
                    .map(|fingerprint| (cache, fingerprint))
            })
            .transpose()?;
        Ok(Some((
            Self {
                document,
                options,
                grid,
                tiling,
                canvas,
                cache,
                hits: std::sync::atomic::AtomicU64::new(0),
            },
            stats,
        )))
    }

    /// Renders one tile, returning it with the rectangle it actually covers.
    fn render_tile(
        &self,
        source: &dyn PixelSource,
        column: u32,
        row: u32,
    ) -> Result<Option<(FloatImage, Region, Region)>, AppError> {
        let Some(tile) = self.grid.tile(column, row) else {
            return Ok(None);
        };
        high_precision::check_cancel(self.options.cancel)?;
        let key = self
            .cache
            .as_ref()
            .map(|(_, fingerprint)| fingerprint.tile_key(&tile));
        if let (Some((cache, _)), Some(key)) = (self.cache.as_ref(), key) {
            if let Some(hit) = cache.get(&key) {
                self.hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                // A cached entry is the tile itself, so it needs no cropping.
                return Ok(Some(((*hit).clone(), tile, tile)));
            }
        }
        let expanded = tile.expanded(self.tiling.halo, &self.canvas);
        let rendered = render_region(
            self.document,
            source,
            RenderOptions {
                scale: self.options.scale,
                cancel: self.options.cancel,
            },
            expanded,
        )?;
        let Some(key) = key else {
            return Ok(Some((rendered, expanded, tile)));
        };
        // Only the tile is cached, never its halo: the halo exists to feed this
        // tile's pixels and is worthless to anyone else.
        let cropped = crop(&rendered, &expanded, &tile)?;
        if let Some((cache, _)) = self.cache.as_ref() {
            cache.insert(key, Arc::new(cropped.clone()));
        }
        Ok(Some((cropped, tile, tile)))
    }

    /// Renders every tile of one tile row into a contiguous band.
    fn render_band(
        &self,
        source: &dyn PixelSource,
        row: u32,
        band: &mut [FloatRgba],
    ) -> Result<(), AppError> {
        let first_row = row * self.grid.tile_size;
        for column in 0..self.grid.columns() {
            let Some((rendered, expanded, tile)) = self.render_tile(source, column, row)? else {
                continue;
            };
            blit_band(
                band,
                self.canvas.width,
                first_row,
                &rendered,
                &expanded,
                &tile,
            )?;
        }
        Ok(())
    }

    /// Renders one tile row with several tiles in flight at once.
    ///
    /// The streaming path needs bands in order, so it cannot use whole bands as
    /// its unit of parallelism the way the full-frame path does. Each worker
    /// renders a tile into its own buffer and then copies it into the shared
    /// band, which is a memcpy beside the cost of rendering it.
    fn render_band_parallel(
        &self,
        source: &dyn PixelSource,
        row: u32,
        band: &mut [FloatRgba],
        workers: usize,
    ) -> Result<(), AppError> {
        if workers <= 1 {
            return self.render_band(source, row, band);
        }
        let first_row = row * self.grid.tile_size;
        let failure = std::sync::Mutex::new(None::<AppError>);
        let columns = std::sync::Mutex::new(0..self.grid.columns());
        let band = std::sync::Mutex::new(band);
        std::thread::scope(|scope| {
            for _ in 0..workers {
                let (columns, failure, band) = (&columns, &failure, &band);
                scope.spawn(move || loop {
                    if lock(failure).is_some() {
                        break;
                    }
                    let Some(column) = lock(columns).next() else {
                        break;
                    };
                    let outcome = high_precision::without_nested_parallelism(|| {
                        let Some((rendered, expanded, tile)) =
                            self.render_tile(source, column, row)?
                        else {
                            return Ok(());
                        };
                        blit_band(
                            &mut lock(band),
                            self.canvas.width,
                            first_row,
                            &rendered,
                            &expanded,
                            &tile,
                        )
                    });
                    if let Err(error) = outcome {
                        let mut slot = lock(failure);
                        if slot.is_none() {
                            *slot = Some(error);
                        }
                        break;
                    }
                });
            }
        });
        failure
            .into_inner()
            .unwrap_or_else(|error| error.into_inner())
            .map_or(Ok(()), Err)
    }
}

/// Resolves each source once per render instead of once per tile.
///
/// `PixelBuffer::linear` converts an encoded 8-bit source to float on every
/// call. Full-frame rendering pays that once per layer; a tiled render would
/// otherwise pay it once per layer *per tile*, which on a 60 MP document is
/// hundreds of full-frame conversions. The memo makes tiling no worse than the
/// renderer it is replacing, and returns the identical buffer either way.
struct MemoSource<'a> {
    inner: &'a dyn PixelSource,
    linear: std::sync::Mutex<std::collections::HashMap<String, Arc<FloatImage>>>,
}

impl<'a> MemoSource<'a> {
    fn new(inner: &'a dyn PixelSource) -> Self {
        Self {
            inner,
            linear: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

impl PixelSource for MemoSource<'_> {
    fn cache_fingerprint(&self, pixel_id: &str) -> Option<[u8; 32]> {
        self.inner.cache_fingerprint(pixel_id)
    }

    fn dimensions(&self, pixel_id: &str) -> Option<(u32, u32)> {
        self.inner.dimensions(pixel_id)
    }

    fn resolve(&self, pixel_id: &str) -> Result<Arc<image::RgbaImage>, AppError> {
        self.inner.resolve(pixel_id)
    }

    fn resolve_linear(&self, pixel_id: &str) -> Result<Arc<FloatImage>, AppError> {
        if let Some(cached) = self
            .linear
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(pixel_id)
        {
            return Ok(Arc::clone(cached));
        }
        // Converted outside the lock: two threads racing on the same source
        // convert it twice and agree on the result, which is cheaper than
        // making every other thread wait for one conversion.
        let image = self.inner.resolve_linear(pixel_id)?;
        self.linear
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(pixel_id.to_string(), Arc::clone(&image));
        Ok(image)
    }

    fn resident_bytes(&self) -> u64 {
        self.inner.resident_bytes()
    }

    fn promotion_bytes(&self, document: &LayerDocument) -> u64 {
        self.inner.promotion_bytes(document)
    }
}

/// Threads a tiled render may use.
///
/// Bounded so an interactive render, a batch job and an export cannot each
/// claim every core at once; see `docs/tiled-rendering.md`.
fn worker_count(bands: usize, requested: usize) -> usize {
    let available = if requested == 0 {
        std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1)
    } else {
        requested
    };
    available.clamp(1, MAX_RENDER_THREADS).min(bands.max(1))
}

/// Extracts the `tile` part of a rendered `source_region` as its own image.
fn crop(
    rendered: &FloatImage,
    source_region: &Region,
    tile: &Region,
) -> Result<FloatImage, AppError> {
    let (offset_x, offset_y) = source_region.offset_of(tile).ok_or_else(|| {
        AppError::InvalidLayerDocument("a tile fell outside the rectangle rendered for it".into())
    })?;
    let mut cropped = FloatImage::blank(tile.width, tile.height, FloatRgba::TRANSPARENT)?;
    let rendered_width = rendered.width() as usize;
    let width = tile.width as usize;
    for row in 0..tile.height as usize {
        let from = (offset_y as usize + row) * rendered_width + offset_x as usize;
        let to = row * width;
        cropped.pixels_mut()[to..to + width]
            .copy_from_slice(&rendered.pixels()[from..from + width]);
    }
    Ok(cropped)
}

/// Copies a tile into a band that starts at `first_row` of the output.
fn blit_band(
    band: &mut [FloatRgba],
    output_width: u32,
    first_row: u32,
    rendered: &FloatImage,
    source_region: &Region,
    tile: &Region,
) -> Result<(), AppError> {
    let (offset_x, offset_y) = source_region.offset_of(tile).ok_or_else(|| {
        AppError::InvalidLayerDocument("a tile fell outside the rectangle rendered for it".into())
    })?;
    let rendered_width = rendered.width() as usize;
    for row in 0..tile.height {
        let from = (offset_y + row) as usize * rendered_width + offset_x as usize;
        // The band's own first row is `first_row` of the output.
        let to = (tile.y + row - first_row) as usize * output_width as usize + tile.x as usize;
        let width = tile.width as usize;
        band[to..to + width].copy_from_slice(&rendered.pixels()[from..from + width]);
    }
    Ok(())
}

/// Renders a whole document at the default tile size.
pub fn render_document_tiled_default(
    document: &LayerDocument,
    source: &dyn PixelSource,
    options: RenderOptions<'_>,
) -> Result<FloatImage, AppError> {
    Ok(render_document_tiled(document, source, options, DEFAULT_TILE_SIZE)?.0)
}

fn composite_onto(
    canvas: &mut FloatImage,
    layers: &[Layer],
    context: &Context<'_>,
    depth: usize,
) -> Result<(), AppError> {
    if depth > MAX_GROUP_DEPTH {
        return Err(AppError::LayerDepthExceeded {
            depth,
            limit: MAX_GROUP_DEPTH,
        });
    }
    for layer in layers {
        high_precision::check_cancel(context.options.cancel)?;
        if !layer.visible || layer.opacity <= 0.0 {
            continue;
        }
        match &layer.content {
            LayerContent::Pixel { pixel_id, .. } => {
                let pixels = context.source.resolve_linear(pixel_id)?;
                draw(canvas, &pixels, layer, context)?;
            }
            LayerContent::Group { children, isolated } => {
                if children.is_empty() {
                    continue;
                }
                if *isolated {
                    // An isolated group is composited into its own buffer and
                    // then drawn through the group's transform. The buffer only
                    // has to cover the part of the canvas this rectangle can
                    // actually sample, which for an untransformed group is the
                    // rectangle itself.
                    let group_region = preimage_region(layer, context)?;
                    if group_region.is_empty() {
                        continue;
                    }
                    let group_context = Context {
                        source: context.source,
                        options: context.options,
                        canvas_width: context.canvas_width,
                        canvas_height: context.canvas_height,
                        region: group_region,
                    };
                    // Region sized, not canvas sized: this is the allocation the
                    // full-frame renderer made at full document size.
                    let mut group = FloatImage::blank(
                        group_region.width,
                        group_region.height,
                        FloatRgba::TRANSPARENT,
                    )?;
                    composite_onto(&mut group, children, &group_context, depth + 1)?;
                    draw_group(canvas, &group, group_region, layer, context)?;
                } else {
                    let mut reworked = canvas.clone();
                    composite_onto(&mut reworked, children, context, depth + 1)?;
                    cross_fade(canvas, &reworked, layer, context)?;
                }
            }
            LayerContent::Shape { shape } => {
                draw_shape(canvas, shape, layer, context)?;
            }
            LayerContent::Adjustment { operation } => {
                let adjusted = high_precision::apply(canvas, operation, context.options.cancel)?;
                mix_adjustment(canvas, &adjusted, layer, context)?;
            }
        }
    }
    high_precision::check_cancel(context.options.cancel)
}

/// Draws a shape layer into `canvas`, which holds `region` of the render canvas.
///
/// The geometry is mapped into device space and rasterised there, so a shape is
/// never resampled: scaling one up draws it again at the new size rather than
/// enlarging pixels. The resulting coverage goes through the same mask, opacity
/// and blend path as every other layer.
fn draw_shape(
    canvas: &mut FloatImage,
    shape: &crate::layers::shape::ShapeContent,
    layer: &Layer,
    context: &Context<'_>,
) -> Result<(), AppError> {
    let scale = context.options.scale;
    let transform = render_transform(&layer.transform, scale);
    let (canvas_width, canvas_height) = (context.canvas_width, context.canvas_height);
    // Shape coordinates are document coordinates, so the render scale is
    // applied first and the layer transform is then taken in the scaled frame,
    // exactly as it is for a pixel layer's bounds.
    let to_device = |(x, y): (f32, f32)| -> (f32, f32) {
        let scaled = ((f64::from(x) * scale) as f32, (f64::from(y) * scale) as f32);
        transform.forward(scaled, canvas_width, canvas_height)
    };
    let region = context.region;
    let Some((colours, coverage)) = crate::layers::shape::render_shape(
        shape,
        to_device,
        region.x as i32,
        region.y as i32,
        region.width,
        region.height,
    )?
    else {
        return Ok(());
    };

    let mask = decoded_mask(layer)?;
    let inverted = mask_inverted(layer);
    for y in 0..region.height {
        high_precision::check_cancel(context.options.cancel)?;
        for x in 0..region.width {
            let index = (y * region.width + x) as usize;
            if coverage[index] <= 0.0 {
                continue;
            }
            let mut source = colours.pixels()[index];
            // A shape has no buffer, so its mask lives on the canvas and is
            // sampled in document coordinates.
            source.alpha *= layer.opacity
                * mask_coverage(
                    mask.as_ref(),
                    inverted,
                    context.document_x(x) as f32,
                    context.document_y(y) as f32,
                    canvas_width,
                    canvas_height,
                    layer.transform.interpolation,
                );
            if source.alpha <= 0.0 {
                continue;
            }
            canvas.pixels_mut()[index] =
                source_over(canvas.pixels()[index], source, layer.blend_mode);
        }
    }
    Ok(())
}

/// Draws a shape over a whole canvas, for the full-frame reference renderer.
///
/// A thin wrapper so `layers::linear` and the tiled path share one shape
/// implementation. Two of them would be two things to keep identical, and one
/// of those two is the oracle the other is checked against.
pub(super) fn draw_shape_full_frame(
    canvas: &mut FloatImage,
    shape: &crate::layers::shape::ShapeContent,
    layer: &Layer,
    options: RenderOptions<'_>,
) -> Result<(), AppError> {
    let (width, height) = canvas.dimensions();
    let context = Context {
        source: &EmptySource,
        options,
        canvas_width: width,
        canvas_height: height,
        region: Region::whole(width, height),
    };
    draw_shape(canvas, shape, layer, &context)
}

/// A source that resolves nothing, for contexts where no pixel buffer is used.
///
/// Drawing a shape needs a `Context` for its region and options, and a shape
/// has no pixel source; refusing every identifier is the honest implementation.
struct EmptySource;

impl PixelSource for EmptySource {
    fn resolve(&self, pixel_id: &str) -> Result<Arc<image::RgbaImage>, AppError> {
        Err(AppError::LayerPixelsMissing(pixel_id.to_string()))
    }
}

/// The part of the canvas an isolated group must cover for this rectangle.
///
/// A group buffer lives in canvas coordinates, so with no transform it needs
/// exactly the rectangle being produced. A transformed group is sampled
/// somewhere else entirely, so the rectangle's four corners are mapped back
/// through the inverse transform and their bounding box is used, widened by one
/// pixel because bilinear sampling reads the neighbour either side.
fn preimage_region(layer: &Layer, context: &Context<'_>) -> Result<Region, AppError> {
    let canvas = Region::whole(context.canvas_width, context.canvas_height);
    let transform = render_transform(&layer.transform, context.options.scale);
    if transform.is_identity() {
        return Ok(context.region);
    }
    let inverse = transform.inverse(context.canvas_width, context.canvas_height)?;
    let region = context.region;
    let corners = [
        (region.x as f32, region.y as f32),
        (region.right() as f32, region.y as f32),
        (region.x as f32, region.bottom() as f32),
        (region.right() as f32, region.bottom() as f32),
    ];
    let (mut min_x, mut min_y) = (f32::MAX, f32::MAX);
    let (mut max_x, mut max_y) = (f32::MIN, f32::MIN);
    for (x, y) in corners {
        let (sx, sy) = inverse.apply(x, y);
        if !sx.is_finite() || !sy.is_finite() {
            // A degenerate transform cannot be bounded; cover the whole canvas
            // rather than guess at a rectangle.
            return Ok(canvas);
        }
        min_x = min_x.min(sx);
        min_y = min_y.min(sy);
        max_x = max_x.max(sx);
        max_y = max_y.max(sy);
    }
    let x = min_x.floor().max(0.0) as u32;
    let y = min_y.floor().max(0.0) as u32;
    let right = (max_x.ceil().max(0.0) as u64).min(u64::from(context.canvas_width));
    let bottom = (max_y.ceil().max(0.0) as u64).min(u64::from(context.canvas_height));
    if u64::from(x) >= right || u64::from(y) >= bottom {
        return Ok(Region::new(x, y, 0, 0));
    }
    Ok(Region::new(
        x,
        y,
        (right - u64::from(x)) as u32,
        (bottom - u64::from(y)) as u32,
    )
    // One pixel of margin for the bilinear taps at the edges.
    .expanded(1, &canvas))
}

/// Draws a group buffer that covers `group_region` of the canvas.
///
/// This differs from `draw` in one way that matters: a group buffer is already
/// in canvas coordinates, so the sample position has the buffer's own origin
/// subtracted from it. `draw` instead maps into a layer's own pixel grid, where
/// no such offset exists.
fn draw_group(
    canvas: &mut FloatImage,
    group: &FloatImage,
    group_region: Region,
    layer: &Layer,
    context: &Context<'_>,
) -> Result<(), AppError> {
    let transform = render_transform(&layer.transform, context.options.scale);
    let bounds = transform.document_bounds(context.canvas_width, context.canvas_height);
    let region = context.region;
    let x0 = bounds.min_x.max(i64::from(region.x));
    let y0 = bounds.min_y.max(i64::from(region.y));
    let x1 = bounds.max_x.min(region.right() as i64);
    let y1 = bounds.max_y.min(region.bottom() as i64);
    if x0 >= x1 || y0 >= y1 {
        return Ok(());
    }
    let inverse = transform.inverse(context.canvas_width, context.canvas_height)?;
    let mask = decoded_mask(layer)?;
    let inverted = mask_inverted(layer);
    let nearest = layer.transform.interpolation == LayerInterpolation::Nearest;
    for y in y0..y1 {
        high_precision::check_cancel(context.options.cancel)?;
        for x in x0..x1 {
            let (sx, sy) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
            let mut p = group.sample(
                sx - group_region.x as f32,
                sy - group_region.y as f32,
                nearest,
            );
            p.alpha *= layer.opacity
                * mask_coverage(
                    mask.as_ref(),
                    inverted,
                    sx,
                    sy,
                    context.canvas_width,
                    context.canvas_height,
                    layer.transform.interpolation,
                );
            if p.alpha <= 0.0 {
                continue;
            }
            let local_x = (x - i64::from(region.x)) as u32;
            let local_y = (y - i64::from(region.y)) as u32;
            let i = (local_y * region.width + local_x) as usize;
            canvas.pixels_mut()[i] = source_over(canvas.pixels()[i], p, layer.blend_mode);
        }
    }
    Ok(())
}

fn channels(p: FloatRgba) -> [f32; 3] {
    [p.red, p.green, p.blue]
}

/// Draws a pixel buffer through its transform, mask, opacity and blend mode.
///
/// The loop runs over document coordinates and writes at local ones, so a layer
/// that only touches part of a tile costs only that part.
fn draw(
    canvas: &mut FloatImage,
    source: &FloatImage,
    layer: &Layer,
    context: &Context<'_>,
) -> Result<(), AppError> {
    let (sw, sh) = source.dimensions();
    let transform = render_transform(&layer.transform, context.options.scale);
    let bounds = transform.document_bounds(sw, sh);
    let region = context.region;
    // Clip the layer's document-space bounds to the rectangle being produced.
    let x0 = bounds.min_x.max(i64::from(region.x));
    let y0 = bounds.min_y.max(i64::from(region.y));
    let x1 = bounds.max_x.min(region.right() as i64);
    let y1 = bounds.max_y.min(region.bottom() as i64);
    if x0 >= x1 || y0 >= y1 {
        return Ok(());
    }
    let inverse = transform.inverse(sw, sh)?;
    let mask = decoded_mask(layer)?;
    let inverted = mask_inverted(layer);
    let nearest = layer.transform.interpolation == LayerInterpolation::Nearest;
    for y in y0..y1 {
        high_precision::check_cancel(context.options.cancel)?;
        for x in x0..x1 {
            let (sx, sy) = inverse.apply(x as f32 + 0.5, y as f32 + 0.5);
            let mut p = source.sample(sx, sy, nearest);
            p.alpha *= layer.opacity
                * mask_coverage(
                    mask.as_ref(),
                    inverted,
                    sx,
                    sy,
                    sw,
                    sh,
                    layer.transform.interpolation,
                );
            if p.alpha <= 0.0 {
                continue;
            }
            let local_x = (x - i64::from(region.x)) as u32;
            let local_y = (y - i64::from(region.y)) as u32;
            let i = (local_y * region.width + local_x) as usize;
            canvas.pixels_mut()[i] = source_over(canvas.pixels()[i], p, layer.blend_mode);
        }
    }
    Ok(())
}

/// Interpolates a pass-through group's reworked backdrop back over the original.
fn cross_fade(
    canvas: &mut FloatImage,
    changed: &FloatImage,
    layer: &Layer,
    context: &Context<'_>,
) -> Result<(), AppError> {
    let mask = decoded_mask(layer)?;
    let inverted = mask_inverted(layer);
    // A group's transform and mask are defined against the whole canvas, so the
    // inverse is built from the canvas dimensions even though only a rectangle
    // is being produced.
    let inverse = render_transform(&layer.transform, context.options.scale)
        .inverse(context.canvas_width, context.canvas_height)?;
    let region = context.region;
    for y in 0..region.height {
        high_precision::check_cancel(context.options.cancel)?;
        for x in 0..region.width {
            let (sx, sy) = inverse.apply(
                context.document_x(x) as f32 + 0.5,
                context.document_y(y) as f32 + 0.5,
            );
            let t = layer.opacity
                * mask_coverage(
                    mask.as_ref(),
                    inverted,
                    sx,
                    sy,
                    context.canvas_width,
                    context.canvas_height,
                    layer.transform.interpolation,
                );
            if t <= 0.0 {
                continue;
            }
            let i = (y * region.width + x) as usize;
            let a = canvas.pixels()[i];
            let b = changed.pixels()[i];
            let alpha = a.alpha + t * (b.alpha - a.alpha);
            let p = if alpha <= 0.0 {
                FloatRgba::TRANSPARENT
            } else {
                let rgb: [f32; 3] = std::array::from_fn(|c| {
                    (channels(a)[c] * a.alpha * (1.0 - t) + channels(b)[c] * b.alpha * t) / alpha
                });
                FloatRgba::new(rgb[0], rgb[1], rgb[2], alpha.clamp(0.0, 1.0))
            };
            canvas.pixels_mut()[i] = p;
        }
    }
    Ok(())
}

fn mix_adjustment(
    canvas: &mut FloatImage,
    changed: &FloatImage,
    layer: &Layer,
    context: &Context<'_>,
) -> Result<(), AppError> {
    if canvas.dimensions() != changed.dimensions() {
        return Err(AppError::InvalidLayerDocument(
            "an adjustment layer must preserve the canvas dimensions".into(),
        ));
    }
    let mask = decoded_mask(layer)?;
    let inverted = mask_inverted(layer);
    let inverse = render_transform(&layer.transform, context.options.scale)
        .inverse(context.canvas_width, context.canvas_height)?;
    let region = context.region;
    for y in 0..region.height {
        high_precision::check_cancel(context.options.cancel)?;
        for x in 0..region.width {
            let (sx, sy) = inverse.apply(
                context.document_x(x) as f32 + 0.5,
                context.document_y(y) as f32 + 0.5,
            );
            let coverage = layer.opacity
                * mask_coverage(
                    mask.as_ref(),
                    inverted,
                    sx,
                    sy,
                    context.canvas_width,
                    context.canvas_height,
                    layer.transform.interpolation,
                );
            if coverage <= 0.0 {
                continue;
            }
            let i = (y * region.width + x) as usize;
            let base = canvas.pixels()[i];
            let blended = blend_linear(
                layer.blend_mode,
                channels(base),
                channels(changed.pixels()[i]),
            );
            let rgb: [f32; 3] = std::array::from_fn(|c| {
                channels(base)[c] + coverage * (blended[c] - channels(base)[c])
            });
            canvas.pixels_mut()[i] = FloatRgba::new(rgb[0], rgb[1], rgb[2], base.alpha);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{DevelopmentParameters, WhiteBalance};
    use crate::domain::EditOperation;
    use crate::layers::{test_pixel_layer, BlendMode, LayerMask, LayerPixelStore, LayerTransform};
    use crate::mask::{MaskBitmap, MaskSnapshot};
    use crate::pixel::DocumentPrecision;

    const W: u32 = 300;
    const H: u32 = 220;

    /// A scene with structure in every channel, so a misplaced tile shows up.
    fn source_image(width: u32, height: u32, seed: u32) -> FloatImage {
        let pixels = (0..width * height)
            .map(|index| {
                let x = index % width;
                let y = index / width;
                let f =
                    |v: u32, k: u32| ((v.wrapping_mul(k).wrapping_add(seed)) % 97) as f32 / 97.0;
                FloatRgba::new(
                    f(x, 7) * 0.9 + 0.05,
                    f(y, 11) * 0.9 + 0.05,
                    f(x.wrapping_add(y), 13) * 0.9 + 0.05,
                    // Varying alpha, including fully transparent runs.
                    if (x / 17 + y / 19).is_multiple_of(11) {
                        0.0
                    } else {
                        0.25 + f(x.wrapping_add(y.wrapping_mul(3)), 5) * 0.75
                    },
                )
            })
            .collect();
        FloatImage::new(width, height, pixels).expect("fixture image")
    }

    fn store_with(images: &[(&str, FloatImage)]) -> LayerPixelStore {
        let mut store = LayerPixelStore::default();
        store.reset(W, H).expect("canvas");
        for (id, image) in images {
            store
                .register_typed_with_id(id, crate::pixel::PixelBuffer::from(image.clone()))
                .expect("register");
        }
        store
    }

    fn document(layers: Vec<Layer>) -> LayerDocument {
        let mut document = LayerDocument::new(W, H);
        document.precision = DocumentPrecision::LinearSrgbF32;
        document.layers = layers;
        document
    }

    fn pixel_layer(id: &str, source: &str) -> Layer {
        test_pixel_layer(id, source, W, H)
    }

    fn half_mask() -> LayerMask {
        let mut bitmap = MaskBitmap::full(W, H).expect("mask");
        for y in 0..H {
            for x in 0..W {
                // A soft diagonal edge, so mask sampling is exercised rather
                // than a single hard boundary that might land between tiles.
                let value = ((x + y) % 256) as u8;
                bitmap.set(x, y, value);
            }
        }
        LayerMask {
            snapshot: MaskSnapshot::encode(&bitmap),
            enabled: true,
            inverted: false,
        }
    }

    /// Renders both ways and returns the largest per-channel difference.
    fn compare(document: &LayerDocument, store: &LayerPixelStore, tile_size: u32) -> f32 {
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .expect("resolve");
        let reference = super::super::linear::render_document_float(
            document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
        )
        .expect("reference render");
        let (tiled, stats) = render_document_tiled(
            document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
            tile_size,
        )
        .expect("tiled render");
        assert_eq!(reference.dimensions(), tiled.dimensions());
        assert!(stats.tiles > 0);
        reference
            .pixels()
            .iter()
            .zip(tiled.pixels())
            .map(|(a, b)| {
                (a.red - b.red)
                    .abs()
                    .max((a.green - b.green).abs())
                    .max((a.blue - b.blue).abs())
                    .max((a.alpha - b.alpha).abs())
            })
            .fold(0.0f32, f32::max)
    }

    /// The tiled renderer performs the same arithmetic in the same order for
    /// tile-local work, so agreement is exact rather than approximate. Any
    /// difference at all would mean a coordinate or clipping mistake.
    /// A deterministic pseudo-random generator, so a failure is reproducible
    /// from its seed rather than "sometimes".
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, limit: u64) -> u64 {
            self.next() % limit.max(1)
        }

        fn unit(&mut self) -> f32 {
            (self.below(1000) as f32) / 1000.0
        }
    }

    /// Builds a small random layer tree: groups, masks, opacity, blend modes,
    /// adjustments and transforms, mixed at random.
    fn random_document(rng: &mut Rng, sources: &[&str]) -> LayerDocument {
        fn build(rng: &mut Rng, sources: &[&str], depth: usize, index: &mut u32) -> Layer {
            *index += 1;
            let id = format!("n{index}");
            let choice = rng.below(10);
            let mut layer = if depth < 2 && choice < 3 {
                let count = 1 + rng.below(2) as usize;
                let children = (0..count)
                    .map(|_| build(rng, sources, depth + 1, index))
                    .collect();
                Layer {
                    content: LayerContent::Group {
                        children,
                        isolated: rng.below(2) == 0,
                    },
                    ..test_pixel_layer(&id, "unused", W, H)
                }
            } else if choice < 5 {
                let operation = match rng.below(4) {
                    0 => EditOperation::Brightness {
                        amount: rng.unit() - 0.5,
                    },
                    1 => EditOperation::Contrast {
                        amount: rng.unit() - 0.5,
                    },
                    2 => EditOperation::Saturation {
                        amount: rng.unit() - 0.5,
                    },
                    _ => EditOperation::Grayscale,
                };
                Layer {
                    content: LayerContent::Adjustment {
                        operation: Box::new(operation),
                    },
                    ..test_pixel_layer(&id, "unused", W, H)
                }
            } else {
                let source = sources[rng.below(sources.len() as u64) as usize];
                test_pixel_layer(&id, source, W, H)
            };
            layer.opacity = 0.2 + rng.unit() * 0.8;
            layer.blend_mode = BlendMode::ALL[rng.below(16) as usize];
            // A pass-through group has no blend mode of its own; the document
            // validator enforces that, so the generator must respect it.
            if matches!(
                layer.content,
                LayerContent::Group {
                    isolated: false,
                    ..
                }
            ) {
                layer.blend_mode = BlendMode::Normal;
            }
            if rng.below(3) == 0 {
                layer.mask = Some(half_mask());
            }
            if rng.below(3) == 0 {
                layer.transform = LayerTransform {
                    translate_x: rng.unit() * 20.0 - 10.0,
                    translate_y: rng.unit() * 20.0 - 10.0,
                    scale_x: 0.8 + rng.unit() * 0.4,
                    scale_y: 0.8 + rng.unit() * 0.4,
                    rotation_degrees: rng.unit() * 8.0 - 4.0,
                    ..LayerTransform::default()
                };
            }
            layer
        }

        let mut index = 0;
        let count = 1 + rng.below(3) as usize;
        let layers = (0..count)
            .map(|_| build(rng, sources, 0, &mut index))
            .collect();
        document(layers)
    }

    /// The broad equivalence check: many random trees, each rendered both ways.
    ///
    /// A pass-through group over a transformed child with a mask and an unusual
    /// blend mode is exactly the combination a hand-written test set tends to
    /// miss, and it is where a tiling mistake would hide.
    #[test]
    fn random_layer_trees_match_the_full_frame_renderer() {
        let store = store_with(&[
            ("a", source_image(W, H, 31)),
            ("b", source_image(W, H, 32)),
            ("c", source_image(W, H, 33)),
        ]);
        let mut worst = 0.0f32;
        for seed in 1..=40u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let document = random_document(&mut rng, &["a", "b", "c"]);
            let difference = compare(&document, &store, 64);
            assert!(
                difference < 1e-5,
                "seed {seed} differed by {difference} between the tiled and full-frame renderers"
            );
            worst = worst.max(difference);
        }
        // Tile-local trees should agree exactly; anything else means the tiled
        // path is doing arithmetic in a different order somewhere.
        assert_eq!(worst, 0.0, "random trees were not bit-identical");
    }

    /// The seam check proper. Without a correct halo the error concentrates in
    /// a band as wide as the kernel, so this measures the whole band rather
    /// than the two pixels nearest the edge.
    #[test]
    fn a_wide_blur_leaves_no_error_band_at_tile_edges() {
        let store = store_with(&[("a", source_image(W, H, 34))]);
        let adjustment = Layer {
            content: LayerContent::Adjustment {
                // Sigma 5 reaches 15 pixels, so a naive halo would leave a band
                // a quarter of a 64-pixel tile wide.
                operation: Box::new(EditOperation::GaussianBlur { radius: 5.0 }),
            },
            ..pixel_layer("blur", "unused")
        };
        let document = document(vec![pixel_layer("base", "a"), adjustment]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .expect("resolve");
        let options = RenderOptions {
            scale: 1.0,
            cancel: None,
        };
        let reference =
            super::super::linear::render_document_float(&document, &resolved, options).unwrap();
        let (tiled, stats) = render_document_tiled(&document, &resolved, options, 64).unwrap();
        assert!(stats.haloed_tiles > 0, "no tile was given a halo");

        let tile_size = 64u32;
        let (mut boundary, mut interior) = (0.0f32, 0.0f32);
        for y in 0..H {
            for x in 0..W {
                let index = (y * W + x) as usize;
                let a = reference.pixels()[index];
                let b = tiled.pixels()[index];
                let error = (a.red - b.red)
                    .abs()
                    .max((a.green - b.green).abs())
                    .max((a.blue - b.blue).abs())
                    .max((a.alpha - b.alpha).abs());
                // The full band a 15-pixel kernel could corrupt, on both sides
                // of every internal tile edge.
                let near = |v: u32, size: u32| v > 0 && (v % size < 16 || v % size > size - 16);
                if near(x, tile_size) || near(y, tile_size) {
                    boundary = boundary.max(error);
                } else {
                    interior = interior.max(error);
                }
            }
        }
        assert!(
            boundary < 1e-3,
            "a seam band of {boundary} appeared at tile edges (interior {interior})"
        );
    }

    /// Halo growth must not change the answer, only the work done.
    #[test]
    fn a_blur_gives_the_same_result_at_every_tile_size() {
        let store = store_with(&[("a", source_image(W, H, 35))]);
        let adjustment = Layer {
            content: LayerContent::Adjustment {
                operation: Box::new(EditOperation::GaussianBlur { radius: 2.5 }),
            },
            ..pixel_layer("blur", "unused")
        };
        let document = document(vec![pixel_layer("base", "a"), adjustment]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let options = RenderOptions {
            scale: 1.0,
            cancel: None,
        };
        let baseline = render_document_tiled(&document, &resolved, options, 512)
            .unwrap()
            .0;
        for tile_size in [64, 96, 128, 256] {
            let other = render_document_tiled(&document, &resolved, options, tile_size)
                .unwrap()
                .0;
            let worst = baseline
                .pixels()
                .iter()
                .zip(other.pixels())
                .map(|(a, b)| (a.red - b.red).abs().max((a.alpha - b.alpha).abs()))
                .fold(0.0f32, f32::max);
            assert!(
                worst < 1e-3,
                "tile size {tile_size} changed a blurred result by {worst}"
            );
        }
    }

    /// A transformed group is sampled from somewhere other than the rectangle
    /// being produced, so its buffer has to cover the pre-image of that
    /// rectangle. This is the case the first implementation got wrong.
    #[test]
    fn a_transformed_group_matches_the_full_frame_renderer() {
        let store = store_with(&[("a", source_image(W, H, 36)), ("b", source_image(W, H, 37))]);
        for (translate, rotation, scale) in [
            (30.0, 0.0, 1.0),
            (-45.5, 0.0, 1.0),
            (0.0, 7.0, 1.0),
            (12.0, -5.0, 1.25),
            (0.0, 0.0, 0.6),
        ] {
            let mut group = pixel_layer("group", "unused");
            group.content = LayerContent::Group {
                children: vec![pixel_layer("inner", "b")],
                isolated: true,
            };
            group.transform = LayerTransform {
                translate_x: translate,
                rotation_degrees: rotation,
                scale_x: scale,
                scale_y: scale,
                ..LayerTransform::default()
            };
            group.opacity = 0.9;
            let document = document(vec![pixel_layer("base", "a"), group]);
            let difference = compare(&document, &store, 64);
            assert!(
                difference < 1e-5,
                "a group translated {translate}, rotated {rotation}, scaled {scale} differed by {difference}"
            );
        }
    }

    #[test]
    fn a_plain_layer_stack_is_bit_identical_to_the_full_frame_renderer() {
        let store = store_with(&[("a", source_image(W, H, 1)), ("b", source_image(W, H, 2))]);
        let mut top = pixel_layer("top", "b");
        top.opacity = 0.6;
        top.blend_mode = BlendMode::Multiply;
        let document = document(vec![pixel_layer("base", "a"), top]);
        assert_eq!(compare(&document, &store, 64), 0.0);
    }

    #[test]
    fn every_blend_mode_matches_the_full_frame_renderer() {
        let store = store_with(&[("a", source_image(W, H, 3)), ("b", source_image(W, H, 4))]);
        for mode in BlendMode::ALL {
            let mut top = pixel_layer("top", "b");
            top.blend_mode = mode;
            top.opacity = 0.75;
            let document = document(vec![pixel_layer("base", "a"), top]);
            assert_eq!(
                compare(&document, &store, 64),
                0.0,
                "{mode:?} differed between the tiled and full-frame renderers"
            );
        }
    }

    #[test]
    fn masks_opacity_and_transforms_match_across_tile_boundaries() {
        let store = store_with(&[("a", source_image(W, H, 5)), ("b", source_image(W, H, 6))]);
        let mut top = pixel_layer("top", "b");
        top.mask = Some(half_mask());
        top.opacity = 0.8;
        top.transform = LayerTransform {
            translate_x: 7.25,
            translate_y: -3.5,
            scale_x: 1.15,
            scale_y: 0.92,
            rotation_degrees: 4.5,
            ..LayerTransform::default()
        };
        let document = document(vec![pixel_layer("base", "a"), top]);
        // A subpixel, rotated, scaled placement is where an origin error in the
        // tiled path would show as a seam.
        assert_eq!(compare(&document, &store, 64), 0.0);
    }

    #[test]
    fn a_layer_partly_off_canvas_matches() {
        let store = store_with(&[("a", source_image(W, H, 7))]);
        let mut layer = pixel_layer("only", "a");
        layer.transform.translate_x = -120.0;
        layer.transform.translate_y = 90.0;
        let document = document(vec![layer]);
        assert_eq!(compare(&document, &store, 64), 0.0);
    }

    #[test]
    fn isolated_and_pass_through_groups_match() {
        let store = store_with(&[("a", source_image(W, H, 8)), ("b", source_image(W, H, 9))]);
        for isolated in [true, false] {
            let mut inner = pixel_layer("inner", "b");
            inner.opacity = 0.7;
            let mut group = pixel_layer("group", "unused");
            group.content = LayerContent::Group {
                children: vec![
                    inner,
                    Layer {
                        content: LayerContent::Adjustment {
                            operation: Box::new(EditOperation::Brightness { amount: 0.2 }),
                        },
                        ..pixel_layer("lift", "unused")
                    },
                ],
                isolated,
            };
            group.opacity = 0.85;
            group.mask = Some(half_mask());
            let document = document(vec![pixel_layer("base", "a"), group]);
            assert_eq!(
                compare(&document, &store, 64),
                0.0,
                "isolated={isolated} group differed"
            );
        }
    }

    #[test]
    fn nested_groups_match() {
        let store = store_with(&[("a", source_image(W, H, 10)), ("b", source_image(W, H, 11))]);
        let mut deep = pixel_layer("deep", "b");
        deep.opacity = 0.6;
        let inner = Layer {
            content: LayerContent::Group {
                children: vec![deep],
                isolated: true,
            },
            ..pixel_layer("inner", "unused")
        };
        let mut outer = pixel_layer("outer", "unused");
        outer.content = LayerContent::Group {
            children: vec![inner],
            isolated: false,
        };
        outer.opacity = 0.9;
        let document = document(vec![pixel_layer("base", "a"), outer]);
        assert_eq!(compare(&document, &store, 64), 0.0);
    }

    #[test]
    fn tile_local_adjustment_layers_match() {
        let store = store_with(&[("a", source_image(W, H, 12))]);
        for operation in [
            EditOperation::Brightness { amount: 0.3 },
            EditOperation::Contrast { amount: -0.2 },
            EditOperation::Saturation { amount: 0.4 },
            EditOperation::Grayscale,
            EditOperation::RawDevelopment {
                parameters: DevelopmentParameters {
                    exposure_ev: 0.5,
                    ..Default::default()
                },
            },
        ] {
            let adjustment = Layer {
                content: LayerContent::Adjustment {
                    operation: Box::new(operation.clone()),
                },
                ..pixel_layer("adjust", "unused")
            };
            let document = document(vec![pixel_layer("base", "a"), adjustment]);
            assert_eq!(
                compare(&document, &store, 64),
                0.0,
                "{operation:?} differed between renderers"
            );
        }
    }

    /// A neighbourhood operation is where tiling can go visibly wrong. The
    /// halo makes each tile read past its own edge; without it the difference
    /// would appear as a grid of seams.
    #[test]
    fn a_blur_adjustment_matches_within_tolerance_and_leaves_no_seam() {
        let store = store_with(&[("a", source_image(W, H, 13))]);
        let adjustment = Layer {
            content: LayerContent::Adjustment {
                operation: Box::new(EditOperation::GaussianBlur { radius: 3.0 }),
            },
            ..pixel_layer("blur", "unused")
        };
        let document = document(vec![pixel_layer("base", "a"), adjustment]);
        let difference = compare(&document, &store, 64);
        assert!(
            difference < 2e-3,
            "a blur differed by {difference} between the tiled and full-frame renderers"
        );
    }

    /// Neighbourhood dependencies compose through sequential adjustments. A
    /// plan that takes only the largest individual halo computes the outer
    /// pixels of the first blur with incomplete input, then feeds those wrong
    /// pixels into the second blur at every tile boundary.
    #[test]
    fn sequential_blurs_match_the_full_frame_reference() {
        let store = store_with(&[("a", source_image(W, H, 1313))]);
        let blur = |id: &str| Layer {
            content: LayerContent::Adjustment {
                operation: Box::new(EditOperation::GaussianBlur { radius: 2.0 }),
            },
            ..pixel_layer(id, "unused")
        };
        let document = document(vec![
            pixel_layer("base", "a"),
            blur("blur-1"),
            blur("blur-2"),
        ]);
        assert_eq!(document_tiling(&document, 1.0).halo, 14);
        let difference = compare(&document, &store, 64);
        assert!(
            difference < 2e-3,
            "sequential blurs differed by {difference} between tiled and full-frame renders"
        );
    }

    /// The seam test proper: without a halo the error concentrates on tile
    /// boundaries, so this asserts the boundary columns are no worse than the
    /// interior rather than merely that the average is small.
    #[test]
    fn blur_tile_boundaries_are_no_worse_than_tile_interiors() {
        let store = store_with(&[("a", source_image(W, H, 14))]);
        let adjustment = Layer {
            content: LayerContent::Adjustment {
                operation: Box::new(EditOperation::GaussianBlur { radius: 4.0 }),
            },
            ..pixel_layer("blur", "unused")
        };
        let document = document(vec![pixel_layer("base", "a"), adjustment]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .expect("resolve");
        let options = RenderOptions {
            scale: 1.0,
            cancel: None,
        };
        let reference =
            super::super::linear::render_document_float(&document, &resolved, options).unwrap();
        let (tiled, _) = render_document_tiled(&document, &resolved, options, 64).unwrap();

        let tile_size = 64u32;
        let mut boundary = 0.0f32;
        let mut interior = 0.0f32;
        for y in 0..H {
            for x in 0..W {
                let index = (y * W + x) as usize;
                let a = reference.pixels()[index];
                let b = tiled.pixels()[index];
                let error = (a.red - b.red)
                    .abs()
                    .max((a.green - b.green).abs())
                    .max((a.blue - b.blue).abs());
                // Within two pixels of an internal tile edge.
                let near_edge = (x % tile_size < 2 && x > 0) || (y % tile_size < 2 && y > 0);
                if near_edge {
                    boundary = boundary.max(error);
                } else {
                    interior = interior.max(error);
                }
            }
        }
        assert!(
            boundary <= interior.max(1e-4) * 4.0,
            "tile boundaries show a seam: boundary {boundary} against interior {interior}"
        );
    }

    #[test]
    fn tile_size_does_not_change_the_result() {
        let store = store_with(&[("a", source_image(W, H, 15)), ("b", source_image(W, H, 16))]);
        let mut top = pixel_layer("top", "b");
        top.blend_mode = BlendMode::Overlay;
        top.mask = Some(half_mask());
        let document = document(vec![pixel_layer("base", "a"), top]);
        // A size that divides the canvas, one that does not, and one larger
        // than the canvas so there is a single tile.
        for tile_size in [64, 100, 150, 512] {
            assert_eq!(
                compare(&document, &store, tile_size),
                0.0,
                "tile size {tile_size} changed the result"
            );
        }
    }

    #[test]
    fn a_tile_larger_than_the_canvas_produces_one_tile() {
        let store = store_with(&[("a", source_image(W, H, 17))]);
        let document = document(vec![pixel_layer("base", "a")]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let (_, stats) = render_document_tiled(
            &document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
            2048,
        )
        .unwrap();
        assert_eq!(stats.tiles, 1);
        assert!(!stats.fell_back_to_full_frame);
    }

    /// A global operation cannot be evaluated per tile. The renderer must say
    /// so and produce the correct full-frame result, not silently normalise
    /// each tile against its own statistics.
    #[test]
    fn a_global_operation_falls_back_to_the_full_frame_and_reports_it() {
        let store = store_with(&[("a", source_image(W, H, 18))]);
        let adjustment = Layer {
            content: LayerContent::Adjustment {
                operation: Box::new(EditOperation::AutoWhiteBalance { strength: 0.6 }),
            },
            ..pixel_layer("awb", "unused")
        };
        let document = document(vec![pixel_layer("base", "a"), adjustment]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let options = RenderOptions {
            scale: 1.0,
            cancel: None,
        };
        let (tiled, stats) = render_document_tiled(&document, &resolved, options, 64).unwrap();
        assert!(stats.fell_back_to_full_frame);
        let reference =
            super::super::linear::render_document_float(&document, &resolved, options).unwrap();
        assert_eq!(reference.pixels(), tiled.pixels());
    }

    /// The memory claim, measured rather than asserted: the largest thing a
    /// tiled render allocates is one expanded tile, not one frame.
    #[test]
    fn a_tiled_render_never_allocates_a_full_frame_intermediate() {
        let store = store_with(&[("a", source_image(W, H, 19))]);
        let mut group = pixel_layer("group", "unused");
        group.content = LayerContent::Group {
            children: vec![pixel_layer("inner", "a")],
            isolated: true,
        };
        let document = document(vec![pixel_layer("base", "a"), group]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let (_, stats) = render_document_tiled(
            &document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
            64,
        )
        .unwrap();
        let frame = Region::whole(W, H).float_bytes();
        assert!(
            stats.peak_region_bytes * 4 < frame,
            "an intermediate of {} bytes is not much smaller than a {frame}-byte frame",
            stats.peak_region_bytes
        );
    }

    #[test]
    fn rendering_a_region_refuses_a_rectangle_outside_the_canvas() {
        let store = store_with(&[("a", source_image(W, H, 20))]);
        let document = document(vec![pixel_layer("base", "a")]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let options = RenderOptions {
            scale: 1.0,
            cancel: None,
        };
        for region in [
            Region::new(0, 0, W + 1, H),
            Region::new(W - 1, 0, 8, 8),
            Region::new(0, 0, 0, 0),
        ] {
            assert!(render_region(&document, &resolved, options, region).is_err());
        }
    }

    /// One rectangle rendered directly must equal the same rectangle taken out
    /// of a whole-canvas render.
    #[test]
    fn a_region_render_equals_that_part_of_the_whole_canvas() {
        let store = store_with(&[("a", source_image(W, H, 21)), ("b", source_image(W, H, 22))]);
        let mut top = pixel_layer("top", "b");
        top.blend_mode = BlendMode::Screen;
        top.opacity = 0.55;
        top.transform.translate_x = 12.5;
        let document = document(vec![pixel_layer("base", "a"), top]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let options = RenderOptions {
            scale: 1.0,
            cancel: None,
        };
        let whole =
            super::super::linear::render_document_float(&document, &resolved, options).unwrap();
        let region = Region::new(97, 61, 83, 47);
        let part = render_region(&document, &resolved, options, region).unwrap();
        for y in 0..region.height {
            for x in 0..region.width {
                let from_whole = whole.pixels()[((region.y + y) * W + region.x + x) as usize];
                let from_part = part.pixels()[(y * region.width + x) as usize];
                assert_eq!(from_whole, from_part, "({x},{y}) differed");
            }
        }
    }

    #[test]
    fn a_preview_scale_render_matches_the_full_frame_renderer() {
        let store = store_with(&[("a", source_image(W, H, 23))]);
        let mut top = pixel_layer("top", "a");
        top.transform.scale_x = 0.8;
        let document = document(vec![pixel_layer("base", "a"), top]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let options = RenderOptions {
            scale: 0.5,
            cancel: None,
        };
        let reference =
            super::super::linear::render_document_float(&document, &resolved, options).unwrap();
        let (tiled, _) = render_document_tiled(&document, &resolved, options, 64).unwrap();
        assert_eq!(reference.dimensions(), tiled.dimensions());
        assert_eq!(reference.pixels(), tiled.pixels());
    }

    #[test]
    fn a_cancelled_tiled_render_stops_and_reports_cancellation() {
        use std::sync::atomic::AtomicBool;
        let store = store_with(&[("a", source_image(W, H, 24))]);
        let document = document(vec![pixel_layer("base", "a")]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let cancel = AtomicBool::new(true);
        let result = render_document_tiled(
            &document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: Some(&cancel),
            },
            64,
        );
        assert!(matches!(result, Err(AppError::RenderCancelled)));
    }

    #[test]
    fn an_empty_document_renders_transparent_in_both_paths() {
        let store = store_with(&[]);
        let document = document(Vec::new());
        let resolved = store.resolve(&[], false).unwrap();
        let options = RenderOptions {
            scale: 1.0,
            cancel: None,
        };
        let (tiled, _) = render_document_tiled(&document, &resolved, options, 64).unwrap();
        assert!(tiled.pixels().iter().all(|p| *p == FloatRgba::TRANSPARENT));
    }

    #[test]
    fn white_balance_auto_inside_a_group_still_forces_the_full_frame() {
        let store = store_with(&[("a", source_image(W, H, 25))]);
        let adjustment = Layer {
            content: LayerContent::Adjustment {
                operation: Box::new(EditOperation::RawDevelopment {
                    parameters: DevelopmentParameters {
                        white_balance: WhiteBalance::Auto,
                        ..Default::default()
                    },
                }),
            },
            ..pixel_layer("awb", "unused")
        };
        let group = Layer {
            content: LayerContent::Group {
                children: vec![adjustment],
                isolated: true,
            },
            ..pixel_layer("group", "unused")
        };
        let document = document(vec![pixel_layer("base", "a"), group]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let (_, stats) = render_document_tiled(
            &document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
            64,
        )
        .unwrap();
        assert!(
            stats.fell_back_to_full_frame,
            "a global operation nested in a group was not detected"
        );
    }

    /// The scheduler must be an optimisation, not a variable. Every thread
    /// count has to produce the same bytes, or "the tiled renderer matches the
    /// reference" would only be true on the machine that ran the test.
    #[test]
    fn the_thread_count_does_not_change_the_result() {
        let store = store_with(&[
            ("a", source_image(W, H, 61)),
            ("b", source_image(W, H, 62)),
            ("c", source_image(W, H, 63)),
        ]);
        for seed in 1..=12u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let document = random_document(&mut rng, &["a", "b", "c"]);
            let resolved = store
                .resolve(&document.referenced_pixel_ids(), false)
                .expect("resolve");
            let options = RenderOptions {
                scale: 1.0,
                cancel: None,
            };
            let single =
                render_document_tiled_with_threads(&document, &resolved, options, 64, 1).unwrap();
            for threads in [2, 3, 4, 8, 64] {
                let other =
                    render_document_tiled_with_threads(&document, &resolved, options, 64, threads)
                        .unwrap();
                assert_eq!(
                    single.0.pixels(),
                    other.0.pixels(),
                    "seed {seed} rendered differently on {threads} threads"
                );
                assert_eq!(single.1.tiles, other.1.tiles);
            }
        }
    }

    /// A blurred document exercises the halo path on every worker at once,
    /// where a shared-state mistake in the scheduler would show up.
    #[test]
    fn a_haloed_render_is_unaffected_by_the_thread_count() {
        let store = store_with(&[("a", source_image(W, H, 64))]);
        let adjustment = Layer {
            content: LayerContent::Adjustment {
                operation: Box::new(EditOperation::GaussianBlur { radius: 4.0 }),
            },
            ..pixel_layer("blur", "unused")
        };
        let document = document(vec![pixel_layer("base", "a"), adjustment]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let options = RenderOptions {
            scale: 1.0,
            cancel: None,
        };
        let single =
            render_document_tiled_with_threads(&document, &resolved, options, 64, 1).unwrap();
        assert!(single.1.haloed_tiles > 0, "no tile was given a halo");
        for threads in [2, 4, 8] {
            let other =
                render_document_tiled_with_threads(&document, &resolved, options, 64, threads)
                    .unwrap();
            assert_eq!(
                single.0.pixels(),
                other.0.pixels(),
                "{threads} threads changed a blurred render"
            );
        }
    }

    /// Cancellation has to reach the workers, not just the caller: a cancelled
    /// render must fail rather than return a half-drawn frame.
    #[test]
    fn a_cancelled_threaded_render_fails_rather_than_returning_a_partial_frame() {
        let store = store_with(&[("a", source_image(W, H, 65))]);
        let document = document(vec![pixel_layer("base", "a")]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(true);
        let error = render_document_tiled_with_threads(
            &document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: Some(&cancel),
            },
            64,
            8,
        )
        .expect_err("a cancelled render should not succeed");
        assert!(
            matches!(error, AppError::RenderCancelled),
            "expected cancellation, got {error:?}"
        );
    }

    /// The pool is bounded whatever the caller or the machine asks for. This is
    /// the difference between a scheduler and an unbounded thread spawn.
    #[test]
    fn the_worker_pool_is_bounded() {
        assert_eq!(worker_count(1000, 1000), MAX_RENDER_THREADS);
        assert!((1..=MAX_RENDER_THREADS).contains(&worker_count(1000, 0)));
        // Never more threads than there is work for them to do.
        assert_eq!(worker_count(2, 8), 2);
        assert_eq!(worker_count(0, 8), 1);
        assert_eq!(worker_count(5, 1), 1);
    }

    /// Reassembles a streamed render so it can be compared pixel for pixel.
    fn collect(
        document: &LayerDocument,
        store: &LayerPixelStore,
        tile_size: u32,
        threads: usize,
    ) -> (FloatImage, TiledStats, Vec<u32>) {
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .expect("resolve");
        let (width, height) = (document.canvas_width, document.canvas_height);
        let mut assembled = FloatImage::blank(width, height, FloatRgba::TRANSPARENT).unwrap();
        let mut starts = Vec::new();
        let stats = render_document_streaming(
            document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
            tile_size,
            threads,
            &mut |first_row, band| {
                starts.push(first_row);
                let offset = first_row as usize * width as usize;
                assembled.pixels_mut()[offset..offset + band.pixels().len()]
                    .copy_from_slice(band.pixels());
                Ok(())
            },
        )
        .expect("streaming render");
        (assembled, stats, starts)
    }

    /// The streaming path is an allocation strategy, not a second renderer.
    #[test]
    fn a_streamed_render_is_identical_to_the_tiled_render() {
        let store = store_with(&[
            ("a", source_image(W, H, 71)),
            ("b", source_image(W, H, 72)),
            ("c", source_image(W, H, 73)),
        ]);
        for seed in 1..=10u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let document = random_document(&mut rng, &["a", "b", "c"]);
            let resolved = store
                .resolve(&document.referenced_pixel_ids(), false)
                .unwrap();
            let tiled = render_document_tiled(
                &document,
                &resolved,
                RenderOptions {
                    scale: 1.0,
                    cancel: None,
                },
                64,
            )
            .unwrap()
            .0;
            for threads in [1, 4] {
                let (streamed, _, _) = collect(&document, &store, 64, threads);
                assert_eq!(
                    tiled.pixels(),
                    streamed.pixels(),
                    "seed {seed} streamed differently on {threads} threads"
                );
            }
        }
    }

    /// A halo makes neighbouring bands overlap in what they read but not in
    /// what they write, which is exactly where a streaming bug would appear.
    #[test]
    fn a_streamed_blur_is_identical_to_the_tiled_blur() {
        let store = store_with(&[("a", source_image(W, H, 74))]);
        let adjustment = Layer {
            content: LayerContent::Adjustment {
                operation: Box::new(EditOperation::GaussianBlur { radius: 4.0 }),
            },
            ..pixel_layer("blur", "unused")
        };
        let document = document(vec![pixel_layer("base", "a"), adjustment]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let tiled = render_document_tiled(
            &document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
            64,
        )
        .unwrap()
        .0;
        let (streamed, stats, _) = collect(&document, &store, 64, 4);
        assert!(stats.haloed_tiles > 0, "no tile was given a halo");
        assert_eq!(tiled.pixels(), streamed.pixels());
    }

    /// Bands have to arrive top to bottom and cover the image exactly once,
    /// because an encoder consuming them cannot seek backwards.
    #[test]
    fn bands_arrive_in_order_and_cover_the_image_exactly_once() {
        let store = store_with(&[("a", source_image(W, H, 75))]);
        let document = document(vec![pixel_layer("base", "a")]);
        let (_, _, starts) = collect(&document, &store, 64, 4);
        let expected: Vec<u32> = (0..H).step_by(64).collect();
        assert_eq!(starts, expected, "bands did not arrive in row order");
    }

    /// An untileable document still has to produce the whole image, once.
    #[test]
    fn an_untileable_document_streams_as_a_single_full_frame_band() {
        let store = store_with(&[("a", source_image(W, H, 76))]);
        let adjustment = Layer {
            content: LayerContent::Adjustment {
                operation: Box::new(EditOperation::AutoWhiteBalance { strength: 0.7 }),
            },
            ..pixel_layer("awb", "unused")
        };
        let document = document(vec![pixel_layer("base", "a"), adjustment]);
        let (assembled, stats, starts) = collect(&document, &store, 64, 4);
        assert!(stats.fell_back_to_full_frame);
        assert_eq!(starts, vec![0], "a fallback should emit exactly one band");
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let reference = super::super::linear::render_document_float(
            &document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
        )
        .unwrap();
        assert_eq!(reference.pixels(), assembled.pixels());
    }

    /// A failing sink must stop the render rather than being called again.
    #[test]
    fn an_error_from_the_sink_stops_the_render() {
        let store = store_with(&[("a", source_image(W, H, 77))]);
        let document = document(vec![pixel_layer("base", "a")]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let mut calls = 0;
        let result = render_document_streaming(
            &document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
            64,
            4,
            &mut |_, _| {
                calls += 1;
                Err(AppError::ExportFailure)
            },
        );
        assert!(matches!(result, Err(AppError::ExportFailure)));
        assert_eq!(calls, 1, "the render continued after the sink failed");
    }

    /// Peak memory is the point of the streaming path, so measure it.
    #[test]
    fn no_band_is_anywhere_near_the_size_of_the_frame() {
        let store = store_with(&[("a", source_image(W, H, 78))]);
        let document = document(vec![pixel_layer("base", "a")]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let frame_bytes = u64::from(W) * u64::from(H) * 16;
        let mut largest = 0u64;
        render_document_streaming(
            &document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
            64,
            4,
            &mut |_, band| {
                largest = largest.max(u64::from(band.width()) * u64::from(band.height()) * 16);
                Ok(())
            },
        )
        .unwrap();
        assert!(
            largest * 3 < frame_bytes,
            "the largest band was {largest} bytes against a {frame_bytes} byte frame"
        );
    }

    /// Renders without any cache: the answer every cached render is judged by.
    fn uncached(document: &LayerDocument, store: &LayerPixelStore, tile_size: u32) -> FloatImage {
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .expect("resolve");
        render_document_tiled(
            document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
            tile_size,
        )
        .expect("uncached render")
        .0
    }

    fn cached(
        document: &LayerDocument,
        store: &LayerPixelStore,
        tile_size: u32,
        cache: &TileCache,
    ) -> (FloatImage, TiledStats) {
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .expect("resolve");
        render_document_tiled_cached(
            document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
            tile_size,
            0,
            Some(cache),
        )
        .expect("cached render")
    }

    /// A small opaque square, so a layer can be moved around a large canvas and
    /// only affect part of it.
    fn small_source(size: u32, seed: u32) -> FloatImage {
        let mut image = FloatImage::blank(size, size, FloatRgba::TRANSPARENT).unwrap();
        for (index, pixel) in image.pixels_mut().iter_mut().enumerate() {
            let n = ((index as u32 * 37 + seed * 11) % 251) as f32 / 251.0;
            *pixel = FloatRgba {
                red: n,
                green: 1.0 - n,
                blue: 0.5,
                alpha: 1.0,
            };
        }
        image
    }

    /// Caching must never change a render, only skip work. Rendering the same
    /// document twice through a warm cache has to give the same bytes.
    #[test]
    fn a_warm_cache_returns_the_same_pixels_it_stored() {
        let store = store_with(&[("a", source_image(W, H, 81)), ("b", source_image(W, H, 82))]);
        let cache = TileCache::default();
        for seed in 1..=10u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let document = random_document(&mut rng, &["a", "b"]);
            let reference = uncached(&document, &store, 64);
            let (cold, cold_stats) = cached(&document, &store, 64, &cache);
            assert_eq!(reference.pixels(), cold.pixels(), "seed {seed} cold");
            let (warm, warm_stats) = cached(&document, &store, 64, &cache);
            assert_eq!(reference.pixels(), warm.pixels(), "seed {seed} warm");
            if !cold_stats.fell_back_to_full_frame {
                assert_eq!(
                    warm_stats.cached_tiles, warm_stats.tiles,
                    "seed {seed} recomputed tiles nothing had changed"
                );
                assert_eq!(
                    cold_stats.cached_tiles, 0,
                    "seed {seed} hit on a cold cache"
                );
            }
        }
    }

    /// Pixel identifiers are local to one store. Opening a new document resets
    /// generated names back to `px1`, so content has to participate in the key
    /// or the first preview of the new document can show the previous photo.
    #[test]
    fn a_reused_pixel_identifier_cannot_serve_a_previous_documents_tiles() {
        let first_store = store_with(&[("px1", source_image(W, H, 901))]);
        let second_store = store_with(&[("px1", source_image(W, H, 902))]);
        let document = document(vec![pixel_layer("base", "px1")]);
        let cache = TileCache::default();

        let _ = cached(&document, &first_store, 64, &cache);
        let expected = uncached(&document, &second_store, 64);
        let (actual, stats) = cached(&document, &second_store, 64, &cache);

        assert_eq!(
            stats.cached_tiles, 0,
            "new source content reused stale tiles"
        );
        assert_eq!(expected.pixels(), actual.pixels());
    }

    /// The staleness test. A cache that reuses a tile it should have dropped
    /// shows the user pixels from a document they no longer have, and every
    /// other guarantee in this module rests on that not happening.
    #[test]
    fn no_mutation_of_a_document_can_produce_a_stale_tile() {
        let store = store_with(&[
            ("a", source_image(W, H, 83)),
            ("b", source_image(W, H, 84)),
            ("small", small_source(48, 3)),
        ]);
        let cache = TileCache::default();
        // Counted so the test cannot pass by never reusing anything: a cache
        // that always missed would satisfy every assertion below for free.
        let mut reuse = 0u64;
        for seed in 1..=30u64 {
            let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let mut document = random_document(&mut rng, &["a", "b", "small"]);
            // Warm the cache on the document as it stands.
            let _ = cached(&document, &store, 64, &cache);

            for step in 0..6 {
                mutate(&mut document, &mut rng);
                if document.validate().is_err() {
                    continue;
                }
                let reference = uncached(&document, &store, 64);
                let (actual, stats) = cached(&document, &store, 64, &cache);
                reuse += stats.cached_tiles;
                let worst = reference
                    .pixels()
                    .iter()
                    .zip(actual.pixels())
                    .map(|(a, b)| {
                        (a.red - b.red)
                            .abs()
                            .max((a.green - b.green).abs())
                            .max((a.blue - b.blue).abs())
                            .max((a.alpha - b.alpha).abs())
                    })
                    .fold(0.0f32, f32::max);
                assert_eq!(
                    worst, 0.0,
                    "seed {seed} step {step} served a stale tile differing by {worst}"
                );
            }
        }
        assert!(
            reuse > 0,
            "no tile was ever reused, so this proved nothing about staleness"
        );
    }

    /// Changes a document the way a user would: move something, hide it, dial
    /// an opacity, retype a blend mode, add or remove a layer.
    fn mutate(document: &mut LayerDocument, rng: &mut Rng) {
        let count = document.layers.len();
        if count == 0 {
            document.layers.push(pixel_layer("added", "a"));
            return;
        }
        let index = rng.below(count as u64) as usize;
        match rng.below(8) {
            0 => document.layers[index].visible = !document.layers[index].visible,
            1 => document.layers[index].opacity = rng.unit(),
            2 => {
                let mode = BlendMode::ALL[rng.below(16) as usize];
                // A pass-through group is only allowed to be Normal, so respect
                // the invariant the document validator enforces.
                let pass_through = matches!(
                    document.layers[index].content,
                    LayerContent::Group {
                        isolated: false,
                        ..
                    }
                );
                document.layers[index].blend_mode = if pass_through {
                    BlendMode::Normal
                } else {
                    mode
                };
            }
            3 => {
                document.layers[index].transform.translate_x = rng.unit() * 200.0 - 100.0;
                document.layers[index].transform.translate_y = rng.unit() * 200.0 - 100.0;
            }
            4 => document.layers[index].transform.rotation_degrees = rng.unit() * 40.0 - 20.0,
            5 => {
                let mut added = pixel_layer(&format!("added{}", rng.next() % 1000), "small");
                added.transform.translate_x = rng.unit() * 200.0;
                added.transform.translate_y = rng.unit() * 150.0;
                document.layers.insert(index, added);
            }
            6 => {
                document.layers.remove(index);
            }
            _ => {
                document.layers[index].mask = if document.layers[index].mask.is_some() {
                    None
                } else {
                    Some(half_mask())
                };
            }
        }
    }

    /// The cache has to be worth having: moving a small layer must leave the
    /// tiles it never touched reusable, not invalidate the whole frame.
    #[test]
    fn moving_a_small_layer_leaves_distant_tiles_cached() {
        let store = store_with(&[
            ("a", source_image(W, H, 85)),
            ("small", small_source(40, 7)),
        ]);
        let mut spot = pixel_layer("spot", "small");
        spot.transform.translate_x = 10.0;
        spot.transform.translate_y = 10.0;
        let mut document = document(vec![pixel_layer("base", "a"), spot]);

        let cache = TileCache::default();
        let (_, first) = cached(&document, &store, 64, &cache);
        assert_eq!(first.cached_tiles, 0);

        // Nudge the small layer a few pixels: only the tiles it covers change.
        document.layers[1].transform.translate_x = 14.0;
        let (actual, second) = cached(&document, &store, 64, &cache);
        assert_eq!(
            uncached(&document, &store, 64).pixels(),
            actual.pixels(),
            "moving a small layer produced the wrong image"
        );
        assert!(
            second.cached_tiles > second.tiles / 2,
            "only {} of {} tiles were reused after moving a 40px layer",
            second.cached_tiles,
            second.tiles
        );
        assert!(
            second.cached_tiles < second.tiles,
            "the tiles the layer moved across were not invalidated"
        );
    }

    /// An adjustment layer rewrites everything beneath it, so changing one has
    /// to invalidate the whole frame however small its parameter change was.
    #[test]
    fn changing_an_adjustment_invalidates_every_tile() {
        let store = store_with(&[("a", source_image(W, H, 86))]);
        let mut adjustment = pixel_layer("bright", "unused");
        adjustment.content = LayerContent::Adjustment {
            operation: Box::new(EditOperation::Brightness { amount: 0.1 }),
        };
        let mut document = document(vec![pixel_layer("base", "a"), adjustment]);
        let cache = TileCache::default();
        let _ = cached(&document, &store, 64, &cache);

        document.layers[1].content = LayerContent::Adjustment {
            operation: Box::new(EditOperation::Brightness { amount: 0.2 }),
        };
        let (actual, stats) = cached(&document, &store, 64, &cache);
        assert_eq!(stats.cached_tiles, 0, "an adjustment change reused tiles");
        assert_eq!(uncached(&document, &store, 64).pixels(), actual.pixels());
    }

    /// A cache too small to hold a frame must still render it correctly; it
    /// simply thrashes. Correctness cannot depend on the budget.
    #[test]
    fn a_cache_too_small_to_help_still_renders_correctly() {
        let store = store_with(&[("a", source_image(W, H, 87))]);
        let document = document(vec![pixel_layer("base", "a")]);
        let cache = TileCache::with_capacity(64 * 64 * 16 * 2);
        let reference = uncached(&document, &store, 64);
        for _ in 0..3 {
            let (actual, _) = cached(&document, &store, 64, &cache);
            assert_eq!(reference.pixels(), actual.pixels());
        }
        let stats = cache.stats();
        assert!(stats.evictions > 0, "a tiny cache never evicted");
        assert!(stats.bytes <= stats.capacity_bytes);
    }

    /// The streaming export path shares the cache, and must agree with the
    /// full-frame path tile for tile.
    #[test]
    fn a_streamed_render_may_use_tiles_the_full_frame_render_cached() {
        let store = store_with(&[("a", source_image(W, H, 88))]);
        let document = document(vec![pixel_layer("base", "a")]);
        let resolved = store
            .resolve(&document.referenced_pixel_ids(), false)
            .unwrap();
        let cache = TileCache::default();
        let (reference, _) = cached(&document, &store, 64, &cache);

        let mut assembled = FloatImage::blank(W, H, FloatRgba::TRANSPARENT).unwrap();
        let stats = render_document_streaming_cached(
            &document,
            &resolved,
            RenderOptions {
                scale: 1.0,
                cancel: None,
            },
            64,
            0,
            Some(&cache),
            &mut |first_row, band| {
                let offset = first_row as usize * W as usize;
                assembled.pixels_mut()[offset..offset + band.pixels().len()]
                    .copy_from_slice(band.pixels());
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            stats.cached_tiles, stats.tiles,
            "the streamed render recomputed tiles the full-frame render had cached"
        );
        assert_eq!(reference.pixels(), assembled.pixels());
    }
}
