mod application;
mod commands;
pub mod components;
pub mod domain;
mod error;
mod image_processing;
pub mod infrastructure;

use application::AppState;
use commands::{
    analyze_image, discover_models, export_image, generate_edit_plan, get_component_diagnostics,
    get_component_snapshot, measure_component_performance, open_image, render_preview,
    scan_plugins, select_planner_provider, select_restoration_engine, test_planner_connection,
    update_component_configuration, validate_guided_plan, validate_plugin_manifest,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            open_image,
            render_preview,
            analyze_image,
            get_component_snapshot,
            get_component_diagnostics,
            measure_component_performance,
            select_planner_provider,
            select_restoration_engine,
            update_component_configuration,
            test_planner_connection,
            discover_models,
            scan_plugins,
            validate_plugin_manifest,
            generate_edit_plan,
            validate_guided_plan,
            export_image
        ])
        .run(tauri::generate_context!())
        .expect("PhotoForge failed to start");
}
