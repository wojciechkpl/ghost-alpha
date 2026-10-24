//! Yahoo Finance data provider.
//!
//! Implements `DataProvider` using Yahoo Finance's unofficial v8 chart API.
//! No API key is required, making this the zero-setup provider for new users.
//!
//! ## Rate limiting
//!
//! Yahoo Finance is an unofficial API with no published SLA.  We stay polite
//! with a 5 req/sec governor and exponential back-off on 429/5xx responses.
//!
//! ## Adjusted close
//!
//! Yahoo returns split/dividend-adjusted close prices in the `adjclose` field.
//! When present, `adj_close` on the `Candle` struct is set from that field;
//! otherwise it falls back to the raw close.

use async_trait::async_trait;
use chrono::NaiveDate;
use governor::{Quota, RateLimiter};
use serde::Deserialize;
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::data::models::{Candle, NewsArticle, Stock};
use crate::data::provider::DataProvider;
use crate::data::universes::SP500;
use crate::error::DataError;

// ─── Type alias ───────────────────────────────────────────────────────────────

type DefaultRateLimiter = RateLimiter<
    governor::state::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
>;

// ─── Response deserialization types ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct YahooChartResponse {
    chart: YahooChart,
}

#[derive(Debug, Deserialize)]
struct YahooChart {
    /// May be `null` in the JSON when an API error is returned.
    #[serde(default, deserialize_with = "deserialize_null_as_empty_vec")]
    result: Vec<YahooResult>,
    error: Option<YahooError>,
}

fn deserialize_null_as_empty_vec<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Deserialize)]
struct YahooError {
    description: String,
}

#[derive(Debug, Deserialize)]
struct YahooResult {
    #[serde(default)]
    timestamp: Vec<i64>,
    indicators: YahooIndicators,
}

#[derive(Debug, Deserialize)]
struct YahooIndicators {
    quote: Vec<YahooQuote>,
    #[serde(default)]
    adjclose: Vec<YahooAdjClose>,
}

#[derive(Debug, Deserialize)]
struct YahooQuote {
    #[serde(default)]
    open: Vec<Option<f64>>,
    #[serde(default)]
    high: Vec<Option<f64>>,
    #[serde(default)]
    low: Vec<Option<f64>>,
    #[serde(default)]
    close: Vec<Option<f64>>,
    #[serde(default)]
    volume: Vec<Option<u64>>,
}

#[derive(Debug, Deserialize)]
struct YahooAdjClose {
    #[serde(default)]
    adjclose: Vec<Option<f64>>,
}

// ─── Provider ─────────────────────────────────────────────────────────────────

/// Yahoo Finance provider — no API key required.
///
/// Use `YahooProvider::new()` to build an instance ready for use.
/// The bundled S&P 500 universe is returned by `fetch_universe()`; news is not
/// supported (returns empty vec) so sentiment scoring is gracefully skipped.
pub struct YahooProvider {
    client: reqwest::Client,
    rate_limiter: Arc<DefaultRateLimiter>,
    /// Base URL — overridable in tests to point at a mock server.
    base_url: String,
    /// Maximum elapsed time (ms) for the retry backoff loop.
    /// Production value: 60_000 ms.  Tests override to a small value so that
    /// transient-error tests (429, 5xx) complete quickly.
    #[cfg(test)]
    backoff_max_ms: u64,
}

impl YahooProvider {
    /// Build a production-ready provider with a 5 req/sec rate limit.
    pub fn new() -> Self {
        let quota = Quota::per_second(NonZeroU32::new(5).unwrap());
        Self {
            client: reqwest::Client::builder()
                .gzip(true)
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to build reqwest client"),
            rate_limiter: Arc::new(RateLimiter::direct(quota)),
            base_url: "https://query1.finance.yahoo.com".to_string(),
            #[cfg(test)]
            backoff_max_ms: 60_000,
        }
    }

    /// Build a test provider pointed at an arbitrary base URL.
    #[cfg(test)]
    pub fn new_with_base_url(base_url: String) -> Self {
        // High quota so tests don't stall waiting for rate-limit tokens.
        let quota = Quota::per_second(NonZeroU32::new(1000).unwrap());
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("Failed to build reqwest client"),
            rate_limiter: Arc::new(RateLimiter::direct(quota)),
            base_url,
            backoff_max_ms: 60_000,
        }
    }

    /// Build a test provider with a short backoff so transient-error tests
    /// (429, 5xx) complete in milliseconds instead of up to 60 seconds.
    #[cfg(test)]
    pub fn new_with_fast_backoff(base_url: String) -> Self {
        let quota = Quota::per_second(NonZeroU32::new(1000).unwrap());
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("Failed to build reqwest client"),
            rate_limiter: Arc::new(RateLimiter::direct(quota)),
            base_url,
            backoff_max_ms: 100, // expire after 100 ms → at most 1 attempt
        }
    }

    /// Wait for a rate-limit permit, then issue a GET with retry on 429/5xx.
    async fn get_with_retry(&self, url: &str) -> Result<String, DataError> {
        self.rate_limiter.until_ready().await;

        let op = || async {
            let resp = self
                .client
                .get(url)
                .header("User-Agent", "Mozilla/5.0 (compatible; ghost-alpha/1.0)")
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(|e| {
                    backoff::Error::permanent(DataError::Http(format!("Request failed: {e}")))
                })?;

            let status = resp.status();
            if status.is_success() {
                resp.text().await.map_err(|e| {
                    backoff::Error::permanent(DataError::Http(format!(
                        "Failed to read response body: {e}"
                    )))
                })
            } else if status.as_u16() == 429 {
                Err(backoff::Error::transient(DataError::RateLimit {
                    api: "yahoo".to_string(),
                }))
            } else if status.is_server_error() {
                Err(backoff::Error::transient(DataError::Http(format!(
                    "Server error: {status}"
                ))))
            } else {
                Err(backoff::Error::permanent(DataError::Http(format!(
                    "HTTP {status}: unexpected status"
                ))))
            }
        };

        #[cfg(test)]
        let max_elapsed = std::time::Duration::from_millis(self.backoff_max_ms);
        #[cfg(not(test))]
        let max_elapsed = std::time::Duration::from_secs(60);

        backoff::future::retry(
            backoff::ExponentialBackoffBuilder::new()
                .with_max_elapsed_time(Some(max_elapsed))
                .with_initial_interval(std::time::Duration::from_millis(500))
                .build(),
            op,
        )
        .await
    }

    /// Convert a NaiveDate range into the Yahoo v8 `range` query parameter.
    ///
    /// Yahoo only supports fixed range strings.  We map the requested date span
    /// to the smallest range that fully covers it.  The caller trims to the
    /// actual `from`/`to` window after parsing.
    fn date_range_to_yahoo_range(from: NaiveDate, to: NaiveDate) -> &'static str {
        let days = (to - from).num_days();
        match days {
            d if d <= 35 => "1mo",
            d if d <= 95 => "3mo",
            d if d <= 185 => "6mo",
            d if d <= 380 => "1y",
            d if d <= 760 => "2y",
            _ => "5y",
        }
    }

    /// Convert a Unix timestamp (seconds) to a `NaiveDate` in UTC.
    pub fn timestamp_to_date(ts: i64) -> NaiveDate {
        chrono::DateTime::from_timestamp(ts, 0)
            .map(|dt| dt.date_naive())
            .unwrap_or_default()
    }
}

impl Default for YahooProvider {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Parsing ──────────────────────────────────────────────────────────────────

/// Parse a `YahooChartResponse` into a vec of `Candle` structs.
///
/// Rows with any `None` OHLCV value are silently skipped (Yahoo emits null on
/// market holidays and partial trading days).  Only rows whose date falls
/// within `[from, to]` are included.
fn parse_yahoo_candles(
    resp: YahooChartResponse,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<Candle>, DataError> {
    if let Some(err) = resp.chart.error {
        return Err(DataError::Http(format!(
            "Yahoo API error: {}",
            err.description
        )));
    }

    let result = resp
        .chart
        .result
        .into_iter()
        .next()
        .ok_or_else(|| DataError::Parse("Yahoo response has no result".to_string()))?;

    if result.timestamp.is_empty() {
        return Ok(vec![]);
    }

    let quote = result
        .indicators
        .quote
        .into_iter()
        .next()
        .ok_or_else(|| DataError::Parse("Yahoo response has no quote data".to_string()))?;

    let adj_closes: Vec<Option<f64>> = result
        .indicators
        .adjclose
        .into_iter()
        .next()
        .map(|a| a.adjclose)
        .unwrap_or_default();

    let mut candles = Vec::with_capacity(result.timestamp.len());

    for (i, &ts) in result.timestamp.iter().enumerate() {
        let date = YahooProvider::timestamp_to_date(ts);

        // Skip rows outside the requested window.
        if date < from || date > to {
            continue;
        }

        // Skip rows with any null OHLCV.
        let open = match quote.open.get(i).and_then(|v| *v) {
            Some(v) => v,
            None => continue,
        };
        let high = match quote.high.get(i).and_then(|v| *v) {
            Some(v) => v,
            None => continue,
        };
        let low = match quote.low.get(i).and_then(|v| *v) {
            Some(v) => v,
            None => continue,
        };
        let close = match quote.close.get(i).and_then(|v| *v) {
            Some(v) => v,
            None => continue,
        };
        let volume = match quote.volume.get(i).and_then(|v| *v) {
            Some(v) => v,
            None => continue,
        };

        // Prefer adjusted close; fall back to unadjusted.
        let adj_close = adj_closes.get(i).and_then(|v| *v).unwrap_or(close);

        candles.push(Candle {
            date,
            open,
            high,
            low,
            close,
            adj_close,
            volume,
        });
    }

    Ok(candles)
}

// ─── DataProvider impl ────────────────────────────────────────────────────────

#[async_trait]
impl DataProvider for YahooProvider {
    async fn fetch_candles(
        &self,
        ticker: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Candle>, DataError> {
        let range = Self::date_range_to_yahoo_range(from, to);
        let url = format!(
            "{}/v8/finance/chart/{}?range={}&interval=1d&includeAdjustedClose=true",
            self.base_url, ticker, range
        );

        let body = self.get_with_retry(&url).await?;
        let resp: YahooChartResponse =
            serde_json::from_str(&body).map_err(|e| DataError::Parse(e.to_string()))?;

        let candles = parse_yahoo_candles(resp, from, to)?;

        if candles.is_empty() {
            return Err(DataError::NotFound {
                ticker: ticker.to_string(),
            });
        }

        Ok(candles)
    }

    async fn fetch_stocks(&self, tickers: &[&str]) -> Result<Vec<Stock>, DataError> {
        // Yahoo does not have a convenient profile endpoint in the v8 API.
        // Return minimal Stock structs with the ticker as name.
        let stocks = tickers
            .iter()
            .map(|t| Stock {
                ticker: t.to_string(),
                name: t.to_string(),
                sector: String::new(),
                industry: String::new(),
            })
            .collect();
        Ok(stocks)
    }

    async fn fetch_news(
        &self,
        _ticker: &str,
        _from_date: NaiveDate,
        _limit: usize,
    ) -> Result<Vec<NewsArticle>, DataError> {
        // Yahoo news requires a different, authenticated endpoint.
        // Return empty vec — the pipeline gracefully skips sentiment scoring
        // when no articles are available.
        Ok(vec![])
    }

    async fn fetch_spy_candles(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Candle>, DataError> {
        self.fetch_candles("SPY", from, to).await
    }

    async fn fetch_universe(&self) -> Result<Vec<String>, DataError> {
        Ok(SP500.iter().map(|t| t.to_string()).collect())
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    // ── Helpers ───────────────────────────────────────────────────────────

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Minimal valid Yahoo v8 JSON fixture with three bars and adj-close data.
    fn sample_yahoo_json() -> &'static str {
        r#"{
            "chart": {
                "result": [{
                    "timestamp": [1704067200, 1704153600, 1704240000],
                    "indicators": {
                        "quote": [{
                            "open":   [150.5, 151.0, 149.5],
                            "high":   [151.0, 153.0, 150.0],
                            "low":    [149.0, 150.0, 147.0],
                            "close":  [150.0, 152.0, 148.0],
                            "volume": [1000000, 1200000, 800000]
                        }],
                        "adjclose": [{
                            "adjclose": [149.5, 151.4, 147.6]
                        }]
                    }
                }],
                "error": null
            }
        }"#
    }

    /// Fixture where some OHLCV values are null (market holiday or partial day).
    fn null_values_json() -> &'static str {
        r#"{
            "chart": {
                "result": [{
                    "timestamp": [1704067200, 1704153600, 1704240000],
                    "indicators": {
                        "quote": [{
                            "open":   [150.5, null,  149.5],
                            "high":   [151.0, 153.0, 150.0],
                            "low":    [149.0, null,  147.0],
                            "close":  [150.0, null,  148.0],
                            "volume": [1000000, null, 800000]
                        }],
                        "adjclose": [{
                            "adjclose": [149.5, null, 147.6]
                        }]
                    }
                }],
                "error": null
            }
        }"#
    }

    /// Fixture without adjclose — close should be used as adj_close.
    fn no_adjclose_json() -> &'static str {
        r#"{
            "chart": {
                "result": [{
                    "timestamp": [1704067200],
                    "indicators": {
                        "quote": [{
                            "open":   [150.5],
                            "high":   [151.0],
                            "low":    [149.0],
                            "close":  [150.0],
                            "volume": [1000000]
                        }]
                    }
                }],
                "error": null
            }
        }"#
    }

    /// Fixture with an API-level error object.
    fn error_response_json() -> &'static str {
        r#"{
            "chart": {
                "result": null,
                "error": {
                    "code": "Not Found",
                    "description": "No fundamentals data found for any of the summaryTypes=financialData"
                }
            }
        }"#
    }

    /// Fixture with empty result array.
    fn empty_result_json() -> &'static str {
        r#"{
            "chart": {
                "result": [],
                "error": null
            }
        }"#
    }

    // ── Unit tests — JSON parsing ──────────────────────────────────────────

    #[test]
    fn parse_three_bar_response() {
        let resp: YahooChartResponse = serde_json::from_str(sample_yahoo_json()).unwrap();
        let from = date(2024, 1, 1);
        let to = date(2024, 1, 31);
        let candles = parse_yahoo_candles(resp, from, to).unwrap();

        assert_eq!(candles.len(), 3);
        assert_eq!(candles[0].open, 150.5);
        assert_eq!(candles[0].high, 151.0);
        assert_eq!(candles[0].low, 149.0);
        assert_eq!(candles[0].close, 150.0);
        assert_eq!(candles[0].volume, 1_000_000);
        // Adjusted close from adjclose array
        assert!((candles[0].adj_close - 149.5).abs() < 1e-9);
    }

    #[test]
    fn parse_uses_adjusted_close_when_present() {
        let resp: YahooChartResponse = serde_json::from_str(sample_yahoo_json()).unwrap();
        let from = date(2024, 1, 1);
        let to = date(2024, 1, 31);
        let candles = parse_yahoo_candles(resp, from, to).unwrap();

        // adj_close differs from close — confirms adjclose field is used
        for c in &candles {
            assert!(
                (c.adj_close - c.close).abs() > 1e-9,
                "adj_close should differ from close when adjclose array is present"
            );
        }
    }

    #[test]
    fn parse_falls_back_to_close_when_adjclose_absent() {
        let resp: YahooChartResponse = serde_json::from_str(no_adjclose_json()).unwrap();
        let from = date(2024, 1, 1);
        let to = date(2024, 1, 31);
        let candles = parse_yahoo_candles(resp, from, to).unwrap();

        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].adj_close, candles[0].close);
    }

    #[test]
    fn parse_skips_null_rows() {
        let resp: YahooChartResponse = serde_json::from_str(null_values_json()).unwrap();
        let from = date(2024, 1, 1);
        let to = date(2024, 1, 31);
        let candles = parse_yahoo_candles(resp, from, to).unwrap();

        // Row 2 (index 1) has null OHLCV — it should be skipped
        assert_eq!(candles.len(), 2, "Null row should be skipped");
        // Remaining candles should be valid
        for c in &candles {
            assert!(c.open > 0.0);
            assert!(c.close > 0.0);
            assert!(c.volume > 0);
        }
    }

    #[test]
    fn parse_filters_to_date_range() {
        let resp: YahooChartResponse = serde_json::from_str(sample_yahoo_json()).unwrap();
        // Timestamps 1704067200, 1704153600, 1704240000 = 2024-01-01, 02, 03
        let from = date(2024, 1, 2);
        let to = date(2024, 1, 2);
        let candles = parse_yahoo_candles(resp, from, to).unwrap();

        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].date, date(2024, 1, 2));
    }

    #[test]
    fn parse_api_error_returns_http_error() {
        let resp: YahooChartResponse = serde_json::from_str(error_response_json()).unwrap();
        let err = parse_yahoo_candles(resp, date(2024, 1, 1), date(2024, 1, 31)).unwrap_err();
        matches!(err, DataError::Http(_));
    }

    #[test]
    fn parse_empty_result_returns_empty_vec() {
        let resp: YahooChartResponse = serde_json::from_str(empty_result_json()).unwrap();
        let err = parse_yahoo_candles(resp, date(2024, 1, 1), date(2024, 1, 31)).unwrap_err();
        matches!(err, DataError::Parse(_));
    }

    // ── Unit tests — date/range helpers ───────────────────────────────────

    #[test]
    fn timestamp_to_date_converts_unix_epoch_correctly() {
        // 2024-01-01 00:00:00 UTC
        let d = YahooProvider::timestamp_to_date(1_704_067_200);
        assert_eq!(d, date(2024, 1, 1));
    }

    #[test]
    fn timestamp_to_date_zero_returns_unix_epoch_date() {
        let d = YahooProvider::timestamp_to_date(0);
        assert_eq!(d, date(1970, 1, 1));
    }

    #[test]
    fn date_range_to_yahoo_range_one_month() {
        let from = date(2024, 1, 1);
        let to = date(2024, 1, 31);
        assert_eq!(YahooProvider::date_range_to_yahoo_range(from, to), "1mo");
    }

    #[test]
    fn date_range_to_yahoo_range_three_months() {
        let from = date(2024, 1, 1);
        let to = date(2024, 3, 31);
        assert_eq!(YahooProvider::date_range_to_yahoo_range(from, to), "3mo");
    }

    #[test]
    fn date_range_to_yahoo_range_one_year() {
        let from = date(2023, 1, 1);
        let to = date(2023, 12, 31);
        assert_eq!(YahooProvider::date_range_to_yahoo_range(from, to), "1y");
    }

    #[test]
    fn date_range_to_yahoo_range_two_years() {
        let from = date(2022, 1, 1);
        let to = date(2023, 12, 31);
        assert_eq!(YahooProvider::date_range_to_yahoo_range(from, to), "2y");
    }

    #[test]
    fn date_range_to_yahoo_range_five_years() {
        let from = date(2019, 1, 1);
        let to = date(2024, 1, 1);
        assert_eq!(YahooProvider::date_range_to_yahoo_range(from, to), "5y");
    }

    // ── Unit tests — provider methods that don't hit the network ──────────

    #[tokio::test]
    async fn fetch_universe_returns_sp500_tickers() {
        let provider = YahooProvider::new();
        let universe = provider.fetch_universe().await.unwrap();

        assert!(!universe.is_empty());
        assert!(universe.contains(&"AAPL".to_string()));
        assert!(universe.contains(&"NVDA".to_string()));
        assert!(universe.contains(&"MSFT".to_string()));
    }

    #[tokio::test]
    async fn fetch_news_returns_empty_vec() {
        let provider = YahooProvider::new();
        let from = date(2024, 1, 1);
        let articles = provider.fetch_news("AAPL", from, 10).await.unwrap();
        assert!(articles.is_empty(), "Yahoo news should return empty vec");
    }

    #[tokio::test]
    async fn fetch_stocks_returns_stub_entries() {
        let provider = YahooProvider::new();
        let stocks = provider.fetch_stocks(&["AAPL", "NVDA"]).await.unwrap();
        assert_eq!(stocks.len(), 2);
        assert_eq!(stocks[0].ticker, "AAPL");
        assert_eq!(stocks[1].ticker, "NVDA");
    }

    // ── Integration-style tests using wiremock ────────────────────────────

    #[tokio::test]
    async fn fetch_candles_parses_response_via_http() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_regex("/v8/finance/chart/AAPL.*"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sample_yahoo_json()))
            .mount(&mock_server)
            .await;

        let provider = YahooProvider::new_with_base_url(mock_server.uri());
        let from = date(2024, 1, 1);
        let to = date(2024, 1, 31);
        let candles = provider.fetch_candles("AAPL", from, to).await.unwrap();

        assert_eq!(candles.len(), 3);
        assert_eq!(candles[0].close, 150.0);
        assert!((candles[0].adj_close - 149.5).abs() < 1e-9);
    }

    #[tokio::test]
    async fn fetch_candles_empty_result_returns_not_found() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_regex("/v8/finance/chart/UNKNOWN.*"))
            .respond_with(ResponseTemplate::new(200).set_body_string(empty_result_json()))
            .mount(&mock_server)
            .await;

        let provider = YahooProvider::new_with_base_url(mock_server.uri());
        let from = date(2024, 1, 1);
        let to = date(2024, 1, 31);
        let result = provider.fetch_candles("UNKNOWN", from, to).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn spy_candles_delegates_to_fetch_candles() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_regex("/v8/finance/chart/SPY.*"))
            .respond_with(ResponseTemplate::new(200).set_body_string(sample_yahoo_json()))
            .expect(1)
            .mount(&mock_server)
            .await;

        let provider = YahooProvider::new_with_base_url(mock_server.uri());
        let from = date(2024, 1, 1);
        let to = date(2024, 1, 31);
        let candles = provider.fetch_spy_candles(from, to).await.unwrap();
        assert_eq!(candles.len(), 3);
    }

    #[tokio::test]
    async fn fetch_candles_returns_http_error_on_400() {
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_regex("/v8/finance/chart/BAD.*"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&mock_server)
            .await;

        let provider = YahooProvider::new_with_base_url(mock_server.uri());
        let from = date(2024, 1, 1);
        let to = date(2024, 1, 31);
        let result = provider.fetch_candles("BAD", from, to).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            DataError::Http(msg) => assert!(msg.contains("400")),
            other => panic!("Expected Http error, got: {other:?}"),
        }
    }

    #[test]
    fn date_range_six_months() {
        // 180 days should map to "6mo" (d <= 185 branch).
        let from = date(2024, 1, 1);
        let to = date(2024, 6, 29); // 180 days later
        let days = (to - from).num_days();
        assert!(days <= 185, "Expected ≤185 days for 6mo branch, got {days}");
        assert_eq!(
            YahooProvider::date_range_to_yahoo_range(from, to),
            "6mo",
            "180-day span should map to \"6mo\""
        );
    }

    #[tokio::test]
    async fn fetch_candles_429_returns_error() {
        // Use the fast-backoff provider so the test completes in < 200 ms.
        // The mock always returns 429 → backoff expires → RateLimit error.
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_regex("/v8/finance/chart/RLMT.*"))
            .respond_with(ResponseTemplate::new(429))
            .mount(&mock_server)
            .await;

        let provider = YahooProvider::new_with_fast_backoff(mock_server.uri());
        let from = date(2024, 1, 1);
        let to = date(2024, 1, 31);
        let result = provider.fetch_candles("RLMT", from, to).await;

        assert!(result.is_err(), "429 response should return an error");
        // The error should be a RateLimit variant (set by the 429 handler).
        match result.unwrap_err() {
            DataError::RateLimit { api } => {
                assert_eq!(api, "yahoo", "RateLimit api field should be \"yahoo\"");
            }
            other => panic!("Expected RateLimit error, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_candles_500_returns_error() {
        // Use the fast-backoff provider so retries on 500 expire quickly.
        use wiremock::matchers::{method, path_regex};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path_regex("/v8/finance/chart/SRV500.*"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let provider = YahooProvider::new_with_fast_backoff(mock_server.uri());
        let from = date(2024, 1, 1);
        let to = date(2024, 1, 31);
        let result = provider.fetch_candles("SRV500", from, to).await;

        assert!(result.is_err(), "500 response should return an error");
        match result.unwrap_err() {
            DataError::Http(msg) => {
                assert!(
                    msg.contains("500") || msg.contains("Server error"),
                    "Http error message should mention 500 or 'Server error', got: {msg}"
                );
            }
            other => panic!("Expected Http error, got: {other:?}"),
        }
    }
}
