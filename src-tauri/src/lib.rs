mod application;
mod commands;
pub mod domain;
mod error;
mod image_processing;
pub mod infrastructure;

use application::AppState;
use commands::{
    analyze_image, export_image, generate_edit_plan, open_image, render_preview,
    validate_guided_plan,
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
            generate_edit_plan,
            validate_guided_plan,
            export_image
        ])
        .run(tauri::generate_context!())
        .expect("PhotoForge failed to start");
}
