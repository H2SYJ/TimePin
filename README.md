# TimePin（时签）项目介绍

## 1. 项目一句话

TimePin 是一款桌面端**视频自动打标与片段导出工具**：用户导入录制好的 MP4，软件通过**可插拔的自动打标规则**（含游戏 GUI 视觉识别）在时间轴上生成标注点/区间，并**不修改原视频**，而是生成与 MP4 同名的外部伴随文件，使 **PotPlayer 可直接读取章节**、**Premiere Pro 可导入时间线标记**，同时支持按标注区间**高效导出视频片段**。

## 2. 背景与要解决的问题

用户录制 MP4 后，需要人工反复观看并标记关键时间点（如游戏事件、操作步骤、章节节点），再交给播放器或剪辑软件使用。现有工具存在三个断裂：

- 自动打标工具产出的结果难以被 PotPlayer 和 Premiere Pro 同时“看见”；
- 不同游戏/场景的打标规则差异大，硬编码规则无法扩展；
- 片段导出若重新编码，耗时极长，无法满足批量处理。

TimePin 的目标是成为**打标规则可插拔、识别结果跨软件可见、片段导出接近零耗时**的桌面工具。

## 3. 产品定位与目标用户

- **定位**：本地优先、插件化的视频时间轴标注工作台。
- **目标用户**：游戏录像整理者、教程/直播切片制作者、需要批量标注视频并交付剪辑软件的用户。
- **非目标**：不做在线视频平台、不做云端协作、不做全功能非线性剪辑器。

## 4. 核心概念（Agent 必须理解）

- **标注（Mark）**：时间点或时间区间 + 标签名 + 可选元数据（置信度、来源插件、截图证据）。
- **外部伴随文件**：与 `video.mp4` 同名的 `video.txt`（OGM 文本章节，UTF-8，格式 `HH:MM:SS,ddd 标题`），供 PotPlayer 自动加载；同时生成 `video.csv`（Premiere 标记格式，列含 `Name, In, Out, Duration`），供 PR 导入。**默认不修改原 MP4 文件**。
- **插件（Rule Plugin）**：独立进程或受控模块，输入视频帧/音频/元数据，输出标注列表。每种游戏或场景对应一个插件。
- **GUI 识别工具**：插件可调用的视觉识别能力集合，包括模板匹配、OCR、目标检测、区域截取与调试预览。
- **片段导出**：根据标注区间调用 FFmpeg 流复制（`-c copy`）快速切分，不重新编码；仅在需要帧精确时对首尾 GOP 做极小重编码。

## 5. 功能需求

### 5.1 视频导入与媒体库
- 支持拖拽/选择导入 MP4（后续可扩展 MKV/MOV）。
- 显示缩略图、时长、分辨率、帧率；支持多文件队列。

### 5.2 自动打标与插件系统
- 插件管理：安装、启用/禁用、更新、卸载、查看日志。
- 插件形态：优先支持 **Python Sidecar 插件**（便于 CV/AI 生态）与 **Rust 编译插件**（高性能）；每个插件独立进程，故障隔离。
- 插件输入：视频路径、采样帧率/时间范围、用户参数、可调用 GUI 识别 SDK。
- 插件输出：标准 JSON 标注列表（时间戳、区间、标签、置信度、证据截图路径）。
- 支持**多插件并行/串行**执行，结果合并去重，用户可在时间轴上审阅与手动修正。
- 插件热加载：不重启主程序即可替换插件版本（至少支持重新加载）。

### 5.3 GUI 识别工具（供插件与高级用户）
- 区域选择器：在视频帧上框选 ROI，保存为模板/配置。
- 模板匹配：OpenCV 多尺度模板匹配，支持阈值与掩码。
- OCR：对指定区域做文字识别（可接 PaddleOCR/Tesseract）。
- 目标检测：可选 ONNX Runtime 加载 YOLO 等模型。
- 调试预览：逐帧/抽帧查看识别结果叠加层，输出命中时间点与截图证据。
- 识别原语以 SDK/CLI 形式暴露给插件，插件不应直接依赖 UI。

### 5.4 时间轴与标注编辑
- 可视化时间轴：显示视频缩略图轨道、标注轨道、插件执行状态。
- 标注可新增、删除、拖动、合并、重命名、改标签颜色。
- 支持按标签筛选、跳转、批量选择。
- 标注列表与时间轴双向联动。

### 5.5 外部章节与标记文件生成
- 一键生成/更新 `video.txt`（OGM 章节）与 `video.csv`（PR 标记）。
- 文件与 MP4 同目录同名；编码 UTF-8；时间格式严格符合规范。
- 可选生成 `video.xml`（FCP7 XML 标记）作为 PR 备选导入方式。
- 生成前预览文件内容；生成后提示 PotPlayer/PR 的使用方式。

### 5.6 片段导出
- 按选中标注区间或全部标注区间批量导出。
- 默认使用 FFmpeg 流复制：`ffmpeg -ss <start> -i input.mp4 -t <dur> -c copy out.mp4`。
- 提供“帧精确”选项：仅对首尾 GOP 重编码，其余流复制。
- 导出队列：进度、速度、预计剩余时间、失败重试、输出命名模板（如 `{原文件名}_{标签}_{序号}.mp4`）。
- 导出任务在后台执行，不阻塞 UI。

### 5.7 设置与体验
- 主题（浅色/深色）、语言（中/英）、FFmpeg 路径、临时目录、并发数。
- 快捷键：播放/暂停、上一/下一标注、打点、导出选中。
- 首次使用引导：导入视频 → 选择插件 → 运行 → 审阅 → 生成文件/导出。

## 6. 插件系统设计要求（Agent 重点）

- **隔离**：每个插件运行在独立 Sidecar 进程，主程序通过 stdin/stdout 或本地 JSON-RPC 通信；插件崩溃不影响主程序。
- **权限**：插件只能访问被授权的视频路径、临时目录和识别 SDK；文件写入限制在输出目录。
- **协议**：定义稳定的 `PluginManifest`（名称、版本、入口、参数 schema、能力声明）与 `PluginResult`（标注数组、日志、证据）。
- **生命周期**：发现 → 校验 → 启动 → 初始化 → 执行 → 流式进度 → 结果 → 停止/回收。
- **可扩展**：新增游戏规则 = 新增一个插件目录/包，不修改主程序。
- **调试**：插件日志集中查看；支持单帧/片段试运行。

## 7. 技术架构（首选方案）

- **外壳**：Tauri 2（Rust 主进程）。
- **前端**：React 18/19 + TypeScript + Vite；组件库使用 **HeroUI v2**（基于 React Aria，无障碍友好）+ Tailwind CSS v3；状态管理 Zustand/Redux Toolkit。
- **核心服务（Rust）**：文件管理、任务调度、插件进程管理、FFmpeg 调用、文件生成、IPC 事件。
- **计算 Sidecar**：Python（OpenCV、ONNX Runtime、OCR、ok-script 识别原语封装）或 Rust 二进制；每个插件独立进程。
- **通信**：Tauri IPC（前端 ↔ Rust）；Rust ↔ Sidecar 用 JSON-RPC/stdio；进度通过 Tauri event 推送到前端。
- **视频处理**：FFmpeg CLI（流复制导出、抽帧）；OpenCV/ONNX 在 Sidecar 内。
- **插件底座**：评估 tauron 的插件隔离与 ACL 能力；若不采用，自研基于 Sidecar 的进程级插件管理器。
- **打包**：Tauri bundler；Sidecar 以外部二进制随包分发；Python 插件可用 PyInstaller 打包或要求用户安装运行时（优先打包）。

## 8. 关键数据流

1. 用户导入 `video.mp4` → Rust 读取元数据 → 前端媒体库展示。
2. 用户选择插件并配置参数 → Rust 启动 Sidecar 插件进程 → 插件按抽帧调用 GUI 识别 SDK → 流式返回标注 → Rust 汇总 → 前端时间轴渲染。
3. 用户审阅修正 → 点击“生成伴随文件” → Rust 写 `video.txt` / `video.csv`。
4. 用户选择区间导出 → Rust 调 FFmpeg `-c copy` → 进度事件 → 输出片段。

## 9. 非功能需求与验收标准

- **性能**：1 小时 1080p 视频的 10 个片段流复制导出总耗时 < 5 秒（不含磁盘 IO 瓶颈）；UI 在插件运行期间保持可交互；内存占用合理（避免 Electron 级开销）。
- **界面**：现代化、深色友好、时间轴拖拽流畅；核心操作 3 步内完成。
- **兼容性**：Windows 10/11 优先，macOS 次之；PotPlayer 能自动加载 `video.txt`；PR 能通过导入 `video.csv` 看到标记。
- **可扩展**：新增一个游戏打标插件不需要改主程序代码。
- **可靠性**：插件崩溃可恢复；导出失败可重试；不修改原 MP4（除非用户显式选择内嵌章节）。
- **可维护**：插件协议有文档与示例插件；核心模块有单元测试。

## 10. 里程碑建议

- **M1 原型**：Tauri 2 + React 壳；导入 MP4；手动打点；生成 `video.txt` 与 `video.csv`；FFmpeg 流复制导出单片段。
- **M2 插件框架**：Sidecar 插件协议；一个示例 Python 插件（模板匹配）；插件管理 UI；时间轴展示插件结果。
- **M3 GUI 识别工具**：ROI 选择、模板匹配、OCR 调试预览；识别 SDK 供插件调用。
- **M4 多插件与批量**：多插件并行、结果合并、批量导出队列、设置与引导。
- **M5 打磨**：性能优化、主题、快捷键、打包分发、文档与示例插件。

## 11. 风险与约束

- FFmpeg 流复制只能切关键帧，需在 UI 明示“快速导出”与“帧精确”差异。
- Python Sidecar 打包体积与运行时依赖需提前验证。
- PotPlayer 外部章节文件格式必须严格 UTF-8 + `HH:MM:SS,ddd 标题`，否则不加载。
- PR 的 CSV 标记导入列名需按实际版本验证，保留 XML 备选。
- 插件生态的安全边界必须清晰，避免任意插件无限制访问文件系统。

## 12. Agent 需要产出的核心交付物

- 可运行的 Tauri 2 + React 桌面应用骨架。
- Rust 核心服务：媒体导入、任务调度、插件进程管理、文件生成、FFmpeg 导出。
- 插件协议文档 + 至少一个示例插件（含 GUI 模板匹配）。
- GUI 识别 SDK/CLI 的最小可用实现与调试界面。
- `video.txt` 与 `video.csv` 生成器及格式校验。
- 片段导出队列与进度反馈。
- 项目 README、架构说明、插件开发指南。

---

## 13. 开发快速看效果指南

### 13.1 三种运行模式对比

| 模式 | 命令 | 启动速度 | 热重载 | 适用场景 |
|------|------|----------|--------|----------|
| **前端热重载开发** | `pnpm dev` | ~3 秒 | ✅ 毫秒级 (React Fast Refresh) | 纯 UI 调整、组件开发、样式调试、页面布局 |
| **Tauri 开发模式** | `pnpm tauri dev` | ~30-60 秒 | 前端 ✅ / Rust 需重编译 | 全栈联调、IPC 测试、插件系统调试、原生 API 验证 |
| **Debug 可执行文件** | `src-tauri/target/debug/timepin.exe` | ~1 秒 | ❌ 无 | 快速验证构建产物、演示、集成测试、Rust 逻辑验证 |

### 13.2 推荐日常开发流程

```bash
# 终端 1：启动前端热重载（主力开发，90% 场景）
pnpm dev
# → 浏览器打开 http://localhost:5173
# → 保存代码自动热更新，保持交互状态不丢失

# 终端 2：需要 Rust 改动时，增量编译 + 运行
cd src-tauri && cargo build  # 增量编译约 5-15 秒
../src-tauri/target/debug/timepin.exe
```

### 13.3 关键文件修改后的生效方式

| 文件类型 | 示例路径 | 生效方式 |
|----------|----------|----------|
| **前端组件/页面/Hooks** | `src/features/**/*.tsx` | `pnpm dev` 自动热重载 |
| **样式** | `src/**/*.css`, `tailwind.config.js` | `pnpm dev` 自动热重载 |
| **类型定义** | `src/types/index.ts` | `pnpm dev` 自动重载 (可能需重启 TS 服务) |
| **Rust 核心逻辑/命令/IPC** | `src-tauri/src/**/*.rs` | `cargo build` + 重启 `timepin.exe` |
| **Rust 依赖** | `src-tauri/Cargo.toml` | `cargo build` (首次较慢，后续增量) |
| **Tauri 配置** | `src-tauri/tauri.conf.json` | `cargo build` + 重启 `timepin.exe` |

### 13.4 常用调试命令速查

```bash
# ===== 前端 =====
pnpm dev                    # 启动 Vite 开发服务器 (http://localhost:5173)
pnpm build                  # 生产构建检查 (tsc + vite build)
pnpm test                   # 运行前端单元测试
pnpm lint                   # ESLint 检查
pnpm format                 # Prettier 格式化

# ===== Rust 后端 =====
cd src-tauri
cargo check                 # 快速类型检查 (不生成二进制，最快)
cargo build                 # Debug 构建 (增量编译快)
cargo build --release       # Release 构建 (优化后，用于性能测试)
cargo test                  # 运行 Rust 单元/集成测试
cargo tauri dev             # Tauri 全栈开发模式 (前端热重载 + Rust)
cargo tauri build           # 生成安装包

# ===== 完整应用运行 =====
./src-tauri/target/debug/timepin.exe      # Debug 版本 (含调试符号)
./src-tauri/target/release/timepin.exe    # Release 版本 (最优性能)

# ===== 清理缓存 =====
cargo clean                 # 清理 Rust 构建缓存
pnpm store prune            # 清理 pnpm 存储缓存
rm -rf node_modules dist    # 清理前端依赖和构建产物
```

### 13.5 调试技巧

| 场景 | 方法 |
|------|------|
| **看 UI 变化** | `pnpm dev` + 浏览器 `http://localhost:5173`，保存即刷新 |
| **调试前端逻辑** | 浏览器 F12 开发者工具 → Console/Sources/React DevTools |
| **调试 IPC/后端逻辑** | `pnpm tauri dev` 或运行 `timepin.exe`，F12 查看 Console 与 Network |
| **调试 Rust 代码** | VS Code 安装 `rust-analyzer` + `CodeLLDB`，Attach 到 `timepin.exe` 进程断点调试 |
| **查看 Rust 日志** | 终端直接显示 `println!`/`log::info!` 输出；或设置 `RUST_LOG=debug` |
| **验证构建产物** | `pnpm tauri build` 生成 `src-tauri/target/release/bundle/` 下的安装包 |

### 13.6 端口冲突处理

```bash
# 如果遇到 "Port 5173 is already in use"
# Windows:
netstat -ano | findstr 5173
taskkill /PID <PID> /F

# Linux/macOS:
lsof -i :5173
kill -9 <PID>
```

---

> 💡 **核心原则**：90% 的 UI 开发只需 `pnpm dev` + 浏览器，保存即刷新，毫秒级反馈。  
> 只有涉及 **Rust 侧逻辑（IPC、文件操作、插件管理、FFmpeg 调用）** 时才需要 `cargo build` + 运行 `timepin.exe`。
