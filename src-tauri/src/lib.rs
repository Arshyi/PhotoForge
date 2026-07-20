mod application;
mod commands;
pub mod components;
pub mod domain;
mod error;
mod image_processing;
pub mod infrastructure;

use application::AppState;
use commands::{
    analyze_image, cancel_batch, cancel_ollama_plan, compare_planners, create_point_operation,
    discover_models, export_image, export_with_profile, export_workflow, generate_edit_plan,
    generate_histogram, generate_ollama_plan, get_batch_status, get_component_diagnostics,
    get_component_snapshot, get_ollama_diagnostics, import_workflow, inspect_image_pixel,
    measure_component_performance, open_image, preview_batch_workflow, refresh_ollama_models,
    render_preview, scan_plugins, select_planner_provider, select_restoration_engine,
    start_batch_workflow, test_ollama_connection, update_component_configuration,
    validate_guided_plan, validate_ollama_json, validate_plugin_manifest,
    validate_shortcut_bindings, validate_workflow_json, validate_workspace_layout,
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
            discover_models,
            scan_plugins,
            validate_plugin_manifest,
            generate_edit_plan,
            validate_guided_plan,
            test_ollama_connection,
            refresh_ollama_models,
            generate_ollama_plan,
            cancel_ollama_plan,
            validate_ollama_json,
            compare_planners,
            get_ollama_diagnostics,
            export_image,
            generate_histogram,
            inspect_image_pixel,
            create_point_operation,
            validate_workflow_json,
            import_workflow,
            export_workflow,
            preview_batch_workflow,
            start_batch_workflow,
            get_batch_status,
            cancel_batch,
            validate_workspace_layout,
            validate_shortcut_bindings,
            export_with_profile
        ])
        .run(tauri::generate_context!())
        .expect("PhotoForge failed to start");
}
