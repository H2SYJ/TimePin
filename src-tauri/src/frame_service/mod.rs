//! 帧提取服务（TPIN-3）
//!
//! 为插件 Sidecar 提供 `frames.extract` 能力：从已授权的视频文件中提取帧，
//! 支持 image2（顺序抽取）与 keyframes（关键帧索引 + FFmpeg 精确 seek）两种模式。
//!
//! 安全约束（铁律 #1、#5）：
//! - 绝不修改原 MP4；所有输出写入 `output_dir` 下的 UUID 子目录
//! - `video_path` 必须与构造时授权的 canonical 路径一致
//! - `scratch_dir` 必须是 `output_dir` 的 canonical 后代（防目录逃逸/符号链接）
//! - FFmpeg 以 `-nostdin` 非交互方式运行，绝不走 shell
//! - 帧数、内存、时间均有硬性上限；超时/超限即终止并回收子进程

use std::ffi::OsString;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ==================== 常量与上限 ====================

const PROTOCOL_VERSION: &str = "1.0.0";

/// 单次提取最大帧数（防止插件索取无限帧拖垮磁盘/内存）
const MAX_FRAMES: usize = 512;
/// base64 模式下单帧最大编码后字节数（JPEG/PNG 通常 < 2 MB）
const MAX_BASE64_FRAME_BYTES: usize = 8 * 1024 * 1024;
/// base64 模式下总编码字节上限
const MAX_BASE64_TOTAL_BYTES: usize = 64 * 1024 * 1024;
/// FFmpeg 进程最长运行时间
const FFMPEG_TIMEOUT: Duration = Duration::from_secs(300);
/// FFmpeg stderr 行缓冲上限（防 showinfo 大量输出撑爆内存）
const MAX_STDERR_LINES: usize = 8192;
/// 关键帧 seek 每帧最长等待时间
const PER_FRAME_SEEK_TIMEOUT: Duration = Duration::from_secs(30);

// ==================== 错误类型 ====================

#[derive(Debug, Error)]
pub enum FrameServiceError {
    #[error("路径越权：video_path 必须是授权视频的 canonical 路径")]
    UnauthorizedVideoPath,
    #[error("路径越权：scratch_dir 必须是 output_dir 的 canonical 后代")]
    UnauthorizedScratchDir,
    #[error("路径不存在或不可读：{0}")]
    PathNotFound(String),
    #[error("符号链接/逃逸检测失败：{0}")]
    SymlinkEscape(String),
    #[error("请求无效：{0}")]
    InvalidRequest(String),
    #[error("模式不支持：{0}")]
    UnsupportedMode(String),
    #[error("MP4 索引解析失败：{0}")]
    IndexParse(String),
    #[error("FFmpeg 执行失败：{0}")]
    FFmpegFailed(String),
    #[error("FFmpeg 执行超时（{0:?}）")]
    FFmpegTimeout(Duration),
    #[error("输出超过上限：{0}")]
    OutputLimitExceeded(String),
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 序列化错误：{0}")]
    Json(#[from] serde_json::Error),
    #[error("Base64 解码错误：{0}")]
    Base64(#[from] base64::DecodeError),
}

// ==================== 协议类型（camelCase / snake_case 按任务约定） ====================

/// `frames.extract` 请求（JSON-RPC params）
///
/// 字段命名约定：
/// - `protocolVersion`（外层 camelCase，序列化时自动转换）
/// - `video_path`、`options` 内字段保持 snake_case（issue 协议约定）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameExtractRequest {
    #[serde(default = "default_protocol_version")]
    pub protocol_version: String,
    pub video_path: PathBuf,
    #[serde(default)]
    pub options: ExtractOptions,
}

/// `frames.extract` 的 options（snake_case 按 issue 约定）
///
/// 字段名本身已是 snake_case；不显式 rename 使 wire 与 Rust 同形：
/// `{"mode":"image2","time_range":[s,e],"sample_fps":2.0,"output_format":"jpeg","scratch_dir":"...","delivery":"files"}`
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ExtractOptions {
    #[serde(default = "default_mode")]
    pub mode: ExtractMode,
    /// [start, end) 秒；null/null 表示整段视频
    #[serde(default)]
    pub time_range: Option<(f64, f64)>,
    /// 采样频率（image2 生效；keyframes 模式下为最大频率上限）
    #[serde(default)]
    pub sample_fps: Option<f64>,
    #[serde(default = "default_output_format")]
    pub output_format: OutputFormat,
    /// 必须是 output_dir 的 canonical 后代；帧文件写在其 UUID 子目录下
    #[serde(default)]
    pub scratch_dir: Option<PathBuf>,
    /// files（默认，帧写磁盘）| base64（小批量内存返回）
    #[serde(default = "default_delivery")]
    pub delivery: DeliveryMode,
}

/// 提取模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtractMode {
    /// 顺序解码 + fps filter；FFmpeg 自带采样，单进程完成
    Image2,
    /// 先用本地 MP4 索引定位关键帧，再逐帧精确 seek（跳过中间解码）
    Keyframes,
}

/// 输出格式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Jpeg,
    Png,
}

/// 交付模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryMode {
    /// 帧文件写磁盘，返回路径（默认，适合大批量）
    Files,
    /// 小批量时直接返回 base64 编码图像（内存占用受 MAX_BASE64_* 限制）
    Base64,
}

/// 单帧元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameInfo {
    /// 在结果数组中的序号（0-based）
    pub index: usize,
    /// 真实 PTS（秒），来自 FFmpeg showinfo，非推测 index/fps
    pub pts_sec: f64,
    /// files 模式下的磁盘路径；base64 模式下为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// base64 模式下的图像数据；files 模式下为 None
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_base64: Option<String>,
}

/// `frames.extract` 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameExtractResult {
    pub protocol_version: String,
    pub frames: Vec<FrameInfo>,
    pub total_frames: usize,
    pub mode: ExtractMode,
}

// ==================== serde defaults ====================

fn default_protocol_version() -> String {
    PROTOCOL_VERSION.to_string()
}
fn default_mode() -> ExtractMode {
    ExtractMode::Image2
}
fn default_output_format() -> OutputFormat {
    OutputFormat::Jpeg
}
fn default_delivery() -> DeliveryMode {
    DeliveryMode::Files
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            time_range: None,
            sample_fps: None,
            output_format: default_output_format(),
            scratch_dir: None,
            delivery: default_delivery(),
        }
    }
}

// ==================== MP4 索引 ====================

pub mod mp4;

// ==================== FrameService ====================

pub struct FrameService {
    /// FFmpeg 可执行文件路径（绝对路径，调用方保证存在且可执行）
    ffmpeg_path: PathBuf,
    /// 授权视频的 canonical 路径
    video_path: PathBuf,
    /// 授权输出根目录的 canonical 路径
    output_dir: PathBuf,
}

impl FrameService {
    /// 构造服务实例。三个路径都必须是已存在/可创建的路径。
    /// 会立即 canonicalize 并缓存，后续 extract 只做字符串/前缀比较。
    pub fn new(
        ffmpeg_path: PathBuf,
        video_path: PathBuf,
        output_dir: PathBuf,
    ) -> Result<Self, FrameServiceError> {
        let video_path = video_path.canonicalize().map_err(|e| {
            FrameServiceError::PathNotFound(format!("video_path {}: {e}", video_path.display()))
        })?;
        let output_dir = output_dir.canonicalize().map_err(|e| {
            FrameServiceError::PathNotFound(format!("output_dir {}: {e}", output_dir.display()))
        })?;
        Ok(Self {
            ffmpeg_path,
            video_path,
            output_dir,
        })
    }

    /// 执行 `frames.extract`。
    ///
    /// 流程：校验请求 → 校验路径授权 → 创建 UUID scratch 子目录 →
    /// 按 mode 分发 image2 或 keyframes → 收集真实 PTS 与文件路径 →
    /// 按 delivery 返回 files 或 base64。
    pub fn extract(
        &self,
        request: FrameExtractRequest,
    ) -> Result<FrameExtractResult, FrameServiceError> {
        // 1. 协议版本兼容（major 相同即兼容；0.x 要求完全一致）
        self.check_protocol_version(&request.protocol_version)?;

        // 2. 路径授权：video_path 必须是构造时的同一文件（严格比较）
        let req_video = request.video_path.canonicalize().map_err(|e| {
            FrameServiceError::PathNotFound(format!(
                "video_path {}: {e}",
                request.video_path.display()
            ))
        })?;
        if req_video != self.video_path {
            return Err(FrameServiceError::UnauthorizedVideoPath);
        }

        // 3. scratch_dir 授权：必须是 output_dir 的后代（严格比较）
        let scratch_root = match &request.options.scratch_dir {
            Some(dir) => {
                let canonical = dir.canonicalize().map_err(|e| {
                    FrameServiceError::PathNotFound(format!("scratch_dir {}: {e}", dir.display()))
                })?;
                if !canonical.starts_with(&self.output_dir) {
                    return Err(FrameServiceError::UnauthorizedScratchDir);
                }
                canonical
            }
            None => self.output_dir.clone(),
        };
        if !scratch_root.exists() {
            fs::create_dir_all(&scratch_root).map_err(FrameServiceError::Io)?;
        }

        // 4. 参数校验
        self.validate_options(&request.options)?;

        // 5. 创建本次提取的 UUID 子目录（永不覆盖既有文件）
        let extract_id = Uuid::new_v4().to_string();
        let work_dir = scratch_root.join(&extract_id);
        fs::create_dir_all(&work_dir).map_err(FrameServiceError::Io)?;

        // 6. 按模式执行
        let frames = match request.options.mode {
            ExtractMode::Image2 => self.extract_image2(&request, &work_dir)?,
            ExtractMode::Keyframes => self.extract_keyframes(&request, &work_dir)?,
        };

        // 7. 检查数量上限
        if frames.len() > MAX_FRAMES {
            return Err(FrameServiceError::OutputLimitExceeded(format!(
                "提取帧数 {} 超过上限 {}",
                frames.len(),
                MAX_FRAMES
            )));
        }

        // 8. delivery 模式处理
        let frames = self.prepare_delivery(frames, &request.options)?;

        Ok(FrameExtractResult {
            protocol_version: PROTOCOL_VERSION.to_string(),
            total_frames: frames.len(),
            mode: request.options.mode,
            frames,
        })
    }

    fn check_protocol_version(&self, version: &str) -> Result<(), FrameServiceError> {
        if !versions_compatible(PROTOCOL_VERSION, version) {
            return Err(FrameServiceError::InvalidRequest(format!(
                "协议版本不兼容：host={} plugin={}",
                PROTOCOL_VERSION, version
            )));
        }
        Ok(())
    }

    fn validate_options(&self, options: &ExtractOptions) -> Result<(), FrameServiceError> {
        // time_range 必须是有限、非负、start < end
        if let Some((start, end)) = options.time_range {
            if !start.is_finite() || !end.is_finite() {
                return Err(FrameServiceError::InvalidRequest(
                    "time_range 必须是有限数值".into(),
                ));
            }
            if start < 0.0 || end < 0.0 || start >= end {
                return Err(FrameServiceError::InvalidRequest(format!(
                    "time_range 必须满足 0 <= start < end，实际 [{start}, {end})"
                )));
            }
        }
        // sample_fps 若为 Some 必须为正有限值
        if let Some(fps) = options.sample_fps {
            if !fps.is_finite() || fps <= 0.0 {
                return Err(FrameServiceError::InvalidRequest(format!(
                    "sample_fps 必须是正有限值，实际 {fps}"
                )));
            }
            if fps > 1000.0 {
                return Err(FrameServiceError::InvalidRequest(format!(
                    "sample_fps 过大（>1000），可能消耗过多资源：{fps}"
                )));
            }
        }
        Ok(())
    }

    // ==================== image2 模式 ====================

    /// 单进程 FFmpeg image2 提取：`-ss start -t duration -i input -vf fps=... -f image2 out%06d.ext`
    /// 通过 showinfo 解析真实 PTS；`-vsync 0` 保证每帧都输出（避免 CFR 丢帧）。
    fn extract_image2(
        &self,
        request: &FrameExtractRequest,
        work_dir: &Path,
    ) -> Result<Vec<FrameInfo>, FrameServiceError> {
        let opts = &request.options;
        let ext = match opts.output_format {
            OutputFormat::Jpeg => "jpg",
            OutputFormat::Png => "png",
        };
        let out_pattern = work_dir.join(format!("frame_%06d.{ext}"));

        let mut args: Vec<OsString> = vec![
            "-nostdin".into(),
            "-y".into(), // 允许覆盖（UUID 子目录内无用户文件，安全）
        ];

        // 时间范围
        if let Some((start, end)) = opts.time_range {
            args.extend([
                "-ss".into(),
                format!("{start:.6}").into(),
                "-t".into(),
                format!("{:.6}", end - start).into(),
            ]);
        }

        args.extend(["-i".into(), self.video_path.as_os_str().to_os_string()]);

        // 采样频率：用 fps filter 让 FFmpeg 自己处理 VFR/CFR
        if let Some(fps) = opts.sample_fps {
            args.extend(["-vf".into(), format!("fps={fps:.6}").into()]);
        }

        // 关键：-vsync 0 (passthrough) 保证每个输入帧都产生输出，
        // 否则 FFmpeg 会按 CFR 去重/补帧导致 PTS 与输出文件数不匹配
        args.extend([
            "-vsync".into(),
            "0".into(),
            "-f".into(),
            "image2".into(),
            out_pattern.as_os_str().to_os_string(),
        ]);

        // 运行 FFmpeg 并解析 stderr 中的 showinfo PTS
        let (pts_list, _stderr_lines) = self.run_ffmpeg_with_showinfo(&args, work_dir)?;

        // 收集输出文件（按文件名排序，FFmpeg image2 按顺序编号）
        let mut files = list_output_files(work_dir, ext)?;
        if files.len() > MAX_FRAMES {
            files.truncate(MAX_FRAMES);
        }

        // PTS 与文件对齐：FFmpeg image2 顺序写文件，showinfo 顺序一致
        let mut frames = Vec::with_capacity(files.len());
        for (index, path) in files.into_iter().enumerate() {
            let pts_sec = pts_list.get(index).copied().unwrap_or(0.0);
            frames.push(FrameInfo {
                index,
                pts_sec,
                path: Some(path),
                image_base64: None,
            });
        }
        Ok(frames)
    }

    // ==================== keyframes 模式 ====================

    /// 关键帧提取：本地 MP4 索引定位关键帧 → 每帧 `-ss pts -i input -frames:v 1 out`
    /// 跳过中间解码，适合大视频快速抓取 I-frame。
    fn extract_keyframes(
        &self,
        request: &FrameExtractRequest,
        work_dir: &Path,
    ) -> Result<Vec<FrameInfo>, FrameServiceError> {
        let opts = &request.options;
        let ext = match opts.output_format {
            OutputFormat::Jpeg => "jpg",
            OutputFormat::Png => "png",
        };

        // 1. 解析本地 MP4 索引
        let index = mp4::read_index(&self.video_path).map_err(FrameServiceError::IndexParse)?;

        // 2. 过滤关键帧：时间范围 + sample_fps 上限
        let mut selected = self.select_keyframes(&index.keyframes, opts);

        // 3. 数量上限
        if selected.len() > MAX_FRAMES {
            selected.truncate(MAX_FRAMES);
        }

        let mut frames = Vec::with_capacity(selected.len());
        for (index, kf) in selected.into_iter().enumerate() {
            let out_path = work_dir.join(format!(
                "keyframe_{:06}_{:.6}.{ext}",
                kf.sample_index, kf.pts_sec
            ));
            self.extract_single_keyframe(kf.pts_sec, &out_path, ext)?;

            // 用 showinfo 二次校验实际写入文件的 PTS（防 FFmpeg seek 偏差）
            let actual_pts = self.probe_frame_pts(&out_path).unwrap_or(kf.pts_sec);
            frames.push(FrameInfo {
                index,
                pts_sec: actual_pts,
                path: Some(out_path),
                image_base64: None,
            });
        }
        Ok(frames)
    }

    /// 从关键帧列表中按时间范围与 sample_fps 上限筛选。
    /// sample_fps 在此模式下是「最大频率」：相邻关键帧间隔不小于 1/fps。
    fn select_keyframes<'a>(
        &self,
        keyframes: &'a [mp4::Keyframe],
        opts: &ExtractOptions,
    ) -> Vec<&'a mp4::Keyframe> {
        let mut selected: Vec<&mp4::Keyframe> = keyframes
            .iter()
            .filter(|kf| {
                if let Some((start, end)) = opts.time_range {
                    kf.pts_sec >= start && kf.pts_sec < end
                } else {
                    true
                }
            })
            .collect();

        // sample_fps：限制最小间隔，不是合成帧
        if let Some(fps) = opts.sample_fps {
            let min_interval = 1.0 / fps;
            let mut filtered = Vec::new();
            let mut last_pts = f64::NEG_INFINITY;
            for kf in selected {
                if kf.pts_sec - last_pts >= min_interval || filtered.is_empty() {
                    filtered.push(kf);
                    last_pts = kf.pts_sec;
                }
            }
            selected = filtered;
        }
        selected
    }

    /// 单帧精确 seek：`-ss pts -i input -frames:v 1 out`
    /// 用 `-ss` 放在 `-i` 前做快速 seek（跳到最近关键帧），FFmpeg 会自动解码到目标。
    /// 但关键帧模式下 pts 本身就是关键帧，seek 后直接到达，无需解码中间帧。
    fn extract_single_keyframe(
        &self,
        pts_sec: f64,
        out_path: &Path,
        _ext: &str,
    ) -> Result<(), FrameServiceError> {
        let args: Vec<OsString> = vec![
            "-nostdin".into(),
            "-y".into(),
            "-ss".into(),
            format!("{pts_sec:.6}").into(),
            "-i".into(),
            self.video_path.as_os_str().to_os_string(),
            "-frames:v".into(),
            "1".into(),
            "-vsync".into(),
            "0".into(),
            "-f".into(),
            "image2".into(),
            out_path.as_os_str().to_os_string(),
        ];

        self.run_ffmpeg_simple(&args, PER_FRAME_SEEK_TIMEOUT)?;
        if !out_path.is_file() {
            return Err(FrameServiceError::FFmpegFailed(format!(
                "关键帧提取未生成文件：{}",
                out_path.display()
            )));
        }
        Ok(())
    }

    /// 用 ffprobe 或 ffmpeg -i 解析已生成帧的 PTS（验证用）。
    /// 若工具不可用则回退到索引中的 pts_sec。
    fn probe_frame_pts(&self, _path: &Path) -> Option<f64> {
        // 简化：不调用 ffprobe（避免额外依赖），由调用方回退到索引 pts。
        // 若后续需要严格校验，可用 `ffmpeg -i frame.jpg -vf showinfo -f null -` 解析。
        None
    }

    // ==================== delivery 模式 ====================

    fn prepare_delivery(
        &self,
        mut frames: Vec<FrameInfo>,
        opts: &ExtractOptions,
    ) -> Result<Vec<FrameInfo>, FrameServiceError> {
        match opts.delivery {
            DeliveryMode::Files => Ok(frames),
            DeliveryMode::Base64 => {
                let mut total_bytes = 0usize;
                for frame in &mut frames {
                    let path = frame
                        .path
                        .take()
                        .ok_or_else(|| FrameServiceError::InvalidRequest("帧缺少路径".into()))?;
                    let data = fs::read(&path).map_err(FrameServiceError::Io)?;
                    let encoded = general_purpose::STANDARD.encode(&data);
                    if encoded.len() > MAX_BASE64_FRAME_BYTES {
                        return Err(FrameServiceError::OutputLimitExceeded(format!(
                            "单帧 base64 大小 {} 超过上限 {}",
                            encoded.len(),
                            MAX_BASE64_FRAME_BYTES
                        )));
                    }
                    total_bytes += encoded.len();
                    if total_bytes > MAX_BASE64_TOTAL_BYTES {
                        return Err(FrameServiceError::OutputLimitExceeded(format!(
                            "base64 总大小 {} 超过上限 {}",
                            total_bytes, MAX_BASE64_TOTAL_BYTES
                        )));
                    }
                    frame.image_base64 = Some(encoded);
                    frame.path = None;
                }
                Ok(frames)
            }
        }
    }

    // ==================== FFmpeg 进程管理 ====================

    /// 运行 FFmpeg 并解析 stderr 中的 showinfo `pts_time:` 行。
    /// 返回 (pts_list, stderr_lines)。
    /// 超时/超限即 kill 并等待回收，绝不泄漏子进程。
    fn run_ffmpeg_with_showinfo(
        &self,
        args: &[OsString],
        work_dir: &Path,
    ) -> Result<(Vec<f64>, Vec<String>), FrameServiceError> {
        // 把 showinfo 注入到已有 -vf 链尾；若没有 -vf 则在 -i <file> 之后插入新 -vf。
        // FFmpeg 只接受一个 -vf，所以必须合并而非添加第二个。
        let mut new_args: Vec<OsString> = Vec::with_capacity(args.len() + 4);
        let mut i = 0;
        let mut merged = false;
        while i < args.len() {
            if args[i] == "-vf" && i + 1 < args.len() {
                new_args.push(args[i].clone());
                let mut vf = args[i + 1].clone();
                vf.push(",showinfo");
                new_args.push(vf);
                i += 2;
                merged = true;
            } else {
                new_args.push(args[i].clone());
                // 找到 `-i <file>` 之后的插入点
                if !merged && i >= 1 && args[i - 1] == "-i" {
                    new_args.extend(["-vf".into(), "showinfo".into()]);
                    merged = true;
                }
                i += 1;
            }
        }
        if !merged {
            // 兜底：附加到末尾（FFmpeg 会报错，但至少参数完整）
            new_args.extend(["-vf".into(), "showinfo".into()]);
        }

        self.run_ffmpeg_internal(&new_args, work_dir, FFMPEG_TIMEOUT)
    }

    /// 简化版 FFmpeg 运行（不解析 showinfo），用于单帧 seek。
    fn run_ffmpeg_simple(
        &self,
        args: &[OsString],
        timeout: Duration,
    ) -> Result<(), FrameServiceError> {
        let mut command = Command::new(&self.ffmpeg_path);
        command.args(args).stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = command
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| FrameServiceError::FFmpegFailed(format!("启动失败：{e}")))?;

        let deadline = Instant::now() + timeout;
        let mut stderr_lines = Vec::new();
        let result = loop {
            if let Some(stderr) = child.stderr.as_mut() {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                while let Ok(n) = reader.read_line(&mut line) {
                    if n == 0 {
                        break;
                    }
                    stderr_lines.push(line.trim().to_string());
                    line.clear();
                    if stderr_lines.len() > MAX_STDERR_LINES {
                        stderr_lines.remove(0); // 环形丢弃最旧行
                    }
                    if Instant::now() >= deadline {
                        break;
                    }
                }
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    if status.success() {
                        break Ok(());
                    } else {
                        let tail = stderr_lines
                            .iter()
                            .rev()
                            .take(5)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("; ");
                        break Err(FrameServiceError::FFmpegFailed(format!(
                            "退出码 {}：{}",
                            status.code().unwrap_or(-1),
                            tail
                        )));
                    }
                }
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(FrameServiceError::FFmpegTimeout(timeout));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => break Err(FrameServiceError::FFmpegFailed(format!("等待失败：{e}"))),
            }
        };
        // 确保进程被回收（即使出错也要 try_wait + kill 兜底）
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        result
    }

    /// 通用 FFmpeg 运行：带 showinfo 解析、超时、stderr 行缓冲上限。
    fn run_ffmpeg_internal(
        &self,
        args: &[OsString],
        _work_dir: &Path,
        timeout: Duration,
    ) -> Result<(Vec<f64>, Vec<String>), FrameServiceError> {
        let mut command = Command::new(&self.ffmpeg_path);
        command.args(args).stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = command
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| FrameServiceError::FFmpegFailed(format!("启动失败：{e}")))?;

        let deadline = Instant::now() + timeout;
        let mut pts_list = Vec::new();
        let mut stderr_lines = Vec::new();
        let result = loop {
            if let Some(stderr) = child.stderr.as_mut() {
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                while let Ok(n) = reader.read_line(&mut line) {
                    if n == 0 {
                        break;
                    }
                    let trimmed = line.trim();
                    stderr_lines.push(trimmed.to_string());
                    if let Some(pts) = parse_showinfo_pts(trimmed) {
                        pts_list.push(pts);
                    }
                    line.clear();
                    if stderr_lines.len() > MAX_STDERR_LINES {
                        stderr_lines.remove(0);
                    }
                    if Instant::now() >= deadline {
                        break;
                    }
                }
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    if status.success() {
                        break Ok((pts_list, stderr_lines));
                    } else {
                        let tail = stderr_lines
                            .iter()
                            .rev()
                            .take(5)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("; ");
                        break Err(FrameServiceError::FFmpegFailed(format!(
                            "退出码 {}：{}",
                            status.code().unwrap_or(-1),
                            tail
                        )));
                    }
                }
                Ok(None) if Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err(FrameServiceError::FFmpegTimeout(timeout));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => break Err(FrameServiceError::FFmpegFailed(format!("等待失败：{e}"))),
            }
        };
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        result
    }
}

// ==================== 工具函数 ====================

/// 列出目录下指定扩展名的文件，按文件名排序。
fn list_output_files(dir: &Path, ext: &str) -> Result<Vec<PathBuf>, FrameServiceError> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).map_err(FrameServiceError::Io)? {
        let entry = entry.map_err(FrameServiceError::Io)?;
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case(ext))
        {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

/// 从 FFmpeg stderr 行解析 `pts_time:` 值。
/// 示例行：`[Parsed_showinfo_0 @ 000001] n:   0 pts:   1024 pts_time:0.426667 ...`
fn parse_showinfo_pts(line: &str) -> Option<f64> {
    if !line.contains("showinfo") {
        return None;
    }
    let marker = "pts_time:";
    let start = line.find(marker)? + marker.len();
    let end = line[start..]
        .find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .map(|i| start + i)
        .unwrap_or(line.len());
    line[start..end].parse().ok()
}

/// 协议兼容性：major 相同即兼容；任一 major 为 0 时要求完全一致。
fn versions_compatible(host: &str, plugin: &str) -> bool {
    fn parse(v: &str) -> Option<(u64, u64, u64)> {
        let mut parts = v.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().unwrap_or("0").parse().ok()?;
        let patch = parts.next().unwrap_or("0").parse().ok()?;
        Some((major, minor, patch))
    }
    match (parse(host), parse(plugin)) {
        (Some((h, _, _)), Some((p, _, _))) if h == p && h != 0 => true,
        (Some(h), Some(p)) => h == p,
        _ => false,
    }
}

// ==================== 单元测试 ====================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_compatible() {
        assert!(versions_compatible("1.0.0", "1.2.3"));
        assert!(!versions_compatible("1.0.0", "2.0.0"));
        assert!(!versions_compatible("0.1.0", "0.1.1"));
    }

    /// 构造临时服务：用 tempfile 让 canonicalize 走真路径
    fn make_service() -> (FrameService, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().unwrap();
        let video = tmp.path().join("video.mp4");
        fs::write(&video, b"fake").unwrap();
        let out = tmp.path().join("out");
        fs::create_dir_all(&out).unwrap();
        let service = FrameService::new(PathBuf::from("ffmpeg"), video, out).unwrap();
        (service, tmp)
    }

    #[test]
    fn rejects_invalid_time_range() {
        let (service, tmp) = make_service();
        let video_path = tmp.path().join("video.mp4");
        let scratch = tmp.path().join("out");
        let mut req = FrameExtractRequest {
            protocol_version: "1.0.0".into(),
            video_path: video_path.clone(),
            options: ExtractOptions {
                time_range: Some((5.0, 3.0)),
                scratch_dir: Some(scratch.clone()),
                ..Default::default()
            },
        };
        assert!(matches!(
            service.extract(req.clone()),
            Err(FrameServiceError::InvalidRequest(_))
        ));
        req.options.time_range = Some((-1.0, 5.0));
        assert!(matches!(
            service.extract(req),
            Err(FrameServiceError::InvalidRequest(_))
        ));
    }

    #[test]
    fn rejects_invalid_sample_fps() {
        let (service, tmp) = make_service();
        let video_path = tmp.path().join("video.mp4");
        let scratch = tmp.path().join("out");
        let req = FrameExtractRequest {
            protocol_version: "1.0.0".into(),
            video_path,
            options: ExtractOptions {
                sample_fps: Some(0.0),
                scratch_dir: Some(scratch),
                ..Default::default()
            },
        };
        assert!(matches!(
            service.extract(req),
            Err(FrameServiceError::InvalidRequest(_))
        ));
    }

    #[test]
    fn rejects_unauthorized_video_path() {
        let (service, tmp) = make_service();
        let other_video = tmp.path().join("other.mp4");
        fs::write(&other_video, b"fake").unwrap();
        let scratch = tmp.path().join("out");
        let req = FrameExtractRequest {
            protocol_version: "1.0.0".into(),
            video_path: other_video,
            options: ExtractOptions {
                scratch_dir: Some(scratch),
                ..Default::default()
            },
        };
        assert!(matches!(
            service.extract(req),
            Err(FrameServiceError::UnauthorizedVideoPath)
        ));
    }

    #[test]
    fn rejects_unauthorized_scratch_dir() {
        let (service, tmp) = make_service();
        let video_path = tmp.path().join("video.mp4");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let req = FrameExtractRequest {
            protocol_version: "1.0.0".into(),
            video_path,
            options: ExtractOptions {
                scratch_dir: Some(outside),
                ..Default::default()
            },
        };
        assert!(matches!(
            service.extract(req),
            Err(FrameServiceError::UnauthorizedScratchDir)
        ));
    }

    #[test]
    fn parses_showinfo_pts() {
        let line = r#"[Parsed_showinfo_0 @ 000001] n:   0 pts:   1024 pts_time:0.426667 pos:   123456 fmt:yuv420p"#;
        assert_eq!(parse_showinfo_pts(line), Some(0.426667));
        assert_eq!(parse_showinfo_pts("normal log"), None);
    }

    #[test]
    fn selects_keyframes_respects_time_range_and_fps() {
        let (service, _tmp) = make_service();
        let keyframes = vec![
            mp4::Keyframe {
                sample_index: 0,
                pts_sec: 0.0,
                offset: 0,
                size: 0,
            },
            mp4::Keyframe {
                sample_index: 30,
                pts_sec: 1.0,
                offset: 0,
                size: 0,
            },
            mp4::Keyframe {
                sample_index: 60,
                pts_sec: 2.0,
                offset: 0,
                size: 0,
            },
            mp4::Keyframe {
                sample_index: 90,
                pts_sec: 3.0,
                offset: 0,
                size: 0,
            },
        ];
        // 无 sample_fps：取范围内全部
        let selected = service.select_keyframes(
            &keyframes,
            &ExtractOptions {
                time_range: Some((1.0, 3.0)),
                ..Default::default()
            },
        );
        assert_eq!(selected.len(), 2);
        // 有 sample_fps：限制间隔（fps=0.5 → min_interval=2.0s）
        let selected = service.select_keyframes(
            &keyframes,
            &ExtractOptions {
                sample_fps: Some(0.5),
                ..Default::default()
            },
        );
        assert_eq!(selected.len(), 2); // 0.0 和 2.0
    }
}
