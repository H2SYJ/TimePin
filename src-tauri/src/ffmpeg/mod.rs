use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::AppHandle;

use crate::core::settings::ExportMode;
use crate::core::tools::resolve_ffmpeg;
use crate::formats::Mark;

/// 导出视频片段。
///
/// 默认使用 FFmpeg 流复制（`-c copy`），不修改原视频。帧精确模式需要
/// 对首尾 GOP 做智能切分，M1 尚未实现，因此这里明确拒绝，而不是偷偷
/// 对整个片段重新编码。
pub fn export_segments(
    app_handle: &AppHandle,
    video_path: &Path,
    marks: &[Mark],
    output_dir: &Path,
    export_mode: ExportMode,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    if export_mode == ExportMode::FrameAccurate {
        return Err(
            "帧精确导出尚未在 M1 开放；当前请使用流复制模式（-c copy），它不会重编码视频".into(),
        );
    }

    let ffmpeg = resolve_ffmpeg(app_handle)
        .ok_or("未找到 FFmpeg，请在设置中配置 ffmpeg 路径或将其加入 PATH")?;

    std::fs::create_dir_all(output_dir)?;

    let video_stem = video_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("video");

    let mut exported_paths = Vec::new();

    for (index, mark) in marks.iter().enumerate() {
        let Some(end_time) = mark.end_time else {
            continue;
        };

        let start_time = mark.start_time;
        let duration = end_time - start_time;

        if !start_time.is_finite()
            || !end_time.is_finite()
            || start_time < 0.0
            || duration <= 0.0
        {
            continue;
        }

        let safe_label = sanitize_filename(&mark.label);
        let output_filename = format!("{}_{:03}_{}.mp4", video_stem, index + 1, safe_label);
        let output_path = output_dir.join(output_filename);

        let ffmpeg_args = build_ffmpeg_args(video_path, start_time, end_time, &output_path, export_mode)?;

        let output = Command::new(&ffmpeg).args(&ffmpeg_args).output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("FFmpeg 导出失败：{}", stderr.trim()).into());
        }

        exported_paths.push(output_path);
    }

    Ok(exported_paths)
}

/// 构建 FFmpeg 参数。
///
/// 参数使用 `OsString`，避免 Windows 中文路径因 `to_str()` 失败而 panic。
fn build_ffmpeg_args(
    input: &Path,
    start: f64,
    end: f64,
    output: &Path,
    mode: ExportMode,
) -> Result<Vec<OsString>, Box<dyn std::error::Error>> {
    if mode == ExportMode::FrameAccurate {
        return Err("帧精确导出尚未实现".into());
    }

    let mut args: Vec<OsString> = vec![
        "-y".into(),
        "-ss".into(),
        format!("{start:.3}").into(),
        "-i".into(),
        input.as_os_str().to_os_string(),
        "-t".into(),
        format!("{:.3}", end - start).into(),
    ];

    // 流复制：极快、无损，并且不会修改输入文件。
    args.extend([
        "-c".into(),
        "copy".into(),
        "-avoid_negative_ts".into(),
        "make_zero".into(),
    ]);

    args.push(output.as_os_str().to_os_string());
    Ok(args)
}

/// 清理文件名中的非法字符
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .take(50)
        .collect();

    let cleaned = cleaned
        .trim()
        .trim_end_matches(|character| character == '.' || character == ' ')
        .to_string();
    if cleaned.is_empty() {
        "clip".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_copy_args_never_reencode() {
        let args = build_ffmpeg_args(
            Path::new("input.mp4"),
            1.0,
            2.5,
            Path::new("output.mp4"),
            ExportMode::StreamCopy,
        )
        .expect("args");

        let args: Vec<String> = args
            .iter()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect();

        assert!(args.contains(&"-c".to_string()));
        assert!(args.contains(&"copy".to_string()));
        assert!(!args.contains(&"libx264".to_string()));
    }

    #[test]
    fn frame_accurate_is_rejected_until_smart_cut_exists() {
        let result = build_ffmpeg_args(
            Path::new("input.mp4"),
            1.0,
            2.5,
            Path::new("output.mp4"),
            ExportMode::FrameAccurate,
        );
        assert!(result.is_err());
    }

    #[test]
    fn sanitizes_invalid_filename_characters() {
        assert_eq!(sanitize_filename("a/b:c*?"), "a_b_c__");
        assert_eq!(sanitize_filename("   "), "clip");
    }
}
