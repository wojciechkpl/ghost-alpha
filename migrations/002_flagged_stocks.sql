-- Ghost Alpha — flagged stocks for exit alert tracking
-- Tracks stocks that passed screening so we can detect breakdowns later.

-- ── Flagged stocks watchlist ────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS flagged_stocks (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker          TEXT NOT NULL,
    flagged_date    TEXT NOT NULL,   -- YYYY-MM-DD when stock first passed screen
    entry_score     REAL NOT NULL,   -- composite_score at time of flagging
    entry_price     REAL NOT NULL,   -- close price at time of flagging
    conditions_at_entry INTEGER NOT NULL, -- Minervini conditions met at entry
    status          TEXT NOT NULL DEFAULT 'ACTIVE' CHECK(status IN ('ACTIVE', 'EXITED', 'EXPIRED')),
    exit_date       TEXT,            -- YYYY-MM-DD when exit signal fired
    exit_reason     TEXT,            -- e.g. "Stage2Breakdown", "RSDrop", "MADeathCross"
    exit_price      REAL,            -- close price at exit
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_flagged_stocks_status ON flagged_stocks(status);
CREATE UNIQUE INDEX IF NOT EXISTS idx_flagged_stocks_ticker_active
    ON flagged_stocks(ticker) WHERE status = 'ACTIVE';

-- Record migration version
INSERT OR IGNORE INTO schema_version (version) VALUES (2);
