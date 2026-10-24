-- Tax lots: the atomic unit of cost basis tracking
CREATE TABLE IF NOT EXISTS tax_lots (
    id                TEXT PRIMARY KEY,  -- UUID
    ticker            TEXT NOT NULL,
    shares            REAL NOT NULL,
    cost_basis_per_share REAL NOT NULL,
    acquired_date     TEXT NOT NULL,     -- YYYY-MM-DD
    acquisition_type  TEXT NOT NULL DEFAULT 'BUY',
    remaining_shares  REAL NOT NULL,
    created_at        TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at        TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_tax_lots_ticker ON tax_lots(ticker);

-- Disposal / sale records (audit trail)
CREATE TABLE IF NOT EXISTS disposals (
    id                TEXT PRIMARY KEY,  -- UUID
    lot_id            TEXT NOT NULL REFERENCES tax_lots(id),
    ticker            TEXT NOT NULL,
    shares_sold       REAL NOT NULL,
    sale_price        REAL NOT NULL,
    sale_date         TEXT NOT NULL,
    proceeds          REAL NOT NULL,
    cost_basis        REAL NOT NULL,
    realised_gain     REAL NOT NULL,
    is_long_term      INTEGER NOT NULL,  -- 0/1
    created_at        TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_disposals_ticker ON disposals(ticker);
CREATE INDEX IF NOT EXISTS idx_disposals_date ON disposals(sale_date);

-- Portfolio snapshots (daily weight captures for time-series analysis)
CREATE TABLE IF NOT EXISTS portfolio_snapshots (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    as_of_date        TEXT NOT NULL,
    total_value       REAL NOT NULL,
    cash              REAL NOT NULL DEFAULT 0.0,
    positions_json    TEXT NOT NULL,     -- JSON: [{ticker, shares, price, weight}]
    hhi               REAL NOT NULL,
    created_at        TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_portfolio_snap_date
    ON portfolio_snapshots(as_of_date);

-- Cash balance tracking
CREATE TABLE IF NOT EXISTS cash_transactions (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    transaction_date  TEXT NOT NULL,
    amount            REAL NOT NULL,     -- positive = inflow, negative = outflow
    source            TEXT NOT NULL,     -- 'SALE_PROCEEDS', 'DEPOSIT', 'WITHDRAWAL', 'DCA'
    ticker            TEXT,              -- nullable: linked ticker if from sale or DCA
    notes             TEXT,
    created_at        TEXT NOT NULL DEFAULT (datetime('now'))
);
