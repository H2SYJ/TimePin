use std::path::PathBuf;
use tauri::{AppHandle, Manager};

pub mod dependencies;
pub mod github;
pub mod library;
pub mod marks;
pub mod mp4;
pub mod settings;
pub mod tools;

/// 初始化核心服务
pub fn init(app_handle: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    // 初始化设置
    settings::init(app_handle)?;
    // Always keep a writable, app-owned plugin root in addition to bundled and custom roots.
    std::fs::create_dir_all(get_default_plugins_dir(app_handle))?;
    // 初始化媒体库
    library::init(app_handle)?;
    Ok(())
}

/// 获取应用数据目录
///
/// `app_data_dir()` 返回 None 时退化到当前工作目录的 **绝对路径**，
/// 避免下游把 `.` 传给插件时触发 `path must be absolute` 校验。
pub fn get_app_data_dir(app_handle: &AppHandle) -> PathBuf {
    app_handle
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
        })
}

/// Default user plugin search root. Restoring defaults never moves existing plugins.
pub fn get_default_plugins_dir(app_handle: &AppHandle) -> PathBuf {
    get_app_data_dir(app_handle).join("plugins")
}

/// 获取视频存储目录
pub fn get_videos_dir(app_handle: &AppHandle) -> PathBuf {
    get_app_data_dir(app_handle).join("videos")
}

/// 获取缩略图存储目录
pub fn get_thumbnails_dir(app_handle: &AppHandle) -> PathBuf {
    get_app_data_dir(app_handle).join("thumbnails")
}

/// 获取伴随文件存储目录
pub fn get_sidecar_dir(app_handle: &AppHandle) -> PathBuf {
    get_app_data_dir(app_handle).join("sidecars")
}
