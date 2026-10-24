#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# GhostAlpha Terminal Screencast Demo (v2.0)
# ─────────────────────────────────────────────────────────────────────────────
#
# Records an asciinema screencast walking through the full GhostAlpha workflow:
# screening → regime → sizing → rotation → strategies → tests → dashboard.
#
# Usage:
#   ./scripts/record-demo.sh            # Record with asciinema
#   ./scripts/record-demo.sh --dry-run  # Run without recording
#
# Prerequisites:
#   brew install asciinema   (or pip install asciinema)
#   cargo build --release
#
# Output:
#   demo/ghost-alpha-demo.cast   — asciinema recording
#
# To replay:   asciinema play demo/ghost-alpha-demo.cast
# To upload:   asciinema upload demo/ghost-alpha-demo.cast
# To GIF:      agg demo/ghost-alpha-demo.cast demo/ghost-alpha-demo.gif
# ─────────────────────────────────────────────────────────────────────────────

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
DEMO_DIR="$PROJECT_DIR/demo"
CAST_FILE="$DEMO_DIR/ghost-alpha-demo.cast"
BINARY="$PROJECT_DIR/target/release/ghost-alpha"
AUTO_MODE="${1:-}"

# ── Colors ───────────────────────────────────────────────────────────────────

BOLD="\033[1m"
DIM="\033[2m"
CYAN="\033[36m"
GREEN="\033[32m"
YELLOW="\033[33m"
MAGENTA="\033[35m"
RESET="\033[0m"

# ── Helpers ──────────────────────────────────────────────────────────────────

# Simulated typing — prints each character with a small delay
type_cmd() {
    local cmd="$1"
    local char_delay="${2:-0.04}"
    printf "${GREEN}\$ ${RESET}"
    for (( i=0; i<${#cmd}; i++ )); do
        printf "%s" "${cmd:$i:1}"
        sleep "$char_delay"
    done
    echo ""
    sleep 0.3
}

# Run a command with simulated typing, then execute it
run() {
    type_cmd "$1"
    eval "$1"
    echo ""
    sleep 1.5
}

# Print a section banner
section() {
    local title="$1"
    echo ""
    printf "${BOLD}${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}\n"
    printf "${BOLD}${CYAN}  ▸ %s${RESET}\n" "$title"
    printf "${BOLD}${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}\n"
    echo ""
    sleep 2
}

# Print a narrator comment
narrate() {
    printf "${DIM}${MAGENTA}# %s${RESET}\n" "$1"
    sleep 1.5
}

# Pause between sections
pause() {
    sleep "${1:-3}"
}

# ── Pre-flight checks ─────────────────────────────────────────────────────

export PATH="$HOME/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:$PATH"

if [[ ! -x "$BINARY" ]]; then
    echo "Binary not found. Building..."
    (cd "$PROJECT_DIR" && cargo build --release)
fi

mkdir -p "$DEMO_DIR"

# ── The Demo Script ──────────────────────────────────────────────────────────

demo_script() {
    export PATH="$HOME/.cargo/bin:/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"
    export TERM="${TERM:-xterm-256color}"
    cd "$PROJECT_DIR"

    clear 2>/dev/null || printf "c"
    printf "${BOLD}${CYAN}"
    cat << 'BANNER'

     ██████╗ ██╗  ██╗ ██████╗ ███████╗████████╗
    ██╔════╝ ██║  ██║██╔═══██╗██╔════╝╚══██╔══╝
    ██║  ███╗███████║██║   ██║███████╗   ██║
    ██║   ██║██╔══██║██║   ██║╚════██║   ██║
    ╚██████╔╝██║  ██║╚██████╔╝███████║   ██║
     ╚═════╝ ╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝

     █████╗ ██╗     ██████╗ ██╗  ██╗ █████╗
    ██╔══██╗██║     ██╔══██╗██║  ██║██╔══██╗
    ███████║██║     ██████╔╝███████║███████║
    ██╔══██║██║     ██╔═══╝ ██╔══██║██╔══██║
    ██║  ██║███████╗██║     ██║  ██║██║  ██║
    ╚═╝  ╚═╝╚══════╝╚═╝     ╚═╝  ╚═╝╚═╝  ╚═╝

BANNER
    printf "${RESET}"
    printf "${DIM}    Adaptive market intelligence agent${RESET}
"
    printf "${DIM}    4 strategies | HMM regime | VIX signals | 1,385 tests${RESET}
"
    echo ""
    sleep 4

    # ── 1. Screening ──────────────────────────────────────────────────

    section "1/10 — Stock Screening"

    narrate "Screen 50 stocks with Minervini trend template:"
    run "./target/release/ghost-alpha demo --top 8"
    pause 2

    # ── 2. Strategies ─────────────────────────────────────────────────

    section "2/10 — Available Strategies"

    narrate "Four built-in strategies, adaptive engine selects weights:"
    run "./target/release/ghost-alpha strategies"
    pause 2

    # ── 3. HMM Regime ─────────────────────────────────────────────────

    section "3/10 — Probabilistic Regime Detection (HMM)"

    narrate "3-state Hidden Markov Model with confidence scores:"
    run "./target/release/ghost-alpha regime --model hmm"
    pause 2

    # ── 4. VIX Term Structure ─────────────────────────────────────────

    section "4/10 — VIX Term Structure Signal"

    narrate "Forward-looking risk from options market:"
    run "./target/release/ghost-alpha vix"
    pause 2

    # ── 5. Factor Exposure ────────────────────────────────────────────

    section "5/10 — Portfolio Factor Exposure"

    narrate "Detect hidden concentration risks:"
    run "./target/release/ghost-alpha factors"
    pause 2

    # ── 6. Position Sizing ────────────────────────────────────────────

    section "6/10 — Position Sizing (\$150K Portfolio)"

    narrate "Convert signals to exact share counts:"
    run "./target/release/ghost-alpha size --portfolio 150000 --max-pct 10"
    pause 2

    # ── 7. Capital Rotation ───────────────────────────────────────────

    section "7/10 — Capital Rotation (Sell Pressure)"

    narrate "Which positions to keep, which to swap:"
    run "./target/release/ghost-alpha rotate"
    pause 2

    # ── 8. Volatility Targeting ───────────────────────────────────────

    section "8/10 — Portfolio Volatility Target"

    narrate "Scale exposure to maintain 15% target vol:"
    run "./target/release/ghost-alpha vol-target --target 15 --invested 70"
    pause 2

    # ── 9. Attribution ────────────────────────────────────────────────

    section "9/10 — Performance Attribution"

    narrate "Where did the alpha come from?"
    run "./target/release/ghost-alpha attribution --period 90"
    pause 2

    # ── 10. Execute ───────────────────────────────────────────────────

    section "10/10 — Trade Execution (Dry Run)"

    narrate "Preview the trade — no money at risk:"
    run "./target/release/ghost-alpha execute --buy CRWD --shares 15"
    narrate ""
    narrate "Add --mode paper for Alpaca paper trading."
    pause 2

    # ── Outro ─────────────────────────────────────────────────────────

    echo ""
    printf "${BOLD}${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}
"
    printf "${BOLD}${GREEN}  Demo complete${RESET}
"
    printf "${BOLD}${CYAN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}
"
    echo ""
    printf "  ${BOLD}Quick start:${RESET}
"
    printf "    ghost-alpha demo              ${DIM}# screen with sample data${RESET}
"
    printf "    ghost-alpha regime --model hmm ${DIM}# probabilistic regime${RESET}
"
    printf "    ghost-alpha vix               ${DIM}# VIX term structure${RESET}
"
    printf "    ghost-alpha factors            ${DIM}# portfolio factor exposure${RESET}
"
    printf "    ghost-alpha size --portfolio X ${DIM}# position sizing${RESET}
"
    printf "    ghost-alpha rotate            ${DIM}# sell pressure analysis${RESET}
"
    printf "    ghost-alpha serve             ${DIM}# web dashboard${RESET}
"
    echo ""
    printf "${DIM}  Single binary (~7MB) | 1,385 tests | Rust + axum + htmx${RESET}
"
    echo ""

    sleep 5
}

# ── Record or run directly ───────────────────────────────────────────────────

if [[ "$AUTO_MODE" == "--dry-run" ]]; then
    echo "Running demo without recording (dry run)..."
    demo_script
    exit 0
fi

# For actual recording, asciinema is required
if ! command -v asciinema &>/dev/null; then
    echo "asciinema is required for recording. Install it with one of:"
    echo "  brew install asciinema"
    echo "  pip install asciinema"
    echo "  pipx install asciinema"
    echo ""
    echo "Or run a dry run without recording:"
    echo "  $0 --dry-run"
    exit 1
fi

# Export the function so the sub-shell can see it
export -f demo_script type_cmd run section narrate pause
export BINARY PROJECT_DIR DEMO_DIR
export BOLD DIM CYAN GREEN YELLOW MAGENTA RESET

echo ""
echo "╔═══════════════════════════════════════════════════════════╗"
echo "║       GhostAlpha Terminal Screencast Recorder (v2.0)     ║"
echo "╠═══════════════════════════════════════════════════════════╣"
echo "║  Output: $CAST_FILE"
echo "║                                                           ║"
echo "║  6 sections:                                              ║"
echo "║    1. Stock Screening (Minervini)                         ║"
echo "║    2. Available Strategies (4 built-in)                   ║"
echo "║    3. Position Sizing (VolatilityTarget)                  ║"
echo "║    4. Capital Rotation (SPS + CDS)                        ║"
echo "║    5. Test Suite (1,300+ tests)                           ║"
echo "║    6. Full Command Reference (19 commands)                ║"
echo "║                                                           ║"
echo "║  Duration: ~2 minutes                                     ║"
echo "╚═══════════════════════════════════════════════════════════╝"
echo ""

asciinema rec \
    --title "GhostAlpha v2.0 — Adaptive Market Intelligence Agent" \
    --idle-time-limit 3 \
    --cols 90 \
    --rows 32 \
    --command "bash -c demo_script" \
    "$CAST_FILE"

echo ""
echo "Recording saved to: $CAST_FILE"
echo ""
echo "Next steps:"
echo "  asciinema play $CAST_FILE          # Replay locally"
echo "  asciinema upload $CAST_FILE        # Share on asciinema.org"
echo "  agg $CAST_FILE demo/demo.gif       # Convert to GIF (brew install agg)"
echo ""
