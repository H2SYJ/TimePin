# GitHub 公开 API 与凭据（IPC v1.0.0）

TimePin 的「发现插件」使用 GitHub REST API。未配置凭据时直接匿名读取公开仓库，不要求先创建 Personal Access Token（PAT）。应用更新器仍使用既有签名更新流程，与本模块的凭据和配额分开。

## 请求与错误边界

- Rust 请求层统一设置 `User-Agent`、GitHub JSON `Accept`；前端只收到结构化结果，不负责推断 HTTP 限流。
- Token 仅用于 `https://api.github.com`，不跟随重定向转发。仓库 ZIP 的下载跳转仅允许安全的 GitHub 下载地址，并且不携带 Authorization。
- GitHub 原始 JSON 使用 snake_case，IPC 使用 camelCase。新结果与错误使用 `schemaVersion: "1.0.0"`，JSON Schema 见 `schemas/github-api.schema.json`。
- 分类依次为：2xx 成功；401 凭据失效；403/429 且 remaining=0 主限流；明确 secondary 证据的次级限流；其他 429 或带 Retry-After 的 403 保守等待；404 不存在或无权访问；其他 403 权限错误；其他客户端、服务端、网络错误分别处理。
- manifest 使用身份隔离的持久化缓存，1 小时内直接返回，不发送 HTTP；软过期后允许匿名或认证请求用 ETag 条件校验。其他已认证请求继续使用内存 ETag/Last-Modified 条件缓存。304 必须有对应可用缓存，意外的无缓存 304 返回 `CLIENT_ERROR`；只有携带有效认证的条件请求收到 304 才有 GitHub 主配额豁免保证，匿名条件请求不保证免配额。
- 成功响应即使 remaining=0 也保留成功结果；下一次请求才等待。不将缺失或无权限的 `plugin.json` 错当限流；发现过程中遇到限流立即停止，不继续为每个仓库发请求。
- 使用响应 `Date` 计算服务器相对时间，`resetAt`、`retryAt` 均为校准后的本地 epoch **毫秒**。Retry-After 支持秒数和 HTTP-date。

## 身份与限流

身份 `ANONYMOUS | AUTHENTICATED | TOKEN_INVALID` 与请求限流门独立。`TOKEN_INVALID` 表示旧凭据已经停用，实际请求匿名发送。

- 资源级主限流按 `X-RateLimit-Resource`（或 URL 推断的 `core`/`search`/`graphql`）分桶，互不干扰：搜索桶耗尽不阻塞普通 REST 调用，反之亦然。`RATE_WAIT` 错误携带 `rateLimit.resource`，让前端识别本次是哪一桶。
- 次级限流与原因未知限流的保守等待是**全局**冷却，不绑定任何资源桶；对应 `RATE_WAIT` 的 `rateLimit` 为 `null`。次级限流采用至少 60 秒、上限 15 分钟的指数退避，连续 3 次进入会话级停闸。
- 切换凭据、清除 Token 或 401 回退不能清除次级/未知/停闸安全闸；但可以重置资源级主限流与本地搜索节奏（因为那是另一账号的配额）。
- 本地搜索节奏是容量为 1 的令牌桶：匿名每 6 秒一次（≈10/min）、已认证每 2 秒一次（≈30/min）。超过节奏返回 `RATE_WAIT` + `rateLimit.resource="search"` + `retryAt`，不阻塞 UI。
- `GET /user` 候选凭据校验不受搜索主限流阻塞：匿名候选可以绕过匿名主冷却去验证新 Token，但次级/未知/停闸全局冷却仍然生效。

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
- 本实现不主动调用 `/rate_limit`，也不启动定时刷新。

## Manifest 持久化缓存（TPIN-26）

缓存由 Rust 管理，通过 `tauri-plugin-store` 同步保存至应用数据目录的 `plugins-cache.json`，不需要前端 Store 权限或 JavaScript 依赖。「发现插件」仍按原流程读取 manifest，不采用懒加载；前端 `discoveryCache` 只负责会话内结果复用与渲染。

- 键包含版本、身份、owner、repo、branch：`manifest:v1:<fingerprint>:<owner>:<repo>:<branch>`。身份为 `anon` 或 `pat:<SHA-256(token) 前 16 位十六进制>`；键组件必须无歧义编码。Store 不保存 Token，也不保存凭据或服务器配额快照。
- **软过期 1 小时**：`now < expiresAt` 直接返回缓存，零 Core 请求；不触发 HTTP，因此不受网络请求冷却门阻塞，也不伪造新的配额数据。
- **软过期至 30 天硬过期之间**：有 ETag 时发 `If-None-Match`；304 续期并保留服务端未更新的 ETag，200 替换缓存。没有 ETag 时重新完整拉取。所有实际网络请求仍遵守资源主限流与全局冷却。
- **硬过期 30 天**：`now >= hardExpiresAt` 删除并重新获取，不使用过期 ETag；启动时清理所有身份桶的硬过期条目。成功的 200 与 304 均从本次校验时间重新计算软、硬期限。
- 新 Token、轮换 Token、清除 Token、401 回退只切换身份桶，不主动清空整个 Store。A → B 不命中 A，B → A 可恢复 A 的未过期缓存；匿名与认证桶同样互相隔离。重启后使用同一 Token（记住的凭据或重新配置）可命中原桶。
- 缓存文件损坏、条目不合法、读写失败不会阻止应用启动或正常联网；记录不含敏感内容的告警并降级。错误响应不作为正常 manifest 持久化。

缓存文件是非加密的本地性能缓存，不能视为凭据存储或远端权限的实时证明：1 小时新鲜期内不会探测服务端是否撤销了 Token。插件发现当前仍面向公开仓库，不承诺私有仓库支持。

### 通用响应内存缓存

- `ResponseCache` 保留，键为完整 URL + `Accept` + `X-GitHub-Api-Version` + 身份指纹，默认 `max_entries=100`、`ttl=10 分钟`、整表 `max_bytes=32 MiB`，按 LRU 逐出。身份预算与缓存不能通过同一 Token 重提或已匿名时再次清除来重置。
- 非 manifest 请求仍只在已认证时发送 ETag/Last-Modified 校验头；匿名条件请求仅允许严格限定的 manifest 读取路径，不扩展到搜索或其他 API。
- 200 保存校验器与响应体；304 必须命中可用内存条目才能返回缓存体，否则为 `CLIENT_ERROR`。
- `public_zip` 的 API 直接 200 ZIP 可继续缓存/重验证；通常的 zipball 302 → codeload 下载不缓存重定向或最终 ZIP，跳转后不发送 `Authorization` 或校验器。
- 搜索、`/user`、ZIP 与 codeload 均不进入 manifest 持久化缓存；重启后通用内存缓存仍为空。

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
8. 资源级限流按桶生效：search 桶耗尽时 core 调用仍被放行；返回的 `RATE_WAIT.rateLimit.resource` 与命中的桶一致。
9. 本地搜索节奏：匿名搜索 6 秒内连续两次调用第二次不发 HTTP、返回 RATE_WAIT；换 Token 后预算按 2 秒节奏重新计时。
10. 通用响应 ETag 缓存仅在已认证下生效：第二次请求带 `If-None-Match`，304 复用缓存体；切换 Token 后不复用另一身份的校验器。
11. 同一身份读取 manifest 后重启，1 小时内命中 Store，不发送 Core 请求；Token A → B → A、匿名 → Token → 匿名均不串桶且可恢复原桶。
12. manifest 恰好满 1 小时发 ETag 条件请求（包括匿名）；304 正确续期、200 正确替换。没有 ETag 时无条件刷新；满 30 天不发送旧 ETag，启动清理所有过期桶。
13. 缓存损坏或磁盘读写失败不影响启动与联网；401 回退重新选择匿名桶，不把认证结果保存到匿名桶。真实 GitHub 304 的主配额表现仅按实际响应头与认证规则解释。

自动化测试与本次实际构建结论以 PR 的验证部分为准；真实 GitHub/NAT 配额与系统凭据库可用性受运行环境影响。
