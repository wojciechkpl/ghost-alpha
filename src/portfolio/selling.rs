use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

// ─── Selling Programme ──────────────────────────────────────────────────────

/// A multi-year selling programme defining when and how much to sell.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SellingProgramme {
    pub id: Option<i64>,
    pub ticker: String,
    pub schedule: Vec<SellingPeriod>,
    pub pace_rules: Vec<PaceRule>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SellingPeriod {
    pub year: u32,
    pub quarter: u8, // 1-4, or 0 for annual
    pub shares_target: f64,
    pub dollar_target: Option<f64>,
    pub executed_shares: f64,
    pub executed_proceeds: f64,
}

/// Rules that accelerate or decelerate the selling pace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaceRule {
    pub condition: PaceCondition,
    pub multiplier: f64, // 2.0 = double, 0.5 = halve
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PaceCondition {
    /// Accelerate if stock breaks below 200-SMA.
    Below200Sma,
    /// Accelerate if market regime is Bear.
    BearRegime,
    /// Decelerate if stock is in strong uptrend (buy more time for LTCG).
    Stage2Uptrend,
    /// Accelerate if price > target (take profits).
    PriceAbove(f64),
    /// Accelerate if composite geopolitical risk score >= 6.
    GeoRiskHigh,
    /// Accelerate if oil price > threshold ($/barrel).
    OilAbove(f64),
    /// Emergency: accelerate if stock drops > N% in a day.
    StockDropPercent(f64),
}

/// All market conditions needed for pace rule evaluation.
#[derive(Debug, Clone, Default)]
pub struct MarketContext {
    pub regime: Option<String>,
    pub current_price: f64,
    pub is_above_200sma: bool,
    pub geo_risk_score: Option<f64>,
    pub oil_price: Option<f64>,
    pub daily_return_pct: Option<f64>,
}

/// A recorded execution against a selling programme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SellingExecution {
    pub id: Option<i64>,
    pub programme_id: i64,
    pub execution_date: NaiveDate,
    pub shares_sold: f64,
    pub price: f64,
    pub proceeds: f64,
    pub lot_ids: Vec<String>,
    pub notes: Option<String>,
}

// ─── Methods ────────────────────────────────────────────────────────────────

impl SellingProgramme {
    /// Sum of all period share targets.
    pub fn total_target_shares(&self) -> f64 {
        self.schedule.iter().map(|p| p.shares_target).sum()
    }

    /// Sum of shares already executed across all periods.
    pub fn total_executed_shares(&self) -> f64 {
        self.schedule.iter().map(|p| p.executed_shares).sum()
    }

    /// Shares remaining to be sold.
    pub fn remaining_shares(&self) -> f64 {
        self.total_target_shares() - self.total_executed_shares()
    }

    /// Completion percentage (0.0–100.0). Returns 0.0 when no targets exist.
    pub fn progress_pct(&self) -> f64 {
        let total = self.total_target_shares();
        if total <= 0.0 {
            return 0.0;
        }
        (self.total_executed_shares() / total) * 100.0
    }

    /// Evaluate all pace rules against current market conditions and return
    /// the combined multiplier (product of all matching rules). Returns 1.0
    /// when no rules match.
    pub fn apply_pace_rules(
        &self,
        regime: Option<&str>,
        current_price: f64,
        is_above_200sma: bool,
    ) -> f64 {
        let ctx = MarketContext {
            regime: regime.map(|s| s.to_string()),
            current_price,
            is_above_200sma,
            ..Default::default()
        };
        self.apply_pace_rules_ctx(&ctx)
    }

    /// Evaluate all pace rules using the full market context.
    ///
    /// The combined multiplier is clamped to [0.25, 4.0] to prevent pathological
    /// compounding in crash scenarios (e.g., Bear × Below200SMA × GeoRisk × StockDrop
    /// could otherwise produce 18x acceleration).
    pub fn apply_pace_rules_ctx(&self, ctx: &MarketContext) -> f64 {
        let mut multiplier = 1.0;
        for rule in &self.pace_rules {
            let matches = match &rule.condition {
                PaceCondition::Below200Sma => !ctx.is_above_200sma,
                PaceCondition::BearRegime => ctx.regime.as_deref() == Some("Bear"),
                PaceCondition::Stage2Uptrend => ctx.is_above_200sma,
                PaceCondition::PriceAbove(threshold) => ctx.current_price > *threshold,
                PaceCondition::GeoRiskHigh => ctx.geo_risk_score.is_some_and(|s| s >= 6.0),
                PaceCondition::OilAbove(threshold) => ctx.oil_price.is_some_and(|p| p > *threshold),
                PaceCondition::StockDropPercent(threshold) => {
                    ctx.daily_return_pct.is_some_and(|r| r < -*threshold)
                }
            };
            if matches {
                multiplier *= rule.multiplier;
            }
        }
        multiplier.clamp(0.25, 4.0)
    }

    /// Convenience constructor that creates quarterly periods from annual share
    /// targets. Each year's target is split evenly into four quarters.
    pub fn build_schedule(ticker: &str, annual_shares: &[f64], years: u32) -> Self {
        let mut schedule = Vec::with_capacity((years as usize) * 4);
        for year_idx in 0..years {
            let annual = annual_shares.get(year_idx as usize).copied().unwrap_or(0.0);
            let per_quarter = annual / 4.0;
            for q in 1..=4u8 {
                schedule.push(SellingPeriod {
                    year: year_idx + 1,
                    quarter: q,
                    shares_target: per_quarter,
                    dollar_target: None,
                    executed_shares: 0.0,
                    executed_proceeds: 0.0,
                });
            }
        }
        Self {
            id: None,
            ticker: ticker.to_string(),
            schedule,
            pace_rules: Vec::new(),
            created_at: None,
        }
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_period(year: u32, quarter: u8, target: f64, executed: f64) -> SellingPeriod {
        SellingPeriod {
            year,
            quarter,
            shares_target: target,
            dollar_target: None,
            executed_shares: executed,
            executed_proceeds: executed * 150.0, // arbitrary price
        }
    }

    fn make_programme(periods: Vec<SellingPeriod>, rules: Vec<PaceRule>) -> SellingProgramme {
        SellingProgramme {
            id: None,
            ticker: "AMZN".to_string(),
            schedule: periods,
            pace_rules: rules,
            created_at: None,
        }
    }

    #[test]
    fn test_total_target_shares() {
        let prog = make_programme(
            vec![
                make_period(1, 1, 25.0, 0.0),
                make_period(1, 2, 25.0, 0.0),
                make_period(1, 3, 25.0, 0.0),
                make_period(1, 4, 25.0, 0.0),
            ],
            vec![],
        );
        assert!((prog.total_target_shares() - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_total_executed_shares() {
        let prog = make_programme(
            vec![
                make_period(1, 1, 25.0, 25.0),
                make_period(1, 2, 25.0, 10.0),
                make_period(1, 3, 25.0, 0.0),
                make_period(1, 4, 25.0, 15.0),
            ],
            vec![],
        );
        assert!((prog.total_executed_shares() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_remaining_shares() {
        let prog = make_programme(
            vec![
                make_period(1, 1, 25.0, 25.0),
                make_period(1, 2, 25.0, 10.0),
                make_period(1, 3, 25.0, 0.0),
                make_period(1, 4, 25.0, 15.0),
            ],
            vec![],
        );
        // total=100, executed=50 => remaining=50
        assert!((prog.remaining_shares() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_progress_pct() {
        let prog = make_programme(
            vec![make_period(1, 1, 50.0, 25.0), make_period(1, 2, 50.0, 25.0)],
            vec![],
        );
        // 50 of 100 => 50%
        assert!((prog.progress_pct() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_progress_pct_empty() {
        let prog = make_programme(vec![], vec![]);
        assert!((prog.progress_pct()).abs() < f64::EPSILON);
    }

    #[test]
    fn test_apply_pace_bear_regime() {
        let prog = make_programme(
            vec![],
            vec![PaceRule {
                condition: PaceCondition::BearRegime,
                multiplier: 2.0,
            }],
        );
        let m = prog.apply_pace_rules(Some("Bear"), 100.0, true);
        assert!((m - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_apply_pace_no_match() {
        let prog = make_programme(
            vec![],
            vec![PaceRule {
                condition: PaceCondition::BearRegime,
                multiplier: 2.0,
            }],
        );
        // regime is Bull, not Bear => no match
        let m = prog.apply_pace_rules(Some("Bull"), 100.0, true);
        assert!((m - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_apply_pace_multiple_rules() {
        let prog = make_programme(
            vec![],
            vec![
                PaceRule {
                    condition: PaceCondition::BearRegime,
                    multiplier: 2.0,
                },
                PaceRule {
                    condition: PaceCondition::Below200Sma,
                    multiplier: 1.5,
                },
            ],
        );
        // Bear regime + below 200 SMA (is_above_200sma=false) => 2.0 * 1.5 = 3.0
        let m = prog.apply_pace_rules(Some("Bear"), 100.0, false);
        assert!((m - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_apply_pace_stage2_uptrend() {
        let prog = make_programme(
            vec![],
            vec![PaceRule {
                condition: PaceCondition::Stage2Uptrend,
                multiplier: 0.5,
            }],
        );
        // is_above_200sma=true => Stage2Uptrend matches
        let m = prog.apply_pace_rules(None, 100.0, true);
        assert!((m - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_apply_pace_price_above() {
        let prog = make_programme(
            vec![],
            vec![PaceRule {
                condition: PaceCondition::PriceAbove(200.0),
                multiplier: 1.5,
            }],
        );
        // current_price=220 > 200 => matches
        let m = prog.apply_pace_rules(None, 220.0, true);
        assert!((m - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_build_schedule_creates_quarters() {
        let prog = SellingProgramme::build_schedule("AMZN", &[100.0, 200.0], 2);
        assert_eq!(prog.ticker, "AMZN");
        assert_eq!(prog.schedule.len(), 8);
        // Year 1: 100/4 = 25 per quarter
        for p in &prog.schedule[0..4] {
            assert_eq!(p.year, 1);
            assert!((p.shares_target - 25.0).abs() < f64::EPSILON);
            assert!((p.executed_shares).abs() < f64::EPSILON);
        }
        // Year 2: 200/4 = 50 per quarter
        for p in &prog.schedule[4..8] {
            assert_eq!(p.year, 2);
            assert!((p.shares_target - 50.0).abs() < f64::EPSILON);
        }
        // Quarters cycle 1-4
        assert_eq!(prog.schedule[0].quarter, 1);
        assert_eq!(prog.schedule[1].quarter, 2);
        assert_eq!(prog.schedule[2].quarter, 3);
        assert_eq!(prog.schedule[3].quarter, 4);
    }

    #[test]
    fn test_selling_period_serde_roundtrip() {
        let period = SellingPeriod {
            year: 2025,
            quarter: 3,
            shares_target: 50.0,
            dollar_target: Some(10_000.0),
            executed_shares: 12.5,
            executed_proceeds: 2500.0,
        };
        let json = serde_json::to_string(&period).expect("serialize");
        let back: SellingPeriod = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.year, 2025);
        assert_eq!(back.quarter, 3);
        assert!((back.shares_target - 50.0).abs() < f64::EPSILON);
        assert!((back.dollar_target.unwrap() - 10_000.0).abs() < f64::EPSILON);
        assert!((back.executed_shares - 12.5).abs() < f64::EPSILON);
        assert!((back.executed_proceeds - 2500.0).abs() < f64::EPSILON);
    }

    // ── New pace conditions (Item 6) ─────────────────────────────────

    #[test]
    fn test_apply_pace_geo_risk_high() {
        let prog = make_programme(
            vec![],
            vec![PaceRule {
                condition: PaceCondition::GeoRiskHigh,
                multiplier: 2.0,
            }],
        );
        let ctx = MarketContext {
            geo_risk_score: Some(7.0),
            ..Default::default()
        };
        assert!((prog.apply_pace_rules_ctx(&ctx) - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_apply_pace_geo_no_match() {
        let prog = make_programme(
            vec![],
            vec![PaceRule {
                condition: PaceCondition::GeoRiskHigh,
                multiplier: 2.0,
            }],
        );
        let ctx = MarketContext {
            geo_risk_score: Some(3.0),
            ..Default::default()
        };
        assert!((prog.apply_pace_rules_ctx(&ctx) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_apply_pace_oil_above() {
        let prog = make_programme(
            vec![],
            vec![PaceRule {
                condition: PaceCondition::OilAbove(120.0),
                multiplier: 1.5,
            }],
        );
        let ctx = MarketContext {
            oil_price: Some(130.0),
            ..Default::default()
        };
        assert!((prog.apply_pace_rules_ctx(&ctx) - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_apply_pace_stock_drop() {
        let prog = make_programme(
            vec![],
            vec![PaceRule {
                condition: PaceCondition::StockDropPercent(5.0),
                multiplier: 3.0,
            }],
        );
        let ctx = MarketContext {
            daily_return_pct: Some(-7.0),
            ..Default::default()
        };
        assert!((prog.apply_pace_rules_ctx(&ctx) - 3.0).abs() < f64::EPSILON);
    }

    // ── Phase 5: Pace cap tests ─────────────────────────────────────────────

    #[test]
    fn test_pace_multiplier_capped_at_4x() {
        // Bear(2.0) × Below200Sma(1.5) × GeoRisk(2.0) × StockDrop(3.0) = 18.0
        // Should be capped at 4.0
        let prog = make_programme(
            vec![],
            vec![
                PaceRule {
                    condition: PaceCondition::BearRegime,
                    multiplier: 2.0,
                },
                PaceRule {
                    condition: PaceCondition::Below200Sma,
                    multiplier: 1.5,
                },
                PaceRule {
                    condition: PaceCondition::GeoRiskHigh,
                    multiplier: 2.0,
                },
                PaceRule {
                    condition: PaceCondition::StockDropPercent(5.0),
                    multiplier: 3.0,
                },
            ],
        );
        let ctx = MarketContext {
            regime: Some("Bear".to_string()),
            is_above_200sma: false,
            geo_risk_score: Some(8.0),
            daily_return_pct: Some(-10.0),
            ..Default::default()
        };
        let result = prog.apply_pace_rules_ctx(&ctx);
        assert!(
            result <= 4.0,
            "Pace multiplier should be capped at 4.0, got {result}"
        );
        assert!((result - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_pace_multiplier_floored_at_025x() {
        // Stage2Uptrend(0.1) should be floored at 0.25
        let prog = make_programme(
            vec![],
            vec![PaceRule {
                condition: PaceCondition::Stage2Uptrend,
                multiplier: 0.1,
            }],
        );
        let ctx = MarketContext {
            is_above_200sma: true,
            ..Default::default()
        };
        let result = prog.apply_pace_rules_ctx(&ctx);
        assert!(
            (result - 0.25).abs() < f64::EPSILON,
            "Floor at 0.25, got {result}"
        );
    }

    #[test]
    fn test_pace_single_rule_within_bounds_unchanged() {
        // A single 2.0x multiplier should pass through unchanged
        let prog = make_programme(
            vec![],
            vec![PaceRule {
                condition: PaceCondition::BearRegime,
                multiplier: 2.0,
            }],
        );
        let ctx = MarketContext {
            regime: Some("Bear".to_string()),
            ..Default::default()
        };
        let result = prog.apply_pace_rules_ctx(&ctx);
        assert!((result - 2.0).abs() < f64::EPSILON);
    }
}
