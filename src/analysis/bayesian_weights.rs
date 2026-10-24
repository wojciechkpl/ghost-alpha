//! Bayesian strategy weight updating — Beta distributions for each strategy's
//! hit rate, with exponential decay and regime-aware blending.
//!
//! Phase 6 (F24): Each strategy-regime pair maintains a Beta(α, β) distribution
//! that updates from realized trade outcomes. Weights are blended with regime
//! priors at a configurable blend_factor (default: 0.4 = 40% Bayesian, 60% prior).

use std::collections::HashMap;

use chrono::NaiveDate;

use crate::analysis::adaptive::StrategyWeights;
use crate::data::models::MarketRegime;

// ─── Strategy Belief ────────────────────────────────────────────────────────

/// Beta distribution belief about a strategy's hit rate in a given regime.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StrategyBelief {
    pub strategy: String,
    /// Pseudo-count of successes (Alpha parameter).
    pub alpha: f64,
    /// Pseudo-count of failures (Beta parameter).
    pub beta: f64,
    pub regime: MarketRegime,
    pub last_updated: NaiveDate,
}

impl StrategyBelief {
    /// Weakly informative prior: Beta(2, 2) centered at 50%.
    pub fn new_prior(strategy: &str, regime: &MarketRegime, as_of: NaiveDate) -> Self {
        Self {
            strategy: strategy.to_string(),
            alpha: 2.0,
            beta: 2.0,
            regime: regime.clone(),
            last_updated: as_of,
        }
    }

    /// Posterior mean: E[p] = α / (α + β).
    pub fn expected_hit_rate(&self) -> f64 {
        let total = self.alpha + self.beta;
        if total <= 0.0 {
            return 0.5;
        }
        self.alpha / total
    }

    /// Posterior variance: Var[p] = αβ / ((α+β)²(α+β+1)).
    pub fn variance(&self) -> f64 {
        let n = self.alpha + self.beta;
        if n <= 0.0 || n + 1.0 <= 0.0 {
            return 0.25;
        }
        (self.alpha * self.beta) / (n * n * (n + 1.0))
    }

    /// Approximate 95% credible interval width.
    pub fn uncertainty(&self) -> f64 {
        2.0 * 1.96 * self.variance().sqrt()
    }

    /// Total observations (pseudo-counts minus prior).
    pub fn observation_count(&self) -> f64 {
        (self.alpha + self.beta - 4.0).max(0.0) // subtract prior (2+2)
    }

    /// Update with a batch of outcomes, applying exponential decay to old evidence.
    ///
    /// `decay_factor` = 0.995 per observation → half-life ~138 observations (~6 months).
    pub fn update_batch(&mut self, outcomes: &[bool], decay_factor: f64, as_of: NaiveDate) {
        if outcomes.is_empty() {
            return;
        }

        // Decay existing evidence
        let decay = decay_factor.powi(outcomes.len() as i32);
        self.alpha *= decay;
        self.beta *= decay;

        // Add new evidence
        let wins = outcomes.iter().filter(|&&o| o).count() as f64;
        let losses = outcomes.len() as f64 - wins;
        self.alpha += wins;
        self.beta += losses;

        // Floor to prevent complete decay
        self.alpha = self.alpha.max(1.0);
        self.beta = self.beta.max(1.0);

        self.last_updated = as_of;
    }
}

// ─── Weight Conversion ──────────────────────────────────────────────────────

/// Convert Bayesian beliefs into strategy weights, blended with regime priors.
///
/// `blend_factor`:
/// - 0.0 = pure regime-based weights (ignores performance)
/// - 1.0 = pure Bayesian weights (ignores priors)
/// - 0.4 = default (60% prior, 40% Bayesian)
///
/// Guardrails: 5% floor, 60% ceiling per strategy.
pub fn bayesian_strategy_weights(
    beliefs: &[StrategyBelief],
    prior_weights: &StrategyWeights,
    blend_factor: f64,
    min_observations: usize,
) -> StrategyWeights {
    let blend = blend_factor.clamp(0.0, 1.0);

    // Check cold start: need minimum observations across all beliefs
    let total_obs: f64 = beliefs.iter().map(|b| b.observation_count()).sum();
    if total_obs < min_observations as f64 {
        return prior_weights.clone();
    }

    // Compute Bayesian weights: confidence-adjusted expected hit rate
    let mut bayes_weights: HashMap<String, f64> = HashMap::new();
    for belief in beliefs {
        let confidence_adjusted =
            belief.expected_hit_rate() * (1.0 - belief.uncertainty().min(0.5));
        bayes_weights.insert(belief.strategy.clone(), confidence_adjusted.max(0.01));
    }

    // Normalize Bayesian weights
    let total: f64 = bayes_weights.values().sum();
    if total > 0.0 {
        for v in bayes_weights.values_mut() {
            *v /= total;
        }
    }

    // Blend with priors
    let mut final_weights: HashMap<String, f64> = HashMap::new();
    for (strategy, prior_w) in &prior_weights.weights {
        let bayes_w = bayes_weights.get(strategy).copied().unwrap_or(0.25);
        let blended = (1.0 - blend) * prior_w + blend * bayes_w;
        final_weights.insert(strategy.clone(), blended);
    }

    // Renormalize with guardrails (5% floor, 60% ceiling)
    for _ in 0..5 {
        for v in final_weights.values_mut() {
            *v = v.clamp(0.05, 0.60);
        }
        let total: f64 = final_weights.values().sum();
        if total > 0.0 {
            for v in final_weights.values_mut() {
                *v /= total;
            }
        }
    }

    StrategyWeights {
        weights: final_weights,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 3, 25).unwrap()
    }

    fn make_prior_weights() -> StrategyWeights {
        StrategyWeights {
            weights: [
                ("minervini".to_string(), 0.30),
                ("breakout".to_string(), 0.30),
                ("mean-reversion".to_string(), 0.20),
                ("trend-strength".to_string(), 0.20),
            ]
            .into_iter()
            .collect(),
        }
    }

    // ── StrategyBelief ──────────────────────────────────────────────────────

    #[test]
    fn prior_has_50_50_hit_rate() {
        let b = StrategyBelief::new_prior("test", &MarketRegime::Bull, today());
        assert!((b.expected_hit_rate() - 0.5).abs() < 0.01);
    }

    #[test]
    fn update_shifts_hit_rate() {
        let mut b = StrategyBelief::new_prior("test", &MarketRegime::Bull, today());
        // 10 wins, 0 losses
        b.update_batch(&[true; 10], 1.0, today());
        assert!(b.expected_hit_rate() > 0.7, "10 wins should push above 0.7");
    }

    #[test]
    fn update_with_losses_reduces_hit_rate() {
        let mut b = StrategyBelief::new_prior("test", &MarketRegime::Bull, today());
        b.update_batch(&[false; 10], 1.0, today());
        assert!(
            b.expected_hit_rate() < 0.3,
            "10 losses should push below 0.3"
        );
    }

    #[test]
    fn decay_reduces_old_evidence() {
        let mut b = StrategyBelief::new_prior("test", &MarketRegime::Bull, today());
        // Add 20 wins
        b.update_batch(&[true; 20], 1.0, today());
        let before = b.expected_hit_rate();

        // Now add 10 losses with heavy decay on old evidence
        b.update_batch(&[false; 10], 0.9, today());
        let after = b.expected_hit_rate();

        assert!(after < before, "Decay + losses should reduce hit rate");
    }

    #[test]
    fn alpha_beta_never_below_1() {
        let mut b = StrategyBelief::new_prior("test", &MarketRegime::Bull, today());
        // Extreme decay
        b.update_batch(&[false; 1], 0.01, today());
        assert!(b.alpha >= 1.0);
        assert!(b.beta >= 1.0);
    }

    #[test]
    fn uncertainty_decreases_with_more_data() {
        let mut b = StrategyBelief::new_prior("test", &MarketRegime::Bull, today());
        let u_prior = b.uncertainty();
        b.update_batch(&[true; 30], 1.0, today());
        let u_after = b.uncertainty();
        assert!(u_after < u_prior, "More data should reduce uncertainty");
    }

    #[test]
    fn observation_count_tracks_updates() {
        let mut b = StrategyBelief::new_prior("test", &MarketRegime::Bull, today());
        assert!((b.observation_count() - 0.0).abs() < 0.01);
        b.update_batch(&[true; 10], 1.0, today());
        assert!((b.observation_count() - 10.0).abs() < 0.01);
    }

    #[test]
    fn empty_update_is_noop() {
        let mut b = StrategyBelief::new_prior("test", &MarketRegime::Bull, today());
        let alpha_before = b.alpha;
        b.update_batch(&[], 0.995, today());
        assert!((b.alpha - alpha_before).abs() < 0.01);
    }

    // ── bayesian_strategy_weights ───────────────────────────────────────────

    #[test]
    fn cold_start_returns_prior() {
        let beliefs: Vec<StrategyBelief> =
            ["minervini", "breakout", "mean-reversion", "trend-strength"]
                .iter()
                .map(|s| StrategyBelief::new_prior(s, &MarketRegime::Bull, today()))
                .collect();

        let prior = make_prior_weights();
        let result = bayesian_strategy_weights(&beliefs, &prior, 0.4, 20);
        // With 0 observations, should return prior unchanged
        assert!((result.get("minervini") - 0.30).abs() < 0.02);
    }

    #[test]
    fn winning_strategy_gets_more_weight() {
        let mut beliefs: Vec<StrategyBelief> =
            ["minervini", "breakout", "mean-reversion", "trend-strength"]
                .iter()
                .map(|s| StrategyBelief::new_prior(s, &MarketRegime::Bull, today()))
                .collect();

        // Breakout wins 20/20
        beliefs[1].update_batch(&[true; 20], 1.0, today());
        // Others win 5/20
        for i in [0, 2, 3] {
            let mut outcomes = vec![true; 5];
            outcomes.extend([false; 15]);
            beliefs[i].update_batch(&outcomes, 1.0, today());
        }

        let prior = make_prior_weights();
        let result = bayesian_strategy_weights(&beliefs, &prior, 0.4, 20);

        assert!(
            result.get("breakout") > result.get("mean-reversion"),
            "Winning strategy should get more weight: breakout={:.3} vs mean-rev={:.3}",
            result.get("breakout"),
            result.get("mean-reversion")
        );
    }

    #[test]
    fn blend_factor_0_returns_prior() {
        let beliefs: Vec<StrategyBelief> =
            ["minervini", "breakout", "mean-reversion", "trend-strength"]
                .iter()
                .map(|s| {
                    let mut b = StrategyBelief::new_prior(s, &MarketRegime::Bull, today());
                    b.update_batch(&[true; 30], 1.0, today());
                    b
                })
                .collect();

        let prior = make_prior_weights();
        let result = bayesian_strategy_weights(&beliefs, &prior, 0.0, 20);
        assert!((result.get("minervini") - 0.30).abs() < 0.02);
    }

    #[test]
    fn weights_sum_to_1() {
        let beliefs: Vec<StrategyBelief> =
            ["minervini", "breakout", "mean-reversion", "trend-strength"]
                .iter()
                .map(|s| {
                    let mut b = StrategyBelief::new_prior(s, &MarketRegime::Bull, today());
                    let mut outcomes = vec![true; 15];
                    outcomes.extend([false; 5]);
                    b.update_batch(&outcomes, 0.995, today());
                    b
                })
                .collect();

        let prior = make_prior_weights();
        let result = bayesian_strategy_weights(&beliefs, &prior, 0.4, 10);
        assert!(
            (result.sum() - 1.0).abs() < 0.02,
            "Sum should be ~1.0, got {}",
            result.sum()
        );
    }

    #[test]
    fn guardrails_prevent_extreme_weights() {
        let mut beliefs: Vec<StrategyBelief> =
            ["minervini", "breakout", "mean-reversion", "trend-strength"]
                .iter()
                .map(|s| StrategyBelief::new_prior(s, &MarketRegime::Bull, today()))
                .collect();

        // Extreme: one strategy 100% win, others 0%
        beliefs[0].update_batch(&[true; 50], 1.0, today());
        for belief in beliefs.iter_mut().skip(1) {
            belief.update_batch(&[false; 50], 1.0, today());
        }

        let prior = make_prior_weights();
        let result = bayesian_strategy_weights(&beliefs, &prior, 0.8, 20);

        for (strategy, weight) in &result.weights {
            assert!(
                *weight >= 0.04,
                "{strategy} weight {weight:.3} below minimum"
            );
            assert!(
                *weight <= 0.65,
                "{strategy} weight {weight:.3} above maximum"
            );
        }
    }
}
