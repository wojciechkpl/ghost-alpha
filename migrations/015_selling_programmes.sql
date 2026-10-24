CREATE TABLE IF NOT EXISTS selling_programmes (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker          TEXT NOT NULL,
    schedule_json   TEXT NOT NULL,
    pace_rules_json TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS selling_executions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    programme_id    INTEGER NOT NULL REFERENCES selling_programmes(id),
    execution_date  TEXT NOT NULL,
    shares_sold     REAL NOT NULL,
    price           REAL NOT NULL,
    proceeds        REAL NOT NULL,
    lot_ids_json    TEXT,
    notes           TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_sell_exec_prog ON selling_executions(programme_id);
CREATE INDEX IF NOT EXISTS idx_sell_prog_ticker ON selling_programmes(ticker);
