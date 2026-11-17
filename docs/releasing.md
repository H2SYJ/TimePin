# Release 与签名更新器

## 概述

TimePin 使用 Tauri 官方 Updater 与 Process 插件提供应用内更新。发布流水线在 `main` 分支（或手动 `workflow_dispatch`）运行，产出**四个平台**的完整产物、签名校验结果与一份**完整**的 `latest.json`，再由单个聚合步骤创建 **draft Release**。发布是手动动作：维护者审阅草稿后才会将其公开，`latest.json` 在此之前不会上线，避免部分平台先行暴露更新。

## 一次发布所需配置

| 名称 | 类型 | 用途 |
|---|---|---|
| `TAURI_UPDATER_PUBLIC_KEY` | Repository Variable | Tauri 更新器公钥（minisign 格式），构建期注入 `plugins.updater.pubkey` |
| `TAURI_SIGNING_PRIVATE_KEY` | Repository Secret | Tauri 更新器私钥，用于生成各更新产物的 `.sig` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Repository Secret | 私钥口令（无口令时置空字符串） |

缺少任何一项（含公钥）`check --release` 预检即失败，流水线不会进入构建阶段。私钥与口令**绝不入库**；仓库内只允许公钥占位符与空 `pubkey`。

`check --release` 还会校验三处版本字段一致（`package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`）、`productName`/`identifier` 未漂移、更新器 endpoint 指向 `https://github.com/H2SYJ/TimePin/releases/latest/download/latest.json`、主窗口 `label` 为 `main`、以及 `v{version}` tag/Release 尚不存在（不覆盖已发布的 feed）。

## 流水线结构

```
preflight (ubuntu, contents:read)
  └─ check --release + test_release.py + 已存在 release/tag 拒绝
        ↓ outputs: version + 平台 matrix
build-tauri × 4 (contents:read, 不可写 Release)
  └─ pnpm tauri build --ci --target <rust-target> --bundles <bundles>
     --config .release-updater.json（注入公钥 + createUpdaterArtifacts=true）
  └─ stage 精确产物清单 → bundle-<platform> artifact
        ↓
draft-release (ubuntu, contents:write)
  └─ 下载全部 bundle-* → minisign 验签 → 生成 latest.json + SHA256SUMS
  └─ 复查 tag/Release 仍不存在 → 创建 draft → 上传全部资产 → 校验数量/大小 → 更新草稿说明
```

关键点：

- 并行构建 job **不持有** `contents:write`，不会各自创建/更新 Release；聚合 `latest.json` 的读改写竞态被单点聚合 job 消除（旧版各 job 分别调用 tauri-action 上传 updater JSON 存在部分平台覆盖风险）。
- 平台与 Rust target 显式绑定：`windows-2022/x86_64-pc-windows-msvc`、`ubuntu-22.04/x86_64-unknown-linux-gnu`、`macos-15/aarch64-apple-darwin`、`macos-15-intel/x86_64-apple-darwin`。GitHub runner 标签变化（如 macos-14 退役）不会让 feed 键静默漂移。
- 仅各平台的**签名更新产物**进入 `latest.json`：Windows `-setup.exe`（nsis）与 `.msi`（msi）、Linux `.AppImage`、macOS `.app.tar.gz`。`.deb`/`.dmg` 作为手动下载资产一并发布，但不参与自动更新。
- Windows 安装模式为 `passive`：沿用 NSIS 默认安装位置与 `com.timepin.app` 标识，保留既有安装身份与用户数据。
- 发布即新 tag `v{version}`：流水线在预检与最终步骤各检查一次 tag/Release 存在性，绝不覆盖已存在的 Release 或其资产。

## 开发构建行为

`src-tauri/tauri.conf.json` 中 `plugins.updater.pubkey` 为空且 `bundle.createUpdaterArtifacts` 为 `false`。这意味着本地/CI 开发构建**未配置签名更新**：前端可调用 `get_updater_config` 取得 `{ schemaVersion, configured: false }`，应据此提示"更新功能未配置"而非当作"已是最新"。`check()`/`download()` 在未配置时失败属预期。

## 手动发布前检查

1. 草稿说明中的四平台资产齐全且 `latest.json`、`SHA256SUMS` 存在。
2. 至少对 Windows 与 macOS 各做一次 `N-1 → N` 更新冒烟测试。
3. 验证已安装身份未变（安装目录、卸载项、`com.timepin.app`）。
4. 确认无需附带发行说明之外的额外资产（例如外部 Sidecar 清单）。

## 版本升级

版本号仍走 `beta → main` 发布流程统一升级（`chore(release): bump version to X.Y.Z`），三个文件同步： `package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`。本流水线不修改版本字段。

## 代码签名与更新签名

`latest.json` 的签名是**更新器签名**（minisign），与操作系统级代码签名（Windows Authenticode / macOS notarization）相互独立。当前流水线只做更新器签名；如需 OS 级签名/公证，另行配置相应证书 secret 并在对应平台 job 中增加签名/公证步骤。

## 测试

`python -I scripts/test_release.py` 用临时目录跑完整契约测试：版本/身份/endpoint/主窗口 label 守卫、公钥格式校验（拒绝文件路径与空值）、构建覆盖只改 `pubkey`+`createUpdaterArtifacts`、缺失/重复产物拒绝、缺失 minisign 时失败关闭且不写 `latest.json`、四平台 runner/target 固定。CI 预检 job 在每个 Release 运行前自动执行这些测试。
