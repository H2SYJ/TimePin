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
  "capabilities": ["metadata","fileWrite"],
  "parametersSchema": { /* JSON Schema properties，前端渲染参数表单 */ },
  "outputSchema": { }
}
```

校验规则见 `src-tauri/src/plugins/manifest.rs::validate()`。

## 新增一个插件

1. 在 `plugins/` 或设置中的 `customPluginDirs` 下新建目录
2. 放入 `plugin.json` + 入口文件
3. 前端点「重新加载」或重启应用 —— 不需要改主程序任何代码

## 结果合并与去重

多插件结果由 `plugins::manager::merge_marks` 合并：同一 `label` 且
`startTime` 相差 < ε（默认建议 0.5s）视为重复，保留 `confidence` 高者。

## 参考实现

`plugins/example-template-match/` —— 纯标准库 Python 示例，演示全部方法、
进度通知、取消与证据文件写入。
