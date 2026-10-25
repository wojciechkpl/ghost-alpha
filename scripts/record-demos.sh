#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Record CLI demo outputs for the README
# ─────────────────────────────────────────────────────────────────────────────
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BINARY="${PROJECT_DIR}/target/release/ghost-alpha"
DEMO_DIR="${PROJECT_DIR}/docs/demos"

mkdir -p "$DEMO_DIR"

echo "Recording CLI demos..."

echo "  → sell-compare"
"$BINARY" sell-compare \
  --ticker AMZN --shares 3000 --cost-basis 50 --price 250 \
  --state CA --filing single --other-income 150000 --max-years 5 \
  2>/dev/null > "$DEMO_DIR/sell-compare.txt"

echo "  → allocate (fidelity)"
"$BINARY" allocate \
  --profile aggressive --brokerage fidelity --value 1000000 \
  2>/dev/null > "$DEMO_DIR/allocate-fidelity.txt"

echo "  → allocate (vanguard)"
"$BINARY" allocate \
  --profile moderate --brokerage vanguard --value 750000 \
  2>/dev/null > "$DEMO_DIR/allocate-vanguard.txt"

echo "  → geo-risk (calm)"
"$BINARY" geo-risk \
  2>/dev/null > "$DEMO_DIR/geo-risk-calm.txt"

echo "  → geo-risk (crisis)"
"$BINARY" geo-risk \
  --oil 140 --vix 38 --gold-3m 0.20 --dxy-3m=-0.08 \
  2>/dev/null > "$DEMO_DIR/geo-risk-crisis.txt"

echo "  → sell-plan"
"$BINARY" sell-plan \
  --ticker AMZN --shares 2000 --cost-basis 60 --price 215 \
  --state CA --filing single --other-income 150000 --years 2 \
  2>/dev/null > "$DEMO_DIR/sell-plan.txt"

echo "  → div-report"
"$BINARY" div-report \
  --ticker AMZN --shares 2000 --cost-basis 60 --price 215 \
  --state CA --filing single --other-income 150000 --other-value 80000 \
  --years 2 --brokerage fidelity --profile moderate \
  --output "$DEMO_DIR/sample-report.html" \
  2>/dev/null > "$DEMO_DIR/div-report.txt"

echo ""
echo "All demos recorded in $DEMO_DIR/"
ls -1 "$DEMO_DIR/"
