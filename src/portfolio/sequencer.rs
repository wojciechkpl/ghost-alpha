//! Goal-based sleeve build-out sequencer for Gap 9.
//!
//! Given a portfolio, a set of target sleeve weights, and incoming cash
//! (from RSU sells, concentrated-position unwind, contributions), routes the next dollars
//! to close the gap toward target.
//!
//! Two strategies:
//!   - Greedy: fill most-underfilled sleeve first
//!   - ProRata: split cash by target weights (ignores current state)

use crate::portfolio::types::Portfolio;
use serde::{Deserialize, Serialize};

/// One sleeve in the build-out plan: target ticker + target weight (0.0..1.0).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleeveTarget {
    pub ticker: String,
    pub target_weight: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RoutingStrategy {
    /// Fill most-underfilled sleeve first (largest absolute gap-to-target).
    /// Move to the next-most-underfilled when the first is at target.
    Greedy,
    /// Split incoming cash by target weights regardless of current state.
    /// Even if a sleeve is already over-target, it still gets its proportional share.
    ProRata,
}

/// One allocation decision: ticker + dollars to add.
/// `incoming_cash` minus `sum(allocations)` is the residual (e.g., from
/// a min_lot_size constraint or from proceeds exceeding remaining gaps).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CashAllocation {
    pub ticker: String,
    pub amount_usd: f64,
}

/// Snapshot view of one sleeve at decision time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SleeveStatus {
    pub ticker: String,
    pub target_weight: f64,
    pub current_value: f64,
    pub target_value: f64,
    pub gap_to_target: f64,
}

/// Compute current vs target sleeve status. Total NAV is `portfolio.total_market_value()`
/// PLUS `pending_cash` (the cash being routed — included so target_value reflects post-add NAV).
pub fn sleeve_status(
    portfolio: &Portfolio,
    targets: &[SleeveTarget],
    pending_cash: f64,
) -> Vec<SleeveStatus> {
    let total_nav = portfolio.total_market_value() + pending_cash;
    targets
        .iter()
        .map(|t| {
            let current_value = portfolio
                .position(&t.ticker)
                .map(|p| p.market_value())
                .unwrap_or(0.0);
            let target_value = total_nav * t.target_weight;
            SleeveStatus {
                ticker: t.ticker.clone(),
                target_weight: t.target_weight,
                current_value,
                target_value,
                gap_to_target: target_value - current_value,
            }
        })
        .collect()
}

/// Route incoming cash across sleeves per `strategy`. Returns `(allocations, residual)`.
/// `residual` is cash NOT allocated (remaining after all gaps filled OR below min_lot_size).
///
/// `min_lot_size_usd`: any sleeve allocation < this is rounded to 0; the truncated dollars
/// fall to residual. Use 0.0 to disable.
pub fn route_cash(
    portfolio: &Portfolio,
    targets: &[SleeveTarget],
    incoming_cash: f64,
    strategy: RoutingStrategy,
    min_lot_size_usd: f64,
) -> (Vec<CashAllocation>, f64) {
    if incoming_cash == 0.0 {
        return (Vec::new(), 0.0);
    }
    if incoming_cash < 0.0 {
        return (Vec::new(), incoming_cash);
    }
    if targets.is_empty() {
        return (Vec::new(), incoming_cash);
    }

    match strategy {
        RoutingStrategy::Greedy => {
            route_greedy(portfolio, targets, incoming_cash, min_lot_size_usd)
        }
        RoutingStrategy::ProRata => route_pro_rata(targets, incoming_cash, min_lot_size_usd),
    }
}

fn route_greedy(
    portfolio: &Portfolio,
    targets: &[SleeveTarget],
    incoming_cash: f64,
    min_lot_size_usd: f64,
) -> (Vec<CashAllocation>, f64) {
    let statuses = sleeve_status(portfolio, targets, incoming_cash);

    // Sort by gap_to_target descending; ties broken by input order (stable sort).
    let mut indexed: Vec<(usize, &SleeveStatus)> = statuses.iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        b.1.gap_to_target
            .partial_cmp(&a.1.gap_to_target)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut remaining = incoming_cash;
    let mut allocs: Vec<CashAllocation> = Vec::new();

    for (_, status) in &indexed {
        if remaining <= 0.0 || status.gap_to_target <= 0.0 {
            continue;
        }
        let amount = remaining.min(status.gap_to_target);
        if amount < min_lot_size_usd {
            // Skip this sleeve; cash continues to the next greedy candidate.
            continue;
        }
        allocs.push(CashAllocation {
            ticker: status.ticker.clone(),
            amount_usd: amount,
        });
        remaining -= amount;
    }

    (allocs, remaining)
}

fn route_pro_rata(
    targets: &[SleeveTarget],
    incoming_cash: f64,
    min_lot_size_usd: f64,
) -> (Vec<CashAllocation>, f64) {
    let mut allocs: Vec<CashAllocation> = Vec::new();
    let mut allocated = 0.0;

    for t in targets {
        let amount = incoming_cash * t.target_weight;
        if amount < min_lot_size_usd {
            // Skip; falls to residual.
            continue;
        }
        allocs.push(CashAllocation {
            ticker: t.ticker.clone(),
            amount_usd: amount,
        });
        allocated += amount;
    }

    let residual = incoming_cash - allocated;
    (allocs, residual)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portfolio::types::{AcquisitionType, Portfolio, Position, TaxLot};
    use chrono::NaiveDate;
    use uuid::Uuid;

    // ── Helper ───────────────────────────────────────────────────────────

    /// Build a Portfolio whose positions carry a single synthetic lot each.
    /// `holdings` is a slice of `(ticker, market_value_usd)`.
    /// Each position is priced at $1/share so that shares == market value.
    fn synthetic_portfolio_with_holdings(holdings: &[(&str, f64)]) -> Portfolio {
        let date = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        let positions = holdings
            .iter()
            .map(|(ticker, value)| Position {
                ticker: ticker.to_string(),
                lots: vec![TaxLot {
                    id: Uuid::new_v4(),
                    ticker: ticker.to_string(),
                    shares: *value,
                    cost_basis_per_share: 1.0,
                    acquired_date: date,
                    acquisition_type: AcquisitionType::Buy,
                    remaining_shares: *value,
                }],
                current_price: 1.0,
                last_updated: date,
            })
            .collect();
        Portfolio {
            positions,
            cash: 0.0,
            as_of: date,
        }
    }

    // ── sleeve_status ─────────────────────────────────────────────────────

    #[test]
    fn sleeve_status_computes_gap_correctly() {
        // NAV = $100k, pending_cash = $0 => NAV used for targets = $100k
        let portfolio =
            synthetic_portfolio_with_holdings(&[("AMZN", 50_000.0), ("FNILX", 50_000.0)]);
        let targets = vec![
            SleeveTarget {
                ticker: "AMZN".into(),
                target_weight: 0.20,
            },
            SleeveTarget {
                ticker: "FNILX".into(),
                target_weight: 0.30,
            },
        ];
        let statuses = sleeve_status(&portfolio, &targets, 0.0);
        assert_eq!(statuses.len(), 2);

        let amzn = statuses.iter().find(|s| s.ticker == "AMZN").unwrap();
        assert!((amzn.current_value - 50_000.0).abs() < 1.0);
        assert!((amzn.target_value - 20_000.0).abs() < 1.0);
        assert!((amzn.gap_to_target - (-30_000.0)).abs() < 1.0);

        let fnilx = statuses.iter().find(|s| s.ticker == "FNILX").unwrap();
        assert!((fnilx.current_value - 50_000.0).abs() < 1.0);
        assert!((fnilx.target_value - 30_000.0).abs() < 1.0);
        assert!((fnilx.gap_to_target - (-20_000.0)).abs() < 1.0);
    }

    #[test]
    fn sleeve_status_includes_pending_cash_in_nav() {
        // NAV = $100k positions + $100k pending = $200k for target computation
        let portfolio =
            synthetic_portfolio_with_holdings(&[("AMZN", 50_000.0), ("FNILX", 50_000.0)]);
        let targets = vec![
            SleeveTarget {
                ticker: "AMZN".into(),
                target_weight: 0.20,
            },
            SleeveTarget {
                ticker: "FNILX".into(),
                target_weight: 0.30,
            },
        ];
        let statuses = sleeve_status(&portfolio, &targets, 100_000.0);

        let amzn = statuses.iter().find(|s| s.ticker == "AMZN").unwrap();
        // target_value = $200k × 0.20 = $40k; gap = $40k − $50k = −$10k
        assert!((amzn.target_value - 40_000.0).abs() < 1.0);
        assert!((amzn.gap_to_target - (-10_000.0)).abs() < 1.0);

        let fnilx = statuses.iter().find(|s| s.ticker == "FNILX").unwrap();
        // target_value = $200k × 0.30 = $60k; gap = $60k − $50k = +$10k
        assert!((fnilx.target_value - 60_000.0).abs() < 1.0);
        assert!((fnilx.gap_to_target - 10_000.0).abs() < 1.0);
    }

    #[test]
    fn sleeve_status_creates_status_for_targets_not_in_portfolio() {
        // Portfolio has only AMZN; targets include IEF and TLT which are absent
        let portfolio = synthetic_portfolio_with_holdings(&[("AMZN", 100_000.0)]);
        let targets = vec![
            SleeveTarget {
                ticker: "AMZN".into(),
                target_weight: 0.50,
            },
            SleeveTarget {
                ticker: "IEF".into(),
                target_weight: 0.30,
            },
            SleeveTarget {
                ticker: "TLT".into(),
                target_weight: 0.20,
            },
        ];
        let statuses = sleeve_status(&portfolio, &targets, 0.0);
        assert_eq!(statuses.len(), 3);

        let ief = statuses.iter().find(|s| s.ticker == "IEF").unwrap();
        assert!((ief.current_value).abs() < 1.0);
        assert!((ief.target_value - 30_000.0).abs() < 1.0);
        assert!((ief.gap_to_target - 30_000.0).abs() < 1.0);

        let tlt = statuses.iter().find(|s| s.ticker == "TLT").unwrap();
        assert!((tlt.current_value).abs() < 1.0);
        assert!((tlt.target_value - 20_000.0).abs() < 1.0);
        assert!((tlt.gap_to_target - 20_000.0).abs() < 1.0);
    }

    // ── route_cash — Greedy ───────────────────────────────────────────────

    #[test]
    fn greedy_fills_single_underfilled_sleeve() {
        let portfolio = synthetic_portfolio_with_holdings(&[]);
        let targets = vec![SleeveTarget {
            ticker: "IEF".into(),
            target_weight: 1.0,
        }];
        let (allocs, residual) =
            route_cash(&portfolio, &targets, 50_000.0, RoutingStrategy::Greedy, 0.0);
        assert_eq!(allocs.len(), 1);
        assert_eq!(allocs[0].ticker, "IEF");
        assert!((allocs[0].amount_usd - 50_000.0).abs() < 1.0);
        assert!(residual.abs() < 1.0);
    }

    #[test]
    fn greedy_fills_largest_gap_first() {
        // AMZN $80k, targets AMZN 50% / IEF 50%, cash $20k.
        // NAV+cash = $100k. AMZN target = $50k (gap = −$30k). IEF target = $50k (gap = +$50k).
        // Greedy routes all $20k to IEF (only positive gap).
        let portfolio = synthetic_portfolio_with_holdings(&[("AMZN", 80_000.0)]);
        let targets = vec![
            SleeveTarget {
                ticker: "AMZN".into(),
                target_weight: 0.50,
            },
            SleeveTarget {
                ticker: "IEF".into(),
                target_weight: 0.50,
            },
        ];
        let (allocs, residual) =
            route_cash(&portfolio, &targets, 20_000.0, RoutingStrategy::Greedy, 0.0);
        assert_eq!(allocs.len(), 1);
        assert_eq!(allocs[0].ticker, "IEF");
        assert!((allocs[0].amount_usd - 20_000.0).abs() < 1.0);
        assert!(residual.abs() < 1.0);
    }

    #[test]
    fn greedy_fills_then_overflows_proportionally() {
        // Empty portfolio, targets A 50% / B 50%, cash $200k.
        // NAV+cash = $200k. Both targets $100k, both gaps $100k.
        // Greedy fills A first: $100k to A, then $100k to B. Residual=0.
        let portfolio = synthetic_portfolio_with_holdings(&[]);
        let targets = vec![
            SleeveTarget {
                ticker: "A".into(),
                target_weight: 0.50,
            },
            SleeveTarget {
                ticker: "B".into(),
                target_weight: 0.50,
            },
        ];
        let (allocs, residual) = route_cash(
            &portfolio,
            &targets,
            200_000.0,
            RoutingStrategy::Greedy,
            0.0,
        );
        let total_allocated: f64 = allocs.iter().map(|a| a.amount_usd).sum();
        assert!((total_allocated - 200_000.0).abs() < 1.0);
        assert!(residual.abs() < 1.0);
        // Both sleeves must be represented
        assert!(allocs.iter().any(|a| a.ticker == "A"));
        assert!(allocs.iter().any(|a| a.ticker == "B"));
    }

    #[test]
    fn greedy_residual_when_all_sleeves_already_at_or_over_target() {
        // AMZN $200k, IEF $50k = $250k NAV. Cash $50k. NAV+cash = $300k.
        // Targets: AMZN 50% ($150k), IEF 50% ($150k).
        // AMZN over by $50k. IEF under by $100k. Only IEF has positive gap.
        let portfolio =
            synthetic_portfolio_with_holdings(&[("AMZN", 200_000.0), ("IEF", 50_000.0)]);
        let targets = vec![
            SleeveTarget {
                ticker: "AMZN".into(),
                target_weight: 0.50,
            },
            SleeveTarget {
                ticker: "IEF".into(),
                target_weight: 0.50,
            },
        ];
        let (allocs, residual) =
            route_cash(&portfolio, &targets, 50_000.0, RoutingStrategy::Greedy, 0.0);
        // All $50k should flow to IEF
        assert_eq!(allocs.len(), 1);
        assert_eq!(allocs[0].ticker, "IEF");
        assert!((allocs[0].amount_usd - 50_000.0).abs() < 1.0);
        assert!(residual.abs() < 1.0);
    }

    #[test]
    fn greedy_min_lot_size_truncates_to_residual() {
        // Cash $20, min_lot=$15. A, B, C each gap ~$10 but first sleeve only gets $10 < $15 → skip.
        // Greedy attempt: A gets $10 → below min_lot → skip → remaining $20 to B?
        // Actually: A gap $10, B gap $10, C gap $10. Cash $20, min_lot $15.
        // Greedy fills A first: would allocate min(gap_A, cash) = $10 < $15 → skip A.
        // Move to B: same, skip. Move to C: same, skip. All cash → residual.
        //
        // Clean case: targets A 50%, B 50%. Cash $20, min_lot $15.
        // NAV+cash=$20, A target=$10, B target=$10. Gaps: A=$10, B=$10.
        // A: $10 < $15 → skip. B: $10 < $15 → skip. residual=$20.
        //
        // Actually use: 3 sleeves, cash $20, min_lot $15. First sleeve gets $15 (its gap is $15),
        // remainder $5 < min_lot for remaining sleeves → residual=$5.
        let portfolio = synthetic_portfolio_with_holdings(&[]);
        let targets = vec![
            SleeveTarget {
                ticker: "A".into(),
                target_weight: 0.50,
            }, // target $30, gap $30
            SleeveTarget {
                ticker: "B".into(),
                target_weight: 0.25,
            }, // target $15, gap $15
            SleeveTarget {
                ticker: "C".into(),
                target_weight: 0.25,
            }, // target $15, gap $15
        ];
        // NAV+cash = $0+$20 = $20. Wait — need cash to determine NAV.
        // Use cash=$60 so NAV+cash=$60. A target=$30, B=$15, C=$15. min_lot=$15.
        // Greedy: A gets min($30, $60) = $30 ✓ (>= $15). Remaining $30.
        //         B gets min($15, $30) = $15 ✓ (>= $15). Remaining $15.
        //         C gets min($15, $15) = $15 ✓ (>= $15). Remaining $0.
        // That fills everything. Use a case where remainder < min_lot:
        // Cash=$20, min_lot=$15. NAV+cash=$20. A target=$10, B=$5, C=$5.
        // Hmm, all < $15. Try: A 75%, B 25% with cash=$20. NAV+cash=$20.
        // A target=$15, gap=$15. B target=$5, gap=$5.
        // A: $15 >= $15 → allocate. Remaining $5. B: $5 < $15 → skip. residual=$5.
        let portfolio2 = synthetic_portfolio_with_holdings(&[]);
        let targets2 = vec![
            SleeveTarget {
                ticker: "A".into(),
                target_weight: 0.75,
            },
            SleeveTarget {
                ticker: "B".into(),
                target_weight: 0.25,
            },
        ];
        let (allocs, residual) =
            route_cash(&portfolio2, &targets2, 20.0, RoutingStrategy::Greedy, 15.0);
        assert_eq!(allocs.len(), 1);
        assert_eq!(allocs[0].ticker, "A");
        assert!((allocs[0].amount_usd - 15.0).abs() < 0.01);
        assert!((residual - 5.0).abs() < 0.01);

        // Suppress unused-variable warning from first portfolio
        let _ = portfolio;
        let _ = targets;
    }

    // ── route_cash — ProRata ──────────────────────────────────────────────

    #[test]
    fn pro_rata_splits_cash_by_target_weights_ignoring_current_state() {
        // Targets A 60%, B 40%, cash $100k. Expected: A=$60k, B=$40k, residual=0.
        let portfolio = synthetic_portfolio_with_holdings(&[("A", 999_999.0), ("B", 999_999.0)]);
        let targets = vec![
            SleeveTarget {
                ticker: "A".into(),
                target_weight: 0.60,
            },
            SleeveTarget {
                ticker: "B".into(),
                target_weight: 0.40,
            },
        ];
        let (allocs, residual) = route_cash(
            &portfolio,
            &targets,
            100_000.0,
            RoutingStrategy::ProRata,
            0.0,
        );
        let a = allocs.iter().find(|a| a.ticker == "A").unwrap();
        let b = allocs.iter().find(|a| a.ticker == "B").unwrap();
        assert!((a.amount_usd - 60_000.0).abs() < 1.0);
        assert!((b.amount_usd - 40_000.0).abs() < 1.0);
        assert!(residual.abs() < 1.0);
    }

    #[test]
    fn pro_rata_does_not_normalize_weights_below_one() {
        // Confirm the non-normalization contract: cash × weight, not cash × (weight/sum).
        // Targets A 0.4, B 0.4 (sum=0.8). cash $100k.
        // If normalized: each gets $50k. If NOT normalized: each gets $40k, residual=$20k.
        // Contract: NOT normalized — each gets $40k.
        let portfolio = synthetic_portfolio_with_holdings(&[]);
        let targets = vec![
            SleeveTarget {
                ticker: "A".into(),
                target_weight: 0.40,
            },
            SleeveTarget {
                ticker: "B".into(),
                target_weight: 0.40,
            },
        ];
        let (allocs, residual) = route_cash(
            &portfolio,
            &targets,
            100_000.0,
            RoutingStrategy::ProRata,
            0.0,
        );
        let a = allocs.iter().find(|a| a.ticker == "A").unwrap();
        let b = allocs.iter().find(|a| a.ticker == "B").unwrap();
        assert!((a.amount_usd - 40_000.0).abs() < 1.0);
        assert!((b.amount_usd - 40_000.0).abs() < 1.0);
        assert!((residual - 20_000.0).abs() < 1.0);
    }

    #[test]
    fn pro_rata_with_target_weights_summing_to_one_zero_residual() {
        // Targets A 70%, B 30%, cash $100k. Sum=1.0. Residual=0.
        let portfolio = synthetic_portfolio_with_holdings(&[]);
        let targets = vec![
            SleeveTarget {
                ticker: "A".into(),
                target_weight: 0.70,
            },
            SleeveTarget {
                ticker: "B".into(),
                target_weight: 0.30,
            },
        ];
        let (allocs, residual) = route_cash(
            &portfolio,
            &targets,
            100_000.0,
            RoutingStrategy::ProRata,
            0.0,
        );
        let a = allocs.iter().find(|a| a.ticker == "A").unwrap();
        let b = allocs.iter().find(|a| a.ticker == "B").unwrap();
        assert!((a.amount_usd - 70_000.0).abs() < 1.0);
        assert!((b.amount_usd - 30_000.0).abs() < 1.0);
        assert!(residual.abs() < 1.0);
    }

    #[test]
    fn pro_rata_with_target_weights_summing_under_one_residual_left() {
        // Targets A 0.5, B 0.3 (sum=0.8). Cash $100k.
        // allocation = cash × target_weight (no normalization).
        // A=$50k, B=$30k, residual=$20k (= $100k × 0.2).
        let portfolio = synthetic_portfolio_with_holdings(&[]);
        let targets = vec![
            SleeveTarget {
                ticker: "A".into(),
                target_weight: 0.50,
            },
            SleeveTarget {
                ticker: "B".into(),
                target_weight: 0.30,
            },
        ];
        let (allocs, residual) = route_cash(
            &portfolio,
            &targets,
            100_000.0,
            RoutingStrategy::ProRata,
            0.0,
        );
        let a = allocs.iter().find(|a| a.ticker == "A").unwrap();
        let b = allocs.iter().find(|a| a.ticker == "B").unwrap();
        assert!((a.amount_usd - 50_000.0).abs() < 1.0);
        assert!((b.amount_usd - 30_000.0).abs() < 1.0);
        assert!((residual - 20_000.0).abs() < 1.0);
    }

    // ── Edge cases ────────────────────────────────────────────────────────

    #[test]
    fn route_cash_zero_incoming_returns_empty_allocations() {
        let portfolio = synthetic_portfolio_with_holdings(&[("AMZN", 100_000.0)]);
        let targets = vec![SleeveTarget {
            ticker: "AMZN".into(),
            target_weight: 1.0,
        }];
        let (allocs, residual) =
            route_cash(&portfolio, &targets, 0.0, RoutingStrategy::Greedy, 0.0);
        assert!(allocs.is_empty());
        assert!(residual.abs() < 1.0);
    }

    #[test]
    fn route_cash_no_targets_returns_empty_allocations_full_residual() {
        let portfolio = synthetic_portfolio_with_holdings(&[("AMZN", 100_000.0)]);
        let (allocs, residual) = route_cash(&portfolio, &[], 100.0, RoutingStrategy::Greedy, 0.0);
        assert!(allocs.is_empty());
        assert!((residual - 100.0).abs() < 0.01);
    }

    #[test]
    fn route_cash_negative_cash_returns_empty_allocations() {
        let portfolio = synthetic_portfolio_with_holdings(&[("AMZN", 100_000.0)]);
        let targets = vec![SleeveTarget {
            ticker: "AMZN".into(),
            target_weight: 1.0,
        }];
        let (allocs, residual) =
            route_cash(&portfolio, &targets, -1.0, RoutingStrategy::Greedy, 0.0);
        assert!(allocs.is_empty());
        assert!((residual - (-1.0)).abs() < 0.01);
    }
}
