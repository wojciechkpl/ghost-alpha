/// FinBERT sentiment scoring via ONNX Runtime.
/// Stub — full implementation in Phase 3.
use chrono::{DateTime, Utc};

use crate::error::AnalysisError;

pub struct SentimentResult {
    pub positive: f64,
    pub negative: f64,
    pub neutral: f64,
    /// compound = positive - negative, range [-1, +1]
    pub compound: f64,
    pub article_count: usize,
}

/// Per-article sentiment with timestamp for time-decay weighting.
pub struct ArticleSentiment {
    pub positive: f64,
    pub negative: f64,
    pub neutral: f64,
    pub compound: f64,
    pub published_at: DateTime<Utc>,
}

/// Compute compound sentiment from positive/negative probabilities.
/// Returns `(positive - negative)` clamped to `[-1.0, 1.0]`.
pub fn compound_from_probs(positive: f64, negative: f64, _neutral: f64) -> f64 {
    (positive - negative).clamp(-1.0, 1.0)
}

/// Exponential time-decay weight with the given half-life in days.
///
/// If `published_at` is in the future relative to `now`, returns 1.0.
pub fn time_decay_weight(
    published_at: DateTime<Utc>,
    now: DateTime<Utc>,
    half_life_days: f64,
) -> f64 {
    let age = now.signed_duration_since(published_at);
    let age_days = age.num_milliseconds() as f64 / (1000.0 * 60.0 * 60.0 * 24.0);
    if age_days <= 0.0 {
        return 1.0;
    }
    (-f64::ln(2.0) / half_life_days * age_days).exp()
}

/// Time-weighted average of article sentiments.
///
/// Returns a neutral `SentimentResult` (all zeros, `article_count` 0) when `articles` is empty.
pub fn aggregate_sentiments(
    articles: &[ArticleSentiment],
    now: DateTime<Utc>,
    half_life_days: f64,
) -> SentimentResult {
    if articles.is_empty() {
        return SentimentResult {
            positive: 0.0,
            negative: 0.0,
            neutral: 0.0,
            compound: 0.0,
            article_count: 0,
        };
    }

    let mut total_weight = 0.0;
    let mut weighted_pos = 0.0;
    let mut weighted_neg = 0.0;
    let mut weighted_neu = 0.0;
    let mut weighted_comp = 0.0;

    for article in articles {
        let w = time_decay_weight(article.published_at, now, half_life_days);
        total_weight += w;
        weighted_pos += w * article.positive;
        weighted_neg += w * article.negative;
        weighted_neu += w * article.neutral;
        weighted_comp += w * article.compound;
    }

    SentimentResult {
        positive: weighted_pos / total_weight,
        negative: weighted_neg / total_weight,
        neutral: weighted_neu / total_weight,
        compound: weighted_comp / total_weight,
        article_count: articles.len(),
    }
}

/// Build an `ArticleSentiment` from raw model probabilities `[positive, negative, neutral]`.
pub fn article_sentiment_from_probs(
    probs: [f64; 3],
    published_at: DateTime<Utc>,
) -> ArticleSentiment {
    ArticleSentiment {
        positive: probs[0],
        negative: probs[1],
        neutral: probs[2],
        compound: compound_from_probs(probs[0], probs[1], probs[2]),
        published_at,
    }
}

/// Scorer that always returns neutral — used as a placeholder until ONNX is integrated.
pub struct NullSentimentScorer;

impl crate::analysis::screener::SentimentScorer for NullSentimentScorer {
    fn score(&self, _headlines: &[&str]) -> Result<f64, AnalysisError> {
        Ok(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::screener::SentimentScorer;
    use chrono::Utc;

    #[test]
    fn null_scorer_returns_neutral() {
        let scorer = NullSentimentScorer;
        let result = scorer.score(&["Good earnings", "Market rally"]).unwrap();
        assert_eq!(result, 0.0);
    }

    #[test]
    fn null_scorer_empty_headlines_returns_neutral() {
        let scorer = NullSentimentScorer;
        let result = scorer.score(&[]).unwrap();
        assert_eq!(result, 0.0);
    }

    #[test]
    fn compound_from_probs_positive() {
        let c = compound_from_probs(0.8, 0.1, 0.1);
        assert!((c - 0.7).abs() < 1e-6);
    }

    #[test]
    fn compound_from_probs_negative() {
        let c = compound_from_probs(0.1, 0.8, 0.1);
        assert!((c - (-0.7)).abs() < 1e-6);
    }

    #[test]
    fn compound_from_probs_neutral() {
        let c = compound_from_probs(0.33, 0.33, 0.34);
        assert!(c.abs() < 0.01);
    }

    #[test]
    fn time_decay_weight_now_is_1() {
        let now = Utc::now();
        let w = time_decay_weight(now, now, 3.5);
        assert!((w - 1.0).abs() < 1e-6);
    }

    #[test]
    fn time_decay_weight_at_half_life() {
        let now = Utc::now();
        let published = now - chrono::Duration::hours(84); // 3.5 days
        let w = time_decay_weight(published, now, 3.5);
        assert!(
            (w - 0.5).abs() < 0.01,
            "Weight at half life should be ~0.5, got {}",
            w
        );
    }

    #[test]
    fn time_decay_weight_old_article_near_zero() {
        let now = Utc::now();
        let published = now - chrono::Duration::days(30);
        let w = time_decay_weight(published, now, 3.5);
        assert!(
            w < 0.01,
            "30-day old article weight should be near zero, got {}",
            w
        );
    }

    #[test]
    fn aggregate_sentiments_single_article() {
        let now = Utc::now();
        let articles = vec![ArticleSentiment {
            positive: 0.8,
            negative: 0.1,
            neutral: 0.1,
            compound: 0.7,
            published_at: now,
        }];
        let result = aggregate_sentiments(&articles, now, 3.5);
        assert!((result.compound - 0.7).abs() < 1e-6);
        assert_eq!(result.article_count, 1);
    }

    #[test]
    fn aggregate_sentiments_decay_weights_recent_higher() {
        let now = Utc::now();
        let recent = ArticleSentiment {
            positive: 0.9,
            negative: 0.05,
            neutral: 0.05,
            compound: 0.85,
            published_at: now,
        };
        let old = ArticleSentiment {
            positive: 0.05,
            negative: 0.9,
            neutral: 0.05,
            compound: -0.85,
            published_at: now - chrono::Duration::days(7),
        };
        let result = aggregate_sentiments(&[recent, old], now, 3.5);
        // Recent article dominates due to decay
        assert!(
            result.compound > 0.0,
            "Recent positive should dominate, got {}",
            result.compound
        );
    }

    #[test]
    fn aggregate_sentiments_empty_returns_neutral() {
        let now = Utc::now();
        let result = aggregate_sentiments(&[], now, 3.5);
        assert_eq!(result.compound, 0.0);
        assert_eq!(result.article_count, 0);
    }

    #[test]
    fn article_sentiment_from_probs_roundtrip() {
        let now = Utc::now();
        let article = article_sentiment_from_probs([0.7, 0.2, 0.1], now);
        assert!((article.positive - 0.7).abs() < 1e-6);
        assert!((article.negative - 0.2).abs() < 1e-6);
        assert!((article.neutral - 0.1).abs() < 1e-6);
        assert!((article.compound - 0.5).abs() < 1e-6);
        assert_eq!(article.published_at, now);
    }
}
