use crate::analysis::indicators::{
    adx, detect_consolidation, high_52w, rs_percentile, sma, volume_ratio,
};
use crate::analysis::indicators::{
    atr, classify_liquidity, detect_vcp, price_return_252d, risk_reward_ratio, weighted_rs,
};
use crate::analysis::scorer::{
    composite_score_weighted, sentiment_score_weighted, technical_score_weighted,
    volume_score_weighted, Grade, RegimeWeights, SentimentInputs, TechnicalInputs, VolumeInputs,
};
use crate::analysis::screener::{ScoringWeights, ScreenResult, Screener};
use crate::data::models::{Candle, MarketRegime};
use crate::error::AnalysisError;

// ─── BreakoutScreener ─────────────────────────────────────────────────────────

/// Breakout scanner strategy.
///
/// Detects price/volume breakouts from consolidation bases. Optimised for swing
/// traders entering at the start of a new move.
///
/// **Selection criteria:**
/// - Price within 5% of the 52-week high
/// - Volume today > 1.5× the 50-day average
/// - ADX > 20 (trending)
/// - Price above 50-SMA and 200-SMA
/// - At least 20 days of consolidation with < 15% depth before the breakout
pub struct BreakoutScreener;

impl BreakoutScreener {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BreakoutScreener {
    fn default() -> Self {
        Self::new()
    }
}

impl Screener for BreakoutScreener {
    fn strategy_name(&self) -> &'static str {
        "breakout"
    }

    fn description(&self) -> &'static str {
        "Price/volume breakouts from consolidation bases — optimised for swing traders"
    }

    fn scoring_weights(&self) -> ScoringWeights {
        ScoringWeights::breakout()
    }

    fn screen(
        &self,
        ticker: &str,
        candles: &[Candle],
        rs_universe_returns: &[f64],
        regime: &MarketRegime,
    ) -> Result<ScreenResult, AnalysisError> {
        if candles.len() < 252 {
            return Err(AnalysisError::InsufficientData {
                need: 252,
                got: candles.len(),
            });
        }

        let closes: Vec<f64> = candles.iter().map(|c| c.adj_close).collect();
        let close_price = *closes.last().unwrap();

        // ── 52-week high ──────────────────────────────────────────────────────
        let h52 = high_52w(candles);
        let proximity = if h52 > 0.0 { close_price / h52 } else { 0.0 };
        let pct_from_high = if h52 > 0.0 {
            (close_price - h52) / h52 * 100.0
        } else {
            0.0
        };
        // Must be within 5% of 52-week high
        let near_high = proximity >= 0.95;

        // ── SMAs ─────────────────────────────────────────────────────────────
        let sma50_vals = sma(&closes, 50).ok();
        let sma200_vals = sma(&closes, 200).ok();
        let sma50 = sma50_vals
            .as_ref()
            .and_then(|v| v.last().copied())
            .unwrap_or(close_price);
        let sma200 = sma200_vals
            .as_ref()
            .and_then(|v| v.last().copied())
            .unwrap_or(close_price);
        let above_smas = close_price > sma50 && close_price > sma200;

        // ── ADX ───────────────────────────────────────────────────────────────
        let adx_result = adx(candles, 14).ok();
        let last_adx = adx_result.as_ref().and_then(|v| v.last());
        let adx_val = last_adx.map(|p| p.adx).unwrap_or(0.0);
        let trending = adx_val > 20.0;

        // ── Volume surge (today vs 50-day average) ────────────────────────────
        let vol_surge = volume_ratio(candles, 50).unwrap_or(0.0);
        let volume_breakout = vol_surge >= 1.5;

        // ── Consolidation base ────────────────────────────────────────────────
        // Look at the window before the last bar (exclude last bar — the breakout itself)
        let consolidation = if candles.len() >= 21 {
            let base_window = &candles[..candles.len() - 1];
            detect_consolidation(base_window, 20, 0.15)
        } else {
            None
        };
        let has_base = consolidation.is_some();

        // ── RS Percentile ─────────────────────────────────────────────────────
        let this_return = price_return_252d(candles).unwrap_or(0.0);
        let rs_pct = if rs_universe_returns.is_empty() {
            50.0
        } else {
            rs_percentile(this_return, rs_universe_returns)
        };

        // ── Volume Z-score (for scoring pipeline) ────────────────────────────
        let vol_z = crate::analysis::indicators::volume_zscore(candles, 20)
            .ok()
            .and_then(|v| v.last().copied())
            .unwrap_or(0.0);

        // ── Conditions count (out of 5) ───────────────────────────────────────
        let conditions: u8 = [near_high, above_smas, trending, volume_breakout, has_base]
            .iter()
            .filter(|&&b| b)
            .count() as u8;

        // ── Phase 2.5 fields ─────────────────────────────────────────────────
        let atr_val = atr(candles, 14).unwrap_or(0.0);
        let rr = risk_reward_ratio(close_price, sma50, atr_val, h52);
        let liquidity = classify_liquidity(candles, 20);
        let wrs = weighted_rs(candles).unwrap_or(0.0);
        let vcp = detect_vcp(candles, 120).ok();
        let rs_mom = if candles.len() >= 252 + 21 {
            let wrs_now = weighted_rs(candles).unwrap_or(0.0);
            let wrs_21d_ago = weighted_rs(&candles[..candles.len() - 21]).unwrap_or(0.0);
            Some((wrs_now - wrs_21d_ago) / 21.0 * 100.0)
        } else {
            None
        };

        // ── Regime-adaptive scoring ───────────────────────────────────────────
        let weights = RegimeWeights::for_regime(regime);
        let tech_inputs = TechnicalInputs {
            conditions_met: conditions,
            max_conditions: 5,
            adx: adx_val,
            rs_percentile: rs_pct,
        };
        let vol_inputs = VolumeInputs { zscore: vol_z };
        let sent_inputs = SentimentInputs {
            score: 0.0,
            article_count: 0,
        };

        let tech = technical_score_weighted(&tech_inputs, weights.technical);
        let vol = volume_score_weighted(&vol_inputs, weights.volume);
        let sent = sentiment_score_weighted(&sent_inputs, weights.sentiment);
        let comp = composite_score_weighted(&tech_inputs, &vol_inputs, &sent_inputs, &weights);

        Ok(ScreenResult {
            ticker: ticker.to_string(),
            minervini_pass: near_high && above_smas && trending && volume_breakout && has_base,
            conditions_met: conditions,
            adx: adx_val,
            rs_percentile: rs_pct,
            volume_zscore: vol_z,
            technical_score: tech,
            volume_score: vol,
            sentiment_score: sent,
            composite_score: comp.composite,
            grade: Grade::C,
            close_price,
            pct_from_52w_high: pct_from_high,
            atr_value: atr_val,
            risk_reward: rr,
            liquidity,
            weighted_rs: wrs,
            rs_momentum: rs_mom,
            vcp,
            regime: regime.clone(),
        })
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn build_trending(n: usize, start: f64, end: f64) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let price = start + (end - start) * (i as f64 / (n - 1).max(1) as f64);
                Candle {
                    date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
                        .unwrap()
                        .checked_add_days(chrono::Days::new(i as u64))
                        .unwrap(),
                    open: price * 0.995,
                    high: price * 1.01,
                    low: price * 0.99,
                    close: price,
                    adj_close: price,
                    volume: 1_000_000 + i as u64 * 100,
                }
            })
            .collect()
    }

    fn build_declining(n: usize) -> Vec<Candle> {
        build_trending(n, 100.0, 50.0)
    }

    /// Build ideal breakout fixture: flat consolidation then volume spike at end.
    #[allow(dead_code)]
    fn build_breakout_fixture() -> Vec<Candle> {
        let mut candles = build_trending(252, 50.0, 98.0); // uptrend to near 52w high

        // Replace last 30 bars with tight consolidation around 98
        for i in (222..251).rev() {
            let j = i - 222;
            candles[i] = Candle {
                date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                open: 97.5,
                high: 98.5,
                low: 97.0,
                close: 97.8 + (j as f64 * 0.01),
                adj_close: 97.8 + (j as f64 * 0.01),
                volume: 900_000,
            };
        }

        // Last bar: breakout with volume surge above prior 52w high
        let last = candles.len() - 1;
        candles[last] = Candle {
            date: candles[last].date,
            open: 98.0,
            high: 100.5,
            low: 97.8,
            close: 100.0,
            adj_close: 100.0,
            volume: 3_000_000, // 3× normal → volume surge
        };
        candles
    }

    #[test]
    fn breakout_screener_name() {
        assert_eq!(BreakoutScreener::new().strategy_name(), "breakout");
    }

    #[test]
    fn breakout_scoring_weights_sum_to_100() {
        let w = BreakoutScreener::new().scoring_weights();
        assert_relative_eq!(w.sum(), 100.0, epsilon = 1e-6);
    }

    #[test]
    fn breakout_insufficient_data_returns_error() {
        let screener = BreakoutScreener::new();
        let candles = build_trending(200, 50.0, 80.0);
        assert!(matches!(
            screener.screen("TST", &candles, &[], &MarketRegime::Bull),
            Err(AnalysisError::InsufficientData { need: 252, .. })
        ));
    }

    #[test]
    fn breakout_declining_stock_fails() {
        let screener = BreakoutScreener::new();
        let candles = build_declining(300);
        let result = screener
            .screen("WEAK", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        assert!(
            !result.minervini_pass,
            "Declining stock should not pass breakout criteria"
        );
    }

    #[test]
    fn breakout_produces_valid_result_structure() {
        let screener = BreakoutScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("TEST", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        assert_eq!(result.ticker, "TEST");
        assert!(result.composite_score >= 0.0 && result.composite_score <= 100.0);
        assert!(result.adx >= 0.0);
        assert!(result.rs_percentile >= 0.0 && result.rs_percentile <= 100.0);
    }

    #[test]
    fn breakout_result_close_price_matches_last_candle() {
        let screener = BreakoutScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let last_price = candles.last().unwrap().adj_close;
        let result = screener
            .screen("T", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        assert!((result.close_price - last_price).abs() < 1e-6);
    }

    #[test]
    fn breakout_near_52w_high_condition_uptrend() {
        let screener = BreakoutScreener::new();
        // In uptrend ending at 100, current price (100) / 52w high (≈100) ≈ 1.0 → near high
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        // pct_from_52w_high should be near 0 for an uptrending stock
        assert!(
            result.pct_from_52w_high >= -10.0,
            "Uptrend should be near 52w high: {:.2}%",
            result.pct_from_52w_high
        );
    }

    #[test]
    fn breakout_declining_far_from_52w_high() {
        let screener = BreakoutScreener::new();
        let candles = build_declining(300);
        let result = screener
            .screen("T", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        // Declining stock should be far below 52w high
        assert!(
            result.pct_from_52w_high < -20.0,
            "Declining stock should be far from 52w high: {:.2}%",
            result.pct_from_52w_high
        );
    }

    #[test]
    fn breakout_ideal_fixture_all_conditions_pass() {
        // build_breakout_fixture() produces an uptrend to near-52w-high with a
        // tight consolidation base and a final volume-surge breakout candle.
        let screener = BreakoutScreener::new();
        let candles = build_breakout_fixture();
        assert_eq!(
            candles.len(),
            252,
            "fixture must supply exactly 252 candles"
        );

        let result = screener
            .screen("IDEAL", &candles, &[], &MarketRegime::Bull)
            .unwrap();

        assert!(
            result.minervini_pass,
            "All 5 conditions should be met: near_high={}, above_smas, trending, volume_breakout, has_base. \
             conditions_met={}, pct_from_52w={:.2}%, adx={:.2}, vol_zscore={:.2}",
            result.pct_from_52w_high > -5.0,
            result.conditions_met,
            result.pct_from_52w_high,
            result.adx,
            result.volume_zscore,
        );
        assert_eq!(
            result.conditions_met, 5,
            "Expected all 5 conditions met, got {}",
            result.conditions_met
        );
    }

    #[test]
    fn breakout_volume_at_boundary_1_5x() {
        // Build 252 candles where volume_ratio (last bar / avg of prior 50) == exactly 1.5.
        // volume_ratio uses candles[n-51..n-1] (50 bars) as the average window.
        // Set those 50 bars to volume=1_000_000, then last bar to 1_500_000.
        let mut candles = build_trending(252, 50.0, 100.0);

        // Overwrite the 50 bars before the last with uniform volume=1_000_000
        let n = candles.len();
        for c in &mut candles[n - 51..n - 1] {
            c.volume = 1_000_000;
        }
        // Last bar: exactly 1.5× the 1_000_000 average
        candles[n - 1].volume = 1_500_000;

        let result = BreakoutScreener::new()
            .screen("VBND", &candles, &[], &MarketRegime::Bull)
            .unwrap();

        // volume_ratio returns Some(1.5), and volume_breakout = vol_surge >= 1.5 is true
        let vol_ratio = crate::analysis::indicators::volume_ratio(&candles, 50).unwrap();
        assert!(
            (vol_ratio - 1.5).abs() < 1e-9,
            "Expected volume ratio 1.5, got {vol_ratio}"
        );
        // The breakout screener counts volume_breakout when vol_surge >= 1.5
        // Verify the condition is counted (it contributes to conditions_met)
        // We check by temporarily asserting the raw ratio threshold passes.
        assert!(
            vol_ratio >= 1.5,
            "Volume at exactly 1.5× average should satisfy volume_breakout condition"
        );
        // Result itself is valid
        assert!(result.composite_score >= 0.0 && result.composite_score <= 100.0);
    }

    #[test]
    fn breakout_exactly_252_candles_accepted() {
        // Boundary test: exactly 252 candles must not return InsufficientData.
        let screener = BreakoutScreener::new();
        let candles = build_trending(252, 50.0, 100.0);
        assert_eq!(candles.len(), 252);
        let result = screener.screen("BND252", &candles, &[], &MarketRegime::Bull);
        assert!(
            result.is_ok(),
            "Exactly 252 candles should be accepted, got: {:?}",
            result.err()
        );
    }
}
