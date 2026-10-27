use crate::error::TaxError;

/// Federal capital gains tax brackets for a given year.
#[derive(Debug, Clone)]
pub struct FederalCapGainsBrackets {
    pub year: u32,
    pub filing_status: FilingStatus,
    /// (upper_bound, rate) pairs, ascending. Last entry has upper_bound = f64::INFINITY.
    pub long_term_brackets: Vec<(f64, f64)>,
    /// Same as ordinary income brackets.
    pub short_term_brackets: Vec<(f64, f64)>,
    /// Net Investment Income Tax threshold.
    pub niit_threshold: f64,
    /// NIIT rate (typically 3.8%).
    pub niit_rate: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilingStatus {
    Single,
    MarriedFilingJointly,
    MarriedFilingSeparately,
    HeadOfHousehold,
}

/// A tiered tax schedule: Vec of (upper_bound, rate), ascending.
/// Last entry must have upper_bound = f64::INFINITY.
/// Identical format to FederalCapGainsBrackets::long_term_brackets.
pub type TaxBracketSchedule = Vec<(f64, f64)>;

/// State tax rates — supports flat rate or tiered brackets.
#[derive(Debug, Clone)]
pub struct StateTaxRate {
    pub state: String,
    /// Flat rate used for states with a single tier. For computation when
    /// `brackets` is non-empty, `brackets` takes precedence.
    pub rate: f64,
    pub has_ltcg_preference: bool,
    /// Annual deduction applied before state cap-gains tax (e.g. WA's $278K).
    /// Gains up to this amount each year are exempt from state tax.
    pub annual_deduction: f64,
    /// True when the state taxes long-term gains only (Washington's excise tax):
    /// short-term gains owe no state tax and use none of the deduction.
    pub long_term_only: bool,
    /// Tiered bracket schedule applied to gains above the deduction.
    /// Empty vec = use flat `rate`. Non-empty = marginal brackets.
    pub brackets: TaxBracketSchedule,
}

/// Parse a filing-status name as accepted by the CLI (`single`, `mfj`, `mfs`,
/// `hoh` and their long forms), case-insensitively.
pub fn parse_filing_status(s: &str) -> Option<FilingStatus> {
    match s.trim().to_lowercase().as_str() {
        "single" | "s" => Some(FilingStatus::Single),
        "mfj" | "married" | "joint" => Some(FilingStatus::MarriedFilingJointly),
        "mfs" | "separate" => Some(FilingStatus::MarriedFilingSeparately),
        "hoh" | "head" => Some(FilingStatus::HeadOfHousehold),
        _ => None,
    }
}

impl FederalCapGainsBrackets {
    /// 2024 brackets for `status` (separate and head-of-household use the
    /// single schedule, as the CLI does).
    pub fn for_filing(status: &FilingStatus) -> Self {
        match status {
            FilingStatus::MarriedFilingJointly => Self::mfj_2024(),
            _ => Self::single_2024(),
        }
    }

    /// 2024 brackets for married filing jointly.
    pub fn mfj_2024() -> Self {
        Self {
            year: 2024,
            filing_status: FilingStatus::MarriedFilingJointly,
            long_term_brackets: vec![(94_050.0, 0.0), (583_750.0, 0.15), (f64::INFINITY, 0.20)],
            short_term_brackets: vec![
                (23_200.0, 0.10),
                (94_300.0, 0.12),
                (201_050.0, 0.22),
                (383_900.0, 0.24),
                (487_450.0, 0.32),
                (731_200.0, 0.35),
                (f64::INFINITY, 0.37),
            ],
            niit_threshold: 250_000.0,
            niit_rate: 0.038,
        }
    }

    /// 2024 brackets for single filer.
    pub fn single_2024() -> Self {
        Self {
            year: 2024,
            filing_status: FilingStatus::Single,
            long_term_brackets: vec![(47_025.0, 0.0), (518_900.0, 0.15), (f64::INFINITY, 0.20)],
            short_term_brackets: vec![
                (11_600.0, 0.10),
                (47_150.0, 0.12),
                (100_525.0, 0.22),
                (191_950.0, 0.24),
                (243_725.0, 0.32),
                (609_350.0, 0.35),
                (f64::INFINITY, 0.37),
            ],
            niit_threshold: 200_000.0,
            niit_rate: 0.038,
        }
    }
}

/// Look up state capital gains tax rate.
pub fn state_tax_rate(state_code: &str) -> Result<StateTaxRate, TaxError> {
    let flat = |state: &str, rate: f64| StateTaxRate {
        state: state.into(),
        rate,
        has_ltcg_preference: false,
        annual_deduction: 0.0,
        long_term_only: false,
        brackets: vec![],
    };
    match state_code {
        "CA" => Ok(StateTaxRate {
            rate: 0.133,
            ..flat("CA", 0.133)
        }),
        "NY" => Ok(StateTaxRate {
            rate: 0.109,
            ..flat("NY", 0.109)
        }),
        "TX" => Ok(flat("TX", 0.0)),
        "FL" => Ok(flat("FL", 0.0)),
        "WA" => Ok(StateTaxRate {
            state: "WA".into(),
            rate: 0.07,
            has_ltcg_preference: false,
            annual_deduction: 278_000.0,
            long_term_only: true,
            brackets: vec![
                (722_000.0, 0.07),      // $278K + $722K = $1M boundary
                (f64::INFINITY, 0.099), // 9.9% above $1M
            ],
        }),
        "NV" => Ok(flat("NV", 0.0)),
        "IL" => Ok(flat("IL", 0.0495)),
        "NJ" => Ok(flat("NJ", 0.1075)),
        "MA" => Ok(StateTaxRate {
            has_ltcg_preference: true,
            ..flat("MA", 0.09)
        }),
        "CO" => Ok(flat("CO", 0.044)),
        _ => Err(TaxError::UnknownState(state_code.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mfj_2024_has_three_ltcg_brackets() {
        let b = FederalCapGainsBrackets::mfj_2024();
        assert_eq!(b.long_term_brackets.len(), 3);
        assert_eq!(b.filing_status, FilingStatus::MarriedFilingJointly);
        assert_eq!(b.year, 2024);
    }

    #[test]
    fn test_single_2024_has_seven_stcg_brackets() {
        let b = FederalCapGainsBrackets::single_2024();
        assert_eq!(b.short_term_brackets.len(), 7);
        assert_eq!(b.filing_status, FilingStatus::Single);
    }

    #[test]
    fn test_niit_threshold_mfj() {
        let b = FederalCapGainsBrackets::mfj_2024();
        assert!((b.niit_threshold - 250_000.0).abs() < f64::EPSILON);
        assert!((b.niit_rate - 0.038).abs() < f64::EPSILON);
    }

    #[test]
    fn test_niit_threshold_single() {
        let b = FederalCapGainsBrackets::single_2024();
        assert!((b.niit_threshold - 200_000.0).abs() < f64::EPSILON);
        assert!((b.niit_rate - 0.038).abs() < f64::EPSILON);
    }

    #[test]
    fn test_state_tax_ca() {
        let s = state_tax_rate("CA").unwrap();
        assert!((s.rate - 0.133).abs() < f64::EPSILON);
        assert_eq!(s.state, "CA");
        assert!(!s.has_ltcg_preference);
    }

    #[test]
    fn test_state_tax_tx_zero() {
        let s = state_tax_rate("TX").unwrap();
        assert!((s.rate - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_state_tax_unknown_returns_error() {
        let result = state_tax_rate("ZZ");
        assert_eq!(
            result.unwrap_err(),
            TaxError::UnknownState("ZZ".to_string())
        );
    }

    // ── Item 1: WA tiered brackets ─────────────────────────────────────

    #[test]
    fn test_wa_2025_deduction_is_278k() {
        let s = state_tax_rate("WA").unwrap();
        assert!((s.annual_deduction - 278_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_wa_2025_has_two_brackets() {
        let s = state_tax_rate("WA").unwrap();
        assert_eq!(s.brackets.len(), 2);
    }

    #[test]
    fn test_wa_bracket_rates() {
        let s = state_tax_rate("WA").unwrap();
        assert!((s.brackets[0].1 - 0.07).abs() < f64::EPSILON);
        assert!((s.brackets[1].1 - 0.099).abs() < f64::EPSILON);
    }

    #[test]
    fn test_wa_bracket_thresholds() {
        let s = state_tax_rate("WA").unwrap();
        // $1M - $278K deduction = $722K in bracket space
        assert!((s.brackets[0].0 - 722_000.0).abs() < f64::EPSILON);
        assert!(s.brackets[1].0.is_infinite());
    }

    #[test]
    fn test_flat_rate_states_have_empty_brackets() {
        let ca = state_tax_rate("CA").unwrap();
        assert!(ca.brackets.is_empty());
    }

    #[test]
    fn test_zero_rate_states_have_empty_brackets() {
        let tx = state_tax_rate("TX").unwrap();
        assert!(tx.brackets.is_empty());
    }
}
