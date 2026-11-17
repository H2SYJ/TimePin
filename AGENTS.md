# TimePin Project - AGENTS.md

## 项目概述
TimePin（时签）是本地优先、插件化的视频时间轴标注工作台：导入录制好的 MP4，通过可插拔的自动打标规则（含游戏 GUI 视觉识别）在时间轴上生成标注点/区间；不修改原视频，而是生成同名外部伴随文件，使 PotPlayer 可直接读取章节、Premiere Pro 可导入时间线标记，并按标注区间用 FFmpeg 流复制高效导出片段。

## 核心技术栈（不可随意替换）
- **外壳**：Tauri 2（Rust 主进程）
- **前端**：React 18/19 + TypeScript + Vite；**HeroUI v2 + Tailwind CSS v3**；Zustand 状态管理
- **Rust 核心**：媒体元数据读取、任务调度、插件进程管理、FFmpeg 调用、伴随文件生成、IPC 事件
- **计算 Sidecar**：Python（OpenCV、ONNX Runtime、PaddleOCR/Tesseract）优先，Rust 二进制插件次之
- **通信**：前端↔Rust 用 Tauri IPC；Rust↔Sidecar 用 JSON-RPC over stdio；进度通过 Tauri event 推送
- **打包**：Tauri bundler；Sidecar 以 externalBin 随包分发；Python 插件优先 PyInstaller 打包

## 项目结构规范
```
timepin/
├── src/                    # React 前端源码
│   ├── components/         # 通用 UI 组件（HeroUI 语义封装，集中在 components/ui）
│   ├── features/           # 业务功能模块
│   │   ├── settings/       # 设置页面
│   │   ├── library/        # 媒体库/视频导入
│   │   └── timeline/       # 时间轴标注（后续）
│   ├── hooks/              # 自定义 hooks
│   ├── lib/                # 工具库
│   ├── stores/             # Zustand stores
│   ├── types/              # TypeScript 类型定义（与 Rust serde 结构一一对应）
│   └── App.tsx
├── src-tauri/              # Tauri/Rust 后端
│   ├── src/
│   │   ├── core/           # 核心业务逻辑
│   │   ├── plugins/        # 插件系统
│   │   ├── ffmpeg/         # FFmpeg 封装
│   │   ├── formats/        # 伴随文件生成器
│   │   ├── commands/       # Tauri 命令
│   │   ├── ipc/            # IPC 事件定义
│   │   └── main.rs
│   ├── Cargo.toml
│   └── tauri.conf.json
├── plugins/                # 插件目录（运行时发现）
│   └── example-plugin/
├── docs/
│   ├── architecture.md
│   └── plugin-protocol.md
├── AGENTS.md               # 本文件
└── README.md
```

## 代码规范

### Rust
- `cargo fmt` + `clippy` 零警告
- 错误处理使用 `thiserror` / `anyhow`
- 异步使用 `tokio`
- 按 `core/`、`plugins/`、`ffmpeg/`、`formats/` 分层
- 所有对外协议与事件 payload 必须版本化，附 JSON Schema 和文档

### TypeScript
- 严格模式 (`strict: true`)
- 禁止滥用 `any`，必要时用 `unknown` + 类型收窄
- IPC 类型与 Rust serde 结构一一对应，集中在 `src/types/`
- 组件使用函数式组件 + hooks

### UI 设计规范
- **简洁精致 + 圆角为主**
- **禁止堆各种阴影**（box-shadow 仅在极少数交互态使用，如 dropdown、tooltip）
- 使用 **HeroUI v2**（`@heroui/react`，基于 React Aria）+ Tailwind CSS v3；统一通过 `src/components/ui` 的封装组件调用，业务代码不直接散落使用 HeroUI 原生 props
- 语义色板使用 HeroUI 设计 token：`bg-background`、`text-foreground`、`text-default-400/500`、`bg-default-50/100`、`border-divider`、`text-primary`；旧 shadcn 变量（`muted-foreground`、`secondary`、`accent` 等）仅保留在 `index.css` 作兼容别名，新代码一律使用 default/foreground/divider 语义类
- 交互 props 遵循 React Aria 习惯：`onPress`/`onValueChange`/`isDisabled`/`selectedKeys`，不再用原生 `onClick`/`onChange`/`disabled` 驱动 HeroUI 组件
- 主色调：中性灰 + 单一品牌色（蓝色系 `primary`）
- 圆角：`rounded-lg` (8px) 为基础，卡片 `rounded-xl` (12px)，按钮 `rounded-md` (6px)
- 间距：基于 4px 网格（`gap-4`、`p-4`、`m-4` 等）
- 字体：统一 `JetBrains Mono`，中文优先 `font-sans`
- **页面骨架统一**：页面根 = `flex h-full min-h-0 flex-col`；页头 = `border-b border-divider px-8 py-5`（左 h1+描述、右操作区）；主体 = `scrollbar-thin overflow-y-auto px-8 py-6` + `space-y-5`。禁止自造页面骨架。
- **表单布局统一**：分组用 `Section`（Card 容器 + 标题/描述 + 可选 `trailing`），字段行用 `Field`（`sm:grid-cols-[180px_1fr]` 标签-控件栅格）。控件只允许 `components/ui` 封装件，禁止原生 `input/select/button` 与 shadcn 旧色板（`muted-foreground`/`secondary`/`accent`）。

## 铁律（不可违反）
1. **默认绝不修改原 MP4**；只有用户显式选择「内嵌章节」时才允许写回，且必须先提示备份
2. **video.txt** 必须 UTF-8（无 BOM），每行严格为 `HH:MM:SS,ddd 标题`（逗号分隔、三位毫秒），首个章节从 `00:00:00,000` 开始；生成器必须附带格式校验与 golden file 单元测试
3. **video.csv** 列名固定为 `Name,In,Out,Duration`，供 Premiere Pro 导入；同时保留 video.xml 作为备选导入方式
4. **导出默认使用 FFmpeg 流复制 `-c copy`**，禁止全量重编码；「帧精确」选项只允许对首尾 GOP 做极小重编码。UI 必须明示两种模式的差异与代价
5. **插件必须运行在独立 Sidecar 进程**，通过 stdio/JSON-RPC 与主程序通信，崩溃不得影响主程序；插件写权限限制在授权输出目录

## v0.0.1 里程碑范围（M1 原型）
- [ ] Tauri 2 + React + TypeScript 项目初始化
- [ ] shadcn/ui + Tailwind CSS 配置
- [ ] **软件设置页面**（设置持久化、基础配置项）
- [ ] **视频导入与媒体库**（拖拽导入、视频列表、缩略图、元数据读取）
- [ ] 基础 IPC 通信框架（前端↔Rust）
- [ ] 伴随文件生成器雏形（video.txt / video.csv 生成接口）
- [ ] 单元测试框架配置

## 开发流程
1. 接到任务先阅读现有代码、README、docs/architecture.md、docs/plugin-protocol.md
2. 给出简短实现计划，然后小步实现
3. 每次完成交付说明：改动文件、验证方式与结果、已知限制与后续建议
4. 遇到产品决策给出 2-3 个选项并推荐其一，不自行猜测
5. 每次交付前尽量运行 `cargo check` / `cargo test` 与 `pnpm build`；无法验证时必须明确说明未验证及原因
6. 不引入未经讨论的重量级依赖；不修改与任务无关的文件

## Git 提交规范
- 使用 Conventional Commits：`feat:`、`fix:`、`docs:`、`refactor:`、`test:`、`chore:`
- 提交信息用英文，格式：`type(scope): description`

## 验收标准（M1）
- 项目能正常 `cargo tauri dev` 启动
- 设置页面可保存/加载配置（持久化到本地）
- 可拖拽导入 MP4 视频，媒体库显示缩略图、时长、分辨率等元数据
- 点击视频项能打开基础详情/播放预览
- 无阴影堆砌，UI 圆角统一，视觉简洁精致