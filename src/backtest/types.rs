use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::data::models::MarketRegime;

// ─── Backtest Signal ─────────────────────────────────────────────────────────

/// A single signal emitted by the screener on a given date, enriched with
/// forward return data computed during backtesting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestSignal {
    pub ticker: String,
    pub signal_date: NaiveDate,
    pub entry_price: f64,
    pub composite_score: f64,
    pub regime: MarketRegime,
    pub forward_returns: ForwardReturns,
}

/// Forward returns measured at fixed horizons after the signal date.
/// `None` means the horizon extends beyond available data.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForwardReturns {
    pub ret_30d: Option<f64>,
    pub ret_60d: Option<f64>,
    pub ret_90d: Option<f64>,
}

// ─── Walk-Forward Period ─────────────────────────────────────────────────────

/// Results for a single walk-forward out-of-sample period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalkForwardPeriod {
    pub period_index: usize,
    pub is_start: NaiveDate,
    pub is_end: NaiveDate,
    pub oos_start: NaiveDate,
    pub oos_end: NaiveDate,
    pub oos_hit_rate: f64,
    pub oos_alpha: f64,
    pub oos_fp_rate: f64,
}

// ─── KPIs ────────────────────────────────────────────────────────────────────

/// Aggregate key performance indicators across all backtest signals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestKpis {
    pub total_signals: usize,
    pub hit_rate_30d: f64,
    pub hit_rate_60d: f64,
    pub hit_rate_90d: f64,
    pub fp_rate_60d: f64,
    pub alpha_60d: f64,
    pub win_loss_ratio: f64,
    pub profit_factor: f64,
    pub max_drawdown_depth: f64,
    pub max_drawdown_duration_days: u32,
    pub sharpe_ratio: f64,
}

/// KPIs broken down by market regime.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegimeKpis {
    pub regime: MarketRegime,
    pub signal_count: usize,
    pub hit_rate_60d: f64,
    pub alpha_60d: f64,
    pub fp_rate: f64,
}

// ─── Confidence Intervals ────────────────────────────────────────────────────

/// A Monte Carlo bootstrap confidence interval for a single metric.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceInterval {
    pub metric_name: String,
    pub point_estimate: f64,
    pub ci_lower: f64,
    pub ci_upper: f64,
}

impl ConfidenceInterval {
    /// Returns `true` if the CI excludes `null_value` (e.g., 0.0 for alpha).
    pub fn rejects_null(&self, null_value: f64) -> bool {
        null_value < self.ci_lower || null_value > self.ci_upper
    }
}

// ─── Drawdown ────────────────────────────────────────────────────────────────

/// A single drawdown episode in the equity curve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawdownEvent {
    pub start_date: NaiveDate,
    pub trough_date: NaiveDate,
    pub recovery_date: Option<NaiveDate>,
    pub depth: f64,
    pub duration_days: u32,
}

// ─── Report ──────────────────────────────────────────────────────────────────

/// The full backtest report combining all analyses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestReport {
    pub kpis: BacktestKpis,
    pub walk_forward_periods: Vec<WalkForwardPeriod>,
    pub regime_kpis: Vec<RegimeKpis>,
    pub confidence_intervals: Vec<ConfidenceInterval>,
    pub drawdowns: Vec<DrawdownEvent>,
    pub equity_curve: Vec<(NaiveDate, f64)>,
}

// ─── Params ──────────────────────────────────────────────────────────────────

/// Parameters controlling a backtest run, populated from `BacktestConfig`.
#[derive(Debug, Clone)]
pub struct BacktestParams {
    pub forward_windows: Vec<u32>,
    pub walk_forward_in_sample_months: u32,
    pub walk_forward_out_of_sample_months: u32,
    pub monte_carlo_iterations: u32,
    pub transaction_cost_bps: f64,
    pub holdout_pct: f64,
}

impl Default for BacktestParams {
    fn default() -> Self {
        Self {
            forward_windows: vec![30, 60, 90],
            walk_forward_in_sample_months: 12,
            walk_forward_out_of_sample_months: 3,
            monte_carlo_iterations: 10000,
            transaction_cost_bps: 10.0,
            holdout_pct: 0.2,
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_prd() {
        let params = BacktestParams::default();
        assert_eq!(params.forward_windows, vec![30, 60, 90]);
        assert_eq!(params.walk_forward_in_sample_months, 12);
        assert_eq!(params.walk_forward_out_of_sample_months, 3);
        assert_eq!(params.monte_carlo_iterations, 10000);
        assert!((params.transaction_cost_bps - 10.0).abs() < 1e-6);
    }

    #[test]
    fn forward_returns_default_none() {
        let fr = ForwardReturns::default();
        assert!(fr.ret_30d.is_none());
        assert!(fr.ret_60d.is_none());
        assert!(fr.ret_90d.is_none());
    }

    #[test]
    fn signal_regime_label() {
        let signal = BacktestSignal {
            ticker: "NVDA".to_string(),
            signal_date: NaiveDate::from_ymd_opt(2024, 6, 15).unwrap(),
            entry_price: 120.0,
            composite_score: 85.0,
            regime: MarketRegime::Bull,
            forward_returns: ForwardReturns::default(),
        };
        assert_eq!(signal.regime.to_string(), "BULL");
    }

    #[test]
    fn ci_contains_point_estimate() {
        let ci = ConfidenceInterval {
            metric_name: "hit_rate_60d".to_string(),
            point_estimate: 0.65,
            ci_lower: 0.55,
            ci_upper: 0.75,
        };
        assert!(ci.point_estimate >= ci.ci_lower);
        assert!(ci.point_estimate <= ci.ci_upper);
    }

    #[test]
    fn drawdown_duration() {
        let dd = DrawdownEvent {
            start_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            trough_date: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            recovery_date: Some(NaiveDate::from_ymd_opt(2024, 3, 1).unwrap()),
            depth: -0.15,
            duration_days: 60,
        };
        assert_eq!(dd.duration_days, 60);
        assert!(dd.depth < 0.0);
    }
}
