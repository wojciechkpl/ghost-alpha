//! Market regime detection — Bull / Sideways / Bear classification.
//!
//! Uses two signals to classify the current market regime:
//! 1. **SPY vs 200-SMA**: Is SPY trading above or below its 200-day SMA?
//! 2. **Market breadth**: What % of the universe has price > 50-day SMA?
//!
//! Classification rules (from PRD F9.1–F9.3):
//! - Bull:     SPY > 200-SMA AND breadth > 60%
//! - Bear:     SPY < 200-SMA AND breadth < 40%
//! - Sideways: Everything else

use crate::analysis::indicators::sma;
use crate::data::models::{Candle, MarketRegime, RegimeState};
use crate::error::AnalysisError;

/// Thresholds for regime classification.
const BREADTH_BULL_THRESHOLD: f64 = 60.0;
const BREADTH_BEAR_THRESHOLD: f64 = 40.0;
const SMA_PERIOD: usize = 200;
const BREADTH_SMA_PERIOD: usize = 50;

/// Detect the current market regime from SPY candles and universe candle data.
///
/// # Arguments
/// - `spy_candles` — At least 200 daily candles for SPY (chronological order).
/// - `universe_candles` — Slice of `(ticker, candles)` for breadth calculation.
///   Each ticker should have at least 50 candles.
///
/// # Returns
/// A `RegimeState` with the computed values and classification.
pub fn detect_regime(
    spy_candles: &[Candle],
    universe_candles: &[(&str, &[Candle])],
) -> Result<RegimeState, AnalysisError> {
    if spy_candles.len() < SMA_PERIOD {
        return Err(AnalysisError::InsufficientData {
            need: SMA_PERIOD,
            got: spy_candles.len(),
        });
    }

    // 1. SPY close vs 200-SMA
    let spy_closes: Vec<f64> = spy_candles.iter().map(|c| c.close).collect();
    let spy_sma200_values = sma(&spy_closes, SMA_PERIOD)?;
    let spy_sma200 = *spy_sma200_values.last().unwrap();
    let spy_close = spy_candles.last().unwrap().close;
    let spy_date = spy_candles.last().unwrap().date;
    let spy_above_sma = spy_close > spy_sma200;

    // 2. Breadth: % of universe with close > 50-SMA
    let breadth_pct = compute_breadth(universe_candles);

    // 3. Classify
    let regime = classify_regime(spy_above_sma, breadth_pct);

    Ok(RegimeState {
        date: spy_date,
        spy_close,
        spy_sma200,
        breadth_pct,
        regime,
    })
}

/// Compute market breadth: % of tickers with close > 50-SMA.
fn compute_breadth(universe_candles: &[(&str, &[Candle])]) -> f64 {
    if universe_candles.is_empty() {
        return 50.0; // Default to neutral if no data
    }

    let mut above_count = 0usize;
    let mut total = 0usize;

    for (_ticker, candles) in universe_candles {
        if candles.len() < BREADTH_SMA_PERIOD {
            continue;
        }

        let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
        if let Ok(sma_values) = sma(&closes, BREADTH_SMA_PERIOD) {
            let current_close = candles.last().unwrap().close;
            let current_sma50 = *sma_values.last().unwrap();
            if current_close > current_sma50 {
                above_count += 1;
            }
            total += 1;
        }
    }

    if total == 0 {
        return 50.0;
    }

    (above_count as f64 / total as f64) * 100.0
}

/// Classify the regime based on SPY position and breadth.
fn classify_regime(spy_above_sma: bool, breadth_pct: f64) -> MarketRegime {
    if spy_above_sma && breadth_pct > BREADTH_BULL_THRESHOLD {
        MarketRegime::Bull
    } else if !spy_above_sma && breadth_pct < BREADTH_BEAR_THRESHOLD {
        MarketRegime::Bear
    } else {
        MarketRegime::Sideways
    }
}

// ─── Regime Transition Hysteresis ────────────────────────────────────────────

/// Regime transition state with hysteresis to prevent whipsaw.
///
/// Requires `days_required` consecutive days in the new regime conditions
/// before officially transitioning.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum RegimeTransition {
    /// Regime is stable — no transition in progress.
    Stable(MarketRegime),
    /// Conditions are deteriorating toward a worse regime.
    Deteriorating {
        from: MarketRegime,
        to: MarketRegime,
        days_held: u32,
        days_required: u32,
    },
    /// Conditions are improving toward a better regime.
    Improving {
        from: MarketRegime,
        to: MarketRegime,
        days_held: u32,
        days_required: u32,
    },
}

impl RegimeTransition {
    /// The effective regime (what the portfolio should act on).
    /// During transitions, this returns the `from` regime until confirmed.
    pub fn effective_regime(&self) -> &MarketRegime {
        match self {
            RegimeTransition::Stable(r) => r,
            RegimeTransition::Deteriorating { from, .. } => from,
            RegimeTransition::Improving { from, .. } => from,
        }
    }

    /// Whether a transition is in progress.
    pub fn is_transitioning(&self) -> bool {
        !matches!(self, RegimeTransition::Stable(_))
    }
}

/// Update regime state with hysteresis.
///
/// Given the raw detected regime and the prior transition state,
/// returns the new transition state. A regime change is only confirmed
/// after `hysteresis_days` consecutive days of the new regime.
pub fn update_regime_with_hysteresis(
    raw_regime: &MarketRegime,
    prior_state: &RegimeTransition,
    hysteresis_days: u32,
) -> RegimeTransition {
    let current_regime = prior_state.effective_regime();

    // No change — cancel any in-progress transition
    if raw_regime == current_regime {
        return RegimeTransition::Stable(current_regime.clone());
    }

    // Check if we're continuing an existing transition to the same target
    let days_held = match prior_state {
        RegimeTransition::Deteriorating { to, days_held, .. }
        | RegimeTransition::Improving { to, days_held, .. }
            if to == raw_regime =>
        {
            days_held + 1
        }
        _ => 1, // new transition started
    };

    // Check if transition is confirmed
    if days_held >= hysteresis_days {
        return RegimeTransition::Stable(raw_regime.clone());
    }

    // Determine direction
    let is_deteriorating = regime_rank(raw_regime) < regime_rank(current_regime);

    if is_deteriorating {
        RegimeTransition::Deteriorating {
            from: current_regime.clone(),
            to: raw_regime.clone(),
            days_held,
            days_required: hysteresis_days,
        }
    } else {
        RegimeTransition::Improving {
            from: current_regime.clone(),
            to: raw_regime.clone(),
            days_held,
            days_required: hysteresis_days,
        }
    }
}

/// Numeric rank for regime comparison (higher = better).
fn regime_rank(regime: &MarketRegime) -> u8 {
    match regime {
        MarketRegime::Bear => 0,
        MarketRegime::Sideways => 1,
        MarketRegime::Bull => 2,
    }
}

/// Determine if alerts should be suppressed based on regime.
/// In Bear regime, only Critical alerts pass through.
pub fn should_suppress_in_bear(regime: &MarketRegime, is_critical: bool) -> bool {
    matches!(regime, MarketRegime::Bear) && !is_critical
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate candles with a steady uptrend above SMA-200.
    fn bull_spy_candles(n: usize) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let base = 400.0 + (i as f64 * 0.5); // Steady uptrend
                Candle::new(
                    &format!("2023-{:02}-{:02}", (i / 28 % 12) + 1, (i % 28) + 1),
                    base - 1.0,
                    base + 2.0,
                    base - 2.0,
                    base,
                    10_000_000,
                )
            })
            .collect()
    }

    /// Generate candles with a steady downtrend below SMA-200.
    fn bear_spy_candles(n: usize) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let base = 500.0 - (i as f64 * 0.5); // Steady downtrend
                Candle::new(
                    &format!("2023-{:02}-{:02}", (i / 28 % 12) + 1, (i % 28) + 1),
                    base + 1.0,
                    base + 2.0,
                    base - 2.0,
                    base,
                    10_000_000,
                )
            })
            .collect()
    }

    /// Generate universe candles where a given % are above their 50-SMA.
    fn make_universe(
        above_pct: f64,
        n_stocks: usize,
        candle_count: usize,
    ) -> Vec<(String, Vec<Candle>)> {
        let above_count = ((above_pct / 100.0) * n_stocks as f64).round() as usize;

        (0..n_stocks)
            .map(|i| {
                let ticker = format!("STK{}", i);
                let candles: Vec<Candle> = (0..candle_count)
                    .map(|j| {
                        let base = if i < above_count {
                            // Trending up: close will be above 50-SMA
                            100.0 + (j as f64 * 1.0)
                        } else {
                            // Trending down: close will be below 50-SMA
                            200.0 - (j as f64 * 1.0)
                        };
                        Candle::new(
                            &format!("2023-{:02}-{:02}", (j / 28 % 12) + 1, (j % 28) + 1),
                            base - 1.0,
                            base + 1.0,
                            base - 2.0,
                            base,
                            1_000_000,
                        )
                    })
                    .collect();
                (ticker, candles)
            })
            .collect()
    }

    // ── Classification unit tests ───────────────────────────────────────────

    #[test]
    fn classify_bull_when_spy_above_and_high_breadth() {
        assert_eq!(classify_regime(true, 75.0), MarketRegime::Bull);
    }

    #[test]
    fn classify_bear_when_spy_below_and_low_breadth() {
        assert_eq!(classify_regime(false, 30.0), MarketRegime::Bear);
    }

    #[test]
    fn classify_sideways_when_mixed_spy_above_low_breadth() {
        assert_eq!(classify_regime(true, 35.0), MarketRegime::Sideways);
    }

    #[test]
    fn classify_sideways_when_mixed_spy_below_high_breadth() {
        assert_eq!(classify_regime(false, 65.0), MarketRegime::Sideways);
    }

    #[test]
    fn classify_sideways_at_boundary_values() {
        // At exactly 60% and above → NOT bull (needs >60)
        assert_eq!(classify_regime(true, 60.0), MarketRegime::Sideways);
        // At exactly 40% and below → NOT bear (needs <40)
        assert_eq!(classify_regime(false, 40.0), MarketRegime::Sideways);
    }

    // ── Breadth tests ───────────────────────────────────────────────────────

    #[test]
    fn breadth_empty_universe_returns_50() {
        assert_eq!(compute_breadth(&[]), 50.0);
    }

    #[test]
    fn breadth_all_above_returns_100() {
        let universe = make_universe(100.0, 10, 60);
        let refs: Vec<(&str, &[Candle])> = universe
            .iter()
            .map(|(t, c)| (t.as_str(), c.as_slice()))
            .collect();
        let breadth = compute_breadth(&refs);
        assert!(
            breadth > 90.0,
            "All-above universe should have breadth near 100, got {}",
            breadth
        );
    }

    #[test]
    fn breadth_all_below_returns_near_0() {
        let universe = make_universe(0.0, 10, 60);
        let refs: Vec<(&str, &[Candle])> = universe
            .iter()
            .map(|(t, c)| (t.as_str(), c.as_slice()))
            .collect();
        let breadth = compute_breadth(&refs);
        assert!(
            breadth < 10.0,
            "All-below universe should have breadth near 0, got {}",
            breadth
        );
    }

    #[test]
    fn breadth_half_above_returns_near_50() {
        let universe = make_universe(50.0, 20, 60);
        let refs: Vec<(&str, &[Candle])> = universe
            .iter()
            .map(|(t, c)| (t.as_str(), c.as_slice()))
            .collect();
        let breadth = compute_breadth(&refs);
        assert!(
            (40.0..60.0).contains(&breadth),
            "Half-above universe should have breadth near 50, got {}",
            breadth
        );
    }

    #[test]
    fn breadth_skips_tickers_with_insufficient_data() {
        let short_candles: Vec<Candle> = (0..10) // Less than 50
            .map(|i| {
                Candle::new(
                    &format!("2023-01-{:02}", i + 1),
                    100.0,
                    101.0,
                    99.0,
                    100.0,
                    1_000_000,
                )
            })
            .collect();
        let universe = vec![("SHORT", short_candles.as_slice())];
        let breadth = compute_breadth(&universe);
        assert_eq!(
            breadth, 50.0,
            "Should default to 50 when no ticker has enough data"
        );
    }

    // ── Detect regime integration tests ─────────────────────────────────────

    #[test]
    fn detect_regime_insufficient_spy_data() {
        let candles = bull_spy_candles(100); // Need 200
        let result = detect_regime(&candles, &[]);
        assert!(result.is_err());
        match result.unwrap_err() {
            AnalysisError::InsufficientData { need, got } => {
                assert_eq!(need, 200);
                assert_eq!(got, 100);
            }
            other => panic!("Expected InsufficientData, got: {:?}", other),
        }
    }

    #[test]
    fn detect_regime_bull_market() {
        let spy = bull_spy_candles(250);
        let universe = make_universe(80.0, 10, 60);
        let refs: Vec<(&str, &[Candle])> = universe
            .iter()
            .map(|(t, c)| (t.as_str(), c.as_slice()))
            .collect();

        let state = detect_regime(&spy, &refs).unwrap();
        assert_eq!(state.regime, MarketRegime::Bull);
        assert!(
            state.spy_close > state.spy_sma200,
            "SPY should be above 200-SMA in uptrend"
        );
    }

    #[test]
    fn detect_regime_bear_market() {
        let spy = bear_spy_candles(250);
        let universe = make_universe(20.0, 10, 60);
        let refs: Vec<(&str, &[Candle])> = universe
            .iter()
            .map(|(t, c)| (t.as_str(), c.as_slice()))
            .collect();

        let state = detect_regime(&spy, &refs).unwrap();
        assert_eq!(state.regime, MarketRegime::Bear);
        assert!(
            state.spy_close < state.spy_sma200,
            "SPY should be below 200-SMA in downtrend"
        );
    }

    #[test]
    fn detect_regime_returns_correct_date() {
        let spy = bull_spy_candles(250);
        let expected_date = spy.last().unwrap().date;
        let state = detect_regime(&spy, &[]).unwrap();
        assert_eq!(state.date, expected_date);
    }

    // ── Suppression tests ───────────────────────────────────────────────────

    #[test]
    fn suppress_non_critical_in_bear() {
        assert!(should_suppress_in_bear(&MarketRegime::Bear, false));
    }

    #[test]
    fn allow_critical_in_bear() {
        assert!(!should_suppress_in_bear(&MarketRegime::Bear, true));
    }

    #[test]
    fn no_suppression_in_bull() {
        assert!(!should_suppress_in_bear(&MarketRegime::Bull, false));
        assert!(!should_suppress_in_bear(&MarketRegime::Bull, true));
    }

    #[test]
    fn no_suppression_in_sideways() {
        assert!(!should_suppress_in_bear(&MarketRegime::Sideways, false));
    }

    // ── Regime hysteresis tests ─────────────────────────────────────────────

    #[test]
    fn hysteresis_stable_when_no_change() {
        let state = RegimeTransition::Stable(MarketRegime::Bull);
        let result = update_regime_with_hysteresis(&MarketRegime::Bull, &state, 3);
        assert_eq!(result, RegimeTransition::Stable(MarketRegime::Bull));
    }

    #[test]
    fn hysteresis_starts_deteriorating_on_first_change() {
        let state = RegimeTransition::Stable(MarketRegime::Bull);
        let result = update_regime_with_hysteresis(&MarketRegime::Sideways, &state, 3);
        assert!(matches!(
            result,
            RegimeTransition::Deteriorating {
                from: MarketRegime::Bull,
                to: MarketRegime::Sideways,
                days_held: 1,
                days_required: 3,
            }
        ));
    }

    #[test]
    fn hysteresis_counts_consecutive_days() {
        let state = RegimeTransition::Deteriorating {
            from: MarketRegime::Bull,
            to: MarketRegime::Sideways,
            days_held: 1,
            days_required: 3,
        };
        let result = update_regime_with_hysteresis(&MarketRegime::Sideways, &state, 3);
        assert!(matches!(
            result,
            RegimeTransition::Deteriorating { days_held: 2, .. }
        ));
    }

    #[test]
    fn hysteresis_confirms_after_n_days() {
        let state = RegimeTransition::Deteriorating {
            from: MarketRegime::Bull,
            to: MarketRegime::Sideways,
            days_held: 2,
            days_required: 3,
        };
        let result = update_regime_with_hysteresis(&MarketRegime::Sideways, &state, 3);
        assert_eq!(result, RegimeTransition::Stable(MarketRegime::Sideways));
    }

    #[test]
    fn hysteresis_cancels_on_reversion() {
        let state = RegimeTransition::Deteriorating {
            from: MarketRegime::Bull,
            to: MarketRegime::Sideways,
            days_held: 2,
            days_required: 3,
        };
        // Breadth recovers — cancel transition
        let result = update_regime_with_hysteresis(&MarketRegime::Bull, &state, 3);
        assert_eq!(result, RegimeTransition::Stable(MarketRegime::Bull));
    }

    #[test]
    fn hysteresis_improving_direction() {
        let state = RegimeTransition::Stable(MarketRegime::Bear);
        let result = update_regime_with_hysteresis(&MarketRegime::Sideways, &state, 3);
        assert!(matches!(
            result,
            RegimeTransition::Improving {
                from: MarketRegime::Bear,
                to: MarketRegime::Sideways,
                days_held: 1,
                ..
            }
        ));
    }

    #[test]
    fn effective_regime_during_transition_is_from() {
        let state = RegimeTransition::Deteriorating {
            from: MarketRegime::Bull,
            to: MarketRegime::Sideways,
            days_held: 1,
            days_required: 3,
        };
        assert_eq!(state.effective_regime(), &MarketRegime::Bull);
    }

    #[test]
    fn is_transitioning_returns_true_during_transition() {
        let stable = RegimeTransition::Stable(MarketRegime::Bull);
        assert!(!stable.is_transitioning());

        let transitioning = RegimeTransition::Deteriorating {
            from: MarketRegime::Bull,
            to: MarketRegime::Sideways,
            days_held: 1,
            days_required: 3,
        };
        assert!(transitioning.is_transitioning());
    }

    #[test]
    fn hysteresis_1_day_confirms_immediately() {
        let state = RegimeTransition::Stable(MarketRegime::Bull);
        let result = update_regime_with_hysteresis(&MarketRegime::Sideways, &state, 1);
        // 1 day required, 1 day held → confirmed
        assert_eq!(result, RegimeTransition::Stable(MarketRegime::Sideways));
    }
}
