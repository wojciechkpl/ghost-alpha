use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use crate::backtest::types::ConfidenceInterval;

/// Bootstrap a confidence interval for a single metric by resampling with replacement.
///
/// `values` is the observed sample. `iterations` is the number of bootstrap draws.
/// `confidence` is the interval width (e.g. 0.95 for 95% CI).
/// `seed` makes results deterministic for reproducibility.
pub fn bootstrap_ci(
    values: &[f64],
    iterations: u32,
    confidence: f64,
    metric_name: &str,
    seed: u64,
) -> ConfidenceInterval {
    if values.is_empty() {
        return ConfidenceInterval {
            metric_name: metric_name.to_string(),
            point_estimate: 0.0,
            ci_lower: 0.0,
            ci_upper: 0.0,
        };
    }

    if iterations == 0 {
        let point_estimate = values.iter().sum::<f64>() / values.len() as f64;
        return ConfidenceInterval {
            metric_name: metric_name.to_string(),
            point_estimate,
            ci_lower: 0.0,
            ci_upper: 0.0,
        };
    }

    let point_estimate = values.iter().sum::<f64>() / values.len() as f64;
    let mut rng = StdRng::seed_from_u64(seed);
    let n = values.len();

    let mut bootstrap_means: Vec<f64> = (0..iterations)
        .map(|_| {
            let sample_sum: f64 = (0..n).map(|_| values[rng.gen_range(0..n)]).sum();
            sample_sum / n as f64
        })
        .collect();

    bootstrap_means.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let alpha = 1.0 - confidence;
    let lower_idx = ((alpha / 2.0) * bootstrap_means.len() as f64) as usize;
    let upper_idx = ((1.0 - alpha / 2.0) * bootstrap_means.len() as f64) as usize;
    let upper_idx = upper_idx.min(bootstrap_means.len() - 1);

    ConfidenceInterval {
        metric_name: metric_name.to_string(),
        point_estimate,
        ci_lower: bootstrap_means[lower_idx],
        ci_upper: bootstrap_means[upper_idx],
    }
}

/// Compute CIs for key backtest metrics.
pub fn compute_all_cis(
    returns_60d: &[f64],
    hit_rate_values: &[f64],
    iterations: u32,
    confidence: f64,
    seed: u64,
) -> Vec<ConfidenceInterval> {
    vec![
        bootstrap_ci(returns_60d, iterations, confidence, "alpha_60d", seed),
        bootstrap_ci(
            hit_rate_values,
            iterations,
            confidence,
            "hit_rate_60d",
            seed + 1,
        ),
    ]
}

/// Check if a CI excludes a null value (convenience wrapper).
pub fn ci_rejects_null(ci: &ConfidenceInterval, null_value: f64) -> bool {
    ci.rejects_null(null_value)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_with_seed() {
        let values = vec![0.01, 0.02, -0.005, 0.03, 0.015, -0.01, 0.025, 0.04];
        let ci1 = bootstrap_ci(&values, 1000, 0.95, "test", 42);
        let ci2 = bootstrap_ci(&values, 1000, 0.95, "test", 42);
        assert_eq!(ci1.ci_lower, ci2.ci_lower);
        assert_eq!(ci1.ci_upper, ci2.ci_upper);
    }

    #[test]
    fn same_values_narrow_ci() {
        let values = vec![0.05; 100];
        let ci = bootstrap_ci(&values, 1000, 0.95, "constant", 42);
        let width = ci.ci_upper - ci.ci_lower;
        assert!(
            width < 1e-6,
            "CI for identical values should be near-zero width, got {}",
            width
        );
    }

    #[test]
    fn wide_for_high_variance() {
        let values: Vec<f64> = (0..100)
            .map(|i| if i % 2 == 0 { 0.20 } else { -0.15 })
            .collect();
        let ci = bootstrap_ci(&values, 5000, 0.95, "volatile", 42);
        let width = ci.ci_upper - ci.ci_lower;
        assert!(
            width > 0.01,
            "CI for high-variance data should be wide, got {}",
            width
        );
    }

    #[test]
    fn ci_95_excludes_tails() {
        // All positive returns — CI should not include large negative values
        let values: Vec<f64> = (0..200).map(|i| 0.05 + (i as f64) * 0.001).collect();
        let ci = bootstrap_ci(&values, 5000, 0.95, "positive", 42);
        assert!(
            ci.ci_lower > 0.0,
            "95% CI for all-positive data should exclude zero, lower={}",
            ci.ci_lower
        );
    }

    #[test]
    fn rejects_null_above() {
        let ci = ConfidenceInterval {
            metric_name: "alpha".to_string(),
            point_estimate: 0.05,
            ci_lower: 0.02,
            ci_upper: 0.08,
        };
        assert!(
            ci_rejects_null(&ci, 0.0),
            "CI [0.02, 0.08] should reject null=0.0"
        );
    }

    #[test]
    fn straddles_does_not_reject() {
        let ci = ConfidenceInterval {
            metric_name: "alpha".to_string(),
            point_estimate: 0.01,
            ci_lower: -0.02,
            ci_upper: 0.04,
        };
        assert!(
            !ci_rejects_null(&ci, 0.0),
            "CI [-0.02, 0.04] should NOT reject null=0.0"
        );
    }

    #[test]
    fn bootstrap_ci_zero_iterations_does_not_panic() {
        // Given: a non-empty value slice but zero iterations
        let values = vec![0.01, 0.02, 0.03];

        // When: bootstrap_ci is called with iterations == 0
        let ci = bootstrap_ci(&values, 0, 0.95, "metric", 42);

        // Then: returns a zeroed CI without panicking; point estimate is the mean
        let expected_mean = (0.01 + 0.02 + 0.03) / 3.0;
        assert!(
            (ci.point_estimate - expected_mean).abs() < 1e-9,
            "point_estimate should be the sample mean, got {}",
            ci.point_estimate
        );
        assert_eq!(ci.ci_lower, 0.0);
        assert_eq!(ci.ci_upper, 0.0);
    }

    #[test]
    fn bootstrap_ci_empty_values_returns_zero_ci() {
        // Given: an empty input slice
        let values: Vec<f64> = vec![];

        // When
        let ci = bootstrap_ci(&values, 1000, 0.95, "empty_metric", 42);

        // Then: all fields are zero and the name is preserved
        assert_eq!(ci.metric_name, "empty_metric");
        assert_eq!(ci.point_estimate, 0.0);
        assert_eq!(ci.ci_lower, 0.0);
        assert_eq!(ci.ci_upper, 0.0);
    }

    #[test]
    fn compute_all_cis_empty_returns_gives_zero_estimates() {
        // Given: empty slices for both metrics
        let returns: Vec<f64> = vec![];
        let hit_rates: Vec<f64> = vec![];

        // When
        let cis = compute_all_cis(&returns, &hit_rates, 1000, 0.95, 42);

        // Then: two CIs are returned and all estimates are zero
        assert_eq!(cis.len(), 2);
        for ci in &cis {
            assert_eq!(
                ci.point_estimate, 0.0,
                "point_estimate for {} should be 0.0, got {}",
                ci.metric_name, ci.point_estimate
            );
            assert_eq!(ci.ci_lower, 0.0);
            assert_eq!(ci.ci_upper, 0.0);
        }
    }
}
