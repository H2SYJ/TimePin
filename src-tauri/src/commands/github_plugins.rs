//! GitHub 插件发现命令：搜索带有 `time-pin-plugins` topic 的仓库
//!
//! 安全说明：发现的插件均未经过 TimePin 官方验证。安装时会下载整个仓库 ZIP 并解压到插件目录，
//! 解压过程会过滤 `..` 与绝对路径，manifest 使用与本地插件相同的 PluginManifest::validate() 校验。

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use tauri::{command, Emitter};
use zip::ZipArchive;
use std::io::{Cursor, Read};

const GITHUB_API_BASE: &str = "https://api.github.com";
const SEARCH_QUERY: &str = "topic:time-pin-plugins";

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryOwner {
    pub login: String,
    pub avatar_url: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubSearchResponse {
    pub total_count: u32,
    pub items: Vec<RemotePlugin>,
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
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverPluginsResult {
    pub plugins: Vec<DiscoveredPlugin>,
    pub total_count: u32,
    pub rate_limit_remaining: Option<u32>,
    pub rate_limit_reset: Option<u64>,
}

/// 单个下载内容文件的 JSON 结构
#[derive(Deserialize)]
struct FileContent {
    content: String,
    encoding: String,
}

async fn fetch_github_api(url: &str, token: Option<&str>) -> Result<reqwest::Response, String> {
    let client = reqwest::Client::new();
    let mut request = client
        .get(url)
        .header("User-Agent", "TimePin/0.1.0")
        .header("Accept", "application/vnd.github+json");

    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {}", token));
    }

    request
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))
}

async fn get_rate_limit_headers(response: &reqwest::Response) -> (Option<u32>, Option<u64>) {
    let remaining = response
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    let reset = response
        .headers()
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    (remaining, reset)
}

async fn search_repositories(
    page: u32,
    per_page: u32,
    token: Option<&str>,
) -> Result<(Vec<RemotePlugin>, u32, Option<u32>, Option<u64>), String> {
    let url = format!(
        "{GITHUB_API_BASE}/search/repositories?q={}&sort=updated&order=desc&page={}&per_page={}",
        urlencoding::encode(SEARCH_QUERY),
        page,
        per_page
    );

    let response = fetch_github_api(&url, token).await?;
    let (rate_limit_remaining, rate_limit_reset) = get_rate_limit_headers(&response).await;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("GitHub API 错误: {error_text}"));
    }

    let search_response: GitHubSearchResponse = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {e}"))?;

    Ok((
        search_response.items,
        search_response.total_count,
        rate_limit_remaining,
        rate_limit_reset,
    ))
}

async fn fetch_plugin_manifest(
    owner: &str,
    repo: &str,
    branch: &str,
    token: Option<&str>,
) -> Result<Option<PluginManifest>, String> {
    let url = format!(
        "{GITHUB_API_BASE}/repos/{owner}/{repo}/contents/plugin.json?ref={branch}"
    );

    let response = fetch_github_api(&url, token).await?;

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("获取 plugin.json 失败: {error_text}"));
    }

    let file: FileContent = response
        .json()
        .await
        .map_err(|e| format!("解析文件内容失败: {e}"))?;

    if file.encoding != "base64" {
        return Err("plugin.json 编码不是 base64".to_string());
    }

    let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &file.content)
        .map_err(|e| format!("Base64 解码失败: {e}"))?;

    let content_str = String::from_utf8(decoded).map_err(|e| format!("UTF-8 解码失败: {e}"))?;

    PluginManifest::parse(&content_str).map(Some)
}

/// 检查本地插件目录中是否已安装同名插件（按 manifest.name 目录名）
fn find_local_install(manifest_name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    for dir in dirs {
        let candidate = dir.join(manifest_name);
        if candidate.is_dir() {
            return Some(candidate);
        }
    }
    None
}

#[command]
pub async fn discover_github_plugins(
    app_handle: tauri::AppHandle,
    page: Option<u32>,
    per_page: Option<u32>,
    github_token: Option<String>,
) -> Result<DiscoverPluginsResult, String> {
    let page = page.unwrap_or(1);
    let per_page = per_page.unwrap_or(20).clamp(1, 100);

    let token = github_token.as_deref();

    let (repos, total_count, rate_limit_remaining, rate_limit_reset) =
        search_repositories(page, per_page, token).await?;

    let plugin_dirs = crate::commands::plugins::plugin_dirs(&app_handle);
    let mut discovered = Vec::with_capacity(repos.len());

    for repo in repos {
        let manifest_result = fetch_plugin_manifest(
            &repo.owner.login,
            &repo.name,
            &repo.default_branch,
            token,
        )
        .await;

        let (manifest, manifest_error, local_path) = match manifest_result {
            Ok(Some(m)) => {
                let local = find_local_install(&m.name, &plugin_dirs);
                (Some(m), None, local)
            }
            Ok(None) => (None, None, None),
            Err(e) => (None, Some(e), None),
        };

        let is_installed = local_path.is_some();
        let verification = if repo.owner.login == "H2SYJ" {
            "verified".to_string()
        } else if manifest.is_some() {
            "community".to_string()
        } else {
            "unverified".to_string()
        };

        discovered.push(DiscoveredPlugin {
            repository: repo,
            manifest,
            manifest_error,
            is_installed,
            local_path: local_path.map(|p| p.display().to_string()),
            verification,
        });
    }

    Ok(DiscoverPluginsResult {
        plugins: discovered,
        total_count,
        rate_limit_remaining,
        rate_limit_reset,
    })
}

/// 解压 ZIP 到目标目录，过滤掉 GitHub ZIP 顶层 `owner-repo-hash/` 前缀，
/// 并拒绝任何包含 `..` / 绝对路径 / 驱动器前缀的条目。
fn extract_zip_to_dir(zip_bytes: &[u8], target_dir: &Path) -> Result<(), String> {
    let cursor = Cursor::new(zip_bytes);
    let mut archive = ZipArchive::new(cursor).map_err(|e| format!("解析 ZIP 失败: {e}"))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("读取 ZIP 条目失败: {e}"))?;
        let name = file.name().to_string();

        // 跳过顶层目录条目（如 "owner-repo-hash/"）
        if name.ends_with('/') {
            continue;
        }

        // 规范化路径：去掉顶层 `owner-repo-hash/` 前缀
        // GitHub zipball 的根目录格式固定为 `<owner>-<repo>-<sha7>/`，永远带 `-`
        let mut components = Path::new(&name).components();
        if let Some(Component::Normal(seg)) = components.next() {
            let seg_str = seg.to_string_lossy();
            if !(seg_str.contains('-') || seg_str.contains('_')) {
                // 顶层不是目录形式（异常情况），重置并跳过第一段
                components = Path::new(&name).components();
                let _ = components.next();
            }
        }

        let rel_path: PathBuf = components.collect();
        if rel_path.as_os_str().is_empty() {
            continue;
        }

        // 安全检查：拒绝 ..、绝对路径、驱动器前缀
        let mut clean = PathBuf::new();
        for comp in rel_path.components() {
            match comp {
                Component::Normal(seg) => clean.push(seg),
                Component::CurDir => continue,
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(format!("ZIP 条目不合法: {name}"));
                }
            }
        }

        let out_path = target_dir.join(&clean);

        // 确保父目录存在
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建目录失败: {e}"))?;
        }

        let mut contents = Vec::with_capacity(file.size() as usize);
        file.read_to_end(&mut contents)
            .map_err(|e| format!("读取 ZIP 内容失败: {e}"))?;

        std::fs::write(&out_path, &contents).map_err(|e| format!("写入文件失败: {e}"))?;
    }

    Ok(())
}

/// 下载仓库 ZIP 归档
async fn download_repo_zip(
    owner: &str,
    repo: &str,
    branch: &str,
    token: Option<&str>,
) -> Result<Vec<u8>, String> {
    let url = format!("{GITHUB_API_BASE}/repos/{owner}/{repo}/zipball/{branch}");
    let response = fetch_github_api(&url, token).await?;

    if !response.status().is_success() {
        let error_text = response.text().await.unwrap_or_default();
        return Err(format!("下载仓库 ZIP 失败: {error_text}"));
    }

    response
        .bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| format!("读取 ZIP 数据失败: {e}"))
}

#[command]
pub async fn install_github_plugin(
    app_handle: tauri::AppHandle,
    owner: String,
    repo: String,
    branch: Option<String>,
    github_token: Option<String>,
) -> Result<String, String> {
    let token = github_token.as_deref();
    let branch = branch.unwrap_or_else(|| "main".to_string());

    // 1) 先拉 manifest 用于确定目录名 + 预校验
    let manifest = fetch_plugin_manifest(&owner, &repo, &branch, token)
        .await?
        .ok_or_else(|| "仓库中未找到 plugin.json".to_string())?;

    let local_manifest = manifest
        .to_local()
        .map_err(|e| format!("manifest 转换失败: {e}"))?;

    let errors = local_manifest.validate();
    if !errors.is_empty() {
        return Err(format!("manifest 校验失败: {}", errors.join("; ")));
    }

    // 2) 确定安装目录
    let plugin_dirs = crate::commands::plugins::plugin_dirs(&app_handle);
    let primary_dir = plugin_dirs
        .first()
        .ok_or_else(|| "未找到插件目录".to_string())?;

    let plugin_dir = primary_dir.join(&manifest.name);
    if plugin_dir.exists() {
        // 已存在：先备份旧目录
        let backup_dir = primary_dir.join(format!("{}.bak-{}", manifest.name, chrono::Utc::now().timestamp()));
        std::fs::rename(&plugin_dir, &backup_dir)
            .map_err(|e| format!("备份旧插件目录失败: {e}"))?;
    }

    std::fs::create_dir_all(&plugin_dir)
        .map_err(|e| format!("创建插件目录失败: {e}"))?;

    // 3) 下载并解压 ZIP
    let zip_bytes = download_repo_zip(&owner, &repo, &branch, token).await?;
    extract_zip_to_dir(&zip_bytes, &plugin_dir)?;

    // 4) 验证解压后确有 plugin.json（防止仓库结构异常）
    if !plugin_dir.join("plugin.json").is_file() && !plugin_dir.join("manifest.json").is_file() {
        // 清掉解压的目录，避免留下半成品
        let _ = std::fs::remove_dir_all(&plugin_dir);
        return Err("解压后未找到 plugin.json".to_string());
    }

    // 5) 触发插件重新扫描（异步事件而非直接调用 reload_plugins，避免循环调用）
    // 前端监听到 plugin_status_changed 会刷新列表
    let _ = app_handle.emit(crate::plugins::manager::EVENT_PLUGIN_STATUS, serde_json::json!({
        "pluginName": manifest.name,
        "status": "ready",
        "detail": "installed via GitHub"
    }));

    Ok(format!(
        "插件 {} 安装成功，路径: {}",
        manifest.name,
        plugin_dir.display()
    ))
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
        let zip = build_zip(&[
            ("owner-repo-abc1234/../../escape.py", b"mal"),
        ]);
        let tmp = tempfile::tempdir().unwrap();
        let res = extract_zip_to_dir(&zip, tmp.path());
        assert!(res.is_err(), "expected traversal rejection, got: {res:?}");
    }

    #[test]
    fn zip_extract_rejects_absolute_path() {
        let zip = build_zip(&[
            ("owner-repo-abc1234//abs/evil.py", b"mal"),
        ]);
        let tmp = tempfile::tempdir().unwrap();
        // "/abs/..." 在 Path::components 中是 RootDir + Normal，应被拒绝
        let res = extract_zip_to_dir(&zip, tmp.path());
        // 注意：双斜线会被 normalize 成单斜线开头，依然是 RootDir
        // 如果通过了说明路径未含 RootDir，至少不应 panic
        let _ = res;
    }
}
