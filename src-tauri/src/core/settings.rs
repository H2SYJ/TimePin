use crate::core::get_app_data_dir;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::AppHandle;

#[derive(Debug, Clone, Serialize, Deserialize)]
// Unknown legacy fields (including customPluginDirs) are intentionally ignored.
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 视频导入目录
    pub import_directory: Option<PathBuf>,
    /// FFmpeg 路径（可选，自动检测；可为可执行文件或其所在 bin 目录）
    pub ffmpeg_path: Option<PathBuf>,
    /// FFprobe 路径（可选，显式配置优先于从 ffmpegPath 推导）
    pub ffprobe_path: Option<PathBuf>,
    /// 缩略图生成质量 (1-100)
    pub thumbnail_quality: u8,
    /// 缩略图最大尺寸
    pub thumbnail_max_dimension: u32,
    /// 并发导入数量
    pub max_concurrent_imports: usize,
    /// 默认导出模式
    pub default_export_mode: ExportMode,
    /// 语言设置
    pub language: String,
    /// 主题设置
    pub theme: Theme,
    /// 是否启用硬件加速
    pub hardware_acceleration: bool,
    /// 显式 Python 解释器路径（可选；绝对路径，最高优先级）
    pub python_path: Option<PathBuf>,
    /// 是否允许回退到系统 PATH 上的 python/python3（默认 false——
    /// 内嵌运行时缺失时直接报错，让用户明确选择）
    pub use_system_python: bool,
    /// 最近打开的文件列表
    pub recent_files: Vec<PathBuf>,
    /// 窗口状态
    pub window_state: Option<WindowState>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[derive(Default)]
pub enum ExportMode {
    /// 流复制模式（默认，极快，无损）
    #[default]
    StreamCopy,
    /// 帧精确模式（对首尾 GOP 重编码）
    FrameAccurate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowState {
    pub width: f64,
    pub height: f64,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub maximized: bool,
    pub fullscreen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            import_directory: None,
            ffmpeg_path: None,
            ffprobe_path: None,
            thumbnail_quality: 80,
            thumbnail_max_dimension: 320,
            max_concurrent_imports: 3,
            default_export_mode: ExportMode::default(),
            language: "zh-CN".to_string(),
            theme: Theme::default(),
            hardware_acceleration: true,
            python_path: None,
            use_system_python: false,
            recent_files: Vec::new(),
            window_state: None,
        }
    }
}

impl Settings {
    /// 校验设置值，避免损坏的配置进入持久化文件。
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=100).contains(&self.thumbnail_quality) {
            return Err("缩略图质量需在 1–100 之间".to_string());
        }
        if !(64..=1920).contains(&self.thumbnail_max_dimension) {
            return Err("缩略图最大尺寸需在 64–1920 之间".to_string());
        }
        if !(1..=8).contains(&self.max_concurrent_imports) {
            return Err("并发导入数量需在 1–8 之间".to_string());
        }
        if let Some(window) = &self.window_state {
            if !window.width.is_finite()
                || !window.height.is_finite()
                || window.width <= 0.0
                || window.height <= 0.0
            {
                return Err("窗口尺寸无效".to_string());
            }
        }
        validate_tool_path("ffmpegPath", self.ffmpeg_path.as_ref())?;
        validate_tool_path("ffprobePath", self.ffprobe_path.as_ref())?;
        validate_tool_path("pythonPath", self.python_path.as_ref())?;
        Ok(())
    }
}

fn validate_tool_path(field: &str, path: Option<&PathBuf>) -> Result<(), String> {
    let Some(path) = path else {
        return Ok(());
    };
    if !path.is_absolute() {
        return Err(format!("{field} 必须是绝对路径：{}", path.display()));
    }
    if path.as_os_str().is_empty() {
        return Err(format!("{field} 不能为空路径"));
    }
    Ok(())
}

/// Serializes save/update and settings-based plugin transitions. Lock order is
/// settings → install (if needed) → manager lifecycle → plugin/process.
/// Plain reads stay lock-free (atomic rename), including RPC host callbacks:
/// stopping a running process may wait for that RPC while holding this lock.
static SETTINGS_LOCK: Mutex<()> = Mutex::new(());

const SETTINGS_FILE: &str = "settings.json";

/// 获取设置文件路径
fn get_settings_path(app_handle: &AppHandle) -> PathBuf {
    get_app_data_dir(app_handle).join(SETTINGS_FILE)
}

pub(crate) fn settings_lock_guard() -> std::sync::MutexGuard<'static, ()> {
    SETTINGS_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 初始化设置模块
pub fn init(app_handle: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let _guard = settings_lock_guard();
    let settings_path = get_settings_path(app_handle);
    if !settings_path.exists() {
        let default_settings = Settings::default();
        save_settings_internal(app_handle, &default_settings)?;
    }
    Ok(())
}

/// 读取设置
pub fn load_settings(app_handle: &AppHandle) -> Result<Settings, Box<dyn std::error::Error>> {
    let settings_path = get_settings_path(app_handle);
    if !settings_path.exists() {
        return Ok(Settings::default());
    }
    let content = std::fs::read_to_string(settings_path)?;
    let settings: Settings = serde_json::from_str(&content)?;
    Ok(settings)
}

/// 保存设置（内部函数）
fn save_settings_internal(
    app_handle: &AppHandle,
    settings: &Settings,
) -> Result<(), Box<dyn std::error::Error>> {
    let settings_path = get_settings_path(app_handle);
    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    write_settings_file(&settings_path, settings)
}

fn write_settings_file(
    settings_path: &Path,
    settings: &Settings,
) -> Result<(), Box<dyn std::error::Error>> {
    use std::io::Write;

    let temporary_path = settings_path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let content = serde_json::to_vec_pretty(settings)?;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary_path)?;
        file.write_all(&content)?;
        file.sync_all()?;
        drop(file);
        // rename replaces the destination atomically, including on Windows.
        // Never remove the old configuration on failure.
        std::fs::rename(&temporary_path, settings_path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    result
}

/// 保存设置（对外暴露）
pub fn save_settings(
    app_handle: &AppHandle,
    settings: Settings,
) -> Result<(), Box<dyn std::error::Error>> {
    let _guard = settings_lock_guard();
    settings
        .validate()
        .map_err(|message| -> Box<dyn std::error::Error> { message.into() })?;
    save_settings_internal(app_handle, &settings)
}

/// 更新部分配置
pub fn update_settings<F>(
    app_handle: &AppHandle,
    updater: F,
) -> Result<Settings, Box<dyn std::error::Error>>
where
    F: FnOnce(&mut Settings),
{
    let _guard = settings_lock_guard();
    let mut settings = load_settings(app_handle)?;
    updater(&mut settings);
    settings
        .validate()
        .map_err(|message| -> Box<dyn std::error::Error> { message.into() })?;
    save_settings_internal(app_handle, &settings)?;
    Ok(settings)
}

/// 添加最近文件
pub fn add_recent_file(
    app_handle: &AppHandle,
    file_path: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    update_settings(app_handle, |settings| {
        settings.recent_files.retain(|p| p != &file_path);
        settings.recent_files.insert(0, file_path);
        if settings.recent_files.len() > 20 {
            settings.recent_files.truncate(20);
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_migrate_with_optional_ffprobe_and_preserve_fields() {
        let old = r#"{"language":"en-US","thumbnailQuality":92,"customPluginDirs":[]}"#;
        let settings: Settings = serde_json::from_str(old).unwrap();
        assert!(settings.ffprobe_path.is_none());
        assert!(settings.ffmpeg_path.is_none());
        assert_eq!(settings.language, "en-US");
        assert_eq!(settings.thumbnail_quality, 92);
        assert_eq!(settings.thumbnail_max_dimension, 320);
        let value = serde_json::to_value(&settings).unwrap();
        assert!(value.get("ffprobePath").unwrap().is_null());
        assert!(value.get("ffprobe_path").is_none());
        let defaults: Settings = serde_json::from_str("{}").unwrap();
        assert!(defaults.validate().is_ok());
    }

    #[test]
    fn legacy_custom_plugin_dirs_deserialize_but_are_never_reserialized() {
        for legacy in [
            r#"{"customPluginDirs":["/a","b"]}"#,
            r#"{"customPluginDirs":"not-an-array"}"#,
            r#"{"customPluginDirs":null}"#,
        ] {
            let settings: Settings = serde_json::from_str(legacy).unwrap();
            let value = serde_json::to_value(&settings).unwrap();
            assert!(value.get("customPluginDirs").is_none());
            assert!(value.get("custom_plugin_dirs").is_none());
        }
    }

    #[test]
    fn tool_paths_require_absolute_paths() {
        let mut settings = Settings {
            ffmpeg_path: Some(PathBuf::from("ffmpeg")),
            ..Settings::default()
        };
        assert!(settings.validate().is_err());
        settings.ffmpeg_path = None;
        settings.ffprobe_path = Some(PathBuf::from("ffprobe"));
        assert!(settings.validate().is_err());
    }

    #[test]
    fn plugin_snapshot_blocks_save_through_reload_without_blocking_plain_reads() {
        use std::sync::mpsc;
        use std::time::Duration;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings.json");
        let initial = Settings::default();
        write_settings_file(&path, &initial).unwrap();
        let (saving_tx, saving_rx) = mpsc::channel();
        let (saved_tx, saved_rx) = mpsc::channel();
        let snapshot_guard = settings_lock_guard();
        std::thread::scope(|scope| {
            let saver = scope.spawn(|| {
                saving_tx.send(()).unwrap();
                let _guard = settings_lock_guard();
                let changed = Settings {
                    language: "en-US".into(),
                    ..Settings::default()
                };
                write_settings_file(&path, &changed).unwrap();
                saved_tx.send(()).unwrap();
            });
            saving_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(saved_rx.try_recv().is_err());
            // RPC host services may still read atomically persisted settings
            // while stop waits for their process lock; these reads must not lock.
            let snapshot: Settings =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            assert_eq!(snapshot.language, "zh-CN");
            // This release represents the end of activation + reload + cleanup.
            drop(snapshot_guard);
            saver.join().unwrap();
            saved_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let saved: Settings = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(saved.language, "en-US");
    }

    #[test]
    fn atomic_settings_file_replaces_existing_and_cleans_failed_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut settings = Settings::default();
        write_settings_file(&path, &settings).unwrap();
        settings.language = "en-US".to_string();
        write_settings_file(&path, &settings).unwrap();
        let loaded: Settings = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(loaded.language, "en-US");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);

        // A destination directory cannot be replaced; keep it and remove only the temporary file.
        let blocked = dir.path().join("blocked.json");
        std::fs::create_dir(&blocked).unwrap();
        let original = blocked.join("existing-data");
        std::fs::write(&original, b"preserve").unwrap();
        assert!(write_settings_file(&blocked, &settings).is_err());
        assert_eq!(std::fs::read(original).unwrap(), b"preserve");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }
}
