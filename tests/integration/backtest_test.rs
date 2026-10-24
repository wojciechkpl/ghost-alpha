use chrono::NaiveDate;
use ghost_alpha::backtest::engine::walk_forward_backtest;
use ghost_alpha::backtest::monte_carlo::compute_all_cis;
use ghost_alpha::backtest::report::run_full_backtest;
use ghost_alpha::backtest::types::{BacktestParams, BacktestSignal, ForwardReturns};
use ghost_alpha::data::models::MarketRegime;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

fn make_signal(
    ticker: &str,
    signal_date: NaiveDate,
    ret_30: f64,
    ret_60: f64,
    ret_90: f64,
    regime: MarketRegime,
) -> BacktestSignal {
    BacktestSignal {
        ticker: ticker.to_string(),
        signal_date,
        entry_price: 100.0,
        composite_score: 80.0,
        regime,
        forward_returns: ForwardReturns {
            ret_30d: Some(ret_30),
            ret_60d: Some(ret_60),
            ret_90d: Some(ret_90),
        },
    }
}

/// Generate 3 years of weekly signals with realistic return distribution.
fn synthetic_signals() -> Vec<BacktestSignal> {
    (0..156) // 3 years of weekly signals
        .map(|i| {
            let week_date = date(2020, 1, 1) + chrono::Duration::weeks(i);
            let regime = if i < 52 {
                MarketRegime::Bull
            } else if i < 104 {
                MarketRegime::Sideways
            } else {
                MarketRegime::Bear
            };
            // Simulate varying returns: mostly positive with some losses
            let base = if i % 3 == 0 {
                -0.03
            } else {
                0.05 + (i as f64 % 7.0) * 0.005
            };
            make_signal("NVDA", week_date, base * 0.6, base, base * 1.5, regime)
        })
        .collect()
}

#[test]
fn synthetic_backtest_produces_report() {
    let signals = synthetic_signals();
    let params = BacktestParams {
        monte_carlo_iterations: 100, // fast for test
        ..Default::default()
    };
    let report = run_full_backtest(&signals, &[], &params);
    assert_eq!(report.kpis.total_signals, 156);
    assert!(report.kpis.hit_rate_60d > 0.0);
    assert!(report.kpis.hit_rate_60d < 1.0);
    assert!(!report.equity_curve.is_empty());
}

#[test]
fn walk_forward_produces_multiple_periods() {
    let signals = synthetic_signals();
    let params = BacktestParams::default();
    let periods = walk_forward_backtest(&signals, &[], &params);
    // 3 years with 12m IS + 3m OOS should yield several periods
    assert!(
        periods.len() >= 4,
        "Expected at least 4 walk-forward periods, got {}",
        periods.len()
    );
}

#[test]
fn monte_carlo_produces_cis() {
    let returns: Vec<f64> = (0..100)
        .map(|i| {
            if i % 3 == 0 {
                -0.03
            } else {
                0.05 + (i as f64 % 5.0) * 0.01
            }
        })
        .collect();
    let hits: Vec<f64> = returns
        .iter()
        .map(|&r| if r > 0.0 { 1.0 } else { 0.0 })
        .collect();
    let cis = compute_all_cis(&returns, &hits, 500, 0.95, 42);
    assert_eq!(cis.len(), 2);
    for ci in &cis {
        assert!(ci.ci_lower <= ci.point_estimate);
        assert!(ci.ci_upper >= ci.point_estimate);
    }
}
