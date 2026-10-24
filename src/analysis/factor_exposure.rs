//! Portfolio factor exposure monitoring.
//!
//! Phase 8 (Wave 2): Measures and alerts on portfolio-level risk factors:
//! market beta, sector concentration, correlation clustering, and style tilts.
//! Institutional funds monitor these daily to prevent hidden concentration risks.

use crate::analysis::correlation::pearson_correlation;

// ─── Types ──────────────────────────────────────────────────────────────────

/// Portfolio factor exposure decomposition.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FactorExposure {
    pub market_beta: f64,
    /// Herfindahl-Hirschman index of sector weights (1/N = diversified, 1.0 = one sector)
    pub sector_hhi: f64,
    pub top_sector_weight: f64,
    pub top_sector_name: String,
    /// Average pairwise correlation among holdings
    pub avg_correlation: f64,
    /// Average RS percentile of holdings (momentum tilt)
    pub momentum_tilt: f64,
    /// Average ATR as % of price (volatility tilt)
    pub volatility_tilt: f64,
}

/// A single factor alert.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FactorAlert {
    pub factor: String,
    pub current: f64,
    pub threshold: f64,
    pub severity: FactorSeverity,
    pub message: String,
}

/// Alert severity level.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum FactorSeverity {
    Info,
    Warning,
    Critical,
}

/// Configurable thresholds for factor alerts.
#[derive(Debug, Clone)]
pub struct FactorThresholds {
    pub max_beta: f64,
    pub max_sector_weight: f64,
    pub max_sector_hhi: f64,
    pub max_avg_correlation: f64,
    pub max_volatility_tilt: f64,
}

impl Default for FactorThresholds {
    fn default() -> Self {
        Self {
            max_beta: 1.3,
            max_sector_weight: 0.50,
            max_sector_hhi: 0.30,
            max_avg_correlation: 0.65,
            max_volatility_tilt: 4.0,
        }
    }
}

// ─── Position Input ─────────────────────────────────────────────────────────

/// Input for a single position's factor data.
#[derive(Debug, Clone)]
pub struct PositionFactorData {
    pub ticker: String,
    pub weight: f64,
    pub sector: String,
    pub rs_percentile: f64,
    pub atr_pct: f64,
    pub daily_returns: Vec<f64>,
}

// ─── Core Computation ───────────────────────────────────────────────────────

/// Compute factor exposures for the current portfolio.
pub fn compute_factor_exposure(
    positions: &[PositionFactorData],
    spy_returns: &[f64],
) -> FactorExposure {
    if positions.is_empty() {
        return FactorExposure {
            market_beta: 0.0,
            sector_hhi: 0.0,
            top_sector_weight: 0.0,
            top_sector_name: String::new(),
            avg_correlation: 0.0,
            momentum_tilt: 0.0,
            volatility_tilt: 0.0,
        };
    }

    // Market beta: weighted average of position betas
    let portfolio_returns = compute_portfolio_returns(positions);
    let market_beta = compute_beta(&portfolio_returns, spy_returns);

    // Sector concentration
    let (sector_hhi, top_sector_weight, top_sector_name) = compute_sector_concentration(positions);

    // Correlation concentration: average pairwise
    let avg_correlation = compute_avg_pairwise_correlation(positions);

    // Momentum tilt: weighted average RS percentile
    let total_weight: f64 = positions.iter().map(|p| p.weight).sum();
    let momentum_tilt = if total_weight > 0.0 {
        positions
            .iter()
            .map(|p| p.rs_percentile * p.weight)
            .sum::<f64>()
            / total_weight
    } else {
        0.0
    };

    // Volatility tilt: weighted average ATR%
    let volatility_tilt = if total_weight > 0.0 {
        positions.iter().map(|p| p.atr_pct * p.weight).sum::<f64>() / total_weight
    } else {
        0.0
    };

    FactorExposure {
        market_beta,
        sector_hhi,
        top_sector_weight,
        top_sector_name,
        avg_correlation,
        momentum_tilt,
        volatility_tilt,
    }
}

/// Generate alerts for factors exceeding thresholds.
pub fn check_factor_alerts(
    exposure: &FactorExposure,
    thresholds: &FactorThresholds,
) -> Vec<FactorAlert> {
    let mut alerts = Vec::new();

    let sector_msg = format!(
        "Top sector ({}) exceeds {:.0}% threshold",
        exposure.top_sector_name,
        thresholds.max_sector_weight * 100.0
    );

    let checks: Vec<(&str, f64, f64, &str)> = vec![
        (
            "Market Beta",
            exposure.market_beta,
            thresholds.max_beta,
            "Portfolio beta is high — correlated with broad market",
        ),
        (
            "Sector Concentration",
            exposure.top_sector_weight,
            thresholds.max_sector_weight,
            &sector_msg,
        ),
        (
            "Sector HHI",
            exposure.sector_hhi,
            thresholds.max_sector_hhi,
            "Sector diversification is poor (high HHI)",
        ),
        (
            "Avg Correlation",
            exposure.avg_correlation,
            thresholds.max_avg_correlation,
            "Holdings are highly correlated — diversification breakdown risk",
        ),
        (
            "Volatility Tilt",
            exposure.volatility_tilt,
            thresholds.max_volatility_tilt,
            "Portfolio tilted toward high-volatility names",
        ),
    ];

    for (factor, current, threshold, message) in checks {
        if current > threshold * 1.10 {
            alerts.push(FactorAlert {
                factor: factor.to_string(),
                current,
                threshold,
                severity: FactorSeverity::Critical,
                message: message.to_string(),
            });
        } else if current > threshold {
            alerts.push(FactorAlert {
                factor: factor.to_string(),
                current,
                threshold,
                severity: FactorSeverity::Warning,
                message: message.to_string(),
            });
        } else if current > threshold * 0.90 {
            alerts.push(FactorAlert {
                factor: factor.to_string(),
                current,
                threshold,
                severity: FactorSeverity::Info,
                message: format!("{factor} approaching threshold ({current:.2} vs {threshold:.2})"),
            });
        }
    }

    alerts
}

// ─── Helper Functions ───────────────────────────────────────────────────────

fn compute_portfolio_returns(positions: &[PositionFactorData]) -> Vec<f64> {
    if positions.is_empty() {
        return vec![];
    }
    let min_len = positions
        .iter()
        .map(|p| p.daily_returns.len())
        .min()
        .unwrap_or(0);
    if min_len == 0 {
        return vec![];
    }

    (0..min_len)
        .map(|t| {
            positions
                .iter()
                .map(|p| p.weight * p.daily_returns[t])
                .sum()
        })
        .collect()
}

fn compute_beta(portfolio_returns: &[f64], benchmark_returns: &[f64]) -> f64 {
    let len = portfolio_returns.len().min(benchmark_returns.len());
    if len < 20 {
        return 1.0; // default to market beta with insufficient data
    }

    let p = &portfolio_returns[portfolio_returns.len() - len..];
    let b = &benchmark_returns[benchmark_returns.len() - len..];

    let b_mean = b.iter().sum::<f64>() / len as f64;
    let p_mean = p.iter().sum::<f64>() / len as f64;

    let cov: f64 = p
        .iter()
        .zip(b.iter())
        .map(|(pi, bi)| (pi - p_mean) * (bi - b_mean))
        .sum::<f64>()
        / (len - 1) as f64;

    let var_b: f64 = b.iter().map(|bi| (bi - b_mean).powi(2)).sum::<f64>() / (len - 1) as f64;

    if var_b < 1e-12 {
        return 1.0;
    }

    cov / var_b
}

fn compute_sector_concentration(positions: &[PositionFactorData]) -> (f64, f64, String) {
    use std::collections::HashMap;

    let mut sector_weights: HashMap<&str, f64> = HashMap::new();
    for p in positions {
        *sector_weights.entry(p.sector.as_str()).or_default() += p.weight;
    }

    let hhi: f64 = sector_weights.values().map(|w| w * w).sum();
    let (top_name, top_weight) = sector_weights
        .iter()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(name, weight)| (name.to_string(), *weight))
        .unwrap_or_default();

    (hhi, top_weight, top_name)
}

fn compute_avg_pairwise_correlation(positions: &[PositionFactorData]) -> f64 {
    if positions.len() < 2 {
        return 0.0;
    }

    let mut total = 0.0;
    let mut count = 0usize;

    for i in 0..positions.len() {
        for j in (i + 1)..positions.len() {
            let a = &positions[i].daily_returns;
            let b = &positions[j].daily_returns;
            let len = a.len().min(b.len());
            if len >= 20 {
                let a_tail = &a[a.len() - len..];
                let b_tail = &b[b.len() - len..];
                if let Ok(corr) = pearson_correlation(a_tail, b_tail) {
                    total += corr;
                    count += 1;
                }
            }
        }
    }

    if count == 0 {
        0.0
    } else {
        total / count as f64
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn spy_returns() -> Vec<f64> {
        (0..60)
            .map(|i| 0.001 + 0.005 * ((i as f64 * 0.3).sin()))
            .collect()
    }

    fn make_position(
        ticker: &str,
        sector: &str,
        weight: f64,
        rs: f64,
        atr_pct: f64,
        beta_approx: f64,
    ) -> PositionFactorData {
        let spy = spy_returns();
        let returns: Vec<f64> = spy
            .iter()
            .map(|r| r * beta_approx + 0.0005 * (ticker.len() as f64))
            .collect();
        PositionFactorData {
            ticker: ticker.to_string(),
            weight,
            sector: sector.to_string(),
            rs_percentile: rs,
            atr_pct,
            daily_returns: returns,
        }
    }

    fn tech_heavy_portfolio() -> Vec<PositionFactorData> {
        vec![
            make_position("NVDA", "Technology", 0.20, 95.0, 3.5, 1.5),
            make_position("GOOGL", "Technology", 0.15, 85.0, 2.8, 1.1),
            make_position("AMD", "Technology", 0.15, 90.0, 4.0, 1.6),
            make_position("CRWD", "Technology", 0.12, 88.0, 3.2, 1.3),
            make_position("JPM", "Financials", 0.10, 70.0, 2.0, 1.0),
            make_position("UNH", "Healthcare", 0.10, 75.0, 1.8, 0.8),
            make_position("XOM", "Energy", 0.08, 60.0, 2.5, 0.7),
            make_position("PG", "Consumer", 0.10, 50.0, 1.2, 0.5),
        ]
    }

    fn diversified_portfolio() -> Vec<PositionFactorData> {
        vec![
            make_position("NVDA", "Technology", 0.125, 90.0, 3.5, 1.3),
            make_position("JPM", "Financials", 0.125, 70.0, 2.0, 1.0),
            make_position("UNH", "Healthcare", 0.125, 75.0, 1.8, 0.8),
            make_position("XOM", "Energy", 0.125, 60.0, 2.5, 0.9),
            make_position("PG", "Consumer", 0.125, 50.0, 1.2, 0.5),
            make_position("NEE", "Utilities", 0.125, 55.0, 1.5, 0.4),
            make_position("AMT", "RealEstate", 0.125, 65.0, 1.9, 0.6),
            make_position("GLD", "Commodities", 0.125, 45.0, 1.0, 0.1),
        ]
    }

    // ── beta ────────────────────────────────────────────────────────────

    #[test]
    fn beta_of_spy_is_near_1() {
        let spy = spy_returns();
        let beta = compute_beta(&spy, &spy);
        assert!(
            (beta - 1.0).abs() < 0.01,
            "SPY vs SPY beta should be 1.0, got {beta}"
        );
    }

    #[test]
    fn beta_of_leveraged_above_1() {
        let spy = spy_returns();
        let leveraged: Vec<f64> = spy.iter().map(|r| r * 2.0).collect();
        let beta = compute_beta(&leveraged, &spy);
        assert!(beta > 1.5, "2x leveraged beta should be ~2.0, got {beta}");
    }

    #[test]
    fn beta_of_defensive_below_1() {
        let spy = spy_returns();
        let defensive: Vec<f64> = spy.iter().map(|r| r * 0.5).collect();
        let beta = compute_beta(&defensive, &spy);
        assert!(beta < 0.7, "0.5x beta should be ~0.5, got {beta}");
    }

    // ── sector concentration ────────────────────────────────────────────

    #[test]
    fn tech_heavy_has_high_sector_hhi() {
        let portfolio = tech_heavy_portfolio();
        let exposure = compute_factor_exposure(&portfolio, &spy_returns());
        assert!(
            exposure.sector_hhi > 0.20,
            "Tech-heavy HHI should be > 0.20, got {}",
            exposure.sector_hhi
        );
        assert_eq!(exposure.top_sector_name, "Technology");
        assert!(
            exposure.top_sector_weight > 0.50,
            "Tech weight should be > 50%, got {}",
            exposure.top_sector_weight
        );
    }

    #[test]
    fn diversified_has_low_hhi() {
        let portfolio = diversified_portfolio();
        let exposure = compute_factor_exposure(&portfolio, &spy_returns());
        assert!(
            exposure.sector_hhi < 0.15,
            "Diversified HHI should be < 0.15, got {}",
            exposure.sector_hhi
        );
    }

    #[test]
    fn single_sector_hhi_is_1() {
        let positions = vec![
            make_position("A", "Tech", 0.5, 80.0, 2.0, 1.0),
            make_position("B", "Tech", 0.5, 80.0, 2.0, 1.0),
        ];
        let exposure = compute_factor_exposure(&positions, &spy_returns());
        assert!((exposure.sector_hhi - 1.0).abs() < 0.01);
    }

    // ── correlation ─────────────────────────────────────────────────────

    #[test]
    fn identical_returns_have_high_correlation() {
        let positions = vec![
            make_position("A", "Tech", 0.5, 80.0, 2.0, 1.0),
            make_position("B", "Tech", 0.5, 80.0, 2.0, 1.0), // same beta = ~same returns
        ];
        let exposure = compute_factor_exposure(&positions, &spy_returns());
        assert!(
            exposure.avg_correlation > 0.90,
            "Similar returns should have high corr, got {}",
            exposure.avg_correlation
        );
    }

    #[test]
    fn single_position_has_zero_correlation() {
        let positions = vec![make_position("A", "Tech", 1.0, 80.0, 2.0, 1.0)];
        let exposure = compute_factor_exposure(&positions, &spy_returns());
        assert!((exposure.avg_correlation - 0.0).abs() < 0.01);
    }

    // ── momentum and vol tilts ──────────────────────────────────────────

    #[test]
    fn momentum_tilt_weighted_average() {
        let portfolio = tech_heavy_portfolio();
        let exposure = compute_factor_exposure(&portfolio, &spy_returns());
        assert!(
            exposure.momentum_tilt > 60.0,
            "Tech-heavy portfolio should have high momentum tilt, got {}",
            exposure.momentum_tilt
        );
    }

    #[test]
    fn volatility_tilt_weighted_average() {
        let portfolio = tech_heavy_portfolio();
        let exposure = compute_factor_exposure(&portfolio, &spy_returns());
        assert!(
            exposure.volatility_tilt > 2.0,
            "Portfolio should have meaningful vol tilt, got {}",
            exposure.volatility_tilt
        );
    }

    // ── alerts ──────────────────────────────────────────────────────────

    #[test]
    fn tech_heavy_triggers_sector_alert() {
        let portfolio = tech_heavy_portfolio();
        let exposure = compute_factor_exposure(&portfolio, &spy_returns());
        let alerts = check_factor_alerts(&exposure, &FactorThresholds::default());
        assert!(
            alerts.iter().any(|a| a.factor == "Sector Concentration"),
            "Tech-heavy portfolio should trigger sector alert"
        );
    }

    #[test]
    fn diversified_no_sector_alert() {
        let portfolio = diversified_portfolio();
        let exposure = compute_factor_exposure(&portfolio, &spy_returns());
        let alerts = check_factor_alerts(&exposure, &FactorThresholds::default());
        assert!(
            !alerts
                .iter()
                .any(|a| a.factor == "Sector Concentration" && a.severity != FactorSeverity::Info),
            "Diversified portfolio should not trigger sector warning/critical"
        );
    }

    #[test]
    fn critical_when_significantly_above() {
        let exposure = FactorExposure {
            market_beta: 1.5,
            sector_hhi: 0.50,
            top_sector_weight: 0.70,
            top_sector_name: "Tech".into(),
            avg_correlation: 0.85,
            momentum_tilt: 90.0,
            volatility_tilt: 5.0,
        };
        let alerts = check_factor_alerts(&exposure, &FactorThresholds::default());
        let critical_count = alerts
            .iter()
            .filter(|a| a.severity == FactorSeverity::Critical)
            .count();
        assert!(
            critical_count >= 3,
            "Extreme exposure should trigger 3+ critical alerts, got {critical_count}"
        );
    }

    #[test]
    fn no_alerts_when_within_thresholds() {
        let exposure = FactorExposure {
            market_beta: 1.0,
            sector_hhi: 0.15,
            top_sector_weight: 0.25,
            top_sector_name: "Tech".into(),
            avg_correlation: 0.40,
            momentum_tilt: 70.0,
            volatility_tilt: 2.0,
        };
        let alerts = check_factor_alerts(&exposure, &FactorThresholds::default());
        assert!(
            alerts.is_empty(),
            "Should have no alerts, got {}",
            alerts.len()
        );
    }

    // ── edge cases ──────────────────────────────────────────────────────

    #[test]
    fn empty_portfolio_safe() {
        let exposure = compute_factor_exposure(&[], &spy_returns());
        assert!((exposure.market_beta - 0.0).abs() < 0.01);
        assert!(exposure.top_sector_name.is_empty());
    }

    #[test]
    fn info_severity_when_approaching() {
        let exposure = FactorExposure {
            market_beta: 1.2, // 1.2 is > 1.3 * 0.9 = 1.17
            sector_hhi: 0.15,
            top_sector_weight: 0.25,
            top_sector_name: "Tech".into(),
            avg_correlation: 0.40,
            momentum_tilt: 70.0,
            volatility_tilt: 2.0,
        };
        let alerts = check_factor_alerts(&exposure, &FactorThresholds::default());
        assert!(
            alerts.iter().any(|a| a.severity == FactorSeverity::Info),
            "Should have Info-level beta alert"
        );
    }
}
