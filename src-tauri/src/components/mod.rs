mod engines;
mod ollama;
mod performance;
mod planners;
mod registry;
mod runtime;

pub use engines::{
    DeterministicEngine, FutureEngine, OnnxRestorationEngine, RealEsrganEngine,
    RestorationEngineFactory,
};
pub use ollama::{
    deterministic_planner_prompt, validate_ollama_endpoint, validate_ollama_plan, OllamaClient,
    OllamaGeneration, PlanValidationFailure, MAX_OLLAMA_PROMPT_CHARS, OLLAMA_PROVIDER_VERSION,
};
pub use performance::measure_component_overhead;
pub use planners::{FuturePlanner, OllamaPlanner, OpenAIPlanner, PlannerFactory};
pub use registry::ComponentRegistry;
pub use runtime::initialize_with_timeout;

/// The planner-facing name for the unified runtime component registry.
pub type PlannerRegistry = ComponentRegistry;
