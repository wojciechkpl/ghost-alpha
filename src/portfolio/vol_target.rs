//! Portfolio-level volatility targeting.
//!
//! Phase 8 (Wave 1): Scales total portfolio exposure to maintain a target
//! annualized volatility. When realized vol exceeds target, reduce exposure.
//! When below target, increase exposure. This is the single highest-impact
//! risk management technique from institutional quant finance.
//!
//! Academic basis: Moreira & Muir (2017) — volatility-managed portfolios
//! improve Sharpe by 0.1-0.3 across most equity strategies.

// ─── Configuration ──────────────────────────────────────────────────────────

/// Configuration for volatility targeting.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VolTargetConfig {
    /// Target annualized portfolio volatility (default: 0.15 = 15%)
    pub target_vol: f64,
    /// Lookback window for realized vol (default: 20 trading days)
    pub lookback_days: usize,
    /// Minimum exposure scalar (default: 0.5 = won't go below 50% exposure)
    pub min_scalar: f64,
    /// Maximum exposure scalar (default: 1.5 = won't go above 150% exposure)
    pub max_scalar: f64,
    /// Deadband: don't rebalance if scalar is within this range of 1.0
    /// (default: 0.05 = ignore changes < 5%)
    pub deadband: f64,
}

impl Default for VolTargetConfig {
    fn default() -> Self {
        Self {
            target_vol: 0.15,
            lookback_days: 20,
            min_scalar: 0.5,
            max_scalar: 1.5,
            deadband: 0.05,
        }
    }
}

// ─── Types ──────────────────────────────────────────────────────────────────

/// Action recommendation from volatility targeting.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum VolTargetAction {
    /// Realized vol significantly exceeds target — trim positions
    ReduceExposure,
    /// Vol near target — no action needed
    MaintainExposure,
    /// Realized vol well below target — can add exposure
    IncreaseExposure,
}

/// Result of volatility targeting computation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VolTargetResult {
    /// Annualized realized portfolio volatility
    pub realized_vol: f64,
    /// Target annualized volatility
    pub target_vol: f64,
    /// Exposure scalar: target / realized, clamped to [min, max]
    pub exposure_scalar: f64,
    /// Recommended action
    pub action: VolTargetAction,
    /// Recommended max invested % after scaling
    /// (e.g., if base max is 80% and scalar is 0.75, recommended is 60%)
    pub recommended_invested_pct: f64,
    /// Current base invested % (before scaling)
    pub base_invested_pct: f64,
}

// ─── Core Functions ─────────────────────────────────────────────────────────

/// Compute annualized realized volatility from a daily return series.
///
/// Uses the trailing `window` observations. Returns `None` if insufficient data.
/// Formula: rv = std(daily_returns) × sqrt(252)
pub fn realized_portfolio_vol(daily_returns: &[f64], window: usize) -> Option<f64> {
    if daily_returns.len() < window || window < 2 {
        return None;
    }

    let tail = &daily_returns[daily_returns.len() - window..];
    let n = tail.len() as f64;
    let mean = tail.iter().sum::<f64>() / n;
    let variance = tail.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
    let daily_vol = variance.sqrt();

    Some(daily_vol * (252.0_f64).sqrt())
}

/// Compute the volatility target exposure scalar and recommendation.
///
/// The scalar adjusts total portfolio exposure to maintain target vol:
/// - scalar > 1.0: realized vol is low, can increase exposure
/// - scalar < 1.0: realized vol is high, should reduce exposure
/// - scalar = 1.0: vol is on target
pub fn compute_vol_target(
    daily_returns: &[f64],
    config: &VolTargetConfig,
    current_invested_pct: f64,
) -> VolTargetResult {
    let realized_vol =
        realized_portfolio_vol(daily_returns, config.lookback_days).unwrap_or(config.target_vol); // default to target if no data

    let raw_scalar = if realized_vol > 0.001 {
        config.target_vol / realized_vol
    } else {
        config.max_scalar // very low vol = safe to max exposure
    };

    let scalar = raw_scalar.clamp(config.min_scalar, config.max_scalar);

    let action = if (scalar - 1.0).abs() <= config.deadband {
        VolTargetAction::MaintainExposure
    } else if scalar < 1.0 {
        VolTargetAction::ReduceExposure
    } else {
        VolTargetAction::IncreaseExposure
    };

    let recommended = (current_invested_pct * scalar).clamp(0.0, 1.0);

    VolTargetResult {
        realized_vol,
        target_vol: config.target_vol,
        exposure_scalar: scalar,
        action,
        recommended_invested_pct: recommended,
        base_invested_pct: current_invested_pct,
    }
}

/// Convenience: compute how many basis points of position to trim or add.
///
/// Positive = add this much exposure. Negative = trim this much.
/// Returns 0 if within deadband.
pub fn exposure_change_pct(result: &VolTargetResult) -> f64 {
    match result.action {
        VolTargetAction::MaintainExposure => 0.0,
        _ => result.recommended_invested_pct - result.base_invested_pct,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn config() -> VolTargetConfig {
        VolTargetConfig::default() // target 15%, window 20, scalar [0.5, 1.5]
    }

    // ── realized_portfolio_vol ───────────────────────────────────────────

    #[test]
    fn realized_vol_flat_returns_is_near_zero() {
        let returns = vec![0.001; 30]; // constant daily return
        let vol = realized_portfolio_vol(&returns, 20).unwrap();
        assert!(
            vol < 0.01,
            "Flat returns should have near-zero vol, got {vol}"
        );
    }

    #[test]
    fn realized_vol_volatile_returns_is_high() {
        // Alternating +3% / -3% daily = ~47% annualized vol
        let returns: Vec<f64> = (0..30)
            .map(|i| if i % 2 == 0 { 0.03 } else { -0.03 })
            .collect();
        let vol = realized_portfolio_vol(&returns, 20).unwrap();
        assert!(
            vol > 0.30,
            "Alternating 3% returns should have high vol, got {vol}"
        );
    }

    #[test]
    fn realized_vol_insufficient_data_returns_none() {
        let returns = vec![0.01; 10];
        assert!(realized_portfolio_vol(&returns, 20).is_none());
    }

    #[test]
    fn realized_vol_window_respected() {
        // First 20 returns are volatile, last 20 are flat
        let mut returns = vec![];
        for i in 0..20 {
            returns.push(if i % 2 == 0 { 0.03 } else { -0.03 });
        }
        returns.extend(std::iter::repeat_n(0.001, 20));
        // Window 20 should use only the flat tail
        let vol = realized_portfolio_vol(&returns, 20).unwrap();
        assert!(vol < 0.05, "Should use only recent flat window, got {vol}");
    }

    #[test]
    fn realized_vol_annualized() {
        // Known: if daily std = 1%, annualized = 1% × sqrt(252) ≈ 15.87%
        let returns: Vec<f64> = (0..100)
            .map(|i| if i % 2 == 0 { 0.01 } else { -0.01 })
            .collect();
        let vol = realized_portfolio_vol(&returns, 100).unwrap();
        // Daily std of alternating +1%/-1% = 0.01
        // Annualized = 0.01 * sqrt(252) ≈ 0.1587
        assert!(
            (vol - 0.1587).abs() < 0.02,
            "Expected ~15.87%, got {:.2}%",
            vol * 100.0
        );
    }

    // ── compute_vol_target ──────────────────────────────────────────────

    #[test]
    fn scalar_1_when_at_target() {
        // Realized vol = target vol → scalar = 1.0
        let cfg = config(); // target 15%
                            // Create returns with ~15% annualized vol: daily std ≈ 0.15/sqrt(252) ≈ 0.00945
        let daily_std = 0.15 / (252.0_f64).sqrt();
        let returns: Vec<f64> = (0..30)
            .map(|i| if i % 2 == 0 { daily_std } else { -daily_std })
            .collect();
        let result = compute_vol_target(&returns, &cfg, 0.70);
        assert!(
            (result.exposure_scalar - 1.0).abs() < 0.15,
            "At-target vol should have scalar ~1.0, got {:.2}",
            result.exposure_scalar
        );
    }

    #[test]
    fn scalar_below_1_when_vol_high() {
        // Realized vol >> target → scalar < 1.0
        let cfg = config();
        let returns: Vec<f64> = (0..30)
            .map(|i| if i % 2 == 0 { 0.03 } else { -0.03 })
            .collect();
        let result = compute_vol_target(&returns, &cfg, 0.80);
        assert!(
            result.exposure_scalar < 1.0,
            "High vol should reduce scalar, got {:.2}",
            result.exposure_scalar
        );
        assert_eq!(result.action, VolTargetAction::ReduceExposure);
    }

    #[test]
    fn scalar_above_1_when_vol_low() {
        // Realized vol << target → scalar > 1.0
        let cfg = config();
        let returns = vec![0.001; 30]; // near-zero vol
        let result = compute_vol_target(&returns, &cfg, 0.60);
        assert!(
            result.exposure_scalar > 1.0,
            "Low vol should increase scalar, got {:.2}",
            result.exposure_scalar
        );
        assert_eq!(result.action, VolTargetAction::IncreaseExposure);
    }

    #[test]
    fn scalar_clamped_at_min() {
        let cfg = config(); // min_scalar = 0.5
                            // Extremely high vol (40%+ annualized)
        let returns: Vec<f64> = (0..30)
            .map(|i| if i % 2 == 0 { 0.05 } else { -0.05 })
            .collect();
        let result = compute_vol_target(&returns, &cfg, 0.80);
        assert!(
            result.exposure_scalar >= cfg.min_scalar - 0.001,
            "Scalar should not go below min {}, got {:.3}",
            cfg.min_scalar,
            result.exposure_scalar
        );
    }

    #[test]
    fn scalar_clamped_at_max() {
        let cfg = config(); // max_scalar = 1.5
        let returns = vec![0.0001; 30]; // near-zero vol → huge raw scalar
        let result = compute_vol_target(&returns, &cfg, 0.50);
        assert!(
            result.exposure_scalar <= cfg.max_scalar + 0.001,
            "Scalar should not exceed max {}, got {:.3}",
            cfg.max_scalar,
            result.exposure_scalar
        );
    }

    #[test]
    fn maintain_action_within_deadband() {
        let cfg = VolTargetConfig {
            deadband: 0.10,
            ..config()
        };
        // Create returns with vol slightly above target (scalar ~0.95)
        let daily_std = 0.16 / (252.0_f64).sqrt(); // 16% vol vs 15% target → scalar 0.94
        let returns: Vec<f64> = (0..30)
            .map(|i| if i % 2 == 0 { daily_std } else { -daily_std })
            .collect();
        let result = compute_vol_target(&returns, &cfg, 0.70);
        // Scalar should be ~0.94, within deadband of 0.10 around 1.0
        if (result.exposure_scalar - 1.0).abs() <= cfg.deadband {
            assert_eq!(result.action, VolTargetAction::MaintainExposure);
        }
    }

    #[test]
    fn recommended_invested_pct_scales() {
        let cfg = config();
        let returns: Vec<f64> = (0..30)
            .map(|i| if i % 2 == 0 { 0.03 } else { -0.03 })
            .collect();
        let result = compute_vol_target(&returns, &cfg, 0.80);
        // High vol → scalar < 1.0 → recommended < base
        assert!(
            result.recommended_invested_pct < result.base_invested_pct,
            "High vol should reduce recommended from {:.1}% to {:.1}%",
            result.base_invested_pct * 100.0,
            result.recommended_invested_pct * 100.0
        );
    }

    #[test]
    fn recommended_invested_pct_capped_at_100() {
        let cfg = config();
        let returns = vec![0.001; 30]; // low vol → high scalar
        let result = compute_vol_target(&returns, &cfg, 0.90);
        assert!(
            result.recommended_invested_pct <= 1.0,
            "Recommended should not exceed 100%, got {:.1}%",
            result.recommended_invested_pct * 100.0
        );
    }

    #[test]
    fn zero_vol_returns_max_scalar() {
        let cfg = config();
        let returns = vec![0.0; 30]; // exactly zero returns
        let result = compute_vol_target(&returns, &cfg, 0.70);
        assert_relative_eq!(result.exposure_scalar, cfg.max_scalar, epsilon = 0.01);
    }

    #[test]
    fn insufficient_data_defaults_to_target() {
        let cfg = config();
        let returns = vec![0.01; 5]; // less than lookback window
        let result = compute_vol_target(&returns, &cfg, 0.70);
        // Default realized vol = target vol → scalar = 1.0
        assert_relative_eq!(result.exposure_scalar, 1.0, epsilon = 0.01);
        assert_eq!(result.action, VolTargetAction::MaintainExposure);
    }

    // ── exposure_change_pct ─────────────────────────────────────────────

    #[test]
    fn exposure_change_zero_when_maintain() {
        let result = VolTargetResult {
            realized_vol: 0.15,
            target_vol: 0.15,
            exposure_scalar: 1.0,
            action: VolTargetAction::MaintainExposure,
            recommended_invested_pct: 0.70,
            base_invested_pct: 0.70,
        };
        assert_relative_eq!(exposure_change_pct(&result), 0.0);
    }

    #[test]
    fn exposure_change_negative_when_reduce() {
        let result = VolTargetResult {
            realized_vol: 0.25,
            target_vol: 0.15,
            exposure_scalar: 0.6,
            action: VolTargetAction::ReduceExposure,
            recommended_invested_pct: 0.48,
            base_invested_pct: 0.80,
        };
        assert!(exposure_change_pct(&result) < 0.0);
    }

    #[test]
    fn exposure_change_positive_when_increase() {
        let result = VolTargetResult {
            realized_vol: 0.08,
            target_vol: 0.15,
            exposure_scalar: 1.5,
            action: VolTargetAction::IncreaseExposure,
            recommended_invested_pct: 0.90,
            base_invested_pct: 0.60,
        };
        assert!(exposure_change_pct(&result) > 0.0);
    }

    // ── config defaults ─────────────────────────────────────────────────

    #[test]
    fn config_defaults_reasonable() {
        let cfg = VolTargetConfig::default();
        assert_relative_eq!(cfg.target_vol, 0.15);
        assert_eq!(cfg.lookback_days, 20);
        assert!(cfg.min_scalar > 0.0 && cfg.min_scalar < 1.0);
        assert!(cfg.max_scalar > 1.0 && cfg.max_scalar < 3.0);
        assert!(cfg.deadband > 0.0 && cfg.deadband < 0.2);
    }
}
