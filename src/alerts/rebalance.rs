use serde::Serialize;

// ─── Types ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct DriftAlert {
    pub ticker: String,
    pub current_weight_pct: f64,
    pub target_weight_pct: f64,
    pub drift_pct: f64,
    pub direction: DriftDirection,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum DriftDirection {
    Over,
    Under,
}

#[derive(Debug, Clone)]
pub struct TargetWeight {
    pub asset: String,
    pub weight_pct: f64, // e.g. 25.0 for 25%
}

// ─── Core Logic ──────────────────────────────────────────────────────────────

/// Check portfolio positions against target weights.
/// Returns alerts for any position drifting more than `threshold_pct` from target.
///
/// `current_weights` contains (ticker, weight-as-fraction) pairs, e.g. 0.25 for 25%.
/// `targets` contains target weights already expressed as percentages.
/// `threshold_pct` is the minimum absolute drift (in percentage points) to trigger an alert.
pub fn check_weight_drift(
    current_weights: &[(&str, f64)],
    targets: &[TargetWeight],
    threshold_pct: f64,
) -> Vec<DriftAlert> {
    let mut alerts = Vec::with_capacity(targets.len());

    for target in targets {
        let current_pct = current_weights
            .iter()
            .find(|(t, _)| *t == target.asset.as_str())
            .map(|(_, w)| *w * 100.0)
            .unwrap_or(0.0);

        let drift = current_pct - target.weight_pct;

        if drift.abs() > threshold_pct {
            alerts.push(DriftAlert {
                ticker: target.asset.clone(),
                current_weight_pct: current_pct,
                target_weight_pct: target.weight_pct,
                drift_pct: drift,
                direction: if drift > 0.0 {
                    DriftDirection::Over
                } else {
                    DriftDirection::Under
                },
            });
        }
    }

    alerts
}

/// Format drift alerts as human-readable messages for alert channels.
pub fn format_drift_alerts(alerts: &[DriftAlert]) -> String {
    if alerts.is_empty() {
        return "Portfolio weights are within target thresholds. No rebalancing needed."
            .to_string();
    }

    let mut lines = Vec::with_capacity(alerts.len() + 1);
    lines.push(format!(
        "Rebalancing alert: {} position(s) drifted beyond threshold",
        alerts.len()
    ));

    for a in alerts {
        let arrow = match a.direction {
            DriftDirection::Over => "OVER",
            DriftDirection::Under => "UNDER",
        };
        lines.push(format!(
            "  {} — current {:.1}% vs target {:.1}% (drift {:+.1}%, {})",
            a.ticker, a.current_weight_pct, a.target_weight_pct, a.drift_pct, arrow,
        ));
    }

    lines.join("\n")
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn tw(asset: &str, weight_pct: f64) -> TargetWeight {
        TargetWeight {
            asset: asset.to_string(),
            weight_pct,
        }
    }

    #[test]
    fn test_no_drift_no_alerts() {
        let current = vec![("AAPL", 0.25), ("GOOG", 0.25), ("MSFT", 0.50)];
        let targets = vec![tw("AAPL", 25.0), tw("GOOG", 25.0), tw("MSFT", 50.0)];
        let alerts = check_weight_drift(&current, &targets, 5.0);
        assert!(
            alerts.is_empty(),
            "Exact target weights should produce no alerts"
        );
    }

    #[test]
    fn test_small_drift_below_threshold() {
        // 2% drift with 5% threshold → no alert
        let current = vec![("AAPL", 0.27)]; // 27% vs 25% target = 2% drift
        let targets = vec![tw("AAPL", 25.0)];
        let alerts = check_weight_drift(&current, &targets, 5.0);
        assert!(
            alerts.is_empty(),
            "2% drift should not trigger alert at 5% threshold"
        );
    }

    #[test]
    fn test_drift_above_threshold_emits_alert() {
        // 6% drift with 5% threshold → alert
        let current = vec![("AAPL", 0.31)]; // 31% vs 25% target = 6% drift
        let targets = vec![tw("AAPL", 25.0)];
        let alerts = check_weight_drift(&current, &targets, 5.0);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].ticker, "AAPL");
    }

    #[test]
    fn test_over_weight_direction() {
        let current = vec![("AAPL", 0.30)]; // 30% vs 25% target
        let targets = vec![tw("AAPL", 25.0)];
        let alerts = check_weight_drift(&current, &targets, 3.0);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].direction, DriftDirection::Over);
    }

    #[test]
    fn test_under_weight_direction() {
        let current = vec![("AAPL", 0.20)]; // 20% vs 25% target
        let targets = vec![tw("AAPL", 25.0)];
        let alerts = check_weight_drift(&current, &targets, 3.0);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].direction, DriftDirection::Under);
    }

    #[test]
    fn test_missing_position_is_full_underweight() {
        // Target 25% but ticker not in portfolio → current = 0%, drift = -25%
        let current: Vec<(&str, f64)> = vec![("GOOG", 1.0)]; // only GOOG, no AAPL
        let targets = vec![tw("AAPL", 25.0)];
        let alerts = check_weight_drift(&current, &targets, 5.0);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].ticker, "AAPL");
        assert!((alerts[0].drift_pct - (-25.0)).abs() < f64::EPSILON);
        assert_eq!(alerts[0].direction, DriftDirection::Under);
    }

    #[test]
    fn test_multiple_drifts() {
        // 3 assets: AAPL drifts over, GOOG on target, MSFT drifts under
        let current = vec![("AAPL", 0.40), ("GOOG", 0.33), ("MSFT", 0.27)];
        let targets = vec![
            tw("AAPL", 33.3), // +6.7% drift → alert
            tw("GOOG", 33.3), // -0.3% drift → no alert
            tw("MSFT", 33.3), // -6.3% drift → alert
        ];
        let alerts = check_weight_drift(&current, &targets, 5.0);
        assert_eq!(alerts.len(), 2, "Expected 2 alerts for 2 drifted positions");

        let tickers: Vec<&str> = alerts.iter().map(|a| a.ticker.as_str()).collect();
        assert!(tickers.contains(&"AAPL"));
        assert!(tickers.contains(&"MSFT"));
    }

    #[test]
    fn test_drift_calculation_accuracy() {
        // current = 0.312 → 31.2%, target = 25.0% → drift = 6.2%
        let current = vec![("XYZ", 0.312)];
        let targets = vec![tw("XYZ", 25.0)];
        let alerts = check_weight_drift(&current, &targets, 5.0);
        assert_eq!(alerts.len(), 1);
        assert!((alerts[0].current_weight_pct - 31.2).abs() < 1e-10);
        assert!((alerts[0].target_weight_pct - 25.0).abs() < f64::EPSILON);
        assert!((alerts[0].drift_pct - 6.2).abs() < 1e-10);
    }

    #[test]
    fn test_format_drift_alerts_contains_tickers() {
        let alerts = vec![
            DriftAlert {
                ticker: "AAPL".to_string(),
                current_weight_pct: 31.0,
                target_weight_pct: 25.0,
                drift_pct: 6.0,
                direction: DriftDirection::Over,
            },
            DriftAlert {
                ticker: "MSFT".to_string(),
                current_weight_pct: 18.0,
                target_weight_pct: 25.0,
                drift_pct: -7.0,
                direction: DriftDirection::Under,
            },
        ];
        let formatted = format_drift_alerts(&alerts);
        assert!(
            formatted.contains("AAPL"),
            "Formatted output should mention AAPL"
        );
        assert!(
            formatted.contains("MSFT"),
            "Formatted output should mention MSFT"
        );
        assert!(
            formatted.contains("OVER"),
            "Should indicate overweight direction"
        );
        assert!(
            formatted.contains("UNDER"),
            "Should indicate underweight direction"
        );
    }

    #[test]
    fn test_format_drift_alerts_empty() {
        let formatted = format_drift_alerts(&[]);
        assert!(
            formatted.contains("No rebalancing needed"),
            "Empty alerts should produce 'no rebalancing needed' message"
        );
    }
}
