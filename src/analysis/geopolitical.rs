//! Geopolitical risk scorer using market-priced proxies.
//!
//! Computes a composite risk score from oil, VIX, gold, and DXY signals.
//! No news API needed — markets price in risk faster than headlines.

use serde::{Deserialize, Serialize};

/// Geopolitical risk inputs — all market-priced, no subjective scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoRiskInputs {
    /// Brent crude oil price ($/barrel).
    pub oil_brent: f64,
    /// CBOE VIX level.
    pub vix: f64,
    /// Gold 3-month price change (fraction, e.g., 0.08 = 8%).
    pub gold_3m_change: f64,
    /// US Dollar Index 3-month change (fraction, e.g., -0.05 = -5%).
    pub dxy_3m_change: f64,
}

/// Composite geopolitical risk score (0–10 scale).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoRiskScore {
    pub composite: f64,
    pub oil_component: f64,
    pub vix_component: f64,
    pub gold_component: f64,
    pub dxy_component: f64,
    pub level: GeoRiskLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GeoRiskLevel {
    Low,
    Moderate,
    High,
    Extreme,
}

impl std::fmt::Display for GeoRiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Low => write!(f, "Low"),
            Self::Moderate => write!(f, "Moderate"),
            Self::High => write!(f, "High"),
            Self::Extreme => write!(f, "Extreme"),
        }
    }
}

/// Compute composite geopolitical risk score from market inputs.
///
/// Each component scales 0–2.5, total 0–10:
/// - Oil: linear ramp $80–$150
/// - VIX: linear ramp 20–45
/// - Gold 3m change: ramp 5%–25% (flight to safety)
/// - DXY 3m decline: ramp 0% to -10% (dollar weakness = risk)
pub fn compute_geo_risk(inputs: &GeoRiskInputs) -> GeoRiskScore {
    let oil_c = ((inputs.oil_brent - 80.0) / 70.0).clamp(0.0, 1.0) * 2.5;
    let vix_c = ((inputs.vix - 20.0) / 25.0).clamp(0.0, 1.0) * 2.5;
    let gold_c = ((inputs.gold_3m_change - 0.05) / 0.20).clamp(0.0, 1.0) * 2.5;
    // Dollar weakness (negative change) increases risk.
    let dxy_c = ((-inputs.dxy_3m_change) / 0.10).clamp(0.0, 1.0) * 2.5;

    let composite = oil_c + vix_c + gold_c + dxy_c;
    let level = if composite < 3.0 {
        GeoRiskLevel::Low
    } else if composite < 5.0 {
        GeoRiskLevel::Moderate
    } else if composite < 7.0 {
        GeoRiskLevel::High
    } else {
        GeoRiskLevel::Extreme
    };

    GeoRiskScore {
        composite,
        oil_component: oil_c,
        vix_component: vix_c,
        gold_component: gold_c,
        dxy_component: dxy_c,
        level,
    }
}

/// Suggested allocation adjustments given a geo risk level.
/// Returns (sector, weight_delta) pairs, e.g., ("energy", 0.07) means +7%.
pub fn geo_allocation_adjustments(level: &GeoRiskLevel) -> Vec<(&'static str, f64)> {
    match level {
        GeoRiskLevel::Low => vec![],
        GeoRiskLevel::Moderate => vec![
            ("energy", 0.03),
            ("cash", 0.02),
            ("em", -0.02),
            ("growth", -0.03),
        ],
        GeoRiskLevel::High => vec![
            ("energy", 0.07),
            ("defense", 0.03),
            ("cash", 0.05),
            ("em", -0.04),
            ("growth", -0.03),
            ("intl_developed", -0.02),
            ("small_cap", -0.02),
        ],
        GeoRiskLevel::Extreme => vec![
            ("energy", 0.10),
            ("defense", 0.05),
            ("cash", 0.10),
            ("bonds", 0.05),
            ("em", -0.06),
            ("growth", -0.06),
            ("intl_developed", -0.04),
            ("small_cap", -0.04),
            ("alternatives", -0.03),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calm() -> GeoRiskInputs {
        GeoRiskInputs {
            oil_brent: 75.0,
            vix: 15.0,
            gold_3m_change: 0.02,
            dxy_3m_change: 0.01,
        }
    }

    #[test]
    fn test_calm_inputs_low_risk() {
        let score = compute_geo_risk(&calm());
        assert_eq!(score.level, GeoRiskLevel::Low);
        assert!(score.composite < 3.0);
    }

    #[test]
    fn test_elevated_oil_moderate_risk() {
        // Oil=$115 => 1.25, VIX=28 => 0.8, Gold=12% => 0.875, DXY=-6% => 1.5 => total ~4.4
        let inputs = GeoRiskInputs {
            oil_brent: 115.0,
            vix: 28.0,
            gold_3m_change: 0.12,
            dxy_3m_change: -0.06,
        };
        let score = compute_geo_risk(&inputs);
        assert_eq!(score.level, GeoRiskLevel::Moderate);
    }

    #[test]
    fn test_crisis_inputs_high_risk() {
        let inputs = GeoRiskInputs {
            oil_brent: 130.0,
            vix: 32.0,
            gold_3m_change: 0.15,
            dxy_3m_change: -0.05,
        };
        let score = compute_geo_risk(&inputs);
        assert_eq!(score.level, GeoRiskLevel::High);
    }

    #[test]
    fn test_extreme_inputs_extreme_risk() {
        let inputs = GeoRiskInputs {
            oil_brent: 150.0,
            vix: 45.0,
            gold_3m_change: 0.25,
            dxy_3m_change: -0.10,
        };
        let score = compute_geo_risk(&inputs);
        assert_eq!(score.level, GeoRiskLevel::Extreme);
    }

    #[test]
    fn test_composite_score_bounded_0_10() {
        // Below ramp
        let low = compute_geo_risk(&calm());
        assert!(low.composite >= 0.0 && low.composite <= 10.0);
        // Above ramp
        let extreme = GeoRiskInputs {
            oil_brent: 200.0,
            vix: 80.0,
            gold_3m_change: 0.50,
            dxy_3m_change: -0.30,
        };
        let high = compute_geo_risk(&extreme);
        assert!((high.composite - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_oil_component_scaled() {
        let low_oil = GeoRiskInputs {
            oil_brent: 60.0,
            ..calm()
        };
        assert!((compute_geo_risk(&low_oil).oil_component).abs() < f64::EPSILON);
        let high_oil = GeoRiskInputs {
            oil_brent: 150.0,
            ..calm()
        };
        assert!((compute_geo_risk(&high_oil).oil_component - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_vix_component_scaled() {
        let low_vix = GeoRiskInputs {
            vix: 12.0,
            ..calm()
        };
        assert!((compute_geo_risk(&low_vix).vix_component).abs() < f64::EPSILON);
        let high_vix = GeoRiskInputs {
            vix: 45.0,
            ..calm()
        };
        assert!((compute_geo_risk(&high_vix).vix_component - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gold_flight_to_safety() {
        let inputs = GeoRiskInputs {
            gold_3m_change: 0.20,
            ..calm()
        };
        assert!(compute_geo_risk(&inputs).gold_component > 1.5);
    }

    #[test]
    fn test_dxy_decline_adds_risk() {
        let inputs = GeoRiskInputs {
            dxy_3m_change: -0.08,
            ..calm()
        };
        assert!(compute_geo_risk(&inputs).dxy_component > 1.5);
    }

    #[test]
    fn test_geo_risk_level_ordering() {
        assert!(GeoRiskLevel::Low < GeoRiskLevel::Moderate);
        assert!(GeoRiskLevel::Moderate < GeoRiskLevel::High);
        assert!(GeoRiskLevel::High < GeoRiskLevel::Extreme);
    }

    #[test]
    fn test_allocation_adjustments_high() {
        let adj = geo_allocation_adjustments(&GeoRiskLevel::High);
        let energy = adj.iter().find(|(s, _)| *s == "energy").unwrap();
        assert!(energy.1 > 0.0, "Energy should increase");
        let em = adj.iter().find(|(s, _)| *s == "em").unwrap();
        assert!(em.1 < 0.0, "EM should decrease");
    }

    #[test]
    fn test_allocation_adjustments_low() {
        let adj = geo_allocation_adjustments(&GeoRiskLevel::Low);
        assert!(adj.is_empty(), "Low risk should have no adjustments");
    }
}
