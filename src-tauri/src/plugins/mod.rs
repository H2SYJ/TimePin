//! 插件系统（M2）
//!
//! - `manifest`：plugin.json 清单结构与校验
//! - `protocol`：JSON-RPC over stdio 协议类型（版本化）
//! - `runtime`：Sidecar 进程运行时（spawn / handshake / call / 通知分发）
//! - `manager`：生命周期管理（发现→校验→启动→执行→回收），崩溃恢复与结果合并
//! - `orchestrator`：M4 多插件编排（串/并行、批量视频、结果合并去重、失败隔离）
//! - `recognition`：识别服务客户端（模板匹配、OCR、目标检测、预览、ROI）

pub mod manager;
pub mod manifest;
pub mod orchestrator;
pub mod protocol;
pub mod recognition;
pub mod runtime;

pub use manager::{PluginError, PluginInfo, PluginManager, PluginRunFinished};
pub use manifest::{PluginCapability, PluginManifest, PluginRuntime};
pub use orchestrator::{
    OrchestrationMode, OrchestrationRequest, OrchestrationResult, PluginOrchestrator,
    PluginRunOutcome, VideoRunSummary,
};
pub use protocol::{PluginResult, PROTOCOL_VERSION};
pub use recognition::{RecognitionError, RecognitionProcess};
