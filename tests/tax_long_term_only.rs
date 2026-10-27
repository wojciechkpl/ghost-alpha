//! RED regression for m2: Washington's capital-gains excise tax is long-term only.
//!
//! `tax::brackets::StateTaxRate` gains `pub long_term_only: bool` (true for WA, false for every other
//! state returned by `state_tax_rate`). `tax::engine::compute_lot_tax` respects it: a short-term lot
//! pays no state tax in such a state (and so uses none of the deduction). This changes the CLI
//! (`sell-plan`, etc.) deliberately.

use approx::assert_relative_eq;
use chrono::NaiveDate;
use ghost_alpha::portfolio::types::{AcquisitionType, TaxLot};
use ghost_alpha::tax::brackets::{state_tax_rate, FederalCapGainsBrackets};
use ghost_alpha::tax::engine::compute_lot_tax;
use uuid::Uuid;

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn lot(acquired: &str) -> TaxLot {
    TaxLot {
        id: Uuid::from_u128(1),
        ticker: "XYZ".into(),
        shares: 1000.0,
        cost_basis_per_share: 100.0,
        acquired_date: d(acquired),
        acquisition_type: AcquisitionType::Buy,
        remaining_shares: 1000.0,
    }
}

#[test]
fn test_state_tax_rate_should_flag_only_wa_as_long_term_only() {
    assert!(state_tax_rate("WA").unwrap().long_term_only);
    for s in ["CA", "NY", "TX", "FL", "NV", "IL", "NJ", "MA", "CO"] {
        assert!(!state_tax_rate(s).unwrap().long_term_only, "{s}");
    }
}

#[test]
fn test_compute_lot_tax_when_wa_short_term_lot_should_owe_no_state_tax() {
    let wa = state_tax_rate("WA").unwrap();
    let price = 100.0 + 1.5 * wa.annual_deduction / 1000.0; // gain 1.5 x deduction
    let r = compute_lot_tax(
        &lot("2027-03-01"), // 122 days at the sale: short term
        1000.0,
        price,
        d("2027-07-01"),
        100_000.0,
        &FederalCapGainsBrackets::single_2024(),
        &wa,
        0.0,
    );
    assert!(!r.is_long_term);
    assert_relative_eq!(r.state_tax, 0.0, epsilon = 1e-9);
    assert!(r.federal_tax > 0.0);
}

#[test]
fn test_compute_lot_tax_when_wa_long_term_lot_should_still_apply_the_deduction() {
    let wa = state_tax_rate("WA").unwrap();
    let ded = wa.annual_deduction;
    let price = 100.0 + 2.0 * ded / 1000.0; // gain 2 x deduction
    let r = compute_lot_tax(
        &lot("2020-01-01"),
        1000.0,
        price,
        d("2027-07-01"),
        100_000.0,
        &FederalCapGainsBrackets::single_2024(),
        &wa,
        0.0,
    );
    assert!(r.is_long_term);
    assert_relative_eq!(r.state_tax, wa.rate * ded, max_relative = 1e-9);
}

#[test]
fn test_compute_lot_tax_when_state_is_not_long_term_only_should_tax_short_term_gain() {
    let ca = state_tax_rate("CA").unwrap();
    let r = compute_lot_tax(
        &lot("2027-03-01"),
        1000.0,
        150.0,
        d("2027-07-01"),
        100_000.0,
        &FederalCapGainsBrackets::single_2024(),
        &ca,
        0.0,
    );
    assert_relative_eq!(r.state_tax, ca.rate * 50_000.0, epsilon = 1e-6);
}

// ── N1: the multi-year projection (sell-plan / sell-compare path) ──

#[test]
fn test_project_multi_year_when_wa_short_term_lot_sold_first_should_not_use_up_the_deduction() {
    use ghost_alpha::tax::projection::{project_multi_year_with_strategy, LotSelectionStrategy};

    let wa = state_tax_rate("WA").unwrap();
    // ST lot (highest basis, so HIFO sells it first): gain 100,000. LT lot: gain 200,000, which is
    // below the WA deduction on its own, so the year's WA tax must be 0.
    let mut st = lot("2027-03-01"); // 122 days at the notional 1 July sale: short term
    st.cost_basis_per_share = 200.0;
    st.id = Uuid::from_u128(2);
    let lt = lot("2020-01-01"); // basis 100
    let price = 300.0;
    assert!(1000.0 * (price - 100.0) < wa.annual_deduction);
    let years = project_multi_year_with_strategy(
        &[st, lt],
        &[(2027, 2000.0)],
        0.0,
        price,
        100_000.0,
        &FederalCapGainsBrackets::single_2024(),
        &wa,
        &LotSelectionStrategy::Hifo,
    );
    assert_eq!(years.len(), 1);
    assert_relative_eq!(years[0].shares_sold, 2000.0, epsilon = 1e-9);
    assert!(years[0].federal_tax > 0.0);
    assert_relative_eq!(years[0].state_tax, 0.0, epsilon = 1e-9);
}
