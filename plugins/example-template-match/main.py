"""TimePin 示例插件：演示 JSON-RPC over stdio 协议。

协议（JSON Lines，一行一条消息）：
- 请求  {"jsonrpc":"2.0","id":N,"method":"...","params":{...}}
- 响应  {"jsonrpc":"2.0","id":N,"result":{...}} 或 {"jsonrpc":"2.0","id":N,"error":{"code":..,"message":".."}}
- 通知  {"jsonrpc":"2.0","method":"progress"|"log","params":{...}}  （无 id，由插件主动发出）

方法：
- initialize  → 返回 {"protocolVersion": "1.0.0", "pluginName": ..., "capabilities": {...}}
- run         → params: {taskId, video, params, outputDir} → 返回 PluginResult
- cancel      → 设置取消标志，run 循环在下一次迭代退出并返回已产出的 marks
- shutdown    → 返回 {} 后进程退出

依赖：仅标准库（Python 3.8+），不需要 OpenCV —— M3 才引入真实视觉识别。
"""

from __future__ import annotations

import io
import json
import os
import sys
import uuid
from typing import Any, Dict, Optional

# Windows 控制台默认 GBK 编码：必须强制 UTF-8，否则中文日志产出乱码
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8", errors="replace")

PROTOCOL_VERSION = "1.0.0"
PLUGIN_NAME = "example-template-match"

_cancelled = False


def _write(message: Dict[str, Any]) -> None:
    """往 stdout 写一条 JSON Lines 消息并立即 flush。"""
    sys.stdout.write(json.dumps(message, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def respond(request_id: Any, result: Any = None, error: Optional[Dict[str, Any]] = None) -> None:
    msg: Dict[str, Any] = {"jsonrpc": "2.0", "id": request_id}
    if error is not None:
        msg["error"] = error
    else:
        msg["result"] = result
    _write(msg)


def notify(method: str, params: Dict[str, Any]) -> None:
    _write({"jsonrpc": "2.0", "method": method, "params": params})


def progress(task_id: str, value: float, message: Optional[str] = None, marks_so_far: int = 0) -> None:
    notify("progress", {
        "taskId": task_id,
        "progress": max(0.0, min(1.0, value)),
        "message": message,
        "marksSoFar": marks_so_far,
    })


def log(task_id: str, message: str, level: str = "info") -> None:
    notify("log", {"taskId": task_id, "level": level, "message": message})


def handle_initialize() -> Dict[str, Any]:
    return {
        "protocolVersion": PROTOCOL_VERSION,
        "pluginName": PLUGIN_NAME,
        "capabilities": {"progressEvents": True, "cancellation": True},
    }


def make_mark(start: float, label: str, confidence: float, source: str) -> Dict[str, Any]:
    return {
        "id": str(uuid.uuid4()),
        "startTime": round(start, 3),
        "endTime": None,
        "label": label,
        "color": None,
        "confidence": confidence,
        "sourcePlugin": source,
        "thumbnailPath": None,
        "metadata": {},
    }


def handle_run(params: Dict[str, Any]) -> Dict[str, Any]:
    global _cancelled
    _cancelled = False

    task_id = params["taskId"]
    video = params["video"]
    user_params = params.get("params") or {}
    output_dir = params["outputDir"]

    interval = float(user_params.get("intervalSeconds", 30))
    label_prefix = str(user_params.get("label", "示例标注"))
    duration = float(video.get("duration", 0) or 0)

    logs: list[str] = []
    marks: list[Dict[str, Any]] = []

    log(task_id, f"开始处理 {video.get('path')}（时长 {duration:.1f}s）")
    logs.append(f"run started: video={video.get('path')} duration={duration}")

    # 写证据文件：回显运行参数（只允许写 outputDir 内，铁律 #5）
    os.makedirs(output_dir, exist_ok=True)
    evidence_path = os.path.join(output_dir, "params.json")
    with open(evidence_path, "w", encoding="utf-8") as f:
        json.dump({"taskId": task_id, "video": video, "params": user_params}, f,
                  ensure_ascii=False, indent=2)

    if duration <= 0:
        log(task_id, "视频时长未知，生成单个演示标注", "warn")
        marks.append(make_mark(0.0, f"{label_prefix} 1", 0.5, PLUGIN_NAME))
    else:
        count = max(1, int(duration // interval))
        for i in range(count):
            if _cancelled:
                log(task_id, "收到取消请求，返回已产出标注", "warn")
                logs.append("cancelled")
                break
            t = (i + 1) * interval
            if t >= duration:
                t = duration - 0.001
            marks.append(make_mark(t, f"{label_prefix} {i + 1}", round(0.5 + 0.4 * (i / max(1, count)), 2), PLUGIN_NAME))
            progress(task_id, (i + 1) / count, f"已生成 {i + 1}/{count} 个标注", len(marks))

    progress(task_id, 1.0, "完成", len(marks))
    logs.append(f"run finished: {len(marks)} marks")

    return {
        "protocolVersion": PROTOCOL_VERSION,
        "pluginName": PLUGIN_NAME,
        "marks": marks,
        "logs": logs,
        "evidencePaths": [evidence_path],
    }


def main() -> None:
    global _cancelled
    stdin = io.TextIOWrapper(sys.stdin.buffer, encoding="utf-8", errors="replace")
    for line in stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            continue  # 非 JSON 行忽略

        method = request.get("method")
        request_id = request.get("id")
        params = request.get("params") or {}

        # 通知（无 id）目前没有需要处理的；全部按请求处理
        if method == "initialize":
            respond(request_id, handle_initialize())
        elif method == "run":
            try:
                respond(request_id, handle_run(params))
            except Exception as exc:  # noqa: BLE001 —— 插件自身崩溃不能影响主程序，错误走 error 字段
                respond(request_id, error={"code": -32000, "message": f"{type(exc).__name__}: {exc}"})
        elif method == "cancel":
            _cancelled = True
            respond(request_id, {"cancelled": True})
        elif method == "shutdown":
            respond(request_id, {})
            return
        else:
            respond(request_id, error={"code": -32601, "message": f"未知方法: {method}"})


if __name__ == "__main__":
    main()
