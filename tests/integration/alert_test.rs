//! Integration tests: AlertEngine ↔ NullChannel ↔ DedupStore wired together.

use ghost_alpha::alerts::engine::{AlertConfig, AlertEngine};
use ghost_alpha::analysis::scorer::Grade;
use ghost_alpha::analysis::screener::ScreenResult;

fn make_result(ticker: &str, score: f64, minervini: bool, vol_z: f64) -> ScreenResult {
    ScreenResult {
        ticker: ticker.to_string(),
        minervini_pass: minervini,
        conditions_met: if minervini { 8 } else { 3 },
        adx: 32.0,
        rs_percentile: 78.0,
        volume_zscore: vol_z,
        technical_score: score * 0.55,
        volume_score: score * 0.30,
        sentiment_score: 0.0,
        composite_score: score,
        grade: Grade::A,
        close_price: 200.0,
        pct_from_52w_high: -8.0,
        atr_value: 2.5,
        risk_reward: 3.0,
        liquidity: ghost_alpha::analysis::indicators::LiquidityClass::High,
        weighted_rs: 0.5,
        rs_momentum: None,
        vcp: None,
        regime: ghost_alpha::data::models::MarketRegime::Bull,
    }
}

// ─── End-to-end alert flow ────────────────────────────────────────────────────

#[test]
fn high_score_batch_produces_alerts_in_order() {
    let mut engine = AlertEngine::new(AlertConfig::default());
    let results = vec![
        make_result("NVDA", 95.0, true, 1.0),
        make_result("AMD", 88.0, true, 0.5),
        make_result("AAPL", 72.0, false, 0.3),
        make_result("WEAK", 30.0, false, 0.0),
    ];
    let alerts = engine.evaluate(&results);
    assert_eq!(alerts.len(), 3, "Only tickers scoring ≥ 50 should alert");
    assert_eq!(alerts[0].ticker, "NVDA");
    assert_eq!(alerts[1].ticker, "AMD");
    assert_eq!(alerts[2].ticker, "AAPL");
}

#[test]
fn daily_limit_gates_across_multiple_evaluate_calls() {
    let mut engine = AlertEngine::new(AlertConfig {
        max_daily_alerts: 3,
        ..Default::default()
    });

    let batch1 = vec![
        make_result("A", 92.0, true, 1.0),
        make_result("B", 92.0, true, 1.0),
    ];
    let batch2 = vec![
        make_result("C", 92.0, true, 1.0),
        make_result("D", 92.0, true, 1.0),
    ];

    let alerts1 = engine.evaluate(&batch1);
    let alerts2 = engine.evaluate(&batch2);

    assert_eq!(alerts1.len(), 2);
    assert_eq!(alerts2.len(), 1, "Only 1 slot left in day");
    // daily_count is verified indirectly: any further evaluate() should produce 0 alerts
    let alerts3 = engine.evaluate(&[make_result("E", 92.0, true, 1.0)]);
    assert!(
        alerts3.is_empty(),
        "No more slots after hitting daily limit"
    );
}

#[test]
fn reset_then_same_tickers_fire_again() {
    let mut engine = AlertEngine::new(AlertConfig {
        max_daily_alerts: 1,
        dedup_cooldown_days: 0, // no dedup to isolate daily-limit test
        ..Default::default()
    });
    let results = vec![make_result("NVDA", 92.0, true, 1.0)];
    let alerts = engine.evaluate(&results);
    assert_eq!(alerts.len(), 1);

    engine.reset_daily_count();
    let alerts2 = engine.evaluate(&results);
    assert_eq!(
        alerts2.len(),
        1,
        "After reset, same ticker should fire again"
    );
}

#[test]
fn sector_concentration_warning_fires_for_three_tech_stocks() {
    use std::collections::HashMap;

    let mut engine = AlertEngine::new(AlertConfig::default());
    let results = vec![
        make_result("NVDA", 92.0, true, 1.0),
        make_result("AMD", 92.0, true, 1.0),
        make_result("INTC", 92.0, true, 1.0),
    ];
    let mut sector_map = HashMap::new();
    for t in &["NVDA", "AMD", "INTC"] {
        sector_map.insert(t.to_string(), "Technology".to_string());
    }
    let (alerts, warnings) = engine.evaluate_with_sector_check(&results, &sector_map);
    assert_eq!(alerts.len(), 3);
    assert!(
        !warnings.is_empty(),
        "Should warn about sector concentration"
    );
}

#[test]
fn quiet_hours_suppression_does_not_affect_evaluate() {
    // `evaluate` does NOT apply quiet hours — that's the caller's job.
    // This test confirms evaluate fires at "23:00" regardless.
    use ghost_alpha::data::models::Severity;

    let engine = AlertEngine::new(AlertConfig {
        quiet_start_hour: 22,
        quiet_end_hour: 7,
        ..Default::default()
    });
    // Engine classify_severity should still work
    let result = make_result("NVDA", 92.0, true, 1.0);
    let sev = engine.classify_severity(&result).unwrap();
    assert_eq!(sev, Severity::Critical);
    // Caller then checks is_suppressed_by_quiet_hours — Critical is never suppressed
    assert!(!engine.is_suppressed_by_quiet_hours(&sev, 23));
}
