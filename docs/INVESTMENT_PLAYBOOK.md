# Ghost Alpha Investment Playbook (Example)

> **Example Budget:** $10,000 · **Date:** 2026-03-17 · **Regime:** Sideways · **Invested:** $6,920 (69.2%) · **Cash:** $3,080 (30.8%)

> **DISCLAIMER:** This is a synthetic example portfolio used for documentation purposes. It does not represent any real holdings, cost basis, or investment advice. All tickers, share counts, and prices are illustrative only.

This playbook documents an example investment strategy for a hypothetical $10K concentrated growth portfolio managed with Ghost Alpha's screening and analysis pipeline. Every decision references Ghost Alpha signals; where the system lacks capability, a development roadmap item is noted.

---

## Table of Contents

1. [Market Environment](#1-market-environment)
2. [Universe & Screening Criteria](#2-universe--screening-criteria)
3. [Current Holdings Analysis](#3-current-holdings-analysis)
4. [Allocation Rationale](#4-allocation-rationale)
5. [Entry Criteria](#5-entry-criteria)
6. [Position Sizing](#6-position-sizing)
7. [Exit Rules](#7-exit-rules)
8. [Rebalancing Schedule](#8-rebalancing-schedule)
9. [Regime Rules Matrix](#9-regime-rules-matrix)
10. [Week-by-Week Action Plan](#10-week-by-week-action-plan)
11. [Development Roadmap](#11-development-roadmap)
12. [Risk Warnings](#12-risk-warnings)

---

## 0. Related: Portfolio Plan Pipeline

This playbook focuses on **stock-screening strategy**. For forward projection of a portfolio with a concentrated position (broker exports, vesting schedule, unwind plan, Monte Carlo NAV bands), see **[PLAN_PIPELINE.md](PLAN_PIPELINE.md)** — run `ghost-alpha init-data my_data` to start from templates.

Where this playbook says "rebalance" or "trim", the plan pipeline lets you model the actual dollar amounts, dates and tax effects for your own holdings.

---

## 1. Market Environment

**Current regime: Sideways** (as of 2026-03-17, per `ghost-alpha regime`)

| Signal | Value | Interpretation |
|--------|-------|---------------|
| SPY vs 200-SMA | Mixed — hovering near the line | No clear directional bias |
| Market breadth | 40-60% range | Neither broad participation nor collapse |
| Macro risk level | Moderate | Elevated VIX, yield curve flattening |

**Implications for this portfolio:**

- **Do not chase.** Sideways regimes reward selectivity. Only A+ setups justify new capital deployment.
- **Hold winners.** Existing positions with strong RS and intact Minervini conditions should be held.
- **Tighten monitoring.** Weekly screening is mandatory. A regime shift to Bear triggers the defensive playbook (Section 9).
- **Cash is a position.** The 30.8% cash reserve is a feature, not a bug. It provides dry powder for a Bull shift or protection during a Bear shift.

---

## 2. Universe & Screening Criteria

### Universe

S&P 500 (default), screened via `ghost-alpha screen --provider yahoo`.

### Screening Pipeline

1. **Minervini 8-point filter** — all 8 conditions checked (`src/analysis/indicators.rs`)
2. **Composite score** — Technical (55) + Volume (30) + Sentiment (15) = 0-100
3. **Grade assignment** — percentile-based: A+ (top 5%), A (top 15%), B (top 30%), C/D/F below

### Regime-Adaptive Grade Minimums

The minimum grade required for new buys varies by market regime:

| Regime | Minimum Grade for New Buys | Rationale |
|--------|---------------------------|-----------|
| **Bull** | B or above | Broad participation — more setups work |
| **Sideways** | A or above | Selectivity required — only the strongest survive |
| **Bear** | A+ only | Capital preservation — exceptional setups only |

**Current requirement (Sideways): Grade ≥ A for new positions.**

Existing positions graded B are "grandfathered" — they were bought when their signals were stronger. They are held unless exit signals trigger (Section 7).

---

## 3. Current Holdings Analysis

All 8 positions were entered between Dec 2025 and Mar 2026. Data below reflects the most recent Ghost Alpha screening run.

| # | Ticker | Shares | Cost | Current | P&L | Grade | RS% | Conditions | Composite | Status |
|---|--------|--------|------|---------|-----|-------|-----|------------|-----------|--------|
| 1 | GOOGL | 5 | $175 | ~$190 | +8.6% | A+ | 90th | 8/8 | 88 | **HOLD** — exceeds Sideways minimum |
| 2 | AMD | 7 | $125 | ~$140 | +12% | A+ | 95th | 8/8 | 91 | **HOLD** — highest RS in portfolio |
| 3 | NVDA | 8 | $135 | ~$180 | +33% | A | 70th | 7/8 | 78 | **HOLD** — monitor RS trend |
| 4 | PLTR | 10 | $85 | ~$151 | +77% | A | 85th | 8/8 | 83 | **HOLD** — selling programme active |
| 5 | AVGO | 3 | $195 | ~$210 | +7.7% | B | 65th | 6/8 | 68 | **REVIEW** — below Sideways minimum, grandfathered |
| 6 | TSLA | 3 | $260 | ~$275 | +5.8% | B | 75th | 6/8 | 65 | **HOLD** — robotaxi catalyst watch |
| 7 | DDOG | 8 | $100 | ~$108 | +8% | B | 60th | 6/8 | 62 | **HOLD** — weakest RS, monitor closely |
| 8 | AAPL | 5 | $215 | ~$220 | +2.3% | B | 55th | 6/8 | 60 | **HOLD** — lowest RS, consider trim if RS < 50 |

### Position Quality Tiers

- **Tier 1 (A+ grade):** GOOGL, AMD — core positions, max allocation, widest stops
- **Tier 2 (A grade):** NVDA, PLTR — strong positions, standard allocation
- **Tier 3 (B grade, grandfathered):** AVGO, TSLA, DDOG, AAPL — on watch, tighter stops

---

## 4. Allocation Rationale

### Initial Deployment Strategy

The $10K budget was deployed in three waves:

| Wave | Timing | Amount | % of Budget | Purpose |
|------|--------|--------|-------------|---------|
| **Immediate** | Dec 2025 - Jan 2026 | ~$5,000 | 50% | Core positions in highest-conviction picks (NVDA, GOOGL, AMD) |
| **Staggered** | Feb 2026 | ~$1,920 | ~19% | Second-tier entries as signals confirmed (AVGO, PLTR, TSLA) |
| **Reserve** | Mar 2026 | ~$1,875 | ~19% | Opportunistic entries (DDOG, AAPL) |
| **Cash** | Held | $3,080 | 30.8% | Dry powder for A+ setups or drawdown protection |

### Why 50/30/20?

- **50% immediate:** Capture strongest signals before they appreciate. A+ picks rarely stay cheap.
- **30% staggered:** Allows price confirmation. Reduces timing risk.
- **20% reserve:** Critical in Sideways regime. Provides flexibility to average up on winners or respond to regime changes.

### Regime-Adaptive Exposure Limits

| Regime | Target Invested % | Max Invested % | Cash Floor |
|--------|-------------------|---------------|------------|
| **Bull** | 80-95% | 100% | 0% |
| **Sideways** | 50-80% | 80% | 20% |
| **Bear** | 20-50% | 50% | 50% |

**Current status:** 69.2% invested — **within Sideways target range** (50-80%). No action needed on overall allocation.

---

## 5. Entry Criteria

New positions must pass both **hard gates** (all required) and score well on **soft factors** (judgment-based).

### Hard Gates (ALL must pass)

| Gate | Threshold | Source | Check |
|------|-----------|--------|-------|
| Minervini filter | All 8 conditions pass | `screen_results.minervini_pass = true` | `ghost-alpha screen` |
| Grade | ≥ regime minimum (currently A) | `screen_results.grade` | Regime Rules Matrix (Section 9) |
| Volume Z-score | > 0.5 | `screen_results` composite breakdown | Confirms institutional interest |
| RS percentile | > 70th | `screen_results` RS field | Relative strength vs peers |
| Available cash | ≥ position size | `portfolio_snapshots` | Can't buy what you can't afford |

### Soft Factors (judgment-based, additive)

| Factor | What to Look For | How to Check |
|--------|-----------------|--------------|
| VCP pattern | Volatility contraction, tightening range | Visual chart review (manual) |
| ADX strength | ADX > 25, ideally > 30 | `ghost-alpha screen` output |
| Risk:Reward ratio | ≥ 3:1 based on ATR stop distance | **GAP: no position sizer yet** → Roadmap P1 |
| Fundamental quality | Revenue growth, margin expansion | **GAP: fundamentals not in composite** → Roadmap P1 |
| Sector diversification | No more than 2 positions per sector | Manual check against portfolio |
| Earnings proximity | Avoid buying within 2 weeks of earnings | **GAP: no earnings calendar** → Roadmap P2 |

---

## 6. Position Sizing

### Target Framework

**1% risk per trade** — maximum loss on any single position should not exceed 1% of portfolio value ($100 on a $10K portfolio).

```
Position Size = Risk Budget / (Entry Price - Stop Price)
Risk Budget   = Portfolio Value × 0.01 × Score Multiplier × Regime Multiplier
Stop Price    = Entry Price - (ATR × Stop Multiplier)
```

### Score Multiplier

| Grade | Multiplier | Effect on Position Size |
|-------|-----------|------------------------|
| A+ | 1.5x | Larger position — highest conviction |
| A | 1.0x | Standard position |
| B | 0.7x | Smaller position — lower conviction |
| C or below | 0.0x | No position — does not meet minimum |

### Regime Multiplier

| Regime | Multiplier | Effect |
|--------|-----------|--------|
| Bull | 1.0x | Full sizing |
| Sideways | 0.8x | Reduced sizing (current) |
| Bear | 0.5x | Half sizing |

### Current Limitations

> **GAP:** Ghost Alpha does not currently have a position sizing engine. All position sizes in this portfolio were determined manually using the framework above. See [Development Roadmap](#11-development-roadmap) item P1.

> **GAP:** ATR-based stop calculation exists in `src/analysis/indicators.rs` but is not integrated into a sizing workflow.

> **GAP:** Macro risk data (VIX, OAS, yield curve) from `macro_risk` table exists but does not feed into position scaling. See Roadmap P2.

### Practical Sizing for Current Portfolio

With $10K and Sideways regime (0.8x multiplier):

| Grade | Effective Risk | Approx. Position Size |
|-------|---------------|----------------------|
| A+ | $100 × 1.5 × 0.8 = $120 | $1,200-$1,500 |
| A | $100 × 1.0 × 0.8 = $80 | $800-$1,000 |
| B (grandfathered) | $100 × 0.7 × 0.8 = $56 | $560-$700 |

---

## 7. Exit Rules

### Existing Exit Signals (implemented in `src/alerts/exit_signals.rs`)

These three signals are already operational and checked daily via `ghost-alpha screen`:

| Signal | Condition | Severity | Action |
|--------|-----------|----------|--------|
| **Stage 2 Breakdown** | ≥3 of 8 Minervini conditions lost | Most severe | **Sell immediately** — structural trend support is gone |
| **RS Deterioration** | RS fell below 50 (was ≥80 at entry) | Moderate | **Sell within 2 days** — stock is weakening relative to peers |
| **MA Death Cross** | 50-SMA crossed below 200-SMA | Long-term | **Sell on next open** — classic trend reversal |

### Proposed Exit Signals (development needed)

| Signal | Condition | Priority | Motivation |
|--------|-----------|----------|------------|
| **Profit-Taking** | Position up ≥ 50% → sell 25%; up ≥ 100% → sell 50% | P2 | Lock in gains; prevent round-trip losses. PLTR at +77% is approaching first trigger. |
| **ATR Trailing Stop** | Stop trails at 2× ATR below highest close | P1 | Graduated protection; tightens as price rises. Current exits are binary (all-or-nothing). |
| **Regime-Change Exit** | Bear shift triggers flag on all sub-A positions | P3 | Automatic risk reduction when macro deteriorates. Currently manual. |
| **Time Stop** | Position held > 90 days with < 5% gain | P3 | Frees capital from dead money. Not urgent but improves capital efficiency. |

### Exit Decision Matrix

| Condition | Tier 1 (A+) | Tier 2 (A) | Tier 3 (B, grandfathered) |
|-----------|-------------|------------|--------------------------|
| Stage 2 Breakdown | Sell 50%, review rest | Sell 100% | Sell 100% |
| RS Deterioration | Watch 1 week | Sell within 2 days | Sell immediately |
| MA Death Cross | Sell 100% | Sell 100% | Sell 100% |
| Grade drops to C | Trim 50% | Sell 100% | Sell 100% |
| Grade drops to D/F | Sell 100% | Sell 100% | Sell 100% |

---

## 8. Rebalancing Schedule

| Frequency | Action | Ghost Alpha Command | Output |
|-----------|--------|-------------------|--------|
| **Daily** (automated) | Screen universe, check exit signals on flagged stocks | `ghost-alpha screen --provider yahoo` | Alerts to Telegram/Discord |
| **Weekly** (Monday) | Full portfolio review — grades, RS, conditions | `ghost-alpha screen` + review `/decisions` page | Decision log REVIEW entry |
| **Monthly** | Rebalance — trim losers, add to winners, sector check | Manual review of portfolio vs. regime rules | Decision log entries per action |
| **Quarterly** | Deep review — regime analysis, strategy performance, roadmap update | `ghost-alpha backtest --format html` + this playbook | Updated playbook |

### Weekly Review Checklist

1. Run `ghost-alpha screen --provider yahoo` — check all 8 positions for exit signals
2. Run `ghost-alpha regime` — confirm Sideways; watch for Bull/Bear shift
3. Check each position's grade — any drops below Tier threshold?
4. Review RS percentiles — any deterioration trends?
5. Check sector concentration — still diversified?
6. Review cash position vs. regime target range
7. Log decisions in `/decisions` dashboard page

---

## 9. Regime Rules Matrix

This matrix governs all investment decisions based on the current market regime detected by `ghost-alpha regime`.

| Parameter | Bull | Sideways (CURRENT) | Bear |
|-----------|------|-------------------|------|
| **Max invested %** | 100% | 80% | 50% |
| **Min grade (new buys)** | B | A | A+ |
| **Max positions** | 12 | 8 | 5 |
| **Position size multiplier** | 1.0x | 0.8x | 0.5x |
| **Stop tightness** | Wide (3× ATR) | Standard (2× ATR) | Tight (1.5× ATR) |
| **New position frequency** | Weekly | Bi-weekly | Monthly max |
| **Cash deployment** | Aggressive | Selective | Defensive |
| **Exit sensitivity** | Patient | Standard | Hair-trigger |

### Regime Transition Rules

| Transition | Action |
|------------|--------|
| Sideways → **Bull** | Increase max invested to 95%. Look for B-grade entries. Widen stops. Deploy cash over 2-3 weeks. |
| Sideways → **Bear** | Immediately flag all B-grade positions for review. Reduce max invested to 50%. Tighten stops to 1.5× ATR. Sell weakest positions first. |
| Bear → **Sideways** | Cautiously increase exposure. Only A+ entries for first 2 weeks. Raise invested cap to 60%, then 80% if breadth confirms. |
| Bull → **Bear** (crash) | Sell all D/F positions immediately. Trim B positions by 50%. Keep only A+ and strong A. Move to 50% cash within 1 week. |

---

## 10. Week-by-Week Action Plan

### Week 1 (Mar 17-21) — Playbook Inception

- [x] Document current portfolio state and all positions
- [x] Seed decision log with playbook inception entries
- [ ] Run `ghost-alpha screen --provider yahoo` — baseline all 8 positions
- [ ] Run `ghost-alpha regime` — confirm Sideways classification
- [ ] Review PLTR selling programme (currently at +77%, approaching 50% profit-take trigger)
- [ ] Identify any A+ candidates in latest screening for potential cash deployment

### Week 2 (Mar 24-28) — Signal Validation

- [ ] Run daily screens; compare Monday vs Friday grades for all 8 positions
- [ ] If any Tier 3 position (AVGO, TSLA, DDOG, AAPL) drops to C grade → execute trim
- [ ] If AAPL RS drops below 50 → execute sell (RS Deterioration rule)
- [ ] Log all decisions via SQL or future CLI command

### Week 3 (Mar 31 - Apr 4) — First Rebalance Window

- [ ] Monthly rebalance: compare actual allocation vs. target
- [ ] If Tier 1 positions (GOOGL, AMD) gained significantly → let them run, don't trim winners
- [ ] If cash > 35% and an A+ candidate appears → enter new position using sizing framework
- [ ] Begin development of position sizing engine (Roadmap P1)

### Week 4 (Apr 7-11) — Regime Check

- [ ] Run `ghost-alpha regime` — any shift signals?
- [ ] If Bull transition begins: plan cash deployment schedule for Weeks 5-6
- [ ] If Bear transition begins: execute Bear transition rules (Section 9)
- [ ] Continue P1 development work (position sizer, ATR trailing stops)

### Week 5 (Apr 14-18) — Mid-Review

- [ ] Compare portfolio performance vs. SPY benchmark
- [ ] Review all positions against original entry thesis (decision_log BUY entries)
- [ ] Any position that has diverged from thesis → flag for exit review
- [ ] Expected: P1 position sizer should be testable

### Week 6 (Apr 21-25) — Strategy Refinement

- [ ] Run `ghost-alpha backtest --format html` — validate scoring accuracy
- [ ] If backtest shows grade A+ significantly outperforms B → increase A+ allocation weight
- [ ] Review sector exposure: currently tech-heavy (7/8 positions) — seek diversification if A+ appears in other sectors
- [ ] Expected: P1 ATR trailing stops integrated

### Week 7 (Apr 28 - May 2) — Earnings Season Prep

- [ ] Major earnings: AAPL, AMD, GOOGL typically report late April
- [ ] Review position sizes before earnings — reduce any oversized positions
- [ ] **GAP:** No earnings calendar integration → manual tracking required
- [ ] Document any position changes in decision log

### Week 8 (May 5-9) — Quarterly Deep Review

- [ ] Full quarterly review: performance, regime changes, strategy effectiveness
- [ ] Update this playbook with actual results vs. plan
- [ ] Evaluate development roadmap progress
- [ ] Set goals for next 8-week cycle
- [ ] Run `ghost-alpha backtest --format html` for comprehensive performance report

---

## 11. Development Roadmap

These are capability gaps identified during playbook creation. Each item includes the priority, estimated effort, affected files, and motivation.

| Priority | Gap | Effort | Files | Motivation |
|----------|-----|--------|-------|------------|
| **P1** | Position sizing engine | 2-3d | new `src/analysis/position_sizer.rs` | Can't manage risk without knowing how much to buy. The 1% risk framework in Section 6 is entirely manual. |
| **P1** | ATR trailing stops | 1-2d | `src/alerts/exit_signals.rs` | Current exits are binary (all-or-nothing). Need graduated profit protection that tightens as price rises. |
| **P1** | Fundamental score in composite | 2-3d | `src/analysis/scorer.rs` | Scorer ignores earnings quality, valuation, balance sheet. Fundamental data exists in `fundamental_metrics` table but isn't weighted. |
| **P2** | Macro risk → position scaling | 2d | `src/analysis/scorer.rs`, `src/analysis/regime.rs` | VIX/OAS/yield curve data exists in `macro_risk` table but doesn't affect sizing or scoring. |
| **P2** | Profit-taking exits | 1-2d | `src/alerts/exit_signals.rs` | No way to lock in gains. PLTR at +77% has no automated trigger. Rides winners back down. |
| **P2** | Earnings calendar gating | 2-3d | new `src/data/earnings_calendar.rs`, `src/main.rs` | Buying within 2 weeks of earnings is a coin flip, not edge. Need to flag or block. |
| **P3** | Regime-change exit triggers | 1d | `src/alerts/exit_signals.rs` | Bear shift should auto-flag sub-A positions for exit instead of requiring manual review. |
| **P3** | Win rate by grade (backtest) | 2-3d | `src/backtest/mod.rs` | Need historical data to validate whether A+ actually outperforms B. Currently assumed, not proven. |

### Development Timeline (Target)

```
Week 3-4:  P1 — Position sizing engine + ATR trailing stops
Week 5-6:  P1 — Fundamental score integration
Week 7-8:  P2 — Macro risk scaling + profit-taking exits
Week 9-10: P2 — Earnings calendar gating
Week 11+:  P3 — Regime-change triggers + grade backtest validation
```

---

## 12. Risk Warnings

### Portfolio-Specific Risks

1. **Concentration risk:** 7 of 8 positions are in the technology sector. A sector-wide selloff (regulation, rate shock, AI capex pullback) would hit the entire portfolio simultaneously. Mitigation: prioritize non-tech A+ candidates for next entry.

2. **Small portfolio effect:** With $10K and 8 positions, each position is $800-$1,500. Transaction costs and bid-ask spreads have proportionally higher impact. Consider consolidating into 5-6 larger positions.

3. **No automated risk management:** Position sizing, trailing stops, and profit-taking are all manual. Until the P1 roadmap items are complete, discipline depends entirely on following this playbook.

4. **Regime detection lag:** The regime detector uses SPY vs 200-SMA and breadth. Both are lagging indicators. A sharp crash can happen before the regime officially shifts to Bear.

5. **Grandfathered positions:** Four B-grade positions (AVGO, TSLA, DDOG, AAPL) would not meet current Sideways entry criteria. They represent embedded risk from earlier, more lenient entry standards.

### General Disclaimers

- **Ghost Alpha is an analysis tool, not investment advice.** All signals are informational. Final decisions rest with the investor.
- **Past screening results do not predict future returns.** Grades and scores reflect current technical conditions, not forward-looking value.
- **This playbook is a living document.** It should be updated at each quarterly review (Section 8) and whenever regime changes occur.
- **Backtested strategies have survivorship and look-ahead biases.** Real-world performance will differ from `ghost-alpha backtest` output.
- **Options and leverage are not covered.** This playbook assumes cash equities only.

---

*Last updated: 2026-05-03 (added § 0 cross-links to Plan Pipeline) · Next review: 2026-05-09 (Week 8)*
