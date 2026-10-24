use chrono::NaiveDate;
use std::collections::HashMap;

use crate::portfolio::types::TaxLot;

/// A tax-loss harvesting opportunity.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HarvestOpportunity {
    pub ticker: String,
    pub lot_id: String,
    pub shares: f64,
    pub cost_basis: f64,
    pub current_value: f64,
    pub unrealised_loss: f64,
    pub days_held: i64,
    pub is_long_term: bool,
    /// True if a wash sale would apply (sold same ticker in last 30 days).
    pub wash_sale_risk: bool,
}

/// Scan all lots for harvestable losses.
///
/// A lot qualifies if its unrealised loss exceeds `min_loss` (as a positive number).
/// Lots whose ticker is missing from `current_prices` are skipped.
pub fn scan_harvest_opportunities(
    lots: &[TaxLot],
    current_prices: &HashMap<String, f64>,
    recent_sales: &[(String, NaiveDate)],
    as_of: NaiveDate,
    min_loss: f64,
) -> Vec<HarvestOpportunity> {
    lots.iter()
        .filter_map(|lot| {
            let price = current_prices.get(&lot.ticker)?;
            let loss = lot.unrealised_gain(*price);
            // loss is negative for actual losses; only include if |loss| >= min_loss
            if loss >= -min_loss {
                return None;
            }

            let wash_risk = recent_sales
                .iter()
                .any(|(t, d)| t == &lot.ticker && (as_of - *d).num_days().abs() <= 30);

            Some(HarvestOpportunity {
                ticker: lot.ticker.clone(),
                lot_id: lot.id.to_string(),
                shares: lot.remaining_shares,
                cost_basis: lot.total_cost_basis(),
                current_value: lot.remaining_shares * price,
                unrealised_loss: loss,
                days_held: lot.holding_period_days(as_of),
                is_long_term: lot.is_long_term(as_of),
                wash_sale_risk: wash_risk,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::types::{AcquisitionType, TaxLot};
    use chrono::NaiveDate;
    use uuid::Uuid;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn make_lot(ticker: &str, cost: f64, shares: f64, date: &str) -> TaxLot {
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

    #[test]
    fn test_finds_losses_above_threshold() {
        let lots = vec![make_lot("AAPL", 150.0, 100.0, "2023-01-01")];
        let mut prices = HashMap::new();
        prices.insert("AAPL".to_string(), 130.0); // loss = (130-150)*100 = -2000
        let result = scan_harvest_opportunities(&lots, &prices, &[], d("2024-06-01"), 1000.0);
        assert_eq!(result.len(), 1);
        assert!(result[0].unrealised_loss < -1000.0);
    }

    #[test]
    fn test_ignores_small_losses() {
        let lots = vec![make_lot("AAPL", 150.0, 100.0, "2023-01-01")];
        let mut prices = HashMap::new();
        prices.insert("AAPL".to_string(), 145.0); // loss = (145-150)*100 = -500
        let result = scan_harvest_opportunities(&lots, &prices, &[], d("2024-06-01"), 1000.0);
        assert!(
            result.is_empty(),
            "Should ignore loss of $500 when min is $1000"
        );
    }

    #[test]
    fn test_ignores_gains() {
        let lots = vec![make_lot("AAPL", 100.0, 100.0, "2023-01-01")];
        let mut prices = HashMap::new();
        prices.insert("AAPL".to_string(), 200.0); // gain = +10000
        let result = scan_harvest_opportunities(&lots, &prices, &[], d("2024-06-01"), 1000.0);
        assert!(
            result.is_empty(),
            "Should not report gains as harvest opportunities"
        );
    }

    #[test]
    fn test_wash_sale_detection() {
        let lots = vec![make_lot("AAPL", 150.0, 100.0, "2023-01-01")];
        let mut prices = HashMap::new();
        prices.insert("AAPL".to_string(), 100.0); // big loss
        let recent_sales = vec![("AAPL".to_string(), d("2024-05-20"))]; // 12 days ago
        let result =
            scan_harvest_opportunities(&lots, &prices, &recent_sales, d("2024-06-01"), 1000.0);
        assert_eq!(result.len(), 1);
        assert!(
            result[0].wash_sale_risk,
            "Should flag wash sale for sale 12 days ago"
        );
    }

    #[test]
    fn test_no_wash_sale_after_31_days() {
        let lots = vec![make_lot("AAPL", 150.0, 100.0, "2023-01-01")];
        let mut prices = HashMap::new();
        prices.insert("AAPL".to_string(), 100.0);
        let recent_sales = vec![("AAPL".to_string(), d("2024-04-25"))]; // 37 days ago
        let result =
            scan_harvest_opportunities(&lots, &prices, &recent_sales, d("2024-06-01"), 1000.0);
        assert_eq!(result.len(), 1);
        assert!(
            !result[0].wash_sale_risk,
            "Should NOT flag wash sale for sale 37 days ago"
        );
    }

    #[test]
    fn test_missing_price_skips_lot() {
        let lots = vec![make_lot("AAPL", 150.0, 100.0, "2023-01-01")];
        let prices = HashMap::new(); // no AAPL price
        let result = scan_harvest_opportunities(&lots, &prices, &[], d("2024-06-01"), 1000.0);
        assert!(result.is_empty(), "Should skip lots with missing prices");
    }
}
