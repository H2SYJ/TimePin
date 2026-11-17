//! 识别服务客户端：连接 Python Sidecar，提供类型安全的调用接口。

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::plugins::protocol::{
    DetectionResult, NormalizedRect, OcrResult, PreviewFrame,
    RECOGNITION_PROTOCOL_VERSION, TemplateMatchResult,
};

const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(300);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Error)]
pub enum RecognitionError {
    #[error("无法启动识别服务进程：{0}")]
    SpawnFailed(String),
    #[error("识别服务握手失败：{0}")]
    HandshakeFailed(String),
    #[error("协议不兼容：主程序 {host}，服务 {service}")]
    IncompatibleProtocol { host: String, service: String },
    #[error("识别服务进程已退出")]
    ProcessExited,
    #[error("等待响应超时（{0:?}）")]
    Timeout(Duration),
    #[error("服务返回错误 [{code}] {message}")]
    RemoteError { code: i32, message: String },
    #[error("响应解析失败：{0}")]
    InvalidResponse(String),
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
    #[error("JSON 序列化错误：{0}")]
    Json(#[from] serde_json::Error),
}

pub type NotificationHandler = Arc<dyn Fn(RecognitionNotification) + Send + Sync>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Incoming {
    Response(RpcResponse),
    Notification(RecognitionNotification),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    id: u64,
    result: Option<serde_json::Value>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcError {
    code: i32,
    message: String,
    #[allow(dead_code)]
    data: Option<serde_json::Value>,
}

/// 识别服务进程
pub struct RecognitionProcess {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<RpcResponse>,
    #[allow(dead_code)]
    reader_thread: thread::JoinHandle<()>,
}

impl RecognitionProcess {
    pub fn spawn(
        service_dir: &Path,
        on_notify: Option<NotificationHandler>,
    ) -> Result<Self, RecognitionError> {
        let entry = service_dir.join("main.py");
        let program = if cfg!(windows) { "python" } else { "python3" };

        let mut child = Command::new(program)
            .arg(entry)
            .current_dir(service_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| RecognitionError::SpawnFailed(format!("{program}: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| RecognitionError::SpawnFailed("无法获取 stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| RecognitionError::SpawnFailed("无法获取 stdout".to_string()))?;
        let stderr = child.stderr.take();

        let (tx, rx) = mpsc::channel::<RpcResponse>();
        let reader_thread = spawn_reader(stdout, stderr, tx, on_notify);

        let mut process = Self {
            child,
            stdin,
            rx,
            reader_thread,
        };

        process.handshake()?;
        Ok(process)
    }

    fn handshake(&mut self) -> Result<(), RecognitionError> {
        let params = json!({
            "protocolVersion": RECOGNITION_PROTOCOL_VERSION,
        });
        let response = self.call_with_timeout("initialize", params, HANDSHAKE_TIMEOUT)?;
        let service_version = response
            .get("protocolVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if !versions_compatible(RECOGNITION_PROTOCOL_VERSION, &service_version) {
            return Err(RecognitionError::IncompatibleProtocol {
                host: RECOGNITION_PROTOCOL_VERSION.to_string(),
                service: service_version,
            });
        }
        Ok(())
    }

    pub fn template_match(
        &mut self,
        params: TemplateMatchParams,
        timeout: Option<Duration>,
    ) -> Result<TemplateMatchResponse, RecognitionError> {
        let result = self.call_with_timeout("recognize.template_match", serde_json::to_value(params)?, timeout.unwrap_or(DEFAULT_CALL_TIMEOUT))?;
        serde_json::from_value(result).map_err(|e| RecognitionError::InvalidResponse(e.to_string()))
    }

    pub fn ocr(
        &mut self,
        params: OcrParams,
        timeout: Option<Duration>,
    ) -> Result<OcrResponse, RecognitionError> {
        let result = self.call_with_timeout("recognize.ocr", serde_json::to_value(params)?, timeout.unwrap_or(DEFAULT_CALL_TIMEOUT))?;
        serde_json::from_value(result).map_err(|e| RecognitionError::InvalidResponse(e.to_string()))
    }

    pub fn detect(
        &mut self,
        params: DetectParams,
        timeout: Option<Duration>,
    ) -> Result<DetectResponse, RecognitionError> {
        let result = self.call_with_timeout("recognize.detect", serde_json::to_value(params)?, timeout.unwrap_or(DEFAULT_CALL_TIMEOUT))?;
        serde_json::from_value(result).map_err(|e| RecognitionError::InvalidResponse(e.to_string()))
    }

    pub fn preview(
        &mut self,
        params: PreviewParams,
        timeout: Option<Duration>,
    ) -> Result<PreviewFrame, RecognitionError> {
        let result = self.call_with_timeout("recognize.preview", serde_json::to_value(params)?, timeout.unwrap_or(DEFAULT_CALL_TIMEOUT))?;
        serde_json::from_value(result).map_err(|e| RecognitionError::InvalidResponse(e.to_string()))
    }

    pub fn roi_save(&mut self, params: RoiSaveParams) -> Result<RoiSaveResponse, RecognitionError> {
        let result = self.call_with_timeout("recognize.roi.save", serde_json::to_value(params)?, DEFAULT_CALL_TIMEOUT)?;
        serde_json::from_value(result).map_err(|e| RecognitionError::InvalidResponse(e.to_string()))
    }

    pub fn roi_load(&mut self, params: RoiLoadParams) -> Result<RoiLoadResponse, RecognitionError> {
        let result = self.call_with_timeout("recognize.roi.load", serde_json::to_value(params)?, DEFAULT_CALL_TIMEOUT)?;
        serde_json::from_value(result).map_err(|e| RecognitionError::InvalidResponse(e.to_string()))
    }

    pub fn roi_list(&mut self) -> Result<RoiListResponse, RecognitionError> {
        let result = self.call_with_timeout("recognize.roi.list", json!({}), DEFAULT_CALL_TIMEOUT)?;
        serde_json::from_value(result).map_err(|e| RecognitionError::InvalidResponse(e.to_string()))
    }

    pub fn configure(&mut self, params: ConfigureParams) -> Result<(), RecognitionError> {
        let _ = self.call_with_timeout("recognize.configure", serde_json::to_value(params)?, DEFAULT_CALL_TIMEOUT)?;
        Ok(())
    }

    pub fn cancel(&mut self, task_id: &str) {
        let id = next_id();
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "cancel",
            "params": { "taskId": task_id },
        });
        if let Ok(line) = serde_json::to_string(&req) {
            let _ = self.stdin.write_all(line.as_bytes());
            let _ = self.stdin.write_all(b"\n");
            let _ = self.stdin.flush();
        }
    }

    pub fn shutdown(&mut self, grace: Duration) {
        let _ = self.call_with_timeout("shutdown", json!({}), grace.min(Duration::from_secs(5)));
        match self.child.try_wait() {
            Ok(Some(_)) => {}
            _ => {
                let deadline = Instant::now() + grace;
                loop {
                    match self.child.try_wait() {
                        Ok(Some(_)) => break,
                        _ if Instant::now() >= deadline => {
                            let _ = self.child.kill();
                            break;
                        }
                        _ => thread::sleep(Duration::from_millis(20)),
                    }
                }
            }
        }
    }

    fn call_with_timeout(
        &mut self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, RecognitionError> {
        let id = next_id();
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&req)
            .map_err(|e| RecognitionError::InvalidResponse(e.to_string()))?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.flush()?;

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.checked_duration_since(Instant::now());
            let Some(remaining) = remaining else {
                return Err(RecognitionError::Timeout(timeout));
            };
            match self.rx.recv_timeout(remaining.min(Duration::from_millis(250))) {
                Ok(response) if response.id == id => {
                    if let Some(error) = response.error {
                        return Err(RecognitionError::RemoteError {
                            code: error.code,
                            message: error.message,
                        });
                    }
                    return Ok(response.result.unwrap_or(serde_json::Value::Null));
                }
                Ok(_) => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !self.is_alive() {
                        return Err(RecognitionError::ProcessExited);
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(RecognitionError::ProcessExited);
                }
            }
        }
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for RecognitionProcess {
    fn drop(&mut self) {
        self.shutdown(Duration::from_secs(2));
    }
}

fn spawn_reader(
    stdout: ChildStdout,
    stderr: Option<std::process::ChildStderr>,
    tx: mpsc::Sender<RpcResponse>,
    on_notify: Option<NotificationHandler>,
) -> thread::JoinHandle<()> {
    if let Some(stderr) = stderr {
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for _line in reader.lines().map_while(Result::ok) {
                // 忽略 stderr
            }
        });
    }

    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<Incoming>(trimmed) {
                Ok(Incoming::Response(response)) => {
                    if tx.send(response).is_err() {
                        break;
                    }
                }
                Ok(Incoming::Notification(notification)) => {
                    if let Some(handler) = &on_notify {
                        handler(notification);
                    }
                }
                Err(_) => continue,
            }
        }
    })
}

/// 协议兼容性：major 相同即兼容；任一 major 为 0 时要求完全一致。
fn versions_compatible(host: &str, service: &str) -> bool {
    fn parse(v: &str) -> Option<(u64, u64, u64)> {
        let mut parts = v.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().unwrap_or("0").parse().ok()?;
        let patch = parts.next().unwrap_or("0").parse().ok()?;
        Some((major, minor, patch))
    }
    match (parse(host), parse(service)) {
        (Some((h, _, _)), Some((s, _, _))) if h == s && h != 0 => true,
        (Some(h), Some(s)) => h == s,
        _ => false,
    }
}

// ==================== 请求/响应类型 ====================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateMatchParams {
    pub task_id: String,
    pub video_path: String,
    pub template_name: String,
    #[serde(default = "default_threshold")]
    pub threshold: f64,
    #[serde(default)]
    pub scales: Option<Vec<f64>>,
    #[serde(default)]
    pub roi: Option<NormalizedRect>,
    #[serde(default)]
    pub frame_timestamps: Option<Vec<f64>>,
    #[serde(default = "default_max_frames")]
    pub max_frames: u32,
}

fn default_threshold() -> f64 { 0.8 }
fn default_max_frames() -> u32 { 50 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateMatchResponse {
    pub matches: Vec<TemplateMatchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrParams {
    pub task_id: String,
    pub video_path: String,
    #[serde(default)]
    pub roi: Option<NormalizedRect>,
    #[serde(default)]
    pub frame_timestamps: Option<Vec<f64>>,
    #[serde(default = "default_max_frames")]
    pub max_frames: u32,
    #[serde(default)]
    pub lang: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrResponse {
    pub results: Vec<OcrResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectParams {
    pub task_id: String,
    pub video_path: String,
    #[serde(default)]
    pub roi: Option<NormalizedRect>,
    #[serde(default)]
    pub frame_timestamps: Option<Vec<f64>>,
    #[serde(default = "default_max_frames")]
    pub max_frames: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectResponse {
    pub results: Vec<DetectionResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewParams {
    pub task_id: String,
    pub video_path: String,
    pub timestamp: f64,
    #[serde(default)]
    pub template_matches: Vec<TemplateMatchResult>,
    #[serde(default)]
    pub ocr_results: Vec<OcrResult>,
    #[serde(default)]
    pub detections: Vec<DetectionResult>,
    #[serde(default)]
    pub rois: Vec<NormalizedRect>,
    #[serde(default = "default_quality")]
    pub quality: u32,
}

fn default_quality() -> u32 { 80 }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoiSaveParams {
    pub name: String,
    pub rect: NormalizedRect,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoiSaveResponse {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoiLoadParams {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoiLoadResponse {
    pub rect: NormalizedRect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoiListResponse {
    pub rois: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigureParams {
    #[serde(default)]
    pub ocr: Option<serde_json::Value>,
    #[serde(default)]
    pub detector: Option<serde_json::Value>,
    #[serde(default)]
    pub templates: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalized_rect_conversion() {
        let rect = NormalizedRect::from_pixels(100, 100, 200, 150, 1920, 1080);
        assert!((rect.x - 100.0/1920.0).abs() < 0.001);
        assert!((rect.y - 100.0/1080.0).abs() < 0.001);
        assert!((rect.width - 200.0/1920.0).abs() < 0.001);
        assert!((rect.height - 150.0/1080.0).abs() < 0.001);

        let (x, y, w, h) = rect.to_pixels(1920, 1080);
        assert_eq!(x, 100);
        assert_eq!(y, 100);
        assert_eq!(w, 200);
        assert_eq!(h, 150);
    }

    #[test]
    fn test_versions_compatible() {
        assert!(versions_compatible("1.0.0", "1.2.3"));
        assert!(!versions_compatible("1.0.0", "2.0.0"));
        assert!(versions_compatible("0.1.0", "0.1.0"));
        assert!(!versions_compatible("0.1.0", "0.1.1"));
    }
}