//! RED-phase contract for `ghost_alpha::portfolio::unwind` (new module): the pure maths of the
//! concentration-hub unwind schedule and the seeded Monte Carlo projection. Synthetic data only.
//!
//! Public API pinned here:
//!
//! ```ignore
//! #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//! pub enum Cadence { Quarterly, Annual }
//!
//! pub struct TaxSettings {            // Debug + Clone
//!     pub state: tax::brackets::StateTaxRate,
//!     pub brackets: tax::brackets::FederalCapGainsBrackets,
//!     pub other_income: f64,
//! }
//!
//! pub struct UnwindInput {            // Debug + Clone
//!     pub lots: Vec<TaxLot>,          // lots of the concentrated symbol only; shares = remaining_shares
//!     pub price: f64,                 // today's price
//!     pub other_value: f64,           // value of everything else, held flat in the schedule
//!     pub years: u32,
//!     pub cadence: Cadence,
//!     pub growth: f64,                // fraction per year (0.10 = +10%/yr)
//!     pub start: NaiveDate,           // date of the first sale
//!     pub tax: Option<TaxSettings>,
//! }
//!
//! pub struct UnwindPeriod {           // Debug + Clone
//!     pub label: String,              // non-empty, unique ("Q1".."Q12" / "Y1".. at implementer's choice)
//!     pub date: NaiveDate,
//!     pub shares: f64, pub proceeds: f64, pub cost_basis: f64, pub gain: f64,
//!     pub long_term_gain: f64, pub short_term_gain: f64,   // sum == gain
//!     pub tax: Option<f64>,           // total tax (federal + state + NIIT); None when input.tax is None
//!     pub net: f64,                   // proceeds - tax (== proceeds when tax is None)
//!     pub weight_after: f64,          // fraction, see below
//! }
//! pub struct UnwindTotals { pub shares: f64, pub proceeds: f64, pub cost_basis: f64, pub gain: f64,
//!                           pub tax: Option<f64>, pub net: f64 }   // Debug + Clone, plain sums
//! pub struct UnwindSchedule { pub periods: Vec<UnwindPeriod>, pub totals: UnwindTotals }
//!
//! pub fn build_schedule(input: &UnwindInput) -> UnwindSchedule;
//! ```
//!
//! Schedule rules:
//! * periods: Quarterly = 4*years, Annual = years. Period k (0-based) sells on `start + k*step` months
//!   (step 3 or 12), so the first sale is on `start`.
//! * shares: total / n each; the last period takes the remainder so everything is sold.
//! * price_k = price * (1 + growth)^(k*step_months/12).
//! * lots are consumed by `tax::hifo::hifo_select` against the lots' remaining shares, carrying the
//!   remainders from period to period; cost_basis is the HIFO cost of the shares sold.
//! * long/short is judged at the period's sale date (`TaxLot::is_long_term(date)`).
//! * tax (when Some): sum over the lots touched of `tax::engine::compute_lot_tax(lot, shares, price_k,
//!   date, other_income, &brackets, &state, state_gains_ytd)`.total_tax, where `state_gains_ytd` is the
//!   total `gain` realised by earlier periods of the same calendar year (0.0 at the first sale of a year).
//! * weight_after_k = remaining_shares_k * price_k / (remaining_shares_k * price_k + other_value + sum of
//!   net_j for j <= k) (net proceeds are reinvested in the rest, held flat). 0.0 after the last sale.
//!
//! ```ignore
//! pub const DEFAULT_STOCK_MU: f64 = 0.08;   pub const DEFAULT_STOCK_SIGMA: f64 = 0.30;
//! pub const DEFAULT_REST_MU: f64 = 0.06;    pub const DEFAULT_REST_SIGMA: f64 = 0.15;
//! pub const DEFAULT_RHO: f64 = 0.6;         pub const DEFAULT_PATHS: usize = 2000;
//! pub const PROJECTION_SEED: u64 = <any>;
//!
//! pub struct ProjectionInput {        // Debug + Clone
//!     pub stock_value: f64, pub other_value: f64, pub years: u32,
//!     pub stock_mu: f64, pub stock_sigma: f64, pub rest_mu: f64, pub rest_sigma: f64, pub rho: f64,
//!     pub paths: usize,
//!     /// len == years. Fraction of the ORIGINAL shares sold at the START of year y (before that year's
//!     /// returns). Sums to 1.0 for a full unwind.
//!     pub sell_fractions: Vec<f64>,
//!     /// Effective tax rate on sale proceeds; after-tax proceeds (1 - rate) move into the rest.
//!     pub tax_rate: f64,
//! }
//! pub struct Projection {
//!     pub years: u32,
//!     pub hold: Vec<FanChartYear>,     // len years+1; index 0 is "Start" (all percentiles equal)
//!     pub unwind: Vec<FanChartYear>,   // same shape
//!     pub weight_p50_hold: Vec<f64>,   // median stock/(stock+rest), len years+1
//!     pub weight_p50_unwind: Vec<f64>,
//! }
//! pub fn project(input: &ProjectionInput, seed: u64) -> Projection;
//! ```
//!
//! Model: two correlated lognormal assets with yearly steps. Each asset's arithmetic mean/std go
//! through `projection::monte_carlo::lognormal_params`; with sigma = 0 this reduces to compounding
//! at (1 + mu) per year exactly. `FanChartYear.year` is 0..=years.

use approx::assert_relative_eq;
use chrono::{Datelike, Months, NaiveDate};
use ghost_alpha::portfolio::types::{AcquisitionType, TaxLot};
use ghost_alpha::portfolio::unwind::*;
use ghost_alpha::tax::brackets::{state_tax_rate, FederalCapGainsBrackets};
use ghost_alpha::tax::engine::compute_lot_tax;
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

fn input(lots: Vec<TaxLot>, price: f64, years: u32, cadence: Cadence) -> UnwindInput {
    UnwindInput {
        lots,
        price,
        other_value: 100_000.0,
        years,
        cadence,
        growth: 0.0,
        start: d("2027-01-01"),
        tax: None,
    }
}

fn one_lot(shares: f64, basis: f64) -> Vec<TaxLot> {
    vec![lot(1, shares, basis, "2020-01-01")]
}

// ───────────────────────── schedule: shape ─────────────────────────

#[test]
fn test_schedule_when_cadence_varies_should_have_expected_period_counts() {
    for (years, cadence, n) in [
        (1, Cadence::Quarterly, 4),
        (3, Cadence::Quarterly, 12),
        (1, Cadence::Annual, 1),
        (3, Cadence::Annual, 3),
        (10, Cadence::Annual, 10),
    ] {
        let s = build_schedule(&input(one_lot(1000.0, 10.0), 50.0, years, cadence));
        assert_eq!(s.periods.len(), n, "{years} {cadence:?}");
    }
}

#[test]
fn test_schedule_when_built_should_use_start_then_fixed_month_steps_for_dates() {
    let q = build_schedule(&input(one_lot(1000.0, 10.0), 50.0, 2, Cadence::Quarterly));
    for (k, p) in q.periods.iter().enumerate() {
        assert_eq!(p.date, d("2027-01-01") + Months::new(3 * k as u32));
    }
    let a = build_schedule(&input(one_lot(1000.0, 10.0), 50.0, 3, Cadence::Annual));
    for (k, p) in a.periods.iter().enumerate() {
        assert_eq!(p.date, d("2027-01-01") + Months::new(12 * k as u32));
    }
}

#[test]
fn test_schedule_labels_should_be_non_empty_and_unique() {
    let s = build_schedule(&input(one_lot(1000.0, 10.0), 50.0, 3, Cadence::Quarterly));
    let mut labels: Vec<&str> = s.periods.iter().map(|p| p.label.as_str()).collect();
    assert!(labels.iter().all(|l| !l.trim().is_empty()));
    labels.sort();
    labels.dedup();
    assert_eq!(labels.len(), 12);
}

#[test]
fn test_schedule_when_shares_divide_evenly_should_split_equally() {
    let s = build_schedule(&input(one_lot(1000.0, 10.0), 50.0, 1, Cadence::Quarterly));
    for p in &s.periods {
        assert_relative_eq!(p.shares, 250.0, epsilon = 1e-9);
        assert_relative_eq!(p.proceeds, 250.0 * 50.0, epsilon = 1e-9);
    }
}

#[test]
fn test_schedule_when_shares_do_not_divide_should_sell_every_share_with_remainder_last() {
    let s = build_schedule(&input(one_lot(100.0, 10.0), 50.0, 3, Cadence::Annual));
    let total: f64 = s.periods.iter().map(|p| p.shares).sum();
    assert_relative_eq!(total, 100.0, epsilon = 1e-9);
    assert_relative_eq!(s.totals.shares, 100.0, epsilon = 1e-9);
    assert_relative_eq!(s.periods[0].shares, 100.0 / 3.0, epsilon = 1e-9);
    assert_relative_eq!(s.periods[1].shares, 100.0 / 3.0, epsilon = 1e-9);
    assert_relative_eq!(
        s.periods[2].shares,
        100.0 - 2.0 * (100.0 / 3.0),
        epsilon = 1e-9
    );
    assert!(s.periods.iter().all(|p| p.shares > 0.0));
}

#[test]
fn test_schedule_when_several_lots_should_sell_all_shares_across_them() {
    let lots = vec![
        lot(1, 400.0, 10.0, "2020-01-01"),
        lot(2, 600.0, 30.0, "2021-01-01"),
    ];
    let s = build_schedule(&input(lots, 50.0, 2, Cadence::Quarterly));
    assert_relative_eq!(s.totals.shares, 1000.0, epsilon = 1e-9);
    assert_relative_eq!(
        s.totals.cost_basis,
        400.0 * 10.0 + 600.0 * 30.0,
        epsilon = 1e-6
    );
}

// ───────────────────────── schedule: HIFO and gains ─────────────────────────

#[test]
fn test_schedule_should_sell_highest_cost_lot_first() {
    let lots = vec![
        lot(1, 1000.0, 50.0, "2020-01-01"),
        lot(2, 1000.0, 150.0, "2021-01-01"),
    ];
    let s = build_schedule(&input(lots, 200.0, 2, Cadence::Annual));
    assert_relative_eq!(s.periods[0].cost_basis, 150_000.0, epsilon = 1e-6);
    assert_relative_eq!(s.periods[1].cost_basis, 50_000.0, epsilon = 1e-6);
}

#[test]
fn test_schedule_hifo_with_three_lots_should_walk_lots_in_descending_cost_order() {
    let lots = vec![
        lot(1, 1000.0, 10.0, "2020-01-01"),
        lot(2, 1000.0, 30.0, "2020-02-01"),
        lot(3, 1000.0, 20.0, "2020-03-01"),
    ];
    let s = build_schedule(&input(lots, 100.0, 3, Cadence::Annual));
    let cb: Vec<f64> = s.periods.iter().map(|p| p.cost_basis).collect();
    assert_relative_eq!(cb[0], 30_000.0, epsilon = 1e-6);
    assert_relative_eq!(cb[1], 20_000.0, epsilon = 1e-6);
    assert_relative_eq!(cb[2], 10_000.0, epsilon = 1e-6);
}

#[test]
fn test_schedule_gain_fields_should_be_consistent() {
    let s = build_schedule(&input(one_lot(1000.0, 10.0), 50.0, 2, Cadence::Quarterly));
    for p in &s.periods {
        assert_relative_eq!(p.gain, p.proceeds - p.cost_basis, epsilon = 1e-6);
        assert_relative_eq!(p.long_term_gain + p.short_term_gain, p.gain, epsilon = 1e-6);
    }
}

#[test]
fn test_schedule_totals_should_equal_sum_of_periods() {
    let mut inp = input(one_lot(1000.0, 10.0), 50.0, 2, Cadence::Quarterly);
    inp.tax = Some(settings("CA", 150_000.0));
    let s = build_schedule(&inp);
    let sum = |f: fn(&UnwindPeriod) -> f64| s.periods.iter().map(f).sum::<f64>();
    assert_relative_eq!(s.totals.proceeds, sum(|p| p.proceeds), epsilon = 1e-6);
    assert_relative_eq!(s.totals.cost_basis, sum(|p| p.cost_basis), epsilon = 1e-6);
    assert_relative_eq!(s.totals.gain, sum(|p| p.gain), epsilon = 1e-6);
    assert_relative_eq!(s.totals.net, sum(|p| p.net), epsilon = 1e-6);
    assert_relative_eq!(
        s.totals.tax.unwrap(),
        s.periods.iter().map(|p| p.tax.unwrap()).sum::<f64>(),
        epsilon = 1e-6
    );
}

// ───────────────────────── schedule: growth ─────────────────────────

#[test]
fn test_schedule_when_annual_growth_should_compound_price_per_year() {
    let mut inp = input(one_lot(1000.0, 10.0), 100.0, 3, Cadence::Annual);
    inp.growth = 0.10;
    let s = build_schedule(&inp);
    for (k, p) in s.periods.iter().enumerate() {
        let price = 100.0 * 1.10_f64.powi(k as i32);
        assert_relative_eq!(p.proceeds, p.shares * price, max_relative = 1e-9);
    }
}

#[test]
fn test_schedule_when_quarterly_growth_should_compound_fractionally() {
    let mut inp = input(one_lot(1000.0, 10.0), 100.0, 1, Cadence::Quarterly);
    inp.growth = 0.10;
    let s = build_schedule(&inp);
    for (k, p) in s.periods.iter().enumerate() {
        let price = 100.0 * 1.10_f64.powf(k as f64 * 0.25);
        assert_relative_eq!(p.proceeds, p.shares * price, max_relative = 1e-9);
    }
}

#[test]
fn test_schedule_when_negative_growth_should_lower_later_proceeds() {
    let mut inp = input(one_lot(1000.0, 10.0), 100.0, 2, Cadence::Annual);
    inp.growth = -0.20;
    let s = build_schedule(&inp);
    assert!(s.periods[1].proceeds < s.periods[0].proceeds);
}

// ───────────────────────── schedule: weight after ─────────────────────────

#[test]
fn test_schedule_weight_after_should_follow_reinvestment_formula_without_tax() {
    // 1000 sh @ 10, other 10,000, 4 quarters of 250: after Q1 stock 7,500 vs rest 12,500.
    let mut inp = input(one_lot(1000.0, 1.0), 10.0, 1, Cadence::Quarterly);
    inp.other_value = 10_000.0;
    let s = build_schedule(&inp);
    assert_relative_eq!(
        s.periods[0].weight_after,
        7_500.0 / 20_000.0,
        epsilon = 1e-9
    );
    assert_relative_eq!(
        s.periods[1].weight_after,
        5_000.0 / 20_000.0,
        epsilon = 1e-9
    );
    assert_relative_eq!(
        s.periods[2].weight_after,
        2_500.0 / 20_000.0,
        epsilon = 1e-9
    );
    assert_relative_eq!(s.periods[3].weight_after, 0.0, epsilon = 1e-12);
}

#[test]
fn test_schedule_weight_after_should_never_increase_and_end_at_zero() {
    let mut inp = input(one_lot(1000.0, 10.0), 50.0, 3, Cadence::Quarterly);
    inp.tax = Some(settings("CA", 200_000.0));
    let s = build_schedule(&inp);
    let w: Vec<f64> = s.periods.iter().map(|p| p.weight_after).collect();
    assert!(w.windows(2).all(|x| x[1] <= x[0] + 1e-12), "{w:?}");
    assert_relative_eq!(*w.last().unwrap(), 0.0, epsilon = 1e-12);
    assert!(w.iter().all(|x| (0.0..=1.0).contains(x)));
}

#[test]
fn test_schedule_when_tax_reduces_net_should_leave_more_weight_in_the_stock() {
    let base = input(one_lot(1000.0, 10.0), 100.0, 2, Cadence::Annual);
    let mut taxed = base.clone();
    taxed.tax = Some(settings("CA", 200_000.0));
    let (a, b) = (build_schedule(&base), build_schedule(&taxed));
    assert!(b.periods[0].weight_after > a.periods[0].weight_after);
}

// ───────────────────────── schedule: tax ─────────────────────────

#[test]
fn test_schedule_when_no_tax_settings_should_have_no_tax_and_net_equals_proceeds() {
    let s = build_schedule(&input(one_lot(1000.0, 10.0), 50.0, 2, Cadence::Annual));
    assert!(s.totals.tax.is_none());
    for p in &s.periods {
        assert!(p.tax.is_none());
        assert_relative_eq!(p.net, p.proceeds, epsilon = 1e-9);
    }
}

#[test]
fn test_schedule_tax_for_single_lot_should_match_compute_lot_tax() {
    let l = lot(1, 1000.0, 10.0, "2020-01-01");
    let t = settings("CA", 120_000.0);
    let expected = compute_lot_tax(
        &l,
        1000.0,
        50.0,
        d("2027-01-01"),
        t.other_income,
        &t.brackets,
        &t.state,
        0.0,
    );
    let mut inp = input(vec![l], 50.0, 1, Cadence::Annual);
    inp.tax = Some(t);
    let p = &build_schedule(&inp).periods[0];
    assert_relative_eq!(p.tax.unwrap(), expected.total_tax, epsilon = 1e-6);
    assert_relative_eq!(p.net, p.proceeds - expected.total_tax, epsilon = 1e-6);
    assert!(expected.total_tax > 0.0);
}

#[test]
fn test_schedule_tax_should_be_computed_on_the_hifo_lot_sold() {
    let a = lot(1, 1000.0, 50.0, "2020-01-01");
    let b = lot(2, 1000.0, 150.0, "2021-01-01");
    let t = settings("TX", 100_000.0);
    let expected = compute_lot_tax(
        &b,
        1000.0,
        200.0,
        d("2027-01-01"),
        t.other_income,
        &t.brackets,
        &t.state,
        0.0,
    )
    .total_tax;
    let mut inp = input(vec![a, b], 200.0, 2, Cadence::Annual);
    inp.tax = Some(t);
    let s = build_schedule(&inp);
    assert_relative_eq!(s.periods[0].tax.unwrap(), expected, epsilon = 1e-6);
}

#[test]
fn test_schedule_when_lot_matures_before_sale_should_tax_later_sale_as_long_term() {
    // Acquired 2026-12-01: 31 days at the 2027-01-01 sale (short term), 396 days at 2028-01-01 (long term).
    let lots = vec![lot(1, 1000.0, 10.0, "2026-12-01")];
    let mut inp = input(lots, 110.0, 2, Cadence::Annual);
    inp.tax = Some(settings("TX", 300_000.0));
    let s = build_schedule(&inp);
    let (p1, p2) = (&s.periods[0], &s.periods[1]);
    assert_relative_eq!(p1.short_term_gain, p1.gain, epsilon = 1e-6);
    assert_relative_eq!(p1.long_term_gain, 0.0, epsilon = 1e-9);
    assert_relative_eq!(p2.long_term_gain, p2.gain, epsilon = 1e-6);
    assert_relative_eq!(p2.short_term_gain, 0.0, epsilon = 1e-9);
    // Equal gains, so the long-term sale must be taxed at a lower federal rate.
    assert_relative_eq!(p1.gain, p2.gain, epsilon = 1e-6);
    assert!(p2.tax.unwrap() < p1.tax.unwrap());
}

#[test]
fn test_schedule_when_selling_at_a_loss_should_have_zero_tax() {
    let mut inp = input(one_lot(1000.0, 100.0), 50.0, 1, Cadence::Annual);
    inp.tax = Some(settings("CA", 200_000.0));
    let p = &build_schedule(&inp).periods[0];
    assert!(p.gain < 0.0);
    assert_relative_eq!(p.tax.unwrap(), 0.0, epsilon = 1e-9);
    assert_relative_eq!(p.net, p.proceeds, epsilon = 1e-9);
}

/// WA's annual state deduction must apply once per calendar year across quarterly sales, and
/// reset in the next year. The state part of each period's tax is WA minus TX (TX has no state tax
/// and the federal part is identical).
#[test]
fn test_schedule_should_apply_state_annual_deduction_once_per_calendar_year() {
    let wa = state_tax_rate("WA").unwrap();
    let ded = wa.annual_deduction;
    assert!(ded > 0.0);
    // Four quarters per year, each realising a gain of 0.4 * deduction: 1.6 * deduction a year.
    let basis = 100.0;
    let price = basis + 0.4 * ded / 1000.0;
    let lots = vec![lot(1, 8000.0, basis, "2020-01-01")]; // 8 quarters of 1000 shares
    let mk = |state: &str| {
        let mut inp = input(lots.clone(), price, 2, Cadence::Quarterly);
        inp.tax = Some(settings(state, 100_000.0));
        build_schedule(&inp)
    };
    let (swa, stx) = (mk("WA"), mk("TX"));
    let state_part: Vec<f64> = swa
        .periods
        .iter()
        .zip(&stx.periods)
        .map(|(a, b)| a.tax.unwrap() - b.tax.unwrap())
        .collect();
    assert_eq!(state_part.len(), 8);
    // Year 1: the first two quarters stay under the deduction (0.4, 0.8 of it).
    assert_relative_eq!(state_part[0], 0.0, epsilon = 1e-6);
    assert_relative_eq!(state_part[1], 0.0, epsilon = 1e-6);
    assert!(state_part[2] > 0.0 && state_part[3] > 0.0);
    // Year total = (1.6 - 1.0) * deduction * the WA rate for the first tier.
    let year1: f64 = state_part[..4].iter().sum();
    assert_relative_eq!(year1, 0.6 * ded * wa.rate, max_relative = 1e-6);
    // Year 2 starts again from zero: same pattern.
    assert_relative_eq!(state_part[4], 0.0, epsilon = 1e-6);
    assert_relative_eq!(state_part[5], 0.0, epsilon = 1e-6);
    assert_relative_eq!(state_part[6], state_part[2], max_relative = 1e-6);
    assert_relative_eq!(state_part[7], state_part[3], max_relative = 1e-6);
}

#[test]
fn test_schedule_state_tax_should_make_high_tax_state_cost_more_than_no_tax_state() {
    let mk = |state: &str| {
        let mut inp = input(one_lot(1000.0, 10.0), 100.0, 1, Cadence::Annual);
        inp.tax = Some(settings(state, 100_000.0));
        build_schedule(&inp).totals.tax.unwrap()
    };
    assert!(mk("CA") > mk("TX"));
}

// ───────────────────────── projection ─────────────────────────

fn pin(years: u32, fractions: Vec<f64>) -> ProjectionInput {
    ProjectionInput {
        stock_value: 800_000.0,
        other_value: 200_000.0,
        years,
        stock_mu: DEFAULT_STOCK_MU,
        stock_sigma: DEFAULT_STOCK_SIGMA,
        rest_mu: DEFAULT_REST_MU,
        rest_sigma: DEFAULT_REST_SIGMA,
        rho: DEFAULT_RHO,
        paths: DEFAULT_PATHS,
        sell_fractions: fractions,
        tax_rate: 0.15,
    }
}

fn even(years: u32) -> Vec<f64> {
    vec![1.0 / years as f64; years as usize]
}

#[test]
fn test_projection_defaults_should_match_plan() {
    assert_relative_eq!(DEFAULT_STOCK_MU, 0.08);
    assert_relative_eq!(DEFAULT_STOCK_SIGMA, 0.30);
    assert_relative_eq!(DEFAULT_REST_MU, 0.06);
    assert_relative_eq!(DEFAULT_REST_SIGMA, 0.15);
    assert_relative_eq!(DEFAULT_RHO, 0.6);
    assert_eq!(DEFAULT_PATHS, 2000);
}

#[test]
fn test_projection_shape_should_have_years_plus_one_points_starting_at_start_value() {
    let p = project(&pin(5, even(5)), 7);
    assert_eq!(p.years, 5);
    for series in [&p.hold, &p.unwind] {
        assert_eq!(series.len(), 6);
        for (i, y) in series.iter().enumerate() {
            assert_eq!(y.year, i as u32);
        }
        let s = &series[0];
        for v in [s.p10, s.p25, s.p50, s.p75, s.p90] {
            assert_relative_eq!(v, 1_000_000.0, epsilon = 1e-6);
        }
    }
    assert_eq!(p.weight_p50_hold.len(), 6);
    assert_eq!(p.weight_p50_unwind.len(), 6);
    assert_relative_eq!(p.weight_p50_hold[0], 0.8, epsilon = 1e-9);
    assert_relative_eq!(p.weight_p50_unwind[0], 0.8, epsilon = 1e-9);
}

#[test]
fn test_projection_when_same_seed_should_be_identical() {
    let a = project(&pin(5, even(5)), 42);
    let b = project(&pin(5, even(5)), 42);
    for (x, y) in a
        .hold
        .iter()
        .chain(&a.unwind)
        .zip(b.hold.iter().chain(&b.unwind))
    {
        assert_eq!(
            (x.p10, x.p25, x.p50, x.p75, x.p90),
            (y.p10, y.p25, y.p50, y.p75, y.p90)
        );
    }
    assert_eq!(a.weight_p50_hold, b.weight_p50_hold);
    assert_eq!(a.weight_p50_unwind, b.weight_p50_unwind);
}

#[test]
fn test_projection_when_seed_differs_should_change_the_paths() {
    let a = project(&pin(5, even(5)), 1);
    let b = project(&pin(5, even(5)), 2);
    assert_ne!(a.hold[5].p50, b.hold[5].p50);
}

#[test]
fn test_projection_percentiles_should_be_ordered_every_year() {
    let p = project(&pin(5, even(5)), 7);
    for y in p.hold.iter().chain(&p.unwind) {
        assert!(
            y.p10 <= y.p25 && y.p25 <= y.p50 && y.p50 <= y.p75 && y.p75 <= y.p90,
            "year {}",
            y.year
        );
    }
    assert!(p.hold[5].p10 < p.hold[5].p90, "risky paths must spread");
}

#[test]
fn test_projection_with_zero_sigma_hold_should_equal_analytic_compounding() {
    let mut inp = pin(4, even(4));
    inp.stock_sigma = 0.0;
    inp.rest_sigma = 0.0;
    let p = project(&inp, 3);
    for y in &p.hold {
        let t = y.year as i32;
        let expected = 800_000.0 * 1.08_f64.powi(t) + 200_000.0 * 1.06_f64.powi(t);
        for v in [y.p10, y.p25, y.p50, y.p75, y.p90] {
            assert_relative_eq!(v, expected, max_relative = 1e-9);
        }
    }
}

#[test]
fn test_projection_with_zero_sigma_unwind_should_equal_analytic_sale_and_reinvest() {
    let fractions = vec![0.4, 0.3, 0.2, 0.1];
    let mut inp = pin(4, fractions.clone());
    inp.stock_sigma = 0.0;
    inp.rest_sigma = 0.0;
    let p = project(&inp, 3);
    // Sales happen at the start of each year, before that year's returns.
    let (mut stock_units, mut rest) = (1.0_f64, 200_000.0_f64); // stock_units: fraction of original shares
    let mut stock_price_value = 800_000.0_f64; // value of ALL original shares at year start
    for (y, f) in fractions.iter().enumerate() {
        let sale = f * stock_price_value;
        stock_units -= f;
        rest = (rest + (1.0 - inp.tax_rate) * sale) * 1.06;
        stock_price_value *= 1.08;
        let expected = stock_units * stock_price_value + rest;
        for v in [
            p.unwind[y + 1].p10,
            p.unwind[y + 1].p50,
            p.unwind[y + 1].p90,
        ] {
            assert_relative_eq!(v, expected, max_relative = 1e-9);
        }
    }
}

#[test]
fn test_projection_when_fully_unwound_should_have_zero_median_stock_weight_at_the_end() {
    let p = project(&pin(5, even(5)), 7);
    assert_relative_eq!(*p.weight_p50_unwind.last().unwrap(), 0.0, epsilon = 1e-12);
    assert!(
        *p.weight_p50_hold.last().unwrap() > 0.5,
        "hold keeps the concentration"
    );
    assert!(p.weight_p50_unwind.windows(2).all(|w| w[1] <= w[0] + 1e-9));
}

#[test]
fn test_projection_with_high_stock_vol_unwind_should_have_better_downside_than_hold() {
    let mut inp = pin(5, even(5));
    inp.stock_sigma = 0.60;
    let p = project(&inp, 11);
    assert!(
        p.unwind[5].p10 > p.hold[5].p10,
        "unwind p10 {} vs hold p10 {}",
        p.unwind[5].p10,
        p.hold[5].p10
    );
}

#[test]
fn test_projection_when_nothing_sold_unwind_should_match_hold() {
    let p = project(&pin(3, vec![0.0, 0.0, 0.0]), 5);
    for (a, b) in p.hold.iter().zip(&p.unwind) {
        assert_relative_eq!(a.p50, b.p50, max_relative = 1e-9);
        assert_relative_eq!(a.p10, b.p10, max_relative = 1e-9);
    }
}

#[test]
fn test_projection_when_assets_perfectly_correlated_should_spread_more_than_uncorrelated() {
    let spread = |rho: f64| {
        let mut inp = pin(5, even(5));
        inp.stock_value = 500_000.0;
        inp.other_value = 500_000.0;
        inp.stock_sigma = 0.30;
        inp.rest_sigma = 0.30;
        inp.rho = rho;
        let p = project(&inp, 21);
        p.hold[5].p90 - p.hold[5].p10
    };
    assert!(spread(1.0) > spread(0.0));
}

// ═════════════════════════ review round: year-level tax (M1, M2) ═════════════════════════
//
// New public API in `portfolio::unwind`:
//
// ```ignore
// #[derive(Debug, Clone, Copy, PartialEq)]
// pub struct YearTax { pub federal: f64, pub state: f64, pub niit: f64, pub total: f64 }
// /// Tax for ONE calendar year from the year's realised gains. `st_gain` / `lt_gain` are the year's
// /// short- and long-term gain totals (each may be negative; no carryforwards, no $3k offset).
// pub fn year_tax(st_gain: f64, lt_gain: f64, tax: &TaxSettings) -> YearTax;
// ```
//
// year_tax rules:
// * netting: a net loss on one side offsets the other (IRS-style); a negative overall result gives
//   zero tax (all fields 0.0). (The WA state figure uses the LT total; tests avoid ST/LT mixes there.)
// * federal = ordinary-bracket tax on net ST stacked on `other_income` (the ST brackets, as
//   `compute_lot_tax` does) + LT-bracket tax on net LT stacked on `other_income + net ST`.
// * niit = niit_rate * min(net, max(0, other_income + net - niit_threshold)), net = net ST + net LT.
// * state: states with `long_term_only` (WA) tax `max(0, lt - annual_deduction)` through the state's
//   brackets (stacked from 0); ST gains neither pay nor use the deduction. Other states: flat
//   `rate * max(0, net - annual_deduction)`.
// * total = federal + state + niit.
//
// `build_schedule` then taxes each period as `year_tax(YTD incl. period) - year_tax(YTD before)`,
// YTD = ST and LT gains realised so far in the period's calendar year (reset each January). A period
// tax may therefore be negative/zero when a loss is realised.

/// Federal tax of a single gain via the existing engine (basis 0, one share, price = gain).
fn ref_fed(gain: f64, long_term: bool, income: f64, b: &FederalCapGainsBrackets) -> f64 {
    let acquired = if long_term {
        "2020-01-01"
    } else {
        "2027-06-01"
    };
    let l = lot(99, 1.0, 0.0, acquired);
    compute_lot_tax(
        &l,
        1.0,
        gain,
        d("2027-07-01"),
        income,
        b,
        &state_tax_rate("TX").unwrap(),
        0.0,
    )
    .federal_tax
}

fn yt(st: f64, lt: f64, state: &str, income: f64) -> YearTax {
    year_tax(st, lt, &settings(state, income))
}

fn assert_yt_eq(a: YearTax, b: YearTax) {
    assert_relative_eq!(a.federal, b.federal, epsilon = 1e-6);
    assert_relative_eq!(a.state, b.state, epsilon = 1e-6);
    assert_relative_eq!(a.niit, b.niit, epsilon = 1e-6);
    assert_relative_eq!(a.total, b.total, epsilon = 1e-6);
}

#[test]
fn test_year_tax_when_long_term_only_should_match_engine_federal_stacking() {
    let b = FederalCapGainsBrackets::single_2024();
    let t = yt(0.0, 60_000.0, "TX", 100_000.0);
    assert_relative_eq!(
        t.federal,
        ref_fed(60_000.0, true, 100_000.0, &b),
        epsilon = 1e-6
    );
    assert_relative_eq!(t.state, 0.0, epsilon = 1e-9);
    assert_relative_eq!(t.total, t.federal + t.state + t.niit, epsilon = 1e-9);
}

#[test]
fn test_year_tax_should_tax_st_at_ordinary_rates_and_stack_lt_on_income_plus_st() {
    let b = FederalCapGainsBrackets::single_2024();
    let t = yt(20_000.0, 30_000.0, "TX", 30_000.0);
    let expected = ref_fed(20_000.0, false, 30_000.0, &b) + ref_fed(30_000.0, true, 50_000.0, &b);
    assert_relative_eq!(t.federal, expected, epsilon = 1e-6);
    assert_relative_eq!(t.niit, 0.0, epsilon = 1e-9); // 30k + 50k < 200k
}

#[test]
fn test_year_tax_when_lt_gain_crosses_the_20_percent_boundary_should_split_across_brackets() {
    let b = FederalCapGainsBrackets::single_2024();
    let t = yt(0.0, 160_000.0, "TX", 400_000.0);
    assert_relative_eq!(
        t.federal,
        ref_fed(160_000.0, true, 400_000.0, &b),
        epsilon = 1e-6
    );
    assert!(t.federal > 0.15 * 160_000.0 + 1.0 && t.federal < 0.20 * 160_000.0 - 1.0);
}

#[test]
fn test_year_tax_niit_should_apply_to_the_part_above_the_threshold() {
    let b = FederalCapGainsBrackets::single_2024();
    let rate = b.niit_rate;
    let thr = b.niit_threshold;
    for (income, st, lt) in [
        // (income, st, lt)
        (150_000.0_f64, 0.0_f64, 80_000.0_f64), // crosses the threshold: only the excess counts
        (250_000.0, 0.0, 50_000.0),             // already above: the whole gain counts
        (100_000.0, 30_000.0, 20_000.0),        // stays below: none
        (190_000.0, 5_000.0, 15_000.0),         // ST + LT together cross it
    ] {
        let net: f64 = st + lt;
        let expected = rate * net.min((income + net - thr).max(0.0));
        let t = yt(st, lt, "TX", income);
        assert_relative_eq!(t.niit, expected, epsilon = 1e-6);
    }
}

#[test]
fn test_year_tax_should_net_losses_across_short_and_long_term() {
    assert_yt_eq(
        yt(-30_000.0, 100_000.0, "CA", 100_000.0),
        yt(0.0, 70_000.0, "CA", 100_000.0),
    );
    assert_yt_eq(
        yt(50_000.0, -20_000.0, "CA", 100_000.0),
        yt(30_000.0, 0.0, "CA", 100_000.0),
    );
}

#[test]
fn test_year_tax_when_net_loss_should_be_zero_everywhere() {
    for (st, lt) in [
        (-10_000.0, -5_000.0),
        (-50_000.0, 20_000.0),
        (20_000.0, -50_000.0),
    ] {
        let t = yt(st, lt, "CA", 300_000.0);
        assert_relative_eq!(t.federal, 0.0, epsilon = 1e-9);
        assert_relative_eq!(t.state, 0.0, epsilon = 1e-9);
        assert_relative_eq!(t.niit, 0.0, epsilon = 1e-9);
        assert_relative_eq!(t.total, 0.0, epsilon = 1e-9);
    }
}

#[test]
fn test_year_tax_flat_state_should_tax_net_gain_at_its_rate() {
    let ca = state_tax_rate("CA").unwrap();
    let t = yt(20_000.0, 30_000.0, "CA", 100_000.0);
    assert_relative_eq!(t.state, ca.rate * 50_000.0, epsilon = 1e-6);
    let t = yt(-5_000.0, 30_000.0, "CA", 100_000.0);
    assert_relative_eq!(t.state, ca.rate * 25_000.0, epsilon = 1e-6);
}

#[test]
fn test_year_tax_wa_should_tax_long_term_gain_above_the_deduction_once() {
    let wa = state_tax_rate("WA").unwrap();
    let ded = wa.annual_deduction;
    assert_relative_eq!(
        yt(0.0, 0.9 * ded, "WA", 100_000.0).state,
        0.0,
        epsilon = 1e-9
    );
    assert_relative_eq!(
        yt(0.0, 1.2 * ded, "WA", 100_000.0).state,
        wa.rate * 0.2 * ded,
        max_relative = 1e-9
    );
}

#[test]
fn test_year_tax_wa_should_ignore_short_term_gains() {
    let wa = state_tax_rate("WA").unwrap();
    let ded = wa.annual_deduction;
    // Big ST gain: no WA tax, and it must not use up the deduction for the LT gain.
    assert_relative_eq!(
        yt(2.0 * ded, 0.0, "WA", 100_000.0).state,
        0.0,
        epsilon = 1e-9
    );
    assert_relative_eq!(
        yt(ded, 1.2 * ded, "WA", 100_000.0).state,
        wa.rate * 0.2 * ded,
        max_relative = 1e-9
    );
}

// ── schedule: M1 (deduction once per year across lots) ──

fn wa_minus_tx(input: &UnwindInput) -> Vec<f64> {
    let mk = |state: &str| {
        let mut i = input.clone();
        i.tax = Some(settings(state, 100_000.0));
        build_schedule(&i)
    };
    mk("WA")
        .periods
        .iter()
        .zip(&mk("TX").periods)
        .map(|(a, b)| a.tax.unwrap() - b.tax.unwrap())
        .collect()
}

#[test]
fn test_schedule_wa_should_share_one_deduction_across_lots_sold_in_the_same_period() {
    let wa = state_tax_rate("WA").unwrap();
    let ded = wa.annual_deduction;
    // Two lots, each gaining 0.6 * deduction, sold together in one annual period: 1.2 * deduction.
    let price = 100.0 + 0.6 * ded / 1000.0;
    let lots = vec![
        lot(1, 1000.0, 100.0, "2020-01-01"),
        lot(2, 1000.0, 100.0, "2021-01-01"),
    ];
    let parts = wa_minus_tx(&input(lots, price, 1, Cadence::Annual));
    assert_eq!(parts.len(), 1);
    assert_relative_eq!(parts[0], wa.rate * 0.2 * ded, max_relative = 1e-6);
}

#[test]
fn test_schedule_wa_should_share_the_deduction_across_lots_spread_over_quarters() {
    let wa = state_tax_rate("WA").unwrap();
    let ded = wa.annual_deduction;
    // HIFO: lot A (basis 100) in Q1-Q2, lot B (basis 50) in Q3-Q4. Total gain 1.5 * deduction.
    let price = (1.5 * ded + 300_000.0) / 4000.0;
    let lots = vec![
        lot(1, 2000.0, 100.0, "2020-01-01"),
        lot(2, 2000.0, 50.0, "2021-01-01"),
    ];
    let parts = wa_minus_tx(&input(lots, price, 1, Cadence::Quarterly));
    assert_relative_eq!(parts[0], 0.0, epsilon = 1e-6);
    assert_relative_eq!(parts[1], 0.0, epsilon = 1e-6);
    let q3_gain = 1000.0 * (price - 50.0);
    assert_relative_eq!(
        parts[2],
        wa.rate * (1000.0 * (price - 100.0) * 2.0 + q3_gain - ded),
        max_relative = 1e-6
    );
    assert_relative_eq!(parts[3], wa.rate * q3_gain, max_relative = 1e-6);
    assert_relative_eq!(
        parts.iter().sum::<f64>(),
        wa.rate * 0.5 * ded,
        max_relative = 1e-6
    );
}

// ── schedule: M2 (year-level tax) ──

fn year_groups(s: &UnwindSchedule) -> Vec<(i32, f64, f64, f64)> {
    // (year, short-term gain, long-term gain, summed period tax)
    let mut out: Vec<(i32, f64, f64, f64)> = Vec::new();
    for p in &s.periods {
        match out.last_mut() {
            Some(g) if g.0 == p.date.year() => {
                g.1 += p.short_term_gain;
                g.2 += p.long_term_gain;
                g.3 += p.tax.unwrap();
            }
            _ => out.push((
                p.date.year(),
                p.short_term_gain,
                p.long_term_gain,
                p.tax.unwrap(),
            )),
        }
    }
    out
}

fn assert_period_taxes_sum_to_year_tax(s: &UnwindSchedule, t: &TaxSettings) {
    for (year, st, lt, sum) in year_groups(s) {
        let expected = year_tax(st, lt, t).total;
        assert_relative_eq!(sum, expected, epsilon = 1e-6, max_relative = 1e-9);
        assert!(expected > 0.0, "year {year} should owe tax");
    }
}

#[test]
fn test_schedule_when_combined_lt_gain_crosses_the_boundary_should_tax_later_quarters_higher() {
    // 2000 sh, basis 0, price 160: 500 sh = 80,000 of LT gain per quarter, other income 400,000.
    let mut inp = input(one_lot(2000.0, 0.0), 160.0, 1, Cadence::Quarterly);
    let t = settings("TX", 400_000.0);
    inp.tax = Some(t.clone());
    let s = build_schedule(&inp);
    let q: Vec<f64> = s.periods.iter().map(|p| p.tax.unwrap()).collect();
    assert!(q[0] < q[1] && q[1] < q[2], "{q:?}"); // Q2 straddles 518,900
    assert_relative_eq!(q[2], q[3], max_relative = 1e-9); // fully in the 20% bracket
    assert_relative_eq!(
        q.iter().sum::<f64>(),
        year_tax(0.0, 320_000.0, &t).total,
        epsilon = 1e-6
    );
}

#[test]
fn test_schedule_when_niit_threshold_is_crossed_only_by_the_year_total_should_still_charge_niit() {
    // 20,000 of LT gain a quarter on 150,000 other income (threshold 200,000): no quarter crosses
    // it alone, the year (80,000) does.
    let mut inp = input(one_lot(2000.0, 0.0), 40.0, 1, Cadence::Quarterly);
    let t = settings("TX", 150_000.0);
    let b = t.brackets.clone();
    inp.tax = Some(t.clone());
    let s = build_schedule(&inp);
    let sum: f64 = s.periods.iter().map(|p| p.tax.unwrap()).sum();
    let niit = b.niit_rate * (150_000.0 + 80_000.0 - b.niit_threshold);
    assert!(niit > 0.0);
    assert_relative_eq!(
        sum,
        ref_fed(80_000.0, true, 150_000.0, &b) + niit,
        epsilon = 1e-6
    );
}

#[test]
fn test_schedule_when_loss_lot_and_gain_lot_share_a_year_should_net_them() {
    // HIFO: the loss lot (basis 150) goes in Q1-Q2 (-25,000 each), the gain lot (basis 0) in Q3-Q4
    // (+50,000 each): the year nets to +50,000 and tax only appears once YTD turns positive.
    let lots = vec![
        lot(1, 1000.0, 150.0, "2020-01-01"),
        lot(2, 1000.0, 0.0, "2020-06-01"),
    ];
    let mut inp = input(lots, 100.0, 1, Cadence::Quarterly);
    let t = settings("TX", 100_000.0);
    inp.tax = Some(t.clone());
    let s = build_schedule(&inp);
    let q: Vec<f64> = s.periods.iter().map(|p| p.tax.unwrap()).collect();
    for x in &q[..3] {
        assert_relative_eq!(*x, 0.0, epsilon = 1e-6);
    }
    let netted = year_tax(0.0, 50_000.0, &t).total;
    assert!(netted > 0.0);
    assert_relative_eq!(q[3], netted, epsilon = 1e-6);
    assert!(
        netted < year_tax(0.0, 100_000.0, &t).total,
        "must be less than taxing the gain lot alone"
    );
}

#[test]
fn test_schedule_period_taxes_should_sum_to_year_tax_for_mixed_short_and_long_term_lots() {
    // Lot S (basis 100) is still short term for its Q1/Q2 sales; lot L (basis 50) is long term.
    let lots = vec![
        lot(1, 1000.0, 100.0, "2026-12-01"),
        lot(2, 1000.0, 50.0, "2020-01-01"),
    ];
    let t = settings("CA", 100_000.0);
    let mut inp = input(lots, 150.0, 1, Cadence::Quarterly);
    inp.tax = Some(t.clone());
    let s = build_schedule(&inp);
    assert!(s.periods.iter().any(|p| p.short_term_gain > 0.0));
    assert!(s.periods.iter().any(|p| p.long_term_gain > 0.0));
    assert_period_taxes_sum_to_year_tax(&s, &t);
}

#[test]
fn test_schedule_year_level_tax_should_reset_every_calendar_year() {
    // 8 quarters: S (ST, basis 100) is sold during 2027, L (LT, basis 50) during 2028.
    let lots = vec![
        lot(1, 1000.0, 100.0, "2026-12-01"),
        lot(2, 1000.0, 50.0, "2020-01-01"),
    ];
    let t = settings("CA", 100_000.0);
    let mut inp = input(lots, 150.0, 2, Cadence::Quarterly);
    inp.tax = Some(t.clone());
    let s = build_schedule(&inp);
    assert_eq!(year_groups(&s).len(), 2);
    assert_period_taxes_sum_to_year_tax(&s, &t);
}

#[test]
fn test_schedule_totals_tax_should_equal_sum_of_year_taxes() {
    let t = settings("CA", 100_000.0);
    let mut inp = input(one_lot(2000.0, 20.0), 100.0, 2, Cadence::Quarterly);
    inp.tax = Some(t.clone());
    let s = build_schedule(&inp);
    let by_year: f64 = year_groups(&s)
        .iter()
        .map(|g| year_tax(g.1, g.2, &t).total)
        .sum();
    assert_relative_eq!(s.totals.tax.unwrap(), by_year, epsilon = 1e-6);
}
