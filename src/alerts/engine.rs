use std::collections::HashMap;

use crate::alerts::channel::OutboundAlert;
use crate::alerts::dedup::{DedupKey, DedupStore, InMemoryDedupStore};
use crate::analysis::screener::ScreenResult;
use crate::data::models::Severity;

// ─── Alert Config ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AlertConfig {
    /// Composite score threshold for CRITICAL alerts (unless Minervini+volume rule fires)
    pub critical_score_threshold: f64,
    /// Composite score threshold for WARNING alerts
    pub warning_score_threshold: f64,
    /// Composite score threshold for INFO alerts
    pub info_score_threshold: f64,
    /// Volume Z-score that, combined with Minervini pass, triggers CRITICAL
    pub volume_spike_threshold: f64,
    /// Deduplication cooldown in days
    pub dedup_cooldown_days: i64,
    /// Maximum alerts per day (0 = unlimited)
    pub max_daily_alerts: usize,
    /// Quiet hours: start hour (24h, local time)
    pub quiet_start_hour: u32,
    /// Quiet hours: end hour (24h, local time)
    pub quiet_end_hour: u32,
    /// Minimum stocks in same GICS sector to trigger concentration warning
    pub sector_concentration_threshold: usize,
}

impl Default for AlertConfig {
    fn default() -> Self {
        Self {
            critical_score_threshold: 90.0,
            warning_score_threshold: 70.0,
            info_score_threshold: 50.0,
            volume_spike_threshold: 2.0,
            dedup_cooldown_days: 7,
            max_daily_alerts: 50,
            quiet_start_hour: 22,
            quiet_end_hour: 7,
            sector_concentration_threshold: 3,
        }
    }
}

/// Convert from the config file representation to the engine's alert config.
/// Both structs have identical fields — this avoids manual field-by-field mapping.
impl From<&crate::config::AlertsConfig> for AlertConfig {
    fn from(cfg: &crate::config::AlertsConfig) -> Self {
        Self {
            critical_score_threshold: cfg.critical_score_threshold,
            warning_score_threshold: cfg.warning_score_threshold,
            info_score_threshold: cfg.info_score_threshold,
            volume_spike_threshold: cfg.volume_spike_threshold,
            dedup_cooldown_days: cfg.dedup_cooldown_days,
            max_daily_alerts: cfg.max_daily_alerts,
            quiet_start_hour: cfg.quiet_start_hour,
            quiet_end_hour: cfg.quiet_end_hour,
            sector_concentration_threshold: cfg.sector_concentration_threshold,
        }
    }
}

// ─── Alert Engine ─────────────────────────────────────────────────────────────

pub struct AlertEngine {
    config: AlertConfig,
    dedup: InMemoryDedupStore,
    daily_count: usize,
}

impl AlertEngine {
    pub fn new(config: AlertConfig) -> Self {
        Self {
            config,
            dedup: InMemoryDedupStore::new(),
            daily_count: 0,
        }
    }

    /// Determine severity for a screen result based on score and special rules.
    pub fn classify_severity(&self, result: &ScreenResult) -> Option<Severity> {
        let is_critical_rule =
            result.minervini_pass && result.volume_zscore >= self.config.volume_spike_threshold;

        if result.composite_score >= self.config.critical_score_threshold || is_critical_rule {
            Some(Severity::Critical)
        } else if result.composite_score >= self.config.warning_score_threshold {
            Some(Severity::Warning)
        } else if result.composite_score >= self.config.info_score_threshold {
            Some(Severity::Info)
        } else {
            None
        }
    }

    /// Return true if `severity` should be suppressed during quiet hours at `hour_of_day`.
    /// Critical alerts always bypass quiet hours.
    pub fn is_suppressed_by_quiet_hours(&self, severity: &Severity, hour_of_day: u32) -> bool {
        if *severity == Severity::Critical {
            return false;
        }
        let start = self.config.quiet_start_hour;
        let end = self.config.quiet_end_hour;
        if start > end {
            // Wraps midnight: e.g. 22–7
            hour_of_day >= start || hour_of_day < end
        } else {
            hour_of_day >= start && hour_of_day < end
        }
    }

    /// Evaluate a batch of screen results and return alerts that should fire.
    ///
    /// Applies deduplication and daily limit. Does NOT apply quiet hours
    /// (that is the caller's responsibility when routing to the channel).
    pub fn evaluate(&mut self, results: &[ScreenResult]) -> Vec<OutboundAlert> {
        let mut alerts = Vec::new();

        for result in results {
            if self.config.max_daily_alerts > 0 && self.daily_count >= self.config.max_daily_alerts
            {
                break;
            }

            let Some(severity) = self.classify_severity(result) else {
                continue;
            };

            let key = DedupKey::new(&result.ticker, "NewSignal");
            if self
                .dedup
                .is_duplicate(&key, self.config.dedup_cooldown_days)
            {
                continue;
            }

            self.dedup.record_sent(&key);
            self.daily_count += 1;

            alerts.push(OutboundAlert {
                ticker: result.ticker.clone(),
                alert_type: "NewSignal".into(),
                severity,
                message: format!(
                    "{}: score={:.1}, conditions={}/8, ADX={:.1}, RS={:.1}%, ATR={:.2}, R:R={:.1}, liq={}",
                    result.ticker,
                    result.composite_score,
                    result.conditions_met,
                    result.adx,
                    result.rs_percentile,
                    result.atr_value,
                    result.risk_reward,
                    result.liquidity,
                ),
                composite_score: result.composite_score,
                narrative: None,
            });
        }

        alerts
    }

    /// Like `evaluate`, but also returns sector concentration warnings.
    pub fn evaluate_with_sector_check(
        &mut self,
        results: &[ScreenResult],
        sector_map: &HashMap<String, String>,
    ) -> (Vec<OutboundAlert>, Vec<String>) {
        let alerts = self.evaluate(results);

        // Count sectors among fired alerts
        let mut sector_counts: HashMap<&str, usize> = HashMap::new();
        for alert in &alerts {
            if let Some(sector) = sector_map.get(&alert.ticker) {
                *sector_counts.entry(sector.as_str()).or_insert(0) += 1;
            }
        }

        let warnings: Vec<String> = sector_counts
            .into_iter()
            .filter(|(_, count)| *count >= self.config.sector_concentration_threshold)
            .map(|(sector, count)| {
                format!(
                    "⚠️ sector concentration: {} alerts in sector '{}' — diversify!",
                    count, sector
                )
            })
            .collect();

        (alerts, warnings)
    }

    /// Reset daily counter (call at midnight or start of new screening run).
    pub fn reset_daily_count(&mut self) {
        self.daily_count = 0;
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(ticker: &str, score: f64, minervini: bool, vol_z: f64) -> ScreenResult {
        ScreenResult {
            ticker: ticker.to_string(),
            minervini_pass: minervini,
            conditions_met: if minervini { 8 } else { 4 },
            adx: 35.0,
            rs_percentile: 80.0,
            volume_zscore: vol_z,
            technical_score: score * 0.55,
            volume_score: score * 0.30,
            sentiment_score: 0.0,
            composite_score: score,
            grade: crate::analysis::scorer::Grade::A,
            close_price: 100.0,
            pct_from_52w_high: -5.0,
            atr_value: 2.5,
            risk_reward: 3.0,
            liquidity: crate::analysis::indicators::LiquidityClass::High,
            weighted_rs: 0.5,
            rs_momentum: None,
            vcp: None,
            regime: crate::data::models::MarketRegime::Bull,
        }
    }

    // ── classify_severity ────────────────────────────────────────────────────

    #[test]
    fn score_90_plus_is_critical() {
        let engine = AlertEngine::new(AlertConfig::default());
        let result = make_result("NVDA", 92.0, true, 1.0);
        assert_eq!(engine.classify_severity(&result), Some(Severity::Critical));
    }

    #[test]
    fn score_70_to_89_is_warning() {
        let engine = AlertEngine::new(AlertConfig::default());
        let result = make_result("AMD", 75.0, false, 0.5);
        assert_eq!(engine.classify_severity(&result), Some(Severity::Warning));
    }

    #[test]
    fn score_50_to_69_is_info() {
        let engine = AlertEngine::new(AlertConfig::default());
        let result = make_result("INTC", 55.0, false, 0.0);
        assert_eq!(engine.classify_severity(&result), Some(Severity::Info));
    }

    #[test]
    fn score_below_50_no_alert() {
        let engine = AlertEngine::new(AlertConfig::default());
        let result = make_result("WEAK", 45.0, false, 0.0);
        assert_eq!(engine.classify_severity(&result), None);
    }

    #[test]
    fn minervini_plus_volume_spike_is_critical_regardless_of_score() {
        let engine = AlertEngine::new(AlertConfig::default());
        // score=72 would normally be WARNING, but Minervini+spike → CRITICAL
        let result = make_result("AAPL", 72.0, true, 3.0);
        assert_eq!(engine.classify_severity(&result), Some(Severity::Critical));
    }

    #[test]
    fn minervini_without_volume_spike_uses_score_only() {
        let engine = AlertEngine::new(AlertConfig::default());
        let result = make_result("AAPL", 72.0, true, 1.0); // vol_z < 2.0
        assert_eq!(engine.classify_severity(&result), Some(Severity::Warning));
    }

    // ── quiet hours ──────────────────────────────────────────────────────────

    #[test]
    fn quiet_hours_suppress_warning_at_23h() {
        let engine = AlertEngine::new(AlertConfig {
            quiet_start_hour: 22,
            quiet_end_hour: 7,
            ..Default::default()
        });
        assert!(engine.is_suppressed_by_quiet_hours(&Severity::Warning, 23));
    }

    #[test]
    fn quiet_hours_suppress_info_at_3h() {
        let engine = AlertEngine::new(AlertConfig {
            quiet_start_hour: 22,
            quiet_end_hour: 7,
            ..Default::default()
        });
        assert!(engine.is_suppressed_by_quiet_hours(&Severity::Info, 3));
    }

    #[test]
    fn quiet_hours_allow_critical_at_23h() {
        let engine = AlertEngine::new(AlertConfig {
            quiet_start_hour: 22,
            quiet_end_hour: 7,
            ..Default::default()
        });
        assert!(!engine.is_suppressed_by_quiet_hours(&Severity::Critical, 23));
    }

    #[test]
    fn quiet_hours_allow_warning_during_day() {
        let engine = AlertEngine::new(AlertConfig::default());
        assert!(!engine.is_suppressed_by_quiet_hours(&Severity::Warning, 14));
    }

    // ── evaluate ─────────────────────────────────────────────────────────────

    #[test]
    fn evaluate_high_score_generates_alert() {
        let mut engine = AlertEngine::new(AlertConfig::default());
        let results = vec![make_result("NVDA", 92.0, true, 1.0)];
        let alerts = engine.evaluate(&results);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].ticker, "NVDA");
        assert_eq!(alerts[0].severity, Severity::Critical);
    }

    #[test]
    fn evaluate_low_score_no_alert() {
        let mut engine = AlertEngine::new(AlertConfig::default());
        let results = vec![make_result("WEAK", 30.0, false, 0.0)];
        let alerts = engine.evaluate(&results);
        assert!(alerts.is_empty());
    }

    #[test]
    fn evaluate_deduplicates_same_ticker() {
        let mut engine = AlertEngine::new(AlertConfig::default());
        let results = vec![
            make_result("NVDA", 92.0, true, 1.0),
            make_result("NVDA", 92.0, true, 1.0), // duplicate
        ];
        let alerts = engine.evaluate(&results);
        assert_eq!(
            alerts.len(),
            1,
            "Duplicate ticker should produce only one alert"
        );
    }

    #[test]
    fn evaluate_different_tickers_both_fire() {
        let mut engine = AlertEngine::new(AlertConfig::default());
        let results = vec![
            make_result("NVDA", 92.0, true, 1.0),
            make_result("AMD", 85.0, true, 0.5),
        ];
        let alerts = engine.evaluate(&results);
        assert_eq!(alerts.len(), 2);
    }

    #[test]
    fn evaluate_respects_daily_limit() {
        let mut engine = AlertEngine::new(AlertConfig {
            max_daily_alerts: 2,
            ..Default::default()
        });
        let results: Vec<ScreenResult> = ["A", "B", "C", "D"]
            .iter()
            .map(|t| make_result(t, 92.0, true, 1.0))
            .collect();
        let alerts = engine.evaluate(&results);
        assert_eq!(alerts.len(), 2, "Should cap at daily limit of 2");
    }

    #[test]
    fn evaluate_zero_daily_limit_is_unlimited() {
        let mut engine = AlertEngine::new(AlertConfig {
            max_daily_alerts: 0,
            ..Default::default()
        });
        let results: Vec<ScreenResult> = ["A", "B", "C", "D", "E"]
            .iter()
            .map(|t| make_result(t, 92.0, true, 1.0))
            .collect();
        let alerts = engine.evaluate(&results);
        assert_eq!(alerts.len(), 5);
    }

    // ── sector concentration ─────────────────────────────────────────────────

    #[test]
    fn three_same_sector_triggers_concentration_warning() {
        let mut engine = AlertEngine::new(AlertConfig::default());
        let results = vec![
            make_result("NVDA", 92.0, true, 1.0),
            make_result("AMD", 92.0, true, 1.0),
            make_result("INTC", 92.0, true, 1.0),
        ];
        let mut sector_map = HashMap::new();
        for ticker in &["NVDA", "AMD", "INTC"] {
            sector_map.insert(ticker.to_string(), "Technology".to_string());
        }
        let (alerts, warnings) = engine.evaluate_with_sector_check(&results, &sector_map);
        assert_eq!(alerts.len(), 3);
        assert!(
            !warnings.is_empty(),
            "Expected sector concentration warning, got none"
        );
        assert!(
            warnings[0].contains("sector concentration"),
            "Warning should mention sector concentration"
        );
    }

    #[test]
    fn two_same_sector_no_concentration_warning() {
        let mut engine = AlertEngine::new(AlertConfig::default());
        let results = vec![
            make_result("NVDA", 92.0, true, 1.0),
            make_result("AMD", 92.0, true, 1.0),
        ];
        let mut sector_map = HashMap::new();
        sector_map.insert("NVDA".to_string(), "Technology".to_string());
        sector_map.insert("AMD".to_string(), "Technology".to_string());
        let (_, warnings) = engine.evaluate_with_sector_check(&results, &sector_map);
        assert!(
            warnings.is_empty(),
            "2 alerts should not trigger concentration warning"
        );
    }

    // ── reset_daily_count ────────────────────────────────────────────────────

    // ── alert message format ────────────────────────────────────────────────

    #[test]
    fn alert_message_contains_atr() {
        let mut engine = AlertEngine::new(AlertConfig::default());
        let results = vec![make_result("NVDA", 92.0, true, 1.0)];
        let alerts = engine.evaluate(&results);
        assert!(
            alerts[0].message.contains("ATR="),
            "Alert message should contain ATR value"
        );
    }

    #[test]
    fn alert_message_contains_risk_reward() {
        let mut engine = AlertEngine::new(AlertConfig::default());
        let results = vec![make_result("NVDA", 92.0, true, 1.0)];
        let alerts = engine.evaluate(&results);
        assert!(
            alerts[0].message.contains("R:R="),
            "Alert message should contain R:R ratio"
        );
    }

    #[test]
    fn alert_message_contains_liquidity_class() {
        let mut engine = AlertEngine::new(AlertConfig::default());
        let results = vec![make_result("NVDA", 92.0, true, 1.0)];
        let alerts = engine.evaluate(&results);
        assert!(
            alerts[0].message.contains("liq=High"),
            "Alert message should contain liquidity class"
        );
    }

    #[test]
    fn evaluate_empty_results_returns_empty_vec() {
        let mut engine = AlertEngine::new(AlertConfig::default());
        let alerts = engine.evaluate(&[]);
        assert!(alerts.is_empty(), "Empty input should produce no alerts");
    }

    #[test]
    fn quiet_hours_non_wrapping_range() {
        // Non-wrapping range: start=9, end=17 → hours [9, 17) are suppressed
        let engine = AlertEngine::new(AlertConfig {
            quiet_start_hour: 9,
            quiet_end_hour: 17,
            ..Default::default()
        });
        // Hour 12 is inside [9, 17) → Warning suppressed
        assert!(
            engine.is_suppressed_by_quiet_hours(&Severity::Warning, 12),
            "Hour 12 should be suppressed in non-wrapping range 9–17"
        );
        // Hour 8 is outside [9, 17) → Warning not suppressed
        assert!(
            !engine.is_suppressed_by_quiet_hours(&Severity::Warning, 8),
            "Hour 8 should NOT be suppressed in non-wrapping range 9–17"
        );
    }

    // ── reset_daily_count ────────────────────────────────────────────────────

    #[test]
    fn reset_daily_count_allows_more_alerts() {
        let mut engine = AlertEngine::new(AlertConfig {
            max_daily_alerts: 1,
            ..Default::default()
        });
        let r1 = make_result("NVDA", 92.0, true, 1.0);
        engine.evaluate(&[r1]);
        assert_eq!(engine.daily_count, 1);
        engine.reset_daily_count();
        assert_eq!(engine.daily_count, 0);

        // A new ticker should fire after reset
        let r2 = make_result("AMD", 92.0, true, 1.0);
        let alerts = engine.evaluate(&[r2]);
        assert_eq!(alerts.len(), 1);
    }
}
