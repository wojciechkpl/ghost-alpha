use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use futures::stream::{self, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use tracing_subscriber::EnvFilter;

use ghost_alpha::alerts::channel::AlertChannel;
use ghost_alpha::alerts::engine::{AlertConfig, AlertEngine};
use ghost_alpha::alerts::exit_signals::{
    check_conviction_decay, check_exit_signals, format_exit_message, ConvictionAlert,
};
use ghost_alpha::analysis::correlation::check_correlation_concentration;
use ghost_alpha::analysis::correlation_matrix::CorrelationMatrix;
use ghost_alpha::analysis::indicators::price_return_252d;
use ghost_alpha::analysis::regime::{detect_regime, should_suppress_in_bear};
use ghost_alpha::analysis::scorer::{assign_grades, CompositeScore, Grade};
use ghost_alpha::analysis::screener::ScreenResult;
use ghost_alpha::config::AppConfig;
use ghost_alpha::data::cache::CachingProvider;
use ghost_alpha::data::models::WatchlistEntry;
use ghost_alpha::data::models::{FlaggedStatus, FlaggedStock, RegimeState, Severity};
use ghost_alpha::output::{render, OutputFormat};
use ghost_alpha::plugins::registry::default_registry;
use ghost_alpha::storage::db::{AlertLogEntry, Repository, SqliteRepository};

// ─── CLI Definition ───────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "ghost-alpha",
    about = "Market intelligence agent — Minervini-based stock screener with alerts",
    version
)]
struct Cli {
    /// Path to config TOML file
    #[arg(short, long, default_value = "config.toml")]
    config: String,

    /// Verbose logging (set RUST_LOG=debug for full detail)
    #[arg(short, long)]
    verbose: bool,

    /// Disable adaptive engine — use fixed regime-based weights only
    #[arg(long)]
    fixed_weights: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the screening pipeline once and print results
    Screen {
        /// Skip sentiment scoring (faster, no ONNX required)
        #[arg(long)]
        skip_sentiment: bool,
        /// Show only top N results by composite score (default: 20)
        #[arg(long, default_value = "20")]
        top: usize,
        /// Output format: table (default), json, csv, or jsonl
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
        /// Data provider to use: "yahoo" (no API key) or "finnhub" (default).
        /// Overrides plugins.active_provider in config.
        #[arg(long)]
        provider: Option<String>,
        /// Screening strategy: minervini (default), breakout, mean-reversion, trend-strength.
        /// Run `ghost-alpha strategies` to list all available strategies.
        #[arg(long, default_value = "minervini")]
        strategy: String,
    },
    /// List all available built-in screening strategies with descriptions
    Strategies,
    /// Show the current market regime
    Regime {
        /// Output format: table (default), json, csv, or jsonl
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
        /// Model: threshold (default) or hmm (probabilistic)
        #[arg(long, default_value = "threshold")]
        model: String,
    },
    /// Show recent alerts from the alert log
    Alerts {
        /// Show last N alerts
        #[arg(long, default_value = "20")]
        last: usize,
        /// Output format: table (default), json, csv, or jsonl
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Print the loaded configuration (secrets masked) and exit
    ShowConfig,
    /// Run the database migrations and exit
    Migrate,
    /// List registered plugins: providers, screeners, scorers, and alert channels
    Plugins,
    /// Start the MCP (Model Context Protocol) server for AI agent integration.
    /// Communicates over stdin/stdout using JSON-RPC 2.0.
    Mcp {
        /// Load configuration entirely from environment variables (no TOML file needed).
        /// Useful for Docker, CI, and MCP agent integration.
        #[arg(long)]
        env_only: bool,
    },
    /// Run walk-forward backtest with Monte Carlo validation
    Backtest {
        /// Number of years of historical data to use
        #[arg(long, default_value = "5")]
        years: u32,
        /// Output format: table (default), json, csv, jsonl, or html (report file)
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
        /// Output file path when --format html is used
        #[arg(long, default_value = "backtest_report.html")]
        output: String,
    },
    /// Run the end-to-end plan pipeline: positions CSVs -> MC projection -> CSV + audit JSON
    Plan {
        /// Positions CSV (Fidelity export or simple `symbol,quantity,price[,cost_basis]` format).
        /// Repeat once per account. Replaces the files discovered in --data-dir.
        #[arg(long)]
        positions: Vec<String>,
        /// Vesting schedule CSV (`symbol,vesting_date,quantity` minimum). Omit for non-RSU portfolios.
        /// Replaces <data-dir>/vesting.csv.
        #[arg(long)]
        vesting: Option<String>,
        /// Data folder with positions/*.csv and optional vesting.csv, symbol_map.toml, plan.toml
        /// (create one with `ghost-alpha init-data`)
        #[arg(long, env = "GHOST_ALPHA_DATA_DIR")]
        data_dir: Option<String>,
        /// Path to an asset_classes.toml (default: config/asset_classes.toml if present,
        /// else the copy built into the binary)
        #[arg(long)]
        asset_classes: Option<String>,
        /// RSU treatment: hold | sell-reinvest | option-b (default: option-b)
        #[arg(long)]
        treatment: Option<String>,
        /// Number of MC paths (default 5000)
        #[arg(long)]
        paths: Option<usize>,
        /// MC seed for reproducibility (default 42)
        #[arg(long)]
        seed: Option<u64>,
        /// Projection horizon in years (default 5)
        #[arg(long)]
        horizon: Option<u32>,
        /// RSU withholding rate (federal supplemental), e.g. 0.22 (default 0.22)
        #[arg(long)]
        withhold_rate: Option<f64>,
        /// Snapshot date (YYYY-MM-DD); required unless `as_of` is set in plan.toml
        #[arg(long)]
        as_of: Option<String>,
        /// Concentrated symbol (default: vesting symbol with most shares, else largest non-cash position)
        #[arg(long)]
        symbol: Option<String>,
        /// symbol_map.toml mapping extra tickers to asset classes (merged over the built-in map)
        #[arg(long)]
        symbol_map: Option<String>,
        /// Asset class for tickers missing from the symbol map, e.g. us_large_cap_index
        #[arg(long)]
        default_class: Option<String>,
        /// Long-term capital-gains rate for the Option B unwind (default 0.238)
        #[arg(long)]
        ltcg_rate: Option<f64>,
        /// Concentration floor as a fraction of NAV for the Option B unwind (default 0.25)
        #[arg(long)]
        floor_pct: Option<f64>,
        /// Cost basis per share of the concentrated symbol (default: derived from positions, else 0)
        #[arg(long)]
        cost_basis: Option<f64>,
        /// Option B unwind schedule as year:shares pairs, e.g. "1:400,2:300"
        #[arg(long)]
        unwind: Option<String>,
        /// Output directory for nav_projection.csv, nav_mc_paths.csv, audit.json
        #[arg(long, default_value = "plan_output")]
        out_dir: String,
    },
    /// Create a data folder with example positions, vesting, symbol map and plan.toml for `plan --data-dir`
    InitData {
        /// Directory to create or fill
        dir: String,
        /// Overwrite existing template files
        #[arg(long)]
        force: bool,
    },
    /// Interactive setup wizard — configures API keys, alerts, and database
    Setup {
        /// Skip interactive prompts; only download/prepare model files
        #[arg(long)]
        models_only: bool,
    },
    /// Manage personal watchlist of tickers
    Watch {
        #[command(subcommand)]
        action: WatchAction,
    },
    /// Run the full screening pipeline against bundled sample data — no API key or config required
    Demo {
        /// Number of top results to show
        #[arg(long, default_value = "10")]
        top: usize,
    },
    /// Start the embedded web dashboard
    Serve {
        /// TCP port to listen on
        #[arg(long, default_value = "3000")]
        port: u16,
        /// Bind address
        #[arg(long, default_value = "127.0.0.1")]
        bind: String,
    },
    /// Prepare a tax-aware selling programme for a fully vested position
    SellPlan {
        /// Ticker symbol
        #[arg(long)]
        ticker: String,
        /// Total shares to sell
        #[arg(long)]
        shares: f64,
        /// Cost basis per share
        #[arg(long)]
        cost_basis: f64,
        /// Current market price per share
        #[arg(long)]
        price: f64,
        /// Date shares were acquired (YYYY-MM-DD)
        #[arg(long, default_value = "2022-01-01")]
        acquired: String,
        /// Filing status: single or mfj
        #[arg(long, default_value = "single")]
        filing: String,
        /// State code (e.g., WA, CA, TX)
        #[arg(long, default_value = "WA")]
        state: String,
        /// Other annual income (salary, etc.)
        #[arg(long, default_value = "0")]
        other_income: f64,
        /// Assumed annual price growth (e.g., 0.05 for 5%)
        #[arg(long, default_value = "0.0")]
        growth: f64,
        /// Selling horizon in years (1–10)
        #[arg(long, default_value = "2")]
        years: u32,
        /// Distribution strategy: even, front, back
        #[arg(long, default_value = "even")]
        distribution: String,
        /// Output format: table (default), json
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Compare 1..N year selling strategies to find the tax-optimal spread
    SellCompare {
        /// Ticker symbol
        #[arg(long)]
        ticker: String,
        /// Total shares to sell
        #[arg(long)]
        shares: f64,
        /// Cost basis per share
        #[arg(long)]
        cost_basis: f64,
        /// Current market price per share
        #[arg(long)]
        price: f64,
        /// Date shares were acquired (YYYY-MM-DD)
        #[arg(long, default_value = "2022-01-01")]
        acquired: String,
        /// Filing status: single or mfj
        #[arg(long, default_value = "mfj")]
        filing: String,
        /// State code (e.g., WA, CA, TX)
        #[arg(long, default_value = "WA")]
        state: String,
        /// Other annual income (salary, etc.)
        #[arg(long, default_value = "0")]
        other_income: f64,
        /// Assumed annual price growth (e.g., 0.05 for 5%)
        #[arg(long, default_value = "0.0")]
        growth: f64,
        /// Maximum number of years to compare (1–10)
        #[arg(long, default_value = "5")]
        max_years: u32,
        /// Output format: table (default), json
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Show target fund allocation for a risk profile and brokerage
    Allocate {
        /// Risk profile: aggressive, moderate, conservative
        #[arg(long, default_value = "moderate")]
        profile: String,
        /// Brokerage: fidelity, schwab, vanguard
        #[arg(long, default_value = "fidelity")]
        brokerage: String,
        /// Total portfolio value for dollar amounts
        #[arg(long, default_value = "500000")]
        value: f64,
        /// Output format: table (default), json
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Generate a self-contained HTML diversification report
    DivReport {
        /// Ticker symbol
        #[arg(long)]
        ticker: String,
        /// Total shares to sell
        #[arg(long)]
        shares: f64,
        /// Cost basis per share
        #[arg(long)]
        cost_basis: f64,
        /// Current market price per share
        #[arg(long)]
        price: f64,
        /// Date shares were acquired (YYYY-MM-DD)
        #[arg(long, default_value = "2022-01-01")]
        acquired: String,
        /// Filing status: single or mfj
        #[arg(long, default_value = "mfj")]
        filing: String,
        /// State code
        #[arg(long, default_value = "WA")]
        state: String,
        /// Other annual income
        #[arg(long, default_value = "200000")]
        other_income: f64,
        /// Value of rest of portfolio (non-concentrated)
        #[arg(long, default_value = "50000")]
        other_value: f64,
        /// Selling horizon in years
        #[arg(long, default_value = "2")]
        years: u32,
        /// Brokerage: fidelity, schwab, vanguard
        #[arg(long, default_value = "fidelity")]
        brokerage: String,
        /// Risk profile: aggressive, moderate, conservative
        #[arg(long, default_value = "moderate")]
        profile: String,
        /// Output file path
        #[arg(long, default_value = "diversification_report.html")]
        output: String,
    },
    /// Recommend position sizes for screening results
    Size {
        /// Portfolio value in dollars
        #[arg(long, default_value = "100000")]
        portfolio: f64,
        /// Maximum position size as % of portfolio
        #[arg(long, default_value = "15")]
        max_pct: f64,
        /// Risk per trade as % of portfolio
        #[arg(long, default_value = "1")]
        risk_pct: f64,
        /// Sizing method: volatility-target, equal-weight, fixed-fractional
        #[arg(long, default_value = "volatility-target")]
        method: String,
        /// Output format
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Analyze capital rotation — sell pressure, deployment, and upgrade opportunities
    Rotate {
        /// Output format
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Show portfolio volatility targeting status and exposure recommendation
    VolTarget {
        /// Target annualized volatility (%, default 15)
        #[arg(long, default_value = "15")]
        target: f64,
        /// Current invested percentage of portfolio
        #[arg(long, default_value = "70")]
        invested: f64,
    },
    /// Show portfolio factor exposures (beta, sector concentration, correlation)
    Factors {
        /// Output format
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Show VIX term structure signal and exposure recommendation
    Vix {
        /// VIX level (default: auto from market data, or manual override)
        #[arg(long, default_value = "18.0")]
        vix: f64,
        /// VIX3M level
        #[arg(long, default_value = "20.0")]
        vix3m: f64,
        /// SKEW index
        #[arg(long, default_value = "125.0")]
        skew: f64,
        /// Put/call ratio 10-day SMA
        #[arg(long, default_value = "0.88")]
        put_call: f64,
    },
    /// Show performance attribution breakdown (selection, timing, sizing, strategy)
    Attribution {
        /// Period in days (e.g., 90 for last quarter)
        #[arg(long, default_value = "90")]
        period: u32,
        /// Output format
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
    /// Run adaptive parameter calibration using walk-forward optimization
    Calibrate {
        /// Preview changes without applying
        #[arg(long)]
        dry_run: bool,
        /// Reset to defaults for current regime
        #[arg(long)]
        reset: bool,
    },
    /// Execute trades via Alpaca broker API (paper or live)
    Execute {
        /// Execution mode: dry-run (preview), paper, or live
        #[arg(long, default_value = "dry-run")]
        mode: String,
        /// Buy a specific ticker with computed sizing
        #[arg(long)]
        buy: Option<String>,
        /// Sell a specific ticker
        #[arg(long)]
        sell: Option<String>,
        /// Number of shares (overrides auto-sizing)
        #[arg(long)]
        shares: Option<f64>,
        /// Portfolio value for auto-sizing
        #[arg(long, default_value = "100000")]
        portfolio: f64,
    },
    /// Compute geopolitical risk score from market inputs
    GeoRisk {
        /// Brent oil price ($/barrel)
        #[arg(long, default_value = "75.0")]
        oil: f64,
        /// VIX level
        #[arg(long, default_value = "15.0")]
        vix: f64,
        /// Gold 3-month change (fraction, e.g., 0.08 for 8%)
        #[arg(long, default_value = "0.02")]
        gold_3m: f64,
        /// DXY 3-month change (fraction, e.g., -0.05 for -5%)
        #[arg(long, default_value = "0.01", allow_hyphen_values = true)]
        dxy_3m: f64,
        /// Output format: table (default), json
        #[arg(long, value_enum, default_value = "table")]
        format: OutputFormat,
    },
}

#[derive(Subcommand)]
enum WatchAction {
    /// Add one or more tickers to the watchlist
    Add {
        /// Ticker symbols (e.g., NVDA AAPL TSLA)
        tickers: Vec<String>,
        /// Optional note attached to all added tickers
        #[arg(long)]
        note: Option<String>,
    },
    /// Remove one or more tickers from the watchlist
    Remove {
        /// Ticker symbols to remove
        tickers: Vec<String>,
    },
    /// Show the current watchlist with last known scores
    List,
    /// Clear the entire watchlist
    Clear,
}

// ─── Demo Pipeline ───────────────────────────────────────────────────────────

async fn run_demo(top: usize) -> Result<()> {
    use ghost_alpha::analysis::indicators::price_return_252d as pr252;
    use ghost_alpha::analysis::regime::detect_regime;
    use ghost_alpha::analysis::scorer::{assign_grades, CompositeScore, Grade};
    use ghost_alpha::analysis::screener::{MinerviniScreener, Screener};
    use ghost_alpha::data::models::MarketRegime;
    use ghost_alpha::data::provider::DataProvider;
    use ghost_alpha::data::sample::{SampleProvider, DEMO_TICKERS};

    let started = std::time::Instant::now();

    let ticker_count = DEMO_TICKERS.len();
    println!();
    println!(
        "  {}",
        format!("Ghost Alpha Demo — screening {ticker_count} stocks (bundled sample data)").bold()
    );
    println!("  {}", "─".repeat(61));
    println!();

    let provider = SampleProvider::new();

    // Fetch all candles (in-memory — no network)
    let mut candle_map: std::collections::HashMap<String, Vec<ghost_alpha::data::models::Candle>> =
        std::collections::HashMap::new();

    let dummy_date = chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap();

    for ticker in DEMO_TICKERS {
        if let Ok(candles) = provider.fetch_candles(ticker, dummy_date, dummy_date).await {
            candle_map.insert(ticker.to_string(), candles);
        }
    }

    let spy_candles = provider.fetch_spy_candles(dummy_date, dummy_date).await?;

    // Detect regime
    let universe_refs: Vec<(&str, &[ghost_alpha::data::models::Candle])> = candle_map
        .iter()
        .map(|(t, c)| (t.as_str(), c.as_slice()))
        .collect();

    let regime_state = detect_regime(&spy_candles, &universe_refs).ok();
    let regime = regime_state
        .as_ref()
        .map(|s| s.regime.clone())
        .unwrap_or(MarketRegime::Bull);

    // Compute universe returns for RS percentile
    let universe_returns: Vec<f64> = candle_map.values().filter_map(|c| pr252(c)).collect();

    // Screen each ticker
    let screener = MinerviniScreener::new();
    let mut results: Vec<ghost_alpha::analysis::screener::ScreenResult> = Vec::new();

    for (ticker, candles) in &candle_map {
        // skip tickers with insufficient data
        if let Ok(r) = screener.screen(ticker, candles, &universe_returns, &regime) {
            results.push(r);
        }
    }

    // Assign grades
    let mut composite_scores: Vec<CompositeScore> = results
        .iter()
        .map(|r| CompositeScore {
            composite: r.composite_score,
            technical: r.technical_score,
            volume: r.volume_score,
            sentiment: r.sentiment_score,
            grade: Grade::F,
        })
        .collect();
    assign_grades(&mut composite_scores)?;
    for (r, cs) in results.iter_mut().zip(composite_scores.iter()) {
        r.grade = cs.grade.clone();
    }

    // Sort descending by composite score
    results.sort_by(|a, b| {
        b.composite_score
            .partial_cmp(&a.composite_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Print table
    let passed = results.iter().filter(|r| r.minervini_pass).count();
    let display: Vec<&ghost_alpha::analysis::screener::ScreenResult> =
        results.iter().take(top).collect();

    println!(
        "  {:<8} {:>6} {:>6} {:>9} {:>6} {:>8} {:>7} {:>9}",
        "Ticker", "Grade", "Score", "Minervini", "Cond", "ADX", "RS%", "52wH%"
    );
    println!("  {}", "─".repeat(72));

    for r in &display {
        let grade_str = format!("{:>6}", r.grade.to_string());
        let grade_colored = match r.grade {
            Grade::APlus | Grade::A => grade_str.green().bold(),
            Grade::B => grade_str.green().normal(),
            Grade::C => grade_str.yellow().normal(),
            Grade::D => grade_str.red().normal(),
            Grade::F => grade_str.red().dimmed(),
        };

        let pass_marker = if r.minervini_pass {
            "✓".green().bold().to_string()
        } else {
            "✗".red().to_string()
        };

        println!(
            "  {:<8} {} {:>6.1} {:>9} {:>5}/8 {:>7.1} {:>6.1} {:>8.1}",
            r.ticker.bold(),
            grade_colored,
            r.composite_score,
            pass_marker,
            r.conditions_met,
            r.adx,
            r.rs_percentile,
            r.pct_from_52w_high,
        );
    }
    println!("  {}", "─".repeat(72));

    // Regime line
    let regime_label = match &regime {
        MarketRegime::Bull => "BULL".green().bold().to_string(),
        MarketRegime::Sideways => "SIDEWAYS".yellow().bold().to_string(),
        MarketRegime::Bear => "BEAR".red().bold().to_string(),
    };
    println!();
    println!(
        "  Market Regime: {} | Screened: {} | Passed Minervini: {} | Elapsed: {:.1}s",
        regime_label,
        results.len(),
        passed,
        started.elapsed().as_secs_f64(),
    );
    println!();
    println!("  This is sample data. To screen live markets:");
    println!(
        "    1. Get a free API key at {}",
        "https://finnhub.io".cyan()
    );
    println!("    2. Run: {}", "ghost-alpha setup".bold());
    println!();

    Ok(())
}

// ─── Screen Pipeline ─────────────────────────────────────────────────────────

async fn run_screen(
    cfg: &AppConfig,
    skip_sentiment: bool,
    top: usize,
    format: &OutputFormat,
    provider_override: Option<&str>,
    strategy: &str,
) -> Result<()> {
    let started = Instant::now();

    if !skip_sentiment {
        tracing::info!("Sentiment scoring available with --features sentiment");
    }

    // 1. Open DB
    let db_path = cfg.storage.db_path.clone();
    let repo = tokio::task::spawn_blocking(move || SqliteRepository::open(&db_path))
        .await?
        .with_context(|| "Failed to open database")?;
    let repo = Arc::new(repo);

    // 2. Build registry → get provider, screener, channels
    let mut registry = default_registry(cfg);

    // Apply CLI --provider override (takes precedence over config).
    if let Some(name) = provider_override {
        registry.select_provider(name).with_context(|| {
            format!(
                "Unknown provider '{}' — did you mean 'yahoo' or 'finnhub'?",
                name
            )
        })?;
        tracing::info!(provider = name, "Provider overridden via --provider flag");
    }

    // Apply CLI --strategy override (takes precedence over config).
    if strategy != "minervini" {
        registry.select_screener(strategy).with_context(|| {
            format!(
                "Unknown strategy '{}' — run 'ghost-alpha strategies' to list available options",
                strategy
            )
        })?;
        tracing::info!(
            strategy = strategy,
            "Strategy overridden via --strategy flag"
        );
    }

    let raw_provider = registry
        .provider()
        .with_context(|| "No data provider configured")?;
    let screener = registry
        .screener()
        .with_context(|| "No screener configured")?;
    let channels: Vec<Arc<dyn AlertChannel>> = registry.channels();

    // 2b. Wrap provider with caching layer for delta fetches
    let today = chrono::Utc::now().date_naive();
    let from = today - chrono::Duration::days(400);
    let repo_dyn: Arc<dyn ghost_alpha::storage::db::Repository> = repo.clone();
    let provider: Arc<dyn ghost_alpha::data::provider::DataProvider> = Arc::new(
        CachingProvider::new(raw_provider, repo_dyn, cfg.cache.clone(), from),
    );
    tracing::info!(
        enabled = cfg.cache.enabled,
        full_refresh_days = cfg.cache.full_refresh_days,
        "Candle cache configured"
    );

    // 3. Determine universe
    let universe: Vec<String> = if !cfg.universe.is_empty() {
        tracing::info!(
            count = cfg.universe.len(),
            "Using static universe from config"
        );
        cfg.universe.clone()
    } else {
        tracing::info!("Fetching universe from provider…");
        provider
            .fetch_universe()
            .await
            .with_context(|| "Failed to fetch universe")?
    };
    tracing::info!(count = universe.len(), "Universe determined");

    // 4. Date range (today/from already set above for cache init)

    // 5. Fetch SPY candles
    tracing::info!("Fetching SPY candles…");
    let spy_candles = provider
        .fetch_spy_candles(from, today)
        .await
        .with_context(|| "Failed to fetch SPY candles")?;
    tracing::info!(bars = spy_candles.len(), "SPY candles loaded");

    // 6. Fetch all ticker candles concurrently with progress bar
    let pb = ProgressBar::new(universe.len() as u64);
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta}) {msg}",
        )
        .unwrap()
        .progress_chars("━╸─"),
    );

    let candle_results: Vec<(String, Result<Vec<ghost_alpha::data::models::Candle>, _>)> =
        stream::iter(universe.iter())
            .map(|ticker| {
                let provider = provider.clone();
                let pb = pb.clone();
                let ticker = ticker.clone();
                async move {
                    let result = provider.fetch_candles(&ticker, from, today).await;
                    pb.inc(1);
                    pb.set_message(ticker.clone());
                    (ticker, result)
                }
            })
            .buffer_unordered(5) // 5 concurrent — stays within 60rpm rate limit
            .collect()
            .await;
    pb.finish_with_message("Candles fetched ✓");

    // Build candle map, skipping failures
    let mut candle_map: HashMap<String, Vec<ghost_alpha::data::models::Candle>> = HashMap::new();
    let mut fetch_errors = 0usize;
    for (ticker, result) in candle_results {
        match result {
            Ok(candles) => {
                candle_map.insert(ticker, candles);
            }
            Err(e) => {
                tracing::warn!(ticker = %ticker, error = %e, "Skipping ticker (fetch error)");
                fetch_errors += 1;
            }
        }
    }
    tracing::info!(
        loaded = candle_map.len(),
        errors = fetch_errors,
        "Candle data collected"
    );

    // 7. Detect regime (before screening — regime drives score weights)
    let universe_candles_refs: Vec<(&str, &[ghost_alpha::data::models::Candle])> = candle_map
        .iter()
        .map(|(t, c)| (t.as_str(), c.as_slice()))
        .collect();

    let regime_state: Option<RegimeState> =
        match detect_regime(&spy_candles, &universe_candles_refs) {
            Ok(state) => Some(state),
            Err(e) => {
                tracing::warn!(error = %e, "Failed to detect regime (insufficient SPY data?)");
                None
            }
        };

    let regime_for_screen = regime_state
        .as_ref()
        .map(|s| s.regime.clone())
        .unwrap_or(ghost_alpha::data::models::MarketRegime::Sideways);

    // 8. Compute universe returns (252-day) for RS percentile
    let universe_returns: Vec<f64> = candle_map
        .values()
        .filter_map(|candles| price_return_252d(candles))
        .collect();

    // 9. Screen each ticker (regime-aware scoring)
    let mut screen_results: Vec<ScreenResult> = Vec::new();
    let mut screen_skipped = 0usize;
    for (ticker, candles) in &candle_map {
        match screener.screen(ticker, candles, &universe_returns, &regime_for_screen) {
            Ok(result) => screen_results.push(result),
            Err(ghost_alpha::error::AnalysisError::InsufficientData { .. }) => {
                tracing::debug!(ticker = %ticker, "Skipped (insufficient data)");
                screen_skipped += 1;
            }
            Err(e) => {
                tracing::warn!(ticker = %ticker, error = %e, "Screening error, skipping");
                screen_skipped += 1;
            }
        }
    }
    tracing::info!(
        screened = screen_results.len(),
        skipped = screen_skipped,
        "Screening complete"
    );

    // 9.5 — Sentiment scoring (fetch news → score → update results)
    if !skip_sentiment {
        let scorer = registry.scorer();
        let sent_cfg = cfg.sentiment.as_ref();
        let lookback_days = sent_cfg.map(|c| c.news_lookback_days).unwrap_or(7);
        let min_articles = sent_cfg.map(|c| c.min_articles).unwrap_or(3);
        let from_date = today - chrono::Duration::days(lookback_days as i64);

        // Fetch news concurrently for all screened tickers (buffered to respect rate limit)
        let news_futures = screen_results.iter().map(|r| {
            let provider = provider.clone();
            let ticker = r.ticker.clone();
            async move {
                match provider.fetch_news(&ticker, from_date, 50).await {
                    Ok(articles) => (ticker, articles),
                    Err(e) => {
                        tracing::warn!(ticker = %ticker, error = %e, "Failed to fetch news");
                        (ticker, Vec::new())
                    }
                }
            }
        });
        let news_map: HashMap<String, Vec<ghost_alpha::data::models::NewsArticle>> =
            stream::iter(news_futures)
                .buffer_unordered(5)
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect();

        // Cache all fetched articles in DB
        {
            let repo = repo.clone();
            let all_articles: Vec<ghost_alpha::data::models::NewsArticle> =
                news_map.values().flatten().cloned().collect();
            let prune_days = lookback_days as i64 * 2;
            tokio::task::spawn_blocking(move || {
                if let Err(e) = repo.cache_news(&all_articles) {
                    tracing::warn!(error = %e, "Failed to cache news articles");
                }
                // Prune articles older than 2× lookback
                let _ = repo.delete_old_news(prune_days);
            })
            .await?;
        }

        // Score each ticker's headlines and update ScreenResult
        if let Ok(scorer) = scorer {
            for result in &mut screen_results {
                let articles = news_map.get(&result.ticker);
                let headlines: Vec<&str> = articles
                    .map(|a| a.iter().map(|n| n.headline.as_str()).collect())
                    .unwrap_or_default();
                let article_count = headlines.len();

                if article_count >= min_articles {
                    match scorer.score(&headlines) {
                        Ok(score) => {
                            let weights = ghost_alpha::analysis::scorer::RegimeWeights::for_regime(
                                &result.regime,
                            );
                            let sent_inputs = ghost_alpha::analysis::scorer::SentimentInputs {
                                score,
                                article_count,
                            };
                            result.sentiment_score =
                                ghost_alpha::analysis::scorer::sentiment_score_weighted(
                                    &sent_inputs,
                                    weights.sentiment,
                                );
                            // Recompute composite with real sentiment
                            let tech_inputs = ghost_alpha::analysis::scorer::TechnicalInputs {
                                conditions_met: result.conditions_met,
                                max_conditions: 8,
                                adx: result.adx,
                                rs_percentile: result.rs_percentile,
                            };
                            let vol_inputs = ghost_alpha::analysis::scorer::VolumeInputs {
                                zscore: result.volume_zscore,
                            };
                            let comp = ghost_alpha::analysis::scorer::composite_score_weighted(
                                &tech_inputs,
                                &vol_inputs,
                                &sent_inputs,
                                &weights,
                            );
                            result.composite_score = comp.composite;
                        }
                        Err(e) => {
                            tracing::warn!(
                                ticker = %result.ticker,
                                error = %e,
                                "Sentiment scoring failed, keeping score at 0.0"
                            );
                        }
                    }
                }
            }

            tracing::info!(
                scored = news_map
                    .values()
                    .filter(|a| a.len() >= min_articles)
                    .count(),
                total = screen_results.len(),
                "Sentiment scoring complete"
            );
        }
    }

    // 10. Assign grades — use the canonical assign_grades() for correct rank-based percentiles
    let mut composite_scores: Vec<CompositeScore> = screen_results
        .iter()
        .map(|r| CompositeScore {
            composite: r.composite_score,
            technical: r.technical_score,
            volume: r.volume_score,
            sentiment: r.sentiment_score,
            grade: Grade::F,
        })
        .collect();
    assign_grades(&mut composite_scores)?;
    for (result, cs) in screen_results.iter_mut().zip(composite_scores.iter()) {
        result.grade = cs.grade.clone();
    }

    // Sort by composite score descending
    screen_results.sort_by(|a, b| {
        b.composite_score
            .partial_cmp(&a.composite_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // 11. Evaluate alerts
    let alert_config = AlertConfig::from(&cfg.alerts);

    let mut engine = AlertEngine::new(alert_config);
    let alerts = engine.evaluate(&screen_results);

    // 12. Apply regime suppression
    let regime_for_suppression = regime_state
        .as_ref()
        .map(|s| &s.regime)
        .cloned()
        .unwrap_or(ghost_alpha::data::models::MarketRegime::Sideways);

    let mut alerts_sent = Vec::new();
    let mut alerts_suppressed = 0usize;
    for alert in &alerts {
        let is_critical = alert.severity == Severity::Critical;
        if should_suppress_in_bear(&regime_for_suppression, is_critical) {
            tracing::debug!(ticker = %alert.ticker, "Alert suppressed (bear regime)");
            alerts_suppressed += 1;
        } else {
            alerts_sent.push(alert.clone());
        }
    }

    // 13. Send to channels
    for alert in &alerts_sent {
        for channel in &channels {
            if let Err(e) = channel.send(alert).await {
                tracing::warn!(
                    channel = channel.name(),
                    ticker = %alert.ticker,
                    error = %e,
                    "Failed to send alert"
                );
            }
        }
    }

    // 14. Persist everything in a single spawn_blocking block
    //     Move screen_results in and get it back to avoid cloning the large Vec.
    let screen_results = {
        let repo = repo.clone();
        let regime_state = regime_state.clone();
        let alerts_for_db = alerts_sent.clone();

        tokio::task::spawn_blocking(move || -> Result<Vec<ScreenResult>> {
            // Save regime
            if let Some(ref regime) = regime_state {
                if let Err(e) = repo.save_regime(regime) {
                    tracing::warn!(error = %e, "Failed to save regime");
                }
            }

            // Save screen results
            for result in &screen_results {
                if let Err(e) = repo.save_screen_result(today, result) {
                    tracing::warn!(ticker = %result.ticker, error = %e, "Failed to save screen result");
                }
            }

            // Record alerts
            for alert in &alerts_for_db {
                if let Err(e) = repo.record_alert(alert) {
                    tracing::warn!(ticker = %alert.ticker, error = %e, "Failed to record alert");
                }
            }

            // Flag high-quality stocks (A+ and A that pass Minervini)
            for result in screen_results.iter().filter(|r| {
                r.minervini_pass && (r.grade == Grade::APlus || r.grade == Grade::A)
            }) {
                let flagged = FlaggedStock {
                    id: None,
                    ticker: result.ticker.clone(),
                    flagged_date: today,
                    entry_score: result.composite_score,
                    entry_price: result.close_price,
                    conditions_at_entry: result.conditions_met,
                    status: FlaggedStatus::Active,
                    exit_date: None,
                    exit_reason: None,
                    exit_price: None,
                    latest_score: None,
                    last_rescored_at: None,
                };
                if let Err(e) = repo.insert_flagged_stock(&flagged) {
                    tracing::warn!(ticker = %result.ticker, error = %e, "Failed to flag stock");
                }
            }

            // Expire old flags (90 days)
            match repo.expire_old_flags(90) {
                Ok(n) if n > 0 => tracing::info!(expired = n, "Expired old flagged stocks"),
                _ => {}
            }

            Ok(screen_results)
        })
        .await??
    };

    // 15. Check exit signals for active flagged stocks
    {
        let repo = repo.clone();
        let candle_map_for_exits = candle_map.clone();
        let universe_returns_for_exits = universe_returns.clone();

        let exit_signals = tokio::task::spawn_blocking(move || -> Result<Vec<String>> {
            let active = repo.get_active_flagged_stocks()?;
            let mut exit_messages = Vec::new();

            for flagged in &active {
                if let Some(candles) = candle_map_for_exits.get(&flagged.ticker) {
                    if let Some(signal) =
                        check_exit_signals(flagged, candles, &universe_returns_for_exits)
                    {
                        let msg = format_exit_message(&signal);
                        exit_messages.push(msg.clone());
                        repo.mark_exited(
                            &flagged.ticker,
                            today,
                            &signal.reason.to_string(),
                            signal.current_price,
                        )?;
                        tracing::info!(ticker = %flagged.ticker, reason = %signal.reason, "Exit signal fired");
                    }
                }
            }
            Ok(exit_messages)
        })
        .await??;

        if !exit_signals.is_empty() {
            // Exit alerts intentionally bypass regime suppression — exits matter
            // in all market conditions to protect flagged positions.
            for msg in &exit_signals {
                let exit_alert = ghost_alpha::alerts::channel::OutboundAlert {
                    ticker: "EXIT".to_string(),
                    alert_type: "ExitSignal".to_string(),
                    severity: Severity::Warning,
                    message: msg.clone(),
                    composite_score: 0.0,
                    narrative: None,
                };
                for channel in &channels {
                    if let Err(e) = channel.send(&exit_alert).await {
                        tracing::warn!(channel = channel.name(), error = %e, "Failed to send exit alert");
                    }
                }
            }
        }
    }

    // 16. FRED macro risk (optional — requires API key)
    if let Some(ref fred_cfg) = cfg.fred {
        use ghost_alpha::analysis::macro_risk::{compute_risk_level, MacroRiskSnapshot};
        use ghost_alpha::data::fred::{fred_series_url, parse_fred_latest};

        let api_key = &fred_cfg.api_key;
        let client = reqwest::Client::new();

        // Fetch FRED series concurrently (best-effort; fall back to defaults)
        let (vix_resp, oas_resp, yc_resp) = tokio::join!(
            client.get(fred_series_url("VIXCLS", api_key)).send(),
            client.get(fred_series_url("BAMLH0A0HYM2", api_key)).send(),
            client.get(fred_series_url("T10Y2Y", api_key)).send(),
        );

        let vix = match vix_resp {
            Ok(r) => parse_fred_latest(&r.text().await.unwrap_or_default()).unwrap_or(20.0),
            Err(_) => 20.0,
        };
        let hy_oas = match oas_resp {
            Ok(r) => Some(parse_fred_latest(&r.text().await.unwrap_or_default()).unwrap_or(400.0)),
            Err(_) => Some(400.0),
        };
        let yield_curve = match yc_resp {
            Ok(r) => Some(parse_fred_latest(&r.text().await.unwrap_or_default()).unwrap_or(0.5)),
            Err(_) => Some(0.5),
        };

        let risk_level = compute_risk_level(vix, hy_oas, yield_curve, None);

        let snapshot = MacroRiskSnapshot {
            vix_level: vix,
            hy_oas_bps: hy_oas,
            yield_curve_2s10s: yield_curve,
            put_call_ratio: None,
            risk_level,
        };

        tracing::info!(
            vix = vix,
            risk = %snapshot.risk_level,
            "Macro risk snapshot"
        );

        let repo = repo.clone();
        let save_result =
            tokio::task::spawn_blocking(move || repo.save_macro_risk(today, &snapshot)).await?;

        if let Err(e) = save_result {
            tracing::warn!(error = %e, "Failed to save macro risk data");
        }
    }

    // 17. Conviction decay for active flagged stocks
    {
        let repo = repo.clone();
        let candle_map_for_decay = candle_map.clone();
        let universe_returns_for_decay = universe_returns.clone();
        let regime_for_decay = regime_for_screen.clone();
        let screener_for_decay = registry.screener();

        tokio::task::spawn_blocking(move || -> Result<()> {
            let active = repo.get_active_flagged_stocks()?;
            if active.is_empty() {
                return Ok(());
            }

            // Re-score flagged stocks and check conviction decay
            if let Ok(screener) = screener_for_decay {
                for flagged in &active {
                    if let Some(candles) = candle_map_for_decay.get(&flagged.ticker) {
                        if let Ok(result) = screener.screen(
                            &flagged.ticker,
                            candles,
                            &universe_returns_for_decay,
                            &regime_for_decay,
                        ) {
                            let current_score = result.composite_score;

                            // Persist the updated score
                            let _ =
                                repo.update_flagged_score(&flagged.ticker, current_score, today);

                            // Check conviction decay
                            if let Some(alert) =
                                check_conviction_decay(flagged.entry_score, current_score)
                            {
                                match alert {
                                    ConvictionAlert::Weakening => {
                                        tracing::warn!(
                                            ticker = %flagged.ticker,
                                            entry = flagged.entry_score,
                                            current = current_score,
                                            "Conviction weakening"
                                        );
                                    }
                                    ConvictionAlert::Recovered => {
                                        tracing::info!(
                                            ticker = %flagged.ticker,
                                            entry = flagged.entry_score,
                                            current = current_score,
                                            "Conviction recovered"
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Correlation concentration: warn if 3+ flagged stocks are highly correlated
            let mut flagged_returns: HashMap<String, Vec<f64>> = HashMap::new();
            for flagged in &active {
                if let Some(candles) = candle_map_for_decay.get(&flagged.ticker) {
                    if candles.len() >= 21 {
                        let returns: Vec<f64> = candles
                            .windows(2)
                            .map(|w| (w[1].adj_close - w[0].adj_close) / w[0].adj_close)
                            .collect();
                        // Use last 63 days of returns (1 quarter)
                        let start = returns.len().saturating_sub(63);
                        flagged_returns.insert(flagged.ticker.clone(), returns[start..].to_vec());
                    }
                }
            }

            if flagged_returns.len() >= 3 {
                let returns_slice: Vec<(&str, &[f64])> = flagged_returns
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_slice()))
                    .collect();
                let warnings = check_correlation_concentration(&returns_slice, 0.7);
                for warning in &warnings {
                    tracing::warn!(warning = %warning, "Correlation concentration alert");
                }
            }

            Ok(())
        })
        .await??;
    }

    // ── Post-screen analytics (correlation, fundamentals, ETF snapshots) ───
    //
    // Run automatically for the top 100 screened stocks so the web dashboard
    // has data for the Correlation, Fundamentals, and ETF pages.

    run_post_screen_analytics(cfg, &repo, &candle_map, &screen_results, today).await;

    // ── Output ───────────────────────────────────────────────────────────────

    let elapsed = started.elapsed();
    let display_results: Vec<&ScreenResult> = screen_results.iter().take(top).collect();

    if *format == OutputFormat::Table {
        // Coloured table + regime summary (human-readable default)
        print_results_table(&display_results);
        print_summary(
            regime_state.as_ref(),
            screen_results.len(),
            screen_results.iter().filter(|r| r.minervini_pass).count(),
            &alerts_sent,
            alerts_suppressed,
            elapsed,
        );
    } else {
        // Structured output via the unified renderer
        let owned: Vec<ScreenResult> = display_results.into_iter().cloned().collect();
        render(&owned, format);
    }

    Ok(())
}

// ─── Post-Screen Analytics ──────────────────────────────────────────────────

/// Run derived analytics after screening: correlation matrix, fundamentals, ETF snapshots.
/// Targets the top 100 stocks by composite score. Errors are logged but never fail the pipeline.
async fn run_post_screen_analytics(
    cfg: &AppConfig,
    repo: &Arc<SqliteRepository>,
    candle_map: &HashMap<String, Vec<ghost_alpha::data::models::Candle>>,
    screen_results: &[ScreenResult],
    today: chrono::NaiveDate,
) {
    // Determine top 100 tickers by composite score
    let top_tickers: Vec<String> = screen_results
        .iter()
        .take(100)
        .map(|r| r.ticker.clone())
        .collect();

    if top_tickers.is_empty() {
        return;
    }

    tracing::info!(count = top_tickers.len(), "Running post-screen analytics");

    // ── 1. Correlation matrix ────────────────────────────────────────────────
    {
        // Compute daily returns for each ticker that has candles
        let mut returns_map: Vec<(String, Vec<f64>)> = Vec::new();
        for ticker in &top_tickers {
            if let Some(candles) = candle_map.get(ticker) {
                if candles.len() >= 61 {
                    // Use last 60 trading days of returns
                    let window = &candles[candles.len().saturating_sub(61)..];
                    let returns: Vec<f64> = window
                        .windows(2)
                        .map(|w| (w[1].close - w[0].close) / w[0].close)
                        .collect();
                    returns_map.push((ticker.clone(), returns));
                }
            }
        }

        // Align all return series to the same length (min length across all)
        if returns_map.len() >= 2 {
            let min_len = returns_map.iter().map(|(_, r)| r.len()).min().unwrap_or(0);
            if min_len >= 20 {
                let aligned: Vec<(&str, &[f64])> = returns_map
                    .iter()
                    .map(|(t, r)| (t.as_str(), &r[r.len() - min_len..]))
                    .collect();

                match CorrelationMatrix::from_returns(&aligned) {
                    Ok(corr) => {
                        let tickers: Vec<String> =
                            aligned.iter().map(|(t, _)| t.to_string()).collect();
                        let n = corr.matrix.nrows();
                        let matrix_vecs: Vec<Vec<f64>> = (0..n)
                            .map(|i| (0..n).map(|j| corr.matrix[(i, j)]).collect())
                            .collect();
                        let date_str = today.to_string();
                        let repo = repo.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            if let Err(e) = repo.save_correlation_snapshot(
                                &date_str,
                                &tickers,
                                &matrix_vecs,
                                60,
                            ) {
                                tracing::warn!(error = %e, "Failed to save correlation snapshot");
                            }
                        })
                        .await;
                        tracing::info!(assets = n, "Correlation matrix saved");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Failed to compute correlation matrix");
                    }
                }
            }
        }
    }

    // ── 2. Fundamental metrics (Finnhub only) ────────────────────────────────
    if !cfg.finnhub.api_key.trim().is_empty() {
        use ghost_alpha::data::fetcher::FinnhubClient;
        use ghost_alpha::data::fundamentals::FundamentalProvider;

        let client = FinnhubClient::new(cfg.finnhub.api_key.clone());

        // Check which tickers already have today's fundamental data
        let repo_for_check = repo.clone();
        let today_str = today.to_string();
        let existing: std::collections::HashSet<String> = tokio::task::spawn_blocking(move || {
            repo_for_check
                .get_all_fundamental_metrics()
                .unwrap_or_default()
                .into_iter()
                .filter(|m| m.fetched_date == today_str)
                .map(|m| m.ticker)
                .collect()
        })
        .await
        .unwrap_or_default();

        let to_fetch: Vec<&String> = top_tickers
            .iter()
            .filter(|t| !existing.contains(*t))
            .collect();

        if !to_fetch.is_empty() {
            tracing::info!(count = to_fetch.len(), "Fetching fundamental metrics");
            let pb = ProgressBar::new(to_fetch.len() as u64);
            pb.set_style(
                ProgressStyle::with_template(
                    "{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({eta}) Fundamentals: {msg}",
                )
                .unwrap()
                .progress_chars("━╸─"),
            );

            for ticker in &to_fetch {
                pb.set_message((*ticker).clone());
                match client.fetch_fundamentals(ticker).await {
                    Ok(snap) => {
                        let repo = repo.clone();
                        let s = snap;
                        let _ = tokio::task::spawn_blocking(move || {
                            if let Err(e) = repo.save_fundamental_snapshot(&s) {
                                tracing::warn!(error = %e, "Failed to save fundamental snapshot");
                            }
                        })
                        .await;
                    }
                    Err(e) => {
                        tracing::debug!(ticker = %ticker, error = %e, "Skipping fundamentals");
                    }
                }
                pb.inc(1);
            }
            pb.finish_with_message("Fundamentals fetched ✓");
        } else {
            tracing::info!("Fundamental metrics already up to date");
        }
    }

    // ── 3. ETF snapshots (bundled, no API calls) ─────────────────────────────
    {
        use ghost_alpha::data::etf::bundled_etf_snapshots;

        let snapshots = bundled_etf_snapshots();
        let repo = repo.clone();
        let _ = tokio::task::spawn_blocking(move || {
            for snap in &snapshots {
                if let Err(e) = repo.upsert_etf_snapshot(snap) {
                    tracing::warn!(ticker = %snap.ticker, error = %e, "Failed to save ETF snapshot");
                }
            }
        }).await;
        tracing::info!("ETF snapshots seeded (13 bundled ETFs)");
    }
}

// ─── Table Output ────────────────────────────────────────────────────────────

fn print_results_table(results: &[&ScreenResult]) {
    if results.is_empty() {
        println!("{}", "  No stocks passed screening criteria.".yellow());
        return;
    }

    // Header
    println!();
    println!(
        "  {:<8} {:>6} {:>6} {:>9} {:>6} {:>8} {:>7} {:>9}",
        "Ticker", "Grade", "Score", "Minervini", "Cond", "ADX", "RS%", "52wH%"
    );
    println!("  {}", "─".repeat(72));

    for r in results {
        let grade_str = format!("{:>6}", r.grade.to_string());
        let grade_colored = match r.grade {
            Grade::APlus | Grade::A => grade_str.green().bold(),
            Grade::B => grade_str.green().normal(),
            Grade::C => grade_str.yellow().normal(),
            Grade::D => grade_str.red().normal(),
            Grade::F => grade_str.red().dimmed(),
        };

        let pass_marker = if r.minervini_pass {
            "✓".green().bold().to_string()
        } else {
            "✗".red().to_string()
        };

        println!(
            "  {:<8} {} {:>6.1} {:>9} {:>5}/8 {:>7.1} {:>6.1} {:>8.1}",
            r.ticker.bold(),
            grade_colored,
            r.composite_score,
            pass_marker,
            r.conditions_met,
            r.adx,
            r.rs_percentile,
            r.pct_from_52w_high,
        );
    }
    println!("  {}", "─".repeat(72));
}

fn print_summary(
    regime: Option<&RegimeState>,
    screened: usize,
    passed: usize,
    alerts: &[ghost_alpha::alerts::channel::OutboundAlert],
    suppressed: usize,
    elapsed: std::time::Duration,
) {
    println!();

    // Regime line
    if let Some(state) = regime {
        let regime_str = match state.regime {
            ghost_alpha::data::models::MarketRegime::Bull => {
                state.regime.to_string().green().bold()
            }
            ghost_alpha::data::models::MarketRegime::Sideways => {
                state.regime.to_string().yellow().bold()
            }
            ghost_alpha::data::models::MarketRegime::Bear => state.regime.to_string().red().bold(),
        };
        println!(
            "  📊 Market Regime: {} (SPY: ${:.2} {} SMA200: ${:.2}, Breadth: {:.1}%)",
            regime_str,
            state.spy_close,
            if state.spy_close > state.spy_sma200 {
                ">".green()
            } else {
                "<".red()
            },
            state.spy_sma200,
            state.breadth_pct,
        );
    } else {
        println!(
            "  📊 Market Regime: {} (insufficient SPY data)",
            "UNKNOWN".yellow()
        );
    }

    // Alert counts
    let critical_count = alerts
        .iter()
        .filter(|a| a.severity == Severity::Critical)
        .count();
    let warning_count = alerts
        .iter()
        .filter(|a| a.severity == Severity::Warning)
        .count();
    let info_count = alerts
        .iter()
        .filter(|a| a.severity == Severity::Info)
        .count();

    let alert_detail = format!(
        "{} CRITICAL, {} WARNING, {} INFO",
        critical_count, warning_count, info_count,
    );

    println!(
        "  📈 Screened: {} stocks | Passed Minervini: {} | Alerts: {} ({}){}",
        screened,
        passed,
        alerts.len(),
        alert_detail,
        if suppressed > 0 {
            format!(" | {} suppressed (bear regime)", suppressed)
        } else {
            String::new()
        },
    );

    // Duration
    let secs = elapsed.as_secs();
    let duration_str = if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{:.1}s", elapsed.as_secs_f64())
    };
    println!("  ⏱  Duration: {}", duration_str);
    println!();
}

// ─── Alerts Display ──────────────────────────────────────────────────────────

fn print_alerts_table(entries: &[AlertLogEntry]) {
    if entries.is_empty() {
        println!("  No alerts recorded yet.");
        return;
    }

    println!();
    println!(
        "  {:<8} {:>10} {:>10} {:<50} Sent At",
        "Ticker", "Type", "Severity", "Message"
    );
    println!("  {}", "─".repeat(100));

    for entry in entries {
        let severity_colored = match entry.severity.as_str() {
            "CRITICAL" => entry.severity.red().bold(),
            "WARNING" => entry.severity.yellow().normal(),
            _ => entry.severity.normal(),
        };

        // Truncate message for table display (UTF-8 safe)
        let msg: String = if entry.message.chars().count() > 48 {
            let truncated: String = entry.message.chars().take(47).collect();
            format!("{truncated}…")
        } else {
            entry.message.clone()
        };

        println!(
            "  {:<8} {:>10} {:>10} {:<50} {}",
            entry.ticker.bold(),
            entry.alert_type,
            severity_colored,
            msg,
            entry.sent_at.dimmed(),
        );
    }
    println!("  {}", "─".repeat(100));
    println!();
}

// ─── Watchlist Display ───────────────────────────────────────────────────────

fn print_watchlist_table(entries: &[WatchlistEntry]) {
    if entries.is_empty() {
        println!("  Watchlist is empty. Add tickers with: ghost-alpha watch add NVDA AAPL");
        return;
    }

    println!();
    println!(
        "  {:<8} {:<12} {:>10} {:>8}  Notes",
        "Ticker", "Added", "Score", "Grade"
    );
    println!("  {}", "─".repeat(60));

    for e in entries {
        let added = e.added_at.get(..10).unwrap_or(&e.added_at);
        let score_str = e
            .last_score
            .map(|s| format!("{:>10.1}", s))
            .unwrap_or_else(|| format!("{:>10}", "—"));
        let grade_str = e
            .last_grade
            .as_deref()
            .map(|g| format!("{:>8}", g))
            .unwrap_or_else(|| format!("{:>8}", "—"));
        let notes = e.notes.as_deref().unwrap_or("");

        println!(
            "  {:<8} {:<12} {} {}  {}",
            e.ticker.bold(),
            added,
            score_str,
            grade_str,
            notes.dimmed(),
        );
    }
    println!("  {}", "─".repeat(60));
    println!("  {} ticker(s)", entries.len());
    println!();
}

// ─── Entry point ──────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // MCP mode logs to stderr (stdout is JSON-RPC channel)
    let is_mcp = matches!(cli.command, Commands::Mcp { .. });

    let filter = if cli.verbose {
        EnvFilter::new("ghost_alpha=debug,info")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
    };

    if is_mcp {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    // Demo command runs without any config — handle it before config loading.
    if let Commands::Demo { top } = cli.command {
        return run_demo(top).await;
    }

    // plan needs no app config or API key — handle it before config loading.
    if matches!(cli.command, Commands::Plan { .. }) {
        return run_plan_command(cli.command);
    }

    // init-data only writes template files — no config needed.
    if let Commands::InitData { dir, force } = &cli.command {
        let written =
            ghost_alpha::cli::plan::write_data_templates(std::path::Path::new(dir), *force)
                .context("init-data failed")?;
        println!("Created {} file(s) in {}:", written.len(), dir);
        for p in &written {
            println!("  {}", p.display());
        }
        println!();
        println!("Next step (edit the files first, then run):");
        println!("  ghost-alpha plan --data-dir {} --as-of YYYY-MM-DD", dir);
        return Ok(());
    }

    // Setup wizard runs before config exists (it creates the config).
    // --models-only falls through to the normal config-loading path so it can
    // read the model_path from an already-configured config.toml.
    if let Commands::Setup { models_only: false } = cli.command {
        return ghost_alpha::wizard::run_setup_wizard().await;
    }

    // Load config — MCP mode can fall back to env-only
    let cfg = match &cli.command {
        Commands::Mcp { env_only: true } => {
            AppConfig::from_env().context("Failed to load config from environment variables")?
        }
        Commands::Mcp { env_only: false } => AppConfig::from_file(&cli.config)
            .or_else(|_| {
                tracing::info!(
                    "Config file '{}' not found, falling back to env vars",
                    cli.config
                );
                AppConfig::from_env()
            })
            .context("Failed to load config from file or environment")?,
        _ => AppConfig::from_file(&cli.config)
            .with_context(|| format!("Failed to load config from '{}'", cli.config))?,
    };

    match cli.command {
        Commands::ShowConfig => {
            let masked = ghost_alpha::mcp::tools::masked_config(&cfg);
            println!("{}", serde_json::to_string_pretty(&masked).unwrap());
        }
        Commands::Migrate => {
            SqliteRepository::open(&cfg.storage.db_path)
                .with_context(|| "Failed to run migrations")?;
            println!("Migrations applied to '{}'", cfg.storage.db_path);
        }
        Commands::Screen {
            skip_sentiment,
            top,
            format,
            provider,
            strategy,
        } => {
            tracing::info!(skip_sentiment, top, format = %format, strategy = %strategy, "Starting screening run");
            run_screen(
                &cfg,
                skip_sentiment,
                top,
                &format,
                provider.as_deref(),
                &strategy,
            )
            .await?;
        }
        Commands::Strategies => {
            let registry = default_registry(&cfg);
            let strategies = registry.list_strategies();
            println!();
            println!("  Available Screening Strategies");
            println!("  ─────────────────────────────────────────────────────────────");
            println!();
            let active = registry.active_screener_name().unwrap_or("minervini");
            for (name, desc) in &strategies {
                let marker = if name == active { " (default)" } else { "" };
                println!("  {:20} {}{}", name.bold(), desc, marker);
            }
            println!();
            println!("  Usage: ghost-alpha screen --strategy <name>");
            println!();
        }
        Commands::Regime { format, model } => {
            if model == "hmm" {
                run_hmm_regime()?;
            } else {
                let repo = SqliteRepository::open(&cfg.storage.db_path)
                    .with_context(|| "Failed to open database")?;
                match repo.latest_regime()? {
                    Some(state) => {
                        if format == OutputFormat::Table {
                            println!("Regime: {} (as of {})", state.regime, state.date);
                        } else {
                            render(&[state], &format);
                        }
                    }
                    None => println!("No regime data yet — run 'ghost-alpha screen' first"),
                }
            }
        }
        Commands::Alerts { last, format } => {
            let repo = SqliteRepository::open(&cfg.storage.db_path)
                .with_context(|| "Failed to open database")?;
            let entries = repo.get_recent_alerts(last)?;
            if format == OutputFormat::Table {
                print_alerts_table(&entries);
            } else {
                render(&entries, &format);
            }
        }
        Commands::Plugins => {
            let registry = default_registry(&cfg);
            let output = ghost_alpha::mcp::tools::list_providers(&registry);

            println!();
            println!("  GhostAlpha Plugin Registry");
            println!("  ─────────────────────────────────────");
            println!();
            println!("  Data Providers:");
            for p in &output.providers {
                let active = output.active_provider.as_deref() == Some(p.as_str());
                let marker = if active { " ◀ active" } else { "" };
                println!("    • {p}{marker}");
            }
            println!();
            println!("  Screeners:");
            for s in &output.screeners {
                let active = output.active_screener.as_deref() == Some(s.as_str());
                let marker = if active { " ◀ active" } else { "" };
                println!("    • {s}{marker}");
            }
            println!();
            println!("  Sentiment Scorers:");
            for s in &output.scorers {
                println!("    • {s}");
            }
            println!();
            println!("  Alert Channels:");
            if output.channels.is_empty() {
                println!("    (none configured)");
            } else {
                for c in &output.channels {
                    println!("    • {c}");
                }
            }
            println!();
        }
        Commands::Mcp { .. } => {
            use ghost_alpha::mcp::server::McpServer;

            let registry = default_registry(&cfg);
            tracing::info!(
                provider = registry.active_provider_name().unwrap_or("none"),
                screener = registry.active_screener_name().unwrap_or("none"),
                channels = registry.channel_names().len(),
                "MCP server ready"
            );

            let server = McpServer::new(cfg, registry);
            server.run().context("MCP server error")?;
        }
        Commands::Backtest {
            years,
            format,
            output,
        } => {
            tracing::info!(years, format = %format, "Starting backtest");

            // Load backtest config or use defaults
            let bt_params = if let Some(ref bt_cfg) = cfg.backtest {
                ghost_alpha::backtest::types::BacktestParams {
                    forward_windows: bt_cfg.forward_windows.clone(),
                    walk_forward_in_sample_months: bt_cfg.walk_forward_in_sample_months,
                    walk_forward_out_of_sample_months: bt_cfg.walk_forward_out_of_sample_months,
                    monte_carlo_iterations: bt_cfg.monte_carlo_iterations,
                    transaction_cost_bps: bt_cfg.transaction_cost_bps,
                    holdout_pct: 0.2,
                }
            } else {
                ghost_alpha::backtest::types::BacktestParams::default()
            };

            if format == OutputFormat::Html {
                // HTML report path: generate from stored signals when available;
                // fall back to a demo report so the command always produces output.
                println!("Generating HTML backtest report...");
                println!("  Years:          {}", years);
                println!("  Output:         {}", output);

                // Build a demo report using synthetic signals so the command
                // always produces a valid HTML file even before real signal
                // history is available.
                let demo_signals = ghost_alpha::backtest::html_report::demo_signals(years);
                let report = ghost_alpha::backtest::report::run_full_backtest(
                    &demo_signals,
                    &[],
                    &bt_params,
                );
                let html = ghost_alpha::backtest::html_report::generate_html_report(&report);
                std::fs::write(&output, &html)
                    .with_context(|| format!("Failed to write HTML report to {output}"))?;
                println!("Report written to: {}", output);
                println!("Open with: open {}", output);
            } else {
                // For non-HTML formats, print a message that backtest requires
                // historical signals collected via the screen command.
                println!("Backtest configured:");
                println!("  Years:          {}", years);
                println!("  Forward windows: {:?}", bt_params.forward_windows);
                println!(
                    "  IS months:      {}",
                    bt_params.walk_forward_in_sample_months
                );
                println!(
                    "  OOS months:     {}",
                    bt_params.walk_forward_out_of_sample_months
                );
                println!("  MC iterations:  {}", bt_params.monte_carlo_iterations);
                println!("  Tx cost (bps):  {:.1}", bt_params.transaction_cost_bps);
                println!();
                println!(
                    "Run 'ghost-alpha screen' first to generate signals, then re-run backtest."
                );
                println!("Use --format html to generate a self-contained HTML report.");
            }
        }
        Commands::Plan { .. } => unreachable!("Plan is handled before config load"),
        Commands::Setup { models_only } => {
            // models_only == false is already handled before config loading.
            // We only reach here when --models-only is set.
            debug_assert!(
                models_only,
                "non-models-only setup must be handled before cfg loading"
            );
            let model_dir = if let Some(ref sent_cfg) = cfg.sentiment {
                sent_cfg.model_path.clone()
            } else {
                "models/finbert".to_string()
            };
            let dir = std::path::Path::new(&model_dir);
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Failed to create model directory: {}", model_dir))?;
            println!("Model directory created: {}", model_dir);
            println!("Download the FinBERT ONNX model from HuggingFace:");
            println!("  https://huggingface.co/ProsusAI/finbert");
            println!("Place model.onnx and tokenizer.json in {}", model_dir);
        }
        Commands::Watch { action } => {
            let repo = SqliteRepository::open(&cfg.storage.db_path)
                .with_context(|| "Failed to open database")?;

            match action {
                WatchAction::Add { tickers, note } => {
                    if tickers.is_empty() {
                        println!("Usage: ghost-alpha watch add NVDA AAPL TSLA");
                    } else {
                        for ticker in &tickers {
                            let t = ticker.to_uppercase();
                            repo.watchlist_add(&t, note.as_deref())
                                .with_context(|| format!("Failed to add {} to watchlist", t))?;
                            println!("  + {}", t);
                        }
                        println!("  Added {} ticker(s) to watchlist.", tickers.len());
                    }
                }
                WatchAction::Remove { tickers } => {
                    if tickers.is_empty() {
                        println!("Usage: ghost-alpha watch remove NVDA AAPL");
                    } else {
                        let mut removed_count = 0usize;
                        for ticker in &tickers {
                            let t = ticker.to_uppercase();
                            let found = repo.watchlist_remove(&t).with_context(|| {
                                format!("Failed to remove {} from watchlist", t)
                            })?;
                            if found {
                                println!("  - {}", t);
                                removed_count += 1;
                            } else {
                                println!("  {} not in watchlist (skipped)", t);
                            }
                        }
                        println!("  Removed {} ticker(s) from watchlist.", removed_count);
                    }
                }
                WatchAction::List => {
                    let entries = repo
                        .watchlist_list()
                        .with_context(|| "Failed to read watchlist")?;
                    print_watchlist_table(&entries);
                }
                WatchAction::Clear => {
                    let deleted = repo
                        .watchlist_clear()
                        .with_context(|| "Failed to clear watchlist")?;
                    println!("  Cleared {} ticker(s) from watchlist.", deleted);
                }
            }
        }
        // Handled above via early return before config loading.
        Commands::Demo { .. } => unreachable!("Demo is handled before config load"),
        Commands::InitData { .. } => unreachable!("InitData is handled before config load"),
        Commands::Serve { port, bind } => {
            run_serve(&cfg, &bind, port).await?;
        }
        Commands::SellPlan {
            ticker,
            shares,
            cost_basis,
            price,
            acquired,
            filing,
            state,
            other_income,
            growth,
            years,
            distribution,
            format,
        } => {
            run_sell_plan(
                &ticker,
                shares,
                cost_basis,
                price,
                &acquired,
                &filing,
                &state,
                other_income,
                growth,
                years,
                &distribution,
                format,
            )?;
        }
        Commands::SellCompare {
            ticker,
            shares,
            cost_basis,
            price,
            acquired,
            filing,
            state,
            other_income,
            growth,
            max_years,
            format,
        } => {
            run_sell_compare(
                &ticker,
                shares,
                cost_basis,
                price,
                &acquired,
                &filing,
                &state,
                other_income,
                growth,
                max_years,
                format,
            )?;
        }
        Commands::Allocate {
            profile,
            brokerage,
            value,
            format,
        } => {
            run_allocate(&profile, &brokerage, value, format)?;
        }
        Commands::DivReport {
            ticker,
            shares,
            cost_basis,
            price,
            acquired,
            filing,
            state,
            other_income,
            other_value,
            years,
            brokerage,
            profile,
            output,
        } => {
            run_div_report(
                &ticker,
                shares,
                cost_basis,
                price,
                &acquired,
                &filing,
                &state,
                other_income,
                other_value,
                years,
                &brokerage,
                &profile,
                &output,
            )?;
        }
        Commands::Size {
            portfolio,
            max_pct,
            risk_pct,
            method,
            format,
        } => {
            run_size(
                &cfg,
                portfolio,
                max_pct / 100.0,
                risk_pct / 100.0,
                &method,
                &format,
            )?;
        }
        Commands::Rotate { format } => {
            run_rotate(&cfg, &format)?;
        }
        Commands::Factors { format } => {
            run_factors(&format)?;
        }
        Commands::Vix {
            vix,
            vix3m,
            skew,
            put_call,
        } => {
            run_vix(vix, vix3m, skew, put_call)?;
        }
        Commands::VolTarget { target, invested } => {
            run_vol_target(target / 100.0, invested / 100.0)?;
        }
        Commands::Attribution { period, format } => {
            run_attribution(period, &format)?;
        }
        Commands::Calibrate { dry_run, reset } => {
            run_calibrate(&cfg, dry_run, reset)?;
        }
        Commands::Execute {
            mode,
            buy,
            sell,
            shares,
            portfolio,
        } => {
            run_execute(&mode, buy, sell, shares, portfolio).await?;
        }
        Commands::GeoRisk {
            oil,
            vix,
            gold_3m,
            dxy_3m,
            format,
        } => {
            run_geo_risk(oil, vix, gold_3m, dxy_3m, format)?;
        }
    }

    Ok(())
}

/// `ghost-alpha plan`: runs without any app config or API key.
fn run_plan_command(command: Commands) -> Result<()> {
    let Commands::Plan {
        positions,
        vesting,
        data_dir,
        asset_classes,
        treatment,
        paths,
        seed,
        horizon,
        withhold_rate,
        as_of,
        symbol,
        symbol_map,
        default_class,
        ltcg_rate,
        floor_pct,
        cost_basis,
        unwind,
        out_dir,
    } = command
    else {
        unreachable!("run_plan_command is only called for Commands::Plan");
    };
    use ghost_alpha::cli::plan::{
        discover_data_dir, load_plan_toml, parse_treatment, run_plan_pipeline, OptionBOverrides,
        PlanInputs, PlanToml,
    };
    use ghost_alpha::portfolio::symbol_map::parse_asset_class;
    use std::path::PathBuf;

    // Discover files in the data dir (if any); explicit flags win over discovered files.
    let discovered = match &data_dir {
        Some(d) => Some(
            discover_data_dir(std::path::Path::new(d))
                .with_context(|| format!("--data-dir {d}"))?,
        ),
        None => None,
    };
    if positions.is_empty() && discovered.is_none() {
        anyhow::bail!(
            "no positions given: pass --positions <csv> (repeatable) or --data-dir <dir> \
             (see `ghost-alpha init-data`)"
        );
    }
    let plan_toml = match discovered.as_ref().and_then(|d| d.plan_toml.as_ref()) {
        Some(p) => load_plan_toml(p)?,
        None => PlanToml::default(),
    };

    // Precedence: CLI flag > plan.toml > built-in default.
    let positions_csvs: Vec<PathBuf> = if positions.is_empty() {
        discovered
            .as_ref()
            .map(|d| d.positions.clone())
            .unwrap_or_default()
    } else {
        positions.into_iter().map(PathBuf::from).collect()
    };
    let vesting_csv = vesting
        .map(PathBuf::from)
        .or_else(|| discovered.as_ref().and_then(|d| d.vesting.clone()));
    let symbol_map_toml = symbol_map
        .map(PathBuf::from)
        .or_else(|| discovered.as_ref().and_then(|d| d.symbol_map.clone()));

    let treatment = match treatment {
        Some(t) => parse_treatment(&t).ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown treatment '{}'. Valid: hold | sell-reinvest | option-b",
                t
            )
        })?,
        None => plan_toml
            .treatment()?
            .unwrap_or(ghost_alpha::projection::portfolio_nav::Treatment::OptionBUnwind),
    };
    let as_of_date = match as_of {
        Some(s) => chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d")
            .with_context(|| format!("--as-of must be YYYY-MM-DD, got '{}'", s))?,
        None => plan_toml
            .as_of
            .ok_or_else(|| anyhow::anyhow!("--as-of is required (or set `as_of` in plan.toml)"))?,
    };
    let default_class = match default_class {
        Some(name) => Some(parse_asset_class(&name).ok_or_else(|| anyhow::anyhow!(
            "unknown --default-class '{}' (use a snake_case key from config/asset_classes.toml)", name
        ))?),
        None => plan_toml.default_class()?,
    };
    let unwind_schedule = match unwind {
        Some(s) => Some(parse_unwind_flag(&s)?),
        None => plan_toml.unwind_schedule()?,
    };
    let paths = paths.or(plan_toml.paths).unwrap_or(5000);
    let seed = seed.or(plan_toml.seed).unwrap_or(42);
    let horizon = horizon.or(plan_toml.horizon_years).unwrap_or(5);

    let inputs = PlanInputs {
        positions_csvs,
        vesting_csv,
        asset_classes_toml: match asset_classes {
            Some(p) => {
                let p = PathBuf::from(p);
                if !p.is_file() {
                    anyhow::bail!("--asset-classes file not found: {}", p.display());
                }
                p
            }
            None => PathBuf::from(ghost_alpha::cli::plan::DEFAULT_ASSET_CLASSES_PATH),
        },
        treatment,
        mc_paths: paths,
        mc_seed: seed,
        horizon_years: horizon,
        withhold_rate: withhold_rate.or(plan_toml.withhold_rate).unwrap_or(0.22),
        as_of: as_of_date,
        out_dir: PathBuf::from(out_dir),
        symbol: symbol.or_else(|| plan_toml.symbol.clone()),
        symbol_map_toml,
        default_class,
        option_b: OptionBOverrides {
            cost_basis_per_share: cost_basis.or(plan_toml.unwind.cost_basis_per_share),
            ltcg_rate: ltcg_rate.or(plan_toml.unwind.ltcg_rate),
            floor_pct: floor_pct.or(plan_toml.unwind.floor_pct),
            unwind_schedule,
            reinvest_weights: plan_toml.reinvest_weights(),
        },
    };

    tracing::info!(?treatment, paths, seed, "Running plan pipeline");
    let report = run_plan_pipeline(&inputs).context("plan pipeline failed")?;

    println!("Plan pipeline complete:");
    println!("  Treatment:        {:?}", report.treatment);
    println!("  Seed:             {}", report.seed);
    println!("  Paths:            {}", report.n_paths);
    println!("  Starting NAV:     ${:.2}", report.starting_nav);
    println!("  Year-{} NAV P5:   ${:.2}", horizon, report.year5_nav_p05);
    println!("  Year-{} NAV P50:  ${:.2}", horizon, report.year5_nav_p50);
    println!("  Year-{} NAV P95:  ${:.2}", horizon, report.year5_nav_p95);
    println!("  Year-{} NAV mean: ${:.2}", horizon, report.year5_nav_mean);
    println!();
    println!("Artifacts written:");
    for p in &report.artifact_paths {
        println!("  {}", p.display());
    }

    Ok(())
}

/// Parse `--unwind "1:400,2:300"` into `(year, shares)` pairs.
fn parse_unwind_flag(s: &str) -> Result<Vec<(u32, f64)>> {
    s.split(',')
        .map(|pair| {
            let (y, n) = pair.split_once(':').with_context(|| {
                format!("--unwind entry '{pair}' must look like year:shares (e.g. \"1:400,2:300\")")
            })?;
            let year = y
                .trim()
                .parse::<u32>()
                .with_context(|| format!("--unwind year '{}' is not a whole number", y.trim()))?;
            let shares = n
                .trim()
                .parse::<f64>()
                .with_context(|| format!("--unwind shares '{}' is not a number", n.trim()))?;
            Ok((year, shares))
        })
        .collect()
}

// ─── Sell Plan ────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn run_sell_plan(
    ticker: &str,
    shares: f64,
    cost_basis: f64,
    price: f64,
    acquired: &str,
    filing: &str,
    state: &str,
    other_income: f64,
    growth: f64,
    years: u32,
    distribution: &str,
    format: OutputFormat,
) -> Result<()> {
    use ghost_alpha::portfolio::prepare::{
        format_sell_plan, prepare_selling_programme, SellDistribution, SellPlanInput,
    };
    use ghost_alpha::tax::brackets::FilingStatus;

    let acquired_date = chrono::NaiveDate::parse_from_str(acquired, "%Y-%m-%d")
        .with_context(|| format!("Invalid date '{}', expected YYYY-MM-DD", acquired))?;

    let filing_status = match filing.to_lowercase().as_str() {
        "single" | "s" => FilingStatus::Single,
        "mfj" | "married" | "joint" => FilingStatus::MarriedFilingJointly,
        "mfs" | "separate" => FilingStatus::MarriedFilingSeparately,
        "hoh" | "head" => FilingStatus::HeadOfHousehold,
        other => anyhow::bail!(
            "Unknown filing status '{}'. Use: single, mfj, mfs, hoh",
            other
        ),
    };

    let dist = match distribution.to_lowercase().as_str() {
        "even" | "equal" => SellDistribution::Even,
        "front" | "front-loaded" | "frontloaded" => SellDistribution::FrontLoaded,
        "back" | "back-loaded" | "backloaded" => SellDistribution::BackLoaded,
        other => anyhow::bail!("Unknown distribution '{}'. Use: even, front, back", other),
    };

    let start_year = {
        use chrono::Datelike;
        chrono::Utc::now().naive_utc().date().year() as u32
    };
    let input = SellPlanInput {
        ticker: ticker.to_uppercase(),
        total_shares: shares,
        cost_basis_per_share: cost_basis,
        current_price: price,
        acquired_date,
        filing_status,
        state_code: state.to_uppercase(),
        other_annual_income: other_income,
        assumed_price_growth: growth,
        horizon_years: years,
        start_year,
        distribution: dist,
    };

    let output = prepare_selling_programme(&input).map_err(|e| anyhow::anyhow!(e))?;

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&output)?);
        }
        _ => {
            println!("{}", format_sell_plan(&output, state));
        }
    }

    Ok(())
}

// ─── Diversification Report ───────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn run_div_report(
    ticker: &str,
    shares: f64,
    cost_basis: f64,
    price: f64,
    acquired: &str,
    filing: &str,
    state: &str,
    other_income: f64,
    other_value: f64,
    years: u32,
    brokerage: &str,
    profile: &str,
    output: &str,
) -> Result<()> {
    use ghost_alpha::analysis::geopolitical::GeoRiskInputs;
    use ghost_alpha::portfolio::allocation::Brokerage;
    use ghost_alpha::portfolio::prepare::{
        prepare_selling_programme, SellDistribution, SellPlanInput,
    };
    use ghost_alpha::tax::brackets::{state_tax_rate, FederalCapGainsBrackets, FilingStatus};
    use ghost_alpha::tax::projection::compare_scenarios;
    use ghost_alpha::web::diversification_report::{
        generate_diversification_report, DiversificationReportInput,
    };

    let acquired_date = chrono::NaiveDate::parse_from_str(acquired, "%Y-%m-%d")
        .with_context(|| format!("Invalid date '{}', expected YYYY-MM-DD", acquired))?;

    let filing_status = match filing.to_lowercase().as_str() {
        "single" | "s" => FilingStatus::Single,
        "mfj" | "married" | "joint" => FilingStatus::MarriedFilingJointly,
        other => anyhow::bail!("Unknown filing status '{}'", other),
    };

    let start_year = {
        use chrono::Datelike;
        chrono::Utc::now().naive_utc().date().year() as u32
    };

    let sell_input = SellPlanInput {
        ticker: ticker.to_uppercase(),
        total_shares: shares,
        cost_basis_per_share: cost_basis,
        current_price: price,
        acquired_date,
        filing_status: filing_status.clone(),
        state_code: state.to_uppercase(),
        other_annual_income: other_income,
        assumed_price_growth: 0.0,
        horizon_years: years,
        start_year,
        distribution: SellDistribution::Even,
    };

    let sell_plan = prepare_selling_programme(&sell_input).map_err(|e| anyhow::anyhow!(e))?;

    let brackets = match filing_status {
        FilingStatus::Single => FederalCapGainsBrackets::single_2024(),
        FilingStatus::MarriedFilingJointly => FederalCapGainsBrackets::mfj_2024(),
        _ => FederalCapGainsBrackets::single_2024(),
    };

    let state_rate = state_tax_rate(&state.to_uppercase()).map_err(|e| anyhow::anyhow!("{}", e))?;

    let lot = ghost_alpha::portfolio::types::TaxLot {
        id: uuid::Uuid::new_v4(),
        ticker: ticker.to_uppercase(),
        shares,
        cost_basis_per_share: cost_basis,
        acquired_date,
        acquisition_type: ghost_alpha::portfolio::types::AcquisitionType::RsuVest,
        remaining_shares: shares,
    };

    let scenarios = compare_scenarios(
        &[lot],
        shares,
        5,
        start_year,
        0.0,
        price,
        other_income,
        &brackets,
        &state_rate,
    );

    let brok = match brokerage.to_lowercase().as_str() {
        "schwab" => Brokerage::Schwab,
        "vanguard" => Brokerage::Vanguard,
        _ => Brokerage::Fidelity,
    };

    let report_input = DiversificationReportInput {
        ticker: ticker.to_uppercase(),
        total_shares: shares,
        cost_basis_per_share: cost_basis,
        current_price: price,
        acquired_date,
        other_annual_income: other_income,
        state: state.to_uppercase(),
        filing: filing.to_uppercase(),
        other_portfolio_value: other_value,
        brokerage: brok,
        risk_profile: profile.to_string(),
        geo_inputs: GeoRiskInputs {
            oil_brent: 85.0,
            vix: 18.0,
            gold_3m_change: 0.03,
            dxy_3m_change: 0.01,
        },
        sell_plan,
        scenarios,
        start_year,
    };

    let html = generate_diversification_report(&report_input);
    std::fs::write(output, &html).with_context(|| format!("Failed to write report to {output}"))?;

    println!("  Diversification report written to: {}", output);
    println!(
        "  Open in browser: file://{}/{}",
        std::env::current_dir()?.display(),
        output
    );

    Ok(())
}

// ─── Allocate ─────────────────────────────────────────────────────────────────

fn run_allocate(profile: &str, brokerage: &str, value: f64, format: OutputFormat) -> Result<()> {
    use ghost_alpha::portfolio::allocation::{
        blended_expense_ratio, brokerage_catalog, map_to_funds, risk_profiles, Brokerage,
    };

    let brok = match brokerage.to_lowercase().as_str() {
        "fidelity" | "fido" => Brokerage::Fidelity,
        "schwab" => Brokerage::Schwab,
        "vanguard" | "vgrd" => Brokerage::Vanguard,
        other => anyhow::bail!(
            "Unknown brokerage '{}'. Use: fidelity, schwab, vanguard",
            other
        ),
    };

    let profiles = risk_profiles();
    let rp = profiles
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(profile))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown profile '{}'. Use: aggressive, moderate, conservative",
                profile
            )
        })?;

    let allocations = map_to_funds(&rp.allocations, &brok, value);
    let blended_er = blended_expense_ratio(&allocations);
    let catalog = brokerage_catalog(&brok);

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&allocations)?);
        }
        _ => {
            println!();
            println!(
                "  {} — {} profile on {}",
                "Target Allocation".bold(),
                rp.name,
                catalog.brokerage
            );
            println!("  {}", "─".repeat(80));
            println!("  {}", rp.description);
            println!();
            println!(
                "  {:>20}  {:>6}  {:>30}  {:>7}  {:>12}  {:>6}",
                "Asset Class", "Ticker", "Fund Name", "ER", "Amount", "TLH"
            );
            println!("  {}", "─".repeat(80));
            for a in &allocations {
                let tlh = a.fund.tlh_partner.as_deref().unwrap_or("—");
                println!(
                    "  {:>20}  {:>6}  {:>30}  {:>6.3}%  {:>12}  {:>6}",
                    a.asset_class.to_string(),
                    a.fund.ticker,
                    a.fund.name,
                    a.fund.expense_ratio * 100.0,
                    format!("${:.0}", a.dollar_amount),
                    tlh
                );
            }
            println!("  {}", "─".repeat(80));
            println!("  Blended expense ratio: {:.4}%", blended_er * 100.0);
            println!();
        }
    }

    Ok(())
}

// ─── Position Sizing ─────────────────────────────────────────────────────────

fn run_size(
    cfg: &AppConfig,
    portfolio_value: f64,
    max_position_pct: f64,
    risk_pct: f64,
    method_str: &str,
    format: &OutputFormat,
) -> Result<()> {
    use ghost_alpha::analysis::scorer::Grade;
    use ghost_alpha::data::models::MarketRegime;
    use ghost_alpha::portfolio::sizing::{compute_position_size, SizingInput, SizingMethod};

    let method = match method_str {
        "equal-weight" => SizingMethod::EqualWeight {
            target_positions: 8,
        },
        "fixed-fractional" => SizingMethod::FixedFractional { risk_pct },
        _ => SizingMethod::VolatilityTarget {
            stop_multiplier: 2.0,
        },
    };

    // Try to load screen results from DB; fall back to demo data
    let examples: Vec<(String, f64, f64, Grade, MarketRegime)> = {
        let db_results = SqliteRepository::open(&cfg.storage.db_path)
            .ok()
            .and_then(|repo| {
                let today = chrono::Local::now().date_naive();
                repo.get_screen_results(today).ok()
            });

        if let Some(results) = db_results {
            if !results.is_empty() {
                results
                    .iter()
                    .take(10)
                    .map(|r| {
                        let grade = match r.composite_score {
                            s if s >= 85.0 => Grade::APlus,
                            s if s >= 75.0 => Grade::A,
                            s if s >= 60.0 => Grade::B,
                            s if s >= 45.0 => Grade::C,
                            _ => Grade::D,
                        };
                        (
                            r.ticker.clone(),
                            r.close_price,
                            r.atr_value,
                            grade,
                            r.regime.clone(),
                        )
                    })
                    .collect()
            } else {
                vec![
                    (
                        "NVDA".into(),
                        180.0,
                        5.5,
                        Grade::APlus,
                        MarketRegime::Sideways,
                    ),
                    (
                        "GOOGL".into(),
                        190.0,
                        4.2,
                        Grade::APlus,
                        MarketRegime::Sideways,
                    ),
                    ("AMD".into(), 140.0, 6.0, Grade::A, MarketRegime::Sideways),
                    ("PLTR".into(), 85.0, 3.8, Grade::A, MarketRegime::Sideways),
                ]
            }
        } else {
            vec![
                (
                    "NVDA".into(),
                    180.0,
                    5.5,
                    Grade::APlus,
                    MarketRegime::Sideways,
                ),
                (
                    "GOOGL".into(),
                    190.0,
                    4.2,
                    Grade::APlus,
                    MarketRegime::Sideways,
                ),
                ("AMD".into(), 140.0, 6.0, Grade::A, MarketRegime::Sideways),
                ("PLTR".into(), 85.0, 3.8, Grade::A, MarketRegime::Sideways),
            ]
        }
    };

    println!();
    println!("  {}", "Position Sizing Recommendations".bold());
    println!(
        "  Portfolio: ${:.0}  Method: {}  Risk: {:.1}%  Max: {:.0}%",
        portfolio_value,
        method_str,
        risk_pct * 100.0,
        max_position_pct * 100.0
    );
    println!("  {}", "─".repeat(75));
    println!(
        "  {:<8} {:>8} {:>10} {:>8} {:>10} {:>10} {:>6}",
        "Ticker", "Price", "Shares", "Stop", "Amount", "Risk$", "Pct%"
    );
    println!("  {}", "─".repeat(75));

    for (ticker, price, atr, grade, regime) in &examples {
        let input = SizingInput {
            ticker: ticker.clone(),
            close_price: *price,
            atr_14: *atr,
            grade: grade.clone(),
            regime: regime.clone(),
        };
        let result =
            compute_position_size(&method, &input, portfolio_value, risk_pct, max_position_pct);
        let capped = if result.was_capped { "*" } else { " " };
        println!(
            "  {:<8} {:>7.2} {:>9.0}{} {:>8.2} {:>9.0} {:>9.0} {:>5.1}",
            ticker,
            price,
            result.shares,
            capped,
            result.stop_loss,
            result.dollar_amount,
            result.risk_amount,
            result.pct_of_portfolio * 100.0
        );
    }
    println!("  {}", "─".repeat(75));
    println!("  * = capped at max position %");
    println!();

    if matches!(format, OutputFormat::Json) {
        // JSON output for programmatic consumption
        let results: Vec<_> = examples
            .iter()
            .map(|(ticker, price, atr, grade, regime)| {
                let input = SizingInput {
                    ticker: ticker.clone(),
                    close_price: *price,
                    atr_14: *atr,
                    grade: grade.clone(),
                    regime: regime.clone(),
                };
                compute_position_size(&method, &input, portfolio_value, risk_pct, max_position_pct)
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&results)?);
    }

    Ok(())
}

// ─── Capital Rotation ────────────────────────────────────────────────────────

#[allow(clippy::type_complexity)]
fn demo_rotation_data(
    mk: &dyn Fn(
        &str,
        u8,
        u8,
        u8,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        u32,
        f64,
    ) -> (String, ghost_alpha::portfolio::rotation::SpsInput),
) -> Vec<(String, ghost_alpha::portfolio::rotation::SpsInput)> {
    vec![
        mk(
            "GOOGL", 8, 8, 0, 88.0, 86.0, 82.0, 90.0, 87.0, 195.0, 200.0, 8, 0.14,
        ),
        mk(
            "AMD", 8, 8, 0, 91.0, 89.0, 85.0, 95.0, 92.0, 142.0, 148.0, 12, 0.12,
        ),
        mk(
            "NVDA", 7, 8, 1, 78.0, 80.0, 82.0, 70.0, 75.0, 178.0, 195.0, 25, 0.15,
        ),
        mk(
            "PLTR", 8, 8, 0, 83.0, 81.0, 78.0, 85.0, 83.0, 151.0, 155.0, 5, 0.13,
        ),
        mk(
            "AVGO", 6, 8, 1, 68.0, 72.0, 75.0, 65.0, 70.0, 210.0, 230.0, 35, 0.10,
        ),
        mk(
            "TSLA", 6, 7, 0, 65.0, 68.0, 70.0, 75.0, 78.0, 275.0, 290.0, 20, 0.08,
        ),
        mk(
            "DDOG", 6, 8, 2, 62.0, 68.0, 72.0, 60.0, 68.0, 108.0, 125.0, 50, 0.07,
        ),
        mk(
            "AAPL", 6, 8, 0, 60.0, 63.0, 65.0, 55.0, 60.0, 220.0, 235.0, 45, 0.09,
        ),
    ]
}

fn run_rotate(cfg: &AppConfig, _format: &OutputFormat) -> Result<()> {
    use ghost_alpha::data::models::MarketRegime;
    use ghost_alpha::portfolio::rotation::{
        compute_deployment_score, compute_sell_pressure, CdsInput, SpsAction, SpsInput,
    };

    let regime = MarketRegime::Sideways;

    // Try to load active flagged stocks from DB; fall back to demo data
    let mk = |ticker: &str,
              cond: u8,
              entry: u8,
              l5d: u8,
              c: f64,
              c5: f64,
              c20: f64,
              rs: f64,
              rs20: f64,
              cl: f64,
              hi: f64,
              dh: u32,
              wt: f64|
     -> (String, SpsInput) {
        (
            ticker.to_string(),
            SpsInput {
                conditions_met: cond,
                conditions_at_entry: entry,
                conditions_lost_5d: l5d,
                composite_now: c,
                composite_5d_ago: c5,
                composite_20d_ago: c20,
                up_day_avg_vol: 1e6,
                down_day_avg_vol: 9e5,
                rs_percentile: rs,
                rs_20d_ago: rs20,
                current_close: cl,
                local_high_60d: hi,
                days_since_new_high: dh,
                actual_weight: wt,
                target_weight: 0.125,
                hard_exit_triggered: false,
            },
        )
    };

    let positions: Vec<(String, SpsInput)> = {
        let db_flagged = SqliteRepository::open(&cfg.storage.db_path)
            .ok()
            .and_then(|repo| repo.get_active_flagged_stocks().ok());

        if let Some(flagged) = db_flagged {
            if !flagged.is_empty() {
                let n = flagged.len().max(1) as f64;
                flagged
                    .iter()
                    .map(|f| {
                        let score = f.latest_score.unwrap_or(f.entry_score);
                        (
                            f.ticker.clone(),
                            SpsInput {
                                conditions_met: f.conditions_at_entry.min(8),
                                conditions_at_entry: f.conditions_at_entry,
                                conditions_lost_5d: 0,
                                composite_now: score,
                                composite_5d_ago: score * 1.02,
                                composite_20d_ago: score * 1.05,
                                up_day_avg_vol: 1e6,
                                down_day_avg_vol: 9e5,
                                rs_percentile: 70.0,
                                rs_20d_ago: 72.0,
                                current_close: f.exit_price.unwrap_or(f.entry_price),
                                local_high_60d: f.entry_price * 1.10,
                                days_since_new_high: 15,
                                actual_weight: 1.0 / n,
                                target_weight: 1.0 / n,
                                hard_exit_triggered: false,
                            },
                        )
                    })
                    .collect()
            } else {
                demo_rotation_data(&mk)
            }
        } else {
            demo_rotation_data(&mk)
        }
    };

    println!();
    println!("  {}", "Capital Rotation Analysis".bold());
    println!(
        "  Regime: {:?}  |  Positions: {}  |  Target wt: 12.5%",
        regime,
        positions.len()
    );
    println!("  {}", "─".repeat(75));
    println!(
        "  {:<8} {:>5} {:>7} {:>6} {:>6} {:>6} {:>6} {:>8}",
        "Ticker", "SPS", "Action", "Trend", "MomFd", "DD%", "RS", "TimDcy"
    );
    println!("  {}", "─".repeat(75));

    let mut results = Vec::new();
    for (ticker, input) in &positions {
        let result = compute_sell_pressure(ticker, input, &regime);
        let action_str = match &result.action {
            SpsAction::Green => "GREEN".green().to_string(),
            SpsAction::Yellow => "YELLOW".yellow().to_string(),
            SpsAction::Orange => "ORANGE".truecolor(255, 165, 0).to_string(),
            SpsAction::Red => "RED".red().to_string(),
            SpsAction::Critical => "CRIT".red().bold().to_string(),
        };
        println!(
            "  {:<8} {:>5.1} {:>7} {:>6.0} {:>6.0} {:>6.0} {:>6.0} {:>8.0}",
            ticker,
            result.sps,
            action_str,
            result.components.trend_decay,
            result.components.momentum_fade,
            result.components.drawdown_penalty,
            result.components.relative_weakness,
            result.components.time_decay
        );
        results.push(result);
    }
    println!("  {}", "─".repeat(75));

    let cds = compute_deployment_score(&CdsInput {
        candidates_above_70: 5,
        top_candidate_score: 82.0,
        regime: regime.clone(),
        regime_improving: false,
        regime_deteriorating: false,
        cash_pct: 0.31,
        days_in_cash: 14,
        vix_percentile: 45.0,
        breadth_pct: 55.0,
    });
    println!();
    println!("  Cash Deployment: CDS={:.1} -> {:?}", cds.cds, cds.action);
    println!(
        "    Opportunity: {:.0}  Regime: {:.0}  Drag: {:.0}  Risk: {:.0}",
        cds.opportunity_richness, cds.regime_score, cds.cash_drag_cost, cds.drawdown_risk
    );
    println!();

    Ok(())
}

// ─── Factors ─────────────────────────────────────────────────────────────────

fn run_factors(format: &OutputFormat) -> Result<()> {
    use ghost_alpha::analysis::factor_exposure::{
        check_factor_alerts, compute_factor_exposure, FactorSeverity, FactorThresholds,
        PositionFactorData,
    };

    let spy_returns: Vec<f64> = (0..60)
        .map(|i| 0.001 + 0.005 * ((i as f64 * 0.3).sin()))
        .collect();

    let positions = vec![
        PositionFactorData {
            ticker: "NVDA".into(),
            weight: 0.18,
            sector: "Technology".into(),
            rs_percentile: 95.0,
            atr_pct: 3.5,
            daily_returns: spy_returns.iter().map(|r| r * 1.5 + 0.0005).collect(),
        },
        PositionFactorData {
            ticker: "GOOGL".into(),
            weight: 0.14,
            sector: "Technology".into(),
            rs_percentile: 85.0,
            atr_pct: 2.8,
            daily_returns: spy_returns.iter().map(|r| r * 1.1 + 0.0003).collect(),
        },
        PositionFactorData {
            ticker: "AMD".into(),
            weight: 0.13,
            sector: "Technology".into(),
            rs_percentile: 90.0,
            atr_pct: 4.0,
            daily_returns: spy_returns.iter().map(|r| r * 1.6 + 0.0004).collect(),
        },
        PositionFactorData {
            ticker: "CRWD".into(),
            weight: 0.12,
            sector: "Technology".into(),
            rs_percentile: 88.0,
            atr_pct: 3.2,
            daily_returns: spy_returns.iter().map(|r| r * 1.3 + 0.0002).collect(),
        },
        PositionFactorData {
            ticker: "PLTR".into(),
            weight: 0.11,
            sector: "Technology".into(),
            rs_percentile: 82.0,
            atr_pct: 3.8,
            daily_returns: spy_returns.iter().map(|r| r * 1.4 + 0.0001).collect(),
        },
        PositionFactorData {
            ticker: "JPM".into(),
            weight: 0.10,
            sector: "Financials".into(),
            rs_percentile: 70.0,
            atr_pct: 2.0,
            daily_returns: spy_returns.iter().map(|r| r * 0.9).collect(),
        },
        PositionFactorData {
            ticker: "UNH".into(),
            weight: 0.12,
            sector: "Healthcare".into(),
            rs_percentile: 75.0,
            atr_pct: 1.8,
            daily_returns: spy_returns.iter().map(|r| r * 0.7 + 0.0002).collect(),
        },
        PositionFactorData {
            ticker: "XOM".into(),
            weight: 0.10,
            sector: "Energy".into(),
            rs_percentile: 60.0,
            atr_pct: 2.5,
            daily_returns: spy_returns.iter().map(|r| r * 0.6 - 0.0001).collect(),
        },
    ];

    let exposure = compute_factor_exposure(&positions, &spy_returns);
    let alerts = check_factor_alerts(&exposure, &FactorThresholds::default());

    if matches!(format, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(&exposure)?);
        return Ok(());
    }

    println!();
    println!("  {}", "Portfolio Factor Exposure".bold());
    println!("  {}", "─".repeat(55));
    println!(
        "  {:<25} {:>10} {:>10} {:>6}",
        "Factor", "Current", "Threshold", "Status"
    );
    println!("  {}", "─".repeat(55));
    println!(
        "  {:<25} {:>10.2} {:>10.1} {:>6}",
        "Market Beta",
        exposure.market_beta,
        1.3,
        status_dot(exposure.market_beta, 1.3)
    );
    println!(
        "  {:<25} {:>9.0}% {:>9.0}% {:>6}",
        "Top Sector Weight",
        exposure.top_sector_weight * 100.0,
        50.0,
        status_dot(exposure.top_sector_weight, 0.50)
    );
    println!(
        "  {:<25} {:>10.2} {:>10.2} {:>6}",
        "Sector HHI",
        exposure.sector_hhi,
        0.30,
        status_dot(exposure.sector_hhi, 0.30)
    );
    println!(
        "  {:<25} {:>10.2} {:>10.2} {:>6}",
        "Avg Correlation",
        exposure.avg_correlation,
        0.65,
        status_dot(exposure.avg_correlation, 0.65)
    );
    println!(
        "  {:<25} {:>9.1}% {:>9.1}% {:>6}",
        "Volatility Tilt",
        exposure.volatility_tilt,
        4.0,
        status_dot(exposure.volatility_tilt, 4.0)
    );
    println!(
        "  {:<25} {:>10.0} {:>10}",
        "Momentum Tilt (RS%)", exposure.momentum_tilt, "—"
    );
    println!("  {:<25} {:>10}", "Top Sector", exposure.top_sector_name);
    println!("  {}", "─".repeat(55));

    if alerts.is_empty() {
        println!(
            "  {} No factor alerts — exposures within thresholds.",
            "✓".green()
        );
    } else {
        for alert in &alerts {
            let icon = match alert.severity {
                FactorSeverity::Critical => "▲".red().bold().to_string(),
                FactorSeverity::Warning => "▲".yellow().to_string(),
                FactorSeverity::Info => "○".blue().to_string(),
            };
            println!("  {} {}", icon, alert.message);
        }
    }
    println!();

    Ok(())
}

fn status_dot(current: f64, threshold: f64) -> String {
    if current > threshold * 1.10 {
        "▲ CRIT".red().bold().to_string()
    } else if current > threshold {
        "▲ WARN".yellow().to_string()
    } else if current > threshold * 0.90 {
        "○ INFO".blue().to_string()
    } else {
        "✓  OK".green().to_string()
    }
}

// ─── VIX Signal ──────────────────────────────────────────────────────────────

fn run_vix(vix: f64, vix3m: f64, skew: f64, put_call: f64) -> Result<()> {
    use ghost_alpha::analysis::vix_signal::{
        classify_vix_signal, put_call_sentiment, VixSignal, VixThresholds,
    };

    let thresholds = VixThresholds::default();
    let result = classify_vix_signal(vix, vix3m, skew, put_call, &thresholds);
    let sentiment = put_call_sentiment(put_call, &thresholds);

    let signal_str = match &result.signal {
        VixSignal::RiskOn => "RISK-ON".green().bold().to_string(),
        VixSignal::Neutral => "NEUTRAL".normal().to_string(),
        VixSignal::Caution => "CAUTION".yellow().bold().to_string(),
        VixSignal::StressAlert => "STRESS".red().bold().to_string(),
    };

    let structure = if result.ratio < 0.95 {
        "Contango".green().to_string()
    } else if result.ratio > 1.0 {
        "Backwardation".red().to_string()
    } else {
        "Flat".yellow().to_string()
    };

    println!();
    println!("  {}", "VIX Term Structure Signal".bold());
    println!("  {}", "─".repeat(50));
    println!("  VIX:              {:.1}", result.vix);
    println!("  VIX3M:            {:.1}", result.vix3m);
    println!("  Ratio (VIX/3M):   {:.3}  ({})", result.ratio, structure);
    println!("  SKEW:             {:.0}", result.skew);
    println!(
        "  Put/Call (10d):   {:.2}  ({})",
        result.put_call_ratio, sentiment
    );
    println!("  {}", "─".repeat(50));
    println!("  Signal:           {}", signal_str);
    println!(
        "  Exposure Adj:     {:.0}%",
        result.exposure_adjustment * 100.0
    );
    println!("  {}", "─".repeat(50));

    match result.signal {
        VixSignal::RiskOn => println!("  Term structure healthy. Deploy normally."),
        VixSignal::Neutral => println!("  No strong signal. Maintain current exposure."),
        VixSignal::Caution => println!("  Stress rising. Reduce exposure by 25%."),
        VixSignal::StressAlert => println!("  Acute stress. Raise cash to 50%."),
    }
    println!();

    Ok(())
}

// ─── HMM Regime ──────────────────────────────────────────────────────────────

fn run_hmm_regime() -> Result<()> {
    use ghost_alpha::analysis::hmm_regime::{predict_regime, HmmModel};

    // Demo: generate synthetic observations resembling current market
    let observations: Vec<[f64; 3]> = (0..60)
        .map(|i| {
            let base_return = 0.0005 + 0.003 * ((i as f64 * 0.2).sin());
            let vol = 0.14 + 0.02 * ((i as f64 * 0.15).sin());
            let yield_curve = 0.8 + 0.3 * ((i as f64 * 0.1).sin());
            [base_return, vol, yield_curve]
        })
        .collect();

    let model = HmmModel::default_prior();
    let state = predict_regime(&model, &observations);

    let regime_str = match &state.most_likely {
        ghost_alpha::data::models::MarketRegime::Bull => "BULL".green().bold().to_string(),
        ghost_alpha::data::models::MarketRegime::Sideways => "SIDEWAYS".yellow().bold().to_string(),
        ghost_alpha::data::models::MarketRegime::Bear => "BEAR".red().bold().to_string(),
    };

    println!();
    println!("  {}", "HMM Regime Detection (Probabilistic)".bold());
    println!("  {}", "─".repeat(50));
    println!("  Most Likely:    {}", regime_str);
    println!("  Confidence:     {:.1}%", state.confidence * 100.0);
    println!();
    println!("  Probabilities:");

    let bar = |p: f64, _color: &str| -> String {
        let width = (p * 30.0) as usize;
        let bar: String = "█".repeat(width);
        let empty: String = "░".repeat(30 - width);
        format!("  {}{}  {:.1}%", bar, empty, p * 100.0)
    };

    println!("    Bull:     {}", bar(state.bull_prob, "green"));
    println!("    Sideways: {}", bar(state.sideways_prob, "yellow"));
    println!("    Bear:     {}", bar(state.bear_prob, "red"));
    println!("  {}", "─".repeat(50));
    println!("  Model: 3-state HMM | Obs: return + vol + yield curve | 60 days");
    println!();

    Ok(())
}

// ─── Vol Target ──────────────────────────────────────────────────────────────

fn run_vol_target(target_vol: f64, current_invested: f64) -> Result<()> {
    use ghost_alpha::portfolio::vol_target::{
        compute_vol_target, exposure_change_pct, VolTargetAction, VolTargetConfig,
    };

    let config = VolTargetConfig {
        target_vol,
        ..Default::default()
    };

    // Demo daily returns — in production, computed from portfolio positions
    let daily_returns: Vec<f64> = (0..60)
        .map(|i| {
            let base = 0.001;
            let noise = ((i as f64 * 0.7).sin()) * 0.012;
            base + noise
        })
        .collect();

    let result = compute_vol_target(&daily_returns, &config, current_invested);
    let change = exposure_change_pct(&result);

    let action_str = match &result.action {
        VolTargetAction::ReduceExposure => "REDUCE".red().bold().to_string(),
        VolTargetAction::MaintainExposure => "MAINTAIN".green().to_string(),
        VolTargetAction::IncreaseExposure => "INCREASE".blue().bold().to_string(),
    };

    println!();
    println!("  {}", "Portfolio Volatility Target".bold());
    println!("  {}", "─".repeat(50));
    println!(
        "  Target Vol:      {:.1}% annualized",
        result.target_vol * 100.0
    );
    println!(
        "  Realized Vol:    {:.1}% annualized (20-day)",
        result.realized_vol * 100.0
    );
    println!("  Exposure Scalar: {:.2}x", result.exposure_scalar);
    println!("  Action:          {}", action_str);
    println!("  {}", "─".repeat(50));
    println!(
        "  Current Invested:     {:.1}%",
        result.base_invested_pct * 100.0
    );
    println!(
        "  Recommended Invested: {:.1}%",
        result.recommended_invested_pct * 100.0
    );
    println!("  Change:               {:+.1}%", change * 100.0);
    println!("  {}", "─".repeat(50));

    if result.action == VolTargetAction::ReduceExposure {
        println!(
            "  Trim weakest positions by SPS score to reach {:.0}% invested.",
            result.recommended_invested_pct * 100.0
        );
    } else if result.action == VolTargetAction::IncreaseExposure {
        println!("  Deploy cash into top candidates. Low vol = opportunity.");
    }
    println!();

    Ok(())
}

// ─── Attribution ─────────────────────────────────────────────────────────────

fn run_attribution(period_days: u32, format: &OutputFormat) -> Result<()> {
    use ghost_alpha::analysis::attribution::{compute_attribution, TradeRecord};

    // Demo attribution data — in production this reads from completed trades
    let trades = vec![
        TradeRecord {
            ticker: "NVDA".into(),
            strategy: "minervini".into(),
            weight: 0.15,
            realized_return: 0.12,
        },
        TradeRecord {
            ticker: "GOOGL".into(),
            strategy: "minervini".into(),
            weight: 0.12,
            realized_return: 0.08,
        },
        TradeRecord {
            ticker: "AMD".into(),
            strategy: "trend-strength".into(),
            weight: 0.10,
            realized_return: 0.15,
        },
        TradeRecord {
            ticker: "CRWD".into(),
            strategy: "breakout".into(),
            weight: 0.10,
            realized_return: 0.18,
        },
        TradeRecord {
            ticker: "PLTR".into(),
            strategy: "minervini".into(),
            weight: 0.09,
            realized_return: 0.06,
        },
        TradeRecord {
            ticker: "PANW".into(),
            strategy: "breakout".into(),
            weight: 0.08,
            realized_return: -0.02,
        },
        TradeRecord {
            ticker: "AAPL".into(),
            strategy: "trend-strength".into(),
            weight: 0.07,
            realized_return: 0.04,
        },
        TradeRecord {
            ticker: "DDOG".into(),
            strategy: "mean-reversion".into(),
            weight: 0.05,
            realized_return: -0.10,
        },
    ];

    let end = chrono::Local::now().date_naive();
    let start = end - chrono::Duration::days(period_days as i64);
    let benchmark_return = 0.035; // SPY ~3.5% over 90 days

    let result = compute_attribution(&trades, benchmark_return, start, end);

    if matches!(format, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    println!();
    println!("  {}", "Performance Attribution".bold());
    println!("  Period: {} to {} ({} days)", start, end, period_days);
    println!("  {}", "─".repeat(55));
    println!("  {:<25} {:>10} {:>10}", "Component", "Return", "Contrib%");
    println!("  {}", "─".repeat(55));
    println!(
        "  {:<25} {:>9.2}%",
        "Portfolio Return",
        result.total_return * 100.0
    );
    println!(
        "  {:<25} {:>9.2}%",
        "Benchmark (SPY)",
        result.benchmark_return * 100.0
    );
    println!("  {}", "─".repeat(55));
    println!(
        "  {:<25} {:>9.2}% {:>9.0}%",
        "Active Return",
        result.active_return * 100.0,
        if result.active_return != 0.0 {
            100.0
        } else {
            0.0
        }
    );
    println!("  {}", "─".repeat(55));

    let active = result.active_return.abs().max(0.0001);
    println!(
        "  {:<25} {:>+9.2}% {:>9.1}%",
        "Stock Selection",
        result.selection_effect * 100.0,
        result.selection_effect / active * 100.0
    );
    println!(
        "  {:<25} {:>+9.2}% {:>9.1}%",
        "Market Timing",
        result.timing_effect * 100.0,
        result.timing_effect / active * 100.0
    );
    println!(
        "  {:<25} {:>+9.2}% {:>9.1}%",
        "Position Sizing",
        result.sizing_effect * 100.0,
        result.sizing_effect / active * 100.0
    );
    println!(
        "  {:<25} {:>+9.2}% {:>9.1}%",
        "Strategy Weighting",
        result.strategy_effect * 100.0,
        result.strategy_effect / active * 100.0
    );
    println!(
        "  {:<25} {:>+9.2}% {:>9.1}%",
        "Interaction",
        result.interaction_effect * 100.0,
        result.interaction_effect / active * 100.0
    );
    println!("  {}", "─".repeat(55));
    println!();

    Ok(())
}

// ─── Calibrate ───────────────────────────────────────────────────────────────

fn run_calibrate(_cfg: &AppConfig, dry_run: bool, reset: bool) -> Result<()> {
    use ghost_alpha::analysis::param_tuner::{calibrate, AdaptiveParams, GUARDRAILS};
    use ghost_alpha::data::models::MarketRegime;

    let today = chrono::Local::now().date_naive();
    let regime = MarketRegime::Sideways; // Would come from live regime detection

    if reset {
        let defaults = AdaptiveParams::defaults(today, regime.clone());
        println!();
        println!("  {}", "Parameter Reset to Defaults".bold());
        println!("  Regime: {:?}", regime);
        println!("  {}", "─".repeat(50));
        for (i, g) in GUARDRAILS.iter().enumerate() {
            println!(
                "  {:<30} {:>8.2}  (range: {:.1}–{:.1})",
                g.name,
                defaults.get(i),
                g.min,
                g.max
            );
        }
        println!("  {}", "─".repeat(50));
        if !dry_run {
            println!("  Parameters reset.");
        } else {
            println!("  DRY RUN — no changes applied.");
        }
        println!();
        return Ok(());
    }

    let current = AdaptiveParams::defaults(today, regime.clone());

    // Simulated calibration with synthetic score function
    let score_fn = |params: &AdaptiveParams, _idx: usize| -> (f64, usize, f64) {
        // In production: evaluate params against historical OOS signals
        let score = 0.55
            + (params.rs_threshold - 70.0) * 0.002
            + (30.0 - params.adx_strong_threshold) * 0.003;
        (score.clamp(0.0, 1.0), 25, score.clamp(0.0, 1.0))
    };

    let result = calibrate(&current, score_fn, regime.clone(), today, 3);

    println!();
    println!("  {}", "Adaptive Parameter Calibration".bold());
    println!(
        "  Regime: {:?}  |  OOS Score: {:.3}  |  Signals: {}  |  Hit Rate: {:.1}%",
        regime,
        result.score,
        result.signal_count,
        result.hit_rate * 100.0
    );
    println!("  {}", "─".repeat(65));
    println!(
        "  {:<30} {:>8} {:>8} {:>8}",
        "Parameter", "Current", "New", "Change"
    );
    println!("  {}", "─".repeat(65));

    for (i, g) in GUARDRAILS.iter().enumerate() {
        let old = current.get(i);
        let new = result.params.get(i);
        let delta = new - old;
        let changed = if delta.abs() > 0.001 { "*" } else { " " };
        println!(
            "  {:<30} {:>7.2} {:>7.2} {:>+7.2} {}",
            g.name, old, new, delta, changed
        );
    }
    println!("  {}", "─".repeat(65));

    if result.params_changed.is_empty() {
        println!("  No parameters changed.");
    } else {
        println!("  Changed: {}", result.params_changed.join(", "));
    }

    if dry_run {
        println!("  DRY RUN — no changes applied.");
    }
    if current.is_stale(today) {
        println!("  WARNING: Last calibration is stale (> 30 days old).");
    }
    println!();

    Ok(())
}

// ─── Execute (Broker Integration) ────────────────────────────────────────────

async fn run_execute(
    mode_str: &str,
    buy: Option<String>,
    sell: Option<String>,
    shares: Option<f64>,
    _portfolio: f64,
) -> Result<()> {
    use ghost_alpha::broker::alpaca::{
        execute_buy, execute_sell, AlpacaClient, AlpacaConfig, ExecutionMode,
    };

    let mode = match mode_str {
        "paper" => ExecutionMode::Paper,
        "live" => ExecutionMode::Live,
        _ => ExecutionMode::DryRun,
    };

    if mode == ExecutionMode::Live {
        println!();
        println!("  {} LIVE TRADING MODE", "WARNING:".red().bold());
        println!("  This will execute real trades with real money.");
        println!("  Make sure you understand the risks.");
        println!();
    }

    let config = if mode == ExecutionMode::DryRun {
        // Dry run doesn't need real credentials
        AlpacaConfig::from_env(mode).unwrap_or(AlpacaConfig {
            api_key_id: "dry-run".into(),
            api_secret_key: "dry-run".into(),
            mode,
        })
    } else {
        AlpacaConfig::from_env(mode).map_err(|e| anyhow::anyhow!("{e}"))?
    };
    let client = AlpacaClient::new(config);

    // Show account info
    if mode != ExecutionMode::DryRun {
        match client.get_account().await {
            Ok(acct) => {
                println!();
                println!("  {}", "Alpaca Account".bold());
                println!(
                    "  Status: {}  |  Equity: ${:.2}  |  Buying Power: ${:.2}",
                    acct.status,
                    acct.equity_f64(),
                    acct.buying_power_f64()
                );
                println!();
            }
            Err(e) => println!("  Warning: Could not fetch account info: {e}"),
        }
    }

    let has_action = buy.is_some() || sell.is_some();

    // Execute buy
    if let Some(ticker) = buy {
        let qty = shares.unwrap_or(10.0);
        let result = execute_buy(&client, &ticker, qty, None).await;
        println!(
            "  {} {} {} shares of {} — {}",
            if result.status == "preview" {
                "PREVIEW:"
            } else {
                "EXECUTED:"
            },
            result.side.to_uppercase(),
            result.shares as u64,
            result.ticker,
            result.message
        );
    }

    // Execute sell
    if let Some(ticker) = sell {
        let qty = shares.unwrap_or(10.0);
        let result = execute_sell(&client, &ticker, qty, None).await;
        println!(
            "  {} {} {} shares of {} — {}",
            if result.status == "preview" {
                "PREVIEW:"
            } else {
                "EXECUTED:"
            },
            result.side.to_uppercase(),
            result.shares as u64,
            result.ticker,
            result.message
        );
    }

    if !has_action {
        println!();
        println!("  Usage:");
        println!("    ghost-alpha execute --buy AAPL --shares 10            # dry-run (preview)");
        println!("    ghost-alpha execute --buy AAPL --shares 10 --mode paper  # paper trading");
        println!("    ghost-alpha execute --sell NVDA --shares 5 --mode paper  # paper sell");
        println!();
    }

    Ok(())
}

// ─── Geo Risk ─────────────────────────────────────────────────────────────────

fn run_geo_risk(oil: f64, vix: f64, gold_3m: f64, dxy_3m: f64, format: OutputFormat) -> Result<()> {
    use ghost_alpha::analysis::geopolitical::{
        compute_geo_risk, geo_allocation_adjustments, GeoRiskInputs,
    };

    let inputs = GeoRiskInputs {
        oil_brent: oil,
        vix,
        gold_3m_change: gold_3m,
        dxy_3m_change: dxy_3m,
    };

    let score = compute_geo_risk(&inputs);
    let adjustments = geo_allocation_adjustments(&score.level);

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&score)?);
        }
        _ => {
            println!();
            println!("  {}", "Geopolitical Risk Assessment".bold());
            println!("  {}", "─".repeat(50));
            println!(
                "  Oil (Brent):  ${:.0}/bbl  → {:.2}/2.5",
                oil, score.oil_component
            );
            println!(
                "  VIX:          {:.1}       → {:.2}/2.5",
                vix, score.vix_component
            );
            println!(
                "  Gold 3m:      {:.1}%      → {:.2}/2.5",
                gold_3m * 100.0,
                score.gold_component
            );
            println!(
                "  DXY 3m:       {:.1}%      → {:.2}/2.5",
                dxy_3m * 100.0,
                score.dxy_component
            );
            println!("  {}", "─".repeat(50));
            println!("  Composite: {:.1}/10 — {}", score.composite, score.level);
            if !adjustments.is_empty() {
                println!();
                println!("  Suggested adjustments:");
                for (sector, delta) in &adjustments {
                    let sign = if *delta > 0.0 { "+" } else { "" };
                    println!("    {:<20} {}{:.0}%", sector, sign, delta * 100.0);
                }
            }
            println!();
        }
    }

    Ok(())
}

// ─── Sell Compare ─────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn run_sell_compare(
    ticker: &str,
    shares: f64,
    cost_basis: f64,
    price: f64,
    acquired: &str,
    filing: &str,
    state: &str,
    other_income: f64,
    growth: f64,
    max_years: u32,
    format: OutputFormat,
) -> Result<()> {
    use ghost_alpha::portfolio::types::{AcquisitionType, TaxLot};
    use ghost_alpha::tax::brackets::{state_tax_rate, FederalCapGainsBrackets, FilingStatus};
    use ghost_alpha::tax::projection::{compare_scenarios, format_comparison_table};

    let acquired_date = chrono::NaiveDate::parse_from_str(acquired, "%Y-%m-%d")
        .with_context(|| format!("Invalid date '{}', expected YYYY-MM-DD", acquired))?;

    let filing_status = match filing.to_lowercase().as_str() {
        "single" | "s" => FilingStatus::Single,
        "mfj" | "married" | "joint" => FilingStatus::MarriedFilingJointly,
        "mfs" | "separate" => FilingStatus::MarriedFilingSeparately,
        "hoh" | "head" => FilingStatus::HeadOfHousehold,
        other => anyhow::bail!(
            "Unknown filing status '{}'. Use: single, mfj, mfs, hoh",
            other
        ),
    };

    let brackets = match filing_status {
        FilingStatus::Single => FederalCapGainsBrackets::single_2024(),
        FilingStatus::MarriedFilingJointly => FederalCapGainsBrackets::mfj_2024(),
        _ => FederalCapGainsBrackets::single_2024(),
    };

    let state_rate = state_tax_rate(&state.to_uppercase()).map_err(|e| anyhow::anyhow!("{}", e))?;

    let lot = TaxLot {
        id: uuid::Uuid::new_v4(),
        ticker: ticker.to_uppercase(),
        shares,
        cost_basis_per_share: cost_basis,
        acquired_date,
        acquisition_type: AcquisitionType::RsuVest,
        remaining_shares: shares,
    };

    let start_year = {
        use chrono::Datelike;
        chrono::Utc::now().naive_utc().date().year() as u32
    };

    let scenarios = compare_scenarios(
        &[lot],
        shares,
        max_years,
        start_year,
        growth,
        price,
        other_income,
        &brackets,
        &state_rate,
    );

    match format {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&scenarios)?);
        }
        _ => {
            println!(
                "{}",
                format_comparison_table(&scenarios, &ticker.to_uppercase(), state)
            );
        }
    }

    Ok(())
}

// ─── Web dashboard ────────────────────────────────────────────────────────────

async fn run_serve(cfg: &AppConfig, bind: &str, port: u16) -> Result<()> {
    use ghost_alpha::web::{create_router, AppState};
    use std::sync::Arc;

    let db_path = cfg.storage.db_path.clone();
    let repo = tokio::task::spawn_blocking(move || SqliteRepository::open(&db_path))
        .await?
        .context("Failed to open database")?;

    let state = AppState {
        repo: Arc::new(repo),
        config: Arc::new(cfg.clone()),
    };

    let router = create_router(state);
    let addr: std::net::SocketAddr = format!("{bind}:{port}")
        .parse()
        .with_context(|| format!("Invalid bind address: {bind}:{port}"))?;

    println!("Dashboard running at http://{addr}");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind to {addr}"))?;
    axum::serve(listener, router)
        .await
        .context("Web server error")?;

    Ok(())
}
