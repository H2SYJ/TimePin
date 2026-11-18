//! GitHub 插件发现命令：搜索带有 `time-pin-plugins` topic 的仓库
//!
//! 安全说明：发现的插件均未经过 TimePin 官方验证。安装时会下载整个仓库 ZIP 并解压到插件目录，
//! 解压过程会过滤 `..` 与绝对路径，manifest 使用与本地插件相同的 PluginManifest::validate() 校验。

use crate::core::github::{
    api_url, ErrorCode, GitHubApiError, GitHubClient, GitHubRateInfo, GitHubResponse, GitHubSession,
};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use tauri::{command, Emitter, State};
use zip::ZipArchive;
const SEARCH_QUERY: &str = "topic:time-pin-plugins";

#[derive(Clone, Copy)]
struct ExtractionLimits {
    entries: usize,
    entry_bytes: u64,
    total_bytes: u64,
}
const EXTRACTION_LIMITS: ExtractionLimits = ExtractionLimits {
    entries: 10_000,
    entry_bytes: 128 * 1024 * 1024,
    total_bytes: 512 * 1024 * 1024,
};

/// IPC models use camelCase; GitHub's wire models below deliberately do not.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePlugin {
    pub name: String,
    pub full_name: String,
    pub description: Option<String>,
    pub html_url: String,
    pub clone_url: String,
    pub stargazers_count: u32,
    pub forks_count: u32,
    pub updated_at: String,
    pub language: Option<String>,
    pub topics: Vec<String>,
    pub owner: RepositoryOwner,
    pub default_branch: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryOwner {
    pub login: String,
    pub avatar_url: String,
    pub html_url: String,
}

#[derive(Deserialize)]
struct ApiRepository {
    name: String,
    full_name: String,
    description: Option<String>,
    html_url: String,
    clone_url: String,
    stargazers_count: u32,
    forks_count: u32,
    updated_at: String,
    language: Option<String>,
    #[serde(default)]
    topics: Vec<String>,
    owner: ApiOwner,
    default_branch: String,
}

#[derive(Deserialize)]
struct ApiOwner {
    login: String,
    avatar_url: String,
    html_url: String,
}

#[derive(Deserialize)]
struct GitHubSearchResponse {
    total_count: u32,
    items: Vec<ApiRepository>,
}

impl From<ApiRepository> for RemotePlugin {
    fn from(repo: ApiRepository) -> Self {
        Self {
            name: repo.name,
            full_name: repo.full_name,
            description: repo.description,
            html_url: repo.html_url,
            clone_url: repo.clone_url,
            stargazers_count: repo.stargazers_count,
            forks_count: repo.forks_count,
            updated_at: repo.updated_at,
            language: repo.language,
            topics: repo.topics,
            owner: RepositoryOwner {
                login: repo.owner.login,
                avatar_url: repo.owner.avatar_url,
                html_url: repo.owner.html_url,
            },
            default_branch: repo.default_branch,
        }
    }
}

/// 供 IPC 返回的 manifest（与 plugins::manifest::PluginManifest 字段保持一致）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub protocol_version: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub entry: String,
    pub runtime: String,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub parameters_schema: serde_json::Value,
    #[serde(default)]
    pub output_schema: serde_json::Value,
    /// Python 解释器约束（可选，仅 runtime=python 生效）
    #[serde(default)]
    pub python: Option<String>,
    /// pip 依赖列表（可选，仅 runtime=python 生效）
    #[serde(default)]
    pub dependencies: Vec<String>,
}

impl PluginManifest {
    /// 从原始 JSON 文本解析，尽量宽容（额外字段忽略）。
    fn parse(json_text: &str) -> Result<Self, String> {
        serde_json::from_str(json_text).map_err(|e| format!("解析 plugin.json 失败: {e}"))
    }

    /// 转换为本地 manifest 类型以复用 validate 逻辑
    fn to_local(&self) -> Result<crate::plugins::manifest::PluginManifest, String> {
        let runtime = match self.runtime.as_str() {
            "python" => crate::plugins::manifest::PluginRuntime::Python,
            "node" => crate::plugins::manifest::PluginRuntime::Node,
            "rust" => crate::plugins::manifest::PluginRuntime::Rust,
            "binary" => crate::plugins::manifest::PluginRuntime::Binary,
            other => return Err(format!("不支持的 runtime: {other}")),
        };

        let capabilities: Vec<crate::plugins::manifest::PluginCapability> = self
            .capabilities
            .iter()
            .filter_map(|cap| match cap.as_str() {
                "videoFrames" => Some(crate::plugins::manifest::PluginCapability::VideoFrames),
                "audioSamples" => Some(crate::plugins::manifest::PluginCapability::AudioSamples),
                "metadata" => Some(crate::plugins::manifest::PluginCapability::Metadata),
                "gpu" => Some(crate::plugins::manifest::PluginCapability::Gpu),
                "network" => Some(crate::plugins::manifest::PluginCapability::Network),
                "fileWrite" => Some(crate::plugins::manifest::PluginCapability::FileWrite),
                _ => None,
            })
            .collect();

        Ok(crate::plugins::manifest::PluginManifest {
            protocol_version: self.protocol_version.clone(),
            name: self.name.clone(),
            version: self.version.clone(),
            description: self.description.clone(),
            author: self.author.clone(),
            entry: self.entry.clone(),
            runtime,
            capabilities,
            parameters_schema: serde_json::from_value(self.parameters_schema.clone())
                .unwrap_or_default(),
            output_schema: serde_json::from_value(self.output_schema.clone()).unwrap_or_default(),
            python: self.python.clone(),
            dependencies: self.dependencies.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredPlugin {
    pub repository: RemotePlugin,
    pub manifest: Option<PluginManifest>,
    pub manifest_error: Option<String>,
    pub is_installed: bool,
    pub local_path: Option<String>,
    /// 简单校验状态：Verified = 官方（H2SYJ org）、Community = 社区仓库、Suspicious = 缺 manifest 等
    pub verification: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverPluginsResult {
    pub schema_version: &'static str,
    pub plugins: Vec<DiscoveredPlugin>,
    pub total_count: u32,
    /// Search bucket snapshot, retained for IPC v1 compatibility.
    pub rate_limit: Option<GitHubRateInfo>,
    /// Latest snapshot of every resource observed by this session/discovery.
    pub rate_limits: Vec<GitHubRateInfo>,
}

#[derive(Deserialize)]
struct FileContent {
    content: String,
    encoding: String,
}

fn parse_response<T: serde::de::DeserializeOwned>(
    response: &GitHubResponse,
    context: &str,
) -> Result<T, GitHubApiError> {
    serde_json::from_slice(&response.body).map_err(|_| {
        let mut error = GitHubApiError::client(format!("{context}响应格式不正确"));
        error.rate_limit = response.rate_limit.clone().map(Box::new);
        error
    })
}

async fn search_repositories(
    client: &GitHubClient,
    page: u32,
    per_page: u32,
) -> Result<
    (
        Vec<RemotePlugin>,
        u32,
        Option<GitHubRateInfo>,
        Vec<GitHubRateInfo>,
    ),
    GitHubApiError,
> {
    let url = api_url(
        &["search", "repositories"],
        &[
            ("q", SEARCH_QUERY),
            ("sort", "updated"),
            ("order", "desc"),
            ("page", &page.to_string()),
            ("per_page", &per_page.to_string()),
        ],
    )?;
    let response = client.public_get(url).await?;
    let search: GitHubSearchResponse = parse_response(&response, "GitHub 搜索")?;
    Ok((
        search.items.into_iter().map(RemotePlugin::from).collect(),
        search.total_count,
        response.rate_limit,
        response.rate_limits,
    ))
}

async fn fetch_plugin_manifest(
    client: &GitHubClient,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<(Option<PluginManifest>, Option<GitHubRateInfo>), GitHubApiError> {
    validate_repository(owner, repo)?;
    let url = api_url(
        &["repos", owner, repo, "contents", "plugin.json"],
        &[("ref", branch)],
    )?;
    let response = match client.public_get(url).await {
        Ok(response) => response,
        Err(error) if error.code == ErrorCode::NotFoundOrPrivate => {
            return Ok((None, error.rate_limit.map(|rate| *rate)))
        }
        Err(error) => return Err(error),
    };
    let file: FileContent = parse_response(&response, "GitHub 文件")?;
    let parse = || -> Result<PluginManifest, String> {
        if file.encoding != "base64" {
            return Err("plugin.json 编码不是 base64".into());
        }
        // GitHub inserts line breaks into base64 contents responses.
        let compact: String = file
            .content
            .chars()
            .filter(|c| !c.is_ascii_whitespace())
            .collect();
        let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, compact)
            .map_err(|_| "plugin.json Base64 内容无效".to_owned())?;
        let text = String::from_utf8(decoded).map_err(|_| "plugin.json 不是 UTF-8".to_owned())?;
        PluginManifest::parse(&text)
    };
    parse()
        .map(|manifest| (Some(manifest), response.rate_limit.clone()))
        .map_err(|message| {
            let mut error = GitHubApiError::client(message);
            error.rate_limit = response.rate_limit.map(Box::new);
            error
        })
}

fn validate_repository(owner: &str, repo: &str) -> Result<(), GitHubApiError> {
    if owner.is_empty()
        || !owner
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        || repo.is_empty()
        || matches!(repo, "." | "..")
        || !repo
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    {
        return Err(GitHubApiError::client("GitHub owner/repo 参数不合法"));
    }
    Ok(())
}

#[command]
pub async fn get_github_session(
    client: State<'_, GitHubClient>,
) -> Result<GitHubSession, GitHubApiError> {
    Ok(client.session().await)
}

#[command(rename_all = "camelCase")]
pub async fn configure_github_token(
    client: State<'_, GitHubClient>,
    candidate: String,
    remember: bool,
) -> Result<GitHubSession, GitHubApiError> {
    client.configure_token(candidate, remember).await
}

#[command]
pub async fn clear_github_token(
    client: State<'_, GitHubClient>,
) -> Result<GitHubSession, GitHubApiError> {
    Ok(client.clear_token().await)
}

/// 检查本地插件目录中是否已安装同名插件（按 manifest.name 目录名）
fn find_local_install(manifest_name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    if !safe_plugin_name(manifest_name) {
        return None;
    }
    for dir in dirs {
        let candidate = dir.join(manifest_name);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

#[command(rename_all = "camelCase")]
pub async fn discover_github_plugins(
    app_handle: tauri::AppHandle,
    client: State<'_, GitHubClient>,
    page: Option<u32>,
    per_page: Option<u32>,
) -> Result<DiscoverPluginsResult, GitHubApiError> {
    discover(
        &client,
        page.unwrap_or(1).max(1),
        per_page.unwrap_or(20).clamp(1, 100),
        &crate::commands::plugins::plugin_dirs(&app_handle),
    )
    .await
}

fn upsert_rate_snapshot(rates: &mut Vec<GitHubRateInfo>, rate: GitHubRateInfo) {
    if let Some(existing) = rates
        .iter_mut()
        .find(|entry| entry.resource == rate.resource)
    {
        *existing = rate;
    } else {
        rates.push(rate);
    }
}

pub(crate) async fn discover(
    client: &GitHubClient,
    page: u32,
    per_page: u32,
    plugin_dirs: &[PathBuf],
) -> Result<DiscoverPluginsResult, GitHubApiError> {
    let (repos, total_count, search_rate, search_rate_limits) =
        search_repositories(client, page, per_page).await?;
    // The discovery headline is the *search* bucket: keep the first observed search
    // snapshot for `rateLimit`, but record every resource snapshot in `rateLimits`.
    let mut rate_limits: Vec<GitHubRateInfo> = search_rate_limits;
    let mut discovered = Vec::with_capacity(repos.len());
    for repo in repos {
        let manifest_result =
            fetch_plugin_manifest(client, &repo.owner.login, &repo.name, &repo.default_branch)
                .await;
        let (manifest, manifest_error, local_path) = match manifest_result {
            Ok((Some(manifest), rate)) => {
                if let Some(rate) = rate {
                    upsert_rate_snapshot(&mut rate_limits, rate);
                }
                let local = find_local_install(&manifest.name, plugin_dirs);
                (Some(manifest), None, local)
            }
            Ok((None, rate)) => {
                if let Some(rate) = rate {
                    upsert_rate_snapshot(&mut rate_limits, rate);
                }
                (None, None, None)
            }
            // A manifest throttle stops discovery immediately; never hide it in one card.
            Err(error) if error.is_rate_limit() || error.code != ErrorCode::ClientError => {
                return Err(error)
            }
            Err(error) => {
                if let Some(rate) = error.rate_limit {
                    upsert_rate_snapshot(&mut rate_limits, *rate);
                }
                (None, Some(error.message), None)
            }
        };
        let verification = if repo.owner.login == "H2SYJ" {
            "verified"
        } else if manifest.is_some() {
            "community"
        } else {
            "unverified"
        }
        .to_owned();
        discovered.push(DiscoveredPlugin {
            repository: repo,
            manifest,
            manifest_error,
            is_installed: local_path.is_some(),
            local_path: local_path.map(|p| p.display().to_string()),
            verification,
        });
    }
    // Keep `rateLimit` on the *search* bucket for IPC v1 compatibility; `rateLimits`
    // is the additive per-resource ledger. Manifest (core) responses never replace it.
    Ok(DiscoverPluginsResult {
        schema_version: "1.0.0",
        plugins: discovered,
        total_count,
        rate_limit: search_rate,
        rate_limits,
    })
}

/// Extract only regular files below the single GitHub archive root. Validate the original
/// spelling before Path normalization so double separators, drive paths and `..` cannot hide.
fn extract_zip_to_dir(zip_bytes: &[u8], target_dir: &Path) -> Result<(), String> {
    extract_zip_with_limits(zip_bytes, target_dir, EXTRACTION_LIMITS)
}

fn extract_zip_with_limits(
    zip_bytes: &[u8],
    target_dir: &Path,
    limits: ExtractionLimits,
) -> Result<(), String> {
    let mut archive =
        ZipArchive::new(Cursor::new(zip_bytes)).map_err(|_| "解析 ZIP 失败".to_owned())?;
    if archive.len() > limits.entries {
        return Err("ZIP 条目数量超过安全限制".into());
    }
    let mut extracted_bytes = 0;
    let mut archive_root: Option<String> = None;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|_| "读取 ZIP 条目失败".to_owned())?;
        let name = file.name().to_owned();
        let parts: Vec<_> = name.trim_end_matches('/').split('/').collect();
        if name.starts_with('/')
            || name.contains(['\\', ':', '\0'])
            || parts
                .iter()
                .any(|part| part.is_empty() || matches!(*part, "." | ".."))
            || file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("ZIP 包含不安全的条目".into());
        }
        if let Some(root) = &archive_root {
            if root != parts[0] {
                return Err("ZIP 顶层目录不一致".into());
            }
        } else {
            archive_root = Some(parts[0].to_owned());
        }
        if file.is_dir() {
            continue;
        }
        if parts.len() < 2 {
            return Err("ZIP 文件不在仓库根目录内".into());
        }
        let relative: PathBuf = parts[1..].iter().collect();
        if !relative
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
        {
            return Err("ZIP 路径不合法".into());
        }
        let out = target_dir.join(relative);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|_| "创建插件子目录失败".to_owned())?;
        }
        let allowed = limits
            .entry_bytes
            .min(limits.total_bytes.saturating_sub(extracted_bytes));
        if file.size() > allowed {
            return Err("ZIP 解压大小超过安全限制".into());
        }
        let mut output = std::fs::File::create(&out).map_err(|_| "创建插件文件失败".to_owned())?;
        // Stream to disk and check actual bytes as well as the untrusted ZIP size metadata.
        let copied = std::io::copy(
            &mut (&mut file).take(allowed.saturating_add(1)),
            &mut output,
        )
        .map_err(|_| "读取或写入 ZIP 内容失败".to_owned())?;
        if copied > allowed {
            return Err("ZIP 解压大小超过安全限制".into());
        }
        extracted_bytes += copied;
    }
    Ok(())
}

fn safe_plugin_name(name: &str) -> bool {
    !name.is_empty()
        && !matches!(name, "." | "..")
        // Same policy as local PluginManifest::validate().
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// Download and validation finish before touching an existing install. Rename failure rolls
/// back the old directory; a throttle/network failure never creates backups or empty installs.
fn activate_archive(
    zip: &[u8],
    manifest: &PluginManifest,
    primary: &Path,
) -> Result<PathBuf, GitHubApiError> {
    if !safe_plugin_name(&manifest.name) {
        return Err(GitHubApiError::client("插件名称不能作为安全目录名"));
    }
    std::fs::create_dir_all(primary).map_err(|_| GitHubApiError::client("创建插件根目录失败"))?;
    // Discovery scans only direct children. Keep transaction folders one level deeper so
    // neither a retained backup nor an interrupted staging directory can shadow a plugin.
    let workspace = primary.join(".timepin-installs");
    std::fs::create_dir_all(&workspace)
        .map_err(|_| GitHubApiError::client("创建插件事务目录失败"))?;
    let staging = workspace.join(format!("stage-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&staging).map_err(|_| GitHubApiError::client("创建插件暂存目录失败"))?;
    let result = (|| {
        extract_zip_to_dir(zip, &staging).map_err(GitHubApiError::client)?;
        let text = std::fs::read_to_string(staging.join("plugin.json"))
            .map_err(|_| GitHubApiError::client("解压后未找到有效的 plugin.json"))?;
        let extracted = PluginManifest::parse(&text).map_err(GitHubApiError::client)?;
        let local = extracted.to_local().map_err(GitHubApiError::client)?;
        if extracted.name != manifest.name || !local.validate().is_empty() {
            return Err(GitHubApiError::client(
                "归档中的 manifest 与安装目标不匹配或校验失败",
            ));
        }
        let target = primary.join(&manifest.name);
        let backup = workspace.join(format!("{}.bak-{}", manifest.name, uuid::Uuid::new_v4()));
        let had_existing = target.exists();
        if had_existing {
            std::fs::rename(&target, &backup)
                .map_err(|_| GitHubApiError::client("备份旧插件目录失败"))?;
        }
        if std::fs::rename(&staging, &target).is_err() {
            if had_existing && std::fs::rename(&backup, &target).is_err() {
                return Err(GitHubApiError::client(
                    "安装切换失败，旧插件已保留在 .timepin-installs 下的 .bak 目录，请手动恢复",
                ));
            }
            return Err(GitHubApiError::client("安装切换失败，旧插件已恢复"));
        }
        Ok(target)
    })();
    if staging.exists() {
        let _ = std::fs::remove_dir_all(staging);
    }
    // Only remove an empty transaction root; retained backups are never deleted here.
    let _ = std::fs::remove_dir(workspace);
    result
}

#[command(rename_all = "camelCase")]
pub async fn install_github_plugin(
    app_handle: tauri::AppHandle,
    client: State<'_, GitHubClient>,
    manager: State<'_, crate::plugins::manager::PluginManager>,
    owner: String,
    repo: String,
    branch: Option<String>,
) -> Result<String, GitHubApiError> {
    let branch = branch.unwrap_or_else(|| "main".to_owned());
    let (manifest, rate_limit) = fetch_plugin_manifest(&client, &owner, &repo, &branch).await?;
    let manifest = manifest.ok_or_else(|| {
        let mut error = GitHubApiError::new(
            ErrorCode::NotFoundOrPrivate,
            "仓库中未找到 plugin.json，或当前身份无权访问",
            crate::core::github::Action::None,
        );
        error.rate_limit = rate_limit.map(Box::new);
        error
    })?;
    let errors = manifest
        .to_local()
        .map_err(GitHubApiError::client)?
        .validate();
    if !errors.is_empty() || !safe_plugin_name(&manifest.name) {
        return Err(GitHubApiError::client(
            "manifest 校验失败或插件目录名不安全",
        ));
    }
    // Bundled search roots can be read-only (e.g. Program Files). Community installs
    // always use the writable, app-owned root, not the first discovery directory.
    let primary = crate::core::get_default_plugins_dir(&app_handle);
    let url = api_url(&["repos", &owner, &repo, "zipball", &branch], &[])?;
    let archive = client.public_zip(url).await?;
    let plugin_name = manifest.name.clone();
    let plugin_dir = tauri::async_runtime::spawn_blocking(move || {
        activate_archive(&archive.body, &manifest, &primary)
    })
    .await
    .map_err(|_| GitHubApiError::client("插件安装任务失败"))??;
    // The archive is on disk now; register it without stopping running plugins
    // so `get_plugins` reflects the install immediately after this call returns.
    let plugin_dirs = crate::commands::plugins::plugin_dirs(&app_handle);
    manager.register_new(&plugin_dirs);
    let _ = app_handle.emit(
        crate::plugins::manager::EVENT_PLUGIN_STATUS,
        serde_json::json!({ "pluginName": plugin_name, "status": "ready", "detail": "installed via GitHub" }),
    );
    // The UI merges this into DiscoveredPlugin.localPath, so return the path only.
    Ok(plugin_dir.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::FileOptions;

    fn build_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            for (name, contents) in entries {
                zip.start_file(*name, FileOptions::default()).unwrap();
                zip.write_all(contents).unwrap();
            }
            zip.finish().unwrap();
        }
        buf
    }

    fn valid_manifest() -> PluginManifest {
        PluginManifest::parse(r#"{"protocolVersion":"1.0.0","name":"example-plugin","version":"1.0.0","description":"test","author":"test","entry":"main.py","runtime":"python","capabilities":["metadata"],"parametersSchema":{},"outputSchema":{}}"#).unwrap()
    }

    #[test]
    fn invalid_archive_does_not_move_or_backup_existing_install() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = valid_manifest();
        let existing = tmp.path().join(&manifest.name);
        std::fs::create_dir(&existing).unwrap();
        std::fs::write(existing.join("old.py"), b"original").unwrap();
        for zip in [
            b"not a zip".to_vec(),
            build_zip(&[("root/missing.txt", b"bad")]),
            build_zip(&[("root/../../escape", b"bad")]),
        ] {
            assert!(activate_archive(&zip, &manifest, tmp.path()).is_err());
            assert_eq!(std::fs::read(existing.join("old.py")).unwrap(), b"original");
            assert_eq!(
                std::fs::read_dir(tmp.path()).unwrap().count(),
                1,
                "no backup or staging should remain"
            );
        }
    }

    #[test]
    fn validated_archive_is_staged_then_replaces_existing_install_with_backup() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = valid_manifest();
        let existing = tmp.path().join(&manifest.name);
        std::fs::create_dir(&existing).unwrap();
        std::fs::write(existing.join("old.py"), b"original").unwrap();
        let json = serde_json::to_vec(&manifest).unwrap();
        std::fs::write(existing.join("plugin.json"), &json).unwrap();
        let zip = build_zip(&[
            ("root/plugin.json", &json),
            ("root/main.py", b"print('new')"),
        ]);
        assert_eq!(
            activate_archive(&zip, &manifest, tmp.path()).unwrap(),
            existing
        );
        assert!(!existing.join("old.py").exists());
        assert_eq!(
            std::fs::read(existing.join("main.py")).unwrap(),
            b"print('new')"
        );
        let workspace = tmp.path().join(".timepin-installs");
        let backups: Vec<_> = std::fs::read_dir(&workspace)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains(".bak-")
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(
            std::fs::read(backups[0].join("old.py")).unwrap(),
            b"original"
        );
        assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 2);
        // An interrupted extraction with a valid manifest is not a direct plugin child.
        let interrupted = workspace.join("stage-interrupted");
        std::fs::create_dir(&interrupted).unwrap();
        std::fs::write(interrupted.join("plugin.json"), &json).unwrap();
        let discoverable: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .flatten()
            .filter_map(|entry| crate::plugins::manifest::PluginManifest::load(&entry.path()).ok())
            .map(|manifest| manifest.name)
            .collect();
        assert_eq!(discoverable, vec!["example-plugin"]);
    }

    #[test]
    fn zip_limits_bound_entry_count_individual_size_and_expanded_total() {
        let zip = build_zip(&[("root/a.txt", b"123"), ("root/b.txt", b"456")]);
        for limits in [
            ExtractionLimits {
                entries: 1,
                entry_bytes: 10,
                total_bytes: 10,
            },
            ExtractionLimits {
                entries: 10,
                entry_bytes: 2,
                total_bytes: 10,
            },
            ExtractionLimits {
                entries: 10,
                entry_bytes: 10,
                total_bytes: 5,
            },
        ] {
            let tmp = tempfile::tempdir().unwrap();
            assert!(extract_zip_with_limits(&zip, tmp.path(), limits).is_err());
        }
        let tmp = tempfile::tempdir().unwrap();
        extract_zip_with_limits(
            &zip,
            tmp.path(),
            ExtractionLimits {
                entries: 2,
                entry_bytes: 3,
                total_bytes: 6,
            },
        )
        .unwrap();
        assert_eq!(std::fs::read(tmp.path().join("b.txt")).unwrap(), b"456");
    }

    #[test]
    fn manifest_and_repository_names_cannot_escape_plugin_root() {
        for name in [
            "../escape",
            "..",
            "/absolute",
            "C:evil",
            "foo/bar",
            "foo\\bar",
            "bad.name",
            "bad.",
        ] {
            assert!(!safe_plugin_name(name));
            assert!(find_local_install(name, &[]).is_none());
        }
        assert!(validate_repository("owner?x=1", "repo").is_err());
        assert!(validate_repository("owner", "repo/../../x").is_err());
    }

    #[test]
    fn zip_extract_strips_top_level_dir() {
        let zip = build_zip(&[
            ("owner-repo-abc1234/plugin.json", b"{\"name\":\"x\"}"),
            ("owner-repo-abc1234/src/main.py", b"print('hi')"),
        ]);
        let tmp = tempfile::tempdir().unwrap();
        extract_zip_to_dir(&zip, tmp.path()).unwrap();
        assert!(tmp.path().join("plugin.json").is_file());
        assert!(tmp.path().join("src/main.py").is_file());
    }

    #[test]
    fn zip_extract_rejects_path_traversal() {
        // 直接构造一个包含 ../ 的恶意条目
        let zip = build_zip(&[("owner-repo-abc1234/../../escape.py", b"mal")]);
        let tmp = tempfile::tempdir().unwrap();
        let res = extract_zip_to_dir(&zip, tmp.path());
        assert!(res.is_err(), "expected traversal rejection, got: {res:?}");
    }

    #[test]
    fn zip_extract_rejects_absolute_path() {
        let zip = build_zip(&[("owner-repo-abc1234//abs/evil.py", b"mal")]);
        let tmp = tempfile::tempdir().unwrap();
        // "/abs/..." 在 Path::components 中是 RootDir + Normal，应被拒绝
        let res = extract_zip_to_dir(&zip, tmp.path());
        assert!(
            res.is_err(),
            "absolute/double-separator entry must be rejected"
        );
    }
}
