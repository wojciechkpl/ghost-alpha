//! RED for Track A: `year_tax`, `TaxSettings` and `YearTax` live in `ghost_alpha::tax::year`, and
//! `portfolio::unwind` re-exports the very same items (same types, same function). Fails to compile
//! until the module exists.

use ghost_alpha::tax::brackets::{state_tax_rate, FederalCapGainsBrackets};
use ghost_alpha::tax::year::{year_tax, TaxSettings, YearTax};

fn settings() -> TaxSettings {
    TaxSettings {
        state: state_tax_rate("CA").unwrap(),
        brackets: FederalCapGainsBrackets::single_2024(),
        other_income: 100_000.0,
    }
}

#[test]
fn test_tax_year_module_when_called_should_apply_year_netting() {
    // ST loss -30,000 nets against LT +70,000 -> LT 40,000: 15% federal, 13.3% CA, no NIIT.
    let t: YearTax = year_tax(-30_000.0, 70_000.0, &settings());
    assert!((t.federal - 6_000.0).abs() < 1e-6);
    assert!((t.state - 5_320.0).abs() < 1e-6);
    assert!(t.niit.abs() < 1e-6);
    assert!((t.total - 11_320.0).abs() < 1e-6);
}

#[test]
fn test_unwind_reexport_should_be_the_same_items_as_tax_year() {
    // The same TaxSettings type: a unwind-path value is accepted where the tax::year path is expected.
    let s: ghost_alpha::portfolio::unwind::TaxSettings = settings();
    let via_tax: YearTax = year_tax(0.0, 50_000.0, &s);
    let via_unwind: ghost_alpha::portfolio::unwind::YearTax =
        ghost_alpha::portfolio::unwind::year_tax(0.0, 50_000.0, &s);
    assert_eq!(via_tax, via_unwind);

    // And the same function, not a copy.
    let a: fn(f64, f64, &TaxSettings) -> YearTax = year_tax;
    let b: fn(f64, f64, &TaxSettings) -> YearTax = ghost_alpha::portfolio::unwind::year_tax;
    assert_eq!(a as usize, b as usize);
}
