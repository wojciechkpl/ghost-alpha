use crate::portfolio::types::TaxLot;

/// Select lots using Highest-In, First-Out ordering to minimise
/// realised gain for a given number of shares to sell.
///
/// Returns `(lot_index, shares_from_lot)` pairs.
pub fn hifo_select(lots: &[TaxLot], shares_needed: f64) -> Vec<(usize, f64)> {
    if shares_needed <= 0.0 {
        return Vec::new();
    }

    // Sort lots by cost_basis_per_share descending (highest cost first)
    let mut indexed: Vec<(usize, &TaxLot)> = lots.iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        b.1.cost_basis_per_share
            .partial_cmp(&a.1.cost_basis_per_share)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut remaining = shares_needed;
    let mut selections = Vec::new();

    for (idx, lot) in indexed {
        if remaining <= 0.0 {
            break;
        }
        let available = lot.remaining_shares;
        if available <= 0.0 {
            continue;
        }
        let take = remaining.min(available);
        selections.push((idx, take));
        remaining -= take;
    }

    selections
}

/// Sell priority tier for lot ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SellPriority {
    /// Tier 1: Short-term loss lots — harvest immediately.
    StLoss,
    /// Tier 2: Long-term gain lots, ascending by gain%.
    LtGainAscending,
    /// Tier 3: Short-term gain lots with >=`ltcg_hold_days` to LTCG.
    StGainFar,
    /// Tier 4: Short-term gain lots with <`ltcg_hold_days` to LTCG — hold if possible.
    StGainNearLtcg,
}

/// Select lots using the 3-tier priority algorithm.
///
/// Priority order:
/// 1. ST loss lots (harvest losses, offset gains at ordinary rate)
/// 2. LT gain lots ascending by gain% (lowest gain% = lowest tax)
/// 3. ST gain lots with >=`ltcg_hold_days` to LTCG (no conversion benefit)
/// 4. ST gain lots with <`ltcg_hold_days` to LTCG (hold for LTCG conversion)
///
/// Tier 4 lots are only used if tiers 1-3 cannot fill `shares_needed`.
///
/// Returns `(lot_index, shares_from_lot)` pairs, same format as `hifo_select`.
pub fn priority_select(
    lots: &[TaxLot],
    shares_needed: f64,
    current_price: f64,
    as_of: chrono::NaiveDate,
    ltcg_hold_days: i64,
) -> Vec<(usize, f64)> {
    if shares_needed <= 0.0 {
        return Vec::new();
    }

    // Classify each lot into a priority tier + gain% sort key.
    let mut classified: Vec<(usize, SellPriority, f64)> = lots
        .iter()
        .enumerate()
        .filter(|(_, lot)| lot.remaining_shares > 0.0)
        .map(|(idx, lot)| {
            let gain_pct = if lot.cost_basis_per_share > 0.0 {
                (current_price - lot.cost_basis_per_share) / lot.cost_basis_per_share
            } else {
                0.0
            };
            let is_lt = lot.is_long_term(as_of);
            let is_loss = current_price < lot.cost_basis_per_share;
            let days_to_lt = lot.days_to_long_term(as_of).unwrap_or(0);

            let priority = if !is_lt && is_loss {
                SellPriority::StLoss
            } else if is_lt {
                SellPriority::LtGainAscending
            } else if days_to_lt > ltcg_hold_days {
                SellPriority::StGainFar
            } else {
                SellPriority::StGainNearLtcg
            };
            (idx, priority, gain_pct)
        })
        .collect();

    // Sort: by priority tier (ascending), then by gain% within tier.
    classified.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
    });

    // Fill shares.
    let mut remaining = shares_needed;
    let mut selections = Vec::new();
    for (idx, _, _) in &classified {
        if remaining <= 0.0 {
            break;
        }
        let available = lots[*idx].remaining_shares;
        let take = remaining.min(available);
        selections.push((*idx, take));
        remaining -= take;
    }
    selections
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::types::{AcquisitionType, TaxLot};
    use chrono::NaiveDate;
    use uuid::Uuid;

    fn make_lot(cost: f64, shares: f64) -> TaxLot {
        TaxLot {
            id: Uuid::new_v4(),
            ticker: "TEST".to_string(),
            shares,
            cost_basis_per_share: cost,
            acquired_date: NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
            acquisition_type: AcquisitionType::Buy,
            remaining_shares: shares,
        }
    }

    fn make_lot_remaining(cost: f64, shares: f64, remaining: f64) -> TaxLot {
        TaxLot {
            id: Uuid::new_v4(),
            ticker: "TEST".to_string(),
            shares,
            cost_basis_per_share: cost,
            acquired_date: NaiveDate::from_ymd_opt(2023, 1, 1).unwrap(),
            acquisition_type: AcquisitionType::Buy,
            remaining_shares: remaining,
        }
    }

    #[test]
    fn test_hifo_selects_highest_basis_first() {
        let lots = vec![
            make_lot(50.0, 100.0),  // idx 0
            make_lot(150.0, 100.0), // idx 1 — highest
            make_lot(100.0, 100.0), // idx 2
        ];
        let result = hifo_select(&lots, 50.0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, 1, "Should select lot index 1 ($150 basis)");
        assert!((result[0].1 - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_hifo_partial_lot() {
        let lots = vec![make_lot(150.0, 100.0)];
        let result = hifo_select(&lots, 50.0);
        assert_eq!(result.len(), 1);
        assert!((result[0].1 - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_hifo_multiple_lots_needed() {
        let lots = vec![
            make_lot(50.0, 30.0),  // idx 0
            make_lot(150.0, 40.0), // idx 1 — highest
            make_lot(100.0, 50.0), // idx 2
        ];
        // Need 80 shares: take 40 from lot 1 ($150), then 50 from lot 2 ($100)
        let result = hifo_select(&lots, 80.0);
        assert_eq!(result.len(), 2);
        // First selection: lot 1 (highest), all 40 shares
        assert_eq!(result[0].0, 1);
        assert!((result[0].1 - 40.0).abs() < f64::EPSILON);
        // Second selection: lot 2, 40 shares
        assert_eq!(result[1].0, 2);
        assert!((result[1].1 - 40.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_hifo_zero_shares_needed() {
        let lots = vec![make_lot(100.0, 100.0)];
        let result = hifo_select(&lots, 0.0);
        assert!(result.is_empty());
    }

    #[test]
    fn test_hifo_skips_empty_lots() {
        let lots = vec![
            make_lot_remaining(200.0, 100.0, 0.0), // idx 0 — highest basis but empty
            make_lot(100.0, 50.0),                 // idx 1
        ];
        let result = hifo_select(&lots, 30.0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, 1, "Should skip empty lot and use lot index 1");
        assert!((result[0].1 - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_hifo_exact_fill() {
        let lots = vec![
            make_lot(150.0, 50.0), // idx 0
            make_lot(100.0, 50.0), // idx 1
        ];
        let result = hifo_select(&lots, 100.0);
        assert_eq!(result.len(), 2);
        let total: f64 = result.iter().map(|(_, s)| s).sum();
        assert!((total - 100.0).abs() < f64::EPSILON);
    }

    // ── priority_select tests ─────────────────────────────────────────

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn make_lot_dated(cost: f64, shares: f64, date: &str) -> TaxLot {
        TaxLot {
            id: Uuid::new_v4(),
            ticker: "TEST".to_string(),
            shares,
            cost_basis_per_share: cost,
            acquired_date: d(date),
            acquisition_type: AcquisitionType::Buy,
            remaining_shares: shares,
        }
    }

    #[test]
    fn test_priority_st_loss_first() {
        let lots = vec![
            make_lot_dated(250.0, 100.0, "2024-06-01"), // idx 0: ST loss at price=200
            make_lot_dated(100.0, 100.0, "2020-01-01"), // idx 1: LT gain
        ];
        let result = priority_select(&lots, 50.0, 200.0, d("2024-12-01"), 60);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, 0, "Should select ST loss lot first");
    }

    #[test]
    fn test_priority_lt_gain_before_st_gain() {
        let lots = vec![
            make_lot_dated(100.0, 100.0, "2020-01-01"), // idx 0: LT gain +100%
            make_lot_dated(150.0, 100.0, "2024-06-01"), // idx 1: ST gain (200 days to LTCG)
        ];
        let result = priority_select(&lots, 50.0, 200.0, d("2024-12-01"), 60);
        assert_eq!(result[0].0, 0, "Should select LT gain lot before ST gain");
    }

    #[test]
    fn test_priority_lt_gain_ascending_by_gain_pct() {
        let lots = vec![
            make_lot_dated(180.0, 100.0, "2020-01-01"), // idx 0: LT +11%
            make_lot_dated(100.0, 100.0, "2020-01-01"), // idx 1: LT +100%
        ];
        let result = priority_select(&lots, 50.0, 200.0, d("2024-12-01"), 60);
        assert_eq!(result[0].0, 0, "Should select lower gain% lot first");
    }

    #[test]
    fn test_priority_st_near_ltcg_last() {
        let lots = vec![
            make_lot_dated(150.0, 100.0, "2024-09-01"), // idx 0: ST gain, ~32 days to LTCG at 2025-08-01
            make_lot_dated(150.0, 100.0, "2024-02-01"), // idx 1: LT gain (546 days held)
            make_lot_dated(150.0, 100.0, "2020-01-01"), // idx 2: LT gain
        ];
        // Need 300 shares to force using the near-LTCG lot
        let result = priority_select(&lots, 300.0, 200.0, d("2025-08-01"), 60);
        assert_eq!(result.len(), 3);
        // Last lot used should be the near-LTCG one (idx 0)
        assert_eq!(result.last().unwrap().0, 0, "ST near-LTCG should be last");
    }

    #[test]
    fn test_priority_uses_near_ltcg_if_needed() {
        // Only ST-near-LTCG lots available
        let lots = vec![
            make_lot_dated(150.0, 100.0, "2024-10-01"), // ST gain, ~30 days to LTCG
        ];
        let result = priority_select(&lots, 50.0, 200.0, d("2025-09-01"), 60);
        assert_eq!(
            result.len(),
            1,
            "Should use near-LTCG lot if it's the only option"
        );
    }

    #[test]
    fn test_priority_zero_shares() {
        let lots = vec![make_lot_dated(100.0, 100.0, "2020-01-01")];
        let result = priority_select(&lots, 0.0, 200.0, d("2024-12-01"), 60);
        assert!(result.is_empty());
    }

    #[test]
    fn test_priority_skips_empty_lots() {
        let mut lot = make_lot_dated(250.0, 100.0, "2024-06-01");
        lot.remaining_shares = 0.0;
        let lots = vec![lot, make_lot_dated(100.0, 50.0, "2020-01-01")];
        let result = priority_select(&lots, 30.0, 200.0, d("2024-12-01"), 60);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, 1, "Should skip empty lot");
    }

    #[test]
    fn test_priority_partial_lot_fill() {
        let lots = vec![make_lot_dated(100.0, 100.0, "2020-01-01")];
        let result = priority_select(&lots, 30.0, 200.0, d("2024-12-01"), 60);
        assert_eq!(result.len(), 1);
        assert!((result[0].1 - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_priority_custom_hold_threshold() {
        // Lot is 50 days from LTCG
        let lots = vec![
            make_lot_dated(150.0, 100.0, "2023-12-01"), // ~316 days held at 2024-10-12 => 50 days to LT
            make_lot_dated(100.0, 100.0, "2020-01-01"), // LT gain
        ];
        // With 60-day threshold: lot 0 is "near LTCG" (50 < 60), deferred
        let result60 = priority_select(&lots, 50.0, 200.0, d("2024-10-12"), 60);
        assert_eq!(
            result60[0].0, 1,
            "With 60-day threshold, LT lot should be first"
        );

        // With 30-day threshold: lot 0 is "far" (50 > 30), not deferred
        let result30 = priority_select(&lots, 50.0, 200.0, d("2024-10-12"), 30);
        assert_eq!(
            result30[0].0, 1,
            "With 30-day threshold, LT gain lot still first (tier 2)"
        );
    }
}
