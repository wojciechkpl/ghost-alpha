use std::sync::Mutex;

use chrono::NaiveDate;
use rusqlite::{params, Connection};

use crate::alerts::channel::OutboundAlert;
use crate::analysis::indicators::LiquidityClass;
use crate::analysis::screener::ScreenResult;
use crate::data::models::{
    Candle, FlaggedStatus, FlaggedStock, MarketRegime, NewsArticle, RegimeState, Stock,
    WatchlistEntry,
};
use crate::error::StorageError;
use crate::storage::migrations::run_migrations;

// ─── Alert Log Entry ─────────────────────────────────────────────────────────

/// A row from the `alert_log` table for display purposes.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AlertLogEntry {
    pub ticker: String,
    pub alert_type: String,
    pub severity: String,
    pub message: String,
    pub sent_at: String,
}

/// A row from the `backtest_runs` table.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BacktestRunEntry {
    pub id: i64,
    pub run_date: String,
    pub params_json: String,
    pub kpis_json: String,
    pub report_json: String,
    pub signal_count: usize,
    pub duration_secs: f64,
}

// ─── Read-only summary types ─────────────────────────────────────────────────

/// Lightweight projection of a screen result row for range queries.
#[derive(Debug, Clone)]
pub struct ScreenResultSummary {
    pub ticker: String,
    pub run_date: String,
    pub composite_score: f64,
    pub sentiment_score: f64,
    pub rs_percentile: f64,
    pub weighted_rs: f64,
    pub grade: String,
}

/// Grade distribution count for a given run date.
#[derive(Debug, Clone)]
pub struct GradeCount {
    pub run_date: String,
    pub grade: String,
    pub count: u32,
}

/// Alert counts aggregated by calendar date.
#[derive(Debug, Clone)]
pub struct AlertDayCount {
    pub date: String,
    pub total: u32,
    pub critical: u32,
}

// ─── Web Dashboard Row Types ────────────────────────────────────────────────

/// A row from the `tax_lots` table for portfolio display.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaxLotRow {
    pub lot_id: String,
    pub ticker: String,
    pub shares: f64,
    pub cost_basis_per_share: f64,
    pub acquisition_date: String,
    pub acquisition_type: String,
}

/// A row from the `fundamental_metrics` table.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FundamentalMetricRow {
    pub ticker: String,
    pub fetched_date: String,
    pub pe_normalized: Option<f64>,
    pub pb_annual: Option<f64>,
    pub ps_annual: Option<f64>,
    pub eps_growth_ttm_yoy: Option<f64>,
    pub rev_growth_ttm_yoy: Option<f64>,
    pub gross_margin_ttm: Option<f64>,
    pub net_margin_ttm: Option<f64>,
    pub roe_ttm: Option<f64>,
    pub fundamental_score: Option<f64>,
}

/// A row from the `earnings_history` table.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EarningsRow {
    pub ticker: String,
    pub period_date: String,
    pub fiscal_year: i32,
    pub fiscal_quarter: i32,
    pub eps_actual: Option<f64>,
    pub eps_estimate: Option<f64>,
    pub eps_surprise_pct: Option<f64>,
    pub is_upcoming: bool,
    pub earnings_date: Option<String>,
}

/// A row from the `sector_pe_benchmarks` table.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SectorPeRow {
    pub sector: String,
    pub snapshot_date: String,
    pub median_pe: Option<f64>,
    pub median_ps: Option<f64>,
    pub stock_count: i32,
}

// ─── Repository trait ─────────────────────────────────────────────────────────

/// Synchronous repository — wrap calls in `spawn_blocking` from async contexts.
pub trait Repository: Send + Sync {
    // --- Stocks ---
    fn upsert_stock(&self, stock: &Stock) -> Result<(), StorageError>;
    fn get_stocks(&self) -> Result<Vec<Stock>, StorageError>;

    // --- Candles ---
    fn upsert_candles(&self, ticker: &str, candles: &[Candle]) -> Result<(), StorageError>;
    fn get_candles(&self, ticker: &str, limit: usize) -> Result<Vec<Candle>, StorageError>;

    // --- Screen results ---
    fn save_screen_result(
        &self,
        run_date: NaiveDate,
        result: &ScreenResult,
    ) -> Result<(), StorageError>;
    fn get_screen_results(&self, run_date: NaiveDate) -> Result<Vec<ScreenResult>, StorageError>;

    // --- Alert log ---
    fn record_alert(&self, alert: &OutboundAlert) -> Result<(), StorageError>;
    fn alert_last_sent(
        &self,
        ticker: &str,
        alert_type: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, StorageError>;

    // --- Market regime ---
    fn save_regime(&self, state: &RegimeState) -> Result<(), StorageError>;
    fn latest_regime(&self) -> Result<Option<RegimeState>, StorageError>;

    // --- Flagged stocks ---
    fn insert_flagged_stock(&self, stock: &FlaggedStock) -> Result<(), StorageError>;
    fn get_active_flagged_stocks(&self) -> Result<Vec<FlaggedStock>, StorageError>;
    fn mark_exited(
        &self,
        ticker: &str,
        exit_date: NaiveDate,
        exit_reason: &str,
        exit_price: f64,
    ) -> Result<(), StorageError>;
    fn expire_old_flags(&self, days_old: i64) -> Result<usize, StorageError>;

    // --- Macro risk ---
    fn save_macro_risk(
        &self,
        date: NaiveDate,
        snapshot: &crate::analysis::macro_risk::MacroRiskSnapshot,
    ) -> Result<(), StorageError>;
    fn latest_macro_risk(
        &self,
    ) -> Result<Option<crate::analysis::macro_risk::MacroRiskSnapshot>, StorageError>;

    // --- Flagged stock score updates ---
    fn update_flagged_score(
        &self,
        ticker: &str,
        score: f64,
        date: NaiveDate,
    ) -> Result<(), StorageError>;

    // --- Alert log queries ---
    fn get_recent_alerts(&self, limit: usize) -> Result<Vec<AlertLogEntry>, StorageError>;

    // --- News cache ---
    fn cache_news(&self, articles: &[NewsArticle]) -> Result<usize, StorageError>;
    fn get_cached_news(
        &self,
        ticker: &str,
        since: NaiveDate,
    ) -> Result<Vec<NewsArticle>, StorageError>;
    fn delete_old_news(&self, older_than_days: i64) -> Result<usize, StorageError>;

    // --- Backtest runs ---
    fn save_backtest_report(
        &self,
        params_json: &str,
        kpis_json: &str,
        report_json: &str,
        signal_count: usize,
        duration_secs: f64,
    ) -> Result<(), StorageError>;
    fn get_backtest_runs(&self, limit: usize) -> Result<Vec<BacktestRunEntry>, StorageError>;

    // --- Watchlist ---
    fn watchlist_add(&self, ticker: &str, notes: Option<&str>) -> Result<(), StorageError>;
    fn watchlist_remove(&self, ticker: &str) -> Result<bool, StorageError>;
    fn watchlist_list(&self) -> Result<Vec<WatchlistEntry>, StorageError>;
    fn watchlist_tickers(&self) -> Result<Vec<String>, StorageError>;
    fn watchlist_update_score(
        &self,
        ticker: &str,
        score: f64,
        grade: &str,
    ) -> Result<(), StorageError>;
    fn watchlist_clear(&self) -> Result<usize, StorageError>;

    // --- Read-only analytics ---
    fn get_screen_results_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<ScreenResultSummary>, StorageError>;
    fn get_regime_history(&self, limit: usize) -> Result<Vec<RegimeState>, StorageError>;
    fn get_macro_risk_history(
        &self,
        limit: usize,
    ) -> Result<Vec<crate::analysis::macro_risk::MacroRiskSnapshot>, StorageError>;
    fn get_grade_distribution(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<GradeCount>, StorageError>;
    fn get_alert_counts_by_date(&self, since: &str) -> Result<Vec<AlertDayCount>, StorageError>;
    fn get_ticker_score_history(
        &self,
        ticker: &str,
        since: &str,
    ) -> Result<Vec<(String, f64)>, StorageError>;

    // --- Decision Audit Log ---
    fn record_decision(
        &self,
        entry: &crate::portfolio::decision::DecisionLogEntry,
    ) -> Result<i64, StorageError>;
    fn get_decisions(
        &self,
        limit: usize,
    ) -> Result<Vec<crate::portfolio::decision::DecisionLogEntry>, StorageError>;
    fn get_decisions_for_ticker(
        &self,
        ticker: &str,
        limit: usize,
    ) -> Result<Vec<crate::portfolio::decision::DecisionLogEntry>, StorageError>;

    // --- Correlation Matrix ---
    fn save_correlation_snapshot(
        &self,
        date: &str,
        tickers: &[String],
        matrix: &[Vec<f64>],
        window: usize,
    ) -> Result<(), StorageError>;
    #[allow(clippy::type_complexity)]
    fn get_latest_correlation_snapshot(
        &self,
    ) -> Result<Option<(String, Vec<String>, Vec<Vec<f64>>, usize)>, StorageError>;

    // --- Fundamental History ---
    fn upsert_fundamental_history(
        &self,
        record: &crate::data::fundamentals::FundamentalHistoryRecord,
    ) -> Result<(), StorageError>;
    fn get_fundamental_history(
        &self,
        ticker: &str,
        limit: usize,
    ) -> Result<Vec<crate::data::fundamentals::FundamentalHistoryRecord>, StorageError>;
    fn get_valuation_bands(
        &self,
        ticker: &str,
        metric: &str,
        years: u32,
    ) -> Result<Vec<(String, f64)>, StorageError>;

    // --- ETF Snapshots ---
    fn upsert_etf_snapshot(
        &self,
        snapshot: &crate::data::etf::EtfSnapshot,
    ) -> Result<(), StorageError>;
    fn get_etf_snapshot(
        &self,
        ticker: &str,
    ) -> Result<Option<crate::data::etf::EtfSnapshot>, StorageError>;
    fn get_all_etf_snapshots(&self) -> Result<Vec<crate::data::etf::EtfSnapshot>, StorageError>;

    // --- Selling Programme ---
    fn save_selling_programme(
        &self,
        prog: &crate::portfolio::selling::SellingProgramme,
    ) -> Result<i64, StorageError>;
    fn get_selling_programme(
        &self,
        id: i64,
    ) -> Result<Option<crate::portfolio::selling::SellingProgramme>, StorageError>;
    fn get_selling_programmes_for_ticker(
        &self,
        ticker: &str,
    ) -> Result<Vec<crate::portfolio::selling::SellingProgramme>, StorageError>;
    fn update_selling_programme(
        &self,
        prog: &crate::portfolio::selling::SellingProgramme,
    ) -> Result<(), StorageError>;
    fn record_selling_execution(
        &self,
        exec: &crate::portfolio::selling::SellingExecution,
    ) -> Result<i64, StorageError>;
    fn get_selling_executions(
        &self,
        programme_id: i64,
    ) -> Result<Vec<crate::portfolio::selling::SellingExecution>, StorageError>;

    // --- Tax Lots (Portfolio) ---
    fn get_all_tax_lots(&self) -> Result<Vec<TaxLotRow>, StorageError>;
    fn get_tax_lots_for_ticker(&self, ticker: &str) -> Result<Vec<TaxLotRow>, StorageError>;

    // --- Fundamental Metrics ---
    fn get_all_fundamental_metrics(&self) -> Result<Vec<FundamentalMetricRow>, StorageError>;
    fn get_fundamental_metrics_for_ticker(
        &self,
        ticker: &str,
    ) -> Result<Vec<FundamentalMetricRow>, StorageError>;

    // --- Earnings History ---
    fn get_earnings_history(
        &self,
        ticker: &str,
        limit: usize,
    ) -> Result<Vec<EarningsRow>, StorageError>;

    // --- Sector PE Benchmarks ---
    fn get_sector_pe_benchmarks(&self) -> Result<Vec<SectorPeRow>, StorageError>;

    // --- Delta cache support ---
    /// Returns the most recent candle date stored for a ticker, or None if no data.
    fn get_latest_candle_date(&self, ticker: &str) -> Result<Option<NaiveDate>, StorageError>;
    /// Returns candles for a ticker from `since` date (inclusive) in chronological order.
    fn get_candles_since(
        &self,
        ticker: &str,
        since: NaiveDate,
    ) -> Result<Vec<Candle>, StorageError>;

    // --- Fundamental snapshot persistence ---
    fn save_fundamental_snapshot(
        &self,
        snap: &crate::data::fundamentals::FundamentalSnapshot,
    ) -> Result<(), StorageError>;
}

// ─── SQLite implementation ────────────────────────────────────────────────────

pub struct SqliteRepository {
    conn: Mutex<Connection>,
}

impl SqliteRepository {
    /// Open (or create) a database at `path` and run migrations.
    pub fn open(path: &str) -> Result<Self, StorageError> {
        let conn = Connection::open(path)?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory database — useful for integration tests.
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory()?;
        run_migrations(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Acquire the database connection lock, converting poison to StorageError.
    pub fn connection(&self) -> Result<std::sync::MutexGuard<'_, Connection>, StorageError> {
        self.conn.lock().map_err(|_| StorageError::Poisoned)
    }
}

impl Repository for SqliteRepository {
    fn upsert_stock(&self, stock: &Stock) -> Result<(), StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO stocks (ticker, name, sector, industry, updated_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(ticker) DO UPDATE SET
               name=excluded.name,
               sector=excluded.sector,
               industry=excluded.industry,
               updated_at=excluded.updated_at",
            params![stock.ticker, stock.name, stock.sector, stock.industry],
        )?;
        Ok(())
    }

    fn get_stocks(&self) -> Result<Vec<Stock>, StorageError> {
        let conn = self.connection()?;
        let mut stmt =
            conn.prepare("SELECT ticker, name, sector, industry FROM stocks ORDER BY ticker")?;
        let stocks = stmt
            .query_map([], |row| {
                Ok(Stock {
                    ticker: row.get(0)?,
                    name: row.get(1)?,
                    sector: row.get(2)?,
                    industry: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(stocks)
    }

    fn upsert_candles(&self, ticker: &str, candles: &[Candle]) -> Result<(), StorageError> {
        let conn = self.connection()?;
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO candles (ticker, date, open, high, low, close, adj_close, volume)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(ticker, date) DO UPDATE SET
                   open=excluded.open, high=excluded.high, low=excluded.low,
                   close=excluded.close, adj_close=excluded.adj_close,
                   volume=excluded.volume",
            )?;
            for c in candles {
                stmt.execute(params![
                    ticker,
                    c.date.to_string(),
                    c.open,
                    c.high,
                    c.low,
                    c.close,
                    c.adj_close,
                    c.volume
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn get_candles(&self, ticker: &str, limit: usize) -> Result<Vec<Candle>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT date, open, high, low, close, adj_close, volume
             FROM candles
             WHERE ticker = ?1
             ORDER BY date DESC
             LIMIT ?2",
        )?;
        let candles: Vec<Candle> = stmt
            .query_map(params![ticker, limit as i64], |row| {
                let date_str: String = row.get(0)?;
                Ok((
                    date_str,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(
                |(date_str, open, high, low, close, adj_close, volume)| Candle {
                    date: NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").unwrap_or_default(),
                    open,
                    high,
                    low,
                    close,
                    adj_close,
                    volume: volume as u64,
                },
            )
            .rev() // Return in chronological order
            .collect();
        Ok(candles)
    }

    fn save_screen_result(
        &self,
        run_date: NaiveDate,
        result: &ScreenResult,
    ) -> Result<(), StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO screen_results
               (run_date, ticker, minervini_pass, conditions_met, adx, rs_percentile,
                volume_zscore, technical_score, volume_score, sentiment_score,
                composite_score, grade, close_price, pct_from_52w_high,
                atr, risk_reward, liquidity, weighted_rs, regime)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
            params![
                run_date.to_string(),
                result.ticker,
                result.minervini_pass as i32,
                result.conditions_met,
                result.adx,
                result.rs_percentile,
                result.volume_zscore,
                result.technical_score,
                result.volume_score,
                result.sentiment_score,
                result.composite_score,
                result.grade.to_string(),
                result.close_price,
                result.pct_from_52w_high,
                result.atr_value,
                result.risk_reward,
                result.liquidity.to_string(),
                result.weighted_rs,
                result.regime.to_string(),
            ],
        )?;
        Ok(())
    }

    fn get_screen_results(&self, run_date: NaiveDate) -> Result<Vec<ScreenResult>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT ticker, minervini_pass, conditions_met, adx, rs_percentile,
                    volume_zscore, technical_score, volume_score, sentiment_score,
                    composite_score, close_price, pct_from_52w_high, grade,
                    atr, risk_reward, liquidity, weighted_rs, regime
             FROM screen_results
             WHERE run_date = ?1
             ORDER BY composite_score DESC",
        )?;
        let results = stmt
            .query_map([run_date.to_string()], |row| {
                let grade_str: String = row.get(12)?;
                let grade = match grade_str.as_str() {
                    "A+" | "APlus" => crate::analysis::scorer::Grade::APlus,
                    "A" => crate::analysis::scorer::Grade::A,
                    "B" => crate::analysis::scorer::Grade::B,
                    "C" => crate::analysis::scorer::Grade::C,
                    "D" => crate::analysis::scorer::Grade::D,
                    _ => crate::analysis::scorer::Grade::F,
                };
                let liquidity_str: String = row.get::<_, String>(15).unwrap_or_default();
                let liquidity = match liquidity_str.as_str() {
                    "High" => LiquidityClass::High,
                    "Medium" => LiquidityClass::Medium,
                    "Low" => LiquidityClass::Low,
                    _ => LiquidityClass::Illiquid,
                };
                let regime_str: String = row.get::<_, String>(17).unwrap_or_default();
                let regime = match regime_str.as_str() {
                    "BULL" => MarketRegime::Bull,
                    "BEAR" => MarketRegime::Bear,
                    _ => MarketRegime::Sideways,
                };
                Ok(ScreenResult {
                    ticker: row.get(0)?,
                    minervini_pass: row.get::<_, i32>(1)? != 0,
                    conditions_met: row.get::<_, u8>(2)?,
                    adx: row.get(3)?,
                    rs_percentile: row.get(4)?,
                    volume_zscore: row.get(5)?,
                    technical_score: row.get(6)?,
                    volume_score: row.get(7)?,
                    sentiment_score: row.get(8)?,
                    composite_score: row.get(9)?,
                    grade,
                    close_price: row.get(10)?,
                    pct_from_52w_high: row.get(11)?,
                    atr_value: row.get::<_, f64>(13).unwrap_or(0.0),
                    risk_reward: row.get::<_, f64>(14).unwrap_or(0.0),
                    liquidity,
                    weighted_rs: row.get::<_, f64>(16).unwrap_or(0.0),
                    rs_momentum: None,
                    vcp: None,
                    regime,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(results)
    }

    fn record_alert(&self, alert: &OutboundAlert) -> Result<(), StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO alert_log (ticker, alert_type, severity, message)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(ticker, alert_type) DO UPDATE SET
               severity=excluded.severity,
               message=excluded.message,
               sent_at=datetime('now')",
            params![
                alert.ticker,
                alert.alert_type,
                alert.severity.to_string(),
                alert.message,
            ],
        )?;
        Ok(())
    }

    fn alert_last_sent(
        &self,
        ticker: &str,
        alert_type: &str,
    ) -> Result<Option<chrono::DateTime<chrono::Utc>>, StorageError> {
        let conn = self.connection()?;
        let result: rusqlite::Result<String> = conn.query_row(
            "SELECT sent_at FROM alert_log WHERE ticker=?1 AND alert_type=?2",
            params![ticker, alert_type],
            |row| row.get(0),
        );
        match result {
            Ok(s) => {
                let dt = chrono::DateTime::parse_from_rfc3339(&s)
                    .or_else(|_| {
                        // SQLite datetime() produces "YYYY-MM-DD HH:MM:SS" without timezone
                        chrono::NaiveDateTime::parse_from_str(&s, "%Y-%m-%d %H:%M:%S")
                            .map(|ndt| ndt.and_utc().fixed_offset())
                    })
                    .map(|dt| dt.to_utc())
                    .map_err(|e| StorageError::NotFound(e.to_string()))?;
                Ok(Some(dt))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    fn save_regime(&self, state: &RegimeState) -> Result<(), StorageError> {
        let conn = self.connection()?;
        let regime_str = match state.regime {
            MarketRegime::Bull => "Bull",
            MarketRegime::Sideways => "Sideways",
            MarketRegime::Bear => "Bear",
        };
        conn.execute(
            "INSERT INTO market_regime (date, spy_close, spy_sma200, breadth_pct, regime)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(date) DO UPDATE SET
               spy_close=excluded.spy_close,
               spy_sma200=excluded.spy_sma200,
               breadth_pct=excluded.breadth_pct,
               regime=excluded.regime",
            params![
                state.date.to_string(),
                state.spy_close,
                state.spy_sma200,
                state.breadth_pct,
                regime_str,
            ],
        )?;
        Ok(())
    }

    fn latest_regime(&self) -> Result<Option<RegimeState>, StorageError> {
        let conn = self.connection()?;
        let result = conn.query_row(
            "SELECT date, spy_close, spy_sma200, breadth_pct, regime
             FROM market_regime ORDER BY date DESC LIMIT 1",
            [],
            |row| {
                let date_str: String = row.get(0)?;
                let regime_str: String = row.get(4)?;
                Ok((date_str, row.get(1)?, row.get(2)?, row.get(3)?, regime_str))
            },
        );
        match result {
            Ok((date_str, spy_close, spy_sma200, breadth_pct, regime_str)) => {
                let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").unwrap_or_default();
                let regime = match regime_str.as_str() {
                    "Bull" => MarketRegime::Bull,
                    "Bear" => MarketRegime::Bear,
                    _ => MarketRegime::Sideways,
                };
                Ok(Some(RegimeState {
                    date,
                    spy_close,
                    spy_sma200,
                    breadth_pct,
                    regime,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    fn insert_flagged_stock(&self, stock: &FlaggedStock) -> Result<(), StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO flagged_stocks
               (ticker, flagged_date, entry_score, entry_price, conditions_at_entry, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(ticker) WHERE status = 'ACTIVE' DO UPDATE SET
               entry_score=excluded.entry_score,
               entry_price=excluded.entry_price,
               conditions_at_entry=excluded.conditions_at_entry",
            params![
                stock.ticker,
                stock.flagged_date.to_string(),
                stock.entry_score,
                stock.entry_price,
                stock.conditions_at_entry as i32,
                stock.status.to_string(),
            ],
        )?;
        Ok(())
    }

    fn get_active_flagged_stocks(&self) -> Result<Vec<FlaggedStock>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, ticker, flagged_date, entry_score, entry_price,
                    conditions_at_entry, status, exit_date, exit_reason, exit_price,
                    latest_score, last_rescored_at
             FROM flagged_stocks
             WHERE status = 'ACTIVE'
             ORDER BY entry_score DESC",
        )?;
        let stocks = stmt
            .query_map([], |row| {
                let flagged_date_str: String = row.get(2)?;
                let exit_date_str: Option<String> = row.get(7)?;
                let last_rescored_str: Option<String> = row.get(11)?;
                let status_str: String = row.get(6)?;
                let status = match status_str.as_str() {
                    "EXITED" => FlaggedStatus::Exited,
                    "EXPIRED" => FlaggedStatus::Expired,
                    _ => FlaggedStatus::Active,
                };
                Ok(FlaggedStock {
                    id: Some(row.get(0)?),
                    ticker: row.get(1)?,
                    flagged_date: NaiveDate::parse_from_str(&flagged_date_str, "%Y-%m-%d")
                        .unwrap_or_default(),
                    entry_score: row.get(3)?,
                    entry_price: row.get(4)?,
                    conditions_at_entry: row.get::<_, i32>(5)? as u8,
                    status,
                    exit_date: exit_date_str
                        .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()),
                    exit_reason: row.get(8)?,
                    exit_price: row.get(9)?,
                    latest_score: row.get(10)?,
                    last_rescored_at: last_rescored_str
                        .and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(stocks)
    }

    fn mark_exited(
        &self,
        ticker: &str,
        exit_date: NaiveDate,
        exit_reason: &str,
        exit_price: f64,
    ) -> Result<(), StorageError> {
        let conn = self.connection()?;
        let updated = conn.execute(
            "UPDATE flagged_stocks
             SET status = 'EXITED', exit_date = ?1, exit_reason = ?2, exit_price = ?3
             WHERE ticker = ?4 AND status = 'ACTIVE'",
            params![exit_date.to_string(), exit_reason, exit_price, ticker],
        )?;
        if updated == 0 {
            return Err(StorageError::NotFound(format!(
                "No active flagged stock for ticker: {}",
                ticker
            )));
        }
        Ok(())
    }

    fn expire_old_flags(&self, days_old: i64) -> Result<usize, StorageError> {
        let conn = self.connection()?;
        let cutoff =
            (chrono::Utc::now().date_naive() - chrono::Duration::days(days_old)).to_string();
        let updated = conn.execute(
            "UPDATE flagged_stocks
             SET status = 'EXPIRED'
             WHERE status = 'ACTIVE' AND flagged_date < ?1",
            params![cutoff],
        )?;
        Ok(updated)
    }

    fn save_macro_risk(
        &self,
        date: NaiveDate,
        snapshot: &crate::analysis::macro_risk::MacroRiskSnapshot,
    ) -> Result<(), StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO macro_risk (date, vix_level, hy_oas_bps, yield_curve_2s10s, put_call_ratio, risk_level)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(date) DO UPDATE SET
               vix_level=excluded.vix_level,
               hy_oas_bps=excluded.hy_oas_bps,
               yield_curve_2s10s=excluded.yield_curve_2s10s,
               put_call_ratio=excluded.put_call_ratio,
               risk_level=excluded.risk_level",
            params![
                date.to_string(),
                snapshot.vix_level,
                snapshot.hy_oas_bps,
                snapshot.yield_curve_2s10s,
                snapshot.put_call_ratio,
                snapshot.risk_level.to_string(),
            ],
        )?;
        Ok(())
    }

    fn latest_macro_risk(
        &self,
    ) -> Result<Option<crate::analysis::macro_risk::MacroRiskSnapshot>, StorageError> {
        use crate::analysis::macro_risk::{MacroRiskLevel, MacroRiskSnapshot};
        let conn = self.connection()?;
        let result = conn.query_row(
            "SELECT vix_level, hy_oas_bps, yield_curve_2s10s, put_call_ratio, risk_level
             FROM macro_risk ORDER BY date DESC LIMIT 1",
            [],
            |row| {
                let risk_str: String = row.get(4)?;
                Ok((
                    row.get::<_, f64>(0)?,
                    row.get::<_, Option<f64>>(1)?,
                    row.get::<_, Option<f64>>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                    risk_str,
                ))
            },
        );
        match result {
            Ok((vix, oas, yc, pc, risk_str)) => {
                let risk_level = match risk_str.as_str() {
                    "HighRisk" => MacroRiskLevel::HighRisk,
                    "Elevated" => MacroRiskLevel::Elevated,
                    _ => MacroRiskLevel::Normal,
                };
                Ok(Some(MacroRiskSnapshot {
                    vix_level: vix,
                    hy_oas_bps: oas,
                    yield_curve_2s10s: yc,
                    put_call_ratio: pc,
                    risk_level,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    fn update_flagged_score(
        &self,
        ticker: &str,
        score: f64,
        date: NaiveDate,
    ) -> Result<(), StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE flagged_stocks SET latest_score = ?1, last_rescored_at = ?2
             WHERE ticker = ?3 AND status = 'ACTIVE'",
            params![score, date.to_string(), ticker],
        )?;
        Ok(())
    }

    fn cache_news(&self, articles: &[NewsArticle]) -> Result<usize, StorageError> {
        let conn = self.connection()?;
        let tx = conn.unchecked_transaction()?;
        let mut inserted = 0usize;
        {
            let mut stmt = tx.prepare(
                "INSERT OR IGNORE INTO news_cache (ticker, headline, summary, published_at, source)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?;
            for article in articles {
                let rows = stmt.execute(params![
                    article.ticker,
                    article.headline,
                    article.summary,
                    article.published_at.to_rfc3339(),
                    article.source,
                ])?;
                inserted += rows;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    fn get_cached_news(
        &self,
        ticker: &str,
        since: NaiveDate,
    ) -> Result<Vec<NewsArticle>, StorageError> {
        let conn = self.connection()?;
        let since_str = since.to_string(); // "YYYY-MM-DD"
        let mut stmt = conn.prepare(
            "SELECT ticker, headline, summary, published_at, source
             FROM news_cache
             WHERE ticker = ?1 AND published_at >= ?2
             ORDER BY published_at DESC",
        )?;
        let articles = stmt
            .query_map(params![ticker, since_str], |row| {
                let pub_str: String = row.get(3)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    pub_str,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|(ticker, headline, summary, pub_str, source)| {
                let published_at = chrono::DateTime::parse_from_rfc3339(&pub_str)
                    .map(|dt| dt.to_utc())
                    .unwrap_or_else(|_| chrono::Utc::now());
                NewsArticle {
                    ticker,
                    headline,
                    summary,
                    published_at,
                    source,
                }
            })
            .collect();
        Ok(articles)
    }

    fn delete_old_news(&self, older_than_days: i64) -> Result<usize, StorageError> {
        let conn = self.connection()?;
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(older_than_days)).to_rfc3339();
        let deleted = conn.execute(
            "DELETE FROM news_cache WHERE published_at < ?1",
            params![cutoff],
        )?;
        Ok(deleted)
    }

    fn get_recent_alerts(&self, limit: usize) -> Result<Vec<AlertLogEntry>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT ticker, alert_type, severity, message, sent_at
             FROM alert_log
             ORDER BY sent_at DESC
             LIMIT ?1",
        )?;
        let entries = stmt
            .query_map([limit as i64], |row| {
                Ok(AlertLogEntry {
                    ticker: row.get(0)?,
                    alert_type: row.get(1)?,
                    severity: row.get(2)?,
                    message: row.get(3)?,
                    sent_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    fn save_backtest_report(
        &self,
        params_json: &str,
        kpis_json: &str,
        report_json: &str,
        signal_count: usize,
        duration_secs: f64,
    ) -> Result<(), StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO backtest_runs (params_json, kpis_json, report_json, signal_count, duration_secs)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![params_json, kpis_json, report_json, signal_count as i64, duration_secs],
        )?;
        Ok(())
    }

    fn get_backtest_runs(&self, limit: usize) -> Result<Vec<BacktestRunEntry>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, run_date, params_json, kpis_json, report_json, signal_count, duration_secs
             FROM backtest_runs
             ORDER BY run_date DESC
             LIMIT ?1",
        )?;
        let entries = stmt
            .query_map([limit as i64], |row| {
                Ok(BacktestRunEntry {
                    id: row.get(0)?,
                    run_date: row.get(1)?,
                    params_json: row.get(2)?,
                    kpis_json: row.get(3)?,
                    report_json: row.get(4)?,
                    signal_count: row.get::<_, i64>(5)? as usize,
                    duration_secs: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    // ── Watchlist ──────────────────────────────────────────────────────────

    fn watchlist_add(&self, ticker: &str, notes: Option<&str>) -> Result<(), StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO watchlist (ticker, notes)
             VALUES (?1, ?2)
             ON CONFLICT(ticker) DO NOTHING",
            params![ticker, notes],
        )?;
        Ok(())
    }

    fn watchlist_remove(&self, ticker: &str) -> Result<bool, StorageError> {
        let conn = self.connection()?;
        let deleted = conn.execute("DELETE FROM watchlist WHERE ticker = ?1", params![ticker])?;
        Ok(deleted > 0)
    }

    fn watchlist_list(&self) -> Result<Vec<WatchlistEntry>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT ticker, added_at, notes, last_score, last_grade, last_screen
             FROM watchlist
             ORDER BY added_at ASC",
        )?;
        let entries = stmt
            .query_map([], |row| {
                Ok(WatchlistEntry {
                    ticker: row.get(0)?,
                    added_at: row.get(1)?,
                    notes: row.get(2)?,
                    last_score: row.get(3)?,
                    last_grade: row.get(4)?,
                    last_screen: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(entries)
    }

    fn watchlist_tickers(&self) -> Result<Vec<String>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare("SELECT ticker FROM watchlist ORDER BY added_at ASC")?;
        let tickers = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(tickers)
    }

    fn watchlist_update_score(
        &self,
        ticker: &str,
        score: f64,
        grade: &str,
    ) -> Result<(), StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "UPDATE watchlist
             SET last_score = ?1, last_grade = ?2, last_screen = datetime('now')
             WHERE ticker = ?3",
            params![score, grade, ticker],
        )?;
        Ok(())
    }

    fn watchlist_clear(&self) -> Result<usize, StorageError> {
        let conn = self.connection()?;
        let deleted = conn.execute("DELETE FROM watchlist", [])?;
        Ok(deleted)
    }

    // ── Read-only analytics ──────────────────────────────────────────────

    fn get_screen_results_range(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<ScreenResultSummary>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT ticker, run_date, composite_score, sentiment_score,
                    rs_percentile, weighted_rs, grade
             FROM screen_results
             WHERE run_date BETWEEN ?1 AND ?2
             ORDER BY run_date ASC, composite_score DESC",
        )?;
        let rows = stmt
            .query_map(params![start, end], |row| {
                Ok(ScreenResultSummary {
                    ticker: row.get(0)?,
                    run_date: row.get(1)?,
                    composite_score: row.get(2)?,
                    sentiment_score: row.get(3)?,
                    rs_percentile: row.get(4)?,
                    weighted_rs: row.get(5)?,
                    grade: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn get_regime_history(&self, limit: usize) -> Result<Vec<RegimeState>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT date, spy_close, spy_sma200, breadth_pct, regime
             FROM market_regime
             ORDER BY date DESC LIMIT ?1",
        )?;
        let mut rows: Vec<RegimeState> = stmt
            .query_map(params![limit as i64], |row| {
                let date_str: String = row.get(0)?;
                let regime_str: String = row.get(4)?;
                Ok((date_str, row.get(1)?, row.get(2)?, row.get(3)?, regime_str))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(
                |(date_str, spy_close, spy_sma200, breadth_pct, regime_str)| {
                    let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").unwrap_or_default();
                    let regime = match regime_str.as_str() {
                        "Bull" => MarketRegime::Bull,
                        "Bear" => MarketRegime::Bear,
                        _ => MarketRegime::Sideways,
                    };
                    RegimeState {
                        date,
                        spy_close,
                        spy_sma200,
                        breadth_pct,
                        regime,
                    }
                },
            )
            .collect();
        rows.reverse(); // chronological order
        Ok(rows)
    }

    fn get_macro_risk_history(
        &self,
        limit: usize,
    ) -> Result<Vec<crate::analysis::macro_risk::MacroRiskSnapshot>, StorageError> {
        use crate::analysis::macro_risk::{MacroRiskLevel, MacroRiskSnapshot};
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT date, vix_level, hy_oas_bps, yield_curve_2s10s, put_call_ratio, risk_level
             FROM macro_risk
             ORDER BY date DESC LIMIT ?1",
        )?;
        let mut rows: Vec<MacroRiskSnapshot> = stmt
            .query_map(params![limit as i64], |row| {
                let risk_str: String = row.get(5)?;
                Ok((
                    row.get::<_, f64>(1)?,
                    row.get::<_, Option<f64>>(2)?,
                    row.get::<_, Option<f64>>(3)?,
                    row.get::<_, Option<f64>>(4)?,
                    risk_str,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|(vix, oas, yc, pc, risk_str)| {
                let risk_level = match risk_str.as_str() {
                    "HighRisk" => MacroRiskLevel::HighRisk,
                    "Elevated" => MacroRiskLevel::Elevated,
                    _ => MacroRiskLevel::Normal,
                };
                MacroRiskSnapshot {
                    vix_level: vix,
                    hy_oas_bps: oas,
                    yield_curve_2s10s: yc,
                    put_call_ratio: pc,
                    risk_level,
                }
            })
            .collect();
        rows.reverse(); // chronological order
        Ok(rows)
    }

    fn get_grade_distribution(
        &self,
        start: &str,
        end: &str,
    ) -> Result<Vec<GradeCount>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT run_date, grade, COUNT(*) as cnt
             FROM screen_results
             WHERE run_date BETWEEN ?1 AND ?2
             GROUP BY run_date, grade
             ORDER BY run_date ASC",
        )?;
        let rows = stmt
            .query_map(params![start, end], |row| {
                Ok(GradeCount {
                    run_date: row.get(0)?,
                    grade: row.get(1)?,
                    count: row.get::<_, u32>(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn get_alert_counts_by_date(&self, since: &str) -> Result<Vec<AlertDayCount>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT DATE(sent_at) as day,
                    COUNT(*) as total,
                    SUM(CASE WHEN severity = 'CRITICAL' THEN 1 ELSE 0 END) as critical
             FROM alert_log
             WHERE sent_at >= ?1
             GROUP BY day
             ORDER BY day ASC",
        )?;
        let rows = stmt
            .query_map(params![since], |row| {
                Ok(AlertDayCount {
                    date: row.get(0)?,
                    total: row.get::<_, u32>(1)?,
                    critical: row.get::<_, u32>(2)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn get_ticker_score_history(
        &self,
        ticker: &str,
        since: &str,
    ) -> Result<Vec<(String, f64)>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT run_date, composite_score
             FROM screen_results
             WHERE ticker = ?1 AND run_date >= ?2
             ORDER BY run_date ASC",
        )?;
        let rows = stmt
            .query_map(params![ticker, since], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // --- Decision Audit Log ---

    fn record_decision(
        &self,
        entry: &crate::portfolio::decision::DecisionLogEntry,
    ) -> Result<i64, StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO decision_log (decision_date, decision_type, ticker, shares, price, rationale, advisor, linked_alert_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entry.decision_date.to_string(),
                entry.decision_type.to_string(),
                entry.ticker,
                entry.shares,
                entry.price,
                entry.rationale,
                entry.advisor,
                entry.linked_alert_id,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    fn get_decisions(
        &self,
        limit: usize,
    ) -> Result<Vec<crate::portfolio::decision::DecisionLogEntry>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, decision_date, decision_type, ticker, shares, price, rationale, advisor, linked_alert_id, created_at
             FROM decision_log ORDER BY decision_date DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(DecisionRow {
                id: row.get(0)?,
                decision_date: row.get(1)?,
                decision_type: row.get(2)?,
                ticker: row.get(3)?,
                shares: row.get(4)?,
                price: row.get(5)?,
                rationale: row.get(6)?,
                advisor: row.get(7)?,
                linked_alert_id: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(decision_row_to_entry(row?)?);
        }
        Ok(results)
    }

    fn get_decisions_for_ticker(
        &self,
        ticker: &str,
        limit: usize,
    ) -> Result<Vec<crate::portfolio::decision::DecisionLogEntry>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, decision_date, decision_type, ticker, shares, price, rationale, advisor, linked_alert_id, created_at
             FROM decision_log WHERE ticker = ?1 ORDER BY decision_date DESC, id DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![ticker, limit as i64], |row| {
            Ok(DecisionRow {
                id: row.get(0)?,
                decision_date: row.get(1)?,
                decision_type: row.get(2)?,
                ticker: row.get(3)?,
                shares: row.get(4)?,
                price: row.get(5)?,
                rationale: row.get(6)?,
                advisor: row.get(7)?,
                linked_alert_id: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(decision_row_to_entry(row?)?);
        }
        Ok(results)
    }

    // --- Correlation Matrix ---

    fn save_correlation_snapshot(
        &self,
        date: &str,
        tickers: &[String],
        matrix: &[Vec<f64>],
        window: usize,
    ) -> Result<(), StorageError> {
        let conn = self.connection()?;
        let tickers_json =
            serde_json::to_string(tickers).map_err(|e| StorageError::NotFound(e.to_string()))?;
        let matrix_json =
            serde_json::to_string(matrix).map_err(|e| StorageError::NotFound(e.to_string()))?;
        conn.execute(
            "INSERT INTO correlation_snapshots (computed_date, tickers_json, matrix_json, window_days)
             VALUES (?1, ?2, ?3, ?4)",
            params![date, tickers_json, matrix_json, window as i64],
        )?;
        Ok(())
    }

    fn get_latest_correlation_snapshot(
        &self,
    ) -> Result<Option<(String, Vec<String>, Vec<Vec<f64>>, usize)>, StorageError> {
        let conn = self.connection()?;
        let result = conn.query_row(
            "SELECT computed_date, tickers_json, matrix_json, window_days
             FROM correlation_snapshots
             ORDER BY id DESC LIMIT 1",
            [],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        );
        match result {
            Ok((date, tickers_str, matrix_str, window)) => {
                let tickers: Vec<String> = serde_json::from_str(&tickers_str)
                    .map_err(|e| StorageError::NotFound(e.to_string()))?;
                let matrix: Vec<Vec<f64>> = serde_json::from_str(&matrix_str)
                    .map_err(|e| StorageError::NotFound(e.to_string()))?;
                Ok(Some((date, tickers, matrix, window as usize)))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    // --- Fundamental History ---

    fn upsert_fundamental_history(
        &self,
        record: &crate::data::fundamentals::FundamentalHistoryRecord,
    ) -> Result<(), StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT OR REPLACE INTO fundamental_history
               (ticker, period_date, period_type, pe_ratio, pb_ratio, ps_ratio,
                ev_ebitda, ev_revenue, gross_margin, operating_margin, net_margin,
                roe, debt_to_equity, revenue, ebitda, fcf)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                record.ticker,
                record.period_date,
                record.period_type,
                record.pe_ratio,
                record.pb_ratio,
                record.ps_ratio,
                record.ev_ebitda,
                record.ev_revenue,
                record.gross_margin,
                record.operating_margin,
                record.net_margin,
                record.roe,
                record.debt_to_equity,
                record.revenue,
                record.ebitda,
                record.fcf,
            ],
        )?;
        Ok(())
    }

    fn get_fundamental_history(
        &self,
        ticker: &str,
        limit: usize,
    ) -> Result<Vec<crate::data::fundamentals::FundamentalHistoryRecord>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT ticker, period_date, period_type, pe_ratio, pb_ratio, ps_ratio,
                    ev_ebitda, ev_revenue, gross_margin, operating_margin, net_margin,
                    roe, debt_to_equity, revenue, ebitda, fcf
             FROM fundamental_history
             WHERE ticker = ?1
             ORDER BY period_date DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![ticker, limit as i64], |row| {
                Ok(crate::data::fundamentals::FundamentalHistoryRecord {
                    ticker: row.get(0)?,
                    period_date: row.get(1)?,
                    period_type: row.get(2)?,
                    pe_ratio: row.get(3)?,
                    pb_ratio: row.get(4)?,
                    ps_ratio: row.get(5)?,
                    ev_ebitda: row.get(6)?,
                    ev_revenue: row.get(7)?,
                    gross_margin: row.get(8)?,
                    operating_margin: row.get(9)?,
                    net_margin: row.get(10)?,
                    roe: row.get(11)?,
                    debt_to_equity: row.get(12)?,
                    revenue: row.get(13)?,
                    ebitda: row.get(14)?,
                    fcf: row.get(15)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn get_valuation_bands(
        &self,
        ticker: &str,
        metric: &str,
        years: u32,
    ) -> Result<Vec<(String, f64)>, StorageError> {
        const ALLOWED_METRICS: &[&str] = &[
            "pe_ratio",
            "pb_ratio",
            "ps_ratio",
            "ev_ebitda",
            "ev_revenue",
        ];
        if !ALLOWED_METRICS.contains(&metric) {
            return Err(StorageError::NotFound(format!(
                "Invalid metric '{}'. Allowed: {:?}",
                metric, ALLOWED_METRICS
            )));
        }
        let conn = self.connection()?;
        let cutoff = chrono::Utc::now().date_naive() - chrono::Duration::days(years as i64 * 365);
        // metric is validated against a whitelist, safe to interpolate
        let sql = format!(
            "SELECT period_date, {metric} FROM fundamental_history
             WHERE ticker = ?1 AND period_date >= ?2 AND {metric} IS NOT NULL
             ORDER BY period_date ASC"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![ticker, cutoff.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // --- ETF Snapshots ---

    fn upsert_etf_snapshot(
        &self,
        snapshot: &crate::data::etf::EtfSnapshot,
    ) -> Result<(), StorageError> {
        let conn = self.connection()?;
        let holdings_json = serde_json::to_string(&snapshot.top_holdings)
            .map_err(|e| StorageError::NotFound(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO etf_snapshots
               (ticker, name, expense_ratio, aum, avg_daily_volume, category,
                inception_date, tracking_index, holdings_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                snapshot.ticker,
                snapshot.name,
                snapshot.expense_ratio,
                snapshot.aum,
                snapshot.avg_daily_volume as i64,
                snapshot.category,
                snapshot.inception_date,
                snapshot.tracking_index,
                holdings_json,
            ],
        )?;
        Ok(())
    }

    fn get_etf_snapshot(
        &self,
        ticker: &str,
    ) -> Result<Option<crate::data::etf::EtfSnapshot>, StorageError> {
        let conn = self.connection()?;
        let result = conn.query_row(
            "SELECT ticker, name, expense_ratio, aum, avg_daily_volume, category,
                    inception_date, tracking_index, holdings_json
             FROM etf_snapshots
             WHERE ticker = ?1",
            params![ticker],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            },
        );
        match result {
            Ok((
                ticker,
                name,
                expense_ratio,
                aum,
                avg_vol,
                category,
                inception,
                tracking,
                holdings_str,
            )) => {
                let top_holdings: Vec<crate::data::etf::EtfHolding> =
                    serde_json::from_str(&holdings_str)
                        .map_err(|e| StorageError::NotFound(e.to_string()))?;
                Ok(Some(crate::data::etf::EtfSnapshot {
                    ticker,
                    name,
                    expense_ratio,
                    aum,
                    avg_daily_volume: avg_vol as u64,
                    category,
                    inception_date: inception,
                    tracking_index: tracking,
                    top_holdings,
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    fn get_all_etf_snapshots(&self) -> Result<Vec<crate::data::etf::EtfSnapshot>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT ticker, name, expense_ratio, aum, avg_daily_volume, category,
                    inception_date, tracking_index, holdings_json
             FROM etf_snapshots
             ORDER BY ticker ASC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut snapshots = Vec::with_capacity(rows.len());
        for (
            ticker,
            name,
            expense_ratio,
            aum,
            avg_vol,
            category,
            inception,
            tracking,
            holdings_str,
        ) in rows
        {
            let top_holdings: Vec<crate::data::etf::EtfHolding> =
                serde_json::from_str(&holdings_str)
                    .map_err(|e| StorageError::NotFound(e.to_string()))?;
            snapshots.push(crate::data::etf::EtfSnapshot {
                ticker,
                name,
                expense_ratio,
                aum,
                avg_daily_volume: avg_vol as u64,
                category,
                inception_date: inception,
                tracking_index: tracking,
                top_holdings,
            });
        }
        Ok(snapshots)
    }

    // --- Selling Programme ---

    fn save_selling_programme(
        &self,
        prog: &crate::portfolio::selling::SellingProgramme,
    ) -> Result<i64, StorageError> {
        let conn = self.connection()?;
        let schedule_json = serde_json::to_string(&prog.schedule)
            .map_err(|e| StorageError::NotFound(e.to_string()))?;
        let pace_rules_json = serde_json::to_string(&prog.pace_rules)
            .map_err(|e| StorageError::NotFound(e.to_string()))?;
        conn.execute(
            "INSERT INTO selling_programmes (ticker, schedule_json, pace_rules_json)
             VALUES (?1, ?2, ?3)",
            params![prog.ticker, schedule_json, pace_rules_json],
        )?;
        Ok(conn.last_insert_rowid())
    }

    fn get_selling_programme(
        &self,
        id: i64,
    ) -> Result<Option<crate::portfolio::selling::SellingProgramme>, StorageError> {
        let conn = self.connection()?;
        let result = conn.query_row(
            "SELECT id, ticker, schedule_json, pace_rules_json, created_at
             FROM selling_programmes
             WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        );
        match result {
            Ok((id, ticker, schedule_str, pace_str, created_at)) => {
                let schedule = serde_json::from_str(&schedule_str)
                    .map_err(|e| StorageError::NotFound(e.to_string()))?;
                let pace_rules = serde_json::from_str(&pace_str)
                    .map_err(|e| StorageError::NotFound(e.to_string()))?;
                Ok(Some(crate::portfolio::selling::SellingProgramme {
                    id: Some(id),
                    ticker,
                    schedule,
                    pace_rules,
                    created_at: Some(created_at),
                }))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::Sqlite(e)),
        }
    }

    fn get_selling_programmes_for_ticker(
        &self,
        ticker: &str,
    ) -> Result<Vec<crate::portfolio::selling::SellingProgramme>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, ticker, schedule_json, pace_rules_json, created_at
             FROM selling_programmes
             WHERE ticker = ?1
             ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map(params![ticker], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut progs = Vec::with_capacity(rows.len());
        for (id, ticker, schedule_str, pace_str, created_at) in rows {
            let schedule = serde_json::from_str(&schedule_str)
                .map_err(|e| StorageError::NotFound(e.to_string()))?;
            let pace_rules = serde_json::from_str(&pace_str)
                .map_err(|e| StorageError::NotFound(e.to_string()))?;
            progs.push(crate::portfolio::selling::SellingProgramme {
                id: Some(id),
                ticker,
                schedule,
                pace_rules,
                created_at: Some(created_at),
            });
        }
        Ok(progs)
    }

    fn update_selling_programme(
        &self,
        prog: &crate::portfolio::selling::SellingProgramme,
    ) -> Result<(), StorageError> {
        let id = prog
            .id
            .ok_or_else(|| StorageError::NotFound("SellingProgramme has no id".to_string()))?;
        let conn = self.connection()?;
        let schedule_json = serde_json::to_string(&prog.schedule)
            .map_err(|e| StorageError::NotFound(e.to_string()))?;
        let pace_rules_json = serde_json::to_string(&prog.pace_rules)
            .map_err(|e| StorageError::NotFound(e.to_string()))?;
        let updated = conn.execute(
            "UPDATE selling_programmes
             SET schedule_json = ?1, pace_rules_json = ?2, updated_at = datetime('now')
             WHERE id = ?3",
            params![schedule_json, pace_rules_json, id],
        )?;
        if updated == 0 {
            return Err(StorageError::NotFound(format!(
                "Selling programme id={} not found",
                id
            )));
        }
        Ok(())
    }

    fn record_selling_execution(
        &self,
        exec: &crate::portfolio::selling::SellingExecution,
    ) -> Result<i64, StorageError> {
        let conn = self.connection()?;
        let lot_ids_json = serde_json::to_string(&exec.lot_ids)
            .map_err(|e| StorageError::NotFound(e.to_string()))?;
        conn.execute(
            "INSERT INTO selling_executions
               (programme_id, execution_date, shares_sold, price, proceeds, lot_ids_json, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                exec.programme_id,
                exec.execution_date.to_string(),
                exec.shares_sold,
                exec.price,
                exec.proceeds,
                lot_ids_json,
                exec.notes,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    fn get_selling_executions(
        &self,
        programme_id: i64,
    ) -> Result<Vec<crate::portfolio::selling::SellingExecution>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, programme_id, execution_date, shares_sold, price, proceeds, lot_ids_json, notes
             FROM selling_executions
             WHERE programme_id = ?1
             ORDER BY execution_date ASC",
        )?;
        let rows = stmt
            .query_map(params![programme_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut execs = Vec::with_capacity(rows.len());
        for (id, prog_id, date_str, shares, price, proceeds, lot_ids_str, notes) in rows {
            let execution_date =
                NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").unwrap_or_default();
            let lot_ids: Vec<String> = lot_ids_str
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();
            execs.push(crate::portfolio::selling::SellingExecution {
                id: Some(id),
                programme_id: prog_id,
                execution_date,
                shares_sold: shares,
                price,
                proceeds,
                lot_ids,
                notes,
            });
        }
        Ok(execs)
    }

    // ── Tax Lots ─────────────────────────────────────────────────────────

    fn get_all_tax_lots(&self) -> Result<Vec<TaxLotRow>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, ticker, remaining_shares, cost_basis_per_share, acquired_date, acquisition_type
             FROM tax_lots
             WHERE remaining_shares > 0
             ORDER BY ticker, acquired_date",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(TaxLotRow {
                    lot_id: row.get(0)?,
                    ticker: row.get(1)?,
                    shares: row.get(2)?,
                    cost_basis_per_share: row.get(3)?,
                    acquisition_date: row.get(4)?,
                    acquisition_type: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn get_tax_lots_for_ticker(&self, ticker: &str) -> Result<Vec<TaxLotRow>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, ticker, remaining_shares, cost_basis_per_share, acquired_date, acquisition_type
             FROM tax_lots
             WHERE ticker = ?1 AND remaining_shares > 0
             ORDER BY acquired_date",
        )?;
        let rows = stmt
            .query_map(params![ticker], |row| {
                Ok(TaxLotRow {
                    lot_id: row.get(0)?,
                    ticker: row.get(1)?,
                    shares: row.get(2)?,
                    cost_basis_per_share: row.get(3)?,
                    acquisition_date: row.get(4)?,
                    acquisition_type: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ── Fundamental Metrics ──────────────────────────────────────────────

    fn get_all_fundamental_metrics(&self) -> Result<Vec<FundamentalMetricRow>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT ticker, fetched_date, pe_normalized, pb_annual, ps_annual,
                    eps_growth_ttm_yoy, rev_growth_ttm_yoy, gross_margin_ttm,
                    net_margin_ttm, roe_ttm, fundamental_score
             FROM fundamental_metrics
             ORDER BY ticker, fetched_date DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(FundamentalMetricRow {
                    ticker: row.get(0)?,
                    fetched_date: row.get(1)?,
                    pe_normalized: row.get(2)?,
                    pb_annual: row.get(3)?,
                    ps_annual: row.get(4)?,
                    eps_growth_ttm_yoy: row.get(5)?,
                    rev_growth_ttm_yoy: row.get(6)?,
                    gross_margin_ttm: row.get(7)?,
                    net_margin_ttm: row.get(8)?,
                    roe_ttm: row.get(9)?,
                    fundamental_score: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn get_fundamental_metrics_for_ticker(
        &self,
        ticker: &str,
    ) -> Result<Vec<FundamentalMetricRow>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT ticker, fetched_date, pe_normalized, pb_annual, ps_annual,
                    eps_growth_ttm_yoy, rev_growth_ttm_yoy, gross_margin_ttm,
                    net_margin_ttm, roe_ttm, fundamental_score
             FROM fundamental_metrics
             WHERE ticker = ?1
             ORDER BY fetched_date DESC",
        )?;
        let rows = stmt
            .query_map(params![ticker], |row| {
                Ok(FundamentalMetricRow {
                    ticker: row.get(0)?,
                    fetched_date: row.get(1)?,
                    pe_normalized: row.get(2)?,
                    pb_annual: row.get(3)?,
                    ps_annual: row.get(4)?,
                    eps_growth_ttm_yoy: row.get(5)?,
                    rev_growth_ttm_yoy: row.get(6)?,
                    gross_margin_ttm: row.get(7)?,
                    net_margin_ttm: row.get(8)?,
                    roe_ttm: row.get(9)?,
                    fundamental_score: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ── Earnings History ─────────────────────────────────────────────────

    fn get_earnings_history(
        &self,
        ticker: &str,
        limit: usize,
    ) -> Result<Vec<EarningsRow>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT ticker, period_date, fiscal_year, fiscal_quarter,
                    eps_actual, eps_estimate, eps_surprise_pct,
                    is_upcoming, earnings_date
             FROM earnings_history
             WHERE ticker = ?1
             ORDER BY period_date DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![ticker, limit as i64], |row| {
                Ok(EarningsRow {
                    ticker: row.get(0)?,
                    period_date: row.get(1)?,
                    fiscal_year: row.get(2)?,
                    fiscal_quarter: row.get(3)?,
                    eps_actual: row.get(4)?,
                    eps_estimate: row.get(5)?,
                    eps_surprise_pct: row.get(6)?,
                    is_upcoming: row.get::<_, i32>(7)? != 0,
                    earnings_date: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ── Sector PE Benchmarks ─────────────────────────────────────────────

    fn get_sector_pe_benchmarks(&self) -> Result<Vec<SectorPeRow>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT sector, snapshot_date, median_pe, median_ps, stock_count
             FROM sector_pe_benchmarks
             ORDER BY snapshot_date DESC, sector",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SectorPeRow {
                    sector: row.get(0)?,
                    snapshot_date: row.get(1)?,
                    median_pe: row.get(2)?,
                    median_ps: row.get(3)?,
                    stock_count: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn get_latest_candle_date(&self, ticker: &str) -> Result<Option<NaiveDate>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare("SELECT MAX(date) FROM candles WHERE ticker = ?1")?;
        let date_str: Option<String> = stmt.query_row(params![ticker], |row| row.get(0))?;
        Ok(date_str.and_then(|s| NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()))
    }

    fn get_candles_since(
        &self,
        ticker: &str,
        since: NaiveDate,
    ) -> Result<Vec<Candle>, StorageError> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare(
            "SELECT date, open, high, low, close, adj_close, volume
             FROM candles
             WHERE ticker = ?1 AND date >= ?2
             ORDER BY date ASC",
        )?;
        let candles = stmt
            .query_map(params![ticker, since.to_string()], |row| {
                let date_str: String = row.get(0)?;
                Ok((
                    date_str,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(
                |(date_str, open, high, low, close, adj_close, volume)| Candle {
                    date: NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").unwrap_or_default(),
                    open,
                    high,
                    low,
                    close,
                    adj_close,
                    volume: volume as u64,
                },
            )
            .collect();
        Ok(candles)
    }

    fn save_fundamental_snapshot(
        &self,
        snap: &crate::data::fundamentals::FundamentalSnapshot,
    ) -> Result<(), StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO fundamental_metrics (
                ticker, fetched_date, pe_normalized, pb_annual, ps_annual,
                eps_growth_ttm_yoy, rev_growth_ttm_yoy, eps_growth_qoq, rev_growth_qoq,
                gross_margin_ttm, net_margin_ttm, roe_ttm, roi_ttm,
                debt_equity, current_ratio, fundamental_score
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
             ON CONFLICT(ticker, fetched_date) DO UPDATE SET
               pe_normalized=excluded.pe_normalized, pb_annual=excluded.pb_annual,
               ps_annual=excluded.ps_annual, eps_growth_ttm_yoy=excluded.eps_growth_ttm_yoy,
               rev_growth_ttm_yoy=excluded.rev_growth_ttm_yoy,
               eps_growth_qoq=excluded.eps_growth_qoq, rev_growth_qoq=excluded.rev_growth_qoq,
               gross_margin_ttm=excluded.gross_margin_ttm, net_margin_ttm=excluded.net_margin_ttm,
               roe_ttm=excluded.roe_ttm, roi_ttm=excluded.roi_ttm,
               debt_equity=excluded.debt_equity, current_ratio=excluded.current_ratio,
               fundamental_score=excluded.fundamental_score",
            params![
                snap.ticker,
                snap.fetched_date.to_string(),
                snap.pe_normalized,
                snap.pb_annual,
                snap.ps_annual,
                snap.eps_growth_ttm_yoy,
                snap.rev_growth_ttm_yoy,
                snap.eps_growth_qoq,
                snap.rev_growth_qoq,
                snap.gross_margin_ttm,
                snap.net_margin_ttm,
                snap.roe_ttm,
                snap.roi_ttm,
                snap.debt_equity,
                snap.current_ratio,
                snap.fundamental_score,
            ],
        )?;
        Ok(())
    }
}

/// Internal helper struct for reading decision_log rows.
struct DecisionRow {
    id: i64,
    decision_date: String,
    decision_type: String,
    ticker: Option<String>,
    shares: Option<f64>,
    price: Option<f64>,
    rationale: String,
    advisor: Option<String>,
    linked_alert_id: Option<i64>,
    created_at: String,
}

/// Convert a `DecisionRow` to a `DecisionLogEntry`.
fn decision_row_to_entry(
    row: DecisionRow,
) -> Result<crate::portfolio::decision::DecisionLogEntry, StorageError> {
    let decision_date = NaiveDate::parse_from_str(&row.decision_date, "%Y-%m-%d")
        .map_err(|e| StorageError::NotFound(format!("invalid decision_date: {e}")))?;
    let decision_type: crate::portfolio::decision::DecisionType =
        row.decision_type.parse().map_err(StorageError::NotFound)?;
    Ok(crate::portfolio::decision::DecisionLogEntry {
        id: Some(row.id),
        decision_date,
        decision_type,
        ticker: row.ticker,
        shares: row.shares,
        price: row.price,
        rationale: row.rationale,
        advisor: row.advisor,
        linked_alert_id: row.linked_alert_id,
        created_at: Some(row.created_at),
    })
}

impl SqliteRepository {
    // ─── Strategy Beliefs (Bayesian Weights) ──────────────────────────────────

    /// Save or update a strategy belief (upsert on strategy+regime).
    pub fn save_strategy_belief(
        &self,
        strategy: &str,
        regime: &str,
        alpha: f64,
        beta: f64,
    ) -> Result<(), StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "INSERT INTO strategy_beliefs (strategy, regime, alpha, beta, updated_at)
             VALUES (?1, ?2, ?3, ?4, datetime('now'))
             ON CONFLICT(strategy, regime) DO UPDATE SET
                alpha = excluded.alpha,
                beta = excluded.beta,
                updated_at = excluded.updated_at",
            rusqlite::params![strategy, regime, alpha, beta],
        )?;
        Ok(())
    }

    /// Load all strategy beliefs for a given regime.
    pub fn load_strategy_beliefs(
        &self,
        regime: &str,
    ) -> Result<Vec<(String, f64, f64)>, StorageError> {
        let conn = self.connection()?;
        let mut stmt =
            conn.prepare("SELECT strategy, alpha, beta FROM strategy_beliefs WHERE regime = ?1")?;
        let rows = stmt.query_map(rusqlite::params![regime], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, f64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row?);
        }
        Ok(results)
    }

    /// Delete all strategy beliefs (used on regime reset).
    pub fn clear_strategy_beliefs(&self, regime: &str) -> Result<(), StorageError> {
        let conn = self.connection()?;
        conn.execute(
            "DELETE FROM strategy_beliefs WHERE regime = ?1",
            rusqlite::params![regime],
        )?;
        Ok(())
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn repo() -> SqliteRepository {
        SqliteRepository::open_in_memory().expect("in-memory SQLite")
    }

    fn sample_stock(ticker: &str) -> Stock {
        Stock {
            ticker: ticker.to_string(),
            name: format!("{} Inc.", ticker),
            sector: "Technology".to_string(),
            industry: "Semiconductors".to_string(),
        }
    }

    fn sample_candle(date: &str, close: f64) -> Candle {
        Candle::new(
            date,
            close - 1.0,
            close + 1.0,
            close - 2.0,
            close,
            1_000_000,
        )
    }

    // ── Stock CRUD ──────────────────────────────────────────────────────────

    #[test]
    fn upsert_and_retrieve_stock() {
        let repo = repo();
        repo.upsert_stock(&sample_stock("NVDA")).unwrap();
        let stocks = repo.get_stocks().unwrap();
        assert_eq!(stocks.len(), 1);
        assert_eq!(stocks[0].ticker, "NVDA");
    }

    #[test]
    fn upsert_stock_updates_existing() {
        let repo = repo();
        repo.upsert_stock(&sample_stock("NVDA")).unwrap();
        let updated = Stock {
            ticker: "NVDA".into(),
            name: "NVIDIA Corporation".into(),
            sector: "Technology".into(),
            industry: "GPUs".into(),
        };
        repo.upsert_stock(&updated).unwrap();
        let stocks = repo.get_stocks().unwrap();
        assert_eq!(stocks.len(), 1, "Upsert should not duplicate");
        assert_eq!(stocks[0].name, "NVIDIA Corporation");
    }

    #[test]
    fn get_stocks_empty_returns_empty_vec() {
        let repo = repo();
        let stocks = repo.get_stocks().unwrap();
        assert!(stocks.is_empty());
    }

    // ── Candle CRUD ─────────────────────────────────────────────────────────

    #[test]
    fn upsert_and_retrieve_candles() {
        let repo = repo();
        repo.upsert_stock(&sample_stock("AAPL")).unwrap();
        let candles = vec![
            sample_candle("2024-01-01", 180.0),
            sample_candle("2024-01-02", 182.0),
            sample_candle("2024-01-03", 185.0),
        ];
        repo.upsert_candles("AAPL", &candles).unwrap();
        let retrieved = repo.get_candles("AAPL", 10).unwrap();
        assert_eq!(retrieved.len(), 3);
        // Should be in chronological order
        assert_eq!(retrieved[0].close, 180.0);
        assert_eq!(retrieved[2].close, 185.0);
    }

    #[test]
    fn get_candles_respects_limit() {
        let repo = repo();
        repo.upsert_stock(&sample_stock("AAPL")).unwrap();
        let candles: Vec<Candle> = (1..=10)
            .map(|d| sample_candle(&format!("2024-01-{:02}", d), 100.0 + d as f64))
            .collect();
        repo.upsert_candles("AAPL", &candles).unwrap();
        let retrieved = repo.get_candles("AAPL", 5).unwrap();
        assert_eq!(retrieved.len(), 5);
    }

    // ── Alert log ───────────────────────────────────────────────────────────

    #[test]
    fn record_alert_and_last_sent() {
        use crate::data::models::Severity;
        let repo = repo();
        let alert = OutboundAlert {
            ticker: "NVDA".into(),
            alert_type: "NewSignal".into(),
            severity: Severity::Critical,
            message: "test alert".into(),
            composite_score: 92.0,
            narrative: None,
        };
        repo.record_alert(&alert).unwrap();
        let last_sent = repo.alert_last_sent("NVDA", "NewSignal").unwrap();
        assert!(last_sent.is_some(), "Should have a last_sent timestamp");
    }

    #[test]
    fn alert_last_sent_unknown_returns_none() {
        let repo = repo();
        let last_sent = repo.alert_last_sent("UNKNOWN", "NewSignal").unwrap();
        assert!(last_sent.is_none());
    }

    // ── Market regime ───────────────────────────────────────────────────────

    #[test]
    fn save_and_retrieve_regime() {
        let repo = repo();
        let state = RegimeState {
            date: NaiveDate::from_ymd_opt(2024, 1, 15).unwrap(),
            spy_close: 480.0,
            spy_sma200: 440.0,
            breadth_pct: 65.0,
            regime: MarketRegime::Bull,
        };
        repo.save_regime(&state).unwrap();
        let latest = repo.latest_regime().unwrap();
        assert!(latest.is_some());
        assert_eq!(latest.unwrap().regime, MarketRegime::Bull);
    }

    #[test]
    fn latest_regime_empty_returns_none() {
        let repo = repo();
        assert!(repo.latest_regime().unwrap().is_none());
    }

    // ── Flagged stocks ──────────────────────────────────────────────────────

    fn sample_flagged(ticker: &str) -> FlaggedStock {
        FlaggedStock {
            id: None,
            ticker: ticker.to_string(),
            flagged_date: NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            entry_score: 85.0,
            entry_price: 150.0,
            conditions_at_entry: 7,
            status: FlaggedStatus::Active,
            exit_date: None,
            exit_reason: None,
            exit_price: None,
            latest_score: None,
            last_rescored_at: None,
        }
    }

    #[test]
    fn insert_and_get_flagged_stocks() {
        let repo = repo();
        repo.insert_flagged_stock(&sample_flagged("NVDA")).unwrap();
        repo.insert_flagged_stock(&sample_flagged("AAPL")).unwrap();
        let active = repo.get_active_flagged_stocks().unwrap();
        assert_eq!(active.len(), 2);
        // Ordered by entry_score DESC — both 85.0, so ticker order
        assert!(active.iter().any(|s| s.ticker == "NVDA"));
        assert!(active.iter().any(|s| s.ticker == "AAPL"));
    }

    #[test]
    fn flagged_stock_upsert_updates_existing() {
        let repo = repo();
        repo.insert_flagged_stock(&sample_flagged("NVDA")).unwrap();
        let mut updated = sample_flagged("NVDA");
        updated.entry_score = 92.0;
        updated.entry_price = 160.0;
        repo.insert_flagged_stock(&updated).unwrap();
        let active = repo.get_active_flagged_stocks().unwrap();
        assert_eq!(active.len(), 1, "Upsert should not duplicate");
        assert_eq!(active[0].entry_score, 92.0);
    }

    #[test]
    fn mark_exited_updates_status() {
        let repo = repo();
        repo.insert_flagged_stock(&sample_flagged("NVDA")).unwrap();
        let exit_date = NaiveDate::from_ymd_opt(2024, 7, 1).unwrap();
        repo.mark_exited("NVDA", exit_date, "Stage2Breakdown", 130.0)
            .unwrap();
        let active = repo.get_active_flagged_stocks().unwrap();
        assert!(
            active.is_empty(),
            "Exited stock should not appear in active list"
        );
    }

    #[test]
    fn mark_exited_unknown_ticker_returns_error() {
        let repo = repo();
        let exit_date = NaiveDate::from_ymd_opt(2024, 7, 1).unwrap();
        let result = repo.mark_exited("UNKNOWN", exit_date, "test", 100.0);
        assert!(result.is_err());
    }

    #[test]
    fn expire_old_flags_removes_stale_entries() {
        let repo = repo();
        // Insert a stock flagged 100 days ago
        let mut old_flag = sample_flagged("OLD");
        old_flag.flagged_date = chrono::Utc::now().date_naive() - chrono::Duration::days(100);
        repo.insert_flagged_stock(&old_flag).unwrap();

        // Insert a recent stock
        let mut new_flag = sample_flagged("NEW");
        new_flag.flagged_date = chrono::Utc::now().date_naive();
        repo.insert_flagged_stock(&new_flag).unwrap();

        let expired = repo.expire_old_flags(90).unwrap();
        assert_eq!(expired, 1, "Should expire 1 old flag");

        let active = repo.get_active_flagged_stocks().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].ticker, "NEW");
    }

    #[test]
    fn get_active_empty_returns_empty() {
        let repo = repo();
        let active = repo.get_active_flagged_stocks().unwrap();
        assert!(active.is_empty());
    }

    // ── Recent alerts ────────────────────────────────────────────────────

    #[test]
    fn get_recent_alerts_empty_returns_empty() {
        let repo = repo();
        let entries = repo.get_recent_alerts(10).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn get_recent_alerts_returns_recorded_alerts() {
        use crate::data::models::Severity;
        let repo = repo();
        let alert = OutboundAlert {
            ticker: "NVDA".into(),
            alert_type: "NewSignal".into(),
            severity: Severity::Critical,
            message: "NVDA: score=92.0".into(),
            composite_score: 92.0,
            narrative: None,
        };
        repo.record_alert(&alert).unwrap();
        let entries = repo.get_recent_alerts(10).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].ticker, "NVDA");
        assert_eq!(entries[0].severity, "CRITICAL");
        assert_eq!(entries[0].alert_type, "NewSignal");
    }

    #[test]
    fn get_recent_alerts_respects_limit() {
        use crate::data::models::Severity;
        let repo = repo();
        for ticker in &["NVDA", "AMD", "AAPL", "TSLA", "MSFT"] {
            let alert = OutboundAlert {
                ticker: ticker.to_string(),
                alert_type: "NewSignal".into(),
                severity: Severity::Warning,
                message: format!("{}: test", ticker),
                composite_score: 75.0,
                narrative: None,
            };
            repo.record_alert(&alert).unwrap();
        }
        let entries = repo.get_recent_alerts(3).unwrap();
        assert_eq!(entries.len(), 3, "Should return at most 3 entries");
    }

    // ── Macro risk ────────────────────────────────────────────────────────

    #[test]
    fn save_and_retrieve_macro_risk() {
        use crate::analysis::macro_risk::{MacroRiskLevel, MacroRiskSnapshot};
        let repo = repo();
        let snapshot = MacroRiskSnapshot {
            vix_level: 18.5,
            hy_oas_bps: Some(380.0),
            yield_curve_2s10s: Some(0.25),
            put_call_ratio: Some(0.85),
            risk_level: MacroRiskLevel::Normal,
        };
        let date = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
        repo.save_macro_risk(date, &snapshot).unwrap();
        let latest = repo.latest_macro_risk().unwrap();
        assert!(latest.is_some());
        let s = latest.unwrap();
        assert!((s.vix_level - 18.5).abs() < 0.01);
        assert_eq!(s.risk_level, MacroRiskLevel::Normal);
    }

    #[test]
    fn latest_macro_risk_empty_returns_none() {
        let repo = repo();
        assert!(repo.latest_macro_risk().unwrap().is_none());
    }

    #[test]
    fn macro_risk_round_trip_with_nones() {
        use crate::analysis::macro_risk::{MacroRiskLevel, MacroRiskSnapshot};
        let repo = repo();
        let snapshot = MacroRiskSnapshot {
            vix_level: 35.0,
            hy_oas_bps: None,
            yield_curve_2s10s: None,
            put_call_ratio: None,
            risk_level: MacroRiskLevel::HighRisk,
        };
        let date = NaiveDate::from_ymd_opt(2024, 6, 15).unwrap();
        repo.save_macro_risk(date, &snapshot).unwrap();
        let latest = repo.latest_macro_risk().unwrap().unwrap();
        assert_eq!(latest.risk_level, MacroRiskLevel::HighRisk);
        assert!(latest.hy_oas_bps.is_none());
    }

    #[test]
    fn get_recent_alerts_returns_all_recorded() {
        use crate::data::models::Severity;
        let repo = repo();
        // Record multiple alerts — ordering depends on sent_at which may
        // share the same second; just verify both are returned
        let alert1 = OutboundAlert {
            ticker: "FIRST".into(),
            alert_type: "NewSignal".into(),
            severity: Severity::Info,
            message: "first".into(),
            composite_score: 55.0,
            narrative: None,
        };
        repo.record_alert(&alert1).unwrap();
        let alert2 = OutboundAlert {
            ticker: "SECOND".into(),
            alert_type: "NewSignal".into(),
            severity: Severity::Critical,
            message: "second".into(),
            composite_score: 92.0,
            narrative: None,
        };
        repo.record_alert(&alert2).unwrap();
        let entries = repo.get_recent_alerts(10).unwrap();
        assert_eq!(entries.len(), 2);
        let tickers: Vec<&str> = entries.iter().map(|e| e.ticker.as_str()).collect();
        assert!(tickers.contains(&"FIRST"));
        assert!(tickers.contains(&"SECOND"));
    }

    // ── Backtest runs ────────────────────────────────────────────────────

    #[test]
    fn save_and_retrieve_backtest_run() {
        let repo = repo();
        repo.save_backtest_report(
            r#"{"forward_windows":[30,60,90]}"#,
            r#"{"hit_rate_60d":0.65}"#,
            r#"{"full":"report"}"#,
            50,
            12.5,
        )
        .unwrap();
        let runs = repo.get_backtest_runs(10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].signal_count, 50);
        assert!((runs[0].duration_secs - 12.5).abs() < 1e-6);
        assert!(runs[0].kpis_json.contains("hit_rate_60d"));
    }

    // ── News cache ─────────────────────────────────────────────────────

    fn sample_news(ticker: &str, headline: &str, days_ago: i64) -> NewsArticle {
        NewsArticle {
            ticker: ticker.to_string(),
            headline: headline.to_string(),
            summary: format!("Summary of {}", headline),
            published_at: chrono::Utc::now() - chrono::Duration::days(days_ago),
            source: "TestSource".to_string(),
        }
    }

    #[test]
    fn cache_news_inserts_articles() {
        let repo = repo();
        let articles = vec![
            sample_news("AAPL", "Apple Beats Earnings", 1),
            sample_news("AAPL", "New iPhone Launch", 2),
        ];
        let inserted = repo.cache_news(&articles).unwrap();
        assert_eq!(inserted, 2);

        let cached = repo
            .get_cached_news(
                "AAPL",
                chrono::Utc::now().date_naive() - chrono::Duration::days(7),
            )
            .unwrap();
        assert_eq!(cached.len(), 2);
    }

    #[test]
    fn cache_news_deduplicates_same_headline() {
        let repo = repo();
        let articles = vec![sample_news("AAPL", "Apple Beats Earnings", 1)];
        let first = repo.cache_news(&articles).unwrap();
        assert_eq!(first, 1);

        // Insert again — same ticker + headline → INSERT OR IGNORE
        let second = repo.cache_news(&articles).unwrap();
        assert_eq!(second, 0, "Duplicate should be ignored");

        let cached = repo
            .get_cached_news(
                "AAPL",
                chrono::Utc::now().date_naive() - chrono::Duration::days(7),
            )
            .unwrap();
        assert_eq!(cached.len(), 1, "Should only have one article");
    }

    #[test]
    fn get_cached_news_respects_since_date() {
        let repo = repo();
        let articles = vec![
            sample_news("AAPL", "Recent news", 1),
            sample_news("AAPL", "Old news", 30),
        ];
        repo.cache_news(&articles).unwrap();

        let since = chrono::Utc::now().date_naive() - chrono::Duration::days(7);
        let cached = repo.get_cached_news("AAPL", since).unwrap();
        assert_eq!(
            cached.len(),
            1,
            "Should only return articles after since date"
        );
        assert_eq!(cached[0].headline, "Recent news");
    }

    #[test]
    fn delete_old_news_removes_stale() {
        let repo = repo();
        let articles = vec![
            sample_news("AAPL", "Recent", 1),
            sample_news("AAPL", "Old", 30),
        ];
        repo.cache_news(&articles).unwrap();

        let deleted = repo.delete_old_news(7).unwrap();
        assert_eq!(deleted, 1, "Should delete 1 old article");

        let all = repo
            .get_cached_news(
                "AAPL",
                chrono::Utc::now().date_naive() - chrono::Duration::days(365),
            )
            .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].headline, "Recent");
    }

    #[test]
    fn get_backtest_runs_empty_returns_empty() {
        let repo = repo();
        let runs = repo.get_backtest_runs(10).unwrap();
        assert!(runs.is_empty());
    }

    #[test]
    fn backtest_run_round_trip() {
        let repo = repo();
        let params = r#"{"mc_iterations":10000}"#;
        let kpis = r#"{"sharpe":1.2,"hit_rate":0.68}"#;
        let report = r#"{"full":"backtest_report","sections":["kpis","wf","mc"]}"#;
        repo.save_backtest_report(params, kpis, report, 100, 30.0)
            .unwrap();
        let runs = repo.get_backtest_runs(1).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].params_json, params);
        assert_eq!(runs[0].kpis_json, kpis);
        assert_eq!(runs[0].report_json, report);
        assert_eq!(runs[0].signal_count, 100);
    }

    // ── Watchlist ────────────────────────────────────────────────────────

    #[test]
    fn watchlist_add_and_list() {
        let repo = repo();
        repo.watchlist_add("NVDA", None).unwrap();
        repo.watchlist_add("AAPL", Some("Strong earnings")).unwrap();
        let entries = repo.watchlist_list().unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|e| e.ticker == "NVDA"));
        let aapl = entries.iter().find(|e| e.ticker == "AAPL").unwrap();
        assert_eq!(aapl.notes.as_deref(), Some("Strong earnings"));
    }

    #[test]
    fn watchlist_add_duplicate_is_noop() {
        let repo = repo();
        repo.watchlist_add("NVDA", None).unwrap();
        repo.watchlist_add("NVDA", Some("duplicate")).unwrap();
        let entries = repo.watchlist_list().unwrap();
        assert_eq!(entries.len(), 1, "Duplicate add should be a no-op");
        // Original notes should be preserved (ON CONFLICT DO NOTHING)
        assert!(entries[0].notes.is_none());
    }

    #[test]
    fn watchlist_remove_existing_ticker() {
        let repo = repo();
        repo.watchlist_add("NVDA", None).unwrap();
        repo.watchlist_add("AAPL", None).unwrap();
        let removed = repo.watchlist_remove("NVDA").unwrap();
        assert!(removed, "Should return true when ticker existed");
        let entries = repo.watchlist_list().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].ticker, "AAPL");
    }

    #[test]
    fn watchlist_remove_nonexistent_returns_false() {
        let repo = repo();
        let removed = repo.watchlist_remove("UNKNOWN").unwrap();
        assert!(!removed, "Should return false for non-existent ticker");
    }

    #[test]
    fn watchlist_tickers_returns_correct_set() {
        let repo = repo();
        repo.watchlist_add("NVDA", None).unwrap();
        repo.watchlist_add("AAPL", None).unwrap();
        repo.watchlist_add("TSLA", None).unwrap();
        repo.watchlist_remove("TSLA").unwrap();
        let tickers = repo.watchlist_tickers().unwrap();
        assert_eq!(tickers.len(), 2);
        assert!(tickers.contains(&"NVDA".to_string()));
        assert!(tickers.contains(&"AAPL".to_string()));
        assert!(!tickers.contains(&"TSLA".to_string()));
    }

    #[test]
    fn watchlist_update_score_persists_grade() {
        let repo = repo();
        repo.watchlist_add("NVDA", None).unwrap();
        repo.watchlist_update_score("NVDA", 94.2, "A+").unwrap();
        let entries = repo.watchlist_list().unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert!((entry.last_score.unwrap() - 94.2).abs() < 0.01);
        assert_eq!(entry.last_grade.as_deref(), Some("A+"));
        assert!(entry.last_screen.is_some(), "last_screen should be set");
    }

    #[test]
    fn watchlist_clear_removes_all() {
        let repo = repo();
        repo.watchlist_add("NVDA", None).unwrap();
        repo.watchlist_add("AAPL", None).unwrap();
        repo.watchlist_add("TSLA", None).unwrap();
        let deleted = repo.watchlist_clear().unwrap();
        assert_eq!(deleted, 3);
        let entries = repo.watchlist_list().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn watchlist_list_empty_returns_empty_vec() {
        let repo = repo();
        let entries = repo.watchlist_list().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn watchlist_tickers_empty_returns_empty_vec() {
        let repo = repo();
        let tickers = repo.watchlist_tickers().unwrap();
        assert!(tickers.is_empty());
    }

    #[test]
    fn watchlist_update_score_persists() {
        let repo = repo();
        repo.watchlist_add("NVDA", None).unwrap();
        repo.watchlist_update_score("NVDA", 88.5, "B+").unwrap();
        let entries = repo.watchlist_list().unwrap();
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert!(
            (entry.last_score.unwrap() - 88.5).abs() < 0.01,
            "Score should persist as 88.5"
        );
        assert_eq!(
            entry.last_grade.as_deref(),
            Some("B+"),
            "Grade should persist as B+"
        );
    }

    #[test]
    fn watchlist_clear_empties_table() {
        let repo = repo();
        repo.watchlist_add("NVDA", None).unwrap();
        repo.watchlist_add("AAPL", None).unwrap();
        repo.watchlist_add("TSLA", None).unwrap();
        let deleted = repo.watchlist_clear().unwrap();
        assert_eq!(deleted, 3, "Should report 3 rows deleted");
        let entries = repo.watchlist_list().unwrap();
        assert!(entries.is_empty(), "Watchlist should be empty after clear");
    }

    // ── Screen results range ─────────────────────────────────────────────

    fn seed_screen_results(conn: &Connection) {
        conn.execute_batch("
            INSERT INTO screen_results (run_date, ticker, minervini_pass, conditions_met, adx, rs_percentile, volume_zscore, technical_score, volume_score, sentiment_score, composite_score, grade, close_price, pct_from_52w_high, weighted_rs)
            VALUES ('2026-03-01', 'NVDA', 1, 8, 35.0, 97.0, 2.8, 48.0, 25.0, 10.0, 94.2, 'A+', 875.50, -2.1, 0.95);
            INSERT INTO screen_results (run_date, ticker, minervini_pass, conditions_met, adx, rs_percentile, volume_zscore, technical_score, volume_score, sentiment_score, composite_score, grade, close_price, pct_from_52w_high, weighted_rs)
            VALUES ('2026-03-01', 'AMD', 1, 7, 28.0, 85.0, 1.5, 40.0, 20.0, 8.0, 78.5, 'B', 165.30, -5.0, 0.80);
            INSERT INTO screen_results (run_date, ticker, minervini_pass, conditions_met, adx, rs_percentile, volume_zscore, technical_score, volume_score, sentiment_score, composite_score, grade, close_price, pct_from_52w_high, weighted_rs)
            VALUES ('2026-03-02', 'NVDA', 1, 8, 36.0, 96.0, 2.5, 47.0, 24.0, 9.5, 92.0, 'A', 880.00, -1.8, 0.93);
            INSERT INTO screen_results (run_date, ticker, minervini_pass, conditions_met, adx, rs_percentile, volume_zscore, technical_score, volume_score, sentiment_score, composite_score, grade, close_price, pct_from_52w_high, weighted_rs)
            VALUES ('2026-03-05', 'TSLA', 1, 6, 22.0, 70.0, 1.0, 32.0, 15.0, 5.0, 60.0, 'C', 245.00, -12.0, 0.65);
        ").unwrap();
    }

    #[test]
    fn get_screen_results_range_returns_matching_rows() {
        let repo = repo();
        let conn = repo.connection().unwrap();
        seed_screen_results(&conn);
        drop(conn);

        let results = repo
            .get_screen_results_range("2026-03-01", "2026-03-02")
            .unwrap();
        assert_eq!(results.len(), 3);
        // First date, highest score first
        assert_eq!(results[0].ticker, "NVDA");
        assert_eq!(results[0].run_date, "2026-03-01");
        assert!((results[0].composite_score - 94.2).abs() < 0.01);
        // AMD is second on the same date
        assert_eq!(results[1].ticker, "AMD");
        // Third row is 2026-03-02
        assert_eq!(results[2].run_date, "2026-03-02");
    }

    #[test]
    fn get_screen_results_range_empty_when_no_match() {
        let repo = repo();
        let conn = repo.connection().unwrap();
        seed_screen_results(&conn);
        drop(conn);

        let results = repo
            .get_screen_results_range("2020-01-01", "2020-12-31")
            .unwrap();
        assert!(results.is_empty());
    }

    // ── Regime history ───────────────────────────────────────────────────

    #[test]
    fn get_regime_history_returns_chronological() {
        let repo = repo();
        for (date, regime) in &[
            ("2026-03-01", MarketRegime::Bull),
            ("2026-03-02", MarketRegime::Sideways),
            ("2026-03-03", MarketRegime::Bear),
        ] {
            let state = RegimeState {
                date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
                spy_close: 480.0,
                spy_sma200: 440.0,
                breadth_pct: 55.0,
                regime: regime.clone(),
            };
            repo.save_regime(&state).unwrap();
        }

        let history = repo.get_regime_history(10).unwrap();
        assert_eq!(history.len(), 3);
        // Should be chronological (ASC)
        assert_eq!(history[0].regime, MarketRegime::Bull);
        assert_eq!(history[1].regime, MarketRegime::Sideways);
        assert_eq!(history[2].regime, MarketRegime::Bear);
    }

    #[test]
    fn get_regime_history_empty_returns_empty() {
        let repo = repo();
        let history = repo.get_regime_history(10).unwrap();
        assert!(history.is_empty());
    }

    #[test]
    fn get_regime_history_respects_limit() {
        let repo = repo();
        for i in 1..=5 {
            let state = RegimeState {
                date: NaiveDate::from_ymd_opt(2026, 3, i).unwrap(),
                spy_close: 480.0,
                spy_sma200: 440.0,
                breadth_pct: 55.0,
                regime: MarketRegime::Bull,
            };
            repo.save_regime(&state).unwrap();
        }

        let history = repo.get_regime_history(3).unwrap();
        assert_eq!(history.len(), 3);
        // Limit 3 from DESC means most recent 3, then reversed to ASC
        assert_eq!(
            history[0].date,
            NaiveDate::from_ymd_opt(2026, 3, 3).unwrap()
        );
        assert_eq!(
            history[2].date,
            NaiveDate::from_ymd_opt(2026, 3, 5).unwrap()
        );
    }

    // ── Macro risk history ───────────────────────────────────────────────

    #[test]
    fn get_macro_risk_history_returns_chronological() {
        use crate::analysis::macro_risk::{MacroRiskLevel, MacroRiskSnapshot};
        let repo = repo();
        for (i, level) in [
            (1, MacroRiskLevel::Normal),
            (2, MacroRiskLevel::Elevated),
            (3, MacroRiskLevel::HighRisk),
        ] {
            let snapshot = MacroRiskSnapshot {
                vix_level: 15.0 + i as f64 * 5.0,
                hy_oas_bps: Some(350.0),
                yield_curve_2s10s: Some(0.2),
                put_call_ratio: Some(0.85),
                risk_level: level,
            };
            let date = NaiveDate::from_ymd_opt(2026, 3, i).unwrap();
            repo.save_macro_risk(date, &snapshot).unwrap();
        }

        let history = repo.get_macro_risk_history(10).unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].risk_level, MacroRiskLevel::Normal);
        assert_eq!(history[2].risk_level, MacroRiskLevel::HighRisk);
    }

    #[test]
    fn get_macro_risk_history_empty_returns_empty() {
        let repo = repo();
        let history = repo.get_macro_risk_history(10).unwrap();
        assert!(history.is_empty());
    }

    // ── Grade distribution ───────────────────────────────────────────────

    #[test]
    fn get_grade_distribution_groups_correctly() {
        let repo = repo();
        let conn = repo.connection().unwrap();
        seed_screen_results(&conn);
        drop(conn);

        let dist = repo
            .get_grade_distribution("2026-03-01", "2026-03-05")
            .unwrap();
        // 2026-03-01 has A+ and B; 2026-03-02 has A; 2026-03-05 has C
        assert_eq!(dist.len(), 4);
        // First group is 2026-03-01
        assert_eq!(dist[0].run_date, "2026-03-01");
    }

    #[test]
    fn get_grade_distribution_empty_range() {
        let repo = repo();
        let dist = repo
            .get_grade_distribution("2020-01-01", "2020-12-31")
            .unwrap();
        assert!(dist.is_empty());
    }

    // ── Alert counts by date ─────────────────────────────────────────────

    #[test]
    fn get_alert_counts_by_date_aggregates() {
        let repo = repo();
        let conn = repo.connection().unwrap();
        conn.execute_batch(
            "
            INSERT INTO alert_log (ticker, alert_type, severity, message, sent_at)
            VALUES ('NVDA', 'NewSignal', 'CRITICAL', 'msg1', '2026-03-01 10:00:00');
        ",
        )
        .unwrap();
        // The unique index is (ticker, alert_type), so use different combos
        conn.execute_batch(
            "
            INSERT INTO alert_log (ticker, alert_type, severity, message, sent_at)
            VALUES ('AMD', 'NewSignal', 'WARNING', 'msg2', '2026-03-01 11:00:00');
        ",
        )
        .unwrap();
        conn.execute_batch(
            "
            INSERT INTO alert_log (ticker, alert_type, severity, message, sent_at)
            VALUES ('TSLA', 'ExitSignal', 'CRITICAL', 'msg3', '2026-03-02 09:00:00');
        ",
        )
        .unwrap();
        drop(conn);

        let counts = repo.get_alert_counts_by_date("2026-03-01").unwrap();
        assert_eq!(counts.len(), 2);
        // First day
        assert_eq!(counts[0].date, "2026-03-01");
        assert_eq!(counts[0].total, 2);
        assert_eq!(counts[0].critical, 1);
        // Second day
        assert_eq!(counts[1].date, "2026-03-02");
        assert_eq!(counts[1].total, 1);
        assert_eq!(counts[1].critical, 1);
    }

    #[test]
    fn get_alert_counts_by_date_empty() {
        let repo = repo();
        let counts = repo.get_alert_counts_by_date("2026-01-01").unwrap();
        assert!(counts.is_empty());
    }

    // ── Ticker score history ─────────────────────────────────────────────

    #[test]
    fn get_ticker_score_history_returns_chronological() {
        let repo = repo();
        let conn = repo.connection().unwrap();
        seed_screen_results(&conn);
        drop(conn);

        let history = repo.get_ticker_score_history("NVDA", "2026-03-01").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].0, "2026-03-01");
        assert!((history[0].1 - 94.2).abs() < 0.01);
        assert_eq!(history[1].0, "2026-03-02");
        assert!((history[1].1 - 92.0).abs() < 0.01);
    }

    #[test]
    fn get_ticker_score_history_no_data_returns_empty() {
        let repo = repo();
        let history = repo
            .get_ticker_score_history("UNKNOWN", "2026-01-01")
            .unwrap();
        assert!(history.is_empty());
    }

    // ── Grade round-trip ──────────────────────────────────────────────────

    #[test]
    fn grade_aplus_round_trips_through_save_and_get() {
        use crate::analysis::indicators::LiquidityClass;
        use crate::analysis::scorer::Grade;
        use crate::analysis::screener::ScreenResult;

        let repo = repo();
        let date = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap();
        let result = ScreenResult {
            ticker: "TEST".to_string(),
            minervini_pass: true,
            conditions_met: 8,
            adx: 35.0,
            rs_percentile: 97.0,
            volume_zscore: 2.8,
            technical_score: 48.0,
            volume_score: 25.0,
            sentiment_score: 10.0,
            composite_score: 94.0,
            grade: Grade::APlus,
            close_price: 100.0,
            pct_from_52w_high: -2.0,
            atr_value: 3.0,
            risk_reward: 2.5,
            liquidity: LiquidityClass::High,
            weighted_rs: 0.95,
            rs_momentum: None,
            vcp: None,
            regime: MarketRegime::Bull,
        };
        repo.save_screen_result(date, &result).unwrap();

        let loaded = repo.get_screen_results(date).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(
            loaded[0].grade,
            Grade::APlus,
            "A+ grade must survive round-trip"
        );
    }

    // ── Decision Audit Log ──────────────────────────────────────────────────

    fn sample_decision(
        ticker: Option<&str>,
        date: &str,
    ) -> crate::portfolio::decision::DecisionLogEntry {
        use crate::portfolio::decision::{DecisionLogEntry, DecisionType};
        DecisionLogEntry {
            id: None,
            decision_date: NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap(),
            decision_type: DecisionType::Buy,
            ticker: ticker.map(|t| t.to_string()),
            shares: Some(50.0),
            price: Some(150.0),
            rationale: "Strong technical setup".to_string(),
            advisor: Some("system".to_string()),
            linked_alert_id: None,
            created_at: None,
        }
    }

    #[test]
    fn test_record_and_get_decision() {
        use crate::portfolio::decision::DecisionType;

        let repo = repo();
        let mut entry = sample_decision(Some("AAPL"), "2026-03-14");
        entry.shares = Some(100.0);
        entry.price = Some(175.50);
        entry.rationale = "Taking profits after 30% gain".to_string();
        entry.decision_type = DecisionType::Sell;
        entry.advisor = Some("manual".to_string());
        entry.linked_alert_id = Some(7);

        let id = repo.record_decision(&entry).unwrap();
        assert!(id > 0);

        let decisions = repo.get_decisions(10).unwrap();
        assert_eq!(decisions.len(), 1);

        let d = &decisions[0];
        assert_eq!(d.id, Some(id));
        assert_eq!(d.decision_date.to_string(), "2026-03-14");
        assert_eq!(d.decision_type, DecisionType::Sell);
        assert_eq!(d.ticker.as_deref(), Some("AAPL"));
        assert_eq!(d.shares, Some(100.0));
        assert_eq!(d.price, Some(175.50));
        assert_eq!(d.rationale, "Taking profits after 30% gain");
        assert_eq!(d.advisor.as_deref(), Some("manual"));
        assert_eq!(d.linked_alert_id, Some(7));
        assert!(d.created_at.is_some());
    }

    #[test]
    fn test_record_decision_returns_id() {
        let repo = repo();
        let entry = sample_decision(Some("MSFT"), "2026-03-10");
        let id = repo.record_decision(&entry).unwrap();
        assert!(id > 0, "returned id should be positive");
    }

    #[test]
    fn test_get_decisions_respects_limit() {
        let repo = repo();
        for i in 1..=5 {
            let entry = sample_decision(Some("TSLA"), &format!("2026-03-{:02}", i));
            repo.record_decision(&entry).unwrap();
        }
        let decisions = repo.get_decisions(3).unwrap();
        assert_eq!(decisions.len(), 3, "limit=3 should return exactly 3 rows");
    }

    #[test]
    fn test_get_decisions_orders_by_date_desc() {
        let repo = repo();
        for date in &["2026-03-01", "2026-03-15", "2026-03-10"] {
            let entry = sample_decision(Some("GOOG"), date);
            repo.record_decision(&entry).unwrap();
        }
        let decisions = repo.get_decisions(10).unwrap();
        assert_eq!(decisions.len(), 3);
        assert_eq!(decisions[0].decision_date.to_string(), "2026-03-15");
        assert_eq!(decisions[1].decision_date.to_string(), "2026-03-10");
        assert_eq!(decisions[2].decision_date.to_string(), "2026-03-01");
    }

    #[test]
    fn test_get_decisions_for_ticker() {
        use crate::portfolio::decision::DecisionType;

        let repo = repo();
        let amzn = sample_decision(Some("AMZN"), "2026-03-05");
        let msft = sample_decision(Some("MSFT"), "2026-03-06");
        let amzn2 = {
            let mut e = sample_decision(Some("AMZN"), "2026-03-07");
            e.decision_type = DecisionType::Hold;
            e
        };
        repo.record_decision(&amzn).unwrap();
        repo.record_decision(&msft).unwrap();
        repo.record_decision(&amzn2).unwrap();

        let amzn_decisions = repo.get_decisions_for_ticker("AMZN", 10).unwrap();
        assert_eq!(amzn_decisions.len(), 2, "should only return AMZN decisions");
        for d in &amzn_decisions {
            assert_eq!(d.ticker.as_deref(), Some("AMZN"));
        }
        // Most recent first
        assert_eq!(amzn_decisions[0].decision_date.to_string(), "2026-03-07");
        assert_eq!(amzn_decisions[0].decision_type, DecisionType::Hold);
    }

    // ── Correlation Matrix ──────────────────────────────────────────────────

    #[test]
    fn save_and_get_correlation_snapshot() {
        let repo = repo();
        let tickers = vec!["AAPL".to_string(), "MSFT".to_string()];
        let matrix = vec![vec![1.0, 0.85], vec![0.85, 1.0]];
        repo.save_correlation_snapshot("2026-03-14", &tickers, &matrix, 60)
            .unwrap();
        let snap = repo.get_latest_correlation_snapshot().unwrap();
        assert!(snap.is_some());
        let (date, t, m, w) = snap.unwrap();
        assert_eq!(date, "2026-03-14");
        assert_eq!(t, tickers);
        assert_eq!(m, matrix);
        assert_eq!(w, 60);
    }

    #[test]
    fn get_latest_correlation_snapshot_empty_returns_none() {
        let repo = repo();
        assert!(repo.get_latest_correlation_snapshot().unwrap().is_none());
    }

    #[test]
    fn correlation_snapshot_latest_wins() {
        let repo = repo();
        let t1 = vec!["A".to_string()];
        let m1 = vec![vec![1.0]];
        repo.save_correlation_snapshot("2026-03-01", &t1, &m1, 30)
            .unwrap();
        let t2 = vec!["B".to_string(), "C".to_string()];
        let m2 = vec![vec![1.0, 0.5], vec![0.5, 1.0]];
        repo.save_correlation_snapshot("2026-03-14", &t2, &m2, 90)
            .unwrap();
        let (date, tickers, _, window) = repo.get_latest_correlation_snapshot().unwrap().unwrap();
        assert_eq!(date, "2026-03-14");
        assert_eq!(tickers, t2);
        assert_eq!(window, 90);
    }

    // ── Fundamental History ─────────────────────────────────────────────────

    fn sample_fundamental_history(
        ticker: &str,
        period_date: &str,
    ) -> crate::data::fundamentals::FundamentalHistoryRecord {
        crate::data::fundamentals::FundamentalHistoryRecord {
            ticker: ticker.to_string(),
            period_date: period_date.to_string(),
            period_type: "Q1".to_string(),
            pe_ratio: Some(25.0),
            pb_ratio: Some(5.0),
            ps_ratio: Some(8.0),
            ev_ebitda: Some(18.0),
            ev_revenue: Some(12.0),
            gross_margin: Some(0.65),
            operating_margin: Some(0.35),
            net_margin: Some(0.25),
            roe: Some(0.30),
            debt_to_equity: Some(0.5),
            revenue: Some(50_000_000_000.0),
            ebitda: Some(20_000_000_000.0),
            fcf: Some(15_000_000_000.0),
        }
    }

    #[test]
    fn upsert_and_get_fundamental_history() {
        let repo = repo();
        let r = sample_fundamental_history("AAPL", "2025-03-31");
        repo.upsert_fundamental_history(&r).unwrap();
        let history = repo.get_fundamental_history("AAPL", 10).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].ticker, "AAPL");
        assert_eq!(history[0].period_date, "2025-03-31");
        assert!((history[0].pe_ratio.unwrap() - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn upsert_fundamental_history_updates_on_conflict() {
        let repo = repo();
        let mut r = sample_fundamental_history("AAPL", "2025-03-31");
        repo.upsert_fundamental_history(&r).unwrap();
        r.pe_ratio = Some(30.0);
        repo.upsert_fundamental_history(&r).unwrap();
        let history = repo.get_fundamental_history("AAPL", 10).unwrap();
        assert_eq!(history.len(), 1, "upsert should not duplicate");
        assert!((history[0].pe_ratio.unwrap() - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn get_fundamental_history_respects_limit() {
        let repo = repo();
        for i in 1..=5 {
            let r = sample_fundamental_history("AAPL", &format!("2025-{:02}-30", i));
            repo.upsert_fundamental_history(&r).unwrap();
        }
        let history = repo.get_fundamental_history("AAPL", 3).unwrap();
        assert_eq!(history.len(), 3);
    }

    #[test]
    fn get_fundamental_history_empty_returns_empty() {
        let repo = repo();
        let history = repo.get_fundamental_history("UNKNOWN", 10).unwrap();
        assert!(history.is_empty());
    }

    #[test]
    fn get_valuation_bands_returns_data() {
        let repo = repo();
        let r = sample_fundamental_history("AAPL", "2025-03-31");
        repo.upsert_fundamental_history(&r).unwrap();
        let bands = repo.get_valuation_bands("AAPL", "pe_ratio", 5).unwrap();
        assert_eq!(bands.len(), 1);
        assert_eq!(bands[0].0, "2025-03-31");
        assert!((bands[0].1 - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn get_valuation_bands_rejects_invalid_metric() {
        let repo = repo();
        let result = repo.get_valuation_bands("AAPL", "drop_table", 5);
        assert!(result.is_err(), "Should reject invalid metric");
    }

    #[test]
    fn get_valuation_bands_empty_returns_empty() {
        let repo = repo();
        let bands = repo.get_valuation_bands("UNKNOWN", "pe_ratio", 5).unwrap();
        assert!(bands.is_empty());
    }

    // ── ETF Snapshots ───────────────────────────────────────────────────────

    fn sample_etf_snapshot(ticker: &str) -> crate::data::etf::EtfSnapshot {
        crate::data::etf::EtfSnapshot {
            ticker: ticker.to_string(),
            name: format!("{} ETF", ticker),
            expense_ratio: 0.0003,
            aum: 100_000_000_000.0,
            avg_daily_volume: 5_000_000,
            category: "US Large Cap".to_string(),
            inception_date: "2010-01-01".to_string(),
            tracking_index: "S&P 500".to_string(),
            top_holdings: vec![crate::data::etf::EtfHolding {
                ticker: "AAPL".to_string(),
                name: "Apple Inc".to_string(),
                weight: 0.07,
            }],
        }
    }

    #[test]
    fn upsert_and_get_etf_snapshot() {
        let repo = repo();
        let snap = sample_etf_snapshot("VOO");
        repo.upsert_etf_snapshot(&snap).unwrap();
        let loaded = repo.get_etf_snapshot("VOO").unwrap();
        assert!(loaded.is_some());
        let s = loaded.unwrap();
        assert_eq!(s.ticker, "VOO");
        assert_eq!(s.name, "VOO ETF");
        assert!((s.expense_ratio - 0.0003).abs() < f64::EPSILON);
        assert_eq!(s.top_holdings.len(), 1);
        assert_eq!(s.top_holdings[0].ticker, "AAPL");
    }

    #[test]
    fn upsert_etf_snapshot_replaces_on_conflict() {
        let repo = repo();
        let mut snap = sample_etf_snapshot("VOO");
        repo.upsert_etf_snapshot(&snap).unwrap();
        snap.expense_ratio = 0.0005;
        repo.upsert_etf_snapshot(&snap).unwrap();
        let loaded = repo.get_etf_snapshot("VOO").unwrap().unwrap();
        assert!((loaded.expense_ratio - 0.0005).abs() < f64::EPSILON);
    }

    #[test]
    fn get_etf_snapshot_missing_returns_none() {
        let repo = repo();
        assert!(repo.get_etf_snapshot("UNKNOWN").unwrap().is_none());
    }

    #[test]
    fn get_all_etf_snapshots_returns_all() {
        let repo = repo();
        repo.upsert_etf_snapshot(&sample_etf_snapshot("VOO"))
            .unwrap();
        repo.upsert_etf_snapshot(&sample_etf_snapshot("VTI"))
            .unwrap();
        let all = repo.get_all_etf_snapshots().unwrap();
        assert_eq!(all.len(), 2);
        // Ordered by ticker ASC
        assert_eq!(all[0].ticker, "VOO");
        assert_eq!(all[1].ticker, "VTI");
    }

    #[test]
    fn get_all_etf_snapshots_empty_returns_empty() {
        let repo = repo();
        let all = repo.get_all_etf_snapshots().unwrap();
        assert!(all.is_empty());
    }

    // ── Selling Programme ───────────────────────────────────────────────────

    fn sample_selling_programme(ticker: &str) -> crate::portfolio::selling::SellingProgramme {
        crate::portfolio::selling::SellingProgramme::build_schedule(ticker, &[100.0, 200.0], 2)
    }

    #[test]
    fn save_and_get_selling_programme() {
        let repo = repo();
        let prog = sample_selling_programme("AMZN");
        let id = repo.save_selling_programme(&prog).unwrap();
        assert!(id > 0);
        let loaded = repo.get_selling_programme(id).unwrap();
        assert!(loaded.is_some());
        let p = loaded.unwrap();
        assert_eq!(p.id, Some(id));
        assert_eq!(p.ticker, "AMZN");
        assert_eq!(p.schedule.len(), 8);
        assert!(p.created_at.is_some());
    }

    #[test]
    fn get_selling_programme_missing_returns_none() {
        let repo = repo();
        assert!(repo.get_selling_programme(999).unwrap().is_none());
    }

    #[test]
    fn get_selling_programmes_for_ticker() {
        let repo = repo();
        repo.save_selling_programme(&sample_selling_programme("AMZN"))
            .unwrap();
        repo.save_selling_programme(&sample_selling_programme("AMZN"))
            .unwrap();
        repo.save_selling_programme(&sample_selling_programme("GOOG"))
            .unwrap();
        let progs = repo.get_selling_programmes_for_ticker("AMZN").unwrap();
        assert_eq!(progs.len(), 2);
        for p in &progs {
            assert_eq!(p.ticker, "AMZN");
        }
    }

    #[test]
    fn get_selling_programmes_for_ticker_empty() {
        let repo = repo();
        let progs = repo.get_selling_programmes_for_ticker("UNKNOWN").unwrap();
        assert!(progs.is_empty());
    }

    #[test]
    fn update_selling_programme_modifies_schedule() {
        let repo = repo();
        let prog = sample_selling_programme("AMZN");
        let id = repo.save_selling_programme(&prog).unwrap();
        let mut loaded = repo.get_selling_programme(id).unwrap().unwrap();
        loaded.schedule[0].executed_shares = 25.0;
        loaded.schedule[0].executed_proceeds = 3750.0;
        repo.update_selling_programme(&loaded).unwrap();
        let reloaded = repo.get_selling_programme(id).unwrap().unwrap();
        assert!((reloaded.schedule[0].executed_shares - 25.0).abs() < f64::EPSILON);
        assert!((reloaded.schedule[0].executed_proceeds - 3750.0).abs() < f64::EPSILON);
    }

    #[test]
    fn update_selling_programme_no_id_returns_error() {
        let repo = repo();
        let prog = sample_selling_programme("AMZN");
        let result = repo.update_selling_programme(&prog);
        assert!(result.is_err(), "Should fail without id");
    }

    #[test]
    fn record_and_get_selling_executions() {
        use chrono::NaiveDate;
        let repo = repo();
        let prog = sample_selling_programme("AMZN");
        let prog_id = repo.save_selling_programme(&prog).unwrap();
        let exec = crate::portfolio::selling::SellingExecution {
            id: None,
            programme_id: prog_id,
            execution_date: NaiveDate::from_ymd_opt(2026, 3, 14).unwrap(),
            shares_sold: 25.0,
            price: 190.0,
            proceeds: 4750.0,
            lot_ids: vec!["LOT-001".to_string(), "LOT-002".to_string()],
            notes: Some("Q1 tranche".to_string()),
        };
        let exec_id = repo.record_selling_execution(&exec).unwrap();
        assert!(exec_id > 0);

        let execs = repo.get_selling_executions(prog_id).unwrap();
        assert_eq!(execs.len(), 1);
        let e = &execs[0];
        assert_eq!(e.id, Some(exec_id));
        assert_eq!(e.programme_id, prog_id);
        assert!((e.shares_sold - 25.0).abs() < f64::EPSILON);
        assert!((e.price - 190.0).abs() < f64::EPSILON);
        assert!((e.proceeds - 4750.0).abs() < f64::EPSILON);
        assert_eq!(e.lot_ids, vec!["LOT-001", "LOT-002"]);
        assert_eq!(e.notes.as_deref(), Some("Q1 tranche"));
    }

    #[test]
    fn get_selling_executions_empty_returns_empty() {
        let repo = repo();
        let execs = repo.get_selling_executions(999).unwrap();
        assert!(execs.is_empty());
    }

    // ── Tax Lots ─────────────────────────────────────────────────────────

    fn insert_tax_lot(
        repo: &SqliteRepository,
        id: &str,
        ticker: &str,
        shares: f64,
        cost: f64,
        date: &str,
        acq_type: &str,
    ) {
        let conn = repo.connection().unwrap();
        conn.execute(
            "INSERT INTO tax_lots (id, ticker, shares, cost_basis_per_share, acquired_date, acquisition_type, remaining_shares)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?3)",
            params![id, ticker, shares, cost, date, acq_type],
        ).unwrap();
    }

    #[test]
    fn get_all_tax_lots_empty() {
        let repo = repo();
        let lots = repo.get_all_tax_lots().unwrap();
        assert!(lots.is_empty());
    }

    #[test]
    fn get_all_tax_lots_returns_lots_with_remaining_shares() {
        let repo = repo();
        insert_tax_lot(&repo, "LOT-1", "AAPL", 100.0, 150.0, "2025-01-10", "BUY");
        insert_tax_lot(&repo, "LOT-2", "MSFT", 50.0, 300.0, "2025-02-15", "BUY");
        // Insert a fully sold lot (remaining_shares = 0) -- should be excluded
        {
            let conn = repo.connection().unwrap();
            conn.execute(
                "INSERT INTO tax_lots (id, ticker, shares, cost_basis_per_share, acquired_date, acquisition_type, remaining_shares)
                 VALUES ('LOT-3', 'GOOG', 10.0, 140.0, '2024-12-01', 'BUY', 0.0)",
                [],
            ).unwrap();
        }

        let lots = repo.get_all_tax_lots().unwrap();
        assert_eq!(lots.len(), 2);
        assert_eq!(lots[0].ticker, "AAPL");
        assert_eq!(lots[1].ticker, "MSFT");
        assert!((lots[0].shares - 100.0).abs() < f64::EPSILON);
        assert!((lots[0].cost_basis_per_share - 150.0).abs() < f64::EPSILON);
        assert_eq!(lots[0].acquisition_date, "2025-01-10");
        assert_eq!(lots[0].acquisition_type, "BUY");
    }

    #[test]
    fn get_tax_lots_for_ticker_filters_correctly() {
        let repo = repo();
        insert_tax_lot(&repo, "LOT-A", "AAPL", 50.0, 145.0, "2025-01-05", "BUY");
        insert_tax_lot(&repo, "LOT-B", "AAPL", 30.0, 155.0, "2025-03-01", "DCA");
        insert_tax_lot(&repo, "LOT-C", "MSFT", 80.0, 310.0, "2025-02-20", "BUY");

        let lots = repo.get_tax_lots_for_ticker("AAPL").unwrap();
        assert_eq!(lots.len(), 2);
        assert!(lots.iter().all(|l| l.ticker == "AAPL"));
        // Ordered by acquired_date
        assert_eq!(lots[0].lot_id, "LOT-A");
        assert_eq!(lots[1].lot_id, "LOT-B");
        assert_eq!(lots[1].acquisition_type, "DCA");
    }

    #[test]
    fn get_tax_lots_for_ticker_empty_when_none_match() {
        let repo = repo();
        insert_tax_lot(&repo, "LOT-X", "NVDA", 20.0, 800.0, "2025-06-01", "BUY");
        let lots = repo.get_tax_lots_for_ticker("AAPL").unwrap();
        assert!(lots.is_empty());
    }

    // ── Fundamental Metrics ──────────────────────────────────────────────

    fn insert_fundamental(
        repo: &SqliteRepository,
        ticker: &str,
        date: &str,
        pe: Option<f64>,
        score: Option<f64>,
    ) {
        let conn = repo.connection().unwrap();
        conn.execute(
            "INSERT INTO fundamental_metrics (ticker, fetched_date, pe_normalized, fundamental_score)
             VALUES (?1, ?2, ?3, ?4)",
            params![ticker, date, pe, score],
        ).unwrap();
    }

    #[test]
    fn get_all_fundamental_metrics_empty() {
        let repo = repo();
        let metrics = repo.get_all_fundamental_metrics().unwrap();
        assert!(metrics.is_empty());
    }

    #[test]
    fn get_all_fundamental_metrics_returns_rows() {
        let repo = repo();
        insert_fundamental(&repo, "AAPL", "2025-03-01", Some(28.5), Some(72.0));
        insert_fundamental(&repo, "MSFT", "2025-03-01", Some(35.0), Some(80.0));

        let metrics = repo.get_all_fundamental_metrics().unwrap();
        assert_eq!(metrics.len(), 2);
        let tickers: Vec<&str> = metrics.iter().map(|m| m.ticker.as_str()).collect();
        assert!(tickers.contains(&"AAPL"));
        assert!(tickers.contains(&"MSFT"));
    }

    #[test]
    fn get_fundamental_metrics_for_ticker_filters_and_orders() {
        let repo = repo();
        insert_fundamental(&repo, "AAPL", "2025-01-01", Some(25.0), Some(65.0));
        insert_fundamental(&repo, "AAPL", "2025-03-01", Some(28.5), Some(72.0));
        insert_fundamental(&repo, "MSFT", "2025-03-01", Some(35.0), Some(80.0));

        let metrics = repo.get_fundamental_metrics_for_ticker("AAPL").unwrap();
        assert_eq!(metrics.len(), 2);
        assert!(metrics.iter().all(|m| m.ticker == "AAPL"));
        // Ordered by fetched_date DESC
        assert_eq!(metrics[0].fetched_date, "2025-03-01");
        assert_eq!(metrics[1].fetched_date, "2025-01-01");
        assert!((metrics[0].pe_normalized.unwrap() - 28.5).abs() < f64::EPSILON);
    }

    #[test]
    fn get_fundamental_metrics_for_ticker_empty() {
        let repo = repo();
        let metrics = repo.get_fundamental_metrics_for_ticker("ZZZ").unwrap();
        assert!(metrics.is_empty());
    }

    #[test]
    fn fundamental_metrics_nullable_fields() {
        let repo = repo();
        // Insert with only required fields, rest NULL
        insert_fundamental(&repo, "TSLA", "2025-02-01", None, None);

        let metrics = repo.get_fundamental_metrics_for_ticker("TSLA").unwrap();
        assert_eq!(metrics.len(), 1);
        assert!(metrics[0].pe_normalized.is_none());
        assert!(metrics[0].pb_annual.is_none());
        assert!(metrics[0].fundamental_score.is_none());
    }

    // ── Earnings History ─────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn insert_earnings(
        repo: &SqliteRepository,
        ticker: &str,
        period: &str,
        fy: i32,
        fq: i32,
        actual: Option<f64>,
        estimate: Option<f64>,
        surprise_pct: Option<f64>,
    ) {
        let conn = repo.connection().unwrap();
        conn.execute(
            "INSERT INTO earnings_history (ticker, period_date, fiscal_year, fiscal_quarter, eps_actual, eps_estimate, eps_surprise_pct, is_upcoming)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
            params![ticker, period, fy, fq, actual, estimate, surprise_pct],
        ).unwrap();
    }

    #[test]
    fn get_earnings_history_empty() {
        let repo = repo();
        let rows = repo.get_earnings_history("AAPL", 10).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn get_earnings_history_returns_rows_ordered_desc() {
        let repo = repo();
        insert_earnings(
            &repo,
            "AAPL",
            "2024-12-31",
            2024,
            4,
            Some(2.10),
            Some(2.05),
            Some(2.4),
        );
        insert_earnings(
            &repo,
            "AAPL",
            "2024-09-30",
            2024,
            3,
            Some(1.95),
            Some(1.90),
            Some(2.6),
        );
        insert_earnings(
            &repo,
            "MSFT",
            "2024-12-31",
            2024,
            4,
            Some(3.00),
            Some(2.90),
            Some(3.4),
        );

        let rows = repo.get_earnings_history("AAPL", 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.ticker == "AAPL"));
        // Ordered by period_date DESC
        assert_eq!(rows[0].period_date, "2024-12-31");
        assert_eq!(rows[1].period_date, "2024-09-30");
        assert_eq!(rows[0].fiscal_quarter, 4);
        assert!((rows[0].eps_actual.unwrap() - 2.10).abs() < f64::EPSILON);
        assert!((rows[0].eps_surprise_pct.unwrap() - 2.4).abs() < f64::EPSILON);
        assert!(!rows[0].is_upcoming);
    }

    #[test]
    fn get_earnings_history_respects_limit() {
        let repo = repo();
        for q in 1..=4 {
            let period = format!("2024-{:02}-30", q * 3);
            insert_earnings(&repo, "NVDA", &period, 2024, q, Some(1.0), None, None);
        }
        let rows = repo.get_earnings_history("NVDA", 2).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn get_earnings_history_upcoming_flag() {
        let repo = repo();
        {
            let conn = repo.connection().unwrap();
            conn.execute(
                "INSERT INTO earnings_history (ticker, period_date, fiscal_year, fiscal_quarter, is_upcoming, earnings_date)
                 VALUES ('AAPL', '2025-03-31', 2025, 1, 1, '2025-04-25')",
                [],
            ).unwrap();
        }

        let rows = repo.get_earnings_history("AAPL", 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_upcoming);
        assert_eq!(rows[0].earnings_date.as_deref(), Some("2025-04-25"));
    }

    // ── Sector PE Benchmarks ─────────────────────────────────────────────

    fn insert_sector_pe(
        repo: &SqliteRepository,
        sector: &str,
        date: &str,
        pe: Option<f64>,
        ps: Option<f64>,
        count: i32,
    ) {
        let conn = repo.connection().unwrap();
        conn.execute(
            "INSERT INTO sector_pe_benchmarks (sector, snapshot_date, median_pe, median_ps, stock_count)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![sector, date, pe, ps, count],
        ).unwrap();
    }

    #[test]
    fn get_sector_pe_benchmarks_empty() {
        let repo = repo();
        let rows = repo.get_sector_pe_benchmarks().unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn get_sector_pe_benchmarks_returns_rows() {
        let repo = repo();
        insert_sector_pe(
            &repo,
            "Technology",
            "2025-03-01",
            Some(30.0),
            Some(8.0),
            150,
        );
        insert_sector_pe(
            &repo,
            "Healthcare",
            "2025-03-01",
            Some(22.0),
            Some(5.5),
            120,
        );
        insert_sector_pe(
            &repo,
            "Technology",
            "2025-02-01",
            Some(28.0),
            Some(7.5),
            148,
        );

        let rows = repo.get_sector_pe_benchmarks().unwrap();
        assert_eq!(rows.len(), 3);
        // Ordered by snapshot_date DESC, then sector
        assert_eq!(rows[0].snapshot_date, "2025-03-01");
        assert_eq!(rows[0].sector, "Healthcare");
        assert_eq!(rows[1].snapshot_date, "2025-03-01");
        assert_eq!(rows[1].sector, "Technology");
        assert!((rows[1].median_pe.unwrap() - 30.0).abs() < f64::EPSILON);
        assert_eq!(rows[1].stock_count, 150);
    }

    #[test]
    fn get_sector_pe_benchmarks_nullable_pe_ps() {
        let repo = repo();
        insert_sector_pe(&repo, "Energy", "2025-03-01", None, None, 50);

        let rows = repo.get_sector_pe_benchmarks().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].median_pe.is_none());
        assert!(rows[0].median_ps.is_none());
        assert_eq!(rows[0].stock_count, 50);
    }

    // ── Strategy Beliefs ────────────────────────────────────────────────

    #[test]
    fn save_and_load_strategy_beliefs() {
        let repo = repo();
        repo.save_strategy_belief("minervini", "Bull", 10.0, 5.0)
            .unwrap();
        repo.save_strategy_belief("breakout", "Bull", 8.0, 7.0)
            .unwrap();

        let beliefs = repo.load_strategy_beliefs("Bull").unwrap();
        assert_eq!(beliefs.len(), 2);
        let min = beliefs.iter().find(|b| b.0 == "minervini").unwrap();
        assert!((min.1 - 10.0).abs() < 0.01); // alpha
        assert!((min.2 - 5.0).abs() < 0.01); // beta
    }

    #[test]
    fn save_upserts_existing_belief() {
        let repo = repo();
        repo.save_strategy_belief("minervini", "Bull", 10.0, 5.0)
            .unwrap();
        repo.save_strategy_belief("minervini", "Bull", 15.0, 8.0)
            .unwrap();

        let beliefs = repo.load_strategy_beliefs("Bull").unwrap();
        assert_eq!(beliefs.len(), 1);
        assert!((beliefs[0].1 - 15.0).abs() < 0.01); // updated alpha
    }

    #[test]
    fn load_beliefs_filters_by_regime() {
        let repo = repo();
        repo.save_strategy_belief("minervini", "Bull", 10.0, 5.0)
            .unwrap();
        repo.save_strategy_belief("minervini", "Bear", 3.0, 12.0)
            .unwrap();

        let bull = repo.load_strategy_beliefs("Bull").unwrap();
        assert_eq!(bull.len(), 1);
        let bear = repo.load_strategy_beliefs("Bear").unwrap();
        assert_eq!(bear.len(), 1);
        assert!((bear[0].1 - 3.0).abs() < 0.01);
    }

    #[test]
    fn clear_beliefs_removes_regime() {
        let repo = repo();
        repo.save_strategy_belief("minervini", "Bull", 10.0, 5.0)
            .unwrap();
        repo.save_strategy_belief("breakout", "Bull", 8.0, 7.0)
            .unwrap();

        repo.clear_strategy_beliefs("Bull").unwrap();
        let beliefs = repo.load_strategy_beliefs("Bull").unwrap();
        assert!(beliefs.is_empty());
    }

    #[test]
    fn load_empty_returns_empty_vec() {
        let repo = repo();
        let beliefs = repo.load_strategy_beliefs("Sideways").unwrap();
        assert!(beliefs.is_empty());
    }
}
