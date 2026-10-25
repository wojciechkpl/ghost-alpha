//! HTTP-client hardening from the code review.
//!
//! Contract:
//!   M1  EdgarError::Forbidden { url: String } for HTTP 403: never retried; Display mentions the
//!       User-Agent and the rate limit / waiting about 10 minutes.
//!   M2  `Retry-After: <seconds>` on a retryable response is honoured (waited at least that long, even when
//!       retry_base_delay_ms is tiny). Errors building the request (e.g. a base URL without a scheme) are not
//!       retried, return promptly, and are EdgarError::InvalidConfig(msg) with msg containing the bad URL.
//!   M6  Stale-if-error: when a cache entry is expired and the refetch fails with 403 / 5xx / a network error,
//!       the stale cached data is returned. Without any cache entry the error is still returned.
//!   L2  Cache entries are namespaced per base URL host(:port): two clients with different base URLs sharing
//!       one cache_dir never serve each other's data.
//!   L11 NotFound for companyfacts mentions (Display, case-insensitive) that the company may not file XBRL
//!       financial data.

use ghost_alpha::data::edgar::*;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UA: &str = "Example Research jane.doe@example.com";
const FACTS: &str = "/api/xbrl/companyfacts/CIK0000999999.json";
const TICKERS: &str = "/files/company_tickers.json";

fn fx(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/edgar")
            .join(name),
    )
    .unwrap()
}

fn ok(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

struct Env {
    data: MockServer,
    www: MockServer,
}

impl Env {
    async fn new() -> Env {
        Env {
            data: MockServer::start().await,
            www: MockServer::start().await,
        }
    }

    async fn serve_ok(&self, entity: &str) {
        Mock::given(method("GET"))
            .and(path(TICKERS))
            .respond_with(ok(fx("company_tickers.json")))
            .mount(&self.www)
            .await;
        let body = fx("companyfacts_exmp.json").replace("Example Corp", entity);
        Mock::given(method("GET"))
            .and(path(FACTS))
            .respond_with(ok(body))
            .mount(&self.data)
            .await;
    }

    async fn serve_status(&self, status: u16) {
        self.data.reset().await;
        self.www.reset().await;
        for s in [&self.data, &self.www] {
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .mount(s)
                .await;
        }
    }

    /// No retries: tests that assert exact request counts must not be perturbed by a
    /// transient connection error being silently retried (flaky on Linux CI).
    fn client_in(&self, cache: &std::path::Path, ttl: u64) -> EdgarClient {
        let mut c = client_config(cache, ttl);
        c.max_retries = 0;
        EdgarClient::new(c)
            .unwrap()
            .with_base_urls(self.data.uri(), self.www.uri())
    }

    /// Same as `client_in` but with one retry, for tests that exercise retry behaviour.
    fn client_in_retrying(&self, cache: &std::path::Path, ttl: u64) -> EdgarClient {
        client_for(&self.data.uri(), &self.www.uri(), cache, ttl)
    }
}

fn client_config(cache: &std::path::Path, ttl: u64) -> EdgarConfig {
    let mut c = EdgarConfig::new(UA, cache.join("c"));
    c.requests_per_second = 1000;
    c.retry_base_delay_ms = 1;
    c.max_retries = 1;
    c.cache_ttl_hours = ttl;
    c
}

fn client_for(data: &str, www: &str, cache: &std::path::Path, ttl: u64) -> EdgarClient {
    EdgarClient::new(client_config(cache, ttl))
        .unwrap()
        .with_base_urls(data, www)
}

async fn hits(s: &MockServer, p: &str) -> usize {
    s.received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == p)
        .count()
}

// ── M1 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_company_facts_when_403_should_be_forbidden_without_retry() {
    let e = Env::new().await;
    Mock::given(method("GET"))
        .and(path(FACTS))
        .respond_with(ResponseTemplate::new(403))
        .mount(&e.data)
        .await;
    let tmp = TempDir::new().unwrap();
    let r = e.client_in(tmp.path(), 24).company_facts("999999").await;
    match r {
        Err(EdgarError::Forbidden { url }) => assert!(url.contains("CIK0000999999"), "{url}"),
        other => panic!("expected Forbidden, got {other:?}"),
    }
    assert_eq!(hits(&e.data, FACTS).await, 1, "403 must not be retried");
}

#[tokio::test]
async fn test_forbidden_display_should_mention_user_agent_and_rate_limit_wait() {
    let e = Env::new().await;
    e.serve_status(403).await;
    let tmp = TempDir::new().unwrap();
    let msg = e
        .client_in(tmp.path(), 24)
        .company_facts("999999")
        .await
        .unwrap_err()
        .to_string();
    let low = msg.to_lowercase();
    assert!(msg.contains("User-Agent"), "{msg}");
    assert!(low.contains("rate limit"), "{msg}");
    assert!(low.contains("10 minutes"), "{msg}");
}

// ── M2 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_retry_when_429_has_retry_after_should_wait_at_least_that_long() {
    let e = Env::new().await;
    Mock::given(method("GET"))
        .and(path(FACTS))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "1"))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&e.data)
        .await;
    Mock::given(method("GET"))
        .and(path(FACTS))
        .respond_with(ok(fx("companyfacts_exmp.json")))
        .with_priority(5)
        .mount(&e.data)
        .await;
    let tmp = TempDir::new().unwrap();
    let t = Instant::now();
    let f = e
        .client_in_retrying(tmp.path(), 24)
        .company_facts("999999")
        .await;
    assert!(f.is_ok(), "{f:?}");
    assert!(
        t.elapsed() >= Duration::from_millis(1000),
        "waited only {:?}",
        t.elapsed()
    );
    assert_eq!(hits(&e.data, FACTS).await, 2);
}

#[tokio::test]
async fn test_request_when_base_url_has_no_scheme_should_fail_fast_with_invalid_config() {
    let tmp = TempDir::new().unwrap();
    let mut c = EdgarConfig::new(UA, tmp.path().join("c"));
    c.requests_per_second = 1000;
    c.max_retries = 3;
    c.retry_base_delay_ms = 3000; // would take >= 3s if (wrongly) retried
    let client = EdgarClient::new(c)
        .unwrap()
        .with_base_urls("data.invalid", "www.invalid");
    let t = Instant::now();
    let r = client.company_facts("999999").await;
    assert!(
        t.elapsed() < Duration::from_millis(1500),
        "{:?}",
        t.elapsed()
    );
    match r {
        Err(EdgarError::InvalidConfig(m)) => assert!(m.contains("data.invalid"), "{m}"),
        other => panic!("expected InvalidConfig, got {other:?}"),
    }
}

// ── M6 ───────────────────────────────────────────────────────────────────────

async fn populate(e: &Env, cache: &std::path::Path) {
    e.serve_ok("Example Corp").await;
    let c = e.client_in(cache, 24);
    c.company_facts("EXMP").await.unwrap();
}

#[tokio::test]
async fn test_cache_when_expired_and_server_errors_should_return_stale_data() {
    for status in [403u16, 500, 503] {
        let e = Env::new().await;
        let tmp = TempDir::new().unwrap();
        populate(&e, tmp.path()).await;
        e.serve_status(status).await;
        let stale = e.client_in(tmp.path(), 0); // ttl 0: everything is expired
        let f = stale
            .company_facts("EXMP")
            .await
            .unwrap_or_else(|err| panic!("status {status}: {err}"));
        assert_eq!(f.entity_name, "Example Corp", "status {status}");
        assert!(
            hits(&e.data, FACTS).await >= 1,
            "an expired entry must still be refetched first (status {status})"
        );
    }
}

#[tokio::test]
async fn test_cache_when_expired_and_network_down_should_return_stale_data() {
    let e = Env::new().await;
    let tmp = TempDir::new().unwrap();
    populate(&e, tmp.path()).await;
    let (data, www) = (e.data.uri(), e.www.uri());
    drop(e); // servers go away: connection refused
    let f = client_for(&data, &www, tmp.path(), 0)
        .company_facts("EXMP")
        .await
        .expect("stale data must be served when the network is down");
    assert_eq!(f.entity_name, "Example Corp");
}

#[tokio::test]
async fn test_cache_when_no_entry_and_server_errors_should_still_fail() {
    let e = Env::new().await;
    e.serve_status(503).await;
    let tmp = TempDir::new().unwrap();
    let r = e.client_in(tmp.path(), 0).company_facts("999999").await;
    assert!(
        matches!(r, Err(EdgarError::HttpStatus { status: 503, .. })),
        "{r:?}"
    );
}

#[tokio::test]
async fn test_cache_when_expired_and_refetch_succeeds_should_return_fresh_data_not_stale() {
    let e = Env::new().await;
    let tmp = TempDir::new().unwrap();
    populate(&e, tmp.path()).await;
    e.data.reset().await;
    let body = fx("companyfacts_exmp.json").replace("Example Corp", "Renamed Corp");
    Mock::given(method("GET"))
        .and(path(FACTS))
        .respond_with(ok(body))
        .mount(&e.data)
        .await;
    let f = e
        .client_in(tmp.path(), 0)
        .company_facts("999999")
        .await
        .unwrap();
    assert_eq!(f.entity_name, "Renamed Corp");
}

// ── L2 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_cache_when_two_base_urls_share_cache_dir_should_not_cross_serve() {
    let a = Env::new().await;
    let b = Env::new().await;
    a.serve_ok("Example Corp").await;
    b.serve_ok("Mirror Corp").await;
    let tmp = TempDir::new().unwrap();

    let fa = a.client_in(tmp.path(), 24).company_facts("EXMP").await;
    assert_eq!(fa.unwrap().entity_name, "Example Corp");
    let fb = b.client_in(tmp.path(), 24).company_facts("EXMP").await;
    assert_eq!(fb.unwrap().entity_name, "Mirror Corp");

    assert_eq!(
        hits(&b.data, FACTS).await,
        1,
        "B must fetch from its own host"
    );
    assert_eq!(hits(&b.www, TICKERS).await, 1);
    // and each host's entry is reused by its own client
    let fa2 = a.client_in(tmp.path(), 24).company_facts("EXMP").await;
    assert_eq!(fa2.unwrap().entity_name, "Example Corp");
    assert_eq!(hits(&a.data, FACTS).await, 1);
}

// ── L11 ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_company_facts_when_404_display_should_explain_company_may_not_file_xbrl() {
    let e = Env::new().await;
    e.serve_status(404).await;
    let tmp = TempDir::new().unwrap();
    let err = e
        .client_in(tmp.path(), 24)
        .company_facts("999999")
        .await
        .unwrap_err();
    assert!(matches!(err, EdgarError::NotFound { .. }), "{err:?}");
    let low = err.to_string().to_lowercase();
    assert!(low.contains("xbrl"), "{err}");
    assert!(low.contains("may not file"), "{err}");
}

// ── cache freshness vs. file mtime (Linux CI regression) ─────────────────────

/// Set the mtime of every file under `dir` (recursively) to `t`.
fn set_mtime_all(dir: &std::path::Path, t: std::time::SystemTime) -> usize {
    let mut n = 0;
    for entry in std::fs::read_dir(dir).unwrap() {
        let p = entry.unwrap().path();
        if p.is_dir() {
            n += set_mtime_all(&p, t);
        } else {
            let f = std::fs::OpenOptions::new().write(true).open(&p).unwrap();
            f.set_modified(t).unwrap();
            n += 1;
        }
    }
    n
}

#[tokio::test]
async fn test_cache_when_entry_mtime_is_slightly_in_the_future_should_still_be_fresh() {
    let e = Env::new().await;
    e.serve_ok("Example Corp").await;
    let tmp = TempDir::new().unwrap();
    let c = e.client_in(tmp.path(), 24);
    c.company_facts("EXMP").await.unwrap();
    assert_eq!(hits(&e.data, FACTS).await, 1);

    // Clock skew / coarse timestamps: file mtime ahead of "now" by about a minute.
    let future = std::time::SystemTime::now() + Duration::from_secs(60);
    assert!(set_mtime_all(&tmp.path().join("c"), future) >= 2);

    let f = e
        .client_in(tmp.path(), 24)
        .company_facts("EXMP")
        .await
        .unwrap();
    assert_eq!(f.entity_name, "Example Corp");
    assert_eq!(
        hits(&e.data, FACTS).await,
        1,
        "future mtime must count as fresh"
    );
    assert_eq!(hits(&e.www, TICKERS).await, 1);
}

#[tokio::test]
async fn test_cache_when_entry_mtime_is_older_than_ttl_should_refetch() {
    let e = Env::new().await;
    e.serve_ok("Example Corp").await;
    let tmp = TempDir::new().unwrap();
    e.client_in(tmp.path(), 24)
        .company_facts("EXMP")
        .await
        .unwrap();

    let old = std::time::SystemTime::now() - Duration::from_secs(25 * 3600);
    assert!(set_mtime_all(&tmp.path().join("c"), old) >= 2);

    e.client_in(tmp.path(), 24)
        .company_facts("EXMP")
        .await
        .unwrap();
    assert_eq!(hits(&e.data, FACTS).await, 2);
    assert_eq!(hits(&e.www, TICKERS).await, 2);
}

// ── F4: how far into the future an mtime may be and still count as fresh ─────

#[tokio::test]
async fn test_cache_when_entry_mtime_is_a_few_minutes_in_the_future_should_still_be_fresh() {
    let e = Env::new().await;
    e.serve_ok("Example Corp").await;
    let tmp = TempDir::new().unwrap();
    e.client_in(tmp.path(), 24)
        .company_facts("EXMP")
        .await
        .unwrap();
    let future = std::time::SystemTime::now() + Duration::from_secs(4 * 60);
    assert!(set_mtime_all(&tmp.path().join("c"), future) >= 2);
    e.client_in(tmp.path(), 24)
        .company_facts("EXMP")
        .await
        .unwrap();
    assert_eq!(hits(&e.data, FACTS).await, 1);
    assert_eq!(hits(&e.www, TICKERS).await, 1);
}

#[tokio::test]
async fn test_cache_when_entry_mtime_is_far_in_the_future_should_be_treated_as_stale() {
    for offset in [
        Duration::from_secs(24 * 3600),
        Duration::from_secs(4 * 365 * 24 * 3600),
    ] {
        let e = Env::new().await;
        e.serve_ok("Example Corp").await;
        let tmp = TempDir::new().unwrap();
        e.client_in(tmp.path(), 24)
            .company_facts("EXMP")
            .await
            .unwrap();
        let future = std::time::SystemTime::now() + offset;
        assert!(set_mtime_all(&tmp.path().join("c"), future) >= 2);
        e.client_in(tmp.path(), 24)
            .company_facts("EXMP")
            .await
            .unwrap();
        assert_eq!(hits(&e.data, FACTS).await, 2, "offset {offset:?}");
        assert_eq!(hits(&e.www, TICKERS).await, 2, "offset {offset:?}");
    }
}
