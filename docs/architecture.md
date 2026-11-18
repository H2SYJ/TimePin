# TimePin 架构说明

> 状态：M5（打磨阶段）。本文档描述当前代码的实际架构，已标注实现缺口。

## 1. 分层架构

```
┌──────────────────────────────────────────────┐
│ React 18 + TypeScript + Vite + HeroUI        │  ← 前端（src/）
│ Zustand store │ Tailwind CSS v3              │
├────── Tauri IPC（invoke + event） ────────────┤
│ Rust 主进程（src-tauri/src/）                │
│ commands/ 薄壳 → core/ + formats/ + ffmpeg/  │
│ plugins/manager.rs → plugins/runtime.rs      │
├────── JSON-RPC over stdio ────────────────────┤
│ Python Sidecar 插件（plugins/）               │  ← 独立进程
│ OpenCV / OCR / 目标检测 / 模板匹配            │
└──────────────────────────────────────────────┘
```

## 2. Rust 模块职责

`src-tauri/src/` 按层组织：

| 模块 | 文件 | 职责 |
|---|---|---|
| `commands/` | `settings.rs` `library.rs` `formats.rs` `plugins.rs` | 14 个 `#[tauri::command]` 薄壳，委托给核心层；阻塞调用已包 `spawn_blocking` |
| `core/` | `settings.rs` `library.rs` `mp4.rs` `tools.rs` | 设置持久化、媒体库 CRUD、MP4 元数据降级解析、FFmpeg/FFprobe 路径解析 |
| `core/github/` | `mod.rs` `manifest_cache.rs` | GitHub 共享会话、凭据与分桶限流；`plugins-cache.json` 中按身份隔离持久化 manifest（1 小时软过期 / 30 天硬过期 / ETag 重验证），启动清理；详见 [GitHub API 文档](github-api.md) |
| `formats/` | `mod.rs` | `video.txt`（OGM）/ `video.csv`（PR 标记）/ `video.xml`（FCP7）生成器 + golden file 测试 |
| `ffmpeg/` | `mod.rs` | `export_segments`：流复制 `-c copy` 导出片段 |
| `plugins/` | `manifest.rs` `manager.rs` `protocol.rs` `runtime.rs` | 插件清单校验、进程生命周期管理、JSON-RPC 协议、Sidecar spawn/stdio 通信 |
| `plugins/package.rs` | `package.rs` | 本地 `.tppkg` / `.zip` 包校验、安全有界解压、暂存与覆盖恢复；`commands/plugins.rs::import_plugin` 通过版本化 IPC 暴露 |
| `commands/updater.rs` | `updater.rs` | `get_updater_config`：返回版本化的更新器配置状态（schemaVersion/configured），由 Rust 侧读取合并后的插件 pubkey |
| `ipc/` | `mod.rs` | `ImportProgressEvent`/`ExportProgressEvent` schema 定义（`SCHEMA_VERSION = "1.0.0"`） |
| `lib.rs` | — | Tauri Builder：注册 14 个命令 + `PluginManager` setup |

### 已注册的 IPC 命令

`get_settings` `save_settings` `import_video` `get_videos` `delete_video` `get_video_metadata` `generate_thumbnail` `generate_txt_chapters` `generate_csv_markers` `export_segments` `get_plugins` `reload_plugins` `run_plugin` `stop_plugin` `cancel_plugin_run`

## 3. 前端结构

```
src/
├── App.tsx                    ← 路由（lazy 三个页面）+ 主题/快捷键初始化
├── components/Sidebar.tsx     ← 侧栏导航 + 快捷键提示
├── features/
│   ├── library/
│   │   ├── LibraryPage.tsx    ← 媒体库页（拖拽/选择导入 + 视频列表）
│   │   ├── PluginsPage.tsx    ← 插件管理页（列表 + 运行 + 日志）
│   │   ├── VideoCard.tsx      ← memo 包装，lazy 缩略图
│   │   └── VideoDetail.tsx    ← 选中视频详情 + 进入标注工作台
│   ├── marks/                ← 标注工作台、区间选择与片段导出
│   └── settings/
│       └── SettingsPage.tsx   ← 设置页（完整表单）
├── hooks/useKeyboardShortcuts.ts
├── lib/
│   ├── ipc.ts                 ← api.* 封装 invoke
│   ├── shortcuts.ts           ← getShortcutAction() 纯函数
│   ├── theme.ts               ← applyTheme() + listenToSystemTheme()
│   └── utils.ts               ← cn() 等
├── stores/app.ts              ← Zustand：settings/videos/page/importing 状态
├── types/index.ts             ← Settings/VideoInfo/PluginInfo/Mark/ExportMode 等
└── index.css                  ← Tailwind + 主题变量（无远程字体 import）
```

**状态模型**：单 Zustand store（`useAppStore`），字段：`page` `settings` `videos` `selectedVideoId` `loading` `importing` `error` `notice`。三个页面通过 `setPage` 切换，`selectedVideoId` 选中视频驱动详情面板。

**主题**：`lib/theme.ts` 封装 `applyTheme(theme)`——切换 `<html>` 的 `dark` class + `color-scheme` + `<meta name="theme-color">`；`listenToSystemTheme()` 在 `settings.theme === 'system'` 时监听 `matchMedia` 变化实时跟随系统。

**快捷键**：`lib/shortcuts.ts` 的 `getShortcutAction(event)` 是纯函数（可测），`hooks/useKeyboardShortcuts.ts` 绑全局 `keydown`。当前硬编码 4 个：`Ctrl/Cmd+O` 导入、`Ctrl/Cmd+,` 设置、`1`/`2`/`3` 切换三个页面。

## 4. 三条数据流链

### 4.1 导入

```
用户拖文件 → importPaths(paths) → 有界并发（maxConcurrentImports，默认 3）
  → invoke("import_video") → spawn_blocking →
    fs::copy 到 videos/ → ffprobe 元数据 → ffmpeg 缩略图 → index.json
→ refreshVideos → UI 更新
```

### 4.2 插件执行

```
PluginsPage 选插件+视频 → invoke("run_plugin") → spawn_blocking →
  manager.ensure_process（懒启动/崩溃自动重启）→
  plugin.run() 在锁外执行（Arc<Mutex<PluginProcess>>，cancel 可达）→
  JSON-RPC 通知 → handle_notification → emit plugin://progress|log|result
→ 前端 listen 更新 UI
```

### 4.3 导出

```
MarksPage 勾选有效区间 → Tauri 选择输出目录 → invoke("export_segments") → spawn_blocking →
  ffmpeg/mod.rs → 每段 Command::new(ffmpeg).args(-ss -t -c copy).output()
→ 返回 Vec<PathBuf> → 工作台展示实际输出文件路径
```

导出模式复用 `settings.defaultExportMode`；帧精确模式目前明确拒绝，不进行全量重编码。
前端显示导出中禁用态（`ExportProgressEvent` 尚未 emit），失败保留选择便于重试。
选择/输出目录不新增持久化配置，详细行为与覆盖、部分失败限制见 [手动标注文档](manual-marks.md#从标注工作台导出片段)。

## 5. 插件生命周期

本地插件包通过插件页按钮或拖放导入，经 `import_plugin` 在阻塞线程内读取、校验和安装到第一个已保存的自定义插件目录；没有目录时引导设置而非写入内置资源。校验成功后安全替换同名目录，调用 `PluginManager::reload` 并刷新列表。详见[本地插件包导入](plugin-import.md)。

```
discovered → validated → started → ready → running → stopped | crashed
                                    ↑        │
                                    └────────┘ (循环)
```

- `manager.rs` 的 `PluginManager` 持有 `Arc<Mutex<HashMap<String, Arc<Mutex<ManagedPlugin>>>>>`
- `ManagedPlugin.process: Option<Arc<Mutex<PluginProcess>>>` — **M5 修复**：外层锁只取 Arc，长 RPC 在内层锁外执行，`cancel()`/`stop()` 在运行期间可达
- `runtime.rs` 的 `PluginProcess` 封装 `std::process::Child` + stdin/stdout JSON-RPC
- 宿主反向服务：`frame_service` 在 `run` 期间向插件提供 `frames.extract`（capability `hostServices.framesExtract`，独立于插件协议版本）；契约见 `docs/frame-service.md` + `docs/schemas/frames-extract.schema.json`，Python 客户端 `plugins/frame_client.py`
- 超时：`send_request` 默认 600s；`cancel(task_id)` 发 cancel 通知尽力取消
- 崩溃恢复：`run_plugin` 发现进程已死 → 自动 `ensure_process` 重启
- 通知：`progress`/`log` → Tauri event `plugin://progress` `plugin://log` `plugin://result`

## 6. 打包

- `tauri.conf.json` `bundle.resources: ["../plugins"]` — 插件目录随包分发到 `resource_dir/plugins/`
- `builtin_plugin_dirs()` 依次尝试：CWD 祖先 `plugins/`（开发）→ `resource_dir/plugins/`（生产）
- 运行时依赖：Python/Node 插件需要用户在 PATH 安装对应解释器；`binary` 类型插件直接执行（externalBin 待接入）
- FFmpeg/FFprobe：`core/tools.rs` 的 `resolve_ffmpeg` 尝试 `settings.ffmpegPath` → `resource_dir/bin/` → PATH
- **更新器**：`tauri.conf.json` `plugins.updater` 固定指向 `https://github.com/H2SYJ/TimePin/releases/latest/download/latest.json`；开发配置中 `pubkey` 留空表示未配置，构建覆盖文件注入仓库变量 `TAURI_UPDATER_PUBLIC_KEY`。Release 流水线仅在 `main` 分支构建四个显式目标平台，构建 job 只读产物，单个聚合 job 验签后生成完整 `latest.json` 与 `SHA256SUMS` 并创建草稿 Release（人工检查后发布）。

## 7. 已实现边界与缺口

### 已实现（M1–M5 当前）

- ✅ 导入 MP4 媒体库（有界并发 + spawn_blocking）
- ✅ 手动打点 + 区间标注 + 标签/备注
- ✅ 生成 `video.txt` / `video.csv` / `video.xml` + golden file 测试
- ✅ FFmpeg 流复制导出（spawn_blocking，不阻塞 UI）
- ✅ 插件系统完整生命周期（发现→校验→启动→执行→进度→结果→停止/重启）
- ✅ 示例 Python 插件（模板匹配）
- ✅ 主题（light/dark/system + 系统主题跟随）
- ✅ 快捷键（硬编码 4 个）
- ✅ 设置持久化（JSON）
- ✅ 插件锁粒度修复（cancel 可达）

### 未实现

- ❌ 时间轴 UI（marks 只在详情面板显示文本列表）
- ❌ 帧精确导出（`ExportMode::FrameAccurate` 返回错误）
- ❌ 拖拽导入（前端未接 Tauri drag event）
- ❌ externalBin / PyInstaller 打包（Python 依赖用户 PATH）
- ❌ 快捷键自定义（Settings 无 `shortcuts` 字段）
- ❌ `ImportProgressEvent`/`ExportProgressEvent` 已定义但未 emit
- ❌ `video.xml` 帧率硬编码 30fps（`formats/mod.rs`）
- ❌ 插件 UI 试运行（单帧/片段调试）

## 8. 关键不变量

1. **不修改原 MP4**：所有伴随文件独立存储，仅「内嵌章节」选项（未实现）才允许写回
2. **video.txt 格式铁律**：UTF-8 无 BOM，`HH:MM:SS,ddd 标题`，首个 `00:00:00,000` 开始
3. **video.csv 列名**：固定 `Name,In,Out,Duration`，供 Premiere Pro 导入
4. **导出默认 `-c copy`**：禁止全量重编码；帧精确只允许首尾 GOP 极小重编码
5. **插件隔离**：独立 Sidecar 进程 + JSON-RPC/stdio，崩溃不影响主程序；文件写限在授权输出目录
