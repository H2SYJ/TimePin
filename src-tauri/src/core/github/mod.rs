//! Shared, backend-only GitHub session. Never derive Debug for a type holding credentials.
//! One mutex covers admission, HTTP and state updates: expired limits get a single probe,
//! not a burst of independently admitted requests. There are no automatic rate-limit retries.

use reqwest::{header::HeaderMap, StatusCode, Url};
use serde::Serialize;
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::sync::Mutex;

const SCHEMA_VERSION: &str = "1.0.0";
const USER_AGENT: &str = concat!("TimePin/", env!("CARGO_PKG_VERSION"));
const SECONDARY_MIN_MS: i64 = 60_000;
const SECONDARY_MAX_MS: i64 = 15 * 60_000;
// Bound both JSON and archive responses, including chunked bodies without Content-Length.
const MAX_RESPONSE_BYTES: usize = 128 * 1024 * 1024;

fn append_response_chunk(
    body: &mut Vec<u8>,
    chunk: &[u8],
    limit: usize,
) -> Result<(), GitHubApiError> {
    if body.len() > limit || chunk.len() > limit.saturating_sub(body.len()) {
        return Err(GitHubApiError::client("GitHub 响应超过 128 MiB 安全限制"));
    }
    body.extend_from_slice(chunk);
    Ok(())
}

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitHubRateInfo {
    pub limit: Option<u64>,
    pub remaining: Option<u64>,
    pub reset_at: Option<i64>,
    pub resource: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    AuthInvalid,
    PrimaryRateLimit,
    SecondaryRateLimit,
    RateLimitUnknown,
    RateWait,
    NotFoundOrPrivate,
    Forbidden,
    ClientError,
    ServerError,
    NetworkError,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Action {
    Wait,
    ConfigureToken,
    Reauthenticate,
    None,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubApiError {
    pub schema_version: &'static str,
    pub code: ErrorCode,
    pub message: String,
    pub retry_at: Option<i64>,
    pub action: Action,
    // Box keeps Result's error variant small; serde preserves the exact IPC object shape.
    pub rate_limit: Option<Box<GitHubRateInfo>>,
}

impl GitHubApiError {
    pub fn client(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ClientError, message, Action::None)
    }

    pub(crate) fn new(code: ErrorCode, message: impl Into<String>, action: Action) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            code,
            message: message.into(),
            retry_at: None,
            action,
            rate_limit: None,
        }
    }

    pub fn is_rate_limit(&self) -> bool {
        matches!(
            self.code,
            ErrorCode::PrimaryRateLimit
                | ErrorCode::SecondaryRateLimit
                | ErrorCode::RateLimitUnknown
                | ErrorCode::RateWait
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Identity {
    Anonymous,
    Authenticated,
    TokenInvalid,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubSession {
    pub schema_version: &'static str,
    pub identity: Identity,
    pub remembered: bool,
    pub storage_warning: Option<String>,
    pub rate_limit: Option<GitHubRateInfo>,
}

trait CredentialStore: Send + Sync {
    fn load(&self) -> Result<Option<String>, ()>;
    fn save(&self, token: &str) -> Result<(), ()>;
    fn delete(&self) -> Result<(), ()>;
}

struct SystemCredentials;
impl SystemCredentials {
    fn entry() -> Result<keyring::Entry, ()> {
        keyring::Entry::new("ai.timepin.github", "github-token").map_err(|_| ())
    }
}
impl CredentialStore for SystemCredentials {
    fn load(&self) -> Result<Option<String>, ()> {
        match Self::entry()?.get_password() {
            Ok(token) => Ok(Some(token)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(()),
        }
    }
    fn save(&self, token: &str) -> Result<(), ()> {
        Self::entry()?.set_password(token).map_err(|_| ())
    }
    fn delete(&self) -> Result<(), ()> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(()),
        }
    }
}

// Private transport boundary enables deterministic, offline tests without relaxing URL policy.
struct Request {
    url: Url,
    token: Option<String>,
}
struct Response {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}
type SendFuture<'a> = Pin<Box<dyn Future<Output = Result<Response, GitHubApiError>> + Send + 'a>>;
trait Transport: Send + Sync {
    fn send(&self, request: Request) -> SendFuture<'_>;
}
struct HttpTransport(reqwest::Client);
impl HttpTransport {
    fn prepare(&self, request: Request) -> Result<reqwest::Request, GitHubApiError> {
        if request.token.is_some() && !is_api_url(&request.url) {
            return Err(GitHubApiError::client("拒绝向非 GitHub API 地址发送凭据"));
        }
        let mut builder = self
            .0
            .get(request.url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = request.token {
            // Sensitive values are redacted even from reqwest's internal HeaderMap Debug.
            let mut value = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| GitHubApiError::client("GitHub 凭据格式不合法"))?;
            value.set_sensitive(true);
            builder = builder.header(reqwest::header::AUTHORIZATION, value);
        }
        builder
            .build()
            .map_err(|_| GitHubApiError::client("无法构建 GitHub 请求"))
    }
}
impl Transport for HttpTransport {
    fn send(&self, request: Request) -> SendFuture<'_> {
        Box::pin(async move {
            let request = self.prepare(request)?;
            let mut response = self.0.execute(request).await.map_err(|_| {
                GitHubApiError::new(
                    ErrorCode::NetworkError,
                    "无法连接 GitHub，请检查网络后重试",
                    Action::None,
                )
            })?;
            let status = response.status();
            let headers = response.headers().clone();
            if response
                .content_length()
                .is_some_and(|size| size > MAX_RESPONSE_BYTES as u64)
            {
                return Err(GitHubApiError::client("GitHub 响应超过 128 MiB 安全限制"));
            }
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| {
                GitHubApiError::new(
                    ErrorCode::NetworkError,
                    "读取 GitHub 响应失败，请重试",
                    Action::None,
                )
            })? {
                append_response_chunk(&mut body, &chunk, MAX_RESPONSE_BYTES)?;
            }
            Ok(Response {
                status,
                headers,
                body,
            })
        })
    }
}

#[derive(Default)]
struct Gate {
    primary_until: Option<i64>,
    secondary_until: Option<i64>,
    unknown_until: Option<i64>,
    secondary_failures: u32,
    stopped: bool,
}
impl Gate {
    fn check(
        &self,
        now: i64,
        candidate_over_anonymous_primary: bool,
        rate_limit: Option<GitHubRateInfo>,
    ) -> Result<(), GitHubApiError> {
        let retry_at = [
            if candidate_over_anonymous_primary {
                None
            } else {
                self.primary_until
            },
            self.secondary_until,
            self.unknown_until,
        ]
        .into_iter()
        .flatten()
        .filter(|until| *until > now)
        .max();
        if self.stopped || retry_at.is_some() {
            return Err(GitHubApiError {
                schema_version: SCHEMA_VERSION,
                code: ErrorCode::RateWait,
                message: if self.stopped {
                    "GitHub 连续三次触发二级限流，本次会话已停止请求，请稍后重启应用再试".into()
                } else {
                    "GitHub 请求处于冷却期，请等待后重试".into()
                },
                retry_at: if self.stopped { None } else { retry_at },
                action: if self.stopped {
                    Action::None
                } else {
                    Action::Wait
                },
                rate_limit: rate_limit.map(Box::new),
            });
        }
        Ok(())
    }

    fn observe(&mut self, error: &mut GitHubApiError, now: i64) {
        match error.code {
            ErrorCode::PrimaryRateLimit => {
                self.primary_until = Some(error.retry_at.unwrap_or(now + SECONDARY_MIN_MS));
                error.retry_at = self.primary_until;
            }
            ErrorCode::SecondaryRateLimit => {
                self.secondary_failures += 1;
                let backoff = (SECONDARY_MIN_MS
                    * 2_i64.pow(self.secondary_failures.saturating_sub(1).min(4)))
                .min(SECONDARY_MAX_MS);
                let until = error.retry_at.unwrap_or(now).max(now + backoff);
                self.secondary_until = Some(until);
                error.retry_at = Some(until);
                if self.secondary_failures >= 3 {
                    self.stopped = true;
                    error.action = Action::None;
                    error.retry_at = None;
                    error.message = "GitHub 连续三次触发二级限流，本次会话已停止请求".into();
                }
            }
            ErrorCode::RateLimitUnknown => {
                let until = error.retry_at.unwrap_or(now + SECONDARY_MIN_MS);
                self.unknown_until = Some(until.max(now + SECONDARY_MIN_MS));
                error.retry_at = self.unknown_until;
            }
            _ => {}
        }
    }

    fn success(&mut self, rate: Option<&GitHubRateInfo>, now: i64) {
        // A success from another resource must not reopen an unexpired primary gate.
        self.primary_until = self.primary_until.filter(|until| *until > now);
        if let Some(rate) = rate.filter(|r| r.remaining == Some(0)) {
            let until = rate.reset_at.unwrap_or(now + SECONDARY_MIN_MS).max(now);
            self.primary_until = Some(self.primary_until.unwrap_or(now).max(until));
        }
        self.secondary_until = None;
        self.unknown_until = None;
        self.secondary_failures = 0;
    }
}

struct SessionState {
    token: Option<String>,
    identity: Identity,
    remembered: bool,
    storage_warning: Option<String>,
    rate_limit: Option<GitHubRateInfo>,
    gate: Gate,
}
impl SessionState {
    fn session(&self) -> GitHubSession {
        GitHubSession {
            schema_version: SCHEMA_VERSION,
            identity: self.identity,
            remembered: self.remembered,
            storage_warning: self.storage_warning.clone(),
            rate_limit: self.rate_limit.clone(),
        }
    }
}

pub struct GitHubClient {
    transport: Arc<dyn Transport>,
    credentials: Arc<dyn CredentialStore>,
    state: Mutex<SessionState>,
}

/// Response carries its own final rate snapshot; callers never race a separate session read.
#[derive(Debug)]
pub struct GitHubResponse {
    pub body: Vec<u8>,
    pub rate_limit: Option<GitHubRateInfo>,
}

impl GitHubClient {
    pub fn new() -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self::with_parts(
            Arc::new(HttpTransport(http)),
            Arc::new(SystemCredentials),
        ))
    }

    fn with_parts(transport: Arc<dyn Transport>, credentials: Arc<dyn CredentialStore>) -> Self {
        let (token, storage_warning) = match credentials.load() {
            Ok(Some(token)) if valid_token(&token) => (Some(token), None),
            Ok(None) => (None, None),
            Ok(Some(_)) => {
                let warning = if credentials.delete().is_ok() {
                    "已移除格式不合法的 GitHub 凭据，当前使用匿名会话"
                } else {
                    "已停用格式不合法的 GitHub 凭据，但无法从系统凭据库删除；请在下次启动前手动移除"
                };
                (None, Some(warning.into()))
            }
            Err(()) => (
                None,
                Some("无法读取系统 GitHub 凭据，当前使用匿名会话".into()),
            ),
        };
        let remembered = token.is_some();
        let identity = if remembered {
            Identity::Authenticated
        } else {
            Identity::Anonymous
        };
        Self {
            transport,
            credentials,
            state: Mutex::new(SessionState {
                token,
                identity,
                remembered,
                storage_warning,
                rate_limit: None,
                gate: Gate::default(),
            }),
        }
    }

    pub async fn session(&self) -> GitHubSession {
        self.state.lock().await.session()
    }

    pub async fn clear_token(&self) -> GitHubSession {
        let mut state = self.state.lock().await;
        let changed = state.token.is_some();
        self.revoke(&mut state, Identity::Anonymous);
        if changed {
            state.gate.primary_until = None;
            state.rate_limit = None;
        }
        state.session()
    }

    fn revoke(&self, state: &mut SessionState, identity: Identity) {
        state.token = None;
        state.identity = identity;
        state.remembered = false;
        state.storage_warning = self.credentials.delete().err().map(|_| {
            "无法删除系统 GitHub 凭据；本次会话已停用，下次启动前请从系统凭据管理器删除".into()
        });
    }

    pub async fn configure_token(
        &self,
        candidate: String,
        remember: bool,
    ) -> Result<GitHubSession, GitHubApiError> {
        let candidate = candidate.trim().to_owned();
        if !valid_token(&candidate) {
            return Err(GitHubApiError::new(
                ErrorCode::AuthInvalid,
                "请输入有效的 GitHub Token",
                Action::Reauthenticate,
            ));
        }
        let mut state = self.state.lock().await;
        state
            .gate
            .check(now_ms(), state.token.is_none(), state.rate_limit.clone())?;
        let response = self
            .transport
            .send(Request {
                url: api_url(&["user"], &[])?,
                token: Some(candidate.clone()),
            })
            .await?;
        let now = now_ms();
        // No fallback, cache, or mutation of the active identity on candidate failure.
        let rate = rate_info(&response.headers, now);
        if let Some(mut error) = classify(&response, now) {
            // Candidate /user primary quota belongs to that candidate, not the active
            // anonymous/old-token identity. Secondary and unknown limits are global.
            if error.code != ErrorCode::PrimaryRateLimit {
                state.gate.observe(&mut error, now);
            }
            return Err(error);
        }
        if response.status != StatusCode::OK {
            return Err(GitHubApiError::client(
                "GitHub Token 验证未返回 200，未应用候选凭据",
            ));
        }
        let deleted = self.credentials.delete().is_ok();
        let saved = remember && self.credentials.save(&candidate).is_ok();
        state.token = Some(candidate);
        state.identity = Identity::Authenticated;
        state.remembered = saved;
        state.storage_warning = if remember && !saved {
            Some("系统凭据存储不可用，Token 仅保存在本次会话内存中".into())
        } else if !remember && !deleted {
            Some("无法删除旧系统凭据，新 Token 仅在内存中；请手动清理旧凭据".into())
        } else {
            None
        };
        state.gate.primary_until = None; // Anonymous quota never follows a new identity.
        state.gate.success(rate.as_ref(), now);
        state.rate_limit = rate;
        Ok(state.session())
    }

    /// Current features only read public repositories. Private access must not use this fallback.
    pub async fn public_get(&self, url: Url) -> Result<GitHubResponse, GitHubApiError> {
        self.get(url, false).await
    }

    pub async fn public_zip(&self, url: Url) -> Result<GitHubResponse, GitHubApiError> {
        self.get(url, true).await
    }

    async fn get(&self, url: Url, archive: bool) -> Result<GitHubResponse, GitHubApiError> {
        if !is_api_url(&url) {
            return Err(GitHubApiError::client("拒绝向非 GitHub API 地址发送请求"));
        }
        let mut state = self.state.lock().await;
        state
            .gate
            .check(now_ms(), false, state.rate_limit.clone())?;
        let mut anonymous_fallback_used = false;
        let mut next_url = url;
        let mut codeload = false;
        let mut redirects = 0;
        loop {
            let response = self
                .transport
                .send(Request {
                    url: next_url.clone(),
                    token: if codeload { None } else { state.token.clone() },
                })
                .await?;
            let now = now_ms();
            let rate = rate_info(&response.headers, now);
            if rate.is_some() {
                state.rate_limit = rate.clone();
            }
            if response.status == StatusCode::UNAUTHORIZED && !codeload && state.token.is_some() {
                self.revoke(&mut state, Identity::TokenInvalid);
                state.gate.primary_until = None;
                if !anonymous_fallback_used {
                    anonymous_fallback_used = true;
                    // Do not erase a secondary/unknown cooldown when changing credentials.
                    state.gate.check(now, false, state.rate_limit.clone())?;
                    continue;
                }
            }
            // Archive API redirects are deliberately handled here, never by reqwest.
            if archive && matches!(response.status.as_u16(), 301 | 302 | 303 | 307 | 308) {
                if redirects >= 3 {
                    return Err(GitHubApiError::client("GitHub 归档重定向次数过多"));
                }
                let target = header(&response.headers, "location")
                    .and_then(|s| Url::parse(s).ok())
                    .filter(is_safe_codeload)
                    .ok_or_else(|| GitHubApiError::client("拒绝不安全的 GitHub 归档重定向"))?;
                // A successful zipball API response may have spent the final quota unit.
                if !codeload {
                    state.gate.success(rate.as_ref(), now);
                }
                next_url = target;
                codeload = true;
                redirects += 1;
                continue;
            }
            if let Some(mut error) = classify(&response, now) {
                state.gate.observe(&mut error, now);
                return Err(error);
            }
            if response.status == StatusCode::NOT_MODIFIED {
                // No ETag cache is implemented: do not pretend an empty 304 is a result.
                return Err(GitHubApiError::client(
                    "收到 304，但当前未启用 GitHub 响应缓存",
                ));
            }
            if !codeload {
                state.gate.success(rate.as_ref(), now);
            }
            return Ok(GitHubResponse {
                body: response.body,
                rate_limit: state.rate_limit.clone(),
            });
        }
    }
}

fn valid_token(token: &str) -> bool {
    !token.is_empty()
        && token.bytes().all(|c| c.is_ascii_graphic())
        && reqwest::header::HeaderValue::from_str(token).is_ok()
}

/// Push path segments instead of interpolating user-controlled owners, repos and refs.
pub fn api_url(segments: &[&str], query: &[(&str, &str)]) -> Result<Url, GitHubApiError> {
    let mut url = Url::parse("https://api.github.com/").expect("static GitHub URL");
    {
        let mut path = url
            .path_segments_mut()
            .expect("GitHub URL supports path segments");
        path.clear();
        for segment in segments {
            if segment.is_empty()
                || matches!(*segment, "." | "..")
                || segment.contains(['\\', '\0'])
            {
                return Err(GitHubApiError::client("GitHub 路径参数不合法"));
            }
            path.push(segment);
        }
    }
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query.iter().copied());
    }
    Ok(url)
}
fn clean_https_host(url: &Url, host: &str) -> bool {
    url.scheme() == "https"
        && url.host_str() == Some(host)
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
}
fn is_api_url(url: &Url) -> bool {
    clean_https_host(url, "api.github.com")
}
fn is_safe_codeload(url: &Url) -> bool {
    clean_https_host(url, "codeload.github.com") && url.query().is_none()
}
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}
fn date_ms(text: &str) -> Option<i64> {
    let time = httpdate::parse_http_date(text).ok()?;
    let duration = time.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(duration.as_millis()).ok()
}
fn calibrated(remote_ms: i64, headers: &HeaderMap, now: i64) -> i64 {
    header(headers, "date")
        .and_then(date_ms)
        .map(|server| now.saturating_add(remote_ms.saturating_sub(server)))
        .unwrap_or(remote_ms)
        .max(now)
}
fn rate_info(headers: &HeaderMap, now: i64) -> Option<GitHubRateInfo> {
    let number = |name| header(headers, name).and_then(|s| s.parse::<u64>().ok());
    let rate = GitHubRateInfo {
        limit: number("x-ratelimit-limit"),
        remaining: number("x-ratelimit-remaining"),
        reset_at: number("x-ratelimit-reset")
            .and_then(|seconds| i64::try_from(seconds).ok()?.checked_mul(1000))
            .map(|ms| calibrated(ms, headers, now)),
        resource: header(headers, "x-ratelimit-resource").map(str::to_owned),
    };
    if rate == GitHubRateInfo::default() {
        None
    } else {
        Some(rate)
    }
}
fn retry_after(headers: &HeaderMap, now: i64) -> Option<i64> {
    let value = header(headers, "retry-after")?;
    if let Ok(seconds) = value.parse::<u64>() {
        return i64::try_from(seconds)
            .ok()?
            .checked_mul(1000)?
            .checked_add(now);
    }
    Some(calibrated(date_ms(value)?, headers, now))
}

/// Ordering is intentional: even misleading rate headers must not turn a 200/304 or 401
/// into a rate failure. Only 403/429 can consume primary/secondary cooldown decisions.
fn classify(response: &Response, now: i64) -> Option<GitHubApiError> {
    let status = response.status;
    if status.is_success() || status == StatusCode::NOT_MODIFIED {
        return None;
    }
    let rate = rate_info(&response.headers, now);
    let retry = retry_after(&response.headers, now);
    let (code, message, action, retry_at) = if status == StatusCode::UNAUTHORIZED {
        (
            ErrorCode::AuthInvalid,
            "GitHub Token 无效或已撤销，请重新认证",
            Action::Reauthenticate,
            None,
        )
    } else if matches!(status.as_u16(), 403 | 429)
        && rate.as_ref().is_some_and(|r| r.remaining == Some(0))
    {
        (
            ErrorCode::PrimaryRateLimit,
            "GitHub 主限流额度已用尽",
            Action::Wait,
            Some(
                rate.as_ref()
                    .and_then(|r| r.reset_at)
                    .unwrap_or(now + SECONDARY_MIN_MS)
                    .max(retry.unwrap_or(now)),
            ),
        )
    } else {
        let text: serde_json::Value = serde_json::from_slice(&response.body).unwrap_or_default();
        let message = text
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let docs = text
            .get("documentation_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(status.as_u16(), 403 | 429)
            && (message.contains("secondary rate limit")
                || message.contains("abuse detection")
                || docs.contains("secondary-rate-limits"))
        {
            (
                ErrorCode::SecondaryRateLimit,
                "GitHub 触发二级限流，请等待后重试",
                Action::Wait,
                retry,
            )
        } else if status == StatusCode::TOO_MANY_REQUESTS
            || (status == StatusCode::FORBIDDEN
                && header(&response.headers, "retry-after").is_some())
        {
            (
                ErrorCode::RateLimitUnknown,
                "GitHub 请求受限，限制类型未知",
                Action::Wait,
                retry,
            )
        } else if status == StatusCode::NOT_FOUND {
            (
                ErrorCode::NotFoundOrPrivate,
                "仓库或文件不存在，或为当前身份不可访问的私有资源",
                Action::None,
                None,
            )
        } else if status == StatusCode::FORBIDDEN {
            (
                ErrorCode::Forbidden,
                "GitHub 拒绝访问，请检查权限",
                Action::None,
                None,
            )
        } else if status.is_server_error() {
            (
                ErrorCode::ServerError,
                "GitHub 服务暂时不可用，请稍后重试",
                Action::None,
                None,
            )
        } else {
            (
                ErrorCode::ClientError,
                "GitHub 请求失败或返回了不支持的重定向",
                Action::None,
                None,
            )
        }
    };
    Some(GitHubApiError {
        schema_version: SCHEMA_VERSION,
        code,
        message: message.into(),
        retry_at,
        action,
        rate_limit: rate.map(Box::new),
    })
}

#[cfg(test)]
mod tests;
