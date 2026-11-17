"""多尺度模板匹配 + 掩码 + 阈值过滤。

只处理静态图像：输入 ndarray / 路径 / 字节；返回 `MatchResult` 列表。
不做视频抽帧、不做 OCR —— 那两个是独立原语。
"""
from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, List, Optional, Sequence, Tuple, Union

import cv2
import numpy as np

from .frames import FrameSource, load_frame
from .preprocess import Roi


# cv2.matchTemplate 支持的方法
TEMPLATE_METHODS = ("TM_CCOEFF_NORMED", "TM_CCORR_NORMED", "TM_SQDIFF_NORMED")


@dataclass(frozen=True)
class MatchResult:
    """一次模板匹配命中。

    Attributes:
        score: 0..1 归一化分数（方法不同时含义略有差异——
               TM_CCOEFF_NORMED / TM_CCORR_NORMED 越高越好；
               TM_SQDIFF_NORMED 我们已做 1-score 转换）。
        bbox: (x, y, w, h) —— 图像坐标系中的命中矩形。
        scale: 模板被缩放的比例（相对模板原图）。
        template_size: (tpl_w, tpl_h) 经过缩放后的模板尺寸。
        method: 使用的匹配方法名。
    """

    score: float
    bbox: Tuple[int, int, int, int]
    scale: float
    template_size: Tuple[int, int]
    method: str


def _as_image(src: FrameSource) -> np.ndarray:
    return load_frame(src).image


def _normalize_method(method: str) -> str:
    m = method.upper()
    if not m.startswith("TM_"):
        m = "TM_" + m
    if m not in TEMPLATE_METHODS:
        raise ValueError(f"unknown template match method: {method}")
    return m


def _method_code(method: str) -> int:
    """把 'TM_*' 名称转为 cv2 常量。"""
    return {
        "TM_CCOEFF_NORMED": cv2.TM_CCOEFF_NORMED,
        "TM_CCORR_NORMED": cv2.TM_CCORR_NORMED,
        "TM_SQDIFF_NORMED": cv2.TM_SQDIFF_NORMED,
    }[method]


def _to_mask(mask: Optional[np.ndarray], tpl_shape: Tuple[int, int]) -> Optional[np.ndarray]:
    """把 mask 规整为模板尺寸的 uint8 单通道；越界 / 尺寸不符视为错误。"""
    if mask is None:
        return None
    m = mask
    if m.dtype != np.uint8:
        m = (m > 0).astype(np.uint8) * 255
    if m.ndim == 3:
        m = cv2.cvtColor(m, cv2.COLOR_BGR2GRAY)
    if m.shape[:2] != tpl_shape[:2]:
        raise ValueError(
            f"mask shape {m.shape[:2]} does not match template {tpl_shape[:2]}"
        )
    return m


def _score_location(
    result_map: np.ndarray,
    method: str,
    x: int,
    y: int,
) -> float:
    """返回统一语义的分数：越高越好（0..1）。"""
    raw = float(result_map[y, x])
    if method == "TM_SQDIFF_NORMED":
        # SQDIFF 越小越好，转成越大越好：score = 1 - dist
        return max(0.0, min(1.0, 1.0 - raw))
    return float(np.clip(raw, 0.0, 1.0))


def match_template(
    image: FrameSource,
    template: FrameSource,
    scales: Optional[Sequence[float]] = None,
    mask: Optional[np.ndarray] = None,
    threshold: float = 0.8,
    *,
    method: str = "TM_CCOEFF_NORMED",
    top_k: int = 1,
    nms_iou: float = 0.3,
) -> List[MatchResult]:
    """在图像上多尺度查找模板。

    Args:
        image / template: ndarray / 路径 / bytes。
        scales: 模板缩放比例列表（相对 template 原尺寸）。
                None → [0.8, 0.9, 1.0, 1.1, 1.2]（参考 AHC combat_outcome_reader）。
        mask: 与 template 同尺寸的 uint8 mask；非零为参与匹配区域。
              对多尺度缩放会同步缩放 mask。
        threshold: 分数下限（统一语义 0..1，越高越好）。
        method: TM_CCOEFF_NORMED | TM_CCORR_NORMED | TM_SQDIFF_NORMED。
        top_k: 每个尺度返回前 K 个候选（再经过 NMS）。
        nms_iou: NMS 的 IoU 阈值，重叠候选合并。

    Returns:
        按 score 降序的 MatchResult 列表（可能为空）。
    """
    img = _as_image(image)
    tpl = _as_image(template)
    if tpl.shape[0] > img.shape[0] or tpl.shape[1] > img.shape[1]:
        # 模板比图像大，跳过缩放列表中的所有 > 1 的项仍可能成功；
        # 这里直接返回空更省事
        return []

    m = _normalize_method(method)
    if scales is None:
        scales = [0.8, 0.9, 1.0, 1.1, 1.2]
    scales = [float(s) for s in scales if s > 0]
    if not scales:
        return []

    base_h, base_w = tpl.shape[:2]
    candidates: List[MatchResult] = []

    # 对每个缩放做一次匹配，把 mask 同步缩放
    for scale in scales:
        new_w = max(1, int(round(base_w * scale)))
        new_h = max(1, int(round(base_h * scale)))
        if new_w > img.shape[1] or new_h > img.shape[0]:
            continue
        if new_w == base_w and new_h == base_h:
            tpl_s = tpl
            mask_s = _to_mask(mask, (base_h, base_w))
        else:
            interp = cv2.INTER_AREA if scale < 1.0 else cv2.INTER_LINEAR
            tpl_s = cv2.resize(tpl, (new_w, new_h), interpolation=interp)
            mask_s = None
            if mask is not None:
                mask_s = _to_mask(mask, (base_h, base_w))
                mask_s = cv2.resize(mask_s, (new_w, new_h), interpolation=cv2.INTER_NEAREST)

        if tpl_s.ndim == 3 and img.ndim == 2:
            img_gray = img
            tpl_gray = cv2.cvtColor(tpl_s, cv2.COLOR_BGR2GRAY)
        elif tpl_s.ndim == 2 and img.ndim == 3:
            tpl_gray = tpl_s
            img_gray = cv2.cvtColor(img, cv2.COLOR_BGR2GRAY)
        elif tpl_s.ndim == 2 and img.ndim == 2:
            tpl_gray, img_gray = tpl_s, img
        else:
            tpl_gray, img_gray = tpl_s, img

        code = _method_code(m)
        try:
            if mask_s is not None:
                res = cv2.matchTemplate(img_gray, tpl_gray, code, mask=mask_s)
            else:
                res = cv2.matchTemplate(img_gray, tpl_gray, code)
        except cv2.error:
            # OpenCV 版本差异：mask 不支持时退化为无 mask
            res = cv2.matchTemplate(img_gray, tpl_gray, code)

        # mask + 稀疏匹配会产生 NaN/±inf；按方法语义分别处理：
        #   - CCOEFF/CCORR：NaN → 0，inf → 0（无效匹配）
        #   - SQDIFF：NaN → 1，inf → 1（最差距离，被 1-dist 转 0）
        if m == "TM_SQDIFF_NORMED":
            res = np.nan_to_num(res, nan=1.0, posinf=1.0, neginf=1.0)
        else:
            res = np.nan_to_num(res, nan=0.0, posinf=1.0, neginf=0.0)

        # 每个尺度只取 top_k 个局部极大
        flat = res.flatten()
        k = min(top_k * 4, flat.size)  # 多取一些给 NMS 筛选
        if k == 0:
            continue
        # SQDIFF：分数越低越好 → 取最小值；其他方法：取最大值
        if m == "TM_SQDIFF_NORMED":
            idx = np.argpartition(flat, k - 1)[:k]
            idx = idx[np.argsort(flat[idx])]  # 升序：最小在前
        else:
            idx = np.argpartition(flat, -k)[-k:]
            idx = idx[np.argsort(flat[idx])[::-1]]
        taken = 0
        for i in idx:
            if taken >= top_k:
                break
            y, x = divmod(int(i), res.shape[1])
            score = _score_location(res, m, x, y)
            if score < threshold:
                continue
            candidates.append(
                MatchResult(
                    score=score,
                    bbox=(x, y, new_w, new_h),
                    scale=scale,
                    template_size=(new_w, new_h),
                    method=m,
                )
            )
            taken += 1

    # NMS：按 score 降序，IoU > nms_iou 的丢弃
    # 过滤 NaN / -inf / +inf 分数（mask 完全无效 / 图像全零时 matchTemplate 会产生 NaN）
    candidates = [
        c for c in candidates
        if np.isfinite(c.score)
    ]
    candidates.sort(key=lambda r: r.score, reverse=True)
    kept: List[MatchResult] = []
    for c in candidates:
        if all(_iou(c.bbox, k.bbox) <= nms_iou for k in kept):
            kept.append(c)
    return kept


def match_all(
    image: FrameSource,
    template: FrameSource,
    scales: Optional[Sequence[float]] = None,
    mask: Optional[np.ndarray] = None,
    threshold: float = 0.8,
    *,
    method: str = "TM_CCOEFF_NORMED",
    top_k: int = 5,
    nms_iou: float = 0.3,
) -> List[MatchResult]:
    """`match_template` 的别名（更明确的命名，强调返回多结果）。"""
    return match_template(
        image, template, scales, mask, threshold,
        method=method, top_k=top_k, nms_iou=nms_iou,
    )


def _iou(a: Tuple[int, int, int, int], b: Tuple[int, int, int, int]) -> float:
    ax, ay, aw, ah = a
    bx, by, bw, bh = b
    x0 = max(ax, bx)
    y0 = max(ay, by)
    x1 = min(ax + aw, bx + bw)
    y1 = min(ay + ah, by + bh)
    inter = max(0, x1 - x0) * max(0, y1 - y0)
    union = aw * ah + bw * bh - inter
    return inter / union if union > 0 else 0.0
