//! Performance attribution — Brinson-Fachler variant decomposing active return
//! into stock selection, timing, sizing, strategy, and interaction effects.
//!
//! Phase 6 (F25): Answers "where did alpha come from?" by separating the
//! contributions of stock picking, market timing, position sizing, and
//! strategy weighting decisions.

use chrono::NaiveDate;

// ─── Types ──────────────────────────────────────────────────────────────────

/// A single trade record for attribution analysis.
#[derive(Debug, Clone)]
pub struct TradeRecord {
    pub ticker: String,
    pub strategy: String,
    pub weight: f64,
    pub realized_return: f64,
}

/// Performance attribution decomposition.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AttributionResult {
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
    /// Total portfolio return over the period.
    pub total_return: f64,
    /// Benchmark (SPY buy-and-hold) return.
    pub benchmark_return: f64,
    /// Active return = total - benchmark.
    pub active_return: f64,
    /// Alpha from picking stocks that outperform the benchmark.
    pub selection_effect: f64,
    /// Alpha from being invested only when signals are active vs always-invested.
    pub timing_effect: f64,
    /// Alpha from actual position sizes vs equal-weighting all signals.
    pub sizing_effect: f64,
    /// Alpha from adaptive strategy weighting vs equal-weighting strategies.
    pub strategy_effect: f64,
    /// Residual cross-term.
    pub interaction_effect: f64,
}

// ─── Computation ────────────────────────────────────────────────────────────

/// Compute attribution for a set of trades against a benchmark.
///
/// This is a simplified Brinson-Fachler variant that decomposes active return
/// into five additive components.
pub fn compute_attribution(
    trades: &[TradeRecord],
    benchmark_return: f64,
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> AttributionResult {
    if trades.is_empty() {
        return AttributionResult {
            period_start,
            period_end,
            total_return: 0.0,
            benchmark_return,
            active_return: -benchmark_return,
            selection_effect: 0.0,
            timing_effect: -benchmark_return,
            sizing_effect: 0.0,
            strategy_effect: 0.0,
            interaction_effect: 0.0,
        };
    }

    // Actual weighted return
    let total_return: f64 = trades.iter().map(|t| t.realized_return * t.weight).sum();
    let active_return = total_return - benchmark_return;

    // Selection: average stock return vs benchmark (stock picking alpha)
    let avg_stock_return =
        trades.iter().map(|t| t.realized_return).sum::<f64>() / trades.len() as f64;
    let selection_effect = avg_stock_return - benchmark_return;

    // APPROXIMATION: Timing effect uses total weight sum as a proxy for
    // "fraction of period invested". A proper implementation would track daily
    // invested fraction over the period. This is adequate for directional
    // comparison but not for precise attribution reporting.
    let invested_weight: f64 = trades.iter().map(|t| t.weight).sum();
    let timing_effect = (1.0 - invested_weight.min(1.0)) * (-benchmark_return);

    // Sizing: actual weighted return vs equal-weighted return
    let equal_w = 1.0 / trades.len() as f64;
    let equal_weighted_return: f64 = trades.iter().map(|t| t.realized_return * equal_w).sum();
    let sizing_effect = total_return - equal_weighted_return;

    // Strategy: group by strategy, compare actual strategy weights vs equal
    let strategy_effect = compute_strategy_effect(trades);

    // Interaction: residual
    let interaction_effect =
        active_return - selection_effect - timing_effect - sizing_effect - strategy_effect;

    AttributionResult {
        period_start,
        period_end,
        total_return,
        benchmark_return,
        active_return,
        selection_effect,
        timing_effect,
        sizing_effect,
        strategy_effect,
        interaction_effect,
    }
}

/// Strategy weighting effect: compare actual strategy-group returns vs equal-strategy returns.
fn compute_strategy_effect(trades: &[TradeRecord]) -> f64 {
    use std::collections::HashMap;

    if trades.is_empty() {
        return 0.0;
    }

    // Group trades by strategy
    let mut by_strategy: HashMap<&str, Vec<&TradeRecord>> = HashMap::new();
    for t in trades {
        by_strategy.entry(t.strategy.as_str()).or_default().push(t);
    }

    if by_strategy.len() <= 1 {
        return 0.0; // single strategy = no strategy effect
    }

    // Actual strategy-weighted return (each trade already has its weight)
    let actual_return: f64 = trades.iter().map(|t| t.realized_return * t.weight).sum();

    // Equal-strategy return: each strategy gets equal allocation, within-strategy equal-weighted
    let strategy_equal_weight = 1.0 / by_strategy.len() as f64;
    let equal_strategy_return: f64 = by_strategy
        .values()
        .map(|group| {
            let group_avg =
                group.iter().map(|t| t.realized_return).sum::<f64>() / group.len() as f64;
            group_avg * strategy_equal_weight
        })
        .sum();

    actual_return - equal_strategy_return
}

/// Compound a vector of period returns.
pub fn compound_returns(returns: &[f64]) -> f64 {
    returns.iter().fold(1.0, |acc, r| acc * (1.0 + r)) - 1.0
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn sample_trades() -> Vec<TradeRecord> {
        vec![
            TradeRecord {
                ticker: "NVDA".into(),
                strategy: "minervini".into(),
                weight: 0.30,
                realized_return: 0.12,
            },
            TradeRecord {
                ticker: "AMD".into(),
                strategy: "minervini".into(),
                weight: 0.25,
                realized_return: 0.08,
            },
            TradeRecord {
                ticker: "CRWD".into(),
                strategy: "breakout".into(),
                weight: 0.20,
                realized_return: 0.15,
            },
            TradeRecord {
                ticker: "PANW".into(),
                strategy: "breakout".into(),
                weight: 0.15,
                realized_return: -0.03,
            },
            TradeRecord {
                ticker: "DDOG".into(),
                strategy: "trend-strength".into(),
                weight: 0.10,
                realized_return: 0.05,
            },
        ]
    }

    #[test]
    fn attribution_components_sum_to_active_return() {
        let trades = sample_trades();
        let result = compute_attribution(&trades, 0.05, d("2026-01-01"), d("2026-03-31"));

        let component_sum = result.selection_effect
            + result.timing_effect
            + result.sizing_effect
            + result.strategy_effect
            + result.interaction_effect;

        assert!(
            (component_sum - result.active_return).abs() < 0.001,
            "Components ({component_sum:.4}) should sum to active return ({:.4})",
            result.active_return
        );
    }

    #[test]
    fn empty_trades_timing_negative() {
        let result = compute_attribution(&[], 0.05, d("2026-01-01"), d("2026-03-31"));
        assert!((result.active_return - (-0.05)).abs() < 0.001);
        assert!((result.timing_effect - (-0.05)).abs() < 0.001);
    }

    #[test]
    fn all_cash_has_zero_selection() {
        let result = compute_attribution(&[], 0.05, d("2026-01-01"), d("2026-03-31"));
        assert!((result.selection_effect - 0.0).abs() < 0.001);
    }

    #[test]
    fn equal_weighted_trades_have_zero_sizing_effect() {
        let trades = vec![
            TradeRecord {
                ticker: "A".into(),
                strategy: "s".into(),
                weight: 0.25,
                realized_return: 0.10,
            },
            TradeRecord {
                ticker: "B".into(),
                strategy: "s".into(),
                weight: 0.25,
                realized_return: 0.05,
            },
            TradeRecord {
                ticker: "C".into(),
                strategy: "s".into(),
                weight: 0.25,
                realized_return: 0.08,
            },
            TradeRecord {
                ticker: "D".into(),
                strategy: "s".into(),
                weight: 0.25,
                realized_return: 0.12,
            },
        ];
        let result = compute_attribution(&trades, 0.05, d("2026-01-01"), d("2026-03-31"));
        assert!(
            result.sizing_effect.abs() < 0.001,
            "Equal weights should have ~0 sizing effect, got {:.4}",
            result.sizing_effect
        );
    }

    #[test]
    fn overweighting_winner_positive_sizing() {
        let trades = vec![
            TradeRecord {
                ticker: "WIN".into(),
                strategy: "s".into(),
                weight: 0.70,
                realized_return: 0.20,
            },
            TradeRecord {
                ticker: "LOSE".into(),
                strategy: "s".into(),
                weight: 0.30,
                realized_return: -0.10,
            },
        ];
        let result = compute_attribution(&trades, 0.05, d("2026-01-01"), d("2026-03-31"));
        assert!(
            result.sizing_effect > 0.0,
            "Overweighting winner should have positive sizing"
        );
    }

    #[test]
    fn compound_returns_basic() {
        let returns = vec![0.10, -0.05, 0.08];
        let total = compound_returns(&returns);
        // (1.10)(0.95)(1.08) - 1 = 1.1286 - 1 = 0.1286
        assert!((total - 0.1286).abs() < 0.001);
    }

    #[test]
    fn compound_returns_empty() {
        assert!((compound_returns(&[]) - 0.0).abs() < 0.001);
    }

    #[test]
    fn selection_positive_when_stocks_outperform() {
        let trades = vec![
            TradeRecord {
                ticker: "A".into(),
                strategy: "s".into(),
                weight: 0.5,
                realized_return: 0.15,
            },
            TradeRecord {
                ticker: "B".into(),
                strategy: "s".into(),
                weight: 0.5,
                realized_return: 0.10,
            },
        ];
        let result = compute_attribution(&trades, 0.05, d("2026-01-01"), d("2026-03-31"));
        assert!(
            result.selection_effect > 0.0,
            "Stocks outperforming benchmark = positive selection"
        );
    }

    #[test]
    fn single_strategy_has_zero_strategy_effect() {
        let trades = vec![
            TradeRecord {
                ticker: "A".into(),
                strategy: "minervini".into(),
                weight: 0.5,
                realized_return: 0.10,
            },
            TradeRecord {
                ticker: "B".into(),
                strategy: "minervini".into(),
                weight: 0.5,
                realized_return: 0.08,
            },
        ];
        let result = compute_attribution(&trades, 0.05, d("2026-01-01"), d("2026-03-31"));
        assert!(
            result.strategy_effect.abs() < 0.001,
            "Single strategy = no strategy effect, got {:.4}",
            result.strategy_effect
        );
    }
}
