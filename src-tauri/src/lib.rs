use tauri::Manager;
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            crate::core::init(app.handle())?;

            // 插件管理器：扫描内置 plugins/ + 设置中的 customPluginDirs。
            // 只发现与校验，进程在执行首个任务时懒启动。
            let plugin_dirs =
                crate::plugins::manager::PluginManager::builtin_plugin_dirs(app.handle())
                    .into_iter()
                    .chain(
                        crate::core::settings::load_settings(app.handle())
                            .map(|s| s.custom_plugin_dirs)
                            .unwrap_or_default(),
                    )
                    .collect::<Vec<_>>();
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
            crate::commands::settings::get_settings,
            crate::commands::settings::save_settings,
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
