//! SEC Rule 10b5-1(c)(1) trading-plan parameter generator.
//!
//! Scope: non-officer/non-director planholder. The 2-business-days-after
//! quarterly-filing branch (SEC 10b5-1(c)(1)(ii)(B)(2)) applies only to officers
//! and directors and is intentionally omitted (see docs/PLAN_PIPELINE.md).
//! Adding officer compliance would be a separate follow-up.
//!
//! No federal-holiday calendar — `is_business_day` returns true for any Mon-Fri.

use chrono::{Datelike, Days, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

/// Configuration parameters for building a 10b5-1 trading plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanConfig {
    pub adoption_date: NaiveDate,
    pub plan_end_date: NaiveDate,
    /// Quarterly earnings releases for the underlying ticker of the concentrated
    /// position. Each → one execution at +N business days post-earnings.
    pub earnings_dates: Vec<NaiveDate>,
    pub post_earnings_business_days: u32,
    pub target_shares_per_execution: f64,
    pub avg_sale_price: f64,
    pub avg_cost_basis: f64,
    pub annual_ltcg_cap: f64,
    pub ltcg_rate: f64,
}

/// A complete 10b5-1 trading plan: cooling-off-validated first execution date,
/// scheduled executions, projected proceeds + tax.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan10b5_1 {
    pub config: PlanConfig,
    pub first_execution_date: NaiveDate,
    pub executions: Vec<PlanExecution>,
}

impl Plan10b5_1 {
    /// Build a plan from config. Wires together:
    ///   - `first_execution_date_non_officer(config.adoption_date)`
    ///   - `execution_dates(first_execution_date, plan_end_date, earnings_dates, post_earnings_business_days)`
    ///   - `size_executions(dates, target_shares_per_execution, avg_sale_price, avg_cost_basis, annual_ltcg_cap)`
    pub fn new(config: PlanConfig) -> Self {
        let first_execution_date = first_execution_date_non_officer(config.adoption_date);
        let dates = execution_dates(
            first_execution_date,
            config.plan_end_date,
            &config.earnings_dates,
            config.post_earnings_business_days,
        );
        let executions = size_executions(
            &dates,
            config.target_shares_per_execution,
            config.avg_sale_price,
            config.avg_cost_basis,
            config.annual_ltcg_cap,
        );
        Self {
            config,
            first_execution_date,
            executions,
        }
    }

    /// Sum of `shares` across all executions.
    pub fn total_shares(&self) -> f64 {
        self.executions.iter().map(|e| e.shares).sum()
    }

    /// Sum of `estimated_proceeds`.
    pub fn total_proceeds(&self) -> f64 {
        self.executions.iter().map(|e| e.estimated_proceeds).sum()
    }

    /// Sum of `estimated_ltcg`.
    pub fn total_ltcg(&self) -> f64 {
        self.executions.iter().map(|e| e.estimated_ltcg).sum()
    }

    /// `total_ltcg() * config.ltcg_rate`.
    pub fn total_tax(&self) -> f64 {
        self.total_ltcg() * self.config.ltcg_rate
    }

    /// `total_proceeds() - total_tax()`.
    pub fn total_after_tax_proceeds(&self) -> f64 {
        self.total_proceeds() - self.total_tax()
    }
}

/// True iff `date` falls on Mon-Fri. Federal holidays are NOT excluded.
pub fn is_business_day(date: NaiveDate) -> bool {
    !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
}

/// If `date` is a Sat/Sun, advance to the following Monday. Otherwise return as-is.
pub fn next_business_day(date: NaiveDate) -> NaiveDate {
    let mut d = date;
    while !is_business_day(d) {
        d = d.checked_add_days(Days::new(1)).expect("date arithmetic");
    }
    d
}

/// Advance `date` by `n` business days. Weekends are skipped (no holiday calendar).
///
/// `n=0` returns `date` unchanged if it is already a business day, otherwise
/// advances to the next business day. `n=1` from Mon = Tue. `n=1` from Fri = Mon.
/// `n=5` from Mon = next Mon.
pub fn advance_business_days(date: NaiveDate, n: u32) -> NaiveDate {
    let mut d = next_business_day(date);
    let mut remaining = n;
    while remaining > 0 {
        d = next_business_day(d.checked_add_days(Days::new(1)).expect("date arithmetic"));
        remaining -= 1;
    }
    d
}

/// Generate execution dates for a 10b5-1 plan window.
///
/// For each earnings release in `earnings_dates`:
///   - compute `advance_business_days(earnings, post_earnings_business_days)`
///   - keep only the result if it falls within `[plan_start, plan_end]` (inclusive)
///
/// Returned dates are sorted ascending and de-duplicated. The function does NOT
/// validate that earnings dates are themselves business days.
pub fn execution_dates(
    plan_start: NaiveDate,
    plan_end: NaiveDate,
    earnings_dates: &[NaiveDate],
    post_earnings_business_days: u32,
) -> Vec<NaiveDate> {
    let mut out: Vec<NaiveDate> = earnings_dates
        .iter()
        .map(|e| advance_business_days(*e, post_earnings_business_days))
        .filter(|d| *d >= plan_start && *d <= plan_end)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// SEC Rule 10b5-1(c)(1) cooling-off: 90 calendar days after adoption,
/// rounded forward to the next business day.
///
/// For non-officer planholders only. Officer/director planholders also need
/// the 2-business-days-after-10Q rule (cut from this scope).
pub fn first_execution_date_non_officer(adoption_date: NaiveDate) -> NaiveDate {
    let plus_90 = adoption_date
        .checked_add_days(Days::new(90))
        .expect("date arithmetic");
    next_business_day(plus_90)
}

/// One scheduled execution of the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanExecution {
    pub date: NaiveDate,
    /// Whole-share quantity (floored at the call site for 10b5-1 compliance).
    pub shares: f64,
    pub estimated_proceeds: f64,
    pub estimated_ltcg: f64,
}

/// Size each execution date with `target_shares_per_execution`, but throttle so that
/// calendar-year-summed `estimated_ltcg` never exceeds `annual_ltcg_cap`.
///
/// Within a year, executions are processed in date order; if adding the next execution
/// would exceed the cap, that execution is shrunk to fit (down to 0 shares if necessary).
/// All subsequent executions in the SAME calendar year are sized 0 (no later-execution
/// "rescue" — the cap is hit and we stop for that year). The next calendar year's
/// budget resets.
///
/// # Preconditions
///
/// All numeric inputs must be ≥ 0 and finite. Input validation is the caller's
/// responsibility; this function does not guard against negative values or NaN.
///
/// `gain_per_share = avg_sale_price - avg_cost_basis` (must be ≥ 0). When
/// `gain_per_share <= 0` there is no LTCG to realise so the cap is never the binding
/// constraint and every execution gets `target_shares_per_execution`.
///
/// The `dates` slice is assumed pre-sorted ascending (matches `execution_dates` output
/// contract). This function does NOT re-sort.
pub fn size_executions(
    dates: &[NaiveDate],
    target_shares_per_execution: f64,
    avg_sale_price: f64,
    avg_cost_basis: f64,
    annual_ltcg_cap: f64,
) -> Vec<PlanExecution> {
    let gain_per_share = avg_sale_price - avg_cost_basis;
    let mut out = Vec::with_capacity(dates.len());

    let mut current_year: Option<i32> = None;
    let mut cumulative_ltcg_this_year = 0.0_f64;
    let mut cap_hit_this_year = false;

    for &date in dates {
        let year = date.year();

        // Reset budget at calendar-year boundary.
        if Some(year) != current_year {
            current_year = Some(year);
            cumulative_ltcg_this_year = 0.0;
            cap_hit_this_year = false;
        }

        let shares = if cap_hit_this_year {
            0.0
        } else if gain_per_share <= 0.0 {
            // Zero-gain edge case: cap is never binding.
            target_shares_per_execution.floor()
        } else {
            let remaining_budget = annual_ltcg_cap - cumulative_ltcg_this_year;
            let max_shares_for_cap = (remaining_budget / gain_per_share).floor();
            target_shares_per_execution
                .min(max_shares_for_cap)
                .max(0.0)
                .floor()
        };

        let estimated_proceeds = shares * avg_sale_price;
        let estimated_ltcg = shares * gain_per_share.max(0.0);

        cumulative_ltcg_this_year += estimated_ltcg;
        // Mark cap hit if this execution was shrunk OR the cumulative total
        // has now reached or exceeded the cap (covers exact-cap landing).
        if shares < target_shares_per_execution || cumulative_ltcg_this_year >= annual_ltcg_cap {
            cap_hit_this_year = true;
        }

        out.push(PlanExecution {
            date,
            shares,
            estimated_proceeds,
            estimated_ltcg,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    // --- is_business_day ---

    #[test]
    fn monday_through_friday_are_business_days() {
        // 2026-05-04 Mon through 2026-05-08 Fri
        for day in 4..=8 {
            let date = NaiveDate::from_ymd_opt(2026, 5, day).unwrap();
            assert!(is_business_day(date), "{date} should be a business day");
        }
    }

    #[test]
    fn saturday_is_not_business_day() {
        // 2026-05-09 is Saturday
        let date = NaiveDate::from_ymd_opt(2026, 5, 9).unwrap();
        assert!(!is_business_day(date));
    }

    #[test]
    fn sunday_is_not_business_day() {
        // 2026-05-10 is Sunday
        let date = NaiveDate::from_ymd_opt(2026, 5, 10).unwrap();
        assert!(!is_business_day(date));
    }

    // --- next_business_day ---

    #[test]
    fn weekday_returns_self() {
        // 2026-05-04 is Monday
        let date = NaiveDate::from_ymd_opt(2026, 5, 4).unwrap();
        assert_eq!(next_business_day(date), date);
    }

    #[test]
    fn friday_returns_self() {
        // 2026-05-08 is Friday
        let date = NaiveDate::from_ymd_opt(2026, 5, 8).unwrap();
        assert_eq!(next_business_day(date), date);
    }

    #[test]
    fn saturday_advances_to_monday() {
        // 2026-05-09 Sat -> 2026-05-11 Mon
        let sat = NaiveDate::from_ymd_opt(2026, 5, 9).unwrap();
        let mon = NaiveDate::from_ymd_opt(2026, 5, 11).unwrap();
        assert_eq!(next_business_day(sat), mon);
    }

    #[test]
    fn sunday_advances_to_monday() {
        // 2026-05-10 Sun -> 2026-05-11 Mon
        let sun = NaiveDate::from_ymd_opt(2026, 5, 10).unwrap();
        let mon = NaiveDate::from_ymd_opt(2026, 5, 11).unwrap();
        assert_eq!(next_business_day(sun), mon);
    }

    // --- first_execution_date_non_officer ---

    #[test]
    fn adoption_2026_03_02_first_exec_rolls_sunday_to_monday_2026_06_01() {
        // 2026-03-02 (Mon) + 90 days = 2026-05-31 (Sun) -> rolls to 2026-06-01 (Mon)
        let adoption = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
        let expected = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        assert_eq!(first_execution_date_non_officer(adoption), expected);
    }

    #[test]
    fn adoption_lands_first_exec_on_weekend_advances_to_monday() {
        // 2026-01-04 (Sun) + 90 days = 2026-04-04 (Sat) -> advances to 2026-04-06 (Mon)
        let adoption = NaiveDate::from_ymd_opt(2026, 1, 4).unwrap();
        let expected = NaiveDate::from_ymd_opt(2026, 4, 6).unwrap();
        assert_eq!(first_execution_date_non_officer(adoption), expected);
    }

    #[test]
    fn adoption_first_exec_landing_on_friday_returns_friday() {
        // 2026-01-03 (Sat) + 90 days = 2026-04-03 (Fri) -> stays 2026-04-03
        let adoption = NaiveDate::from_ymd_opt(2026, 1, 3).unwrap();
        let expected = NaiveDate::from_ymd_opt(2026, 4, 3).unwrap();
        assert_eq!(first_execution_date_non_officer(adoption), expected);
    }

    #[test]
    fn cooling_off_is_exactly_90_calendar_days_pre_business_day_round() {
        // For any adoption date, the raw +90 offset before rounding must be
        // exactly 90 days; after rounding forward, the gap is 90, 91, or 92.
        let adoption = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let result = first_execution_date_non_officer(adoption);
        let days = (result - adoption).num_days();
        assert!(
            (90..=92).contains(&days),
            "expected 90-92 days gap, got {days}"
        );
    }

    #[test]
    fn example_filing_2026_03_02_first_exec_is_91_days_later() {
        // 90-day cooling-off lands on a Sunday, so the first execution is the
        // following Monday: 2026-06-01, i.e. 91 calendar days after adoption.
        let adoption = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
        let result = first_execution_date_non_officer(adoption);
        assert_eq!(result, NaiveDate::from_ymd_opt(2026, 6, 1).unwrap());
        assert_eq!((result - adoption).num_days(), 91);
    }

    // --- advance_business_days ---

    #[test]
    fn advance_zero_returns_same_business_day() {
        // 2026-05-04 is Monday; n=0 on a business day returns the same date.
        let mon = NaiveDate::from_ymd_opt(2026, 5, 4).unwrap();
        assert_eq!(advance_business_days(mon, 0), mon);
    }

    #[test]
    fn advance_zero_from_saturday_returns_monday() {
        // 2026-05-09 (Sat) with n=0 still rounds forward to the next business day.
        let sat = NaiveDate::from_ymd_opt(2026, 5, 9).unwrap();
        let mon = NaiveDate::from_ymd_opt(2026, 5, 11).unwrap();
        assert_eq!(advance_business_days(sat, 0), mon);
    }

    #[test]
    fn advance_one_from_monday_returns_tuesday() {
        // 2026-05-04 (Mon) + 1 biz day = 2026-05-05 (Tue)
        let mon = NaiveDate::from_ymd_opt(2026, 5, 4).unwrap();
        let tue = NaiveDate::from_ymd_opt(2026, 5, 5).unwrap();
        assert_eq!(advance_business_days(mon, 1), tue);
    }

    #[test]
    fn advance_one_from_friday_returns_monday() {
        // 2026-05-08 (Fri) + 1 biz day skips Sat/Sun -> 2026-05-11 (Mon)
        let fri = NaiveDate::from_ymd_opt(2026, 5, 8).unwrap();
        let mon = NaiveDate::from_ymd_opt(2026, 5, 11).unwrap();
        assert_eq!(advance_business_days(fri, 1), mon);
    }

    #[test]
    fn advance_five_from_monday_returns_next_monday() {
        // 2026-05-04 (Mon) + 5 biz days = 2026-05-11 (Mon); one full trading week.
        let mon = NaiveDate::from_ymd_opt(2026, 5, 4).unwrap();
        let next_mon = NaiveDate::from_ymd_opt(2026, 5, 11).unwrap();
        assert_eq!(advance_business_days(mon, 5), next_mon);
    }

    // --- execution_dates ---

    #[test]
    fn execution_dates_empty_earnings_returns_empty() {
        let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2027, 12, 31).unwrap();
        assert!(execution_dates(start, end, &[], 5).is_empty());
    }

    #[test]
    fn execution_dates_outside_window_excluded() {
        // advance_business_days(2026-04-30, 5) = 2026-05-07 -> before plan_start -> excluded
        // advance_business_days(2027-01-30 (Sat), 5) = 2027-02-08 (Mon) -> in window -> kept
        // advance_business_days(2099-01-01, 5) = 2099-01-08 -> after plan_end -> excluded
        let earnings = [
            NaiveDate::from_ymd_opt(2026, 4, 30).unwrap(),
            NaiveDate::from_ymd_opt(2027, 1, 30).unwrap(),
            NaiveDate::from_ymd_opt(2099, 1, 1).unwrap(),
        ];
        let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2027, 12, 31).unwrap();
        let result = execution_dates(start, end, &earnings, 5);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], NaiveDate::from_ymd_opt(2027, 2, 8).unwrap());
    }

    #[test]
    fn execution_dates_post_earnings_business_day_rounding() {
        // 2026-07-30 (Thu) + 5 biz days: Thu->Fri->Mon->Tue->Wed->Thu = 2026-08-06 (Thu)
        let earnings = [NaiveDate::from_ymd_opt(2026, 7, 30).unwrap()];
        let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2099, 12, 31).unwrap();
        let result = execution_dates(start, end, &earnings, 5);
        assert_eq!(result, vec![NaiveDate::from_ymd_opt(2026, 8, 6).unwrap()]);
    }

    #[test]
    fn execution_dates_quarterly_schedule_synthetic() {
        // Synthetic quarterly earnings cadence.
        // post_earnings_business_days=10 matches ~2-week broker scheduling lag.
        let earnings = [
            NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 29).unwrap(),
            NaiveDate::from_ymd_opt(2027, 1, 28).unwrap(),
            NaiveDate::from_ymd_opt(2027, 4, 29).unwrap(),
            NaiveDate::from_ymd_opt(2027, 7, 29).unwrap(),
            NaiveDate::from_ymd_opt(2027, 10, 28).unwrap(),
            NaiveDate::from_ymd_opt(2028, 1, 27).unwrap(),
        ];
        let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2028, 3, 1).unwrap();
        let result = execution_dates(start, end, &earnings, 10);
        assert_eq!(
            result.len(),
            7,
            "expected one execution date per earnings quarter"
        );
    }

    // --- sort + dedup invariants ---

    #[test]
    fn execution_dates_returns_sorted_ascending() {
        // Earnings provided in random order; result must be sorted ascending.
        let earnings = [
            NaiveDate::from_ymd_opt(2027, 4, 29).unwrap(),
            NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 29).unwrap(),
        ];
        let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2099, 12, 31).unwrap();
        let result = execution_dates(start, end, &earnings, 5);
        assert_eq!(result.len(), 3);
        assert!(
            result.windows(2).all(|w| w[0] < w[1]),
            "result is not sorted: {result:?}"
        );
    }

    #[test]
    fn execution_dates_deduplicates_when_same_earnings_date_repeated() {
        // Same earnings date provided twice; result must contain exactly one entry.
        let dup = NaiveDate::from_ymd_opt(2026, 8, 14).unwrap();
        let earnings = [dup, dup];
        let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        let end = NaiveDate::from_ymd_opt(2099, 12, 31).unwrap();
        let result = execution_dates(start, end, &earnings, 5);
        assert_eq!(
            result.len(),
            1,
            "duplicate earnings date produced duplicate execution: {result:?}"
        );
    }

    // --- size_executions: basic sizing — no cap binding ---

    #[test]
    fn size_executions_returns_target_shares_when_cap_never_binding() {
        // 4 quarterly dates in 2027; gain=$100/share; 4×700×$100 = $280k < $350k cap.
        let dates = [
            NaiveDate::from_ymd_opt(2027, 2, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 5, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 8, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 11, 15).unwrap(),
        ];
        let result = size_executions(&dates, 700.0, 250.0, 150.0, 350_000.0);
        assert_eq!(result.len(), 4);
        for exec in &result {
            assert_eq!(exec.shares, 700.0);
            assert_eq!(exec.estimated_ltcg, 70_000.0);
            assert_eq!(exec.estimated_proceeds, 700.0 * 250.0);
        }
    }

    #[test]
    fn size_executions_respects_zero_executions() {
        // Empty input → empty output.
        let result = size_executions(&[], 700.0, 250.0, 150.0, 350_000.0);
        assert!(result.is_empty());
    }

    #[test]
    fn size_executions_with_zero_gain_per_share_never_caps() {
        // avg_sale_price == avg_cost_basis → gain_per_share = 0; cap never binds.
        let dates = [
            NaiveDate::from_ymd_opt(2027, 2, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 5, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 8, 15).unwrap(),
        ];
        let result = size_executions(&dates, 700.0, 200.0, 200.0, 100.0);
        assert_eq!(result.len(), 3);
        for exec in &result {
            assert_eq!(exec.shares, 700.0);
            assert_eq!(exec.estimated_ltcg, 0.0);
        }
    }

    // --- size_executions: cap binding mid-year ---

    #[test]
    fn size_executions_shrinks_last_execution_when_cap_would_be_exceeded() {
        // 4 dates in 2027; target=1000; gain=$100; 3×$100k=$300k; 4th fits $50k → 500 shares.
        let dates = [
            NaiveDate::from_ymd_opt(2027, 2, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 5, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 8, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 11, 15).unwrap(),
        ];
        let result = size_executions(&dates, 1000.0, 200.0, 100.0, 350_000.0);
        assert_eq!(result[0].shares, 1000.0);
        assert_eq!(result[1].shares, 1000.0);
        assert_eq!(result[2].shares, 1000.0);
        assert_eq!(result[3].shares, 500.0);
    }

    #[test]
    fn size_executions_zeros_remaining_after_cap_hit_in_year() {
        // cap=$200k; exec1=$100k; exec2=$100k → cumulative=$200k (exactly hits cap);
        // exec3 and exec4 are 0.
        let dates = [
            NaiveDate::from_ymd_opt(2027, 2, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 5, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 8, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 11, 15).unwrap(),
        ];
        let result = size_executions(&dates, 1000.0, 200.0, 100.0, 200_000.0);
        assert_eq!(result[0].shares, 1000.0);
        assert_eq!(result[1].shares, 1000.0); // lands exactly at cap — full allocation
        assert_eq!(result[2].shares, 0.0);
        assert_eq!(result[3].shares, 0.0);
    }

    #[test]
    fn size_executions_partial_shrink_then_zero_in_same_year() {
        // cap=$150k; exec1=1000($100k); exec2 shrunk to 500($50k→cap hit); exec3,4 = 0.
        let dates = [
            NaiveDate::from_ymd_opt(2027, 2, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 5, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 8, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 11, 15).unwrap(),
        ];
        let result = size_executions(&dates, 1000.0, 200.0, 100.0, 150_000.0);
        assert_eq!(result[0].shares, 1000.0);
        assert_eq!(result[1].shares, 500.0);
        assert_eq!(result[2].shares, 0.0);
        assert_eq!(result[3].shares, 0.0);
    }

    // --- size_executions: year boundary resets cap ---

    #[test]
    fn size_executions_year_boundary_resets_cap_budget() {
        // 2 dates in 2026 + 2 in 2027; each year cap=$150k, gain=$100/share.
        // 2026: exec1=1000($100k), exec2=500($50k→cap hit).
        // 2027: budget resets, exec3=1000($100k), exec4=500($50k→cap hit).
        let dates = [
            NaiveDate::from_ymd_opt(2026, 7, 15).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 2, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 5, 15).unwrap(),
        ];
        let result = size_executions(&dates, 1000.0, 200.0, 100.0, 150_000.0);
        assert_eq!(result[0].shares, 1000.0); // 2026 exec 1
        assert_eq!(result[1].shares, 500.0); // 2026 exec 2, shrunk
        assert_eq!(result[2].shares, 1000.0); // 2027 exec 1, budget reset
        assert_eq!(result[3].shares, 500.0); // 2027 exec 2, shrunk
    }

    #[test]
    fn size_executions_full_calendar_independence_per_year() {
        // 3 calendar years (2025, 2026, 2027), 2 executions each, cap=$150k, gain=$100.
        // Each year: exec_a=1000($100k), exec_b=500($50k→cap hit). Years are independent.
        let dates = [
            NaiveDate::from_ymd_opt(2025, 4, 15).unwrap(),
            NaiveDate::from_ymd_opt(2025, 10, 15).unwrap(),
            NaiveDate::from_ymd_opt(2026, 4, 15).unwrap(),
            NaiveDate::from_ymd_opt(2026, 10, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 4, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 10, 15).unwrap(),
        ];
        let result = size_executions(&dates, 1000.0, 200.0, 100.0, 150_000.0);
        assert_eq!(result.len(), 6);
        for chunk in result.chunks(2) {
            assert_eq!(
                chunk[0].shares, 1000.0,
                "first exec of each year should be 1000"
            );
            assert_eq!(
                chunk[1].shares, 500.0,
                "second exec of each year should be shrunk to 500"
            );
        }
    }

    // --- size_executions: PlanExecution field correctness ---

    #[test]
    fn plan_execution_estimated_proceeds_is_shares_times_price() {
        // For any sized execution, estimated_proceeds == shares × avg_sale_price exactly.
        let dates = [
            NaiveDate::from_ymd_opt(2027, 2, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 5, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 8, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 11, 15).unwrap(),
        ];
        let avg_sale_price = 237.50_f64;
        let result = size_executions(&dates, 1000.0, avg_sale_price, 100.0, 150_000.0);
        for exec in &result {
            assert_eq!(exec.estimated_proceeds, exec.shares * avg_sale_price);
        }
    }

    #[test]
    fn plan_execution_estimated_ltcg_is_shares_times_gain_per_share() {
        // For any sized execution where gain > 0, estimated_ltcg == shares × gain_per_share.
        let dates = [
            NaiveDate::from_ymd_opt(2027, 2, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 5, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 8, 15).unwrap(),
            NaiveDate::from_ymd_opt(2027, 11, 15).unwrap(),
        ];
        let avg_sale_price = 250.0_f64;
        let avg_cost_basis = 100.0_f64;
        let gain_per_share = avg_sale_price - avg_cost_basis;
        let result = size_executions(&dates, 1000.0, avg_sale_price, avg_cost_basis, 150_000.0);
        for exec in &result {
            assert_eq!(exec.estimated_ltcg, exec.shares * gain_per_share);
        }
    }

    // --- Plan10b5_1 constructor wiring ---

    fn example_option_b_config() -> PlanConfig {
        PlanConfig {
            adoption_date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            plan_end_date: NaiveDate::from_ymd_opt(2028, 5, 15).unwrap(),
            earnings_dates: vec![
                NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
                NaiveDate::from_ymd_opt(2026, 10, 29).unwrap(),
                NaiveDate::from_ymd_opt(2027, 1, 28).unwrap(),
                NaiveDate::from_ymd_opt(2027, 4, 29).unwrap(),
                NaiveDate::from_ymd_opt(2027, 7, 29).unwrap(),
                NaiveDate::from_ymd_opt(2027, 10, 28).unwrap(),
                NaiveDate::from_ymd_opt(2028, 1, 27).unwrap(),
            ],
            post_earnings_business_days: 10,
            target_shares_per_execution: 700.0,
            avg_sale_price: 280.00,
            avg_cost_basis: 150.0,
            annual_ltcg_cap: 350_000.0,
            ltcg_rate: 0.30,
        }
    }

    #[test]
    fn plan_new_first_execution_uses_cooling_off_math() {
        // adoption_date=2026-03-02; first_execution_date_non_officer returns 2026-06-01.
        let config = PlanConfig {
            adoption_date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            plan_end_date: NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            earnings_dates: vec![],
            post_earnings_business_days: 10,
            target_shares_per_execution: 700.0,
            avg_sale_price: 280.0,
            avg_cost_basis: 150.0,
            annual_ltcg_cap: 350_000.0,
            ltcg_rate: 0.30,
        };
        let plan = Plan10b5_1::new(config.clone());
        assert_eq!(
            plan.first_execution_date,
            first_execution_date_non_officer(config.adoption_date)
        );
        assert_eq!(
            plan.first_execution_date,
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        );
    }

    #[test]
    fn plan_new_executions_filter_by_window() {
        // adoption=2026-03-02 (first_exec=2026-06-01); plan_end=2027-12-31.
        // pre-window: 2026-04-30 +10bd = 2026-05-14 < 2026-06-01 → excluded
        // in-window:  2026-07-30 +10bd = 2026-08-13 >= first_exec → included
        // in-window:  2027-01-28 +10bd = 2027-02-11 <= plan_end  → included
        // post-window: 2099-01-01 +10bd > plan_end                → excluded
        let config = PlanConfig {
            adoption_date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            plan_end_date: NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            earnings_dates: vec![
                NaiveDate::from_ymd_opt(2026, 4, 30).unwrap(),
                NaiveDate::from_ymd_opt(2026, 7, 30).unwrap(),
                NaiveDate::from_ymd_opt(2027, 1, 28).unwrap(),
                NaiveDate::from_ymd_opt(2099, 1, 1).unwrap(),
            ],
            post_earnings_business_days: 10,
            target_shares_per_execution: 700.0,
            avg_sale_price: 280.0,
            avg_cost_basis: 150.0,
            annual_ltcg_cap: 350_000.0,
            ltcg_rate: 0.30,
        };
        let plan = Plan10b5_1::new(config);
        assert_eq!(plan.executions.len(), 2);
    }

    #[test]
    fn plan_new_executions_sized_by_3c_logic() {
        // 4 quarterly dates in 2027; target=1000; gain=$100/sh; cap=$250k.
        // exec1: $100k cumul; exec2: $200k cumul; exec3: 500 sh → $250k cap hit; exec4: 0.
        let config = PlanConfig {
            adoption_date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            plan_end_date: NaiveDate::from_ymd_opt(2028, 12, 31).unwrap(),
            earnings_dates: vec![
                NaiveDate::from_ymd_opt(2027, 1, 4).unwrap(),
                NaiveDate::from_ymd_opt(2027, 4, 1).unwrap(),
                NaiveDate::from_ymd_opt(2027, 7, 1).unwrap(),
                NaiveDate::from_ymd_opt(2027, 10, 1).unwrap(),
            ],
            post_earnings_business_days: 0,
            target_shares_per_execution: 1000.0,
            avg_sale_price: 200.0,
            avg_cost_basis: 100.0,
            annual_ltcg_cap: 250_000.0,
            ltcg_rate: 0.30,
        };
        let plan = Plan10b5_1::new(config);
        assert_eq!(plan.executions.len(), 4);
        assert_eq!(plan.executions[0].shares, 1000.0);
        assert_eq!(plan.executions[1].shares, 1000.0);
        assert_eq!(plan.executions[2].shares, 500.0);
        assert_eq!(plan.executions[3].shares, 0.0);
    }

    #[test]
    fn plan_new_executions_dates_match_3b_logic() {
        // earnings=2026-05-18 (Mon) +10bd = 2026-06-01; adoption=2026-03-02
        // first_exec_date=2026-06-01, so this earnings execution is exactly on the boundary.
        let config = PlanConfig {
            adoption_date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            plan_end_date: NaiveDate::from_ymd_opt(2027, 12, 31).unwrap(),
            earnings_dates: vec![NaiveDate::from_ymd_opt(2026, 5, 18).unwrap()],
            post_earnings_business_days: 10,
            target_shares_per_execution: 700.0,
            avg_sale_price: 280.0,
            avg_cost_basis: 150.0,
            annual_ltcg_cap: 350_000.0,
            ltcg_rate: 0.30,
        };
        let plan = Plan10b5_1::new(config);
        assert_eq!(plan.executions.len(), 1);
        let expected_date =
            advance_business_days(NaiveDate::from_ymd_opt(2026, 5, 18).unwrap(), 10);
        assert_eq!(expected_date, NaiveDate::from_ymd_opt(2026, 6, 1).unwrap());
        assert_eq!(plan.executions[0].date, expected_date);
    }

    // --- Plan10b5_1 summary helpers ---

    fn make_synthetic_plan() -> Plan10b5_1 {
        // Manually construct a plan with known executions to isolate summary helpers.
        // shares: [1000, 1000, 500, 0]; avg_sale_price=$200; gain=$100/sh; ltcg_rate=0.30
        let config = PlanConfig {
            adoption_date: NaiveDate::from_ymd_opt(2026, 3, 2).unwrap(),
            plan_end_date: NaiveDate::from_ymd_opt(2028, 12, 31).unwrap(),
            earnings_dates: vec![
                NaiveDate::from_ymd_opt(2027, 1, 4).unwrap(),
                NaiveDate::from_ymd_opt(2027, 4, 1).unwrap(),
                NaiveDate::from_ymd_opt(2027, 7, 1).unwrap(),
                NaiveDate::from_ymd_opt(2027, 10, 1).unwrap(),
            ],
            post_earnings_business_days: 0,
            target_shares_per_execution: 1000.0,
            avg_sale_price: 200.0,
            avg_cost_basis: 100.0,
            annual_ltcg_cap: 250_000.0,
            ltcg_rate: 0.30,
        };
        Plan10b5_1::new(config)
    }

    #[test]
    fn total_shares_sums_all_executions() {
        // executions: [1000, 1000, 500, 0] → 2500
        let plan = make_synthetic_plan();
        assert_eq!(plan.total_shares(), 2500.0);
    }

    #[test]
    fn total_proceeds_sums_estimated_proceeds() {
        // 2500 shares × $200 avg_sale_price = $500_000
        let plan = make_synthetic_plan();
        assert_eq!(plan.total_proceeds(), 500_000.0);
    }

    #[test]
    fn total_ltcg_sums_estimated_ltcg() {
        // 2500 shares × $100 gain = $250_000
        let plan = make_synthetic_plan();
        assert_eq!(plan.total_ltcg(), 250_000.0);
    }

    #[test]
    fn total_tax_applies_ltcg_rate() {
        // $250_000 × 0.30 = $75_000
        let plan = make_synthetic_plan();
        let expected = 250_000.0_f64 * 0.30;
        assert!((plan.total_tax() - expected).abs() < 1e-6);
    }

    #[test]
    fn total_after_tax_proceeds_subtracts_tax_from_proceeds() {
        // $500_000 - $75_000 = $425_000
        let plan = make_synthetic_plan();
        let expected = plan.total_proceeds() - plan.total_tax();
        assert!((plan.total_after_tax_proceeds() - expected).abs() < 1e-6);
        assert!((plan.total_after_tax_proceeds() - 425_000.0).abs() < 1.0);
    }

    // --- Example-recipe smoke tests ---

    #[test]
    fn example_option_b_first_execution_2026_06_01() {
        let plan = Plan10b5_1::new(example_option_b_config());
        assert_eq!(
            plan.first_execution_date,
            NaiveDate::from_ymd_opt(2026, 6, 1).unwrap()
        );
        assert_eq!(plan.executions.len(), 7);
    }

    #[test]
    fn example_option_b_total_metrics_in_documented_range() {
        let plan = Plan10b5_1::new(example_option_b_config());
        // 7 × 700 = 4900 shares; allow slack for cap truncation in outer years.
        assert!(
            (4_500.0..=5_500.0).contains(&plan.total_shares()),
            "total_shares out of range: {}",
            plan.total_shares()
        );
        // gain_per_share = $280 - $150.0 = $131.11; spread over 3 calendar years with
        // $350k/yr cap → expected total_ltcg in [$600k, $720k].
        assert!(
            (600_000.0..=720_000.0).contains(&plan.total_ltcg()),
            "total_ltcg out of range: {}",
            plan.total_ltcg()
        );
        // total_tax ≈ 30% of total_ltcg; allow [180k, 230k].
        assert!(
            (180_000.0..=230_000.0).contains(&plan.total_tax()),
            "total_tax out of range: {}",
            plan.total_tax()
        );
    }
}
