use crate::analysis::indicators::{
    adx, atr, check_minervini_conditions, classify_liquidity, detect_vcp, price_return_252d,
    rs_percentile, volume_zscore, weighted_rs, LiquidityClass, VcpResult,
};
use crate::analysis::scorer::{
    composite_score_weighted, technical_score_weighted, volume_score_weighted, Grade,
    RegimeWeights, SentimentInputs, TechnicalInputs, VolumeInputs,
};
use crate::data::models::{Candle, MarketRegime};
use crate::error::AnalysisError;

// ─── Result Types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScreenResult {
    pub ticker: String,
    pub minervini_pass: bool,
    pub conditions_met: u8,
    pub adx: f64,
    pub rs_percentile: f64,
    pub volume_zscore: f64,
    pub technical_score: f64,
    pub volume_score: f64,
    pub sentiment_score: f64,
    pub composite_score: f64,
    pub grade: Grade,
    pub close_price: f64,
    pub pct_from_52w_high: f64,
    // Phase 2.5 additions
    pub atr_value: f64,
    pub risk_reward: f64,
    pub liquidity: LiquidityClass,
    pub weighted_rs: f64,
    pub rs_momentum: Option<f64>,
    pub vcp: Option<VcpResult>,
    pub regime: MarketRegime,
}

// ─── Scoring Weights ─────────────────────────────────────────────────────────

/// Per-strategy scoring weight distribution.
///
/// Weights represent the maximum points each component contributes to the
/// 0–100 composite score. They must sum to 100.
#[derive(Debug, Clone, PartialEq)]
pub struct ScoringWeights {
    pub technical: f64,
    pub volume: f64,
    pub sentiment: f64,
}

impl ScoringWeights {
    /// Standard Minervini 55/30/15 weights.
    pub fn standard() -> Self {
        Self {
            technical: 55.0,
            volume: 30.0,
            sentiment: 15.0,
        }
    }

    /// Breakout strategy: 30% proximity, 30% volume, 25% consolidation, 15% RS.
    /// Mapped to tech/vol/sent split: technical covers proximity+RS, volume covers surge.
    pub fn breakout() -> Self {
        Self {
            technical: 45.0,
            volume: 40.0,
            sentiment: 15.0,
        }
    }

    /// Mean-reversion strategy: 30% RSI depth, 25% SMA distance, 25% RS, 20% vol contraction.
    pub fn mean_reversion() -> Self {
        Self {
            technical: 55.0,
            volume: 20.0,
            sentiment: 25.0,
        }
    }

    /// Trend-strength strategy: 35% ADX, 20% DI, 20% MA alignment, 15% RS, 10% volume.
    pub fn trend_strength() -> Self {
        Self {
            technical: 75.0,
            volume: 10.0,
            sentiment: 15.0,
        }
    }

    /// Sum of all weights (should always be 100).
    pub fn sum(&self) -> f64 {
        self.technical + self.volume + self.sentiment
    }
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self::standard()
    }
}

// ─── Screener Trait ──────────────────────────────────────────────────────────

pub trait Screener: Send + Sync {
    /// Screen a single ticker.
    /// `rs_universe_returns` is the 252-day return for all stocks in the universe,
    /// used to compute this ticker's relative strength percentile.
    /// `regime` is the current market regime for adaptive scoring.
    fn screen(
        &self,
        ticker: &str,
        candles: &[Candle],
        rs_universe_returns: &[f64],
        regime: &MarketRegime,
    ) -> Result<ScreenResult, AnalysisError>;

    /// Strategy-specific scoring weights.
    /// Default returns standard 55/30/15 Minervini weights.
    fn scoring_weights(&self) -> ScoringWeights {
        ScoringWeights::default()
    }

    /// Human-readable name used for CLI display and registry lookup.
    fn strategy_name(&self) -> &'static str {
        "unknown"
    }

    /// One-line description shown by `ghost-alpha strategies`.
    fn description(&self) -> &'static str {
        ""
    }
}

pub trait SentimentScorer: Send + Sync {
    fn score(&self, headlines: &[&str]) -> Result<f64, AnalysisError>;
}

// ─── MinerviniScreener ────────────────────────────────────────────────────────

pub struct MinerviniScreener;

impl MinerviniScreener {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MinerviniScreener {
    fn default() -> Self {
        Self::new()
    }
}

impl Screener for MinerviniScreener {
    fn strategy_name(&self) -> &'static str {
        "minervini"
    }

    fn description(&self) -> &'static str {
        "Stage 2 uptrend momentum — buy stocks in confirmed uptrends before acceleration"
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

        // ── Minervini conditions ──────────────────────────────────────────────
        let mut conditions = check_minervini_conditions(&closes)?;

        // ── ADX (14-period) ───────────────────────────────────────────────────
        let adx_val = adx(candles, 14)
            .ok()
            .and_then(|v| v.last().map(|p| p.adx))
            .unwrap_or(0.0);

        // ── 252-day return and RS percentile ─────────────────────────────────
        let this_return = price_return_252d(candles).unwrap_or(0.0);
        let rs_pct = if rs_universe_returns.is_empty() {
            50.0
        } else {
            rs_percentile(this_return, rs_universe_returns)
        };

        // RS condition: above 70th percentile
        conditions.rs_above_threshold = rs_pct >= 70.0;

        // ── Volume Z-score (20-day window) ────────────────────────────────────
        let vol_z = volume_zscore(candles, 20)
            .ok()
            .and_then(|v| v.last().copied())
            .unwrap_or(0.0);

        // ── 52-week high distance ─────────────────────────────────────────────
        let window = &closes[closes.len().saturating_sub(252)..];
        let high_52w = window.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let pct_from_high = if high_52w > 0.0 {
            (close_price - high_52w) / high_52w * 100.0
        } else {
            0.0
        };

        // ── Phase 2.5: ATR, Risk-Reward, Liquidity, Weighted RS, VCP ─────────
        let atr_val = atr(candles, 14).unwrap_or(0.0);

        let sma50 = crate::analysis::indicators::sma(&closes, 50)
            .ok()
            .and_then(|v| v.last().copied())
            .unwrap_or(close_price);
        let rr =
            crate::analysis::indicators::risk_reward_ratio(close_price, sma50, atr_val, high_52w);

        let liquidity = classify_liquidity(candles, 20);
        let wrs = weighted_rs(candles).unwrap_or(0.0);
        let vcp = detect_vcp(candles, 120).ok();

        // ── RS momentum: daily change rate of weighted_rs over 21 days ──────
        let rs_mom = if candles.len() >= 252 + 21 {
            let wrs_now = weighted_rs(candles).unwrap_or(0.0);
            let wrs_21d_ago = weighted_rs(&candles[..candles.len() - 21]).unwrap_or(0.0);
            Some((wrs_now - wrs_21d_ago) / 21.0 * 100.0)
        } else {
            None
        };

        // ── Regime-adaptive scores ────────────────────────────────────────────
        let weights = RegimeWeights::for_regime(regime);
        let tech_inputs = TechnicalInputs {
            conditions_met: conditions.count(),
            max_conditions: 8,
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
        let sent =
            crate::analysis::scorer::sentiment_score_weighted(&sent_inputs, weights.sentiment);
        let comp = composite_score_weighted(&tech_inputs, &vol_inputs, &sent_inputs, &weights);

        Ok(ScreenResult {
            ticker: ticker.to_string(),
            minervini_pass: conditions.passes_all(),
            conditions_met: conditions.count(),
            adx: adx_val,
            rs_percentile: rs_pct,
            volume_zscore: vol_z,
            technical_score: tech,
            volume_score: vol,
            sentiment_score: sent,
            composite_score: comp.composite,
            grade: Grade::C, // assign_grades() sets this after universe ranking
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

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

    fn universe_returns(n: usize) -> Vec<f64> {
        (0..n).map(|i| i as f64 * 0.01).collect()
    }

    fn default_regime() -> MarketRegime {
        MarketRegime::Bull
    }

    // ── Happy path ──────────────────────────────────────────────────────────

    #[test]
    fn screen_trending_stock_produces_result() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("AAPL", &candles, &universe_returns(500), &default_regime())
            .unwrap();
        assert_eq!(result.ticker, "AAPL");
        assert!(result.composite_score > 0.0);
    }

    #[test]
    fn screen_trending_stock_high_score() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("AAPL", &candles, &universe_returns(500), &default_regime())
            .unwrap();
        assert!(
            result.composite_score >= 55.0,
            "Trending stock should score ≥55, got {:.1}",
            result.composite_score
        );
    }

    #[test]
    fn screen_trending_stock_7_conditions_met() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let universe: Vec<f64> = vec![-0.5; 500];
        let result = screener
            .screen("NVDA", &candles, &universe, &default_regime())
            .unwrap();
        assert_eq!(result.conditions_met, 8, "Should meet all 8 when RS is top");
        assert!(result.minervini_pass);
    }

    #[test]
    fn screen_declining_stock_low_score() {
        let screener = MinerviniScreener::new();
        let candles = build_declining(300);
        let result = screener
            .screen("WEAK", &candles, &universe_returns(500), &default_regime())
            .unwrap();
        assert!(
            result.composite_score < 40.0,
            "Declining stock should score <40, got {:.1}",
            result.composite_score
        );
        assert!(!result.minervini_pass);
    }

    #[test]
    fn screen_declining_stock_low_conditions() {
        let screener = MinerviniScreener::new();
        let candles = build_declining(300);
        let result = screener
            .screen("WEAK", &candles, &universe_returns(500), &default_regime())
            .unwrap();
        assert!(
            result.conditions_met <= 2,
            "Declining stock should fail most Minervini conditions, got {}",
            result.conditions_met
        );
    }

    // ── Error cases ──────────────────────────────────────────────────────────

    #[test]
    fn screen_insufficient_data_returns_error() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(200, 50.0, 80.0);
        assert!(matches!(
            screener.screen("TST", &candles, &[], &default_regime()),
            Err(AnalysisError::InsufficientData { need: 252, .. })
        ));
    }

    // ── Field validation ─────────────────────────────────────────────────────

    #[test]
    fn screen_result_adx_non_negative() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &default_regime())
            .unwrap();
        assert!(result.adx >= 0.0, "ADX should be non-negative");
    }

    #[test]
    fn screen_result_rs_bounded() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &universe_returns(500), &default_regime())
            .unwrap();
        assert!(
            result.rs_percentile >= 0.0 && result.rs_percentile <= 100.0,
            "RS should be 0–100"
        );
    }

    #[test]
    fn screen_result_close_price_matches_last_candle() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let last_price = candles.last().unwrap().adj_close;
        let result = screener
            .screen("T", &candles, &[], &default_regime())
            .unwrap();
        assert!((result.close_price - last_price).abs() < 1e-6);
    }

    #[test]
    fn screen_empty_universe_returns_uses_50_pct_rs() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &default_regime())
            .unwrap();
        assert_relative_eq!(result.rs_percentile, 50.0, epsilon = 0.1);
    }

    // ── Phase 2.5 field validation ──────────────────────────────────────────

    #[test]
    fn screen_result_atr_populated() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &default_regime())
            .unwrap();
        assert!(
            result.atr_value > 0.0,
            "ATR should be positive, got {:.4}",
            result.atr_value
        );
    }

    #[test]
    fn screen_result_risk_reward_reasonable() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &default_regime())
            .unwrap();
        // Just verify it's a finite number
        assert!(result.risk_reward.is_finite(), "R:R should be finite");
    }

    #[test]
    fn screen_result_liquidity_class_populated() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &default_regime())
            .unwrap();
        // The trending builder uses volume=1M + i*100, close~100 → ~$100M → High
        let _class = &result.liquidity; // just ensure it's populated
    }

    #[test]
    fn screen_result_weighted_rs_populated() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &default_regime())
            .unwrap();
        assert!(
            result.weighted_rs > 0.0,
            "Uptrend weighted_rs should be positive"
        );
    }

    #[test]
    fn screen_different_regimes_different_scores() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let bull = screener
            .screen("T", &candles, &[], &MarketRegime::Bull)
            .unwrap();
        let bear = screener
            .screen("T", &candles, &[], &MarketRegime::Bear)
            .unwrap();
        // Different weight distributions should produce different composites
        assert!(
            (bull.composite_score - bear.composite_score).abs() > 0.01,
            "Bull={:.1} vs Bear={:.1} should differ",
            bull.composite_score,
            bear.composite_score
        );
    }

    #[test]
    fn screen_result_vcp_populated() {
        let screener = MinerviniScreener::new();
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &default_regime())
            .unwrap();
        // VCP may or may not be detected, but should be populated
        assert!(
            result.vcp.is_some(),
            "VCP result should be Some (even if not detected)"
        );
    }

    #[test]
    fn rs_momentum_populated_with_enough_data() {
        let screener = MinerviniScreener::new();
        // 300 bars > 252 + 21 = 273, so rs_momentum should be Some
        let candles = build_trending(300, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &default_regime())
            .unwrap();
        assert!(
            result.rs_momentum.is_some(),
            "rs_momentum should be Some with 300 bars"
        );
    }

    #[test]
    fn rs_momentum_none_with_insufficient_data() {
        let screener = MinerviniScreener::new();
        // 260 bars < 252 + 21 = 273, so rs_momentum should be None
        let candles = build_trending(260, 50.0, 100.0);
        let result = screener
            .screen("T", &candles, &[], &default_regime())
            .unwrap();
        assert!(
            result.rs_momentum.is_none(),
            "rs_momentum should be None with 260 bars"
        );
    }

    #[test]
    fn rs_momentum_positive_for_accelerating_uptrend() {
        let screener = MinerviniScreener::new();
        // Build an exponentially accelerating uptrend where percentage returns increase.
        // Linear price trends (build_trending) actually decelerate in % terms, so we
        // construct an exponential curve where the daily growth rate ramps up.
        let n = 300usize;
        let candles: Vec<Candle> = (0..n)
            .map(|i| {
                // Exponential growth: price = 50 * e^(k*i^2) with small k
                // This ensures percentage returns accelerate over time
                let t = i as f64 / n as f64;
                let price = 50.0 * (3.0 * t * t).exp(); // accelerating exponential
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
            .collect();
        let result = screener
            .screen("T", &candles, &[], &default_regime())
            .unwrap();
        if let Some(mom) = result.rs_momentum {
            // In an accelerating trend, recent weighted_rs should be higher than 21 days ago
            assert!(
                mom > 0.0,
                "Accelerating uptrend should have positive rs_momentum, got {:.4}",
                mom
            );
        }
    }

    use approx::assert_relative_eq;

    // ── ScoringWeights ───────────────────────────────────────────────────────

    #[test]
    fn scoring_weights_standard_sums_to_100() {
        let w = ScoringWeights::standard();
        assert_relative_eq!(w.sum(), 100.0, epsilon = 1e-6);
    }

    #[test]
    fn scoring_weights_breakout_sums_to_100() {
        let w = ScoringWeights::breakout();
        assert_relative_eq!(w.sum(), 100.0, epsilon = 1e-6);
    }

    #[test]
    fn scoring_weights_mean_reversion_sums_to_100() {
        let w = ScoringWeights::mean_reversion();
        assert_relative_eq!(w.sum(), 100.0, epsilon = 1e-6);
    }

    #[test]
    fn scoring_weights_trend_strength_sums_to_100() {
        let w = ScoringWeights::trend_strength();
        assert_relative_eq!(w.sum(), 100.0, epsilon = 1e-6);
    }

    #[test]
    fn scoring_weights_default_is_standard() {
        let d = ScoringWeights::default();
        let s = ScoringWeights::standard();
        assert_eq!(d, s);
    }

    #[test]
    fn minervini_screener_has_correct_strategy_name() {
        let s = MinerviniScreener::new();
        assert_eq!(s.strategy_name(), "minervini");
    }

    #[test]
    fn minervini_screener_default_weights() {
        let s = MinerviniScreener::new();
        let w = s.scoring_weights();
        assert_relative_eq!(w.sum(), 100.0, epsilon = 1e-6);
        assert_relative_eq!(w.technical, 55.0, epsilon = 1e-6);
    }
}
