use chrono::{DateTime, Duration, Utc};
use std::collections::HashMap;

/// Key uniquely identifying an alert type for deduplication.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DedupKey {
    pub ticker: String,
    pub alert_type: String,
}

impl DedupKey {
    pub fn new(ticker: &str, alert_type: &str) -> Self {
        Self {
            ticker: ticker.to_string(),
            alert_type: alert_type.to_string(),
        }
    }
}

/// Deduplication store trait — in-memory for tests, SQLite-backed in production.
pub trait DedupStore: Send + Sync {
    /// Returns true if an alert with this key was sent within `cooldown_days`.
    fn is_duplicate(&self, key: &DedupKey, cooldown_days: i64) -> bool;
    /// Record that an alert with this key was sent now.
    fn record_sent(&mut self, key: &DedupKey);
    /// Number of entries in the store.
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// In-memory deduplication store (used in unit tests and as the Phase-1 implementation).
pub struct InMemoryDedupStore {
    records: HashMap<DedupKey, DateTime<Utc>>,
    /// Injectable clock for deterministic testing
    clock: Box<dyn Fn() -> DateTime<Utc> + Send + Sync>,
}

impl InMemoryDedupStore {
    pub fn new() -> Self {
        Self {
            records: HashMap::new(),
            clock: Box::new(Utc::now),
        }
    }

    /// Create a store with a fixed clock (for tests).
    #[cfg(test)]
    pub fn with_clock(clock: impl Fn() -> DateTime<Utc> + Send + Sync + 'static) -> Self {
        Self {
            records: HashMap::new(),
            clock: Box::new(clock),
        }
    }

    fn now(&self) -> DateTime<Utc> {
        (self.clock)()
    }
}

impl Default for InMemoryDedupStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DedupStore for InMemoryDedupStore {
    fn is_duplicate(&self, key: &DedupKey, cooldown_days: i64) -> bool {
        match self.records.get(key) {
            Some(&last_sent) => {
                let age = self.now() - last_sent;
                age < Duration::days(cooldown_days)
            }
            None => false,
        }
    }

    fn record_sent(&mut self, key: &DedupKey) {
        self.records.insert(key.clone(), self.now());
    }

    fn len(&self) -> usize {
        self.records.len()
    }
}

// ─── SQLite-backed dedup store ───────────────────────────────────────────────

use std::sync::Mutex;

use crate::storage::db::{Repository, SqliteRepository};

/// Persistent dedup store backed by the `alert_log` table in SQLite.
/// Survives process restarts — essential for autonomous daily operation.
pub struct SqliteDedupStore {
    repo: Mutex<SqliteRepository>,
}

impl SqliteDedupStore {
    pub fn new(repo: SqliteRepository) -> Self {
        Self {
            repo: Mutex::new(repo),
        }
    }
}

impl DedupStore for SqliteDedupStore {
    fn is_duplicate(&self, key: &DedupKey, cooldown_days: i64) -> bool {
        let repo = self.repo.lock().unwrap();
        match repo.alert_last_sent(&key.ticker, &key.alert_type) {
            Ok(Some(last_sent)) => {
                let age = Utc::now() - last_sent;
                age < Duration::days(cooldown_days)
            }
            _ => false,
        }
    }

    fn record_sent(&mut self, key: &DedupKey) {
        use crate::alerts::channel::OutboundAlert;
        use crate::data::models::Severity;

        let repo = self.repo.lock().unwrap();
        let alert = OutboundAlert {
            ticker: key.ticker.clone(),
            alert_type: key.alert_type.clone(),
            severity: Severity::Info, // Severity is unknown at dedup layer; record as Info
            message: String::new(),
            composite_score: 0.0,
            narrative: None,
        };
        if let Err(e) = repo.record_alert(&alert) {
            tracing::error!(
                ticker = %key.ticker,
                alert_type = %key.alert_type,
                error = %e,
                "Failed to persist dedup record to SQLite"
            );
        }
    }

    fn len(&self) -> usize {
        let repo = self.repo.lock().unwrap_or_else(|e| e.into_inner());
        // Count distinct (ticker, alert_type) pairs in alert_log
        let conn = match repo.connection() {
            Ok(c) => c,
            Err(_) => return 0,
        };
        conn.query_row("SELECT COUNT(*) FROM alert_log", [], |row| row.get(0))
            .unwrap_or(0)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_clock(dt: DateTime<Utc>) -> impl Fn() -> DateTime<Utc> + Send + Sync {
        move || dt
    }

    // ── InMemoryDedupStore tests ────────────────────────────────────────────

    #[test]
    fn new_alert_is_not_duplicate() {
        let store = InMemoryDedupStore::new();
        let key = DedupKey::new("AAPL", "NewSignal");
        assert!(!store.is_duplicate(&key, 7));
    }

    #[test]
    fn alert_sent_today_is_duplicate_within_7_days() {
        let now = Utc::now();
        let mut store = InMemoryDedupStore::with_clock(fixed_clock(now));
        let key = DedupKey::new("AAPL", "NewSignal");
        store.record_sent(&key);
        assert!(store.is_duplicate(&key, 7));
    }

    #[test]
    fn alert_sent_8_days_ago_not_duplicate_with_7_day_cooldown() {
        // t0: 8 days ago; check at "now"
        let t0 = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
        let t_now = t0 + Duration::days(8);

        // Record at t0
        let mut store = InMemoryDedupStore::with_clock(fixed_clock(t0));
        let key = DedupKey::new("AAPL", "NewSignal");
        store.record_sent(&key);

        // Check at t_now
        let store_now = InMemoryDedupStore {
            records: store.records,
            clock: Box::new(fixed_clock(t_now)),
        };
        assert!(
            !store_now.is_duplicate(&key, 7),
            "8-day old alert should NOT be duplicate with 7-day cooldown"
        );
    }

    #[test]
    fn alert_sent_6_days_ago_is_duplicate_with_7_day_cooldown() {
        let t0 = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
        let t_now = t0 + Duration::days(6);

        let mut store = InMemoryDedupStore::with_clock(fixed_clock(t0));
        let key = DedupKey::new("NVDA", "BreakoutVolume");
        store.record_sent(&key);

        let store_now = InMemoryDedupStore {
            records: store.records,
            clock: Box::new(fixed_clock(t_now)),
        };
        assert!(
            store_now.is_duplicate(&key, 7),
            "6-day old alert should be duplicate with 7-day cooldown"
        );
    }

    #[test]
    fn different_tickers_not_deduplicated() {
        let mut store = InMemoryDedupStore::new();
        let key_aapl = DedupKey::new("AAPL", "NewSignal");
        let key_nvda = DedupKey::new("NVDA", "NewSignal");
        store.record_sent(&key_aapl);
        assert!(!store.is_duplicate(&key_nvda, 7));
    }

    #[test]
    fn different_alert_types_not_deduplicated() {
        let mut store = InMemoryDedupStore::new();
        let key_entry = DedupKey::new("AAPL", "NewSignal");
        let key_exit = DedupKey::new("AAPL", "ExitSignal");
        store.record_sent(&key_entry);
        assert!(!store.is_duplicate(&key_exit, 7));
    }

    #[test]
    fn len_increments_per_unique_key() {
        let mut store = InMemoryDedupStore::new();
        assert_eq!(store.len(), 0);
        store.record_sent(&DedupKey::new("AAPL", "NewSignal"));
        assert_eq!(store.len(), 1);
        store.record_sent(&DedupKey::new("NVDA", "NewSignal"));
        assert_eq!(store.len(), 2);
        // Re-recording same key updates timestamp but doesn't add a new entry
        store.record_sent(&DedupKey::new("AAPL", "NewSignal"));
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn zero_cooldown_always_not_duplicate() {
        let mut store = InMemoryDedupStore::new();
        let key = DedupKey::new("AAPL", "NewSignal");
        store.record_sent(&key);
        // cooldown = 0 days → any age is >= 0 → never duplicate
        assert!(!store.is_duplicate(&key, 0));
    }

    // ── SqliteDedupStore tests ──────────────────────────────────────────────

    #[test]
    fn sqlite_dedup_new_alert_not_duplicate() {
        let repo = SqliteRepository::open_in_memory().unwrap();
        let store = SqliteDedupStore::new(repo);
        let key = DedupKey::new("AAPL", "NewSignal");
        assert!(!store.is_duplicate(&key, 7));
    }

    #[test]
    fn sqlite_dedup_recorded_alert_is_duplicate() {
        let repo = SqliteRepository::open_in_memory().unwrap();
        let mut store = SqliteDedupStore::new(repo);
        let key = DedupKey::new("AAPL", "NewSignal");
        store.record_sent(&key);
        assert!(
            store.is_duplicate(&key, 7),
            "Just-recorded alert should be duplicate within 7-day cooldown"
        );
    }

    #[test]
    fn sqlite_dedup_different_tickers_independent() {
        let repo = SqliteRepository::open_in_memory().unwrap();
        let mut store = SqliteDedupStore::new(repo);
        store.record_sent(&DedupKey::new("AAPL", "NewSignal"));
        assert!(!store.is_duplicate(&DedupKey::new("NVDA", "NewSignal"), 7));
    }

    #[test]
    fn sqlite_dedup_different_alert_types_independent() {
        let repo = SqliteRepository::open_in_memory().unwrap();
        let mut store = SqliteDedupStore::new(repo);
        store.record_sent(&DedupKey::new("AAPL", "NewSignal"));
        assert!(!store.is_duplicate(&DedupKey::new("AAPL", "ExitSignal"), 7));
    }

    #[test]
    fn sqlite_dedup_len_counts_entries() {
        let repo = SqliteRepository::open_in_memory().unwrap();
        let mut store = SqliteDedupStore::new(repo);
        assert_eq!(store.len(), 0);
        store.record_sent(&DedupKey::new("AAPL", "NewSignal"));
        assert_eq!(store.len(), 1);
        store.record_sent(&DedupKey::new("NVDA", "NewSignal"));
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn sqlite_dedup_zero_cooldown_never_duplicate() {
        let repo = SqliteRepository::open_in_memory().unwrap();
        let mut store = SqliteDedupStore::new(repo);
        let key = DedupKey::new("AAPL", "NewSignal");
        store.record_sent(&key);
        assert!(!store.is_duplicate(&key, 0));
    }
}
