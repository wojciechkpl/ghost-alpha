//! Adaptive parameter tuning — rolling calibration within guardrails.
//!
//! Phase 6 (F21): Six screening parameters adapt via coordinate descent
//! on a walk-forward IS/OOS split. Guardrails prevent overfitting:
//! max step per cycle, signal count penalty, regime-change reset.
//!
//! Design philosophy: **Adapt slowly. Constrain tightly. Audit everything.**

use crate::data::models::MarketRegime;
use chrono::NaiveDate;

// ─── Adaptive Parameters ────────────────────────────────────────────────────

/// Parameters that can be adapted within guardrails.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AdaptiveParams {
    /// RS percentile threshold. Default: 70.0. Range: [60.0, 85.0].
    pub rs_threshold: f64,
    /// ADX threshold for "strong trend". Default: 30.0. Range: [22.0, 40.0].
    pub adx_strong_threshold: f64,
    /// ADX threshold for "trending". Default: 20.0. Range: [15.0, 28.0].
    pub adx_trending_threshold: f64,
    /// Volume surge ratio for breakout. Default: 1.5. Range: [1.2, 2.5].
    pub volume_surge_ratio: f64,
    /// Max consolidation depth. Default: 0.15. Range: [0.08, 0.25].
    pub consolidation_max_depth: f64,
    /// RSI oversold threshold. Default: 30.0. Range: [20.0, 40.0].
    pub rsi_oversold_threshold: f64,
    /// Metadata
    pub calibrated_at: NaiveDate,
    pub regime_at_calibration: MarketRegime,
}

impl AdaptiveParams {
    /// Default (non-adaptive) parameter set.
    pub fn defaults(as_of: NaiveDate, regime: MarketRegime) -> Self {
        Self {
            rs_threshold: 70.0,
            adx_strong_threshold: 30.0,
            adx_trending_threshold: 20.0,
            volume_surge_ratio: 1.5,
            consolidation_max_depth: 0.15,
            rsi_oversold_threshold: 30.0,
            calibrated_at: as_of,
            regime_at_calibration: regime,
        }
    }

    /// Get parameter value by index (for coordinate descent).
    /// Returns None if index is out of bounds.
    pub fn get(&self, idx: usize) -> f64 {
        match idx {
            0 => self.rs_threshold,
            1 => self.adx_strong_threshold,
            2 => self.adx_trending_threshold,
            3 => self.volume_surge_ratio,
            4 => self.consolidation_max_depth,
            5 => self.rsi_oversold_threshold,
            _ => self.rs_threshold, // fallback to first param for safety
        }
    }

    /// Set parameter value by index. Ignores out-of-bounds indices.
    pub fn set(&mut self, idx: usize, value: f64) {
        match idx {
            0 => self.rs_threshold = value,
            1 => self.adx_strong_threshold = value,
            2 => self.adx_trending_threshold = value,
            3 => self.volume_surge_ratio = value,
            4 => self.consolidation_max_depth = value,
            5 => self.rsi_oversold_threshold = value,
            _ => {} // ignore out-of-bounds
        }
    }

    /// Number of tunable parameters.
    pub const PARAM_COUNT: usize = 6;

    /// Whether calibration is stale (> 30 days old).
    pub fn is_stale(&self, now: NaiveDate) -> bool {
        (now - self.calibrated_at).num_days() > 30
    }
}

// ─── Guardrails ─────────────────────────────────────────────────────────────

/// Constraint for a single parameter.
#[derive(Debug, Clone)]
pub struct ParamGuardrail {
    pub name: &'static str,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// Maximum change per calibration cycle.
    pub max_step: f64,
}

pub const GUARDRAILS: [ParamGuardrail; 6] = [
    ParamGuardrail {
        name: "rs_threshold",
        min: 60.0,
        max: 85.0,
        default: 70.0,
        max_step: 5.0,
    },
    ParamGuardrail {
        name: "adx_strong_threshold",
        min: 22.0,
        max: 40.0,
        default: 30.0,
        max_step: 3.0,
    },
    ParamGuardrail {
        name: "adx_trending_threshold",
        min: 15.0,
        max: 28.0,
        default: 20.0,
        max_step: 3.0,
    },
    ParamGuardrail {
        name: "volume_surge_ratio",
        min: 1.2,
        max: 2.5,
        default: 1.5,
        max_step: 0.2,
    },
    ParamGuardrail {
        name: "consolidation_max_depth",
        min: 0.08,
        max: 0.25,
        default: 0.15,
        max_step: 0.03,
    },
    ParamGuardrail {
        name: "rsi_oversold_threshold",
        min: 20.0,
        max: 40.0,
        default: 30.0,
        max_step: 5.0,
    },
];

/// Generate 5 evenly-spaced test values within guardrail range.
pub fn grid_values(guardrail: &ParamGuardrail) -> [f64; 5] {
    let step = (guardrail.max - guardrail.min) / 4.0;
    [
        guardrail.min,
        guardrail.min + step,
        guardrail.min + 2.0 * step,
        guardrail.min + 3.0 * step,
        guardrail.max,
    ]
}

/// Enforce max_step constraint: new value cannot differ from current by more than max_step.
pub fn apply_max_step(current: f64, candidate: f64, guardrail: &ParamGuardrail) -> f64 {
    let clamped = candidate.clamp(guardrail.min, guardrail.max);
    let delta = clamped - current;
    let bounded_delta = delta.clamp(-guardrail.max_step, guardrail.max_step);
    (current + bounded_delta).clamp(guardrail.min, guardrail.max)
}

// ─── Calibration ────────────────────────────────────────────────────────────

/// A single calibration signal with hit/miss outcome.
#[derive(Debug, Clone)]
pub struct CalibrationSignal {
    pub date: NaiveDate,
    /// Forward 60-day return (positive = hit, negative = miss)
    pub forward_return_60d: f64,
    /// Which parameter values produced this signal
    pub rs_at_signal: f64,
    pub adx_at_signal: f64,
    pub volume_zscore_at_signal: f64,
}

/// Score a parameter set against out-of-sample signals.
///
/// Score = hit_rate × min(1.0, signal_count / min_signals)
/// The signal count penalty prevents overfitting to rare events.
pub fn score_params(signals: &[CalibrationSignal], min_signals: usize) -> f64 {
    if signals.is_empty() {
        return 0.0;
    }
    let hits = signals
        .iter()
        .filter(|s| s.forward_return_60d > 0.0)
        .count();
    let hit_rate = hits as f64 / signals.len() as f64;
    let count_factor = (signals.len() as f64 / min_signals as f64).min(1.0);
    hit_rate * count_factor
}

/// Result of a calibration run.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CalibrationResult {
    pub params: AdaptiveParams,
    pub score: f64,
    pub signal_count: usize,
    pub hit_rate: f64,
    pub params_changed: Vec<String>,
}

/// Calibrate parameters using coordinate descent on a 5-point grid.
///
/// Algorithm:
/// 1. Start from current params
/// 2. For each parameter (one at a time):
///    - Test 5 grid values within guardrail range
///    - Score each using `score_fn` (typically `score_params` on OOS data)
///    - Select best, enforce max_step from current
/// 3. Repeat for `passes` iterations (default: 3)
/// 4. Return calibrated params with audit trail
///
/// This is deterministic: no randomness, fixed grid points, reproducible.
pub fn calibrate<F>(
    current: &AdaptiveParams,
    score_fn: F,
    regime: MarketRegime,
    as_of: NaiveDate,
    passes: usize,
) -> CalibrationResult
where
    F: Fn(&AdaptiveParams, usize) -> (f64, usize, f64), // returns (score, signal_count, hit_rate)
{
    let mut best = current.clone();
    let mut best_signals = 0;
    let mut best_hit_rate = 0.0;
    let mut changed = Vec::new();

    // Evaluate current params first
    let (current_score, _, _) = score_fn(&best, 0);
    let mut best_score = current_score;

    for _pass in 0..passes {
        for (param_idx, guardrail) in GUARDRAILS.iter().enumerate() {
            let grid = grid_values(guardrail);
            let current_val = best.get(param_idx);

            let mut best_val = current_val;
            let mut best_val_score = best_score;

            for &candidate in &grid {
                // Apply max_step constraint
                let stepped = apply_max_step(current.get(param_idx), candidate, guardrail);

                let mut trial = best.clone();
                trial.set(param_idx, stepped);

                let (score, signals, hit_rate) = score_fn(&trial, param_idx);
                if score > best_val_score {
                    best_val = stepped;
                    best_val_score = score;
                    best_signals = signals;
                    best_hit_rate = hit_rate;
                }
            }

            if (best_val - current_val).abs() > 1e-10 {
                best.set(param_idx, best_val);
                best_score = best_val_score;
                if !changed.contains(&guardrail.name.to_string()) {
                    changed.push(guardrail.name.to_string());
                }
            }
        }
    }

    best.calibrated_at = as_of;
    best.regime_at_calibration = regime;

    CalibrationResult {
        params: best,
        score: best_score,
        signal_count: best_signals,
        hit_rate: best_hit_rate,
        params_changed: changed,
    }
}

/// Reset params to defaults when regime changes.
pub fn reset_for_regime_change(
    current: &AdaptiveParams,
    new_regime: MarketRegime,
    as_of: NaiveDate,
) -> AdaptiveParams {
    if current.regime_at_calibration == new_regime {
        return current.clone();
    }
    AdaptiveParams::defaults(as_of, new_regime)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, 25).unwrap()
    }

    // ── defaults ────────────────────────────────────────────────────────────

    #[test]
    fn defaults_within_guardrails() {
        let params = AdaptiveParams::defaults(today(), MarketRegime::Bull);
        for (idx, g) in GUARDRAILS.iter().enumerate() {
            let val = params.get(idx);
            assert!(
                val >= g.min && val <= g.max,
                "{}: default {} outside [{}, {}]",
                g.name,
                val,
                g.min,
                g.max
            );
        }
    }

    #[test]
    fn defaults_match_documented_values() {
        let p = AdaptiveParams::defaults(today(), MarketRegime::Bull);
        assert!((p.rs_threshold - 70.0).abs() < 0.01);
        assert!((p.adx_strong_threshold - 30.0).abs() < 0.01);
        assert!((p.rsi_oversold_threshold - 30.0).abs() < 0.01);
    }

    // ── grid values ─────────────────────────────────────────────────────────

    #[test]
    fn grid_covers_full_range() {
        for g in &GUARDRAILS {
            let grid = grid_values(g);
            assert!((grid[0] - g.min).abs() < 1e-10, "{}: first != min", g.name);
            assert!((grid[4] - g.max).abs() < 1e-10, "{}: last != max", g.name);
            // Monotonically increasing
            for i in 0..4 {
                assert!(grid[i + 1] > grid[i], "{}: grid not increasing", g.name);
            }
        }
    }

    #[test]
    fn grid_has_5_values() {
        for g in &GUARDRAILS {
            assert_eq!(grid_values(g).len(), 5);
        }
    }

    // ── max step ────────────────────────────────────────────────────────────

    #[test]
    fn max_step_enforced() {
        let g = &GUARDRAILS[0]; // rs_threshold: max_step = 5.0
                                // Current = 70.0, candidate = 85.0 (too far)
        let result = apply_max_step(70.0, 85.0, g);
        assert!(
            (result - 75.0).abs() < 0.01,
            "Should cap at 70+5=75, got {result}"
        );
    }

    #[test]
    fn max_step_allows_within_range() {
        let g = &GUARDRAILS[0]; // max_step = 5.0
        let result = apply_max_step(70.0, 73.0, g);
        assert!(
            (result - 73.0).abs() < 0.01,
            "Within step should pass through, got {result}"
        );
    }

    #[test]
    fn max_step_respects_guardrail_bounds() {
        let g = &GUARDRAILS[0]; // min=60, max=85
                                // Current near max, candidate above max
        let result = apply_max_step(83.0, 100.0, g);
        assert!(result <= 85.0, "Should not exceed max, got {result}");
    }

    #[test]
    fn max_step_negative_direction() {
        let g = &GUARDRAILS[0]; // max_step = 5.0
        let result = apply_max_step(70.0, 60.0, g);
        assert!(
            (result - 65.0).abs() < 0.01,
            "Should cap at 70-5=65, got {result}"
        );
    }

    // ── score_params ────────────────────────────────────────────────────────

    #[test]
    fn score_empty_signals_is_zero() {
        assert_eq!(score_params(&[], 20), 0.0);
    }

    #[test]
    fn score_all_hits_above_min_signals() {
        let signals: Vec<CalibrationSignal> = (0..25)
            .map(|_| CalibrationSignal {
                date: today(),
                forward_return_60d: 0.05,
                rs_at_signal: 80.0,
                adx_at_signal: 35.0,
                volume_zscore_at_signal: 2.0,
            })
            .collect();
        let score = score_params(&signals, 20);
        assert!(
            (score - 1.0).abs() < 0.01,
            "All hits, enough signals = 1.0, got {score}"
        );
    }

    #[test]
    fn score_penalizes_few_signals() {
        let signals: Vec<CalibrationSignal> = (0..10)
            .map(|_| CalibrationSignal {
                date: today(),
                forward_return_60d: 0.05,
                rs_at_signal: 80.0,
                adx_at_signal: 35.0,
                volume_zscore_at_signal: 2.0,
            })
            .collect();
        let score = score_params(&signals, 20);
        // hit_rate=1.0, count_factor=10/20=0.5 → score=0.5
        assert!(
            (score - 0.5).abs() < 0.01,
            "Few signals should penalize, got {score}"
        );
    }

    #[test]
    fn score_50_50_hit_rate() {
        let mut signals = Vec::new();
        for i in 0..20 {
            signals.push(CalibrationSignal {
                date: today(),
                forward_return_60d: if i % 2 == 0 { 0.05 } else { -0.03 },
                rs_at_signal: 75.0,
                adx_at_signal: 30.0,
                volume_zscore_at_signal: 1.5,
            });
        }
        let score = score_params(&signals, 20);
        assert!(
            (score - 0.5).abs() < 0.01,
            "50% hit rate = 0.5, got {score}"
        );
    }

    // ── calibrate ───────────────────────────────────────────────────────────

    #[test]
    fn calibrate_improves_or_stays_same() {
        let current = AdaptiveParams::defaults(today(), MarketRegime::Bull);

        // Score function: higher rs_threshold gets better score (simulated)
        let score_fn = |params: &AdaptiveParams, _idx: usize| -> (f64, usize, f64) {
            let score = params.rs_threshold / 100.0; // higher = better (simplified)
            (score, 30, score)
        };

        let result = calibrate(&current, score_fn, MarketRegime::Bull, today(), 3);
        assert!(
            result.score >= current.rs_threshold / 100.0,
            "Calibration should improve or maintain score"
        );
    }

    #[test]
    fn calibrate_respects_max_step() {
        let current = AdaptiveParams::defaults(today(), MarketRegime::Bull);
        let original_rs = current.rs_threshold; // 70.0

        // Score function strongly prefers max rs_threshold
        let score_fn = |params: &AdaptiveParams, _idx: usize| -> (f64, usize, f64) {
            (params.rs_threshold / 100.0, 30, params.rs_threshold / 100.0)
        };

        let result = calibrate(&current, score_fn, MarketRegime::Bull, today(), 1);
        let delta = (result.params.rs_threshold - original_rs).abs();
        assert!(
            delta <= GUARDRAILS[0].max_step + 0.01,
            "RS delta {delta} exceeds max_step {}",
            GUARDRAILS[0].max_step
        );
    }

    #[test]
    fn calibrate_returns_changed_params() {
        let current = AdaptiveParams::defaults(today(), MarketRegime::Bull);

        let score_fn = |params: &AdaptiveParams, _idx: usize| -> (f64, usize, f64) {
            // Prefer higher rs and lower adx
            let score = params.rs_threshold / 100.0 + (50.0 - params.adx_strong_threshold) / 100.0;
            (score, 30, 0.6)
        };

        let result = calibrate(&current, score_fn, MarketRegime::Bull, today(), 3);
        // At least rs_threshold should have changed
        assert!(
            !result.params_changed.is_empty(),
            "Some params should have changed"
        );
    }

    #[test]
    fn calibrate_preserves_regime_and_date() {
        let current = AdaptiveParams::defaults(today(), MarketRegime::Sideways);
        let score_fn = |_: &AdaptiveParams, _: usize| -> (f64, usize, f64) { (0.5, 20, 0.5) };
        let result = calibrate(&current, score_fn, MarketRegime::Bear, today(), 1);
        assert_eq!(result.params.regime_at_calibration, MarketRegime::Bear);
        assert_eq!(result.params.calibrated_at, today());
    }

    // ── regime change reset ─────────────────────────────────────────────────

    #[test]
    fn regime_change_resets_to_defaults() {
        let mut params = AdaptiveParams::defaults(today(), MarketRegime::Bull);
        params.rs_threshold = 80.0; // modified from default

        let reset = reset_for_regime_change(&params, MarketRegime::Bear, today());
        assert!(
            (reset.rs_threshold - 70.0).abs() < 0.01,
            "Should reset to default"
        );
        assert_eq!(reset.regime_at_calibration, MarketRegime::Bear);
    }

    #[test]
    fn same_regime_no_reset() {
        let mut params = AdaptiveParams::defaults(today(), MarketRegime::Bull);
        params.rs_threshold = 80.0;

        let kept = reset_for_regime_change(&params, MarketRegime::Bull, today());
        assert!(
            (kept.rs_threshold - 80.0).abs() < 0.01,
            "Same regime should not reset"
        );
    }

    // ── staleness ───────────────────────────────────────────────────────────

    #[test]
    fn staleness_within_30_days() {
        let params = AdaptiveParams::defaults(today(), MarketRegime::Bull);
        let check = today() + chrono::Duration::days(25);
        assert!(!params.is_stale(check));
    }

    #[test]
    fn staleness_after_30_days() {
        let params = AdaptiveParams::defaults(today(), MarketRegime::Bull);
        let check = today() + chrono::Duration::days(35);
        assert!(params.is_stale(check));
    }
}
