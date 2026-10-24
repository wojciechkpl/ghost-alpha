use async_trait::async_trait;
use std::sync::{Arc, Mutex};

use crate::data::models::Severity;
use crate::error::AlertError;

/// An outbound alert ready for delivery.
#[derive(Debug, Clone)]
pub struct OutboundAlert {
    pub ticker: String,
    pub alert_type: String,
    pub severity: Severity,
    pub message: String,
    pub composite_score: f64,
    /// Plain-English narrative summary, populated when `alert_format = "narrative"`.
    /// `None` in the default data mode.
    pub narrative: Option<String>,
}

/// Delivery channel abstraction — swap Telegram for email/webhook in tests.
#[async_trait]
pub trait AlertChannel: Send + Sync {
    async fn send(&self, alert: &OutboundAlert) -> Result<(), AlertError>;
    fn name(&self) -> &'static str;
}

/// No-op channel for unit tests — records calls without hitting the network.
pub struct NullChannel {
    pub sent: Arc<Mutex<Vec<OutboundAlert>>>,
}

impl NullChannel {
    pub fn new() -> Self {
        Self {
            sent: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn sent_count(&self) -> usize {
        self.sent.lock().unwrap().len()
    }

    pub fn sent_alerts(&self) -> Vec<OutboundAlert> {
        self.sent.lock().unwrap().clone()
    }
}

impl Default for NullChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AlertChannel for NullChannel {
    async fn send(&self, alert: &OutboundAlert) -> Result<(), AlertError> {
        self.sent.lock().unwrap().push(alert.clone());
        Ok(())
    }

    fn name(&self) -> &'static str {
        "null"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn null_channel_records_sent_alert() {
        let ch = NullChannel::new();
        let alert = OutboundAlert {
            ticker: "AAPL".into(),
            alert_type: "NewSignal".into(),
            severity: Severity::Critical,
            message: "test".into(),
            composite_score: 92.0,
            narrative: None,
        };
        ch.send(&alert).await.unwrap();
        assert_eq!(ch.sent_count(), 1);
        assert_eq!(ch.sent_alerts()[0].ticker, "AAPL");
    }

    #[tokio::test]
    async fn null_channel_name_is_null() {
        let ch = NullChannel::new();
        assert_eq!(ch.name(), "null");
    }
}
