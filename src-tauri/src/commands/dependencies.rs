use tauri::{AppHandle, Emitter};

use crate::core::dependencies::{
    self, DependencyInstallProgress, DependencyInstallStage, InstalledDependencies,
    INSTALL_PROGRESS_EVENT,
};

#[tauri::command]
pub async fn install_dependencies(
    app_handle: AppHandle,
    task_id: String,
) -> Result<InstalledDependencies, String> {
    let worker_handle = app_handle.clone();
    let worker_task_id = task_id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        dependencies::install_dependencies(&worker_handle, &worker_task_id, |event| {
            if let Err(error) = worker_handle.emit(INSTALL_PROGRESS_EVENT, &event) {
                log::warn!("Cannot emit dependency installation progress: {error}");
            }
        })
    })
    .await;
    match result {
        Ok(result) => result,
        Err(error) => {
            let message = format!("依赖安装任务异常终止，请重试：{error}");
            let event = DependencyInstallProgress {
                schema_version: "1.0.0".to_string(),
                task_id,
                stage: DependencyInstallStage::Failed,
                progress: 0.0,
                message: "依赖安装失败".to_string(),
                error: Some(message.clone()),
            };
            if let Err(error) = app_handle.emit(INSTALL_PROGRESS_EVENT, &event) {
                log::warn!("Cannot emit dependency installation failure: {error}");
            }
            Err(message)
        }
    }
}
