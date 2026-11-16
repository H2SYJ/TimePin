use crate::core::settings::Settings;
use tauri::AppHandle;

#[tauri::command]
pub async fn get_settings(app_handle: AppHandle) -> Result<Settings, String> {
    crate::core::settings::load_settings(&app_handle).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_settings(app_handle: AppHandle, settings: Settings) -> Result<(), String> {
    crate::core::settings::save_settings(&app_handle, settings).map_err(|e| e.to_string())
}
