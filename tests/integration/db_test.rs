//! Integration tests for `SqliteRepository` against a real in-memory SQLite database.

use crate::common::fixtures;
use ghost_alpha::data::models::{MarketRegime, RegimeState, Stock};
use ghost_alpha::storage::db::{Repository, SqliteRepository};

fn repo() -> SqliteRepository {
    SqliteRepository::open_in_memory().expect("in-memory SQLite")
}

// ─── Round-trip tests ─────────────────────────────────────────────────────────

#[test]
fn candle_round_trip_preserves_all_fields() {
    let repo = repo();
    let stock = Stock {
        ticker: "AAPL".into(),
        name: "Apple Inc.".into(),
        sector: "Technology".into(),
        industry: "Consumer Electronics".into(),
    };
    repo.upsert_stock(&stock).unwrap();

    let candles = fixtures::minervini_pass_candles();
    let first_close = candles[0].close;
    let last_close = candles.last().unwrap().close;

    repo.upsert_candles("AAPL", &candles).unwrap();
    let retrieved = repo.get_candles("AAPL", 300).unwrap();

    assert_eq!(retrieved.len(), 300);
    assert!((retrieved[0].close - first_close).abs() < 0.01);
    assert!((retrieved.last().unwrap().close - last_close).abs() < 0.01);
}

#[test]
fn candle_upsert_is_idempotent() {
    let repo = repo();
    let stock = Stock {
        ticker: "NVDA".into(),
        name: "NVIDIA".into(),
        sector: "Technology".into(),
        industry: "Semiconductors".into(),
    };
    repo.upsert_stock(&stock).unwrap();

    let candles = fixtures::uniform_volume_candles();
    repo.upsert_candles("NVDA", &candles).unwrap();
    repo.upsert_candles("NVDA", &candles).unwrap(); // second upsert
    let retrieved = repo.get_candles("NVDA", 200).unwrap();
    assert_eq!(
        retrieved.len(),
        60,
        "Idempotent upsert should not duplicate rows"
    );
}

#[test]
fn regime_overwrites_same_date() {
    let repo = repo();
    let date = chrono::NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();

    let state1 = RegimeState {
        date,
        spy_close: 480.0,
        spy_sma200: 440.0,
        breadth_pct: 65.0,
        regime: MarketRegime::Bull,
    };
    let state2 = RegimeState {
        date,
        spy_close: 475.0,
        spy_sma200: 440.0,
        breadth_pct: 48.0,
        regime: MarketRegime::Sideways,
    };
    repo.save_regime(&state1).unwrap();
    repo.save_regime(&state2).unwrap();

    let latest = repo.latest_regime().unwrap().unwrap();
    assert_eq!(
        latest.regime,
        MarketRegime::Sideways,
        "Second upsert should overwrite"
    );
    assert!((latest.breadth_pct - 48.0).abs() < 0.01);
}

#[test]
fn multiple_stocks_sorted_alphabetically() {
    let repo = repo();
    for ticker in &["NVDA", "AAPL", "AMD", "MSFT"] {
        repo.upsert_stock(&Stock {
            ticker: (*ticker).into(),
            name: format!("{} Inc.", ticker),
            sector: "Technology".into(),
            industry: "Semiconductors".into(),
        })
        .unwrap();
    }
    let stocks = repo.get_stocks().unwrap();
    assert_eq!(stocks.len(), 4);
    assert_eq!(stocks[0].ticker, "AAPL");
    assert_eq!(stocks[3].ticker, "NVDA");
}
