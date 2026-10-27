//! Concentration hub, L2b slice 1: the unwind schedule and projection fan
//! chart. `GET /partials/unwind` renders the panel that the concentration page
//! lazy-loads into `#unwind`; the page itself only embeds the placeholder.
//!
//! Query parameters (validated before any database access, never reflected):
//! `symbol`, `years` (1-10), `cadence` (quarterly|annual), `growth`, `mu` and
//! `vol` (percent per year; growth and mu in -50..=50, vol in 1..=150).

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use chrono::NaiveDate;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::portfolio::lookthrough::summarize;
use crate::portfolio::types::{AcquisitionType, TaxLot};
use crate::portfolio::unwind::{
    build_schedule, project, Cadence, ProjectionInput, TaxSettings, UnwindInput, DEFAULT_PATHS,
    DEFAULT_REST_MU, DEFAULT_REST_SIGMA, DEFAULT_RHO, PROJECTION_SEED,
};
use crate::tax::brackets::{
    parse_filing_status, state_tax_rate, FederalCapGainsBrackets, FilingStatus,
};

use super::charts::{accessible_with_id, fan_chart_svg_with_overlay};
use super::concentration::{
    fmt_usd, kpi, load_portfolio, num1, parse_symbol, pct1, pct_encode, valid_ticker, DB_ERROR,
};
use super::shell::{html_escape, table_view};
use super::AppState;

const PARTIAL_PATH: &str = "/partials/unwind";

// ─── Parameters ─────────────────────────────────────────────────────────────

/// Validated unwind parameters (percent values stay in percent).
#[derive(Debug, Clone)]
pub(super) struct UnwindParams {
    years: u32,
    cadence: Cadence,
    growth: f64,
    mu: f64,
    vol: f64,
}

fn cadence_name(c: Cadence) -> &'static str {
    match c {
        Cadence::Quarterly => "quarterly",
        Cadence::Annual => "annual",
    }
}

/// Parse and validate. The error is a static message that never contains input.
pub(super) fn parse_unwind_params(
    q: &HashMap<String, String>,
) -> Result<UnwindParams, &'static str> {
    let get = |k: &str| q.get(k).map(String::as_str).filter(|v| !v.is_empty());
    let pct = |k: &str, default: f64, lo: f64, hi: f64, msg: &'static str| match get(k) {
        None => Ok(default),
        Some(s) => match s.parse::<f64>() {
            Ok(v) if v.is_finite() && (lo..=hi).contains(&v) => Ok(v),
            _ => Err(msg),
        },
    };
    let years = match get("years") {
        None => 3,
        Some(s) => match s.parse::<u32>() {
            Ok(v) if (1..=10).contains(&v) => v,
            _ => return Err("Invalid years: expected a whole number from 1 to 10."),
        },
    };
    let cadence = match get("cadence").map(str::to_ascii_lowercase).as_deref() {
        None | Some("quarterly") => Cadence::Quarterly,
        Some("annual") => Cadence::Annual,
        Some(_) => return Err("Invalid cadence: expected quarterly or annual."),
    };
    Ok(UnwindParams {
        years,
        cadence,
        growth: pct(
            "growth",
            0.0,
            -50.0,
            50.0,
            "Invalid growth: expected -50 to 50 percent a year.",
        )?,
        mu: pct(
            "mu",
            8.0,
            -50.0,
            50.0,
            "Invalid mu: expected -50 to 50 percent a year.",
        )?,
        vol: pct(
            "vol",
            30.0,
            1.0,
            150.0,
            "Invalid vol: expected 1 to 150 percent a year.",
        )?,
    })
}

/// The `#unwind` placeholder embedded in the page. An invalid default symbol
/// (derived from the largest position) gets a static explanation instead.
pub(super) fn region_placeholder(symbol: &str, defaulted: bool, up: &UnwindParams) -> String {
    if defaulted && !valid_ticker(symbol) {
        return r#"<div id="unwind"><p class="empty-state">Your largest position cannot be looked up automatically. Choose the stock by adding <code>?symbol=TICKER</code> to the address.</p></div>"#.to_string();
    }
    let enc = |v: &str| html_escape(&pct_encode(v));
    let qs = format!(
        "symbol={}&amp;years={}&amp;cadence={}&amp;growth={}&amp;mu={}&amp;vol={}",
        enc(symbol),
        up.years,
        cadence_name(up.cadence),
        enc(&up.growth.to_string()),
        enc(&up.mu.to_string()),
        enc(&up.vol.to_string()),
    );
    format!(
        r#"<div id="unwind" hx-get="{PARTIAL_PATH}?{qs}" hx-trigger="load" hx-swap="outerHTML" aria-busy="true">
      <p>Building the unwind schedule&hellip;</p>
      <noscript><a href="{PARTIAL_PATH}?{qs}">Load the unwind schedule</a></noscript>
    </div>"#
    )
}

// ─── Responses ──────────────────────────────────────────────────────────────

fn region(inner: &str) -> Html<String> {
    Html(format!(
        r#"<div id="unwind" aria-busy="false">{inner}</div>"#
    ))
}

fn alert(status: StatusCode, msg: &str) -> Response {
    (
        status,
        Html(format!(
            r#"<div id="unwind" role="alert"><p class="error">{}</p></div>"#,
            html_escape(msg)
        )),
    )
        .into_response()
}

fn empty_state(msg: &str) -> Response {
    region(&format!(r#"<p class="empty-state">{msg}</p>"#)).into_response()
}

// ─── Inputs ─────────────────────────────────────────────────────────────────

/// How the profile resolved into tax settings.
enum TaxProfile {
    /// Settings plus a note about the filing status.
    Ready(Box<TaxSettings>, FilingNote),
    /// No `[profile] state`.
    NoState,
    /// A state that is set but has no built-in rates.
    Unsupported(String),
}

enum FilingNote {
    /// A recognised status with its own schedule (or none given: single).
    Plain(&'static str),
    /// A recognised status that uses the single schedule (MFS, HOH).
    UsesSingle(&'static str),
    /// A value that is not a filing status.
    Unrecognised(String),
}

fn filing_label(s: &FilingStatus) -> &'static str {
    match s {
        FilingStatus::Single => "single",
        FilingStatus::MarriedFilingJointly => "married filing jointly",
        FilingStatus::MarriedFilingSeparately => "married filing separately",
        FilingStatus::HeadOfHousehold => "head of household",
    }
}

fn tax_profile(config: &AppConfig) -> TaxProfile {
    let Some(p) = config.profile.as_ref() else {
        return TaxProfile::NoState;
    };
    let Some(raw_state) = p.state.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return TaxProfile::NoState;
    };
    let Ok(state) = state_tax_rate(&raw_state.to_uppercase()) else {
        return TaxProfile::Unsupported(raw_state.chars().take(16).collect());
    };
    let (status, note) = match p.filing.as_deref().map(str::trim).filter(|f| !f.is_empty()) {
        None => (FilingStatus::Single, FilingNote::Plain("single")),
        Some(f) => match parse_filing_status(f) {
            Some(st) => {
                let label = filing_label(&st);
                let note = match st {
                    FilingStatus::Single | FilingStatus::MarriedFilingJointly => {
                        FilingNote::Plain(label)
                    }
                    _ => FilingNote::UsesSingle(label),
                };
                (st, note)
            }
            None => (
                FilingStatus::Single,
                FilingNote::Unrecognised(f.chars().take(24).collect()),
            ),
        },
    };
    TaxProfile::Ready(
        Box::new(TaxSettings {
            state,
            brackets: FederalCapGainsBrackets::for_filing(&status),
            other_income: p.other_income.unwrap_or(0.0),
        }),
        note,
    )
}

fn to_tax_lot(row: &crate::storage::db::TaxLotRow, today: NaiveDate) -> TaxLot {
    TaxLot {
        id: Uuid::parse_str(&row.lot_id).unwrap_or_default(),
        ticker: row.ticker.clone(),
        shares: row.shares,
        cost_basis_per_share: row.cost_basis_per_share,
        // An unreadable date is treated as just acquired (short term), the cautious choice.
        acquired_date: NaiveDate::parse_from_str(&row.acquisition_date, "%Y-%m-%d")
            .unwrap_or(today),
        acquisition_type: AcquisitionType::Buy,
        remaining_shares: row.shares,
    }
}

// ─── Handler ────────────────────────────────────────────────────────────────

pub async fn unwind_partial(
    State(state): State<AppState>,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let requested = match parse_symbol(&q) {
        Ok(s) => s,
        Err(msg) => return alert(StatusCode::BAD_REQUEST, msg),
    };
    let up = match parse_unwind_params(&q) {
        Ok(p) => p,
        Err(msg) => return alert(StatusCode::BAD_REQUEST, msg),
    };
    let Ok(pf) = load_portfolio(&state).await else {
        return alert(StatusCode::INTERNAL_SERVER_ERROR, DB_ERROR);
    };
    let Some(summary) = summarize(&pf.positions, requested.as_deref()) else {
        return empty_state(
            "No portfolio data, so there is nothing to unwind. Import lots with <code>ghost-alpha portfolio import</code>.",
        );
    };
    if requested.is_none() && !valid_ticker(&summary.symbol) {
        return empty_state(
            "Your largest position cannot be looked up automatically. Add <code>?symbol=TICKER</code> to choose the stock.",
        );
    }
    let symbol = summary.symbol.clone();
    let Some(position) = pf
        .positions
        .iter()
        .find(|p| p.ticker.eq_ignore_ascii_case(&symbol))
    else {
        return empty_state(
            "That symbol is not held (or cannot be priced), so there is nothing to unwind.",
        );
    };

    let mut unsupported_state: Option<String> = None;
    let today = chrono::Local::now().date_naive();
    let lots: Vec<TaxLot> = pf
        .lots
        .iter()
        .filter(|l| l.ticker.eq_ignore_ascii_case(&symbol) && l.shares > 0.0)
        .map(|l| to_tax_lot(l, today))
        .collect();
    let other_value = (summary.total_value - summary.direct_value).max(0.0);
    let (tax, tax_note) = match tax_profile(&state.config) {
        TaxProfile::Ready(t, note) => (Some(*t), Some(note)),
        TaxProfile::NoState => (None, None),
        TaxProfile::Unsupported(s) => {
            unsupported_state = Some(s);
            (None, None)
        }
    };
    let has_tax = tax.is_some();
    let bracket_year = tax.as_ref().map(|t| t.brackets.year);

    let schedule = build_schedule(&UnwindInput {
        lots,
        price: position.price,
        other_value,
        years: up.years,
        cadence: up.cadence,
        growth: up.growth / 100.0,
        start: today,
        tax,
    });

    // Per-year fractions of the original shares, and the effective tax rate.
    let per_year = match up.cadence {
        Cadence::Quarterly => 4,
        Cadence::Annual => 1,
    };
    let mut sell_fractions = vec![0.0; up.years as usize];
    if schedule.totals.shares > 0.0 {
        for (k, p) in schedule.periods.iter().enumerate() {
            if let Some(f) = sell_fractions.get_mut(k / per_year) {
                *f += p.shares / schedule.totals.shares;
            }
        }
    }
    let tax_rate = match schedule.totals.tax {
        Some(t) if schedule.totals.proceeds > 0.0 => t / schedule.totals.proceeds,
        _ => 0.0,
    };
    let proj_input = ProjectionInput {
        stock_value: summary.direct_value,
        other_value,
        years: up.years,
        stock_mu: up.mu / 100.0,
        stock_sigma: up.vol / 100.0,
        rest_mu: DEFAULT_REST_MU,
        rest_sigma: DEFAULT_REST_SIGMA,
        rho: DEFAULT_RHO,
        paths: DEFAULT_PATHS,
        sell_fractions,
        tax_rate,
    };
    let proj =
        match tokio::task::spawn_blocking(move || project(&proj_input, PROJECTION_SEED)).await {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("unwind: projection task failed: {e}");
                return alert(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "The projection could not be computed.",
                );
            }
        };

    let sym = html_escape(&symbol);
    let t = &schedule.totals;

    // KPIs
    let last = up.years as usize;
    let mut kpis = Vec::new();
    if let Some(total_tax) = t.tax {
        kpis.push(kpi("total-tax", "Total tax", &fmt_usd(total_tax)));
    }
    kpis.push(kpi(
        "after-tax-proceeds",
        "After-tax proceeds",
        &fmt_usd(t.net),
    ));
    kpis.push(kpi(
        "p10-hold",
        &format!("Year {} downside (p10), hold", up.years),
        &fmt_usd(proj.hold[last].p10),
    ));
    kpis.push(kpi(
        "p10-unwind",
        &format!("Year {} downside (p10), unwind", up.years),
        &fmt_usd(proj.unwind[last].p10),
    ));

    // Schedule table
    let tax_th = if has_tax {
        r#"<th scope="col">Tax</th>"#
    } else {
        ""
    };
    let mut rows = String::new();
    for (k, p) in schedule.periods.iter().enumerate() {
        let tax_td = match (has_tax, p.tax) {
            (true, Some(x)) => format!(r#"<td data-col="tax">{}</td>"#, fmt_usd(x)),
            _ => String::new(),
        };
        rows.push_str(&format!(
            r#"<tr data-period="{n}"><td>{label}</td><td data-col="date">{date}</td><td data-col="shares">{shares:.2}</td><td data-col="proceeds">{proceeds}</td>{tax_td}<td data-col="net">{net}</td><td data-col="weight-after">{w}</td></tr>"#,
            n = k + 1,
            label = html_escape(&p.label),
            date = p.date,
            shares = p.shares,
            proceeds = fmt_usd(p.proceeds),
            net = fmt_usd(p.net),
            w = pct1(p.weight_after),
        ));
    }
    let total_tax_td = match t.tax {
        Some(x) if has_tax => format!(r#"<td data-col="tax">{}</td>"#, fmt_usd(x)),
        _ => String::new(),
    };
    rows.push_str(&format!(
        r#"<tr data-period="total"><td>Total</td><td data-col="date"></td><td data-col="shares">{shares:.2}</td><td data-col="proceeds">{proceeds}</td>{total_tax_td}<td data-col="net">{net}</td><td data-col="weight-after"></td></tr>"#,
        shares = t.shares,
        proceeds = fmt_usd(t.proceeds),
        net = fmt_usd(t.net),
    ));
    let n_periods = schedule.periods.len();
    let caption = format!(
        "{sym} unwind: {n_periods} {} sale(s) over {} year(s), highest-cost lots first",
        cadence_name(up.cadence),
        up.years
    );

    // Fan chart
    let hold_median: Vec<f64> = proj.hold.iter().map(|y| y.p50).collect();
    let (h, u) = (&proj.hold[last], &proj.unwind[last]);
    let desc = format!(
        "Fan chart of portfolio value over {} years. Year {}: hold p10 {}, p50 {}, p90 {}; unwind p10 {}, p50 {}, p90 {}. The shaded fan is the unwind range and the dashed line is the hold median.",
        up.years,
        up.years,
        fmt_usd(h.p10),
        fmt_usd(h.p50),
        fmt_usd(h.p90),
        fmt_usd(u.p10),
        fmt_usd(u.p50),
        fmt_usd(u.p90),
    );
    let chart = accessible_with_id(
        &fan_chart_svg_with_overlay(&proj.unwind, Some(&hold_median), 640, 320),
        &format!("Portfolio value projection, hold versus unwinding {symbol}"),
        &desc,
        "unwind-fan",
    );
    let table_rows: Vec<Vec<String>> = proj
        .hold
        .iter()
        .zip(&proj.unwind)
        .map(|(a, b)| {
            vec![
                if a.year == 0 {
                    "Start".to_string()
                } else {
                    a.year.to_string()
                },
                fmt_usd(a.p10),
                fmt_usd(a.p50),
                fmt_usd(a.p90),
                fmt_usd(b.p10),
                fmt_usd(b.p50),
                fmt_usd(b.p90),
            ]
        })
        .collect();
    let view = table_view(
        "Projected portfolio value percentiles, hold versus unwind",
        &[
            "Year",
            "Hold p10",
            "Hold p50",
            "Hold p90",
            "Unwind p10",
            "Unwind p50",
            "Unwind p90",
        ],
        &table_rows,
    );

    let hint = if let Some(st) = &unsupported_state {
        format!(
            r#"<p role="note" class="muted"><small>Tax is not shown because the state "{}" in your profile is not supported. Run <code>ghost-alpha setup --section profile</code> to choose a supported state.</small></p>"#,
            html_escape(st)
        )
    } else if !has_tax {
        r#"<p role="note" class="muted"><small>Tax is not shown because no state is set in your profile. Run <code>ghost-alpha setup --section profile</code> to add it.</small></p>"#.to_string()
    } else {
        String::new()
    };
    let bracket_note = match (&tax_note, bracket_year) {
        (Some(note), Some(year)) => {
            let (label, extra) = match note {
                FilingNote::Plain(l) => (html_escape(l), String::new()),
                FilingNote::UsesSingle(l) => (
                    html_escape(l),
                    " No separate schedule is built in for this status, so the single schedule is used.".to_string(),
                ),
                FilingNote::Unrecognised(raw) => (
                    "single".to_string(),
                    format!(
                        " Your filing status \"{}\" was not recognised, so the single schedule is used.",
                        html_escape(raw)
                    ),
                ),
            };
            format!(
                r#"<p role="note" class="muted"><small>Federal brackets: {year}, filing status {label}.{extra} Tax is the change in each calendar year's total tax, with short- and long-term gains netted within the year. Each calendar year stands alone: there is no loss carryforward and no $3,000 offset. A negative period tax means that sale reduces the year's tax.</small></p>"#
            )
        }
        _ => String::new(),
    };
    let rate_pct = if t.proceeds > 0.0 {
        t.tax.map_or(0.0, |x| x / t.proceeds) * 100.0
    } else {
        0.0
    };
    let cost_note = if position.priced_at_cost {
        r#"<p role="note" class="muted"><small>No recent close for this symbol: the price used is your average cost.</small></p>"#
    } else {
        ""
    };

    let inner = format!(
        r#"{kpis}
{hint}{bracket_note}{cost_note}
<table><caption>{caption}</caption>
<thead><tr><th scope="col">Period</th><th scope="col">Date</th><th scope="col">Shares</th><th scope="col">Proceeds</th>{tax_th}<th scope="col">Net</th><th scope="col">{sym} weight after</th></tr></thead>
<tbody>{rows}</tbody></table>
<div class="chart-container">{chart}</div>
{view}
<p><strong>Illustrative assumptions.</strong> Stock expected return {mu}% and volatility {vol}% a year; rest of the portfolio {rest_mu:.0}% and {rest_sigma:.0}%; correlation {rho}; {paths} simulated paths with random seed {seed}. Schedule prices grow {growth}% a year. Sales are modelled at the start of each year and net proceeds move into the rest of the portfolio. Hold values are before tax that would be due on a later sale; unwind proceeds are reduced by the schedule's average tax rate ({rate:.1}%). Not tax or investment advice.</p>"#,
        kpis = kpis.join("\n"),
        mu = num1(up.mu),
        vol = num1(up.vol),
        growth = num1(up.growth),
        rate = rate_pct,
        rest_mu = DEFAULT_REST_MU * 100.0,
        rest_sigma = DEFAULT_REST_SIGMA * 100.0,
        rho = DEFAULT_RHO,
        paths = DEFAULT_PATHS,
        seed = PROJECTION_SEED,
    );
    region(&inner).into_response()
}
