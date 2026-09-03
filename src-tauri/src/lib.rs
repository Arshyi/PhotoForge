mod application;
mod commands;
pub mod components;
pub mod domain;
pub mod error;
mod image_processing;
pub mod infrastructure;
pub mod layers;
pub mod mask;
mod network_policy;

use application::AppState;
use commands::{
    analyze_image, apply_operations_to_layer, cancel_batch, cancel_mask_operation,
    cancel_ollama_plan, color_range_selection, compare_planners, compose_selection_masks,
    create_layer_mask, create_layer_pixels, create_point_operation, discard_recovery_snapshot,
    discover_models, export_image, export_layer_composite, export_mask_file, export_mask_png,
    export_with_profile, export_workflow, flatten_layer_document, generate_edit_plan,
    generate_histogram, generate_ollama_plan, get_batch_status, get_component_diagnostics,
    get_component_snapshot, get_mask_progress, get_ollama_diagnostics, import_layer_image,
    import_mask_file, import_mask_png, import_workflow, inspect_image_pixel,
    inspect_selection_mask, layer_mask_from_selection, layer_store_report, list_recovery_snapshots,
    load_layer_project, magic_wand_selection, measure_component_performance, merge_layer_pixels,
    open_image, plan_layer_workflow, preview_batch_workflow, rasterize_layer_transform,
    rasterize_selection, refine_selection_mask, refresh_ollama_models, remap_selection_masks,
    render_layer_composite, render_layer_thumbnail, render_preview, restore_recovery_snapshot,
    retain_layer_pixels, save_layer_project, scan_plugins, select_planner_provider,
    select_restoration_engine, selection_from_layer_mask, start_batch_workflow,
    test_ollama_connection, transform_selection_mask, update_component_configuration,
    validate_guided_plan, validate_layer_document, validate_mask_snapshot, validate_ollama_json,
    validate_plugin_manifest, validate_shortcut_bindings, validate_workflow_json,
    validate_workspace_layout, write_recovery_snapshot,
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
