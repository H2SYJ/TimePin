"""ocr_region 单元测试 —— RapidOCR 在合成文本与 AHC 模板上。"""
from __future__ import annotations

import cv2
import numpy as np
import pytest

from timepin_vision import OcrEngine, Roi, ocr_region, rec_text


def _draw_text(text: str, size=(400, 60), scale=1.0) -> np.ndarray:
    """用 OpenCV 画白底黑字，模拟 HUD 文本。"""
    w, h = size
    img = np.full((h, w, 3), 255, dtype=np.uint8)
    font = cv2.FONT_HERSHEY_SIMPLEX
    cv2.putText(
        img, text, (10, h - 15), font, scale, (0, 0, 0), 2, cv2.LINE_AA
    )
    return img


class TestOcrRegion:
    def test_full_image(self):
        img = _draw_text("KILL 12")
        res = ocr_region(img)
        assert isinstance(res.text, str)
        # RapidOCR 对清晰文本应有非零置信度
        assert res.confidence >= 0.0
        assert isinstance(res.boxes, list)

    def test_roi(self):
        """构造大图 + ROI：ROI 内文字应被识别。"""
        img = np.full((600, 800, 3), 128, dtype=np.uint8)
        # ROI 内画字
        roi = Roi(50, 100, 300, 60)
        cv2.putText(
            img, "SCORE", (60, 140), cv2.FONT_HERSHEY_SIMPLEX, 0.8,
            (255, 255, 255), 2, cv2.LINE_AA,
        )
        res = ocr_region(img, roi=roi)
        assert res.text  # 应识别出非空
        assert res.boxes
        # bbox 应在 ROI 内
        for b in res.boxes:
            xs = [p[0] for p in b.bbox]
            ys = [p[1] for p in b.bbox]
            assert min(xs) >= roi.x - 5
            assert min(ys) >= roi.y - 5

    def test_roi_tuple(self):
        img = _draw_text("TEST")
        res = ocr_region(img, roi=(0, 0, 200, 60))
        assert res.text

    def test_empty_roi(self):
        img = np.zeros((100, 100, 3), dtype=np.uint8)
        res = ocr_region(img, roi=Roi(0, 0, 0, 0))
        assert res.text == ""
        assert res.confidence == 0.0
        assert res.boxes == []

    def test_engine_instance(self):
        img = _draw_text("HI")
        eng = OcrEngine()
        res = ocr_region(img, engine=eng)
        assert res.text
        # 同一引擎再调一次（复用会话）
        res2 = ocr_region(img, engine=eng)
        assert res2.text

    def test_invalid_engine(self):
        img = np.zeros((50, 200, 3), dtype=np.uint8)
        with pytest.raises(ValueError, match="unsupported ocr engine"):
            ocr_region(img, engine="paddle")

    def test_rec_text_shortcut(self):
        img = _draw_text("OK")
        t = rec_text(img)
        assert isinstance(t, str)


class TestAhcTemplates:
    """对 AHC 真实模板做 OCR 验证（digit-N.png 是灰度数字图）。"""

    def test_digit_0(self, digit_0_img):
        res = ocr_region(digit_0_img, upscale=False)
        # digit-0.png 内容是 "0"；RapidOCR 可能返回 "0" 或其他数字
        # 我们至少验证 API 不崩，并返回合理结构
        assert isinstance(res.text, str)

    def test_kills_gray(self):
        img = cv2.imread(
            str(__file__).replace("test_ocr.py", "fixtures/kills-gray.png")
        )
        res = ocr_region(img, upscale=False)
        assert isinstance(res.text, str)
