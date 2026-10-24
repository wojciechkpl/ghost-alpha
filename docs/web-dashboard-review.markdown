# GhostAlpha Web Dashboard — Financial Analyst Review

**Date:** 2026-03-14
**Reviewer role:** Financial Analyst / Wealth Management Advisory
**Context:** Evaluating dashboard readiness for AMZN single-stock diversification advisory

---

## Executive Summary

The current dashboard functions as a **stock screener** but lacks the portfolio-level analytics, tax modelling, and forward-looking visualisations required for a diversification engagement. Nine pages were reviewed; one is broken (Fundamentals — blank), several have data quality issues (sentiment heatmap, RS momentum map), and the most critical analytical modules implemented in Sprints 1–5 have **no web presence**.

---

## Critical Issues

### 1. Fundamentals Page — Completely Blank
The `/fundamentals` route renders an empty white page. Fundamental data (P/E, EV/EBITDA, revenue growth, margins, FCF yield) is the backbone of any stock evaluation. This page should surface the `fundamental_metrics` and `fundamental_history` tables with trend sparklines.

### 2. No Pages for Core Diversification Modules
Sprints 1–5 delivered substantial analytical capabilities with zero web integration:

| Module | Backend Ready | Web Page | Gap |
|--------|:---:|:---:|-----|
| Portfolio Overview (weights, HHI, cost basis, P&L) | ✅ | ❌ | No visibility into concentration risk |
| Correlation Matrix (NxN heatmap, rolling, crisis) | ✅ | ❌ | Cannot assess diversification benefit |
| DCF Valuation (3-stage, reverse DCF, scenarios) | ✅ | ❌ | No fair-value anchoring |
| Portfolio Optimization (MVO, risk parity, frontier) | ✅ | ❌ | No target allocation visual |
| Tax Impact (marginal brackets, HIFO, harvest, projection) | ✅ | ❌ | Cannot evaluate transition cost |
| Selling Programme (quarterly schedule, pace rules) | ✅ | ❌ | No execution timeline |
| ETF Comparison (13 proxies, overlap analysis) | ✅ | ❌ | Cannot compare vehicles |
| Monte Carlo (forward GBM, percentile bands) | ✅ | ❌ | No wealth projection |
| Decision Log / IPS | ✅ | ❌ | No audit trail |

### 3. Sentiment Heatmap — No Meaningful Differentiation
The sentiment heatmap on `/analytics` is uniformly red with no variation. Either:
- All sentiment scores cluster near zero/negative (insufficient dynamic range), or
- The colour scale is not normalised to the data range.

A financial analyst must quickly identify sentiment outliers — this chart fails that purpose entirely.

---

## Major Issues

### 4. RS Momentum Map — Poor Data Spread
All scatter dots cluster in the bottom-left quadrant. A useful RS momentum map needs a 2×2 quadrant layout (Improving / Leading / Weakening / Lagging) with reasonable spread. Likely causes: axis auto-scaling not accounting for data range, or RS calculation window too short.

### 5. Macro Risk Section Empty
The `/regime` page shows SPY price and SMA200 but macro risk indicators (yield curve, VIX percentile, credit spreads, ISM) are blank. For diversification strategy, macro context directly drives asset allocation tilt.

### 6. Grade Distribution Chart Compressed
The A/B/C/D/F bar chart on `/analytics` is too small to read with 40 stocks. Needs more vertical space and labelled bar values.

---

## Missing Visualisations for Diversification Advisory

### 7. Concentration Risk Dashboard
A single-stock diversification client needs:
- **Pie/donut chart** of current portfolio weights (AMZN dominance visually obvious)
- **HHI score** with traffic-light indicator (red >0.25, amber 0.15–0.25, green <0.15)
- **Sector & geography exposure** breakdown bars

### 8. Tax-Aware Transition Timeline
The selling programme module exists in code but has no visual. A **Gantt-style chart** showing quarterly sell tranches with cumulative tax impact bars would be the centrepiece of a diversification presentation.

### 9. What-If Scenario Panel
The `run_scenarios()` engine has no web interface. An interactive panel to adjust growth rate, discount rate, or sell pace and see updated fair value / tax impact would dramatically improve decision-making.

### 10. Correlation Heatmap
The correlation matrix is computed but never displayed. A colour-coded NxN heatmap (red = high positive, blue = negative) is standard in portfolio analytics.

### 11. Efficient Frontier Chart
Plot risk (x-axis) vs expected return (y-axis) with:
- Current portfolio point (highlighted)
- Minimum-variance point
- Maximum-Sharpe point
- Efficient frontier curve
- Individual asset dots

### 12. Monte Carlo Fan Chart
Forward simulation should render as a fan chart:
- Median wealth line
- 10th / 25th / 75th / 90th percentile bands
- Horizontal line for target wealth / retirement threshold
- 10–30 year horizon

---

## Navigation & UX Issues

### 13. Missing Navigation Links
Sidebar/nav needs entries for: Portfolio, Correlation, Valuation, Optimize, Tax, Reports, Decisions.

### 14. No Data Vintage Indicator
The home screening table shows 40 stocks with scores but no "Last updated" timestamp. Stale data is a serious risk in advisory contexts.

### 15. No Export Capability
A financial analyst preparing client presentations needs PDF/CSV export buttons on key pages (Portfolio summary, Tax projection, Quarterly report).

---

## Recommended Implementation Priority

| Priority | Item | Effort | Impact |
|----------|------|--------|--------|
| **P0** | Fix Fundamentals page (blank) | S | Unblocks basic stock analysis |
| **P0** | Portfolio Overview page (weights, HHI, P&L) | M | Core diversification visual |
| **P0** | Correlation heatmap page | M | Shows diversification benefit |
| **P1** | Valuation dashboard (DCF + scenarios) | M | Anchors fair value discussion |
| **P1** | Tax impact & selling programme timeline | L | Central to transition planning |
| **P1** | Fix sentiment heatmap colour scale | S | Data already exists |
| **P1** | Fix RS Momentum Map axis scaling | S | Data already exists |
| **P2** | Efficient frontier chart | M | Target allocation visual |
| **P2** | Monte Carlo fan chart | M | Compelling wealth projection |
| **P2** | Macro risk data population | M | Contextualises regime |
| **P3** | Decision log / IPS viewer | S | Audit & governance |
| **P3** | ETF comparison page | M | Vehicle selection support |
| **P3** | Navigation updates + timestamps | S | UX hygiene |
| **P3** | Export (PDF/CSV) buttons | M | Client deliverable support |

**Effort key:** S = small (< 1 day), M = medium (1–3 days), L = large (3–5 days)

---

## Summary Recommendation

The backend analytical engine is remarkably comprehensive — correlation, valuation, optimization, tax modelling, and Monte Carlo are all implemented and tested. The critical gap is **surfacing these capabilities in the web dashboard**. Addressing the P0 and P1 items would transform this from a screening tool into a genuine diversification advisory platform.
