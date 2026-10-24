CREATE TABLE IF NOT EXISTS watchlist (
    ticker      TEXT PRIMARY KEY,
    added_at    TEXT NOT NULL DEFAULT (datetime('now')),
    notes       TEXT,
    last_score  REAL,
    last_grade  TEXT,
    last_screen TEXT  -- ISO datetime of last screen run
);
CREATE INDEX IF NOT EXISTS idx_watchlist_added_at
    ON watchlist(added_at DESC);
