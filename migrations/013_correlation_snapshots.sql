CREATE TABLE IF NOT EXISTS correlation_snapshots (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    computed_date   TEXT NOT NULL,
    tickers_json    TEXT NOT NULL,
    matrix_json     TEXT NOT NULL,
    window_days     INTEGER NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_corr_snap_date ON correlation_snapshots(computed_date);
