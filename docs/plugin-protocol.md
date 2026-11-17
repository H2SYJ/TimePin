# TimePin 插件协议（v1.0.0）

TimePin 插件运行在**独立 Sidecar 进程**中（铁律 #5），通过 stdio 上的
JSON-RPC 2.0 与主程序通信。插件崩溃、死循环、打印垃圾输出都不会影响主程序：
主程序只按行解析合法 JSON，解析失败的行直接忽略。

## 传输层

- **编码**：UTF-8（Windows 上插件必须强制 `sys.stdout`/`stdin` 为 UTF-8，见示例插件）
- **分帧**：JSON Lines —— 每条消息一行，`\n` 结尾，消息体内不允许出现裸换行
- **方向**：
  - 主程序 → 插件：Request（带 `id`）
  - 插件 → 主程序：Response（带回 `id`）与 Notification（无 `id`）

## 生命周期

```
主程序启动 → 扫描插件目录（plugin.json）→ 校验 manifest
首次 run_plugin → spawn 进程 → initialize 握手 → ready
run 调用      → progress/log 通知 → 返回 PluginResult → ready
进程崩溃     → 状态标记 crashed → 下次 run 自动重启
应用退出/热重载 → shutdown 请求 → 超时强杀
```

## 方法

### `initialize`（握手，spawn 后第一个调用）

**Request**
```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{
  "protocolVersion":"1.0.0",
  "capabilities":{"progressEvents":true,"cancellation":true}
}}
```

**Response**
```json
{"jsonrpc":"2.0","id":1,"result":{
  "protocolVersion":"1.0.0",
  "pluginName":"my-plugin",
  "capabilities":{"progressEvents":true,"cancellation":true}
}}
```

版本兼容性：semver **major 相同即兼容**；任一 major 为 0 时要求完全一致。
不兼容 → 主程序拒绝启动并报 `IncompatibleProtocol`。

### `run`（执行打标）

**Request params**（`RunParams`，camelCase）
```json
{
  "taskId": "uuid",
  "video": {"id":"...","path":"C:/video.mp4","duration":3600.0,
             "width":1920,"height":1080,"fps":30.0,"metadata":{}},
  "params": { /* manifest.parametersSchema 定义的参数 */ },
  "outputDir": "C:/…/plugin-output/example-plugin/uuid"
}
```

`outputDir` 是插件**唯一允许写入**的目录（证据截图、中间产物）。
主程序为每次运行新建目录；插件写其他路径属于违规行为。

**Response result** = `PluginResult`
```json
{
  "protocolVersion":"1.0.0",
  "pluginName":"my-plugin",
  "marks":[ {"id":"uuid","startTime":12.5,"endTime":null,
             "label":"Boss 击杀","color":null,"confidence":0.92,
             "sourcePlugin":"my-plugin","thumbnailPath":null,"metadata":{}} ],
  "logs":["…"],
  "evidencePaths":["C:/…/shot_001.png"]
}
```

`Mark` 字段与 `src-tauri/src/formats/mod.rs` 的 `Mark` 结构一一对应
（camelCase）：`startTime`/`endTime` 为秒（f64），`endTime: null` 表示时间点标注。

### `cancel`

**Request** `{"method":"cancel","params":{"taskId":"…"}}`
插件设置内部取消标志，在下一个进度点退出 `run` 循环，返回**已产出的部分结果**。
响应 `{"cancelled":true}`。

### `shutdown`

**Request** `{"method":"shutdown","params":{}}` → 响应 `{}` 后自行退出。
主程序等待约 3s，超时强杀（`child.kill()`）。

## 通知（插件 → 主程序）

### `hostServices`（宿主反向服务，能力独立版本）

`initialize` **请求**的 `params.capabilities.hostServices` 是宿主广播给插件的服务能力表（不是插件 response 的一部分）。例如：

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{
  "protocolVersion":"1.0.0",
  "capabilities":{"progressEvents":true,"cancellation":true,
                  "hostServices":{"framesExtract":"1.0.0"}}
}}
```

- `framesExtract:"1.0.0"` 表示宿主在 `run` 期间提供反向服务 `frames.extract`；插件读它判断可调用，**不要**写回自己的 response。
- 字段名 camelCase、值是 semver 字符串、与插件 RPC `protocolVersion` 分离演进。
- 请求/响应契约与 JSON Schema 见 `docs/frame-service.md` 与 `docs/schemas/frames-extract.schema.json`；Python 客户端为 `plugins/frame_client.py`（标准库实现）。

### `progress`
```json
{"jsonrpc":"2.0","method":"progress","params":{
  "taskId":"…","progress":0.42,"message":"已处理 42%","marksSoFar":17
}}
```
主程序转发为 Tauri event `plugin://progress`。

### `log`
```json
{"jsonrpc":"2.0","method":"log","params":{
  "taskId":"…","level":"info|warn|error","message":"…"
}}
```
转发为 `plugin://log`。stdout 上的非 JSON 行（print 调试等）会被主程序忽略，
但建议一律走 `log` 通知，这样能在插件日志视图集中查看。

## 主程序 → 前端事件（版本化，`schemaVersion` 字段）

| 事件名 | Payload |
|---|---|
| `plugin://status` | `PluginStatusEvent{schemaVersion,pluginName,status,detail?}` status ∈ discovered/validated/started/ready/running/stopped/crashed |
| `plugin://progress` | `PluginProgressEvent{schemaVersion,pluginName,taskId,progress,message?}` |
| `plugin://log` | `{schemaVersion,pluginName,taskId,level,message}` |
| `plugin://result` | `PluginRunFinished{schemaVersion,taskId,pluginName,videoId,outcome,error?,marksCount}` outcome ∈ ok/error/cancelled/crashed |
| `plugin://env-progress` | `PluginEnvProgress{schemaVersion,pluginName,stage,message,error}` stage ∈ creating-venv/installing/ready/failed |

## 插件清单（`plugin.json`）

```json
{
  "protocolVersion": "1.0.0",
  "name": "my-plugin",           // ASCII [a-zA-Z0-9_-]，目录内唯一
  "version": "0.1.0",
  "description": "…",
  "author": "…",
  "entry": "main.py",            // 相对路径，禁止绝对路径与 ".."
  "runtime": "python",           // python | node | rust | binary
  "python": ">=3.10,<3.13",      // 可选，仅 runtime=python 生效：版本约束
  "dependencies": [              // 可选，仅 runtime=python 生效：pip 依赖
    "opencv-python>=4.8.0",
    "numpy>=1.24.0"
  ],
  "capabilities": ["metadata","fileWrite"],
  "parametersSchema": { /* JSON Schema properties，前端渲染参数表单 */ },
  "outputSchema": { }
}
```

校验规则见 `src-tauri/src/plugins/manifest.rs::validate()`。

### Python 运行时与依赖

TimePin 分发内嵌 Python（`runtime/python/`），用户无需安装系统 Python。
解释器解析优先级（`plugins/python_env.rs::resolve_python`）：

1. 设置中的 `pythonPath`（显式覆盖，供调试用）
2. `<install>/runtime/python/`（Tauri `bundle.resources` 分发）
3. `<exe>/dependencies/python/`（开发期手动放置，与 FFmpeg 依赖目录约定一致）
4. 系统 `python` / `python3`（仅当设置开启 `useSystemPython`）

声明 `python` 版本约束时按 PEP 440 简化语法校验：
`>=3.10`、`>=3.10,<3.13`、`==3.11.*`、`~=3.11`、裸版本 `3.11`（前缀匹配 `3.11.x`）。

### 依赖与虚拟环境

`runtime=python` 且 `dependencies` 非空的插件在首次 spawn 前自动建置隔离 venv：

- 位置：`<appData>/pluginEnvironments/<name>-<dirHash8>/venv`
- 指纹：dependencies + 解释器版本 哈希 → 变化时自动重建
- 失败：透传 pip stdout/stderr 到 `PluginInfo.pythonEnvironment.error`
- 进度事件：`plugin://env-progress` → `PluginEnvProgress{stage: creating-venv|installing|ready|failed, message, error}`

同一台机器可同时安装 onnxruntime 1.16 与 1.20（两个插件各自独立 venv）。

删除插件目录后 `reload` 自动清理 `<pluginEnvironments>/` 下的孤儿目录；
手动重建全部环境可在设置页点「重建所有插件环境」。

### 共享 SDK

`plugins/_shared/timepin_vision/` 提供可复用的视觉识别 SDK（模板匹配、OCR、
序列差分、ONNX 模型加载等）。插件在 `dependencies` 中写本地相对路径
（如 `"../_shared/timepin_vision"`）即可让它装到自己的 venv 里；
打包时 `_shared/` 随 `plugins/` 一起经 `bundle.resources` 分发。

## 新增一个插件

1. 在 `plugins/` 或设置中的 `customPluginDirs` 下新建目录
2. 放入 `plugin.json` + 入口文件（按需声明 `dependencies`）
3. 前端点「重新加载」或重启应用 —— 不需要改主程序任何代码

## 结果合并与去重

多插件结果由 `plugins::manager::merge_marks` 合并：同一 `label` 且
`startTime` 相差 < ε（默认建议 0.5s）视为重复，保留 `confidence` 高者。

## 参考实现

`plugins/example-template-match/` —— 纯标准库 Python 示例，演示全部方法、
进度通知、取消与证据文件写入。
