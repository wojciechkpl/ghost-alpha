//! Multi-asset data fetching and return calculations.
//!
//! Uses [`DataProvider`] to fetch historical candles for asset-class ETF proxies,
//! then computes daily returns, annualised return, and annualised volatility.

use std::collections::HashMap;

use chrono::NaiveDate;

use crate::data::models::Candle;
use crate::data::provider::DataProvider;
use crate::error::DataError;

/// Fetch historical candles for all asset-class ETF proxies.
///
/// Returns a map of ticker to candles. Tickers that fail to fetch are logged
/// and skipped (partial results are still useful for diversification analysis).
pub async fn fetch_asset_class_candles(
    provider: &dyn DataProvider,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<HashMap<String, Vec<Candle>>, DataError> {
    let proxies = super::universes::asset_class_proxies();
    let mut result = HashMap::with_capacity(proxies.len());
    for proxy in &proxies {
        match provider.fetch_candles(proxy.ticker, from, to).await {
            Ok(candles) => {
                result.insert(proxy.ticker.to_string(), candles);
            }
            Err(e) => {
                tracing::warn!(ticker = proxy.ticker, error = %e, "Skipping asset class proxy");
            }
        }
    }
    Ok(result)
}

/// Compute daily simple returns from a slice of candles.
///
/// Uses `adj_close` for split/dividend-adjusted calculations.
/// Returns `n-1` returns for `n` candles; empty input yields empty output.
pub fn daily_returns(candles: &[Candle]) -> Vec<f64> {
    candles
        .windows(2)
        .map(|w| (w[1].adj_close - w[0].adj_close) / w[0].adj_close)
        .collect()
}

/// Compute annualised return from a series of daily returns.
///
/// Uses geometric compounding: `(product(1 + r_i))^(252/n) - 1`.
/// Returns `0.0` for empty input.
pub fn annualised_return(daily_returns: &[f64]) -> f64 {
    if daily_returns.is_empty() {
        return 0.0;
    }
    let product: f64 = daily_returns.iter().map(|r| 1.0 + r).product();
    let trading_days = daily_returns.len() as f64;
    product.powf(252.0 / trading_days) - 1.0
}

/// Compute annualised volatility from daily returns.
///
/// Uses sample standard deviation (Bessel-corrected) scaled by `sqrt(252)`.
/// Returns `0.0` if fewer than 2 returns are provided.
pub fn annualised_volatility(daily_returns: &[f64]) -> f64 {
    if daily_returns.len() < 2 {
        return 0.0;
    }
    let n = daily_returns.len() as f64;
    let mean = daily_returns.iter().sum::<f64>() / n;
    let variance = daily_returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
    variance.sqrt() * 252.0_f64.sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::models::Candle;
    use crate::data::provider::mock::MockDataProvider;
    use chrono::NaiveDate;

    fn make_candle(date: &str, adj_close: f64) -> Candle {
        Candle {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            open: adj_close,
            high: adj_close,
            low: adj_close,
            close: adj_close,
            adj_close,
            volume: 1_000_000,
        }
    }

    // ---- daily_returns ----

    #[test]
    fn daily_returns_simple() {
        let candles = vec![
            make_candle("2024-01-02", 100.0),
            make_candle("2024-01-03", 110.0),
            make_candle("2024-01-04", 105.0),
        ];
        let rets = daily_returns(&candles);
        assert_eq!(rets.len(), 2);
        assert!((rets[0] - 0.1).abs() < 1e-10);
        assert!((rets[1] - (-5.0 / 110.0)).abs() < 1e-10);
    }

    #[test]
    fn daily_returns_empty() {
        let rets = daily_returns(&[]);
        assert!(rets.is_empty());
    }

    #[test]
    fn daily_returns_single() {
        let candles = vec![make_candle("2024-01-02", 100.0)];
        let rets = daily_returns(&candles);
        assert!(rets.is_empty());
    }

    // ---- annualised_return ----

    #[test]
    fn annualised_return_flat() {
        let rets = vec![0.0; 252];
        assert!((annualised_return(&rets)).abs() < 1e-10);
    }

    #[test]
    fn annualised_return_positive() {
        // 252 days of 0.1% daily return
        let daily = 0.001;
        let rets = vec![daily; 252];
        let ann = annualised_return(&rets);
        // Expected: (1.001)^252 - 1 ≈ 0.2862
        let expected = (1.0 + daily).powi(252) - 1.0;
        assert!(
            (ann - expected).abs() < 1e-8,
            "Expected {expected}, got {ann}"
        );
    }

    #[test]
    fn annualised_return_empty() {
        assert!((annualised_return(&[])).abs() < 1e-10);
    }

    // ---- annualised_volatility ----

    #[test]
    fn annualised_volatility_zero() {
        // Constant returns → zero volatility
        let rets = vec![0.01; 100];
        assert!((annualised_volatility(&rets)).abs() < 1e-10);
    }

    #[test]
    fn annualised_volatility_positive() {
        let rets = vec![0.01, -0.01, 0.02, -0.02, 0.01];
        let vol = annualised_volatility(&rets);
        // Should be positive and reasonable (roughly 15-25% annualised for these swings)
        assert!(vol > 0.0, "Volatility should be positive");
        assert!(vol < 1.0, "Volatility should be less than 100%");
    }

    #[test]
    fn annualised_volatility_single_return() {
        let rets = vec![0.05];
        assert!((annualised_volatility(&rets)).abs() < 1e-10);
    }

    #[test]
    fn annualised_volatility_empty() {
        assert!((annualised_volatility(&[])).abs() < 1e-10);
    }

    // ---- fetch_asset_class_candles ----

    #[tokio::test]
    async fn fetch_asset_class_candles_returns_available_tickers() {
        let provider = MockDataProvider::new()
            .with_candles("VTI", vec![make_candle("2024-01-02", 200.0)])
            .with_candles("BND", vec![make_candle("2024-01-02", 75.0)]);

        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
        let result = fetch_asset_class_candles(&provider, from, to).await.unwrap();

        assert!(result.contains_key("VTI"));
        assert!(result.contains_key("BND"));
        // Tickers not in mock should be silently skipped
        assert!(!result.contains_key("MISSING"));
    }

    #[tokio::test]
    async fn fetch_asset_class_candles_partial_failure_still_returns() {
        // Only one ticker available — the rest fail silently
        let provider = MockDataProvider::new()
            .with_candles("GLD", vec![make_candle("2024-06-01", 180.0)]);

        let from = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2024, 12, 31).unwrap();
        let result = fetch_asset_class_candles(&provider, from, to).await.unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains_key("GLD"));
    }
}
