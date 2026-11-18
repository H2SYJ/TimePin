//! Shared, backend-only GitHub session. Never derive Debug for a type holding credentials.
//! One mutex covers admission, HTTP and state updates: expired limits get a single probe,
//! not a burst of independently admitted requests. There are no automatic rate-limit retries.

use reqwest::{header::HeaderMap, StatusCode, Url};
use serde::Serialize;
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::sync::Mutex;

const SCHEMA_VERSION: &str = "1.0.0";
const USER_AGENT: &str = concat!("TimePin/", env!("CARGO_PKG_VERSION"));
const SECONDARY_MIN_MS: i64 = 60_000;
const SECONDARY_MAX_MS: i64 = 15 * 60_000;
// Bound both JSON and archive responses, including chunked bodies without Content-Length.
const MAX_RESPONSE_BYTES: usize = 128 * 1024 * 1024;
const ACCEPT: &str = "application/vnd.github+json";
const API_VERSION: &str = "2022-11-28";

/// GitHub's `x-ratelimit-resource` buckets we gate on independently. Requests whose
/// URL does not map to a known bucket fall back to CORE; codeload archive bodies are
/// never bucketed here because they carry no rate headers worth recording.
mod resource {
    pub const CORE: &str = "core";
    pub const SEARCH: &str = "search";
    pub const GRAPHQL: &str = "graphql";
}

/// Map a request URL to its rate-limit resource bucket. Called *before* the request
/// is sent so admission and local pacing do not wait for `x-ratelimit-resource`.
fn infer_resource(url: &Url) -> &'static str {
    let path = url.path();
    if path.starts_with("/search/") || path == "/search" {
        resource::SEARCH
    } else if path == "/graphql" {
        resource::GRAPHQL
    } else {
        resource::CORE
    }
}

/// In-memory only. Bodies larger than the total byte budget are never cached.
#[derive(Clone, Copy)]
pub struct CacheConfig {
    pub max_entries: usize,
    pub ttl: Duration,
    pub max_bytes: usize,
}
impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_entries: 100,
            ttl: Duration::from_secs(10 * 60),
            max_bytes: 32 * 1024 * 1024,
        }
    }
}

trait Clock: Send + Sync {
    fn now_ms(&self) -> i64;
}
struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        now_ms()
    }
}

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
    /// Last observed response, retained for IPC v1 compatibility.
    pub rate_limit: Option<GitHubRateInfo>,
    pub rate_limits: Vec<GitHubRateInfo>,
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
#[derive(Clone, Default)]
struct Validators {
    etag: Option<String>,
    last_modified: Option<String>,
}
impl Validators {
    fn from_headers(headers: &HeaderMap) -> Self {
        Self {
            etag: header(headers, "etag").map(str::to_owned),
            last_modified: header(headers, "last-modified").map(str::to_owned),
        }
    }
    fn is_empty(&self) -> bool {
        self.etag.is_none() && self.last_modified.is_none()
    }
}

struct Request {
    url: Url,
    token: Option<String>,
    accept: &'static str,
    api_version: &'static str,
    validators: Validators,
}
impl Request {
    fn new(url: Url, token: Option<String>) -> Self {
        Self {
            url,
            token,
            accept: ACCEPT,
            api_version: API_VERSION,
            validators: Validators::default(),
        }
    }
    fn cache_key(&self, epoch: u64) -> CacheKey {
        CacheKey {
            url: self.url.as_str().to_owned(),
            accept: self.accept.to_owned(),
            api_version: self.api_version.to_owned(),
            epoch,
        }
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CacheKey {
    url: String,
    accept: String,
    api_version: String,
    epoch: u64,
}
struct CacheEntry {
    body: Vec<u8>,
    validators: Validators,
    expires_at: i64,
    last_used: u64,
    bytes: usize,
}
struct ResponseCache {
    config: CacheConfig,
    entries: BTreeMap<CacheKey, CacheEntry>,
    bytes: usize,
    sequence: u64,
}
impl ResponseCache {
    fn new(config: CacheConfig) -> Self {
        Self {
            config,
            entries: BTreeMap::new(),
            bytes: 0,
            sequence: 0,
        }
    }
    fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
    fn remove(&mut self, key: &CacheKey) {
        if let Some(entry) = self.entries.remove(key) {
            self.bytes -= entry.bytes;
        }
    }
    fn purge(&mut self, now: i64) {
        self.entries.retain(|_, entry| entry.expires_at > now);
        self.bytes = self.entries.values().map(|entry| entry.bytes).sum();
    }
    fn live(&mut self, key: &CacheKey, now: i64) -> Option<&CacheEntry> {
        self.purge(now);
        self.sequence = self.sequence.wrapping_add(1);
        let entry = self.entries.get_mut(key)?;
        entry.last_used = self.sequence;
        Some(entry)
    }
    fn expires_at(&self, now: i64) -> i64 {
        now.saturating_add(i64::try_from(self.config.ttl.as_millis()).unwrap_or(i64::MAX))
    }
    fn store(&mut self, key: CacheKey, headers: &HeaderMap, body: &[u8], now: i64) {
        self.purge(now);
        self.remove(&key);
        let validators = Validators::from_headers(headers);
        // Account for keys and validators too, not just bodies. The fixed overhead bounds
        // small entries while max_entries independently bounds map allocations.
        let bytes = body
            .len()
            .saturating_add(key.url.len())
            .saturating_add(key.accept.len())
            .saturating_add(key.api_version.len())
            .saturating_add(validators.etag.as_ref().map_or(0, String::len))
            .saturating_add(validators.last_modified.as_ref().map_or(0, String::len))
            .saturating_add(256);
        if validators.is_empty()
            || self.config.max_entries == 0
            || self.config.ttl.is_zero()
            || bytes > self.config.max_bytes
        {
            return;
        }
        while self.entries.len() >= self.config.max_entries
            || self.bytes > self.config.max_bytes.saturating_sub(bytes)
        {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                self.remove(&oldest);
            } else {
                return;
            }
        }
        self.sequence = self.sequence.wrapping_add(1);
        self.entries.insert(
            key,
            CacheEntry {
                body: body.to_vec(),
                validators,
                expires_at: self.expires_at(now),
                last_used: self.sequence,
                bytes,
            },
        );
        self.bytes += bytes;
    }
    fn revalidate(&mut self, key: &CacheKey, headers: &HeaderMap, now: i64) -> Option<Vec<u8>> {
        let entry = self.live(key, now)?;
        let body = entry.body.clone();
        // 304 may update validators; retain those omitted by the server.
        let mut updated = headers.clone();
        for (name, value) in [
            ("etag", &entry.validators.etag),
            ("last-modified", &entry.validators.last_modified),
        ] {
            if !updated.contains_key(name) {
                if let Some(value) = value {
                    updated.insert(
                        reqwest::header::HeaderName::from_static(name),
                        value.parse().ok()?,
                    );
                }
            }
        }
        self.store(key.clone(), &updated, &body, now);
        Some(body)
    }
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
        if (!request.validators.is_empty() && request.token.is_none())
            || ((!request.validators.is_empty() || request.token.is_some())
                && !is_api_url(&request.url))
        {
            return Err(GitHubApiError::client(
                "拒绝向非认证 GitHub API 请求发送凭据或缓存校验头",
            ));
        }
        let mut builder = self
            .0
            .get(request.url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::ACCEPT, request.accept)
            .header("X-GitHub-Api-Version", request.api_version);
        if let Some(etag) = request.validators.etag {
            builder = builder.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        if let Some(modified) = request.validators.last_modified {
            builder = builder.header(reqwest::header::IF_MODIFIED_SINCE, modified);
        }
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
    primary_until: BTreeMap<String, i64>,
    secondary_until: Option<i64>,
    unknown_until: Option<i64>,
    secondary_failures: u32,
    stopped: bool,
}
impl Gate {
    fn check(
        &self,
        now: i64,
        resource: &str,
        candidate_over_anonymous_primary: bool,
        rate_limit: Option<GitHubRateInfo>,
    ) -> Result<(), GitHubApiError> {
        // Global safety takes precedence and deliberately carries no resource snapshot:
        // consumers must not mistake this RATE_WAIT for a resource-local primary gate.
        let global_until = [self.secondary_until, self.unknown_until]
            .into_iter()
            .flatten()
            .filter(|until| *until > now)
            .max();
        let primary_until = if candidate_over_anonymous_primary {
            None
        } else {
            self.primary_until
                .get(resource)
                .copied()
                .filter(|until| *until > now)
        };
        if self.stopped || global_until.is_some() || primary_until.is_some() {
            let global = self.stopped || global_until.is_some();
            return Err(GitHubApiError {
                schema_version: SCHEMA_VERSION,
                code: ErrorCode::RateWait,
                message: if self.stopped {
                    "GitHub 连续三次触发二级限流，本次会话已停止请求，请稍后重启应用再试".into()
                } else {
                    "GitHub 请求处于冷却期，请等待后重试".into()
                },
                retry_at: if self.stopped {
                    None
                } else {
                    global_until.or(primary_until)
                },
                action: if self.stopped {
                    Action::None
                } else {
                    Action::Wait
                },
                rate_limit: if global {
                    None
                } else {
                    Some(Box::new(rate_limit.unwrap_or_else(|| GitHubRateInfo {
                        resource: Some(resource.to_owned()),
                        ..GitHubRateInfo::default()
                    })))
                },
            });
        }
        Ok(())
    }

    /// Observe a classified error. `inferred` fills the resource when the response
    /// omits `x-ratelimit-resource` so primary gates stay bucketed per resource.
    fn observe(&mut self, error: &mut GitHubApiError, now: i64, inferred: &str) {
        match error.code {
            ErrorCode::PrimaryRateLimit => {
                let rate = error
                    .rate_limit
                    .get_or_insert_with(|| Box::new(GitHubRateInfo::default()));
                let resource = rate
                    .resource
                    .get_or_insert_with(|| inferred.to_owned())
                    .clone();
                let until = error.retry_at.unwrap_or(now + SECONDARY_MIN_MS);
                self.primary_until.insert(resource, until);
                error.retry_at = Some(until);
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

    /// Register an observed primary exhaustion from a success response's own headers
    /// (e.g. remaining=0 on a 200), keyed by that response's resource bucket.
    fn observe_exhausted(&mut self, rate: &GitHubRateInfo, now: i64) {
        if rate.remaining != Some(0) {
            return;
        }
        let Some(resource) = &rate.resource else {
            return;
        };
        let until = rate.reset_at.unwrap_or(now + SECONDARY_MIN_MS).max(now);
        let current = self.primary_until.entry(resource.clone()).or_insert(now);
        *current = (*current).max(until);
    }

    fn success(&mut self, rate: Option<&GitHubRateInfo>, now: i64) {
        // A success from another resource must not reopen an unexpired primary gate.
        self.primary_until.retain(|_, until| *until > now);
        if let Some(rate) = rate {
            self.observe_exhausted(rate, now);
        }
        // Requests are admitted only after global cooldowns expire.
        self.secondary_until = None;
        self.unknown_until = None;
        self.secondary_failures = 0;
    }
}

/// Capacity-one token bucket: no accumulated idle credits and thus no search bursts.
#[derive(Default)]
struct SearchBudget {
    next_at: Option<i64>,
}
impl SearchBudget {
    fn take(&mut self, now: i64, authenticated: bool) -> Result<(), GitHubApiError> {
        if let Some(until) = self.next_at.filter(|until| *until > now) {
            return Err(GitHubApiError {
                schema_version: SCHEMA_VERSION,
                code: ErrorCode::RateWait,
                message: "GitHub 搜索请求过于频繁，请等待后重试".into(),
                retry_at: Some(until),
                action: Action::Wait,
                // This is a local pacing signal, not a fabricated server quota snapshot.
                rate_limit: Some(Box::new(GitHubRateInfo {
                    resource: Some("search".into()),
                    ..GitHubRateInfo::default()
                })),
            });
        }
        self.next_at = Some(now.saturating_add(if authenticated { 2_000 } else { 6_000 }));
        Ok(())
    }
}

struct SessionState {
    token: Option<String>,
    identity: Identity,
    remembered: bool,
    storage_warning: Option<String>,
    rate_limit: Option<GitHubRateInfo>,
    rate_limits: BTreeMap<String, GitHubRateInfo>,
    gate: Gate,
    search: SearchBudget,
    identity_epoch: u64,
    cache: ResponseCache,
}
impl SessionState {
    fn session(&self) -> GitHubSession {
        GitHubSession {
            schema_version: SCHEMA_VERSION,
            identity: self.identity,
            remembered: self.remembered,
            storage_warning: self.storage_warning.clone(),
            rate_limit: self.rate_limit.clone(),
            rate_limits: self.rate_limits.values().cloned().collect(),
        }
    }
    fn change_identity(&mut self) {
        self.identity_epoch = self.identity_epoch.wrapping_add(1);
        self.cache.clear();
        self.gate.primary_until.clear();
        self.search = SearchBudget::default();
        self.rate_limit = None;
        self.rate_limits.clear();
        // secondary/unknown/stopped remain global, including across 401 fallback.
    }
    fn admit(&mut self, resource: &str, now: i64) -> Result<(), GitHubApiError> {
        self.gate.check(
            now,
            resource,
            false,
            self.rate_limits.get(resource).cloned(),
        )?;
        if resource == "search" {
            self.search.take(now, self.token.is_some())?;
        }
        Ok(())
    }
    fn admit_probe(
        &mut self,
        resource: &str,
        now: i64,
        anonymous_primary_ok: bool,
    ) -> Result<(), GitHubApiError> {
        self.gate.check(
            now,
            resource,
            anonymous_primary_ok,
            self.rate_limits.get(resource).cloned(),
        )?;
        if resource == "search" {
            self.search.take(now, self.token.is_some())?;
        }
        Ok(())
    }
    /// Merge a newly observed rate snapshot into the per-resource ledger and return
    /// the stored form. `inferred` is the URL-derived bucket used when GitHub omits
    /// `x-ratelimit-resource`; it never overwrites a header-declared resource.
    fn observe_rate(&mut self, mut rate: GitHubRateInfo, inferred: &str) -> GitHubRateInfo {
        let resource = rate
            .resource
            .get_or_insert_with(|| inferred.to_owned())
            .clone();
        if let Some(previous) = self.rate_limits.get(&resource) {
            rate.limit = rate.limit.or(previous.limit);
            rate.remaining = rate.remaining.or(previous.remaining);
            rate.reset_at = rate.reset_at.or(previous.reset_at);
        }
        self.rate_limits.insert(resource, rate.clone());
        self.rate_limit = Some(rate.clone());
        rate
    }
}

pub struct GitHubClient {
    transport: Arc<dyn Transport>,
    credentials: Arc<dyn CredentialStore>,
    clock: Arc<dyn Clock>,
    state: Mutex<SessionState>,
}

/// Response carries its own resource snapshot, never an unrelated last response's quota.
#[derive(Debug)]
pub struct GitHubResponse {
    pub body: Vec<u8>,
    /// This response's own bucket, or the ledger entry for the request's inferred
    /// bucket when the response carried no rate headers (e.g. a 304 hit).
    pub rate_limit: Option<GitHubRateInfo>,
    /// Every per-resource snapshot the session has observed so far. Callers that
    /// need a headline bucket (e.g. search for discovery) pick it from here;
    /// callers that only need one value keep using `rate_limit`.
    pub rate_limits: Vec<GitHubRateInfo>,
}

impl GitHubClient {
    pub fn new() -> Result<Self, reqwest::Error> {
        Self::with_cache_config(CacheConfig::default())
    }

    pub fn with_cache_config(config: CacheConfig) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self::with_runtime(
            Arc::new(HttpTransport(http)),
            Arc::new(SystemCredentials),
            Arc::new(SystemClock),
            config,
        ))
    }

    #[cfg(test)]
    fn with_parts(transport: Arc<dyn Transport>, credentials: Arc<dyn CredentialStore>) -> Self {
        Self::with_runtime(
            transport,
            credentials,
            Arc::new(SystemClock),
            CacheConfig::default(),
        )
    }

    #[cfg(test)]
    fn with_test_parts(
        transport: Arc<dyn Transport>,
        credentials: Arc<dyn CredentialStore>,
        clock: Arc<dyn Clock>,
        config: CacheConfig,
    ) -> Self {
        Self::with_runtime(transport, credentials, clock, config)
    }

    fn with_runtime(
        transport: Arc<dyn Transport>,
        credentials: Arc<dyn CredentialStore>,
        clock: Arc<dyn Clock>,
        config: CacheConfig,
    ) -> Self {
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
            clock,
            state: Mutex::new(SessionState {
                token,
                identity,
                remembered,
                storage_warning,
                rate_limit: None,
                rate_limits: BTreeMap::new(),
                gate: Gate::default(),
                search: SearchBudget::default(),
                identity_epoch: 0,
                cache: ResponseCache::new(config),
            }),
        }
    }

    pub async fn session(&self) -> GitHubSession {
        self.state.lock().await.session()
    }

    pub async fn clear_token(&self) -> GitHubSession {
        let mut state = self.state.lock().await;
        self.revoke(&mut state, Identity::Anonymous);
        state.session()
    }

    fn revoke(&self, state: &mut SessionState, identity: Identity) {
        if state.token.is_some() {
            state.change_identity();
        }
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
        let now = self.clock.now_ms();
        let anonymous = state.token.is_none();
        state.admit_probe(resource::CORE, now, anonymous)?;
        let response = self
            .transport
            .send(Request::new(
                api_url(&["user"], &[])?,
                Some(candidate.clone()),
            ))
            .await?;
        let now = self.clock.now_ms();
        // No fallback, cache, or mutation of the active identity on candidate failure.
        let rate = rate_info(&response.headers, now);
        if let Some(mut error) = classify(&response, now) {
            // Candidate /user primary quota belongs to that candidate, not the active
            // anonymous/old-token identity. Secondary and unknown limits are global.
            if error.code != ErrorCode::PrimaryRateLimit {
                state.gate.observe(&mut error, now, resource::CORE);
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
        let identity_changed = state.token.as_deref() != Some(candidate.as_str());
        if identity_changed {
            // Re-submitting the same credential is not a new quota/cache identity.
            state.change_identity();
        }
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
        // A validated identity may replace its own primary/search budgets, but never
        // resets secondary/unknown/stopped safety, even when credentials change.
        state.gate.success(rate.as_ref(), now);
        if let Some(rate) = rate {
            let rate = state.observe_rate(rate, resource::CORE);
            state.gate.observe_exhausted(&rate, now);
        }
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
        // Resource is inferred from the request URL BEFORE the request is sent, so
        // admission, local search pacing and any resource-less error bucket all agree.
        let inferred = infer_resource(&url);
        let now = self.clock.now_ms();
        state.admit(inferred, now)?;
        let mut anonymous_fallback_used = false;
        let mut next_url = url;
        let mut codeload = false;
        let mut redirects = 0;
        loop {
            let authenticated = !codeload && state.token.is_some();
            let mut request = Request::new(
                next_url.clone(),
                if codeload { None } else { state.token.clone() },
            );
            let cache_key = authenticated.then(|| request.cache_key(state.identity_epoch));
            if let Some(key) = cache_key.as_ref() {
                // Live entries supply validators for the conditional revalidation.
                if let Some(validators) = state
                    .cache
                    .live(key, now)
                    .map(|entry| entry.validators.clone())
                {
                    request.validators = validators;
                }
            }
            let response = self.transport.send(request).await?;
            let now = self.clock.now_ms();
            // The response's resource comes from x-ratelimit-resource; a missing header
            // falls back to the request's inferred bucket. Core and search never mix.
            let rate = (!codeload)
                .then(|| rate_info(&response.headers, now))
                .flatten()
                .map(|rate| state.observe_rate(rate, inferred));
            if response.status == StatusCode::UNAUTHORIZED && !codeload && state.token.is_some() {
                self.revoke(&mut state, Identity::TokenInvalid);
                if !anonymous_fallback_used {
                    anonymous_fallback_used = true;
                    // Do not erase a secondary/unknown cooldown when changing credentials.
                    state.admit(inferred, now)?;
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
                    if let Some(rate) = rate {
                        state.gate.observe_exhausted(&rate, now);
                    }
                }
                next_url = target;
                codeload = true;
                redirects += 1;
                continue;
            }
            if let Some(mut error) = classify(&response, now) {
                // Error consumers (including manifest 404 handling) receive the same
                // resource-inferred snapshot as the session, never a resource-less copy.
                if let Some(rate) = rate {
                    error.rate_limit = Some(Box::new(rate));
                }
                state.gate.observe(&mut error, now, inferred);
                return Err(error);
            }
            if response.status == StatusCode::NOT_MODIFIED {
                let Some(key) = cache_key else {
                    // Anonymous callers and codeload never send validators, so a 304
                    // here means GitHub answered an unconditional request, which is an
                    // error rather than cacheable content.
                    return Err(GitHubApiError::client(
                        "收到 304，但当前请求未携带缓存校验信息",
                    ));
                };
                let Some(body) = state.cache.revalidate(&key, &response.headers, now) else {
                    return Err(GitHubApiError::client(
                        "收到 304，但本地缓存中没有对应的可用条目",
                    ));
                };
                state.gate.success(rate.as_ref(), now);
                if let Some(rate) = &rate {
                    state.gate.observe_exhausted(rate, now);
                }
                return Ok(GitHubResponse {
                    body,
                    rate_limit: rate.or_else(|| state.rate_limits.get(inferred).cloned()),
                    rate_limits: state.rate_limits.values().cloned().collect(),
                });
            }
            if !codeload {
                state.gate.success(rate.as_ref(), now);
                if let Some(rate) = &rate {
                    state.gate.observe_exhausted(rate, now);
                }
                if authenticated {
                    if let Some(key) = cache_key {
                        state
                            .cache
                            .store(key, &response.headers, &response.body, now);
                    }
                }
            }
            return Ok(GitHubResponse {
                body: response.body,
                rate_limit: rate.or_else(|| state.rate_limits.get(inferred).cloned()),
                rate_limits: state.rate_limits.values().cloned().collect(),
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
