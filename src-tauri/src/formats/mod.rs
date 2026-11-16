use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    pub id: String,
    pub start_time: f64,       // 秒
    pub end_time: Option<f64>, // 秒，None 表示时间点
    pub label: String,
    pub color: Option<String>,   // 十六进制颜色
    pub confidence: Option<f32>, // 0.0-1.0
    pub source_plugin: Option<String>,
    pub thumbnail_path: Option<PathBuf>,
    pub metadata: std::collections::HashMap<String, String>,
}

impl Mark {
    pub fn new_point(time: f64, label: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            start_time: time,
            end_time: None,
            label,
            color: None,
            confidence: None,
            source_plugin: None,
            thumbnail_path: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    pub fn new_range(start: f64, end: f64, label: String) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            start_time: start,
            end_time: Some(end),
            label,
            color: None,
            confidence: None,
            source_plugin: None,
            thumbnail_path: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    pub fn is_point(&self) -> bool {
        self.end_time.is_none()
    }

    pub fn is_range(&self) -> bool {
        self.end_time.is_some()
    }

    pub fn duration(&self) -> f64 {
        self.end_time.unwrap_or(self.start_time) - self.start_time
    }
}

/// 将秒数格式化为 HH:MM:SS,ddd 格式（OGM 章节格式）
pub fn format_time_ogm(seconds: f64) -> String {
    let total_ms = seconds_to_millis(seconds).unwrap_or(0);
    let hours = total_ms / 3_600_000;
    let minutes = (total_ms % 3_600_000) / 60_000;
    let secs = (total_ms % 60_000) / 1000;
    let ms = total_ms % 1000;
    format!("{hours:02}:{minutes:02}:{secs:02},{ms:03}")
}

/// 将秒数格式化为 HH:MM:SS.ddd 格式（Premiere CSV 格式）
pub fn format_time_premiere(seconds: f64) -> String {
    let total_ms = seconds_to_millis(seconds).unwrap_or(0);
    let hours = total_ms / 3_600_000;
    let minutes = (total_ms % 3_600_000) / 60_000;
    let secs = (total_ms % 60_000) / 1000;
    let ms = total_ms % 1000;
    format!("{hours:02}:{minutes:02}:{secs:02}.{ms:03}")
}

fn seconds_to_millis(seconds: f64) -> Option<i64> {
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some((seconds * 1000.0).round() as i64)
}

/// 验证 OGM 时间字符串，例如 `00:01:02,345`。
pub fn validate_ogm_time_format(time_str: &str) -> bool {
    let Some((time_part, ms_part)) = time_str.split_once(',') else {
        return false;
    };

    if ms_part.len() != 3 || !ms_part.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }

    let time_parts: Vec<&str> = time_part.split(':').collect();
    if time_parts.len() != 3 {
        return false;
    }

    time_parts
        .iter()
        .all(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_digit()))
}

/// 验证一整行 OGM 章节，格式必须是 `HH:MM:SS,ddd 标题`。
pub fn validate_ogm_line(line: &str) -> bool {
    let Some((time, title)) = line.split_once(' ') else {
        return false;
    };

    !title.trim().is_empty()
        && !title.contains('\n')
        && !title.contains('\r')
        && validate_ogm_time_format(time)
}

fn normalized_label(label: &str) -> String {
    let label = label.replace(['\r', '\n'], " ");
    let label = label.trim();
    if label.is_empty() {
        "未命名标注".to_string()
    } else {
        label.to_string()
    }
}

fn sorted_marks(marks: &[Mark]) -> Vec<Mark> {
    let mut sorted = marks.to_vec();
    sorted.sort_by(|a, b| a.start_time.total_cmp(&b.start_time));
    sorted
}

fn validate_marks(marks: &[Mark], video_duration: f64) -> Result<(), String> {
    for mark in marks {
        if !mark.start_time.is_finite() || mark.start_time < 0.0 {
            return Err(format!("标注「{}」的开始时间无效", mark.label));
        }

        if let Some(end) = mark.end_time {
            if !end.is_finite() || end < mark.start_time {
                return Err(format!("标注「{}」的结束时间无效", mark.label));
            }
        }

        if video_duration.is_finite()
            && video_duration > 0.0
            && mark.start_time > video_duration + 1.0
        {
            return Err(format!(
                "标注「{}」超出视频时长（{:.3}s）",
                mark.label, video_duration
            ));
        }
    }

    Ok(())
}

/// 生成 video.txt (OGM 章节格式)
///
/// 每行严格为 `HH:MM:SS,ddd 标题`，首个章节必须从 `00:00:00,000` 开始。
pub fn generate_txt_chapters(
    marks: &[Mark],
    video_duration: f64,
) -> Result<String, Box<dyn std::error::Error>> {
    validate_marks(marks, video_duration)?;

    let sorted = sorted_marks(marks);
    let mut lines = Vec::new();

    let has_zero_chapter = sorted
        .iter()
        .any(|mark| mark.start_time.abs() < f64::EPSILON);
    if !has_zero_chapter {
        lines.push("00:00:00,000 视频开始".to_string());
    }

    for mark in &sorted {
        let label = normalized_label(&mark.label);
        lines.push(format!("{} {}", format_time_ogm(mark.start_time), label));

        if let Some(end) = mark.end_time {
            // 零长度区间按时间点处理，避免生成重复章节。
            if end > mark.start_time {
                lines.push(format!("{} {} (结束)", format_time_ogm(end), label));
            }
        }
    }

    for line in &lines {
        if !validate_ogm_line(line) {
            return Err(format!("无效的 OGM 章节行：{line}").into());
        }
    }

    Ok(lines.join("\n"))
}

/// 生成 video.csv (Premiere Pro 标记格式)
///
/// 列名固定为 `Name,In,Out,Duration`。
pub fn generate_csv_markers(
    marks: &[Mark],
    video_duration: f64,
) -> Result<String, Box<dyn std::error::Error>> {
    validate_marks(marks, video_duration)?;

    let mut lines = vec!["Name,In,Out,Duration".to_string()];

    for mark in sorted_marks(marks) {
        let in_time = format_time_premiere(mark.start_time);
        let (out_time, duration) = if let Some(end) = mark.end_time {
            (
                format_time_premiere(end),
                format_time_premiere((end - mark.start_time).max(0.0)),
            )
        } else {
            (in_time.clone(), "00:00:00.000".to_string())
        };

        lines.push(format!(
            "{},{},{},{}",
            escape_csv_field(&normalized_label(&mark.label)),
            in_time,
            out_time,
            duration
        ));
    }

    Ok(lines.join("\n"))
}

/// 转义 CSV 字段
fn escape_csv_field(field: &str) -> String {
    if field
        .chars()
        .any(|character| matches!(character, ',' | '"' | '\n' | '\r'))
    {
        let escaped = field.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        field.to_string()
    }
}

/// 生成 video.xml (FCP7 XML / xmeml 格式，备选导入方式)。
///
/// M1 使用 30fps 作为时间线帧率；后续可从视频元数据传入真实帧率。
pub fn generate_fcp7_xml(
    marks: &[Mark],
    video_path: &Path,
    video_duration: f64,
) -> Result<String, Box<dyn std::error::Error>> {
    validate_marks(marks, video_duration)?;

    const FPS: f64 = 30.0;
    let total_frames = seconds_to_frames(video_duration, FPS);
    let video_name = escape_xml(
        video_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("video.mp4"),
    );
    let path_url = escape_xml(&path_to_file_url(video_path));
    let rate = "<rate><timebase>30</timebase><ntsc>FALSE</ntsc></rate>";

    let mut markers = String::new();
    for mark in sorted_marks(marks) {
        let start = seconds_to_frames(mark.start_time, FPS);
        let end = mark
            .end_time
            .map(|end| seconds_to_frames(end, FPS))
            .unwrap_or(start + 1)
            .max(start + 1);
        markers.push_str(&format!(
            "            <marker><name>{}</name><comment></comment><in>{start}</in><out>{end}</out></marker>\n",
            escape_xml(&normalized_label(&mark.label))
        ));
    }

    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE xmeml>
<xmeml version="4">
  <sequence id="sequence-1">
    <name>TimePin 标记</name>
    <duration>{total_frames}</duration>
    {rate}
    <media>
      <video>
        <track>
          <clipitem id="clipitem-1">
            <name>{video_name}</name>
            <duration>{total_frames}</duration>
            {rate}
            <file id="file-1">
              <name>{video_name}</name>
              <pathurl>{path_url}</pathurl>
              {rate}
              <duration>{total_frames}</duration>
              <media><video><samplecharacteristics>{rate}</samplecharacteristics></video></media>
            </file>
            <start>0</start>
            <end>{total_frames}</end>
            <in>0</in>
            <out>{total_frames}</out>
{markers}          </clipitem>
        </track>
      </video>
    </media>
  </sequence>
</xmeml>
"#
    );

    Ok(xml)
}

fn seconds_to_frames(seconds: f64, fps: f64) -> i64 {
    if !seconds.is_finite() || seconds <= 0.0 || !fps.is_finite() || fps <= 0.0 {
        return 0;
    }
    (seconds * fps).round() as i64
}

fn path_to_file_url(path: &Path) -> String {
    let mut path = path.to_string_lossy().replace('\\', "/");
    if !path.starts_with('/') {
        path.insert(0, '/');
    }

    let mut encoded = String::with_capacity(path.len());
    for character in path.chars() {
        match character {
            ' ' => encoded.push_str("%20"),
            '"' => encoded.push_str("%22"),
            '#' => encoded.push_str("%23"),
            '?' => encoded.push_str("%3F"),
            '%' => encoded.push_str("%25"),
            _ => encoded.push(character),
        }
    }

    format!("file://localhost{encoded}")
}

/// 转义 XML 特殊字符
fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// 保存伴随文件到视频同目录，文件名与视频同名。
///
/// video.txt 使用 UTF-8 无 BOM 写入。
pub fn save_sidecar_files(
    _app_handle: &tauri::AppHandle,
    _video_id: &str,
    marks: &[Mark],
    video_duration: f64,
    video_path: &Path,
) -> Result<SidecarPaths, Box<dyn std::error::Error>> {
    let txt_content = generate_txt_chapters(marks, video_duration)?;
    let csv_content = generate_csv_markers(marks, video_duration)?;
    let xml_content = generate_fcp7_xml(marks, video_path, video_duration)?;

    let txt_path = video_path.with_extension("txt");
    let csv_path = video_path.with_extension("csv");
    let xml_path = video_path.with_extension("xml");

    std::fs::write(&txt_path, txt_content.as_bytes())?;
    std::fs::write(&csv_path, csv_content.as_bytes())?;
    std::fs::write(&xml_path, xml_content.as_bytes())?;

    Ok(SidecarPaths {
        txt: txt_path,
        csv: csv_path,
        xml: xml_path,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarPaths {
    pub txt: PathBuf,
    pub csv: PathBuf,
    pub xml: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_ogm_time_with_three_digit_millis() {
        assert_eq!(format_time_ogm(0.0), "00:00:00,000");
        assert_eq!(format_time_ogm(65.5), "00:01:05,500");
        assert_eq!(format_time_ogm(3661.234), "01:01:01,234");
    }

    #[test]
    fn rejects_invalid_times() {
        let invalid = Mark::new_point(-1.0, "负时间".to_string());
        assert!(generate_txt_chapters(&[invalid], 10.0).is_err());

        let reversed = Mark::new_range(5.0, 4.0, "反向区间".to_string());
        assert!(generate_csv_markers(&[reversed], 10.0).is_err());
    }

    #[test]
    fn validates_ogm_lines_not_just_time() {
        assert!(validate_ogm_line("00:00:01,000 开场"));
        assert!(!validate_ogm_line("00:00:01,000 "));
        assert!(!validate_ogm_line("00:00:01,000 开场\n第二行"));
        assert!(!validate_ogm_line("00:00:01.000 开场"));
    }

    #[test]
    fn escapes_csv_fields() {
        let mark = Mark::new_point(1.0, "Boss, \"第一阶段\"".to_string());
        let csv = generate_csv_markers(&[mark], 10.0).expect("csv");
        assert!(csv.contains("\"Boss, \"\"第一阶段\"\"\""));
    }
}
