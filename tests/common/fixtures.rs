/// Shared test data builders for integration and E2E tests.
///
/// These produce synthetic candle series that satisfy specific
/// Minervini conditions so tests are deterministic and readable.
use ghost_alpha::data::models::Candle;

// ─── Candle builders ──────────────────────────────────────────────────────────

/// Build `n` candles with a linearly interpolated close from `start` → `end`.
pub fn trending_candles(n: usize, start: f64, end: f64) -> Vec<Candle> {
    (0..n)
        .map(|i| {
            let t = if n > 1 {
                i as f64 / (n - 1) as f64
            } else {
                0.0
            };
            let c = start + t * (end - start);
            Candle {
                date: chrono::NaiveDate::from_ymd_opt(2022, 1, 1)
                    .unwrap()
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                open: c * 0.995,
                high: c * 1.01,
                low: c * 0.99,
                close: c,
                adj_close: c,
                volume: 1_000_000,
            }
        })
        .collect()
}

/// 300-bar strongly uptrending series (passes all 8 Minervini conditions).
pub fn minervini_pass_candles() -> Vec<Candle> {
    trending_candles(300, 50.0, 150.0)
}

/// 300-bar declining series (fails most Minervini conditions).
pub fn minervini_fail_candles() -> Vec<Candle> {
    trending_candles(300, 150.0, 50.0)
}

/// 60-bar flat series with a large volume spike on the last bar.
pub fn volume_spike_candles() -> Vec<Candle> {
    let mut candles = trending_candles(60, 100.0, 100.0);
    let last = candles.len() - 1;
    candles[last].volume = 10_000_000; // 10x spike
    candles[last].close = 101.0; // up-close
    candles[last].open = 100.0;
    candles
}

/// 60-bar series with perfectly uniform volume (useful for baseline tests).
pub fn uniform_volume_candles() -> Vec<Candle> {
    trending_candles(60, 100.0, 100.0) // all volumes = 1_000_000
}

/// 300-bar SPY series above its 200-day MA — Bull regime.
pub fn spy_bull_candles() -> Vec<Candle> {
    trending_candles(300, 350.0, 500.0)
}

/// 300-bar SPY series falling below its 200-day MA — Bear regime.
pub fn spy_bear_candles() -> Vec<Candle> {
    trending_candles(300, 500.0, 300.0)
}

/// 120-bar VCP (Volatility Contraction Pattern) with 3 contracting pullbacks.
pub fn vcp_candles() -> Vec<Candle> {
    let base_date = chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap();
    (0..120)
        .map(|i| {
            let base = 100.0 + (i as f64 * 0.2); // slow upward drift
            let phase = i % 40;
            let cycle = i / 40; // 0, 1, 2
                                // Each cycle contracts: amplitude shrinks with each successive base
            let amplitude = match cycle {
                0 => 12.0,
                1 => 7.0,
                _ => 3.0,
            };
            let wave = (phase as f64 / 40.0 * std::f64::consts::PI * 2.0).sin() * amplitude;
            let c = base + wave;
            Candle {
                date: base_date
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                open: c * 0.998,
                high: c * 1.005,
                low: c * 0.995,
                close: c,
                adj_close: c,
                volume: 2_000_000,
            }
        })
        .collect()
}

/// 300-bar high-liquidity series (~$300M daily dollar volume).
pub fn high_liquidity_candles() -> Vec<Candle> {
    let base_date = chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap();
    (0..300)
        .map(|i| {
            let c = 150.0 + (i as f64 * 0.3);
            Candle {
                date: base_date
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                open: c * 0.998,
                high: c * 1.01,
                low: c * 0.99,
                close: c,
                adj_close: c,
                volume: 2_000_000, // 2M shares × ~$150 = ~$300M
            }
        })
        .collect()
}

/// Build a minimal `AppConfig` string (TOML) for integration tests.
pub fn test_config_toml(db_path: &str) -> String {
    format!(
        r#"
[finnhub]
api_key = "test-key"

[storage]
db_path = "{db_path}"
"#
    )
}
