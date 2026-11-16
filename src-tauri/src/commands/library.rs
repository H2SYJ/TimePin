use std::path::PathBuf;
use tauri::AppHandle;
use crate::core::library::{VideoInfo, VideoMetadata, ImportResult};

#[tauri::command]
pub async fn import_video(app_handle: AppHandle, source_path: PathBuf) -> Result<ImportResult, String> {
    crate::core::library::import_video(&app_handle, source_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_videos(app_handle: AppHandle) -> Result<Vec<VideoInfo>, String> {
    crate::core::library::get_videos(&app_handle)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_video(app_handle: AppHandle, id: String) -> Result<(), String> {
    crate::core::library::delete_video(&app_handle, &id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_video_metadata(
    app_handle: AppHandle,
    path: PathBuf,
) -> Result<VideoMetadata, String> {
    crate::core::library::get_video_metadata(&app_handle, &path).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn generate_thumbnail(
    app_handle: AppHandle,
    video_path: PathBuf,
    video_id: String,
) -> Result<PathBuf, String> {
    let duration = crate::core::library::get_video_by_id(&app_handle, &video_id)
        .map(|video| video.duration)
        .unwrap_or(0.0);

    crate::core::library::generate_thumbnail(&app_handle, &video_path, &video_id, duration)
        .map_err(|e| e.to_string())
}