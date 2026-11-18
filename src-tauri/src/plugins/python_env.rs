//! Python 运行时与插件虚拟环境管理（TPIN-4）。
//!
//! 目标：用户无需自行安装 Python，TimePin 携带随包分发的解释器；
//! 每个 Python 插件在其专属 venv 中运行，依赖隔离，声明式安装。
//!
//! 结构：
//! - 内嵌运行时：`<exe>/runtime/python/python.exe`（打包分发），
//!   开发回退到 `dependencies/python/` 或系统 Python。
//! - 插件 venv：`<appData>/pluginEnvironments/<plugin>/venv`。
//!   指纹文件 `<venv>/.fingerprint` 记录「解释器版本 + 依赖列表」，
//!   不匹配时自动重建（不区分大小写、规范化空白）。
//! - 插件数据目录：`<appData>/plugins/<plugin>/` 仍属插件写权限范围；
//!   venv 目录由主程序管理，插件不得写入。
//!
//! venv 建置失败时必须把 pip 输出透传给用户（验收标准）。

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use thiserror::Error;

use crate::ipc::SCHEMA_VERSION;
use crate::plugins::manifest::PluginManifest;

/// 前端监听的事件名：插件 Python 环境建置进度。
pub const ENV_PROGRESS_EVENT: &str = "plugin://env-progress";

/// venv 建置超时（numpy/onnxruntime 等 wheel 较大，给足余量）。
const VENV_CREATE_TIMEOUT: Duration = Duration::from_secs(120);
const PIP_INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Error)]
pub enum PythonEnvError {
    #[error("找不到可用的 Python 解释器：{0}")]
    InterpreterNotFound(String),
    #[error("Python 版本不满足插件要求 {required}：实际 {actual}")]
    VersionMismatch { required: String, actual: String },
    #[error("创建虚拟环境失败：{0}")]
    VenvCreateFailed(String),
    #[error("安装依赖失败：{0}")]
    PipInstallFailed(String),
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
}

/// 一次环境建置的进度事件 payload。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvProgressEvent {
    pub schema_version: String,
    pub plugin_name: String,
    /// creating-venv | installing | ready | failed
    pub stage: String,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

fn emit_env_progress(
    app: &AppHandle,
    plugin_name: &str,
    stage: &str,
    message: Option<String>,
    error: Option<String>,
) {
    let _ = app.emit(
        ENV_PROGRESS_EVENT,
        EnvProgressEvent {
            schema_version: SCHEMA_VERSION.to_string(),
            plugin_name: plugin_name.to_string(),
            stage: stage.to_string(),
            message,
            error,
        },
    );
}

/// 运行中的 venv 建置互斥锁（同一时刻只允许一个建置任务）。
static BUILD_RUNNING: AtomicBool = AtomicBool::new(false);

struct BuildGuard;

impl BuildGuard {
    fn acquire() -> Result<Self, PythonEnvError> {
        BUILD_RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| {
                PythonEnvError::VenvCreateFailed("已有插件环境建置任务正在运行".to_string())
            })
    }
}

impl Drop for BuildGuard {
    fn drop(&mut self) {
        BUILD_RUNNING.store(false, Ordering::Release);
    }
}

/// Python 解释器解析结果。
#[derive(Debug, Clone)]
pub struct ResolvedPython {
    pub executable: PathBuf,
    /// `python --version` 输出（如 "Python 3.11.9"）。
    pub version_string: String,
    /// 来源：bundled | user-configured | system
    pub source: PythonSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PythonSource {
    Bundled,
    UserConfigured,
    System,
}

impl ResolvedPython {
    /// 解析 semver 三元组用于约束比较。
    fn version_tuple(&self) -> Option<(u64, u64, u64)> {
        let raw = self.version_string.trim_start_matches("Python ").trim();
        let mut parts = raw.split('.');
        Some((
            parts.next()?.parse().ok()?,
            parts.next().unwrap_or("0").parse().ok()?,
            parts.next().unwrap_or("0").parse().ok()?,
        ))
    }
}

/// 按优先级解析 Python 解释器：
/// 1. `settings.pythonPath`（显式配置，必须存在且可执行）
/// 2. 内嵌运行时 `<exe>/runtime/python/`（生产包携带）
/// 3. `dependencies/python/`（开发期下载，与 FFmpeg 安装目录约定一致）
/// 4. `useSystemPython == true` 时的 `python`/`python3`
///
/// 找不到时返回 `InterpreterNotFound`，绝不静默回退到不可控解释器。
pub fn resolve_python(
    app: &AppHandle,
    settings: &crate::core::settings::Settings,
) -> Result<ResolvedPython, PythonEnvError> {
    // 1) 显式配置
    if let Some(configured) = &settings.python_path {
        if configured.as_os_str().is_empty() {
            return Err(PythonEnvError::InterpreterNotFound(
                "pythonPath 已配置但为空路径".to_string(),
            ));
        }
        if !configured.is_absolute() {
            return Err(PythonEnvError::InterpreterNotFound(format!(
                "pythonPath 必须是绝对路径：{}",
                configured.display()
            )));
        }
        let exe = normalize_python_candidate(configured);
        if exe.is_file() {
            let version = probe_python_version(&exe)?;
            return Ok(ResolvedPython {
                executable: exe,
                version_string: version,
                source: PythonSource::UserConfigured,
            });
        }
        return Err(PythonEnvError::InterpreterNotFound(format!(
            "pythonPath 指向的文件不存在：{}",
            configured.display()
        )));
    }

    // 2) 内嵌运行时（打包分发）：<exe>/runtime/python/
    if let Some(bundled) = bundled_python_from_exe(app) {
        if let Ok(resolved) = try_resolve(&bundled, PythonSource::Bundled) {
            return Ok(resolved);
        }
    }

    // 3) 开发/下载目录：exe 旁 dependencies/python/
    if let Some(dev) = dependencies_python_from_exe() {
        if let Ok(resolved) = try_resolve(&dev, PythonSource::Bundled) {
            return Ok(resolved);
        }
    }

    // 4) 用户显式允许的系统 Python
    if settings.use_system_python {
        for candidate in system_python_candidates() {
            if let Ok(resolved) = try_resolve(&candidate, PythonSource::System) {
                return Ok(resolved);
            }
        }
        return Err(PythonEnvError::InterpreterNotFound(
            "useSystemPython 已启用，但 PATH 中找不到 python/python3".to_string(),
        ));
    }

    Err(PythonEnvError::InterpreterNotFound(
        "未配置内嵌运行时且未启用系统 Python。请在高级设置中开启「允许使用系统 Python」或安装内嵌运行时。".to_string(),
    ))
}

fn try_resolve(exe: &Path, source: PythonSource) -> Result<ResolvedPython, PythonEnvError> {
    if !exe.is_file() {
        return Err(PythonEnvError::InterpreterNotFound(format!(
            "解释器不存在：{}",
            exe.display()
        )));
    }
    let version = probe_python_version(exe)?;
    Ok(ResolvedPython {
        executable: exe.to_path_buf(),
        version_string: version,
        source,
    })
}

/// `python --version` → "Python 3.11.9"。失败视为不可用。
fn probe_python_version(exe: &Path) -> Result<String, PythonEnvError> {
    let output = Command::new(exe)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| PythonEnvError::InterpreterNotFound(format!("{}: {e}", exe.display())))?;
    if !output.status.success() {
        return Err(PythonEnvError::InterpreterNotFound(format!(
            "{} --version 退出码非 0",
            exe.display()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return Err(PythonEnvError::InterpreterNotFound(format!(
            "{} --version 输出为空",
            exe.display()
        )));
    }
    Ok(text)
}

/// 规范化 python 可执行文件名（Windows 上允许省略 .exe）。
fn normalize_python_candidate(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        if path.extension().is_none() {
            return path.with_extension("exe");
        }
    }
    path.to_path_buf()
}

/// `<resource>/runtime/python/python[.exe]` — 打包分发目录。
/// Tauri 会把 `bundle.resources` 里的目录平铺到 `resource_dir`，
/// 所以 `runtime/` 作为资源目录存在；找不到时 fallback 到 exe 旁（开发期）。
fn bundled_python_from_exe(app: &AppHandle) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "python.exe"
    } else {
        "python"
    };
    // 优先 Tauri resource_dir（bundle 分发的标准位置）
    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidate = resource_dir.join("runtime").join("python").join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    // 开发期 / 绿色版：exe 旁 runtime/
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let candidate = exe_dir.join("runtime").join("python").join(name);
    candidate.is_file().then_some(candidate)
}

/// `<exe>/dependencies/python/python[.exe]` — 与 FFmpeg 依赖安装目录同级的开发约定。
fn dependencies_python_from_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let name = if cfg!(windows) {
        "python.exe"
    } else {
        "python3"
    };
    let candidate = exe_dir.join("dependencies").join("python").join(name);
    candidate.is_file().then_some(candidate)
}

fn system_python_candidates() -> Vec<PathBuf> {
    if cfg!(windows) {
        vec![PathBuf::from("python"), PathBuf::from("python3")]
    } else {
        vec![PathBuf::from("python3"), PathBuf::from("python")]
    }
}

/// 校验 manifest.python 约束（如 `">=3.10,<3.13"`、`"3.11.*"`、`"~=3.11"`）。
/// 无约束 → 通过。不支持的约束语法 → 报错（保守策略：宁缺毋滥）。
pub fn check_python_constraint(
    constraint: Option<&str>,
    resolved: &ResolvedPython,
) -> Result<(), PythonEnvError> {
    let Some(constraint) = constraint.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(());
    };
    let actual = resolved
        .version_tuple()
        .ok_or_else(|| PythonEnvError::VersionMismatch {
            required: constraint.to_string(),
            actual: resolved.version_string.clone(),
        })?;

    for part in constraint.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if !constraint_part_satisfied(part, actual) {
            return Err(PythonEnvError::VersionMismatch {
                required: constraint.to_string(),
                actual: resolved.version_string.clone(),
            });
        }
    }
    Ok(())
}

fn constraint_part_satisfied(part: &str, actual: (u64, u64, u64)) -> bool {
    // 支持常见 PEP 440 简化语法：>=, >, <=, <, ==, !=, ~=, X.Y.*
    type Cmp = fn((u64, u64, u64), (u64, u64, u64)) -> bool;
    let ops: &[(&str, Cmp)] = &[
        (">=", |a, b| a >= b),
        ("<=", |a, b| a <= b),
        ("!=", |a, b| a != b),
        ("==", |a, b| a == b),
        ("~=", |a, b| a >= b && a.0 == b.0),
        (">", |a, b| a > b),
        ("<", |a, b| a < b),
    ];
    for (prefix, cmp) in ops {
        if let Some(v) = part.strip_prefix(prefix) {
            let v = v.trim();
            if let Some(star) = v.strip_suffix(".*") {
                // "3.11.*" → 前缀匹配
                let mut segs = star.split('.');
                let major: u64 = segs.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                let minor: u64 = segs.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                // == X.Y.* → major/minor 精确匹配；!= X.Y.* → 反之
                let prefix_matches = actual.0 == major && actual.1 == minor;
                return match *prefix {
                    "==" => prefix_matches,
                    "!=" => !prefix_matches,
                    _ => false, // 其他操作符不支持 .*，视为不满足
                };
            }
            let expected = parse_version_tuple(v).unwrap_or((0, 0, 0));
            return cmp(actual, expected);
        }
    }
    // 裸版本号 → 视为 ==（但 "3.11" 按 PEP 440 匹配 3.11.x 前缀，
    // 与 =="3.11.*" 语义一致；"3.11.9" 才是精确匹配）
    if let Some(expected) = parse_version_tuple(part) {
        let parts = part.split('.').count();
        return match parts {
            1 => actual.0 == expected.0,
            2 => actual.0 == expected.0 && actual.1 == expected.1,
            _ => actual == expected,
        };
    }
    // 其他写法（如 PEP 508 marker）不支持 → 视为不满足（保守）
    false
}

fn parse_version_tuple(v: &str) -> Option<(u64, u64, u64)> {
    let mut parts = v.trim().split('.');
    Some((
        parts.next()?.parse().ok()?,
        parts.next().unwrap_or("0").parse().ok()?,
        parts.next().unwrap_or("0").parse().ok()?,
    ))
}

/// 插件 venv 根目录：`<appData>/pluginEnvironments/<pluginName>-<hash>/venv`。
/// `<hash>` 是规范化插件目录的 8 位十六进制后缀，防止同名插件在不同目录下碰撞。
pub fn plugin_venv_dir(app: &AppHandle, manifest: &PluginManifest, plugin_dir: &Path) -> PathBuf {
    crate::core::get_app_data_dir(app)
        .join("pluginEnvironments")
        .join(env_dir_name(manifest, plugin_dir))
        .join("venv")
}

/// 环境目录名：`name-hash8`。规范化插件目录的哈希保证同名不同源插件隔离。
/// `pub(crate)`：manager 在清理孤儿环境时需要枚举期望值。
pub(crate) fn env_dir_name(manifest: &PluginManifest, plugin_dir: &Path) -> String {
    let canonical = plugin_dir
        .canonicalize()
        .unwrap_or_else(|_| plugin_dir.to_path_buf());
    let mut h = DefaultHasher::new();
    canonical.to_string_lossy().to_lowercase().hash(&mut h);
    format!(
        "{}-{:08x}",
        sanitize_name(&manifest.name),
        h.finish() as u32
    )
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn venv_python(venv_dir: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "python.exe"
    } else {
        "python3"
    };
    #[cfg(windows)]
    {
        venv_dir.join("Scripts").join(name)
    }
    #[cfg(not(windows))]
    {
        venv_dir.join("bin").join(name)
    }
}

/// 依赖指纹：解释器版本 + 依赖列表的哈希，存入 `<venv>/.fingerprint`。
/// 不一致时重建 venv（用户改插件 manifest 后无需手动删目录）。
fn env_fingerprint(resolved: &ResolvedPython, dependencies: &[String]) -> String {
    let mut h = DefaultHasher::new();
    resolved.version_string.hash(&mut h);
    for dep in dependencies {
        dep.trim().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// 插件环境状态查询：venv 是否存在、解释器是否可用、指纹是否匹配。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginEnvStatus {
    /// creating-venv | installing | ready | failed | absent
    pub stage: String,
    #[serde(default)]
    pub venv_dir: Option<PathBuf>,
    #[serde(default)]
    pub python_version: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// 查询插件当前环境状态（不触发建置），供 UI 展示。
pub fn plugin_env_status(
    manifest: &PluginManifest,
    plugin_dir: &Path,
    app: &AppHandle,
    settings: &crate::core::settings::Settings,
) -> PluginEnvStatus {
    if manifest.runtime != crate::plugins::manifest::PluginRuntime::Python {
        return PluginEnvStatus {
            stage: "absent".to_string(),
            venv_dir: None,
            python_version: None,
            error: None,
        };
    }
    if manifest.dependencies.is_empty() {
        // 无依赖不需要独立环境——状态以共享解释器为准
        return match resolve_python(app, settings) {
            Ok(r) => PluginEnvStatus {
                stage: "ready".to_string(),
                venv_dir: None,
                python_version: Some(r.version_string),
                error: None,
            },
            Err(e) => PluginEnvStatus {
                stage: "failed".to_string(),
                venv_dir: None,
                python_version: None,
                error: Some(e.to_string()),
            },
        };
    }
    let venv_dir = plugin_venv_dir(app, manifest, plugin_dir);
    if !venv_dir.is_dir() {
        return PluginEnvStatus {
            stage: "absent".to_string(),
            venv_dir: Some(venv_dir),
            python_version: None,
            error: None,
        };
    }
    let exe = venv_python(&venv_dir);
    let version = probe_python_version(&exe).ok();
    // 指纹比对需要 resolved 才能复算——只校验「venv 完整」这一层面；
    // 完整重建决策留给 ensure_python_environment。
    PluginEnvStatus {
        stage: if exe.is_file() { "ready" } else { "failed" }.to_string(),
        venv_dir: Some(venv_dir),
        python_version: version,
        error: if exe.is_file() {
            None
        } else {
            Some("venv 存在但解释器缺失".to_string())
        },
    }
}

/// 保证插件的 Python 运行环境就绪，返回 venv 内的解释器路径。
///
/// - 无依赖且无需隔离：直接返回共享解释器（内嵌或系统）。
/// - 有 `dependencies`：建/复用 `<appData>/pluginEnvironments/<name>/venv`，
///   指纹一致直接复用，否则删旧建新 + `pip install`。
///
/// `app` 可空：测试与内部调用不传时静默执行。
pub fn ensure_python_environment(
    manifest: &PluginManifest,
    plugin_dir: &Path,
    app: &AppHandle,
    settings: &crate::core::settings::Settings,
) -> Result<PathBuf, PythonEnvError> {
    let resolved = resolve_python(app, settings)?;
    check_python_constraint(manifest.python.as_deref(), &resolved)?;

    // 无依赖 → 直接用共享解释器，不建 venv（保持轻量）。
    if manifest.dependencies.is_empty() {
        return Ok(resolved.executable);
    }

    let _guard = BuildGuard::acquire()?;
    let venv_dir = plugin_venv_dir(app, manifest, plugin_dir);
    let fingerprint = env_fingerprint(&resolved, &manifest.dependencies);
    let fingerprint_path = venv_dir.join(".fingerprint");

    // 指纹匹配 → 直接复用
    if fingerprint_path.is_file() {
        if let Ok(existing) = std::fs::read_to_string(&fingerprint_path) {
            if existing.trim() == fingerprint {
                let exe = venv_python(&venv_dir);
                if exe.is_file() {
                    return Ok(exe);
                }
            }
        }
    }

    // 重建：先清后建，失败不残留半成品（删旧目录前先确认进程未在跑——
    // 调用方（manager）已保证插件进程已停止）。
    emit_env_progress(
        app,
        &manifest.name,
        "creating-venv",
        Some(format!("基于 {} 重建虚拟环境", resolved.version_string)),
        None,
    );
    if venv_dir.exists() {
        std::fs::remove_dir_all(&venv_dir).map_err(|e| {
            PythonEnvError::VenvCreateFailed(format!("清理旧环境失败 {}: {e}", venv_dir.display()))
        })?;
    }
    if let Some(parent) = venv_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }

    run_timed(
        Command::new(&resolved.executable)
            .arg("-m")
            .arg("venv")
            .arg(&venv_dir)
            .current_dir(plugin_dir),
        VENV_CREATE_TIMEOUT,
        "创建虚拟环境",
    )
    .map_err(|e| PythonEnvError::VenvCreateFailed(e.to_string()))?;

    let venv_exe = venv_python(&venv_dir);
    if !venv_exe.is_file() {
        return Err(PythonEnvError::VenvCreateFailed(format!(
            "venv 创建后未找到解释器 {}",
            venv_exe.display()
        )));
    }

    // 安装依赖：pip install 带超时，输出透传为进度事件与错误详情。
    emit_env_progress(
        app,
        &manifest.name,
        "installing",
        Some(format!("安装 {} 个依赖", manifest.dependencies.len())),
        None,
    );
    let mut pip_cmd = Command::new(&venv_exe);
    pip_cmd
        .arg("-m")
        .arg("pip")
        .arg("install")
        .arg("--disable-pip-version-check")
        .arg("--no-input")
        .args(&manifest.dependencies)
        .current_dir(plugin_dir);

    run_timed(&mut pip_cmd, PIP_INSTALL_TIMEOUT, "pip install").map_err(|e| {
        emit_env_progress(app, &manifest.name, "failed", None, Some(e.clone()));
        PythonEnvError::PipInstallFailed(e)
    })?;

    std::fs::write(&fingerprint_path, &fingerprint)
        .map_err(|e| PythonEnvError::VenvCreateFailed(format!("写入环境指纹失败: {e}")))?;
    emit_env_progress(
        app,
        &manifest.name,
        "ready",
        Some("依赖安装完成".to_string()),
        None,
    );
    Ok(venv_exe)
}

/// 有超时的子进程执行，收集 stdout/stderr 供失败时透传。
/// pip 失败验收要求「给出清晰的 pip 输出」，stdout/stderr 都会进错误信息。
fn run_timed(cmd: &mut Command, timeout: Duration, label: &str) -> Result<(), String> {
    use std::io::Read;
    use std::thread;

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{label} 启动失败：{e}"))?;
    let deadline = std::time::Instant::now() + timeout;
    let stdout = child.stdout.take().map(|mut s| {
        thread::spawn(move || {
            let mut buf = String::new();
            let _ = s.read_to_string(&mut buf);
            buf
        })
    });
    let stderr = child.stderr.take().map(|mut s| {
        thread::spawn(move || {
            let mut buf = String::new();
            let _ = s.read_to_string(&mut buf);
            buf
        })
    });
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = stdout
                    .map(|h| h.join().unwrap_or_default())
                    .unwrap_or_default();
                let err = stderr
                    .map(|h| h.join().unwrap_or_default())
                    .unwrap_or_default();
                if status.success() {
                    return Ok(());
                }
                return Err(format!(
                    "{label} 失败（{status}）\nstdout:\n{}\nstderr:\n{}",
                    out.trim(),
                    err.trim()
                ));
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    return Err(format!("{label} 超时（{timeout:?}）"));
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(format!("{label} 状态查询失败：{e}")),
        }
    }
}

/// 删除插件时清理对应 venv。按 env_dir_name 匹配（名称+目录哈希），
/// 目录不存在 → no-op。
pub fn remove_plugin_environment(
    app: &AppHandle,
    manifest: &PluginManifest,
    plugin_dir: &Path,
) -> Result<(), PythonEnvError> {
    let root = crate::core::get_app_data_dir(app)
        .join("pluginEnvironments")
        .join(env_dir_name(manifest, plugin_dir));
    if root.exists() {
        std::fs::remove_dir_all(&root).map_err(|e| {
            PythonEnvError::Io(std::io::Error::new(
                e.kind(),
                format!("删除插件环境失败 {}: {e}", root.display()),
            ))
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolved(version: &str) -> ResolvedPython {
        ResolvedPython {
            executable: PathBuf::from("python"),
            version_string: format!("Python {version}"),
            source: PythonSource::Bundled,
        }
    }

    #[test]
    fn parses_version_tuple() {
        assert_eq!(resolved("3.11.9").version_tuple(), Some((3, 11, 9)));
        assert_eq!(resolved("3.9").version_tuple(), Some((3, 9, 0)));
    }

    #[test]
    fn constraint_bare_version_means_exact() {
        assert!(constraint_part_satisfied("3.11", (3, 11, 9)));
        assert!(!constraint_part_satisfied("3.11", (3, 12, 0)));
    }

    #[test]
    fn constraint_operators() {
        assert!(constraint_part_satisfied(">=3.10", (3, 11, 0)));
        assert!(!constraint_part_satisfied(">=3.12", (3, 11, 0)));
        assert!(constraint_part_satisfied("<3.13", (3, 12, 5)));
        assert!(constraint_part_satisfied("==3.11.*", (3, 11, 9)));
        assert!(!constraint_part_satisfied("==3.11.*", (3, 12, 0)));
        assert!(constraint_part_satisfied("~=3.11", (3, 11, 9)));
        assert!(!constraint_part_satisfied("~=3.11", (4, 0, 0)));
    }

    #[test]
    fn constraint_comma_separated() {
        let r = resolved("3.11.9");
        assert!(check_python_constraint(Some(">=3.10,<3.13"), &r).is_ok());
        assert!(check_python_constraint(Some(">=3.10,<3.11"), &r).is_err());
        assert!(check_python_constraint(None, &r).is_ok());
        assert!(check_python_constraint(Some(""), &r).is_ok());
    }

    #[test]
    fn fingerprint_changes_with_deps() {
        let r = resolved("3.11.9");
        let a = env_fingerprint(&r, &["numpy>=1.24".to_string()]);
        let b = env_fingerprint(&r, &["numpy>=1.25".to_string()]);
        let c = env_fingerprint(&r, &["numpy>=1.24".to_string()]);
        assert_ne!(a, b);
        assert_eq!(a, c);
    }
}
