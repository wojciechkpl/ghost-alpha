//! Point-in-time semantics of `CompanyFacts::value_as_of` and `CompanyFacts::series`.
//!
//! Required API:
//!   CompanyFacts::value_as_of(&self, taxonomy, concept, unit, period: Period, as_of: NaiveDate)
//!       -> Option<&Fact>
//!   enum Period { Instant(NaiveDate), Duration { start: NaiveDate, end: NaiveDate } }
//!       (Debug + Clone + Copy + PartialEq + Eq). Instant matches facts with no `start` and that
//!       `end`; Duration matches facts whose (start, end) equal exactly.
//!   CompanyFacts::series(&self, taxonomy, concept, unit, Periodicity, as_of: NaiveDate) -> Vec<&Fact>
//!       (a Vec<Fact> is also acceptable to these tests; they never clone/compare whole facts)
//!   enum Periodicity { Annual, Quarterly }  (Debug + Clone + Copy + PartialEq)
//!
//! Rules pinned here:
//!  * only facts with filed <= as_of (inclusive) matching the period are candidates
//!  * winner = latest `filed`; on equal `filed` an AMENDMENT (form ending "/A") beats the original;
//!    then higher accn; then greater `start` (None lowest). See edgar_pit_duration.rs for details.
//!  * series Annual   = facts with form "10-K" or "10-K/A" and fp == "FY"
//!    series Quarterly = facts with form "10-Q" or "10-Q/A" and fp in Q1..Q3
//!    one fact per distinct `end` (winner by the rule above), ascending by `end`;
//!    facts after as_of are invisible

use chrono::{Datelike, NaiveDate};
use ghost_alpha::data::edgar::*;

fn facts() -> CompanyFacts {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/edgar/companyfacts_exmp.json");
    parse_company_facts(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

/// Calendar-year duration ending on `period_end` (starts on 1 January of that year).
fn cy(period_end: &str) -> Period {
    let end = d(period_end);
    let start = NaiveDate::from_ymd_opt(end.year(), 1, 1).unwrap();
    Period::Duration { start, end }
}

fn rev_at(period_end: &str, as_of: &str) -> Option<(f64, String)> {
    facts()
        .value_as_of("us-gaap", "Revenues", "USD", cy(period_end), d(as_of))
        .map(|f| (f.val, f.form.clone()))
}

#[test]
fn test_value_as_of_when_as_of_before_first_filing_should_be_none() {
    assert_eq!(rev_at("2022-12-31", "2023-02-14"), None);
}

#[test]
fn test_value_as_of_when_as_of_equals_filed_date_should_include_the_fact() {
    assert_eq!(
        rev_at("2022-12-31", "2023-02-15"),
        Some((1000.0, "10-K".into()))
    );
}

#[test]
fn test_value_as_of_when_restatement_not_yet_filed_should_return_original_value() {
    assert_eq!(
        rev_at("2022-12-31", "2024-02-19"),
        Some((1000.0, "10-K".into()))
    );
}

#[test]
fn test_value_as_of_when_restatement_filed_should_return_restated_value() {
    assert_eq!(
        rev_at("2022-12-31", "2024-02-20"),
        Some((1050.0, "10-K".into()))
    );
    assert_eq!(
        rev_at("2022-12-31", "2026-10-25"),
        Some((1050.0, "10-K".into()))
    );
}

#[test]
fn test_value_as_of_when_period_not_yet_reported_should_be_none() {
    assert_eq!(rev_at("2023-12-31", "2024-02-19"), None);
    assert_eq!(rev_at("2024-12-31", "2026-10-25"), None);
}

#[test]
fn test_value_as_of_when_amendment_filed_later_should_pick_latest_filing_on_or_before_as_of() {
    assert_eq!(
        rev_at("2023-12-31", "2024-04-09"),
        Some((1300.0, "10-K".into()))
    );
    assert_eq!(
        rev_at("2023-12-31", "2024-04-10"),
        Some((1310.0, "10-K/A".into()))
    );
}

#[test]
fn test_value_as_of_when_original_and_amendment_filed_same_day_should_prefer_amendment() {
    let f = facts()
        .value_as_of(
            "us-gaap",
            "NetIncomeLoss",
            "USD",
            cy("2023-12-31"),
            d("2024-03-01"),
        )
        .map(|f| (f.val, f.form.clone()));
    // product decision: a same-day /A is usually a correction, so it wins
    assert_eq!(f, Some((155.0, "10-K/A".into())));
}

#[test]
fn test_value_as_of_when_instant_fact_should_match_on_end_date() {
    let c = facts();
    let a = c
        .value_as_of(
            "us-gaap",
            "Assets",
            "USD",
            Period::Instant(d("2023-12-31")),
            d("2024-03-01"),
        )
        .unwrap();
    assert_eq!(a.val, 5000.0);
    assert_eq!(a.start, None);
}

#[test]
fn test_value_as_of_when_unknown_concept_taxonomy_or_unit_should_be_none() {
    let c = facts();
    let (p, a) = (cy("2023-12-31"), d("2026-10-25"));
    assert!(c.value_as_of("us-gaap", "Nope", "USD", p, a).is_none());
    assert!(c.value_as_of("dei", "Revenues", "USD", p, a).is_none());
    assert!(c
        .value_as_of("us-gaap", "Revenues", "shares", p, a)
        .is_none());
}

#[test]
fn test_value_as_of_when_period_end_does_not_match_any_fact_should_be_none() {
    assert_eq!(rev_at("2023-12-30", "2026-10-25"), None);
}

// ── series ───────────────────────────────────────────────────────────────────

fn series(p: Periodicity, as_of: &str) -> Vec<(String, f64)> {
    facts()
        .series("us-gaap", "Revenues", "USD", p, d(as_of))
        .iter()
        .map(|f| (f.end.to_string(), f.val))
        .collect()
}

fn s(v: &[(&str, f64)]) -> Vec<(String, f64)> {
    v.iter().map(|(a, b)| (a.to_string(), *b)).collect()
}

#[test]
fn test_series_when_annual_and_latest_as_of_should_dedup_by_period_with_restated_values() {
    assert_eq!(
        series(Periodicity::Annual, "2024-03-01"),
        s(&[
            ("2021-12-31", 800.0),
            ("2022-12-31", 1050.0),
            ("2023-12-31", 1300.0)
        ])
    );
}

#[test]
fn test_series_when_annual_and_as_of_before_restatement_should_show_values_as_originally_reported()
{
    assert_eq!(
        series(Periodicity::Annual, "2023-12-31"),
        s(&[("2021-12-31", 800.0), ("2022-12-31", 1000.0)])
    );
}

#[test]
fn test_series_when_annual_and_amendment_filed_should_use_amended_value() {
    assert_eq!(
        series(Periodicity::Annual, "2026-10-25"),
        s(&[
            ("2021-12-31", 800.0),
            ("2022-12-31", 1050.0),
            ("2023-12-31", 1310.0)
        ])
    );
}

#[test]
fn test_series_when_as_of_before_any_filing_should_be_empty() {
    assert!(series(Periodicity::Annual, "2022-01-01").is_empty());
    assert!(series(Periodicity::Quarterly, "2023-01-01").is_empty());
}

#[test]
fn test_series_when_annual_should_exclude_quarterly_facts() {
    let ends: Vec<String> = series(Periodicity::Annual, "2026-10-25")
        .into_iter()
        .map(|x| x.0)
        .collect();
    assert!(!ends.contains(&"2023-03-31".to_string()));
}

#[test]
fn test_series_when_quarterly_should_return_only_10q_periods_ascending_with_restated_comparative() {
    assert_eq!(
        series(Periodicity::Quarterly, "2024-06-01"),
        s(&[
            ("2023-03-31", 285.0), // restated by the Q1 2024 10-Q
            ("2023-06-30", 300.0),
            ("2023-09-30", 320.0),
            ("2024-03-31", 330.0),
        ])
    );
}

#[test]
fn test_series_when_quarterly_and_as_of_before_comparative_should_use_original_quarter_value() {
    assert_eq!(
        series(Periodicity::Quarterly, "2023-12-31"),
        s(&[
            ("2023-03-31", 280.0),
            ("2023-06-30", 300.0),
            ("2023-09-30", 320.0)
        ])
    );
}

#[test]
fn test_series_when_concept_unknown_should_be_empty() {
    let c = facts();
    let v = c.series(
        "us-gaap",
        "Nope",
        "USD",
        Periodicity::Annual,
        d("2026-10-25"),
    );
    assert!(v.is_empty());
}

#[test]
fn test_series_when_instant_concept_annual_should_work_without_start_dates() {
    let c = facts();
    let v = c.series(
        "us-gaap",
        "Assets",
        "USD",
        Periodicity::Annual,
        d("2026-10-25"),
    );
    let got: Vec<(String, f64)> = v.iter().map(|f| (f.end.to_string(), f.val)).collect();
    assert_eq!(got, s(&[("2022-12-31", 4200.0), ("2023-12-31", 5000.0)]));
}
