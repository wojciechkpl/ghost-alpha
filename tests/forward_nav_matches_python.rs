//! Step 6 of Gap 1: cross-language regression test.
//!
//! Runs `project_path` end-to-end with the synthetic example portfolio
//! (example_main.csv + example_small.csv), the synthetic MSFT vesting schedule
//! (vesting/sample.csv) and the deterministic-base scenario, then asserts
//! year-5 NAV under HOLD, SellReinvest and OptionBUnwind match the Python
//! reference fixture within 1%.
//!
//! Fixture: tests/fixtures/python_mc/audit_deterministic.json
//! Reference parameters: tests/fixtures/python_mc/README.md

use chrono::NaiveDate;
use ghost_alpha::portfolio::import::{read_fidelity_positions_csv, roll_up_to_portfolio};
use ghost_alpha::portfolio::symbol_map::SymbolMap;
use ghost_alpha::portfolio::types::Portfolio;
use ghost_alpha::portfolio::vesting::DistributionSchedule;
use ghost_alpha::projection::asset_class_library::AssetClassLibrary;
use ghost_alpha::projection::monte_carlo::AssetClass;
use ghost_alpha::projection::portfolio_nav::{
    project_path, OptionBConfig, ProjectionConfig, Treatment,
};
use ghost_alpha::projection::scenario::ScenarioPath;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

const PY_FIXTURE: &str = include_str!("fixtures/python_mc/audit_deterministic.json");

// Parameters of the Python reference run (see tests/fixtures/python_mc/README.md).
const WITHHOLD_RATE: f64 = 0.22;
const LTCG_RATE: f64 = 0.238;
const COST_BASIS: f64 = 185.0;
const FLOOR_PCT: f64 = 0.25;
const CONCENTRATED_SYMBOL: &str = "MSFT";
const TOLERANCE_PCT: f64 = 1.0;

#[derive(Deserialize)]
struct PythonAudit {
    mode: String,
    total_nav_t0: f64,
    year5_nav: HashMap<String, f64>,
}

fn t0() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()
}

fn fixture(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel)
}

/// Synthetic example portfolio: example_main.csv + example_small.csv
/// (NAV 1,043,770.00 as of 2026-05-01; MSFT is 52.31%).
fn example_portfolio() -> Portfolio {
    let mut rows = read_fidelity_positions_csv(&fixture("fidelity/example_main.csv")).unwrap();
    rows.extend(read_fidelity_positions_csv(&fixture("fidelity/example_small.csv")).unwrap());
    roll_up_to_portfolio(&rows, t0())
}

/// 12 synthetic MSFT vests from tests/fixtures/vesting/sample.csv.
fn example_vesting_schedule() -> DistributionSchedule {
    DistributionSchedule::from_csv_file(&fixture("vesting/sample.csv")).unwrap()
}

/// ProjectionConfig matching tests/fixtures/python_mc/README.md.
fn reference_config() -> ProjectionConfig {
    let mut cfg = ProjectionConfig::new(CONCENTRATED_SYMBOL);
    cfg.symbol_map = SymbolMap::builtin();
    cfg.option_b = OptionBConfig {
        cost_basis_per_share: COST_BASIS,
        ltcg_rate: LTCG_RATE,
        floor_pct: FLOOR_PCT,
        unwind_schedule: vec![(1, 400.0), (2, 300.0)],
        ..OptionBConfig::default()
    };
    cfg
}

/// Build a deterministic ScenarioPath where every year's per-class return is
/// the long-run mean from the library (Python's "base" scenario).
fn deterministic_base_scenario(library: &AssetClassLibrary, horizon: u32) -> ScenarioPath {
    let zero_year: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
    let constant_year: HashMap<AssetClass, f64> = AssetClass::iter_all()
        .map(|c| (c, library.mean(c)))
        .collect();

    let mut yearly_returns = vec![zero_year];
    for _ in 0..horizon {
        yearly_returns.push(constant_year.clone());
    }
    ScenarioPath {
        id: "deterministic_base".to_string(),
        yearly_returns,
    }
}

fn rust_y5_nav(treatment: Treatment) -> f64 {
    let library = AssetClassLibrary::builtin_defaults();
    let scenario = deterministic_base_scenario(&library, 5);
    let portfolio = example_portfolio();
    let vesting = example_vesting_schedule();

    let states = project_path(
        &portfolio,
        &vesting,
        treatment,
        &scenario,
        &library,
        WITHHOLD_RATE,
        &reference_config(),
    )
    .expect("project_path should succeed");
    assert_eq!(states.len(), 6, "horizon=5 → 6 states (year 0..5)");
    states[5].nav()
}

fn assert_within_pct(rust_nav: f64, py_nav: f64, label: &str) {
    let diff_pct = 100.0 * (rust_nav - py_nav).abs() / py_nav;
    assert!(
        diff_pct < TOLERANCE_PCT,
        "{}: rust=${:.2} python=${:.2} diff={:.3}% (> {:.1}% tolerance)",
        label,
        rust_nav,
        py_nav,
        diff_pct,
        TOLERANCE_PCT
    );
}

#[test]
fn rust_hold_y5_nav_matches_python_within_1pct() {
    let py: PythonAudit = serde_json::from_str(PY_FIXTURE).expect("fixture parses");
    assert_eq!(py.mode, "deterministic");

    let py_nav = py.year5_nav["base-HOLD"];
    let rust_nav = rust_y5_nav(Treatment::Hold);
    assert_within_pct(rust_nav, py_nav, "base-HOLD y5");
}

#[test]
fn rust_sell_reinvest_y5_nav_matches_python_within_1pct() {
    let py: PythonAudit = serde_json::from_str(PY_FIXTURE).expect("fixture parses");
    let py_nav = py.year5_nav["base-SELL_REINVEST"];
    let rust_nav = rust_y5_nav(Treatment::SellReinvest);
    assert_within_pct(rust_nav, py_nav, "base-SELL_REINVEST y5");
}

#[test]
fn rust_option_b_unwind_y5_nav_matches_python_within_1pct() {
    let py: PythonAudit = serde_json::from_str(PY_FIXTURE).expect("fixture parses");
    let py_nav = py.year5_nav["base-OPTION_B_UNWIND"];
    let rust_nav = rust_y5_nav(Treatment::OptionBUnwind);
    assert_within_pct(rust_nav, py_nav, "base-OPTION_B_UNWIND y5");
}

#[test]
fn example_inputs_match_the_fixture_header() {
    let py: PythonAudit = serde_json::from_str(PY_FIXTURE).expect("fixture parses");
    let portfolio = example_portfolio();
    assert!(
        (portfolio.total_market_value() - py.total_nav_t0).abs() < 0.01,
        "synthetic portfolio NAV {} != fixture total_nav_t0 {}",
        portfolio.total_market_value(),
        py.total_nav_t0
    );
    assert!((py.total_nav_t0 - 1_043_770.0).abs() < 0.01);
    let vesting = example_vesting_schedule();
    assert_eq!(vesting.len(), 12);
    assert!((vesting.total_gross_quantity("MSFT") - 420.0).abs() < 1e-9);
}

#[test]
fn msft_is_the_resolved_concentrated_symbol_for_the_example_inputs() {
    use ghost_alpha::projection::portfolio_nav::resolve_concentrated_symbol;
    let portfolio = example_portfolio();
    let vesting = example_vesting_schedule();
    let map = SymbolMap::builtin();
    // via vesting
    assert_eq!(
        resolve_concentrated_symbol(None, &vesting, &portfolio, &map).as_deref(),
        Some("MSFT")
    );
    // via largest non-cash position (no vesting)
    let none = DistributionSchedule { events: vec![] };
    assert_eq!(
        resolve_concentrated_symbol(None, &none, &portfolio, &map).as_deref(),
        Some("MSFT")
    );
}
