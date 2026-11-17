"""OCR（文字识别）原语：封装 RapidOCR + ONNX Runtime。

对静态图像 / ROI 调用 OCR，返回文字、置信度、文字框。
插件可传自己的 .onnx 模型（如 `hud_digits_v1.onnx`）做定制识别。
"""
from __future__ import annotations

from dataclasses import dataclass, field
from typing import Iterable, List, Optional, Sequence, Tuple, Union

import cv2
import numpy as np

from .frames import FrameSource, VideoFrame, load_frame
from .preprocess import Roi, crop


# ---- 结果类型 ----


@dataclass
class OcrBox:
    """单个文字框。

    Attributes:
        text:  识别出的文字。
        confidence: 0..1 置信度。
        bbox: 四顶点 [(x,y), ...] 图像坐标系（未裁 ROI 前）。
        quad: 同 bbox，命名兼容。
    """

    text: str
    confidence: float
    bbox: List[Tuple[float, float]]
    quad: List[Tuple[float, float]] = field(default_factory=list)

    def __post_init__(self):
        if not self.quad:
            self.quad = list(self.bbox)


@dataclass
class OcrResult:
    """一次 OCR 的完整结果。

    Attributes:
        text: 拼接后的全文本（按 bbox 左→右、上→下排序）。
        confidence: 所有 box 置信度的最小值（保守估计）。
        boxes: OcrBox 列表。
        raw: RapidOCR 原始返回（调试用）。
    """

    text: str
    confidence: float
    boxes: List[OcrBox]
    raw: Optional[list] = None


# ---- 引擎 ----


class OcrEngine:
    """轻量 RapidOCR 封装：det + cls + rec 三段 ONNX。

    构造时只加载一次；重复调用 `__call__` / `recognize` 复用会话。
    """

    def __init__(
        self,
        *,
        det_model_path: Optional[str] = None,
        cls_model_path: Optional[str] = None,
        rec_model_path: Optional[str] = None,
        providers: Optional[Iterable[str]] = None,
        intra_op_num_threads: int = 2,
        inter_op_num_threads: int = 1,
        rec_batch_num: int = 6,
    ):
        # 延迟 import，避免未安装 rapidocr 时整个包 import 失败
        from rapidocr_onnxruntime import RapidOCR

        self._providers = list(providers) if providers else None
        kwargs = dict(
            intra_op_num_threads=intra_op_num_threads,
            inter_op_num_threads=inter_op_num_threads,
            rec_batch_num=rec_batch_num,
        )
        if det_model_path:
            kwargs["det_model_path"] = det_model_path
        if cls_model_path:
            kwargs["cls_model_path"] = cls_model_path
        if rec_model_path:
            kwargs["rec_model_path"] = rec_model_path
        if self._providers:
            kwargs["engine_cfg"] = {"intra_op": {"num_threads": intra_op_num_threads}}
        self._ocr = RapidOCR(**kwargs)

    def recognize(self, image: np.ndarray) -> Tuple[List[Tuple[str, float, List]], float]:
        """底层调用：返回 RapidOCR 原始结构 [(box, text, conf), ...] 和耗时（秒）。"""
        result, elapsed = self._ocr(image)
        if not result:
            return [], 0.0
        # RapidOCR 返回 [(box4, text, conf), ...]；我们统一为 (text, conf, box)
        # 便于下游 `_boxes_from_raw` 使用
        out: List[Tuple[str, float, List]] = []
        for item in result:
            if len(item) < 3:
                continue
            box, text, conf = item[0], item[1], float(item[2])
            out.append((text, conf, box))
        elapsed_s = float(sum(elapsed)) if isinstance(elapsed, list) else float(elapsed or 0.0)
        return out, elapsed_s

    def __call__(self, image: np.ndarray) -> Tuple[List[Tuple[str, float, List]], float]:
        return self.recognize(image)


# ---- 高层 API ----


def _as_bgr(src: FrameSource) -> Tuple[np.ndarray, Optional[object]]:
    f = load_frame(src)
    return f.image, f.path


def _boxes_from_raw(
    raw: List[Tuple[str, float, List]],
    offset: Tuple[int, int] = (0, 0),
) -> List[OcrBox]:
    """把 RapidOCR 输出转成 OcrBox，bbox 加上 ROI 偏移。"""
    ox, oy = offset
    boxes: List[OcrBox] = []
    for item in raw or []:
        if len(item) < 3:
            continue
        text, conf, quad = item[0], float(item[1]), item[2]
        if not text:
            continue
        bbox = [(float(p[0]) + ox, float(p[1]) + oy) for p in quad]
        boxes.append(OcrBox(text=text, confidence=conf, bbox=bbox))
    return boxes


def _sort_boxes(boxes: List[OcrBox]) -> List[OcrBox]:
    """按 (行, 列) 排序：y 中心分桶 → x 排序。"""
    if not boxes:
        return boxes
    heights = [b.bbox[1][1] - b.bbox[0][1] for b in boxes if len(b.bbox) >= 2]
    med_h = float(np.median(heights)) if heights else 20.0
    row_tol = max(8.0, med_h * 0.6)

    def key(b: OcrBox):
        ys = [p[1] for p in b.bbox]
        xs = [p[0] for p in b.bbox]
        return (round(sum(ys) / len(ys) / row_tol), min(xs))

    return sorted(boxes, key=key)


def ocr_region(
    image: FrameSource,
    roi: Optional[Roi] = None,
    engine: Union[str, OcrEngine] = "onnx",
    model_path: Optional[str] = None,
    *,
    min_confidence: float = 0.0,
    upscale: bool = True,
    providers: Optional[Iterable[str]] = None,
) -> OcrResult:
    """对图像（或 ROI）做 OCR。

    Args:
        image: ndarray / 路径 / bytes。
        roi: Roi 实例或 (x, y, w, h) 元组；None → 整图。
        engine:
            "onnx" / "rapidocr" — 默认 RapidOCR（ONNX Runtime）。
            OcrEngine 实例 — 复用已有引擎。
            "paddle" / 其他 — 当前不支持，抛 ValueError。
        model_path: 插件自带的 rec 模型 .onnx 路径（覆盖默认 rec 模型）。
                    例如 AHC 的 `hud_digits_v1.onnx`。
        min_confidence: 过滤低于此阈值的 box。
        upscale: True 时对小 ROI 自动 3x 放大（CUBIC），提升小字识别率。
        providers: 仅当 engine 是字符串且 model_path 提供时透传。

    Returns:
        OcrResult(text, confidence, boxes)。text 是排序拼接结果。
    """
    img, _ = _as_bgr(image)

    # 解析 roi
    if roi is None:
        r = Roi(0, 0, img.shape[1], img.shape[0])
        offset = (0, 0)
    elif isinstance(roi, Roi):
        r = roi.clamp(img)
        offset = (r.x, r.y)
    elif isinstance(roi, (tuple, list)) and len(roi) == 4:
        r = Roi(int(roi[0]), int(roi[1]), int(roi[2]), int(roi[3])).clamp(img)
        offset = (r.x, r.y)
    else:
        raise TypeError(f"unsupported roi type: {type(roi).__name__}")

    if r.is_empty:
        return OcrResult(text="", confidence=0.0, boxes=[], raw=[])

    view_raw = crop(img, r)
    if view_raw.size == 0:
        return OcrResult(text="", confidence=0.0, boxes=[], raw=[])

    # OCR 前对小图 3x 放大（CUBIC）提升小字识别率；bbox 按 1/factor 缩回原图
    if upscale and view_raw.shape[0] < 32:
        view = cv2.resize(view_raw, None, fx=3, fy=3, interpolation=cv2.INTER_CUBIC)
        scale_back = 1.0 / 3.0
    else:
        view = view_raw
        scale_back = 1.0

    # 选择 / 构建引擎
    eng: OcrEngine
    if isinstance(engine, OcrEngine):
        eng = engine
    elif isinstance(engine, str):
        e = engine.lower()
        if e in ("onnx", "rapidocr", "rapidocr_onnxruntime"):
            kwargs = {}
            if model_path:
                kwargs["rec_model_path"] = model_path
            if providers:
                kwargs["providers"] = providers
            eng = OcrEngine(**kwargs)
        else:
            raise ValueError(f"unsupported ocr engine: {engine}")
    else:
        raise TypeError(f"engine must be str or OcrEngine, got {type(engine).__name__}")

    raw, _elapsed = eng.recognize(view)

    # bbox 缩放回原图坐标：先除以 scale_back，再加 ROI offset
    boxes: List[OcrBox] = []
    for item in raw or []:
        if len(item) < 3:
            continue
        text, conf, quad = item[0], float(item[1]), item[2]
        if not text:
            continue
        bbox = [
            (float(p[0]) * scale_back + offset[0], float(p[1]) * scale_back + offset[1])
            for p in quad
        ]
        boxes.append(OcrBox(text=text, confidence=conf, bbox=bbox))

    boxes = _sort_boxes(boxes)
    boxes = [b for b in boxes if b.confidence >= min_confidence]

    if not boxes:
        return OcrResult(text="", confidence=0.0, boxes=[], raw=raw)

    text = "".join(b.text for b in boxes)
    confidence = min(b.confidence for b in boxes)
    return OcrResult(text=text, confidence=confidence, boxes=boxes, raw=raw)


def rec_text(
    image: FrameSource,
    roi: Optional[Roi] = None,
    *,
    engine: Union[str, OcrEngine] = "onnx",
    model_path: Optional[str] = None,
    **kwargs,
) -> str:
    """`ocr_region` 的便捷封装：只返回拼接文本。"""
    return ocr_region(image, roi, engine=engine, model_path=model_path, **kwargs).text
