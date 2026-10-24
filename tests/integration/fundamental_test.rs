//! Integration tests for the fundamental scoring pipeline.
//!
//! Verifies that `FundamentalSnapshot` values flow through `fundamental_score()`
//! and into `composite_score_v2()`, affecting final grades.

use ghost_alpha::analysis::fundamental_scorer::{
    fundamental_score, FundamentalInputs, FundamentalScore,
};
use ghost_alpha::analysis::scorer::{
    composite_score_v2, CompositeScore, FundamentalScoreInput, Grade, RegimeWeights,
    SentimentInputs, TechnicalInputs, VolumeInputs,
};
use ghost_alpha::data::fundamentals::{EarningsQuality, FundamentalSnapshot};
use ghost_alpha::data::models::MarketRegime;

use chrono::NaiveDate;

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).expect("valid date")
}

/// Strong-company snapshot: low P/E, high growth, healthy balance sheet.
fn strong_snapshot() -> FundamentalSnapshot {
    FundamentalSnapshot {
        ticker: "STRONG".into(),
        fetched_date: date(2026, 1, 15),
        pe_normalized: Some(14.0),
        pb_annual: Some(3.0),
        ps_annual: Some(4.0),
        eps_growth_ttm_yoy: Some(28.0),
        rev_growth_ttm_yoy: Some(22.0),
        eps_growth_qoq: Some(35.0),
        rev_growth_qoq: Some(30.0),
        gross_margin_ttm: Some(60.0),
        net_margin_ttm: Some(18.0),
        roe_ttm: Some(24.0),
        roi_ttm: Some(16.0),
        debt_equity: Some(0.3),
        current_ratio: Some(2.5),
        fundamental_score: None,
    }
}

/// Weak-company snapshot: high P/E, negative growth, stressed balance sheet.
fn weak_snapshot() -> FundamentalSnapshot {
    FundamentalSnapshot {
        ticker: "WEAK".into(),
        fetched_date: date(2026, 1, 15),
        pe_normalized: Some(95.0),
        pb_annual: Some(12.0),
        ps_annual: Some(18.0),
        eps_growth_ttm_yoy: Some(-15.0),
        rev_growth_ttm_yoy: Some(-8.0),
        eps_growth_qoq: Some(-20.0),
        rev_growth_qoq: Some(-12.0),
        gross_margin_ttm: Some(12.0),
        net_margin_ttm: Some(1.0),
        roe_ttm: Some(2.0),
        roi_ttm: Some(1.0),
        debt_equity: Some(3.5),
        current_ratio: Some(0.6),
        fundamental_score: None,
    }
}

fn strong_quality() -> EarningsQuality {
    EarningsQuality {
        consecutive_beats: 4,
        latest_surprise_pct: Some(12.0),
        avg_surprise_pct_4q: Some(7.0),
        eps_growth_yoy: Some(28.0),
        rev_growth_yoy: Some(22.0),
        days_to_next_earnings: Some(45),
    }
}

fn weak_quality() -> EarningsQuality {
    EarningsQuality {
        consecutive_beats: 0,
        latest_surprise_pct: Some(-6.0),
        avg_surprise_pct_4q: Some(-5.0),
        eps_growth_yoy: Some(-15.0),
        rev_growth_yoy: Some(-8.0),
        days_to_next_earnings: None,
    }
}

/// Good technical profile: 7/8 conditions, strong ADX and RS.
fn good_technicals() -> TechnicalInputs {
    TechnicalInputs {
        conditions_met: 7,
        max_conditions: 8,
        adx: 38.0,
        rs_percentile: 88.0,
    }
}

/// Weak technical profile: 2/8 conditions, low ADX and RS.
fn weak_technicals() -> TechnicalInputs {
    TechnicalInputs {
        conditions_met: 2,
        max_conditions: 8,
        adx: 12.0,
        rs_percentile: 20.0,
    }
}

fn positive_volume() -> VolumeInputs {
    VolumeInputs { zscore: 2.0 }
}

fn neutral_sentiment() -> SentimentInputs {
    SentimentInputs {
        score: 0.0,
        article_count: 0,
    }
}

// ─── E4 Test 1: Fundamental score flows into composite_score_v2 ──────────────

#[test]
fn fundamental_score_included_in_composite_v2() {
    // Step 1: Compute fundamental score from a strong snapshot.
    let inputs = FundamentalInputs {
        snapshot: strong_snapshot(),
        quality: strong_quality(),
        sector_median_pe: Some(20.0),
    };
    let fs: FundamentalScore = fundamental_score(&inputs);

    // Sanity: strong inputs should yield a high fundamental score.
    assert!(
        fs.score >= 75.0,
        "Strong fundamental score should be >= 75, got {:.1}",
        fs.score
    );

    // Step 2: Feed into composite_score_v2 with Bull regime weights.
    let weights = RegimeWeights::for_regime(&MarketRegime::Bull);
    let tech = good_technicals();
    let vol = positive_volume();
    let sent = neutral_sentiment();

    let with_fund = composite_score_v2(
        &tech,
        &vol,
        &sent,
        &FundamentalScoreInput {
            score: Some(fs.score),
        },
        &weights,
    );

    let without_fund = composite_score_v2(
        &tech,
        &vol,
        &sent,
        &FundamentalScoreInput { score: None },
        &weights,
    );

    // The fundamental component should be non-zero when present.
    assert!(
        with_fund.fundamental > 0.0,
        "Fundamental component should be > 0 when score is provided, got {:.2}",
        with_fund.fundamental
    );

    // Without fundamentals, fundamental component must be zero.
    assert!(
        without_fund.fundamental == 0.0,
        "Fundamental component should be 0 when score is None, got {:.2}",
        without_fund.fundamental
    );

    // The two composites should differ because a high fundamental score adds points.
    assert!(
        (with_fund.composite - without_fund.composite).abs() > 0.5,
        "Composites should differ: with={:.2}, without={:.2}",
        with_fund.composite,
        without_fund.composite
    );
}

// ─── E4 Test 2: End-to-end pipeline — snapshot to pillar scores to grade ─────

#[test]
fn end_to_end_strong_vs_weak_grade_differs() {
    let sector_pe = Some(20.0);
    let weights = RegimeWeights::for_regime(&MarketRegime::Bull);

    // --- Strong stock pipeline ---
    let strong_fs = fundamental_score(&FundamentalInputs {
        snapshot: strong_snapshot(),
        quality: strong_quality(),
        sector_median_pe: sector_pe,
    });

    // Verify 4-pillar breakdown is populated.
    assert!(strong_fs.earnings_quality > 0.0, "Earnings quality pillar should be > 0");
    assert!(strong_fs.growth > 0.0, "Growth pillar should be > 0");
    assert!(strong_fs.valuation > 0.0, "Valuation pillar should be > 0");
    assert!(strong_fs.balance_sheet > 0.0, "Balance sheet pillar should be > 0");

    let strong_composite = composite_score_v2(
        &good_technicals(),
        &positive_volume(),
        &neutral_sentiment(),
        &FundamentalScoreInput {
            score: Some(strong_fs.score),
        },
        &weights,
    );

    // --- Weak stock pipeline ---
    let weak_fs = fundamental_score(&FundamentalInputs {
        snapshot: weak_snapshot(),
        quality: weak_quality(),
        sector_median_pe: sector_pe,
    });

    // Weak fundamentals should score much lower.
    assert!(
        weak_fs.score < strong_fs.score - 30.0,
        "Weak score ({:.1}) should be at least 30 points below strong ({:.1})",
        weak_fs.score,
        strong_fs.score
    );

    let weak_composite = composite_score_v2(
        &weak_technicals(),
        &VolumeInputs { zscore: -1.5 },
        &neutral_sentiment(),
        &FundamentalScoreInput {
            score: Some(weak_fs.score),
        },
        &weights,
    );

    // Strong composite should meaningfully beat weak composite.
    assert!(
        strong_composite.composite > weak_composite.composite + 20.0,
        "Strong composite ({:.1}) should exceed weak ({:.1}) by > 20 pts",
        strong_composite.composite,
        weak_composite.composite
    );

    // Use assign_grades to verify grade separation.
    let mut scores = vec![strong_composite.clone(), weak_composite.clone()];
    ghost_alpha::analysis::scorer::assign_grades(&mut scores).expect("assign_grades should succeed");

    // With only 2 stocks, the higher one gets a better grade.
    assert!(
        scores[0].grade > scores[1].grade,
        "Strong stock grade ({}) should beat weak stock grade ({})",
        scores[0].grade,
        scores[1].grade
    );
}

// ─── E4 Test 3: Fundamental weight = 0 means no fundamental influence ────────

#[test]
fn legacy_weights_ignore_fundamental_score() {
    let legacy = RegimeWeights::legacy(&MarketRegime::Bull);
    assert!(
        legacy.fundamental == 0.0,
        "Legacy weights should have 0 fundamental weight"
    );

    let fs = fundamental_score(&FundamentalInputs {
        snapshot: strong_snapshot(),
        quality: strong_quality(),
        sector_median_pe: Some(20.0),
    });

    let result = composite_score_v2(
        &good_technicals(),
        &positive_volume(),
        &neutral_sentiment(),
        &FundamentalScoreInput {
            score: Some(fs.score),
        },
        &legacy,
    );

    // Even though we provide a fundamental score, the weight is 0, so the
    // composite fundamental component should be 0 and the weight should be
    // redistributed to the other three components.
    assert!(
        result.fundamental == 0.0,
        "Fundamental component should be 0 with legacy weights, got {:.2}",
        result.fundamental
    );
}
