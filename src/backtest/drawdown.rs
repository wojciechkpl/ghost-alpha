use chrono::NaiveDate;

use crate::backtest::types::DrawdownEvent;

/// Build a cumulative equity curve from a series of returns.
///
/// Each return is applied multiplicatively: equity[i+1] = equity[i] * (1 + return[i]).
pub fn build_equity_curve(
    returns: &[(NaiveDate, f64)],
    initial_capital: f64,
) -> Vec<(NaiveDate, f64)> {
    let mut curve = Vec::with_capacity(returns.len() + 1);
    if returns.is_empty() {
        return curve;
    }

    let mut equity = initial_capital;
    curve.push((returns[0].0, equity));

    for &(date, ret) in returns {
        equity *= 1.0 + ret;
        curve.push((date, equity));
    }

    curve
}

/// Identify all drawdown events from an equity curve.
///
/// A drawdown starts when equity drops below a running peak and ends when
/// equity recovers to or above that peak.
pub fn compute_drawdowns(equity_curve: &[(NaiveDate, f64)]) -> Vec<DrawdownEvent> {
    if equity_curve.len() < 2 {
        return Vec::new();
    }

    let mut drawdowns = Vec::new();
    let mut peak = equity_curve[0].1;
    let mut in_drawdown = false;
    let mut dd_start = equity_curve[0].0;
    let mut trough_val = peak;
    let mut trough_date = equity_curve[0].0;

    for &(date, value) in &equity_curve[1..] {
        if value >= peak {
            if in_drawdown {
                // Recovery — close this drawdown
                let depth = (trough_val - peak) / peak;
                let duration = (date - dd_start).num_days() as u32;
                drawdowns.push(DrawdownEvent {
                    start_date: dd_start,
                    trough_date,
                    recovery_date: Some(date),
                    depth,
                    duration_days: duration,
                });
                in_drawdown = false;
            }
            peak = value;
        } else {
            if !in_drawdown {
                // Start of a new drawdown
                in_drawdown = true;
                dd_start = date;
                trough_val = value;
                trough_date = date;
            }
            if value < trough_val {
                trough_val = value;
                trough_date = date;
            }
        }
    }

    // Handle ongoing drawdown at end of data
    if in_drawdown {
        let depth = (trough_val - peak) / peak;
        let last_date = equity_curve.last().unwrap().0;
        let duration = (last_date - dd_start).num_days() as u32;
        drawdowns.push(DrawdownEvent {
            start_date: dd_start,
            trough_date,
            recovery_date: None,
            depth,
            duration_days: duration,
        });
    }

    drawdowns
}

/// Maximum drawdown depth (most negative value, or 0.0 if no drawdowns).
pub fn max_drawdown_depth(drawdowns: &[DrawdownEvent]) -> f64 {
    drawdowns
        .iter()
        .map(|d| d.depth)
        .min_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap_or(0.0)
}

/// Maximum drawdown duration in days (or 0 if no drawdowns).
pub fn max_drawdown_duration(drawdowns: &[DrawdownEvent]) -> u32 {
    drawdowns.iter().map(|d| d.duration_days).max().unwrap_or(0)
}

/// Average number of days to recover across all recovered drawdowns.
pub fn avg_recovery_days(drawdowns: &[DrawdownEvent]) -> f64 {
    let recovered: Vec<&DrawdownEvent> = drawdowns
        .iter()
        .filter(|d| d.recovery_date.is_some())
        .collect();

    if recovered.is_empty() {
        return 0.0;
    }

    let total_days: u32 = recovered.iter().map(|d| d.duration_days).sum();
    total_days as f64 / recovered.len() as f64
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn monotone_up_no_drawdowns() {
        let curve: Vec<(NaiveDate, f64)> = (0..30)
            .map(|i| {
                (
                    date(2024, 1, 1) + chrono::Duration::days(i),
                    100.0 + i as f64,
                )
            })
            .collect();
        let dds = compute_drawdowns(&curve);
        assert!(dds.is_empty(), "Monotone uptrend should have no drawdowns");
    }

    #[test]
    fn single_dip_one_drawdown() {
        let curve = vec![
            (date(2024, 1, 1), 100.0),
            (date(2024, 1, 2), 95.0), // start drawdown
            (date(2024, 1, 3), 90.0), // trough
            (date(2024, 1, 4), 95.0),
            (date(2024, 1, 5), 100.0), // recovery
        ];
        let dds = compute_drawdowns(&curve);
        assert_eq!(dds.len(), 1);
        assert!(
            (dds[0].depth - (-0.10)).abs() < 1e-6,
            "Depth should be -10%, got {}",
            dds[0].depth
        );
        assert!(dds[0].recovery_date.is_some());
    }

    #[test]
    fn multiple_dips() {
        let curve = vec![
            (date(2024, 1, 1), 100.0),
            (date(2024, 1, 2), 95.0),  // dip 1
            (date(2024, 1, 3), 100.0), // recover
            (date(2024, 1, 4), 105.0), // new peak
            (date(2024, 1, 5), 100.0), // dip 2
            (date(2024, 1, 6), 105.0), // recover
        ];
        let dds = compute_drawdowns(&curve);
        assert_eq!(dds.len(), 2, "Should have 2 drawdowns");
    }

    #[test]
    fn max_depth() {
        let dds = vec![
            DrawdownEvent {
                start_date: date(2024, 1, 1),
                trough_date: date(2024, 1, 5),
                recovery_date: Some(date(2024, 1, 10)),
                depth: -0.05,
                duration_days: 9,
            },
            DrawdownEvent {
                start_date: date(2024, 2, 1),
                trough_date: date(2024, 2, 10),
                recovery_date: Some(date(2024, 2, 20)),
                depth: -0.15,
                duration_days: 19,
            },
        ];
        assert!((max_drawdown_depth(&dds) - (-0.15)).abs() < 1e-6);
    }

    #[test]
    fn max_duration() {
        let dds = vec![
            DrawdownEvent {
                start_date: date(2024, 1, 1),
                trough_date: date(2024, 1, 5),
                recovery_date: Some(date(2024, 1, 10)),
                depth: -0.05,
                duration_days: 9,
            },
            DrawdownEvent {
                start_date: date(2024, 2, 1),
                trough_date: date(2024, 2, 10),
                recovery_date: Some(date(2024, 2, 20)),
                depth: -0.15,
                duration_days: 19,
            },
        ];
        assert_eq!(max_drawdown_duration(&dds), 19);
    }

    #[test]
    fn avg_recovery() {
        let dds = vec![
            DrawdownEvent {
                start_date: date(2024, 1, 1),
                trough_date: date(2024, 1, 5),
                recovery_date: Some(date(2024, 1, 11)),
                depth: -0.05,
                duration_days: 10,
            },
            DrawdownEvent {
                start_date: date(2024, 2, 1),
                trough_date: date(2024, 2, 10),
                recovery_date: Some(date(2024, 2, 21)),
                depth: -0.10,
                duration_days: 20,
            },
        ];
        assert!((avg_recovery_days(&dds) - 15.0).abs() < 1e-6);
    }

    #[test]
    fn equity_curve_from_returns() {
        let returns = vec![
            (date(2024, 1, 1), 0.10),  // 10% gain
            (date(2024, 1, 2), -0.05), // 5% loss
            (date(2024, 1, 3), 0.03),  // 3% gain
        ];
        let curve = build_equity_curve(&returns, 10_000.0);
        // Initial: 10000
        // After +10%: 11000
        // After -5%: 10450
        // After +3%: 10763.5
        assert_eq!(curve.len(), 4);
        assert!((curve[0].1 - 10_000.0).abs() < 1e-6);
        assert!((curve[1].1 - 11_000.0).abs() < 1e-2);
        assert!((curve[2].1 - 10_450.0).abs() < 1e-2);
        assert!((curve[3].1 - 10_763.5).abs() < 1e-2);
    }

    #[test]
    fn hard_limits_check() {
        // Verify drawdown depth is always <= 0
        let curve = vec![
            (date(2024, 1, 1), 100.0),
            (date(2024, 1, 2), 80.0),
            (date(2024, 1, 3), 60.0),
        ];
        let dds = compute_drawdowns(&curve);
        for dd in &dds {
            assert!(
                dd.depth <= 0.0,
                "Drawdown depth should be <= 0, got {}",
                dd.depth
            );
        }
    }
}
