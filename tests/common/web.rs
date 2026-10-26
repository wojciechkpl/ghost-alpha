//! Helpers for in-process web dashboard tests (no network, in-memory SQLite).
//!
//! Contract relied upon: `ghost_alpha::web::{create_router, AppState}` with
//! public `repo` / `config` fields (already the case today).

use std::sync::Arc;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use ghost_alpha::config::AppConfig;
use ghost_alpha::storage::db::SqliteRepository;
use ghost_alpha::web::{create_router, AppState};
use tower::ServiceExt;

pub struct Resp {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers
            .get(name)
            .map(|v| v.to_str().unwrap().to_string())
    }
}

pub fn app() -> Router {
    let repo = SqliteRepository::open_in_memory().expect("in-memory db");
    let config: AppConfig = toml::from_str("").expect("empty config must deserialize");
    create_router(AppState {
        repo: Arc::new(repo),
        config: Arc::new(config),
        edgar: None,
        etf_timeout: None,
    })
}

pub async fn get_with(app: &Router, path: &str, headers: &[(&str, &str)]) -> Resp {
    let mut req = Request::get(path);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = app
        .clone()
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap()
        .to_vec();
    Resp {
        status,
        headers,
        body,
    }
}

pub async fn get(app: &Router, path: &str) -> Resp {
    get_with(app, path, &[]).await
}

/// All `.route("<path>", ...get(...))` registrations in `src/web/mod.rs`,
/// excluding `/static/*` and `/partials/*`. These are the full-page routes.
/// (Source-parsed on purpose: the router exposes no route table.)
pub fn registered_routes() -> Vec<(String, String)> {
    let src =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/web/mod.rs")).unwrap();
    // Only the non-test part of the file.
    let src = src.split("#[cfg(test)]").next().unwrap().to_string();
    let mut out = Vec::new();
    let mut rest = src.as_str();
    while let Some(i) = rest.find(".route(") {
        rest = &rest[i + ".route(".len()..];
        let rest_trim = rest.trim_start();
        let Some(stripped) = rest_trim.strip_prefix('"') else {
            continue;
        };
        let end = stripped.find('"').unwrap();
        let path = stripped[..end].to_string();
        let after = &stripped[end + 1..];
        let after = after.trim_start().trim_start_matches(',').trim_start();
        let method = if after.starts_with("axum::routing::post(") || after.starts_with("post(") {
            "POST"
        } else {
            "GET"
        };
        out.push((method.to_string(), path));
    }
    out
}

pub fn page_routes() -> Vec<String> {
    registered_routes()
        .into_iter()
        .filter(|(m, p)| m == "GET" && !p.starts_with("/static") && !p.starts_with("/partials"))
        .map(|(_, p)| p)
        .collect()
}

pub fn partial_routes() -> Vec<String> {
    registered_routes()
        .into_iter()
        .filter(|(m, p)| m == "GET" && p.starts_with("/partials"))
        .map(|(_, p)| p)
        .collect()
}

/// Count non-overlapping occurrences of `needle`.
pub fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

/// Return every opening tag (`<name ...>`) for the given tag name.
pub fn open_tags(html: &str, name: &str) -> Vec<String> {
    let pat = format!("<{name}");
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find(&pat) {
        let after = &rest[i + pat.len()..];
        let boundary = after
            .chars()
            .next()
            .map(|c| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(false);
        if boundary {
            let end = after.find('>').map(|e| e + 1).unwrap_or(after.len());
            out.push(format!("{pat}{}", &after[..end]));
        }
        rest = after;
    }
    out
}

/// Extract the value of attribute `attr` from an opening tag string.
pub fn attr(tag: &str, attr: &str) -> Option<String> {
    for q in ['"', '\''] {
        let pat = format!(" {attr}={q}");
        if let Some(i) = tag.find(&pat) {
            let v = &tag[i + pat.len()..];
            return v.find(q).map(|e| v[..e].to_string());
        }
    }
    None
}

/// Every opening/closing tag string in document order (`<...>`).
pub fn all_tags(html: &str) -> Vec<String> {
    let mut v = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        let a = &rest[i..];
        match a.find('>') {
            Some(e) => {
                v.push(a[..=e].to_string());
                rest = &a[e..];
            }
            None => break,
        }
    }
    v
}

/// Text between `<tag ...>` and `</tag>` for the first occurrence, with nested tags stripped.
pub fn first_text(html: &str, tag: &str) -> Option<String> {
    let s = html.find(&format!("<{tag}"))?;
    let after = &html[s..];
    let open_end = after.find('>')? + 1;
    let close = after.find(&format!("</{tag}>"))?;
    Some(strip_tags(&after[open_end..close]))
}

pub fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut depth = false;
    for c in s.chars() {
        match c {
            '<' => depth = true,
            '>' => depth = false,
            _ if !depth => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `/static/...` URLs referenced via `href=` or `src=` (query string included).
pub fn static_urls(html: &str) -> Vec<String> {
    all_tags(html)
        .iter()
        .flat_map(|t| {
            ["href", "src"]
                .iter()
                .filter_map(|a| attr(t, a))
                .filter(|u| u.starts_with("/static/"))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Lowercase hex SHA-256 of an embedded static asset (name relative to web/static/).
pub fn asset_sha256_hex(name: &str) -> String {
    let f = ghost_alpha::web::assets::StaticAssets::get(name)
        .unwrap_or_else(|| panic!("embedded asset {name} missing"));
    f.metadata
        .sha256_hash()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

// ─────────────────────────── seeded fixtures (batch 2) ───────────────────────────

use chrono::{Datelike, Duration, NaiveDate, Utc, Weekday};
use ghost_alpha::alerts::channel::OutboundAlert;
use ghost_alpha::analysis::indicators::LiquidityClass;
use ghost_alpha::analysis::macro_risk::{MacroRiskLevel, MacroRiskSnapshot};
use ghost_alpha::analysis::scorer::Grade;
use ghost_alpha::analysis::screener::ScreenResult;
use ghost_alpha::data::models::{MarketRegime, RegimeState, Severity};
use ghost_alpha::storage::db::Repository;

pub fn today() -> NaiveDate {
    Utc::now().date_naive()
}

fn is_weekday(d: NaiveDate) -> bool {
    !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

/// Weekdays (Mon-Fri) in the half-open interval `(from, to]`; the rule the dashboard pins
/// for "trading days". Test-owned copy so fixtures do not depend on the implementation.
pub fn weekdays_between(from: NaiveDate, to: NaiveDate) -> u32 {
    let mut n = 0;
    let mut d = from;
    while d < to {
        d += Duration::days(1);
        if is_weekday(d) {
            n += 1;
        }
    }
    n
}

/// A weekday `d <= today` with exactly `age` weekdays in `(d, today]`.
pub fn date_with_age(today: NaiveDate, age: u32) -> NaiveDate {
    let mut d = today;
    loop {
        if is_weekday(d) && weekdays_between(d, today) == age {
            return d;
        }
        d -= Duration::days(1);
        assert!(today - d < Duration::days(60), "no date with age {age}");
    }
}

pub fn screen_result(ticker: &str, score: f64, grade: Grade, regime: MarketRegime) -> ScreenResult {
    ScreenResult {
        ticker: ticker.to_string(),
        minervini_pass: true,
        conditions_met: 7,
        adx: 28.0,
        rs_percentile: 40.0 + score / 2.0,
        volume_zscore: 1.5,
        technical_score: 75.0,
        volume_score: 80.0,
        sentiment_score: 0.3,
        composite_score: score,
        grade,
        close_price: 175.0,
        pct_from_52w_high: -3.2,
        atr_value: 4.5,
        risk_reward: 2.0,
        liquidity: LiquidityClass::High,
        weighted_rs: 0.9,
        rs_momentum: Some(score / 10.0 - 5.0),
        vcp: None,
        regime,
    }
}

/// Seed one screen run per tickers on `date`.
pub fn seed_screen_run(repo: &SqliteRepository, date: NaiveDate) {
    let rows = [
        ("AAPL", 82.0, Grade::A),
        ("MSFT", 71.0, Grade::B),
        ("NVDA", 91.0, Grade::APlus),
    ];
    for (t, s, g) in rows {
        repo.save_screen_result(date, &screen_result(t, s, g, MarketRegime::Bull))
            .unwrap();
    }
}

/// Everything needed for pages and partials to render real tables and charts.
pub fn seed_full(repo: &SqliteRepository, today: NaiveDate) {
    for back in 0..6 {
        let d = today - Duration::days(back);
        let rows = [
            ("AAPL", 82.0 - back as f64, Grade::A),
            ("MSFT", 71.0 + back as f64, Grade::B),
            ("NVDA", 91.0 - 2.0 * back as f64, Grade::APlus),
            ("XOM", 40.0 + back as f64, Grade::D),
        ];
        for (t, s, g) in rows {
            repo.save_screen_result(d, &screen_result(t, s, g, MarketRegime::Bull))
                .unwrap();
        }
    }
    for back in 0..12 {
        let regime = match back / 4 {
            0 => MarketRegime::Bull,
            1 => MarketRegime::Sideways,
            _ => MarketRegime::Bear,
        };
        repo.save_regime(&RegimeState {
            date: today - Duration::days(back),
            spy_close: 500.0 + back as f64 * 2.0,
            spy_sma200: 480.0,
            breadth_pct: 55.0,
            regime,
        })
        .unwrap();
    }
    for back in 0..5 {
        repo.save_macro_risk(
            today - Duration::days(back),
            &MacroRiskSnapshot {
                vix_level: 15.0 + back as f64,
                hy_oas_bps: Some(350.0),
                yield_curve_2s10s: Some(0.4),
                put_call_ratio: Some(0.9),
                risk_level: MacroRiskLevel::Normal,
            },
        )
        .unwrap();
    }
    for (t, sev) in [
        ("AAPL", Severity::Critical),
        ("MSFT", Severity::Warning),
        ("NVDA", Severity::Info),
    ] {
        repo.record_alert(&OutboundAlert {
            ticker: t.to_string(),
            alert_type: "score_drop".to_string(),
            severity: sev,
            message: format!("{t} score changed"),
            composite_score: 70.0,
            narrative: None,
        })
        .unwrap();
    }
    repo.watchlist_add("AAPL", Some("core")).unwrap();
    repo.watchlist_add("NVDA", None).unwrap();
    repo.save_correlation_snapshot(
        &today.to_string(),
        &["AAPL".to_string(), "MSFT".to_string(), "NVDA".to_string()],
        &[
            vec![1.0, 0.6, 0.7],
            vec![0.6, 1.0, 0.5],
            vec![0.7, 0.5, 1.0],
        ],
        90,
    )
    .unwrap();
    let conn = repo.connection().unwrap();
    // Calendar heatmap ignores weekends, so back-date some alerts onto weekdays.
    for (k, sev) in [(1, "CRITICAL"), (2, "WARNING"), (3, "INFO")] {
        let day = date_with_age(today, k);
        conn.execute(
            "INSERT INTO alert_log (ticker, alert_type, severity, message, sent_at) \
             VALUES ('AAPL', ?1, ?2, 'calendar fixture', ?3)",
            rusqlite::params![format!("cal-{k}"), sev, format!("{day} 12:00:00")],
        )
        .unwrap();
    }
    for (id, t, sh, cb) in [
        ("lot-1", "AMZN", 100.0, 120.0),
        ("lot-2", "VTI", 50.0, 200.0),
    ] {
        conn.execute(
            "INSERT INTO tax_lots (id, ticker, shares, cost_basis_per_share, acquired_date, \
             acquisition_type, remaining_shares) VALUES (?1, ?2, ?3, ?4, '2022-01-03', 'BUY', ?3)",
            rusqlite::params![id, t, sh, cb],
        )
        .unwrap();
    }
}

pub fn app_with(seed: impl FnOnce(&SqliteRepository)) -> Router {
    let repo = SqliteRepository::open_in_memory().expect("in-memory db");
    seed(&repo);
    let config: AppConfig = toml::from_str("").unwrap();
    create_router(AppState {
        repo: Arc::new(repo),
        config: Arc::new(config),
        edgar: None,
        etf_timeout: None,
    })
}

/// Router with an optional shared EDGAR client (`AppState.edgar`).
pub fn app_with_edgar(
    seed: impl FnOnce(&SqliteRepository),
    edgar: Option<Arc<ghost_alpha::data::edgar::EdgarClient>>,
) -> Router {
    app_with_edgar_timeout(seed, edgar, None)
}

/// Like [`app_with_edgar`] with a per-ETF evaluation timeout override
/// (`AppState.etf_timeout`; `None` = the production default).
pub fn app_with_edgar_timeout(
    seed: impl FnOnce(&SqliteRepository),
    edgar: Option<Arc<ghost_alpha::data::edgar::EdgarClient>>,
    etf_timeout: Option<std::time::Duration>,
) -> Router {
    let repo = SqliteRepository::open_in_memory().expect("in-memory db");
    seed(&repo);
    let config: AppConfig = toml::from_str("").unwrap();
    create_router(AppState {
        repo: Arc::new(repo),
        config: Arc::new(config),
        edgar,
        etf_timeout,
    })
}

/// Insert one open tax lot (remaining == shares).
pub fn insert_lot(repo: &SqliteRepository, id: &str, ticker: &str, shares: f64, cost: f64) {
    let conn = repo.connection().unwrap();
    conn.execute(
        "INSERT INTO tax_lots (id, ticker, shares, cost_basis_per_share, acquired_date, \
         acquisition_type, remaining_shares) VALUES (?1, ?2, ?3, ?4, '2022-01-03', 'BUY', ?3)",
        rusqlite::params![id, ticker, shares, cost],
    )
    .unwrap();
}

/// Insert a daily candle (close = adj_close = `close`); creates the stock row first.
pub fn insert_close(repo: &SqliteRepository, ticker: &str, date: &str, close: f64) {
    use ghost_alpha::data::models::{Candle, Stock};
    repo.upsert_stock(&Stock {
        ticker: ticker.to_string(),
        name: format!("{ticker} test"),
        sector: "Test".to_string(),
        industry: "Test".to_string(),
    })
    .unwrap();
    repo.upsert_candles(
        ticker,
        &[Candle::new(date, close, close, close, close, 1_000)],
    )
    .unwrap();
}

pub fn seeded_app() -> Router {
    app_with(|r| seed_full(r, today()))
}

/// Full-page routes (from NAV via router parse) and all GET partials.
pub fn all_get_paths() -> Vec<String> {
    let mut v = page_routes();
    v.extend(partial_routes());
    v
}

/// Text of the element opened by `open_tag` (a tag string found in `html`).
pub fn element_text(html: &str, open_tag: &str) -> String {
    let start = html.find(open_tag).expect("tag in html") + open_tag.len();
    let name: String = open_tag[1..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    let end = html[start..]
        .find(&format!("</{name}>"))
        .expect("closing tag")
        + start;
    strip_tags(&html[start..end])
}

/// Every `<svg ...>...</svg>` slice.
pub fn svgs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("<svg") {
        let a = &rest[i..];
        let e = a.find("</svg>").map(|e| e + 6).unwrap_or(a.len());
        out.push(a[..e].to_string());
        rest = &a[e..];
    }
    out
}
