//! Mean-Variance Optimisation (MVO) using analytical Lagrangian solutions.
//!
//! Avoids external QP solvers by using closed-form solutions for unconstrained
//! problems and projected gradient for box constraints.

use nalgebra::{DMatrix, DVector};

use crate::error::OptimizeError;

// ─── Types ──────────────────────────────────────────────────────────────────

/// Inputs for Mean-Variance Optimisation.
#[derive(Debug, Clone)]
pub struct MvoInputs {
    pub tickers: Vec<String>,
    /// Expected annual returns per asset.
    pub expected_returns: Vec<f64>,
    /// Annualised covariance matrix (N x N).
    pub covariance: DMatrix<f64>,
    /// Min weight per asset (default 0.0 = no short selling).
    pub min_weights: Vec<f64>,
    /// Max weight per asset (default 1.0).
    pub max_weights: Vec<f64>,
}

/// Result of a portfolio optimisation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OptimalPortfolio {
    pub weights: Vec<(String, f64)>,
    pub expected_return: f64,
    pub volatility: f64,
    pub sharpe_ratio: f64,
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Compute portfolio expected return: w' * mu.
pub fn portfolio_return(weights: &[f64], returns: &[f64]) -> f64 {
    weights.iter().zip(returns.iter()).map(|(w, r)| w * r).sum()
}

/// Compute portfolio volatility: sqrt(w' * Sigma * w).
pub fn portfolio_volatility(weights: &[f64], covariance: &DMatrix<f64>) -> f64 {
    let w = DVector::from_column_slice(weights);
    let sigma_w = covariance * &w;
    let var = w.dot(&sigma_w);
    // Guard against tiny negative values from floating-point noise.
    if var < 0.0 && var > -1e-12 {
        return 0.0;
    }
    var.max(0.0).sqrt()
}

/// Project weights onto box constraints [min, max] and renormalise to sum = 1.
///
/// Uses iterative clipping: clip to bounds, redistribute surplus/deficit
/// proportionally among free (non-clamped) weights, repeat until stable.
fn project_to_constraints(
    weights: &mut [f64],
    min_w: &[f64],
    max_w: &[f64],
    max_iter: usize,
) -> Result<(), OptimizeError> {
    let n = weights.len();
    for _ in 0..max_iter {
        // Clip to [min, max].
        let mut clamped = vec![false; n];
        let mut clamped_sum = 0.0;
        for i in 0..n {
            if weights[i] < min_w[i] {
                weights[i] = min_w[i];
                clamped[i] = true;
                clamped_sum += weights[i];
            } else if weights[i] > max_w[i] {
                weights[i] = max_w[i];
                clamped[i] = true;
                clamped_sum += weights[i];
            }
        }

        let free_sum: f64 = weights
            .iter()
            .enumerate()
            .filter(|(i, _)| !clamped[*i])
            .map(|(_, w)| *w)
            .sum();

        let target_free_sum = 1.0 - clamped_sum;

        if free_sum.abs() < 1e-15 {
            // All weights are clamped. Check feasibility.
            let total: f64 = weights.iter().sum();
            if (total - 1.0).abs() > 1e-6 {
                return Err(OptimizeError::InfeasibleConstraints(
                    "cannot satisfy sum-to-one with given box constraints".into(),
                ));
            }
            return Ok(());
        }

        let scale = target_free_sum / free_sum;
        for i in 0..n {
            if !clamped[i] {
                weights[i] *= scale;
            }
        }

        // Check convergence: all within bounds and sum ~1.
        let total: f64 = weights.iter().sum();
        let all_in_bounds = weights
            .iter()
            .enumerate()
            .all(|(i, &w)| w >= min_w[i] - 1e-12 && w <= max_w[i] + 1e-12);
        if all_in_bounds && (total - 1.0).abs() < 1e-10 {
            return Ok(());
        }
    }
    Ok(())
}

/// Build an `OptimalPortfolio` from raw weight vector and inputs.
fn build_portfolio(weights: &[f64], inputs: &MvoInputs, risk_free_rate: f64) -> OptimalPortfolio {
    let ret = portfolio_return(weights, &inputs.expected_returns);
    let vol = portfolio_volatility(weights, &inputs.covariance);
    let sharpe = if vol > 1e-15 {
        (ret - risk_free_rate) / vol
    } else {
        0.0
    };
    let named: Vec<(String, f64)> = inputs
        .tickers
        .iter()
        .zip(weights.iter())
        .map(|(t, &w)| (t.clone(), w))
        .collect();
    OptimalPortfolio {
        weights: named,
        expected_return: ret,
        volatility: vol,
        sharpe_ratio: sharpe,
    }
}

// ─── Core solvers ───────────────────────────────────────────────────────────

/// Solve for the minimum-variance portfolio for a given target return.
///
/// Uses the Lagrangian closed-form for the two-constraint problem:
///   min  0.5 * w' Sigma w
///   s.t. mu' w = target_return
///        1' w  = 1
///
/// Then projects to box constraints via iterative clipping.
pub fn minimum_variance(
    inputs: &MvoInputs,
    target_return: f64,
    risk_free_rate: f64,
) -> Result<OptimalPortfolio, OptimizeError> {
    let n = inputs.tickers.len();
    let sigma = &inputs.covariance;

    // Invert covariance matrix.
    let sigma_inv = sigma
        .clone()
        .try_inverse()
        .ok_or(OptimizeError::SingularMatrix)?;

    let ones = DVector::from_element(n, 1.0);
    let mu = DVector::from_column_slice(&inputs.expected_returns);

    // Compute Lagrangian constants.
    let a = (ones.transpose() * &sigma_inv * &ones)[(0, 0)];
    let b = (ones.transpose() * &sigma_inv * &mu)[(0, 0)];
    let c = (mu.transpose() * &sigma_inv * &mu)[(0, 0)];

    let det = a * c - b * b;
    if det.abs() < 1e-10 {
        // Degenerate case: all assets have same expected return.
        // Just solve minimum variance without return constraint.
        let w_vec = &sigma_inv * &ones;
        let mut weights: Vec<f64> = w_vec.iter().map(|&v| v / a).collect();
        project_to_constraints(&mut weights, &inputs.min_weights, &inputs.max_weights, 100)?;
        return Ok(build_portfolio(&weights, inputs, risk_free_rate));
    }

    // Lagrange multipliers:
    // lambda = (c - b * target) / det
    // gamma  = (a * target - b) / det
    let lambda = (c - b * target_return) / det;
    let gamma = (a * target_return - b) / det;

    let w = &sigma_inv * (lambda * &ones + gamma * &mu);
    let mut weights: Vec<f64> = w.iter().copied().collect();

    project_to_constraints(&mut weights, &inputs.min_weights, &inputs.max_weights, 100)?;

    Ok(build_portfolio(&weights, inputs, risk_free_rate))
}

/// Solve for the tangency (maximum Sharpe ratio) portfolio.
///
/// Analytical: w proportional to Sigma^{-1} * (mu - rf * 1), then normalise.
pub fn max_sharpe(
    inputs: &MvoInputs,
    risk_free_rate: f64,
) -> Result<OptimalPortfolio, OptimizeError> {
    let n = inputs.tickers.len();
    let sigma = &inputs.covariance;

    let sigma_inv = sigma
        .clone()
        .try_inverse()
        .ok_or(OptimizeError::SingularMatrix)?;

    let mu = DVector::from_column_slice(&inputs.expected_returns);
    let ones = DVector::from_element(n, 1.0);
    let excess = &mu - risk_free_rate * &ones;

    let raw = &sigma_inv * &excess;
    let raw_sum: f64 = raw.iter().sum();

    if raw_sum.abs() < 1e-15 {
        return Err(OptimizeError::SolverFailed(
            "all excess returns are zero or cancel out".into(),
        ));
    }

    let mut weights: Vec<f64> = raw.iter().map(|&v| v / raw_sum).collect();

    project_to_constraints(&mut weights, &inputs.min_weights, &inputs.max_weights, 100)?;

    Ok(build_portfolio(&weights, inputs, risk_free_rate))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a simple 2-asset MVO input.
    fn two_asset_uncorrelated(vol_a: f64, vol_b: f64, ret_a: f64, ret_b: f64) -> MvoInputs {
        let var_a = vol_a * vol_a;
        let var_b = vol_b * vol_b;
        let cov = DMatrix::from_row_slice(2, 2, &[var_a, 0.0, 0.0, var_b]);
        MvoInputs {
            tickers: vec!["A".into(), "B".into()],
            expected_returns: vec![ret_a, ret_b],
            covariance: cov,
            min_weights: vec![0.0, 0.0],
            max_weights: vec![1.0, 1.0],
        }
    }

    #[test]
    fn test_portfolio_return_calculation() {
        let weights = vec![0.6, 0.4];
        let returns = vec![0.10, 0.05];
        let ret = portfolio_return(&weights, &returns);
        assert!((ret - 0.08).abs() < 1e-10, "Expected 0.08, got {ret}");
    }

    #[test]
    fn test_portfolio_volatility_calculation() {
        // Two uncorrelated assets, equal weight, vol = 0.20 each.
        // Portfolio variance = 0.5^2 * 0.04 + 0.5^2 * 0.04 = 0.02
        // Portfolio vol = sqrt(0.02) ~ 0.14142
        let cov = DMatrix::from_row_slice(2, 2, &[0.04, 0.0, 0.0, 0.04]);
        let weights = vec![0.5, 0.5];
        let vol = portfolio_volatility(&weights, &cov);
        let expected = (0.02_f64).sqrt();
        assert!(
            (vol - expected).abs() < 1e-10,
            "Expected {expected}, got {vol}"
        );
    }

    #[test]
    fn test_two_asset_equal_vol_uncorrelated() {
        // Two assets, same vol (0.20), zero correlation.
        // Minimum variance portfolio should be 50/50.
        let inputs = two_asset_uncorrelated(0.20, 0.20, 0.10, 0.10);
        let target = 0.10; // both have same return
        let result = minimum_variance(&inputs, target, 0.0).expect("should succeed");

        let w: Vec<f64> = result.weights.iter().map(|(_, w)| *w).collect();
        assert!(
            (w[0] - 0.5).abs() < 1e-6,
            "Expected ~0.5 for asset A, got {}",
            w[0]
        );
        assert!(
            (w[1] - 0.5).abs() < 1e-6,
            "Expected ~0.5 for asset B, got {}",
            w[1]
        );
    }

    #[test]
    fn test_weights_sum_to_one() {
        let inputs = two_asset_uncorrelated(0.20, 0.30, 0.08, 0.12);
        let result = minimum_variance(&inputs, 0.10, 0.0).expect("should succeed");
        let sum: f64 = result.weights.iter().map(|(_, w)| w).sum();
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "Weights should sum to 1.0, got {sum}"
        );
    }

    #[test]
    fn test_all_weights_non_negative() {
        // With long-only constraints, all weights >= 0.
        let inputs = two_asset_uncorrelated(0.20, 0.30, 0.08, 0.12);
        let result = minimum_variance(&inputs, 0.10, 0.0).expect("should succeed");
        for (ticker, w) in &result.weights {
            assert!(*w >= -1e-8, "Weight for {ticker} is negative: {w}");
        }
    }

    #[test]
    fn test_higher_target_return_higher_vol() {
        let inputs = two_asset_uncorrelated(0.20, 0.30, 0.05, 0.15);
        let low = minimum_variance(&inputs, 0.06, 0.0).expect("low target");
        let high = minimum_variance(&inputs, 0.14, 0.0).expect("high target");
        assert!(
            high.volatility >= low.volatility - 1e-8,
            "Higher target return should yield higher (or equal) vol: {} vs {}",
            high.volatility,
            low.volatility
        );
    }

    #[test]
    fn test_max_sharpe_positive_weights() {
        let inputs = two_asset_uncorrelated(0.20, 0.30, 0.10, 0.15);
        let result = max_sharpe(&inputs, 0.02).expect("should succeed");
        for (ticker, w) in &result.weights {
            assert!(
                *w >= -1e-8,
                "Max-sharpe weight for {ticker} is negative: {w}"
            );
        }
        let sum: f64 = result.weights.iter().map(|(_, w)| w).sum();
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "Max-sharpe weights should sum to 1.0, got {sum}"
        );
    }

    #[test]
    fn test_singular_matrix_returns_error() {
        let cov = DMatrix::zeros(2, 2);
        let inputs = MvoInputs {
            tickers: vec!["A".into(), "B".into()],
            expected_returns: vec![0.10, 0.10],
            covariance: cov,
            min_weights: vec![0.0, 0.0],
            max_weights: vec![1.0, 1.0],
        };
        let err = minimum_variance(&inputs, 0.10, 0.0).unwrap_err();
        assert_eq!(err, OptimizeError::SingularMatrix);
    }

    #[test]
    fn test_three_asset_weights_sum_to_one() {
        // Verify with a larger problem.
        let cov = DMatrix::from_row_slice(
            3,
            3,
            &[0.04, 0.006, 0.002, 0.006, 0.09, 0.009, 0.002, 0.009, 0.01],
        );
        let inputs = MvoInputs {
            tickers: vec!["A".into(), "B".into(), "C".into()],
            expected_returns: vec![0.08, 0.12, 0.04],
            covariance: cov,
            min_weights: vec![0.0, 0.0, 0.0],
            max_weights: vec![1.0, 1.0, 1.0],
        };
        let result = minimum_variance(&inputs, 0.08, 0.02).expect("should succeed");
        let sum: f64 = result.weights.iter().map(|(_, w)| w).sum();
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "3-asset weights should sum to 1.0, got {sum}"
        );
    }

    #[test]
    fn test_max_sharpe_selects_higher_return_when_same_vol() {
        // If both assets have same vol but different returns, max-sharpe
        // should overweight the higher-return asset.
        let inputs = two_asset_uncorrelated(0.20, 0.20, 0.05, 0.15);
        let result = max_sharpe(&inputs, 0.02).expect("should succeed");
        let w: Vec<f64> = result.weights.iter().map(|(_, w)| *w).collect();
        assert!(
            w[1] > w[0],
            "Asset B (higher return) should get more weight: A={}, B={}",
            w[0],
            w[1]
        );
    }
}
