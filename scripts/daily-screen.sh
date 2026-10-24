#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Ghost Alpha — Daily Screen + Weekly Summary
# ─────────────────────────────────────────────────────────────────────────────
# Usage:
#   ./scripts/daily-screen.sh              # daily screen only
#   ./scripts/daily-screen.sh --summary    # daily screen + weekly summary
#   SUMMARY_ONLY=1 ./scripts/daily-screen.sh --summary  # summary without screen
#
# Schedule with cron:
#   0  18 * * 1-5  /path/to/scripts/daily-screen.sh
#   15 18 * * 5    /path/to/scripts/daily-screen.sh --summary
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

# ── Configuration ────────────────────────────────────────────────────────────

GHOST_DIR="${GHOST_DIR:-$(cd "$(dirname "$0")/.." && pwd)}"
BINARY="${GHOST_DIR}/target/release/ghost-alpha"
LOG_DIR="${GHOST_DIR}/logs"
DB_PATH="${GHOST_ALPHA_DB_PATH:-${GHOST_DIR}/ghost_alpha.db}"
SUMMARY_DIR="${GHOST_DIR}/reports"

TOP_N=50
FORMAT="table"
STRATEGY="minervini"

# ── Setup ────────────────────────────────────────────────────────────────────

mkdir -p "$LOG_DIR" "$SUMMARY_DIR"

TODAY=$(date +%Y-%m-%d)
DOW=$(date +%u)  # 1=Mon, 5=Fri
LOG_FILE="${LOG_DIR}/screen-${TODAY}.log"

log() { echo "[$(date '+%H:%M:%S')] $*" | tee -a "$LOG_FILE"; }

# Load .env if present
if [[ -f "${GHOST_DIR}/.env" ]]; then
    set -a
    # shellcheck disable=SC1091
    source "${GHOST_DIR}/.env"
    set +a
fi

# ── Preflight ────────────────────────────────────────────────────────────────

if [[ ! -x "$BINARY" ]]; then
    log "ERROR: Binary not found at $BINARY"
    log "       Run: cd $GHOST_DIR && cargo build --release"
    exit 1
fi

# ── Daily Screen ─────────────────────────────────────────────────────────────

if [[ "${SUMMARY_ONLY:-}" != "1" ]]; then
    log "=== Daily Screen (${TODAY}, $(date +%A)) ==="
    log "Strategy: ${STRATEGY} | Top: ${TOP_N} | Format: ${FORMAT}"

    START_TIME=$(date +%s)

    "$BINARY" -c "${GHOST_DIR}/config.toml" screen \
        --top "$TOP_N" \
        --format "$FORMAT" \
        --strategy "$STRATEGY" \
        --skip-sentiment \
        2>&1 | tee -a "$LOG_FILE"

    ELAPSED=$(( $(date +%s) - START_TIME ))
    log "Screen completed in ${ELAPSED}s"

    # Also save JSON for programmatic analysis
    "$BINARY" -c "${GHOST_DIR}/config.toml" screen \
        --top "$TOP_N" \
        --format json \
        --strategy "$STRATEGY" \
        --skip-sentiment \
        > "${LOG_DIR}/results-${TODAY}.json" 2>/dev/null || true

    log "JSON results saved to ${LOG_DIR}/results-${TODAY}.json"
fi

# ── Weekly Summary (Friday or --summary flag) ────────────────────────────────

if [[ "${1:-}" == "--summary" ]] || [[ "$DOW" == "5" && "${1:-}" == "--auto-friday" ]]; then
    log "=== Weekly Summary ==="

    WEEK_START=$(date -v-4d +%Y-%m-%d 2>/dev/null || date -d "4 days ago" +%Y-%m-%d)
    SUMMARY_FILE="${SUMMARY_DIR}/weekly-${TODAY}.txt"

    {
        echo "╔═══════════════════════════════════════════════════════════════╗"
        echo "║          GHOST ALPHA — WEEKLY SUMMARY                       ║"
        echo "║          Week of ${WEEK_START} to ${TODAY}                  ║"
        echo "╚═══════════════════════════════════════════════════════════════╝"
        echo ""

        # ── 1. Market Regime ──
        echo "── Market Regime ─────────────────────────────────────────────"
        "$BINARY" -c "${GHOST_DIR}/config.toml" regime --format table 2>/dev/null || echo "(no regime data)"
        echo ""

        # ── 2. Today's Top Picks ──
        echo "── Top 50 Stocks (Friday Close) ───────────────────────"
        "$BINARY" -c "${GHOST_DIR}/config.toml" screen \
            --top "$TOP_N" \
            --format table \
            --strategy "$STRATEGY" \
            --skip-sentiment 2>/dev/null || echo "(screen failed)"
        echo ""

        # ── 3. Alert Summary ──
        echo "── Alerts This Week ─────────────────────────────────────────"
        "$BINARY" -c "${GHOST_DIR}/config.toml" alerts --last 50 --format table 2>/dev/null || echo "(no alerts)"
        echo ""

        # ── 4. Watchlist Status ──
        echo "── Watchlist ─────────────────────────────────────────────────"
        "$BINARY" -c "${GHOST_DIR}/config.toml" watch list 2>/dev/null || echo "(empty watchlist)"
        echo ""

        # ── 5. Score Changes (Mon vs Fri JSON diff) ──
        MON_JSON="${LOG_DIR}/results-${WEEK_START}.json"
        FRI_JSON="${LOG_DIR}/results-${TODAY}.json"
        if [[ -f "$MON_JSON" && -f "$FRI_JSON" ]]; then
            echo "── Score Changes (Mon → Fri) ─────────────────────────────"
            python3 -c "
import json, sys
try:
    mon = {r['ticker']: r['composite_score'] for r in json.load(open('$MON_JSON'))}
    fri = {r['ticker']: r['composite_score'] for r in json.load(open('$FRI_JSON'))}
    all_tickers = sorted(set(mon) | set(fri), key=lambda t: fri.get(t, 0), reverse=True)
    print(f\"{'Ticker':<8} {'Mon':>7} {'Fri':>7} {'Change':>8}\")
    print('-' * 32)
    for t in all_tickers[:20]:
        m = mon.get(t, 0)
        f = fri.get(t, 0)
        d = f - m
        arrow = '^' if d > 0 else 'v' if d < 0 else '-'
        if t not in mon:
            arrow = '*'
        elif t not in fri:
            arrow = 'x'
        print(f'{t:<8} {m:7.1f} {f:7.1f} {arrow} {d:+6.1f}')
    new = set(fri) - set(mon)
    gone = set(mon) - set(fri)
    if new: print(f\"\nNew entries: {', '.join(sorted(new))}\")
    if gone: print(f\"Dropped out: {', '.join(sorted(gone))}\")
except Exception as e:
    print(f'(comparison unavailable: {e})')
" 2>/dev/null || echo "(python3 not available or JSON missing)"
            echo ""
        fi

        # ── 6. Strategy Comparison ──
        echo "── Strategy Comparison (Top 5 per strategy) ─────────────────"
        for strat in minervini breakout mean-reversion trend-strength; do
            echo ""
            echo "  [$strat]"
            "$BINARY" -c "${GHOST_DIR}/config.toml" screen \
                --top 5 \
                --format table \
                --strategy "$strat" \
                --skip-sentiment 2>/dev/null | head -20 || echo "  (failed)"
        done
        echo ""

        echo "═══════════════════════════════════════════════════════════════"
        echo "Generated: $(date)"
        echo "Database:  ${DB_PATH}"
        echo "═══════════════════════════════════════════════════════════════"

    } | tee "$SUMMARY_FILE"

    log "Weekly summary saved to ${SUMMARY_FILE}"
fi

# ── Cleanup old logs (keep 30 days) ─────────────────────────────────────────

find "$LOG_DIR" -name "*.log" -mtime +30 -delete 2>/dev/null || true
find "$LOG_DIR" -name "*.json" -mtime +30 -delete 2>/dev/null || true
find "$SUMMARY_DIR" -name "*.txt" -mtime +90 -delete 2>/dev/null || true

log "Done."
