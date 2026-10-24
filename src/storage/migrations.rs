use rusqlite::Connection;

use crate::error::StorageError;

/// SQL for each migration version (1-indexed; version N is at index N-1).
const MIGRATIONS: &[&str] = &[
    include_str!("../../migrations/001_initial_schema.sql"),
    include_str!("../../migrations/002_flagged_stocks.sql"),
    include_str!("../../migrations/003_macro_risk.sql"),
    include_str!("../../migrations/004_conviction_decay.sql"),
    include_str!("../../migrations/005_screen_result_v2.sql"),
    include_str!("../../migrations/006_backtest_results.sql"),
    include_str!("../../migrations/007_news_cache.sql"),
    include_str!("../../migrations/008_watchlist.sql"),
    include_str!("../../migrations/009_fundamental_metrics.sql"),
    include_str!("../../migrations/010_earnings_history.sql"),
    include_str!("../../migrations/011_sector_pe_benchmarks.sql"),
    include_str!("../../migrations/012_portfolio.sql"),
    include_str!("../../migrations/013_correlation_snapshots.sql"),
    include_str!("../../migrations/014_fundamental_history.sql"),
    include_str!("../../migrations/015_selling_programmes.sql"),
    include_str!("../../migrations/016_etf_snapshots.sql"),
    include_str!("../../migrations/017_decision_log.sql"),
    include_str!("../../migrations/018_strategy_beliefs.sql"),
];

/// Apply all pending migrations to the connection.
/// Idempotent: skips versions already recorded in `schema_version`.
pub fn run_migrations(conn: &Connection) -> Result<(), StorageError> {
    // Bootstrap the version table if it doesn't exist yet.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version    INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .map_err(|e| StorageError::Migration {
        version: 0,
        reason: e.to_string(),
    })?;

    let current_version: u32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let version = (i + 1) as u32;
        if version <= current_version {
            continue;
        }
        conn.execute_batch(sql)
            .map_err(|e| StorageError::Migration {
                version,
                reason: e.to_string(),
            })?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_version (version) VALUES (?1)",
            [version],
        )
        .map_err(|e| StorageError::Migration {
            version,
            reason: e.to_string(),
        })?;
    }

    Ok(())
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn open_memory() -> Connection {
        Connection::open_in_memory().expect("in-memory DB")
    }

    #[test]
    fn migrations_run_on_fresh_db() {
        let conn = open_memory();
        run_migrations(&conn).expect("migrations should succeed on fresh DB");
    }

    #[test]
    fn migrations_are_idempotent() {
        let conn = open_memory();
        run_migrations(&conn).expect("first run");
        run_migrations(&conn).expect("second run should be idempotent");
    }

    #[test]
    fn schema_version_is_recorded() {
        let conn = open_memory();
        run_migrations(&conn).expect("run migrations");
        let version: u32 = conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |r| r.get(0))
            .expect("query version");
        assert_eq!(
            version, 18,
            "Schema should be at version 18 after migrations"
        );
    }

    #[test]
    fn expected_tables_exist_after_migration() {
        let conn = open_memory();
        run_migrations(&conn).expect("run migrations");
        for table in &[
            "stocks",
            "candles",
            "screen_results",
            "alert_log",
            "market_regime",
            "flagged_stocks",
            "macro_risk",
            "backtest_runs",
            "news_cache",
            "watchlist",
            "fundamental_metrics",
            "earnings_history",
            "sector_pe_benchmarks",
            "tax_lots",
            "disposals",
            "portfolio_snapshots",
            "cash_transactions",
            "correlation_snapshots",
            "fundamental_history",
            "selling_programmes",
            "selling_executions",
            "etf_snapshots",
            "decision_log",
            "strategy_beliefs",
        ] {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |r| r.get(0),
                )
                .expect("query table existence");
            assert_eq!(count, 1, "Table '{}' should exist after migration", table);
        }
    }
}
