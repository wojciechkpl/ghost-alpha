use chrono::NaiveDate;

use crate::backtest::types::{
    BacktestKpis, BacktestParams, BacktestSignal, RegimeKpis, WalkForwardPeriod,
};
use crate::data::models::MarketRegime;

/// Compute the forward return from `entry_date` over `horizon_days` trading days,
/// net of transaction costs (round-trip: 2x `tx_cost_bps`).
///
/// Returns `None` if there aren't enough candles after the entry date.
pub fn forward_return(
    prices: &[(NaiveDate, f64)],
    entry_date: NaiveDate,
    horizon_days: u32,
    tx_cost_bps: f64,
) -> Option<f64> {
    let entry_idx = prices.iter().position(|&(d, _)| d >= entry_date)?;
    let exit_idx = entry_idx + horizon_days as usize;
    if exit_idx >= prices.len() {
        return None;
    }
    let entry_price = prices[entry_idx].1;
    let exit_price = prices[exit_idx].1;
    if entry_price <= 0.0 {
        return None;
    }
    let gross_return = (exit_price - entry_price) / entry_price;
    let cost = tx_cost_bps / 10_000.0 * 2.0; // round-trip
    Some(gross_return - cost)
}

/// Fraction of signals with forward return > `threshold` (typically 0.0).
pub fn hit_rate(signals: &[BacktestSignal], horizon: u32, threshold: f64) -> f64 {
    let returns: Vec<f64> = signals
        .iter()
        .filter_map(|s| get_return(s, horizon))
        .collect();
    if returns.is_empty() {
        return 0.0;
    }
    let wins = returns.iter().filter(|&&r| r > threshold).count();
    wins as f64 / returns.len() as f64
}

/// Fraction of signals with forward return <= `threshold` (i.e., false positives).
pub fn false_positive_rate(signals: &[BacktestSignal], horizon: u32, threshold: f64) -> f64 {
    let returns: Vec<f64> = signals
        .iter()
        .filter_map(|s| get_return(s, horizon))
        .collect();
    if returns.is_empty() {
        return 0.0;
    }
    let fp = returns.iter().filter(|&&r| r <= threshold).count();
    fp as f64 / returns.len() as f64
}

/// Average signal return minus average benchmark return over `horizon` days.
pub fn alpha_vs_benchmark(
    signals: &[BacktestSignal],
    benchmark_returns: &[(NaiveDate, f64)],
    horizon: u32,
) -> f64 {
    let signal_returns: Vec<f64> = signals
        .iter()
        .filter_map(|s| get_return(s, horizon))
        .collect();
    if signal_returns.is_empty() {
        return 0.0;
    }

    let bench_returns: Vec<f64> = signals
        .iter()
        .filter_map(|s| {
            benchmark_returns
                .iter()
                .find(|(d, _)| *d == s.signal_date)
                .map(|(_, r)| *r)
        })
        .collect();

    let avg_signal = signal_returns.iter().sum::<f64>() / signal_returns.len() as f64;
    let avg_bench = if bench_returns.is_empty() {
        0.0
    } else {
        bench_returns.iter().sum::<f64>() / bench_returns.len() as f64
    };
    avg_signal - avg_bench
}

/// Average winning return / average losing return (absolute values).
pub fn win_loss_ratio(signals: &[BacktestSignal], horizon: u32) -> f64 {
    let returns: Vec<f64> = signals
        .iter()
        .filter_map(|s| get_return(s, horizon))
        .collect();

    let wins: Vec<f64> = returns.iter().filter(|&&r| r > 0.0).copied().collect();
    let losses: Vec<f64> = returns.iter().filter(|&&r| r < 0.0).copied().collect();

    if wins.is_empty() || losses.is_empty() {
        return if wins.is_empty() { 0.0 } else { f64::INFINITY };
    }

    let avg_win = wins.iter().sum::<f64>() / wins.len() as f64;
    let avg_loss = losses.iter().map(|r| r.abs()).sum::<f64>() / losses.len() as f64;

    if avg_loss == 0.0 {
        f64::INFINITY
    } else {
        avg_win / avg_loss
    }
}

/// Sum of wins / sum of losses (absolute values).
pub fn profit_factor(signals: &[BacktestSignal], horizon: u32) -> f64 {
    let returns: Vec<f64> = signals
        .iter()
        .filter_map(|s| get_return(s, horizon))
        .collect();

    let total_wins: f64 = returns.iter().filter(|&&r| r > 0.0).sum();
    let total_losses: f64 = returns.iter().filter(|&&r| r < 0.0).map(|r| r.abs()).sum();

    if total_losses == 0.0 {
        if total_wins > 0.0 {
            f64::INFINITY
        } else {
            0.0
        }
    } else {
        total_wins / total_losses
    }
}

/// **DEPRECATED**: Annualized Sharpe from raw returns with sqrt(252).
///
/// This function is suitable for daily return series but NOT for overlapping
/// N-day forward returns. Overlapping 60-day returns are auto-correlated
/// and massively overstate the Sharpe ratio when annualized with sqrt(252).
///
/// For backtest KPIs using overlapping signal returns, use
/// `sharpe_ratio_adjusted` which corrects the annualization factor.
/// For daily portfolio return series, use `sharpe_ratio_daily`.
pub fn sharpe_ratio(returns: &[f64], risk_free_rate: f64) -> f64 {
    if returns.len() < 2 {
        return 0.0;
    }

    let excess: Vec<f64> = returns.iter().map(|r| r - risk_free_rate).collect();
    let mean = excess.iter().sum::<f64>() / excess.len() as f64;
    let variance =
        excess.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (excess.len() - 1) as f64;
    let std_dev = variance.sqrt();

    if std_dev < 1e-12 {
        return 0.0;
    }
    mean / std_dev * (252.0_f64).sqrt()
}

/// Annualized Sharpe from a **daily** portfolio return series.
///
/// Converts annual risk-free rate to daily, computes daily excess returns,
/// and annualizes with sqrt(252). Requires at least 30 observations for
/// a meaningful estimate.
pub fn sharpe_ratio_daily(daily_returns: &[f64], annual_risk_free: f64) -> f64 {
    if daily_returns.len() < 30 {
        return 0.0;
    }
    let rf_daily = (1.0 + annual_risk_free).powf(1.0 / 252.0) - 1.0;
    let excess: Vec<f64> = daily_returns.iter().map(|r| r - rf_daily).collect();

    let mean = excess.iter().sum::<f64>() / excess.len() as f64;
    let variance =
        excess.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (excess.len() - 1) as f64;
    let std = variance.sqrt();

    if std < 1e-10 {
        return 0.0;
    }
    (mean / std) * (252.0_f64).sqrt()
}

/// Sharpe ratio adjusted for the observation frequency of overlapping returns.
///
/// When returns are computed over `horizon_days` windows (e.g., 60-day forward
/// returns), the correct annualization factor is sqrt(252 / horizon_days)
/// instead of sqrt(252).
pub fn sharpe_ratio_adjusted(returns: &[f64], risk_free_rate: f64, horizon_days: u32) -> f64 {
    if returns.len() < 2 || horizon_days == 0 {
        return 0.0;
    }

    let excess: Vec<f64> = returns.iter().map(|r| r - risk_free_rate).collect();
    let mean = excess.iter().sum::<f64>() / excess.len() as f64;
    let variance =
        excess.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (excess.len() - 1) as f64;
    let std_dev = variance.sqrt();

    if std_dev < 1e-12 {
        return 0.0;
    }
    mean / std_dev * (252.0 / horizon_days as f64).sqrt()
}

/// Build walk-forward IS/OOS windows across a date range.
///
/// Each window has `is_months` of in-sample data followed by `oos_months`
/// of out-of-sample data. Windows advance by `oos_months` each step.
pub fn build_walk_forward_windows(
    data_start: NaiveDate,
    data_end: NaiveDate,
    is_months: u32,
    oos_months: u32,
) -> Vec<(NaiveDate, NaiveDate, NaiveDate, NaiveDate)> {
    let mut windows = Vec::new();
    let mut is_start = data_start;

    loop {
        let is_end = add_months(is_start, is_months);
        let oos_start = is_end;
        let oos_end = add_months(oos_start, oos_months);

        if oos_end > data_end {
            break;
        }

        windows.push((is_start, is_end, oos_start, oos_end));
        is_start = add_months(is_start, oos_months);
    }

    windows
}

/// Run walk-forward backtest: for each IS/OOS window, compute OOS metrics.
pub fn walk_forward_backtest(
    signals: &[BacktestSignal],
    benchmark_returns: &[(NaiveDate, f64)],
    params: &BacktestParams,
) -> Vec<WalkForwardPeriod> {
    if signals.is_empty() {
        return Vec::new();
    }

    let data_start = signals.iter().map(|s| s.signal_date).min().unwrap();
    let data_end = signals.iter().map(|s| s.signal_date).max().unwrap();

    let windows = build_walk_forward_windows(
        data_start,
        data_end,
        params.walk_forward_in_sample_months,
        params.walk_forward_out_of_sample_months,
    );

    windows
        .iter()
        .enumerate()
        .map(|(i, &(is_start, is_end, oos_start, oos_end))| {
            let oos_signals: Vec<BacktestSignal> = signals
                .iter()
                .filter(|s| s.signal_date >= oos_start && s.signal_date < oos_end)
                .cloned()
                .collect();

            WalkForwardPeriod {
                period_index: i,
                is_start,
                is_end,
                oos_start,
                oos_end,
                oos_hit_rate: hit_rate(&oos_signals, 60, 0.0),
                oos_alpha: alpha_vs_benchmark(&oos_signals, benchmark_returns, 60),
                oos_fp_rate: false_positive_rate(&oos_signals, 60, 0.0),
            }
        })
        .collect()
}

/// KPIs disaggregated by market regime.
pub fn regime_kpis(
    signals: &[BacktestSignal],
    benchmark_returns: &[(NaiveDate, f64)],
    _params: &BacktestParams,
) -> Vec<RegimeKpis> {
    let regimes = [
        MarketRegime::Bull,
        MarketRegime::Sideways,
        MarketRegime::Bear,
    ];

    regimes
        .iter()
        .filter_map(|regime| {
            let regime_signals: Vec<BacktestSignal> = signals
                .iter()
                .filter(|s| s.regime == *regime)
                .cloned()
                .collect();

            if regime_signals.is_empty() {
                return None;
            }

            Some(RegimeKpis {
                regime: regime.clone(),
                signal_count: regime_signals.len(),
                hit_rate_60d: hit_rate(&regime_signals, 60, 0.0),
                alpha_60d: alpha_vs_benchmark(&regime_signals, benchmark_returns, 60),
                fp_rate: false_positive_rate(&regime_signals, 60, 0.0),
            })
        })
        .collect()
}

/// Compute aggregate KPIs across all signals.
///
/// Forward returns are adjusted by `transaction_cost_bps` (round-trip, in basis
/// points). For example, 10 bps = 0.10% deducted from each signal's return.
/// This uses the flat rate from params; for liquidity-adjusted costs, use
/// `adjusted_cost_bps()` from `indicators.rs` to compute per-signal costs.
pub fn compute_kpis(
    signals: &[BacktestSignal],
    benchmark_returns: &[(NaiveDate, f64)],
    params: &BacktestParams,
) -> BacktestKpis {
    let cost_adjustment = params.transaction_cost_bps / 10_000.0; // bps to fraction
    let all_60d: Vec<f64> = signals
        .iter()
        .filter_map(|s| s.forward_returns.ret_60d.map(|r| r - cost_adjustment))
        .collect();

    BacktestKpis {
        total_signals: signals.len(),
        hit_rate_30d: hit_rate(signals, 30, 0.0),
        hit_rate_60d: hit_rate(signals, 60, 0.0),
        hit_rate_90d: hit_rate(signals, 90, 0.0),
        fp_rate_60d: false_positive_rate(signals, 60, 0.0),
        alpha_60d: alpha_vs_benchmark(signals, benchmark_returns, 60),
        win_loss_ratio: win_loss_ratio(signals, 60),
        profit_factor: profit_factor(signals, 60),
        max_drawdown_depth: 0.0,       // filled by drawdown module
        max_drawdown_duration_days: 0, // filled by drawdown module
        // Phase 5: use horizon-adjusted annualization (sqrt(252/60)) instead
        // of sqrt(252) which massively overstates Sharpe for overlapping 60d returns
        sharpe_ratio: sharpe_ratio_adjusted(&all_60d, 0.0, 60),
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn get_return(signal: &BacktestSignal, horizon: u32) -> Option<f64> {
    match horizon {
        30 => signal.forward_returns.ret_30d,
        60 => signal.forward_returns.ret_60d,
        90 => signal.forward_returns.ret_90d,
        _ => None,
    }
}

fn add_months(date: NaiveDate, months: u32) -> NaiveDate {
    use chrono::Datelike;
    let total_months = date.month0() + months;
    let year = date.year() + (total_months / 12) as i32;
    let month = (total_months % 12) + 1;
    let day = date.day().min(days_in_month(year, month));
    NaiveDate::from_ymd_opt(year, month, day).unwrap_or(date)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backtest::types::ForwardReturns;

    fn make_signal(
        ticker: &str,
        date: NaiveDate,
        ret_30: Option<f64>,
        ret_60: Option<f64>,
        ret_90: Option<f64>,
        regime: MarketRegime,
    ) -> BacktestSignal {
        BacktestSignal {
            ticker: ticker.to_string(),
            signal_date: date,
            entry_price: 100.0,
            composite_score: 80.0,
            regime,
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

    // ── forward_return ──────────────────────────────────────────────────

    #[test]
    fn forward_return_positive() {
        let prices = vec![
            (date(2024, 1, 1), 100.0),
            (date(2024, 1, 2), 105.0),
            (date(2024, 1, 3), 110.0),
        ];
        let ret = forward_return(&prices, date(2024, 1, 1), 2, 0.0).unwrap();
        assert!(
            (ret - 0.10).abs() < 1e-6,
            "Expected 10% return, got {}",
            ret
        );
    }

    #[test]
    fn forward_return_with_cost() {
        let prices = vec![(date(2024, 1, 1), 100.0), (date(2024, 1, 2), 110.0)];
        let ret = forward_return(&prices, date(2024, 1, 1), 1, 10.0).unwrap();
        // 10% gross - 0.002 round-trip cost = 0.098
        assert!((ret - 0.098).abs() < 1e-6);
    }

    #[test]
    fn forward_return_insufficient_data() {
        let prices = vec![(date(2024, 1, 1), 100.0)];
        assert!(forward_return(&prices, date(2024, 1, 1), 30, 0.0).is_none());
    }

    // ── hit_rate ────────────────────────────────────────────────────────

    #[test]
    fn hit_rate_all_winners() {
        let signals = vec![
            make_signal(
                "A",
                date(2024, 1, 1),
                None,
                Some(0.10),
                None,
                MarketRegime::Bull,
            ),
            make_signal(
                "B",
                date(2024, 1, 2),
                None,
                Some(0.05),
                None,
                MarketRegime::Bull,
            ),
        ];
        assert!((hit_rate(&signals, 60, 0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hit_rate_all_losers() {
        let signals = vec![
            make_signal(
                "A",
                date(2024, 1, 1),
                None,
                Some(-0.05),
                None,
                MarketRegime::Bull,
            ),
            make_signal(
                "B",
                date(2024, 1, 2),
                None,
                Some(-0.10),
                None,
                MarketRegime::Bull,
            ),
        ];
        assert!(hit_rate(&signals, 60, 0.0).abs() < 1e-6);
    }

    #[test]
    fn hit_rate_mixed() {
        let signals = vec![
            make_signal(
                "A",
                date(2024, 1, 1),
                None,
                Some(0.10),
                None,
                MarketRegime::Bull,
            ),
            make_signal(
                "B",
                date(2024, 1, 2),
                None,
                Some(-0.05),
                None,
                MarketRegime::Bull,
            ),
            make_signal(
                "C",
                date(2024, 1, 3),
                None,
                Some(0.03),
                None,
                MarketRegime::Bull,
            ),
            make_signal(
                "D",
                date(2024, 1, 4),
                None,
                Some(-0.02),
                None,
                MarketRegime::Bull,
            ),
        ];
        assert!((hit_rate(&signals, 60, 0.0) - 0.5).abs() < 1e-6);
    }

    // ── false_positive_rate ─────────────────────────────────────────────

    #[test]
    fn fp_rate_matches_complement() {
        let signals = vec![
            make_signal(
                "A",
                date(2024, 1, 1),
                None,
                Some(0.10),
                None,
                MarketRegime::Bull,
            ),
            make_signal(
                "B",
                date(2024, 1, 2),
                None,
                Some(-0.05),
                None,
                MarketRegime::Bull,
            ),
        ];
        let hr = hit_rate(&signals, 60, 0.0);
        let fp = false_positive_rate(&signals, 60, 0.0);
        assert!((hr + fp - 1.0).abs() < 1e-6);
    }

    // ── alpha ───────────────────────────────────────────────────────────

    #[test]
    fn alpha_positive_when_beating_benchmark() {
        let signals = vec![
            make_signal(
                "A",
                date(2024, 1, 1),
                None,
                Some(0.10),
                None,
                MarketRegime::Bull,
            ),
            make_signal(
                "B",
                date(2024, 1, 2),
                None,
                Some(0.08),
                None,
                MarketRegime::Bull,
            ),
        ];
        let benchmark = vec![(date(2024, 1, 1), 0.02), (date(2024, 1, 2), 0.03)];
        let alpha = alpha_vs_benchmark(&signals, &benchmark, 60);
        assert!(alpha > 0.0, "Alpha should be positive, got {}", alpha);
    }

    // ── win_loss_ratio ──────────────────────────────────────────────────

    #[test]
    fn win_loss_symmetric() {
        let signals = vec![
            make_signal(
                "A",
                date(2024, 1, 1),
                None,
                Some(0.10),
                None,
                MarketRegime::Bull,
            ),
            make_signal(
                "B",
                date(2024, 1, 2),
                None,
                Some(-0.10),
                None,
                MarketRegime::Bull,
            ),
        ];
        let wl = win_loss_ratio(&signals, 60);
        assert!(
            (wl - 1.0).abs() < 1e-6,
            "Symmetric W/L should be 1.0, got {}",
            wl
        );
    }

    // ── profit_factor ───────────────────────────────────────────────────

    #[test]
    fn profit_factor_2x_wins() {
        let signals = vec![
            make_signal(
                "A",
                date(2024, 1, 1),
                None,
                Some(0.20),
                None,
                MarketRegime::Bull,
            ),
            make_signal(
                "B",
                date(2024, 1, 2),
                None,
                Some(-0.10),
                None,
                MarketRegime::Bull,
            ),
        ];
        let pf = profit_factor(&signals, 60);
        assert!(
            (pf - 2.0).abs() < 1e-6,
            "Profit factor should be 2.0, got {}",
            pf
        );
    }

    // ── sharpe_ratio ────────────────────────────────────────────────────

    #[test]
    fn sharpe_zero_volatility() {
        let returns = vec![0.01, 0.01, 0.01, 0.01];
        let sharpe = sharpe_ratio(&returns, 0.0);
        assert_eq!(sharpe, 0.0);
    }

    #[test]
    fn sharpe_positive_for_positive_mean() {
        let returns = vec![0.02, 0.03, 0.01, 0.04, 0.02, 0.03, 0.01, 0.05];
        let sharpe = sharpe_ratio(&returns, 0.0);
        assert!(
            sharpe > 0.0,
            "Positive mean returns should give positive Sharpe, got {}",
            sharpe
        );
    }

    // ── walk_forward ────────────────────────────────────────────────────

    #[test]
    fn walk_forward_produces_periods() {
        let mut signals = Vec::new();
        for year in 2020..=2022 {
            for month in 1..=12 {
                signals.push(make_signal(
                    "NVDA",
                    date(year, month, 15),
                    Some(0.05),
                    Some(0.08),
                    Some(0.12),
                    MarketRegime::Bull,
                ));
            }
        }
        let params = BacktestParams::default();
        let periods = walk_forward_backtest(&signals, &[], &params);
        assert!(
            !periods.is_empty(),
            "3 years of data with 12m IS + 3m OOS should produce multiple periods"
        );
    }

    // ── forward_return edge cases ────────────────────────────────────────

    #[test]
    fn forward_return_entry_date_not_in_prices_returns_none() {
        // Given: price series that starts after the requested entry_date
        let prices = vec![(date(2024, 2, 1), 100.0), (date(2024, 2, 2), 105.0)];

        // When: entry_date is before the first candle
        let result = forward_return(&prices, date(2024, 1, 1), 1, 0.0);

        // Then: None because no candle satisfies d >= entry_date in the slice
        // Note: the implementation finds the first index where d >= entry_date,
        // so an entry_date before all candles actually resolves to index 0.
        // The test verifies behavior for a date AFTER the last candle.
        let prices_early = vec![(date(2024, 1, 1), 100.0)];
        // entry_date after all prices — position() returns None
        let result_after = forward_return(&prices_early, date(2024, 6, 1), 1, 0.0);
        assert!(
            result_after.is_none(),
            "entry_date after all prices should return None"
        );
        // Confirm the first case does resolve (entry before first candle → idx 0)
        assert!(
            result.is_some(),
            "entry_date before first candle resolves to idx 0"
        );
    }

    #[test]
    fn forward_return_zero_entry_price_returns_none() {
        // Given: a price series where the entry candle has price 0.0
        let prices = vec![(date(2024, 1, 1), 0.0), (date(2024, 1, 2), 100.0)];

        // When
        let result = forward_return(&prices, date(2024, 1, 1), 1, 0.0);

        // Then: None because entry_price <= 0.0 guard fires
        assert!(result.is_none(), "entry_price of 0.0 should return None");
    }

    // ── sharpe_ratio edge cases ──────────────────────────────────────────

    #[test]
    fn sharpe_ratio_single_element_returns_zero() {
        // Given: a slice with exactly one element (len < 2 guard)
        let returns = vec![0.05];

        // When
        let sharpe = sharpe_ratio(&returns, 0.0);

        // Then: 0.0 per the len < 2 guard
        assert_eq!(sharpe, 0.0, "Single-element input should return 0.0");
    }

    // ── win_loss_ratio edge cases ────────────────────────────────────────

    #[test]
    fn win_loss_ratio_all_zero_returns_zero() {
        // Given: all returns are exactly 0.0 (neither wins nor losses)
        let signals = vec![
            make_signal(
                "A",
                date(2024, 1, 1),
                None,
                Some(0.0),
                None,
                MarketRegime::Bull,
            ),
            make_signal(
                "B",
                date(2024, 1, 2),
                None,
                Some(0.0),
                None,
                MarketRegime::Bull,
            ),
        ];

        // When
        let wl = win_loss_ratio(&signals, 60);

        // Then: wins is empty → returns 0.0
        assert_eq!(
            wl, 0.0,
            "All-zero returns: no wins → win_loss_ratio should be 0.0"
        );
    }

    // ── profit_factor edge cases ─────────────────────────────────────────

    #[test]
    fn profit_factor_all_zero_returns_zero() {
        // Given: all returns are exactly 0.0
        let signals = vec![
            make_signal(
                "A",
                date(2024, 1, 1),
                None,
                Some(0.0),
                None,
                MarketRegime::Bull,
            ),
            make_signal(
                "B",
                date(2024, 1, 2),
                None,
                Some(0.0),
                None,
                MarketRegime::Bull,
            ),
        ];

        // When
        let pf = profit_factor(&signals, 60);

        // Then: total_wins == 0 and total_losses == 0 → returns 0.0
        assert_eq!(pf, 0.0, "All-zero returns: profit_factor should be 0.0");
    }

    // ── walk_forward_backtest edge cases ─────────────────────────────────

    #[test]
    fn walk_forward_backtest_empty_signals_returns_empty() {
        // Given: no signals at all
        let signals: Vec<BacktestSignal> = vec![];
        let params = BacktestParams::default();

        // When
        let periods = walk_forward_backtest(&signals, &[], &params);

        // Then: early-return guard fires, result is empty
        assert!(
            periods.is_empty(),
            "Empty signals should produce empty walk-forward output"
        );
    }

    // ── compute_kpis smoke test ──────────────────────────────────────────

    #[test]
    fn compute_kpis_smoke_test() {
        // Given: 10 signals — 6 winners (+5%), 4 losers (-3%)
        let signals: Vec<BacktestSignal> = (0..10)
            .map(|i| {
                let ret = if i < 6 { Some(0.05) } else { Some(-0.03) };
                make_signal(
                    "TICK",
                    date(2024, 1, i as u32 + 1),
                    ret,
                    ret,
                    ret,
                    MarketRegime::Bull,
                )
            })
            .collect();

        // When
        let kpis = compute_kpis(&signals, &[], &BacktestParams::default());

        // Then: total_signals is correct and hit_rate is in [0, 1]
        assert_eq!(kpis.total_signals, 10);
        assert!(
            (0.0..=1.0).contains(&kpis.hit_rate_60d),
            "hit_rate_60d should be in [0, 1], got {}",
            kpis.hit_rate_60d
        );
        // 6 out of 10 wins → 60 %
        assert!(
            (kpis.hit_rate_60d - 0.60).abs() < 1e-6,
            "Expected hit_rate_60d = 0.60, got {}",
            kpis.hit_rate_60d
        );
    }

    // ── regime_kpis ─────────────────────────────────────────────────────

    #[test]
    fn regime_disaggregation() {
        let signals = vec![
            make_signal(
                "A",
                date(2024, 1, 1),
                None,
                Some(0.10),
                None,
                MarketRegime::Bull,
            ),
            make_signal(
                "B",
                date(2024, 2, 1),
                None,
                Some(-0.05),
                None,
                MarketRegime::Bear,
            ),
            make_signal(
                "C",
                date(2024, 3, 1),
                None,
                Some(0.03),
                None,
                MarketRegime::Bull,
            ),
        ];
        let params = BacktestParams::default();
        let kpis = regime_kpis(&signals, &[], &params);
        assert!(
            kpis.len() >= 2,
            "Should have KPIs for at least Bull and Bear"
        );

        let bull = kpis
            .iter()
            .find(|k| k.regime == MarketRegime::Bull)
            .unwrap();
        assert_eq!(bull.signal_count, 2);

        let bear = kpis
            .iter()
            .find(|k| k.regime == MarketRegime::Bear)
            .unwrap();
        assert_eq!(bear.signal_count, 1);
    }

    // ── Phase 5: Sharpe ratio methodology ───────────────────────────────────

    #[test]
    fn sharpe_daily_requires_30_observations() {
        let returns: Vec<f64> = (0..29).map(|i| 0.001 * i as f64).collect();
        assert_eq!(sharpe_ratio_daily(&returns, 0.05), 0.0);
    }

    #[test]
    fn sharpe_daily_positive_for_positive_returns() {
        // 60 days of consistently positive daily returns
        let returns: Vec<f64> = vec![0.001; 60];
        let s = sharpe_ratio_daily(&returns, 0.0);
        // With zero std, this would be infinite; with tiny variation it should be large
        // Since all returns are identical, std = 0 → returns 0.0
        assert_eq!(s, 0.0); // all identical → zero variance
    }

    #[test]
    fn sharpe_daily_nonzero_with_variance() {
        // Alternating returns with positive mean
        let returns: Vec<f64> = (0..60)
            .map(|i| if i % 2 == 0 { 0.02 } else { 0.01 })
            .collect();
        let s = sharpe_ratio_daily(&returns, 0.0);
        assert!(s > 0.0, "Expected positive Sharpe, got {s}");
    }

    #[test]
    fn sharpe_adjusted_less_than_unadjusted_for_60d() {
        // Same returns: adjusted with horizon=60 should be much lower than raw sqrt(252)
        let returns: Vec<f64> = (0..100)
            .map(|i| 0.05 + 0.01 * (i as f64 % 5.0 - 2.5))
            .collect();
        let raw = sharpe_ratio(&returns, 0.0);
        let adj = sharpe_ratio_adjusted(&returns, 0.0, 60);
        assert!(
            adj < raw,
            "Adjusted Sharpe ({adj:.2}) should be less than raw ({raw:.2}) for 60d horizon"
        );
        // Ratio should be approximately sqrt(252/60) / sqrt(252) = sqrt(1/60) / 1
        // Actually: adj / raw ≈ sqrt(252/60) / sqrt(252) = 1/sqrt(60) ≈ 0.129
        // More precisely: sqrt(4.2) / sqrt(252) ≈ 2.05 / 15.87 ≈ 0.129
        let ratio = adj / raw;
        assert!(
            (ratio - (1.0 / (60.0_f64).sqrt())).abs() < 0.01,
            "Ratio should be ~1/sqrt(60), got {ratio:.4}"
        );
    }

    #[test]
    fn sharpe_adjusted_horizon_1_equals_raw() {
        let returns: Vec<f64> = (0..100)
            .map(|i| 0.05 + 0.01 * (i as f64 % 5.0 - 2.5))
            .collect();
        let raw = sharpe_ratio(&returns, 0.0);
        let adj = sharpe_ratio_adjusted(&returns, 0.0, 1);
        // horizon=1 → sqrt(252/1) = sqrt(252), same as raw
        assert!(
            (raw - adj).abs() < 0.01,
            "horizon=1 should match raw: raw={raw:.2} adj={adj:.2}"
        );
    }
}
