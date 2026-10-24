# GhostAlpha Tutorial

A step-by-step guide to using GhostAlpha for stock screening, position sizing, and portfolio management.

---

## 1. Quick Start (Zero Config)

No API key needed. Run with bundled sample data:

```bash
cargo build --release
./target/release/ghost-alpha demo --top 10
```

This screens 50 bundled stocks using the Minervini trend template, scores them with regime-adaptive composite weights, and prints a ranked table with grades.

## 2. Position Sizing

Convert screening signals into exact trade sizes:

```bash
# Default: volatility-target method, $100K portfolio
ghost-alpha size --portfolio 100000

# Custom: equal-weight across 8 positions, 10% max position
ghost-alpha size --portfolio 150000 --method equal-weight --max-pct 10

# Fixed-fractional: risk 1% per trade
ghost-alpha size --portfolio 150000 --method fixed-fractional --risk-pct 1
```

The volatility-target method sizes inversely to ATR:
- `Shares = RiskBudget / (ATR x StopMultiplier)`
- Risk budget scales with grade (A+ = 1.5x, B = 0.7x) and regime (Bull = 1.0x, Bear = 0.5x)
- Positions are capped at the configurable max percentage

## 3. Capital Rotation

Evaluate existing positions for sell pressure and deployment opportunities:

```bash
ghost-alpha rotate
```

Output includes:
- **Sell Pressure Score (SPS)** per position: GREEN (hold) to CRITICAL (sell all)
- **Cash Deployment Score (CDS)**: should you deploy freed capital or hold cash?
- Component breakdown: TrendDecay, MomentumFade, Drawdown, RS Fade, TimeDecay

SPS action thresholds:
| Score | Action |
|-------|--------|
| 0-25 | GREEN — hold |
| 26-45 | YELLOW — tighten stop |
| 46-65 | ORANGE — evaluate swap |
| 66-80 | RED — sell half |
| 81-100 | CRITICAL — sell all |

## 4. Available Strategies

```bash
ghost-alpha strategies
```

Four built-in screening strategies:

| Strategy | Conditions | Best For |
|----------|-----------|----------|
| **Minervini** | 8-point trend template | Position trades in uptrends |
| **Breakout** | Near 52w high, volume surge, consolidation | Swing trades on breakouts |
| **MeanReversion** | RSI oversold, above 200-SMA, quality filter | Pullback entries in leaders |
| **TrendStrength** | ADX > 30, MA alignment, strong RS | Long-term trend-following |

The adaptive engine automatically weights these based on the current market regime.

## 5. Market Regime

```bash
ghost-alpha regime
```

Classifies the market as Bull, Sideways, or Bear using SPY vs 200-SMA and market breadth. Regime detection includes hysteresis — requires 3 consecutive days in new conditions before transitioning, preventing whipsaw.

## 6. Full Screening (Live Data)

First, set up your config:

```bash
ghost-alpha setup   # interactive wizard
```

Then screen with live data:

```bash
# Screen S&P 500 with Yahoo Finance (no API key needed)
ghost-alpha screen --provider yahoo

# Screen with Finnhub (requires API key in .env)
ghost-alpha screen --provider finnhub

# Screen a specific universe
ghost-alpha screen --provider yahoo --universe russell1000
```

## 7. Alerts

Configure Telegram or Discord alerts in `config.toml`:

```toml
[alerts]
channels = ["telegram"]
min_grade = "B"

[alerts.telegram]
bot_token = "your-bot-token"
chat_id = "your-chat-id"
```

GhostAlpha sends alerts for:
- New screening signals (stock passes screen for first time)
- Exit signals (Stage 2 breakdown, RS deterioration, MA death cross)
- Conviction decay (composite score drops 15+ points from entry)

## 8. Backtesting

```bash
# Walk-forward backtest with Monte Carlo confidence intervals
ghost-alpha backtest

# Specific strategy
ghost-alpha backtest --strategy breakout
```

Backtesting uses walk-forward validation: optimize in-sample, validate out-of-sample, roll forward. Sharpe ratios are computed from daily portfolio returns (not overlapping forward returns).

## 9. Web Dashboard

```bash
ghost-alpha serve --port 3000
# Then open http://localhost:3000
```

The embedded dashboard (axum + htmx + pico.css) provides:
- Portfolio overview with P&L tracking
- Tax-aware selling programmes
- Diversification analysis (sector, geography, concentration)
- Screening results with sorting
- Regime and macro risk indicators
- Alert history

## 10. Tax-Aware Selling

```bash
# Compare HIFO vs priority lot selection for a position
ghost-alpha sell-compare --ticker AAPL --shares 50 --price 220

# Create a multi-year selling programme
ghost-alpha sell-plan --ticker NVDA --shares 100 --years 3
```

The tax engine supports:
- HIFO (highest-in, first-out) lot selection
- 3-tier priority selection (ST losses first, then LT gains, then ST gains)
- Wash sale detection (30-day window, both buy and sell side)
- Federal + state brackets (WA tiered, CA, NY, TX, etc.)
- NIIT threshold tracking
- Multi-year projection with scenario comparison

## 11. Watchlist

```bash
ghost-alpha watch add CRWD PANW META
ghost-alpha watch list
ghost-alpha watch remove CRWD
```

## 12. MCP Server (AI Agent Integration)

GhostAlpha exposes an MCP server for AI agents (Claude Desktop, VS Code Copilot):

```bash
ghost-alpha mcp
```

5 tools available: `screen`, `regime`, `alerts`, `config`, `plugins`.

## 13. JSON Output

All commands support JSON output for scripting:

```bash
ghost-alpha demo --format json | jq '.[0]'
ghost-alpha size --portfolio 100000 --format json
ghost-alpha regime --format json
```

---

## 14. Volatility Targeting

Monitor portfolio volatility and get exposure adjustment recommendations:

```bash
ghost-alpha vol-target --target 15 --invested 70
```

When realized vol exceeds target, the system recommends trimming. When vol is low, it recommends deploying cash. Scalar is clamped to [0.5x, 1.5x] to prevent extreme swings.

## 15. Performance Attribution

Break down your returns by source — stock selection, timing, sizing, or strategy:

```bash
ghost-alpha attribution --period 90
ghost-alpha attribution --period 180 --format json
```

Uses Brinson-Fachler decomposition to answer "where did alpha come from?"

## 16. Adaptive Calibration

Tune screening parameters using walk-forward optimization:

```bash
ghost-alpha calibrate --dry-run   # preview changes
ghost-alpha calibrate             # apply calibrated parameters
ghost-alpha calibrate --reset     # reset to defaults for current regime
```

Calibration uses coordinate descent with guardrails: max step per cycle, signal count penalty, and regime-change reset.

## 17. Trade Execution (Alpaca)

Execute trades via Alpaca's API. Three safety modes:

```bash
# Preview (default) — no API calls, shows what would happen
ghost-alpha execute --buy AAPL --shares 10

# Paper trading — executes on paper account (no real money)
ghost-alpha execute --buy AAPL --shares 10 --mode paper

# Sell with stop
ghost-alpha execute --sell NVDA --shares 5 --mode paper
```

Requires `ALPACA_API_KEY_ID` and `ALPACA_API_SECRET_KEY` environment variables for paper/live modes.

## 18. Geopolitical Risk

Compute a geopolitical risk score from market-priced proxies:

```bash
ghost-alpha geo-risk --oil 85 --vix 22 --gold-3m 8.5 --dxy-3m -2.1
```

---

## Common Workflows

### Daily Screen (Cron)

```bash
# Add to crontab: run at 5:30 PM ET on weekdays
30 17 * * 1-5 cd /path/to/ghost-alpha && ./target/release/ghost-alpha screen --provider yahoo
```

### Weekly Rotation Review

```bash
ghost-alpha rotate           # Check sell pressure on all positions
ghost-alpha screen           # Find new candidates
ghost-alpha size --portfolio $(cat portfolio_value.txt)  # Size new entries
```

### Before Selling: Tax Check

```bash
ghost-alpha sell-compare --ticker DDOG --shares 50 --price 112
# Shows HIFO vs priority selection tax impact
# Warns about wash sale conflicts
```
