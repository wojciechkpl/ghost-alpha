use async_trait::async_trait;
use chrono::NaiveDate;

use crate::data::models::{Candle, NewsArticle, Stock};
use crate::error::DataError;

/// Primary seam for mocking all external market data.
/// Pass `Arc<dyn DataProvider>` through the pipeline; swap for `MockDataProvider` in tests.
#[async_trait]
pub trait DataProvider: Send + Sync {
    async fn fetch_candles(
        &self,
        ticker: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Candle>, DataError>;

    async fn fetch_stocks(&self, tickers: &[&str]) -> Result<Vec<Stock>, DataError>;

    async fn fetch_news(
        &self,
        ticker: &str,
        from_date: NaiveDate,
        limit: usize,
    ) -> Result<Vec<NewsArticle>, DataError>;

    async fn fetch_spy_candles(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Candle>, DataError>;

    async fn fetch_universe(&self) -> Result<Vec<String>, DataError>;
}

/// In-tree mock — only compiled in test builds.
#[cfg(test)]
pub mod mock {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    pub struct MockDataProvider {
        candles: HashMap<String, Vec<Candle>>,
        pub call_count: Arc<AtomicUsize>,
    }

    impl Default for MockDataProvider {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockDataProvider {
        pub fn new() -> Self {
            Self {
                candles: HashMap::new(),
                call_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        pub fn with_candles(mut self, ticker: &str, candles: Vec<Candle>) -> Self {
            self.candles.insert(ticker.to_string(), candles);
            self
        }

        pub fn call_count(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl DataProvider for MockDataProvider {
        async fn fetch_candles(
            &self,
            ticker: &str,
            _from: NaiveDate,
            _to: NaiveDate,
        ) -> Result<Vec<Candle>, DataError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            self.candles
                .get(ticker)
                .cloned()
                .ok_or(DataError::NotFound {
                    ticker: ticker.to_string(),
                })
        }

        async fn fetch_stocks(&self, _tickers: &[&str]) -> Result<Vec<Stock>, DataError> {
            Ok(vec![])
        }

        async fn fetch_news(
            &self,
            _ticker: &str,
            _from_date: NaiveDate,
            _limit: usize,
        ) -> Result<Vec<NewsArticle>, DataError> {
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
            Ok(self.candles.keys().cloned().collect())
        }
    }
}
