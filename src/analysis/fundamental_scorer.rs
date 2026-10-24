use crate::data::fundamentals::{EarningsQuality, EarningsRecord, FundamentalSnapshot};
use chrono::NaiveDate;

// ─── Input / Output Types ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FundamentalInputs {
    pub snapshot: FundamentalSnapshot,
    pub quality: EarningsQuality,
    pub sector_median_pe: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct FundamentalScore {
    /// Composite fundamental score clamped to 0–100.
    pub score: f64,
    /// Earnings quality pillar (0–40).
    pub earnings_quality: f64,
    /// Growth rate pillar (0–30).
    pub growth: f64,
    /// Valuation pillar (0–20).
    pub valuation: f64,
    /// Balance sheet pillar (0–10).
    pub balance_sheet: f64,
}

// ─── Main Scoring Function ──────────────────────────────────────────────────

/// Compute a 0–100 fundamental sub-score from financial metrics and earnings
/// quality data.
pub fn fundamental_score(inputs: &FundamentalInputs) -> FundamentalScore {
    let earnings_quality = earnings_quality_score(&inputs.quality);
    let growth = growth_score(&inputs.snapshot);
    let valuation = valuation_score(&inputs.snapshot, inputs.sector_median_pe);
    let balance_sheet = balance_sheet_score(&inputs.snapshot);
    let score = (earnings_quality + growth + valuation + balance_sheet).clamp(0.0, 100.0);
    FundamentalScore {
        score,
        earnings_quality,
        growth,
        valuation,
        balance_sheet,
    }
}

// ─── Earnings Quality Pillar: 0–40 pts ─────────────────────────────────────

/// Score earnings quality on a 0–40 scale.
///
/// - Consecutive beats: 5 pts per quarter, max 4 quarters → 20 pts
/// - Latest EPS surprise: >= 10% → 12, linear 0–10% → 0–12, negative → 0
/// - Average 4Q surprise: >= 5% → 8, 0–5% → 4–8, None → 4, negative → scaled 0–4
fn earnings_quality_score(q: &EarningsQuality) -> f64 {
    // Consecutive beats: 5 per quarter, cap at 4.
    let beats = (q.consecutive_beats.min(4) as f64) * 5.0;

    // Latest surprise percentage.
    let latest = match q.latest_surprise_pct {
        Some(s) if s >= 10.0 => 12.0,
        Some(s) if s > 0.0 => (s / 10.0) * 12.0,
        Some(_) => 0.0, // negative or zero
        None => 0.0,
    };

    // Average 4Q surprise percentage.
    let avg = match q.avg_surprise_pct_4q {
        Some(a) if a >= 5.0 => 8.0,
        Some(a) if a >= 0.0 => 4.0 + (a / 5.0) * 4.0,
        Some(a) => {
            // Negative: scale 0–4, where -10% or worse → 0.
            (4.0 + (a / 10.0) * 4.0).clamp(0.0, 4.0)
        }
        None => 4.0, // neutral
    };

    (beats + latest + avg).min(40.0)
}

// ─── Growth Rate Pillar: 0–30 pts ──────────────────────────────────────────

/// Map an optional growth rate to 0–`max` points.
///
/// - >= 30% → full points
/// - 0–30% → linear interpolation
/// - negative → 0
/// - None → half points (neutral)
fn growth_pts(growth: Option<f64>, max: f64) -> f64 {
    match growth {
        Some(g) if g >= 30.0 => max,
        Some(g) if g >= 0.0 => (g / 30.0) * max,
        Some(_) => 0.0,
        None => max / 2.0,
    }
}

/// Score growth on a 0–30 scale (revenue 15 + EPS 15).
fn growth_score(snap: &FundamentalSnapshot) -> f64 {
    let rev = growth_pts(snap.rev_growth_qoq, 15.0);
    let eps = growth_pts(snap.eps_growth_qoq, 15.0);
    rev + eps
}

// ─── Valuation Pillar: 0–20 pts ────────────────────────────────────────────

/// Score valuation on a 0–20 scale.
///
/// With sector median PE: PE/median ratio determines bucket.
/// Without sector data: absolute PE fallback.
/// No PE or negative earnings → 10 (neutral).
fn valuation_score(snap: &FundamentalSnapshot, sector_median_pe: Option<f64>) -> f64 {
    let pe = match snap.pe_normalized {
        Some(pe) if pe > 0.0 => pe,
        _ => return 10.0, // negative earnings or unavailable → neutral
    };

    if let Some(median) = sector_median_pe {
        if median > 0.0 {
            let ratio = pe / median;
            return if ratio <= 1.0 {
                20.0
            } else if ratio <= 1.5 {
                15.0
            } else if ratio <= 2.0 {
                8.0
            } else {
                2.0
            };
        }
    }

    // Absolute PE fallback.
    if pe <= 15.0 {
        20.0
    } else if pe <= 25.0 {
        15.0
    } else if pe <= 40.0 {
        8.0
    } else {
        2.0
    }
}

// ─── Balance Sheet Pillar: 0–10 pts ────────────────────────────────────────

/// Score balance sheet health on a 0–10 scale.
///
/// - Current ratio: >= 1.5 → 5, >= 1.0 → 3, < 1.0 → 0, None → 2.5
/// - Debt/equity: <= 0.5 → 5, <= 1.0 → 3, <= 2.0 → 1, > 2.0 → 0, None → 2.5
fn balance_sheet_score(snap: &FundamentalSnapshot) -> f64 {
    let cr = match snap.current_ratio {
        Some(r) if r >= 1.5 => 5.0,
        Some(r) if r >= 1.0 => 3.0,
        Some(_) => 0.0,
        None => 2.5,
    };

    let de = match snap.debt_equity {
        Some(d) if d <= 0.5 => 5.0,
        Some(d) if d <= 1.0 => 3.0,
        Some(d) if d <= 2.0 => 1.0,
        Some(_) => 0.0,
        None => 2.5,
    };

    cr + de
}

// ─── Derive Earnings Quality ────────────────────────────────────────────────

/// Derive [`EarningsQuality`] from a slice of [`EarningsRecord`]s.
///
/// Records are filtered to past (reported) entries, sorted most-recent-first,
/// and used to compute consecutive beats, average surprise, and days to next
/// upcoming report.
pub fn derive_earnings_quality(records: &[EarningsRecord], today: NaiveDate) -> EarningsQuality {
    // Partition into reported (past) and upcoming (future / unreported).
    let mut reported: Vec<&EarningsRecord> = records
        .iter()
        .filter(|r| r.eps_actual.is_some() && !r.is_upcoming)
        .collect();
    reported.sort_by(|a, b| b.period_date.cmp(&a.period_date));

    let upcoming: Option<&EarningsRecord> = records
        .iter()
        .filter(|r| r.is_upcoming)
        .filter_map(|r| {
            let date = r.earnings_date.unwrap_or(r.period_date);
            if date > today { Some((r, date)) } else { None }
        })
        .min_by_key(|(_, d)| *d)
        .map(|(r, _)| r);

    // Consecutive beats from most recent.
    let mut consecutive_beats: u8 = 0;
    for r in &reported {
        match r.eps_surprise_pct {
            Some(s) if s > 0.0 => consecutive_beats += 1,
            _ => break,
        }
    }

    // Latest surprise.
    let latest_surprise_pct = reported.first().and_then(|r| r.eps_surprise_pct);

    // Average surprise over last 4 reported quarters.
    let avg_surprise_pct_4q = {
        let last4: Vec<f64> = reported
            .iter()
            .take(4)
            .filter_map(|r| r.eps_surprise_pct)
            .collect();
        if last4.is_empty() {
            None
        } else {
            Some(last4.iter().sum::<f64>() / last4.len() as f64)
        }
    };

    // Days to next upcoming earnings.
    let days_to_next_earnings = upcoming.map(|r| {
        let date = r.earnings_date.unwrap_or(r.period_date);
        (date - today).num_days()
    });

    // Growth YoY from snapshot-adjacent records (if available).
    // These are passed through from the snapshot in normal usage, so we leave
    // them as None here — the caller can overlay from the snapshot.
    EarningsQuality {
        consecutive_beats,
        latest_surprise_pct,
        avg_surprise_pct_4q,
        eps_growth_yoy: None,
        rev_growth_yoy: None,
        days_to_next_earnings,
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    // ── Test Helpers ────────────────────────────────────────────────────────

    fn perfect_snapshot() -> FundamentalSnapshot {
        FundamentalSnapshot {
            ticker: "TEST".into(),
            fetched_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            pe_normalized: Some(15.0),
            pb_annual: Some(3.0),
            ps_annual: Some(5.0),
            eps_growth_ttm_yoy: Some(25.0),
            rev_growth_ttm_yoy: Some(20.0),
            eps_growth_qoq: Some(35.0),
            rev_growth_qoq: Some(32.0),
            gross_margin_ttm: Some(65.0),
            net_margin_ttm: Some(20.0),
            roe_ttm: Some(25.0),
            roi_ttm: Some(18.0),
            debt_equity: Some(0.3),
            current_ratio: Some(2.5),
            fundamental_score: None,
        }
    }

    fn zero_snapshot() -> FundamentalSnapshot {
        FundamentalSnapshot {
            ticker: "TEST".into(),
            fetched_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            pe_normalized: Some(100.0),
            pb_annual: Some(10.0),
            ps_annual: Some(15.0),
            eps_growth_ttm_yoy: Some(-10.0),
            rev_growth_ttm_yoy: Some(-5.0),
            eps_growth_qoq: Some(-20.0),
            rev_growth_qoq: Some(-10.0),
            gross_margin_ttm: Some(10.0),
            net_margin_ttm: Some(2.0),
            roe_ttm: Some(3.0),
            roi_ttm: Some(2.0),
            debt_equity: Some(3.0),
            current_ratio: Some(0.5),
            fundamental_score: None,
        }
    }

    fn perfect_quality() -> EarningsQuality {
        EarningsQuality {
            consecutive_beats: 4,
            latest_surprise_pct: Some(15.0),
            avg_surprise_pct_4q: Some(8.0),
            eps_growth_yoy: Some(25.0),
            rev_growth_yoy: Some(20.0),
            days_to_next_earnings: Some(30),
        }
    }

    fn zero_quality() -> EarningsQuality {
        EarningsQuality {
            consecutive_beats: 0,
            latest_surprise_pct: Some(-5.0),
            avg_surprise_pct_4q: Some(-8.0),
            eps_growth_yoy: Some(-10.0),
            rev_growth_yoy: Some(-5.0),
            days_to_next_earnings: None,
        }
    }

    fn neutral_quality() -> EarningsQuality {
        EarningsQuality {
            consecutive_beats: 0,
            latest_surprise_pct: None,
            avg_surprise_pct_4q: None,
            eps_growth_yoy: None,
            rev_growth_yoy: None,
            days_to_next_earnings: None,
        }
    }

    // ── Earnings Quality Tests ──────────────────────────────────────────────

    #[test]
    fn earnings_quality_perfect_near_40() {
        let score = earnings_quality_score(&perfect_quality());
        assert!(
            score >= 38.0 && score <= 40.0,
            "perfect quality should be near 40, got {score}"
        );
    }

    #[test]
    fn earnings_quality_zero_beats_negative_surprise() {
        let score = earnings_quality_score(&zero_quality());
        // 0 beats + negative latest (0) + negative avg (scaled ~0.8)
        assert!(
            score < 2.0,
            "zero beats + negative surprise should be near 0, got {score}"
        );
    }

    #[test]
    fn earnings_quality_no_data_neutral() {
        let score = earnings_quality_score(&neutral_quality());
        // 0 beats + 0 latest + 4 neutral avg = 4
        assert!(
            (score - 4.0).abs() < 0.01,
            "no data should give neutral 4, got {score}"
        );
    }

    #[test]
    fn earnings_quality_four_consecutive_beats_base_20() {
        let q = EarningsQuality {
            consecutive_beats: 4,
            latest_surprise_pct: Some(0.0),
            avg_surprise_pct_4q: Some(0.0),
            ..EarningsQuality::default()
        };
        let score = earnings_quality_score(&q);
        // 20 (beats) + 0 (latest: 0% → 0) + 4 (avg: 0% → 4.0) = 24
        assert!(
            (score - 24.0).abs() < 0.01,
            "4 beats + zero surprise should give 24, got {score}"
        );
    }

    #[test]
    fn earnings_quality_capped_at_40() {
        let q = EarningsQuality {
            consecutive_beats: 10, // more than 4
            latest_surprise_pct: Some(50.0),
            avg_surprise_pct_4q: Some(20.0),
            ..EarningsQuality::default()
        };
        let score = earnings_quality_score(&q);
        assert!(
            (score - 40.0).abs() < 0.01,
            "should cap at 40, got {score}"
        );
    }

    // ── Growth Score Tests ──────────────────────────────────────────────────

    #[test]
    fn growth_30_pct_each_gives_30() {
        let mut snap = perfect_snapshot();
        snap.rev_growth_qoq = Some(30.0);
        snap.eps_growth_qoq = Some(30.0);
        let score = growth_score(&snap);
        assert!(
            (score - 30.0).abs() < 0.01,
            "30% each should give 30, got {score}"
        );
    }

    #[test]
    fn growth_zero_gives_zero() {
        let mut snap = perfect_snapshot();
        snap.rev_growth_qoq = Some(0.0);
        snap.eps_growth_qoq = Some(0.0);
        let score = growth_score(&snap);
        assert!(
            score.abs() < 0.01,
            "zero growth should give 0, got {score}"
        );
    }

    #[test]
    fn growth_none_gives_half() {
        let mut snap = perfect_snapshot();
        snap.rev_growth_qoq = None;
        snap.eps_growth_qoq = None;
        let score = growth_score(&snap);
        assert!(
            (score - 15.0).abs() < 0.01,
            "None growth should give 15 (half), got {score}"
        );
    }

    #[test]
    fn growth_negative_gives_zero() {
        let mut snap = perfect_snapshot();
        snap.rev_growth_qoq = Some(-10.0);
        snap.eps_growth_qoq = Some(-5.0);
        let score = growth_score(&snap);
        assert!(
            score.abs() < 0.01,
            "negative growth should give 0, got {score}"
        );
    }

    // ── Valuation Score Tests ───────────────────────────────────────────────

    #[test]
    fn valuation_below_sector_median_gives_20() {
        let mut snap = perfect_snapshot();
        snap.pe_normalized = Some(12.0);
        let score = valuation_score(&snap, Some(15.0));
        assert!(
            (score - 20.0).abs() < 0.01,
            "below median should give 20, got {score}"
        );
    }

    #[test]
    fn valuation_at_2x_median_gives_8() {
        let mut snap = perfect_snapshot();
        snap.pe_normalized = Some(30.0);
        let score = valuation_score(&snap, Some(15.0));
        assert!(
            (score - 8.0).abs() < 0.01,
            "2x median should give 8, got {score}"
        );
    }

    #[test]
    fn valuation_above_2x_median_gives_2() {
        let mut snap = perfect_snapshot();
        snap.pe_normalized = Some(50.0);
        let score = valuation_score(&snap, Some(15.0));
        assert!(
            (score - 2.0).abs() < 0.01,
            "above 2x median should give 2, got {score}"
        );
    }

    #[test]
    fn valuation_no_pe_neutral_10() {
        let mut snap = perfect_snapshot();
        snap.pe_normalized = None;
        let score = valuation_score(&snap, Some(15.0));
        assert!(
            (score - 10.0).abs() < 0.01,
            "no PE should give neutral 10, got {score}"
        );
    }

    #[test]
    fn valuation_negative_pe_neutral_10() {
        let mut snap = perfect_snapshot();
        snap.pe_normalized = Some(-5.0);
        let score = valuation_score(&snap, Some(15.0));
        assert!(
            (score - 10.0).abs() < 0.01,
            "negative PE should give neutral 10, got {score}"
        );
    }

    #[test]
    fn valuation_no_sector_uses_absolute_fallback() {
        let mut snap = perfect_snapshot();
        snap.pe_normalized = Some(20.0); // 15–25 bucket → 15 pts
        let score = valuation_score(&snap, None);
        assert!(
            (score - 15.0).abs() < 0.01,
            "no sector, PE 20 should give 15 (absolute fallback), got {score}"
        );
    }

    // ── Balance Sheet Tests ─────────────────────────────────────────────────

    #[test]
    fn balance_sheet_perfect_gives_10() {
        let snap = perfect_snapshot(); // CR 2.5, D/E 0.3
        let score = balance_sheet_score(&snap);
        assert!(
            (score - 10.0).abs() < 0.01,
            "perfect balance sheet should give 10, got {score}"
        );
    }

    #[test]
    fn balance_sheet_high_debt_low_cr_gives_zero() {
        let snap = zero_snapshot(); // CR 0.5, D/E 3.0
        let score = balance_sheet_score(&snap);
        assert!(
            score.abs() < 0.01,
            "high debt + low CR should give 0, got {score}"
        );
    }

    #[test]
    fn balance_sheet_no_data_gives_5() {
        let mut snap = perfect_snapshot();
        snap.current_ratio = None;
        snap.debt_equity = None;
        let score = balance_sheet_score(&snap);
        assert!(
            (score - 5.0).abs() < 0.01,
            "no data should give 5 (neutral), got {score}"
        );
    }

    // ── Composite Fundamental Score Tests ───────────────────────────────────

    #[test]
    fn composite_bounded_0_100() {
        let inputs = FundamentalInputs {
            snapshot: perfect_snapshot(),
            quality: perfect_quality(),
            sector_median_pe: Some(20.0),
        };
        let fs = fundamental_score(&inputs);
        assert!(fs.score >= 0.0 && fs.score <= 100.0);
    }

    #[test]
    fn composite_perfect_inputs_gte_85() {
        let inputs = FundamentalInputs {
            snapshot: perfect_snapshot(),
            quality: perfect_quality(),
            sector_median_pe: Some(20.0),
        };
        let fs = fundamental_score(&inputs);
        assert!(
            fs.score >= 85.0,
            "perfect inputs should score >= 85, got {}",
            fs.score
        );
    }

    #[test]
    fn composite_poor_inputs_lt_15() {
        let inputs = FundamentalInputs {
            snapshot: zero_snapshot(),
            quality: zero_quality(),
            sector_median_pe: Some(15.0),
        };
        let fs = fundamental_score(&inputs);
        assert!(
            fs.score < 15.0,
            "poor inputs should score < 15, got {}",
            fs.score
        );
    }

    // ── Derive Earnings Quality Tests ───────────────────────────────────────

    fn make_record(
        period: &str,
        actual: Option<f64>,
        surprise_pct: Option<f64>,
        is_upcoming: bool,
        earnings_date: Option<&str>,
    ) -> EarningsRecord {
        EarningsRecord {
            ticker: "TEST".into(),
            period_date: NaiveDate::parse_from_str(period, "%Y-%m-%d").unwrap(),
            fiscal_year: 2025,
            fiscal_quarter: 1,
            eps_actual: actual,
            eps_estimate: Some(1.0),
            eps_surprise: actual.map(|a| a - 1.0),
            eps_surprise_pct: surprise_pct,
            is_upcoming,
            earnings_date: earnings_date
                .map(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap()),
        }
    }

    #[test]
    fn derive_counts_consecutive_beats() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let records = vec![
            make_record("2025-03-31", Some(1.10), Some(10.0), false, None),
            make_record("2025-06-30", Some(1.15), Some(15.0), false, None),
            make_record("2025-09-30", Some(1.05), Some(5.0), false, None),
            make_record("2025-12-31", Some(1.20), Some(20.0), false, None),
        ];
        let q = derive_earnings_quality(&records, today);
        assert_eq!(q.consecutive_beats, 4);
    }

    #[test]
    fn derive_miss_breaks_streak() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let records = vec![
            make_record("2025-03-31", Some(1.10), Some(10.0), false, None),
            make_record("2025-06-30", Some(0.90), Some(-10.0), false, None), // miss
            make_record("2025-09-30", Some(1.05), Some(5.0), false, None),
            make_record("2025-12-31", Some(1.20), Some(20.0), false, None),
        ];
        let q = derive_earnings_quality(&records, today);
        // Most recent two beat (2025-12, 2025-09), then a miss → streak = 2
        assert_eq!(q.consecutive_beats, 2);
    }

    #[test]
    fn derive_empty_records() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let q = derive_earnings_quality(&[], today);
        assert_eq!(q.consecutive_beats, 0);
        assert!(q.latest_surprise_pct.is_none());
        assert!(q.avg_surprise_pct_4q.is_none());
        assert!(q.days_to_next_earnings.is_none());
    }

    #[test]
    fn derive_finds_days_to_next_earnings() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let records = vec![
            make_record("2025-12-31", Some(1.10), Some(10.0), false, None),
            // Upcoming: no actual EPS yet, earnings_date is the report date.
            make_record("2026-03-31", None, None, true, Some("2026-04-15")),
        ];
        let q = derive_earnings_quality(&records, today);
        assert_eq!(q.days_to_next_earnings, Some(45));
    }

    #[test]
    fn derive_avg_surprise_uses_last_4() {
        let today = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let records = vec![
            make_record("2024-09-30", Some(1.05), Some(5.0), false, None), // 5th oldest, excluded
            make_record("2024-12-31", Some(1.10), Some(10.0), false, None),
            make_record("2025-03-31", Some(1.20), Some(20.0), false, None),
            make_record("2025-06-30", Some(1.00), Some(0.0), false, None),
            make_record("2025-09-30", Some(1.30), Some(30.0), false, None),
        ];
        let q = derive_earnings_quality(&records, today);
        // Last 4 by date: 2025-09 (30), 2025-06 (0), 2025-03 (20), 2024-12 (10) → avg 15
        assert!(
            (q.avg_surprise_pct_4q.unwrap() - 15.0).abs() < 0.01,
            "avg 4Q surprise should be 15.0, got {:?}",
            q.avg_surprise_pct_4q
        );
    }
}
