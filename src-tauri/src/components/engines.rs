use crate::domain::{EditOperation, EngineProvider, RestorationCapabilities, RestorationEngine};
use crate::error::AppError;
use crate::image_processing::apply_pipeline;
use image::DynamicImage;

#[derive(Debug, Default, Clone, Copy)]
pub struct DeterministicEngine;

impl RestorationEngine for DeterministicEngine {
    fn provider(&self) -> EngineProvider {
        EngineProvider::Deterministic
    }

    fn capabilities(&self) -> RestorationCapabilities {
        RestorationCapabilities {
            supports_restoration: true,
            supports_neural_models: false,
            requires_model: false,
            offline: true,
            preserves_alpha: true,
            max_input_megapixels: 40.0,
        }
    }

    fn process(
        &self,
        image: &DynamicImage,
        operations: &[EditOperation],
    ) -> Result<DynamicImage, AppError> {
        apply_pipeline(image, operations)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OnnxRestorationEngine;

#[derive(Debug, Default, Clone, Copy)]
pub struct RealEsrganEngine;

#[derive(Debug, Default, Clone, Copy)]
pub struct FutureEngine;

macro_rules! unavailable_engine {
    ($engine:ty, $provider:expr) => {
        impl RestorationEngine for $engine {
            fn provider(&self) -> EngineProvider {
                $provider
            }

            fn capabilities(&self) -> RestorationCapabilities {
                RestorationCapabilities {
                    supports_restoration: true,
                    supports_neural_models: true,
                    requires_model: true,
                    offline: true,
                    preserves_alpha: false,
                    max_input_megapixels: 0.0,
                }
            }

            fn process(
                &self,
                _image: &DynamicImage,
                _operations: &[EditOperation],
            ) -> Result<DynamicImage, AppError> {
                Err(AppError::RestorationEngineNotInstalled)
            }
        }
    };
}

unavailable_engine!(OnnxRestorationEngine, EngineProvider::Onnx);
unavailable_engine!(RealEsrganEngine, EngineProvider::RealEsrgan);
unavailable_engine!(FutureEngine, EngineProvider::Future);

pub struct RestorationEngineFactory;

impl RestorationEngineFactory {
    pub fn create(provider: EngineProvider) -> Box<dyn RestorationEngine> {
        match provider {
            EngineProvider::Deterministic => Box::new(DeterministicEngine),
            EngineProvider::Onnx => Box::new(OnnxRestorationEngine),
            EngineProvider::RealEsrgan => Box::new(RealEsrganEngine),
            EngineProvider::Future => Box::new(FutureEngine),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, Rgba, RgbaImage};

    fn image() -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([20, 30, 40, 128])))
    }

    #[test]
    fn deterministic_engine_processes_existing_operations() {
        let engine = RestorationEngineFactory::create(EngineProvider::Deterministic);
        let processed = engine
            .process(&image(), &[EditOperation::Brightness { amount: 0.1 }])
            .unwrap()
            .to_rgba8();
        assert!(processed.get_pixel(0, 0).0[0] > 20);
        assert_eq!(processed.get_pixel(0, 0).0[3], 128);
    }

    #[test]
    fn deterministic_engine_handles_empty_pipeline() {
        let engine = RestorationEngineFactory::create(EngineProvider::Deterministic);
        assert_eq!(
            engine.process(&image(), &[]).unwrap().to_rgba8(),
            image().to_rgba8()
        );
    }

    #[test]
    fn deterministic_engine_capabilities_match_phase_three() {
        let capabilities = DeterministicEngine.capabilities();
        assert!(capabilities.supports_restoration);
        assert!(capabilities.offline);
        assert!(capabilities.preserves_alpha);
        assert!(!capabilities.requires_model);
        assert!(!capabilities.supports_neural_models);
        assert_eq!(capabilities.max_input_megapixels, 40.0);
    }

    #[test]
    fn onnx_placeholder_reports_not_installed() {
        let engine = RestorationEngineFactory::create(EngineProvider::Onnx);
        assert!(matches!(
            engine.process(&image(), &[]),
            Err(AppError::RestorationEngineNotInstalled)
        ));
    }

    #[test]
    fn real_esrgan_placeholder_reports_not_installed() {
        let engine = RestorationEngineFactory::create(EngineProvider::RealEsrgan);
        assert!(matches!(
            engine.process(&image(), &[]),
            Err(AppError::RestorationEngineNotInstalled)
        ));
    }

    #[test]
    fn future_engine_placeholder_reports_not_installed() {
        let engine = RestorationEngineFactory::create(EngineProvider::Future);
        assert!(matches!(
            engine.process(&image(), &[]),
            Err(AppError::RestorationEngineNotInstalled)
        ));
    }

    #[test]
    fn unavailable_engines_require_models_and_neural_support() {
        for provider in [
            EngineProvider::Onnx,
            EngineProvider::RealEsrgan,
            EngineProvider::Future,
        ] {
            let capabilities = RestorationEngineFactory::create(provider).capabilities();
            assert!(capabilities.requires_model);
            assert!(capabilities.supports_neural_models);
        }
    }

    #[test]
    fn factory_preserves_engine_provider_identity() {
        for provider in EngineProvider::ALL {
            assert_eq!(
                RestorationEngineFactory::create(provider).provider(),
                provider
            );
        }
    }
}
