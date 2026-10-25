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
//!   Follow-ups: N4 one submissions request per `filings` call even with cache_ttl_hours = 0;
//!   N5 only page names matching ^CIK\d{10}-submissions-\d{3}\.json$ are requested (others skipped, Ok);
//!   N6 a failing older page (404/5xx) is skipped: `filings` returns the other results (Ok).

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
async fn test_filings_when_a_needed_page_is_missing_should_skip_it_and_return_other_results() {
    // (was: propagate NotFound) -- N6: one failing older page must not discard the rest.
    let e = Env::new().await;
    e.data.reset().await;
    Mock::given(method("GET"))
        .and(path(MAIN))
        .respond_with(ok(fx("submissions_paged.json")))
        .mount(&e.data)
        .await;
    let v = e
        .client()
        .filings("999999", None, None, Some(d("2021-01-01")))
        .await
        .unwrap();
    assert_eq!(v.len(), 3, "recent filings only; page 001 was a 404");
    assert_eq!(hits(&e.data, P1).await, 1);
}

#[tokio::test]
async fn test_filings_when_one_page_fails_with_5xx_and_other_succeeds_should_merge_the_good_page() {
    let e = Env::new().await;
    e.data.reset().await;
    Mock::given(method("GET"))
        .and(path(MAIN))
        .respond_with(ok(fx("submissions_paged.json")))
        .mount(&e.data)
        .await;
    Mock::given(method("GET"))
        .and(path(P1))
        .respond_with(ResponseTemplate::new(500))
        .mount(&e.data)
        .await;
    Mock::given(method("GET"))
        .and(path(P2))
        .respond_with(ok(fx("submissions_page_002.json")))
        .mount(&e.data)
        .await;
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
            "0000999999-18-000041",
            "0000999999-12-000001"
        ]
    );
}

#[tokio::test]
async fn test_filings_when_main_submissions_document_fails_should_still_error() {
    let e = Env::new().await;
    e.data.reset().await;
    Mock::given(method("GET"))
        .and(path(MAIN))
        .respond_with(ResponseTemplate::new(500))
        .mount(&e.data)
        .await;
    let r = e
        .client()
        .filings("999999", None, None, Some(d("2000-01-01")))
        .await;
    assert!(
        matches!(r, Err(EdgarError::HttpStatus { status: 500, .. })),
        "{r:?}"
    );
}

#[tokio::test]
async fn test_filings_when_cache_ttl_is_zero_should_request_each_document_once_per_call() {
    let e = Env::new().await;
    let mut c = EdgarConfig::new(UA, e.cache.path().join("ttl0"));
    c.requests_per_second = 1000;
    c.retry_base_delay_ms = 1;
    c.max_retries = 0;
    c.cache_ttl_hours = 0;
    let client = EdgarClient::new(c)
        .unwrap()
        .with_base_urls(e.data.uri(), e.www.uri());

    client.filings("999999", None, None, None).await.unwrap();
    assert_eq!(hits(&e.data, MAIN).await, 1, "since = None");

    client
        .filings("999999", None, None, Some(d("2000-01-01")))
        .await
        .unwrap();
    assert_eq!(hits(&e.data, MAIN).await, 2, "one more fetch, not two");
    assert_eq!(hits(&e.data, P1).await, 1);
    assert_eq!(hits(&e.data, P2).await, 1);
}

// ── N5: page-name validation ─────────────────────────────────────────────────

#[tokio::test]
async fn test_filings_when_page_names_are_hostile_should_skip_them_without_request_or_write() {
    let e = Env::new().await;
    let outside = TempDir::new().unwrap();
    let abs = format!("{}/abs_evil.json", outside.path().display());
    let mut v: serde_json::Value = serde_json::from_str(&fx("submissions_paged.json")).unwrap();
    let files = v["filings"]["files"].as_array_mut().unwrap();
    files.retain(|f| f["name"] == "CIK0000999999-submissions-001.json");
    for n in [
        "../../evil.json",
        "../evil2.json",
        "../../../evil3.json",
        "CIK999999-submissions-001.json",
        "CIK0000999999-submissions-1.json",
        "CIK0000999999-submissions-002.json?x=1",
        "sub/CIK0000999999-submissions-002.json",
        "CIK0000999999-submissions-001.json/../../evil4.json",
        // F1: non-ASCII input must be skipped, never panic in the validator
        "CIK0000999999-submissions\u{e9}01.json", // 34 bytes, multibyte char at the slice boundary
        "CIK000099999\u{e9}-submissions-001.json",
        "CIK0000999999-submissions-00\u{e9}.json",
        "CIK\u{ff10}\u{ff10}\u{ff10}\u{ff10}\u{ff10}\u{ff10}\u{ff10}\u{ff10}\u{ff10}\u{ff10}-submissions-001.json", // fullwidth digits
        "CIK0000999999-submissions-001.json\u{200b}",
        "\u{1F600}.json",
        "",
        abs.as_str(),
    ] {
        files.push(serde_json::json!({"name": n, "filingCount": 1,
            "filingFrom": "2001-01-01", "filingTo": "2020-01-01"}));
    }
    e.data.reset().await;
    Mock::given(method("GET"))
        .and(path(MAIN))
        .respond_with(ok(v.to_string()))
        .with_priority(1)
        .mount(&e.data)
        .await;
    Mock::given(method("GET"))
        .and(path(P1))
        .respond_with(ok(fx("submissions_page_001.json")))
        .with_priority(1)
        .mount(&e.data)
        .await;
    // bait: a valid-looking page for any other path
    Mock::given(method("GET"))
        .respond_with(ok(fx("submissions_page_002.json")))
        .with_priority(9)
        .mount(&e.data)
        .await;

    let got = e
        .client()
        .filings("999999", None, None, Some(d("2000-01-01")))
        .await
        .unwrap();
    assert_eq!(got.len(), 3 + 3, "recent + page 001 only");

    let paths: Vec<String> = e
        .data
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.path().to_string())
        .collect();
    for p in &paths {
        assert!(
            p == MAIN || p == P1,
            "unexpected request {p:?} in {paths:?}"
        );
    }

    let cache_root = e.cache.path().join("c");
    for name in ["evil.json", "evil2.json", "evil3.json", "evil4.json"] {
        assert!(
            !e.cache.path().join(name).exists(),
            "{name} escaped the cache"
        );
        assert!(
            !cache_root.join(name).exists(),
            "{name} written at cache root"
        );
    }
    assert!(
        !std::path::Path::new(&abs).exists(),
        "absolute page name written"
    );
    fn all_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let p = entry.unwrap().path();
            if p.is_dir() {
                all_files(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    all_files(e.cache.path(), &mut files);
    for f in files {
        assert!(
            f.starts_with(&cache_root),
            "{} is outside the cache dir",
            f.display()
        );
    }
}

// ── F3: a 403 on an older page ends the page loop ────────────────────────────

const P3: &str = "/submissions/CIK0000999999-submissions-003.json";

async fn three_page_env() -> Env {
    let e = Env::new().await;
    let mut v: serde_json::Value = serde_json::from_str(&fx("submissions_paged.json")).unwrap();
    v["filings"]["files"].as_array_mut().unwrap().push(
        serde_json::json!({"name": "CIK0000999999-submissions-003.json", "filingCount": 1,
            "filingFrom": "2009-05-01", "filingTo": "2009-05-01"}),
    );
    e.data.reset().await;
    Mock::given(method("GET"))
        .and(path(MAIN))
        .respond_with(ok(v.to_string()))
        .mount(&e.data)
        .await;
    Mock::given(method("GET"))
        .and(path(P3))
        .respond_with(ok(fx("submissions_page_003.json")))
        .mount(&e.data)
        .await;
    e
}

#[tokio::test]
async fn test_filings_when_middle_page_is_403_should_keep_earlier_pages_and_not_request_later_ones()
{
    let e = three_page_env().await;
    Mock::given(method("GET"))
        .and(path(P1))
        .respond_with(ok(fx("submissions_page_001.json")))
        .mount(&e.data)
        .await;
    Mock::given(method("GET"))
        .and(path(P2))
        .respond_with(ResponseTemplate::new(403))
        .mount(&e.data)
        .await;
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
            "0000999999-19-000020"
        ]
    );
    assert_eq!(hits(&e.data, P1).await, 1);
    assert_eq!(hits(&e.data, P2).await, 1);
    assert_eq!(hits(&e.data, P3).await, 0);
}

#[tokio::test]
async fn test_filings_when_first_page_is_403_should_return_recent_only_and_request_no_more_pages() {
    let e = three_page_env().await;
    Mock::given(method("GET"))
        .and(path(P1))
        .respond_with(ResponseTemplate::new(403))
        .mount(&e.data)
        .await;
    Mock::given(method("GET"))
        .and(path(P2))
        .respond_with(ok(fx("submissions_page_002.json")))
        .mount(&e.data)
        .await;
    let v = e
        .client()
        .filings("999999", None, None, Some(d("2000-01-01")))
        .await
        .unwrap();
    assert_eq!(v.len(), 3, "recent only");
    assert_eq!(hits(&e.data, P1).await, 1);
    assert_eq!(hits(&e.data, P2).await + hits(&e.data, P3).await, 0);
}

#[tokio::test]
async fn test_filings_when_page_is_404_should_still_continue_with_later_pages() {
    // only 403 (rate-limit block) aborts the loop; a missing page does not
    let e = three_page_env().await;
    Mock::given(method("GET"))
        .and(path(P1))
        .respond_with(ResponseTemplate::new(404))
        .mount(&e.data)
        .await;
    Mock::given(method("GET"))
        .and(path(P2))
        .respond_with(ok(fx("submissions_page_002.json")))
        .mount(&e.data)
        .await;
    let v = e
        .client()
        .filings("999999", None, None, Some(d("2000-01-01")))
        .await
        .unwrap();
    assert_eq!(v.len(), 3 + 2 + 1);
    assert_eq!(hits(&e.data, P2).await, 1);
    assert_eq!(hits(&e.data, P3).await, 1);
}
