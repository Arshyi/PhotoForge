use crate::components::{ComponentRegistry, PlannerFactory, RestorationEngineFactory};
use crate::domain::{
    ColorCastEstimate, ComponentPerformanceMetrics, ImageQualityAnalysis, PlannerProvider,
};
use std::hint::black_box;
use std::time::Instant;

pub fn measure_component_overhead(
    registry: &ComponentRegistry,
    requested_samples: u32,
) -> ComponentPerformanceMetrics {
    let samples = requested_samples.clamp(1, 2_000);

    let started = Instant::now();
    for _ in 0..samples {
        black_box(registry.active_planner());
        black_box(registry.active_engine());
    }
    let registry_lookup_average_ns = average_ns(started.elapsed().as_nanos(), samples);

    let planner = PlannerFactory::create(PlannerProvider::Rule);
    let analysis = diagnostic_analysis();
    let started = Instant::now();
    for _ in 0..samples {
        black_box(planner.create_plan("brighten and sharpen", &analysis).ok());
    }
    let planner_dispatch_average_ns = average_ns(started.elapsed().as_nanos(), samples);

    let planner_provider = registry.active_planner();
    let engine_provider = registry.active_engine();
    let started = Instant::now();
    for _ in 0..samples {
        black_box(PlannerFactory::create(planner_provider));
        black_box(RestorationEngineFactory::create(engine_provider));
    }
    let component_factory_average_ns = average_ns(started.elapsed().as_nanos(), samples);

    ComponentPerformanceMetrics {
        samples,
        registry_lookup_average_ns,
        planner_dispatch_average_ns,
        component_factory_average_ns,
        note: "Local diagnostic only; no model loading, network connection, or plugin execution occurred."
            .into(),
    }
}

fn average_ns(total_ns: u128, samples: u32) -> u64 {
    (total_ns / u128::from(samples)).min(u128::from(u64::MAX)) as u64
}

fn diagnostic_analysis() -> ImageQualityAnalysis {
    ImageQualityAnalysis {
        average_luminance: 0.42,
        luminance_spread: 0.46,
        estimated_color_cast: ColorCastEstimate {
            dominant: "neutral".into(),
            red_bias: 0.0,
            green_bias: 0.0,
            blue_bias: 0.0,
        },
        estimated_noise: 0.08,
        estimated_sharpness: 0.12,
        estimated_local_contrast: 0.10,
        edge_density: 0.15,
        white_background_ratio: 0.05,
        likely_document: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn performance_measurement_reports_all_categories() {
        let metrics = measure_component_overhead(&ComponentRegistry::default(), 20);
        assert_eq!(metrics.samples, 20);
        assert!(metrics.planner_dispatch_average_ns > 0);
        assert!(metrics.note.contains("no model loading"));
    }

    #[test]
    fn performance_samples_are_bounded() {
        assert_eq!(
            measure_component_overhead(&ComponentRegistry::default(), 0).samples,
            1
        );
        assert_eq!(
            measure_component_overhead(&ComponentRegistry::default(), u32::MAX).samples,
            2_000
        );
    }

    #[test]
    fn diagnostic_analysis_is_valid_for_the_rule_planner() {
        let planner = PlannerFactory::create(PlannerProvider::Rule);
        assert!(planner
            .create_plan("brighten and sharpen", &diagnostic_analysis())
            .is_ok());
    }
}
