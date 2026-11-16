use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::core::mp4;
use crate::core::settings::load_settings;
use crate::core::tools::{resolve_ffmpeg, resolve_ffprobe};
use crate::core::{get_thumbnails_dir, get_videos_dir};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    pub id: String,
    pub name: String,
    /// 媒体库中的副本路径。原视频永远不会被修改。
    pub path: PathBuf,
    /// 用户导入时选择的原始文件路径。
    #[serde(default)]
    pub source_path: PathBuf,
    pub duration: f64, // 秒
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub codec: String,
    pub file_size: u64,
    pub imported_at: chrono::DateTime<chrono::Utc>,
    pub thumbnail_path: Option<PathBuf>,
    pub sidecar_txt_path: Option<PathBuf>,
    pub sidecar_csv_path: Option<PathBuf>,
    pub sidecar_xml_path: Option<PathBuf>,
    pub tags: Vec<String>,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub success: bool,
    pub video: Option<VideoInfo>,
    pub error: Option<String>,
    pub skipped: bool,
}

/// 初始化媒体库
pub fn init(app_handle: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let videos_dir = get_videos_dir(app_handle);
    let thumbnails_dir = get_thumbnails_dir(app_handle);

    std::fs::create_dir_all(&videos_dir)?;
    std::fs::create_dir_all(&thumbnails_dir)?;

    Ok(())
}

/// 导入视频文件。
///
/// 导入只会在应用数据目录创建一份副本，原始文件不会被修改或移动。
pub fn import_video(
    app_handle: &AppHandle,
    source_path: PathBuf,
) -> Result<ImportResult, Box<dyn std::error::Error>> {
    if !source_path.is_file() {
        return Ok(ImportResult {
            success: false,
            video: None,
            error: Some("文件不存在".to_string()),
            skipped: false,
        });
    }

    if !is_video_file(&source_path) {
        return Ok(ImportResult {
            success: false,
            video: None,
            error: Some("不支持的文件格式".to_string()),
            skipped: false,
        });
    }

    let videos_dir = get_videos_dir(app_handle);
    std::fs::create_dir_all(&videos_dir)?;

    let file_name = source_path
        .file_name()
        .ok_or("无效的文件名")?
        .to_string_lossy()
        .to_string();
    let dest_path = videos_dir.join(&file_name);

    if dest_path.exists() {
        if let Ok(existing) = get_video_by_path(app_handle, &dest_path) {
            return Ok(ImportResult {
                success: true,
                video: Some(existing),
                error: None,
                skipped: true,
            });
        }

        return Ok(ImportResult {
            success: false,
            video: None,
            error: Some("媒体库目录已存在同名文件，请先移除旧条目或重命名源文件".to_string()),
            skipped: false,
        });
    }

    std::fs::copy(&source_path, &dest_path)?;

    let metadata = match get_video_metadata(app_handle, &dest_path) {
        Ok(metadata) => metadata,
        Err(error) => {
            let _ = std::fs::remove_file(&dest_path);
            return Err(error);
        }
    };

    let thumbnail_path =
        generate_thumbnail(app_handle, &dest_path, &metadata.id, metadata.duration).ok();

    let video = VideoInfo {
        id: metadata.id.clone(),
        name: file_name,
        path: dest_path.clone(),
        source_path,
        duration: metadata.duration,
        width: metadata.width,
        height: metadata.height,
        fps: metadata.fps,
        codec: metadata.codec,
        file_size: metadata.file_size,
        imported_at: chrono::Utc::now(),
        thumbnail_path,
        sidecar_txt_path: Some(dest_path.with_extension("txt")),
        sidecar_csv_path: Some(dest_path.with_extension("csv")),
        sidecar_xml_path: Some(dest_path.with_extension("xml")),
        tags: Vec::new(),
        metadata: HashMap::new(),
    };

    save_video_info(app_handle, &video)?;

    // 最近文件只是便利信息，写入失败不应让一次成功的导入失败。
    let _ = crate::core::settings::add_recent_file(app_handle, video.source_path.clone());

    Ok(ImportResult {
        success: true,
        video: Some(video),
        error: None,
        skipped: false,
    })
}

/// 批量导入视频
pub fn import_videos(app_handle: &AppHandle, source_paths: Vec<PathBuf>) -> Vec<ImportResult> {
    source_paths
        .into_iter()
        .map(|path| {
            import_video(app_handle, path).unwrap_or_else(|error| ImportResult {
                success: false,
                video: None,
                error: Some(error.to_string()),
                skipped: false,
            })
        })
        .collect()
}

/// 获取所有视频列表
pub fn get_videos(app_handle: &AppHandle) -> Result<Vec<VideoInfo>, Box<dyn std::error::Error>> {
    let index_path = get_videos_dir(app_handle).join("index.json");
    if !index_path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(index_path)?;
    let videos: Vec<VideoInfo> = serde_json::from_str(&content)?;
    Ok(videos)
}

/// 根据路径获取视频信息
pub fn get_video_by_path(
    app_handle: &AppHandle,
    path: &Path,
) -> Result<VideoInfo, Box<dyn std::error::Error>> {
    let videos = get_videos(app_handle)?;
    videos
        .into_iter()
        .find(|video| video.path == path)
        .ok_or_else(|| "视频不存在".into())
}

/// 根据 ID 获取视频信息
pub fn get_video_by_id(
    app_handle: &AppHandle,
    id: &str,
) -> Result<VideoInfo, Box<dyn std::error::Error>> {
    let videos = get_videos(app_handle)?;
    videos
        .into_iter()
        .find(|video| video.id == id)
        .ok_or_else(|| "视频不存在".into())
}

/// 从媒体库移除视频及其副本、缩略图和伴随文件。
///
/// 原始视频（`source_path`）不会被删除。
pub fn delete_video(
    app_handle: &AppHandle,
    id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut videos = get_videos(app_handle)?;
    if let Some(index) = videos.iter().position(|video| video.id == id) {
        let video = videos.remove(index);

        if video.path.exists() {
            std::fs::remove_file(&video.path)?;
        }

        if let Some(thumbnail) = &video.thumbnail_path {
            if thumbnail.exists() {
                std::fs::remove_file(thumbnail)?;
            }
        }

        for sidecar in [
            &video.sidecar_txt_path,
            &video.sidecar_csv_path,
            &video.sidecar_xml_path,
        ] {
            if let Some(path) = sidecar {
                if path.exists() {
                    std::fs::remove_file(path)?;
                }
            }
        }

        save_videos_index(app_handle, &videos)?;
    }
    Ok(())
}

/// 保存视频信息到索引
fn save_video_info(
    app_handle: &AppHandle,
    video: &VideoInfo,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut videos = get_videos(app_handle)?;
    videos.retain(|existing| existing.id != video.id);
    videos.insert(0, video.clone());
    save_videos_index(app_handle, &videos)
}

/// 保存视频索引
fn save_videos_index(
    app_handle: &AppHandle,
    videos: &[VideoInfo],
) -> Result<(), Box<dyn std::error::Error>> {
    let index_path = get_videos_dir(app_handle).join("index.json");
    let content = serde_json::to_string_pretty(videos)?;
    std::fs::write(index_path, content)?;
    Ok(())
}

/// 检查是否为支持的视频文件
fn is_video_file(path: &Path) -> bool {
    if let Some(extension) = path.extension() {
        let extension = extension.to_string_lossy().to_lowercase();
        matches!(
            extension.as_str(),
            "mp4" | "mkv" | "mov" | "avi" | "flv" | "webm" | "m4v" | "ts" | "mts" | "m2ts"
        )
    } else {
        false
    }
}

/// 视频元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoMetadata {
    pub id: String,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub codec: String,
    pub file_size: u64,
}

#[derive(Debug)]
struct ProbeData {
    duration: f64,
    width: u32,
    height: u32,
    fps: f64,
    codec: String,
}

/// 获取视频元数据。
///
/// 优先使用 ffprobe；当 ffprobe 不存在或执行失败时，对 MP4/MOV
/// 使用内置的 `moov` 原子解析器降级，避免媒体库在未安装 FFmpeg
/// 的机器上完全不可用。
pub fn get_video_metadata(
    app_handle: &AppHandle,
    path: &Path,
) -> Result<VideoMetadata, Box<dyn std::error::Error>> {
    let file_size = std::fs::metadata(path)?.len();
    let id = uuid::Uuid::new_v4().to_string();

    let mut probe_error: Option<String> = None;

    if let Some(ffprobe) = resolve_ffprobe(app_handle) {
        match probe_with_ffprobe(&ffprobe, path) {
            Ok(Some(data)) => {
                return Ok(VideoMetadata {
                    id,
                    duration: data.duration,
                    width: data.width,
                    height: data.height,
                    fps: data.fps,
                    codec: data.codec,
                    file_size,
                });
            }
            Ok(None) => {
                probe_error = Some("ffprobe 未返回视频流信息".to_string());
            }
            Err(error) => {
                probe_error = Some(error.to_string());
            }
        }
    }

    if let Some(data) = mp4::probe(path)? {
        return Ok(VideoMetadata {
            id,
            duration: data.duration,
            width: data.width,
            height: data.height,
            fps: data.fps,
            codec: data.codec,
            file_size,
        });
    }

    let message = match probe_error {
        Some(error) => format!("无法读取视频元数据：{error}"),
        None => {
            "无法读取视频元数据：未找到 ffprobe，且该文件不是可解析的 MP4/MOV".to_string()
        }
    };
    Err(message.into())
}

fn probe_with_ffprobe(
    ffprobe: &Path,
    path: &Path,
) -> Result<Option<ProbeData>, Box<dyn std::error::Error>> {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate,codec_name,duration:format=duration",
            "-of",
            "json",
        ])
        .arg(path)
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffprobe 执行失败：{}", stderr.trim()).into());
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let Some(stream) = json["streams"].get(0) else {
        return Ok(None);
    };

    let width = stream["width"].as_u64().unwrap_or(0) as u32;
    let height = stream["height"].as_u64().unwrap_or(0) as u32;
    let codec = stream["codec_name"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    let duration = stream["duration"]
        .as_str()
        .and_then(parse_duration)
        .or_else(|| json["format"]["duration"].as_str().and_then(parse_duration))
        .unwrap_or(0.0);

    let fps = stream["r_frame_rate"]
        .as_str()
        .and_then(parse_fps)
        .unwrap_or(0.0);

    Ok(Some(ProbeData {
        duration,
        width,
        height,
        fps,
        codec,
    }))
}

fn parse_duration(value: &str) -> Option<f64> {
    let parsed = value.parse::<f64>().ok()?;
    if parsed.is_finite() && parsed >= 0.0 {
        Some(parsed)
    } else {
        None
    }
}

/// 解析帧率字符串 (如 "30000/1001")
fn parse_fps(fps_str: &str) -> Option<f64> {
    let parts: Vec<&str> = fps_str.split('/').collect();
    if parts.len() == 2 {
        let numerator = parts[0].parse::<f64>().ok()?;
        let denominator = parts[1].parse::<f64>().ok()?;
        if denominator != 0.0 {
            let fps = numerator / denominator;
            return (fps.is_finite() && fps > 0.0).then_some(fps);
        }
        return None;
    }

    let fps = fps_str.parse::<f64>().ok()?;
    (fps.is_finite() && fps > 0.0).then_some(fps)
}

/// 生成缩略图
pub fn generate_thumbnail(
    app_handle: &AppHandle,
    video_path: &Path,
    video_id: &str,
    duration: f64,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let ffmpeg = resolve_ffmpeg(app_handle)
        .ok_or("未找到 FFmpeg，请在设置中配置 ffmpeg 路径或将其加入 PATH")?;

    let thumbnails_dir = get_thumbnails_dir(app_handle);
    std::fs::create_dir_all(&thumbnails_dir)?;
    let thumbnail_path = thumbnails_dir.join(format!("{video_id}.jpg"));

    let settings = load_settings(app_handle).unwrap_or_default();
    let max_dimension = u32::from(settings.thumbnail_max_dimension.clamp(64, 1920));
    let quality = u32::from(settings.thumbnail_quality.clamp(1, 100));
    let qscale = (31.0 - (quality as f64 / 100.0) * 29.0)
        .round()
        .clamp(2.0, 31.0) as u32;
    let seek_time = if duration.is_finite() && duration > 0.0 {
        duration * 0.1
    } else {
        0.0
    };
    let scale_filter = format!(
        "scale=w='min(iw,{max_dimension})':h='min(ih,{max_dimension})':force_original_aspect_ratio=decrease"
    );

    let output = Command::new(ffmpeg)
        .args(["-y", "-hide_banner", "-loglevel", "error"])
        .arg("-ss")
        .arg(format!("{seek_time:.3}"))
        .arg("-i")
        .arg(video_path)
        .args(["-frames:v", "1", "-vf", &scale_filter, "-q:v", &qscale.to_string()])
        .arg(&thumbnail_path)
        .output()?;

    if output.status.success() && thumbnail_path.exists() {
        Ok(thumbnail_path)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("缩略图生成失败：{}", stderr.trim()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rational_fps() {
        let fps = parse_fps("30000/1001").expect("fps");
        assert!((fps - 29.970_029_97).abs() < 0.001);
    }

    #[test]
    fn rejects_invalid_fps() {
        assert_eq!(parse_fps("0/0"), None);
        assert_eq!(parse_fps("abc"), None);
    }

    #[test]
    fn recognizes_video_extensions() {
        assert!(is_video_file(Path::new("clip.MP4")));
        assert!(is_video_file(Path::new("clip.mkv")));
        assert!(!is_video_file(Path::new("clip.txt")));
    }
}
