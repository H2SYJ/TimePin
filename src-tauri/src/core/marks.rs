//! Versioned manual marks, stored only below the application data directory.

use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::core::library::{load_videos_index, VideoInfo};
use crate::formats::Mark;

pub const MARKS_SCHEMA_VERSION: &str = "1.0.0";

// Guard the entire read/validate/replace transaction, including readers. This is
// deliberately process-wide so independent command invocations share one lock.
static MARKS_IO_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarksDocument {
    pub schema_version: String,
    pub video_id: String,
    pub marks: Vec<Mark>,
}

impl MarksDocument {
    fn new(video_id: &str, marks: Vec<Mark>) -> Self {
        Self {
            schema_version: MARKS_SCHEMA_VERSION.to_string(),
            video_id: video_id.to_string(),
            marks,
        }
    }
}

pub fn get_marks(app_handle: &AppHandle, video_id: &str) -> Result<MarksDocument> {
    // Unlike the legacy directory helper, never fall back to the working
    // directory if the platform cannot resolve the application data directory.
    let data_dir = app_handle.path().app_data_dir()?;
    load_from_dir(&data_dir, video_id)
}

pub fn save_marks(
    app_handle: &AppHandle,
    video_id: &str,
    marks: Vec<Mark>,
) -> Result<MarksDocument> {
    let data_dir = app_handle.path().app_data_dir()?;
    save_to_dir(&data_dir, video_id, marks)
}

// These file functions need neither a Tauri runtime nor a real video/FFmpeg.
fn load_from_dir(data_dir: &Path, video_id: &str) -> Result<MarksDocument> {
    validate_video_id(video_id)?;
    let _guard = MARKS_IO_LOCK
        .lock()
        .map_err(|_| anyhow!("标注存储锁不可用，请重启应用"))?;
    let video = library_video(data_dir, video_id)?;
    read_document(&data_dir.join("marks"), &video)
}

fn save_to_dir(data_dir: &Path, video_id: &str, marks: Vec<Mark>) -> Result<MarksDocument> {
    validate_video_id(video_id)?;
    let _guard = MARKS_IO_LOCK
        .lock()
        .map_err(|_| anyhow!("标注存储锁不可用，请重启应用"))?;
    let video = library_video(data_dir, video_id)?;
    let document = MarksDocument::new(video_id, marks);
    validate_document(&document, &video)?;

    let marks_dir = data_dir.join("marks");
    // Do not let an autosave erase malformed, mismatched or newer documents.
    // Only a genuinely missing file is treated as an empty document.
    read_document(&marks_dir, &video)?;
    let content = serde_json::to_vec_pretty(&document).context("无法序列化标注文档")?;
    atomic_write(&marks_dir.join(format!("{video_id}.json")), &content)?;
    Ok(document)
}

fn library_video(data_dir: &Path, video_id: &str) -> Result<VideoInfo> {
    let videos = load_videos_index(&data_dir.join("videos"))
        .map_err(|error| anyhow!("无法读取媒体库：{error}"))?;
    videos
        .into_iter()
        .find(|video| video.id == video_id)
        .ok_or_else(|| anyhow!("媒体库中不存在视频：{video_id}"))
}

fn validate_video_id(video_id: &str) -> Result<()> {
    // A single portable filename component, including Windows device names.
    if video_id.is_empty()
        || video_id.len() > 128
        || !video_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("无效的视频 ID：仅允许 1–128 个 ASCII 字母、数字、连字符和下划线");
    }
    let upper = video_id.to_ascii_uppercase();
    let numbered_device = upper
        .strip_prefix("COM")
        .or_else(|| upper.strip_prefix("LPT"))
        .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'));
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered_device {
        bail!("无效的视频 ID：不允许 Windows 保留设备名");
    }
    Ok(())
}

fn validate_document(document: &MarksDocument, video: &VideoInfo) -> Result<()> {
    if document.schema_version != MARKS_SCHEMA_VERSION {
        bail!("不支持的标注文档版本：{}", document.schema_version);
    }
    if document.video_id != video.id {
        bail!("标注文档的视频 ID 与媒体库不匹配");
    }

    // The library uses zero when probing cannot determine the duration.
    let known_duration = video.duration.is_finite() && video.duration > 0.0;
    let mut ids = HashSet::new();
    for mark in &document.marks {
        if mark.id.trim().is_empty() || !ids.insert(mark.id.as_str()) {
            bail!("标注 ID 为空或重复：{}", mark.id);
        }
        if mark.label.trim().is_empty() {
            bail!("标注「{}」的标签不能为空", mark.id);
        }
        if !mark.start_time.is_finite() || mark.start_time < 0.0 {
            bail!("标注「{}」的开始时间必须是非负有限数", mark.id);
        }
        if known_duration && mark.start_time > video.duration {
            bail!("标注「{}」的开始时间超出视频时长", mark.id);
        }
        if let Some(end) = mark.end_time {
            if !end.is_finite() || end <= mark.start_time {
                bail!("标注「{}」的结束时间必须是大于开始时间的有限数", mark.id);
            }
            if known_duration && end > video.duration {
                bail!("标注「{}」的结束时间超出视频时长", mark.id);
            }
        }
        if let Some(confidence) = mark.confidence {
            if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
                bail!("标注「{}」的置信度必须是 0–1 之间的有限数", mark.id);
            }
        }
    }
    Ok(())
}

fn read_document(marks_dir: &Path, video: &VideoInfo) -> Result<MarksDocument> {
    let path = marks_dir.join(format!("{}.json", video.id));
    let content = match fs::read(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(MarksDocument::new(&video.id, Vec::new()));
        }
        Err(error) => return Err(error).context("无法读取标注文档"),
    };
    let document: MarksDocument = serde_json::from_slice(&content)
        .context("标注文档 JSON 损坏；请备份并修复该文件，保存操作不会覆盖它")?;
    validate_document(&document, video).context("已有标注文档校验失败；保存操作不会覆盖它")?;
    Ok(document)
}

fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let parent = path.parent().context("标注文档缺少父目录")?;
    fs::create_dir_all(parent).context("无法创建标注目录")?;
    let temporary_path = parent.join(format!(".marks-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .context("无法创建标注临时文件")?;
    let write_result = file
        .write_all(content)
        .context("无法写入标注临时文件")
        .and_then(|()| file.sync_all().context("无法同步标注临时文件"));
    // Close the handle before rename/remove (important on Windows).
    drop(file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }
    // std::fs::rename replaces existing files on Windows too; never remove the
    // old document first, so a failed rename leaves the last good file in
    // place. A leftover .tmp from a hard crash is harmless: readers never load it.
    fs::rename(&temporary_path, path).context("无法原子替换标注文档")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_video(video_id: &str, duration: f64) -> serde_json::Value {
        // Only the fields read by validation/path resolution need real values.
        serde_json::json!([{
            "id": video_id,
            "name": "sample.mp4",
            "path": "videos/sample.mp4",
            "sourcePath": "C:/source/sample.mp4",
            "duration": duration,
            "width": 1920,
            "height": 1080,
            "fps": 30.0,
            "codec": "h264",
            "fileSize": 1024,
            "importedAt": "2026-01-01T00:00:00Z",
            "thumbnailPath": null,
            "sidecarTxtPath": null,
            "sidecarCsvPath": null,
            "sidecarXmlPath": null,
            "tags": [],
            "metadata": {}
        }])
    }

    fn fixture_index() -> tempfile::TempDir {
        let data_dir = tempfile::tempdir().expect("tempdir");
        let videos_dir = data_dir.path().join("videos");
        fs::create_dir_all(&videos_dir).expect("videos dir");
        fs::write(
            videos_dir.join("index.json"),
            serde_json::to_vec(&index_video("video-1", 120.0)).expect("index json"),
        )
        .expect("write index");
        data_dir
    }

    fn range_mark(id: &str, start: f64, end: f64, label: &str) -> Mark {
        Mark {
            id: id.to_string(),
            start_time: start,
            end_time: Some(end),
            label: label.to_string(),
            color: None,
            confidence: None,
            source_plugin: Some("manual".to_string()),
            thumbnail_path: None,
            metadata: Default::default(),
        }
    }

    #[test]
    fn rejects_unsafe_or_empty_video_ids() {
        for id in [
            "", "..", "a/b", "a\\b", "a:b", "*", "?", ".", "NUL", "com1", "  x",
        ] {
            assert!(validate_video_id(id).is_err(), "accepted {id:?}");
        }
        assert!(validate_video_id(&"a".repeat(128)).is_ok());
        assert!(validate_video_id(&"a".repeat(129)).is_err());
        assert!(validate_video_id("video-01_AB").is_ok());
    }

    #[test]
    fn missing_marks_file_returns_empty_document() {
        let data_dir = fixture_index();
        let document = load_from_dir(data_dir.path(), "video-1").expect("load");
        assert_eq!(document.schema_version, MARKS_SCHEMA_VERSION);
        assert_eq!(document.video_id, "video-1");
        assert!(document.marks.is_empty());
    }

    #[test]
    fn rejects_video_not_in_library() {
        let data_dir = fixture_index();
        assert!(load_from_dir(data_dir.path(), "unknown-video").is_err());
        assert!(save_to_dir(data_dir.path(), "unknown-video", vec![]).is_err());
    }

    #[test]
    fn saves_and_reads_back_document() {
        let data_dir = fixture_index();
        let marks = vec![
            Mark::new_point(5.0, "开场".to_string()),
            range_mark("range-1", 10.0, 20.0, "战斗"),
        ];
        let saved = save_to_dir(data_dir.path(), "video-1", marks.clone()).expect("save");
        assert_eq!(saved.schema_version, MARKS_SCHEMA_VERSION);
        assert_eq!(saved.marks.len(), 2);
        assert!(data_dir.path().join("marks/video-1.json").is_file());

        let loaded = load_from_dir(data_dir.path(), "video-1").expect("load");
        assert_eq!(loaded.marks.len(), 2);
        assert_eq!(loaded.marks[1].label, "战斗");
        assert_eq!(loaded.marks[1].end_time, Some(20.0));
    }

    #[test]
    fn corrupted_document_is_never_overwritten() {
        let data_dir = fixture_index();
        let marks_dir = data_dir.path().join("marks");
        fs::create_dir_all(&marks_dir).expect("marks dir");
        let path = marks_dir.join("video-1.json");
        fs::write(&path, b"{ not valid json").expect("corrupt file");

        // Reads surface the corruption error...
        let error = load_from_dir(data_dir.path(), "video-1").expect_err("corrupt load");
        assert!(error.to_string().contains("损坏"));
        // ...and saves refuse to clobber it.
        let error = save_to_dir(data_dir.path(), "video-1", vec![])
            .expect_err("save must fail on corrupt file");
        assert!(error.to_string().contains("损坏") || error.to_string().contains("校验"));
        assert_eq!(fs::read(&path).expect("file"), b"{ not valid json");
    }

    #[test]
    fn mismatched_schema_version_is_rejected_and_preserved() {
        let data_dir = fixture_index();
        let marks_dir = data_dir.path().join("marks");
        fs::create_dir_all(&marks_dir).expect("marks dir");
        fs::write(
            marks_dir.join("video-1.json"),
            br#"{"schemaVersion":"9.9.9","videoId":"video-1","marks":[]}"#,
        )
        .expect("write stale document");
        assert!(load_from_dir(data_dir.path(), "video-1").is_err());
        assert!(save_to_dir(data_dir.path(), "video-1", vec![]).is_err());
    }

    #[test]
    fn validates_mark_invariants() {
        let data_dir = fixture_index();
        let duration = 120.0;

        let duplicate = vec![
            range_mark("m1", 1.0, 2.0, "a"),
            range_mark("m1", 3.0, 4.0, "b"),
        ];
        assert!(save_to_dir(data_dir.path(), "video-1", duplicate).is_err());

        let negative = vec![Mark::new_point(-1.0, "负时间".to_string())];
        assert!(save_to_dir(data_dir.path(), "video-1", negative).is_err());

        let infinite = vec![Mark::new_point(f64::INFINITY, "inf".to_string())];
        assert!(save_to_dir(data_dir.path(), "video-1", infinite).is_err());

        let beyond = vec![Mark::new_point(duration + 1.0, "越界".to_string())];
        assert!(save_to_dir(data_dir.path(), "video-1", beyond).is_err());

        let blank_label = vec![range_mark("m2", 1.0, 2.0, "   ")];
        assert!(save_to_dir(data_dir.path(), "video-1", blank_label).is_err());

        let reversed = vec![range_mark("m3", 5.0, 5.0, "零长")];
        assert!(save_to_dir(data_dir.path(), "video-1", reversed).is_err());
        let end_before = vec![range_mark("m4", 6.0, 4.0, "反向")];
        assert!(save_to_dir(data_dir.path(), "video-1", end_before).is_err());

        let mut bad_confidence = range_mark("m5", 1.0, 2.0, "置信度");
        bad_confidence.confidence = Some(1.5);
        assert!(save_to_dir(data_dir.path(), "video-1", vec![bad_confidence]).is_err());

        // End exactly at duration is still valid.
        assert!(save_to_dir(
            data_dir.path(),
            "video-1",
            vec![range_mark("ok", duration - 1.0, duration, "边界")]
        )
        .is_ok());
    }

    #[test]
    fn concurrent_saves_preserve_last_writer() {
        let data_dir = fixture_index();
        let shared = data_dir.path().to_path_buf();
        let mut handles = Vec::new();
        for index in 0..4 {
            let dir = shared.clone();
            handles.push(std::thread::spawn(move || {
                let mark = range_mark(
                    &format!("writer-{index}"),
                    index as f64,
                    index as f64 + 1.0,
                    "并发",
                );
                save_to_dir(&dir, "video-1", vec![mark]).expect("save");
            }));
        }
        for handle in handles {
            handle.join().expect("join");
        }
        let loaded = load_from_dir(data_dir.path(), "video-1").expect("load");
        assert_eq!(loaded.marks.len(), 1);
        assert!(loaded.marks[0].id.starts_with("writer-"));
    }
}
