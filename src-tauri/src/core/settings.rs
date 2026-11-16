use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;
use crate::core::get_app_data_dir;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// 视频导入目录
    pub import_directory: Option<PathBuf>,
    /// FFmpeg 路径（可选，自动检测）
    pub ffmpeg_path: Option<PathBuf>,
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
    /// 自定义插件目录
    pub custom_plugin_dirs: Vec<PathBuf>,
    /// 最近打开的文件列表
    pub recent_files: Vec<PathBuf>,
    /// 窗口状态
    pub window_state: Option<WindowState>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ExportMode {
    /// 流复制模式（默认，极快，无损）
    StreamCopy,
    /// 帧精确模式（对首尾 GOP 重编码）
    FrameAccurate,
}

impl Default for ExportMode {
    fn default() -> Self {
        ExportMode::StreamCopy
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    System,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::System
    }
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
            thumbnail_quality: 80,
            thumbnail_max_dimension: 320,
            max_concurrent_imports: 3,
            default_export_mode: ExportMode::default(),
            language: "zh-CN".to_string(),
            theme: Theme::default(),
            hardware_acceleration: true,
            custom_plugin_dirs: Vec::new(),
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
            if !window.width.is_finite() || !window.height.is_finite() || window.width <= 0.0 || window.height <= 0.0 {
                return Err("窗口尺寸无效".to_string());
            }
        }
        Ok(())
    }
}

const SETTINGS_FILE: &str = "settings.json";

/// 获取设置文件路径
fn get_settings_path(app_handle: &AppHandle) -> PathBuf {
    get_app_data_dir(app_handle).join(SETTINGS_FILE)
}

/// 初始化设置模块
pub fn init(app_handle: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
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
fn save_settings_internal(app_handle: &AppHandle, settings: &Settings) -> Result<(), Box<dyn std::error::Error>> {
    let settings_path = get_settings_path(app_handle);
    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(settings)?;
    std::fs::write(settings_path, content)?;
    Ok(())
}

/// 保存设置（对外暴露）
pub fn save_settings(app_handle: &AppHandle, settings: Settings) -> Result<(), Box<dyn std::error::Error>> {
    settings.validate().map_err(|message| -> Box<dyn std::error::Error> { message.into() })?;
    save_settings_internal(app_handle, &settings)
}

/// 更新部分配置
pub fn update_settings<F>(app_handle: &AppHandle, updater: F) -> Result<Settings, Box<dyn std::error::Error>>
where
    F: FnOnce(&mut Settings),
{
    let mut settings = load_settings(app_handle)?;
    updater(&mut settings);
    settings.validate().map_err(|message| -> Box<dyn std::error::Error> { message.into() })?;
    save_settings_internal(app_handle, &settings)?;
    Ok(settings)
}

/// 添加最近文件
pub fn add_recent_file(app_handle: &AppHandle, file_path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    update_settings(app_handle, |settings| {
        settings.recent_files.retain(|p| p != &file_path);
        settings.recent_files.insert(0, file_path);
        if settings.recent_files.len() > 20 {
            settings.recent_files.truncate(20);
        }
    })?;
    Ok(())
}