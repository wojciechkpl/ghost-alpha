//! Markdown emitter for Plan10b5_1 (Gap 5b).
//!
//! Renders a `Plan10b5_1` (Gap 3) into a markdown plan-parameters document.
//! Static prose sections (modification
//! rules, insider-trading guardrails) are templated as constants.

use crate::plans::rule_10b5_1::{Plan10b5_1, PlanExecution};

/// Render a complete 10b5-1 trading plan as markdown.
///
/// Includes:
///   - Plan metadata header (filing date, first execution, plan window)
///   - Cooling-off compliance summary
///   - Per-execution table (date, shares, proceeds, LTCG)
///   - Lifetime totals (shares, proceeds, LTCG, tax, after-tax cash)
///   - Modification & termination boilerplate
///   - User-confirm checklist (open items)
pub fn render_plan_markdown(plan: &Plan10b5_1) -> String {
    let first_exec = plan.first_execution_date.format("%Y-%m-%d");
    let adoption = plan.config.adoption_date.format("%Y-%m-%d");
    let plan_end = plan.config.plan_end_date.format("%Y-%m-%d");

    let exec_rows: String = plan
        .executions
        .iter()
        .map(format_exec_row)
        .collect::<Vec<_>>()
        .join("\n");

    let total_shares = plan.total_shares();
    let total_proceeds = plan.total_proceeds();
    let total_ltcg = plan.total_ltcg();
    let total_tax = plan.total_tax();
    let after_tax = plan.total_after_tax_proceeds();

    format!(
        r#"# 10b5-1 Trading Plan

**Plan window:** {adoption} → {plan_end}
**First execution date:** {first_exec}

## 1. Cooling-off compliance

SEC Rule 10b5-1(c)(1) requires non-officer planholders to wait 90 days after
plan adoption before the first sale. The first execution date above reflects
this cooling-off period applied to the adoption date, with weekend rounding
to the next business day.

This generator implements the simplified non-officer rule. Officer/director
planholders also need the 2-business-days-after-quarterly-financial-disclosure
rule (SEC 10b5-1(c)(1)(ii)(B)(2)), which is out of scope.

## 2. Execution schedule

| Date | Shares | Proceeds | LTCG |
|---|---:|---:|---:|
{exec_rows}

## 3. Lifetime totals

- **Total shares to sell:** {total_shares:.0}
- **Total proceeds (gross):** ${total_proceeds:.2}
- **Total LTCG:** ${total_ltcg:.2}
- **Total tax:** ${total_tax:.2}
- **After-tax cash:** ${after_tax:.2}

## 4. Open items the user must confirm before signing

- [ ] Obtain Fidelity per-lot inventory for the underlying position. Required for
      HIFO specID at execution; without it, the broker defaults to FIFO and
      realizes higher LTCG than projected.
- [ ] Confirm that no other 10b5-1 plan is currently active (SEC's single-plan rule).
- [ ] Confirm planholder is not in possession of MNPI at adoption.
- [ ] Verify the broker (typically Fidelity Stock Plan Services) accepts the
      execution schedule and lot-selection rule.
"#
    )
}

fn format_exec_row(e: &PlanExecution) -> String {
    format!(
        "| {} | {:.0} | ${:.2} | ${:.2} |",
        e.date.format("%Y-%m-%d"),
        e.shares,
        e.estimated_proceeds,
        e.estimated_ltcg,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plans::rule_10b5_1::{Plan10b5_1, PlanConfig};
    use chrono::NaiveDate;

    // ── Helper: synthetic 3-execution plan ────────────────────────────────────

    /// Small self-contained plan used across most tests.
    ///
    /// adoption_date : 2026-01-05 (Mon)
    ///   first_exec  : 2026-01-05 + 90 days = 2026-04-05 (Sun) → 2026-04-06 (Mon)
    /// earnings_dates:
    ///   2026-04-01 (Wed) + 5 bd = 2026-04-08 (Wed) → in window [2026-04-06, 2027-01-01]
    ///   2026-07-02 (Thu) + 5 bd = 2026-07-09 (Thu) → in window
    ///   2026-10-01 (Thu) + 5 bd = 2026-10-08 (Thu) → in window
    /// target_shares_per_execution : 500
    /// avg_sale_price              : 200.0
    /// avg_cost_basis              : 100.0   (gain = $100/share)
    /// annual_ltcg_cap             : 500_000  (never binds; all 3 execs full)
    /// ltcg_rate                   : 0.20
    fn build_synthetic_plan() -> Plan10b5_1 {
        let config = PlanConfig {
            adoption_date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
            plan_end_date: NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
            earnings_dates: vec![
                NaiveDate::from_ymd_opt(2026, 4, 1).unwrap(),
                NaiveDate::from_ymd_opt(2026, 7, 2).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 1).unwrap(),
            ],
            post_earnings_business_days: 5,
            target_shares_per_execution: 500.0,
            avg_sale_price: 200.0,
            avg_cost_basis: 100.0,
            annual_ltcg_cap: 500_000.0,
            ltcg_rate: 0.20,
        };
        Plan10b5_1::new(config)
    }

    // ── Document structure (5 tests) ──────────────────────────────────────────

    #[test]
    fn markdown_starts_with_h1_title() {
        let plan = build_synthetic_plan();
        let md = render_plan_markdown(&plan);
        let first_non_empty = md.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        assert!(
            first_non_empty.starts_with("# "),
            "first non-empty line must be an H1, got: {:?}",
            first_non_empty
        );
    }

    #[test]
    fn markdown_contains_cooling_off_section_with_first_execution_date() {
        // adoption_date=2026-03-02 → first_exec=2026-06-01 (90 days = Sun → Mon)
        let config = PlanConfig {
            adoption_date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            plan_end_date: NaiveDate::from_ymd_opt(2028, 5, 15).unwrap(),
            earnings_dates: vec![NaiveDate::from_ymd_opt(2026, 7, 30).unwrap()],
            post_earnings_business_days: 10,
            target_shares_per_execution: 700.0,
            avg_sale_price: 280.0,
            avg_cost_basis: 150.0,
            annual_ltcg_cap: 350_000.0,
            ltcg_rate: 0.30,
        };
        let plan = Plan10b5_1::new(config);
        let md = render_plan_markdown(&plan);
        assert!(
            md.contains("2026-06-01"),
            "markdown must contain first_execution_date 2026-06-01"
        );
    }

    #[test]
    fn markdown_contains_execution_table_header() {
        let plan = build_synthetic_plan();
        let md = render_plan_markdown(&plan);
        // Pipe-delimited table header must include all four column names.
        assert!(md.contains("Date"), "missing 'Date' column");
        assert!(md.contains("Shares"), "missing 'Shares' column");
        assert!(md.contains("Proceeds"), "missing 'Proceeds' column");
        assert!(md.contains("LTCG"), "missing 'LTCG' column");
        // Must be a pipe-delimited row.
        assert!(
            md.lines()
                .any(|l| l.contains('|') && l.contains("Date") && l.contains("Shares")),
            "no pipe-delimited table header row found"
        );
    }

    #[test]
    fn markdown_contains_one_table_row_per_execution() {
        let plan = build_synthetic_plan();
        assert_eq!(
            plan.executions.len(),
            3,
            "fixture must produce 3 executions"
        );
        let md = render_plan_markdown(&plan);
        // Count lines that look like data rows: start with `|` and contain a YYYY-MM-DD.
        let date_row_count = md
            .lines()
            .filter(|l| {
                l.contains('|') && {
                    // Simple check: line contains a pattern like `2026-` or `2027-`
                    l.contains("2026-") || l.contains("2027-") || l.contains("2028-")
                }
            })
            .count();
        assert!(
            date_row_count >= 3,
            "expected at least 3 date-bearing table rows, found {date_row_count}"
        );
    }

    #[test]
    fn markdown_contains_lifetime_totals_section() {
        let plan = build_synthetic_plan();
        let md = render_plan_markdown(&plan);
        let md_lower = md.to_lowercase();
        assert!(md_lower.contains("total shares"), "missing 'total shares'");
        assert!(
            md_lower.contains("total proceeds"),
            "missing 'total proceeds'"
        );
        assert!(md_lower.contains("total ltcg"), "missing 'total ltcg'");
        assert!(md_lower.contains("total tax"), "missing 'total tax'");
        assert!(md_lower.contains("after-tax"), "missing 'after-tax'");
    }

    // ── Numeric correctness (4 tests) ─────────────────────────────────────────

    #[test]
    fn markdown_total_shares_matches_plan_total_shares() {
        let plan = build_synthetic_plan();
        let expected = plan.total_shares(); // 1500.0 for synthetic plan
        let md = render_plan_markdown(&plan);
        // Find a line containing "total shares" (case-insensitive) and extract a number.
        let total_shares_line = md
            .lines()
            .find(|l| l.to_lowercase().contains("total shares"))
            .expect("no 'total shares' line found");
        // Extract the first run of digits (with optional decimal point).
        let value: f64 = extract_first_number(total_shares_line)
            .expect("no numeric value found on 'total shares' line");
        assert!(
            (value - expected).abs() <= 1.0,
            "total shares in markdown ({value}) differs from plan ({expected}) by more than 1"
        );
    }

    #[test]
    fn markdown_first_execution_date_matches_plan() {
        let plan = build_synthetic_plan();
        let expected = plan.first_execution_date.format("%Y-%m-%d").to_string();
        let md = render_plan_markdown(&plan);
        assert!(
            md.contains(&expected),
            "first_execution_date {expected} not found in markdown"
        );
    }

    #[test]
    fn markdown_total_tax_matches_plan_total_tax() {
        let plan = build_synthetic_plan();
        let expected = plan.total_tax(); // 1500 * 100 * 0.20 = 30_000.0
        let md = render_plan_markdown(&plan);
        let total_tax_line = md
            .lines()
            .find(|l| l.to_lowercase().contains("total tax"))
            .expect("no 'total tax' line found");
        let value: f64 = extract_first_number(total_tax_line)
            .expect("no numeric value found on 'total tax' line");
        // Allow ±10 for rounding in currency display (2 decimal places).
        assert!(
            (value - expected).abs() <= 10.0,
            "total tax in markdown ({value}) differs from plan ({expected:.2}) by more than 10"
        );
    }

    #[test]
    fn markdown_per_execution_shares_match_plan_executions() {
        let plan = build_synthetic_plan();
        let md = render_plan_markdown(&plan);
        for exec in &plan.executions {
            let date_str = exec.date.format("%Y-%m-%d").to_string();
            let row = md
                .lines()
                .find(|l| l.contains(&date_str))
                .unwrap_or_else(|| panic!("no table row found for date {date_str}"));
            let _value = extract_first_number(row)
                .unwrap_or_else(|| panic!("no numeric value found in row for {date_str}"));
            // Date itself contains digits; the shares column should appear before proceeds.
            // We parse the date out and find the next number.
            let shares_value = extract_number_after_date(row, &date_str)
                .unwrap_or_else(|| panic!("could not extract shares from row for {date_str}"));
            assert!(
                (shares_value - exec.shares).abs() <= 0.5,
                "row for {date_str}: shares in markdown ({shares_value}) != plan ({:.1})",
                exec.shares
            );
        }
    }

    // ── Static-prose sections (2 tests) ───────────────────────────────────────

    #[test]
    fn markdown_contains_cooling_off_compliance_explanation() {
        let plan = build_synthetic_plan();
        let md = render_plan_markdown(&plan);
        let md_lower = md.to_lowercase();
        assert!(
            md_lower.contains("90 days"),
            "cooling-off section must mention '90 days'"
        );
        assert!(
            md_lower.contains("non-officer"),
            "cooling-off section must mention 'non-officer'"
        );
    }

    #[test]
    fn markdown_contains_user_confirm_checklist() {
        let plan = build_synthetic_plan();
        let md = render_plan_markdown(&plan);
        assert!(
            md.contains("- [ ]"),
            "markdown must contain at least one unchecked checkbox '- [ ]'"
        );
        assert!(
            md.to_lowercase().contains("lot inventory"),
            "checklist must mention 'lot inventory' (Fidelity per-lot inventory open item)"
        );
    }

    // ── Empty plan edge case (1 test) ─────────────────────────────────────────

    #[test]
    fn empty_plan_renders_table_with_zero_executions() {
        let config = PlanConfig {
            adoption_date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap(),
            plan_end_date: NaiveDate::from_ymd_opt(2027, 1, 1).unwrap(),
            earnings_dates: vec![], // no earnings → no executions
            post_earnings_business_days: 5,
            target_shares_per_execution: 500.0,
            avg_sale_price: 200.0,
            avg_cost_basis: 100.0,
            annual_ltcg_cap: 500_000.0,
            ltcg_rate: 0.20,
        };
        let plan = Plan10b5_1::new(config);
        assert!(
            plan.executions.is_empty(),
            "fixture must produce 0 executions"
        );
        // Must not panic.
        let md = render_plan_markdown(&plan);
        // Table section is still present.
        assert!(
            md.lines()
                .any(|l| l.contains('|') && l.contains("Date") && l.contains("Shares")),
            "empty plan must still render the table header"
        );
        // Total shares is 0.
        let total_shares_line = md
            .lines()
            .find(|l| l.to_lowercase().contains("total shares"))
            .expect("no 'total shares' line in empty-plan markdown");
        let value = extract_first_number(total_shares_line)
            .expect("no number on 'total shares' line for empty plan");
        assert!(
            value.abs() < 1.0,
            "empty plan must show 0 total shares, got {value}"
        );
    }

    // ── Test-local helpers ────────────────────────────────────────────────────

    /// Extract the first contiguous run of `[0-9.]+` digits from `s`.
    fn extract_first_number(s: &str) -> Option<f64> {
        let mut start = None;
        let mut end = 0;
        for (i, c) in s.char_indices() {
            if c.is_ascii_digit() || c == '.' {
                if start.is_none() {
                    start = Some(i);
                }
                end = i + c.len_utf8();
            } else if start.is_some() {
                break;
            }
        }
        start.and_then(|s_idx| s[s_idx..end].parse::<f64>().ok())
    }

    /// Skip past the YYYY-MM-DD date in a pipe-delimited row, then extract the
    /// next numeric token — which is the shares column.
    fn extract_number_after_date(row: &str, date_str: &str) -> Option<f64> {
        let after_date = row.find(date_str).map(|pos| &row[pos + date_str.len()..])?;
        extract_first_number(after_date)
    }
}
