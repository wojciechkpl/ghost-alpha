//! Pairwise correlation analysis for portfolio concentration alerting.

use crate::error::AnalysisError;

/// Pearson correlation coefficient between two equal-length return series.
///
/// Returns a value in [-1.0, +1.0].
pub fn pearson_correlation(a: &[f64], b: &[f64]) -> Result<f64, AnalysisError> {
    if a.len() != b.len() {
        return Err(AnalysisError::InsufficientData {
            need: a.len(),
            got: b.len(),
        });
    }
    if a.len() < 3 {
        return Err(AnalysisError::InsufficientData {
            need: 3,
            got: a.len(),
        });
    }

    let n = a.len() as f64;
    let mean_a = a.iter().sum::<f64>() / n;
    let mean_b = b.iter().sum::<f64>() / n;

    let mut cov = 0.0;
    let mut var_a = 0.0;
    let mut var_b = 0.0;
    for (ai, bi) in a.iter().zip(b.iter()) {
        let da = ai - mean_a;
        let db = bi - mean_b;
        cov += da * db;
        var_a += da * da;
        var_b += db * db;
    }

    let denom = (var_a * var_b).sqrt();
    if denom < 1e-10 {
        return Ok(0.0);
    }

    Ok((cov / denom).clamp(-1.0, 1.0))
}

/// Check for correlation concentration: find warnings when 3+ stocks
/// have pairwise correlation > threshold.
///
/// Returns a list of warning messages for concentrated groups.
pub fn check_correlation_concentration(
    stock_returns: &[(&str, &[f64])],
    threshold: f64,
) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut _high_corr_count = 0usize;
    let mut correlated_tickers: Vec<&str> = Vec::new();

    for i in 0..stock_returns.len() {
        for j in (i + 1)..stock_returns.len() {
            let (ticker_a, returns_a) = stock_returns[i];
            let (ticker_b, returns_b) = stock_returns[j];

            // Skip if lengths don't match
            if returns_a.len() != returns_b.len() || returns_a.len() < 3 {
                continue;
            }

            if let Ok(corr) = pearson_correlation(returns_a, returns_b) {
                if corr > threshold {
                    _high_corr_count += 1;
                    if !correlated_tickers.contains(&ticker_a) {
                        correlated_tickers.push(ticker_a);
                    }
                    if !correlated_tickers.contains(&ticker_b) {
                        correlated_tickers.push(ticker_b);
                    }
                }
            }
        }
    }

    if correlated_tickers.len() >= 3 {
        warnings.push(format!(
            "Correlation concentration: {} stocks with pairwise r > {:.2} ({})",
            correlated_tickers.len(),
            threshold,
            correlated_tickers.join(", "),
        ));
    }

    warnings
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_series_correlation_is_one() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let r = pearson_correlation(&a, &a).unwrap();
        assert!(
            (r - 1.0).abs() < 1e-10,
            "Identical series should have r=1.0, got {r:.6}"
        );
    }

    #[test]
    fn inverse_series_correlation_is_negative_one() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let b = vec![5.0, 4.0, 3.0, 2.0, 1.0];
        let r = pearson_correlation(&a, &b).unwrap();
        assert!(
            (r + 1.0).abs() < 1e-10,
            "Inverse series should have r=-1.0, got {r:.6}"
        );
    }

    #[test]
    fn unequal_lengths_error() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0];
        assert!(matches!(
            pearson_correlation(&a, &b),
            Err(AnalysisError::InsufficientData { .. })
        ));
    }

    #[test]
    fn too_short_series_error() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0];
        assert!(matches!(
            pearson_correlation(&a, &b),
            Err(AnalysisError::InsufficientData { need: 3, got: 2 })
        ));
    }

    #[test]
    fn three_correlated_stocks_fires_warning() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let b = vec![1.1, 2.1, 3.1, 4.1, 5.1]; // near-identical
        let c = vec![1.2, 2.2, 3.2, 4.2, 5.2]; // near-identical
        let stocks = vec![
            ("NVDA", a.as_slice()),
            ("AMD", b.as_slice()),
            ("INTC", c.as_slice()),
        ];
        let warnings = check_correlation_concentration(&stocks, 0.7);
        assert!(
            !warnings.is_empty(),
            "3 highly correlated stocks should fire warning"
        );
    }

    #[test]
    fn uncorrelated_stocks_no_warning() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let b = vec![5.0, 3.0, 1.0, 4.0, 2.0]; // random
        let c = vec![2.0, 5.0, 1.0, 3.0, 4.0]; // random
        let stocks = vec![
            ("AAPL", a.as_slice()),
            ("XOM", b.as_slice()),
            ("JNJ", c.as_slice()),
        ];
        let warnings = check_correlation_concentration(&stocks, 0.7);
        assert!(
            warnings.is_empty(),
            "Uncorrelated stocks should not fire warning"
        );
    }
}
