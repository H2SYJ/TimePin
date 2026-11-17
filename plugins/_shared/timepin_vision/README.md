# timepin_vision — TimePin 插件视觉识别 SDK

为 TimePin 规则插件提供静态图像处理原语：**模板匹配、OCR、数字识别、序列差异检测**。

> 设计目标：插件只做"拿到帧 → 识别 → 出标注"这一步；
> 视频解码、FFmpeg 抽帧由 Rust 主程序的 frame_service 完成，
> 本包只接受 `ndarray` / `bytes` / `path` 三种输入。

## 安装

```bash
pip install plugins/_shared/timepin_vision
# 或在 requirements.txt: timepin-vision>=0.1
```

## 快速示例

```python
import numpy as np
from timepin_vision import (
    match_template, ocr_region, diff_sequence, read_digits,
    Roi, load_frame, OnnxDigitDecoder,
)

frame = load_frame("frames/00012.png")

# 1) 模板匹配 —— 多尺度 + 阈值
hits = match_template(frame.image, "assets/counter_marker.png", threshold=0.85)
if hits:
    top = hits[0]
    print(top.score, top.bbox)   # 0.93 (x, y, w, h)

# 2) OCR —— 对 ROI 识别文字
res = ocr_region(frame.image, roi=Roi(0, 0, 400, 60))
print(res.text, res.confidence)
for b in res.boxes:
    print(" ", b.text, b.confidence, b.bbox)

# 3) HUD 数字识别 —— CNN+CTC（对齐 AHC hud_digits_v1）
dec = OnnxDigitDecoder("assets/hud_digits_v1.onnx")
text, score = dec.decode(crop(frame.image, Roi(10, 10, 80, 28)))

# 4) 序列差异 —— 文本变化点
frames = [f"frames/{i:05d}.png" for i in range(200)]
texts = [ocr_region(p).text for p in frames]
deltas = diff_sequence(frames, texts, pts_sec=[i * 0.5 for i in range(200)])
for d in deltas:
    if d.changed:
        print(f"t={d.frame.pts_sec:.2f}s  → {d.text}")

# 5) 批量数字读取
results = read_digits(frames, roi=Roi(10, 10, 80, 28), decoder=dec)
for idx, pts, digits, score in results:
    print(idx, pts, digits, score)
```

## API 速查

| 函数 / 类 | 用途 |
|---|---|
| `match_template(image, template, scales, mask, threshold, method, top_k, nms_iou)` | 多尺度模板匹配，返回 `MatchResult[]` |
| `match_all(...)` | `match_template` 别名（语义更明确） |
| `ocr_region(image, roi, engine, model_path, min_confidence, upscale)` | ROI OCR，返回 `OcrResult` |
| `rec_text(...)` | 只取 `OcrResult.text` |
| `OcrEngine(det_model_path, cls_model_path, rec_model_path, providers, ...)` | 可复用的 RapidOCR 封装 |
| `OnnxDigitDecoder(model_path)` | HUD 数字 CNN+CTC 解码器（28×64 灰度输入） |
| `extract_digits(image, roi, decoder)` | 单帧数字提取 → `(digits, score)` |
| `read_digits(frames, roi, decoder, pts_sec)` | 序列数字提取 → `[(idx, pts, digits, score)]` |
| `diff_sequence(frames, texts, scores, pts_sec, stable_frames)` | 帧级文本差异 → `FrameDelta[]` |
| `load_frame(src, pts_sec, index)` / `load_frames(sources)` | 输入规整 → `VideoFrame` |
| `FrameBatch(frames)` | `Sequence[VideoFrame]` 容器 |
| `Roi(x, y, w, h)` / `crop(image, roi)` | 感兴趣区域 |
| `to_hsv` / `to_gray` / `to_binary` / `normalize_luma` | 色彩 / 亮度预处理 |
| `resize_keep_ratio` / `pad_to_aspect` / `upscale_for_ocr` | 缩放 / 对齐 |
| `perspective_fix(image, src_quad, out_size)` | 四点透视矫正 |
| `load_onnx_session(path_or_bytes, providers, ...)` | ONNX 会话加载（自动 provider 回退） |

## 设计说明

- **不做视频解码**：输入必须是已解码的 ndarray / PNG·JPG 字节 / 磁盘文件。
  视频 → 帧的管线由 TimePin Rust `frame_service` 负责，通过 JSON-RPC 传给插件。
- **不修改输入**：所有函数返回新 ndarray；`Roi.clamp` 返回新 `Roi`。
- **方法学对齐 AHC**：
  `match_template` 默认 `scales=[0.8, 0.9, 1.0, 1.1, 1.2]`，
  `OnnxDigitDecoder` 的预处理（28×64 灰度左对齐画布、归一化、CTC 贪心解码）
  直接对齐 `hud_digit_reader.py`。
- **依赖最小化**：只依赖 `opencv-python` + `onnxruntime` + `numpy` +
  `rapidocr-onnxruntime`（后三者是 ONNX Runtime 推理栈）。
- **线程安全**：无全局可变状态。`OcrEngine` / `OnnxDigitDecoder` 是
  InferenceSession 的封装，进程内可复用、跨进程不要共享。

## 与 AHC (apex-highlight-clipper) 的对应

| AHC 代码 | timepin_vision API |
|---|---|
| `counter_marker.match_frame` | `match_template(scales=[0.8,0.9,1.0,1.1,1.2])` |
| `hud_digit_reader.read_digits` | `OnnxDigitDecoder.decode` / `read_digits` |
| `weapon_reader.caption_image` | `ocr_region`（ROI 裁剪 + upscale 3x） |
| `weapon_reader.WeaponTextReader` | `OcrEngine`（RapidOCR + 自定义 rec 模型） |

## 版本

- `0.1.0` — TPIN-5 初版；API 以 issue spec 为准。

## 测试

```bash
cd plugins/_shared/timepin_vision
pip install -e .[dev]
pytest tests/
```
