# Plan 03: Interactive Setup Wizard

**Effort:** Medium (3-4 days)
**Impact:** High
**Priority:** 3

## Problem

Current setup requires manually editing `config.toml`, knowing environment variable names, and running separate commands for migration and model download. The `ghost-alpha setup` command only downloads ONNX models — it doesn't handle the full onboarding flow.

## Solution

Expand `ghost-alpha setup` into a guided, interactive wizard that configures everything in one pass.

## User flow

```
$ ghost-alpha setup

  Ghost Alpha Setup Wizard
  ════════════════════════

  Step 1/5: Data Provider
  ───────────────────────
  Enter your Finnhub API key (free at https://finnhub.io):
  > ████████████████

  Testing connection... ✓ Connected (S&P 500: 503 tickers)

  Step 2/5: Alert Channels (optional, press Enter to skip)
  ─────────────────────────────────────────────────────────
  Telegram bot token (from @BotFather):
  >
  (skipped)

  Discord webhook URL:
  > https://discord.com/api/webhooks/1234/abcd

  Testing webhook... ✓ Test message sent to Discord

  Step 3/5: Database
  ──────────────────
  Database path [ghost_alpha.db]:
  >
  (using default)

  Running migrations... ✓ 7 migrations applied

  Step 4/5: Sentiment Model (optional)
  ─────────────────────────────────────
  Download FinBERT ONNX model (~400MB)? [y/N]:
  > n
  (skipped — use --skip-sentiment when screening)

  Step 5/5: Test Screen
  ─────────────────────
  Running a quick test with 5 tickers...

  Ticker  Score  Minervini  RS%ile
  AAPL    71.2    6/8       72
  MSFT    68.5    5/8       68
  ...
  ✓ Everything works!

  Config saved to: ./config.toml
  Secrets saved to: ./.env

  Next steps:
    ghost-alpha screen          # Full S&P 500 screening
    ghost-alpha screen --top 10 # Top 10 only
    ghost-alpha regime          # Current market regime
```

## Implementation

### 1. Add `dialoguer` dependency

```toml
[dependencies]
dialoguer = "0.11"   # Interactive prompts (Input, Confirm, Select)
```

`dialoguer` provides styled input prompts, password masking, confirmation, and multi-select. It's the de facto Rust crate for CLI wizards.

### 2. Restructure `setup` subcommand

Replace the current `setup` handler in `src/main.rs`:

```rust
Commands::Setup {
    /// Skip interactive prompts, only download models
    #[arg(long)]
    models_only: bool,
} => {
    if models_only {
        download_models().await?;
    } else {
        run_setup_wizard().await?;
    }
}
```

### 3. Wizard module

Create `src/wizard.rs`:

```rust
pub async fn run_setup_wizard() -> Result<()> {
    println!("\n  Ghost Alpha Setup Wizard");
    println!("  ════════════════════════\n");

    let api_key = step_data_provider().await?;
    let telegram = step_telegram().await?;
    let discord = step_discord().await?;
    let db_path = step_database().await?;
    let sentiment = step_sentiment().await?;

    write_config_toml(&api_key, &telegram, &discord, &db_path, &sentiment)?;
    write_dot_env(&api_key, &telegram, &discord)?;

    step_test_screen(&api_key).await?;

    Ok(())
}
```

### 4. Validation at each step

Each step validates immediately:
- **Finnhub key:** Make a test API call (`/stock/symbol?exchange=US`)
- **Telegram:** Send a test message ("Ghost Alpha connected!")
- **Discord:** Send a test embed to the webhook
- **Database:** Run migrations, verify write
- **Sentiment:** Check model files exist and ONNX session loads

### 5. Output files

The wizard writes two files:

**`config.toml`** (non-sensitive settings):
```toml
[finnhub]
api_key = "FROM_ENV"  # actual key in .env

[alerts]
critical_score_threshold = 90.0
warning_score_threshold = 70.0

[storage]
db_path = "ghost_alpha.db"
```

**`.env`** (secrets, gitignored):
```
GHOST_ALPHA_FINNHUB_API_KEY=actual_key_here
GHOST_ALPHA_DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/...
```

### 6. Idempotent re-runs

If `config.toml` already exists, the wizard:
- Loads current values as defaults
- Shows "(current: ****1234)" for secrets
- Only overwrites fields the user changes
- Preserves any custom settings not covered by the wizard

## Files to create/modify

| File | Action |
|------|--------|
| `src/wizard.rs` | Create — wizard logic |
| `src/main.rs` | Modify — update `Setup` subcommand, add `mod wizard` |
| `src/lib.rs` | Modify — add `pub mod wizard` |
| `Cargo.toml` | Modify — add `dialoguer = "0.11"` |
| `.gitignore` | Modify — ensure `.env` is listed |

## Dependencies

- `dialoguer = "0.11"` — interactive CLI prompts

## Testing

- Unit test: Config file generation produces valid TOML
- Unit test: `.env` file generation has correct format
- Integration test: Wizard with mocked Finnhub validates API key
- Manual test: Full wizard flow on clean machine

## Acceptance criteria

- [ ] `ghost-alpha setup` walks through all 5 steps interactively
- [ ] Each step validates immediately (API test, webhook test, etc.)
- [ ] Secrets go to `.env`, config goes to `config.toml`
- [ ] Existing config is preserved on re-run
- [ ] `ghost-alpha setup --models-only` retains current behavior
- [ ] Skipping optional steps (Telegram, Discord, sentiment) works cleanly
