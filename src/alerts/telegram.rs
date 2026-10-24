//! Telegram Bot API delivery channel.
//!
//! Sends alerts via the Telegram Bot API with MarkdownV2 formatting.
//! Falls back to plain text if MarkdownV2 parsing fails (HTTP 400).
//!
//! # Configuration
//!
//! Via TOML:
//! ```toml
//! [telegram]
//! bot_token = "123456:ABC-DEF..."
//! chat_id = "-100123456789"
//! ```
//!
//! Via env vars:
//! ```bash
//! GHOST_ALPHA_TELEGRAM_BOT_TOKEN=123456:ABC-DEF...
//! GHOST_ALPHA_TELEGRAM_CHAT_ID=-100123456789
//! ```

use async_trait::async_trait;
use serde::Serialize;

use crate::alerts::channel::{AlertChannel, OutboundAlert};
use crate::data::models::Severity;
use crate::error::AlertError;

/// Telegram sendMessage request body.
#[derive(Debug, Serialize)]
struct TelegramPayload {
    chat_id: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_mode: Option<String>,
}

/// Escape special characters for MarkdownV2 as per Telegram Bot API docs.
/// Characters that must be escaped: _ * [ ] ( ) ~ ` > # + - = | { } . !
fn escape_markdown_v2(text: &str) -> String {
    let special = [
        '_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!',
    ];
    let mut result = String::with_capacity(text.len() * 2);
    for ch in text.chars() {
        if special.contains(&ch) {
            result.push('\\');
        }
        result.push(ch);
    }
    result
}

/// Build a MarkdownV2 formatted message from an alert.
fn format_markdown_v2(alert: &OutboundAlert) -> String {
    let emoji = match alert.severity {
        Severity::Critical => "🚨",
        Severity::Warning => "⚠️",
        Severity::Info => "ℹ️",
    };

    let ticker = escape_markdown_v2(&alert.ticker);
    let severity = escape_markdown_v2(&alert.severity.to_string());
    let alert_type = escape_markdown_v2(&alert.alert_type);
    let message = escape_markdown_v2(&alert.message);
    let score = escape_markdown_v2(&format!("{:.1}", alert.composite_score));
    let ts = escape_markdown_v2(&chrono::Utc::now().format("%Y-%m-%d %H:%M UTC").to_string());

    format!(
        "{emoji} *{severity}* \\| `{ticker}` \\| {alert_type}\n\n\
         {message}\n\n\
         *Score:* {score}\n\
         _GhostAlpha • {ts}_"
    )
}

/// Build a plain-text fallback message (no Markdown).
fn format_plain_text(alert: &OutboundAlert) -> String {
    let emoji = match alert.severity {
        Severity::Critical => "🚨",
        Severity::Warning => "⚠️",
        Severity::Info => "ℹ️",
    };

    format!(
        "{emoji} {severity} | {ticker} | {alert_type}\n\n\
         {message}\n\n\
         Score: {score:.1}\n\
         GhostAlpha • {ts}",
        severity = alert.severity,
        ticker = alert.ticker,
        alert_type = alert.alert_type,
        message = alert.message,
        score = alert.composite_score,
        ts = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC"),
    )
}

pub struct TelegramChannel {
    bot_token: String,
    chat_id: String,
    client: reqwest::Client,
    /// Overridable base URL (for testing with wiremock).
    base_url: String,
}

impl TelegramChannel {
    pub fn new(bot_token: impl Into<String>, chat_id: impl Into<String>) -> Self {
        Self {
            bot_token: bot_token.into(),
            chat_id: chat_id.into(),
            client: reqwest::Client::new(),
            base_url: "https://api.telegram.org".to_string(),
        }
    }

    #[cfg(test)]
    pub fn new_with_base_url(
        bot_token: impl Into<String>,
        chat_id: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            bot_token: bot_token.into(),
            chat_id: chat_id.into(),
            client: reqwest::Client::new(),
            base_url: base_url.into(),
        }
    }

    /// Build the full sendMessage URL.
    fn send_message_url(&self) -> String {
        format!("{}/bot{}/sendMessage", self.base_url, self.bot_token)
    }

    /// Send a message with the given payload, return Ok or an error.
    async fn post_message(&self, payload: &TelegramPayload) -> Result<(), AlertError> {
        let response = self
            .client
            .post(self.send_message_url())
            .json(payload)
            .send()
            .await
            .map_err(|e| {
                AlertError::ChannelUnavailable(format!("Telegram request failed: {}", e))
            })?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            Err(AlertError::TelegramApi { status, body })
        }
    }
}

#[async_trait]
impl AlertChannel for TelegramChannel {
    async fn send(&self, alert: &OutboundAlert) -> Result<(), AlertError> {
        // Narrative mode: send as plain text to avoid MarkdownV2 escaping
        // issues with prose (dots, parentheses, dashes, etc.).
        if let Some(ref narrative) = alert.narrative {
            let plain_payload = TelegramPayload {
                chat_id: self.chat_id.clone(),
                text: narrative.clone(),
                parse_mode: None,
            };
            return self.post_message(&plain_payload).await.map(|()| {
                tracing::info!(ticker = %alert.ticker, "Telegram alert delivered (narrative)");
            });
        }

        // Default data mode: try MarkdownV2 first, fall back to plain text.
        let md_payload = TelegramPayload {
            chat_id: self.chat_id.clone(),
            text: format_markdown_v2(alert),
            parse_mode: Some("MarkdownV2".to_string()),
        };

        match self.post_message(&md_payload).await {
            Ok(()) => {
                tracing::info!(ticker = %alert.ticker, "Telegram alert delivered (MarkdownV2)");
                Ok(())
            }
            Err(AlertError::TelegramApi { status: 400, .. }) => {
                // MarkdownV2 parse error — fallback to plain text
                tracing::warn!(
                    ticker = %alert.ticker,
                    "MarkdownV2 failed (400), falling back to plain text"
                );
                let plain_payload = TelegramPayload {
                    chat_id: self.chat_id.clone(),
                    text: format_plain_text(alert),
                    parse_mode: None,
                };
                self.post_message(&plain_payload).await.map(|()| {
                    tracing::info!(ticker = %alert.ticker, "Telegram alert delivered (plain text)");
                })
            }
            Err(e) => Err(e),
        }
    }

    fn name(&self) -> &'static str {
        "telegram"
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alerts::channel::OutboundAlert;
    use crate::data::models::Severity;
    use wiremock::matchers::{body_string_contains, method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_alert() -> OutboundAlert {
        OutboundAlert {
            ticker: "NVDA".into(),
            alert_type: "NewSignal".into(),
            severity: Severity::Critical,
            message: "Strong Minervini setup with volume breakout".into(),
            composite_score: 92.5,
            narrative: None,
        }
    }

    // ── Unit tests (no HTTP) ────────────────────────────────────────────────

    #[test]
    fn telegram_channel_name() {
        let ch = TelegramChannel::new("token", "chat_id");
        assert_eq!(ch.name(), "telegram");
    }

    #[test]
    fn escape_markdown_v2_escapes_special_chars() {
        let input = "Price $500.00 (breakout) [confirmed]";
        let escaped = escape_markdown_v2(input);
        assert!(escaped.contains("\\."), "dot should be escaped");
        assert!(escaped.contains("\\("), "open paren should be escaped");
        assert!(escaped.contains("\\)"), "close paren should be escaped");
        assert!(escaped.contains("\\["), "open bracket should be escaped");
        assert!(escaped.contains("\\]"), "close bracket should be escaped");
    }

    #[test]
    fn escape_markdown_v2_preserves_normal_chars() {
        assert_eq!(escape_markdown_v2("NVDA"), "NVDA");
        assert_eq!(escape_markdown_v2("ABC123"), "ABC123");
    }

    #[test]
    fn escape_markdown_v2_empty_string() {
        assert_eq!(escape_markdown_v2(""), "");
    }

    #[test]
    fn format_markdown_v2_contains_ticker_and_severity() {
        let alert = sample_alert();
        let text = format_markdown_v2(&alert);
        assert!(text.contains("NVDA"), "Should contain ticker");
        assert!(text.contains("CRITICAL"), "Should contain severity");
        assert!(text.contains("Score"), "Should contain score label");
        assert!(text.contains("GhostAlpha"), "Should contain branding");
    }

    #[test]
    fn format_plain_text_contains_all_fields() {
        let alert = sample_alert();
        let text = format_plain_text(&alert);
        assert!(text.contains("NVDA"));
        assert!(text.contains("CRITICAL"));
        assert!(text.contains("NewSignal"));
        assert!(text.contains("92.5"));
        assert!(text.contains("GhostAlpha"));
    }

    #[test]
    fn send_message_url_format() {
        let ch = TelegramChannel::new("123:ABC", "chat_id");
        assert_eq!(
            ch.send_message_url(),
            "https://api.telegram.org/bot123:ABC/sendMessage"
        );
    }

    #[test]
    fn send_message_url_with_custom_base() {
        let ch = TelegramChannel::new_with_base_url("123:ABC", "chat_id", "http://localhost:8080");
        assert_eq!(
            ch.send_message_url(),
            "http://localhost:8080/bot123:ABC/sendMessage"
        );
    }

    // ── Integration tests with wiremock ─────────────────────────────────────

    #[tokio::test]
    async fn send_success_with_markdown_v2() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path_regex(r"/bot.*/sendMessage"))
            .and(body_string_contains("MarkdownV2"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "result": {}})),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let ch = TelegramChannel::new_with_base_url("test_token", "test_chat", mock_server.uri());
        let result = ch.send(&sample_alert()).await;
        assert!(result.is_ok(), "Should succeed on 200: {:?}", result);
    }

    #[tokio::test]
    async fn send_returns_error_on_401() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path_regex(r"/bot.*/sendMessage"))
            .respond_with(ResponseTemplate::new(401).set_body_json(
                serde_json::json!({"ok": false, "error_code": 401, "description": "Unauthorized"}),
            ))
            .mount(&mock_server)
            .await;

        let ch = TelegramChannel::new_with_base_url("bad_token", "test_chat", mock_server.uri());
        let result = ch.send(&sample_alert()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AlertError::TelegramApi { status, body } => {
                assert_eq!(status, 401);
                assert!(body.contains("Unauthorized"));
            }
            other => panic!("Expected TelegramApi error, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn send_returns_error_on_429_rate_limit() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path_regex(r"/bot.*/sendMessage"))
            .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
                "ok": false,
                "error_code": 429,
                "description": "Too Many Requests: retry after 30",
                "parameters": {"retry_after": 30}
            })))
            .mount(&mock_server)
            .await;

        let ch = TelegramChannel::new_with_base_url("test_token", "test_chat", mock_server.uri());
        let result = ch.send(&sample_alert()).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            AlertError::TelegramApi { status, .. } => assert_eq!(status, 429),
            other => panic!("Expected TelegramApi, got: {:?}", other),
        }
    }

    #[tokio::test]
    async fn telegram_sends_narrative_as_plain_text() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path_regex(r"/bot.*/sendMessage"))
            .and(body_string_contains("Hello NVDA"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "result": {}})),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let ch = TelegramChannel::new_with_base_url("test_token", "test_chat", mock_server.uri());
        let alert = OutboundAlert {
            narrative: Some("Hello NVDA".into()),
            ..sample_alert()
        };
        let result = ch.send(&alert).await;
        assert!(
            result.is_ok(),
            "Narrative alert should succeed: {:?}",
            result
        );

        // Verify the captured request does NOT contain "MarkdownV2"
        let received = mock_server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
        let body = std::str::from_utf8(&received[0].body).unwrap();
        assert!(
            body.contains("Hello NVDA"),
            "Body must contain the narrative text"
        );
        assert!(
            !body.contains("MarkdownV2"),
            "Narrative mode must NOT set parse_mode=MarkdownV2"
        );
    }

    #[tokio::test]
    async fn payload_contains_correct_chat_id() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path_regex(r"/bot.*/sendMessage"))
            .and(body_string_contains("my_chat_123"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "result": {}})),
            )
            .expect(1)
            .mount(&mock_server)
            .await;

        let ch = TelegramChannel::new_with_base_url("test_token", "my_chat_123", mock_server.uri());
        let result = ch.send(&sample_alert()).await;
        assert!(result.is_ok());
    }
}
