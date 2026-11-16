use std::collections::HashMap;
use std::path::PathBuf;

use timepin_lib::formats::{
    generate_csv_markers, generate_fcp7_xml, generate_txt_chapters, validate_ogm_line, Mark,
};

fn mark(id: &str, start: f64, end: Option<f64>, label: &str) -> Mark {
    Mark {
        id: id.to_string(),
        start_time: start,
        end_time: end,
        label: label.to_string(),
        color: None,
        confidence: None,
        source_plugin: None,
        thumbnail_path: None,
        metadata: HashMap::new(),
    }
}

fn sample_marks() -> Vec<Mark> {
    vec![
        mark("m1", 0.0, None, "开场"),
        mark("m2", 5.5, Some(12.25), "Boss 战"),
        mark("m3", 30.0, None, "拾取道具"),
    ]
}

/// Golden files 在 Windows 上可能被 git 转为 CRLF；生成器固定输出 LF，
/// 所以比较前统一归一化。
fn normalized(content: &str) -> String {
    content.replace("\r\n", "\n").trim_end().to_string()
}

#[test]
fn txt_matches_golden_file() {
    let actual = generate_txt_chapters(&sample_marks(), 60.0).expect("txt");
    let expected = normalized(include_str!("fixtures/sample_video.txt"));
    assert_eq!(actual, expected);
}

#[test]
fn csv_matches_golden_file() {
    let actual = generate_csv_markers(&sample_marks(), 60.0).expect("csv");
    let expected = normalized(include_str!("fixtures/sample_video.csv"));
    assert_eq!(actual, expected);
}

#[test]
fn txt_is_utf8_without_bom_and_starts_at_zero() {
    let actual = generate_txt_chapters(&sample_marks(), 60.0).expect("txt");
    assert!(!actual.as_bytes().starts_with(&[0xEF, 0xBB, 0xBF]));
    assert!(actual.starts_with("00:00:00,000 "));
    assert!(actual.lines().all(validate_ogm_line));
}

#[test]
fn txt_injects_zero_chapter_when_needed() {
    let marks = vec![mark("m1", 3.0, None, "第一个标记")];
    let actual = generate_txt_chapters(&marks, 10.0).expect("txt");
    assert_eq!(actual.lines().next(), Some("00:00:00,000 视频开始"));
    assert!(actual.contains("00:00:03,000 第一个标记"));
}

#[test]
fn csv_has_fixed_header() {
    let actual = generate_csv_markers(&sample_marks(), 60.0).expect("csv");
    assert_eq!(actual.lines().next(), Some("Name,In,Out,Duration"));
}

#[test]
fn xml_uses_fcp7_xmeml_and_escapes_labels() {
    let marks = vec![mark("m1", 1.0, Some(2.0), "Boss & \"阶段\"")];
    let actual =
        generate_fcp7_xml(&marks, &PathBuf::from(r"C:\Videos\测试 & 片段.mp4"), 10.0).expect("xml");

    assert!(actual.contains("<xmeml version=\"4\">"));
    assert!(actual.contains("Boss &amp; &quot;阶段&quot;"));
    assert!(!actual.contains("<fcpxml"));
}
