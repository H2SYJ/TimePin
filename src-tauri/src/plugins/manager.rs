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

use super::manifest::{LocalizedText, PluginCapability, PluginManifest, PluginRuntime};
use super::protocol::{
    LogParams, PluginNotification, PluginResult, ProgressParams, RunParams, RunVideoInfo,
    PROTOCOL_VERSION,
};
use super::python_env::{self, PythonEnvError};
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
    #[error(transparent)]
    Env(#[from] PythonEnvError),
    #[error("任务失败：{0}")]
    Task(String),
}

/// 前端可见的插件条目（`get_plugins` 返回）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    /// 本地化文本（字符串或 locale→string 映射），由前端按 settings.language 解析
    pub description: LocalizedText,
    pub author: String,
    pub runtime: String,
    pub capabilities: Vec<String>,
    pub parameters_schema: serde_json::Value,
    pub dir: PathBuf,
    /// ready | invalid | crashed | running
    pub status: String,
    #[serde(default)]
    pub status_detail: Option<String>,
    /// Python 环境状态（runtime=python 时填充）
    #[serde(default)]
    pub python_environment: Option<python_env::PluginEnvStatus>,
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
    /// Serializes process admission with stop/switch/rediscovery, not run RPCs.
    /// Acquire settings (and an install lock if needed) before this lock.
    lifecycle: Arc<Mutex<()>>,
}

type SharedPlugin = Arc<Mutex<ManagedPlugin>>;

/// Discovery must preserve live entries, except the one explicitly stopped for
/// replacement. Dropping another entry here would lose its process/state.
fn merge_discovered_plugins<T>(
    plugins: &mut HashMap<String, T>,
    fresh: HashMap<String, T>,
    replacement: Option<&str>,
) {
    for (name, plugin) in fresh {
        if replacement == Some(name.as_str()) {
            plugins.insert(name, plugin);
        } else {
            plugins.entry(name).or_insert(plugin);
        }
    }
}

fn install_transition<T, E>(
    lifecycle: &Mutex<()>,
    stop: impl FnOnce(),
    activate: impl FnOnce() -> Result<T, E>,
    reload: impl FnOnce(),
) -> Result<T, E> {
    let _lifecycle = lifecycle.lock().expect("lifecycle mutex poisoned");
    stop();
    let result = activate();
    reload();
    result
}

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
            lifecycle: Arc::new(Mutex::new(())),
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

    /// 在不停止已有进程的前提下，把新扫描到的插件目录并入注册表。
    /// 供 GitHub 安装后/目录变更探测使用；已有条目保持原状态与进程。
    pub fn register_new(&self, plugin_dirs: &[PathBuf]) {
        let _lifecycle = self.lifecycle.lock().expect("lifecycle mutex poisoned");
        self.register_new_locked(plugin_dirs, None);
    }

    /// 仅替换指定名称的注册条目并停止其既有进程。
    /// 用于 GitHub 覆盖安装同一插件：TPIN-18 契约不允许像本地导入那样
    /// 全局停止其它插件。已按名称存在的条目会在激活失败时恢复原注册状态；
    /// 激活成功后由 register_new_locked 用新目录覆盖。
    pub fn install_and_register_new<T, E>(
        &self,
        plugin_name: &str,
        plugin_dirs: &[PathBuf],
        activate: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        install_transition(
            &self.lifecycle,
            || self.stop_locked(plugin_name),
            activate,
            || self.register_new_locked(plugin_dirs, Some(plugin_name)),
        )
    }

    /// Caller owns lifecycle; waits only for the named plugin's RPC, never other plugins.
    fn stop_locked(&self, name: &str) {
        let plugins = self.plugins.lock().expect("plugins mutex poisoned");
        if let Some(plugin) = plugins.get(name) {
            let mut p = plugin.lock().expect("plugin mutex poisoned");
            if let Some(process) = p.process.take() {
                if let Ok(mut guard) = process.lock() {
                    guard.shutdown(Duration::from_secs(2));
                }
            }
        }
    }

    /// Caller owns lifecycle; only the explicitly stopped replacement may be
    /// overwritten. Other entries keep their state and process.
    fn register_new_locked(&self, plugin_dirs: &[PathBuf], replacement: Option<&str>) {
        let fresh = Self::discover(&self.app, plugin_dirs);
        let fresh_map = Arc::try_unwrap(fresh.plugins)
            .map(|m| m.into_inner().expect("fresh map poisoned"))
            .unwrap_or_default();
        let mut plugins = self.plugins.lock().expect("plugins mutex poisoned");
        merge_discovered_plugins(&mut plugins, fresh_map, replacement);
    }

    /// 列出全部插件（含 invalid/crashed 状态）。
    /// `settings` 传入用于查询 Python 环境状态；传 `None` 时该字段为 None。
    pub fn list(&self, settings: Option<&crate::core::settings::Settings>) -> Vec<PluginInfo> {
        let plugins = self.plugins.lock().expect("plugins mutex poisoned");
        let mut infos: Vec<PluginInfo> = plugins
            .values()
            .map(|p| {
                let p = p.lock().expect("plugin mutex poisoned");
                let python_environment = settings
                    .map(|s| python_env::plugin_env_status(&p.manifest, &p.dir, &self.app, s));
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
                    python_environment,
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
    fn ensure_process(
        &self,
        plugin: &SharedPlugin,
        settings: &crate::core::settings::Settings,
    ) -> Result<(), PluginError> {
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
        // Python runtime：先准备解释器（共享或插件 venv），再 spawn。
        // 环境建置失败必须阻断启动并把 pip 输出透传出去——铁律验收。
        let python_interpreter = if p.manifest.runtime == PluginRuntime::Python {
            Some(python_env::ensure_python_environment(
                &p.manifest,
                &p.dir,
                &self.app,
                settings,
            )?)
        } else {
            None
        };
        let app = self.app.clone();
        let plugin_name = p.manifest.name.clone();
        let on_notify: NotificationHandler = Arc::new(move |notification| {
            handle_notification(&app, &plugin_name, notification);
        });
        let process = PluginProcess::spawn(
            p.manifest.clone(),
            &p.dir,
            python_interpreter.as_deref(),
            Some(on_notify),
        )?;
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
        let _lifecycle = self.lifecycle.lock().expect("lifecycle mutex poisoned");
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
        let _lifecycle = self.lifecycle.lock().expect("lifecycle mutex poisoned");
        self.stop_all_locked();
    }

    /// Caller owns lifecycle; waiting for an existing RPC is intentional. Never
    /// reacquire settings here: that RPC may call a settings-reading host service.
    fn stop_all_locked(&self) {
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
    /// 同时清理「已不存在于任何插件目录」的孤儿 venv。
    pub fn reload(&self, plugin_dirs: &[PathBuf]) {
        let _lifecycle = self.lifecycle.lock().expect("lifecycle mutex poisoned");
        self.stop_all_locked();
        self.rediscover_locked(plugin_dirs);
    }

    /// Only call after staging/validation, with settings → install locks held.
    /// No new launch can slip between stop and switch. Reconcile even on a
    /// failed switch so the restored install remains runnable.
    pub fn install_and_reload<T, E>(
        &self,
        plugin_dirs: &[PathBuf],
        activate: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E> {
        install_transition(
            &self.lifecycle,
            || self.stop_all_locked(),
            activate,
            || self.rediscover_locked(plugin_dirs),
        )
    }

    fn rediscover_locked(&self, plugin_dirs: &[PathBuf]) {
        let fresh = Self::discover(&self.app, plugin_dirs);
        let fresh_map = Arc::try_unwrap(fresh.plugins)
            .map(|m| m.into_inner().expect("fresh map poisoned"))
            .unwrap_or_default();
        let mut plugins = self.plugins.lock().expect("plugins mutex poisoned");
        *plugins = fresh_map;
        drop(plugins);
        self.cleanup_orphan_environments();
    }

    /// 清理 `<appData>/pluginEnvironments/` 下没有对应插件的目录。
    /// 删除插件目录后 reload 时自动触发。
    fn cleanup_orphan_environments(&self) {
        let envs_root = crate::core::get_app_data_dir(&self.app).join("pluginEnvironments");
        let Ok(entries) = std::fs::read_dir(&envs_root) else {
            return;
        };
        // 当前所有插件期望的环境目录名集合
        let expected: std::collections::HashSet<String> = {
            let plugins = self.plugins.lock().expect("plugins mutex poisoned");
            plugins
                .values()
                .filter_map(|p| {
                    let p = p.lock().ok()?;
                    if p.manifest.runtime == PluginRuntime::Python
                        && !p.manifest.dependencies.is_empty()
                    {
                        Some(python_env::env_dir_name(&p.manifest, &p.dir))
                    } else {
                        None
                    }
                })
                .collect()
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !expected.contains(&name) {
                log::info!("清理孤儿插件环境：{}", path.display());
                let _ = std::fs::remove_dir_all(&path);
            }
        }
    }

    /// 删除某插件对应的 Python 环境（卸载插件时调用）。
    /// 插件目录可能已不存在——按 manifest+dir 定位 env 目录即可。
    pub fn remove_environment(&self, name: &str) -> Result<(), PluginError> {
        let _lifecycle = self.lifecycle.lock().expect("lifecycle mutex poisoned");
        let plugin = self.get(name)?;
        let p = plugin.lock().expect("plugin mutex poisoned");
        python_env::remove_plugin_environment(&self.app, &p.manifest, &p.dir)?;
        Ok(())
    }

    /// 重建所有 Python 插件环境（设置页「重建所有插件环境」按钮）。
    /// 进程中的插件会先停止；逐个插件重建，单插件失败不影响其他。
    pub fn rebuild_environments(
        &self,
        settings: &crate::core::settings::Settings,
    ) -> Vec<(String, Result<PathBuf, String>)> {
        let _lifecycle = self.lifecycle.lock().expect("lifecycle mutex poisoned");
        self.stop_all_locked();
        let plugins = self.plugins.lock().expect("plugins mutex poisoned");
        let mut results = Vec::new();
        for plugin in plugins.values() {
            let p = plugin.lock().expect("plugin mutex poisoned");
            if p.manifest.runtime != PluginRuntime::Python || p.manifest.dependencies.is_empty() {
                continue;
            }
            let result =
                python_env::ensure_python_environment(&p.manifest, &p.dir, &self.app, settings)
                    .map_err(|e| e.to_string());
            results.push((p.manifest.name.clone(), result));
        }
        results
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
        let settings_guard = crate::core::settings::settings_lock_guard();
        let settings = crate::core::settings::load_settings(&self.app)
            .map_err(|e| PluginError::Task(format!("读取设置失败：{e}")))?;
        let lifecycle = self.lifecycle.lock().expect("lifecycle mutex poisoned");
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

        self.ensure_process(&plugin, &settings)?;

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
        // Reserve the process RPC mutex while admission is still held: a reload
        // can wait for this run, but can never replace the map then make a new
        // launch spawn an orphaned process against a stale entry. RPC completion
        // requires no lifecycle/settings lock, so shutdown cannot deadlock.
        let process_arc = {
            let mut p = plugin.lock().expect("plugin mutex poisoned");
            p.state = ProcessState::Running;
            self.emit_status(&plugin_name, "running", None);
            p.process.clone()
        };
        let process =
            process_arc.ok_or_else(|| PluginProcessError::SpawnFailed("进程未启动".to_string()))?;
        let mut process_guard = process.lock().expect("process mutex poisoned");
        drop(lifecycle);
        drop(settings_guard);

        // 构建 run 生命周期内的宿主反向服务处理器（frames.extract）。
        // 只在 frame_service 实现存在时注入；当前 FrameService 由子代理并行开发，
        // 若模块尚未就位则以 `cfg` 软连接，避免阻塞主代码编译。
        let video_path = video.path.clone();
        let host_handler = self.build_host_service_handler(video_path, output_dir.clone());
        // 长 RPC 在生命周期/设置锁外执行，保证 cancel/stop/reload 在运行期间可达。
        let result = process_guard.run_with_host_services(
            serde_json::to_value(&run_params).map_err(|e| PluginError::Task(e.to_string()))?,
            None,
            Some(host_handler),
        );
        drop(process_guard);

        // A reload may have completed while the RPC was finishing. Do not emit
        // stale status for an entry that is no longer in the manager.
        let _lifecycle = self.lifecycle.lock().expect("lifecycle mutex poisoned");
        if !self
            .get(name)
            .is_ok_and(|current| Arc::ptr_eq(&current, &plugin))
        {
            return result.map(|r| (task_id, r)).map_err(PluginError::from);
        }
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
        Arc::new(
            move |request: &super::protocol::RpcRequest| match request.method.as_str() {
                HOST_SERVICE_FRAMES_EXTRACT => {
                    let params =
                        match serde_json::from_value::<serde_json::Value>(request.params.clone()) {
                            Ok(v) => v,
                            Err(e) => {
                                return Err((-32602, format!("frames.extract 参数解析失败：{e}")))
                            }
                        };
                    let frame_request: FrameExtractRequest = match serde_json::from_value(params) {
                        Ok(r) => r,
                        Err(e) => {
                            return Err((-32602, format!("frames.extract 请求结构错误：{e}")))
                        }
                    };
                    let ffmpeg = match crate::core::tools::resolve_ffmpeg(&app) {
                        Some(p) => p,
                        None => return Err((-32000, "ffmpeg 未配置或不可用".to_string())),
                    };
                    let service =
                        match FrameService::new(ffmpeg, video_path.clone(), output_dir.clone()) {
                            Ok(s) => s,
                            Err(e) => return Err((-32000, e.to_string())),
                        };
                    match service.extract(frame_request) {
                        Ok(result) => serde_json::to_value(&result)
                            .map_err(|e| (-32603, format!("frames.extract 结果序列化失败：{e}"))),
                        Err(e) => Err((-32000, e.to_string())),
                    }
                }
                _ => Err((-32601, format!("不支持的宿主服务方法：{}", request.method))),
            },
        )
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
    use std::sync::mpsc;

    #[test]
    fn installation_holds_admission_through_reload_even_on_switch_failure() {
        for succeeds in [true, false] {
            let lifecycle = Mutex::new(());
            let events = Mutex::new(Vec::new());
            let result = install_transition(
                &lifecycle,
                || {
                    assert!(lifecycle.try_lock().is_err());
                    events.lock().unwrap().push("stop");
                },
                || {
                    assert!(lifecycle.try_lock().is_err());
                    events.lock().unwrap().push("switch");
                    if succeeds {
                        Ok(())
                    } else {
                        Err("switch failed")
                    }
                },
                || {
                    assert!(lifecycle.try_lock().is_err());
                    events.lock().unwrap().push("reload");
                },
            );
            assert_eq!(result.is_ok(), succeeds);
            assert_eq!(*events.lock().unwrap(), ["stop", "switch", "reload"]);
            assert!(lifecycle.try_lock().is_ok());
        }
    }

    #[test]
    fn launch_waits_for_stop_switch_reload_but_existing_rpc_can_finish() {
        let lifecycle = Mutex::new(());
        let process_rpc = Mutex::new(());
        let version = Mutex::new("old");
        let (stopping_tx, stopping_rx) = mpsc::channel();
        let (launching_tx, launching_rx) = mpsc::channel();
        let (launched_tx, launched_rx) = mpsc::channel();
        // Model an admitted run: release lifecycle before the long RPC.
        let admission = lifecycle.lock().unwrap();
        let running_rpc = process_rpc.lock().unwrap();
        drop(admission);
        assert!(lifecycle.try_lock().is_ok());
        std::thread::scope(|scope| {
            let installer = scope.spawn(|| {
                install_transition(
                    &lifecycle,
                    || {
                        stopping_tx.send(()).unwrap();
                        // The real stop also waits for the process RPC lock.
                        let _rpc = process_rpc.lock().unwrap();
                    },
                    || {
                        *version.lock().unwrap() = "switched";
                        Ok::<_, ()>(())
                    },
                    || *version.lock().unwrap() = "reloaded",
                )
                .unwrap();
            });
            stopping_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let launcher = scope.spawn(|| {
                launching_tx.send(()).unwrap();
                let _admission = lifecycle.lock().unwrap();
                launched_tx.send(*version.lock().unwrap()).unwrap();
            });
            launching_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(launched_rx.try_recv().is_err());
            // Finishing the RPC never needs lifecycle/settings, avoiding deadlock.
            drop(running_rpc);
            installer.join().unwrap();
            launcher.join().unwrap();
            assert_eq!(launched_rx.recv().unwrap(), "reloaded");
        });
    }

    /// Models `install_and_register_new`: the stop step must target only the
    /// named plugin's process lock (never other plugins) and the reload step
    /// must merge the fresh entry under the same name without clearing the map.
    /// TPIN-18 forbids the global stop_all/rediscover path on GitHub installs.
    #[test]
    fn github_transition_stops_only_target_plugin_and_merges_fresh_entry() {
        let lifecycle = Mutex::new(());
        // One lock per running plugin process; `other` stays held to prove the
        // transition never waits on it.
        let target_rpc = Mutex::new(());
        let other_rpc = Mutex::new(());
        let stopped = Mutex::new(Vec::new());
        let other_guard = other_rpc.lock().unwrap();
        let result = install_transition(
            &lifecycle,
            || {
                // Mirror stop_locked("target"): touch only target_rpc. If the
                // implementation ever reached for other_rpc this would deadlock,
                // and the timed assert below would fail the test.
                let _target = target_rpc.lock().unwrap();
                stopped.lock().unwrap().push("target");
            },
            || Ok::<_, &str>("switched"),
            || {
                // Mirror register_new_locked: insert/overwrite by name only.
                stopped.lock().unwrap().push("register");
            },
        );
        assert_eq!(result.unwrap(), "switched");
        assert_eq!(*stopped.lock().unwrap(), ["target", "register"]);
        drop(other_guard);
    }

    #[test]
    fn discovery_preserves_live_entries_and_replaces_only_the_stopped_target() {
        for replacement in [None, Some("target")] {
            let target = Arc::new("old-target");
            let other = Arc::new("running-other");
            let fresh_target = Arc::new("new-target");
            let fresh_other = Arc::new("idle-other");
            let fresh_new = Arc::new("new-plugin");
            let mut registry = HashMap::from([
                ("target".to_string(), target.clone()),
                ("other".to_string(), other.clone()),
            ]);
            merge_discovered_plugins(
                &mut registry,
                HashMap::from([
                    ("target".to_string(), fresh_target.clone()),
                    ("other".to_string(), fresh_other),
                    ("new".to_string(), fresh_new.clone()),
                ]),
                replacement,
            );
            assert!(Arc::ptr_eq(&registry["other"], &other));
            assert!(Arc::ptr_eq(&registry["new"], &fresh_new));
            assert!(Arc::ptr_eq(
                &registry["target"],
                if replacement.is_some() {
                    &fresh_target
                } else {
                    &target
                },
            ));
        }
    }

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
