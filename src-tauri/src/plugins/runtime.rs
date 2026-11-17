//! 插件 Sidecar 进程运行时
//!
//! 插件必须运行在独立进程（铁律 #5），通过 stdio 上的 JSON-RPC 与主程序通信。
//! 一行一个 JSON 消息（JSON Lines）：
//! - 请求：`{"jsonrpc":"2.0","id":1,"method":"run","params":{...}}`
//! - 响应：`{"jsonrpc":"2.0","id":1,"result":{...},"error":null}`
//! - 通知（插件 → 主程序，无 id）：`{"jsonrpc":"2.0","method":"progress","params":{...}}`
//!
//! 协议版本见 `PROTOCOL_VERSION`，不兼容的版本握手会被拒绝。

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use super::manifest::PluginManifest;
use super::protocol::{
    PluginNotification, PluginResult, RpcError, RpcRequest, RpcResponse, PROTOCOL_VERSION,
};

/// 插件侧车进程启动/单次调用默认超时
const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(600);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// 宿主可提供给插件的反向 JSON-RPC 服务方法
pub const HOST_SERVICE_FRAMES_EXTRACT: &str = "frames.extract";

#[derive(Debug, Error)]
pub enum PluginProcessError {
    #[error("无法启动插件进程：{0}")]
    SpawnFailed(String),
    #[error("插件握手失败：{0}")]
    HandshakeFailed(String),
    #[error("插件协议不兼容：主程序 {host}，插件 {plugin}")]
    IncompatibleProtocol { host: String, plugin: String },
    #[error("插件进程已退出（stdout 关闭）")]
    ProcessExited,
    #[error("等待插件响应超时（{0:?}）")]
    Timeout(Duration),
    #[error("插件返回错误 [{code}] {message}")]
    RemoteError { code: i32, message: String },
    #[error("插件响应解析失败：{0}")]
    InvalidResponse(String),
    #[error("IO 错误：{0}")]
    Io(#[from] std::io::Error),
}

/// 插件 → 主程序的通知回调（progress / log）
pub type NotificationHandler = Arc<dyn Fn(PluginNotification) + Send + Sync>;

/// 插件 → 主程序的 JSON-RPC 请求处理回调（如 `frames.extract`）。
///
/// 仅在 `run` 调用期间生效（`run_with_services` 注入），
/// 返回 `(code, message)` 错误或 result 值。
pub type HostRequestHandler = Arc<dyn Fn(&RpcRequest) -> Result<serde_json::Value, (i32, String)> + Send + Sync>;

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// 一个运行中的插件 Sidecar 进程
pub struct PluginProcess {
    pub manifest: PluginManifest,
    pub plugin_dir: PathBuf,
    child: Child,
    stdin: ChildStdin,
    /// 后台读取线程产出的响应/请求/通知
    rx: Receiver<Incoming>,
    #[allow(dead_code)]
    reader_thread: thread::JoinHandle<()>,
    /// `run` 期间注入的宿主服务处理回调；`None` 时收到插件请求会回 `-32603`
    host_service_handler: Option<HostRequestHandler>,
}

impl PluginProcess {
    /// 构建启动命令：根据 manifest.runtime 与 entry 决定可执行文件与参数。
    ///
    /// - `binary`：entry 即二进制文件
    /// - `python`：`python <entry>`（解释器从 `python`/`python3` PATH 解析）
    /// - `node`：`node <entry>`
    /// - `rust`：按 binary 处理（Rust 插件编译为独立可执行文件）
    pub fn command(&self) -> (PathBuf, Vec<String>) {
        let entry = self.plugin_dir.join(&self.manifest.entry);
        match self.manifest.runtime {
            super::manifest::PluginRuntime::Python => (
                PathBuf::from(python_binary()),
                vec![entry.to_string_lossy().to_string()],
            ),
            super::manifest::PluginRuntime::Node => (
                PathBuf::from("node"),
                vec![entry.to_string_lossy().to_string()],
            ),
            super::manifest::PluginRuntime::Rust | super::manifest::PluginRuntime::Binary => {
                #[cfg(windows)]
                let entry = {
                    // Windows 上 entry 可能没有 .exe 后缀
                    if entry.extension().is_none() {
                        entry.with_extension("exe")
                    } else {
                        entry
                    }
                };
                (entry, Vec::new())
            }
        }
    }

    /// 启动插件进程并完成协议握手。
    pub fn spawn(
        manifest: PluginManifest,
        plugin_dir: &Path,
        on_notify: Option<NotificationHandler>,
    ) -> Result<Self, PluginProcessError> {
        let plugin_dir = plugin_dir.to_path_buf();
        let (program, args) = {
            let entry = plugin_dir.join(&manifest.entry);
            match manifest.runtime {
                super::manifest::PluginRuntime::Python => (
                    PathBuf::from(python_binary()),
                    vec![entry.to_string_lossy().to_string()],
                ),
                super::manifest::PluginRuntime::Node => (
                    PathBuf::from("node"),
                    vec![entry.to_string_lossy().to_string()],
                ),
                super::manifest::PluginRuntime::Rust | super::manifest::PluginRuntime::Binary => {
                    #[cfg(windows)]
                    let entry = if entry.extension().is_none() {
                        entry.with_extension("exe")
                    } else {
                        entry.clone()
                    };
                    (entry, Vec::new())
                }
            }
        };

        let mut child = Command::new(&program)
            .args(&args)
            .current_dir(&plugin_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| PluginProcessError::SpawnFailed(format!("{}: {e}", program.display())))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PluginProcessError::SpawnFailed("无法获取 stdin".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| PluginProcessError::SpawnFailed("无法获取 stdout".to_string()))?;
        let stderr = child.stderr.take();

        let (tx, rx) = mpsc::channel::<Incoming>();
        let reader_thread = spawn_reader(stdout, stderr, tx, on_notify);

        let mut process = Self {
            manifest,
            plugin_dir,
            child,
            stdin,
            rx,
            reader_thread,
            host_service_handler: None,
        };

        process.handshake()?;
        Ok(process)
    }

    /// 协议握手：发送 `initialize`，校验 protocolVersion 兼容性，
    /// 并声明宿主提供的反向服务（如 `frames.extract`）。
    ///
    /// 版本规则：major 相同即兼容；主程序 major 为 0 时要求完全一致。
    fn handshake(&mut self) -> Result<(), PluginProcessError> {
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "progressEvents": true,
                "cancellation": true,
                "hostServices": {
                    HOST_SERVICE_FRAMES_EXTRACT: "1.0.0",
                },
            }
        });
        let response = self.call_with_timeout("initialize", params, HANDSHAKE_TIMEOUT)?;
        let plugin_version = response
            .get("protocolVersion")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if !versions_compatible(PROTOCOL_VERSION, &plugin_version) {
            return Err(PluginProcessError::IncompatibleProtocol {
                host: PROTOCOL_VERSION.to_string(),
                plugin: plugin_version,
            });
        }
        Ok(())
    }

    /// 调用 `run` 方法并等待最终 `PluginResult`。
    /// 旧接口：不启用宿主反向服务；插件请求 `frames.extract` 会收到 `-32603`。
    pub fn run(
        &mut self,
        params: serde_json::Value,
        timeout: Option<Duration>,
    ) -> Result<PluginResult, PluginProcessError> {
        self.run_with_host_services(params, timeout, None)
    }

    /// 调用 `run` 并注册插件→宿主的反向 JSON-RPC 服务处理器。
    ///
    /// `handler` 只在本次 `run` 期间有效；收到 `frames.extract` 等请求时，
    /// 由 `handler` 返回结果或 `(code, message)` 错误；宿主内部错误统一包为
    /// `-32603`。
    pub fn run_with_host_services(
        &mut self,
        params: serde_json::Value,
        timeout: Option<Duration>,
        handler: Option<HostRequestHandler>,
    ) -> Result<PluginResult, PluginProcessError> {
        self.host_service_handler = handler;
        let result =
            self.call_with_timeout("run", params, timeout.unwrap_or(DEFAULT_CALL_TIMEOUT));
        self.host_service_handler = None;
        let result = result?;
        serde_json::from_value(result)
            .map_err(|e| PluginProcessError::InvalidResponse(format!("PluginResult 解析失败: {e}")))
    }

    /// 发送 `cancel` 请求（尽力而为，不阻塞等待）。
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

    /// 发送 `shutdown` 请求并等待进程退出；超时则强杀。
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

    /// 发送一个 JSON-RPC 请求并等待匹配 id 的响应。
    fn call_with_timeout(
        &mut self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, PluginProcessError> {
        let id = next_id();
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let mut line = serde_json::to_string(&req)
            .map_err(|e| PluginProcessError::InvalidResponse(e.to_string()))?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.flush()?;

        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.checked_duration_since(Instant::now());
            let Some(remaining) = remaining else {
                return Err(PluginProcessError::Timeout(timeout));
            };
            match self
                .rx
                .recv_timeout(remaining.min(Duration::from_millis(250)))
            {
                Ok(Incoming::Response(response)) if response.id == id => {
                    if let Some(error) = response.error {
                        return Err(PluginProcessError::RemoteError {
                            code: error.code,
                            message: error.message,
                        });
                    }
                    return Ok(response.result.unwrap_or(serde_json::Value::Null));
                }
                Ok(Incoming::Request(request)) => {
                    self.dispatch_host_request(&request);
                }
                Ok(_) => continue, // 迟到/其他 id 的响应：丢弃
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !self.is_alive() {
                        return Err(PluginProcessError::ProcessExited);
                    }
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(PluginProcessError::ProcessExited);
                }
            }
        }
    }

    /// 处理插件反向请求：由 `host_service_handler` 分发，
    /// 结果按 JSON-RPC 响应写回插件 stdin。
    ///
    /// 注意：只有 `run` 调用持有 `PluginProcess` 锁，`cancel`/`shutdown`
    /// 等并发调用会先等锁，因此此处写 stdin 不会与其他写入交错。
    fn dispatch_host_request(&mut self, request: &RpcRequest) {
        let response = match &self.host_service_handler {
            Some(handler) => match handler(request) {
                Ok(result) => RpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: Some(result),
                    error: None,
                },
                Err((code, message)) => RpcResponse {
                    jsonrpc: "2.0".to_string(),
                    id: request.id,
                    result: None,
                    error: Some(RpcError {
                        code,
                        message,
                        data: None,
                    }),
                },
            },
            None => RpcResponse {
                jsonrpc: "2.0".to_string(),
                id: request.id,
                result: None,
                error: Some(RpcError {
                    code: -32603,
                    message: "宿主服务不可用（当前不在 run 调用期间）".to_string(),
                    data: None,
                }),
            },
        };
        let _ = self.write_response(&response);
    }

    /// 序列化响应并写入插件 stdin（尽力而为）。
    fn write_response(&mut self, response: &RpcResponse) -> std::io::Result<()> {
        let mut line = serde_json::to_string(response)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.flush()
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for PluginProcess {
    fn drop(&mut self) {
        // 进程退出不得影响主程序；尽力而为
        self.shutdown(Duration::from_secs(2));
    }
}

/// stdout/stderr 读取线程：按行读取 stdout，解析为 `RpcResponse`；
/// `id == 0` 表示通知（progress/log/任意 method），交给 handler。
fn spawn_reader(
    stdout: ChildStdout,
    stderr: Option<std::process::ChildStderr>,
    tx: mpsc::Sender<Incoming>,
    on_notify: Option<NotificationHandler>,
) -> thread::JoinHandle<()> {
    // stderr 直接丢弃到独立 drain 线程，防止插件写满管道阻塞
    if let Some(stderr) = stderr {
        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for _line in reader.lines().map_while(Result::ok) {
                // TODO(M3): 汇聚到插件日志视图
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
                Ok(Incoming::Request(request)) => {
                    if tx.send(Incoming::Request(request)).is_err() {
                        break;
                    }
                }
                Ok(Incoming::Response(response)) => {
                    if tx.send(Incoming::Response(response)).is_err() {
                        break;
                    }
                }
                Ok(Incoming::Notification(notification)) => {
                    if let Some(handler) = &on_notify {
                        handler(notification);
                    }
                }
                Err(_) => {
                    // 非 JSON 行（print 输出等）：忽略，不杀死插件
                    continue;
                }
            }
        }
        // stdout 关闭：reader 线程结束；channel 随 tx drop 断开，
        // 等待方会收到 Disconnected → ProcessExited
    })
}

/// 入站消息：插件发起的请求（带 `id` + `method`）、
/// 插件对宿主请求的响应（带 `id` + `result`/`error`）、
/// 或无 `id` 的通知（progress / log）。
///
/// `untagged` 顺序很重要：`RpcResponse` 的所有字段都有 `Option` 默认值，
/// 会把插件请求误判成"空响应"，因此 `Request` 必须放在最前面。
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Incoming {
    Request(RpcRequest),
    Response(RpcResponse),
    Notification(PluginNotification),
}

/// Windows 下 `python` 优先，其次 `python3`；Unix 下 `python3` 优先。
fn python_binary() -> &'static str {
    if cfg!(windows) {
        "python"
    } else {
        "python3"
    }
}

/// 协议兼容性：major 相同即兼容；任一 major 为 0 时要求完全一致。
pub fn versions_compatible(host: &str, plugin: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_versions_compatible_when_major_matches() {
        assert!(versions_compatible("1.0.0", "1.2.3"));
        assert!(versions_compatible("2.0.0", "2.0.0"));
        assert!(!versions_compatible("1.0.0", "2.0.0"));
        assert!(!versions_compatible("2.0.0", "1.0.0"));
    }

    #[test]
    fn zero_major_requires_exact_match() {
        assert!(versions_compatible("0.1.0", "0.1.0"));
        assert!(!versions_compatible("0.1.0", "0.1.1"));
    }

    #[test]
    fn malformed_versions_are_incompatible() {
        assert!(!versions_compatible("1.0.0", ""));
        assert!(!versions_compatible("1.0.0", "abc"));
        assert!(!versions_compatible("", "1.0.0"));
    }

    #[test]
    fn incoming_parses_response_and_notification() {
        let resp: Incoming =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true},"error":null}"#)
                .expect("response");
        match resp {
            Incoming::Response(r) => assert_eq!(r.id, 7),
            _ => panic!("expected response"),
        }

        let notif: Incoming = serde_json::from_str(
            r#"{"jsonrpc":"2.0","method":"progress","params":{"progress":0.5}}"#,
        )
        .expect("notification");
        match notif {
            Incoming::Notification(n) => assert_eq!(n.method, "progress"),
            _ => panic!("expected notification"),
        }
    }

    #[test]
    fn incoming_parses_plugin_request_before_response() {
        // 插件请求带 id + method：untagged 必须优先匹配 Request，
        // 否则 RpcResponse 的全 Option 字段会把它错当成"空响应"。
        let req: Incoming = serde_json::from_str(
            r#"{"jsonrpc":"2.0","id":42,"method":"frames.extract","params":{"video_path":"/v.mp4"}}"#,
        )
        .expect("request");
        match req {
            Incoming::Request(r) => {
                assert_eq!(r.id, 42);
                assert_eq!(r.method, "frames.extract");
            }
            _ => panic!("expected request"),
        }
    }
}
