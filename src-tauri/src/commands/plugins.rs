use std::path::PathBuf;

use tauri::{AppHandle, Emitter, State};

use crate::core::library::VideoInfo;
use crate::ipc::SCHEMA_VERSION;
use crate::plugins::manager::{PluginInfo, PluginManager, PluginRunFinished, EVENT_PLUGIN_RESULT};
use crate::plugins::orchestrator::{OrchestrationRequest, OrchestrationResult, PluginOrchestrator};
use crate::plugins::runtime::PluginProcessError;
use crate::plugins::PluginError;

pub fn plugin_dirs(app: &AppHandle) -> Vec<PathBuf> {
    let custom_dirs = crate::core::settings::load_settings(app)
        .map(|settings| settings.custom_plugin_dirs)
        .unwrap_or_default();
    PluginManager::plugin_dirs(app, &custom_dirs)
}

#[tauri::command]
pub async fn get_plugins(
    app_handle: AppHandle,
    manager: State<'_, PluginManager>,
) -> Result<Vec<PluginInfo>, String> {
    let settings = crate::core::settings::load_settings(&app_handle).ok();
    Ok(manager.list(settings.as_ref()))
}

/// 只允许打开已保存的自定义搜索根，或应用自有的默认目录。
#[tauri::command]
pub async fn open_plugin_directory(
    app_handle: AppHandle,
    index: Option<usize>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let settings =
            crate::core::settings::load_settings(&app_handle).map_err(|error| error.to_string())?;
        let path = select_plugin_directory(
            crate::core::get_default_plugins_dir(&app_handle),
            &settings.custom_plugin_dirs,
            index,
        )?;
        if index.is_none() {
            std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        }
        if !path.is_dir() {
            return Err("插件目录不存在或不是目录，请检查已保存的设置".to_string());
        }
        open_directory(&path)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn select_plugin_directory(
    default_directory: PathBuf,
    custom_directories: &[PathBuf],
    index: Option<usize>,
) -> Result<PathBuf, String> {
    let path = match index {
        None => default_directory,
        Some(index) => custom_directories
            .get(index)
            .cloned()
            .ok_or_else(|| "自定义插件目录索引无效，请先保存设置".to_string())?,
    };
    if !path.is_absolute() {
        return Err("插件目录必须是绝对路径".to_string());
    }
    Ok(path)
}

fn open_directory(path: &std::path::Path) -> Result<(), String> {
    use std::process::{Command, Stdio};

    #[cfg(windows)]
    let mut command = {
        let windows =
            std::env::var_os("WINDIR").ok_or_else(|| "无法找到 Windows 资源管理器".to_string())?;
        Command::new(PathBuf::from(windows).join("explorer.exe"))
    };
    #[cfg(target_os = "macos")]
    let mut command = Command::new("/usr/bin/open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = Command::new("xdg-open");
    let mut child = command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("无法打开插件目录：{error}"))?;
    // Some system openers stay alive until their window closes. Do not hold the
    // IPC call open for that window's lifetime, but still reap the child.
    std::thread::spawn(move || {
        if let Err(error) = child.wait() {
            log::warn!("Failed to reap plugin-directory opener: {error}");
        }
    });
    Ok(())
}

/// 重新扫描插件目录（热重载）：停掉全部进程、重新发现、清理孤儿环境。
#[tauri::command]
pub async fn reload_plugins(
    app_handle: AppHandle,
    manager: State<'_, PluginManager>,
) -> Result<Vec<PluginInfo>, String> {
    let dirs = plugin_dirs(&app_handle);
    let manager = manager.inner().clone();
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        manager.reload(&dirs);
        let settings = crate::core::settings::load_settings(&app).ok();
        manager.list(settings.as_ref())
    })
    .await
    .map_err(|e| e.to_string())
}

/// 单插件环境建置结果（供前端展示）
#[derive(serde::Serialize)]
pub struct EnvRebuildResult {
    pub name: String,
    pub venv_dir: Option<String>,
    pub error: Option<String>,
}

/// 重建所有 Python 插件的虚拟环境（设置页「重建所有插件环境」按钮）。
/// 单插件失败不影响其他；返回每个插件的建置结果。
#[tauri::command]
pub async fn rebuild_plugin_environments(
    app_handle: AppHandle,
    manager: State<'_, PluginManager>,
) -> Result<Vec<EnvRebuildResult>, String> {
    let manager = manager.inner().clone();
    let app = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let settings =
            crate::core::settings::load_settings(&app).map_err(|e| format!("读取设置失败：{e}"))?;
        let results = manager.rebuild_environments(&settings);
        Ok(results
            .into_iter()
            .map(|(name, r)| EnvRebuildResult {
                name,
                venv_dir: r.as_ref().ok().map(|p| p.to_string_lossy().to_string()),
                error: r.err(),
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
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

#[cfg(test)]
mod directory_tests {
    use super::*;

    #[test]
    fn opens_only_saved_roots_or_app_owned_default() {
        let root = tempfile::tempdir().unwrap();
        let default = root.path().join("app-data/plugins");
        let first = root.path().join("custom-first");
        let second = root.path().join("custom-second");
        let custom = vec![first.clone(), second.clone()];
        assert_eq!(
            select_plugin_directory(default.clone(), &custom, None).unwrap(),
            default
        );
        assert_eq!(
            select_plugin_directory(default.clone(), &custom, Some(0)).unwrap(),
            first
        );
        assert_eq!(
            select_plugin_directory(default.clone(), &custom, Some(1)).unwrap(),
            second
        );
        assert!(select_plugin_directory(default.clone(), &custom, Some(2)).is_err());
        assert!(select_plugin_directory(default, &[], Some(0)).is_err());
        assert!(select_plugin_directory(PathBuf::from("relative"), &[], None).is_err());
    }
}
