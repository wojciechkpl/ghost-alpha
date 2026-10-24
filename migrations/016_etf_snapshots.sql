CREATE TABLE IF NOT EXISTS etf_snapshots (
    ticker           TEXT PRIMARY KEY,
    name             TEXT NOT NULL,
    expense_ratio    REAL,
    aum              REAL,
    avg_daily_volume INTEGER,
    category         TEXT,
    inception_date   TEXT,
    tracking_index   TEXT,
    holdings_json    TEXT,
    fetched_at       TEXT NOT NULL DEFAULT (datetime('now'))
);
