use super::brackets::{FederalCapGainsBrackets, StateTaxRate};
use crate::portfolio::types::TaxLot;

/// Result of computing tax on a disposal or hypothetical sale.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaxImpact {
    pub proceeds: f64,
    pub cost_basis: f64,
    pub realised_gain: f64,
    pub is_long_term: bool,
    pub federal_tax: f64,
    pub state_tax: f64,
    pub niit: f64,
    pub total_tax: f64,
    pub effective_rate: f64,
    pub after_tax_proceeds: f64,
}

/// Compute the total tax impact of selling `shares_to_sell` of a specific lot.
///
/// `state_gains_ytd` tracks how much capital gain has already been realised
/// in the same tax year. This is needed for states with annual deductions
/// (e.g. Washington's $250K LTCG exemption). Pass `0.0` when this is the
/// first sale of the year or when the state has no deduction.
#[allow(clippy::too_many_arguments)]
pub fn compute_lot_tax(
    lot: &TaxLot,
    shares_to_sell: f64,
    sale_price: f64,
    sale_date: chrono::NaiveDate,
    other_income: f64,
    brackets: &FederalCapGainsBrackets,
    state: &StateTaxRate,
    state_gains_ytd: f64,
) -> TaxImpact {
    let is_lt = lot.is_long_term(sale_date);
    let cost_basis = shares_to_sell * lot.cost_basis_per_share;
    let proceeds = shares_to_sell * sale_price;
    let gain = proceeds - cost_basis;

    let federal_tax = if gain <= 0.0 {
        0.0
    } else if is_lt {
        bracket_tax(gain, &brackets.long_term_brackets, other_income)
    } else {
        bracket_tax(gain, &brackets.short_term_brackets, other_income)
    };

    let niit = if gain > 0.0 && other_income + gain > brackets.niit_threshold {
        gain * brackets.niit_rate
    } else {
        0.0
    };

    // State tax with annual deduction support.
    // Only the portion of cumulative gains exceeding the deduction is taxed.
    // Marginal approach: tax on (ytd + gain) minus tax already owed on (ytd).
    let state_tax = if gain <= 0.0 || (state.rate == 0.0 && state.brackets.is_empty()) {
        0.0
    } else {
        let ded = state.annual_deduction;
        let already_taxable = (state_gains_ytd - ded).max(0.0);
        let now_taxable = (state_gains_ytd + gain - ded).max(0.0);
        let marginal_gain = now_taxable - already_taxable;
        if state.brackets.is_empty() {
            // Flat rate state (CA, IL, NJ, etc.)
            marginal_gain * state.rate
        } else {
            // Tiered state (WA) — reuse bracket_tax with stacking
            bracket_tax(marginal_gain, &state.brackets, already_taxable)
        }
    };

    let total_tax = federal_tax + niit + state_tax;

    TaxImpact {
        proceeds,
        cost_basis,
        realised_gain: gain,
        is_long_term: is_lt,
        federal_tax,
        state_tax,
        niit,
        total_tax,
        effective_rate: if gain > 0.0 { total_tax / gain } else { 0.0 },
        after_tax_proceeds: proceeds - total_tax,
    }
}

/// Marginal bracket tax computation with stacking.
///
/// `gain` is stacked on top of `base_income` and taxed through
/// the provided brackets. Each bracket is `(upper_bound, rate)`.
pub(crate) fn bracket_tax(gain: f64, brackets: &[(f64, f64)], base_income: f64) -> f64 {
    let mut remaining = gain;
    let mut tax = 0.0;
    let mut lower = 0.0_f64;

    for &(upper, rate) in brackets {
        if remaining <= 0.0 {
            break;
        }
        if base_income >= upper {
            lower = upper;
            continue;
        }
        let bracket_start = lower.max(base_income);
        let space = upper - bracket_start;
        let taxable = remaining.min(space);
        tax += taxable * rate;
        remaining -= taxable;
        lower = upper;
    }
    tax
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::types::{AcquisitionType, TaxLot};
    use crate::tax::brackets::{state_tax_rate, FederalCapGainsBrackets};
    use chrono::NaiveDate;
    use uuid::Uuid;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn make_lot(cost: f64, date: &str, shares: f64) -> TaxLot {
        TaxLot {
            id: Uuid::new_v4(),
            ticker: "TEST".to_string(),
            shares,
            cost_basis_per_share: cost,
            acquired_date: d(date),
            acquisition_type: AcquisitionType::Buy,
            remaining_shares: shares,
        }
    }

    fn tx_state() -> StateTaxRate {
        state_tax_rate("TX").unwrap()
    }

    fn ca_state() -> StateTaxRate {
        state_tax_rate("CA").unwrap()
    }

    // ── Tests ────────────────────────────────────────────────────────────

    #[test]
    fn test_zero_gain_zero_tax() {
        let lot = make_lot(100.0, "2020-01-01", 100.0);
        let b = FederalCapGainsBrackets::single_2024();
        let impact = compute_lot_tax(
            &lot,
            50.0,
            100.0,
            d("2024-06-01"),
            0.0,
            &b,
            &tx_state(),
            0.0,
        );
        assert!((impact.realised_gain).abs() < f64::EPSILON);
        assert!((impact.federal_tax).abs() < f64::EPSILON);
        assert!((impact.state_tax).abs() < f64::EPSILON);
        assert!((impact.niit).abs() < f64::EPSILON);
        assert!((impact.total_tax).abs() < f64::EPSILON);
    }

    #[test]
    fn test_loss_no_tax() {
        let lot = make_lot(150.0, "2020-01-01", 100.0);
        let b = FederalCapGainsBrackets::single_2024();
        let impact = compute_lot_tax(
            &lot,
            50.0,
            100.0,
            d("2024-06-01"),
            0.0,
            &b,
            &tx_state(),
            0.0,
        );
        assert!(impact.realised_gain < 0.0);
        assert!((impact.total_tax).abs() < f64::EPSILON);
    }

    #[test]
    fn test_long_term_lower_rate() {
        let b = FederalCapGainsBrackets::single_2024();
        let gain_amount = 100_000.0;

        // Long-term: bought 2020, sold 2024
        let lt_lot = make_lot(100.0, "2020-01-01", 1000.0);
        let lt = compute_lot_tax(
            &lt_lot,
            1000.0,
            200.0,
            d("2024-06-01"),
            0.0,
            &b,
            &tx_state(),
            0.0,
        );

        // Short-term: bought 2024-01, sold 2024-06
        let st_lot = make_lot(100.0, "2024-01-01", 1000.0);
        let st = compute_lot_tax(
            &st_lot,
            1000.0,
            200.0,
            d("2024-06-01"),
            0.0,
            &b,
            &tx_state(),
            0.0,
        );

        assert!((lt.realised_gain - gain_amount).abs() < f64::EPSILON);
        assert!((st.realised_gain - gain_amount).abs() < f64::EPSILON);
        assert!(
            lt.federal_tax < st.federal_tax,
            "LT tax should be lower than ST tax"
        );
    }

    #[test]
    fn test_bracket_stacking_with_other_income() {
        let b = FederalCapGainsBrackets::single_2024();
        let lot = make_lot(100.0, "2020-01-01", 100.0);

        // No other income
        let low = compute_lot_tax(
            &lot,
            100.0,
            200.0,
            d("2024-06-01"),
            0.0,
            &b,
            &tx_state(),
            0.0,
        );
        // $200K other income pushes gain into higher bracket
        let high = compute_lot_tax(
            &lot,
            100.0,
            200.0,
            d("2024-06-01"),
            200_000.0,
            &b,
            &tx_state(),
            0.0,
        );

        assert!(
            high.federal_tax > low.federal_tax,
            "Higher base income should push gain into higher brackets"
        );
    }

    #[test]
    fn test_niit_applies_above_threshold() {
        let b = FederalCapGainsBrackets::single_2024();
        // threshold = 200K. other_income=180K, gain=50K => total=230K > 200K
        let lot = make_lot(100.0, "2020-01-01", 500.0);
        let impact = compute_lot_tax(
            &lot,
            500.0,
            200.0,
            d("2024-06-01"),
            180_000.0,
            &b,
            &tx_state(),
            0.0,
        );
        let expected_niit = 50_000.0 * 0.038;
        assert!(
            (impact.niit - expected_niit).abs() < 0.01,
            "NIIT should be gain * 3.8%, got {}",
            impact.niit
        );
    }

    #[test]
    fn test_niit_does_not_apply_below_threshold() {
        let b = FederalCapGainsBrackets::single_2024();
        // threshold = 200K. other_income=0, gain=10K => total=10K < 200K
        let lot = make_lot(100.0, "2020-01-01", 100.0);
        let impact = compute_lot_tax(
            &lot,
            100.0,
            200.0,
            d("2024-06-01"),
            0.0,
            &b,
            &tx_state(),
            0.0,
        );
        assert!((impact.niit).abs() < f64::EPSILON);
    }

    #[test]
    fn test_state_tax_proportional() {
        let b = FederalCapGainsBrackets::single_2024();
        let lot = make_lot(100.0, "2020-01-01", 100.0);
        let impact = compute_lot_tax(
            &lot,
            100.0,
            200.0,
            d("2024-06-01"),
            0.0,
            &b,
            &ca_state(),
            0.0,
        );
        let expected_state = 10_000.0 * 0.133;
        assert!((impact.state_tax - expected_state).abs() < 0.01);
    }

    #[test]
    fn test_effective_rate_bounded() {
        let b = FederalCapGainsBrackets::single_2024();
        // Huge gain to push into top brackets
        let lot = make_lot(1.0, "2020-01-01", 1_000_000.0);
        let impact = compute_lot_tax(
            &lot,
            1_000_000.0,
            100.0,
            d("2024-06-01"),
            500_000.0,
            &b,
            &ca_state(),
            0.0,
        );
        assert!(impact.effective_rate >= 0.0, "Rate should be >= 0");
        assert!(
            impact.effective_rate < 1.0,
            "Rate should be < 1, got {}",
            impact.effective_rate
        );

        // Zero gain
        let lot2 = make_lot(100.0, "2020-01-01", 100.0);
        let impact2 = compute_lot_tax(
            &lot2,
            100.0,
            100.0,
            d("2024-06-01"),
            0.0,
            &b,
            &ca_state(),
            0.0,
        );
        assert!((impact2.effective_rate).abs() < f64::EPSILON);
    }

    #[test]
    fn test_after_tax_proceeds_positive() {
        let b = FederalCapGainsBrackets::single_2024();
        let lot = make_lot(50.0, "2020-01-01", 100.0);
        let impact = compute_lot_tax(
            &lot,
            100.0,
            200.0,
            d("2024-06-01"),
            0.0,
            &b,
            &ca_state(),
            0.0,
        );
        assert!(
            impact.after_tax_proceeds > 0.0,
            "After-tax proceeds should be positive"
        );
        assert!(impact.after_tax_proceeds <= impact.proceeds);
    }

    #[test]
    fn test_wa_deduction_exempts_gains_under_278k() {
        let b = FederalCapGainsBrackets::single_2024();
        let wa = state_tax_rate("WA").unwrap();
        // $200K gain, no prior YTD gains => entirely under $278K deduction
        let lot = make_lot(100.0, "2020-01-01", 2000.0);
        let impact = compute_lot_tax(&lot, 2000.0, 200.0, d("2024-06-01"), 0.0, &b, &wa, 0.0);
        assert!(
            (impact.state_tax).abs() < f64::EPSILON,
            "WA should exempt $200K gain (under $278K deduction), got {}",
            impact.state_tax
        );
    }

    #[test]
    fn test_wa_tiered_first_bracket_only() {
        let b = FederalCapGainsBrackets::single_2024();
        let wa = state_tax_rate("WA").unwrap();
        // $500K gain => taxable = $222K ($500K - $278K), all at 7%
        let lot = make_lot(10.0, "2020-01-01", 2000.0);
        let impact = compute_lot_tax(&lot, 2000.0, 260.0, d("2024-06-01"), 0.0, &b, &wa, 0.0);
        let expected_state = 222_000.0 * 0.07; // $15,540
        assert!(
            (impact.state_tax - expected_state).abs() < 0.01,
            "WA tier 1: $222K at 7% = $15,540, got {}",
            impact.state_tax
        );
    }

    #[test]
    fn test_wa_tiered_crosses_into_second_bracket() {
        let b = FederalCapGainsBrackets::single_2024();
        let wa = state_tax_rate("WA").unwrap();
        // $1.5M gain => taxable = $1,222K ($1.5M - $278K)
        // First $722K at 7% = $50,540; remaining $500K at 9.9% = $49,500
        let lot = make_lot(10.0, "2020-01-01", 10000.0);
        let impact = compute_lot_tax(&lot, 10000.0, 160.0, d("2024-06-01"), 0.0, &b, &wa, 0.0);
        let expected_state = 722_000.0 * 0.07 + 500_000.0 * 0.099; // $50,540 + $49,500 = $100,040
        assert!(
            (impact.state_tax - expected_state).abs() < 1.0,
            "WA tiered: $722K@7% + $500K@9.9% = ${expected_state:.0}, got {}",
            impact.state_tax
        );
    }

    #[test]
    fn test_wa_tiered_marginal_with_ytd_gains() {
        let b = FederalCapGainsBrackets::single_2024();
        let wa = state_tax_rate("WA").unwrap();
        // YTD = $300K (above $278K deduction, so $22K already taxable)
        // New gain = $100K => cumulative taxable goes from $22K to $122K
        // Marginal $100K all within first bracket (< $722K) at 7%
        let lot = make_lot(100.0, "2020-01-01", 1000.0);
        let impact = compute_lot_tax(
            &lot,
            1000.0,
            200.0,
            d("2024-06-01"),
            0.0,
            &b,
            &wa,
            300_000.0,
        );
        let expected_state = 100_000.0 * 0.07; // $7,000
        assert!(
            (impact.state_tax - expected_state).abs() < 0.01,
            "WA marginal: $100K in tier 1 at 7% = $7,000, got {}",
            impact.state_tax
        );
    }

    #[test]
    fn test_wa_tiered_marginal_crossing_tier2() {
        let b = FederalCapGainsBrackets::single_2024();
        let wa = state_tax_rate("WA").unwrap();
        // YTD = $900K (taxable = $622K, within tier 1)
        // New gain = $200K => taxable goes from $622K to $822K
        // $100K in tier 1 ($622K→$722K) at 7% = $7,000
        // $100K in tier 2 ($722K→$822K) at 9.9% = $9,900
        let lot = make_lot(100.0, "2020-01-01", 2000.0);
        let impact = compute_lot_tax(
            &lot,
            2000.0,
            200.0,
            d("2024-06-01"),
            0.0,
            &b,
            &wa,
            900_000.0,
        );
        let expected_state = 100_000.0 * 0.07 + 100_000.0 * 0.099; // $16,900
        assert!(
            (impact.state_tax - expected_state).abs() < 0.01,
            "WA marginal crossing tier 2: expected $16,900, got {}",
            impact.state_tax
        );
    }

    #[test]
    fn test_flat_state_unchanged() {
        let b = FederalCapGainsBrackets::single_2024();
        let lot = make_lot(100.0, "2020-01-01", 100.0);
        let impact = compute_lot_tax(
            &lot,
            100.0,
            200.0,
            d("2024-06-01"),
            0.0,
            &b,
            &ca_state(),
            0.0,
        );
        let expected_state = 10_000.0 * 0.133;
        assert!(
            (impact.state_tax - expected_state).abs() < 0.01,
            "CA flat rate unchanged: expected {expected_state}, got {}",
            impact.state_tax
        );
    }

    #[test]
    fn test_zero_state_unchanged() {
        let b = FederalCapGainsBrackets::single_2024();
        let lot = make_lot(100.0, "2020-01-01", 100.0);
        let impact = compute_lot_tax(
            &lot,
            100.0,
            200.0,
            d("2024-06-01"),
            0.0,
            &b,
            &tx_state(),
            0.0,
        );
        assert!(
            (impact.state_tax).abs() < f64::EPSILON,
            "TX should have $0 state tax"
        );
    }

    #[test]
    fn test_bracket_tax_known_values() {
        // Single 2024 LTCG: $50K gain, $0 other income
        // First $47,025 at 0% = $0
        // Remaining $2,975 at 15% = $446.25
        let b = FederalCapGainsBrackets::single_2024();
        let result = bracket_tax(50_000.0, &b.long_term_brackets, 0.0);
        let expected = 2_975.0 * 0.15;
        assert!(
            (result - expected).abs() < 0.01,
            "Expected {expected}, got {result}"
        );
    }
}
