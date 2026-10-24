//! Finnhub API client with rate limiting and exponential retry.
//!
//! The `FinnhubClient` implements `DataProvider` and handles:
//! - OHLCV candle fetching via `/stock/candle`
//! - Company profile via `/stock/profile2`
//! - S&P 500 constituents via `/index/constituents`
//! - Company news via `/company-news` (Phase 3)
//!
//! Rate limiting: 60 req/min on the free tier (governor crate).
//! Retry: Exponential backoff on 429/5xx (backoff crate).

use async_trait::async_trait;
use chrono::NaiveDate;
use governor::{Quota, RateLimiter};
use serde::Deserialize;
use std::num::NonZeroU32;
use std::sync::Arc;

use crate::data::models::{Candle, NewsArticle, Stock};
use crate::data::provider::DataProvider;
use crate::error::DataError;

/// Finnhub `/stock/candle` response format.
#[derive(Debug, Deserialize)]
struct FinnhubCandleResponse {
    /// Status: "ok" or "no_data"
    s: String,
    /// Close prices
    #[serde(default)]
    c: Vec<f64>,
    /// High prices
    #[serde(default)]
    h: Vec<f64>,
    /// Low prices
    #[serde(default)]
    l: Vec<f64>,
    /// Open prices
    #[serde(default)]
    o: Vec<f64>,
    /// Unix timestamps
    #[serde(default)]
    t: Vec<i64>,
    /// Volumes
    #[serde(default)]
    v: Vec<f64>,
}

/// Finnhub `/stock/profile2` response format.
#[derive(Debug, Deserialize)]
struct FinnhubProfileResponse {
    #[serde(default)]
    ticker: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "finnhubIndustry")]
    finnhub_industry: Option<String>,
}

/// Finnhub `/index/constituents` response format.
#[derive(Debug, Deserialize)]
struct FinnhubIndexResponse {
    #[serde(default)]
    constituents: Vec<String>,
}

/// Finnhub `/company-news` response item.
#[derive(Debug, Deserialize)]
struct FinnhubNewsItem {
    #[serde(default)]
    headline: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    datetime: i64,
    #[serde(default)]
    source: String,
}

type DefaultRateLimiter = RateLimiter<
    governor::state::NotKeyed,
    governor::state::InMemoryState,
    governor::clock::DefaultClock,
>;

pub struct FinnhubClient {
    pub(crate) api_key: String,
    pub base_url: String,
    client: reqwest::Client,
    rate_limiter: Arc<DefaultRateLimiter>,
}

impl std::fmt::Debug for FinnhubClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FinnhubClient")
            .field("api_key", &"[REDACTED]")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl FinnhubClient {
    pub fn new(api_key: String) -> Self {
        let quota = Quota::per_minute(NonZeroU32::new(60).unwrap());
        Self {
            api_key,
            base_url: "https://finnhub.io/api/v1".to_string(),
            client: reqwest::Client::builder()
                .gzip(true)
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("Failed to build reqwest client"),
            rate_limiter: Arc::new(RateLimiter::direct(quota)),
        }
    }

    #[cfg(test)]
    pub fn new_with_base_url(api_key: String, base_url: String) -> Self {
        let quota = Quota::per_minute(NonZeroU32::new(600).unwrap()); // Higher limit for tests
        Self {
            api_key,
            base_url,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .expect("Failed to build reqwest client"),
            rate_limiter: Arc::new(RateLimiter::direct(quota)),
        }
    }

    /// Wait for rate limit permit, then make a GET request with retry on 429/5xx.
    pub(crate) async fn get_with_retry(&self, url: &str) -> Result<String, DataError> {
        // Wait for rate limit token
        self.rate_limiter.until_ready().await;

        let op = || async {
            let response = self.client.get(url).send().await.map_err(|e| {
                backoff::Error::permanent(DataError::Http(format!("Request failed: {}", e)))
            })?;

            let status = response.status();
            if status.is_success() {
                let body = response.text().await.map_err(|e| {
                    backoff::Error::permanent(DataError::Http(format!(
                        "Failed to read response body: {}",
                        e
                    )))
                })?;
                Ok(body)
            } else if status.as_u16() == 429 {
                Err(backoff::Error::transient(DataError::RateLimit {
                    api: "finnhub".to_string(),
                }))
            } else if status.is_server_error() {
                Err(backoff::Error::transient(DataError::Http(format!(
                    "Server error: {}",
                    status
                ))))
            } else {
                Err(backoff::Error::permanent(DataError::Http(format!(
                    "HTTP {}: unexpected status",
                    status
                ))))
            }
        };

        backoff::future::retry(
            backoff::ExponentialBackoffBuilder::new()
                .with_max_elapsed_time(Some(std::time::Duration::from_secs(60)))
                .with_initial_interval(std::time::Duration::from_millis(500))
                .build(),
            op,
        )
        .await
    }

    /// Convert Unix timestamp to NaiveDate.
    fn timestamp_to_date(ts: i64) -> NaiveDate {
        chrono::DateTime::from_timestamp(ts, 0)
            .map(|dt| dt.date_naive())
            .unwrap_or_default()
    }
}

#[async_trait]
impl DataProvider for FinnhubClient {
    async fn fetch_candles(
        &self,
        ticker: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Candle>, DataError> {
        let from_ts = from.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp();
        let to_ts = to.and_hms_opt(23, 59, 59).unwrap().and_utc().timestamp();

        let url = format!(
            "{}/stock/candle?symbol={}&resolution=D&from={}&to={}&token={}",
            self.base_url, ticker, from_ts, to_ts, self.api_key
        );

        let body = self.get_with_retry(&url).await?;
        let resp: FinnhubCandleResponse =
            serde_json::from_str(&body).map_err(|e| DataError::Parse(e.to_string()))?;

        if resp.s == "no_data" || resp.c.is_empty() {
            return Err(DataError::NotFound {
                ticker: ticker.to_string(),
            });
        }

        let candles: Vec<Candle> = (0..resp.c.len())
            .map(|i| Candle {
                date: Self::timestamp_to_date(resp.t[i]),
                open: resp.o[i],
                high: resp.h[i],
                low: resp.l[i],
                close: resp.c[i],
                adj_close: resp.c[i], // Finnhub doesn't provide adjusted close separately
                volume: resp.v[i] as u64,
            })
            .collect();

        Ok(candles)
    }

    async fn fetch_stocks(&self, tickers: &[&str]) -> Result<Vec<Stock>, DataError> {
        let mut stocks = Vec::with_capacity(tickers.len());

        for ticker in tickers {
            let url = format!(
                "{}/stock/profile2?symbol={}&token={}",
                self.base_url, ticker, self.api_key
            );

            let body = self.get_with_retry(&url).await?;
            let profile: FinnhubProfileResponse =
                serde_json::from_str(&body).map_err(|e| DataError::Parse(e.to_string()))?;

            if let Some(name) = profile.name {
                stocks.push(Stock {
                    ticker: profile.ticker.unwrap_or_else(|| ticker.to_string()),
                    name,
                    sector: profile.finnhub_industry.clone().unwrap_or_default(),
                    industry: profile.finnhub_industry.unwrap_or_default(),
                });
            }
        }

        Ok(stocks)
    }

    async fn fetch_news(
        &self,
        ticker: &str,
        from_date: NaiveDate,
        limit: usize,
    ) -> Result<Vec<NewsArticle>, DataError> {
        let today = chrono::Utc::now().date_naive();

        let url = format!(
            "{}/company-news?symbol={}&from={}&to={}&token={}",
            self.base_url, ticker, from_date, today, self.api_key
        );

        let body = self.get_with_retry(&url).await?;
        let items: Vec<FinnhubNewsItem> =
            serde_json::from_str(&body).map_err(|e| DataError::Parse(e.to_string()))?;

        let articles: Vec<NewsArticle> = items
            .into_iter()
            .take(limit)
            .map(|item| NewsArticle {
                ticker: ticker.to_string(),
                headline: item.headline,
                summary: item.summary,
                published_at: chrono::DateTime::from_timestamp(item.datetime, 0)
                    .unwrap_or_else(chrono::Utc::now),
                source: item.source,
            })
            .collect();

        Ok(articles)
    }

    async fn fetch_spy_candles(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Candle>, DataError> {
        self.fetch_candles("SPY", from, to).await
    }

    async fn fetch_universe(&self) -> Result<Vec<String>, DataError> {
        let url = format!(
            "{}/index/constituents?symbol=^GSPC&token={}",
            self.base_url, self.api_key
        );

        let body = self.get_with_retry(&url).await?;
        let resp: FinnhubIndexResponse =
            serde_json::from_str(&body).map_err(|e| DataError::Parse(e.to_string()))?;

        if resp.constituents.is_empty() {
            return Err(DataError::NotFound {
                ticker: "^GSPC".to_string(),
            });
        }

        Ok(resp.constituents)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_candle_response() -> serde_json::Value {
        serde_json::json!({
            "s": "ok",
            "c": [150.0, 152.0, 148.0],
            "h": [151.0, 153.0, 150.0],
            "l": [149.0, 150.0, 147.0],
            "o": [150.5, 151.0, 149.5],
            "t": [1704067200, 1704153600, 1704240000],
            "v": [1000000.0, 1200000.0, 800000.0]
        })
    }

    // ── Unit tests ──────────────────────────────────────────────────────────

    #[test]
    fn timestamp_to_date_converts_correctly() {
        // 2024-01-01 00:00:00 UTC
        let date = FinnhubClient::timestamp_to_date(1704067200);
        assert_eq!(date, NaiveDate::from_ymd_opt(2024, 1, 1).unwrap());
    }

    #[test]
    fn timestamp_to_date_zero_returns_default() {
        let date = FinnhubClient::timestamp_to_date(0);
        assert_eq!(date, NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
    }

    #[test]
    fn deserialize_candle_response() {
        let json = serde_json::to_string(&sample_candle_response()).unwrap();
        let resp: FinnhubCandleResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(resp.s, "ok");
        assert_eq!(resp.c.len(), 3);
        assert_eq!(resp.c[0], 150.0);
        assert_eq!(resp.t[0], 1704067200);
    }

    #[test]
    fn deserialize_no_data_response() {
        let json = r#"{"s":"no_data"}"#;
        let resp: FinnhubCandleResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.s, "no_data");
        assert!(resp.c.is_empty());
    }

    #[test]
    fn deserialize_profile_response() {
        let json = r#"{"ticker":"AAPL","name":"Apple Inc","finnhubIndustry":"Technology"}"#;
        let resp: FinnhubProfileResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.ticker.unwrap(), "AAPL");
        assert_eq!(resp.name.unwrap(), "Apple Inc");
        assert_eq!(resp.finnhub_industry.unwrap(), "Technology");
    }

    #[test]
    fn deserialize_index_response() {
        let json = r#"{"constituents":["AAPL","MSFT","NVDA"],"symbol":"^GSPC"}"#;
        let resp: FinnhubIndexResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.constituents.len(), 3);
        assert_eq!(resp.constituents[0], "AAPL");
    }

    // ── Integration tests with wiremock ─────────────────────────────────────

    #[tokio::test]
    async fn fetch_candles_parses_response() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/stock/candle"))
            .and(query_param("symbol", "AAPL"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_candle_response()))
            .mount(&mock_server)
            .await;

        let client = FinnhubClient::new_with_base_url("test_key".into(), mock_server.uri());
        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();
        let candles = client.fetch_candles("AAPL", from, to).await.unwrap();

        assert_eq!(candles.len(), 3);
        assert_eq!(candles[0].close, 150.0);
        assert_eq!(candles[0].open, 150.5);
        assert_eq!(candles[0].high, 151.0);
        assert_eq!(candles[0].low, 149.0);
        assert_eq!(candles[0].volume, 1_000_000);
    }

    #[tokio::test]
    async fn fetch_candles_no_data_returns_not_found() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/stock/candle"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"s": "no_data"})),
            )
            .mount(&mock_server)
            .await;

        let client = FinnhubClient::new_with_base_url("test_key".into(), mock_server.uri());
        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();
        let result = client.fetch_candles("INVALID", from, to).await;

        assert!(result.is_err());
        match result.unwrap_err() {
            DataError::NotFound { ticker } => assert_eq!(ticker, "INVALID"),
            other => panic!("Expected NotFound, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn fetch_stocks_parses_profile() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/stock/profile2"))
            .and(query_param("symbol", "NVDA"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ticker": "NVDA",
                "name": "NVIDIA Corporation",
                "finnhubIndustry": "Semiconductors"
            })))
            .mount(&mock_server)
            .await;

        let client = FinnhubClient::new_with_base_url("test_key".into(), mock_server.uri());
        let stocks = client.fetch_stocks(&["NVDA"]).await.unwrap();

        assert_eq!(stocks.len(), 1);
        assert_eq!(stocks[0].ticker, "NVDA");
        assert_eq!(stocks[0].name, "NVIDIA Corporation");
        assert_eq!(stocks[0].sector, "Semiconductors");
    }

    #[tokio::test]
    async fn fetch_universe_returns_constituents() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/index/constituents"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "constituents": ["AAPL", "MSFT", "NVDA", "AMZN", "META"],
                "symbol": "^GSPC"
            })))
            .mount(&mock_server)
            .await;

        let client = FinnhubClient::new_with_base_url("test_key".into(), mock_server.uri());
        let universe = client.fetch_universe().await.unwrap();

        assert_eq!(universe.len(), 5);
        assert!(universe.contains(&"AAPL".to_string()));
        assert!(universe.contains(&"NVDA".to_string()));
    }

    #[tokio::test]
    async fn fetch_universe_empty_returns_not_found() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/index/constituents"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"constituents": [], "symbol": "^GSPC"})),
            )
            .mount(&mock_server)
            .await;

        let client = FinnhubClient::new_with_base_url("test_key".into(), mock_server.uri());
        let result = client.fetch_universe().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fetch_candles_handles_400_error() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/stock/candle"))
            .respond_with(ResponseTemplate::new(400))
            .mount(&mock_server)
            .await;

        let client = FinnhubClient::new_with_base_url("test_key".into(), mock_server.uri());
        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();
        let result = client.fetch_candles("AAPL", from, to).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            DataError::Http(msg) => assert!(msg.contains("400")),
            other => panic!("Expected Http error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn fetch_news_parses_articles() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/company-news"))
            .and(query_param("symbol", "AAPL"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {
                    "headline": "Apple Beats Earnings",
                    "summary": "Apple reported strong Q4 results",
                    "datetime": 1704067200,
                    "source": "Reuters"
                },
                {
                    "headline": "New iPhone Launch",
                    "summary": "Apple unveils iPhone 16",
                    "datetime": 1704153600,
                    "source": "Bloomberg"
                }
            ])))
            .mount(&mock_server)
            .await;

        let client = FinnhubClient::new_with_base_url("test_key".into(), mock_server.uri());
        let from_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let articles = client.fetch_news("AAPL", from_date, 10).await.unwrap();

        assert_eq!(articles.len(), 2);
        assert_eq!(articles[0].headline, "Apple Beats Earnings");
        assert_eq!(articles[0].source, "Reuters");
        assert_eq!(articles[0].ticker, "AAPL");
    }

    #[tokio::test]
    async fn fetch_news_respects_limit() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/company-news"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"headline": "A", "summary": "", "datetime": 1, "source": ""},
                {"headline": "B", "summary": "", "datetime": 2, "source": ""},
                {"headline": "C", "summary": "", "datetime": 3, "source": ""}
            ])))
            .mount(&mock_server)
            .await;

        let client = FinnhubClient::new_with_base_url("test_key".into(), mock_server.uri());
        let from_date = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let articles = client.fetch_news("AAPL", from_date, 1).await.unwrap();
        assert_eq!(articles.len(), 1);
    }

    #[tokio::test]
    async fn fetch_news_custom_lookback_uses_from_date() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/company-news"))
            .and(query_param("from", "2024-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"headline": "Custom lookback test", "summary": "", "datetime": 1717200000, "source": "Test"}
            ])))
            .mount(&mock_server)
            .await;

        let client = FinnhubClient::new_with_base_url("test_key".into(), mock_server.uri());
        let from_date = NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();
        let articles = client.fetch_news("AAPL", from_date, 50).await.unwrap();
        assert_eq!(articles.len(), 1);
        assert_eq!(articles[0].headline, "Custom lookback test");
    }

    #[tokio::test]
    async fn spy_candles_delegates_to_fetch_candles() {
        let mock_server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/stock/candle"))
            .and(query_param("symbol", "SPY"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_candle_response()))
            .expect(1)
            .mount(&mock_server)
            .await;

        let client = FinnhubClient::new_with_base_url("test_key".into(), mock_server.uri());
        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 1, 3).unwrap();
        let candles = client.fetch_spy_candles(from, to).await.unwrap();
        assert_eq!(candles.len(), 3);
    }
}
