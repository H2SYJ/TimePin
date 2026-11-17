"""match_template 单元测试 —— 用 AHC 真实模板。"""
from __future__ import annotations

import cv2
import numpy as np
import pytest

from timepin_vision import Roi, match_all, match_template


class TestMatchTemplate:
    def test_perfect_match(self, frame_with_marker, damage_marker):
        img, gt = frame_with_marker
        hits = match_template(img, str(damage_marker), threshold=0.7)
        assert hits, "应至少命中一次"
        top = hits[0]
        assert top.score > 0.9
        x, y, w, h = top.bbox
        gx, gy, gw, gh = gt
        # bbox 中心对齐即可（缩放会改 bbox 尺寸）
        cx, cy = x + w / 2, y + h / 2
        assert abs(cx - (gx + gw / 2)) < 10
        assert abs(cy - (gy + gh / 2)) < 10

    def test_no_match_below_threshold(self, noise_frame, damage_marker):
        hits = match_template(noise_frame, str(damage_marker), threshold=0.99)
        assert not hits

    def test_scale_search(self, frame_with_marker, damage_marker):
        """模板 1.2x 缩放后仍能命中（在多尺度列表内）。"""
        tpl = cv2.imread(str(damage_marker))
        th, tw = tpl.shape[:2]
        # 放大 1.2x
        tpl_big = cv2.resize(tpl, None, fx=1.2, fy=1.2, interpolation=cv2.INTER_LINEAR)
        img = np.zeros((300, 400, 3), dtype=np.uint8)
        x, y = 100, 80
        img[y : y + tpl_big.shape[0], x : x + tpl_big.shape[1]] = tpl_big
        hits = match_template(img, str(damage_marker), threshold=0.7)
        assert hits
        # 1.2 缩放应在结果里
        scales = [h.scale for h in hits]
        assert any(abs(s - 1.2) < 0.05 for s in scales)

    def test_mask(self, elimination_template, elimination_mask, blank_frame):
        tpl = cv2.imread(str(elimination_template))
        mask = cv2.imread(str(elimination_mask), cv2.IMREAD_GRAYSCALE)
        th, tw = tpl.shape[:2]
        # 贴到 1080p 帧里
        x, y = 700, 400
        img = blank_frame.copy()
        img[y : y + th, x : x + tw] = tpl
        hits = match_template(
            img,
            str(elimination_template),
            mask=mask,
            threshold=0.7,
        )
        assert hits
        top = hits[0]
        assert top.score > 0.85

    def test_methods(self, frame_with_marker, damage_marker):
        img, _ = frame_with_marker
        for m in ("TM_CCOEFF_NORMED", "TM_CCORR_NORMED", "TM_SQDIFF_NORMED"):
            hits = match_template(img, str(damage_marker), threshold=0.7, method=m)
            assert hits, f"{m} 应命中"
            assert hits[0].method == m

    def test_top_k(self, frame_with_marker, damage_marker):
        img, _ = frame_with_marker
        hits = match_template(img, str(damage_marker), threshold=0.3, top_k=3)
        # NMS 去重后 ≤ 3 * len(scales)
        assert len(hits) <= 15  # 5 scales × 3
        # 分数降序
        for i in range(len(hits) - 1):
            assert hits[i].score >= hits[i + 1].score

    def test_template_larger_than_image(self, damage_marker):
        small = np.zeros((10, 10, 3), dtype=np.uint8)
        hits = match_template(small, str(damage_marker))
        assert hits == []

    def test_custom_scales(self, frame_with_marker, damage_marker):
        img, _ = frame_with_marker
        hits = match_template(img, str(damage_marker), scales=[1.0], threshold=0.7)
        assert hits
        assert all(abs(h.scale - 1.0) < 0.01 for h in hits)


class TestMatchAll:
    def test_alias(self, frame_with_marker, damage_marker):
        img, _ = frame_with_marker
        a = match_all(img, str(damage_marker), threshold=0.7)
        b = match_template(img, str(damage_marker), threshold=0.7)
        assert len(a) == len(b)
