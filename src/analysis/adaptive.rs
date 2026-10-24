//! Adaptive strategy selection engine — the Phase 6 orchestrator.
//!
//! Combines market regime, microstructure state, and timeframe effectiveness
//! to produce a blended strategy weight vector that determines which screeners
//! to emphasize in the current market environment.

use std::collections::HashMap;

use crate::analysis::microstructure::{MicrostructureState, VolRegime};
use crate::analysis::multi_horizon::TimeframeWeights;
use crate::data::models::MarketRegime;

// ─── Strategy-Horizon Affinity Matrix ───────────────────────────────────────

/// Static affinity: how well each strategy fits each time horizon.
/// Values in [0.0, 1.0]. This is fixed — it reflects structural design, not performance.
#[derive(Debug, Clone)]
pub struct StrategyHorizonAffinity {
    pub strategy: &'static str,
    pub swing: f64,
    pub position: f64,
    pub trend: f64,
}

pub const AFFINITY_MATRIX: &[StrategyHorizonAffinity] = &[
    StrategyHorizonAffinity {
        strategy: "minervini",
        swing: 0.3,
        position: 0.8,
        trend: 0.6,
    },
    StrategyHorizonAffinity {
        strategy: "breakout",
        swing: 0.9,
        position: 0.5,
        trend: 0.2,
    },
    StrategyHorizonAffinity {
        strategy: "mean-reversion",
        swing: 0.8,
        position: 0.4,
        trend: 0.1,
    },
    StrategyHorizonAffinity {
        strategy: "trend-strength",
        swing: 0.2,
        position: 0.6,
        trend: 0.9,
    },
];

// ─── Strategy Weights ───────────────────────────────────────────────────────

/// Normalized strategy weight allocation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StrategyWeights {
    pub weights: HashMap<String, f64>,
}

impl StrategyWeights {
    /// Get weight for a strategy, defaulting to 0.
    pub fn get(&self, strategy: &str) -> f64 {
        self.weights.get(strategy).copied().unwrap_or(0.0)
    }

    /// Check that weights sum to approximately 1.0.
    pub fn sum(&self) -> f64 {
        self.weights.values().sum()
    }
}

// ─── Regime Modifiers ───────────────────────────────────────────────────────

fn regime_modifiers(regime: &MarketRegime) -> HashMap<&'static str, f64> {
    match regime {
        MarketRegime::Bull => [
            ("minervini", 1.0),
            ("breakout", 1.2),
            ("mean-reversion", 0.6),
            ("trend-strength", 1.1),
        ]
        .into_iter()
        .collect(),
        MarketRegime::Sideways => [
            ("minervini", 1.0),
            ("breakout", 0.7),
            ("mean-reversion", 1.3),
            ("trend-strength", 0.8),
        ]
        .into_iter()
        .collect(),
        MarketRegime::Bear => [
            ("minervini", 0.8),
            ("breakout", 0.3),
            ("mean-reversion", 1.0),
            ("trend-strength", 0.5),
        ]
        .into_iter()
        .collect(),
    }
}

fn vol_adjustments(vol_regime: &VolRegime) -> HashMap<&'static str, f64> {
    match vol_regime {
        VolRegime::Low => [
            ("breakout", 1.2),
            ("mean-reversion", 0.8),
            ("trend-strength", 1.0),
            ("minervini", 1.0),
        ]
        .into_iter()
        .collect(),
        VolRegime::Normal => [
            ("breakout", 1.0),
            ("mean-reversion", 1.0),
            ("trend-strength", 1.0),
            ("minervini", 1.0),
        ]
        .into_iter()
        .collect(),
        VolRegime::High => [
            ("breakout", 0.6),
            ("mean-reversion", 1.3),
            ("trend-strength", 0.7),
            ("minervini", 0.9),
        ]
        .into_iter()
        .collect(),
    }
}

// ─── Core Computation ───────────────────────────────────────────────────────

/// Compute blended strategy weights from current market state.
///
/// weight[strategy] = sum over horizons of:
///   affinity[strategy][horizon] × horizon_weight[horizon] × regime_mod × vol_adj
///
/// Then normalize to sum to 1.0 with guardrails (5% floor, 60% ceiling).
pub fn compute_strategy_weights(
    regime: &MarketRegime,
    microstructure: &MicrostructureState,
    tf_weights: &TimeframeWeights,
) -> StrategyWeights {
    let regime_mod = regime_modifiers(regime);
    let vol_adj = vol_adjustments(&microstructure.vol_regime);

    let mut raw_weights: HashMap<String, f64> = HashMap::new();

    for entry in AFFINITY_MATRIX {
        let horizon_score = entry.swing * tf_weights.swing
            + entry.position * tf_weights.position
            + entry.trend * tf_weights.trend;

        let r_mod = regime_mod.get(entry.strategy).copied().unwrap_or(1.0);
        let v_adj = vol_adj.get(entry.strategy).copied().unwrap_or(1.0);

        raw_weights.insert(entry.strategy.to_string(), horizon_score * r_mod * v_adj);
    }

    // Normalize to sum to 1.0
    let total: f64 = raw_weights.values().sum();
    if total > 0.0 {
        for v in raw_weights.values_mut() {
            *v /= total;
        }
    }

    // Apply guardrails: 5% floor, 60% ceiling
    apply_guardrails(&mut raw_weights, 0.05, 0.60);

    StrategyWeights {
        weights: raw_weights,
    }
}

/// Clamp all weights to [min, max] and renormalize.
fn apply_guardrails(weights: &mut HashMap<String, f64>, min: f64, max: f64) {
    // Iterative clamping + renormalization (converges in 3-4 iterations)
    for _ in 0..5 {
        for v in weights.values_mut() {
            *v = v.clamp(min, max);
        }
        let total: f64 = weights.values().sum();
        if total > 0.0 {
            for v in weights.values_mut() {
                *v /= total;
            }
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::microstructure::{
        CorrRegime, LiquidityEvent, MicrostructureState, VolRegime,
    };
    use chrono::NaiveDate;

    fn normal_micro() -> MicrostructureState {
        MicrostructureState {
            vol_regime: VolRegime::Normal,
            corr_regime: CorrRegime::Normal,
            liquidity_event: LiquidityEvent::Normal,
            realized_vol_20d: 15.0,
            avg_sector_correlation: 0.50,
            as_of: NaiveDate::from_ymd_opt(2026, 3, 10).unwrap(),
        }
    }

    // ── basic properties ────────────────────────────────────────────────────

    #[test]
    fn weights_sum_to_1() {
        let w = compute_strategy_weights(
            &MarketRegime::Bull,
            &normal_micro(),
            &TimeframeWeights::equal(),
        );
        assert!(
            (w.sum() - 1.0).abs() < 0.02,
            "Sum should be ~1.0, got {}",
            w.sum()
        );
    }

    #[test]
    fn all_strategies_have_positive_weight() {
        for regime in &[
            MarketRegime::Bull,
            MarketRegime::Sideways,
            MarketRegime::Bear,
        ] {
            let w = compute_strategy_weights(regime, &normal_micro(), &TimeframeWeights::equal());
            for entry in AFFINITY_MATRIX {
                let wt = w.get(entry.strategy);
                assert!(
                    wt > 0.0,
                    "{:?} regime: {} has zero weight",
                    regime,
                    entry.strategy
                );
            }
        }
    }

    #[test]
    fn weights_contain_all_four_strategies() {
        let w = compute_strategy_weights(
            &MarketRegime::Bull,
            &normal_micro(),
            &TimeframeWeights::equal(),
        );
        assert_eq!(w.weights.len(), 4);
        assert!(w.weights.contains_key("minervini"));
        assert!(w.weights.contains_key("breakout"));
        assert!(w.weights.contains_key("mean-reversion"));
        assert!(w.weights.contains_key("trend-strength"));
    }

    // ── regime effects ──────────────────────────────────────────────────────

    #[test]
    fn bull_favors_breakout() {
        let w = compute_strategy_weights(
            &MarketRegime::Bull,
            &normal_micro(),
            &TimeframeWeights::equal(),
        );
        assert!(
            w.get("breakout") > w.get("mean-reversion"),
            "Bull should favor breakout ({:.3}) over mean-reversion ({:.3})",
            w.get("breakout"),
            w.get("mean-reversion")
        );
    }

    #[test]
    fn sideways_favors_mean_reversion() {
        let w = compute_strategy_weights(
            &MarketRegime::Sideways,
            &normal_micro(),
            &TimeframeWeights::equal(),
        );
        assert!(
            w.get("mean-reversion") > w.get("breakout"),
            "Sideways should favor mean-reversion ({:.3}) over breakout ({:.3})",
            w.get("mean-reversion"),
            w.get("breakout")
        );
    }

    #[test]
    fn bear_reduces_breakout() {
        let bull = compute_strategy_weights(
            &MarketRegime::Bull,
            &normal_micro(),
            &TimeframeWeights::equal(),
        );
        let bear = compute_strategy_weights(
            &MarketRegime::Bear,
            &normal_micro(),
            &TimeframeWeights::equal(),
        );
        assert!(
            bear.get("breakout") < bull.get("breakout"),
            "Bear breakout ({:.3}) should be less than bull breakout ({:.3})",
            bear.get("breakout"),
            bull.get("breakout")
        );
    }

    // ── volatility effects ──────────────────────────────────────────────────

    #[test]
    fn high_vol_boosts_mean_reversion() {
        let mut normal = normal_micro();
        let w_normal =
            compute_strategy_weights(&MarketRegime::Sideways, &normal, &TimeframeWeights::equal());

        normal.vol_regime = VolRegime::High;
        let w_high =
            compute_strategy_weights(&MarketRegime::Sideways, &normal, &TimeframeWeights::equal());

        assert!(
            w_high.get("mean-reversion") > w_normal.get("mean-reversion"),
            "High vol should boost mean-reversion"
        );
    }

    #[test]
    fn low_vol_boosts_breakout() {
        let mut micro = normal_micro();
        let w_normal =
            compute_strategy_weights(&MarketRegime::Bull, &micro, &TimeframeWeights::equal());

        micro.vol_regime = VolRegime::Low;
        let w_low =
            compute_strategy_weights(&MarketRegime::Bull, &micro, &TimeframeWeights::equal());

        assert!(
            w_low.get("breakout") >= w_normal.get("breakout"),
            "Low vol should boost breakout"
        );
    }

    // ── timeframe effects ───────────────────────────────────────────────────

    #[test]
    fn swing_timeframe_boosts_breakout() {
        let swing_heavy = TimeframeWeights {
            swing: 0.6,
            position: 0.2,
            trend: 0.2,
        };
        let trend_heavy = TimeframeWeights {
            swing: 0.2,
            position: 0.2,
            trend: 0.6,
        };

        let w_swing = compute_strategy_weights(&MarketRegime::Bull, &normal_micro(), &swing_heavy);
        let w_trend = compute_strategy_weights(&MarketRegime::Bull, &normal_micro(), &trend_heavy);

        assert!(
            w_swing.get("breakout") > w_trend.get("breakout"),
            "Swing timeframe should favor breakout more"
        );
    }

    #[test]
    fn trend_timeframe_boosts_trend_strength() {
        let trend_heavy = TimeframeWeights {
            swing: 0.1,
            position: 0.2,
            trend: 0.7,
        };
        let swing_heavy = TimeframeWeights {
            swing: 0.7,
            position: 0.2,
            trend: 0.1,
        };

        let w_trend = compute_strategy_weights(&MarketRegime::Bull, &normal_micro(), &trend_heavy);
        let w_swing = compute_strategy_weights(&MarketRegime::Bull, &normal_micro(), &swing_heavy);

        assert!(
            w_trend.get("trend-strength") > w_swing.get("trend-strength"),
            "Trend timeframe should favor trend-strength more"
        );
    }

    // ── guardrails ──────────────────────────────────────────────────────────

    #[test]
    fn no_strategy_below_5_percent() {
        // Bear + high vol = worst for breakout. Should still have >= 5%.
        let mut micro = normal_micro();
        micro.vol_regime = VolRegime::High;
        let w = compute_strategy_weights(&MarketRegime::Bear, &micro, &TimeframeWeights::equal());
        for (strategy, weight) in &w.weights {
            assert!(
                *weight >= 0.04,
                "{strategy} weight {weight:.3} below minimum"
            ); // slight tolerance for renorm
        }
    }

    #[test]
    fn no_strategy_above_60_percent() {
        // Even with extreme inputs, no single strategy should dominate
        let micro = normal_micro();
        let extreme = TimeframeWeights {
            swing: 0.9,
            position: 0.05,
            trend: 0.05,
        };
        let w = compute_strategy_weights(&MarketRegime::Bull, &micro, &extreme);
        for (strategy, weight) in &w.weights {
            assert!(
                *weight <= 0.65,
                "{strategy} weight {weight:.3} above maximum"
            ); // slight tolerance
        }
    }

    // ── different inputs produce different weights ──────────────────────────

    #[test]
    fn different_regimes_different_weights() {
        let bull = compute_strategy_weights(
            &MarketRegime::Bull,
            &normal_micro(),
            &TimeframeWeights::equal(),
        );
        let bear = compute_strategy_weights(
            &MarketRegime::Bear,
            &normal_micro(),
            &TimeframeWeights::equal(),
        );
        // At least one strategy should have meaningfully different weight
        let max_diff = AFFINITY_MATRIX
            .iter()
            .map(|e| (bull.get(e.strategy) - bear.get(e.strategy)).abs())
            .fold(0.0_f64, f64::max);
        assert!(
            max_diff > 0.01,
            "Regimes should produce different weights, max_diff={max_diff}"
        );
    }
}
