# Ghost Alpha Diversification Engine — Gap Analysis & Implementation Plan

**Date:** 2026-03-20
**Source PRD:** `docs/ghost alpha diversification prd.md`
**Compared against:** Ghost Alpha codebase (54 source files, 24K LOC, 624 tests)

---

## Current Coverage Summary

| PRD Module | Current Coverage | Quality | Key Files |
|---|---|---|---|
| Tax engine (bracket stacking, NIIT, state) | 85% | Excellent | `src/tax/engine.rs`, `brackets.rs` |
| Tax lot types & tracking | 90% | Solid | `src/portfolio/types.rs` |
| HIFO lot selection | 100% | Complete | `src/tax/hifo.rs` |
| Multi-year projection | 70% | Single strategy only | `src/tax/projection.rs` |
| Selling programme | 75% | Schedule + pace rules | `src/portfolio/selling.rs`, `prepare.rs` |
| CSV import | 80% | Most brokerage formats | `src/portfolio/import.rs` |
| Market data | 70% | Yahoo, Finnhub, FRED | `src/data/` |
| Regime detection | 80% | SPY SMA + breadth | `src/analysis/regime.rs` |
| Web dashboard | 85% | 17 pages, SVG, htmx | `src/web/` |
| Optimization | 90% | MVO, Risk Parity, frontier | `src/optimize/` |
| Tax-loss harvesting | 80% | Scan + wash sale detection | `src/tax/harvest.rs` |

---

## Critical Gaps

### Gap 1: WA State Tax Brackets Are Outdated

**Severity:** HIGH — directly misstates tax liability

Current `brackets.rs` uses a flat 7% rate with $250K deduction. The 2025+ WA rules are:

- $278K standard deduction (not $250K)
- 7% on gains $278K–$1M
- 9.9% on gains above $1M

For a $1.5M liquidation in a single year: correct WA tax = $71,380 vs naive flat 7% = $87,500. That's a $16K error.

**Fix:** Refactor `StateTaxRate` to support tiered rates. Update WA entry in `state_tax_rate()`.

**Files:** `src/tax/brackets.rs`, `src/tax/engine.rs`

---

### Gap 2: No Multi-Year Scenario Comparison

**Severity:** HIGH — the PRD's core value proposition

`project_multi_year` projects a single strategy. The killer feature is `compare_scenarios`: run the same total gain through 1..N year spreads and find the optimal plateau.

```
1 year:  $68,543 tax (18.9% effective)
2 years: $56,880 tax (15.7%)
3 years: $54,479 tax (15.0%)  ← OPTIMAL (plateau)
4 years: $54,479 tax (15.0%)
5 years: $54,479 tax (15.0%)
```

The plateau occurs when annual gains drop below the WA $278K deduction threshold. This insight saves $14K+.

**Fix:** Add `compare_scenarios()` to `src/tax/projection.rs`. Add `sell-compare` CLI command.

**Output:** Table showing per-year tax, total tax, effective rate, and savings vs lump sum for each spread option.

---

### Gap 3: No Sell Priority Algorithm (Beyond HIFO)

**Severity:** MEDIUM-HIGH — HIFO minimizes gains but ignores tax-loss harvesting sequencing

The PRD defines a 3-tier priority:

1. **ST loss lots first** — harvest losses immediately (offset gains at 40.8% ordinary rate)
2. **ST gain lots** — check `days_to_ltcg`; if <60 days, hold for LTCG conversion
3. **LT gain lots ascending** — lowest gain% first (not highest cost basis)

HIFO is a subset of this (optimizes #3 only). A lot with -$12K unrealized loss should be sold in Week 1, regardless of cost basis rank.

**Fix:** Add `priority_select()` to `src/tax/hifo.rs` implementing the 3-tier algorithm.

**Files:** `src/tax/hifo.rs`, `src/portfolio/types.rs`

---

### Gap 4: No Geopolitical Risk Overlay

**Severity:** MEDIUM — makes the plan adaptive vs static

Current regime detection is market-structure-based (breadth + SMA). The PRD adds a geopolitical layer using market-priced proxies:

```
Geo risk = f(Oil_Brent, VIX, Gold_3m_change, DXY_3m_change)
```

This drives allocation shifts (oil crisis → +7% energy/defense, -4% EM, -3% growth).

**Fix:** Add `src/analysis/geopolitical.rs` with composite risk scorer. Extend `PaceCondition` with:

- `GeoRiskHigh` — accelerate selling if composite score >= 6
- `OilAbove(f64)` — oil-specific trigger (Brent > $120)
- `StockDropPercent(f64)` — emergency: stock drops > N% in a day

**Data sources:** Already available via FRED + Yahoo (VIX, oil, gold, DXY).

---

### Gap 5: No Allocation Builder with Brokerage Fund Mapping

**Severity:** MEDIUM — optimization is pointless without actionable fund tickers

MVO and Risk Parity output optimal weights, but nothing maps `{"us_broad_market": 15%}` to `{"FZROX": 15%, ER: 0.00%, TLH partner: FSKAX"}`.

The PRD has detailed fund maps for Fidelity, Schwab, and Vanguard with:

- Expense ratios
- TLH (tax-loss harvesting) partner funds
- Transferability flags (ZERO funds can't transfer in-kind)
- Minimum investment requirements

**Fix:** Add `src/portfolio/allocation.rs` with:

- Risk profile templates (aggressive, moderate, conservative)
- `FundMapper` selecting platform-specific funds
- TLH pair management

---

### Gap 6: No Concentration Decay Visualization

**Severity:** LOW-MEDIUM — behavioral finance shows this chart drives client action

The web dashboard shows HHI and position weights, but not the projected volatility reduction from the selling programme.

BNY Mellon data: 1 stock = 34% annual vol → 5 stocks = 21.4% → 20+ stocks = 15.8%.

**Fix:** Add endpoint/chart mapping quarterly selling schedule to projected portfolio volatility over time. Show "your risk drops 40% in Phase 1 alone."

**Files:** `src/web/dashboard_routes.rs`, `src/web/charts.rs`

---

## Specific Code Improvements

### 1. Update WA tax to 2-tier (`src/tax/brackets.rs:94`)

```rust
// Current: flat 7% above $250K
"WA" => Ok(StateTaxRate { rate: 0.07, annual_deduction: 250_000.0, ... })

// Target: 7% up to $1M, 9.9% above, with $278K deduction
// Requires extending StateTaxRate to support tiered rates or
// adding a secondary rate + tier threshold field
```

### 2. Add `compare_scenarios` (`src/tax/projection.rs`)

New function that calls `project_multi_year` in a loop for 1..max_years, returning the optimal spread with savings vs lump sum.

### 3. Add `days_to_long_term()` to TaxLot (`src/portfolio/types.rs`)

```rust
pub fn days_to_long_term(&self, as_of: NaiveDate) -> Option<i64> {
    if self.is_long_term(as_of) { return None; }
    Some(366 - self.holding_period_days(as_of))
}
```

Critical for the "hold 47 more days and save $8K" insight.

### 4. Extend PaceCondition (`src/portfolio/selling.rs`)

```rust
GeoRiskHigh,           // Accelerate if composite geo score >= 6
OilAbove(f64),         // Oil-specific trigger (Brent > $120)
StockDropPercent(f64), // Emergency: stock drops > N% in a day
```

### 5. Add `sell-compare` CLI command

```
ghost-alpha sell-compare --ticker AMZN --shares 1000 --cost-basis 95 \
  --price 210 --state WA --filing mfj --other-income 200000 --max-years 5
```

---

## What NOT to Implement from the PRD

1. **Python/FastAPI/PostgreSQL stack** — ignore. Rust single-binary is architecturally superior for this use case (2s lot analysis, no infra, embedded dashboard).

2. **Options overlay (collars, protective puts)** — defer. Requires options chain data ($29/mo Polygon), pricing models, broker-specific APIs. High complexity, low priority vs tax comparison.

3. **Multi-client advisor portal** — defer. Single-user CLI+dashboard is the right scope. Multi-tenancy is a 3-month project.

4. **Direct indexing / FidFolios** — omit. Managed product, not replicable in software.

---

## Implementation Order

| Priority | Feature | LOE | Impact | Files |
|---|---|---|---|---|
| P0 | Fix WA 2-tier brackets + $278K deduction | 2h | Correctness | `brackets.rs`, `engine.rs` |
| P0 | `compare_scenarios` + `sell-compare` CLI | 4h | Core value prop | `projection.rs`, `main.rs` |
| P1 | 3-tier sell priority algorithm | 3h | Tax optimization | `hifo.rs`, `types.rs` |
| P1 | `days_to_long_term()` on TaxLot | 30min | LTCG conversion insight | `types.rs` |
| P2 | Geopolitical risk scorer | 4h | Adaptive planning | new `geopolitical.rs` |
| P2 | Brokerage fund mapper (Fidelity first) | 4h | Actionable output | new `allocation.rs` |
| P3 | Concentration decay chart | 2h | Client persuasion | `dashboard_routes.rs`, `charts.rs` |
| P3 | Client-facing HTML report | 6h | Deliverable | new `report/diversification.rs` |

**Total estimated LOE:** ~26 hours across all priorities
