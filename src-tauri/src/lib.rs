#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            crate::core::init(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            crate::commands::settings::get_settings,
            crate::commands::settings::save_settings,
            crate::commands::library::import_video,
            crate::commands::library::get_videos,
            crate::commands::library::delete_video,
            crate::commands::library::get_video_metadata,
            crate::commands::library::generate_thumbnail,
            crate::commands::formats::generate_txt_chapters,
            crate::commands::formats::generate_csv_markers,
            crate::commands::formats::export_segments,
        ])
        .run(tauri::generate_context!())
        .expect("error while building tauri application");
}

pub mod commands;
pub mod core;
pub mod ffmpeg;
pub mod formats;
pub mod ipc;
pub mod plugins;