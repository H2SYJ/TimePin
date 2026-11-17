//! 插件管理器：发现 → 校验 → 启动 → 执行 → 回收
//!
//! 生命周期（对应任务要求）：
//! `discovered → validated → started → ready → running → stopped | crashed`
//!
//! - 发现：扫描内置 `plugins/` 目录 + 设置中的 `customPluginDirs`
//! - 校验：`PluginManifest::validate()`，不兼容直接标 `invalid`
//! - 启动：`PluginProcess::spawn()`（独立 Sidecar 进程 + 协议握手）
//! - 执行：`run_plugin()` 把执行丢到 tokio 任务中，UI 不阻塞；
//!   进度/日志通知通过 Tauri event 推送
//! - 崩溃恢复：进程意外退出标记 `crashed`，下次 `run_plugin` 自动重启

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use thiserror::Error;

use crate::formats::Mark;
use crate::frame_service::{FrameExtractRequest, FrameService};
use crate::ipc::SCHEMA_VERSION;

use super::manifest::{PluginCapability, PluginManifest};
use super::protocol::{
    LogParams, PluginNotification, PluginResult, ProgressParams, RunParams, RunVideoInfo,
    PROTOCOL_VERSION,
};
use super::runtime::{
    HostRequestHandler, NotificationHandler, PluginProcess, PluginProcessError,
    HOST_SERVICE_FRAMES_EXTRACT,
};

/// 插件生命周期事件名（前端 listen 用）
pub const EVENT_PLUGIN_STATUS: &str = "plugin://status";
pub const EVENT_PLUGIN_PROGRESS: &str = "plugin://progress";
pub const EVENT_PLUGIN_LOG: &str = "plugin://log";
pub const EVENT_PLUGIN_RESULT: &str = "plugin://result";

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("插件不存在：{0}")]
    NotFound(String),
    #[error("插件校验失败：{0}")]
    Invalid(String),
    #[error("插件缺少能力：{0:?}")]
    MissingCapability(PluginCapability),
    #[error(transparent)]
    Process(#[from] PluginProcessError),
    #[error("任务失败：{0}")]
    Task(String),
}

/// 前端可见的插件条目（`get_plugins` 返回）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub runtime: String,
    pub capabilities: Vec<String>,
    pub parameters_schema: serde_json::Value,
    pub dir: PathBuf,
    /// ready | invalid | crashed | running
    pub status: String,
    #[serde(default)]
    pub status_detail: Option<String>,
}

/// 一次执行完成后发给前端的 payload
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRunFinished {
    pub schema_version: String,
    pub task_id: String,
    pub plugin_name: String,
    pub video_id: String,
    /// ok | error | cancelled | crashed
    pub outcome: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub marks_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProcessState {
    /// 已启动、握手成功，可接受任务
    Ready,
    /// 上次执行崩溃/退出，需要重启
    Crashed,
    /// 正在执行任务
    Running,
}

struct ManagedPlugin {
    manifest: PluginManifest,
    dir: PathBuf,
    status: String,
    status_detail: Option<String>,
    state: ProcessState,
    /// Arc<Mutex> 包装：run_plugin 在锁外执行长 RPC，
    /// 锁内只做「取 Arc / 存新进程」的短操作，保证 cancel/stop 可达。
    process: Option<Arc<Mutex<PluginProcess>>>,
}

/// 插件管理器：持有所有已发现插件及其进程。
///
/// 用 `Mutex<HashMap>` 而非每插件独立锁：插件数量小（< 20），
/// 简单锁够用；执行路径里锁只覆盖「取进程指针」的短临界区，
/// 长阻塞的 RPC 在锁外进行（`PluginProcess` 用 `Arc<Mutex>` 共享）。
#[derive(Clone)]
pub struct PluginManager {
    app: AppHandle,
    plugins: Arc<Mutex<HashMap<String, Arc<Mutex<ManagedPlugin>>>>>,
}

type SharedPlugin = Arc<Mutex<ManagedPlugin>>;

impl PluginManager {
    /// 扫描目录并构建管理器；不会启动任何进程（懒启动）。
    pub fn discover(app: &AppHandle, plugin_dirs: &[PathBuf]) -> Self {
        let mut plugins = HashMap::new();
        for dir in plugin_dirs {
            if !dir.is_dir() {
                continue;
            }
            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let plugin_dir = entry.path();
                if !plugin_dir.is_dir() {
                    continue;
                }
                match PluginManifest::load(&plugin_dir) {
                    Ok(manifest) => {
                        let errors = manifest.validate();
                        let (status, detail) = if errors.is_empty() {
                            ("ready".to_string(), None)
                        } else {
                            ("invalid".to_string(), Some(errors.join("; ")))
                        };
                        let name = manifest.name.clone();
                        plugins.insert(
                            name.clone(),
                            Arc::new(Mutex::new(ManagedPlugin {
                                manifest,
                                dir: plugin_dir,
                                status,
                                status_detail: detail,
                                state: ProcessState::Ready,
                                process: None,
                            })),
                        );
                    }
                    Err(e) => {
                        log::warn!("跳过插件目录 {}: {e}", plugin_dir.display());
                    }
                }
            }
        }
        Self {
            app: app.clone(),
            plugins: Arc::new(Mutex::new(plugins)),
        }
    }

    /// 内置插件目录（打包后位于资源目录；开发时位于仓库根 `plugins/`）。
    pub fn builtin_plugin_dirs(app: &AppHandle) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        // 开发模式：仓库根目录的 plugins/
        if let Ok(cwd) = std::env::current_dir() {
            for ancestor in cwd.ancestors().take(4) {
                let candidate = ancestor.join("plugins");
                if candidate.is_dir() {
                    dirs.push(candidate);
                    break;
                }
            }
        }
        // 生产模式：resource_dir/plugins
        if let Ok(resource) = app.path().resource_dir() {
            let candidate = resource.join("plugins");
            if candidate.is_dir() {
                dirs.push(candidate);
            }
        }
        dirs
    }

    /// The app-owned user directory is always searched, even with custom roots.
    pub fn plugin_dirs(app: &AppHandle, custom_dirs: &[PathBuf]) -> Vec<PathBuf> {
        merge_plugin_dirs(
            Self::builtin_plugin_dirs(app),
            crate::core::get_default_plugins_dir(app),
            custom_dirs,
        )
    }

    /// 列出全部插件（含 invalid/crashed 状态）
    pub fn list(&self) -> Vec<PluginInfo> {
        let plugins = self.plugins.lock().expect("plugins mutex poisoned");
        let mut infos: Vec<PluginInfo> = plugins
            .values()
            .map(|p| {
                let p = p.lock().expect("plugin mutex poisoned");
                PluginInfo {
                    name: p.manifest.name.clone(),
                    version: p.manifest.version.clone(),
                    description: p.manifest.description.clone(),
                    author: p.manifest.author.clone(),
                    runtime: format!("{:?}", p.manifest.runtime).to_lowercase(),
                    capabilities: p
                        .manifest
                        .capabilities
                        .iter()
                        .map(|c| format!("{c:?}").to_lowercase())
                        .collect(),
                    parameters_schema: serde_json::to_value(&p.manifest.parameters_schema)
                        .unwrap_or(serde_json::Value::Null),
                    dir: p.dir.clone(),
                    status: p.status.clone(),
                    status_detail: p.status_detail.clone(),
                }
            })
            .collect();
        infos.sort_by(|a, b| a.name.cmp(&b.name));
        infos
    }

    fn get(&self, name: &str) -> Result<SharedPlugin, PluginError> {
        let plugins = self.plugins.lock().expect("plugins mutex poisoned");
        plugins
            .get(name)
            .cloned()
            .ok_or_else(|| PluginError::NotFound(name.to_string()))
    }

    /// 确保插件进程已启动；崩溃后自动重启。
    fn ensure_process(&self, plugin: &SharedPlugin) -> Result<(), PluginError> {
        let mut p = plugin.lock().expect("plugin mutex poisoned");
        if p.status == "invalid" {
            return Err(PluginError::Invalid(
                p.status_detail.clone().unwrap_or_default(),
            ));
        }
        let need_spawn = if p.state == ProcessState::Crashed {
            true
        } else {
            match p.process.as_ref() {
                None => true,
                Some(process) => {
                    // 进程句柄存在但已退出 → crashed
                    let alive = process.lock().expect("process mutex").is_alive();
                    if alive {
                        false
                    } else {
                        p.state = ProcessState::Crashed;
                        true
                    }
                }
            }
        };
        if !need_spawn {
            return Ok(());
        }
        // 旧进程尽力回收
        if let Some(old) = p.process.take() {
            if let Ok(mut guard) = old.lock() {
                guard.shutdown(Duration::from_secs(2));
            }
        }
        let app = self.app.clone();
        let plugin_name = p.manifest.name.clone();
        let on_notify: NotificationHandler = Arc::new(move |notification| {
            handle_notification(&app, &plugin_name, notification);
        });
        let process = PluginProcess::spawn(p.manifest.clone(), &p.dir, Some(on_notify))?;
        p.process = Some(Arc::new(Mutex::new(process)));
        p.state = ProcessState::Ready;
        p.status = "ready".to_string();
        p.status_detail = None;
        let started_name = p.manifest.name.clone();
        self.emit_status(&started_name, "started", None);
        Ok(())
    }

    /// 停止插件进程（设置变更/插件禁用/应用退出时调用）
    pub fn stop(&self, name: &str) -> Result<(), PluginError> {
        let plugin = self.get(name)?;
        let mut p = plugin.lock().expect("plugin mutex poisoned");
        if let Some(process) = p.process.take() {
            if let Ok(mut guard) = process.lock() {
                guard.shutdown(Duration::from_secs(3));
            }
        }
        p.state = ProcessState::Ready;
        if p.status != "invalid" {
            p.status = "ready".to_string();
        }
        self.emit_status(name, "stopped", None);
        Ok(())
    }

    /// 停止所有插件进程（应用退出时）
    pub fn stop_all(&self) {
        let plugins = self.plugins.lock().expect("plugins mutex poisoned");
        for plugin in plugins.values() {
            let mut p = plugin.lock().expect("plugin mutex poisoned");
            if let Some(process) = p.process.take() {
                if let Ok(mut guard) = process.lock() {
                    guard.shutdown(Duration::from_secs(2));
                }
            }
        }
    }

    /// 重新加载（热重载）：停止进程并重新扫描清单。
    ///
    /// 新增插件目录 = 新增一个目录，不需要改主程序；
    /// `reload` 允许前端按钮触发重新发现。
    pub fn reload(&self, plugin_dirs: &[PathBuf]) {
        self.stop_all();
        let fresh = Self::discover(&self.app, plugin_dirs);
        let fresh_map = Arc::try_unwrap(fresh.plugins)
            .map(|m| m.into_inner().expect("fresh map poisoned"))
            .unwrap_or_default();
        let mut plugins = self.plugins.lock().expect("plugins mutex poisoned");
        *plugins = fresh_map;
    }

    /// 执行插件任务（同步阻塞版本；调用方用 `spawn_blocking`）。
    ///
    /// 返回 `(task_id, PluginResult)`；进度通过 notification handler → Tauri event。
    pub fn run_plugin(
        &self,
        name: &str,
        video: &crate::core::library::VideoInfo,
        params: serde_json::Value,
        output_dir: PathBuf,
    ) -> Result<(String, PluginResult), PluginError> {
        let plugin = self.get(name)?;

        // 能力校验：FileWrite 需要 output_dir（我们总会提供，但插件若声明
        // videoFrames/audioSamples 等只是声明，不强制检查）
        {
            let p = plugin.lock().expect("plugin mutex poisoned");
            if p.manifest
                .capabilities
                .contains(&PluginCapability::FileWrite)
            {
                std::fs::create_dir_all(&output_dir)
                    .map_err(|e| PluginError::Task(format!("创建输出目录失败: {e}")))?;
            }
        }

        self.ensure_process(&plugin)?;

        let task_id = uuid::Uuid::new_v4().to_string();
        let run_params = RunParams {
            task_id: task_id.clone(),
            video: RunVideoInfo {
                id: video.id.clone(),
                path: video.path.clone(),
                duration: video.duration,
                width: video.width,
                height: video.height,
                fps: video.fps,
                metadata: video.metadata.clone(),
            },
            params,
            output_dir: output_dir.clone(),
        };

        let plugin_name = name.to_string();
        // 取进程 Arc 并标记 running，然后释放锁 — 长 RPC 在锁外执行，
        // 保证 cancel/stop/reload 在运行期间可达。
        let process_arc = {
            let mut p = plugin.lock().expect("plugin mutex poisoned");
            p.state = ProcessState::Running;
            self.emit_status(&plugin_name, "running", None);
            p.process.clone()
        };
        // 构建 run 生命周期内的宿主反向服务处理器（frames.extract）。
        // 只在 frame_service 实现存在时注入；当前 FrameService 由子代理并行开发，
        // 若模块尚未就位则以 `cfg` 软连接，避免阻塞主代码编译。
        let video_path = video.path.clone();
        let host_handler = self.build_host_service_handler(video_path, output_dir.clone());

        let result = match process_arc {
            Some(process) => {
                let mut guard = process.lock().expect("process mutex poisoned");
                guard.run_with_host_services(
                    serde_json::to_value(&run_params)
                        .map_err(|e| PluginError::Task(e.to_string()))?,
                    None,
                    Some(host_handler),
                )
            }
            None => Err(PluginProcessError::SpawnFailed("进程未启动".to_string())),
        };

        let mut p = plugin.lock().expect("plugin mutex poisoned");
        match &result {
            Ok(_) => {
                p.state = ProcessState::Ready;
                p.status = "ready".to_string();
            }
            Err(PluginProcessError::ProcessExited) => {
                p.state = ProcessState::Crashed;
                p.status = "crashed".to_string();
                p.status_detail = Some("插件进程意外退出".to_string());
                self.emit_status(&plugin_name, "crashed", p.status_detail.clone());
            }
            Err(_) => {
                p.state = ProcessState::Ready;
            }
        }
        result.map(|r| (task_id, r)).map_err(PluginError::from)
    }

    /// 请求取消某插件的当前任务（尽力而为）
    pub fn cancel(&self, name: &str, task_id: &str) {
        if let Ok(plugin) = self.get(name) {
            let p = plugin.lock().expect("plugin mutex poisoned");
            if let Some(process) = p.process.as_ref() {
                if let Ok(mut guard) = process.lock() {
                    guard.cancel(task_id);
                }
            }
        }
    }

    /// 组装 `frames.extract` 反向服务处理器。
    ///
    /// - `video_path` / `output_dir` 在本次 `run` 开始时快照，确保
    ///   服务上下文与 `RunParams` 一致（插件后续修改自身 `outputDir`
    ///   不会影响授权边界）。
    /// - `FrameService` 每次请求临时创建，避免长生命周期状态泄漏；
    ///   ffmpeg 路径从 settings 解析，与依赖报告使用同一逻辑。
    fn build_host_service_handler(
        &self,
        video_path: std::path::PathBuf,
        output_dir: PathBuf,
    ) -> HostRequestHandler {
        let app = self.app.clone();
        Arc::new(move |request: &super::protocol::RpcRequest| {
            match request.method.as_str() {
                HOST_SERVICE_FRAMES_EXTRACT => {
                    let params = match serde_json::from_value::<serde_json::Value>(
                        request.params.clone(),
                    ) {
                        Ok(v) => v,
                        Err(e) => {
                            return Err((
                                -32602,
                                format!("frames.extract 参数解析失败：{e}"),
                            ))
                        }
                    };
                    let frame_request: FrameExtractRequest =
                        match serde_json::from_value(params) {
                            Ok(r) => r,
                            Err(e) => {
                                return Err((
                                    -32602,
                                    format!("frames.extract 请求结构错误：{e}"),
                                ))
                            }
                        };
                    let ffmpeg = match crate::core::tools::resolve_ffmpeg(&app) {
                        Some(p) => p,
                        None => {
                            return Err((
                                -32000,
                                "ffmpeg 未配置或不可用".to_string(),
                            ))
                        }
                    };
                    let service = match FrameService::new(
                        ffmpeg,
                        video_path.clone(),
                        output_dir.clone(),
                    ) {
                        Ok(s) => s,
                        Err(e) => return Err((-32000, e.to_string())),
                    };
                    match service.extract(frame_request) {
                        Ok(result) => serde_json::to_value(&result).map_err(|e| {
                            (-32603, format!("frames.extract 结果序列化失败：{e}"))
                        }),
                        Err(e) => Err((-32000, e.to_string())),
                    }
                }
                _ => Err((
                    -32601,
                    format!("不支持的宿主服务方法：{}", request.method),
                )),
            }
        })
    }

    fn emit_status(&self, plugin_name: &str, status: &str, detail: Option<String>) {
        let _ = self.app.emit(
            EVENT_PLUGIN_STATUS,
            super::protocol::PluginStatusEvent {
                schema_version: SCHEMA_VERSION.to_string(),
                plugin_name: plugin_name.to_string(),
                status: status.to_string(),
                detail,
            },
        );
    }
}

/// 处理插件通知：转发为 Tauri event 给前端。
fn handle_notification(app: &AppHandle, plugin_name: &str, notification: PluginNotification) {
    match notification.method.as_str() {
        "progress" => {
            if let Ok(params) = serde_json::from_value::<ProgressParams>(notification.params) {
                let _ = app.emit(
                    EVENT_PLUGIN_PROGRESS,
                    super::protocol::PluginProgressEvent {
                        schema_version: SCHEMA_VERSION.to_string(),
                        plugin_name: plugin_name.to_string(),
                        task_id: params.task_id,
                        progress: params.progress,
                        message: params.message,
                    },
                );
            }
        }
        "log" => {
            if let Ok(params) = serde_json::from_value::<LogParams>(notification.params) {
                let _ = app.emit(
                    EVENT_PLUGIN_LOG,
                    serde_json::json!({
                        "schemaVersion": SCHEMA_VERSION,
                        "pluginName": plugin_name,
                        "taskId": params.task_id,
                        "level": params.level,
                        "message": params.message,
                    }),
                );
            }
        }
        _ => {
            // 未知通知：透传，前端可按需忽略
            let _ = app.emit(
                EVENT_PLUGIN_LOG,
                serde_json::json!({
                    "schemaVersion": SCHEMA_VERSION,
                    "pluginName": plugin_name,
                    "method": notification.method,
                    "params": notification.params,
                }),
            );
        }
    }
}

/// 合并多个插件结果并去重：
/// 同一视频上、start_time 相差 < `epsilon` 秒且 label 相同的标注视为重复，
/// 保留 confidence 较高者（无 confidence 的排后）。
pub fn merge_marks(marks: Vec<Mark>, epsilon: f64) -> Vec<Mark> {
    let mut sorted = marks;
    sorted.sort_by(|a, b| {
        a.start_time
            .total_cmp(&b.start_time)
            .then_with(|| a.label.cmp(&b.label))
    });

    let mut merged: Vec<Mark> = Vec::with_capacity(sorted.len());
    for mark in sorted {
        let dominated = merged.iter_mut().find(|existing| {
            existing.label == mark.label
                && (existing.start_time - mark.start_time).abs() < epsilon
                && existing.end_time == mark.end_time
        });
        match dominated {
            Some(existing) => {
                let better = mark.confidence.unwrap_or(0.0) > existing.confidence.unwrap_or(0.0);
                if better {
                    *existing = mark;
                }
            }
            None => merged.push(mark),
        }
    }
    merged
}

/// 序列化版本常量（供插件 manifest 默认）
pub const fn protocol_version() -> &'static str {
    PROTOCOL_VERSION
}

fn merge_plugin_dirs(
    mut builtin_dirs: Vec<PathBuf>,
    default_dir: PathBuf,
    custom_dirs: &[PathBuf],
) -> Vec<PathBuf> {
    builtin_dirs.push(default_dir);
    builtin_dirs.extend_from_slice(custom_dirs);
    let mut unique = Vec::new();
    for dir in builtin_dirs {
        if !unique.contains(&dir) {
            unique.push(dir);
        }
    }
    unique
}

#[cfg(test)]
mod directory_tests {
    use super::*;

    #[test]
    fn keeps_builtin_and_default_roots_with_multiple_custom_directories() {
        let builtin = PathBuf::from("builtin");
        let default = PathBuf::from("app-data/plugins");
        let first = PathBuf::from("custom-first");
        let second = PathBuf::from("custom-second");
        assert_eq!(
            merge_plugin_dirs(
                vec![builtin.clone()],
                default.clone(),
                &[
                    first.clone(),
                    default.clone(),
                    second.clone(),
                    first.clone()
                ],
            ),
            vec![builtin.clone(), default.clone(), first, second],
        );
        assert_eq!(
            merge_plugin_dirs(vec![builtin.clone()], default.clone(), &[]),
            vec![builtin, default],
        );
    }
}
