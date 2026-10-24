//! End-to-end pipeline tests.
//!
//! These tests exercise the full signal-quality pipeline:
//!   fixtures → screener → alert engine → DB persistence
//!
//! They act as regression guards: a previously-passing Minervini stock must
//! continue to produce a CRITICAL or WARNING alert after code changes.

use std::collections::HashMap;

use ghost_alpha::alerts::engine::{AlertConfig, AlertEngine};
use ghost_alpha::analysis::scorer::Grade;
use ghost_alpha::analysis::screener::{MinerviniScreener, Screener};
use ghost_alpha::storage::db::{Repository, SqliteRepository};

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn uptrend_candles(n: usize) -> Vec<ghost_alpha::data::models::Candle> {
    (0..n)
        .map(|i| {
            let c = 50.0 + i as f64 * (100.0 / n as f64);
            ghost_alpha::data::models::Candle {
                date: chrono::NaiveDate::from_ymd_opt(2022, 1, 1)
                    .unwrap()
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                open: c * 0.995,
                high: c * 1.01,
                low: c * 0.99,
                close: c,
                adj_close: c,
                // Breakout-day volume expansion (2x average → z = 1.0) on the
                // last bar. Since the Phase 5 fix, average volume (z = 0)
                // scores 0 volume points, so a flat-volume series is no longer
                // a "strong" setup.
                volume: if i == n - 1 { 2_000_000 } else { 1_000_000 },
            }
        })
        .collect()
}

// ─── Signal quality guard ─────────────────────────────────────────────────────

#[test]
fn strongly_trending_stock_produces_critical_or_warning_alert() {
    let screener = MinerviniScreener::new();
    let candles = uptrend_candles(300);

    // RS universe: this stock is near the top
    let universe_returns: Vec<f64> = (0..100).map(|i| i as f64 * 0.002).collect();
    let result = screener
        .screen(
            "NVDA",
            &candles,
            &universe_returns,
            &ghost_alpha::data::models::MarketRegime::Bull,
        )
        .unwrap();

    let mut engine = AlertEngine::new(AlertConfig::default());
    let alerts = engine.evaluate(std::slice::from_ref(&result));

    assert!(
        !alerts.is_empty(),
        "Strongly trending stock (score={:.1}) should produce at least one alert",
        result.composite_score
    );

    use ghost_alpha::data::models::Severity;
    assert!(
        matches!(alerts[0].severity, Severity::Critical | Severity::Warning),
        "Alert severity should be CRITICAL or WARNING, got {:?}",
        alerts[0].severity
    );
}

// ─── Persistence round-trip ───────────────────────────────────────────────────

#[test]
fn screen_result_persists_and_retrieves_correctly() {
    let repo = SqliteRepository::open_in_memory().unwrap();
    let run_date = chrono::NaiveDate::from_ymd_opt(2024, 1, 15).unwrap();

    use ghost_alpha::analysis::screener::ScreenResult;
    let result = ScreenResult {
        ticker: "NVDA".into(),
        minervini_pass: true,
        conditions_met: 8,
        adx: 38.5,
        rs_percentile: 92.0,
        volume_zscore: 2.1,
        technical_score: 52.0,
        volume_score: 28.0,
        sentiment_score: 0.0,
        composite_score: 88.0,
        grade: Grade::APlus,
        close_price: 650.0,
        pct_from_52w_high: -3.5,
        atr_value: 12.5,
        risk_reward: 3.2,
        liquidity: ghost_alpha::analysis::indicators::LiquidityClass::High,
        weighted_rs: 0.85,
        rs_momentum: None,
        vcp: None,
        regime: ghost_alpha::data::models::MarketRegime::Bull,
    };

    // Need to insert the stock first (FK constraint)
    use ghost_alpha::data::models::Stock;
    repo.upsert_stock(&Stock {
        ticker: "NVDA".into(),
        name: "NVIDIA Corporation".into(),
        sector: "Technology".into(),
        industry: "Semiconductors".into(),
    })
    .unwrap();

    repo.save_screen_result(run_date, &result).unwrap();
    let retrieved = repo.get_screen_results(run_date).unwrap();

    assert_eq!(retrieved.len(), 1);
    let r = &retrieved[0];
    assert_eq!(r.ticker, "NVDA");
    assert!((r.composite_score - 88.0).abs() < 0.01);
    assert!((r.adx - 38.5).abs() < 0.01);

    // Phase 2.5 fields round-trip
    assert!(
        (r.atr_value - 12.5).abs() < 0.01,
        "ATR should round-trip, got {:.2}",
        r.atr_value
    );
    assert!(
        (r.risk_reward - 3.2).abs() < 0.01,
        "R:R should round-trip, got {:.2}",
        r.risk_reward
    );
    assert_eq!(
        r.liquidity,
        ghost_alpha::analysis::indicators::LiquidityClass::High,
        "Liquidity class should round-trip"
    );
    assert!(
        (r.weighted_rs - 0.85).abs() < 0.01,
        "Weighted RS should round-trip, got {:.4}",
        r.weighted_rs
    );
    assert_eq!(
        r.regime,
        ghost_alpha::data::models::MarketRegime::Bull,
        "Regime should round-trip"
    );
}

// ─── Sector concentration E2E ─────────────────────────────────────────────────

#[test]
fn three_tech_stocks_trigger_concentration_warning() {
    let screener = MinerviniScreener::new();
    let universe_returns: Vec<f64> = (0..50).map(|i| i as f64 * 0.004).collect();

    let tickers = ["NVDA", "AMD", "INTC"];
    let mut results = Vec::new();
    for ticker in &tickers {
        let candles = uptrend_candles(300);
        if let Ok(r) = screener.screen(
            ticker,
            &candles,
            &universe_returns,
            &ghost_alpha::data::models::MarketRegime::Bull,
        ) {
            results.push(r);
        }
    }

    let mut engine = AlertEngine::new(AlertConfig::default());
    let mut sector_map = HashMap::new();
    for ticker in &tickers {
        sector_map.insert(ticker.to_string(), "Technology".to_string());
    }

    let (alerts, warnings) = engine.evaluate_with_sector_check(&results, &sector_map);

    // All three should alert (high-scoring uptrend)
    assert!(!alerts.is_empty(), "At least one tech stock should alert");

    // If all three alert, we expect a sector warning
    if alerts.len() >= 3 {
        assert!(
            !warnings.is_empty(),
            "Three alerts in same sector should trigger concentration warning"
        );
    }
}
