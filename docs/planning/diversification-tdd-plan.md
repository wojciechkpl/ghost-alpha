# Ghost Alpha Diversification Engine -- TDD Implementation Plan

**Date:** 2026-03-20
**Source:** `docs/planning/diversification-gap-analysis.md`
**Approach:** Strict Red-Green-Refactor TDD. Every feature starts with failing tests.

---

## Dependency Order

```
Item 1: Tiered StateTaxRate (P0)          -- foundational; engine.rs, brackets.rs
Item 2: days_to_long_term() on TaxLot (P1) -- tiny; types.rs only
Item 3: 3-Tier Sell Priority (P1)          -- depends on Item 2 (days_to_long_term)
Item 4: compare_scenarios (P0)             -- depends on Item 1 (correct WA tax)
Item 5: sell-compare CLI (P0)              -- depends on Item 4
Item 6: Geopolitical Risk Scorer (P2)      -- depends on macro_risk.rs patterns
Item 7: Brokerage Fund Mapper (P2)         -- independent
Item 8: Concentration Decay Chart (P3)     -- depends on Item 4 (scenario data)
```

---

## Item 1: Tiered State Tax Brackets (P0)

### Motivation

WA's capital gains tax is 7% on gains $278K--$1M and 9.9% above $1M (with $278K standard deduction). The current `StateTaxRate` only supports a flat rate, misstating tax by up to $16K on a $1.5M liquidation.

### Reuse Analysis

- **`bracket_tax()` in `engine.rs`** already implements marginal bracket stacking. State tiered rates are the same concept. We will reuse `bracket_tax()` directly for state tax computation rather than duplicating the logic.
- **`compute_lot_tax()`** currently does `(now_taxable - already_taxable) * state.rate` -- this single-rate formula must be replaced with a call to `bracket_tax()` using the state's bracket schedule.
- **`state_tax_rate()`** lookup stays as-is but returns richer data.

### Refactoring: Promote `bracket_tax` to `pub(crate)`

Currently `bracket_tax` is `fn` (private) in `engine.rs`. Both federal and state tax use the same bracket-stacking algorithm. Promote it to `pub(crate)` so the state tax path can reuse it.

### Types and Signatures

**File:** `src/tax/brackets.rs`

```rust
/// A tiered tax schedule: Vec of (upper_bound, rate), ascending.
/// Last entry must have upper_bound = f64::INFINITY.
/// Identical format to FederalCapGainsBrackets::long_term_brackets.
pub type TaxBracketSchedule = Vec<(f64, f64)>;

#[derive(Debug, Clone)]
pub struct StateTaxRate {
    pub state: String,
    /// Flat rate used for states with a single tier. Kept for backward compat
    /// with any code reading this field directly. For computation, use `brackets`.
    pub rate: f64,
    pub has_ltcg_preference: bool,
    /// Annual deduction applied before state cap-gains tax.
    pub annual_deduction: f64,
    /// Tiered bracket schedule applied to gains above the deduction.
    /// Empty vec = use flat `rate`. Non-empty = marginal brackets.
    pub brackets: TaxBracketSchedule,
}
```

**File:** `src/tax/engine.rs`

```rust
// Change visibility:
pub(crate) fn bracket_tax(gain: f64, brackets: &[(f64, f64)], base_income: f64) -> f64

// State tax section in compute_lot_tax() changes from:
//   (now_taxable - already_taxable) * state.rate
// to:
//   bracket_tax(marginal_gain, &state_brackets, already_taxable)
// where state_brackets = if state.brackets.is_empty() { flat } else { state.brackets }
```

### Test List

**File:** `src/tax/brackets.rs` -- inline `#[cfg(test)]`

| Test Name | Assertion |
|---|---|
| `test_wa_2025_deduction_is_278k` | `state_tax_rate("WA").unwrap().annual_deduction == 278_000.0` |
| `test_wa_2025_has_two_brackets` | `state_tax_rate("WA").unwrap().brackets.len() == 2` |
| `test_wa_bracket_rates` | First bracket rate = 0.07, second = 0.099 |
| `test_wa_bracket_thresholds` | First bracket upper = 722_000.0 ($1M - $278K deduction), second = INFINITY |
| `test_flat_rate_states_have_empty_brackets` | `state_tax_rate("CA").unwrap().brackets.is_empty()` |
| `test_zero_rate_states_have_empty_brackets` | `state_tax_rate("TX").unwrap().brackets.is_empty()` |

**File:** `src/tax/engine.rs` -- inline `#[cfg(test)]`

| Test Name | Assertion |
|---|---|
| `test_wa_tiered_under_deduction_no_tax` | $200K gain in WA => $0 state tax (under $278K deduction) |
| `test_wa_tiered_first_bracket_only` | $500K gain => taxable = $222K (500-278), all at 7% => $15,540 |
| `test_wa_tiered_crosses_into_second_bracket` | $1.5M gain => taxable $1,222K; first $722K at 7% = $50,540; remaining $500K at 9.9% = $49,500; total = $100,040 |
| `test_wa_tiered_marginal_with_ytd_gains` | YTD = $300K (above deduction), new gain $800K. Marginal: from $22K already taxable to $822K. First bracket covers $22K--$722K at 7% = $49,000; second bracket $722K--$822K at 9.9% = $9,900; total = $58,900 |
| `test_wa_tiered_all_in_second_bracket` | YTD = $800K, new gain $200K. Already taxable = $522K, now taxable = $722K+. Entire $200K marginal is partially in tier 1 and partially in tier 2. |
| `test_flat_state_unchanged` | CA $100K gain at 13.3% = $13,300 (same as before refactor) |
| `test_zero_state_unchanged` | TX gain = $0 state tax (same as before) |
| `test_bracket_tax_is_pub_crate` | (compile-time: import from `crate::tax::engine::bracket_tax` succeeds in another test module) |

### Implementation Notes

1. Add `brackets: TaxBracketSchedule` field to `StateTaxRate` with default `vec![]` for all flat-rate states.
2. Update WA entry in `state_tax_rate()`:
   ```rust
   "WA" => Ok(StateTaxRate {
       state: "WA".into(),
       rate: 0.07,  // kept for display/compat
       has_ltcg_preference: false,
       annual_deduction: 278_000.0,
       brackets: vec![
           (722_000.0, 0.07),    // $278K+$722K = $1M boundary
           (f64::INFINITY, 0.099),
       ],
   })
   ```
3. In `compute_lot_tax`, replace the state tax block:
   ```rust
   let state_tax = if gain <= 0.0 || (state.rate == 0.0 && state.brackets.is_empty()) {
       0.0
   } else {
       let ded = state.annual_deduction;
       let already_taxable = (state_gains_ytd - ded).max(0.0);
       let now_taxable = (state_gains_ytd + gain - ded).max(0.0);
       let marginal_gain = now_taxable - already_taxable;
       if state.brackets.is_empty() {
           marginal_gain * state.rate
       } else {
           bracket_tax(marginal_gain, &state.brackets, already_taxable)
       }
   };
   ```
4. Change `bracket_tax` from `fn` to `pub(crate) fn`.

### Existing Test Updates

- `test_wa_deduction_exempts_gains_under_250k` -- update expected deduction to $278K (gains under $278K exempt, not $250K). Rename to `test_wa_deduction_exempts_gains_under_278k`.
- `test_wa_deduction_taxes_excess_over_250k` -- update to $278K boundary and verify 7% rate on first tier.
- `test_wa_deduction_marginal_with_ytd_gains` -- update for $278K deduction.
- `test_wa_deduction_fully_consumed_ytd` -- verify tiered rates when deduction already consumed.

### Error Handling

No new error variants needed. Existing `TaxError::UnknownState` suffices.

### Migration Notes

None. No DB schema involved.

---

## Item 2: `days_to_long_term()` on TaxLot (P1)

### Motivation

The "hold 47 more days and save $8K" insight requires knowing exactly how many days until a short-term lot becomes long-term. This is a pure function on `TaxLot` with no dependencies.

### Reuse Analysis

- **`TaxLot::holding_period_days()`** and **`TaxLot::is_long_term()`** already exist. `days_to_long_term` composes them.

### Types and Signatures

**File:** `src/portfolio/types.rs`

```rust
impl TaxLot {
    /// Days remaining until this lot qualifies for long-term capital gains.
    /// Returns `None` if already long-term. Returns `Some(0)` if today is the
    /// crossover day (held exactly 366 days).
    pub fn days_to_long_term(&self, as_of: NaiveDate) -> Option<i64> {
        if self.is_long_term(as_of) {
            return None;
        }
        Some(366 - self.holding_period_days(as_of))
    }
}
```

### Test List

**File:** `src/portfolio/types.rs` -- inline `#[cfg(test)]`

| Test Name | Assertion |
|---|---|
| `test_days_to_long_term_already_lt` | Lot held 400 days => `None` |
| `test_days_to_long_term_exactly_366` | Lot held exactly 366 days => `None` (already LT since >365) |
| `test_days_to_long_term_day_365` | Lot held exactly 365 days => `Some(1)` (one more day) |
| `test_days_to_long_term_recent` | Lot held 100 days => `Some(266)` |
| `test_days_to_long_term_day_zero` | Lot acquired today => `Some(366)` |
| `test_days_to_long_term_47_days_left` | Lot held 319 days => `Some(47)` -- the "hold 47 more days" scenario |

### Implementation Notes

Trivial -- three lines of code. No other modules affected.

### Error Handling

None needed. Pure computation, no failure modes.

### Migration Notes

None.

---

## Item 3: 3-Tier Sell Priority Algorithm (P1)

### Motivation

HIFO minimizes gain amount but ignores loss harvesting sequencing and LTCG conversion timing. The 3-tier priority sells ST losses first (harvest at 40.8% ordinary offset rate), then ST gains with short LTCG horizon (hold for conversion), then LT gains ascending by gain% (lowest taxed first).

### Reuse Analysis

- **`hifo_select()`** -- stays as-is for backward compat. The new function `priority_select()` replaces it for the selling programme use case.
- **`TaxLot::is_long_term()`**, **`TaxLot::unrealised_gain()`**, **`TaxLot::days_to_long_term()`** (Item 2) -- all reused.
- **`scan_harvest_opportunities()`** in `harvest.rs` -- conceptually related but different concern (scanning vs ordering). No code reuse, but the priorities align.

### Types and Signatures

**File:** `src/tax/hifo.rs`

```rust
/// Sell priority tier for lot ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SellPriority {
    /// Tier 1: Short-term loss lots -- harvest immediately.
    StLoss,
    /// Tier 2: Long-term gain lots, ascending by gain%.
    LtGainAscending,
    /// Tier 3: Short-term gain lots with <60 days to LTCG -- hold if possible.
    StGainNearLtcg,
    /// Tier 4: Short-term gain lots with >=60 days to LTCG.
    StGainFar,
}

/// Select lots using the 3-tier priority algorithm.
///
/// Priority order:
/// 1. ST loss lots (harvest losses, offset gains at ordinary rate)
/// 2. LT gain lots ascending by gain% (lowest gain% = lowest tax)
/// 3. ST gain lots with >=60 days to LTCG (no conversion benefit)
/// 4. ST gain lots with <60 days to LTCG (skip -- hold for LTCG conversion)
///
/// Tier 4 lots are only used if tiers 1-3 cannot fill `shares_needed`.
///
/// `ltcg_hold_days` controls the "near LTCG" threshold (default: 60).
///
/// Returns `(lot_index, shares_from_lot)` pairs, same format as `hifo_select`.
pub fn priority_select(
    lots: &[TaxLot],
    shares_needed: f64,
    current_price: f64,
    as_of: NaiveDate,
    ltcg_hold_days: i64,
) -> Vec<(usize, f64)>
```

### Test List

**File:** `src/tax/hifo.rs` -- inline `#[cfg(test)]`

| Test Name | Assertion |
|---|---|
| `test_priority_st_loss_first` | Lot A: ST loss (-$5K), Lot B: LT gain (+$20K). Selling 50 shares takes from A first. |
| `test_priority_lt_gain_before_st_gain` | Lot A: LT gain 50%, Lot B: ST gain 30% (200 days to LTCG). Selling takes from A before B. |
| `test_priority_lt_gain_ascending_by_gain_pct` | Lot A: LT +10%, Lot B: LT +50%. Sells A first (lower gain%). |
| `test_priority_st_near_ltcg_last` | Lot A: ST gain, 30 days to LTCG. Lot B: ST gain, 200 days to LTCG. Lot C: LT gain. Order: C, B, A. |
| `test_priority_uses_near_ltcg_only_if_needed` | Only ST-near-LTCG lots available. If shares_needed > 0, they are used (not infinite deferral). |
| `test_priority_zero_shares` | `priority_select(lots, 0.0, ...)` returns empty vec. |
| `test_priority_skips_empty_lots` | Lot with `remaining_shares == 0.0` is skipped. |
| `test_priority_partial_lot_fill` | Need 30 shares from a 100-share lot => takes 30. |
| `test_priority_mixed_tiers_fill_order` | 4 lots, one per tier. Need shares spanning 3 tiers. Verify tier-1 exhausted first, then tier-2, then tier-3. |
| `test_priority_same_result_format_as_hifo` | Return type is `Vec<(usize, f64)>` with valid indices and positive share amounts. |
| `test_priority_custom_hold_threshold` | `ltcg_hold_days = 90` instead of 60. Lot 50 days from LTCG is now "near" and deferred. |
| `test_priority_loss_lot_gain_pct_negative` | Lot with cost=150, price=100 has gain%=-33%. Classified as StLoss. |

### Implementation Notes

```rust
pub fn priority_select(
    lots: &[TaxLot],
    shares_needed: f64,
    current_price: f64,
    as_of: NaiveDate,
    ltcg_hold_days: i64,
) -> Vec<(usize, f64)> {
    if shares_needed <= 0.0 {
        return Vec::new();
    }

    // Classify each lot into a priority tier + sort key
    let mut classified: Vec<(usize, SellPriority, f64)> = lots
        .iter()
        .enumerate()
        .filter(|(_, lot)| lot.remaining_shares > 0.0)
        .map(|(idx, lot)| {
            let gain_pct = (current_price - lot.cost_basis_per_share) / lot.cost_basis_per_share;
            let is_lt = lot.is_long_term(as_of);
            let is_loss = current_price < lot.cost_basis_per_share;
            let days_to_lt = lot.days_to_long_term(as_of).unwrap_or(0);

            let priority = if !is_lt && is_loss {
                SellPriority::StLoss
            } else if is_lt && !is_loss {
                SellPriority::LtGainAscending
            } else if !is_lt && days_to_lt <= ltcg_hold_days {
                SellPriority::StGainNearLtcg
            } else {
                SellPriority::StGainFar
            };
            (idx, priority, gain_pct)
        })
        .collect();

    // Sort: by priority tier (ascending), then by gain% within tier
    classified.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
    });

    // Fill shares
    let mut remaining = shares_needed;
    let mut selections = Vec::new();
    for (idx, _, _) in &classified {
        if remaining <= 0.0 { break; }
        let available = lots[*idx].remaining_shares;
        let take = remaining.min(available);
        selections.push((*idx, take));
        remaining -= take;
    }
    selections
}
```

### Refactoring Opportunity

- `project_multi_year()` currently hardcodes `hifo_select`. Add a `LotSelector` enum or parameter:
  ```rust
  pub enum LotSelectionStrategy {
      Hifo,
      Priority { current_price: f64, ltcg_hold_days: i64 },
  }
  ```
  This allows `project_multi_year` to use either algorithm. **Do this in a follow-up refactor** after both algorithms are tested independently. For now, `priority_select` is called directly by new code paths (`compare_scenarios`, `sell-compare`) while `project_multi_year` keeps using `hifo_select` for backward compat.

### Error Handling

No new error variants. The function is infallible (returns empty vec on edge cases).

### Migration Notes

None.

---

## Item 4: Multi-Year Scenario Comparison (P0)

### Motivation

The core value proposition: compare spreading the same total gain across 1..N years to find the optimal plateau where WA's $278K deduction fully shelters annual gains. This can save $14K+ in tax.

### Reuse Analysis

- **`project_multi_year()`** -- reused directly. `compare_scenarios` calls it N times with different `annual_shares` vectors.
- **`compute_annual_targets()`** in `prepare.rs` -- reused for even distribution. Currently `fn` (private). Promote to `pub(crate)`.
- **`compute_summary()`** in `prepare.rs` -- reused for per-scenario summary. Promote to `pub(crate)`.
- **`SellPlanSummary`** -- reused as-is for each scenario row.

### Types and Signatures

**File:** `src/tax/projection.rs`

```rust
/// A single scenario in a multi-year comparison.
#[derive(Debug, Clone, Serialize)]
pub struct ScenarioResult {
    /// Number of years the gain is spread across.
    pub spread_years: u32,
    /// Shares sold per year (even distribution).
    pub annual_shares: f64,
    /// Year-by-year projections.
    pub projections: Vec<YearProjection>,
    /// Aggregate tax summary.
    pub total_proceeds: f64,
    pub total_gain: f64,
    pub total_tax: f64,
    pub effective_rate: f64,
    /// Savings compared to lump-sum (spread_years=1).
    pub savings_vs_lump: f64,
    /// True if this is the optimal (lowest tax) scenario.
    pub is_optimal: bool,
}

/// Compare spreading total_shares across 1..=max_years.
///
/// For each spread, shares are distributed evenly. Returns all scenarios
/// sorted by spread_years ascending, with `is_optimal` set on the best one.
///
/// `plateau_threshold` (e.g., 0.005 = 0.5%) controls when savings are
/// considered to have plateaued. When the marginal savings from adding
/// one more year drops below this fraction of the lump-sum tax, we mark
/// the prior year as optimal.
pub fn compare_scenarios(
    lots: &[TaxLot],
    total_shares: f64,
    max_years: u32,
    assumed_price_growth: f64,
    current_price: f64,
    other_annual_income: f64,
    brackets: &FederalCapGainsBrackets,
    state: &StateTaxRate,
) -> Vec<ScenarioResult>
```

### Test List

**File:** `src/tax/projection.rs` -- inline `#[cfg(test)]`

| Test Name | Assertion |
|---|---|
| `test_compare_single_year_matches_project` | `compare_scenarios(max_years=1)` produces one result whose total_tax matches `project_multi_year` for the same inputs. |
| `test_compare_more_years_lower_or_equal_tax` | For a WA taxpayer with $1M gain: tax(2yr) <= tax(1yr). Monotonically non-increasing. |
| `test_compare_savings_vs_lump` | `scenarios[0].savings_vs_lump == 0.0` (lump sum is the baseline). `scenarios[1].savings_vs_lump >= 0.0`. |
| `test_compare_optimal_flag_set` | Exactly one scenario has `is_optimal == true`. |
| `test_compare_plateau_detection` | With $600K gain in WA: 1yr has $22K+ WA tax. At 3 years ($200K/yr < $278K deduction), WA tax = $0. Years 3, 4, 5 should all have same total tax. Optimal = 3. |
| `test_compare_no_state_tax_plateau_still_works` | TX (no state tax) -- plateau is driven by federal bracket stacking. Still finds an optimal year count. |
| `test_compare_shares_sum_to_total` | For every scenario, sum of `annual_shares * spread_years` = `total_shares` (within epsilon). |
| `test_compare_empty_lots_returns_empty` | Empty lots vec => empty results. |
| `test_compare_max_years_capped` | `max_years=10` produces exactly 10 scenarios. |
| `test_compare_with_price_growth` | With 10% growth, later-year scenarios have higher proceeds but same gain base. Verify proceeds increase with spread years. |
| `test_compare_effective_rate_bounded` | All scenarios have `effective_rate` in [0.0, 1.0). |

### Implementation Notes

```rust
pub fn compare_scenarios(
    lots: &[TaxLot],
    total_shares: f64,
    max_years: u32,
    assumed_price_growth: f64,
    current_price: f64,
    other_annual_income: f64,
    brackets: &FederalCapGainsBrackets,
    state: &StateTaxRate,
) -> Vec<ScenarioResult> {
    if lots.is_empty() || total_shares <= 0.0 || max_years == 0 {
        return Vec::new();
    }

    let current_year = chrono::Utc::now().naive_utc().date().year() as u32;
    let mut scenarios = Vec::with_capacity(max_years as usize);

    for years in 1..=max_years {
        let per_year = total_shares / years as f64;
        let annual_shares: Vec<(u32, f64)> = (0..years)
            .map(|i| (current_year + i, per_year))
            .collect();

        let projections = project_multi_year(
            lots,
            &annual_shares,
            assumed_price_growth,
            current_price,
            other_annual_income,
            brackets,
            state,
        );

        let total_proceeds: f64 = projections.iter().map(|p| p.proceeds).sum();
        let total_gain: f64 = projections.iter().map(|p| p.realised_gain).sum();
        let total_tax: f64 = projections.iter().map(|p| p.total_tax).sum();

        scenarios.push(ScenarioResult {
            spread_years: years,
            annual_shares: per_year,
            projections,
            total_proceeds,
            total_gain,
            total_tax,
            effective_rate: if total_gain > 0.0 { total_tax / total_gain } else { 0.0 },
            savings_vs_lump: 0.0, // filled after loop
            is_optimal: false,     // filled after loop
        });
    }

    // Compute savings_vs_lump
    let lump_tax = scenarios.first().map(|s| s.total_tax).unwrap_or(0.0);
    let mut min_tax = f64::MAX;
    let mut optimal_idx = 0;

    for (i, s) in scenarios.iter_mut().enumerate() {
        s.savings_vs_lump = lump_tax - s.total_tax;
        if s.total_tax < min_tax {
            min_tax = s.total_tax;
            optimal_idx = i;
        }
    }
    if !scenarios.is_empty() {
        scenarios[optimal_idx].is_optimal = true;
    }

    scenarios
}
```

### Refactoring: Promote helpers in `prepare.rs`

- `compute_summary()` -> `pub(crate)`. Used by both `prepare_selling_programme` and `compare_scenarios`.
- `compute_annual_targets()` -> `pub(crate)`. Potentially useful but `compare_scenarios` only needs even distribution (trivial inline).

### Error Handling

No new error variants. The function returns an empty vec on invalid inputs rather than erroring. This matches `project_multi_year`'s convention.

### Migration Notes

None.

---

## Item 5: `sell-compare` CLI Command (P0)

### Motivation

Expose the scenario comparison engine as a CLI command so users can run `ghost-alpha sell-compare` and see the optimal year spread.

### Reuse Analysis

- **`compare_scenarios()`** (Item 4) -- the core engine.
- **`format_sell_plan()`** in `prepare.rs` -- pattern to follow for formatting. A new `format_comparison_table()` is needed.
- **`state_tax_rate()`**, **`FederalCapGainsBrackets`** -- reused for input resolution.
- **Clap CLI patterns** -- follow existing command registration in `main.rs`.

### Types and Signatures

**File:** `src/tax/projection.rs`

```rust
/// Format the scenario comparison as a human-readable table.
pub fn format_comparison_table(scenarios: &[ScenarioResult], state: &str) -> String
```

**File:** `src/main.rs` (or wherever CLI commands are registered)

```rust
/// CLI subcommand struct:
#[derive(clap::Args)]
pub struct SellCompareArgs {
    #[arg(long)]
    ticker: String,
    #[arg(long)]
    shares: f64,
    #[arg(long, alias = "cost-basis")]
    cost_basis: f64,
    #[arg(long)]
    price: f64,
    #[arg(long)]
    state: String,
    #[arg(long, default_value = "single")]
    filing: String,
    #[arg(long, alias = "other-income", default_value_t = 0.0)]
    other_income: f64,
    #[arg(long, alias = "max-years", default_value_t = 5)]
    max_years: u32,
    #[arg(long, alias = "price-growth", default_value_t = 0.0)]
    price_growth: f64,
    #[arg(long, alias = "acquired-date")]
    acquired_date: Option<String>,
}
```

### Test List

**File:** `src/tax/projection.rs` -- inline `#[cfg(test)]`

| Test Name | Assertion |
|---|---|
| `test_format_comparison_table_header` | Output contains "Years", "Tax", "Eff %", "Savings" columns. |
| `test_format_comparison_table_marks_optimal` | Output contains a marker (e.g., "<<<" or "OPTIMAL") on the best row. |
| `test_format_comparison_table_savings_column` | First row (1 year) shows $0 savings. Subsequent rows show positive savings. |
| `test_format_comparison_table_empty_input` | Empty scenarios => returns a "no data" message, not a panic. |

**File:** Integration test or `src/main.rs` `#[cfg(test)]`

| Test Name | Assertion |
|---|---|
| `test_sell_compare_cli_parse` | `SellCompareArgs` parses from: `sell-compare --ticker AMZN --shares 1000 --cost-basis 95 --price 210 --state CA --filing single --other-income 150000 --max-years 5` |

### Implementation Notes

The CLI handler constructs a single `TaxLot` from the args (like `prepare_selling_programme` does), resolves brackets and state, calls `compare_scenarios`, formats the output table, and prints.

```
$ ghost-alpha sell-compare --ticker AMZN --shares 1000 --cost-basis 95 --price 210 --state WA

  AMZN -- Multi-year scenario comparison (State: WA, Filing: MFJ)
  ──────────────────────────────────────────────────────────
  Years    Shares/yr     Total Tax      Eff %     Savings
  ──────────────────────────────────────────────────────────
      1       1,000       $68,543      18.9%          $0
      2         500       $56,880      15.7%     $11,663
      3         334       $54,479      15.0%     $14,064  <<< OPTIMAL
      4         250       $54,479      15.0%     $14,064
      5         200       $54,479      15.0%     $14,064
  ──────────────────────────────────────────────────────────
  Spreading over 3 years saves $14,064 vs lump sum.
```

### Error Handling

Reuse existing patterns:
- `TaxError::UnknownState` for bad `--state`
- String error for bad `--filing` value
- Clap handles missing required args

### Migration Notes

None.

---

## Item 6: Geopolitical Risk Scorer (P2)

### Motivation

Market regime detection is currently structural (SMA + breadth). Adding a geopolitical layer using market-priced proxies (oil, VIX, gold, DXY) makes selling pace adaptive to macro shocks.

### Reuse Analysis

- **`MacroRiskLevel`** and **`compute_risk_level()`** in `macro_risk.rs` -- pattern to follow. The geo scorer produces a similar enum and integrates with the macro risk framework.
- **`PaceCondition`** in `selling.rs` -- extended with new variants.
- **`apply_pace_rules()`** -- already handles arbitrary PaceConditions via pattern matching. Just add new arms.
- **Data providers** -- VIX, oil, gold, DXY already fetchable via Yahoo/FRED providers.

### Types and Signatures

**File:** `src/analysis/geopolitical.rs` (NEW)

```rust
use serde::{Deserialize, Serialize};

/// Geopolitical risk inputs -- all market-priced, no subjective scoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoRiskInputs {
    /// Brent crude oil price ($/barrel).
    pub oil_brent: f64,
    /// CBOE VIX level.
    pub vix: f64,
    /// Gold 3-month price change (percentage, e.g., 0.08 = 8%).
    pub gold_3m_change: f64,
    /// US Dollar Index 3-month change (percentage).
    pub dxy_3m_change: f64,
}

/// Composite geopolitical risk score (0--10 scale).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeoRiskScore {
    pub composite: f64,
    pub oil_component: f64,
    pub vix_component: f64,
    pub gold_component: f64,
    pub dxy_component: f64,
    pub level: GeoRiskLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GeoRiskLevel {
    Low,       // 0-3
    Moderate,  // 4-5
    High,      // 6-7
    Extreme,   // 8-10
}

/// Compute composite geopolitical risk score from market inputs.
pub fn compute_geo_risk(inputs: &GeoRiskInputs) -> GeoRiskScore

/// Suggested allocation adjustments given a geo risk level.
/// Returns (sector, weight_delta) pairs, e.g., ("energy", 0.07) means +7%.
pub fn geo_allocation_adjustments(level: &GeoRiskLevel) -> Vec<(&'static str, f64)>
```

**File:** `src/portfolio/selling.rs` -- extend `PaceCondition`

```rust
pub enum PaceCondition {
    Below200Sma,
    BearRegime,
    Stage2Uptrend,
    PriceAbove(f64),
    // New:
    GeoRiskHigh,              // Accelerate if composite geo score >= 6
    OilAbove(f64),            // Oil-specific trigger
    StockDropPercent(f64),    // Emergency: stock drops > N% in a day
}
```

### Test List

**File:** `src/analysis/geopolitical.rs` -- inline `#[cfg(test)]`

| Test Name | Assertion |
|---|---|
| `test_calm_inputs_low_risk` | Oil=$75, VIX=15, gold_3m=2%, dxy_3m=1% => `GeoRiskLevel::Low` |
| `test_elevated_oil_moderate_risk` | Oil=$100, VIX=22, gold_3m=5%, dxy_3m=3% => `GeoRiskLevel::Moderate` |
| `test_crisis_inputs_high_risk` | Oil=$130, VIX=32, gold_3m=15%, dxy_3m=-5% => `GeoRiskLevel::High` |
| `test_extreme_inputs_extreme_risk` | Oil=$150, VIX=45, gold_3m=25%, dxy_3m=-10% => `GeoRiskLevel::Extreme` |
| `test_composite_score_bounded_0_10` | Any input => `composite` in [0.0, 10.0] |
| `test_oil_component_scaled` | Oil=$60 => oil_component ~0. Oil=$150 => oil_component ~2.5 |
| `test_vix_component_scaled` | VIX=12 => ~0. VIX=45 => ~2.5 |
| `test_gold_flight_to_safety` | gold_3m=20% => high gold_component (flight to safety signal) |
| `test_dxy_decline_adds_risk` | dxy_3m=-8% => positive dxy_component (dollar weakness = risk) |
| `test_geo_risk_level_ordering` | `Low < Moderate < High < Extreme` |
| `test_allocation_adjustments_high` | `GeoRiskLevel::High` => contains ("energy", positive), ("em", negative) |
| `test_allocation_adjustments_low` | `GeoRiskLevel::Low` => empty or all-zero adjustments |

**File:** `src/portfolio/selling.rs` -- inline `#[cfg(test)]`

| Test Name | Assertion |
|---|---|
| `test_apply_pace_geo_risk_high` | PaceCondition::GeoRiskHigh with geo_score >= 6 => multiplier applied. (Requires extending `apply_pace_rules` signature.) |
| `test_apply_pace_oil_above` | PaceCondition::OilAbove(120.0) with oil=$130 => multiplier applied. |
| `test_apply_pace_stock_drop` | PaceCondition::StockDropPercent(5.0) with 7% drop => multiplier applied. |
| `test_apply_pace_geo_no_match` | PaceCondition::GeoRiskHigh with geo_score = 3 => no match, multiplier = 1.0. |

### Implementation Notes

**Scoring formula** (each component 0--2.5, total 0--10):
```rust
fn compute_geo_risk(inputs: &GeoRiskInputs) -> GeoRiskScore {
    // Oil: linear ramp $80-$150 => 0-2.5
    let oil_c = ((inputs.oil_brent - 80.0) / 70.0).clamp(0.0, 1.0) * 2.5;
    // VIX: linear ramp 20-45 => 0-2.5
    let vix_c = ((inputs.vix - 20.0) / 25.0).clamp(0.0, 1.0) * 2.5;
    // Gold 3m change: ramp 5%-25% => 0-2.5 (flight to safety)
    let gold_c = ((inputs.gold_3m_change - 0.05) / 0.20).clamp(0.0, 1.0) * 2.5;
    // DXY 3m decline: ramp 0% to -10% => 0-2.5
    let dxy_c = ((-inputs.dxy_3m_change) / 0.10).clamp(0.0, 1.0) * 2.5;

    let composite = oil_c + vix_c + gold_c + dxy_c;
    let level = match composite as u32 {
        0..=3 => GeoRiskLevel::Low,
        4..=5 => GeoRiskLevel::Moderate,
        6..=7 => GeoRiskLevel::High,
        _ => GeoRiskLevel::Extreme,
    };
    GeoRiskScore { composite, oil_component: oil_c, vix_component: vix_c,
                   gold_component: gold_c, dxy_component: dxy_c, level }
}
```

**Extending `apply_pace_rules`:** The current signature is:
```rust
pub fn apply_pace_rules(&self, regime: Option<&str>, current_price: f64, is_above_200sma: bool) -> f64
```

Adding geo risk requires more context. Introduce a `MarketContext` struct:

```rust
/// All market conditions needed for pace rule evaluation.
#[derive(Debug, Clone, Default)]
pub struct MarketContext {
    pub regime: Option<String>,
    pub current_price: f64,
    pub is_above_200sma: bool,
    // New fields:
    pub geo_risk_score: Option<f64>,
    pub oil_price: Option<f64>,
    pub daily_return_pct: Option<f64>,
}
```

Refactor `apply_pace_rules` to take `&MarketContext` instead of individual parameters. This is a **breaking change** to the method signature but keeps the logic clean and extensible.

### Module Registration

**File:** `src/analysis/mod.rs` -- add `pub mod geopolitical;`

### Error Handling

No new error variants. The scorer is a pure function. Invalid/missing data is handled by the caller (skip geo assessment if data unavailable).

### Migration Notes

None -- no DB schema involved. The geo risk score is computed on the fly, not persisted.

---

## Item 7: Brokerage Fund Mapper (P2)

### Motivation

MVO/Risk Parity output optimal weights by asset class, but nothing maps abstract allocations to actionable fund tickers. A $15M portfolio getting `{"us_broad_market": 15%}` needs to know that means `FZROX` at Fidelity with 0.00% ER and `FSKAX` as the TLH partner.

### Reuse Analysis

- **`OptimalPortfolio`** from `optimize/mvo.rs` -- the output weights are the input to the fund mapper.
- **`HarvestOpportunity`** patterns from `harvest.rs` -- TLH partner concept.

### Types and Signatures

**File:** `src/portfolio/allocation.rs` (NEW)

```rust
use serde::{Deserialize, Serialize};

// ─── Asset Classes ──────────────────────────────────────────────────────────

/// Canonical asset class identifiers used across the system.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetClass {
    UsBroadMarket,
    UsLargeCap,
    UsMidCap,
    UsSmallCap,
    IntlDeveloped,
    IntlEmerging,
    UsBonds,
    IntlBonds,
    Tips,
    RealEstate,
    Commodities,
}

// ─── Risk Profiles ──────────────────────────────────────────────────────────

/// A named target allocation across asset classes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskProfile {
    pub name: String,
    pub description: String,
    /// Target weights by asset class. Must sum to ~1.0.
    pub allocations: Vec<(AssetClass, f64)>,
}

/// Built-in risk profiles from the PRD.
pub fn risk_profiles() -> Vec<RiskProfile>

// ─── Fund Catalog ───────────────────────────────────────────────────────────

/// A specific fund at a brokerage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FundEntry {
    pub ticker: String,
    pub name: String,
    pub asset_class: AssetClass,
    pub expense_ratio: f64,
    /// Tax-loss harvesting partner ticker (substantially similar fund).
    pub tlh_partner: Option<String>,
    /// Whether the fund can be transferred in-kind to another brokerage.
    pub transferable: bool,
    /// Minimum investment in dollars (0.0 = no minimum).
    pub minimum_investment: f64,
}

/// A brokerage's fund catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrokerageCatalog {
    pub brokerage: String,
    pub funds: Vec<FundEntry>,
}

/// Brokerage identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Brokerage {
    Fidelity,
    Schwab,
    Vanguard,
}

/// Get the fund catalog for a brokerage.
pub fn brokerage_catalog(brokerage: &Brokerage) -> BrokerageCatalog

// ─── Fund Mapping ───────────────────────────────────────────────────────────

/// A concrete allocation: asset class mapped to a specific fund with dollar amount.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcreteAllocation {
    pub asset_class: AssetClass,
    pub fund: FundEntry,
    pub target_weight: f64,
    pub dollar_amount: f64,
}

/// Map abstract asset class weights to concrete fund picks for a brokerage.
///
/// For each asset class in `weights`, selects the fund from the brokerage catalog
/// with the lowest expense ratio. Skips asset classes with no matching fund.
pub fn map_to_funds(
    weights: &[(AssetClass, f64)],
    brokerage: &Brokerage,
    total_portfolio_value: f64,
) -> Vec<ConcreteAllocation>

/// Compute the weighted average expense ratio for a set of concrete allocations.
pub fn blended_expense_ratio(allocations: &[ConcreteAllocation]) -> f64
```

### Test List

**File:** `src/portfolio/allocation.rs` -- inline `#[cfg(test)]`

| Test Name | Assertion |
|---|---|
| `test_risk_profiles_weights_sum_to_one` | All built-in profiles' allocations sum to 1.0 (within 0.01). |
| `test_risk_profiles_have_names` | At least 3 profiles (aggressive, moderate, conservative). |
| `test_fidelity_catalog_has_us_broad` | Fidelity catalog has a fund for `AssetClass::UsBroadMarket`. |
| `test_fidelity_zero_funds_not_transferable` | Fidelity ZERO funds (FZROX, FZILX) have `transferable == false`. |
| `test_fidelity_zero_funds_zero_er` | FZROX has `expense_ratio == 0.0`. |
| `test_schwab_catalog_has_us_broad` | Schwab catalog has a fund for `AssetClass::UsBroadMarket`. |
| `test_vanguard_catalog_has_us_broad` | Vanguard catalog has a fund for `AssetClass::UsBroadMarket`. |
| `test_map_to_funds_basic` | Given 3 asset classes and Fidelity, returns 3 ConcreteAllocations with correct dollar amounts. |
| `test_map_to_funds_selects_lowest_er` | If catalog has FSKAX (0.015%) and FZROX (0.00%) for UsBroadMarket, selects FZROX. |
| `test_map_to_funds_skips_unmapped_class` | An asset class not in the catalog is omitted from results (not an error). |
| `test_map_to_funds_dollar_amounts` | $1M portfolio at 15% weight => $150K allocation. |
| `test_blended_expense_ratio` | 50% FZROX (0.00%) + 50% FXAIX (0.015%) => 0.0075%. |
| `test_blended_expense_ratio_empty` | Empty allocations => 0.0. |
| `test_tlh_partner_exists` | FZROX's TLH partner is FSKAX (or vice versa). |
| `test_all_funds_have_asset_class` | Every fund in every catalog has a valid AssetClass. |
| `test_map_preserves_weight_sum` | Sum of output weights equals sum of input weights. |

### Implementation Notes

Fund catalogs are hardcoded static data (not DB-stored). This matches the existing pattern of `state_tax_rate()` using a match statement. Future extensibility: load from TOML config.

Fidelity fund map (from PRD):
```
UsBroadMarket: FZROX (ER=0.00%, TLH=FSKAX, transferable=false)
IntlDeveloped: FZILX (ER=0.00%, TLH=FTIHX, transferable=false)
UsBonds: FXNAX (ER=0.025%)
UsLargeCap: FXAIX (ER=0.015%, TLH=FNILX)
...
```

### Module Registration

**File:** `src/portfolio/mod.rs` -- add `pub mod allocation;`

### Error Handling

No new error variants. Unknown brokerage returns an empty catalog. Missing asset class mappings are silently skipped (the caller can check if output covers all input classes).

### Migration Notes

None. Static data, no persistence.

---

## Item 8: Concentration Decay Chart (P3)

### Motivation

Show how portfolio volatility decreases over the selling programme timeline. BNY Mellon data: 1 stock = 34% vol -> 5 stocks = 21.4% -> 20+ = 15.8%. This visual drives client action more than raw numbers.

### Reuse Analysis

- **`Portfolio::hhi()`** -- existing concentration metric. The chart shows HHI declining over quarters.
- **`SellingProgramme::schedule`** -- quarterly share targets drive the simulation.
- **`score_bar_svg()`** / **`regime_timeline_svg()`** in `charts.rs` -- patterns for SVG generation.
- **`compare_scenarios()`** (Item 4) -- provides the year-by-year data. The chart adds quarterly granularity.

### Types and Signatures

**File:** `src/web/charts.rs`

```rust
/// A single point on the concentration decay curve.
#[derive(Debug, Clone, Serialize)]
pub struct ConcentrationPoint {
    pub quarter_label: String,  // e.g., "2025 Q1"
    pub concentrated_weight: f64,  // weight of the concentrated position
    pub hhi: f64,
    pub estimated_annual_vol: f64,  // interpolated from BNY Mellon curve
    pub shares_remaining: f64,
}

/// Compute the projected concentration decay over a selling programme.
///
/// Given a starting portfolio, the concentrated ticker, and a selling
/// programme, simulate quarter-by-quarter weight reduction and map
/// to estimated volatility using the diversification benefit curve.
pub fn concentration_decay_points(
    starting_weight: f64,
    starting_shares: f64,
    programme: &SellingProgramme,
    current_price: f64,
    other_portfolio_value: f64,
) -> Vec<ConcentrationPoint>

/// Render a concentration decay chart as an inline SVG.
pub fn concentration_decay_svg(
    points: &[ConcentrationPoint],
    width: u32,
    height: u32,
) -> String
```

**File:** `src/web/dashboard_routes.rs`

```rust
/// GET /dashboard/concentration-decay?ticker=AMZN
pub async fn concentration_decay_page(
    State(state): State<AppState>,
    Query(params): Query<TickerParam>,
) -> Html<String>
```

### Test List

**File:** `src/web/charts.rs` -- inline `#[cfg(test)]`

| Test Name | Assertion |
|---|---|
| `test_concentration_decay_basic` | 4-quarter programme: first point has starting weight, last has lower weight. |
| `test_concentration_decay_vol_decreases` | estimated_annual_vol is monotonically non-increasing across points. |
| `test_concentration_decay_shares_decrease` | shares_remaining monotonically decreasing. |
| `test_concentration_decay_hhi_decreases` | HHI decreases as concentration drops. |
| `test_concentration_decay_empty_programme` | Empty schedule => single point (starting state). |
| `test_concentration_decay_100pct_start` | 100% concentrated => starting vol ~ 34%. |
| `test_concentration_decay_zero_weight` | After selling everything, weight = 0, vol approaches market vol (~15.8%). |
| `test_vol_interpolation_known_points` | 1 stock (100% weight) -> 34% vol. 5-stock-equivalent (~20% weight) -> ~21.4% vol. |

**File:** `src/web/charts.rs` -- inline `#[cfg(test)]` (SVG rendering)

| Test Name | Assertion |
|---|---|
| `test_concentration_decay_svg_valid` | Output starts with `<svg` and contains `</svg>`. |
| `test_concentration_decay_svg_has_data_points` | SVG contains `<circle` or `<line` elements for each data point. |
| `test_concentration_decay_svg_has_labels` | SVG contains quarter labels and vol% text. |
| `test_concentration_decay_svg_empty` | Empty points => valid SVG with "no data" message. |

### Implementation Notes

**Volatility interpolation** -- use the BNY Mellon empirical curve as a lookup table:

```rust
/// BNY Mellon empirical data: (number_of_effective_positions, annual_vol%)
const DIVERSIFICATION_CURVE: &[(f64, f64)] = &[
    (1.0, 34.0),
    (2.0, 27.2),
    (5.0, 21.4),
    (10.0, 18.6),
    (20.0, 15.8),
    (50.0, 14.5),
];

/// Estimate annual volatility given the effective number of positions.
/// Uses linear interpolation between known points.
fn interpolate_vol(effective_positions: f64) -> f64
```

Effective number of positions is `1.0 / HHI` (the inverse HHI). For a single 80% position + 20% diversified: HHI ~ 0.64 + 0.04 = 0.68, effective positions ~ 1.47.

The quarter simulation:
```rust
for period in &programme.schedule {
    remaining_shares -= period.shares_target;
    let conc_value = remaining_shares * current_price;
    let total = conc_value + other_portfolio_value;
    let weight = conc_value / total;
    // Simplified HHI: weight^2 + (1 - weight) * market_weight^2
    // Approximate: assume rest is a diversified index (HHI ~ 0.01)
    let hhi = weight * weight + (1.0 - weight).powi(2) * 0.01;
    let eff_n = 1.0 / hhi;
    let vol = interpolate_vol(eff_n);
    points.push(ConcentrationPoint { ... });
}
```

### Web Route Registration

Add route in the axum router setup alongside existing dashboard routes. Follow the pattern of `selling_page()`.

### Error Handling

No new error variants. Missing data produces an empty chart.

### Migration Notes

None.

---

## Summary: New Files and Module Changes

### New Files

| File | Contents |
|---|---|
| `src/analysis/geopolitical.rs` | `GeoRiskInputs`, `GeoRiskScore`, `GeoRiskLevel`, `compute_geo_risk()`, `geo_allocation_adjustments()` |
| `src/portfolio/allocation.rs` | `AssetClass`, `RiskProfile`, `FundEntry`, `BrokerageCatalog`, `Brokerage`, `ConcreteAllocation`, `map_to_funds()`, `blended_expense_ratio()` |

### Modified Files

| File | Changes |
|---|---|
| `src/tax/brackets.rs` | Add `TaxBracketSchedule` type alias. Add `brackets` field to `StateTaxRate`. Update WA entry to 2-tier with $278K deduction. |
| `src/tax/engine.rs` | Promote `bracket_tax` to `pub(crate)`. Refactor state tax computation to use `bracket_tax` for tiered states. |
| `src/tax/projection.rs` | Add `ScenarioResult`, `compare_scenarios()`, `format_comparison_table()`. |
| `src/portfolio/types.rs` | Add `TaxLot::days_to_long_term()`. |
| `src/tax/hifo.rs` | Add `SellPriority` enum, `priority_select()`. |
| `src/portfolio/selling.rs` | Add `GeoRiskHigh`, `OilAbove(f64)`, `StockDropPercent(f64)` to `PaceCondition`. Add `MarketContext` struct. Refactor `apply_pace_rules()` signature. |
| `src/portfolio/prepare.rs` | Promote `compute_summary()` and `compute_annual_targets()` to `pub(crate)`. |
| `src/web/charts.rs` | Add `ConcentrationPoint`, `concentration_decay_points()`, `concentration_decay_svg()`, vol interpolation. |
| `src/web/dashboard_routes.rs` | Add `concentration_decay_page()` route handler. |
| `src/analysis/mod.rs` | Add `pub mod geopolitical;` |
| `src/portfolio/mod.rs` | Add `pub mod allocation;` |
| `src/main.rs` | Add `sell-compare` CLI subcommand. |

### Error Handling Changes

No new error variants required. All new functions use existing error types (`TaxError`, `PortfolioError`) or are infallible (return empty collections on edge cases).

### DB Migration Notes

No schema changes required across any items.

---

## Testing Checklist

Total new tests across all items: **~75 tests**

| Item | Module | New Tests |
|---|---|---|
| 1. Tiered StateTaxRate | `brackets.rs`, `engine.rs` | 14 (6 brackets + 8 engine) |
| 2. days_to_long_term | `types.rs` | 6 |
| 3. 3-Tier Priority | `hifo.rs` | 12 |
| 4. compare_scenarios | `projection.rs` | 11 |
| 5. sell-compare CLI | `projection.rs`, `main.rs` | 5 |
| 6. Geo Risk Scorer | `geopolitical.rs`, `selling.rs` | 15 |
| 7. Fund Mapper | `allocation.rs` | 16 |
| 8. Decay Chart | `charts.rs` | 12 |

Run order: `cargo test --lib` after each item to verify no regressions.

---

## Implementation Sequence for the Agent

1. **Item 1** first -- it touches foundational types used everywhere.
2. **Item 2** next -- trivial, unblocks Item 3.
3. **Item 3** after Item 2 -- uses `days_to_long_term`.
4. **Item 4** after Item 1 -- needs correct WA brackets for meaningful tests.
5. **Item 5** after Item 4 -- wraps compare_scenarios in CLI.
6. **Items 6 and 7** can proceed in parallel -- they are independent.
7. **Item 8** last -- depends on selling programme data structures being stable.

For each item: write ALL tests first (they must fail), then implement until green, then refactor. Commit after each item.
