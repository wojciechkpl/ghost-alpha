# Plan 07: Published Backtest Report

**Effort:** Medium (3-4 days)
**Impact:** High
**Priority:** 7

## Problem

The backtesting engine computes walk-forward KPIs, Monte Carlo confidence intervals, and drawdown analysis — but there's no visual output. Users can't see how Ghost Alpha signals would have performed historically. This is the single biggest trust gap for adoption.

## Solution

Add `ghost-alpha backtest --report html` that generates a self-contained HTML report with charts, tables, and key takeaways. Also generate a reference report for the README.

## Report contents

### Page 1: Executive Summary
- Overall hit rate (30/60/90 day)
- Alpha vs SPY buy-and-hold
- Win/loss ratio, profit factor
- Sharpe ratio
- Max drawdown (depth + duration)
- Signal count by year

### Page 2: Walk-Forward Results
- Table of each walk-forward period (in-sample dates, OOS dates, OOS hit rate, OOS alpha)
- Pass/fail indicator: did OOS alpha > 0?
- Summary: X of Y periods had positive OOS alpha

### Page 3: Regime Analysis
- Per-regime breakdown: Bull / Sideways / Bear
- Hit rates by regime
- Average alpha by regime
- Signal volume by regime

### Page 4: Monte Carlo Confidence
- 95% CI for hit rate
- 95% CI for alpha
- Distribution histogram (text-based or embedded SVG)
- Null hypothesis test result

### Page 5: Drawdown Analysis
- Max drawdown depth and duration
- Recovery time
- Top 5 worst drawdown periods

### Page 6: Signal Log
- Sortable table of all signals with forward returns
- Color-coded: green (win), red (loss)
- Filterable by year, regime, grade

## Implementation

### 1. HTML template engine

Use `askama` (compile-time Rust templates) for the HTML report:

```toml
[dependencies]
askama = "0.12"
```

Create `templates/backtest_report.html`:

```html
<!DOCTYPE html>
<html>
<head>
    <title>Ghost Alpha Backtest Report</title>
    <style>
        /* Embedded CSS — no external deps */
        body { font-family: system-ui; max-width: 900px; margin: 0 auto; }
        .kpi { display: inline-block; padding: 1rem; margin: 0.5rem; border-radius: 8px; }
        .kpi-good { background: #d3f9d8; }
        .kpi-warn { background: #fff3bf; }
        .kpi-bad  { background: #ffe3e3; }
        table { border-collapse: collapse; width: 100%; }
        th, td { border: 1px solid #dee2e6; padding: 8px; text-align: right; }
        th { background: #f8f9fa; }
        .win { color: #2b8a3e; }
        .loss { color: #c92a2a; }
    </style>
</head>
<body>
    <h1>Ghost Alpha Backtest Report</h1>
    <p>Generated: {{ generated_at }} | Period: {{ start_date }} to {{ end_date }}</p>

    <h2>Key Metrics</h2>
    <div class="kpi {{ hit_rate_class }}">
        <div>Hit Rate (60d)</div>
        <div><strong>{{ hit_rate_60d }}%</strong></div>
    </div>
    <!-- ... more KPIs ... -->

    <h2>Walk-Forward Results</h2>
    <table>
        <tr><th>Period</th><th>In-Sample</th><th>OOS</th><th>OOS Hit Rate</th><th>OOS Alpha</th><th>Pass</th></tr>
        {% for period in walk_forward_periods %}
        <tr>
            <td>{{ period.id }}</td>
            <td>{{ period.is_range }}</td>
            <td>{{ period.oos_range }}</td>
            <td>{{ period.oos_hit_rate }}%</td>
            <td class="{{ period.alpha_class }}">{{ period.oos_alpha }}%</td>
            <td>{{ period.pass_icon }}</td>
        </tr>
        {% endfor %}
    </table>

    <!-- ... regime analysis, Monte Carlo, drawdowns, signal log ... -->
</body>
</html>
```

### 2. SVG chart generation

For histograms and timelines, generate inline SVG (no JavaScript dependencies):

```rust
fn monte_carlo_histogram_svg(samples: &[f64], ci_low: f64, ci_high: f64) -> String {
    // Generate SVG bars + CI markers
    // All self-contained, no external dependencies
}
```

### 3. CLI integration

```rust
Commands::Backtest {
    #[arg(long, default_value = "5")]
    years: u32,
    /// Output format: table (terminal), json, html (report file)
    #[arg(long, default_value = "table")]
    format: BacktestFormat,
    /// Output file path for HTML report
    #[arg(long, default_value = "backtest_report.html")]
    output: String,
}
```

### 4. Report data struct

```rust
#[derive(Template)]  // askama
#[template(path = "backtest_report.html")]
pub struct BacktestReport {
    pub generated_at: String,
    pub start_date: String,
    pub end_date: String,
    pub kpis: BacktestKpis,
    pub walk_forward_periods: Vec<WalkForwardResult>,
    pub regime_breakdown: RegimeBreakdown,
    pub monte_carlo: MonteCarloResult,
    pub drawdowns: Vec<DrawdownPeriod>,
    pub signals: Vec<SignalWithReturn>,
}
```

## Files to create/modify

| File | Action |
|------|--------|
| `templates/backtest_report.html` | Create — Askama HTML template |
| `src/backtest/report.rs` | Modify — add HTML rendering |
| `src/backtest/charts.rs` | Create — SVG chart generators |
| `src/main.rs` | Modify — add `--format html` to backtest command |
| `Cargo.toml` | Modify — add `askama = "0.12"` |

## Dependencies

- `askama = "0.12"` — compile-time HTML templates (zero runtime overhead)

## Example usage

```bash
# Generate HTML report
ghost-alpha backtest --years 5 --format html --output report.html

# Open in browser
open report.html

# Generate JSON for custom analysis
ghost-alpha backtest --years 5 --format json > backtest.json
```

## Testing

- Unit test: Report struct populates all fields from backtest results
- Unit test: SVG histogram generates valid SVG
- Unit test: HTML output contains all expected sections
- Integration test: Full backtest → HTML pipeline with sample data

## Acceptance criteria

- [ ] `ghost-alpha backtest --format html` produces a self-contained HTML file
- [ ] Report opens in any browser with no external dependencies
- [ ] All 6 sections present: summary, walk-forward, regime, Monte Carlo, drawdowns, signals
- [ ] KPIs are color-coded (green/yellow/red) based on thresholds
- [ ] SVG charts render inline (no JavaScript required)
- [ ] File size < 1MB for a 5-year backtest
