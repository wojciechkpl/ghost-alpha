//! Generic webhook alert channel — BYOA (Bring Your Own Agent).
//!
//! Sends alerts as JSON POST requests to any HTTP endpoint. This enables
//! users to integrate with Slack, PagerDuty, n8n, custom backends, or any
//! service that accepts webhook payloads.
//!
//! # Configuration
//!
//! Via TOML (`config.toml`):
//! ```toml
//! [[plugins.webhooks]]
//! url = "https://hooks.slack.com/services/..."
//! auth_header = "Bearer your-token"
//! headers = ["X-Custom=value"]
//! ```
//!
//! Via environment variable:
//! ```bash
//! GHOST_ALPHA_WEBHOOK_URL=https://your-endpoint.com/alerts
//! GHOST_ALPHA_WEBHOOK_AUTH_HEADER="Bearer token"
//! ```

use async_trait::async_trait;
use serde::Serialize;

use crate::alerts::channel::{AlertChannel, OutboundAlert};
use crate::error::AlertError;

/// JSON payload sent to the webhook endpoint.
#[derive(Debug, Serialize)]
pub struct WebhookPayload {
    pub source: &'static str,
    pub ticker: String,
    pub alert_type: String,
    pub severity: String,
    pub message: String,
    pub composite_score: f64,
    pub timestamp: String,
    /// Plain-English narrative, present when `alert_format = "narrative"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub narrative: Option<String>,
}

impl From<&OutboundAlert> for WebhookPayload {
    fn from(alert: &OutboundAlert) -> Self {
        Self {
            source: "ghost-alpha",
            ticker: alert.ticker.clone(),
            alert_type: alert.alert_type.clone(),
            severity: alert.severity.to_string(),
            message: alert.message.clone(),
            composite_score: alert.composite_score,
            timestamp: chrono::Utc::now().to_rfc3339(),
            narrative: alert.narrative.clone(),
        }
    }
}

/// Generic webhook channel that POSTs JSON alerts to any URL.
pub struct WebhookChannel {
    url: String,
    auth_header: Option<String>,
    extra_headers: Vec<(String, String)>,
    client: reqwest::Client,
    label: String,
}

impl WebhookChannel {
    /// Create a new webhook channel.
    ///
    /// # Arguments
    /// - `url` — Target URL for POST requests
    /// - `auth_header` — Optional `Authorization` header value
    /// - `headers` — Extra headers as `["Key=Value", ...]`
    /// - `label` — Human-readable name for this channel (e.g., "slack-webhook")
    pub fn new(
        url: String,
        auth_header: Option<String>,
        headers: &[String],
        label: String,
    ) -> Self {
        let extra_headers = headers
            .iter()
            .filter_map(|h| {
                let parts: Vec<&str> = h.splitn(2, '=').collect();
                if parts.len() == 2 {
                    Some((parts[0].to_string(), parts[1].to_string()))
                } else {
                    tracing::warn!(header = %h, "Ignoring malformed header (expected Key=Value)");
                    None
                }
            })
            .collect();

        Self {
            url,
            auth_header,
            extra_headers,
            client: reqwest::Client::new(),
            label,
        }
    }
}

#[async_trait]
impl AlertChannel for WebhookChannel {
    async fn send(&self, alert: &OutboundAlert) -> Result<(), AlertError> {
        let payload = WebhookPayload::from(alert);

        let mut request = self.client.post(&self.url).json(&payload);

        if let Some(ref auth) = self.auth_header {
            request = request.header("Authorization", auth);
        }

        for (key, value) in &self.extra_headers {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request.send().await.map_err(|e| {
            AlertError::ChannelUnavailable(format!("Webhook POST to {} failed: {}", self.url, e))
        })?;

        if response.status().is_success() {
            tracing::info!(
                channel = %self.label,
                ticker = %alert.ticker,
                status = %response.status(),
                "Webhook alert delivered"
            );
            Ok(())
        } else {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            Err(AlertError::ChannelUnavailable(format!(
                "Webhook {} returned {}: {}",
                self.url, status, body
            )))
        }
    }

    fn name(&self) -> &'static str {
        // We leak the label string so it has a 'static lifetime.
        // This is fine because channels are long-lived singletons.
        Box::leak(self.label.clone().into_boxed_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_payload_from_alert() {
        let alert = OutboundAlert {
            ticker: "NVDA".into(),
            alert_type: "NewSignal".into(),
            severity: crate::data::models::Severity::Critical,
            message: "Strong setup".into(),
            composite_score: 92.5,
            narrative: None,
        };
        let payload = WebhookPayload::from(&alert);
        assert_eq!(payload.source, "ghost-alpha");
        assert_eq!(payload.ticker, "NVDA");
        assert_eq!(payload.severity, "CRITICAL");
    }

    #[test]
    fn parses_headers() {
        let ch = WebhookChannel::new(
            "https://example.com".into(),
            None,
            &[
                "X-Custom=value".to_string(),
                "Accept=application/json".to_string(),
            ],
            "test".into(),
        );
        assert_eq!(ch.extra_headers.len(), 2);
        assert_eq!(
            ch.extra_headers[0],
            ("X-Custom".to_string(), "value".to_string())
        );
    }

    #[test]
    fn ignores_malformed_headers() {
        let ch = WebhookChannel::new(
            "https://example.com".into(),
            None,
            &["no-equals-sign".to_string()],
            "test".into(),
        );
        assert!(ch.extra_headers.is_empty());
    }
}
