use crate::core::marks::MarksDocument;
use crate::formats::Mark;
use tauri::AppHandle;

#[tauri::command]
pub async fn get_marks(app_handle: AppHandle, video_id: String) -> Result<MarksDocument, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::core::marks::get_marks(&app_handle, &video_id).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn save_marks(
    app_handle: AppHandle,
    video_id: String,
    marks: Vec<Mark>,
) -> Result<MarksDocument, String> {
    tauri::async_runtime::spawn_blocking(move || {
        crate::core::marks::save_marks(&app_handle, &video_id, marks).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}
