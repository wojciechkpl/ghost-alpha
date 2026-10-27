use super::brackets::{FederalCapGainsBrackets, StateTaxRate};
use super::engine::compute_lot_tax;
use super::hifo::{hifo_select, priority_select};
use crate::portfolio::types::TaxLot;

/// Strategy for selecting which lots to sell.
#[derive(Debug, Clone)]
pub enum LotSelectionStrategy {
    /// Highest-In, First-Out — minimises realised gain amount.
    Hifo,
    /// 3-tier priority — harvests ST losses first, then LT ascending by gain%, defers near-LTCG.
    Priority {
        /// Lots within this many days of LTCG are deferred.
        ltcg_hold_days: i64,
    },
}

/// Projected tax liability for a single year.
#[derive(Debug, Clone, serde::Serialize)]
pub struct YearProjection {
    pub year: u32,
    pub shares_sold: f64,
    pub proceeds: f64,
    pub realised_gain: f64,
    pub federal_tax: f64,
    pub state_tax: f64,
    pub niit: f64,
    pub total_tax: f64,
    pub cumulative_shares_sold: f64,
    pub remaining_shares: f64,
}

/// Given a selling schedule and assumptions, project year-by-year tax impact.
///
/// For each year in `annual_shares`, the projected sale price is
/// `current_price * (1 + assumed_price_growth)^year_offset` where
/// `year_offset = year - first_year`. Lots are selected via HIFO and
/// their remaining shares are decremented across years.
pub fn project_multi_year(
    lots: &[TaxLot],
    annual_shares: &[(u32, f64)],
    assumed_price_growth: f64,
    current_price: f64,
    other_annual_income: f64,
    brackets: &FederalCapGainsBrackets,
    state: &StateTaxRate,
) -> Vec<YearProjection> {
    project_multi_year_with_strategy(
        lots,
        annual_shares,
        assumed_price_growth,
        current_price,
        other_annual_income,
        brackets,
        state,
        &LotSelectionStrategy::Hifo,
    )
}

/// Like `project_multi_year` but with explicit lot selection strategy.
#[allow(clippy::too_many_arguments)]
pub fn project_multi_year_with_strategy(
    lots: &[TaxLot],
    annual_shares: &[(u32, f64)],
    assumed_price_growth: f64,
    current_price: f64,
    other_annual_income: f64,
    brackets: &FederalCapGainsBrackets,
    state: &StateTaxRate,
    strategy: &LotSelectionStrategy,
) -> Vec<YearProjection> {
    if annual_shares.is_empty() {
        return Vec::new();
    }

    // Clone lots so we can mutate remaining_shares across years.
    let mut working_lots = lots.to_vec();
    let first_year = annual_shares[0].0;
    let mut cumulative = 0.0;

    annual_shares
        .iter()
        .map(|&(year, shares_to_sell)| {
            let year_offset = year.saturating_sub(first_year);
            let projected_price =
                current_price * (1.0 + assumed_price_growth).powi(year_offset as i32);

            // Use Jul 1 of the given year as notional sale date.
            let sale_date = chrono::NaiveDate::from_ymd_opt(year as i32, 7, 1)
                .unwrap_or_else(|| chrono::NaiveDate::from_ymd_opt(year as i32, 1, 1).unwrap());

            // Select lots based on strategy.
            let selections = match strategy {
                LotSelectionStrategy::Hifo => hifo_select(&working_lots, shares_to_sell),
                LotSelectionStrategy::Priority { ltcg_hold_days } => priority_select(
                    &working_lots,
                    shares_to_sell,
                    projected_price,
                    sale_date,
                    *ltcg_hold_days,
                ),
            };

            let mut year_proceeds = 0.0;
            let mut year_gain = 0.0;
            let mut year_federal = 0.0;
            let mut year_state = 0.0;
            let mut year_niit = 0.0;
            let mut year_shares = 0.0;
            let mut state_gains_ytd = 0.0;

            for (idx, shares) in &selections {
                let impact = compute_lot_tax(
                    &working_lots[*idx],
                    *shares,
                    projected_price,
                    sale_date,
                    other_annual_income,
                    brackets,
                    state,
                    state_gains_ytd,
                );
                year_proceeds += impact.proceeds;
                year_gain += impact.realised_gain;
                year_federal += impact.federal_tax;
                year_state += impact.state_tax;
                year_niit += impact.niit;
                year_shares += shares;
                // Track cumulative gains for the state deduction. A long-term-only
                // state (WA) neither taxes short-term gains nor uses the deduction on them.
                if impact.realised_gain > 0.0 && !(state.long_term_only && !impact.is_long_term) {
                    state_gains_ytd += impact.realised_gain;
                }
            }

            // Decrement remaining shares in working lots.
            for (idx, shares) in &selections {
                working_lots[*idx].remaining_shares -= shares;
            }

            cumulative += year_shares;
            let remaining: f64 = working_lots.iter().map(|l| l.remaining_shares).sum();

            YearProjection {
                year,
                shares_sold: year_shares,
                proceeds: year_proceeds,
                realised_gain: year_gain,
                federal_tax: year_federal,
                state_tax: year_state,
                niit: year_niit,
                total_tax: year_federal + year_state + year_niit,
                cumulative_shares_sold: cumulative,
                remaining_shares: remaining,
            }
        })
        .collect()
}

/// A single scenario in a multi-year comparison.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScenarioResult {
    /// Number of years the gain is spread across.
    pub spread_years: u32,
    /// Shares sold per year (even distribution).
    pub annual_shares: f64,
    /// Year-by-year projections.
    pub projections: Vec<YearProjection>,
    /// Aggregate totals.
    pub total_proceeds: f64,
    pub total_gain: f64,
    pub total_federal_tax: f64,
    pub total_state_tax: f64,
    pub total_niit: f64,
    pub total_tax: f64,
    pub effective_rate: f64,
    /// Savings compared to lump-sum (spread_years=1).
    pub savings_vs_lump: f64,
    /// True if this is the optimal (lowest tax) scenario.
    pub is_optimal: bool,
}

/// Compare spreading `total_shares` across 1..=`max_years`.
///
/// For each spread, shares are distributed evenly. Returns all scenarios
/// sorted by `spread_years` ascending, with `is_optimal` set on the best one.
///
/// `start_year` is the first year of the selling programme (e.g. 2026).
#[allow(clippy::too_many_arguments)]
pub fn compare_scenarios(
    lots: &[TaxLot],
    total_shares: f64,
    max_years: u32,
    start_year: u32,
    assumed_price_growth: f64,
    current_price: f64,
    other_annual_income: f64,
    brackets: &FederalCapGainsBrackets,
    state: &StateTaxRate,
) -> Vec<ScenarioResult> {
    if lots.is_empty() || total_shares <= 0.0 || max_years == 0 {
        return Vec::new();
    }

    let mut scenarios = Vec::with_capacity(max_years as usize);

    for years in 1..=max_years {
        let per_year = total_shares / years as f64;
        let annual_shares: Vec<(u32, f64)> =
            (0..years).map(|i| (start_year + i, per_year)).collect();

        let projections = project_multi_year(
            lots,
            &annual_shares,
            assumed_price_growth,
            current_price,
            other_annual_income,
            brackets,
            state,
        );

        let total_proceeds: f64 = projections.iter().map(|p| p.proceeds).sum();
        let total_gain: f64 = projections.iter().map(|p| p.realised_gain).sum();
        let total_federal: f64 = projections.iter().map(|p| p.federal_tax).sum();
        let total_state: f64 = projections.iter().map(|p| p.state_tax).sum();
        let total_niit: f64 = projections.iter().map(|p| p.niit).sum();
        let total_tax: f64 = projections.iter().map(|p| p.total_tax).sum();

        scenarios.push(ScenarioResult {
            spread_years: years,
            annual_shares: per_year,
            projections,
            total_proceeds,
            total_gain,
            total_federal_tax: total_federal,
            total_state_tax: total_state,
            total_niit,
            total_tax,
            effective_rate: if total_gain > 0.0 {
                total_tax / total_gain
            } else {
                0.0
            },
            savings_vs_lump: 0.0,
            is_optimal: false,
        });
    }

    // Compute savings vs lump sum and find optimal.
    let lump_tax = scenarios.first().map(|s| s.total_tax).unwrap_or(0.0);
    let mut min_tax = f64::MAX;
    let mut optimal_idx = 0;

    for (i, s) in scenarios.iter_mut().enumerate() {
        s.savings_vs_lump = lump_tax - s.total_tax;
        if s.total_tax < min_tax {
            min_tax = s.total_tax;
            optimal_idx = i;
        }
    }
    if !scenarios.is_empty() {
        scenarios[optimal_idx].is_optimal = true;
    }

    scenarios
}

/// Format the scenario comparison as a human-readable table.
pub fn format_comparison_table(scenarios: &[ScenarioResult], ticker: &str, state: &str) -> String {
    if scenarios.is_empty() {
        return "  No scenarios to compare.".to_string();
    }

    let mut lines = Vec::new();
    lines.push(String::new());
    lines.push(format!(
        "  {} — Multi-year scenario comparison (State: {})",
        ticker, state
    ));
    lines.push(format!("  {}", "─".repeat(72)));
    lines.push(format!(
        "  {:>5}  {:>10}  {:>12}  {:>10}  {:>10}  {:>7}  {:>10}",
        "Years", "Shares/yr", "Total Tax", "Fed", "State", "Eff %", "Savings"
    ));
    lines.push(format!("  {}", "─".repeat(72)));

    for s in scenarios {
        let marker = if s.is_optimal { " <<<" } else { "" };
        lines.push(format!(
            "  {:>5}  {:>10.0}  {:>12}  {:>10}  {:>10}  {:>6.1}%  {:>10}{}",
            s.spread_years,
            s.annual_shares,
            format_dollars(s.total_tax),
            format_dollars(s.total_federal_tax),
            format_dollars(s.total_state_tax),
            s.effective_rate * 100.0,
            format_dollars(s.savings_vs_lump),
            marker,
        ));
    }

    lines.push(format!("  {}", "─".repeat(72)));

    if let Some(opt) = scenarios.iter().find(|s| s.is_optimal) {
        if opt.savings_vs_lump > 0.0 {
            lines.push(format!(
                "  Spreading over {} year{} saves {} vs lump sum.",
                opt.spread_years,
                if opt.spread_years > 1 { "s" } else { "" },
                format_dollars(opt.savings_vs_lump),
            ));
        } else {
            lines.push("  No savings from spreading — sell in 1 year.".to_string());
        }
    }
    lines.push(String::new());
    lines.join("\n")
}

fn format_dollars(amount: f64) -> String {
    if amount < 0.0 {
        format!("-${:.0}", amount.abs())
    } else {
        format!("${:.0}", amount)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::types::{AcquisitionType, TaxLot};
    use crate::tax::brackets::state_tax_rate;
    use chrono::NaiveDate;
    use uuid::Uuid;

    fn make_lot(cost: f64, shares: f64) -> TaxLot {
        TaxLot {
            id: Uuid::new_v4(),
            ticker: "TEST".to_string(),
            shares,
            cost_basis_per_share: cost,
            acquired_date: NaiveDate::from_ymd_opt(2020, 1, 1).unwrap(),
            acquisition_type: AcquisitionType::Buy,
            remaining_shares: shares,
        }
    }

    fn tx() -> StateTaxRate {
        state_tax_rate("TX").unwrap()
    }

    #[test]
    fn test_single_year_projection() {
        let lots = vec![make_lot(100.0, 500.0)];
        let b = FederalCapGainsBrackets::single_2024();
        let result = project_multi_year(&lots, &[(2025, 100.0)], 0.0, 200.0, 0.0, &b, &tx());
        assert_eq!(result.len(), 1);
        assert!((result[0].shares_sold - 100.0).abs() < f64::EPSILON);
        assert!((result[0].proceeds - 20_000.0).abs() < 0.01);
        assert!(result[0].total_tax >= 0.0);
    }

    #[test]
    fn test_multi_year_cumulative() {
        let lots = vec![make_lot(100.0, 500.0)];
        let b = FederalCapGainsBrackets::single_2024();
        let result = project_multi_year(
            &lots,
            &[(2025, 100.0), (2026, 150.0), (2027, 50.0)],
            0.0,
            200.0,
            0.0,
            &b,
            &tx(),
        );
        assert_eq!(result.len(), 3);
        assert!((result[2].cumulative_shares_sold - 300.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_price_growth_affects_proceeds() {
        let lots = vec![make_lot(100.0, 500.0)];
        let b = FederalCapGainsBrackets::single_2024();
        let result = project_multi_year(
            &lots,
            &[(2025, 100.0), (2026, 100.0)],
            0.10, // 10% annual growth
            200.0,
            0.0,
            &b,
            &tx(),
        );
        assert_eq!(result.len(), 2);
        // Year 1 price = 200.0 (offset 0), Year 2 price = 220.0 (offset 1)
        assert!((result[0].proceeds - 20_000.0).abs() < 0.01);
        assert!((result[1].proceeds - 22_000.0).abs() < 0.01);
    }

    #[test]
    fn test_remaining_shares_decrease() {
        let lots = vec![make_lot(100.0, 300.0)];
        let b = FederalCapGainsBrackets::single_2024();
        let result = project_multi_year(
            &lots,
            &[(2025, 100.0), (2026, 100.0), (2027, 100.0)],
            0.0,
            200.0,
            0.0,
            &b,
            &tx(),
        );
        assert!((result[0].remaining_shares - 200.0).abs() < f64::EPSILON);
        assert!((result[1].remaining_shares - 100.0).abs() < f64::EPSILON);
        assert!((result[2].remaining_shares - 0.0).abs() < f64::EPSILON);
    }

    // ── compare_scenarios tests ──────────────────────────────────────

    #[test]
    fn test_compare_single_year_matches_project() {
        let lots = vec![make_lot(100.0, 500.0)];
        let b = FederalCapGainsBrackets::single_2024();
        let scenarios = compare_scenarios(&lots, 500.0, 1, 2026, 0.0, 200.0, 0.0, &b, &tx());
        assert_eq!(scenarios.len(), 1);

        let direct = project_multi_year(&lots, &[(2026, 500.0)], 0.0, 200.0, 0.0, &b, &tx());
        let direct_tax: f64 = direct.iter().map(|p| p.total_tax).sum();
        assert!((scenarios[0].total_tax - direct_tax).abs() < 0.01);
    }

    #[test]
    fn test_compare_more_years_lower_or_equal_tax() {
        let lots = vec![make_lot(50.0, 2000.0)];
        let b = FederalCapGainsBrackets::mfj_2024();
        let wa = state_tax_rate("WA").unwrap();
        let scenarios = compare_scenarios(&lots, 2000.0, 5, 2026, 0.0, 500.0, 200_000.0, &b, &wa);
        for i in 1..scenarios.len() {
            assert!(
                scenarios[i].total_tax <= scenarios[i - 1].total_tax + 0.01,
                "Year {} tax should be <= year {} tax",
                i + 1,
                i
            );
        }
    }

    #[test]
    fn test_compare_savings_vs_lump() {
        let lots = vec![make_lot(100.0, 1000.0)];
        let b = FederalCapGainsBrackets::single_2024();
        let scenarios = compare_scenarios(&lots, 1000.0, 3, 2026, 0.0, 200.0, 100_000.0, &b, &tx());
        assert!(
            (scenarios[0].savings_vs_lump).abs() < f64::EPSILON,
            "Lump sum savings should be $0"
        );
        for s in &scenarios[1..] {
            assert!(s.savings_vs_lump >= -0.01, "Savings should be non-negative");
        }
    }

    #[test]
    fn test_compare_optimal_flag_set() {
        let lots = vec![make_lot(100.0, 500.0)];
        let b = FederalCapGainsBrackets::single_2024();
        let scenarios = compare_scenarios(&lots, 500.0, 5, 2026, 0.0, 200.0, 0.0, &b, &tx());
        let optimal_count = scenarios.iter().filter(|s| s.is_optimal).count();
        assert_eq!(optimal_count, 1, "Exactly one scenario should be optimal");
    }

    #[test]
    fn test_compare_wa_plateau_detection() {
        let lots = vec![make_lot(50.0, 3000.0)];
        let b = FederalCapGainsBrackets::mfj_2024();
        let wa = state_tax_rate("WA").unwrap();
        // $600K total gain. At 1 year: $322K taxable in WA. At 3 years: $200K/yr < $278K => $0 WA tax.
        let scenarios = compare_scenarios(&lots, 3000.0, 5, 2026, 0.0, 250.0, 100_000.0, &b, &wa);
        assert!(scenarios.len() == 5);
        // 3+ years should have $0 WA state tax
        assert!(
            (scenarios[2].total_state_tax).abs() < 0.01,
            "3-year spread: WA tax should be ~$0, got {}",
            scenarios[2].total_state_tax
        );
    }

    #[test]
    fn test_compare_empty_lots_returns_empty() {
        let b = FederalCapGainsBrackets::single_2024();
        let scenarios = compare_scenarios(&[], 500.0, 5, 2026, 0.0, 200.0, 0.0, &b, &tx());
        assert!(scenarios.is_empty());
    }

    #[test]
    fn test_compare_max_years_capped() {
        let lots = vec![make_lot(100.0, 500.0)];
        let b = FederalCapGainsBrackets::single_2024();
        let scenarios = compare_scenarios(&lots, 500.0, 10, 2026, 0.0, 200.0, 0.0, &b, &tx());
        assert_eq!(scenarios.len(), 10);
    }

    #[test]
    fn test_compare_effective_rate_bounded() {
        let lots = vec![make_lot(50.0, 1000.0)];
        let b = FederalCapGainsBrackets::single_2024();
        let ca = state_tax_rate("CA").unwrap();
        let scenarios = compare_scenarios(&lots, 1000.0, 5, 2026, 0.0, 300.0, 200_000.0, &b, &ca);
        for s in &scenarios {
            assert!(
                s.effective_rate >= 0.0 && s.effective_rate < 1.0,
                "Effective rate should be in [0, 1), got {}",
                s.effective_rate
            );
        }
    }

    #[test]
    fn test_format_comparison_table_content() {
        let lots = vec![make_lot(100.0, 500.0)];
        let b = FederalCapGainsBrackets::single_2024();
        let scenarios = compare_scenarios(&lots, 500.0, 3, 2026, 0.0, 200.0, 0.0, &b, &tx());
        let table = format_comparison_table(&scenarios, "AMZN", "TX");
        assert!(table.contains("AMZN"));
        assert!(table.contains("Years"));
        assert!(table.contains("Savings"));
        assert!(table.contains("<<<"));
    }

    #[test]
    fn test_format_comparison_table_empty() {
        let table = format_comparison_table(&[], "AMZN", "TX");
        assert!(table.contains("No scenarios"));
    }
}
