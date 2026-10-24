//! Position sizing engine for converting screening signals into actionable trade sizes.
//!
//! Supports four methods:
//! - **Volatility Target** (default): sizes inversely proportional to ATR
//! - **Equal Weight**: 1/N allocation across target number of positions
//! - **Half-Kelly**: Kelly criterion with 50% dampening for safety
//! - **Fixed Fractional**: risk a fixed % of portfolio per trade

use crate::analysis::scorer::Grade;
use crate::data::models::MarketRegime;

// ─── Types ──────────────────────────────────────────────────────────────────

/// Position sizing method selection.
#[derive(Debug, Clone)]
pub enum SizingMethod {
    /// Size inversely proportional to ATR. Naturally takes smaller positions
    /// in volatile stocks.
    VolatilityTarget {
        /// ATR multiplier for stop distance (default: 2.0)
        stop_multiplier: f64,
    },
    /// Equal weight: 1/N allocation across target number of positions.
    EqualWeight {
        /// Target number of positions in portfolio (default: 8)
        target_positions: usize,
    },
    /// Kelly fraction with half-Kelly dampening. Requires historical win rate
    /// and average win/loss ratio from backtesting.
    HalfKelly {
        win_rate: f64,
        avg_win_loss_ratio: f64,
    },
    /// Risk a fixed percentage of portfolio per trade.
    FixedFractional {
        /// Fraction of portfolio to risk (default: 0.01 = 1%)
        risk_pct: f64,
    },
}

/// Computed position size recommendation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PositionSize {
    pub ticker: String,
    pub shares: f64,
    pub dollar_amount: f64,
    pub pct_of_portfolio: f64,
    pub stop_loss: f64,
    pub risk_amount: f64,
    pub sizing_method: String,
    /// Whether the position was capped by max_position_pct
    pub was_capped: bool,
}

/// Inputs required for position sizing.
#[derive(Debug, Clone)]
pub struct SizingInput {
    pub ticker: String,
    pub close_price: f64,
    pub atr_14: f64,
    pub grade: Grade,
    pub regime: MarketRegime,
}

// ─── Grade & Regime Multipliers ─────────────────────────────────────────────

/// Grade multiplier: higher-graded stocks get larger positions.
pub fn grade_multiplier(grade: &Grade) -> f64 {
    match grade {
        Grade::APlus => 1.5,
        Grade::A => 1.0,
        Grade::B => 0.7,
        Grade::C => 0.5,
        Grade::D => 0.3,
        Grade::F => 0.0,
    }
}

/// Regime multiplier: reduce exposure in weaker regimes.
pub fn regime_multiplier(regime: &MarketRegime) -> f64 {
    match regime {
        MarketRegime::Bull => 1.0,
        MarketRegime::Sideways => 0.8,
        MarketRegime::Bear => 0.5,
    }
}

// ─── Core Sizing Function ───────────────────────────────────────────────────

/// Compute position size for a given stock and method.
///
/// Risk budget = portfolio_value × risk_pct × grade_multiplier × regime_multiplier.
/// Final size is clamped to max_position_pct of portfolio.
pub fn compute_position_size(
    method: &SizingMethod,
    input: &SizingInput,
    portfolio_value: f64,
    risk_pct: f64,
    max_position_pct: f64,
) -> PositionSize {
    if portfolio_value <= 0.0 || input.close_price <= 0.0 {
        return zero_position(&input.ticker, method);
    }

    let gm = grade_multiplier(&input.grade);
    let rm = regime_multiplier(&input.regime);

    if gm == 0.0 {
        return zero_position(&input.ticker, method);
    }

    let risk_budget = portfolio_value * risk_pct * gm * rm;

    let (raw_shares, stop_loss) = match method {
        SizingMethod::VolatilityTarget { stop_multiplier } => {
            let stop = input.close_price - (input.atr_14 * stop_multiplier);
            let risk_per_share = (input.close_price - stop).max(0.01);
            (risk_budget / risk_per_share, stop)
        }
        SizingMethod::EqualWeight { target_positions } => {
            let alloc = portfolio_value / (*target_positions).max(1) as f64;
            let shares = alloc / input.close_price;
            let stop = input.close_price - (2.0 * input.atr_14);
            (shares, stop)
        }
        SizingMethod::FixedFractional { risk_pct: frac } => {
            let risk = portfolio_value * frac;
            let stop = input.close_price - (2.0 * input.atr_14);
            let risk_per_share = (input.close_price - stop).max(0.01);
            (risk / risk_per_share, stop)
        }
        SizingMethod::HalfKelly {
            win_rate,
            avg_win_loss_ratio,
        } => {
            let kelly = if *avg_win_loss_ratio > 0.0 {
                (win_rate * avg_win_loss_ratio - (1.0 - win_rate)) / avg_win_loss_ratio
            } else {
                0.0
            };
            let half_kelly = kelly.clamp(0.0, 0.25) / 2.0;
            let alloc = portfolio_value * half_kelly;
            let shares = alloc / input.close_price;
            let stop = input.close_price - (2.0 * input.atr_14);
            (shares, stop)
        }
    };

    // Enforce position cap
    let max_shares = (max_position_pct * portfolio_value) / input.close_price;
    let was_capped = raw_shares > max_shares;
    let final_shares = raw_shares.min(max_shares).max(0.0).floor();

    let dollar_amount = final_shares * input.close_price;

    PositionSize {
        ticker: input.ticker.clone(),
        shares: final_shares,
        dollar_amount,
        pct_of_portfolio: if portfolio_value > 0.0 {
            dollar_amount / portfolio_value
        } else {
            0.0
        },
        stop_loss,
        risk_amount: final_shares * (input.close_price - stop_loss).max(0.0),
        sizing_method: format_method(method),
        was_capped,
    }
}

fn zero_position(ticker: &str, method: &SizingMethod) -> PositionSize {
    PositionSize {
        ticker: ticker.to_string(),
        shares: 0.0,
        dollar_amount: 0.0,
        pct_of_portfolio: 0.0,
        stop_loss: 0.0,
        risk_amount: 0.0,
        sizing_method: format_method(method),
        was_capped: false,
    }
}

fn format_method(method: &SizingMethod) -> String {
    match method {
        SizingMethod::VolatilityTarget { .. } => "VolatilityTarget".to_string(),
        SizingMethod::EqualWeight { .. } => "EqualWeight".to_string(),
        SizingMethod::HalfKelly { .. } => "HalfKelly".to_string(),
        SizingMethod::FixedFractional { .. } => "FixedFractional".to_string(),
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_input() -> SizingInput {
        SizingInput {
            ticker: "AAPL".to_string(),
            close_price: 190.0,
            atr_14: 4.0,
            grade: Grade::A,
            regime: MarketRegime::Bull,
        }
    }

    // ── grade & regime multipliers ──────────────────────────────────────────

    #[test]
    fn grade_multipliers_ordered() {
        assert!(grade_multiplier(&Grade::APlus) > grade_multiplier(&Grade::A));
        assert!(grade_multiplier(&Grade::A) > grade_multiplier(&Grade::B));
        assert!(grade_multiplier(&Grade::B) > grade_multiplier(&Grade::C));
        assert!(grade_multiplier(&Grade::C) > grade_multiplier(&Grade::D));
        assert_eq!(grade_multiplier(&Grade::F), 0.0);
    }

    #[test]
    fn regime_multipliers_ordered() {
        assert!(
            regime_multiplier(&MarketRegime::Bull) > regime_multiplier(&MarketRegime::Sideways)
        );
        assert!(
            regime_multiplier(&MarketRegime::Sideways) > regime_multiplier(&MarketRegime::Bear)
        );
    }

    // ── volatility target ───────────────────────────────────────────────────

    #[test]
    fn vol_target_basic() {
        let method = SizingMethod::VolatilityTarget {
            stop_multiplier: 2.0,
        };
        let input = default_input(); // price=190, ATR=4, A grade, Bull
        let result = compute_position_size(&method, &input, 100_000.0, 0.01, 0.15);

        // risk_budget = 100K × 0.01 × 1.0 (A) × 1.0 (Bull) = $1,000
        // stop = 190 - 2×4 = 182
        // risk_per_share = 190 - 182 = 8
        // raw_shares = 1000 / 8 = 125
        // max_shares = 0.15 × 100K / 190 = 78.9 → capped at 78
        assert_eq!(result.shares, 78.0);
        assert!(result.was_capped);
        assert!((result.stop_loss - 182.0).abs() < 0.01);
    }

    #[test]
    fn vol_target_bear_regime_reduces_size() {
        let method = SizingMethod::VolatilityTarget {
            stop_multiplier: 2.0,
        };
        let mut input = default_input();
        input.regime = MarketRegime::Bear;

        let result = compute_position_size(&method, &input, 100_000.0, 0.01, 0.15);

        // risk_budget = 100K × 0.01 × 1.0 × 0.5 (Bear) = $500
        // raw_shares = 500 / 8 = 62.5 → 62
        assert_eq!(result.shares, 62.0);
        assert!(!result.was_capped);
    }

    #[test]
    fn vol_target_b_grade_reduces_size() {
        let method = SizingMethod::VolatilityTarget {
            stop_multiplier: 2.0,
        };
        let mut input = default_input();
        input.grade = Grade::B;

        let result = compute_position_size(&method, &input, 100_000.0, 0.01, 0.15);

        // risk_budget = 100K × 0.01 × 0.7 (B) × 1.0 = $700
        // raw_shares = 700 / 8 = 87.5 → capped at 78
        assert!(result.shares <= 78.0);
    }

    #[test]
    fn vol_target_f_grade_returns_zero() {
        let method = SizingMethod::VolatilityTarget {
            stop_multiplier: 2.0,
        };
        let mut input = default_input();
        input.grade = Grade::F;

        let result = compute_position_size(&method, &input, 100_000.0, 0.01, 0.15);
        assert_eq!(result.shares, 0.0);
    }

    #[test]
    fn vol_target_high_atr_means_fewer_shares() {
        let method = SizingMethod::VolatilityTarget {
            stop_multiplier: 2.0,
        };
        let mut low_vol = default_input();
        low_vol.atr_14 = 2.0;

        let mut high_vol = default_input();
        high_vol.atr_14 = 8.0;

        let low_result = compute_position_size(&method, &low_vol, 100_000.0, 0.01, 1.0);
        let high_result = compute_position_size(&method, &high_vol, 100_000.0, 0.01, 1.0);

        assert!(
            high_result.shares < low_result.shares,
            "Higher ATR should mean fewer shares: low={} high={}",
            low_result.shares,
            high_result.shares
        );
    }

    // ── equal weight ────────────────────────────────────────────────────────

    #[test]
    fn equal_weight_distributes_evenly() {
        let method = SizingMethod::EqualWeight {
            target_positions: 8,
        };
        let input = default_input();
        let result = compute_position_size(&method, &input, 100_000.0, 0.01, 0.15);

        // alloc = 100K / 8 = 12,500
        // shares = 12,500 / 190 = 65.7 → 65
        assert_eq!(result.shares, 65.0);
    }

    // ── fixed fractional ────────────────────────────────────────────────────

    #[test]
    fn fixed_fractional_basic() {
        let method = SizingMethod::FixedFractional { risk_pct: 0.01 };
        let input = default_input();
        let result = compute_position_size(&method, &input, 100_000.0, 0.01, 0.15);

        // risk = 100K × 0.01 = $1,000
        // stop = 190 - 2×4 = 182, risk_per_share = 8
        // shares = 1000 / 8 = 125 → capped at 78
        assert_eq!(result.shares, 78.0);
        assert!(result.was_capped);
    }

    // ── half kelly ──────────────────────────────────────────────────────────

    #[test]
    fn half_kelly_with_good_stats() {
        let method = SizingMethod::HalfKelly {
            win_rate: 0.6,
            avg_win_loss_ratio: 2.0,
        };
        let input = default_input();
        let result = compute_position_size(&method, &input, 100_000.0, 0.01, 0.50);

        // kelly = (0.6 × 2.0 - 0.4) / 2.0 = (1.2 - 0.4) / 2.0 = 0.4
        // clamped to 0.25, then halved = 0.125
        // alloc = 100K × 0.125 = $12,500
        // shares = 12,500 / 190 = 65.7 → 65
        assert_eq!(result.shares, 65.0);
        assert!(result.dollar_amount > 0.0);
    }

    #[test]
    fn half_kelly_negative_edge_returns_zero() {
        let method = SizingMethod::HalfKelly {
            win_rate: 0.3,
            avg_win_loss_ratio: 1.0,
        };
        let input = default_input();
        let result = compute_position_size(&method, &input, 100_000.0, 0.01, 0.15);

        // kelly = (0.3 × 1.0 - 0.7) / 1.0 = -0.4 → clamped to 0 → 0 shares
        assert_eq!(result.shares, 0.0);
    }

    // ── position cap ────────────────────────────────────────────────────────

    #[test]
    fn position_cap_enforced() {
        let method = SizingMethod::VolatilityTarget {
            stop_multiplier: 2.0,
        };
        let input = default_input();

        // With 5% cap
        let result = compute_position_size(&method, &input, 100_000.0, 0.01, 0.05);
        let max_dollar = 0.05 * 100_000.0;
        assert!(result.dollar_amount <= max_dollar + input.close_price); // +1 share tolerance
        assert!(result.was_capped);
    }

    // ── edge cases ──────────────────────────────────────────────────────────

    #[test]
    fn zero_portfolio_returns_zero() {
        let method = SizingMethod::VolatilityTarget {
            stop_multiplier: 2.0,
        };
        let input = default_input();
        let result = compute_position_size(&method, &input, 0.0, 0.01, 0.15);
        assert_eq!(result.shares, 0.0);
    }

    #[test]
    fn zero_price_returns_zero() {
        let method = SizingMethod::VolatilityTarget {
            stop_multiplier: 2.0,
        };
        let mut input = default_input();
        input.close_price = 0.0;
        let result = compute_position_size(&method, &input, 100_000.0, 0.01, 0.15);
        assert_eq!(result.shares, 0.0);
    }

    #[test]
    fn pct_of_portfolio_correct() {
        let method = SizingMethod::EqualWeight {
            target_positions: 10,
        };
        let input = default_input();
        let result = compute_position_size(&method, &input, 100_000.0, 0.01, 0.15);

        let expected_pct = result.dollar_amount / 100_000.0;
        assert!((result.pct_of_portfolio - expected_pct).abs() < 0.001);
    }

    #[test]
    fn risk_amount_positive() {
        let method = SizingMethod::VolatilityTarget {
            stop_multiplier: 2.0,
        };
        let input = default_input();
        let result = compute_position_size(&method, &input, 100_000.0, 0.01, 0.15);

        assert!(result.risk_amount > 0.0);
        assert!(result.risk_amount <= result.dollar_amount);
    }
}
