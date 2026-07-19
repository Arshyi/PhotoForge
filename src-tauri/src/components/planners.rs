use crate::domain::{
    EditPlan, EditPlanner, ImageAnalysis, PlannerCapabilities, PlannerProvider, RulePlanner,
};
use crate::error::AppError;

#[derive(Debug, Default, Clone, Copy)]
pub struct OllamaPlanner;

#[derive(Debug, Default, Clone, Copy)]
pub struct OpenAIPlanner;

#[derive(Debug, Default, Clone, Copy)]
pub struct FuturePlanner;

impl EditPlanner for OllamaPlanner {
    fn provider(&self) -> PlannerProvider {
        PlannerProvider::Ollama
    }

    fn capabilities(&self) -> PlannerCapabilities {
        PlannerCapabilities {
            supports_guided_editing: true,
            supports_reasoning: true,
            requires_model: true,
            offline: true,
        }
    }

    fn create_plan(&self, _request: &str, _analysis: &ImageAnalysis) -> Result<EditPlan, AppError> {
        Err(AppError::ComponentInitializationFailure(
            "Ollama uses the cancellable asynchronous local-provider command".into(),
        ))
    }
}

macro_rules! unavailable_planner {
    ($planner:ty, $provider:expr, $offline:expr) => {
        impl EditPlanner for $planner {
            fn provider(&self) -> PlannerProvider {
                $provider
            }

            fn capabilities(&self) -> PlannerCapabilities {
                PlannerCapabilities {
                    supports_guided_editing: true,
                    supports_reasoning: true,
                    requires_model: true,
                    offline: $offline,
                }
            }

            fn create_plan(
                &self,
                _request: &str,
                _analysis: &ImageAnalysis,
            ) -> Result<EditPlan, AppError> {
                Err(AppError::PlannerNotInstalled)
            }
        }
    };
}

unavailable_planner!(OpenAIPlanner, PlannerProvider::OpenAi, false);
unavailable_planner!(FuturePlanner, PlannerProvider::Future, false);

pub struct PlannerFactory;

impl PlannerFactory {
    pub fn create(provider: PlannerProvider) -> Box<dyn EditPlanner> {
        match provider {
            PlannerProvider::Rule => Box::new(RulePlanner),
            PlannerProvider::Ollama => Box::new(OllamaPlanner),
            PlannerProvider::OpenAi => Box::new(OpenAIPlanner),
            PlannerProvider::Future => Box::new(FuturePlanner),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ColorCastEstimate, ImageQualityAnalysis};

    fn analysis() -> ImageQualityAnalysis {
        ImageQualityAnalysis {
            average_luminance: 0.5,
            luminance_spread: 0.4,
            estimated_color_cast: ColorCastEstimate {
                dominant: "neutral".into(),
                red_bias: 0.0,
                green_bias: 0.0,
                blue_bias: 0.0,
            },
            estimated_noise: 0.1,
            estimated_sharpness: 0.1,
            estimated_local_contrast: 0.1,
            edge_density: 0.1,
            white_background_ratio: 0.0,
            likely_document: false,
        }
    }

    #[test]
    fn rule_factory_creates_functional_planner() {
        let planner = PlannerFactory::create(PlannerProvider::Rule);
        assert!(planner.create_plan("reduce noise", &analysis()).is_ok());
    }

    #[test]
    fn rule_factory_reports_offline_capabilities() {
        let planner = PlannerFactory::create(PlannerProvider::Rule);
        let capabilities = planner.capabilities();
        assert!(capabilities.supports_guided_editing);
        assert!(capabilities.offline);
        assert!(!capabilities.requires_model);
        assert!(!capabilities.supports_reasoning);
    }

    #[test]
    fn ollama_factory_routes_planning_to_async_adapter() {
        let planner = PlannerFactory::create(PlannerProvider::Ollama);
        assert!(matches!(
            planner.create_plan("test", &analysis()),
            Err(AppError::ComponentInitializationFailure(_))
        ));
    }

    #[test]
    fn openai_placeholder_reports_not_installed() {
        let planner = PlannerFactory::create(PlannerProvider::OpenAi);
        assert!(matches!(
            planner.create_plan("test", &analysis()),
            Err(AppError::PlannerNotInstalled)
        ));
    }

    #[test]
    fn future_placeholder_reports_not_installed() {
        let planner = PlannerFactory::create(PlannerProvider::Future);
        assert!(matches!(
            planner.create_plan("test", &analysis()),
            Err(AppError::PlannerNotInstalled)
        ));
    }

    #[test]
    fn unavailable_planners_require_models() {
        for provider in [
            PlannerProvider::Ollama,
            PlannerProvider::OpenAi,
            PlannerProvider::Future,
        ] {
            assert!(
                PlannerFactory::create(provider)
                    .capabilities()
                    .requires_model
            );
        }
    }

    #[test]
    fn factory_preserves_provider_identity() {
        for provider in PlannerProvider::ALL {
            assert_eq!(PlannerFactory::create(provider).provider(), provider);
        }
    }

    #[test]
    fn ollama_is_declared_offline_but_never_contacted() {
        let planner = PlannerFactory::create(PlannerProvider::Ollama);
        assert!(planner.capabilities().offline);
        assert!(matches!(
            planner.create_plan("test", &analysis()),
            Err(AppError::ComponentInitializationFailure(_))
        ));
    }
}
