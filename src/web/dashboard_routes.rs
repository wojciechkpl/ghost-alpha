//! Extended dashboard page handlers for diversification advisory features.
//!
//! Each handler follows the project pattern: fetch from the repository via
//! `spawn_blocking`, render server-side HTML with pico.css, and return
//! `Html<String>`. Empty DB states are handled gracefully with informative
//! messages.

use axum::extract::{Query, State};
use axum::response::Html;
use serde::Deserialize;

use crate::storage::db::Repository;
use crate::web::charts::{
    correlation_matrix_svg, donut_svg, frontier_svg, timeline_bar_svg, DonutSlice, FrontierPoint,
    TimelinePeriod,
};

use super::shell::{html_escape, shell};
use super::AppState;

// ─── Utilities ──────────────────────────────────────────────────────────────

/// Format an optional f64 to a fixed-decimal string, or a dash for `None`.
fn fmt_opt(v: Option<f64>, decimals: usize) -> String {
    match v {
        Some(val) if val.is_finite() => format!("{val:.decimals$}"),
        _ => "&mdash;".to_string(),
    }
}

/// Format a percentage with sign.
#[allow(dead_code)]
fn fmt_pct(v: f64) -> String {
    if v.is_finite() {
        format!("{v:+.2}%")
    } else {
        "&mdash;".to_string()
    }
}

/// CSS class for gain/loss styling.
#[allow(dead_code)]
fn pl_class(v: f64) -> &'static str {
    if v >= 0.0 {
        "gain"
    } else {
        "loss"
    }
}

// ─── Query Parameter Structs ────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct TickerParam {
    pub ticker: Option<String>,
}

// ─── Portfolio Colours ──────────────────────────────────────────────────────

const PORTFOLIO_COLOURS: &[&str] = &[
    "#4c6ef5", "#7950f2", "#ae3ec9", "#e64980", "#f76707", "#f59f00", "#82c91e", "#20c997",
    "#15aabf", "#339af0", "#5c7cfa", "#845ef7",
];

// ─── 1. Portfolio Overview ──────────────────────────────────────────────────

pub async fn portfolio_page(State(state): State<AppState>) -> Html<String> {
    let repo = state.repo.clone();
    let lots = tokio::task::spawn_blocking(move || repo.get_all_tax_lots())
        .await
        .unwrap_or_else(|_| Ok(Vec::new()))
        .unwrap_or_default();

    if lots.is_empty() {
        let body = r#"<p class="empty-state">No portfolio data. Import via <code>ghost-alpha portfolio import</code></p>"#;
        return Html(shell("/portfolio", "Portfolio Overview", body));
    }

    // Aggregate by ticker: total shares, weighted cost basis
    let mut agg: std::collections::BTreeMap<String, (f64, f64)> = std::collections::BTreeMap::new();
    for lot in &lots {
        let entry = agg.entry(lot.ticker.clone()).or_insert((0.0, 0.0));
        entry.0 += lot.shares;
        entry.1 += lot.shares * lot.cost_basis_per_share;
    }

    let total_cost: f64 = agg.values().map(|(_, cost)| cost).sum();

    // Build positions table and donut slices
    let mut rows = String::new();
    let mut slices: Vec<DonutSlice> = Vec::with_capacity(agg.len());
    let mut hhi: f64 = 0.0;

    for (i, (ticker, (shares, total_lot_cost))) in agg.iter().enumerate() {
        let weight = if total_cost > 0.0 {
            total_lot_cost / total_cost * 100.0
        } else {
            0.0
        };
        hhi += (weight / 100.0).powi(2);
        let avg_cost = if *shares > 0.0 {
            total_lot_cost / shares
        } else {
            0.0
        };
        let escaped_ticker = html_escape(ticker);
        let color = PORTFOLIO_COLOURS[i % PORTFOLIO_COLOURS.len()];

        slices.push(DonutSlice {
            label: ticker.clone(),
            value: *total_lot_cost,
            color: color.to_string(),
        });

        rows.push_str(&format!(
            "<tr><td>{escaped_ticker}</td><td>{shares:.2}</td><td>${avg_cost:.2}</td>\
             <td>${total_lot_cost:.2}</td><td>{weight:.1}%</td></tr>"
        ));
    }

    let donut = donut_svg(&slices, 300, 300);
    let hhi_display = format!("{:.4}", hhi);
    let hhi_label = if hhi > 0.25 {
        "Highly Concentrated"
    } else if hhi > 0.15 {
        "Moderately Concentrated"
    } else {
        "Diversified"
    };

    let body = format!(
        r#"<div class="grid">
  <div>
    <h2>Positions</h2>
    <table><caption class="sr-only">Portfolio positions: shares, cost in USD, weight in percent</caption>
      <thead><tr><th scope="col">Ticker</th><th scope="col">Shares</th><th scope="col">Avg Cost</th><th scope="col">Cost Basis</th><th scope="col">Weight</th></tr></thead>
      <tbody>{rows}</tbody>
    </table>
  </div>
  <div>
    <h2>Allocation</h2>
    <div class="chart-container">{donut}</div>
    <p><strong>HHI:</strong> {hhi_display} &mdash; {hhi_label}</p>
  </div>
</div>"#
    );
    Html(shell("/portfolio", "Portfolio Overview", &body))
}

// ─── 2. Correlation Matrix ──────────────────────────────────────────────────

pub async fn correlation_page(State(state): State<AppState>) -> Html<String> {
    let repo = state.repo.clone();
    let snapshot = tokio::task::spawn_blocking(move || repo.get_latest_correlation_snapshot())
        .await
        .unwrap_or_else(|_| Ok(None))
        .unwrap_or(None);

    let Some((snap_date, tickers, matrix, window)) = snapshot else {
        let body = r#"<p class="empty-state">No correlation data. Run <code>ghost-alpha correlate</code> to generate.</p>"#;
        return Html(shell("/correlation", "Correlation Matrix", body));
    };

    let ticker_refs: Vec<&str> = tickers.iter().map(|s| s.as_str()).collect();
    let heatmap = correlation_matrix_svg(&ticker_refs, &matrix, 600, 600);

    // Find high-correlation pairs (>0.8, excluding diagonal)
    let mut high_pairs = Vec::new();
    for i in 0..tickers.len() {
        for j in (i + 1)..tickers.len() {
            if let Some(row) = matrix.get(i) {
                if let Some(&val) = row.get(j) {
                    if val > 0.8 {
                        high_pairs.push((tickers[i].clone(), tickers[j].clone(), val));
                    }
                }
            }
        }
    }
    high_pairs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    let mut pair_rows = String::new();
    for (a, b, corr) in &high_pairs {
        let ea = html_escape(a);
        let eb = html_escape(b);
        pair_rows.push_str(&format!(
            "<tr><td>{ea}</td><td>{eb}</td><td>{corr:.3}</td></tr>"
        ));
    }

    let pair_table = if pair_rows.is_empty() {
        "<p class=\"empty-state\">No pairs with correlation &gt; 0.80</p>".to_string()
    } else {
        format!(
            r#"<table><caption class="sr-only">Highly correlated ticker pairs (correlation above 0.80)</caption>
  <thead><tr><th scope="col">Ticker A</th><th scope="col">Ticker B</th><th scope="col">Correlation</th></tr></thead>
  <tbody>{pair_rows}</tbody>
</table>"#
        )
    };

    let escaped_date = html_escape(&snap_date);
    let body = format!(
        r#"<p>Snapshot: {escaped_date} &mdash; Window: {window} days</p>
<div class="chart-container">{heatmap}</div>
<h2>Highly Correlated Pairs (&gt; 0.80)</h2>
{pair_table}"#
    );
    Html(shell("/correlation", "Correlation Matrix", &body))
}

// ─── 3. Valuation Dashboard ────────────────────────────────────────────────

pub async fn valuation_page(
    State(state): State<AppState>,
    Query(params): Query<TickerParam>,
) -> Html<String> {
    let ticker = match params.ticker {
        Some(t) if !t.is_empty() => t,
        _ => {
            let body = r#"<p class="empty-state">No valuation data available. Select a ticker: <code>/valuation?ticker=AAPL</code></p>"#;
            return Html(shell("/valuation", "Valuation Dashboard", body));
        }
    };

    let repo = state.repo.clone();
    let t1 = ticker.clone();
    let t2 = ticker.clone();
    let repo2 = state.repo.clone();

    let history = tokio::task::spawn_blocking(move || repo.get_fundamental_history(&t1, 20))
        .await
        .unwrap_or_else(|_| Ok(Vec::new()))
        .unwrap_or_default();

    let bands = tokio::task::spawn_blocking(move || repo2.get_valuation_bands(&t2, "pe_ratio", 5))
        .await
        .unwrap_or_else(|_| Ok(Vec::new()))
        .unwrap_or_default();

    let escaped_ticker = html_escape(&ticker);

    if history.is_empty() {
        let body = format!(
            r#"<h2>Valuation &mdash; {escaped_ticker}</h2>
<p class="empty-state">No valuation data available for {escaped_ticker}.</p>"#
        );
        return Html(shell("/valuation", "Valuation", &body));
    }

    // Build metrics table
    let mut metric_rows = String::new();
    for rec in &history {
        let period = html_escape(&rec.period_date);
        let ptype = html_escape(&rec.period_type);
        metric_rows.push_str(&format!(
            "<tr><td>{period}</td><td>{ptype}</td><td>{pe}</td><td>{pb}</td>\
             <td>{ev}</td><td>{gm}</td><td>{om}</td><td>{nm}</td></tr>",
            pe = fmt_opt(rec.pe_ratio, 1),
            pb = fmt_opt(rec.pb_ratio, 2),
            ev = fmt_opt(rec.ev_ebitda, 1),
            gm = fmt_opt(rec.gross_margin.map(|v| v * 100.0), 1),
            om = fmt_opt(rec.operating_margin.map(|v| v * 100.0), 1),
            nm = fmt_opt(rec.net_margin.map(|v| v * 100.0), 1),
        ));
    }

    // PE sparkline from bands
    let sparkline = if !bands.is_empty() {
        let values: Vec<f64> = bands.iter().map(|(_, v)| *v).collect();
        crate::web::charts::sparkline_svg(&values, 200, 40)
    } else {
        String::new()
    };

    let body = format!(
        r#"<h2>Valuation &mdash; {escaped_ticker}</h2>
<h2>P/E Trend</h2>
<div class="chart-container">{sparkline}</div>
<h2>Fundamental History</h2>
<div style="overflow-x: auto;">
<table><caption class="sr-only">Valuation ratios and margins by period</caption>
  <thead><tr><th scope="col">Period</th><th scope="col">Type</th><th scope="col">P/E</th><th scope="col">P/B</th><th scope="col">EV/EBITDA</th><th scope="col">Gross %</th><th scope="col">Op %</th><th scope="col">Net %</th></tr></thead>
  <tbody>{metric_rows}</tbody>
</table>
</div>"#
    );
    Html(shell("/valuation", "Valuation", &body))
}

// ─── 4. Tax Overview ────────────────────────────────────────────────────────

pub async fn tax_page(State(state): State<AppState>) -> Html<String> {
    let repo = state.repo.clone();
    let lots = tokio::task::spawn_blocking(move || repo.get_all_tax_lots())
        .await
        .unwrap_or_else(|_| Ok(Vec::new()))
        .unwrap_or_default();

    if lots.is_empty() {
        let body = r#"<p class="empty-state">No tax lots imported.</p>"#;
        return Html(shell("/tax", "Tax Overview", body));
    }

    // Build detail table
    let mut detail_rows = String::new();
    let mut total_cost: f64 = 0.0;

    for lot in &lots {
        let lot_id = html_escape(&lot.lot_id);
        let ticker = html_escape(&lot.ticker);
        let acq_date = html_escape(&lot.acquisition_date);
        let acq_type = html_escape(&lot.acquisition_type);
        let cost = lot.shares * lot.cost_basis_per_share;
        total_cost += cost;

        detail_rows.push_str(&format!(
            "<tr><td>{lot_id}</td><td>{ticker}</td><td>{shares:.2}</td>\
             <td>${cost_per:.2}</td><td>${cost:.2}</td><td>{acq_date}</td><td>{acq_type}</td></tr>",
            shares = lot.shares,
            cost_per = lot.cost_basis_per_share,
        ));
    }

    let total_lots = lots.len();

    let body = format!(
        r#"<h2>Summary</h2>
<table><caption class="sr-only">Tax summary metrics</caption>
  <thead><tr><th scope="col">Metric</th><th scope="col">Value</th></tr></thead>
  <tbody>
    <tr><td>Total Lots</td><td>{total_lots}</td></tr>
    <tr><td>Total Cost Basis</td><td>${total_cost:.2}</td></tr>
  </tbody>
</table>
<h2>Tax Lot Detail</h2>
<div style="overflow-x: auto;">
<table><caption class="sr-only">Tax lots with cost basis in USD and holding type</caption>
  <thead><tr><th scope="col">Lot ID</th><th scope="col">Ticker</th><th scope="col">Shares</th><th scope="col">Cost/Share</th><th scope="col">Total Cost</th><th scope="col">Acquired</th><th scope="col">Type</th></tr></thead>
  <tbody>{detail_rows}</tbody>
</table>
</div>"#
    );
    Html(shell("/tax", "Tax Overview", &body))
}

// ─── 5. Optimization ───────────────────────────────────────────────────────

pub async fn optimize_page(State(state): State<AppState>) -> Html<String> {
    let repo = state.repo.clone();
    let snapshot = tokio::task::spawn_blocking(move || repo.get_latest_correlation_snapshot())
        .await
        .unwrap_or_else(|_| Ok(None))
        .unwrap_or(None);

    // Model portfolios (static data)
    let models = vec![
        crate::optimize::models::conservative(),
        crate::optimize::models::moderate(),
        crate::optimize::models::growth(),
    ];

    let mut model_table = String::new();
    for model in &models {
        let name = html_escape(&model.name);
        let desc = html_escape(&model.description);
        let weights_str: String = model
            .weights
            .iter()
            .map(|(t, w)| format!("{} {:.0}%", html_escape(t), w * 100.0))
            .collect::<Vec<_>>()
            .join(", ");
        model_table.push_str(&format!(
            "<tr><td><strong>{name}</strong></td><td>{desc}</td><td>{weights_str}</td></tr>"
        ));
    }

    // Build frontier chart from demo points if correlation data exists
    let frontier_html = if snapshot.is_some() {
        let demo_points = vec![
            FrontierPoint {
                risk: 5.0,
                ret: 3.0,
                label: "Conservative".into(),
                color: "#4c6ef5".into(),
                highlighted: true,
            },
            FrontierPoint {
                risk: 10.0,
                ret: 6.5,
                label: "Moderate".into(),
                color: "#7950f2".into(),
                highlighted: true,
            },
            FrontierPoint {
                risk: 16.0,
                ret: 9.0,
                label: "Growth".into(),
                color: "#ae3ec9".into(),
                highlighted: true,
            },
        ];
        let svg = frontier_svg(&demo_points, None, 600, 400);
        format!(
            r#"<h2>Efficient Frontier</h2>
<div class="chart-container">{svg}</div>"#
        )
    } else {
        r#"<p class="empty-state">Run <code>ghost-alpha correlate</code> to enable frontier chart.</p>"#
            .to_string()
    };

    let body = format!(
        r#"{frontier_html}
<h2>Model Portfolios</h2>
<table><caption class="sr-only">Optimization profiles and their asset weights</caption>
  <thead><tr><th scope="col">Name</th><th scope="col">Description</th><th scope="col">Weights</th></tr></thead>
  <tbody>{model_table}</tbody>
</table>"#
    );
    Html(shell("/optimize", "Portfolio Optimization", &body))
}

// ─── 6. ETF Comparison ─────────────────────────────────────────────────────

pub async fn etf_page(State(state): State<AppState>) -> Html<String> {
    let repo = state.repo.clone();
    let snapshots = tokio::task::spawn_blocking(move || repo.get_all_etf_snapshots())
        .await
        .unwrap_or_else(|_| Ok(Vec::new()))
        .unwrap_or_default();

    if snapshots.is_empty() {
        let body = r#"<p class="empty-state">No ETF data. Run <code>ghost-alpha etf-sync</code> to populate.</p>"#;
        return Html(shell("/etf", "ETF Comparison", body));
    }

    let mut etf_rows = String::new();
    for snap in &snapshots {
        let ticker = html_escape(&snap.ticker);
        let name = html_escape(&snap.name);
        let category = html_escape(&snap.category);
        let holdings_summary: String = snap
            .top_holdings
            .iter()
            .take(5)
            .map(|h| format!("{} ({:.1}%)", html_escape(&h.ticker), h.weight * 100.0))
            .collect::<Vec<_>>()
            .join(", ");
        let aum_display = if snap.aum >= 1_000_000_000.0 {
            format!("${:.1}B", snap.aum / 1_000_000_000.0)
        } else if snap.aum >= 1_000_000.0 {
            format!("${:.1}M", snap.aum / 1_000_000.0)
        } else {
            format!("${:.0}", snap.aum)
        };

        etf_rows.push_str(&format!(
            "<tr><td><strong>{ticker}</strong></td><td>{name}</td><td>{category}</td>\
             <td>{er:.2}%</td><td>{aum_display}</td><td>{holdings_summary}</td></tr>",
            er = snap.expense_ratio * 100.0,
        ));
    }

    let body = format!(
        r#"<div style="overflow-x: auto;">
<table><caption class="sr-only">ETF comparison: expense ratio, assets under management and top holdings</caption>
  <thead><tr><th scope="col">Ticker</th><th scope="col">Name</th><th scope="col">Category</th><th scope="col">Expense Ratio</th><th scope="col">AUM</th><th scope="col">Top Holdings</th></tr></thead>
  <tbody>{etf_rows}</tbody>
</table>
</div>"#
    );
    Html(shell("/etf", "ETF Comparison", &body))
}

// ─── 7. Selling Programme ──────────────────────────────────────────────────

pub async fn selling_page(State(state): State<AppState>) -> Html<String> {
    // Fetch all selling programmes by looking at all tax lot tickers
    let repo = state.repo.clone();
    let repo2 = state.repo.clone();
    let lots = tokio::task::spawn_blocking(move || repo.get_all_tax_lots())
        .await
        .unwrap_or_else(|_| Ok(Vec::new()))
        .unwrap_or_default();

    let tickers: Vec<String> = {
        let mut t: Vec<String> = lots.iter().map(|l| l.ticker.clone()).collect();
        t.sort();
        t.dedup();
        t
    };

    // Fetch programmes concurrently for all tickers
    let futures: Vec<_> = tickers
        .iter()
        .map(|ticker| {
            let r = repo2.clone();
            let t = ticker.clone();
            tokio::task::spawn_blocking(move || r.get_selling_programmes_for_ticker(&t))
        })
        .collect();
    let results = futures::future::join_all(futures).await;
    let mut all_programmes = Vec::new();
    for result in results {
        all_programmes.extend(
            result
                .unwrap_or_else(|_| Ok(Vec::new()))
                .unwrap_or_default(),
        );
    }

    if all_programmes.is_empty() {
        let body = r#"<p class="empty-state">No selling programme configured.</p>"#;
        return Html(shell("/selling", "Selling Programme", body));
    }

    let mut sections = String::new();
    for prog in &all_programmes {
        let ticker = html_escape(&prog.ticker);
        let prog_id = prog.id.unwrap_or(0);
        let progress = prog.progress_pct();

        // Build timeline chart
        let periods: Vec<TimelinePeriod> = prog
            .schedule
            .iter()
            .map(|p| TimelinePeriod {
                label: format!("{}Q{}", p.year, p.quarter),
                target: p.shares_target,
                executed: p.executed_shares,
                color: "#51cf66".to_string(),
            })
            .collect();
        let chart = timeline_bar_svg(&periods, 600, (periods.len() as u32 * 40).max(100));

        // Schedule table
        let mut schedule_rows = String::new();
        for p in &prog.schedule {
            schedule_rows.push_str(&format!(
                "<tr><td>{}Q{}</td><td>{:.0}</td><td>{:.0}</td><td>{:.0}</td></tr>",
                p.year,
                p.quarter,
                p.shares_target,
                p.executed_shares,
                p.shares_target - p.executed_shares,
            ));
        }

        sections.push_str(&format!(
            r#"<article>
<h2>{ticker} &mdash; Programme #{prog_id}</h2>
<p>Progress: {progress:.1}% &mdash; Remaining: {remaining:.0} shares</p>
<div class="chart-container">{chart}</div>
<table><caption class="sr-only">Selling programme schedule: target, executed and remaining shares</caption>
  <thead><tr><th scope="col">Period</th><th scope="col">Target</th><th scope="col">Executed</th><th scope="col">Remaining</th></tr></thead>
  <tbody>{schedule_rows}</tbody>
</table>
</article>"#,
            remaining = prog.remaining_shares(),
        ));
    }

    Html(shell("/selling", "Selling Programme", &sections))
}

// ─── 8. Decision Log ───────────────────────────────────────────────────────

pub async fn decisions_page(State(state): State<AppState>) -> Html<String> {
    let repo = state.repo.clone();
    let decisions = tokio::task::spawn_blocking(move || repo.get_decisions(100))
        .await
        .unwrap_or_else(|_| Ok(Vec::new()))
        .unwrap_or_default();

    if decisions.is_empty() {
        let body = r#"<p class="empty-state">No decisions logged.</p>"#;
        return Html(shell("/decisions", "Decision Log", body));
    }

    let mut rows = String::new();
    for d in &decisions {
        let date = html_escape(&d.decision_date.to_string());
        let dtype = html_escape(&d.decision_type.to_string());
        let ticker = d
            .ticker
            .as_deref()
            .map(html_escape)
            .unwrap_or_else(|| "&mdash;".into());
        let shares = d
            .shares
            .map(|s| format!("{s:.2}"))
            .unwrap_or_else(|| "&mdash;".into());
        let price = d
            .price
            .map(|p| format!("${p:.2}"))
            .unwrap_or_else(|| "&mdash;".into());
        let rationale = html_escape(&d.rationale);
        let advisor = d
            .advisor
            .as_deref()
            .map(html_escape)
            .unwrap_or_else(|| "&mdash;".into());

        rows.push_str(&format!(
            "<tr><td>{date}</td><td>{dtype}</td><td>{ticker}</td><td>{shares}</td>\
             <td>{price}</td><td>{rationale}</td><td>{advisor}</td></tr>"
        ));
    }

    let body = format!(
        r#"<div style="overflow-x: auto;">
<table><caption class="sr-only">Logged trade decisions with price in USD and rationale</caption>
  <thead><tr><th scope="col">Date</th><th scope="col">Type</th><th scope="col">Ticker</th><th scope="col">Shares</th><th scope="col">Price</th><th scope="col">Rationale</th><th scope="col">Advisor</th></tr></thead>
  <tbody>{rows}</tbody>
</table>
</div>"#
    );
    Html(shell("/decisions", "Decision Log", &body))
}

// ─── 9. Fundamentals ───────────────────────────────────────────────────────

pub async fn fundamentals_page(State(state): State<AppState>) -> Html<String> {
    let repo = state.repo.clone();
    let repo2 = state.repo.clone();

    let metrics = tokio::task::spawn_blocking(move || repo.get_all_fundamental_metrics())
        .await
        .unwrap_or_else(|_| Ok(Vec::new()))
        .unwrap_or_default();

    let sectors = tokio::task::spawn_blocking(move || repo2.get_sector_pe_benchmarks())
        .await
        .unwrap_or_else(|_| Ok(Vec::new()))
        .unwrap_or_default();

    if metrics.is_empty() && sectors.is_empty() {
        let body = r#"<p class="empty-state">No fundamental data available.</p>"#;
        return Html(shell("/fundamentals", "Fundamentals", body));
    }

    // Metrics table
    let mut metric_rows = String::new();
    for m in &metrics {
        let ticker = html_escape(&m.ticker);
        let date = html_escape(&m.fetched_date);
        metric_rows.push_str(&format!(
            "<tr><td>{ticker}</td><td>{date}</td><td>{pe}</td><td>{pb}</td>\
             <td>{eps_g}</td><td>{rev_g}</td><td>{gm}</td><td>{nm}</td>\
             <td>{roe}</td><td>{score}</td></tr>",
            pe = fmt_opt(m.pe_normalized, 1),
            pb = fmt_opt(m.pb_annual, 2),
            eps_g = fmt_opt(m.eps_growth_ttm_yoy.map(|v| v * 100.0), 1),
            rev_g = fmt_opt(m.rev_growth_ttm_yoy.map(|v| v * 100.0), 1),
            gm = fmt_opt(m.gross_margin_ttm.map(|v| v * 100.0), 1),
            nm = fmt_opt(m.net_margin_ttm.map(|v| v * 100.0), 1),
            roe = fmt_opt(m.roe_ttm.map(|v| v * 100.0), 1),
            score = fmt_opt(m.fundamental_score, 1),
        ));
    }

    let metrics_section = if metric_rows.is_empty() {
        String::new()
    } else {
        format!(
            r#"<h2>Fundamental Metrics</h2>
<div style="overflow-x: auto;">
<table><caption class="sr-only">Fundamental ratios per ticker, growth and margins in percent</caption>
  <thead><tr><th scope="col">Ticker</th><th scope="col">Date</th><th scope="col">P/E</th><th scope="col">P/B</th><th scope="col">EPS Gr%</th><th scope="col">Rev Gr%</th><th scope="col">Gross %</th><th scope="col">Net %</th><th scope="col">ROE %</th><th scope="col">Score</th></tr></thead>
  <tbody>{metric_rows}</tbody>
</table>
</div>"#
        )
    };

    // Sector benchmarks table
    let mut sector_rows = String::new();
    for s in &sectors {
        let sector = html_escape(&s.sector);
        let date = html_escape(&s.snapshot_date);
        sector_rows.push_str(&format!(
            "<tr><td>{sector}</td><td>{date}</td><td>{pe}</td><td>{ps}</td><td>{count}</td></tr>",
            pe = fmt_opt(s.median_pe, 1),
            ps = fmt_opt(s.median_ps, 2),
            count = s.stock_count,
        ));
    }

    let sector_section = if sector_rows.is_empty() {
        String::new()
    } else {
        format!(
            r#"<h2>Sector P/E Benchmarks</h2>
<table><caption class="sr-only">Median valuation multiples by sector</caption>
  <thead><tr><th scope="col">Sector</th><th scope="col">Date</th><th scope="col">Median P/E</th><th scope="col">Median P/S</th><th scope="col">Stocks</th></tr></thead>
  <tbody>{sector_rows}</tbody>
</table>"#
        )
    };

    let body = format!(
        r#"{metrics_section}
{sector_section}"#
    );
    Html(shell("/fundamentals", "Fundamentals", &body))
}

// ─── 10. Concentration Decay ─────────────────────────────────────────────────

/// Query params for concentration decay page.
#[derive(Deserialize)]
pub struct ConcentrationDecayParams {
    pub ticker: Option<String>,
    pub shares: Option<f64>,
    pub price: Option<f64>,
    pub other_value: Option<f64>,
    pub years: Option<u32>,
    pub start_year: Option<u32>,
}

pub async fn concentration_decay_page(
    Query(params): Query<ConcentrationDecayParams>,
) -> Html<String> {
    use crate::portfolio::selling::SellingProgramme;
    use crate::web::charts::{concentration_decay_points, concentration_decay_svg};

    let ticker = params.ticker.unwrap_or_else(|| "AMZN".into());
    let shares = params.shares.unwrap_or(1000.0);
    let price = params.price.unwrap_or(200.0);
    let other_value = params.other_value.unwrap_or(50_000.0);
    let years = params.years.unwrap_or(2);
    let start_year = params.start_year.unwrap_or_else(|| {
        use chrono::Datelike;
        chrono::Utc::now().naive_utc().date().year() as u32
    });

    let per_year = shares / years as f64;
    let annual_shares: Vec<f64> = vec![per_year; years as usize];
    let programme = SellingProgramme::build_schedule(&ticker, &annual_shares, years);

    let total_value = shares * price + other_value;
    let starting_weight = shares * price / total_value;

    let points = concentration_decay_points(
        starting_weight,
        shares,
        &programme,
        price,
        other_value,
        start_year,
    );
    let chart = concentration_decay_svg(&points, 700, 350);

    let first_vol = points
        .first()
        .map(|p| p.estimated_annual_vol)
        .unwrap_or(0.0);
    let last_vol = points.last().map(|p| p.estimated_annual_vol).unwrap_or(0.0);
    let vol_reduction = if first_vol > 0.0 {
        ((first_vol - last_vol) / first_vol * 100.0).max(0.0)
    } else {
        0.0
    };

    let body = format!(
        r#"<h2>Concentration Decay — {ticker}</h2>
<p>Projected portfolio volatility reduction as concentration decreases over {years} year(s).</p>
<div style="display:grid;grid-template-columns:repeat(3,1fr);gap:12px;margin-bottom:20px">
  <div><strong>Starting weight:</strong> {:.1}%</div>
  <div><strong>Starting vol:</strong> {:.1}%</div>
  <div><strong>Vol reduction:</strong> {:.1}%</div>
</div>
<div class="chart-container">{chart}</div>
<details><summary>Quarterly breakdown</summary><table><caption class="sr-only">Quarterly breakdown of weight, estimated volatility and shares left</caption>
<thead><tr><th scope="col">Quarter</th><th scope="col">Weight</th><th scope="col">Est. Vol</th><th scope="col">Shares Left</th></tr></thead>
<tbody>{rows}</tbody></table></details>"#,
        starting_weight * 100.0,
        first_vol,
        vol_reduction,
        rows = points
            .iter()
            .map(|p| format!(
                "<tr><td>{}</td><td>{:.1}%</td><td>{:.1}%</td><td>{:.0}</td></tr>",
                p.quarter_label,
                p.concentrated_weight * 100.0,
                p.estimated_annual_vol,
                p.shares_remaining
            ))
            .collect::<String>(),
    );

    Html(shell("/concentration-decay", "Concentration Decay", &body))
}

// ─── 11. Allocation ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct AllocationParams {
    pub brokerage: Option<String>,
    pub profile: Option<String>,
    pub portfolio_value: Option<f64>,
}

pub async fn allocation_page(Query(params): Query<AllocationParams>) -> Html<String> {
    use crate::portfolio::allocation::{
        blended_expense_ratio, brokerage_catalog, map_to_funds, risk_profiles, Brokerage,
    };

    let brokerage = match params.brokerage.as_deref() {
        Some("schwab") => Brokerage::Schwab,
        Some("vanguard") => Brokerage::Vanguard,
        _ => Brokerage::Fidelity,
    };

    let profiles = risk_profiles();
    let profile_name = params.profile.as_deref().unwrap_or("Moderate");
    let profile = profiles
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(profile_name))
        .unwrap_or(&profiles[1]); // default to Moderate

    let portfolio_value = params.portfolio_value.unwrap_or(500_000.0);

    let allocations = map_to_funds(&profile.allocations, &brokerage, portfolio_value);
    let blended_er = blended_expense_ratio(&allocations);

    let catalog = brokerage_catalog(&brokerage);

    let rows: String = allocations.iter().map(|a| {
        let tlh = a.fund.tlh_partner.as_deref().unwrap_or("—");
        let transfer = if a.fund.transferable { "Yes" } else { "No" };
        format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:.3}%</td><td>${:.0}</td><td>{}</td><td>{}</td></tr>",
            a.asset_class, a.fund.ticker, a.fund.name,
            a.fund.expense_ratio * 100.0,
            a.dollar_amount, tlh, transfer,
        )
    }).collect();

    let body = format!(
        r#"<h2>Target Allocation — {profile_name}</h2>
<p>{desc}</p>
<p><strong>Brokerage:</strong> {brok} | <strong>Portfolio Value:</strong> ${pv:.0} | <strong>Blended ER:</strong> {er:.4}%</p>
<table><caption class="sr-only">Target allocation by asset class and fund, amounts in USD</caption>
<thead><tr><th scope="col">Asset Class</th><th scope="col">Ticker</th><th scope="col">Fund</th><th scope="col">ER</th><th scope="col">Amount</th><th scope="col">TLH Partner</th><th scope="col">Transferable</th></tr></thead>
<tbody>{rows}</tbody>
</table>
<p style="color:#888;font-size:0.85em">Try: <code>/allocation?brokerage=schwab&amp;profile=aggressive&amp;portfolio_value=1000000</code></p>"#,
        desc = profile.description,
        brok = catalog.brokerage,
        pv = portfolio_value,
        er = blended_er * 100.0,
    );

    Html(shell("/allocation", "Target Allocation", &body))
}

// ─── 12. Geopolitical Risk ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct GeoRiskParams {
    pub oil: Option<f64>,
    pub vix: Option<f64>,
    pub gold_3m: Option<f64>,
    pub dxy_3m: Option<f64>,
}

pub async fn geo_risk_page(Query(params): Query<GeoRiskParams>) -> Html<String> {
    use crate::analysis::geopolitical::{
        compute_geo_risk, geo_allocation_adjustments, GeoRiskInputs,
    };

    let inputs = GeoRiskInputs {
        oil_brent: params.oil.unwrap_or(75.0),
        vix: params.vix.unwrap_or(15.0),
        gold_3m_change: params.gold_3m.unwrap_or(0.02),
        dxy_3m_change: params.dxy_3m.unwrap_or(0.01),
    };

    let score = compute_geo_risk(&inputs);
    let adjustments = geo_allocation_adjustments(&score.level);

    let level_color = match &score.level {
        crate::analysis::geopolitical::GeoRiskLevel::Low => "#51cf66",
        crate::analysis::geopolitical::GeoRiskLevel::Moderate => "#ffd43b",
        crate::analysis::geopolitical::GeoRiskLevel::High => "#ff922b",
        crate::analysis::geopolitical::GeoRiskLevel::Extreme => "#ff6b6b",
    };

    let adj_rows: String = if adjustments.is_empty() {
        "<tr><td colspan='2'>No adjustments — risk is low.</td></tr>".into()
    } else {
        adjustments
            .iter()
            .map(|(sector, delta)| {
                let sign = if *delta > 0.0 { "+" } else { "" };
                format!(
                    "<tr><td>{}</td><td>{}{:.0}%</td></tr>",
                    sector,
                    sign,
                    delta * 100.0
                )
            })
            .collect()
    };

    let body = format!(
        r#"<div style="display:grid;grid-template-columns:repeat(4,1fr);gap:12px;margin-bottom:20px">
  <div><strong>Oil (Brent):</strong> ${:.0}/bbl</div>
  <div><strong>VIX:</strong> {:.1}</div>
  <div><strong>Gold 3m:</strong> {:.1}%</div>
  <div><strong>DXY 3m:</strong> {:.1}%</div>
</div>
<div style="display:grid;grid-template-columns:repeat(4,1fr);gap:12px;margin-bottom:20px">
  <div>Oil: <strong>{:.2}</strong>/2.5</div>
  <div>VIX: <strong>{:.2}</strong>/2.5</div>
  <div>Gold: <strong>{:.2}</strong>/2.5</div>
  <div>DXY: <strong>{:.2}</strong>/2.5</div>
</div>
<p><strong>Composite Score:</strong> <span style="font-size:1.5em;color:{level_color}">{:.1}/10</span>
   — <strong style="color:{level_color}">{}</strong></p>
<h2>Suggested Allocation Adjustments</h2>
<table><caption class="sr-only">Suggested allocation adjustments by sector</caption>
<thead><tr><th scope="col">Sector</th><th scope="col">Adjustment</th></tr></thead>
<tbody>{adj_rows}</tbody>
</table>
<p style="color:#888;font-size:0.85em">Try: <code>/geo-risk?oil=130&amp;vix=32&amp;gold_3m=0.15&amp;dxy_3m=-0.05</code></p>"#,
        inputs.oil_brent,
        inputs.vix,
        inputs.gold_3m_change * 100.0,
        inputs.dxy_3m_change * 100.0,
        score.oil_component,
        score.vix_component,
        score.gold_component,
        score.dxy_component,
        score.composite,
        score.level,
    );

    Html(shell("/geo-risk", "Geopolitical Risk Assessment", &body))
}

// ─── Rotation Page ──────────────────────────────────────────────────────────

pub async fn rotation_page() -> Html<String> {
    use crate::data::models::MarketRegime;
    use crate::portfolio::rotation::{
        compute_deployment_score, compute_sell_pressure, CdsInput, SpsAction, SpsInput,
    };

    let regime = MarketRegime::Sideways;

    let positions: Vec<(&str, SpsInput)> = vec![
        (
            "GOOGL",
            SpsInput {
                conditions_met: 8,
                conditions_at_entry: 8,
                conditions_lost_5d: 0,
                composite_now: 88.0,
                composite_5d_ago: 86.0,
                composite_20d_ago: 82.0,
                up_day_avg_vol: 1e6,
                down_day_avg_vol: 9e5,
                rs_percentile: 90.0,
                rs_20d_ago: 87.0,
                current_close: 195.0,
                local_high_60d: 200.0,
                days_since_new_high: 8,
                actual_weight: 0.14,
                target_weight: 0.125,
                hard_exit_triggered: false,
            },
        ),
        (
            "AMD",
            SpsInput {
                conditions_met: 8,
                conditions_at_entry: 8,
                conditions_lost_5d: 0,
                composite_now: 91.0,
                composite_5d_ago: 89.0,
                composite_20d_ago: 85.0,
                up_day_avg_vol: 1e6,
                down_day_avg_vol: 9e5,
                rs_percentile: 95.0,
                rs_20d_ago: 92.0,
                current_close: 142.0,
                local_high_60d: 148.0,
                days_since_new_high: 12,
                actual_weight: 0.12,
                target_weight: 0.125,
                hard_exit_triggered: false,
            },
        ),
        (
            "NVDA",
            SpsInput {
                conditions_met: 7,
                conditions_at_entry: 8,
                conditions_lost_5d: 1,
                composite_now: 78.0,
                composite_5d_ago: 80.0,
                composite_20d_ago: 82.0,
                up_day_avg_vol: 1e6,
                down_day_avg_vol: 9e5,
                rs_percentile: 70.0,
                rs_20d_ago: 75.0,
                current_close: 178.0,
                local_high_60d: 195.0,
                days_since_new_high: 25,
                actual_weight: 0.15,
                target_weight: 0.125,
                hard_exit_triggered: false,
            },
        ),
        (
            "PLTR",
            SpsInput {
                conditions_met: 8,
                conditions_at_entry: 8,
                conditions_lost_5d: 0,
                composite_now: 83.0,
                composite_5d_ago: 81.0,
                composite_20d_ago: 78.0,
                up_day_avg_vol: 1e6,
                down_day_avg_vol: 9e5,
                rs_percentile: 85.0,
                rs_20d_ago: 83.0,
                current_close: 151.0,
                local_high_60d: 155.0,
                days_since_new_high: 5,
                actual_weight: 0.13,
                target_weight: 0.125,
                hard_exit_triggered: false,
            },
        ),
        (
            "DDOG",
            SpsInput {
                conditions_met: 6,
                conditions_at_entry: 8,
                conditions_lost_5d: 2,
                composite_now: 62.0,
                composite_5d_ago: 68.0,
                composite_20d_ago: 72.0,
                up_day_avg_vol: 5e5,
                down_day_avg_vol: 12e5,
                rs_percentile: 60.0,
                rs_20d_ago: 68.0,
                current_close: 108.0,
                local_high_60d: 125.0,
                days_since_new_high: 50,
                actual_weight: 0.07,
                target_weight: 0.125,
                hard_exit_triggered: false,
            },
        ),
        (
            "AAPL",
            SpsInput {
                conditions_met: 6,
                conditions_at_entry: 8,
                conditions_lost_5d: 0,
                composite_now: 60.0,
                composite_5d_ago: 63.0,
                composite_20d_ago: 65.0,
                up_day_avg_vol: 1e6,
                down_day_avg_vol: 9e5,
                rs_percentile: 55.0,
                rs_20d_ago: 60.0,
                current_close: 220.0,
                local_high_60d: 235.0,
                days_since_new_high: 45,
                actual_weight: 0.09,
                target_weight: 0.125,
                hard_exit_triggered: false,
            },
        ),
    ];

    let mut rows = String::new();
    for (ticker, input) in &positions {
        let result = compute_sell_pressure(ticker, input, &regime);
        let (action_label, action_color) = match &result.action {
            SpsAction::Green => ("GREEN", "#51cf66"),
            SpsAction::Yellow => ("YELLOW", "#ffd43b"),
            SpsAction::Orange => ("ORANGE", "#ff922b"),
            SpsAction::Red => ("RED", "#ff6b6b"),
            SpsAction::Critical => ("CRIT", "#e03131"),
        };
        rows.push_str(&format!(
            "<tr><td><strong>{}</strong></td><td>{:.1}</td><td><span style='background:{};color:#000;padding:2px 8px;border-radius:4px;font-size:0.8em'>{}</span></td><td>{:.0}</td><td>{:.0}</td><td>{:.0}</td><td>{:.0}</td><td>{:.0}</td></tr>",
            ticker, result.sps, action_color, action_label,
            result.components.trend_decay, result.components.momentum_fade,
            result.components.drawdown_penalty, result.components.relative_weakness,
            result.components.time_decay
        ));
    }

    let cds = compute_deployment_score(&CdsInput {
        candidates_above_70: 5,
        top_candidate_score: 82.0,
        regime: regime.clone(),
        regime_improving: false,
        regime_deteriorating: false,
        cash_pct: 0.31,
        days_in_cash: 14,
        vix_percentile: 45.0,
        breadth_pct: 55.0,
    });

    let body = format!(
        r#"
<p>Regime: <strong>Sideways</strong> | Target weight: 12.5% per position</p>

<h2>Sell Pressure Scores</h2>
<table><caption class="sr-only">Sell pressure scores per ticker with component breakdown</caption>
<thead><tr><th scope="col">Ticker</th><th scope="col">SPS</th><th scope="col">Action</th><th scope="col">Trend</th><th scope="col">Momentum</th><th scope="col">Drawdown</th><th scope="col">RS Fade</th><th scope="col">Time Decay</th></tr></thead>
<tbody>{rows}</tbody>
</table>

<h2>Cash Deployment</h2>
<table><caption class="sr-only">Cash deployment scores by component</caption>
<tr><td>CDS Score</td><td><strong>{:.1}</strong></td><td>{:?}</td></tr>
<tr><td>Opportunity Richness</td><td>{:.0}</td><td></td></tr>
<tr><td>Regime Score</td><td>{:.0}</td><td></td></tr>
<tr><td>Cash Drag</td><td>{:.0}</td><td></td></tr>
<tr><td>Drawdown Risk</td><td>{:.0}</td><td></td></tr>
</table>

<p><em>SPS thresholds: GREEN (0-25) YELLOW (26-45) ORANGE (46-65) RED (66-80) CRITICAL (81+)</em></p>
"#,
        cds.cds,
        cds.action,
        cds.opportunity_richness,
        cds.regime_score,
        cds.cash_drag_cost,
        cds.drawdown_risk
    );

    Html(shell("/rotation", "Capital Rotation Analysis", &body))
}

// ─── Sizing Page ────────────────────────────────────────────────────────────

pub async fn sizing_page() -> Html<String> {
    use crate::analysis::scorer::Grade;
    use crate::data::models::MarketRegime;
    use crate::portfolio::sizing::{compute_position_size, SizingInput, SizingMethod};

    let portfolio = 150_000.0;
    let method = SizingMethod::VolatilityTarget {
        stop_multiplier: 2.0,
    };
    let risk_pct = 0.01;
    let max_pct = 0.15;

    let candidates = vec![
        ("NVDA", 180.0, 5.5, Grade::APlus, MarketRegime::Sideways),
        ("GOOGL", 190.0, 4.2, Grade::APlus, MarketRegime::Sideways),
        ("AMD", 140.0, 6.0, Grade::A, MarketRegime::Sideways),
        ("CRWD", 385.0, 8.5, Grade::A, MarketRegime::Sideways),
        ("PLTR", 85.0, 3.8, Grade::A, MarketRegime::Sideways),
        ("PANW", 420.0, 12.0, Grade::B, MarketRegime::Sideways),
    ];

    let mut rows = String::new();
    for (ticker, price, atr, grade, regime) in &candidates {
        let input = SizingInput {
            ticker: ticker.to_string(),
            close_price: *price,
            atr_14: *atr,
            grade: grade.clone(),
            regime: regime.clone(),
        };
        let result = compute_position_size(&method, &input, portfolio, risk_pct, max_pct);
        let capped = if result.was_capped { " *" } else { "" };
        rows.push_str(&format!(
            "<tr><td><strong>{}</strong></td><td>{}</td><td>${:.0}</td><td>${:.2}</td><td>{:.0}{}</td><td>${:.0}</td><td>${:.0}</td><td>{:.1}%</td></tr>",
            ticker, grade, price, result.stop_loss, result.shares, capped,
            result.dollar_amount, result.risk_amount, result.pct_of_portfolio * 100.0
        ));
    }

    let body = format!(
        r#"
<p>Portfolio: <strong>${:.0}</strong> | Method: VolatilityTarget (2x ATR) | Risk: 1% | Max: 15%</p>

<table><caption class="sr-only">Recommended position sizes: shares, amount and risk in USD</caption>
<thead><tr><th scope="col">Ticker</th><th scope="col">Grade</th><th scope="col">Price</th><th scope="col">Stop</th><th scope="col">Shares</th><th scope="col">Amount</th><th scope="col">Risk $</th><th scope="col">% of Port</th></tr></thead>
<tbody>{rows}</tbody>
</table>

<p><em>* = position capped at max percentage. Grade multipliers: A+ = 1.5x, A = 1.0x, B = 0.7x. Regime: Sideways = 0.8x.</em></p>

<h2>Sizing Methods</h2>
<table><caption class="sr-only">Available sizing methods and how each works</caption>
<tr><td><strong>VolatilityTarget</strong> (default)</td><td>Shares = RiskBudget / (ATR x StopMult). Smaller positions in volatile stocks.</td></tr>
<tr><td><strong>EqualWeight</strong></td><td>Portfolio / N positions. Simple 1/N allocation.</td></tr>
<tr><td><strong>FixedFractional</strong></td><td>Risk X% of portfolio per trade. Stop determines share count.</td></tr>
<tr><td><strong>HalfKelly</strong></td><td>Kelly criterion / 2 for safety. Requires backtest win rate.</td></tr>
</table>
"#,
        portfolio
    );

    Html(shell("/sizing", "Position Sizing", &body))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use std::sync::Arc;
    use tower::ServiceExt;

    fn test_router() -> (axum::Router, Arc<crate::storage::db::SqliteRepository>) {
        let state = crate::web::tests::make_state();
        let repo = state.repo.clone();
        let router = axum::Router::new()
            .route("/portfolio", axum::routing::get(portfolio_page))
            .route("/correlation", axum::routing::get(correlation_page))
            .route("/valuation", axum::routing::get(valuation_page))
            .route("/tax", axum::routing::get(tax_page))
            .route("/optimize", axum::routing::get(optimize_page))
            .route("/etf", axum::routing::get(etf_page))
            .route("/selling", axum::routing::get(selling_page))
            .route("/decisions", axum::routing::get(decisions_page))
            .route("/fundamentals", axum::routing::get(fundamentals_page))
            .route(
                "/concentration-decay",
                axum::routing::get(concentration_decay_page),
            )
            .route("/allocation", axum::routing::get(allocation_page))
            .route("/geo-risk", axum::routing::get(geo_risk_page))
            .with_state(state);
        (router, repo)
    }

    async fn get_page(router: &axum::Router, path: &str) -> (StatusCode, String) {
        let req = Request::builder()
            .uri(path)
            .body(Body::empty())
            .expect("request should build");
        let resp = router.clone().oneshot(req).await.expect("request failed");
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body collect failed");
        let body = String::from_utf8_lossy(&bytes).to_string();
        (status, body)
    }

    // ── HTML escape tests ───────────────────────────────────────────────────

    #[test]
    fn html_escape_handles_special_chars() {
        assert_eq!(
            html_escape(r#"<script>alert("xss")</script>"#),
            "&lt;script&gt;alert(&quot;xss&quot;)&lt;/script&gt;"
        );
        assert_eq!(html_escape("a&b"), "a&amp;b");
        assert_eq!(html_escape("it's"), "it&#39;s");
    }

    // ── Helper utility tests ────────────────────────────────────────────────

    #[test]
    fn fmt_opt_renders_values_and_dashes() {
        assert_eq!(fmt_opt(Some(12.345), 2), "12.35");
        assert_eq!(fmt_opt(None, 2), "&mdash;");
        assert_eq!(fmt_opt(Some(f64::NAN), 1), "&mdash;");
    }

    #[test]
    fn fmt_pct_renders_with_sign() {
        assert_eq!(fmt_pct(5.0), "+5.00%");
        assert_eq!(fmt_pct(-3.2), "-3.20%");
        assert_eq!(fmt_pct(f64::NAN), "&mdash;");
    }

    #[test]
    fn pl_class_returns_correct_class() {
        assert_eq!(pl_class(10.0), "gain");
        assert_eq!(pl_class(-5.0), "loss");
        assert_eq!(pl_class(0.0), "gain");
    }

    // ── Empty state page tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn portfolio_empty_state() {
        let (router, _) = test_router();
        let (status, body) = get_page(&router, "/portfolio").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Portfolio Overview"));
        assert!(body.contains("empty-state"));
        assert!(body.contains("ghost-alpha portfolio import"));
    }

    #[tokio::test]
    async fn correlation_empty_state() {
        let (router, _) = test_router();
        let (status, body) = get_page(&router, "/correlation").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Correlation Matrix"));
        assert!(body.contains("empty-state"));
        assert!(body.contains("ghost-alpha correlate"));
    }

    #[tokio::test]
    async fn valuation_empty_state_no_ticker() {
        let (router, _) = test_router();
        let (status, body) = get_page(&router, "/valuation").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Valuation Dashboard"));
        assert!(body.contains("empty-state"));
    }

    #[tokio::test]
    async fn valuation_empty_state_with_unknown_ticker() {
        let (router, _) = test_router();
        let (status, body) = get_page(&router, "/valuation?ticker=ZZZZZ").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("ZZZZZ"));
        assert!(body.contains("empty-state"));
    }

    #[tokio::test]
    async fn tax_empty_state() {
        let (router, _) = test_router();
        let (status, body) = get_page(&router, "/tax").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Tax Overview"));
        assert!(body.contains("empty-state"));
        assert!(body.contains("No tax lots imported"));
    }

    #[tokio::test]
    async fn optimize_empty_state_still_shows_models() {
        let (router, _) = test_router();
        let (status, body) = get_page(&router, "/optimize").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Portfolio Optimization"));
        assert!(body.contains("Model Portfolios"));
        assert!(body.contains("Conservative"));
        assert!(body.contains("Moderate"));
        assert!(body.contains("Growth"));
        // No frontier since no correlation data
        assert!(body.contains("ghost-alpha correlate"));
    }

    #[tokio::test]
    async fn etf_empty_state() {
        let (router, _) = test_router();
        let (status, body) = get_page(&router, "/etf").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("ETF Comparison"));
        assert!(body.contains("empty-state"));
        assert!(body.contains("ghost-alpha etf-sync"));
    }

    #[tokio::test]
    async fn selling_empty_state() {
        let (router, _) = test_router();
        let (status, body) = get_page(&router, "/selling").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Selling Programme"));
        assert!(body.contains("empty-state"));
    }

    #[tokio::test]
    async fn decisions_empty_state() {
        let (router, _) = test_router();
        let (status, body) = get_page(&router, "/decisions").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Decision Log"));
        assert!(body.contains("empty-state"));
        assert!(body.contains("No decisions logged"));
    }

    #[tokio::test]
    async fn fundamentals_empty_state() {
        let (router, _) = test_router();
        let (status, body) = get_page(&router, "/fundamentals").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Fundamentals"));
        assert!(body.contains("empty-state"));
    }

    // ── Data-seeded tests ───────────────────────────────────────────────────

    #[tokio::test]
    async fn portfolio_with_data_shows_table_and_chart() {
        let (router, repo) = test_router();

        // Seed tax lots via raw SQL
        {
            let conn = repo.connection().unwrap();
            conn.execute_batch(
                "INSERT INTO tax_lots (id, ticker, shares, cost_basis_per_share, acquired_date, acquisition_type, remaining_shares)
                 VALUES ('L001', 'AAPL', 100, 150.00, '2023-01-15', 'purchase', 100),
                        ('L002', 'MSFT', 50, 280.00, '2023-03-01', 'purchase', 50),
                        ('L003', 'AAPL', 25, 160.00, '2023-06-10', 'purchase', 25);",
            ).expect("seed should succeed");
        }

        let (status, body) = get_page(&router, "/portfolio").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Portfolio Overview"));
        assert!(body.contains("AAPL"));
        assert!(body.contains("MSFT"));
        assert!(body.contains("<table"));
        assert!(body.contains("HHI:"));
        assert!(body.contains("<svg")); // donut chart
    }

    #[tokio::test]
    async fn correlation_with_data_shows_heatmap() {
        let (router, repo) = test_router();

        // Seed correlation snapshot
        repo.save_correlation_snapshot(
            "2025-01-15",
            &["AAPL".into(), "MSFT".into(), "GOOG".into()],
            &[
                vec![1.0, 0.85, 0.6],
                vec![0.85, 1.0, 0.72],
                vec![0.6, 0.72, 1.0],
            ],
            60,
        )
        .expect("seed should succeed");

        let (status, body) = get_page(&router, "/correlation").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Correlation Matrix"));
        assert!(body.contains("2025-01-15"));
        assert!(body.contains("60 days"));
        assert!(body.contains("<svg")); // heatmap
                                        // Should show AAPL-MSFT as high pair
        assert!(body.contains("AAPL"));
        assert!(body.contains("MSFT"));
        assert!(body.contains("0.850"));
    }

    #[tokio::test]
    async fn tax_with_data_shows_detail_table() {
        let (router, repo) = test_router();

        {
            let conn = repo.connection().unwrap();
            conn.execute_batch(
                "INSERT INTO tax_lots (id, ticker, shares, cost_basis_per_share, acquired_date, acquisition_type, remaining_shares)
                 VALUES ('T001', 'TSLA', 10, 200.00, '2024-01-10', 'purchase', 10);",
            ).expect("seed should succeed");
        }

        let (status, body) = get_page(&router, "/tax").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Tax Overview"));
        assert!(body.contains("T001"));
        assert!(body.contains("TSLA"));
        assert!(body.contains("$200.00")); // cost per share
        assert!(body.contains("$2000.00")); // total cost
        assert!(body.contains("Lot ID"));
    }

    #[tokio::test]
    async fn decisions_with_data_shows_rows() {
        let (router, repo) = test_router();

        use crate::portfolio::decision::{DecisionLogEntry, DecisionType};
        use chrono::NaiveDate;

        let entry = DecisionLogEntry {
            id: None,
            decision_date: NaiveDate::from_ymd_opt(2025, 3, 1).expect("valid date"),
            decision_type: DecisionType::Sell,
            ticker: Some("AMZN".into()),
            shares: Some(50.0),
            price: Some(185.50),
            rationale: "Concentration too high".into(),
            advisor: Some("Alice".into()),
            linked_alert_id: None,
            created_at: None,
        };
        repo.record_decision(&entry).expect("seed should succeed");

        let (status, body) = get_page(&router, "/decisions").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Decision Log"));
        assert!(body.contains("2025-03-01"));
        assert!(body.contains("SELL"));
        assert!(body.contains("AMZN"));
        assert!(body.contains("50.00"));
        assert!(body.contains("$185.50"));
        assert!(body.contains("Concentration too high"));
        assert!(body.contains("Alice"));
    }

    #[tokio::test]
    async fn etf_with_data_shows_table() {
        let (router, repo) = test_router();

        use crate::data::etf::{EtfHolding, EtfSnapshot};

        let snap = EtfSnapshot {
            ticker: "VTI".into(),
            name: "Vanguard Total Stock Market".into(),
            expense_ratio: 0.0003,
            aum: 350_000_000_000.0,
            avg_daily_volume: 3_000_000,
            category: "US Total Market".into(),
            inception_date: "2001-05-24".into(),
            tracking_index: "CRSP US Total Market".into(),
            top_holdings: vec![
                EtfHolding {
                    ticker: "AAPL".into(),
                    name: "Apple Inc".into(),
                    weight: 0.065,
                },
                EtfHolding {
                    ticker: "MSFT".into(),
                    name: "Microsoft Corp".into(),
                    weight: 0.058,
                },
            ],
        };
        repo.upsert_etf_snapshot(&snap)
            .expect("seed should succeed");

        let (status, body) = get_page(&router, "/etf").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("ETF Comparison"));
        assert!(body.contains("VTI"));
        assert!(body.contains("Vanguard Total Stock Market"));
        assert!(body.contains("0.03%")); // expense ratio
        assert!(body.contains("$350.0B")); // AUM
        assert!(body.contains("AAPL"));
    }

    #[tokio::test]
    async fn fundamentals_with_metric_data() {
        let (router, repo) = test_router();

        {
            let conn = repo.connection().unwrap();
            conn.execute_batch(
                "INSERT INTO fundamental_metrics (ticker, fetched_date, pe_normalized, pb_annual, gross_margin_ttm, net_margin_ttm, roe_ttm, fundamental_score)
                 VALUES ('NVDA', '2025-03-01', 35.2, 28.5, 0.74, 0.55, 1.15, 82.0);",
            ).expect("seed should succeed");
        }

        let (status, body) = get_page(&router, "/fundamentals").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Fundamentals"));
        assert!(body.contains("NVDA"));
        assert!(body.contains("35.2")); // P/E
        assert!(body.contains("82.0")); // score
        assert!(body.contains("Fundamental Metrics"));
    }

    // ── All pages return 200 ────────────────────────────────────────────────

    #[tokio::test]
    async fn all_pages_return_200() {
        let (router, _) = test_router();
        let pages = [
            "/portfolio",
            "/correlation",
            "/valuation",
            "/valuation?ticker=TEST",
            "/tax",
            "/optimize",
            "/etf",
            "/selling",
            "/decisions",
            "/fundamentals",
            "/concentration-decay",
            "/allocation",
            "/geo-risk",
        ];
        for path in pages {
            let (status, body) = get_page(&router, path).await;
            assert_eq!(status, StatusCode::OK, "page {path} returned {status}");
            assert!(
                body.contains("<!DOCTYPE html>"),
                "page {path} missing doctype"
            );
            assert!(
                body.contains("Ghost Alpha"),
                "page {path} missing nav brand"
            );
        }
    }
}
