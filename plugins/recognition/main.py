"""TimePin 识别 Sidecar 入口：JSON-RPC over stdio。

协议同插件系统：
- 请求  {"jsonrpc":"2.0","id":N,"method":"...","params":{...}}
- 响应  {"jsonrpc":"2.0","id":N,"result":{...}} 或 {"jsonrpc":"2.0","id":N,"error":{"code":..,"message":".."}}
- 通知  {"jsonrpc":"2.0","method":"progress"|"log","params":{...}}  （无 id）

方法：
- initialize  → 返回版本与能力
- recognize.template_match  → 模板匹配
- recognize.ocr  → OCR 识别
- recognize.detect  → 目标检测
- recognize.preview  → 调试预览帧
- recognize.roi.save / roi.load / roi.list  → ROI 管理
- shutdown    → 返回 {} 后进程退出
"""

from __future__ import annotations

import io
import json
import os
import sys
import traceback
import uuid
from typing import Any, Dict, List, Optional

import cv2
import numpy as np

# Windows 控制台默认 GBK 编码：必须强制 UTF-8
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8", errors="replace")

from recognition import (
    PROTOCOL_VERSION,
    SERVICE_NAME,
    Rect,
    TemplateMatchResult,
    OcrResult,
    DetectionResult,
    PreviewFrame,
    TemplateMatcher,
    OcrEngine,
    ObjectDetector,
    RoiManager,
    extract_frame,
    extract_frames_range,
    encode_image_base64,
    draw_overlays,
)

SERVICE_VERSION = "0.1.0"

# 全局识别器实例
_template_matcher = TemplateMatcher()
_ocr_engine = OcrEngine()
_object_detector = ObjectDetector()
_roi_manager = None  # 延迟初始化
_cancelled = False


def _write(message: Dict[str, Any]) -> None:
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


def progress(task_id: str, value: float, message: Optional[str] = None) -> None:
    notify("progress", {"taskId": task_id, "progress": max(0.0, min(1.0, value)), "message": message})


def log(task_id: str, message: str, level: str = "info") -> None:
    notify("log", {"taskId": task_id, "level": level, "message": message})


# ==================== 方法处理器 ====================

def handle_initialize() -> Dict[str, Any]:
    return {
        "protocolVersion": PROTOCOL_VERSION,
        "serviceName": SERVICE_NAME,
        "serviceVersion": SERVICE_VERSION,
        "capabilities": {
            "templateMatch": True,
            "ocr": True,
            "objectDetection": True,
            "preview": True,
            "roiManagement": True,
            "progressEvents": True,
            "cancellation": True,
        },
    }


def handle_template_match(params: Dict[str, Any]) -> Dict[str, Any]:
    global _cancelled
    _cancelled = False

    task_id = params.get("taskId", str(uuid.uuid4()))
    video_path = params.get("videoPath")
    template_name = params.get("templateName")
    threshold = float(params.get("threshold", 0.8))
    scales = params.get("scales")
    roi_data = params.get("roi")
    frame_timestamps = params.get("frameTimestamps")  # 可选：指定帧时间戳列表
    max_frames = int(params.get("maxFrames", 50))

    if not video_path or not os.path.exists(video_path):
        return {"error": f"视频文件不存在: {video_path}"}
    if not template_name:
        return {"error": "templateName 不能为空"}

    roi = Rect(**roi_data) if roi_data else None

    log(task_id, f"开始模板匹配: {template_name} @ {video_path}")

    # 如果未指定帧，均匀采样
    if frame_timestamps:
        timestamps = frame_timestamps
    else:
        cap = cv2.VideoCapture(video_path)
        fps = cap.get(cv2.CAP_PROP_FPS)
        total_frames = int(cap.get(cv2.CAP_PROP_FRAME_COUNT))
        cap.release()
        duration = total_frames / fps if fps > 0 else 0
        timestamps = np.linspace(0, duration, min(max_frames, max(1, int(duration * 2)))).tolist()

    all_matches: List[TemplateMatchResult] = []

    for i, ts in enumerate(timestamps):
        if _cancelled:
            log(task_id, "收到取消请求", "warn")
            break

        frame = extract_frame(video_path, ts)
        if frame is None:
            continue

        matches = _template_matcher.match(frame, template_name, threshold, scales, roi)
        for m in matches:
            # 添加帧时间戳信息
            m_dict = {
                "rect": {"x": m.rect.x, "y": m.rect.y, "width": m.rect.width, "height": m.rect.height},
                "confidence": m.confidence,
                "scale": m.scale,
                "templateName": m.template_name,
                "timestamp": round(ts, 3),
            }
            all_matches.append(m_dict)

        progress(task_id, (i + 1) / len(timestamps), f"已处理 {i + 1}/{len(timestamps)} 帧")

    log(task_id, f"模板匹配完成，共找到 {len(all_matches)} 个匹配")
    return {"matches": all_matches}


def handle_ocr(params: Dict[str, Any]) -> Dict[str, Any]:
    global _cancelled
    _cancelled = False

    task_id = params.get("taskId", str(uuid.uuid4()))
    video_path = params.get("videoPath")
    roi_data = params.get("roi")
    frame_timestamps = params.get("frameTimestamps")
    max_frames = int(params.get("maxFrames", 30))
    lang = params.get("lang")

    if not video_path or not os.path.exists(video_path):
        return {"error": f"视频文件不存在: {video_path}"}

    roi = Rect(**roi_data) if roi_data else None

    log(task_id, f"开始 OCR 识别: {video_path}")

    if frame_timestamps:
        timestamps = frame_timestamps
    else:
        cap = cv2.VideoCapture(video_path)
        fps = cap.get(cv2.CAP_PROP_FPS)
        total_frames = int(cap.get(cv2.CAP_PROP_FRAME_COUNT))
        cap.release()
        duration = total_frames / fps if fps > 0 else 0
        timestamps = np.linspace(0, duration, min(max_frames, max(1, int(duration)))).tolist()

    all_results: List[Dict[str, Any]] = []

    for i, ts in enumerate(timestamps):
        if _cancelled:
            log(task_id, "收到取消请求", "warn")
            break

        frame = extract_frame(video_path, ts)
        if frame is None:
            continue

        results = _ocr_engine.recognize(frame, roi, lang)
        for r in results:
            all_results.append({
                "rect": {"x": r.rect.x, "y": r.rect.y, "width": r.rect.width, "height": r.rect.height},
                "text": r.text,
                "confidence": r.confidence,
                "timestamp": round(ts, 3),
            })

        progress(task_id, (i + 1) / len(timestamps), f"已处理 {i + 1}/{len(timestamps)} 帧")

    log(task_id, f"OCR 完成，共识别 {len(all_results)} 个文本区域")
    return {"results": all_results}


def handle_detect(params: Dict[str, Any]) -> Dict[str, Any]:
    global _cancelled
    _cancelled = False

    task_id = params.get("taskId", str(uuid.uuid4()))
    video_path = params.get("videoPath")
    roi_data = params.get("roi")
    frame_timestamps = params.get("frameTimestamps")
    max_frames = int(params.get("maxFrames", 30))

    if not video_path or not os.path.exists(video_path):
        return {"error": f"视频文件不存在: {video_path}"}

    roi = Rect(**roi_data) if roi_data else None

    log(task_id, f"开始目标检测: {video_path}")

    if frame_timestamps:
        timestamps = frame_timestamps
    else:
        cap = cv2.VideoCapture(video_path)
        fps = cap.get(cv2.CAP_PROP_FPS)
        total_frames = int(cap.get(cv2.CAP_PROP_FRAME_COUNT))
        cap.release()
        duration = total_frames / fps if fps > 0 else 0
        timestamps = np.linspace(0, duration, min(max_frames, max(1, int(duration)))).tolist()

    all_results: List[Dict[str, Any]] = []

    for i, ts in enumerate(timestamps):
        if _cancelled:
            log(task_id, "收到取消请求", "warn")
            break

        frame = extract_frame(video_path, ts)
        if frame is None:
            continue

        results = _object_detector.detect(frame, roi)
        for r in results:
            all_results.append({
                "rect": {"x": r.rect.x, "y": r.rect.y, "width": r.rect.width, "height": r.rect.height},
                "label": r.label,
                "confidence": r.confidence,
                "classId": r.class_id,
                "timestamp": round(ts, 3),
            })

        progress(task_id, (i + 1) / len(timestamps), f"已处理 {i + 1}/{len(timestamps)} 帧")

    log(task_id, f"目标检测完成，共检测 {len(all_results)} 个目标")
    return {"results": all_results}


def handle_preview(params: Dict[str, Any]) -> Dict[str, Any]:
    """生成带叠加层的预览帧"""
    task_id = params.get("taskId", str(uuid.uuid4()))
    video_path = params.get("videoPath")
    timestamp = float(params.get("timestamp", 0))
    template_matches = params.get("templateMatches", [])
    ocr_results = params.get("ocrResults", [])
    detections = params.get("detections", [])
    rois = params.get("rois", [])
    quality = int(params.get("quality", 80))

    if not video_path or not os.path.exists(video_path):
        return {"error": f"视频文件不存在: {video_path}"}

    frame = extract_frame(video_path, timestamp)
    if frame is None:
        return {"error": f"无法提取帧: {timestamp}s"}

    # 转换为识别器对象
    tm_objs = [TemplateMatchResult(
        rect=Rect(**m["rect"]),
        confidence=m["confidence"],
        scale=m.get("scale", 1.0),
        template_name=m.get("templateName", ""),
    ) for m in template_matches]

    ocr_objs = [OcrResult(
        rect=Rect(**r["rect"]),
        text=r["text"],
        confidence=r["confidence"],
    ) for r in ocr_results]

    det_objs = [DetectionResult(
        rect=Rect(**d["rect"]),
        label=d["label"],
        confidence=d["confidence"],
        class_id=d.get("classId", 0),
    ) for d in detections]

    roi_objs = [Rect(**r) for r in rois]

    vis = draw_overlays(frame, tm_objs, ocr_objs, det_objs, roi_objs)
    image_b64 = encode_image_base64(vis, quality)

    return {
        "frameIndex": -1,
        "timestamp": round(timestamp, 3),
        "imageBase64": image_b64,
        "width": frame.shape[1],
        "height": frame.shape[0],
    }


def handle_roi_save(params: Dict[str, Any]) -> Dict[str, Any]:
    global _roi_manager
    if _roi_manager is None:
        _roi_manager = RoiManager(os.path.join(os.path.dirname(__file__), "rois"))

    name = params.get("name")
    rect_data = params.get("rect")
    metadata = params.get("metadata")

    if not name or not rect_data:
        return {"error": "name 和 rect 不能为空"}

    roi = Rect(**rect_data)
    path = _roi_manager.save_roi(name, roi, metadata)
    return {"path": path}


def handle_roi_load(params: Dict[str, Any]) -> Dict[str, Any]:
    global _roi_manager
    if _roi_manager is None:
        _roi_manager = RoiManager(os.path.join(os.path.dirname(__file__), "rois"))

    name = params.get("name")
    if not name:
        return {"error": "name 不能为空"}

    roi = _roi_manager.load_roi(name)
    if roi is None:
        return {"error": f"ROI 不存在: {name}"}

    return {"rect": {"x": roi.x, "y": roi.y, "width": roi.width, "height": roi.height}}


def handle_roi_list(params: Dict[str, Any]) -> Dict[str, Any]:
    global _roi_manager
    if _roi_manager is None:
        _roi_manager = RoiManager(os.path.join(os.path.dirname(__file__), "rois"))

    return {"rois": _roi_manager.list_rois()}


def handle_configure(params: Dict[str, Any]) -> Dict[str, Any]:
    """配置识别器"""
    global _ocr_engine, _object_detector

    ocr_config = params.get("ocr")
    if ocr_config:
        _ocr_engine.initialize(ocr_config)

    detector_config = params.get("detector")
    if detector_config:
        _object_detector.initialize(detector_config)

    template_config = params.get("templates")
    if template_config:
        for name, cfg in template_config.items():
            img_path = cfg.get("imagePath")
            mask_path = cfg.get("maskPath")
            if img_path:
                _template_matcher.load_template(name, img_path, mask_path)

    return {"ok": True}


# ==================== 主循环 ====================

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
            continue

        method = request.get("method")
        request_id = request.get("id")
        params = request.get("params") or {}

        if method == "initialize":
            respond(request_id, handle_initialize())
        elif method == "recognize.template_match":
            try:
                respond(request_id, handle_template_match(params))
            except Exception as exc:
                respond(request_id, error={"code": -32000, "message": f"{type(exc).__name__}: {exc}"})
        elif method == "recognize.ocr":
            try:
                respond(request_id, handle_ocr(params))
            except Exception as exc:
                respond(request_id, error={"code": -32000, "message": f"{type(exc).__name__}: {exc}"})
        elif method == "recognize.detect":
            try:
                respond(request_id, handle_detect(params))
            except Exception as exc:
                respond(request_id, error={"code": -32000, "message": f"{type(exc).__name__}: {exc}"})
        elif method == "recognize.preview":
            try:
                respond(request_id, handle_preview(params))
            except Exception as exc:
                respond(request_id, error={"code": -32000, "message": f"{type(exc).__name__}: {exc}"})
        elif method == "recognize.roi.save":
            try:
                respond(request_id, handle_roi_save(params))
            except Exception as exc:
                respond(request_id, error={"code": -32000, "message": f"{type(exc).__name__}: {exc}"})
        elif method == "recognize.roi.load":
            try:
                respond(request_id, handle_roi_load(params))
            except Exception as exc:
                respond(request_id, error={"code": -32000, "message": f"{type(exc).__name__}: {exc}"})
        elif method == "recognize.roi.list":
            try:
                respond(request_id, handle_roi_list(params))
            except Exception as exc:
                respond(request_id, error={"code": -32000, "message": f"{type(exc).__name__}: {exc}"})
        elif method == "recognize.configure":
            try:
                respond(request_id, handle_configure(params))
            except Exception as exc:
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