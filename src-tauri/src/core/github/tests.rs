use super::*;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex as StdMutex,
    },
    time::Duration,
};

#[derive(Default)]
struct MemoryCredentials {
    value: StdMutex<Option<String>>,
    unavailable: AtomicBool,
}
impl CredentialStore for MemoryCredentials {
    fn load(&self) -> Result<Option<String>, ()> {
        if self.unavailable.load(Ordering::SeqCst) {
            Err(())
        } else {
            Ok(self.value.lock().unwrap().clone())
        }
    }
    fn save(&self, token: &str) -> Result<(), ()> {
        if self.unavailable.load(Ordering::SeqCst) {
            return Err(());
        }
        *self.value.lock().unwrap() = Some(token.to_owned());
        Ok(())
    }
    fn delete(&self) -> Result<(), ()> {
        if self.unavailable.load(Ordering::SeqCst) {
            return Err(());
        }
        *self.value.lock().unwrap() = None;
        Ok(())
    }
}
struct MockTransport {
    responses: StdMutex<VecDeque<Response>>,
    requests: StdMutex<Vec<Request>>,
    active: AtomicUsize,
    max_active: AtomicUsize,
}
impl Transport for MockTransport {
    fn send(&self, request: Request) -> SendFuture<'_> {
        Box::pin(async move {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            tokio::task::yield_now().await;
            self.requests.lock().unwrap().push(request);
            let response = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected HTTP request");
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(response)
        })
    }
}
fn response(status: u16, headers: &[(&str, &str)], body: &str) -> Response {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.insert(
            reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            value.parse().unwrap(),
        );
    }
    Response {
        status: StatusCode::from_u16(status).unwrap(),
        headers: map,
        body: body.as_bytes().to_vec(),
    }
}
struct ManualClock {
    now: AtomicUsize,
}
impl ManualClock {
    fn new(start: i64) -> Self {
        Self {
            now: AtomicUsize::new(start as usize),
        }
    }
    fn advance(&self, ms: i64) {
        self.now.fetch_add(ms as usize, Ordering::SeqCst);
    }
}
impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.now.load(Ordering::SeqCst) as i64
    }
}
fn fixture(
    responses: Vec<Response>,
    token: Option<&str>,
) -> (GitHubClient, Arc<MockTransport>, Arc<MemoryCredentials>) {
    fixture_with(responses, token, None, None).0
}
#[allow(clippy::type_complexity)]
fn fixture_with(
    responses: Vec<Response>,
    token: Option<&str>,
    clock: Option<Arc<ManualClock>>,
    cache: Option<CacheConfig>,
) -> (
    (GitHubClient, Arc<MockTransport>, Arc<MemoryCredentials>),
    Arc<ManualClock>,
) {
    let transport = Arc::new(MockTransport {
        responses: StdMutex::new(responses.into()),
        requests: StdMutex::new(Vec::new()),
        active: AtomicUsize::new(0),
        max_active: AtomicUsize::new(0),
    });
    let store = Arc::new(MemoryCredentials::default());
    *store.value.lock().unwrap() = token.map(str::to_owned);
    let clock = clock.unwrap_or_else(|| Arc::new(ManualClock::new(now_ms())));
    let cache = cache.unwrap_or_default();
    (
        (
            GitHubClient::with_test_parts(transport.clone(), store.clone(), clock.clone(), cache),
            transport,
            store,
        ),
        clock,
    )
}
fn endpoint() -> Url {
    api_url(
        &["search", "repositories"],
        &[("q", "topic:time-pin-plugins")],
    )
    .unwrap()
}

#[test]
fn bounded_response_chunks_reject_overflow_without_appending() {
    let mut body = Vec::new();
    append_response_chunk(&mut body, b"abc", 5).unwrap();
    append_response_chunk(&mut body, b"de", 5).unwrap();
    assert!(append_response_chunk(&mut body, b"f", 5).is_err());
    assert_eq!(body, b"abcde");
    append_response_chunk(&mut body, b"", 5).unwrap();
    assert!(append_response_chunk(&mut body, b"", 4).is_err());
}

#[tokio::test]
async fn malformed_persisted_credential_is_deleted_once_without_echoing_secret() {
    let (client, transport, store) = fixture(Vec::new(), Some("bad\nsecret"));
    let session = client.session().await;
    assert_eq!(session.identity, Identity::Anonymous);
    assert!(!session.remembered);
    assert!(session.storage_warning.is_some());
    assert!(!serde_json::to_string(&session).unwrap().contains("secret"));
    assert!(store.value.lock().unwrap().is_none());
    let restarted = GitHubClient::with_parts(transport, store);
    assert!(restarted.session().await.storage_warning.is_none());
}

#[test]
fn mandatory_user_agent_and_sensitive_authorization_are_applied_at_transport_boundary() {
    let transport = HttpTransport(reqwest::Client::new());
    let authenticated = transport
        .prepare(Request::new(endpoint(), Some("unit-test-secret".into())))
        .unwrap();
    assert_eq!(
        authenticated.headers()[reqwest::header::USER_AGENT],
        USER_AGENT
    );
    assert!(authenticated.headers()[reqwest::header::AUTHORIZATION].is_sensitive());
    assert!(!format!("{authenticated:?}").contains("unit-test-secret"));
    let anonymous = transport.prepare(Request::new(endpoint(), None)).unwrap();
    assert!(anonymous
        .headers()
        .get(reqwest::header::AUTHORIZATION)
        .is_none());
    assert_eq!(anonymous.headers()[reqwest::header::USER_AGENT], USER_AGENT);
    assert!(transport
        .prepare(Request::new(
            Url::parse("https://codeload.github.com/zip").unwrap(),
            Some("unit-test-secret".into())
        ))
        .is_err());
}

#[test]
fn classification_priority_is_status_then_primary_then_secondary_then_unknown() {
    let rate_headers = [("x-ratelimit-remaining", "0"), ("retry-after", "10")];
    let secondary = r#"{"message":"You have exceeded a secondary rate limit"}"#;
    for status in [200, 201, 204, 304] {
        assert!(classify(&response(status, &rate_headers, secondary), 1_000).is_none());
    }
    let cases = [
        (401, &rate_headers[..], secondary, ErrorCode::AuthInvalid),
        (
            403,
            &rate_headers[..],
            secondary,
            ErrorCode::PrimaryRateLimit,
        ),
        (
            429,
            &rate_headers[..],
            secondary,
            ErrorCode::PrimaryRateLimit,
        ),
        (403, &[][..], secondary, ErrorCode::SecondaryRateLimit),
        (429, &[][..], secondary, ErrorCode::SecondaryRateLimit),
        (
            403,
            &[("retry-after", "10")][..],
            "{}",
            ErrorCode::RateLimitUnknown,
        ),
        (429, &[][..], "{}", ErrorCode::RateLimitUnknown),
        (
            404,
            &rate_headers[..],
            secondary,
            ErrorCode::NotFoundOrPrivate,
        ),
        (403, &[][..], "{}", ErrorCode::Forbidden),
        (422, &[][..], "{}", ErrorCode::ClientError),
        (503, &rate_headers[..], secondary, ErrorCode::ServerError),
    ];
    for (status, headers, body, expected) in cases {
        assert_eq!(
            classify(&response(status, headers, body), 1_000)
                .unwrap()
                .code,
            expected
        );
    }
}

#[test]
fn headers_use_local_calibrated_milliseconds_not_server_wall_clock() {
    let server = "Wed, 21 Oct 2015 07:28:00 GMT";
    let reset = (date_ms(server).unwrap() / 1000 + 120).to_string();
    let response = response(
        403,
        &[
            ("date", server),
            ("x-ratelimit-limit", "60"),
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", &reset),
            ("x-ratelimit-resource", "search"),
            ("retry-after", "Wed, 21 Oct 2015 07:29:00 GMT"),
        ],
        "{}",
    );
    let local_now = 2_000_000_000_123;
    let rate = rate_info(&response.headers, local_now).unwrap();
    assert_eq!(rate.reset_at, Some(local_now + 120_000));
    assert_eq!(rate.limit, Some(60));
    assert_eq!(rate.resource.as_deref(), Some("search"));
    assert_eq!(
        retry_after(&response.headers, local_now),
        Some(local_now + 60_000)
    );
    assert_eq!(
        classify(&response, local_now).unwrap().retry_at,
        Some(local_now + 120_000)
    );
    let seconds = response_with_retry("45");
    assert_eq!(
        retry_after(&seconds.headers, local_now),
        Some(local_now + 45_000)
    );
    for value in ["garbage", "-1", "18446744073709551615"] {
        assert_eq!(
            retry_after(&response_with_retry(value).headers, local_now),
            None
        );
    }
    let invalid = response_with_retry("Thu, 01 Jan 1970 00:00:00 GMT");
    assert_eq!(retry_after(&invalid.headers, local_now), Some(local_now));
}
fn response_with_retry(value: &str) -> Response {
    response(429, &[("retry-after", value)], "{}")
}

#[test]
fn malformed_headers_are_nullable_and_cannot_overflow() {
    let res = response(
        200,
        &[
            ("x-ratelimit-limit", "garbage"),
            ("x-ratelimit-remaining", "-1"),
            ("x-ratelimit-reset", "18446744073709551615"),
            ("date", "bad"),
        ],
        "{}",
    );
    assert_eq!(rate_info(&res.headers, 0), None);
    let res = response(200, &[("x-ratelimit-reset", "10")], "{}");
    assert_eq!(rate_info(&res.headers, 100).unwrap().reset_at, Some(10_000));
}

#[test]
fn secondary_backoff_has_minimum_exponential_growth_and_three_strike_stop() {
    let mut gate = Gate::default();
    let mut now = 1_000;
    for (strike, delay) in [(1, 60_000), (2, 120_000), (3, 240_000)] {
        let mut error = classify(
            &response(
                403,
                &[("retry-after", "1")],
                r#"{"message":"secondary rate limit"}"#,
            ),
            now,
        )
        .unwrap();
        gate.observe(&mut error, now, resource::CORE);
        assert_eq!(gate.secondary_until, Some(now + delay));
        assert!(gate.check(now, resource::CORE, true, None).is_err());
        if strike < 3 {
            assert!(gate.check(now + delay, resource::CORE, false, None).is_ok());
            assert_eq!(error.retry_at, Some(now + delay));
        } else {
            assert!(gate
                .check(now + SECONDARY_MAX_MS * 100, resource::CORE, true, None)
                .is_err());
            assert_eq!(error.action, Action::None);
            assert_eq!(error.retry_at, None);
        }
        now += delay;
    }
    let mut high = Gate {
        secondary_failures: 20,
        ..Gate::default()
    };
    let mut err = GitHubApiError::new(ErrorCode::SecondaryRateLimit, "limit", Action::Wait);
    high.observe(&mut err, 0, resource::CORE);
    assert_eq!(high.secondary_until, Some(SECONDARY_MAX_MS));
}

#[tokio::test]
async fn anonymous_default_and_ipc_never_contains_token() {
    let (client, _, _) = fixture(vec![], None);
    let session = client.session().await;
    assert_eq!(session.identity, Identity::Anonymous);
    let value = serde_json::to_value(session).unwrap();
    assert_eq!(value["schemaVersion"], "1.0.0");
    assert_eq!(value["identity"], "ANONYMOUS");
    assert_eq!(value["remembered"], false);
    assert!(value.get("token").is_none());
    let error = GitHubApiError::new(ErrorCode::AuthInvalid, "invalid", Action::Reauthenticate);
    let value = serde_json::to_value(error).unwrap();
    assert!(value.is_object());
    assert_eq!(value["code"], "AUTH_INVALID");
    assert_eq!(value["action"], "REAUTHENTICATE");
    assert!(value.get("retryAt").is_some());
}

#[tokio::test]
async fn candidate_only_applies_on_200_and_never_falls_back_or_deletes_active_token() {
    let (client, transport, store) = fixture(
        vec![
            response(401, &[], "{}"),
            response(201, &[], "{}"),
            response(200, &[], "{}"),
        ],
        Some("old-secret"),
    );
    assert_eq!(
        client
            .configure_token("bad-secret".into(), true)
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthInvalid
    );
    assert_eq!(client.session().await.identity, Identity::Authenticated);
    assert_eq!(store.value.lock().unwrap().as_deref(), Some("old-secret"));
    assert!(client
        .configure_token("candidate".into(), true)
        .await
        .is_err());
    assert_eq!(store.value.lock().unwrap().as_deref(), Some("old-secret"));
    let session = client
        .configure_token("new-secret".into(), false)
        .await
        .unwrap();
    assert_eq!(session.identity, Identity::Authenticated);
    assert!(!session.remembered);
    assert!(store.value.lock().unwrap().is_none());
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].url.as_str(), "https://api.github.com/user");
    assert!(requests.iter().all(|request| request.token.is_some()));
    assert!(!serde_json::to_string(&session)
        .unwrap()
        .contains("new-secret"));
}

#[tokio::test]
async fn rate_limited_candidate_preserves_existing_identity_and_credentials() {
    for status in [403, 429] {
        let (client, _, store) = fixture(
            vec![response(status, &[("x-ratelimit-remaining", "0")], "{}")],
            Some("old-secret"),
        );
        assert_eq!(
            client
                .configure_token("candidate".into(), true)
                .await
                .unwrap_err()
                .code,
            ErrorCode::PrimaryRateLimit
        );
        assert_eq!(client.session().await.identity, Identity::Authenticated);
        assert_eq!(store.value.lock().unwrap().as_deref(), Some("old-secret"));
        assert!(client.state.lock().await.token.as_deref() == Some("old-secret"));
    }
}

#[tokio::test]
async fn storage_failure_keeps_memory_token_and_returns_warning() {
    let (client, _, store) = fixture(vec![response(200, &[], "{}")], None);
    store.unavailable.store(true, Ordering::SeqCst);
    let session = client.configure_token("secret".into(), true).await.unwrap();
    assert_eq!(session.identity, Identity::Authenticated);
    assert!(!session.remembered);
    assert!(session.storage_warning.is_some());
    assert!(client.state.lock().await.token.is_some());
    assert_eq!(client.clear_token().await.identity, Identity::Anonymous);
}

#[tokio::test]
async fn restored_token_401_is_revoked_and_public_get_retries_anonymously_only_once() {
    let (client, transport, store) = fixture(
        vec![
            response(401, &[("x-ratelimit-remaining", "0")], "{}"),
            response(200, &[], "{}"),
        ],
        Some("revoked-secret"),
    );
    client.public_get(endpoint()).await.unwrap();
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].token.is_some());
        assert!(requests[1].token.is_none());
        assert_eq!(requests[0].url, requests[1].url);
    }
    assert!(store.value.lock().unwrap().is_none());
    assert_eq!(client.session().await.identity, Identity::TokenInvalid);
    let (client, transport, _) = fixture(
        vec![response(401, &[], "{}"), response(401, &[], "{}")],
        Some("revoked"),
    );
    assert_eq!(
        client.public_get(endpoint()).await.unwrap_err().code,
        ErrorCode::AuthInvalid
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn success_at_zero_remains_success_but_blocks_following_request() {
    let reset = (now_ms() / 1000 + 600).to_string();
    let (client, transport, _) = fixture(
        vec![response(
            200,
            &[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", &reset),
            ],
            "{}",
        )],
        None,
    );
    assert_eq!(
        client
            .public_get(endpoint())
            .await
            .unwrap()
            .rate_limit
            .unwrap()
            .remaining,
        Some(0)
    );
    assert_eq!(
        client.public_get(endpoint()).await.unwrap_err().code,
        ErrorCode::RateWait
    );
    client.clear_token().await; // Clearing an already-anonymous session cannot evade its quota.
    assert_eq!(
        client.public_get(endpoint()).await.unwrap_err().code,
        ErrorCode::RateWait
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn only_anonymous_primary_allows_candidate_probe_and_success_reopens_requests() {
    let (client, transport, _) = fixture(
        vec![response(200, &[], "{}"), response(200, &[], "{}")],
        None,
    );
    client
        .state
        .lock()
        .await
        .gate
        .primary_until
        .insert(resource::CORE.into(), now_ms() + 600_000);
    client
        .configure_token("new-secret".into(), false)
        .await
        .unwrap();
    client.public_get(endpoint()).await.unwrap();
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
    client
        .state
        .lock()
        .await
        .gate
        .primary_until
        .insert(resource::CORE.into(), now_ms() + 600_000);
    assert_eq!(
        client
            .configure_token("other".into(), false)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateWait
    );
    client.clear_token().await;
    for secondary in [true, false] {
        let mut state = client.state.lock().await;
        if secondary {
            state.gate.secondary_until = Some(now_ms() + 600_000);
        } else {
            state.gate.unknown_until = Some(now_ms() + 600_000);
        }
        drop(state);
        client.clear_token().await;
        assert_eq!(
            client
                .configure_token("new-secret".into(), false)
                .await
                .unwrap_err()
                .code,
            ErrorCode::RateWait
        );
    }
}

#[tokio::test]
async fn global_gate_serializes_concurrent_calls_and_expired_primary_has_one_probe() {
    let reset = (now_ms() / 1000 + 600).to_string();
    let (client, transport, _) = fixture(
        vec![response(
            403,
            &[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", &reset),
            ],
            "{}",
        )],
        None,
    );
    client
        .state
        .lock()
        .await
        .gate
        .primary_until
        .insert(resource::SEARCH.into(), now_ms() - 1);
    let (first, second) =
        tokio::join!(client.public_get(endpoint()), client.public_get(endpoint()));
    assert_eq!(first.unwrap_err().code, ErrorCode::PrimaryRateLimit);
    assert_eq!(second.unwrap_err().code, ErrorCode::RateWait);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    let (client, transport, _) = fixture(
        vec![response(200, &[], "{}"), response(200, &[], "{}")],
        None,
    );
    let (first, second) = tokio::join!(
        client.public_get(api_url(&["repos", "a", "b"], &[]).unwrap()),
        client.public_get(api_url(&["repos", "c", "d"], &[]).unwrap())
    );
    first.unwrap();
    second.unwrap();
    assert_eq!(transport.max_active.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn archive_redirect_sends_token_only_to_api_and_retains_spent_quota() {
    let reset = (now_ms() / 1000 + 600).to_string();
    let (client, transport, _) = fixture(
        vec![
            response(
                302,
                &[
                    (
                        "location",
                        "https://codeload.github.com/owner/repo/legacy.zip/main",
                    ),
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", &reset),
                ],
                "",
            ),
            response(200, &[], "zip"),
        ],
        Some("secret"),
    );
    assert_eq!(
        client
            .public_zip(api_url(&["repos", "owner", "repo", "zipball", "main"], &[]).unwrap())
            .await
            .unwrap()
            .body,
        b"zip"
    );
    {
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].token.is_some());
        assert!(requests[1].token.is_none());
        assert_eq!(requests[1].url.host_str(), Some("codeload.github.com"));
    }
    // The redirect observed a core bucket with remaining=0: core is gated, but a
    // different resource (search) is not, and the next core call gets RATE_WAIT.
    assert_eq!(
        client
            .public_get(api_url(&["repos", "a", "b"], &[]).unwrap())
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateWait
    );
}

#[tokio::test]
async fn credentials_never_leave_exact_https_api_host_and_untrusted_redirect_is_not_followed() {
    for url in [
        "https://evil.test/x",
        "http://api.github.com/x",
        "https://api.github.com.evil.test/x",
        "https://user@api.github.com/x",
        "https://api.github.com:444/x",
    ] {
        let (client, transport, _) = fixture(vec![], Some("secret"));
        assert!(client.public_get(Url::parse(url).unwrap()).await.is_err());
        assert!(transport.requests.lock().unwrap().is_empty());
    }
    for location in [
        "https://evil.test/zip",
        "http://codeload.github.com/zip",
        "https://codeload.github.com.evil/zip",
        "https://user:pass@codeload.github.com/zip",
        "https://codeload.github.com:444/zip",
        "https://codeload.github.com/zip?access_token=x",
        "https://api.github.com/repos/o/r/zipball",
    ] {
        let (client, transport, _) = fixture(
            vec![response(302, &[("location", location)], "")],
            Some("secret"),
        );
        assert_eq!(
            client.public_zip(endpoint()).await.unwrap_err().code,
            ErrorCode::ClientError
        );
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
    }
    let (client, transport, _) = fixture(
        vec![response(
            302,
            &[("location", "https://api.github.com/other")],
            "",
        )],
        Some("secret"),
    );
    assert!(client.public_get(endpoint()).await.is_err());
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

#[test]
fn urls_encode_refs_and_search_queries_without_injection() {
    let branch = "feature/a?foo=1&bar=#你好";
    let url = api_url(&["repos", "owner", "repo", "zipball", branch], &[]).unwrap();
    assert_eq!(url.host_str(), Some("api.github.com"));
    assert!(url.path().contains("feature%2Fa%3Ffoo=1&bar=%23"));
    assert!(url.query().is_none());
    assert!(url.fragment().is_none());
    let url = api_url(&["contents", "plugin.json"], &[("ref", branch)]).unwrap();
    assert_eq!(
        url.query_pairs().collect::<Vec<_>>(),
        vec![("ref".into(), branch.into())]
    );
    assert!(api_url(&[".."], &[]).is_err());
}

#[tokio::test]
async fn no_etag_cache_is_fabricated_for_304() {
    let (client, transport, _) = fixture(
        vec![response(304, &[("x-ratelimit-remaining", "0")], "")],
        None,
    );
    assert_eq!(
        client.public_get(endpoint()).await.unwrap_err().code,
        ErrorCode::ClientError
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

fn search_body(count: usize) -> String {
    let repos: Vec<_> = (0..count).map(|i| serde_json::json!({
        "name": format!("plugin-{i}"), "full_name": format!("owner/plugin-{i}"), "description": null,
        "html_url": "https://github.com/owner/repo", "clone_url": "https://github.com/owner/repo.git",
        "stargazers_count": 12, "forks_count": 3, "updated_at": "2026-01-01T00:00:00Z", "language": "Python",
        "topics": ["time-pin-plugins"], "default_branch": "feature/a&b",
        "owner": {"login": "owner", "avatar_url": "https://example.test/avatar", "html_url": "https://github.com/owner"}
    })).collect();
    serde_json::json!({"total_count": count, "items": repos}).to_string()
}

#[tokio::test]
async fn discovery_uses_real_snake_case_github_input_and_search_headline_rate() {
    let (client, transport, _) = fixture(
        vec![
            response(
                200,
                &[
                    ("x-ratelimit-remaining", "9"),
                    ("x-ratelimit-resource", "search"),
                ],
                &search_body(1),
            ),
            response(
                404,
                &[
                    ("x-ratelimit-remaining", "57"),
                    ("x-ratelimit-resource", "core"),
                ],
                "{}",
            ),
        ],
        None,
    );
    let result = crate::commands::github_plugins::discover(&client, 1, 20, &[])
        .await
        .unwrap();
    let value = serde_json::to_value(result).unwrap();
    assert_eq!(value["schemaVersion"], "1.0.0");
    assert_eq!(
        value["plugins"][0]["repository"]["fullName"],
        "owner/plugin-0"
    );
    assert_eq!(
        value["plugins"][0]["repository"]["owner"]["avatarUrl"],
        "https://example.test/avatar"
    );
    assert_eq!(value["totalCount"], 1);
    // rateLimit reports the search bucket (the discovery headline), not the last
    // manifest core response. Manifest snapshots still appear in rateLimits.
    assert_eq!(value["rateLimit"]["remaining"], 9);
    assert_eq!(value["rateLimit"]["resource"], "search");
    let buckets: Vec<_> = value["rateLimits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["resource"].as_str().unwrap().to_owned())
        .collect();
    assert!(buckets.contains(&"search".to_owned()));
    assert!(buckets.contains(&"core".to_owned()));
    let requests = transport.requests.lock().unwrap();
    assert_eq!(
        requests[1].url.query_pairs().collect::<Vec<_>>(),
        vec![("ref".into(), "feature/a&b".into())]
    );
}

#[tokio::test]
async fn manifest_rate_limit_bubbles_and_stops_all_later_requests() {
    let (client, transport, _) = fixture(
        vec![
            response(200, &[("x-ratelimit-remaining", "9")], &search_body(3)),
            response(
                429,
                &[("retry-after", "90")],
                r#"{"message":"secondary rate limit"}"#,
            ),
        ],
        None,
    );
    let error = crate::commands::github_plugins::discover(&client, 1, 20, &[])
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::SecondaryRateLimit);
    assert!(error.retry_at.is_some());
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

// ---------- TPIN-19: per-resource primary gates ----------

#[tokio::test]
async fn primary_gates_are_per_resource_and_search_is_not_blocked_by_core() {
    // Core bucket exhausts; the next core call waits, but search (and any other
    // resource) is still admitted — and vice versa.
    let reset = (now_ms() / 1000 + 600).to_string();
    let (client, transport, _) = fixture(
        vec![
            response(
                200,
                &[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", &reset),
                    ("x-ratelimit-resource", "core"),
                ],
                "{}",
            ),
            response(200, &[], "{}"),
            response(200, &[], "{}"),
        ],
        None,
    );
    let repos = api_url(&["repos", "a", "b"], &[]).unwrap();
    assert_eq!(
        client
            .public_get(repos.clone())
            .await
            .unwrap()
            .rate_limit
            .unwrap()
            .resource
            .as_deref(),
        Some("core")
    );
    // Same bucket: blocked without a network call.
    let wait = client.public_get(repos.clone()).await.unwrap_err();
    assert_eq!(wait.code, ErrorCode::RateWait);
    assert_eq!(
        wait.rate_limit.as_deref().unwrap().resource.as_deref(),
        Some("core")
    );
    // Different bucket: admitted.
    client.public_get(endpoint()).await.unwrap();
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn missing_resource_header_infers_bucket_from_url() {
    // A 403 primary response without x-ratelimit-resource must bucket under the
    // request's inferred resource — never silently pollute core.
    let reset = (now_ms() / 1000 + 600).to_string();
    let (client, transport, _) = fixture(
        vec![
            response(
                403,
                &[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", &reset),
                ],
                "{}",
            ),
            response(200, &[], "{}"),
        ],
        None,
    );
    assert_eq!(
        client.public_get(endpoint()).await.unwrap_err().code,
        ErrorCode::PrimaryRateLimit
    );
    // Inferred search bucket is gated; a core request is still free.
    assert_eq!(
        client.public_get(endpoint()).await.unwrap_err().code,
        ErrorCode::RateWait
    );
    client
        .public_get(api_url(&["repos", "a", "b"], &[]).unwrap())
        .await
        .unwrap();
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn global_secondary_wait_carries_no_resource() {
    // Coordinator contract: a secondary/unknown/stopped RATE_WAIT must not be
    // misread as a scoped bucket cooldown — rate_limit stays None.
    let (client, _, _) = fixture(
        vec![response(
            403,
            &[],
            r#"{"message":"You have exceeded a secondary rate limit"}"#,
        )],
        None,
    );
    assert_eq!(
        client
            .public_get(api_url(&["repos", "a", "b"], &[]).unwrap())
            .await
            .unwrap_err()
            .code,
        ErrorCode::SecondaryRateLimit
    );
    let wait = client
        .public_get(api_url(&["repos", "a", "b"], &[]).unwrap())
        .await
        .unwrap_err();
    assert_eq!(wait.code, ErrorCode::RateWait);
    assert!(wait.rate_limit.is_none());
    // And identity switches cannot reopen a global secondary cooldown.
    client.clear_token().await;
    let wait = client
        .public_get(api_url(&["repos", "a", "b"], &[]).unwrap())
        .await
        .unwrap_err();
    assert_eq!(wait.code, ErrorCode::RateWait);
    assert!(wait.rate_limit.is_none());
}

#[tokio::test]
async fn candidate_user_check_is_not_blocked_by_search_primary() {
    let reset = (now_ms() / 1000 + 600).to_string();
    let (client, transport, _) = fixture(
        vec![
            response(
                200,
                &[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", &reset),
                    ("x-ratelimit-resource", "search"),
                ],
                "{}",
            ),
            response(200, &[], "{}"),
        ],
        None,
    );
    // Search primary is spent; /user is core and must still admit the candidate.
    client.public_get(endpoint()).await.unwrap();
    client
        .configure_token("new-secret".into(), false)
        .await
        .unwrap();
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].url.as_str(), "https://api.github.com/user");
}

// ---------- TPIN-19: local search pacing ----------

#[tokio::test]
async fn search_pacing_blocks_burst_without_wall_clock_sleep() {
    let ((client, transport, _), clock) = fixture_with(
        vec![response(200, &[], "{}"), response(200, &[], "{}")],
        None,
        Some(Arc::new(ManualClock::new(now_ms()))),
        None,
    );
    // First anonymous search is admitted immediately.
    client.public_get(endpoint()).await.unwrap();
    // Immediate retry is paced locally: RATE_WAIT carrying the search resource,
    // not a synthetic server quota.
    let wait = client.public_get(endpoint()).await.unwrap_err();
    assert_eq!(wait.code, ErrorCode::RateWait);
    assert_eq!(wait.action, Action::Wait);
    assert_eq!(
        wait.rate_limit.as_deref().unwrap().resource.as_deref(),
        Some("search")
    );
    assert!(wait.retry_at.unwrap() > clock.now_ms());
    // No network request was emitted for the paced retry.
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    // Anonymous pacing is one request per 6s (10/min); after advancing past it
    // the next request is admitted again.
    clock.advance(6_001);
    client.public_get(endpoint()).await.unwrap();
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn search_pacing_uses_authenticated_interval_and_resets_on_identity_change() {
    let ((client, transport, _), clock) = fixture_with(
        vec![
            response(200, &[], "{}"),
            response(200, &[], "{}"),
            response(200, &[], "{}"),
        ],
        Some("secret"),
        Some(Arc::new(ManualClock::new(now_ms()))),
        None,
    );
    client.public_get(endpoint()).await.unwrap();
    // Authenticated pacing is 2s (30/min) — tighter than the anonymous 6s.
    let wait = client.public_get(endpoint()).await.unwrap_err();
    assert_eq!(wait.code, ErrorCode::RateWait);
    let wait_at = wait.retry_at.unwrap();
    assert!(wait_at - clock.now_ms() <= 2_000);
    clock.advance(2_001);
    client.public_get(endpoint()).await.unwrap();
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
    // Clearing credentials resets the search budget; a fresh anonymous burst is
    // allowed once, then paced again on the anonymous 6s interval.
    client.clear_token().await;
    client.public_get(endpoint()).await.unwrap();
    let wait = client.public_get(endpoint()).await.unwrap_err();
    assert_eq!(wait.code, ErrorCode::RateWait);
    assert!(transport.requests.lock().unwrap().len() == 3);
}

#[tokio::test]
async fn core_requests_are_not_subject_to_search_pacing() {
    let (client, transport, _) = fixture(
        vec![
            response(200, &[], "{}"),
            response(200, &[], "{}"),
            response(200, &[], "{}"),
        ],
        None,
    );
    // Pacing only applies to the search bucket; back-to-back core calls are
    // never locally throttled.
    for _ in 0..3 {
        client
            .public_get(api_url(&["repos", "a", "b"], &[]).unwrap())
            .await
            .unwrap();
    }
    assert_eq!(transport.requests.lock().unwrap().len(), 3);
}

// ---------- TPIN-19: shared conditional cache ----------

#[tokio::test]
async fn authenticated_200_stores_validators_and_304_reuses_cached_body() {
    let ((client, transport, _), _clock) = fixture_with(
        vec![
            response(
                200,
                &[
                    ("etag", "\"v1\""),
                    ("x-ratelimit-remaining", "5"),
                    ("x-ratelimit-resource", "core"),
                ],
                "body-v1",
            ),
            response(304, &[("x-ratelimit-remaining", "5")], ""),
        ],
        Some("secret"),
        Some(Arc::new(ManualClock::new(now_ms()))),
        None,
    );
    let url = api_url(&["repos", "a", "b"], &[]).unwrap();
    assert_eq!(
        client.public_get(url.clone()).await.unwrap().body,
        b"body-v1"
    );
    // Second call: live cache entry sends If-None-Match and a 304 reuses the body.
    let response = client.public_get(url.clone()).await.unwrap();
    assert_eq!(response.body, b"body-v1");
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].validators.etag.as_deref(), Some("\"v1\""));
}

#[tokio::test]
async fn anonymous_requests_never_send_conditionals_and_never_cache() {
    let (client, transport, _) = fixture(
        vec![
            response(200, &[("etag", "\"v1\"")], "body-v1"),
            response(304, &[], ""),
        ],
        None,
    );
    let url = api_url(&["repos", "a", "b"], &[]).unwrap();
    client.public_get(url.clone()).await.unwrap();
    // Anonymous call sends no validators; an unsolicited 304 is an error.
    assert_eq!(
        client.public_get(url).await.unwrap_err().code,
        ErrorCode::ClientError
    );
    let requests = transport.requests.lock().unwrap();
    assert!(requests[0].validators.is_empty());
    assert!(requests[1].validators.is_empty());
}

#[tokio::test]
async fn cache_is_bounded_and_expired_entries_are_not_used() {
    let ((client, transport, _), clock) = fixture_with(
        vec![
            response(200, &[("etag", "\"v1\"")], "v1"),
            response(200, &[("etag", "\"v2\"")], "v2"),
        ],
        Some("secret"),
        Some(Arc::new(ManualClock::new(now_ms()))),
        Some(CacheConfig {
            max_entries: 4,
            ttl: Duration::from_millis(1_000),
            max_bytes: 32 * 1024 * 1024,
        }),
    );
    let url = api_url(&["repos", "a", "b"], &[]).unwrap();
    client.public_get(url.clone()).await.unwrap();
    // Advance past TTL: the entry is stale, so no conditional headers go out.
    clock.advance(2_000);
    client.public_get(url.clone()).await.unwrap();
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].validators.is_empty());
}

#[tokio::test]
async fn stale_or_missing_cache_entry_makes_304_an_error() {
    let (client, transport, _) = fixture(
        vec![
            response(200, &[("etag", "\"v1\"")], "v1"),
            response(200, &[], "{}"), // /user probe for configure_token
            response(304, &[], ""),   // new identity asks again; cache is empty
        ],
        Some("secret"),
    );
    let url = api_url(&["repos", "a", "b"], &[]).unwrap();
    client.public_get(url.clone()).await.unwrap();
    // Invalidate the session by switching identity: the cache entry is cleared,
    // so a server-side 304 for a URL the client no longer tracks is an error.
    client.clear_token().await;
    client
        .configure_token("new-secret".into(), false)
        .await
        .unwrap();
    let error = client.public_get(url).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::ClientError);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    // New epoch: no stale ETag from the previous identity is sent.
    assert!(requests[2].validators.is_empty());
}

#[tokio::test]
async fn redirect_zip_uses_shared_cache_but_codeload_never_gets_credentials() {
    let reset = (now_ms() / 1000 + 600).to_string();
    let (client, transport, _) = fixture(
        vec![
            response(
                302,
                &[
                    (
                        "location",
                        "https://codeload.github.com/owner/repo/legacy.zip/main",
                    ),
                    ("etag", "\"zip-v1\""),
                    ("x-ratelimit-reset", &reset),
                ],
                "",
            ),
            response(200, &[], "zip-bytes"),
        ],
        Some("secret"),
    );
    let zip = client
        .public_zip(api_url(&["repos", "owner", "repo", "zipball", "main"], &[]).unwrap())
        .await
        .unwrap();
    assert_eq!(zip.body, b"zip-bytes");
    let requests = transport.requests.lock().unwrap();
    // The codeload leg carries neither Authorization nor cache validators.
    assert!(requests[1].token.is_none());
    assert!(requests[1].validators.is_empty());
}

// ---------- TPIN-19: identity isolation ----------

#[tokio::test]
async fn clearing_token_clears_primary_and_search_budget_but_not_global_safety() {
    let reset = (now_ms() / 1000 + 600).to_string();
    let (client, transport, _) = fixture(
        vec![
            response(
                200,
                &[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", &reset),
                    ("x-ratelimit-resource", "search"),
                ],
                "{}",
            ),
            response(200, &[], "{}"),
        ],
        Some("secret"),
    );
    // Authenticated search exhausts its bucket.
    client.public_get(endpoint()).await.unwrap();
    assert_eq!(
        client.public_get(endpoint()).await.unwrap_err().code,
        ErrorCode::RateWait
    );
    // Switching identity resets the search budget and the primary ledger.
    client.clear_token().await;
    client.public_get(endpoint()).await.unwrap();
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn rate_limits_ledger_reports_all_observed_buckets_on_session() {
    let (client, _, _) = fixture(
        vec![
            response(
                200,
                &[
                    ("x-ratelimit-remaining", "9"),
                    ("x-ratelimit-resource", "search"),
                ],
                "{}",
            ),
            response(
                200,
                &[
                    ("x-ratelimit-remaining", "57"),
                    ("x-ratelimit-resource", "core"),
                ],
                "{}",
            ),
        ],
        Some("secret"),
    );
    client.public_get(endpoint()).await.unwrap();
    client
        .public_get(api_url(&["repos", "a", "b"], &[]).unwrap())
        .await
        .unwrap();
    let session = client.session().await;
    let resources: Vec<_> = session
        .rate_limits
        .iter()
        .filter_map(|rate| rate.resource.as_deref())
        .collect();
    assert!(resources.contains(&"search"));
    assert!(resources.contains(&"core"));
}

// === TPIN-19: per-resource gates, local search pacing, ETag cache ===

#[tokio::test]
async fn primary_gate_is_per_resource_and_independent() {
    let reset = (now_ms() / 1000 + 600).to_string();
    let (client, transport, _) = fixture(
        vec![
            // Search exhausts: next search RATE_WAITs but core still admitted.
            response(
                403,
                &[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", &reset),
                    ("x-ratelimit-resource", "search"),
                ],
                "{}",
            ),
            response(200, &[("x-ratelimit-resource", "core")], "{}"),
        ],
        None,
    );
    assert_eq!(
        client.public_get(endpoint()).await.unwrap_err().code,
        ErrorCode::PrimaryRateLimit
    );
    let error = client.public_get(endpoint()).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::RateWait);
    assert_eq!(
        error.rate_limit.as_deref().unwrap().resource.as_deref(),
        Some("search")
    );
    assert_eq!(error.action, Action::Wait);
    // Core bucket unaffected by search primary exhaustion.
    client
        .public_get(api_url(&["repos", "a", "b"], &[]).unwrap())
        .await
        .unwrap();
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn response_without_resource_header_infers_request_bucket() {
    let reset = (now_ms() / 1000 + 600).to_string();
    let (client, _, _) = fixture(
        vec![response(
            403,
            &[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", &reset),
            ],
            "{}",
        )],
        None,
    );
    assert_eq!(
        client.public_get(endpoint()).await.unwrap_err().code,
        ErrorCode::PrimaryRateLimit
    );
    let error = client.public_get(endpoint()).await.unwrap_err();
    assert_eq!(
        error.rate_limit.as_deref().unwrap().resource.as_deref(),
        Some("search")
    );
}

#[tokio::test]
async fn local_search_pacing_returns_rate_wait_with_search_resource() {
    let ((client, transport, _), clock) = fixture_with(
        vec![
            response(200, &[], &search_body(1)),
            response(200, &[], &search_body(1)),
        ],
        None,
        Some(Arc::new(ManualClock::new(1_000_000))),
        None,
    );
    client.public_get(endpoint()).await.unwrap();
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    // Immediate retry: locally paced, no HTTP call.
    let error = client.public_get(endpoint()).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::RateWait);
    assert_eq!(error.action, Action::Wait);
    assert_eq!(
        error.rate_limit.as_deref().unwrap().resource.as_deref(),
        Some("search")
    );
    let retry_at = error.retry_at.unwrap();
    assert_eq!(retry_at, 1_006_000, "anonymous search waits 6s");
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    // After the local window, the next request goes through.
    clock.advance(6_001);
    client.public_get(endpoint()).await.unwrap();
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn authenticated_search_pacing_uses_two_second_interval() {
    let ((client, transport, _), clock) = fixture_with(
        vec![
            response(200, &[], &search_body(1)),
            response(200, &[], &search_body(1)),
        ],
        Some("secret"),
        Some(Arc::new(ManualClock::new(1_000_000))),
        None,
    );
    client.public_get(endpoint()).await.unwrap();
    let error = client.public_get(endpoint()).await.unwrap_err();
    assert_eq!(
        error.retry_at.unwrap(),
        1_002_000,
        "authenticated search waits 2s"
    );
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    clock.advance(2_001);
    client.public_get(endpoint()).await.unwrap();
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn core_calls_do_not_consume_search_pacing_budget() {
    let ((client, transport, _), _clock) = fixture_with(
        vec![
            response(200, &[], &search_body(1)),
            response(200, &[], "{}"),
            response(200, &[], "{}"),
        ],
        None,
        Some(Arc::new(ManualClock::new(1_000_000))),
        None,
    );
    client.public_get(endpoint()).await.unwrap();
    client
        .public_get(api_url(&["repos", "a", "b"], &[]).unwrap())
        .await
        .unwrap();
    client
        .public_get(api_url(&["repos", "c", "d"], &[]).unwrap())
        .await
        .unwrap();
    // Two consecutive core calls are not search-paced.
    assert_eq!(transport.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn conditional_get_sends_validators_and_reuses_cached_body() {
    let ((client, transport, _), clock) = fixture_with(
        vec![
            response(200, &[("etag", "\"abc\"")], &search_body(1)),
            response(304, &[("x-ratelimit-remaining", "8")], ""),
        ],
        Some("secret"),
        Some(Arc::new(ManualClock::new(1_000_000))),
        None,
    );
    let first = client.public_get(endpoint()).await.unwrap();
    assert_eq!(first.body, search_body(1).into_bytes());
    // Second call after the pacing window sends If-None-Match; a 304 hits cache.
    clock.advance(2_100);
    let second = client.public_get(endpoint()).await.unwrap();
    assert_eq!(second.body, first.body);
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].validators.etag.as_deref(), Some("\"abc\""));
    // 304 carries no synthetic quota decrement; observe its own remaining header.
    assert_eq!(second.rate_limit.unwrap().remaining, Some(8));
}

#[tokio::test]
async fn cached_entry_expires_after_ttl_and_etag_is_respected() {
    let ((client, transport, _), clock) = fixture_with(
        vec![
            response(200, &[("etag", "\"abc\"")], &search_body(1)),
            response(200, &[("etag", "\"def\"")], &search_body(2)),
        ],
        Some("secret"),
        Some(Arc::new(ManualClock::new(1_000_000))),
        Some(CacheConfig {
            max_entries: 100,
            ttl: Duration::from_secs(60),
            max_bytes: 32 * 1024 * 1024,
        }),
    );
    client.public_get(endpoint()).await.unwrap();
    clock.advance(2_100); // inside TTL: would revalidate
    clock.advance(60_000); // past TTL: entry gone — unconditional request
    client.public_get(endpoint()).await.unwrap();
    let requests = transport.requests.lock().unwrap();
    assert_eq!(
        requests[1].validators.etag, None,
        "expired entry sends no validators"
    );
}

#[tokio::test]
async fn cache_evicts_oldest_when_entry_bound_is_reached() {
    let ((client, transport, _), _clock) = fixture_with(
        vec![
            response(200, &[("etag", "\"a\"")], &search_body(1)),
            response(200, &[("etag", "\"b\"")], &search_body(1)),
            response(200, &[("etag", "\"c\"")], &search_body(1)),
            response(200, &[("etag", "\"d\"")], &search_body(1)),
        ],
        Some("secret"),
        Some(Arc::new(ManualClock::new(1_000_000))),
        Some(CacheConfig {
            max_entries: 2,
            ttl: Duration::from_secs(600),
            max_bytes: 32 * 1024 * 1024,
        }),
    );
    let repos_a = api_url(&["repos", "a", "one"], &[]).unwrap();
    let repos_b = api_url(&["repos", "b", "two"], &[]).unwrap();
    let repos_c = api_url(&["repos", "c", "three"], &[]).unwrap();
    client.public_get(repos_a.clone()).await.unwrap();
    client.public_get(repos_b.clone()).await.unwrap();
    client.public_get(repos_c.clone()).await.unwrap();
    // LRU bound: repos_a evicted by repos_c; repos_b still cached.
    let state = client.state.lock().await;
    assert_eq!(state.cache.entries.len(), 2);
    drop(state);
    client.public_get(repos_b).await.unwrap();
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests[3].validators.etag.as_deref(), Some("\"b\""));
}

#[tokio::test]
async fn identity_change_clears_cache_and_search_budget_but_keeps_global_gates() {
    let ((client, transport, _), clock) = fixture_with(
        vec![
            response(200, &[("etag", "\"a\"")], &search_body(1)),
            // Candidate probe: new identity accepted.
            response(200, &[("x-ratelimit-resource", "core")], "{}"),
            // Post-configure search: anonymous budget should not throttle it.
            response(200, &[("etag", "\"b\"")], &search_body(1)),
        ],
        None,
        Some(Arc::new(ManualClock::new(1_000_000))),
        None,
    );
    client.public_get(endpoint()).await.unwrap();
    clock.advance(6_001); // anonymous search window elapsed
    client
        .configure_token("new-secret".into(), false)
        .await
        .unwrap();
    // Immediately after identity change: new identity's search budget is fresh.
    // 2-second authenticated budget applies; request goes through now.
    let result = client.public_get(endpoint()).await;
    assert!(result.is_ok() || result.unwrap_err().code == ErrorCode::NetworkError);
    // The search request fired (not blocked by a stale anonymous budget).
    assert_eq!(transport.requests.lock().unwrap().len(), 3);
    // Cached etag entry from the old identity is not reused (epoch changed).
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests[2].validators.etag, None);
}

#[tokio::test]
async fn global_secondary_gate_has_no_resource_and_blocks_everything() {
    let (client, _, _) = fixture(
        vec![response(
            403,
            &[("retry-after", "60")],
            r#"{"message":"secondary rate limit"}"#,
        )],
        None,
    );
    let error = client.public_get(endpoint()).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::SecondaryRateLimit);
    // Next call: global RATE_WAIT carries no resource.
    let error = client.public_get(endpoint()).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::RateWait);
    assert!(
        error.rate_limit.is_none(),
        "global secondary wait has no resource"
    );
    let error = client
        .public_get(api_url(&["repos", "a", "b"], &[]).unwrap())
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::RateWait);
    assert!(error.rate_limit.is_none());
}

#[tokio::test]
async fn resubmitting_same_token_keeps_epoch_cache_and_search_budget() {
    let ((client, transport, _), _) = fixture_with(
        vec![
            response(200, &[("etag", "\"same-identity\"")], "cached"),
            response(200, &[], "{}"), // search
            response(200, &[], "{}"), // /user
            response(304, &[], ""),
        ],
        Some("secret"),
        Some(Arc::new(ManualClock::new(1_000_000))),
        None,
    );
    let url = api_url(&["repos", "a", "b", "contents", "plugin.json"], &[]).unwrap();
    client.public_get(url.clone()).await.unwrap();
    client.public_get(endpoint()).await.unwrap();
    client
        .configure_token("secret".into(), false)
        .await
        .unwrap();
    let wait = client.public_get(endpoint()).await.unwrap_err();
    assert_eq!(wait.code, ErrorCode::RateWait);
    assert_eq!(wait.retry_at, Some(1_002_000));
    assert_eq!(client.state.lock().await.identity_epoch, 0);
    assert_eq!(client.public_get(url).await.unwrap().body, b"cached");
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        requests[3].validators.etag.as_deref(),
        Some("\"same-identity\"")
    );
}

#[tokio::test]
async fn all_three_resources_have_independent_primary_gates() {
    let ((client, transport, _), _) = fixture_with(
        vec![
            response(
                200,
                &[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", "2000"),
                ],
                "core",
            ),
            response(
                200,
                &[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", "2000"),
                ],
                "graphql",
            ),
            response(
                200,
                &[
                    ("x-ratelimit-remaining", "0"),
                    ("x-ratelimit-reset", "2000"),
                ],
                "search",
            ),
        ],
        Some("secret"),
        Some(Arc::new(ManualClock::new(1_000_000))),
        None,
    );
    let urls = [
        api_url(&["repos", "a", "b"], &[]).unwrap(),
        api_url(&["graphql"], &[]).unwrap(),
        endpoint(),
    ];
    for url in &urls {
        client.public_get(url.clone()).await.unwrap();
    }
    for (url, expected) in urls.into_iter().zip(["core", "graphql", "search"]) {
        let wait = client.public_get(url).await.unwrap_err();
        assert_eq!(wait.code, ErrorCode::RateWait);
        assert_eq!(
            wait.rate_limit.as_deref().unwrap().resource.as_deref(),
            Some(expected)
        );
    }
    assert_eq!(client.session().await.rate_limits.len(), 3);
    assert_eq!(transport.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn missing_resource_in_manifest_errors_keeps_one_latest_core_snapshot() {
    let (client, _, _) = fixture(
        vec![
            response(200, &[("x-ratelimit-remaining", "9")], &search_body(2)),
            response(404, &[("x-ratelimit-remaining", "57")], "{}"),
            response(404, &[("x-ratelimit-remaining", "56")], "{}"),
        ],
        None,
    );
    let result = crate::commands::github_plugins::discover(&client, 1, 20, &[])
        .await
        .unwrap();
    assert_eq!(
        result.rate_limit.unwrap().resource.as_deref(),
        Some("search")
    );
    assert_eq!(result.rate_limits.len(), 2);
    let core = result
        .rate_limits
        .iter()
        .find(|r| r.resource.as_deref() == Some("core"))
        .unwrap();
    assert_eq!(core.remaining, Some(56));
}

#[test]
fn cache_keys_include_url_accept_version_and_epoch() {
    let url = api_url(&["repos", "a", "b"], &[("ref", "main")]).unwrap();
    let mut request = Request::new(url, Some("secret".into()));
    let original = request.cache_key(1);
    assert!(original != request.cache_key(2));
    request.accept = "application/vnd.github.raw+json";
    assert!(original != request.cache_key(1));
    request.accept = ACCEPT;
    request.api_version = "2099-01-01";
    assert!(original != request.cache_key(1));
    request.api_version = API_VERSION;
    request.url.set_query(Some("ref=dev"));
    assert!(original != request.cache_key(1));
}

#[test]
fn conditional_headers_are_built_only_for_authenticated_api_requests() {
    let transport = HttpTransport(reqwest::Client::new());
    let validators = Validators {
        etag: Some("\"v1\"".into()),
        last_modified: Some("Wed, 21 Oct 2015 07:28:00 GMT".into()),
    };
    let mut request = Request::new(endpoint(), Some("secret".into()));
    request.validators = validators.clone();
    let request = transport.prepare(request).unwrap();
    assert_eq!(request.headers()["if-none-match"], "\"v1\"");
    assert_eq!(
        request.headers()["if-modified-since"],
        "Wed, 21 Oct 2015 07:28:00 GMT"
    );
    assert_eq!(request.headers()["accept"], ACCEPT);
    assert_eq!(request.headers()["x-github-api-version"], API_VERSION);
    for (url, token) in [
        (endpoint(), None),
        (
            Url::parse("https://codeload.github.com/o/r/zip/main").unwrap(),
            None,
        ),
        (
            Url::parse("https://codeload.github.com/o/r/zip/main").unwrap(),
            Some("secret".into()),
        ),
    ] {
        let mut request = Request::new(url, token);
        request.validators = validators.clone();
        assert!(transport.prepare(request).is_err());
    }
}

#[tokio::test]
async fn last_modified_cache_and_url_variants_are_isolated() {
    let (client, transport, _) = fixture(
        vec![
            response(
                200,
                &[
                    ("last-modified", "Wed, 21 Oct 2015 07:28:00 GMT"),
                    ("x-ratelimit-remaining", "7"),
                ],
                "main-body",
            ),
            response(200, &[("etag", "\"dev\"")], "dev-body"),
            response(304, &[("etag", "\"main-v2\"")], ""),
            response(304, &[], ""),
        ],
        Some("secret"),
    );
    let main = api_url(
        &["repos", "a", "b", "contents", "plugin.json"],
        &[("ref", "main")],
    )
    .unwrap();
    let dev = api_url(
        &["repos", "a", "b", "contents", "plugin.json"],
        &[("ref", "dev")],
    )
    .unwrap();
    client.public_get(main.clone()).await.unwrap();
    client.public_get(dev).await.unwrap();
    let result = client.public_get(main.clone()).await.unwrap();
    assert_eq!(result.body, b"main-body");
    assert_eq!(result.rate_limit.unwrap().remaining, Some(7));
    assert_eq!(client.public_get(main).await.unwrap().body, b"main-body");
    let requests = transport.requests.lock().unwrap();
    assert!(requests[1].validators.is_empty());
    assert_eq!(
        requests[2].validators.last_modified.as_deref(),
        Some("Wed, 21 Oct 2015 07:28:00 GMT")
    );
    assert_eq!(requests[3].validators.etag.as_deref(), Some("\"main-v2\""));
}

#[tokio::test]
async fn byte_budget_evicts_and_skips_large_archive_bodies() {
    let ((client, transport, _), _) = fixture_with(
        vec![
            response(200, &[("etag", "\"a\"")], &"a".repeat(100)),
            response(200, &[("etag", "\"b\"")], &"b".repeat(100)),
            response(200, &[("etag", "\"large\"")], &"z".repeat(1_000)),
            response(200, &[], "again"),
        ],
        Some("secret"),
        None,
        Some(CacheConfig {
            max_entries: 100,
            ttl: Duration::from_secs(600),
            max_bytes: 800,
        }),
    );
    let a = api_url(&["repos", "a", "a", "zipball", "main"], &[]).unwrap();
    let b = api_url(&["repos", "b", "b", "zipball", "main"], &[]).unwrap();
    let large = api_url(&["repos", "c", "c", "zipball", "main"], &[]).unwrap();
    client.public_zip(a.clone()).await.unwrap();
    client.public_zip(b).await.unwrap();
    client.public_zip(large).await.unwrap();
    let state = client.state.lock().await;
    assert_eq!(state.cache.entries.len(), 1);
    assert!(state.cache.bytes <= 800);
    drop(state);
    client.public_zip(a).await.unwrap();
    assert!(transport.requests.lock().unwrap()[3].validators.is_empty());
}

#[tokio::test]
async fn expired_cache_entry_cannot_satisfy_304() {
    let ((client, transport, _), clock) = fixture_with(
        vec![
            response(200, &[("etag", "\"v1\"")], "body"),
            response(304, &[], ""),
        ],
        Some("secret"),
        None,
        Some(CacheConfig {
            max_entries: 100,
            ttl: Duration::from_secs(1),
            max_bytes: 1024,
        }),
    );
    let url = api_url(&["repos", "a", "b"], &[]).unwrap();
    client.public_get(url.clone()).await.unwrap();
    clock.advance(1_000);
    assert_eq!(
        client.public_get(url).await.unwrap_err().code,
        ErrorCode::ClientError
    );
    assert!(transport.requests.lock().unwrap()[1].validators.is_empty());
}

#[tokio::test]
async fn direct_zip_uses_shared_cache_but_redirect_final_body_is_not_cached() {
    let (client, transport, _) = fixture(
        vec![
            response(200, &[("etag", "\"direct\"")], "zip-body"),
            response(304, &[], ""),
            response(
                302,
                &[("location", "https://codeload.github.com/o/r/zip/main")],
                "",
            ),
            response(200, &[("etag", "\"codeload\"")], "downloaded"),
            response(
                302,
                &[("location", "https://codeload.github.com/o/r/zip/main")],
                "",
            ),
            response(200, &[("etag", "\"codeload\"")], "downloaded-again"),
        ],
        Some("secret"),
    );
    let url = api_url(&["repos", "o", "r", "zipball", "main"], &[]).unwrap();
    client.public_zip(url.clone()).await.unwrap();
    assert_eq!(
        client.public_zip(url.clone()).await.unwrap().body,
        b"zip-body"
    );
    assert_eq!(
        client.public_zip(url.clone()).await.unwrap().body,
        b"downloaded"
    );
    assert_eq!(
        client.public_zip(url).await.unwrap().body,
        b"downloaded-again"
    );
    let requests = transport.requests.lock().unwrap();
    assert_eq!(requests[1].validators.etag.as_deref(), Some("\"direct\""));
    for index in [3, 5] {
        assert!(requests[index].token.is_none());
        assert!(requests[index].validators.is_empty());
    }
    // codeload bytes and validators were never associated with the API URL.
    assert_ne!(requests[4].validators.etag.as_deref(), Some("\"codeload\""));
}
