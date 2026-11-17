"""TimePin 识别 Sidecar 核心模块。

提供给插件调用的识别原语：
- 模板匹配：多尺度、掩码、阈值
- OCR：PaddleOCR / Tesseract 封装
- 目标检测：ONNX Runtime (YOLO 等)
- ROI 管理：保存/加载、坐标归一化
- 调试预览：逐帧叠加识别结果、导出证据图

协议：JSON-RPC over stdio（同插件协议）
"""

from __future__ import annotations

import base64
import io
import json
import os
import sys
import uuid
from dataclasses import dataclass, asdict
from typing import Any, Dict, List, Optional, Tuple

import cv2
import numpy as np

# Windows 控制台默认 GBK 编码：必须强制 UTF-8
sys.stdout = io.TextIOWrapper(sys.stdout.buffer, encoding="utf-8", errors="replace")
sys.stderr = io.TextIOWrapper(sys.stderr.buffer, encoding="utf-8", errors="replace")

PROTOCOL_VERSION = "1.0.0"
SERVICE_NAME = "timepin-recognition"


# ==================== 数据结构 ====================

@dataclass
class Rect:
    """归一化矩形 [0,1]"""
    x: float
    y: float
    width: float
    height: float

    def to_pixels(self, img_w: int, img_h: int) -> Tuple[int, int, int, int]:
        return (
            int(self.x * img_w),
            int(self.y * img_h),
            int(self.width * img_w),
            int(self.height * img_h),
        )

    @classmethod
    def from_pixels(cls, x: int, y: int, w: int, h: int, img_w: int, img_h: int) -> "Rect":
        return cls(x / img_w, y / img_h, w / img_w, h / img_h)


@dataclass
class TemplateMatchResult:
    """模板匹配单个结果"""
    rect: Rect
    confidence: float
    scale: float
    template_name: str


@dataclass
class OcrResult:
    """OCR 单行结果"""
    rect: Rect
    text: str
    confidence: float


@dataclass
class DetectionResult:
    """目标检测单个结果"""
    rect: Rect
    label: str
    confidence: float
    class_id: int


@dataclass
class PreviewFrame:
    """预览帧数据"""
    frame_index: int
    timestamp: float
    image_base64: str  # JPEG base64
    overlays: List[Dict[str, Any]]  # 叠加层：rects, texts, points


# ==================== 识别器基类 ====================

class Recognizer:
    """识别器基类"""

    def initialize(self, config: Dict[str, Any]) -> None:
        pass

    def shutdown(self) -> None:
        pass


# ==================== 模板匹配 ====================

class TemplateMatcher(Recognizer):
    """多尺度模板匹配器"""

    def __init__(self):
        self.templates: Dict[str, np.ndarray] = {}
        self.template_masks: Dict[str, np.ndarray] = {}

    def load_template(self, name: str, image_path: str, mask_path: Optional[str] = None) -> bool:
        """加载模板图片（支持透明通道作为掩码）"""
        img = cv2.imread(image_path, cv2.IMREAD_UNCHANGED)
        if img is None:
            return False

        if img.shape[2] == 4:
            # 有 Alpha 通道，分离
            self.templates[name] = img[:, :, :3]
            self.template_masks[name] = img[:, :, 3]
        else:
            self.templates[name] = img
            self.template_masks[name] = None

        if mask_path:
            mask = cv2.imread(mask_path, cv2.IMREAD_GRAYSCALE)
            if mask is not None:
                self.template_masks[name] = mask

        return True

    def match(
        self,
        frame: np.ndarray,
        template_name: str,
        threshold: float = 0.8,
        scales: Optional[List[float]] = None,
        roi: Optional[Rect] = None,
    ) -> List[TemplateMatchResult]:
        """在帧中查找模板"""
        if template_name not in self.templates:
            return []

        template = self.templates[template_name]
        mask = self.template_masks.get(template_name)

        # ROI 裁剪
        search_frame = frame
        roi_offset = (0, 0)
        if roi:
            h, w = frame.shape[:2]
            x, y, rw, rh = roi.to_pixels(w, h)
            x, y = max(0, x), max(0, y)
            rw, rh = min(w - x, rw), min(h - y, rh)
            if rw <= 0 or rh <= 0:
                return []
            search_frame = frame[y:y+rh, x:x+rw]
            roi_offset = (x, y)

        h, w = search_frame.shape[:2]
        th, tw = template.shape[:2]

        if scales is None:
            scales = [1.0]

        results = []

        for scale in scales:
            if scale <= 0:
                continue
            sw, sh = int(tw * scale), int(th * scale)
            if sw >= w or sh >= h:
                continue

            scaled_template = cv2.resize(template, (sw, sh), interpolation=cv2.INTER_AREA)
            scaled_mask = None
            if mask is not None:
                scaled_mask = cv2.resize(mask, (sw, sh), interpolation=cv2.INTER_NEAREST)

            # 模板匹配
            method = cv2.TM_CCOEFF_NORMED
            result = cv2.matchTemplate(search_frame, scaled_template, method, mask=scaled_mask)

            # 阈值筛选
            locations = np.where(result >= threshold)
            for pt_y, pt_x in zip(*locations):
                confidence = float(result[pt_y, pt_x])
                rect = Rect.from_pixels(
                    pt_x + roi_offset[0], pt_y + roi_offset[1], sw, sh, w, h
                )
                results.append(TemplateMatchResult(
                    rect=rect,
                    confidence=confidence,
                    scale=scale,
                    template_name=template_name,
                ))

        # 非极大值抑制（简单去重）
        return self._nms(results, iou_threshold=0.3)

    def _nms(self, results: List[TemplateMatchResult], iou_threshold: float) -> List[TemplateMatchResult]:
        if not results:
            return []

        # 按置信度降序
        results.sort(key=lambda r: r.confidence, reverse=True)
        kept = []

        for r in results:
            overlap = False
            for k in kept:
                if self._iou(r.rect, k.rect) > iou_threshold:
                    overlap = True
                    break
            if not overlap:
                kept.append(r)

        return kept

    def _iou(self, a: Rect, b: Rect) -> float:
        ax1, ay1 = a.x, a.y
        ax2, ay2 = a.x + a.width, a.y + a.height
        bx1, by1 = b.x, b.y
        bx2, by2 = b.x + b.width, b.y + b.height

        ix1, iy1 = max(ax1, bx1), max(ay1, by1)
        ix2, iy2 = min(ax2, bx2), min(ay2, by2)

        if ix2 <= ix1 or iy2 <= iy1:
            return 0.0

        inter = (ix2 - ix1) * (iy2 - iy1)
        area_a = a.width * a.height
        area_b = b.width * b.height
        return inter / (area_a + area_b - inter)


# ==================== OCR ====================

class OcrEngine(Recognizer):
    """OCR 引擎封装（优先 PaddleOCR，回退 Tesseract）"""

    def __init__(self):
        self.engine = None
        self.engine_type = "none"

    def initialize(self, config: Dict[str, Any]) -> None:
        # 尝试 PaddleOCR
        try:
            from paddleocr import PaddleOCR
            self.engine = PaddleOCR(
                use_angle_cls=config.get("use_angle_cls", True),
                lang=config.get("lang", "ch"),
                use_gpu=config.get("use_gpu", False),
                show_log=False,
            )
            self.engine_type = "paddleocr"
            return
        except ImportError:
            pass

        # 回退 Tesseract
        try:
            import pytesseract
            self.engine = pytesseract
            self.engine_type = "tesseract"
            return
        except ImportError:
            pass

        self.engine = None
        self.engine_type = "none"

    def recognize(
        self,
        frame: np.ndarray,
        roi: Optional[Rect] = None,
        lang: Optional[str] = None,
    ) -> List[OcrResult]:
        if self.engine is None:
            return []

        search_frame = frame
        roi_offset = (0, 0)
        if roi:
            h, w = frame.shape[:2]
            x, y, rw, rh = roi.to_pixels(w, h)
            x, y = max(0, x), max(0, y)
            rw, rh = min(w - x, rw), min(h - y, rh)
            if rw <= 0 or rh <= 0:
                return []
            search_frame = frame[y:y+rh, x:x+rw]
            roi_offset = (x, y)

        results = []

        if self.engine_type == "paddleocr":
            ocr_result = self.engine.ocr(search_frame, cls=True)
            if ocr_result and ocr_result[0]:
                for line in ocr_result[0]:
                    box = line[0]  # [[x1,y1], [x2,y2], [x3,y3], [x4,y4]]
                    text = line[1][0]
                    confidence = line[1][1]

                    # 计算外接矩形
                    xs = [p[0] for p in box]
                    ys = [p[1] for p in box]
                    rx, ry = min(xs), min(ys)
                    rw, rh = max(xs) - min(xs), max(ys) - min(ys)

                    rect = Rect.from_pixels(
                        rx + roi_offset[0], ry + roi_offset[1], rw, rh,
                        search_frame.shape[1], search_frame.shape[0]
                    )
                    results.append(OcrResult(rect=rect, text=text, confidence=confidence))

        elif self.engine_type == "tesseract":
            import pytesseract
            data = pytesseract.image_to_data(
                search_frame,
                lang=lang or "chi_sim+eng",
                output_type=pytesseract.Output.DICT
            )
            for i in range(len(data["text"])):
                text = data["text"][i].strip()
                if not text:
                    continue
                conf = float(data["conf"][i]) / 100.0
                x, y, w, h = data["left"][i], data["top"][i], data["width"][i], data["height"][i]
                rect = Rect.from_pixels(
                    x + roi_offset[0], y + roi_offset[1], w, h,
                    search_frame.shape[1], search_frame.shape[0]
                )
                results.append(OcrResult(rect=rect, text=text, confidence=conf))

        return results


# ==================== 目标检测 ====================

class ObjectDetector(Recognizer):
    """ONNX Runtime 目标检测（YOLO 系列）"""

    def __init__(self):
        self.session = None
        self.input_name = None
        self.output_names = None
        self.class_names: List[str] = []
        self.input_size = (640, 640)
        self.conf_threshold = 0.25
        self.iou_threshold = 0.45

    def initialize(self, config: Dict[str, Any]) -> None:
        model_path = config.get("model_path")
        if not model_path or not os.path.exists(model_path):
            return

        import onnxruntime as ort
        providers = ["CUDAExecutionProvider", "CPUExecutionProvider"] if config.get("use_gpu") else ["CPUExecutionProvider"]
        self.session = ort.InferenceSession(model_path, providers=providers)

        self.input_name = self.session.get_inputs()[0].name
        self.output_names = [o.name for o in self.session.get_outputs()]

        # 解析输入尺寸
        input_shape = self.session.get_inputs()[0].shape
        if len(input_shape) == 4:
            self.input_size = (input_shape[2], input_shape[3])  # H, W

        self.class_names = config.get("class_names", [])
        self.conf_threshold = config.get("conf_threshold", 0.25)
        self.iou_threshold = config.get("iou_threshold", 0.45)

    def detect(
        self,
        frame: np.ndarray,
        roi: Optional[Rect] = None,
    ) -> List[DetectionResult]:
        if self.session is None:
            return []

        search_frame = frame
        roi_offset = (0, 0)
        if roi:
            h, w = frame.shape[:2]
            x, y, rw, rh = roi.to_pixels(w, h)
            x, y = max(0, x), max(0, y)
            rw, rh = min(w - x, rw), min(h - y, rh)
            if rw <= 0 or rh <= 0:
                return []
            search_frame = frame[y:y+rh, x:x+rw]
            roi_offset = (x, y)

        # 预处理
        input_h, input_w = self.input_size
        resized = cv2.resize(search_frame, (input_w, input_h))
        rgb = cv2.cvtColor(resized, cv2.COLOR_BGR2RGB)
        normalized = rgb.astype(np.float32) / 255.0
        input_tensor = np.transpose(normalized, (2, 0, 1))[np.newaxis, ...]  # 1,3,H,W

        # 推理
        outputs = self.session.run(self.output_names, {self.input_name: input_tensor})
        predictions = outputs[0]  # 假设输出格式为 (1, N, 85) 或类似

        # 后处理（简化版，针对 YOLOv5/v8 格式）
        results = self._postprocess(predictions, search_frame.shape[1], search_frame.shape[0], roi_offset)
        return results

    def _postprocess(
        self,
        predictions: np.ndarray,
        orig_w: int,
        orig_h: int,
        roi_offset: Tuple[int, int],
    ) -> List[DetectionResult]:
        results = []

        # 兼容多种输出格式
        if predictions.ndim == 3:
            predictions = predictions[0]  # (N, 85)

        if predictions.ndim != 2:
            return results

        for pred in predictions:
            if len(pred) < 6:
                continue

            cx, cy, w, h = pred[0], pred[1], pred[2], pred[3]
            obj_conf = pred[4]

            if obj_conf < self.conf_threshold:
                continue

            # 类别概率
            class_probs = pred[5:]
            if len(class_probs) == 0:
                continue

            class_id = int(np.argmax(class_probs))
            class_conf = float(class_probs[class_id])
            confidence = float(obj_conf * class_conf)

            if confidence < self.conf_threshold:
                continue

            # 转换为原图坐标
            scale_x = orig_w / self.input_size[1]
            scale_y = orig_h / self.input_size[0]

            x1 = (cx - w / 2) * scale_x + roi_offset[0]
            y1 = (cy - h / 2) * scale_y + roi_offset[1]
            x2 = (cx + w / 2) * scale_x + roi_offset[0]
            y2 = (cy + h / 2) * scale_y + roi_offset[1]

            # 归一化
            rect = Rect(
                x=max(0, x1) / orig_w,
                y=max(0, y1) / orig_h,
                width=max(0, min(1, (x2 - x1) / orig_w)),
                height=max(0, min(1, (y2 - y1) / orig_h)),
            )

            label = self.class_names[class_id] if class_id < len(self.class_names) else f"class_{class_id}"

            results.append(DetectionResult(
                rect=rect,
                label=label,
                confidence=confidence,
                class_id=class_id,
            ))

        # NMS
        return self._nms(results)

    def _nms(self, results: List[DetectionResult], iou_threshold: Optional[float] = None) -> List[DetectionResult]:
        if not results:
            return []

        thresh = iou_threshold or self.iou_threshold
        results.sort(key=lambda r: r.confidence, reverse=True)
        kept = []

        for r in results:
            overlap = False
            for k in kept:
                if self._iou(r.rect, k.rect) > thresh:
                    overlap = True
                    break
            if not overlap:
                kept.append(r)

        return kept

    def _iou(self, a: Rect, b: Rect) -> float:
        ax1, ay1 = a.x, a.y
        ax2, ay2 = a.x + a.width, a.y + a.height
        bx1, by1 = b.x, b.y
        bx2, by2 = b.x + b.width, b.y + b.height

        ix1, iy1 = max(ax1, bx1), max(ay1, by1)
        ix2, iy2 = min(ax2, bx2), min(ay2, by2)

        if ix2 <= ix1 or iy2 <= iy1:
            return 0.0

        inter = (ix2 - ix1) * (iy2 - iy1)
        area_a = a.width * a.height
        area_b = b.width * b.height
        return inter / (area_a + area_b - inter)


# ==================== ROI 管理 ====================

class RoiManager:
    """ROI 区域管理"""

    def __init__(self, storage_dir: str):
        self.storage_dir = storage_dir
        os.makedirs(storage_dir, exist_ok=True)

    def save_roi(self, name: str, roi: Rect, metadata: Optional[Dict] = None) -> str:
        path = os.path.join(self.storage_dir, f"{name}.json")
        data = {
            "name": name,
            "rect": asdict(roi),
            "metadata": metadata or {},
        }
        with open(path, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False, indent=2)
        return path

    def load_roi(self, name: str) -> Optional[Rect]:
        path = os.path.join(self.storage_dir, f"{name}.json")
        if not os.path.exists(path):
            return None
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
        return Rect(**data["rect"])

    def list_rois(self) -> List[str]:
        return [f[:-5] for f in os.listdir(self.storage_dir) if f.endswith(".json")]


# ==================== 视频帧提取 ====================

def extract_frame(video_path: str, timestamp: float) -> Optional[np.ndarray]:
    """从视频指定时间戳提取帧"""
    cap = cv2.VideoCapture(video_path)
    if not cap.isOpened():
        return None

    fps = cap.get(cv2.CAP_PROP_FPS)
    frame_index = int(timestamp * fps)
    cap.set(cv2.CAP_PROP_POS_FRAMES, frame_index)
    ret, frame = cap.read()
    cap.release()

    if not ret:
        return None
    return frame


def extract_frames_range(video_path: str, start: float, end: float, max_frames: int = 100) -> List[Tuple[float, np.ndarray]]:
    """提取时间范围内的帧（均匀采样）"""
    cap = cv2.VideoCapture(video_path)
    if not cap.isOpened():
        return []

    fps = cap.get(cv2.CAP_PROP_FPS)
    total_frames = int(cap.get(cv2.CAP_PROP_FRAME_COUNT))
    duration = total_frames / fps if fps > 0 else 0

    start = max(0, min(start, duration))
    end = max(start, min(end, duration))

    frame_count = min(max_frames, int((end - start) * fps))
    if frame_count <= 0:
        cap.release()
        return []

    indices = np.linspace(start * fps, end * fps, frame_count, dtype=int)
    frames = []

    for idx in indices:
        cap.set(cv2.CAP_PROP_POS_FRAMES, idx)
        ret, frame = cap.read()
        if ret:
            timestamp = idx / fps
            frames.append((timestamp, frame))

    cap.release()
    return frames


# ==================== 图像编码工具 ====================

def encode_image_base64(image: np.ndarray, quality: int = 80) -> str:
    """将图像编码为 JPEG base64"""
    _, buffer = cv2.imencode(".jpg", image, [cv2.IMWRITE_JPEG_QUALITY, quality])
    return base64.b64encode(buffer).decode("ascii")


def draw_overlays(
    image: np.ndarray,
    template_matches: List[TemplateMatchResult] = None,
    ocr_results: List[OcrResult] = None,
    detections: List[DetectionResult] = None,
    rois: List[Rect] = None,
) -> np.ndarray:
    """在图像上绘制识别结果叠加层"""
    vis = image.copy()
    h, w = vis.shape[:2]

    # ROI（虚线框）
    if rois:
        for roi in rois:
            x, y, rw, rh = roi.to_pixels(w, h)
            cv2.rectangle(vis, (x, y), (x + rw, y + rh), (255, 255, 0), 2, cv2.LINE_8, 0)
            cv2.putText(vis, "ROI", (x, y - 5), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (255, 255, 0), 1)

    # 模板匹配（绿框）
    if template_matches:
        for m in template_matches:
            x, y, rw, rh = m.rect.to_pixels(w, h)
            cv2.rectangle(vis, (x, y), (x + rw, y + rh), (0, 255, 0), 2)
            label = f"{m.template_name} {m.confidence:.2f}@{m.scale:.2f}x"
            cv2.putText(vis, label, (x, y - 5), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (0, 255, 0), 1)

    # OCR（蓝框 + 文字）
    if ocr_results:
        for r in ocr_results:
            x, y, rw, rh = r.rect.to_pixels(w, h)
            cv2.rectangle(vis, (x, y), (x + rw, y + rh), (255, 0, 0), 1)
            cv2.putText(vis, r.text, (x, y - 5), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (255, 0, 0), 1)

    # 目标检测（红框）
    if detections:
        for d in detections:
            x, y, rw, rh = d.rect.to_pixels(w, h)
            cv2.rectangle(vis, (x, y), (x + rw, y + rh), (0, 0, 255), 2)
            label = f"{d.label} {d.confidence:.2f}"
            cv2.putText(vis, label, (x, y - 5), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (0, 0, 255), 1)

    return vis