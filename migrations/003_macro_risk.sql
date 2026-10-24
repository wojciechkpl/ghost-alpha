-- Migration V3: macro risk indicators from FRED API
CREATE TABLE IF NOT EXISTS macro_risk (
    date TEXT PRIMARY KEY,
    vix_level REAL NOT NULL,
    hy_oas_bps REAL,
    yield_curve_2s10s REAL,
    put_call_ratio REAL,
    risk_level TEXT NOT NULL CHECK(risk_level IN ('Normal','Elevated','HighRisk')),
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
