//! Exit / breakdown signal detection for flagged stocks.
//!
//! Checks stocks that previously passed screening for deterioration:
//! - **Stage 2 Breakdown**: ≥3 Minervini conditions lost since entry
//! - **RS Deterioration**: RS percentile drops below 50 from prior ≥80
//! - **MA Death Cross**: 50-SMA crosses below 200-SMA
//!
//! PRD references: F10.1–F10.7

use crate::analysis::indicators::{check_minervini_conditions, rs_percentile, sma};
use crate::data::models::{Candle, FlaggedStock};

/// Result of checking exit signals on a flagged stock.
#[derive(Debug, Clone)]
pub struct ExitSignal {
    pub ticker: String,
    pub reason: ExitReason,
    pub current_price: f64,
    pub entry_price: f64,
    pub entry_score: f64,
    pub pnl_pct: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitReason {
    /// ≥3 Minervini conditions lost since entry
    Stage2Breakdown {
        conditions_now: u8,
        conditions_at_entry: u8,
    },
    /// RS percentile dropped below 50 (was ≥80 at entry)
    RSDrop { rs_now: u8 },
    /// 50-SMA crossed below 200-SMA
    MADeathCross,
}

impl std::fmt::Display for ExitReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExitReason::Stage2Breakdown {
                conditions_now,
                conditions_at_entry,
            } => {
                write!(
                    f,
                    "Stage2Breakdown ({}/{} conditions)",
                    conditions_now, conditions_at_entry
                )
            }
            ExitReason::RSDrop { rs_now } => write!(f, "RSDrop (RS={}%)", rs_now),
            ExitReason::MADeathCross => write!(f, "MADeathCross (50-SMA < 200-SMA)"),
        }
    }
}

/// Check a flagged stock for exit signals.
///
/// # Arguments
/// - `flagged` — The flagged stock record from the database
/// - `candles` — Current candle history for this ticker (≥252 bars)
/// - `universe_returns` — 252-day returns for all stocks (for RS percentile)
///
/// # Returns
/// `Some(ExitSignal)` if an exit condition is met, `None` if still healthy.
pub fn check_exit_signals(
    flagged: &FlaggedStock,
    candles: &[Candle],
    universe_returns: &[f64],
) -> Option<ExitSignal> {
    if candles.len() < 252 {
        return None; // Not enough data to evaluate
    }

    let current_price = candles.last().unwrap().close;
    let entry_price = flagged.entry_price;
    let pnl_pct = ((current_price - entry_price) / entry_price) * 100.0;

    // 1. Stage 2 Breakdown — check how many Minervini conditions are still met
    if let Some(signal) =
        check_stage2_breakdown(flagged, candles, current_price, pnl_pct, universe_returns)
    {
        return Some(signal);
    }

    // 2. RS Deterioration — RS dropped below 50 from ≥80 at entry
    if let Some(signal) =
        check_rs_deterioration(flagged, candles, current_price, pnl_pct, universe_returns)
    {
        return Some(signal);
    }

    // 3. MA Death Cross — 50-SMA below 200-SMA
    if let Some(signal) = check_ma_death_cross(flagged, candles, current_price, pnl_pct) {
        return Some(signal);
    }

    None
}

/// Stage 2 Breakdown: ≥3 Minervini conditions lost since entry.
fn check_stage2_breakdown(
    flagged: &FlaggedStock,
    candles: &[Candle],
    current_price: f64,
    pnl_pct: f64,
    universe_returns: &[f64],
) -> Option<ExitSignal> {
    let closes: Vec<f64> = candles.iter().map(|c| c.adj_close).collect();
    let ticker_return = crate::analysis::indicators::price_return_252d(candles);
    let rs = ticker_return
        .map(|r| rs_percentile(r, universe_returns))
        .unwrap_or(50.0);

    let mut conditions = check_minervini_conditions(&closes).ok()?;
    // Set RS threshold based on computed RS
    conditions.rs_above_threshold = rs >= 70.0;
    let conditions_now = conditions.count();
    let conditions_at_entry = flagged.conditions_at_entry;

    if conditions_at_entry >= 3 && conditions_now + 3 <= conditions_at_entry {
        Some(ExitSignal {
            ticker: flagged.ticker.clone(),
            reason: ExitReason::Stage2Breakdown {
                conditions_now,
                conditions_at_entry,
            },
            current_price,
            entry_price: flagged.entry_price,
            entry_score: flagged.entry_score,
            pnl_pct,
        })
    } else {
        None
    }
}

/// RS Deterioration: RS dropped below 50 from ≥80 at entry.
fn check_rs_deterioration(
    flagged: &FlaggedStock,
    candles: &[Candle],
    current_price: f64,
    pnl_pct: f64,
    universe_returns: &[f64],
) -> Option<ExitSignal> {
    // Only applies if entry score was high enough to suggest strong RS
    if flagged.entry_score < 70.0 {
        return None;
    }

    let ticker_return = crate::analysis::indicators::price_return_252d(candles)?;
    let rs = rs_percentile(ticker_return, universe_returns);

    if rs < 50.0 {
        Some(ExitSignal {
            ticker: flagged.ticker.clone(),
            reason: ExitReason::RSDrop { rs_now: rs as u8 },
            current_price,
            entry_price: flagged.entry_price,
            entry_score: flagged.entry_score,
            pnl_pct,
        })
    } else {
        None
    }
}

/// MA Death Cross: 50-SMA crossed below 200-SMA.
fn check_ma_death_cross(
    flagged: &FlaggedStock,
    candles: &[Candle],
    current_price: f64,
    pnl_pct: f64,
) -> Option<ExitSignal> {
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();

    let sma50 = sma(&closes, 50).ok()?;
    let sma200 = sma(&closes, 200).ok()?;

    let current_sma50 = *sma50.last()?;
    let current_sma200 = *sma200.last()?;

    // Death cross: 50-SMA below 200-SMA
    if current_sma50 < current_sma200 {
        Some(ExitSignal {
            ticker: flagged.ticker.clone(),
            reason: ExitReason::MADeathCross,
            current_price,
            entry_price: flagged.entry_price,
            entry_score: flagged.entry_score,
            pnl_pct,
        })
    } else {
        None
    }
}

/// Format an exit signal as an alert message.
pub fn format_exit_message(signal: &ExitSignal) -> String {
    let direction = if signal.pnl_pct >= 0.0 { "+" } else { "" };
    format!(
        "{ticker} EXIT — {reason}\n\
         Entry: ${entry:.2} | Current: ${current:.2} | P&L: {dir}{pnl:.1}%\n\
         Entry score: {score:.1}",
        ticker = signal.ticker,
        reason = signal.reason,
        entry = signal.entry_price,
        current = signal.current_price,
        dir = direction,
        pnl = signal.pnl_pct,
        score = signal.entry_score,
    )
}

// ─── Conviction Decay ────────────────────────────────────────────────────────

/// Alert state for conviction decay monitoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvictionAlert {
    /// Score dropped ≥15 points from entry — thesis weakening.
    Weakening,
    /// Score recovered to within 5 points of entry.
    Recovered,
}

impl std::fmt::Display for ConvictionAlert {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConvictionAlert::Weakening => write!(f, "CONVICTION_WEAKENING"),
            ConvictionAlert::Recovered => write!(f, "CONVICTION_RECOVERED"),
        }
    }
}

/// Check for conviction decay between entry score and current re-scored value.
///
/// - **Weakening**: current score dropped ≥15 points below entry.
/// - **Recovered**: current score is within 5 points of entry (after previously weakening).
/// - Returns `None` for moderate changes that don't cross either threshold.
pub fn check_conviction_decay(entry_score: f64, current_score: f64) -> Option<ConvictionAlert> {
    let delta = entry_score - current_score;
    if delta >= 15.0 {
        Some(ConvictionAlert::Weakening)
    } else if delta <= 5.0 {
        Some(ConvictionAlert::Recovered)
    } else {
        None
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::models::{FlaggedStatus, FlaggedStock};
    use chrono::NaiveDate;

    fn make_flagged(
        ticker: &str,
        conditions: u8,
        entry_score: f64,
        entry_price: f64,
    ) -> FlaggedStock {
        FlaggedStock {
            id: Some(1),
            ticker: ticker.to_string(),
            flagged_date: NaiveDate::from_ymd_opt(2024, 1, 1).unwrap(),
            entry_score,
            entry_price,
            conditions_at_entry: conditions,
            status: FlaggedStatus::Active,
            exit_date: None,
            exit_reason: None,
            exit_price: None,
            latest_score: None,
            last_rescored_at: None,
        }
    }

    /// Generate a strong uptrend (no exit signals expected).
    fn strong_uptrend_candles(n: usize) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let base = 100.0 + (i as f64 * 0.5);
                Candle::new(
                    &format!("2023-{:02}-{:02}", (i / 28 % 12) + 1, (i % 28) + 1),
                    base - 1.0,
                    base + 2.0,
                    base - 2.0,
                    base,
                    1_000_000,
                )
            })
            .collect()
    }

    /// Generate a strong downtrend (should trigger exit signals).
    fn downtrend_candles(n: usize) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let base = 300.0 - (i as f64 * 0.5);
                Candle::new(
                    &format!("2023-{:02}-{:02}", (i / 28 % 12) + 1, (i % 28) + 1),
                    base + 1.0,
                    base + 2.0,
                    base - 2.0,
                    base,
                    1_000_000,
                )
            })
            .collect()
    }

    // ── Exit reason display ─────────────────────────────────────────────────

    #[test]
    fn exit_reason_stage2_display() {
        let reason = ExitReason::Stage2Breakdown {
            conditions_now: 3,
            conditions_at_entry: 7,
        };
        let s = reason.to_string();
        assert!(s.contains("Stage2Breakdown"));
        assert!(s.contains("3/7"));
    }

    #[test]
    fn exit_reason_rs_drop_display() {
        let reason = ExitReason::RSDrop { rs_now: 35 };
        assert!(reason.to_string().contains("RSDrop"));
        assert!(reason.to_string().contains("35"));
    }

    #[test]
    fn exit_reason_death_cross_display() {
        assert!(ExitReason::MADeathCross
            .to_string()
            .contains("MADeathCross"));
    }

    // ── Insufficient data ───────────────────────────────────────────────────

    #[test]
    fn check_exit_with_insufficient_data_returns_none() {
        let flagged = make_flagged("NVDA", 7, 85.0, 150.0);
        let candles = strong_uptrend_candles(100); // Need 252
        assert!(check_exit_signals(&flagged, &candles, &[]).is_none());
    }

    // ── MA Death Cross ──────────────────────────────────────────────────────

    #[test]
    fn death_cross_detected_in_downtrend() {
        let flagged = make_flagged("BAD", 7, 85.0, 250.0);
        let candles = downtrend_candles(300);
        let signal = check_ma_death_cross(&flagged, &candles, candles.last().unwrap().close, -20.0);
        assert!(signal.is_some(), "Downtrend should produce death cross");
        assert_eq!(signal.unwrap().reason, ExitReason::MADeathCross);
    }

    #[test]
    fn no_death_cross_in_uptrend() {
        let flagged = make_flagged("GOOD", 7, 85.0, 100.0);
        let candles = strong_uptrend_candles(300);
        let signal = check_ma_death_cross(&flagged, &candles, candles.last().unwrap().close, 50.0);
        assert!(
            signal.is_none(),
            "Strong uptrend should NOT produce death cross"
        );
    }

    // ── Stage 2 Breakdown ───────────────────────────────────────────────────

    #[test]
    fn stage2_breakdown_with_many_conditions_lost() {
        let flagged = make_flagged("BAD", 7, 85.0, 250.0);
        let candles = downtrend_candles(300);
        let signal = check_stage2_breakdown(
            &flagged,
            &candles,
            candles.last().unwrap().close,
            -20.0,
            &[],
        );
        // Downtrend stock should lose many conditions
        assert!(
            signal.is_some(),
            "Downtrending stock should trigger stage2 breakdown"
        );
    }

    #[test]
    fn stage2_no_breakdown_in_uptrend() {
        let flagged = make_flagged("GOOD", 7, 85.0, 100.0);
        let candles = strong_uptrend_candles(300);
        let signal =
            check_stage2_breakdown(&flagged, &candles, candles.last().unwrap().close, 50.0, &[]);
        assert!(
            signal.is_none(),
            "Uptrending stock should NOT trigger stage2 breakdown"
        );
    }

    // ── Format message ──────────────────────────────────────────────────────

    #[test]
    fn format_exit_message_positive_pnl() {
        let signal = ExitSignal {
            ticker: "NVDA".into(),
            reason: ExitReason::MADeathCross,
            current_price: 160.0,
            entry_price: 150.0,
            entry_score: 85.0,
            pnl_pct: 6.67,
        };
        let msg = format_exit_message(&signal);
        assert!(msg.contains("NVDA"));
        assert!(msg.contains("EXIT"));
        assert!(msg.contains("+6.7%"));
        assert!(msg.contains("$150.00"));
        assert!(msg.contains("$160.00"));
    }

    #[test]
    fn format_exit_message_negative_pnl() {
        let signal = ExitSignal {
            ticker: "BAD".into(),
            reason: ExitReason::Stage2Breakdown {
                conditions_now: 2,
                conditions_at_entry: 7,
            },
            current_price: 80.0,
            entry_price: 100.0,
            entry_score: 90.0,
            pnl_pct: -20.0,
        };
        let msg = format_exit_message(&signal);
        assert!(msg.contains("BAD"));
        assert!(msg.contains("-20.0%"));
        assert!(msg.contains("Stage2Breakdown"));
    }

    // ── Full check_exit_signals integration ──────────────────────────────────

    #[test]
    fn full_check_detects_exit_in_downtrend() {
        let flagged = make_flagged("BAD", 7, 85.0, 250.0);
        let candles = downtrend_candles(300);
        let signal = check_exit_signals(&flagged, &candles, &[]);
        assert!(
            signal.is_some(),
            "Downtrend should trigger some exit signal"
        );
    }

    #[test]
    fn full_check_no_exit_in_uptrend() {
        let flagged = make_flagged("GOOD", 7, 85.0, 100.0);
        let candles = strong_uptrend_candles(300);
        // Universe returns — use a low return so our uptrend stock ranks highly
        // Our stock's 252d return is ~0.63 (63%), so give the universe worse returns
        let universe_returns = vec![0.1, 0.2, 0.15, 0.05, 0.3];
        let signal = check_exit_signals(&flagged, &candles, &universe_returns);
        assert!(signal.is_none(), "Strong uptrend should NOT trigger exit");
    }

    // ── Conviction Decay ──────────────────────────────────────────────────────

    #[test]
    fn conviction_weakening_at_15_point_drop() {
        let result = check_conviction_decay(85.0, 70.0);
        assert_eq!(result, Some(ConvictionAlert::Weakening));
    }

    #[test]
    fn conviction_weakening_at_large_drop() {
        let result = check_conviction_decay(90.0, 50.0);
        assert_eq!(result, Some(ConvictionAlert::Weakening));
    }

    #[test]
    fn conviction_recovered_within_5_points() {
        let result = check_conviction_decay(85.0, 82.0);
        assert_eq!(result, Some(ConvictionAlert::Recovered));
    }

    #[test]
    fn conviction_recovered_above_entry() {
        let result = check_conviction_decay(85.0, 90.0);
        assert_eq!(result, Some(ConvictionAlert::Recovered));
    }

    #[test]
    fn conviction_no_alert_moderate_drop() {
        // Drop of 10 points — between 5 and 15, no alert
        let result = check_conviction_decay(85.0, 75.0);
        assert!(
            result.is_none(),
            "10-point drop should not trigger either alert"
        );
    }
}
