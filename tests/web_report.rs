//! RED-phase contract: the standalone diversification report (`ghost-alpha div-report`, produced
//! by `web::diversification_report::generate_diversification_report`) is a self-contained file,
//! so it must DEFINE every `--ga-*` custom property its markup references (inline the token
//! block in its own `<style>`). Hex literals inside that `<style>` are fine.

use chrono::NaiveDate;
use ghost_alpha::analysis::geopolitical::GeoRiskInputs;
use ghost_alpha::portfolio::allocation::Brokerage;
use ghost_alpha::portfolio::prepare::{prepare_selling_programme, SellDistribution, SellPlanInput};
use ghost_alpha::portfolio::types::{AcquisitionType, TaxLot};
use ghost_alpha::tax::brackets::{state_tax_rate, FederalCapGainsBrackets, FilingStatus};
use ghost_alpha::tax::projection::compare_scenarios;
use ghost_alpha::web::diversification_report::{
    generate_diversification_report, DiversificationReportInput,
};

fn sample_input() -> DiversificationReportInput {
    let acquired = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
    let plan = SellPlanInput {
        ticker: "XYZ".into(),
        total_shares: 1000.0,
        cost_basis_per_share: 100.0,
        current_price: 200.0,
        acquired_date: acquired,
        filing_status: FilingStatus::Single,
        state_code: "CA".into(),
        other_annual_income: 150_000.0,
        assumed_price_growth: 0.0,
        horizon_years: 2,
        start_year: 2026,
        distribution: SellDistribution::Even,
    };
    let sell_plan = prepare_selling_programme(&plan).unwrap();
    let lot = TaxLot {
        id: uuid::Uuid::new_v4(),
        ticker: "XYZ".into(),
        shares: 1000.0,
        cost_basis_per_share: 100.0,
        acquired_date: acquired,
        acquisition_type: AcquisitionType::RsuVest,
        remaining_shares: 1000.0,
    };
    let scenarios = compare_scenarios(
        &[lot],
        1000.0,
        5,
        2026,
        0.0,
        200.0,
        150_000.0,
        &FederalCapGainsBrackets::single_2024(),
        &state_tax_rate("CA").unwrap(),
    );
    DiversificationReportInput {
        ticker: "XYZ".into(),
        total_shares: 1000.0,
        cost_basis_per_share: 100.0,
        current_price: 200.0,
        acquired_date: acquired,
        other_annual_income: 150_000.0,
        state: "CA".into(),
        filing: "Single".into(),
        other_portfolio_value: 50_000.0,
        brokerage: Brokerage::Fidelity,
        risk_profile: "Moderate".into(),
        geo_inputs: GeoRiskInputs {
            oil_brent: 85.0,
            vix: 18.0,
            gold_3m_change: 0.03,
            dxy_3m_change: 0.01,
        },
        sell_plan,
        scenarios,
        start_year: 2026,
    }
}

/// Names `--ga-xxx` appearing inside `var(...)`.
fn referenced(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("var(--ga-") {
        let a = &rest[i + 4..];
        let end = a.find([')', ',']).unwrap();
        out.push(a[..end].trim().to_string());
        rest = &a[end..];
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn test_report_when_generated_should_define_every_ga_custom_property_it_references() {
    let html = generate_diversification_report(&sample_input());
    let used = referenced(&html);
    assert!(
        !used.is_empty(),
        "report is expected to use --ga tokens in its chart"
    );
    let missing: Vec<_> = used
        .iter()
        .filter(|name| {
            !html
                .match_indices(&format!("{name}:"))
                .chain(html.match_indices(&format!("{name} :")))
                .any(|(i, _)| {
                    // a declaration, not a var() reference
                    !html[..i].ends_with("var(")
                })
        })
        .collect();
    assert!(
        missing.is_empty(),
        "report references undefined custom properties: {missing:?}"
    );
}

#[test]
fn test_report_when_generated_should_define_tokens_inside_its_own_style_element() {
    let html = generate_diversification_report(&sample_input());
    let s = html.find("<style>").expect("style");
    let e = html.find("</style>").expect("/style");
    let style = &html[s..e];
    for name in referenced(&html) {
        assert!(
            style.contains(&format!("{name}:")),
            "{name} not defined in <style>"
        );
    }
}

#[test]
fn test_report_when_generated_should_stay_self_contained() {
    let html = generate_diversification_report(&sample_input());
    assert!(!html.contains("/static/"), "no served-asset references");
    assert!(!html.contains("<script"), "no JavaScript");
}
