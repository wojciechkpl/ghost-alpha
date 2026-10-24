CREATE TABLE IF NOT EXISTS earnings_history (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker           TEXT NOT NULL,
    period_date      TEXT NOT NULL,
    fiscal_year      INTEGER NOT NULL,
    fiscal_quarter   INTEGER NOT NULL CHECK(fiscal_quarter IN (1, 2, 3, 4)),
    eps_actual       REAL,
    eps_estimate     REAL,
    eps_surprise     REAL,
    eps_surprise_pct REAL,
    is_upcoming      INTEGER NOT NULL DEFAULT 0 CHECK(is_upcoming IN (0, 1)),
    earnings_date    TEXT,
    fetched_at       TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (ticker, period_date)
);

CREATE INDEX IF NOT EXISTS idx_earnings_ticker_period
    ON earnings_history(ticker, period_date DESC);
CREATE INDEX IF NOT EXISTS idx_earnings_upcoming
    ON earnings_history(earnings_date, is_upcoming)
    WHERE is_upcoming = 1;

INSERT OR IGNORE INTO schema_version (version) VALUES (10);
