# Scheduling Ghost Alpha — Daily Screens & Weekly Summaries

Run Ghost Alpha automatically on weekdays after market close, with a weekly
summary report every Friday.

---

## Table of Contents

1. [Overview](#overview)
2. [The Wrapper Script](#the-wrapper-script)
3. [Option A: cron (macOS / Linux)](#option-a-cron-macos--linux)
4. [Option B: launchd (macOS native)](#option-b-launchd-macos-native)
5. [Option C: systemd (Linux)](#option-c-systemd-linux)
6. [Option D: Docker + cron](#option-d-docker--cron)
7. [Weekly Summary Details](#weekly-summary-details)
8. [Monitoring & Logs](#monitoring--logs)
9. [Troubleshooting](#troubleshooting)

---

## Overview

**Schedule:**

| What | When | Duration |
|------|------|----------|
| Daily screen | Mon–Fri, 18:00 ET (after market close) | ~8 sec (delta cache) / ~8 min (first run) |
| Weekly summary | Friday, 18:15 ET (after daily screen) | ~2 sec (DB queries only) |
| Quarterly plan projection | Last Friday of Mar/Jun/Sep/Dec, 19:00 ET | ~15 sec (5000 paths, 5 years) |

**Data flow with delta caching:**

```
Mon 18:00  ──▶  delta fetch (1 day)  ──▶  screen  ──▶  alerts  ──▶  DB
Tue 18:00  ──▶  delta fetch (1 day)  ──▶  screen  ──▶  alerts  ──▶  DB
Wed 18:00  ──▶  delta fetch (1 day)  ──▶  screen  ──▶  alerts  ──▶  DB
Thu 18:00  ──▶  delta fetch (1 day)  ──▶  screen  ──▶  alerts  ──▶  DB
Fri 18:00  ──▶  delta fetch (1 day)  ──▶  screen  ──▶  alerts  ──▶  DB
Fri 18:15  ──▶  weekly summary (reads DB, no API calls)  ──▶  report
```

The delta cache means only the first run of the week fetches significant data.
Subsequent runs pull a single day of candles per ticker (~500 API calls, ~8 seconds).
Every 7 days the cache does a full refresh to pick up split/dividend adjustments.

---

## The Wrapper Script

Save as `scripts/daily-screen.sh` in the project root. This is the single
entry point for both daily screening and weekly summaries.

```bash
#!/usr/bin/env bash
# ─────────────────────────────────────────────────────────────────────────────
# Ghost Alpha — Daily Screen + Weekly Summary + Quarterly Plan Projection
# ─────────────────────────────────────────────────────────────────────────────
# Usage:
#   ./scripts/daily-screen.sh              # daily screen only
#   ./scripts/daily-screen.sh --summary    # daily screen + weekly summary
#   SUMMARY_ONLY=1 ./scripts/daily-screen.sh --summary  # summary without screen
#   ./scripts/daily-screen.sh --plan       # quarterly forward-NAV projection (run quarterly)
#
# Schedule with cron:
#   0  18 * * 1-5  /path/to/scripts/daily-screen.sh
#   15 18 * * 5    /path/to/scripts/daily-screen.sh --summary
#   0  19 28-31 3,6,9,12 5  /path/to/scripts/daily-screen.sh --plan  # Quarterly (last Fri Q-end)
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
            # Extract tickers and scores, compare
            python3 -c "
import json, sys
try:
    mon = {r['ticker']: r['composite_score'] for r in json.load(open('$MON_JSON'))}
    fri = {r['ticker']: r['composite_score'] for r in json.load(open('$FRI_JSON'))}
    all_tickers = sorted(set(mon) | set(fri), key=lambda t: fri.get(t, 0), reverse=True)
    print(f\"{'Ticker':<8} {'Mon':>7} {'Fri':>7} {'Change':>8}\")
    print('─' * 32)
    for t in all_tickers[:20]:
        m = mon.get(t, 0)
        f = fri.get(t, 0)
        d = f - m
        arrow = '▲' if d > 0 else '▼' if d < 0 else '─'
        if t not in mon:
            arrow = '★'  # new entry
        elif t not in fri:
            arrow = '✗'  # dropped out
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
```

> **Note:** The script uses `python3` for the Mon→Fri score diff. If Python is not
> available, that section is gracefully skipped.

---

## Option A: cron (macOS / Linux)

The simplest option. Works on both macOS and Linux.

### Step 1: Make the script executable

```bash
mkdir -p scripts
# Copy the script above to scripts/daily-screen.sh
chmod +x scripts/daily-screen.sh
```

### Step 2: Edit your crontab

```bash
crontab -e
```

Add these two lines:

```cron
# Ghost Alpha — daily screen Mon-Fri at 18:00 ET
0  18 * * 1-5  /path/to/ghost-alpha/scripts/daily-screen.sh >> /path/to/ghost-alpha/logs/cron.log 2>&1

# Ghost Alpha — weekly summary on Friday at 18:15 ET
15 18 * * 5    /path/to/ghost-alpha/scripts/daily-screen.sh --summary >> /path/to/ghost-alpha/logs/cron.log 2>&1
```

### Step 3: Verify

```bash
# Check crontab is saved
crontab -l

# Test manually
./scripts/daily-screen.sh
./scripts/daily-screen.sh --summary
```

### Timezone Note

cron uses the system timezone. If your system is UTC but you want US market hours:

```cron
# 18:00 ET = 22:00 UTC (EST) or 23:00 UTC (EDT)
# Use 23:00 UTC to cover both (run slightly late during EST)
0  23 * * 1-5  /path/to/ghost-alpha/scripts/daily-screen.sh
15 23 * * 5    /path/to/ghost-alpha/scripts/daily-screen.sh --summary
```

Or set the timezone for cron:
```cron
CRON_TZ=America/New_York
0  18 * * 1-5  /path/to/ghost-alpha/scripts/daily-screen.sh
15 18 * * 5    /path/to/ghost-alpha/scripts/daily-screen.sh --summary
```

---

## Option B: launchd (macOS native)

macOS's native scheduler. More robust than cron for laptops (handles sleep/wake).

### Step 1: Create the daily screen plist

Save as `~/Library/LaunchAgents/com.ghostalpha.daily-screen.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.ghostalpha.daily-screen</string>

    <key>ProgramArguments</key>
    <array>
        <string>/path/to/ghost-alpha/scripts/daily-screen.sh</string>
    </array>

    <key>WorkingDirectory</key>
    <string>/path/to/ghost-alpha</string>

    <key>StartCalendarInterval</key>
    <array>
        <!-- Monday through Friday at 18:00 -->
        <dict><key>Weekday</key><integer>1</integer><key>Hour</key><integer>18</integer><key>Minute</key><integer>0</integer></dict>
        <dict><key>Weekday</key><integer>2</integer><key>Hour</key><integer>18</integer><key>Minute</key><integer>0</integer></dict>
        <dict><key>Weekday</key><integer>3</integer><key>Hour</key><integer>18</integer><key>Minute</key><integer>0</integer></dict>
        <dict><key>Weekday</key><integer>4</integer><key>Hour</key><integer>18</integer><key>Minute</key><integer>0</integer></dict>
        <dict><key>Weekday</key><integer>5</integer><key>Hour</key><integer>18</integer><key>Minute</key><integer>0</integer></dict>
    </array>

    <key>StandardOutPath</key>
    <string>/path/to/ghost-alpha/logs/launchd-daily.log</string>
    <key>StandardErrorPath</key>
    <string>/path/to/ghost-alpha/logs/launchd-daily.err</string>

    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/usr/local/bin:/usr/bin:/bin</string>
    </dict>
</dict>
</plist>
```

### Step 2: Create the Friday summary plist

Save as `~/Library/LaunchAgents/com.ghostalpha.weekly-summary.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.ghostalpha.weekly-summary</string>

    <key>ProgramArguments</key>
    <array>
        <string>/path/to/ghost-alpha/scripts/daily-screen.sh</string>
        <string>--summary</string>
    </array>

    <key>WorkingDirectory</key>
    <string>/path/to/ghost-alpha</string>

    <key>StartCalendarInterval</key>
    <dict>
        <!-- Friday at 18:15 (after daily screen completes) -->
        <key>Weekday</key><integer>5</integer>
        <key>Hour</key><integer>18</integer>
        <key>Minute</key><integer>15</integer>
    </dict>

    <key>StandardOutPath</key>
    <string>/path/to/ghost-alpha/logs/launchd-summary.log</string>
    <key>StandardErrorPath</key>
    <string>/path/to/ghost-alpha/logs/launchd-summary.err</string>

    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>/usr/local/bin:/usr/bin:/bin</string>
    </dict>
</dict>
</plist>
```

### Step 3: Load the agents

```bash
# Replace /path/to/ghost-alpha with your actual path in both plist files, then:
launchctl load ~/Library/LaunchAgents/com.ghostalpha.daily-screen.plist
launchctl load ~/Library/LaunchAgents/com.ghostalpha.weekly-summary.plist

# Verify
launchctl list | grep ghostalpha
```

### Step 4: Test manually

```bash
# Trigger immediately without waiting for schedule
launchctl start com.ghostalpha.daily-screen
launchctl start com.ghostalpha.weekly-summary

# Check logs
tail -f /path/to/ghost-alpha/logs/launchd-daily.log
```

### To unload

```bash
launchctl unload ~/Library/LaunchAgents/com.ghostalpha.daily-screen.plist
launchctl unload ~/Library/LaunchAgents/com.ghostalpha.weekly-summary.plist
```

> **Why launchd over cron on macOS?** If your Mac sleeps at 18:00, cron misses
> the job entirely. launchd runs it as soon as the Mac wakes up (within the
> same calendar day).

---

## Option C: systemd (Linux)

### Step 1: Service unit

`/etc/systemd/system/ghost-alpha-screen.service`:

```ini
[Unit]
Description=Ghost Alpha daily stock screening
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
User=ghost-alpha
WorkingDirectory=/opt/ghost-alpha
ExecStart=/opt/ghost-alpha/scripts/daily-screen.sh
StandardOutput=journal
StandardError=journal
```

### Step 2: Daily timer

`/etc/systemd/system/ghost-alpha-screen.timer`:

```ini
[Unit]
Description=Ghost Alpha daily screen Mon-Fri 18:00 ET

[Timer]
OnCalendar=Mon..Fri 18:00 America/New_York
Persistent=true

[Install]
WantedBy=timers.target
```

### Step 3: Weekly summary service + timer

`/etc/systemd/system/ghost-alpha-summary.service`:

```ini
[Unit]
Description=Ghost Alpha weekly summary
After=ghost-alpha-screen.service

[Service]
Type=oneshot
User=ghost-alpha
WorkingDirectory=/opt/ghost-alpha
ExecStart=/opt/ghost-alpha/scripts/daily-screen.sh --summary
Environment=SUMMARY_ONLY=1
StandardOutput=journal
StandardError=journal
```

`/etc/systemd/system/ghost-alpha-summary.timer`:

```ini
[Unit]
Description=Ghost Alpha weekly summary Friday 18:15 ET

[Timer]
OnCalendar=Fri 18:15 America/New_York
Persistent=true

[Install]
WantedBy=timers.target
```

### Step 4: Enable

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now ghost-alpha-screen.timer
sudo systemctl enable --now ghost-alpha-summary.timer

# Check timers
systemctl list-timers | grep ghost

# View logs
journalctl -u ghost-alpha-screen -f
journalctl -u ghost-alpha-summary -f
```

---

## Option D: Docker + cron

### Step 1: Build the image

```bash
docker build -t ghost-alpha .
```

### Step 2: Create a Docker wrapper script

Save as `scripts/docker-screen.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

IMAGE="ghost-alpha"
VOLUME="ghost-alpha-data"

# Daily screen
docker run --rm \
    --env-file /path/to/ghost-alpha/.env \
    -e GHOST_ALPHA_DB_PATH=/data/ghost_alpha.db \
    -v "${VOLUME}:/data" \
    "$IMAGE" screen --top 50

# Weekly summary (pass --summary to trigger)
if [[ "${1:-}" == "--summary" ]]; then
    docker run --rm \
        --env-file /path/to/ghost-alpha/.env \
        -e GHOST_ALPHA_DB_PATH=/data/ghost_alpha.db \
        -v "${VOLUME}:/data" \
        "$IMAGE" regime

    docker run --rm \
        --env-file /path/to/ghost-alpha/.env \
        -e GHOST_ALPHA_DB_PATH=/data/ghost_alpha.db \
        -v "${VOLUME}:/data" \
        "$IMAGE" alerts --last 50
fi
```

### Step 3: Schedule with cron

```cron
0  18 * * 1-5  /path/to/scripts/docker-screen.sh >> /var/log/ghost-alpha.log 2>&1
15 18 * * 5    /path/to/scripts/docker-screen.sh --summary >> /var/log/ghost-alpha.log 2>&1
```

---

## Weekly Summary Details

The Friday summary includes these sections:

| Section | Source | What It Shows |
|---------|--------|---------------|
| **Market Regime** | `ghost-alpha regime` | Bull / Sideways / Bear + SPY metrics |
| **Top Stocks** | `ghost-alpha screen` | Friday's top 20 ranked by composite score |
| **Alerts This Week** | `ghost-alpha alerts` | All alerts fired during the week |
| **Watchlist** | `ghost-alpha watch list` | Current watchlist with latest scores |
| **Score Changes** | JSON diff (Mon vs Fri) | Which stocks improved/declined + new entries/exits |
| **Strategy Comparison** | 4x screen runs | Top 5 from each strategy side-by-side |

Output is saved to `reports/weekly-YYYY-MM-DD.txt` and also printed to stdout
(which cron/launchd captures to the log file).

### Example summary output

```
╔═══════════════════════════════════════════════════════════════╗
║          GHOST ALPHA — WEEKLY SUMMARY                       ║
║          Week of 2026-03-09 to 2026-03-13                   ║
╚═══════════════════════════════════════════════════════════════╝

── Market Regime ─────────────────────────────────────────────
  Regime: Bull | SPY: $542.10 | SMA200: $498.33 | Breadth: 67%

── Top 50 Stocks (Friday Close) ──────────────────────────────
  NVDA    97.2  A+  | PLTR   94.1  A+ | ...

── Score Changes (Mon → Fri) ──────────────────────────────────
  Ticker     Mon     Fri   Change
  ─────────────────────────────────
  NVDA      95.1    97.2  ▲  +2.1
  AMD       82.4    78.1  ▼  -4.3
  SMCI       0.0    88.7  ★ +88.7   (new entry)

── Strategy Comparison (Top 5 per strategy) ────────────────────
  [minervini]   NVDA  PLTR  APP   ANET  CEG
  [breakout]    SMCI  MSTR  COIN  HOOD  RIOT
  ...
```

---

## Monitoring & Logs

### Log locations

| File | Contents |
|------|----------|
| `logs/screen-YYYY-MM-DD.log` | Daily screen console output |
| `logs/results-YYYY-MM-DD.json` | Daily results in JSON (for programmatic use) |
| `reports/weekly-YYYY-MM-DD.txt` | Weekly summary report |
| `logs/cron.log` | cron stderr/stdout (if configured) |

### Health checks

```bash
# Check last run time
ls -lt logs/screen-*.log | head -1

# Check database size and last screen date
sqlite3 ghost_alpha.db "SELECT MAX(run_date) FROM screen_results;"
sqlite3 ghost_alpha.db "SELECT COUNT(DISTINCT run_date) FROM screen_results;"

# Check cache status (latest candle dates)
sqlite3 ghost_alpha.db "SELECT ticker, MAX(date) FROM candles GROUP BY ticker LIMIT 5;"
```

### Auto-cleanup

The wrapper script automatically deletes:
- Logs older than 30 days
- JSON results older than 30 days
- Weekly summaries older than 90 days

---

## Troubleshooting

| Problem | Cause | Fix |
|---------|-------|-----|
| cron job doesn't run | PATH not set | Use absolute paths in cron, or add `PATH=/usr/local/bin:/usr/bin:/bin` at the top of crontab |
| launchd job missed | Mac was asleep | launchd runs on wake — check if the `Persistent` key is missing (it should run on next wake) |
| `Binary not found` | Release binary not built | Run `cargo build --release` |
| `.env not loaded` | Wrong GHOST_DIR | Set `GHOST_DIR=/absolute/path` in crontab or export it |
| No score diff on Friday | Monday JSON missing | Only works after a full Mon–Fri week of runs |
| Summary shows `(screen failed)` | Config not found | Ensure `config.toml` exists at `GHOST_DIR` or use env-only mode |
| Timezone wrong | System TZ differs from market TZ | Use `CRON_TZ=America/New_York` in crontab or adjust hours |
| Docker volume empty | First run needs full fetch | Run manually once first: `docker run ... screen` |
