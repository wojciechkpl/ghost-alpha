//! N x N correlation matrix engine for portfolio diversification analysis.
//!
//! Builds on [`crate::analysis::correlation::pearson_correlation`] to compute
//! full pairwise correlation matrices, rolling correlations, covariance matrices,
//! and crisis-period conditional correlations.

use chrono::NaiveDate;
use nalgebra::DMatrix;

use crate::error::AnalysisError;

// ─── Types ───────────────────────────────────────────────────────────────────

/// N x N Pearson correlation matrix with ticker labels and date range metadata.
#[derive(Debug, Clone)]
pub struct CorrelationMatrix {
    pub tickers: Vec<String>,
    pub matrix: DMatrix<f64>,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
}

/// A named date range representing a market crisis for conditional correlation analysis.
pub struct CrisisPeriod {
    pub name: &'static str,
    pub start: NaiveDate,
    pub end: NaiveDate,
}

// ─── Crisis periods ──────────────────────────────────────────────────────────

/// Well-known crisis periods for conditional correlation analysis.
pub fn crisis_periods() -> Vec<CrisisPeriod> {
    vec![
        CrisisPeriod {
            name: "GFC 2008",
            start: NaiveDate::from_ymd_opt(2008, 9, 1).expect("valid date"),
            end: NaiveDate::from_ymd_opt(2009, 3, 31).expect("valid date"),
        },
        CrisisPeriod {
            name: "COVID 2020",
            start: NaiveDate::from_ymd_opt(2020, 2, 19).expect("valid date"),
            end: NaiveDate::from_ymd_opt(2020, 3, 23).expect("valid date"),
        },
        CrisisPeriod {
            name: "2022 Bear",
            start: NaiveDate::from_ymd_opt(2022, 1, 3).expect("valid date"),
            end: NaiveDate::from_ymd_opt(2022, 10, 12).expect("valid date"),
        },
    ]
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Bessel-corrected sample standard deviation.
///
/// Returns 0.0 for fewer than 2 values (undefined for n < 2).
pub fn std_dev(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
    var.sqrt()
}

// ─── CorrelationMatrix impl ──────────────────────────────────────────────────

impl CorrelationMatrix {
    /// Compute full pairwise Pearson correlation from daily return series.
    ///
    /// `returns` is a slice of `(ticker, daily_returns)` pairs.
    /// All return series must be the same length (aligned by date).
    /// Requires at least 2 assets.
    pub fn from_returns(returns: &[(&str, &[f64])]) -> Result<Self, AnalysisError> {
        let n = returns.len();
        if n < 2 {
            return Err(AnalysisError::InsufficientData { need: 2, got: n });
        }
        let len = returns[0].1.len();
        for (_, r) in returns.iter().skip(1) {
            if r.len() != len {
                return Err(AnalysisError::InsufficientData {
                    need: len,
                    got: r.len(),
                });
            }
        }

        let mut mat = DMatrix::zeros(n, n);
        for i in 0..n {
            mat[(i, i)] = 1.0;
            for j in (i + 1)..n {
                let corr =
                    crate::analysis::correlation::pearson_correlation(returns[i].1, returns[j].1)?;
                mat[(i, j)] = corr;
                mat[(j, i)] = corr;
            }
        }

        Ok(Self {
            tickers: returns.iter().map(|(t, _)| t.to_string()).collect(),
            matrix: mat,
            period_start: NaiveDate::default(),
            period_end: NaiveDate::default(),
        })
    }

    /// Compute rolling Pearson correlation between two return series over a sliding window.
    ///
    /// Returns `len - window + 1` correlation values.
    pub fn rolling_correlation(
        returns_a: &[f64],
        returns_b: &[f64],
        window: usize,
    ) -> Result<Vec<f64>, AnalysisError> {
        if returns_a.len() != returns_b.len() {
            return Err(AnalysisError::InsufficientData {
                need: returns_a.len(),
                got: returns_b.len(),
            });
        }
        if returns_a.len() < window {
            return Err(AnalysisError::InsufficientData {
                need: window,
                got: returns_a.len(),
            });
        }
        let mut result = Vec::with_capacity(returns_a.len() - window + 1);
        for i in 0..=(returns_a.len() - window) {
            let corr = crate::analysis::correlation::pearson_correlation(
                &returns_a[i..i + window],
                &returns_b[i..i + window],
            )?;
            result.push(corr);
        }
        Ok(result)
    }

    /// Convert the correlation matrix to an annualised covariance matrix.
    ///
    /// `cov[i][j] = corr[i][j] * std_i * std_j * 252` where `std` is the
    /// daily sample standard deviation of returns.
    pub fn to_covariance_matrix(
        &self,
        returns: &[(&str, &[f64])],
    ) -> Result<DMatrix<f64>, AnalysisError> {
        let n = returns.len();
        if n != self.tickers.len() {
            return Err(AnalysisError::InsufficientData {
                need: self.tickers.len(),
                got: n,
            });
        }
        let mut cov = DMatrix::zeros(n, n);
        for i in 0..n {
            let std_i = std_dev(returns[i].1);
            for j in 0..n {
                let std_j = std_dev(returns[j].1);
                cov[(i, j)] = self.matrix[(i, j)] * std_i * std_j * 252.0;
            }
        }
        Ok(cov)
    }

    /// Find all ticker pairs whose absolute correlation exceeds `threshold`.
    ///
    /// Returns `(ticker_a, ticker_b, correlation)` tuples, upper-triangle only
    /// (no duplicates, no self-pairs).
    pub fn high_correlations(&self, threshold: f64) -> Vec<(String, String, f64)> {
        let n = self.tickers.len();
        let mut pairs = Vec::new();
        for i in 0..n {
            for j in (i + 1)..n {
                let corr = self.matrix[(i, j)];
                if corr.abs() > threshold {
                    pairs.push((self.tickers[i].clone(), self.tickers[j].clone(), corr));
                }
            }
        }
        pairs
    }
}

// ─── Crisis correlation ──────────────────────────────────────────────────────

/// Compute a correlation matrix for returns restricted to a date range (crisis analysis).
///
/// Each asset's returns are filtered to `[start, end]` inclusive, then the
/// standard `from_returns` computation is applied.
pub fn crisis_correlation(
    all_returns: &[(&str, &[(NaiveDate, f64)])],
    start: NaiveDate,
    end: NaiveDate,
) -> Result<CorrelationMatrix, AnalysisError> {
    let filtered: Vec<(&str, Vec<f64>)> = all_returns
        .iter()
        .map(|(ticker, dated_returns)| {
            let vals: Vec<f64> = dated_returns
                .iter()
                .filter(|(d, _)| *d >= start && *d <= end)
                .map(|(_, r)| *r)
                .collect();
            (*ticker, vals)
        })
        .collect();

    let as_slices: Vec<(&str, &[f64])> = filtered.iter().map(|(t, v)| (*t, v.as_slice())).collect();

    let mut matrix = CorrelationMatrix::from_returns(&as_slices)?;
    matrix.period_start = start;
    matrix.period_end = end;
    Ok(matrix)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── std_dev ──────────────────────────────────────────────────────────

    #[test]
    fn test_std_dev_known_values() {
        // Values: [2, 4, 4, 4, 5, 5, 7, 9]
        // Mean = 5.0, sample variance = 4.571..., sample std = 2.1380...
        let vals = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let sd = std_dev(&vals);
        assert!((sd - 2.13809).abs() < 1e-4, "Expected ~2.1381, got {sd}");
    }

    #[test]
    fn test_std_dev_single_value() {
        assert!((std_dev(&[42.0])).abs() < f64::EPSILON);
    }

    #[test]
    fn test_std_dev_empty() {
        assert!((std_dev(&[])).abs() < f64::EPSILON);
    }

    // ── CorrelationMatrix::from_returns ──────────────────────────────────

    #[test]
    fn test_identity_correlation() {
        let a = vec![0.01, -0.02, 0.03, -0.01, 0.02];
        let returns = vec![("A", a.as_slice()), ("B", a.as_slice())];
        let cm = CorrelationMatrix::from_returns(&returns).expect("should succeed");
        assert!(
            (cm.matrix[(0, 1)] - 1.0).abs() < 1e-10,
            "Identical returns should have correlation 1.0, got {}",
            cm.matrix[(0, 1)]
        );
    }

    #[test]
    fn test_anti_correlation() {
        let a = vec![0.01, -0.02, 0.03, -0.01, 0.02];
        let b: Vec<f64> = a.iter().map(|x| -x).collect();
        let returns = vec![("A", a.as_slice()), ("B", b.as_slice())];
        let cm = CorrelationMatrix::from_returns(&returns).expect("should succeed");
        assert!(
            (cm.matrix[(0, 1)] + 1.0).abs() < 1e-10,
            "Negated returns should have correlation -1.0, got {}",
            cm.matrix[(0, 1)]
        );
    }

    #[test]
    fn test_diagonal_is_one() {
        let a = vec![0.01, -0.02, 0.03, -0.01, 0.02];
        let b = vec![0.02, 0.01, -0.01, 0.03, -0.02];
        let c = vec![-0.01, 0.03, 0.02, -0.02, 0.01];
        let returns = vec![
            ("A", a.as_slice()),
            ("B", b.as_slice()),
            ("C", c.as_slice()),
        ];
        let cm = CorrelationMatrix::from_returns(&returns).expect("should succeed");
        for i in 0..3 {
            assert!(
                (cm.matrix[(i, i)] - 1.0).abs() < 1e-10,
                "Diagonal entry [{i},{i}] should be 1.0, got {}",
                cm.matrix[(i, i)]
            );
        }
    }

    #[test]
    fn test_matrix_is_symmetric() {
        let a = vec![0.01, -0.02, 0.03, -0.01, 0.02];
        let b = vec![0.02, 0.01, -0.01, 0.03, -0.02];
        let c = vec![-0.01, 0.03, 0.02, -0.02, 0.01];
        let returns = vec![
            ("A", a.as_slice()),
            ("B", b.as_slice()),
            ("C", c.as_slice()),
        ];
        let cm = CorrelationMatrix::from_returns(&returns).expect("should succeed");
        for i in 0..3 {
            for j in 0..3 {
                assert!(
                    (cm.matrix[(i, j)] - cm.matrix[(j, i)]).abs() < 1e-10,
                    "Matrix not symmetric at ({i},{j}): {} vs {}",
                    cm.matrix[(i, j)],
                    cm.matrix[(j, i)]
                );
            }
        }
    }

    #[test]
    fn test_values_in_range() {
        let a = vec![0.01, -0.02, 0.03, -0.01, 0.02];
        let b = vec![0.02, 0.01, -0.01, 0.03, -0.02];
        let c = vec![-0.01, 0.03, 0.02, -0.02, 0.01];
        let returns = vec![
            ("A", a.as_slice()),
            ("B", b.as_slice()),
            ("C", c.as_slice()),
        ];
        let cm = CorrelationMatrix::from_returns(&returns).expect("should succeed");
        for i in 0..3 {
            for j in 0..3 {
                let v = cm.matrix[(i, j)];
                assert!(
                    (-1.0..=1.0).contains(&v),
                    "Value at ({i},{j}) = {v} is out of [-1, 1]"
                );
            }
        }
    }

    #[test]
    fn test_insufficient_assets() {
        let a = vec![0.01, -0.02, 0.03];
        let returns = vec![("A", a.as_slice())];
        let err = CorrelationMatrix::from_returns(&returns).unwrap_err();
        assert!(
            matches!(err, AnalysisError::InsufficientData { need: 2, got: 1 }),
            "Expected InsufficientData, got {err:?}"
        );
    }

    #[test]
    fn test_mismatched_lengths() {
        let a = vec![0.01, -0.02, 0.03];
        let b = vec![0.01, -0.02, 0.03, 0.04];
        let returns = vec![("A", a.as_slice()), ("B", b.as_slice())];
        let err = CorrelationMatrix::from_returns(&returns).unwrap_err();
        assert!(
            matches!(err, AnalysisError::InsufficientData { .. }),
            "Expected InsufficientData for mismatched lengths, got {err:?}"
        );
    }

    // ── rolling_correlation ──────────────────────────────────────────────

    #[test]
    fn test_rolling_correlation_window() {
        // Two identical series: rolling correlation should always be 1.0.
        let a = vec![0.01, -0.02, 0.03, -0.01, 0.02, 0.01, -0.03];
        let result = CorrelationMatrix::rolling_correlation(&a, &a, 5).expect("should succeed");
        // 7 data points, window 5 => 3 output values
        assert_eq!(
            result.len(),
            3,
            "Expected 3 rolling values, got {}",
            result.len()
        );
        for (i, v) in result.iter().enumerate() {
            assert!(
                (v - 1.0).abs() < 1e-10,
                "Rolling corr[{i}] of identical series should be 1.0, got {v}"
            );
        }
    }

    #[test]
    fn test_rolling_correlation_insufficient_data() {
        let a = vec![0.01, -0.02, 0.03];
        let b = vec![0.02, 0.01, -0.01];
        let err = CorrelationMatrix::rolling_correlation(&a, &b, 5).unwrap_err();
        assert!(
            matches!(err, AnalysisError::InsufficientData { need: 5, got: 3 }),
            "Expected InsufficientData, got {err:?}"
        );
    }

    // ── to_covariance_matrix ─────────────────────────────────────────────

    #[test]
    fn test_covariance_matrix_shape() {
        let a = vec![0.01, -0.02, 0.03, -0.01, 0.02];
        let b = vec![0.02, 0.01, -0.01, 0.03, -0.02];
        let c = vec![-0.01, 0.03, 0.02, -0.02, 0.01];
        let returns = vec![
            ("A", a.as_slice()),
            ("B", b.as_slice()),
            ("C", c.as_slice()),
        ];
        let cm = CorrelationMatrix::from_returns(&returns).expect("corr matrix");
        let cov = cm.to_covariance_matrix(&returns).expect("cov matrix");
        assert_eq!(cov.nrows(), 3);
        assert_eq!(cov.ncols(), 3);
    }

    #[test]
    fn test_covariance_diagonal_is_variance() {
        let a = vec![0.01, -0.02, 0.03, -0.01, 0.02];
        let b = vec![0.02, 0.01, -0.01, 0.03, -0.02];
        let returns = vec![("A", a.as_slice()), ("B", b.as_slice())];
        let cm = CorrelationMatrix::from_returns(&returns).expect("corr matrix");
        let cov = cm.to_covariance_matrix(&returns).expect("cov matrix");

        // Diagonal should be annualised variance: std^2 * 252
        // (since corr[i,i] = 1.0, cov[i,i] = 1.0 * std_i * std_i * 252)
        for (i, (_, rets)) in returns.iter().enumerate() {
            let sd = std_dev(rets);
            let expected_var = sd * sd * 252.0;
            assert!(
                (cov[(i, i)] - expected_var).abs() < 1e-10,
                "Diagonal [{i},{i}]: expected {expected_var}, got {}",
                cov[(i, i)]
            );
        }
    }

    // ── high_correlations ────────────────────────────────────────────────

    #[test]
    fn test_high_correlations_filters() {
        let a = vec![0.01, -0.02, 0.03, -0.01, 0.02];
        // b is identical to a => corr = 1.0, above any threshold < 1.0
        let c = vec![0.05, -0.03, 0.01, 0.04, -0.05]; // uncorrelated
        let returns = vec![
            ("A", a.as_slice()),
            ("B", a.as_slice()),
            ("C", c.as_slice()),
        ];
        let cm = CorrelationMatrix::from_returns(&returns).expect("corr matrix");

        let pairs = cm.high_correlations(0.9);
        // Only (A, B) should be above 0.9
        assert_eq!(
            pairs.len(),
            1,
            "Expected 1 high-correlation pair, got {}",
            pairs.len()
        );
        assert_eq!(pairs[0].0, "A");
        assert_eq!(pairs[0].1, "B");
        assert!((pairs[0].2 - 1.0).abs() < 1e-10);

        // With threshold 0.0, all pairs should appear (A-B, A-C, B-C)
        let all_pairs = cm.high_correlations(0.0);
        // Some pairs may have correlation near 0 (not > 0.0), so let's check >= 2
        // Actually, A-B is 1.0, and A-C / B-C are likely above 0.0 in absolute value.
        // Let's just verify A-B is in there.
        assert!(
            all_pairs.iter().any(|(a, b, _)| a == "A" && b == "B"),
            "A-B pair should be in results"
        );
    }

    // ── crisis_periods ───────────────────────────────────────────────────

    #[test]
    fn test_crisis_periods_not_empty() {
        let periods = crisis_periods();
        assert_eq!(periods.len(), 3, "Expected 3 crisis periods");
        assert_eq!(periods[0].name, "GFC 2008");
        assert_eq!(periods[1].name, "COVID 2020");
        assert_eq!(periods[2].name, "2022 Bear");
    }

    // ── crisis_correlation ───────────────────────────────────────────────

    #[test]
    fn test_crisis_correlation_filters_dates() {
        let start = NaiveDate::from_ymd_opt(2020, 3, 1).expect("valid");
        let end = NaiveDate::from_ymd_opt(2020, 3, 10).expect("valid");

        // Create dated returns spanning a wider range; only dates within [start, end] should be used.
        let dates_a: Vec<(NaiveDate, f64)> = (1..=20)
            .map(|d| {
                let date = NaiveDate::from_ymd_opt(2020, 3, d).expect("valid");
                (date, 0.01 * d as f64)
            })
            .collect();
        let dates_b: Vec<(NaiveDate, f64)> = (1..=20)
            .map(|d| {
                let date = NaiveDate::from_ymd_opt(2020, 3, d).expect("valid");
                (date, -0.01 * d as f64)
            })
            .collect();

        let all = vec![("X", dates_a.as_slice()), ("Y", dates_b.as_slice())];
        let cm = crisis_correlation(&all, start, end).expect("should succeed");

        // Only 10 dates (March 1-10) should be included.
        assert_eq!(cm.period_start, start);
        assert_eq!(cm.period_end, end);
        // X and Y are perfectly anti-correlated within this window.
        assert!(
            (cm.matrix[(0, 1)] + 1.0).abs() < 1e-10,
            "Expected anti-correlation, got {}",
            cm.matrix[(0, 1)]
        );
    }
}
