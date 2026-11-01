//! RED-phase contract for `UnwindSchedule.years` (Track B, tax by calendar year).
//! Synthetic data only (ticker XYZ). This file fails to compile until the field exists.
//!
//! Public API pinned here (additive; every existing `UnwindSchedule` field keeps its meaning):
//!
//! ```ignore
//! pub struct UnwindSchedule {
//!     pub periods: Vec<UnwindPeriod>,
//!     pub totals: UnwindTotals,
//!     pub years: Vec<UnwindYear>,        // NEW: ascending by calendar year, no empty years
//! }
//! #[derive(Debug, Clone)]
//! pub struct UnwindYear {
//!     pub year: i32,                     // calendar year of the sale dates
//!     pub shares: f64,                   // sum of the year's period shares
//!     pub proceeds: f64,                 // sum of the year's period proceeds
//!     pub short_term_gain: f64,          // sum of the year's period short_term_gain
//!     pub long_term_gain: f64,           // sum of the year's period long_term_gain
//!     pub tax: Option<YearTax>,          // None when input.tax is None; otherwise
//!                                        // year_tax(short_term_gain, long_term_gain, settings)
//! }
//! ```
//!
//! Invariants: periods are grouped by `date.year()`; the periods' `tax` values in a year sum to
//! that year's `tax.total` (1e-6); `years` is empty when nothing is sold.

use approx::assert_relative_eq;
use chrono::{Datelike, NaiveDate};
use ghost_alpha::portfolio::types::{AcquisitionType, TaxLot};
use ghost_alpha::portfolio::unwind::*;
use ghost_alpha::tax::brackets::{state_tax_rate, FederalCapGainsBrackets};
use uuid::Uuid;

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn lot(n: u128, shares: f64, basis: f64, acquired: &str) -> TaxLot {
    TaxLot {
        id: Uuid::from_u128(n),
        ticker: "XYZ".to_string(),
        shares,
        cost_basis_per_share: basis,
        acquired_date: d(acquired),
        acquisition_type: AcquisitionType::Buy,
        remaining_shares: shares,
    }
}

fn settings(state: &str, other_income: f64) -> TaxSettings {
    TaxSettings {
        state: state_tax_rate(state).unwrap(),
        brackets: FederalCapGainsBrackets::single_2024(),
        other_income,
    }
}

fn input(
    lots: Vec<TaxLot>,
    price: f64,
    years: u32,
    cadence: Cadence,
    start: &str,
    tax: Option<TaxSettings>,
) -> UnwindInput {
    UnwindInput {
        lots,
        price,
        other_value: 100_000.0,
        years,
        cadence,
        growth: 0.0,
        start: d(start),
        tax,
    }
}

/// 12,000 shares: half long term (basis 20), half bought in 2027 (basis 80, short term for a year).
fn mixed_lots() -> Vec<TaxLot> {
    vec![
        lot(1, 6000.0, 20.0, "2020-01-01"),
        lot(2, 6000.0, 80.0, "2027-01-10"),
    ]
}

/// 3 years of quarterly sales starting 2027-05-15: 12 periods over calendar years 2027..=2030.
fn mid_year_schedule(tax: Option<TaxSettings>) -> UnwindSchedule {
    build_schedule(&input(
        mixed_lots(),
        100.0,
        3,
        Cadence::Quarterly,
        "2027-05-15",
        tax,
    ))
}

fn periods_in(s: &UnwindSchedule, year: i32) -> Vec<&UnwindPeriod> {
    s.periods.iter().filter(|p| p.date.year() == year).collect()
}

#[test]
fn test_years_when_quarterly_schedule_starts_mid_year_should_span_four_calendar_years() {
    let s = mid_year_schedule(None);
    assert_eq!(s.periods.len(), 12);
    let ys: Vec<i32> = s.years.iter().map(|y| y.year).collect();
    assert_eq!(ys, vec![2027, 2028, 2029, 2030]);
}

#[test]
fn test_years_when_annual_schedule_starts_in_january_should_have_one_year_per_period() {
    let s = build_schedule(&input(
        mixed_lots(),
        100.0,
        3,
        Cadence::Annual,
        "2027-01-01",
        None,
    ));
    let ys: Vec<i32> = s.years.iter().map(|y| y.year).collect();
    assert_eq!(ys, vec![2027, 2028, 2029]);
}

#[test]
fn test_years_when_nothing_to_sell_should_be_empty() {
    let s = build_schedule(&input(
        vec![],
        100.0,
        3,
        Cadence::Quarterly,
        "2027-05-15",
        None,
    ));
    assert!(s.periods.is_empty());
    assert!(s.years.is_empty());
}

#[test]
fn test_years_shares_and_proceeds_should_equal_the_periods_in_that_year() {
    let s = mid_year_schedule(None);
    assert_eq!(s.years.len(), 4);
    for y in &s.years {
        let ps = periods_in(&s, y.year);
        assert!(!ps.is_empty(), "year {} has no periods", y.year);
        assert_relative_eq!(
            y.shares,
            ps.iter().map(|p| p.shares).sum::<f64>(),
            epsilon = 1e-6
        );
        assert_relative_eq!(
            y.proceeds,
            ps.iter().map(|p| p.proceeds).sum::<f64>(),
            epsilon = 1e-6
        );
    }
}

#[test]
fn test_years_shares_and_proceeds_should_sum_to_the_schedule_totals() {
    let s = mid_year_schedule(None);
    assert_relative_eq!(
        s.years.iter().map(|y| y.shares).sum::<f64>(),
        s.totals.shares,
        epsilon = 1e-6
    );
    assert_relative_eq!(
        s.years.iter().map(|y| y.proceeds).sum::<f64>(),
        s.totals.proceeds,
        epsilon = 1e-6
    );
    assert_relative_eq!(s.totals.shares, 12_000.0, epsilon = 1e-6);
}

#[test]
fn test_years_gains_should_equal_the_periods_short_and_long_term_gains() {
    let s = mid_year_schedule(None);
    for y in &s.years {
        let ps = periods_in(&s, y.year);
        assert_relative_eq!(
            y.short_term_gain,
            ps.iter().map(|p| p.short_term_gain).sum::<f64>(),
            epsilon = 1e-6
        );
        assert_relative_eq!(
            y.long_term_gain,
            ps.iter().map(|p| p.long_term_gain).sum::<f64>(),
            epsilon = 1e-6
        );
    }
    // The fixture really does mix terms (otherwise this test proves little).
    assert!(s.years.iter().any(|y| y.short_term_gain > 0.0));
    assert!(s.years.iter().any(|y| y.long_term_gain > 0.0));
}

#[test]
fn test_years_without_tax_settings_should_have_no_tax() {
    let s = mid_year_schedule(None);
    assert!(!s.years.is_empty());
    assert!(s.years.iter().all(|y| y.tax.is_none()));
}

#[test]
fn test_years_with_tax_settings_should_have_tax_for_every_year() {
    let s = mid_year_schedule(Some(settings("CA", 200_000.0)));
    assert_eq!(s.years.len(), 4);
    assert!(s.years.iter().all(|y| y.tax.is_some()));
}

#[test]
fn test_years_period_taxes_in_a_year_should_sum_to_the_year_tax_total() {
    let s = mid_year_schedule(Some(settings("CA", 200_000.0)));
    assert_eq!(s.years.len(), 4);
    for y in &s.years {
        let t = y.tax.expect("tax");
        let sum: f64 = periods_in(&s, y.year)
            .iter()
            .map(|p| p.tax.expect("period tax"))
            .sum();
        assert_relative_eq!(sum, t.total, epsilon = 1e-6);
        assert!(t.total > 0.0, "year {} should owe tax", y.year);
    }
}

#[test]
fn test_years_tax_totals_should_sum_to_the_schedule_total_tax() {
    let s = mid_year_schedule(Some(settings("CA", 200_000.0)));
    let sum: f64 = s.years.iter().map(|y| y.tax.unwrap().total).sum();
    assert_relative_eq!(sum, s.totals.tax.unwrap(), epsilon = 1e-6);
}

#[test]
fn test_years_tax_components_should_add_up_to_the_total() {
    // High other income so that federal, state and NIIT are all non-zero somewhere.
    let s = mid_year_schedule(Some(settings("CA", 250_000.0)));
    for y in &s.years {
        let t = y.tax.unwrap();
        assert_relative_eq!(t.federal + t.state + t.niit, t.total, epsilon = 1e-6);
    }
    assert!(s.years.iter().any(|y| y.tax.unwrap().federal > 0.0));
    assert!(s.years.iter().any(|y| y.tax.unwrap().state > 0.0));
    assert!(s.years.iter().any(|y| y.tax.unwrap().niit > 0.0));
}

#[test]
fn test_years_tax_should_equal_year_tax_of_the_years_gains() {
    let st = settings("CA", 200_000.0);
    let s = mid_year_schedule(Some(st.clone()));
    assert_eq!(s.years.len(), 4);
    for y in &s.years {
        let expect = year_tax(y.short_term_gain, y.long_term_gain, &st);
        let got = y.tax.unwrap();
        assert_relative_eq!(got.federal, expect.federal, epsilon = 1e-6);
        assert_relative_eq!(got.state, expect.state, epsilon = 1e-6);
        assert_relative_eq!(got.niit, expect.niit, epsilon = 1e-6);
        assert_relative_eq!(got.total, expect.total, epsilon = 1e-6);
    }
}

/// Washington: the long-term exemption applies once per calendar year, not once per sale and not
/// once overall. Eight quarterly sales over 2027 and 2028 each realise 0.4 * deduction, so each
/// year realises 1.6 * deduction and owes state tax on the 0.6 * deduction above the exemption.
#[test]
fn test_years_washington_exemption_should_apply_once_per_calendar_year() {
    let wa = state_tax_rate("WA").unwrap();
    let ded = wa.annual_deduction;
    assert!(ded > 0.0);
    let basis = 100.0;
    let price = basis + 0.4 * ded / 1000.0;
    let s = build_schedule(&input(
        vec![lot(1, 8000.0, basis, "2020-01-01")],
        price,
        2,
        Cadence::Quarterly,
        "2027-01-01",
        Some(settings("WA", 100_000.0)),
    ));
    assert_eq!(s.years.len(), 2);
    for y in &s.years {
        assert_relative_eq!(y.long_term_gain, 1.6 * ded, max_relative = 1e-6);
        let t = y.tax.unwrap();
        assert_relative_eq!(t.state, 0.6 * ded * wa.rate, max_relative = 1e-6);
    }
}

#[test]
fn test_years_texas_should_have_no_state_component() {
    let s = mid_year_schedule(Some(settings("TX", 200_000.0)));
    assert_eq!(s.years.len(), 4);
    assert!(s.years.iter().all(|y| y.tax.unwrap().state == 0.0));
}
