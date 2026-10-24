//! Alpaca broker integration for paper and live trading.
//!
//! Uses Alpaca's REST API v2. Paper trading uses `paper-api.alpaca.markets`,
//! live uses `api.alpaca.markets`. Both use the same API contract.
//!
//! Requires environment variables:
//!   ALPACA_API_KEY_ID
//!   ALPACA_API_SECRET_KEY
//!   ALPACA_PAPER=true (default, for safety)

use serde::{Deserialize, Serialize};

// ─── Configuration ──────────────────────────────────────────────────────────

const PAPER_BASE_URL: &str = "https://paper-api.alpaca.markets/v2";
const LIVE_BASE_URL: &str = "https://api.alpaca.markets/v2";

/// Alpaca execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Preview what would be executed — no API calls
    DryRun,
    /// Execute on paper trading account
    Paper,
    /// Execute on live account (requires confirmation)
    Live,
}

/// Alpaca API credentials.
#[derive(Debug, Clone)]
pub struct AlpacaConfig {
    pub api_key_id: String,
    pub api_secret_key: String,
    pub mode: ExecutionMode,
}

impl AlpacaConfig {
    /// Load from environment variables.
    pub fn from_env(mode: ExecutionMode) -> Result<Self, AlpacaError> {
        let api_key_id = std::env::var("ALPACA_API_KEY_ID")
            .map_err(|_| AlpacaError::MissingCredential("ALPACA_API_KEY_ID".into()))?;
        let api_secret_key = std::env::var("ALPACA_API_SECRET_KEY")
            .map_err(|_| AlpacaError::MissingCredential("ALPACA_API_SECRET_KEY".into()))?;

        Ok(Self {
            api_key_id,
            api_secret_key,
            mode,
        })
    }

    fn base_url(&self) -> &str {
        match self.mode {
            ExecutionMode::DryRun | ExecutionMode::Paper => PAPER_BASE_URL,
            ExecutionMode::Live => LIVE_BASE_URL,
        }
    }
}

// ─── Error Type ─────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum AlpacaError {
    #[error("Missing credential: {0}")]
    MissingCredential(String),
    #[error("HTTP error: {0}")]
    Http(String),
    #[error("API error: {status} — {message}")]
    Api { status: u16, message: String },
    #[error("Deserialization error: {0}")]
    Deserialize(String),
    #[error("Order rejected: {0}")]
    OrderRejected(String),
    #[error("Insufficient buying power: need ${needed:.2}, have ${available:.2}")]
    InsufficientFunds { needed: f64, available: f64 },
    #[error("Dry run mode — no order submitted")]
    DryRunBlocked,
}

// ─── API Types ──────────────────────────────────────────────────────────────

/// Alpaca account information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub status: String,
    pub buying_power: String,
    pub cash: String,
    pub portfolio_value: String,
    pub equity: String,
    pub last_equity: String,
    pub pattern_day_trader: bool,
}

impl Account {
    pub fn buying_power_f64(&self) -> f64 {
        self.buying_power.parse().unwrap_or(0.0)
    }

    pub fn equity_f64(&self) -> f64 {
        self.equity.parse().unwrap_or(0.0)
    }
}

/// Alpaca position.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlpacaPosition {
    pub symbol: String,
    pub qty: String,
    pub avg_entry_price: String,
    pub market_value: String,
    pub current_price: String,
    pub unrealized_pl: String,
    pub unrealized_plpc: String,
    pub side: String,
}

/// Order request.
#[derive(Debug, Clone, Serialize)]
pub struct OrderRequest {
    pub symbol: String,
    pub qty: String,
    pub side: OrderSide,
    pub r#type: OrderType,
    pub time_in_force: TimeInForce,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_price: Option<String>,
}

/// Order response from Alpaca.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderResponse {
    pub id: String,
    pub status: String,
    pub symbol: String,
    pub qty: String,
    pub side: String,
    pub r#type: String,
    pub filled_qty: Option<String>,
    pub filled_avg_price: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrderType {
    Market,
    Limit,
    Stop,
    #[serde(rename = "stop_limit")]
    StopLimit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TimeInForce {
    Day,
    Gtc,
    Ioc,
}

// ─── Client ─────────────────────────────────────────────────────────────────

/// Alpaca API client.
pub struct AlpacaClient {
    config: AlpacaConfig,
    http: reqwest::Client,
}

impl AlpacaClient {
    pub fn new(config: AlpacaConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    fn headers(&self) -> Result<reqwest::header::HeaderMap, AlpacaError> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "APCA-API-KEY-ID",
            self.config.api_key_id.trim().parse().map_err(|_| {
                AlpacaError::MissingCredential("APCA-API-KEY-ID contains invalid characters".into())
            })?,
        );
        headers.insert(
            "APCA-API-SECRET-KEY",
            self.config.api_secret_key.trim().parse().map_err(|_| {
                AlpacaError::MissingCredential(
                    "APCA-API-SECRET-KEY contains invalid characters".into(),
                )
            })?,
        );
        Ok(headers)
    }

    /// Get account information.
    pub async fn get_account(&self) -> Result<Account, AlpacaError> {
        let url = format!("{}/account", self.config.base_url());
        let resp = self
            .http
            .get(&url)
            .headers(self.headers()?)
            .send()
            .await
            .map_err(|e| AlpacaError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(AlpacaError::Api {
                status,
                message: body,
            });
        }

        resp.json::<Account>()
            .await
            .map_err(|e| AlpacaError::Deserialize(e.to_string()))
    }

    /// Get all open positions.
    pub async fn get_positions(&self) -> Result<Vec<AlpacaPosition>, AlpacaError> {
        let url = format!("{}/positions", self.config.base_url());
        let resp = self
            .http
            .get(&url)
            .headers(self.headers()?)
            .send()
            .await
            .map_err(|e| AlpacaError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(AlpacaError::Api {
                status,
                message: body,
            });
        }

        resp.json::<Vec<AlpacaPosition>>()
            .await
            .map_err(|e| AlpacaError::Deserialize(e.to_string()))
    }

    /// Submit an order. Returns DryRunBlocked in dry-run mode.
    pub async fn submit_order(&self, order: &OrderRequest) -> Result<OrderResponse, AlpacaError> {
        if self.config.mode == ExecutionMode::DryRun {
            return Err(AlpacaError::DryRunBlocked);
        }

        let url = format!("{}/orders", self.config.base_url());
        let resp = self
            .http
            .post(&url)
            .headers(self.headers()?)
            .json(order)
            .send()
            .await
            .map_err(|e| AlpacaError::Http(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(AlpacaError::Api {
                status,
                message: body,
            });
        }

        resp.json::<OrderResponse>()
            .await
            .map_err(|e| AlpacaError::Deserialize(e.to_string()))
    }
}

// ─── Execution Helpers ──────────────────────────────────────────────────────

/// Result of an execution attempt.
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionResult {
    pub ticker: String,
    pub side: String,
    pub shares: f64,
    pub mode: String,
    pub status: String,
    pub order_id: Option<String>,
    pub message: String,
}

/// Execute a buy order from a position sizing recommendation.
pub async fn execute_buy(
    client: &AlpacaClient,
    ticker: &str,
    shares: f64,
    limit_price: Option<f64>,
) -> ExecutionResult {
    let mode = match client.config.mode {
        ExecutionMode::DryRun => "dry-run",
        ExecutionMode::Paper => "paper",
        ExecutionMode::Live => "live",
    };

    if shares <= 0.0 {
        return ExecutionResult {
            ticker: ticker.to_string(),
            side: "buy".into(),
            shares,
            mode: mode.into(),
            status: "skipped".into(),
            order_id: None,
            message: "Zero or negative shares".into(),
        };
    }

    let order = OrderRequest {
        symbol: ticker.to_string(),
        qty: (shares as u64).to_string(),
        side: OrderSide::Buy,
        r#type: if limit_price.is_some() {
            OrderType::Limit
        } else {
            OrderType::Market
        },
        time_in_force: TimeInForce::Day,
        limit_price: limit_price.map(|p| format!("{p:.2}")),
        stop_price: None,
    };

    match client.submit_order(&order).await {
        Ok(resp) => ExecutionResult {
            ticker: ticker.to_string(),
            side: "buy".into(),
            shares,
            mode: mode.into(),
            message: format!("Order {} — {}", resp.status, resp.id),
            status: resp.status.clone(),
            order_id: Some(resp.id),
        },
        Err(AlpacaError::DryRunBlocked) => ExecutionResult {
            ticker: ticker.to_string(),
            side: "buy".into(),
            shares,
            mode: mode.into(),
            status: "preview".into(),
            order_id: None,
            message: format!(
                "DRY RUN: Would buy {} shares of {} {}",
                shares as u64,
                ticker,
                limit_price
                    .map(|p| format!("@ ${p:.2} limit"))
                    .unwrap_or_else(|| "@ market".into())
            ),
        },
        Err(e) => ExecutionResult {
            ticker: ticker.to_string(),
            side: "buy".into(),
            shares,
            mode: mode.into(),
            status: "error".into(),
            order_id: None,
            message: format!("Error: {e}"),
        },
    }
}

/// Execute a sell order.
pub async fn execute_sell(
    client: &AlpacaClient,
    ticker: &str,
    shares: f64,
    stop_price: Option<f64>,
) -> ExecutionResult {
    let mode = match client.config.mode {
        ExecutionMode::DryRun => "dry-run",
        ExecutionMode::Paper => "paper",
        ExecutionMode::Live => "live",
    };

    if shares <= 0.0 {
        return ExecutionResult {
            ticker: ticker.to_string(),
            side: "sell".into(),
            shares,
            mode: mode.into(),
            status: "skipped".into(),
            order_id: None,
            message: "Zero or negative shares".into(),
        };
    }

    let order = OrderRequest {
        symbol: ticker.to_string(),
        qty: (shares as u64).to_string(),
        side: OrderSide::Sell,
        r#type: if stop_price.is_some() {
            OrderType::Stop
        } else {
            OrderType::Market
        },
        time_in_force: TimeInForce::Day,
        limit_price: None,
        stop_price: stop_price.map(|p| format!("{p:.2}")),
    };

    match client.submit_order(&order).await {
        Ok(resp) => ExecutionResult {
            ticker: ticker.to_string(),
            side: "sell".into(),
            shares,
            mode: mode.into(),
            message: format!("Order {} — {}", resp.status, resp.id),
            status: resp.status.clone(),
            order_id: Some(resp.id),
        },
        Err(AlpacaError::DryRunBlocked) => ExecutionResult {
            ticker: ticker.to_string(),
            side: "sell".into(),
            shares,
            mode: mode.into(),
            status: "preview".into(),
            order_id: None,
            message: format!(
                "DRY RUN: Would sell {} shares of {} {}",
                shares as u64,
                ticker,
                stop_price
                    .map(|p| format!("with stop @ ${p:.2}"))
                    .unwrap_or_else(|| "@ market".into())
            ),
        },
        Err(e) => ExecutionResult {
            ticker: ticker.to_string(),
            side: "sell".into(),
            shares,
            mode: mode.into(),
            status: "error".into(),
            order_id: None,
            message: format!("Error: {e}"),
        },
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_from_env_missing_key() {
        // Clear env vars to test error case
        std::env::remove_var("ALPACA_API_KEY_ID");
        let result = AlpacaConfig::from_env(ExecutionMode::Paper);
        assert!(result.is_err());
    }

    #[test]
    fn paper_mode_uses_paper_url() {
        let config = AlpacaConfig {
            api_key_id: "test".into(),
            api_secret_key: "test".into(),
            mode: ExecutionMode::Paper,
        };
        assert!(config.base_url().contains("paper"));
    }

    #[test]
    fn live_mode_uses_live_url() {
        let config = AlpacaConfig {
            api_key_id: "test".into(),
            api_secret_key: "test".into(),
            mode: ExecutionMode::Live,
        };
        assert!(!config.base_url().contains("paper"));
    }

    #[test]
    fn dry_run_uses_paper_url() {
        let config = AlpacaConfig {
            api_key_id: "test".into(),
            api_secret_key: "test".into(),
            mode: ExecutionMode::DryRun,
        };
        assert!(config.base_url().contains("paper"));
    }

    #[test]
    fn account_buying_power_parse() {
        let acct = Account {
            id: "test".into(),
            status: "ACTIVE".into(),
            buying_power: "150000.50".into(),
            cash: "50000".into(),
            portfolio_value: "200000".into(),
            equity: "200000".into(),
            last_equity: "198000".into(),
            pattern_day_trader: false,
        };
        assert!((acct.buying_power_f64() - 150000.50).abs() < 0.01);
        assert!((acct.equity_f64() - 200000.0).abs() < 0.01);
    }

    #[test]
    fn order_request_serializes() {
        let order = OrderRequest {
            symbol: "AAPL".into(),
            qty: "10".into(),
            side: OrderSide::Buy,
            r#type: OrderType::Market,
            time_in_force: TimeInForce::Day,
            limit_price: None,
            stop_price: None,
        };
        let json = serde_json::to_string(&order).unwrap();
        assert!(json.contains("\"symbol\":\"AAPL\""));
        assert!(json.contains("\"side\":\"buy\""));
        assert!(!json.contains("limit_price")); // skip_serializing_if
    }

    #[test]
    fn order_with_limit_price() {
        let order = OrderRequest {
            symbol: "NVDA".into(),
            qty: "5".into(),
            side: OrderSide::Buy,
            r#type: OrderType::Limit,
            time_in_force: TimeInForce::Day,
            limit_price: Some("180.50".into()),
            stop_price: None,
        };
        let json = serde_json::to_string(&order).unwrap();
        assert!(json.contains("\"limit_price\":\"180.50\""));
        assert!(json.contains("\"type\":\"limit\""));
    }

    #[tokio::test]
    async fn dry_run_buy_returns_preview() {
        let config = AlpacaConfig {
            api_key_id: "test".into(),
            api_secret_key: "test".into(),
            mode: ExecutionMode::DryRun,
        };
        let client = AlpacaClient::new(config);

        let result = execute_buy(&client, "AAPL", 10.0, None).await;
        assert_eq!(result.status, "preview");
        assert!(result.message.contains("DRY RUN"));
        assert!(result.message.contains("10 shares"));
        assert!(result.order_id.is_none());
    }

    #[tokio::test]
    async fn dry_run_sell_returns_preview() {
        let config = AlpacaConfig {
            api_key_id: "test".into(),
            api_secret_key: "test".into(),
            mode: ExecutionMode::DryRun,
        };
        let client = AlpacaClient::new(config);

        let result = execute_sell(&client, "NVDA", 5.0, Some(170.0)).await;
        assert_eq!(result.status, "preview");
        assert!(result.message.contains("DRY RUN"));
        assert!(result.message.contains("stop"));
    }

    #[tokio::test]
    async fn zero_shares_skipped() {
        let config = AlpacaConfig {
            api_key_id: "test".into(),
            api_secret_key: "test".into(),
            mode: ExecutionMode::Paper,
        };
        let client = AlpacaClient::new(config);

        let result = execute_buy(&client, "AAPL", 0.0, None).await;
        assert_eq!(result.status, "skipped");
    }
}
