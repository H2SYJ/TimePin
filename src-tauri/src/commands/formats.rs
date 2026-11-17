use crate::formats::Mark;
use std::path::PathBuf;
use tauri::AppHandle;

#[tauri::command]
pub async fn generate_txt_chapters(
    marks: Vec<Mark>,
    video_duration: f64,
) -> Result<String, String> {
    crate::formats::generate_txt_chapters(&marks, video_duration).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn generate_csv_markers(marks: Vec<Mark>, video_duration: f64) -> Result<String, String> {
    crate::formats::generate_csv_markers(&marks, video_duration).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn export_segments(
    app_handle: AppHandle,
    video_path: PathBuf,
    marks: Vec<Mark>,
    output_dir: PathBuf,
    export_mode: crate::core::settings::ExportMode,
) -> Result<Vec<PathBuf>, String> {
    // 导出内部做多个同步 Command::output()，必须 spawn_blocking
    // 以免长时间占用 tokio worker、导致 UI 失去响应。
    tauri::async_runtime::spawn_blocking(move || {
        crate::ffmpeg::export_segments(&app_handle, &video_path, &marks, &output_dir, export_mode)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
