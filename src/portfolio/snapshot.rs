use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use super::types::Portfolio;

/// Serialisable snapshot of portfolio state, suitable for DB persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioSnapshot {
    pub as_of_date: NaiveDate,
    pub total_value: f64,
    pub cash: f64,
    pub positions_json: String,
    pub hhi: f64,
}

/// Per-position summary stored inside `positions_json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionSummary {
    pub ticker: String,
    pub shares: f64,
    pub price: f64,
    pub weight: f64,
}

/// Compute a snapshot from a live `Portfolio`.
pub fn compute_snapshot(portfolio: &Portfolio) -> PortfolioSnapshot {
    let total = portfolio.total_market_value();
    let weights = portfolio.weights();

    let summaries: Vec<PositionSummary> = portfolio
        .positions
        .iter()
        .map(|p| {
            let weight = weights
                .iter()
                .find(|(t, _)| *t == p.ticker.as_str())
                .map(|(_, w)| *w)
                .unwrap_or(0.0);
            PositionSummary {
                ticker: p.ticker.clone(),
                shares: p.total_shares(),
                price: p.current_price,
                weight,
            }
        })
        .collect();

    let positions_json = serde_json::to_string(&summaries).unwrap_or_else(|_| "[]".to_string());

    PortfolioSnapshot {
        as_of_date: portfolio.as_of,
        total_value: total,
        cash: portfolio.cash,
        positions_json,
        hhi: portfolio.hhi(),
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::types::{AcquisitionType, Portfolio, Position, TaxLot};
    use chrono::NaiveDate;
    use uuid::Uuid;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn make_lot(ticker: &str, shares: f64, cost: f64, date: &str) -> TaxLot {
        TaxLot {
            id: Uuid::new_v4(),
            ticker: ticker.to_string(),
            shares,
            cost_basis_per_share: cost,
            acquired_date: d(date),
            acquisition_type: AcquisitionType::Buy,
            remaining_shares: shares,
        }
    }

    fn sample_portfolio() -> Portfolio {
        Portfolio {
            positions: vec![
                Position {
                    ticker: "AMZN".to_string(),
                    lots: vec![make_lot("AMZN", 100.0, 150.0, "2022-01-01")],
                    current_price: 200.0,
                    last_updated: d("2024-12-01"),
                },
                Position {
                    ticker: "AAPL".to_string(),
                    lots: vec![make_lot("AAPL", 50.0, 100.0, "2023-01-01")],
                    current_price: 200.0,
                    last_updated: d("2024-12-01"),
                },
            ],
            cash: 10000.0,
            as_of: d("2024-12-01"),
        }
    }

    #[test]
    fn snapshot_captures_total_value() {
        let snap = compute_snapshot(&sample_portfolio());
        // AMZN: 20000 + AAPL: 10000 + cash: 10000 = 40000
        assert!((snap.total_value - 40000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn snapshot_captures_cash() {
        let snap = compute_snapshot(&sample_portfolio());
        assert!((snap.cash - 10000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn snapshot_captures_hhi() {
        let snap = compute_snapshot(&sample_portfolio());
        // 0.5^2 + 0.25^2 = 0.3125
        assert!((snap.hhi - 0.3125).abs() < f64::EPSILON);
    }

    #[test]
    fn snapshot_captures_date() {
        let snap = compute_snapshot(&sample_portfolio());
        assert_eq!(snap.as_of_date, d("2024-12-01"));
    }

    #[test]
    fn snapshot_positions_json_is_valid() {
        let snap = compute_snapshot(&sample_portfolio());
        let summaries: Vec<PositionSummary> = serde_json::from_str(&snap.positions_json).unwrap();
        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].ticker, "AMZN");
        assert!((summaries[0].shares - 100.0).abs() < f64::EPSILON);
        assert!((summaries[0].weight - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn snapshot_empty_portfolio() {
        let p = Portfolio {
            positions: vec![],
            cash: 0.0,
            as_of: d("2024-12-01"),
        };
        let snap = compute_snapshot(&p);
        assert!((snap.total_value).abs() < f64::EPSILON);
        assert!((snap.hhi).abs() < f64::EPSILON);
        let summaries: Vec<PositionSummary> = serde_json::from_str(&snap.positions_json).unwrap();
        assert!(summaries.is_empty());
    }
}
