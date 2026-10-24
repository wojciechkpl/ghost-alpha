//! End-to-end plan orchestrator for the `ghost-alpha plan` subcommand.
//!
//! Composes:
//!   - positions import (Fidelity export or simple CSV) -> `roll_up_to_portfolio`
//!   - `DistributionSchedule::from_csv_file` (optional vesting schedule)
//!   - `SymbolMap` (built-in + optional `symbol_map.toml`)
//!   - `AssetClassLibrary::from_toml_file`
//!   - `build_block_corr` + `run_monte_carlo` + `percentiles_per_year`
//!   - `project_path` (per scenario x treatment, driven by a `ProjectionConfig`)
//!   - output: `nav_projection.csv`, `nav_mc_paths.csv`, `audit.json`
//!
//! Also provides the data-directory helpers (`discover_data_dir`,
//! `load_plan_toml`, `write_data_templates`). See `docs/PLAN_PIPELINE.md`.

use crate::portfolio::symbol_map::parse_asset_class;
use crate::projection::monte_carlo::AssetClass;
use crate::projection::portfolio_nav::Treatment;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Default (relative) location of the asset-class assumptions file.
pub const DEFAULT_ASSET_CLASSES_PATH: &str = "config/asset_classes.toml";

/// Copy of `config/asset_classes.toml` built into the binary. Used when the
/// default path does not exist relative to the working directory.
const EMBEDDED_ASSET_CLASSES_TOML: &str = include_str!("../../config/asset_classes.toml");

/// Optional overrides for the Option B (concentration unwind) parameters.
/// `None` means "use the derived value or built-in default".
#[derive(Debug, Clone, Default)]
pub struct OptionBOverrides {
    /// Cost basis per share of the concentrated position.
    pub cost_basis_per_share: Option<f64>,
    pub ltcg_rate: Option<f64>,
    pub floor_pct: Option<f64>,
    /// `(year, shares)` pairs.
    pub unwind_schedule: Option<Vec<(u32, f64)>>,
    /// `(ticker, weight)` pairs for reinvesting after-tax proceeds.
    pub reinvest_weights: Option<Vec<(String, f64)>>,
}

/// Inputs for the plan orchestrator. Caller provides paths + scenario knobs.
#[derive(Debug, Clone)]
pub struct PlanInputs {
    /// One or more positions CSVs (Fidelity export or simple format, auto-detected).
    pub positions_csvs: Vec<PathBuf>,
    /// Vesting schedule CSV; `None` if no vests.
    pub vesting_csv: Option<PathBuf>,
    /// config/asset_classes.toml
    pub asset_classes_toml: PathBuf,
    pub treatment: Treatment,
    pub mc_paths: usize,
    pub mc_seed: u64,
    pub horizon_years: u32,
    pub withhold_rate: f64,
    pub as_of: NaiveDate,
    pub out_dir: PathBuf,
    /// Concentrated symbol; `None` = auto-resolve (vesting symbol, else largest non-cash position).
    pub symbol: Option<String>,
    /// Optional `symbol_map.toml` merged over the built-in map.
    pub symbol_map_toml: Option<PathBuf>,
    /// Fallback asset class for unmapped tickers (overrides `default_class` in the TOML map).
    pub default_class: Option<AssetClass>,
    pub option_b: OptionBOverrides,
}

/// Summary returned by the orchestrator. Detailed artifacts written to out_dir.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanReport {
    pub treatment: Treatment,
    pub seed: u64,
    pub n_paths: usize,
    pub starting_nav: f64,
    pub year5_nav_p05: f64,
    pub year5_nav_p50: f64,
    pub year5_nav_p95: f64,
    pub year5_nav_mean: f64,
    /// Files written to out_dir (absolute paths).
    pub artifact_paths: Vec<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("Positions import: {0}")]
    Fidelity(#[from] crate::portfolio::import::FidelityImportError),
    #[error("Vesting import: {0}")]
    Vesting(#[from] crate::portfolio::vesting::VestingError),
    #[error("Asset library: {0}")]
    Library(#[from] crate::projection::asset_class_library::AssetClassLibraryError),
    #[error("Symbol map: {0}")]
    SymbolMap(#[from] crate::portfolio::symbol_map::SymbolMapError),
    #[error("Projection: {0}")]
    Projection(#[from] crate::projection::portfolio_nav::ProjectionError),
    #[error("CSV write: {0}")]
    Csv(#[from] csv::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Data directory: {0}")]
    DataDir(String),
    #[error("plan.toml: {0}")]
    PlanToml(String),
    #[error("Invalid input: {0}")]
    Invalid(String),
    #[error(
        "Could not determine the concentrated symbol (no vesting schedule and no non-cash \
         position). Pass --symbol or set `symbol` in plan.toml"
    )]
    NoConcentratedSymbol,
}

/// Parse a treatment name: `hold | sell-reinvest | option-b` (several spellings accepted).
pub fn parse_treatment(s: &str) -> Option<Treatment> {
    match s {
        "hold" | "Hold" | "HOLD" => Some(Treatment::Hold),
        "sell-reinvest" | "sell_reinvest" | "SellReinvest" | "SELL_REINVEST" => {
            Some(Treatment::SellReinvest)
        }
        "option-b" | "option_b" | "OptionBUnwind" | "OPTION_B_UNWIND" | "option-b-unwind" => {
            Some(Treatment::OptionBUnwind)
        }
        _ => None,
    }
}

/// Run the full plan pipeline. Returns a summary; writes detailed artifacts to `inputs.out_dir`.
///
/// Artifacts written (file names):
///   - `nav_projection.csv`     — percentile aggregation per (year)
///   - `nav_mc_paths.csv`       — raw long-format paths
///   - `audit.json`             — McAuditBlock + report summary + concentrated symbol / cost-basis source
pub fn run_plan_pipeline(inputs: &PlanInputs) -> Result<PlanReport, PlanError> {
    use crate::portfolio::import::{read_positions_auto, roll_up_to_portfolio};
    use crate::portfolio::symbol_map::SymbolMap;
    use crate::portfolio::vesting::DistributionSchedule;
    use crate::projection::asset_class_library::AssetClassLibrary;
    use crate::projection::monte_carlo::{
        build_block_corr, percentiles_per_year, run_monte_carlo, CorrelationRules, McConfig,
    };
    use crate::projection::output::{
        build_mc_audit_block, build_nav_projection_rows, write_nav_mc_paths_csv,
        write_nav_projection_csv, NavMcPathRow,
    };
    use crate::projection::portfolio_nav::{
        project_path, resolve_concentrated_symbol, ProjectionConfig,
    };
    use std::fs;

    validate_scalars(inputs)?;

    // 1. Load positions (either format) and roll up to a Portfolio.
    let mut all_rows = Vec::new();
    for path in &inputs.positions_csvs {
        all_rows.extend(read_positions_auto(path)?);
    }
    validate_rows(&all_rows)?;
    let portfolio = roll_up_to_portfolio(&all_rows, inputs.as_of);
    let starting_nav = portfolio.total_market_value();

    // 2. Load vesting schedule (or empty if None).
    let vesting = match &inputs.vesting_csv {
        Some(vp) => DistributionSchedule::from_csv_file(vp)?,
        None => DistributionSchedule { events: vec![] },
    };

    // 3. Symbol map, concentrated symbol and Option B configuration.
    let mut symbol_map = match &inputs.symbol_map_toml {
        Some(p) => SymbolMap::from_toml_file(p)?,
        None => SymbolMap::builtin(),
    };
    if inputs.default_class.is_some() {
        symbol_map = symbol_map.with_default_class(inputs.default_class);
    }
    let explicit_symbol = inputs
        .symbol
        .as_deref()
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty());
    let symbol = resolve_concentrated_symbol(
        explicit_symbol.as_deref(),
        &vesting,
        &portfolio,
        &symbol_map,
    )
    .ok_or(PlanError::NoConcentratedSymbol)?;

    let (derived_basis, basis_source) = match inputs.option_b.cost_basis_per_share {
        Some(b) => (b, "override"),
        None => match weighted_cost_basis(&all_rows, &symbol) {
            Some(b) => (b, "positions"),
            None => (0.0, "none"),
        },
    };

    let mut config = ProjectionConfig::new(symbol.clone());
    config.symbol_map = symbol_map;
    config.option_b.cost_basis_per_share = derived_basis;
    if let Some(v) = inputs.option_b.ltcg_rate {
        config.option_b.ltcg_rate = v;
    }
    if let Some(v) = inputs.option_b.floor_pct {
        config.option_b.floor_pct = v;
    }
    if let Some(v) = &inputs.option_b.unwind_schedule {
        config.option_b.unwind_schedule = v.clone();
    }
    if let Some(v) = &inputs.option_b.reinvest_weights {
        config.option_b.reinvest_weights = v
            .iter()
            .map(|(t, w)| (t.trim().to_uppercase(), *w))
            .collect();
    }

    // 4. Load asset-class library.
    let library = if inputs.asset_classes_toml == Path::new(DEFAULT_ASSET_CLASSES_PATH)
        && !inputs.asset_classes_toml.exists()
    {
        AssetClassLibrary::from_toml_str(EMBEDDED_ASSET_CLASSES_TOML)?
    } else {
        AssetClassLibrary::from_toml_file(&inputs.asset_classes_toml)?
    };

    // 5. Build correlation matrix and run MC.
    let classes: Vec<AssetClass> = AssetClass::iter_all().collect();
    let corr = build_block_corr(&classes, &CorrelationRules::default());
    let mc_cfg = McConfig {
        n_paths: inputs.mc_paths,
        seed: inputs.mc_seed,
        horizon_years: inputs.horizon_years,
        ..McConfig::default()
    };
    let mc = run_monte_carlo(&library, &corr, &mc_cfg);

    // 6. Run project_path for each MC path.
    let mut all_states: Vec<Vec<crate::projection::portfolio_nav::PortfolioState>> =
        Vec::with_capacity(mc.paths.len());
    for path in &mc.paths {
        all_states.push(project_path(
            &portfolio,
            &vesting,
            inputs.treatment,
            path,
            &library,
            inputs.withhold_rate,
            &config,
        )?);
    }

    // 7. Aggregate to percentiles per year.
    let nav_pcts = percentiles_per_year(&all_states, |s| s.nav(), &[5, 25, 50, 75, 95]);
    let conc_pcts = percentiles_per_year(
        &all_states,
        |s| s.concentrated_pct_of_nav(),
        &[5, 25, 50, 75, 95],
    );

    // 8. Build as_of date labels and treatment key.
    let as_of_per_year: Vec<String> = (0..=inputs.horizon_years as usize)
        .map(|y| {
            inputs
                .as_of
                .checked_add_days(chrono::Days::new(365 * y as u64))
                .expect("date arithmetic")
                .to_string()
        })
        .collect();
    let treatment_label = format!("{:?}", inputs.treatment);
    let mut nav_pcts_map = BTreeMap::new();
    nav_pcts_map.insert(treatment_label.clone(), nav_pcts);
    let mut conc_pcts_map = BTreeMap::new();
    conc_pcts_map.insert(treatment_label.clone(), conc_pcts);

    // 9. Build CSV rows.
    let proj_rows = build_nav_projection_rows(
        std::slice::from_ref(&treatment_label),
        &nav_pcts_map,
        &conc_pcts_map,
        &as_of_per_year,
    );

    let raw_rows: Vec<NavMcPathRow> = mc
        .paths
        .iter()
        .enumerate()
        .flat_map(|(i, path)| {
            let states = &all_states[i];
            let tr_label = treatment_label.clone();
            let dates = as_of_per_year.clone();
            states
                .iter()
                .enumerate()
                .map(move |(y, s)| NavMcPathRow {
                    scenario: path.id.clone(),
                    rsu_treatment: tr_label.clone(),
                    year: y as u32,
                    as_of: dates[y].clone(),
                    nav: s.nav(),
                    concentrated_value: s.concentrated_value(),
                    concentrated_pct_of_nav: s.concentrated_pct_of_nav(),
                })
                .collect::<Vec<_>>()
        })
        .collect();

    // 10. Ensure out_dir exists, write artifacts.
    fs::create_dir_all(&inputs.out_dir)?;
    let nav_proj_path = inputs.out_dir.join("nav_projection.csv");
    let nav_paths_path = inputs.out_dir.join("nav_mc_paths.csv");
    let audit_path = inputs.out_dir.join("audit.json");

    write_nav_projection_csv(&nav_proj_path, &proj_rows)?;
    write_nav_mc_paths_csv(&nav_paths_path, &raw_rows)?;

    // 11. Build audit JSON.
    let last_year_idx = inputs.horizon_years as usize;
    let last_nav_pct = &nav_pcts_map[&treatment_label][last_year_idx];
    let p = |q: u8| {
        last_nav_pct
            .percentiles
            .iter()
            .find(|(qq, _)| *qq == q)
            .map(|(_, v)| *v)
            .unwrap_or(0.0)
    };

    let audit = build_mc_audit_block(
        inputs.mc_seed,
        inputs.mc_paths,
        mc.paths
            .first()
            .map(|path| path.id.clone())
            .unwrap_or_default(),
        inputs.horizon_years,
        std::slice::from_ref(&treatment_label),
        &nav_pcts_map,
        &conc_pcts_map,
    );
    let audit_json = serde_json::to_string_pretty(&serde_json::json!({
        "mc": audit,
        "concentrated_symbol": symbol,
        "cost_basis_source": basis_source,
        "cost_basis_per_share": derived_basis,
        "report_summary": {
            "treatment": treatment_label,
            "starting_nav": starting_nav,
            "year5_nav_p05": p(5),
            "year5_nav_p50": p(50),
            "year5_nav_p95": p(95),
        },
    }))?;
    fs::write(&audit_path, audit_json)?;

    Ok(PlanReport {
        treatment: inputs.treatment,
        seed: inputs.mc_seed,
        n_paths: inputs.mc_paths,
        starting_nav,
        year5_nav_p05: p(5),
        year5_nav_p50: p(50),
        year5_nav_p95: p(95),
        year5_nav_mean: last_nav_pct.mean,
        artifact_paths: vec![nav_proj_path, nav_paths_path, audit_path],
    })
}

fn invalid(msg: impl Into<String>) -> PlanError {
    PlanError::Invalid(msg.into())
}

fn check_unit_interval(name: &str, v: f64) -> Result<(), PlanError> {
    if v.is_finite() && (0.0..1.0).contains(&v) {
        Ok(())
    } else {
        Err(invalid(format!(
            "{name} must be a finite number in [0, 1), got {v}"
        )))
    }
}

/// Validate numeric knobs before any file is read.
fn validate_scalars(inputs: &PlanInputs) -> Result<(), PlanError> {
    if inputs.mc_paths == 0 {
        return Err(invalid("paths must be greater than 0"));
    }
    if inputs.horizon_years == 0 {
        return Err(invalid("horizon must be at least 1 year"));
    }
    check_unit_interval("withhold rate", inputs.withhold_rate)?;
    let ob = &inputs.option_b;
    if let Some(v) = ob.ltcg_rate {
        check_unit_interval("ltcg rate", v)?;
    }
    if let Some(v) = ob.floor_pct {
        check_unit_interval("floor pct", v)?;
    }
    if let Some(v) = ob.cost_basis_per_share {
        if !v.is_finite() || v < 0.0 {
            return Err(invalid(format!(
                "cost basis per share must be finite and >= 0, got {v}"
            )));
        }
    }
    if let Some(sched) = &ob.unwind_schedule {
        let mut years = std::collections::BTreeSet::new();
        for (year, shares) in sched {
            if !shares.is_finite() || *shares < 0.0 {
                return Err(invalid(format!(
                    "unwind schedule: shares for year {year} must be finite and >= 0, got {shares}"
                )));
            }
            if !years.insert(*year) {
                return Err(invalid(format!(
                    "unwind schedule lists year {year} more than once"
                )));
            }
        }
    }
    if let Some(weights) = &ob.reinvest_weights {
        if weights.iter().any(|(_, w)| !w.is_finite() || *w < 0.0) {
            return Err(invalid("reinvest weights must be finite and >= 0"));
        }
        let sum: f64 = weights.iter().map(|(_, w)| w).sum();
        if (sum - 1.0).abs() > 1e-6 {
            return Err(invalid(format!(
                "reinvest weights must sum to 1, got {sum}"
            )));
        }
    }
    Ok(())
}

/// Validate imported position rows.
fn validate_rows(rows: &[crate::portfolio::import::FidelityPositionRow]) -> Result<(), PlanError> {
    for r in rows {
        if !r.quantity.is_finite() || r.quantity < 0.0 {
            return Err(invalid(format!(
                "position {}: quantity must be finite and >= 0, got {} (short positions are not supported)",
                r.symbol, r.quantity
            )));
        }
        if !r.last_price.is_finite() || !r.current_value.is_finite() {
            return Err(invalid(format!(
                "position {}: price/value must be finite, got price {} value {}",
                r.symbol, r.last_price, r.current_value
            )));
        }
    }
    Ok(())
}

/// Share-weighted average cost basis of `symbol` across rows that report one.
fn weighted_cost_basis(
    rows: &[crate::portfolio::import::FidelityPositionRow],
    symbol: &str,
) -> Option<f64> {
    let (mut cost, mut qty) = (0.0, 0.0);
    for r in rows
        .iter()
        .filter(|r| r.symbol == symbol && r.quantity > 0.0)
    {
        if let Some(total) = r.cost_basis_total {
            cost += total;
            qty += r.quantity;
        }
    }
    (qty > 0.0).then(|| cost / qty)
}

// ---------------------------------------------------------------------------
// Data directory
// ---------------------------------------------------------------------------

/// Files discovered in a plan data directory.
#[derive(Debug, Clone, PartialEq)]
pub struct DataDir {
    pub positions: Vec<PathBuf>,
    pub vesting: Option<PathBuf>,
    pub symbol_map: Option<PathBuf>,
    pub plan_toml: Option<PathBuf>,
}

/// Discover plan inputs under `dir`:
/// `positions/*.csv` (required, sorted), and optional `vesting.csv`,
/// `symbol_map.toml`, `plan.toml`.
pub fn discover_data_dir(dir: &Path) -> Result<DataDir, PlanError> {
    if !dir.is_dir() {
        return Err(PlanError::DataDir(format!(
            "{} is not a directory (create it with `ghost-alpha init-data`)",
            dir.display()
        )));
    }
    let pos_dir = dir.join("positions");
    let no_positions = || {
        PlanError::DataDir(format!(
            "no positions CSV files found in {} (expected positions/*.csv)",
            pos_dir.display()
        ))
    };
    if !pos_dir.is_dir() {
        return Err(no_positions());
    }
    let mut positions: Vec<PathBuf> = std::fs::read_dir(&pos_dir)?
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
        })
        .collect();
    if positions.is_empty() {
        return Err(no_positions());
    }
    positions.sort();
    let optional = |name: &str| {
        let p = dir.join(name);
        p.is_file().then_some(p)
    };
    Ok(DataDir {
        positions,
        vesting: optional("vesting.csv"),
        symbol_map: optional("symbol_map.toml"),
        plan_toml: optional("plan.toml"),
    })
}

/// `[unwind]` table of `plan.toml`.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnwindToml {
    pub ltcg_rate: Option<f64>,
    pub floor_pct: Option<f64>,
    pub cost_basis_per_share: Option<f64>,
    /// `{"1" = 400, "2" = 300}`: year -> shares.
    pub schedule: Option<BTreeMap<String, f64>>,
    /// `{VOO = 0.5, IEF = 0.5}`: ticker -> weight (quote tickers containing dots).
    pub reinvest_weights: Option<BTreeMap<String, f64>>,
}

/// Run settings read from `plan.toml`. CLI flags take precedence over these.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanToml {
    pub as_of: Option<NaiveDate>,
    pub symbol: Option<String>,
    /// `hold | sell-reinvest | option-b`
    pub treatment: Option<String>,
    pub withhold_rate: Option<f64>,
    pub horizon_years: Option<u32>,
    pub paths: Option<usize>,
    pub seed: Option<u64>,
    /// snake_case asset class used for unmapped tickers.
    pub default_class: Option<String>,
    #[serde(default)]
    pub unwind: UnwindToml,
}

impl PlanToml {
    /// `unwind.schedule` as sorted `(year, shares)` pairs.
    pub fn unwind_schedule(&self) -> Result<Option<Vec<(u32, f64)>>, PlanError> {
        let Some(map) = &self.unwind.schedule else {
            return Ok(None);
        };
        let mut out = Vec::with_capacity(map.len());
        for (k, v) in map {
            let year = k.trim().parse::<u32>().map_err(|_| {
                PlanError::PlanToml(format!("unwind.schedule key '{k}' is not a year number"))
            })?;
            out.push((year, *v));
        }
        out.sort_by_key(|(y, _)| *y);
        Ok(Some(out))
    }

    /// `unwind.reinvest_weights` as `(ticker, weight)` pairs.
    pub fn reinvest_weights(&self) -> Option<Vec<(String, f64)>> {
        self.unwind.reinvest_weights.as_ref().map(|m| {
            let mut out: Vec<(String, f64)> = Vec::new();
            for (k, v) in m {
                let key = k.trim().to_uppercase();
                match out.iter_mut().find(|(t, _)| *t == key) {
                    Some((_, w)) => *w += *v,
                    None => out.push((key, *v)),
                }
            }
            out
        })
    }

    /// `default_class` parsed into an [`AssetClass`].
    pub fn default_class(&self) -> Result<Option<AssetClass>, PlanError> {
        match &self.default_class {
            None => Ok(None),
            Some(name) => parse_asset_class(name)
                .map(Some)
                .ok_or_else(|| PlanError::PlanToml(format!("unknown default_class '{name}'"))),
        }
    }

    /// `treatment` parsed into a [`Treatment`].
    pub fn treatment(&self) -> Result<Option<Treatment>, PlanError> {
        match &self.treatment {
            None => Ok(None),
            Some(name) => parse_treatment(name).map(Some).ok_or_else(|| {
                PlanError::PlanToml(format!(
                    "unknown treatment '{name}' (valid: hold | sell-reinvest | option-b)"
                ))
            }),
        }
    }
}

/// Read and validate a `plan.toml`.
pub fn load_plan_toml(p: &Path) -> Result<PlanToml, PlanError> {
    let text = std::fs::read_to_string(p)?;
    let mut pt: PlanToml =
        toml::from_str(&text).map_err(|e| PlanError::PlanToml(format!("{}: {e}", p.display())))?;
    pt.symbol = pt
        .symbol
        .map(|s| s.trim().to_uppercase())
        .filter(|s| !s.is_empty());
    // Fail early on values that cannot be interpreted.
    pt.unwind_schedule()?;
    pt.default_class()?;
    pt.treatment()?;
    Ok(pt)
}

// ---------------------------------------------------------------------------
// Templates (`ghost-alpha init-data`)
// ---------------------------------------------------------------------------

const TEMPLATE_POSITIONS: &str = "\
# Positions CSV, simple format. One file per account; drop more files in this folder.
# Required columns: symbol, quantity, price.  Optional: cost_basis (per share), account, description.
# A Fidelity Portfolio_Positions export also works here unchanged.
symbol,quantity,price,cost_basis,account,description
MSFT,400,420.00,185.00,BROKERAGE,Example concentrated position
VOO,250,600.00,420.00,BROKERAGE,Example S&P 500 index fund
VXUS,500,70.00,58.00,BROKERAGE,Example international index fund
IEF,600,95.00,98.00,BROKERAGE,Example intermediate treasury fund
SPAXX,25000,1.00,1.00,BROKERAGE,Example money market
";

const TEMPLATE_VESTING: &str = "\
symbol,vesting_date,quantity
MSFT,2026-08-15,40
MSFT,2026-11-15,40
MSFT,2027-02-15,40
MSFT,2027-05-15,40
MSFT,2027-08-15,40
MSFT,2027-11-15,40
";

const TEMPLATE_SYMBOL_MAP: &str = "\
# Map tickers that are not in the built-in list to an asset class.
# Class names are the snake_case keys of config/asset_classes.toml
# (e.g. us_large_cap_index, us_total_market, tech_mega_cap, cash_mmf, gold).
#
# Optional fallback class for any ticker that is not mapped:
# default_class = \"us_large_cap_index\"

[symbols]
# FSKAX = \"us_total_market\"
# \"BRK.B\" = \"us_large_cap_index\"   # tickers containing a dot must be quoted
";

const TEMPLATE_PLAN_TOML: &str = "\
# Run settings for `ghost-alpha plan --data-dir <this folder>`.
# Command-line flags override anything set here; anything unset falls back to a built-in default.

# Snapshot date of the positions (YYYY-MM-DD). Required here or via --as-of.
# as_of = \"2026-05-01\"

# Concentrated position. Default: the symbol with the most vesting shares,
# else the largest non-cash position.
# symbol = \"MSFT\"

# hold | sell-reinvest | option-b   (default: option-b)
# treatment = \"option-b\"

# Employer sell-to-cover withholding on vests (default 0.22).
# withhold_rate = 0.22
# horizon_years = 5
# paths = 5000
# seed = 42

# Asset class for tickers missing from the symbol map.
# default_class = \"us_large_cap_index\"

# Option B (concentration unwind) parameters. Defaults: LTCG 0.238, floor 0.25,
# cost basis taken from the positions file (else 0.0), empty schedule.
[unwind]
# ltcg_rate = 0.238
# floor_pct = 0.25
# cost_basis_per_share = 185.0

# [unwind.schedule]   # existing shares to sell: year = shares
# \"1\" = 100
# \"2\" = 100

# [unwind.reinvest_weights]   # where after-tax proceeds go (quote tickers with dots)
# VOO = 0.6
# IEF = 0.4
";

const TEMPLATE_README: &str = "\
# GhostAlpha plan data folder

    positions/*.csv   one CSV per account (required)
    vesting.csv       optional RSU vesting schedule: symbol,vesting_date,quantity
    symbol_map.toml   optional ticker -> asset class overrides
    plan.toml         optional run settings (CLI flags override it)

Run:

    ghost-alpha plan --data-dir . --as-of 2026-05-01 --out-dir plan_output

Run it from inside this folder (as above) or pass the folder path to --data-dir
from anywhere. Results (nav_projection.csv, nav_mc_paths.csv, audit.json) are
written to ./plan_output.

The files here are examples with made-up numbers. Replace them with your own.
See docs/PLAN_PIPELINE.md in the GhostAlpha repository for the full reference.
";

/// Write the example data folder (`positions/example_positions.csv`, `vesting.csv`,
/// `symbol_map.toml`, `plan.toml`, `README.md`) into `dir`.
///
/// Every target is checked first: if any exists and `force` is false, nothing is
/// written and an error is returned.
pub fn write_data_templates(dir: &Path, force: bool) -> Result<Vec<PathBuf>, PlanError> {
    let files: [(&str, &str); 5] = [
        ("positions/example_positions.csv", TEMPLATE_POSITIONS),
        ("vesting.csv", TEMPLATE_VESTING),
        ("symbol_map.toml", TEMPLATE_SYMBOL_MAP),
        ("plan.toml", TEMPLATE_PLAN_TOML),
        ("README.md", TEMPLATE_README),
    ];
    let targets: Vec<(PathBuf, &str)> = files.iter().map(|(rel, c)| (dir.join(rel), *c)).collect();
    if !force {
        let existing: Vec<String> = targets
            .iter()
            .filter(|(p, _)| p.exists())
            .map(|(p, _)| p.display().to_string())
            .collect();
        if !existing.is_empty() {
            return Err(PlanError::DataDir(format!(
                "refusing to overwrite existing file(s): {} (use --force)",
                existing.join(", ")
            )));
        }
    }
    let mut written = Vec::with_capacity(targets.len());
    for (path, content) in targets {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, content)?;
        written.push(path);
    }
    Ok(written)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::import::read_positions_auto;
    use crate::portfolio::import::{read_fidelity_positions_csv, roll_up_to_portfolio};
    use crate::portfolio::symbol_map::SymbolMap;
    use crate::projection::monte_carlo::AssetClass;
    use crate::projection::portfolio_nav::ProjectionError;
    use std::collections::HashSet;
    use std::path::Path;
    use tempfile::TempDir;

    // ── Fixture paths ────────────────────────────────────────────────────────

    fn fixture(relative: &str) -> PathBuf {
        // Tests run from the workspace root; fixtures live under tests/fixtures/.
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
        Path::new(&manifest).join(relative)
    }

    /// Synthetic Fidelity export, account EX0000001 (MSFT is the concentrated position).
    fn fidelity_main() -> PathBuf {
        fixture("tests/fixtures/fidelity/example_main.csv")
    }

    /// Synthetic Fidelity export, account EX0000002.
    fn fidelity_small() -> PathBuf {
        fixture("tests/fixtures/fidelity/example_small.csv")
    }

    /// Combined NAV of the two example accounts.
    const EXAMPLE_NAV: f64 = 1_043_770.00;

    fn vesting_sample() -> PathBuf {
        fixture("tests/fixtures/vesting/sample.csv")
    }

    fn asset_classes_toml() -> PathBuf {
        fixture("config/asset_classes.toml")
    }

    fn as_of() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()
    }

    /// Build a canonical PlanInputs with both example Fidelity CSVs, sample
    /// vesting, and the committed asset_classes.toml. The concentrated symbol is
    /// left to auto-resolution (→ MSFT).
    fn canonical_inputs(out_dir: &TempDir) -> PlanInputs {
        PlanInputs {
            positions_csvs: vec![fidelity_main(), fidelity_small()],
            vesting_csv: Some(vesting_sample()),
            asset_classes_toml: asset_classes_toml(),
            treatment: Treatment::Hold,
            mc_paths: 100,
            mc_seed: 42,
            horizon_years: 5,
            withhold_rate: 0.22,
            as_of: as_of(),
            out_dir: out_dir.path().to_path_buf(),
            symbol: None,
            symbol_map_toml: None,
            default_class: None,
            option_b: OptionBOverrides::default(),
        }
    }

    // ── Smoke tests (3) ──────────────────────────────────────────────────────

    #[test]
    fn run_plan_pipeline_with_example_fidelity_fixtures_returns_ok() {
        let tmp = TempDir::new().unwrap();
        let inputs = canonical_inputs(&tmp);
        let result = run_plan_pipeline(&inputs);
        assert!(
            result.is_ok(),
            "pipeline returned Err: {:?}",
            result.unwrap_err()
        );
    }

    #[test]
    fn run_plan_pipeline_writes_three_artifacts_to_out_dir() {
        let tmp = TempDir::new().unwrap();
        let inputs = canonical_inputs(&tmp);
        let report = run_plan_pipeline(&inputs).expect("pipeline must succeed");

        assert_eq!(
            report.artifact_paths.len(),
            3,
            "expected exactly 3 artifact paths, got {}",
            report.artifact_paths.len()
        );
        for path in &report.artifact_paths {
            assert!(
                path.exists(),
                "artifact path does not exist on disk: {}",
                path.display()
            );
        }
    }

    #[test]
    fn run_plan_pipeline_artifact_paths_match_documented_filenames() {
        let tmp = TempDir::new().unwrap();
        let inputs = canonical_inputs(&tmp);
        let report = run_plan_pipeline(&inputs).expect("pipeline must succeed");

        let filenames: HashSet<&str> = report
            .artifact_paths
            .iter()
            .map(|p| p.file_name().and_then(|n| n.to_str()).unwrap_or(""))
            .collect();

        let expected: HashSet<&str> = ["nav_projection.csv", "nav_mc_paths.csv", "audit.json"]
            .iter()
            .copied()
            .collect();

        assert_eq!(
            filenames, expected,
            "artifact filenames mismatch — got {:?}, expected {:?}",
            filenames, expected
        );
    }

    // ── Report field tests (4) ───────────────────────────────────────────────

    #[test]
    fn report_starting_nav_matches_portfolio_total() {
        let tmp = TempDir::new().unwrap();
        let inputs = canonical_inputs(&tmp);
        let report = run_plan_pipeline(&inputs).expect("pipeline must succeed");

        // Independently compute expected NAV from the same CSV files.
        let mut all_rows = Vec::new();
        for csv_path in &inputs.positions_csvs {
            let rows = read_fidelity_positions_csv(csv_path).expect("fixture CSV must be readable");
            all_rows.extend(rows);
        }
        let portfolio = roll_up_to_portfolio(&all_rows, inputs.as_of);
        let expected_nav = portfolio.total_market_value();
        assert!(
            (expected_nav - EXAMPLE_NAV).abs() < 0.01,
            "example fixtures must total {EXAMPLE_NAV}, got {expected_nav}"
        );

        assert!(
            (report.starting_nav - expected_nav).abs() < 0.01,
            "starting_nav {} differs from portfolio.total_market_value() {} by more than $0.01",
            report.starting_nav,
            expected_nav
        );
    }

    #[test]
    fn report_year5_percentiles_in_sane_order() {
        let tmp = TempDir::new().unwrap();
        let inputs = canonical_inputs(&tmp);
        let report = run_plan_pipeline(&inputs).expect("pipeline must succeed");

        assert!(
            report.year5_nav_p05 < report.year5_nav_p50,
            "p05 ({}) must be less than p50 ({})",
            report.year5_nav_p05,
            report.year5_nav_p50
        );
        assert!(
            report.year5_nav_p50 < report.year5_nav_p95,
            "p50 ({}) must be less than p95 ({})",
            report.year5_nav_p50,
            report.year5_nav_p95
        );
    }

    #[test]
    fn report_records_seed_and_n_paths() {
        let tmp = TempDir::new().unwrap();
        let inputs = canonical_inputs(&tmp);
        let report = run_plan_pipeline(&inputs).expect("pipeline must succeed");

        assert_eq!(
            report.seed, inputs.mc_seed,
            "report.seed {} != inputs.mc_seed {}",
            report.seed, inputs.mc_seed
        );
        assert_eq!(
            report.n_paths, inputs.mc_paths,
            "report.n_paths {} != inputs.mc_paths {}",
            report.n_paths, inputs.mc_paths
        );
    }

    #[test]
    fn report_records_treatment() {
        let tmp = TempDir::new().unwrap();
        let inputs = canonical_inputs(&tmp);
        let report = run_plan_pipeline(&inputs).expect("pipeline must succeed");

        assert_eq!(
            report.treatment, inputs.treatment,
            "report.treatment {:?} != inputs.treatment {:?}",
            report.treatment, inputs.treatment
        );
    }

    // ── Treatment tests (3) ──────────────────────────────────────────────────

    #[test]
    fn pipeline_works_for_hold_treatment() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = canonical_inputs(&tmp);
        inputs.treatment = Treatment::Hold;
        let result = run_plan_pipeline(&inputs);
        assert!(
            result.is_ok(),
            "Treatment::Hold pipeline returned Err: {:?}",
            result.unwrap_err()
        );
    }

    #[test]
    fn pipeline_works_for_sell_reinvest_treatment() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = canonical_inputs(&tmp);
        inputs.treatment = Treatment::SellReinvest;
        let result = run_plan_pipeline(&inputs);
        assert!(
            result.is_ok(),
            "Treatment::SellReinvest pipeline returned Err: {:?}",
            result.unwrap_err()
        );
    }

    #[test]
    fn pipeline_works_for_option_b_unwind_treatment() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = canonical_inputs(&tmp);
        inputs.treatment = Treatment::OptionBUnwind;
        let result = run_plan_pipeline(&inputs);
        assert!(
            result.is_ok(),
            "Treatment::OptionBUnwind pipeline returned Err: {:?}. \
             If this is NotImplementedYet, Gap 4 branch is incomplete.",
            result.unwrap_err()
        );
    }

    // ── Reproducibility test (1) ─────────────────────────────────────────────

    #[test]
    fn pipeline_with_same_seed_returns_byte_identical_csvs() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();

        let mut inputs1 = canonical_inputs(&tmp1);
        inputs1.mc_paths = 50;
        inputs1.mc_seed = 42;

        let mut inputs2 = canonical_inputs(&tmp2);
        inputs2.mc_paths = 50;
        inputs2.mc_seed = 42;

        run_plan_pipeline(&inputs1).expect("first run must succeed");
        run_plan_pipeline(&inputs2).expect("second run must succeed");

        let csv1 = std::fs::read(tmp1.path().join("nav_projection.csv"))
            .expect("nav_projection.csv must exist after first run");
        let csv2 = std::fs::read(tmp2.path().join("nav_projection.csv"))
            .expect("nav_projection.csv must exist after second run");

        assert_eq!(
            csv1, csv2,
            "nav_projection.csv bytes differ between two runs with identical seed — \
             seed-pinning is broken"
        );
    }

    // ── Error path tests (3) ─────────────────────────────────────────────────

    #[test]
    fn pipeline_returns_err_when_positions_csv_missing() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = canonical_inputs(&tmp);
        inputs.positions_csvs = vec![PathBuf::from("/nonexistent/does_not_exist.csv")];

        let result = run_plan_pipeline(&inputs);
        assert!(
            result.is_err(),
            "expected Err for missing positions CSV but got Ok"
        );
        // The error must be an I/O variant.
        match result.unwrap_err() {
            PlanError::Io(_) | PlanError::Fidelity(_) => { /* expected */ }
            other => panic!(
                "expected PlanError::Io or PlanError::Fidelity for missing CSV, got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn pipeline_returns_err_when_asset_classes_toml_missing() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = canonical_inputs(&tmp);
        inputs.asset_classes_toml = PathBuf::from("/nonexistent/asset_classes.toml");

        let result = run_plan_pipeline(&inputs);
        assert!(
            result.is_err(),
            "expected Err for missing asset_classes.toml but got Ok"
        );
        match result.unwrap_err() {
            PlanError::Library(_) | PlanError::Io(_) => { /* expected */ }
            other => panic!(
                "expected PlanError::Library or PlanError::Io for missing TOML, got: {:?}",
                other
            ),
        }
    }

    #[test]
    fn pipeline_with_no_vesting_works_with_none() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = canonical_inputs(&tmp);
        inputs.vesting_csv = None;

        let result = run_plan_pipeline(&inputs);
        assert!(
            result.is_ok(),
            "pipeline with vesting_csv=None returned Err: {:?}",
            result.unwrap_err()
        );
        // starting_nav must still be positive (example portfolio has positions).
        let report = result.unwrap();
        assert!(
            report.starting_nav > 0.0,
            "starting_nav must be > 0 even with no vesting schedule"
        );
    }

    // ═════════════════════════════════════════════════════════════════════════
    // Spec §5: generalized pipeline (symbol, symbol map, overrides, audit keys)
    // ═════════════════════════════════════════════════════════════════════════

    fn write_file(root: &Path, rel: &str, content: &str) -> PathBuf {
        let p = root.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, content).unwrap();
        p
    }

    /// Depth-first search for `key` anywhere in a JSON document.
    /// (The spec does not pin whether the audit keys are top-level or nested.)
    fn find_key<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
        match v {
            serde_json::Value::Object(m) => {
                if let Some(x) = m.get(key) {
                    return Some(x);
                }
                m.values().find_map(|c| find_key(c, key))
            }
            serde_json::Value::Array(a) => a.iter().find_map(|c| find_key(c, key)),
            _ => None,
        }
    }

    fn read_audit(out: &Path) -> serde_json::Value {
        let s = std::fs::read_to_string(out.join("audit.json")).expect("audit.json exists");
        serde_json::from_str(&s).expect("audit.json is valid JSON")
    }

    /// Simple-format positions with per-share cost basis for every row.
    const SIMPLE_WITH_BASIS: &str = "symbol,quantity,price,cost_basis\n\
        MSFT,400,420,185\nVOO,100,600,420\nIEF,500,95,98\nSPAXX,20000,1,1\n";

    /// Same holdings, no cost_basis column at all.
    const SIMPLE_NO_BASIS: &str = "symbol,quantity,price\n\
        MSFT,400,420\nVOO,100,600\nIEF,500,95\nSPAXX,20000,1\n";

    fn simple_inputs(tmp: &TempDir, positions_csv: &str) -> PlanInputs {
        let pos = write_file(tmp.path(), "in/pos.csv", positions_csv);
        PlanInputs {
            positions_csvs: vec![pos],
            vesting_csv: None,
            asset_classes_toml: asset_classes_toml(),
            treatment: Treatment::OptionBUnwind,
            mc_paths: 40,
            mc_seed: 11,
            horizon_years: 3,
            withhold_rate: 0.22,
            as_of: as_of(),
            out_dir: tmp.path().join("out"),
            symbol: Some("MSFT".to_string()),
            symbol_map_toml: None,
            default_class: None,
            option_b: OptionBOverrides {
                unwind_schedule: Some(vec![(1, 100.0)]),
                ..OptionBOverrides::default()
            },
        }
    }

    // ── Auto-detect through the pipeline ──────────────────────────────────

    #[test]
    fn pipeline_accepts_simple_format_positions() {
        let tmp = TempDir::new().unwrap();
        let inputs = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        let report = run_plan_pipeline(&inputs).expect("simple-format positions must work");
        let expected = 400.0 * 420.0 + 100.0 * 600.0 + 500.0 * 95.0 + 20_000.0;
        assert!(
            (report.starting_nav - expected).abs() < 0.01,
            "{}",
            report.starting_nav
        );
    }

    #[test]
    fn pipeline_accepts_mixed_fidelity_and_simple_position_files() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = simple_inputs(&tmp, "symbol,quantity,price\nVOO,10,600\n");
        inputs.positions_csvs.push(fidelity_main());
        let report = run_plan_pipeline(&inputs).expect("mixed formats must work");
        assert!(
            (report.starting_nav - (6_000.0 + 948_250.0)).abs() < 0.01,
            "NAV must be simple (6,000) + example_main (948,250); got {}",
            report.starting_nav
        );
    }

    // ── Concentrated symbol: explicit vs auto-resolved ────────────────────

    #[test]
    fn auto_resolved_symbol_gives_same_report_as_explicit_msft() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        let mut auto = canonical_inputs(&tmp1);
        auto.mc_paths = 40;
        let mut explicit = canonical_inputs(&tmp2);
        explicit.mc_paths = 40;
        explicit.symbol = Some("MSFT".to_string());
        let a = run_plan_pipeline(&auto).unwrap();
        let b = run_plan_pipeline(&explicit).unwrap();
        assert_eq!(a.year5_nav_p50, b.year5_nav_p50);
        assert_eq!(a.year5_nav_mean, b.year5_nav_mean);
        let audit = read_audit(tmp1.path());
        assert_eq!(
            find_key(&audit, "concentrated_symbol").and_then(|v| v.as_str()),
            Some("MSFT")
        );
    }

    #[test]
    fn audit_json_records_concentrated_symbol() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = canonical_inputs(&tmp);
        inputs.symbol = Some("MSFT".to_string());
        run_plan_pipeline(&inputs).unwrap();
        let audit = read_audit(tmp.path());
        let v = find_key(&audit, "concentrated_symbol")
            .expect("audit.json must record concentrated_symbol");
        assert_eq!(v.as_str(), Some("MSFT"));
    }

    #[test]
    fn explicit_symbol_other_than_vesting_symbol_is_honoured_and_recorded() {
        // --symbol AAPL with the MSFT vesting file: the MSFT vests must NOT be
        // treated as AAPL vests, so the result equals AAPL with no vesting at all.
        let t_with = TempDir::new().unwrap();
        let mut with_vesting = canonical_inputs(&t_with);
        with_vesting.symbol = Some("AAPL".to_string()); // example_main holds AAPL 200 sh
        with_vesting.mc_paths = 40;
        let r_with = run_plan_pipeline(&with_vesting).expect("AAPL is in the portfolio");
        let audit = read_audit(t_with.path());
        assert_eq!(
            find_key(&audit, "concentrated_symbol").and_then(|v| v.as_str()),
            Some("AAPL")
        );

        let t_none = TempDir::new().unwrap();
        let mut no_vesting = canonical_inputs(&t_none);
        no_vesting.symbol = Some("AAPL".to_string());
        no_vesting.vesting_csv = None;
        no_vesting.mc_paths = 40;
        let r_none = run_plan_pipeline(&no_vesting).unwrap();

        assert_eq!(r_with.starting_nav, r_none.starting_nav);
        assert!(
            (r_with.year5_nav_mean - r_none.year5_nav_mean).abs() < 1e-6,
            "MSFT vests leaked into the AAPL projection: {} vs {}",
            r_with.year5_nav_mean,
            r_none.year5_nav_mean
        );
        assert!((r_with.year5_nav_p50 - r_none.year5_nav_p50).abs() < 1e-6);
    }

    #[test]
    fn explicit_symbol_missing_from_portfolio_with_vesting_is_an_error() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = canonical_inputs(&tmp);
        inputs.symbol = Some("TSLA".to_string());
        let err = run_plan_pipeline(&inputs).expect_err("TSLA not held, vesting non-empty");
        assert!(
            matches!(err, PlanError::Projection(ProjectionError::MissingConcentrated(ref s)) if s == "TSLA"),
            "{err:?}"
        );
    }

    // ── cost_basis_source / derivation ────────────────────────────────────

    fn cost_basis_source(out: &Path) -> String {
        find_key(&read_audit(out), "cost_basis_source")
            .and_then(|v| v.as_str())
            .expect("audit.json must record cost_basis_source")
            .to_string()
    }

    #[test]
    fn cost_basis_source_is_positions_when_derived_from_rows() {
        let tmp = TempDir::new().unwrap();
        let inputs = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        run_plan_pipeline(&inputs).unwrap();
        assert_eq!(cost_basis_source(&tmp.path().join("out")), "positions");
    }

    #[test]
    fn cost_basis_source_is_override_when_explicit_value_given() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        inputs.option_b.cost_basis_per_share = Some(150.0);
        run_plan_pipeline(&inputs).unwrap();
        assert_eq!(cost_basis_source(&tmp.path().join("out")), "override");
    }

    #[test]
    fn cost_basis_source_is_none_when_no_basis_available() {
        let tmp = TempDir::new().unwrap();
        let inputs = simple_inputs(&tmp, SIMPLE_NO_BASIS);
        run_plan_pipeline(&inputs).unwrap();
        assert_eq!(cost_basis_source(&tmp.path().join("out")), "none");
    }

    #[test]
    fn explicit_zero_cost_basis_is_still_an_override() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        inputs.option_b.cost_basis_per_share = Some(0.0);
        run_plan_pipeline(&inputs).unwrap();
        assert_eq!(cost_basis_source(&tmp.path().join("out")), "override");
    }

    /// MSFT rows in two files: 100 sh @ basis 100 and 300 sh @ basis 200
    /// → share-weighted average = (100×100 + 300×200) / 400 = 175.
    fn two_account_inputs(tmp: &TempDir) -> PlanInputs {
        let a = write_file(
            tmp.path(),
            "in/a.csv",
            "symbol,quantity,price,cost_basis\nMSFT,100,420,100\nVOO,100,600,420\n",
        );
        let b = write_file(
            tmp.path(),
            "in/b.csv",
            "symbol,quantity,price,cost_basis\nMSFT,300,420,200\nIEF,500,95,98\n",
        );
        let mut inputs = simple_inputs(tmp, SIMPLE_WITH_BASIS);
        inputs.positions_csvs = vec![a, b];
        inputs
    }

    #[test]
    fn derived_cost_basis_is_share_weighted_average_across_rows() {
        // derived (no override) must equal an explicit override of 175 …
        let t_derived = TempDir::new().unwrap();
        let derived = two_account_inputs(&t_derived);
        let r_derived = run_plan_pipeline(&derived).unwrap();

        let t_175 = TempDir::new().unwrap();
        let mut o175 = two_account_inputs(&t_175);
        o175.option_b.cost_basis_per_share = Some(175.0);
        let r_175 = run_plan_pipeline(&o175).unwrap();

        // … and differ from the simple (unweighted) mean of 150 and from 0.
        let t_150 = TempDir::new().unwrap();
        let mut o150 = two_account_inputs(&t_150);
        o150.option_b.cost_basis_per_share = Some(150.0);
        let r_150 = run_plan_pipeline(&o150).unwrap();

        assert!(
            (r_derived.year5_nav_mean - r_175.year5_nav_mean).abs() < 1e-6,
            "derived {} vs override-175 {}",
            r_derived.year5_nav_mean,
            r_175.year5_nav_mean
        );
        assert!(
            (r_derived.year5_nav_mean - r_150.year5_nav_mean).abs() > 1e-3,
            "derived basis must not be the unweighted mean (150)"
        );
    }

    #[test]
    fn cost_basis_override_changes_unwind_tax_and_outcome() {
        let t0 = TempDir::new().unwrap();
        let mut zero = simple_inputs(&t0, SIMPLE_WITH_BASIS);
        zero.option_b.cost_basis_per_share = Some(0.0);
        let r_zero = run_plan_pipeline(&zero).unwrap();

        let t1 = TempDir::new().unwrap();
        let mut high = simple_inputs(&t1, SIMPLE_WITH_BASIS);
        high.option_b.cost_basis_per_share = Some(400.0);
        let r_high = run_plan_pipeline(&high).unwrap();

        assert!(
            r_high.year5_nav_mean > r_zero.year5_nav_mean + 1.0,
            "higher basis ⇒ less tax ⇒ higher NAV: {} vs {}",
            r_high.year5_nav_mean,
            r_zero.year5_nav_mean
        );
    }

    // ── Option-B overrides ────────────────────────────────────────────────

    #[test]
    fn unwind_schedule_override_changes_option_b_outcome_vs_default_empty_schedule() {
        let t_default = TempDir::new().unwrap();
        let mut none = simple_inputs(&t_default, SIMPLE_WITH_BASIS);
        none.option_b.unwind_schedule = None;
        let r_none = run_plan_pipeline(&none).unwrap();

        let t_sched = TempDir::new().unwrap();
        let sched = simple_inputs(&t_sched, SIMPLE_WITH_BASIS); // year-1: 100 shares
        let r_sched = run_plan_pipeline(&sched).unwrap();

        assert!(
            (r_none.year5_nav_mean - r_sched.year5_nav_mean).abs() > 1e-3,
            "a scheduled unwind must change the outcome"
        );
    }

    #[test]
    fn ltcg_rate_floor_and_reinvest_overrides_are_accepted() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        inputs.option_b.ltcg_rate = Some(0.15);
        inputs.option_b.floor_pct = Some(0.30);
        inputs.option_b.reinvest_weights =
            Some(vec![("VOO".to_string(), 0.7), ("IEF".to_string(), 0.3)]);
        run_plan_pipeline(&inputs).expect("overrides must be accepted");
    }

    #[test]
    fn hold_treatment_ignores_option_b_overrides() {
        let t1 = TempDir::new().unwrap();
        let mut a = simple_inputs(&t1, SIMPLE_WITH_BASIS);
        a.treatment = Treatment::Hold;
        a.option_b = OptionBOverrides::default();
        let t2 = TempDir::new().unwrap();
        let mut b = simple_inputs(&t2, SIMPLE_WITH_BASIS);
        b.treatment = Treatment::Hold;
        b.option_b.cost_basis_per_share = Some(1.0);
        b.option_b.ltcg_rate = Some(0.5);
        let ra = run_plan_pipeline(&a).unwrap();
        let rb = run_plan_pipeline(&b).unwrap();
        assert_eq!(ra.year5_nav_mean, rb.year5_nav_mean);
    }

    // ── Symbol map / default class ────────────────────────────────────────

    const WITH_UNMAPPED: &str = "symbol,quantity,price\n\
        MSFT,400,420\nZZZQ,100,300\nQQQZ,50,250\nSPAXX,20000,1\n";

    #[test]
    fn unmapped_tickers_fail_with_all_of_them_listed_and_toml_hint() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = simple_inputs(&tmp, WITH_UNMAPPED);
        inputs.treatment = Treatment::Hold;
        let err = run_plan_pipeline(&inputs).expect_err("ZZZQ/QQQZ are not in the builtin map");
        assert!(
            matches!(
                err,
                PlanError::Projection(ProjectionError::UnknownSymbol(_))
            ),
            "{err:?}"
        );
        let msg = err.to_string();
        assert!(msg.contains("ZZZQ") && msg.contains("QQQZ"), "{msg}");
        assert!(msg.contains("symbol_map.toml"), "{msg}");
    }

    #[test]
    fn symbol_map_toml_resolves_unmapped_tickers() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = simple_inputs(&tmp, WITH_UNMAPPED);
        inputs.treatment = Treatment::Hold;
        let map = write_file(
            tmp.path(),
            "in/symbol_map.toml",
            "[symbols]\nZZZQ = \"industrial\"\nQQQZ = \"aerospace_defense\"\n",
        );
        inputs.symbol_map_toml = Some(map);
        run_plan_pipeline(&inputs).expect("symbol map must resolve ZZZQ/QQQZ");
    }

    #[test]
    fn default_class_input_resolves_unmapped_tickers() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = simple_inputs(&tmp, WITH_UNMAPPED);
        inputs.treatment = Treatment::Hold;
        inputs.default_class = Some(AssetClass::UsLargeCapIndex);
        run_plan_pipeline(&inputs).expect("default_class must resolve ZZZQ/QQQZ");
    }

    #[test]
    fn default_class_input_overrides_default_class_in_symbol_map_toml() {
        // TOML says "gold"; the explicit input (CLI-level) must win. Both resolve,
        // so assert via the outcome difference between the two classes.
        let t1 = TempDir::new().unwrap();
        let mut a = simple_inputs(&t1, WITH_UNMAPPED);
        a.treatment = Treatment::Hold;
        a.symbol_map_toml = Some(write_file(
            t1.path(),
            "in/m.toml",
            "default_class = \"cash_mmf\"\n",
        ));
        let ra = run_plan_pipeline(&a).unwrap();

        let t2 = TempDir::new().unwrap();
        let mut b = simple_inputs(&t2, WITH_UNMAPPED);
        b.treatment = Treatment::Hold;
        b.symbol_map_toml = Some(write_file(
            t2.path(),
            "in/m.toml",
            "default_class = \"cash_mmf\"\n",
        ));
        b.default_class = Some(AssetClass::TechMegaCap);
        let rb = run_plan_pipeline(&b).unwrap();

        assert!(
            (ra.year5_nav_mean - rb.year5_nav_mean).abs() > 1.0,
            "explicit default_class (tech_mega_cap) must override TOML's cash_mmf"
        );
    }

    #[test]
    fn missing_symbol_map_file_is_an_error() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        inputs.symbol_map_toml = Some(tmp.path().join("nope/symbol_map.toml"));
        assert!(run_plan_pipeline(&inputs).is_err());
    }

    // ── Output schema ─────────────────────────────────────────────────────

    #[test]
    fn output_csvs_use_concentrated_column_names() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = canonical_inputs(&tmp);
        inputs.mc_paths = 20;
        run_plan_pipeline(&inputs).unwrap();
        let proj = std::fs::read_to_string(tmp.path().join("nav_projection.csv")).unwrap();
        let header = proj.lines().next().unwrap();
        assert!(
            header.contains("concentrated_pct_p05") && header.contains("concentrated_pct_p95"),
            "{header}"
        );
        assert!(!header.contains("amzn"), "{header}");
        let paths = std::fs::read_to_string(tmp.path().join("nav_mc_paths.csv")).unwrap();
        let header = paths.lines().next().unwrap();
        assert!(
            header.contains("concentrated_value") && header.contains("concentrated_pct_of_nav"),
            "{header}"
        );
        assert!(!header.contains("amzn"), "{header}");
    }

    #[test]
    fn year0_concentrated_pct_equals_msft_share_of_nav() {
        // MSFT = 546,000 / 1,043,770 = 52.31%
        let tmp = TempDir::new().unwrap();
        let mut inputs = canonical_inputs(&tmp);
        inputs.mc_paths = 20;
        run_plan_pipeline(&inputs).unwrap();
        let mut rdr = csv::Reader::from_path(tmp.path().join("nav_projection.csv")).unwrap();
        let headers = rdr.headers().unwrap().clone();
        let idx = headers
            .iter()
            .position(|h| h == "concentrated_pct_p50")
            .unwrap();
        let first = rdr.records().next().unwrap().unwrap();
        let v: f64 = first[idx].parse().unwrap();
        assert!((v - 52.31).abs() < 0.01, "year-0 concentrated pct = {v}");
    }

    // ═════════════════════════════════════════════════════════════════════════
    // discover_data_dir
    // ═════════════════════════════════════════════════════════════════════════

    #[test]
    fn discover_data_dir_finds_sorted_csvs_and_optional_files() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "positions/c.csv", "x");
        write_file(tmp.path(), "positions/a.csv", "x");
        write_file(tmp.path(), "positions/b.CSV", "x"); // case-insensitive extension
        write_file(tmp.path(), "positions/notes.txt", "ignore me");
        write_file(tmp.path(), "vesting.csv", "x");
        write_file(tmp.path(), "symbol_map.toml", "");
        write_file(tmp.path(), "plan.toml", "");
        let dd = discover_data_dir(tmp.path()).unwrap();
        let names: Vec<String> = dd
            .positions
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.csv", "b.CSV", "c.csv"]);
        assert_eq!(dd.vesting, Some(tmp.path().join("vesting.csv")));
        assert_eq!(dd.symbol_map, Some(tmp.path().join("symbol_map.toml")));
        assert_eq!(dd.plan_toml, Some(tmp.path().join("plan.toml")));
    }

    #[test]
    fn discover_data_dir_optional_files_are_none_when_absent() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "positions/only.csv", "x");
        let dd = discover_data_dir(tmp.path()).unwrap();
        assert_eq!(dd.positions.len(), 1);
        assert_eq!(dd.vesting, None);
        assert_eq!(dd.symbol_map, None);
        assert_eq!(dd.plan_toml, None);
    }

    #[test]
    fn discover_data_dir_errors_when_positions_dir_missing() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "vesting.csv", "x");
        let err = discover_data_dir(tmp.path()).expect_err("no positions/ dir must be an error");
        assert!(err.to_string().contains("positions"), "{err}");
    }

    #[test]
    fn discover_data_dir_errors_when_positions_dir_has_no_csv() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "positions/readme.txt", "x");
        std::fs::create_dir_all(tmp.path().join("positions/nested.csv")).unwrap(); // a dir, not a file
        assert!(discover_data_dir(tmp.path()).is_err());
    }

    #[test]
    fn discover_data_dir_errors_for_nonexistent_dir() {
        let tmp = TempDir::new().unwrap();
        assert!(discover_data_dir(&tmp.path().join("missing")).is_err());
    }

    // ═════════════════════════════════════════════════════════════════════════
    // load_plan_toml
    // ═════════════════════════════════════════════════════════════════════════

    const FULL_PLAN_TOML: &str = r#"
as_of = "2026-05-01"
symbol = "MSFT"
treatment = "hold"
withhold_rate = 0.25
horizon_years = 3
paths = 200
seed = 7
default_class = "us_large_cap_index"

[unwind]
ltcg_rate = 0.20
floor_pct = 0.30
cost_basis_per_share = 150.0

[unwind.schedule]
"1" = 400
"2" = 300

[unwind.reinvest_weights]
VOO = 0.5
IEF = 0.5
"#;

    #[test]
    fn load_plan_toml_parses_full_document() {
        let tmp = TempDir::new().unwrap();
        let p = write_file(tmp.path(), "plan.toml", FULL_PLAN_TOML);
        let pt = load_plan_toml(&p).expect("full plan.toml must parse");
        assert_eq!(pt.symbol.as_deref(), Some("MSFT"));
        assert_eq!(pt.withhold_rate, Some(0.25));
        assert_eq!(pt.horizon_years, Some(3));
        assert_eq!(pt.paths, Some(200));
        assert_eq!(pt.seed, Some(7));
    }

    #[test]
    fn load_plan_toml_empty_file_yields_defaults() {
        let tmp = TempDir::new().unwrap();
        let p = write_file(tmp.path(), "plan.toml", "# everything commented out\n");
        let pt = load_plan_toml(&p).expect("empty plan.toml must parse");
        assert_eq!(pt.symbol, None);
        assert_eq!(pt.withhold_rate, None);
        assert_eq!(pt.horizon_years, None);
        assert_eq!(pt.paths, None);
        assert_eq!(pt.seed, None);
    }

    #[test]
    fn load_plan_toml_partial_document_leaves_other_fields_none() {
        let tmp = TempDir::new().unwrap();
        let p = write_file(tmp.path(), "plan.toml", "seed = 99\n");
        let pt = load_plan_toml(&p).unwrap();
        assert_eq!(pt.seed, Some(99));
        assert_eq!(pt.symbol, None);
    }

    #[test]
    fn load_plan_toml_malformed_is_an_error() {
        let tmp = TempDir::new().unwrap();
        let p = write_file(tmp.path(), "plan.toml", "seed = = 3\n[[");
        assert!(load_plan_toml(&p).is_err());
    }

    #[test]
    fn load_plan_toml_wrong_type_is_an_error() {
        let tmp = TempDir::new().unwrap();
        let p = write_file(tmp.path(), "plan.toml", "paths = \"many\"\n");
        assert!(load_plan_toml(&p).is_err());
    }

    #[test]
    fn load_plan_toml_missing_file_is_an_io_error() {
        let tmp = TempDir::new().unwrap();
        let err = load_plan_toml(&tmp.path().join("plan.toml")).expect_err("missing file");
        assert!(matches!(err, PlanError::Io(_)), "{err:?}");
    }

    // ═════════════════════════════════════════════════════════════════════════
    // write_data_templates
    // ═════════════════════════════════════════════════════════════════════════

    const TEMPLATE_FILES: [&str; 5] = [
        "positions/example_positions.csv",
        "vesting.csv",
        "symbol_map.toml",
        "plan.toml",
        "README.md",
    ];

    #[test]
    fn write_data_templates_creates_all_five_files() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("my_data");
        let written = write_data_templates(&dir, false).expect("fresh dir");
        assert_eq!(written.len(), 5, "{written:?}");
        for rel in TEMPLATE_FILES {
            let p = dir.join(rel);
            assert!(p.is_file(), "missing template {rel}");
            assert!(std::fs::metadata(&p).unwrap().len() > 0, "{rel} is empty");
            assert!(
                written.iter().any(|w| w.ends_with(rel)),
                "returned paths must include {rel}: {written:?}"
            );
        }
    }

    #[test]
    fn write_data_templates_works_into_existing_empty_dir() {
        let tmp = TempDir::new().unwrap();
        assert!(write_data_templates(tmp.path(), false).is_ok());
        assert!(tmp.path().join("README.md").is_file());
    }

    #[test]
    fn write_data_templates_refuses_to_overwrite_without_force() {
        let tmp = TempDir::new().unwrap();
        write_data_templates(tmp.path(), false).unwrap();
        let vesting = tmp.path().join("vesting.csv");
        std::fs::write(&vesting, "USER EDITED").unwrap();
        assert!(
            write_data_templates(tmp.path(), false).is_err(),
            "must not overwrite without force"
        );
        assert_eq!(std::fs::read_to_string(&vesting).unwrap(), "USER EDITED");
    }

    #[test]
    fn write_data_templates_errors_if_any_single_target_exists_and_writes_nothing_else() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "plan.toml", "USER PLAN");
        assert!(write_data_templates(tmp.path(), false).is_err());
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("plan.toml")).unwrap(),
            "USER PLAN"
        );
        // Assumption: the existence check happens before any write (all-or-nothing).
        assert!(!tmp.path().join("README.md").exists());
        assert!(!tmp.path().join("vesting.csv").exists());
        assert!(!tmp.path().join("positions/example_positions.csv").exists());
    }

    #[test]
    fn write_data_templates_force_overwrites() {
        let tmp = TempDir::new().unwrap();
        write_data_templates(tmp.path(), false).unwrap();
        let readme = tmp.path().join("README.md");
        let original = std::fs::read_to_string(&readme).unwrap();
        std::fs::write(&readme, "USER EDITED").unwrap();
        write_data_templates(tmp.path(), true).expect("force must overwrite");
        assert_eq!(std::fs::read_to_string(&readme).unwrap(), original);
    }

    #[test]
    fn template_positions_are_simple_format_and_vesting_is_minimal_format() {
        let tmp = TempDir::new().unwrap();
        write_data_templates(tmp.path(), false).unwrap();
        let pos =
            std::fs::read_to_string(tmp.path().join("positions/example_positions.csv")).unwrap();
        let first_data_line = pos
            .lines()
            .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .unwrap();
        let cols: Vec<String> = first_data_line
            .split(',')
            .map(|c| c.trim().to_lowercase())
            .collect();
        assert!(
            cols.contains(&"symbol".to_string())
                && cols.contains(&"quantity".to_string())
                && cols.contains(&"price".to_string()),
            "{first_data_line}"
        );
        assert!(
            !cols.contains(&"account number".to_string()),
            "must be simple format, not Fidelity"
        );
        let ves = std::fs::read_to_string(tmp.path().join("vesting.csv")).unwrap();
        let vheader = ves.lines().next().unwrap();
        assert!(
            vheader.contains("symbol")
                && vheader.contains("vesting_date")
                && vheader.contains("quantity"),
            "{vheader}"
        );
        assert!(
            vheader.split(',').count() < 12,
            "template vesting must use the minimal format: {vheader}"
        );
    }

    #[test]
    fn template_files_each_load_with_their_own_loader() {
        let tmp = TempDir::new().unwrap();
        write_data_templates(tmp.path(), false).unwrap();
        let dd = discover_data_dir(tmp.path()).expect("templates form a valid data dir");
        assert_eq!(dd.positions.len(), 1);
        assert!(dd.vesting.is_some() && dd.symbol_map.is_some() && dd.plan_toml.is_some());
        let rows = read_positions_auto(&dd.positions[0]).expect("positions template loads");
        assert!(!rows.is_empty());
        let sched = crate::portfolio::vesting::DistributionSchedule::from_csv_file(
            dd.vesting.as_ref().unwrap(),
        )
        .expect("vesting template loads");
        assert!(!sched.is_empty());
        SymbolMap::from_toml_file(dd.symbol_map.as_ref().unwrap())
            .expect("symbol_map template loads");
        load_plan_toml(dd.plan_toml.as_ref().unwrap()).expect("plan.toml template loads");
    }

    #[test]
    fn templates_run_through_pipeline_unchanged() {
        let data = TempDir::new().unwrap();
        write_data_templates(data.path(), false).unwrap();
        let dd = discover_data_dir(data.path()).unwrap();
        let out = TempDir::new().unwrap();
        for treatment in [
            Treatment::Hold,
            Treatment::SellReinvest,
            Treatment::OptionBUnwind,
        ] {
            let inputs = PlanInputs {
                positions_csvs: dd.positions.clone(),
                vesting_csv: dd.vesting.clone(),
                asset_classes_toml: asset_classes_toml(),
                treatment,
                mc_paths: 30,
                mc_seed: 5,
                horizon_years: 5,
                withhold_rate: 0.22,
                as_of: as_of(),
                out_dir: out.path().to_path_buf(),
                symbol: None,
                symbol_map_toml: dd.symbol_map.clone(),
                default_class: None,
                option_b: OptionBOverrides::default(),
            };
            let report = run_plan_pipeline(&inputs).unwrap_or_else(|e| {
                panic!("template data must run unchanged under {treatment:?}: {e}")
            });
            assert!(report.starting_nav > 0.0);
            assert_eq!(report.n_paths, 30);
            assert_eq!(report.artifact_paths.len(), 3);
        }
        let audit = read_audit(out.path());
        let sym = find_key(&audit, "concentrated_symbol")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        assert!(
            !sym.is_empty(),
            "audit must record a concentrated_symbol for template data"
        );
    }

    // ═════════════════════════════════════════════════════════════════════════
    // Round 2 regressions (H1, H2, H4, M1, M4, audit key)
    // ═════════════════════════════════════════════════════════════════════════

    // ── H1: concentrated symbol that is also a reinvest target ────────────

    const VOO_HEAVY: &str = "symbol,quantity,price,cost_basis\n\
        VOO,400,600,420\nIEF,500,95,98\nSPAXX,20000,1,1\n";

    #[test]
    fn h1_concentrated_symbol_in_reinvest_weights_is_dropped_and_rest_renormalized() {
        // weights [VOO 0.5, SPAXX 0.5] with --symbol VOO must behave exactly like
        // [SPAXX 1.0]: VOO is dropped, SPAXX is renormalized to the original sum.
        let t1 = TempDir::new().unwrap();
        let mut a = simple_inputs(&t1, VOO_HEAVY);
        a.symbol = Some("VOO".to_string());
        a.option_b.cost_basis_per_share = Some(600.0);
        a.option_b.reinvest_weights =
            Some(vec![("VOO".to_string(), 0.5), ("SPAXX".to_string(), 0.5)]);
        let ra = run_plan_pipeline(&a).expect("VOO in weights must not be an error");

        let t2 = TempDir::new().unwrap();
        let mut b = simple_inputs(&t2, VOO_HEAVY);
        b.symbol = Some("VOO".to_string());
        b.option_b.cost_basis_per_share = Some(600.0);
        b.option_b.reinvest_weights = Some(vec![("SPAXX".to_string(), 1.0)]);
        let rb = run_plan_pipeline(&b).unwrap();

        assert!(
            (ra.year5_nav_mean - rb.year5_nav_mean).abs() < 1e-6,
            "proceeds routed to the concentrated symbol were lost: {} vs {}",
            ra.year5_nav_mean,
            rb.year5_nav_mean
        );
    }

    #[test]
    fn h1_default_weights_with_symbol_voo_runs() {
        let tmp = TempDir::new().unwrap();
        let mut inputs = simple_inputs(&tmp, VOO_HEAVY);
        inputs.symbol = Some("VOO".to_string());
        run_plan_pipeline(&inputs).expect("--symbol VOO with default weights must work");
    }

    // ── H2: vests apply only to the concentrated symbol ───────────────────

    #[test]
    fn h2_mixed_symbol_vesting_file_only_applies_matching_symbol() {
        // Vesting has MSFT 40 + AAPL 5000 events. With symbol MSFT only the MSFT
        // events may count: result equals a vesting file with just the MSFT rows.
        let tmp = TempDir::new().unwrap();
        let mixed = write_file(
            tmp.path(), "in/mixed.csv",
            "symbol,vesting_date,quantity\nMSFT,2026-08-15,40\nAAPL,2026-09-15,5000\nMSFT,2026-11-15,40\n",
        );
        let only = write_file(
            tmp.path(),
            "in/only.csv",
            "symbol,vesting_date,quantity\nMSFT,2026-08-15,40\nMSFT,2026-11-15,40\n",
        );
        let mut a = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        a.treatment = Treatment::Hold;
        a.vesting_csv = Some(mixed);
        a.out_dir = tmp.path().join("out_a");
        let ra = run_plan_pipeline(&a).unwrap();
        let mut b = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        b.treatment = Treatment::Hold;
        b.vesting_csv = Some(only);
        b.out_dir = tmp.path().join("out_b");
        let rb = run_plan_pipeline(&b).unwrap();
        assert!(
            (ra.year5_nav_mean - rb.year5_nav_mean).abs() < 1e-6,
            "AAPL vests leaked into MSFT projection: {} vs {}",
            ra.year5_nav_mean,
            rb.year5_nav_mean
        );
    }

    // ── M1: tickers upper-cased at ingestion ──────────────────────────────

    const LOWER_POSITIONS: &str = "symbol,quantity,price,cost_basis\n\
        msft,400,420,185\nvoo,100,600,420\nief,500,95,98\nspaxx,20000,1,1\n";
    const VEST_UPPER: &str =
        "symbol,vesting_date,quantity\nMSFT,2026-08-15,40\nMSFT,2026-11-15,40\n";
    const VEST_LOWER: &str =
        "symbol,vesting_date,quantity\nmsft,2026-08-15,40\nmsft,2026-11-15,40\n";

    fn run_with(
        positions: &str,
        vesting: &str,
        symbol: Option<&str>,
    ) -> (PlanReport, serde_json::Value) {
        let tmp = TempDir::new().unwrap();
        let mut inputs = simple_inputs(&tmp, positions);
        inputs.treatment = Treatment::Hold;
        inputs.vesting_csv = Some(write_file(tmp.path(), "in/v.csv", vesting));
        inputs.symbol = symbol.map(|s| s.to_string());
        let r = run_plan_pipeline(&inputs).expect("pipeline must succeed");
        (r, read_audit(&tmp.path().join("out")))
    }

    #[test]
    fn m1_lowercase_positions_with_uppercase_vesting_work() {
        let (base, _) = run_with(SIMPLE_WITH_BASIS, VEST_UPPER, None);
        let (r, audit) = run_with(LOWER_POSITIONS, VEST_UPPER, None);
        assert_eq!(
            find_key(&audit, "concentrated_symbol").and_then(|v| v.as_str()),
            Some("MSFT")
        );
        assert!((r.year5_nav_mean - base.year5_nav_mean).abs() < 1e-6);
        assert!((r.starting_nav - base.starting_nav).abs() < 1e-9);
    }

    #[test]
    fn m1_uppercase_positions_with_lowercase_vesting_work() {
        let (base, _) = run_with(SIMPLE_WITH_BASIS, VEST_UPPER, None);
        let (r, audit) = run_with(SIMPLE_WITH_BASIS, VEST_LOWER, None);
        assert_eq!(
            find_key(&audit, "concentrated_symbol").and_then(|v| v.as_str()),
            Some("MSFT")
        );
        assert!(
            (r.year5_nav_mean - base.year5_nav_mean).abs() < 1e-6,
            "lowercase vesting symbol must still apply (same result as uppercase)"
        );
    }

    #[test]
    fn m1_lowercase_explicit_symbol_is_upper_cased_and_recorded() {
        let (base, _) = run_with(SIMPLE_WITH_BASIS, VEST_UPPER, Some("MSFT"));
        let (r, audit) = run_with(SIMPLE_WITH_BASIS, VEST_UPPER, Some("msft"));
        assert_eq!(
            find_key(&audit, "concentrated_symbol").and_then(|v| v.as_str()),
            Some("MSFT")
        );
        assert!((r.year5_nav_mean - base.year5_nav_mean).abs() < 1e-6);
    }

    #[test]
    fn m1_lowercase_reinvest_weight_tickers_behave_like_uppercase() {
        let t1 = TempDir::new().unwrap();
        let mut a = simple_inputs(&t1, SIMPLE_WITH_BASIS);
        a.option_b.reinvest_weights =
            Some(vec![("VOO".to_string(), 0.7), ("IEF".to_string(), 0.3)]);
        let ra = run_plan_pipeline(&a).unwrap();
        let t2 = TempDir::new().unwrap();
        let mut b = simple_inputs(&t2, SIMPLE_WITH_BASIS);
        b.option_b.reinvest_weights =
            Some(vec![("voo".to_string(), 0.7), ("ief".to_string(), 0.3)]);
        let rb = run_plan_pipeline(&b).unwrap();
        assert!((ra.year5_nav_mean - rb.year5_nav_mean).abs() < 1e-6);
    }

    #[test]
    fn m1_load_plan_toml_upper_cases_symbol_and_reinvest_weight_tickers() {
        let tmp = TempDir::new().unwrap();
        let p = write_file(
            tmp.path(),
            "plan.toml",
            "symbol = \"msft\"\n[unwind.reinvest_weights]\nvoo = 0.5\nief = 0.5\n",
        );
        let pt = load_plan_toml(&p).unwrap();
        assert_eq!(pt.symbol.as_deref(), Some("MSFT"));
        let mut tickers: Vec<String> = pt
            .reinvest_weights()
            .expect("weights present")
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        tickers.sort();
        assert_eq!(tickers, vec!["IEF".to_string(), "VOO".to_string()]);
    }

    // ── M4: validation pass returns PlanError, never panics ───────────────

    fn assert_rejected(inputs: &PlanInputs, keyword: &str) {
        match run_plan_pipeline(inputs) {
            Ok(_) => panic!("expected a validation error mentioning `{keyword}`, got Ok"),
            Err(e) => assert!(
                e.to_string().to_lowercase().contains(keyword),
                "error should mention `{keyword}`: {e}"
            ),
        }
    }

    #[test]
    fn m4_zero_paths_is_rejected() {
        let tmp = TempDir::new().unwrap();
        let mut i = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        i.mc_paths = 0;
        assert_rejected(&i, "paths");
    }

    #[test]
    fn m4_withhold_rate_outside_unit_interval_is_rejected() {
        for bad in [-0.01, 1.0, 1.5, f64::NAN, f64::INFINITY] {
            let tmp = TempDir::new().unwrap();
            let mut i = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
            i.withhold_rate = bad;
            assert_rejected(&i, "withhold");
        }
    }

    #[test]
    fn m4_withhold_rate_zero_is_valid() {
        let tmp = TempDir::new().unwrap();
        let mut i = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        i.withhold_rate = 0.0;
        run_plan_pipeline(&i).expect("0.0 is inside [0,1)");
    }

    #[test]
    fn m4_ltcg_rate_outside_unit_interval_is_rejected() {
        for bad in [-0.1, 1.0, 2.0, f64::NAN] {
            let tmp = TempDir::new().unwrap();
            let mut i = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
            i.option_b.ltcg_rate = Some(bad);
            assert_rejected(&i, "ltcg");
        }
    }

    #[test]
    fn m4_floor_pct_outside_unit_interval_is_rejected() {
        for bad in [-0.1, 1.0, 3.0, f64::NAN] {
            let tmp = TempDir::new().unwrap();
            let mut i = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
            i.option_b.floor_pct = Some(bad);
            assert_rejected(&i, "floor");
        }
    }

    #[test]
    fn m4_floor_pct_zero_is_valid_and_means_no_floor() {
        let tmp = TempDir::new().unwrap();
        let mut i = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        i.option_b.floor_pct = Some(0.0);
        run_plan_pipeline(&i).expect("floor_pct = 0 is valid");
    }

    #[test]
    fn m4_non_finite_quantity_or_price_is_rejected() {
        for (csv, kw) in [
            ("symbol,quantity,price\nMSFT,NaN,420\n", "quantity"),
            ("symbol,quantity,price\nMSFT,inf,420\n", "quantity"),
            ("symbol,quantity,price\nMSFT,10,NaN\n", "price"),
            ("symbol,quantity,price\nMSFT,10,inf\n", "price"),
        ] {
            let tmp = TempDir::new().unwrap();
            let i = simple_inputs(&tmp, csv);
            assert_rejected(&i, kw);
        }
    }

    #[test]
    fn m4_negative_quantity_is_rejected() {
        let tmp = TempDir::new().unwrap();
        let i = simple_inputs(&tmp, "symbol,quantity,price\nMSFT,-5,420\nVOO,10,600\n");
        assert_rejected(&i, "quantity");
    }

    #[test]
    fn m4_negative_unwind_shares_is_rejected() {
        let tmp = TempDir::new().unwrap();
        let mut i = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        i.option_b.unwind_schedule = Some(vec![(1, -10.0)]);
        assert_rejected(&i, "unwind");
    }

    #[test]
    fn m4_duplicate_unwind_years_are_rejected() {
        let tmp = TempDir::new().unwrap();
        let mut i = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        i.option_b.unwind_schedule = Some(vec![(1, 10.0), (1, 20.0)]);
        assert_rejected(&i, "unwind");
    }

    #[test]
    fn m4_reinvest_weights_must_sum_to_one() {
        for bad in [
            vec![("VOO".to_string(), 0.5), ("IEF".to_string(), 0.3)],
            vec![("VOO".to_string(), 0.7), ("IEF".to_string(), 0.5)],
            vec![("VOO".to_string(), 0.600002), ("IEF".to_string(), 0.4)],
        ] {
            let tmp = TempDir::new().unwrap();
            let mut i = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
            i.option_b.reinvest_weights = Some(bad);
            assert_rejected(&i, "weight");
        }
    }

    #[test]
    fn m4_reinvest_weights_within_1e6_of_one_are_accepted() {
        let tmp = TempDir::new().unwrap();
        let mut i = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        i.option_b.reinvest_weights = Some(vec![
            ("VOO".to_string(), 0.6000005),
            ("IEF".to_string(), 0.4),
        ]);
        run_plan_pipeline(&i).expect("sum 1.0000005 is within tolerance");
    }

    #[test]
    fn m4_default_reinvest_weights_pass_validation() {
        // Whatever the built-in default weights sum to, a default run must work.
        let tmp = TempDir::new().unwrap();
        let mut i = simple_inputs(&tmp, SIMPLE_WITH_BASIS);
        i.option_b.reinvest_weights = None;
        run_plan_pipeline(&i).expect("default weights must validate");
    }

    // ── Audit key rename ──────────────────────────────────────────────────

    #[test]
    fn audit_json_uses_year5_concentrated_pct_percentiles_key() {
        let tmp = TempDir::new().unwrap();
        let mut i = canonical_inputs(&tmp);
        i.mc_paths = 20;
        run_plan_pipeline(&i).unwrap();
        let audit = read_audit(tmp.path());
        assert!(
            find_key(&audit, "year5_concentrated_pct_percentiles").is_some(),
            "audit.json must contain year5_concentrated_pct_percentiles"
        );
        assert!(
            find_key(&audit, "year5_amzn_pct_percentiles").is_none(),
            "old amzn-named key must be gone"
        );
        let raw = std::fs::read_to_string(tmp.path().join("audit.json")).unwrap();
        assert!(
            !raw.to_lowercase().contains("amzn"),
            "no amzn mention in audit.json"
        );
    }

    // ── H4: asset-classes error message includes the path ─────────────────

    #[test]
    fn h4_missing_asset_classes_error_message_includes_the_path() {
        let tmp = TempDir::new().unwrap();
        let mut i = canonical_inputs(&tmp);
        i.asset_classes_toml = PathBuf::from("/nonexistent_dir_for_ghost_alpha/xyz_classes.toml");
        let err = run_plan_pipeline(&i).expect_err("missing asset classes file");
        assert!(err.to_string().contains("xyz_classes.toml"), "{err}");
    }

    // ═══ Round 3 ═══

    #[test]
    fn r3_pipeline_errors_when_all_reinvest_weight_is_on_the_concentrated_symbol() {
        let tmp = TempDir::new().unwrap();
        let mut i = simple_inputs(&tmp, VOO_HEAVY);
        i.symbol = Some("VOO".to_string());
        i.option_b.reinvest_weights = Some(vec![("VOO".to_string(), 1.0)]);
        let err = run_plan_pipeline(&i).expect_err("proceeds would be lost");
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("reinvest") && msg.contains("concentrated"),
            "{msg}"
        );
    }

    #[test]
    fn r3_negative_quantity_error_says_shorts_are_not_supported() {
        let tmp = TempDir::new().unwrap();
        let i = simple_inputs(&tmp, "symbol,quantity,price\nMSFT,-5,420\nVOO,10,600\n");
        let msg = run_plan_pipeline(&i)
            .expect_err("negative quantity")
            .to_string()
            .to_lowercase();
        assert!(
            msg.contains("short") || msg.contains("not supported"),
            "{msg}"
        );
    }
}
