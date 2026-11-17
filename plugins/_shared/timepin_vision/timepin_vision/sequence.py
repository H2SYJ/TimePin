"""数字序列比对与逐位数字识别。

`diff_sequence` 是纯函数式 API：给定一个帧序列和"每帧识别出的字符串"，
返回带变更点的 `FrameDelta` 列表 —— 规则插件据此生成标注。

`read_digits` / `extract_digits` 是便利封装：把 `OcrEngine` 或自定义
`DigitDecoder` 应用到帧序列，返回每帧识别的字符串。
"""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Callable, Iterable, List, Optional, Sequence, Tuple, Union

import cv2
import numpy as np

from .frames import FrameBatch, FrameSource, VideoFrame, load_frame
from .model import load_onnx_session
from .preprocess import Roi, crop


# ---- 数据类型 ----


@dataclass(frozen=True)
class FrameDelta:
    """一帧相对前一有效帧的差异。

    Attributes:
        index: 帧序号（在传入序列中的位置）。
        frame: 该帧的 VideoFrame（含图像与时间戳）。
        text:  该帧识别出的字符串（None 表示未识别 / 跳过）。
        changed: 文本是否相对上一个有效帧发生变化。
        score: 置信度（0..1；无则为 0）。
    """

    index: int
    frame: VideoFrame
    text: Optional[str]
    changed: bool
    score: float = 0.0


class DigitDecoder:
    """数字识别器接口。

    子类实现 `decode(image) -> (text, score)`。
    """

    def decode(self, image: np.ndarray) -> Tuple[str, float]:
        raise NotImplementedError

    def __call__(self, image: np.ndarray) -> Tuple[str, float]:
        return self.decode(image)


class OnnxDigitDecoder(DigitDecoder):
    """CNN+CTC 数字识别器 —— 对齐 AHC `hud_digit_reader.py`。

    输入: BGR ndarray（已裁剪到 HUD 数字区域）
    输出: (text, score) —— text 为纯数字字符串，score 为平均字符置信度。

    模型结构对齐 AHC：输入 [1,1,28,64] float32 灰度，输出 [1,T,11] logits
    （10 个数字 + blank）。
    """

    HEIGHT = 28
    WIDTH = 64
    # AHC hud_digits_v1.onnx 的字符表：index 0 = CTC blank，index 1..10 = '0'..'9'
    CHARSET = "0123456789"
    BLANK = 0

    def __init__(
        self,
        model_path: Union[str, bytes],
        *,
        providers: Optional[Iterable[str]] = None,
        intra_op_num_threads: int = 2,
        inter_op_num_threads: int = 1,
    ):
        self.session = load_onnx_session(
            model_path,
            providers=providers,
            intra_op_num_threads=intra_op_num_threads,
            inter_op_num_threads=inter_op_num_threads,
        )
        self.input_name = self.session.get_inputs()[0].name
        self.output_name = self.session.get_outputs()[0].name

    def _prepare(self, image: np.ndarray) -> np.ndarray:
        """BGR → 灰度 [0,1] → 等比缩放到 HEIGHT → 左对齐放进 HEIGHT×WIDTH 画布。

        对齐 AHC `hud_digit_reader.prepare`：
        - gray.astype(np.float32) / 255
        - width = max(4, round(w * HEIGHT / h))
        - canvas[:, :min(WIDTH, width)] = resized[:, :WIDTH]
        """
        if image.ndim == 3:
            gray = cv2.cvtColor(image, cv2.COLOR_BGR2GRAY)
        else:
            gray = image
        gray = gray.astype(np.float32) / 255.0
        h, w = gray.shape
        new_w = max(4, int(round(w * self.HEIGHT / h))) if h > 0 else 4
        interp = cv2.INTER_AREA if h >= self.HEIGHT else cv2.INTER_LINEAR
        resized = cv2.resize(gray, (new_w, self.HEIGHT), interpolation=interp)
        canvas = np.zeros((self.HEIGHT, self.WIDTH), dtype=np.float32)
        canvas[:, : min(self.WIDTH, new_w)] = resized[:, : self.WIDTH]
        return canvas.reshape(1, 1, self.HEIGHT, self.WIDTH)

    def _ctc_decode(self, probs: np.ndarray) -> Tuple[str, float]:
        """CTC 贪心解码（对齐 AHC `hud_digit_reader.decode`）。

        模型输出 `probs` 已是 softmax 概率（不是 logits）。
        - index 0 是 blank
        - index 1..10 是 '0'..'9'
        - 连续相同 label 合并；confidence 取所输出字符最大概率中的最小值。
        """
        if probs.ndim == 3:
            probs = probs[0]
        # probs: [T, C=11]
        idx = np.argmax(probs, axis=-1)
        chars: List[str] = []
        confidence = 1.0
        prev = -1
        for step, c in enumerate(idx.tolist()):
            if c != prev and c != self.BLANK:
                # label 1..10 → '0'..'9'
                if 1 <= c <= 10:
                    chars.append(str(c - 1))
                    confidence = min(confidence, float(probs[step, c]))
            prev = c
        text = "".join(chars)
        if not text:
            confidence = 0.0
        return text, confidence

    def decode(self, image: np.ndarray) -> Tuple[str, float]:
        x = self._prepare(image)
        out = self.session.run([self.output_name], {self.input_name: x})[0]
        return self._ctc_decode(out)


# ---- 序列比对 ----


def _norm_text(s: Optional[str]) -> str:
    return (s or "").strip()


def diff_sequence(
    frames: Union[FrameBatch, Sequence[FrameSource]],
    texts: Sequence[Optional[str]],
    *,
    scores: Optional[Sequence[float]] = None,
    pts_sec: Optional[Sequence[float]] = None,
    start_index: int = 0,
    stable_frames: int = 1,
) -> List[FrameDelta]:
    """对齐帧与识别文本，输出带变更标记的 `FrameDelta` 列表。

    Args:
        frames: FrameBatch 或等长的 FrameSource 序列。
        texts: 与 frames 等长的字符串序列（None/"" 表示未识别）。
        scores: 可选置信度序列。
        pts_sec: 可选时间戳序列（秒）；提供时覆盖 VideoFrame.pts_sec。
        start_index: 帧序号起始值（用于视频抽帧场景）。
        stable_frames: 文本连续出现 >= N 帧才算"稳定"；
                       默认 1 = 立即生效。HUD 抖动场景建议 2-3。

    Returns:
        List[FrameDelta]，长度 == len(frames)。
        `changed=True` 表示该帧文本与上一个稳定值不同。
    """
    if isinstance(frames, FrameBatch):
        batch = frames
        sources = list(batch)
    else:
        sources = list(frames)
        batch = None

    n = len(sources)
    if len(texts) != n:
        raise ValueError(f"texts length {len(texts)} != frames length {n}")
    if scores is not None and len(scores) != n:
        raise ValueError(f"scores length {len(scores)} != frames length {n}")
    if pts_sec is not None and len(pts_sec) != n:
        raise ValueError(f"pts_sec length {len(pts_sec)} != frames length {n}")

    # 物化帧（如果是 FrameBatch 则已加载；否则按需 load_frame）
    video_frames: List[VideoFrame] = []
    for i, src in enumerate(sources):
        if isinstance(src, VideoFrame):
            vf = src
        else:
            pts = pts_sec[i] if pts_sec is not None else None
            idx = start_index + i
            vf = load_frame(src, pts_sec=pts, index=idx)
        video_frames.append(vf)

    deltas: List[FrameDelta] = []
    last_stable: Optional[str] = None
    pending_text: Optional[str] = None
    pending_count = 0

    for i, vf in enumerate(video_frames):
        text = _norm_text(texts[i]) or None
        score = float(scores[i]) if scores is not None else 0.0

        # stable_frames 逻辑：连续 N 帧相同文本才更新 stable
        if text is not None and text == pending_text:
            pending_count += 1
        else:
            pending_text = text
            pending_count = 1 if text is not None else 0

        changed = False
        if text is not None and pending_count >= stable_frames:
            stable_text = pending_text
            if stable_text != last_stable:
                changed = True
                last_stable = stable_text
        elif text is None:
            # 未识别帧：不更新 stable，changed=False
            pass

        deltas.append(
            FrameDelta(
                index=start_index + i,
                frame=vf,
                text=text,
                changed=changed,
                score=score,
            )
        )

    return deltas


# ---- 便利 API ----


def extract_digits(
    image: FrameSource,
    roi: Optional[Roi] = None,
    decoder: Union[DigitDecoder, OnnxDigitDecoder, str, bytes] = "onnx",
    *,
    model_path: Optional[Union[str, bytes]] = None,
) -> Tuple[str, float]:
    """对单帧 ROI 提取数字字符串。

    Args:
        image: ndarray / 路径 / bytes。
        roi: Roi / (x,y,w,h) / None（整图）。
        decoder:
            - OnnxDigitDecoder 实例：直接用。
            - str / bytes：视为 ONNX 模型路径或字节，构造 OnnxDigitDecoder。
            - "onnx" / "ocr"：未实现（保留接口）。
        model_path: 当 decoder 为 "onnx" 时使用的模型路径。

    Returns:
        (digits, score) —— digits 为纯数字字符串（可能空），score 为置信度。
    """
    img = load_frame(image).image
    if roi is None:
        r = Roi(0, 0, img.shape[1], img.shape[0])
    elif isinstance(roi, Roi):
        r = roi.clamp(img)
    elif isinstance(roi, (tuple, list)) and len(roi) == 4:
        r = Roi(int(roi[0]), int(roi[1]), int(roi[2]), int(roi[3])).clamp(img)
    else:
        raise TypeError(f"unsupported roi type: {type(roi).__name__}")

    if r.is_empty:
        return "", 0.0
    view = crop(img, r)
    if view.size == 0:
        return "", 0.0

    dec: DigitDecoder
    if isinstance(decoder, DigitDecoder):
        dec = decoder
    elif isinstance(decoder, (str, bytes, bytearray)):
        # str 路径 or bytes 模型
        if isinstance(decoder, str) and decoder in ("onnx", "ocr", "rapidocr"):
            # 命名引擎：onnx → 需要 model_path
            if not model_path:
                raise ValueError("model_path required when decoder='onnx'")
            dec = OnnxDigitDecoder(model_path if isinstance(model_path, str) else model_path)
        else:
            # 直接当作模型路径/字节
            dec = OnnxDigitDecoder(decoder)
    else:
        raise TypeError(f"unsupported decoder type: {type(decoder).__name__}")

    text, score = dec.decode(view)
    # 只保留数字
    digits = re.sub(r"\D", "", text)
    return digits, score


def read_digits(
    frames: Union[FrameBatch, Sequence[FrameSource]],
    roi: Optional[Roi] = None,
    decoder: Union[DigitDecoder, OnnxDigitDecoder, str, bytes] = "onnx",
    *,
    model_path: Optional[Union[str, bytes]] = None,
    pts_sec: Optional[Sequence[float]] = None,
    start_index: int = 0,
) -> List[Tuple[int, float, str, float]]:
    """对帧序列批量提取数字。

    Returns:
        List[(index, pts_sec, digits, score)]，长度 == len(frames)。
    """
    if isinstance(frames, FrameBatch):
        batch = frames
        sources = list(batch)
    else:
        sources = list(frames)
        batch = None

    n = len(sources)
    if pts_sec is not None and len(pts_sec) != n:
        raise ValueError(f"pts_sec length {len(pts_sec)} != frames length {n}")

    results: List[Tuple[int, float, str, float]] = []
    # 复用 decoder（避免重复加载模型）
    dec: Optional[DigitDecoder] = None
    if isinstance(decoder, DigitDecoder):
        dec = decoder
    elif isinstance(decoder, (str, bytes, bytearray)):
        if isinstance(decoder, str) and decoder in ("onnx", "ocr", "rapidocr"):
            if not model_path:
                raise ValueError("model_path required when decoder='onnx'")
            dec = OnnxDigitDecoder(model_path)
        else:
            dec = OnnxDigitDecoder(decoder)
    else:
        raise TypeError(f"unsupported decoder type: {type(decoder).__name__}")

    for i, src in enumerate(sources):
        if isinstance(src, VideoFrame):
            img = src.image
            pts = pts_sec[i] if pts_sec is not None else src.pts_sec
            idx = start_index + i
        else:
            pts = pts_sec[i] if pts_sec is not None else None
            vf = load_frame(src, pts_sec=pts, index=start_index + i)
            img = vf.image
            pts = vf.pts_sec
            idx = start_index + i

        if roi is None:
            r = Roi(0, 0, img.shape[1], img.shape[0])
        elif isinstance(roi, Roi):
            r = roi.clamp(img)
        elif isinstance(roi, (tuple, list)) and len(roi) == 4:
            r = Roi(int(roi[0]), int(roi[1]), int(roi[2]), int(roi[3])).clamp(img)
        else:
            raise TypeError(f"unsupported roi type: {type(roi).__name__}")

        if r.is_empty:
            results.append((idx, float(pts or 0.0), "", 0.0))
            continue
        view = crop(img, r)
        if view.size == 0:
            results.append((idx, float(pts or 0.0), "", 0.0))
            continue
        text, score = dec.decode(view)
        digits = re.sub(r"\D", "", text)
        results.append((idx, float(pts or 0.0), digits, score))

    return results
