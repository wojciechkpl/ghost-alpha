# Product Requirements Document
# GhostAlpha — Concentrated Stock Position Diversification Engine

**Version:** 1.0  
**Date:** March 20, 2026  
**Status:** Draft — Ready for Engineering Review  
**Author:** AI-Assisted Wealth Strategy Team

---

## Table of contents

1. Executive summary
2. Problem statement
3. System architecture
4. Module 1: Position analyzer
5. Module 2: Market data collector
6. Module 3: Tax optimizer
7. Module 4: Allocation builder
8. Module 5: Geopolitical risk overlay
9. Module 6: Execution planner
10. Module 7: Visualization layer
11. Module 8: Report generator
12. Data models and schemas
13. API design
14. Visualization specifications
15. Technology stack recommendations
16. Testing strategy
17. Deployment and CI/CD
18. Future roadmap

---

## 1. Executive summary

GhostAlpha is a software system that automates the analysis and planning required to safely diversify out of a concentrated single-stock position. It takes raw brokerage position data as input and produces a tax-optimized, macro-aware, platform-specific execution plan as output.

The system was designed around the common challenge of concentrated single-stock positions — typical of long-tenured tech employees with RSU/ESPP accumulation. The reference scenario used during design was a hypothetical portfolio with ~75% concentration in a single large-cap tech stock across ~20 tax lots. The methodology generalizes to any single-stock concentration on any major brokerage platform.

### What the system produces

- Prioritized sell order across all tax lots with phase assignments
- Multi-year tax scenario comparison (1-year vs. 2-year vs. N-year spread)
- Target allocation mapped to platform-native zero/low-cost funds
- Geopolitical risk overlay with conditional deployment triggers
- Interactive dashboard with real-time monitoring
- Professional PDF/HTML report for client delivery

### Key metrics from the prototype analysis

- 19 tax lots analyzed and sequenced in under 2 seconds
- 5-year tax scenario comparison identified $14,065 in savings from optimal spreading
- Allocation builder generated Fidelity-specific fund map with 0.34% weighted expense ratio
- Oil crisis overlay automatically shifted energy from 5% → 12% and EM from 8% → 4%

---

## 2. Problem statement

### The user

A retail investor or wealth advisor managing a portfolio where a single stock represents >25% of total value. They need to reduce concentration risk while minimizing tax impact, accounting for current market conditions, and deploying proceeds into appropriate diversified positions.

### Pain points solved

1. **Manual lot analysis is error-prone.** Investors with 10-50 tax lots struggle to determine optimal sell order. A wrong sequence can cost thousands in unnecessary taxes.
2. **Tax threshold awareness requires expertise.** Federal LTCG brackets, NIIT, and state-specific rules (WA's 7%/9.9% CG tax, CA's 13.3% income tax) interact in complex ways.
3. **Market conditions change the plan.** A diversification plan designed in January 2026 became dangerously wrong after the Iran war started February 28. Static plans fail.
4. **Platform lock-in ignorance.** Investors don't know that Fidelity ZERO funds can't transfer in-kind, or that FNILX and FXAIX are valid TLH pairs. Platform-specific knowledge is critical.
5. **No integrated view.** Currently, an investor needs separate tools for lot analysis, tax modeling, fund screening, and macro research. GhostAlpha unifies them.

### Success criteria

- Reduce analysis time from ~8 hours (manual) to <5 minutes (automated)
- Tax savings identified should exceed $5,000 for portfolios above $500K
- Plan should be adaptable to geopolitical shocks within 24 hours
- Output should be actionable: specific tickers, specific lots, specific dates

---

## 3. System architecture

### High-level architecture

```
┌─────────────────────────────────────────────────────────┐
│                    GhostAlpha Engine                     │
│                                                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────┐ │
│  │ Position  │→│  Market   │→│   Tax    │→│ Alloc  │ │
│  │ Analyzer  │  │  Data    │  │Optimizer │  │Builder │ │
│  └──────────┘  └──────────┘  └──────────┘  └────────┘ │
│       ↓              ↓             ↓            ↓       │
│  ┌──────────────────────────────────────────────────┐  │
│  │           Geopolitical Risk Overlay               │  │
│  └──────────────────────────────────────────────────┘  │
│       ↓                                                 │
│  ┌──────────┐  ┌──────────┐  ┌──────────────────────┐ │
│  │Execution │→│  Report   │→│   Visualization      │ │
│  │ Planner  │  │Generator │  │   Dashboard          │ │
│  └──────────┘  └──────────┘  └──────────────────────┘ │
└─────────────────────────────────────────────────────────┘
         ↑                              ↓
   ┌───────────┐               ┌──────────────┐
   │ Brokerage │               │  Client-     │
   │ CSV/API   │               │  Facing      │
   │ Import    │               │  Output      │
   └───────────┘               └──────────────┘
```

### Data flow

```
positions.csv → PositionAnalyzer → enriched_lots[]
                                        ↓
yfinance/APIs  → MarketDataCollector → market_snapshot{}
                                        ↓
enriched_lots + market_snapshot → TaxOptimizer → tax_scenarios{}
                                        ↓
tax_scenarios + risk_profile → AllocationBuilder → target_allocation{}
                                        ↓
target_allocation + geopolitical_data → GeoOverlay → adjusted_allocation{}
                                        ↓
adjusted_allocation + enriched_lots → ExecutionPlanner → phased_plan{}
                                        ↓
phased_plan → ReportGenerator → PDF/HTML/Interactive Dashboard
```

---

## 4. Module 1: Position analyzer

### Purpose

Parses raw brokerage position data, classifies each tax lot, computes optimal sell priority, and assigns phase timeline.

### Input schema

```typescript
interface PositionInput {
  acquired: string;        // ISO date or "MMM-DD-YYYY"
  term: "Short" | "Long";
  quantity: number;
  avg_cost_basis: number;  // Per share
  cost_basis_total: number;
  current_value: number;
  gain_loss_dollar: number;
  gain_loss_pct: number;
}
```

### CSV format (brokerage export)

```csv
acquired,term,quantity,avg_cost_basis,cost_basis_total,current_value,gain_loss_dollar,gain_loss_pct
2026-02-23,Short,43,$204.91,$8811.29,$9253.60,+$442.31,+5.02
2025-11-17,Short,736,$231.49,$170373.92,$158387.20,-$11986.72,-7.04
2024-02-15,Long,670,$169.14,$113326.82,$144184.00,+$30857.18,+27.23
```

### Output schema

```typescript
interface EnrichedLot extends PositionInput {
  sell_priority: number;        // 1 = sell first
  priority_reason: string;      // Human-readable explanation
  phase: string;                // "Phase 1 (Week 1)", "Phase 2 (Month 7-12)", etc.
  estimated_tax_per_100k: number; // Tax cost per $100K of proceeds sold
  days_held: number;
  days_to_ltcg: number | null;  // Days until short-term becomes long-term
}

interface PositionAnalysis {
  summary: {
    total_shares: number;
    total_current_value: number;
    total_cost_basis: number;
    total_unrealized_gain: number;
    weighted_avg_gain_pct: number;
    concentration_pct: number;
  };
  lot_classification: {
    short_term_loss_lots: number;
    short_term_gain_lots: number;
    long_term_gain_lots: number;
    harvestable_st_losses: number;
    total_lt_gains: number;
  };
  sell_order: EnrichedLot[];  // Sorted by sell_priority
  tax_plan: MultiYearTaxPlan;
}
```

### Priority algorithm

```python
def classify_and_prioritize(lots, thresholds):
    """
    Sell priority (lower = sell first):
    
    Priority 1-N: Short-term LOSS lots
      - Sorted by most negative first
      - Reason: Losses offset gains at ordinary income rates (up to 40.8%)
      - Phase: "Week 1" — sell immediately
      - Reinvest into broad index fund (not substantially identical = no wash sale)
    
    Priority N+1-M: Short-term GAIN lots  
      - Check days_to_ltcg: if < 60 days, flag for potential hold
      - Large lots (>$50K) should sell regardless if stock has downside risk
      - Small lots (<$10K) near anniversary should hold for LTCG conversion
      - Phase: "Month 1-3"
    
    Priority M+1-end: Long-term gain lots, sorted ascending by gain_loss_pct
      - +41% lot generates 69% less taxable gain than +133% lot per dollar sold
      - Lowest % gain first → highest % gain last
      - Highest-gain lots are candidates for charitable donation (DAF)
      - Phase: distributed across Phases 1-3 based on gain level
    """
```

### Key code snippet — tax cost estimation

```python
def estimate_tax_per_100k(lot: TaxLot) -> float:
    """Estimate tax on $100K of proceeds from selling this lot."""
    if lot.term == "Short" and lot.gain_loss_dollar < 0:
        return 0  # Loss = tax benefit
    
    if lot.term == "Short":
        # Short-term gains taxed at ordinary income rates
        gain_ratio = lot.gain_loss_pct / 100
        return gain_ratio * 100_000 * 0.408  # 37% + 3.8% NIIT worst case
    
    # Long-term: portion of proceeds that is gain
    effective_gain_ratio = lot.gain_loss_pct / (100 + lot.gain_loss_pct)
    return effective_gain_ratio * 100_000 * 0.188  # 15% + 3.8% NIIT
```

---

## 5. Module 2: Market data collector

### Purpose

Pulls real-time and historical data for the concentrated stock, macro indicators, sector rotation, and geopolitical risk proxies. Provides the context layer for all downstream decisions.

### Data sources

| Data category | Primary source | Backup source | API type |
|---|---|---|---|
| Stock fundamentals | yfinance | Alpha Vantage | REST / Python lib |
| Analyst ratings | yfinance `.info` | StockAnalysis.com (scrape) | REST |
| Macro indicators | FRED API | Trading Economics | REST (free key) |
| Sector performance | yfinance sector ETFs | Schwab sector outlook | REST |
| Oil/commodities | yfinance futures | EIA API | REST |
| Geopolitical proxy | VIX + Oil + Gold + DXY | Bloomberg BI (paid) | Computed |
| EM currency stress | yfinance FX pairs | FRED EM dollar index | REST |

### Output schema

```typescript
interface MarketSnapshot {
  ticker: string;
  collected_at: string;  // ISO timestamp
  
  fundamentals: {
    price: { current, previous_close, 52w_high, 52w_low, market_cap };
    valuation: { trailing_pe, forward_pe, peg_ratio, price_to_book, ev_to_ebitda };
    earnings: { trailing_eps, forward_eps, revenue_ttm, revenue_growth, 
                profit_margin, operating_margin, free_cash_flow };
    analyst: { recommendation, target_mean, target_median, target_high, 
               target_low, num_analysts, implied_upside_pct };
  };
  
  volatility: {
    annualized_volatility: number;  // % — stock vs. SPY
    max_drawdown_pct: number;
    ytd_return_pct: number;
  };
  
  macro: {
    sp500: { current, 1m_change_pct, 6m_change_pct };
    vix: { current, signal: "low" | "normal" | "elevated" | "high" };
    treasury_10y: { current, 1m_change_pct };
    oil_wti: { current, 1m_change_pct };
    oil_brent: { current, 1m_change_pct };
    usd_index: { current, 1m_change_pct };
    yield_curve_spread: { value, signal: "inverted" | "flat" | "normal" };
  };
  
  sectors: {
    [sectorName: string]: {
      etf: string;
      current_price: number;
      "1m_return_pct": number;
      "3m_return_pct": number;
      "6m_return_pct": number;
    };
    rotation_signal: {
      leading_sectors: string[];
      lagging_sectors: string[];
      regime: "pro-cyclical" | "defensive" | "inflationary" | "mixed";
    };
  };
  
  geopolitical: {
    overall_risk: "NORMAL" | "ELEVATED" | "HIGH";
    oil_signal: "NORMAL" | "ELEVATED" | "CRISIS";
    safe_haven_bid: boolean;  // Gold + USD both surging
    vix_regime: string;
  };
  
  concentration_risk: {
    stock_annual_volatility: number;
    sp500_annual_volatility: number;
    volatility_multiplier: number;  // e.g., 2.2x
  };
}
```

### Key code snippet — sector rotation classifier

```python
def classify_rotation_regime(ranked_sectors: list) -> str:
    """
    Determine market regime from sector leadership.
    Input: list of (sector_name, 3m_return_pct) tuples, sorted descending.
    """
    leaders = {name for name, _ in ranked_sectors[:4]}
    
    defensive = {"Utilities", "Healthcare", "Consumer Staples", "Real Estate"}
    cyclical = {"Technology", "Consumer Discretionary", "Financials", "Industrials"}
    
    defensive_count = len(leaders & defensive)
    cyclical_count = len(leaders & cyclical)
    
    if cyclical_count >= 3:
        return "pro-cyclical"      # Risk-on: growth/tech leading
    elif defensive_count >= 3:
        return "defensive"         # Risk-off: flight to safety
    elif "Energy" in leaders:
        return "inflationary"      # Energy-led, possible stagflation
    else:
        return "mixed"
```

### Key code snippet — geopolitical risk scoring

```python
def score_geopolitical_risk(macro: dict) -> dict:
    """
    Derive geopolitical risk level from market proxies.
    No news API needed — markets price in risk faster than headlines.
    """
    oil = macro.get("oil_brent", {}).get("current", 70)
    vix = macro.get("vix", {}).get("current", 15)
    gold_change = macro.get("gold", {}).get("3m_change_pct", 0)
    usd_change = macro.get("usd_index", {}).get("3m_change_pct", 0)
    
    risk_score = 0
    
    # Oil thresholds (calibrated from 2026 Iran crisis)
    if oil > 120: risk_score += 3    # CRISIS
    elif oil > 90: risk_score += 2   # ELEVATED  
    elif oil > 80: risk_score += 1   # WARNING
    
    # VIX thresholds
    if vix > 35: risk_score += 3
    elif vix > 25: risk_score += 2
    elif vix > 20: risk_score += 1
    
    # Safe haven bid (gold + USD both up = flight from risk)
    if gold_change > 5 and usd_change > 2:
        risk_score += 2
    
    if risk_score >= 6:
        return {"level": "HIGH", "action": "accelerate_selling"}
    elif risk_score >= 3:
        return {"level": "ELEVATED", "action": "apply_oil_crisis_overlay"}
    else:
        return {"level": "NORMAL", "action": "proceed_as_planned"}
```

---

## 6. Module 3: Tax optimizer

### Purpose

Models multi-year tax scenarios for selling a concentrated position. Compares lump-sum vs. spread strategies, accounting for federal LTCG brackets, NIIT, and state-specific capital gains taxes. Identifies the optimal number of tax years to spread sales across.

### Supported states (initial release)

| State | CG treatment | Key thresholds |
|---|---|---|
| WA | Separate 7%/9.9% CG tax | $278K deduction; 9.9% above $1M |
| CA | CG taxed as ordinary income | Up to 13.3% |
| NY | CG taxed as ordinary income | Up to 10.9% + NYC 3.876% |
| TX | No state CG tax | — |
| FL | No state CG tax | — |

### Core algorithm

```python
def calculate_federal_ltcg_tax(gains, other_income, filing_status):
    """
    Federal long-term capital gains tax calculation.
    
    2026 brackets (MFJ):
      0%:  $0 — $98,900
      15%: $98,900 — $613,700
      20%: Above $613,700
    
    Plus NIIT 3.8% on investment income when MAGI > $250K (MFJ)
    
    Key insight: gains "stack" on top of ordinary income.
    If ordinary income is $200K, the first $98,900 - $200K = $0 
    of gains gets 0% rate. The 15% bracket starts immediately.
    """
    brackets = [
        (0, 98_900, 0.00),
        (98_900, 613_700, 0.15),
        (613_700, float("inf"), 0.20),
    ]
    
    tax = 0
    remaining = gains
    cursor = other_income  # Start from where ordinary income ends
    
    for lower, upper, rate in brackets:
        if cursor >= upper:
            continue
        start = max(lower, cursor)
        space = upper - start
        taxable = min(remaining, space)
        if taxable > 0:
            tax += taxable * rate
            remaining -= taxable
            cursor += taxable
        if remaining <= 0:
            break
    
    # NIIT: 3.8% on lesser of (net investment income) or (MAGI - threshold)
    threshold = 250_000  # MFJ
    magi = other_income + gains
    niit = max(0, min(gains, magi - threshold)) * 0.038
    
    return {"ltcg_tax": tax, "niit": niit, "total": tax + niit}
```

### Key code snippet — multi-year comparison

```python
def compare_scenarios(total_lt_gains, state, filing, max_years=5):
    """
    Compare tax liability across 1 to N year spreading strategies.
    
    Returns the optimal strategy and savings vs. worst case.
    
    Example output for $375K LT gains in WA (MFJ):
      1 year:  $68,543 tax (18.9% effective) 
      2 years: $56,880 tax (15.7% effective)
      3 years: $54,479 tax (15.0% effective)  ← OPTIMAL
      4 years: $54,479 tax (15.0% effective)
      5 years: $54,479 tax (15.0% effective)
      
    Savings: $14,065 by spreading over 3 years vs. lump sum
    
    The diminishing returns plateau happens when annual gains
    drop below the state deduction threshold ($278K for WA).
    """
    results = {}
    for n in range(1, max_years + 1):
        annual_gain = total_lt_gains / n
        fed = calculate_federal_ltcg_tax(annual_gain, 100_000, filing)
        state_tax = calculate_state_cg_tax(annual_gain, state)
        year_tax = fed["total"] + state_tax
        results[n] = {
            "annual_gain": annual_gain,
            "tax_per_year": year_tax,
            "total_tax": year_tax * n,
            "effective_rate": (year_tax * n) / total_lt_gains * 100,
        }
    
    optimal = min(results.items(), key=lambda x: x[1]["total_tax"])
    return results, optimal
```

### Washington state special handling

```python
def calculate_wa_cg_tax(net_lt_gains: float) -> float:
    """
    Washington State capital gains tax (effective 2022, expanded 2025+).
    
    - Applies ONLY to long-term capital gains
    - $278,000 standard deduction for MFJ (2026)
    - 7% on gains above deduction, up to $1M
    - 9.9% on gains above $1M
    - $111,000 charitable deduction available
    - Short-term gains are EXEMPT from WA CG tax
    
    Critical planning insight: keeping net LTCG below $278K/year
    results in ZERO Washington state tax. For a $375K total gain,
    spreading over 2 years saves ~$6,800 in state tax alone.
    """
    if net_lt_gains <= 278_000:
        return 0.0
    
    taxable = net_lt_gains - 278_000
    
    if taxable <= (1_000_000 - 278_000):  # Under $1M total
        return taxable * 0.07
    else:
        tier1 = (1_000_000 - 278_000) * 0.07
        tier2 = (taxable - (1_000_000 - 278_000)) * 0.099
        return tier1 + tier2
```

---

## 7. Module 4: Allocation builder

### Purpose

Generates a target portfolio allocation based on risk profile, investment timeline, and brokerage platform. Maps each allocation category to specific fund tickers with expense ratios and TLH pairs.

### Risk profile templates

```python
TEMPLATES = {
    "aggressive": {
        "retained_stock":          (5, 8),   # Keep some emotional anchor
        "us_broad_market":         (12, 15),
        "growth_thematic":         (12, 15), # AI, semis, innovation
        "small_cap":               (8, 10),
        "international_developed": (8, 10),
        "emerging_markets":        (4, 8),
        "bonds_fixed_income":      (0, 3),
        "sector_bets":             (15, 20), # Energy, biotech, cyber, defense
        "cash_money_market":       (5, 10),
        "alternatives":            (3, 5),   # Crypto
    },
    # moderate, conservative, growth templates also defined...
}
```

### Platform fund mapping (Fidelity)

```python
FIDELITY_FUNDS = {
    "us_broad_market": [
        {
            "ticker": "FZROX",
            "name": "Fidelity ZERO Total Market",
            "er": 0.00,
            "notes": "Zero cost; 2,500+ stocks; NOT transferable to other brokerages",
            "tlh_partner": "FSKAX",  # Different index = not substantially identical
            "min_investment": 0,
        },
        {
            "ticker": "FSKAX", 
            "name": "Fidelity Total Market Index",
            "er": 0.015,
            "notes": "Ultra-low cost; transferable; TLH partner to FZROX",
            "tlh_partner": "FZROX",
        },
        {
            "ticker": "FNILX",
            "name": "Fidelity ZERO Large Cap",
            "er": 0.00,
            "notes": "S&P 500 proxy (different index); TLH partner to FXAIX",
            "tlh_partner": "FXAIX",
        },
    ],
    "growth_thematic": [
        {
            "ticker": "FSELX",
            "name": "Fidelity Select Semiconductors",
            "er": 0.62,
            "notes": "Bloomberg #1 stock picker 3/4 years; ~25% NVDA; top Fidelity fund",
        },
        {
            "ticker": "FSPGX",
            "name": "Fidelity Large Cap Growth Index",
            "er": 0.035,
            "notes": "Passive large-cap growth",
        },
    ],
    "sector_bets": {
        "energy":       [{"ticker": "XLE", "name": "Energy Select Sector SPDR", "er": 0.09}],
        "biotech":      [{"ticker": "FBIOX", "name": "Fidelity Select Biotech", "er": 0.64}],
        "cybersecurity": [{"ticker": "CIBR", "name": "First Trust Cybersecurity", "er": 0.59}],
        "defense":      [{"ticker": "ITA", "name": "iShares Aerospace & Defense", "er": 0.40}],
    },
    "direct_indexing": {
        "product": "Fidelity Managed FidFolios",
        "fee": 0.40,  # Annual advisory fee
        "minimum": 5000,
        "tlh_frequency": "daily",  # Scans for TLH opportunities daily
        "can_exclude_stocks": True,  # Exclude AMZN to avoid wash sale
    },
}
```

### Geopolitical overlay engine

```python
def apply_geopolitical_overlay(allocation: dict, overlay_type: str) -> dict:
    """
    Adjusts allocation based on current geopolitical conditions.
    
    Overlay types:
      "oil_crisis"      — Iran/Hormuz scenario (active as of March 2026)
      "recession_risk"  — Yield curve inversion + rising unemployment
      "trade_war"       — Tariff escalation
      "none"            — Standard allocation
    
    The overlay is additive: it shifts percentages between categories
    while keeping the total at 100%.
    """
    OVERLAYS = {
        "oil_crisis": {
            "sector_bets":             +7,   # More energy + defense
            "cash_money_market":       +5,   # Larger dry powder buffer
            "emerging_markets":        -4,   # EM oil importers crushed
            "growth_thematic":         -3,   # High-growth faces margin pressure
            "international_developed": -2,   # Europe exposed to gas prices
            "small_cap":               -2,   # Most rate-sensitive
            "alternatives":            -1,   # Reduce risk assets
        },
        "recession_risk": {
            "bonds_fixed_income":      +8,
            "cash_money_market":       +5,
            "sector_bets":             -3,
            "growth_thematic":         -4,
            "small_cap":               -3,
            "alternatives":            -3,
        },
    }
    
    adjustments = OVERLAYS.get(overlay_type, {})
    for category, delta in adjustments.items():
        if category in allocation:
            allocation[category] = max(0, allocation[category] + delta)
    
    # Re-normalize to 100%
    total = sum(allocation.values())
    return {k: round(v / total * 100, 1) for k, v in allocation.items()}
```

---

## 8. Module 5: Geopolitical risk overlay (detailed)

### Purpose

Monitors market-based proxies for geopolitical risk and automatically adjusts the diversification plan. This module was critical during the Iran-Strait of Hormuz crisis that began February 28, 2026.

### Risk signal matrix

```python
RISK_SIGNALS = {
    "oil_brent": {
        "metric": "price_per_barrel",
        "thresholds": {
            "NORMAL":   (0, 80),
            "WARNING":  (80, 95),
            "ELEVATED": (95, 120),
            "CRISIS":   (120, float("inf")),
        },
        "weight": 3,  # Oil is the strongest geopolitical signal
    },
    "vix": {
        "metric": "index_level",
        "thresholds": {
            "NORMAL":   (0, 18),
            "WARNING":  (18, 25),
            "ELEVATED": (25, 35),
            "CRISIS":   (35, float("inf")),
        },
        "weight": 2,
    },
    "gold_3m_change": {
        "metric": "pct_change_3m",
        "thresholds": {
            "NORMAL":   (-999, 3),
            "ELEVATED": (3, 8),
            "CRISIS":   (8, float("inf")),
        },
        "weight": 1,
    },
    "dxy_3m_change": {
        "metric": "pct_change_3m",
        "thresholds": {
            "NORMAL":   (-999, 2),
            "ELEVATED": (2, 5),
            "CRISIS":   (5, float("inf")),
        },
        "weight": 1,
    },
}
```

### Conditional trigger engine

```python
class ConditionalTrigger:
    """
    Defines actions that fire when specific market conditions are met.
    Used in the execution planner to adapt the plan in real-time.
    """
    
    TRIGGERS = [
        {
            "name": "strait_reopens",
            "condition": lambda data: data["oil_brent"]["current"] < 85 
                         and data["oil_brent"]["1w_change_pct"] < -10,
            "action": "deploy_em_and_intl",
            "description": "Deploy parked cash into EM (FZILX) and international (FTIHX)",
            "allocation_change": {"emerging_markets": +4, "international_developed": +2, 
                                  "cash_money_market": -6},
        },
        {
            "name": "sp500_correction",
            "condition": lambda data: data["sp500"]["drawdown_from_high_pct"] < -10,
            "action": "deploy_cash_at_discount",
            "description": "Deploy remaining cash into FZROX and FZIPX at discount",
            "allocation_change": {"us_broad_market": +3, "small_cap": +2, 
                                  "cash_money_market": -5},
        },
        {
            "name": "oil_escalation",
            "condition": lambda data: data["oil_brent"]["current"] > 130 
                         and data["oil_brent"]["days_above_120"] > 14,
            "action": "increase_defensives",
            "description": "Increase energy to 15%; add bond allocation; accelerate AMZN sales",
            "allocation_change": {"sector_bets": +3, "bonds_fixed_income": +5, 
                                  "growth_thematic": -4, "small_cap": -4},
        },
        {
            "name": "stock_crash",
            "condition": lambda data: data["concentrated_stock"]["1d_change_pct"] < -15,
            "action": "emergency_accelerate",
            "description": "Accelerate selling; convert collar to pure protective put",
            "phase_override": "sell_all_remaining_within_30_days",
        },
    ]
```

---

## 9. Module 6: Execution planner

### Purpose

Generates a concrete, calendar-aware, phase-by-phase selling and deployment plan with specific lot assignments, dollar amounts, and platform instructions.

### Phase structure

```python
@dataclass
class ExecutionPhase:
    name: str               # "Phase 1: Rapid De-Risk"
    target_concentration: float  # Target AMZN % at end of phase
    duration_months: int
    lots_to_sell: List[EnrichedLot]
    deployment_targets: Dict[str, float]  # {"FZROX": 30000, "FTIHX": 15000}
    options_strategy: Optional[OptionsOverlay]
    tax_year_target: float  # Max LTCG to realize in this tax year
    platform_instructions: List[str]  # Step-by-step Fidelity instructions

@dataclass  
class OptionsOverlay:
    strategy: str           # "protective_put", "covered_call", "collar"
    put_strike: float       # e.g., 190 (~10% OTM)
    put_expiry: str         # "June 2026"
    call_strike: float      # e.g., 235 (~10% OTM)
    call_expiry: str
    shares_covered: int
    estimated_net_cost: float  # Negative = credit (collar can be costless)

@dataclass
class ExecutionPlan:
    phases: List[ExecutionPhase]
    total_duration_months: int  # 24
    starting_concentration: float  # 75%
    ending_concentration: float    # 5-8%
    total_estimated_tax: float
    conditional_triggers: List[ConditionalTrigger]
    weekly_checklist: List[str]
    quarterly_review_items: List[str]
```

### Platform-specific instructions generator

```python
def generate_fidelity_instructions(lot: EnrichedLot, action: str) -> List[str]:
    """
    Generate step-by-step Fidelity platform instructions.
    """
    if action == "sell_specific_lot":
        return [
            "1. Log into Fidelity.com → Accounts & Trade → Trade",
            f"2. Enter symbol: {lot.ticker} | Action: Sell",
            "3. Click 'More Trade Selections' at bottom",
            "4. Select 'Sell Specific Shares'",
            f"5. Find lot acquired {lot.acquired} with {lot.quantity} shares",
            f"6. Enter quantity: {lot.quantity}",
            "7. Order type: Market (or Limit if volatile)",
            "8. Review and confirm — verify lot selection on confirmation screen",
            "9. After settlement (T+1), verify on Closed Positions page",
        ]
    elif action == "change_cost_basis_method":
        return [
            "1. Accounts & Trade → Account Features",
            "2. Click 'Cost Basis Information Tracking'",
            "3. Find the concentrated stock position",
            "4. Change from 'FIFO' to 'Specific Identification'",
            "5. Confirm the change",
            "NOTE: Do this BEFORE your first sell order",
        ]
    elif action == "buy_protective_put":
        return [
            "1. Download Active Trader Pro (free) from Fidelity.com",
            "2. Trade → Options → Single Leg",
            f"3. Symbol: {lot.ticker} | Action: Buy to Open | Type: Put",
            f"4. Strike: ${lot.suggested_put_strike} | Expiry: {lot.suggested_expiry}",
            "5. Quantity: number of contracts = shares ÷ 100",
            "6. Order type: Limit | Set at mid-price of bid/ask",
            "NOTE: Requires Options Level 2+ approval",
        ]
```

---

## 10. Module 7: Visualization layer

### Purpose

Renders interactive dashboards, charts, and reports for both the advisor and the end client. Two output modes: embedded web widget (for chat/app integration) and standalone HTML report.

### Visualization 1: Portfolio overview dashboard

**Type:** Metric cards + donut chart  
**Library:** Chart.js 4.x  
**Interaction:** Click sector to drill down via `sendPrompt()`

```html
<!-- Metric cards row -->
<div style="display: grid; grid-template-columns: repeat(4, 1fr); gap: 12px;">
  <div class="metric-card">
    <div class="label">AMZN Price</div>
    <div class="value">$211.74</div>
    <div class="note">-8% YTD</div>
  </div>
  <div class="metric-card">
    <div class="label">Concentration</div>
    <div class="value danger">75%</div>
    <div class="note">Target: 5-8%</div>
  </div>
  <div class="metric-card">
    <div class="label">Unrealized Gain</div>
    <div class="value success">$368,778</div>
    <div class="note">32.7% weighted avg</div>
  </div>
  <div class="metric-card">
    <div class="label">Harvestable Losses</div>
    <div class="value">$12,414</div>
    <div class="note">3 short-term lots</div>
  </div>
</div>

<!-- Donut chart: current vs target allocation -->
<canvas id="allocationChart"></canvas>
<script>
new Chart(document.getElementById('allocationChart'), {
  type: 'doughnut',
  data: {
    labels: ['AMZN (current)', 'Other (current)', 'AMZN (target)', 'Diversified (target)'],
    datasets: [
      { data: [75, 25], backgroundColor: ['#E24B4A', '#888780'] },  // Current
      { data: [7, 93], backgroundColor: ['#D85A30', '#1D9E75'] },   // Target
    ]
  },
  options: {
    cutout: '60%',
    plugins: { legend: { display: false } }
  }
});
</script>
```

### Visualization 2: Tax lot priority waterfall

**Type:** Horizontal bar chart showing sell order with color-coded gain/loss  
**Library:** Chart.js with custom plugin  
**Key feature:** Bars colored by tax efficiency (green = loss/low gain, red = high gain)

```javascript
// Tax lot waterfall chart
const lots = sortedLots.map(lot => ({
  label: `${lot.acquired} (${lot.quantity} shares)`,
  value: lot.gain_loss_pct,
  phase: lot.phase,
  color: lot.gain_loss_pct < 0 ? '#34d399' :     // Loss = green (tax benefit)
         lot.gain_loss_pct < 30 ? '#60a5fa' :     // Low gain = blue
         lot.gain_loss_pct < 70 ? '#fbbf24' :     // Medium = amber
         '#f87171',                                 // High gain = red (expensive)
}));

new Chart(ctx, {
  type: 'bar',
  data: {
    labels: lots.map(l => l.label),
    datasets: [{
      data: lots.map(l => l.value),
      backgroundColor: lots.map(l => l.color),
    }]
  },
  options: {
    indexAxis: 'y',  // Horizontal bars
    plugins: {
      tooltip: {
        callbacks: {
          label: (ctx) => {
            const lot = lots[ctx.dataIndex];
            return `${lot.value > 0 ? '+' : ''}${lot.value.toFixed(1)}% | ${lot.phase}`;
          }
        }
      }
    }
  }
});
```

### Visualization 3: Multi-year tax comparison

**Type:** Grouped bar chart comparing total tax across 1-5 year strategies  
**Highlight:** Optimal strategy gets accent border

```javascript
new Chart(ctx, {
  type: 'bar',
  data: {
    labels: ['1 Year', '2 Years', '3 Years', '4 Years', '5 Years'],
    datasets: [
      {
        label: 'Federal LTCG',
        data: [52_125, 43_419, 41_578, 41_578, 41_578],
        backgroundColor: '#378ADD',
      },
      {
        label: 'NIIT (3.8%)',
        data: [13_813, 11_462, 10_963, 10_963, 10_963],
        backgroundColor: '#7F77DD',
      },
      {
        label: 'WA State',
        data: [6_832, 2_401, 0, 0, 0],
        backgroundColor: '#E24B4A',
      },
    ]
  },
  options: {
    scales: { x: { stacked: true }, y: { stacked: true } },
    plugins: {
      annotation: {
        annotations: {
          optimalLine: {
            type: 'line',
            xMin: 2, xMax: 2,  // 3 Years is optimal
            borderColor: '#34d399',
            borderWidth: 2,
            label: { content: 'Optimal', display: true }
          }
        }
      }
    }
  }
});
```

### Visualization 4: Geopolitical impact tracker

**Type:** Dual-axis line chart (equities on left, oil on right)  
**Data:** S&P 500, MSCI EM, Europe returns vs. Brent crude since war start  
**Interaction:** Hover for daily values; click data point for news context

```javascript
// Geopolitical event impact widget design
// Key components: 4 metric cards (Brent, S&P, MSCI EM, DXY) above chart
// Chart: 3 equity lines (left axis, %) + 1 oil line (right axis, $/bbl)
// Below chart: country-level EM stress table with currency/debt/rate columns
// Bottom: 3 scenario cards (short war / prolonged / extended)
```

### Visualization 5: Target allocation ring with fund details

**Type:** Interactive donut with clickable segments  
**Feature:** Click a segment to show the recommended Fidelity fund, expense ratio, and rationale

```javascript
// Outer ring: target allocation percentages
// Inner ring: current allocation (dominated by AMZN)
// Click handler: sendPrompt(`Tell me more about ${segment.category}`)
// Hover: show fund ticker, ER, and key stat
```

### Visualization 6: Execution timeline (Gantt-style)

**Type:** Horizontal timeline with phases, milestones, and conditional triggers  
**Library:** Pure HTML/CSS or D3.js

```html
<!-- Phase bars -->
<div class="timeline">
  <div class="phase" style="left:0%; width:25%; background: var(--red-dim);">
    <span>Phase 1: 75% → 35%</span>
    <span class="milestone" style="left: 5%;">Harvest losses</span>
    <span class="milestone" style="left: 50%;">Sell ST gains</span>
    <span class="milestone" style="left: 95%;">Begin LT lots</span>
  </div>
  <div class="phase" style="left:25%; width:50%; background: var(--amber-dim);">
    <span>Phase 2: 35% → 15%</span>
    <span class="milestone" style="left: 0%;">Tax Year 2027 starts</span>
    <span class="milestone" style="left: 50%;">Roll collars</span>
  </div>
  <div class="phase" style="left:75%; width:25%; background: var(--green-dim);">
    <span>Phase 3: 15% → 5-8%</span>
    <span class="milestone" style="left: 80%;">DAF donations</span>
  </div>
</div>
<!-- Conditional triggers as diamonds on the timeline -->
<div class="trigger" style="left: 30%;" title="If Strait reopens: deploy EM">◆</div>
<div class="trigger" style="left: 45%;" title="If S&P hits 6300: deploy cash">◆</div>
```

### Visualization 7: Concentration risk decay curve

**Type:** Area chart showing how portfolio volatility decreases as concentration drops  
**Data source:** BNY Mellon research: 1 stock = 34% vol → 5 stocks = 21.4% → 20+ stocks = 15.8%

```javascript
// X-axis: number of positions (1, 5, 10, 20, 50, 500)
// Y-axis: annual portfolio volatility (%)
// Shaded area under curve
// Vertical marker at "current" (1 stock at 75%) and "target" (diversified)
// Annotation: "Your risk drops 40% in Phase 1 alone"
```

### Visualization 8: Sector rotation heatmap

**Type:** Grid heatmap of 11 S&P sectors × 3 time periods (1m, 3m, 6m)  
**Color:** Green (outperforming) to red (underperforming)

```javascript
// Rows: Technology, Healthcare, Energy, Financials, etc.
// Columns: 1M return, 3M return, 6M return
// Cell color: d3.scaleLinear([-10, 0, +20], ['#f87171', '#888', '#34d399'])
// Highlight: sectors in the target allocation get a border
```

---

## 11. Module 8: Report generator

### Purpose

Produces a client-deliverable report combining all analysis, visualizations, and action items into a single document (PDF, HTML, or interactive web page).

### Report structure

```
1. Executive Summary
   - Current position snapshot (4 metric cards)
   - Key recommendation (1-sentence)
   - Estimated tax savings from optimal strategy

2. Position Analysis
   - All lots table with priority ranking
   - Tax lot waterfall visualization
   - Concentration risk quantification

3. Market Context
   - Stock fundamental analysis
   - Macro environment snapshot
   - Sector rotation heatmap
   - Geopolitical risk overlay (if active)

4. Tax Strategy
   - Multi-year comparison chart
   - Annual gain targets
   - State-specific considerations
   - TLH opportunities

5. Target Allocation
   - Allocation donut chart (current vs. target)
   - Fund selection table with expense ratios
   - Platform-specific instructions

6. Execution Plan
   - 3-phase timeline (Gantt visualization)
   - This-week action checklist
   - Options overlay recommendation
   - Conditional triggers

7. Monitoring Plan
   - Weekly, quarterly, annual review cadence
   - Alert thresholds
   - Rebalancing rules

8. Disclaimer
   - Not financial advice
   - Consult CPA and RIA
   - Past performance ≠ future results
```

---

## 12. Data models and schemas

### Core database schema (if persisted)

```sql
-- Users/clients
CREATE TABLE clients (
  id UUID PRIMARY KEY,
  name TEXT,
  filing_status TEXT CHECK (filing_status IN ('MFJ','Single','HoH')),
  state TEXT,
  risk_tolerance TEXT,
  timeline_years INT,
  created_at TIMESTAMP DEFAULT NOW()
);

-- Stock positions (tax lots)
CREATE TABLE tax_lots (
  id UUID PRIMARY KEY,
  client_id UUID REFERENCES clients(id),
  ticker TEXT NOT NULL,
  acquired DATE NOT NULL,
  term TEXT CHECK (term IN ('Short','Long')),
  quantity INT NOT NULL,
  cost_basis_per_share DECIMAL(10,2),
  cost_basis_total DECIMAL(12,2),
  current_value DECIMAL(12,2),
  gain_loss_dollar DECIMAL(12,2),
  gain_loss_pct DECIMAL(6,2),
  sell_priority INT,
  phase TEXT,
  status TEXT DEFAULT 'open' CHECK (status IN ('open','sold','donated')),
  sold_date DATE,
  sold_price DECIMAL(10,2)
);

-- Market snapshots (time series)
CREATE TABLE market_snapshots (
  id UUID PRIMARY KEY,
  ticker TEXT,
  collected_at TIMESTAMP NOT NULL,
  data JSONB NOT NULL  -- Full MarketSnapshot object
);

-- Execution plans
CREATE TABLE execution_plans (
  id UUID PRIMARY KEY,
  client_id UUID REFERENCES clients(id),
  created_at TIMESTAMP DEFAULT NOW(),
  plan_data JSONB NOT NULL,  -- Full ExecutionPlan object
  geopolitical_overlay TEXT,
  status TEXT DEFAULT 'active'
);

-- Trigger log
CREATE TABLE trigger_events (
  id UUID PRIMARY KEY,
  plan_id UUID REFERENCES execution_plans(id),
  trigger_name TEXT NOT NULL,
  fired_at TIMESTAMP DEFAULT NOW(),
  market_data JSONB,
  action_taken TEXT
);
```

---

## 13. API design

### REST endpoints (if built as a service)

```yaml
# Position Management
POST   /api/v1/positions/import          # Upload CSV of tax lots
GET    /api/v1/positions/{client_id}     # Get all lots with enrichment
GET    /api/v1/positions/{client_id}/sell-order  # Get prioritized sell order

# Market Data
GET    /api/v1/market/{ticker}           # Full market snapshot
GET    /api/v1/market/sectors            # Sector rotation data
GET    /api/v1/market/geopolitical       # Risk signals

# Tax Optimization
POST   /api/v1/tax/compare              # Compare N-year scenarios
GET    /api/v1/tax/thresholds/{state}   # State-specific thresholds

# Allocation
POST   /api/v1/allocation/build         # Generate target allocation
POST   /api/v1/allocation/overlay       # Apply geopolitical overlay

# Execution
POST   /api/v1/plan/generate            # Full execution plan
GET    /api/v1/plan/{plan_id}/triggers  # Check active triggers
POST   /api/v1/plan/{plan_id}/execute   # Record a completed action

# Reports
GET    /api/v1/report/{client_id}/html  # Interactive HTML report
GET    /api/v1/report/{client_id}/pdf   # PDF report
```

---

## 14. Technology stack recommendations

### Backend

| Component | Recommendation | Rationale |
|---|---|---|
| Language | Python 3.11+ | Best ecosystem for finance (pandas, yfinance, numpy) |
| Web framework | FastAPI | Async, auto-docs, type hints, fast |
| Database | PostgreSQL + JSONB | Relational + flexible JSON for market snapshots |
| Task queue | Celery + Redis | Async market data collection, scheduled monitoring |
| Caching | Redis | Cache market data (TTL: 5 min for prices, 1 hour for fundamentals) |

### Frontend

| Component | Recommendation | Rationale |
|---|---|---|
| Framework | React 18 + TypeScript | Component-based; strong ecosystem |
| Charts | Chart.js 4 or Recharts | Lightweight; good for financial data |
| Styling | Tailwind CSS | Utility-first; fast iteration |
| PDF generation | Puppeteer (headless Chrome) | Render HTML report → PDF |

### Data sources

| Source | Cost | Data |
|---|---|---|
| yfinance | Free | Stock prices, fundamentals, analyst data |
| FRED API | Free (key required) | Macro indicators, Treasury yields |
| Alpha Vantage | Free tier (25/day) | Backup stock data |
| Polygon.io | $29/mo starter | Real-time prices, options chains |
| EODHD | $20/mo | Fundamental data, ETF holdings |
| News API | Free tier | Geopolitical news for context |

---

## 15. Testing strategy

```python
# Unit tests for each module
class TestPositionAnalyzer:
    def test_loss_lots_prioritized_first(self):
        """ST loss lots should always have lowest sell_priority numbers."""
        
    def test_lt_lots_sorted_by_gain_ascending(self):
        """Long-term lots sorted lowest gain % first."""
        
    def test_days_to_ltcg_calculated_correctly(self):
        """Short-term lots should show days until 1-year anniversary."""
        
    def test_tax_estimate_accuracy(self):
        """Tax per $100K should be within 5% of manual calculation."""

class TestTaxOptimizer:
    def test_wa_under_threshold_zero_tax(self):
        """WA state tax should be $0 when gains < $278K."""
        
    def test_wa_above_threshold(self):
        """WA state tax should be 7% on amount above $278K."""
        
    def test_spreading_reduces_total_tax(self):
        """Multi-year spread should produce lower total tax than lump sum."""
        
    def test_niit_threshold(self):
        """NIIT should only apply above $250K MAGI for MFJ."""

class TestAllocationBuilder:
    def test_allocations_sum_to_100(self):
        """All allocation percentages must sum to 100%."""
        
    def test_overlay_preserves_total(self):
        """Geopolitical overlay should not change total allocation."""
        
    def test_platform_funds_exist(self):
        """Every recommended fund ticker should be valid on the platform."""

class TestGeopoliticalOverlay:
    def test_oil_crisis_increases_energy(self):
        """Oil crisis overlay should increase energy/defense allocation."""
        
    def test_triggers_fire_correctly(self):
        """Conditional triggers should fire when conditions are met."""
```

---

## 16. Deployment

```yaml
# docker-compose.yml
version: '3.8'
services:
  api:
    build: ./backend
    ports: ["8000:8000"]
    environment:
      - DATABASE_URL=postgresql://ghost:alpha@db:5432/ghostalpha
      - REDIS_URL=redis://redis:6379
      - FRED_API_KEY=${FRED_API_KEY}
    depends_on: [db, redis]
    
  worker:
    build: ./backend
    command: celery -A tasks worker --loglevel=info
    depends_on: [redis]
    
  scheduler:
    build: ./backend
    command: celery -A tasks beat --loglevel=info
    depends_on: [redis]
    
  frontend:
    build: ./frontend
    ports: ["3000:3000"]
    
  db:
    image: postgres:16
    volumes: ["pgdata:/var/lib/postgresql/data"]
    environment:
      - POSTGRES_DB=ghostalpha
      - POSTGRES_USER=ghost
      - POSTGRES_PASSWORD=${DB_PASSWORD}
      
  redis:
    image: redis:7-alpine
```

---

## 17. Future roadmap

### Phase 1 (MVP — 4 weeks)
- Position analyzer with CSV import
- Tax optimizer with WA/CA/TX/NY/FL support
- Allocation builder with Fidelity/Schwab/Vanguard
- Static HTML report output

### Phase 2 (6 weeks)
- Market data auto-collection (daily cron)
- Geopolitical overlay engine
- Interactive web dashboard
- Options strategy recommender (collar/covered call)

### Phase 3 (8 weeks)
- Brokerage API integration (Fidelity, Schwab via Plaid)
- Automated trigger monitoring with alerts (email/SMS)
- Direct indexing TLH tracking
- Multi-client advisor portal

### Phase 4 (future)
- AI-powered narrative report generation (Claude API integration)
- Backtesting: "what if we had started this plan 6 months ago?"
- Tax projection integration with TurboTax/H&R Block exports
- Mobile app for monitoring and alerts
