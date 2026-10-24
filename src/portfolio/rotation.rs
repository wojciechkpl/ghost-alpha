//! Capital rotation engine — Sell Pressure Score (SPS), Cash Deployment Score
//! (CDS), and Upgrade Score (US) for systematic position management.
//!
//! Phase 5.5: Bridges the gap between binary exit signals and continuous
//! portfolio optimization by quantifying sell urgency, deployment opportunity,
//! and swap value.

use crate::analysis::scorer::Grade;
use crate::data::models::MarketRegime;

// ─── Sell Pressure Score (SPS) ──────────────────────────────────────────────

/// Action derived from SPS thresholds.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum SpsAction {
    /// 0-25: Hold, no action needed
    Green,
    /// 26-45: Tighten trailing stop to 1.5x ATR
    Yellow,
    /// 46-65: Evaluate upgrade candidates
    Orange,
    /// 66-80: Sell half, begin selling programme
    Red,
    /// 81-100: Sell all immediately
    Critical,
}

impl SpsAction {
    pub fn from_score(sps: f64) -> Self {
        match sps {
            s if s >= 81.0 => SpsAction::Critical,
            s if s >= 66.0 => SpsAction::Red,
            s if s >= 46.0 => SpsAction::Orange,
            s if s >= 26.0 => SpsAction::Yellow,
            _ => SpsAction::Green,
        }
    }
}

/// Individual SPS component scores (each 0-100).
#[derive(Debug, Clone, serde::Serialize)]
pub struct SpsComponents {
    pub trend_decay: f64,
    pub momentum_fade: f64,
    pub volume_abandonment: f64,
    pub relative_weakness: f64,
    pub drawdown_penalty: f64,
    pub time_decay: f64,
    pub concentration_penalty: f64,
}

/// Regime-adaptive SPS component weights.
#[derive(Debug, Clone)]
pub struct SpsWeights {
    pub trend_decay: f64,
    pub momentum_fade: f64,
    pub volume_abandonment: f64,
    pub relative_weakness: f64,
    pub drawdown_penalty: f64,
    pub time_decay: f64,
    pub concentration_penalty: f64,
}

impl SpsWeights {
    pub fn for_regime(regime: &MarketRegime) -> Self {
        match regime {
            MarketRegime::Bull => Self {
                trend_decay: 0.25,
                momentum_fade: 0.15,
                volume_abandonment: 0.10,
                relative_weakness: 0.10,
                drawdown_penalty: 0.10,
                time_decay: 0.05,
                concentration_penalty: 0.10,
                // Note: weights don't sum to 1.0 because FundamentalDrift (0.15) is deferred
            },
            MarketRegime::Sideways => Self {
                trend_decay: 0.20,
                momentum_fade: 0.15,
                volume_abandonment: 0.10,
                relative_weakness: 0.20,
                drawdown_penalty: 0.10,
                time_decay: 0.05,
                concentration_penalty: 0.10,
            },
            MarketRegime::Bear => Self {
                trend_decay: 0.15,
                momentum_fade: 0.20,
                volume_abandonment: 0.10,
                relative_weakness: 0.15,
                drawdown_penalty: 0.20,
                time_decay: 0.05,
                concentration_penalty: 0.10,
            },
        }
    }

    fn sum(&self) -> f64 {
        self.trend_decay
            + self.momentum_fade
            + self.volume_abandonment
            + self.relative_weakness
            + self.drawdown_penalty
            + self.time_decay
            + self.concentration_penalty
    }
}

/// Full sell pressure assessment for a position.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SellPressure {
    pub ticker: String,
    pub sps: f64,
    pub components: SpsComponents,
    pub action: SpsAction,
    pub hard_exit_override: bool,
}

/// Inputs needed to compute SPS for a single position.
#[derive(Debug, Clone)]
pub struct SpsInput {
    /// Current Minervini conditions met
    pub conditions_met: u8,
    /// Conditions met at entry
    pub conditions_at_entry: u8,
    /// Conditions lost in last 5 sessions
    pub conditions_lost_5d: u8,
    /// Current composite score
    pub composite_now: f64,
    /// Composite score 5 sessions ago
    pub composite_5d_ago: f64,
    /// Composite score 20 sessions ago
    pub composite_20d_ago: f64,
    /// Average volume on up-days last 20 sessions
    pub up_day_avg_vol: f64,
    /// Average volume on down-days last 20 sessions
    pub down_day_avg_vol: f64,
    /// Current RS percentile (0-100)
    pub rs_percentile: f64,
    /// RS percentile 20 sessions ago
    pub rs_20d_ago: f64,
    /// Current close price
    pub current_close: f64,
    /// Highest close in last 60 sessions
    pub local_high_60d: f64,
    /// Trading sessions since last 20-day new high
    pub days_since_new_high: u32,
    /// Position value / portfolio value
    pub actual_weight: f64,
    /// 1/N target weight
    pub target_weight: f64,
    /// Whether a hard exit signal has fired
    pub hard_exit_triggered: bool,
}

/// Compute the Sell Pressure Score for a position.
pub fn compute_sell_pressure(
    ticker: &str,
    input: &SpsInput,
    regime: &MarketRegime,
) -> SellPressure {
    let components = compute_sps_components(input);
    let weights = SpsWeights::for_regime(regime);
    let weight_sum = weights.sum();

    let raw_sps = if weight_sum > 0.0 {
        (weights.trend_decay * components.trend_decay
            + weights.momentum_fade * components.momentum_fade
            + weights.volume_abandonment * components.volume_abandonment
            + weights.relative_weakness * components.relative_weakness
            + weights.drawdown_penalty * components.drawdown_penalty
            + weights.time_decay * components.time_decay
            + weights.concentration_penalty * components.concentration_penalty)
            / weight_sum
    } else {
        0.0
    };

    // Hard exit override: force SPS >= 85
    let sps = if input.hard_exit_triggered {
        raw_sps.max(85.0)
    } else {
        raw_sps
    }
    .clamp(0.0, 100.0);

    SellPressure {
        ticker: ticker.to_string(),
        sps,
        components,
        action: SpsAction::from_score(sps),
        hard_exit_override: input.hard_exit_triggered,
    }
}

fn compute_sps_components(input: &SpsInput) -> SpsComponents {
    SpsComponents {
        trend_decay: trend_decay(input),
        momentum_fade: momentum_fade(input),
        volume_abandonment: volume_abandonment(input),
        relative_weakness: relative_weakness(input),
        drawdown_penalty: drawdown_penalty(input),
        time_decay: time_decay(input),
        concentration_penalty: concentration_penalty(input),
    }
}

/// TrendDecay: How many Minervini conditions have been lost since entry.
fn trend_decay(input: &SpsInput) -> f64 {
    if input.conditions_at_entry == 0 {
        return 0.0;
    }
    let decay_rate = (input.conditions_at_entry
        - input.conditions_met.min(input.conditions_at_entry)) as f64
        / input.conditions_at_entry as f64;
    let mut score = decay_rate * 100.0;

    // Acceleration bonus: rapid loss of conditions
    if input.conditions_lost_5d >= 2 {
        score = (score + 20.0).min(100.0);
    }
    score.clamp(0.0, 100.0)
}

/// MomentumFade: Rate of change of composite score (short + long term).
fn momentum_fade(input: &SpsInput) -> f64 {
    let short_delta = if input.composite_5d_ago > 0.0 {
        (input.composite_5d_ago - input.composite_now) / input.composite_5d_ago * 100.0
    } else {
        0.0
    };
    let long_delta = if input.composite_20d_ago > 0.0 {
        (input.composite_20d_ago - input.composite_now) / input.composite_20d_ago * 100.0
    } else {
        0.0
    };
    (0.4 * short_delta + 0.6 * long_delta).clamp(0.0, 100.0)
}

/// VolumeAbandonment: Down-day volume vs up-day volume ratio.
fn volume_abandonment(input: &SpsInput) -> f64 {
    if input.up_day_avg_vol <= 0.0 {
        return 50.0; // neutral if no data
    }
    let ratio = input.down_day_avg_vol / input.up_day_avg_vol;
    // ratio = 1.0 -> 50 (neutral), ratio = 1.5 -> 75, ratio = 0.5 -> 25
    ((ratio - 1.0) * 50.0 + 50.0).clamp(0.0, 100.0)
}

/// RelativeWeakness: Low RS + declining RS trajectory.
fn relative_weakness(input: &SpsInput) -> f64 {
    let rs_delta = (input.rs_20d_ago - input.rs_percentile).max(0.0);
    ((100.0 - input.rs_percentile) * 0.6 + rs_delta * 2.0).clamp(0.0, 100.0)
}

/// DrawdownPenalty: Current drawdown from local high (aggressive: 20% DD = 100).
fn drawdown_penalty(input: &SpsInput) -> f64 {
    if input.local_high_60d <= 0.0 {
        return 0.0;
    }
    let dd = (input.local_high_60d - input.current_close) / input.local_high_60d;
    (dd * 500.0).clamp(0.0, 100.0) // 20% drawdown = 100
}

/// TimeDecay: Sessions since last new high, penalized after 40-day grace period.
fn time_decay(input: &SpsInput) -> f64 {
    let excess = input.days_since_new_high.saturating_sub(40) as f64;
    (excess * 2.0).clamp(0.0, 100.0) // 2 pts per day after 40
}

/// ConcentrationPenalty: Position oversized relative to target weight.
fn concentration_penalty(input: &SpsInput) -> f64 {
    if input.target_weight <= 0.0 {
        return 0.0;
    }
    let overshoot =
        (input.actual_weight - input.target_weight * 1.5).max(0.0) / input.target_weight;
    (overshoot * 100.0).clamp(0.0, 100.0)
}

// ─── Cash Deployment Score (CDS) ────────────────────────────────────────────

/// Action derived from CDS thresholds.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum DeployAction {
    /// CDS 0-30: Hold cash, regime or opportunity is poor
    HoldCash,
    /// CDS 31-50: Half-size positions, require composite >= 80
    DeployCautiously,
    /// CDS 51-70: Standard sizing, composite >= 70
    DeployNormally,
    /// CDS 71-100: Full sizing, composite >= 65
    DeployAggressively,
}

impl DeployAction {
    pub fn from_score(cds: f64) -> Self {
        match cds {
            s if s >= 71.0 => DeployAction::DeployAggressively,
            s if s >= 51.0 => DeployAction::DeployNormally,
            s if s >= 31.0 => DeployAction::DeployCautiously,
            _ => DeployAction::HoldCash,
        }
    }

    /// Minimum composite score required for entry at this deployment level.
    pub fn min_composite(&self) -> f64 {
        match self {
            DeployAction::HoldCash => f64::MAX,
            DeployAction::DeployCautiously => 80.0,
            DeployAction::DeployNormally => 70.0,
            DeployAction::DeployAggressively => 65.0,
        }
    }
}

/// Cash Deployment Score result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DeploymentScore {
    pub cds: f64,
    pub action: DeployAction,
    pub opportunity_richness: f64,
    pub regime_score: f64,
    pub cash_drag_cost: f64,
    pub drawdown_risk: f64,
}

/// Inputs for CDS computation.
#[derive(Debug, Clone)]
pub struct CdsInput {
    /// Number of candidates passing screen with composite >= 70
    pub candidates_above_70: usize,
    /// Highest composite score among candidates
    pub top_candidate_score: f64,
    /// Current market regime
    pub regime: MarketRegime,
    /// Whether regime just improved (e.g., Bear -> Sideways)
    pub regime_improving: bool,
    /// Whether regime just deteriorated (e.g., Bull -> Sideways)
    pub regime_deteriorating: bool,
    /// Cash as fraction of portfolio value
    pub cash_pct: f64,
    /// Sessions since capital was freed
    pub days_in_cash: u32,
    /// VIX percentile vs 1-year range (0-100, 100 = highest vol)
    pub vix_percentile: f64,
    /// Market breadth: % of S&P 500 above 200-SMA
    pub breadth_pct: f64,
}

/// Compute the Cash Deployment Score.
pub fn compute_deployment_score(input: &CdsInput) -> DeploymentScore {
    let opportunity = opportunity_richness(input);
    let regime = regime_score(input);
    let drag = cash_drag_cost(input);
    let risk = drawdown_risk(input);

    let cds = (0.35 * opportunity + 0.25 * regime + 0.15 * drag - 0.25 * risk).clamp(0.0, 100.0);

    DeploymentScore {
        cds,
        action: DeployAction::from_score(cds),
        opportunity_richness: opportunity,
        regime_score: regime,
        cash_drag_cost: drag,
        drawdown_risk: risk,
    }
}

fn opportunity_richness(input: &CdsInput) -> f64 {
    let count_score = (input.candidates_above_70 as f64 * 5.0).min(50.0);
    let quality_score = input.top_candidate_score * 0.5;
    let raw = count_score + quality_score;

    // If fewer than 3 candidates, cap at 30
    if input.candidates_above_70 < 3 {
        raw.min(30.0)
    } else {
        raw.min(100.0)
    }
}

fn regime_score(input: &CdsInput) -> f64 {
    let base = match input.regime {
        MarketRegime::Bull => 80.0,
        MarketRegime::Sideways => 45.0,
        MarketRegime::Bear => 15.0,
    };
    let adjustment: f64 = if input.regime_improving {
        10.0
    } else if input.regime_deteriorating {
        -10.0
    } else {
        0.0
    };
    (base + adjustment).clamp(0.0, 100.0)
}

fn cash_drag_cost(input: &CdsInput) -> f64 {
    (input.cash_pct * 100.0 * 0.5 + input.days_in_cash as f64 * 0.5).clamp(0.0, 100.0)
}

fn drawdown_risk(input: &CdsInput) -> f64 {
    (0.5 * input.vix_percentile + 0.5 * (100.0 - input.breadth_pct)).clamp(0.0, 100.0)
}

// ─── Upgrade Score (US) ─────────────────────────────────────────────────────

/// Upgrade recommendation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub enum UpgradeAction {
    /// US > threshold: Execute the swap
    Upgrade,
    /// US in [5, threshold]: Flag for manual review
    Consider,
    /// US < 5: Friction exceeds benefit
    Hold,
}

/// Upgrade comparison result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UpgradeComparison {
    pub incumbent: String,
    pub candidate: String,
    pub upgrade_score: f64,
    pub alpha_advantage: f64,
    pub momentum_edge: f64,
    pub risk_improvement: f64,
    pub cost_friction: f64,
    pub recommendation: UpgradeAction,
}

/// Inputs for upgrade comparison.
#[derive(Debug, Clone)]
pub struct UpgradeInput {
    /// Incumbent position data
    pub incumbent_composite: f64,
    pub incumbent_rs: f64,
    pub incumbent_rs_rising: bool,
    pub incumbent_atr_pct: f64,
    pub incumbent_portfolio_corr: f64,
    /// Candidate screen result data
    pub candidate_composite: f64,
    pub candidate_rs: f64,
    pub candidate_rs_rising: bool,
    pub candidate_atr_pct: f64,
    pub candidate_portfolio_corr: f64,
    /// Estimated friction costs
    pub spread_cost_pct: f64,
    pub tax_cost_pct: f64,
    pub wash_sale_risk: bool,
    /// Grade of incumbent (determines threshold)
    pub incumbent_grade: Grade,
}

/// Compute upgrade score and recommendation.
pub fn compute_upgrade_score(
    incumbent_ticker: &str,
    candidate_ticker: &str,
    input: &UpgradeInput,
) -> UpgradeComparison {
    let alpha = alpha_advantage(input);
    let momentum = momentum_edge(input);
    let risk = risk_improvement(input);
    let friction = cost_friction(input);

    let us = alpha + momentum + risk + friction;

    let threshold = upgrade_threshold(&input.incumbent_grade);
    let recommendation = if us > threshold {
        UpgradeAction::Upgrade
    } else if us > 5.0 {
        UpgradeAction::Consider
    } else {
        UpgradeAction::Hold
    };

    UpgradeComparison {
        incumbent: incumbent_ticker.to_string(),
        candidate: candidate_ticker.to_string(),
        upgrade_score: us,
        alpha_advantage: alpha,
        momentum_edge: momentum,
        risk_improvement: risk,
        cost_friction: friction,
        recommendation,
    }
}

/// Tier-adjusted upgrade threshold.
fn upgrade_threshold(grade: &Grade) -> f64 {
    match grade {
        Grade::APlus => 25.0, // A+ positions need very compelling upgrade
        Grade::A => 15.0,     // Standard threshold
        Grade::B => 10.0,     // Grandfathered positions — lower bar
        Grade::C => 10.0,
        Grade::D => 5.0, // Weak positions — easy to justify swap
        Grade::F => 5.0,
    }
}

fn alpha_advantage(input: &UpgradeInput) -> f64 {
    let diff = input.candidate_composite - input.incumbent_composite;
    (diff * 0.5).clamp(-50.0, 50.0)
}

fn momentum_edge(input: &UpgradeInput) -> f64 {
    let rs_diff = (input.candidate_rs - input.incumbent_rs) * 0.2;
    let slope_bonus = match (input.candidate_rs_rising, input.incumbent_rs_rising) {
        (true, false) => 15.0,  // candidate rising, incumbent falling
        (true, true) => 3.0,    // both rising, slight candidate edge
        (false, false) => 0.0,  // both flat/falling
        (false, true) => -10.0, // incumbent rising, candidate not — penalty
    };
    (rs_diff + slope_bonus).clamp(-30.0, 30.0)
}

fn risk_improvement(input: &UpgradeInput) -> f64 {
    let vol_improvement = if input.incumbent_atr_pct > 0.0 {
        (input.incumbent_atr_pct - input.candidate_atr_pct) / input.incumbent_atr_pct * 20.0
    } else {
        0.0
    };
    let corr_improvement = (input.incumbent_portfolio_corr - input.candidate_portfolio_corr) * 20.0;
    (0.5 * vol_improvement + 0.5 * corr_improvement).clamp(-20.0, 20.0)
}

fn cost_friction(input: &UpgradeInput) -> f64 {
    let wash_penalty = if input.wash_sale_risk { 5.0 } else { 0.0 };
    -(input.spread_cost_pct + input.tax_cost_pct + wash_penalty).clamp(0.0, 30.0)
}

// ─── Anti-Churn Safeguards ──────────────────────────────────────────────────

/// Check whether a position is eligible for upgrade evaluation.
pub fn is_eligible_for_upgrade(
    days_held: u32,
    days_since_last_swap: u32,
    swaps_this_week: u32,
    hard_exit_triggered: bool,
) -> bool {
    // Hard exits bypass all churn protection
    if hard_exit_triggered {
        return true;
    }
    // Minimum holding period: 10 sessions
    if days_held < 10 {
        return false;
    }
    // Swap cooldown: 20 sessions since last swap-in
    if days_since_last_swap < 20 {
        return false;
    }
    // Max 2 swaps per week
    if swaps_this_week >= 2 {
        return false;
    }
    true
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── SPS helpers ─────────────────────────────────────────────────────────

    fn healthy_sps_input() -> SpsInput {
        SpsInput {
            conditions_met: 8,
            conditions_at_entry: 8,
            conditions_lost_5d: 0,
            composite_now: 85.0,
            composite_5d_ago: 83.0,
            composite_20d_ago: 80.0,
            up_day_avg_vol: 1_000_000.0,
            down_day_avg_vol: 800_000.0,
            rs_percentile: 85.0,
            rs_20d_ago: 82.0,
            current_close: 195.0,
            local_high_60d: 200.0,
            days_since_new_high: 10,
            actual_weight: 0.12,
            target_weight: 0.125,
            hard_exit_triggered: false,
        }
    }

    fn deteriorating_sps_input() -> SpsInput {
        SpsInput {
            conditions_met: 4,
            conditions_at_entry: 8,
            conditions_lost_5d: 3,
            composite_now: 55.0,
            composite_5d_ago: 70.0,
            composite_20d_ago: 80.0,
            up_day_avg_vol: 500_000.0,
            down_day_avg_vol: 1_200_000.0,
            rs_percentile: 35.0,
            rs_20d_ago: 65.0,
            current_close: 160.0,
            local_high_60d: 200.0,
            days_since_new_high: 55,
            actual_weight: 0.20,
            target_weight: 0.125,
            hard_exit_triggered: false,
        }
    }

    // ── SPS tests ───────────────────────────────────────────────────────────

    #[test]
    fn sps_healthy_position_is_green() {
        let result = compute_sell_pressure("AAPL", &healthy_sps_input(), &MarketRegime::Bull);
        assert_eq!(result.action, SpsAction::Green);
        assert!(
            result.sps < 25.0,
            "Healthy position SPS should be < 25, got {:.1}",
            result.sps
        );
    }

    #[test]
    fn sps_deteriorating_position_is_red_or_worse() {
        let result =
            compute_sell_pressure("AAPL", &deteriorating_sps_input(), &MarketRegime::Sideways);
        assert!(
            result.sps >= 46.0,
            "Deteriorating position should be at least Orange, got {:.1} ({:?})",
            result.sps,
            result.action
        );
    }

    #[test]
    fn sps_hard_exit_overrides_to_critical() {
        let mut input = healthy_sps_input();
        input.hard_exit_triggered = true;
        let result = compute_sell_pressure("AAPL", &input, &MarketRegime::Bull);
        assert!(
            result.sps >= 85.0,
            "Hard exit should force SPS >= 85, got {:.1}",
            result.sps
        );
        assert_eq!(result.action, SpsAction::Critical);
        assert!(result.hard_exit_override);
    }

    #[test]
    fn sps_clamped_to_0_100() {
        let result = compute_sell_pressure("AAPL", &healthy_sps_input(), &MarketRegime::Bull);
        assert!(result.sps >= 0.0 && result.sps <= 100.0);

        let result2 =
            compute_sell_pressure("AAPL", &deteriorating_sps_input(), &MarketRegime::Bear);
        assert!(result2.sps >= 0.0 && result2.sps <= 100.0);
    }

    #[test]
    fn sps_bear_regime_more_aggressive() {
        let input = deteriorating_sps_input();
        let bull_sps = compute_sell_pressure("AAPL", &input, &MarketRegime::Bull);
        let bear_sps = compute_sell_pressure("AAPL", &input, &MarketRegime::Bear);
        // Bear weights increase DrawdownPenalty and MomentumFade
        // For a deteriorating position, bear should give equal or higher SPS
        assert!(
            bear_sps.sps >= bull_sps.sps - 5.0,
            "Bear SPS ({:.1}) should be >= Bull SPS ({:.1}) for deteriorating position",
            bear_sps.sps,
            bull_sps.sps
        );
    }

    // ── Component tests ─────────────────────────────────────────────────────

    #[test]
    fn trend_decay_zero_when_all_conditions_held() {
        let input = healthy_sps_input();
        assert!((trend_decay(&input) - 0.0).abs() < 0.01);
    }

    #[test]
    fn trend_decay_50_when_half_conditions_lost() {
        let mut input = healthy_sps_input();
        input.conditions_met = 4;
        input.conditions_lost_5d = 0;
        let score = trend_decay(&input);
        assert!((score - 50.0).abs() < 0.01, "Expected ~50, got {score}");
    }

    #[test]
    fn trend_decay_acceleration_bonus() {
        let mut input = healthy_sps_input();
        input.conditions_met = 5;
        input.conditions_lost_5d = 3; // rapid loss
        let score = trend_decay(&input);
        let without_bonus = (3.0 / 8.0) * 100.0; // 37.5
        assert!(
            score > without_bonus,
            "Acceleration bonus should increase score: {score}"
        );
    }

    #[test]
    fn momentum_fade_zero_when_improving() {
        let input = healthy_sps_input(); // composite rising
        let score = momentum_fade(&input);
        assert!(
            (score - 0.0).abs() < 0.01,
            "Rising composite should have 0 fade, got {score}"
        );
    }

    #[test]
    fn momentum_fade_positive_when_declining() {
        let input = deteriorating_sps_input();
        let score = momentum_fade(&input);
        assert!(
            score > 20.0,
            "Declining composite should have significant fade, got {score}"
        );
    }

    #[test]
    fn volume_abandonment_neutral_at_1_ratio() {
        let mut input = healthy_sps_input();
        input.up_day_avg_vol = 1_000_000.0;
        input.down_day_avg_vol = 1_000_000.0;
        let score = volume_abandonment(&input);
        assert!(
            (score - 50.0).abs() < 0.01,
            "Equal vol should be 50, got {score}"
        );
    }

    #[test]
    fn volume_abandonment_high_when_distribution() {
        let input = deteriorating_sps_input(); // down_vol > up_vol
        let score = volume_abandonment(&input);
        assert!(score > 50.0, "Distribution should be > 50, got {score}");
    }

    #[test]
    fn drawdown_penalty_zero_at_high() {
        let mut input = healthy_sps_input();
        input.current_close = 200.0;
        input.local_high_60d = 200.0;
        assert!((drawdown_penalty(&input) - 0.0).abs() < 0.01);
    }

    #[test]
    fn drawdown_penalty_100_at_20pct() {
        let mut input = healthy_sps_input();
        input.current_close = 160.0;
        input.local_high_60d = 200.0;
        let score = drawdown_penalty(&input);
        assert!(
            (score - 100.0).abs() < 0.01,
            "20% DD should be 100, got {score}"
        );
    }

    #[test]
    fn time_decay_zero_within_grace_period() {
        let mut input = healthy_sps_input();
        input.days_since_new_high = 30;
        assert!((time_decay(&input) - 0.0).abs() < 0.01);
    }

    #[test]
    fn time_decay_increases_after_40_days() {
        let mut input = healthy_sps_input();
        input.days_since_new_high = 50;
        let score = time_decay(&input);
        assert!(
            (score - 20.0).abs() < 0.01,
            "50 days = 10 excess × 2 = 20, got {score}"
        );
    }

    #[test]
    fn concentration_penalty_zero_when_on_target() {
        let input = healthy_sps_input(); // 0.12 actual vs 0.125 target
        let score = concentration_penalty(&input);
        assert!(
            (score - 0.0).abs() < 0.01,
            "On-target should be 0, got {score}"
        );
    }

    #[test]
    fn concentration_penalty_positive_when_oversized() {
        let mut input = healthy_sps_input();
        input.actual_weight = 0.30;
        input.target_weight = 0.125;
        let score = concentration_penalty(&input);
        assert!(
            score > 0.0,
            "Oversized position should have penalty, got {score}"
        );
    }

    // ── CDS tests ───────────────────────────────────────────────────────────

    fn bull_cds_input() -> CdsInput {
        CdsInput {
            candidates_above_70: 8,
            top_candidate_score: 88.0,
            regime: MarketRegime::Bull,
            regime_improving: false,
            regime_deteriorating: false,
            cash_pct: 0.30,
            days_in_cash: 10,
            vix_percentile: 20.0,
            breadth_pct: 75.0,
        }
    }

    #[test]
    fn cds_bull_with_opportunities_is_deploy() {
        let result = compute_deployment_score(&bull_cds_input());
        assert!(
            result.cds > 30.0,
            "Bull + opportunities should be at least DeployCautiously, got CDS={:.1}",
            result.cds
        );
        assert!(
            matches!(
                result.action,
                DeployAction::DeployCautiously
                    | DeployAction::DeployNormally
                    | DeployAction::DeployAggressively
            ),
            "Should recommend some form of deployment, got {:?}",
            result.action
        );
    }

    #[test]
    fn cds_bear_with_no_candidates_is_hold() {
        let input = CdsInput {
            candidates_above_70: 0,
            top_candidate_score: 40.0,
            regime: MarketRegime::Bear,
            regime_improving: false,
            regime_deteriorating: false,
            cash_pct: 0.10,
            days_in_cash: 5,
            vix_percentile: 85.0,
            breadth_pct: 25.0,
        };
        let result = compute_deployment_score(&input);
        assert_eq!(result.action, DeployAction::HoldCash);
    }

    #[test]
    fn cds_regime_improving_adds_10() {
        let mut base = bull_cds_input();
        base.regime = MarketRegime::Sideways;
        base.regime_improving = false;
        let normal = compute_deployment_score(&base);

        base.regime_improving = true;
        let improving = compute_deployment_score(&base);

        assert!(improving.cds > normal.cds, "Improving should boost CDS");
    }

    #[test]
    fn cds_high_vix_reduces_score() {
        let mut low_risk = bull_cds_input();
        low_risk.vix_percentile = 10.0;
        low_risk.breadth_pct = 80.0;

        let mut high_risk = bull_cds_input();
        high_risk.vix_percentile = 90.0;
        high_risk.breadth_pct = 30.0;

        let low = compute_deployment_score(&low_risk);
        let high = compute_deployment_score(&high_risk);
        assert!(
            low.cds > high.cds,
            "Low VIX should have higher CDS than high VIX"
        );
    }

    #[test]
    fn cds_bounded_0_100() {
        let result = compute_deployment_score(&bull_cds_input());
        assert!(result.cds >= 0.0 && result.cds <= 100.0);
    }

    // ── Upgrade Score tests ─────────────────────────────────────────────────

    fn clear_upgrade_input() -> UpgradeInput {
        UpgradeInput {
            incumbent_composite: 60.0,
            incumbent_rs: 50.0,
            incumbent_rs_rising: false,
            incumbent_atr_pct: 2.5,
            incumbent_portfolio_corr: 0.70,
            candidate_composite: 88.0,
            candidate_rs: 90.0,
            candidate_rs_rising: true,
            candidate_atr_pct: 2.0,
            candidate_portfolio_corr: 0.30,
            spread_cost_pct: 0.5,
            tax_cost_pct: 1.5,
            wash_sale_risk: false,
            incumbent_grade: Grade::B,
        }
    }

    #[test]
    fn upgrade_clear_winner_recommends_upgrade() {
        let input = clear_upgrade_input();
        let result = compute_upgrade_score("AAPL", "NVDA", &input);
        assert_eq!(result.recommendation, UpgradeAction::Upgrade);
        assert!(result.upgrade_score > 15.0);
    }

    #[test]
    fn upgrade_similar_stocks_recommends_hold() {
        let input = UpgradeInput {
            incumbent_composite: 75.0,
            incumbent_rs: 80.0,
            incumbent_rs_rising: true,
            incumbent_atr_pct: 2.0,
            incumbent_portfolio_corr: 0.50,
            candidate_composite: 77.0,
            candidate_rs: 82.0,
            candidate_rs_rising: true,
            candidate_atr_pct: 2.1,
            candidate_portfolio_corr: 0.55,
            spread_cost_pct: 1.0,
            tax_cost_pct: 2.0,
            wash_sale_risk: false,
            incumbent_grade: Grade::A,
        };
        let result = compute_upgrade_score("MSFT", "GOOG", &input);
        assert_eq!(result.recommendation, UpgradeAction::Hold);
    }

    #[test]
    fn upgrade_wash_sale_increases_friction() {
        let mut input = clear_upgrade_input();
        let no_wash = compute_upgrade_score("A", "B", &input);

        input.wash_sale_risk = true;
        let with_wash = compute_upgrade_score("A", "B", &input);

        assert!(with_wash.upgrade_score < no_wash.upgrade_score);
    }

    #[test]
    fn upgrade_a_plus_has_higher_threshold() {
        let mut input = clear_upgrade_input();
        input.incumbent_grade = Grade::APlus;
        input.incumbent_composite = 80.0;
        input.candidate_composite = 90.0;
        // Smaller alpha advantage vs A+ → harder to justify
        let _result = compute_upgrade_score("GOOGL", "NEW", &input);
        // With 10-point composite diff, alpha_advantage = 5.0
        // US may not exceed 25.0 (A+ threshold)
        assert!(
            upgrade_threshold(&Grade::APlus) > upgrade_threshold(&Grade::B),
            "A+ threshold should be higher than B"
        );
    }

    #[test]
    fn upgrade_components_have_correct_signs() {
        let input = clear_upgrade_input();
        let result = compute_upgrade_score("A", "B", &input);
        assert!(
            result.alpha_advantage > 0.0,
            "Better candidate should have positive alpha"
        );
        assert!(
            result.momentum_edge > 0.0,
            "Rising candidate should have positive momentum"
        );
        assert!(
            result.cost_friction < 0.0,
            "Friction should always be negative"
        );
    }

    #[test]
    fn upgrade_d_grade_easy_to_replace() {
        let mut input = clear_upgrade_input();
        input.incumbent_grade = Grade::D;
        let result = compute_upgrade_score("WEAK", "STRONG", &input);
        // D-grade threshold is 5.0, much easier to clear
        assert_eq!(result.recommendation, UpgradeAction::Upgrade);
    }

    // ── Anti-churn tests ────────────────────────────────────────────────────

    #[test]
    fn churn_blocks_new_position() {
        assert!(!is_eligible_for_upgrade(5, 30, 0, false)); // only 5 days held
    }

    #[test]
    fn churn_blocks_recent_swap() {
        assert!(!is_eligible_for_upgrade(30, 10, 0, false)); // swapped 10 days ago
    }

    #[test]
    fn churn_blocks_third_weekly_swap() {
        assert!(!is_eligible_for_upgrade(30, 30, 2, false)); // already 2 swaps this week
    }

    #[test]
    fn churn_allows_eligible_position() {
        assert!(is_eligible_for_upgrade(30, 30, 1, false));
    }

    #[test]
    fn churn_hard_exit_bypasses_all() {
        assert!(is_eligible_for_upgrade(1, 1, 5, true)); // everything blocked except hard exit
    }

    // ── Action threshold tests ──────────────────────────────────────────────

    #[test]
    fn sps_action_thresholds() {
        assert_eq!(SpsAction::from_score(0.0), SpsAction::Green);
        assert_eq!(SpsAction::from_score(25.0), SpsAction::Green);
        assert_eq!(SpsAction::from_score(26.0), SpsAction::Yellow);
        assert_eq!(SpsAction::from_score(45.0), SpsAction::Yellow);
        assert_eq!(SpsAction::from_score(46.0), SpsAction::Orange);
        assert_eq!(SpsAction::from_score(65.0), SpsAction::Orange);
        assert_eq!(SpsAction::from_score(66.0), SpsAction::Red);
        assert_eq!(SpsAction::from_score(80.0), SpsAction::Red);
        assert_eq!(SpsAction::from_score(81.0), SpsAction::Critical);
        assert_eq!(SpsAction::from_score(100.0), SpsAction::Critical);
    }

    #[test]
    fn deploy_action_thresholds() {
        assert_eq!(DeployAction::from_score(0.0), DeployAction::HoldCash);
        assert_eq!(DeployAction::from_score(30.0), DeployAction::HoldCash);
        assert_eq!(
            DeployAction::from_score(31.0),
            DeployAction::DeployCautiously
        );
        assert_eq!(DeployAction::from_score(51.0), DeployAction::DeployNormally);
        assert_eq!(
            DeployAction::from_score(71.0),
            DeployAction::DeployAggressively
        );
    }
}
