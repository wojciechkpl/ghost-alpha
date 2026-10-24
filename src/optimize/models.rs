//! Pre-defined model portfolio templates.
//!
//! Provides conservative, moderate, and growth allocations using common ETFs.

/// A named model portfolio with fixed allocations.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelPortfolio {
    pub name: String,
    pub description: String,
    pub weights: Vec<(String, f64)>,
}

/// Conservative portfolio: ~60% bonds, ~30% equities, ~10% alternatives.
pub fn conservative() -> ModelPortfolio {
    ModelPortfolio {
        name: "Conservative".into(),
        description: "60% bonds, 30% equities, 10% alternatives".into(),
        weights: vec![
            ("AGG".into(), 0.40),
            ("TIP".into(), 0.10),
            ("IEF".into(), 0.10),
            ("VTI".into(), 0.20),
            ("EFA".into(), 0.10),
            ("GLD".into(), 0.05),
            ("VNQ".into(), 0.05),
        ],
    }
}

/// Moderate portfolio: ~40% bonds, ~50% equities, ~10% alternatives.
pub fn moderate() -> ModelPortfolio {
    ModelPortfolio {
        name: "Moderate".into(),
        description: "40% bonds, 50% equities, 10% alternatives".into(),
        weights: vec![
            ("AGG".into(), 0.25),
            ("TIP".into(), 0.05),
            ("IEF".into(), 0.10),
            ("VTI".into(), 0.30),
            ("EFA".into(), 0.15),
            ("VWO".into(), 0.05),
            ("GLD".into(), 0.05),
            ("VNQ".into(), 0.05),
        ],
    }
}

/// Growth portfolio: ~20% bonds, ~70% equities, ~10% alternatives.
pub fn growth() -> ModelPortfolio {
    ModelPortfolio {
        name: "Growth".into(),
        description: "20% bonds, 70% equities, 10% alternatives".into(),
        weights: vec![
            ("AGG".into(), 0.10),
            ("TIP".into(), 0.05),
            ("IEF".into(), 0.05),
            ("VTI".into(), 0.35),
            ("EFA".into(), 0.15),
            ("VWO".into(), 0.10),
            ("QQQ".into(), 0.10),
            ("GLD".into(), 0.05),
            ("VNQ".into(), 0.05),
        ],
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conservative_weights_sum_to_one() {
        let p = conservative();
        let sum: f64 = p.weights.iter().map(|(_, w)| w).sum();
        assert!(
            (sum - 1.0).abs() < 1e-10,
            "Conservative weights sum to {sum}, expected 1.0"
        );
    }

    #[test]
    fn test_moderate_weights_sum_to_one() {
        let p = moderate();
        let sum: f64 = p.weights.iter().map(|(_, w)| w).sum();
        assert!(
            (sum - 1.0).abs() < 1e-10,
            "Moderate weights sum to {sum}, expected 1.0"
        );
    }

    #[test]
    fn test_growth_weights_sum_to_one() {
        let p = growth();
        let sum: f64 = p.weights.iter().map(|(_, w)| w).sum();
        assert!(
            (sum - 1.0).abs() < 1e-10,
            "Growth weights sum to {sum}, expected 1.0"
        );
    }

    #[test]
    fn test_conservative_has_more_bonds_than_growth() {
        let bond_tickers = ["AGG", "TIP", "IEF"];
        let cons = conservative();
        let grow = growth();

        let cons_bonds: f64 = cons
            .weights
            .iter()
            .filter(|(t, _)| bond_tickers.contains(&t.as_str()))
            .map(|(_, w)| w)
            .sum();
        let grow_bonds: f64 = grow
            .weights
            .iter()
            .filter(|(t, _)| bond_tickers.contains(&t.as_str()))
            .map(|(_, w)| w)
            .sum();

        assert!(
            cons_bonds > grow_bonds,
            "Conservative should have more bonds ({cons_bonds}) than growth ({grow_bonds})"
        );
    }

    #[test]
    fn test_growth_has_more_equities_than_conservative() {
        let equity_tickers = ["VTI", "EFA", "VWO", "QQQ"];
        let cons = conservative();
        let grow = growth();

        let cons_eq: f64 = cons
            .weights
            .iter()
            .filter(|(t, _)| equity_tickers.contains(&t.as_str()))
            .map(|(_, w)| w)
            .sum();
        let grow_eq: f64 = grow
            .weights
            .iter()
            .filter(|(t, _)| equity_tickers.contains(&t.as_str()))
            .map(|(_, w)| w)
            .sum();

        assert!(
            grow_eq > cons_eq,
            "Growth should have more equities ({grow_eq}) than conservative ({cons_eq})"
        );
    }
}
