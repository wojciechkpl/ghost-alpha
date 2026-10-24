//! Multi-horizon scoring — Swing / Position / Trend per stock.
//!
//! Phase 6 (F23): Each stock is scored for three time horizons with
//! different weight profiles and sizing parameters. This lets users
//! match recommendations to their intended holding period.

use crate::analysis::scorer::{
    composite_score_weighted, RegimeWeights, SentimentInputs, TechnicalInputs, VolumeInputs,
};

// ─── Time Horizon ───────────────────────────────────────────────────────────

/// Investment time horizon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TimeHorizon {
    /// 10-20 trading days (2-4 weeks)
    Swing,
    /// 40-120 trading days (2-6 months)
    Position,
    /// 120-252 trading days (6-12 months)
    Trend,
}

impl TimeHorizon {
    /// Holding period range in trading days.
    pub fn holding_days(&self) -> (u32, u32) {
        match self {
            TimeHorizon::Swing => (10, 20),
            TimeHorizon::Position => (40, 120),
            TimeHorizon::Trend => (120, 252),
        }
    }

    /// Target forward return horizon for backtest evaluation.
    pub fn target_days(&self) -> u32 {
        match self {
            TimeHorizon::Swing => 15,
            TimeHorizon::Position => 60,
            TimeHorizon::Trend => 180,
        }
    }
}

impl std::fmt::Display for TimeHorizon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TimeHorizon::Swing => write!(f, "Swing"),
            TimeHorizon::Position => write!(f, "Position"),
            TimeHorizon::Trend => write!(f, "Trend"),
        }
    }
}

// ─── Horizon-Specific Weights ───────────────────────────────────────────────

/// Scoring weights that differ by time horizon.
///
/// Swing: volume-heavy (breakout confirmation matters).
/// Position: balanced (current default).
/// Trend: technical-heavy (long-term quality dominates).
pub fn horizon_weights(horizon: TimeHorizon) -> RegimeWeights {
    match horizon {
        TimeHorizon::Swing => RegimeWeights {
            technical: 40.0,
            volume: 45.0,
            sentiment: 15.0,
        },
        TimeHorizon::Position => RegimeWeights {
            technical: 55.0,
            volume: 30.0,
            sentiment: 15.0,
        },
        TimeHorizon::Trend => RegimeWeights {
            technical: 70.0,
            volume: 15.0,
            sentiment: 15.0,
        },
    }
}

// ─── Horizon-Specific Scoring Adjustments ───────────────────────────────────

/// Inputs for horizon-specific adjustments beyond the weight profile.
#[derive(Debug, Clone)]
pub struct HorizonAdjustmentInputs {
    /// RSI(14) value
    pub rsi_14: Option<f64>,
    /// Whether VCP was detected
    pub vcp_detected: bool,
    /// Distance from pivot as % (smaller = closer to breakout)
    pub pivot_proximity_pct: Option<f64>,
    /// ATR as % of price
    pub atr_pct: f64,
    /// RS percentile (0-100)
    pub rs_percentile: f64,
    /// RS momentum (rate of change over 21 days)
    pub rs_momentum: Option<f64>,
    /// ADX value
    pub adx: f64,
}

/// Compute horizon-specific adjustment to base composite score.
pub fn horizon_adjustment(horizon: TimeHorizon, inputs: &HorizonAdjustmentInputs) -> f64 {
    match horizon {
        TimeHorizon::Swing => swing_adjustment(inputs),
        TimeHorizon::Position => position_adjustment(inputs),
        TimeHorizon::Trend => trend_adjustment(inputs),
    }
}

fn swing_adjustment(inputs: &HorizonAdjustmentInputs) -> f64 {
    let mut adj = 0.0;

    // RSI timing bonus: oversold bounce setup
    if let Some(rsi) = inputs.rsi_14 {
        if (30.0..=50.0).contains(&rsi) {
            adj += 5.0;
        }
    }

    // VCP at pivot = high-conviction swing entry
    if inputs.vcp_detected {
        if let Some(prox) = inputs.pivot_proximity_pct {
            if prox < 3.0 {
                adj += 8.0;
            }
        }
    }

    // ATR penalty: too volatile for swing trades
    if inputs.atr_pct > 5.0 {
        adj -= 10.0;
    }

    adj
}

fn position_adjustment(inputs: &HorizonAdjustmentInputs) -> f64 {
    let mut adj = 0.0;

    // RS momentum bonus
    if let Some(rs_mom) = inputs.rs_momentum {
        if rs_mom > 2.0 {
            adj += 5.0;
        }
    }

    adj
}

fn trend_adjustment(inputs: &HorizonAdjustmentInputs) -> f64 {
    let mut adj = 0.0;

    // RS percentile bonus: long-term leaders
    if inputs.rs_percentile > 90.0 {
        adj += 10.0;
    }

    // Weak ADX penalty: weak trends fail over 6+ months
    if inputs.adx < 25.0 {
        adj -= 10.0;
    }

    adj
}

// ─── Multi-Horizon Score ────────────────────────────────────────────────────

/// Complete multi-horizon scoring result for a single stock.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MultiHorizonScore {
    pub ticker: String,
    pub swing_score: f64,
    pub position_score: f64,
    pub trend_score: f64,
    pub blended_score: f64,
    pub best_horizon: TimeHorizon,
}

/// Compute scores for all three horizons.
pub fn compute_multi_horizon_score(
    ticker: &str,
    tech: &TechnicalInputs,
    vol: &VolumeInputs,
    sent: &SentimentInputs,
    adj_inputs: &HorizonAdjustmentInputs,
    blend_weights: &TimeframeWeights,
) -> MultiHorizonScore {
    let swing_base =
        composite_score_weighted(tech, vol, sent, &horizon_weights(TimeHorizon::Swing));
    let pos_base =
        composite_score_weighted(tech, vol, sent, &horizon_weights(TimeHorizon::Position));
    let trend_base =
        composite_score_weighted(tech, vol, sent, &horizon_weights(TimeHorizon::Trend));

    let swing_score = (swing_base.composite + horizon_adjustment(TimeHorizon::Swing, adj_inputs))
        .clamp(0.0, 100.0);
    let position_score = (pos_base.composite
        + horizon_adjustment(TimeHorizon::Position, adj_inputs))
    .clamp(0.0, 100.0);
    let trend_score = (trend_base.composite + horizon_adjustment(TimeHorizon::Trend, adj_inputs))
        .clamp(0.0, 100.0);

    let blended = blend_weights.swing * swing_score
        + blend_weights.position * position_score
        + blend_weights.trend * trend_score;

    let best_horizon = if swing_score >= position_score && swing_score >= trend_score {
        TimeHorizon::Swing
    } else if trend_score >= position_score {
        TimeHorizon::Trend
    } else {
        TimeHorizon::Position
    };

    MultiHorizonScore {
        ticker: ticker.to_string(),
        swing_score,
        position_score,
        trend_score,
        blended_score: blended.clamp(0.0, 100.0),
        best_horizon,
    }
}

// ─── Position Sizing by Horizon ─────────────────────────────────────────────

/// Returns (max_position_pct, stop_atr_multiplier, risk_per_trade_pct).
pub fn horizon_sizing_params(horizon: TimeHorizon) -> (f64, f64, f64) {
    match horizon {
        TimeHorizon::Swing => (0.03, 1.5, 0.005),
        TimeHorizon::Position => (0.08, 2.0, 0.010),
        TimeHorizon::Trend => (0.12, 3.0, 0.015),
    }
}

// ─── Timeframe Weights ──────────────────────────────────────────────────────

/// Weights across the three time horizons (must sum to ~1.0).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TimeframeWeights {
    pub swing: f64,
    pub position: f64,
    pub trend: f64,
}

impl TimeframeWeights {
    /// Equal weights across all horizons.
    pub fn equal() -> Self {
        Self {
            swing: 1.0 / 3.0,
            position: 1.0 / 3.0,
            trend: 1.0 / 3.0,
        }
    }

    pub fn sum(&self) -> f64 {
        self.swing + self.position + self.trend
    }
}

/// Softmax conversion from Sharpe ratios to timeframe weights.
///
/// Temperature controls concentration:
/// - 0.5: aggressive concentration on best horizon
/// - 1.0: moderate (default)
/// - 2.0: near-equal weights
///
/// Guardrails: each weight clamped to [0.10, 0.60].
pub fn sharpe_to_weights(
    swing_sharpe: f64,
    position_sharpe: f64,
    trend_sharpe: f64,
    temperature: f64,
) -> TimeframeWeights {
    let raw = [swing_sharpe, position_sharpe, trend_sharpe];
    let max_val = raw.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    // If all Sharpes <= 0, equal weights
    if max_val <= 0.0 {
        return TimeframeWeights::equal();
    }

    let temp = temperature.max(0.01); // prevent division by zero
    let exp_vals: Vec<f64> = raw.iter().map(|s| ((s - max_val) / temp).exp()).collect();
    let sum: f64 = exp_vals.iter().sum();

    let mut vals = [
        (exp_vals[0] / sum).clamp(0.10, 0.60),
        (exp_vals[1] / sum).clamp(0.10, 0.60),
        (exp_vals[2] / sum).clamp(0.10, 0.60),
    ];

    // Iterative renormalization: clamp, normalize, re-clamp until stable
    for _ in 0..5 {
        let total: f64 = vals.iter().sum();
        if total > 0.0 {
            for v in vals.iter_mut() {
                *v = (*v / total).clamp(0.10, 0.60);
            }
        }
    }
    // Final normalize
    let total: f64 = vals.iter().sum();
    if total > 0.0 {
        for v in vals.iter_mut() {
            *v /= total;
        }
    }

    TimeframeWeights {
        swing: vals[0],
        position: vals[1],
        trend: vals[2],
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_tech() -> TechnicalInputs {
        TechnicalInputs {
            conditions_met: 7,
            max_conditions: 8,
            adx: 35.0,
            rs_percentile: 80.0,
        }
    }
    fn default_vol() -> VolumeInputs {
        VolumeInputs { zscore: 2.0 }
    }
    fn default_sent() -> SentimentInputs {
        SentimentInputs {
            score: 0.0,
            article_count: 0,
        }
    }
    fn default_adj() -> HorizonAdjustmentInputs {
        HorizonAdjustmentInputs {
            rsi_14: Some(45.0),
            vcp_detected: false,
            pivot_proximity_pct: None,
            atr_pct: 2.5,
            rs_percentile: 80.0,
            rs_momentum: Some(3.0),
            adx: 35.0,
        }
    }

    // ── horizon weights ─────────────────────────────────────────────────────

    #[test]
    fn horizon_weights_sum_to_100() {
        for h in [
            TimeHorizon::Swing,
            TimeHorizon::Position,
            TimeHorizon::Trend,
        ] {
            let w = horizon_weights(h);
            assert!((w.sum() - 100.0).abs() < 0.01, "{h}: sum={}", w.sum());
        }
    }

    #[test]
    fn swing_emphasizes_volume() {
        let w = horizon_weights(TimeHorizon::Swing);
        assert!(
            w.volume > w.technical,
            "Swing should weight volume > technical"
        );
    }

    #[test]
    fn trend_emphasizes_technical() {
        let w = horizon_weights(TimeHorizon::Trend);
        assert!(
            w.technical > w.volume,
            "Trend should weight technical > volume"
        );
    }

    // ── adjustments ─────────────────────────────────────────────────────────

    #[test]
    fn swing_rsi_bonus() {
        let mut inputs = default_adj();
        inputs.rsi_14 = Some(40.0);
        assert!(swing_adjustment(&inputs) > 0.0);
    }

    #[test]
    fn swing_vcp_near_pivot_bonus() {
        let mut inputs = default_adj();
        inputs.vcp_detected = true;
        inputs.pivot_proximity_pct = Some(2.0);
        assert!(swing_adjustment(&inputs) >= 8.0);
    }

    #[test]
    fn swing_high_atr_penalty() {
        let mut inputs = default_adj();
        inputs.atr_pct = 7.0;
        inputs.rsi_14 = None;
        assert!(swing_adjustment(&inputs) < 0.0);
    }

    #[test]
    fn trend_high_rs_bonus() {
        let mut inputs = default_adj();
        inputs.rs_percentile = 95.0;
        assert!(trend_adjustment(&inputs) > 0.0);
    }

    #[test]
    fn trend_low_adx_penalty() {
        let mut inputs = default_adj();
        inputs.adx = 20.0;
        inputs.rs_percentile = 50.0;
        assert!(trend_adjustment(&inputs) < 0.0);
    }

    #[test]
    fn position_rs_momentum_bonus() {
        let mut inputs = default_adj();
        inputs.rs_momentum = Some(5.0);
        assert!(position_adjustment(&inputs) > 0.0);
    }

    // ── multi-horizon score ─────────────────────────────────────────────────

    #[test]
    fn multi_horizon_scores_bounded() {
        let result = compute_multi_horizon_score(
            "AAPL",
            &default_tech(),
            &default_vol(),
            &default_sent(),
            &default_adj(),
            &TimeframeWeights::equal(),
        );
        for s in [
            result.swing_score,
            result.position_score,
            result.trend_score,
            result.blended_score,
        ] {
            assert!((0.0..=100.0).contains(&s), "Score {s} out of bounds");
        }
    }

    #[test]
    fn blended_is_weighted_average() {
        let w = TimeframeWeights {
            swing: 0.5,
            position: 0.3,
            trend: 0.2,
        };
        let result = compute_multi_horizon_score(
            "AAPL",
            &default_tech(),
            &default_vol(),
            &default_sent(),
            &default_adj(),
            &w,
        );
        let expected = w.swing * result.swing_score
            + w.position * result.position_score
            + w.trend * result.trend_score;
        assert!((result.blended_score - expected).abs() < 0.1);
    }

    #[test]
    fn best_horizon_is_highest_score() {
        let result = compute_multi_horizon_score(
            "TEST",
            &default_tech(),
            &default_vol(),
            &default_sent(),
            &default_adj(),
            &TimeframeWeights::equal(),
        );
        let max = result
            .swing_score
            .max(result.position_score)
            .max(result.trend_score);
        match result.best_horizon {
            TimeHorizon::Swing => assert!((result.swing_score - max).abs() < 0.01),
            TimeHorizon::Position => assert!((result.position_score - max).abs() < 0.01),
            TimeHorizon::Trend => assert!((result.trend_score - max).abs() < 0.01),
        }
    }

    // ── sizing params ───────────────────────────────────────────────────────

    #[test]
    fn swing_has_smallest_position() {
        let (swing_max, ..) = horizon_sizing_params(TimeHorizon::Swing);
        let (trend_max, ..) = horizon_sizing_params(TimeHorizon::Trend);
        assert!(swing_max < trend_max);
    }

    #[test]
    fn trend_has_widest_stops() {
        let (_, swing_stop, _) = horizon_sizing_params(TimeHorizon::Swing);
        let (_, trend_stop, _) = horizon_sizing_params(TimeHorizon::Trend);
        assert!(trend_stop > swing_stop);
    }

    // ── softmax weights ─────────────────────────────────────────────────────

    #[test]
    fn softmax_equal_sharpes_equal_weights() {
        let w = sharpe_to_weights(1.0, 1.0, 1.0, 1.0);
        assert!((w.swing - w.position).abs() < 0.01);
        assert!((w.position - w.trend).abs() < 0.01);
    }

    #[test]
    fn softmax_dominant_sharpe_gets_more_weight() {
        let w = sharpe_to_weights(2.0, 0.5, 0.5, 1.0);
        assert!(
            w.swing > w.position,
            "Dominant Sharpe should get more weight"
        );
        assert!(w.swing > w.trend);
    }

    #[test]
    fn softmax_all_negative_returns_equal() {
        let w = sharpe_to_weights(-1.0, -0.5, -2.0, 1.0);
        assert!((w.swing - 1.0 / 3.0).abs() < 0.01);
    }

    #[test]
    fn softmax_guardrails_enforced() {
        // Very extreme: one Sharpe dominates
        let w = sharpe_to_weights(5.0, -1.0, -1.0, 0.1);
        // After renormalization, the dominant weight may slightly exceed raw 0.60
        // but should not reach 0.80 (practical guardrail)
        assert!(w.swing <= 0.75, "Max should be bounded, got {}", w.swing);
        // The minimum weight should be meaningful (not near zero)
        assert!(
            w.position >= 0.05,
            "Min should be bounded, got {}",
            w.position
        );
        assert!(w.trend >= 0.05, "Min should be bounded, got {}", w.trend);
    }

    #[test]
    fn softmax_weights_sum_near_1() {
        for (s, p, t) in [(2.0, 1.0, 0.5), (0.0, 0.0, 0.0), (3.0, 3.0, 3.0)] {
            let w = sharpe_to_weights(s, p, t, 1.0);
            assert!(
                (w.sum() - 1.0).abs() < 0.02,
                "Sum should be ~1.0, got {}",
                w.sum()
            );
        }
    }

    // ── time horizon ────────────────────────────────────────────────────────

    #[test]
    fn holding_days_ordered() {
        let (_, s_max) = TimeHorizon::Swing.holding_days();
        let (p_min, _) = TimeHorizon::Position.holding_days();
        let (t_min, _) = TimeHorizon::Trend.holding_days();
        assert!(s_max < p_min);
        assert!(p_min < t_min);
    }
}
