//! Whole-calendar-year capital-gains tax shared by the unwind page and the CLI projections.

use super::brackets::{FederalCapGainsBrackets, StateTaxRate};
use super::engine::bracket_tax;

/// Tax inputs from the user's profile.
#[derive(Debug, Clone)]
pub struct TaxSettings {
    pub state: StateTaxRate,
    pub brackets: FederalCapGainsBrackets,
    pub other_income: f64,
}

/// Tax for one calendar year (see [`year_tax`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct YearTax {
    pub federal: f64,
    pub state: f64,
    pub niit: f64,
    pub total: f64,
}

/// Tax for ONE calendar year from its realised short- and long-term gain totals
/// (either may be negative; no carryforwards and no $3,000 offset).
///
/// Short and long term net against each other. Federal tax is the ordinary-bracket
/// tax on net short-term gain stacked on other income, plus the long-term bracket
/// tax on net long-term gain stacked on other income plus net short-term gain. NIIT
/// is `rate * min(net, max(0, other_income + net - threshold))`. States with
/// `long_term_only` (Washington) tax only the long-term total above the deduction;
/// other states tax the net gain above the deduction. On a net loss there is no
/// federal tax or NIIT, and non-long-term-only states owe nothing; a long-term-only
/// state still taxes a positive long-term gain, as short-term losses do not reduce it.
pub fn year_tax(st_gain: f64, lt_gain: f64, tax: &TaxSettings) -> YearTax {
    let net = st_gain + lt_gain;
    let s = &tax.state;
    let taxable = if s.long_term_only {
        (lt_gain - s.annual_deduction).max(0.0)
    } else {
        (net - s.annual_deduction).max(0.0)
    };
    let state = if taxable.is_nan() || taxable <= 0.0 || (s.rate == 0.0 && s.brackets.is_empty()) {
        0.0
    } else if s.brackets.is_empty() {
        taxable * s.rate
    } else {
        bracket_tax(taxable, &s.brackets, 0.0)
    };
    if net.is_nan() || net <= 0.0 {
        // Federal tax and NIIT need a net gain; a long-term-only state (Washington)
        // still taxes the long-term gain that short-term losses do not reduce.
        return YearTax {
            federal: 0.0,
            state,
            niit: 0.0,
            total: state,
        };
    }
    // IRS-style netting: a loss on one side offsets the other.
    let (st, lt) = if st_gain < 0.0 {
        (0.0, lt_gain + st_gain)
    } else if lt_gain < 0.0 {
        (st_gain + lt_gain, 0.0)
    } else {
        (st_gain, lt_gain)
    };
    let b = &tax.brackets;
    let income = tax.other_income;
    let federal = bracket_tax(st, &b.short_term_brackets, income)
        + bracket_tax(lt, &b.long_term_brackets, income + st);
    let niit = b.niit_rate * net.min((income + net - b.niit_threshold).max(0.0));
    YearTax {
        federal,
        state,
        niit,
        total: federal + state + niit,
    }
}
