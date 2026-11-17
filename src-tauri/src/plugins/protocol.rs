use crate::formats::Mark;
use serde::{Deserialize, Serialize};

/// 插件 JSON-RPC 协议版本（major 相同即兼容；major 为 0 时要求完全一致）
pub const PROTOCOL_VERSION: &str = "1.0.0";

/// 识别服务协议版本
pub const RECOGNITION_PROTOCOL_VERSION: &str = "1.0.0";

/// 归一化矩形 [0,1]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl NormalizedRect {
    pub fn to_pixels(&self, img_w: u32, img_h: u32) -> (i32, i32, i32, i32) {
        (
            (self.x * img_w as f64) as i32,
            (self.y * img_h as f64) as i32,
            (self.width * img_w as f64) as i32,
            (self.height * img_h as f64) as i32,
        )
    }

    pub fn from_pixels(x: i32, y: i32, w: i32, h: i32, img_w: u32, img_h: u32) -> Self {
        Self {
            x: x as f64 / img_w as f64,
            y: y as f64 / img_h as f64,
            width: w as f64 / img_w as f64,
            height: h as f64 / img_h as f64,
        }
    }
}

/// 模板匹配结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemplateMatchResult {
    pub rect: NormalizedRect,
    pub confidence: f64,
    pub scale: f64,
    pub template_name: String,
    pub timestamp: f64,
}

/// OCR 结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrResult {
    pub rect: NormalizedRect,
    pub text: String,
    pub confidence: f64,
    pub timestamp: f64,
}

/// 目标检测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectionResult {
    pub rect: NormalizedRect,
    pub label: String,
    pub confidence: f64,
    pub class_id: i32,
    pub timestamp: f64,
}

/// 预览帧
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFrame {
    pub frame_index: i32,
    pub timestamp: f64,
    pub image_base64: String,
    pub width: u32,
    pub height: u32,
}

/// 识别服务初始化响应
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionInitializeResult {
    pub protocol_version: String,
    pub service_name: String,
    pub service_version: String,
    pub capabilities: serde_json::Value,
}

/// JSON-RPC 请求
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcRequest {
    pub jsonrpc: String,
    pub id: u64,
    pub method: String,
    pub params: serde_json::Value,
}

impl RpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 响应（`id` 必须匹配请求；通知消息没有 `id`，不入此结构）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcResponse {
    pub jsonrpc: String,
    pub id: u64,
    pub result: Option<serde_json::Value>,
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    pub data: Option<serde_json::Value>,
}

/// 插件 → 主程序的 JSON-RPC 通知（无 `id`）。
///
/// 约定 method：`progress` / `log`；其余自定义 method 也会透传给 UI。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// `initialize` 方法的响应：插件回告自身协议版本与能力
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    #[serde(default)]
    pub plugin_name: Option<String>,
    #[serde(default)]
    pub capabilities: serde_json::Value,
}

/// `run` 方法参数：一次执行任务的完整上下文
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunParams {
    pub task_id: String,
    pub video: RunVideoInfo,
    #[serde(default)]
    pub params: serde_json::Value,
    /// 允许插件写入证据截图等产物的授权目录（铁律 #5：仅限此目录）
    pub output_dir: std::path::PathBuf,
}

/// 传给插件的视频信息（只读描述，不是文件句柄）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunVideoInfo {
    pub id: String,
    pub path: std::path::PathBuf,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    #[serde(default)]
    pub metadata: std::collections::HashMap<String, String>,
}

/// `progress` 通知的 params
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressParams {
    pub task_id: String,
    /// 0.0 - 1.0
    pub progress: f32,
    #[serde(default)]
    pub message: Option<String>,
    /// 插件已产出的标注数（可选，用于 UI 增量显示）
    #[serde(default)]
    pub marks_so_far: Option<u32>,
}

/// `log` 通知的 params
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogParams {
    pub task_id: String,
    #[serde(default = "default_log_level")]
    pub level: String,
    pub message: String,
}

fn default_log_level() -> String {
    "info".to_string()
}

/// `cancel` 方法参数
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelParams {
    pub task_id: String,
}

/// 插件执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginResult {
    pub protocol_version: String,
    pub plugin_name: String,
    pub marks: Vec<Mark>,
    pub logs: Vec<String>,
    pub evidence_paths: Vec<String>,
}

/// 插件进度事件（Tauri event payload，转发给前端）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginProgressEvent {
    pub schema_version: String,
    pub plugin_name: String,
    pub task_id: String,
    pub progress: f32,
    pub message: Option<String>,
}

/// 插件状态事件（Tauri event payload）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginStatusEvent {
    pub schema_version: String,
    pub plugin_name: String,
    /// discovered | validated | started | ready | running | stopped | crashed
    pub status: String,
    #[serde(default)]
    pub detail: Option<String>,
}
