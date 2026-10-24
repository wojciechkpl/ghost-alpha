//! Brokerage fund mapper and risk profile templates.
//!
//! Maps abstract asset class allocations to concrete fund picks
//! with expense ratios, TLH partners, and transferability flags.

use serde::{Deserialize, Serialize};

// ─── Asset Classes ──────────────────────────────────────────────────────────

/// Canonical asset class identifiers used across the system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetClass {
    UsBroadMarket,
    UsLargeCap,
    UsMidCap,
    UsSmallCap,
    IntlDeveloped,
    IntlEmerging,
    UsBonds,
    IntlBonds,
    Tips,
    RealEstate,
    Commodities,
}

impl std::fmt::Display for AssetClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UsBroadMarket => write!(f, "US Broad Market"),
            Self::UsLargeCap => write!(f, "US Large Cap"),
            Self::UsMidCap => write!(f, "US Mid Cap"),
            Self::UsSmallCap => write!(f, "US Small Cap"),
            Self::IntlDeveloped => write!(f, "Intl Developed"),
            Self::IntlEmerging => write!(f, "Intl Emerging"),
            Self::UsBonds => write!(f, "US Bonds"),
            Self::IntlBonds => write!(f, "Intl Bonds"),
            Self::Tips => write!(f, "TIPS"),
            Self::RealEstate => write!(f, "Real Estate"),
            Self::Commodities => write!(f, "Commodities"),
        }
    }
}

// ─── Risk Profiles ──────────────────────────────────────────────────────────

/// A named target allocation across asset classes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskProfile {
    pub name: String,
    pub description: String,
    /// Target weights by asset class. Must sum to ~1.0.
    pub allocations: Vec<(AssetClass, f64)>,
}

/// Built-in risk profiles.
pub fn risk_profiles() -> Vec<RiskProfile> {
    vec![
        RiskProfile {
            name: "Aggressive".into(),
            description: "High growth, minimal bonds. Suited for 10+ year horizon.".into(),
            allocations: vec![
                (AssetClass::UsBroadMarket, 0.30),
                (AssetClass::UsSmallCap, 0.10),
                (AssetClass::IntlDeveloped, 0.15),
                (AssetClass::IntlEmerging, 0.10),
                (AssetClass::UsBonds, 0.05),
                (AssetClass::RealEstate, 0.10),
                (AssetClass::UsLargeCap, 0.15),
                (AssetClass::Commodities, 0.05),
            ],
        },
        RiskProfile {
            name: "Moderate".into(),
            description: "Balanced growth and income. Suited for 5-10 year horizon.".into(),
            allocations: vec![
                (AssetClass::UsBroadMarket, 0.25),
                (AssetClass::UsLargeCap, 0.10),
                (AssetClass::IntlDeveloped, 0.10),
                (AssetClass::IntlEmerging, 0.05),
                (AssetClass::UsBonds, 0.25),
                (AssetClass::Tips, 0.05),
                (AssetClass::RealEstate, 0.10),
                (AssetClass::UsSmallCap, 0.05),
                (AssetClass::Commodities, 0.05),
            ],
        },
        RiskProfile {
            name: "Conservative".into(),
            description: "Capital preservation with modest growth. Suited for <5 year horizon."
                .into(),
            allocations: vec![
                (AssetClass::UsBonds, 0.40),
                (AssetClass::Tips, 0.10),
                (AssetClass::UsBroadMarket, 0.20),
                (AssetClass::IntlDeveloped, 0.10),
                (AssetClass::IntlBonds, 0.10),
                (AssetClass::RealEstate, 0.05),
                (AssetClass::UsLargeCap, 0.05),
            ],
        },
    ]
}

// ─── Fund Catalog ───────────────────────────────────────────────────────────

/// A specific fund at a brokerage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundEntry {
    pub ticker: String,
    pub name: String,
    pub asset_class: AssetClass,
    pub expense_ratio: f64,
    /// Tax-loss harvesting partner ticker (substantially similar fund).
    pub tlh_partner: Option<String>,
    /// Whether the fund can be transferred in-kind to another brokerage.
    pub transferable: bool,
    /// Minimum investment in dollars (0.0 = no minimum).
    pub minimum_investment: f64,
}

/// A brokerage's fund catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerageCatalog {
    pub brokerage: String,
    pub funds: Vec<FundEntry>,
}

/// Brokerage identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Brokerage {
    Fidelity,
    Schwab,
    Vanguard,
}

/// Get the fund catalog for a brokerage.
pub fn brokerage_catalog(brokerage: &Brokerage) -> BrokerageCatalog {
    match brokerage {
        Brokerage::Fidelity => fidelity_catalog(),
        Brokerage::Schwab => schwab_catalog(),
        Brokerage::Vanguard => vanguard_catalog(),
    }
}

fn fidelity_catalog() -> BrokerageCatalog {
    BrokerageCatalog {
        brokerage: "Fidelity".into(),
        funds: vec![
            FundEntry {
                ticker: "FZROX".into(),
                name: "Fidelity ZERO Total Market".into(),
                asset_class: AssetClass::UsBroadMarket,
                expense_ratio: 0.0,
                tlh_partner: Some("FSKAX".into()),
                transferable: false,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "FSKAX".into(),
                name: "Fidelity Total Market Index".into(),
                asset_class: AssetClass::UsBroadMarket,
                expense_ratio: 0.00015,
                tlh_partner: Some("FZROX".into()),
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "FXAIX".into(),
                name: "Fidelity 500 Index".into(),
                asset_class: AssetClass::UsLargeCap,
                expense_ratio: 0.00015,
                tlh_partner: Some("FNILX".into()),
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "FNILX".into(),
                name: "Fidelity ZERO Large Cap".into(),
                asset_class: AssetClass::UsLargeCap,
                expense_ratio: 0.0,
                tlh_partner: Some("FXAIX".into()),
                transferable: false,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "FZILX".into(),
                name: "Fidelity ZERO Intl Index".into(),
                asset_class: AssetClass::IntlDeveloped,
                expense_ratio: 0.0,
                tlh_partner: Some("FTIHX".into()),
                transferable: false,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "FTIHX".into(),
                name: "Fidelity Total Intl Index".into(),
                asset_class: AssetClass::IntlDeveloped,
                expense_ratio: 0.0006,
                tlh_partner: Some("FZILX".into()),
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "FPADX".into(),
                name: "Fidelity Emerging Markets Index".into(),
                asset_class: AssetClass::IntlEmerging,
                expense_ratio: 0.00075,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "FSMDX".into(),
                name: "Fidelity Mid Cap Index".into(),
                asset_class: AssetClass::UsMidCap,
                expense_ratio: 0.00025,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "FSSNX".into(),
                name: "Fidelity Small Cap Index".into(),
                asset_class: AssetClass::UsSmallCap,
                expense_ratio: 0.00025,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "FXNAX".into(),
                name: "Fidelity US Bond Index".into(),
                asset_class: AssetClass::UsBonds,
                expense_ratio: 0.00025,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "FIPDX".into(),
                name: "Fidelity Inflation-Protected Bond Index".into(),
                asset_class: AssetClass::Tips,
                expense_ratio: 0.0005,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "FREL".into(),
                name: "Fidelity MSCI Real Estate ETF".into(),
                asset_class: AssetClass::RealEstate,
                expense_ratio: 0.00084,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
        ],
    }
}

fn schwab_catalog() -> BrokerageCatalog {
    BrokerageCatalog {
        brokerage: "Schwab".into(),
        funds: vec![
            FundEntry {
                ticker: "SWTSX".into(),
                name: "Schwab Total Stock Market Index".into(),
                asset_class: AssetClass::UsBroadMarket,
                expense_ratio: 0.0003,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "SWPPX".into(),
                name: "Schwab S&P 500 Index".into(),
                asset_class: AssetClass::UsLargeCap,
                expense_ratio: 0.0002,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "SWISX".into(),
                name: "Schwab Intl Index".into(),
                asset_class: AssetClass::IntlDeveloped,
                expense_ratio: 0.0006,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "SFENX".into(),
                name: "Schwab Emerging Markets Equity ETF".into(),
                asset_class: AssetClass::IntlEmerging,
                expense_ratio: 0.0011,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "SWSSX".into(),
                name: "Schwab Small Cap Index".into(),
                asset_class: AssetClass::UsSmallCap,
                expense_ratio: 0.0004,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "SWAGX".into(),
                name: "Schwab US Aggregate Bond Index".into(),
                asset_class: AssetClass::UsBonds,
                expense_ratio: 0.0004,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
        ],
    }
}

fn vanguard_catalog() -> BrokerageCatalog {
    BrokerageCatalog {
        brokerage: "Vanguard".into(),
        funds: vec![
            FundEntry {
                ticker: "VTI".into(),
                name: "Vanguard Total Stock Market ETF".into(),
                asset_class: AssetClass::UsBroadMarket,
                expense_ratio: 0.0003,
                tlh_partner: Some("ITOT".into()),
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "VOO".into(),
                name: "Vanguard S&P 500 ETF".into(),
                asset_class: AssetClass::UsLargeCap,
                expense_ratio: 0.0003,
                tlh_partner: Some("IVV".into()),
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "VXUS".into(),
                name: "Vanguard Total Intl Stock ETF".into(),
                asset_class: AssetClass::IntlDeveloped,
                expense_ratio: 0.0007,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "VWO".into(),
                name: "Vanguard FTSE Emerging Markets ETF".into(),
                asset_class: AssetClass::IntlEmerging,
                expense_ratio: 0.0008,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "VB".into(),
                name: "Vanguard Small-Cap ETF".into(),
                asset_class: AssetClass::UsSmallCap,
                expense_ratio: 0.0005,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "BND".into(),
                name: "Vanguard Total Bond Market ETF".into(),
                asset_class: AssetClass::UsBonds,
                expense_ratio: 0.0003,
                tlh_partner: Some("AGG".into()),
                transferable: true,
                minimum_investment: 0.0,
            },
            FundEntry {
                ticker: "VNQ".into(),
                name: "Vanguard Real Estate ETF".into(),
                asset_class: AssetClass::RealEstate,
                expense_ratio: 0.0012,
                tlh_partner: None,
                transferable: true,
                minimum_investment: 0.0,
            },
        ],
    }
}

// ─── Fund Mapping ───────────────────────────────────────────────────────────

/// A concrete allocation: asset class mapped to a specific fund with dollar amount.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcreteAllocation {
    pub asset_class: AssetClass,
    pub fund: FundEntry,
    pub target_weight: f64,
    pub dollar_amount: f64,
}

/// Map abstract asset class weights to concrete fund picks for a brokerage.
///
/// For each asset class in `weights`, selects the fund from the brokerage catalog
/// with the lowest expense ratio. Skips asset classes with no matching fund.
pub fn map_to_funds(
    weights: &[(AssetClass, f64)],
    brokerage: &Brokerage,
    total_portfolio_value: f64,
) -> Vec<ConcreteAllocation> {
    let catalog = brokerage_catalog(brokerage);

    weights
        .iter()
        .filter_map(|(ac, weight)| {
            // Find the fund with the lowest ER for this asset class.
            let best = catalog
                .funds
                .iter()
                .filter(|f| f.asset_class == *ac)
                .min_by(|a, b| a.expense_ratio.partial_cmp(&b.expense_ratio).unwrap());

            best.map(|fund| ConcreteAllocation {
                asset_class: ac.clone(),
                fund: fund.clone(),
                target_weight: *weight,
                dollar_amount: total_portfolio_value * weight,
            })
        })
        .collect()
}

/// Compute the weighted average expense ratio for a set of concrete allocations.
pub fn blended_expense_ratio(allocations: &[ConcreteAllocation]) -> f64 {
    let total_weight: f64 = allocations.iter().map(|a| a.target_weight).sum();
    if total_weight <= 0.0 {
        return 0.0;
    }
    allocations
        .iter()
        .map(|a| a.fund.expense_ratio * a.target_weight)
        .sum::<f64>()
        / total_weight
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_risk_profiles_weights_sum_to_one() {
        for profile in risk_profiles() {
            let sum: f64 = profile.allocations.iter().map(|(_, w)| w).sum();
            assert!(
                (sum - 1.0).abs() < 0.02,
                "Profile '{}' weights sum to {}, expected ~1.0",
                profile.name,
                sum
            );
        }
    }

    #[test]
    fn test_risk_profiles_have_names() {
        let profiles = risk_profiles();
        assert!(profiles.len() >= 3, "Need at least 3 profiles");
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"Aggressive"));
        assert!(names.contains(&"Moderate"));
        assert!(names.contains(&"Conservative"));
    }

    #[test]
    fn test_fidelity_catalog_has_us_broad() {
        let cat = brokerage_catalog(&Brokerage::Fidelity);
        assert!(cat
            .funds
            .iter()
            .any(|f| f.asset_class == AssetClass::UsBroadMarket));
    }

    #[test]
    fn test_fidelity_zero_funds_not_transferable() {
        let cat = brokerage_catalog(&Brokerage::Fidelity);
        for f in cat.funds.iter().filter(|f| f.ticker.starts_with("FZ")) {
            assert!(
                !f.transferable,
                "ZERO fund {} should not be transferable",
                f.ticker
            );
        }
    }

    #[test]
    fn test_fidelity_zero_funds_zero_er() {
        let cat = brokerage_catalog(&Brokerage::Fidelity);
        let fzrox = cat.funds.iter().find(|f| f.ticker == "FZROX").unwrap();
        assert!((fzrox.expense_ratio).abs() < f64::EPSILON);
    }

    #[test]
    fn test_schwab_catalog_has_us_broad() {
        let cat = brokerage_catalog(&Brokerage::Schwab);
        assert!(cat
            .funds
            .iter()
            .any(|f| f.asset_class == AssetClass::UsBroadMarket));
    }

    #[test]
    fn test_vanguard_catalog_has_us_broad() {
        let cat = brokerage_catalog(&Brokerage::Vanguard);
        assert!(cat
            .funds
            .iter()
            .any(|f| f.asset_class == AssetClass::UsBroadMarket));
    }

    #[test]
    fn test_map_to_funds_basic() {
        let weights = vec![
            (AssetClass::UsBroadMarket, 0.40),
            (AssetClass::IntlDeveloped, 0.30),
            (AssetClass::UsBonds, 0.30),
        ];
        let result = map_to_funds(&weights, &Brokerage::Fidelity, 1_000_000.0);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_map_to_funds_selects_lowest_er() {
        let weights = vec![(AssetClass::UsBroadMarket, 1.0)];
        let result = map_to_funds(&weights, &Brokerage::Fidelity, 100_000.0);
        assert_eq!(
            result[0].fund.ticker, "FZROX",
            "Should select FZROX (0.00% ER)"
        );
    }

    #[test]
    fn test_map_to_funds_skips_unmapped_class() {
        let weights = vec![(AssetClass::Commodities, 0.10)];
        let result = map_to_funds(&weights, &Brokerage::Fidelity, 100_000.0);
        assert!(result.is_empty(), "Commodities not in Fidelity catalog");
    }

    #[test]
    fn test_map_to_funds_dollar_amounts() {
        let weights = vec![(AssetClass::UsBroadMarket, 0.15)];
        let result = map_to_funds(&weights, &Brokerage::Fidelity, 1_000_000.0);
        assert!((result[0].dollar_amount - 150_000.0).abs() < 0.01);
    }

    #[test]
    fn test_blended_expense_ratio() {
        let allocs = vec![
            ConcreteAllocation {
                asset_class: AssetClass::UsBroadMarket,
                fund: FundEntry {
                    ticker: "FZROX".into(),
                    name: "".into(),
                    asset_class: AssetClass::UsBroadMarket,
                    expense_ratio: 0.0,
                    tlh_partner: None,
                    transferable: false,
                    minimum_investment: 0.0,
                },
                target_weight: 0.50,
                dollar_amount: 500_000.0,
            },
            ConcreteAllocation {
                asset_class: AssetClass::UsLargeCap,
                fund: FundEntry {
                    ticker: "FXAIX".into(),
                    name: "".into(),
                    asset_class: AssetClass::UsLargeCap,
                    expense_ratio: 0.00015,
                    tlh_partner: None,
                    transferable: true,
                    minimum_investment: 0.0,
                },
                target_weight: 0.50,
                dollar_amount: 500_000.0,
            },
        ];
        let er = blended_expense_ratio(&allocs);
        assert!((er - 0.000075).abs() < 0.00001);
    }

    #[test]
    fn test_blended_expense_ratio_empty() {
        assert!((blended_expense_ratio(&[])).abs() < f64::EPSILON);
    }

    #[test]
    fn test_tlh_partner_exists() {
        let cat = brokerage_catalog(&Brokerage::Fidelity);
        let fzrox = cat.funds.iter().find(|f| f.ticker == "FZROX").unwrap();
        assert_eq!(fzrox.tlh_partner.as_deref(), Some("FSKAX"));
    }

    #[test]
    fn test_map_preserves_weight_sum() {
        let weights = vec![
            (AssetClass::UsBroadMarket, 0.40),
            (AssetClass::UsBonds, 0.30),
            (AssetClass::IntlDeveloped, 0.30),
        ];
        let result = map_to_funds(&weights, &Brokerage::Fidelity, 1_000_000.0);
        let sum: f64 = result.iter().map(|a| a.target_weight).sum();
        assert!((sum - 1.0).abs() < f64::EPSILON);
    }
}
