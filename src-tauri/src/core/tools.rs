use std::path::{Path, PathBuf};

use tauri::AppHandle;

use crate::core::settings::load_settings;

/// 解析 FFmpeg 可执行文件路径。
///
/// 优先使用设置中的路径；该路径可以是可执行文件，也可以是包含
/// `ffmpeg` / `ffprobe` 的目录。未配置时回退到 PATH。
pub fn resolve_ffmpeg(app_handle: &AppHandle) -> Option<PathBuf> {
    resolve_binary(app_handle, "ffmpeg")
}

/// 解析 ffprobe 可执行文件路径。
pub fn resolve_ffprobe(app_handle: &AppHandle) -> Option<PathBuf> {
    resolve_binary(app_handle, "ffprobe")
}

fn resolve_binary(app_handle: &AppHandle, binary: &str) -> Option<PathBuf> {
    let configured = load_settings(app_handle)
        .ok()
        .and_then(|settings| settings.ffmpeg_path);

    if let Some(path) = configured {
        if path.is_dir() {
            if let Some(candidate) = existing_candidate(&path, binary) {
                return Some(candidate);
            }
        } else if path.is_file() {
            if binary == "ffmpeg" {
                return Some(path);
            }

            if let Some(candidate) = sibling_binary(&path, binary) {
                return Some(candidate);
            }
        }

        // 用户显式配置了路径但不可用时，不再静默回退到 PATH，
        // 这样错误信息能指向真正的问题。
        return None;
    }

    find_in_path(binary)
}

fn sibling_binary(ffmpeg_path: &Path, binary: &str) -> Option<PathBuf> {
    let parent = ffmpeg_path.parent()?;
    existing_candidate(parent, binary)
}

fn existing_candidate(dir: &Path, binary: &str) -> Option<PathBuf> {
    let names = if cfg!(windows) {
        vec![format!("{binary}.exe"), binary.to_string()]
    } else {
        vec![binary.to_string()]
    };

    names
        .into_iter()
        .map(|name| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn find_in_path(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names = if cfg!(windows) {
        vec![format!("{binary}.exe"), binary.to_string()]
    } else {
        vec![binary.to_string()]
    };

    for dir in std::env::split_paths(&path) {
        for name in &names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_binary_in_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let name = if cfg!(windows) {
            "ffmpeg.exe"
        } else {
            "ffmpeg"
        };
        let path = dir.path().join(name);
        std::fs::write(&path, b"").expect("write");
        assert_eq!(existing_candidate(dir.path(), "ffmpeg"), Some(path));
    }
}
