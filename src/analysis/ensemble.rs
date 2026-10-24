//! Multi-strategy ensemble scoring.
//!
//! Combines results from multiple screeners to identify higher-conviction signals.
//! A stock passing 2+ strategies is tagged as a consensus pick with an averaged
//! composite score and conviction tier.

use crate::analysis::scorer::Grade;
use crate::analysis::screener::ScreenResult;

// ─── Types ──────────────────────────────────────────────────────────────────

/// Conviction tier based on number of agreeing strategies.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum ConvictionTier {
    /// Only 1 strategy passed — no ensemble signal
    Standard,
    /// 2 strategies agree
    High,
    /// 3+ strategies agree
    Highest,
}

impl std::fmt::Display for ConvictionTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConvictionTier::Standard => write!(f, "Standard"),
            ConvictionTier::High => write!(f, "High"),
            ConvictionTier::Highest => write!(f, "Highest"),
        }
    }
}

/// Result of ensemble screening across multiple strategies.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EnsembleResult {
    pub ticker: String,
    /// Names of strategies that passed
    pub strategies_passing: Vec<String>,
    /// Average composite score across passing strategies
    pub avg_composite: f64,
    /// Conviction tier based on strategy count
    pub conviction_tier: ConvictionTier,
    /// Consensus grade (from averaged composite)
    pub consensus_grade: Grade,
}

// ─── Core Function ──────────────────────────────────────────────────────────

/// Combine screen results from multiple strategies for a single ticker.
///
/// Each entry is `(strategy_name, Option<ScreenResult>)`. Only results where
/// the stock passed the screen (Some) are included.
///
/// Returns `None` if fewer than 2 strategies agree (minimum for consensus).
pub fn ensemble_combine(
    ticker: &str,
    results: &[(&str, Option<&ScreenResult>)],
    universe_scores: &[f64],
) -> Option<EnsembleResult> {
    let passing: Vec<(&str, &ScreenResult)> = results
        .iter()
        .filter_map(|(name, result)| result.map(|r| (*name, r)))
        .collect();

    if passing.len() < 2 {
        return None;
    }

    let conviction = match passing.len() {
        2 => ConvictionTier::High,
        _ => ConvictionTier::Highest,
    };

    let avg_composite =
        passing.iter().map(|(_, r)| r.composite_score).sum::<f64>() / passing.len() as f64;

    let strategies = passing.iter().map(|(name, _)| name.to_string()).collect();

    let consensus_grade = grade_from_score(avg_composite, universe_scores);

    Some(EnsembleResult {
        ticker: ticker.to_string(),
        strategies_passing: strategies,
        avg_composite,
        conviction_tier: conviction,
        consensus_grade,
    })
}

/// Derive a grade from a composite score relative to the universe.
fn grade_from_score(score: f64, universe_scores: &[f64]) -> Grade {
    if universe_scores.is_empty() {
        return Grade::C;
    }
    let below = universe_scores.iter().filter(|&&s| s < score).count();
    let percentile = 100.0 * below as f64 / universe_scores.len() as f64;

    if percentile >= 95.0 {
        Grade::APlus
    } else if percentile >= 85.0 {
        Grade::A
    } else if percentile >= 65.0 {
        Grade::B
    } else if percentile >= 40.0 {
        Grade::C
    } else if percentile >= 25.0 {
        Grade::D
    } else {
        Grade::F
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::indicators::LiquidityClass;
    use crate::analysis::indicators::VcpResult;
    use crate::data::models::MarketRegime;

    fn make_screen_result(ticker: &str, composite: f64) -> ScreenResult {
        ScreenResult {
            ticker: ticker.to_string(),
            minervini_pass: true,
            conditions_met: 6,
            adx: 35.0,
            rs_percentile: 80.0,
            volume_zscore: 1.5,
            technical_score: composite * 0.55,
            volume_score: composite * 0.30,
            sentiment_score: 0.0,
            composite_score: composite,
            grade: Grade::A,
            close_price: 100.0,
            pct_from_52w_high: -5.0,
            atr_value: 3.0,
            risk_reward: 2.5,
            liquidity: LiquidityClass::High,
            weighted_rs: 85.0,
            rs_momentum: Some(5.0),
            vcp: Some(VcpResult {
                vcp_detected: false,
                base_count: 1,
                base_depth_pct: 10.0,
                contraction_ratio: 0.8,
                pivot: 105.0,
                pivot_proximity_pct: -4.8,
            }),
            regime: MarketRegime::Bull,
        }
    }

    #[test]
    fn returns_none_with_single_strategy() {
        let r = make_screen_result("AAPL", 80.0);
        let results: Vec<(&str, Option<&ScreenResult>)> =
            vec![("Minervini", Some(&r)), ("Breakout", None)];
        assert!(ensemble_combine("AAPL", &results, &[]).is_none());
    }

    #[test]
    fn returns_high_conviction_for_2_strategies() {
        let r1 = make_screen_result("AAPL", 80.0);
        let r2 = make_screen_result("AAPL", 75.0);
        let results: Vec<(&str, Option<&ScreenResult>)> = vec![
            ("Minervini", Some(&r1)),
            ("TrendStrength", Some(&r2)),
            ("Breakout", None),
        ];
        let ensemble = ensemble_combine("AAPL", &results, &[]).unwrap();
        assert_eq!(ensemble.conviction_tier, ConvictionTier::High);
        assert_eq!(ensemble.strategies_passing.len(), 2);
        assert!((ensemble.avg_composite - 77.5).abs() < 0.01);
    }

    #[test]
    fn returns_highest_conviction_for_3_strategies() {
        let r1 = make_screen_result("AAPL", 80.0);
        let r2 = make_screen_result("AAPL", 75.0);
        let r3 = make_screen_result("AAPL", 85.0);
        let results: Vec<(&str, Option<&ScreenResult>)> = vec![
            ("Minervini", Some(&r1)),
            ("TrendStrength", Some(&r2)),
            ("Breakout", Some(&r3)),
        ];
        let ensemble = ensemble_combine("AAPL", &results, &[]).unwrap();
        assert_eq!(ensemble.conviction_tier, ConvictionTier::Highest);
        assert_eq!(ensemble.strategies_passing.len(), 3);
        assert!((ensemble.avg_composite - 80.0).abs() < 0.01);
    }

    #[test]
    fn returns_none_with_no_passing() {
        let results: Vec<(&str, Option<&ScreenResult>)> =
            vec![("Minervini", None), ("Breakout", None)];
        assert!(ensemble_combine("AAPL", &results, &[]).is_none());
    }

    #[test]
    fn grade_uses_universe_percentile() {
        let r1 = make_screen_result("AAPL", 90.0);
        let r2 = make_screen_result("AAPL", 85.0);
        let results: Vec<(&str, Option<&ScreenResult>)> =
            vec![("Minervini", Some(&r1)), ("TrendStrength", Some(&r2))];
        // avg = 87.5. Universe: 50 stocks scoring 0-99
        let universe: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let ensemble = ensemble_combine("AAPL", &results, &universe).unwrap();
        assert_eq!(ensemble.consensus_grade, Grade::A);
    }

    #[test]
    fn strategies_names_preserved() {
        let r1 = make_screen_result("AAPL", 80.0);
        let r2 = make_screen_result("AAPL", 70.0);
        let results: Vec<(&str, Option<&ScreenResult>)> =
            vec![("Minervini", Some(&r1)), ("MeanReversion", Some(&r2))];
        let ensemble = ensemble_combine("AAPL", &results, &[]).unwrap();
        assert!(ensemble
            .strategies_passing
            .contains(&"Minervini".to_string()));
        assert!(ensemble
            .strategies_passing
            .contains(&"MeanReversion".to_string()));
    }
}
