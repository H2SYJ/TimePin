# 手动标注持久化（Manual Marks）

手动打点/区间标注按视频分别持久化在应用数据目录，**不修改原 MP4，也不写源文件路径**。

## 存储位置

```
<app_data_dir>/marks/<videoId>.json
```

- `app_data_dir` 由 Tauri 解析（Windows 通常为 `%APPDATA%/com.timepin.app`）。
- `videoId` 只允许 1–128 个 ASCII 字母、数字、`-`、`_`，且禁止 Windows 保留设备名（`CON`/`NUL`/`COM1`–`COM9`/`LPT1`–`LPT9`）——防止路径穿越。
- 视频必须存在于媒体库索引（`videos/index.json`）中，否则命令报错。
- 删除媒体库条目不会自动删除其标注文档（目录在应用数据内，可由用户清理）。

## 文档 Schema（`schemaVersion: "1.0.0"`）

```jsonc
{
  "schemaVersion": "1.0.0",
  "videoId": "3f4c…",
  "marks": [
    {
      "id": "…",                    // 非空且文档内唯一
      "startTime": 12.5,            // 秒；有限、>= 0、<= 视频已知时长
      "endTime": null,              // null=时间点；区间时必须为有限数且 > startTime、<= 时长
      "label": "Boss 击杀",          // 去空白后非空
      "color": null,
      "confidence": 0.92,           // 可选；有限且 0–1
      "sourcePlugin": "manual",
      "thumbnailPath": null,
      "metadata": {}
    }
  ]
}
```

序列化为 `camelCase`，与 `src-tauri/src/formats/mod.rs` 的 `Mark`、
`src/types/index.ts` 的 `MarksDocument` 一一对应；未知字段会被拒绝
（`deny_unknown_fields`）。视频时长未知（探测失败记为 0）时不做时长上限校验。

## IPC 命令

| 命令 | 参数（invoke camelCase） | 返回 |
|---|---|---|
| `get_marks` | `{ videoId: string }` | `MarksDocument`（文件缺失时返回空文档） |
| `save_marks` | `{ videoId: string, marks: Mark[] }` | 校验后持久化并返回 `MarksDocument` |

前端封装：`api.getMarks(videoId)` / `api.saveMarks(videoId, marks)`（`src/lib/ipc.ts`）。

## 校验与持久性保证

- **校验**：schemaVersion 精确匹配 `1.0.0`；videoId 与媒体库一致；mark id 非空去重；
  label 非空；时间为有限数、`start >= 0`、区间 `end > start`；已知时长时不得越界；
  `confidence ∈ [0,1]`。
- **读**：文件不存在 → 空文档；JSON 损坏/schema 不符 → 报错，且 `save_marks` 同样拒绝，
  **绝不覆盖损坏文件**（需人工备份修复）。
- **写**：进程内 `Mutex` 串行化全部读写；先写同目录 `.marks-<uuid>.tmp` 临时文件，
  `sync_all` 刷盘后 `fs::rename` 原子替换（Windows 兼容：先关句柄再 rename，
  失败不删旧文件；残留 `.tmp` 不影响读取，可安全清理）。

## 实现位置

- `src-tauri/src/core/marks.rs` — 文档模型、校验、原子写入（纯目录参数，可脱离 Tauri 测试）。
- `src-tauri/src/commands/marks.rs` — `get_marks` / `save_marks` 薄壳（`spawn_blocking`）。
- `src-tauri/src/core/library.rs` — 暴露 `load_videos_index` 供按目录校验视频存在性。
