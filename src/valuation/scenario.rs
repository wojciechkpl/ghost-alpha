use crate::error::ValuationError;
use crate::valuation::dcf::{three_stage_dcf, DcfInputs, DcfResult};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Scenario {
    pub name: String,
    pub probability: f64,
    pub dcf_inputs: DcfInputs,
    pub result: DcfResult,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScenarioAnalysis {
    pub scenarios: Vec<Scenario>,
    /// Probability-weighted fair value per share.
    pub expected_value: f64,
    pub current_price: f64,
    pub expected_upside_pct: f64,
}

/// Run probability-weighted scenario analysis.
///
/// Each tuple is `(name, probability, dcf_inputs)`.
/// Returns the weighted expected fair value and upside vs `current_price`.
pub fn run_scenarios(
    scenarios: Vec<(String, f64, DcfInputs)>,
    current_price: f64,
) -> Result<ScenarioAnalysis, ValuationError> {
    if scenarios.is_empty() {
        return Err(ValuationError::MissingInput(
            "at least one scenario is required".into(),
        ));
    }

    let mut results = Vec::with_capacity(scenarios.len());
    for (name, prob, inputs) in scenarios {
        let result = three_stage_dcf(&inputs)?;
        results.push(Scenario {
            name,
            probability: prob,
            dcf_inputs: inputs,
            result,
        });
    }

    let expected_value: f64 = results
        .iter()
        .map(|s| s.probability * s.result.fair_value_per_share)
        .sum();

    let expected_upside_pct = if current_price.abs() > f64::EPSILON {
        (expected_value - current_price) / current_price * 100.0
    } else {
        0.0
    };

    Ok(ScenarioAnalysis {
        scenarios: results,
        expected_value,
        current_price,
        expected_upside_pct,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    fn base_inputs(growth: f64) -> DcfInputs {
        DcfInputs {
            base_revenue: 1_000.0,
            base_fcf: 150.0,
            stage1_growth_rate: growth,
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
    fn test_single_scenario_100pct() {
        let inputs = base_inputs(0.20);
        let single_result = three_stage_dcf(&inputs).unwrap();
        let expected_fv = single_result.fair_value_per_share;

        let analysis = run_scenarios(vec![("Base".into(), 1.0, inputs)], 50.0).unwrap();

        assert_relative_eq!(analysis.expected_value, expected_fv, epsilon = 0.01);
    }

    #[test]
    fn test_probabilities_weight_correctly() {
        let bear = base_inputs(0.05);
        let base = base_inputs(0.15);
        let bull = base_inputs(0.30);

        let bear_fv = three_stage_dcf(&bear).unwrap().fair_value_per_share;
        let base_fv = three_stage_dcf(&base).unwrap().fair_value_per_share;
        let bull_fv = three_stage_dcf(&bull).unwrap().fair_value_per_share;

        let expected_weighted = 0.25 * bear_fv + 0.50 * base_fv + 0.25 * bull_fv;

        let analysis = run_scenarios(
            vec![
                ("Bear".into(), 0.25, base_inputs(0.05)),
                ("Base".into(), 0.50, base_inputs(0.15)),
                ("Bull".into(), 0.25, base_inputs(0.30)),
            ],
            50.0,
        )
        .unwrap();

        assert_relative_eq!(analysis.expected_value, expected_weighted, epsilon = 0.01);
    }

    #[test]
    fn test_upside_calculation() {
        let current_price = 50.0;
        let analysis =
            run_scenarios(vec![("Base".into(), 1.0, base_inputs(0.20))], current_price).unwrap();

        let expected_upside = (analysis.expected_value - current_price) / current_price * 100.0;
        assert_relative_eq!(
            analysis.expected_upside_pct,
            expected_upside,
            epsilon = 0.01
        );
    }

    #[test]
    fn test_empty_scenarios() {
        let result = run_scenarios(vec![], 50.0);
        assert!(matches!(result, Err(ValuationError::MissingInput(_))));
    }
}
