use tauri::Manager;
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            #[cfg(desktop)]
            {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())?;
                app.handle().plugin(tauri_plugin_process::init())?;
            }

            crate::core::init(app.handle())?;

            // Discover bundled, app-owned and saved custom roots without starting processes.
            let custom_dirs = crate::core::settings::load_settings(app.handle())
                .map(|settings| settings.custom_plugin_dirs)
                .unwrap_or_default();
            let plugin_dirs =
                crate::plugins::manager::PluginManager::plugin_dirs(app.handle(), &custom_dirs);
            let manager =
                crate::plugins::manager::PluginManager::discover(app.handle(), &plugin_dirs);
            app.manage(manager);

            // 识别服务状态
            let recognition_state =
                crate::commands::recognition::RecognitionState::new(app.handle());
            app.manage(recognition_state);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            crate::commands::updater::get_updater_config,
            crate::commands::settings::get_settings,
            crate::commands::settings::save_settings,
            crate::commands::settings::validate_dependencies,
            crate::commands::settings::get_advanced_settings_info,
            crate::commands::plugins::open_plugin_directory,
            crate::commands::dependencies::install_dependencies,
            crate::commands::library::import_video,
            crate::commands::library::get_videos,
            crate::commands::library::delete_video,
            crate::commands::library::get_video_metadata,
            crate::commands::library::generate_thumbnail,
            crate::commands::marks::get_marks,
            crate::commands::marks::save_marks,
            crate::commands::formats::generate_txt_chapters,
            crate::commands::formats::generate_csv_markers,
            crate::commands::formats::export_segments,
            crate::commands::plugins::get_plugins,
            crate::commands::plugins::reload_plugins,
            crate::commands::plugins::run_plugin,
            crate::commands::plugins::run_plugins,
            crate::commands::plugins::stop_plugin,
            crate::commands::plugins::cancel_plugin_run,
            // M3 识别服务命令
            crate::commands::recognition::recognition_template_match,
            crate::commands::recognition::recognition_ocr,
            crate::commands::recognition::recognition_detect,
            crate::commands::recognition::recognition_preview,
            crate::commands::recognition::recognition_roi_save,
            crate::commands::recognition::recognition_roi_load,
            crate::commands::recognition::recognition_roi_list,
            crate::commands::recognition::recognition_configure,
            // GitHub 插件发现命令
            crate::commands::github_plugins::discover_github_plugins,
            crate::commands::github_plugins::install_github_plugin,
        ])
        .run(tauri::generate_context!())
        .expect("error while building tauri application");
}

pub mod commands;
pub mod core;
pub mod ffmpeg;
pub mod formats;
pub mod frame_service;
pub mod ipc;
pub mod plugins;
