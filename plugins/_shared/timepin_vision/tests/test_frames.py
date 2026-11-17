"""frames 模块单元测试。"""
from __future__ import annotations

import io

import cv2
import numpy as np
import pytest

from timepin_vision import (
    FrameBatch,
    VideoFrame,
    decode_frame,
    load_frame,
    load_frames,
)


def _png_bytes(img: np.ndarray) -> bytes:
    ok, buf = cv2.imencode(".png", img)
    assert ok
    return buf.tobytes()


class TestLoadFrame:
    def test_from_ndarray_bgr(self, blank_frame):
        f = load_frame(blank_frame)
        assert f.image.shape == (1080, 1920, 3)
        assert f.path is None
        assert f.index == -1
        assert f.pts_sec is None

    def test_from_ndarray_gray(self):
        gray = np.zeros((100, 200), dtype=np.uint8)
        f = load_frame(gray)
        assert f.image.shape == (100, 200, 3)  # 自动转 BGR

    def test_from_ndarray_bgra(self):
        bgra = np.zeros((50, 60, 4), dtype=np.uint8)
        f = load_frame(bgra)
        assert f.image.shape == (50, 60, 3)

    def test_from_path(self, tmp_path):
        img = np.random.default_rng(0).integers(0, 255, (80, 120, 3), dtype=np.uint8)
        p = tmp_path / "x.png"
        cv2.imwrite(str(p), img)
        f = load_frame(str(p))
        assert f.path == p
        assert f.image.shape == (80, 120, 3)

    def test_from_bytes(self, tmp_path):
        img = np.random.default_rng(0).integers(0, 255, (40, 60, 3), dtype=np.uint8)
        f = load_frame(_png_bytes(img))
        assert f.image.shape == (40, 60, 3)
        assert f.path is None

    def test_pts_and_index(self, blank_frame):
        f = load_frame(blank_frame, pts_sec=1.5, index=42)
        assert f.pts_sec == 1.5
        assert f.index == 42

    def test_invalid_source(self, tmp_path):
        with pytest.raises(FileNotFoundError):
            load_frame(tmp_path / "nope.png")

    def test_invalid_bytes(self):
        with pytest.raises(ValueError):
            load_frame(b"not an image")

    def test_image_too_large(self):
        # 模拟超大图：MAX_FRAME_PIXELS = 80M，构造 10000x9000 = 90M
        big = np.zeros((9000, 10000, 3), dtype=np.uint8)
        with pytest.raises(ValueError, match="too large"):
            load_frame(big)


class TestLoadFrames:
    def test_sequence(self, blank_frame):
        fs = load_frames([blank_frame, blank_frame, blank_frame], start_index=10)
        assert len(fs) == 3
        assert [f.index for f in fs] == [10, 11, 12]

    def test_pts_list(self, blank_frame):
        fs = load_frames([blank_frame, blank_frame], pts_list=[0.5, 1.0])
        assert fs[0].pts_sec == 0.5
        assert fs[1].pts_sec == 1.0

    def test_pts_list_mismatch(self, blank_frame):
        with pytest.raises(ValueError, match="pts_list length"):
            load_frames([blank_frame, blank_frame], pts_list=[1.0])


class TestFrameBatch:
    def test_container(self, blank_frame):
        b = FrameBatch([blank_frame, blank_frame])
        assert len(b) == 2
        assert isinstance(b[0], VideoFrame)
        assert b.images()[0].shape == blank_frame.shape

    def test_from_mixed(self, blank_frame, tmp_path):
        p = tmp_path / "f.png"
        cv2.imwrite(str(p), blank_frame)
        b = FrameBatch([blank_frame, str(p), _png_bytes(blank_frame)])
        assert len(b) == 3
        assert all(isinstance(f, VideoFrame) for f in b)
