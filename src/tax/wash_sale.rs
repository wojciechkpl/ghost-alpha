//! Wash sale rule enforcement (IRS §1091).
//!
//! A wash sale occurs when a security is sold at a loss and the same (or
//! substantially identical) security is purchased within 30 calendar days
//! before or after the sale. The loss is disallowed for tax purposes.

use chrono::{Duration, NaiveDate};

use crate::portfolio::types::TaxLot;

// ─── Types ──────────────────────────────────────────────────────────────────

/// Result of a wash sale check.
#[derive(Debug, Clone, PartialEq)]
pub enum WashSaleResult {
    /// No wash sale conflict found.
    Clear,
    /// Wash sale violation detected.
    Violation {
        /// Number of lots that conflict within the 30-day window
        conflicting_lots: usize,
        /// Earliest conflicting acquisition date
        earliest: NaiveDate,
        /// Latest conflicting acquisition date
        latest: NaiveDate,
    },
}

/// A proposed or recorded sale for wash sale checking.
#[derive(Debug, Clone)]
pub struct SaleRecord {
    pub ticker: String,
    pub sale_date: NaiveDate,
    pub is_loss: bool,
}

// ─── Core Detection ─────────────────────────────────────────────────────────

/// Check whether a proposed loss sale would trigger wash sale rules.
///
/// Scans `recent_transactions` for purchases of the same ticker within
/// 30 calendar days before or after `sale_date`.
///
/// **Limitation:** This checks exact ticker match only. The IRS wash sale rule
/// also covers "substantially identical" securities (e.g., GOOG vs GOOGL, or
/// an ETF vs a near-identical fund). Those cases are NOT detected here.
pub fn check_wash_sale(
    ticker: &str,
    sale_date: NaiveDate,
    recent_transactions: &[TaxLot],
) -> WashSaleResult {
    let start = sale_date - Duration::days(30);
    let end = sale_date + Duration::days(30);

    let conflicting: Vec<&TaxLot> = recent_transactions
        .iter()
        .filter(|lot| {
            lot.ticker == ticker
                && lot.acquired_date >= start
                && lot.acquired_date <= end
                && lot.acquired_date != sale_date
        })
        .collect();

    if conflicting.is_empty() {
        WashSaleResult::Clear
    } else {
        WashSaleResult::Violation {
            conflicting_lots: conflicting.len(),
            earliest: conflicting.iter().map(|l| l.acquired_date).min().unwrap(),
            latest: conflicting.iter().map(|l| l.acquired_date).max().unwrap(),
        }
    }
}

/// Check whether a proposed **buy** would create wash sale risk.
///
/// If the ticker was sold at a loss within the past 30 days, buying it
/// again would trigger a wash sale on that prior loss sale.
pub fn check_buy_wash_risk(
    ticker: &str,
    buy_date: NaiveDate,
    recent_loss_sales: &[SaleRecord],
) -> WashSaleResult {
    let start = buy_date - Duration::days(30);

    let conflicting: Vec<&SaleRecord> = recent_loss_sales
        .iter()
        .filter(|s| {
            s.ticker == ticker && s.is_loss && s.sale_date >= start && s.sale_date < buy_date
        })
        .collect();

    if conflicting.is_empty() {
        WashSaleResult::Clear
    } else {
        WashSaleResult::Violation {
            conflicting_lots: conflicting.len(),
            earliest: conflicting.iter().map(|s| s.sale_date).min().unwrap(),
            latest: conflicting.iter().map(|s| s.sale_date).max().unwrap(),
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::types::AcquisitionType;
    use uuid::Uuid;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn make_lot(ticker: &str, acquired: &str) -> TaxLot {
        TaxLot {
            id: Uuid::new_v4(),
            ticker: ticker.to_string(),
            shares: 100.0,
            cost_basis_per_share: 50.0,
            acquired_date: d(acquired),
            acquisition_type: AcquisitionType::Buy,
            remaining_shares: 100.0,
        }
    }

    fn make_sale(ticker: &str, date: &str, is_loss: bool) -> SaleRecord {
        SaleRecord {
            ticker: ticker.to_string(),
            sale_date: d(date),
            is_loss,
        }
    }

    // ── check_wash_sale ─────────────────────────────────────────────────────

    #[test]
    fn no_conflicting_purchases_is_clear() {
        let lots = vec![make_lot("MSFT", "2024-01-15")];
        let result = check_wash_sale("AAPL", d("2024-03-01"), &lots);
        assert_eq!(result, WashSaleResult::Clear);
    }

    #[test]
    fn purchase_within_30_days_before_is_violation() {
        let lots = vec![make_lot("AAPL", "2024-02-20")]; // 10 days before sale
        let result = check_wash_sale("AAPL", d("2024-03-01"), &lots);
        assert!(matches!(
            result,
            WashSaleResult::Violation {
                conflicting_lots: 1,
                ..
            }
        ));
    }

    #[test]
    fn purchase_within_30_days_after_is_violation() {
        let lots = vec![make_lot("AAPL", "2024-03-15")]; // 14 days after sale
        let result = check_wash_sale("AAPL", d("2024-03-01"), &lots);
        assert!(matches!(
            result,
            WashSaleResult::Violation {
                conflicting_lots: 1,
                ..
            }
        ));
    }

    #[test]
    fn purchase_on_same_date_excluded() {
        // Same-day is the sale itself, not a conflicting purchase
        let lots = vec![make_lot("AAPL", "2024-03-01")];
        let result = check_wash_sale("AAPL", d("2024-03-01"), &lots);
        assert_eq!(result, WashSaleResult::Clear);
    }

    #[test]
    fn purchase_exactly_30_days_before_is_violation() {
        let lots = vec![make_lot("AAPL", "2024-01-31")]; // exactly 30 days before
        let result = check_wash_sale("AAPL", d("2024-03-01"), &lots);
        assert!(matches!(result, WashSaleResult::Violation { .. }));
    }

    #[test]
    fn purchase_31_days_before_is_clear() {
        let lots = vec![make_lot("AAPL", "2024-01-30")]; // 31 days before
        let result = check_wash_sale("AAPL", d("2024-03-01"), &lots);
        assert_eq!(result, WashSaleResult::Clear);
    }

    #[test]
    fn purchase_exactly_30_days_after_is_violation() {
        let lots = vec![make_lot("AAPL", "2024-03-31")]; // exactly 30 days after
        let result = check_wash_sale("AAPL", d("2024-03-01"), &lots);
        assert!(matches!(result, WashSaleResult::Violation { .. }));
    }

    #[test]
    fn purchase_31_days_after_is_clear() {
        let lots = vec![make_lot("AAPL", "2024-04-01")]; // 31 days after
        let result = check_wash_sale("AAPL", d("2024-03-01"), &lots);
        assert_eq!(result, WashSaleResult::Clear);
    }

    #[test]
    fn different_ticker_is_clear() {
        let lots = vec![make_lot("MSFT", "2024-03-05")]; // same window, different ticker
        let result = check_wash_sale("AAPL", d("2024-03-01"), &lots);
        assert_eq!(result, WashSaleResult::Clear);
    }

    #[test]
    fn multiple_conflicting_lots_counted() {
        let lots = vec![
            make_lot("AAPL", "2024-02-15"),
            make_lot("AAPL", "2024-03-10"),
            make_lot("MSFT", "2024-03-05"), // different ticker, excluded
        ];
        let result = check_wash_sale("AAPL", d("2024-03-01"), &lots);
        match result {
            WashSaleResult::Violation {
                conflicting_lots,
                earliest,
                latest,
            } => {
                assert_eq!(conflicting_lots, 2);
                assert_eq!(earliest, d("2024-02-15"));
                assert_eq!(latest, d("2024-03-10"));
            }
            _ => panic!("Expected Violation"),
        }
    }

    // ── check_buy_wash_risk ─────────────────────────────────────────────────

    #[test]
    fn no_recent_loss_sales_is_clear() {
        let sales = vec![make_sale("AAPL", "2024-01-01", true)]; // >30 days ago
        let result = check_buy_wash_risk("AAPL", d("2024-03-01"), &sales);
        assert_eq!(result, WashSaleResult::Clear);
    }

    #[test]
    fn recent_loss_sale_triggers_buy_risk() {
        let sales = vec![make_sale("AAPL", "2024-02-20", true)]; // 10 days ago
        let result = check_buy_wash_risk("AAPL", d("2024-03-01"), &sales);
        assert!(matches!(
            result,
            WashSaleResult::Violation {
                conflicting_lots: 1,
                ..
            }
        ));
    }

    #[test]
    fn recent_gain_sale_does_not_trigger() {
        let sales = vec![make_sale("AAPL", "2024-02-20", false)]; // gain, not loss
        let result = check_buy_wash_risk("AAPL", d("2024-03-01"), &sales);
        assert_eq!(result, WashSaleResult::Clear);
    }

    #[test]
    fn different_ticker_sale_is_clear() {
        let sales = vec![make_sale("MSFT", "2024-02-20", true)];
        let result = check_buy_wash_risk("AAPL", d("2024-03-01"), &sales);
        assert_eq!(result, WashSaleResult::Clear);
    }
}
