//! VIX term structure signal for forward-looking risk assessment.
//!
//! Phase 8 (Wave 4): VIX level alone doesn't capture stress direction.
//! The term structure (VIX vs VIX3M) and SKEW index provide forward-looking
//! signals that have preceded every major drawdown since 2010 by 1-10 days.
//!
//! Key insight: backwardation (VIX > VIX3M) signals acute near-term stress.
//! Contango (VIX < VIX3M) signals calm. SKEW > 140 signals tail-risk demand.

// ─── Types ──────────────────────────────────────────────────────────────────

/// VIX term structure analysis result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct VixTermStructure {
    pub vix: f64,
    pub vix3m: f64,
    /// VIX / VIX3M ratio (>1.0 = backwardation = stress)
    pub ratio: f64,
    /// CBOE SKEW index (>140 = elevated tail risk demand)
    pub skew: f64,
    /// 10-day SMA of equity put/call ratio
    pub put_call_ratio: f64,
    /// Combined signal classification
    pub signal: VixSignal,
    /// Recommended exposure adjustment multiplier
    pub exposure_adjustment: f64,
}

/// VIX signal classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum VixSignal {
    /// Contango, low SKEW — markets calm. Deploy freely.
    RiskOn,
    /// Near flat term structure — no strong signal.
    Neutral,
    /// Backwardation OR elevated SKEW — stress rising, reduce exposure.
    Caution,
    /// Deep backwardation AND high SKEW — acute stress, raise cash.
    StressAlert,
}

impl std::fmt::Display for VixSignal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VixSignal::RiskOn => write!(f, "RISK-ON"),
            VixSignal::Neutral => write!(f, "NEUTRAL"),
            VixSignal::Caution => write!(f, "CAUTION"),
            VixSignal::StressAlert => write!(f, "STRESS"),
        }
    }
}

// ─── Thresholds ─────────────────────────────────────────────────────────────

/// Configurable thresholds for VIX signal classification.
#[derive(Debug, Clone)]
pub struct VixThresholds {
    /// VIX/VIX3M ratio above which = backwardation caution (default: 1.0)
    pub backwardation_threshold: f64,
    /// Deep backwardation threshold (default: 1.05)
    pub deep_backwardation: f64,
    /// Contango threshold below which = risk-on (default: 0.90)
    pub contango_risk_on: f64,
    /// SKEW level above which = elevated tail risk (default: 140)
    pub skew_caution: f64,
    /// SKEW level above which = extreme tail risk (default: 145)
    pub skew_stress: f64,
    /// Put/call ratio above which = extreme fear (contrarian bullish) (default: 1.20)
    pub put_call_fear: f64,
    /// Put/call ratio below which = extreme greed (contrarian bearish) (default: 0.72)
    pub put_call_greed: f64,
}

impl Default for VixThresholds {
    fn default() -> Self {
        Self {
            backwardation_threshold: 1.0,
            deep_backwardation: 1.05,
            contango_risk_on: 0.90,
            skew_caution: 140.0,
            skew_stress: 145.0,
            put_call_fear: 1.20,
            put_call_greed: 0.72,
        }
    }
}

// ─── Core Logic ─────────────────────────────────────────────────────────────

/// Classify VIX term structure signal from raw inputs.
pub fn classify_vix_signal(
    vix: f64,
    vix3m: f64,
    skew: f64,
    put_call_10d_sma: f64,
    thresholds: &VixThresholds,
) -> VixTermStructure {
    let ratio = if vix3m > 0.0 { vix / vix3m } else { 1.0 };

    let signal = determine_signal(ratio, skew, thresholds);
    let exposure = exposure_adjustment(&signal);

    VixTermStructure {
        vix,
        vix3m,
        ratio,
        skew,
        put_call_ratio: put_call_10d_sma,
        signal,
        exposure_adjustment: exposure,
    }
}

fn determine_signal(ratio: f64, skew: f64, t: &VixThresholds) -> VixSignal {
    // StressAlert: deep backwardation AND high SKEW
    if ratio > t.deep_backwardation && skew > t.skew_stress {
        return VixSignal::StressAlert;
    }

    // Caution: backwardation OR elevated SKEW
    if ratio > t.backwardation_threshold || skew > t.skew_caution {
        return VixSignal::Caution;
    }

    // RiskOn: healthy contango, low SKEW
    if ratio < t.contango_risk_on && skew < t.skew_caution {
        return VixSignal::RiskOn;
    }

    VixSignal::Neutral
}

/// Recommended exposure multiplier based on VIX signal.
///
/// - RiskOn: 1.0 (full exposure)
/// - Neutral: 1.0
/// - Caution: 0.75 (reduce by 25%)
/// - StressAlert: 0.50 (reduce by 50%)
pub fn exposure_adjustment(signal: &VixSignal) -> f64 {
    match signal {
        VixSignal::RiskOn => 1.0,
        VixSignal::Neutral => 1.0,
        VixSignal::Caution => 0.75,
        VixSignal::StressAlert => 0.50,
    }
}

/// Classify put/call ratio extremes (contrarian signals).
pub fn put_call_sentiment(ratio: f64, thresholds: &VixThresholds) -> &'static str {
    if ratio > thresholds.put_call_fear {
        "Extreme Fear (contrarian bullish)"
    } else if ratio < thresholds.put_call_greed {
        "Extreme Greed (contrarian bearish)"
    } else {
        "Neutral"
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> VixThresholds {
        VixThresholds::default()
    }

    // ── signal classification ───────────────────────────────────────────

    #[test]
    fn contango_low_skew_is_risk_on() {
        let result = classify_vix_signal(14.0, 17.0, 120.0, 0.85, &defaults());
        assert_eq!(result.signal, VixSignal::RiskOn);
    }

    #[test]
    fn backwardation_is_caution() {
        let result = classify_vix_signal(22.0, 20.0, 130.0, 0.90, &defaults());
        assert_eq!(result.signal, VixSignal::Caution);
    }

    #[test]
    fn deep_backwardation_high_skew_is_stress() {
        let result = classify_vix_signal(28.0, 24.0, 148.0, 1.10, &defaults());
        assert_eq!(result.signal, VixSignal::StressAlert);
    }

    #[test]
    fn normal_conditions_are_neutral() {
        let result = classify_vix_signal(17.0, 18.0, 125.0, 0.88, &defaults());
        assert_eq!(result.signal, VixSignal::Neutral);
    }

    #[test]
    fn high_skew_alone_triggers_caution() {
        // Ratio is normal (contango) but SKEW is elevated
        let result = classify_vix_signal(15.0, 17.0, 142.0, 0.85, &defaults());
        assert_eq!(result.signal, VixSignal::Caution);
    }

    // ── exposure adjustment ─────────────────────────────────────────────

    #[test]
    fn risk_on_full_exposure() {
        assert!((exposure_adjustment(&VixSignal::RiskOn) - 1.0).abs() < 0.01);
    }

    #[test]
    fn caution_reduces_25pct() {
        assert!((exposure_adjustment(&VixSignal::Caution) - 0.75).abs() < 0.01);
    }

    #[test]
    fn stress_reduces_50pct() {
        assert!((exposure_adjustment(&VixSignal::StressAlert) - 0.50).abs() < 0.01);
    }

    // ── ratio computation ───────────────────────────────────────────────

    #[test]
    fn ratio_computed_correctly() {
        let result = classify_vix_signal(20.0, 18.0, 130.0, 0.90, &defaults());
        assert!((result.ratio - 20.0 / 18.0).abs() < 0.01);
    }

    #[test]
    fn zero_vix3m_defaults_to_ratio_1() {
        let result = classify_vix_signal(20.0, 0.0, 130.0, 0.90, &defaults());
        assert!((result.ratio - 1.0).abs() < 0.01);
    }

    // ── put/call sentiment ──────────────────────────────────────────────

    #[test]
    fn extreme_fear_contrarian_bullish() {
        assert_eq!(
            put_call_sentiment(1.30, &defaults()),
            "Extreme Fear (contrarian bullish)"
        );
    }

    #[test]
    fn extreme_greed_contrarian_bearish() {
        assert_eq!(
            put_call_sentiment(0.65, &defaults()),
            "Extreme Greed (contrarian bearish)"
        );
    }

    #[test]
    fn normal_put_call() {
        assert_eq!(put_call_sentiment(0.90, &defaults()), "Neutral");
    }

    // ── combined signal uses worst case ─────────────────────────────────

    #[test]
    fn stress_overrides_contango() {
        // Even if ratio is in contango, deep SKEW can escalate to caution
        let result = classify_vix_signal(14.0, 16.0, 142.0, 0.85, &defaults());
        assert_ne!(
            result.signal,
            VixSignal::RiskOn,
            "High SKEW should override contango"
        );
    }
}
