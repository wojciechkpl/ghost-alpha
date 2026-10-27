//! Concentration hub, slice 1: how concentrated is the portfolio once ETF
//! holdings are looked through, and which diversifier ETF lowers the exposure
//! to the concentrated stock the most.
//!
//! * `GET /portfolio/concentration`: full page (donut, KPIs, positions table,
//!   lazily loaded overlap region).
//! * `GET /partials/etf-overlap`: htmx fragment built from SEC N-PORT filings.
//!
//! All query parameters are validated before any database or EDGAR access, and
//! rejected values are never echoed back.

use std::collections::HashMap;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use chrono::NaiveDate;

use crate::data::edgar::{EdgarError, TickerMap};
use crate::data::nport::{exposure, resolve_fund_id, FundId, IssuerMatcher};
use crate::portfolio::lookthrough::{
    look_through, summarize, value_positions, weight_after_buy, ConcentrationSummary, PositionValue,
};
use crate::storage::db::{Repository, TaxLotRow};

use super::charts::{accessible, donut_svg, svg_escape, DonutSlice};
use super::shell::{html_escape, page, table_view};
use super::AppState;

const PAGE_PATH: &str = "/portfolio/concentration";
const PARTIAL_PATH: &str = "/partials/etf-overlap";
const TITLE: &str = "Concentration & Diversification";

const DEFAULT_LIMIT: f64 = 25.0;
const DEFAULT_BUY: f64 = 50_000.0;
const DEFAULT_CANDIDATES: &str = "VTI,VXUS,QQQ,SCHD";
const MAX_CANDIDATES: usize = 10;
const MAX_TICKER_LEN: usize = 12;
const MAX_BUY: f64 = 1e12;
/// Closes older than this many days against the newest close are marked stale.
const STALE_DAYS: i64 = 7;

/// Held funds evaluated per request (the largest by value); the rest are skipped.
pub const MAX_HELD_FUNDS: usize = 15;
/// Default bound on one ETF evaluation (N-PORT feed and document download).
pub const ETF_TIMEOUT: Duration = Duration::from_secs(20);

const SLICE_COLOURS: &[&str] = &[
    "var(--ga-cat-1)",
    "var(--ga-cat-2)",
    "var(--ga-cat-3)",
    "var(--ga-cat-4)",
    "var(--ga-cat-5)",
    "var(--ga-cat-6)",
    "var(--ga-cat-7)",
    "var(--ga-cat-8)",
    "var(--ga-cat-9)",
    "var(--ga-cat-10)",
    "var(--ga-cat-11)",
    "var(--ga-cat-12)",
];

// ─── Query parameters ───────────────────────────────────────────────────────

/// Validated, normalised query parameters shared by both routes.
#[derive(Debug, Clone)]
struct Params {
    symbol: Option<String>,
    limit: f64,
    candidates: Vec<String>,
    buy: f64,
}

pub(super) fn valid_ticker(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_TICKER_LEN
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

/// The optional `symbol` parameter, validated and upper-cased.
pub(super) fn parse_symbol(q: &HashMap<String, String>) -> Result<Option<String>, &'static str> {
    match q
        .get("symbol")
        .map(String::as_str)
        .filter(|v| !v.is_empty())
    {
        None => Ok(None),
        Some(s) if valid_ticker(s) => Ok(Some(s.to_ascii_uppercase())),
        Some(_) => Err("Invalid symbol: use letters, digits, '.' and '-' only."),
    }
}

/// Parse and validate. The error is a static message that never contains input.
fn parse_params(q: &HashMap<String, String>) -> Result<Params, &'static str> {
    let non_empty = |k: &str| q.get(k).map(String::as_str).filter(|v| !v.is_empty());

    let symbol = parse_symbol(q)?;
    let limit = match non_empty("limit") {
        None => DEFAULT_LIMIT,
        Some(s) => match s.parse::<f64>() {
            Ok(v) if v.is_finite() && v > 0.0 && v <= 100.0 => v,
            _ => return Err("Invalid limit: expected a number above 0 and at most 100."),
        },
    };
    let buy = match non_empty("buy") {
        None => DEFAULT_BUY,
        Some(s) => match s.parse::<f64>() {
            Ok(v) if v.is_finite() && v > 0.0 && v <= MAX_BUY => v,
            _ => return Err("Invalid buy amount: expected a number above 0 and at most 1e12."),
        },
    };
    let raw = non_empty("candidates").unwrap_or(DEFAULT_CANDIDATES);
    let parts: Vec<&str> = raw.split(',').collect();
    if parts.len() > MAX_CANDIDATES {
        return Err("Too many candidates: at most 10 tickers.");
    }
    let mut candidates: Vec<String> = Vec::with_capacity(parts.len());
    for p in parts {
        if !valid_ticker(p) {
            return Err("Invalid candidates: tickers use letters, digits, '.' and '-' only.");
        }
        let t = p.to_ascii_uppercase();
        if !candidates.contains(&t) {
            candidates.push(t);
        }
    }
    Ok(Params {
        symbol,
        limit,
        candidates,
        buy,
    })
}

pub(super) fn bad_request(fragment: bool, msg: &str) -> Response {
    let body = if fragment {
        format!(
            r#"<div id="etf-overlap" role="alert"><p class="error">{}</p></div>"#,
            html_escape(msg)
        )
    } else {
        format!(
            r#"<main class="container"><h1>Bad request</h1><p role="alert">{}</p><p><a href="{PAGE_PATH}">Back to concentration</a></p></main>"#,
            html_escape(msg)
        )
    };
    (StatusCode::BAD_REQUEST, Html(body)).into_response()
}

pub(super) const DB_ERROR: &str =
    "The portfolio could not be read from the database. Check the server log and try again.";

fn db_error_fragment() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Html(format!(
            r#"<div id="etf-overlap" role="alert"><p class="error">{DB_ERROR}</p></div>"#
        )),
    )
        .into_response()
}

/// Percent-encode everything except RFC 3986 unreserved characters.
pub(super) fn pct_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Query string carrying the normalised parameters: each value percent-encoded,
/// then HTML-escaped (so `&` separators appear as `&amp;`).
fn query_string(symbol: &str, p: &Params) -> String {
    let enc = |v: &str| html_escape(&pct_encode(v));
    format!(
        "symbol={}&amp;limit={}&amp;candidates={}&amp;buy={}",
        enc(symbol),
        enc(&p.limit.to_string()),
        enc(&p.candidates.join(",")),
        enc(&p.buy.to_string())
    )
}

// ─── Data ───────────────────────────────────────────────────────────────────

pub(super) struct Portfolio {
    /// Positions with a usable price, largest first.
    pub(super) positions: Vec<PositionValue>,
    /// Positions with neither a close nor a positive cost basis (excluded from all maths).
    pub(super) unpriced: Vec<PositionValue>,
    /// All open lots as stored.
    pub(super) lots: Vec<TaxLotRow>,
    lot_count: usize,
    /// Newest close date across the held tickers.
    priced_as_of: Option<NaiveDate>,
    close_dates: HashMap<String, NaiveDate>,
}

/// Lots and the latest close per ticker, read on a blocking thread. `Err` means
/// the database (or its task) failed; callers must not treat that as "empty".
pub(super) async fn load_portfolio(state: &AppState) -> Result<Portfolio, ()> {
    let repo = state.repo.clone();
    let loaded = tokio::task::spawn_blocking(move || {
        let lots: Vec<TaxLotRow> = repo.get_all_tax_lots()?;
        let mut closes: HashMap<String, f64> = HashMap::new();
        let mut dates: HashMap<String, NaiveDate> = HashMap::new();
        let mut as_of: Option<NaiveDate> = None;
        let mut tickers: Vec<&str> = lots.iter().map(|l| l.ticker.as_str()).collect();
        tickers.sort_unstable();
        tickers.dedup();
        for t in tickers {
            if let Some(c) = repo
                .get_candles(t, 1)?
                .pop()
                .filter(|c| c.close.is_finite() && c.close > 0.0)
            {
                closes.insert(t.to_string(), c.close);
                dates.insert(t.to_string(), c.date);
                as_of = as_of.max(Some(c.date));
            }
        }
        Ok::<_, crate::error::StorageError>((lots, closes, dates, as_of))
    })
    .await;
    match loaded {
        Ok(Ok((lots, closes, close_dates, priced_as_of))) => {
            let (unpriced, positions): (Vec<_>, Vec<_>) = value_positions(&lots, &closes)
                .into_iter()
                .partition(|p| p.priced_at_cost && !(p.price.is_finite() && p.price > 0.0));
            Ok(Portfolio {
                positions,
                unpriced,
                lot_count: lots.len(),
                lots,
                priced_as_of,
                close_dates,
            })
        }
        Ok(Err(e)) => {
            tracing::error!("concentration: reading the portfolio failed: {e}");
            Err(())
        }
        Err(e) => {
            tracing::error!("concentration: portfolio load task failed: {e}");
            Err(())
        }
    }
}

// ─── Formatting ─────────────────────────────────────────────────────────────

pub(super) fn fmt_usd(v: f64) -> String {
    let rounded = v.abs().round() as u128;
    let digits = rounded.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 2);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    let sign = if v < 0.0 && rounded > 0 { "-" } else { "" };
    format!("{sign}${out}")
}

pub(super) fn pct1(fraction: f64) -> String {
    format!("{:.1}%", fraction * 100.0)
}

/// At most one decimal ("25", "33.3").
pub(super) fn num1(v: f64) -> String {
    let s = format!("{v:.1}");
    s.strip_suffix(".0").map(str::to_string).unwrap_or(s)
}

pub(super) fn kpi(name: &str, label: &str, value: &str) -> String {
    format!(
        r#"<div class="kpi" data-kpi="{name}"><span class="kpi-label">{label}</span> <strong>{value}</strong></div>"#
    )
}

// ─── Page ───────────────────────────────────────────────────────────────────

pub async fn concentration_page(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let params = match parse_params(&q) {
        Ok(p) => p,
        Err(msg) => return bad_request(false, msg),
    };
    let unwind_params = match super::unwind::parse_unwind_params(&q) {
        Ok(p) => p,
        Err(msg) => return bad_request(false, msg),
    };
    let Ok(pf) = load_portfolio(&state).await else {
        let body = format!(r#"<p role="alert" class="error">{DB_ERROR}</p>"#);
        let html = page(&state, PAGE_PATH, TITLE, &body).await;
        return (StatusCode::INTERNAL_SERVER_ERROR, html).into_response();
    };
    let Some(summary) = summarize(&pf.positions, params.symbol.as_deref()) else {
        let body = if pf.unpriced.is_empty() {
            r#"<p class="empty-state">No portfolio data. Import your lots with <code>ghost-alpha portfolio import</code> or run <code>ghost-alpha setup --section portfolio</code>.</p>"#
        } else {
            r#"<p class="empty-state">None of your lots can be valued: there is no close and no positive cost basis. Fix the lots with <code>ghost-alpha portfolio import</code>.</p>"#
        };
        return page(&state, PAGE_PATH, TITLE, body).await.into_response();
    };
    let title = format!("{TITLE} \u{2014} {}", summary.symbol);
    let body = page_body(&pf, &summary, &params, &unwind_params);
    page(&state, PAGE_PATH, &title, &body).await.into_response()
}

fn page_body(
    pf: &Portfolio,
    s: &ConcentrationSummary,
    p: &Params,
    up: &super::unwind::UnwindParams,
) -> String {
    let at_cost = pf.positions.iter().filter(|x| x.priced_at_cost).count();
    let as_of = pf
        .priced_as_of
        .map_or_else(|| "no prices".to_string(), |d| format!("prices as of {d}"));
    let top1_pct = s.top1.1 * 100.0;
    let limit = num1(p.limit);
    let limit_status = if top1_pct > p.limit {
        format!(
            "\u{26A0} Over limit: +{:.1} pts above the {limit}% top-1 limit",
            top1_pct - p.limit,
        )
    } else {
        format!("Within limit: top-1 is under the {limit}% limit")
    };

    let slices: Vec<DonutSlice> = pf
        .positions
        .iter()
        .enumerate()
        .map(|(i, x)| DonutSlice {
            label: x.ticker.clone(),
            value: x.value,
            color: SLICE_COLOURS[i % SLICE_COLOURS.len()].to_string(),
        })
        .collect();
    let desc = s
        .weights
        .iter()
        .map(|(t, w)| format!("{t} {:.0} percent", w * 100.0))
        .collect::<Vec<_>>()
        .join(", ");
    let donut = accessible(
        &donut_svg(&slices, 300, 300),
        "Portfolio allocation by market value",
        &format!("Donut chart of market value by ticker: {desc}."),
    );

    let mut rows = String::new();
    for (x, (_, w)) in pf.positions.iter().zip(&s.weights) {
        let marker = if x.priced_at_cost {
            r#" <small class="at-cost">at cost</small>"#.to_string()
        } else {
            match (pf.close_dates.get(&x.ticker), pf.priced_as_of) {
                (Some(d), Some(newest)) if (newest - *d).num_days() > STALE_DAYS => {
                    format!(r#" <small class="stale">stale (close {d})</small>"#)
                }
                _ => String::new(),
            }
        };
        rows.push_str(&format!(
            r#"<tr data-ticker="{t}"><td>{t}</td><td>{sh:.2}</td><td>${pr:.2}{marker}</td><td>{val}</td><td>{w}</td></tr>"#,
            t = html_escape(&x.ticker),
            sh = x.shares,
            pr = x.price,
            val = fmt_usd(x.value),
            w = pct1(*w),
        ));
    }
    for x in &pf.unpriced {
        rows.push_str(&format!(
            r#"<tr data-ticker="{t}"><td>{t}</td><td>{sh:.2}</td><td><small class="no-price">no price</small></td><td>&mdash;</td><td>&mdash;</td></tr>"#,
            t = html_escape(&x.ticker),
            sh = x.shares,
        ));
    }
    let unpriced_note = if pf.unpriced.is_empty() {
        String::new()
    } else {
        format!(
            r#"<p><small>{} position(s) have no close and no positive cost basis; they are excluded from totals and weights.</small></p>"#,
            pf.unpriced.len()
        )
    };

    let overlap = if p.symbol.is_none() && !valid_ticker(&s.symbol) {
        // The default (largest) position cannot be used as a request parameter.
        r#"<div id="etf-overlap"><p class="empty-state">Your largest position cannot be looked up automatically. Choose the stock by adding <code>?symbol=TICKER</code> to the address.</p></div>"#.to_string()
    } else {
        let qs = query_string(&s.symbol, p);
        format!(
            r#"<div id="etf-overlap" hx-get="{PARTIAL_PATH}?{qs}" hx-trigger="load" hx-swap="outerHTML" aria-busy="true">
        <p>Loading ETF holdings from SEC N-PORT filings&hellip;</p>
        <noscript><a href="{PARTIAL_PATH}?{qs}">Load the ETF overlap</a></noscript>
      </div>"#
        )
    };

    let kpis = [
        kpi(
            "direct-weight",
            &format!("{} direct", html_escape(&s.symbol)),
            &pct1(s.direct_weight),
        ),
        kpi(
            "top1",
            "Top position",
            &format!("{} {}", html_escape(&s.top1.0), pct1(s.top1.1)),
        ),
        kpi("limit-status", "Top-1 limit", &limit_status),
        kpi(
            "effective-n",
            "Effective positions (direct holdings)",
            &format!("{:.1}", s.effective_n),
        ),
        kpi("at-cost-count", "Priced at cost", &at_cost.to_string()),
    ]
    .join("\n    ");

    format!(
        r#"<p class="muted">Based on: {lots} lots &middot; {as_of}</p>
<div class="grid">
  <section aria-labelledby="today-h">
    <h2 id="today-h">Today</h2>
    <div class="chart-container">{donut}</div>
    {kpis}
    <table><caption>Positions by market value</caption>
      <thead><tr><th scope="col">Ticker</th><th scope="col">Shares</th><th scope="col">Price</th><th scope="col">Value</th><th scope="col">Weight</th></tr></thead>
      <tbody>{rows}</tbody>
    </table>
    {unpriced_note}
  </section>
  <section aria-labelledby="overlap-h">
    <h2 id="overlap-h">Diversifier ETF overlap with {sym}</h2>
    <div class="overlap-live" aria-live="polite">
      {overlap}
    </div>
  </section>
</div>
<section aria-labelledby="unwind-h">
  <h2 id="unwind-h">Unwind schedule and projection for {sym}</h2>
  <div class="unwind-live" aria-live="polite">
    {unwind}
  </div>
</section>
<p class="muted"><small>Equity holdings only; debt and derivatives are listed separately. Not tax or investment advice.</small></p>"#,
        lots = pf.lot_count,
        sym = html_escape(&s.symbol),
        unwind = super::unwind::region_placeholder(&s.symbol, p.symbol.is_none(), up),
    )
}

// ─── Partial ────────────────────────────────────────────────────────────────

/// Fragment that replaces the placeholder (`outerHTML`), clearing `aria-busy`.
/// The persistent `aria-live` ancestor lives on the page.
fn region(inner: &str) -> Html<String> {
    Html(format!(
        r#"<div id="etf-overlap" aria-busy="false">{inner}</div>"#
    ))
}

/// Why an ETF has no figures.
struct Fail {
    msg: String,
    /// Counts as "partial" when the ETF is a held fund.
    partial: bool,
}

struct EtfRow {
    etf: String,
    held_value: Option<f64>,
    result: Result<EtfOk, Fail>,
}

struct EtfOk {
    period_end: NaiveDate,
    /// Equity exposure as a fraction of net assets.
    pct: f64,
    note: Option<String>,
}

/// What to do for one row.
enum Plan {
    Evaluate(FundId),
    Done(Fail),
}

async fn evaluate_etf(
    client: &crate::data::edgar::EdgarClient,
    etf: String,
    held_value: Option<f64>,
    plan: Plan,
    matcher: &IssuerMatcher,
    today: NaiveDate,
    timeout: Duration,
) -> EtfRow {
    let result = match plan {
        Plan::Done(f) => Err(f),
        Plan::Evaluate(id) => {
            let work = async {
                let (_filing, report) = client.nport_as_of_for(&id, &etf, today).await?;
                let exp = exposure(&report, matcher);
                let others = exp
                    .matches
                    .iter()
                    .filter(|m| !m.counted && m.holding.asset_cat != "DBT")
                    .count();
                let mut notes = Vec::new();
                if exp.debt_pct != 0.0 {
                    notes.push(format!("plus {:.2}% in debt (not counted)", exp.debt_pct));
                }
                if others > 0 {
                    notes.push(format!("{others} derivative or other line(s) not counted"));
                }
                Ok::<_, EdgarError>(EtfOk {
                    period_end: report.period_end,
                    pct: exp.pct / 100.0,
                    note: (!notes.is_empty()).then(|| notes.join("; ")),
                })
            };
            match tokio::time::timeout(timeout, work).await {
                Ok(Ok(ok)) => Ok(ok),
                Ok(Err(e @ EdgarError::NoNportFiling { .. })) => Err(Fail {
                    msg: e.to_string(),
                    partial: false,
                }),
                Ok(Err(e)) => Err(Fail {
                    msg: e.to_string(),
                    partial: true,
                }),
                Err(_) => Err(Fail {
                    msg: format!(
                        "Request timed out after {} s; SEC did not answer in time",
                        timeout.as_secs_f64().round()
                    ),
                    partial: true,
                }),
            }
        }
    };
    EtfRow {
        etf,
        held_value,
        result,
    }
}

pub async fn etf_overlap_partial(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let params = match parse_params(&q) {
        Ok(p) => p,
        Err(msg) => return bad_request(true, msg),
    };
    let Ok(pf) = load_portfolio(&state).await else {
        return db_error_fragment();
    };
    let Some(summary) = summarize(&pf.positions, params.symbol.as_deref()) else {
        return region(
            r#"<p class="empty-state">No portfolio data, so there is nothing to compare. Import lots with <code>ghost-alpha portfolio import</code>.</p>"#,
        )
        .into_response();
    };
    if params.symbol.is_none() && !valid_ticker(&summary.symbol) {
        return region(
            r#"<p class="empty-state">Your largest position cannot be looked up automatically. Add <code>?symbol=TICKER</code> to choose the stock.</p>"#,
        )
        .into_response();
    }
    let Some(client) = state.edgar.clone() else {
        return region(
            r#"<p class="empty-state">SEC EDGAR is not configured, so ETF holdings cannot be loaded. Run <code>ghost-alpha setup --section edgar</code> to set a User-Agent, then restart the dashboard.</p>"#,
        )
        .into_response();
    };

    let today = chrono::Local::now().date_naive();
    let timeout = state.etf_timeout.unwrap_or(ETF_TIMEOUT);
    let symbol = summary.symbol.clone();

    // Both ticker maps are loaded once per request and shared by every lookup.
    let (funds, companies) = tokio::join!(client.fund_tickers(), client.company_ticker_map());
    if let Err(e) = &funds {
        tracing::warn!("etf-overlap: fund list unavailable: {e}");
    }
    let (matcher, matcher_note) = match &companies {
        Ok(c) => (IssuerMatcher::new(&symbol, c.title(&symbol)), None),
        Err(e) => {
            tracing::warn!("etf-overlap: company list unavailable: {e}");
            (
                IssuerMatcher::new(&symbol, None),
                Some("Company name unavailable from SEC; matching by ticker only, so results may understate overlap."),
            )
        }
    };

    // Targets: held funds (largest first, capped) then the remaining candidates.
    let mut targets: Vec<(String, Option<f64>, Plan)> = Vec::new();
    if let Ok(funds) = &funds {
        let mut evaluated = 0usize;
        for pos in &pf.positions {
            let t = pos.ticker.to_ascii_uppercase();
            if t == symbol {
                continue;
            }
            let Some(id) = funds.get(&t) else { continue };
            let plan = if evaluated < MAX_HELD_FUNDS {
                evaluated += 1;
                Plan::Evaluate(id.clone())
            } else {
                Plan::Done(Fail {
                    msg: format!(
                        "Skipped: only the {MAX_HELD_FUNDS} largest held funds are evaluated"
                    ),
                    partial: true,
                })
            };
            targets.push((t, Some(pos.value), plan));
        }
    }
    let no_companies = TickerMap::default();
    for c in &params.candidates {
        if targets.iter().any(|(t, _, _)| t == c) {
            continue;
        }
        let plan = match &funds {
            Err(_) => Plan::Done(Fail {
                msg: "SEC fund list unavailable".to_string(),
                partial: false,
            }),
            Ok(f) => match &companies {
                Ok(co) => resolve_fund_id(f, co, c).map_or_else(
                    |e| {
                        Plan::Done(Fail {
                            msg: e.to_string(),
                            partial: false,
                        })
                    },
                    Plan::Evaluate,
                ),
                Err(_) => resolve_fund_id(f, &no_companies, c).map_or_else(
                    |_| {
                        Plan::Done(Fail {
                            msg: "Not in the SEC fund list, and the company list is unavailable"
                                .to_string(),
                            partial: false,
                        })
                    },
                    Plan::Evaluate,
                ),
            },
        };
        targets.push((c.clone(), None, plan));
    }

    let rows: Vec<EtfRow> =
        futures::future::join_all(targets.into_iter().map(|(etf, held, plan)| {
            evaluate_etf(&client, etf, held, plan, &matcher, today, timeout)
        }))
        .await;

    let held_ok: Vec<(f64, f64)> = rows
        .iter()
        .filter_map(|r| match (&r.result, r.held_value) {
            (Ok(ok), Some(v)) => Some((v, ok.pct)),
            _ => None,
        })
        .collect();
    let held_failed = rows
        .iter()
        .filter(|r| r.held_value.is_some() && matches!(&r.result, Err(f) if f.partial))
        .count();
    let lt = look_through(summary.direct_value, summary.total_value, &held_ok);

    // After-buy weights use the look-through exposure as the base.
    let afters: Vec<Option<f64>> = rows
        .iter()
        .map(|r| {
            r.result
                .as_ref()
                .ok()
                .map(|ok| weight_after_buy(lt.value, summary.total_value, params.buy, ok.pct))
        })
        .collect();
    let best = afters
        .iter()
        .enumerate()
        .filter_map(|(i, a)| a.map(|a| (i, a)))
        .fold(None::<(usize, f64)>, |best, (i, a)| match best {
            Some((_, b)) if b <= a => best,
            _ => Some((i, a)),
        })
        .map(|(i, _)| i);

    let sym = html_escape(&symbol);
    let via_weight = if summary.total_value > 0.0 {
        lt.via_etfs_value / summary.total_value
    } else {
        0.0
    };
    let mut reasons: Vec<String> = Vec::new();
    if funds.is_err() {
        reasons.push(
            "SEC fund list unavailable, so held funds could not be looked through".to_string(),
        );
    }
    if held_failed > 0 {
        reasons.push(format!(
            "{held_failed} held fund(s) could not be evaluated and are excluded"
        ));
    }
    let badge = if reasons.is_empty() {
        String::new()
    } else {
        format!(
            r#" <span class="badge badge-warning" data-badge="partial">Partial: {}</span>"#,
            html_escape(&reasons.join("; "))
        )
    };
    let lt_kpi = kpi(
        "look-through",
        &format!("Look-through {sym}"),
        &format!(
            "{} (direct {} + ETFs {})",
            pct1(lt.weight),
            pct1(summary.direct_weight),
            pct1(via_weight)
        ),
    );

    let buy = fmt_usd(params.buy);
    let mut body_rows = String::new();
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let etf = html_escape(&r.etf);
        let held = r.held_value.map(fmt_usd).unwrap_or_default();
        match (&r.result, afters[i]) {
            (Ok(ok), Some(after)) => {
                let is_best = best == Some(i);
                let star = if is_best {
                    r#" <span aria-hidden="true">★</span><span class="sr-only"> lowest weight after the purchase</span>"#
                } else {
                    ""
                };
                let best_attr = if is_best { r#" data-best="true""# } else { "" };
                let note = ok.note.as_deref().map(html_escape).unwrap_or_default();
                body_rows.push_str(&format!(
                    r#"<tr data-etf="{etf}"{best_attr}><td>{etf}{star}</td><td data-col="held">{held}</td><td data-col="pct">{pct:.2}%</td><td data-col="as-of">{as_of}</td><td data-col="after">{after}</td><td data-col="status"><small>{note}</small></td></tr>"#,
                    pct = ok.pct * 100.0,
                    as_of = ok.period_end,
                    after = pct1(after),
                ));
                table_rows.push(vec![
                    if is_best {
                        format!("{} ★", r.etf)
                    } else {
                        r.etf.clone()
                    },
                    held,
                    format!("{:.2}%", ok.pct * 100.0),
                    ok.period_end.to_string(),
                    pct1(after),
                ]);
            }
            _ => {
                let msg = match &r.result {
                    Err(f) => f.msg.replace('%', " percent"),
                    Ok(_) => String::new(),
                };
                body_rows.push_str(&format!(
                    r#"<tr data-etf="{etf}"><td>{etf}</td><td data-col="held">{held}</td><td data-col="pct"></td><td data-col="as-of"></td><td data-col="after"></td><td data-col="status">{msg}</td></tr>"#,
                    msg = html_escape(&msg),
                ));
            }
        }
    }

    let chart = bar_chart(&rows, &afters, best, &symbol, &buy);
    let table = if table_rows.is_empty() {
        String::new()
    } else {
        table_view(
            &format!("{symbol} weight after a {buy} purchase of each ETF"),
            &["ETF", "Held", "Equity %", "Holdings as of", "After buy"],
            &table_rows,
        )
    };
    let matcher_note = matcher_note
        .map(|n| format!(r#"<p role="note"><small>{}</small></p>"#, html_escape(n)))
        .unwrap_or_default();

    let inner = format!(
        r#"{lt_kpi}{badge}{matcher_note}
<table><caption>{sym} exposure by ETF and weight after buying {buy_h}</caption>
<thead><tr><th scope="col">ETF</th><th scope="col">Held</th><th scope="col">{sym} %</th><th scope="col">Holdings as of</th><th scope="col">{sym} after {buy_h} buy</th><th scope="col">Status</th></tr></thead>
<tbody>{body_rows}</tbody></table>
{chart}{table}
<p><small>★ marks the lowest look-through weight after the purchase. Holdings come from each fund's latest SEC N-PORT filing.</small></p>"#,
        buy_h = html_escape(&buy),
    );
    region(&inner).into_response()
}

/// Horizontal bar chart of the after-buy weights; colours come from CSS tokens.
fn bar_chart(
    rows: &[EtfRow],
    afters: &[Option<f64>],
    best: Option<usize>,
    symbol: &str,
    buy: &str,
) -> String {
    let bars: Vec<(&str, f64, bool)> = rows
        .iter()
        .zip(afters)
        .enumerate()
        .filter_map(|(i, (r, a))| a.map(|a| (r.etf.as_str(), a, best == Some(i))))
        .collect();
    if bars.is_empty() {
        return String::new();
    }
    let max = bars.iter().map(|b| b.1).fold(0.0_f64, f64::max);
    let (label_w, bar_w, row_h, pad) = (70.0, 300.0, 28.0, 6.0);
    let width = label_w + bar_w + 70.0;
    let height = bars.len() as f64 * row_h + pad * 2.0;
    let mut inner = String::new();
    for (i, (etf, after, is_best)) in bars.iter().enumerate() {
        let y = pad + i as f64 * row_h;
        let w = if max > 0.0 {
            after.max(0.0) / max * bar_w
        } else {
            0.0
        };
        let fill = if *is_best {
            "var(--ga-series-3)"
        } else {
            "var(--ga-series-1)"
        };
        let star = if *is_best { " \u{2605}" } else { "" };
        inner.push_str(&format!(
            r#"<text x="{lx}" y="{ty}" text-anchor="end" font-size="12" fill="var(--ga-text, currentColor)">{etf}</text><rect x="{label_w}" y="{y}" width="{w:.1}" height="{bh}" fill="{fill}"/><text x="{vx:.1}" y="{ty}" font-size="12" fill="var(--ga-text, currentColor)">{val}{star}</text>"#,
            lx = label_w - 6.0,
            ty = y + row_h / 2.0 + 4.0,
            bh = row_h - 8.0,
            vx = label_w + w + 6.0,
            etf = svg_escape(etf),
            val = pct1(*after),
        ));
    }
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">{inner}</svg>"#
    );
    let desc = bars
        .iter()
        .map(|(e, a, _)| format!("{e} {}", pct1(*a)))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"<div class="chart-container">{}</div>"#,
        accessible(
            &svg,
            &format!("{symbol} weight after buying {buy} of each ETF"),
            &format!("Bar chart of look-through {symbol} weight after the purchase: {desc}."),
        )
    )
}
