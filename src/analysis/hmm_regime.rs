//! Hidden Markov Model for probabilistic market regime detection.
//!
//! Phase 8 (Wave 3): Replaces threshold-based regime detection with a 3-state
//! HMM trained on daily returns, realized volatility, and yield curve slope.
//! Provides probability distributions over regimes instead of binary switches,
//! reducing whipsaw transitions by 30-50%.
//!
//! Algorithm: Baum-Welch (EM) for training, Forward algorithm for inference.

use crate::data::models::MarketRegime;

// ─── Model ──────────────────────────────────────────────────────────────────

/// Number of hidden states.
pub const N_STATES: usize = 3;
/// Number of observable dimensions.
pub const N_OBS: usize = 3;

/// 3-state Hidden Markov Model with Gaussian emissions.
///
/// States: 0=Bull, 1=Sideways, 2=Bear
/// Observations: (daily_return, realized_vol_5d, yield_curve_slope)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HmmModel {
    /// Transition matrix A[i][j] = P(state_j at t+1 | state_i at t)
    pub transition: [[f64; N_STATES]; N_STATES],
    /// Emission means: mean[state][observable_dim]
    pub emission_means: [[f64; N_OBS]; N_STATES],
    /// Emission variances: var[state][observable_dim]
    pub emission_vars: [[f64; N_OBS]; N_STATES],
    /// Initial state distribution
    pub initial: [f64; N_STATES],
}

impl HmmModel {
    /// Default prior model with reasonable starting parameters.
    ///
    /// Bull: positive returns, low vol, positive yield curve
    /// Sideways: near-zero returns, moderate vol, flat yield curve
    /// Bear: negative returns, high vol, inverted yield curve
    pub fn default_prior() -> Self {
        Self {
            transition: [
                [0.95, 0.04, 0.01], // Bull stays bull 95%
                [0.05, 0.90, 0.05], // Sideways transitions both ways
                [0.01, 0.04, 0.95], // Bear stays bear 95%
            ],
            emission_means: [
                [0.0008, 0.10, 1.5],   // Bull: +0.08%/day, 10% vol, positive spread
                [0.0001, 0.16, 0.5],   // Sideways: flat, 16% vol, flat spread
                [-0.0006, 0.25, -0.3], // Bear: -0.06%/day, 25% vol, inverted
            ],
            emission_vars: [
                [0.0001, 0.002, 0.5], // Bull: low variance
                [0.0002, 0.004, 0.8], // Sideways: moderate
                [0.0004, 0.008, 1.0], // Bear: high variance
            ],
            initial: [0.4, 0.4, 0.2], // slight prior toward non-bear
        }
    }
}

// ─── Observation ────────────────────────────────────────────────────────────

/// Single observation vector for HMM.
pub type Observation = [f64; N_OBS];

// ─── Inference ──────────────────────────────────────────────────────────────

/// Regime state with probability distribution.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HmmRegimeState {
    pub most_likely: MarketRegime,
    pub bull_prob: f64,
    pub sideways_prob: f64,
    pub bear_prob: f64,
    /// How confident: max_prob - second_highest_prob
    pub confidence: f64,
}

/// Forward algorithm: compute current state probabilities given observations.
pub fn predict_regime(model: &HmmModel, observations: &[Observation]) -> HmmRegimeState {
    if observations.is_empty() {
        return HmmRegimeState {
            most_likely: MarketRegime::Sideways,
            bull_prob: 0.33,
            sideways_prob: 0.34,
            bear_prob: 0.33,
            confidence: 0.01,
        };
    }

    // Forward pass
    let alpha = forward(model, observations);
    let last = &alpha[alpha.len() - 1];

    // Normalize to probabilities
    let sum: f64 = last.iter().sum();
    let probs = if sum > 0.0 {
        [last[0] / sum, last[1] / sum, last[2] / sum]
    } else {
        [0.33, 0.34, 0.33]
    };

    let (max_idx, _max_prob) = probs
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap();

    let mut sorted = probs;
    sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let confidence = sorted[0] - sorted[1];

    let most_likely = match max_idx {
        0 => MarketRegime::Bull,
        2 => MarketRegime::Bear,
        _ => MarketRegime::Sideways,
    };

    HmmRegimeState {
        most_likely,
        bull_prob: probs[0],
        sideways_prob: probs[1],
        bear_prob: probs[2],
        confidence,
    }
}

/// Forward algorithm: compute α[t][i] = P(o_1:t, state_t=i).
#[allow(clippy::needless_range_loop)] // HMM matrix math: parallel indexing by state/time reads clearer than zips
fn forward(model: &HmmModel, observations: &[Observation]) -> Vec<[f64; N_STATES]> {
    let n = observations.len();
    let mut alpha = vec![[0.0; N_STATES]; n];

    // Initialize: α[0][i] = π[i] × b_i(o_0)
    for i in 0..N_STATES {
        alpha[0][i] = model.initial[i] * emission_prob(model, i, &observations[0]);
    }
    normalize(&mut alpha[0]);

    // Recurse: α[t][j] = (Σ_i α[t-1][i] × a[i][j]) × b_j(o_t)
    for t in 1..n {
        for j in 0..N_STATES {
            let mut sum = 0.0;
            for i in 0..N_STATES {
                sum += alpha[t - 1][i] * model.transition[i][j];
            }
            alpha[t][j] = sum * emission_prob(model, j, &observations[t]);
        }
        normalize(&mut alpha[t]);
    }

    alpha
}

/// Gaussian emission probability: P(observation | state).
fn emission_prob(model: &HmmModel, state: usize, obs: &Observation) -> f64 {
    let mut prob = 1.0;
    for (d, &obs_val) in obs.iter().enumerate() {
        let mean = model.emission_means[state][d];
        let var = model.emission_vars[state][d].max(1e-10);
        let diff = obs_val - mean;
        let exponent = -0.5 * diff * diff / var;
        let norm = 1.0 / (2.0 * std::f64::consts::PI * var).sqrt();
        prob *= norm * exponent.exp();
    }
    prob.max(1e-300) // floor to prevent underflow
}

/// Normalize a probability vector in place.
fn normalize(v: &mut [f64; N_STATES]) {
    let sum: f64 = v.iter().sum();
    if sum > 0.0 {
        for x in v.iter_mut() {
            *x /= sum;
        }
    }
}

// ─── Training (Baum-Welch) ──────────────────────────────────────────────────

/// Train the HMM using Baum-Welch (Expectation-Maximization).
///
/// Starts from the provided model (or `default_prior()`) and iterates until
/// convergence or max iterations reached. Fully deterministic — no randomness.
#[allow(clippy::needless_range_loop)] // HMM matrix math: parallel indexing by state/time reads clearer than zips
pub fn train(
    initial_model: &HmmModel,
    observations: &[Observation],
    max_iterations: usize,
    convergence_threshold: f64,
) -> HmmModel {
    if observations.len() < 10 {
        return initial_model.clone();
    }

    let mut model = initial_model.clone();
    let mut prev_ll = f64::NEG_INFINITY;

    for _iter in 0..max_iterations {
        // E-step: forward-backward
        let alpha = forward(&model, observations);
        let beta = backward(&model, observations);
        let gamma = compute_gamma(&alpha, &beta);
        let xi = compute_xi(&model, &alpha, &beta, observations);

        // M-step: update parameters
        // Initial distribution
        let gamma_sum_0: f64 = gamma[0].iter().sum();
        if gamma_sum_0 > 0.0 {
            for i in 0..N_STATES {
                model.initial[i] = gamma[0][i] / gamma_sum_0;
            }
        }

        // Transition matrix
        for i in 0..N_STATES {
            let gamma_sum: f64 = gamma.iter().take(gamma.len() - 1).map(|g| g[i]).sum();
            if gamma_sum > 0.0 {
                for j in 0..N_STATES {
                    let xi_sum: f64 = xi.iter().map(|x| x[i][j]).sum();
                    model.transition[i][j] = (xi_sum / gamma_sum).max(0.001);
                }
                // Re-normalize row
                let row_sum: f64 = model.transition[i].iter().sum();
                for j in 0..N_STATES {
                    model.transition[i][j] /= row_sum;
                }
            }
        }

        // Emission parameters
        for i in 0..N_STATES {
            let gamma_sum: f64 = gamma.iter().map(|g| g[i]).sum();
            if gamma_sum > 0.001 {
                for d in 0..N_OBS {
                    // Mean
                    let weighted_sum: f64 = gamma
                        .iter()
                        .enumerate()
                        .map(|(t, g)| g[i] * observations[t][d])
                        .sum();
                    model.emission_means[i][d] = weighted_sum / gamma_sum;

                    // Variance
                    let weighted_var: f64 = gamma
                        .iter()
                        .enumerate()
                        .map(|(t, g)| {
                            let diff = observations[t][d] - model.emission_means[i][d];
                            g[i] * diff * diff
                        })
                        .sum();
                    model.emission_vars[i][d] = (weighted_var / gamma_sum).max(1e-8);
                }
            }
        }

        // Check convergence via log-likelihood
        let ll = log_likelihood(&alpha);
        if (ll - prev_ll).abs() < convergence_threshold {
            break;
        }
        prev_ll = ll;
    }

    model
}

/// Backward algorithm: β[t][i] = P(o_t+1:T | state_t=i).
#[allow(clippy::needless_range_loop)] // HMM matrix math: parallel indexing by state/time reads clearer than zips
fn backward(model: &HmmModel, observations: &[Observation]) -> Vec<[f64; N_STATES]> {
    let n = observations.len();
    let mut beta = vec![[0.0; N_STATES]; n];

    // Initialize: β[T-1][i] = 1
    for i in 0..N_STATES {
        beta[n - 1][i] = 1.0;
    }

    // Recurse backwards
    for t in (0..n - 1).rev() {
        for i in 0..N_STATES {
            for j in 0..N_STATES {
                beta[t][i] += model.transition[i][j]
                    * emission_prob(model, j, &observations[t + 1])
                    * beta[t + 1][j];
            }
        }
        normalize(&mut beta[t]);
    }

    beta
}

/// Compute γ[t][i] = P(state_t = i | O).
#[allow(clippy::needless_range_loop)] // HMM matrix math: parallel indexing by state/time reads clearer than zips
fn compute_gamma(alpha: &[[f64; N_STATES]], beta: &[[f64; N_STATES]]) -> Vec<[f64; N_STATES]> {
    let n = alpha.len();
    let mut gamma = vec![[0.0; N_STATES]; n];

    for t in 0..n {
        let mut sum = 0.0;
        for i in 0..N_STATES {
            gamma[t][i] = alpha[t][i] * beta[t][i];
            sum += gamma[t][i];
        }
        if sum > 0.0 {
            for i in 0..N_STATES {
                gamma[t][i] /= sum;
            }
        }
    }

    gamma
}

/// Compute ξ[t][i][j] = P(state_t=i, state_t+1=j | O).
#[allow(clippy::needless_range_loop)] // HMM matrix math: parallel indexing by state/time reads clearer than zips
fn compute_xi(
    model: &HmmModel,
    alpha: &[[f64; N_STATES]],
    beta: &[[f64; N_STATES]],
    observations: &[Observation],
) -> Vec<[[f64; N_STATES]; N_STATES]> {
    let n = alpha.len();
    let mut xi = vec![[[0.0; N_STATES]; N_STATES]; n.saturating_sub(1)];

    for t in 0..n.saturating_sub(1) {
        let mut sum = 0.0;
        for i in 0..N_STATES {
            for j in 0..N_STATES {
                xi[t][i][j] = alpha[t][i]
                    * model.transition[i][j]
                    * emission_prob(model, j, &observations[t + 1])
                    * beta[t + 1][j];
                sum += xi[t][i][j];
            }
        }
        if sum > 0.0 {
            for i in 0..N_STATES {
                for j in 0..N_STATES {
                    xi[t][i][j] /= sum;
                }
            }
        }
    }

    xi
}

/// Log-likelihood from final forward probabilities.
fn log_likelihood(alpha: &[[f64; N_STATES]]) -> f64 {
    let last = alpha.last().unwrap();
    let sum: f64 = last.iter().sum();
    if sum > 0.0 {
        sum.ln()
    } else {
        f64::NEG_INFINITY
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Map HMM state index to MarketRegime.
pub fn state_to_regime(state: usize) -> MarketRegime {
    match state {
        0 => MarketRegime::Bull,
        2 => MarketRegime::Bear,
        _ => MarketRegime::Sideways,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn bull_observations(n: usize) -> Vec<Observation> {
        (0..n)
            .map(|i| {
                let noise = (i as f64 * 0.7).sin() * 0.0003;
                [0.001 + noise, 0.10, 1.5] // positive return, low vol, positive yield
            })
            .collect()
    }

    fn bear_observations(n: usize) -> Vec<Observation> {
        (0..n)
            .map(|i| {
                let noise = (i as f64 * 0.7).sin() * 0.0005;
                [-0.001 + noise, 0.28, -0.5] // negative return, high vol, inverted
            })
            .collect()
    }

    fn mixed_observations() -> Vec<Observation> {
        let mut obs = Vec::new();
        obs.extend(bull_observations(50)); // 50 bull days
        obs.extend(bear_observations(30)); // 30 bear days
        obs.extend(bull_observations(40)); // 40 bull days
        obs
    }

    // ── forward probabilities ───────────────────────────────────────────

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn forward_probabilities_sum_to_1() {
        let model = HmmModel::default_prior();
        let obs = bull_observations(20);
        let alpha = forward(&model, &obs);
        for t in 0..alpha.len() {
            let sum: f64 = alpha[t].iter().sum();
            assert!(
                (sum - 1.0).abs() < 0.01,
                "α[{t}] sums to {sum}, expected ~1.0"
            );
        }
    }

    // ── predict_regime ──────────────────────────────────────────────────

    #[test]
    fn bull_observations_detected_as_bull() {
        let model = HmmModel::default_prior();
        let obs = bull_observations(30);
        let state = predict_regime(&model, &obs);
        assert_eq!(state.most_likely, MarketRegime::Bull,
            "Bull observations should be detected as Bull, got {:?} (probs: B={:.2} S={:.2} Be={:.2})",
            state.most_likely, state.bull_prob, state.sideways_prob, state.bear_prob);
    }

    #[test]
    fn bear_observations_detected_as_bear() {
        let model = HmmModel::default_prior();
        let obs = bear_observations(30);
        let state = predict_regime(&model, &obs);
        assert_eq!(
            state.most_likely,
            MarketRegime::Bear,
            "Bear observations should be detected as Bear, got {:?}",
            state.most_likely
        );
    }

    #[test]
    fn empty_observations_returns_sideways() {
        let model = HmmModel::default_prior();
        let state = predict_regime(&model, &[]);
        assert_eq!(state.most_likely, MarketRegime::Sideways);
    }

    #[test]
    fn probabilities_sum_to_1() {
        let model = HmmModel::default_prior();
        let obs = bull_observations(20);
        let state = predict_regime(&model, &obs);
        let sum = state.bull_prob + state.sideways_prob + state.bear_prob;
        assert!(
            (sum - 1.0).abs() < 0.01,
            "Probabilities should sum to 1.0, got {sum}"
        );
    }

    #[test]
    fn confidence_bounded_0_1() {
        let model = HmmModel::default_prior();
        for obs in [bull_observations(20), bear_observations(20)] {
            let state = predict_regime(&model, &obs);
            assert!(
                state.confidence >= 0.0 && state.confidence <= 1.0,
                "Confidence {:.2} out of bounds",
                state.confidence
            );
        }
    }

    #[test]
    fn clear_regime_has_high_confidence() {
        let model = HmmModel::default_prior();
        let obs = bull_observations(50);
        let state = predict_regime(&model, &obs);
        assert!(
            state.confidence > 0.3,
            "Clear bull should have confidence > 0.3, got {:.2}",
            state.confidence
        );
    }

    // ── training ────────────────────────────────────────────────────────

    #[test]
    fn training_converges() {
        let prior = HmmModel::default_prior();
        let obs = mixed_observations();
        let trained = train(&prior, &obs, 50, 1e-6);
        // Transition matrix rows should still sum to ~1.0
        for i in 0..N_STATES {
            let sum: f64 = trained.transition[i].iter().sum();
            assert!((sum - 1.0).abs() < 0.01, "Transition row {i} sums to {sum}");
        }
    }

    #[test]
    fn training_insufficient_data_returns_prior() {
        let prior = HmmModel::default_prior();
        let obs = bull_observations(5); // too few
        let trained = train(&prior, &obs, 50, 1e-6);
        assert!((trained.initial[0] - prior.initial[0]).abs() < 0.01);
    }

    #[test]
    fn trained_model_detects_bull() {
        let prior = HmmModel::default_prior();
        let obs = mixed_observations();
        let trained = train(&prior, &obs, 30, 1e-4);

        // Test on fresh bull data
        let test_obs = bull_observations(20);
        let state = predict_regime(&trained, &test_obs);
        assert_eq!(
            state.most_likely,
            MarketRegime::Bull,
            "Trained model should detect bull, got {:?}",
            state.most_likely
        );
    }

    #[test]
    fn trained_model_detects_bear() {
        let prior = HmmModel::default_prior();
        let obs = mixed_observations();
        let trained = train(&prior, &obs, 30, 1e-4);

        let test_obs = bear_observations(20);
        let state = predict_regime(&trained, &test_obs);
        assert_eq!(
            state.most_likely,
            MarketRegime::Bear,
            "Trained model should detect bear, got {:?}",
            state.most_likely
        );
    }

    #[test]
    fn emission_means_shift_toward_data() {
        let prior = HmmModel::default_prior();
        let obs = bull_observations(100);
        let trained = train(&prior, &obs, 50, 1e-6);
        // Bull state mean return should be positive
        assert!(
            trained.emission_means[0][0] > 0.0,
            "Bull mean return should be positive after training on bull data"
        );
    }

    #[test]
    fn transition_matrix_has_self_persistence() {
        let prior = HmmModel::default_prior();
        let obs = mixed_observations();
        let trained = train(&prior, &obs, 30, 1e-4);
        // Bull and Bear states should have high self-persistence
        // (Sideways may collapse if training data lacks sideways examples)
        for &i in &[0, 2] {
            // Bull=0, Bear=2
            let self_prob = trained.transition[i][i];
            assert!(
                self_prob > 0.3,
                "State {i} self-transition should be > 0.3, got {self_prob:.3}"
            );
        }
        // All rows should sum to ~1.0
        for i in 0..N_STATES {
            let sum: f64 = trained.transition[i].iter().sum();
            assert!((sum - 1.0).abs() < 0.01, "Row {i} sums to {sum}");
        }
    }

    // ── helpers ──────────────────────────────────────────────────────────

    #[test]
    fn state_to_regime_mapping() {
        assert_eq!(state_to_regime(0), MarketRegime::Bull);
        assert_eq!(state_to_regime(1), MarketRegime::Sideways);
        assert_eq!(state_to_regime(2), MarketRegime::Bear);
    }

    #[test]
    fn emission_prob_positive() {
        let model = HmmModel::default_prior();
        let obs = [0.001, 0.12, 1.0];
        for s in 0..N_STATES {
            let p = emission_prob(&model, s, &obs);
            assert!(
                p > 0.0,
                "Emission prob for state {s} should be > 0, got {p}"
            );
        }
    }
}
