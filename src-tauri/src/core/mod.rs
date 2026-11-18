use std::path::{Path, PathBuf};
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
    settings::init(app_handle)?;
    // Resolve once, fail startup on errors, and cache the absolute root. Commands
    // must never silently turn an unavailable executable into a relative path.
    let root = resolve_plugin_dir()?;
    ensure_plugin_dir(&root)?;
    app_handle.manage(PluginDirectory { root });
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
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

struct PluginDirectory {
    root: PathBuf,
}

/// The only plugin root is `plugins` next to the running executable, just as
/// dependencies::install_directory resolves dependencies next to that executable.
/// `init` resolves and validates this absolute path before registering commands;
/// reading the cached state never falls back to CWD, appData or resource paths.
pub fn get_plugin_dir(app: &AppHandle) -> PathBuf {
    app.state::<PluginDirectory>().root.clone()
}

fn resolve_plugin_dir() -> Result<PathBuf, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("无法定位可执行文件目录：{error}"))?;
    plugin_dir_from_executable(&executable)
}

fn plugin_dir_from_executable(executable: &Path) -> Result<PathBuf, String> {
    if !executable.is_absolute() || executable.file_name().is_none() {
        return Err("无法定位可执行文件目录".to_owned());
    }
    executable
        .parent()
        .map(|parent| parent.join("plugins"))
        .ok_or_else(|| "无法定位可执行文件目录".to_owned())
}

/// Shared startup/open-directory validation. Never creates a relative directory
/// or falls back when the executable's parent is not writable.
pub(crate) fn ensure_plugin_dir(root: &Path) -> Result<(), String> {
    if !root.is_absolute() {
        return Err("插件目录必须是可执行文件旁的绝对路径".to_owned());
    }
    std::fs::create_dir_all(root)
        .map_err(|error| format!("无法创建插件目录 {}：{error}", root.display()))?;
    if !root.is_dir() {
        return Err(format!("插件目录不存在或不是目录：{}", root.display()));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_root_is_always_executable_adjacent() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("app").join("TimePin.exe");
        assert_eq!(
            plugin_dir_from_executable(&executable).unwrap(),
            directory.path().join("app/plugins")
        );
        assert!(plugin_dir_from_executable(Path::new("TimePin.exe")).is_err());
        assert!(plugin_dir_from_executable(Path::new("")).is_err());
    }

    #[test]
    fn startup_creates_only_the_unified_plugin_root_and_keeps_existing_plugins() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("plugins");
        ensure_plugin_dir(&root).unwrap();
        assert!(root.is_dir());
        let existing = root.join("existing-plugin");
        std::fs::create_dir(&existing).unwrap();
        std::fs::write(existing.join("plugin.json"), b"preserve").unwrap();
        ensure_plugin_dir(&root).unwrap();
        assert_eq!(
            std::fs::read(existing.join("plugin.json")).unwrap(),
            b"preserve"
        );
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn startup_reports_invalid_or_blocked_plugin_roots_without_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let blocked = directory.path().join("plugins");
        std::fs::write(&blocked, b"not a directory").unwrap();
        assert!(ensure_plugin_dir(&blocked)
            .unwrap_err()
            .contains("无法创建插件目录"));
        assert!(ensure_plugin_dir(&blocked.join("plugins")).is_err());
        assert!(ensure_plugin_dir(Path::new("plugins")).is_err());
        assert!(ensure_plugin_dir(Path::new("")).is_err());
        assert_eq!(std::fs::read(blocked).unwrap(), b"not a directory");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
