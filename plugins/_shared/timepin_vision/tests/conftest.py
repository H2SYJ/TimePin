"""pytest fixtures：合成图、AHC 真数据。
"""
from __future__ import annotations

from pathlib import Path

import cv2
import numpy as np
import pytest


FIXTURES = Path(__file__).parent / "fixtures"


@pytest.fixture(scope="session")
def fixtures_dir() -> Path:
    return FIXTURES


@pytest.fixture()
def hud_digits_onnx() -> Path:
    return FIXTURES / "hud_digits_v1.onnx"


@pytest.fixture()
def damage_marker() -> Path:
    return FIXTURES / "damage-marker.png"


@pytest.fixture()
def elimination_template() -> Path:
    return FIXTURES / "elimination-tc.png"


@pytest.fixture()
def elimination_mask() -> Path:
    return FIXTURES / "elimination-tc-mask.png"


@pytest.fixture()
def blank_frame() -> np.ndarray:
    """1920x1080 黑色 BGR 帧。"""
    return np.zeros((1080, 1920, 3), dtype=np.uint8)


@pytest.fixture()
def noise_frame() -> np.ndarray:
    """有噪声的 1080p 帧（模拟游戏画面）。"""
    rng = np.random.default_rng(42)
    img = rng.integers(30, 90, (1080, 1920, 3), dtype=np.uint8)
    return img


@pytest.fixture()
def frame_with_marker(noise_frame, damage_marker) -> tuple[np.ndarray, tuple[int, int, int, int]]:
    """把 damage-marker 模板贴到 noise_frame 的固定位置，返回 (帧, ground_truth_bbox)。"""
    tpl = cv2.imread(str(damage_marker))
    th, tw = tpl.shape[:2]
    x, y = 800, 500  # 屏幕中部
    img = noise_frame.copy()
    img[y : y + th, x : x + tw] = tpl
    return img, (x, y, tw, th)


@pytest.fixture()
def digit_0_img() -> np.ndarray:
    return cv2.imread(str(FIXTURES / "digit-0.png"))


@pytest.fixture()
def digit_6_img() -> np.ndarray:
    return cv2.imread(str(FIXTURES / "digit-6.png"))


@pytest.fixture()
def digit_8_img() -> np.ndarray:
    return cv2.imread(str(FIXTURES / "digit-8.png"))
