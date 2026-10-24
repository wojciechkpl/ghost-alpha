use chrono::NaiveDate;

use crate::backtest::drawdown::{
    build_equity_curve, compute_drawdowns, max_drawdown_depth, max_drawdown_duration,
};
use crate::backtest::engine::{compute_kpis, regime_kpis, walk_forward_backtest};
use crate::backtest::monte_carlo::compute_all_cis;
use crate::backtest::types::{BacktestParams, BacktestReport, BacktestSignal};

/// Run the full backtest pipeline: KPIs, walk-forward, drawdown, Monte Carlo.
pub fn run_full_backtest(
    signals: &[BacktestSignal],
    benchmark_returns: &[(NaiveDate, f64)],
    params: &BacktestParams,
) -> BacktestReport {
    // 1. Aggregate KPIs
    let mut kpis = compute_kpis(signals, benchmark_returns, params);

    // 2. Walk-forward analysis
    let wf_periods = walk_forward_backtest(signals, benchmark_returns, params);

    // 3. Regime disaggregation
    let regime = regime_kpis(signals, benchmark_returns, params);

    // 4. Drawdown analysis
    let returns_60d: Vec<(NaiveDate, f64)> = signals
        .iter()
        .filter_map(|s| s.forward_returns.ret_60d.map(|r| (s.signal_date, r)))
        .collect();
    let equity_curve = build_equity_curve(&returns_60d, 10_000.0);
    let drawdowns = compute_drawdowns(&equity_curve);
    kpis.max_drawdown_depth = max_drawdown_depth(&drawdowns);
    kpis.max_drawdown_duration_days = max_drawdown_duration(&drawdowns);

    // 5. Monte Carlo confidence intervals
    let ret_values: Vec<f64> = signals
        .iter()
        .filter_map(|s| s.forward_returns.ret_60d)
        .collect();
    let hit_values: Vec<f64> = signals
        .iter()
        .filter_map(|s| {
            s.forward_returns
                .ret_60d
                .map(|r| if r > 0.0 { 1.0 } else { 0.0 })
        })
        .collect();
    let cis = compute_all_cis(
        &ret_values,
        &hit_values,
        params.monte_carlo_iterations,
        0.95,
        42,
    );

    BacktestReport {
        kpis,
        walk_forward_periods: wf_periods,
        regime_kpis: regime,
        confidence_intervals: cis,
        drawdowns,
        equity_curve,
    }
}

/// Format the report as a human-readable table.
pub fn format_report(report: &BacktestReport) -> String {
    let mut out = String::new();

    out.push_str("══════════════════════════════════════════════════\n");
    out.push_str("             GHOST ALPHA BACKTEST REPORT          \n");
    out.push_str("══════════════════════════════════════════════════\n\n");

    // KPIs section
    out.push_str("── Key Performance Indicators ──────────────────────\n");
    out.push_str(&format!(
        "  Total Signals:         {}\n",
        report.kpis.total_signals
    ));
    out.push_str(&format!(
        "  Hit Rate (30d):        {:.1}%\n",
        report.kpis.hit_rate_30d * 100.0
    ));
    out.push_str(&format!(
        "  Hit Rate (60d):        {:.1}%\n",
        report.kpis.hit_rate_60d * 100.0
    ));
    out.push_str(&format!(
        "  Hit Rate (90d):        {:.1}%\n",
        report.kpis.hit_rate_90d * 100.0
    ));
    out.push_str(&format!(
        "  False Positive (60d):  {:.1}%\n",
        report.kpis.fp_rate_60d * 100.0
    ));
    out.push_str(&format!(
        "  Alpha vs Bench (60d):  {:.2}%\n",
        report.kpis.alpha_60d * 100.0
    ));
    out.push_str(&format!(
        "  Win/Loss Ratio:        {:.2}\n",
        report.kpis.win_loss_ratio
    ));
    out.push_str(&format!(
        "  Profit Factor:         {:.2}\n",
        report.kpis.profit_factor
    ));
    out.push_str(&format!(
        "  Sharpe Ratio:          {:.2}\n",
        report.kpis.sharpe_ratio
    ));
    out.push_str(&format!(
        "  Max Drawdown:          {:.1}%\n",
        report.kpis.max_drawdown_depth * 100.0
    ));
    out.push_str(&format!(
        "  Max DD Duration:       {} days\n",
        report.kpis.max_drawdown_duration_days
    ));
    out.push('\n');

    // Walk-forward section
    if !report.walk_forward_periods.is_empty() {
        out.push_str("── Walk-Forward Results ────────────────────────────\n");
        out.push_str(&format!(
            "  {:>5} {:>12} {:>12} {:>8} {:>8} {:>8}\n",
            "Period", "OOS Start", "OOS End", "Hit%", "Alpha%", "FP%"
        ));
        for p in &report.walk_forward_periods {
            out.push_str(&format!(
                "  {:>5} {:>12} {:>12} {:>7.1} {:>7.2} {:>7.1}\n",
                p.period_index,
                p.oos_start,
                p.oos_end,
                p.oos_hit_rate * 100.0,
                p.oos_alpha * 100.0,
                p.oos_fp_rate * 100.0,
            ));
        }
        out.push('\n');
    }

    // Regime section
    if !report.regime_kpis.is_empty() {
        out.push_str("── Regime Breakdown ────────────────────────────────\n");
        for rk in &report.regime_kpis {
            out.push_str(&format!(
                "  {:<10} signals={:<4} hit60={:.1}%  alpha60={:.2}%  fp={:.1}%\n",
                rk.regime.to_string(),
                rk.signal_count,
                rk.hit_rate_60d * 100.0,
                rk.alpha_60d * 100.0,
                rk.fp_rate * 100.0,
            ));
        }
        out.push('\n');
    }

    // Confidence intervals
    if !report.confidence_intervals.is_empty() {
        out.push_str("── Confidence Intervals (95%) ──────────────────────\n");
        for ci in &report.confidence_intervals {
            out.push_str(&format!(
                "  {:<16} {:.4}  [{:.4}, {:.4}]\n",
                ci.metric_name, ci.point_estimate, ci.ci_lower, ci.ci_upper,
            ));
        }
        out.push('\n');
    }

    out.push_str("══════════════════════════════════════════════════\n");
    out
}

/// Format the report as JSON.
pub fn format_report_json(report: &BacktestReport) -> String {
    serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".to_string())
}

/// Validate KPIs against PRD thresholds. Returns warnings for any violations.
///
/// PRD targets:
/// - Hit rate (60d): >= 55%
/// - False positive rate: <= 40%
/// - Max drawdown: >= -25%
/// - Sharpe: >= 0.5
pub fn validate_kpis(report: &BacktestReport) -> Vec<String> {
    let mut warnings = Vec::new();

    if report.kpis.hit_rate_60d < 0.55 {
        warnings.push(format!(
            "Hit rate (60d) {:.1}% is below PRD target of 55%",
            report.kpis.hit_rate_60d * 100.0
        ));
    }

    if report.kpis.fp_rate_60d > 0.40 {
        warnings.push(format!(
            "False positive rate {:.1}% exceeds PRD limit of 40%",
            report.kpis.fp_rate_60d * 100.0
        ));
    }

    if report.kpis.max_drawdown_depth < -0.25 {
        warnings.push(format!(
            "Max drawdown {:.1}% exceeds PRD limit of -25%",
            report.kpis.max_drawdown_depth * 100.0
        ));
    }

    if report.kpis.sharpe_ratio < 0.5 {
        warnings.push(format!(
            "Sharpe ratio {:.2} is below PRD target of 0.5",
            report.kpis.sharpe_ratio
        ));
    }

    warnings
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::types::{BacktestSignal, ForwardReturns};
    use crate::data::models::MarketRegime;

    fn make_signal(
        ticker: &str,
        date: NaiveDate,
        ret_30: Option<f64>,
        ret_60: Option<f64>,
        ret_90: Option<f64>,
    ) -> BacktestSignal {
        BacktestSignal {
            ticker: ticker.to_string(),
            signal_date: date,
            entry_price: 100.0,
            composite_score: 80.0,
            regime: MarketRegime::Bull,
            forward_returns: ForwardReturns {
                ret_30d: ret_30,
                ret_60d: ret_60,
                ret_90d: ret_90,
            },
        }
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn full_backtest_synthetic_data() {
        let signals: Vec<BacktestSignal> = (0..50)
            .map(|i| {
                make_signal(
                    "NVDA",
                    date(2022, 1, 1) + chrono::Duration::days(i * 7),
                    Some(0.03 + (i as f64) * 0.001),
                    Some(0.05 + (i as f64) * 0.001),
                    Some(0.08 + (i as f64) * 0.001),
                )
            })
            .collect();
        let params = BacktestParams {
            monte_carlo_iterations: 100,
            ..Default::default()
        };
        let report = run_full_backtest(&signals, &[], &params);
        assert_eq!(report.kpis.total_signals, 50);
        assert!(report.kpis.hit_rate_60d > 0.0);
    }

    #[test]
    fn format_includes_all_sections() {
        let signals: Vec<BacktestSignal> = (0..20)
            .map(|i| {
                make_signal(
                    "AAPL",
                    date(2022, 1, 1) + chrono::Duration::days(i * 14),
                    Some(0.05),
                    Some(0.08),
                    Some(0.10),
                )
            })
            .collect();
        let params = BacktestParams {
            monte_carlo_iterations: 100,
            ..Default::default()
        };
        let report = run_full_backtest(&signals, &[], &params);
        let text = format_report(&report);
        assert!(text.contains("Key Performance Indicators"));
        assert!(text.contains("Hit Rate"));
        assert!(text.contains("Confidence Intervals"));
    }

    #[test]
    fn validate_passes_good_data() {
        // Vary returns slightly so Sharpe is calculable (non-zero std dev)
        let signals: Vec<BacktestSignal> = (0..100)
            .map(|i| {
                let jitter = (i as f64 % 5.0) * 0.005;
                make_signal(
                    "GOOD",
                    date(2022, 1, 1) + chrono::Duration::days(i * 3),
                    Some(0.05 + jitter),
                    Some(0.08 + jitter),
                    Some(0.12 + jitter),
                )
            })
            .collect();
        let params = BacktestParams {
            monte_carlo_iterations: 100,
            ..Default::default()
        };
        let report = run_full_backtest(&signals, &[], &params);
        let warnings = validate_kpis(&report);
        assert!(
            warnings.is_empty(),
            "All-positive data should pass validation, got: {:?}",
            warnings
        );
    }

    #[test]
    fn validate_catches_low_hit_rate() {
        // Most signals lose
        let signals: Vec<BacktestSignal> = (0..100)
            .map(|i| {
                let ret = if i < 30 { 0.05 } else { -0.03 };
                make_signal(
                    "WEAK",
                    date(2022, 1, 1) + chrono::Duration::days(i * 3),
                    Some(ret),
                    Some(ret),
                    Some(ret),
                )
            })
            .collect();
        let params = BacktestParams {
            monte_carlo_iterations: 100,
            ..Default::default()
        };
        let report = run_full_backtest(&signals, &[], &params);
        let warnings = validate_kpis(&report);
        assert!(
            warnings.iter().any(|w| w.contains("Hit rate")),
            "Should warn about low hit rate, got: {:?}",
            warnings
        );
    }

    #[test]
    fn validate_catches_excessive_drawdown() {
        // Signals with a large drawdown in the middle
        let signals: Vec<BacktestSignal> = (0..100)
            .map(|i| {
                let ret = if (30..60).contains(&i) { -0.10 } else { 0.02 };
                make_signal(
                    "DD",
                    date(2022, 1, 1) + chrono::Duration::days(i * 3),
                    Some(ret),
                    Some(ret),
                    Some(ret),
                )
            })
            .collect();
        let params = BacktestParams {
            monte_carlo_iterations: 100,
            ..Default::default()
        };
        let report = run_full_backtest(&signals, &[], &params);
        let warnings = validate_kpis(&report);
        assert!(
            warnings.iter().any(|w| w.contains("drawdown")),
            "Should warn about excessive drawdown, got: {:?}",
            warnings
        );
    }
}
