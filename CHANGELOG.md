# Changelog

All notable changes to GhostAlpha are documented in this file.

## [Unreleased] — v2.0

### Concentration hub: ETF overlap
- **New:** `/portfolio/concentration` (Portfolio menu): donut and positions table valued at the latest close (cost basis flagged "at cost" when a ticker has no candles), top-1 limit status in text, effective number of positions, and a lazily loaded ETF overlap panel (`/partials/etf-overlap`)
- **New:** the panel looks through the funds you hold using SEC N-PORT filings and ranks candidate ETFs (`candidates`, default `VTI,VXUS,QQQ,SCHD`) by the concentrated stock's look-through weight after buying `buy` dollars of each. One ETF failing shows a status row, and a failed, timed-out or skipped held fund (the 15 largest are evaluated; each ETF is bounded to 20 s) adds a "partial" badge. Ticker maps load once per request, and EDGAR cache reads and parsing no longer block the async runtime. Positions without a close and without a positive cost are shown as "no price" and excluded; closes more than 7 days older than the newest are marked stale; `buy` is capped at 1e12
- **Changed:** `AppState` carries a shared EDGAR client built by `ghost-alpha serve` with the same User-Agent, cache and base-URL resolution as `ghost-alpha edgar`; without a User-Agent the dashboard still starts and the panel shows the setup hint. `cli::edgar::build_client` is now public

### Guided setup wizard
- **New:** `ghost-alpha setup` reads the existing `config.toml` and `.env`, pre-fills every prompt from them, and edits both in place: comments, ordering and unknown keys survive, and unrelated `.env` lines are kept. Secrets are asked with masked input and only ever go to `.env` (mode 0600)
- **New:** nothing is written before a review that shows each file as new, changed (with a diff in which every `.env` value is masked) or unchanged; changed files are backed up to `*.bak` and written atomically. `--yes` skips only that final confirmation
- **New:** `setup --section <data|edgar|profile|alerts|storage|portfolio>` re-runs one step. Steps now cover the EDGAR User-Agent, a `[profile]` tax profile (state, filing, other income), the FRED key (`GHOST_ALPHA_FRED_API_KEY`, now read by the config loader), and an optional portfolio import that validates your positions and vesting CSVs with the same loaders as `plan`, copies them into a data folder and writes `plan.toml`
- **Changed:** `sell-plan` and `sell-compare` no longer require `--state`: `--state`, `--filing` and `--other-income` fall back to `[profile]` in the config file (a stderr note names what came from it), and `--state` is an explicit error naming `setup --section profile` when neither is set
- **Changed:** `.gitignore` ignores `data/` and `*.bak`; the wizard warns when the chosen data folder is inside a git work tree and not ignored

### Web dashboard: shared shell, accessibility and hardening
- **Changed:** every page renders through one shell (`web::shell`) with a single navigation grouped into Portfolio, Markets, Research, Tools and System; `/concentration-decay` is now reachable from the nav; the menu collapses on small screens
- **New:** `tokens.css` light/dark colour tokens (also driving Pico), with WCAG contrast checked in tests; status badges pair colour with an icon
- **Changed:** accessibility: skip link, landmarks, one `<h1>` per page without skipped heading levels, informative table captions with `scope` on headers, 24px minimum targets, screen-reader announcements only for user-triggered updates
- **Changed:** security and caching: Content-Security-Policy, `nosniff` and `Referrer-Policy` on every response, gzip compression, permissive CORS removed, content-hashed static URLs (`?v=`) served immutable, weak ETags with 304s
- **Fixed:** the home screen's 60-second refresh no longer resets a chosen past date

### Web dashboard: freshness, honesty and chart accessibility
- **New:** a freshness chip in every page header shows the newest screen run, a state word (Fresh, Aging, Stale, No data) and its age in trading days (weekdays; exchange holidays are not modelled)
- **New:** pages driven by built-in sample inputs (`/rotation`, `/sizing`, `/optimize`, and `/concentration-decay` without `shares` and `price`) carry an "Illustrative inputs" banner linking to the import instructions
- **New:** sortable column headers and a filter box on the screening, watchlist and alert tables (`table.js`, keyboard accessible, kept across 60-second refreshes); ticker cells link to `/valuation?ticker=SYM`
- **New:** every chart has a title and description for screen readers, a "Table view" with the plotted numbers, axis titles with units, and labels of at least 11px; decorative in-cell graphics are hidden from assistive technology
- **Changed:** chart and inline colours come from the theme tokens (`var(--ga-*)`), so charts follow light and dark mode; the concentration-decay chart starts at "Start" instead of "Q0"
- **Changed:** `nav.js` loads synchronously to avoid a flash of the open menu on small screens; `status.js` announces user-triggered updates ("Updated 14:03:21") in the page's status region

### SEC EDGAR client
- **New:** `ghost-alpha edgar facts <TICKER>`: point-in-time XBRL company facts. Annual (335-395 day) and quarterly (61-121 day) series; restatements and amendments appear only from their filing date (`--as-of`, inclusive); START and FILING FY/FP columns; stderr hints for unknown concepts or units, stale concepts (with active alternatives) and series emptied by form filtering (e.g. 20-F filers)
- **New:** `ghost-alpha edgar filings <TICKER>`: filings filtered by `--form`, 8-K `--item` and `--since` (follows older paginated submission pages, skipping a failing page with a warning); `--format table|json`
- **New:** `ghost_alpha::data::edgar`: ticker→CIK (normalises `BRK.B` → `BRK-B`), `companyfacts`/`submissions` parsers, `Period`-based `value_as_of`, `filings_report` (filings plus warnings; validated page names, one submissions fetch per call), one deterministic winner order (latest filing, same-day amendment over original, higher accession, later start)
- **New:** SEC fair-access client: 8 req/s limiter, retry with `Retry-After` and jittered backoff, actionable 403 message (never retried), 24h on-disk cache namespaced per host with stale-if-error fallback
- **New:** SEC requires a User-Agent with a contact email: set `GHOST_ALPHA_EDGAR_USER_AGENT` or `[edgar] user_agent` in config.toml. No API key, Finnhub key or other config needed; an unreadable config.toml only produces a warning

### Tax CLI defaults
- **Changed:** `sell-plan`, `sell-compare` and `div-report` no longer assume a state: `--state` is now required. `--filing` defaults to `single` (was `mfj` for `sell-compare` and `div-report`)

### Bring-your-own-data for `ghost-alpha plan`
- **New:** `ghost-alpha init-data <DIR>` writes editable templates (positions, vesting, symbol map, plan.toml)
- **New:** `plan --data-dir` / `GHOST_ALPHA_DATA_DIR` discovers `positions/*.csv`, `vesting.csv`, `symbol_map.toml`, `plan.toml`
- **New:** positions auto-detect — Fidelity export or simple `symbol,quantity,price[,cost_basis][,account]` CSV
- **New:** minimal vesting CSV (`symbol,vesting_date,quantity`)
- **New:** user `symbol_map.toml` merged over a generic built-in map, with optional `default_class` fallback
- **New:** any concentrated ticker (`--symbol`, inferred if omitted); unwind parameters are inputs (`--ltcg-rate`, `--floor-pct`, `--cost-basis`, `--unwind`)
- **Changed:** `--withhold-rate` default 0.37 → 0.22; output columns `amzn_*` → `concentrated_*`
- **Changed:** test fixtures are fully synthetic

### Phase 7: Portfolio Plan Pipeline
- **New:** `ghost-alpha plan` command — end-to-end forward-NAV projection pipeline
- **New:** 11 gaps shipped (~30 SP, 282 new tests across lib + 4 integration suites)
- **New:** `src/projection/monte_carlo.rs` — Lognormal MC with 5000 paths, block-correlation Σ (PSD-enforced)
- **New:** `src/projection/asset_class_library.rs` — TOML-loaded per-asset μ/σ library (19 asset classes)
- **New:** `src/projection/scenario.rs` — ScenarioPath + walk_price (Hold/SellReinvest/OptionBUnwind)
- **New:** `src/projection/portfolio_nav.rs` — project_path for multi-year MC simulation
- **New:** `src/projection/output.rs` — CSV + audit JSON emitters
- **New:** `src/portfolio/symbol_map.rs` — Ticker → AssetClass lookup (23 tickers)
- **New:** `src/portfolio/vesting.rs` — VestingEvent + DistributionSchedule (unified vesting ETL)
- **New:** `src/portfolio/concentration.rs` — compute_k_max single-name floor solver (concentration-aware unwind)
- **New:** `src/portfolio/sequencer.rs` — route_cash (Greedy + ProRata allocation)
- **New:** `src/portfolio/import.rs` extensions — Fidelity Portfolio_Positions ETL (BOM + disclaimer + FCASH** sleeve handling)
- **New:** `src/plans/rule_10b5_1.rs` — SEC Rule 10b5-1(c)(1) plan generator with cooling-off + execution dates + share sizing
- **New:** `src/output/plan_10b5_1_md.rs` — Markdown rendering for Plan10b5_1
- **New:** `src/cli/plan.rs` — run_plan_pipeline orchestrator (composes projection stack)
- **New:** `src/output/mermaid.rs` — quote_label_if_needed for strict Mermaid renderers
- **New:** `config/asset_classes.toml` — canonical per-class μ/σ assumptions (19 classes, hand-curated)
- **New:** `tests/forward_nav_matches_python.rs` — cross-language validation (within 1% year-5 NAV)
- **New:** `tests/mc_returns_match_python.rs` — lognormal MC path validation
- **New:** `tests/monte_carlo_smoke.rs` — MC engine smoke tests
- **New:** `tests/output_csv_round_trip.rs` — CSV round-trip validation
- **New:** `benches/projection_mc.rs` — Criterion bench (11.2 ms / 5000 paths)
- **New:** `tests/fixtures/fidelity/`, `tests/fixtures/python_mc/`, `tests/fixtures/vesting/` — test data fixtures
- Cross-validated against an independent Python reference implementation: within 1% on year-5 NAV for all three treatments
- Output artifacts: `nav_projection.csv` (percentile bands), `nav_mc_paths.csv` (raw paths), `audit.json` (reproducibility receipt)

### Phase 8: Broker Integration
- **New:** `ghost-alpha execute` command with Alpaca API (dry-run, paper, live modes)
- **New:** `src/broker/alpaca.rs` — AlpacaClient with get_account, get_positions, submit_order
- Dry-run mode works without credentials for safe previewing

### Phase 6: Adaptive Engine
- **New:** `src/analysis/microstructure.rs` — Vol/Corr/Liquidity regime classification
- **New:** `src/analysis/multi_horizon.rs` — Swing/Position/Trend scoring with softmax weights
- **New:** `src/analysis/adaptive.rs` — Strategy weight selection (affinity x regime x vol)
- **New:** `src/analysis/param_tuner.rs` — Coordinate descent calibration with guardrails
- **New:** `src/analysis/bayesian_weights.rs` — Beta distribution strategy beliefs with decay
- **New:** `src/analysis/attribution.rs` — Brinson-Fachler performance decomposition
- **New:** `ghost-alpha attribution` command — performance breakdown by selection/timing/sizing/strategy
- **New:** `ghost-alpha calibrate` command — adaptive parameter tuning with --dry-run and --reset

### Phase 5.5: Capital Rotation
- **New:** `src/portfolio/rotation.rs` — SPS (sell pressure), CDS (deployment), US (upgrade) scores
- **New:** `ghost-alpha rotate` command — 8-position sell pressure analysis with CDS
- Anti-churn safeguards: 10-session hold, 2 swaps/week max, 20-session cooldown
- Tier-adjusted thresholds: A+ wider room (SPS 55), B shorter leash (SPS 40)

### Phase 5: Quantitative Rigor
- **Fixed:** Volume score bias — z=0 now scores 0, not 15 (was inflating all composites by ~15%)
- **Fixed:** Screener conditions normalization — all 4 screeners now score to 100% of their criteria
- **Fixed:** Sentiment weight redistribution — dead weight (article_count=0) redirected to tech/volume
- **Fixed:** Sharpe ratio methodology — uses horizon-adjusted annualization for overlapping returns
- **Fixed:** Selling programme pace cap — combined multiplier clamped to [0.25, 4.0]
- **New:** `src/portfolio/sizing.rs` — 4 methods (VolatilityTarget, EqualWeight, HalfKelly, FixedFractional)
- **New:** `src/tax/wash_sale.rs` — 30-day wash sale detection (buy + sell side)
- **New:** `src/analysis/ensemble.rs` — Multi-strategy consensus scoring
- **New:** `ghost-alpha size` command — position sizing recommendations
- Added liquidity-adjusted slippage model (1x-5x by LiquidityClass)
- Added data quality validation (>50% daily move detection, split heuristic)
- Added regime transition hysteresis (3-day confirmation before regime change)
- Transaction costs now deducted from backtest forward returns

### Phase 8: Institutional Techniques (Wave 1)
- **New:** `src/portfolio/vol_target.rs` — Portfolio-level volatility targeting
- **New:** `ghost-alpha vol-target` command — exposure scaling recommendations

### Improvements
- **Fixed:** Stored XSS vulnerability in watchlist notes (now html_escape'd)
- **Fixed:** AlpacaClient headers panic on invalid API key characters (returns Result)
- **Fixed:** MCP server serde_json unwrap in write_response (returns io::Error)
- **Fixed:** param_tuner get/set no longer panic on out-of-bounds index
- **Fixed:** All clippy warnings resolved across lib + test targets
- **New:** `--fixed-weights` global CLI flag to disable adaptive engine
- **New:** Rotation and sizing dashboard tabs in web UI
- **New:** `[adaptive]` and `[broker]` sections in config.example.toml
- **Updated:** ARCHITECTURE.md — complete rewrite with Mermaid diagrams for 102 source files
- **New:** docs/TUTORIAL.md — 13-section tutorial covering all commands
- **New:** CHANGELOG.md — this file
- Added .venv to .gitignore
- Wired size/rotate CLI to read from SQLite when available (demo fallback)

## [1.2] — 2026-03

### Phase 4: Validation & Polish
- Walk-forward backtesting with Monte Carlo confidence intervals
- HTML backtest reports
- MCP server for AI agent integration (JSON-RPC 2.0 over stdio)
- Plugin registry (BYOA providers, screeners, channels)
- Discord webhook alerts
- Performance optimization (LTO, stripped binary ~6MB)

### Phase 3: Sentiment
- ONNX Runtime + FinBERT integration (feature-gated)
- News fetcher with time-decay aggregation
- Sentiment -> composite pipeline

### Phase 2.5: Signal Quality
- Weighted multi-timeframe RS (40/20/20/20)
- Regime-adaptive scoring weights (55/30/15 Bull, 60/20/20 Sideways, 70/10/20 Bear)
- ATR, risk-reward ratio, liquidity classification
- VCP detection, base pattern analysis
- Macro risk indicators (VIX, credit spreads, yield curve via FRED)
- Correlation alerting, conviction decay monitoring

### Phase 2: Alerts & Persistence
- SQLite storage (7 migrations)
- Telegram bot integration
- Alert engine with dedup (7-day cooldown)
- Exit signals (Stage 2 breakdown, RS deterioration, MA death cross)
- Flagged stock watchlist

### Phase 1: Core Engine
- Minervini 8-point trend template
- Volume Z-score anomaly detection
- Relative strength percentile ranking
- ADX trend strength
- Composite scoring (0-100)
- CLI interface (screen, regime, alerts)
- TOML configuration
- Finnhub + Yahoo data providers
- Sample data provider (50 bundled tickers)
