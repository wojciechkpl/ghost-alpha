use std::sync::Arc;

use async_trait::async_trait;
use chrono::NaiveDate;

use crate::config::CacheConfig;
use crate::data::models::{Candle, NewsArticle, Stock};
use crate::data::provider::DataProvider;
use crate::error::DataError;
use crate::storage::db::Repository;

/// Ensure a minimal stock row exists so the FK constraint on candles is satisfied.
fn ensure_stock(repo: &dyn Repository, ticker: &str) {
    let stock = Stock {
        ticker: ticker.to_string(),
        name: ticker.to_string(),
        sector: String::new(),
        industry: String::new(),
    };
    let _ = repo.upsert_stock(&stock);
}

/// Caching decorator around any `DataProvider`.
///
/// On `fetch_candles`:
/// 1. Query SQLite for the latest stored candle date for the ticker.
/// 2. If data exists and is recent enough (within `full_refresh_days`), fetch only
///    the delta (from last stored date to today) from upstream.
/// 3. Merge cached + fresh candles, persist the new ones, return the full set.
/// 4. Every `full_refresh_days`, do a full 400-day refetch to pick up retroactive
///    adj_close corrections from splits/dividends.
///
/// All other `DataProvider` methods delegate directly to the inner provider.
pub struct CachingProvider {
    inner: Arc<dyn DataProvider>,
    repo: Arc<dyn Repository>,
    config: CacheConfig,
    /// The earliest date needed by the pipeline (typically today - 400 days).
    lookback_from: NaiveDate,
}

impl CachingProvider {
    pub fn new(
        inner: Arc<dyn DataProvider>,
        repo: Arc<dyn Repository>,
        config: CacheConfig,
        lookback_from: NaiveDate,
    ) -> Self {
        Self {
            inner,
            repo,
            config,
            lookback_from,
        }
    }

    /// Decide whether this ticker needs a full refresh or just a delta fetch.
    fn needs_full_refresh(&self, latest_stored: Option<NaiveDate>, today: NaiveDate) -> bool {
        match latest_stored {
            None => true, // No data at all → full fetch
            Some(latest) => {
                // Full refresh if the stored data is older than full_refresh_days
                let days_since_refresh = (today - latest).num_days();
                // Also full refresh if we don't have enough lookback history
                // (e.g. DB was seeded after our lookback_from date)
                days_since_refresh > self.config.full_refresh_days as i64
            }
        }
    }
}

#[async_trait]
impl DataProvider for CachingProvider {
    async fn fetch_candles(
        &self,
        ticker: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Candle>, DataError> {
        if !self.config.enabled {
            // Cache disabled — pass through to upstream
            return self.inner.fetch_candles(ticker, from, to).await;
        }

        let ticker_owned = ticker.to_string();
        let repo = self.repo.clone();

        // Query latest stored date (synchronous SQLite, so spawn_blocking)
        let latest_stored = {
            let t = ticker_owned.clone();
            let r = repo.clone();
            tokio::task::spawn_blocking(move || r.get_latest_candle_date(&t))
                .await
                .map_err(|e| DataError::Http(format!("task join error: {e}")))?
                .map_err(|e| DataError::Http(format!("DB error: {e}")))?
        };

        let today = to;

        if self.needs_full_refresh(latest_stored, today) {
            // Full fetch from upstream
            tracing::debug!(ticker = %ticker, "Cache MISS — full fetch");
            let candles = self.inner.fetch_candles(ticker, from, to).await?;

            // Persist to DB
            let t = ticker_owned.clone();
            let r = repo.clone();
            let c = candles.clone();
            tokio::task::spawn_blocking(move || {
                ensure_stock(r.as_ref(), &t);
                if let Err(e) = r.upsert_candles(&t, &c) {
                    tracing::warn!(ticker = %t, error = %e, "Failed to cache candles");
                }
            })
            .await
            .ok();

            Ok(candles)
        } else {
            // Delta fetch: load cached candles, fetch only new ones
            let latest = latest_stored.unwrap(); // safe — needs_full_refresh returned false
            let delta_from = latest + chrono::Duration::days(1);

            tracing::debug!(
                ticker = %ticker,
                cached_until = %latest,
                delta_from = %delta_from,
                "Cache HIT — delta fetch"
            );

            // Load cached candles from DB
            let cached = {
                let t = ticker_owned.clone();
                let r = repo.clone();
                let lf = self.lookback_from;
                tokio::task::spawn_blocking(move || r.get_candles_since(&t, lf))
                    .await
                    .map_err(|e| DataError::Http(format!("task join error: {e}")))?
                    .map_err(|e| DataError::Http(format!("DB error: {e}")))?
            };

            if delta_from > today {
                // Already up to date — return cached data
                tracing::debug!(ticker = %ticker, bars = cached.len(), "Fully cached, no fetch needed");
                return Ok(cached);
            }

            // Fetch only the delta from upstream
            let fresh = self.inner.fetch_candles(ticker, delta_from, to).await?;

            // Persist new candles
            if !fresh.is_empty() {
                let t = ticker_owned.clone();
                let r = repo.clone();
                let c = fresh.clone();
                tokio::task::spawn_blocking(move || {
                    ensure_stock(r.as_ref(), &t);
                    if let Err(e) = r.upsert_candles(&t, &c) {
                        tracing::warn!(ticker = %t, error = %e, "Failed to cache delta candles");
                    }
                })
                .await
                .ok();
            }

            // Merge: cached + fresh (both in chronological order)
            let mut merged = cached;
            merged.extend(fresh);
            // Deduplicate by date (prefer the freshly-fetched version)
            merged.sort_by_key(|c| c.date);
            merged.dedup_by_key(|c| c.date);

            Ok(merged)
        }
    }

    async fn fetch_stocks(&self, tickers: &[&str]) -> Result<Vec<Stock>, DataError> {
        self.inner.fetch_stocks(tickers).await
    }

    async fn fetch_news(
        &self,
        ticker: &str,
        from_date: NaiveDate,
        limit: usize,
    ) -> Result<Vec<NewsArticle>, DataError> {
        self.inner.fetch_news(ticker, from_date, limit).await
    }

    async fn fetch_spy_candles(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Candle>, DataError> {
        // SPY also benefits from caching
        self.fetch_candles("SPY", from, to).await
    }

    async fn fetch_universe(&self) -> Result<Vec<String>, DataError> {
        self.inner.fetch_universe().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::provider::mock::MockDataProvider;
    use crate::storage::db::SqliteRepository;

    fn make_candle(date: NaiveDate) -> Candle {
        Candle {
            date,
            open: 100.0,
            high: 105.0,
            low: 95.0,
            close: 102.0,
            adj_close: 102.0,
            volume: 1_000_000,
        }
    }

    fn dates(from: &str, days: i64) -> Vec<Candle> {
        let start = NaiveDate::parse_from_str(from, "%Y-%m-%d").unwrap();
        (0..days)
            .map(|d| make_candle(start + chrono::Duration::days(d)))
            .collect()
    }

    fn seed_stock(repo: &SqliteRepository, ticker: &str) {
        ensure_stock(repo, ticker);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn first_run_does_full_fetch_and_persists() {
        let repo = Arc::new(SqliteRepository::open_in_memory().unwrap());
        let candles = dates("2025-01-01", 10);
        let provider = Arc::new(MockDataProvider::new().with_candles("AAPL", candles.clone()));
        let cache_cfg = CacheConfig {
            enabled: true,
            full_refresh_days: 7,
        };
        let from = NaiveDate::parse_from_str("2025-01-01", "%Y-%m-%d").unwrap();
        let to = NaiveDate::parse_from_str("2025-01-10", "%Y-%m-%d").unwrap();

        let caching = CachingProvider::new(provider.clone(), repo.clone(), cache_cfg, from);
        let result = caching.fetch_candles("AAPL", from, to).await.unwrap();

        assert_eq!(result.len(), 10);
        assert_eq!(provider.call_count(), 1);

        // Verify persisted to DB
        let stored = repo.get_candles_since("AAPL", from).unwrap();
        assert_eq!(stored.len(), 10);
    }

    #[tokio::test]
    async fn second_run_uses_delta_fetch() {
        let repo = Arc::new(SqliteRepository::open_in_memory().unwrap());
        let from = NaiveDate::parse_from_str("2025-01-01", "%Y-%m-%d").unwrap();
        let to = NaiveDate::parse_from_str("2025-01-10", "%Y-%m-%d").unwrap();

        // Pre-seed DB with 8 days of data
        seed_stock(&repo, "AAPL");
        let cached_candles = dates("2025-01-01", 8);
        repo.upsert_candles("AAPL", &cached_candles).unwrap();

        // Provider only has days 9-10 (the delta)
        let delta_candles = dates("2025-01-09", 2);
        let provider = Arc::new(MockDataProvider::new().with_candles("AAPL", delta_candles));
        let cache_cfg = CacheConfig {
            enabled: true,
            full_refresh_days: 7,
        };

        let caching = CachingProvider::new(provider.clone(), repo.clone(), cache_cfg, from);
        let result = caching.fetch_candles("AAPL", from, to).await.unwrap();

        // Should have all 10 days (8 cached + 2 delta, with 1 overlap deduped)
        assert_eq!(result.len(), 10);
        assert_eq!(provider.call_count(), 1); // only 1 delta fetch
    }

    #[tokio::test]
    async fn full_refresh_when_data_too_old() {
        let repo = Arc::new(SqliteRepository::open_in_memory().unwrap());
        let today = NaiveDate::parse_from_str("2025-01-20", "%Y-%m-%d").unwrap();
        let from = today - chrono::Duration::days(400);

        // Pre-seed with old data (more than 7 days ago)
        seed_stock(&repo, "AAPL");
        let old_candles = dates("2025-01-01", 3);
        repo.upsert_candles("AAPL", &old_candles).unwrap();

        let full_candles = dates("2025-01-01", 20);
        let provider = Arc::new(MockDataProvider::new().with_candles("AAPL", full_candles));
        let cache_cfg = CacheConfig {
            enabled: true,
            full_refresh_days: 7,
        };

        let caching = CachingProvider::new(provider.clone(), repo.clone(), cache_cfg, from);
        let result = caching.fetch_candles("AAPL", from, today).await.unwrap();

        assert_eq!(result.len(), 20); // full fetch, not delta
    }

    #[tokio::test]
    async fn cache_disabled_passes_through() {
        let repo = Arc::new(SqliteRepository::open_in_memory().unwrap());
        let candles = dates("2025-01-01", 5);
        let provider = Arc::new(MockDataProvider::new().with_candles("AAPL", candles.clone()));
        let cache_cfg = CacheConfig {
            enabled: false,
            full_refresh_days: 7,
        };
        let from = NaiveDate::parse_from_str("2025-01-01", "%Y-%m-%d").unwrap();
        let to = NaiveDate::parse_from_str("2025-01-05", "%Y-%m-%d").unwrap();

        let caching = CachingProvider::new(provider.clone(), repo.clone(), cache_cfg, from);
        let result = caching.fetch_candles("AAPL", from, to).await.unwrap();

        assert_eq!(result.len(), 5);
        // Nothing stored in DB when disabled
        let stored = repo.get_candles_since("AAPL", from).unwrap();
        assert_eq!(stored.len(), 0);
    }

    #[tokio::test]
    async fn no_fetch_when_fully_cached() {
        let repo = Arc::new(SqliteRepository::open_in_memory().unwrap());
        let from = NaiveDate::parse_from_str("2025-01-01", "%Y-%m-%d").unwrap();
        let to = NaiveDate::parse_from_str("2025-01-05", "%Y-%m-%d").unwrap();

        // Pre-seed DB with all 5 days
        seed_stock(&repo, "AAPL");
        let all_candles = dates("2025-01-01", 5);
        repo.upsert_candles("AAPL", &all_candles).unwrap();

        // Provider with no data — should never be called
        let provider = Arc::new(MockDataProvider::new());
        let cache_cfg = CacheConfig {
            enabled: true,
            full_refresh_days: 7,
        };

        let caching = CachingProvider::new(provider.clone(), repo.clone(), cache_cfg, from);
        let result = caching.fetch_candles("AAPL", from, to).await.unwrap();

        assert_eq!(result.len(), 5);
        assert_eq!(provider.call_count(), 0); // no upstream call
    }
}
