//! Macro risk level assessment from market-wide indicators.
//!
//! Combines VIX, HY OAS, yield curve, and put/call ratio into
//! a single `MacroRiskLevel` for position sizing and regime overlay.

use serde::{Deserialize, Serialize};

// ─── Risk Level ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MacroRiskLevel {
    Normal,
    Elevated,
    HighRisk,
}

impl std::fmt::Display for MacroRiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MacroRiskLevel::Normal => write!(f, "Normal"),
            MacroRiskLevel::Elevated => write!(f, "Elevated"),
            MacroRiskLevel::HighRisk => write!(f, "HighRisk"),
        }
    }
}

// ─── Snapshot ────────────────────────────────────────────────────────────────

/// A point-in-time snapshot of macro risk indicators.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacroRiskSnapshot {
    /// CBOE VIX level
    pub vix_level: f64,
    /// High-yield OAS spread in basis points
    pub hy_oas_bps: Option<f64>,
    /// 2s10s yield curve spread (negative = inverted)
    pub yield_curve_2s10s: Option<f64>,
    /// Equity put/call ratio
    pub put_call_ratio: Option<f64>,
    /// Computed risk level
    pub risk_level: MacroRiskLevel,
}

// ─── Risk Computation ────────────────────────────────────────────────────────

/// Compute the macro risk level from indicator values.
///
/// **HighRisk**: VIX ≥ 35 OR HY OAS ≥ 600 bps OR yield curve < -0.30
/// **Elevated**: VIX ≥ 25 OR HY OAS ≥ 500 bps OR yield curve < 0.0 OR put/call > 1.2
/// **Normal**: everything else
pub fn compute_risk_level(
    vix: f64,
    hy_oas_bps: Option<f64>,
    yield_curve_2s10s: Option<f64>,
    put_call_ratio: Option<f64>,
) -> MacroRiskLevel {
    // HighRisk triggers
    if vix >= 35.0 {
        return MacroRiskLevel::HighRisk;
    }
    if let Some(oas) = hy_oas_bps {
        if oas >= 600.0 {
            return MacroRiskLevel::HighRisk;
        }
    }
    if let Some(yc) = yield_curve_2s10s {
        if yc < -0.30 {
            return MacroRiskLevel::HighRisk;
        }
    }

    // Elevated triggers
    if vix >= 25.0 {
        return MacroRiskLevel::Elevated;
    }
    if let Some(oas) = hy_oas_bps {
        if oas >= 500.0 {
            return MacroRiskLevel::Elevated;
        }
    }
    if let Some(yc) = yield_curve_2s10s {
        if yc < 0.0 {
            return MacroRiskLevel::Elevated;
        }
    }
    if let Some(pc) = put_call_ratio {
        if pc > 1.2 {
            return MacroRiskLevel::Elevated;
        }
    }

    MacroRiskLevel::Normal
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calm_market_is_normal() {
        let level = compute_risk_level(15.0, Some(350.0), Some(0.5), Some(0.8));
        assert_eq!(level, MacroRiskLevel::Normal);
    }

    #[test]
    fn high_vix_is_elevated() {
        let level = compute_risk_level(28.0, Some(400.0), Some(0.3), Some(0.9));
        assert_eq!(level, MacroRiskLevel::Elevated);
    }

    #[test]
    fn extreme_vix_is_high_risk() {
        let level = compute_risk_level(40.0, Some(400.0), Some(0.3), Some(0.9));
        assert_eq!(level, MacroRiskLevel::HighRisk);
    }

    #[test]
    fn risk_level_ordering() {
        assert!(MacroRiskLevel::Normal < MacroRiskLevel::Elevated);
        assert!(MacroRiskLevel::Elevated < MacroRiskLevel::HighRisk);
    }

    #[test]
    fn boundary_values() {
        // VIX = 25.0 exact → Elevated
        assert_eq!(
            compute_risk_level(25.0, None, None, None),
            MacroRiskLevel::Elevated
        );
        // VIX = 24.9 → Normal
        assert_eq!(
            compute_risk_level(24.9, None, None, None),
            MacroRiskLevel::Normal
        );
        // VIX = 35.0 exact → HighRisk
        assert_eq!(
            compute_risk_level(35.0, None, None, None),
            MacroRiskLevel::HighRisk
        );
        // Inverted yield curve → Elevated
        assert_eq!(
            compute_risk_level(15.0, None, Some(-0.1), None),
            MacroRiskLevel::Elevated
        );
        // Deeply inverted → HighRisk
        assert_eq!(
            compute_risk_level(15.0, None, Some(-0.35), None),
            MacroRiskLevel::HighRisk
        );
    }
}
