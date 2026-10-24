# Ghost Alpha — Step-by-Step Setup Guide

Complete guide covering developer setup, user configuration, and production scheduling.

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Part A — Developer Setup (Build from Source)](#part-a--developer-setup-build-from-source)
3. [Part B — User Setup (Configure & Run)](#part-b--user-setup-configure--run)
4. [Part C — Production / Cron Setup (Daily Delta Runs)](#part-c--production--cron-setup-daily-delta-runs)
5. [Configuration Reference](#configuration-reference)
6. [Troubleshooting](#troubleshooting)

---

## 1. Prerequisites

| Requirement        | Version  | Notes                                              |
|--------------------|----------|----------------------------------------------------|
| **Rust toolchain** | 1.85+    | Install via [rustup.rs](https://rustup.rs)         |
| **Git**            | 2.x+     | For cloning the repo                               |
| **SQLite**         | bundled  | Compiled into the binary (`rusqlite` bundled flag)  |
| **Docker**         | 24+      | Optional — only for containerized deployment       |

**API Keys (at least one required for live data):**

| Provider   | Cost | Rate Limit | How to Get                                                  |
|------------|------|------------|-------------------------------------------------------------|
| Finnhub    | Free | 60 req/min | [finnhub.io/register](https://finnhub.io/register)         |
| Yahoo      | Free | ~5 req/sec | No signup — use `--provider yahoo`                          |

> **Tip:** You can try Ghost Alpha immediately with zero API keys using `ghost-alpha demo`.

---

## Part A — Developer Setup (Build from Source)

### Step 1: Clone the repository

```bash
git clone https://github.com/wojciechkpl/ghost-alpha.git
cd ghost-alpha
```

### Step 2: Build the binary

```bash
# Debug build (fast compilation, slower runtime)
cargo build

# Release build (slower compilation, optimized binary ~6MB)
cargo build --release
```

### Step 3: Verify the build

```bash
# Check version
./target/release/ghost-alpha --version

# Run bundled demo (no config/API key needed)
./target/release/ghost-alpha demo
```

Expected output: a table of ~50 sample stocks scored by the Minervini screener.

### Step 4: Run the test suite

```bash
# Unit tests (fast, ~995 tests, <2 seconds)
cargo test --lib --bins

# Integration tests (requires --features flag)
cargo test --features integration

# Single module tests (useful during development)
cargo test --lib data::cache
cargo test --lib analysis::screener
```

### Step 5: Lint and format

```bash
cargo fmt --check          # Check formatting
cargo fmt                  # Auto-format
cargo clippy -- -D warnings  # Lint with warnings as errors
```

### Optional: Build with extra features

```bash
# MCP server support (for AI agent integration)
cargo build --release --features mcp

# Sentiment scoring (requires ONNX runtime)
cargo build --release --features sentiment

# All features
cargo build --release --features "mcp sentiment"
```

---

## Part B — User Setup (Configure & Run)

You have three paths: **Interactive Wizard** (recommended), **Manual Config**, or **Env-Only Mode**.

### Path 1: Interactive Wizard (Recommended)

```bash
ghost-alpha setup
```

The wizard walks you through 5 steps:

1. **Finnhub API Key** — paste your key (or press Enter to skip for Yahoo)
2. **Telegram Alerts** — bot token + chat ID (optional, press Enter to skip)
3. **Discord Alerts** — webhook URL (optional, press Enter to skip)
4. **Database Path** — where to store SQLite data (default: `ghost_alpha.db`)
5. **Sentiment Model** — download FinBERT ONNX (~400MB, optional)

The wizard generates two files:

| File          | Contents                          | Commit to git? |
|---------------|-----------------------------------|----------------|
| `config.toml` | Non-sensitive settings            | Yes            |
| `.env`         | API keys and tokens (secrets)     | **Never**      |

### Path 2: Manual Configuration

```bash
# Copy the annotated example
cp config.example.toml config.toml
cp .env.example .env

# Edit with your values
$EDITOR .env         # Add API keys
$EDITOR config.toml  # Adjust thresholds, universe, etc.
```

**Minimal `.env` for Finnhub:**
```bash
GHOST_ALPHA_FINNHUB_API_KEY=your_key_here
```

**Minimal `.env` for Yahoo (no API key):**
```bash
# Leave empty — Yahoo needs no key, just set provider in config.toml
```

**Minimal `config.toml` for Yahoo:**
```toml
[finnhub]
api_key = ""

[plugins]
active_provider = "yahoo"
```

### Path 3: Env-Only Mode (Docker / CI)

No config file needed — everything via environment variables:

```bash
export GHOST_ALPHA_FINNHUB_API_KEY=your_key_here
export GHOST_ALPHA_DB_PATH=./ghost_alpha.db
ghost-alpha screen
```

### Step 6: Run your first live screen

```bash
# With Finnhub (default provider)
ghost-alpha screen

# With Yahoo (no API key needed)
ghost-alpha screen --provider yahoo

# Show top 10, JSON output
ghost-alpha screen --top 10 --format json

# Try different strategies
ghost-alpha screen --strategy breakout
ghost-alpha screen --strategy mean-reversion
ghost-alpha screen --strategy trend-strength
```

### Step 7: Verify configuration

```bash
# Print loaded config (secrets masked)
ghost-alpha show-config

# List registered plugins
ghost-alpha plugins

# List available strategies
ghost-alpha strategies
```

### Step 8: Start the web dashboard

```bash
ghost-alpha serve
# Dashboard running at http://127.0.0.1:3000

# Custom port/bind
ghost-alpha serve --port 8080 --bind 0.0.0.0
```

### Step 9: Set up alerts (Optional)

**Telegram:**
1. Message [@BotFather](https://t.me/BotFather) on Telegram → `/newbot` → copy the token
2. Add the bot to your group/channel
3. Message [@userinfobot](https://t.me/userinfobot) to get your chat ID
4. Add to `.env`:
   ```bash
   GHOST_ALPHA_TELEGRAM_BOT_TOKEN=123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11
   GHOST_ALPHA_TELEGRAM_CHAT_ID=-1001234567890
   ```

**Discord:**
1. Server Settings → Integrations → Webhooks → New Webhook → Copy URL
2. Add to `.env`:
   ```bash
   GHOST_ALPHA_DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/...
   ```

### Step 10: Set up the watchlist (Optional)

```bash
ghost-alpha watch add NVDA AAPL TSLA AMD
ghost-alpha watch list
ghost-alpha watch remove TSLA
```

### Step 11: Forward NAV Projection (Concentrated Portfolios)

For RSU-heavy or concentrated portfolios, `ghost-alpha plan` projects forward NAV under three vest treatments using Monte Carlo simulation.

**Prerequisites:**
- Positions file(s): a Fidelity Portfolio_Positions export or a simple `symbol,quantity,price[,cost_basis][,account]` CSV
- Optional: a vesting schedule (`symbol,vesting_date,quantity`)
- Optional: `symbol_map.toml` for tickers outside the built-in map
- `config/asset_classes.toml` (included; 19 asset classes with μ/σ)

**Quick start:**

```bash
ghost-alpha init-data my_data      # editable templates; my_data/ is gitignored
ghost-alpha plan --data-dir my_data --as-of 2026-05-01 --out-dir ./plan_output
```

**Output:** Three files in `plan_output/`:
- `nav_projection.csv` — Year-end percentile bands (P5/P25/P50/P75/P95)
- `nav_mc_paths.csv` — Raw simulation paths (for advanced analysis)
- `audit.json` — Reproducibility metadata (seed, path count, percentiles)

**See also:** [`docs/PLAN_PIPELINE.md`](PLAN_PIPELINE.md) for full CLI reference, output schema, and library usage.

---

## Part C — Production / Cron Setup (Daily Delta Runs)

Ghost Alpha now includes **delta caching**: the first run fetches full history (~400 days), subsequent runs only fetch new candles since the last stored date. A weekly full refresh catches retroactive price adjustments.

### Option 1: Cron (Bare Metal / VM)

```bash
# Edit your crontab
crontab -e
```

Add one of these schedules:

```cron
# ── Daily screen at 6:00 PM ET (after market close), weekdays only ──
0 18 * * 1-5  cd /path/to/ghost-alpha && ./target/release/ghost-alpha screen --top 20 >> /var/log/ghost-alpha.log 2>&1

# ── With JSON output appended to a file ──
0 18 * * 1-5  cd /path/to/ghost-alpha && ./target/release/ghost-alpha screen --format json >> /var/log/ghost-alpha-results.jsonl 2>&1

# ── Dashboard always-on (start at boot via @reboot) ──
@reboot cd /path/to/ghost-alpha && ./target/release/ghost-alpha serve >> /var/log/ghost-alpha-dashboard.log 2>&1
```

### Option 2: Systemd Timer (Linux)

**Service unit** — `/etc/systemd/system/ghost-alpha.service`:
```ini
[Unit]
Description=Ghost Alpha daily stock screening
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
User=ghost-alpha
WorkingDirectory=/opt/ghost-alpha
EnvironmentFile=/opt/ghost-alpha/.env
ExecStart=/opt/ghost-alpha/ghost-alpha screen --top 20
StandardOutput=journal
StandardError=journal
```

**Timer unit** — `/etc/systemd/system/ghost-alpha.timer`:
```ini
[Unit]
Description=Run Ghost Alpha screening weekdays at 18:00 ET

[Timer]
OnCalendar=Mon..Fri 18:00 America/New_York
Persistent=true

[Install]
WantedBy=timers.target
```

**Enable:**
```bash
sudo systemctl daemon-reload
sudo systemctl enable --now ghost-alpha.timer
sudo systemctl list-timers | grep ghost
```

### Option 3: Docker + Cron

**Build the image:**
```bash
docker build -t ghost-alpha .
```

**Run manually:**
```bash
docker run --rm \
  -e GHOST_ALPHA_FINNHUB_API_KEY=your_key \
  -v ghost-alpha-data:/data \
  ghost-alpha screen --top 20
```

**Schedule via cron:**
```cron
0 18 * * 1-5  docker run --rm -e GHOST_ALPHA_FINNHUB_API_KEY=your_key -v ghost-alpha-data:/data ghost-alpha screen --top 20 >> /var/log/ghost-alpha.log 2>&1
```

**Or use Docker Compose:**
```bash
# One-off run
docker compose run --rm ghost-alpha

# Schedule in cron
0 18 * * 1-5  docker compose -f /path/to/docker-compose.yml run --rm ghost-alpha
```

### Option 4: launchd (macOS)

**Plist** — `~/Library/LaunchAgents/com.ghostalpha.screen.plist`:
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.ghostalpha.screen</string>
    <key>ProgramArguments</key>
    <array>
        <string>/path/to/ghost-alpha</string>
        <string>screen</string>
        <string>--top</string>
        <string>20</string>
    </array>
    <key>WorkingDirectory</key>
    <string>/path/to/ghost-alpha</string>
    <key>StartCalendarInterval</key>
    <array>
        <!-- Mon-Fri at 18:00 -->
        <dict><key>Weekday</key><integer>1</integer><key>Hour</key><integer>18</integer><key>Minute</key><integer>0</integer></dict>
        <dict><key>Weekday</key><integer>2</integer><key>Hour</key><integer>18</integer><key>Minute</key><integer>0</integer></dict>
        <dict><key>Weekday</key><integer>3</integer><key>Hour</key><integer>18</integer><key>Minute</key><integer>0</integer></dict>
        <dict><key>Weekday</key><integer>4</integer><key>Hour</key><integer>18</integer><key>Minute</key><integer>0</integer></dict>
        <dict><key>Weekday</key><integer>5</integer><key>Hour</key><integer>18</integer><key>Minute</key><integer>0</integer></dict>
    </array>
    <key>StandardOutPath</key>
    <string>/tmp/ghost-alpha.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/ghost-alpha.err</string>
</dict>
</plist>
```

**Load:**
```bash
launchctl load ~/Library/LaunchAgents/com.ghostalpha.screen.plist
launchctl list | grep ghostalpha
```

### Delta Cache Configuration

The cache is **enabled by default**. To tune it, add to `config.toml`:

```toml
[cache]
# Enable delta fetching (default: true)
enabled = true
# Days between full refreshes to catch adj_close corrections (default: 7)
full_refresh_days = 7
```

**How it behaves in production:**

| Run # | Day     | What Happens                                        | API Calls      | Time    |
|-------|---------|-----------------------------------------------------|----------------|---------|
| 1     | Monday  | Full fetch — 400 days × ~500 tickers                | ~500           | ~8 min  |
| 2     | Tuesday | Delta — 1 new day per ticker                         | ~500 (1 day)   | ~8 sec  |
| 3     | Wednesday | Delta — 1 new day per ticker                       | ~500 (1 day)   | ~8 sec  |
| ...   | ...     | Delta continues...                                   | ~500 (1 day)   | ~8 sec  |
| 6     | Monday  | Full refresh (7 days since last full) — catches splits/dividends | ~500   | ~8 min  |

**To force a full refresh on demand:**
```toml
[cache]
enabled = false  # Temporarily disable, then re-enable after one run
```

---

## Configuration Reference

### Environment Variables

| Variable                           | Required | Default            | Description                       |
|------------------------------------|----------|--------------------|-----------------------------------|
| `GHOST_ALPHA_FINNHUB_API_KEY`     | Yes*     | —                  | Finnhub API key                   |
| `GHOST_ALPHA_TELEGRAM_BOT_TOKEN`  | No       | —                  | Telegram bot token                |
| `GHOST_ALPHA_TELEGRAM_CHAT_ID`    | No       | —                  | Telegram chat/group ID            |
| `GHOST_ALPHA_DISCORD_WEBHOOK_URL` | No       | —                  | Discord webhook URL               |
| `GHOST_ALPHA_WEBHOOK_URL`         | No       | —                  | Generic webhook for BYOA          |
| `GHOST_ALPHA_WEBHOOK_AUTH_HEADER` | No       | —                  | Auth header for generic webhook   |
| `GHOST_ALPHA_DB_PATH`             | No       | `ghost_alpha.db`   | SQLite database file path         |
| `GHOST_ALPHA_LOG_LEVEL`           | No       | `info`             | trace / debug / info / warn / error |
| `GHOST_ALPHA_MCP_ENABLED`         | No       | `false`            | Enable MCP server mode            |

*Not required when using `--provider yahoo`.

### config.toml Sections

| Section       | Key Fields                                    | Defaults                     |
|---------------|-----------------------------------------------|------------------------------|
| `[finnhub]`   | `api_key`, `requests_per_minute`, `retry_delay_secs` | `""`, `60`, `5`        |
| `[alerts]`    | `critical_score_threshold`, `warning_score_threshold`, `info_score_threshold` | `90`, `70`, `50` |
| `[storage]`   | `db_path`                                     | `ghost_alpha.db`             |
| `[plugins]`   | `active_provider`, `active_screener`          | `finnhub`, `minervini`       |
| `[cache]`     | `enabled`, `full_refresh_days`                | `true`, `7`                  |
| `[telegram]`  | `bot_token`, `chat_id`                        | —                            |
| `[discord]`   | `webhook_url`                                 | —                            |
| `[sentiment]` | `model_path`, `news_lookback_days`, `min_articles` | —, `7`, `3`            |
| `[backtest]`  | `default_years`, `forward_windows`, `monte_carlo_iterations` | `5`, `[30,60,90]`, `10000` |

### CLI Quick Reference

```
ghost-alpha demo                          # Bundled sample data, no config needed
ghost-alpha setup                         # Interactive wizard
ghost-alpha screen                        # Run screening pipeline
ghost-alpha screen --provider yahoo       # Use Yahoo (no API key)
ghost-alpha screen --strategy breakout    # Alternative strategy
ghost-alpha screen --top 10 --format json # Top 10, JSON output
ghost-alpha strategies                    # List all strategies
ghost-alpha regime                        # Show market regime (Bull/Sideways/Bear)
ghost-alpha alerts --last 50              # Recent alert log
ghost-alpha watch add NVDA AAPL           # Add to watchlist
ghost-alpha watch list                    # Show watchlist
ghost-alpha backtest --years 3            # Walk-forward backtest
ghost-alpha serve                         # Web dashboard on :3000
ghost-alpha serve --port 8080             # Custom port
ghost-alpha show-config                   # Print config (secrets masked)
ghost-alpha plugins                       # List registered plugins
ghost-alpha migrate                       # Run DB migrations only
ghost-alpha mcp --env-only                # Start MCP server (AI agents)
```

---

## Troubleshooting

| Problem | Cause | Fix |
|---------|-------|-----|
| `MissingField: finnhub.api_key` | No API key configured | Set `GHOST_ALPHA_FINNHUB_API_KEY` in `.env`, or use `--provider yahoo` |
| `Rate limit exceeded` | Too many API calls | Reduce `requests_per_minute` in config, or switch to Yahoo |
| `Failed to open database` | Invalid path or permissions | Check `GHOST_ALPHA_DB_PATH`, ensure parent directory exists |
| `Skipping ticker (fetch error)` on many tickers | Network issues or API downtime | Check internet connection; retry in a few minutes |
| Stale prices after a stock split | Cache has old adj_close values | Set `cache.full_refresh_days = 1` temporarily, or disable cache for one run |
| `FOREIGN KEY constraint failed` | Database schema mismatch | Run `ghost-alpha migrate` to update schema |
| Binary too large | Debug build | Use `cargo build --release` (~6MB vs ~60MB debug) |
| Cron job not running | Wrong path or missing env | Use absolute paths in cron; source `.env` or use `EnvironmentFile=` in systemd |
| Dashboard shows no data | No screening runs yet | Run `ghost-alpha screen` first to populate the database |
