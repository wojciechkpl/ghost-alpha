use crate::error::ValuationError;
use serde::{Deserialize, Serialize};

/// Inputs for a 3-stage DCF model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DcfInputs {
    pub base_revenue: f64,
    pub base_fcf: f64,
    /// Stage 1: high growth (years 1-5)
    pub stage1_growth_rate: f64,
    pub stage1_years: u32,
    /// Stage 2: transition (years 6-10)
    pub stage2_growth_rate: f64,
    pub stage2_years: u32,
    /// Stage 3: terminal growth rate
    pub terminal_growth_rate: f64,
    /// FCF margin assumed at maturity
    pub terminal_fcf_margin: f64,
    /// Weighted average cost of capital
    pub wacc: f64,
    /// Shares outstanding (diluted)
    pub shares_outstanding: f64,
    /// Net debt (debt - cash)
    pub net_debt: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DcfResult {
    pub enterprise_value: f64,
    pub equity_value: f64,
    pub fair_value_per_share: f64,
    pub terminal_value: f64,
    pub terminal_pct_of_ev: f64,
    pub yearly_fcf: Vec<(u32, f64)>,
    /// Implied upside vs current price (caller sets this).
    pub implied_upside_pct: f64,
}

/// Run a 3-stage DCF valuation.
///
/// Stage 1: grow revenue at `stage1_growth_rate` for `stage1_years`.
/// Stage 2: linearly interpolate growth from stage1 rate down to stage2 rate.
/// Terminal: Gordon Growth Model.
pub fn three_stage_dcf(inputs: &DcfInputs) -> Result<DcfResult, ValuationError> {
    if inputs.wacc <= inputs.terminal_growth_rate {
        return Err(ValuationError::InvalidWacc(format!(
            "WACC ({:.4}) must exceed terminal growth rate ({:.4})",
            inputs.wacc, inputs.terminal_growth_rate
        )));
    }

    let mut fcfs = Vec::with_capacity((inputs.stage1_years + inputs.stage2_years) as usize);
    let mut revenue = inputs.base_revenue;
    let mut year = 0u32;

    // Stage 1: high growth
    for _ in 0..inputs.stage1_years {
        year += 1;
        revenue *= 1.0 + inputs.stage1_growth_rate;
        let fcf = revenue * inputs.terminal_fcf_margin;
        fcfs.push((year, fcf));
    }

    // Stage 2: linear interpolation of growth rate from stage1 down to stage2
    if inputs.stage2_years > 0 {
        let growth_step =
            (inputs.stage1_growth_rate - inputs.stage2_growth_rate) / inputs.stage2_years as f64;
        let mut growth = inputs.stage1_growth_rate;
        for _ in 0..inputs.stage2_years {
            year += 1;
            growth -= growth_step;
            revenue *= 1.0 + growth;
            let fcf = revenue * inputs.terminal_fcf_margin;
            fcfs.push((year, fcf));
        }
    }

    // PV of explicit-period FCFs
    let pv_fcfs: f64 = fcfs
        .iter()
        .map(|(y, fcf)| fcf / (1.0 + inputs.wacc).powi(*y as i32))
        .sum();

    // Terminal value (Gordon Growth Model)
    let last_fcf = fcfs.last().map(|(_, f)| *f).unwrap_or(inputs.base_fcf);
    let terminal_fcf = last_fcf * (1.0 + inputs.terminal_growth_rate);
    let terminal_value = terminal_fcf / (inputs.wacc - inputs.terminal_growth_rate);
    let pv_terminal = terminal_value / (1.0 + inputs.wacc).powi(year as i32);

    let ev = pv_fcfs + pv_terminal;
    let equity = ev - inputs.net_debt;
    let per_share = equity / inputs.shares_outstanding;

    Ok(DcfResult {
        enterprise_value: ev,
        equity_value: equity,
        fair_value_per_share: per_share,
        terminal_value: pv_terminal,
        terminal_pct_of_ev: if ev > 0.0 {
            pv_terminal / ev * 100.0
        } else {
            0.0
        },
        yearly_fcf: fcfs,
        implied_upside_pct: 0.0, // caller sets from current price
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    /// Helper: build a standard set of DCF inputs for testing.
    fn base_inputs() -> DcfInputs {
        DcfInputs {
            base_revenue: 1_000.0,
            base_fcf: 100.0,
            stage1_growth_rate: 0.20,
            stage1_years: 5,
            stage2_growth_rate: 0.10,
            stage2_years: 5,
            terminal_growth_rate: 0.03,
            terminal_fcf_margin: 0.15,
            wacc: 0.10,
            shares_outstanding: 100.0,
            net_debt: 200.0,
        }
    }

    #[test]
    fn test_wacc_below_terminal_returns_error() {
        let mut inputs = base_inputs();
        inputs.wacc = 0.03;
        inputs.terminal_growth_rate = 0.04;
        let result = three_stage_dcf(&inputs);
        assert!(matches!(result, Err(ValuationError::InvalidWacc(_))));
    }

    #[test]
    fn test_wacc_equals_terminal_returns_error() {
        let mut inputs = base_inputs();
        inputs.wacc = 0.04;
        inputs.terminal_growth_rate = 0.04;
        let result = three_stage_dcf(&inputs);
        assert!(matches!(result, Err(ValuationError::InvalidWacc(_))));
    }

    #[test]
    fn test_constant_growth_matches_gordon() {
        // When all growth stages use the same rate, the DCF should approximate
        // a single-stage Gordon Growth Model:
        //   FCF_1 / (WACC - g)
        let g = 0.05;
        let wacc = 0.10;
        let base_rev = 1_000.0;
        let margin = 0.15;
        let inputs = DcfInputs {
            base_revenue: base_rev,
            base_fcf: base_rev * margin,
            stage1_growth_rate: g,
            stage1_years: 5,
            stage2_growth_rate: g,
            stage2_years: 5,
            terminal_growth_rate: g,
            terminal_fcf_margin: margin,
            wacc,
            shares_outstanding: 100.0,
            net_debt: 0.0,
        };
        let result = three_stage_dcf(&inputs).unwrap();

        // Gordon Growth (perpetuity from year 0):
        // FCF_1 = base_rev * (1+g) * margin
        // EV = FCF_1 / (wacc - g)
        let fcf1 = base_rev * (1.0 + g) * margin;
        let gordon_ev = fcf1 / (wacc - g);

        // The 3-stage DCF with constant growth should be close to Gordon Growth.
        // Not exact because the explicit period is finite, but within a few percent.
        assert_relative_eq!(
            result.enterprise_value,
            gordon_ev,
            epsilon = gordon_ev * 0.01
        );
    }

    #[test]
    fn test_zero_growth() {
        let wacc = 0.10;
        let base_rev = 1_000.0;
        let margin = 0.10;
        let inputs = DcfInputs {
            base_revenue: base_rev,
            base_fcf: base_rev * margin,
            stage1_growth_rate: 0.0,
            stage1_years: 5,
            stage2_growth_rate: 0.0,
            stage2_years: 5,
            terminal_growth_rate: 0.0,
            terminal_fcf_margin: margin,
            wacc,
            shares_outstanding: 100.0,
            net_debt: 0.0,
        };
        let result = three_stage_dcf(&inputs).unwrap();
        // With 0% growth everywhere, FCF is flat = base_rev * margin = 100.
        // Gordon perpetuity = 100 / 0.10 = 1000 (from year 0).
        // The result should be close to 1000.
        assert_relative_eq!(result.enterprise_value, 1_000.0, epsilon = 5.0);
        assert!(result.fair_value_per_share > 0.0);
    }

    #[test]
    fn test_positive_net_debt_reduces_equity() {
        let mut inputs = base_inputs();
        inputs.net_debt = 0.0;
        let low_debt = three_stage_dcf(&inputs).unwrap();

        inputs.net_debt = 500.0;
        let high_debt = three_stage_dcf(&inputs).unwrap();

        assert!(
            low_debt.fair_value_per_share > high_debt.fair_value_per_share,
            "Higher net debt should reduce per-share value: {:.2} vs {:.2}",
            low_debt.fair_value_per_share,
            high_debt.fair_value_per_share,
        );
    }

    #[test]
    fn test_terminal_value_dominates() {
        // For a growth company, terminal value typically 60-90% of EV.
        let inputs = base_inputs();
        let result = three_stage_dcf(&inputs).unwrap();
        assert!(
            result.terminal_pct_of_ev >= 40.0 && result.terminal_pct_of_ev <= 95.0,
            "Terminal % of EV should be 40-95% for growth co, got {:.1}%",
            result.terminal_pct_of_ev,
        );
    }

    #[test]
    fn test_yearly_fcf_length() {
        let inputs = base_inputs();
        let result = three_stage_dcf(&inputs).unwrap();
        let expected_len = (inputs.stage1_years + inputs.stage2_years) as usize;
        assert_eq!(
            result.yearly_fcf.len(),
            expected_len,
            "Expected {} yearly FCF entries, got {}",
            expected_len,
            result.yearly_fcf.len(),
        );
    }

    #[test]
    fn test_higher_wacc_reduces_value() {
        let mut inputs = base_inputs();
        inputs.wacc = 0.08;
        let low_wacc = three_stage_dcf(&inputs).unwrap();

        inputs.wacc = 0.15;
        let high_wacc = three_stage_dcf(&inputs).unwrap();

        assert!(
            low_wacc.fair_value_per_share > high_wacc.fair_value_per_share,
            "Higher WACC should reduce value: {:.2} vs {:.2}",
            low_wacc.fair_value_per_share,
            high_wacc.fair_value_per_share,
        );
    }

    #[test]
    fn test_higher_growth_increases_value() {
        let mut inputs = base_inputs();
        inputs.stage1_growth_rate = 0.10;
        let low_growth = three_stage_dcf(&inputs).unwrap();

        inputs.stage1_growth_rate = 0.30;
        let high_growth = three_stage_dcf(&inputs).unwrap();

        assert!(
            high_growth.fair_value_per_share > low_growth.fair_value_per_share,
            "Higher growth should increase value: {:.2} vs {:.2}",
            high_growth.fair_value_per_share,
            low_growth.fair_value_per_share,
        );
    }
}
