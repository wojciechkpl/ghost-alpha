//! RED-phase contract for the concentration hub, slice 1 (ETF overlap):
//!   GET /portfolio/concentration   full page (donut, KPIs, positions table, lazy overlap region)
//!   GET /partials/etf-overlap      htmx fragment (look-through KPI, per-ETF rows, bar chart)
//!
//! Public API pinned here (everything else is the implementer's choice):
//!
//! * `ghost_alpha::web::AppState` gains `pub edgar: Option<Arc<ghost_alpha::data::edgar::EdgarClient>>`.
//! * `ghost_alpha::cli::edgar::build_client` becomes `pub` (not exercised here, see the plan).
//! * Nav: `("/portfolio/concentration", "Concentration")` in the "Portfolio" group of `NAV`
//!   (the pinned membership in `tests/web_shell.rs` was updated accordingly).
//! * Query params (both routes): `symbol` (A-Z0-9.- only, case-insensitive, normalised to upper
//!   case; default = largest position), `limit` (page only matters; number in (0, 100], default 25),
//!   `candidates` (comma list, default `VTI,VXUS,QQQ,SCHD`, at most 10, each A-Z0-9.- only,
//!   normalised to upper case), `buy` (finite number > 0, default 50000).
//!   Any invalid value => HTTP 400, an HTML body that never reflects the raw input, and no
//!   outbound EDGAR request.
//!
//! HTML hooks (all attributes are on the element named; nested same-name elements are fine):
//!
//! Page
//! * one `<h1>` whose text contains "Concentration" and the (upper-case) symbol
//! * KPI elements `data-kpi="<name>"`: `direct-weight` ("57.1%"), `top1` (ticker + weight),
//!   `effective-n` ("2.3"), `limit-status` (contains "Over limit" and the points over, "32.1", when
//!   the top-1 weight exceeds `limit`; contains "Within limit" otherwise: text, not only colour),
//!   `at-cost-count` (number of positions priced at cost)
//! * positions `<table>` with `<caption>`, every `<th scope="col">`, one `<tr data-ticker="XYZ">`
//!   per position; a row priced at cost contains the text "at cost"
//! * donut `<svg role="img">` with a `<title>`
//! * `<div id="etf-overlap" hx-get="/partials/etf-overlap?symbol=..&amp;limit=..&amp;candidates=..&amp;buy=.."
//!   hx-trigger="load" aria-live="polite" aria-busy="true">`; the query carries the *normalised*
//!   params with `&` escaped as `&amp;`; a `<noscript>` holds an `<a href="/partials/etf-overlap?...">`
//! * empty portfolio: HTTP 200, an `empty-state` element naming `ghost-alpha portfolio import` and
//!   `setup --section portfolio`
//!
//! Partial
//! * one `<tr data-etf="VTI">` per held fund U candidate (deduplicated, tickers upper case);
//!   cells are `<td data-col="held|pct|as-of|after|status">`:
//!   `held` "$10,000" (blank for a non-held candidate), `pct` equity % of net assets with two
//!   decimals ("3.19%"), `as-of` the N-PORT period end (YYYY-MM-DD), `after` the look-through
//!   weight of the symbol after the buy with one decimal ("25.8%"), `status` a message when the ETF
//!   could not be evaluated (error row: no `%` anywhere in the row text)
//! * the single row with the lowest `after` has `data-best="true"` and a star character in its text
//! * `data-kpi="look-through"` ("58.1%", plus the direct weight "57.1%" in its text)
//! * `data-badge="partial"` present only when a *held* fund failed
//! * bar chart `<svg role="img">` with `<title>`, no hex colours; `<details>` holding a `<table>`
//! * EDGAR not configured: HTTP 200 and the text `ghost-alpha setup --section edgar`
//!
//! Review round (regressions):
//! * `ghost_alpha::web::concentration::{MAX_HELD_FUNDS: usize = 15, ETF_TIMEOUT: Duration}` are pub.
//!   `AppState` gains `pub etf_timeout: Option<std::time::Duration>` (None = `ETF_TIMEOUT`); each ETF
//!   evaluation (`nport_as_of` + exposure) is bounded by it. A timeout gives a status containing
//!   "timed out"; for a held fund it counts as partial.
//! * The persistent `aria-live="polite"` element is an *ancestor* of `#etf-overlap` (never the swapped
//!   element). `#etf-overlap` is replaced (outerHTML) by the partial's own `#etf-overlap`, also for 400s.
//! * hx-get/noscript values are percent-encoded then HTML-escaped (`candidates=VXUS%2CQQQ`).
//! * A static JS asset linked from the page handles `htmx:beforeSwap` + `shouldSwap` for `etf-overlap`.
//! * DB failure => HTTP 500 with `role="alert"` (never the empty-state message).
//!
//! Synthetic data only: made-up lots, public tickers, fund documents derived from the shipped
//! N-PORT fixture with the issuer weight rewritten.

mod common;

use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::Router;
use common::web::*;
use ghost_alpha::data::edgar::{EdgarClient, EdgarConfig};
use ghost_alpha::storage::db::SqliteRepository;
use ghost_alpha::web::shell::NAV;
use tempfile::TempDir;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UA: &str = "Example Research jane.doe@example.com";
const PAGE: &str = "/portfolio/concentration";
const PARTIAL: &str = "/partials/etf-overlap";

// ───────────────────────────── portfolio seed ─────────────────────────────
// XYZ-style numbers on the real ticker AMZN (the issuer the N-PORT fixtures talk about):
//   AMZN 60@100 + 40@120, latest close 200 (an older 150 close must be ignored) = 20,000
//   VTI  100@80, close 100                                                      = 10,000
//   ABC  100@50, no candles => priced at cost                                   =  5,000
//   total 35,000; AMZN 57.1%; effective N 49/21 = 2.33

fn seed_portfolio(repo: &SqliteRepository) {
    insert_lot(repo, "lot-1", "AMZN", 60.0, 100.0);
    insert_lot(repo, "lot-2", "AMZN", 40.0, 120.0);
    insert_lot(repo, "lot-3", "VTI", 100.0, 80.0);
    insert_lot(repo, "lot-4", "ABC", 100.0, 50.0);
    insert_close(repo, "AMZN", "2026-10-22", 150.0);
    insert_close(repo, "AMZN", "2026-10-24", 200.0);
    insert_close(repo, "VTI", "2026-10-24", 100.0);
}

fn page_app() -> Router {
    app_with_edgar(seed_portfolio, None)
}

// ───────────────────────────── html helpers ─────────────────────────────

/// Inner text of the element opened by `open_tag`, honouring nested same-name elements.
fn inner_text(html: &str, open_tag: &str) -> String {
    let start = html.find(open_tag).expect("open tag in html") + open_tag.len();
    let name: String = open_tag[1..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    let (open, close) = (format!("<{name}"), format!("</{name}>"));
    let mut depth = 1;
    let mut i = start;
    loop {
        let rest = &html[i..];
        let next_close = rest.find(&close).expect("closing tag");
        match rest.find(&open) {
            Some(o)
                if o < next_close && rest[o + open.len()..].starts_with(['>', ' ', '/', '\n']) =>
            {
                depth += 1;
                i += o + open.len();
            }
            _ => {
                depth -= 1;
                if depth == 0 {
                    return strip_tags(&html[start..i + next_close]);
                }
                i += next_close + close.len();
            }
        }
    }
}

/// Full opening tag (any element) whose `attr` equals `val`.
fn tag_with(html: &str, attr_name: &str, val: &str) -> Option<String> {
    all_tags(html)
        .into_iter()
        .find(|t| attr(t, attr_name).as_deref() == Some(val))
}

fn kpi(html: &str, name: &str) -> String {
    let tag = tag_with(html, "data-kpi", name)
        .unwrap_or_else(|| panic!("no data-kpi=\"{name}\" element in:\n{html}"));
    inner_text(html, &tag)
}

/// The `<tr ...>...</tr>` of `data-etf=etf`, if present.
fn etf_row(html: &str, etf: &str) -> Option<String> {
    let tag = open_tags(html, "tr")
        .into_iter()
        .find(|t| attr(t, "data-etf").as_deref() == Some(etf))?;
    let s = html.find(&tag)?;
    let e = html[s..].find("</tr>")? + s + 5;
    Some(html[s..e].to_string())
}

fn cell(row: &str, col: &str) -> Option<String> {
    let tag = open_tags(row, "td")
        .into_iter()
        .find(|t| attr(t, "data-col").as_deref() == Some(col))?;
    Some(inner_text(row, &tag))
}

fn etf_rows(html: &str) -> Vec<String> {
    open_tags(html, "tr")
        .iter()
        .filter_map(|t| attr(t, "data-etf"))
        .collect()
}

/// Decoded `hx-get` of the overlap placeholder as (path, raw attribute, params).
fn placeholder(body: &str) -> (String, String, Vec<(String, String)>) {
    let tag = open_tags(body, "div")
        .into_iter()
        .find(|t| attr(t, "id").as_deref() == Some("etf-overlap"))
        .expect("<div id=\"etf-overlap\"> placeholder");
    assert_eq!(attr(&tag, "hx-trigger").as_deref(), Some("load"), "{tag}");
    assert_eq!(attr(&tag, "aria-busy").as_deref(), Some("true"), "{tag}");
    let raw = attr(&tag, "hx-get").expect("hx-get");
    parse_url(&raw)
}

/// m7: the first `aria-live="polite"` element must be a persistent *ancestor* of
/// `#etf-overlap` (not the swapped element itself), so the live region survives the swap.
fn assert_live_ancestor(body: &str) {
    let live = tag_with(body, "aria-live", "polite").expect("an aria-live=\"polite\" element");
    assert_ne!(attr(&live, "id").as_deref(), Some("etf-overlap"), "{live}");
    let start = body.find(&live).unwrap() + live.len();
    let name: String = live[1..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    let end = body[start..]
        .find(&format!("</{name}>"))
        .expect("live region close")
        + start;
    assert!(
        body[start..end].contains("id=\"etf-overlap\""),
        "#etf-overlap must live inside the persistent aria-live region: {live}"
    );
}

fn parse_url(raw: &str) -> (String, String, Vec<(String, String)>) {
    let (p, q) = raw.split_once('?').expect("query string on hx-get");
    let params = q
        .replace("&amp;", "&")
        .split('&')
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k.to_string(), v.replace("%2C", ",").replace("%2c", ","))
        })
        .collect();
    (p.to_string(), raw.to_string(), params)
}

fn param<'a>(params: &'a [(String, String)], k: &str) -> &'a str {
    params
        .iter()
        .find(|(n, _)| n == k)
        .map(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("param {k} missing in {params:?}"))
}

/// Hex colour literals in style/fill/stroke-like contexts (same rule as tests/web_charts.rs).
fn hex_literals(html: &str) -> Vec<String> {
    let b = html.as_bytes();
    let mut out = Vec::new();
    for i in 0..b.len() {
        if b[i] != b'#' || i == 0 {
            continue;
        }
        let prev = b[i - 1] as char;
        let ctx_ok = match prev {
            '"' | '\'' | ':' => true,
            ' ' => html[..i].trim_end().ends_with(':'),
            _ => false,
        };
        if !ctx_ok {
            continue;
        }
        let before = &html[i.saturating_sub(14)..i];
        if ["href=", "hx-", "for=", "url(", "aria-", "id="]
            .iter()
            .any(|k| before.contains(k))
        {
            continue;
        }
        let run: String = html[i + 1..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        let next = html[i + 1 + run.len()..].chars().next();
        let boundary = next.is_none_or(|c| !(c.is_alphanumeric() || c == '-' || c == '_'));
        if matches!(run.len(), 3 | 4 | 6 | 8) && boundary {
            out.push(format!("#{run}"));
        }
    }
    out
}

// ───────────────────────────── wiremock SEC ─────────────────────────────

struct Etf {
    ticker: &'static str,
    series: &'static str,
    cik: u32,
    acc: &'static str,
    /// Equity percent of net assets reported for Amazon (N-PORT `pctVal`, a percent).
    pct: &'static str,
    period: &'static str,
    filed: &'static str,
}

const VTI: Etf = Etf {
    ticker: "VTI",
    series: "S000002848",
    cik: 36405,
    acc: "0000036405-26-000011",
    pct: "3.19",
    period: "2026-06-30",
    filed: "2026-08-28",
};
const VXUS: Etf = Etf {
    ticker: "VXUS",
    series: "S000900001",
    cik: 1000001,
    acc: "0001000001-26-000012",
    pct: "0",
    period: "2026-07-31",
    filed: "2026-09-26",
};
const QQQ: Etf = Etf {
    ticker: "QQQ",
    series: "S000900002",
    cik: 1000002,
    acc: "0001000002-26-000013",
    pct: "4.5",
    period: "2026-07-31",
    filed: "2026-09-26",
};

fn fx(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/edgar/nport")
            .join(name),
    )
    .unwrap()
}

fn doc_for(e: &Etf) -> String {
    fx("xly_primary_doc.xml")
        .replace("S000006408", e.series)
        .replace("22.22150378005", e.pct)
        .replace(
            "<repPdDate>2026-06-30</repPdDate>",
            &format!("<repPdDate>{}</repPdDate>", e.period),
        )
}

fn feed_for(e: &Etf) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><feed xmlns=\"http://www.w3.org/2005/Atom\">\
         <entry><content type=\"text/xml\"><accession-number>{acc}</accession-number>\
         <filing-date>{filed}</filing-date>\
         <filing-href>https://www.sec.gov/Archives/edgar/data/{cik}/{nodash}/{acc}-index.htm</filing-href>\
         <filing-type>NPORT-P</filing-type></content></entry></feed>",
        acc = e.acc,
        filed = e.filed,
        cik = e.cik,
        nodash = e.acc.replace('-', ""),
    )
}

const MMF: Etf = Etf {
    ticker: "MMF",
    series: "S000900009",
    cik: 1000009,
    acc: "0001000009-26-000019",
    pct: "0",
    period: "2026-07-31",
    filed: "2026-09-26",
};

#[derive(Default)]
struct SecOpts {
    /// Tickers whose N-PORT feed answers HTTP 500.
    broken: Vec<&'static str>,
    /// Status for `company_tickers_mf.json` (None = 200 with the map).
    mf_status: Option<u16>,
    /// (ticker, delay ms) added to the N-PORT feed response.
    delays: Vec<(&'static str, u64)>,
    /// Extra funds present in the map, each with a feed and a document.
    extra: Vec<&'static Etf>,
}

struct Sec {
    server: MockServer,
    _cache: TempDir,
    client: Arc<EdgarClient>,
}

impl Sec {
    /// `broken`: tickers whose N-PORT feed answers HTTP 500.
    async fn start(broken: &[&str]) -> Sec {
        let broken = broken
            .iter()
            .map(|b| -> &'static str { Box::leak(b.to_string().into_boxed_str()) })
            .collect();
        Sec::start_with(SecOpts {
            broken,
            ..Default::default()
        })
        .await
    }

    async fn start_with(o: SecOpts) -> Sec {
        let server = MockServer::start().await;
        let funds: Vec<&Etf> = [&VTI, &VXUS, &QQQ]
            .into_iter()
            .chain(o.extra.iter().copied())
            .collect();
        let mut data: Vec<String> = funds
            .iter()
            .chain([&&MMF])
            .map(|e| {
                format!(
                    "[{},\"{}\",\"C{}\",\"{}\"]",
                    e.cik, e.series, e.series, e.ticker
                )
            })
            .collect();
        data.sort();
        let map = format!(
            "{{\"fields\":[\"cik\",\"seriesId\",\"classId\",\"symbol\"],\"data\":[{}]}}",
            data.join(",")
        );
        let mf_resp = match o.mf_status {
            Some(code) => ResponseTemplate::new(code),
            None => ResponseTemplate::new(200).set_body_raw(map, "application/json"),
        };
        Mock::given(method("GET"))
            .and(path("/files/company_tickers_mf.json"))
            .respond_with(mf_resp)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/files/company_tickers.json"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(fx("company_tickers.json"), "application/json"),
            )
            .mount(&server)
            .await;
        // Money-market style fund: in the fund map, but its N-PORT feed has no filings.
        Mock::given(method("GET"))
            .and(path("/cgi-bin/browse-edgar"))
            .and(query_param("CIK", MMF.series))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(fx("empty_feed.atom"), "application/atom+xml"),
            )
            .mount(&server)
            .await;
        for e in funds {
            let mut feed_resp = if o.broken.contains(&e.ticker) {
                ResponseTemplate::new(500)
            } else {
                ResponseTemplate::new(200).set_body_raw(feed_for(e), "application/atom+xml")
            };
            if let Some((_, ms)) = o.delays.iter().find(|(t, _)| *t == e.ticker) {
                feed_resp = feed_resp.set_delay(std::time::Duration::from_millis(*ms));
            }
            Mock::given(method("GET"))
                .and(path("/cgi-bin/browse-edgar"))
                .and(query_param("CIK", e.series))
                .respond_with(feed_resp)
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path(format!(
                    "/Archives/edgar/data/{}/{}/primary_doc.xml",
                    e.cik,
                    e.acc.replace('-', "")
                )))
                .respond_with(
                    ResponseTemplate::new(200).set_body_raw(doc_for(e), "application/xml"),
                )
                .mount(&server)
                .await;
        }
        let cache = TempDir::new().unwrap();
        let mut cfg = EdgarConfig::new(UA, cache.path().join("edgar"));
        cfg.requests_per_second = 1000;
        cfg.retry_base_delay_ms = 1;
        cfg.max_retries = 1;
        cfg.cache_ttl_hours = 24;
        let client = EdgarClient::new(cfg)
            .unwrap()
            .with_base_urls(server.uri(), server.uri());
        Sec {
            server,
            _cache: cache,
            client: Arc::new(client),
        }
    }

    /// Requests received whose URL path+query contains `needle`.
    async fn hits(&self, needle: &str) -> usize {
        self.server
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .filter(|r| {
                let u = r.url.as_str();
                u.contains(needle)
            })
            .count()
    }

    fn app(&self) -> Router {
        app_with_edgar(seed_portfolio, Some(self.client.clone()))
    }

    async fn request_count(&self) -> usize {
        self.server
            .received_requests()
            .await
            .unwrap_or_default()
            .len()
    }
}

fn pct1(w: f64) -> String {
    format!("{:.1}%", w * 100.0)
}

// ═════════════════════════════ page: nav and shell ═════════════════════════════

#[test]
fn test_nav_when_inspected_should_list_concentration_in_portfolio_group() {
    let group = NAV.iter().find(|g| g.label == "Portfolio").unwrap();
    let item = group
        .items
        .iter()
        .find(|i| i.href == PAGE)
        .expect("/portfolio/concentration in the Portfolio nav group");
    assert_eq!(item.label, "Concentration");
}

#[tokio::test]
async fn test_page_when_requested_should_mark_nav_link_aria_current_and_have_one_h1() {
    let r = get(&page_app(), PAGE).await;
    assert_eq!(r.status, StatusCode::OK);
    let body = r.text();
    let nav_start = body.find("<nav").expect("nav");
    let nav = &body[nav_start..body[nav_start..].find("</nav>").unwrap() + nav_start];
    let link = open_tags(nav, "a")
        .into_iter()
        .find(|t| attr(t, "href").as_deref() == Some(PAGE))
        .expect("nav link");
    assert_eq!(
        attr(&link, "aria-current").as_deref(),
        Some("page"),
        "{link}"
    );
    assert_eq!(count(&body, "<h1"), 1);
    let h1 = first_text(&body, "h1").unwrap();
    assert!(h1.contains("Concentration"), "{h1}");
    assert!(
        h1.contains("AMZN"),
        "default symbol is the largest position: {h1}"
    );
}

#[tokio::test]
async fn test_page_when_symbol_given_lowercase_should_override_default_symbol() {
    let r = get(&page_app(), &format!("{PAGE}?symbol=vti")).await;
    assert_eq!(r.status, StatusCode::OK);
    let body = r.text();
    assert!(first_text(&body, "h1").unwrap().contains("VTI"));
    assert!(kpi(&body, "direct-weight").contains("28.6%"), "10k of 35k");
}

// ═════════════════════════════ page: content ═════════════════════════════

#[tokio::test]
async fn test_page_when_portfolio_seeded_should_show_kpis_from_latest_close() {
    let body = get(&page_app(), PAGE).await.text();
    assert!(
        kpi(&body, "direct-weight").contains("57.1%"),
        "{}",
        kpi(&body, "direct-weight")
    );
    let top1 = kpi(&body, "top1");
    assert!(top1.contains("AMZN") && top1.contains("57.1%"), "{top1}");
    assert!(kpi(&body, "effective-n").contains("2.3"));
}

#[tokio::test]
async fn test_page_when_top1_above_default_limit_should_state_breach_in_text() {
    let body = get(&page_app(), PAGE).await.text();
    let s = kpi(&body, "limit-status");
    assert!(s.contains("Over limit"), "{s}");
    assert!(s.contains("32.1"), "points over the 25% default limit: {s}");
}

#[tokio::test]
async fn test_page_when_top1_below_limit_should_say_within_limit() {
    let body = get(&page_app(), &format!("{PAGE}?limit=60")).await.text();
    let s = kpi(&body, "limit-status");
    assert!(s.contains("Within limit"), "{s}");
    assert!(!s.contains("Over limit"), "{s}");
}

#[tokio::test]
async fn test_page_when_ticker_has_no_candles_should_mark_it_at_cost() {
    let body = get(&page_app(), PAGE).await.text();
    let rows: Vec<String> = open_tags(&body, "tr")
        .into_iter()
        .filter(|t| attr(t, "data-ticker").is_some())
        .map(|t| {
            format!(
                "{}|{}",
                attr(&t, "data-ticker").unwrap(),
                inner_text(&body, &t)
            )
        })
        .collect();
    assert_eq!(rows.len(), 3, "{rows:?}");
    let by = |t: &str| {
        rows.iter()
            .find(|r| r.starts_with(&format!("{t}|")))
            .unwrap()
            .clone()
    };
    assert!(by("ABC").contains("at cost"), "{rows:?}");
    assert!(!by("AMZN").contains("at cost"), "{rows:?}");
    assert!(!by("VTI").contains("at cost"), "{rows:?}");
    assert_eq!(
        kpi(&body, "at-cost-count").split_whitespace().last(),
        Some("1")
    );
}

#[tokio::test]
async fn test_page_positions_table_should_have_caption_and_column_scopes() {
    let body = get(&page_app(), PAGE).await.text();
    assert!(body.contains("<caption"), "positions table needs a caption");
    let ths = open_tags(&body, "th");
    assert!(!ths.is_empty());
    for th in ths {
        assert_eq!(attr(&th, "scope").as_deref(), Some("col"), "{th}");
    }
}

#[tokio::test]
async fn test_page_donut_should_be_an_accessible_image_with_title() {
    let body = get(&page_app(), PAGE).await.text();
    let donut = svgs(&body)
        .into_iter()
        .next()
        .expect("donut svg on the page");
    assert!(donut.contains("role=\"img\""), "{donut}");
    assert!(donut.contains("<title"), "{donut}");
}

#[tokio::test]
async fn test_page_should_not_contain_inline_hex_colours() {
    let r = get(&page_app(), PAGE).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(hex_literals(&r.text()), Vec::<String>::new());
}

#[tokio::test]
async fn test_page_when_no_lots_should_show_empty_state_with_import_and_setup_hints() {
    let app = app_with_edgar(|_| {}, None);
    let r = get(&app, PAGE).await;
    assert_eq!(r.status, StatusCode::OK);
    let body = r.text();
    assert!(body.contains("empty-state"), "{body}");
    assert!(body.contains("ghost-alpha portfolio import"));
    assert!(body.contains("setup --section portfolio"));
    assert_eq!(count(&body, "<h1"), 1);
}

#[tokio::test]
async fn test_page_when_lots_without_any_edgar_should_still_render() {
    // The page itself never needs EDGAR; only the lazy partial does.
    let r = get(&page_app(), PAGE).await;
    assert_eq!(r.status, StatusCode::OK);
}

// ═════════════════════════════ page: lazy overlap region ═════════════════════════════

#[tokio::test]
async fn test_page_overlap_placeholder_when_no_params_should_carry_defaults() {
    let body = get(&page_app(), PAGE).await.text();
    let (p, raw, params) = placeholder(&body);
    assert_eq!(p, PARTIAL);
    assert_eq!(param(&params, "symbol"), "AMZN");
    assert_eq!(param(&params, "limit"), "25");
    assert_eq!(param(&params, "candidates"), "VTI,VXUS,QQQ,SCHD");
    assert_eq!(param(&params, "buy"), "50000");
    assert!(
        !raw.replace("&amp;", "").contains('&'),
        "query separators must be HTML-escaped: {raw}"
    );
}

#[tokio::test]
async fn test_page_overlap_placeholder_when_params_given_should_carry_normalised_query() {
    let body = get(
        &page_app(),
        &format!("{PAGE}?symbol=amzn&limit=30&candidates=vxus,qqq&buy=12000"),
    )
    .await
    .text();
    let (_, raw, params) = placeholder(&body);
    assert_eq!(param(&params, "symbol"), "AMZN");
    assert_eq!(param(&params, "limit"), "30");
    assert_eq!(param(&params, "candidates"), "VXUS,QQQ");
    assert_eq!(param(&params, "buy"), "12000");
    assert!(!raw.replace("&amp;", "").contains('&'), "{raw}");
}

#[tokio::test]
async fn test_page_should_offer_noscript_fallback_link_to_the_partial() {
    let body = get(&page_app(), PAGE).await.text();
    let s = body.find("<noscript").expect("<noscript> fallback");
    let e = body[s..].find("</noscript>").unwrap() + s;
    let link = open_tags(&body[s..e], "a")
        .into_iter()
        .next()
        .expect("link inside noscript");
    assert!(
        attr(&link, "href")
            .unwrap()
            .starts_with("/partials/etf-overlap?"),
        "{link}"
    );
}

// ═════════════════════════════ page: input validation ═════════════════════════════

#[tokio::test]
async fn test_page_when_params_invalid_should_return_400_without_reflecting_input() {
    let cases = [
        "symbol=%3Cscript%3Ealert(1)%3C/script%3E",
        "symbol=AM%20ZN",
        "symbol=AMZN%22%3E%3Cb%3E",
        "candidates=VTI,%3Cscript%3Ealert(1)%3C/script%3E",
        "candidates=VTI;DROP",
        "candidates=VT%20I",
        "candidates=A,B,C,D,E,F,G,H,I,J,K",
        "buy=0",
        "buy=-5",
        "buy=abc",
        "limit=0",
        "limit=-1",
        "limit=abc",
        "limit=101",
    ];
    for q in cases {
        let r = get(&page_app(), &format!("{PAGE}?{q}")).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{q}");
        let body = r.text();
        assert!(!body.contains("<script>alert"), "{q}: {body}");
        assert!(!body.contains("<b>"), "{q}: {body}");
    }
}

#[tokio::test]
async fn test_page_when_exactly_ten_candidates_should_be_accepted() {
    let r = get(
        &page_app(),
        &format!("{PAGE}?candidates=A1,B2,C3,D4,E5,F6,G7,H8,I9,J.K"),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
}

// ═════════════════════════════ partial: happy path ═════════════════════════════

#[tokio::test]
async fn test_partial_when_edgar_has_filings_should_render_one_row_per_held_fund_and_candidate() {
    let sec = Sec::start(&[]).await;
    let r = get(&sec.app(), &format!("{PARTIAL}?candidates=VXUS,QQQ,SPY")).await;
    assert_eq!(r.status, StatusCode::OK);
    let mut rows = etf_rows(&r.text());
    rows.sort();
    // VTI is held (and a fund); AMZN and ABC are held but not funds, so they get no row.
    assert_eq!(rows, ["QQQ", "SPY", "VTI", "VXUS"]);
}

#[tokio::test]
async fn test_partial_when_etf_listed_should_show_amzn_equity_pct_from_fixture() {
    let sec = Sec::start(&[]).await;
    let body = get(&sec.app(), &format!("{PARTIAL}?candidates=VXUS,QQQ,SPY"))
        .await
        .text();
    assert_eq!(
        cell(&etf_row(&body, "VTI").unwrap(), "pct").unwrap(),
        "3.19%"
    );
    assert_eq!(
        cell(&etf_row(&body, "QQQ").unwrap(), "pct").unwrap(),
        "4.50%"
    );
    assert_eq!(
        cell(&etf_row(&body, "VXUS").unwrap(), "pct").unwrap(),
        "0.00%"
    );
}

#[tokio::test]
async fn test_partial_when_etf_listed_should_show_holdings_as_of_and_held_value() {
    let sec = Sec::start(&[]).await;
    let body = get(&sec.app(), &format!("{PARTIAL}?candidates=VXUS,QQQ,SPY"))
        .await
        .text();
    let vti = etf_row(&body, "VTI").unwrap();
    assert_eq!(cell(&vti, "as-of").unwrap(), "2026-06-30");
    assert!(cell(&vti, "held").unwrap().contains("10,000"), "{vti}");
    let vxus = etf_row(&body, "VXUS").unwrap();
    assert_eq!(cell(&vxus, "as-of").unwrap(), "2026-07-31");
    assert!(
        !cell(&vxus, "held").unwrap_or_default().contains('$'),
        "{vxus}"
    );
}

#[tokio::test]
async fn test_partial_look_through_should_add_held_etf_value_times_pct() {
    // direct 20,000 + 10,000 * 3.19% = 20,319 of 35,000 = 58.05%
    let sec = Sec::start(&[]).await;
    let body = get(&sec.app(), &format!("{PARTIAL}?candidates=VXUS,QQQ,SPY"))
        .await
        .text();
    let k = kpi(&body, "look-through");
    assert!(k.contains(&pct1(20_319.0 / 35_000.0)), "{k}");
    assert!(k.contains("57.1%"), "direct weight stays visible: {k}");
    assert!(
        tag_with(&body, "data-badge", "partial").is_none(),
        "nothing failed"
    );
}

#[tokio::test]
async fn test_partial_after_buy_should_use_look_through_exposure_as_base() {
    // base 20,319; total 35,000; buy 50,000 => denominator 85,000.
    let sec = Sec::start(&[]).await;
    let body = get(&sec.app(), &format!("{PARTIAL}?candidates=VXUS,QQQ,SPY"))
        .await
        .text();
    let after = |t: &str| cell(&etf_row(&body, t).unwrap(), "after").unwrap();
    assert_eq!(after("VXUS"), pct1(20_319.0 / 85_000.0)); // 23.9%
    assert_eq!(
        after("VTI"),
        pct1((20_319.0 + 50_000.0 * 0.0319) / 85_000.0)
    ); // 25.8%
    assert_eq!(after("QQQ"), pct1((20_319.0 + 50_000.0 * 0.045) / 85_000.0)); // 26.6%
}

#[tokio::test]
async fn test_partial_when_buy_param_given_should_recompute_after_weights() {
    let sec = Sec::start(&[]).await;
    let body = get(&sec.app(), &format!("{PARTIAL}?candidates=VXUS&buy=100000"))
        .await
        .text();
    let a = cell(&etf_row(&body, "VXUS").unwrap(), "after").unwrap();
    assert_eq!(a, pct1(20_319.0 / 135_000.0)); // 15.1%
}

#[tokio::test]
async fn test_partial_should_star_exactly_the_row_with_the_lowest_after_weight() {
    let sec = Sec::start(&[]).await;
    let body = get(&sec.app(), &format!("{PARTIAL}?candidates=VXUS,QQQ,SPY"))
        .await
        .text();
    let best: Vec<String> = open_tags(&body, "tr")
        .into_iter()
        .filter(|t| attr(t, "data-best").as_deref() == Some("true"))
        .filter_map(|t| attr(&t, "data-etf"))
        .collect();
    assert_eq!(best, ["VXUS"]);
    assert!(strip_tags(&etf_row(&body, "VXUS").unwrap()).contains('★'));
    assert!(!strip_tags(&etf_row(&body, "QQQ").unwrap()).contains('★'));
}

#[tokio::test]
async fn test_partial_when_candidate_is_also_held_should_not_duplicate_the_row() {
    let sec = Sec::start(&[]).await;
    let body = get(&sec.app(), &format!("{PARTIAL}?candidates=VTI,VXUS"))
        .await
        .text();
    let mut rows = etf_rows(&body);
    rows.sort();
    assert_eq!(rows, ["VTI", "VXUS"]);
}

#[tokio::test]
async fn test_partial_when_candidates_lowercase_should_normalise_to_upper_case() {
    let sec = Sec::start(&[]).await;
    let body = get(&sec.app(), &format!("{PARTIAL}?candidates=vxus,qqq"))
        .await
        .text();
    assert!(etf_row(&body, "VXUS").is_some() && etf_row(&body, "QQQ").is_some());
}

// ═════════════════════════════ partial: failures ═════════════════════════════

#[tokio::test]
async fn test_partial_when_candidate_is_not_an_nport_filer_should_give_status_row_and_keep_others()
{
    let sec = Sec::start(&[]).await;
    let r = get(&sec.app(), &format!("{PARTIAL}?candidates=SPY,VXUS")).await;
    assert_eq!(r.status, StatusCode::OK);
    let body = r.text();
    let spy = etf_row(&body, "SPY").expect("SPY still gets a row");
    let status = cell(&spy, "status").expect("status cell");
    assert!(status.contains("N-PORT"), "{status}");
    assert!(
        !strip_tags(&spy).contains('%'),
        "no numbers for an unevaluated ETF: {spy}"
    );
    assert!(!spy.contains("data-best"), "{spy}");
    assert_eq!(
        cell(&etf_row(&body, "VXUS").unwrap(), "pct").unwrap(),
        "0.00%"
    );
}

#[tokio::test]
async fn test_partial_when_unknown_candidate_should_give_status_row_not_fail_fragment() {
    let sec = Sec::start(&[]).await;
    let r = get(&sec.app(), &format!("{PARTIAL}?candidates=ZZZZ,VXUS")).await;
    assert_eq!(r.status, StatusCode::OK);
    let body = r.text();
    assert!(!cell(&etf_row(&body, "ZZZZ").unwrap(), "status")
        .unwrap()
        .is_empty());
    assert!(etf_row(&body, "VXUS").is_some());
}

#[tokio::test]
async fn test_partial_when_held_fund_fails_should_show_partial_badge_and_use_direct_only() {
    let sec = Sec::start(&["VTI"]).await;
    let r = get(&sec.app(), &format!("{PARTIAL}?candidates=VXUS,QQQ")).await;
    assert_eq!(
        r.status,
        StatusCode::OK,
        "one failing ETF never fails the fragment"
    );
    let body = r.text();
    let badge = tag_with(&body, "data-badge", "partial").expect("partial badge");
    assert!(inner_text(&body, &badge).to_lowercase().contains("partial"));
    let vti = etf_row(&body, "VTI").unwrap();
    assert!(!cell(&vti, "status").unwrap().is_empty(), "{vti}");
    assert!(
        kpi(&body, "look-through").contains("57.1%"),
        "direct only: 20k of 35k"
    );
    assert!(etf_row(&body, "VXUS").is_some() && etf_row(&body, "QQQ").is_some());
}

#[tokio::test]
async fn test_partial_when_candidate_fails_but_held_fund_ok_should_not_show_partial_badge() {
    let sec = Sec::start(&["QQQ"]).await;
    let body = get(&sec.app(), &format!("{PARTIAL}?candidates=VXUS,QQQ"))
        .await
        .text();
    assert!(tag_with(&body, "data-badge", "partial").is_none());
    assert!(!cell(&etf_row(&body, "QQQ").unwrap(), "status")
        .unwrap()
        .is_empty());
    assert!(kpi(&body, "look-through").contains("58.1%"));
}

// ═════════════════════════════ partial: edgar not configured ═════════════════════════════

#[tokio::test]
async fn test_partial_when_edgar_not_configured_should_return_200_with_setup_hint() {
    let r = get(&page_app(), &format!("{PARTIAL}?candidates=VXUS,QQQ")).await;
    assert_eq!(r.status, StatusCode::OK);
    let body = r.text();
    assert!(body.contains("ghost-alpha setup --section edgar"), "{body}");
    assert!(etf_rows(&body).is_empty());
}

// ═════════════════════════════ partial: validation and no-request guarantees ═════════════════════════════

#[tokio::test]
async fn test_partial_when_params_invalid_should_return_400_and_make_no_requests() {
    let sec = Sec::start(&[]).await;
    let cases = [
        "candidates=VTI,%3Cscript%3Ealert(1)%3C/script%3E",
        "candidates=VTI;DROP",
        "candidates=VT%20I",
        "candidates=A,B,C,D,E,F,G,H,I,J,K",
        "buy=0",
        "buy=-5",
        "buy=abc",
        "symbol=%3Cb%3E",
    ];
    for q in cases {
        let r = get(&sec.app(), &format!("{PARTIAL}?{q}")).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{q}");
        let body = r.text();
        assert!(!body.contains("<script>alert"), "{q}: {body}");
        assert!(!body.contains("<b>"), "{q}: {body}");
    }
    assert_eq!(
        sec.request_count().await,
        0,
        "validation happens before any EDGAR request"
    );
}

#[tokio::test]
async fn test_partial_when_portfolio_empty_should_make_no_requests() {
    let sec = Sec::start(&[]).await;
    let app = app_with_edgar(|_| {}, Some(sec.client.clone()));
    let r = get(&app, &format!("{PARTIAL}?candidates=VXUS,QQQ")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(etf_rows(&r.text()).is_empty());
    assert_eq!(sec.request_count().await, 0);
}

// ═════════════════════════════ partial: chart and a11y ═════════════════════════════

#[tokio::test]
async fn test_partial_chart_should_be_accessible_svg_with_table_view_and_no_hex() {
    let sec = Sec::start(&[]).await;
    let body = get(&sec.app(), &format!("{PARTIAL}?candidates=VXUS,QQQ,SPY"))
        .await
        .text();
    let charts = svgs(&body);
    assert!(!charts.is_empty(), "bar chart svg expected");
    for s in &charts {
        assert!(s.contains("role=\"img\""), "{s}");
        assert!(s.contains("<title"), "{s}");
    }
    let d = body.find("<details").expect("<details> table view");
    assert!(body[d..].contains("<table"), "table inside <details>");
    assert_eq!(hex_literals(&body), Vec::<String>::new());
}

// ═════════════════════════════ review round: shared helpers ═════════════════════════════

fn position_rows(body: &str) -> Vec<(String, String)> {
    open_tags(body, "tr")
        .into_iter()
        .filter_map(|t| attr(&t, "data-ticker").map(|k| (k, inner_text(body, &t))))
        .collect()
}

fn row_text(body: &str, ticker: &str) -> String {
    position_rows(body)
        .into_iter()
        .find(|(k, _)| k == ticker)
        .unwrap_or_else(|| panic!("no position row for {ticker}"))
        .1
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn fund(n: u32) -> &'static Etf {
    let cik = 1_100_000 + n;
    Box::leak(Box::new(Etf {
        ticker: leak(format!("FND{n:02}")),
        series: leak(format!("S00091{n:04}")),
        cik,
        acc: leak(format!("{cik:010}-26-{n:06}")),
        pct: "1.0",
        period: "2026-07-31",
        filed: "2026-09-26",
    }))
}

// ═════════════════════════════ M3: URL safety and derived symbol ═════════════════════════════

#[tokio::test]
async fn test_page_overlap_urls_should_percent_encode_values_before_html_escaping() {
    let body = get(&page_app(), &format!("{PAGE}?candidates=VXUS,QQQ"))
        .await
        .text();
    let (_, raw, _) = placeholder(&body);
    assert!(raw.contains("candidates=VXUS%2CQQQ"), "{raw}");
    assert!(!raw.contains("VXUS,QQQ"), "{raw}");
    let s = body.find("<noscript").unwrap();
    let e = body[s..].find("</noscript>").unwrap() + s;
    let link = open_tags(&body[s..e], "a").into_iter().next().unwrap();
    let href = attr(&link, "href").unwrap();
    assert!(href.contains("candidates=VXUS%2CQQQ"), "{href}");
    assert!(!href.replace("&amp;", "").contains('&'), "{href}");
}

#[tokio::test]
async fn test_page_when_symbol_has_dot_should_keep_it_in_the_overlap_query() {
    let app = app_with_edgar(
        |r| {
            seed_portfolio(r);
            insert_lot(r, "lot-b", "BRK.B", 1000.0, 1000.0);
        },
        None,
    );
    let body = get(&app, PAGE).await.text();
    let (_, _, params) = placeholder(&body);
    assert_eq!(param(&params, "symbol"), "BRK.B");
}

#[tokio::test]
async fn test_page_when_default_symbol_invalid_should_render_without_hx_get_and_explain() {
    for bad in ["BRK/B", "SPAXX**", "<i>Q</i>"] {
        let app = app_with_edgar(
            |r| {
                seed_portfolio(r);
                insert_lot(r, "lot-bad", bad, 1000.0, 1000.0);
            },
            None,
        );
        let r = get(&app, PAGE).await;
        assert_eq!(r.status, StatusCode::OK, "{bad}");
        let body = r.text();
        let tag = open_tags(&body, "div")
            .into_iter()
            .find(|t| attr(t, "id").as_deref() == Some("etf-overlap"))
            .unwrap_or_else(|| panic!("{bad}: #etf-overlap region must still exist"));
        assert!(attr(&tag, "hx-get").is_none(), "{bad}: {tag}");
        assert!(attr(&tag, "hx-trigger").is_none(), "{bad}: {tag}");
        assert!(
            body.contains("?symbol="),
            "{bad}: message must tell the user to pass ?symbol="
        );
        assert!(
            !body.contains("/partials/etf-overlap"),
            "{bad}: no request URL may be emitted"
        );
        assert!(
            !body.contains("<i>Q</i>"),
            "{bad}: raw ticker echoed unescaped"
        );
    }
}

#[tokio::test]
async fn test_page_when_default_symbol_invalid_but_symbol_param_given_should_load_overlap() {
    let app = app_with_edgar(
        |r| {
            seed_portfolio(r);
            insert_lot(r, "lot-bad", "BRK/B", 1000.0, 1000.0);
        },
        None,
    );
    let body = get(&app, &format!("{PAGE}?symbol=AMZN")).await.text();
    let (_, _, params) = placeholder(&body);
    assert_eq!(param(&params, "symbol"), "AMZN");
}

#[tokio::test]
async fn test_partial_400_body_should_be_wrapped_in_the_overlap_region() {
    let r = get(&page_app(), &format!("{PARTIAL}?buy=0")).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let body = r.text();
    assert!(
        open_tags(&body, "div")
            .iter()
            .any(|t| attr(t, "id").as_deref() == Some("etf-overlap")),
        "{body}"
    );
    assert!(body.contains("role=\"alert\""), "{body}");
}

#[tokio::test]
async fn test_static_js_should_let_htmx_swap_4xx_responses_for_the_overlap_region() {
    let body = get(&page_app(), PAGE).await.text();
    let mut found = false;
    for url in static_urls(&body) {
        let name = url
            .trim_start_matches("/static/")
            .split('?')
            .next()
            .unwrap()
            .to_string();
        if !name.ends_with(".js") || name.contains("htmx") {
            continue;
        }
        let f = ghost_alpha::web::assets::StaticAssets::get(&name).unwrap();
        let js = String::from_utf8_lossy(&f.data).into_owned();
        if js.contains("htmx:beforeSwap") && js.contains("shouldSwap") && js.contains("etf-overlap")
        {
            found = true;
        }
    }
    assert!(
        found,
        "a script linked from the page must handle htmx:beforeSwap for etf-overlap"
    );
}

// ═════════════════════════════ M4: fund lookup errors ═════════════════════════════

#[tokio::test]
async fn test_partial_when_fund_list_unavailable_should_flag_partial_and_use_direct_only() {
    for code in [500u16, 403] {
        let sec = Sec::start_with(SecOpts {
            mf_status: Some(code),
            ..Default::default()
        })
        .await;
        let r = get(&sec.app(), &format!("{PARTIAL}?candidates=VXUS")).await;
        assert_eq!(r.status, StatusCode::OK, "{code}");
        let body = r.text();
        let low = body.to_lowercase();
        assert!(
            tag_with(&body, "data-badge", "partial").is_some(),
            "{code}: {body}"
        );
        assert!(
            low.contains("fund list") && low.contains("unavailable"),
            "{code}: {body}"
        );
        assert!(kpi(&body, "look-through").contains("57.1%"), "{code}");
    }
}

// ═════════════════════════════ M2: bounded work ═════════════════════════════

#[test]
fn test_limits_should_expose_cap_and_default_timeout() {
    use ghost_alpha::web::concentration::{ETF_TIMEOUT, MAX_HELD_FUNDS};
    assert_eq!(MAX_HELD_FUNDS, 15);
    assert!(
        ETF_TIMEOUT >= std::time::Duration::from_secs(5)
            && ETF_TIMEOUT <= std::time::Duration::from_secs(60),
        "{ETF_TIMEOUT:?}"
    );
}

#[tokio::test]
async fn test_partial_when_more_than_15_held_funds_should_evaluate_largest_and_skip_rest() {
    let funds: Vec<&'static Etf> = (1..=17).map(fund).collect();
    let sec = Sec::start_with(SecOpts {
        extra: funds.clone(),
        ..Default::default()
    })
    .await;
    let seed = |r: &SqliteRepository| {
        insert_lot(r, "lot-a", "AMZN", 100.0, 200.0);
        for n in 1..=17u32 {
            insert_lot(
                r,
                &format!("lot-f{n}"),
                &format!("FND{n:02}"),
                10.0,
                n as f64 * 10.0,
            );
        }
    };
    let app = app_with_edgar(seed, Some(sec.client.clone()));
    let r = get(&app, &format!("{PARTIAL}?candidates=FND17")).await;
    assert_eq!(r.status, StatusCode::OK);
    let body = r.text();
    assert!(
        tag_with(&body, "data-badge", "partial").is_some(),
        "skipped funds count as partial"
    );
    // Smallest two held funds are skipped, with a note.
    for n in [1u32, 2] {
        let row = etf_row(&body, &format!("FND{n:02}")).expect("skipped fund keeps a row");
        assert!(
            cell(&row, "status")
                .unwrap()
                .to_lowercase()
                .contains("skipped"),
            "{row}"
        );
        assert_eq!(
            sec.hits(&format!("CIK={}", funds[n as usize - 1].series))
                .await,
            0
        );
    }
    // The other 15 are evaluated: pct filled in.
    for n in 3..=17u32 {
        let row = etf_row(&body, &format!("FND{n:02}")).unwrap();
        assert_eq!(cell(&row, "pct").unwrap(), "1.00%", "{row}");
    }
    assert_eq!(sec.hits("/cgi-bin/browse-edgar").await, 15);
    assert_eq!(
        sec.hits("company_tickers_mf.json").await,
        1,
        "fund map loaded once per request"
    );
}

#[tokio::test]
async fn test_partial_when_held_fund_times_out_should_say_timed_out_and_flag_partial() {
    let sec = Sec::start_with(SecOpts {
        delays: vec![("VTI", 3000)],
        ..Default::default()
    })
    .await;
    let app = app_with_edgar_timeout(
        seed_portfolio,
        Some(sec.client.clone()),
        Some(std::time::Duration::from_millis(300)),
    );
    let started = std::time::Instant::now();
    let r = get(&app, &format!("{PARTIAL}?candidates=VXUS")).await;
    assert!(
        started.elapsed() < std::time::Duration::from_millis(2500),
        "timeout not applied"
    );
    assert_eq!(r.status, StatusCode::OK);
    let body = r.text();
    let vti = etf_row(&body, "VTI").unwrap();
    assert!(cell(&vti, "status").unwrap().contains("timed out"), "{vti}");
    assert!(tag_with(&body, "data-badge", "partial").is_some());
    assert!(kpi(&body, "look-through").contains("57.1%"));
    assert_eq!(
        cell(&etf_row(&body, "VXUS").unwrap(), "pct").unwrap(),
        "0.00%"
    );
}

// ═════════════════════════════ M1: one map load per request ═════════════════════════════

#[tokio::test]
async fn test_partial_when_many_held_tickers_should_load_ticker_maps_once() {
    let sec = Sec::start(&[]).await;
    let app = app_with_edgar(
        |r| {
            seed_portfolio(r);
            for i in 1..=6 {
                insert_lot(r, &format!("lot-x{i}"), &format!("XQ{i}"), 10.0, 10.0);
            }
        },
        Some(sec.client.clone()),
    );
    let r = get(&app, &format!("{PARTIAL}?candidates=VXUS")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(sec.hits("company_tickers_mf.json").await, 1);
    assert_eq!(sec.hits("company_tickers.json").await, 1);
}

// ═════════════════════════════ m2: NoNportFiling is a note, not partial ═════════════════════════════

#[tokio::test]
async fn test_partial_when_held_fund_has_no_nport_filing_should_note_but_not_flag_partial() {
    let sec = Sec::start(&[]).await;
    let app = app_with_edgar(
        |r| {
            seed_portfolio(r);
            insert_lot(r, "lot-m", "MMF", 100.0, 1.0);
        },
        Some(sec.client.clone()),
    );
    let body = get(&app, &format!("{PARTIAL}?candidates=VXUS"))
        .await
        .text();
    let mmf = etf_row(&body, "MMF").expect("held fund keeps a row");
    assert!(cell(&mmf, "status").unwrap().contains("N-PORT"), "{mmf}");
    assert!(tag_with(&body, "data-badge", "partial").is_none(), "{body}");
    assert!(kpi(&body, "look-through").contains(&pct1(20_319.0 / 35_100.0)));
}

// ═════════════════════════════ m1: donut label escaping ═════════════════════════════

#[tokio::test]
async fn test_page_donut_should_escape_html_in_ticker_labels() {
    let app = app_with_edgar(
        |r| {
            seed_portfolio(r);
            insert_lot(r, "lot-h", "<b>x</b>", 1.0, 1.0);
        },
        None,
    );
    let body = get(&app, PAGE).await.text();
    for s in svgs(&body) {
        assert!(!s.contains("<b>"), "raw markup inside svg: {s}");
    }
    assert!(!body.contains("<b>x</b>"));
}

// ═════════════════════════════ m3: stale prices ═════════════════════════════

fn stale_app(vti_close_date: &str) -> Router {
    let d = vti_close_date.to_string();
    app_with_edgar(
        move |r| {
            insert_lot(r, "lot-1", "AMZN", 100.0, 100.0);
            insert_lot(r, "lot-3", "VTI", 100.0, 80.0);
            insert_lot(r, "lot-4", "ABC", 100.0, 50.0);
            insert_close(r, "AMZN", "2026-10-24", 200.0);
            insert_close(r, "VTI", &d, 100.0);
        },
        None,
    )
}

#[tokio::test]
async fn test_page_when_close_older_than_7_days_than_newest_should_mark_row_stale() {
    let body = get(&stale_app("2026-10-10"), PAGE).await.text();
    assert!(row_text(&body, "VTI").contains("stale"));
    assert!(!row_text(&body, "AMZN").contains("stale"));
    assert!(
        !row_text(&body, "ABC").contains("stale"),
        "at-cost rows are not stale"
    );
}

#[tokio::test]
async fn test_page_when_close_exactly_7_days_older_should_not_be_stale() {
    let body = get(&stale_app("2026-10-17"), PAGE).await.text();
    assert!(!row_text(&body, "VTI").contains("stale"));
}

// ═════════════════════════════ m4: DB errors ═════════════════════════════

fn broken_db_app() -> Router {
    app_with_edgar(
        |r| {
            r.connection()
                .unwrap()
                .execute_batch("DROP TABLE tax_lots;")
                .unwrap();
        },
        None,
    )
}

#[tokio::test]
async fn test_page_when_db_fails_should_show_error_state_not_empty_portfolio() {
    let r = get(&broken_db_app(), PAGE).await;
    assert_eq!(r.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body = r.text();
    assert!(body.contains("role=\"alert\""), "{body}");
    assert!(!body.contains("empty-state"), "{body}");
    assert!(!body.contains("ghost-alpha portfolio import"), "{body}");
}

#[tokio::test]
async fn test_partial_when_db_fails_should_return_wrapped_error_not_empty_state() {
    let r = get(&broken_db_app(), &format!("{PARTIAL}?candidates=VXUS")).await;
    assert_eq!(r.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body = r.text();
    assert!(
        open_tags(&body, "div")
            .iter()
            .any(|t| attr(t, "id").as_deref() == Some("etf-overlap")),
        "{body}"
    );
    assert!(
        body.contains("role=\"alert\"") && !body.contains("empty-state"),
        "{body}"
    );
}

// ═════════════════════════════ m5: unpriceable lots ═════════════════════════════

#[tokio::test]
async fn test_page_when_lot_has_non_positive_cost_and_no_close_should_show_no_price_and_exclude() {
    let app = app_with_edgar(
        |r| {
            seed_portfolio(r);
            insert_lot(r, "lot-z", "ZERO", 100.0, 0.0);
            insert_lot(r, "lot-n", "NEG", 100.0, -5.0);
        },
        None,
    );
    let body = get(&app, PAGE).await.text();
    assert!(row_text(&body, "ZERO").contains("no price"));
    assert!(row_text(&body, "NEG").contains("no price"));
    // Totals and weights are unchanged from the seeded 35,000 portfolio.
    assert!(kpi(&body, "direct-weight").contains("57.1%"));
    assert!(kpi(&body, "effective-n").contains("2.3"));
    assert_eq!(
        kpi(&body, "at-cost-count").split_whitespace().last(),
        Some("1")
    );
}

// ═════════════════════════════ m6: buy upper bound ═════════════════════════════

#[tokio::test]
async fn test_buy_above_one_trillion_should_be_rejected_on_page_and_partial() {
    for q in ["buy=1000000000001", "buy=1e13"] {
        let r = get(&page_app(), &format!("{PAGE}?{q}")).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "page {q}");
        let r = get(&page_app(), &format!("{PARTIAL}?{q}")).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "partial {q}");
    }
    let r = get(&page_app(), &format!("{PAGE}?buy=1000000000000")).await;
    assert_eq!(r.status, StatusCode::OK, "exactly 1e12 is allowed");
}

// ═════════════════════════════ m7 + nits ═════════════════════════════

#[tokio::test]
async fn test_page_should_keep_a_persistent_live_region_around_the_swapped_overlap() {
    let body = get(&page_app(), PAGE).await.text();
    assert_live_ancestor(&body);
}

#[tokio::test]
async fn test_partial_star_should_be_aria_hidden_with_screen_reader_text() {
    let sec = Sec::start(&[]).await;
    let body = get(&sec.app(), &format!("{PARTIAL}?candidates=VXUS,QQQ"))
        .await
        .text();
    let row = etf_row(&body, "VXUS").unwrap();
    assert!(row.contains("<span aria-hidden=\"true\">★</span>"), "{row}");
    assert!(row.contains("sr-only"), "{row}");
}

#[tokio::test]
async fn test_page_limit_should_display_at_most_one_decimal() {
    let body = get(&page_app(), &format!("{PAGE}?limit=33.333333"))
        .await
        .text();
    let s = kpi(&body, "limit-status");
    assert!(s.contains("33.3%") && !s.contains("33.33"), "{s}");
    assert!(s.contains("23.8"), "{s}");
}

#[tokio::test]
async fn test_page_effective_positions_label_should_say_direct_holdings() {
    let body = get(&page_app(), PAGE).await.text();
    assert!(kpi(&body, "effective-n").contains("(direct holdings)"));
}
