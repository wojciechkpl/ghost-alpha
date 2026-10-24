//! Broker integration for optional trade execution.
//!
//! Phase 7: Ghost Alpha is advisory by default — it recommends trades but
//! doesn't execute them. This module adds optional broker integration,
//! starting with Alpaca paper trading.
//!
//! Safety hierarchy:
//! 1. `--dry-run` — preview what would be executed (default)
//! 2. `--paper`  — execute on Alpaca paper trading (no real money)
//! 3. `--live`   — execute on live account (requires explicit confirmation)

pub mod alpaca;
