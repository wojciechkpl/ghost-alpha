//! Duration-aware point-in-time semantics (review finding H1).
//!
//! Contract:
//!   enum Period { Instant(NaiveDate), Duration { start: NaiveDate, end: NaiveDate } }
//!   CompanyFacts::value_as_of(taxonomy, concept, unit, period: Period, as_of) -> Option<&Fact>
//!     Instant(e) matches facts with start == None and end == e; Duration matches (start, end) exactly.
//!   CompanyFacts::series(...) applies the form/fp filter AND a duration filter, then keeps one
//!   fact per `end`:
//!     Annual    : start is None OR 335..=395 days (end - start)
//!     Quarterly : start is None OR 61..=121 days
//!   Winner order shared by value_as_of and series (file order never matters):
//!     latest `filed`; then an amendment (form ends "/A") beats the original; then higher `accn`;
//!     then greater `start` (None is lowest).
//!
//! Fixture: tests/fixtures/edgar/companyfacts_realistic.json (fictional, CIK 999999).

use chrono::NaiveDate;
use ghost_alpha::data::edgar::*;
use serde_json::Value;

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn dur(start: &str, end: &str) -> Period {
    Period::Duration {
        start: d(start),
        end: d(end),
    }
}

fn raw() -> Value {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/edgar/companyfacts_realistic.json");
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn parse(v: &Value) -> CompanyFacts {
    parse_company_facts(&v.to_string()).unwrap()
}

fn facts() -> CompanyFacts {
    parse(&raw())
}

fn rows(c: &CompanyFacts, concept: &str, p: Periodicity, as_of: &str) -> Vec<(String, f64)> {
    c.series("us-gaap", concept, "USD", p, d(as_of))
        .iter()
        .map(|f| (f.end.to_string(), f.val))
        .collect()
}

fn s(v: &[(&str, f64)]) -> Vec<(String, f64)> {
    v.iter().map(|(a, b)| (a.to_string(), *b)).collect()
}

fn val(c: &CompanyFacts, concept: &str, p: Period, as_of: &str) -> Option<f64> {
    c.value_as_of("us-gaap", concept, "USD", p, d(as_of))
        .map(|f| f.val)
}

/// Deterministic permutation of every unit array in the fixture.
fn permuted(seed: u64) -> Value {
    let mut v = raw();
    let mut state = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
    for (_, concept) in v["facts"]["us-gaap"].as_object_mut().unwrap() {
        for (_, arr) in concept["units"].as_object_mut().unwrap() {
            let a = arr.as_array_mut().unwrap();
            if seed == 0 {
                a.reverse();
                continue;
            }
            for i in (1..a.len()).rev() {
                state = state
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                a.swap(i, (state >> 33) as usize % (i + 1));
            }
        }
    }
    v
}

// ── quarterly: 3-month vs YTD ────────────────────────────────────────────────

#[test]
fn test_series_when_q2_has_three_month_and_ytd_facts_should_keep_only_the_quarter() {
    let c = facts();
    assert_eq!(
        rows(&c, "Revenues", Periodicity::Quarterly, "2024-01-01"),
        s(&[
            ("2023-03-31", 280.0),
            ("2023-06-30", 300.0),
            ("2023-09-30", 320.0)
        ])
    );
}

#[test]
fn test_series_when_fact_file_order_reversed_or_shuffled_should_give_identical_results() {
    let base = facts();
    for seed in 0..8u64 {
        let c = parse(&permuted(seed));
        for concept in [
            "Revenues",
            "Assets",
            "NetIncomeLoss",
            "OperatingIncomeLoss",
            "SalesRevenueNet",
            "CostOfRevenue",
            "NetCashProvidedByUsedInOperatingActivities",
            "Wobble",
        ] {
            for p in [Periodicity::Annual, Periodicity::Quarterly] {
                for as_of in ["2023-12-31", "2024-03-01", "2026-10-25"] {
                    let a = base.series("us-gaap", concept, "USD", p, d(as_of));
                    let b = c.series("us-gaap", concept, "USD", p, d(as_of));
                    let key = |v: &Vec<&Fact>| {
                        v.iter()
                            .map(|f| (f.start, f.end, f.val.to_bits(), f.accn.clone()))
                            .collect::<Vec<_>>()
                    };
                    assert_eq!(key(&a), key(&b), "{concept} {p:?} {as_of} seed {seed}");
                }
            }
        }
    }
}

#[test]
fn test_value_as_of_when_explicit_duration_should_return_each_of_quarter_and_ytd() {
    let c = facts();
    let a = "2024-01-01";
    assert_eq!(
        val(&c, "Revenues", dur("2023-04-01", "2023-06-30"), a),
        Some(300.0)
    );
    assert_eq!(
        val(&c, "Revenues", dur("2023-01-01", "2023-06-30"), a),
        Some(580.0)
    );
    assert_eq!(
        val(&c, "Revenues", dur("2023-01-01", "2023-09-30"), a),
        Some(900.0)
    );
}

#[test]
fn test_value_as_of_when_both_file_orders_should_return_the_same_per_period_values() {
    for seed in 0..4u64 {
        let c = parse(&permuted(seed));
        assert_eq!(
            val(
                &c,
                "Revenues",
                dur("2023-04-01", "2023-06-30"),
                "2024-01-01"
            ),
            Some(300.0)
        );
        assert_eq!(
            val(
                &c,
                "Revenues",
                dur("2023-01-01", "2023-06-30"),
                "2024-01-01"
            ),
            Some(580.0)
        );
    }
}

#[test]
fn test_value_as_of_when_duration_start_differs_should_not_match_other_duration_with_same_end() {
    let c = facts();
    assert_eq!(
        val(
            &c,
            "Revenues",
            dur("2023-02-01", "2023-06-30"),
            "2026-10-25"
        ),
        None
    );
}

#[test]
fn test_value_as_of_when_duration_not_yet_filed_should_be_none_and_visible_on_filed_date() {
    let c = facts();
    let p = dur("2023-01-01", "2023-12-31");
    assert_eq!(val(&c, "Revenues", p, "2024-02-11"), None);
    assert_eq!(val(&c, "Revenues", p, "2024-02-12"), Some(1300.0));
}

#[test]
fn test_value_as_of_when_q4_slice_restated_later_should_return_it_for_its_own_duration_only() {
    let c = facts();
    let q4 = dur("2023-10-01", "2023-12-31");
    assert_eq!(val(&c, "Revenues", q4, "2025-02-10"), Some(400.0));
    assert_eq!(val(&c, "Revenues", q4, "2025-02-11"), Some(410.0));
    assert_eq!(
        val(
            &c,
            "Revenues",
            dur("2023-01-01", "2023-12-31"),
            "2026-10-25"
        ),
        Some(1300.0)
    );
}

// ── annual: 12-month vs Q4/quarter slices in a 10-K ──────────────────────────

#[test]
fn test_series_when_10k_has_annual_q4_and_quarter_slices_should_give_one_row_per_fiscal_year() {
    let c = facts();
    assert_eq!(
        rows(&c, "Revenues", Periodicity::Annual, "2024-03-01"),
        s(&[("2022-12-31", 1200.0), ("2023-12-31", 1300.0)])
    );
}

#[test]
fn test_series_when_annual_should_not_contain_quarter_end_rows_from_10k_slices() {
    let c = facts();
    let ends: Vec<String> = rows(&c, "Revenues", Periodicity::Annual, "2026-10-25")
        .into_iter()
        .map(|r| r.0)
        .collect();
    for q in ["2023-03-31", "2023-06-30", "2023-09-30"] {
        assert!(!ends.contains(&q.to_string()), "{q} leaked: {ends:?}");
    }
    assert_eq!(ends.len(), 3, "{ends:?}"); // FY2022, FY2023, FY2024
}

#[test]
fn test_series_when_later_10k_reports_only_a_q4_slice_should_keep_the_annual_value() {
    let c = facts();
    // FY2024 10-K (2025-02-11) restates only the Q4-2023 slice (410); FY2023 stays 1300.
    assert_eq!(
        rows(&c, "Revenues", Periodicity::Annual, "2026-10-25"),
        s(&[
            ("2022-12-31", 1200.0),
            ("2023-12-31", 1300.0),
            ("2024-12-31", 1500.0)
        ])
    );
}

#[test]
fn test_series_when_as_of_before_first_10k_should_be_empty() {
    let c = facts();
    assert!(rows(&c, "Revenues", Periodicity::Annual, "2023-02-09").is_empty());
}

#[test]
fn test_series_when_52_53_week_years_should_count_as_annual() {
    let c = facts(); // SalesRevenueNet: 363-day FY2021 and 371-day FY2022
    assert_eq!(
        rows(&c, "SalesRevenueNet", Periodicity::Annual, "2026-10-25"),
        s(&[("2021-12-25", 900.0), ("2022-12-31", 950.0)])
    );
}

#[test]
fn test_series_when_13_and_14_week_quarters_should_count_as_quarterly_and_ytd_excluded() {
    let c = facts(); // CostOfRevenue: 97-day Q1, 90-day Q2, 188-day YTD
    assert_eq!(
        rows(&c, "CostOfRevenue", Periodicity::Quarterly, "2026-10-25"),
        s(&[("2023-04-08", 100.0), ("2023-07-08", 110.0)])
    );
}

#[test]
fn test_series_when_cash_flow_is_ytd_only_should_give_only_the_q1_quarter() {
    let c = facts();
    let k = "NetCashProvidedByUsedInOperatingActivities";
    assert_eq!(
        rows(&c, k, Periodicity::Quarterly, "2026-10-25"),
        s(&[("2023-03-31", 120.0)])
    );
    assert_eq!(
        rows(&c, k, Periodicity::Annual, "2026-10-25"),
        s(&[("2023-12-31", 560.0)])
    );
}

// ── instants unaffected ──────────────────────────────────────────────────────

#[test]
fn test_series_when_instant_concept_should_ignore_duration_filter() {
    let c = facts();
    assert_eq!(
        rows(&c, "Assets", Periodicity::Annual, "2024-03-01"),
        s(&[("2022-12-31", 4200.0), ("2023-12-31", 5000.0)])
    );
    assert_eq!(
        rows(&c, "Assets", Periodicity::Quarterly, "2024-01-01"),
        s(&[
            ("2023-03-31", 4300.0),
            ("2023-06-30", 4400.0),
            ("2023-09-30", 4500.0)
        ])
    );
    assert_eq!(
        val(&c, "Assets", Period::Instant(d("2023-12-31")), "2024-03-01"),
        Some(5000.0)
    );
}

#[test]
fn test_value_as_of_when_instant_period_given_for_a_duration_concept_should_be_none() {
    let c = facts();
    assert_eq!(
        val(
            &c,
            "Revenues",
            Period::Instant(d("2023-12-31")),
            "2026-10-25"
        ),
        None
    );
}

#[test]
fn test_value_as_of_when_duration_period_given_for_an_instant_concept_should_be_none() {
    let c = facts();
    assert_eq!(
        val(&c, "Assets", dur("2023-01-01", "2023-12-31"), "2026-10-25"),
        None
    );
}

// ── total order for the winner ───────────────────────────────────────────────

#[test]
fn test_winner_when_same_day_amendment_has_lower_accn_should_still_beat_original() {
    let c = facts(); // NetIncomeLoss: 10-K accn ...-24-000006 (150) vs 10-K/A accn ...-24-000005 (155)
    let p = dur("2023-01-01", "2023-12-31");
    let f = c
        .value_as_of("us-gaap", "NetIncomeLoss", "USD", p, d("2024-03-01"))
        .unwrap();
    assert_eq!((f.val, f.form.as_str()), (155.0, "10-K/A"));
    assert_eq!(
        rows(&c, "NetIncomeLoss", Periodicity::Annual, "2024-03-01"),
        s(&[("2023-12-31", 155.0)])
    );
}

#[test]
fn test_winner_when_same_day_same_form_should_take_higher_accn() {
    let c = facts(); // OperatingIncomeLoss: accn -000006 (200) vs -000009 (210)
    assert_eq!(
        val(
            &c,
            "OperatingIncomeLoss",
            dur("2023-01-01", "2023-12-31"),
            "2024-03-01"
        ),
        Some(210.0)
    );
    assert_eq!(
        rows(&c, "OperatingIncomeLoss", Periodicity::Annual, "2024-03-01"),
        s(&[("2023-12-31", 210.0)])
    );
}

#[test]
fn test_winner_when_everything_else_ties_should_take_greater_start_in_any_file_order() {
    // Wobble: two quarterly-length facts (90d and 97d) with the same end, filed, form, accn.
    for seed in 0..6u64 {
        let c = parse(&permuted(seed));
        assert_eq!(
            rows(&c, "Wobble", Periodicity::Quarterly, "2026-10-25"),
            s(&[("2023-06-30", 1.0)]),
            "seed {seed}"
        );
    }
}

// ── duration boundaries (synthetic) ──────────────────────────────────────────

fn synthetic(form: &str, fp: &str, durations: &[(&str, &str, f64)]) -> CompanyFacts {
    let arr: Vec<Value> = durations
        .iter()
        .map(|(s, e, v)| {
            serde_json::json!({"start": s, "end": e, "val": v, "accn": "0000999999-24-000001",
                "fy": 2024, "fp": fp, "form": form, "filed": "2025-03-01"})
        })
        .collect();
    let j = serde_json::json!({"cik": 999999, "entityName": "Synthetic", "facts": {"us-gaap": {
        "X": {"units": {"USD": arr}}}}});
    parse(&j)
}

#[test]
fn test_series_when_annual_duration_at_boundaries_should_accept_335_to_395_days_only() {
    // days: 334, 335, 395, 396 (all different ends)
    let c = synthetic(
        "10-K",
        "FY",
        &[
            ("2020-01-01", "2020-11-30", 1.0), // 334
            ("2021-01-01", "2021-12-02", 2.0), // 335
            ("2022-01-01", "2023-01-31", 3.0), // 395
            ("2023-01-01", "2024-02-01", 4.0), // 396
        ],
    );
    assert_eq!(
        rows(&c, "X", Periodicity::Annual, "2026-01-01"),
        s(&[("2021-12-02", 2.0), ("2023-01-31", 3.0)])
    );
}

#[test]
fn test_series_when_quarterly_duration_at_boundaries_should_accept_61_to_121_days_only() {
    let c = synthetic(
        "10-Q",
        "Q2",
        &[
            ("2024-01-01", "2024-02-29", 1.0), // 59
            ("2024-01-01", "2024-03-01", 2.0), // 60
            ("2024-01-01", "2024-03-02", 3.0), // 61
            ("2024-01-01", "2024-04-30", 4.0), // 120
            ("2024-01-01", "2024-05-01", 5.0), // 121
            ("2024-01-01", "2024-05-02", 6.0), // 122
        ],
    );
    assert_eq!(
        rows(&c, "X", Periodicity::Quarterly, "2026-01-01"),
        s(&[
            ("2024-03-02", 3.0),
            ("2024-04-30", 4.0),
            ("2024-05-01", 5.0)
        ])
    );
}
