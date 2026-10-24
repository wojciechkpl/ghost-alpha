//! Stress test E9: exercise the fundamental scoring pipeline for a 500-stock universe.
//!
//! Generates 500 deterministic stock profiles with varying financial metrics,
//! runs them through `derive_earnings_quality()`, `fundamental_score()`, and
//! `composite_score_v2()`, then asserts correctness bounds and performance.

use ghost_alpha::analysis::fundamental_scorer::{
    derive_earnings_quality, fundamental_score, FundamentalInputs,
};
use ghost_alpha::analysis::scorer::{
    assign_grades, composite_score_v2, FundamentalScoreInput, RegimeWeights, SentimentInputs,
    TechnicalInputs, VolumeInputs,
};
use ghost_alpha::data::fundamentals::{EarningsRecord, FundamentalSnapshot};
use ghost_alpha::data::models::MarketRegime;

use chrono::NaiveDate;
use std::time::Instant;

// ─── Deterministic Generators ───────────────────────────────────────────────

/// Simple deterministic hash to produce varied but repeatable f64 values.
/// Returns a value in [0.0, 1.0).
fn pseudo_random(seed: u64) -> f64 {
    // xorshift-style mixing
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    (x as f64) / (u64::MAX as f64)
}

/// Map a [0,1) value into a range [lo, hi].
fn in_range(t: f64, lo: f64, hi: f64) -> f64 {
    lo + t * (hi - lo)
}

/// Map a [0,1) value into an Option<f64> in [lo, hi], with ~20% chance of None.
fn maybe_in_range(t: f64, lo: f64, hi: f64) -> Option<f64> {
    if t < 0.2 {
        None
    } else {
        Some(in_range((t - 0.2) / 0.8, lo, hi))
    }
}

fn generate_snapshot(i: u64) -> FundamentalSnapshot {
    FundamentalSnapshot {
        ticker: format!("STK{i:04}"),
        fetched_date: NaiveDate::from_ymd_opt(2026, 1, 15).expect("valid date"),
        pe_normalized: maybe_in_range(pseudo_random(i * 17 + 1), -5.0, 80.0),
        pb_annual: maybe_in_range(pseudo_random(i * 17 + 2), 0.5, 15.0),
        ps_annual: maybe_in_range(pseudo_random(i * 17 + 3), 0.5, 20.0),
        eps_growth_ttm_yoy: maybe_in_range(pseudo_random(i * 17 + 4), -30.0, 60.0),
        rev_growth_ttm_yoy: maybe_in_range(pseudo_random(i * 17 + 5), -20.0, 50.0),
        eps_growth_qoq: maybe_in_range(pseudo_random(i * 17 + 6), -25.0, 45.0),
        rev_growth_qoq: maybe_in_range(pseudo_random(i * 17 + 7), -15.0, 40.0),
        gross_margin_ttm: maybe_in_range(pseudo_random(i * 17 + 8), 5.0, 80.0),
        net_margin_ttm: maybe_in_range(pseudo_random(i * 17 + 9), -10.0, 35.0),
        roe_ttm: maybe_in_range(pseudo_random(i * 17 + 10), -5.0, 40.0),
        roi_ttm: maybe_in_range(pseudo_random(i * 17 + 11), -5.0, 30.0),
        debt_equity: maybe_in_range(pseudo_random(i * 17 + 12), 0.0, 4.0),
        current_ratio: maybe_in_range(pseudo_random(i * 17 + 13), 0.3, 3.5),
        fundamental_score: None,
    }
}

fn generate_earnings_records(i: u64) -> Vec<EarningsRecord> {
    let base_date = NaiveDate::from_ymd_opt(2025, 3, 31).expect("valid date");
    let ticker = format!("STK{i:04}");

    (0..4)
        .map(|q| {
            let seed = i * 100 + q;
            let t = pseudo_random(seed);
            let surprise = in_range(pseudo_random(seed + 50), -15.0, 25.0);
            let actual = in_range(t, 0.5, 3.0);
            let period_date = base_date
                .checked_add_months(chrono::Months::new(q as u32 * 3))
                .expect("valid date offset");

            EarningsRecord {
                ticker: ticker.clone(),
                period_date,
                fiscal_year: 2025,
                fiscal_quarter: (q + 1) as u8,
                eps_actual: Some(actual),
                eps_estimate: Some(actual - surprise * actual / 100.0),
                eps_surprise: Some(surprise * actual / 100.0),
                eps_surprise_pct: Some(surprise),
                is_upcoming: false,
                earnings_date: None,
            }
        })
        .collect()
}

fn generate_technical_inputs(i: u64) -> TechnicalInputs {
    let t = pseudo_random(i * 31 + 100);
    TechnicalInputs {
        conditions_met: (in_range(t, 0.0, 8.99) as u8).min(8),
        max_conditions: 8,
        adx: in_range(pseudo_random(i * 31 + 101), 5.0, 60.0),
        rs_percentile: in_range(pseudo_random(i * 31 + 102), 0.0, 100.0),
    }
}

fn generate_volume_inputs(i: u64) -> VolumeInputs {
    VolumeInputs {
        zscore: in_range(pseudo_random(i * 31 + 200), -3.5, 3.5),
    }
}

fn generate_sentiment_inputs(i: u64) -> SentimentInputs {
    SentimentInputs {
        score: in_range(pseudo_random(i * 31 + 300), -1.0, 1.0),
        article_count: in_range(pseudo_random(i * 31 + 301), 0.0, 10.99) as usize,
    }
}

// ─── Test ───────────────────────────────────────────────────────────────────

#[test]
fn stress_500_stocks_scoring_pipeline() {
    const N: u64 = 500;
    let today = NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid date");
    let weights = RegimeWeights::for_regime(&MarketRegime::Bull);

    let start = Instant::now();

    let mut composite_scores = Vec::with_capacity(N as usize);

    for i in 0..N {
        // Generate deterministic inputs for this stock.
        let snapshot = generate_snapshot(i);
        let records = generate_earnings_records(i);
        let tech = generate_technical_inputs(i);
        let vol = generate_volume_inputs(i);
        let sent = generate_sentiment_inputs(i);

        // Step 1: derive earnings quality from records.
        let quality = derive_earnings_quality(&records, today);

        // Step 2: compute fundamental score.
        let sector_median_pe = if pseudo_random(i * 7 + 999) > 0.3 {
            Some(in_range(pseudo_random(i * 7 + 998), 10.0, 30.0))
        } else {
            None
        };
        let fund_inputs = FundamentalInputs {
            snapshot,
            quality,
            sector_median_pe,
        };
        let fund_score = fundamental_score(&fund_inputs);

        // Assert fundamental sub-score is in [0, 100].
        assert!(
            fund_score.score >= 0.0 && fund_score.score <= 100.0,
            "Stock {i}: fundamental score {:.2} out of [0, 100]",
            fund_score.score
        );
        // Assert pillar bounds.
        assert!(
            fund_score.earnings_quality >= 0.0 && fund_score.earnings_quality <= 40.0,
            "Stock {i}: earnings_quality {:.2} out of [0, 40]",
            fund_score.earnings_quality
        );
        assert!(
            fund_score.growth >= 0.0 && fund_score.growth <= 30.0,
            "Stock {i}: growth {:.2} out of [0, 30]",
            fund_score.growth
        );
        assert!(
            fund_score.valuation >= 0.0 && fund_score.valuation <= 20.0,
            "Stock {i}: valuation {:.2} out of [0, 20]",
            fund_score.valuation
        );
        assert!(
            fund_score.balance_sheet >= 0.0 && fund_score.balance_sheet <= 10.0,
            "Stock {i}: balance_sheet {:.2} out of [0, 10]",
            fund_score.balance_sheet
        );

        // Step 3: composite score v2.
        let fund_input = FundamentalScoreInput {
            score: Some(fund_score.score),
        };
        let composite = composite_score_v2(&tech, &vol, &sent, &fund_input, &weights);

        assert!(
            composite.composite >= 0.0 && composite.composite <= 100.0,
            "Stock {i}: composite {:.2} out of [0, 100]",
            composite.composite
        );
        assert!(
            composite.fundamental >= 0.0,
            "Stock {i}: composite.fundamental {:.2} negative",
            composite.fundamental
        );

        composite_scores.push(composite);
    }

    // Step 4: assign percentile grades across the full universe.
    assign_grades(&mut composite_scores).expect("assign_grades should not fail");

    let elapsed = start.elapsed();

    // All 500 stocks processed.
    assert_eq!(composite_scores.len(), N as usize);

    // Performance: must complete within 1 second.
    assert!(
        elapsed.as_secs_f64() < 1.0,
        "Pipeline took {:.3}s for {N} stocks, expected < 1.0s",
        elapsed.as_secs_f64()
    );

    // Verify grade distribution is non-degenerate (not all the same grade).
    let first_grade = &composite_scores[0].grade;
    let all_same = composite_scores.iter().all(|s| &s.grade == first_grade);
    assert!(
        !all_same,
        "All 500 stocks received the same grade ({first_grade}), distribution is degenerate"
    );

    // Verify score spread: the universe should have some variance.
    let min_score = composite_scores
        .iter()
        .map(|s| s.composite)
        .fold(f64::INFINITY, f64::min);
    let max_score = composite_scores
        .iter()
        .map(|s| s.composite)
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(
        max_score - min_score > 10.0,
        "Score spread too narrow: min={min_score:.2}, max={max_score:.2}"
    );
}
