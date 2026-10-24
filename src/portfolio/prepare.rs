use chrono::NaiveDate;
use serde::Serialize;

use crate::portfolio::selling::SellingProgramme;
use crate::portfolio::types::TaxLot;
use crate::tax::brackets::{state_tax_rate, FederalCapGainsBrackets, FilingStatus};
use crate::tax::projection::{project_multi_year, YearProjection};

/// Input parameters for generating a selling programme.
#[derive(Debug, Clone)]
pub struct SellPlanInput {
    pub ticker: String,
    pub total_shares: f64,
    pub cost_basis_per_share: f64,
    pub current_price: f64,
    pub acquired_date: NaiveDate,
    pub filing_status: FilingStatus,
    pub state_code: String,
    pub other_annual_income: f64,
    pub assumed_price_growth: f64,
    pub horizon_years: u32,
    /// First year of the selling programme (e.g. 2026).
    pub start_year: u32,
    /// How to distribute shares across years.
    pub distribution: SellDistribution,
}

/// How to spread the selling across the programme horizon.
#[derive(Debug, Clone)]
pub enum SellDistribution {
    /// Equal shares each year.
    Even,
    /// Front-loaded: sell more in year 1, less in later years.
    FrontLoaded,
    /// Back-loaded: sell less early, more later (ride momentum).
    BackLoaded,
    /// Custom annual share amounts.
    Custom(Vec<f64>),
}

/// Full output of a selling programme preparation.
#[derive(Debug, Clone, Serialize)]
pub struct SellPlanOutput {
    pub ticker: String,
    pub total_shares: f64,
    pub horizon_years: u32,
    pub annual_targets: Vec<f64>,
    pub year_projections: Vec<YearProjection>,
    pub programme: SellingProgramme,
    pub summary: SellPlanSummary,
}

/// Aggregate summary across all years.
#[derive(Debug, Clone, Serialize)]
pub struct SellPlanSummary {
    pub total_proceeds: f64,
    pub total_gain: f64,
    pub total_federal_tax: f64,
    pub total_state_tax: f64,
    pub total_niit: f64,
    pub total_tax: f64,
    pub total_after_tax: f64,
    pub blended_effective_rate: f64,
}

/// Prepare a selling programme with tax projections.
///
/// Given a position (all vested), this function:
/// 1. Distributes shares across years per the chosen distribution
/// 2. Creates a `SellingProgramme` with quarterly schedule
/// 3. Projects year-by-year tax impact using the tax engine
/// 4. Returns the complete plan with summary statistics
pub fn prepare_selling_programme(input: &SellPlanInput) -> Result<SellPlanOutput, String> {
    if input.total_shares <= 0.0 {
        return Err("total_shares must be positive".into());
    }
    if input.horizon_years == 0 || input.horizon_years > 10 {
        return Err("horizon_years must be 1–10".into());
    }
    if input.current_price <= 0.0 {
        return Err("current_price must be positive".into());
    }

    // 1. Compute annual share targets.
    let annual_targets =
        compute_annual_targets(input.total_shares, input.horizon_years, &input.distribution);

    // 2. Build the quarterly selling programme.
    let programme =
        SellingProgramme::build_schedule(&input.ticker, &annual_targets, input.horizon_years);

    // 3. Build a single tax lot representing the fully vested position.
    let lot = TaxLot {
        id: uuid::Uuid::new_v4(),
        ticker: input.ticker.clone(),
        shares: input.total_shares,
        cost_basis_per_share: input.cost_basis_per_share,
        acquired_date: input.acquired_date,
        acquisition_type: crate::portfolio::types::AcquisitionType::RsuVest,
        remaining_shares: input.total_shares,
    };

    // 4. Select brackets by filing status.
    let brackets = match input.filing_status {
        FilingStatus::Single => FederalCapGainsBrackets::single_2024(),
        FilingStatus::MarriedFilingJointly => FederalCapGainsBrackets::mfj_2024(),
        // Fall back to single for other statuses.
        _ => FederalCapGainsBrackets::single_2024(),
    };

    let state = state_tax_rate(&input.state_code)
        .map_err(|e| format!("Unknown state '{}': {}", input.state_code, e))?;

    // 5. Build annual_shares as (year, shares) for projection.
    let annual_shares: Vec<(u32, f64)> = annual_targets
        .iter()
        .enumerate()
        .map(|(i, &shares)| (input.start_year + i as u32, shares))
        .collect();

    // 6. Project tax impact.
    let projections = project_multi_year(
        &[lot],
        &annual_shares,
        input.assumed_price_growth,
        input.current_price,
        input.other_annual_income,
        &brackets,
        &state,
    );

    // 7. Compute summary.
    let summary = compute_summary(&projections);

    Ok(SellPlanOutput {
        ticker: input.ticker.clone(),
        total_shares: input.total_shares,
        horizon_years: input.horizon_years,
        annual_targets,
        year_projections: projections,
        programme,
        summary,
    })
}

fn compute_annual_targets(
    total_shares: f64,
    years: u32,
    distribution: &SellDistribution,
) -> Vec<f64> {
    match distribution {
        SellDistribution::Even => {
            let per_year = total_shares / years as f64;
            vec![per_year; years as usize]
        }
        SellDistribution::FrontLoaded => {
            // Weighted: year 1 gets more, linearly decreasing.
            // Weights: n, n-1, ..., 1 => sum = n*(n+1)/2
            let n = years as f64;
            let weight_sum = n * (n + 1.0) / 2.0;
            (0..years)
                .map(|i| {
                    let weight = n - i as f64;
                    total_shares * weight / weight_sum
                })
                .collect()
        }
        SellDistribution::BackLoaded => {
            // Weighted: year 1 gets less, linearly increasing.
            // Weights: 1, 2, ..., n => sum = n*(n+1)/2
            let n = years as f64;
            let weight_sum = n * (n + 1.0) / 2.0;
            (0..years)
                .map(|i| {
                    let weight = (i + 1) as f64;
                    total_shares * weight / weight_sum
                })
                .collect()
        }
        SellDistribution::Custom(targets) => {
            let mut result = targets.clone();
            // Pad with zeros or truncate to match horizon.
            result.resize(years as usize, 0.0);
            result
        }
    }
}

fn compute_summary(projections: &[YearProjection]) -> SellPlanSummary {
    let total_proceeds: f64 = projections.iter().map(|p| p.proceeds).sum();
    let total_gain: f64 = projections.iter().map(|p| p.realised_gain).sum();
    let total_federal: f64 = projections.iter().map(|p| p.federal_tax).sum();
    let total_state: f64 = projections.iter().map(|p| p.state_tax).sum();
    let total_niit: f64 = projections.iter().map(|p| p.niit).sum();
    let total_tax: f64 = projections.iter().map(|p| p.total_tax).sum();

    SellPlanSummary {
        total_proceeds,
        total_gain,
        total_federal_tax: total_federal,
        total_state_tax: total_state,
        total_niit,
        total_tax,
        total_after_tax: total_proceeds - total_tax,
        blended_effective_rate: if total_gain > 0.0 {
            total_tax / total_gain
        } else {
            0.0
        },
    }
}

/// Format the sell plan as a human-readable table.
pub fn format_sell_plan(output: &SellPlanOutput, state: &str) -> String {
    let mut lines = Vec::new();

    lines.push(String::new());
    lines.push(format!(
        "  {} — {}-year selling programme",
        output.ticker, output.horizon_years
    ));
    lines.push(format!("  {}", "─".repeat(58)));
    lines.push(format!(
        "  Total shares: {:.0}  │  Current price: ${:.2}  │  State: {}",
        output.total_shares,
        output
            .year_projections
            .first()
            .map_or(0.0, |p| p.proceeds / p.shares_sold),
        state,
    ));
    lines.push(String::new());

    // Year-by-year table
    lines.push(format!(
        "  {:>4}  {:>10}  {:>12}  {:>12}  {:>10}  {:>10}  {:>10}  {:>7}",
        "Year", "Shares", "Proceeds", "Gain", "Fed Tax", "State", "NIIT", "Eff %"
    ));
    lines.push(format!("  {}", "─".repeat(83)));

    for p in &output.year_projections {
        let eff_rate = if p.realised_gain > 0.0 {
            p.total_tax / p.realised_gain * 100.0
        } else {
            0.0
        };
        lines.push(format!(
            "  {:>4}  {:>10.0}  {:>12}  {:>12}  {:>10}  {:>10}  {:>10}  {:>6.1}%",
            p.year,
            p.shares_sold,
            format_dollars(p.proceeds),
            format_dollars(p.realised_gain),
            format_dollars(p.federal_tax),
            format_dollars(p.state_tax),
            format_dollars(p.niit),
            eff_rate,
        ));
    }

    lines.push(format!("  {}", "─".repeat(83)));

    let s = &output.summary;
    lines.push(format!(
        "  {:>4}  {:>10.0}  {:>12}  {:>12}  {:>10}  {:>10}  {:>10}  {:>6.1}%",
        "TOTAL",
        output.total_shares,
        format_dollars(s.total_proceeds),
        format_dollars(s.total_gain),
        format_dollars(s.total_federal_tax),
        format_dollars(s.total_state_tax),
        format_dollars(s.total_niit),
        s.blended_effective_rate * 100.0,
    ));

    lines.push(String::new());
    lines.push(format!(
        "  After-tax proceeds: {}",
        format_dollars(s.total_after_tax)
    ));

    // Quarterly breakdown
    lines.push(String::new());
    lines.push("  Quarterly schedule:".to_string());
    for period in &output.programme.schedule {
        lines.push(format!(
            "    Year {} Q{}: {:.0} shares",
            period.year, period.quarter, period.shares_target
        ));
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

    fn default_input() -> SellPlanInput {
        SellPlanInput {
            ticker: "AMZN".to_string(),
            total_shares: 1000.0,
            cost_basis_per_share: 100.0,
            current_price: 200.0,
            acquired_date: NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(),
            filing_status: FilingStatus::Single,
            state_code: "WA".to_string(),
            other_annual_income: 200_000.0,
            assumed_price_growth: 0.0,
            horizon_years: 2,
            start_year: 2026,
            distribution: SellDistribution::Even,
        }
    }

    #[test]
    fn test_even_distribution() {
        let targets = compute_annual_targets(1000.0, 2, &SellDistribution::Even);
        assert_eq!(targets.len(), 2);
        assert!((targets[0] - 500.0).abs() < f64::EPSILON);
        assert!((targets[1] - 500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_front_loaded_distribution() {
        let targets = compute_annual_targets(300.0, 3, &SellDistribution::FrontLoaded);
        assert_eq!(targets.len(), 3);
        // Weights: 3, 2, 1 => sum=6 => 150, 100, 50
        assert!((targets[0] - 150.0).abs() < f64::EPSILON);
        assert!((targets[1] - 100.0).abs() < f64::EPSILON);
        assert!((targets[2] - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_back_loaded_distribution() {
        let targets = compute_annual_targets(300.0, 3, &SellDistribution::BackLoaded);
        assert_eq!(targets.len(), 3);
        // Weights: 1, 2, 3 => sum=6 => 50, 100, 150
        assert!((targets[0] - 50.0).abs() < f64::EPSILON);
        assert!((targets[1] - 100.0).abs() < f64::EPSILON);
        assert!((targets[2] - 150.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_custom_distribution() {
        let targets =
            compute_annual_targets(1000.0, 3, &SellDistribution::Custom(vec![200.0, 300.0]));
        assert_eq!(targets.len(), 3);
        assert!((targets[0] - 200.0).abs() < f64::EPSILON);
        assert!((targets[1] - 300.0).abs() < f64::EPSILON);
        assert!((targets[2] - 0.0).abs() < f64::EPSILON); // padded
    }

    #[test]
    fn test_prepare_basic() {
        let input = default_input();
        let output = prepare_selling_programme(&input).unwrap();
        assert_eq!(output.ticker, "AMZN");
        assert_eq!(output.horizon_years, 2);
        assert_eq!(output.annual_targets.len(), 2);
        assert_eq!(output.year_projections.len(), 2);
        assert_eq!(output.programme.schedule.len(), 8); // 2 years * 4 quarters
        assert!(output.summary.total_proceeds > 0.0);
        assert!(output.summary.total_tax >= 0.0);
        assert!(output.summary.total_after_tax > 0.0);
        assert!(output.summary.blended_effective_rate >= 0.0);
        assert!(output.summary.blended_effective_rate < 1.0);
    }

    #[test]
    fn test_prepare_zero_shares_errors() {
        let mut input = default_input();
        input.total_shares = 0.0;
        assert!(prepare_selling_programme(&input).is_err());
    }

    #[test]
    fn test_prepare_zero_horizon_errors() {
        let mut input = default_input();
        input.horizon_years = 0;
        assert!(prepare_selling_programme(&input).is_err());
    }

    #[test]
    fn test_prepare_unknown_state_errors() {
        let mut input = default_input();
        input.state_code = "ZZ".to_string();
        assert!(prepare_selling_programme(&input).is_err());
    }

    #[test]
    fn test_prepare_no_gain_no_tax() {
        let mut input = default_input();
        input.current_price = input.cost_basis_per_share; // no gain
        let output = prepare_selling_programme(&input).unwrap();
        assert!((output.summary.total_gain).abs() < 0.01);
        assert!((output.summary.total_tax).abs() < 0.01);
    }

    #[test]
    fn test_prepare_with_growth() {
        let mut input = default_input();
        input.assumed_price_growth = 0.10; // 10% annual
        let output = prepare_selling_programme(&input).unwrap();
        // Year 2 should have higher proceeds per share than year 1
        assert!(output.year_projections[1].proceeds > output.year_projections[0].proceeds - 1.0);
    }

    #[test]
    fn test_prepare_mfj_vs_single_lower_tax() {
        let single_input = default_input();
        let single = prepare_selling_programme(&single_input).unwrap();

        let mut mfj_input = default_input();
        mfj_input.filing_status = FilingStatus::MarriedFilingJointly;
        let mfj = prepare_selling_programme(&mfj_input).unwrap();

        // MFJ has wider brackets => lower tax on same income
        assert!(
            mfj.summary.total_tax <= single.summary.total_tax,
            "MFJ should have equal or lower tax"
        );
    }

    #[test]
    fn test_prepare_tx_vs_ca_state_tax() {
        let mut tx_input = default_input();
        tx_input.state_code = "TX".to_string();
        let tx = prepare_selling_programme(&tx_input).unwrap();

        let mut ca_input = default_input();
        ca_input.state_code = "CA".to_string();
        let ca = prepare_selling_programme(&ca_input).unwrap();

        assert!(
            (tx.summary.total_state_tax).abs() < f64::EPSILON,
            "TX has no state tax"
        );
        assert!(ca.summary.total_state_tax > 0.0, "CA should have state tax");
    }

    #[test]
    fn test_format_sell_plan_output() {
        let input = default_input();
        let output = prepare_selling_programme(&input).unwrap();
        let formatted = format_sell_plan(&output, "WA");
        assert!(formatted.contains("AMZN"));
        assert!(formatted.contains("2-year selling programme"));
        assert!(formatted.contains("TOTAL"));
        assert!(formatted.contains("After-tax proceeds"));
        assert!(formatted.contains("Quarterly schedule"));
    }

    #[test]
    fn test_summary_totals_match_projections() {
        let input = default_input();
        let output = prepare_selling_programme(&input).unwrap();

        let sum_proceeds: f64 = output.year_projections.iter().map(|p| p.proceeds).sum();
        let sum_tax: f64 = output.year_projections.iter().map(|p| p.total_tax).sum();

        assert!((output.summary.total_proceeds - sum_proceeds).abs() < 0.01);
        assert!((output.summary.total_tax - sum_tax).abs() < 0.01);
    }
}
