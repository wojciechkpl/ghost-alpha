CREATE TABLE IF NOT EXISTS sector_pe_benchmarks (
    sector           TEXT NOT NULL,
    snapshot_date    TEXT NOT NULL,
    median_pe        REAL,
    median_ps        REAL,
    stock_count      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (sector, snapshot_date)
);

CREATE INDEX IF NOT EXISTS idx_sector_pe_date
    ON sector_pe_benchmarks(snapshot_date DESC);

INSERT OR IGNORE INTO schema_version (version) VALUES (11);
