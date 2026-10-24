//! MCP (Model Context Protocol) binary entry point for GhostAlpha.
//!
//! This binary runs GhostAlpha as an MCP server, communicating over stdin/stdout
//! using JSON-RPC 2.0. Any MCP-compatible AI agent can connect and use
//! GhostAlpha's screening, scoring, and analysis tools.
//!
//! # Usage
//!
//! ## Direct invocation
//! ```bash
//! ghost-alpha-mcp                      # Uses config.toml
//! ghost-alpha-mcp -c custom.toml       # Custom config file
//! GHOST_ALPHA_FINNHUB_API_KEY=xxx ghost-alpha-mcp --env-only  # Env vars only
//! ```
//!
//! ## Claude Desktop integration
//! Add to `~/.config/claude/claude_desktop_config.json`:
//! ```json
//! {
//!   "mcpServers": {
//!     "ghost-alpha": {
//!       "command": "ghost-alpha-mcp",
//!       "env": {
//!         "GHOST_ALPHA_FINNHUB_API_KEY": "your-key-here"
//!       }
//!     }
//!   }
//! }
//! ```
//!
//! ## Goose integration
//! Add to Goose extensions config:
//! ```yaml
//! extensions:
//!   ghost-alpha:
//!     type: stdio
//!     command: ghost-alpha-mcp
//!     env:
//!       GHOST_ALPHA_FINNHUB_API_KEY: "your-key-here"
//! ```

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::EnvFilter;

use ghost_alpha::config::AppConfig;
use ghost_alpha::mcp::server::McpServer;
use ghost_alpha::plugins::registry::default_registry;

#[derive(Parser)]
#[command(
    name = "ghost-alpha-mcp",
    about = "GhostAlpha MCP server — AI agent integration for stock screening",
    version
)]
struct Cli {
    /// Path to config TOML file (ignored if --env-only is set)
    #[arg(short, long, default_value = "config.toml")]
    config: String,

    /// Load configuration entirely from environment variables (no TOML file needed).
    /// Useful for Docker, CI, and MCP agent integration.
    #[arg(long)]
    env_only: bool,

    /// Enable debug logging to stderr (stdout is reserved for MCP JSON-RPC)
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Logging goes to stderr — stdout is the MCP JSON-RPC channel
    let filter = if cli.verbose {
        EnvFilter::new("ghost_alpha=debug,info")
    } else {
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"))
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    // Load config
    let config = if cli.env_only {
        AppConfig::from_env().context("Failed to load config from environment variables")?
    } else {
        AppConfig::from_file(&cli.config)
            .or_else(|_| {
                tracing::info!(
                    "Config file '{}' not found, falling back to environment variables",
                    cli.config
                );
                AppConfig::from_env()
            })
            .context("Failed to load config from file or environment")?
    };

    // Build plugin registry
    let registry = default_registry(&config);

    tracing::info!(
        provider = registry.active_provider_name().unwrap_or("none"),
        screener = registry.active_screener_name().unwrap_or("none"),
        "MCP server ready"
    );

    // Run the server (blocks until EOF or shutdown)
    let server = McpServer::new(config, registry);
    server.run().context("MCP server error")?;

    Ok(())
}
