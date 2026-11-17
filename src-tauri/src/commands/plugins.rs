use std::path::PathBuf;

use tauri::{AppHandle, Emitter, State};

use crate::core::library::VideoInfo;
use crate::ipc::SCHEMA_VERSION;
use crate::plugins::manager::{PluginInfo, PluginManager, PluginRunFinished, EVENT_PLUGIN_RESULT};
use crate::plugins::orchestrator::{OrchestrationRequest, OrchestrationResult, PluginOrchestrator};
use crate::plugins::runtime::PluginProcessError;
use crate::plugins::PluginError;

fn plugin_dirs(app: &AppHandle) -> Vec<PathBuf> {
    let mut dirs = PluginManager::builtin_plugin_dirs(app);
    if let Ok(settings) = crate::core::settings::load_settings(app) {
        dirs.extend(settings.custom_plugin_dirs);
    }
    dirs
}

#[tauri::command]
pub async fn get_plugins(manager: State<'_, PluginManager>) -> Result<Vec<PluginInfo>, String> {
    Ok(manager.list())
}

/// 重新扫描插件目录（热重载）：停掉全部进程、重新发现。
#[tauri::command]
pub async fn reload_plugins(
    app_handle: AppHandle,
    manager: State<'_, PluginManager>,
) -> Result<Vec<PluginInfo>, String> {
    let dirs = plugin_dirs(&app_handle);
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        manager.reload(&dirs);
        manager.list()
    })
    .await
    .map_err(|e| e.to_string())
}

/// 在指定视频上运行插件。
///
/// 同步执行（spawn_blocking），进度通过 `plugin://progress` 事件推送；
/// 完成后发出 `plugin://result` 并返回 `PluginResult`。
#[tauri::command]
pub async fn run_plugin(
    app_handle: AppHandle,
    manager: State<'_, PluginManager>,
    plugin_name: String,
    video: VideoInfo,
    params: serde_json::Value,
    output_dir: PathBuf,
) -> Result<crate::plugins::PluginResult, String> {
    let manager = manager.inner().clone();
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = manager.run_plugin(&plugin_name, &video, params, output_dir);
        let (outcome, error, marks_count, task_id) = match &result {
            Ok((task_id, r)) => ("ok".to_string(), None, r.marks.len(), task_id.clone()),
            Err(e) => {
                let outcome = match e {
                    PluginError::Process(PluginProcessError::ProcessExited) => "crashed",
                    _ => "error",
                };
                (outcome.to_string(), Some(e.to_string()), 0, String::new())
            }
        };
        let _ = app.emit(
            EVENT_PLUGIN_RESULT,
            PluginRunFinished {
                schema_version: SCHEMA_VERSION.to_string(),
                task_id,
                plugin_name: plugin_name.clone(),
                video_id: video.id.clone(),
                outcome,
                error,
                marks_count,
            },
        );
        result.map(|(_, r)| r).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 停止插件进程
#[tauri::command]
pub async fn stop_plugin(
    manager: State<'_, PluginManager>,
    plugin_name: String,
) -> Result<(), String> {
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || manager.stop(&plugin_name))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// 取消插件当前任务（尽力而为）
#[tauri::command]
pub async fn cancel_plugin_run(
    manager: State<'_, PluginManager>,
    plugin_name: String,
    task_id: String,
) -> Result<(), String> {
    manager.cancel(&plugin_name, &task_id);
    Ok(())
}

/// M4：多插件编排——在一次调用里跨多个视频运行多个插件。
///
/// - `sequential`：按 `plugin_names` 顺序依次执行，单插件失败不阻塞后续；
/// - `parallel`：多线程 fan-out，`max_concurrency` 限制并发度；
/// - 每个 (plugin, video) 失败都被捕获到 `PluginRunOutcome`，不会终止整批；
/// - 完成后对每个视频执行 `merge_marks` 去重。
///
/// 进度通过 `plugin://orchestration` 事件推送（stage/completed/total），
/// 单插件进度仍走 `plugin://status` / `plugin://progress` / `plugin://log`。
#[tauri::command]
pub async fn run_plugins(
    app_handle: AppHandle,
    manager: State<'_, PluginManager>,
    request: OrchestrationRequest,
) -> Result<OrchestrationResult, String> {
    let manager = manager.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let orchestrator = PluginOrchestrator::new(manager, app_handle);
        orchestrator.run(request)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())
}
