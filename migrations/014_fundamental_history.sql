CREATE TABLE IF NOT EXISTS fundamental_history (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker          TEXT NOT NULL,
    period_date     TEXT NOT NULL,
    period_type     TEXT NOT NULL,
    pe_ratio        REAL,
    pb_ratio        REAL,
    ps_ratio        REAL,
    ev_ebitda       REAL,
    ev_revenue      REAL,
    gross_margin    REAL,
    operating_margin REAL,
    net_margin      REAL,
    roe             REAL,
    debt_to_equity  REAL,
    revenue         REAL,
    ebitda          REAL,
    fcf             REAL,
    fetched_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_fund_hist_ticker_period
    ON fundamental_history(ticker, period_date, period_type);

CREATE INDEX IF NOT EXISTS idx_fund_hist_ticker ON fundamental_history(ticker);
