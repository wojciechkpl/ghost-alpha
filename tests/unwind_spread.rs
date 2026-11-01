//! RED-phase contract for L2b slice 3 (1-5 year spread comparison), pure API in
//! `ghost_alpha::portfolio::unwind`. Synthetic data only (ticker XYZ). Fails to compile until the
//! symbols below exist.
//!
//! ```ignore
//! pub const SPREAD_MAX_YEARS: u32 = 5;
//! pub fn sell_fractions(schedule: &UnwindSchedule, cadence: Cadence, years: u32) -> Vec<f64>;
//! pub fn effective_tax_rate(totals: &UnwindTotals) -> f64;
//!
//! pub struct SpreadRow {              // Debug + Clone
//!     pub years: u32,                 // 1..=max_years
//!     pub sales: usize,               // periods in the schedule (years * sales per year)
//!     pub calendar_years: usize,      // schedule.years.len()
//!     pub proceeds: f64,              // schedule.totals.proceeds
//!     pub tax: Option<YearTax>,       // per-year YearTax summed component-wise; None without tax
//!     pub net: f64,                   // schedule.totals.net
//!     pub tax_rate: f64,              // effective_tax_rate(&schedule.totals)
//!     pub gain: f64,                  // schedule.totals.gain
//!     pub gain_tax_rate: f64,         // tax / gain when gain > 0, else 0 (also 0 without tax)
//!     pub saved_vs_1y: Option<f64>,   // (g_1 - g_N) * gain_N, g = gain_tax_rate; None without tax
//!     pub p10: f64, pub p50: f64,     // unwind portfolio value at the horizon
//! }
//! pub struct SpreadComparison {       // Debug + Clone
//!     pub horizon: u32,               // == max_years
//!     pub hold_p10: f64, pub hold_p50: f64,   // hold portfolio value at the horizon
//!     pub rows: Vec<SpreadRow>,       // N = 1..=max_years in order
//!     pub lowest_tax: Option<u32>,    // N with the lowest gain_tax_rate; ties within 0.0005 -> shorter (round 3)
//! }
//! pub fn compare_spreads(base: &UnwindInput, proj: &ProjectionInput, max_years: u32, seed: u64)
//!     -> SpreadComparison;
//! ```
//!
//! Assumptions pinned here (the spec leaves them open):
//! * `base.years` and `proj.years` / `proj.sell_fractions` / `proj.tax_rate` are OVERRIDDEN per
//!   spread: the inputs of row N are `build_schedule(base with years = N)` and
//!   `project(proj with years = max_years, sell_fractions = sell_fractions(..), tax_rate =
//!   effective_tax_rate(..), seed)`; `p10`/`p50` are the unwind fan at index `max_years`, the hold
//!   figures are the hold fan at index `max_years`.
//! * `horizon == max_years`, so "hold is the same for every row" is per call (one hold figure per
//!   comparison); it is not claimed to be equal across different `max_years`.

use approx::assert_relative_eq;
use chrono::NaiveDate;
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

fn base(lots: Vec<TaxLot>, cadence: Cadence, start: &str, tax: Option<TaxSettings>) -> UnwindInput {
    UnwindInput {
        lots,
        price: 100.0,
        other_value: 200_000.0,
        // Deliberately not 1..=5: compare_spreads must override it per spread.
        years: 9,
        cadence,
        growth: 0.0,
        start: d(start),
        tax,
    }
}

fn proj_base() -> ProjectionInput {
    ProjectionInput {
        stock_value: 800_000.0,
        other_value: 200_000.0,
        // Deliberately wrong placeholders: compare_spreads must override all three.
        years: 1,
        stock_mu: 0.08,
        stock_sigma: 0.30,
        rest_mu: 0.06,
        rest_sigma: 0.15,
        rho: 0.6,
        paths: 300,
        sell_fractions: vec![],
        tax_rate: 0.9,
    }
}

/// 8,000 shares @ 100 = 800,000; basis 20, long term; 640,000 gain.
fn lt_lots() -> Vec<TaxLot> {
    vec![lot(1, 8000.0, 20.0, "2020-01-01")]
}

/// 10,000 shares, basis 10, price 100: 900,000 long-term gain (above the WA deduction).
fn big_gain_lots() -> Vec<TaxLot> {
    vec![lot(1, 10_000.0, 10.0, "2020-01-01")]
}

fn sched(b: &UnwindInput, years: u32) -> UnwindSchedule {
    let mut i = b.clone();
    i.years = years;
    build_schedule(&i)
}

fn per_year(c: Cadence) -> usize {
    match c {
        Cadence::Quarterly => 4,
        Cadence::Annual => 1,
    }
}

// ─────────────────────────────── constants ───────────────────────────────

#[test]
fn test_spread_max_years_should_be_five() {
    assert_eq!(SPREAD_MAX_YEARS, 5);
}

// ─────────────────────────────── sell_fractions ───────────────────────────────

#[test]
fn test_sell_fractions_quarterly_one_year_should_sell_everything_in_year_one() {
    let s = sched(&base(lt_lots(), Cadence::Quarterly, "2027-01-15", None), 1);
    assert_eq!(s.periods.len(), 4);
    let f = sell_fractions(&s, Cadence::Quarterly, 1);
    assert_eq!(f.len(), 1);
    assert_relative_eq!(f[0], 1.0, epsilon = 1e-12);
}

#[test]
fn test_sell_fractions_annual_three_years_should_be_thirds() {
    let s = sched(&base(lt_lots(), Cadence::Annual, "2027-01-15", None), 3);
    let f = sell_fractions(&s, Cadence::Annual, 3);
    assert_eq!(f.len(), 3);
    for v in &f {
        assert_relative_eq!(*v, 1.0 / 3.0, epsilon = 1e-12);
    }
}

#[test]
fn test_sell_fractions_quarterly_two_years_should_be_halves() {
    let s = sched(&base(lt_lots(), Cadence::Quarterly, "2027-01-15", None), 2);
    let f = sell_fractions(&s, Cadence::Quarterly, 2);
    assert_eq!(f.len(), 2);
    assert_relative_eq!(f[0], 0.5, epsilon = 1e-12);
    assert_relative_eq!(f[1], 0.5, epsilon = 1e-12);
}

#[test]
fn test_sell_fractions_should_have_length_years_and_sum_to_one_for_every_spread() {
    for cadence in [Cadence::Quarterly, Cadence::Annual] {
        for years in 1..=5u32 {
            let s = sched(&base(lt_lots(), cadence, "2027-05-15", None), years);
            let f = sell_fractions(&s, cadence, years);
            assert_eq!(f.len(), years as usize, "{cadence:?} {years}");
            assert_relative_eq!(f.iter().sum::<f64>(), 1.0, epsilon = 1e-9);
            assert!(f.iter().all(|v| *v > 0.0), "{f:?}");
        }
    }
}

#[test]
fn test_sell_fractions_when_nothing_to_sell_should_be_zeros_of_length_years() {
    let s = sched(&base(vec![], Cadence::Annual, "2027-01-15", None), 3);
    let f = sell_fractions(&s, Cadence::Annual, 3);
    assert_eq!(f, vec![0.0, 0.0, 0.0]);
}

// ─────────────────────────────── effective_tax_rate ───────────────────────────────

fn totals(proceeds: f64, tax: Option<f64>) -> UnwindTotals {
    UnwindTotals {
        shares: 10.0,
        proceeds,
        cost_basis: 400.0,
        gain: proceeds - 400.0,
        tax,
        net: proceeds - tax.unwrap_or(0.0),
    }
}

#[test]
fn test_effective_tax_rate_should_be_tax_over_proceeds() {
    assert_relative_eq!(
        effective_tax_rate(&totals(1000.0, Some(150.0))),
        0.15,
        epsilon = 1e-12
    );
}

#[test]
fn test_effective_tax_rate_without_tax_should_be_zero() {
    assert_eq!(effective_tax_rate(&totals(1000.0, None)), 0.0);
}

#[test]
fn test_effective_tax_rate_without_proceeds_should_be_zero() {
    assert_eq!(effective_tax_rate(&totals(0.0, Some(0.0))), 0.0);
    assert_eq!(effective_tax_rate(&totals(0.0, Some(25.0))), 0.0);
}

#[test]
fn test_effective_tax_rate_should_match_a_built_schedule() {
    let b = base(
        big_gain_lots(),
        Cadence::Annual,
        "2027-01-15",
        Some(settings("WA", 0.0)),
    );
    let s = sched(&b, 2);
    let t = s.totals.tax.unwrap();
    assert!(t > 0.0);
    assert_relative_eq!(
        effective_tax_rate(&s.totals),
        t / s.totals.proceeds,
        epsilon = 1e-12
    );
    let empty = sched(
        &base(
            vec![],
            Cadence::Annual,
            "2027-01-15",
            Some(settings("WA", 0.0)),
        ),
        2,
    );
    assert_eq!(effective_tax_rate(&empty.totals), 0.0);
}

// ─────────────────────────────── compare_spreads: rows ───────────────────────────────

#[test]
fn test_compare_spreads_should_return_rows_for_one_to_max_years_in_order() {
    let b = base(lt_lots(), Cadence::Quarterly, "2027-05-15", None);
    let c = compare_spreads(&b, &proj_base(), 5, 42);
    let ys: Vec<u32> = c.rows.iter().map(|r| r.years).collect();
    assert_eq!(ys, vec![1, 2, 3, 4, 5]);
    assert_eq!(c.horizon, 5);
    let c3 = compare_spreads(&b, &proj_base(), 3, 42);
    let ys3: Vec<u32> = c3.rows.iter().map(|r| r.years).collect();
    assert_eq!(ys3, vec![1, 2, 3]);
    assert_eq!(c3.horizon, 3);
}

#[test]
fn test_compare_spreads_with_zero_max_years_should_give_no_rows() {
    let b = base(
        lt_lots(),
        Cadence::Quarterly,
        "2027-05-15",
        Some(settings("WA", 0.0)),
    );
    let c = compare_spreads(&b, &proj_base(), 0, 42);
    assert!(c.rows.is_empty());
    assert_eq!(c.lowest_tax, None);
}

#[test]
fn test_compare_spreads_rows_should_match_build_schedule_totals() {
    for cadence in [Cadence::Quarterly, Cadence::Annual] {
        let b = base(
            big_gain_lots(),
            cadence,
            "2027-05-15",
            Some(settings("WA", 50_000.0)),
        );
        let c = compare_spreads(&b, &proj_base(), 5, 42);
        for r in &c.rows {
            let s = sched(&b, r.years);
            let t = r.tax.expect("tax settings given");
            assert_relative_eq!(t.total, s.totals.tax.unwrap(), epsilon = 1e-6);
            assert_relative_eq!(r.proceeds, s.totals.proceeds, epsilon = 1e-6);
            assert_relative_eq!(r.net, s.totals.net, epsilon = 1e-6);
            assert_relative_eq!(r.tax_rate, effective_tax_rate(&s.totals), epsilon = 1e-12);
            assert_eq!(r.sales, r.years as usize * per_year(cadence), "{cadence:?}");
            assert_eq!(r.calendar_years, s.years.len(), "{cadence:?} {}", r.years);
        }
    }
}

#[test]
fn test_compare_spreads_tax_should_sum_the_calendar_years_component_by_component() {
    let b = base(
        big_gain_lots(),
        Cadence::Quarterly,
        "2027-05-15",
        Some(settings("WA", 50_000.0)),
    );
    let c = compare_spreads(&b, &proj_base(), 5, 42);
    for r in &c.rows {
        let s = sched(&b, r.years);
        let ys: Vec<YearTax> = s.years.iter().map(|y| y.tax.unwrap()).collect();
        let t = r.tax.unwrap();
        assert_relative_eq!(
            t.federal,
            ys.iter().map(|y| y.federal).sum::<f64>(),
            epsilon = 1e-6
        );
        assert_relative_eq!(
            t.state,
            ys.iter().map(|y| y.state).sum::<f64>(),
            epsilon = 1e-6
        );
        assert_relative_eq!(
            t.niit,
            ys.iter().map(|y| y.niit).sum::<f64>(),
            epsilon = 1e-6
        );
        assert_relative_eq!(t.total, t.federal + t.state + t.niit, epsilon = 1e-6);
    }
}

#[test]
fn test_compare_spreads_quarterly_from_mid_year_should_span_more_calendar_years_than_spread_years()
{
    // Sales on the 15th of Feb/May/Aug/Nov from 2027-05-15: hand-counted calendar years.
    let b = base(lt_lots(), Cadence::Quarterly, "2027-05-15", None);
    let c = compare_spreads(&b, &proj_base(), 5, 42);
    let cal: Vec<usize> = c.rows.iter().map(|r| r.calendar_years).collect();
    assert_eq!(cal, vec![2, 3, 4, 5, 6]);
    let sales: Vec<usize> = c.rows.iter().map(|r| r.sales).collect();
    assert_eq!(sales, vec![4, 8, 12, 16, 20]);
}

#[test]
fn test_compare_spreads_annual_should_tax_one_calendar_year_per_spread_year() {
    let b = base(lt_lots(), Cadence::Annual, "2027-05-15", None);
    let c = compare_spreads(&b, &proj_base(), 5, 42);
    let cal: Vec<usize> = c.rows.iter().map(|r| r.calendar_years).collect();
    assert_eq!(cal, vec![1, 2, 3, 4, 5]);
    let sales: Vec<usize> = c.rows.iter().map(|r| r.sales).collect();
    assert_eq!(sales, vec![1, 2, 3, 4, 5]);
}

#[test]
fn test_compare_spreads_should_ignore_the_years_field_of_its_base_inputs() {
    let mut b1 = base(lt_lots(), Cadence::Annual, "2027-01-15", None);
    let mut b2 = b1.clone();
    b1.years = 1;
    b2.years = 7;
    let (mut p1, mut p2) = (proj_base(), proj_base());
    p1.years = 2;
    p2.years = 8;
    p2.sell_fractions = vec![0.25; 4];
    p2.tax_rate = 0.0;
    let a = compare_spreads(&b1, &p1, 5, 42);
    let b = compare_spreads(&b2, &p2, 5, 42);
    assert_eq!(format!("{a:?}"), format!("{b:?}"));
}

// ─────────────────────────────── compare_spreads: projection ───────────────────────────────

/// What a row's p10/p50 must be, built only from public functions.
fn reference(b: &UnwindInput, p: &ProjectionInput, n: u32, horizon: u32, seed: u64) -> Projection {
    let s = sched(b, n);
    let mut pi = p.clone();
    pi.years = horizon;
    pi.sell_fractions = sell_fractions(&s, b.cadence, n);
    pi.tax_rate = effective_tax_rate(&s.totals);
    project(&pi, seed)
}

#[test]
fn test_compare_spreads_rows_should_project_over_the_common_horizon_with_the_same_seed() {
    for (tax, cadence) in [
        (Some(settings("WA", 0.0)), Cadence::Quarterly),
        (None, Cadence::Annual),
    ] {
        let b = base(big_gain_lots(), cadence, "2027-05-15", tax);
        let c = compare_spreads(&b, &proj_base(), 5, 20_261_031);
        for r in &c.rows {
            let p = reference(&b, &proj_base(), r.years, 5, 20_261_031);
            assert_relative_eq!(r.p10, p.unwind[5].p10, max_relative = 1e-9);
            assert_relative_eq!(r.p50, p.unwind[5].p50, max_relative = 1e-9);
            assert_relative_eq!(c.hold_p10, p.hold[5].p10, max_relative = 1e-9);
            assert_relative_eq!(c.hold_p50, p.hold[5].p50, max_relative = 1e-9);
        }
    }
}

#[test]
fn test_compare_spreads_hold_should_not_depend_on_the_spread_or_the_tax() {
    let with_tax = base(
        big_gain_lots(),
        Cadence::Annual,
        "2027-01-15",
        Some(settings("WA", 0.0)),
    );
    let no_tax = base(big_gain_lots(), Cadence::Annual, "2027-01-15", None);
    let a = compare_spreads(&with_tax, &proj_base(), 5, 7);
    let b = compare_spreads(&no_tax, &proj_base(), 5, 7);
    assert_eq!(a.hold_p10, b.hold_p10);
    assert_eq!(a.hold_p50, b.hold_p50);
    // Hold at the horizon is the plain hold fan of the projection.
    let p = reference(&no_tax, &proj_base(), 1, 5, 7);
    assert_relative_eq!(a.hold_p10, p.hold[5].p10, max_relative = 1e-9);
}

#[test]
fn test_compare_spreads_when_max_years_is_three_should_use_a_three_year_horizon() {
    let b = base(lt_lots(), Cadence::Annual, "2027-01-15", None);
    let c = compare_spreads(&b, &proj_base(), 3, 5);
    let p = reference(&b, &proj_base(), 2, 3, 5);
    assert_relative_eq!(c.rows[1].p50, p.unwind[3].p50, max_relative = 1e-9);
    assert_relative_eq!(c.hold_p50, p.hold[3].p50, max_relative = 1e-9);
}

#[test]
fn test_compare_spreads_should_be_deterministic_per_seed_and_vary_with_it() {
    let b = base(
        big_gain_lots(),
        Cadence::Quarterly,
        "2027-05-15",
        Some(settings("WA", 0.0)),
    );
    let a = compare_spreads(&b, &proj_base(), 5, 11);
    let again = compare_spreads(&b, &proj_base(), 5, 11);
    assert_eq!(format!("{a:?}"), format!("{again:?}"));
    let other = compare_spreads(&b, &proj_base(), 5, 12);
    assert_ne!(a.rows[2].p10, other.rows[2].p10);
    assert_ne!(a.hold_p50, other.hold_p50);
}

#[test]
fn test_compare_spreads_with_zero_volatility_should_equal_the_analytic_values() {
    // 800k stock at 8% and 200k rest at 6%, no tax: sell 1/N of the shares at the start of
    // each of the first N years, proceeds join the rest, everything compounds to year 5.
    let b = base(lt_lots(), Cadence::Annual, "2027-01-15", None);
    let mut p = proj_base();
    p.stock_sigma = 0.0;
    p.rest_sigma = 0.0;
    let c = compare_spreads(&b, &p, 5, 3);
    let hold = 800_000.0 * 1.08_f64.powi(5) + 200_000.0 * 1.06_f64.powi(5);
    assert_relative_eq!(c.hold_p10, hold, max_relative = 1e-9);
    assert_relative_eq!(c.hold_p50, hold, max_relative = 1e-9);
    for r in &c.rows {
        let n = r.years as usize;
        let (mut stock_all, mut rest, mut units) = (800_000.0_f64, 200_000.0_f64, 1.0_f64);
        for y in 0..5 {
            if y < n {
                let f = 1.0 / n as f64;
                rest += f * stock_all;
                units -= f;
            }
            rest *= 1.06;
            stock_all *= 1.08;
        }
        let expected = units.max(0.0) * stock_all + rest;
        assert_relative_eq!(r.p50, expected, max_relative = 1e-9);
        assert_relative_eq!(r.p10, expected, max_relative = 1e-9);
    }
}

// ─────────────────────────────── compare_spreads: tax and lowest_tax ───────────────────────────────

#[test]
fn test_compare_spreads_without_tax_settings_should_have_no_tax_and_no_lowest() {
    let b = base(lt_lots(), Cadence::Quarterly, "2027-05-15", None);
    let c = compare_spreads(&b, &proj_base(), 5, 42);
    assert_eq!(c.lowest_tax, None);
    for r in &c.rows {
        assert!(r.tax.is_none());
        assert!(r.saved_vs_1y.is_none(), "{r:?}");
        assert_eq!(r.tax_rate, 0.0);
        assert_relative_eq!(r.net, r.proceeds, epsilon = 1e-9);
        assert_relative_eq!(r.proceeds, 800_000.0, epsilon = 1e-6);
    }
}

#[test]
fn test_compare_spreads_wa_long_term_gain_above_deduction_should_tax_less_the_longer_the_spread() {
    let b = base(
        big_gain_lots(),
        Cadence::Annual,
        "2027-01-15",
        Some(settings("WA", 0.0)),
    );
    let c = compare_spreads(&b, &proj_base(), 5, 42);
    let totals: Vec<f64> = c.rows.iter().map(|r| r.tax.unwrap().total).collect();
    for w in totals.windows(2) {
        assert!(w[1] < w[0], "{totals:?}");
    }
    assert_eq!(c.lowest_tax, Some(5));
    // The state share is what shrinks: WA's deduction applies to each calendar year.
    let state: Vec<f64> = c.rows.iter().map(|r| r.tax.unwrap().state).collect();
    assert!(state[4] < state[0], "{state:?}");
}

#[test]
fn test_compare_spreads_when_every_spread_pays_nothing_should_pick_the_shortest() {
    // Price equals basis: no gain, so every spread's tax is within $0.50 of zero.
    let flat = vec![lot(1, 8000.0, 100.0, "2020-01-01")];
    let b = base(
        flat,
        Cadence::Annual,
        "2027-01-15",
        Some(settings("WA", 0.0)),
    );
    let c = compare_spreads(&b, &proj_base(), 5, 42);
    for r in &c.rows {
        assert!(r.tax.unwrap().total.abs() < 0.5, "{r:?}");
    }
    assert_eq!(c.lowest_tax, Some(1));
}

/// The rule under test: the shortest spread whose tax rate is within 0.0005 of the lowest rate.
fn expected_lowest(c: &SpreadComparison) -> u32 {
    let min = c
        .rows
        .iter()
        .map(|r| r.gain_tax_rate)
        .fold(f64::INFINITY, f64::min);
    c.rows
        .iter()
        .find(|r| r.gain_tax_rate <= min + 0.0005)
        .map(|r| r.years)
        .unwrap()
}

fn with_growth(mut b: UnwindInput, g: f64) -> UnwindInput {
    b.growth = g;
    b
}

/// CA, 120k other income, 10,000 sh @ 10, annual, +8% a year. Hand-checked against
/// `build_schedule`: total tax is lowest at 2 years ($302,770) but the tax RATE keeps falling
/// (31.1%, 29.1%, 28.3%, 28.2%, 28.1%) because later sales are at higher prices.
fn growth_scenario() -> UnwindInput {
    with_growth(
        base(
            big_gain_lots(),
            Cadence::Annual,
            "2027-01-15",
            Some(settings("CA", 120_000.0)),
        ),
        0.08,
    )
}

#[test]
fn test_compare_spreads_lowest_tax_should_rank_by_gain_tax_rate_not_dollars_when_prices_grow() {
    let c = compare_spreads(&growth_scenario(), &proj_base(), 5, 42);
    let dollars: Vec<f64> = c.rows.iter().map(|r| r.tax.unwrap().total).collect();
    let dollar_min = dollars
        .iter()
        .position(|t| *t == dollars.iter().cloned().fold(f64::INFINITY, f64::min))
        .unwrap()
        + 1;
    assert_eq!(
        dollar_min, 2,
        "precondition: dollar minimum at 2 years: {dollars:?}"
    );
    assert_eq!(
        c.lowest_tax,
        Some(5),
        "rates {:?}",
        c.rows.iter().map(|r| r.tax_rate).collect::<Vec<_>>()
    );
    assert_eq!(c.lowest_tax, Some(expected_lowest(&c)));
}

#[test]
fn test_compare_spreads_lowest_tax_tolerance_should_pick_the_shorter_of_two_close_gain_rates() {
    // CA, 50k other income, 8,000 sh @ 50, annual, +8%. Tax as a share of the GAIN:
    // 30.68%, 29.46%, 28.56%, 28.32%, 28.30%. The exact minimum is 5 years, but 4 years is
    // within 0.0005 of it (0.00015) while 3 years is not (0.0026): the shortest within
    // tolerance wins.
    let b = with_growth(
        base(
            vec![lot(1, 8000.0, 50.0, "2020-01-01")],
            Cadence::Annual,
            "2027-01-15",
            Some(settings("CA", 50_000.0)),
        ),
        0.08,
    );
    let c = compare_spreads(&b, &proj_base(), 5, 42);
    let g: Vec<f64> = c.rows.iter().map(|r| r.gain_tax_rate).collect();
    assert!(g[4] < g[3] && g[3] - g[4] < 0.0005, "precondition: {g:?}");
    assert!(g[2] - g[4] > 0.0005, "precondition: {g:?}");
    assert_eq!(c.lowest_tax, Some(4), "{g:?}");
    assert_eq!(c.lowest_tax, Some(expected_lowest(&c)));
    // Ranking by tax / proceeds would have picked another row (1 year, within tolerance of 2).
    let pmin = c
        .rows
        .iter()
        .map(|r| r.tax_rate)
        .fold(f64::INFINITY, f64::min);
    let by_proceeds = c
        .rows
        .iter()
        .find(|r| r.tax_rate <= pmin + 0.0005)
        .unwrap()
        .years;
    assert_ne!(
        by_proceeds, 4,
        "fixture must separate gain rate from proceeds rate"
    );
}

#[test]
fn test_compare_spreads_lowest_tax_with_growth_zero_should_still_pick_the_cheapest_rate() {
    let b = base(
        big_gain_lots(),
        Cadence::Quarterly,
        "2027-05-15",
        Some(settings("CA", 120_000.0)),
    );
    let c = compare_spreads(&b, &proj_base(), 5, 42);
    assert_eq!(c.lowest_tax, Some(expected_lowest(&c)));
}

// ─────────────────────────────── gain, gain_tax_rate, saved_vs_1y ───────────────────────────────

#[test]
fn test_compare_spreads_rows_should_carry_the_schedule_gain_and_the_tax_rate_on_it() {
    for growth in [0.0, 0.08] {
        let b = with_growth(
            base(
                big_gain_lots(),
                Cadence::Quarterly,
                "2027-05-15",
                Some(settings("CA", 120_000.0)),
            ),
            growth,
        );
        let c = compare_spreads(&b, &proj_base(), 5, 42);
        for r in &c.rows {
            let s = sched(&b, r.years);
            assert_relative_eq!(r.gain, s.totals.gain, epsilon = 1e-6);
            assert!(r.gain > 0.0);
            assert_relative_eq!(
                r.gain_tax_rate,
                s.totals.tax.unwrap() / s.totals.gain,
                epsilon = 1e-12
            );
        }
    }
}

#[test]
fn test_compare_spreads_gain_tax_rate_should_be_zero_without_tax_settings() {
    let b = base(lt_lots(), Cadence::Annual, "2027-01-15", None);
    let c = compare_spreads(&b, &proj_base(), 5, 42);
    for r in &c.rows {
        assert!(r.gain > 0.0, "gain is reported without tax: {r:?}");
        assert_eq!(r.gain_tax_rate, 0.0);
    }
}

#[test]
fn test_compare_spreads_gain_tax_rate_when_selling_at_a_loss_should_be_zero() {
    // Basis 150, price 100: every sale realises a loss.
    let b = base(
        vec![lot(1, 1000.0, 150.0, "2020-01-01")],
        Cadence::Annual,
        "2027-01-15",
        Some(settings("CA", 120_000.0)),
    );
    let c = compare_spreads(&b, &proj_base(), 5, 42);
    for r in &c.rows {
        assert!(r.gain < 0.0, "{r:?}");
        assert_eq!(r.gain_tax_rate, 0.0, "{r:?}");
        assert!(r.saved_vs_1y.unwrap().abs() < 1e-9, "{r:?}");
    }
}

#[test]
fn test_compare_spreads_saved_vs_1y_for_one_year_should_be_zero() {
    let c = compare_spreads(&growth_scenario(), &proj_base(), 5, 42);
    assert_eq!(c.rows[0].saved_vs_1y, Some(0.0));
}

#[test]
fn test_compare_spreads_saved_vs_1y_with_flat_prices_should_equal_the_dollar_difference() {
    let b = base(
        big_gain_lots(),
        Cadence::Annual,
        "2027-01-15",
        Some(settings("WA", 0.0)),
    );
    let c = compare_spreads(&b, &proj_base(), 5, 42);
    let t1 = c.rows[0].tax.unwrap().total;
    for r in &c.rows {
        let want = t1 - r.tax.unwrap().total;
        assert_relative_eq!(r.saved_vs_1y.unwrap(), want, epsilon = 1e-6);
    }
    assert!(c.rows[4].saved_vs_1y.unwrap() > 0.0);
}

#[test]
fn test_compare_spreads_saved_vs_1y_with_growth_should_apply_the_one_year_gain_rate_to_the_gain() {
    let c = compare_spreads(&growth_scenario(), &proj_base(), 5, 42);
    let g1 = c.rows[0].gain_tax_rate;
    for r in &c.rows {
        assert_relative_eq!(
            r.saved_vs_1y.unwrap(),
            (g1 - r.gain_tax_rate) * r.gain,
            epsilon = 1e-6
        );
    }
    // It is not the dollar difference: 5 years pays MORE dollars than 1 year ...
    let r5 = &c.rows[4];
    let dollar_diff = c.rows[0].tax.unwrap().total - r5.tax.unwrap().total;
    assert!(dollar_diff < 0.0, "precondition: {dollar_diff}");
    // ... yet it saves tax at the 1-year gain rate (about $41,000).
    assert!(r5.saved_vs_1y.unwrap() > 30_000.0, "{:?}", r5.saved_vs_1y);
}

#[test]
fn test_compare_spreads_with_a_flat_rate_should_save_nothing_whatever_the_price_path() {
    // TX single, 80k other income, all long term inside the 15% band, below the NIIT threshold:
    // the tax on the gain is 15% for every spread, so spreading saves exactly nothing and the
    // 1-year spread is the lowest. (Ranking by tax / proceeds would move with the price path.)
    for growth in [-0.10, 0.0, 0.08] {
        let b = with_growth(
            base(
                vec![lot(1, 1000.0, 20.0, "2020-01-01")],
                Cadence::Annual,
                "2027-01-15",
                Some(settings("TX", 80_000.0)),
            ),
            growth,
        );
        let c = compare_spreads(&b, &proj_base(), 5, 42);
        for r in &c.rows {
            assert_relative_eq!(r.gain_tax_rate, 0.15, epsilon = 1e-9);
            assert!(
                r.saved_vs_1y.unwrap().abs() < 0.01,
                "growth {growth}, {} yr: {:?}",
                r.years,
                r.saved_vs_1y
            );
        }
        assert_eq!(c.lowest_tax, Some(1), "growth {growth}");
    }
}

#[test]
fn test_compare_spreads_saved_vs_1y_is_negative_when_a_longer_spread_pushes_the_gain_into_tax() {
    // TX, no other income, 1,000 sh @ 60, +50% a year: the 1-year gain ($40k) sits in the 0%
    // band, while 5 years of rising prices put $89k in the last year into the 15% band.
    let b = with_growth(
        base(
            vec![lot(1, 1000.0, 60.0, "2020-01-01")],
            Cadence::Annual,
            "2027-01-15",
            Some(settings("TX", 0.0)),
        ),
        0.5,
    );
    let c = compare_spreads(&b, &proj_base(), 5, 42);
    assert_eq!(c.rows[0].gain_tax_rate, 0.0);
    let r5 = &c.rows[4];
    assert!(r5.gain_tax_rate > 0.03, "{r5:?}");
    let saved = r5.saved_vs_1y.unwrap();
    assert!(saved < -5_000.0, "{saved}");
    assert_relative_eq!(
        saved,
        (c.rows[0].gain_tax_rate - r5.gain_tax_rate) * r5.gain,
        epsilon = 1e-6
    );
    // 1 and 2 years both pay nothing: the shorter one is the lowest.
    assert_eq!(c.lowest_tax, Some(1));
}
