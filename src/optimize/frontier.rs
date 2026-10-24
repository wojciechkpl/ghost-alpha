//! Efficient frontier computation by sweeping target returns.

use crate::error::OptimizeError;
use crate::optimize::mvo::{minimum_variance, MvoInputs, OptimalPortfolio};

/// Compute the efficient frontier as a series of optimal portfolios.
///
/// Sweeps target returns from `min(expected_returns)` to `max(expected_returns)`
/// in `points` evenly spaced steps, solving the minimum-variance problem at each.
pub fn efficient_frontier(
    inputs: &MvoInputs,
    risk_free_rate: f64,
    points: usize,
) -> Result<Vec<OptimalPortfolio>, OptimizeError> {
    if points == 0 {
        return Ok(Vec::new());
    }

    let min_ret = inputs
        .expected_returns
        .iter()
        .copied()
        .fold(f64::INFINITY, f64::min);
    let max_ret = inputs
        .expected_returns
        .iter()
        .copied()
        .fold(f64::NEG_INFINITY, f64::max);

    if points == 1 {
        let portfolio = minimum_variance(inputs, (min_ret + max_ret) / 2.0, risk_free_rate)?;
        return Ok(vec![portfolio]);
    }

    let step = (max_ret - min_ret) / (points - 1) as f64;

    (0..points)
        .map(|i| {
            let target = min_ret + step * i as f64;
            minimum_variance(inputs, target, risk_free_rate)
        })
        .collect()
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::DMatrix;

    fn sample_inputs() -> MvoInputs {
        let cov = DMatrix::from_row_slice(
            3,
            3,
            &[0.04, 0.006, 0.002, 0.006, 0.09, 0.009, 0.002, 0.009, 0.01],
        );
        MvoInputs {
            tickers: vec!["A".into(), "B".into(), "C".into()],
            expected_returns: vec![0.05, 0.15, 0.03],
            covariance: cov,
            min_weights: vec![0.0, 0.0, 0.0],
            max_weights: vec![1.0, 1.0, 1.0],
        }
    }

    #[test]
    fn test_frontier_returns_correct_point_count() {
        let inputs = sample_inputs();
        let frontier = efficient_frontier(&inputs, 0.02, 20).expect("should succeed");
        assert_eq!(
            frontier.len(),
            20,
            "Expected 20 frontier points, got {}",
            frontier.len()
        );
    }

    #[test]
    fn test_frontier_returns_monotonically_increase() {
        let inputs = sample_inputs();
        let frontier = efficient_frontier(&inputs, 0.02, 20).expect("should succeed");
        for i in 1..frontier.len() {
            assert!(
                frontier[i].expected_return >= frontier[i - 1].expected_return - 1e-8,
                "Frontier returns should increase: point {} ({}) < point {} ({})",
                i,
                frontier[i].expected_return,
                i - 1,
                frontier[i - 1].expected_return,
            );
        }
    }

    #[test]
    fn test_frontier_all_valid() {
        let inputs = sample_inputs();
        let frontier = efficient_frontier(&inputs, 0.02, 10).expect("should succeed");
        for (idx, portfolio) in frontier.iter().enumerate() {
            let sum: f64 = portfolio.weights.iter().map(|(_, w)| w).sum();
            assert!(
                (sum - 1.0).abs() < 1e-4,
                "Point {idx}: weights sum to {sum}, expected 1.0"
            );
            for (ticker, w) in &portfolio.weights {
                assert!(
                    *w >= -1e-6,
                    "Point {idx}: weight for {ticker} is negative: {w}"
                );
            }
        }
    }

    #[test]
    fn test_frontier_zero_points() {
        let inputs = sample_inputs();
        let frontier = efficient_frontier(&inputs, 0.02, 0).expect("should succeed");
        assert!(frontier.is_empty());
    }

    #[test]
    fn test_frontier_single_point() {
        let inputs = sample_inputs();
        let frontier = efficient_frontier(&inputs, 0.02, 1).expect("should succeed");
        assert_eq!(frontier.len(), 1);
    }
}
