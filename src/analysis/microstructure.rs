//! Market microstructure awareness — volatility regime, correlation regime,
//! and liquidity calendar classification.
//!
//! Phase 6 (F22): Adds market structure context beyond simple Bull/Bear
//! regime detection. Three independent signals combine into a favorability
//! score that modulates position sizing and strategy selection aggression.

use chrono::NaiveDate;

// ─── Volatility Regime ──────────────────────────────────────────────────────

/// Volatility regime classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum VolRegime {
    /// VIX < 15 AND realized vol < 10%
    Low,
    /// Normal conditions
    Normal,
    /// VIX > 25 OR realized vol > 20%
    High,
}

/// Classify volatility regime from VIX level and realized volatility.
///
/// Both VIX and realized vol are required to confirm Low; either alone
/// can trigger High (defensive posture).
pub fn classify_vol_regime(vix: f64, realized_vol_20d_annualized: f64) -> VolRegime {
    if vix > 25.0 || realized_vol_20d_annualized > 20.0 {
        VolRegime::High
    } else if vix < 15.0 && realized_vol_20d_annualized < 10.0 {
        VolRegime::Low
    } else {
        VolRegime::Normal
    }
}

// ─── Correlation Regime ─────────────────────────────────────────────────────

/// Correlation regime — how correlated are sector returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CorrRegime {
    /// avg pairwise sector correlation < 0.30 — stock picker's market
    LowCorrelation,
    /// Normal correlation environment
    Normal,
    /// avg pairwise sector correlation > 0.70 — risk-on/off market
    HighCorrelation,
}

/// Classify correlation regime from average pairwise sector correlations.
///
/// Takes pre-computed average correlation. Use `compute_avg_sector_correlation`
/// to compute from raw sector return series.
pub fn classify_corr_regime(avg_sector_correlation: f64) -> CorrRegime {
    if avg_sector_correlation > 0.70 {
        CorrRegime::HighCorrelation
    } else if avg_sector_correlation < 0.30 {
        CorrRegime::LowCorrelation
    } else {
        CorrRegime::Normal
    }
}

/// Compute average pairwise correlation from sector return series.
///
/// Each entry is (sector_name, daily_returns). Uses trailing `window` days.
/// Returns 0.5 (neutral) if insufficient data.
pub fn compute_avg_sector_correlation(sector_returns: &[(&str, &[f64])], window: usize) -> f64 {
    use crate::analysis::correlation::pearson_correlation;

    if sector_returns.len() < 2 {
        return 0.5;
    }

    let mut total_corr = 0.0;
    let mut pair_count = 0usize;

    for i in 0..sector_returns.len() {
        for j in (i + 1)..sector_returns.len() {
            let a = sector_returns[i].1;
            let b = sector_returns[j].1;
            let a_tail = &a[a.len().saturating_sub(window)..];
            let b_tail = &b[b.len().saturating_sub(window)..];
            if a_tail.len() == b_tail.len() && a_tail.len() >= 20 {
                if let Ok(corr) = pearson_correlation(a_tail, b_tail) {
                    total_corr += corr;
                    pair_count += 1;
                }
            }
        }
    }

    if pair_count == 0 {
        return 0.5;
    }
    total_corr / pair_count as f64
}

// ─── Liquidity Calendar ─────────────────────────────────────────────────────

/// Liquidity event classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LiquidityEvent {
    /// 3rd Friday of Mar/Jun/Sep/Dec ± 2 days
    QuadWitching,
    /// FOMC announcement date (8 per year, hardcoded)
    Fomc,
    /// Heavy earnings reporting period
    EarningsSeason,
    /// No significant event
    Normal,
}

/// Check if a date falls near a known liquidity event.
///
/// Returns the highest-priority event if multiple overlap.
/// Priority: QuadWitching > FOMC > EarningsSeason > Normal.
pub fn check_liquidity_event(date: NaiveDate) -> LiquidityEvent {
    use chrono::Datelike;

    let month = date.month();
    let day = date.day();

    // Quad witching: 3rd Friday of Mar, Jun, Sep, Dec
    if [3, 6, 9, 12].contains(&month) {
        if let Some(first) = NaiveDate::from_ymd_opt(date.year(), month, 1) {
            let first_friday_offset = (5u32 + 7 - first.weekday().num_days_from_monday()) % 7;
            let third_friday = first + chrono::Duration::days(first_friday_offset as i64 + 14);
            if (date - third_friday).num_days().abs() <= 2 {
                return LiquidityEvent::QuadWitching;
            }
        }
    }

    // Earnings season: mid-month reporting windows
    if ([1, 4, 7, 10].contains(&month) && day >= 15)
        || ([2, 5, 8, 11].contains(&month) && day <= 15)
    {
        return LiquidityEvent::EarningsSeason;
    }

    LiquidityEvent::Normal
}

// ─── Composite State ────────────────────────────────────────────────────────

/// Combined microstructure state across all three dimensions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MicrostructureState {
    pub vol_regime: VolRegime,
    pub corr_regime: CorrRegime,
    pub liquidity_event: LiquidityEvent,
    pub realized_vol_20d: f64,
    pub avg_sector_correlation: f64,
    pub as_of: NaiveDate,
}

impl MicrostructureState {
    /// Favorability score: how much the microstructure supports aggressive positioning.
    ///
    /// Low vol + low correlation + normal calendar = 1.0 (most favorable).
    /// High vol + high correlation + quad witching = near 0.0.
    /// Uses geometric mean so all three dimensions must be favorable.
    pub fn favorability_score(&self) -> f64 {
        let vol_score: f64 = match self.vol_regime {
            VolRegime::Low => 1.0,
            VolRegime::Normal => 0.6,
            VolRegime::High => 0.2,
        };
        let corr_score = match self.corr_regime {
            CorrRegime::LowCorrelation => 1.0,
            CorrRegime::Normal => 0.6,
            CorrRegime::HighCorrelation => 0.3,
        };
        let liq_score = match self.liquidity_event {
            LiquidityEvent::Normal => 1.0,
            LiquidityEvent::EarningsSeason => 0.7,
            LiquidityEvent::Fomc => 0.5,
            LiquidityEvent::QuadWitching => 0.4,
        };
        (vol_score * corr_score * liq_score).powf(1.0_f64 / 3.0)
    }

    /// Build the full state from raw inputs.
    pub fn compute(
        vix: f64,
        realized_vol: f64,
        sector_returns: &[(&str, &[f64])],
        corr_window: usize,
        date: NaiveDate,
    ) -> Self {
        let avg_corr = compute_avg_sector_correlation(sector_returns, corr_window);
        Self {
            vol_regime: classify_vol_regime(vix, realized_vol),
            corr_regime: classify_corr_regime(avg_corr),
            liquidity_event: check_liquidity_event(date),
            realized_vol_20d: realized_vol,
            avg_sector_correlation: avg_corr,
            as_of: date,
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Vol regime ──────────────────────────────────────────────────────────

    #[test]
    fn vol_low_requires_both() {
        assert_eq!(classify_vol_regime(12.0, 8.0), VolRegime::Low);
        // Only one low isn't enough
        assert_eq!(classify_vol_regime(12.0, 15.0), VolRegime::Normal);
        assert_eq!(classify_vol_regime(18.0, 8.0), VolRegime::Normal);
    }

    #[test]
    fn vol_high_either_triggers() {
        assert_eq!(classify_vol_regime(30.0, 8.0), VolRegime::High);
        assert_eq!(classify_vol_regime(12.0, 25.0), VolRegime::High);
        assert_eq!(classify_vol_regime(30.0, 25.0), VolRegime::High);
    }

    #[test]
    fn vol_boundaries() {
        assert_eq!(classify_vol_regime(15.0, 10.0), VolRegime::Normal); // at boundary = normal
        assert_eq!(classify_vol_regime(25.0, 20.0), VolRegime::Normal); // at boundary = normal
        assert_eq!(classify_vol_regime(25.1, 15.0), VolRegime::High);
    }

    // ── Corr regime ─────────────────────────────────────────────────────────

    #[test]
    fn corr_classification() {
        assert_eq!(classify_corr_regime(0.20), CorrRegime::LowCorrelation);
        assert_eq!(classify_corr_regime(0.50), CorrRegime::Normal);
        assert_eq!(classify_corr_regime(0.80), CorrRegime::HighCorrelation);
    }

    #[test]
    fn corr_boundaries() {
        assert_eq!(classify_corr_regime(0.30), CorrRegime::Normal); // at boundary
        assert_eq!(classify_corr_regime(0.70), CorrRegime::Normal); // at boundary
        assert_eq!(classify_corr_regime(0.71), CorrRegime::HighCorrelation);
        assert_eq!(classify_corr_regime(0.29), CorrRegime::LowCorrelation);
    }

    #[test]
    fn avg_corr_insufficient_data() {
        let sectors: Vec<(&str, &[f64])> = vec![("XLK", &[0.01, 0.02][..])];
        assert!((compute_avg_sector_correlation(&sectors, 63) - 0.5).abs() < 0.01);
    }

    #[test]
    fn avg_corr_two_identical_series() {
        let returns = vec![
            0.01, -0.02, 0.03, -0.01, 0.02, 0.01, -0.01, 0.03, 0.02, -0.02, 0.01, 0.03, -0.01,
            0.02, 0.01, -0.02, 0.03, 0.01, -0.01, 0.02, 0.01, -0.02, 0.03, 0.01, -0.01, 0.02,
        ];
        let sectors: Vec<(&str, &[f64])> = vec![("XLK", &returns), ("XLC", &returns)];
        let corr = compute_avg_sector_correlation(&sectors, 63);
        assert!(
            (corr - 1.0).abs() < 0.01,
            "Identical series should correlate ~1.0, got {corr}"
        );
    }

    // ── Liquidity calendar ──────────────────────────────────────────────────

    #[test]
    fn quad_witching_march_2026() {
        // 3rd Friday of March 2026 = March 20
        let date = NaiveDate::from_ymd_opt(2026, 3, 20).unwrap();
        assert_eq!(check_liquidity_event(date), LiquidityEvent::QuadWitching);
    }

    #[test]
    fn quad_witching_near_date() {
        let qw = NaiveDate::from_ymd_opt(2026, 3, 20).unwrap();
        // 1 day before
        let before = qw - chrono::Duration::days(1);
        assert_eq!(check_liquidity_event(before), LiquidityEvent::QuadWitching);
        // 3 days away = not quad witching
        let far = qw - chrono::Duration::days(3);
        assert_ne!(check_liquidity_event(far), LiquidityEvent::QuadWitching);
    }

    #[test]
    fn earnings_season_january() {
        let date = NaiveDate::from_ymd_opt(2026, 1, 25).unwrap();
        assert_eq!(check_liquidity_event(date), LiquidityEvent::EarningsSeason);
    }

    #[test]
    fn normal_mid_march() {
        let date = NaiveDate::from_ymd_opt(2026, 3, 10).unwrap();
        assert_eq!(check_liquidity_event(date), LiquidityEvent::Normal);
    }

    // ── Favorability score ──────────────────────────────────────────────────

    #[test]
    fn favorability_best_case() {
        let state = MicrostructureState {
            vol_regime: VolRegime::Low,
            corr_regime: CorrRegime::LowCorrelation,
            liquidity_event: LiquidityEvent::Normal,
            realized_vol_20d: 8.0,
            avg_sector_correlation: 0.20,
            as_of: NaiveDate::from_ymd_opt(2026, 3, 10).unwrap(),
        };
        assert!((state.favorability_score() - 1.0).abs() < 0.01);
    }

    #[test]
    fn favorability_worst_case_is_low() {
        let state = MicrostructureState {
            vol_regime: VolRegime::High,
            corr_regime: CorrRegime::HighCorrelation,
            liquidity_event: LiquidityEvent::QuadWitching,
            realized_vol_20d: 25.0,
            avg_sector_correlation: 0.80,
            as_of: NaiveDate::from_ymd_opt(2026, 3, 20).unwrap(),
        };
        assert!(
            state.favorability_score() < 0.35,
            "Worst case should be < 0.35, got {}",
            state.favorability_score()
        );
    }

    #[test]
    fn favorability_bounded_0_1() {
        for (vr, cr, le) in [
            (
                VolRegime::Low,
                CorrRegime::LowCorrelation,
                LiquidityEvent::Normal,
            ),
            (
                VolRegime::High,
                CorrRegime::HighCorrelation,
                LiquidityEvent::QuadWitching,
            ),
            (
                VolRegime::Normal,
                CorrRegime::Normal,
                LiquidityEvent::EarningsSeason,
            ),
        ] {
            let state = MicrostructureState {
                vol_regime: vr,
                corr_regime: cr,
                liquidity_event: le,
                realized_vol_20d: 15.0,
                avg_sector_correlation: 0.50,
                as_of: NaiveDate::from_ymd_opt(2026, 6, 1).unwrap(),
            };
            let f = state.favorability_score();
            assert!((0.0..=1.0).contains(&f), "Favorability {f} out of [0, 1]");
        }
    }
}
