//! Equal Risk Contribution (Risk Parity) portfolio optimisation.
//!
//! Iterative algorithm that adjusts weights so each asset contributes equally
//! to total portfolio variance.

use nalgebra::{DMatrix, DVector};

use crate::error::OptimizeError;
use crate::optimize::mvo::OptimalPortfolio;

/// Compute the risk parity (equal risk contribution) portfolio.
///
/// Algorithm:
/// 1. Start with equal weights (1/N).
/// 2. Compute marginal risk contribution: MRC_i = (Sigma * w)_i / sigma_p.
/// 3. Risk contribution: RC_i = w_i * MRC_i.
/// 4. Target: RC_i = sigma_p / N for all i.
/// 5. New weights: w_i *= (target_RC / RC_i), then renormalise.
/// 6. Repeat until convergence.
pub fn risk_parity(
    covariance: &DMatrix<f64>,
    tickers: &[String],
    tolerance: f64,
    max_iterations: u32,
) -> Result<OptimalPortfolio, OptimizeError> {
    let n = covariance.nrows();
    if n == 0 {
        return Err(OptimizeError::SolverFailed(
            "empty covariance matrix".into(),
        ));
    }

    let mut weights = vec![1.0 / n as f64; n];

    for _ in 0..max_iterations {
        let w = DVector::from_column_slice(&weights);
        let sigma_w = covariance * &w;
        let port_var = w.dot(&sigma_w);

        if port_var <= 0.0 {
            return Err(OptimizeError::SolverFailed(
                "non-positive portfolio variance".into(),
            ));
        }

        let sigma_p = port_var.sqrt();

        // Marginal risk contributions.
        let mrc: Vec<f64> = sigma_w.iter().map(|&v| v / sigma_p).collect();

        // Risk contributions: RC_i = w_i * MRC_i.
        let rc: Vec<f64> = weights
            .iter()
            .zip(mrc.iter())
            .map(|(&w_i, &m)| w_i * m)
            .collect();

        let target_rc = sigma_p / n as f64;

        // Check convergence: all RC_i close to target.
        let max_dev = rc
            .iter()
            .map(|&r| (r - target_rc).abs())
            .fold(0.0_f64, f64::max);

        if max_dev < tolerance {
            let named: Vec<(String, f64)> = tickers
                .iter()
                .zip(weights.iter())
                .map(|(t, &w)| (t.clone(), w))
                .collect();
            // Expected return is unknown without return estimates; set to 0.
            let vol = sigma_p;
            return Ok(OptimalPortfolio {
                weights: named,
                expected_return: 0.0,
                volatility: vol,
                sharpe_ratio: 0.0,
            });
        }

        // Update: w_i *= (target_rc / rc_i)^damping, then renormalise.
        // Damping factor prevents oscillation with highly disparate vols.
        let damping = 0.5;
        for i in 0..n {
            if rc[i] > 1e-15 {
                let ratio = target_rc / rc[i];
                weights[i] *= ratio.powf(damping);
            }
        }

        // Renormalise.
        let sum: f64 = weights.iter().sum();
        if sum > 1e-15 {
            for w in &mut weights {
                *w /= sum;
            }
        }
    }

    Err(OptimizeError::SolverFailed(
        "risk parity did not converge within max iterations".into(),
    ))
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimize::mvo::portfolio_volatility;

    #[test]
    fn test_equal_vol_uncorrelated_gives_equal_weight() {
        // Identity covariance (all assets have vol=1, uncorrelated) => 1/N each.
        let n = 4;
        let cov = DMatrix::identity(n, n);
        let tickers: Vec<String> = (0..n).map(|i| format!("T{i}")).collect();
        let result = risk_parity(&cov, &tickers, 1e-8, 1000).expect("should converge");

        let expected = 1.0 / n as f64;
        for (ticker, w) in &result.weights {
            assert!(
                (w - expected).abs() < 1e-6,
                "Expected ~{expected} for {ticker}, got {w}"
            );
        }
    }

    #[test]
    fn test_higher_vol_gets_lower_weight() {
        // Asset A: vol=0.10, Asset B: vol=0.20 (uncorrelated).
        // Risk parity should give A roughly double the weight of B.
        let cov = DMatrix::from_row_slice(2, 2, &[0.01, 0.0, 0.0, 0.04]);
        let tickers = vec!["A".into(), "B".into()];
        let result = risk_parity(&cov, &tickers, 1e-10, 1000).expect("should converge");

        let w_a = result.weights[0].1;
        let w_b = result.weights[1].1;
        assert!(
            w_a > w_b,
            "Lower-vol asset A should get more weight: A={w_a}, B={w_b}"
        );
        // For uncorrelated assets, risk parity gives w proportional to 1/vol.
        // w_a/w_b should be approximately vol_b/vol_a = 0.20/0.10 = 2.0.
        let ratio = w_a / w_b;
        assert!(
            (ratio - 2.0).abs() < 0.1,
            "Weight ratio should be ~2.0, got {ratio}"
        );
    }

    #[test]
    fn test_weights_sum_to_one() {
        let cov = DMatrix::from_row_slice(
            3,
            3,
            &[0.04, 0.01, 0.005, 0.01, 0.09, 0.015, 0.005, 0.015, 0.0225],
        );
        let tickers = vec!["A".into(), "B".into(), "C".into()];
        let result = risk_parity(&cov, &tickers, 1e-8, 1000).expect("should converge");

        let sum: f64 = result.weights.iter().map(|(_, w)| w).sum();
        assert!(
            (sum - 1.0).abs() < 1e-6,
            "Weights should sum to 1.0, got {sum}"
        );
    }

    #[test]
    fn test_risk_contributions_equal() {
        let cov = DMatrix::from_row_slice(
            3,
            3,
            &[0.04, 0.006, 0.002, 0.006, 0.09, 0.009, 0.002, 0.009, 0.01],
        );
        let tickers = vec!["A".into(), "B".into(), "C".into()];
        let result = risk_parity(&cov, &tickers, 1e-10, 5000).expect("should converge");

        let n = tickers.len();
        let weights: Vec<f64> = result.weights.iter().map(|(_, w)| *w).collect();
        let w = DVector::from_column_slice(&weights);
        let sigma_w = &cov * &w;
        let sigma_p = portfolio_volatility(&weights, &cov);

        let rc: Vec<f64> = weights
            .iter()
            .zip(sigma_w.iter())
            .map(|(&wi, &sw)| wi * sw / sigma_p)
            .collect();

        let target = sigma_p / n as f64;
        for (i, &rci) in rc.iter().enumerate() {
            assert!(
                (rci - target).abs() < 1e-4,
                "RC[{i}] = {rci}, expected ~{target}"
            );
        }
    }

    #[test]
    fn test_convergence() {
        // Should converge within 1000 iterations for a reasonable input.
        let cov = DMatrix::from_row_slice(2, 2, &[0.04, 0.01, 0.01, 0.09]);
        let tickers = vec!["A".into(), "B".into()];
        let result = risk_parity(&cov, &tickers, 1e-8, 1000);
        assert!(result.is_ok(), "Should converge: {result:?}");
    }
}
