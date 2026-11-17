# GitHub 公开 API 与凭据（IPC v1.0.0）

TimePin 的「发现插件」使用 GitHub REST API。未配置凭据时直接匿名读取公开仓库，不要求先创建 Personal Access Token（PAT）。应用更新器仍使用既有签名更新流程，与本模块的凭据和配额分开。

## 请求与错误边界

- Rust 请求层统一设置 `User-Agent`、GitHub JSON `Accept`；前端只收到结构化结果，不负责推断 HTTP 限流。
- Token 仅用于 `https://api.github.com`，不跟随重定向转发。仓库 ZIP 的下载跳转仅允许安全的 GitHub 下载地址，并且不携带 Authorization。
- GitHub 原始 JSON 使用 snake_case，IPC 使用 camelCase。新结果与错误使用 `schemaVersion: "1.0.0"`，JSON Schema 见 `schemas/github-api.schema.json`。
- 分类依次为：2xx 成功；401 凭据失效；403/429 且 remaining=0 主限流；明确 secondary 证据的次级限流；其他 429 或带 Retry-After 的 403 保守等待；404 不存在或无权访问；其他 403 权限错误；其他客户端、服务端、网络错误分别处理。当前不发送条件请求；意外收到 304 时因没有对应缓存体，返回 `CLIENT_ERROR`，不伪造成功数据。
- 成功响应即使 remaining=0 也保留成功结果；下一次请求才等待。不将缺失或无权限的 `plugin.json` 错当限流；发现过程中遇到限流立即停止，不继续为每个仓库发请求。
- 使用响应 `Date` 计算服务器相对时间，`resetAt`、`retryAt` 均为校准后的本地 epoch **毫秒**。Retry-After 支持秒数和 HTTP-date。

## 身份与限流

身份 `ANONYMOUS | AUTHENTICATED | TOKEN_INVALID` 与请求限流门独立。`TOKEN_INVALID` 表示旧凭据已经停用，实际请求匿名发送。

主限流到期后串行放行探测；次级限流采用至少 60 秒、上限 15 分钟的指数退避。切换凭据不能清除次级限流。没有无限定时重试：等待后由用户再试，连续失败保护防止请求风暴。

公开 GET 收到 401 后删除旧凭据，最多匿名降级一次，同时向用户显示旧 Token 失效。当前插件发现面向公开仓库，不承诺私有仓库支持；404 始终用「不存在或无权访问」表述，不声称仓库一定不存在。

## Token 配置事务

1. 输入框只是候选值，编辑不会发送 API 请求。
2. 用户显式提交后，Rust 通过 `GET /user` 校验；只有 200 才激活新 Token。
3. 校验时遇到限流、权限、网络或服务端错误不会将候选 Token 判为失效，也不会未校验就保存。
4. 默认仅保存在 Rust 进程内存。勾选「记住」才使用系统凭据库；凭据库不可用时保留内存凭据并给出提示，不退回明文文件。
5. 成功后重试原操作一次。清除 Token 或 401 作废时删除系统凭据。旧版本 `localStorage.github_token` 只删除，不迁移或自动发送。

系统凭据库通过 keyring 的平台后端实现：Windows Credential Manager、macOS Keychain、Linux Secret Service。Linux 无桌面密钥服务、锁定的 Keychain 或系统权限拒绝时可能只能使用会话内存。

### 创建最小权限 PAT

推荐 [fine-grained PAT](https://github.com/settings/personal-access-tokens/new)，选择 **Public repositories (read-only)**，无需添加额外写权限；建议 30–90 天有效期。classic PAT 不勾任何 scope 也可读取公开数据，**不要为了读公开仓库勾选包含写权限的 public_repo**。

## 配额与提示

- 普通 REST：匿名通常为每出口 IP 60 次/小时（NAT/公司网络共享）；PAT 通常每账号 5000 次/小时，同一账号多个 Token 共享。
- 搜索使用独立桶，通常匿名 10 次/分钟、认证 30 次/分钟。发现插件的搜索与后续 manifest 读取会使用不同桶。
- 实时显示优先使用响应 `X-RateLimit-Limit/Remaining/Resource`，不是将上面的常见默认值作为所有请求的额度。
- 匿名 remaining < 10 只温和预警。确认匿名主限流后才显示含恢复时间的会话一次性引导；用户可选择配置、等待或查看已有内存结果。
- 次级限流及无法确认原因的限流只提示等待，不能宣传换 Token 可解决。401 提示重新配置，404 是另一处可由用户主动选择的认证入口。
- 本实现不主动调用 `/rate_limit`，也不启动定时刷新。界面保留上次成功结果作为内存缓存；不实现跨启动磁盘缓存或 ETag 条件缓存。匿名 304 仍消耗配额，不对匿名缓存宣称「免配额」。

## 安装安全边界

社区插件安装到应用数据下的可写 `plugins` 目录，不向只读的内置资源目录写入。下载与解压校验全部成功后才切换旧目录；暂存与旧版本备份位于其下的 `.timepin-installs`，不作为可发现插件扫描，避免旧版本覆盖新版本。

HTTP 响应（含 chunked）最多 128 MiB；ZIP 最多 10,000 个条目、单文件 128 MiB、解压总量 512 MiB。解压按流写盘，拒绝路径穿越、绝对路径、符号链接与多根目录。超过限制会明确失败，不继续替换现有插件；较大的模型/运行时依赖应通过既有依赖管理流程安装。

## 手工验收建议

使用受控 mock 服务或测试替身验证以下情形，不要通过反复请求真实 GitHub 故意耗尽配额：

1. 无 Token 首次发现成功，候选输入变化不触发请求。
2. 403 和 429 + remaining=0 都显示校准后的恢复时间；重复操作不重复弹引导，已有结果不丢失。
3. secondary / Retry-After 等待期间不发请求；换身份不绕过冷却。
4. `/user` 返回 200 后仅重试原搜索或安装一次；401、限流、网络错误都不保存候选。
5. 旧凭据 401 后撤销，公开 GET 最多匿名重试一次；404 与一般 403 不被错误识别为限流。
6. 凭据库失败时有明确仅内存提示；日志、配置、localStorage、结果 payload 均没有 Token。
7. 仓库 ZIP 跨域下载无 Authorization，非允许域/非 HTTPS 跳转被拒绝。

自动化测试与本次实际构建结论以 PR 的验证部分为准；真实 GitHub/NAT 配额与系统凭据库可用性受运行环境影响。
