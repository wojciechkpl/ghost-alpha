use crate::analysis::indicators::{
    adx, atr, classify_liquidity, detect_vcp, price_return_252d, risk_reward_ratio, rs_percentile,
    rsi, sma, volume_zscore, weighted_rs,
};
use crate::analysis::scorer::{
    composite_score_weighted, sentiment_score_weighted, technical_score_weighted,
    volume_score_weighted, Grade, RegimeWeights, SentimentInputs, TechnicalInputs, VolumeInputs,
};
use crate::analysis::screener::{ScoringWeights, ScreenResult, Screener};
use crate::data::models::{Candle, MarketRegime};
use crate::error::AnalysisError;

// ─── MeanReversionScreener ────────────────────────────────────────────────────

/// Mean-reversion (oversold bounce) strategy.
///
/// Buys quality stocks that are temporarily oversold in a bullish market.
/// Contrarian approach — suitable for short-to-medium term entries on pullbacks.
///
/// **Selection criteria:**
/// - RSI(14) < 30 (oversold)
/// - Price below 20-SMA but above 200-SMA (pullback, not breakdown)
/// - RS percentile > 50 (still relatively strong on longer timeframe)
/// - Market regime = Bull or Sideways (don't catch falling knives in bears)
/// - ADX < 25 (not in a strong downtrend)
pub struct MeanReversionScreener;

impl MeanReversionScreener {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MeanReversionScreener {
    fn default() -> Self {
        Self::new()
    }
}

impl Screener for MeanReversionScreener {
    fn strategy_name(&self) -> &'static str {
        "mean-reversion"
    }

    fn description(&self) -> &'static str {
        "Buy quality stocks temporarily oversold in Bull/Sideways regimes — contrarian entries"
    }

    fn scoring_weights(&self) -> ScoringWeights {
        ScoringWeights::mean_reversion()
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

        // ── RSI(14) ───────────────────────────────────────────────────────────
        let rsi_val = rsi(&closes, 14).unwrap_or(50.0);
        let oversold = rsi_val < 30.0;

        // ── SMAs ─────────────────────────────────────────────────────────────
        let sma20_vals = sma(&closes, 20).ok();
        let sma200_vals = sma(&closes, 200).ok();
        let sma20 = sma20_vals
            .as_ref()
            .and_then(|v| v.last().copied())
            .unwrap_or(close_price);
        let sma200 = sma200_vals
            .as_ref()
            .and_then(|v| v.last().copied())
            .unwrap_or(close_price);

        // Price must be below 20-SMA (pullback) but above 200-SMA (not a breakdown)
        let pullback_not_breakdown = close_price < sma20 && close_price > sma200;

        // ── ADX ───────────────────────────────────────────────────────────────
        let adx_result = adx(candles, 14).ok();
        let adx_val = adx_result
            .as_ref()
            .and_then(|v| v.last())
            .map(|p| p.adx)
            .unwrap_or(0.0);
        // ADX < 25 means not in a strong directional downtrend
        let not_strong_downtrend = adx_val < 25.0;

        // ── RS Percentile ─────────────────────────────────────────────────────
        let this_return = price_return_252d(candles).unwrap_or(0.0);
        let rs_pct = if rs_universe_returns.is_empty() {
            50.0
        } else {
            rs_percentile(this_return, rs_universe_returns)
        };
        let quality_filter = rs_pct > 50.0;

        // ── Regime filter: only Bull or Sideways ─────────────────────────────
        let regime_ok = !matches!(regime, MarketRegime::Bear);

        // ── Volume Z-score ────────────────────────────────────────────────────
        let vol_z = volume_zscore(candles, 20)
            .ok()
            .and_then(|v| v.last().copied())
            .unwrap_or(0.0);

        // ── Conditions count (out of 5) ───────────────────────────────────────
        let conditions: u8 = [
            oversold,
            pullback_not_breakdown,
            not_strong_downtrend,
            quality_filter,
            regime_ok,
        ]
        .iter()
        .filter(|&&b| b)
        .count() as u8;

        // ── 52-week high distance ─────────────────────────────────────────────
        let window_52w = &closes[closes.len().saturating_sub(252)..];
        let high_52w = window_52w.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let pct_from_high = if high_52w > 0.0 {
            (close_price - high_52w) / high_52w * 100.0
        } else {
            0.0
        };

        // ── Phase 2.5 fields ─────────────────────────────────────────────────
        let sma50_val = sma(&closes, 50)
            .ok()
            .and_then(|v| v.last().copied())
            .unwrap_or(close_price);
        let atr_val = atr(candles, 14).unwrap_or(0.0);
        let rr = risk_reward_ratio(close_price, sma50_val, atr_val, high_52w);
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
            minervini_pass: oversold
                && pullback_not_breakdown
                && not_strong_downtrend
                && quality_filter
                && regime_ok,
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

    fn build_candle(i: usize, price: f64, vol: u64) -> Candle {
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
            volume: vol,
        }
    }

    fn build_trending(n: usize, start: f64, end: f64) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let price = start + (end - start) * (i as f64 / (n - 1).max(1) as f64);
                build_candle(i, price, 1_000_000)
            })
            .collect()
    }

    /// Build an oversold-bounce fixture: uptrend establishes 200-SMA base, then pulls back
    /// below 20-SMA (with RSI < 30), but stays above 200-SMA.
    fn build_oversold_fixture() -> Vec<Candle> {
        // 200 bars uptrending from 80→100 (establishes 200-SMA)
        let mut candles: Vec<Candle> = (0..200)
            .map(|i| {
                let price = 80.0 + 20.0 * (i as f64 / 199.0);
                build_candle(i, price, 1_000_000)
            })
            .collect();
        // 30 bars of steady decline (oversells RSI) but price stays above 200-SMA level (~90)
        // The 200-SMA after 200 bars of 80→100 is ~90
        for i in 0..52 {
            let price = 100.0 - (i as f64 * 0.5); // declines from 100 to ~74, but we need > sma200
                                                  // Keep above ~85 to stay above 200-SMA
            let price = price.max(87.0);
            candles.push(build_candle(200 + i, price, 800_000));
        }
        candles
    }

    #[test]
    fn mean_reversion_screener_name() {
        assert_eq!(
            MeanReversionScreener::new().strategy_name(),
            "mean-reversion"
        );
    }

    #[test]
    fn mean_reversion_scoring_weights_sum_to_100() {
        let w = MeanReversionScreener::new().scoring_weights();
        assert_relative_eq!(w.sum(), 100.0, epsilon = 1e-6);
    }

    #[test]
    fn mean_reversion_insufficient_data_returns_error() {
        let screener = MeanReversionScreener::new();
        let candles = build_trending(200, 50.0, 80.0);
        assert!(matches!(
            screener.screen("TST", &candles, &[], &MarketRegime::Bull),
            Err(AnalysisError::InsufficientData { need: 252, .. })
        ));
    }

    #[test]
    fn mean_reversion_produces_valid_result() {
        let screener = MeanReversionScreener::new();
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
    fn mean_reversion_close_price_matches_last_candle() {
        let screener = MeanReversionScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let last = candles.last().unwrap().adj_close;
        let result = screener
            .screen("T", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        assert!((result.close_price - last).abs() < 1e-6);
    }

    #[test]
    fn mean_reversion_trending_stock_not_oversold() {
        // A strongly trending stock has RSI well above 30, so it should not pass
        let screener = MeanReversionScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        // In a continuous uptrend, RSI is typically high — oversold criterion fails
        assert!(
            !result.minervini_pass,
            "Strongly trending stock should not pass mean-reversion criteria"
        );
    }

    #[test]
    fn mean_reversion_bear_regime_fails_regime_filter() {
        let screener = MeanReversionScreener::new();
        let candles = build_oversold_fixture();
        let result = screener
            .screen("T", &candles, &[], &MarketRegime::Bear)
            .unwrap();
        // regime_ok = false in Bear → minervini_pass must be false
        assert!(
            !result.minervini_pass,
            "Bear regime should fail the regime filter"
        );
    }

    #[test]
    fn mean_reversion_regime_condition_in_bear_fails() {
        let screener = MeanReversionScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let bull = screener
            .screen("T", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        let bear = screener
            .screen("T", &candles, &[], &MarketRegime::Bear)
            .unwrap();
        // Bear regime reduces conditions_met by at least 1 (regime_ok condition)
        assert!(
            bull.conditions_met >= bear.conditions_met,
            "Bull should have >= conditions than Bear"
        );
    }

    #[test]
    fn mean_reversion_atr_populated() {
        let screener = MeanReversionScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        assert!(result.atr_value > 0.0, "ATR should be positive");
    }

    #[test]
    fn mean_reversion_bull_all_conditions_pass() {
        // Build a fixture satisfying all 5 mean-reversion conditions:
        //
        //   1. RSI(14) < 30  — achieved by 3 sharp decline bars after long oscillation
        //   2. price < SMA20 && price > SMA200  — oscillation plateau anchors SMA200 low
        //   3. ADX(14) < 25  — symmetric oscillation decays ADX from uptrend to < 3,
        //                       then only 3 decline bars (Wilder-smoothed from ~2 → ~11)
        //   4. rs_pct > 50   — 252d return ≈ +1900% vs near-zero universe
        //   5. regime = Bull — passed as argument
        //
        // Three-phase structure (283 bars total):
        //
        //  Phase 1 (220 bars): uptrend 100 → 2000 via build_candle.
        //    Establishes SMA200 well below the plateau price.
        //    ADX converges toward 100 (strong uptrend).
        //
        //  Phase 2 (60 bars): symmetric alternating +2 / -2 oscillation around 2000.
        //    +DM ≈ -DM each cycle → DX ≈ 1 → ADX decays from ~100 to ~2.4
        //    (Wilder: 100 × (13/14)^60 ≈ 1.4, plus steady-state contribution ≈ 1.0).
        //    RSI converges toward ~50 (equal gains and losses).
        //
        //  Phase 3 (3 bars): each bar declines by 7 (close −7, high/low from price).
        //    RSI: avg_gain ≈ 0.86, avg_loss ≈ 2.21 → RS ≈ 0.39 → RSI ≈ 27.9 < 30 ✓
        //    ADX: DX ≈ 46 (from partially rotated DM, not 100) over 3 bars from 2.4
        //         → Wilder result ≈ 11 < 25 ✓
        //    SMA200 (last 200 bars): avg ≈ 1594 << price 1979 ✓
        //    SMA20 (last 20 bars): avg ≈ 1998 > price 1979 ✓

        // ── Phase 1: uptrend 100 → 2000 ──────────────────────────────────────
        let mut candles: Vec<Candle> = (0..220_usize)
            .map(|i| {
                let price = 100.0 + 1900.0 * (i as f64 / 219.0);
                build_candle(i, price, 1_000_000)
            })
            .collect();

        // ── Phase 2: symmetric ±2 oscillation around 2000 ────────────────────
        // Alternates between 2001 and 1999 so +DM ≈ -DM per cycle → DX ≈ 1.
        let phase2_start = candles.len();
        for j in 0..60_usize {
            let price = if j % 2 == 0 { 2001.0 } else { 1999.0 };
            candles.push(build_candle(phase2_start + j, price, 1_000_000));
        }

        // ── Phase 3: 3 sharp decline bars (−7 each) ──────────────────────────
        let phase3_start = candles.len();
        let mut close_p3 = 2000.0_f64;
        for j in 0..3_usize {
            close_p3 -= 7.0;
            candles.push(build_candle(phase3_start + j, close_p3, 800_000));
        }

        assert!(candles.len() >= 252, "fixture needs ≥252 bars");

        // Universe: near-zero returns → this stock's +1900% 252d return → rs_pct ≈ 100%.
        let universe_returns: Vec<f64> = (-50..50).map(|i| i as f64 * 0.001).collect();

        let screener = MeanReversionScreener::new();
        let result = screener
            .screen("MR", &candles, &universe_returns, &MarketRegime::Bull)
            .unwrap();

        assert!(
            result.minervini_pass,
            "Bull regime oversold fixture should pass all 5 conditions; \
             conditions_met={}, adx={:.2}, rs_pct={:.1}, close={:.2}",
            result.conditions_met, result.adx, result.rs_percentile, result.close_price,
        );
        assert_eq!(
            result.conditions_met, 5,
            "Expected all 5 conditions met, got {}",
            result.conditions_met
        );
    }

    #[test]
    fn mean_reversion_rsi_boundary_at_30_fails() {
        // The oversold condition is `rsi_val < 30.0` (strict less-than).
        // This test verifies that RSI >= 30 does NOT trigger oversold.
        //
        // Build 300 candles with alternating +9/-7 moves (more gains than losses
        // in magnitude × frequency).  With 150 gains of +9 and 150 losses of -7
        // the Wilder seed gives avg_gain ≈ 4.5, avg_loss ≈ 3.5, RS ≈ 1.286,
        // RSI ≈ 56.  Well above 30 → oversold = false.
        //
        // Starting price is high enough to stay positive even after 150 declines
        // of 7 (worst case: 150 * 7 = 1050, so start at 2000).
        let mut prices: Vec<f64> = Vec::with_capacity(301);
        prices.push(2000.0);
        for i in 0..300_usize {
            let prev = *prices.last().unwrap();
            if i % 2 == 0 {
                prices.push(prev + 9.0); // gain bar
            } else {
                prices.push(prev - 7.0); // loss bar
            }
        }

        let rsi_val = rsi(&prices, 14).unwrap();
        assert!(
            rsi_val >= 30.0,
            "RSI should be >= 30 for this fixture, got {rsi_val:.4}"
        );

        // Build Candle structs from those prices.
        let candles: Vec<Candle> = prices
            .iter()
            .enumerate()
            .map(|(i, &p)| build_candle(i, p.max(1.0), 1_000_000))
            .collect();

        // The oversold condition is RSI < 30 (strict less-than).
        // RSI >= 30 → oversold = false → minervini_pass must be false.
        let screener = MeanReversionScreener::new();
        let result = screener
            .screen("RSIBND", &candles, &[], &MarketRegime::Bull)
            .unwrap();

        assert!(
            !result.minervini_pass,
            "RSI = {rsi_val:.4} >= 30.0 so oversold is false; minervini_pass must be false"
        );
        // oversold = false reduces conditions_met by at least 1
        assert!(
            result.conditions_met < 5,
            "With RSI >= 30, conditions_met must be < 5, got {}",
            result.conditions_met
        );
    }

    #[test]
    fn mean_reversion_exactly_252_candles_accepted() {
        // Boundary test: exactly 252 candles must not return InsufficientData.
        let screener = MeanReversionScreener::new();
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
