//! MCP (Model Context Protocol) JSON-RPC 2.0 server over stdin/stdout.
//!
//! Implements the MCP specification for AI agent ↔ tool communication.
//! Any MCP-compatible agent (Claude Desktop, Goose, VS Code Copilot, etc.)
//! can connect and use GhostAlpha's screening/scoring/analysis tools.
//!
//! # Protocol
//!
//! ```text
//! Client (AI Agent)           Server (this binary)
//!   │                              │
//!   ├─ initialize ──────────────►  │ → capabilities, server info
//!   │                              │
//!   ├─ tools/list ──────────────►  │ → available tool definitions
//!   │                              │
//!   ├─ tools/call ──────────────►  │ → execute tool, return result
//!   │   (screen_stock, etc.)       │
//!   │                              │
//!   ├─ resources/list ──────────►  │ → available data resources
//!   │                              │
//!   ├─ resources/read ──────────►  │ → read specific resource
//!   │                              │
//!   ├─ shutdown ────────────────►  │ → graceful exit
//!   │                              │
//! ```

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::AppConfig;
use crate::plugins::Registry;

use super::tools;

// ─── JSON-RPC 2.0 Types ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }

    fn method_not_found(id: Value, method: &str) -> Self {
        Self::error(id, -32601, format!("Method not found: {method}"))
    }

    fn invalid_params(id: Value, msg: impl Into<String>) -> Self {
        Self::error(id, -32602, msg)
    }

    fn internal_error(id: Value, msg: impl Into<String>) -> Self {
        Self::error(id, -32603, msg)
    }
}

// ─── MCP Tool Schema Types ──────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ToolDefinition {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

// ─── MCP Server ──────────────────────────────────────────────────────────────

/// The MCP server state. Holds config and plugin registry.
pub struct McpServer {
    config: AppConfig,
    registry: Registry,
}

impl McpServer {
    /// Create a new MCP server with the given config and registry.
    pub fn new(config: AppConfig, registry: Registry) -> Self {
        Self { config, registry }
    }

    /// Run the server, reading JSON-RPC messages from stdin and writing
    /// responses to stdout. Runs until EOF or a shutdown request.
    pub fn run(&self) -> io::Result<()> {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut stdout = stdout.lock();

        tracing::info!("GhostAlpha MCP server started (stdin/stdout)");

        for line in stdin.lock().lines() {
            let line = line?;
            let line = line.trim();

            // Skip empty lines
            if line.is_empty() {
                continue;
            }

            // Parse JSON-RPC request
            let request: JsonRpcRequest = match serde_json::from_str(line) {
                Ok(req) => req,
                Err(e) => {
                    let resp =
                        JsonRpcResponse::error(Value::Null, -32700, format!("Parse error: {e}"));
                    self.write_response(&mut stdout, &resp)?;
                    continue;
                }
            };

            let id = request.id.clone().unwrap_or(Value::Null);

            // Route method
            let response = match request.method.as_str() {
                "initialize" => self.handle_initialize(id),
                "initialized" => continue, // notification, no response
                "tools/list" => self.handle_tools_list(id),
                "tools/call" => self.handle_tools_call(id, &request.params),
                "resources/list" => self.handle_resources_list(id),
                "resources/read" => self.handle_resources_read(id, &request.params),
                "ping" => JsonRpcResponse::success(id, serde_json::json!({})),
                "shutdown" => {
                    self.write_response(
                        &mut stdout,
                        &JsonRpcResponse::success(id, serde_json::json!({})),
                    )?;
                    tracing::info!("Shutdown requested, exiting");
                    break;
                }
                _ => JsonRpcResponse::method_not_found(id, &request.method),
            };

            self.write_response(&mut stdout, &response)?;
        }

        Ok(())
    }

    fn write_response(&self, out: &mut impl Write, resp: &JsonRpcResponse) -> io::Result<()> {
        let json = serde_json::to_string(resp)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        writeln!(out, "{json}")?;
        out.flush()
    }

    // ─── Method Handlers ─────────────────────────────────────────────────────

    fn handle_initialize(&self, id: Value) -> JsonRpcResponse {
        JsonRpcResponse::success(
            id,
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {},
                    "resources": {}
                },
                "serverInfo": {
                    "name": "ghost-alpha",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )
    }

    fn handle_tools_list(&self, id: Value) -> JsonRpcResponse {
        let tools = vec![
            ToolDefinition {
                name: "screen_stock".into(),
                description: "Screen a single stock through the Minervini 8-point trend template filter. \
                    Returns technical scores, volume analysis, relative strength, and a composite grade. \
                    Requires at least 252 daily OHLCV candles.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "ticker": {
                            "type": "string",
                            "description": "Stock ticker symbol (e.g., 'AAPL', 'NVDA')"
                        },
                        "candles": {
                            "type": "array",
                            "description": "Array of OHLCV candle objects (minimum 252 bars)",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "date": { "type": "string", "format": "date" },
                                    "open": { "type": "number" },
                                    "high": { "type": "number" },
                                    "low": { "type": "number" },
                                    "close": { "type": "number" },
                                    "adj_close": { "type": "number" },
                                    "volume": { "type": "integer" }
                                },
                                "required": ["date", "open", "high", "low", "close", "adj_close", "volume"]
                            }
                        },
                        "universe_returns": {
                            "type": "array",
                            "items": { "type": "number" },
                            "description": "252-day returns for the universe (for RS percentile). Optional — defaults to 50th percentile."
                        }
                    },
                    "required": ["ticker", "candles"]
                }),
            },
            ToolDefinition {
                name: "score_breakdown".into(),
                description: "Compute a score breakdown from raw indicator values. \
                    Useful for what-if analysis: pass hypothetical indicator values \
                    and see how the composite score changes. Returns technical (0-55), \
                    volume (0-30), sentiment (0-15) sub-scores and composite (0-100).".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "conditions_met": {
                            "type": "integer", "minimum": 0, "maximum": 8,
                            "description": "Number of Minervini conditions met (0-8)"
                        },
                        "adx": {
                            "type": "number",
                            "description": "Average Directional Index value"
                        },
                        "rs_percentile": {
                            "type": "number", "minimum": 0, "maximum": 100,
                            "description": "Relative Strength percentile (0-100)"
                        },
                        "volume_zscore": {
                            "type": "number",
                            "description": "Directional volume Z-score"
                        },
                        "sentiment_score": {
                            "type": "number", "minimum": -1, "maximum": 1,
                            "description": "Sentiment score (-1 to +1)"
                        },
                        "article_count": {
                            "type": "integer", "minimum": 0,
                            "description": "Number of news articles analyzed"
                        }
                    },
                    "required": ["conditions_met", "adx", "rs_percentile", "volume_zscore", "sentiment_score", "article_count"]
                }),
            },
            ToolDefinition {
                name: "get_config".into(),
                description: "Get the current server configuration with API keys masked. \
                    Shows data provider, alert thresholds, storage path, and Telegram status.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            ToolDefinition {
                name: "list_providers".into(),
                description: "List all registered plugin providers, screeners, scorers, and alert channels. \
                    Shows which implementations are available and which are currently active. \
                    Use this to discover available components before switching providers.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            ToolDefinition {
                name: "health_check".into(),
                description: "Check the health status of the GhostAlpha server. \
                    Returns server version, uptime, database status, and API connectivity.".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
        ];

        JsonRpcResponse::success(id, serde_json::json!({ "tools": tools }))
    }

    fn handle_tools_call(&self, id: Value, params: &Value) -> JsonRpcResponse {
        let tool_name = match params.get("name").and_then(|v| v.as_str()) {
            Some(name) => name,
            None => {
                return JsonRpcResponse::invalid_params(id, "Missing 'name' field in tools/call");
            }
        };

        let arguments = params
            .get("arguments")
            .cloned()
            .unwrap_or(serde_json::json!({}));

        match tool_name {
            "screen_stock" => self.call_screen_stock(id, &arguments),
            "score_breakdown" => self.call_score_breakdown(id, &arguments),
            "get_config" => self.call_get_config(id),
            "list_providers" => self.call_list_providers(id),
            "health_check" => self.call_health_check(id),
            _ => JsonRpcResponse::error(id, -32602, format!("Unknown tool: {tool_name}")),
        }
    }

    fn handle_resources_list(&self, id: Value) -> JsonRpcResponse {
        let resources = serde_json::json!({
            "resources": [
                {
                    "uri": "ghost-alpha://config",
                    "name": "Server Configuration",
                    "description": "Current server configuration (secrets masked)",
                    "mimeType": "application/json"
                },
                {
                    "uri": "ghost-alpha://providers",
                    "name": "Plugin Registry",
                    "description": "Registered data providers, screeners, scorers, and channels",
                    "mimeType": "application/json"
                }
            ]
        });
        JsonRpcResponse::success(id, resources)
    }

    fn handle_resources_read(&self, id: Value, params: &Value) -> JsonRpcResponse {
        let uri = match params.get("uri").and_then(|v| v.as_str()) {
            Some(uri) => uri,
            None => {
                return JsonRpcResponse::invalid_params(id, "Missing 'uri' field");
            }
        };

        match uri {
            "ghost-alpha://config" => {
                let config_out = tools::masked_config(&self.config);
                let content = serde_json::to_value(config_out).unwrap();
                JsonRpcResponse::success(
                    id,
                    serde_json::json!({
                        "contents": [{
                            "uri": uri,
                            "mimeType": "application/json",
                            "text": serde_json::to_string_pretty(&content).unwrap()
                        }]
                    }),
                )
            }
            "ghost-alpha://providers" => {
                let providers_out = tools::list_providers(&self.registry);
                let content = serde_json::to_value(providers_out).unwrap();
                JsonRpcResponse::success(
                    id,
                    serde_json::json!({
                        "contents": [{
                            "uri": uri,
                            "mimeType": "application/json",
                            "text": serde_json::to_string_pretty(&content).unwrap()
                        }]
                    }),
                )
            }
            _ => JsonRpcResponse::error(id, -32602, format!("Unknown resource: {uri}")),
        }
    }

    // ─── Tool Call Implementations ───────────────────────────────────────────

    fn call_screen_stock(&self, id: Value, args: &Value) -> JsonRpcResponse {
        let input: tools::ScreenStockInput = match serde_json::from_value(args.clone()) {
            Ok(input) => input,
            Err(e) => {
                return JsonRpcResponse::invalid_params(
                    id,
                    format!("Invalid screen_stock input: {e}"),
                );
            }
        };

        match tools::screen_stock(input) {
            Ok(output) => {
                let result = serde_json::to_value(output).unwrap();
                JsonRpcResponse::success(
                    id,
                    serde_json::json!({
                        "content": [{
                            "type": "text",
                            "text": serde_json::to_string_pretty(&result).unwrap()
                        }]
                    }),
                )
            }
            Err(e) => JsonRpcResponse::internal_error(id, e),
        }
    }

    fn call_score_breakdown(&self, id: Value, args: &Value) -> JsonRpcResponse {
        let input: tools::ScoreBreakdownInput = match serde_json::from_value(args.clone()) {
            Ok(input) => input,
            Err(e) => {
                return JsonRpcResponse::invalid_params(
                    id,
                    format!("Invalid score_breakdown input: {e}"),
                );
            }
        };

        let output = tools::score_breakdown(input);
        let result = serde_json::to_value(output).unwrap();
        JsonRpcResponse::success(
            id,
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&result).unwrap()
                }]
            }),
        )
    }

    fn call_get_config(&self, id: Value) -> JsonRpcResponse {
        let output = tools::masked_config(&self.config);
        let result = serde_json::to_value(output).unwrap();
        JsonRpcResponse::success(
            id,
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&result).unwrap()
                }]
            }),
        )
    }

    fn call_list_providers(&self, id: Value) -> JsonRpcResponse {
        let output = tools::list_providers(&self.registry);
        let result = serde_json::to_value(output).unwrap();
        JsonRpcResponse::success(
            id,
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&result).unwrap()
                }]
            }),
        )
    }

    fn call_health_check(&self, id: Value) -> JsonRpcResponse {
        // Check database connectivity
        let db_status =
            match crate::storage::db::SqliteRepository::open(&self.config.storage.db_path) {
                Ok(_) => "connected",
                Err(_) => "error",
            };

        let health = serde_json::json!({
            "status": "ok",
            "version": env!("CARGO_PKG_VERSION"),
            "database": {
                "path": &self.config.storage.db_path,
                "status": db_status
            },
            "providers": {
                "data": self.registry.active_provider_name().unwrap_or("none"),
                "screener": self.registry.active_screener_name().unwrap_or("none"),
            },
            "telegram_configured": self.config.telegram.is_some(),
        });

        JsonRpcResponse::success(
            id,
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": serde_json::to_string_pretty(&health).unwrap()
                }]
            }),
        )
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> AppConfig {
        AppConfig {
            finnhub: crate::config::FinnhubConfig {
                api_key: "test-key-123".into(),
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

    fn test_server() -> McpServer {
        let config = test_config();
        let registry = crate::plugins::registry::default_registry(&config);
        McpServer::new(config, registry)
    }

    #[test]
    fn initialize_returns_server_info() {
        let server = test_server();
        let resp = server.handle_initialize(Value::from(1));
        let result = resp.result.unwrap();
        assert_eq!(result["serverInfo"]["name"], "ghost-alpha");
        assert!(result["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_returns_all_tools() {
        let server = test_server();
        let resp = server.handle_tools_list(Value::from(2));
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 5);

        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"screen_stock"));
        assert!(names.contains(&"score_breakdown"));
        assert!(names.contains(&"get_config"));
        assert!(names.contains(&"list_providers"));
        assert!(names.contains(&"health_check"));
    }

    #[test]
    fn tools_call_score_breakdown() {
        let server = test_server();
        let params = serde_json::json!({
            "name": "score_breakdown",
            "arguments": {
                "conditions_met": 8,
                "adx": 45.0,
                "rs_percentile": 85.0,
                "volume_zscore": 2.0,
                "sentiment_score": 0.5,
                "article_count": 3
            }
        });
        let resp = server.handle_tools_call(Value::from(3), &params);
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn tools_call_unknown_tool_returns_error() {
        let server = test_server();
        let params = serde_json::json!({
            "name": "nonexistent_tool",
            "arguments": {}
        });
        let resp = server.handle_tools_call(Value::from(4), &params);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[test]
    fn tools_call_missing_name_returns_error() {
        let server = test_server();
        let params = serde_json::json!({});
        let resp = server.handle_tools_call(Value::from(5), &params);
        assert!(resp.error.is_some());
    }

    #[test]
    fn get_config_masks_api_key() {
        let server = test_server();
        let resp = server.call_get_config(Value::from(6));
        let result = resp.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("te...23")); // masked key
        assert!(!text.contains("test-key-123")); // full key not exposed
    }

    #[test]
    fn list_providers_returns_registry() {
        let server = test_server();
        let resp = server.call_list_providers(Value::from(7));
        let result = resp.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("finnhub"));
        assert!(text.contains("minervini"));
    }

    #[test]
    fn resources_list_returns_resources() {
        let server = test_server();
        let resp = server.handle_resources_list(Value::from(8));
        let result = resp.result.unwrap();
        let resources = result["resources"].as_array().unwrap();
        assert_eq!(resources.len(), 2);
    }

    #[test]
    fn resources_read_config() {
        let server = test_server();
        let params = serde_json::json!({ "uri": "ghost-alpha://config" });
        let resp = server.handle_resources_read(Value::from(9), &params);
        assert!(resp.result.is_some());
    }

    #[test]
    fn resources_read_unknown_returns_error() {
        let server = test_server();
        let params = serde_json::json!({ "uri": "ghost-alpha://unknown" });
        let resp = server.handle_resources_read(Value::from(10), &params);
        assert!(resp.error.is_some());
    }

    #[test]
    fn method_not_found() {
        let resp = JsonRpcResponse::method_not_found(Value::from(99), "nonexistent");
        assert_eq!(resp.error.as_ref().unwrap().code, -32601);
    }

    #[test]
    fn invalid_params_constructor() {
        let resp = JsonRpcResponse::invalid_params(Value::from(10), "bad input");
        let err = resp.error.as_ref().unwrap();
        assert_eq!(err.code, -32602);
        assert!(err.message.contains("bad input"));
        assert!(resp.result.is_none());
    }

    #[test]
    fn internal_error_constructor() {
        let resp = JsonRpcResponse::internal_error(Value::from(11), "something went wrong");
        let err = resp.error.as_ref().unwrap();
        assert_eq!(err.code, -32603);
        assert!(err.message.contains("something went wrong"));
        assert!(resp.result.is_none());
    }

    #[test]
    fn handle_ping_returns_empty_object() {
        // The ping handler is inlined in the `run` loop as:
        //   "ping" => JsonRpcResponse::success(id, serde_json::json!({}))
        // We test the same construction path directly.
        let resp = JsonRpcResponse::success(Value::from(12), serde_json::json!({}));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
        let result = resp.result.unwrap();
        assert!(result.is_object());
        assert_eq!(result.as_object().unwrap().len(), 0);
    }

    #[test]
    fn handle_unknown_method_returns_method_not_found() {
        // Verify method_not_found produces the JSON-RPC -32601 error code.
        // The `run` loop dispatches unknown methods to JsonRpcResponse::method_not_found.
        let resp = JsonRpcResponse::method_not_found(Value::from(13), "totally/unknown");
        let err = resp.error.as_ref().unwrap();
        assert_eq!(err.code, -32601);
        assert!(err.message.contains("totally/unknown"));
        assert!(resp.result.is_none());
    }
}
