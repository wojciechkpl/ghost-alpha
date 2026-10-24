use crate::data::models::MarketRegime;
use crate::error::AnalysisError;

// ─── Score Components ────────────────────────────────────────────────────────

/// Inputs for the technical sub-score (Minervini + ADX + RS).
#[derive(Debug, Clone)]
pub struct TechnicalInputs {
    /// Number of conditions satisfied (0–max_conditions)
    pub conditions_met: u8,
    /// Maximum conditions for this screener (8 for Minervini, 5 for others).
    /// Phase 5: normalizes scoring so all screeners can reach 100% of the
    /// technical base allocation.
    pub max_conditions: u8,
    /// Latest ADX value
    pub adx: f64,
    /// Relative Strength percentile (0–100)
    pub rs_percentile: f64,
}

/// Inputs for the volume sub-score.
#[derive(Debug, Clone)]
pub struct VolumeInputs {
    /// Directional volume Z-score (negative = distribution)
    pub zscore: f64,
}

/// Inputs for the sentiment sub-score.
#[derive(Debug, Clone)]
pub struct SentimentInputs {
    /// Compound sentiment score in [–1, +1]
    pub score: f64,
    /// Number of articles analysed (lower weight if few articles)
    pub article_count: usize,
}

// ─── Grades ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub enum Grade {
    F,
    D,
    C,
    B,
    A,
    APlus,
}

impl std::fmt::Display for Grade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Grade::APlus => write!(f, "A+"),
            Grade::A => write!(f, "A"),
            Grade::B => write!(f, "B"),
            Grade::C => write!(f, "C"),
            Grade::D => write!(f, "D"),
            Grade::F => write!(f, "F"),
        }
    }
}

// ─── Composite Score ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct CompositeScore {
    pub composite: f64,
    pub technical: f64,
    pub volume: f64,
    pub sentiment: f64,
    pub grade: Grade,
}

/// Compute the 0-100 composite score.
///
/// Weights: Technical 55%, Volume 30%, Sentiment 15%.
pub fn composite_score(
    tech: &TechnicalInputs,
    vol: &VolumeInputs,
    sent: &SentimentInputs,
) -> CompositeScore {
    let technical = technical_score(tech);
    let volume = volume_score(vol);
    let sentiment = sentiment_score(sent);
    let composite = (technical + volume + sentiment).clamp(0.0, 100.0);
    CompositeScore {
        composite,
        technical,
        volume,
        sentiment,
        grade: Grade::F, // assign_grades() sets this after universe ranking
    }
}

/// Technical sub-score: 0–55 points.
/// - Base: conditions_met/max_conditions × 35
/// - ADX bonus: up to 10 pts (ADX 25–50 range)
/// - RS bonus: up to 10 pts (RS 70–90+ range)
pub fn technical_score(inputs: &TechnicalInputs) -> f64 {
    let denom = inputs.max_conditions.max(1) as f64;
    let base = (inputs.conditions_met as f64 / denom) * 35.0;

    let adx_bonus = if inputs.adx >= 50.0 {
        10.0
    } else if inputs.adx >= 25.0 {
        ((inputs.adx - 25.0) / 25.0) * 10.0
    } else {
        0.0
    };

    let rs_bonus = if inputs.rs_percentile >= 90.0 {
        10.0
    } else if inputs.rs_percentile >= 70.0 {
        ((inputs.rs_percentile - 70.0) / 20.0) * 10.0
    } else {
        0.0
    };

    (base + adx_bonus + rs_bonus).clamp(0.0, 55.0)
}

/// Volume sub-score: 0–30 points.
///
/// Only rewards above-average (accumulation) volume. Distribution days (z < 0)
/// score 0. A z-score of 0 (perfectly average volume) produces 0, not 15.
///
/// Phase 5 fix: previously used `((z + 3) / 6) * 30` which gave a free +15
/// bias to every stock. Now uses `(z.clamp(0, 3) / 3) * 30`.
pub fn volume_score(inputs: &VolumeInputs) -> f64 {
    let z = inputs.zscore.clamp(0.0, 3.0);
    (z / 3.0 * 30.0).clamp(0.0, 30.0)
}

/// Sentiment sub-score: 0–15 points.
/// Score maps –1..+1 → 0..15. Confidence-weighted by article count.
pub fn sentiment_score(inputs: &SentimentInputs) -> f64 {
    let raw = ((inputs.score + 1.0) / 2.0 * 15.0).clamp(0.0, 15.0);
    // Reduce weight when few articles (< 3 = half weight, 0 = no weight)
    let confidence = match inputs.article_count {
        0 => 0.0,
        1 => 0.33,
        2 => 0.67,
        _ => 1.0,
    };
    raw * confidence
}

// ─── Regime-Adaptive Weights ────────────────────────────────────────────────

/// Scoring weights that adapt to the current market regime.
///
/// In bear markets, technical quality dominates (defensive).
/// In bull markets, volume and sentiment get more weight (risk-on).
#[derive(Debug, Clone, PartialEq)]
pub struct RegimeWeights {
    pub technical: f64,
    pub volume: f64,
    pub sentiment: f64,
}

impl RegimeWeights {
    /// Return the weight profile for a given regime.
    pub fn for_regime(regime: &MarketRegime) -> Self {
        match regime {
            MarketRegime::Bull => Self {
                technical: 55.0,
                volume: 30.0,
                sentiment: 15.0,
            },
            MarketRegime::Sideways => Self {
                technical: 60.0,
                volume: 20.0,
                sentiment: 20.0,
            },
            MarketRegime::Bear => Self {
                technical: 70.0,
                volume: 10.0,
                sentiment: 20.0,
            },
        }
    }

    /// Sum of all weights (should always be 100).
    pub fn sum(&self) -> f64 {
        self.technical + self.volume + self.sentiment
    }

    /// When no sentiment data is available (article_count == 0), redistribute
    /// the sentiment weight: 70% to technical, 30% to volume.
    ///
    /// Returns unmodified weights if articles > 0.
    pub fn redistribute_if_no_sentiment(&self, article_count: usize) -> Self {
        if article_count > 0 {
            return self.clone();
        }
        Self {
            technical: self.technical + self.sentiment * 0.7,
            volume: self.volume + self.sentiment * 0.3,
            sentiment: 0.0,
        }
    }
}

/// Technical sub-score scaled to a custom max (instead of hardcoded 55).
pub fn technical_score_weighted(inputs: &TechnicalInputs, max: f64) -> f64 {
    // Same internal formula, but rescale from 0–55 to 0–max
    let raw = technical_score(inputs); // 0–55
    (raw / 55.0 * max).clamp(0.0, max)
}

/// Volume sub-score scaled to a custom max (instead of hardcoded 30).
pub fn volume_score_weighted(inputs: &VolumeInputs, max: f64) -> f64 {
    let raw = volume_score(inputs); // 0–30
    (raw / 30.0 * max).clamp(0.0, max)
}

/// Sentiment sub-score scaled to a custom max (instead of hardcoded 15).
pub fn sentiment_score_weighted(inputs: &SentimentInputs, max: f64) -> f64 {
    let raw = sentiment_score(inputs); // 0–15
    (raw / 15.0 * max).clamp(0.0, max)
}

/// Compute a regime-adaptive composite score.
///
/// Uses the provided `RegimeWeights` to scale each sub-score to its
/// regime-appropriate maximum, then sums and clamps to 0–100.
/// When no sentiment data is available, redistributes sentiment weight
/// to technical (70%) and volume (30%).
pub fn composite_score_weighted(
    tech: &TechnicalInputs,
    vol: &VolumeInputs,
    sent: &SentimentInputs,
    weights: &RegimeWeights,
) -> CompositeScore {
    let w = weights.redistribute_if_no_sentiment(sent.article_count);
    let technical = technical_score_weighted(tech, w.technical);
    let volume = volume_score_weighted(vol, w.volume);
    let sentiment = sentiment_score_weighted(sent, w.sentiment);
    let composite = (technical + volume + sentiment).clamp(0.0, 100.0);
    CompositeScore {
        composite,
        technical,
        volume,
        sentiment,
        grade: Grade::F, // assign_grades() sets this after universe ranking
    }
}

/// Assign percentile-based grades to a slice of composite scores (in place).
/// Grades are relative to the current screening universe, not fixed thresholds.
///
/// - A+: top 5%
/// - A:  top 15%
/// - B:  top 35%
/// - C:  top 60%
/// - D:  top 75%
/// - F:  bottom 25%
pub fn assign_grades(scores: &mut [CompositeScore]) -> Result<(), AnalysisError> {
    if scores.is_empty() {
        return Ok(());
    }
    // Sort descending by composite to compute rank
    let mut indexed: Vec<(usize, f64)> = scores
        .iter()
        .enumerate()
        .map(|(i, s)| (i, s.composite))
        .collect();
    indexed.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let n = indexed.len() as f64;
    for (rank, (original_idx, _)) in indexed.iter().enumerate() {
        let percentile = 100.0 * (1.0 - rank as f64 / n);
        scores[*original_idx].grade = grade_by_percentile_value(percentile);
    }
    Ok(())
}

fn grade_by_percentile_value(percentile: f64) -> Grade {
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

/// Convenience: grade a single score relative to a universe of raw composite values.
pub fn grade_by_percentile(this_score: f64, universe_scores: &[f64]) -> Grade {
    if universe_scores.is_empty() {
        return Grade::C;
    }
    let below = universe_scores.iter().filter(|&&s| s < this_score).count();
    let percentile = 100.0 * below as f64 / universe_scores.len() as f64;
    grade_by_percentile_value(percentile)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    // ── technical_score ──────────────────────────────────────────────────────

    #[test]
    fn technical_score_perfect_inputs_at_55() {
        let score = technical_score(&TechnicalInputs {
            conditions_met: 8,
            max_conditions: 8,
            adx: 60.0,
            rs_percentile: 99.0,
        });
        assert_relative_eq!(score, 55.0, epsilon = 0.001);
    }

    #[test]
    fn technical_score_zero_conditions_zero_adx_zero_rs() {
        let score = technical_score(&TechnicalInputs {
            conditions_met: 0,
            max_conditions: 8,
            adx: 0.0,
            rs_percentile: 0.0,
        });
        assert_relative_eq!(score, 0.0, epsilon = 0.001);
    }

    #[test]
    fn technical_score_8_conditions_no_bonuses() {
        let score = technical_score(&TechnicalInputs {
            conditions_met: 8,
            max_conditions: 8,
            adx: 10.0,
            rs_percentile: 50.0,
        });
        assert_relative_eq!(score, 35.0, epsilon = 0.001);
    }

    #[test]
    fn technical_score_4_of_8_conditions_is_half_base() {
        let score = technical_score(&TechnicalInputs {
            conditions_met: 4,
            max_conditions: 8,
            adx: 0.0,
            rs_percentile: 0.0,
        });
        assert_relative_eq!(score, 17.5, epsilon = 0.001);
    }

    #[test]
    fn technical_score_5_of_5_conditions_is_full_base() {
        // Phase 5: Breakout/MeanReversion/TrendStrength have 5 conditions.
        // 5/5 should produce the same base (35) as 8/8.
        let score = technical_score(&TechnicalInputs {
            conditions_met: 5,
            max_conditions: 5,
            adx: 0.0,
            rs_percentile: 0.0,
        });
        assert_relative_eq!(score, 35.0, epsilon = 0.001);
    }

    #[test]
    fn technical_score_3_of_5_is_21() {
        // 3/5 = 60% → 60% of 35 = 21
        let score = technical_score(&TechnicalInputs {
            conditions_met: 3,
            max_conditions: 5,
            adx: 0.0,
            rs_percentile: 0.0,
        });
        assert_relative_eq!(score, 21.0, epsilon = 0.001);
    }

    #[test]
    fn technical_score_adx_bonus_at_threshold() {
        // ADX = 25 → bonus starts; ADX = 50 → full 10 pts
        let at_25 = technical_score(&TechnicalInputs {
            conditions_met: 0,
            max_conditions: 8,
            adx: 25.0,
            rs_percentile: 0.0,
        });
        assert_relative_eq!(at_25, 0.0, epsilon = 0.001);

        let at_50 = technical_score(&TechnicalInputs {
            conditions_met: 0,
            max_conditions: 8,
            adx: 50.0,
            rs_percentile: 0.0,
        });
        assert_relative_eq!(at_50, 10.0, epsilon = 0.001);
    }

    #[test]
    fn technical_score_rs_bonus_at_threshold() {
        let at_70 = technical_score(&TechnicalInputs {
            conditions_met: 0,
            max_conditions: 8,
            adx: 0.0,
            rs_percentile: 70.0,
        });
        assert_relative_eq!(at_70, 0.0, epsilon = 0.001);

        let at_90 = technical_score(&TechnicalInputs {
            conditions_met: 0,
            max_conditions: 8,
            adx: 0.0,
            rs_percentile: 90.0,
        });
        assert_relative_eq!(at_90, 10.0, epsilon = 0.001);
    }

    #[test]
    fn technical_score_capped_at_55() {
        let score = technical_score(&TechnicalInputs {
            conditions_met: 8,
            max_conditions: 8,
            adx: 100.0,
            rs_percentile: 100.0,
        });
        assert_relative_eq!(score, 55.0, epsilon = 0.001);
    }

    // ── volume_score ─────────────────────────────────────────────────────────

    #[test]
    fn volume_score_z_plus_3_is_30() {
        let score = volume_score(&VolumeInputs { zscore: 3.0 });
        assert_relative_eq!(score, 30.0, epsilon = 0.001);
    }

    #[test]
    fn volume_score_z_zero_is_zero() {
        // Phase 5 fix: z=0 (average volume) must produce 0, not 15.
        // Only above-average (accumulation) volume should be rewarded.
        let score = volume_score(&VolumeInputs { zscore: 0.0 });
        assert_relative_eq!(score, 0.0, epsilon = 0.001);
    }

    #[test]
    fn volume_score_z_minus_3_is_0() {
        let score = volume_score(&VolumeInputs { zscore: -3.0 });
        assert_relative_eq!(score, 0.0, epsilon = 0.001);
    }

    #[test]
    fn volume_score_negative_z_is_zero() {
        // Distribution days (negative z) should produce 0 points
        for z in [-0.5, -1.0, -2.0, -3.0, -5.0] {
            let s = volume_score(&VolumeInputs { zscore: z });
            assert!(s.abs() < 0.001, "z={z}: expected 0, got {s}");
        }
    }

    #[test]
    fn volume_score_positive_z_scales_linearly() {
        // z=1.5 should be halfway between 0 and 30 → 15
        let score = volume_score(&VolumeInputs { zscore: 1.5 });
        assert_relative_eq!(score, 15.0, epsilon = 0.001);
    }

    #[test]
    fn volume_score_bounded_0_to_30() {
        for z in [-10.0, -5.0, 0.0, 5.0, 10.0] {
            let s = volume_score(&VolumeInputs { zscore: z });
            assert!((0.0..=30.0).contains(&s), "z={z}: score out of range: {s}");
        }
    }

    // ── sentiment_score ───────────────────────────────────────────────────────

    #[test]
    fn sentiment_score_positive_1_with_articles_is_15() {
        let score = sentiment_score(&SentimentInputs {
            score: 1.0,
            article_count: 5,
        });
        assert_relative_eq!(score, 15.0, epsilon = 0.001);
    }

    #[test]
    fn sentiment_score_negative_1_is_0() {
        let score = sentiment_score(&SentimentInputs {
            score: -1.0,
            article_count: 5,
        });
        assert_relative_eq!(score, 0.0, epsilon = 0.001);
    }

    #[test]
    fn sentiment_score_neutral_is_7_5() {
        let score = sentiment_score(&SentimentInputs {
            score: 0.0,
            article_count: 5,
        });
        assert_relative_eq!(score, 7.5, epsilon = 0.001);
    }

    #[test]
    fn sentiment_score_zero_articles_is_zero() {
        let score = sentiment_score(&SentimentInputs {
            score: 1.0,
            article_count: 0,
        });
        assert_relative_eq!(score, 0.0, epsilon = 0.001);
    }

    #[test]
    fn sentiment_score_one_article_reduced() {
        let full = sentiment_score(&SentimentInputs {
            score: 1.0,
            article_count: 5,
        });
        let one = sentiment_score(&SentimentInputs {
            score: 1.0,
            article_count: 1,
        });
        assert!(
            one < full,
            "1 article should produce lower score than 5 articles"
        );
    }

    // ── composite_score ───────────────────────────────────────────────────────

    #[test]
    fn composite_score_bounded_0_to_100() {
        let score = composite_score(
            &TechnicalInputs {
                conditions_met: 8,
                max_conditions: 8,
                adx: 50.0,
                rs_percentile: 90.0,
            },
            &VolumeInputs { zscore: 3.5 },
            &SentimentInputs {
                score: 0.9,
                article_count: 10,
            },
        );
        assert!(
            score.composite >= 0.0 && score.composite <= 100.0,
            "composite out of range: {}",
            score.composite
        );
    }

    #[test]
    fn composite_score_all_zeros_is_zero() {
        // Phase 5 fix: with zero-centered volume scoring, all-zero inputs = 0
        let score = composite_score(
            &TechnicalInputs {
                conditions_met: 0,
                max_conditions: 8,
                adx: 0.0,
                rs_percentile: 0.0,
            },
            &VolumeInputs { zscore: 0.0 },
            &SentimentInputs {
                score: -1.0,
                article_count: 5,
            },
        );
        assert_relative_eq!(score.composite, 0.0, epsilon = 0.001);
    }

    #[test]
    fn composite_score_high_inputs_above_80() {
        let score = composite_score(
            &TechnicalInputs {
                conditions_met: 8,
                max_conditions: 8,
                adx: 40.0,
                rs_percentile: 90.0,
            },
            &VolumeInputs { zscore: 2.5 },
            &SentimentInputs {
                score: 0.8,
                article_count: 5,
            },
        );
        assert!(
            score.composite > 80.0,
            "Expected > 80 for high-quality inputs, got {}",
            score.composite
        );
    }

    // ── assign_grades ─────────────────────────────────────────────────────────

    #[test]
    fn assign_grades_top_score_gets_a_plus() {
        let mut scores: Vec<CompositeScore> = (0..=100)
            .map(|i| CompositeScore {
                composite: i as f64,
                technical: 0.0,
                volume: 0.0,
                sentiment: 0.0,
                grade: Grade::F,
            })
            .collect();
        assign_grades(&mut scores).unwrap();
        assert_eq!(scores[100].grade, Grade::APlus);
    }

    #[test]
    fn assign_grades_bottom_score_gets_f() {
        let mut scores: Vec<CompositeScore> = (0..=100)
            .map(|i| CompositeScore {
                composite: i as f64,
                technical: 0.0,
                volume: 0.0,
                sentiment: 0.0,
                grade: Grade::F,
            })
            .collect();
        assign_grades(&mut scores).unwrap();
        assert_eq!(scores[0].grade, Grade::F);
    }

    #[test]
    fn assign_grades_empty_slice_no_panic() {
        let mut scores: Vec<CompositeScore> = vec![];
        assign_grades(&mut scores).unwrap();
    }

    // ── regime weights ────────────────────────────────────────────────────────

    #[test]
    fn regime_weights_bull_is_55_30_15() {
        let w = RegimeWeights::for_regime(&MarketRegime::Bull);
        assert_relative_eq!(w.technical, 55.0);
        assert_relative_eq!(w.volume, 30.0);
        assert_relative_eq!(w.sentiment, 15.0);
    }

    #[test]
    fn regime_weights_sideways_is_60_20_20() {
        let w = RegimeWeights::for_regime(&MarketRegime::Sideways);
        assert_relative_eq!(w.technical, 60.0);
        assert_relative_eq!(w.volume, 20.0);
        assert_relative_eq!(w.sentiment, 20.0);
    }

    #[test]
    fn regime_weights_bear_is_70_10_20() {
        let w = RegimeWeights::for_regime(&MarketRegime::Bear);
        assert_relative_eq!(w.technical, 70.0);
        assert_relative_eq!(w.volume, 10.0);
        assert_relative_eq!(w.sentiment, 20.0);
    }

    #[test]
    fn regime_weights_all_sum_to_100() {
        for regime in &[
            MarketRegime::Bull,
            MarketRegime::Sideways,
            MarketRegime::Bear,
        ] {
            let w = RegimeWeights::for_regime(regime);
            assert_relative_eq!(w.sum(), 100.0, epsilon = 1e-10);
        }
    }

    #[test]
    fn technical_score_weighted_scales_to_custom_max() {
        let inputs = TechnicalInputs {
            conditions_met: 8,
            max_conditions: 8,
            adx: 50.0,
            rs_percentile: 90.0,
        };
        // Perfect score at max=55 → 55.0, at max=70 → 70.0
        let at_55 = technical_score_weighted(&inputs, 55.0);
        let at_70 = technical_score_weighted(&inputs, 70.0);
        assert_relative_eq!(at_55, 55.0, epsilon = 0.01);
        assert_relative_eq!(at_70, 70.0, epsilon = 0.01);
    }

    #[test]
    fn composite_weighted_bounded_0_100() {
        let weights = RegimeWeights::for_regime(&MarketRegime::Bull);
        let score = composite_score_weighted(
            &TechnicalInputs {
                conditions_met: 8,
                max_conditions: 8,
                adx: 50.0,
                rs_percentile: 90.0,
            },
            &VolumeInputs { zscore: 3.5 },
            &SentimentInputs {
                score: 0.9,
                article_count: 10,
            },
            &weights,
        );
        assert!(score.composite >= 0.0 && score.composite <= 100.0);
    }

    #[test]
    fn bear_regime_technical_dominates() {
        let tech = TechnicalInputs {
            conditions_met: 8,
            max_conditions: 8,
            adx: 50.0,
            rs_percentile: 90.0,
        };
        let vol = VolumeInputs { zscore: 3.0 };
        let sent = SentimentInputs {
            score: 1.0,
            article_count: 5,
        };

        let bear_score = composite_score_weighted(
            &tech,
            &vol,
            &sent,
            &RegimeWeights::for_regime(&MarketRegime::Bear),
        );
        // In bear regime, technical = 70% weight vs volume = 10%
        assert!(
            bear_score.technical > bear_score.volume,
            "Bear regime should have technical > volume, got tech={:.1} vol={:.1}",
            bear_score.technical,
            bear_score.volume
        );
    }

    #[test]
    fn backward_compat_with_bull_weights() {
        // composite_score() uses hardcoded 55/30/15 = Bull weights
        let tech = TechnicalInputs {
            conditions_met: 6,
            max_conditions: 8,
            adx: 35.0,
            rs_percentile: 80.0,
        };
        let vol = VolumeInputs { zscore: 1.5 };
        let sent = SentimentInputs {
            score: 0.5,
            article_count: 3,
        };

        let old = composite_score(&tech, &vol, &sent);
        let new = composite_score_weighted(
            &tech,
            &vol,
            &sent,
            &RegimeWeights::for_regime(&MarketRegime::Bull),
        );
        assert_relative_eq!(old.composite, new.composite, epsilon = 0.1);
    }

    #[test]
    fn different_regimes_produce_different_scores() {
        // Use inputs where technical and volume sub-scores differ enough
        // that regime weight shifts produce visibly different composites.
        let tech = TechnicalInputs {
            conditions_met: 4,
            max_conditions: 8,
            adx: 35.0,
            rs_percentile: 80.0,
        };
        let vol = VolumeInputs { zscore: 3.0 };
        let sent = SentimentInputs {
            score: 0.8,
            article_count: 5,
        };

        let bull = composite_score_weighted(
            &tech,
            &vol,
            &sent,
            &RegimeWeights::for_regime(&MarketRegime::Bull),
        );
        let bear = composite_score_weighted(
            &tech,
            &vol,
            &sent,
            &RegimeWeights::for_regime(&MarketRegime::Bear),
        );
        assert!(
            (bull.composite - bear.composite).abs() > 0.1,
            "Different regimes should produce different scores: bull={:.1} bear={:.1}",
            bull.composite,
            bear.composite
        );
    }

    // ── sentiment weight redistribution ────────────────────────────────────────

    #[test]
    fn redistribute_weights_when_no_sentiment_data() {
        // When article_count=0, sentiment weight should go to tech (70%) and volume (30%)
        let weights = RegimeWeights::for_regime(&MarketRegime::Bull); // 55/30/15
        let adjusted = weights.redistribute_if_no_sentiment(0);
        // sentiment=15 → tech gets 15*0.7=10.5, volume gets 15*0.3=4.5
        assert_relative_eq!(adjusted.technical, 65.5, epsilon = 0.001);
        assert_relative_eq!(adjusted.volume, 34.5, epsilon = 0.001);
        assert_relative_eq!(adjusted.sentiment, 0.0, epsilon = 0.001);
        assert_relative_eq!(adjusted.sum(), 100.0, epsilon = 0.001);
    }

    #[test]
    fn no_redistribution_when_articles_present() {
        let weights = RegimeWeights::for_regime(&MarketRegime::Bull);
        let adjusted = weights.redistribute_if_no_sentiment(3);
        assert_eq!(adjusted, weights);
    }

    #[test]
    fn redistribute_bear_regime_weights() {
        let weights = RegimeWeights::for_regime(&MarketRegime::Bear); // 70/10/20
        let adjusted = weights.redistribute_if_no_sentiment(0);
        // sentiment=20 → tech gets 20*0.7=14, volume gets 20*0.3=6
        assert_relative_eq!(adjusted.technical, 84.0, epsilon = 0.001);
        assert_relative_eq!(adjusted.volume, 16.0, epsilon = 0.001);
        assert_relative_eq!(adjusted.sentiment, 0.0, epsilon = 0.001);
    }

    // ── grade_by_percentile ───────────────────────────────────────────────────

    #[test]
    fn grade_by_percentile_a_plus_above_95th() {
        let universe: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let grade = grade_by_percentile(99.0, &universe);
        assert_eq!(grade, Grade::APlus);
    }

    #[test]
    fn grade_by_percentile_f_below_25th() {
        let universe: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let grade = grade_by_percentile(10.0, &universe);
        assert_eq!(grade, Grade::F);
    }

    #[test]
    fn grade_by_percentile_boundaries() {
        let universe: Vec<f64> = (0..100).map(|i| i as f64).collect();
        // 96th percentile → A+
        assert_eq!(grade_by_percentile(96.0, &universe), Grade::APlus);
        // 88th percentile → A
        assert_eq!(grade_by_percentile(88.0, &universe), Grade::A);
        // 70th percentile → B
        assert_eq!(grade_by_percentile(70.0, &universe), Grade::B);
        // 50th percentile → C
        assert_eq!(grade_by_percentile(50.0, &universe), Grade::C);
        // 30th percentile → D
        assert_eq!(grade_by_percentile(30.0, &universe), Grade::D);
    }
}
