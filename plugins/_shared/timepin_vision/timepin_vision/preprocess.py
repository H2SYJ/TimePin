"""图像预处理原语：HSV、亮度规整、保持长宽比缩放、透视矫正、OCR 放大。

所有函数输入都是 BGR ndarray 或可被 `_check_image` 规整的灰度/浮点图；
返回同为 uint8 ndarray。
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Optional, Sequence, Tuple, Union

import cv2
import numpy as np

from .frames import _check_image


# ---- ROI ----


@dataclass(frozen=True)
class Roi:
    """图像内的一个矩形感兴趣区域（像素坐标）。

    (x, y) 为左上角；w / h 为宽高。所有字段为非负 int。
    """

    x: int
    y: int
    w: int
    h: int

    def clamp(self, shape_or_size: Union[Tuple[int, int], np.ndarray]) -> "Roi":
        """把 ROI 裁剪到给定图像范围内。可能返回零宽/高的 Roi（调用方检查 is_empty）。

        Args:
            shape_or_size: (width, height) 或 ndarray（用其 shape）。
        """
        if isinstance(shape_or_size, np.ndarray):
            ih, iw = shape_or_size.shape[:2]
        else:
            iw, ih = shape_or_size
        x0 = max(0, min(self.x, iw))
        y0 = max(0, min(self.y, ih))
        x1 = max(0, min(self.x + self.w, iw))
        y1 = max(0, min(self.y + self.h, ih))
        return Roi(x=x0, y=y0, w=max(0, x1 - x0), h=max(0, y1 - y0))

    @property
    def is_empty(self) -> bool:
        return self.w <= 0 or self.h <= 0

    def as_slice(self) -> Tuple[slice, slice]:
        """y/x 切片，可直接 img[roi.as_slice()]。"""
        return (
            slice(self.y, self.y + self.h),
            slice(self.x, self.x + self.w),
        )


# CropRoi 是 Roi 的别名 —— 历史命名，两者等价。
CropRoi = Roi


def crop(image: np.ndarray, roi: Roi) -> np.ndarray:
    """提取 ROI；ROI 出界部分按零裁剪（超出图像部分不会返回）。ROI 完全出界返回 0x0 图。"""
    img = _check_image(image)
    r = roi.clamp(img)
    if r.is_empty:
        return np.zeros((0, 0, 3), dtype=np.uint8)
    return img[r.as_slice()].copy()


# ---- 色彩 / 亮度 ----


def to_hsv(image: np.ndarray) -> np.ndarray:
    """BGR → HSV (H:0-179, S/V:0-255)。"""
    img = _check_image(image)
    return cv2.cvtColor(img, cv2.COLOR_BGR2HSV)


def to_gray(image: np.ndarray) -> np.ndarray:
    """BGR → 灰度（uint8）。"""
    img = _check_image(image)
    if img.ndim == 2:
        return img
    return cv2.cvtColor(img, cv2.COLOR_BGR2GRAY)


def to_binary(
    image: np.ndarray,
    *,
    method: str = "otsu",
    threshold: int = 0,
    adaptive_block: int = 11,
    adaptive_c: int = 2,
    invert: bool = False,
) -> np.ndarray:
    """二值化。

    method:
        "otsu"      — 大津法（推荐干净 HUD）
        "fixed"     — 用 `threshold` 定阈
        "adaptive"  — 自适应局部阈值（适合背景复杂）
    invert: True 时输出反相（背景白、前景黑 → 反过来）。
    """
    gray = to_gray(image)
    flag = cv2.THRESH_BINARY_INV if invert else cv2.THRESH_BINARY
    if method == "otsu":
        _, bw = cv2.threshold(gray, 0, 255, flag | cv2.THRESH_OTSU)
    elif method == "fixed":
        _, bw = cv2.threshold(gray, threshold, 255, flag)
    elif method == "adaptive":
        bw = cv2.adaptiveThreshold(
            gray, 255, cv2.ADAPTIVE_THRESH_GAUSSIAN_C, flag, adaptive_block, adaptive_c
        )
    else:
        raise ValueError(f"unknown threshold method: {method}")
    return bw


def normalize_luma(image: np.ndarray, *, target_mean: float = 128.0) -> np.ndarray:
    """把图像亮度均一化到 target_mean：V 通道乘性缩放。

    用于把不同曝光/伽马的 HUD 帧拉到同一亮度水平，提升模板匹配/
    OCR 的稳定性。输入 BGR，输出 BGR。
    """
    img = _check_image(image)
    hsv = cv2.cvtColor(img, cv2.COLOR_BGR2HSV).astype(np.float32)
    v = hsv[:, :, 2]
    mean = float(v.mean())
    if mean <= 0:
        return img
    scale = target_mean / mean
    hsv[:, :, 2] = np.clip(v * scale, 0, 255)
    return cv2.cvtColor(hsv.astype(np.uint8), cv2.COLOR_HSV2BGR)


# ---- 缩放 ----


def resize_keep_ratio(
    image: np.ndarray,
    target_height: Optional[int] = None,
    target_width: Optional[int] = None,
    *,
    fill: Tuple[int, int, int] = (0, 0, 0),
    align: str = "center",
) -> np.ndarray:
    """等比缩放到目标高度或宽度，不足处用 `fill` 补齐。

    只传一个维度：等比缩放到该维度（另一维按比例，不裁不补）。
    两个都传：图像等比放进 (target_width, target_height) 画布居中。

    align: "center" | "left" | "top"（只在补 padding 时生效）。
    """
    img = _check_image(image)
    h, w = img.shape[:2]
    if target_height is None and target_width is None:
        return img
    if target_height is None:
        new_w = int(target_width)
        new_h = max(1, round(h * target_width / w))
        interp = cv2.INTER_AREA if h >= new_h else cv2.INTER_LINEAR
        return cv2.resize(img, (new_w, new_h), interpolation=interp)
    if target_width is None:
        new_h = int(target_height)
        new_w = max(1, round(w * target_height / h))
        interp = cv2.INTER_AREA if w >= new_w else cv2.INTER_LINEAR
        return cv2.resize(img, (new_w, new_h), interpolation=interp)

    # 两个都给：等比 fit 进画布
    scale = min(target_width / w, target_height / h)
    new_w = max(1, round(w * scale))
    new_h = max(1, round(h * scale))
    interp = cv2.INTER_AREA if (h >= new_h or w >= new_w) else cv2.INTER_LINEAR
    resized = cv2.resize(img, (new_w, new_h), interpolation=interp)
    canvas = np.full((target_height, target_width, 3), fill, dtype=np.uint8)
    if align == "left":
        ox, oy = 0, (target_height - new_h) // 2
    elif align == "top":
        ox, oy = (target_width - new_w) // 2, 0
    else:
        ox, oy = (target_width - new_w) // 2, (target_height - new_h) // 2
    canvas[oy : oy + new_h, ox : ox + new_w] = resized
    return canvas


def pad_to_aspect(
    image: np.ndarray,
    aspect: float,
    *,
    fill: Tuple[int, int, int] = (0, 0, 0),
) -> np.ndarray:
    """补 padding 让图像达到指定宽高比（w/h），不裁剪。"""
    img = _check_image(image)
    h, w = img.shape[:2]
    cur = w / h
    if abs(cur - aspect) < 1e-3:
        return img
    if cur < aspect:
        new_w = int(round(h * aspect))
        pad = new_w - w
        left = pad // 2
        right = pad - left
        return cv2.copyMakeBorder(img, 0, 0, left, right, cv2.BORDER_CONSTANT, value=fill)
    new_h = int(round(w / aspect))
    pad = new_h - h
    top = pad // 2
    bottom = pad - top
    return cv2.copyMakeBorder(img, top, bottom, 0, 0, cv2.BORDER_CONSTANT, value=fill)


def upscale_for_ocr(
    image: np.ndarray,
    *,
    min_height: int = 32,
    factor: int = 3,
) -> np.ndarray:
    """为 OCR 提升小图：小于 `min_height` 时整体放大 `factor` 倍（CUBIC）。"""
    img = _check_image(image)
    h, _w = img.shape[:2]
    if h >= min_height:
        return img
    return cv2.resize(img, None, fx=factor, fy=factor, interpolation=cv2.INTER_CUBIC)


# ---- 透视矫正 ----


def perspective_fix(
    image: np.ndarray,
    src_quad: Sequence[Tuple[float, float]],
    out_size: Optional[Tuple[int, int]] = None,
    *,
    border_value: Tuple[int, int, int] = (0, 0, 0),
) -> np.ndarray:
    """四点透视矫正。

    Args:
        image: BGR ndarray。
        src_quad: 源图像四顶点 [(x,y), ...]，顺序必须一致
                  （TL, TR, BR, BL）。
        out_size: (width, height)；None 时按四边形最大边长估算。
        border_value: 越界填充色。
    """
    img = _check_image(image)
    if len(src_quad) != 4:
        raise ValueError("src_quad must have exactly 4 points")
    src = np.asarray(src_quad, dtype=np.float32)
    if out_size is None:
        w = int(round(max(
            np.linalg.norm(src[0] - src[1]),
            np.linalg.norm(src[2] - src[3]),
        )))
        h = int(round(max(
            np.linalg.norm(src[0] - src[3]),
            np.linalg.norm(src[1] - src[2]),
        )))
        w = max(8, w)
        h = max(8, h)
    else:
        w, h = int(out_size[0]), int(out_size[1])
    dst = np.asarray(
        [[0, 0], [w - 1, 0], [w - 1, h - 1], [0, h - 1]], dtype=np.float32
    )
    M = cv2.getPerspectiveTransform(src, dst)
    return cv2.warpPerspective(img, M, (w, h), borderValue=border_value)
