-- Ghost Alpha — initial schema
-- Applies on first run; subsequent runs check schema_version.

PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

-- ── Schema version tracking ────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS schema_version (
    version     INTEGER PRIMARY KEY,
    applied_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ── Stock universe ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS stocks (
    ticker      TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    sector      TEXT NOT NULL DEFAULT '',
    industry    TEXT NOT NULL DEFAULT '',
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- ── Daily OHLCV candles ────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS candles (
    ticker      TEXT NOT NULL,
    date        TEXT NOT NULL,   -- ISO-8601: YYYY-MM-DD
    open        REAL NOT NULL,
    high        REAL NOT NULL,
    low         REAL NOT NULL,
    close       REAL NOT NULL,
    adj_close   REAL NOT NULL,
    volume      INTEGER NOT NULL,
    PRIMARY KEY (ticker, date),
    FOREIGN KEY (ticker) REFERENCES stocks(ticker) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_candles_ticker_date ON candles(ticker, date DESC);

-- ── Screen results ─────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS screen_results (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    run_date            TEXT NOT NULL,   -- YYYY-MM-DD of screening run
    ticker              TEXT NOT NULL,
    minervini_pass      INTEGER NOT NULL CHECK(minervini_pass IN (0, 1)),
    conditions_met      INTEGER NOT NULL,
    adx                 REAL NOT NULL,
    rs_percentile       REAL NOT NULL,
    volume_zscore       REAL NOT NULL,
    technical_score     REAL NOT NULL,
    volume_score        REAL NOT NULL,
    sentiment_score     REAL NOT NULL,
    composite_score     REAL NOT NULL,
    grade               TEXT NOT NULL,
    close_price         REAL NOT NULL,
    pct_from_52w_high   REAL NOT NULL,
    created_at          TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_screen_results_run_date ON screen_results(run_date DESC);
CREATE INDEX IF NOT EXISTS idx_screen_results_ticker   ON screen_results(ticker);

-- ── Alert deduplication log ────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS alert_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker      TEXT NOT NULL,
    alert_type  TEXT NOT NULL,
    severity    TEXT NOT NULL,
    message     TEXT NOT NULL,
    sent_at     TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_alert_log_ticker_type ON alert_log(ticker, alert_type);

-- ── Market regime log ──────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS market_regime (
    date        TEXT PRIMARY KEY,
    spy_close   REAL NOT NULL,
    spy_sma200  REAL NOT NULL,
    breadth_pct REAL NOT NULL,
    regime      TEXT NOT NULL CHECK(regime IN ('Bull', 'Sideways', 'Bear'))
);

-- Seed schema version
INSERT OR IGNORE INTO schema_version (version) VALUES (1);
