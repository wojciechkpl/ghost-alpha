use std::env;
use std::path::Path;

use serde::Deserialize;

// ─── Alert format ─────────────────────────────────────────────────────────────

/// Controls how alert text is presented to the user.
///
/// - `Data` (default): compact data table, e.g. `Score: 94.2 | RS: 97th | Vol-Z: 2.8`
/// - `Narrative`: plain-English summary suitable for non-technical recipients.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AlertFormat {
    #[default]
    Data,
    Narrative,
}

use crate::error::ConfigError;

// ─── Environment variable names ─────────────────────────────────────────────

/// All secret/sensitive configuration can be overridden via environment variables.
/// Env vars take precedence over TOML file values.
///
/// SECURITY: No secrets are ever hardcoded. All credentials must be provided via
/// environment variables or a `.env` file (auto-loaded by `dotenvy`).
pub const ENV_FINNHUB_API_KEY: &str = "GHOST_ALPHA_FINNHUB_API_KEY";
pub const ENV_TELEGRAM_BOT_TOKEN: &str = "GHOST_ALPHA_TELEGRAM_BOT_TOKEN";
pub const ENV_TELEGRAM_CHAT_ID: &str = "GHOST_ALPHA_TELEGRAM_CHAT_ID";
pub const ENV_DB_PATH: &str = "GHOST_ALPHA_DB_PATH";
pub const ENV_LOG_LEVEL: &str = "GHOST_ALPHA_LOG_LEVEL";

// ── Webhook / BYOA channels ─────────────────────────────────────────────────
pub const ENV_WEBHOOK_URL: &str = "GHOST_ALPHA_WEBHOOK_URL";
pub const ENV_WEBHOOK_AUTH_HEADER: &str = "GHOST_ALPHA_WEBHOOK_AUTH_HEADER";
pub const ENV_DISCORD_WEBHOOK_URL: &str = "GHOST_ALPHA_DISCORD_WEBHOOK_URL";

// ── MCP server ──────────────────────────────────────────────────────────────
pub const ENV_MCP_ENABLED: &str = "GHOST_ALPHA_MCP_ENABLED";

/// Load `.env` file if present (silently ignored if absent).
/// Call this once at startup before any `from_file` / `from_env` calls.
pub fn load_dotenv() {
    // dotenvy::dotenv() loads .env from the current directory.
    // It's intentionally silent on missing file — in Docker/CI the env vars
    // are injected directly and no .env file exists.
    let _ = dotenvy::dotenv();
}

// ─── Sub-configs ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct FinnhubConfig {
    /// Finnhub API key.  May be empty when `plugins.active_provider = "yahoo"`.
    #[serde(default)]
    pub api_key: String,
    /// Requests per minute (default: 60 — free tier limit)
    #[serde(default = "FinnhubConfig::default_rpm")]
    pub requests_per_minute: u32,
    /// Seconds to wait on retry (default: 5)
    #[serde(default = "FinnhubConfig::default_retry_delay")]
    pub retry_delay_secs: u64,
}

impl FinnhubConfig {
    fn default_rpm() -> u32 {
        60
    }
    fn default_retry_delay() -> u64 {
        5
    }
}

impl Default for FinnhubConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            requests_per_minute: Self::default_rpm(),
            retry_delay_secs: Self::default_retry_delay(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: String,
    pub chat_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlertsConfig {
    /// Composite score ≥ this → CRITICAL
    #[serde(default = "AlertsConfig::default_critical")]
    pub critical_score_threshold: f64,
    /// Composite score ≥ this → WARNING
    #[serde(default = "AlertsConfig::default_warning")]
    pub warning_score_threshold: f64,
    /// Composite score ≥ this → INFO
    #[serde(default = "AlertsConfig::default_info")]
    pub info_score_threshold: f64,
    /// Volume Z-score that, combined with Minervini pass, escalates to CRITICAL
    #[serde(default = "AlertsConfig::default_vol_spike")]
    pub volume_spike_threshold: f64,
    /// Deduplication cooldown in days
    #[serde(default = "AlertsConfig::default_dedup")]
    pub dedup_cooldown_days: i64,
    /// Max alerts per day (0 = unlimited)
    #[serde(default = "AlertsConfig::default_daily")]
    pub max_daily_alerts: usize,
    /// Quiet hours start (24h local)
    #[serde(default = "AlertsConfig::default_quiet_start")]
    pub quiet_start_hour: u32,
    /// Quiet hours end (24h local)
    #[serde(default = "AlertsConfig::default_quiet_end")]
    pub quiet_end_hour: u32,
    /// Minimum stocks in same GICS sector to emit concentration warning
    #[serde(default = "AlertsConfig::default_sector")]
    pub sector_concentration_threshold: usize,
    /// Alert presentation format: "data" (default) or "narrative"
    #[serde(default)]
    pub alert_format: AlertFormat,
}

impl AlertsConfig {
    fn default_critical() -> f64 {
        90.0
    }
    fn default_warning() -> f64 {
        70.0
    }
    fn default_info() -> f64 {
        50.0
    }
    fn default_vol_spike() -> f64 {
        2.0
    }
    fn default_dedup() -> i64 {
        7
    }
    fn default_daily() -> usize {
        50
    }
    fn default_quiet_start() -> u32 {
        22
    }
    fn default_quiet_end() -> u32 {
        7
    }
    fn default_sector() -> usize {
        3
    }
}

impl Default for AlertsConfig {
    fn default() -> Self {
        Self {
            critical_score_threshold: Self::default_critical(),
            warning_score_threshold: Self::default_warning(),
            info_score_threshold: Self::default_info(),
            volume_spike_threshold: Self::default_vol_spike(),
            dedup_cooldown_days: Self::default_dedup(),
            max_daily_alerts: Self::default_daily(),
            quiet_start_hour: Self::default_quiet_start(),
            quiet_end_hour: Self::default_quiet_end(),
            sector_concentration_threshold: Self::default_sector(),
            alert_format: AlertFormat::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct StorageConfig {
    /// Path to SQLite database file
    #[serde(default = "StorageConfig::default_db_path")]
    pub db_path: String,
}

impl StorageConfig {
    fn default_db_path() -> String {
        "ghost_alpha.db".to_string()
    }
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            db_path: Self::default_db_path(),
        }
    }
}

// ─── FRED config ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct FredConfig {
    /// FRED API key (free at https://fred.stlouisfed.org/docs/api/api_key.html)
    pub api_key: String,
}

// ─── Webhook config (generic BYOA alert channel) ────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct WebhookConfig {
    /// Target URL for POST requests (or set GHOST_ALPHA_WEBHOOK_URL env var)
    pub url: String,
    /// Optional Authorization header value (or set GHOST_ALPHA_WEBHOOK_AUTH_HEADER env var)
    pub auth_header: Option<String>,
    /// Custom headers as key=value pairs
    #[serde(default)]
    pub headers: Vec<String>,
}

// ─── Discord webhook config ─────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct DiscordConfig {
    /// Discord webhook URL (or set GHOST_ALPHA_DISCORD_WEBHOOK_URL env var)
    pub webhook_url: String,
}

// ─── Plugins config (BYOA registration) ─────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct PluginsConfig {
    /// Active data provider name (default: "finnhub")
    #[serde(default = "PluginsConfig::default_provider")]
    pub active_provider: String,
    /// Active screener name (default: "minervini")
    #[serde(default = "PluginsConfig::default_screener")]
    pub active_screener: String,
    /// Custom webhook alert channels
    #[serde(default)]
    pub webhooks: Vec<WebhookConfig>,
}

impl PluginsConfig {
    fn default_provider() -> String {
        "finnhub".to_string()
    }
    fn default_screener() -> String {
        "minervini".to_string()
    }
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            active_provider: Self::default_provider(),
            active_screener: Self::default_screener(),
            webhooks: Vec::new(),
        }
    }
}

// ─── Sentiment config ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SentimentConfig {
    /// Path to the ONNX model directory
    pub model_path: String,
    /// How many days of news to look back (default: 7)
    #[serde(default = "SentimentConfig::default_lookback")]
    pub news_lookback_days: u32,
    /// Minimum articles required for a valid sentiment score (default: 3)
    #[serde(default = "SentimentConfig::default_min_articles")]
    pub min_articles: usize,
    /// Half-life for time-decay weighting in days (default: 3.5)
    #[serde(default = "SentimentConfig::default_half_life")]
    pub half_life_days: f64,
}

impl SentimentConfig {
    fn default_lookback() -> u32 {
        7
    }
    fn default_min_articles() -> usize {
        3
    }
    fn default_half_life() -> f64 {
        3.5
    }
}

// ─── Backtest config ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct BacktestConfig {
    /// Default number of years of historical data (default: 5)
    #[serde(default = "BacktestConfig::default_years")]
    pub default_years: u32,
    /// Forward return horizons in days (default: [30, 60, 90])
    #[serde(default = "BacktestConfig::default_forward_windows")]
    pub forward_windows: Vec<u32>,
    /// Walk-forward in-sample period in months (default: 12)
    #[serde(default = "BacktestConfig::default_is_months")]
    pub walk_forward_in_sample_months: u32,
    /// Walk-forward out-of-sample period in months (default: 3)
    #[serde(default = "BacktestConfig::default_oos_months")]
    pub walk_forward_out_of_sample_months: u32,
    /// Monte Carlo bootstrap iterations (default: 10000)
    #[serde(default = "BacktestConfig::default_mc_iterations")]
    pub monte_carlo_iterations: u32,
    /// Transaction cost in basis points (default: 10.0)
    #[serde(default = "BacktestConfig::default_tx_cost")]
    pub transaction_cost_bps: f64,
}

impl BacktestConfig {
    fn default_years() -> u32 {
        5
    }
    fn default_forward_windows() -> Vec<u32> {
        vec![30, 60, 90]
    }
    fn default_is_months() -> u32 {
        12
    }
    fn default_oos_months() -> u32 {
        3
    }
    fn default_mc_iterations() -> u32 {
        10000
    }
    fn default_tx_cost() -> f64 {
        10.0
    }
}

// ─── Cache config ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    /// Days between full data refreshes (default: 7).
    /// Between full refreshes, only delta (new) candles are fetched.
    /// A full refresh catches retroactive adj_close corrections from splits/dividends.
    #[serde(default = "CacheConfig::default_full_refresh_days")]
    pub full_refresh_days: u32,
    /// Enable delta caching (default: true). When false, always fetches full history.
    #[serde(default = "CacheConfig::default_enabled")]
    pub enabled: bool,
}

impl CacheConfig {
    fn default_full_refresh_days() -> u32 {
        7
    }
    fn default_enabled() -> bool {
        true
    }
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            full_refresh_days: Self::default_full_refresh_days(),
            enabled: Self::default_enabled(),
        }
    }
}

// ─── Root config ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct AppConfig {
    /// Finnhub credentials.  Not required when `plugins.active_provider = "yahoo"`.
    #[serde(default)]
    pub finnhub: FinnhubConfig,
    pub telegram: Option<TelegramConfig>,
    pub discord: Option<DiscordConfig>,
    pub fred: Option<FredConfig>,
    pub sentiment: Option<SentimentConfig>,
    pub backtest: Option<BacktestConfig>,
    #[serde(default)]
    pub alerts: AlertsConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    /// Universe of tickers to screen (overrides fetched list when non-empty)
    #[serde(default)]
    pub universe: Vec<String>,
}

impl AppConfig {
    /// Load config from a TOML file, then overlay environment variable overrides.
    ///
    /// Precedence (highest wins):
    /// 1. Environment variables (`GHOST_ALPHA_*`)
    /// 2. `.env` file (auto-loaded by dotenvy)
    /// 3. TOML config file values
    /// 4. Serde defaults
    pub fn from_file(path: &str) -> Result<Self, ConfigError> {
        // Auto-load .env file if present (no-op if absent)
        load_dotenv();

        if !Path::new(path).exists() {
            return Err(ConfigError::FileNotFound {
                path: path.to_string(),
            });
        }
        let content =
            std::fs::read_to_string(path).map_err(|e| ConfigError::Parse(e.to_string()))?;
        let mut config: AppConfig =
            toml::from_str(&content).map_err(|e| ConfigError::Parse(e.to_string()))?;
        config.resolve_env_overrides();
        config.validate()?;
        Ok(config)
    }

    /// Load config purely from environment variables (no TOML file required).
    /// Useful for Docker / CI environments where all config is injected via env.
    ///
    /// Auto-loads `.env` file if present via dotenvy.
    pub fn from_env() -> Result<Self, ConfigError> {
        // Auto-load .env file if present
        load_dotenv();

        // Finnhub API key is optional — only required when provider = "finnhub".
        let finnhub_api_key = env::var(ENV_FINNHUB_API_KEY).unwrap_or_default();

        let telegram = match (
            env::var(ENV_TELEGRAM_BOT_TOKEN),
            env::var(ENV_TELEGRAM_CHAT_ID),
        ) {
            (Ok(token), Ok(chat)) if !token.trim().is_empty() && !chat.trim().is_empty() => {
                Some(TelegramConfig {
                    bot_token: token,
                    chat_id: chat,
                })
            }
            _ => None,
        };

        let discord = env::var(ENV_DISCORD_WEBHOOK_URL)
            .ok()
            .filter(|u| !u.trim().is_empty())
            .map(|url| DiscordConfig { webhook_url: url });

        let storage = StorageConfig {
            db_path: env::var(ENV_DB_PATH).unwrap_or_else(|_| StorageConfig::default_db_path()),
        };

        // Build webhook configs from env vars
        let mut webhooks = Vec::new();
        if let Ok(url) = env::var(ENV_WEBHOOK_URL) {
            if !url.trim().is_empty() {
                webhooks.push(WebhookConfig {
                    url,
                    auth_header: env::var(ENV_WEBHOOK_AUTH_HEADER)
                        .ok()
                        .filter(|h| !h.trim().is_empty()),
                    headers: Vec::new(),
                });
            }
        }

        // Determine active provider: default to "yahoo" when no Finnhub key is set.
        let active_provider = if finnhub_api_key.trim().is_empty() {
            "yahoo".to_string()
        } else {
            PluginsConfig::default_provider()
        };

        let config = AppConfig {
            finnhub: FinnhubConfig {
                api_key: finnhub_api_key,
                requests_per_minute: FinnhubConfig::default_rpm(),
                retry_delay_secs: FinnhubConfig::default_retry_delay(),
            },
            telegram,
            discord,
            fred: None,
            sentiment: None,
            backtest: None,
            alerts: AlertsConfig::default(),
            storage,
            plugins: PluginsConfig {
                webhooks,
                active_provider,
                ..Default::default()
            },
            cache: CacheConfig::default(),
            universe: Vec::new(),
        };
        config.validate()?;
        Ok(config)
    }

    /// Overlay environment variables on top of existing config.
    /// Only non-empty env vars override the config value.
    fn resolve_env_overrides(&mut self) {
        if let Ok(key) = env::var(ENV_FINNHUB_API_KEY) {
            if !key.trim().is_empty() {
                self.finnhub.api_key = key;
            }
        }

        if let Ok(token) = env::var(ENV_TELEGRAM_BOT_TOKEN) {
            if !token.trim().is_empty() {
                if let Some(ref mut tg) = self.telegram {
                    tg.bot_token = token;
                } else {
                    // Create Telegram config from env if not in TOML
                    let chat_id = env::var(ENV_TELEGRAM_CHAT_ID).unwrap_or_default();
                    self.telegram = Some(TelegramConfig {
                        bot_token: token,
                        chat_id,
                    });
                }
            }
        }

        if let Ok(chat_id) = env::var(ENV_TELEGRAM_CHAT_ID) {
            if !chat_id.trim().is_empty() {
                if let Some(ref mut tg) = self.telegram {
                    tg.chat_id = chat_id;
                }
            }
        }

        // Discord webhook
        if let Ok(url) = env::var(ENV_DISCORD_WEBHOOK_URL) {
            if !url.trim().is_empty() {
                self.discord = Some(DiscordConfig { webhook_url: url });
            }
        }

        // Generic webhook
        if let Ok(url) = env::var(ENV_WEBHOOK_URL) {
            if !url.trim().is_empty() {
                let auth = env::var(ENV_WEBHOOK_AUTH_HEADER)
                    .ok()
                    .filter(|h| !h.trim().is_empty());
                // Add env-based webhook if not already in the TOML list
                let already_present = self.plugins.webhooks.iter().any(|w| w.url == url);
                if !already_present {
                    self.plugins.webhooks.push(WebhookConfig {
                        url,
                        auth_header: auth,
                        headers: Vec::new(),
                    });
                }
            }
        }

        if let Ok(db_path) = env::var(ENV_DB_PATH) {
            if !db_path.trim().is_empty() {
                self.storage.db_path = db_path;
            }
        }
    }

    fn validate(&self) -> Result<(), ConfigError> {
        // The Finnhub API key is only required when Finnhub is the active provider.
        let provider = self.plugins.active_provider.as_str();
        if provider == "finnhub" && self.finnhub.api_key.trim().is_empty() {
            return Err(ConfigError::MissingField {
                field: "finnhub.api_key (set in TOML or GHOST_ALPHA_FINNHUB_API_KEY env var)"
                    .into(),
            });
        }
        if self.alerts.critical_score_threshold <= self.alerts.warning_score_threshold {
            return Err(ConfigError::InvalidValue {
                field: "alerts.critical_score_threshold".into(),
                reason: "must be greater than warning_score_threshold".into(),
            });
        }
        if self.alerts.warning_score_threshold <= self.alerts.info_score_threshold {
            return Err(ConfigError::InvalidValue {
                field: "alerts.warning_score_threshold".into(),
                reason: "must be greater than info_score_threshold".into(),
            });
        }
        Ok(())
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_toml() -> &'static str {
        r#"
[finnhub]
api_key = "test-key"
"#
    }

    fn full_toml() -> &'static str {
        r#"
universe = ["NVDA", "AMD", "AAPL"]

[finnhub]
api_key = "abc123"
requests_per_minute = 30

[telegram]
bot_token = "bot:TOKEN"
chat_id = "-1001234567890"

[alerts]
critical_score_threshold = 90.0
warning_score_threshold  = 70.0
info_score_threshold     = 50.0
max_daily_alerts         = 100

[storage]
db_path = "/var/lib/ghost-alpha/data.db"
"#
    }

    #[test]
    fn parse_minimal_config_uses_defaults() {
        let cfg: AppConfig = toml::from_str(minimal_toml()).unwrap();
        assert_eq!(cfg.finnhub.api_key, "test-key");
        assert_eq!(cfg.finnhub.requests_per_minute, 60);
        assert_eq!(cfg.alerts.critical_score_threshold, 90.0);
        assert_eq!(cfg.alerts.max_daily_alerts, 50);
        assert!(cfg.telegram.is_none());
        assert!(cfg.universe.is_empty());
    }

    #[test]
    fn parse_full_config() {
        let cfg: AppConfig = toml::from_str(full_toml()).unwrap();
        assert_eq!(cfg.finnhub.requests_per_minute, 30);
        assert_eq!(cfg.alerts.max_daily_alerts, 100);
        assert_eq!(cfg.storage.db_path, "/var/lib/ghost-alpha/data.db");
        assert_eq!(cfg.universe, vec!["NVDA", "AMD", "AAPL"]);
        let tg = cfg.telegram.unwrap();
        assert_eq!(tg.chat_id, "-1001234567890");
    }

    #[test]
    fn validate_rejects_empty_api_key() {
        let toml = r#"[finnhub]
api_key = "   "
"#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err();
        matches!(err, ConfigError::MissingField { .. });
    }

    #[test]
    fn validate_rejects_inverted_thresholds() {
        let toml = r#"[finnhub]
api_key = "key"
[alerts]
critical_score_threshold = 60.0
warning_score_threshold  = 70.0
info_score_threshold     = 50.0
"#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        let err = cfg.validate().unwrap_err();
        matches!(err, ConfigError::InvalidValue { .. });
    }

    #[test]
    fn file_not_found_returns_error() {
        let err = AppConfig::from_file("/nonexistent/path/config.toml").unwrap_err();
        matches!(err, ConfigError::FileNotFound { .. });
    }

    #[test]
    fn alerts_config_default() {
        let cfg = AlertsConfig::default();
        assert_eq!(cfg.critical_score_threshold, 90.0);
        assert_eq!(cfg.dedup_cooldown_days, 7);
        assert_eq!(cfg.quiet_start_hour, 22);
    }

    #[test]
    fn storage_config_default_path() {
        let cfg = StorageConfig::default();
        assert_eq!(cfg.db_path, "ghost_alpha.db");
    }

    // ── Environment variable tests ─────────────────────────────────────────

    #[test]
    fn env_override_finnhub_api_key() {
        let mut cfg: AppConfig = toml::from_str(minimal_toml()).unwrap();
        assert_eq!(cfg.finnhub.api_key, "test-key");

        // Simulate env var override
        std::env::set_var(super::ENV_FINNHUB_API_KEY, "env-secret-key");
        cfg.resolve_env_overrides();
        assert_eq!(cfg.finnhub.api_key, "env-secret-key");
        std::env::remove_var(super::ENV_FINNHUB_API_KEY);
    }

    #[test]
    fn env_override_creates_telegram_config() {
        let mut cfg: AppConfig = toml::from_str(minimal_toml()).unwrap();
        assert!(cfg.telegram.is_none());

        std::env::set_var(super::ENV_TELEGRAM_BOT_TOKEN, "bot:ENV_TOKEN");
        std::env::set_var(super::ENV_TELEGRAM_CHAT_ID, "-999");
        cfg.resolve_env_overrides();
        let tg = cfg.telegram.unwrap();
        assert_eq!(tg.bot_token, "bot:ENV_TOKEN");
        assert_eq!(tg.chat_id, "-999");
        std::env::remove_var(super::ENV_TELEGRAM_BOT_TOKEN);
        std::env::remove_var(super::ENV_TELEGRAM_CHAT_ID);
    }

    #[test]
    fn env_override_db_path() {
        let mut cfg: AppConfig = toml::from_str(minimal_toml()).unwrap();
        assert_eq!(cfg.storage.db_path, "ghost_alpha.db");

        std::env::set_var(super::ENV_DB_PATH, "/tmp/test.db");
        cfg.resolve_env_overrides();
        assert_eq!(cfg.storage.db_path, "/tmp/test.db");
        std::env::remove_var(super::ENV_DB_PATH);
    }

    #[test]
    fn env_override_empty_string_does_not_override() {
        // Ensure no stale env var from parallel tests
        std::env::remove_var(super::ENV_FINNHUB_API_KEY);

        let mut cfg: AppConfig = toml::from_str(minimal_toml()).unwrap();
        let original_key = cfg.finnhub.api_key.clone();

        std::env::set_var(super::ENV_FINNHUB_API_KEY, "   ");
        cfg.resolve_env_overrides();
        assert_eq!(cfg.finnhub.api_key, original_key);
        std::env::remove_var(super::ENV_FINNHUB_API_KEY);
    }

    #[test]
    fn env_var_constants_have_correct_prefix() {
        assert!(super::ENV_FINNHUB_API_KEY.starts_with("GHOST_ALPHA_"));
        assert!(super::ENV_TELEGRAM_BOT_TOKEN.starts_with("GHOST_ALPHA_"));
        assert!(super::ENV_TELEGRAM_CHAT_ID.starts_with("GHOST_ALPHA_"));
        assert!(super::ENV_DB_PATH.starts_with("GHOST_ALPHA_"));
        assert!(super::ENV_LOG_LEVEL.starts_with("GHOST_ALPHA_"));
        assert!(super::ENV_WEBHOOK_URL.starts_with("GHOST_ALPHA_"));
        assert!(super::ENV_WEBHOOK_AUTH_HEADER.starts_with("GHOST_ALPHA_"));
        assert!(super::ENV_DISCORD_WEBHOOK_URL.starts_with("GHOST_ALPHA_"));
        assert!(super::ENV_MCP_ENABLED.starts_with("GHOST_ALPHA_"));
    }

    #[test]
    fn plugins_config_defaults() {
        let cfg = PluginsConfig::default();
        assert_eq!(cfg.active_provider, "finnhub");
        assert_eq!(cfg.active_screener, "minervini");
        assert!(cfg.webhooks.is_empty());
    }

    #[test]
    fn parse_sentiment_config() {
        let toml = r#"
[finnhub]
api_key = "test-key"
[sentiment]
model_path = "/models/finbert"
news_lookback_days = 14
min_articles = 5
half_life_days = 2.0
"#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        let sent = cfg.sentiment.unwrap();
        assert_eq!(sent.model_path, "/models/finbert");
        assert_eq!(sent.news_lookback_days, 14);
        assert_eq!(sent.min_articles, 5);
        assert!((sent.half_life_days - 2.0).abs() < 1e-6);
    }

    #[test]
    fn sentiment_config_defaults() {
        let toml = r#"
[finnhub]
api_key = "test-key"
[sentiment]
model_path = "/models/finbert"
"#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        let sent = cfg.sentiment.unwrap();
        assert_eq!(sent.news_lookback_days, 7);
        assert_eq!(sent.min_articles, 3);
        assert!((sent.half_life_days - 3.5).abs() < 1e-6);
    }

    #[test]
    fn parse_backtest_config() {
        let toml = r#"
[finnhub]
api_key = "test-key"
[backtest]
default_years = 3
forward_windows = [30, 60]
monte_carlo_iterations = 5000
transaction_cost_bps = 20.0
"#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        let bt = cfg.backtest.unwrap();
        assert_eq!(bt.default_years, 3);
        assert_eq!(bt.forward_windows, vec![30, 60]);
        assert_eq!(bt.monte_carlo_iterations, 5000);
        assert!((bt.transaction_cost_bps - 20.0).abs() < 1e-6);
    }

    #[test]
    fn backtest_config_defaults() {
        let toml = r#"
[finnhub]
api_key = "test-key"
[backtest]
"#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        let bt = cfg.backtest.unwrap();
        assert_eq!(bt.default_years, 5);
        assert_eq!(bt.forward_windows, vec![30, 60, 90]);
        assert_eq!(bt.walk_forward_in_sample_months, 12);
        assert_eq!(bt.walk_forward_out_of_sample_months, 3);
        assert_eq!(bt.monte_carlo_iterations, 10000);
        assert!((bt.transaction_cost_bps - 10.0).abs() < 1e-6);
    }

    #[test]
    fn full_config_with_sentiment_and_backtest() {
        let toml = r#"
universe = ["NVDA"]
[finnhub]
api_key = "abc123"
[sentiment]
model_path = "/models/finbert"
[backtest]
default_years = 3
"#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert!(cfg.sentiment.is_some());
        assert!(cfg.backtest.is_some());
        assert_eq!(cfg.universe, vec!["NVDA"]);
    }

    #[test]
    fn alert_format_defaults_to_data() {
        let cfg = AlertsConfig::default();
        assert_eq!(cfg.alert_format, AlertFormat::Data);
    }

    #[test]
    fn alert_format_parses_narrative_from_toml() {
        let toml = r#"
[finnhub]
api_key = "test-key"
[alerts]
alert_format = "narrative"
"#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.alerts.alert_format, AlertFormat::Narrative);
    }

    #[test]
    fn alert_format_parses_data_from_toml() {
        let toml = r#"
[finnhub]
api_key = "test-key"
[alerts]
alert_format = "data"
"#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.alerts.alert_format, AlertFormat::Data);
    }

    #[test]
    fn alert_format_missing_from_toml_defaults_to_data() {
        let toml = r#"
[finnhub]
api_key = "test-key"
[alerts]
max_daily_alerts = 10
"#;
        let cfg: AppConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.alerts.alert_format, AlertFormat::Data);
    }
}
