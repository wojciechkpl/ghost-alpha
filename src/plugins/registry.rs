//! Central registry for pluggable components.
//!
//! The [`Registry`] holds named implementations of each trait behind `Arc<dyn Trait>`.
//! This allows multiple components to share the same provider (e.g., one FinnhubClient
//! shared by screener and pipeline), while still allowing runtime swapping.

use std::collections::HashMap;
use std::sync::Arc;

use crate::alerts::channel::AlertChannel;
use crate::analysis::screener::{Screener, SentimentScorer};
use crate::data::provider::DataProvider;
use crate::error::ConfigError;

// ─── Registry ────────────────────────────────────────────────────────────────

/// Central component registry.
///
/// Holds named implementations of each trait. The pipeline selects which
/// implementation to use via config (`[provider] type = "finnhub"`).
///
/// All registrations are `Arc`-wrapped so they can be cheaply cloned and
/// shared across async tasks.
pub struct Registry {
    providers: HashMap<String, Arc<dyn DataProvider>>,
    screeners: HashMap<String, Arc<dyn Screener>>,
    scorers: HashMap<String, Arc<dyn SentimentScorer>>,
    channels: HashMap<String, Arc<dyn AlertChannel>>,

    /// Active selections (set via config or `select_*` methods)
    active_provider: Option<String>,
    active_screener: Option<String>,
    active_scorer: Option<String>,
}

impl Registry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            screeners: HashMap::new(),
            scorers: HashMap::new(),
            channels: HashMap::new(),
            active_provider: None,
            active_screener: None,
            active_scorer: None,
        }
    }

    // ── Registration ─────────────────────────────────────────────────────

    /// Register a data provider implementation under a name.
    ///
    /// ```rust,ignore
    /// registry.register_provider("finnhub", FinnhubClient::new(key));
    /// registry.register_provider("alpaca", AlpacaClient::new(key, secret));
    /// ```
    pub fn register_provider(&mut self, name: &str, provider: impl DataProvider + 'static) {
        self.providers.insert(name.to_string(), Arc::new(provider));
        // Auto-select first registered as default
        if self.active_provider.is_none() {
            self.active_provider = Some(name.to_string());
        }
    }

    /// Register a screener implementation under a name.
    pub fn register_screener(&mut self, name: &str, screener: impl Screener + 'static) {
        self.screeners.insert(name.to_string(), Arc::new(screener));
        if self.active_screener.is_none() {
            self.active_screener = Some(name.to_string());
        }
    }

    /// Register a sentiment scorer implementation under a name.
    pub fn register_scorer(&mut self, name: &str, scorer: impl SentimentScorer + 'static) {
        self.scorers.insert(name.to_string(), Arc::new(scorer));
        if self.active_scorer.is_none() {
            self.active_scorer = Some(name.to_string());
        }
    }

    /// Register an alert delivery channel under a name.
    /// Multiple channels can be active simultaneously (alerts fan out to all).
    pub fn register_channel(&mut self, name: &str, channel: impl AlertChannel + 'static) {
        self.channels.insert(name.to_string(), Arc::new(channel));
    }

    // ── Selection ────────────────────────────────────────────────────────

    /// Select which registered provider to use as the active data source.
    pub fn select_provider(&mut self, name: &str) -> Result<(), ConfigError> {
        if self.providers.contains_key(name) {
            self.active_provider = Some(name.to_string());
            Ok(())
        } else {
            Err(ConfigError::InvalidValue {
                field: "provider".into(),
                reason: format!(
                    "unknown provider '{}', available: {:?}",
                    name,
                    self.providers.keys().collect::<Vec<_>>()
                ),
            })
        }
    }

    /// Select which registered screener to use.
    pub fn select_screener(&mut self, name: &str) -> Result<(), ConfigError> {
        if self.screeners.contains_key(name) {
            self.active_screener = Some(name.to_string());
            Ok(())
        } else {
            Err(ConfigError::InvalidValue {
                field: "screener".into(),
                reason: format!(
                    "unknown screener '{}', available: {:?}",
                    name,
                    self.screeners.keys().collect::<Vec<_>>()
                ),
            })
        }
    }

    /// Select which registered sentiment scorer to use.
    pub fn select_scorer(&mut self, name: &str) -> Result<(), ConfigError> {
        if self.scorers.contains_key(name) {
            self.active_scorer = Some(name.to_string());
            Ok(())
        } else {
            Err(ConfigError::InvalidValue {
                field: "scorer".into(),
                reason: format!(
                    "unknown scorer '{}', available: {:?}",
                    name,
                    self.scorers.keys().collect::<Vec<_>>()
                ),
            })
        }
    }

    // ── Accessors ────────────────────────────────────────────────────────

    /// Get the active data provider.
    pub fn provider(&self) -> Result<Arc<dyn DataProvider>, ConfigError> {
        let name = self
            .active_provider
            .as_ref()
            .ok_or(ConfigError::MissingField {
                field: "No data provider registered. Call register_provider() first.".into(),
            })?;
        self.providers
            .get(name)
            .cloned()
            .ok_or(ConfigError::InvalidValue {
                field: "provider".into(),
                reason: format!("active provider '{}' not found in registry", name),
            })
    }

    /// Get the active screener.
    pub fn screener(&self) -> Result<Arc<dyn Screener>, ConfigError> {
        let name = self
            .active_screener
            .as_ref()
            .ok_or(ConfigError::MissingField {
                field: "No screener registered. Call register_screener() first.".into(),
            })?;
        self.screeners
            .get(name)
            .cloned()
            .ok_or(ConfigError::InvalidValue {
                field: "screener".into(),
                reason: format!("active screener '{}' not found in registry", name),
            })
    }

    /// Get the active sentiment scorer.
    pub fn scorer(&self) -> Result<Arc<dyn SentimentScorer>, ConfigError> {
        let name = self
            .active_scorer
            .as_ref()
            .ok_or(ConfigError::MissingField {
                field: "No scorer registered. Call register_scorer() first.".into(),
            })?;
        self.scorers
            .get(name)
            .cloned()
            .ok_or(ConfigError::InvalidValue {
                field: "scorer".into(),
                reason: format!("active scorer '{}' not found in registry", name),
            })
    }

    /// Get all registered alert channels (alerts fan out to all of them).
    pub fn channels(&self) -> Vec<Arc<dyn AlertChannel>> {
        self.channels.values().cloned().collect()
    }

    /// Get a specific named channel.
    pub fn channel(&self, name: &str) -> Option<Arc<dyn AlertChannel>> {
        self.channels.get(name).cloned()
    }

    // ── Introspection ────────────────────────────────────────────────────

    /// List all registered provider names.
    pub fn provider_names(&self) -> Vec<&str> {
        self.providers.keys().map(|s| s.as_str()).collect()
    }

    /// List all registered screener names.
    pub fn screener_names(&self) -> Vec<&str> {
        self.screeners.keys().map(|s| s.as_str()).collect()
    }

    /// List all registered scorer names.
    pub fn scorer_names(&self) -> Vec<&str> {
        self.scorers.keys().map(|s| s.as_str()).collect()
    }

    /// List all registered channel names.
    pub fn channel_names(&self) -> Vec<&str> {
        self.channels.keys().map(|s| s.as_str()).collect()
    }

    /// Name of the currently active provider.
    pub fn active_provider_name(&self) -> Option<&str> {
        self.active_provider.as_deref()
    }

    /// Name of the currently active screener.
    pub fn active_screener_name(&self) -> Option<&str> {
        self.active_screener.as_deref()
    }

    /// Return a list of `(name, description)` pairs for all registered screeners.
    ///
    /// Used by the `ghost-alpha strategies` subcommand to list available strategies.
    pub fn list_strategies(&self) -> Vec<(String, String)> {
        let mut entries: Vec<(String, String)> = self
            .screeners
            .iter()
            .map(|(name, screener)| (name.clone(), screener.description().to_string()))
            .collect();
        // Sort alphabetically for deterministic output
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Default Registry Builder ────────────────────────────────────────────────

/// Build a registry pre-loaded with the built-in implementations.
///
/// This is the standard way to create a registry for normal use:
/// ```rust,ignore
/// let registry = ghost_alpha::plugins::registry::default_registry(&config);
/// ```
///
/// # BYOA (Bring Your Own Agent) Support
///
/// After building the default registry, custom webhook channels are
/// automatically registered from `[plugins.webhooks]` in config and
/// from `GHOST_ALPHA_WEBHOOK_URL` env var. Any HTTP endpoint that
/// accepts JSON POST payloads can receive alerts.
pub fn default_registry(config: &crate::config::AppConfig) -> Registry {
    use crate::analysis::screener::MinerviniScreener;
    use crate::analysis::sentiment::NullSentimentScorer;
    use crate::data::yahoo::YahooProvider;

    let mut registry = Registry::new();

    // ── Built-in data providers ──────────────────────────────────────────

    // Yahoo is always available — no API key required.
    registry.register_provider("yahoo", YahooProvider::new());

    // Finnhub is registered only when an API key is present.
    if !config.finnhub.api_key.trim().is_empty() {
        use crate::data::fetcher::FinnhubClient;
        registry.register_provider(
            "finnhub",
            FinnhubClient::new(config.finnhub.api_key.clone()),
        );
    }

    // ── Built-in screeners (minervini is default — registered first) ─────
    registry.register_screener("minervini", MinerviniScreener::new());
    {
        use crate::analysis::breakout::BreakoutScreener;
        use crate::analysis::mean_reversion::MeanReversionScreener;
        use crate::analysis::trend_strength::TrendStrengthScreener;
        registry.register_screener("breakout", BreakoutScreener::new());
        registry.register_screener("mean-reversion", MeanReversionScreener::new());
        registry.register_screener("trend-strength", TrendStrengthScreener::new());
    }

    // ── Built-in sentiment scorer ────────────────────────────────────────
    #[cfg(feature = "sentiment")]
    if let Some(ref sent_cfg) = config.sentiment {
        use crate::analysis::onnx_scorer::FinBertScorer;
        registry.register_scorer(
            "finbert",
            FinBertScorer::new(sent_cfg.model_path.clone(), sent_cfg.half_life_days),
        );
    }

    // Null scorer as fallback (always available)
    registry.register_scorer("null", NullSentimentScorer);

    // ── Built-in alert channels ──────────────────────────────────────────
    if let Some(ref tg) = config.telegram {
        use crate::alerts::telegram::TelegramChannel;
        registry.register_channel(
            "telegram",
            TelegramChannel::new(tg.bot_token.clone(), tg.chat_id.clone()),
        );
    }

    // ── Discord webhook channel ──────────────────────────────────────────
    if let Some(ref dc) = config.discord {
        use crate::alerts::discord::DiscordChannel;
        registry.register_channel("discord", DiscordChannel::new(dc.webhook_url.clone()));
    }

    // ── BYOA: Config-driven webhook channels ────────────────────────────
    for (i, wh) in config.plugins.webhooks.iter().enumerate() {
        use crate::alerts::webhook::WebhookChannel;
        let label = format!("webhook-{}", i);
        registry.register_channel(
            Box::leak(label.clone().into_boxed_str()),
            WebhookChannel::new(wh.url.clone(), wh.auth_header.clone(), &wh.headers, label),
        );
    }

    // ── Apply config-driven active selections ───────────────────────────
    if let Err(e) = registry.select_provider(&config.plugins.active_provider) {
        tracing::warn!(
            provider = %config.plugins.active_provider,
            error = %e,
            "Configured active_provider not found, using default"
        );
    }
    if let Err(e) = registry.select_screener(&config.plugins.active_screener) {
        tracing::warn!(
            screener = %config.plugins.active_screener,
            error = %e,
            "Configured active_screener not found, using default"
        );
    }

    registry
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts::channel::NullChannel;
    use crate::analysis::screener::MinerviniScreener;
    use crate::analysis::sentiment::NullSentimentScorer;

    // Helper: create a minimal mock provider for testing
    struct StubProvider;

    #[async_trait::async_trait]
    impl DataProvider for StubProvider {
        async fn fetch_candles(
            &self,
            _t: &str,
            _f: chrono::NaiveDate,
            _to: chrono::NaiveDate,
        ) -> Result<Vec<crate::data::models::Candle>, crate::error::DataError> {
            Ok(vec![])
        }
        async fn fetch_stocks(
            &self,
            _t: &[&str],
        ) -> Result<Vec<crate::data::models::Stock>, crate::error::DataError> {
            Ok(vec![])
        }
        async fn fetch_news(
            &self,
            _t: &str,
            _f: chrono::NaiveDate,
            _l: usize,
        ) -> Result<Vec<crate::data::models::NewsArticle>, crate::error::DataError> {
            Ok(vec![])
        }
        async fn fetch_spy_candles(
            &self,
            _f: chrono::NaiveDate,
            _t: chrono::NaiveDate,
        ) -> Result<Vec<crate::data::models::Candle>, crate::error::DataError> {
            Ok(vec![])
        }
        async fn fetch_universe(&self) -> Result<Vec<String>, crate::error::DataError> {
            Ok(vec!["AAPL".into(), "NVDA".into()])
        }
    }

    #[test]
    fn register_and_retrieve_provider() {
        let mut reg = Registry::new();
        reg.register_provider("stub", StubProvider);
        assert_eq!(reg.active_provider_name(), Some("stub"));
        assert!(reg.provider().is_ok());
    }

    #[test]
    fn first_registered_is_auto_selected() {
        let mut reg = Registry::new();
        reg.register_provider("first", StubProvider);
        reg.register_provider("second", StubProvider);
        assert_eq!(reg.active_provider_name(), Some("first"));
    }

    #[test]
    fn select_provider_switches_active() {
        let mut reg = Registry::new();
        reg.register_provider("a", StubProvider);
        reg.register_provider("b", StubProvider);
        reg.select_provider("b").unwrap();
        assert_eq!(reg.active_provider_name(), Some("b"));
    }

    #[test]
    fn select_unknown_provider_returns_error() {
        let mut reg = Registry::new();
        reg.register_provider("a", StubProvider);
        let err = reg.select_provider("nonexistent");
        assert!(err.is_err());
    }

    #[test]
    fn register_and_retrieve_screener() {
        let mut reg = Registry::new();
        reg.register_screener("minervini", MinerviniScreener::new());
        assert!(reg.screener().is_ok());
        assert_eq!(reg.active_screener_name(), Some("minervini"));
    }

    #[test]
    fn register_and_retrieve_scorer() {
        let mut reg = Registry::new();
        reg.register_scorer("null", NullSentimentScorer);
        assert!(reg.scorer().is_ok());
    }

    #[test]
    fn register_multiple_channels() {
        let mut reg = Registry::new();
        reg.register_channel("null1", NullChannel::new());
        reg.register_channel("null2", NullChannel::new());
        assert_eq!(reg.channels().len(), 2);
        assert!(reg.channel("null1").is_some());
        assert!(reg.channel("null2").is_some());
        assert!(reg.channel("nonexistent").is_none());
    }

    #[test]
    fn introspection_lists_all_registered() {
        let mut reg = Registry::new();
        reg.register_provider("finnhub", StubProvider);
        reg.register_provider("alpaca", StubProvider);
        reg.register_screener("minervini", MinerviniScreener::new());
        reg.register_channel("telegram", NullChannel::new());

        let providers = reg.provider_names();
        assert!(providers.contains(&"finnhub"));
        assert!(providers.contains(&"alpaca"));
        assert_eq!(reg.screener_names(), vec!["minervini"]);
        assert_eq!(reg.channel_names(), vec!["telegram"]);
    }

    #[test]
    fn empty_registry_returns_error_on_access() {
        let reg = Registry::new();
        assert!(reg.provider().is_err());
        assert!(reg.screener().is_err());
        assert!(reg.scorer().is_err());
        assert!(reg.channels().is_empty());
    }

    #[test]
    fn default_registry_has_builtins() {
        let config = crate::config::AppConfig {
            finnhub: crate::config::FinnhubConfig {
                api_key: "test-key".into(),
                requests_per_minute: 60,
                retry_delay_secs: 5,
            },
            telegram: None,
            discord: None,
            fred: None,
            sentiment: None,
            backtest: None,
            alerts: Default::default(),
            storage: Default::default(),
            plugins: Default::default(),
            cache: Default::default(),
            universe: vec![],
            edgar: None,
            profile: None,
        };
        let reg = default_registry(&config);
        assert!(reg.provider().is_ok());
        assert!(reg.screener().is_ok());
        assert!(reg.scorer().is_ok());
        assert!(reg.channels().is_empty()); // no telegram configured
    }

    #[test]
    fn default_registry_includes_telegram_when_configured() {
        let config = crate::config::AppConfig {
            finnhub: crate::config::FinnhubConfig {
                api_key: "test-key".into(),
                requests_per_minute: 60,
                retry_delay_secs: 5,
            },
            telegram: Some(crate::config::TelegramConfig {
                bot_token: "tok".into(),
                chat_id: "chat".into(),
            }),
            discord: None,
            fred: None,
            sentiment: None,
            backtest: None,
            alerts: Default::default(),
            storage: Default::default(),
            plugins: Default::default(),
            cache: Default::default(),
            universe: vec![],
            edgar: None,
            profile: None,
        };
        let reg = default_registry(&config);
        assert_eq!(reg.channel_names().len(), 1);
        assert!(reg.channel("telegram").is_some());
    }

    #[test]
    fn default_registry_includes_webhook_from_config() {
        let config = crate::config::AppConfig {
            finnhub: crate::config::FinnhubConfig {
                api_key: "test-key".into(),
                requests_per_minute: 60,
                retry_delay_secs: 5,
            },
            telegram: None,
            discord: None,
            fred: None,
            sentiment: None,
            backtest: None,
            alerts: Default::default(),
            storage: Default::default(),
            plugins: crate::config::PluginsConfig {
                webhooks: vec![crate::config::WebhookConfig {
                    url: "https://example.com/hook".into(),
                    auth_header: Some("Bearer test".into()),
                    headers: vec![],
                }],
                ..Default::default()
            },
            cache: Default::default(),
            universe: vec![],
            edgar: None,
            profile: None,
        };
        let reg = default_registry(&config);
        assert!(reg.channel("webhook-0").is_some());
    }

    #[test]
    fn default_registry_includes_discord_when_configured() {
        let config = crate::config::AppConfig {
            finnhub: crate::config::FinnhubConfig {
                api_key: "test-key".into(),
                requests_per_minute: 60,
                retry_delay_secs: 5,
            },
            telegram: None,
            discord: Some(crate::config::DiscordConfig {
                webhook_url: "https://discord.com/api/webhooks/test".into(),
            }),
            fred: None,
            sentiment: None,
            backtest: None,
            alerts: Default::default(),
            storage: Default::default(),
            plugins: Default::default(),
            cache: Default::default(),
            universe: vec![],
            edgar: None,
            profile: None,
        };
        let reg = default_registry(&config);
        assert!(reg.channel("discord").is_some());
    }

    #[test]
    fn default_registry_without_sentiment_uses_null_scorer() {
        let config = crate::config::AppConfig {
            finnhub: crate::config::FinnhubConfig {
                api_key: "test-key".into(),
                requests_per_minute: 60,
                retry_delay_secs: 5,
            },
            telegram: None,
            discord: None,
            fred: None,
            sentiment: None,
            backtest: None,
            alerts: Default::default(),
            storage: Default::default(),
            plugins: Default::default(),
            cache: Default::default(),
            universe: vec![],
            edgar: None,
            profile: None,
        };
        let reg = default_registry(&config);
        assert!(reg.scorer().is_ok());
        // Without sentiment feature, only null scorer is registered
        assert!(reg.scorer_names().contains(&"null"));
    }

    #[test]
    fn default_registry_always_has_null_scorer() {
        // Even with sentiment config, null scorer should still be registered
        let config = crate::config::AppConfig {
            finnhub: crate::config::FinnhubConfig {
                api_key: "test-key".into(),
                requests_per_minute: 60,
                retry_delay_secs: 5,
            },
            telegram: None,
            discord: None,
            fred: None,
            sentiment: Some(crate::config::SentimentConfig {
                model_path: "/models/finbert".into(),
                news_lookback_days: 7,
                min_articles: 3,
                half_life_days: 3.5,
            }),
            backtest: None,
            alerts: Default::default(),
            storage: Default::default(),
            plugins: Default::default(),
            cache: Default::default(),
            universe: vec![],
            edgar: None,
            profile: None,
        };
        let reg = default_registry(&config);
        assert!(reg.scorer_names().contains(&"null"));
    }

    fn minimal_config() -> crate::config::AppConfig {
        crate::config::AppConfig {
            finnhub: crate::config::FinnhubConfig {
                api_key: "test-key".into(),
                requests_per_minute: 60,
                retry_delay_secs: 5,
            },
            telegram: None,
            discord: None,
            fred: None,
            sentiment: None,
            backtest: None,
            alerts: Default::default(),
            storage: Default::default(),
            plugins: Default::default(),
            cache: Default::default(),
            universe: vec![],
            edgar: None,
            profile: None,
        }
    }

    #[test]
    fn default_registry_registers_all_four_screeners() {
        let reg = default_registry(&minimal_config());
        let names = reg.screener_names();
        assert!(names.contains(&"minervini"), "missing minervini");
        assert!(names.contains(&"breakout"), "missing breakout");
        assert!(names.contains(&"mean-reversion"), "missing mean-reversion");
        assert!(names.contains(&"trend-strength"), "missing trend-strength");
        assert_eq!(
            names.len(),
            4,
            "Should have exactly 4 strategies, got {:?}",
            names
        );
    }

    #[test]
    fn default_registry_active_screener_is_minervini() {
        let reg = default_registry(&minimal_config());
        assert_eq!(
            reg.active_screener_name(),
            Some("minervini"),
            "Default strategy must be minervini"
        );
    }

    #[test]
    fn default_registry_select_breakout_works() {
        let mut reg = default_registry(&minimal_config());
        assert!(reg.select_screener("breakout").is_ok());
        assert_eq!(reg.active_screener_name(), Some("breakout"));
    }

    #[test]
    fn default_registry_select_mean_reversion_works() {
        let mut reg = default_registry(&minimal_config());
        assert!(reg.select_screener("mean-reversion").is_ok());
    }

    #[test]
    fn default_registry_select_trend_strength_works() {
        let mut reg = default_registry(&minimal_config());
        assert!(reg.select_screener("trend-strength").is_ok());
    }

    #[test]
    fn list_strategies_returns_all_four() {
        let reg = default_registry(&minimal_config());
        let strategies = reg.list_strategies();
        assert_eq!(
            strategies.len(),
            4,
            "list_strategies should return 4 entries"
        );
        let names: Vec<&str> = strategies.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"minervini"));
        assert!(names.contains(&"breakout"));
        assert!(names.contains(&"mean-reversion"));
        assert!(names.contains(&"trend-strength"));
    }

    #[test]
    fn list_strategies_entries_have_descriptions() {
        let reg = default_registry(&minimal_config());
        for (name, desc) in reg.list_strategies() {
            assert!(
                !desc.is_empty(),
                "Strategy '{}' should have a non-empty description",
                name
            );
        }
    }
}
