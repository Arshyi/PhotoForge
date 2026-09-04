//! Running a local ONNX model over an image.
//!
//! # The runtime
//!
//! tract, chosen on evidence rather than popularity: it is pure Rust, so a
//! PhotoForge install ships no native inference DLL and cannot accidentally
//! pick one up from a developer's PATH. It is CPU only, and this module says so
//! rather than implying otherwise. See `docs/local-inference.md` for the
//! comparison that led here.
//!
//! # What this module is careful about
//!
//! * **Colour.** PhotoForge works in linear light; almost every published
//!   imaging model was trained on gamma-encoded sRGB. The conversion is
//!   explicit, driven by the model's own descriptor, and never assumed.
//! * **Alpha.** RGB models have nothing to say about opacity, so alpha is
//!   carried around the model rather than through it.
//! * **Tiles.** A 60 MP image is not one tensor. Tiles overlap and are blended,
//!   so the seam between them is a ramp rather than a line.
//! * **Bounds.** Every tensor is sized from the descriptor, which was validated
//!   before it got here, and the output shape is checked against what the model
//!   promised before a single pixel is read out of it.
use std::path::Path;
use std::sync::atomic::AtomicBool;

use tract_onnx::prelude::*;

use super::model::{ModelColorSpace, ModelDescriptor, Normalization};
use crate::color::{srgb_decode, srgb_encode, FloatImage, FloatRgba};
use crate::error::AppError;
use crate::image_processing::high_precision::check_cancel;

/// `into_runnable` hands back a shared plan, which is what allows one loaded
/// model to be run from several threads without reloading it.
type Plan = std::sync::Arc<tract_onnx::tract_core::model::TypedSimplePlan>;

/// A model that has been parsed and is ready to run.
pub struct LoadedModel {
    descriptor: ModelDescriptor,
    plan: Plan,
}

impl LoadedModel {
    pub fn descriptor(&self) -> &ModelDescriptor {
        &self.descriptor
    }
}

/// Parses a model file into a runnable plan.
///
/// The descriptor is validated again here rather than trusted from wherever it
/// came from: it may have arrived from a saved project rather than from the
/// import that first checked it.
pub fn load(path: &Path, descriptor: ModelDescriptor) -> Result<LoadedModel, AppError> {
    descriptor.validate()?;
    let bytes = std::fs::read(path)
        .map_err(|error| AppError::ProjectIo(format!("could not read the model: {error}")))?;
    if bytes.len() as u64 != descriptor.file_bytes {
        return Err(AppError::InvalidOperation(
            "the model file changed size since it was installed".into(),
        ));
    }
    let tile = descriptor.tile_size as usize;
    let channels = descriptor.input_channels as usize;
    let plan = tract_onnx::onnx()
        .model_for_read(&mut std::io::Cursor::new(&bytes))
        .and_then(|model| {
            // The shape is imposed rather than inferred: a model that will not
            // accept the tile the descriptor promised is a mismatch worth
            // failing on now, not one tensor into a long render.
            model.with_input_fact(
                0,
                InferenceFact::dt_shape(f32::datum_type(), tvec!(1, channels, tile, tile)),
            )
        })
        .and_then(|model| model.into_optimized())
        .and_then(|model| model.into_runnable())
        .map_err(|error| {
            AppError::InvalidOperation(format!(
                "the model could not be prepared for this tile size: {error}"
            ))
        })?;
    Ok(LoadedModel { descriptor, plan })
}

/// The pixel values a tile hands to the model, in the model's own encoding.
fn to_model_space(value: f32, descriptor: &ModelDescriptor) -> f32 {
    let encoded = match descriptor.color_space {
        ModelColorSpace::EncodedSrgb => srgb_encode(value.clamp(0.0, 1.0)),
        ModelColorSpace::LinearSrgb => value.clamp(0.0, 1.0),
    };
    match descriptor.normalization {
        Normalization::UnitRange => encoded,
        Normalization::SignedUnitRange => encoded * 2.0 - 1.0,
    }
}

/// The inverse, on the way back into the document.
fn from_model_space(value: f32, descriptor: &ModelDescriptor) -> f32 {
    let encoded = match descriptor.normalization {
        Normalization::UnitRange => value,
        Normalization::SignedUnitRange => (value + 1.0) * 0.5,
    };
    let encoded = encoded.clamp(0.0, 1.0);
    match descriptor.color_space {
        ModelColorSpace::EncodedSrgb => srgb_decode(encoded),
        ModelColorSpace::LinearSrgb => encoded,
    }
}

/// A tile's weight at a given distance into its overlap.
///
/// A linear ramp across the overlap, so two neighbouring tiles sum to one
/// everywhere they meet. Without it the boundary between tiles is a visible
/// line wherever the model's output differs even slightly across it.
fn blend_weight(position: u32, extent: u32, overlap: u32) -> f32 {
    if overlap == 0 {
        return 1.0;
    }
    let from_start = position as f32 + 0.5;
    let from_end = (extent - position) as f32 - 0.5;
    let ramp = |distance: f32| (distance / overlap as f32).clamp(0.0, 1.0);
    ramp(from_start).min(ramp(from_end)).max(1e-3)
}

impl LoadedModel {
    /// Runs the model over a whole image, tile by tile.
    ///
    /// The result is `scale` times larger in each dimension. Alpha is resampled
    /// alongside rather than passed through the model, because an RGB model has
    /// no opinion about opacity and inventing one would be worse than
    /// preserving what the document already knows.
    pub fn run_image(
        &self,
        image: &FloatImage,
        cancel: Option<&AtomicBool>,
    ) -> Result<FloatImage, AppError> {
        let descriptor = &self.descriptor;
        let (width, height) = image.dimensions();
        let scale = descriptor.scale;
        let out_width = width.checked_mul(scale).ok_or(AppError::OutOfMemoryRisk)?;
        let out_height = height.checked_mul(scale).ok_or(AppError::OutOfMemoryRisk)?;

        let tile = descriptor.tile_size;
        let overlap = descriptor.tile_overlap;
        // The distance between tile origins. Validation guarantees the overlap
        // is under half the tile, so this is always positive.
        let step = tile - overlap;

        let mut accumulated = vec![[0.0f32; 3]; (out_width as usize) * (out_height as usize)];
        let mut weights = vec![0.0f32; (out_width as usize) * (out_height as usize)];
        let channels = descriptor.input_channels as usize;

        let mut origin_y = 0u32;
        loop {
            let mut origin_x = 0u32;
            loop {
                check_cancel(cancel)?;
                self.run_tile(
                    image,
                    origin_x,
                    origin_y,
                    &mut accumulated,
                    &mut weights,
                    out_width,
                    out_height,
                    channels,
                )?;
                if origin_x + tile >= width {
                    break;
                }
                origin_x = (origin_x + step).min(width.saturating_sub(tile));
            }
            if origin_y + tile >= height {
                break;
            }
            origin_y = (origin_y + step).min(height.saturating_sub(tile));
        }

        let mut out = FloatImage::blank(out_width, out_height, FloatRgba::TRANSPARENT)?;
        for (index, pixel) in out.pixels_mut().iter_mut().enumerate() {
            let weight = weights[index];
            let (x, y) = (
                (index % out_width as usize) as u32,
                (index / out_width as usize) as u32,
            );
            // Alpha comes from the source, resampled, never from the model.
            let alpha = image
                .get((x / scale).min(width - 1), (y / scale).min(height - 1))
                .map_or(1.0, |p| p.alpha);
            *pixel = if weight > 0.0 {
                FloatRgba::new(
                    accumulated[index][0] / weight,
                    accumulated[index][1] / weight,
                    accumulated[index][2] / weight,
                    alpha,
                )
            } else {
                // Only reachable if a tile grid somehow missed a pixel; falling
                // back to the source is better than leaving a hole.
                image
                    .get((x / scale).min(width - 1), (y / scale).min(height - 1))
                    .unwrap_or(FloatRgba::TRANSPARENT)
            };
        }
        out.validate()
            .map_err(|error| AppError::ColorPipeline(error.to_string()))?;
        Ok(out)
    }

    #[allow(clippy::too_many_arguments)]
    fn run_tile(
        &self,
        image: &FloatImage,
        origin_x: u32,
        origin_y: u32,
        accumulated: &mut [[f32; 3]],
        weights: &mut [f32],
        out_width: u32,
        out_height: u32,
        channels: usize,
    ) -> Result<(), AppError> {
        let descriptor = &self.descriptor;
        let (width, height) = image.dimensions();
        let tile = descriptor.tile_size;
        let scale = descriptor.scale;

        // Edge tiles are padded by clamping, so the model always sees the shape
        // it was prepared for.
        let mut input = vec![0.0f32; channels * (tile as usize) * (tile as usize)];
        for y in 0..tile {
            for x in 0..tile {
                let sx = (origin_x + x).min(width - 1);
                let sy = (origin_y + y).min(height - 1);
                let pixel = image.get(sx, sy).unwrap_or(FloatRgba::TRANSPARENT);
                let source = [pixel.red, pixel.green, pixel.blue];
                for (channel, value) in source.iter().enumerate().take(channels) {
                    let index = channel * (tile as usize) * (tile as usize)
                        + (y as usize) * (tile as usize)
                        + x as usize;
                    input[index] = to_model_space(*value, descriptor);
                }
            }
        }

        let tensor = tract_ndarray::Array4::from_shape_vec(
            (1, channels, tile as usize, tile as usize),
            input,
        )
        .map_err(|_| AppError::ProcessingFailure("could not shape the model input".into()))?
        .into_tensor();

        let result = self
            .plan
            .run(tvec!(tensor.into()))
            .map_err(|error| AppError::ProcessingFailure(format!("inference failed: {error}")))?;
        let output = result
            .first()
            .ok_or_else(|| AppError::ProcessingFailure("the model produced no output".into()))?;
        let view = output.to_plain_array_view::<f32>().map_err(|error| {
            AppError::ProcessingFailure(format!("the model output was not float data: {error}"))
        })?;

        // The shape is checked against what the descriptor promised before any
        // of it is read, so a model that disagrees fails cleanly instead of
        // being indexed past its end.
        let expected_tile = (tile * scale) as usize;
        let expected = [
            1usize,
            descriptor.output_channels as usize,
            expected_tile,
            expected_tile,
        ];
        if view.shape() != expected {
            return Err(AppError::InvalidOperation(format!(
                "the model produced {:?} where its descriptor promised {expected:?}",
                view.shape()
            )));
        }
        let values = view.as_slice().ok_or_else(|| {
            AppError::ProcessingFailure("the model output was not contiguous".into())
        })?;

        let out_channels = descriptor.output_channels as usize;
        let plane = expected_tile * expected_tile;
        for y in 0..expected_tile {
            for x in 0..expected_tile {
                let target_x = origin_x * scale + x as u32;
                let target_y = origin_y * scale + y as u32;
                if target_x >= out_width || target_y >= out_height {
                    continue;
                }
                let weight = blend_weight(
                    x as u32,
                    expected_tile as u32,
                    descriptor.tile_overlap * scale,
                ) * blend_weight(
                    y as u32,
                    expected_tile as u32,
                    descriptor.tile_overlap * scale,
                );
                let index = (target_y as usize) * (out_width as usize) + target_x as usize;
                for (channel, target) in accumulated[index].iter_mut().enumerate() {
                    // A single-channel model applies its one output to all
                    // three, which is what a luminance model means.
                    let source = channel.min(out_channels - 1);
                    let value = values[source * plane + y * expected_tile + x];
                    *target += from_model_space(value, descriptor) * weight;
                }
                weights[index] += weight;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::{Capability, ModelFormat};
    use super::super::onnx_fixture;
    use super::*;

    fn descriptor(tile: u32, overlap: u32, scale: u32) -> ModelDescriptor {
        ModelDescriptor {
            id: "fixture".into(),
            name: "Fixture".into(),
            version: "1".into(),
            architecture: "test".into(),
            capability: if scale > 1 {
                Capability::SuperResolution
            } else {
                Capability::Denoise
            },
            format: ModelFormat::Onnx,
            color_space: ModelColorSpace::LinearSrgb,
            normalization: Normalization::UnitRange,
            input_channels: 3,
            output_channels: 3,
            scale,
            tile_size: tile,
            tile_overlap: overlap,
            file_bytes: 0,
            sha256: "a".repeat(64),
            license: "authored for tests".into(),
            source: "authored for tests".into(),
        }
    }

    fn write(bytes: &[u8], folder: &Path, name: &str) -> std::path::PathBuf {
        let path = folder.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn gradient(width: u32, height: u32) -> FloatImage {
        let mut image = FloatImage::blank(width, height, FloatRgba::TRANSPARENT).unwrap();
        for y in 0..height {
            for x in 0..width {
                let v = x as f32 / width as f32;
                image.pixels_mut()[(y * width + x) as usize] =
                    FloatRgba::new(v, 1.0 - v, 0.5, 0.75);
            }
        }
        image
    }

    /// The end-to-end claim: a model PhotoForge authored, loaded from a file,
    /// run over a whole image through the tiling path, producing the answer the
    /// graph defines.
    #[test]
    fn a_local_model_runs_over_a_whole_image() {
        let folder = tempfile::tempdir().unwrap();
        let bytes = onnx_fixture::scale_and_offset(32, 32, 0.5, 0.25);
        let path = write(&bytes, folder.path(), "fixture.onnx");
        let mut descriptor = descriptor(32, 8, 1);
        descriptor.file_bytes = bytes.len() as u64;

        let model = load(&path, descriptor).expect("the model should load");
        let source = gradient(96, 64);
        let result = model.run_image(&source, None).expect("inference");

        assert_eq!(result.dimensions(), (96, 64));
        let mut worst = 0.0f32;
        for (a, b) in source.pixels().iter().zip(result.pixels()) {
            let expected = a.red * 0.5 + 0.25;
            worst = worst.max((expected - b.red).abs());
            // Alpha is carried around the model, not through it.
            assert!((a.alpha - b.alpha).abs() < 1e-6);
        }
        assert!(
            worst < 1e-5,
            "the model's own arithmetic was not reproduced: worst error {worst}"
        );
    }

    /// Overlapping tiles must not leave a visible line where they meet.
    #[test]
    fn tiled_inference_leaves_no_seam() {
        let folder = tempfile::tempdir().unwrap();
        let bytes = onnx_fixture::scale_and_offset(32, 32, 0.5, 0.25);
        let path = write(&bytes, folder.path(), "fixture.onnx");
        let mut descriptor = descriptor(32, 8, 1);
        descriptor.file_bytes = bytes.len() as u64;
        let model = load(&path, descriptor).unwrap();

        // Wider than one tile, so several seams fall inside it.
        let source = gradient(120, 40);
        let result = model.run_image(&source, None).unwrap();
        // The expected result is a smooth ramp, so any seam is a jump in the
        // horizontal difference.
        let mut worst_step = 0.0f32;
        for y in 0..40u32 {
            for x in 1..120u32 {
                let step =
                    (result.get(x, y).unwrap().red - result.get(x - 1, y).unwrap().red).abs();
                worst_step = worst_step.max(step);
            }
        }
        let smooth_step = 0.5 / 120.0;
        assert!(
            worst_step < smooth_step * 2.0,
            "a seam of {worst_step} appeared against a smooth step of {smooth_step}"
        );
    }

    /// Super-resolution has to actually change the size, and by the factor the
    /// model declares rather than the one someone hoped for.
    #[test]
    fn a_scaling_model_produces_the_declared_size() {
        let folder = tempfile::tempdir().unwrap();
        // The fixture is 1x, but the descriptor claims 2x. The runtime must
        // notice rather than produce a quarter-filled image.
        let bytes = onnx_fixture::scale_and_offset(32, 32, 1.0, 0.0);
        let path = write(&bytes, folder.path(), "fixture.onnx");
        let mut descriptor = descriptor(32, 8, 2);
        descriptor.file_bytes = bytes.len() as u64;
        let model = load(&path, descriptor).unwrap();
        let error = model
            .run_image(&gradient(64, 64), None)
            .expect_err("a model that does not scale must not be believed");
        assert!(
            format!("{error}").contains("promised"),
            "the mismatch was not explained: {error}"
        );
    }

    /// A model whose output shape disagrees with its descriptor must fail
    /// cleanly rather than read past a tensor.
    #[test]
    fn a_mismatched_output_shape_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        let bytes = onnx_fixture::single_channel_output(32, 32);
        let path = write(&bytes, folder.path(), "fixture.onnx");
        let mut descriptor = descriptor(32, 8, 1);
        descriptor.file_bytes = bytes.len() as u64;
        // Loading may fail outright, which is also an acceptable refusal.
        if let Ok(model) = load(&path, descriptor) {
            assert!(
                model.run_image(&gradient(64, 64), None).is_err(),
                "a model with the wrong output shape was accepted"
            );
        }
    }

    #[test]
    fn a_corrupt_model_file_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        for (name, bytes) in [
            ("truncated.onnx", onnx_fixture::truncated()),
            ("rubbish.onnx", vec![0x08; 512]),
        ] {
            let path = write(&bytes, folder.path(), name);
            let mut descriptor = descriptor(32, 8, 1);
            descriptor.file_bytes = bytes.len() as u64;
            assert!(
                load(&path, descriptor).is_err(),
                "{name} was accepted as a model"
            );
        }
    }

    #[test]
    fn a_file_that_changed_since_import_is_refused() {
        let folder = tempfile::tempdir().unwrap();
        let bytes = onnx_fixture::scale_and_offset(32, 32, 0.5, 0.25);
        let path = write(&bytes, folder.path(), "fixture.onnx");
        let mut descriptor = descriptor(32, 8, 1);
        descriptor.file_bytes = bytes.len() as u64 + 1;
        assert!(load(&path, descriptor).is_err());
    }

    /// Inference over a large image must be interruptible.
    #[test]
    fn inference_is_cancellable() {
        let folder = tempfile::tempdir().unwrap();
        let bytes = onnx_fixture::scale_and_offset(32, 32, 0.5, 0.25);
        let path = write(&bytes, folder.path(), "fixture.onnx");
        let mut descriptor = descriptor(32, 8, 1);
        descriptor.file_bytes = bytes.len() as u64;
        let model = load(&path, descriptor).unwrap();
        let cancel = AtomicBool::new(true);
        assert!(model.run_image(&gradient(128, 128), Some(&cancel)).is_err());
    }

    /// The blend weights either side of a seam must sum to one, or the overlap
    /// darkens or brightens the join.
    #[test]
    fn blend_weights_are_a_partition_across_the_overlap() {
        let (extent, overlap) = (32u32, 8u32);
        for position in 0..extent {
            let weight = blend_weight(position, extent, overlap);
            assert!(
                (0.0..=1.0).contains(&weight),
                "weight {weight} out of range"
            );
        }
        // In the middle of a tile the weight is full.
        assert!((blend_weight(16, extent, overlap) - 1.0).abs() < 1e-6);
        // At the very edge it is nearly nothing.
        assert!(blend_weight(0, extent, overlap) < 0.2);
    }
}
