# GhostAlpha — Feature & Tool Gap Analysis for Diversification Advisory

> **Context:** Mapping existing GhostAlpha capabilities against Phases 2–8 of the
> AMZN Concentrated Position Diversification Research Plan.
>
> **Legend:**
> - **HAS** = capability exists and is production-ready
> - **PARTIAL** = foundation exists but needs extension
> - **GAP** = capability does not exist — must be built or integrated externally

---

## Phase 2 — AMZN Fundamental Deep Dive

### 2.1 Business Quality Assessment

| Requirement | Status | What Exists | Gap |
|---|---|---|---|
| Revenue segment breakdown (AWS / Ads / Retail) | **GAP** | `FundamentalSnapshot` has aggregate revenue only | Need segment-level revenue, operating income, and margins. Finnhub `/stock/financials-reported` or SEC EDGAR XBRL could provide segment data |
| Margin trajectory (quarterly, by segment) | **GAP** | Single-point gross/operating/net margins in `FundamentalSnapshot` | Need time-series margin history. Add `margin_history` table + quarterly fetch |
| Capital allocation analysis (CapEx, buybacks, debt) | **GAP** | No cash-flow statement data | Need CapEx, FCF, share buyback, debt issuance/repayment series. Requires cash-flow-statement API or SEC filing parser |
| Competitive moat scoring | **GAP** | No qualitative business model analysis | Out of scope for quantitative tool. Could add a structured "moat scorecard" input form on web dashboard for manual analyst entry |
| Management & governance (insider ownership, board) | **GAP** | No insider transaction or proxy data | Finnhub has `/stock/insider-transactions` and `/stock/insider-sentiment`. Would need new fetcher + table |
| Regulatory risk tracking | **GAP** | News fetcher captures headlines but no structured legal/regulatory tracker | Could tag news articles with "regulatory" label via NLP classifier, or integrate a legal filing feed |

### 2.2 Valuation Framework

| Requirement | Status | What Exists | Gap |
|---|---|---|---|
| DCF model (3-stage) | **GAP** | No DCF engine | Need: revenue projections, margin assumptions, WACC calculator, terminal value logic. Major new module (`src/analysis/valuation.rs`) |
| Sum-of-the-Parts valuation | **GAP** | No segment-level data (see 2.1) | Blocked by segment data gap. Once segments available, need SOTP calculator with per-segment multiples |
| Relative valuation (P/E, EV/EBITDA vs peers) | **PARTIAL** | Have P/E, P/B, P/S in `FundamentalSnapshot` + `SectorPeBenchmark` for sector medians | Missing: EV/EBITDA, EV/Revenue, P/FCF, PEG ratio. Need enterprise value calculation (market cap + debt - cash). Need peer-group comparison view (not just sector median) |
| Historical valuation bands (5-year range) | **GAP** | Only current-point fundamentals stored | Need time-series fundamental storage. Add `fundamental_history` table with quarterly snapshots. Chart valuation bands on web dashboard |
| Implied growth rate (reverse DCF) | **GAP** | No reverse-engineering of market expectations | Need reverse-DCF solver: given current price, solve for implied revenue growth rate. Useful "is the market pricing in X% growth?" sanity check |
| Bear/Base/Bull scenario modelling | **GAP** | No scenario engine | Need scenario input framework (assumptions → projected price range → expected value). Could be a web form + calculator |

### 2.3 Technical & Momentum Check

| Requirement | Status | What Exists | Gap |
|---|---|---|---|
| Trend regime (Stage 2 detection) | **HAS** | MinerviniScreener with full 8-point Minervini conditions, Stage 2 detection | Fully implemented |
| Relative strength vs S&P 500 | **HAS** | RS percentile calculated cross-sectionally, SPY benchmark candles fetched | Fully implemented, shown on dashboard scatter plot |
| ADX / RSI / volume analysis | **HAS** | ADX(14), RSI(14), Volume Z-score, ATR(14), 52-week high/low all implemented | Fully implemented |
| Institutional flows (13F positioning) | **GAP** | No 13F / institutional ownership data | Finnhub has `/stock/ownership` and `/institutional-ownership`. Would need new fetcher, table, and trend analysis (accumulation vs distribution) |
| Options market (IV skew, put/call, blocks) | **GAP** | `MacroRiskLevel` references put/call ratio as input but no options data fetcher exists | Need options chain data provider. Finnhub has `/stock/option-chain`. Would enable IV skew, put/call OI, unusual activity detection |
| Selling-into-strength timing logic | **PARTIAL** | Exit signals detect deterioration (Stage 2 breakdown, RS decay, death cross) | Inverse logic needed: "sell readiness signal" when stock is in strong uptrend + overbought + high volume — optimal window to reduce |

---

## Phase 3 — Tax & Legal Strategy

| Requirement | Status | What Exists | Gap |
|---|---|---|---|
| Lot-level position tracking (cost basis, date, shares) | **GAP** | No portfolio/position management at all. GhostAlpha is a screener, not a portfolio tracker | **MAJOR GAP.** Need entirely new `src/portfolio/` module: `Position`, `TaxLot`, `Portfolio` structs. Import from brokerage CSV/API. DB tables for lots |
| Tax impact calculator (federal + state + NIIT) | **GAP** | No tax logic | Need tax rate tables (2024/2025 brackets), LTCG/STCG classification per lot, NIIT threshold check, state tax lookup. New module `src/tax/` |
| HIFO lot selection optimiser | **GAP** | No lot-level data | Once lots exist, implement HIFO (Highest In, First Out) sort + selection to minimise realised gain per $ liquidated |
| Tax-loss harvesting scanner | **GAP** | No portfolio positions tracked | Need to scan existing holdings for unrealised losses that can offset AMZN gains. Requires portfolio module |
| Charitable giving calculator (CRT, DAF) | **GAP** | No charitable/estate logic | Out of scope for a trading tool. Better served by external tax planning software (Holistiplan, BNA). Could add a simple "donate X shares → deduction value" calculator |
| Exchange fund eligibility check | **GAP** | No portfolio valuation | Needs net worth / position size threshold check. Simple rule engine once portfolio data exists |
| 10b5-1 plan modelling | **GAP** | No scheduled-selling logic | Need "selling programme" module: define schedule (shares/month or $/month), project future tax impact per period, simulate price scenarios |
| Options-based hedging (collar, prepaid forward) | **GAP** | No options pricing or strategy modelling | Need Black-Scholes or binomial pricer, collar payoff diagram, constructive sale rule checker. Significant scope |
| Multi-year tax projection | **GAP** | No tax bracket projection | Need income projection (salary + AMZN sales + dividends + other), bracket stacking across years, AMT check |

**Summary for Phase 3:** GhostAlpha has **zero** portfolio/tax capabilities. This is the largest single gap — it requires an entirely new domain (portfolio management + tax modelling) that is orthogonal to the existing screener architecture.

---

## Phase 4 — Target Allocation Design

### 4.1 Strategic Asset Allocation

| Requirement | Status | What Exists | Gap |
|---|---|---|---|
| Multi-asset-class data (bonds, REITs, commodities, international) | **GAP** | Only US equities via Finnhub/Yahoo. No bond, commodity, REIT, or international index data | Need to fetch ETF price data for asset class proxies (AGG, TIP, VNQ, GLD, EFA, EEM, etc.). Yahoo provider can already fetch ETF candles — just need to expand universe concept beyond stocks |
| Historical correlation matrix | **PARTIAL** | `src/analysis/correlation.rs` has sector concentration detection only | Need full N×N pairwise correlation calculator for arbitrary ticker lists. Foundation exists (have candle data + return calculations) but correlation matrix computation is missing |
| Rolling correlation (3-year windows) | **GAP** | No rolling window correlation | Need windowed correlation with time-series output + chart on dashboard |
| Crisis correlation analysis | **GAP** | No event-based analysis framework | Need to define crisis periods (2008, 2020, 2022) and compute correlation matrices during those windows specifically. Relatively easy once base correlation module exists |
| Factor decomposition (market, size, value, momentum, quality) | **GAP** | RS percentile is a single momentum factor. No multi-factor model | Need factor return series (Fama-French, AQR), regression engine to decompose asset returns into factor exposures. Major analytical addition |
| Mean-variance optimisation (MVO) | **GAP** | No portfolio optimisation engine | Need: expected return estimates, covariance matrix, constraints (min/max weights, no short), efficient frontier solver. Could use quadratic programming library |
| Monte Carlo simulation for portfolios | **PARTIAL** | Monte Carlo exists for backtest bootstrap, not for portfolio projection | Need forward-looking Monte Carlo: simulate portfolio paths over 5/10/20 years with rebalancing, withdrawals, tax drag. Extend existing `monte_carlo.rs` |
| Risk parity portfolio construction | **GAP** | No risk parity algorithm | Need inverse-volatility weighting or full risk parity (equal risk contribution). Iterative solver required |

### 4.2 Model Portfolio Construction

| Requirement | Status | What Exists | Gap |
|---|---|---|---|
| Multiple portfolio templates (Conservative / Moderate / Growth) | **GAP** | No portfolio modelling concept | Need `ModelPortfolio` struct with asset class weights, expected return/vol/Sharpe calculations, drawdown simulation |
| Tax cost of transition modelling | **GAP** | Blocked by portfolio + tax modules (Phase 3) | Once tax module exists, compute cost-of-transition from current state → each model portfolio |
| Probability of meeting return target | **GAP** | No goal-based planning | Need Monte Carlo simulation with success probability (e.g., "87% chance of reaching $X by year Y") |

---

## Phase 5 — Vehicle Selection

| Requirement | Status | What Exists | Gap |
|---|---|---|---|
| ETF screening & comparison | **GAP** | No ETF-specific data (expense ratio, tracking error, AUM, holdings) | Need ETF data provider. Options: ETF.com API, Finnhub `/etf/holdings`, or Yahoo Finance ETF pages. New `EtfSnapshot` struct |
| Holdings overlap analysis | **GAP** | No ETF holdings data | Need to check "if I buy VTI, how much AMZN am I adding back?" Requires ETF constituent weights |
| Expense ratio comparison | **GAP** | No fee data | Part of ETF data provider |
| Tax cost ratio (Morningstar-style) | **GAP** | No after-tax return tracking | Need tax-adjusted return calculation. Could estimate from capital gains distribution history |
| Fund flow / AUM trend | **GAP** | No fund flow data | Low priority — available on external platforms |
| Direct indexing simulation | **GAP** | No custom index construction | Need: replicate index (e.g., S&P 500 ex-AMZN) with individual stocks, simulate continuous tax-loss harvesting alpha, compare to ETF. Advanced feature |
| SMA vs ETF vs mutual fund comparison matrix | **GAP** | No vehicle comparison framework | Need decision-tree or scoring model: given position size, tax situation, and customisation needs, recommend optimal vehicle type |

---

## Phase 6 — Transition Plan & Execution

| Requirement | Status | What Exists | Gap |
|---|---|---|---|
| Multi-year liquidation schedule builder | **GAP** | No selling programme logic | Need: define annual selling targets (shares or $ amount), assign lots via HIFO, project tax per year, visualise schedule. Requires portfolio + tax modules |
| Decision rules engine (accelerate / decelerate) | **PARTIAL** | Exit signals exist for flagged stocks. Regime detection (Bull/Sideways/Bear) could inform pace | Need to connect regime + technical signals to selling pace. E.g., "if AMZN breaks 200-SMA, accelerate to 2× planned pace" |
| Hedging strategy modeller (collar, put spread) | **GAP** | No options pricing | Need options payoff calculator, collar design tool (select strikes to achieve zero-cost), P&L diagram generator |
| Cash management / redeployment tracker | **GAP** | No cash position or deployment tracking | Need: track proceeds from AMZN sales, DCA schedule into target allocations, progress bar toward target weights |
| Transaction cost modelling | **HAS** | Backtest engine has `transaction_cost_bps` (10 bps default round-trip) | Exists for backtesting. Could extend to liquidation planning |

---

## Phase 7 — Monitoring & Rebalancing

| Requirement | Status | What Exists | Gap |
|---|---|---|---|
| Portfolio weight tracking | **GAP** | No portfolio concept | Blocked by portfolio module (Phase 3) |
| HHI concentration metric | **GAP** | No portfolio-level diversification metric | Simple to compute once portfolio weights exist: HHI = Σ(weight²) |
| Automated tax-loss harvesting detection | **GAP** | No portfolio positions | Once positions exist: scan daily for lots with losses > threshold, check wash sale 30-day window, surface opportunities |
| Threshold-based rebalancing alerts | **PARTIAL** | Alert engine exists with configurable thresholds and dedup. Telegram/Discord/Webhook channels ready | Need new alert type: "AMZN weight drifted to 32% (target: 25%, threshold: +5%)" — easy to add once portfolio weights exist |
| AMZN fundamental thesis quarterly review | **PARTIAL** | Earnings history tracked, fundamental snapshot exists | Need: automated post-earnings comparison (actual vs estimate), thesis change detection, dashboard card showing "earnings beat/miss streak" |
| Performance attribution (portfolio vs benchmark) | **GAP** | Backtest has alpha calculation, but it's signal-level not portfolio-level | Need Brinson-style attribution: allocation effect + selection effect + interaction. Requires portfolio with actual weights |
| Tax alpha reporting | **GAP** | No tax-lot-level tracking | Need: "tax-loss harvesting saved $X this year" running total. Requires full tax lot lifecycle tracking |
| Quarterly performance report (PDF/HTML) | **PARTIAL** | HTML backtest report exists (`html_report.rs`). CSV/JSON export exists | Need portfolio-specific report template: performance, allocation pie chart, rebalancing actions taken, tax summary, risk metrics. Extend existing HTML report infrastructure |

---

## Phase 8 — Behavioural & Governance

| Requirement | Status | What Exists | Gap |
|---|---|---|---|
| Investment Policy Statement (IPS) generator | **GAP** | No document generation | Need structured form → IPS document (PDF/Markdown). Fields: objectives, constraints, target allocation, rebalancing rules, risk limits, review schedule |
| "Would you buy this today?" framing tool | **GAP** | No behavioural nudge features | Simple but valuable: show "If you had $X cash, would you put Y% into AMZN?" reframing on dashboard when AMZN weight exceeds target |
| Concentration risk visualisation | **PARTIAL** | Heatmap and scatter charts exist on analytics page | Need: portfolio-level pie chart showing AMZN vs everything else, historical concentration trajectory chart, drawdown comparison (concentrated vs diversified) |
| Automated selling programme (scheduled trades) | **GAP** | No trade execution or scheduling | Need integration with brokerage API (Alpaca, IBKR, Schwab) for scheduled sells, or at minimum a "planned vs executed" tracker |
| Audit trail / decision log | **PARTIAL** | `alert_log` table records all alerts with timestamps. `screen_results` records daily signals | Need: explicit "decision_log" for manual advisor entries — "Sold 100 shares AMZN lot #3 because [reason]" with timestamp. Simple table + web form |
| Review cadence scheduler / reminders | **GAP** | No scheduling or calendar features | Could use alert channels (Telegram/Discord) with cron-like scheduler: "Quarterly review due in 5 days" |

---

## Gap Summary — Prioritised by Impact

### Tier 1 — Foundational (blocks multiple phases)

| # | Gap | Phases Blocked | Effort | Description |
|---|---|---|---|---|
| **G1** | **Portfolio & Position Management** | 3, 4, 6, 7, 8 | **XL** | Tax lots, cost basis, holdings, weights. The single biggest missing piece. Without this, Phases 3–8 cannot function. New module: `src/portfolio/` with `Position`, `TaxLot`, `Portfolio`, `PortfolioSnapshot` structs + DB tables + import from CSV/brokerage |
| **G2** | **Multi-Asset Data (bonds, REITs, commodities, intl)** | 4, 5 | **M** | Extend Yahoo provider to fetch ETF/index candles (AGG, TIP, VNQ, GLD, EFA, EEM). Yahoo already supports ETFs — mainly universe + UI work |
| **G3** | **Correlation Matrix Engine** | 4, 7 | **M** | Full N×N pairwise Pearson correlation from return series. Rolling window support. Crisis-period slicing. Build on existing candle/return infrastructure |
| **G4** | **Tax Calculation Engine** | 3, 6, 7 | **L** | Federal + state brackets, LTCG/STCG classification, NIIT, AMT check. Lot-level gain/loss computation. Multi-year projection |

### Tier 2 — High Value (enables key analysis)

| # | Gap | Phases Enabled | Effort | Description |
|---|---|---|---|---|
| **G5** | **DCF / Valuation Module** | 2 | **L** | 3-stage DCF, reverse DCF (implied growth), SOTP calculator, scenario engine (bear/base/bull). New `src/analysis/valuation.rs` |
| **G6** | **Fundamental Time-Series** | 2, 7 | **M** | Quarterly fundamental snapshots over time (not just latest point). Historical P/E bands, margin trajectory. New `fundamental_history` table + fetch logic |
| **G7** | **Portfolio Optimisation (MVO / Risk Parity)** | 4 | **L** | Efficient frontier solver, minimum-variance portfolio, risk parity weights. Quadratic programming or iterative solver |
| **G8** | **ETF Screening & Holdings Data** | 5 | **M** | ETF metadata (ER, AUM, tracking error), constituent holdings for overlap analysis. New `EtfSnapshot` + data fetcher |
| **G9** | **Selling Programme / Liquidation Scheduler** | 6 | **M** | Multi-year selling schedule with lot selection, annual tax projection, pace rules linked to regime/technicals |
| **G10** | **Options Data & Pricing** | 2, 6 | **L** | Options chain fetcher, Black-Scholes pricer, collar designer, payoff diagrams. Finnhub has `/stock/option-chain` |

### Tier 3 — Enhancing (improves quality of advice)

| # | Gap | Phase | Effort | Description |
|---|---|---|---|---|
| **G11** | **Segment-Level Financials** | 2 | **M** | Revenue/margin by business segment (AWS, Ads, Retail). SEC EDGAR XBRL or Finnhub financial-statements-as-reported |
| **G12** | **Institutional Ownership / 13F** | 2 | **S** | Fetch institutional holders, ownership %, quarterly changes. Finnhub `/stock/ownership` |
| **G13** | **Factor Decomposition (Fama-French)** | 4 | **L** | Multi-factor regression: market, size, value, momentum, quality. Requires factor return series from Ken French Data Library |
| **G14** | **Forward Monte Carlo (Portfolio Projection)** | 4, 7 | **M** | Simulate portfolio paths over 10–30 years with rebalancing, withdrawals, inflation, tax drag. Goal-based success probability |
| **G15** | **Performance Attribution (Brinson)** | 7 | **M** | Allocation + selection + interaction effects. Requires portfolio weights + benchmark weights |
| **G16** | **IPS Document Generator** | 8 | **S** | Structured form → Markdown/PDF Investment Policy Statement |
| **G17** | **Decision Audit Log** | 8 | **S** | Manual log entries with rationale, linked to trades. Simple table + web form |
| **G18** | **Rebalancing Alert Engine** | 7 | **S** | Weight drift detection → alert via existing Telegram/Discord channels. Easy once portfolio weights exist |
| **G19** | **Direct Indexing Simulator** | 5 | **L** | Custom index replication, continuous TLH simulation, fee-adjusted comparison vs ETF |
| **G20** | **Quarterly Report Generator (HTML/PDF)** | 7 | **M** | Portfolio performance report extending existing `html_report.rs` template |

---

## What GhostAlpha Already Covers Well

These capabilities directly support the diversification research plan with little or no modification:

| Capability | Phase | Notes |
|---|---|---|
| **Technical analysis (Minervini, ADX, RSI, RS, volume)** | 2.3 | Full suite of momentum/trend indicators |
| **4 screener strategies** | 2.3 | Minervini, Breakout, MeanReversion, TrendStrength |
| **Market regime detection (Bull/Sideways/Bear)** | 2.3, 6 | SPY-based with breadth confirmation |
| **Sentiment analysis (FinBERT + news)** | 2 | News-driven sentiment scoring with time decay |
| **Fundamental snapshot (P/E, P/B, margins, growth)** | 2.2 | Single-point fundamentals with sector benchmarks |
| **Earnings tracking (beats, surprises, quality)** | 2.1 | Consecutive beat streaks, surprise %, growth |
| **Macro risk indicators (VIX, OAS, yield curve)** | 2.3, 6 | Input structure exists (data fetch is manual) |
| **Walk-forward backtesting + Monte Carlo** | 2.3 | Signal-level backtesting with bootstrap CI |
| **Exit signal detection** | 6 | Stage 2 breakdown, RS decay, death cross |
| **Alert engine (Telegram, Discord, Webhook)** | 7 | Multi-channel with dedup, rate limiting, quiet hours |
| **Web dashboard (9 pages, 14 htmx partials)** | All | Extensible framework for new pages/charts |
| **SQLite persistence (13 tables)** | All | Schema migration system, async-safe |
| **MCP integration** | All | Enables AI assistant access to screening data |
| **CLI with multiple output formats** | All | Table, JSON, CSV, JSONL, HTML |
| **Plugin architecture** | All | Extensible providers, screeners, channels |

---

## Recommended Implementation Roadmap

### Sprint 1 (Foundation) — ~2 weeks
- **G1** Portfolio module (positions, lots, import)
- **G2** Multi-asset ETF data via Yahoo

### Sprint 2 (Analysis Core) — ~2 weeks
- **G3** Correlation matrix engine
- **G6** Fundamental time-series storage
- **G5** DCF / valuation module (basic)

### Sprint 3 (Tax & Planning) — ~2 weeks
- **G4** Tax calculation engine
- **G9** Selling programme scheduler

### Sprint 4 (Optimisation) — ~2 weeks
- **G7** Portfolio optimisation (MVO)
- **G8** ETF screening & holdings
- **G14** Forward Monte Carlo

### Sprint 5 (Polish & Reporting) — ~1 week
- **G16** IPS generator
- **G17** Decision audit log
- **G18** Rebalancing alerts
- **G20** Quarterly report

### Deferred / External
- **G10** Options pricing (high complexity, consider external library)
- **G11** Segment financials (requires SEC EDGAR parser)
- **G13** Factor decomposition (academic, consider Python/R sidecar)
- **G15** Brinson attribution (needs portfolio + benchmark granularity)
- **G19** Direct indexing simulator (niche use case)

---

## External Tools That May Be Better Than Building

Some capabilities are better served by specialised external tools rather than building into GhostAlpha:

| Capability | Recommended External Tool | Reason |
|---|---|---|
| Tax lot management at scale | Addepar, Black Diamond, Orion | Brokerage integration, audit compliance, GAAP accounting |
| Estate planning (trusts, GRAT, IDGT) | Wealth.com, WealthCounsel | Legal document generation requires attorney review |
| Options pricing & strategy | OptionStrat, Thinkorswim, IBKR TWS | Real-time Greeks, vol surface, what-if |
| Factor regression | Python (statsmodels, scikit-learn) | Academic factor libraries are Python-native |
| Client risk profiling | Riskalyze, FinaMetrica | Validated psychometric questionnaires |
| Financial planning projections | MoneyGuidePro, eMoney Advisor | CFP-grade planning with tax integration |

GhostAlpha's strength is **quantitative equity screening, technical analysis, and signal generation**. The diversification use case extends it into **portfolio construction, tax optimisation, and financial planning** — adjacent but distinct domains. The recommended approach is to build the analytical core (correlation, valuation, optimisation) natively in Rust, while integrating with or deferring to specialised tools for tax/legal/estate and client-facing planning.
