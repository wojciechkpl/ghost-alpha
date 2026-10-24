CREATE TABLE IF NOT EXISTS decision_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    decision_date   TEXT NOT NULL,
    decision_type   TEXT NOT NULL,
    ticker          TEXT,
    shares          REAL,
    price           REAL,
    rationale       TEXT NOT NULL,
    advisor         TEXT,
    linked_alert_id INTEGER,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_decision_log_date ON decision_log(decision_date);
CREATE INDEX IF NOT EXISTS idx_decision_log_ticker ON decision_log(ticker);
