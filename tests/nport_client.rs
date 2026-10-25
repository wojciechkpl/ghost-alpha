//! `EdgarClient` N-PORT behaviour against wiremock (phase 4b). No real network.
//!
//! Contract pinned here:
//!   All N-PORT traffic goes to the *www* base URL:
//!     GET /files/company_tickers_mf.json                       fund ticker map
//!     GET /files/company_tickers.json                          (only to tell a UIT from an unknown ticker,
//!                                                               and for issuer_title)
//!     GET /cgi-bin/browse-edgar?action=getcompany&CIK=<seriesId>&type=NPORT-P&owner=include&count=40&output=atom
//!     GET /Archives/edgar/data/<numeric cik>/<accession without dashes>/primary_doc.xml
//!       (cik = the NportFiling's cik, no zero padding)
//!   Every request carries the configured User-Agent.
//!   async fund_id(&self, ticker) -> Result<FundId, EdgarError>
//!       Err(UnknownFund { ticker as given, suggestions }) when absent from the fund map AND from
//!       company_tickers.json; Err(NotNportFiler { ticker }) when absent from the fund map but a known
//!       SEC company (e.g. SPY, a unit investment trust; message mentions "N-PORT").
//!   async issuer_title(&self, ticker) -> Result<Option<String>, EdgarError>
//!       EDGAR company title from company_tickers.json (case-insensitive); None when the ticker is absent.
//!   async nport_filings(&self, series_id) -> Result<Vec<NportFiling>, EdgarError>   (feed order)
//!   async nport_as_of(&self, ticker, as_of) -> Result<(NportFiling, NportReport), EdgarError>
//!       newest original NPORT-P filed <= as_of; then up to 3 NPORT-P/A candidates (newest first) are
//!       fetched and the newest one whose period_end equals the original's replaces it. The returned
//!       NportFiling is the one the report came from. No original on/before as_of (or empty feed)
//!       => Err(NoNportFiling { ticker as given, as_of }).
//!   Cache: feed and fund map honour cache_ttl_hours; N-PORT documents are immutable per accession and
//!       are cached forever (even with cache_ttl_hours = 0).

use chrono::NaiveDate;
use ghost_alpha::data::edgar::*;
use std::path::PathBuf;
use tempfile::TempDir;
use wiremock::matchers::{header, header_regex, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UA: &str = "Example Research jane.doe@example.com";
const XLY_SERIES: &str = "S000006408";
const ACC_NEW: &str = "0001410368-26-089594"; // filed 2026-08-28, period 2026-06-30
const ACC_OLD: &str = "0001410368-26-055328"; // filed 2026-05-28, period 2026-03-31 (derived below)
const ACC_AMEND_OLDPERIOD: &str = "0001410368-26-100001"; // filed 2026-09-15, amends 2026-03-31
const ACC_AMEND_SAME: &str = "0001410368-26-100002"; // filed 2026-09-20, amends 2026-06-30

fn fx(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/edgar/nport")
            .join(name),
    )
    .unwrap()
}

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn doc_path(acc: &str) -> String {
    format!(
        "/Archives/edgar/data/1064641/{}/primary_doc.xml",
        acc.replace('-', "")
    )
}

/// The real XLY document re-dated to the previous quarter (stand-in for an older filing).
fn doc_for_period(period: &str) -> String {
    fx("xly_primary_doc.xml").replace(
        "<repPdDate>2026-06-30</repPdDate>",
        &format!("<repPdDate>{period}</repPdDate>"),
    )
}

/// The real XLY document, restated for the same period with a different Amazon weight.
fn doc_restated() -> String {
    fx("xly_primary_doc.xml").replace("22.22150378005", "21.5")
}

fn xml_ok(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/xml")
}
fn json_ok(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

struct Env {
    www: MockServer,
    data: MockServer,
    cache: TempDir,
}

impl Env {
    async fn new() -> Env {
        Env {
            www: MockServer::start().await,
            data: MockServer::start().await,
            cache: TempDir::new().unwrap(),
        }
    }

    async fn mount_json(&self, p: &str, fixture: &str) {
        Mock::given(method("GET"))
            .and(path(p))
            .and(header("user-agent", UA))
            .respond_with(json_ok(fx(fixture)))
            .mount(&self.www)
            .await;
    }

    async fn mount_maps(&self) {
        self.mount_json("/files/company_tickers_mf.json", "company_tickers_mf.json")
            .await;
        self.mount_json("/files/company_tickers.json", "company_tickers.json")
            .await;
    }

    async fn mount_feed_body(&self, body: String) {
        Mock::given(method("GET"))
            .and(path("/cgi-bin/browse-edgar"))
            .and(query_param("action", "getcompany"))
            .and(query_param("CIK", XLY_SERIES))
            .and(query_param("type", "NPORT-P"))
            .and(query_param("output", "atom"))
            .and(header("user-agent", UA))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "application/atom+xml"))
            .mount(&self.www)
            .await;
    }

    async fn mount_doc(&self, acc: &str, body: String) {
        Mock::given(method("GET"))
            .and(path(doc_path(acc)))
            .and(header("user-agent", UA))
            .respond_with(xml_ok(body))
            .mount(&self.www)
            .await;
    }

    fn config(&self) -> EdgarConfig {
        let mut c = EdgarConfig::new(UA, self.cache.path().join("edgar"));
        c.requests_per_second = 1000;
        c.retry_base_delay_ms = 1;
        c.max_retries = 1;
        c.cache_ttl_hours = 24;
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

// ── fund_id ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_fund_id_when_known_should_fetch_map_from_www_with_user_agent_and_gzip() {
    let e = Env::new().await;
    Mock::given(method("GET"))
        .and(path("/files/company_tickers_mf.json"))
        .and(header("user-agent", UA))
        .and(header_regex("accept-encoding", "gzip"))
        .respond_with(json_ok(fx("company_tickers_mf.json")))
        .mount(&e.www)
        .await;
    let id = e.client().fund_id("xly").await.unwrap();
    assert_eq!(id.cik, Cik::new(1064641));
    assert_eq!(id.series_id, XLY_SERIES);
    assert_eq!(id.class_id, "C000017594");
    assert!(e.data.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn test_fund_id_when_called_twice_should_use_cached_map() {
    let e = Env::new().await;
    e.mount_maps().await;
    let c = e.client();
    c.fund_id("XLY").await.unwrap();
    e.client().fund_id("VTI").await.unwrap(); // second instance shares the on-disk cache
    c.fund_id("IVV").await.unwrap();
    assert_eq!(hits(&e.www, "/files/company_tickers_mf.json").await, 1);
}

#[tokio::test]
async fn test_fund_id_when_unknown_ticker_should_return_unknown_fund_with_suggestions() {
    let e = Env::new().await;
    e.mount_maps().await;
    let err = e.client().fund_id("XLYY").await.unwrap_err();
    match err {
        EdgarError::UnknownFund {
            ticker,
            suggestions,
        } => {
            assert_eq!(ticker, "XLYY");
            assert!(suggestions.contains(&"XLY".to_string()), "{suggestions:?}");
        }
        other => panic!("expected UnknownFund, got {other:?}"),
    }
}

#[tokio::test]
async fn test_fund_id_when_unit_investment_trust_should_say_not_an_nport_filer() {
    let e = Env::new().await;
    e.mount_maps().await;
    let err = e.client().fund_id("SPY").await.unwrap_err();
    assert!(
        matches!(&err, EdgarError::NotNportFiler { ticker } if ticker == "SPY"),
        "{err:?}"
    );
    let msg = err.to_string();
    assert!(msg.contains("N-PORT"), "{msg}");
    assert!(msg.contains("not a registered fund series"), "{msg}");
    assert!(
        msg.contains("unit investment trust") && msg.contains("operating company"),
        "{msg}"
    );
}

// ── issuer_title ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_issuer_title_when_known_should_return_edgar_company_title() {
    let e = Env::new().await;
    e.mount_json("/files/company_tickers.json", "company_tickers.json")
        .await;
    let c = e.client();
    assert_eq!(
        c.issuer_title("amzn").await.unwrap().as_deref(),
        Some("AMAZON COM INC")
    );
    assert_eq!(
        c.issuer_title("GOOG").await.unwrap().as_deref(),
        Some("Alphabet Inc.")
    );
}

#[tokio::test]
async fn test_issuer_title_when_ticker_absent_should_be_none_not_error() {
    let e = Env::new().await;
    e.mount_json("/files/company_tickers.json", "company_tickers.json")
        .await;
    assert_eq!(e.client().issuer_title("ZZZZ").await.unwrap(), None);
}

// ── nport_filings ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_nport_filings_when_series_given_should_query_browse_edgar_atom_feed() {
    let e = Env::new().await;
    e.mount_feed_body(fx("xly_feed.atom")).await;
    let f = e.client().nport_filings(XLY_SERIES).await.unwrap();
    assert_eq!(f.len(), 4);
    assert_eq!(f[0].accession, ACC_NEW);
    assert_eq!(f[0].filing_date, d("2026-08-28"));
}

#[tokio::test]
async fn test_nport_filings_when_ttl_positive_should_cache_feed() {
    let e = Env::new().await;
    e.mount_feed_body(fx("xly_feed.atom")).await;
    e.client().nport_filings(XLY_SERIES).await.unwrap();
    e.client().nport_filings(XLY_SERIES).await.unwrap();
    assert_eq!(hits(&e.www, "/cgi-bin/browse-edgar").await, 1);
}

#[tokio::test]
async fn test_nport_filings_when_ttl_zero_should_refetch_feed() {
    let e = Env::new().await;
    e.mount_feed_body(fx("xly_feed.atom")).await;
    let mut c = e.config();
    c.cache_ttl_hours = 0;
    let c = e.client_with(c);
    c.nport_filings(XLY_SERIES).await.unwrap();
    c.nport_filings(XLY_SERIES).await.unwrap();
    assert_eq!(hits(&e.www, "/cgi-bin/browse-edgar").await, 2);
}

// ── nport_as_of: selection ───────────────────────────────────────────────────

#[tokio::test]
async fn test_nport_as_of_when_recent_date_should_return_newest_original_and_fetch_archive_path() {
    let e = Env::new().await;
    e.mount_maps().await;
    e.mount_feed_body(fx("xly_feed.atom")).await;
    e.mount_doc(ACC_NEW, fx("xly_primary_doc.xml")).await;
    let (filing, report) = e
        .client()
        .nport_as_of("XLY", d("2026-09-01"))
        .await
        .unwrap();
    assert_eq!(filing.accession, ACC_NEW);
    assert_eq!(filing.filing_date, d("2026-08-28"));
    assert_eq!(report.period_end, d("2026-06-30"));
    assert_eq!(report.series_id, XLY_SERIES);
    assert_eq!(hits(&e.www, &doc_path(ACC_NEW)).await, 1);
}

#[tokio::test]
async fn test_nport_as_of_when_earlier_date_should_pick_older_filing_and_skip_newer_document() {
    let e = Env::new().await;
    e.mount_maps().await;
    e.mount_feed_body(fx("xly_feed.atom")).await;
    e.mount_doc(ACC_OLD, doc_for_period("2026-03-31")).await;
    e.mount_doc(ACC_NEW, fx("xly_primary_doc.xml")).await;
    let (filing, report) = e
        .client()
        .nport_as_of("xly", d("2026-06-01"))
        .await
        .unwrap();
    assert_eq!(filing.accession, ACC_OLD);
    assert_eq!(report.period_end, d("2026-03-31"));
    assert_eq!(hits(&e.www, &doc_path(ACC_NEW)).await, 0, "no look-ahead");
}

#[tokio::test]
async fn test_nport_as_of_when_no_filing_on_or_before_date_should_be_no_nport_filing() {
    let e = Env::new().await;
    e.mount_maps().await;
    e.mount_feed_body(fx("xly_feed.atom")).await;
    let err = e
        .client()
        .nport_as_of("XLY", d("2020-01-01"))
        .await
        .unwrap_err();
    match err {
        EdgarError::NoNportFiling { ticker, as_of } => {
            assert_eq!(ticker, "XLY");
            assert_eq!(as_of, d("2020-01-01"));
        }
        other => panic!("expected NoNportFiling, got {other:?}"),
    }
}

#[tokio::test]
async fn test_nport_as_of_when_feed_empty_should_be_no_nport_filing() {
    let e = Env::new().await;
    e.mount_maps().await;
    e.mount_feed_body(fx("empty_feed.atom")).await;
    let err = e
        .client()
        .nport_as_of("XLY", d("2026-09-01"))
        .await
        .unwrap_err();
    assert!(matches!(err, EdgarError::NoNportFiling { .. }), "{err:?}");
}

#[tokio::test]
async fn test_nport_as_of_when_unit_investment_trust_should_be_not_nport_filer() {
    let e = Env::new().await;
    e.mount_maps().await;
    let err = e
        .client()
        .nport_as_of("SPY", d("2026-09-01"))
        .await
        .unwrap_err();
    assert!(matches!(err, EdgarError::NotNportFiler { .. }), "{err:?}");
    assert_eq!(hits(&e.www, "/cgi-bin/browse-edgar").await, 0);
}

#[tokio::test]
async fn test_nport_as_of_when_unknown_ticker_should_be_unknown_fund() {
    let e = Env::new().await;
    e.mount_maps().await;
    let err = e
        .client()
        .nport_as_of("NOPE", d("2026-09-01"))
        .await
        .unwrap_err();
    assert!(matches!(err, EdgarError::UnknownFund { .. }), "{err:?}");
}

// ── nport_as_of: amendments ──────────────────────────────────────────────────

async fn amendment_env() -> Env {
    let e = Env::new().await;
    e.mount_maps().await;
    e.mount_feed_body(fx("synthetic_feed_amendments.atom"))
        .await;
    e.mount_doc(ACC_NEW, fx("xly_primary_doc.xml")).await;
    e.mount_doc(ACC_OLD, doc_for_period("2026-03-31")).await;
    e.mount_doc(ACC_AMEND_OLDPERIOD, doc_for_period("2026-03-31"))
        .await;
    e.mount_doc(ACC_AMEND_SAME, doc_restated()).await;
    e
}

#[tokio::test]
async fn test_nport_as_of_when_no_amendment_filed_yet_should_return_original_without_fetching_any()
{
    let e = amendment_env().await;
    let (filing, _) = e
        .client()
        .nport_as_of("XLY", d("2026-09-01"))
        .await
        .unwrap();
    assert_eq!(filing.accession, ACC_NEW);
    assert_eq!(hits(&e.www, &doc_path(ACC_AMEND_OLDPERIOD)).await, 0);
    assert_eq!(hits(&e.www, &doc_path(ACC_AMEND_SAME)).await, 0);
}

#[tokio::test]
async fn test_nport_as_of_when_amendment_is_for_older_period_should_keep_newest_original() {
    let e = amendment_env().await;
    let (filing, report) = e
        .client()
        .nport_as_of("XLY", d("2026-09-16"))
        .await
        .unwrap();
    assert_eq!(
        filing.accession, ACC_NEW,
        "older-period /A must not replace"
    );
    assert_eq!(filing.form, "NPORT-P");
    assert_eq!(report.period_end, d("2026-06-30"));
}

#[tokio::test]
async fn test_nport_as_of_when_amendment_is_for_same_period_should_replace_original() {
    let e = amendment_env().await;
    let (filing, report) = e
        .client()
        .nport_as_of("XLY", d("2026-09-30"))
        .await
        .unwrap();
    assert_eq!(filing.accession, ACC_AMEND_SAME);
    assert_eq!(filing.form, "NPORT-P/A");
    assert_eq!(report.period_end, d("2026-06-30"));
    let amzn = report
        .holdings
        .iter()
        .find(|h| h.name == "Amazon.com Inc")
        .unwrap();
    approx::assert_relative_eq!(amzn.pct, 21.5);
}

#[tokio::test]
async fn test_nport_as_of_when_many_older_period_amendments_should_fetch_at_most_three() {
    let e = Env::new().await;
    e.mount_maps().await;
    // Original plus five later NPORT-P/A filings, all for an older period.
    let mut entries = String::new();
    for i in (1..=5).rev() {
        let acc = format!("0001410368-26-2000{i:02}");
        entries.push_str(&format!(
            "<entry><content type=\"text/xml\"><accession-number>{acc}</accession-number>\
             <filing-date>2026-09-{:02}</filing-date>\
             <filing-href>https://www.sec.gov/Archives/edgar/data/1064641/{}/{acc}-index.htm</filing-href>\
             <filing-type>NPORT-P/A</filing-type></content></entry>\n",
            10 + i,
            acc.replace('-', "")
        ));
        e.mount_doc(&acc, doc_for_period("2026-03-31")).await;
    }
    entries.push_str(&format!(
        "<entry><content type=\"text/xml\"><accession-number>{ACC_NEW}</accession-number>\
         <filing-date>2026-08-28</filing-date>\
         <filing-href>https://www.sec.gov/Archives/edgar/data/1064641/{}/{ACC_NEW}-index.htm</filing-href>\
         <filing-type>NPORT-P</filing-type></content></entry>\n",
        ACC_NEW.replace('-', "")
    ));
    e.mount_feed_body(format!(
        "<?xml version=\"1.0\"?><feed xmlns=\"http://www.w3.org/2005/Atom\">{entries}</feed>"
    ))
    .await;
    e.mount_doc(ACC_NEW, fx("xly_primary_doc.xml")).await;

    let (filing, _) = e
        .client()
        .nport_as_of("XLY", d("2026-12-31"))
        .await
        .unwrap();
    assert_eq!(filing.accession, ACC_NEW);
    let mut fetched = 0;
    for i in 1..=5 {
        let acc = format!("0001410368-26-2000{i:02}");
        fetched += hits(&e.www, &doc_path(&acc)).await;
    }
    assert_eq!(fetched, 3, "MAX_AMENDMENTS = 3");
}

// ── caching of immutable documents / errors ──────────────────────────────────

#[tokio::test]
async fn test_nport_as_of_when_ttl_zero_should_still_cache_documents_forever_but_refetch_feed() {
    let e = Env::new().await;
    e.mount_maps().await;
    e.mount_feed_body(fx("xly_feed.atom")).await;
    e.mount_doc(ACC_NEW, fx("xly_primary_doc.xml")).await;
    let mut cfg = e.config();
    cfg.cache_ttl_hours = 0;
    let c = e.client_with(cfg);
    c.nport_as_of("XLY", d("2026-09-01")).await.unwrap();
    c.nport_as_of("XLY", d("2026-09-01")).await.unwrap();
    assert_eq!(hits(&e.www, &doc_path(ACC_NEW)).await, 1, "doc immutable");
    assert_eq!(hits(&e.www, "/cgi-bin/browse-edgar").await, 2);
}

#[tokio::test]
async fn test_nport_as_of_when_document_404_should_return_not_found() {
    let e = Env::new().await;
    e.mount_maps().await;
    e.mount_feed_body(fx("xly_feed.atom")).await;
    let err = e
        .client()
        .nport_as_of("XLY", d("2026-09-01"))
        .await
        .unwrap_err();
    assert!(matches!(err, EdgarError::NotFound { .. }), "{err:?}");
}

#[tokio::test]
async fn test_nport_as_of_when_document_is_not_nport_xml_should_return_parse_error() {
    let e = Env::new().await;
    e.mount_maps().await;
    e.mount_feed_body(fx("xly_feed.atom")).await;
    e.mount_doc(ACC_NEW, "<html>maintenance</html>".to_string())
        .await;
    let err = e
        .client()
        .nport_as_of("XLY", d("2026-09-01"))
        .await
        .unwrap_err();
    assert!(matches!(err, EdgarError::Parse(_)), "{err:?}");
}

// ── round 2: series check, tolerant amendments ───────────────────────────────

#[tokio::test]
async fn test_nport_as_of_when_document_is_for_another_series_should_be_series_mismatch() {
    let e = Env::new().await;
    e.mount_maps().await;
    e.mount_feed_body(fx("xly_feed.atom")).await;
    // The feed points at a document that belongs to a different series (S999999001).
    e.mount_doc(ACC_NEW, fx("synthetic_nport.xml")).await;
    let err = e
        .client()
        .nport_as_of("XLY", d("2026-09-01"))
        .await
        .unwrap_err();
    match err {
        EdgarError::SeriesMismatch {
            ticker,
            expected,
            found,
        } => {
            assert_eq!(ticker, "XLY");
            assert_eq!(expected, XLY_SERIES);
            assert_eq!(found, "S999999001");
        }
        other => panic!("expected SeriesMismatch, got {other:?}"),
    }
}

#[tokio::test]
async fn test_nport_as_of_when_amendment_document_404_should_keep_original() {
    let e = Env::new().await;
    e.mount_maps().await;
    e.mount_feed_body(fx("synthetic_feed_amendments.atom"))
        .await;
    e.mount_doc(ACC_NEW, fx("xly_primary_doc.xml")).await;
    // No mocks for the two amendments: both answer 404.
    let (filing, report) = e
        .client()
        .nport_as_of("XLY", d("2026-09-30"))
        .await
        .unwrap();
    assert_eq!(filing.accession, ACC_NEW);
    assert_eq!(report.period_end, d("2026-06-30"));
}

#[tokio::test]
async fn test_nport_as_of_when_amendment_document_malformed_should_keep_original() {
    let e = Env::new().await;
    e.mount_maps().await;
    e.mount_feed_body(fx("synthetic_feed_amendments.atom"))
        .await;
    e.mount_doc(ACC_NEW, fx("xly_primary_doc.xml")).await;
    e.mount_doc(ACC_AMEND_SAME, "<html>oops</html>".to_string())
        .await;
    e.mount_doc(ACC_AMEND_OLDPERIOD, "not xml at all".to_string())
        .await;
    let (filing, _) = e
        .client()
        .nport_as_of("XLY", d("2026-09-30"))
        .await
        .unwrap();
    assert_eq!(filing.accession, ACC_NEW);
}

#[tokio::test]
async fn test_nport_as_of_when_newest_amendment_broken_but_next_is_same_period_should_use_next() {
    let e = Env::new().await;
    e.mount_maps().await;
    // Feed order: 100002 (broken), 100001 (same period here), original.
    e.mount_feed_body(fx("synthetic_feed_amendments.atom"))
        .await;
    e.mount_doc(ACC_NEW, fx("xly_primary_doc.xml")).await;
    e.mount_doc(ACC_AMEND_SAME, "<html>oops</html>".to_string())
        .await;
    e.mount_doc(ACC_AMEND_OLDPERIOD, doc_restated()).await;
    let (filing, report) = e
        .client()
        .nport_as_of("XLY", d("2026-09-30"))
        .await
        .unwrap();
    assert_eq!(filing.accession, ACC_AMEND_OLDPERIOD);
    assert_eq!(report.period_end, d("2026-06-30"));
}

#[tokio::test]
async fn test_nport_as_of_when_original_document_404_should_still_be_an_error() {
    let e = Env::new().await;
    e.mount_maps().await;
    e.mount_feed_body(fx("synthetic_feed_amendments.atom"))
        .await;
    e.mount_doc(ACC_AMEND_SAME, doc_restated()).await;
    let err = e
        .client()
        .nport_as_of("XLY", d("2026-09-30"))
        .await
        .unwrap_err();
    assert!(matches!(err, EdgarError::NotFound { .. }), "{err:?}");
}
