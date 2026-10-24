#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────────────────
# GhostAlpha Full Feature Demo
#
# Walks through the complete investment workflow:
#   1. Screening    — Find high-conviction stocks
#   2. Regime       — Assess market conditions
#   3. Sizing       — Compute position sizes
#   4. Rotation     — Evaluate existing positions
#   5. Dashboard    — Launch web interface
#
# Usage: ./scripts/demo-full.sh [--no-pause] [--port 3000]
# ──────────────────────────────────────────────────────────────────────────────
set -euo pipefail

BOLD="\033[1m"
DIM="\033[2m"
GREEN="\033[32m"
BLUE="\033[34m"
YELLOW="\033[33m"
CYAN="\033[36m"
RESET="\033[0m"
SEPARATOR="━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

NO_PAUSE=false
PORT=3000
for arg in "$@"; do
  case $arg in
    --no-pause) NO_PAUSE=true ;;
    --port) shift; PORT="${2:-3000}" ;;
  esac
done

pause() {
  if [ "$NO_PAUSE" = false ]; then
    echo ""
    echo -e "${DIM}  Press Enter to continue...${RESET}"
    read -r
  fi
}

banner() {
  echo ""
  echo -e "${BLUE}${SEPARATOR}${RESET}"
  echo -e "${BOLD}  $1${RESET}"
  echo -e "${BLUE}${SEPARATOR}${RESET}"
  echo ""
}

step() {
  echo -e "${CYAN}  ▸ $1${RESET}"
}

# ──────────────────────────────────────────────────────────────────────────────

clear
cat << 'EOF'

   ██████╗ ██╗  ██╗ ██████╗ ███████╗████████╗     █████╗ ██╗     ██████╗ ██╗  ██╗ █████╗
  ██╔════╝ ██║  ██║██╔═══██╗██╔════╝╚══██╔══╝    ██╔══██╗██║     ██╔══██╗██║  ██║██╔══██╗
  ██║  ███╗███████║██║   ██║███████╗   ██║       ███████║██║     ██████╔╝███████║███████║
  ██║   ██║██╔══██║██║   ██║╚════██║   ██║       ██╔══██║██║     ██╔═══╝ ██╔══██║██╔══██║
  ╚██████╔╝██║  ██║╚██████╔╝███████║   ██║       ██║  ██║███████╗██║     ██║  ██║██║  ██║
   ╚═════╝ ╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝       ╚═╝  ╚═╝╚══════╝╚═╝     ╚═╝  ╚═╝╚═╝  ╚═╝

EOF

echo -e "${BOLD}  Market Intelligence Agent — Full Feature Demo${RESET}"
echo -e "${DIM}  Quantitative stock screening with adaptive strategy selection${RESET}"
echo ""
echo -e "  Features demonstrated:"
echo -e "    ${GREEN}1.${RESET} Stock screening with Minervini trend template"
echo -e "    ${GREEN}2.${RESET} Market regime detection with hysteresis"
echo -e "    ${GREEN}3.${RESET} Position sizing (volatility-target method)"
echo -e "    ${GREEN}4.${RESET} Capital rotation analysis (sell pressure + deployment)"
echo -e "    ${GREEN}5.${RESET} Multi-strategy ensemble scoring"
echo -e "    ${GREEN}6.${RESET} Web dashboard with portfolio analytics"
echo ""

pause

# ──────────────────────────────────────────────────────────────────────────────
# Step 1: Screening
# ──────────────────────────────────────────────────────────────────────────────

banner "Step 1: Stock Screening — Finding High-Conviction Setups"

step "Running Minervini screen on 50-stock universe (bundled sample data)..."
echo ""

cargo run --release --quiet -- demo --top 15 2>&1

echo ""
echo -e "${DIM}  The screener evaluates 8 Minervini conditions per stock:${RESET}"
echo -e "${DIM}    1. Price > 150-SMA > 200-SMA    5. RS percentile > 70${RESET}"
echo -e "${DIM}    2. Price > 50-SMA                6. ADX > 20 (trending)${RESET}"
echo -e "${DIM}    3. 50-SMA > 150-SMA              7. Volume z-score > 0${RESET}"
echo -e "${DIM}    4. Price within 25% of 52w high   8. 200-SMA rising 1+ month${RESET}"
echo ""
echo -e "${DIM}  Composite score = Technical (55%) + Volume (30%) + Sentiment (15%)${RESET}"
echo -e "${DIM}  Weights adapt by regime: Bear shifts to 70/10/20 for safety.${RESET}"

pause

# ──────────────────────────────────────────────────────────────────────────────
# Step 2: Available Strategies
# ──────────────────────────────────────────────────────────────────────────────

banner "Step 2: Available Screening Strategies"

step "Ghost Alpha supports 4 screening strategies..."
echo ""

cargo run --release --quiet -- strategies 2>&1

echo ""
echo -e "${DIM}  Phase 6 Adaptive Engine selects strategy weights based on:${RESET}"
echo -e "${DIM}    • Market regime (Bull favors Breakout, Sideways favors MeanReversion)${RESET}"
echo -e "${DIM}    • Volatility regime (High vol boosts MeanReversion, reduces Breakout)${RESET}"
echo -e "${DIM}    • Timeframe effectiveness (which horizon is working now?)${RESET}"

pause

# ──────────────────────────────────────────────────────────────────────────────
# Step 3: Position Sizing
# ──────────────────────────────────────────────────────────────────────────────

banner "Step 3: Position Sizing — Converting Signals to Trade Sizes"

step "Computing position sizes for a \$150,000 portfolio..."
echo ""

cargo run --release --quiet -- size --portfolio 150000 --risk-pct 1 --max-pct 10 2>&1

echo ""
echo -e "${DIM}  The volatility-target method sizes inversely to ATR:${RESET}"
echo -e "${DIM}    Shares = Risk Budget / (ATR × Stop Multiplier)${RESET}"
echo -e "${DIM}    Risk Budget = Portfolio × 1% × Grade Mult × Regime Mult${RESET}"
echo -e "${DIM}${RESET}"
echo -e "${DIM}  Grade multipliers: A+ = 1.5x, A = 1.0x, B = 0.7x${RESET}"
echo -e "${DIM}  Regime multipliers: Bull = 1.0x, Sideways = 0.8x, Bear = 0.5x${RESET}"

pause

# ──────────────────────────────────────────────────────────────────────────────
# Step 4: Capital Rotation
# ──────────────────────────────────────────────────────────────────────────────

banner "Step 4: Capital Rotation — Sell Pressure & Deployment Analysis"

step "Analyzing 8-position portfolio for rotation opportunities..."
echo ""

cargo run --release --quiet -- rotate 2>&1

echo ""
echo -e "${DIM}  Sell Pressure Score (SPS) components:${RESET}"
echo -e "${DIM}    Trend Decay      — Minervini condition erosion since entry${RESET}"
echo -e "${DIM}    Momentum Fade    — Rate of composite score decline${RESET}"
echo -e "${DIM}    Drawdown %       — Distance from 60-day local high${RESET}"
echo -e "${DIM}    RS Fade          — Relative strength deterioration${RESET}"
echo -e "${DIM}    Time Decay       — Stagnation penalty after 40-day grace${RESET}"
echo ""
echo -e "${DIM}  Action thresholds: GREEN (0-25) YELLOW (26-45) ORANGE (46-65) RED (66-80) CRIT (81+)${RESET}"

pause

# ──────────────────────────────────────────────────────────────────────────────
# Step 5: Help & Configuration
# ──────────────────────────────────────────────────────────────────────────────

banner "Step 5: All Available Commands"

step "Ghost Alpha subcommands..."
echo ""

cargo run --release --quiet -- --help 2>&1

pause

# ──────────────────────────────────────────────────────────────────────────────
# Step 6: Web Dashboard
# ──────────────────────────────────────────────────────────────────────────────

banner "Step 6: Web Dashboard — Interactive Portfolio Analytics"

echo -e "  The web dashboard provides:"
echo -e "    ${GREEN}•${RESET} Portfolio overview with P&L tracking"
echo -e "    ${GREEN}•${RESET} Tax-aware selling programmes with pace rules"
echo -e "    ${GREEN}•${RESET} Diversification analysis (sector, geography, concentration)"
echo -e "    ${GREEN}•${RESET} Interactive screening results with sorting"
echo -e "    ${GREEN}•${RESET} Regime and macro risk indicators"
echo -e "    ${GREEN}•${RESET} Alert history and watchlist management"
echo ""
echo -e "  ${BOLD}To launch:${RESET}  ghost-alpha serve --port ${PORT}"
echo -e "  ${BOLD}Then open:${RESET}  http://localhost:${PORT}"
echo ""

if [ "$NO_PAUSE" = false ]; then
  echo -e "${YELLOW}  Launch the web dashboard now? [y/N]${RESET}"
  read -r -n 1 LAUNCH
  echo ""
  if [[ "$LAUNCH" =~ [yY] ]]; then
    step "Starting web server on port ${PORT}..."
    cargo run --release --quiet -- serve --port "$PORT" &
    SERVER_PID=$!
    sleep 2
    open "http://localhost:${PORT}" 2>/dev/null || xdg-open "http://localhost:${PORT}" 2>/dev/null || true
    echo -e "  ${GREEN}Dashboard running at http://localhost:${PORT}${RESET}"
    echo -e "  ${DIM}Press Ctrl+C to stop${RESET}"
    wait $SERVER_PID
  fi
fi

# ──────────────────────────────────────────────────────────────────────────────
# Summary
# ──────────────────────────────────────────────────────────────────────────────

banner "Demo Complete"

echo -e "  ${BOLD}GhostAlpha Capabilities:${RESET}"
echo ""
echo -e "  ${GREEN}Screening${RESET}        4 strategies (Minervini, Breakout, MeanReversion, TrendStrength)"
echo -e "  ${GREEN}Scoring${RESET}          Regime-adaptive composite with zero-centered volume"
echo -e "  ${GREEN}Position Sizing${RESET}  VolatilityTarget, EqualWeight, HalfKelly, FixedFractional"
echo -e "  ${GREEN}Tax Engine${RESET}       HIFO/priority lot selection, wash sale enforcement"
echo -e "  ${GREEN}Sell Pressure${RESET}    8-component score with regime-adaptive weights"
echo -e "  ${GREEN}Cash Deployment${RESET}  Opportunity + regime + drag + risk scoring"
echo -e "  ${GREEN}Rotation${RESET}         Head-to-head upgrade comparison with tier-adjusted thresholds"
echo -e "  ${GREEN}Multi-Horizon${RESET}    Swing (2-4w) / Position (2-6m) / Trend (6-12m)"
echo -e "  ${GREEN}Microstructure${RESET}   Volatility regime, correlation regime, liquidity calendar"
echo -e "  ${GREEN}Adaptive Engine${RESET}  Strategy selection, parameter tuning, Bayesian weight updating"
echo -e "  ${GREEN}Attribution${RESET}      Selection, timing, sizing, strategy decomposition"
echo -e "  ${GREEN}Alerts${RESET}           Telegram, Discord, Webhook channels"
echo -e "  ${GREEN}Dashboard${RESET}        Embedded web UI (axum + htmx + pico.css)"
echo ""
echo -e "  ${DIM}1,306 unit tests | 10 modules | Single binary (~6MB)${RESET}"
echo ""
