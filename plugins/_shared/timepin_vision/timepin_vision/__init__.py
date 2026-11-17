"""timepin_vision — TimePin 插件共享视觉识别 SDK。

给 TimePin 规则插件提供一组稳定、无副作用的计算原语：

- `match_template` / `match_all` —— 多尺度模板匹配（静态图像）
- `ocr_region` / `rec_text` —— OCR（RapidOCR + ONNX Runtime）
- `diff_sequence` —— 帧序列文本差异检测（纯函数）
- `read_digits` / `extract_digits` —— HUD 数字识别（CNN+CTC ONNX）
- `load_frame` / `load_frames` / `FrameBatch` —— 帧输入抽象（ndarray/路径/bytes）
- `Roi` / `crop` / `to_hsv` / `to_gray` / `to_binary` / `normalize_luma` /
  `resize_keep_ratio` / `pad_to_aspect` / `upscale_for_ocr` / `perspective_fix`
  —— 预处理工具
- `load_onnx_session` / `providers_available` / `DEFAULT_PROVIDERS`
  —— ONNX Runtime 会话管理

插件侧典型用法::

    from timepin_vision import match_template, ocr_region, diff_sequence, Roi

    hits = match_template(frame, "assets/marker.png", threshold=0.85)
    text = ocr_region(frame, roi=Roi(0, 0, 200, 40)).text
    deltas = diff_sequence(frames, [d.text for d in results])

设计约束（对齐 TimePin 插件协议）：

- 不修改输入图像；所有函数返回新对象。
- 不做视频解码 —— 帧来源由插件通过 TimePin Rust frame_service 提供，
  本包只接受 ndarray / 文件路径 / bytes。
- 不依赖 TimePin Rust 侧类型；插件自行完成 JSON-RPC 反序列化。
- 线程安全：无全局可变状态；`OcrEngine` / `OnnxDigitDecoder` 实例
  可在同一进程内复用，但不要跨进程共享。
"""

from .frames import (
    FrameBatch,
    FrameSource,
    VideoFrame,
    decode_frame,
    load_frame,
    load_frames,
)
from .match import MatchResult, match_all, match_template
from .model import DEFAULT_PROVIDERS, load_onnx_session, providers_available
from .ocr import OcrBox, OcrEngine, OcrResult, ocr_region, rec_text
from .preprocess import (
    CropRoi,
    Roi,
    crop,
    normalize_luma,
    pad_to_aspect,
    perspective_fix,
    resize_keep_ratio,
    to_binary,
    to_gray,
    to_hsv,
    upscale_for_ocr,
)
from .sequence import (
    DigitDecoder,
    FrameDelta,
    OnnxDigitDecoder,
    diff_sequence,
    extract_digits,
    read_digits,
)

__version__ = "0.1.0"

__all__ = [
    # frames
    "FrameBatch",
    "FrameSource",
    "VideoFrame",
    "decode_frame",
    "load_frame",
    "load_frames",
    # match
    "MatchResult",
    "match_all",
    "match_template",
    # model
    "DEFAULT_PROVIDERS",
    "load_onnx_session",
    "providers_available",
    # ocr
    "OcrBox",
    "OcrEngine",
    "OcrResult",
    "ocr_region",
    "rec_text",
    # preprocess
    "CropRoi",
    "Roi",
    "crop",
    "normalize_luma",
    "pad_to_aspect",
    "perspective_fix",
    "resize_keep_ratio",
    "to_binary",
    "to_gray",
    "to_hsv",
    "upscale_for_ocr",
    # sequence
    "DigitDecoder",
    "FrameDelta",
    "OnnxDigitDecoder",
    "diff_sequence",
    "extract_digits",
    "read_digits",
]
