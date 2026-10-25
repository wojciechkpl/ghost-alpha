//! EdgarClient HTTP behaviour against wiremock (no real network).
//!
//! Required API:
//!   EdgarClient::new(EdgarConfig) -> Result<EdgarClient, EdgarError>        (validates config)
//!   EdgarClient::with_base_urls(self, data_base: impl Into<String>, www_base: impl Into<String>) -> EdgarClient
//!       base URLs have no required trailing slash (a trailing slash must also work).
//!       www_base serves   GET /files/company_tickers.json
//!       data_base serves  GET /api/xbrl/companyfacts/CIK{10-digit}.json
//!                         GET /submissions/CIK{10-digit}.json
//!       Defaults (not tested): https://data.sec.gov and https://www.sec.gov
//!   async fn cik_for(&self, ticker: &str) -> Result<Cik, EdgarError>             (case-insensitive)
//!   async fn company_facts(&self, ticker_or_cik: &str) -> Result<CompanyFacts, EdgarError>
//!   async fn submissions(&self, ticker_or_cik: &str) -> Result<Vec<Filing>, EdgarError>
//!   async fn filings(&self, ticker_or_cik: &str, form: Option<&str>, item: Option<&str>,
//!                    since: Option<NaiveDate>) -> Result<Vec<Filing>, EdgarError>
//!       form = exact match ("8-K" does not match "8-K/A"), item = Filing::has_item, since = inclusive
//!       filing_date >= since; result keeps SEC order (newest first).
//!   `ticker_or_cik`: an all-digit string is a CIK (padded to 10 digits, no ticker lookup, no
//!       company_tickers request); anything else is a ticker.
//!   Errors: UnknownTicker { ticker, suggestions } (ticker as given by caller; suggestions = TickerMap::suggest),
//!           NotFound { url } on HTTP 404, HttpStatus { status, url } for other non-success once retries
//!           are exhausted (429 and 5xx are retried up to max_retries times with backoff
//!           retry_base_delay_ms; other 4xx are NOT retried), Parse(String) for bad JSON bodies.
//!   Every request carries `User-Agent: <config.user_agent>` and `Accept-Encoding` containing gzip.
//!   Cache: JSON bodies cached on disk under cache_dir (dir created if missing, may be nested),
//!       shared across EdgarClient instances; an entry is fresh while age < cache_ttl_hours
//!       (so cache_ttl_hours = 0 means "always refetch"). The ticker map is cached too.

use chrono::NaiveDate;
use ghost_alpha::data::edgar::*;
use std::path::PathBuf;
use tempfile::TempDir;
use wiremock::matchers::{header, header_regex, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UA: &str = "Example Research jane.doe@example.com";

fn fx(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/edgar")
            .join(name),
    )
    .unwrap()
}

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn json_ok(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

struct Env {
    data: MockServer,
    www: MockServer,
    cache: TempDir,
}

impl Env {
    async fn new() -> Env {
        Env {
            data: MockServer::start().await,
            www: MockServer::start().await,
            cache: TempDir::new().unwrap(),
        }
    }

    async fn mount_tickers(&self) {
        Mock::given(method("GET"))
            .and(path("/files/company_tickers.json"))
            .respond_with(json_ok(fx("company_tickers.json")))
            .mount(&self.www)
            .await;
    }

    async fn mount_facts(&self) {
        Mock::given(method("GET"))
            .and(path("/api/xbrl/companyfacts/CIK0000999999.json"))
            .respond_with(json_ok(fx("companyfacts_exmp.json")))
            .mount(&self.data)
            .await;
    }

    async fn mount_submissions(&self) {
        Mock::given(method("GET"))
            .and(path("/submissions/CIK0000999999.json"))
            .respond_with(json_ok(fx("submissions_exmp.json")))
            .mount(&self.data)
            .await;
    }

    fn config(&self) -> EdgarConfig {
        let mut c = EdgarConfig::new(UA, self.cache.path().join("nested").join("edgar"));
        c.requests_per_second = 1000;
        c.retry_base_delay_ms = 1;
        c.max_retries = 3;
        c
    }

    fn client_with(&self, c: EdgarConfig) -> EdgarClient {
        EdgarClient::new(c)
            .unwrap()
            .with_base_urls(self.data.uri(), self.www.uri())
    }

    fn client(&self) -> EdgarClient {
        self.client_with(self.config())
    }
}

async fn hits(s: &MockServer, p: &str) -> usize {
    s.received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == p)
        .count()
}

// ── happy paths and URL construction ─────────────────────────────────────────

#[tokio::test]
async fn test_cik_for_when_ticker_known_should_fetch_tickers_from_www_host() {
    let e = Env::new().await;
    e.mount_tickers().await;
    let cik = e.client().cik_for("EXMP").await.unwrap();
    assert_eq!(cik.as_str(), "0000999999");
    assert_eq!(hits(&e.www, "/files/company_tickers.json").await, 1);
    assert!(e.data.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_cik_for_when_lowercase_ticker_should_resolve() {
    let e = Env::new().await;
    e.mount_tickers().await;
    assert_eq!(e.client().cik_for("exmp").await.unwrap().numeric(), 999999);
}

#[tokio::test]
async fn test_company_facts_when_ticker_given_should_resolve_cik_then_fetch_from_data_host() {
    let e = Env::new().await;
    e.mount_tickers().await;
    e.mount_facts().await;
    let f = e.client().company_facts("EXMP").await.unwrap();
    assert_eq!(f.entity_name, "Example Corp");
    assert_eq!(f.facts("us-gaap", "Revenues", "USD").len(), 10);
    assert_eq!(
        hits(&e.data, "/api/xbrl/companyfacts/CIK0000999999.json").await,
        1
    );
    assert_eq!(hits(&e.www, "/files/company_tickers.json").await, 1);
}

#[tokio::test]
async fn test_company_facts_when_all_digit_id_given_should_skip_ticker_lookup() {
    let e = Env::new().await;
    e.mount_facts().await; // note: no tickers mock mounted
    for id in ["999999", "0000999999"] {
        let f = e.client_with(e.config()).company_facts(id).await.unwrap();
        assert_eq!(f.cik.as_str(), "0000999999");
    }
    assert!(e.www.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_base_urls_when_trailing_slash_should_still_build_correct_paths() {
    let e = Env::new().await;
    e.mount_facts().await;
    let c = EdgarClient::new(e.config())
        .unwrap()
        .with_base_urls(format!("{}/", e.data.uri()), format!("{}/", e.www.uri()));
    assert!(c.company_facts("999999").await.is_ok());
}

#[tokio::test]
async fn test_submissions_when_ticker_given_should_return_parsed_filings() {
    let e = Env::new().await;
    e.mount_tickers().await;
    e.mount_submissions().await;
    let v = e.client().submissions("EXMP").await.unwrap();
    assert_eq!(v.len(), 8);
    assert_eq!(v[0].form, "10-Q");
}

#[tokio::test]
async fn test_submissions_when_body_has_mismatched_columns_should_error_parse() {
    let e = Env::new().await;
    Mock::given(method("GET"))
        .and(path("/submissions/CIK0000999999.json"))
        .respond_with(json_ok(fx("submissions_malformed.json")))
        .mount(&e.data)
        .await;
    let r = e.client().submissions("999999").await;
    assert!(matches!(r, Err(EdgarError::Parse(_))), "{r:?}");
}

#[tokio::test]
async fn test_company_facts_when_body_is_not_json_should_error_parse() {
    let e = Env::new().await;
    Mock::given(method("GET"))
        .and(path("/api/xbrl/companyfacts/CIK0000999999.json"))
        .respond_with(json_ok("<html>nope</html>".into()))
        .mount(&e.data)
        .await;
    let r = e.client().company_facts("999999").await;
    assert!(matches!(r, Err(EdgarError::Parse(_))), "{r:?}");
}

// ── filings filters ──────────────────────────────────────────────────────────

fn accns(v: &[Filing]) -> Vec<&str> {
    v.iter().map(|f| f.accession_number.as_str()).collect()
}

#[tokio::test]
async fn test_filings_when_form_and_item_filter_should_return_only_matching_8k_newest_first() {
    let e = Env::new().await;
    e.mount_tickers().await;
    e.mount_submissions().await;
    let v = e
        .client()
        .filings("EXMP", Some("8-K"), Some("2.02"), None)
        .await
        .unwrap();
    assert_eq!(
        accns(&v),
        vec![
            "0000999999-24-000035",
            "0000999999-24-000011",
            "0000999999-23-000041"
        ]
    );
    assert!(v.iter().all(|f| f.form == "8-K" && f.has_item("2.02")));
}

#[tokio::test]
async fn test_filings_when_item_filter_without_match_should_be_empty() {
    let e = Env::new().await;
    e.mount_tickers().await;
    e.mount_submissions().await;
    let v = e
        .client()
        .filings("EXMP", Some("8-K"), Some("8.01"), None)
        .await
        .unwrap();
    assert!(v.is_empty());
}

#[tokio::test]
async fn test_filings_when_since_given_should_be_inclusive_lower_bound() {
    let e = Env::new().await;
    e.mount_tickers().await;
    e.mount_submissions().await;
    let v = e
        .client()
        .filings("EXMP", Some("8-K"), Some("2.02"), Some(d("2024-02-20")))
        .await
        .unwrap();
    assert_eq!(
        accns(&v),
        vec!["0000999999-24-000035", "0000999999-24-000011"]
    );
}

#[tokio::test]
async fn test_filings_when_form_filter_only_should_ignore_items() {
    let e = Env::new().await;
    e.mount_tickers().await;
    e.mount_submissions().await;
    let v = e
        .client()
        .filings("EXMP", Some("10-K"), None, None)
        .await
        .unwrap();
    assert_eq!(accns(&v), vec!["0000999999-24-000012"]); // 10-K/A is a different form
}

#[tokio::test]
async fn test_filings_when_no_filters_should_return_everything_in_sec_order() {
    let e = Env::new().await;
    e.mount_tickers().await;
    e.mount_submissions().await;
    let v = e.client().filings("EXMP", None, None, None).await.unwrap();
    assert_eq!(v.len(), 8);
    assert_eq!(v[0].filing_date, d("2024-05-03"));
}

// ── headers ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_requests_when_sent_should_carry_configured_user_agent_and_gzip_on_both_hosts() {
    let e = Env::new().await;
    // Mocks only answer when the headers are right; otherwise wiremock replies 404 => NotFound.
    Mock::given(method("GET"))
        .and(path("/files/company_tickers.json"))
        .and(header("user-agent", UA))
        .and(header_regex("accept-encoding", "gzip"))
        .respond_with(json_ok(fx("company_tickers.json")))
        .expect(1)
        .mount(&e.www)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/xbrl/companyfacts/CIK0000999999.json"))
        .and(header("user-agent", UA))
        .and(header_regex("accept-encoding", "gzip"))
        .respond_with(json_ok(fx("companyfacts_exmp.json")))
        .expect(1)
        .mount(&e.data)
        .await;
    let f = e.client().company_facts("EXMP").await;
    assert!(f.is_ok(), "{f:?}");
}

// ── retry / error mapping ────────────────────────────────────────────────────

#[tokio::test]
async fn test_company_facts_when_429_then_200_should_retry_and_succeed() {
    let e = Env::new().await;
    Mock::given(method("GET"))
        .and(path("/api/xbrl/companyfacts/CIK0000999999.json"))
        .respond_with(ResponseTemplate::new(429))
        .up_to_n_times(2)
        .with_priority(1)
        .mount(&e.data)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/xbrl/companyfacts/CIK0000999999.json"))
        .respond_with(json_ok(fx("companyfacts_exmp.json")))
        .with_priority(5)
        .mount(&e.data)
        .await;
    let f = e.client().company_facts("999999").await.unwrap();
    assert_eq!(f.entity_name, "Example Corp");
    assert_eq!(
        hits(&e.data, "/api/xbrl/companyfacts/CIK0000999999.json").await,
        3
    );
}

#[tokio::test]
async fn test_company_facts_when_503_then_200_should_retry_and_succeed() {
    let e = Env::new().await;
    Mock::given(method("GET"))
        .and(path("/api/xbrl/companyfacts/CIK0000999999.json"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&e.data)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/xbrl/companyfacts/CIK0000999999.json"))
        .respond_with(json_ok(fx("companyfacts_exmp.json")))
        .with_priority(5)
        .mount(&e.data)
        .await;
    assert!(e.client().company_facts("999999").await.is_ok());
}

#[tokio::test]
async fn test_company_facts_when_429_persists_should_give_up_after_max_retries_with_http_status() {
    let e = Env::new().await;
    Mock::given(method("GET"))
        .and(path("/api/xbrl/companyfacts/CIK0000999999.json"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&e.data)
        .await;
    let mut c = e.config();
    c.max_retries = 2;
    let r = e.client_with(c).company_facts("999999").await;
    assert!(
        matches!(r, Err(EdgarError::HttpStatus { status: 429, .. })),
        "{r:?}"
    );
    // 1 initial attempt + max_retries retries
    assert_eq!(
        hits(&e.data, "/api/xbrl/companyfacts/CIK0000999999.json").await,
        3
    );
}

#[tokio::test]
async fn test_company_facts_when_max_retries_zero_should_not_retry() {
    let e = Env::new().await;
    Mock::given(method("GET"))
        .and(path("/api/xbrl/companyfacts/CIK0000999999.json"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&e.data)
        .await;
    let mut c = e.config();
    c.max_retries = 0;
    let r = e.client_with(c).company_facts("999999").await;
    assert!(
        matches!(r, Err(EdgarError::HttpStatus { status: 500, .. })),
        "{r:?}"
    );
    assert_eq!(
        hits(&e.data, "/api/xbrl/companyfacts/CIK0000999999.json").await,
        1
    );
}

#[tokio::test]
async fn test_company_facts_when_404_should_give_typed_not_found_without_retry() {
    let e = Env::new().await;
    Mock::given(method("GET"))
        .and(path("/api/xbrl/companyfacts/CIK0000123456.json"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&e.data)
        .await;
    let r = e.client().company_facts("123456").await;
    match r {
        Err(EdgarError::NotFound { url }) => assert!(url.contains("CIK0000123456"), "{url}"),
        other => panic!("expected NotFound, got {other:?}"),
    }
    assert_eq!(
        hits(&e.data, "/api/xbrl/companyfacts/CIK0000123456.json").await,
        1
    );
}

#[tokio::test]
async fn test_company_facts_when_other_4xx_should_not_retry_and_report_status_400() {
    let e = Env::new().await;
    Mock::given(method("GET"))
        .and(path("/api/xbrl/companyfacts/CIK0000999999.json"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&e.data)
        .await;
    let r = e.client().company_facts("999999").await;
    assert!(
        matches!(r, Err(EdgarError::HttpStatus { status: 400, .. })),
        "{r:?}"
    );
    assert_eq!(
        hits(&e.data, "/api/xbrl/companyfacts/CIK0000999999.json").await,
        1
    );
}

#[tokio::test]
async fn test_cik_for_when_unknown_ticker_should_error_with_suggestions() {
    let e = Env::new().await;
    e.mount_tickers().await;
    let r = e.client().cik_for("EXM").await;
    match r {
        Err(EdgarError::UnknownTicker {
            ticker,
            suggestions,
        }) => {
            assert_eq!(ticker, "EXM");
            assert_eq!(suggestions, vec!["EXMA".to_string(), "EXMP".to_string()]);
        }
        other => panic!("expected UnknownTicker, got {other:?}"),
    }
}

#[tokio::test]
async fn test_unknown_ticker_error_message_should_mention_ticker_and_suggestions() {
    let e = Env::new().await;
    e.mount_tickers().await;
    let msg = e.client().cik_for("EXM").await.unwrap_err().to_string();
    assert!(
        msg.contains("EXM") && msg.contains("EXMP") && msg.contains("EXMA"),
        "{msg}"
    );
}

#[tokio::test]
async fn test_company_facts_when_unknown_ticker_should_not_hit_data_host() {
    let e = Env::new().await;
    e.mount_tickers().await;
    let r = e.client().company_facts("NOPE").await;
    assert!(matches!(r, Err(EdgarError::UnknownTicker { .. })), "{r:?}");
    assert!(e.data.received_requests().await.unwrap().is_empty());
}

// ── cache ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_cache_when_same_call_twice_on_one_client_should_make_no_second_request() {
    let e = Env::new().await;
    e.mount_tickers().await;
    e.mount_facts().await;
    let c = e.client();
    c.company_facts("EXMP").await.unwrap();
    let f = c.company_facts("EXMP").await.unwrap();
    assert_eq!(f.entity_name, "Example Corp");
    assert_eq!(
        hits(&e.data, "/api/xbrl/companyfacts/CIK0000999999.json").await,
        1
    );
    assert_eq!(hits(&e.www, "/files/company_tickers.json").await, 1);
}

#[tokio::test]
async fn test_cache_when_second_client_shares_cache_dir_should_read_from_disk() {
    let e = Env::new().await;
    e.mount_tickers().await;
    e.mount_facts().await;
    e.mount_submissions().await;
    e.client().company_facts("EXMP").await.unwrap();
    // New client, same cache_dir: facts + ticker map come from disk.
    e.client().company_facts("EXMP").await.unwrap();
    assert_eq!(
        hits(&e.data, "/api/xbrl/companyfacts/CIK0000999999.json").await,
        1
    );
    assert_eq!(hits(&e.www, "/files/company_tickers.json").await, 1);
    // A different resource still hits the network once, but reuses the cached ticker map.
    e.client().submissions("EXMP").await.unwrap();
    assert_eq!(hits(&e.data, "/submissions/CIK0000999999.json").await, 1);
    assert_eq!(hits(&e.www, "/files/company_tickers.json").await, 1);
}

#[tokio::test]
async fn test_cache_when_cache_dir_missing_should_be_created_and_populated() {
    let e = Env::new().await;
    e.mount_tickers().await;
    e.mount_facts().await;
    let dir = e.config().cache_dir;
    assert!(!dir.exists());
    e.client().company_facts("EXMP").await.unwrap();
    assert!(dir.is_dir());
    assert!(std::fs::read_dir(&dir).unwrap().count() >= 1);
}

#[tokio::test]
async fn test_cache_when_ttl_expired_should_refetch() {
    let e = Env::new().await;
    e.mount_tickers().await;
    e.mount_facts().await;
    let mut c = e.config();
    c.cache_ttl_hours = 0; // every entry is immediately stale
    let client = e.client_with(c);
    client.company_facts("EXMP").await.unwrap();
    client.company_facts("EXMP").await.unwrap();
    assert_eq!(
        hits(&e.data, "/api/xbrl/companyfacts/CIK0000999999.json").await,
        2
    );
    assert_eq!(hits(&e.www, "/files/company_tickers.json").await, 2);
}

#[tokio::test]
async fn test_cache_when_error_response_should_not_be_cached() {
    let e = Env::new().await;
    Mock::given(method("GET"))
        .and(path("/api/xbrl/companyfacts/CIK0000999999.json"))
        .respond_with(ResponseTemplate::new(404))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&e.data)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/xbrl/companyfacts/CIK0000999999.json"))
        .respond_with(json_ok(fx("companyfacts_exmp.json")))
        .with_priority(5)
        .mount(&e.data)
        .await;
    let c = e.client();
    assert!(matches!(
        c.company_facts("999999").await,
        Err(EdgarError::NotFound { .. })
    ));
    assert!(c.company_facts("999999").await.is_ok());
}

#[tokio::test]
async fn test_user_agent_header_matcher_sanity_wrong_ua_would_not_match() {
    // Guards the header-based tests above: a server that demands another UA must not be satisfied.
    let e = Env::new().await;
    Mock::given(method("GET"))
        .and(path("/api/xbrl/companyfacts/CIK0000999999.json"))
        .and(header("user-agent", "Someone Else else@example.com"))
        .respond_with(json_ok(fx("companyfacts_exmp.json")))
        .mount(&e.data)
        .await;
    let r = e.client().company_facts("999999").await;
    assert!(matches!(r, Err(EdgarError::NotFound { .. })), "{r:?}");
}
