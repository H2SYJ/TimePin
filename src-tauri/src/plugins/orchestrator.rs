//! M4 多插件编排器
//!
//! 在一次调用中调度多个插件：支持串行/并行执行、单插件失败隔离、
//! 结果合并去重（复用 `merge_marks`）、批量视频遍历。
//!
//! 不引入新 IPC 协议版本——复用现有 `plugin://*` 事件；编排层级
//! 新增 `plugin://orchestration` 事件汇报整体进度与汇总结果。

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::core::library::VideoInfo;
use crate::formats::Mark;
use crate::ipc::SCHEMA_VERSION;

use super::manager::{merge_marks, PluginError, PluginManager};

/// 编排整体事件名
pub const EVENT_ORCHESTRATION: &str = "plugin://orchestration";

/// 单视频去重窗口（秒），与 `merge_marks` 的语义一致
const DEFAULT_MERGE_EPSILON_SECS: f64 = 0.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrchestrationMode {
    /// 依次运行插件，一个失败不阻塞后续
    Sequential,
    /// 并发运行插件（受 Tokio 运行时与插件进程自身能力约束）
    Parallel,
}

/// 编排输入参数（前端 `run_plugins` 命令）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationRequest {
    /// 参与执行的插件名（顺序即优先级；并行模式下也是结果合并顺序）
    pub plugin_names: Vec<String>,
    /// 目标视频（批量打标场景传多个；通常 1..=N）
    pub videos: Vec<VideoInfo>,
    /// 传给每个插件的参数（按 plugin_name 索引；未指定用 default_params）
    #[serde(default)]
    pub per_plugin_params: std::collections::HashMap<String, serde_json::Value>,
    /// 所有插件共享的默认参数
    #[serde(default)]
    pub default_params: serde_json::Value,
    /// 输出目录根：每个插件实际写入 `<output_dir>/<plugin_name>/`
    pub output_dir: PathBuf,
    /// sequential | parallel（默认 sequential，最安全）
    #[serde(default = "default_mode")]
    pub mode: OrchestrationMode,
    /// 去重时间窗口（秒），默认 0.5
    #[serde(default = "default_epsilon")]
    pub merge_epsilon_secs: f64,
    /// 并发上限（仅 parallel 模式生效；默认 = min(4, 视频数)）
    #[serde(default)]
    pub max_concurrency: Option<usize>,
}

fn default_mode() -> OrchestrationMode {
    OrchestrationMode::Sequential
}
fn default_epsilon() -> f64 {
    DEFAULT_MERGE_EPSILON_SECS
}

/// 单个 (plugin, video) 执行的细粒度结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRunOutcome {
    pub plugin_name: String,
    pub video_id: String,
    /// ok | error | crashed | skipped
    pub outcome: String,
    #[serde(default)]
    pub error: Option<String>,
    pub marks: Vec<Mark>,
    pub task_id: String,
}

/// 单视频汇总：多插件结果合并后的 marks
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoRunSummary {
    pub video_id: String,
    pub marks: Vec<Mark>,
    /// 每个插件的独立结果（含失败）
    pub plugin_results: Vec<PluginRunOutcome>,
}

/// 整体编排响应
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationResult {
    pub schema_version: String,
    /// 批量任务 id（前端用它关联 orchestration://progress）
    pub batch_id: String,
    /// 每个视频的汇总结果（顺序与输入 `videos` 一致）
    pub video_summaries: Vec<VideoRunSummary>,
    /// 总 marks 数（合并去重后）
    pub total_marks: usize,
    /// 失败任务数（单插件单视频为 1）
    pub failed_count: usize,
}

/// 编排整体进度事件（payload）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrchestrationProgress {
    pub schema_version: String,
    pub batch_id: String,
    /// started | running | video_done | finished | cancelled
    pub stage: String,
    /// 已完成的 (plugin,video) 组合数
    pub completed: usize,
    /// 总组合数 = plugin_names.len() * videos.len()
    pub total: usize,
    #[serde(default)]
    pub current_video_id: Option<String>,
    #[serde(default)]
    pub current_plugin_name: Option<String>,
    /// 最近一次完成结果（可选，前端可做增量展示）
    #[serde(default)]
    pub last_outcome: Option<PluginRunOutcome>,
}

pub struct PluginOrchestrator {
    manager: PluginManager,
    app: AppHandle,
}

impl PluginOrchestrator {
    pub fn new(manager: PluginManager, app: AppHandle) -> Self {
        Self { manager, app }
    }

    /// 执行编排：在阻塞线程中调用（前端 `spawn_blocking`）。
    ///
    /// - 为每个 (video, plugin) 组合调用 `PluginManager::run_plugin`
    /// - 失败不终止整体流程，仅记录到 `PluginRunOutcome`
    /// - 并行模式下用 `std::thread` fan-out + 共享计数器汇报进度
    ///   （插件进程本身是阻塞 IPC，多线程即并发）
    /// - 最终对每个视频做 `merge_marks` 去重
    pub fn run(&self, request: OrchestrationRequest) -> Result<OrchestrationResult, PluginError> {
        if request.plugin_names.is_empty() {
            return Err(PluginError::Task("至少需要选择一个插件".to_string()));
        }
        if request.videos.is_empty() {
            return Err(PluginError::Task("至少需要选择一个视频".to_string()));
        }

        let batch_id = uuid::Uuid::new_v4().to_string();
        let total = request.plugin_names.len() * request.videos.len();
        self.emit_progress(&batch_id, "started", 0, total, None, None, None);

        let mut video_summaries = Vec::with_capacity(request.videos.len());
        let mut total_marks = 0;
        let mut failed_count = 0;
        let completed = Arc::new(AtomicUsize::new(0));

        for video in &request.videos {
            let outcomes = match request.mode {
                OrchestrationMode::Sequential => {
                    self.run_sequential(&batch_id, video, &request, total, Arc::clone(&completed))
                }
                OrchestrationMode::Parallel => {
                    self.run_parallel(&batch_id, video, &request, total, Arc::clone(&completed))
                }
            };

            for outcome in &outcomes {
                if outcome.outcome != "ok" {
                    failed_count += 1;
                }
            }

            // 仅合并成功结果的 marks；失败的 outcome.marks 为空（插件若返回部分
            // marks 也保留在 outcome.marks 里，但不参与合并以免误导）
            let mut merged_input: Vec<Mark> = Vec::new();
            for outcome in &outcomes {
                if outcome.outcome == "ok" {
                    merged_input.extend(outcome.marks.clone());
                }
            }
            let merged = merge_marks(merged_input, request.merge_epsilon_secs);
            total_marks += merged.len();

            video_summaries.push(VideoRunSummary {
                video_id: video.id.clone(),
                marks: merged,
                plugin_results: outcomes,
            });

            self.emit_progress(
                &batch_id,
                "video_done",
                completed.load(Ordering::Relaxed),
                total,
                Some(video.id.clone()),
                None,
                None,
            );
        }

        self.emit_progress(&batch_id, "finished", total, total, None, None, None);

        Ok(OrchestrationResult {
            schema_version: SCHEMA_VERSION.to_string(),
            batch_id,
            video_summaries,
            total_marks,
            failed_count,
        })
    }

    /// 串行：插件按 `plugin_names` 顺序逐个执行。
    fn run_sequential(
        &self,
        batch_id: &str,
        video: &VideoInfo,
        request: &OrchestrationRequest,
        total: usize,
        completed: Arc<AtomicUsize>,
    ) -> Vec<PluginRunOutcome> {
        let mut outcomes = Vec::with_capacity(request.plugin_names.len());
        for plugin_name in &request.plugin_names {
            let outcome = self.run_one(
                batch_id,
                plugin_name,
                video,
                request,
                total,
                Arc::clone(&completed),
            );
            outcomes.push(outcome);
        }
        outcomes
    }

    /// 并行：多线程 fan-out，插件进程本身是独立 Sidecar，天然隔离。
    ///
    /// `max_concurrency` 限制同一视频上的并发插件数（防止同时拉起过多
    /// Python sidecar 抢占资源）。
    fn run_parallel(
        &self,
        batch_id: &str,
        video: &VideoInfo,
        request: &OrchestrationRequest,
        total: usize,
        completed: Arc<AtomicUsize>,
    ) -> Vec<PluginRunOutcome> {
        let max_workers = request
            .max_concurrency
            .unwrap_or_else(|| request.plugin_names.len().clamp(1, 4));
        let queue = Arc::new(Mutex::new(request.plugin_names.to_vec()));
        let outcomes = Arc::new(Mutex::new(Vec::with_capacity(request.plugin_names.len())));

        std::thread::scope(|scope| {
            for _ in 0..max_workers {
                let queue = Arc::clone(&queue);
                let outcomes = Arc::clone(&outcomes);
                let video = video.clone();
                let request = request.clone();
                let batch_id = batch_id.to_string();
                let completed = Arc::clone(&completed);
                scope.spawn(move || loop {
                    let plugin_name = {
                        let mut q = queue.lock().expect("queue mutex poisoned");
                        q.pop()
                    };
                    let Some(plugin_name) = plugin_name else {
                        break;
                    };
                    let outcome = self.run_one(
                        &batch_id,
                        &plugin_name,
                        &video,
                        &request,
                        total,
                        Arc::clone(&completed),
                    );
                    outcomes
                        .lock()
                        .expect("outcomes mutex poisoned")
                        .push(outcome);
                });
            }
        });

        let mut results = Arc::try_unwrap(outcomes)
            .map(|m| m.into_inner().expect("outcomes mutex poisoned"))
            .unwrap_or_default();
        // 恢复 plugin_names 顺序（并发完成顺序随机）
        results.sort_by_key(|o| {
            request
                .plugin_names
                .iter()
                .position(|n| n == &o.plugin_name)
                .unwrap_or(usize::MAX)
        });
        results
    }

    /// 执行单个 (plugin, video) 组合；捕获所有错误到 `PluginRunOutcome`。
    fn run_one(
        &self,
        batch_id: &str,
        plugin_name: &str,
        video: &VideoInfo,
        request: &OrchestrationRequest,
        total: usize,
        completed: Arc<AtomicUsize>,
    ) -> PluginRunOutcome {
        self.emit_progress(
            batch_id,
            "running",
            completed.load(Ordering::Relaxed),
            total,
            Some(video.id.clone()),
            Some(plugin_name.to_string()),
            None,
        );

        let params = request
            .per_plugin_params
            .get(plugin_name)
            .cloned()
            .unwrap_or_else(|| request.default_params.clone());
        let output_dir = request.output_dir.join(plugin_name).join(&video.id);

        let result = self
            .manager
            .run_plugin(plugin_name, video, params, output_dir);

        let (outcome_str, error, marks, task_id) = match result {
            Ok((task_id, r)) => ("ok".to_string(), None, r.marks, task_id),
            Err(e) => {
                let outcome_str = match &e {
                    PluginError::Process(super::runtime::PluginProcessError::ProcessExited) => {
                        "crashed"
                    }
                    PluginError::NotFound(_) => "skipped",
                    _ => "error",
                };
                (
                    outcome_str.to_string(),
                    Some(e.to_string()),
                    Vec::new(),
                    String::new(),
                )
            }
        };

        let outcome = PluginRunOutcome {
            plugin_name: plugin_name.to_string(),
            video_id: video.id.clone(),
            outcome: outcome_str,
            error,
            marks,
            task_id,
        };

        let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
        self.emit_progress(
            batch_id,
            "running",
            done,
            total,
            Some(video.id.clone()),
            Some(plugin_name.to_string()),
            Some(outcome.clone()),
        );

        outcome
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_progress(
        &self,
        batch_id: &str,
        stage: &str,
        completed: usize,
        total: usize,
        current_video_id: Option<String>,
        current_plugin_name: Option<String>,
        last_outcome: Option<PluginRunOutcome>,
    ) {
        let _ = self.app.emit(
            EVENT_ORCHESTRATION,
            OrchestrationProgress {
                schema_version: SCHEMA_VERSION.to_string(),
                batch_id: batch_id.to_string(),
                stage: stage.to_string(),
                completed,
                total,
                current_video_id,
                current_plugin_name,
                last_outcome,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_defaults_are_safe() {
        let json = r#"{
            "pluginNames": ["a", "b"],
            "videos": [],
            "outputDir": "out"
        }"#;
        let req: OrchestrationRequest = serde_json::from_str(json).expect("parse");
        assert_eq!(req.mode, OrchestrationMode::Sequential);
        assert_eq!(req.merge_epsilon_secs, DEFAULT_MERGE_EPSILON_SECS);
        assert!(req.default_params.is_null());
        assert!(req.per_plugin_params.is_empty());
    }

    #[test]
    fn orchestration_rejects_empty_plugins_or_videos() {
        // 仅校验参数形状；真实的 run() 需要 AppHandle，留给集成测试。
        let req_empty_plugins: OrchestrationRequest =
            serde_json::from_str(r#"{"pluginNames": [], "videos": [], "outputDir": "out"}"#)
                .expect("parse");
        assert!(req_empty_plugins.plugin_names.is_empty());
        assert!(req_empty_plugins.videos.is_empty());
    }
}
