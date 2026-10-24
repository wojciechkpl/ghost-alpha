use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

/// A single entry in the decision audit log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionLogEntry {
    pub id: Option<i64>,
    pub decision_date: NaiveDate,
    pub decision_type: DecisionType,
    pub ticker: Option<String>,
    pub shares: Option<f64>,
    pub price: Option<f64>,
    pub rationale: String,
    pub advisor: Option<String>,
    pub linked_alert_id: Option<i64>,
    pub created_at: Option<String>,
}

/// The kind of portfolio decision being logged.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DecisionType {
    Sell,
    Buy,
    Hold,
    Rebalance,
    Review,
}

impl std::fmt::Display for DecisionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sell => write!(f, "SELL"),
            Self::Buy => write!(f, "BUY"),
            Self::Hold => write!(f, "HOLD"),
            Self::Rebalance => write!(f, "REBALANCE"),
            Self::Review => write!(f, "REVIEW"),
        }
    }
}

impl std::str::FromStr for DecisionType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "SELL" => Ok(Self::Sell),
            "BUY" => Ok(Self::Buy),
            "HOLD" => Ok(Self::Hold),
            "REBALANCE" => Ok(Self::Rebalance),
            "REVIEW" => Ok(Self::Review),
            other => Err(format!("unknown decision type: {other}")),
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decision_type_display() {
        assert_eq!(DecisionType::Sell.to_string(), "SELL");
        assert_eq!(DecisionType::Buy.to_string(), "BUY");
        assert_eq!(DecisionType::Hold.to_string(), "HOLD");
        assert_eq!(DecisionType::Rebalance.to_string(), "REBALANCE");
        assert_eq!(DecisionType::Review.to_string(), "REVIEW");
    }

    #[test]
    fn test_decision_type_from_str() {
        assert_eq!("SELL".parse::<DecisionType>().unwrap(), DecisionType::Sell);
        assert_eq!("buy".parse::<DecisionType>().unwrap(), DecisionType::Buy);
        assert_eq!("Hold".parse::<DecisionType>().unwrap(), DecisionType::Hold);
        assert!("INVALID".parse::<DecisionType>().is_err());
    }

    #[test]
    fn test_decision_serde_roundtrip() {
        let entry = DecisionLogEntry {
            id: Some(42),
            decision_date: NaiveDate::from_ymd_opt(2026, 3, 14).unwrap(),
            decision_type: DecisionType::Sell,
            ticker: Some("AAPL".to_string()),
            shares: Some(100.0),
            price: Some(175.50),
            rationale: "Taking profits after 30% gain".to_string(),
            advisor: Some("manual".to_string()),
            linked_alert_id: Some(7),
            created_at: Some("2026-03-14T10:00:00".to_string()),
        };

        let json = serde_json::to_string(&entry).expect("serialize");
        let restored: DecisionLogEntry = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(restored.id, Some(42));
        assert_eq!(restored.decision_type, DecisionType::Sell);
        assert_eq!(restored.ticker.as_deref(), Some("AAPL"));
        assert_eq!(restored.shares, Some(100.0));
        assert_eq!(restored.price, Some(175.50));
        assert_eq!(restored.rationale, "Taking profits after 30% gain");
        assert_eq!(restored.advisor.as_deref(), Some("manual"));
        assert_eq!(restored.linked_alert_id, Some(7));
    }

    #[test]
    fn test_decision_serde_with_none_fields() {
        let entry = DecisionLogEntry {
            id: None,
            decision_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            decision_type: DecisionType::Review,
            ticker: None,
            shares: None,
            price: None,
            rationale: "Quarterly portfolio review".to_string(),
            advisor: None,
            linked_alert_id: None,
            created_at: None,
        };

        let json = serde_json::to_string(&entry).expect("serialize");
        let restored: DecisionLogEntry = serde_json::from_str(&json).expect("deserialize");

        assert!(restored.id.is_none());
        assert!(restored.ticker.is_none());
        assert!(restored.shares.is_none());
        assert_eq!(restored.decision_type, DecisionType::Review);
    }
}
