# Plan 08: Yahoo Finance Data Provider

**Effort:** Medium (3-4 days)
**Impact:** Very High
**Priority:** 8

## Problem

Ghost Alpha requires a Finnhub API key to function. While Finnhub's free tier is generous (60 req/min), it still requires signup and creates a barrier to entry. More importantly, it limits the universe to what Finnhub covers and makes the tool dependent on a single provider.

The `DataProvider` trait already abstracts the data layer — but Finnhub is the only implementation.

## Solution

Add a Yahoo Finance provider as a zero-signup alternative. Yahoo Finance covers global equities, ETFs, and indices with no API key required. This also validates the plugin architecture and demonstrates provider extensibility.

## Implementation

### 1. Yahoo Finance client

Create `src/data/yahoo.rs`:

Yahoo Finance's v8 API endpoint provides OHLCV data:
```
https://query1.finance.yahoo.com/v8/finance/chart/AAPL?range=2y&interval=1d
```

```rust
pub struct YahooProvider {
    client: reqwest::Client,
    rate_limiter: RateLimiter,  // Be polite: 5 req/sec
}

#[async_trait]
impl DataProvider for YahooProvider {
    async fn fetch_candles(&self, ticker: &str, days: u32) -> Result<Vec<Candle>> {
        let range = days_to_range(days);  // "1y", "2y", "5y"
        let url = format!(
            "https://query1.finance.yahoo.com/v8/finance/chart/{}?range={}&interval=1d",
            ticker, range
        );
        let resp: YahooChartResponse = self.client
            .get(&url)
            .header("User-Agent", "Mozilla/5.0")  // Required
            .send()
            .await?
            .json()
            .await?;
        Ok(parse_yahoo_candles(resp))
    }

    async fn fetch_universe(&self) -> Result<Vec<String>> {
        // Use S&P 500 constituents from Wikipedia or a bundled list
        Ok(SP500_TICKERS.to_vec())
    }

    async fn fetch_news(&self, _ticker: &str) -> Result<Vec<NewsArticle>> {
        // Yahoo news endpoint (limited)
        // Fallback: return empty vec (sentiment disabled)
        Ok(vec![])
    }

    async fn fetch_spy_candles(&self, days: u32) -> Result<Vec<Candle>> {
        self.fetch_candles("SPY", days).await
    }

    fn name(&self) -> &'static str {
        "yahoo"
    }
}
```

### 2. Response parsing

Yahoo's JSON response structure:

```rust
#[derive(Deserialize)]
struct YahooChartResponse {
    chart: YahooChart,
}

#[derive(Deserialize)]
struct YahooChart {
    result: Vec<YahooResult>,
}

#[derive(Deserialize)]
struct YahooResult {
    timestamp: Vec<i64>,
    indicators: YahooIndicators,
}

#[derive(Deserialize)]
struct YahooIndicators {
    quote: Vec<YahooQuote>,
    adjclose: Option<Vec<YahooAdjClose>>,
}

fn parse_yahoo_candles(resp: YahooChartResponse) -> Vec<Candle> {
    let result = &resp.chart.result[0];
    let quote = &result.indicators.quote[0];
    let adjclose = result.indicators.adjclose.as_ref().map(|a| &a[0].adjclose);

    result.timestamp.iter().enumerate().filter_map(|(i, &ts)| {
        Some(Candle {
            date: DateTime::from_timestamp(ts, 0)?.format("%Y-%m-%d").to_string(),
            open: quote.open[i]?,
            high: quote.high[i]?,
            low: quote.low[i]?,
            close: adjclose.and_then(|a| a[i]).unwrap_or(quote.close[i]?),
            volume: quote.volume[i]? as f64,
        })
    }).collect()
}
```

### 3. Provider selection

Update config and CLI:

```toml
[plugins]
active_provider = "yahoo"  # or "finnhub" (default)
```

```bash
# Use Yahoo (no API key needed)
ghost-alpha screen --provider yahoo

# Use Finnhub (default)
ghost-alpha screen --provider finnhub
```

### 4. Plugin registry integration

In `src/plugins/registry.rs`:

```rust
pub fn default_registry(config: &AppConfig) -> Registry {
    let mut registry = Registry::new();

    // Always register Yahoo (no config needed)
    registry.register_provider("yahoo", YahooProvider::new());

    // Register Finnhub if API key is available
    if !config.finnhub.api_key.trim().is_empty() {
        registry.register_provider("finnhub", FinnhubClient::new(...));
    }

    // ...
}
```

### 5. Bundled S&P 500 ticker list

Since Yahoo doesn't have a universe endpoint, bundle a list:

```rust
// src/data/universes.rs
pub const SP500: &[&str] = &[
    "AAPL", "ABBV", "ABT", "ACN", "ADBE", /* ... 498 more ... */
];

pub const RUSSELL_1000: &[&str] = &[/* ... */];
```

Update periodically via a script. The bundled list is a fallback — users can also specify `universe = [...]` in config.

### 6. Rate limiting

Yahoo is unofficial and will block aggressive scrapers:
- Default: 5 requests/second (vs Finnhub's 1/sec)
- Respect 429 responses with exponential backoff
- Add `User-Agent` header (required)
- Consider caching aggressively (SQLite) to minimize requests

## Limitations vs Finnhub

| Feature | Finnhub | Yahoo |
|---------|---------|-------|
| API key required | Yes | No |
| Rate limit | 60/min (free) | ~5/sec (unofficial) |
| News headlines | Yes | Limited |
| Earnings calendar | Yes | No |
| S&P 500 constituents | Yes (API) | No (bundled list) |
| Reliability | SLA-backed | No guarantees |
| Adjusted close | Yes | Yes |

**Recommendation:** Yahoo as the getting-started provider, Finnhub for production daily use.

## Files to create/modify

| File | Action |
|------|--------|
| `src/data/yahoo.rs` | Create — Yahoo Finance provider |
| `src/data/universes.rs` | Create — bundled ticker lists |
| `src/data/mod.rs` | Modify — add `pub mod yahoo`, `pub mod universes` |
| `src/plugins/registry.rs` | Modify — register Yahoo provider |
| `src/main.rs` | Modify — add `--provider` flag |
| `src/config.rs` | Modify — make `finnhub.api_key` optional when provider=yahoo |
| `scripts/update_sp500.py` | Create — script to refresh bundled ticker list |

## Testing

- Unit test: Parse real Yahoo JSON response fixture
- Unit test: Handle missing data points (null OHLCV values)
- Unit test: Date conversion from Unix timestamps
- Integration test: Fetch AAPL candles (live, gated behind `--features integration`)
- Integration test: Full screen pipeline with Yahoo provider

## Acceptance criteria

- [ ] `ghost-alpha screen --provider yahoo` works with zero config
- [ ] No API key required for Yahoo provider
- [ ] Adjusted close prices used for all MA calculations
- [ ] Rate limiting prevents Yahoo blocks
- [ ] Bundled S&P 500 list as fallback universe
- [ ] Graceful degradation: news/sentiment skipped when Yahoo is provider
- [ ] Can switch between providers via config or CLI flag
