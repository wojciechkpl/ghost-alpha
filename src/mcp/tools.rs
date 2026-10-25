//! MCP tool definitions for GhostAlpha.
//!
//! Each public function here becomes an MCP tool that AI agents can call.
//! Tools are pure functions that take structured input and return JSON output.

use serde::{Deserialize, Serialize};

use crate::analysis::scorer::{self, SentimentInputs, TechnicalInputs, VolumeInputs};
use crate::analysis::screener::{MinerviniScreener, Screener};
use crate::data::models::{Candle, MarketRegime, RegimeState};

// ─── Tool Input/Output Types ─────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ScreenStockInput {
    /// Stock ticker symbol (e.g., "AAPL", "NVDA")
    pub ticker: String,
    /// OHLCV candle data (minimum 252 bars required)
    pub candles: Vec<Candle>,
    /// 252-day returns for the universe (for RS percentile calculation).
    /// If empty, RS defaults to 50th percentile.
    #[serde(default)]
    pub universe_returns: Vec<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScreenStockOutput {
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
    pub grade: String,
    pub close_price: f64,
    pub pct_from_52w_high: f64,
    pub signal_strength: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetRegimeOutput {
    pub regime: String,
    pub date: String,
    pub spy_close: f64,
    pub spy_sma200: f64,
    pub breadth_pct: f64,
    pub description: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListProvidersOutput {
    pub providers: Vec<String>,
    pub screeners: Vec<String>,
    pub scorers: Vec<String>,
    pub channels: Vec<String>,
    pub active_provider: Option<String>,
    pub active_screener: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConfigOutput {
    pub finnhub_api_key: String,
    pub finnhub_rpm: u32,
    pub telegram_configured: bool,
    pub db_path: String,
    pub critical_threshold: f64,
    pub warning_threshold: f64,
    pub info_threshold: f64,
    pub universe_size: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScoreBreakdownInput {
    pub conditions_met: u8,
    pub adx: f64,
    pub rs_percentile: f64,
    pub volume_zscore: f64,
    pub sentiment_score: f64,
    pub article_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ScoreBreakdownOutput {
    pub technical: f64,
    pub volume: f64,
    pub sentiment: f64,
    pub composite: f64,
    pub grade: String,
    pub breakdown: String,
}

// ─── Tool Implementations ────────────────────────────────────────────────────

/// Screen a single stock through the Minervini 8-point filter.
///
/// Requires at least 252 daily OHLCV candles. Returns technical scores,
/// volume analysis, relative strength, and a composite grade.
pub fn screen_stock(input: ScreenStockInput) -> Result<ScreenStockOutput, String> {
    let screener = MinerviniScreener::new();
    let result = screener
        .screen(
            &input.ticker,
            &input.candles,
            &input.universe_returns,
            &MarketRegime::Sideways,
        )
        .map_err(|e| format!("Screening failed: {e}"))?;

    let signal_strength = match result.composite_score {
        s if s >= 90.0 => "VERY_STRONG",
        s if s >= 70.0 => "STRONG",
        s if s >= 50.0 => "MODERATE",
        s if s >= 30.0 => "WEAK",
        _ => "NO_SIGNAL",
    };

    Ok(ScreenStockOutput {
        ticker: result.ticker,
        minervini_pass: result.minervini_pass,
        conditions_met: result.conditions_met,
        adx: round2(result.adx),
        rs_percentile: round2(result.rs_percentile),
        volume_zscore: round2(result.volume_zscore),
        technical_score: round2(result.technical_score),
        volume_score: round2(result.volume_score),
        sentiment_score: round2(result.sentiment_score),
        composite_score: round2(result.composite_score),
        grade: result.grade.to_string(),
        close_price: round2(result.close_price),
        pct_from_52w_high: round2(result.pct_from_52w_high),
        signal_strength: signal_strength.to_string(),
    })
}

/// Compute a score breakdown from raw indicator values.
///
/// Useful for "what-if" analysis: pass hypothetical indicator values
/// and see how the composite score changes.
pub fn score_breakdown(input: ScoreBreakdownInput) -> ScoreBreakdownOutput {
    let tech_inputs = TechnicalInputs {
        conditions_met: input.conditions_met,
        max_conditions: 8,
        adx: input.adx,
        rs_percentile: input.rs_percentile,
    };
    let vol_inputs = VolumeInputs {
        zscore: input.volume_zscore,
    };
    let sent_inputs = SentimentInputs {
        score: input.sentiment_score,
        article_count: input.article_count,
    };

    let composite = scorer::composite_score(&tech_inputs, &vol_inputs, &sent_inputs);

    let breakdown = format!(
        "Technical: {:.1}/55 (conditions={}/8, ADX={:.1}, RS={:.1}th pct)\n\
         Volume: {:.1}/30 (Z-score={:.2})\n\
         Sentiment: {:.1}/15 (score={:.2}, articles={})\n\
         ─────────────\n\
         Composite: {:.1}/100",
        composite.technical,
        input.conditions_met,
        input.adx,
        input.rs_percentile,
        composite.volume,
        input.volume_zscore,
        composite.sentiment,
        input.sentiment_score,
        input.article_count,
        composite.composite,
    );

    ScoreBreakdownOutput {
        technical: round2(composite.technical),
        volume: round2(composite.volume),
        sentiment: round2(composite.sentiment),
        composite: round2(composite.composite),
        grade: composite.grade.to_string(),
        breakdown,
    }
}

/// Get the current market regime from the database.
pub fn format_regime(state: &RegimeState) -> GetRegimeOutput {
    let description = match state.regime {
        MarketRegime::Bull => {
            "Market is in a bullish regime. SPY is above its 200-day SMA with healthy breadth. \
             Screening signals are high-confidence."
        }
        MarketRegime::Sideways => {
            "Market is in a sideways/transitional regime. Exercise caution with new positions. \
             Consider tighter position sizing."
        }
        MarketRegime::Bear => {
            "Market is in a bearish regime. SPY is below its 200-day SMA with weak breadth. \
             Most screening signals should be treated with skepticism."
        }
    };

    GetRegimeOutput {
        regime: state.regime.to_string(),
        date: state.date.to_string(),
        spy_close: round2(state.spy_close),
        spy_sma200: round2(state.spy_sma200),
        breadth_pct: round2(state.breadth_pct),
        description: description.to_string(),
    }
}

/// Build a masked config output (secrets are redacted).
pub fn masked_config(config: &crate::config::AppConfig) -> ConfigOutput {
    let key = &config.finnhub.api_key;
    let masked = if key.len() > 4 {
        format!("{}...{}", &key[..2], &key[key.len() - 2..])
    } else {
        "****".to_string()
    };

    ConfigOutput {
        finnhub_api_key: masked,
        finnhub_rpm: config.finnhub.requests_per_minute,
        telegram_configured: config.telegram.is_some(),
        db_path: config.storage.db_path.clone(),
        critical_threshold: config.alerts.critical_score_threshold,
        warning_threshold: config.alerts.warning_score_threshold,
        info_threshold: config.alerts.info_score_threshold,
        universe_size: config.universe.len(),
    }
}

/// List all registered plugins.
pub fn list_providers(registry: &crate::plugins::Registry) -> ListProvidersOutput {
    ListProvidersOutput {
        providers: registry
            .provider_names()
            .into_iter()
            .map(String::from)
            .collect(),
        screeners: registry
            .screener_names()
            .into_iter()
            .map(String::from)
            .collect(),
        scorers: registry
            .scorer_names()
            .into_iter()
            .map(String::from)
            .collect(),
        channels: registry
            .channel_names()
            .into_iter()
            .map(String::from)
            .collect(),
        active_provider: registry.active_provider_name().map(String::from),
        active_screener: registry.active_screener_name().map(String::from),
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn make_candles(n: usize, start: f64, end: f64) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let price = start + (end - start) * (i as f64 / (n - 1).max(1) as f64);
                Candle {
                    date: NaiveDate::from_ymd_opt(2024, 1, 1)
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

    #[test]
    fn screen_stock_returns_valid_output() {
        let input = ScreenStockInput {
            ticker: "AAPL".into(),
            candles: make_candles(300, 50.0, 100.0),
            universe_returns: vec![0.1; 500],
        };
        let output = screen_stock(input).unwrap();
        assert_eq!(output.ticker, "AAPL");
        assert!(output.composite_score >= 0.0 && output.composite_score <= 100.0);
        assert!(!output.signal_strength.is_empty());
    }

    #[test]
    fn screen_stock_insufficient_data_returns_error() {
        let input = ScreenStockInput {
            ticker: "AAPL".into(),
            candles: make_candles(100, 50.0, 100.0),
            universe_returns: vec![],
        };
        let err = screen_stock(input).unwrap_err();
        assert!(err.contains("Screening failed"));
    }

    #[test]
    fn score_breakdown_computes_correctly() {
        let output = score_breakdown(ScoreBreakdownInput {
            conditions_met: 8,
            adx: 50.0,
            rs_percentile: 90.0,
            volume_zscore: 2.0,
            sentiment_score: 0.0,
            article_count: 0,
        });
        assert!(output.composite > 0.0);
        assert!(output.breakdown.contains("Technical:"));
        assert!(output.breakdown.contains("Volume:"));
    }

    #[test]
    fn masked_config_redacts_api_key() {
        let config = crate::config::AppConfig {
            finnhub: crate::config::FinnhubConfig {
                api_key: "sk_live_abc123xyz789".into(),
                requests_per_minute: 60,
                retry_delay_secs: 5,
            },
            telegram: None,
            discord: None,
            fred: None,
            sentiment: None,
            backtest: None,
            alerts: Default::default(),
            storage: Default::default(),
            plugins: Default::default(),
            cache: Default::default(),
            universe: vec![],
            edgar: None,
            profile: None,
        };
        let output = masked_config(&config);
        assert_eq!(output.finnhub_api_key, "sk...89");
        assert!(!output.finnhub_api_key.contains("abc123"));
    }

    #[test]
    fn masked_config_short_key_fully_redacted() {
        let config = crate::config::AppConfig {
            finnhub: crate::config::FinnhubConfig {
                api_key: "abc".into(),
                requests_per_minute: 60,
                retry_delay_secs: 5,
            },
            telegram: None,
            discord: None,
            fred: None,
            sentiment: None,
            backtest: None,
            alerts: Default::default(),
            storage: Default::default(),
            plugins: Default::default(),
            cache: Default::default(),
            universe: vec![],
            edgar: None,
            profile: None,
        };
        let output = masked_config(&config);
        assert_eq!(output.finnhub_api_key, "****");
    }

    #[test]
    fn format_regime_bull() {
        let state = RegimeState {
            date: NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            spy_close: 450.0,
            spy_sma200: 430.0,
            breadth_pct: 65.0,
            regime: MarketRegime::Bull,
        };
        let output = format_regime(&state);
        assert_eq!(output.regime, "BULL");
        assert!(output.description.contains("bullish"));
    }

    #[test]
    fn list_providers_empty_registry() {
        let reg = crate::plugins::Registry::new();
        let output = list_providers(&reg);
        assert!(output.providers.is_empty());
        assert!(output.active_provider.is_none());
    }

    #[test]
    fn format_regime_sideways() {
        let state = RegimeState {
            date: NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            spy_close: 440.0,
            spy_sma200: 435.0,
            breadth_pct: 48.0,
            regime: MarketRegime::Sideways,
        };
        let output = format_regime(&state);
        assert_eq!(output.regime, "SIDEWAYS");
        assert!(output.description.contains("sideways"));
        assert!(output.description.contains("caution"));
    }

    #[test]
    fn format_regime_bear() {
        let state = RegimeState {
            date: NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            spy_close: 400.0,
            spy_sma200: 440.0,
            breadth_pct: 30.0,
            regime: MarketRegime::Bear,
        };
        let output = format_regime(&state);
        assert_eq!(output.regime, "BEAR");
        assert!(output.description.contains("bearish"));
        assert!(output.description.contains("skepticism"));
    }

    #[test]
    fn score_breakdown_all_zero_inputs() {
        // conditions=0, adx=0, rs=0, vol_zscore=0, sentiment=0, articles=0
        // technical_score: 0 (no conditions, no ADX bonus, no RS bonus)
        // volume_score(z=0): zero-centered — average volume scores 0
        // sentiment_score(score=0, articles=0): confidence=0 → 0
        // composite = 0 + 0 + 0 = 0
        let output = score_breakdown(ScoreBreakdownInput {
            conditions_met: 0,
            adx: 0.0,
            rs_percentile: 0.0,
            volume_zscore: 0.0,
            sentiment_score: 0.0,
            article_count: 0,
        });
        assert_eq!(output.technical, 0.0);
        assert_eq!(output.volume, 0.0);
        assert_eq!(output.sentiment, 0.0);
        assert_eq!(output.composite, 0.0);
    }
}
