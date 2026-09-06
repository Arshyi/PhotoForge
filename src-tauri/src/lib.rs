// Public so the integration tests can drive the real Tauri command boundary —
// dispatch, argument deserialization, and managed state — instead of calling the
// underlying Rust functions directly. Nothing outside the app consumes them.
pub mod application;
pub mod color;
pub mod color_management;
pub mod commands;
pub mod components;
pub mod domain;
pub mod error;
#[cfg(feature = "gpu")]
pub mod gpu;
mod image_processing;
/// Optional local neural inference. Present in every build; the runtime and any
/// models are separately optional, and the editor is complete without both.
pub mod inference;
/// Vector geometry: paths, semantic shapes and float-coverage rasterisation.
pub mod vector;
pub use image_processing::high_precision;
/// Fixtures and metrics are public so benchmarks and integration tests can
/// score restoration against known-clean images rather than against opinion.
pub use image_processing::{fixtures, kernels as image_processing_kernels, metrics};
pub mod infrastructure;
pub mod layers;
pub mod mask;
mod network_policy;
pub mod pixel;
pub mod raw;
pub mod resources;

use application::AppState;
use commands::{
    analyze_image, apply_operations_to_layer, cancel_batch, cancel_mask_operation,
    cancel_ollama_plan, clear_render_cache, color_range_selection, compare_planners,
    compose_selection_masks, create_layer_mask, create_layer_pixels, create_point_operation,
    default_render_cache_budget, develop_raw_layer, discard_recovery_snapshot, discover_models,
    export_developed_png16, export_image, export_layer_composite, export_mask_file,
    export_mask_png, export_raw_layer_png16, export_with_profile, export_workflow,
    flatten_layer_document, generate_edit_plan, generate_histogram, generate_ollama_plan,
    get_batch_status, get_component_diagnostics, get_component_snapshot, get_mask_progress,
    get_ollama_diagnostics, get_render_backend_mode, import_inference_model, import_layer_image,
    import_mask_file, import_mask_png, import_workflow, inference_status, inspect_image_pixel,
    inspect_raw, inspect_selection_mask, layer_mask_from_selection, layer_store_report,
    list_recovery_snapshots, load_layer_project, magic_wand_selection,
    measure_component_performance, merge_layer_pixels, open_image, open_raw_image, open_raw_layer,
    plan_layer_workflow, preview_batch_workflow, rasterize_layer_transform, rasterize_selection,
    refine_selection_mask, refresh_ollama_models, relink_raw_source, remap_selection_masks,
    remove_inference_model, render_diagnostics, render_layer_composite, render_layer_thumbnail,
    render_preview, restore_recovery_snapshot, retain_layer_pixels, save_layer_project,
    scan_plugins, select_planner_provider, select_restoration_engine, selection_from_layer_mask,
    set_render_backend_mode, set_render_cache_budget, start_batch_workflow, test_ollama_connection,
    transform_selection_mask, update_component_configuration, validate_guided_plan,
    validate_layer_document, validate_mask_snapshot, validate_ollama_json,
    validate_plugin_manifest, validate_shortcut_bindings, validate_workflow_json,
    validate_workspace_layout, verify_raw_source, write_recovery_snapshot,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .setup(|app| {
            network_policy::create_main_window(app)?;
            Ok(())
        })
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
            render_diagnostics,
            inference_status,
            import_inference_model,
            remove_inference_model,
            get_render_backend_mode,
            set_render_backend_mode,
            clear_render_cache,
            set_render_cache_budget,
            default_render_cache_budget,
            export_image,
            export_developed_png16,
            generate_histogram,
            inspect_image_pixel,
            inspect_raw,
            open_raw_image,
            open_raw_layer,
            develop_raw_layer,
            verify_raw_source,
            relink_raw_source,
            export_raw_layer_png16,
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
            export_with_profile,
            rasterize_selection,
            transform_selection_mask,
            magic_wand_selection,
            color_range_selection,
            compose_selection_masks,
            remap_selection_masks,
            refine_selection_mask,
            cancel_mask_operation,
            get_mask_progress,
            inspect_selection_mask,
            validate_mask_snapshot,
            import_mask_file,
            export_mask_file,
            import_mask_png,
            export_mask_png,
            render_layer_composite,
            export_layer_composite,
            import_layer_image,
            create_layer_pixels,
            merge_layer_pixels,
            flatten_layer_document,
            rasterize_layer_transform,
            apply_operations_to_layer,
            render_layer_thumbnail,
            layer_mask_from_selection,
            selection_from_layer_mask,
            create_layer_mask,
            validate_layer_document,
            retain_layer_pixels,
            layer_store_report,
            save_layer_project,
            load_layer_project,
            write_recovery_snapshot,
            list_recovery_snapshots,
            restore_recovery_snapshot,
            discard_recovery_snapshot,
            plan_layer_workflow
        ])
        .run(tauri::generate_context!())
        .expect("PhotoForge failed to start");
}
