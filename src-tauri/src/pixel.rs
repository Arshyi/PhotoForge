//! Typed image boundaries. A byte buffer never implicitly becomes linear RGB.
use crate::color::FloatImage;
use crate::error::AppError;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DocumentPrecision {
    /// Missing in older projects: preserve their encoded-space byte renderer.
    #[default]
    LegacySrgb8,
    LinearSrgbF32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleRepresentation {
    Unorm8,
    Float32,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferFunction {
    Srgb,
    Linear,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelColorSpace {
    SrgbD65,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaRepresentation {
    Straight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PixelFormat {
    pub samples: SampleRepresentation,
    pub transfer: TransferFunction,
    pub color_space: PixelColorSpace,
    pub alpha: AlphaRepresentation,
}

impl PixelFormat {
    pub const SRGBA8: Self = Self {
        samples: SampleRepresentation::Unorm8,
        transfer: TransferFunction::Srgb,
        color_space: PixelColorSpace::SrgbD65,
        alpha: AlphaRepresentation::Straight,
    };
    pub const LINEAR_RGBA_F32: Self = Self {
        samples: SampleRepresentation::Float32,
        transfer: TransferFunction::Linear,
        color_space: PixelColorSpace::SrgbD65,
        alpha: AlphaRepresentation::Straight,
    };
}

/// Immutable buffers are shared by live documents and undo/redo through Arc.
#[derive(Debug, Clone)]
pub enum PixelBuffer {
    EncodedSrgba8(Arc<RgbaImage>),
    LinearRgbaF32(Arc<FloatImage>),
}

impl PixelBuffer {
    pub fn format(&self) -> PixelFormat {
        match self {
            Self::EncodedSrgba8(_) => PixelFormat::SRGBA8,
            Self::LinearRgbaF32(_) => PixelFormat::LINEAR_RGBA_F32,
        }
    }
    pub fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::EncodedSrgba8(image) => image.dimensions(),
            Self::LinearRgbaF32(image) => image.dimensions(),
        }
    }
    pub fn bytes(&self) -> u64 {
        let (width, height) = self.dimensions();
        u64::from(width)
            * u64::from(height)
            * match self {
                Self::EncodedSrgba8(_) => 4,
                Self::LinearRgbaF32(_) => 16,
            }
    }
    pub fn linear(&self) -> Result<Arc<FloatImage>, AppError> {
        match self {
            Self::EncodedSrgba8(image) => Ok(Arc::new(FloatImage::from_rgba8(image)?)),
            Self::LinearRgbaF32(image) => Ok(Arc::clone(image)),
        }
    }
    /// Deliberate legacy/display boundary. Final float renders use `linear`.
    pub fn encoded8(&self) -> Arc<RgbaImage> {
        match self {
            Self::EncodedSrgba8(image) => Arc::clone(image),
            Self::LinearRgbaF32(image) => Arc::new(image.to_rgba8()),
        }
    }
}

impl From<RgbaImage> for PixelBuffer {
    fn from(value: RgbaImage) -> Self {
        Self::EncodedSrgba8(Arc::new(value))
    }
}
impl From<FloatImage> for PixelBuffer {
    fn from(value: FloatImage) -> Self {
        Self::LinearRgbaF32(Arc::new(value))
    }
}

impl From<crate::color::ColorPipelineError> for AppError {
    fn from(error: crate::color::ColorPipelineError) -> Self {
        Self::ColorPipeline(error.to_string())
    }
}
