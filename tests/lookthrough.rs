//! RED-phase contract for the pure maths of the concentration hub
//! (`ghost_alpha::portfolio::lookthrough`, new module). Synthetic data only.
//!
//! Public API pinned here:
//!
//! ```ignore
//! pub struct PositionValue { pub ticker: String, pub shares: f64, pub price: f64,
//!                            pub value: f64, pub priced_at_cost: bool }
//! /// Aggregate lots by ticker. value = shares * latest close; no close => price is the
//! /// share-weighted average cost basis and priced_at_cost = true. Lots with shares <= 0
//! /// are ignored. Sorted by value descending, ties by ticker ascending.
//! pub fn value_positions(lots: &[TaxLotRow], closes: &HashMap<String, f64>) -> Vec<PositionValue>;
//!
//! pub struct ConcentrationSummary {
//!     pub symbol: String,               // upper-cased
//!     pub total_value: f64,
//!     pub direct_value: f64,            // 0.0 when symbol is not held
//!     pub direct_weight: f64,           // fraction 0..1
//!     pub top1: (String, f64),          // (ticker, weight fraction) of the largest position
//!     pub effective_n: f64,             // 1 / sum(w^2); 0.0 when total_value == 0
//!     pub weights: Vec<(String, f64)>,  // same order as the input positions, fractions
//! }
//! /// symbol = requested (case-insensitive) or, when None, the largest position by value.
//! /// Empty `positions` => None. Total value 0 => all weights 0.0 (never NaN).
//! pub fn summarize(positions: &[PositionValue], symbol: Option<&str>) -> Option<ConcentrationSummary>;
//!
//! pub struct LookThrough { pub value: f64, pub weight: f64, pub via_etfs_value: f64 }
//! /// held_etfs: (market value held, fraction of fund net assets in the symbol; may be negative).
//! pub fn look_through(direct_value: f64, total_value: f64, held_etfs: &[(f64, f64)]) -> LookThrough;
//!
//! /// (exposure + buy*pct) / (total + buy); 0.0 when the denominator is 0.
//! pub fn weight_after_buy(exposure_value: f64, total_value: f64, buy: f64, pct: f64) -> f64;
//! ```

use std::collections::HashMap;

use approx::assert_relative_eq;
use ghost_alpha::portfolio::lookthrough::*;
use ghost_alpha::storage::db::TaxLotRow;

fn lot(id: &str, ticker: &str, shares: f64, cost: f64) -> TaxLotRow {
    TaxLotRow {
        lot_id: id.to_string(),
        ticker: ticker.to_string(),
        shares,
        cost_basis_per_share: cost,
        acquisition_date: "2022-01-03".to_string(),
        acquisition_type: "BUY".to_string(),
    }
}

fn closes(pairs: &[(&str, f64)]) -> HashMap<String, f64> {
    pairs.iter().map(|(t, c)| (t.to_string(), *c)).collect()
}

fn pv(ticker: &str, value: f64) -> PositionValue {
    PositionValue {
        ticker: ticker.to_string(),
        shares: 1.0,
        price: value,
        value,
        priced_at_cost: false,
    }
}

fn find<'a>(v: &'a [PositionValue], t: &str) -> &'a PositionValue {
    v.iter()
        .find(|p| p.ticker == t)
        .unwrap_or_else(|| panic!("no {t} among {} positions", v.len()))
}

// ───────────────────────── value_positions ─────────────────────────

#[test]
fn test_value_positions_when_close_available_should_value_at_close() {
    let lots = [lot("a", "XYZ", 100.0, 80.0)];
    let out = value_positions(&lots, &closes(&[("XYZ", 200.0)]));
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].ticker, "XYZ");
    assert_relative_eq!(out[0].shares, 100.0);
    assert_relative_eq!(out[0].price, 200.0);
    assert_relative_eq!(out[0].value, 20_000.0);
    assert!(!out[0].priced_at_cost);
}

#[test]
fn test_value_positions_when_no_close_should_fall_back_to_cost_and_flag_it() {
    let lots = [lot("a", "ABC", 100.0, 50.0)];
    let out = value_positions(&lots, &HashMap::new());
    assert_relative_eq!(out[0].price, 50.0);
    assert_relative_eq!(out[0].value, 5_000.0);
    assert!(out[0].priced_at_cost);
}

#[test]
fn test_value_positions_when_several_lots_should_aggregate_by_ticker() {
    let lots = [lot("a", "XYZ", 60.0, 100.0), lot("b", "XYZ", 40.0, 120.0)];
    let out = value_positions(&lots, &closes(&[("XYZ", 200.0)]));
    assert_eq!(out.len(), 1);
    assert_relative_eq!(out[0].shares, 100.0);
    assert_relative_eq!(out[0].value, 20_000.0);
}

#[test]
fn test_value_positions_when_several_lots_at_cost_should_use_weighted_average_cost() {
    // (60*100 + 40*120) / 100 = 108
    let lots = [lot("a", "XYZ", 60.0, 100.0), lot("b", "XYZ", 40.0, 120.0)];
    let out = value_positions(&lots, &HashMap::new());
    assert_relative_eq!(out[0].price, 108.0);
    assert_relative_eq!(out[0].value, 10_800.0);
    assert!(out[0].priced_at_cost);
}

#[test]
fn test_value_positions_when_mixed_should_flag_only_the_unpriced_ticker() {
    let lots = [lot("a", "XYZ", 100.0, 80.0), lot("b", "ABC", 100.0, 50.0)];
    let out = value_positions(&lots, &closes(&[("XYZ", 200.0)]));
    assert!(!find(&out, "XYZ").priced_at_cost);
    assert!(find(&out, "ABC").priced_at_cost);
}

#[test]
fn test_value_positions_when_several_tickers_should_sort_by_value_desc_then_ticker() {
    let lots = [
        lot("a", "SMALL", 1.0, 10.0),
        lot("b", "BIG", 10.0, 100.0),
        lot("c", "MID2", 5.0, 20.0),
        lot("d", "MID1", 5.0, 20.0),
    ];
    let out = value_positions(&lots, &HashMap::new());
    let order: Vec<&str> = out.iter().map(|p| p.ticker.as_str()).collect();
    assert_eq!(order, ["BIG", "MID1", "MID2", "SMALL"]);
}

#[test]
fn test_value_positions_when_no_lots_should_be_empty() {
    assert!(value_positions(&[], &closes(&[("XYZ", 1.0)])).is_empty());
}

#[test]
fn test_value_positions_when_lot_has_zero_shares_should_ignore_it() {
    let lots = [lot("a", "XYZ", 0.0, 100.0), lot("b", "ABC", 10.0, 10.0)];
    let out = value_positions(&lots, &HashMap::new());
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].ticker, "ABC");
}

// ───────────────────────── summarize ─────────────────────────

#[test]
fn test_summarize_when_no_symbol_should_default_to_largest_position() {
    let ps = [pv("XYZ", 20_000.0), pv("VTI", 10_000.0), pv("ABC", 5_000.0)];
    let s = summarize(&ps, None).unwrap();
    assert_eq!(s.symbol, "XYZ");
    assert_relative_eq!(s.total_value, 35_000.0);
    assert_relative_eq!(s.direct_value, 20_000.0);
    assert_relative_eq!(s.direct_weight, 20_000.0 / 35_000.0, epsilon = 1e-12);
    assert_eq!(s.top1.0, "XYZ");
    assert_relative_eq!(s.top1.1, 20_000.0 / 35_000.0, epsilon = 1e-12);
}

#[test]
fn test_summarize_when_symbol_given_should_override_case_insensitively() {
    let ps = [pv("XYZ", 20_000.0), pv("VTI", 10_000.0), pv("ABC", 5_000.0)];
    let s = summarize(&ps, Some("vti")).unwrap();
    assert_eq!(s.symbol, "VTI");
    assert_relative_eq!(s.direct_value, 10_000.0);
    assert_relative_eq!(s.direct_weight, 10_000.0 / 35_000.0, epsilon = 1e-12);
    // top1 is still the portfolio's largest position, not the requested symbol.
    assert_eq!(s.top1.0, "XYZ");
}

#[test]
fn test_summarize_when_symbol_not_held_should_have_zero_direct_exposure() {
    let ps = [pv("XYZ", 20_000.0), pv("VTI", 10_000.0)];
    let s = summarize(&ps, Some("nope")).unwrap();
    assert_eq!(s.symbol, "NOPE");
    assert_relative_eq!(s.direct_value, 0.0);
    assert_relative_eq!(s.direct_weight, 0.0);
}

#[test]
fn test_summarize_when_equal_weights_should_have_effective_n_equal_to_count() {
    let ps = [
        pv("A", 100.0),
        pv("B", 100.0),
        pv("C", 100.0),
        pv("D", 100.0),
    ];
    let s = summarize(&ps, None).unwrap();
    assert_relative_eq!(s.effective_n, 4.0, epsilon = 1e-9);
}

#[test]
fn test_summarize_when_single_position_should_have_effective_n_one() {
    let s = summarize(&[pv("A", 42.0)], None).unwrap();
    assert_relative_eq!(s.effective_n, 1.0, epsilon = 1e-12);
    assert_relative_eq!(s.direct_weight, 1.0, epsilon = 1e-12);
}

#[test]
fn test_summarize_when_unequal_weights_should_use_inverse_herfindahl() {
    // w = 4/7, 2/7, 1/7  =>  sum w^2 = 21/49  =>  N = 49/21
    let ps = [pv("XYZ", 20_000.0), pv("VTI", 10_000.0), pv("ABC", 5_000.0)];
    let s = summarize(&ps, None).unwrap();
    assert_relative_eq!(s.effective_n, 49.0 / 21.0, epsilon = 1e-9);
}

#[test]
fn test_summarize_when_called_should_return_weights_in_input_order_summing_to_one() {
    let ps = [pv("XYZ", 20_000.0), pv("VTI", 10_000.0), pv("ABC", 5_000.0)];
    let s = summarize(&ps, None).unwrap();
    let tickers: Vec<&str> = s.weights.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(tickers, ["XYZ", "VTI", "ABC"]);
    let sum: f64 = s.weights.iter().map(|(_, w)| w).sum();
    assert_relative_eq!(sum, 1.0, epsilon = 1e-12);
}

#[test]
fn test_summarize_when_empty_portfolio_should_be_none() {
    assert!(summarize(&[], None).is_none());
    assert!(summarize(&[], Some("XYZ")).is_none());
}

#[test]
fn test_summarize_when_total_value_zero_should_give_zero_weights_not_nan() {
    let ps = [pv("A", 0.0), pv("B", 0.0)];
    let s = summarize(&ps, None).unwrap();
    assert_relative_eq!(s.total_value, 0.0);
    assert_relative_eq!(s.direct_weight, 0.0);
    assert_relative_eq!(s.top1.1, 0.0);
    assert_relative_eq!(s.effective_n, 0.0);
    assert!(s.weights.iter().all(|(_, w)| *w == 0.0));
}

// ───────────────────────── look_through ─────────────────────────

#[test]
fn test_look_through_when_etf_holds_symbol_should_add_value_times_pct() {
    // direct 340k of 500k; $40k of an ETF with 3.19% in the symbol adds 1,276.
    let lt = look_through(340_000.0, 500_000.0, &[(40_000.0, 0.0319)]);
    assert_relative_eq!(lt.via_etfs_value, 1_276.0, epsilon = 1e-9);
    assert_relative_eq!(lt.value, 341_276.0, epsilon = 1e-9);
    assert_relative_eq!(lt.weight, 341_276.0 / 500_000.0, epsilon = 1e-12);
}

#[test]
fn test_look_through_when_no_etfs_should_equal_direct() {
    let lt = look_through(340_000.0, 500_000.0, &[]);
    assert_relative_eq!(lt.via_etfs_value, 0.0);
    assert_relative_eq!(lt.value, 340_000.0);
    assert_relative_eq!(lt.weight, 0.68, epsilon = 1e-12);
}

#[test]
fn test_look_through_when_several_etfs_should_sum_contributions() {
    let lt = look_through(20_000.0, 35_000.0, &[(10_000.0, 0.0319), (4_000.0, 0.05)]);
    assert_relative_eq!(lt.via_etfs_value, 319.0 + 200.0, epsilon = 1e-9);
    assert_relative_eq!(lt.value, 20_519.0, epsilon = 1e-9);
}

#[test]
fn test_look_through_when_net_short_pct_should_reduce_exposure() {
    let lt = look_through(20_000.0, 35_000.0, &[(10_000.0, -0.02)]);
    assert_relative_eq!(lt.via_etfs_value, -200.0, epsilon = 1e-9);
    assert_relative_eq!(lt.value, 19_800.0, epsilon = 1e-9);
}

#[test]
fn test_look_through_when_total_zero_should_have_zero_weight_not_nan() {
    let lt = look_through(0.0, 0.0, &[]);
    assert_relative_eq!(lt.weight, 0.0);
    assert!(lt.weight.is_finite());
}

// ───────────────────────── weight_after_buy ─────────────────────────

#[test]
fn test_weight_after_buy_when_zero_pct_etf_should_dilute_per_plan_example() {
    // 68% direct of $500k, buy $50k of an ETF with 0% in the symbol => 340k / 550k.
    let w = weight_after_buy(340_000.0, 500_000.0, 50_000.0, 0.0);
    assert_relative_eq!(w, 340_000.0 / 550_000.0, epsilon = 1e-12);
    assert_eq!(format!("{:.1}", w * 100.0), "61.8");
}

#[test]
fn test_weight_after_buy_when_etf_holds_symbol_should_add_buy_times_pct() {
    // (340,000 + 50,000 * 0.0319) / 550,000
    let w = weight_after_buy(340_000.0, 500_000.0, 50_000.0, 0.0319);
    assert_relative_eq!(w, (340_000.0 + 1_595.0) / 550_000.0, epsilon = 1e-12);
}

#[test]
fn test_weight_after_buy_when_zero_buy_should_equal_current_weight() {
    let w = weight_after_buy(340_000.0, 500_000.0, 0.0, 0.05);
    assert_relative_eq!(w, 0.68, epsilon = 1e-12);
}

#[test]
fn test_weight_after_buy_when_negative_pct_should_lower_exposure() {
    let w = weight_after_buy(20_000.0, 35_000.0, 50_000.0, -0.01);
    assert_relative_eq!(w, (20_000.0 - 500.0) / 85_000.0, epsilon = 1e-12);
}

#[test]
fn test_weight_after_buy_when_denominator_zero_should_be_zero_not_nan() {
    let w = weight_after_buy(0.0, 0.0, 0.0, 0.03);
    assert_relative_eq!(w, 0.0);
    assert!(w.is_finite());
}

#[test]
fn test_weight_after_buy_when_empty_portfolio_should_be_buy_pct() {
    let w = weight_after_buy(0.0, 0.0, 10_000.0, 0.04);
    assert_relative_eq!(w, 0.04, epsilon = 1e-12);
}
