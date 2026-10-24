//! Discord webhook alert channel.
//!
//! Sends alerts to Discord via webhook URL. Uses Discord's webhook API format
//! with rich embeds for visual formatting.
//!
//! # Configuration
//!
//! Via TOML:
//! ```toml
//! [discord]
//! webhook_url = "https://discord.com/api/webhooks/..."
//! ```
//!
//! Via env var:
//! ```bash
//! GHOST_ALPHA_DISCORD_WEBHOOK_URL=https://discord.com/api/webhooks/...
//! ```

use async_trait::async_trait;
use serde::Serialize;

use crate::alerts::channel::{AlertChannel, OutboundAlert};
use crate::data::models::Severity;
use crate::error::AlertError;

/// Discord webhook payload with embed.
#[derive(Debug, Serialize)]
struct DiscordPayload {
    username: &'static str,
    embeds: Vec<DiscordEmbed>,
}

#[derive(Debug, Serialize)]
struct DiscordEmbed {
    title: String,
    description: String,
    color: u32,
    fields: Vec<DiscordField>,
    footer: DiscordFooter,
}

#[derive(Debug, Serialize)]
struct DiscordField {
    name: String,
    value: String,
    inline: bool,
}

#[derive(Debug, Serialize)]
struct DiscordFooter {
    text: String,
}

pub struct DiscordChannel {
    webhook_url: String,
    client: reqwest::Client,
}

impl DiscordChannel {
    pub fn new(webhook_url: String) -> Self {
        Self {
            webhook_url,
            client: reqwest::Client::new(),
        }
    }

    fn severity_color(severity: &Severity) -> u32 {
        match severity {
            Severity::Critical => 0xFF0000, // Red
            Severity::Warning => 0xFFA500,  // Orange
            Severity::Info => 0x3498DB,     // Blue
        }
    }
}

#[async_trait]
impl AlertChannel for DiscordChannel {
    async fn send(&self, alert: &OutboundAlert) -> Result<(), AlertError> {
        // Use the narrative as the embed description when available; fall back
        // to the compact data message for the default format.
        let description = alert
            .narrative
            .as_deref()
            .unwrap_or(&alert.message)
            .to_string();

        let payload = DiscordPayload {
            username: "GhostAlpha",
            embeds: vec![DiscordEmbed {
                title: format!("{} {} — {}", alert.severity, alert.alert_type, alert.ticker),
                description,
                color: Self::severity_color(&alert.severity),
                fields: vec![
                    DiscordField {
                        name: "Ticker".into(),
                        value: format!("`{}`", alert.ticker),
                        inline: true,
                    },
                    DiscordField {
                        name: "Score".into(),
                        value: format!("{:.1}", alert.composite_score),
                        inline: true,
                    },
                    DiscordField {
                        name: "Severity".into(),
                        value: alert.severity.to_string(),
                        inline: true,
                    },
                ],
                footer: DiscordFooter {
                    text: format!(
                        "GhostAlpha • {}",
                        chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
                    ),
                },
            }],
        };

        let response = self
            .client
            .post(&self.webhook_url)
            .json(&payload)
            .send()
            .await
            .map_err(|e| {
                AlertError::ChannelUnavailable(format!("Discord webhook failed: {}", e))
            })?;

        if response.status().is_success() || response.status().as_u16() == 204 {
            tracing::info!(ticker = %alert.ticker, "Discord alert delivered");
            Ok(())
        } else {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            Err(AlertError::ChannelUnavailable(format!(
                "Discord webhook returned {}: {}",
                status, body
            )))
        }
    }

    fn name(&self) -> &'static str {
        "discord"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_string_contains, method};
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
    fn severity_colors() {
        assert_eq!(
            DiscordChannel::severity_color(&Severity::Critical),
            0xFF0000
        );
        assert_eq!(DiscordChannel::severity_color(&Severity::Warning), 0xFFA500);
        assert_eq!(DiscordChannel::severity_color(&Severity::Info), 0x3498DB);
    }

    #[test]
    fn channel_name_is_discord() {
        let ch = DiscordChannel::new("https://example.com/webhook".into());
        assert_eq!(ch.name(), "discord");
    }

    // ── Integration tests with wiremock ─────────────────────────────────────

    #[tokio::test]
    async fn send_success_on_204() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(body_string_contains("GhostAlpha"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&mock_server)
            .await;

        let ch = DiscordChannel::new(mock_server.uri());
        let result = ch.send(&sample_alert()).await;
        assert!(result.is_ok(), "Should succeed on 204: {:?}", result);
    }

    #[tokio::test]
    async fn send_success_on_200() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&mock_server)
            .await;

        let ch = DiscordChannel::new(mock_server.uri());
        let result = ch.send(&sample_alert()).await;
        assert!(result.is_ok(), "Should succeed on 200: {:?}", result);
    }

    #[tokio::test]
    async fn send_returns_error_on_401() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Invalid Webhook Token"))
            .mount(&mock_server)
            .await;

        let ch = DiscordChannel::new(mock_server.uri());
        let result = ch.send(&sample_alert()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("401"),
            "Error should contain status code: {msg}"
        );
    }

    #[tokio::test]
    async fn send_returns_error_on_429_rate_limit() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).set_body_string("You are being rate limited."))
            .mount(&mock_server)
            .await;

        let ch = DiscordChannel::new(mock_server.uri());
        let result = ch.send(&sample_alert()).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("429"), "Error should mention 429: {msg}");
    }

    #[tokio::test]
    async fn payload_contains_ticker_and_embed_fields() {
        let mock_server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(body_string_contains("NVDA"))
            .and(body_string_contains("92.5"))
            .and(body_string_contains("embeds"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&mock_server)
            .await;

        let ch = DiscordChannel::new(mock_server.uri());
        let result = ch.send(&sample_alert()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn payload_uses_correct_color_for_severity() {
        let mock_server = MockServer::start().await;

        // 0xFF0000 = 16711680 in decimal — Critical red
        Mock::given(method("POST"))
            .and(body_string_contains("16711680"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&mock_server)
            .await;

        let ch = DiscordChannel::new(mock_server.uri());
        let result = ch.send(&sample_alert()).await;
        assert!(result.is_ok(), "Payload should contain Critical color");
    }
}
