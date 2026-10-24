//! Self-contained HTML backtest report generator.
//!
//! Produces a single `.html` file with embedded CSS and inline SVG charts —
//! no external dependencies, no JavaScript required.

use chrono::Local;

use crate::backtest::types::{
    BacktestReport, ConfidenceInterval, DrawdownEvent, WalkForwardPeriod,
};

// ─── Public entry point ───────────────────────────────────────────────────────

/// Generate a self-contained HTML report from a `BacktestReport`.
///
/// Returns an HTML string that can be written directly to a `.html` file.
/// The output is fully self-contained: all CSS is embedded, all charts are
/// inline SVG — no internet access required to view the file.
pub fn generate_html_report(report: &BacktestReport) -> String {
    let generated_at = Local::now().format("%Y-%m-%d %H:%M:%S UTC%z").to_string();

    let kpi_section = render_kpi_section(report);
    let wf_section = render_walk_forward_section(&report.walk_forward_periods);
    let regime_section = render_regime_section(report);
    let mc_section = render_monte_carlo_section(&report.confidence_intervals);
    let dd_section = render_drawdown_section(&report.drawdowns);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Ghost Alpha Backtest Report</title>
<style>
{css}
</style>
</head>
<body>
<header>
  <h1>Ghost Alpha Backtest Report</h1>
  <p class="meta">Generated: {generated_at} &nbsp;|&nbsp; Total signals: {total_signals}</p>
</header>
<main>
{kpi_section}
{wf_section}
{regime_section}
{mc_section}
{dd_section}
</main>
<footer>
  <p>Ghost Alpha &mdash; Minervini-based stock screener &mdash; generated {generated_at}</p>
</footer>
</body>
</html>
"#,
        css = EMBEDDED_CSS,
        generated_at = generated_at,
        total_signals = report.kpis.total_signals,
        kpi_section = kpi_section,
        wf_section = wf_section,
        regime_section = regime_section,
        mc_section = mc_section,
        dd_section = dd_section,
    )
}

// ─── KPI section ─────────────────────────────────────────────────────────────

fn render_kpi_section(report: &BacktestReport) -> String {
    let kpis = &report.kpis;

    let hit_class = kpi_class(kpis.hit_rate_60d, 0.60, 0.55);
    let alpha_class = kpi_class(kpis.alpha_60d, 0.03, 0.0);
    let sharpe_class = kpi_class(kpis.sharpe_ratio, 1.0, 0.5);
    // drawdown: deeper negative = worse; flip so that >= -0.10 is good, >= -0.25 is warn
    let dd_class = kpi_class_lower(kpis.max_drawdown_depth, -0.10, -0.25);

    format!(
        r#"<section id="summary">
  <h2>Executive Summary</h2>
  <div class="kpi-grid">
    <div class="kpi {hit_class}">
      <div class="kpi-label">Hit Rate (60d)</div>
      <div class="kpi-value">{hit_rate:.1}%</div>
    </div>
    <div class="kpi {hit30_class}">
      <div class="kpi-label">Hit Rate (30d)</div>
      <div class="kpi-value">{hit30:.1}%</div>
    </div>
    <div class="kpi {hit90_class}">
      <div class="kpi-label">Hit Rate (90d)</div>
      <div class="kpi-value">{hit90:.1}%</div>
    </div>
    <div class="kpi {alpha_class}">
      <div class="kpi-label">Alpha vs Bench (60d)</div>
      <div class="kpi-value">{alpha:+.2}%</div>
    </div>
    <div class="kpi {sharpe_class}">
      <div class="kpi-label">Sharpe Ratio</div>
      <div class="kpi-value">{sharpe:.2}</div>
    </div>
    <div class="kpi {dd_class}">
      <div class="kpi-label">Max Drawdown</div>
      <div class="kpi-value">{dd:.1}%</div>
    </div>
    <div class="kpi kpi-neutral">
      <div class="kpi-label">Win / Loss Ratio</div>
      <div class="kpi-value">{wl:.2}</div>
    </div>
    <div class="kpi kpi-neutral">
      <div class="kpi-label">Profit Factor</div>
      <div class="kpi-value">{pf:.2}</div>
    </div>
    <div class="kpi kpi-neutral">
      <div class="kpi-label">Max DD Duration</div>
      <div class="kpi-value">{dd_days} days</div>
    </div>
    <div class="kpi kpi-neutral">
      <div class="kpi-label">False Positive (60d)</div>
      <div class="kpi-value">{fp:.1}%</div>
    </div>
  </div>
</section>"#,
        hit_class = hit_class,
        hit30_class = kpi_class(kpis.hit_rate_30d, 0.60, 0.55),
        hit90_class = kpi_class(kpis.hit_rate_90d, 0.60, 0.55),
        hit_rate = kpis.hit_rate_60d * 100.0,
        hit30 = kpis.hit_rate_30d * 100.0,
        hit90 = kpis.hit_rate_90d * 100.0,
        alpha_class = alpha_class,
        alpha = kpis.alpha_60d * 100.0,
        sharpe_class = sharpe_class,
        sharpe = kpis.sharpe_ratio,
        dd_class = dd_class,
        dd = kpis.max_drawdown_depth * 100.0,
        wl = kpis.win_loss_ratio,
        pf = kpis.profit_factor,
        dd_days = kpis.max_drawdown_duration_days,
        fp = kpis.fp_rate_60d * 100.0,
    )
}

// ─── Walk-forward section ─────────────────────────────────────────────────────

fn render_walk_forward_section(periods: &[WalkForwardPeriod]) -> String {
    if periods.is_empty() {
        return r#"<section id="walk-forward">
  <h2>Walk-Forward Results</h2>
  <p class="empty">No walk-forward periods available.</p>
</section>"#
            .to_string();
    }

    let positive_count = periods.iter().filter(|p| p.oos_alpha > 0.0).count();

    let rows: String = periods
        .iter()
        .map(|p| {
            let alpha_class = if p.oos_alpha > 0.0 { "win" } else { "loss" };
            let pass_icon = if p.oos_alpha > 0.0 {
                "&#10003;"
            } else {
                "&#10007;"
            };
            let pass_class = if p.oos_alpha > 0.0 {
                "pass-yes"
            } else {
                "pass-no"
            };
            format!(
                r#"    <tr>
      <td>{idx}</td>
      <td>{is_start} &ndash; {is_end}</td>
      <td>{oos_start} &ndash; {oos_end}</td>
      <td>{hit:.1}%</td>
      <td class="{alpha_class}">{alpha:+.2}%</td>
      <td class="{alpha_class}">{fp:.1}%</td>
      <td class="{pass_class}">{pass_icon}</td>
    </tr>"#,
                idx = p.period_index,
                is_start = p.is_start,
                is_end = p.is_end,
                oos_start = p.oos_start,
                oos_end = p.oos_end,
                hit = p.oos_hit_rate * 100.0,
                alpha_class = alpha_class,
                alpha = p.oos_alpha * 100.0,
                fp = p.oos_fp_rate * 100.0,
                pass_class = pass_class,
                pass_icon = pass_icon,
            )
        })
        .collect();

    format!(
        r#"<section id="walk-forward">
  <h2>Walk-Forward Results</h2>
  <p class="summary">{positive_count} of {total} periods had positive OOS alpha.</p>
  <table>
    <thead>
      <tr>
        <th>#</th>
        <th>In-Sample</th>
        <th>Out-of-Sample</th>
        <th>OOS Hit%</th>
        <th>OOS Alpha%</th>
        <th>OOS FP%</th>
        <th>Pass</th>
      </tr>
    </thead>
    <tbody>
{rows}
    </tbody>
  </table>
</section>"#,
        positive_count = positive_count,
        total = periods.len(),
        rows = rows,
    )
}

// ─── Regime section ───────────────────────────────────────────────────────────

fn render_regime_section(report: &BacktestReport) -> String {
    if report.regime_kpis.is_empty() {
        return r#"<section id="regime">
  <h2>Regime Analysis</h2>
  <p class="empty">No regime breakdown available.</p>
</section>"#
            .to_string();
    }

    let rows: String = report
        .regime_kpis
        .iter()
        .map(|rk| {
            let alpha_class = if rk.alpha_60d > 0.0 { "win" } else { "loss" };
            format!(
                r#"    <tr>
      <td>{regime}</td>
      <td>{count}</td>
      <td>{hit:.1}%</td>
      <td class="{alpha_class}">{alpha:+.2}%</td>
      <td>{fp:.1}%</td>
    </tr>"#,
                regime = rk.regime,
                count = rk.signal_count,
                hit = rk.hit_rate_60d * 100.0,
                alpha_class = alpha_class,
                alpha = rk.alpha_60d * 100.0,
                fp = rk.fp_rate * 100.0,
            )
        })
        .collect();

    format!(
        r#"<section id="regime">
  <h2>Regime Analysis</h2>
  <table>
    <thead>
      <tr>
        <th>Regime</th>
        <th>Signals</th>
        <th>Hit Rate (60d)</th>
        <th>Alpha (60d)</th>
        <th>FP Rate</th>
      </tr>
    </thead>
    <tbody>
{rows}
    </tbody>
  </table>
</section>"#,
        rows = rows,
    )
}

// ─── Monte Carlo section ──────────────────────────────────────────────────────

fn render_monte_carlo_section(cis: &[ConfidenceInterval]) -> String {
    if cis.is_empty() {
        return r#"<section id="monte-carlo">
  <h2>Monte Carlo Confidence</h2>
  <p class="empty">No Monte Carlo results available.</p>
</section>"#
            .to_string();
    }

    let rows: String = cis
        .iter()
        .map(|ci| {
            let rejects = ci.rejects_null(0.0);
            let null_class = if rejects { "win" } else { "loss" };
            let null_text = if rejects { "Yes (p&lt;0.05)" } else { "No" };
            format!(
                r#"    <tr>
      <td>{name}</td>
      <td>{est:.4}</td>
      <td>[{lo:.4}, {hi:.4}]</td>
      <td class="{null_class}">{null_text}</td>
    </tr>"#,
                name = ci.metric_name,
                est = ci.point_estimate,
                lo = ci.ci_lower,
                hi = ci.ci_upper,
                null_class = null_class,
                null_text = null_text,
            )
        })
        .collect();

    // Build an SVG histogram for the first CI (usually hit_rate or alpha)
    let svg = monte_carlo_ci_svg(cis);

    format!(
        r#"<section id="monte-carlo">
  <h2>Monte Carlo Confidence (95% CI)</h2>
  <table>
    <thead>
      <tr>
        <th>Metric</th>
        <th>Point Estimate</th>
        <th>95% CI</th>
        <th>Rejects H₀ = 0</th>
      </tr>
    </thead>
    <tbody>
{rows}
    </tbody>
  </table>
  <div class="chart-container">
{svg}
  </div>
</section>"#,
        rows = rows,
        svg = svg,
    )
}

/// Generate an inline SVG showing the CI ranges as a horizontal bar chart.
fn monte_carlo_ci_svg(cis: &[ConfidenceInterval]) -> String {
    if cis.is_empty() {
        return String::new();
    }

    let bar_height = 28i32;
    let bar_gap = 10i32;
    let left_margin = 160i32;
    let right_margin = 20i32;
    let chart_width = 500i32;
    let plot_width = chart_width - left_margin - right_margin;
    let svg_height = (bar_height + bar_gap) * cis.len() as i32 + bar_gap + 30;

    // Determine axis range: min of all ci_lower, max of all ci_upper
    let all_vals: Vec<f64> = cis
        .iter()
        .flat_map(|ci| [ci.ci_lower, ci.ci_upper, ci.point_estimate, 0.0])
        .collect();
    let x_min = all_vals.iter().cloned().fold(f64::INFINITY, f64::min);
    let x_max = all_vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let x_range = (x_max - x_min).max(1e-6);

    let to_px =
        |v: f64| -> i32 { left_margin + ((v - x_min) / x_range * plot_width as f64) as i32 };

    let zero_px = to_px(0.0);

    // Build bar elements individually using push_str to avoid format! parsing
    // attribute names containing hyphens (e.g. stroke-width) as format args.
    let mut bars = String::new();

    for (i, ci) in cis.iter().enumerate() {
        let y = bar_gap + (bar_height + bar_gap) * i as i32;
        let lo_px = to_px(ci.ci_lower);
        let hi_px = to_px(ci.ci_upper);
        let pt_px = to_px(ci.point_estimate);
        let bar_w = (hi_px - lo_px).max(2);
        let fill = if ci.point_estimate > 0.0 {
            "#2b8a3e"
        } else {
            "#c92a2a"
        };
        let text_y = y + bar_height / 2 + 5;
        let bot = y + bar_height;
        let label_x = left_margin - 6;

        // <text> label
        bars.push_str("  <text x=\"");
        bars.push_str(&label_x.to_string());
        bars.push_str("\" y=\"");
        bars.push_str(&text_y.to_string());
        bars.push_str("\" class=\"ci-label\">");
        bars.push_str(&ci.metric_name);
        bars.push_str("</text>\n");

        // <rect> CI band
        bars.push_str("  <rect x=\"");
        bars.push_str(&lo_px.to_string());
        bars.push_str("\" y=\"");
        bars.push_str(&y.to_string());
        bars.push_str("\" width=\"");
        bars.push_str(&bar_w.to_string());
        bars.push_str("\" height=\"");
        bars.push_str(&bar_height.to_string());
        bars.push_str("\" fill=\"");
        bars.push_str(fill);
        bars.push_str("\" opacity=\"0.35\" rx=\"3\"/>\n");

        // <line> point estimate marker
        bars.push_str("  <line x1=\"");
        bars.push_str(&pt_px.to_string());
        bars.push_str("\" y1=\"");
        bars.push_str(&y.to_string());
        bars.push_str("\" x2=\"");
        bars.push_str(&pt_px.to_string());
        bars.push_str("\" y2=\"");
        bars.push_str(&bot.to_string());
        bars.push_str("\" stroke=\"");
        bars.push_str(fill);
        bars.push_str("\" stroke-width=\"3\"/>\n");
    }

    // Build outer SVG wrapper the same way
    let mut svg = String::new();
    svg.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"");
    svg.push_str(&chart_width.to_string());
    svg.push_str("\" height=\"");
    svg.push_str(&svg_height.to_string());
    svg.push_str("\" class=\"ci-chart\">\n");
    svg.push_str("  <style>\n");
    svg.push_str("    .ci-label { font: 11px system-ui; fill: #495057; text-anchor: end; }\n");
    svg.push_str("    .axis-label { font: 10px system-ui; fill: #868e96; text-anchor: middle; }\n");
    svg.push_str("  </style>\n");

    // Zero reference line
    svg.push_str("  <line x1=\"");
    svg.push_str(&zero_px.to_string());
    svg.push_str("\" y1=\"0\" x2=\"");
    svg.push_str(&zero_px.to_string());
    svg.push_str("\" y2=\"");
    svg.push_str(&svg_height.to_string());
    svg.push_str("\" stroke=\"#adb5bd\" stroke-width=\"1\" stroke-dasharray=\"4,3\"/>\n");

    // Zero axis label
    svg.push_str("  <text x=\"");
    svg.push_str(&zero_px.to_string());
    svg.push_str("\" y=\"");
    svg.push_str(&(svg_height - 5).to_string());
    svg.push_str("\" class=\"axis-label\">0</text>\n");

    svg.push_str(&bars);
    svg.push_str("</svg>");
    svg
}

// ─── Drawdown section ─────────────────────────────────────────────────────────

fn render_drawdown_section(drawdowns: &[DrawdownEvent]) -> String {
    if drawdowns.is_empty() {
        return r#"<section id="drawdowns">
  <h2>Drawdown Analysis</h2>
  <p class="empty">No drawdown events recorded.</p>
</section>"#
            .to_string();
    }

    // Show top 5 worst drawdowns (deepest first)
    let mut sorted: Vec<&DrawdownEvent> = drawdowns.iter().collect();
    sorted.sort_by(|a, b| {
        a.depth
            .partial_cmp(&b.depth)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let top5 = sorted.iter().take(5);

    let rows: String = top5
        .map(|dd| {
            let recovery = dd
                .recovery_date
                .map(|d| d.to_string())
                .unwrap_or_else(|| "N/A".to_string());
            format!(
                r#"    <tr>
      <td>{start}</td>
      <td>{trough}</td>
      <td>{recovery}</td>
      <td class="loss">{depth:.1}%</td>
      <td>{duration} days</td>
    </tr>"#,
                start = dd.start_date,
                trough = dd.trough_date,
                recovery = recovery,
                depth = dd.depth * 100.0,
                duration = dd.duration_days,
            )
        })
        .collect();

    format!(
        r#"<section id="drawdowns">
  <h2>Drawdown Analysis</h2>
  <p class="summary">Top {shown} worst drawdown periods (of {total} total).</p>
  <table>
    <thead>
      <tr>
        <th>Start</th>
        <th>Trough</th>
        <th>Recovery</th>
        <th>Depth</th>
        <th>Duration</th>
      </tr>
    </thead>
    <tbody>
{rows}
    </tbody>
  </table>
</section>"#,
        shown = sorted.len().min(5),
        total = drawdowns.len(),
        rows = rows,
    )
}

// ─── KPI colour helpers ───────────────────────────────────────────────────────

/// Returns CSS class based on whether `value` exceeds good/warn thresholds.
/// Higher is better.
fn kpi_class(value: f64, good: f64, warn: f64) -> &'static str {
    if value >= good {
        "kpi-good"
    } else if value >= warn {
        "kpi-warn"
    } else {
        "kpi-bad"
    }
}

/// Like `kpi_class` but higher is worse (e.g. drawdown depth where more negative = worse).
/// `good` and `warn` are thresholds; value >= good is good, value >= warn is warn, else bad.
fn kpi_class_lower(value: f64, good: f64, warn: f64) -> &'static str {
    kpi_class(value, good, warn)
}

// ─── Embedded CSS ────────────────────────────────────────────────────────────

const EMBEDDED_CSS: &str = r#"
/* Reset & base */
*, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
html { font-size: 15px; }
body {
  font-family: system-ui, -apple-system, sans-serif;
  background: #f8f9fa;
  color: #212529;
  max-width: 960px;
  margin: 0 auto;
  padding: 1.5rem;
}

/* Header */
header {
  border-bottom: 2px solid #343a40;
  padding-bottom: 0.75rem;
  margin-bottom: 2rem;
}
header h1 { font-size: 1.6rem; color: #1c7ed6; }
.meta { font-size: 0.8rem; color: #868e96; margin-top: 0.25rem; }

/* Sections */
section {
  background: #fff;
  border-radius: 8px;
  padding: 1.25rem 1.5rem;
  margin-bottom: 1.5rem;
  box-shadow: 0 1px 4px rgba(0,0,0,0.08);
}
section h2 {
  font-size: 1.1rem;
  color: #343a40;
  margin-bottom: 1rem;
  padding-bottom: 0.4rem;
  border-bottom: 1px solid #e9ecef;
}

/* KPI grid */
.kpi-grid {
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem;
}
.kpi {
  flex: 0 0 auto;
  min-width: 130px;
  border-radius: 6px;
  padding: 0.75rem 1rem;
  text-align: center;
}
.kpi-label { font-size: 0.72rem; color: #495057; text-transform: uppercase; letter-spacing: 0.04em; margin-bottom: 0.3rem; }
.kpi-value { font-size: 1.4rem; font-weight: 700; }
.kpi-good  { background: #d3f9d8; }
.kpi-warn  { background: #fff3bf; }
.kpi-bad   { background: #ffe3e3; }
.kpi-neutral { background: #e9ecef; }

/* Tables */
table { border-collapse: collapse; width: 100%; margin-top: 0.5rem; font-size: 0.85rem; }
th, td { border: 1px solid #dee2e6; padding: 0.45rem 0.7rem; }
th { background: #f1f3f5; text-align: center; font-weight: 600; color: #495057; }
td { text-align: right; }
td:first-child { text-align: left; }
tr:nth-child(even) td { background: #f8f9fa; }

/* Win/loss colours */
.win  { color: #2b8a3e; font-weight: 600; }
.loss { color: #c92a2a; font-weight: 600; }
.pass-yes { color: #2b8a3e; font-weight: 700; text-align: center !important; }
.pass-no  { color: #c92a2a; font-weight: 700; text-align: center !important; }

/* Summary & empty text */
.summary { font-size: 0.85rem; color: #495057; margin-bottom: 0.75rem; }
.empty   { font-size: 0.85rem; color: #868e96; font-style: italic; }

/* SVG chart container */
.chart-container { margin-top: 1rem; overflow-x: auto; }
.ci-chart { display: block; }

/* Footer */
footer {
  text-align: center;
  font-size: 0.75rem;
  color: #adb5bd;
  margin-top: 2rem;
  padding-top: 1rem;
  border-top: 1px solid #dee2e6;
}

/* Dark mode */
@media (prefers-color-scheme: dark) {
  body { background: #1a1b1e; color: #c1c2c5; }
  header { border-bottom-color: #495057; }
  header h1 { color: #74c0fc; }
  section { background: #25262b; box-shadow: 0 1px 4px rgba(0,0,0,0.4); }
  section h2 { color: #e9ecef; border-bottom-color: #373a40; }
  th { background: #2c2e33; color: #ced4da; }
  td { color: #c1c2c5; }
  tr:nth-child(even) td { background: #2c2e33; }
  table, th, td { border-color: #373a40; }
  .kpi-label { color: #868e96; }
  .kpi-neutral { background: #373a40; }
  .kpi-good  { background: #1a4731; }
  .kpi-warn  { background: #3d3000; }
  .kpi-bad   { background: #4a0c0c; }
  .summary { color: #868e96; }
  footer { color: #495057; border-top-color: #373a40; }
}
"#;

// ─── Demo data helper ─────────────────────────────────────────────────────────

/// Generate synthetic demo signals spanning `years` years.
///
/// Used when `--format html` is requested but no real signal history has been
/// collected yet.  The returns are deliberately varied so all KPI paths are
/// exercised (drawdown, Sharpe, etc.).
pub fn demo_signals(years: u32) -> Vec<crate::backtest::types::BacktestSignal> {
    use crate::backtest::types::{BacktestSignal, ForwardReturns};
    use crate::data::models::MarketRegime;

    let total_days = years as i64 * 365;
    let step = 14i64; // one signal every 2 weeks
    let n = (total_days / step) as usize;

    (0..n)
        .map(|i| {
            let base_date = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()
                + chrono::Duration::days(i as i64 * step);
            // Alternate wins and losses with a slight positive bias
            let ret_30 = if i % 4 == 0 {
                -0.04
            } else {
                0.05 + (i as f64 % 7.0) * 0.003
            };
            let ret_60 = if i % 5 == 0 {
                -0.06
            } else {
                0.07 + (i as f64 % 5.0) * 0.004
            };
            let ret_90 = if i % 6 == 0 {
                -0.05
            } else {
                0.09 + (i as f64 % 4.0) * 0.003
            };
            let regime = match i % 3 {
                0 => MarketRegime::Bull,
                1 => MarketRegime::Sideways,
                _ => MarketRegime::Bear,
            };
            BacktestSignal {
                ticker: ["NVDA", "AAPL", "MSFT", "META", "AMZN"][i % 5].to_string(),
                signal_date: base_date,
                entry_price: 100.0 + (i as f64 % 20.0),
                composite_score: 70.0 + (i as f64 % 25.0),
                regime,
                forward_returns: ForwardReturns {
                    ret_30d: Some(ret_30),
                    ret_60d: Some(ret_60),
                    ret_90d: Some(ret_90),
                },
            }
        })
        .collect()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::report::run_full_backtest;
    use crate::backtest::types::{BacktestParams, BacktestSignal, ForwardReturns};
    use crate::data::models::MarketRegime;
    use chrono::NaiveDate;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn make_signal(ticker: &str, day_offset: i64, ret: f64) -> BacktestSignal {
        BacktestSignal {
            ticker: ticker.to_string(),
            signal_date: date(2022, 1, 1) + chrono::Duration::days(day_offset * 7),
            entry_price: 100.0,
            composite_score: 80.0,
            regime: MarketRegime::Bull,
            forward_returns: ForwardReturns {
                ret_30d: Some(ret),
                ret_60d: Some(ret),
                ret_90d: Some(ret),
            },
        }
    }

    fn sample_report() -> BacktestReport {
        let signals: Vec<BacktestSignal> = (0..40)
            .map(|i| {
                let ret = if i % 3 == 0 {
                    -0.03
                } else {
                    0.06 + (i as f64) * 0.001
                };
                make_signal("AAPL", i, ret)
            })
            .collect();
        let params = BacktestParams {
            monte_carlo_iterations: 200,
            ..Default::default()
        };
        run_full_backtest(&signals, &[], &params)
    }

    // ── DOCTYPE and structure ─────────────────────────────────────────────────

    #[test]
    fn html_starts_with_doctype() {
        let report = sample_report();
        let html = generate_html_report(&report);
        assert!(html.trim_start().starts_with("<!DOCTYPE html>"));
    }

    #[test]
    fn html_contains_title() {
        let report = sample_report();
        let html = generate_html_report(&report);
        assert!(html.contains("<title>Ghost Alpha Backtest Report</title>"));
    }

    // ── Sections present ──────────────────────────────────────────────────────

    #[test]
    fn html_contains_executive_summary_section() {
        let report = sample_report();
        let html = generate_html_report(&report);
        assert!(
            html.contains("Executive Summary"),
            "must have Executive Summary heading"
        );
        assert!(
            html.contains("Hit Rate (60d)"),
            "must show 60d hit rate KPI"
        );
    }

    #[test]
    fn html_contains_walk_forward_section() {
        let report = sample_report();
        let html = generate_html_report(&report);
        assert!(
            html.contains("Walk-Forward Results"),
            "must have Walk-Forward section heading"
        );
    }

    #[test]
    fn html_contains_regime_section() {
        let report = sample_report();
        let html = generate_html_report(&report);
        assert!(
            html.contains("Regime Analysis"),
            "must have Regime Analysis section"
        );
    }

    #[test]
    fn html_contains_monte_carlo_section() {
        let report = sample_report();
        let html = generate_html_report(&report);
        assert!(
            html.contains("Monte Carlo Confidence"),
            "must have Monte Carlo section"
        );
    }

    #[test]
    fn html_contains_drawdown_section() {
        let report = sample_report();
        let html = generate_html_report(&report);
        assert!(
            html.contains("Drawdown Analysis"),
            "must have Drawdown Analysis section"
        );
    }

    // ── Self-contained (no external deps) ────────────────────────────────────

    #[test]
    fn html_has_no_external_stylesheet_links() {
        let report = sample_report();
        let html = generate_html_report(&report);
        // All styles must be embedded — no <link rel="stylesheet" href="http...">
        assert!(
            !html.contains(r#"<link rel="stylesheet" href="http"#),
            "report must not link external stylesheets"
        );
    }

    #[test]
    fn html_has_no_script_tags() {
        let report = sample_report();
        let html = generate_html_report(&report);
        assert!(
            !html.contains("<script"),
            "report must not contain JavaScript"
        );
    }

    #[test]
    fn html_contains_embedded_css() {
        let report = sample_report();
        let html = generate_html_report(&report);
        assert!(
            html.contains("<style>"),
            "report must embed CSS in <style> block"
        );
    }

    // ── SVG chart ─────────────────────────────────────────────────────────────

    #[test]
    fn html_contains_inline_svg_when_mc_data_present() {
        let report = sample_report();
        let html = generate_html_report(&report);
        if !report.confidence_intervals.is_empty() {
            assert!(
                html.contains("<svg"),
                "HTML must contain inline SVG for CI chart"
            );
        }
    }

    #[test]
    fn svg_ci_chart_standalone() {
        use crate::backtest::types::ConfidenceInterval;
        let cis = vec![
            ConfidenceInterval {
                metric_name: "hit_rate".to_string(),
                point_estimate: 0.62,
                ci_lower: 0.52,
                ci_upper: 0.72,
            },
            ConfidenceInterval {
                metric_name: "alpha".to_string(),
                point_estimate: 0.04,
                ci_lower: 0.01,
                ci_upper: 0.07,
            },
        ];
        let svg = monte_carlo_ci_svg(&cis);
        assert!(svg.starts_with("<svg"), "output must be an SVG element");
        assert!(svg.contains("hit_rate"), "SVG must label the hit_rate CI");
        assert!(svg.contains("alpha"), "SVG must label the alpha CI");
    }

    // ── KPI colour thresholds ─────────────────────────────────────────────────

    #[test]
    fn kpi_class_returns_good_above_threshold() {
        assert_eq!(kpi_class(0.65, 0.60, 0.55), "kpi-good");
    }

    #[test]
    fn kpi_class_returns_warn_between_thresholds() {
        assert_eq!(kpi_class(0.57, 0.60, 0.55), "kpi-warn");
    }

    #[test]
    fn kpi_class_returns_bad_below_warn() {
        assert_eq!(kpi_class(0.40, 0.60, 0.55), "kpi-bad");
    }

    // ── Drawdown top-5 ────────────────────────────────────────────────────────

    #[test]
    fn drawdown_section_limits_to_five_rows() {
        use crate::backtest::types::DrawdownEvent;
        // Create 8 drawdown events
        let drawdowns: Vec<DrawdownEvent> = (1..=8)
            .map(|i| DrawdownEvent {
                start_date: date(2022, i, 1),
                trough_date: date(2022, i, 15),
                recovery_date: None,
                depth: -(i as f64) * 0.03,
                duration_days: i * 10,
            })
            .collect();
        let html = render_drawdown_section(&drawdowns);
        // Count <tr> tags in tbody (each row is one drawdown)
        let row_count = html.matches("<tr>").count();
        // 1 header row + up to 5 data rows = 6 max
        assert!(
            row_count <= 6,
            "should render at most 5 data rows, got {}",
            row_count - 1
        );
    }

    // ── File size sanity ──────────────────────────────────────────────────────

    #[test]
    fn html_file_size_below_1mb() {
        let report = sample_report();
        let html = generate_html_report(&report);
        let size_bytes = html.len();
        assert!(
            size_bytes < 1_000_000,
            "HTML report is {}KB, must be < 1MB",
            size_bytes / 1024
        );
    }

    // ── Footer ────────────────────────────────────────────────────────────────

    #[test]
    fn html_contains_footer_with_timestamp() {
        let report = sample_report();
        let html = generate_html_report(&report);
        assert!(html.contains("<footer>"), "must have a <footer> element");
        assert!(
            html.contains("Ghost Alpha"),
            "footer must mention Ghost Alpha"
        );
    }
}
