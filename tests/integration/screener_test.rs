//! Integration tests for the MinerviniScreener against realistic candle fixtures.

use crate::common::fixtures;
use ghost_alpha::analysis::screener::{MinerviniScreener, Screener};
use ghost_alpha::data::models::MarketRegime;

fn screener() -> MinerviniScreener {
    MinerviniScreener::new()
}

fn default_regime() -> MarketRegime {
    MarketRegime::Bull
}

// ─── Pass / Fail ──────────────────────────────────────────────────────────────

#[test]
fn minervini_pass_candles_produce_high_score() {
    let s = screener();
    let candles = fixtures::minervini_pass_candles();
    let universe_returns = vec![0.05, 0.08, 0.12, 0.20, 0.30];
    let result = s
        .screen("NVDA", &candles, &universe_returns, &default_regime())
        .unwrap();

    assert!(
        result.composite_score >= 50.0,
        "Uptrending stock should score at least 50, got {:.1}",
        result.composite_score
    );
    assert!(result.minervini_pass, "Should pass Minervini template");
    assert!(
        result.conditions_met >= 5,
        "Should meet at least 5 conditions"
    );
}

#[test]
fn minervini_fail_candles_produce_low_score() {
    let s = screener();
    let candles = fixtures::minervini_fail_candles();
    let universe_returns = vec![0.05, 0.08, 0.12, 0.20, 0.30];
    let result = s
        .screen("WEAK", &candles, &universe_returns, &default_regime())
        .unwrap();

    assert!(
        result.composite_score < 50.0,
        "Declining stock should score below 50, got {:.1}",
        result.composite_score
    );
    assert!(!result.minervini_pass, "Should fail Minervini template");
}

// ─── Volume integration ───────────────────────────────────────────────────────

/// Build a 300-bar trending series with a large volume spike on the last bar.
fn spike_candles_300() -> Vec<ghost_alpha::data::models::Candle> {
    let mut candles = fixtures::trending_candles(300, 50.0, 150.0);
    let last = candles.len() - 1;
    candles[last].volume = 10_000_000; // 10x normal
    candles[last].close = candles[last - 1].close * 1.02; // up-close
    candles[last].adj_close = candles[last].close;
    candles[last].open = candles[last - 1].close;
    candles
}

#[test]
fn volume_spike_candles_show_elevated_zscore() {
    let s = screener();
    let candles = spike_candles_300();
    let universe_returns = vec![0.0];
    let result = s
        .screen("SPIKE", &candles, &universe_returns, &default_regime())
        .unwrap();
    assert!(
        result.volume_zscore > 1.0,
        "Volume spike should produce z > 1, got {:.2}",
        result.volume_zscore
    );
}

#[test]
fn uniform_volume_candles_show_near_zero_zscore() {
    let s = screener();
    // Use a flat 300-bar series (all volumes identical)
    let candles = fixtures::trending_candles(300, 100.0, 100.0);
    let universe_returns = vec![0.0];
    let result = s
        .screen("FLAT", &candles, &universe_returns, &default_regime())
        .unwrap();
    assert!(
        result.volume_zscore.abs() < 1.0,
        "Uniform volume should give near-zero z, got {:.2}",
        result.volume_zscore
    );
}

// ─── Regime-aware scoring ─────────────────────────────────────────────────────

#[test]
fn different_regimes_produce_different_scores() {
    let s = screener();
    let candles = fixtures::minervini_pass_candles();
    let universe_returns = vec![0.05, 0.08, 0.12, 0.20, 0.30];

    let bull_result = s
        .screen("NVDA", &candles, &universe_returns, &MarketRegime::Bull)
        .unwrap();
    let bear_result = s
        .screen("NVDA", &candles, &universe_returns, &MarketRegime::Bear)
        .unwrap();
    let sideways_result = s
        .screen("NVDA", &candles, &universe_returns, &MarketRegime::Sideways)
        .unwrap();

    // Same candles under different regimes should yield different composite scores
    // because RegimeWeights change the technical/volume/sentiment balance.
    // At minimum Bull and Bear should differ (55/30/15 vs 70/10/20).
    assert!(
        (bull_result.composite_score - bear_result.composite_score).abs() > 0.01,
        "Bull ({:.2}) and Bear ({:.2}) should produce different scores",
        bull_result.composite_score,
        bear_result.composite_score,
    );

    // Regime field should be populated correctly
    assert_eq!(bull_result.regime, MarketRegime::Bull);
    assert_eq!(bear_result.regime, MarketRegime::Bear);
    assert_eq!(sideways_result.regime, MarketRegime::Sideways);
}

// ─── Phase 2.5 field validation ──────────────────────────────────────────────

#[test]
fn screen_result_phase25_fields_populated() {
    let s = screener();
    let candles = fixtures::minervini_pass_candles();
    let universe_returns = vec![0.05, 0.08, 0.12, 0.20, 0.30];
    let result = s
        .screen("NVDA", &candles, &universe_returns, &default_regime())
        .unwrap();

    // ATR should be positive for any series with price movement
    assert!(
        result.atr_value > 0.0,
        "ATR should be positive, got {:.4}",
        result.atr_value
    );

    // Risk-reward should be non-negative (clamped)
    assert!(
        result.risk_reward >= 0.0,
        "R:R should be >= 0, got {:.2}",
        result.risk_reward
    );

    // Weighted RS should be populated (uptrend → positive)
    assert!(
        result.weighted_rs > 0.0,
        "Weighted RS should be positive for uptrend, got {:.4}",
        result.weighted_rs
    );

    // Regime field matches what was passed in
    assert_eq!(result.regime, MarketRegime::Bull);
}

// ─── Field invariants ─────────────────────────────────────────────────────────

#[test]
fn screen_result_fields_are_in_valid_ranges() {
    let s = screener();
    let candles = fixtures::minervini_pass_candles();
    let result = s
        .screen("NVDA", &candles, &[0.1, 0.2, 0.3], &default_regime())
        .unwrap();

    assert!(
        result.adx >= 0.0 && result.adx <= 100.0,
        "ADX must be 0–100"
    );
    assert!(
        result.rs_percentile >= 0.0 && result.rs_percentile <= 100.0,
        "RS must be 0–100"
    );
    assert!(
        result.composite_score >= 0.0 && result.composite_score <= 100.0,
        "Composite must be 0–100"
    );
    assert!(result.conditions_met <= 8, "Conditions met must be 0–8");
    assert!(result.close_price > 0.0, "Close price must be positive");
}
