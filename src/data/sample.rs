//! Bundled sample data for the `demo` subcommand.
//!
//! Generates deterministic synthetic OHLCV data for ~50 tickers × 300 trading
//! days without any external dependencies or compressed binary blobs.
//! All data is derived from a seeded LCG so the output is reproducible.

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::NaiveDate;

use crate::data::models::{Candle, NewsArticle, Stock};
use crate::data::provider::DataProvider;
use crate::error::DataError;

// ─── Ticker Universe ─────────────────────────────────────────────────────────

/// 50 representative tickers used in demo mode.
/// Mix of large-caps, growth, and sector leaders — plausible for a Minervini screen.
pub const DEMO_TICKERS: &[&str] = &[
    "AAPL", "MSFT", "NVDA", "AMZN", "GOOGL", "META", "TSLA", "AVGO", "ORCL", "AMD", "CRM", "ADBE",
    "PANW", "CDNS", "SNPS", "KLAC", "LRCX", "AMAT", "TXN", "MU", "PLTR", "CRWD", "NET", "DDOG",
    "ZS", "APP", "FTNT", "OKTA", "SNOW", "MDB", "MELI", "SE", "SHOP", "SQ", "PYPL", "COIN", "RBLX",
    "UBER", "LYFT", "ABNB", "LLY", "NVO", "REGN", "ISRG", "DXCM", "GEHC", "HCA", "MOH", "UNH",
    "CVS",
];

/// SPY — used for regime detection in demo mode.
const SPY_TICKER: &str = "SPY";

// ─── Deterministic LCG ───────────────────────────────────────────────────────

/// Very small LCG (Lehmer generator). No external deps, no `rand` crate.
/// Period = 2^31 − 1. Good enough for generating plausible-looking OHLCV.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed % 0x7fff_ffff + 1)
    }

    /// Returns next value in [0, 1).
    fn next_f64(&mut self) -> f64 {
        self.0 = (self.0.wrapping_mul(16807)) % 0x7fff_ffff;
        self.0 as f64 / 0x7fff_ffff as f64
    }

    /// Returns a value in [low, high].
    fn range(&mut self, low: f64, high: f64) -> f64 {
        low + self.next_f64() * (high - low)
    }

    /// Returns a roughly normal variate via Box-Muller (no libm needed).
    fn normal(&mut self, mean: f64, std: f64) -> f64 {
        // Box-Muller: need two uniform samples
        let u1 = self.next_f64().max(1e-12);
        let u2 = self.next_f64();
        // ln approximated via a few Newton steps from u1 (avoids libm dependency)
        // Actually we can just use a simple sum of 12 uniforms ≈ N(6, 1) − 6
        // but Box-Muller is cleaner. Rust std does have f64::ln.
        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
        mean + std * z
    }
}

// ─── Price history generator ─────────────────────────────────────────────────

/// Generate `n_days` candles for one ticker starting from `base_price`.
///
/// The price follows a geometric random walk with a slight upward bias
/// and realistic intra-bar spread. Volume follows a log-normal distribution
/// around a base volume with occasional spikes.
fn generate_candles(
    ticker: &str,
    base_price: f64,
    n_days: usize,
    anchor_date: NaiveDate,
    trend_bias: f64, // daily drift in %; positive = trending up
) -> Vec<Candle> {
    // Seed from ticker hash so each ticker is independent but reproducible
    let seed: u64 = ticker.bytes().enumerate().fold(0u64, |acc, (i, b)| {
        acc.wrapping_add((b as u64).wrapping_mul(i as u64 + 31))
    });
    let mut rng = Lcg::new(seed.max(1));

    let mut price = base_price;
    let daily_vol = 0.018; // ~1.8% daily vol — realistic for mid/large cap
    let base_vol: f64 = rng.range(1_000_000.0, 50_000_000.0);

    let mut candles = Vec::with_capacity(n_days);

    // Approximate trading-day calendar: skip Sat/Sun
    let mut date = anchor_date;
    let mut days_added = 0usize;

    while days_added < n_days {
        // Skip weekends
        use chrono::Datelike;
        let weekday = date.weekday();
        if weekday == chrono::Weekday::Sat || weekday == chrono::Weekday::Sun {
            date = date.succ_opt().unwrap_or(date);
            continue;
        }

        // Daily return: geometric random walk + bias
        let ret = rng.normal(trend_bias / 100.0, daily_vol);
        let close = (price * (1.0 + ret)).max(1.0);

        // Intraday spread
        let spread_pct = rng.range(0.003, 0.025);
        let high = close * (1.0 + spread_pct * rng.range(0.3, 1.0));
        let low = close * (1.0 - spread_pct * rng.range(0.3, 1.0));
        let open = price * (1.0 + rng.normal(0.0, 0.005));
        let open = open.clamp(low, high);

        // Volume: log-normal with occasional spikes
        let vol_mult = if rng.next_f64() > 0.92 {
            rng.range(2.0, 5.0) // ~8% chance of a volume spike
        } else {
            rng.range(0.5, 1.8)
        };
        let volume = (base_vol * vol_mult) as u64;

        candles.push(Candle {
            date,
            open: round2(open),
            high: round2(high),
            low: round2(low),
            close: round2(close),
            adj_close: round2(close),
            volume,
        });

        price = close;
        date = date.succ_opt().unwrap_or(date);
        days_added += 1;
    }

    candles
}

#[inline]
fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

// ─── Sample data table ───────────────────────────────────────────────────────

/// (ticker, base_price, trend_bias_pct_per_day)
///
/// Prices are loosely inspired by ballpark levels (not current quotes).
/// Trend biases are crafted so a healthy mix of tickers pass/fail Minervini.
const TICKER_PARAMS: &[(&str, f64, f64)] = &[
    ("AAPL", 178.0, 0.04),
    ("MSFT", 415.0, 0.05),
    ("NVDA", 860.0, 0.10),
    ("AMZN", 195.0, 0.06),
    ("GOOGL", 175.0, 0.04),
    ("META", 510.0, 0.07),
    ("TSLA", 185.0, 0.03),
    ("AVGO", 170.0, 0.08),
    ("ORCL", 120.0, 0.05),
    ("AMD", 180.0, 0.09),
    ("CRM", 295.0, 0.04),
    ("ADBE", 460.0, 0.03),
    ("PANW", 310.0, 0.06),
    ("CDNS", 270.0, 0.05),
    ("SNPS", 580.0, 0.04),
    ("KLAC", 735.0, 0.05),
    ("LRCX", 930.0, 0.04),
    ("AMAT", 215.0, 0.06),
    ("TXN", 175.0, 0.02),
    ("MU", 105.0, 0.05),
    ("PLTR", 25.0, 0.08),
    ("CRWD", 360.0, 0.07),
    ("NET", 90.0, 0.06),
    ("DDOG", 125.0, 0.07),
    ("ZS", 215.0, 0.05),
    ("APP", 75.0, 0.09),
    ("FTNT", 75.0, 0.04),
    ("OKTA", 110.0, 0.03),
    ("SNOW", 165.0, 0.04),
    ("MDB", 395.0, 0.05),
    ("MELI", 1850.0, 0.05),
    ("SE", 80.0, 0.04),
    ("SHOP", 75.0, 0.05),
    ("SQ", 80.0, 0.03),
    ("PYPL", 65.0, 0.02),
    ("COIN", 190.0, 0.06),
    ("RBLX", 45.0, 0.04),
    ("UBER", 80.0, 0.05),
    ("LYFT", 18.0, 0.03),
    ("ABNB", 145.0, 0.04),
    ("LLY", 790.0, 0.06),
    ("NVO", 130.0, 0.05),
    ("REGN", 910.0, 0.03),
    ("ISRG", 445.0, 0.04),
    ("DXCM", 105.0, 0.04),
    ("GEHC", 90.0, 0.03),
    ("HCA", 330.0, 0.03),
    ("MOH", 330.0, 0.02),
    ("UNH", 510.0, 0.02),
    ("CVS", 65.0, 0.01),
];

/// SPY: roughly 500 price, mild uptrend (regime → Bull)
const SPY_BASE: f64 = 490.0;
const SPY_TREND: f64 = 0.03;

/// Snapshot anchor: 300 trading days before 2026-03-01
const ANCHOR_DATE: &str = "2025-01-15";
const N_DAYS: usize = 300;

// ─── SampleProvider ──────────────────────────────────────────────────────────

/// An in-memory data provider backed by deterministic synthetic OHLCV data.
///
/// Used exclusively by `ghost-alpha demo` — no API key, no network, no config required.
pub struct SampleProvider {
    candles: HashMap<String, Vec<Candle>>,
}

impl SampleProvider {
    /// Build the provider, generating all sample candles in memory.
    ///
    /// Generation is fast (<5ms for 50 tickers × 300 days) and fully deterministic.
    pub fn new() -> Self {
        let anchor = NaiveDate::parse_from_str(ANCHOR_DATE, "%Y-%m-%d")
            .expect("hard-coded anchor date must be valid");

        let mut candles: HashMap<String, Vec<Candle>> = HashMap::new();

        // Stock tickers
        for &(ticker, base, trend) in TICKER_PARAMS {
            candles.insert(
                ticker.to_string(),
                generate_candles(ticker, base, N_DAYS, anchor, trend),
            );
        }

        // SPY
        candles.insert(
            SPY_TICKER.to_string(),
            generate_candles(SPY_TICKER, SPY_BASE, N_DAYS, anchor, SPY_TREND),
        );

        Self { candles }
    }

    /// Number of tickers in the sample universe (excluding SPY).
    pub fn ticker_count(&self) -> usize {
        self.candles.len().saturating_sub(1) // minus SPY
    }

    /// Total candle count across all tickers (including SPY).
    pub fn total_candles(&self) -> usize {
        self.candles.values().map(|v| v.len()).sum()
    }
}

impl Default for SampleProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DataProvider for SampleProvider {
    async fn fetch_candles(
        &self,
        ticker: &str,
        _from: NaiveDate,
        _to: NaiveDate,
    ) -> Result<Vec<Candle>, DataError> {
        self.candles
            .get(ticker)
            .cloned()
            .ok_or_else(|| DataError::NotFound {
                ticker: ticker.to_string(),
            })
    }

    async fn fetch_stocks(&self, tickers: &[&str]) -> Result<Vec<Stock>, DataError> {
        let stocks = tickers
            .iter()
            .map(|t| Stock {
                ticker: t.to_string(),
                name: t.to_string(),
                sector: "Technology".to_string(),
                industry: "Software".to_string(),
            })
            .collect();
        Ok(stocks)
    }

    async fn fetch_news(
        &self,
        ticker: &str,
        _from_date: NaiveDate,
        _limit: usize,
    ) -> Result<Vec<NewsArticle>, DataError> {
        // Return a few synthetic headlines so sentiment demo path is exercised
        let published_at = chrono::Utc::now();
        let articles = vec![
            NewsArticle {
                ticker: ticker.to_string(),
                headline: format!("{ticker} posts strong quarterly results, raises guidance"),
                summary: "Earnings beat on both top and bottom line.".to_string(),
                published_at,
                source: "Demo".to_string(),
            },
            NewsArticle {
                ticker: ticker.to_string(),
                headline: format!("{ticker} expands into new markets with strategic partnership"),
                summary: "Deal expected to add significant revenue in fiscal year.".to_string(),
                published_at,
                source: "Demo".to_string(),
            },
            NewsArticle {
                ticker: ticker.to_string(),
                headline: format!(
                    "Analysts raise price target for {ticker} after positive outlook"
                ),
                summary: "Multiple upgrades following the earnings call.".to_string(),
                published_at,
                source: "Demo".to_string(),
            },
        ];
        Ok(articles)
    }

    async fn fetch_spy_candles(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Candle>, DataError> {
        self.fetch_candles(SPY_TICKER, from, to).await
    }

    async fn fetch_universe(&self) -> Result<Vec<String>, DataError> {
        Ok(DEMO_TICKERS.iter().map(|s| s.to_string()).collect())
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_provider_has_correct_ticker_count() {
        let provider = SampleProvider::new();
        // 50 tickers + SPY = 51 entries in the map
        assert_eq!(provider.candles.len(), DEMO_TICKERS.len() + 1);
    }

    #[test]
    fn each_ticker_has_roughly_n_days_candles() {
        let provider = SampleProvider::new();
        for ticker in DEMO_TICKERS {
            let candles = provider.candles.get(*ticker).expect("ticker must exist");
            // Trading calendar skips weekends; 300 trading days ≈ 300 entries
            // (we count trading days, not calendar days)
            assert!(
                candles.len() >= 280 && candles.len() <= 320,
                "ticker {ticker} has {} candles, expected ~300",
                candles.len()
            );
        }
    }

    #[test]
    fn spy_candles_are_present() {
        let provider = SampleProvider::new();
        assert!(provider.candles.contains_key("SPY"));
        assert!(provider.candles["SPY"].len() >= 280);
    }

    #[test]
    fn candles_are_ordered_chronologically() {
        let provider = SampleProvider::new();
        let candles = &provider.candles["NVDA"];
        for w in candles.windows(2) {
            assert!(w[0].date < w[1].date, "dates must be strictly ascending");
        }
    }

    #[test]
    fn candles_have_valid_ohlcv() {
        let provider = SampleProvider::new();
        for ticker in DEMO_TICKERS {
            let candles = provider.candles.get(*ticker).unwrap();
            for c in candles {
                assert!(c.high >= c.low, "{ticker}: high < low on {}", c.date);
                assert!(
                    c.open > 0.0 && c.close > 0.0,
                    "{ticker}: non-positive price"
                );
                assert!(c.volume > 0, "{ticker}: zero volume on {}", c.date);
                assert!(
                    c.open >= c.low * 0.999 && c.open <= c.high * 1.001,
                    "{ticker}: open {:.2} outside [low={:.2}, high={:.2}] on {}",
                    c.open,
                    c.low,
                    c.high,
                    c.date
                );
            }
        }
    }

    #[test]
    fn generation_is_deterministic() {
        let p1 = SampleProvider::new();
        let p2 = SampleProvider::new();
        let c1 = &p1.candles["NVDA"];
        let c2 = &p2.candles["NVDA"];
        assert_eq!(c1.len(), c2.len());
        for (a, b) in c1.iter().zip(c2.iter()) {
            assert_eq!(a.date, b.date);
            assert!(
                (a.close - b.close).abs() < 1e-9,
                "generation is not deterministic"
            );
        }
    }

    #[test]
    fn ticker_count_helper_excludes_spy() {
        let provider = SampleProvider::new();
        assert_eq!(provider.ticker_count(), DEMO_TICKERS.len());
    }

    #[test]
    fn total_candles_includes_all_tickers() {
        let provider = SampleProvider::new();
        let expected_min = (DEMO_TICKERS.len() + 1) * 280; // +1 for SPY
        assert!(provider.total_candles() >= expected_min);
    }

    #[tokio::test]
    async fn fetch_universe_returns_demo_tickers() {
        let provider = SampleProvider::new();
        let universe = provider.fetch_universe().await.unwrap();
        assert_eq!(universe.len(), DEMO_TICKERS.len());
        assert!(universe.contains(&"NVDA".to_string()));
        assert!(universe.contains(&"AAPL".to_string()));
    }

    #[tokio::test]
    async fn fetch_candles_returns_data_for_known_ticker() {
        let provider = SampleProvider::new();
        let from = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let candles = provider.fetch_candles("AAPL", from, to).await.unwrap();
        assert!(!candles.is_empty());
    }

    #[tokio::test]
    async fn fetch_candles_returns_error_for_unknown_ticker() {
        let provider = SampleProvider::new();
        let from = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let result = provider.fetch_candles("UNKNOWN_XYZ", from, to).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fetch_spy_candles_returns_data() {
        let provider = SampleProvider::new();
        let from = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
        let candles = provider.fetch_spy_candles(from, to).await.unwrap();
        assert!(candles.len() >= 280);
    }

    #[tokio::test]
    async fn fetch_news_returns_synthetic_headlines() {
        let provider = SampleProvider::new();
        let from = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let articles = provider.fetch_news("NVDA", from, 10).await.unwrap();
        assert_eq!(articles.len(), 3);
        assert!(articles.iter().all(|a| a.ticker == "NVDA"));
    }
}
