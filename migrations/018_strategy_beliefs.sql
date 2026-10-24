-- Strategy beliefs for Bayesian weight updating (Phase 6, F24)
CREATE TABLE IF NOT EXISTS strategy_beliefs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    strategy    TEXT NOT NULL,
    regime      TEXT NOT NULL,
    alpha       REAL NOT NULL DEFAULT 2.0,
    beta        REAL NOT NULL DEFAULT 2.0,
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(strategy, regime)
);
