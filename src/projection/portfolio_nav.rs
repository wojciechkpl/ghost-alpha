//! Forward portfolio NAV projection types.
//!
//! `Treatment` is the policy choice for handling RSU vests during projection.
//! `PortfolioState` is the year-end snapshot the projection engine produces.
//! `project_path` walks a portfolio forward under a [`ProjectionConfig`] that
//! names the concentrated position and carries the Option B parameters.

use crate::portfolio::symbol_map::SymbolMap;
use crate::portfolio::types::Portfolio;
use crate::portfolio::vesting::DistributionSchedule;
use crate::projection::asset_class_library::AssetClassLibrary;
use crate::projection::monte_carlo::AssetClass;
use crate::projection::scenario::ScenarioPath;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Key of the synthetic cash holding used when SellReinvest has nowhere to park proceeds.
const SYNTHETIC_CASH: &str = "CASH";

/// Policy for handling vested RSU shares during projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Treatment {
    /// Vested shares accumulate in the concentrated position; concentration grows.
    Hold,
    /// Sell every vested share at vest date; redistribute proceeds pro-rata
    /// across the non-concentrated holdings at then-current weights.
    SellReinvest,
    /// Sell vested shares AND run the concentration-floor unwind plan.
    OptionBUnwind,
}

/// Portfolio state at a year-end during a projected path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortfolioState {
    pub year: u32,
    pub as_of: NaiveDate,
    pub holdings: BTreeMap<String, f64>,
    /// Ticker of the concentrated position these figures refer to.
    pub concentrated_symbol: String,
    pub concentrated_shares: f64,
    pub cash: f64,
    pub treatment: Treatment,
}

impl PortfolioState {
    /// Sum of all holdings values.
    pub fn nav(&self) -> f64 {
        self.holdings.values().sum()
    }

    /// Concentrated position $ value (0.0 if not in holdings).
    pub fn concentrated_value(&self) -> f64 {
        self.holdings
            .get(&self.concentrated_symbol)
            .copied()
            .unwrap_or(0.0)
    }

    /// Concentrated position as percentage of NAV (0.0 if NAV is zero).
    pub fn concentrated_pct_of_nav(&self) -> f64 {
        let nav = self.nav();
        if nav <= 0.0 {
            return 0.0;
        }
        100.0 * self.concentrated_value() / nav
    }
}

// ─── ProjectionError ─────────────────────────────────────────────────────────

/// Errors that can be returned by `project_path`.
#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    #[error(
        "Portfolio contains unknown ticker(s): {0}. Map them in symbol_map.toml \
         (or set default_class)"
    )]
    UnknownSymbol(String),

    #[error("Portfolio missing {0} position; required for vesting projections")]
    MissingConcentrated(String),

    #[error(
        "Option B reinvest weights leave no target other than the concentrated symbol {0}; \
         add at least one other ticker with a positive weight"
    )]
    NoReinvestTarget(String),

    #[error("Treatment {0:?} not implemented yet (Gap {1})")]
    NotImplementedYet(Treatment, &'static str),

    #[error("Empty scenario path: yearly_returns must have at least 2 entries (year 0 + horizon)")]
    EmptyScenario,
}

// ─── Config ──────────────────────────────────────────────────────────────────

/// Configuration for `Treatment::OptionBUnwind`.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionBConfig {
    /// Average cost basis per share of the concentrated position.
    /// 0.0 is conservative: the whole sale price is taxed as gain.
    pub cost_basis_per_share: f64,
    /// Long-term capital-gains rate (20% federal + 3.8% NIIT, no state).
    pub ltcg_rate: f64,
    /// Concentration floor, as a fraction of NAV.
    pub floor_pct: f64,
    /// Year-indexed schedule `(year, shares)` of existing shares to attempt to sell.
    pub unwind_schedule: Vec<(u32, f64)>,
    /// Reinvestment routing for after-tax proceeds (sums to ~1.0).
    pub reinvest_weights: Vec<(String, f64)>,
}

impl Default for OptionBConfig {
    fn default() -> Self {
        Self {
            cost_basis_per_share: 0.0,
            ltcg_rate: 0.238,
            floor_pct: 0.25,
            unwind_schedule: Vec::new(),
            reinvest_weights: vec![
                ("IEF".into(), 0.214),
                ("TLT".into(), 0.053),
                ("SCHP".into(), 0.053),
                ("IAU".into(), 0.053),
                ("DBMF".into(), 0.071),
                ("VXUS".into(), 0.178),
                ("VWO".into(), 0.071),
                ("VOO".into(), 0.281),
                ("FZROX".into(), 0.025),
            ],
        }
    }
}

impl OptionBConfig {
    /// Scheduled shares to sell in `year`; 0.0 if the year is not in the schedule.
    pub fn scheduled_for_year(&self, year: u32) -> f64 {
        self.unwind_schedule
            .iter()
            .find(|(y, _)| *y == year)
            .map(|(_, qty)| *qty)
            .unwrap_or(0.0)
    }
}

/// Everything `project_path` needs beyond the portfolio, vesting and scenario.
#[derive(Debug, Clone)]
pub struct ProjectionConfig {
    pub concentrated_symbol: String,
    pub symbol_map: SymbolMap,
    pub option_b: OptionBConfig,
}

impl ProjectionConfig {
    /// Built-in symbol map and default Option B parameters.
    pub fn new(symbol: impl Into<String>) -> Self {
        Self {
            concentrated_symbol: symbol.into(),
            symbol_map: SymbolMap::builtin(),
            option_b: OptionBConfig::default(),
        }
    }
}

/// Pick the concentrated symbol: explicit > vesting symbol with the most gross
/// shares > largest-market-value position whose class is not `CashMmf`.
pub fn resolve_concentrated_symbol(
    explicit: Option<&str>,
    vesting: &DistributionSchedule,
    portfolio: &Portfolio,
    map: &SymbolMap,
) -> Option<String> {
    if let Some(sym) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(sym.to_string());
    }
    let mut best: Option<(String, f64)> = None;
    for sym in vesting.symbols() {
        let total = vesting.total_gross_quantity(&sym);
        if best.as_ref().is_none_or(|(_, b)| total > *b) {
            best = Some((sym, total));
        }
    }
    if let Some((sym, _)) = best {
        return Some(sym);
    }
    portfolio
        .positions
        .iter()
        .filter(|p| map.class_of(&p.ticker) != Some(AssetClass::CashMmf))
        .max_by(|a, b| {
            a.market_value()
                .partial_cmp(&b.market_value())
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|p| p.ticker.clone())
}

// ─── project_path ─────────────────────────────────────────────────────────────

/// Project portfolio state forward over `scenario`'s horizon under `treatment`.
///
/// Returns `Vec<PortfolioState>` of length `horizon + 1` where index 0 is
/// year-0 (initial portfolio) and indices 1..=horizon are EOY snapshots.
///
/// `withhold_rate` is the employer sell-to-cover withholding for RSU vests
/// (e.g. 0.22 for the federal supplemental rate).
pub fn project_path(
    portfolio: &Portfolio,
    vesting: &DistributionSchedule,
    treatment: Treatment,
    scenario: &ScenarioPath,
    _library: &AssetClassLibrary,
    withhold_rate: f64,
    config: &ProjectionConfig,
) -> Result<Vec<PortfolioState>, ProjectionError> {
    let sym = config.concentrated_symbol.as_str();
    let map = &config.symbol_map;

    // Class lookup for any holding key. The synthetic cash key always behaves
    // as CashMmf unless the user mapped it explicitly.
    let class_of = |t: &str| -> Option<AssetClass> {
        map.class_of(t)
            .or((t == SYNTHETIC_CASH).then_some(AssetClass::CashMmf))
    };
    // Class driving the concentrated symbol's price walk. If the symbol is
    // absent from the portfolio its value is zero whatever class is used.
    let conc_class = class_of(sym).unwrap_or(AssetClass::UsLargeCapIndex);

    // ---- Validate horizon ----
    let horizon = scenario
        .yearly_returns
        .len()
        .checked_sub(1)
        .ok_or(ProjectionError::EmptyScenario)?;
    if horizon == 0 {
        return Err(ProjectionError::EmptyScenario);
    }

    // ---- Reinvest weights: drop the concentrated symbol (selling it and buying it
    // back would be overwritten by the Step 4 rebuild) and renormalize the rest. ----
    let reinvest_weights: Vec<(String, f64)> = {
        let kept: Vec<&(String, f64)> = config
            .option_b
            .reinvest_weights
            .iter()
            .filter(|(t, _)| !t.eq_ignore_ascii_case(sym))
            .collect();
        let total_all: f64 = config
            .option_b
            .reinvest_weights
            .iter()
            .map(|(_, w)| w)
            .sum();
        let total_kept: f64 = kept.iter().map(|(_, w)| w).sum();
        if kept.len() == config.option_b.reinvest_weights.len() || total_kept <= 0.0 {
            kept.into_iter().cloned().collect()
        } else {
            kept.into_iter()
                .map(|(t, w)| (t.clone(), w * total_all / total_kept))
                .collect()
        }
    };

    if treatment == Treatment::OptionBUnwind
        && reinvest_weights.iter().map(|(_, w)| w).sum::<f64>() <= 0.0
    {
        return Err(ProjectionError::NoReinvestTarget(sym.to_string()));
    }

    // ---- Verify all positions (and Option B reinvest tickers) have known asset classes ----
    let mut unknown: Vec<&str> = Vec::new();
    let option_b_tickers = (treatment == Treatment::OptionBUnwind)
        .then_some(reinvest_weights.iter().map(|(t, _)| t.as_str()))
        .into_iter()
        .flatten();
    for ticker in portfolio
        .positions
        .iter()
        .map(|p| p.ticker.as_str())
        .chain(option_b_tickers)
    {
        if class_of(ticker).is_none() && !unknown.contains(&ticker) {
            unknown.push(ticker);
        }
    }
    if !unknown.is_empty() {
        return Err(ProjectionError::UnknownSymbol(unknown.join(", ")));
    }

    // ---- Concentrated position initial state ----
    let conc_pos = portfolio.position(sym);
    let mut conc_shares = conc_pos.map(|p| p.total_shares()).unwrap_or(0.0);
    let conc_price_t0 = conc_pos.map(|p| p.current_price).unwrap_or(0.0);

    // A single-year horizon with no concentrated position is permitted (vested
    // shares accumulate at t0_price=0). Multi-year projections need a real price.
    if conc_pos.is_none() && !vesting.events.is_empty() && horizon > 1 {
        return Err(ProjectionError::MissingConcentrated(sym.to_string()));
    }

    // ---- Build year-0 holdings from market values ----
    let mut holdings: BTreeMap<String, f64> = portfolio
        .positions
        .iter()
        .map(|p| (p.ticker.clone(), p.market_value()))
        .collect();

    let cash_at = |h: &BTreeMap<String, f64>| -> f64 {
        h.iter()
            .filter(|(t, _)| t.as_str() != sym && class_of(t) == Some(AssetClass::CashMmf))
            .map(|(_, v)| *v)
            .sum()
    };

    let as_of_for = |y: u32| {
        portfolio
            .as_of
            .checked_add_days(chrono::Days::new(365u64 * y as u64))
            .expect("date arithmetic")
    };

    let snapshot = |year: u32,
                    as_of: NaiveDate,
                    holdings: &BTreeMap<String, f64>,
                    conc_shares: f64| PortfolioState {
        year,
        as_of,
        holdings: holdings.clone(),
        concentrated_symbol: sym.to_string(),
        concentrated_shares: conc_shares,
        cash: cash_at(holdings),
        treatment,
    };

    let mut states = Vec::with_capacity(horizon + 1);
    states.push(snapshot(0, portfolio.as_of, &holdings, conc_shares));

    // ---- Walk years 1..=horizon ----
    for y in 1..=horizon {
        let year_start = as_of_for((y - 1) as u32);
        let year_end = as_of_for(y as u32);

        // Step 1: compound non-concentrated holdings by their class return (full year).
        for (ticker, value) in holdings.iter_mut() {
            if ticker == sym {
                continue; // handled separately via shares x walked price
            }
            let class = class_of(ticker).expect("validated above; cannot fail");
            *value *= 1.0 + scenario.class_return(y, class);
        }

        // Step 2: process this year's vests of the concentrated symbol (case-insensitive).
        let vests_in_year: Vec<_> = vesting
            .events_in_range(year_start, year_end)
            .into_iter()
            .filter(|e| e.symbol.eq_ignore_ascii_case(sym))
            .collect();
        match treatment {
            Treatment::Hold => {
                for vest in vests_in_year {
                    conc_shares += vest.net_quantity(withhold_rate);
                }
            }
            Treatment::SellReinvest => {
                // Sell at vest-date price; distribute proceeds pro-rata to the
                // non-concentrated sleeve. Concentrated share count never changes.
                for vest in vests_in_year {
                    let qty_net = vest.net_quantity(withhold_rate);
                    let vest_yrs_from_t0 =
                        (vest.vesting_date - portfolio.as_of).num_days() as f64 / 365.0;
                    let price_at_vest =
                        scenario.walk_price(conc_price_t0, conc_class, vest_yrs_from_t0);
                    let proceeds = qty_net * price_at_vest;
                    let yrs_remaining = (year_end - vest.vesting_date).num_days() as f64 / 365.0;

                    // Python iterates the live dict, so weights for each vest
                    // include previous vest additions.
                    let sleeve_total: f64 = holdings
                        .iter()
                        .filter(|(t, _)| t.as_str() != sym)
                        .map(|(_, v)| *v)
                        .sum();

                    if sleeve_total <= 0.0 {
                        // No sleeve: park proceeds at face value (not compounded) in the
                        // first CashMmf holding, else a synthetic CASH holding.
                        let target = holdings
                            .keys()
                            .find(|t| t.as_str() != sym && class_of(t) == Some(AssetClass::CashMmf))
                            .cloned()
                            .unwrap_or_else(|| SYNTHETIC_CASH.to_string());
                        *holdings.entry(target).or_insert(0.0) += proceeds;
                    } else {
                        let sleeve_tickers: Vec<String> = holdings
                            .keys()
                            .filter(|t| t.as_str() != sym)
                            .cloned()
                            .collect();
                        for ticker in sleeve_tickers {
                            let value = holdings[&ticker];
                            let share_w = value / sleeve_total;
                            let class = class_of(&ticker).expect("validated above; cannot fail");
                            let addition = proceeds
                                * share_w
                                * (1.0 + scenario.class_return(y, class)).powf(yrs_remaining);
                            *holdings.get_mut(&ticker).expect("key collected above") += addition;
                        }
                    }
                }
            }
            Treatment::OptionBUnwind => {
                // Sell at vest-date price; distribute proceeds to fixed reinvest weights.
                for vest in vests_in_year {
                    let qty_net = vest.net_quantity(withhold_rate);
                    let vest_yrs_from_t0 =
                        (vest.vesting_date - portfolio.as_of).num_days() as f64 / 365.0;
                    let price_at_vest =
                        scenario.walk_price(conc_price_t0, conc_class, vest_yrs_from_t0);
                    let proceeds = qty_net * price_at_vest;
                    let yrs_remaining = (year_end - vest.vesting_date).num_days() as f64 / 365.0;
                    for (ticker, weight) in &reinvest_weights {
                        let class = class_of(ticker).expect("validated above; cannot fail");
                        let r = scenario.class_return(y, class);
                        let addn = proceeds * weight * (1.0 + r).powf(yrs_remaining);
                        *holdings.entry(ticker.clone()).or_insert(0.0) += addn;
                    }
                }
            }
        }

        // Step 3 (OptionBUnwind only): unwind existing shares per schedule, capped by floor.
        if treatment == Treatment::OptionBUnwind {
            let cfg = &config.option_b;
            let sh_scheduled = cfg.scheduled_for_year(y as u32).min(conc_shares);
            if sh_scheduled > 0.0 {
                let price_year_start =
                    scenario.walk_price(conc_price_t0, conc_class, (y - 1) as f64);
                let price_year_end = scenario.walk_price(conc_price_t0, conc_class, y as f64);
                let avg_sale_price = (price_year_start + price_year_end) / 2.0;
                let gain_per_share = avg_sale_price - cfg.cost_basis_per_share;
                let net_per_share = avg_sale_price - gain_per_share.max(0.0) * cfg.ltcg_rate;

                // Non-concentrated EOY value after step 1 compound + step 2 vest distribution.
                let other_eoy: f64 = holdings
                    .iter()
                    .filter(|(t, _)| t.as_str() != sym)
                    .map(|(_, v)| *v)
                    .sum();

                // floor_pct == 0 means "no floor".
                let k_max = if cfg.floor_pct <= 0.0 {
                    f64::INFINITY
                } else {
                    crate::portfolio::concentration::compute_k_max(
                        conc_shares,
                        price_year_end,
                        other_eoy,
                        net_per_share,
                        cfg.floor_pct,
                    )
                };
                // 10b5-1 plans sell whole shares only.
                let sh_to_sell = sh_scheduled.min(k_max).floor();
                if sh_to_sell > 0.0 {
                    let gross = sh_to_sell * avg_sale_price;
                    let ltcg = (sh_to_sell * gain_per_share).max(0.0);
                    let net_proceeds = gross - ltcg * cfg.ltcg_rate;
                    conc_shares -= sh_to_sell;
                    // Distribute mid-year (0.5y compounding) into reinvest weights.
                    for (ticker, weight) in &reinvest_weights {
                        let class = class_of(ticker).expect("validated above; cannot fail");
                        let r = scenario.class_return(y, class);
                        let addn = net_proceeds * weight * (1.0 + r).powf(0.5);
                        *holdings.entry(ticker.clone()).or_insert(0.0) += addn;
                    }
                }
            }
        }

        // Step 4: rebuild concentrated EOY value from share count x walked price.
        let conc_value_eoy = conc_shares * scenario.walk_price(conc_price_t0, conc_class, y as f64);
        holdings.insert(sym.to_string(), conc_value_eoy);

        states.push(snapshot(y as u32, year_end, &holdings, conc_shares));
    }

    Ok(states)
}

#[cfg(test)]
mod tests {
    use super::{
        project_path as project_path_with_config, resolve_concentrated_symbol, OptionBConfig,
        PortfolioState, ProjectionConfig, ProjectionError, Treatment,
    };
    use crate::portfolio::symbol_map::SymbolMap;
    use crate::portfolio::types::{AcquisitionType, Portfolio, Position, TaxLot};
    use crate::portfolio::vesting::{DistributionSchedule, VestingEvent};
    use crate::projection::asset_class_library::AssetClassLibrary;
    use crate::projection::monte_carlo::AssetClass;
    use crate::projection::scenario::ScenarioPath;
    use chrono::NaiveDate;
    use std::collections::{BTreeMap, HashMap};
    use uuid::Uuid;

    // ── Test helpers ──────────────────────────────────────────────────────────

    /// Neutral Option-B parameters used by the legacy-style tests below
    /// (basis $100, LTCG 20%, floor 35%, schedule year1=2000, year2=1500).
    fn test_config() -> ProjectionConfig {
        let mut cfg = ProjectionConfig::new("AMZN");
        cfg.option_b = OptionBConfig {
            cost_basis_per_share: 100.0,
            ltcg_rate: 0.20,
            floor_pct: 0.35,
            unwind_schedule: vec![(1, 2000.0), (2, 1500.0)],
            ..OptionBConfig::default()
        };
        cfg
    }

    /// Same as `project_path_with_config` using `test_config()` (symbol AMZN).
    fn project_path(
        portfolio: &Portfolio,
        vesting: &DistributionSchedule,
        treatment: Treatment,
        scenario: &ScenarioPath,
        library: &AssetClassLibrary,
        withhold_rate: f64,
    ) -> Result<Vec<PortfolioState>, ProjectionError> {
        project_path_with_config(
            portfolio,
            vesting,
            treatment,
            scenario,
            library,
            withhold_rate,
            &test_config(),
        )
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn make_lot(ticker: &str, shares: f64, cost: f64, date: &str) -> TaxLot {
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

    fn make_position(ticker: &str, shares: f64, price: f64) -> Position {
        Position {
            ticker: ticker.to_string(),
            lots: vec![make_lot(ticker, shares, price * 0.8, "2023-01-01")],
            current_price: price,
            last_updated: d("2026-05-01"),
        }
    }

    /// Synthetic 2-position portfolio used across most tests:
    ///   AMZN: 100 shares @ $200.00 = $20,000
    ///   FNILX: 1000 shares @ $25.00 = $25,000
    ///   cash: $5,000
    ///   Total NAV: $50,000
    ///   as_of: 2026-05-01
    fn synthetic_portfolio() -> Portfolio {
        Portfolio {
            positions: vec![
                make_position("AMZN", 100.0, 200.0),
                make_position("FNILX", 1000.0, 25.0),
            ],
            cash: 5_000.0,
            as_of: d("2026-05-01"),
        }
    }

    /// 2-year scenario where every asset class returns exactly +10% each year.
    /// yearly_returns[0] is the zero placeholder; [1] and [2] are +10%.
    fn synthetic_scenario_2yr_constant_10pct() -> ScenarioPath {
        let zero: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let ten_pct: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.10)).collect();
        ScenarioPath {
            id: "test_2yr_10pct".to_string(),
            yearly_returns: vec![zero, ten_pct.clone(), ten_pct],
        }
    }

    fn empty_vesting() -> DistributionSchedule {
        DistributionSchedule { events: vec![] }
    }

    fn default_library() -> AssetClassLibrary {
        AssetClassLibrary::builtin_defaults()
    }

    /// Build an arbitrary portfolio from a list of (ticker, shares, price) plus
    /// a raw cash amount (stored as SPAXX shares @$1 if non-zero, otherwise pure cash).
    /// All positions use a cost basis of 80% of price and a 2023-01-01 lot date.
    fn synthetic_portfolio_with(holdings: &[(&str, f64, f64)]) -> Portfolio {
        let positions = holdings
            .iter()
            .map(|(ticker, shares, price)| make_position(ticker, *shares, *price))
            .collect();
        Portfolio {
            positions,
            cash: 0.0,
            as_of: d("2026-05-01"),
        }
    }

    /// 1-year scenario where every class returns the given constant rate.
    fn synthetic_scenario_1yr_const(rate: f64) -> ScenarioPath {
        let zero: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let uniform: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, rate)).collect();
        ScenarioPath {
            id: "test_1yr_const".to_string(),
            yearly_returns: vec![zero, uniform],
        }
    }

    /// 1-year scenario with per-class overrides; unspecified classes get `default_rate`.
    fn synthetic_scenario_1yr_overrides(
        default_rate: f64,
        overrides: &[(AssetClass, f64)],
    ) -> ScenarioPath {
        let zero: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let mut yr1: HashMap<AssetClass, f64> =
            AssetClass::iter_all().map(|c| (c, default_rate)).collect();
        for (cls, r) in overrides {
            yr1.insert(*cls, *r);
        }
        ScenarioPath {
            id: "test_1yr_overrides".to_string(),
            yearly_returns: vec![zero, yr1],
        }
    }

    fn make_vest_event(vest_date: &str, gross_qty: f64) -> VestingEvent {
        make_vest_event_for("AMZN", vest_date, gross_qty)
    }

    fn make_vest_event_for(symbol: &str, vest_date: &str, gross_qty: f64) -> VestingEvent {
        VestingEvent {
            snapshot_date: d("2026-05-01"),
            plan_name: "EXAMPLE RSU".to_string(),
            symbol: symbol.to_string(),
            plan_account: "EX0000001".to_string(),
            grant_date: d("2025-04-01"),
            grant_id: "G-0001".to_string(),
            vesting_date: d(vest_date),
            distribution_date: d(vest_date),
            gross_quantity: gross_qty,
            distribution_value: gross_qty * 200.0,
            distributed_in: "Shares".to_string(),
            source_file: "test.pdf".to_string(),
        }
    }

    // ── ScenarioPath helpers: 4 tests ─────────────────────────────────────

    #[test]
    fn class_return_returns_year_y_class_return() {
        // Build a scenario where year 1 TechMegaCap = 0.20
        let zero: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let mut yr1: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.05)).collect();
        yr1.insert(AssetClass::TechMegaCap, 0.20);
        let scenario = ScenarioPath {
            id: "t".to_string(),
            yearly_returns: vec![zero, yr1],
        };
        assert!(
            (scenario.class_return(1, AssetClass::TechMegaCap) - 0.20).abs() < 1e-12,
            "expected 0.20, got {}",
            scenario.class_return(1, AssetClass::TechMegaCap)
        );
    }

    #[test]
    fn class_return_year_zero_returns_placeholder_zero() {
        let scenario = synthetic_scenario_2yr_constant_10pct();
        assert_eq!(scenario.class_return(0, AssetClass::ConsumerMegaCap), 0.0);
    }

    #[test]
    fn walk_price_full_year_compounds() {
        // t0=$100, year 1 ConsumerMegaCap = 10% → walk_price(100, _, 1.0) == 110.0
        let scenario = synthetic_scenario_2yr_constant_10pct();
        let result = scenario.walk_price(100.0, AssetClass::ConsumerMegaCap, 1.0);
        assert!(
            (result - 110.0).abs() < 1e-9,
            "expected 110.0, got {result}"
        );
    }

    #[test]
    fn walk_price_fractional_year_uses_partial_compounding() {
        // t0=$100, year 1 = 10% → walk_price(100, _, 0.5) ≈ 100*(1.10)^0.5
        let expected = 100.0 * 1.10_f64.powf(0.5); // ≈ 104.88088...
        let scenario = synthetic_scenario_2yr_constant_10pct();
        let result = scenario.walk_price(100.0, AssetClass::ConsumerMegaCap, 0.5);
        assert!(
            (result - expected).abs() < 1e-6,
            "expected {expected}, got {result}"
        );
    }

    // ── project_path shape: 3 tests ───────────────────────────────────────

    #[test]
    fn project_path_returns_horizon_plus_one_states() {
        let portfolio = synthetic_portfolio();
        let scenario = synthetic_scenario_2yr_constant_10pct();
        let result = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("should succeed with synthetic portfolio");
        assert_eq!(
            result.len(),
            3,
            "expected horizon+1=3 states, got {}",
            result.len()
        );
    }

    #[test]
    fn project_path_year_zero_state_matches_initial_portfolio() {
        let portfolio = synthetic_portfolio();
        let scenario = synthetic_scenario_2yr_constant_10pct();
        let result = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("should succeed");
        let yr0 = &result[0];
        // Holdings should reflect initial market values
        let amzn_val = yr0.holdings.get("AMZN").copied().unwrap_or(0.0);
        assert!(
            (amzn_val - 20_000.0).abs() < 1e-6,
            "expected AMZN $20,000, got {amzn_val}"
        );
        let fnilx_val = yr0.holdings.get("FNILX").copied().unwrap_or(0.0);
        assert!(
            (fnilx_val - 25_000.0).abs() < 1e-6,
            "expected FNILX $25,000, got {fnilx_val}"
        );
        // amzn_shares should equal the portfolio's AMZN position total_shares
        assert!(
            (yr0.concentrated_shares - 100.0).abs() < 1e-9,
            "expected 100 AMZN shares, got {}",
            yr0.concentrated_shares
        );
    }

    #[test]
    fn project_path_year_zero_treatment_field_is_set() {
        let portfolio = synthetic_portfolio();
        let scenario = synthetic_scenario_2yr_constant_10pct();
        let result = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("should succeed");
        assert_eq!(result[0].treatment, Treatment::Hold);
    }

    // ── HOLD treatment correctness: 5 tests ──────────────────────────────

    #[test]
    fn hold_compounds_non_amzn_by_class_cagr() {
        // FNILX = UsLargeCapIndex, year 1 = 10% → $25,000 * 1.10 = $27,500
        let portfolio = synthetic_portfolio();
        let scenario = synthetic_scenario_2yr_constant_10pct();
        let result = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("should succeed");
        let fnilx_yr1 = result[1].holdings.get("FNILX").copied().unwrap_or(0.0);
        assert!(
            (fnilx_yr1 - 27_500.0).abs() < 1e-6,
            "expected FNILX $27,500 at year 1, got {fnilx_yr1}"
        );
    }

    #[test]
    fn hold_walks_amzn_price_via_scenario() {
        // AMZN = 100 shares @ $200, ConsumerMegaCap +10% → year 1 value = $22,000
        let portfolio = synthetic_portfolio();
        let scenario = synthetic_scenario_2yr_constant_10pct();
        let result = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("should succeed");
        let amzn_yr1 = result[1].holdings.get("AMZN").copied().unwrap_or(0.0);
        assert!(
            (amzn_yr1 - 22_000.0).abs() < 1e-6,
            "expected AMZN $22,000 at year 1, got {amzn_yr1}"
        );
    }

    #[test]
    fn hold_adds_vested_shares_to_amzn_at_net_quantity() {
        // Portfolio with no initial AMZN; 1 vest event of 1000 gross mid-year-1;
        // withhold=0.37 → net = 630 shares added
        let portfolio = Portfolio {
            positions: vec![make_position("FNILX", 1000.0, 25.0)],
            cash: 0.0,
            as_of: d("2026-05-01"),
        };
        let vest_date = "2026-11-01"; // mid-year between as_of and as_of+365
        let vesting = DistributionSchedule {
            events: vec![make_vest_event(vest_date, 1000.0)],
        };
        // Build a 1-year scenario (year 0 placeholder + year 1)
        let zero: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let ten_pct: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.10)).collect();
        let scenario = ScenarioPath {
            id: "t".to_string(),
            yearly_returns: vec![zero, ten_pct],
        };
        let result = project_path(
            &portfolio,
            &vesting,
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("should succeed");
        let amzn_shares_yr1 = result[1].concentrated_shares;
        assert!(
            (amzn_shares_yr1 - 630.0).abs() < 1e-6,
            "expected 630.0 AMZN shares at year 1, got {amzn_shares_yr1}"
        );
    }

    #[test]
    fn hold_amzn_value_includes_vested_shares_at_eoy_price() {
        // Same setup as test 10; assert year-1 AMZN value = 630 × eoy_price
        // eoy_price = walk_price(0.0, ConsumerMegaCap, 1.0) from price=0 initial…
        // Since AMZN not in initial portfolio, amzn_price_t0 = 0.
        // walk_price(0, ConsumerMegaCap, 1.0) = 0 → value = 0.
        // The vest mid-year brings 630 shares at price walk_price(200, _, vest_yrs)
        // Then grows to EOY. Since t0_price=0 for no-AMZN portfolio, EOY value = 630 * walk_price(0, ...) = 0.
        //
        // Instead: use portfolio WITH initial AMZN at $0 shares but vest event brings new shares.
        // More precisely: portfolio has AMZN 0 shares @ 200, so amzn_price_t0 = 200.
        // Vest 1000 gross → 630 net → value at yr1 = 630 × walk_price(200, ConsumerMegaCap, 1.0) = 630 × 220 = 138,600
        let portfolio = Portfolio {
            positions: vec![
                Position {
                    ticker: "AMZN".to_string(),
                    lots: vec![], // 0 shares
                    current_price: 200.0,
                    last_updated: d("2026-05-01"),
                },
                make_position("FNILX", 1000.0, 25.0),
            ],
            cash: 0.0,
            as_of: d("2026-05-01"),
        };
        let vest_date = "2026-11-01";
        let vesting = DistributionSchedule {
            events: vec![make_vest_event(vest_date, 1000.0)],
        };
        let zero: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let ten_pct: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.10)).collect();
        let scenario = ScenarioPath {
            id: "t".to_string(),
            yearly_returns: vec![zero, ten_pct],
        };
        let result = project_path(
            &portfolio,
            &vesting,
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("should succeed");
        // 630 shares × walk_price(200, ConsumerMegaCap, 1.0) = 630 × 220 = 138,600
        let eoy_amzn_price = scenario.walk_price(200.0, AssetClass::ConsumerMegaCap, 1.0);
        let expected_amzn_value = 630.0 * eoy_amzn_price;
        let actual_amzn_value = result[1].holdings.get("AMZN").copied().unwrap_or(0.0);
        assert!(
            (actual_amzn_value - expected_amzn_value).abs() < 1e-6,
            "expected AMZN value {expected_amzn_value}, got {actual_amzn_value}"
        );
    }

    #[test]
    fn hold_cash_is_preserved_across_years() {
        // SPAXX (CashMmf) at 4% return: $5,000 → $5,200 at year 1.
        // result.cash for HOLD sums SPAXX + FCASH** holdings value.
        let portfolio = Portfolio {
            positions: vec![make_position("SPAXX", 5_000.0, 1.0)],
            cash: 0.0,
            as_of: d("2026-05-01"),
        };
        let zero: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let mut four_pct: HashMap<AssetClass, f64> =
            AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        four_pct.insert(AssetClass::CashMmf, 0.04);
        let scenario = ScenarioPath {
            id: "t".to_string(),
            yearly_returns: vec![zero, four_pct],
        };
        let result = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("should succeed");
        let spaxx_yr1 = result[1].holdings.get("SPAXX").copied().unwrap_or(0.0);
        assert!(
            (spaxx_yr1 - 5_200.0).abs() < 1e-6,
            "expected SPAXX $5,200 at year 1, got {spaxx_yr1}"
        );
        // cash field aggregates SPAXX + FCASH**
        assert!(
            (result[1].cash - 5_200.0).abs() < 1e-6,
            "expected result.cash = 5200.0, got {}",
            result[1].cash
        );
    }

    // ── Error paths: 4 tests ─────────────────────────────────────────────

    #[test]
    fn project_path_rejects_unknown_symbol() {
        let portfolio = Portfolio {
            positions: vec![make_position("BOGUS_XYZ", 100.0, 50.0)],
            cash: 0.0,
            as_of: d("2026-05-01"),
        };
        let scenario = synthetic_scenario_2yr_constant_10pct();
        let result = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.37,
        );
        match result {
            Err(e @ ProjectionError::UnknownSymbol(_)) => {
                assert!(e.to_string().contains("BOGUS_XYZ"), "{e}");
            }
            other => panic!("expected UnknownSymbol, got {other:?}"),
        }
    }

    #[test]
    fn unknown_symbol_error_lists_all_unknown_tickers_and_hints_at_symbol_map_toml() {
        let portfolio = Portfolio {
            positions: vec![
                make_position("BOGUS_AAA", 100.0, 50.0),
                make_position("FNILX", 100.0, 25.0),
                make_position("BOGUS_BBB", 10.0, 70.0),
                make_position("BOGUS_CCC", 10.0, 70.0),
            ],
            cash: 0.0,
            as_of: d("2026-05-01"),
        };
        let scenario = synthetic_scenario_2yr_constant_10pct();
        let err = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.22,
        )
        .expect_err("unknown tickers must be rejected");
        assert!(matches!(err, ProjectionError::UnknownSymbol(_)), "{err:?}");
        let msg = err.to_string();
        for t in ["BOGUS_AAA", "BOGUS_BBB", "BOGUS_CCC"] {
            assert!(msg.contains(t), "message must list {t}: {msg}");
        }
        assert!(
            !msg.contains("FNILX"),
            "known tickers must not be listed: {msg}"
        );
        assert!(
            msg.contains("symbol_map.toml"),
            "message must hint at symbol_map.toml: {msg}"
        );
    }

    #[test]
    fn unknown_symbol_is_resolved_by_adding_it_to_the_symbol_map() {
        let portfolio = Portfolio {
            positions: vec![make_position("ZZZQ", 100.0, 50.0)],
            cash: 0.0,
            as_of: d("2026-05-01"),
        };
        let scenario = synthetic_scenario_1yr_const(0.10);
        let mut cfg = ProjectionConfig::new("AMZN");
        assert!(project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.22,
            &cfg
        )
        .is_err());
        cfg.symbol_map = SymbolMap::from_toml_str("[symbols]\nZZZQ = \"industrial\"\n").unwrap();
        let states = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.22,
            &cfg,
        )
        .expect("ZZZQ is mapped now");
        assert!((states[1].holdings["ZZZQ"] - 5_500.0).abs() < 1e-6);
    }

    #[test]
    fn default_class_makes_unmapped_tickers_projectable() {
        let portfolio = Portfolio {
            positions: vec![make_position("ZZZQ", 100.0, 50.0)],
            cash: 0.0,
            as_of: d("2026-05-01"),
        };
        let scenario = synthetic_scenario_1yr_overrides(0.0, &[(AssetClass::Gold, 0.20)]);
        let mut cfg = ProjectionConfig::new("AMZN");
        cfg.symbol_map = SymbolMap::builtin().with_default_class(Some(AssetClass::Gold));
        let states = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.22,
            &cfg,
        )
        .expect("default_class must satisfy unmapped tickers");
        assert!((states[1].holdings["ZZZQ"] - 6_000.0).abs() < 1e-6);
    }

    #[test]
    fn project_path_rejects_missing_concentrated_position_when_vesting_nonempty() {
        // Portfolio without the concentrated symbol (AMZN) + non-empty vesting
        // → MissingConcentrated("AMZN")
        let portfolio = Portfolio {
            positions: vec![make_position("FNILX", 1000.0, 25.0)],
            cash: 0.0,
            as_of: d("2026-05-01"),
        };
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 500.0)],
        };
        let scenario = synthetic_scenario_2yr_constant_10pct();
        let result = project_path(
            &portfolio,
            &vesting,
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.37,
        );
        assert!(
            matches!(result, Err(ProjectionError::MissingConcentrated(ref s)) if s == "AMZN"),
            "expected MissingConcentrated(\"AMZN\"), got {:?}",
            result
        );
    }

    // ── OptionBConfig unit tests (spec §4 defaults) ───────────────────────

    #[test]
    fn option_b_config_default_scalar_values() {
        let cfg = OptionBConfig::default();
        assert_eq!(
            cfg.cost_basis_per_share, 0.0,
            "conservative: whole sale price taxed as gain"
        );
        assert!(
            (cfg.ltcg_rate - 0.238).abs() < 1e-12,
            "20% federal + 3.8% NIIT"
        );
        assert!((cfg.floor_pct - 0.25).abs() < 1e-12);
        assert!(cfg.unwind_schedule.is_empty(), "default schedule is empty");
    }

    #[test]
    fn option_b_config_default_reinvest_weights_are_the_documented_list() {
        let cfg = OptionBConfig::default();
        let tickers: Vec<&str> = cfg
            .reinvest_weights
            .iter()
            .map(|(t, _)| t.as_str())
            .collect();
        assert_eq!(
            tickers,
            vec!["IEF", "TLT", "SCHP", "IAU", "DBMF", "VXUS", "VWO", "VOO", "FZROX"]
        );
        let w = |t: &str| cfg.reinvest_weights.iter().find(|(s, _)| s == t).unwrap().1;
        assert!((w("IEF") - 0.214).abs() < 1e-9);
        assert!((w("VOO") - 0.281).abs() < 1e-9);
        let sum: f64 = cfg.reinvest_weights.iter().map(|(_, w)| w).sum();
        assert!(
            (sum - 1.0).abs() < 0.01,
            "reinvest weights should sum to ~1.0, got {sum}"
        );
    }

    /// Every default reinvest ticker must be classifiable by the builtin map
    /// (otherwise project_path would panic/err on the default OptionB config).
    #[test]
    fn every_default_reinvest_weight_ticker_is_in_builtin_symbol_map() {
        let map = SymbolMap::builtin();
        for (ticker, _) in &OptionBConfig::default().reinvest_weights {
            assert!(
                map.class_of(ticker).is_some(),
                "default reinvest ticker {ticker} missing from builtin SymbolMap"
            );
        }
    }

    #[test]
    fn option_b_config_scheduled_for_year_looks_up_schedule() {
        let cfg = OptionBConfig {
            unwind_schedule: vec![(1, 400.0), (2, 300.0)],
            ..OptionBConfig::default()
        };
        assert_eq!(cfg.scheduled_for_year(1), 400.0);
        assert_eq!(cfg.scheduled_for_year(2), 300.0);
        assert_eq!(cfg.scheduled_for_year(3), 0.0, "year not in schedule → 0.0");
        assert_eq!(cfg.scheduled_for_year(99), 0.0);
    }

    #[test]
    fn option_b_config_scheduled_for_year_is_zero_for_default_config() {
        let cfg = OptionBConfig::default();
        assert_eq!(cfg.scheduled_for_year(1), 0.0);
    }

    #[test]
    fn option_b_config_is_clone_and_partial_eq() {
        let a = OptionBConfig::default();
        assert_eq!(a.clone(), a);
        let b = OptionBConfig {
            floor_pct: 0.5,
            ..a.clone()
        };
        assert_ne!(a, b);
    }

    // ── ProjectionConfig ──────────────────────────────────────────────────

    #[test]
    fn projection_config_new_uses_symbol_builtin_map_and_default_option_b() {
        let cfg = ProjectionConfig::new("MSFT");
        assert_eq!(cfg.concentrated_symbol, "MSFT");
        assert_eq!(cfg.option_b, OptionBConfig::default());
        assert_eq!(
            cfg.symbol_map.class_of("AAPL"),
            SymbolMap::builtin().class_of("AAPL")
        );
        assert_eq!(cfg.symbol_map.class_of("ZZZQ"), None);
    }

    #[test]
    fn projection_config_new_accepts_string_and_str() {
        assert_eq!(
            ProjectionConfig::new(String::from("AAPL")).concentrated_symbol,
            "AAPL"
        );
        assert_eq!(ProjectionConfig::new("AAPL").concentrated_symbol, "AAPL");
    }

    // ── OptionBUnwind vest handling: 3 tests ─────────────────────────────

    /// AMZN=100sh@$200, FNILX=1200sh@$25 ($30k), AAPL=100sh@$100 ($10k).
    /// Vest 1000 net (no withholding) at 2026-11-01 at price $200 → $200k proceeds.
    /// Scenario: all 0% returns. OptionB routes to fixed weights, NOT pro-rata.
    /// FNILX and AAPL are NOT in OPTION_B_REINVEST_WEIGHTS → they get ZERO vest addition.
    /// VOO gets 200k × 0.281 = $56,200; IEF gets 200k × 0.214 = $42,800.
    #[test]
    fn option_b_vest_proceeds_route_to_fixed_weights_not_pro_rata() {
        let portfolio = synthetic_portfolio_with(&[
            ("AMZN", 100.0, 200.0),
            ("FNILX", 1200.0, 25.0),
            ("AAPL", 100.0, 100.0),
        ]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 1000.0)],
        };
        let scenario = synthetic_scenario_1yr_const(0.0);
        let states = project_path(
            &portfolio,
            &vesting,
            Treatment::OptionBUnwind,
            &scenario,
            &default_library(),
            0.0,
        )
        .expect("OptionBUnwind should succeed");
        let fnilx = states[1].holdings.get("FNILX").copied().unwrap_or(0.0);
        let aapl = states[1].holdings.get("AAPL").copied().unwrap_or(0.0);
        let voo = states[1].holdings.get("VOO").copied().unwrap_or(0.0);
        let ief = states[1].holdings.get("IEF").copied().unwrap_or(0.0);
        // Pro-rata would boost FNILX/AAPL; fixed-weight routing must NOT.
        assert!(
            (fnilx - 30_000.0).abs() < 1.0,
            "FNILX must not receive vest proceeds, got {fnilx}"
        );
        assert!(
            (aapl - 10_000.0).abs() < 1.0,
            "AAPL must not receive vest proceeds, got {aapl}"
        );
        assert!(
            (voo - 56_200.0).abs() < 5.0,
            "VOO expected ~$56,200, got {voo}"
        );
        assert!(
            (ief - 42_800.0).abs() < 5.0,
            "IEF expected ~$42,800, got {ief}"
        );
    }

    /// Portfolio with ZERO IEF/TLT/etc. After a vest event those sleeves must appear.
    #[test]
    fn option_b_vest_creates_new_holdings_for_unfilled_sleeves() {
        let portfolio = synthetic_portfolio_with(&[("AMZN", 100.0, 200.0), ("FNILX", 400.0, 25.0)]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 500.0)],
        };
        let scenario = synthetic_scenario_1yr_const(0.0);
        let states = project_path(
            &portfolio,
            &vesting,
            Treatment::OptionBUnwind,
            &scenario,
            &default_library(),
            0.0,
        )
        .expect("OptionBUnwind should succeed");
        // Each OPTION_B_REINVEST_WEIGHTS ticker must appear in year-1 holdings
        for ticker in &[
            "IEF", "TLT", "SCHP", "IAU", "DBMF", "VXUS", "VWO", "VOO", "FZROX",
        ] {
            assert!(
                states[1].holdings.contains_key(*ticker),
                "{ticker} should be created as a new holding after OptionB vest"
            );
            assert!(
                states[1].holdings[*ticker] > 0.0,
                "{ticker} value should be > 0 after receiving vest proceeds"
            );
        }
    }

    /// IEF return = 5%. Vest 1000 net at 2026-11-01 → $200k proceeds.
    /// yrs_remaining = 181/365 ≈ 0.49589.
    /// IEF addition = 200k × 0.214 × (1.05)^0.49589 ≈ 43,856 (tolerance ±50).
    #[test]
    fn option_b_vest_compounds_proceeds_partial_year_at_recipient_cagr() {
        let portfolio = synthetic_portfolio_with(&[("AMZN", 100.0, 200.0), ("FNILX", 400.0, 25.0)]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 1000.0)],
        };
        // IEF is UsTreasuryIntermediate; set that class to 5%, everything else 0%
        use crate::projection::monte_carlo::AssetClass;
        let scenario =
            synthetic_scenario_1yr_overrides(0.0, &[(AssetClass::UsTreasuryIntermediate, 0.05)]);
        let states = project_path(
            &portfolio,
            &vesting,
            Treatment::OptionBUnwind,
            &scenario,
            &default_library(),
            0.0,
        )
        .expect("OptionBUnwind should succeed");
        let yrs_rem = 181.0_f64 / 365.0;
        let expected_ief = 200_000.0 * 0.214 * 1.05_f64.powf(yrs_rem);
        let actual_ief = states[1].holdings.get("IEF").copied().unwrap_or(0.0);
        assert!(
            (actual_ief - expected_ief).abs() < 50.0,
            "IEF expected ≈{expected_ief:.0}, got {actual_ief:.0}"
        );
    }

    // ── OptionBUnwind existing-AMZN unwind: 5 tests ──────────────────────

    /// Build a portfolio with high AMZN concentration:
    ///   AMZN: amzn_shares @ amzn_price, OTHER: FNILX at other_value / $25 per share.
    fn synthetic_portfolio_with_amzn_dominant(
        amzn_shares: f64,
        amzn_price: f64,
        other_value: f64,
    ) -> Portfolio {
        let fnilx_shares = (other_value / 25.0).ceil();
        synthetic_portfolio_with(&[
            ("AMZN", amzn_shares, amzn_price),
            ("FNILX", fnilx_shares, 25.0),
        ])
    }

    /// AMZN=10000sh@$200 ($2M), FNILX=$1M — AMZN/NAV ≈ 67%, well above 35% floor.
    /// Year-1 schedule = 2000 shares. k_max >> 2000 → full schedule executes.
    /// Expected: concentrated_shares[1] = 8000.
    #[test]
    fn option_b_existing_amzn_sells_per_schedule_when_above_floor() {
        let portfolio = synthetic_portfolio_with_amzn_dominant(10_000.0, 200.0, 1_000_000.0);
        let scenario = synthetic_scenario_1yr_const(0.0);
        let states = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("OptionBUnwind should succeed");
        assert!(
            (states[1].concentrated_shares - 8_000.0).abs() < 0.5,
            "expected 8000 AMZN shares after year-1 unwind, got {}",
            states[1].concentrated_shares
        );
    }

    /// AMZN=200sh@$200 ($40k), FNILX=$74k — AMZN/NAV ≈ 35.1%, just above floor.
    /// k_max ≈ 0.514 shares; nothing significant is sold.
    /// Assert amzn_shares after year 1 ≥ 199.5 (effectively no sale).
    #[test]
    fn option_b_existing_amzn_throttled_by_floor_when_at_boundary() {
        let portfolio = synthetic_portfolio_with_amzn_dominant(200.0, 200.0, 74_000.0);
        let scenario = synthetic_scenario_1yr_const(0.0);
        let states = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("OptionBUnwind should succeed");
        assert!(
            states[1].concentrated_shares >= 199.5,
            "expected near-zero sale at floor boundary; amzn_shares = {}",
            states[1].concentrated_shares
        );
    }

    /// AMZN=10000sh@$200 (only holding). Year 1 sells 2000sh (test_config basis $100, LTCG 20%).
    /// avg_price = 200; gain = 100; tax = 100×0.20 = 20; net_per_share = 180;
    /// net_proceeds = 2000 × 180 = 360,000.
    /// Those proceeds route to the reinvest weights at 0.5y compound (0% returns).
    /// IEF portion = 360,000 × 0.214 = 77,040 (0% return so factor = 1).
    #[test]
    fn option_b_after_tax_proceeds_redistribute_to_reinvest_weights() {
        let portfolio = synthetic_portfolio_with(&[("AMZN", 10_000.0, 200.0)]);
        let scenario = synthetic_scenario_1yr_const(0.0);
        let states = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("OptionBUnwind should succeed");
        // 2000 shares sold at avg_price=200; net_per_share = 200 - 100*0.20 = 180
        let net_proceeds = 2000.0 * (200.0 - (200.0 - 100.0) * 0.20);
        let expected_ief = net_proceeds * 0.214;
        let actual_ief = states[1].holdings.get("IEF").copied().unwrap_or(0.0);
        assert!(
            (actual_ief - expected_ief).abs() < 50.0,
            "IEF expected ≈{expected_ief:.0}, got {actual_ief:.0}"
        );
        // Reinvest tickers must all be present and positive
        for ticker in &["IEF", "TLT", "VOO", "VXUS"] {
            assert!(
                states[1].holdings.get(*ticker).copied().unwrap_or(0.0) > 0.0,
                "{ticker} should receive proceeds from AMZN unwind"
            );
        }
    }

    /// 5-year scenario, no vests. Schedule: year 1=2000, year 2=1500, years 3-5=0.
    /// AMZN shares should decrease in years 1 and 2 but be flat years 3-5.
    #[test]
    fn option_b_year_3_plus_no_existing_sales() {
        let portfolio = synthetic_portfolio_with_amzn_dominant(10_000.0, 200.0, 1_000_000.0);
        let zero: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let flat: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let scenario = ScenarioPath {
            id: "5yr_flat".to_string(),
            yearly_returns: vec![
                zero,
                flat.clone(),
                flat.clone(),
                flat.clone(),
                flat.clone(),
                flat,
            ],
        };
        let states = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("OptionBUnwind should succeed");
        // Year 1 and 2: shares decrease
        assert!(
            states[1].concentrated_shares < states[0].concentrated_shares,
            "year 1 should sell shares"
        );
        assert!(
            states[2].concentrated_shares < states[1].concentrated_shares,
            "year 2 should sell shares"
        );
        // Years 3-5: schedule is 0 → no further sales
        assert!(
            (states[3].concentrated_shares - states[2].concentrated_shares).abs() < 0.5,
            "year 3 must be flat"
        );
        assert!(
            (states[4].concentrated_shares - states[2].concentrated_shares).abs() < 0.5,
            "year 4 must be flat"
        );
        assert!(
            (states[5].concentrated_shares - states[2].concentrated_shares).abs() < 0.5,
            "year 5 must be flat"
        );
    }

    /// Same 5-year setup. Assert shares monotonically non-increasing through the path.
    #[test]
    fn option_b_amzn_shares_monotonically_decrease_during_unwind_window() {
        let portfolio = synthetic_portfolio_with_amzn_dominant(10_000.0, 200.0, 1_000_000.0);
        let zero: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let flat: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let scenario = ScenarioPath {
            id: "5yr_mono".to_string(),
            yearly_returns: vec![
                zero,
                flat.clone(),
                flat.clone(),
                flat.clone(),
                flat.clone(),
                flat,
            ],
        };
        let states = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("OptionBUnwind should succeed");
        for i in 1..states.len() {
            assert!(
                states[i].concentrated_shares <= states[i - 1].concentrated_shares + 1e-9,
                "amzn_shares must be non-increasing: year {} ({}) > year {} ({})",
                i,
                states[i].concentrated_shares,
                i - 1,
                states[i - 1].concentrated_shares
            );
        }
        // Years 3-5 flat with year 2
        assert!((states[3].concentrated_shares - states[2].concentrated_shares).abs() < 0.5);
        assert!((states[4].concentrated_shares - states[2].concentrated_shares).abs() < 0.5);
        assert!((states[5].concentrated_shares - states[2].concentrated_shares).abs() < 0.5);
    }

    // ── OptionBUnwind integration smoke: 1 test ───────────────────────────

    /// 2-year, 10% scenario + 1 vest in year 1. Smoke-test that the
    /// full path runs, NAV > 0, and AMZN concentration is below 50% by year 2
    /// (demonstrating the unwind has reduced concentration from ~67%).
    #[test]
    fn option_b_2yr_synthetic_full_walk() {
        // AMZN=10000sh@$200 ($2M) + FNILX=$1M. AMZN/NAV ≈ 67% at year 0.
        // 1 vest: 2026-11-01, 1000 gross, withhold=0.37 → 630 net shares.
        // All returns 10%. 2-year horizon.
        let portfolio = synthetic_portfolio_with_amzn_dominant(10_000.0, 200.0, 1_000_000.0);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 1000.0)],
        };
        let scenario = synthetic_scenario_2yr_constant_10pct();
        let states = project_path(
            &portfolio,
            &vesting,
            Treatment::OptionBUnwind,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("OptionBUnwind 2yr full walk should succeed");
        assert_eq!(states.len(), 3, "expected 3 states (year 0, 1, 2)");
        assert!(states[2].nav() > 0.0, "year-2 NAV must be positive");
        // Unwind (2000 shares in yr1, 1500 in yr2) plus vest proceeds routed off AMZN
        // should drive AMZN below 50% of NAV.
        assert!(
            states[2].concentrated_pct_of_nav() < 50.0,
            "AMZN concentration should be below 50% by year 2, got {:.1}%",
            states[2].concentrated_pct_of_nav()
        );
        // Sanity: year-2 shares = 10000 + 630 - 2000 - 1500 = 7130 (subject to k_max cap)
        assert!(
            states[2].concentrated_shares < 10_000.0,
            "shares must have decreased from unwind"
        );
    }

    // ── Integration smoke: 1 test ─────────────────────────────────────────

    #[test]
    fn project_path_hold_2yr_synthetic_scenario_full_walk() {
        // 3-position portfolio: AMZN 100sh@$200, FNILX 1000sh@$25, SPAXX 5000sh@$1
        // 1 vest event: 2026-11-01, 1000 gross AMZN shares, withhold=0.37 → 630 net
        // 2-year scenario: year 1 all +10%, year 2 all +20%
        //
        // Hand-computed expected year-2 NAV:
        //
        // Initial state (year 0):
        //   AMZN: 100sh, FNILX: $25,000, SPAXX: $5,000
        //
        // Year 1:
        //   FNILX: $25,000 * 1.10 = $27,500
        //   SPAXX: $5,000 * 1.10 = $5,500  (CashMmf also +10% in this scenario)
        //   AMZN: 100 initial shares grow to year-end price = walk_price(200, ConsumerMegaCap, 1.0) = 220
        //     + 630 vested shares (vest mid-year-1, both initial and vested use EOY price)
        //     Total: (100 + 630) * 220 = 730 * 220 = $160,600
        //   Year-1 NAV = 160,600 + 27,500 + 5,500 = $193,600
        //
        // Year 2:
        //   FNILX: $27,500 * 1.20 = $33,000
        //   SPAXX: $5,500 * 1.20 = $6,600
        //   AMZN: 730 shares * walk_price(200, ConsumerMegaCap, 2.0)
        //     walk_price(200, _, 2.0) = 200 * 1.10 * 1.20 = 200 * 1.32 = 264.0
        //     730 * 264 = $192,720
        //   Year-2 NAV = 192,720 + 33,000 + 6,600 = $232,320

        let portfolio = Portfolio {
            positions: vec![
                make_position("AMZN", 100.0, 200.0),
                make_position("FNILX", 1000.0, 25.0),
                make_position("SPAXX", 5_000.0, 1.0),
            ],
            cash: 0.0,
            as_of: d("2026-05-01"),
        };
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 1000.0)],
        };
        let zero: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let yr1: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.10)).collect();
        let yr2: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.20)).collect();
        let scenario = ScenarioPath {
            id: "smoke".to_string(),
            yearly_returns: vec![zero, yr1, yr2],
        };

        let result = project_path(
            &portfolio,
            &vesting,
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("should succeed with valid inputs");

        assert_eq!(result.len(), 3);

        // Year-1 NAV
        let yr1_nav = result[1].nav();
        assert!(
            (yr1_nav - 193_600.0).abs() < 1e-3,
            "expected year-1 NAV = 193,600, got {yr1_nav}"
        );

        // Year-2 NAV
        let yr2_nav = result[2].nav();
        assert!(
            (yr2_nav - 232_320.0).abs() < 1e-3,
            "expected year-2 NAV = 232,320, got {yr2_nav}"
        );
    }

    // ── SellReinvest shape: 2 tests ───────────────────────────────────────

    #[test]
    fn sell_reinvest_returns_horizon_plus_one_states() {
        // Basic call returns Ok with right length (same shape contract as Hold).
        let portfolio = synthetic_portfolio();
        let scenario = synthetic_scenario_2yr_constant_10pct();
        let result = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::SellReinvest,
            &scenario,
            &default_library(),
            0.37,
        );
        let states = result.expect("SellReinvest should return Ok");
        assert_eq!(
            states.len(),
            3,
            "expected horizon+1=3 states, got {}",
            states.len()
        );
    }

    #[test]
    fn sell_reinvest_year_zero_state_matches_initial_portfolio() {
        // Year-0 state must be identical regardless of treatment.
        let portfolio = synthetic_portfolio();
        let scenario = synthetic_scenario_2yr_constant_10pct();
        let result = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::SellReinvest,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("should succeed");
        let yr0 = &result[0];
        assert!(
            (yr0.holdings.get("AMZN").copied().unwrap_or(0.0) - 20_000.0).abs() < 1e-6,
            "year-0 AMZN value mismatch"
        );
        assert!(
            (yr0.holdings.get("FNILX").copied().unwrap_or(0.0) - 25_000.0).abs() < 1e-6,
            "year-0 FNILX value mismatch"
        );
        assert!(
            (yr0.concentrated_shares - 100.0).abs() < 1e-9,
            "year-0 amzn_shares mismatch"
        );
        assert_eq!(yr0.treatment, Treatment::SellReinvest);
    }

    // ── SellReinvest AMZN handling: 2 tests ──────────────────────────────

    #[test]
    fn sell_reinvest_does_not_add_vested_shares_to_amzn() {
        // AMZN=100sh@$200, vest 1000 gross mid-year-1 with withhold=0.0.
        // SellReinvest sells the net shares; amzn_shares stays at 100.
        // Year-1 AMZN value = 100 × walk_price(200, ConsumerMegaCap, 1.0).
        // Scenario: year 1 ConsumerMegaCap=10%.
        let portfolio = synthetic_portfolio_with(&[("AMZN", 100.0, 200.0), ("FNILX", 400.0, 25.0)]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 1000.0)],
        };
        let scenario = synthetic_scenario_1yr_const(0.10);
        let result = project_path(
            &portfolio,
            &vesting,
            Treatment::SellReinvest,
            &scenario,
            &default_library(),
            0.0,
        )
        .expect("should succeed");
        // amzn_shares must be unchanged at 100
        assert!(
            (result[1].concentrated_shares - 100.0).abs() < 1e-9,
            "expected amzn_shares=100, got {}",
            result[1].concentrated_shares
        );
        // AMZN value = 100 × 200 × 1.10 = 22,000
        let amzn_val = result[1].holdings.get("AMZN").copied().unwrap_or(0.0);
        assert!(
            (amzn_val - 22_000.0).abs() < 1e-6,
            "expected AMZN $22,000, got {amzn_val}"
        );
    }

    #[test]
    fn sell_reinvest_amzn_compounds_normally_without_vests() {
        // No vesting — AMZN still walks price correctly.
        // AMZN=100sh@$200, no vests, ConsumerMegaCap +10%.
        // Year-1 AMZN value = 100 × 220 = $22,000.
        let portfolio = synthetic_portfolio_with(&[("AMZN", 100.0, 200.0), ("FNILX", 400.0, 25.0)]);
        let scenario = synthetic_scenario_1yr_const(0.10);
        let result = project_path(
            &portfolio,
            &empty_vesting(),
            Treatment::SellReinvest,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("should succeed");
        let amzn_val = result[1].holdings.get("AMZN").copied().unwrap_or(0.0);
        assert!(
            (amzn_val - 22_000.0).abs() < 1e-6,
            "expected AMZN $22,000, got {amzn_val}"
        );
        assert!(
            (result[1].concentrated_shares - 100.0).abs() < 1e-9,
            "expected 100 shares, got {}",
            result[1].concentrated_shares
        );
    }

    // ── SellReinvest pro-rata distribution: 4 tests ───────────────────────

    #[test]
    fn sell_reinvest_distributes_proceeds_to_single_non_amzn_holding() {
        // AMZN=100sh@$200 + FNILX=$10,000 (400sh@$25).
        // Vest mid-year-1 (2026-11-01): 1000 gross, withhold=0.0.
        // Scenario: year 1 all 0% (so AMZN price at vest = 200, proceeds = $200,000).
        // FNILX post-step1 = $10,000 (0% return). Gets all proceeds at 0% for yrs_remaining.
        // Expected: FNILX yr1 = $210,000, AMZN = $20,000, total NAV = $230,000.
        let portfolio = synthetic_portfolio_with(&[("AMZN", 100.0, 200.0), ("FNILX", 400.0, 25.0)]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 1000.0)],
        };
        let scenario = synthetic_scenario_1yr_const(0.0);
        let result = project_path(
            &portfolio,
            &vesting,
            Treatment::SellReinvest,
            &scenario,
            &default_library(),
            0.0,
        )
        .expect("should succeed");
        let fnilx_yr1 = result[1].holdings.get("FNILX").copied().unwrap_or(0.0);
        let amzn_yr1 = result[1].holdings.get("AMZN").copied().unwrap_or(0.0);
        assert!(
            (fnilx_yr1 - 210_000.0).abs() < 1e-3,
            "expected FNILX $210,000, got {fnilx_yr1}"
        );
        assert!(
            (amzn_yr1 - 20_000.0).abs() < 1e-6,
            "expected AMZN $20,000, got {amzn_yr1}"
        );
        let nav = result[1].nav();
        assert!(
            (nav - 230_000.0).abs() < 1e-3,
            "expected total NAV $230,000, got {nav}"
        );
    }

    #[test]
    fn sell_reinvest_distributes_proceeds_pro_rata_across_two_non_amzn_holdings() {
        // AMZN=100sh@$200 + FNILX=1200sh@$25 ($30,000) + AAPL=100sh@$100 ($10,000).
        // FNILX:AAPL weight = 3:1.  Vest mid-year-1 (2026-11-01): 1000 gross, withhold=0.0.
        // Scenario: year 1 all 0% (proceeds = 1000 × 200 = $200,000).
        // POST-COMPOUND weights: FNILX=$30,000 (75%), AAPL=$10,000 (25%).
        // Expected: FNILX yr1 = $180,000, AAPL yr1 = $60,000, AMZN yr1 = $20,000, NAV = $260,000.
        let portfolio = synthetic_portfolio_with(&[
            ("AMZN", 100.0, 200.0),
            ("FNILX", 1200.0, 25.0),
            ("AAPL", 100.0, 100.0),
        ]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 1000.0)],
        };
        let scenario = synthetic_scenario_1yr_const(0.0);
        let result = project_path(
            &portfolio,
            &vesting,
            Treatment::SellReinvest,
            &scenario,
            &default_library(),
            0.0,
        )
        .expect("should succeed");
        let fnilx_yr1 = result[1].holdings.get("FNILX").copied().unwrap_or(0.0);
        let aapl_yr1 = result[1].holdings.get("AAPL").copied().unwrap_or(0.0);
        let amzn_yr1 = result[1].holdings.get("AMZN").copied().unwrap_or(0.0);
        assert!(
            (fnilx_yr1 - 180_000.0).abs() < 1e-3,
            "expected FNILX $180,000, got {fnilx_yr1}"
        );
        assert!(
            (aapl_yr1 - 60_000.0).abs() < 1e-3,
            "expected AAPL $60,000, got {aapl_yr1}"
        );
        assert!(
            (amzn_yr1 - 20_000.0).abs() < 1e-6,
            "expected AMZN $20,000, got {amzn_yr1}"
        );
        let nav = result[1].nav();
        assert!(
            (nav - 260_000.0).abs() < 1e-3,
            "expected NAV $260,000, got {nav}"
        );
    }

    #[test]
    fn sell_reinvest_parks_proceeds_in_synthetic_cash_when_no_non_concentrated_sleeve() {
        // AMZN-only portfolio. Vest 1000 net shares mid-year-1.
        // No other holdings and no CashMmf holding → proceeds are parked in a
        // synthetic "CASH" holding (classed CashMmf).
        // Scenario: all 0% → price at vest = 200, proceeds = 1000 × 200 = $200,000.
        let portfolio = synthetic_portfolio_with(&[("AMZN", 100.0, 200.0)]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 1000.0)],
        };
        let scenario = synthetic_scenario_1yr_const(0.0);
        let result = project_path(
            &portfolio,
            &vesting,
            Treatment::SellReinvest,
            &scenario,
            &default_library(),
            0.0,
        )
        .expect("should succeed");
        assert!(
            !result[0].holdings.contains_key("CASH"),
            "CASH should not exist in year-0 holdings"
        );
        let cash_yr1 = result[1].holdings.get("CASH").copied().unwrap_or(0.0);
        assert!(
            (cash_yr1 - 200_000.0).abs() < 1e-3,
            "expected synthetic CASH $200,000 (proceeds fallback), got {cash_yr1}"
        );
        assert!(
            !result[1].holdings.contains_key("SPAXX"),
            "literal SPAXX must no longer be used as the fallback"
        );
        // cash detection is class-based: synthetic CASH counts as cash.
        assert!(
            (result[1].cash - 200_000.0).abs() < 1e-3,
            "cash = {}",
            result[1].cash
        );
    }

    #[test]
    fn sell_reinvest_fallback_uses_first_cash_mmf_holding_when_sleeve_is_empty() {
        // SPAXX exists but has zero value; AMZN is the only valued holding.
        // The no-sleeve fallback must park proceeds in the existing CashMmf
        // holding (SPAXX), not create a synthetic CASH entry.
        let portfolio = synthetic_portfolio_with(&[("AMZN", 100.0, 200.0), ("SPAXX", 0.0, 1.0)]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 1000.0)],
        };
        let scenario = synthetic_scenario_1yr_const(0.0);
        let result = project_path(
            &portfolio,
            &vesting,
            Treatment::SellReinvest,
            &scenario,
            &default_library(),
            0.0,
        )
        .expect("should succeed");
        let spaxx_yr1 = result[1].holdings.get("SPAXX").copied().unwrap_or(0.0);
        assert!((spaxx_yr1 - 200_000.0).abs() < 1e-3, "SPAXX = {spaxx_yr1}");
        assert!(!result[1].holdings.contains_key("CASH"));
    }

    #[test]
    fn sell_reinvest_synthetic_cash_compounds_at_cash_mmf_rate_in_later_years() {
        // Year 1: proceeds parked in synthetic CASH ($200,000, 0% year).
        // Year 2: CashMmf returns 5% → CASH must compound to $210,000 (and the
        // projection must not fail on the synthetic ticker).
        let portfolio = synthetic_portfolio_with(&[("AMZN", 100.0, 200.0)]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 1000.0)],
        };
        let zero: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let mut yr2 = zero.clone();
        yr2.insert(AssetClass::CashMmf, 0.05);
        let scenario = ScenarioPath {
            id: "cash2y".to_string(),
            yearly_returns: vec![zero.clone(), zero, yr2],
        };
        let result = project_path(
            &portfolio,
            &vesting,
            Treatment::SellReinvest,
            &scenario,
            &default_library(),
            0.0,
        )
        .expect("two-year path with synthetic CASH should succeed");
        let cash_yr2 = result[2].holdings.get("CASH").copied().unwrap_or(0.0);
        assert!((cash_yr2 - 210_000.0).abs() < 1e-3, "CASH yr2 = {cash_yr2}");
    }

    #[test]
    fn sell_reinvest_partial_year_compounds_proceeds_to_year_end() {
        // AMZN=100sh@$200 + FNILX=400sh@$25 ($10,000).
        // Vest 2026-08-01 (92 days after as_of=2026-05-01): 1000 gross, withhold=0.0.
        // Scenario: year 1 ConsumerMegaCap=0% (price at vest = 200), UsLargeCapIndex=12%.
        // year_end = 2027-05-01, yrs_remaining = (2027-05-01 - 2026-08-01).days / 365.0 = 273/365.
        // proceeds = 1000 × 200 = $200,000; FNILX post-step1 = 10,000 × 1.12 = $11,200.
        // Vest addition = $200,000 × (1.12)^(273/365).
        // FNILX yr1 ≈ $11,200 + $217,691.96 ≈ $228,891.96. Assert within 0.5%.
        let portfolio = synthetic_portfolio_with(&[("AMZN", 100.0, 200.0), ("FNILX", 400.0, 25.0)]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-08-01", 1000.0)],
        };
        let scenario =
            synthetic_scenario_1yr_overrides(0.0, &[(AssetClass::UsLargeCapIndex, 0.12)]);
        let result = project_path(
            &portfolio,
            &vesting,
            Treatment::SellReinvest,
            &scenario,
            &default_library(),
            0.0,
        )
        .expect("should succeed");
        // yrs_remaining = 273 / 365.0
        let yrs_rem = 273.0_f64 / 365.0;
        let expected_fnilx = 11_200.0 + 200_000.0 * 1.12_f64.powf(yrs_rem);
        let actual_fnilx = result[1].holdings.get("FNILX").copied().unwrap_or(0.0);
        let tolerance = expected_fnilx * 0.005; // 0.5%
        assert!(
            (actual_fnilx - expected_fnilx).abs() < tolerance,
            "expected FNILX ≈ {expected_fnilx:.2} (±0.5%), got {actual_fnilx:.2}"
        );
    }

    // ── SellReinvest cash semantics: 1 test ──────────────────────────────

    #[test]
    fn sell_reinvest_cash_field_includes_post_distribution_spaxx() {
        // Portfolio: AMZN=100sh@$200 + SPAXX=5000sh@$1 ($5,000).
        // Vest mid-year-1 (2026-11-01): 1000 net, 0% returns.
        // SPAXX is the sole non-AMZN holding, so it receives all proceeds.
        // Expected: SPAXX yr1 = $5,000 + $200,000 = $205,000.
        // cash field must equal SPAXX value at year 1.
        let portfolio =
            synthetic_portfolio_with(&[("AMZN", 100.0, 200.0), ("SPAXX", 5_000.0, 1.0)]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 1000.0)],
        };
        let scenario = synthetic_scenario_1yr_const(0.0);
        let result = project_path(
            &portfolio,
            &vesting,
            Treatment::SellReinvest,
            &scenario,
            &default_library(),
            0.0,
        )
        .expect("should succeed");
        let spaxx_yr1 = result[1].holdings.get("SPAXX").copied().unwrap_or(0.0);
        assert!(
            (spaxx_yr1 - 205_000.0).abs() < 1e-3,
            "expected SPAXX $205,000, got {spaxx_yr1}"
        );
        assert!(
            (result[1].cash - 205_000.0).abs() < 1e-3,
            "expected cash = $205,000, got {}",
            result[1].cash
        );
    }

    // ── SellReinvest integration smoke: 1 test ────────────────────────────

    #[test]
    fn sell_reinvest_2yr_synthetic_full_walk() {
        // Portfolio: AMZN=100sh@$200, FNILX=800sh@$25 ($20,000), SPAXX=5000sh@$1 ($5,000).
        // Vest 2026-11-01: 1000 gross, withhold=0.37 → 630 net.
        // Year 1: ConsumerMegaCap=10%, UsLargeCapIndex=8%, CashMmf=4% (others=0%).
        // Year 2: ConsumerMegaCap=15%, UsLargeCapIndex=10%, CashMmf=3% (others=0%, no vests).
        //
        // Year 1 hand calculation:
        //   Step 1 (full-year compound): FNILX = 20,000 × 1.08 = 21,600; SPAXX = 5,000 × 1.04 = 5,200.
        //   Vest: amzn_price_at_vest = walk_price(200, ConsumerMegaCap, 184/365)
        //         = 200 × 1.10^(184/365) ≈ 209.8439
        //         proceeds = 630 × 209.8439 ≈ 132,201.69
        //   sleeve_total = 21,600 + 5,200 = 26,800
        //   yrs_remaining = 181/365 ≈ 0.495890
        //   FNILX_addn = 132,201.69 × (21600/26800) × 1.08^0.495890
        //   SPAXX_addn = 132,201.69 × (5200/26800) × 1.04^0.495890
        //   amzn_shares unchanged = 100; AMZN yr1 = 100 × 200 × 1.10 = 22,000.
        //   NAV yr1 ≈ FNILX_yr1 + SPAXX_yr1 + 22,000.
        //
        // Year 2 (no vests):
        //   FNILX yr2 = FNILX_yr1 × 1.10
        //   SPAXX yr2 = SPAXX_yr1 × 1.03
        //   amzn_shares = 100; AMZN yr2 = 100 × walk_price(200, ConsumerMegaCap, 2.0)
        //                             = 100 × 200 × 1.10 × 1.15 = 25,300.
        //
        // Python reference gives: NAV yr1 ≈ 185,650.48, NAV yr2 ≈ 203,120.69.
        // Assert within 0.1%.
        let portfolio = synthetic_portfolio_with(&[
            ("AMZN", 100.0, 200.0),
            ("FNILX", 800.0, 25.0),
            ("SPAXX", 5_000.0, 1.0),
        ]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event("2026-11-01", 1000.0)],
        };
        let zero: HashMap<AssetClass, f64> = AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        let mut yr1_map: HashMap<AssetClass, f64> =
            AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        yr1_map.insert(AssetClass::ConsumerMegaCap, 0.10);
        yr1_map.insert(AssetClass::UsLargeCapIndex, 0.08);
        yr1_map.insert(AssetClass::CashMmf, 0.04);
        let mut yr2_map: HashMap<AssetClass, f64> =
            AssetClass::iter_all().map(|c| (c, 0.0)).collect();
        yr2_map.insert(AssetClass::ConsumerMegaCap, 0.15);
        yr2_map.insert(AssetClass::UsLargeCapIndex, 0.10);
        yr2_map.insert(AssetClass::CashMmf, 0.03);
        let scenario = ScenarioPath {
            id: "smoke_sr".to_string(),
            yearly_returns: vec![zero, yr1_map, yr2_map],
        };
        let result = project_path(
            &portfolio,
            &vesting,
            Treatment::SellReinvest,
            &scenario,
            &default_library(),
            0.37,
        )
        .expect("should succeed");
        assert_eq!(result.len(), 3);
        let nav_yr1 = result[1].nav();
        let nav_yr2 = result[2].nav();
        // Python-computed: 185,650.48 and 203,120.69 (0.1% tolerance)
        assert!(
            (nav_yr1 - 185_650.48).abs() < nav_yr1 * 0.001,
            "expected yr1 NAV ≈ 185,650.48, got {nav_yr1:.2}"
        );
        assert!(
            (nav_yr2 - 203_120.69).abs() < nav_yr2 * 0.001,
            "expected yr2 NAV ≈ 203,120.69, got {nav_yr2:.2}"
        );
    }

    // ── Treatment: 3 tests ────────────────────────────────────────────────

    #[test]
    fn treatment_round_trips_through_json() {
        for variant in [
            Treatment::Hold,
            Treatment::SellReinvest,
            Treatment::OptionBUnwind,
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            let back: Treatment = serde_json::from_str(&json).unwrap();
            assert_eq!(variant, back);
        }
    }

    #[test]
    fn treatment_implements_copy_and_eq() {
        let a = Treatment::Hold;
        let b = a; // Copy
        assert_eq!(a, b);
    }

    #[test]
    fn treatment_can_be_used_as_hashmap_key() {
        let mut m: HashMap<Treatment, i32> = HashMap::new();
        m.insert(Treatment::Hold, 1);
        assert_eq!(m[&Treatment::Hold], 1);
    }

    // ── PortfolioState helpers (no struct literals: built from project_path) ──

    /// Year-0 state from a trivial AMZN portfolio with `holdings` replaced.
    fn make_state(holdings: BTreeMap<String, f64>) -> PortfolioState {
        let mut st = project_path(
            &synthetic_portfolio(),
            &empty_vesting(),
            Treatment::Hold,
            &synthetic_scenario_2yr_constant_10pct(),
            &default_library(),
            0.22,
        )
        .expect("trivial projection")[0]
            .clone();
        st.holdings = holdings;
        st
    }

    #[test]
    fn nav_sums_holdings_values() {
        let holdings = BTreeMap::from([
            ("AMZN".to_string(), 1000.0),
            ("AAPL".to_string(), 500.0),
            ("FNILX".to_string(), 200.0),
        ]);
        assert_eq!(make_state(holdings).nav(), 1700.0);
    }

    #[test]
    fn nav_returns_zero_for_empty_holdings() {
        assert_eq!(make_state(BTreeMap::new()).nav(), 0.0);
    }

    #[test]
    fn nav_includes_negative_values() {
        let holdings = BTreeMap::from([("AMZN".to_string(), 1000.0), ("ERR".to_string(), -100.0)]);
        assert_eq!(make_state(holdings).nav(), 900.0);
    }

    // ── PortfolioState::concentrated_value / concentrated_pct_of_nav ──────

    #[test]
    fn concentrated_value_returns_value_when_present() {
        let holdings = BTreeMap::from([("AMZN".to_string(), 1500.0)]);
        assert_eq!(make_state(holdings).concentrated_value(), 1500.0);
    }

    #[test]
    fn concentrated_value_returns_zero_when_absent() {
        let holdings = BTreeMap::from([("AAPL".to_string(), 800.0)]);
        assert_eq!(make_state(holdings).concentrated_value(), 0.0);
    }

    #[test]
    fn concentrated_pct_returns_correct_percentage_for_known_distribution() {
        // 1500 / (1500 + 1000) = 60 %
        let holdings =
            BTreeMap::from([("AMZN".to_string(), 1500.0), ("FNILX".to_string(), 1000.0)]);
        let pct = make_state(holdings).concentrated_pct_of_nav();
        assert!((pct - 60.0_f64).abs() < 1e-9, "expected 60, got {pct}");
    }

    #[test]
    fn concentrated_pct_returns_zero_when_nav_is_zero() {
        assert_eq!(make_state(BTreeMap::new()).concentrated_pct_of_nav(), 0.0);
    }

    #[test]
    fn concentrated_pct_returns_zero_when_concentrated_symbol_absent() {
        let holdings = BTreeMap::from([("AAPL".to_string(), 500.0)]);
        assert_eq!(make_state(holdings).concentrated_pct_of_nav(), 0.0);
    }

    // ── Round-trip: 1 test ────────────────────────────────────────────────

    #[test]
    fn portfolio_state_round_trips_through_json() {
        let mut state = make_state(BTreeMap::from([
            ("AMZN".to_string(), 2_500_000.0),
            ("AAPL".to_string(), 100_000.0),
        ]));
        state.year = 3;
        state.concentrated_shares = 8000.0;
        state.cash = 50_000.0;
        state.treatment = Treatment::SellReinvest;
        let json = serde_json::to_string(&state).unwrap();
        let back: PortfolioState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, back);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Generalized concentrated symbol (spec §4): MSFT walks as tech_mega_cap
    // ═══════════════════════════════════════════════════════════════════════

    fn msft_config() -> ProjectionConfig {
        let mut cfg = ProjectionConfig::new("MSFT");
        cfg.option_b = OptionBConfig {
            cost_basis_per_share: 100.0,
            ltcg_rate: 0.20,
            floor_pct: 0.35,
            unwind_schedule: vec![(1, 2000.0)],
            ..OptionBConfig::default()
        };
        cfg
    }

    /// TechMegaCap +20%, ConsumerMegaCap 0%, everything else +5% (1 year).
    fn tech_vs_consumer_scenario() -> ScenarioPath {
        synthetic_scenario_1yr_overrides(
            0.05,
            &[
                (AssetClass::TechMegaCap, 0.20),
                (AssetClass::ConsumerMegaCap, 0.0),
            ],
        )
    }

    #[test]
    fn msft_concentrated_symbol_walks_price_with_tech_mega_cap_class() {
        // MSFT 100sh @ $400 = $40,000; TechMegaCap +20% → $48,000 (NOT consumer 0%).
        let portfolio = synthetic_portfolio_with(&[("MSFT", 100.0, 400.0), ("FNILX", 400.0, 25.0)]);
        let states = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &tech_vs_consumer_scenario(),
            &default_library(),
            0.22,
            &msft_config(),
        )
        .expect("MSFT projection should succeed");
        assert!(
            (states[1].holdings["MSFT"] - 48_000.0).abs() < 1e-6,
            "{:?}",
            states[1].holdings
        );
        assert!((states[1].concentrated_shares - 100.0).abs() < 1e-9);
        assert!((states[1].concentrated_value() - 48_000.0).abs() < 1e-6);
        // FNILX (UsLargeCapIndex) compounds at the +5% default.
        assert!((states[1].holdings["FNILX"] - 10_500.0).abs() < 1e-6);
    }

    #[test]
    fn msft_concentrated_pct_of_nav_uses_msft_value() {
        let portfolio = synthetic_portfolio_with(&[("MSFT", 100.0, 400.0), ("FNILX", 400.0, 25.0)]);
        let states = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &tech_vs_consumer_scenario(),
            &default_library(),
            0.22,
            &msft_config(),
        )
        .unwrap();
        // year 0: 40,000 / 50,000 = 80%
        assert!((states[0].concentrated_pct_of_nav() - 80.0).abs() < 1e-9);
        // year 1: 48,000 / 58,500
        let expected = 100.0 * 48_000.0 / 58_500.0;
        assert!((states[1].concentrated_pct_of_nav() - expected).abs() < 1e-9);
    }

    #[test]
    fn non_concentrated_amzn_holding_compounds_like_any_other_holding() {
        // With MSFT as the concentrated symbol, AMZN is an ordinary holding:
        // it compounds at its class return (ConsumerMegaCap = 0% here), and is
        // NOT treated as the walked-price concentrated position.
        let portfolio = synthetic_portfolio_with(&[("MSFT", 100.0, 400.0), ("AMZN", 50.0, 200.0)]);
        let states = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &tech_vs_consumer_scenario(),
            &default_library(),
            0.22,
            &msft_config(),
        )
        .unwrap();
        assert!((states[1].holdings["AMZN"] - 10_000.0).abs() < 1e-6);
        assert!((states[1].concentrated_shares - 100.0).abs() < 1e-9);
    }

    #[test]
    fn hold_accumulates_msft_vests_into_concentrated_shares() {
        let portfolio = synthetic_portfolio_with(&[("MSFT", 100.0, 400.0), ("FNILX", 400.0, 25.0)]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event_for("MSFT", "2026-11-01", 100.0)],
        };
        let states = project_path_with_config(
            &portfolio,
            &vesting,
            Treatment::Hold,
            &tech_vs_consumer_scenario(),
            &default_library(),
            0.22,
            &msft_config(),
        )
        .unwrap();
        // 100 + 100 × (1 − 0.22) = 178 shares; value at EOY tech price 480.
        assert!((states[1].concentrated_shares - 178.0).abs() < 1e-9);
        assert!((states[1].holdings["MSFT"] - 178.0 * 480.0).abs() < 1e-6);
    }

    #[test]
    fn sell_reinvest_prices_msft_vest_with_tech_mega_cap_walk() {
        // MSFT 100sh@$400, FNILX $10,000 (0% return). Vest 2026-11-01 (184d), 100 gross,
        // withhold 0. TechMegaCap +10%, all else 0.
        // price_at_vest = 400 × 1.10^(184/365); proceeds = 100 × price_at_vest,
        // all routed to the single non-concentrated holding FNILX.
        let portfolio = synthetic_portfolio_with(&[("MSFT", 100.0, 400.0), ("FNILX", 400.0, 25.0)]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event_for("MSFT", "2026-11-01", 100.0)],
        };
        let scenario = synthetic_scenario_1yr_overrides(0.0, &[(AssetClass::TechMegaCap, 0.10)]);
        let states = project_path_with_config(
            &portfolio,
            &vesting,
            Treatment::SellReinvest,
            &scenario,
            &default_library(),
            0.0,
            &msft_config(),
        )
        .unwrap();
        let price_at_vest = 400.0 * 1.10_f64.powf(184.0 / 365.0);
        let expected_fnilx = 10_000.0 + 100.0 * price_at_vest;
        assert!(
            (states[1].holdings["FNILX"] - expected_fnilx).abs() < 1e-3,
            "FNILX expected {expected_fnilx}, got {}",
            states[1].holdings["FNILX"]
        );
        assert!(
            (states[1].concentrated_shares - 100.0).abs() < 1e-9,
            "shares unchanged"
        );
        assert!((states[1].holdings["MSFT"] - 44_000.0).abs() < 1e-6);
    }

    #[test]
    fn option_b_unwinds_msft_per_config_schedule() {
        let portfolio =
            synthetic_portfolio_with(&[("MSFT", 10_000.0, 200.0), ("FNILX", 40_000.0, 25.0)]);
        let scenario = synthetic_scenario_1yr_const(0.0);
        let states = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &scenario,
            &default_library(),
            0.22,
            &msft_config(),
        )
        .unwrap();
        assert!((states[1].concentrated_shares - 8_000.0).abs() < 0.5);
        // proceeds = 2000 × (200 − 100×0.20) = 360,000; IEF gets 21.4%
        let ief = states[1].holdings["IEF"];
        assert!((ief - 360_000.0 * 0.214).abs() < 50.0, "IEF = {ief}");
    }

    #[test]
    fn missing_concentrated_symbol_reports_the_configured_symbol() {
        let portfolio = synthetic_portfolio_with(&[("AMZN", 100.0, 200.0), ("FNILX", 400.0, 25.0)]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event_for("MSFT", "2026-11-01", 100.0)],
        };
        let result = project_path_with_config(
            &portfolio,
            &vesting,
            Treatment::Hold,
            &synthetic_scenario_2yr_constant_10pct(),
            &default_library(),
            0.22,
            &msft_config(),
        );
        assert!(
            matches!(result, Err(ProjectionError::MissingConcentrated(ref s)) if s == "MSFT"),
            "expected MissingConcentrated(\"MSFT\"), got {result:?}"
        );
    }

    #[test]
    fn engine_does_not_hardcode_amzn_when_concentrated_symbol_is_other() {
        // Portfolio has NO AMZN at all; with symbol MSFT the run must succeed
        // and no AMZN key may appear in the holdings.
        let portfolio = synthetic_portfolio_with(&[("MSFT", 100.0, 400.0), ("FNILX", 400.0, 25.0)]);
        let states = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &synthetic_scenario_2yr_constant_10pct(),
            &default_library(),
            0.22,
            &msft_config(),
        )
        .unwrap();
        for st in &states {
            assert!(!st.holdings.contains_key("AMZN"), "{:?}", st.holdings);
        }
    }

    // ── OptionB defaults behave as documented end-to-end ──────────────────

    #[test]
    fn default_option_b_config_sells_no_existing_shares() {
        let portfolio =
            synthetic_portfolio_with(&[("MSFT", 10_000.0, 200.0), ("FNILX", 40_000.0, 25.0)]);
        let states = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &synthetic_scenario_1yr_const(0.0),
            &default_library(),
            0.22,
            &ProjectionConfig::new("MSFT"),
        )
        .unwrap();
        assert!((states[1].concentrated_shares - 10_000.0).abs() < 1e-9);
    }

    #[test]
    fn default_cost_basis_zero_taxes_whole_sale_price() {
        // Only the schedule is set; basis stays 0.0 and LTCG stays 23.8%.
        // 1000 sh sold at avg price 200 → net = 1000 × 200 × (1 − 0.238) = 152,400.
        let mut cfg = ProjectionConfig::new("MSFT");
        cfg.option_b.unwind_schedule = vec![(1, 1000.0)];
        let portfolio = synthetic_portfolio_with(&[("MSFT", 10_000.0, 200.0)]);
        let states = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &synthetic_scenario_1yr_const(0.0),
            &default_library(),
            0.22,
            &cfg,
        )
        .unwrap();
        let ief = states[1].holdings["IEF"];
        assert!((ief - 152_400.0 * 0.214).abs() < 5.0, "IEF = {ief}");
    }

    #[test]
    fn default_floor_pct_is_25_percent() {
        // MSFT 1200sh@$100 ($120k) + FNILX 11200sh@$25 ($280k): 30% concentrated.
        // Schedule asks for 1000 sh. With the default 25% floor, basis 0, LTCG 23.8%:
        //   net/sh = 100×(1−0.238) = 76.2; ratio = 0.25/0.75 = 1/3
        //   k_max = (120000 − 280000/3) / (100 + 76.2/3) = 212.6 → floor → 212 sold.
        // (A 35% floor would sell nothing.)
        let mut cfg = ProjectionConfig::new("MSFT");
        cfg.option_b.unwind_schedule = vec![(1, 1000.0)];
        let portfolio =
            synthetic_portfolio_with(&[("MSFT", 1200.0, 100.0), ("FNILX", 11_200.0, 25.0)]);
        let states = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &synthetic_scenario_1yr_const(0.0),
            &default_library(),
            0.22,
            &cfg,
        )
        .unwrap();
        let sold: f64 = 1200.0 - states[1].concentrated_shares;
        assert!(
            (sold - 212.0).abs() < 1.0,
            "expected ≈212 shares sold, got {sold}"
        );
    }

    #[test]
    fn custom_reinvest_weights_override_defaults() {
        let mut cfg = ProjectionConfig::new("MSFT");
        cfg.option_b.unwind_schedule = vec![(1, 1000.0)];
        cfg.option_b.cost_basis_per_share = 200.0; // no gain → no tax
        cfg.option_b.reinvest_weights = vec![("VOO".to_string(), 0.6), ("GLD".to_string(), 0.4)];
        let portfolio = synthetic_portfolio_with(&[("MSFT", 10_000.0, 200.0)]);
        let states = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &synthetic_scenario_1yr_const(0.0),
            &default_library(),
            0.22,
            &cfg,
        )
        .unwrap();
        assert!((states[1].holdings["VOO"] - 120_000.0).abs() < 1.0);
        assert!((states[1].holdings["GLD"] - 80_000.0).abs() < 1.0);
        assert!(
            !states[1].holdings.contains_key("IEF"),
            "default weights must not be used"
        );
    }

    // ── resolve_concentrated_symbol precedence ────────────────────────────

    fn vesting_of(events: &[(&str, &str, f64)]) -> DistributionSchedule {
        DistributionSchedule {
            events: events
                .iter()
                .map(|(sym, date, q)| make_vest_event_for(sym, date, *q))
                .collect(),
        }
    }

    #[test]
    fn resolve_explicit_symbol_wins_over_vesting_and_portfolio() {
        let portfolio = synthetic_portfolio_with(&[("MSFT", 1000.0, 400.0)]);
        let vesting = vesting_of(&[("NVDA", "2026-08-01", 500.0)]);
        let got =
            resolve_concentrated_symbol(Some("AAPL"), &vesting, &portfolio, &SymbolMap::builtin());
        assert_eq!(got.as_deref(), Some("AAPL"));
    }

    #[test]
    fn resolve_uses_vesting_symbol_with_most_gross_shares() {
        // Portfolio's largest position is MSFT, but vesting is dominated by NVDA (300 vs 100+120).
        let portfolio = synthetic_portfolio_with(&[("MSFT", 1000.0, 400.0), ("NVDA", 10.0, 100.0)]);
        let vesting = vesting_of(&[
            ("MSFT", "2026-08-01", 100.0),
            ("NVDA", "2026-09-01", 300.0),
            ("MSFT", "2026-10-01", 120.0),
        ]);
        let got = resolve_concentrated_symbol(None, &vesting, &portfolio, &SymbolMap::builtin());
        assert_eq!(got.as_deref(), Some("NVDA"));
    }

    #[test]
    fn resolve_sums_gross_shares_across_events_per_symbol() {
        // MSFT 3 × 100 = 300 beats NVDA single 250.
        let portfolio = synthetic_portfolio_with(&[("NVDA", 1000.0, 100.0)]);
        let vesting = vesting_of(&[
            ("MSFT", "2026-08-01", 100.0),
            ("NVDA", "2026-09-01", 250.0),
            ("MSFT", "2026-10-01", 100.0),
            ("MSFT", "2026-11-01", 100.0),
        ]);
        let got = resolve_concentrated_symbol(None, &vesting, &portfolio, &SymbolMap::builtin());
        assert_eq!(got.as_deref(), Some("MSFT"));
    }

    #[test]
    fn resolve_falls_back_to_largest_non_cash_position_when_no_vesting() {
        // Cash is the largest holding by value; VOO (300k) beats MSFT (100k).
        let portfolio = synthetic_portfolio_with(&[
            ("SPAXX", 500_000.0, 1.0),
            ("MSFT", 250.0, 400.0),
            ("VOO", 500.0, 600.0),
        ]);
        let got =
            resolve_concentrated_symbol(None, &empty_vesting(), &portfolio, &SymbolMap::builtin());
        assert_eq!(got.as_deref(), Some("VOO"));
    }

    #[test]
    fn resolve_fallback_skips_every_cash_mmf_class_ticker() {
        let portfolio = synthetic_portfolio_with(&[
            ("FCASH**", 900_000.0, 1.0),
            ("SPAXX", 800_000.0, 1.0),
            ("IAU", 10.0, 60.0),
        ]);
        let got =
            resolve_concentrated_symbol(None, &empty_vesting(), &portfolio, &SymbolMap::builtin());
        assert_eq!(got.as_deref(), Some("IAU"));
    }

    #[test]
    fn resolve_fallback_respects_custom_symbol_map_cash_classes() {
        // Custom map marks MYMMF as cash_mmf → must be skipped even though it is largest.
        let map = SymbolMap::from_toml_str("[symbols]\nMYMMF = \"cash_mmf\"\n").unwrap();
        let portfolio =
            synthetic_portfolio_with(&[("MYMMF", 900_000.0, 1.0), ("VOO", 10.0, 600.0)]);
        let got = resolve_concentrated_symbol(None, &empty_vesting(), &portfolio, &map);
        assert_eq!(got.as_deref(), Some("VOO"));
    }

    #[test]
    fn resolve_returns_none_for_empty_portfolio_and_vesting() {
        let portfolio = Portfolio {
            positions: vec![],
            cash: 0.0,
            as_of: d("2026-05-01"),
        };
        let got =
            resolve_concentrated_symbol(None, &empty_vesting(), &portfolio, &SymbolMap::builtin());
        assert_eq!(got, None);
    }

    #[test]
    fn resolve_returns_none_when_only_cash_positions_exist() {
        let portfolio = synthetic_portfolio_with(&[("SPAXX", 1000.0, 1.0), ("FCASH**", 50.0, 1.0)]);
        let got =
            resolve_concentrated_symbol(None, &empty_vesting(), &portfolio, &SymbolMap::builtin());
        assert_eq!(got, None);
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Round 2 regressions (H1, H2, H3, M4 floor = 0)
    // ═══════════════════════════════════════════════════════════════════════

    fn voo_config(schedule: Vec<(u32, f64)>) -> ProjectionConfig {
        let mut cfg = ProjectionConfig::new("VOO");
        cfg.option_b = OptionBConfig {
            cost_basis_per_share: 100.0, // == price in the flat scenarios → no tax
            ltcg_rate: 0.238,
            floor_pct: 0.001,
            unwind_schedule: schedule,
            ..OptionBConfig::default()
        };
        cfg
    }

    // ── H1 ────────────────────────────────────────────────────────────────

    /// VOO is the concentrated symbol AND a default reinvest target. Unwind
    /// proceeds must not vanish: drop VOO from the weights and renormalize.
    #[test]
    fn h1_unwind_proceeds_are_conserved_when_concentrated_symbol_is_a_reinvest_target() {
        // VOO 1000 @ $100 = $100k (concentrated), IEF 500 @ $100 = $50k. 0% returns,
        // basis == price ⇒ no tax. Unwind 500 VOO in year 1 ⇒ NAV must stay $150k.
        let portfolio = synthetic_portfolio_with(&[("VOO", 1000.0, 100.0), ("IEF", 500.0, 100.0)]);
        let scenario = synthetic_scenario_1yr_const(0.0);
        let unwound = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &scenario,
            &default_library(),
            0.22,
            &voo_config(vec![(1, 500.0)]),
        )
        .expect("must not error");
        let held = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::Hold,
            &scenario,
            &default_library(),
            0.22,
            &voo_config(vec![]),
        )
        .unwrap();
        let nav = unwound[1].nav();
        let hold_nav = held[1].nav();
        assert!((hold_nav - 150_000.0).abs() < 1e-6);
        // Default weights sum to ≈1 (0.999 or 1.0): at most 0.1% of the $50k proceeds
        // may be undeployed — never the ~28% that VOO's weight would otherwise lose.
        assert!(
            (nav - hold_nav).abs() < 60.0,
            "NAV not conserved: unwind {nav} vs hold {hold_nav} (proceeds lost)"
        );
        assert!((unwound[1].concentrated_shares - 500.0).abs() < 1e-9);
        assert!(
            (unwound[1].holdings["VOO"] - 50_000.0).abs() < 1e-6,
            "VOO holds only the remaining shares"
        );
    }

    #[test]
    fn h1_remaining_weights_are_renormalized_pro_rata() {
        let portfolio = synthetic_portfolio_with(&[("VOO", 1000.0, 100.0), ("IEF", 500.0, 100.0)]);
        let mut cfg = voo_config(vec![(1, 500.0)]);
        cfg.option_b.reinvest_weights = vec![
            ("VOO".to_string(), 0.5),
            ("IEF".to_string(), 0.3),
            ("GLD".to_string(), 0.2),
        ];
        let states = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &synthetic_scenario_1yr_const(0.0),
            &default_library(),
            0.22,
            &cfg,
        )
        .unwrap();
        // proceeds $50,000 split IEF:GLD = 3:2 → +30,000 / +20,000
        assert!(
            (states[1].holdings["IEF"] - 80_000.0).abs() < 1.0,
            "{:?}",
            states[1].holdings
        );
        assert!(
            (states[1].holdings["GLD"] - 20_000.0).abs() < 1.0,
            "{:?}",
            states[1].holdings
        );
        assert!((states[1].nav() - 150_000.0).abs() < 1.0);
    }

    #[test]
    fn h1_vest_proceeds_are_conserved_when_concentrated_symbol_is_a_reinvest_target() {
        // OptionB sells every vest and routes it to the weights; VOO's slice must
        // be redistributed, not dropped. 100 gross VOO vest, withhold 0 ⇒ +$10,000.
        let portfolio = synthetic_portfolio_with(&[("VOO", 1000.0, 100.0), ("IEF", 500.0, 100.0)]);
        let vesting = DistributionSchedule {
            events: vec![make_vest_event_for("VOO", "2026-11-01", 100.0)],
        };
        let states = project_path_with_config(
            &portfolio,
            &vesting,
            Treatment::OptionBUnwind,
            &synthetic_scenario_1yr_const(0.0),
            &default_library(),
            0.0,
            &voo_config(vec![]),
        )
        .unwrap();
        assert!(
            (states[1].nav() - 160_000.0).abs() < 60.0,
            "vest proceeds lost: NAV {}",
            states[1].nav()
        );
        assert!((states[1].concentrated_shares - 1000.0).abs() < 1e-9);
    }

    // ── H2 ────────────────────────────────────────────────────────────────

    fn hold_msft_with(events: Vec<VestingEvent>, treatment: Treatment) -> Vec<PortfolioState> {
        let portfolio = synthetic_portfolio_with(&[("MSFT", 100.0, 400.0), ("FNILX", 400.0, 25.0)]);
        project_path_with_config(
            &portfolio,
            &DistributionSchedule { events },
            treatment,
            &synthetic_scenario_1yr_const(0.0),
            &default_library(),
            0.22,
            &msft_config(),
        )
        .unwrap()
    }

    #[test]
    fn h2_hold_ignores_vests_for_other_symbols() {
        let st = hold_msft_with(
            vec![make_vest_event_for("AAPL", "2026-11-01", 500.0)],
            Treatment::Hold,
        );
        assert!(
            (st[1].concentrated_shares - 100.0).abs() < 1e-9,
            "{}",
            st[1].concentrated_shares
        );
    }

    #[test]
    fn h2_vest_symbol_match_is_case_insensitive() {
        let st = hold_msft_with(
            vec![make_vest_event_for("msft", "2026-11-01", 100.0)],
            Treatment::Hold,
        );
        assert!(
            (st[1].concentrated_shares - 178.0).abs() < 1e-9,
            "{}",
            st[1].concentrated_shares
        );
    }

    #[test]
    fn h2_mixed_symbol_schedule_applies_only_matching_events() {
        let st = hold_msft_with(
            vec![
                make_vest_event_for("MSFT", "2026-08-01", 40.0),
                make_vest_event_for("AAPL", "2026-09-01", 500.0),
                make_vest_event_for("msft", "2026-10-01", 60.0),
                make_vest_event_for("NVDA", "2026-11-01", 900.0),
            ],
            Treatment::Hold,
        );
        // 100 + (40 + 60) × 0.78 = 178
        assert!(
            (st[1].concentrated_shares - 178.0).abs() < 1e-9,
            "{}",
            st[1].concentrated_shares
        );
    }

    #[test]
    fn h2_sell_reinvest_ignores_vests_for_other_symbols() {
        let st = hold_msft_with(
            vec![make_vest_event_for("AAPL", "2026-11-01", 500.0)],
            Treatment::SellReinvest,
        );
        // FNILX 400 × $25 = $10,000 at 0% return; no MSFT vest ⇒ no proceeds.
        assert!(
            (st[1].holdings["FNILX"] - 10_000.0).abs() < 1e-6,
            "{:?}",
            st[1].holdings
        );
    }

    #[test]
    fn h2_option_b_ignores_vests_for_other_symbols() {
        let st = hold_msft_with(
            vec![make_vest_event_for("AAPL", "2026-11-01", 500.0)],
            Treatment::OptionBUnwind,
        );
        // msft_config() schedules a year-1 unwind, so reinvest holdings exist either way;
        // the foreign-symbol vest must leave the result identical to a no-vest run.
        let baseline = hold_msft_with(vec![], Treatment::OptionBUnwind);
        assert_eq!(
            st[1].holdings, baseline[1].holdings,
            "AAPL vest must not affect MSFT projection"
        );
        assert!((st[1].concentrated_shares - baseline[1].concentrated_shares).abs() < 1e-9);
    }

    // ── H3 ────────────────────────────────────────────────────────────────

    #[test]
    fn h3_unknown_reinvest_weight_ticker_is_an_error_not_a_panic() {
        let portfolio =
            synthetic_portfolio_with(&[("MSFT", 10_000.0, 200.0), ("FNILX", 40_000.0, 25.0)]);
        let mut cfg = msft_config();
        cfg.option_b.reinvest_weights = vec![("ZZZQ".to_string(), 0.5), ("VOO".to_string(), 0.5)];
        // horizon 2 (2yr scenario) so the unknown ticker would be hit by later compounding
        let result = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &synthetic_scenario_2yr_constant_10pct(),
            &default_library(),
            0.22,
            &cfg,
        );
        match result {
            Err(e @ ProjectionError::UnknownSymbol(_)) => {
                assert!(e.to_string().contains("ZZZQ"), "{e}");
            }
            other => panic!("expected UnknownSymbol(ZZZQ), got {other:?}"),
        }
    }

    #[test]
    fn h3_unknown_reinvest_ticker_can_be_mapped_via_symbol_map() {
        let portfolio =
            synthetic_portfolio_with(&[("MSFT", 10_000.0, 200.0), ("FNILX", 40_000.0, 25.0)]);
        let mut cfg = msft_config();
        cfg.option_b.reinvest_weights = vec![("ZZZQ".to_string(), 1.0)];
        cfg.symbol_map = SymbolMap::from_toml_str("[symbols]\nZZZQ = \"gold\"\n").unwrap();
        let st = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &synthetic_scenario_2yr_constant_10pct(),
            &default_library(),
            0.22,
            &cfg,
        )
        .expect("mapped ticker must work");
        assert!(st[2].holdings["ZZZQ"] > 0.0);
    }

    // ── M4: floor_pct == 0 means "no floor" ───────────────────────────────

    #[test]
    fn m4_floor_pct_zero_sells_the_full_scheduled_shares() {
        // 30% concentrated: with a 25% floor only ≈212 of 1000 scheduled shares sell
        // (see default_floor_pct_is_25_percent). With floor 0 all 1000 must sell.
        let mut cfg = ProjectionConfig::new("MSFT");
        cfg.option_b.unwind_schedule = vec![(1, 1000.0)];
        cfg.option_b.floor_pct = 0.0;
        let portfolio =
            synthetic_portfolio_with(&[("MSFT", 1200.0, 100.0), ("FNILX", 11_200.0, 25.0)]);
        let states = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &synthetic_scenario_1yr_const(0.0),
            &default_library(),
            0.22,
            &cfg,
        )
        .unwrap();
        assert!(
            (states[1].concentrated_shares - 200.0).abs() < 1e-9,
            "floor 0 ⇒ no floor ⇒ all 1000 scheduled shares sold, got {} left",
            states[1].concentrated_shares
        );
    }

    // ═══ Round 3 ═══

    #[test]
    fn r3_all_reinvest_weight_on_concentrated_symbol_is_an_error() {
        let portfolio = synthetic_portfolio_with(&[("VOO", 1000.0, 100.0), ("IEF", 500.0, 100.0)]);
        let mut cfg = voo_config(vec![(1, 500.0)]);
        cfg.option_b.reinvest_weights = vec![("VOO".to_string(), 1.0)];
        let result = project_path_with_config(
            &portfolio,
            &empty_vesting(),
            Treatment::OptionBUnwind,
            &synthetic_scenario_1yr_const(0.0),
            &default_library(),
            0.22,
            &cfg,
        );
        let err = result.expect_err("proceeds must not be silently lost");
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("reinvest") && msg.contains("concentrated"),
            "{msg}"
        );
    }

    #[test]
    fn r3_all_reinvest_weight_on_concentrated_symbol_lowercase_still_error() {
        let portfolio = synthetic_portfolio_with(&[("VOO", 1000.0, 100.0), ("IEF", 500.0, 100.0)]);
        let mut cfg = voo_config(vec![]);
        cfg.option_b.reinvest_weights = vec![("VOO".to_string(), 0.6), ("VOO".to_string(), 0.4)];
        let vesting = DistributionSchedule {
            events: vec![make_vest_event_for("VOO", "2026-11-01", 100.0)],
        };
        let err = project_path_with_config(
            &portfolio,
            &vesting,
            Treatment::OptionBUnwind,
            &synthetic_scenario_1yr_const(0.0),
            &default_library(),
            0.0,
            &cfg,
        )
        .expect_err("every weight on the concentrated symbol");
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("reinvest") && msg.contains("concentrated"),
            "{msg}"
        );
    }
}
