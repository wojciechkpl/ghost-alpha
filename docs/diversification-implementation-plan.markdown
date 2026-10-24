# GhostAlpha Diversification Advisory -- Implementation Plan

> **Scope:** 20 gaps (G1--G20) from the diversification gap analysis.
> **Convention anchor:** All new code follows existing patterns -- `thiserror`
> per-module errors, `async_trait` for async traits, `rusqlite` + `spawn_blocking`,
> inline `#[cfg(test)]` modules, feature-gated integration tests.
>
> **Migration baseline:** Existing migrations end at `011_sector_pe_benchmarks.sql`.
> New migrations start at **012**.

---

## Table of Contents

1. [New Dependencies](#new-dependencies)
2. [Module Map](#module-map)
3. [Shared Types & Error Additions](#shared-types--error-additions)
4. [G1 -- Portfolio & Position Management](#g1----portfolio--position-management)
5. [G2 -- Multi-Asset Data](#g2----multi-asset-data)
6. [G3 -- Correlation Matrix Engine](#g3----correlation-matrix-engine)
7. [G4 -- Tax Calculation Engine](#g4----tax-calculation-engine)
8. [G5 -- DCF / Valuation Module](#g5----dcf--valuation-module)
9. [G6 -- Fundamental Time-Series](#g6----fundamental-time-series)
10. [G7 -- Portfolio Optimisation (MVO / Risk Parity)](#g7----portfolio-optimisation-mvo--risk-parity)
11. [G8 -- ETF Screening & Holdings Data](#g8----etf-screening--holdings-data)
12. [G9 -- Selling Programme / Liquidation Scheduler](#g9----selling-programme--liquidation-scheduler)
13. [G10 -- Options Data & Pricing](#g10----options-data--pricing)
14. [G11 -- Segment-Level Financials](#g11----segment-level-financials)
15. [G12 -- Institutional Ownership / 13F](#g12----institutional-ownership--13f)
16. [G13 -- Factor Decomposition](#g13----factor-decomposition)
17. [G14 -- Forward Monte Carlo](#g14----forward-monte-carlo)
18. [G15 -- Performance Attribution (Brinson)](#g15----performance-attribution-brinson)
19. [G16 -- IPS Document Generator](#g16----ips-document-generator)
20. [G17 -- Decision Audit Log](#g17----decision-audit-log)
21. [G18 -- Rebalancing Alert Engine](#g18----rebalancing-alert-engine)
22. [G19 -- Direct Indexing Simulator](#g19----direct-indexing-simulator)
23. [G20 -- Quarterly Report Generator](#g20----quarterly-report-generator)

---

## New Dependencies

Add to `Cargo.toml` `[dependencies]`:

```toml
# Linear algebra for correlation matrices, MVO, factor regression
nalgebra    = "0.33"
# Quadratic programming for MVO efficient frontier
osqp        = "0.6"
# UUID for tax lot identifiers
uuid        = { version = "1", features = ["v4", "serde"] }
# Date math for tax lot holding periods
time        = "0.3"
```

Add to `[dev-dependencies]`:

```toml
proptest    = "1"
```

No new features are required -- all 20 gaps compile into the default binary.
`nalgebra` and `osqp` are the only heavyweight additions; both are pure Rust
(or have C bindings bundled) and add ~1 MB to the release binary.

---

## Module Map

New top-level modules registered in `src/lib.rs`:

```rust
pub mod portfolio;   // G1, G9
pub mod tax;         // G4
pub mod valuation;   // G5
pub mod optimize;    // G7, G13, G14, G19
```

Extended existing modules:

| Module | Files Added / Changed |
|---|---|
| `src/analysis/` | `correlation_matrix.rs` (G3), `factor.rs` (G13), `attribution.rs` (G15) |
| `src/data/` | `etf.rs` (G8), `options.rs` (G10), `segments.rs` (G11), `ownership.rs` (G12) |
| `src/storage/` | `migrations.rs` updated, `db.rs` gets new `Repository` methods |
| `src/web/` | `routes.rs` extended, new `portfolio_routes.rs`, `report_routes.rs` |
| `src/mcp/` | `tools.rs` extended |
| `src/backtest/` | `monte_carlo.rs` extended (G14) |
| `src/alerts/` | `rebalance.rs` (G18) |
| `src/config.rs` | New config sections |

Full file tree of new files:

```
src/
  portfolio/
    mod.rs              # pub mod types, import, snapshot
    types.rs            # Position, TaxLot, Portfolio, PortfolioSnapshot
    import.rs           # CSV/JSON import, brokerage format parsers
    snapshot.rs         # compute weights, HHI, concentration from holdings
  tax/
    mod.rs              # pub mod brackets, engine, hifo, harvest, projection
    brackets.rs         # FederalBrackets, StateBrackets, NiitThreshold
    engine.rs           # TaxEngine: classify lot, compute gain, total tax
    hifo.rs             # HIFO lot selection optimiser
    harvest.rs          # Tax-loss harvesting scanner
    projection.rs       # Multi-year bracket projection
  valuation/
    mod.rs              # pub mod dcf, reverse_dcf, sotp, scenario
    dcf.rs              # ThreeStageDcf, WaccCalculator
    reverse_dcf.rs      # ReverseDcfSolver (implied growth)
    sotp.rs             # SumOfTheParts calculator
    scenario.rs         # ScenarioEngine: bear/base/bull
  optimize/
    mod.rs              # pub mod mvo, risk_parity, frontier
    mvo.rs              # MeanVarianceOptimizer using OSQP
    risk_parity.rs      # RiskParityOptimizer (iterative)
    frontier.rs         # EfficientFrontier: sweep target returns
  analysis/
    correlation_matrix.rs  # NxN matrix, rolling window, crisis slicing
    factor.rs              # FamaFrench 5-factor regression
    attribution.rs         # Brinson performance attribution
  data/
    etf.rs                 # EtfSnapshot, EtfHolding, EtfProvider trait
    options.rs             # OptionChain, BlackScholes, CollarDesigner
    segments.rs            # SegmentFinancials, SegmentFetcher
    ownership.rs           # InstitutionalHolder, OwnershipTrend
  alerts/
    rebalance.rs           # RebalanceAlertRule, weight drift detection
  web/
    portfolio_routes.rs    # portfolio pages + htmx partials
    report_routes.rs       # quarterly report download
```

---

## Shared Types & Error Additions

### `src/error.rs` -- new error variants

```rust
#[derive(Debug, Error)]
pub enum GhostError {
    // ... existing variants ...
    #[error("Portfolio error: {0}")]
    Portfolio(#[from] PortfolioError),
    #[error("Tax error: {0}")]
    Tax(#[from] TaxError),
    #[error("Valuation error: {0}")]
    Valuation(#[from] ValuationError),
    #[error("Optimisation error: {0}")]
    Optimize(#[from] OptimizeError),
}

#[derive(Debug, Error, PartialEq)]
pub enum PortfolioError {
    #[error("No positions loaded")]
    Empty,
    #[error("Duplicate lot ID: {0}")]
    DuplicateLot(String),
    #[error("Import parse error at line {line}: {reason}")]
    ImportParse { line: usize, reason: String },
    #[error("Ticker not found in portfolio: {0}")]
    TickerNotFound(String),
}

#[derive(Debug, Error, PartialEq)]
pub enum TaxError {
    #[error("Unknown state: {0}")]
    UnknownState(String),
    #[error("Negative shares in lot {lot_id}")]
    NegativeShares { lot_id: String },
    #[error("Calculation overflow")]
    Overflow,
}

#[derive(Debug, Error, PartialEq)]
pub enum ValuationError {
    #[error("Invalid WACC: {0}")]
    InvalidWacc(String),
    #[error("No convergence in reverse DCF after {iterations} iterations")]
    NoConvergence { iterations: u32 },
    #[error("Missing input: {0}")]
    MissingInput(String),
}

#[derive(Debug, Error, PartialEq)]
pub enum OptimizeError {
    #[error("Solver failed: {0}")]
    SolverFailed(String),
    #[error("Singular covariance matrix")]
    SingularMatrix,
    #[error("Infeasible constraints: {0}")]
    InfeasibleConstraints(String),
}
```

### `src/config.rs` -- new config sections

```rust
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PortfolioConfig {
    /// Path to portfolio CSV import file (optional, can also import via CLI)
    #[serde(default)]
    pub import_path: Option<String>,
    /// Default state for tax calculations (e.g., "CA", "NY", "TX")
    #[serde(default = "PortfolioConfig::default_state")]
    pub tax_state: String,
    /// Target allocation weights as [ticker/asset_class, weight] pairs
    #[serde(default)]
    pub target_weights: Vec<TargetWeight>,
    /// Rebalancing drift threshold percentage (default: 5.0)
    #[serde(default = "PortfolioConfig::default_drift_threshold")]
    pub drift_threshold_pct: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TargetWeight {
    pub asset: String,
    pub weight: f64,
}

impl PortfolioConfig {
    fn default_state() -> String { "CA".to_string() }
    fn default_drift_threshold() -> f64 { 5.0 }
}
```

Add `portfolio: Option<PortfolioConfig>` to `AppConfig`.

---

## G1 -- Portfolio & Position Management

**Blocks:** G4, G7, G9, G14, G15, G18, G20

### 1.1 Types (`src/portfolio/types.rs`)

```rust
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single tax lot -- the atomic unit of cost basis tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaxLot {
    pub id: Uuid,
    pub ticker: String,
    pub shares: f64,
    pub cost_basis_per_share: f64,
    pub acquired_date: NaiveDate,
    /// "BUY", "RSU_VEST", "ESPP", "GIFT", "INHERITANCE"
    pub acquisition_type: AcquisitionType,
    /// Remaining shares after partial sales
    pub remaining_shares: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AcquisitionType {
    Buy,
    RsuVest,
    Espp,
    Gift,
    Inheritance,
}

impl TaxLot {
    pub fn total_cost_basis(&self) -> f64 {
        self.remaining_shares * self.cost_basis_per_share
    }

    pub fn holding_period_days(&self, as_of: NaiveDate) -> i64 {
        (as_of - self.acquired_date).num_days()
    }

    pub fn is_long_term(&self, as_of: NaiveDate) -> bool {
        self.holding_period_days(as_of) > 365
    }

    pub fn unrealised_gain(&self, current_price: f64) -> f64 {
        (current_price - self.cost_basis_per_share) * self.remaining_shares
    }
}

/// Aggregated position for a single ticker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub ticker: String,
    pub lots: Vec<TaxLot>,
    pub current_price: f64,
    pub last_updated: NaiveDate,
}

impl Position {
    pub fn total_shares(&self) -> f64 {
        self.lots.iter().map(|l| l.remaining_shares).sum()
    }

    pub fn total_cost_basis(&self) -> f64 {
        self.lots.iter().map(|l| l.total_cost_basis()).sum()
    }

    pub fn market_value(&self) -> f64 {
        self.total_shares() * self.current_price
    }

    pub fn total_unrealised_gain(&self) -> f64 {
        self.lots.iter().map(|l| l.unrealised_gain(self.current_price)).sum()
    }

    pub fn weighted_cost_basis(&self) -> f64 {
        let total = self.total_shares();
        if total <= 0.0 { return 0.0; }
        self.total_cost_basis() / total
    }
}

/// Full portfolio snapshot at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Portfolio {
    pub positions: Vec<Position>,
    pub cash: f64,
    pub as_of: NaiveDate,
}

impl Portfolio {
    pub fn total_market_value(&self) -> f64 {
        self.positions.iter().map(|p| p.market_value()).sum::<f64>() + self.cash
    }

    pub fn weights(&self) -> Vec<(&str, f64)> {
        let total = self.total_market_value();
        if total <= 0.0 { return Vec::new(); }
        self.positions.iter()
            .map(|p| (p.ticker.as_str(), p.market_value() / total))
            .collect()
    }

    pub fn hhi(&self) -> f64 {
        self.weights().iter().map(|(_, w)| w * w).sum()
    }

    pub fn position(&self, ticker: &str) -> Option<&Position> {
        self.positions.iter().find(|p| p.ticker == ticker)
    }
}

/// A recorded sale/disposal event for audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Disposal {
    pub id: Uuid,
    pub lot_id: Uuid,
    pub ticker: String,
    pub shares_sold: f64,
    pub sale_price: f64,
    pub sale_date: NaiveDate,
    pub proceeds: f64,
    pub cost_basis: f64,
    pub realised_gain: f64,
    pub is_long_term: bool,
}
```

### 1.2 Database Schema -- Migration 012

File: `migrations/012_portfolio.sql`

```sql
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
```

### 1.3 Import (`src/portfolio/import.rs`)

Support three CSV formats out of the box, auto-detected by header row:

| Format | Required Columns |
|---|---|
| **Generic** | `ticker,shares,cost_basis,date,type` |
| **Schwab** | `Symbol,Quantity,Cost Basis,Date Acquired,Action` |
| **Fidelity** | `Symbol,Quantity,Cost Basis Per Share,Acquired Date,Type` |

```rust
use std::io::Read;
use csv::ReaderBuilder;
use crate::error::PortfolioError;
use super::types::{TaxLot, AcquisitionType};

/// Detect format from header row and parse all lots.
pub fn import_lots_from_reader<R: Read>(reader: R) -> Result<Vec<TaxLot>, PortfolioError> {
    // ... header detection, row parsing, UUID generation
    todo!()
}

/// Convenience wrapper for file path.
pub fn import_lots_from_file(path: &str) -> Result<Vec<TaxLot>, PortfolioError> {
    let file = std::fs::File::open(path)
        .map_err(|e| PortfolioError::ImportParse { line: 0, reason: e.to_string() })?;
    import_lots_from_reader(file)
}
```

### 1.4 Repository Extensions

Add to `Repository` trait in `src/storage/db.rs`:

```rust
// --- Portfolio / Tax Lots ---
fn upsert_tax_lot(&self, lot: &TaxLot) -> Result<(), StorageError>;
fn get_tax_lots(&self, ticker: &str) -> Result<Vec<TaxLot>, StorageError>;
fn get_all_tax_lots(&self) -> Result<Vec<TaxLot>, StorageError>;
fn update_lot_remaining_shares(&self, lot_id: &str, remaining: f64) -> Result<(), StorageError>;
fn record_disposal(&self, disposal: &Disposal) -> Result<(), StorageError>;
fn get_disposals(&self, ticker: &str, year: i32) -> Result<Vec<Disposal>, StorageError>;
fn save_portfolio_snapshot(&self, snapshot: &PortfolioSnapshotRow) -> Result<(), StorageError>;
fn get_portfolio_snapshots(&self, limit: usize) -> Result<Vec<PortfolioSnapshotRow>, StorageError>;
fn record_cash_transaction(&self, txn: &CashTransaction) -> Result<(), StorageError>;
fn get_cash_balance(&self) -> Result<f64, StorageError>;
```

### 1.5 CLI Commands

```rust
/// Portfolio management: import lots, view positions, record sales
Portfolio {
    #[command(subcommand)]
    action: PortfolioAction,
},

#[derive(Subcommand)]
enum PortfolioAction {
    /// Import tax lots from CSV file
    Import {
        /// Path to CSV file (Schwab, Fidelity, or generic format)
        #[arg(long)]
        file: String,
        /// Dry run: parse and validate without saving
        #[arg(long)]
        dry_run: bool,
    },
    /// Show current positions with cost basis and unrealised P&L
    Positions {
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Show all tax lots for a ticker
    Lots {
        /// Ticker symbol
        ticker: String,
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Show portfolio weights and concentration metrics
    Weights {
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Take a snapshot of current portfolio (for historical tracking)
    Snapshot,
}
```

### 1.6 MCP Tools

```rust
pub struct PortfolioWeightsInput {}
pub struct PortfolioWeightsOutput {
    pub total_value: f64,
    pub cash: f64,
    pub hhi: f64,
    pub positions: Vec<PositionWeight>,
}
pub struct PositionWeight {
    pub ticker: String,
    pub shares: f64,
    pub market_value: f64,
    pub weight_pct: f64,
    pub unrealised_gain: f64,
    pub cost_basis: f64,
}
```

Tool: `portfolio_weights` -- returns current portfolio composition.

### 1.7 Web Routes

| Route | Type | Description |
|---|---|---|
| `GET /portfolio` | Page | Portfolio overview with positions table, pie chart, HHI gauge |
| `GET /partials/portfolio-positions` | Partial | Positions table fragment |
| `GET /partials/portfolio-allocation` | Partial | SVG pie chart of allocation |
| `GET /partials/portfolio-concentration` | Partial | HHI over time chart |
| `POST /portfolio/import` | Mutation | Upload CSV via multipart form |

Add "Portfolio" to the nav bar between "Watchlist" and "Regime".

### 1.8 Testing Strategy

- **Unit:** `TaxLot::is_long_term`, `Position::market_value`, `Portfolio::hhi`, `Portfolio::weights` -- all pure computations.
- **Property (proptest):** For any set of positive lots, `total_shares >= 0`, `hhi in [0, 1]`, `weights sum to 1.0`.
- **Integration:** Import a fixture CSV, verify DB round-trip, verify snapshot serialisation.

---

## G2 -- Multi-Asset Data

### 2.1 Approach

The Yahoo provider (`src/data/yahoo.rs`) already fetches OHLCV candles for any
ticker symbol. ETFs like AGG, TIP, VNQ, GLD, EFA, EEM are valid Yahoo symbols.
The gap is conceptual -- the "universe" concept is stock-centric.

### 2.2 Changes

**`src/data/universes.rs`** -- add asset-class proxy lists:

```rust
pub const ASSET_CLASS_ETFS: &[(&str, &str)] = &[
    ("AGG", "US Aggregate Bond"),
    ("TIP", "TIPS / Inflation Protected"),
    ("VNQ", "US REITs"),
    ("GLD", "Gold"),
    ("EFA", "International Developed"),
    ("EEM", "Emerging Markets"),
    ("VTI", "US Total Market"),
    ("BND", "US Total Bond"),
    ("DBC", "Commodities"),
    ("IEF", "7-10Y Treasury"),
    ("TLT", "20+Y Treasury"),
    ("HYG", "High Yield Corporate"),
];

pub fn asset_class_tickers() -> Vec<&'static str> {
    ASSET_CLASS_ETFS.iter().map(|(t, _)| *t).collect()
}
```

**`src/config.rs`** -- add to `AppConfig`:

```rust
/// Additional tickers for multi-asset analysis (ETFs, indices)
#[serde(default)]
pub asset_class_universe: Vec<String>,
```

### 2.3 CLI

```rust
/// Fetch and cache candle data for asset class ETF proxies
AssetData {
    /// Number of years of history (default: 5)
    #[arg(long, default_value = "5")]
    years: u32,
},
```

### 2.4 Testing

- **Unit:** Verify `ASSET_CLASS_ETFS` contains no duplicates.
- **Integration (feature-gated):** Fetch 1 year of AGG candles from Yahoo, verify 200+ bars returned.

---

## G3 -- Correlation Matrix Engine

### 3.1 File: `src/analysis/correlation_matrix.rs`

```rust
use nalgebra::DMatrix;
use chrono::NaiveDate;
use crate::error::AnalysisError;

/// N x N correlation matrix with ticker labels.
#[derive(Debug, Clone)]
pub struct CorrelationMatrix {
    pub tickers: Vec<String>,
    pub matrix: DMatrix<f64>,
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,
}

impl CorrelationMatrix {
    /// Compute full pairwise Pearson correlation from daily return series.
    ///
    /// `returns` is a slice of (ticker, daily_returns) pairs.
    /// All return series must be the same length (aligned by date).
    pub fn from_returns(
        returns: &[(&str, &[f64])],
    ) -> Result<Self, AnalysisError> {
        let n = returns.len();
        if n < 2 {
            return Err(AnalysisError::InsufficientData { need: 2, got: n });
        }
        let len = returns[0].1.len();
        for (_, r) in returns.iter() {
            if r.len() != len {
                return Err(AnalysisError::InsufficientData { need: len, got: r.len() });
            }
        }

        let mut mat = DMatrix::zeros(n, n);
        for i in 0..n {
            mat[(i, i)] = 1.0;
            for j in (i + 1)..n {
                let corr = crate::analysis::correlation::pearson_correlation(
                    returns[i].1, returns[j].1,
                )?;
                mat[(i, j)] = corr;
                mat[(j, i)] = corr;
            }
        }

        Ok(Self {
            tickers: returns.iter().map(|(t, _)| t.to_string()).collect(),
            matrix: mat,
            period_start: NaiveDate::default(),
            period_end: NaiveDate::default(),
        })
    }

    /// Compute rolling correlation between two assets over a window.
    pub fn rolling_correlation(
        returns_a: &[f64],
        returns_b: &[f64],
        window: usize,
    ) -> Result<Vec<f64>, AnalysisError> {
        if returns_a.len() != returns_b.len() {
            return Err(AnalysisError::InsufficientData {
                need: returns_a.len(), got: returns_b.len(),
            });
        }
        if returns_a.len() < window {
            return Err(AnalysisError::InsufficientData {
                need: window, got: returns_a.len(),
            });
        }
        let mut result = Vec::with_capacity(returns_a.len() - window + 1);
        for i in 0..=(returns_a.len() - window) {
            let corr = crate::analysis::correlation::pearson_correlation(
                &returns_a[i..i + window],
                &returns_b[i..i + window],
            )?;
            result.push(corr);
        }
        Ok(result)
    }

    /// Extract the covariance matrix (annualised, 252 trading days).
    pub fn to_covariance_matrix(
        &self,
        returns: &[(&str, &[f64])],
    ) -> Result<DMatrix<f64>, AnalysisError> {
        let n = returns.len();
        let mut cov = DMatrix::zeros(n, n);
        for i in 0..n {
            let std_i = std_dev(returns[i].1);
            for j in 0..n {
                let std_j = std_dev(returns[j].1);
                cov[(i, j)] = self.matrix[(i, j)] * std_i * std_j * 252.0;
            }
        }
        Ok(cov)
    }
}

/// Crisis period definitions for conditional correlation analysis.
pub struct CrisisPeriod {
    pub name: &'static str,
    pub start: NaiveDate,
    pub end: NaiveDate,
}

pub const CRISIS_PERIODS: &[CrisisPeriod] = &[
    // These are populated at compile time with known dates
    // GFC, COVID crash, 2022 bear market
];

/// Compute correlation matrix for a specific date range (crisis analysis).
pub fn crisis_correlation(
    all_returns: &[(&str, &[(NaiveDate, f64)])],
    start: NaiveDate,
    end: NaiveDate,
) -> Result<CorrelationMatrix, AnalysisError> {
    // Filter returns to [start, end], then delegate to CorrelationMatrix::from_returns
    todo!()
}

fn std_dev(values: &[f64]) -> f64 {
    let n = values.len() as f64;
    if n < 2.0 { return 0.0; }
    let mean = values.iter().sum::<f64>() / n;
    let var = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
    var.sqrt()
}
```

### 3.2 Data Flow

1. Candles fetched via `DataProvider::fetch_candles` for each ticker (stock + ETF).
2. Daily returns computed: `r_t = (close_t - close_{t-1}) / close_{t-1}`.
3. Returns aligned by date (inner join on trading days).
4. `CorrelationMatrix::from_returns` called.
5. Result cached in DB or returned directly.

### 3.3 Database Schema -- Migration 013

```sql
-- Correlation snapshots for dashboard display
CREATE TABLE IF NOT EXISTS correlation_snapshots (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    computed_date   TEXT NOT NULL,
    tickers_json    TEXT NOT NULL,       -- JSON array of tickers
    matrix_json     TEXT NOT NULL,       -- JSON 2D array of correlations
    window_days     INTEGER NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_corr_snap_date ON correlation_snapshots(computed_date);
```

### 3.4 Web Routes

| Route | Description |
|---|---|
| `GET /correlation` | Full page with correlation heatmap |
| `GET /partials/correlation-heatmap` | SVG heatmap partial, params: `?window=756&tickers=AMZN,AGG,GLD,VNQ` |
| `GET /partials/rolling-correlation` | Line chart: rolling 63-day correlation between two selected tickers |

### 3.5 CLI

```rust
/// Compute and display correlation matrix for given tickers
Correlation {
    /// Tickers to correlate (space-separated)
    tickers: Vec<String>,
    /// Window in trading days (default: 756 = ~3 years)
    #[arg(long, default_value = "756")]
    window: usize,
    #[arg(long, value_enum, default_value = "table")]
    format: OutputFormat,
},
```

### 3.6 MCP Tool

```rust
pub struct CorrelationMatrixInput {
    pub tickers: Vec<String>,
    pub window_days: Option<usize>,
}
pub struct CorrelationMatrixOutput {
    pub tickers: Vec<String>,
    pub matrix: Vec<Vec<f64>>,
    pub high_correlations: Vec<HighCorrelation>, // pairs > 0.7
}
```

### 3.7 Testing

- **Unit:** 2-asset perfect correlation (identical returns) -> 1.0. Anti-correlated -> -1.0. Uncorrelated (random) -> near 0.
- **Property:** Matrix is symmetric. Diagonal is 1.0. All values in [-1, 1].
- **Integration:** Fetch AGG + VTI + GLD from Yahoo, compute matrix, verify shape and bounds.

---

## G4 -- Tax Calculation Engine

### 4.1 Types (`src/tax/brackets.rs`)

```rust
/// Federal capital gains tax brackets for a given year.
#[derive(Debug, Clone)]
pub struct FederalCapGainsBrackets {
    pub year: u32,
    pub filing_status: FilingStatus,
    /// (upper_bound, rate) pairs, ascending. Last entry has upper_bound = f64::INFINITY.
    pub long_term_brackets: Vec<(f64, f64)>,
    pub short_term_brackets: Vec<(f64, f64)>, // same as ordinary income brackets
    pub niit_threshold: f64,  // Net Investment Income Tax threshold
    pub niit_rate: f64,       // 3.8%
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilingStatus {
    Single,
    MarriedFilingJointly,
    MarriedFilingSeparately,
    HeadOfHousehold,
}

/// State tax rates (simplified: flat or top marginal).
#[derive(Debug, Clone)]
pub struct StateTaxRate {
    pub state: String,
    pub rate: f64,           // effective rate for capital gains
    pub has_ltcg_preference: bool,
}

impl FederalCapGainsBrackets {
    /// 2024 brackets for married filing jointly.
    pub fn mfj_2024() -> Self {
        Self {
            year: 2024,
            filing_status: FilingStatus::MarriedFilingJointly,
            long_term_brackets: vec![
                (94_050.0, 0.0),
                (583_750.0, 0.15),
                (f64::INFINITY, 0.20),
            ],
            short_term_brackets: vec![
                (23_200.0, 0.10),
                (94_300.0, 0.12),
                (201_050.0, 0.22),
                (383_900.0, 0.24),
                (487_450.0, 0.32),
                (731_200.0, 0.35),
                (f64::INFINITY, 0.37),
            ],
            niit_threshold: 250_000.0,
            niit_rate: 0.038,
        }
    }

    pub fn single_2024() -> Self {
        Self {
            year: 2024,
            filing_status: FilingStatus::Single,
            long_term_brackets: vec![
                (47_025.0, 0.0),
                (518_900.0, 0.15),
                (f64::INFINITY, 0.20),
            ],
            short_term_brackets: vec![
                (11_600.0, 0.10),
                (47_150.0, 0.12),
                (100_525.0, 0.22),
                (191_950.0, 0.24),
                (243_725.0, 0.32),
                (609_350.0, 0.35),
                (f64::INFINITY, 0.37),
            ],
            niit_threshold: 200_000.0,
            niit_rate: 0.038,
        }
    }
}

/// Look up state capital gains tax rate.
pub fn state_tax_rate(state_code: &str) -> Result<StateTaxRate, crate::error::TaxError> {
    match state_code {
        "CA" => Ok(StateTaxRate { state: "CA".into(), rate: 0.133, has_ltcg_preference: false }),
        "NY" => Ok(StateTaxRate { state: "NY".into(), rate: 0.109, has_ltcg_preference: false }),
        "TX" => Ok(StateTaxRate { state: "TX".into(), rate: 0.0,   has_ltcg_preference: false }),
        "FL" => Ok(StateTaxRate { state: "FL".into(), rate: 0.0,   has_ltcg_preference: false }),
        "WA" => Ok(StateTaxRate { state: "WA".into(), rate: 0.07,  has_ltcg_preference: false }),
        // ... additional states
        _ => Err(crate::error::TaxError::UnknownState(state_code.to_string())),
    }
}
```

### 4.2 Tax Engine (`src/tax/engine.rs`)

```rust
use crate::portfolio::types::{TaxLot, Disposal};
use super::brackets::{FederalCapGainsBrackets, StateTaxRate};

/// Result of computing tax on a disposal or hypothetical sale.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaxImpact {
    pub proceeds: f64,
    pub cost_basis: f64,
    pub realised_gain: f64,
    pub is_long_term: bool,
    pub federal_tax: f64,
    pub state_tax: f64,
    pub niit: f64,
    pub total_tax: f64,
    pub effective_rate: f64,
    pub after_tax_proceeds: f64,
}

/// Compute the total tax impact of selling `shares` of a specific lot.
pub fn compute_lot_tax(
    lot: &TaxLot,
    shares_to_sell: f64,
    sale_price: f64,
    sale_date: chrono::NaiveDate,
    other_income: f64,
    brackets: &FederalCapGainsBrackets,
    state: &StateTaxRate,
) -> TaxImpact {
    let is_lt = lot.is_long_term(sale_date);
    let cost_basis = shares_to_sell * lot.cost_basis_per_share;
    let proceeds = shares_to_sell * sale_price;
    let gain = proceeds - cost_basis;

    let federal_tax = if is_lt {
        bracket_tax(gain.max(0.0), &brackets.long_term_brackets, other_income)
    } else {
        bracket_tax(gain.max(0.0), &brackets.short_term_brackets, other_income)
    };

    let niit = if other_income + gain > brackets.niit_threshold {
        gain.max(0.0) * brackets.niit_rate
    } else {
        0.0
    };

    let state_tax = gain.max(0.0) * state.rate;
    let total_tax = federal_tax + niit + state_tax;

    TaxImpact {
        proceeds,
        cost_basis,
        realised_gain: gain,
        is_long_term: is_lt,
        federal_tax,
        state_tax,
        niit,
        total_tax,
        effective_rate: if gain > 0.0 { total_tax / gain } else { 0.0 },
        after_tax_proceeds: proceeds - total_tax,
    }
}

/// Marginal bracket tax computation with stacking.
fn bracket_tax(gain: f64, brackets: &[(f64, f64)], base_income: f64) -> f64 {
    // Walk through brackets, stacking gain on top of base_income
    let mut remaining = gain;
    let mut tax = 0.0;
    let mut lower = 0.0_f64;

    for &(upper, rate) in brackets {
        if base_income >= upper {
            lower = upper;
            continue;
        }
        let bracket_start = lower.max(base_income);
        let space = upper - bracket_start;
        let taxable = remaining.min(space);
        tax += taxable * rate;
        remaining -= taxable;
        if remaining <= 0.0 { break; }
        lower = upper;
    }
    tax
}
```

### 4.3 HIFO Lot Selection (`src/tax/hifo.rs`)

```rust
use crate::portfolio::types::TaxLot;

/// Select lots using Highest-In, First-Out ordering to minimise
/// realised gain for a given number of shares to sell.
///
/// Returns `(lots_to_sell, shares_from_each_lot)` pairs.
pub fn hifo_select(
    lots: &[TaxLot],
    shares_needed: f64,
) -> Vec<(usize, f64)> {
    // Sort lots by cost_basis_per_share descending (highest cost first)
    let mut indexed: Vec<(usize, &TaxLot)> = lots.iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        b.1.cost_basis_per_share
            .partial_cmp(&a.1.cost_basis_per_share)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut remaining = shares_needed;
    let mut selections = Vec::new();

    for (idx, lot) in indexed {
        if remaining <= 0.0 { break; }
        let available = lot.remaining_shares;
        if available <= 0.0 { continue; }
        let take = remaining.min(available);
        selections.push((idx, take));
        remaining -= take;
    }

    selections
}
```

### 4.4 Tax-Loss Harvesting Scanner (`src/tax/harvest.rs`)

```rust
use chrono::NaiveDate;
use crate::portfolio::types::TaxLot;

#[derive(Debug, Clone, serde::Serialize)]
pub struct HarvestOpportunity {
    pub ticker: String,
    pub lot_id: String,
    pub shares: f64,
    pub cost_basis: f64,
    pub current_value: f64,
    pub unrealised_loss: f64,
    pub days_held: i64,
    pub is_long_term: bool,
    /// True if a wash sale would apply (sold same ticker in last 30 days)
    pub wash_sale_risk: bool,
}

/// Scan all lots for harvestable losses.
pub fn scan_harvest_opportunities(
    lots: &[TaxLot],
    current_prices: &std::collections::HashMap<String, f64>,
    recent_sales: &[(String, NaiveDate)], // (ticker, sale_date)
    as_of: NaiveDate,
    min_loss: f64,
) -> Vec<HarvestOpportunity> {
    lots.iter()
        .filter_map(|lot| {
            let price = current_prices.get(&lot.ticker)?;
            let loss = lot.unrealised_gain(*price);
            if loss >= -min_loss { return None; } // not a big enough loss

            let wash_risk = recent_sales.iter().any(|(t, d)| {
                t == &lot.ticker && (as_of - *d).num_days().abs() <= 30
            });

            Some(HarvestOpportunity {
                ticker: lot.ticker.clone(),
                lot_id: lot.id.to_string(),
                shares: lot.remaining_shares,
                cost_basis: lot.total_cost_basis(),
                current_value: lot.remaining_shares * price,
                unrealised_loss: loss,
                days_held: lot.holding_period_days(as_of),
                is_long_term: lot.is_long_term(as_of),
                wash_sale_risk: wash_risk,
            })
        })
        .collect()
}
```

### 4.5 Multi-Year Projection (`src/tax/projection.rs`)

```rust
/// Project tax liability across multiple years of planned selling.
#[derive(Debug, Clone, serde::Serialize)]
pub struct YearProjection {
    pub year: u32,
    pub shares_sold: f64,
    pub proceeds: f64,
    pub realised_gain: f64,
    pub federal_tax: f64,
    pub state_tax: f64,
    pub niit: f64,
    pub total_tax: f64,
    pub cumulative_shares_sold: f64,
    pub remaining_shares: f64,
}

/// Given a selling schedule and assumptions, project year-by-year tax impact.
pub fn project_multi_year(
    lots: &[TaxLot],
    annual_shares: &[(u32, f64)],   // (year, shares_to_sell)
    assumed_price_growth: f64,       // annual % (e.g. 0.08 for 8%)
    current_price: f64,
    other_annual_income: f64,
    brackets: &FederalCapGainsBrackets,
    state: &StateTaxRate,
) -> Vec<YearProjection> {
    // For each year: project price, use HIFO to select lots, compute tax
    todo!()
}
```

### 4.6 Testing

- **Unit:** `bracket_tax` with known inputs against hand-calculated results. `hifo_select` ordering correctness. `is_long_term` boundary (365 vs 366 days).
- **Property:** For any positive gain, `total_tax <= gain`. `effective_rate in [0, 1]`. `after_tax_proceeds >= 0`.
- **Integration:** Full pipeline: import CSV -> HIFO select -> compute tax -> record disposal -> verify DB state.

### 4.7 CLI

```rust
/// Tax analysis: compute tax impact, find harvesting opportunities, project multi-year
Tax {
    #[command(subcommand)]
    action: TaxAction,
},

#[derive(Subcommand)]
enum TaxAction {
    /// Compute tax impact of selling N shares of a ticker
    Impact {
        ticker: String,
        shares: f64,
        /// Assumed sale price (or "market" for latest close)
        #[arg(long, default_value = "market")]
        price: String,
        /// Annual other income for bracket stacking
        #[arg(long, default_value = "0")]
        other_income: f64,
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Scan for tax-loss harvesting opportunities
    Harvest {
        #[arg(long, default_value = "1000")]
        min_loss: f64,
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Project multi-year liquidation tax schedule
    Project {
        ticker: String,
        /// Annual shares to sell (comma-separated: 100,200,300)
        #[arg(long)]
        annual_shares: String,
        #[arg(long, default_value = "0.08")]
        price_growth: f64,
        #[arg(long, default_value = "0")]
        other_income: f64,
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
}
```

---

## G5 -- DCF / Valuation Module

### 5.1 Three-Stage DCF (`src/valuation/dcf.rs`)

```rust
/// Inputs for a 3-stage DCF model.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DcfInputs {
    pub base_revenue: f64,
    pub base_fcf: f64,
    /// Stage 1: high growth (years 1-5)
    pub stage1_growth_rate: f64,
    pub stage1_years: u32,
    /// Stage 2: transition (years 6-10)
    pub stage2_growth_rate: f64,
    pub stage2_years: u32,
    /// Stage 3: terminal growth rate
    pub terminal_growth_rate: f64,
    /// FCF margin assumed at maturity
    pub terminal_fcf_margin: f64,
    /// Weighted average cost of capital
    pub wacc: f64,
    /// Shares outstanding (diluted)
    pub shares_outstanding: f64,
    /// Net debt (debt - cash)
    pub net_debt: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DcfResult {
    pub enterprise_value: f64,
    pub equity_value: f64,
    pub fair_value_per_share: f64,
    pub terminal_value: f64,
    pub terminal_pct_of_ev: f64,
    pub yearly_fcf: Vec<(u32, f64)>,
    pub implied_upside_pct: f64,     // vs current price
}

pub fn three_stage_dcf(inputs: &DcfInputs) -> Result<DcfResult, crate::error::ValuationError> {
    if inputs.wacc <= inputs.terminal_growth_rate {
        return Err(crate::error::ValuationError::InvalidWacc(
            "WACC must exceed terminal growth rate".into(),
        ));
    }

    let mut fcfs = Vec::new();
    let mut revenue = inputs.base_revenue;
    let mut year = 0u32;

    // Stage 1
    for _ in 0..inputs.stage1_years {
        year += 1;
        revenue *= 1.0 + inputs.stage1_growth_rate;
        let fcf = revenue * inputs.terminal_fcf_margin; // simplified
        fcfs.push((year, fcf));
    }

    // Stage 2: linear interpolation of growth rate
    let growth_step = (inputs.stage1_growth_rate - inputs.stage2_growth_rate)
        / inputs.stage2_years as f64;
    let mut growth = inputs.stage1_growth_rate;
    for _ in 0..inputs.stage2_years {
        year += 1;
        growth -= growth_step;
        revenue *= 1.0 + growth;
        let fcf = revenue * inputs.terminal_fcf_margin;
        fcfs.push((year, fcf));
    }

    // PV of explicit period
    let pv_fcfs: f64 = fcfs.iter()
        .map(|(y, fcf)| fcf / (1.0 + inputs.wacc).powi(*y as i32))
        .sum();

    // Terminal value (Gordon Growth)
    let terminal_fcf = *fcfs.last().map(|(_, f)| f).unwrap_or(&0.0) * (1.0 + inputs.terminal_growth_rate);
    let terminal_value = terminal_fcf / (inputs.wacc - inputs.terminal_growth_rate);
    let pv_terminal = terminal_value / (1.0 + inputs.wacc).powi(year as i32);

    let ev = pv_fcfs + pv_terminal;
    let equity = ev - inputs.net_debt;
    let per_share = equity / inputs.shares_outstanding;

    Ok(DcfResult {
        enterprise_value: ev,
        equity_value: equity,
        fair_value_per_share: per_share,
        terminal_value: pv_terminal,
        terminal_pct_of_ev: pv_terminal / ev * 100.0,
        yearly_fcf: fcfs,
        implied_upside_pct: 0.0, // caller sets from current price
    })
}
```

### 5.2 Reverse DCF (`src/valuation/reverse_dcf.rs`)

```rust
/// Given a current stock price, solve for the implied revenue growth rate.
/// Uses bisection method.
pub fn reverse_dcf(
    current_price: f64,
    base_inputs: &DcfInputs,
    tolerance: f64,
    max_iterations: u32,
) -> Result<f64, crate::error::ValuationError> {
    let mut lo = -0.10;  // -10% growth
    let mut hi = 0.80;   // +80% growth
    let target_ev = current_price * base_inputs.shares_outstanding + base_inputs.net_debt;

    for _ in 0..max_iterations {
        let mid = (lo + hi) / 2.0;
        let mut inputs = base_inputs.clone();
        inputs.stage1_growth_rate = mid;
        let result = three_stage_dcf(&inputs)?;
        let diff = result.enterprise_value - target_ev;
        if diff.abs() / target_ev < tolerance {
            return Ok(mid);
        }
        if diff > 0.0 { hi = mid; } else { lo = mid; }
    }
    Err(crate::error::ValuationError::NoConvergence { iterations: max_iterations })
}
```

### 5.3 Scenario Engine (`src/valuation/scenario.rs`)

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct Scenario {
    pub name: String,           // "Bear", "Base", "Bull"
    pub probability: f64,       // e.g. 0.25, 0.50, 0.25
    pub dcf_inputs: DcfInputs,
    pub result: DcfResult,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ScenarioAnalysis {
    pub scenarios: Vec<Scenario>,
    pub expected_value: f64,     // probability-weighted fair value
    pub current_price: f64,
    pub expected_upside_pct: f64,
}

pub fn run_scenarios(
    scenarios: Vec<(String, f64, DcfInputs)>,
    current_price: f64,
) -> Result<ScenarioAnalysis, crate::error::ValuationError> {
    let mut results = Vec::new();
    for (name, prob, inputs) in scenarios {
        let result = three_stage_dcf(&inputs)?;
        results.push(Scenario { name, probability: prob, dcf_inputs: inputs, result });
    }
    let ev = results.iter()
        .map(|s| s.probability * s.result.fair_value_per_share)
        .sum::<f64>();
    Ok(ScenarioAnalysis {
        scenarios: results,
        expected_value: ev,
        current_price,
        expected_upside_pct: (ev - current_price) / current_price * 100.0,
    })
}
```

### 5.4 CLI

```rust
/// Valuation tools: DCF, reverse DCF, scenario analysis
Valuation {
    #[command(subcommand)]
    action: ValuationAction,
},

#[derive(Subcommand)]
enum ValuationAction {
    /// Run 3-stage DCF model
    Dcf {
        /// Path to JSON file with DCF inputs
        #[arg(long)]
        inputs: String,
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Reverse DCF: what growth rate does the market imply?
    ReverseDcf {
        ticker: String,
        #[arg(long)]
        inputs: String,
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Run bear/base/bull scenario analysis
    Scenarios {
        #[arg(long)]
        inputs: String,
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
}
```

### 5.5 Web Routes

| Route | Description |
|---|---|
| `GET /valuation` | Page with DCF calculator form |
| `POST /valuation/dcf` | Run DCF, return result partial |
| `GET /partials/scenario-chart` | SVG bar chart of scenario outcomes |

### 5.6 MCP Tool

```rust
pub struct DcfInput { pub inputs: DcfInputs }
pub struct DcfOutput { pub result: DcfResult }
// Tool: "run_dcf"

pub struct ReverseDcfInput { pub current_price: f64, pub inputs: DcfInputs }
pub struct ReverseDcfOutput { pub implied_growth: f64 }
// Tool: "reverse_dcf"
```

### 5.7 Testing

- **Unit:** Known DCF (constant growth = Gordon Growth) matches closed-form. Reverse DCF recovers input growth rate. WACC < terminal growth returns error.
- **Property:** `fair_value_per_share > 0` for any positive FCF and valid WACC. `terminal_pct_of_ev in [0, 100]`.

---

## G6 -- Fundamental Time-Series

### 6.1 Database Schema -- Migration 014

```sql
CREATE TABLE IF NOT EXISTS fundamental_history (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker          TEXT NOT NULL,
    period_date     TEXT NOT NULL,       -- YYYY-MM-DD (quarter end)
    period_type     TEXT NOT NULL,       -- 'Q1','Q2','Q3','Q4','FY'
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
```

### 6.2 Repository Extensions

```rust
fn upsert_fundamental_history(&self, record: &FundamentalHistoryRecord) -> Result<(), StorageError>;
fn get_fundamental_history(
    &self, ticker: &str, limit: usize,
) -> Result<Vec<FundamentalHistoryRecord>, StorageError>;
fn get_valuation_bands(
    &self, ticker: &str, metric: &str, years: u32,
) -> Result<Vec<(String, f64)>, StorageError>;
```

### 6.3 Web Routes

| Route | Description |
|---|---|
| `GET /partials/valuation-bands` | Line chart showing 5Y P/E, P/S, EV/EBITDA band (min/median/max) |
| `GET /partials/margin-trajectory` | Line chart of gross/operating/net margin over quarters |

### 6.4 Testing

- **Unit:** Verify `upsert` is idempotent (second insert updates, doesn't duplicate).
- **Integration:** Insert 8 quarters, query bands, verify min/median/max calculations.

---

## G7 -- Portfolio Optimisation (MVO / Risk Parity)

### 7.1 MVO (`src/optimize/mvo.rs`)

```rust
use nalgebra::DMatrix;

/// Inputs for Mean-Variance Optimisation.
#[derive(Debug, Clone)]
pub struct MvoInputs {
    pub tickers: Vec<String>,
    /// Expected annual returns per asset
    pub expected_returns: Vec<f64>,
    /// Annualised covariance matrix (N x N)
    pub covariance: DMatrix<f64>,
    /// Min weight per asset (default 0.0 = no short selling)
    pub min_weights: Vec<f64>,
    /// Max weight per asset (default 1.0)
    pub max_weights: Vec<f64>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct OptimalPortfolio {
    pub weights: Vec<(String, f64)>,
    pub expected_return: f64,
    pub volatility: f64,
    pub sharpe_ratio: f64,
}

/// Solve for the minimum-variance portfolio for a given target return.
///
/// Uses OSQP (Operator Splitting Quadratic Program) solver.
///
/// min  0.5 * w' * Sigma * w
/// s.t. mu' * w >= target_return
///      sum(w) = 1
///      min_w <= w <= max_w
pub fn minimum_variance(
    inputs: &MvoInputs,
    target_return: f64,
    risk_free_rate: f64,
) -> Result<OptimalPortfolio, crate::error::OptimizeError> {
    let n = inputs.tickers.len();
    // Construct OSQP problem matrices
    // P = covariance matrix (upper triangular CSC)
    // q = zero vector
    // A = [expected_returns; ones; identity] (stacked constraints)
    // l, u = constraint bounds
    todo!()
}

/// Solve for the tangency (max Sharpe) portfolio.
pub fn max_sharpe(
    inputs: &MvoInputs,
    risk_free_rate: f64,
) -> Result<OptimalPortfolio, crate::error::OptimizeError> {
    // Sweep target returns from min to max, find highest Sharpe
    todo!()
}
```

### 7.2 Risk Parity (`src/optimize/risk_parity.rs`)

```rust
/// Equal Risk Contribution portfolio.
///
/// Iterative algorithm: adjust weights so each asset contributes equally
/// to total portfolio variance. Converges via Newton-like steps.
pub fn risk_parity(
    covariance: &DMatrix<f64>,
    tickers: &[String],
    tolerance: f64,
    max_iterations: u32,
) -> Result<OptimalPortfolio, crate::error::OptimizeError> {
    let n = covariance.nrows();
    let mut weights = vec![1.0 / n as f64; n]; // start equal-weight

    for _ in 0..max_iterations {
        // Compute marginal risk contribution for each asset
        // Adjust weights proportionally
        // Check convergence: all risk contributions equal within tolerance
        todo!()
    }

    Err(crate::error::OptimizeError::SolverFailed("max iterations".into()))
}
```

### 7.3 Efficient Frontier (`src/optimize/frontier.rs`)

```rust
/// Compute the efficient frontier as a series of (volatility, return) points.
pub fn efficient_frontier(
    inputs: &MvoInputs,
    risk_free_rate: f64,
    points: usize,
) -> Result<Vec<OptimalPortfolio>, crate::error::OptimizeError> {
    let min_ret = inputs.expected_returns.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_ret = inputs.expected_returns.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let step = (max_ret - min_ret) / (points - 1) as f64;

    (0..points)
        .map(|i| {
            let target = min_ret + step * i as f64;
            minimum_variance(inputs, target, risk_free_rate)
        })
        .collect()
}
```

### 7.4 Model Portfolios

```rust
/// Pre-defined model portfolio templates.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModelPortfolio {
    pub name: String,
    pub description: String,
    pub weights: Vec<(String, f64)>,
}

pub fn conservative() -> ModelPortfolio {
    ModelPortfolio {
        name: "Conservative".into(),
        description: "60% bonds, 30% equities, 10% alternatives".into(),
        weights: vec![
            ("AGG".into(), 0.40), ("TIP".into(), 0.10), ("IEF".into(), 0.10),
            ("VTI".into(), 0.20), ("EFA".into(), 0.10), ("GLD".into(), 0.05),
            ("VNQ".into(), 0.05),
        ],
    }
}

pub fn moderate() -> ModelPortfolio { /* 40/50/10 */ todo!() }
pub fn growth() -> ModelPortfolio { /* 20/70/10 */ todo!() }
```

### 7.5 CLI

```rust
/// Portfolio optimisation: MVO, risk parity, efficient frontier
Optimize {
    #[command(subcommand)]
    action: OptimizeAction,
},

#[derive(Subcommand)]
enum OptimizeAction {
    /// Compute efficient frontier for given tickers
    Frontier {
        tickers: Vec<String>,
        #[arg(long, default_value = "20")]
        points: usize,
        #[arg(long, default_value = "0.05")]
        risk_free_rate: f64,
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Maximum Sharpe portfolio
    MaxSharpe {
        tickers: Vec<String>,
        #[arg(long, default_value = "0.05")]
        risk_free_rate: f64,
    },
    /// Risk parity allocation
    RiskParity {
        tickers: Vec<String>,
    },
    /// Show model portfolio templates
    Models,
}
```

### 7.6 Web Routes

| Route | Description |
|---|---|
| `GET /optimize` | Page with optimiser form + frontier chart |
| `POST /optimize/frontier` | Compute frontier, return SVG chart partial |
| `GET /partials/model-portfolios` | Table of model portfolio weights |

### 7.7 MCP Tool

```rust
// Tool: "optimize_portfolio"
pub struct OptimizeInput {
    pub tickers: Vec<String>,
    pub method: String, // "max_sharpe" | "risk_parity" | "min_variance"
}
pub struct OptimizeOutput {
    pub weights: Vec<(String, f64)>,
    pub expected_return: f64,
    pub volatility: f64,
    pub sharpe: f64,
}
```

### 7.8 Testing

- **Unit:** 2-asset MVO with known analytical solution. Risk parity with uncorrelated equal-vol assets returns equal weights.
- **Property:** All weights >= 0 (long-only constraint). Weights sum to 1.0 +/- epsilon. Frontier returns are monotonically increasing.
- **Integration:** Fetch real data for VTI+AGG+GLD, compute frontier, verify 20 points returned.

---

## G8 -- ETF Screening & Holdings Data

### 8.1 Types (`src/data/etf.rs`)

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtfSnapshot {
    pub ticker: String,
    pub name: String,
    pub expense_ratio: f64,       // e.g. 0.0003 for VTI
    pub aum: f64,                 // in USD
    pub avg_daily_volume: u64,
    pub category: String,         // "US Large Cap", "Aggregate Bond", etc.
    pub inception_date: String,
    pub tracking_index: String,
    pub top_holdings: Vec<EtfHolding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EtfHolding {
    pub ticker: String,
    pub name: String,
    pub weight: f64,              // e.g. 0.065 for 6.5%
}

#[async_trait::async_trait]
pub trait EtfProvider: Send + Sync {
    async fn fetch_etf_snapshot(&self, ticker: &str) -> Result<EtfSnapshot, crate::error::DataError>;
    async fn fetch_etf_holdings(&self, ticker: &str) -> Result<Vec<EtfHolding>, crate::error::DataError>;
}

/// Check how much of a specific stock you'd be adding back via an ETF.
pub fn overlap_analysis(
    holdings: &[EtfHolding],
    concentrated_ticker: &str,
) -> Option<f64> {
    holdings.iter()
        .find(|h| h.ticker == concentrated_ticker)
        .map(|h| h.weight)
}
```

### 8.2 Database Schema -- Migration 015

```sql
CREATE TABLE IF NOT EXISTS etf_snapshots (
    ticker          TEXT PRIMARY KEY,
    name            TEXT NOT NULL,
    expense_ratio   REAL,
    aum             REAL,
    avg_daily_volume INTEGER,
    category        TEXT,
    inception_date  TEXT,
    tracking_index  TEXT,
    holdings_json   TEXT,       -- JSON array of {ticker, name, weight}
    fetched_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### 8.3 CLI

```rust
/// ETF analysis: compare ETFs, check holdings overlap
Etf {
    #[command(subcommand)]
    action: EtfAction,
},

#[derive(Subcommand)]
enum EtfAction {
    /// Show ETF snapshot (holdings, expense ratio, AUM)
    Info { ticker: String },
    /// Check overlap: how much of TICKER is in this ETF?
    Overlap {
        etf: String,
        ticker: String,
    },
    /// Compare multiple ETFs side-by-side
    Compare { tickers: Vec<String> },
}
```

### 8.4 Testing

- **Unit:** `overlap_analysis` returns correct weight. Missing ticker returns None.
- **Integration:** Fetch VTI holdings from Finnhub, verify AAPL/MSFT are present.

---

## G9 -- Selling Programme / Liquidation Scheduler

### 9.1 Types (`src/portfolio/mod.rs` -- extend)

```rust
/// A multi-year selling programme defining when and how much to sell.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SellingProgramme {
    pub ticker: String,
    pub schedule: Vec<SellingPeriod>,
    pub pace_rules: Vec<PaceRule>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SellingPeriod {
    pub year: u32,
    pub quarter: u8,
    pub shares_target: f64,
    pub dollar_target: Option<f64>,
    pub executed_shares: f64,
    pub executed_proceeds: f64,
}

/// Rules that accelerate or decelerate the selling pace.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PaceRule {
    pub condition: PaceCondition,
    pub multiplier: f64,  // e.g. 2.0 = double pace, 0.5 = halve
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PaceCondition {
    /// Accelerate if stock breaks below 200-SMA
    Below200Sma,
    /// Accelerate if market regime is Bear
    BearRegime,
    /// Decelerate if stock is in strong uptrend (buy more time for LTCG)
    Stage2Uptrend,
    /// Accelerate if price > target (take profits)
    PriceAbove(f64),
}
```

### 9.2 Database Schema -- Migration 016

```sql
CREATE TABLE IF NOT EXISTS selling_programmes (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker          TEXT NOT NULL,
    schedule_json   TEXT NOT NULL,
    pace_rules_json TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS selling_executions (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    programme_id    INTEGER NOT NULL REFERENCES selling_programmes(id),
    execution_date  TEXT NOT NULL,
    shares_sold     REAL NOT NULL,
    price           REAL NOT NULL,
    proceeds        REAL NOT NULL,
    lot_ids_json    TEXT,
    notes           TEXT,
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
```

### 9.3 Integration with Existing Modules

- **Regime detection:** `detect_regime()` output feeds `PaceCondition::BearRegime`.
- **Exit signals:** `check_exit_signals()` output feeds `PaceCondition::Below200Sma`.
- **Tax engine (G4):** Each period's tax impact computed via `compute_lot_tax` with HIFO selection.

### 9.4 CLI

```rust
/// Selling programme: create, view, and track a liquidation schedule
Sell {
    #[command(subcommand)]
    action: SellAction,
},

#[derive(Subcommand)]
enum SellAction {
    /// Create a new selling programme
    Plan {
        ticker: String,
        /// Shares per year (comma-separated for each year)
        #[arg(long)]
        annual_shares: String,
        /// Number of years
        #[arg(long)]
        years: u32,
    },
    /// Show current programme with projected tax impact
    Show {
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Record an executed sale
    Execute {
        ticker: String,
        shares: f64,
        price: f64,
    },
}
```

### 9.5 Testing

- **Unit:** `PaceRule` multiplier correctly doubles shares in bear regime. Schedule JSON round-trip.
- **Integration:** Create programme -> execute partial sale -> verify remaining schedule updates.

---

## G10 -- Options Data & Pricing

### 10.1 Types (`src/data/options.rs`)

```rust
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionContract {
    pub ticker: String,
    pub expiration: NaiveDate,
    pub strike: f64,
    pub option_type: OptionType,
    pub last_price: f64,
    pub bid: f64,
    pub ask: f64,
    pub volume: u64,
    pub open_interest: u64,
    pub implied_volatility: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum OptionType { Call, Put }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptionChain {
    pub ticker: String,
    pub underlying_price: f64,
    pub expirations: Vec<NaiveDate>,
    pub contracts: Vec<OptionContract>,
}

/// Black-Scholes price for a European option.
pub fn black_scholes(
    spot: f64,
    strike: f64,
    time_to_expiry: f64,   // in years
    risk_free_rate: f64,
    volatility: f64,
    option_type: &OptionType,
) -> f64 {
    let d1 = ((spot / strike).ln() + (risk_free_rate + 0.5 * volatility.powi(2)) * time_to_expiry)
        / (volatility * time_to_expiry.sqrt());
    let d2 = d1 - volatility * time_to_expiry.sqrt();

    match option_type {
        OptionType::Call => spot * normal_cdf(d1) - strike * (-risk_free_rate * time_to_expiry).exp() * normal_cdf(d2),
        OptionType::Put => strike * (-risk_free_rate * time_to_expiry).exp() * normal_cdf(-d2) - spot * normal_cdf(-d1),
    }
}

/// Design a zero-cost collar: find put/call strikes for given parameters.
#[derive(Debug, Clone, Serialize)]
pub struct CollarDesign {
    pub put_strike: f64,
    pub call_strike: f64,
    pub put_premium: f64,
    pub call_premium: f64,
    pub net_premium: f64,      // target: ~0 for zero-cost
    pub max_loss_pct: f64,     // from put strike
    pub max_gain_pct: f64,     // from call strike
    pub expiration: NaiveDate,
}

pub fn design_collar(
    spot: f64,
    chain: &OptionChain,
    target_expiration: NaiveDate,
    max_downside_pct: f64,
) -> Option<CollarDesign> {
    // Find put at target downside, then find call whose premium offsets
    todo!()
}

fn normal_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

fn erf(x: f64) -> f64 {
    // Abramowitz & Stegun approximation (7.1.26)
    let a1 = 0.254829592;
    let a2 = -0.284496736;
    let a3 = 1.421413741;
    let a4 = -1.453152027;
    let a5 = 1.061405429;
    let p = 0.3275911;
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + p * x);
    let y = 1.0 - (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) * t * (-x * x).exp();
    sign * y
}
```

### 10.2 Data Provider

```rust
#[async_trait::async_trait]
pub trait OptionsProvider: Send + Sync {
    async fn fetch_option_chain(
        &self, ticker: &str,
    ) -> Result<OptionChain, crate::error::DataError>;
}
```

Implement via Finnhub `/stock/option-chain` endpoint.

### 10.3 Testing

- **Unit:** Black-Scholes put-call parity: `C - P = S - K * e^(-rT)`. Known option price from textbook example. `normal_cdf(0) == 0.5`.
- **Property:** Call price monotonically increasing with spot. Put price monotonically decreasing with spot.

---

## G11 -- Segment-Level Financials

### 11.1 Types (`src/data/segments.rs`)

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SegmentFinancials {
    pub ticker: String,
    pub period: String,             // "2024-Q3"
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub name: String,               // "AWS", "Advertising", "Retail"
    pub revenue: f64,
    pub operating_income: Option<f64>,
    pub operating_margin: Option<f64>,
}

#[async_trait::async_trait]
pub trait SegmentProvider: Send + Sync {
    async fn fetch_segments(
        &self, ticker: &str, periods: usize,
    ) -> Result<Vec<SegmentFinancials>, crate::error::DataError>;
}
```

### 11.2 Database Schema -- Migration 017

```sql
CREATE TABLE IF NOT EXISTS segment_financials (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker          TEXT NOT NULL,
    period          TEXT NOT NULL,
    segment_name    TEXT NOT NULL,
    revenue         REAL,
    operating_income REAL,
    operating_margin REAL,
    fetched_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_seg_fin_unique
    ON segment_financials(ticker, period, segment_name);
```

### 11.3 SOTP Calculator (`src/valuation/sotp.rs`)

```rust
#[derive(Debug, Clone, serde::Serialize)]
pub struct SotpValuation {
    pub ticker: String,
    pub segment_values: Vec<SegmentValue>,
    pub total_enterprise_value: f64,
    pub net_debt: f64,
    pub equity_value: f64,
    pub per_share: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SegmentValue {
    pub segment_name: String,
    pub revenue: f64,
    pub multiple: f64,           // EV/Revenue or EV/EBITDA applied
    pub implied_value: f64,
}

pub fn sum_of_the_parts(
    segments: &[Segment],
    multiples: &[(String, f64)],  // (segment_name, multiple)
    net_debt: f64,
    shares_outstanding: f64,
) -> SotpValuation {
    let segment_values: Vec<SegmentValue> = segments.iter()
        .map(|s| {
            let mult = multiples.iter()
                .find(|(n, _)| n == &s.name)
                .map(|(_, m)| *m)
                .unwrap_or(3.0); // default multiple
            SegmentValue {
                segment_name: s.name.clone(),
                revenue: s.revenue,
                multiple: mult,
                implied_value: s.revenue * mult,
            }
        })
        .collect();

    let total_ev: f64 = segment_values.iter().map(|v| v.implied_value).sum();
    let equity = total_ev - net_debt;

    SotpValuation {
        ticker: String::new(),
        segment_values,
        total_enterprise_value: total_ev,
        net_debt,
        equity_value: equity,
        per_share: equity / shares_outstanding,
    }
}
```

### 11.4 Testing

- **Unit:** SOTP with known segments and multiples matches hand calculation.

---

## G12 -- Institutional Ownership / 13F

### 12.1 Types (`src/data/ownership.rs`)

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InstitutionalHolder {
    pub name: String,
    pub shares: u64,
    pub value_usd: f64,
    pub pct_of_outstanding: f64,
    pub change_shares: i64,        // quarter-over-quarter change
    pub filing_date: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OwnershipSummary {
    pub ticker: String,
    pub total_institutional_pct: f64,
    pub top_holders: Vec<InstitutionalHolder>,
    pub net_buying: bool,          // aggregate change positive
    pub quarters_of_accumulation: u32,
}
```

### 12.2 Database Schema -- Migration 018

```sql
CREATE TABLE IF NOT EXISTS institutional_ownership (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    ticker          TEXT NOT NULL,
    holder_name     TEXT NOT NULL,
    shares          INTEGER NOT NULL,
    value_usd       REAL,
    pct_of_outstanding REAL,
    change_shares   INTEGER,
    filing_date     TEXT NOT NULL,
    fetched_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_inst_own_ticker ON institutional_ownership(ticker);
```

### 12.3 Testing

- **Unit:** `net_buying` correctly detects accumulation vs distribution.

---

## G13 -- Factor Decomposition

### 13.1 File: `src/analysis/factor.rs`

```rust
use nalgebra::{DMatrix, DVector};

/// Fama-French factor exposure from regression.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FactorExposure {
    pub ticker: String,
    pub alpha: f64,                   // annualised alpha (intercept * 252)
    pub market_beta: f64,
    pub smb_beta: f64,                // size factor
    pub hml_beta: f64,                // value factor
    pub rmw_beta: f64,                // profitability
    pub cma_beta: f64,                // investment
    pub r_squared: f64,
    pub residual_vol: f64,
}

/// Run OLS regression: R_excess = alpha + beta_mkt * MKT + beta_smb * SMB + ...
///
/// `factor_returns` is a (T x 5) matrix of daily factor returns.
/// `asset_returns` is a (T x 1) vector of daily excess returns.
pub fn factor_regression(
    asset_returns: &[f64],
    factor_returns: &DMatrix<f64>,
) -> Result<FactorExposure, crate::error::AnalysisError> {
    let t = asset_returns.len();
    if t != factor_returns.nrows() {
        return Err(crate::error::AnalysisError::InsufficientData {
            need: factor_returns.nrows(), got: t,
        });
    }

    // OLS: beta = (X'X)^{-1} X'y
    // X = [ones | factor_returns]  (T x 6)
    let ones = DVector::from_element(t, 1.0);
    let x = DMatrix::from_columns(&[
        ones.clone(),
        factor_returns.column(0).into(),
        factor_returns.column(1).into(),
        factor_returns.column(2).into(),
        factor_returns.column(3).into(),
        factor_returns.column(4).into(),
    ]);
    let y = DVector::from_column_slice(asset_returns);

    let xtx = x.transpose() * &x;
    let xty = x.transpose() * &y;
    let beta = xtx.try_inverse()
        .ok_or(crate::error::AnalysisError::DivisionByZero)?
        * xty;

    let y_hat = &x * &beta;
    let residuals = &y - &y_hat;
    let ss_res = residuals.dot(&residuals);
    let y_mean = y.mean();
    let ss_tot = y.iter().map(|v| (v - y_mean).powi(2)).sum::<f64>();
    let r_sq = 1.0 - ss_res / ss_tot;

    Ok(FactorExposure {
        ticker: String::new(),
        alpha: beta[0] * 252.0,
        market_beta: beta[1],
        smb_beta: beta[2],
        hml_beta: beta[3],
        rmw_beta: beta[4],
        cma_beta: beta[5],
        r_squared: r_sq,
        residual_vol: (ss_res / (t - 6) as f64).sqrt() * 252.0_f64.sqrt(),
    })
}
```

### 13.2 Factor Data Source

Fama-French factor returns are freely available from Ken French's Data Library
as CSV files. Add a fetcher that downloads and caches the daily factor file:

```rust
/// Fetch Fama-French 5-factor daily returns from Ken French Data Library.
pub async fn fetch_ff5_factors(
    from: NaiveDate, to: NaiveDate,
) -> Result<DMatrix<f64>, crate::error::DataError> {
    // URL: https://mba.tuck.dartmouth.edu/pages/faculty/ken.french/ftp/F-F_Research_Data_5_Factors_2x3_daily_CSV.zip
    // Download, unzip, parse CSV, return as matrix
    todo!()
}
```

### 13.3 Testing

- **Unit:** Regression with synthetic data (known betas) recovers them. R-squared = 1.0 for perfectly explained returns.
- **Property:** `r_squared in [0, 1]`.

---

## G14 -- Forward Monte Carlo

### 14.1 Extension to `src/backtest/monte_carlo.rs`

```rust
/// Forward-looking portfolio simulation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ForwardSimConfig {
    pub initial_value: f64,
    pub annual_return: f64,
    pub annual_volatility: f64,
    pub years: u32,
    pub annual_withdrawal: f64,
    pub annual_contribution: f64,
    pub tax_drag_pct: f64,          // annual tax drag (e.g. 0.005 for 50bps)
    pub inflation_rate: f64,
    pub rebalance_frequency: u32,   // months between rebalances
    pub iterations: u32,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ForwardSimResult {
    pub median_terminal: f64,
    pub p10_terminal: f64,
    pub p90_terminal: f64,
    pub success_rate: f64,          // % of paths where value > 0
    pub target_success_rate: Option<f64>, // % of paths reaching goal
    pub percentile_paths: Vec<Vec<f64>>,  // [p10, p25, p50, p75, p90] yearly values
}

/// Run geometric Brownian motion forward simulation.
pub fn forward_monte_carlo(
    config: &ForwardSimConfig,
    goal: Option<f64>,
    seed: u64,
) -> ForwardSimResult {
    let mut rng = StdRng::seed_from_u64(seed);
    let dt = 1.0 / 12.0; // monthly steps
    let steps = config.years as usize * 12;
    let monthly_return = config.annual_return / 12.0;
    let monthly_vol = config.annual_volatility / 12.0_f64.sqrt();
    let monthly_tax_drag = config.tax_drag_pct / 12.0;
    let monthly_withdrawal = config.annual_withdrawal / 12.0;
    let monthly_contribution = config.annual_contribution / 12.0;

    let mut terminal_values = Vec::with_capacity(config.iterations as usize);

    for _ in 0..config.iterations {
        let mut value = config.initial_value;
        for _step in 0..steps {
            let z: f64 = rng.sample(rand::distributions::Standard);
            let ret = monthly_return - 0.5 * monthly_vol.powi(2) + monthly_vol * z;
            value *= ret.exp();
            value -= monthly_tax_drag * value;
            value += monthly_contribution - monthly_withdrawal;
            if value < 0.0 { value = 0.0; break; }
        }
        terminal_values.push(value);
    }

    terminal_values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = terminal_values.len();

    ForwardSimResult {
        median_terminal: terminal_values[n / 2],
        p10_terminal: terminal_values[n / 10],
        p90_terminal: terminal_values[n * 9 / 10],
        success_rate: terminal_values.iter().filter(|&&v| v > 0.0).count() as f64 / n as f64 * 100.0,
        target_success_rate: goal.map(|g| {
            terminal_values.iter().filter(|&&v| v >= g).count() as f64 / n as f64 * 100.0
        }),
        percentile_paths: Vec::new(), // simplified; full implementation stores yearly snapshots
    }
}
```

### 14.2 CLI

```rust
/// Forward Monte Carlo portfolio simulation
Simulate {
    /// Initial portfolio value
    #[arg(long)]
    value: f64,
    /// Expected annual return (e.g. 0.08 for 8%)
    #[arg(long, default_value = "0.08")]
    annual_return: f64,
    /// Annual volatility (e.g. 0.15 for 15%)
    #[arg(long, default_value = "0.15")]
    volatility: f64,
    /// Simulation horizon in years
    #[arg(long, default_value = "20")]
    years: u32,
    /// Annual withdrawal amount
    #[arg(long, default_value = "0")]
    withdrawal: f64,
    /// Goal amount for success probability
    #[arg(long)]
    goal: Option<f64>,
    /// Monte Carlo iterations
    #[arg(long, default_value = "10000")]
    iterations: u32,
    #[arg(long, value_enum, default_value = "table")]
    format: OutputFormat,
},
```

### 14.3 Testing

- **Unit:** Zero volatility, zero withdrawal -> terminal = initial * (1+r)^years. Zero return, positive withdrawal -> depletes at predictable rate.
- **Property:** `median_terminal > 0` for reasonable parameters. `p10 <= median <= p90`.

---

## G15 -- Performance Attribution (Brinson)

### 15.1 File: `src/analysis/attribution.rs`

```rust
/// Brinson-Fachler attribution: decompose portfolio return vs benchmark.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BrinsonAttribution {
    pub period: String,
    pub portfolio_return: f64,
    pub benchmark_return: f64,
    pub allocation_effect: f64,
    pub selection_effect: f64,
    pub interaction_effect: f64,
    pub total_active_return: f64,
    pub sector_details: Vec<SectorAttribution>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SectorAttribution {
    pub sector: String,
    pub portfolio_weight: f64,
    pub benchmark_weight: f64,
    pub portfolio_return: f64,
    pub benchmark_return: f64,
    pub allocation: f64,
    pub selection: f64,
    pub interaction: f64,
}

/// Compute single-period Brinson attribution.
///
/// `portfolio_sectors`: (sector, weight, return) for portfolio
/// `benchmark_sectors`: (sector, weight, return) for benchmark
pub fn brinson_attribution(
    portfolio_sectors: &[(&str, f64, f64)],
    benchmark_sectors: &[(&str, f64, f64)],
    period: &str,
) -> BrinsonAttribution {
    let bench_total_return: f64 = benchmark_sectors.iter()
        .map(|(_, w, r)| w * r)
        .sum();

    let mut details = Vec::new();
    let mut total_alloc = 0.0;
    let mut total_select = 0.0;
    let mut total_interact = 0.0;

    for (sector, pw, pr) in portfolio_sectors {
        let (_, bw, br) = benchmark_sectors.iter()
            .find(|(s, _, _)| s == sector)
            .unwrap_or(&(sector, &0.0, &0.0));

        let alloc = (pw - bw) * (br - bench_total_return);
        let select = bw * (pr - br);
        let interact = (pw - bw) * (pr - br);

        total_alloc += alloc;
        total_select += select;
        total_interact += interact;

        details.push(SectorAttribution {
            sector: sector.to_string(),
            portfolio_weight: *pw,
            benchmark_weight: *bw,
            portfolio_return: *pr,
            benchmark_return: *br,
            allocation: alloc,
            selection: select,
            interaction: interact,
        });
    }

    let port_return: f64 = portfolio_sectors.iter().map(|(_, w, r)| w * r).sum();

    BrinsonAttribution {
        period: period.to_string(),
        portfolio_return: port_return,
        benchmark_return: bench_total_return,
        allocation_effect: total_alloc,
        selection_effect: total_select,
        interaction_effect: total_interact,
        total_active_return: port_return - bench_total_return,
        sector_details: details,
    }
}
```

### 15.2 Testing

- **Unit:** When portfolio = benchmark, all effects = 0. Known textbook example from CFA curriculum.
- **Property:** `allocation + selection + interaction == total_active_return` (within epsilon).

---

## G16 -- IPS Document Generator

### 16.1 File: `src/portfolio/ips.rs` (new file, registered in `src/portfolio/mod.rs`)

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IpsInputs {
    pub client_name: String,
    pub date: String,
    pub objectives: String,
    pub risk_tolerance: String,           // "Conservative" / "Moderate" / "Aggressive"
    pub time_horizon: String,
    pub income_needs: String,
    pub liquidity_needs: String,
    pub tax_situation: String,
    pub legal_constraints: String,
    pub target_allocation: Vec<(String, f64)>,
    pub rebalancing_policy: String,
    pub review_frequency: String,
}

/// Generate a Markdown Investment Policy Statement from structured inputs.
pub fn generate_ips_markdown(inputs: &IpsInputs) -> String {
    let alloc_table: String = inputs.target_allocation.iter()
        .map(|(asset, w)| format!("| {} | {:.1}% |", asset, w * 100.0))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"# Investment Policy Statement

**Client:** {client}
**Date:** {date}

## 1. Investment Objectives
{objectives}

## 2. Risk Tolerance
{risk}

## 3. Time Horizon
{horizon}

## 4. Income & Liquidity Needs
- **Income:** {income}
- **Liquidity:** {liquidity}

## 5. Tax Considerations
{tax}

## 6. Legal & Regulatory Constraints
{legal}

## 7. Target Asset Allocation

| Asset Class | Weight |
|---|---|
{alloc_table}

## 8. Rebalancing Policy
{rebalance}

## 9. Review Schedule
{review}

---
*Generated by GhostAlpha on {date}*
"#,
        client = inputs.client_name,
        date = inputs.date,
        objectives = inputs.objectives,
        risk = inputs.risk_tolerance,
        horizon = inputs.time_horizon,
        income = inputs.income_needs,
        liquidity = inputs.liquidity_needs,
        tax = inputs.tax_situation,
        legal = inputs.legal_constraints,
        alloc_table = alloc_table,
        rebalance = inputs.rebalancing_policy,
        review = inputs.review_frequency,
    )
}
```

### 16.2 Web Route

`POST /portfolio/ips` -- accept form inputs, return rendered Markdown (with download link).

### 16.3 CLI

```rust
/// Generate Investment Policy Statement from JSON inputs
Ips {
    /// Path to JSON file with IPS inputs
    #[arg(long)]
    inputs: String,
    /// Output file path
    #[arg(long, default_value = "ips.md")]
    output: String,
},
```

### 16.4 Testing

- **Unit:** Output contains all input fields. Target weights appear in table.

---

## G17 -- Decision Audit Log

### 17.1 Database Schema -- Migration 019

```sql
CREATE TABLE IF NOT EXISTS decision_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    decision_date   TEXT NOT NULL,
    decision_type   TEXT NOT NULL,       -- 'SELL', 'BUY', 'HOLD', 'REBALANCE', 'REVIEW'
    ticker          TEXT,
    shares          REAL,
    price           REAL,
    rationale       TEXT NOT NULL,
    advisor         TEXT,               -- who made the decision
    linked_alert_id INTEGER,            -- FK to alert_log if triggered by alert
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_decision_log_date ON decision_log(decision_date);
CREATE INDEX IF NOT EXISTS idx_decision_log_ticker ON decision_log(ticker);
```

### 17.2 Repository Extensions

```rust
fn record_decision(&self, entry: &DecisionLogEntry) -> Result<(), StorageError>;
fn get_decisions(&self, limit: usize) -> Result<Vec<DecisionLogEntry>, StorageError>;
fn get_decisions_for_ticker(&self, ticker: &str, limit: usize) -> Result<Vec<DecisionLogEntry>, StorageError>;
```

### 17.3 Web Route

| Route | Description |
|---|---|
| `GET /decisions` | Page listing decision log entries |
| `POST /decisions/add` | Form to add new decision entry |

### 17.4 CLI

```rust
/// Record a manual decision in the audit log
Decision {
    #[command(subcommand)]
    action: DecisionAction,
},

#[derive(Subcommand)]
enum DecisionAction {
    /// Record a new decision
    Add {
        #[arg(long)]
        decision_type: String,
        #[arg(long)]
        ticker: Option<String>,
        #[arg(long)]
        rationale: String,
    },
    /// Show recent decisions
    List {
        #[arg(long, default_value = "20")]
        last: usize,
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
}
```

### 17.5 Testing

- **Unit:** Insert and retrieve round-trip.

---

## G18 -- Rebalancing Alert Engine

### 18.1 File: `src/alerts/rebalance.rs`

```rust
use crate::portfolio::types::Portfolio;
use crate::config::TargetWeight;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DriftAlert {
    pub ticker: String,
    pub current_weight: f64,
    pub target_weight: f64,
    pub drift_pct: f64,            // absolute drift
    pub direction: DriftDirection,
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum DriftDirection { Over, Under }

/// Check all positions against target weights and emit drift alerts.
pub fn check_weight_drift(
    portfolio: &Portfolio,
    targets: &[TargetWeight],
    threshold_pct: f64,
) -> Vec<DriftAlert> {
    let weights = portfolio.weights();
    let mut alerts = Vec::new();

    for target in targets {
        let current = weights.iter()
            .find(|(t, _)| *t == target.asset.as_str())
            .map(|(_, w)| *w * 100.0)
            .unwrap_or(0.0);
        let drift = current - target.weight;
        if drift.abs() > threshold_pct {
            alerts.push(DriftAlert {
                ticker: target.asset.clone(),
                current_weight: current,
                target_weight: target.weight,
                drift_pct: drift,
                direction: if drift > 0.0 { DriftDirection::Over } else { DriftDirection::Under },
            });
        }
    }

    alerts
}
```

### 18.2 Integration

Wire into the existing `AlertEngine` in `src/alerts/engine.rs`. After screening
runs, if a portfolio exists, call `check_weight_drift` and emit alerts through
existing channels (Telegram, Discord, Webhook).

New alert type variant added to `OutboundAlert`:

```rust
pub enum AlertType {
    // ... existing variants
    RebalanceDrift { ticker: String, current_pct: f64, target_pct: f64 },
}
```

### 18.3 Testing

- **Unit:** Portfolio at target weights -> no alerts. 6% drift with 5% threshold -> alert emitted. Missing position -> drift = full target weight.

---

## G19 -- Direct Indexing Simulator

### 19.1 File: `src/optimize/direct_index.rs`

```rust
/// Simulate replicating an index with individual stocks while
/// performing continuous tax-loss harvesting.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DirectIndexConfig {
    pub index_ticker: String,         // e.g. "SPY"
    pub exclude_tickers: Vec<String>, // e.g. ["AMZN"]
    pub initial_investment: f64,
    pub holding_period_years: u32,
    pub tax_rate: f64,
    pub min_loss_threshold: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DirectIndexResult {
    pub tracking_error_annualised: f64,
    pub tax_alpha_estimate: f64,       // estimated annual TLH benefit
    pub total_harvested_losses: f64,
    pub number_of_harvest_events: u32,
    pub vs_etf_fee_savings: f64,
    pub net_advantage: f64,            // tax alpha - management fee
}

/// Run the simulation over historical data.
pub fn simulate_direct_index(
    config: &DirectIndexConfig,
    index_holdings: &[(String, f64)],  // (ticker, weight)
    daily_prices: &std::collections::HashMap<String, Vec<(chrono::NaiveDate, f64)>>,
) -> Result<DirectIndexResult, crate::error::OptimizeError> {
    // For each holding day:
    //   1. Track individual lots
    //   2. Scan for losses > threshold
    //   3. "Harvest" by selling and buying substitute
    //   4. Track cumulative tax savings
    todo!()
}
```

### 19.2 Testing

- **Unit:** With no price changes, no harvesting occurs. With known drawdown, correct losses are harvested.

---

## G20 -- Quarterly Report Generator

### 20.1 File: `src/web/report_routes.rs`

Extends the existing `html_report.rs` infrastructure used for backtest reports.

```rust
use crate::portfolio::types::Portfolio;
use crate::analysis::attribution::BrinsonAttribution;

#[derive(Debug, Clone, serde::Serialize)]
pub struct QuarterlyReportData {
    pub quarter: String,                  // "2024 Q3"
    pub portfolio_return: f64,
    pub benchmark_return: f64,
    pub allocation_weights: Vec<(String, f64)>,
    pub hhi: f64,
    pub concentration_history: Vec<(String, f64)>,
    pub attribution: Option<BrinsonAttribution>,
    pub tax_summary: TaxQuarterSummary,
    pub rebalancing_actions: Vec<String>,
    pub risk_metrics: RiskMetrics,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TaxQuarterSummary {
    pub realised_gains_lt: f64,
    pub realised_gains_st: f64,
    pub harvested_losses: f64,
    pub net_tax_liability: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RiskMetrics {
    pub portfolio_volatility: f64,
    pub sharpe_ratio: f64,
    pub max_drawdown: f64,
    pub var_95: f64,
}

/// Generate full HTML quarterly report.
pub fn render_quarterly_report(data: &QuarterlyReportData) -> String {
    // Uses the same inline HTML template pattern as html_report.rs
    // Includes: performance table, allocation pie, concentration chart,
    // attribution waterfall, tax summary, risk gauges
    todo!()
}
```

### 20.2 Web Route

| Route | Description |
|---|---|
| `GET /reports` | Page listing available quarterly reports |
| `GET /reports/quarterly?q=2024Q3` | Render and return HTML report |
| `GET /reports/quarterly/download?q=2024Q3` | Download as self-contained HTML file |

### 20.3 CLI

```rust
/// Generate quarterly portfolio report
Report {
    /// Quarter identifier (e.g. "2024Q3")
    #[arg(long)]
    quarter: String,
    /// Output file path
    #[arg(long, default_value = "quarterly_report.html")]
    output: String,
    #[arg(long, value_enum, default_value = "html")]
    format: OutputFormat,
},
```

### 20.4 Testing

- **Unit:** Report render produces valid HTML (contains `<html>`, `<table>`, expected headings).
- **Integration:** Full pipeline: insert positions, run attribution, generate report, verify non-empty output.

---

## Migration Summary

| # | File | Tables Created |
|---|---|---|
| 012 | `012_portfolio.sql` | `tax_lots`, `disposals`, `portfolio_snapshots`, `cash_transactions` |
| 013 | `013_correlation_snapshots.sql` | `correlation_snapshots` |
| 014 | `014_fundamental_history.sql` | `fundamental_history` |
| 015 | `015_etf_snapshots.sql` | `etf_snapshots` |
| 016 | `016_selling_programmes.sql` | `selling_programmes`, `selling_executions` |
| 017 | `017_segment_financials.sql` | `segment_financials` |
| 018 | `018_institutional_ownership.sql` | `institutional_ownership` |
| 019 | `019_decision_log.sql` | `decision_log` |

Update `src/storage/migrations.rs`:

```rust
const MIGRATIONS: &[&str] = &[
    // ... existing 001-011 ...
    include_str!("../../migrations/012_portfolio.sql"),
    include_str!("../../migrations/013_correlation_snapshots.sql"),
    include_str!("../../migrations/014_fundamental_history.sql"),
    include_str!("../../migrations/015_etf_snapshots.sql"),
    include_str!("../../migrations/016_selling_programmes.sql"),
    include_str!("../../migrations/017_segment_financials.sql"),
    include_str!("../../migrations/018_institutional_ownership.sql"),
    include_str!("../../migrations/019_decision_log.sql"),
];
```

---

## MCP Tool Summary

| Tool Name | Gap | Input | Output |
|---|---|---|---|
| `portfolio_weights` | G1 | (none) | positions, weights, HHI |
| `correlation_matrix` | G3 | tickers, window | matrix, high pairs |
| `tax_impact` | G4 | ticker, shares, price | federal/state/NIIT tax |
| `harvest_scan` | G4 | min_loss | opportunities list |
| `run_dcf` | G5 | DcfInputs | fair value, EV, per-share |
| `reverse_dcf` | G5 | price, inputs | implied growth rate |
| `optimize_portfolio` | G7 | tickers, method | weights, return, vol |
| `etf_overlap` | G8 | etf, ticker | overlap weight |
| `forward_simulate` | G14 | config | terminal values, success rate |
| `factor_exposure` | G13 | ticker | betas, alpha, R-squared |

---

## CLI Command Summary

| Subcommand | Gap |
|---|---|
| `ghost-alpha portfolio import --file lots.csv` | G1 |
| `ghost-alpha portfolio positions` | G1 |
| `ghost-alpha portfolio lots AMZN` | G1 |
| `ghost-alpha portfolio weights` | G1 |
| `ghost-alpha portfolio snapshot` | G1 |
| `ghost-alpha asset-data --years 5` | G2 |
| `ghost-alpha correlation AMZN AGG GLD VNQ --window 756` | G3 |
| `ghost-alpha tax impact AMZN 100 --price market` | G4 |
| `ghost-alpha tax harvest --min-loss 1000` | G4 |
| `ghost-alpha tax project AMZN --annual-shares 100,200,300` | G4 |
| `ghost-alpha valuation dcf --inputs dcf.json` | G5 |
| `ghost-alpha valuation reverse-dcf AMZN --inputs base.json` | G5 |
| `ghost-alpha valuation scenarios --inputs scenarios.json` | G5 |
| `ghost-alpha optimize frontier VTI AGG GLD VNQ` | G7 |
| `ghost-alpha optimize max-sharpe VTI AGG GLD VNQ` | G7 |
| `ghost-alpha optimize risk-parity VTI AGG GLD VNQ` | G7 |
| `ghost-alpha optimize models` | G7 |
| `ghost-alpha etf info VTI` | G8 |
| `ghost-alpha etf overlap VTI AMZN` | G8 |
| `ghost-alpha etf compare VTI VOO IVV` | G8 |
| `ghost-alpha sell plan AMZN --annual-shares 100,200 --years 2` | G9 |
| `ghost-alpha sell show` | G9 |
| `ghost-alpha sell execute AMZN 50 185.50` | G9 |
| `ghost-alpha simulate --value 1000000 --years 20 --goal 3000000` | G14 |
| `ghost-alpha ips --inputs ips.json --output ips.md` | G16 |
| `ghost-alpha decision add --type SELL --ticker AMZN --rationale "..."` | G17 |
| `ghost-alpha decision list --last 20` | G17 |
| `ghost-alpha report --quarter 2024Q3 --output report.html` | G20 |

---

## Web Dashboard Summary

New pages added to navigation:

| Page | Route | Gap |
|---|---|---|
| Portfolio | `/portfolio` | G1 |
| Correlation | `/correlation` | G3 |
| Valuation | `/valuation` | G5 |
| Optimiser | `/optimize` | G7 |
| Reports | `/reports` | G20 |
| Decisions | `/decisions` | G17 |

New htmx partials:

| Partial | Route | Gap |
|---|---|---|
| Portfolio positions | `/partials/portfolio-positions` | G1 |
| Allocation pie chart | `/partials/portfolio-allocation` | G1 |
| Concentration history | `/partials/portfolio-concentration` | G1 |
| Correlation heatmap | `/partials/correlation-heatmap` | G3 |
| Rolling correlation | `/partials/rolling-correlation` | G3 |
| Valuation bands | `/partials/valuation-bands` | G6 |
| Margin trajectory | `/partials/margin-trajectory` | G6 |
| Scenario chart | `/partials/scenario-chart` | G5 |
| Efficient frontier | `/partials/efficient-frontier` | G7 |
| Model portfolios | `/partials/model-portfolios` | G7 |
| Weight drift gauge | `/partials/weight-drift` | G18 |

---

## Implementation Order

Following the sprint structure from the gap analysis, with dependency resolution:

### Sprint 1: Foundation (G1 + G2)
1. Create `src/portfolio/` module with types, import, snapshot
2. Migration 012 for tax lots, disposals, snapshots, cash
3. Repository trait extensions + `SqliteRepository` implementations
4. CLI: `portfolio` subcommands
5. Asset class ETF universe in `src/data/universes.rs`
6. Web: `/portfolio` page with positions table

### Sprint 2: Analysis Core (G3 + G6 + G5)
1. `src/analysis/correlation_matrix.rs` with `nalgebra`
2. Migration 013 for correlation snapshots
3. Migration 014 for fundamental history
4. `src/valuation/` module: DCF, reverse DCF, SOTP, scenarios
5. CLI: `correlation`, `valuation` subcommands
6. Web: `/correlation`, `/valuation` pages

### Sprint 3: Tax & Planning (G4 + G9)
1. `src/tax/` module: brackets, engine, HIFO, harvest, projection
2. Migration 016 for selling programmes
3. CLI: `tax`, `sell` subcommands
4. Wire selling programme to regime detection + exit signals

### Sprint 4: Optimisation (G7 + G8 + G14)
1. Add `nalgebra` + `osqp` dependencies
2. `src/optimize/` module: MVO, risk parity, frontier
3. `src/data/etf.rs` + migration 015
4. Forward Monte Carlo extension
5. CLI: `optimize`, `etf`, `simulate` subcommands
6. Web: `/optimize` page

### Sprint 5: Polish & Reporting (G16 + G17 + G18 + G20)
1. IPS generator
2. Migration 019 for decision log
3. Rebalance alert engine
4. Quarterly report generator
5. All MCP tool registrations

### Deferred (G10 + G11 + G12 + G13 + G15 + G19)
1. Options pricing + data (G10)
2. Segment financials (G11) + migration 017
3. Institutional ownership (G12) + migration 018
4. Factor decomposition (G13)
5. Brinson attribution (G15)
6. Direct indexing simulator (G19)
