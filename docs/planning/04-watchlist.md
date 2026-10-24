# Plan 04: Watchlist-First Workflow

**Effort:** Small (2 days)
**Impact:** High
**Priority:** 4

## Problem

The `universe` config field exists but is buried in `config.toml`. Most retail investors don't want to screen the full S&P 500 — they want to monitor their own portfolio or watchlist of 10-30 stocks. Currently there's no ergonomic way to manage this.

## Solution

Add a `watch` subcommand family for managing a persistent watchlist stored in SQLite. Screening the watchlist should be the default daily workflow for most users.

## User flow

```bash
# Add tickers to watchlist
ghost-alpha watch add NVDA AAPL TSLA META AMZN

# View current watchlist
ghost-alpha watch list

#   Ticker  Added       Last Score  Last Grade
#   NVDA    2026-03-01  94.2        A+
#   AAPL    2026-03-01  71.2        B+
#   TSLA    2026-03-01  45.8        C
#   META    2026-03-01  82.5        A-
#   AMZN    2026-03-01  67.3        B

# Remove a ticker
ghost-alpha watch remove TSLA

# Screen only watchlist tickers (fast — 5 API calls instead of 500)
ghost-alpha watch screen

# Screen watchlist with alerts enabled
ghost-alpha watch screen --alerts
```

## Implementation

### 1. Database migration (008)

Create `migrations/008_watchlist.sql`:

```sql
CREATE TABLE IF NOT EXISTS watchlist (
    ticker      TEXT PRIMARY KEY,
    added_at    TEXT NOT NULL DEFAULT (datetime('now')),
    notes       TEXT,
    last_score  REAL,
    last_grade  TEXT,
    last_screen TEXT  -- ISO datetime of last screen run
);
```

### 2. Repository methods

Add to `src/storage/db.rs`:

```rust
impl SqliteRepository {
    pub fn watchlist_add(&self, ticker: &str, notes: Option<&str>) -> Result<()>;
    pub fn watchlist_remove(&self, ticker: &str) -> Result<bool>;
    pub fn watchlist_list(&self) -> Result<Vec<WatchlistEntry>>;
    pub fn watchlist_update_score(&self, ticker: &str, score: f64, grade: &str) -> Result<()>;
    pub fn watchlist_tickers(&self) -> Result<Vec<String>>;
}
```

### 3. CLI subcommand

Add to `src/main.rs`:

```rust
/// Manage personal watchlist
Watch {
    #[command(subcommand)]
    action: WatchAction,
},

#[derive(Subcommand)]
enum WatchAction {
    /// Add tickers to watchlist
    Add {
        /// Ticker symbols (e.g., NVDA AAPL TSLA)
        tickers: Vec<String>,
        /// Optional note
        #[arg(long)]
        note: Option<String>,
    },
    /// Remove tickers from watchlist
    Remove {
        tickers: Vec<String>,
    },
    /// Show current watchlist with last known scores
    List {
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },
    /// Screen only watchlist tickers
    Screen {
        #[arg(long)]
        alerts: bool,
        #[arg(long)]
        skip_sentiment: bool,
        #[arg(long, default_value = "table")]
        format: OutputFormat,
    },
    /// Clear entire watchlist
    Clear,
}
```

### 4. Watch screen integration

`watch screen` reuses the existing screening pipeline but overrides the universe:

```rust
WatchAction::Screen { alerts, .. } => {
    let tickers = repo.watchlist_tickers()?;
    if tickers.is_empty() {
        println!("Watchlist is empty. Add tickers with: ghost-alpha watch add NVDA AAPL");
        return Ok(());
    }
    // Override config universe
    config.universe = tickers;
    // Run normal screening pipeline...
    run_screen(&config, alerts).await?;
    // Update last_score for each result
    for result in &results {
        repo.watchlist_update_score(&result.ticker, result.score, &result.grade)?;
    }
}
```

### 5. Import from config

For users who already have `universe = [...]` in config:

```bash
ghost-alpha watch import-config  # Imports universe tickers into watchlist
```

## Files to create/modify

| File | Action |
|------|--------|
| `migrations/008_watchlist.sql` | Create |
| `src/storage/db.rs` | Modify — add watchlist methods |
| `src/data/models.rs` | Modify — add `WatchlistEntry` struct |
| `src/main.rs` | Modify — add `Watch` subcommand |

## Testing

- Unit test: `watchlist_add` / `watchlist_remove` / `watchlist_list` on `:memory:` DB
- Unit test: `watchlist_tickers` returns correct set after add/remove
- Integration test: `watch screen` produces results for added tickers
- Edge case: Adding duplicate ticker is a no-op (UPSERT)
- Edge case: Removing non-existent ticker returns false, no error

## Acceptance criteria

- [ ] `ghost-alpha watch add/remove/list/screen/clear` all work
- [ ] Watchlist persists across runs (SQLite)
- [ ] `watch screen` uses only watchlist tickers (not full S&P 500)
- [ ] Last score/grade updates after each `watch screen`
- [ ] `watch list` shows formatted table with scores
- [ ] Duplicate adds are idempotent
