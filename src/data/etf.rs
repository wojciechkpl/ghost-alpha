use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtfSnapshot {
    pub ticker: String,
    pub name: String,
    pub expense_ratio: f64,
    pub aum: f64,
    pub avg_daily_volume: u64,
    pub category: String,
    pub inception_date: String,
    pub tracking_index: String,
    pub top_holdings: Vec<EtfHolding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtfHolding {
    pub ticker: String,
    pub name: String,
    pub weight: f64,
}

/// Async trait for fetching ETF data from external providers.
#[async_trait::async_trait]
pub trait EtfProvider: Send + Sync {
    async fn fetch_etf_snapshot(
        &self,
        ticker: &str,
    ) -> Result<EtfSnapshot, crate::error::DataError>;

    async fn fetch_etf_holdings(
        &self,
        ticker: &str,
    ) -> Result<Vec<EtfHolding>, crate::error::DataError>;
}

// ─── Overlap Analysis ────────────────────────────────────────────────────────

/// Check how much of a specific stock is held within an ETF.
/// Returns the weight (e.g. 0.065 for 6.5%) if found, None otherwise.
pub fn overlap_analysis(holdings: &[EtfHolding], concentrated_ticker: &str) -> Option<f64> {
    holdings
        .iter()
        .find(|h| h.ticker == concentrated_ticker)
        .map(|h| h.weight)
}

// ─── ETF Comparison ──────────────────────────────────────────────────────────

/// Side-by-side comparison of multiple ETFs.
#[derive(Debug, Clone, Serialize)]
pub struct EtfComparison {
    pub snapshots: Vec<EtfSnapshot>,
    pub common_holdings: Vec<CommonHolding>,
}

/// A holding that appears in more than one ETF.
#[derive(Debug, Clone, Serialize)]
pub struct CommonHolding {
    pub ticker: String,
    pub name: String,
    /// (etf_ticker, weight_in_that_etf)
    pub weights: Vec<(String, f64)>,
}

/// Find holdings that appear in multiple ETFs.
pub fn compare_etfs(snapshots: &[EtfSnapshot]) -> EtfComparison {
    let mut holding_map: HashMap<String, Vec<(String, f64, String)>> = HashMap::new();

    for snap in snapshots {
        for h in &snap.top_holdings {
            holding_map.entry(h.ticker.clone()).or_default().push((
                snap.ticker.clone(),
                h.weight,
                h.name.clone(),
            ));
        }
    }

    let mut common_holdings: Vec<CommonHolding> = holding_map
        .into_iter()
        .filter(|(_, entries)| entries.len() > 1)
        .map(|(ticker, entries)| CommonHolding {
            name: entries[0].2.clone(),
            ticker,
            weights: entries.into_iter().map(|(etf, w, _)| (etf, w)).collect(),
        })
        .collect();

    // Deterministic ordering for tests
    common_holdings.sort_by(|a, b| a.ticker.cmp(&b.ticker));

    EtfComparison {
        snapshots: snapshots.to_vec(),
        common_holdings,
    }
}

// ─── Bundled ETF Data ────────────────────────────────────────────────────────

/// Bundled ETF metadata for asset class proxies.
/// Updated periodically -- provides offline capability without API keys.
pub fn bundled_etf_snapshots() -> Vec<EtfSnapshot> {
    vec![
        EtfSnapshot {
            ticker: "VTI".into(),
            name: "Vanguard Total Stock Market ETF".into(),
            expense_ratio: 0.0003,
            aum: 400_000_000_000.0,
            avg_daily_volume: 3_500_000,
            category: "US Total Market".into(),
            inception_date: "2001-05-24".into(),
            tracking_index: "CRSP US Total Market Index".into(),
            top_holdings: vec![
                EtfHolding {
                    ticker: "AAPL".into(),
                    name: "Apple Inc".into(),
                    weight: 0.065,
                },
                EtfHolding {
                    ticker: "MSFT".into(),
                    name: "Microsoft Corp".into(),
                    weight: 0.060,
                },
                EtfHolding {
                    ticker: "NVDA".into(),
                    name: "NVIDIA Corp".into(),
                    weight: 0.055,
                },
                EtfHolding {
                    ticker: "AMZN".into(),
                    name: "Amazon.com Inc".into(),
                    weight: 0.035,
                },
                EtfHolding {
                    ticker: "GOOGL".into(),
                    name: "Alphabet Inc".into(),
                    weight: 0.020,
                },
                EtfHolding {
                    ticker: "META".into(),
                    name: "Meta Platforms".into(),
                    weight: 0.025,
                },
                EtfHolding {
                    ticker: "TSLA".into(),
                    name: "Tesla Inc".into(),
                    weight: 0.015,
                },
                EtfHolding {
                    ticker: "BRK.B".into(),
                    name: "Berkshire Hathaway".into(),
                    weight: 0.015,
                },
                EtfHolding {
                    ticker: "JPM".into(),
                    name: "JPMorgan Chase".into(),
                    weight: 0.012,
                },
                EtfHolding {
                    ticker: "UNH".into(),
                    name: "UnitedHealth Group".into(),
                    weight: 0.011,
                },
            ],
        },
        EtfSnapshot {
            ticker: "VOO".into(),
            name: "Vanguard S&P 500 ETF".into(),
            expense_ratio: 0.0003,
            aum: 500_000_000_000.0,
            avg_daily_volume: 4_000_000,
            category: "US Large Cap".into(),
            inception_date: "2010-09-07".into(),
            tracking_index: "S&P 500 Index".into(),
            top_holdings: vec![
                EtfHolding {
                    ticker: "AAPL".into(),
                    name: "Apple Inc".into(),
                    weight: 0.072,
                },
                EtfHolding {
                    ticker: "MSFT".into(),
                    name: "Microsoft Corp".into(),
                    weight: 0.066,
                },
                EtfHolding {
                    ticker: "NVDA".into(),
                    name: "NVIDIA Corp".into(),
                    weight: 0.061,
                },
                EtfHolding {
                    ticker: "AMZN".into(),
                    name: "Amazon.com Inc".into(),
                    weight: 0.038,
                },
                EtfHolding {
                    ticker: "META".into(),
                    name: "Meta Platforms".into(),
                    weight: 0.027,
                },
                EtfHolding {
                    ticker: "GOOGL".into(),
                    name: "Alphabet Inc".into(),
                    weight: 0.022,
                },
                EtfHolding {
                    ticker: "BRK.B".into(),
                    name: "Berkshire Hathaway".into(),
                    weight: 0.017,
                },
                EtfHolding {
                    ticker: "LLY".into(),
                    name: "Eli Lilly".into(),
                    weight: 0.016,
                },
                EtfHolding {
                    ticker: "JPM".into(),
                    name: "JPMorgan Chase".into(),
                    weight: 0.013,
                },
                EtfHolding {
                    ticker: "UNH".into(),
                    name: "UnitedHealth Group".into(),
                    weight: 0.012,
                },
            ],
        },
        EtfSnapshot {
            ticker: "VB".into(),
            name: "Vanguard Small-Cap ETF".into(),
            expense_ratio: 0.0005,
            aum: 55_000_000_000.0,
            avg_daily_volume: 900_000,
            category: "US Small Cap".into(),
            inception_date: "2004-01-26".into(),
            tracking_index: "CRSP US Small Cap Index".into(),
            top_holdings: vec![
                EtfHolding {
                    ticker: "SMCI".into(),
                    name: "Super Micro Computer".into(),
                    weight: 0.007,
                },
                EtfHolding {
                    ticker: "DECK".into(),
                    name: "Deckers Outdoor".into(),
                    weight: 0.005,
                },
                EtfHolding {
                    ticker: "SAIA".into(),
                    name: "Saia Inc".into(),
                    weight: 0.004,
                },
            ],
        },
        EtfSnapshot {
            ticker: "VEA".into(),
            name: "Vanguard FTSE Developed Markets ETF".into(),
            expense_ratio: 0.0005,
            aum: 130_000_000_000.0,
            avg_daily_volume: 8_000_000,
            category: "International Developed".into(),
            inception_date: "2007-07-20".into(),
            tracking_index: "FTSE Developed All Cap ex US Index".into(),
            top_holdings: vec![
                EtfHolding {
                    ticker: "NESN.SW".into(),
                    name: "Nestle SA".into(),
                    weight: 0.017,
                },
                EtfHolding {
                    ticker: "NOVO-B.CO".into(),
                    name: "Novo Nordisk".into(),
                    weight: 0.016,
                },
                EtfHolding {
                    ticker: "ASML.AS".into(),
                    name: "ASML Holding".into(),
                    weight: 0.015,
                },
                EtfHolding {
                    ticker: "AZN.L".into(),
                    name: "AstraZeneca".into(),
                    weight: 0.012,
                },
                EtfHolding {
                    ticker: "SAP.DE".into(),
                    name: "SAP SE".into(),
                    weight: 0.011,
                },
                EtfHolding {
                    ticker: "7203.T".into(),
                    name: "Toyota Motor".into(),
                    weight: 0.010,
                },
            ],
        },
        EtfSnapshot {
            ticker: "VWO".into(),
            name: "Vanguard FTSE Emerging Markets ETF".into(),
            expense_ratio: 0.0008,
            aum: 85_000_000_000.0,
            avg_daily_volume: 7_000_000,
            category: "Emerging Markets".into(),
            inception_date: "2005-03-04".into(),
            tracking_index: "FTSE Emerging Markets All Cap China A Inclusion Index".into(),
            top_holdings: vec![
                EtfHolding {
                    ticker: "2330.TW".into(),
                    name: "Taiwan Semiconductor".into(),
                    weight: 0.090,
                },
                EtfHolding {
                    ticker: "9988.HK".into(),
                    name: "Alibaba Group".into(),
                    weight: 0.030,
                },
                EtfHolding {
                    ticker: "RELIANCE.NS".into(),
                    name: "Reliance Industries".into(),
                    weight: 0.015,
                },
            ],
        },
        EtfSnapshot {
            ticker: "BND".into(),
            name: "Vanguard Total Bond Market ETF".into(),
            expense_ratio: 0.0003,
            aum: 110_000_000_000.0,
            avg_daily_volume: 5_500_000,
            category: "US Aggregate Bond".into(),
            inception_date: "2007-04-03".into(),
            tracking_index: "Bloomberg US Aggregate Float Adjusted Index".into(),
            // Bond ETFs hold fixed-income instruments, not equities
            top_holdings: vec![],
        },
        EtfSnapshot {
            ticker: "TIP".into(),
            name: "iShares TIPS Bond ETF".into(),
            expense_ratio: 0.0019,
            aum: 20_000_000_000.0,
            avg_daily_volume: 3_000_000,
            category: "TIPS".into(),
            inception_date: "2003-12-04".into(),
            tracking_index: "Bloomberg US Treasury Inflation-Linked Bond Index".into(),
            top_holdings: vec![],
        },
        EtfSnapshot {
            ticker: "HYG".into(),
            name: "iShares iBoxx High Yield Corporate Bond ETF".into(),
            expense_ratio: 0.0049,
            aum: 16_000_000_000.0,
            avg_daily_volume: 20_000_000,
            category: "High Yield Bond".into(),
            inception_date: "2007-04-04".into(),
            tracking_index: "Markit iBoxx USD Liquid High Yield Index".into(),
            top_holdings: vec![],
        },
        EtfSnapshot {
            ticker: "VNQ".into(),
            name: "Vanguard Real Estate ETF".into(),
            expense_ratio: 0.0012,
            aum: 35_000_000_000.0,
            avg_daily_volume: 3_200_000,
            category: "US REITs".into(),
            inception_date: "2004-09-23".into(),
            tracking_index: "MSCI US Investable Market Real Estate 25/50 Index".into(),
            top_holdings: vec![
                EtfHolding {
                    ticker: "PLD".into(),
                    name: "Prologis Inc".into(),
                    weight: 0.085,
                },
                EtfHolding {
                    ticker: "AMT".into(),
                    name: "American Tower Corp".into(),
                    weight: 0.065,
                },
                EtfHolding {
                    ticker: "EQIX".into(),
                    name: "Equinix Inc".into(),
                    weight: 0.055,
                },
                EtfHolding {
                    ticker: "WELL".into(),
                    name: "Welltower Inc".into(),
                    weight: 0.040,
                },
                EtfHolding {
                    ticker: "SPG".into(),
                    name: "Simon Property Group".into(),
                    weight: 0.035,
                },
            ],
        },
        EtfSnapshot {
            ticker: "GLD".into(),
            name: "SPDR Gold Shares".into(),
            expense_ratio: 0.0040,
            aum: 60_000_000_000.0,
            avg_daily_volume: 6_000_000,
            category: "Commodities - Gold".into(),
            inception_date: "2004-11-18".into(),
            tracking_index: "LBMA Gold Price".into(),
            // Physical gold ETF -- no equity holdings
            top_holdings: vec![],
        },
        EtfSnapshot {
            ticker: "BNDX".into(),
            name: "Vanguard Total International Bond ETF".into(),
            expense_ratio: 0.0007,
            aum: 55_000_000_000.0,
            avg_daily_volume: 2_500_000,
            category: "International Bond".into(),
            inception_date: "2013-05-31".into(),
            tracking_index: "Bloomberg Global Aggregate ex-USD Float Adjusted RIC Capped Index"
                .into(),
            top_holdings: vec![],
        },
        EtfSnapshot {
            ticker: "DBC".into(),
            name: "Invesco DB Commodity Index Tracking Fund".into(),
            expense_ratio: 0.0087,
            aum: 3_000_000_000.0,
            avg_daily_volume: 1_500_000,
            category: "Broad Commodities".into(),
            inception_date: "2006-02-03".into(),
            tracking_index: "DBIQ Optimum Yield Diversified Commodity Index".into(),
            top_holdings: vec![],
        },
        EtfSnapshot {
            ticker: "GOVT".into(),
            name: "iShares US Treasury Bond ETF".into(),
            expense_ratio: 0.0005,
            aum: 28_000_000_000.0,
            avg_daily_volume: 4_000_000,
            category: "US Treasury".into(),
            inception_date: "2012-02-14".into(),
            tracking_index: "ICE US Treasury Core Bond Index".into(),
            top_holdings: vec![],
        },
    ]
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── overlap_analysis ─────────────────────────────────────────────────

    #[test]
    fn test_overlap_finds_amzn_in_vti() {
        let snapshots = bundled_etf_snapshots();
        let vti = snapshots.iter().find(|s| s.ticker == "VTI").unwrap();
        let weight = overlap_analysis(&vti.top_holdings, "AMZN");
        assert!(weight.is_some(), "AMZN should be in VTI holdings");
        let w = weight.unwrap();
        assert!(
            (w - 0.035).abs() < 1e-9,
            "AMZN weight should be ~3.5%, got {w}"
        );
    }

    #[test]
    fn test_overlap_missing_ticker_returns_none() {
        let snapshots = bundled_etf_snapshots();
        let vti = snapshots.iter().find(|s| s.ticker == "VTI").unwrap();
        let weight = overlap_analysis(&vti.top_holdings, "XYZ");
        assert!(weight.is_none(), "XYZ should not be in VTI");
    }

    // ── compare_etfs ─────────────────────────────────────────────────────

    #[test]
    fn test_compare_finds_common_holdings() {
        let snapshots = bundled_etf_snapshots();
        let vti = snapshots
            .iter()
            .find(|s| s.ticker == "VTI")
            .unwrap()
            .clone();
        let voo = snapshots
            .iter()
            .find(|s| s.ticker == "VOO")
            .unwrap()
            .clone();

        let comparison = compare_etfs(&[vti, voo]);
        assert!(
            !comparison.common_holdings.is_empty(),
            "VTI and VOO should share common holdings"
        );

        let aapl = comparison
            .common_holdings
            .iter()
            .find(|h| h.ticker == "AAPL");
        assert!(aapl.is_some(), "AAPL should be a common holding");
        let aapl = aapl.unwrap();
        assert_eq!(aapl.weights.len(), 2, "AAPL should appear in both ETFs");
    }

    #[test]
    fn test_compare_no_common_with_bond_etf() {
        let snapshots = bundled_etf_snapshots();
        let vti = snapshots
            .iter()
            .find(|s| s.ticker == "VTI")
            .unwrap()
            .clone();
        let bnd = snapshots
            .iter()
            .find(|s| s.ticker == "BND")
            .unwrap()
            .clone();

        let comparison = compare_etfs(&[vti, bnd]);
        assert!(
            comparison.common_holdings.is_empty(),
            "VTI and BND should have no common stock holdings"
        );
    }

    // ── bundled_etf_snapshots ────────────────────────────────────────────

    #[test]
    fn test_bundled_snapshots_not_empty() {
        let snapshots = bundled_etf_snapshots();
        assert!(
            snapshots.len() >= 6,
            "Should have at least 6 bundled ETFs, got {}",
            snapshots.len()
        );
    }

    #[test]
    fn test_bundled_snapshots_vti_has_holdings() {
        let snapshots = bundled_etf_snapshots();
        let vti = snapshots.iter().find(|s| s.ticker == "VTI").unwrap();
        assert!(
            vti.top_holdings.len() >= 5,
            "VTI should have at least 5 top holdings, got {}",
            vti.top_holdings.len()
        );
    }

    #[test]
    fn test_bundled_expense_ratios_reasonable() {
        let snapshots = bundled_etf_snapshots();
        for snap in &snapshots {
            assert!(
                snap.expense_ratio >= 0.0 && snap.expense_ratio <= 0.01,
                "{} has unreasonable expense ratio: {}",
                snap.ticker,
                snap.expense_ratio
            );
        }
    }

    #[test]
    fn test_etf_snapshot_serde_roundtrip() {
        let snapshots = bundled_etf_snapshots();
        let vti = snapshots.iter().find(|s| s.ticker == "VTI").unwrap();

        let json = serde_json::to_string(vti).expect("serialize");
        let deserialized: EtfSnapshot = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.ticker, "VTI");
        assert_eq!(deserialized.name, vti.name);
        assert_eq!(deserialized.top_holdings.len(), vti.top_holdings.len());
        assert!((deserialized.expense_ratio - vti.expense_ratio).abs() < f64::EPSILON);
    }

    // ── Edge cases ───────────────────────────────────────────────────────

    #[test]
    fn test_compare_single_etf_no_common() {
        let snapshots = bundled_etf_snapshots();
        let vti = snapshots
            .iter()
            .find(|s| s.ticker == "VTI")
            .unwrap()
            .clone();
        let comparison = compare_etfs(&[vti]);
        assert!(
            comparison.common_holdings.is_empty(),
            "Single ETF cannot have common holdings"
        );
    }

    #[test]
    fn test_overlap_empty_holdings() {
        let weight = overlap_analysis(&[], "AAPL");
        assert!(weight.is_none());
    }

    #[test]
    fn test_holding_weights_sum_reasonable() {
        let snapshots = bundled_etf_snapshots();
        for snap in &snapshots {
            let total: f64 = snap.top_holdings.iter().map(|h| h.weight).sum();
            // Top holdings are a subset, so total should be < 1.0
            assert!(
                total <= 1.0,
                "{} top holdings weights sum to {}, should be <= 1.0",
                snap.ticker,
                total
            );
        }
    }
}
