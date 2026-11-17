# M3 实现计划：GUI 识别工具

## 1. Python 识别 Sidecar (`plugins/recognition/`)
- `recognition.py` - 核心识别服务（模板匹配、OCR、目标检测、ROI 管理）
- `main.py` - JSON-RPC 入口，暴露 `recognize_template`、`recognize_ocr`、`detect_objects`、`select_roi`、`preview_frame` 等方法
- `requirements.txt` - 依赖：opencv-python, onnxruntime, paddleocr(可选), numpy

## 2. 插件协议扩展 (`src-tauri/src/plugins/protocol.rs`)
- 新增 `RecognitionRequest` / `RecognitionResponse` 类型
- 新增方法：`recognize.template_match`、`recognize.ocr`、`recognize.detect`、`recognize.preview`、`recognize.roi_select`

## 3. Rust 识别客户端 (`src-tauri/src/plugins/recognition.rs`)
- `RecognitionClient` - 连接 Python Sidecar，提供类型安全的调用接口
- 集成到 `PluginManager`，插件可通过 `params.recognition` 调用

## 4. 前端组件 (`src/features/recognition/`)
- `RoiSelector.tsx` - 视频帧上框选 ROI，返回归一化坐标
- `RecognitionPreview.tsx` - 调试预览：叠加识别框、置信度、逐帧播放
- `RecognitionPanel.tsx` - 统一面板：参数配置、运行、结果列表

## 5. 演示插件 (`plugins/template-match-ocr/`)
- 真实的模板匹配 + OCR 插件
- 使用识别 SDK 调用 Sidecar

## 6. 测试
- Python 端单元测试（模板匹配精度、OCR 准确率）
- Rust 端集成测试（Sidecar 启动、调用、关闭）
- 前端 ROI 选择器交互测试

---

## 实现顺序

1. [ ] Python 识别 Sidecar 核心代码
2. [ ] 插件协议扩展 + Rust 客户端
3. [ ] 前端 ROI 选择器 + 预览组件
4. [ ] 演示插件
5. [ ] 集成测试与验收