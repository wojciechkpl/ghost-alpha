# Concentrated AMZN Position — Diversification Research Plan

> **Purpose:** Step-by-step research framework for a wealth manager advising a client
> with a substantial concentrated position in Amazon (AMZN) stock, seeking
> strategic diversification while optimising for tax efficiency, risk reduction,
> and long-term wealth preservation.

---

## Phase 1 — Client Discovery & Position Diagnostics

### 1.1 Full Financial Inventory

| Data Point | Why It Matters |
|---|---|
| Total net worth (liquid + illiquid) | Determines AMZN as % of total wealth, not just portfolio |
| Cost basis & lot-level detail | Drives every tax decision — which lots to sell, when, and how |
| Holding period per lot | Long-term vs short-term capital gains rates (0/15/20% vs ordinary income) |
| Unrealised gain/loss per lot | Quantifies the "tax drag" of selling — the real cost of diversification |
| Vesting schedule (if RSUs/options) | Future inflows increase concentration; must plan for shares not yet owned |
| Outstanding pledged/collateral shares | Margin loans or lines of credit secured by AMZN restrict sellable shares |
| Other income sources & tax bracket | Determines marginal rate, NIIT (3.8%) exposure, state tax implications |
| Estate plan & beneficiary structure | Step-up basis at death may change the optimal liquidation timeline |
| Liquidity needs (12–60 months) | Sets the floor for how much must be in cash or near-cash instruments |
| Risk tolerance (psychometric + revealed) | Calibrate drawdown tolerance — questionnaire plus actual behaviour in 2022 |

### 1.2 Concentration Risk Quantification

**Metrics to compute:**

- **AMZN weight** as percentage of total investable assets
- **Contribution to portfolio variance** — single-stock variance + covariance with remaining holdings
- **Maximum drawdown scenario** — AMZN's worst historical drawdown (−56% in 2022, −94% in 2000–2001) applied to current position
- **Dollar-at-risk** — VaR (95th/99th percentile, 1-year horizon) for the concentrated position vs a diversified benchmark
- **Herfindahl-Hirschman Index (HHI)** of the overall portfolio — quantifies how concentrated vs diversified
- **Correlation analysis** — AMZN vs existing other holdings to check if "diversification" already exists or is illusory

**Deliverable:** One-page "Concentration Risk Scorecard" showing current state, target state, and the gap.

---

## Phase 2 — AMZN Fundamental Deep Dive

Before recommending what to diversify *into*, evaluate what you are diversifying *away from*. The client must understand the forward-looking risk/reward of holding vs selling.

### 2.1 Business Quality Assessment

| Dimension | Key Questions | Data Sources |
|---|---|---|
| Revenue mix & trajectory | AWS vs Advertising vs Retail vs Other — growth rates, margins, TAM | 10-K segment reporting, earnings transcripts |
| Margin expansion path | Is AWS margin structurally improving? Retail reaching profitability at scale? | Quarterly filings, sell-side consensus models |
| Capital allocation | CapEx intensity (data centres), buyback programme, debt management | Cash flow statements, management guidance |
| Competitive moat | Network effects (marketplace), switching costs (AWS), scale advantages | Porter's Five Forces framework applied to each segment |
| Management & governance | CEO track record, board composition, insider ownership trends | Proxy statements, SEC Form 4 filings |
| Regulatory risk | Antitrust (FTC), EU Digital Markets Act, labour regulation | Legal filings, regulatory tracker, political risk assessment |

### 2.2 Valuation Framework

Run multiple valuation approaches and triangulate:

1. **DCF (3-stage)** — explicit forecast (5yr) → fade period (5yr) → terminal value. Sensitivity table on WACC (8–12%) and terminal growth (2–4%)
2. **Sum-of-the-Parts** — value AWS, Advertising, Retail, and Other segments independently using segment-appropriate multiples (EV/EBITDA for AWS vs EV/Revenue for Advertising)
3. **Relative valuation** — P/E, EV/EBITDA, P/FCF vs mega-cap tech peers (MSFT, GOOGL, META) and vs AMZN's own 5-year history
4. **Implied expectations** — reverse-engineer what growth rate the current price implies and assess if that is realistic
5. **Bear/Base/Bull scenarios** — assign probabilities, compute expected value

**Key output:** Is AMZN priced for perfection, fairly valued, or still offering upside? This directly affects how aggressively to diversify.

### 2.3 Technical & Momentum Check

- **Trend regime** — is AMZN in a Stage 2 uptrend (Weinstein/Minervini) or breaking down?
- **Relative strength** — AMZN vs S&P 500 over 3/6/12 months (use GhostAlpha RS percentile)
- **Institutional flows** — 13F aggregate positioning trend (accumulation vs distribution)
- **Options market** — implied volatility skew, put/call ratio, large block trades

> *Rationale:* Selling into strength is mechanically and psychologically easier than selling into weakness. Timing the diversification programme around technical context improves execution.

---

## Phase 3 — Tax & Legal Strategy Design

### 3.1 Tax Impact Modelling

For each potential selling scenario, model:

```
Scenario          | Shares Sold | Realised Gain | Federal Tax | State Tax | NIIT  | Net Proceeds
------------------|-------------|---------------|-------------|-----------|-------|-------------
Sell 10% (high basis lots) | ...    | ...           | ...         | ...       | ...   | ...
Sell 25% (HIFO)   | ...         | ...           | ...         | ...       | ...   | ...
Sell 50% (mixed)   | ...         | ...           | ...         | ...       | ...   | ...
Exchange Fund      | N/A         | Deferred      | Deferred    | ...       | N/A   | ...
Charitable transfer| ...         | Avoided       | Deduction   | ...       | N/A   | ...
```

**Tax-efficient techniques to evaluate:**

| Technique | Mechanism | Suitability Criteria |
|---|---|---|
| **Specific lot identification (HIFO)** | Sell highest-cost-basis lots first to minimise gain | Always applicable; foundational |
| **Tax-loss harvesting offsets** | Realise losses elsewhere to offset AMZN gains | Requires losers in portfolio; wash sale rules apply |
| **Charitable remainder trust (CRT)** | Transfer appreciated shares → income stream + charitable deduction | High-net-worth, philanthropic intent, illiquidity tolerance |
| **Donor-advised fund (DAF)** | Donate appreciated shares, get immediate deduction, avoid cap gains | Charitable intent; limited to 30% AGI for appreciated property |
| **Exchange fund (swap fund)** | Pool concentrated stock with other investors → receive diversified fund | Typically $5M+ minimum; 7-year lock-up; accredited investors only |
| **Qualified Opportunity Zone (QOZ)** | Defer gains by reinvesting in QOZ fund within 180 days | Suitable for real estate allocation; complex compliance |
| **10b5-1 plan** | Pre-scheduled selling programme (if insider/restricted) | Eliminates timing decisions; provides legal safe harbour |
| **Protective collar / prepaid variable forward** | Options-based hedge: buy puts + sell calls to lock in a range | Monetises without immediate sale; complex tax (constructive sale risk) |
| **Gifting (annual/lifetime)** | Transfer shares to family members in lower brackets | $18K/yr annual exclusion; $13.6M lifetime exemption (2024) |
| **Step-up basis at death** | Hold until death → heirs receive at FMV, gains erased | Only appropriate if client has no near-term need and long horizon |

### 3.2 State Tax Considerations

- Identify state of residence and applicable state capital gains rate
- Evaluate if relocation or trust domicile in a no-income-tax state is viable and desired
- Consider instalment sales to spread recognition across tax years

### 3.3 Estate & Succession Planning

- **Grantor Retained Annuity Trust (GRAT)** — transfer appreciation to heirs gift-tax-free
- **Intentionally Defective Grantor Trust (IDGT)** — sell AMZN to trust in exchange for a note; freeze estate value
- **Family Limited Partnership (FLP)** — transfer shares at valuation discount
- **Generation-skipping trusts** — for multi-generational wealth transfer

**Deliverable:** Tax strategy memo from CPA/tax attorney with projected after-tax proceeds for each scenario.

---

## Phase 4 — Target Allocation Design

### 4.1 Strategic Asset Allocation (SAA)

Define the long-term target portfolio using Modern Portfolio Theory as a starting point, then adjust for the client's specific situation.

**Asset classes to evaluate:**

| Asset Class | Role in Portfolio | Evaluation Criteria |
|---|---|---|
| **US Large-Cap Equity** | Core growth + income | Sector diversification away from tech; dividend sustainability |
| **US Mid/Small-Cap** | Higher growth potential, less correlated to mega-cap | Factor exposure (value, quality, momentum); liquidity |
| **International Developed** | Geographic diversification | Currency hedging decision; valuation discount to US |
| **Emerging Markets** | Long-term growth, low correlation | Political risk, governance, currency volatility |
| **US Investment-Grade Bonds** | Stability, income, deflation hedge | Duration management; credit quality; real yield |
| **TIPS / I-Bonds** | Inflation protection | Break-even inflation rate analysis |
| **High-Yield / Bank Loans** | Income enhancement | Credit cycle positioning; default rate expectations |
| **Real Estate (REITs / Direct)** | Inflation hedge, income, diversification | Cap rates, occupancy, sector (industrial vs office vs residential) |
| **Private Equity** | Illiquidity premium, uncorrelated returns | J-curve, vintage year diversification, fee drag |
| **Private Credit** | Yield enhancement | Floating rate benefit in rising rate environment |
| **Commodities / Gold** | Tail risk hedge, inflation protection | Storage costs, roll yield, correlation in crisis periods |
| **Alternatives (Hedge Funds)** | Absolute return, downside protection | Strategy selection (L/S equity, global macro, trend-following) |
| **Cash / T-Bills** | Liquidity, optionality | Opportunity cost vs safety |

### 4.2 Factor & Correlation Analysis

For each candidate asset class/fund:

1. **Historical correlation matrix** — vs AMZN specifically (not just vs "US equity")
2. **Rolling correlation** — 3-year rolling windows to detect regime changes
3. **Crisis correlation** — behaviour during 2008, 2020 COVID, 2022 rate shock (correlations converge to 1 in crises)
4. **Factor decomposition** — what systematic factors (market, size, value, momentum, quality, low-vol) drive returns?
5. **Tail risk analysis** — maximum drawdown, Sortino ratio, conditional VaR

### 4.3 Model Portfolio Construction

Build 3 candidate portfolios reflecting different risk profiles:

| Component | Conservative | Moderate | Growth |
|---|---|---|---|
| AMZN (retained) | 10–15% | 15–25% | 25–35% |
| US Equity (ex-AMZN) | 15–20% | 20–25% | 25–30% |
| International Equity | 10–15% | 10–15% | 10–15% |
| Fixed Income | 30–35% | 15–20% | 5–10% |
| Real Assets | 10–15% | 10–15% | 10–15% |
| Alternatives | 0–5% | 5–10% | 5–10% |
| Cash | 5–10% | 3–5% | 2–3% |

**For each portfolio, compute:**

- Expected return (geometric, after fees and taxes)
- Expected volatility (annualised)
- Sharpe ratio
- Maximum drawdown (historical simulation)
- Probability of meeting client's return target
- Tax cost of transition from current state

---

## Phase 5 — Implementation Vehicle Selection

### 5.1 Vehicle Evaluation Matrix

For each asset class allocation, select the optimal vehicle:

| Criteria | Individual Stocks | ETFs | Mutual Funds | SMAs | Private Funds |
|---|---|---|---|---|---|
| Tax efficiency | High (lot control) | High (in-kind redemptions) | Low (cap gain distributions) | High (tax-loss harvesting) | Varies (K-1 complexity) |
| Cost | $0 commission | 0.03–0.50% ER | 0.10–1.00% ER | 0.20–0.50% wrap | 1–2% + 20% carry |
| Customisation | Full | None | None | High (ESG screens, etc.) | Limited |
| Minimum investment | Low | Low | Low–Medium | $100K–$500K | $250K–$5M+ |
| Liquidity | Daily | Daily | Daily | Daily | Quarterly–Annual |
| Complexity | High | Low | Low | Medium | High |

### 5.2 Specific Fund/ETF Due Diligence Checklist

For each candidate fund:

- [ ] Expense ratio vs category median
- [ ] Tracking error (for index funds) or alpha generation (for active)
- [ ] Tax cost ratio (Morningstar) — after-tax return drag
- [ ] Fund size / AUM — too small risks closure; too large risks capacity constraints
- [ ] Manager tenure and track record (for active)
- [ ] Holdings overlap — avoid duplicating AMZN exposure in "diversifying" funds (S&P 500 index has ~3.5% AMZN)
- [ ] Securities lending revenue (offsets fees in some ETFs)
- [ ] Capital gains overhang (for mutual funds — embedded unrealised gains that could be distributed)

### 5.3 Direct Indexing Consideration

For the US equity sleeve, evaluate **direct indexing** as an alternative to ETFs:

- Own individual stocks replicating an index (S&P 500 ex-AMZN, or custom)
- **Advantages:** Continuous tax-loss harvesting, exclude AMZN to avoid overlap, ESG/values customisation
- **Disadvantages:** Higher management fee (0.20–0.40%), requires SMA platform, rebalancing complexity
- **Break-even analysis:** At what unrealised gain level does the tax alpha from direct indexing exceed the fee premium?

---

## Phase 6 — Transition Plan & Execution

### 6.1 Liquidation Schedule

Design a multi-year selling programme to spread tax impact:

```
Year 1:  Sell [X]% of AMZN — highest basis lots first
         → Redeploy into [bonds, international, REITs]
         → Estimated tax: $___

Year 2:  Sell [X]% — next tranche
         → Redeploy into [remaining equity diversification]
         → Harvest losses from Year 1 positions if available
         → Estimated tax: $___

Year 3+: Continue until target allocation reached
         → Reassess annually based on AMZN price, tax law changes, client needs
```

**Decision rules for acceleration/deceleration:**

- **Accelerate selling** if: AMZN breaks below 200-day MA, sector faces regulatory action, client liquidity needs increase, tax law changes pending (rate increases)
- **Decelerate selling** if: AMZN in strong uptrend with positive catalysts, tax-loss harvesting opportunities reduce effective rate, market-wide correction creates better entry points for target allocations

### 6.2 Hedging During Transition

While the selling programme executes over multiple years, the remaining concentrated position needs risk management:

| Hedge | Cost | Protection Level | Tax Impact |
|---|---|---|---|
| **Protective puts** (3–6 month rolling) | 2–5% annualised premium | Hard floor at strike | Premium is non-deductible; exercise triggers sale |
| **Costless collar** (buy put, sell call) | Zero premium (cap upside) | Floor + ceiling | Constructive sale risk if too tight |
| **Put spread collar** | Reduced cost, partial protection | Partial floor + ceiling | Lower constructive sale risk |
| **Inverse/short sector ETF** | Basis risk vs AMZN | Imperfect but liquid | Short-term gains on hedge |
| **Structured notes** | Issuer credit risk | Customisable payoff | Ordinary income treatment |

### 6.3 Cash Management During Transition

- Park proceeds in T-bills / money market (currently yielding ~5%)
- Dollar-cost average into target allocations over 3–6 month windows to reduce timing risk
- Maintain liquidity reserve equal to 12–24 months of client's spending needs

---

## Phase 7 — Ongoing Monitoring & Rebalancing

### 7.1 Monitoring Framework

| Metric | Frequency | Trigger for Action |
|---|---|---|
| AMZN weight in portfolio | Weekly | Drifts >5% above target → sell; >5% below → pause selling |
| Overall portfolio HHI | Monthly | Must trend toward target diversification level |
| Tax-loss harvesting opportunities | Daily (automated) | Harvest losses >$1,000 with no wash sale conflict |
| Correlation regime | Quarterly | If AMZN correlation to rest of portfolio spikes, accelerate diversification |
| Fundamental thesis review | Quarterly (post-earnings) | Material deterioration in AWS growth or margin → accelerate exit |
| Client life events | As needed | Divorce, retirement, home purchase, inheritance → reassess plan |
| Tax law changes | As announced | Proposed rate increases → accelerate; decreases → slow down |
| Estate plan review | Annually | Update trust structures, beneficiary designations |

### 7.2 Rebalancing Policy

- **Calendar rebalancing:** Review quarterly, rebalance if any asset class drifts >5% from target
- **Threshold rebalancing:** Immediate rebalance if any position exceeds 2x target weight
- **Tax-aware rebalancing:** Prefer rebalancing via new cash flows (dividends, additional savings) over selling to minimise tax friction
- **Band-based approach:** Allow wider bands (e.g., +/- 7%) for tax-inefficient positions, tighter bands (+/- 3%) for tax-efficient ones

### 7.3 Performance Reporting

Quarterly report including:

- Portfolio return vs blended benchmark (weighted by target allocation)
- Tax alpha generated (value of tax-loss harvesting and lot selection)
- Diversification progress: AMZN weight trajectory vs plan
- Risk metrics: portfolio volatility, drawdown, Sharpe ratio
- Forward-looking: upcoming tax events, rebalancing needs, macro outlook

---

## Phase 8 — Behavioural & Governance Considerations

### 8.1 Behavioural Biases to Manage

| Bias | Risk | Mitigation |
|---|---|---|
| **Endowment effect** | Overvaluing AMZN because "it's mine" / emotional attachment | Frame as "would you buy this much AMZN today with cash?" |
| **Anchoring** | Waiting for AMZN to hit a specific price before selling | Pre-commit to systematic schedule, not price targets |
| **Regret aversion** | Fear of selling before a further rally | Show historical cost of concentration vs diversification |
| **Status quo bias** | Inertia — doing nothing feels safer than acting | Automate the selling programme (10b5-1 or advisor-directed) |
| **Recency bias** | If AMZN recently outperformed, reluctance to sell | Show full-cycle data including 2000-2001 (-94%) and 2022 (-56%) |
| **Home bias** | Reluctance to add international exposure | Present data on US vs international valuations and long-term cycles |

### 8.2 Governance Structure

- **Investment Policy Statement (IPS):** Formalise target allocation, constraints, rebalancing rules, and risk limits in a signed document
- **Decision rights:** Clarify who authorises trades (client, advisor, discretionary mandate)
- **Review cadence:** Quarterly portfolio review meetings; annual comprehensive plan review
- **Documentation:** Every trade rationale documented for tax audit trail and fiduciary record

---

## Appendix A — Research Tools & Data Sources

| Category | Tools / Sources |
|---|---|
| Fundamental data | SEC EDGAR (10-K, 10-Q, proxy), Bloomberg, FactSet, S&P Capital IQ |
| Valuation models | Custom DCF spreadsheet, Aswath Damodaran's templates, sell-side consensus |
| Technical analysis | GhostAlpha (RS percentile, regime detection), TradingView, Bloomberg |
| Tax modelling | CPA collaboration, tax lot accounting software (Addepar, Black Diamond) |
| Portfolio construction | Mean-variance optimisation (Python/R), Monte Carlo simulation, risk parity models |
| Fund screening | Morningstar Direct, ETF.com, fund prospectuses |
| Alternatives due diligence | Preqin (PE/VC data), CAIA curriculum frameworks, manager DDQs |
| Behavioural finance | Kahneman & Tversky frameworks, client risk profiling tools (Riskalyze, FinaMetrica) |
| Estate planning | Collaboration with estate attorney; Wealth.com or similar trust management platform |

## Appendix B — Key Risk Scenarios to Stress Test

| Scenario | AMZN Impact | Portfolio Impact | Action |
|---|---|---|---|
| AWS growth decelerates to <15% | -20 to -30% | Depends on weight | Accelerate diversification |
| Broad tech selloff (2022 repeat) | -40 to -50% | Severe if concentrated | Hedges activate; pause selling at lows |
| Antitrust forced breakup | -15 to -25% (uncertainty) or +10% (sum > whole) | Varies | Re-evaluate SOTP value |
| Rising rates / recession | -20 to -35% | Bonds cushion in moderate/conservative | Rebalance into equities at lower prices |
| Inflation spike | -10 to -20% (multiple compression) | TIPS, commodities, real assets hedge | Verify inflation hedge allocations |
| Client health emergency | Forced liquidation at worst time | Liquidity reserve critical | Maintain 12–24 month cash buffer |
| Tax rate increase legislation | Higher cost of future sales | Accelerate selling before effective date | Monitor Congressional calendar |

---

*This plan should be executed in close collaboration with the client's CPA (tax strategy),
estate attorney (trust structures), and insurance advisor (life insurance for estate liquidity).
No trades should be executed until the full discovery phase is complete and the Investment Policy
Statement is signed by all parties.*
