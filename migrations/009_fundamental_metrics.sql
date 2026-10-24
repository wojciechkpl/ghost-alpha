CREATE TABLE IF NOT EXISTS fundamental_metrics (
    ticker               TEXT NOT NULL,
    fetched_date         TEXT NOT NULL,
    pe_normalized        REAL,
    pb_annual            REAL,
    ps_annual            REAL,
    eps_growth_ttm_yoy   REAL,
    rev_growth_ttm_yoy   REAL,
    eps_growth_qoq       REAL,
    rev_growth_qoq       REAL,
    gross_margin_ttm     REAL,
    net_margin_ttm       REAL,
    roe_ttm              REAL,
    roi_ttm              REAL,
    debt_equity          REAL,
    current_ratio        REAL,
    fundamental_score    REAL,
    PRIMARY KEY (ticker, fetched_date)
);

CREATE INDEX IF NOT EXISTS idx_fundamental_metrics_ticker
    ON fundamental_metrics(ticker, fetched_date DESC);

INSERT OR IGNORE INTO schema_version (version) VALUES (9);
