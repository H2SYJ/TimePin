"""静态帧访问：文件路径 / PNG·JPG 字节 → BGR ndarray。

只处理**静态图像**——不负责视频解码；视频抽帧由主程序的
frame_service / FFmpeg image2 管线完成，插件拿到的就是文件或字节。
"""
from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, Iterator, List, Optional, Sequence, Union

import cv2
import numpy as np

# 单帧像素上限（防 OOM）：1920*1080*3 ≈ 6.2 MB；设 8000 万像素足够余量
MAX_FRAME_PIXELS = 80_000_000

# 一张「静态帧」的输入：BGR/GRAY ndarray、PNG/JPG 字节流、或磁盘文件路径。
FrameSource = Union[np.ndarray, bytes, bytearray, str, os.PathLike]


@dataclass
class VideoFrame:
    """一帧静态图像及其元数据。

    Attributes:
        image: BGR ndarray (H, W, 3) uint8；灰度输入会自动转 3 通道。
        path:  原始文件路径（来自 bytes/ndarray 时为 None）。
        pts_sec: 帧在视频中的时间戳（秒）；未知为 None。由调用方在
                 load_frames 时或通过构造函数传入。
        index: 帧序号（通常对应抽帧顺序）。
    """

    image: np.ndarray
    path: Optional[Path] = None
    pts_sec: Optional[float] = None
    index: int = -1

    @property
    def width(self) -> int:
        return int(self.image.shape[1])

    @property
    def height(self) -> int:
        return int(self.image.shape[0])


def _check_image(img: np.ndarray, *, name: str = "image") -> np.ndarray:
    """规整输入：接受 BGR / BGRA / 灰度，返回 BGR (uint8)。"""
    if img is None:
        raise ValueError(f"{name} is None")
    if not isinstance(img, np.ndarray):
        raise TypeError(f"{name} must be numpy.ndarray, got {type(img).__name__}")
    if img.dtype != np.uint8:
        if img.dtype in (np.float32, np.float64):
            # 容忍浮点 [0,1] / [0,255]，归一化到 uint8
            max_v = float(img.max()) if img.size else 0.0
            scale = 255.0 if max_v <= 1.0 else 1.0
            img = np.clip(img * scale, 0, 255).astype(np.uint8)
        else:
            img = img.astype(np.uint8)
    if img.ndim == 2:
        return cv2.cvtColor(img, cv2.COLOR_GRAY2BGR)
    if img.ndim == 3 and img.shape[2] == 3:
        return img
    if img.ndim == 3 and img.shape[2] == 4:
        return cv2.cvtColor(img, cv2.COLOR_BGRA2BGR)
    raise ValueError(f"{name} has unsupported shape {img.shape}")


def _check_size(img: np.ndarray) -> np.ndarray:
    h, w = img.shape[:2]
    if h <= 0 or w <= 0:
        raise ValueError("image has empty dimension")
    if h * w > MAX_FRAME_PIXELS:
        raise ValueError(
            f"image too large: {w}x{h} ({h * w} px > {MAX_FRAME_PIXELS})"
        )
    return img


def decode_frame(data: bytes | bytearray, *, flags: int = cv2.IMREAD_COLOR) -> np.ndarray:
    """把 PNG/JPEG 字节解码为 BGR ndarray。

    Args:
        data: 图像字节（PNG / JPEG / WebP / BMP 等 OpenCV 支持格式）。
        flags: `cv2.imdecode` 的 flags，默认强制 3 通道 BGR。

    Raises:
        ValueError: 解码失败 / 图像为空 / 像素超限。
    """
    if not data:
        raise ValueError("empty frame bytes")
    arr = np.frombuffer(data, dtype=np.uint8)
    img = cv2.imdecode(arr, flags)
    if img is None:
        raise ValueError("cv2.imdecode failed (not a valid image?)")
    img = _check_image(img, name="decoded")
    return _check_size(img)


def load_frame(
    source: FrameSource,
    *,
    pts_sec: Optional[float] = None,
    index: int = -1,
    flags: int = cv2.IMREAD_COLOR,
) -> VideoFrame:
    """把一种输入统一为 `VideoFrame`。

    Args:
        source: ndarray / bytes / 文件路径。
        pts_sec / index: 透传到 `VideoFrame` 的元数据。
        flags: `cv2.imread` / `cv2.imdecode` 标志。
    """
    if isinstance(source, np.ndarray):
        img = _check_image(source)
        _check_size(img)
        return VideoFrame(image=img, path=None, pts_sec=pts_sec, index=index)

    if isinstance(source, (bytes, bytearray)):
        img = decode_frame(bytes(source), flags=flags)
        return VideoFrame(image=img, path=None, pts_sec=pts_sec, index=index)

    path = Path(source)
    if not path.is_file():
        raise FileNotFoundError(f"frame not found: {path}")
    img = cv2.imread(str(path), flags)
    if img is None:
        raise ValueError(f"cv2.imread failed: {path}")
    img = _check_image(img, name=str(path))
    _check_size(img)
    return VideoFrame(image=img, path=path, pts_sec=pts_sec, index=index)


def load_frames(
    sources: Iterable[FrameSource],
    *,
    pts_list: Optional[Sequence[Optional[float]]] = None,
    start_index: int = 0,
) -> List[VideoFrame]:
    """批量加载。`pts_list` 长度与 sources 一致或为 None。"""
    out: List[VideoFrame] = []
    src_list = list(sources)
    if pts_list is not None and len(pts_list) != len(src_list):
        raise ValueError(
            f"pts_list length {len(pts_list)} != sources length {len(src_list)}"
        )
    for i, src in enumerate(src_list):
        pts = pts_list[i] if pts_list else None
        out.append(load_frame(src, pts_sec=pts, index=start_index + i))
    return out


class FrameBatch:
    """一小组 `VideoFrame`（diff_sequence / 模型推理的批量输入）。

    实现 Sequence 协议：len / iter / getitem / slices。
    """

    def __init__(self, frames: Iterable[Union[VideoFrame, FrameSource]]):
        self._frames: List[VideoFrame] = []
        for f in frames:
            self._frames.append(
                f if isinstance(f, VideoFrame) else load_frame(f)
            )

    def __len__(self) -> int:
        return len(self._frames)

    def __iter__(self) -> Iterator[VideoFrame]:
        return iter(self._frames)

    def __getitem__(self, i):
        return self._frames[i]

    @property
    def frames(self) -> List[VideoFrame]:
        return list(self._frames)

    def images(self) -> List[np.ndarray]:
        return [f.image for f in self._frames]

    def pts(self) -> List[Optional[float]]:
        return [f.pts_sec for f in self._frames]
