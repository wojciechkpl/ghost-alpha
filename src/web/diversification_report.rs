//! Self-contained HTML diversification report generator.
//!
//! Produces a single `.html` file combining position analysis, tax scenario
//! comparison, selling programme, fund allocation, geo risk assessment, and
//! concentration decay chart — no external dependencies, no JavaScript required.

use chrono::NaiveDate;

use crate::analysis::geopolitical::{compute_geo_risk, geo_allocation_adjustments, GeoRiskInputs};
use crate::portfolio::allocation::{blended_expense_ratio, map_to_funds, risk_profiles, Brokerage};
use crate::portfolio::prepare::SellPlanOutput;
use crate::portfolio::selling::SellingProgramme;
use crate::tax::projection::ScenarioResult;
use crate::web::charts::{concentration_decay_points, concentration_decay_svg};

/// All inputs needed to generate a diversification report.
pub struct DiversificationReportInput {
    pub ticker: String,
    pub total_shares: f64,
    pub cost_basis_per_share: f64,
    pub current_price: f64,
    pub acquired_date: NaiveDate,
    pub other_annual_income: f64,
    pub state: String,
    pub filing: String,
    pub other_portfolio_value: f64,
    pub brokerage: Brokerage,
    pub risk_profile: String,
    pub geo_inputs: GeoRiskInputs,
    pub sell_plan: SellPlanOutput,
    pub scenarios: Vec<ScenarioResult>,
    pub start_year: u32,
}

/// Generate a self-contained HTML diversification report.
pub fn generate_diversification_report(input: &DiversificationReportInput) -> String {
    let generated_at = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();

    let position_section = render_position_section(input);
    let scenario_section = render_scenario_section(&input.scenarios, &input.state);
    let sell_plan_section = render_sell_plan_section(&input.sell_plan);
    let allocation_section = render_allocation_section(
        &input.brokerage,
        &input.risk_profile,
        input.other_portfolio_value + input.total_shares * input.current_price,
    );
    let geo_section = render_geo_section(&input.geo_inputs);
    let decay_section = render_decay_section(input);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Ghost Alpha — {ticker} Diversification Plan</title>
<style>
{css}
</style>
</head>
<body>
<header>
  <h1>Concentrated Position Diversification Plan</h1>
  <p class="meta">{ticker} &nbsp;|&nbsp; {shares:.0} shares @ ${price:.2} &nbsp;|&nbsp; State: {state} ({filing}) &nbsp;|&nbsp; Generated: {generated_at}</p>
</header>
<main>
{position_section}
{scenario_section}
{sell_plan_section}
{allocation_section}
{geo_section}
{decay_section}
</main>
<footer>
  <p>Ghost Alpha Diversification Engine &mdash; generated {generated_at}</p>
  <p style="margin-top:4px">This is not financial advice. Consult a CPA and RIA before executing any selling programme.</p>
</footer>
</body>
</html>"#,
        ticker = input.ticker,
        shares = input.total_shares,
        price = input.current_price,
        state = input.state,
        filing = input.filing,
        css = REPORT_CSS,
    )
}

// ─── Section Renderers ──────────────────────────────────────────────────────

fn render_position_section(input: &DiversificationReportInput) -> String {
    let market_value = input.total_shares * input.current_price;
    let cost_basis = input.total_shares * input.cost_basis_per_share;
    let unrealised_gain = market_value - cost_basis;
    let gain_pct = if cost_basis > 0.0 {
        unrealised_gain / cost_basis * 100.0
    } else {
        0.0
    };
    let total_portfolio = market_value + input.other_portfolio_value;
    let concentration = if total_portfolio > 0.0 {
        market_value / total_portfolio * 100.0
    } else {
        0.0
    };
    let is_lt = (chrono::Utc::now().naive_utc().date() - input.acquired_date).num_days() > 365;
    let term = if is_lt { "Long-term" } else { "Short-term" };

    format!(
        r#"<section>
<h2>1. Position Summary</h2>
<div class="kpi-grid">
  <div class="kpi kpi-neutral"><div class="kpi-label">Shares</div><div class="kpi-value">{shares:.0}</div></div>
  <div class="kpi kpi-neutral"><div class="kpi-label">Price</div><div class="kpi-value">${price:.2}</div></div>
  <div class="kpi kpi-neutral"><div class="kpi-label">Market Value</div><div class="kpi-value">${mv}</div></div>
  <div class="kpi kpi-neutral"><div class="kpi-label">Cost Basis</div><div class="kpi-value">${cb}</div></div>
  <div class="kpi {gain_class}"><div class="kpi-label">Unrealised Gain</div><div class="kpi-value">${gain} ({gain_pct:.1}%)</div></div>
  <div class="kpi {conc_class}"><div class="kpi-label">Concentration</div><div class="kpi-value">{concentration:.0}%</div></div>
  <div class="kpi kpi-neutral"><div class="kpi-label">Holding</div><div class="kpi-value">{term}</div></div>
  <div class="kpi kpi-neutral"><div class="kpi-label">Other Income</div><div class="kpi-value">${income}</div></div>
</div>
</section>"#,
        shares = input.total_shares,
        price = input.current_price,
        mv = fmt_k(market_value),
        cb = fmt_k(cost_basis),
        gain = fmt_k(unrealised_gain),
        gain_pct = gain_pct,
        gain_class = if unrealised_gain > 0.0 {
            "kpi-good"
        } else {
            "kpi-bad"
        },
        concentration = concentration,
        conc_class = if concentration > 50.0 {
            "kpi-bad"
        } else if concentration > 25.0 {
            "kpi-warn"
        } else {
            "kpi-good"
        },
        term = term,
        income = fmt_k(input.other_annual_income),
    )
}

fn render_scenario_section(scenarios: &[ScenarioResult], state: &str) -> String {
    if scenarios.is_empty() {
        return "<section><h2>2. Tax Scenario Comparison</h2><p class=\"empty\">No scenarios.</p></section>".into();
    }

    let optimal = scenarios.iter().find(|s| s.is_optimal);
    let savings_msg = match optimal {
        Some(o) if o.savings_vs_lump > 0.0 => format!(
            "<p class=\"summary\"><strong>Recommendation:</strong> Spread over <strong>{} year{}</strong> to save <strong>${}</strong> vs lump-sum sale. \
             The savings plateau occurs when annual gains drop below the {} deduction threshold.</p>",
            o.spread_years,
            if o.spread_years > 1 { "s" } else { "" },
            fmt_k(o.savings_vs_lump),
            state,
        ),
        _ => "<p class=\"summary\">No significant tax savings from spreading — single-year execution is optimal.</p>".into(),
    };

    let rows: String = scenarios.iter().map(|s| {
        let marker = if s.is_optimal { " class=\"optimal\"" } else { "" };
        format!(
            "<tr{marker}><td>{}</td><td>{:.0}</td><td>${}</td><td>${}</td><td>${}</td><td>${}</td><td>{:.1}%</td><td>${}</td></tr>",
            s.spread_years, s.annual_shares,
            fmt_k(s.total_proceeds), fmt_k(s.total_gain),
            fmt_k(s.total_federal_tax), fmt_k(s.total_state_tax),
            s.effective_rate * 100.0, fmt_k(s.savings_vs_lump),
        )
    }).collect();

    format!(
        r#"<section>
<h2>2. Tax Scenario Comparison (State: {state})</h2>
{savings_msg}
<table>
<thead><tr><th>Years</th><th>Shares/yr</th><th>Proceeds</th><th>Gain</th><th>Federal</th><th>State</th><th>Eff %</th><th>Savings</th></tr></thead>
<tbody>{rows}</tbody>
</table>
</section>"#,
    )
}

fn render_sell_plan_section(plan: &SellPlanOutput) -> String {
    let year_rows: String = plan.year_projections.iter().map(|p| {
        let eff = if p.realised_gain > 0.0 { p.total_tax / p.realised_gain * 100.0 } else { 0.0 };
        format!(
            "<tr><td>{}</td><td>{:.0}</td><td>${}</td><td>${}</td><td>${}</td><td>{:.1}%</td></tr>",
            p.year, p.shares_sold, fmt_k(p.proceeds), fmt_k(p.realised_gain), fmt_k(p.total_tax), eff,
        )
    }).collect();

    let q_rows: String = plan
        .programme
        .schedule
        .iter()
        .map(|p| {
            format!(
                "<tr><td>Y{} Q{}</td><td>{:.0}</td></tr>",
                p.year, p.quarter, p.shares_target
            )
        })
        .collect();

    let s = &plan.summary;

    format!(
        r#"<section>
<h2>3. Selling Programme — {years}-Year Plan</h2>
<div class="kpi-grid" style="margin-bottom:1rem">
  <div class="kpi kpi-neutral"><div class="kpi-label">Total Proceeds</div><div class="kpi-value">${proceeds}</div></div>
  <div class="kpi kpi-neutral"><div class="kpi-label">Total Tax</div><div class="kpi-value">${tax}</div></div>
  <div class="kpi kpi-good"><div class="kpi-label">After-Tax</div><div class="kpi-value">${after}</div></div>
  <div class="kpi kpi-neutral"><div class="kpi-label">Effective Rate</div><div class="kpi-value">{eff:.1}%</div></div>
</div>
<h3>Year-by-Year Projection</h3>
<table>
<thead><tr><th>Year</th><th>Shares</th><th>Proceeds</th><th>Gain</th><th>Tax</th><th>Eff %</th></tr></thead>
<tbody>{year_rows}</tbody>
</table>
<h3>Quarterly Schedule</h3>
<table style="max-width:300px">
<thead><tr><th>Period</th><th>Shares</th></tr></thead>
<tbody>{q_rows}</tbody>
</table>
</section>"#,
        years = plan.horizon_years,
        proceeds = fmt_k(s.total_proceeds),
        tax = fmt_k(s.total_tax),
        after = fmt_k(s.total_after_tax),
        eff = s.blended_effective_rate * 100.0,
    )
}

fn render_allocation_section(
    brokerage: &Brokerage,
    profile_name: &str,
    portfolio_value: f64,
) -> String {
    let profiles = risk_profiles();
    let profile = profiles
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(profile_name))
        .unwrap_or(&profiles[1]);

    let allocations = map_to_funds(&profile.allocations, brokerage, portfolio_value);
    let blended_er = blended_expense_ratio(&allocations);

    let rows: String = allocations.iter().map(|a| {
        let tlh = a.fund.tlh_partner.as_deref().unwrap_or("—");
        let transfer = if a.fund.transferable { "Yes" } else { "No" };
        format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{:.3}%</td><td>${}</td><td>{}</td><td>{}</td></tr>",
            a.asset_class, a.fund.ticker, a.fund.name,
            a.fund.expense_ratio * 100.0, fmt_k(a.dollar_amount), tlh, transfer,
        )
    }).collect();

    let brok_name = match brokerage {
        Brokerage::Fidelity => "Fidelity",
        Brokerage::Schwab => "Schwab",
        Brokerage::Vanguard => "Vanguard",
    };

    format!(
        r#"<section>
<h2>4. Target Allocation — {profile} on {brok}</h2>
<p class="summary">{desc} &nbsp;|&nbsp; Blended ER: <strong>{er:.4}%</strong></p>
<table>
<thead><tr><th>Asset Class</th><th>Ticker</th><th>Fund</th><th>ER</th><th>Amount</th><th>TLH Partner</th><th>Transferable</th></tr></thead>
<tbody>{rows}</tbody>
</table>
</section>"#,
        profile = profile.name,
        brok = brok_name,
        desc = profile.description,
        er = blended_er * 100.0,
    )
}

fn render_geo_section(inputs: &GeoRiskInputs) -> String {
    let score = compute_geo_risk(inputs);
    let adjustments = geo_allocation_adjustments(&score.level);

    let level_color = match &score.level {
        crate::analysis::geopolitical::GeoRiskLevel::Low => "#2b8a3e",
        crate::analysis::geopolitical::GeoRiskLevel::Moderate => "#e67700",
        crate::analysis::geopolitical::GeoRiskLevel::High => "#d9480f",
        crate::analysis::geopolitical::GeoRiskLevel::Extreme => "#c92a2a",
    };

    let adj_rows: String = if adjustments.is_empty() {
        "<tr><td colspan=\"2\">No adjustments — risk is low.</td></tr>".into()
    } else {
        adjustments
            .iter()
            .map(|(sector, delta)| {
                let sign = if *delta > 0.0 { "+" } else { "" };
                let cls = if *delta > 0.0 { "win" } else { "loss" };
                format!(
                    "<tr><td>{}</td><td class=\"{cls}\">{}{:.0}%</td></tr>",
                    sector,
                    sign,
                    delta * 100.0
                )
            })
            .collect()
    };

    format!(
        r#"<section>
<h2>5. Geopolitical Risk Assessment</h2>
<div class="kpi-grid">
  <div class="kpi kpi-neutral"><div class="kpi-label">Oil (Brent)</div><div class="kpi-value">${oil:.0}</div><div class="kpi-sub">{oil_c:.2}/2.5</div></div>
  <div class="kpi kpi-neutral"><div class="kpi-label">VIX</div><div class="kpi-value">{vix:.1}</div><div class="kpi-sub">{vix_c:.2}/2.5</div></div>
  <div class="kpi kpi-neutral"><div class="kpi-label">Gold 3m</div><div class="kpi-value">{gold:.1}%</div><div class="kpi-sub">{gold_c:.2}/2.5</div></div>
  <div class="kpi kpi-neutral"><div class="kpi-label">DXY 3m</div><div class="kpi-value">{dxy:.1}%</div><div class="kpi-sub">{dxy_c:.2}/2.5</div></div>
</div>
<p style="margin-top:1rem"><strong>Composite:</strong> <span style="font-size:1.3em;color:{level_color}">{composite:.1}/10 — {level}</span></p>
<h3>Suggested Allocation Adjustments</h3>
<table style="max-width:400px">
<thead><tr><th>Sector</th><th>Adjustment</th></tr></thead>
<tbody>{adj_rows}</tbody>
</table>
</section>"#,
        oil = inputs.oil_brent,
        vix = inputs.vix,
        gold = inputs.gold_3m_change * 100.0,
        dxy = inputs.dxy_3m_change * 100.0,
        oil_c = score.oil_component,
        vix_c = score.vix_component,
        gold_c = score.gold_component,
        dxy_c = score.dxy_component,
        composite = score.composite,
        level = score.level,
    )
}

fn render_decay_section(input: &DiversificationReportInput) -> String {
    let per_year = input.total_shares / input.sell_plan.horizon_years as f64;
    let annual: Vec<f64> = vec![per_year; input.sell_plan.horizon_years as usize];
    let programme =
        SellingProgramme::build_schedule(&input.ticker, &annual, input.sell_plan.horizon_years);

    let points = concentration_decay_points(
        0.0, // computed inside
        input.total_shares,
        &programme,
        input.current_price,
        input.other_portfolio_value,
        input.start_year,
    );

    let chart = concentration_decay_svg(&points, 860, 320);

    let first_vol = points
        .first()
        .map(|p| p.estimated_annual_vol)
        .unwrap_or(0.0);
    let last_vol = points.last().map(|p| p.estimated_annual_vol).unwrap_or(0.0);
    let vol_reduction = if first_vol > 0.0 {
        (first_vol - last_vol) / first_vol * 100.0
    } else {
        0.0
    };

    format!(
        r#"<section>
<h2>6. Concentration Decay — Projected Volatility Reduction</h2>
<p class="summary">As you sell down {ticker}, your portfolio's estimated annual volatility drops from <strong>{first:.1}%</strong> to <strong>{last:.1}%</strong> — a <strong>{red:.0}%</strong> reduction in risk.</p>
<div class="chart-container">{chart}</div>
</section>"#,
        ticker = input.ticker,
        first = first_vol,
        last = last_vol,
        red = vol_reduction,
    )
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn fmt_k(v: f64) -> String {
    let abs = v.abs();
    let sign = if v < 0.0 { "-" } else { "" };
    if abs >= 1_000_000.0 {
        format!("{sign}{:.1}M", abs / 1_000_000.0)
    } else if abs >= 10_000.0 {
        format!("{sign}{:.0}K", abs / 1_000.0)
    } else if abs >= 1_000.0 {
        format!("{sign}{:.1}K", abs / 1_000.0)
    } else {
        format!("{sign}{:.0}", abs)
    }
}

// ─── CSS ────────────────────────────────────────────────────────────────────

const REPORT_CSS: &str = r#"
*, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
html { font-size: 15px; }
body { font-family: system-ui, -apple-system, sans-serif; background: #f8f9fa; color: #212529; max-width: 960px; margin: 0 auto; padding: 1.5rem; }
header { border-bottom: 2px solid #343a40; padding-bottom: 0.75rem; margin-bottom: 2rem; }
header h1 { font-size: 1.6rem; color: #1c7ed6; }
.meta { font-size: 0.8rem; color: #868e96; margin-top: 0.25rem; }
section { background: #fff; border-radius: 8px; padding: 1.25rem 1.5rem; margin-bottom: 1.5rem; box-shadow: 0 1px 4px rgba(0,0,0,0.08); }
section h2 { font-size: 1.1rem; color: #343a40; margin-bottom: 1rem; padding-bottom: 0.4rem; border-bottom: 1px solid #e9ecef; }
section h3 { font-size: 0.95rem; color: #495057; margin: 1rem 0 0.5rem; }
.kpi-grid { display: flex; flex-wrap: wrap; gap: 0.75rem; }
.kpi { flex: 0 0 auto; min-width: 130px; border-radius: 6px; padding: 0.75rem 1rem; text-align: center; }
.kpi-label { font-size: 0.72rem; color: #495057; text-transform: uppercase; letter-spacing: 0.04em; margin-bottom: 0.3rem; }
.kpi-value { font-size: 1.25rem; font-weight: 700; }
.kpi-sub { font-size: 0.7rem; color: #868e96; margin-top: 0.2rem; }
.kpi-good { background: #d3f9d8; }
.kpi-warn { background: #fff3bf; }
.kpi-bad { background: #ffe3e3; }
.kpi-neutral { background: #e9ecef; }
table { border-collapse: collapse; width: 100%; margin-top: 0.5rem; font-size: 0.85rem; }
th, td { border: 1px solid #dee2e6; padding: 0.45rem 0.7rem; }
th { background: #f1f3f5; text-align: center; font-weight: 600; color: #495057; }
td { text-align: right; }
td:first-child { text-align: left; }
tr:nth-child(even) td { background: #f8f9fa; }
tr.optimal td { background: #d3f9d8 !important; font-weight: 600; }
.win { color: #2b8a3e; font-weight: 600; }
.loss { color: #c92a2a; font-weight: 600; }
.summary { font-size: 0.85rem; color: #495057; margin-bottom: 0.75rem; }
.empty { font-size: 0.85rem; color: #868e96; font-style: italic; }
.chart-container { margin-top: 1rem; overflow-x: auto; }
/* Chart tokens (the report is a standalone file, so it carries its own values). */
:root { --ga-series-1: #0969da; --ga-text: #1f2328; --ga-muted: #57606a; }
svg text { fill: var(--ga-text); }
svg text.chart-muted { fill: var(--ga-muted); }
footer { text-align: center; font-size: 0.75rem; color: #adb5bd; margin-top: 2rem; padding-top: 1rem; border-top: 1px solid #dee2e6; }
@media (prefers-color-scheme: dark) {
  :root { --ga-series-1: #58a6ff; --ga-text: #c1c2c5; --ga-muted: #9ea7b3; }
  body { background: #1a1b1e; color: #c1c2c5; }
  header { border-bottom-color: #495057; }
  header h1 { color: #74c0fc; }
  section { background: #25262b; box-shadow: 0 1px 4px rgba(0,0,0,0.4); }
  section h2 { color: #e9ecef; border-bottom-color: #373a40; }
  th { background: #2c2e33; color: #ced4da; }
  td { color: #c1c2c5; }
  tr:nth-child(even) td { background: #2c2e33; }
  tr.optimal td { background: #1b4332 !important; }
  .kpi-good { background: #1b4332; }
  .kpi-warn { background: #664d03; }
  .kpi-bad { background: #5c2020; }
  .kpi-neutral { background: #343a40; }
  .kpi-label { color: #adb5bd; }
  footer { border-top-color: #495057; }
}
@media print { body { max-width: 100%; } section { break-inside: avoid; } }
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::prepare::{prepare_selling_programme, SellDistribution, SellPlanInput};
    use crate::tax::brackets::{state_tax_rate, FederalCapGainsBrackets, FilingStatus};
    use crate::tax::projection::compare_scenarios;

    fn sample_input() -> DiversificationReportInput {
        let plan_input = SellPlanInput {
            ticker: "XYZ".into(),
            total_shares: 1000.0,
            cost_basis_per_share: 100.0,
            current_price: 200.0,
            acquired_date: NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(),
            filing_status: FilingStatus::Single,
            state_code: "CA".into(),
            other_annual_income: 150_000.0,
            assumed_price_growth: 0.0,
            horizon_years: 2,
            start_year: 2026,
            distribution: SellDistribution::Even,
        };

        let sell_plan = prepare_selling_programme(&plan_input).unwrap();

        let b = FederalCapGainsBrackets::single_2024();
        let wa = state_tax_rate("CA").unwrap();
        let lot = crate::portfolio::types::TaxLot {
            id: uuid::Uuid::new_v4(),
            ticker: "XYZ".into(),
            shares: 1000.0,
            cost_basis_per_share: 100.0,
            acquired_date: NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(),
            acquisition_type: crate::portfolio::types::AcquisitionType::RsuVest,
            remaining_shares: 1000.0,
        };
        let scenarios = compare_scenarios(&[lot], 1000.0, 5, 2026, 0.0, 200.0, 150_000.0, &b, &wa);

        DiversificationReportInput {
            ticker: "XYZ".into(),
            total_shares: 1000.0,
            cost_basis_per_share: 100.0,
            current_price: 200.0,
            acquired_date: NaiveDate::from_ymd_opt(2022, 1, 1).unwrap(),
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

    #[test]
    fn test_report_generates_valid_html() {
        let html = generate_diversification_report(&sample_input());
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("</html>"));
        assert!(html.contains("XYZ"));
    }

    #[test]
    fn test_report_has_all_sections() {
        let html = generate_diversification_report(&sample_input());
        assert!(html.contains("Position Summary"));
        assert!(html.contains("Tax Scenario Comparison"));
        assert!(html.contains("Selling Programme"));
        assert!(html.contains("Target Allocation"));
        assert!(html.contains("Geopolitical Risk"));
        assert!(html.contains("Concentration Decay"));
    }

    #[test]
    fn test_report_has_scenario_table() {
        let html = generate_diversification_report(&sample_input());
        assert!(html.contains("<th>Years</th>"));
        assert!(html.contains("<th>Savings</th>"));
    }

    #[test]
    fn test_report_has_fund_table() {
        let html = generate_diversification_report(&sample_input());
        assert!(html.contains("FZROX") || html.contains("FSKAX"));
        assert!(html.contains("TLH Partner"));
    }

    #[test]
    fn test_report_has_svg_chart() {
        let html = generate_diversification_report(&sample_input());
        assert!(html.contains("<svg"));
        assert!(html.contains("Volatility Decay"));
    }

    #[test]
    fn test_report_has_disclaimer() {
        let html = generate_diversification_report(&sample_input());
        assert!(html.contains("not financial advice"));
    }

    #[test]
    fn test_report_has_dark_mode() {
        let html = generate_diversification_report(&sample_input());
        assert!(html.contains("prefers-color-scheme: dark"));
    }

    #[test]
    fn test_report_has_print_styles() {
        let html = generate_diversification_report(&sample_input());
        assert!(html.contains("@media print"));
    }

    #[test]
    fn test_fmt_k() {
        assert_eq!(fmt_k(500.0), "500");
        assert_eq!(fmt_k(5_000.0), "5.0K");
        assert_eq!(fmt_k(50_000.0), "50K");
        assert_eq!(fmt_k(1_500_000.0), "1.5M");
        assert_eq!(fmt_k(-25_000.0), "-25K");
    }
}
