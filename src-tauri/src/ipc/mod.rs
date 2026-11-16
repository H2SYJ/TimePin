//! IPC 事件定义（版本化）
//!
//! 所有事件 payload 必须包含 schema_version 字段

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: &str = "1.0.0";

/// 视频导入进度事件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportProgressEvent {
    pub schema_version: String,
    pub task_id: String,
    pub total: usize,
    pub completed: usize,
    pub current_file: Option<String>,
    pub status: ImportStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImportStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

/// 导出进度事件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportProgressEvent {
    pub schema_version: String,
    pub task_id: String,
    pub total: usize,
    pub completed: usize,
    pub current_segment: Option<String>,
    pub status: ExportStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ExportStatus {
    Pending,
    Running,
    Completed,
    Failed,
}
