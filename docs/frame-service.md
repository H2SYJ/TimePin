# 宿主帧提取服务（frames.extract，capability v1.0.0）

`frames.extract` 是宿主在插件 `run` 执行期间提供的反向 JSON-RPC 服务。它与插件 RPC 协议（仍 `1.0.0`）**分开版本化**：插件握手时通过 `initialize` 请求 `params.capabilities.hostServices.framesExtract` 读取能力版本，不是写在插件 response 里。

- 方法名：`frames.extract`
- JSON Schema：`docs/schemas/frames-extract.schema.json`
- Python 客户端：`plugins/frame_client.py`（仅标准库；cv2/numpy 是可选 helper，不强制）

## 用法（plugin run handler）

```python
from frame_client import FramesClient, JsonRpcPeer

peer = JsonRpcPeer()                              # 同步 stdio peer
frames = FramesClient(peer.send_request)          # 也可用已有 dispatcher 的 send_request

def handle(method, params):
    if method == "run":
        out = params["outputDir"]                 # 宿主分配的授权目录
        result = frames.extract(
            params["video"]["path"],
            time_range=[60.0, 90.0],
            scratch_dir=f"{out}/frames",          # 必须 canonicalize 落在 outputDir 内
            mode="image2",
            sample_fps=2.0,
            output_format="jpeg",
            delivery="files",
        )
        for f in result["frames"]:
            use(f["pts_sec"], f["path"])          # pts_sec 为真实源时间戳
        return {"protocolVersion": "1.0.0", "pluginName": "x", "marks": [], "logs": [], "evidencePaths": []}
    if method == "shutdown":
        peer.close()
        return {}

peer.serve_forever(handle)
```

已有 dispatcher 的插件只需把 `send_request(method, params)` 回调注入 `FramesClient`，不需要再启用 `JsonRpcPeer`。

## 请求契约

`options` 字段（详见 schema）：

| 字段 | 类型 | 默认 | 说明 |
|---|---|---|---|
| `mode` | `image2`/`keyframes` | `image2` | image2=ffmpeg 解码均匀采样；keyframes=只读 mp4 关键帧索引 |
| `time_range` | `[start, end)` 秒 | 必填 | 半开区间，`0 <= start < end` |
| `sample_fps` | 正数或 `null` | `null` | `0` 直接 `-32602`；`null`=不过滤 |
| `output_format` | `jpeg`/`png` | `jpeg` | jpeg 更小更快；png 无损 |
| `scratch_dir` | 路径 | 必填 | 必须落在本次 run `outputDir` 内，否则 `-32000` |
| `delivery` | `files`/`base64` | `files` | base64 仍写盘并额外返回 `image_base64` |

`keyframes` 模式下 `sample_fps` 是**最大采样率上限**：宿主只在真实关键帧列表上按 ≥ 1/sample_fps 间隔筛选（不合成中间帧）；`null` 表示返回范围内全部关键帧。

## 响应契约

```json
{"protocolVersion":"1.0.0","frames":[
  {"index":0,"pts_sec":60.0417,"path":"<scratch_dir>/<uuid>/frame_000.jpg","image_base64":"..."},
  {"index":1,"pts_sec":60.5417,"path":"<scratch_dir>/<uuid>/frame_001.jpg"}
 ],"total_frames":2,"mode":"image2"}
```

- `pts_sec` 是宿主读到的**真实源 PTS**，绝不等于 `index / sample_fps`；插件必须直接使用。
- `total_frames == frames.length`。
- `image_base64` 仅在 `delivery == "base64"` 时出现，文件仍落盘。

## 文件生命周期

- `delivery="files"`：帧写到 `<scratch_dir>/<uuid>/`，JSON 返回 `path`。宿主**不删除**。
- `delivery="base64"`：文件同样落盘（便于插件二次处理），`image_base64` 内联帧数据；插件可直接消费内嵌数据。
- 插件复用目录前需自行清理。`PluginsClient`/`read_image_bytes`/`frame_to_ndarray` 都不会隐式删文件。

## 错误码

| code | 含义 |
|---|---|
| `-32601` | 宿主未注册该服务（未广播 `framesExtract` 能力） |
| `-32602` | params 校验失败（`sample_fps=0`、缺字段、类型错等） |
| `-32000` | 服务错误：视频不可读、`scratch_dir` 越界、不支持的封装、资源超限 |
| `-32603` | 宿主内部序列化/IO 异常 |

## 限制

- `keyframes` 依赖本地 MP4 关键帧索引（stss/stts/ctts/elst）。fragmented MP4、复杂 edit list 无法解析时返回 `-32000`。
- `image2` 只依赖 ffmpeg，理论上任意 ffmpeg 能解的格式可用。
- 单次调用帧数、总字节数、ffmpeg 执行时长都有内部上限（防 OOM/防卡死），常量见 `frame_service` 实现。
- 不支持「indexed raw media」模式；`mode` 只允许 `image2`/`keyframes`。
- `read_image_bytes` 用 Python `open` 读 Unicode 路径，**不走** `cv2.imread`；`frame_to_ndarray` 走 `cv2.imdecode`，可选依赖。
- `JsonRpcPeer` 单线程可重入；不要在别的线程再开 read loop；通知 handler 抛异常会被吞掉以保持循环。
