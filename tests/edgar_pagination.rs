//! Submissions pagination (review finding H3).
//!
//! Contract:
//!   parse_submissions_files(&str) -> Result<Vec<SubmissionPage>, EdgarError>
//!       reads `filings.files[]` of a submissions document (empty vec when the array is empty/absent;
//!       Parse error for malformed entries)
//!   SubmissionPage { pub name: String, pub filing_count: u64, pub filing_from: NaiveDate, pub filing_to: NaiveDate }
//!       (Debug + Clone + PartialEq)
//!   parse_submissions(&str) accepts BOTH the full document (`filings.recent`) and an older-page document
//!       whose columnar arrays are at the top level (same keys as `recent`).
//!   EdgarClient::filings(id, form, item, since):
//!     * since == None: only `recent` is used, no page request is made;
//!     * since == Some(d): every page with filing_to >= d is fetched from the DATA host at
//!       `/submissions/{name}`; pages entirely older than `d` are NOT requested;
//!       results are merged, filtered, and kept in SEC order (newest first);
//!     * page bodies are cached like other resources.
//!   EdgarClient::submissions(id) returns `recent` only (never fetches pages).

use chrono::NaiveDate;
use ghost_alpha::data::edgar::*;
use std::path::PathBuf;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UA: &str = "Example Research jane.doe@example.com";
const P1: &str = "/submissions/CIK0000999999-submissions-001.json";
const P2: &str = "/submissions/CIK0000999999-submissions-002.json";
const MAIN: &str = "/submissions/CIK0000999999.json";

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

fn ok(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

struct Env {
    data: MockServer,
    www: MockServer,
    cache: TempDir,
}

impl Env {
    async fn new() -> Env {
        let e = Env {
            data: MockServer::start().await,
            www: MockServer::start().await,
            cache: TempDir::new().unwrap(),
        };
        for (p, f) in [
            (MAIN, "submissions_paged.json"),
            (P1, "submissions_page_001.json"),
            (P2, "submissions_page_002.json"),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ok(fx(f)))
                .mount(&e.data)
                .await;
        }
        e
    }

    fn client(&self) -> EdgarClient {
        let mut c = EdgarConfig::new(UA, self.cache.path().join("c"));
        c.requests_per_second = 1000;
        c.retry_base_delay_ms = 1;
        c.max_retries = 0;
        EdgarClient::new(c)
            .unwrap()
            .with_base_urls(self.data.uri(), self.www.uri())
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

fn accns(v: &[Filing]) -> Vec<&str> {
    v.iter().map(|f| f.accession_number.as_str()).collect()
}

// ── parsers ──────────────────────────────────────────────────────────────────

#[test]
fn test_parse_submissions_files_when_pages_listed_should_return_name_count_and_range() {
    let v = parse_submissions_files(&fx("submissions_paged.json")).unwrap();
    assert_eq!(
        v,
        vec![
            SubmissionPage {
                name: "CIK0000999999-submissions-001.json".into(),
                filing_count: 3,
                filing_from: d("2019-05-02"),
                filing_to: d("2021-11-04"),
            },
            SubmissionPage {
                name: "CIK0000999999-submissions-002.json".into(),
                filing_count: 2,
                filing_from: d("2012-03-01"),
                filing_to: d("2018-11-01"),
            },
        ]
    );
}

#[test]
fn test_parse_submissions_files_when_no_pages_should_be_empty() {
    assert!(parse_submissions_files(&fx("submissions_exmp.json"))
        .unwrap()
        .is_empty());
}

#[test]
fn test_parse_submissions_files_when_entry_malformed_should_error_parse() {
    for bad in [
        "not json",
        r#"{"filings":{"recent":{},"files":[{"name":"x.json"}]}}"#,
        r#"{"filings":{"recent":{},"files":[{"name":"x.json","filingCount":1,"filingFrom":"bad","filingTo":"2020-01-01"}]}}"#,
    ] {
        assert!(
            matches!(parse_submissions_files(bad), Err(EdgarError::Parse(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn test_parse_submissions_when_older_page_shape_should_parse_top_level_columns() {
    let v = parse_submissions(&fx("submissions_page_001.json")).unwrap();
    assert_eq!(
        accns(&v),
        vec![
            "0000999999-21-000040",
            "0000999999-21-000008",
            "0000999999-19-000020"
        ]
    );
    assert_eq!(v[0].items, vec!["2.02".to_string(), "9.01".to_string()]);
}

#[test]
fn test_parse_submissions_when_full_document_has_pages_should_return_recent_only() {
    let v = parse_submissions(&fx("submissions_paged.json")).unwrap();
    assert_eq!(v.len(), 3);
}

// ── client ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_filings_when_since_reaches_into_first_page_only_should_fetch_just_that_page() {
    let e = Env::new().await;
    let v = e
        .client()
        .filings("999999", Some("8-K"), Some("2.02"), Some(d("2021-01-01")))
        .await
        .unwrap();
    assert_eq!(
        accns(&v),
        vec![
            "0000999999-24-000011",
            "0000999999-23-000041",
            "0000999999-21-000040"
        ]
    );
    assert_eq!(hits(&e.data, P1).await, 1);
    assert_eq!(hits(&e.data, P2).await, 0, "page entirely older than since");
}

#[tokio::test]
async fn test_filings_when_since_equals_page_filing_to_should_fetch_that_page() {
    let e = Env::new().await;
    let v = e
        .client()
        .filings("999999", Some("8-K"), None, Some(d("2021-11-04")))
        .await
        .unwrap();
    assert_eq!(
        accns(&v),
        vec![
            "0000999999-24-000011",
            "0000999999-23-000041",
            "0000999999-21-000040"
        ]
    );
    assert_eq!(hits(&e.data, P1).await, 1);
    assert_eq!(hits(&e.data, P2).await, 0);
}

#[tokio::test]
async fn test_filings_when_since_falls_inside_a_page_should_apply_inclusive_bound_on_merged_rows() {
    let e = Env::new().await;
    let v = e
        .client()
        .filings("999999", Some("8-K"), Some("2.02"), Some(d("2019-05-02")))
        .await
        .unwrap();
    assert_eq!(v.last().unwrap().accession_number, "0000999999-19-000020");
    assert_eq!(v.len(), 4);
    assert_eq!(hits(&e.data, P2).await, 0);
}

#[tokio::test]
async fn test_filings_when_since_reaches_oldest_page_should_fetch_all_and_keep_sec_order() {
    let e = Env::new().await;
    let v = e
        .client()
        .filings("999999", Some("8-K"), Some("2.02"), Some(d("2000-01-01")))
        .await
        .unwrap();
    assert_eq!(
        accns(&v),
        vec![
            "0000999999-24-000011",
            "0000999999-23-000041",
            "0000999999-21-000040",
            "0000999999-19-000020",
            "0000999999-18-000041",
            "0000999999-12-000001",
        ]
    );
    assert_eq!(hits(&e.data, P1).await, 1);
    assert_eq!(hits(&e.data, P2).await, 1);
    let dates: Vec<_> = v.iter().map(|f| f.filing_date).collect();
    let mut sorted = dates.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(dates, sorted, "newest first");
}

#[tokio::test]
async fn test_filings_when_since_is_none_should_not_fetch_any_page() {
    let e = Env::new().await;
    let v = e
        .client()
        .filings("999999", Some("8-K"), Some("2.02"), None)
        .await
        .unwrap();
    assert_eq!(
        accns(&v),
        vec!["0000999999-24-000011", "0000999999-23-000041"]
    );
    assert_eq!(hits(&e.data, P1).await, 0);
    assert_eq!(hits(&e.data, P2).await, 0);
}

#[tokio::test]
async fn test_filings_when_since_newer_than_every_page_should_not_fetch_pages() {
    let e = Env::new().await;
    let v = e
        .client()
        .filings("999999", None, None, Some(d("2023-01-01")))
        .await
        .unwrap();
    assert_eq!(v.len(), 3);
    assert_eq!(hits(&e.data, P1).await + hits(&e.data, P2).await, 0);
}

#[tokio::test]
async fn test_submissions_when_pages_exist_should_return_recent_only_without_page_requests() {
    let e = Env::new().await;
    let v = e.client().submissions("999999").await.unwrap();
    assert_eq!(v.len(), 3);
    assert_eq!(hits(&e.data, P1).await + hits(&e.data, P2).await, 0);
}

#[tokio::test]
async fn test_filings_when_pages_requested_twice_should_cache_page_bodies() {
    let e = Env::new().await;
    let c = e.client();
    for _ in 0..2 {
        c.filings("999999", None, None, Some(d("2000-01-01")))
            .await
            .unwrap();
    }
    assert_eq!(hits(&e.data, P1).await, 1);
    assert_eq!(hits(&e.data, P2).await, 1);
    assert_eq!(hits(&e.data, MAIN).await, 1);
}

#[tokio::test]
async fn test_filings_when_a_needed_page_is_missing_should_propagate_not_found() {
    let e = Env::new().await;
    e.data.reset().await;
    Mock::given(method("GET"))
        .and(path(MAIN))
        .respond_with(ok(fx("submissions_paged.json")))
        .mount(&e.data)
        .await;
    let r = e
        .client()
        .filings("999999", None, None, Some(d("2021-01-01")))
        .await;
    assert!(matches!(r, Err(EdgarError::NotFound { .. })), "{r:?}");
}
