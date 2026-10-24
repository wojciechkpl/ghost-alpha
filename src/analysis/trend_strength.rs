use crate::analysis::indicators::{
    adx, atr, classify_liquidity, detect_vcp, price_return_252d, risk_reward_ratio, rs_percentile,
    sma, volume_zscore, weighted_rs,
};
use crate::analysis::scorer::{
    composite_score_weighted, sentiment_score_weighted, technical_score_weighted,
    volume_score_weighted, Grade, RegimeWeights, SentimentInputs, TechnicalInputs, VolumeInputs,
};
use crate::analysis::screener::{ScoringWeights, ScreenResult, Screener};
use crate::data::models::{Candle, MarketRegime};
use crate::error::AnalysisError;

// ─── TrendStrengthScreener ────────────────────────────────────────────────────

/// ADX momentum strategy.
///
/// Rides strong established trends regardless of entry stage. Focuses on trend
/// strength rather than precise entry timing.
///
/// **Selection criteria:**
/// - ADX > 30 (strong trend)
/// - +DI > -DI (bullish directional movement)
/// - Price above 20-SMA, 50-SMA, and 200-SMA (all MAs aligned)
/// - RS percentile > 70 (top 30% of universe)
/// - Volume above 20-day average (vol_z > 0)
pub struct TrendStrengthScreener;

impl TrendStrengthScreener {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TrendStrengthScreener {
    fn default() -> Self {
        Self::new()
    }
}

impl Screener for TrendStrengthScreener {
    fn strategy_name(&self) -> &'static str {
        "trend-strength"
    }

    fn description(&self) -> &'static str {
        "Ride strong established trends: ADX > 30, all MAs aligned, RS percentile > 70"
    }

    fn scoring_weights(&self) -> ScoringWeights {
        ScoringWeights::trend_strength()
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

        // ── ADX + Directional Indicators ──────────────────────────────────────
        let adx_result = adx(candles, 14).ok();
        let last_adx = adx_result.as_ref().and_then(|v| v.last());
        let adx_val = last_adx.map(|p| p.adx).unwrap_or(0.0);
        let plus_di = last_adx.map(|p| p.plus_di).unwrap_or(0.0);
        let minus_di = last_adx.map(|p| p.minus_di).unwrap_or(0.0);

        let strong_trend = adx_val > 30.0;
        let bullish_di = plus_di > minus_di;

        // ── SMAs — all three must be aligned ─────────────────────────────────
        let sma20_vals = sma(&closes, 20).ok();
        let sma50_vals = sma(&closes, 50).ok();
        let sma200_vals = sma(&closes, 200).ok();

        let sma20 = sma20_vals
            .as_ref()
            .and_then(|v| v.last().copied())
            .unwrap_or(close_price);
        let sma50 = sma50_vals
            .as_ref()
            .and_then(|v| v.last().copied())
            .unwrap_or(close_price);
        let sma200 = sma200_vals
            .as_ref()
            .and_then(|v| v.last().copied())
            .unwrap_or(close_price);

        // All MAs aligned: price > SMA20 > SMA50 > SMA200
        let mas_aligned = close_price > sma20 && sma20 > sma50 && sma50 > sma200;

        // ── RS Percentile ─────────────────────────────────────────────────────
        let this_return = price_return_252d(candles).unwrap_or(0.0);
        let rs_pct = if rs_universe_returns.is_empty() {
            50.0
        } else {
            rs_percentile(this_return, rs_universe_returns)
        };
        let high_rs = rs_pct > 70.0;

        // ── Volume Z-score ────────────────────────────────────────────────────
        let vol_z = volume_zscore(candles, 20)
            .ok()
            .and_then(|v| v.last().copied())
            .unwrap_or(0.0);
        let volume_above_avg = vol_z > 0.0;

        // ── Conditions count (out of 5) ───────────────────────────────────────
        let conditions: u8 = [
            strong_trend,
            bullish_di,
            mas_aligned,
            high_rs,
            volume_above_avg,
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
        let atr_val = atr(candles, 14).unwrap_or(0.0);
        let rr = risk_reward_ratio(close_price, sma50, atr_val, high_52w);
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
            minervini_pass: strong_trend
                && bullish_di
                && mas_aligned
                && high_rs
                && volume_above_avg,
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

    #[test]
    fn trend_strength_screener_name() {
        assert_eq!(
            TrendStrengthScreener::new().strategy_name(),
            "trend-strength"
        );
    }

    #[test]
    fn trend_strength_scoring_weights_sum_to_100() {
        let w = TrendStrengthScreener::new().scoring_weights();
        assert_relative_eq!(w.sum(), 100.0, epsilon = 1e-6);
    }

    #[test]
    fn trend_strength_insufficient_data_returns_error() {
        let screener = TrendStrengthScreener::new();
        let candles = build_trending(200, 50.0, 80.0);
        assert!(matches!(
            screener.screen("TST", &candles, &[], &MarketRegime::Bull),
            Err(AnalysisError::InsufficientData { need: 252, .. })
        ));
    }

    #[test]
    fn trend_strength_produces_valid_result() {
        let screener = TrendStrengthScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("TEST", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        assert_eq!(result.ticker, "TEST");
        assert!(result.composite_score >= 0.0 && result.composite_score <= 100.0);
    }

    #[test]
    fn trend_strength_declining_stock_fails() {
        let screener = TrendStrengthScreener::new();
        let candles = build_declining(300);
        let result = screener
            .screen("WEAK", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        assert!(
            !result.minervini_pass,
            "Declining stock should not pass trend-strength criteria"
        );
    }

    #[test]
    fn trend_strength_uptrend_adx_positive() {
        let screener = TrendStrengthScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        assert!(
            result.adx > 0.0,
            "ADX should be positive in uptrend, got {:.2}",
            result.adx
        );
    }

    #[test]
    fn trend_strength_uptrend_plus_di_dominant() {
        let screener = TrendStrengthScreener::new();
        // In a strong uptrend, +DI > -DI (bullish_di condition should be met)
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        // At least some conditions should be met
        assert!(
            result.conditions_met > 0,
            "Uptrend should meet at least some conditions"
        );
    }

    #[test]
    fn trend_strength_close_price_matches_last_candle() {
        let screener = TrendStrengthScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let last = candles.last().unwrap().adj_close;
        let result = screener
            .screen("T", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        assert!((result.close_price - last).abs() < 1e-6);
    }

    #[test]
    fn trend_strength_high_rs_universe_raises_conditions() {
        let screener = TrendStrengthScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        // Universe where this stock outperforms everyone (RS > 70)
        let top_universe: Vec<f64> = vec![-0.5; 500];
        let result_top = screener
            .screen("T", &candles, &top_universe, &MarketRegime::Bull)
            .unwrap();
        // Universe where this stock is median (RS = 50 → fails > 70 threshold)
        let mid_universe: Vec<f64> = (0..500).map(|i| i as f64 * 0.01).collect();
        let result_mid = screener
            .screen("T", &candles, &mid_universe, &MarketRegime::Bull)
            .unwrap();
        // With top RS, conditions_met should be higher or equal
        assert!(
            result_top.conditions_met >= result_mid.conditions_met,
            "Top RS should produce >= conditions: top={} vs mid={}",
            result_top.conditions_met,
            result_mid.conditions_met
        );
    }

    #[test]
    fn trend_strength_rs_bounded_0_to_100() {
        let screener = TrendStrengthScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let universe: Vec<f64> = (0..500).map(|i| i as f64 * 0.01).collect();
        let result = screener
            .screen("T", &candles, &universe, &MarketRegime::Bull)
            .unwrap();
        assert!(
            result.rs_percentile >= 0.0 && result.rs_percentile <= 100.0,
            "RS percentile should be 0–100, got {:.2}",
            result.rs_percentile
        );
    }

    #[test]
    fn trend_strength_different_regimes_different_scores() {
        let screener = TrendStrengthScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let bull = screener
            .screen("T", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        let bear = screener
            .screen("T", &candles, &[], &MarketRegime::Bear)
            .unwrap();
        assert!(
            (bull.composite_score - bear.composite_score).abs() > 0.01,
            "Bull={:.1} vs Bear={:.1} should differ",
            bull.composite_score,
            bear.composite_score
        );
    }
}
