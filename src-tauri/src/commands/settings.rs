use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::core::settings::Settings;
use crate::core::tools::{self, DependencyReport};
use crate::ipc::SCHEMA_VERSION;

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
pub async fn save_settings(app_handle: AppHandle, settings: Settings) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        settings.validate()?;
        tools::validate_configured_tools(&settings)?;
        crate::core::settings::save_settings(&app_handle, settings).map_err(|e| e.to_string())
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
        default_plugin_directory: crate::core::get_plugin_dir(&app_handle),
        install_directory: crate::core::dependencies::install_directory(&app_handle),
    })
}
