"""diff_sequence / read_digits / extract_digits / OnnxDigitDecoder 单元测试。"""
from __future__ import annotations

import cv2
import numpy as np
import pytest

from timepin_vision import (
    FrameBatch,
    FrameDelta,
    OnnxDigitDecoder,
    Roi,
    diff_sequence,
    extract_digits,
    read_digits,
)


class TestDiffSequence:
    def test_basic_change(self, blank_frame):
        frames = [blank_frame.copy() for _ in range(5)]
        texts = ["1", "1", "2", "2", "3"]
        deltas = diff_sequence(frames, texts)
        assert len(deltas) == 5
        assert deltas[0].changed  # 初始值出现 → changed
        assert deltas[1].changed is False
        assert deltas[2].changed  # 1 → 2
        assert deltas[3].changed is False
        assert deltas[4].changed  # 2 → 3
        assert [d.text for d in deltas] == ["1", "1", "2", "2", "3"]

    def test_stable_frames(self, blank_frame):
        """stable_frames=2：文本需连续出现 2 次才计 changed。"""
        frames = [blank_frame.copy() for _ in range(6)]
        texts = ["1", "1", "1", "1", "2", "2"]
        deltas = diff_sequence(frames, texts, stable_frames=2)
        # 第一次 "1" 出现 → stable_frames=2 时第一次还不算
        # 第 2 次 "1" → 触发 stable → changed
        assert deltas[0].changed is False
        assert deltas[1].changed is True  # 第二次稳定 → changed
        # "2" 第一次出现 → 不稳定
        assert deltas[4].changed is False
        # "2" 第二次 → changed
        assert deltas[5].changed is True

    def test_none_text(self, blank_frame):
        """None/空文本不更新 stable，changed=False。"""
        frames = [blank_frame.copy() for _ in range(4)]
        texts = ["5", None, "5", "7"]
        deltas = diff_sequence(frames, texts)
        assert deltas[0].changed
        assert deltas[1].changed is False
        assert deltas[2].changed is False  # 与 stable "5" 相同
        assert deltas[3].changed  # "7" != "5"

    def test_pts_sec(self, blank_frame):
        frames = [blank_frame.copy() for _ in range(3)]
        deltas = diff_sequence(frames, ["a", "b", "c"], pts_sec=[0.5, 1.0, 1.5])
        assert deltas[0].frame.pts_sec == 0.5
        assert deltas[1].frame.pts_sec == 1.0
        assert deltas[2].frame.pts_sec == 1.5

    def test_scores(self, blank_frame):
        frames = [blank_frame.copy() for _ in range(2)]
        deltas = diff_sequence(frames, ["x", "y"], scores=[0.9, 0.8])
        assert deltas[0].score == 0.9
        assert deltas[1].score == 0.8

    def test_frame_batch(self, blank_frame):
        b = FrameBatch([blank_frame, blank_frame])
        deltas = diff_sequence(b, ["1", "2"])
        assert len(deltas) == 2

    def test_length_mismatch(self, blank_frame):
        with pytest.raises(ValueError, match="texts length"):
            diff_sequence([blank_frame, blank_frame], ["a"])

    def test_scores_mismatch(self, blank_frame):
        with pytest.raises(ValueError, match="scores length"):
            diff_sequence([blank_frame, blank_frame], ["a", "b"], scores=[1.0])


class TestOnnxDigitDecoder:
    """用 AHC hud_digits_v1.onnx 做端到端数字识别。"""

    @pytest.fixture()
    def decoder(self, hud_digits_onnx):
        return OnnxDigitDecoder(str(hud_digits_onnx))

    def test_digit_0(self, decoder, digit_0_img):
        text, score = decoder.decode(digit_0_img)
        assert isinstance(text, str)
        assert isinstance(score, float)
        assert 0.0 <= score <= 1.0

    def test_digit_6(self, decoder, digit_6_img):
        text, score = decoder.decode(digit_6_img)
        assert isinstance(text, str)

    def test_digit_8(self, decoder, digit_8_img):
        text, score = decoder.decode(digit_8_img)
        assert isinstance(text, str)

    def test_invalid_model_path(self):
        with pytest.raises(FileNotFoundError):
            OnnxDigitDecoder("nonexistent.onnx")


class TestExtractDigits:
    def test_with_decoder(self, hud_digits_onnx, digit_0_img):
        dec = OnnxDigitDecoder(str(hud_digits_onnx))
        digits, score = extract_digits(digit_0_img, decoder=dec)
        assert digits.isdigit() or digits == ""
        assert 0.0 <= score <= 1.0

    def test_roi(self, hud_digits_onnx, digit_0_img, blank_frame):
        dec = OnnxDigitDecoder(str(hud_digits_onnx))
        # digit 贴到大帧 ROI 内
        img = blank_frame.copy()
        dh, dw = digit_0_img.shape[:2]
        img[100 : 100 + dh, 200 : 200 + dw] = digit_0_img
        digits, _ = extract_digits(img, roi=Roi(200, 100, dw, dh), decoder=dec)
        assert isinstance(digits, str)


class TestReadDigits:
    def test_batch(self, hud_digits_onnx, digit_0_img, digit_6_img, digit_8_img):
        dec = OnnxDigitDecoder(str(hud_digits_onnx))
        results = read_digits(
            [digit_0_img, digit_6_img, digit_8_img],
            decoder=dec,
        )
        assert len(results) == 3
        for idx, pts, digits, score in results:
            assert isinstance(idx, int)
            assert isinstance(pts, float)
            assert isinstance(digits, str)
            assert isinstance(score, float)

    def test_pts_sec(self, hud_digits_onnx, digit_0_img):
        dec = OnnxDigitDecoder(str(hud_digits_onnx))
        results = read_digits([digit_0_img, digit_0_img], decoder=dec, pts_sec=[0.5, 1.0])
        assert results[0][1] == 0.5
        assert results[1][1] == 1.0
