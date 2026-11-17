use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::core::settings::Settings;
use crate::core::tools::{self, DependencyReport};
use crate::ipc::SCHEMA_VERSION;
use crate::plugins::manager::PluginManager;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdvancedSettingsInfo {
    pub schema_version: String,
    pub default_plugin_directory: PathBuf,
    /// 依赖（ffmpeg/ffprobe）安装根目录。
    pub install_directory: PathBuf,
}

#[tauri::command]
pub async fn get_settings(app_handle: AppHandle) -> Result<Settings, String> {
    crate::core::settings::load_settings(&app_handle).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_settings(
    app_handle: AppHandle,
    manager: tauri::State<'_, PluginManager>,
    settings: Settings,
) -> Result<(), String> {
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        settings.validate()?;
        tools::validate_configured_tools(&settings)?;

        // 只在目录列表改变时热重载，避免普通设置保存停止正在运行的插件。
        crate::core::settings::save_settings_with_plugin_dirs(
            &app_handle,
            settings,
            |custom_dirs| {
                let dirs = PluginManager::plugin_dirs(&app_handle, custom_dirs);
                manager.reload(&dirs);
            },
        )
        .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn validate_dependencies(settings: Settings) -> Result<DependencyReport, String> {
    tauri::async_runtime::spawn_blocking(move || tools::check_dependencies(&settings))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_advanced_settings_info(
    app_handle: AppHandle,
) -> Result<AdvancedSettingsInfo, String> {
    Ok(AdvancedSettingsInfo {
        schema_version: SCHEMA_VERSION.to_string(),
        default_plugin_directory: crate::core::get_default_plugins_dir(&app_handle),
        install_directory: crate::core::get_app_data_dir(&app_handle)
            .join("tools")
            .join("ffmpeg"),
    })
}
