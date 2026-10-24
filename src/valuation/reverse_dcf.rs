use crate::error::ValuationError;
use crate::valuation::dcf::{three_stage_dcf, DcfInputs};

/// Given a current stock price, solve for the implied stage-1 revenue growth rate
/// using bisection.
///
/// Returns the implied growth rate (e.g. 0.20 for 20%).
pub fn reverse_dcf(
    current_price: f64,
    base_inputs: &DcfInputs,
    tolerance: f64,
    max_iterations: u32,
) -> Result<f64, ValuationError> {
    let mut lo = -0.10;
    let mut hi = 0.80;
    let target_ev = current_price * base_inputs.shares_outstanding + base_inputs.net_debt;

    for _ in 0..max_iterations {
        let mid = (lo + hi) / 2.0;
        let mut inputs = base_inputs.clone();
        inputs.stage1_growth_rate = mid;
        let result = three_stage_dcf(&inputs)?;
        let diff = result.enterprise_value - target_ev;

        if target_ev.abs() > f64::EPSILON && (diff.abs() / target_ev.abs()) < tolerance {
            return Ok(mid);
        }

        if diff > 0.0 {
            hi = mid;
        } else {
            lo = mid;
        }
    }

    Err(ValuationError::NoConvergence {
        iterations: max_iterations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::valuation::dcf::DcfInputs;
    use approx::assert_abs_diff_eq;

    fn base_inputs() -> DcfInputs {
        DcfInputs {
            base_revenue: 1_000.0,
            base_fcf: 150.0,
            stage1_growth_rate: 0.20, // will be overwritten by reverse_dcf
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
    fn test_reverse_dcf_recovers_growth() {
        let target_growth = 0.20;
        let mut inputs = base_inputs();
        inputs.stage1_growth_rate = target_growth;

        // Forward: get the fair value at 20% growth
        let forward = three_stage_dcf(&inputs).unwrap();
        let price = forward.fair_value_per_share;

        // Reverse: recover the implied growth from that price
        let implied = reverse_dcf(price, &base_inputs(), 0.001, 100).unwrap();

        assert_abs_diff_eq!(implied, target_growth, epsilon = 0.01);
    }

    #[test]
    fn test_reverse_dcf_no_convergence() {
        let result = reverse_dcf(100.0, &base_inputs(), 1e-15, 1);
        assert!(matches!(
            result,
            Err(ValuationError::NoConvergence { iterations: 1 })
        ));
    }

    #[test]
    fn test_reverse_dcf_low_price_implies_low_growth() {
        let inputs = base_inputs();
        let low_price_growth = reverse_dcf(10.0, &inputs, 0.001, 100).unwrap();
        let high_price_growth = reverse_dcf(100.0, &inputs, 0.001, 100).unwrap();

        assert!(
            low_price_growth < high_price_growth,
            "Lower price should imply lower growth: {:.4} vs {:.4}",
            low_price_growth,
            high_price_growth,
        );
    }

    #[test]
    fn test_reverse_dcf_high_price_implies_high_growth() {
        let inputs = base_inputs();
        let moderate = reverse_dcf(50.0, &inputs, 0.001, 100).unwrap();
        let expensive = reverse_dcf(150.0, &inputs, 0.001, 100).unwrap();

        assert!(
            expensive > moderate,
            "Higher price should imply higher growth: {:.4} vs {:.4}",
            expensive,
            moderate,
        );
    }
}
