//! RED for Track A (fix/cli-year-tax): the multi-year projection behind `sell-plan` and
//! `sell-compare` must tax each calendar year as a whole through `year_tax`, not lot by lot.
//!
//! Contract: for every entry, federal / state / NIIT are the component-wise difference
//! `year_tax(ytd + entry) - year_tax(ytd)`, with `ytd` kept per calendar year. `total_tax` is the sum
//! of the three. `realised_gain`, `proceeds` and `shares_sold` are unchanged.
//!
//! Fixtures are synthetic (ticker MSFT, round numbers). Every test pairs a hand-computed number with
//! the `year_tax` oracle. Hand numbers use `single_2024` brackets: LT 0% to 47,025, 15% to 518,900,
//! 20% above; NIIT 3.8% over 200,000. The state is CA (13.3% flat, no deduction) or TX (0%) unless
//! stated otherwise.

use approx::assert_relative_eq;
use chrono::NaiveDate;
use ghost_alpha::portfolio::prepare::{prepare_selling_programme, SellDistribution, SellPlanInput};
use ghost_alpha::portfolio::types::{AcquisitionType, TaxLot};
use ghost_alpha::portfolio::unwind::{year_tax, TaxSettings};
use ghost_alpha::tax::brackets::{
    state_tax_rate, FederalCapGainsBrackets, FilingStatus, StateTaxRate,
};
use ghost_alpha::tax::projection::{
    compare_scenarios, project_multi_year, project_multi_year_with_strategy, LotSelectionStrategy,
    YearProjection,
};
use uuid::Uuid;

const EPS: f64 = 1e-6;

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn lot(n: u128, acquired: &str, basis: f64, shares: f64) -> TaxLot {
    TaxLot {
        id: Uuid::from_u128(n),
        ticker: "MSFT".into(),
        shares,
        cost_basis_per_share: basis,
        acquired_date: d(acquired),
        acquisition_type: AcquisitionType::Buy,
        remaining_shares: shares,
    }
}

/// Long term at every notional 1 July sale in 2027/2028.
fn lt_lot(n: u128, basis: f64, shares: f64) -> TaxLot {
    lot(n, "2020-01-01", basis, shares)
}

/// Bought 1 March 2027: short term at the notional 1 July 2027 sale (122 days).
fn st_lot(n: u128, basis: f64, shares: f64) -> TaxLot {
    lot(n, "2027-03-01", basis, shares)
}

fn single() -> FederalCapGainsBrackets {
    FederalCapGainsBrackets::single_2024()
}

fn settings(state: &StateTaxRate, other_income: f64) -> TaxSettings {
    TaxSettings {
        state: state.clone(),
        brackets: single(),
        other_income,
    }
}

fn hifo(
    lots: &[TaxLot],
    entries: &[(u32, f64)],
    price: f64,
    other_income: f64,
    state: &StateTaxRate,
) -> Vec<YearProjection> {
    project_multi_year(lots, entries, 0.0, price, other_income, &single(), state)
}

fn assert_components(p: &YearProjection, federal: f64, state: f64, niit: f64) {
    assert_relative_eq!(p.federal_tax, federal, epsilon = EPS);
    assert_relative_eq!(p.state_tax, state, epsilon = EPS);
    assert_relative_eq!(p.niit, niit, epsilon = EPS);
    assert_relative_eq!(p.total_tax, federal + state + niit, epsilon = EPS);
}

// ── Losses offset gains within a year ───────────────────────────────────────

#[test]
fn test_project_multi_year_when_lt_loss_lot_sold_with_lt_gain_lot_should_tax_only_the_net() {
    // Gain lot: basis 50, 1000 sh, price 120 -> +70,000. Loss lot: basis 150 -> -30,000. Net LT 40,000.
    // Single, other income 100,000 (above the 0% LT band), CA.
    // Federal 40,000 * 15% = 6,000; state 40,000 * 13.3% = 5,320; NIIT 0 (140,000 < 200,000).
    // Per-lot taxation would give 10,500 federal and 9,310 state.
    let ca = state_tax_rate("CA").unwrap();
    let lots = [lt_lot(1, 50.0, 1000.0), lt_lot(2, 150.0, 1000.0)];
    let r = hifo(&lots, &[(2027, 2000.0)], 120.0, 100_000.0, &ca);

    assert_eq!(r.len(), 1);
    assert_relative_eq!(r[0].realised_gain, 40_000.0, epsilon = EPS);
    assert_relative_eq!(r[0].proceeds, 240_000.0, epsilon = EPS);
    assert_relative_eq!(r[0].shares_sold, 2000.0, epsilon = EPS);
    assert_components(&r[0], 6_000.0, 5_320.0, 0.0);

    let oracle = year_tax(0.0, 40_000.0, &settings(&ca, 100_000.0));
    assert_relative_eq!(r[0].federal_tax, oracle.federal, epsilon = EPS);
    assert_relative_eq!(r[0].state_tax, oracle.state, epsilon = EPS);
    assert_relative_eq!(r[0].niit, oracle.niit, epsilon = EPS);
}

#[test]
fn test_project_multi_year_when_st_loss_lot_sold_with_lt_gain_lot_should_net_across_terms() {
    // ST loss -30,000 offsets LT gain +70,000: net 40,000 taxed at LT rates (year_tax's netting).
    let ca = state_tax_rate("CA").unwrap();
    let lots = [st_lot(1, 150.0, 1000.0), lt_lot(2, 50.0, 1000.0)];
    let r = hifo(&lots, &[(2027, 2000.0)], 120.0, 100_000.0, &ca);

    assert_relative_eq!(r[0].realised_gain, 40_000.0, epsilon = EPS);
    assert_components(&r[0], 6_000.0, 5_320.0, 0.0);
    let oracle = year_tax(-30_000.0, 70_000.0, &settings(&ca, 100_000.0));
    assert_relative_eq!(r[0].total_tax, oracle.total, epsilon = EPS);
}

#[test]
fn test_project_multi_year_when_lots_net_to_a_loss_should_owe_nothing() {
    // +20,000 and -30,000 in the same year: net loss, so zero tax (no carryforward).
    let ca = state_tax_rate("CA").unwrap();
    let lots = [lt_lot(1, 100.0, 1000.0), lt_lot(2, 150.0, 1000.0)];
    let r = hifo(&lots, &[(2027, 2000.0)], 120.0, 100_000.0, &ca);

    assert_relative_eq!(r[0].realised_gain, -10_000.0, epsilon = EPS);
    assert_components(&r[0], 0.0, 0.0, 0.0);
}

#[test]
fn test_project_multi_year_when_loss_comes_in_an_earlier_entry_should_offset_a_later_same_year_gain(
) {
    // HIFO sells the loss lot first. Entry 1 (-30,000) owes 0. Entry 2 (+70,000) is taxed as
    // year_tax(net 40,000) - year_tax(net -30,000) = 6,000 federal, 5,320 state.
    let ca = state_tax_rate("CA").unwrap();
    let lots = [lt_lot(1, 50.0, 1000.0), lt_lot(2, 150.0, 1000.0)];
    let r = hifo(
        &lots,
        &[(2027, 1000.0), (2027, 1000.0)],
        120.0,
        100_000.0,
        &ca,
    );

    assert_eq!(r.len(), 2);
    assert_relative_eq!(r[0].realised_gain, -30_000.0, epsilon = EPS);
    assert_components(&r[0], 0.0, 0.0, 0.0);
    assert_relative_eq!(r[1].realised_gain, 70_000.0, epsilon = EPS);
    assert_components(&r[1], 6_000.0, 5_320.0, 0.0);
}

// ── Brackets stack across lots ──────────────────────────────────────────────

#[test]
fn test_project_multi_year_when_two_lt_lots_cross_the_15_to_20_boundary_should_stack_brackets() {
    // Two LT lots, +100,000 each, other income 400,000. The 20% band starts at 518,900.
    // Combined 200,000 from 400,000: 118,900 at 15% = 17,835 and 81,100 at 20% = 16,220 -> 34,055.
    // Per-lot taxation gives 30,000 (both lots seen as starting from 400,000).
    // NIIT = 3.8% * min(200,000, 400,000) = 7,600.
    let tx = state_tax_rate("TX").unwrap();
    let lots = [lt_lot(1, 100.0, 1000.0), lt_lot(2, 100.0, 1000.0)];
    let r = hifo(&lots, &[(2027, 2000.0)], 200.0, 400_000.0, &tx);

    assert_components(&r[0], 34_055.0, 0.0, 7_600.0);
    let oracle = year_tax(0.0, 200_000.0, &settings(&tx, 400_000.0));
    assert_relative_eq!(r[0].federal_tax, oracle.federal, epsilon = EPS);
    assert_relative_eq!(r[0].niit, oracle.niit, epsilon = EPS);
}

#[test]
fn test_project_multi_year_when_gain_crosses_the_0_to_15_boundary_should_tax_only_the_excess() {
    // Other income 0; two LT lots with +30,000 each. Combined 60,000: first 47,025 at 0%, rest
    // 12,975 at 15% = 1,946.25. Per-lot: 0% on each 30,000 (no stacking) = 0.
    let tx = state_tax_rate("TX").unwrap();
    let lots = [lt_lot(1, 100.0, 300.0), lt_lot(2, 100.0, 300.0)];
    let r = hifo(&lots, &[(2027, 600.0)], 200.0, 0.0, &tx);

    assert_relative_eq!(r[0].realised_gain, 60_000.0, epsilon = EPS);
    assert_components(&r[0], 1_946.25, 0.0, 0.0);
}

// ── NIIT is min(net, excess over threshold) ─────────────────────────────────

#[test]
fn test_project_multi_year_when_income_is_near_the_niit_threshold_should_charge_niit_on_the_excess()
{
    // (other income, LT gain, expected federal, expected NIIT)
    //  150,000 / 100,000: excess 50,000 -> NIIT 1,900; federal 100,000 * 15% = 15,000
    //  190,000 /  20,000: excess 10,000 -> NIIT   380; federal  20,000 * 15% =  3,000
    //  250,000 / 100,000: already over    -> NIIT 3,800 (full gain); federal 15,000
    let tx = state_tax_rate("TX").unwrap();
    let rows = [
        (150_000.0, 100_000.0, 15_000.0, 1_900.0),
        (190_000.0, 20_000.0, 3_000.0, 380.0),
        (250_000.0, 100_000.0, 15_000.0, 3_800.0),
    ];
    for (income, gain, federal, niit) in rows {
        let lots = [lt_lot(1, 100.0, 1000.0)];
        let price = 100.0 + gain / 1000.0;
        let r = hifo(&lots, &[(2027, 1000.0)], price, income, &tx);
        assert_relative_eq!(r[0].realised_gain, gain, epsilon = EPS);
        assert_relative_eq!(r[0].federal_tax, federal, epsilon = EPS);
        assert_relative_eq!(r[0].niit, niit, epsilon = EPS);
        let oracle = year_tax(0.0, gain, &settings(&tx, income));
        assert_relative_eq!(r[0].niit, oracle.niit, epsilon = EPS);
    }
}

// ── Washington: LT-only, deduction, 9.9% tier above 1M ──────────────────────

#[test]
fn test_project_multi_year_when_wa_has_st_gain_lt_loss_and_lt_gain_should_tax_net_lt_only() {
    // Price 1,378. ST lot basis 1,078 -> +300,000 (WA ignores it and it uses none of the 278,000
    // deduction). LT gain lot basis 100 -> +1,278,000. LT loss lot basis 1,578 -> -200,000.
    // Net LT 1,078,000; taxable above the deduction 800,000: 722,000 at 7% = 50,540
    // plus 78,000 at 9.9% = 7,722 -> 58,262. Per-lot taxation gives 78,062 (the loss is ignored).
    let wa = state_tax_rate("WA").unwrap();
    let lots = [
        st_lot(1, 1_078.0, 1000.0),
        lt_lot(2, 100.0, 1000.0),
        lt_lot(3, 1_578.0, 1000.0),
    ];
    let r = hifo(&lots, &[(2027, 3000.0)], 1_378.0, 100_000.0, &wa);

    assert_relative_eq!(
        r[0].realised_gain,
        300_000.0 + 1_278_000.0 - 200_000.0,
        epsilon = EPS
    );
    assert_relative_eq!(r[0].state_tax, 58_262.0, epsilon = EPS);
    let oracle = year_tax(300_000.0, 1_078_000.0, &settings(&wa, 100_000.0));
    assert_relative_eq!(r[0].state_tax, oracle.state, epsilon = EPS);
    assert_relative_eq!(r[0].federal_tax, oracle.federal, epsilon = EPS);
    assert_relative_eq!(r[0].niit, oracle.niit, epsilon = EPS);
    assert_relative_eq!(r[0].total_tax, oracle.total, epsilon = EPS);
}

#[test]
fn test_project_multi_year_when_wa_lt_gain_is_over_one_million_above_deduction_should_apply_top_tier(
) {
    // Two LT lots, +639,000 each = 1,278,000. Taxable 1,000,000: 722,000 * 7% = 50,540 plus
    // 278,000 * 9.9% = 27,522 -> 78,062. The second lot is taxed in the higher tier.
    let wa = state_tax_rate("WA").unwrap();
    let lots = [lt_lot(1, 100.0, 500.0), lt_lot(2, 100.0, 500.0)];
    let price = 100.0 + 1_278_000.0 / 1000.0;
    let r = hifo(&lots, &[(2027, 1000.0)], price, 100_000.0, &wa);

    assert_relative_eq!(r[0].realised_gain, 1_278_000.0, epsilon = 1e-4);
    assert_relative_eq!(r[0].state_tax, 78_062.0, epsilon = 1e-3);
    // Federal brackets stack across the two lots too: oracle on the combined gain.
    let oracle = year_tax(0.0, 1_278_000.0, &settings(&wa, 100_000.0));
    assert_relative_eq!(r[0].federal_tax, oracle.federal, epsilon = 1e-3);
    assert_relative_eq!(r[0].niit, oracle.niit, epsilon = 1e-3);
}

// ── Same-year entries stack; different years do not ─────────────────────────

#[test]
fn test_project_multi_year_when_two_entries_share_a_year_should_sum_to_one_combined_entry() {
    // Four equal LT lots, +100,000 per 1000 sh at price 200. Other income 400,000, CA.
    let ca = state_tax_rate("CA").unwrap();
    let lots = [
        lt_lot(1, 100.0, 1000.0),
        lt_lot(2, 100.0, 1000.0),
        lt_lot(3, 100.0, 1000.0),
        lt_lot(4, 100.0, 1000.0),
    ];
    let split = hifo(
        &lots,
        &[(2027, 2000.0), (2027, 2000.0)],
        200.0,
        400_000.0,
        &ca,
    );
    let combined = hifo(&lots, &[(2027, 4000.0)], 200.0, 400_000.0, &ca);

    let sum = |f: fn(&YearProjection) -> f64| split.iter().map(f).sum::<f64>();
    assert_relative_eq!(
        sum(|p| p.federal_tax),
        combined[0].federal_tax,
        epsilon = EPS
    );
    assert_relative_eq!(sum(|p| p.state_tax), combined[0].state_tax, epsilon = EPS);
    assert_relative_eq!(sum(|p| p.niit), combined[0].niit, epsilon = EPS);
    assert_relative_eq!(sum(|p| p.total_tax), combined[0].total_tax, epsilon = EPS);

    // Hand numbers: combined 400,000 gain from 400,000 -> 118,900 at 15% + 281,100 at 20% = 74,055;
    // CA 400,000 * 13.3% = 53,200; NIIT 3.8% * 400,000 = 15,200.
    assert_components(&combined[0], 74_055.0, 53_200.0, 15_200.0);
    // Entry 1 alone (200,000): 34,055 / 26,600 / 7,600; entry 2 is the remainder.
    assert_components(&split[0], 34_055.0, 26_600.0, 7_600.0);
    assert_components(&split[1], 40_000.0, 26_600.0, 7_600.0);
}

#[test]
fn test_project_multi_year_when_entries_are_in_different_years_should_tax_each_year_alone() {
    // 1000 sh sold per year, +100,000 each, other income 150,000, TX: each year stands alone.
    // Federal 100,000 * 15% = 15,000; NIIT 3.8% * min(100,000, 50,000) = 1,900 per year
    // (per-lot taxation charges 3,800).
    let tx = state_tax_rate("TX").unwrap();
    let lots = [lt_lot(1, 100.0, 1000.0), lt_lot(2, 100.0, 1000.0)];
    let r = hifo(
        &lots,
        &[(2027, 1000.0), (2028, 1000.0)],
        200.0,
        150_000.0,
        &tx,
    );

    assert_eq!(r.len(), 2);
    for p in &r {
        assert_components(p, 15_000.0, 0.0, 1_900.0);
        assert_relative_eq!(p.realised_gain, 100_000.0, epsilon = EPS);
    }
}

// ── Priority strategy uses the same netting ─────────────────────────────────

#[test]
fn test_project_multi_year_with_priority_strategy_when_st_loss_and_lt_gain_should_net_in_the_year()
{
    // Priority sells the ST loss lot first, then the LT gain lot. Same year, same netting as HIFO:
    // net 40,000 -> 6,000 federal, 5,320 CA state.
    let ca = state_tax_rate("CA").unwrap();
    let lots = [lt_lot(1, 50.0, 1000.0), st_lot(2, 150.0, 1000.0)];
    let r = project_multi_year_with_strategy(
        &lots,
        &[(2027, 2000.0)],
        0.0,
        120.0,
        100_000.0,
        &single(),
        &ca,
        &LotSelectionStrategy::Priority { ltcg_hold_days: 30 },
    );

    assert_relative_eq!(r[0].realised_gain, 40_000.0, epsilon = EPS);
    assert_components(&r[0], 6_000.0, 5_320.0, 0.0);
    let oracle = year_tax(-30_000.0, 70_000.0, &settings(&ca, 100_000.0));
    assert_relative_eq!(r[0].total_tax, oracle.total, epsilon = EPS);
}

// ── compare_scenarios ───────────────────────────────────────────────────────

#[test]
fn test_compare_scenarios_totals_should_equal_the_sum_of_per_year_year_tax() {
    // Loss lot (basis 150) is sold first, then the gain lot (basis 50); price 120, other 100,000, CA.
    // 1-year spread: one calendar year, net +40,000 -> fed 6,000 + state 5,320 = 11,320.
    // 2-year spread: 2026 = -30,000 -> 0; 2027 = +70,000 -> fed 10,500 + state 9,310 = 19,810.
    let ca = state_tax_rate("CA").unwrap();
    let lots = [lt_lot(1, 50.0, 1000.0), lt_lot(2, 150.0, 1000.0)];
    let s = compare_scenarios(
        &lots,
        2000.0,
        2,
        2026,
        0.0,
        120.0,
        100_000.0,
        &single(),
        &ca,
    );
    assert_eq!(s.len(), 2);

    let ts = settings(&ca, 100_000.0);
    let one = year_tax(0.0, 40_000.0, &ts);
    assert_relative_eq!(s[0].total_federal_tax, one.federal, epsilon = EPS);
    assert_relative_eq!(s[0].total_state_tax, one.state, epsilon = EPS);
    assert_relative_eq!(s[0].total_niit, one.niit, epsilon = EPS);
    assert_relative_eq!(s[0].total_tax, 11_320.0, epsilon = EPS);

    let y1 = year_tax(0.0, -30_000.0, &ts);
    let y2 = year_tax(0.0, 70_000.0, &ts);
    assert_relative_eq!(
        s[1].total_federal_tax,
        y1.federal + y2.federal,
        epsilon = EPS
    );
    assert_relative_eq!(s[1].total_state_tax, y1.state + y2.state, epsilon = EPS);
    assert_relative_eq!(s[1].total_tax, y1.total + y2.total, epsilon = EPS);
    assert_relative_eq!(s[1].total_tax, 19_810.0, epsilon = EPS);

    assert_relative_eq!(s[1].savings_vs_lump, 11_320.0 - 19_810.0, epsilon = EPS);
    assert!(s[0].is_optimal && !s[1].is_optimal);
}

// ── sell-plan (prepare) ─────────────────────────────────────────────────────

#[test]
fn test_prepare_selling_programme_when_single_year_gain_straddles_niit_threshold_should_use_partial_niit(
) {
    // 1000 sh, basis 100, price 200, LT, Single, TX, other income 150,000, one year.
    // Federal 15,000; NIIT 3.8% * min(100,000, 50,000) = 1,900; total 16,900 (per-lot: 18,800).
    let plan = SellPlanInput {
        ticker: "MSFT".into(),
        total_shares: 1000.0,
        cost_basis_per_share: 100.0,
        current_price: 200.0,
        acquired_date: d("2020-01-01"),
        filing_status: FilingStatus::Single,
        state_code: "TX".into(),
        other_annual_income: 150_000.0,
        assumed_price_growth: 0.0,
        horizon_years: 1,
        start_year: 2027,
        distribution: SellDistribution::Even,
    };
    let out = prepare_selling_programme(&plan).unwrap();

    assert_relative_eq!(out.summary.total_gain, 100_000.0, epsilon = EPS);
    assert_relative_eq!(out.summary.total_federal_tax, 15_000.0, epsilon = EPS);
    assert_relative_eq!(out.summary.total_niit, 1_900.0, epsilon = EPS);
    assert_relative_eq!(out.summary.total_tax, 16_900.0, epsilon = EPS);
    assert_relative_eq!(out.year_projections[0].niit, 1_900.0, epsilon = EPS);
}

// ── CLI: sell-plan / sell-compare JSON ──────────────────────────────────────

fn run_cli_json(args: &[&str]) -> serde_json::Value {
    // Minimal config (the yahoo provider needs no API key); temp cwd so no user data is touched.
    let dir = tempfile::tempdir().unwrap();
    let cfg = dir.path().join("cli.toml");
    std::fs::write(&cfg, "[plugins]\nactive_provider = \"yahoo\"\n").unwrap();
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_ghost-alpha"));
    c.current_dir(dir.path())
        .env("RUST_LOG", "error")
        .arg("--config")
        .arg(&cfg);
    for (k, _) in std::env::vars_os() {
        let upper = k.to_string_lossy().to_uppercase();
        if upper.starts_with("GHOST_ALPHA_") || upper.starts_with("FINNHUB") {
            c.env_remove(&k);
        }
    }
    let out = c.args(args).output().expect("run ghost-alpha");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is JSON")
}

#[test]
fn test_cli_sell_plan_json_when_gain_straddles_niit_threshold_should_report_partial_niit() {
    let v = run_cli_json(&[
        "sell-plan",
        "--ticker",
        "MSFT",
        "--shares",
        "1000",
        "--cost-basis",
        "100",
        "--price",
        "200",
        "--acquired",
        "2020-01-01",
        "--filing",
        "single",
        "--state",
        "TX",
        "--other-income",
        "150000",
        "--years",
        "1",
        "--format",
        "json",
    ]);
    assert_relative_eq!(
        v["summary"]["total_niit"].as_f64().unwrap(),
        1_900.0,
        epsilon = EPS
    );
    assert_relative_eq!(
        v["summary"]["total_tax"].as_f64().unwrap(),
        16_900.0,
        epsilon = EPS
    );
}

#[test]
fn test_cli_sell_compare_json_when_gain_straddles_niit_threshold_should_report_partial_niit() {
    let v = run_cli_json(&[
        "sell-compare",
        "--ticker",
        "MSFT",
        "--shares",
        "1000",
        "--cost-basis",
        "100",
        "--price",
        "200",
        "--acquired",
        "2020-01-01",
        "--filing",
        "single",
        "--state",
        "TX",
        "--other-income",
        "150000",
        "--max-years",
        "1",
        "--format",
        "json",
    ]);
    let first = &v.as_array().expect("array of scenarios")[0];
    assert_relative_eq!(
        first["total_niit"].as_f64().unwrap(),
        1_900.0,
        epsilon = EPS
    );
    assert_relative_eq!(
        first["total_tax"].as_f64().unwrap(),
        16_900.0,
        epsilon = EPS
    );
}

// ── Review follow-ups ───────────────────────────────────────────────────────

fn mfj_settings(state: &StateTaxRate, other_income: f64) -> TaxSettings {
    TaxSettings {
        state: state.clone(),
        brackets: FederalCapGainsBrackets::mfj_2024(),
        other_income,
    }
}

#[test]
fn test_year_tax_when_wa_has_st_loss_and_lt_gain_with_net_zero_or_loss_should_still_owe_wa_tax() {
    // WA taxes the LT gain (600,000 - 278,000 deduction = 322,000 at 7% = 22,540) even when an ST
    // loss pulls the federal net to zero or below. Federal and NIIT stay 0.
    let wa = state_tax_rate("WA").unwrap();
    let ts = mfj_settings(&wa, 300_000.0);
    // (st, lt): net exactly 0, then a deeper ST loss (net -300,000).
    for (st, lt) in [(-600_000.0, 600_000.0), (-900_000.0, 600_000.0)] {
        let t = year_tax(st, lt, &ts);
        assert_relative_eq!(t.federal, 0.0, epsilon = EPS);
        assert_relative_eq!(t.niit, 0.0, epsilon = EPS);
        assert_relative_eq!(t.state, 22_540.0, epsilon = EPS);
        assert_relative_eq!(t.total, 22_540.0, epsilon = EPS);
    }
}

#[test]
fn test_year_tax_when_non_long_term_only_state_has_net_loss_should_owe_nothing() {
    let ca = state_tax_rate("CA").unwrap();
    let ts = mfj_settings(&ca, 300_000.0);
    for (st, lt) in [(-600_000.0, 600_000.0), (-900_000.0, 600_000.0)] {
        let t = year_tax(st, lt, &ts);
        assert_relative_eq!(t.federal, 0.0, epsilon = EPS);
        assert_relative_eq!(t.state, 0.0, epsilon = EPS);
        assert_relative_eq!(t.niit, 0.0, epsilon = EPS);
        assert_relative_eq!(t.total, 0.0, epsilon = EPS);
    }
}

#[test]
fn test_project_multi_year_with_strategy_when_wa_st_loss_and_lt_gain_net_to_zero_should_owe_wa_tax()
{
    // ST lot: basis 1,600, price 1,000 -> -600,000. LT lot: basis 400 -> +600,000. MFJ, other 300,000.
    let wa = state_tax_rate("WA").unwrap();
    let lots = [st_lot(1, 1_600.0, 1000.0), lt_lot(2, 400.0, 1000.0)];
    let r = project_multi_year_with_strategy(
        &lots,
        &[(2027, 2000.0)],
        0.0,
        1_000.0,
        300_000.0,
        &FederalCapGainsBrackets::mfj_2024(),
        &wa,
        &LotSelectionStrategy::Hifo,
    );
    assert_relative_eq!(r[0].realised_gain, 0.0, epsilon = EPS);
    assert_components(&r[0], 0.0, 22_540.0, 0.0);
}

#[test]
fn test_project_multi_year_when_years_are_interleaved_should_keep_ytd_per_calendar_year() {
    // [(2027,a),(2028,b),(2027,c)]: lots are +100,000 per 1000 sh, other 150,000, CA.
    // 2027 combined 200,000: fed 30,000, NIIT 3.8% * min(200,000, 150,000) = 5,700, state 26,600.
    // 2028 alone 100,000: fed 15,000, NIIT 1,900, state 13,300.
    let ca = state_tax_rate("CA").unwrap();
    let lots = [
        lt_lot(1, 100.0, 1000.0),
        lt_lot(2, 100.0, 1000.0),
        lt_lot(3, 100.0, 1000.0),
    ];
    let r = hifo(
        &lots,
        &[(2027, 1000.0), (2028, 1000.0), (2027, 1000.0)],
        200.0,
        150_000.0,
        &ca,
    );
    let ts = settings(&ca, 150_000.0);
    let y27 = year_tax(0.0, 200_000.0, &ts);
    let y28 = year_tax(0.0, 100_000.0, &ts);

    assert_relative_eq!(
        r[0].federal_tax + r[2].federal_tax,
        y27.federal,
        epsilon = EPS
    );
    assert_relative_eq!(r[0].state_tax + r[2].state_tax, y27.state, epsilon = EPS);
    assert_relative_eq!(r[0].niit + r[2].niit, y27.niit, epsilon = EPS);
    assert_components(&r[1], y28.federal, y28.state, y28.niit);
    // Hand numbers.
    assert_components(&r[0], 15_000.0, 13_300.0, 1_900.0);
    assert_components(&r[1], 15_000.0, 13_300.0, 1_900.0);
    assert_components(&r[2], 15_000.0, 13_300.0, 3_800.0);
}
