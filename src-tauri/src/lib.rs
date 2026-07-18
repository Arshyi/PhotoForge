mod application;
mod commands;
pub mod domain;
mod error;
mod image_processing;
pub mod infrastructure;

use application::AppState;
use commands::{export_image, open_image, render_preview};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            open_image,
            render_preview,
            export_image
        ])
        .run(tauri::generate_context!())
        .expect("PhotoForge failed to start");
}
