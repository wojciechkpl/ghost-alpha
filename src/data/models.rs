use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// Single OHLCV bar. Use `adj_close` for all MA and RS calculations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candle {
    pub date: NaiveDate,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub adj_close: f64,
    pub volume: u64,
}

impl Candle {
    pub fn new(date: &str, o: f64, h: f64, l: f64, c: f64, v: u64) -> Self {
        Self {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            open: o,
            high: h,
            low: l,
            close: c,
            adj_close: c,
            volume: v,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stock {
    pub ticker: String,
    pub name: String,
    pub sector: String,
    pub industry: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsArticle {
    pub ticker: String,
    pub headline: String,
    pub summary: String,
    pub published_at: chrono::DateTime<chrono::Utc>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MarketRegime {
    Bull,
    Sideways,
    Bear,
}

impl std::fmt::Display for MarketRegime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MarketRegime::Bull => write!(f, "BULL"),
            MarketRegime::Sideways => write!(f, "SIDEWAYS"),
            MarketRegime::Bear => write!(f, "BEAR"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegimeState {
    pub date: NaiveDate,
    pub spy_close: f64,
    pub spy_sma200: f64,
    pub breadth_pct: f64,
    pub regime: MarketRegime,
}

/// A stock flagged by the screener for ongoing monitoring / exit alerts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlaggedStock {
    pub id: Option<i64>,
    pub ticker: String,
    pub flagged_date: NaiveDate,
    pub entry_score: f64,
    pub entry_price: f64,
    pub conditions_at_entry: u8,
    pub status: FlaggedStatus,
    pub exit_date: Option<NaiveDate>,
    pub exit_reason: Option<String>,
    pub exit_price: Option<f64>,
    /// Most recent composite score (for conviction decay tracking)
    pub latest_score: Option<f64>,
    /// Date of last re-scoring
    pub last_rescored_at: Option<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlaggedStatus {
    Active,
    Exited,
    Expired,
}

impl std::fmt::Display for FlaggedStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlaggedStatus::Active => write!(f, "ACTIVE"),
            FlaggedStatus::Exited => write!(f, "EXITED"),
            FlaggedStatus::Expired => write!(f, "EXPIRED"),
        }
    }
}

/// A single row from the `watchlist` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchlistEntry {
    pub ticker: String,
    pub added_at: String,
    pub notes: Option<String>,
    pub last_score: Option<f64>,
    pub last_grade: Option<String>,
    pub last_screen: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Severity {
    Critical,
    Warning,
    Info,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Critical => write!(f, "CRITICAL"),
            Severity::Warning => write!(f, "WARNING"),
            Severity::Info => write!(f, "INFO"),
        }
    }
}
