//! Domain types and `FundamentalProvider` trait for financial metrics and earnings data.
//!
//! Provides:
//! - `FundamentalSnapshot` — point-in-time financial ratios
//! - `EarningsRecord` — single-quarter earnings
//! - `EarningsQuality` — summary for scoring
//! - `SectorPeBenchmark` — sector-level valuation benchmarks
//! - `FundamentalProvider` trait with Finnhub implementation

use async_trait::async_trait;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::error::DataError;

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// Point-in-time snapshot of financial ratios for a single ticker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundamentalSnapshot {
    pub ticker: String,
    pub fetched_date: NaiveDate,
    pub pe_normalized: Option<f64>,
    pub pb_annual: Option<f64>,
    pub ps_annual: Option<f64>,
    pub eps_growth_ttm_yoy: Option<f64>,
    pub rev_growth_ttm_yoy: Option<f64>,
    pub eps_growth_qoq: Option<f64>,
    pub rev_growth_qoq: Option<f64>,
    pub gross_margin_ttm: Option<f64>,
    pub net_margin_ttm: Option<f64>,
    pub roe_ttm: Option<f64>,
    pub roi_ttm: Option<f64>,
    pub debt_equity: Option<f64>,
    pub current_ratio: Option<f64>,
    pub fundamental_score: Option<f64>,
}

/// Single-quarter earnings record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EarningsRecord {
    pub ticker: String,
    pub period_date: NaiveDate,
    pub fiscal_year: i32,
    pub fiscal_quarter: u8,
    pub eps_actual: Option<f64>,
    pub eps_estimate: Option<f64>,
    pub eps_surprise: Option<f64>,
    pub eps_surprise_pct: Option<f64>,
    pub is_upcoming: bool,
    pub earnings_date: Option<NaiveDate>,
}

/// Summary of recent earnings quality for scoring.
#[derive(Debug, Clone, Default)]
pub struct EarningsQuality {
    pub latest_surprise_pct: Option<f64>,
    pub consecutive_beats: u8,
    pub avg_surprise_pct_4q: Option<f64>,
    pub eps_growth_yoy: Option<f64>,
    pub rev_growth_yoy: Option<f64>,
    pub days_to_next_earnings: Option<i64>,
}

/// A single quarterly fundamental data point for time-series analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundamentalHistoryRecord {
    pub ticker: String,
    pub period_date: String, // YYYY-MM-DD (quarter end date)
    pub period_type: String, // "Q1", "Q2", "Q3", "Q4", "FY"
    pub pe_ratio: Option<f64>,
    pub pb_ratio: Option<f64>,
    pub ps_ratio: Option<f64>,
    pub ev_ebitda: Option<f64>,
    pub ev_revenue: Option<f64>,
    pub gross_margin: Option<f64>,
    pub operating_margin: Option<f64>,
    pub net_margin: Option<f64>,
    pub roe: Option<f64>,
    pub debt_to_equity: Option<f64>,
    pub revenue: Option<f64>,
    pub ebitda: Option<f64>,
    pub fcf: Option<f64>,
}

/// Sector P/E benchmark entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SectorPeBenchmark {
    pub sector: String,
    pub snapshot_date: NaiveDate,
    pub median_pe: Option<f64>,
    pub median_ps: Option<f64>,
    pub stock_count: u32,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Async provider for fundamental financial data and earnings calendars.
#[async_trait]
pub trait FundamentalProvider: Send + Sync {
    /// Fetch a point-in-time snapshot of financial ratios for `ticker`.
    async fn fetch_fundamentals(&self, ticker: &str) -> Result<FundamentalSnapshot, DataError>;

    /// Fetch historical quarterly earnings for `ticker`.
    async fn fetch_earnings(&self, ticker: &str) -> Result<Vec<EarningsRecord>, DataError>;

    /// Fetch earnings calendar entries between `from` and `to` (inclusive).
    async fn fetch_earnings_calendar(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<EarningsRecord>, DataError>;
}

// ---------------------------------------------------------------------------
// Finnhub JSON deserialization types (private)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct FinnhubMetricResponse {
    metric: FinnhubMetricFields,
}

#[derive(Debug, Deserialize)]
struct FinnhubMetricFields {
    #[serde(rename = "peNormalizedAnnual")]
    pe_normalized_annual: Option<f64>,
    #[serde(rename = "pbAnnual")]
    pb_annual: Option<f64>,
    #[serde(rename = "psAnnual")]
    ps_annual: Option<f64>,
    #[serde(rename = "epsGrowthTTMYoy")]
    eps_growth_ttm_yoy: Option<f64>,
    #[serde(rename = "revenueGrowthTTMYoy")]
    revenue_growth_ttm_yoy: Option<f64>,
    #[serde(rename = "epsGrowthQuarterlyYoy")]
    eps_growth_quarterly_yoy: Option<f64>,
    #[serde(rename = "revenueGrowthQuarterlyYoy")]
    revenue_growth_quarterly_yoy: Option<f64>,
    #[serde(rename = "grossMarginTTM")]
    gross_margin_ttm: Option<f64>,
    #[serde(rename = "netMarginTTM")]
    net_margin_ttm: Option<f64>,
    #[serde(rename = "roeTTM")]
    roe_ttm: Option<f64>,
    #[serde(rename = "roiTTM")]
    roi_ttm: Option<f64>,
    #[serde(rename = "debtEquityAnnual")]
    debt_equity_annual: Option<f64>,
    #[serde(rename = "currentRatioAnnual")]
    current_ratio_annual: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct FinnhubEarningsResponse {
    #[serde(rename = "earningsCalendar")]
    earnings_calendar: Vec<FinnhubEarningsItem>,
}

#[derive(Debug, Deserialize)]
struct FinnhubEarningsItem {
    #[serde(default)]
    actual: Option<f64>,
    #[serde(default)]
    estimate: Option<f64>,
    #[serde(default)]
    period: String,
    #[serde(default)]
    quarter: u8,
    #[serde(default, rename = "surprisePercent")]
    surprise_percent: Option<f64>,
    #[serde(default)]
    symbol: String,
    #[serde(default)]
    year: i32,
    #[serde(default)]
    date: Option<String>,
}

// ---------------------------------------------------------------------------
// Finnhub implementation
// ---------------------------------------------------------------------------

use crate::data::fetcher::FinnhubClient;

#[async_trait]
impl FundamentalProvider for FinnhubClient {
    async fn fetch_fundamentals(&self, ticker: &str) -> Result<FundamentalSnapshot, DataError> {
        let url = format!(
            "{}/stock/metric?symbol={}&metric=all&token={}",
            self.base_url, ticker, self.api_key
        );
        let body = self.get_with_retry(&url).await?;
        let resp: FinnhubMetricResponse =
            serde_json::from_str(&body).map_err(|e| DataError::Parse(e.to_string()))?;

        let m = &resp.metric;
        Ok(FundamentalSnapshot {
            ticker: ticker.to_string(),
            fetched_date: chrono::Utc::now().date_naive(),
            pe_normalized: m.pe_normalized_annual,
            pb_annual: m.pb_annual,
            ps_annual: m.ps_annual,
            eps_growth_ttm_yoy: m.eps_growth_ttm_yoy,
            rev_growth_ttm_yoy: m.revenue_growth_ttm_yoy,
            eps_growth_qoq: m.eps_growth_quarterly_yoy,
            rev_growth_qoq: m.revenue_growth_quarterly_yoy,
            gross_margin_ttm: m.gross_margin_ttm,
            net_margin_ttm: m.net_margin_ttm,
            roe_ttm: m.roe_ttm,
            roi_ttm: m.roi_ttm,
            debt_equity: m.debt_equity_annual,
            current_ratio: m.current_ratio_annual,
            fundamental_score: None,
        })
    }

    async fn fetch_earnings(&self, ticker: &str) -> Result<Vec<EarningsRecord>, DataError> {
        let url = format!(
            "{}/stock/earnings?symbol={}&token={}",
            self.base_url, ticker, self.api_key
        );
        let body = self.get_with_retry(&url).await?;
        let items: Vec<FinnhubEarningsItem> =
            serde_json::from_str(&body).map_err(|e| DataError::Parse(e.to_string()))?;

        let records = items
            .into_iter()
            .filter_map(|item| {
                let period_date = NaiveDate::parse_from_str(&item.period, "%Y-%m-%d").ok()?;
                let eps_surprise = match (item.actual, item.estimate) {
                    (Some(a), Some(e)) => Some(a - e),
                    _ => None,
                };
                Some(EarningsRecord {
                    ticker: ticker.to_string(),
                    period_date,
                    fiscal_year: item.year,
                    fiscal_quarter: item.quarter,
                    eps_actual: item.actual,
                    eps_estimate: item.estimate,
                    eps_surprise,
                    eps_surprise_pct: item.surprise_percent,
                    is_upcoming: false,
                    earnings_date: None,
                })
            })
            .collect();

        Ok(records)
    }

    async fn fetch_earnings_calendar(
        &self,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<EarningsRecord>, DataError> {
        let url = format!(
            "{}/calendar/earnings?from={}&to={}&token={}",
            self.base_url, from, to, self.api_key
        );
        let body = self.get_with_retry(&url).await?;
        let resp: FinnhubEarningsResponse =
            serde_json::from_str(&body).map_err(|e| DataError::Parse(e.to_string()))?;

        let records = resp
            .earnings_calendar
            .into_iter()
            .filter_map(|item| {
                let period_date = NaiveDate::parse_from_str(&item.period, "%Y-%m-%d").ok()?;
                let earnings_date = item
                    .date
                    .as_deref()
                    .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
                let eps_surprise = match (item.actual, item.estimate) {
                    (Some(a), Some(e)) => Some(a - e),
                    _ => None,
                };
                Some(EarningsRecord {
                    ticker: item.symbol,
                    period_date,
                    fiscal_year: item.year,
                    fiscal_quarter: item.quarter,
                    eps_actual: item.actual,
                    eps_estimate: item.estimate,
                    eps_surprise,
                    eps_surprise_pct: item.surprise_percent,
                    is_upcoming: true,
                    earnings_date,
                })
            })
            .collect();

        Ok(records)
    }
}

// ---------------------------------------------------------------------------
// Mock provider (available in tests throughout the crate)
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "test-support"))]
pub mod mock {
    use super::*;

    #[derive(Default)]
    pub struct MockFundamentalProvider {
        pub snapshots: std::collections::HashMap<String, FundamentalSnapshot>,
        pub earnings: std::collections::HashMap<String, Vec<EarningsRecord>>,
    }

    impl MockFundamentalProvider {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl FundamentalProvider for MockFundamentalProvider {
        async fn fetch_fundamentals(&self, ticker: &str) -> Result<FundamentalSnapshot, DataError> {
            self.snapshots
                .get(ticker)
                .cloned()
                .ok_or(DataError::NotFound {
                    ticker: ticker.to_string(),
                })
        }

        async fn fetch_earnings(&self, ticker: &str) -> Result<Vec<EarningsRecord>, DataError> {
            Ok(self.earnings.get(ticker).cloned().unwrap_or_default())
        }

        async fn fetch_earnings_calendar(
            &self,
            _from: NaiveDate,
            _to: NaiveDate,
        ) -> Result<Vec<EarningsRecord>, DataError> {
            Ok(vec![])
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_finnhub_metric_response() {
        let json = r#"{
            "metric": {
                "peNormalizedAnnual": 25.3,
                "pbAnnual": 4.1,
                "psAnnual": 6.7,
                "epsGrowthTTMYoy": 12.5,
                "revenueGrowthTTMYoy": 8.3,
                "epsGrowthQuarterlyYoy": 15.0,
                "revenueGrowthQuarterlyYoy": 10.2,
                "grossMarginTTM": 45.6,
                "netMarginTTM": 22.1,
                "roeTTM": 30.5,
                "roiTTM": 18.9,
                "debtEquityAnnual": 0.45,
                "currentRatioAnnual": 2.1
            }
        }"#;

        let resp: FinnhubMetricResponse = serde_json::from_str(json).unwrap();
        assert!((resp.metric.pe_normalized_annual.unwrap() - 25.3).abs() < f64::EPSILON);
        assert!((resp.metric.pb_annual.unwrap() - 4.1).abs() < f64::EPSILON);
        assert!((resp.metric.gross_margin_ttm.unwrap() - 45.6).abs() < f64::EPSILON);
        assert!((resp.metric.debt_equity_annual.unwrap() - 0.45).abs() < f64::EPSILON);
    }

    #[test]
    fn deserialize_finnhub_metric_response_with_nulls() {
        let json = r#"{
            "metric": {
                "peNormalizedAnnual": null,
                "pbAnnual": 4.1
            }
        }"#;

        let resp: FinnhubMetricResponse = serde_json::from_str(json).unwrap();
        assert!(resp.metric.pe_normalized_annual.is_none());
        assert!(resp.metric.pb_annual.is_some());
        assert!(resp.metric.eps_growth_ttm_yoy.is_none());
    }

    #[test]
    fn deserialize_finnhub_earnings_items() {
        let json = r#"[
            {
                "actual": 2.10,
                "estimate": 1.89,
                "period": "2024-09-30",
                "quarter": 3,
                "surprisePercent": 11.11,
                "symbol": "AAPL",
                "year": 2024
            },
            {
                "actual": 1.53,
                "estimate": 1.50,
                "period": "2024-06-30",
                "quarter": 2,
                "surprisePercent": 2.0,
                "symbol": "AAPL",
                "year": 2024
            }
        ]"#;

        let items: Vec<FinnhubEarningsItem> = serde_json::from_str(json).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].symbol, "AAPL");
        assert_eq!(items[0].quarter, 3);
        assert!((items[0].actual.unwrap() - 2.10).abs() < f64::EPSILON);
        assert!((items[0].surprise_percent.unwrap() - 11.11).abs() < f64::EPSILON);
        assert_eq!(items[1].period, "2024-06-30");
    }

    #[test]
    fn deserialize_finnhub_earnings_calendar_response() {
        let json = r#"{
            "earningsCalendar": [
                {
                    "date": "2025-01-28",
                    "actual": null,
                    "estimate": 2.35,
                    "period": "2024-12-31",
                    "quarter": 4,
                    "surprisePercent": null,
                    "symbol": "MSFT",
                    "year": 2024
                }
            ]
        }"#;

        let resp: FinnhubEarningsResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.earnings_calendar.len(), 1);
        let item = &resp.earnings_calendar[0];
        assert_eq!(item.symbol, "MSFT");
        assert!(item.actual.is_none());
        assert!((item.estimate.unwrap() - 2.35).abs() < f64::EPSILON);
        assert_eq!(item.date.as_deref(), Some("2025-01-28"));
    }

    #[test]
    fn earnings_quality_default_values() {
        let eq = EarningsQuality::default();
        assert!(eq.latest_surprise_pct.is_none());
        assert_eq!(eq.consecutive_beats, 0);
        assert!(eq.avg_surprise_pct_4q.is_none());
        assert!(eq.eps_growth_yoy.is_none());
        assert!(eq.rev_growth_yoy.is_none());
        assert!(eq.days_to_next_earnings.is_none());
    }

    // ── FundamentalHistoryRecord tests ───────────────────────────────────

    #[test]
    fn test_fundamental_history_record_default_fields() {
        let record = FundamentalHistoryRecord {
            ticker: "AAPL".to_string(),
            period_date: "2025-03-31".to_string(),
            period_type: "Q1".to_string(),
            pe_ratio: None,
            pb_ratio: None,
            ps_ratio: None,
            ev_ebitda: None,
            ev_revenue: None,
            gross_margin: None,
            operating_margin: None,
            net_margin: None,
            roe: None,
            debt_to_equity: None,
            revenue: None,
            ebitda: None,
            fcf: None,
        };
        assert_eq!(record.ticker, "AAPL");
        assert_eq!(record.period_date, "2025-03-31");
        assert_eq!(record.period_type, "Q1");
        assert!(record.pe_ratio.is_none());
        assert!(record.pb_ratio.is_none());
        assert!(record.ps_ratio.is_none());
        assert!(record.ev_ebitda.is_none());
        assert!(record.ev_revenue.is_none());
        assert!(record.gross_margin.is_none());
        assert!(record.operating_margin.is_none());
        assert!(record.net_margin.is_none());
        assert!(record.roe.is_none());
        assert!(record.debt_to_equity.is_none());
        assert!(record.revenue.is_none());
        assert!(record.ebitda.is_none());
        assert!(record.fcf.is_none());
    }

    #[test]
    fn test_fundamental_history_record_serializes() {
        let record = FundamentalHistoryRecord {
            ticker: "MSFT".to_string(),
            period_date: "2025-06-30".to_string(),
            period_type: "Q2".to_string(),
            pe_ratio: Some(28.5),
            pb_ratio: Some(12.3),
            ps_ratio: Some(10.1),
            ev_ebitda: Some(20.0),
            ev_revenue: Some(15.0),
            gross_margin: Some(0.69),
            operating_margin: Some(0.42),
            net_margin: Some(0.36),
            roe: Some(0.40),
            debt_to_equity: Some(0.35),
            revenue: Some(56_000_000_000.0),
            ebitda: Some(24_000_000_000.0),
            fcf: Some(18_000_000_000.0),
        };
        let json = serde_json::to_string(&record).unwrap();
        let deserialized: FundamentalHistoryRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.ticker, "MSFT");
        assert_eq!(deserialized.period_type, "Q2");
        assert!((deserialized.pe_ratio.unwrap() - 28.5).abs() < f64::EPSILON);
        assert!((deserialized.gross_margin.unwrap() - 0.69).abs() < f64::EPSILON);
        assert!((deserialized.revenue.unwrap() - 56_000_000_000.0).abs() < f64::EPSILON);
    }
}
