use super::*;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex as StdMutex,
    },
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
fn fixture(
    responses: Vec<Response>,
    token: Option<&str>,
) -> (GitHubClient, Arc<MockTransport>, Arc<MemoryCredentials>) {
    let transport = Arc::new(MockTransport {
        responses: StdMutex::new(responses.into()),
        requests: StdMutex::new(Vec::new()),
        active: AtomicUsize::new(0),
        max_active: AtomicUsize::new(0),
    });
    let store = Arc::new(MemoryCredentials::default());
    *store.value.lock().unwrap() = token.map(str::to_owned);
    (
        GitHubClient::with_parts(transport.clone(), store.clone()),
        transport,
        store,
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
        .prepare(Request {
            url: endpoint(),
            token: Some("unit-test-secret".into()),
        })
        .unwrap();
    assert_eq!(
        authenticated.headers()[reqwest::header::USER_AGENT],
        USER_AGENT
    );
    assert!(authenticated.headers()[reqwest::header::AUTHORIZATION].is_sensitive());
    assert!(!format!("{authenticated:?}").contains("unit-test-secret"));
    let anonymous = transport
        .prepare(Request {
            url: endpoint(),
            token: None,
        })
        .unwrap();
    assert!(anonymous
        .headers()
        .get(reqwest::header::AUTHORIZATION)
        .is_none());
    assert_eq!(anonymous.headers()[reqwest::header::USER_AGENT], USER_AGENT);
    assert!(transport
        .prepare(Request {
            url: Url::parse("https://codeload.github.com/zip").unwrap(),
            token: Some("unit-test-secret".into())
        })
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
        gate.observe(&mut error, now);
        assert_eq!(gate.secondary_until, Some(now + delay));
        assert!(gate.check(now, true, None).is_err());
        if strike < 3 {
            assert!(gate.check(now + delay, false, None).is_ok());
            assert_eq!(error.retry_at, Some(now + delay));
        } else {
            assert!(gate
                .check(now + SECONDARY_MAX_MS * 100, true, None)
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
    high.observe(&mut err, 0);
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
    client.state.lock().await.gate.primary_until = Some(now_ms() + 600_000);
    client
        .configure_token("new-secret".into(), false)
        .await
        .unwrap();
    client.public_get(endpoint()).await.unwrap();
    assert_eq!(transport.requests.lock().unwrap().len(), 2);
    client.state.lock().await.gate.primary_until = Some(now_ms() + 600_000);
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
    client.state.lock().await.gate.primary_until = Some(now_ms() - 1);
    let (first, second) =
        tokio::join!(client.public_get(endpoint()), client.public_get(endpoint()));
    assert_eq!(first.unwrap_err().code, ErrorCode::PrimaryRateLimit);
    assert_eq!(second.unwrap_err().code, ErrorCode::RateWait);
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
    let (client, transport, _) = fixture(
        vec![response(200, &[], "{}"), response(200, &[], "{}")],
        None,
    );
    let (first, second) =
        tokio::join!(client.public_get(endpoint()), client.public_get(endpoint()));
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
    assert_eq!(
        client.public_get(endpoint()).await.unwrap_err().code,
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
async fn discovery_uses_real_snake_case_github_input_and_final_manifest_rate() {
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
    assert_eq!(value["rateLimit"]["remaining"], 57);
    assert_eq!(value["rateLimit"]["resource"], "core");
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
