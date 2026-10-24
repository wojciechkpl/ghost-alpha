//! Plain-English narrative generation for outbound alerts.
//!
//! All functions in this module are pure — no side effects, no I/O.
//! The entry point is [`generate_narrative`], which translates a
//! [`ScreenResult`] into a human-readable summary suitable for
//! non-technical recipients on Telegram or Discord.
//!
//! # Example
//!
//! ```text
//! NVDA is showing one of the strongest setups in the market right now.
//! It passes all 8 Minervini trend conditions. It ranks in the top 3%
//! for relative strength. Volume surged to 2.8x normal levels —
//! institutional interest likely. Market regime is bullish.
//! Score: 94.2/100 (A+) | Ghost Alpha
//! ```

use crate::analysis::screener::ScreenResult;
use crate::data::models::MarketRegime;

// ─── Public entry point ───────────────────────────────────────────────────────

/// Generate a plain-English narrative for a screened stock.
///
/// The returned string is self-contained and ready to be set as the
/// `narrative` field on an [`crate::alerts::channel::OutboundAlert`].
pub fn generate_narrative(result: &ScreenResult) -> String {
    let mut parts: Vec<String> = Vec::new();

    // Opening sentence — what's happening overall
    parts.push(format!(
        "{} is showing {} in the market right now.",
        result.ticker,
        strength_description(result.composite_score),
    ));

    // Minervini conditions
    parts.push(minervini_sentence(result));

    // Relative strength
    parts.push(rs_sentence(result));

    // Volume
    parts.push(volume_sentence(result));

    // Regime
    parts.push(regime_sentence(&result.regime));

    // Score footer
    parts.push(score_footer(result));

    parts.join(" ")
}

/// Generate a narrative for an exit signal.
///
/// Used when a flagged stock starts losing its Minervini conditions or
/// RS percentile drops significantly.
pub fn generate_exit_narrative(ticker: &str, message: &str) -> String {
    format!(
        "{ticker} may be signalling a position review. {message} \
         Consider reviewing your position size and stop-loss levels.",
    )
}

// ─── Helper sentences ─────────────────────────────────────────────────────────

/// Map a composite score to a short strength phrase.
pub fn strength_description(score: f64) -> &'static str {
    match score as u32 {
        90..=100 => "one of the strongest setups",
        80..=89 => "a very strong setup",
        70..=79 => "a solid setup",
        _ => "a notable setup",
    }
}

/// Produce a sentence describing how many Minervini conditions are met.
pub fn minervini_sentence(result: &ScreenResult) -> String {
    if result.minervini_pass {
        format!(
            "It passes all {} Minervini trend conditions.",
            result.conditions_met,
        )
    } else {
        format!(
            "It currently meets {} of 8 Minervini trend conditions.",
            result.conditions_met,
        )
    }
}

/// Produce a sentence describing the relative strength percentile.
pub fn rs_sentence(result: &ScreenResult) -> String {
    let pct = result.rs_percentile;
    if pct >= 90.0 {
        let top = (100.0 - pct) as u32;
        format!("It ranks in the top {}% for relative strength.", top.max(1))
    } else if pct >= 70.0 {
        format!(
            "Relative strength is above average at the {}th percentile.",
            pct as u32,
        )
    } else {
        format!(
            "Relative strength is moderate at the {}th percentile.",
            pct as u32,
        )
    }
}

/// Produce a sentence describing volume activity.
pub fn volume_sentence(result: &ScreenResult) -> String {
    let z = result.volume_zscore;
    if z >= 2.0 {
        format!(
            "Volume surged to {:.1}x normal levels — institutional interest likely.",
            z,
        )
    } else if z >= 1.0 {
        format!("Volume is elevated at {:.1}x normal.", z)
    } else {
        "Volume is near average.".to_string()
    }
}

/// Produce a sentence describing the current market regime.
pub fn regime_sentence(regime: &MarketRegime) -> String {
    match regime {
        MarketRegime::Bull => "Market regime is bullish.".to_string(),
        MarketRegime::Sideways => "Market regime is neutral — trade with caution.".to_string(),
        MarketRegime::Bear => "Market regime is bearish — manage risk carefully.".to_string(),
    }
}

/// Produce the score/grade footer line.
fn score_footer(result: &ScreenResult) -> String {
    format!(
        "Score: {:.1}/100 ({}) | Ghost Alpha",
        result.composite_score, result.grade,
    )
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::indicators::LiquidityClass;
    use crate::analysis::scorer::Grade;
    use crate::data::models::MarketRegime;

    fn make_result(
        score: f64,
        conditions: u8,
        rs_pct: f64,
        vol_z: f64,
        regime: MarketRegime,
    ) -> ScreenResult {
        ScreenResult {
            ticker: "NVDA".to_string(),
            minervini_pass: conditions >= 8,
            conditions_met: conditions,
            adx: 35.0,
            rs_percentile: rs_pct,
            volume_zscore: vol_z,
            technical_score: score * 0.55,
            volume_score: score * 0.30,
            sentiment_score: 0.0,
            composite_score: score,
            grade: Grade::A,
            close_price: 142.0,
            pct_from_52w_high: -4.5,
            atr_value: 3.5,
            risk_reward: 3.2,
            liquidity: LiquidityClass::High,
            weighted_rs: 0.65,
            rs_momentum: None,
            vcp: None,
            regime,
        }
    }

    // ── strength_description ────────────────────────────────────────────────

    #[test]
    fn strength_description_score_95_is_strongest() {
        assert_eq!(strength_description(95.0), "one of the strongest setups");
    }

    #[test]
    fn strength_description_score_90_is_strongest() {
        assert_eq!(strength_description(90.0), "one of the strongest setups");
    }

    #[test]
    fn strength_description_score_85_is_very_strong() {
        assert_eq!(strength_description(85.0), "a very strong setup");
    }

    #[test]
    fn strength_description_score_80_is_very_strong() {
        assert_eq!(strength_description(80.0), "a very strong setup");
    }

    #[test]
    fn strength_description_score_75_is_solid() {
        assert_eq!(strength_description(75.0), "a solid setup");
    }

    #[test]
    fn strength_description_score_70_is_solid() {
        assert_eq!(strength_description(70.0), "a solid setup");
    }

    #[test]
    fn strength_description_score_60_is_notable() {
        assert_eq!(strength_description(60.0), "a notable setup");
    }

    #[test]
    fn strength_description_score_zero_is_notable() {
        assert_eq!(strength_description(0.0), "a notable setup");
    }

    // ── minervini_sentence ──────────────────────────────────────────────────

    #[test]
    fn minervini_sentence_full_pass() {
        let r = make_result(92.0, 8, 97.0, 2.8, MarketRegime::Bull);
        let s = minervini_sentence(&r);
        assert!(s.contains("passes all 8"), "Expected full pass: {s}");
    }

    #[test]
    fn minervini_sentence_partial() {
        let r = make_result(72.0, 5, 75.0, 1.0, MarketRegime::Sideways);
        let s = minervini_sentence(&r);
        assert!(s.contains("5 of 8"), "Expected partial: {s}");
    }

    #[test]
    fn minervini_sentence_zero_conditions() {
        let r = make_result(52.0, 0, 55.0, 0.5, MarketRegime::Bear);
        let s = minervini_sentence(&r);
        assert!(s.contains("0 of 8"), "Expected zero conditions: {s}");
    }

    // ── rs_sentence ─────────────────────────────────────────────────────────

    #[test]
    fn rs_sentence_top_3_percent() {
        let r = make_result(92.0, 8, 97.0, 2.8, MarketRegime::Bull);
        let s = rs_sentence(&r);
        assert!(s.contains("top 3%"), "Expected top 3%: {s}");
    }

    #[test]
    fn rs_sentence_top_10_percent() {
        let r = make_result(85.0, 8, 90.0, 1.5, MarketRegime::Bull);
        let s = rs_sentence(&r);
        assert!(s.contains("top 10%"), "Expected top 10%: {s}");
    }

    #[test]
    fn rs_sentence_above_average() {
        let r = make_result(75.0, 6, 78.0, 0.5, MarketRegime::Sideways);
        let s = rs_sentence(&r);
        assert!(s.contains("above average"), "Expected above average: {s}");
        assert!(s.contains("78th"), "Expected 78th: {s}");
    }

    #[test]
    fn rs_sentence_moderate() {
        let r = make_result(55.0, 4, 55.0, 0.2, MarketRegime::Sideways);
        let s = rs_sentence(&r);
        assert!(s.contains("moderate"), "Expected moderate: {s}");
        assert!(s.contains("55th"), "Expected 55th: {s}");
    }

    #[test]
    fn rs_sentence_zero_percentile_is_moderate() {
        let r = make_result(50.0, 3, 0.0, 0.0, MarketRegime::Bear);
        let s = rs_sentence(&r);
        assert!(s.contains("moderate"), "Expected moderate for 0th: {s}");
    }

    #[test]
    fn rs_sentence_exactly_100_percentile_clamps_to_1() {
        let r = make_result(95.0, 8, 100.0, 3.0, MarketRegime::Bull);
        let s = rs_sentence(&r);
        // top (100 - 100) = 0, clamped to 1
        assert!(s.contains("top 1%"), "Expected top 1%: {s}");
    }

    // ── volume_sentence ─────────────────────────────────────────────────────

    #[test]
    fn volume_sentence_surge_above_2x() {
        let r = make_result(92.0, 8, 97.0, 2.8, MarketRegime::Bull);
        let s = volume_sentence(&r);
        assert!(s.contains("surged"), "Expected surge mention: {s}");
        assert!(s.contains("2.8x"), "Expected 2.8x: {s}");
        assert!(
            s.contains("institutional"),
            "Expected institutional mention: {s}"
        );
    }

    #[test]
    fn volume_sentence_exactly_2x_is_surge() {
        let r = make_result(85.0, 8, 90.0, 2.0, MarketRegime::Bull);
        let s = volume_sentence(&r);
        assert!(s.contains("surged"), "Expected surge at exactly 2.0x: {s}");
    }

    #[test]
    fn volume_sentence_elevated() {
        let r = make_result(75.0, 6, 80.0, 1.5, MarketRegime::Bull);
        let s = volume_sentence(&r);
        assert!(s.contains("elevated"), "Expected elevated: {s}");
        assert!(s.contains("1.5x"), "Expected 1.5x: {s}");
    }

    #[test]
    fn volume_sentence_near_average() {
        let r = make_result(55.0, 4, 65.0, 0.3, MarketRegime::Sideways);
        let s = volume_sentence(&r);
        assert!(s.contains("near average"), "Expected near average: {s}");
    }

    #[test]
    fn volume_sentence_zero_zscore_is_near_average() {
        let r = make_result(50.0, 3, 60.0, 0.0, MarketRegime::Bear);
        let s = volume_sentence(&r);
        assert!(
            s.contains("near average"),
            "Expected near average for 0: {s}"
        );
    }

    #[test]
    fn volume_sentence_negative_zscore_is_near_average() {
        let r = make_result(50.0, 3, 60.0, -1.5, MarketRegime::Bear);
        let s = volume_sentence(&r);
        assert!(
            s.contains("near average"),
            "Expected near average for negative z: {s}"
        );
    }

    // ── regime_sentence ─────────────────────────────────────────────────────

    #[test]
    fn regime_sentence_bull() {
        let s = regime_sentence(&MarketRegime::Bull);
        assert!(s.contains("bullish"), "Expected bullish: {s}");
    }

    #[test]
    fn regime_sentence_sideways() {
        let s = regime_sentence(&MarketRegime::Sideways);
        assert!(s.contains("neutral"), "Expected neutral: {s}");
    }

    #[test]
    fn regime_sentence_bear() {
        let s = regime_sentence(&MarketRegime::Bear);
        assert!(s.contains("bearish"), "Expected bearish: {s}");
    }

    // ── generate_narrative ──────────────────────────────────────────────────

    #[test]
    fn generate_narrative_critical_contains_all_sections() {
        let r = make_result(94.2, 8, 97.0, 2.8, MarketRegime::Bull);
        let text = generate_narrative(&r);

        assert!(!text.is_empty(), "Narrative must not be empty");
        assert!(text.contains("NVDA"), "Must contain ticker");
        assert!(text.contains("strongest"), "Must describe strength");
        assert!(text.contains("Minervini"), "Must mention Minervini");
        assert!(text.contains("relative strength"), "Must mention RS");
        assert!(text.contains("Volume"), "Must mention volume");
        assert!(text.contains("bullish"), "Must mention regime");
        assert!(text.contains("Score:"), "Must have score footer");
        assert!(text.contains("Ghost Alpha"), "Must have branding");
    }

    #[test]
    fn generate_narrative_warning_severity() {
        let r = make_result(75.0, 6, 78.0, 1.0, MarketRegime::Sideways);
        let text = generate_narrative(&r);
        assert!(!text.is_empty());
        assert!(
            text.contains("solid setup")
                || text.contains("notable setup")
                || text.contains("very strong setup")
        );
        assert!(text.contains("neutral") || text.contains("caution"));
    }

    #[test]
    fn generate_narrative_info_severity() {
        let r = make_result(55.0, 3, 55.0, 0.5, MarketRegime::Bear);
        let text = generate_narrative(&r);
        assert!(!text.is_empty());
        assert!(text.contains("bearish") || text.contains("risk"));
    }

    #[test]
    fn generate_narrative_is_non_empty_for_all_regimes() {
        for regime in [
            MarketRegime::Bull,
            MarketRegime::Sideways,
            MarketRegime::Bear,
        ] {
            let r = make_result(82.0, 7, 88.0, 1.8, regime);
            let text = generate_narrative(&r);
            assert!(
                !text.is_empty(),
                "Narrative should be non-empty for all regimes"
            );
        }
    }

    #[test]
    fn generate_narrative_golden_snapshot() {
        let r = make_result(94.2, 8, 97.0, 2.8, MarketRegime::Bull);
        let text = generate_narrative(&r);
        // Snapshot test: verify structure, not exact wording
        assert!(text.starts_with("NVDA is showing one of the strongest setups"));
        assert!(text.contains("passes all 8 Minervini"));
        assert!(text.contains("top 3%"));
        assert!(text.contains("2.8x normal levels"));
        assert!(text.contains("Market regime is bullish"));
        assert!(text.contains("Score: 94.2/100"));
    }

    // ── generate_exit_narrative ─────────────────────────────────────────────

    #[test]
    fn generate_exit_narrative_contains_ticker_and_message() {
        let text = generate_exit_narrative("NVDA", "Lost 3 of 8 Minervini conditions.");
        assert!(text.contains("NVDA"), "Must contain ticker");
        assert!(
            text.contains("Lost 3 of 8"),
            "Must contain original message"
        );
        assert!(text.contains("review"), "Must suggest review");
    }

    #[test]
    fn generate_exit_narrative_non_empty() {
        let text = generate_exit_narrative("AMD", "RS dropped 35 points.");
        assert!(!text.is_empty());
    }
}
