# Phase 6: Adaptive Market Engine — PRD Section

> **Status**: Design complete, not yet implemented
> **Depends on**: Phases 1-5 (all complete)
> **Feature range**: F20-F25
> **Estimated scope**: ~3,500 LOC across 6 new modules + extensions to 4 existing modules

---

## Executive Summary

The Adaptive Market Engine transforms GhostAlpha from a fixed-parameter screening system into a self-calibrating system that adjusts strategy selection, parameter tuning, position sizing, and signal presentation based on the prevailing market microstructure. It operates across three time horizons (Swing/Position/Trend) and uses a Bayesian feedback loop to update strategy weights from realized performance.

The core principle: **the engine adapts slowly and conservatively, using guardrails to prevent overfitting, while retaining full auditability and determinism.**

---

## Architecture Overview

```
                 ┌──────────────────────────────────┐
                 │       MarketMicrostructure        │  F22
                 │  (VolRegime, CorrRegime, LiqCal)  │
                 └─────────────┬────────────────────┘
                               │
            ┌──────────────────▼──────────────────┐
            │        AdaptiveEngine (F20)          │
            │                                      │
            │  ┌─────────┐  ┌──────────┐  ┌─────┐ │
            │  │TimeframeSelector│  │ParamTuner │  │Bayesian│ │
            │  │  (F20)  │  │  (F21)   │  │(F24) │ │
            │  └────┬────┘  └────┬─────┘  └──┬──┘ │
            │       │            │            │    │
            └───────┼────────────┼────────────┼────┘
                    │            │            │
        ┌───────────▼─┐   ┌─────▼──────┐  ┌─▼──────────────┐
        │ MultiHorizon │   │  Screener  │  │  Performance   │
        │  Scorer (F23)│   │  (existing)│  │  Attribution   │
        │              │   │            │  │    (F25)       │
        └──────────────┘   └────────────┘  └────────────────┘
```

New modules:
- `src/analysis/adaptive.rs` — AdaptiveEngine orchestrator (F20)
- `src/analysis/param_tuner.rs` — Adaptive parameter tuning (F21)
- `src/analysis/microstructure.rs` — Volatility/correlation/liquidity regimes (F22)
- `src/analysis/multi_horizon.rs` — Multi-timeframe scoring (F23)
- `src/analysis/bayesian_weights.rs` — Bayesian strategy weight updating (F24)
- `src/analysis/attribution.rs` — Performance attribution (F25)

Extended modules:
- `src/analysis/screener.rs` — `Screener` trait gains `preferred_horizon()` method
- `src/analysis/ensemble.rs` — `EnsembleResult` extended with per-horizon scores
- `src/backtest/types.rs` — New `AdaptiveBacktestSignal` with multi-horizon returns
- `src/web/dashboard_routes.rs` — Adaptive engine dashboard tab

---

## F20: Multi-Timeframe Strategy Selection

### Problem

All four screeners currently run with equal emphasis regardless of market conditions. In reality:
- **Breakout** excels in low-vol Bull markets with healthy breadth
- **MeanReversion** excels in Sideways markets with sector rotation
- **TrendStrength** excels in strong Bull markets with momentum persistence
- **Minervini** is the universal workhorse but can be improved with timeframe awareness

### Design

#### 20.1 Time Horizon Definitions

```rust
/// src/analysis/multi_horizon.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimeHorizon {
    /// Swing: 10-20 trading days (2-4 weeks)
    Swing,
    /// Position: 40-120 trading days (2-6 months)
    Position,
    /// Trend: 120-252 trading days (6-12 months)
    Trend,
}

impl TimeHorizon {
    pub fn holding_days(&self) -> (u32, u32) {
        match self {
            TimeHorizon::Swing => (10, 20),
            TimeHorizon::Position => (40, 120),
            TimeHorizon::Trend => (120, 252),
        }
    }

    /// Midpoint horizon for forward return calculation.
    pub fn target_days(&self) -> u32 {
        match self {
            TimeHorizon::Swing => 15,
            TimeHorizon::Position => 60,
            TimeHorizon::Trend => 180,
        }
    }
}
```

#### 20.2 Strategy-Horizon Affinity Matrix

Each screener has a natural affinity for specific horizons. This is a **fixed** matrix (not adaptive) because it reflects the structural design of each strategy.

```rust
/// Static affinity: how well each strategy fits each horizon.
/// Values in [0.0, 1.0]. Sum across horizons per strategy need not equal 1.
pub struct StrategyHorizonAffinity {
    pub strategy: &'static str,
    pub swing: f64,
    pub position: f64,
    pub trend: f64,
}

pub const AFFINITY_MATRIX: &[StrategyHorizonAffinity] = &[
    StrategyHorizonAffinity { strategy: "minervini",      swing: 0.3, position: 0.8, trend: 0.6 },
    StrategyHorizonAffinity { strategy: "breakout",       swing: 0.9, position: 0.5, trend: 0.2 },
    StrategyHorizonAffinity { strategy: "mean-reversion", swing: 0.8, position: 0.4, trend: 0.1 },
    StrategyHorizonAffinity { strategy: "trend-strength", swing: 0.2, position: 0.6, trend: 0.9 },
];
```

#### 20.3 Timeframe Effectiveness Detection

The engine determines which timeframe is "working" by measuring recent forward returns at each horizon using a rolling window.

```rust
/// Which timeframe is producing the best risk-adjusted signals right now?
///
/// Computed from the last `lookback_days` of backtest signals.
/// Uses horizon-adjusted Sharpe (F4.4 in backtest engine).
pub struct TimeframeEffectiveness {
    pub swing_sharpe: f64,    // Sharpe of 15-day forward returns
    pub position_sharpe: f64, // Sharpe of 60-day forward returns
    pub trend_sharpe: f64,    // Sharpe of 180-day forward returns
    pub as_of: NaiveDate,
}

impl TimeframeEffectiveness {
    /// Softmax allocation: convert Sharpe ratios to weights.
    /// Temperature parameter controls concentration.
    /// temperature = 1.0: moderate concentration
    /// temperature = 0.5: aggressive concentration on best horizon
    /// temperature = 2.0: near-equal weights
    pub fn to_weights(&self, temperature: f64) -> TimeframeWeights {
        let raw = [self.swing_sharpe, self.position_sharpe, self.trend_sharpe];
        let max_val = raw.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

        // Guard: if all Sharpes <= 0, return equal weights
        if max_val <= 0.0 {
            return TimeframeWeights {
                swing: 1.0 / 3.0,
                position: 1.0 / 3.0,
                trend: 1.0 / 3.0,
            };
        }

        let exp_vals: Vec<f64> = raw.iter()
            .map(|s| ((s - max_val) / temperature).exp())
            .collect();
        let sum: f64 = exp_vals.iter().sum();

        TimeframeWeights {
            swing:    (exp_vals[0] / sum).clamp(0.10, 0.60),
            position: (exp_vals[1] / sum).clamp(0.10, 0.60),
            trend:    (exp_vals[2] / sum).clamp(0.10, 0.60),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeframeWeights {
    pub swing: f64,
    pub position: f64,
    pub trend: f64,
}
```

**Guardrails:**
- Minimum weight per horizon: 10% (prevents total abandonment of any timeframe)
- Maximum weight per horizon: 60% (prevents over-concentration)
- Softmax temperature: configurable, default 1.0
- Lookback for effectiveness: 63 trading days (1 quarter), requiring at least 20 signals per horizon

#### 20.4 Composite Strategy Selection

Given the current regime (F9), microstructure state (F22), and timeframe effectiveness (F20.3), compute a blended strategy weight vector:

```rust
/// Final strategy weights for the current market state.
///
/// weight[strategy] = sum over horizons of:
///   affinity[strategy][horizon] * horizon_weight[horizon] * regime_modifier[strategy][regime]
///
/// Then normalize to sum to 1.0.
pub fn compute_strategy_weights(
    regime: &MarketRegime,
    microstructure: &MicrostructureState,
    tf_weights: &TimeframeWeights,
) -> StrategyWeights {
    // Regime modifiers: how much to favor each strategy per regime
    let regime_mod: HashMap<&str, f64> = match regime {
        MarketRegime::Bull => [
            ("minervini", 1.0), ("breakout", 1.2),
            ("mean-reversion", 0.6), ("trend-strength", 1.1),
        ].into(),
        MarketRegime::Sideways => [
            ("minervini", 1.0), ("breakout", 0.7),
            ("mean-reversion", 1.3), ("trend-strength", 0.8),
        ].into(),
        MarketRegime::Bear => [
            ("minervini", 0.8), ("breakout", 0.3),
            ("mean-reversion", 1.0), ("trend-strength", 0.5),
        ].into(),
    };

    // Volatility override: in high-vol regimes, boost mean-reversion, reduce breakout
    let vol_adj: HashMap<&str, f64> = match microstructure.vol_regime {
        VolRegime::Low =>    [("breakout", 1.2), ("mean-reversion", 0.8), ("trend-strength", 1.0), ("minervini", 1.0)].into(),
        VolRegime::Normal => [("breakout", 1.0), ("mean-reversion", 1.0), ("trend-strength", 1.0), ("minervini", 1.0)].into(),
        VolRegime::High =>   [("breakout", 0.6), ("mean-reversion", 1.3), ("trend-strength", 0.7), ("minervini", 0.9)].into(),
    };

    let mut weights = HashMap::new();
    for entry in AFFINITY_MATRIX {
        let horizon_score =
            entry.swing * tf_weights.swing +
            entry.position * tf_weights.position +
            entry.trend * tf_weights.trend;
        let regime_m = regime_mod.get(entry.strategy).unwrap_or(&1.0);
        let vol_m = vol_adj.get(entry.strategy).unwrap_or(&1.0);
        weights.insert(entry.strategy.to_string(), horizon_score * regime_m * vol_m);
    }

    // Normalize
    let total: f64 = weights.values().sum();
    if total > 0.0 {
        for v in weights.values_mut() {
            *v /= total;
        }
    }

    StrategyWeights(weights)
}
```

**Output format for user:**
```
★ STRATEGY BLEND (Bull + Low Vol + Position Timeframe Working):
  Breakout:       38% (swing affinity + vol boost)
  Minervini:      28% (universal workhorse)
  TrendStrength:  22% (trend affinity)
  MeanReversion:  12% (reduced — not ideal for bull)
```

---

## F21: Adaptive Parameter Tuning

### Problem

Minervini's 8 conditions use fixed thresholds (e.g., RS > 70th percentile, ADX > 25). These work well on average but could improve by tightening in bull markets and loosening in bear markets.

### Design Philosophy

**Adapt slowly. Constrain tightly. Audit everything.**

Split parameters into three tiers:
1. **Never adapt** (structural): Number of Minervini conditions (8), SMA periods (50/150/200), ADX period (14)
2. **Regime-switch only** (discrete): Scoring weight profiles already done in F9
3. **Rolling calibration** (continuous): RS threshold, ADX threshold, volume Z-score threshold, consolidation depth

#### 21.1 Adaptive Parameters

```rust
/// src/analysis/param_tuner.rs

/// Parameters that can be adapted within guardrails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdaptiveParams {
    /// RS percentile threshold for Minervini condition #8.
    /// Fixed: 70.0. Adaptive range: [60.0, 85.0].
    pub rs_threshold: f64,

    /// ADX threshold for TrendStrength "strong trend" condition.
    /// Fixed: 30.0. Adaptive range: [22.0, 40.0].
    pub adx_strong_threshold: f64,

    /// ADX threshold for Breakout "trending" condition.
    /// Fixed: 20.0. Adaptive range: [15.0, 28.0].
    pub adx_trending_threshold: f64,

    /// Volume surge ratio for Breakout volume condition.
    /// Fixed: 1.5. Adaptive range: [1.2, 2.5].
    pub volume_surge_ratio: f64,

    /// Consolidation max depth for Breakout base detection.
    /// Fixed: 0.15. Adaptive range: [0.08, 0.25].
    pub consolidation_max_depth: f64,

    /// RSI oversold threshold for MeanReversion.
    /// Fixed: 30.0. Adaptive range: [20.0, 40.0].
    pub rsi_oversold_threshold: f64,

    /// Calibration metadata.
    pub calibrated_at: NaiveDate,
    pub calibration_window_days: u32,
    pub regime_at_calibration: MarketRegime,
}

/// Guardrails for each parameter.
pub struct ParamGuardrail {
    pub name: &'static str,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// Maximum change per calibration cycle (prevents jumps).
    pub max_step: f64,
}

pub const GUARDRAILS: &[ParamGuardrail] = &[
    ParamGuardrail { name: "rs_threshold",           min: 60.0, max: 85.0, default: 70.0, max_step: 5.0 },
    ParamGuardrail { name: "adx_strong_threshold",   min: 22.0, max: 40.0, default: 30.0, max_step: 3.0 },
    ParamGuardrail { name: "adx_trending_threshold",  min: 15.0, max: 28.0, default: 20.0, max_step: 3.0 },
    ParamGuardrail { name: "volume_surge_ratio",      min: 1.2,  max: 2.5,  default: 1.5,  max_step: 0.2 },
    ParamGuardrail { name: "consolidation_max_depth", min: 0.08, max: 0.25, default: 0.15, max_step: 0.03 },
    ParamGuardrail { name: "rsi_oversold_threshold",  min: 20.0, max: 40.0, default: 30.0, max_step: 5.0 },
];
```

#### 21.2 Rolling Calibration Algorithm

Calibration runs weekly (or on-demand via `ghost-alpha calibrate`). It uses a walk-forward approach on the last N trading days to find the parameter set that maximizes hit rate while maintaining minimum signal count.

```rust
/// Calibrate adaptive parameters using grid search over guardrail ranges.
///
/// Algorithm:
/// 1. Take the last `window_days` of historical data (default: 252 = 1 year)
/// 2. Split 80% in-sample, 20% out-of-sample (chronological, no shuffle)
/// 3. For each parameter, test 5 values evenly spaced within guardrail range
/// 4. Score: hit_rate_60d × min(1.0, signal_count / 20)
///    (penalty for producing fewer than 20 signals — prevents overfitting to rare events)
/// 5. Select the parameter set maximizing OOS score
/// 6. Apply max_step constraint: new value cannot differ from current by more than max_step
/// 7. Log the calibration event with full audit trail
///
/// Returns the calibrated params, or the existing defaults if calibration fails.
pub fn calibrate(
    historical_signals: &[BacktestSignal],
    current_params: &AdaptiveParams,
    window_days: u32,
) -> AdaptiveParams {
    // Implementation uses 5-point grid per parameter, not full grid search
    // (6 params × 5 values each = 15,625 combos if full grid — too expensive).
    // Instead: coordinate descent, one parameter at a time, 3 passes.
    // Deterministic: no randomness, fixed grid points.
    todo!()
}
```

**Anti-overfitting measures:**
1. **Walk-forward validation**: Never test on in-sample data
2. **Signal count penalty**: Score decays if parameter set produces fewer than 20 OOS signals
3. **Max step size**: Parameters cannot jump more than `max_step` per calibration cycle
4. **Guardrail clamps**: Hard min/max boundaries that cannot be exceeded
5. **Regime lock**: Calibration results are tagged with the regime they were calibrated in; if regime changes, params reset to defaults for the new regime (with 1 calibration cycle of warm-up)
6. **Staleness detection**: If last calibration is > 30 days old, log a warning and suggest re-calibration

---

## F22: Market Microstructure Awareness

### Problem

The existing regime detector (Bull/Sideways/Bear) captures the directional bias but misses volatility structure, cross-asset correlation, and calendar effects that materially impact which strategies work.

### Design

#### 22.1 Volatility Regime Classification

```rust
/// src/analysis/microstructure.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VolRegime {
    /// VIX < 15 AND 20-day realized vol < 10%
    Low,
    /// Default state
    Normal,
    /// VIX > 25 OR 20-day realized vol > 20%
    High,
}

/// Classify the current volatility regime.
///
/// Uses both implied vol (VIX) and realized vol (20-day close-to-close).
/// High regime requires either condition. Low regime requires both conditions.
pub fn classify_vol_regime(vix: f64, realized_vol_20d: f64) -> VolRegime {
    if vix > 25.0 || realized_vol_20d > 0.20 {
        VolRegime::High
    } else if vix < 15.0 && realized_vol_20d < 0.10 {
        VolRegime::Low
    } else {
        VolRegime::Normal
    }
}

/// 20-day close-to-close annualized realized volatility.
///
/// rv = std(ln(c[i]/c[i-1])) * sqrt(252)
pub fn realized_vol(closes: &[f64], window: usize) -> Option<f64> {
    if closes.len() < window + 1 {
        return None;
    }
    let returns: Vec<f64> = closes[closes.len() - window..]
        .windows(2)
        .map(|w| (w[1] / w[0]).ln())
        .collect();
    let n = returns.len() as f64;
    let mean = returns.iter().sum::<f64>() / n;
    let variance = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
    Some(variance.sqrt() * (252.0_f64).sqrt())
}
```

#### 22.2 Correlation Regime Classification

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CorrRegime {
    /// Average pairwise correlation > 0.70 — "risk on/off" market.
    /// Breakout and MeanReversion suffer; TrendStrength works if right direction.
    HighCorrelation,
    /// Average pairwise correlation 0.30-0.70 — normal stock-picking environment.
    Normal,
    /// Average pairwise correlation < 0.30 — sector rotation, stock picker's market.
    /// All strategies can work. Diversification benefits are real.
    LowCorrelation,
}

/// Classify correlation regime from a sample of sector ETF returns.
///
/// Uses 11 SPDR sector ETFs (XLB, XLC, XLE, XLF, XLI, XLK, XLP, XLRE, XLU, XLV, XLY).
/// Computes average of all 55 pairwise 63-day rolling correlations.
///
/// Thresholds chosen from empirical distribution:
/// - High: avg_corr > 0.70 (occurs ~15% of the time historically, typically crisis periods)
/// - Low: avg_corr < 0.30 (occurs ~20% of the time, good stock-picking environments)
/// - Normal: everything else
pub fn classify_corr_regime(
    sector_returns: &[(&str, &[f64])], // (etf_ticker, daily_returns)
    window: usize,                      // default: 63
) -> CorrRegime {
    let mut total_corr = 0.0;
    let mut pair_count = 0usize;

    for i in 0..sector_returns.len() {
        for j in (i + 1)..sector_returns.len() {
            let a = &sector_returns[i].1;
            let b = &sector_returns[j].1;
            let a_tail = &a[a.len().saturating_sub(window)..];
            let b_tail = &b[b.len().saturating_sub(window)..];
            if a_tail.len() == b_tail.len() && a_tail.len() >= 20 {
                if let Ok(corr) = crate::analysis::correlation::pearson_correlation(a_tail, b_tail) {
                    total_corr += corr;
                    pair_count += 1;
                }
            }
        }
    }

    if pair_count == 0 {
        return CorrRegime::Normal;
    }

    let avg_corr = total_corr / pair_count as f64;

    if avg_corr > 0.70 {
        CorrRegime::HighCorrelation
    } else if avg_corr < 0.30 {
        CorrRegime::LowCorrelation
    } else {
        CorrRegime::Normal
    }
}
```

#### 22.3 Liquidity Calendar

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiquidityEvent {
    /// Quarterly options/futures expiry (3rd Friday of Mar/Jun/Sep/Dec)
    QuadWitching,
    /// FOMC announcement date (8 meetings per year)
    Fomc,
    /// Week of heavy earnings (>10% of S&P500 by market cap reporting)
    EarningsSeason,
    /// No significant event
    Normal,
}

/// Check if a given date falls near a known liquidity event.
///
/// FOMC dates: hardcoded for current year + fetched from FRED schedule.
/// Quad witching: 3rd Friday of March, June, September, December.
/// Earnings season: Jan 15-Feb 15, Apr 15-May 15, Jul 15-Aug 15, Oct 15-Nov 15.
///
/// Returns the highest-priority event if multiple overlap.
pub fn check_liquidity_event(date: NaiveDate) -> LiquidityEvent {
    use chrono::Datelike;

    // Quad witching: 3rd Friday of month 3, 6, 9, 12
    let month = date.month();
    if [3, 6, 9, 12].contains(&month) {
        let first_of_month = NaiveDate::from_ymd_opt(date.year(), month, 1).unwrap();
        let first_friday_offset = (5 - first_of_month.weekday().num_days_from_monday() + 7) % 7;
        let third_friday = first_of_month + chrono::Duration::days(first_friday_offset as i64 + 14);
        // Within 2 trading days of quad witching
        if (date - third_friday).num_days().abs() <= 2 {
            return LiquidityEvent::QuadWitching;
        }
    }

    // Earnings season approximation
    if ([1, 4, 7, 10].contains(&month) && date.day() >= 15)
        || ([2, 5, 8, 11].contains(&month) && date.day() <= 15)
    {
        return LiquidityEvent::EarningsSeason;
    }

    LiquidityEvent::Normal
}
```

#### 22.4 Composite Microstructure State

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MicrostructureState {
    pub vol_regime: VolRegime,
    pub corr_regime: CorrRegime,
    pub liquidity_event: LiquidityEvent,
    pub realized_vol_20d: f64,
    pub avg_sector_correlation: f64,
    pub as_of: NaiveDate,
}

impl MicrostructureState {
    /// Whether the current state favors aggressive positioning.
    /// Low vol + low correlation + normal calendar = favorable.
    pub fn favorability_score(&self) -> f64 {
        let vol_score = match self.vol_regime {
            VolRegime::Low => 1.0,
            VolRegime::Normal => 0.6,
            VolRegime::High => 0.2,
        };
        let corr_score = match self.corr_regime {
            CorrRegime::LowCorrelation => 1.0,
            CorrRegime::Normal => 0.6,
            CorrRegime::HighCorrelation => 0.3,
        };
        let liq_score = match self.liquidity_event {
            LiquidityEvent::Normal => 1.0,
            LiquidityEvent::EarningsSeason => 0.7,
            LiquidityEvent::Fomc => 0.5,
            LiquidityEvent::QuadWitching => 0.4,
        };
        // Geometric mean so all three dimensions matter
        (vol_score * corr_score * liq_score).powf(1.0 / 3.0)
    }
}
```

---

## F23: Multi-Horizon Scoring & Presentation

### Problem

A stock might score 85 as a swing trade (near breakout pivot, high volume) but only 50 as a trend trade (mediocre long-term RS). The current system collapses this into a single composite score, losing actionable information.

### Design

#### 23.1 Horizon-Specific Scoring

```rust
/// src/analysis/multi_horizon.rs

/// Scoring weights that differ by time horizon.
/// Short-horizon rewards momentum and volume. Long-horizon rewards trend quality and RS.
pub fn horizon_weights(horizon: TimeHorizon) -> RegimeWeights {
    match horizon {
        // Swing: volume matters most (breakout confirmation), sentiment is noise
        TimeHorizon::Swing => RegimeWeights {
            technical: 40.0,
            volume: 45.0,
            sentiment: 15.0,
        },
        // Position: balanced (current default)
        TimeHorizon::Position => RegimeWeights {
            technical: 55.0,
            volume: 30.0,
            sentiment: 15.0,
        },
        // Trend: technical quality dominates, volume less important
        TimeHorizon::Trend => RegimeWeights {
            technical: 70.0,
            volume: 15.0,
            sentiment: 15.0,
        },
    }
}

/// Extended screen result with per-horizon scores.
#[derive(Debug, Clone, Serialize)]
pub struct MultiHorizonScore {
    pub ticker: String,
    pub swing_score: f64,
    pub position_score: f64,
    pub trend_score: f64,
    /// Weighted blend using current TimeframeWeights.
    pub blended_score: f64,
    /// Best horizon for this stock.
    pub best_horizon: TimeHorizon,
    /// Horizon-specific position sizing recommendation.
    pub swing_size_pct: f64,
    pub position_size_pct: f64,
    pub trend_size_pct: f64,
}
```

#### 23.2 Horizon-Specific Scoring Adjustments

Beyond weight shifts, each horizon applies different indicator emphasis:

```
Swing scoring adjustments:
  - RSI-based timing bonus: +5 pts if RSI(14) in [30, 50] (oversold bounce setup)
  - Volume recency bonus: weight last 5 days of volume 2x relative to 20-day average
  - VCP detection bonus: +8 pts if VCP detected with pivot proximity < 3%
  - ATR penalty: if ATR > 5% of price, reduce score by 10 pts (too volatile for swing)

Position scoring adjustments:
  - Use existing composite scoring (this is the default horizon)
  - RS momentum bonus: +5 pts if rs_momentum > 2.0
  - Earnings proximity penalty: -5 pts if earnings within 10 trading days

Trend scoring adjustments:
  - RS percentile bonus: +10 pts if RS > 90th percentile (long-term leaders)
  - Fundamental quality bonus: if fundamental data available, +5 pts for ROE > 15%
  - Sector leadership bonus: +5 pts if stock's sector is in top 3 by 6-month return
  - Higher ADX requirement: -10 pts if ADX < 25 (weak trends fail over 6+ months)
```

#### 23.3 Position Sizing by Horizon

```rust
/// Horizon-adjusted position sizing.
///
/// Swing trades: smaller positions (max 3% of portfolio), tighter stops (1.5× ATR).
/// Position trades: standard sizing (max 8% of portfolio), 2× ATR stops.
/// Trend trades: larger positions (max 12% of portfolio), 3× ATR stops (room to breathe).
pub fn horizon_sizing_params(horizon: TimeHorizon) -> (f64, f64, f64) {
    // Returns: (max_position_pct, stop_atr_multiplier, risk_per_trade_pct)
    match horizon {
        TimeHorizon::Swing    => (0.03, 1.5, 0.005),  // 3% max, 1.5x ATR stop, 0.5% risk
        TimeHorizon::Position => (0.08, 2.0, 0.010),  // 8% max, 2x ATR stop, 1.0% risk
        TimeHorizon::Trend    => (0.12, 3.0, 0.015),  // 12% max, 3x ATR stop, 1.5% risk
    }
}
```

#### 23.4 Dashboard Presentation

The web dashboard displays multi-horizon results in a table:

```
Ticker  │ Swing │ Position │ Trend  │ Best     │ Blend  │ Grade
────────┼───────┼──────────┼────────┼──────────┼────────┼──────
NVDA    │  72   │   88     │   91   │ Trend    │  85.3  │ A+
CRWD    │  89   │   75     │   62   │ Swing    │  76.8  │ A
AMZN    │  45   │   82     │   84   │ Trend    │  74.1  │ B
PANW    │  91   │   68     │   55   │ Swing    │  72.5  │ B
```

Color coding: Green if best horizon matches the dominant timeframe weight. Amber if second-best. Red if worst horizon is dominant.

---

## F24: Bayesian Strategy Weight Updating

### Problem

The regime-based strategy weights in F20 are reasonable priors, but they should update based on observed performance. A strategy that has been producing 70% hit rate in the current regime deserves more weight than one producing 45%.

### Design

#### 24.1 Bayesian Framework

```rust
/// src/analysis/bayesian_weights.rs

/// Beta distribution parameters for each strategy's hit rate.
/// Prior: Beta(alpha=2, beta=2) = uniform-ish prior centered at 50%.
/// Each realized win adds 1 to alpha; each loss adds 1 to beta.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyBelief {
    pub strategy: String,
    pub alpha: f64,  // pseudo-count of successes
    pub beta: f64,   // pseudo-count of failures
    pub regime: MarketRegime,
    pub last_updated: NaiveDate,
}

impl StrategyBelief {
    /// Prior: weakly informative Beta(2, 2).
    pub fn new_prior(strategy: &str, regime: &MarketRegime) -> Self {
        Self {
            strategy: strategy.to_string(),
            alpha: 2.0,
            beta: 2.0,
            regime: regime.clone(),
            last_updated: chrono::Local::now().date_naive(),
        }
    }

    /// Posterior mean: E[p] = alpha / (alpha + beta).
    pub fn expected_hit_rate(&self) -> f64 {
        self.alpha / (self.alpha + self.beta)
    }

    /// Posterior variance.
    pub fn variance(&self) -> f64 {
        let n = self.alpha + self.beta;
        (self.alpha * self.beta) / (n * n * (n + 1.0))
    }

    /// 95% credible interval width (approximation).
    pub fn uncertainty(&self) -> f64 {
        2.0 * 1.96 * self.variance().sqrt()
    }

    /// Update with a batch of observed outcomes.
    /// `outcomes`: true = signal was profitable at 60-day horizon, false = not.
    ///
    /// Applies exponential decay to old evidence so the belief adapts over time.
    /// decay_factor = 0.995 per observation means a half-life of ~138 observations.
    pub fn update_batch(&mut self, outcomes: &[bool], decay_factor: f64) {
        // Decay existing evidence
        self.alpha *= decay_factor.powi(outcomes.len() as i32);
        self.beta *= decay_factor.powi(outcomes.len() as i32);

        // Add new evidence
        let wins = outcomes.iter().filter(|&&o| o).count() as f64;
        let losses = outcomes.len() as f64 - wins;
        self.alpha += wins;
        self.beta += losses;

        // Floor to prevent prior from decaying to nothing
        self.alpha = self.alpha.max(1.0);
        self.beta = self.beta.max(1.0);

        self.last_updated = chrono::Local::now().date_naive();
    }
}
```

#### 24.2 Weight Conversion

```rust
/// Convert Bayesian beliefs into strategy weights.
///
/// Uses Thompson sampling insight: weight proportional to
/// expected_hit_rate × (1 - uncertainty), so strategies with
/// wide confidence intervals get less weight.
///
/// Then blend with the regime-based prior weights (F20.4) using
/// blend_factor to control how much the Bayesian update matters.
///
/// blend_factor = 0.0: pure regime-based weights (ignores performance data)
/// blend_factor = 1.0: pure Bayesian weights (ignores regime priors)
/// Default: 0.4 (performance data provides 40% of signal, regime priors 60%)
pub fn bayesian_strategy_weights(
    beliefs: &[StrategyBelief],
    prior_weights: &StrategyWeights,
    blend_factor: f64,
) -> StrategyWeights {
    let blend = blend_factor.clamp(0.0, 1.0);

    let mut bayes_weights: HashMap<String, f64> = HashMap::new();
    for belief in beliefs {
        let confidence_adjusted = belief.expected_hit_rate() * (1.0 - belief.uncertainty().min(0.5));
        bayes_weights.insert(belief.strategy.clone(), confidence_adjusted.max(0.01));
    }

    // Normalize Bayesian weights
    let total: f64 = bayes_weights.values().sum();
    if total > 0.0 {
        for v in bayes_weights.values_mut() {
            *v /= total;
        }
    }

    // Blend with priors
    let mut final_weights = HashMap::new();
    for (strategy, prior_w) in &prior_weights.0 {
        let bayes_w = bayes_weights.get(strategy).unwrap_or(&0.25);
        let blended = (1.0 - blend) * prior_w + blend * bayes_w;
        final_weights.insert(strategy.clone(), blended);
    }

    // Re-normalize
    let final_total: f64 = final_weights.values().sum();
    if final_total > 0.0 {
        for v in final_weights.values_mut() {
            *v /= final_total;
        }
    }

    StrategyWeights(final_weights)
}
```

**Guardrails:**
- No strategy weight can drop below 5% (prevents total abandonment based on a bad streak)
- No strategy weight can exceed 60% (prevents over-reliance on one approach)
- Beliefs reset to prior when regime changes (different regime = different game)
- Minimum 20 observations before Bayesian weights influence blending (cold start protection)
- `blend_factor` default: 0.4 (conservative — the regime prior gets 60% influence)

#### 24.3 Persistence

Beliefs are stored in SQLite alongside the existing `regime_history` table:

```sql
CREATE TABLE IF NOT EXISTS strategy_beliefs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    strategy    TEXT NOT NULL,
    regime      TEXT NOT NULL,
    alpha       REAL NOT NULL,
    beta        REAL NOT NULL,
    updated_at  TEXT NOT NULL,
    UNIQUE(strategy, regime)
);

CREATE TABLE IF NOT EXISTS calibration_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    calibrated_at   TEXT NOT NULL,
    regime          TEXT NOT NULL,
    params_json     TEXT NOT NULL,
    oos_hit_rate    REAL,
    oos_signal_count INTEGER,
    notes           TEXT
);
```

---

## F25: Performance Attribution

### Problem

When the engine produces alpha, the user needs to know *where* it came from: was it stock selection, market timing, strategy weighting, position sizing, or the parameter tuning? Without attribution, there is no way to identify which parts of the system are working and which are dead weight.

### Design

#### 25.1 Attribution Components

```rust
/// src/analysis/attribution.rs

/// Performance attribution decomposition.
///
/// Total return = Selection + Timing + Sizing + Strategy + Interaction
///
/// - Selection: Alpha from picking stocks that outperform equal-weighted universe
/// - Timing:    Alpha from regime-aware entry/exit timing vs always-invested
/// - Sizing:    Alpha from position sizing vs equal-weight all positions
/// - Strategy:  Alpha from strategy weighting vs equal-weight all strategies
/// - Interaction: Residual (cross-term)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributionResult {
    pub period_start: NaiveDate,
    pub period_end: NaiveDate,

    /// Total portfolio return over the period.
    pub total_return: f64,
    /// Benchmark (SPY buy-and-hold) return over the period.
    pub benchmark_return: f64,
    /// Active return = total - benchmark.
    pub active_return: f64,

    /// Stock selection effect:
    /// sum over positions of: (stock_return - benchmark_return) × benchmark_weight
    pub selection_effect: f64,

    /// Timing effect:
    /// Return difference from being invested only when signals are active
    /// vs being always invested. Measures regime filter value.
    pub timing_effect: f64,

    /// Sizing effect:
    /// Return difference from actual position sizes vs equal-weighting all signals.
    pub sizing_effect: f64,

    /// Strategy weighting effect:
    /// Return difference from adaptive strategy weights vs equal-weighting all strategies.
    pub strategy_effect: f64,

    /// Interaction / residual.
    pub interaction_effect: f64,
}
```

#### 25.2 Attribution Algorithm (Brinson-Fachler Variant)

```rust
/// Compute attribution for a completed backtest period.
///
/// Requires:
/// - `actual_trades`: the trades the engine recommended with actual weights and returns
/// - `benchmark_returns`: SPY daily returns over the period
/// - `equal_weight_returns`: returns if all signals were equal-weighted (no sizing alpha)
/// - `equal_strategy_returns`: returns if all strategies had equal weight
pub fn compute_attribution(
    actual_trades: &[TradeRecord],
    benchmark_daily: &[f64],
    period_start: NaiveDate,
    period_end: NaiveDate,
) -> AttributionResult {
    let total_return = compound_returns(
        &actual_trades.iter().map(|t| t.realized_return).collect::<Vec<_>>()
    );
    let benchmark_return = compound_returns(benchmark_daily);
    let active_return = total_return - benchmark_return;

    // Selection: average stock return - benchmark return, weighted by benchmark weights
    let avg_stock_return = actual_trades.iter()
        .map(|t| t.realized_return)
        .sum::<f64>() / actual_trades.len().max(1) as f64;
    let selection_effect = avg_stock_return - benchmark_return;

    // Timing: compare actual invested periods vs always-invested
    let invested_fraction = actual_trades.len() as f64 / 252.0; // rough
    let timing_effect = (1.0 - invested_fraction) * (-benchmark_return);

    // Sizing: actual weighted return vs equal-weighted return
    let equal_w = 1.0 / actual_trades.len().max(1) as f64;
    let equal_return: f64 = actual_trades.iter()
        .map(|t| t.realized_return * equal_w)
        .sum();
    let actual_weighted: f64 = actual_trades.iter()
        .map(|t| t.realized_return * t.weight)
        .sum();
    let sizing_effect = actual_weighted - equal_return;

    // Strategy: compare adaptive strategy weights vs equal strategy weights
    // (requires grouping trades by source strategy)
    let strategy_effect = 0.0; // filled by grouping logic

    let interaction_effect = active_return - selection_effect - timing_effect
        - sizing_effect - strategy_effect;

    AttributionResult {
        period_start,
        period_end,
        total_return,
        benchmark_return,
        active_return,
        selection_effect,
        timing_effect,
        sizing_effect,
        strategy_effect,
        interaction_effect,
    }
}
```

#### 25.3 Attribution Reporting

The web dashboard adds an Attribution tab showing a waterfall chart:

```
Performance Attribution (Last 90 Days)
──────────────────────────────────────────────────────
Benchmark (SPY):     +4.2%
Stock Selection:     +2.8%  ████████████░░  (largest contributor)
Timing:              +0.9%  ████░░░░░░░░░░
Strategy Weighting:  +0.4%  ██░░░░░░░░░░░░
Position Sizing:     +0.3%  █░░░░░░░░░░░░░
Interaction:         -0.1%  ░░░░░░░░░░░░░░
──────────────────────────────────────────────────────
TOTAL RETURN:        +8.5%  (vs SPY +4.2%, alpha +4.3%)
```

---

## Data Model Extensions

### New SQLite Tables

```sql
-- Microstructure state history
CREATE TABLE IF NOT EXISTS microstructure_history (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    date              TEXT NOT NULL UNIQUE,
    vol_regime        TEXT NOT NULL,    -- Low/Normal/High
    corr_regime       TEXT NOT NULL,    -- LowCorrelation/Normal/HighCorrelation
    liquidity_event   TEXT NOT NULL,    -- Normal/QuadWitching/Fomc/EarningsSeason
    realized_vol_20d  REAL NOT NULL,
    avg_sector_corr   REAL NOT NULL,
    favorability      REAL NOT NULL
);

-- Adaptive parameter history
CREATE TABLE IF NOT EXISTS adaptive_params_history (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    calibrated_at           TEXT NOT NULL,
    regime                  TEXT NOT NULL,
    rs_threshold            REAL NOT NULL,
    adx_strong_threshold    REAL NOT NULL,
    adx_trending_threshold  REAL NOT NULL,
    volume_surge_ratio      REAL NOT NULL,
    consolidation_max_depth REAL NOT NULL,
    rsi_oversold_threshold  REAL NOT NULL,
    oos_hit_rate            REAL,
    oos_signal_count        INTEGER
);

-- Multi-horizon screening results
CREATE TABLE IF NOT EXISTS multi_horizon_scores (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    screen_date     TEXT NOT NULL,
    ticker          TEXT NOT NULL,
    swing_score     REAL NOT NULL,
    position_score  REAL NOT NULL,
    trend_score     REAL NOT NULL,
    blended_score   REAL NOT NULL,
    best_horizon    TEXT NOT NULL,
    UNIQUE(screen_date, ticker)
);

-- Strategy beliefs (Bayesian)
CREATE TABLE IF NOT EXISTS strategy_beliefs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    strategy    TEXT NOT NULL,
    regime      TEXT NOT NULL,
    alpha       REAL NOT NULL,
    beta        REAL NOT NULL,
    updated_at  TEXT NOT NULL,
    UNIQUE(strategy, regime)
);

-- Calibration audit log
CREATE TABLE IF NOT EXISTS calibration_log (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    calibrated_at   TEXT NOT NULL,
    regime          TEXT NOT NULL,
    params_json     TEXT NOT NULL,
    oos_hit_rate    REAL,
    oos_signal_count INTEGER,
    notes           TEXT
);

-- Performance attribution records
CREATE TABLE IF NOT EXISTS attribution_history (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    period_start        TEXT NOT NULL,
    period_end          TEXT NOT NULL,
    total_return        REAL NOT NULL,
    benchmark_return    REAL NOT NULL,
    active_return       REAL NOT NULL,
    selection_effect    REAL NOT NULL,
    timing_effect       REAL NOT NULL,
    sizing_effect       REAL NOT NULL,
    strategy_effect     REAL NOT NULL,
    interaction_effect  REAL NOT NULL
);
```

### Screener Trait Extension

```rust
/// Extended Screener trait for Phase 6.
pub trait Screener: Send + Sync {
    // ... existing methods ...

    /// Preferred time horizon for this strategy's signals.
    /// Used by the multi-horizon scorer to apply horizon-specific adjustments.
    fn preferred_horizon(&self) -> TimeHorizon {
        TimeHorizon::Position // default for backward compatibility
    }
}

// Implementations:
// MinerviniScreener::preferred_horizon() -> Position
// BreakoutScreener::preferred_horizon() -> Swing
// MeanReversionScreener::preferred_horizon() -> Swing
// TrendStrengthScreener::preferred_horizon() -> Trend
```

---

## CLI Extensions

```
ghost-alpha screen --adaptive              # Use adaptive weights (default after F20)
ghost-alpha screen --fixed-weights         # Use fixed regime weights (opt-out)
ghost-alpha screen --horizon swing         # Filter results for swing horizon only
ghost-alpha screen --horizon position      # Filter for position trades
ghost-alpha screen --horizon trend         # Filter for trend trades

ghost-alpha calibrate                      # Run parameter calibration
ghost-alpha calibrate --dry-run            # Show what would change, don't apply
ghost-alpha calibrate --reset              # Reset to defaults for current regime

ghost-alpha status                         # Extended with microstructure state
ghost-alpha attribution --period 90d       # Show last 90 days attribution
ghost-alpha beliefs                        # Show current Bayesian strategy beliefs
```

---

## Implementation Priority

| Priority | Feature | Module | Est. LOC | Dependencies |
|----------|---------|--------|----------|--------------|
| **P0**   | F22: Microstructure Awareness | `microstructure.rs` | ~400 | F9 (regime) |
| **P0**   | F23: Multi-Horizon Scoring | `multi_horizon.rs` | ~500 | F22, existing screeners |
| **P1**   | F20: Strategy Selection | `adaptive.rs` | ~600 | F22, F23 |
| **P1**   | F21: Param Tuning | `param_tuner.rs` | ~700 | F20, backtest engine |
| **P2**   | F24: Bayesian Weights | `bayesian_weights.rs` | ~500 | F20, SQLite |
| **P2**   | F25: Attribution | `attribution.rs` | ~400 | F24, backtest engine |
| **P3**   | Dashboard integration | `dashboard_routes.rs` | ~400 | All above |

**Implementation order**: F22 -> F23 -> F20 -> F21 -> F24 -> F25 -> Dashboard

Each feature is independently testable. F22 and F23 can ship as standalone improvements before the full adaptive engine is wired.

---

## Testing Strategy

### Unit Tests (inline `#[cfg(test)]`)

Each module gets comprehensive unit tests:

1. **microstructure.rs**: Test vol regime classification at boundaries (VIX=15.0, VIX=25.0), correlation regime with synthetic sector returns, liquidity calendar against known dates (e.g., 2026-03-20 = quad witching)

2. **multi_horizon.rs**: Test horizon weights sum to 100, softmax produces valid distributions, swing/position/trend scoring adjustments apply correctly, sizing params match horizon specs

3. **adaptive.rs**: Test strategy weight normalization, regime modifier application, vol adjustment stacking, weight guardrails enforced

4. **param_tuner.rs**: Test guardrail clamping, max_step enforcement, calibration with synthetic signals, regime-change reset behavior

5. **bayesian_weights.rs**: Test Beta prior initialization, update mechanics, decay behavior, blend with prior weights, guardrail enforcement (5%/60%)

6. **attribution.rs**: Test decomposition sums to active return, all-cash attribution = 0, single-stock attribution = selection only

### Integration Tests (`tests/integration/`)

```rust
/// Feature-gated: --features integration

#[test]
fn adaptive_engine_produces_different_weights_per_regime() {
    // Create Bull/Sideways/Bear market fixtures
    // Assert strategy weights differ across regimes
    // Assert weights sum to 1.0 in all cases
}

#[test]
fn multi_horizon_scores_diverge_for_different_stock_types() {
    // Breakout fixture: swing_score > trend_score
    // Slow grinder: trend_score > swing_score
}

#[test]
fn bayesian_update_shifts_weights_toward_winning_strategy() {
    // Start with equal priors
    // Feed 20 wins for Breakout, 20 losses for TrendStrength
    // Assert Breakout weight > TrendStrength weight
}

#[test]
fn param_calibration_respects_guardrails() {
    // Calibrate with extreme data that would push params beyond limits
    // Assert all params within guardrail ranges
    // Assert max_step not exceeded
}
```

---

## Risk Analysis

### What Could Go Wrong

| Risk | Impact | Mitigation |
|------|--------|------------|
| Overfitting in param tuning | Parameters tuned to noise, degrades OOS | Walk-forward validation, max_step limits, signal count penalty |
| Bayesian weights chase recent performance | Excessive turnover, strategy whipsaw | Exponential decay (half-life 138 obs), blend_factor=0.4, regime-change reset |
| Microstructure misclassification | Wrong strategy emphasized | 3 independent signals (vol, corr, liquidity), conservative thresholds, hysteresis |
| Complexity obscures signal source | User can't debug bad recommendations | Full attribution chain, `--fixed-weights` escape hatch, calibration audit log |
| Backtest overfitting to historical regimes | Poor forward performance | Walk-forward IS/OOS split, Monte Carlo CI on all metrics, staleness warnings |
| Calendar effects in liquidity model are US-centric | Misses non-US events | Clearly documented scope, future: allow user-defined calendar |

### Determinism Guarantees

Every output of the adaptive engine is fully reproducible given:
1. Same price data (pinned to specific date range)
2. Same `AdaptiveParams` version (stored in calibration_log)
3. Same `StrategyBelief` state (stored in strategy_beliefs table)
4. Same `MicrostructureState` (stored in microstructure_history)
5. Fixed random seeds for any Monte Carlo components (inherited from existing backtest engine)

The calibration process uses coordinate descent with a fixed grid (no randomness). Bayesian updates are deterministic given the same observation sequence.

---

## Configuration

New TOML config section:

```toml
[adaptive]
enabled = true                    # Master switch for adaptive engine
blend_factor = 0.4               # Bayesian blend (0=prior only, 1=Bayesian only)
softmax_temperature = 1.0        # Timeframe weight concentration
calibration_window_days = 252    # Rolling calibration lookback
min_signals_for_calibration = 20 # Cold start protection
decay_factor = 0.995             # Bayesian evidence decay per observation
param_tuning_enabled = true      # Separate toggle for parameter adaptation
reset_on_regime_change = true    # Reset beliefs when regime transitions confirm
```

All adaptive features default to `enabled = true` but can be individually disabled. The `--fixed-weights` CLI flag overrides all adaptive behavior for comparison purposes.
