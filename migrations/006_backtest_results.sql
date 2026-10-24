-- Migration V6: Backtest results storage
CREATE TABLE IF NOT EXISTS backtest_runs (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    run_date      TEXT NOT NULL DEFAULT (datetime('now')),
    params_json   TEXT NOT NULL,
    kpis_json     TEXT NOT NULL,
    report_json   TEXT NOT NULL,
    signal_count  INTEGER NOT NULL,
    duration_secs REAL NOT NULL
);
