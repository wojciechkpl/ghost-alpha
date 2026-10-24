# GhostAlpha Tutorial

A step-by-step guide to setting up, configuring, and using GhostAlpha as both a CLI tool and an MCP server for AI agents.

---

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [Installation](#installation)
3. [Configuration](#configuration)
4. [CLI Usage](#cli-usage)
   - [Quick Start](#quick-start)
   - [Screening & Analysis](#screen-stocks)
   - [Watchlist & Backtesting](#watchlist-management)
   - [Forward NAV Projection](#forward-nav-projection-concentrated-portfolios)
   - [Dashboard & Monitoring](#web-dashboard)
5. [MCP Server Setup](#mcp-server-setup)
6. [Claude Desktop Integration](#claude-desktop-integration)
7. [Goose Integration](#goose-integration)
8. [Bring Your Own Agent](#bring-your-own-agent)
9. [Plugin System](#plugin-system)
10. [Troubleshooting](#troubleshooting)

---

## Prerequisites

- **Rust** 1.70+ — [Install via rustup](https://rustup.rs/)
- **Finnhub API key** (optional) — [Free registration](https://finnhub.io/register) (60 requests/minute). Not required when using `--provider yahoo` or `ghost-alpha demo`.
- **SQLite** — bundled with the project (no separate install needed)
- **Telegram Bot** (optional) — Create via [@BotFather](https://t.me/BotFather)
- **Discord Webhook** (optional) — Create via Discord server settings

---

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/wojciechkpl/ghost-alpha.git
cd ghost-alpha

# Build both binaries (CLI + MCP server)
cargo build --release

# The binaries are at:
#   target/release/ghost-alpha       # CLI tool
#   target/release/ghost-alpha-mcp   # MCP server

# Optional: install to PATH
cargo install --path .
```

### Verify Installation

```bash
ghost-alpha --version
ghost-alpha-mcp --version
```

### Try Instantly (Zero Config)

```bash
# Run with bundled sample data — no API key or config file needed
ghost-alpha demo
```

---

## Configuration

GhostAlpha supports three configuration methods with clear precedence:

```
Environment Variables  >  TOML Config File  >  Built-in Defaults
     (highest)                                      (lowest)
```

### Method 1: Environment Variables (Recommended for MCP)

```bash
# Copy the example and fill in values
cp .env.example .env

# Required only when using the finnhub provider (not needed with --provider yahoo or demo)
export GHOST_ALPHA_FINNHUB_API_KEY="your-finnhub-api-key"

# Optional — Telegram alerts
export GHOST_ALPHA_TELEGRAM_BOT_TOKEN="123456:ABC-DEF..."
export GHOST_ALPHA_TELEGRAM_CHAT_ID="-1001234567890"

# Optional — Storage
export GHOST_ALPHA_DB_PATH="/path/to/ghost_alpha.db"

# Optional — Logging
export GHOST_ALPHA_LOG_LEVEL="info"  # trace, debug, info, warn, error
```

### Method 2: TOML Config File

```bash
# Copy the example
cp config.example.toml config.toml

# Edit config.toml with your values
```

```toml
[finnhub]
api_key = "your-finnhub-api-key"
requests_per_minute = 60
retry_delay_secs = 5

[telegram]
bot_token = "123456:ABC-DEF..."
chat_id = "-1001234567890"

[alerts]
critical_score_threshold = 90.0
warning_score_threshold  = 70.0
info_score_threshold     = 50.0
dedup_cooldown_days      = 7
max_daily_alerts         = 50

[discord]
webhook_url = "https://discord.com/api/webhooks/..."

[storage]
db_path = "ghost_alpha.db"

[plugins]
provider = "yahoo"       # "finnhub", "yahoo", or "sample"
strategy = "minervini"   # "minervini", "breakout", "mean-reversion", or "trend-strength"
```

### Method 3: Env-Only Mode (No Config File)

```bash
# All config from environment — perfect for Docker/CI
GHOST_ALPHA_FINNHUB_API_KEY=xxx ghost-alpha-mcp --env-only
```

> **Security**: API keys are never hardcoded. The `.gitignore` excludes `.env` and `config.toml`. Only `.env.example` and `config.example.toml` are tracked.

---

## CLI Usage

### Quick Start

```bash
# Zero-config demo with bundled sample data
ghost-alpha demo

# Interactive setup wizard — walks you through config
ghost-alpha setup
```

### Run Database Migrations

```bash
ghost-alpha migrate
# Output: Migrations applied to 'ghost_alpha.db'
```

### Screen Stocks

```bash
# Run the full screening pipeline
ghost-alpha screen

# Skip sentiment scoring (faster, no ONNX needed)
ghost-alpha screen --skip-sentiment

# Use Yahoo Finance as data source (no API key needed)
ghost-alpha screen --provider yahoo

# Use a different screening strategy
ghost-alpha screen --strategy breakout

# Export results in different formats
ghost-alpha screen --format json    # also: csv, jsonl, html
```

### Strategies

```bash
# List all available screening strategies
ghost-alpha strategies
```

### Watchlist Management

```bash
# Add a ticker to the watchlist
ghost-alpha watch add NVDA

# List all watched tickers
ghost-alpha watch list
```

### Backtesting

```bash
# Run a backtest with HTML report output
ghost-alpha backtest --format html
```

### Alerts

```bash
# Show recent alerts
ghost-alpha alerts
```

### Forward NAV Projection (Concentrated Portfolios)

Project forward NAV for a portfolio with a concentrated stock position (e.g. RSUs), using your own data:

```bash
# Start from templates (my_data/ is gitignored)
ghost-alpha init-data my_data
# ...edit my_data/positions/*.csv (Fidelity export or symbol,quantity,price[,cost_basis]),
#    my_data/vesting.csv (symbol,vesting_date,quantity), optional symbol_map.toml / plan.toml
ghost-alpha plan --data-dir my_data --as-of 2026-05-01

# Or pass files directly
ghost-alpha plan \
  --positions account_a.csv \
  --positions account_b.csv \
  --vesting vesting.csv \
  --symbol MSFT \
  --treatment option-b --unwind "1:400,2:300" --floor-pct 0.25 \
  --paths 10000 --seed 42 --horizon 5 \
  --as-of 2026-05-01 --out-dir ./projection

# Three vest treatments:
#  hold           — accumulate vested shares (concentration grows)
#  sell-reinvest  — sell on vest, redeploy pro-rata (diversified)
#  option-b       — sell on vest + scheduled unwind, never below --floor-pct of NAV
```

**Output:** `nav_projection.csv` (percentile bands), `nav_mc_paths.csv` (raw paths), `audit.json` (metadata)

**See also:** [`docs/PLAN_PIPELINE.md`](docs/PLAN_PIPELINE.md) for full CLI reference, output schema, and reproducibility contract.

### Web Dashboard

```bash
# Start the embedded web dashboard
ghost-alpha serve
```

### Check Market Regime

```bash
ghost-alpha regime
# Output: Regime: BULL (as of 2026-02-27)
```

### View Configuration

```bash
ghost-alpha show-config
```

### Verbose Mode

```bash
ghost-alpha -v screen  # Debug logging
```

---

## MCP Server Setup

The MCP (Model Context Protocol) server exposes GhostAlpha as a set of tools that any AI agent can call.

### Start the MCP Server Manually

```bash
# With config file
ghost-alpha-mcp -c config.toml

# With environment variables only
GHOST_ALPHA_FINNHUB_API_KEY=xxx ghost-alpha-mcp --env-only

# With debug logging (goes to stderr)
ghost-alpha-mcp --env-only --verbose
```

### Available MCP Tools

| Tool | Description |
|------|-------------|
| `screen_stock` | Screen a stock through Minervini 8-point filter. Returns composite score, grade, and signal strength. |
| `score_breakdown` | What-if analysis: compute score from hypothetical indicator values. |
| `get_config` | View server configuration (API keys are masked). |
| `list_providers` | List registered plugins, screeners, scorers, and alert channels. |
| `health_check` | Check server health, database status, and provider connectivity. |

### Available MCP Resources

| URI | Description |
|-----|-------------|
| `ghost-alpha://config` | Server configuration as JSON (secrets masked) |
| `ghost-alpha://providers` | Plugin registry state as JSON |

### Test MCP Server Manually

```bash
# Start the server
ghost-alpha-mcp --env-only &

# Send an initialize request
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | ghost-alpha-mcp --env-only

# List available tools
echo '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}' | ghost-alpha-mcp --env-only

# Call score_breakdown
echo '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"score_breakdown","arguments":{"conditions_met":7,"adx":35,"rs_percentile":82,"volume_zscore":1.5,"sentiment_score":0.3,"article_count":5}}}' | ghost-alpha-mcp --env-only
```

---

## Claude Desktop Integration

### Step 1: Build and Install

```bash
cd ghost-alpha
cargo install --path .
```

### Step 2: Configure Claude Desktop

Edit `~/.config/claude/claude_desktop_config.json` (macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "ghost-alpha": {
      "command": "ghost-alpha-mcp",
      "args": ["--env-only"],
      "env": {
        "GHOST_ALPHA_FINNHUB_API_KEY": "your-finnhub-api-key"
      }
    }
  }
}
```

### Step 3: Restart Claude Desktop

The GhostAlpha tools will appear in Claude's tool list. You can now ask:

- *"Screen NVDA through the Minervini filter"*
- *"What would the composite score be if ADX was 40 and RS was in the 90th percentile?"*
- *"Check the health status of the GhostAlpha server"*
- *"What plugins are available?"*

---

## Goose Integration

### Option 1: Via Goose Settings UI

1. Open Goose Desktop → Settings → Extensions
2. Click "Add Extension"
3. Fill in:
   - **Name**: ghost-alpha
   - **Type**: stdio
   - **Command**: `ghost-alpha-mcp`
   - **Args**: `--env-only`
   - **Environment Variables**:
     - `GHOST_ALPHA_FINNHUB_API_KEY`: your key

### Option 2: Via Configuration File

Add to your Goose profile (typically `~/.config/goose/profiles.yaml`):

```yaml
extensions:
  ghost-alpha:
    type: stdio
    command: ghost-alpha-mcp
    args: ["--env-only"]
    env:
      GHOST_ALPHA_FINNHUB_API_KEY: "your-finnhub-api-key"
      GHOST_ALPHA_DB_PATH: "ghost_alpha.db"
```

### Option 3: Via CLI

```bash
goose configure extensions
# Follow interactive prompts to add ghost-alpha
```

### Usage in Goose

Once configured, you can interact naturally:

```
You: Can you screen Apple stock and tell me if it passes the Minervini trend template?

Goose: I'll use the ghost-alpha screen_stock tool to analyze AAPL...
[Uses screen_stock tool with AAPL candle data]
AAPL passes 6 out of 8 Minervini conditions with a composite score of 72.3 (Grade B)...
```

---

## Bring Your Own Agent

GhostAlpha's MCP server works with **any** MCP-compatible AI agent. Here's how to integrate it.

### Generic MCP Client Setup

Any application that speaks JSON-RPC 2.0 over stdin/stdout can connect:

```python
import subprocess
import json

# Start the MCP server as a subprocess
proc = subprocess.Popen(
    ["ghost-alpha-mcp", "--env-only"],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    env={"GHOST_ALPHA_FINNHUB_API_KEY": "your-key", "PATH": os.environ["PATH"]},
)

# Initialize
request = {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}
proc.stdin.write(json.dumps(request).encode() + b"\n")
proc.stdin.flush()
response = json.loads(proc.stdout.readline())

# List tools
request = {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}
proc.stdin.write(json.dumps(request).encode() + b"\n")
proc.stdin.flush()
tools = json.loads(proc.stdout.readline())

# Call a tool
request = {
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tools/call",
    "params": {
        "name": "score_breakdown",
        "arguments": {
            "conditions_met": 7,
            "adx": 35.0,
            "rs_percentile": 82.0,
            "volume_zscore": 1.5,
            "sentiment_score": 0.3,
            "article_count": 5,
        },
    },
}
proc.stdin.write(json.dumps(request).encode() + b"\n")
proc.stdin.flush()
result = json.loads(proc.stdout.readline())
print(result)
```

### VS Code / Cursor Integration

If your editor supports MCP servers, add to your workspace settings:

```json
{
  "mcp.servers": {
    "ghost-alpha": {
      "command": "ghost-alpha-mcp",
      "args": ["--env-only"],
      "env": {
        "GHOST_ALPHA_FINNHUB_API_KEY": "your-key"
      }
    }
  }
}
```

### Docker-Based Deployment

The repository includes a `Dockerfile` and `docker-compose.yml`. The Dockerfile uses a multi-stage build with dependency caching:

```dockerfile
# ── Stage 1: Build ────────────────────────────────────────────
FROM rust:1.85-bookworm AS builder
WORKDIR /app

# Copy dependency manifests first for layer caching
COPY Cargo.toml Cargo.lock ./

# Create stub sources so cargo can cache all crate deps
RUN mkdir -p src/bin && \
    echo 'fn main() {}' > src/main.rs && \
    echo 'fn main() {}' > src/bin/ghost-alpha-mcp.rs && \
    touch src/lib.rs && \
    cargo build --release 2>/dev/null; \
    rm -rf src

# Copy real source and rebuild (only workspace crate is dirty)
COPY src/       src/
COPY migrations/ migrations/
RUN touch src/main.rs src/lib.rs && \
    cargo build --release

# ── Stage 2: Runtime ─────────────────────────────────────────
FROM debian:bookworm-slim
RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ghost-alpha /usr/local/bin/ghost-alpha
ENV GHOST_ALPHA_DB_PATH=/data/ghost_alpha.db
VOLUME ["/data"]
ENTRYPOINT ["ghost-alpha"]
CMD ["screen"]
```

```bash
# Build and run
docker build -t ghost-alpha .
docker run -e GHOST_ALPHA_FINNHUB_API_KEY=xxx -v ghost-data:/data ghost-alpha

# Or use docker-compose
docker-compose up
```

---

## Plugin System

GhostAlpha has a pluggable architecture via trait-based registration.

### Available Extension Points

| Trait | Purpose | Built-in |
|-------|---------|----------|
| `DataProvider` | Market data fetching | FinnhubClient, YahooProvider, SampleProvider |
| `Screener` | Stock screening logic | MinerviniScreener, BreakoutScreener, MeanReversionScreener, TrendStrengthScreener |
| `SentimentScorer` | News sentiment analysis | NullSentimentScorer |
| `AlertChannel` | Alert delivery | TelegramChannel, DiscordChannel, WebhookChannel, NullChannel |

### Implementing a Custom Data Provider

```rust
use async_trait::async_trait;
use ghost_alpha::data::provider::DataProvider;
use ghost_alpha::data::models::{Candle, Stock, NewsArticle};
use ghost_alpha::error::DataError;

pub struct MyCustomProvider {
    api_key: String,
}

#[async_trait]
impl DataProvider for MyCustomProvider {
    async fn fetch_candles(
        &self, ticker: &str, from: NaiveDate, to: NaiveDate,
    ) -> Result<Vec<Candle>, DataError> {
        // Your implementation here
        todo!()
    }

    async fn fetch_stocks(&self, tickers: &[&str]) -> Result<Vec<Stock>, DataError> {
        todo!()
    }

    async fn fetch_news(&self, ticker: &str, limit: usize) -> Result<Vec<NewsArticle>, DataError> {
        todo!()
    }

    async fn fetch_spy_candles(
        &self, from: NaiveDate, to: NaiveDate,
    ) -> Result<Vec<Candle>, DataError> {
        todo!()
    }

    async fn fetch_universe(&self) -> Result<Vec<String>, DataError> {
        todo!()
    }
}
```

### Registering Custom Plugins

```rust
use ghost_alpha::plugins::Registry;

let mut registry = Registry::new();

// Register your custom provider
registry.register_provider("my-provider", MyCustomProvider::new("key"));

// Register alongside built-ins
registry.register_screener("minervini", MinerviniScreener::new());
registry.register_screener("my-screener", MyCustomScreener::new());

// Select which one to use
registry.select_provider("my-provider").unwrap();
registry.select_screener("my-screener").unwrap();
```

### Adding Custom Alert Channels

```rust
use async_trait::async_trait;
use ghost_alpha::alerts::channel::{AlertChannel, OutboundAlert};
use ghost_alpha::error::AlertError;

pub struct SlackChannel {
    webhook_url: String,
}

#[async_trait]
impl AlertChannel for SlackChannel {
    async fn send(&self, alert: &OutboundAlert) -> Result<(), AlertError> {
        // POST to Slack webhook
        todo!()
    }

    fn name(&self) -> &'static str {
        "slack"
    }
}

// Register it
registry.register_channel("slack", SlackChannel::new("https://hooks.slack.com/..."));
```

---

## Troubleshooting

### Common Issues

#### "Config file not found"

```bash
# Option 1: Create config file
cp config.example.toml config.toml
# Edit config.toml with your API key

# Option 2: Use environment variables
export GHOST_ALPHA_FINNHUB_API_KEY="your-key"
ghost-alpha-mcp --env-only
```

#### "Missing required field: finnhub.api_key"

This only applies when using the Finnhub provider (the default). You can either set the key or switch providers:
```bash
# Option 1: Set the key
export GHOST_ALPHA_FINNHUB_API_KEY="your-key"

# Option 2: Use Yahoo Finance instead (no key needed)
ghost-alpha screen --provider yahoo

# Option 3: Run the zero-config demo
ghost-alpha demo
```

#### MCP server not responding

```bash
# Test with a simple ping
echo '{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}' | ghost-alpha-mcp --env-only
# Expected: {"jsonrpc":"2.0","id":1,"result":{}}
```

#### Claude Desktop can't find the binary

Ensure `ghost-alpha-mcp` is in your PATH:
```bash
which ghost-alpha-mcp
# If not found:
cargo install --path /path/to/ghost-alpha
```

#### Debug logging

```bash
# MCP server logs go to stderr (stdout is for JSON-RPC)
ghost-alpha-mcp --env-only --verbose 2>debug.log

# Or set log level via env
GHOST_ALPHA_LOG_LEVEL=debug ghost-alpha-mcp --env-only 2>debug.log
```

### Environment Variable Reference

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `GHOST_ALPHA_FINNHUB_API_KEY` | No* | — | Finnhub API key. Required only when using the `finnhub` provider. Not needed with `--provider yahoo` or `ghost-alpha demo`. |
| `GHOST_ALPHA_TELEGRAM_BOT_TOKEN` | No | — | Telegram bot token |
| `GHOST_ALPHA_TELEGRAM_CHAT_ID` | No | — | Telegram chat/group ID |
| `GHOST_ALPHA_DISCORD_WEBHOOK_URL` | No | — | Discord webhook URL for alert delivery |
| `GHOST_ALPHA_WEBHOOK_URL` | No | — | Generic webhook URL for alert delivery |
| `GHOST_ALPHA_DB_PATH` | No | `ghost_alpha.db` | SQLite database path |
| `GHOST_ALPHA_LOG_LEVEL` | No | `info` | Log level (trace/debug/info/warn/error) |
