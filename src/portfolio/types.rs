use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::PortfolioError;

// ─── Acquisition Type ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AcquisitionType {
    Buy,
    RsuVest,
    Espp,
    Gift,
    Inheritance,
}

impl AcquisitionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::RsuVest => "RSU_VEST",
            Self::Espp => "ESPP",
            Self::Gift => "GIFT",
            Self::Inheritance => "INHERITANCE",
        }
    }

    pub fn from_str_flexible(s: &str) -> Result<Self, PortfolioError> {
        match s.trim().to_uppercase().as_str() {
            "BUY" | "BOUGHT" | "MARKET BUY" | "LIMIT BUY" => Ok(Self::Buy),
            "RSU" | "RSU_VEST" | "RSU VEST" | "VEST" => Ok(Self::RsuVest),
            "ESPP" | "ESPP PURCHASE" => Ok(Self::Espp),
            "GIFT" | "GIFTED" => Ok(Self::Gift),
            "INHERITANCE" | "INHERITED" => Ok(Self::Inheritance),
            other => Err(PortfolioError::UnknownAcquisitionType(other.to_string())),
        }
    }
}

impl std::fmt::Display for AcquisitionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ─── Tax Lot ─────────────────────────────────────────────────────────────────

/// A single tax lot -- the atomic unit of cost basis tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxLot {
    pub id: Uuid,
    pub ticker: String,
    pub shares: f64,
    pub cost_basis_per_share: f64,
    pub acquired_date: NaiveDate,
    pub acquisition_type: AcquisitionType,
    /// Remaining shares after partial sales.
    pub remaining_shares: f64,
}

impl TaxLot {
    /// Total cost basis for the remaining shares in this lot.
    pub fn total_cost_basis(&self) -> f64 {
        self.remaining_shares * self.cost_basis_per_share
    }

    /// Number of calendar days held as of `as_of`.
    pub fn holding_period_days(&self, as_of: NaiveDate) -> i64 {
        (as_of - self.acquired_date).num_days()
    }

    /// Whether the holding qualifies for long-term capital gains (> 365 days).
    pub fn is_long_term(&self, as_of: NaiveDate) -> bool {
        self.holding_period_days(as_of) > 365
    }

    /// Unrealised gain/loss given a current market price.
    pub fn unrealised_gain(&self, current_price: f64) -> f64 {
        (current_price - self.cost_basis_per_share) * self.remaining_shares
    }

    /// Days remaining until this lot qualifies for long-term capital gains.
    /// Returns `None` if already long-term.
    pub fn days_to_long_term(&self, as_of: NaiveDate) -> Option<i64> {
        if self.is_long_term(as_of) {
            return None;
        }
        Some(366 - self.holding_period_days(as_of))
    }
}

// ─── Position ────────────────────────────────────────────────────────────────

/// Aggregated position for a single ticker, composed of one or more tax lots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub ticker: String,
    pub lots: Vec<TaxLot>,
    pub current_price: f64,
    pub last_updated: NaiveDate,
}

impl Position {
    pub fn total_shares(&self) -> f64 {
        self.lots.iter().map(|l| l.remaining_shares).sum()
    }

    pub fn total_cost_basis(&self) -> f64 {
        self.lots.iter().map(|l| l.total_cost_basis()).sum()
    }

    pub fn market_value(&self) -> f64 {
        self.total_shares() * self.current_price
    }

    pub fn total_unrealised_gain(&self) -> f64 {
        self.lots
            .iter()
            .map(|l| l.unrealised_gain(self.current_price))
            .sum()
    }

    /// Volume-weighted average cost basis per share.
    pub fn weighted_cost_basis(&self) -> f64 {
        let total = self.total_shares();
        if total <= 0.0 {
            return 0.0;
        }
        self.total_cost_basis() / total
    }
}

// ─── Portfolio ───────────────────────────────────────────────────────────────

/// Full portfolio snapshot at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Portfolio {
    pub positions: Vec<Position>,
    pub cash: f64,
    pub as_of: NaiveDate,
}

impl Portfolio {
    /// Total market value including cash.
    pub fn total_market_value(&self) -> f64 {
        self.positions.iter().map(|p| p.market_value()).sum::<f64>() + self.cash
    }

    /// Position weights as fraction of total portfolio value.
    pub fn weights(&self) -> Vec<(&str, f64)> {
        let total = self.total_market_value();
        if total <= 0.0 {
            return Vec::new();
        }
        self.positions
            .iter()
            .map(|p| (p.ticker.as_str(), p.market_value() / total))
            .collect()
    }

    /// Herfindahl-Hirschman Index — sum of squared weights.
    /// 1.0 = single position, 1/N = perfectly equal-weighted.
    pub fn hhi(&self) -> f64 {
        self.weights().iter().map(|(_, w)| w * w).sum()
    }

    /// Look up a position by ticker.
    pub fn position(&self, ticker: &str) -> Option<&Position> {
        self.positions.iter().find(|p| p.ticker == ticker)
    }
}

// ─── Disposal ────────────────────────────────────────────────────────────────

/// A recorded sale/disposal event for audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Disposal {
    pub id: Uuid,
    pub lot_id: Uuid,
    pub ticker: String,
    pub shares_sold: f64,
    pub sale_price: f64,
    pub sale_date: NaiveDate,
    pub proceeds: f64,
    pub cost_basis: f64,
    pub realised_gain: f64,
    pub is_long_term: bool,
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn make_lot(ticker: &str, shares: f64, cost: f64, date: &str, remaining: f64) -> TaxLot {
        TaxLot {
            id: Uuid::new_v4(),
            ticker: ticker.to_string(),
            shares,
            cost_basis_per_share: cost,
            acquired_date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            acquisition_type: AcquisitionType::Buy,
            remaining_shares: remaining,
        }
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    // ── AcquisitionType ──────────────────────────────────────────────────

    #[test]
    fn acquisition_type_roundtrip() {
        let cases = [
            ("BUY", AcquisitionType::Buy),
            ("RSU_VEST", AcquisitionType::RsuVest),
            ("ESPP", AcquisitionType::Espp),
            ("GIFT", AcquisitionType::Gift),
            ("INHERITANCE", AcquisitionType::Inheritance),
        ];
        for (s, expected) in &cases {
            let parsed = AcquisitionType::from_str_flexible(s).unwrap();
            assert_eq!(&parsed, expected);
            // Display -> from_str_flexible roundtrip
            let display = parsed.as_str();
            let back = AcquisitionType::from_str_flexible(display).unwrap();
            assert_eq!(&back, expected);
        }
    }

    #[test]
    fn acquisition_type_flexible_parsing() {
        assert_eq!(
            AcquisitionType::from_str_flexible("  bought ").unwrap(),
            AcquisitionType::Buy
        );
        assert_eq!(
            AcquisitionType::from_str_flexible("RSU Vest").unwrap(),
            AcquisitionType::RsuVest
        );
        assert_eq!(
            AcquisitionType::from_str_flexible("ESPP PURCHASE").unwrap(),
            AcquisitionType::Espp
        );
    }

    #[test]
    fn acquisition_type_unknown_returns_error() {
        let result = AcquisitionType::from_str_flexible("MAGIC");
        assert!(result.is_err());
    }

    // ── TaxLot ───────────────────────────────────────────────────────────

    #[test]
    fn tax_lot_total_cost_basis() {
        let lot = make_lot("AMZN", 100.0, 150.0, "2023-01-15", 60.0);
        assert!((lot.total_cost_basis() - 9000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn tax_lot_holding_period_days() {
        let lot = make_lot("AMZN", 100.0, 150.0, "2023-01-15", 100.0);
        // 2023-01-15 to 2024-01-15 = 366 days (2024 is a leap year)
        assert_eq!(lot.holding_period_days(d("2024-01-15")), 365);
        assert_eq!(lot.holding_period_days(d("2023-01-15")), 0);
        assert_eq!(lot.holding_period_days(d("2023-02-14")), 30);
    }

    #[test]
    fn tax_lot_is_long_term() {
        let lot = make_lot("AMZN", 100.0, 150.0, "2023-01-15", 100.0);
        // 365 days = NOT long term (must be > 365)
        assert!(!lot.is_long_term(d("2024-01-15")));
        // 366 days = long term (2024-01-16 is day 366)
        assert!(lot.is_long_term(d("2024-01-16")));
        // Recent = short term
        assert!(!lot.is_long_term(d("2023-06-01")));
    }

    #[test]
    fn tax_lot_unrealised_gain_profit() {
        let lot = make_lot("AMZN", 100.0, 150.0, "2023-01-15", 50.0);
        let gain = lot.unrealised_gain(200.0);
        assert!((gain - 2500.0).abs() < f64::EPSILON); // (200-150)*50
    }

    #[test]
    fn tax_lot_unrealised_gain_loss() {
        let lot = make_lot("AMZN", 100.0, 150.0, "2023-01-15", 50.0);
        let gain = lot.unrealised_gain(100.0);
        assert!((gain - (-2500.0)).abs() < f64::EPSILON); // (100-150)*50
    }

    #[test]
    fn tax_lot_zero_remaining_shares() {
        let lot = make_lot("AMZN", 100.0, 150.0, "2023-01-15", 0.0);
        assert!((lot.total_cost_basis()).abs() < f64::EPSILON);
        assert!((lot.unrealised_gain(200.0)).abs() < f64::EPSILON);
    }

    // ── days_to_long_term ───────────────────────────────────────────────

    #[test]
    fn test_days_to_long_term_already_lt() {
        let lot = make_lot("AMZN", 100.0, 150.0, "2023-01-15", 100.0);
        // 400 days held => already long-term
        assert!(lot.days_to_long_term(d("2024-02-19")).is_none());
    }

    #[test]
    fn test_days_to_long_term_exactly_366() {
        let lot = make_lot("AMZN", 100.0, 150.0, "2023-01-15", 100.0);
        // 366 days => already LT (>365)
        assert!(lot.days_to_long_term(d("2024-01-16")).is_none());
    }

    #[test]
    fn test_days_to_long_term_day_365() {
        let lot = make_lot("AMZN", 100.0, 150.0, "2023-01-15", 100.0);
        // Exactly 365 days = NOT long-term, need 1 more day
        assert_eq!(lot.days_to_long_term(d("2024-01-15")), Some(1));
    }

    #[test]
    fn test_days_to_long_term_recent() {
        let lot = make_lot("AMZN", 100.0, 150.0, "2023-01-15", 100.0);
        // 100 days held => 266 days to go
        assert_eq!(lot.days_to_long_term(d("2023-04-25")), Some(266));
    }

    #[test]
    fn test_days_to_long_term_day_zero() {
        let lot = make_lot("AMZN", 100.0, 150.0, "2023-01-15", 100.0);
        // Acquired today => 366 days to go
        assert_eq!(lot.days_to_long_term(d("2023-01-15")), Some(366));
    }

    #[test]
    fn test_days_to_long_term_47_days_left() {
        let lot = make_lot("AMZN", 100.0, 150.0, "2023-01-15", 100.0);
        // 319 days held => 47 days to go
        assert_eq!(lot.days_to_long_term(d("2023-11-30")), Some(47));
    }

    // ── Position ─────────────────────────────────────────────────────────

    fn make_position() -> Position {
        Position {
            ticker: "AMZN".to_string(),
            lots: vec![
                make_lot("AMZN", 100.0, 150.0, "2022-01-01", 100.0),
                make_lot("AMZN", 50.0, 180.0, "2023-06-15", 30.0),
            ],
            current_price: 200.0,
            last_updated: d("2024-12-01"),
        }
    }

    #[test]
    fn position_total_shares() {
        let pos = make_position();
        assert!((pos.total_shares() - 130.0).abs() < f64::EPSILON);
    }

    #[test]
    fn position_total_cost_basis() {
        let pos = make_position();
        // 100*150 + 30*180 = 15000 + 5400 = 20400
        assert!((pos.total_cost_basis() - 20400.0).abs() < f64::EPSILON);
    }

    #[test]
    fn position_market_value() {
        let pos = make_position();
        // 130 * 200 = 26000
        assert!((pos.market_value() - 26000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn position_total_unrealised_gain() {
        let pos = make_position();
        // lot1: (200-150)*100 = 5000, lot2: (200-180)*30 = 600 => 5600
        assert!((pos.total_unrealised_gain() - 5600.0).abs() < f64::EPSILON);
    }

    #[test]
    fn position_weighted_cost_basis() {
        let pos = make_position();
        // 20400 / 130 = 156.923...
        let wcb = pos.weighted_cost_basis();
        assert!((wcb - 156.923_076_923_076_92).abs() < 0.01);
    }

    #[test]
    fn position_zero_shares_weighted_cost_basis() {
        let pos = Position {
            ticker: "GONE".to_string(),
            lots: vec![make_lot("GONE", 100.0, 50.0, "2023-01-01", 0.0)],
            current_price: 100.0,
            last_updated: d("2024-12-01"),
        };
        assert!((pos.weighted_cost_basis()).abs() < f64::EPSILON);
    }

    // ── Portfolio ────────────────────────────────────────────────────────

    fn make_portfolio() -> Portfolio {
        Portfolio {
            positions: vec![
                Position {
                    ticker: "AMZN".to_string(),
                    lots: vec![make_lot("AMZN", 100.0, 150.0, "2022-01-01", 100.0)],
                    current_price: 200.0,
                    last_updated: d("2024-12-01"),
                },
                Position {
                    ticker: "AAPL".to_string(),
                    lots: vec![make_lot("AAPL", 50.0, 100.0, "2023-01-01", 50.0)],
                    current_price: 200.0,
                    last_updated: d("2024-12-01"),
                },
            ],
            cash: 10000.0,
            as_of: d("2024-12-01"),
        }
    }

    #[test]
    fn portfolio_total_market_value() {
        let p = make_portfolio();
        // AMZN: 100*200=20000, AAPL: 50*200=10000, cash: 10000 => 40000
        assert!((p.total_market_value() - 40000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn portfolio_weights() {
        let p = make_portfolio();
        let weights = p.weights();
        assert_eq!(weights.len(), 2);
        // AMZN: 20000/40000 = 0.5, AAPL: 10000/40000 = 0.25
        let amzn_w = weights.iter().find(|(t, _)| *t == "AMZN").unwrap().1;
        let aapl_w = weights.iter().find(|(t, _)| *t == "AAPL").unwrap().1;
        assert!((amzn_w - 0.5).abs() < f64::EPSILON);
        assert!((aapl_w - 0.25).abs() < f64::EPSILON);
    }

    #[test]
    fn portfolio_weights_sum_excludes_cash() {
        // Weights sum < 1.0 because cash is part of total_market_value denominator
        let p = make_portfolio();
        let w_sum: f64 = p.weights().iter().map(|(_, w)| w).sum();
        assert!((w_sum - 0.75).abs() < f64::EPSILON); // 30000/40000
    }

    #[test]
    fn portfolio_hhi() {
        let p = make_portfolio();
        // HHI = 0.5^2 + 0.25^2 = 0.25 + 0.0625 = 0.3125
        assert!((p.hhi() - 0.3125).abs() < f64::EPSILON);
    }

    #[test]
    fn portfolio_hhi_single_position() {
        let p = Portfolio {
            positions: vec![Position {
                ticker: "AMZN".to_string(),
                lots: vec![make_lot("AMZN", 100.0, 150.0, "2022-01-01", 100.0)],
                current_price: 200.0,
                last_updated: d("2024-12-01"),
            }],
            cash: 0.0,
            as_of: d("2024-12-01"),
        };
        assert!((p.hhi() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn portfolio_hhi_empty() {
        let p = Portfolio {
            positions: vec![],
            cash: 0.0,
            as_of: d("2024-12-01"),
        };
        // Empty portfolio with zero total value => empty weights => hhi = 0
        assert!((p.hhi()).abs() < f64::EPSILON);
    }

    #[test]
    fn portfolio_position_lookup() {
        let p = make_portfolio();
        assert!(p.position("AMZN").is_some());
        assert!(p.position("AAPL").is_some());
        assert!(p.position("GOOG").is_none());
    }

    #[test]
    fn portfolio_empty_weights() {
        let p = Portfolio {
            positions: vec![],
            cash: 0.0,
            as_of: d("2024-12-01"),
        };
        assert!(p.weights().is_empty());
    }
}
