//! FrameService 集成测试（TPIN-3）
//!
//! 使用 `TIMEPIN_TEST_FFMPEG` 环境变量定位 FFmpeg；未设置时跳过真实提取测试。
//! 单元测试（路径授权、参数校验、协议兼容）始终运行，不依赖外部工具。

use std::env;
use std::fs;
use std::path::PathBuf;

use base64::{engine::general_purpose, Engine as _};
use tempfile::TempDir;
use timepin_lib::frame_service::{
    DeliveryMode, ExtractMode, ExtractOptions, FrameExtractRequest, FrameService,
    FrameServiceError, OutputFormat,
};

// ==================== 测试辅助 ====================

/// 构造一个最小可运行的 FrameService：video_path 和 output_dir 用临时目录中的虚拟文件。
/// 返回 (service, tmp_dir_handle)，保证临时目录生命周期覆盖测试。
fn make_service() -> (FrameService, TempDir) {
    let tmp = TempDir::new().expect("创建临时目录");
    let video_path = tmp.path().join("video.mp4");
    fs::write(&video_path, b"fake mp4").expect("写虚拟视频");
    let output_dir = tmp.path().join("out");
    fs::create_dir_all(&output_dir).expect("创建输出目录");
    let ffmpeg = env::var("TIMEPIN_TEST_FFMPEG")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("ffmpeg"));
    let service = FrameService::new(ffmpeg, video_path, output_dir)
        .expect("构造 FrameService 失败");
    (service, tmp)
}

/// 构造合法请求骨架
fn make_request(video_path: PathBuf, scratch_dir: PathBuf) -> FrameExtractRequest {
    FrameExtractRequest {
        protocol_version: "1.0.0".into(),
        video_path,
        options: ExtractOptions {
            mode: ExtractMode::Image2,
            time_range: Some((0.0, 1.0)),
            sample_fps: Some(1.0),
            output_format: OutputFormat::Jpeg,
            scratch_dir: Some(scratch_dir),
            delivery: DeliveryMode::Files,
        },
    }
}

// ==================== 路径授权测试 ====================

#[test]
fn rejects_video_path_mismatch() {
    let (service, tmp) = make_service();
    let other_video = tmp.path().join("other.mp4");
    fs::write(&other_video, b"fake").unwrap();
    let scratch = tmp.path().join("out");
    let req = make_request(other_video, scratch);
    assert!(matches!(
        service.extract(req),
        Err(FrameServiceError::UnauthorizedVideoPath)
    ));
}

#[test]
fn rejects_scratch_dir_escape() {
    let (service, tmp) = make_service();
    let video_path = tmp.path().join("video.mp4");
    let escape_dir = tmp.path().join("outside");
    fs::create_dir_all(&escape_dir).unwrap();
    let req = make_request(video_path, escape_dir);
    assert!(matches!(
        service.extract(req),
        Err(FrameServiceError::UnauthorizedScratchDir)
    ));
}

#[test]
fn accepts_scratch_dir_under_output_dir() {
    let (service, tmp) = make_service();
    let video_path = tmp.path().join("video.mp4");
    let scratch = tmp.path().join("out/subdir");
    fs::create_dir_all(&scratch).unwrap();
    let req = make_request(video_path, scratch);
    // 不会到达 FFmpeg（ffmpeg 路径是假的），但授权检查应通过
    let err = service.extract(req).unwrap_err();
    assert!(!matches!(err, FrameServiceError::UnauthorizedScratchDir));
}

// ==================== 参数校验测试 ====================

#[test]
fn rejects_non_finite_time_range() {
    let (service, tmp) = make_service();
    let video_path = tmp.path().join("video.mp4");
    let scratch = tmp.path().join("out");
    let mut req = make_request(video_path, scratch);
    req.options.time_range = Some((f64::NAN, 5.0));
    assert!(matches!(
        service.extract(req),
        Err(FrameServiceError::InvalidRequest(_))
    ));
}

#[test]
fn rejects_negative_time_range() {
    let (service, tmp) = make_service();
    let video_path = tmp.path().join("video.mp4");
    let scratch = tmp.path().join("out");
    let mut req = make_request(video_path, scratch);
    req.options.time_range = Some((-1.0, 5.0));
    assert!(matches!(
        service.extract(req),
        Err(FrameServiceError::InvalidRequest(_))
    ));
}

#[test]
fn rejects_reversed_time_range() {
    let (service, tmp) = make_service();
    let video_path = tmp.path().join("video.mp4");
    let scratch = tmp.path().join("out");
    let mut req = make_request(video_path, scratch);
    req.options.time_range = Some((5.0, 3.0));
    assert!(matches!(
        service.extract(req),
        Err(FrameServiceError::InvalidRequest(_))
    ));
}

#[test]
fn rejects_zero_sample_fps() {
    let (service, tmp) = make_service();
    let video_path = tmp.path().join("video.mp4");
    let scratch = tmp.path().join("out");
    let mut req = make_request(video_path, scratch);
    req.options.sample_fps = Some(0.0);
    assert!(matches!(
        service.extract(req),
        Err(FrameServiceError::InvalidRequest(_))
    ));
}

#[test]
fn rejects_infinite_sample_fps() {
    let (service, tmp) = make_service();
    let video_path = tmp.path().join("video.mp4");
    let scratch = tmp.path().join("out");
    let mut req = make_request(video_path, scratch);
    req.options.sample_fps = Some(f64::INFINITY);
    assert!(matches!(
        service.extract(req),
        Err(FrameServiceError::InvalidRequest(_))
    ));
}

// ==================== 协议版本测试 ====================

#[test]
fn rejects_incompatible_protocol_version() {
    let (service, tmp) = make_service();
    let video_path = tmp.path().join("video.mp4");
    let scratch = tmp.path().join("out");
    let mut req = make_request(video_path, scratch);
    req.protocol_version = "2.0.0".into();
    assert!(matches!(
        service.extract(req),
        Err(FrameServiceError::InvalidRequest(_))
    ));
}

// ==================== MP4 索引测试 ====================

#[test]
fn mp4_index_parses_minimal_file() {
    // 构造一个最小 MP4：ftyp + moov（仅含一个 vide trak）
    // 这是 hand-crafted 数据，验证 read_index 不崩溃且能识别 video track
    let tmp = TempDir::new().unwrap();
    let mp4_path = tmp.path().join("minimal.mp4");
    // ftyp box: size=24, type=ftyp, major=isom, minor=0, brands=[isom, mp41]
    let ftyp: &[u8] = &[
        0, 0, 0, 24, b'f', b't', b'y', b'p',
        b'i', b's', b'o', b'm',
        0, 0, 0, 0,
        b'i', b's', b'o', b'm', b'm', b'p', b'4', b'1',
    ];
    // moov box: 空 trak（无有效数据，read_index 应返回错误而非 panic）
    let trak_inner: &[u8] = &[];
    let mut moov = Vec::new();
    moov.extend_from_slice(&[0, 0, 0, 0, b't', b'r', b'a', b'k']); // trak header placeholder
    moov.extend_from_slice(trak_inner);
    let moov_size = (moov.len() + 8) as u32;
    let mut moov_box = Vec::new();
    moov_box.extend_from_slice(&moov_size.to_be_bytes());
    moov_box.extend_from_slice(b"moov");
    moov_box.extend_from_slice(&moov);
    fs::write(&mp4_path, [ftyp, &moov_box].concat()).unwrap();
    let result = timepin_lib::frame_service::mp4::read_index(&mp4_path);
    // 空 trak 应返回错误（找不到视频 track），不 panic
    assert!(result.is_err());
}

// ==================== 真实 FFmpeg 集成测试 ====================

#[test]
#[ignore = "需要真实 FFmpeg 与测试视频；设置 TIMEPIN_TEST_FFMPEG 后运行"]
fn real_ffmpeg_extracts_image2_frames() {
    let ffmpeg = env::var("TIMEPIN_TEST_FFMPEG").expect("未设置 TIMEPIN_TEST_FFMPEG");
    let tmp = TempDir::new().unwrap();
    // 用 FFmpeg 生成 5 秒纯色测试视频（避免依赖外部样本文件）
    let video_path = tmp.path().join("test.mp4");
    let status = std::process::Command::new(&ffmpeg)
        .args([
            "-f", "lavfi", "-i", "testsrc=duration=5:size=320x240:rate=30",
            "-c:v", "libx264", "-pix_fmt", "yuv420p",
            video_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "生成测试视频失败");

    let output_dir = tmp.path().join("out");
    fs::create_dir_all(&output_dir).unwrap();
    let service = FrameService::new(PathBuf::from(&ffmpeg), video_path.clone(), output_dir.clone())
        .unwrap();
    let req = FrameExtractRequest {
        protocol_version: "1.0.0".into(),
        video_path,
        options: ExtractOptions {
            mode: ExtractMode::Image2,
            time_range: Some((1.0, 3.0)),
            sample_fps: Some(2.0),
            output_format: OutputFormat::Jpeg,
            scratch_dir: Some(output_dir),
            delivery: DeliveryMode::Files,
        },
    };
    let result = service.extract(req).unwrap();
    assert_eq!(result.protocol_version, "1.0.0");
    assert_eq!(result.mode, ExtractMode::Image2);
    assert!(!result.frames.is_empty());
    assert!(result.total_frames <= 4); // 2 fps * 2s = 4 帧上限
    for frame in &result.frames {
        assert!(frame.pts_sec >= 1.0 && frame.pts_sec < 3.0);
        assert!(frame.path.as_ref().unwrap().is_file());
        assert!(frame.image_base64.is_none());
    }
}

#[test]
#[ignore = "需要真实 FFmpeg；设置 TIMEPIN_TEST_FFMPEG 后运行"]
fn real_ffmpeg_extracts_keyframes() {
    let ffmpeg = env::var("TIMEPIN_TEST_FFMPEG").expect("未设置 TIMEPIN_TEST_FFMPEG");
    let tmp = TempDir::new().unwrap();
    let video_path = tmp.path().join("test.mp4");
    // 生成带关键帧的视频（每 1 秒一个关键帧）
    let status = std::process::Command::new(&ffmpeg)
        .args([
            "-f", "lavfi", "-i", "testsrc=duration=5:size=320x240:rate=30",
            "-c:v", "libx264", "-g", "30", "-pix_fmt", "yuv420p",
            video_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success(), "生成测试视频失败");

    let output_dir = tmp.path().join("out");
    fs::create_dir_all(&output_dir).unwrap();
    let service = FrameService::new(PathBuf::from(&ffmpeg), video_path.clone(), output_dir.clone())
        .unwrap();
    let req = FrameExtractRequest {
        protocol_version: "1.0.0".into(),
        video_path,
        options: ExtractOptions {
            mode: ExtractMode::Keyframes,
            time_range: Some((0.0, 5.0)),
            sample_fps: None,
            output_format: OutputFormat::Jpeg,
            scratch_dir: Some(output_dir),
            delivery: DeliveryMode::Files,
        },
    };
    let result = service.extract(req).unwrap();
    assert_eq!(result.mode, ExtractMode::Keyframes);
    // 关键帧数量应与 -g 30 和 30fps 一致（约 5 个）
    assert!(result.frames.len() >= 3 && result.frames.len() <= 10);
    for frame in &result.frames {
        assert!(frame.pts_sec >= 0.0 && frame.pts_sec < 5.0);
        assert!(frame.path.as_ref().unwrap().is_file());
    }
}

#[test]
#[ignore = "需要真实 FFmpeg；设置 TIMEPIN_TEST_FFMPEG 后运行"]
fn real_ffmpeg_base64_delivery_bounded() {
    let ffmpeg = env::var("TIMEPIN_TEST_FFMPEG").expect("未设置 TIMEPIN_TEST_FFMPEG");
    let tmp = TempDir::new().unwrap();
    let video_path = tmp.path().join("test.mp4");
    let status = std::process::Command::new(&ffmpeg)
        .args([
            "-f", "lavfi", "-i", "testsrc=duration=2:size=160x120:rate=10",
            "-c:v", "libx264", "-pix_fmt", "yuv420p",
            video_path.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let output_dir = tmp.path().join("out");
    fs::create_dir_all(&output_dir).unwrap();
    let service = FrameService::new(PathBuf::from(&ffmpeg), video_path.clone(), output_dir.clone())
        .unwrap();
    let req = FrameExtractRequest {
        protocol_version: "1.0.0".into(),
        video_path,
        options: ExtractOptions {
            mode: ExtractMode::Image2,
            time_range: Some((0.0, 1.0)),
            sample_fps: Some(5.0),
            output_format: OutputFormat::Png,
            scratch_dir: Some(output_dir),
            delivery: DeliveryMode::Base64,
        },
    };
    let result = service.extract(req).unwrap();
    assert!(!result.frames.is_empty());
    for frame in &result.frames {
        assert!(frame.path.is_none());
        let b64 = frame.image_base64.as_ref().unwrap();
        assert!(b64.len() <= 8 * 1024 * 1024); // 单帧上限
        // 可以解码为图像（PNG/JPEG magic bytes）
        let bytes = general_purpose::STANDARD.decode(b64).unwrap();
        assert!(bytes.starts_with(b"\x89PNG") || bytes.starts_with(b"\xFF\xD8"));
    }
}
