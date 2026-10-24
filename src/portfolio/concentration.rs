//! Concentration-floor solver. Pure math kernel used by Gap 4's
//! `Treatment::OptionBUnwind` in `project_path`.

/// Solve for the maximum shares of the concentrated position sellable in a
/// period such that post-sale concentrated/NAV stays at or above `floor_pct`.
///
/// Inputs (all in consistent units — shares, $, fraction):
/// - `concentrated_shares`: current holdings of the concentrated position (pre-sale)
/// - `price_eoy`: year-end price of the concentrated position
/// - `other_eoy`: total $ value of all other holdings at year-end
///   (assumed already compounded; the new $ from the sale will be added on top)
/// - `net_per_share`: after-tax $ proceeds per share sold
/// - `floor_pct`: minimum concentrated/NAV ratio (e.g. 0.35 for 35% floor)
///
/// Returns `max(0, K_max)`. Caller should further cap at `concentrated_shares`.
///
/// Edge cases:
/// - If `floor_pct >= 1.0` or `floor_pct <= 0.0`, returns 0.0 (degenerate).
/// - If `price_eoy <= 0.0`, returns 0.0 (would divide by zero or near-zero).
/// - If `concentrated_shares <= 0.0`, returns 0.0 (nothing to sell).
pub fn compute_k_max(
    concentrated_shares: f64,
    price_eoy: f64,
    other_eoy: f64,
    net_per_share: f64,
    floor_pct: f64,
) -> f64 {
    if concentrated_shares <= 0.0 || price_eoy <= 0.0 {
        return 0.0;
    }
    if floor_pct >= 1.0 || floor_pct <= 0.0 {
        return 0.0;
    }

    let ratio = floor_pct / (1.0 - floor_pct);
    let k_max_num = concentrated_shares * price_eoy - ratio * other_eoy;
    let k_max_den = price_eoy + ratio * net_per_share;

    if k_max_den.abs() < 1e-12 {
        return 0.0;
    }
    (k_max_num / k_max_den).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Algebraic correctness ────────────────────────────────────────────────

    #[test]
    fn k_max_zero_when_amzn_already_at_floor() {
        // AMZN = 100 × $100 = $10 000; non_amzn = $18 571.43
        // NAV = $28 571.43; AMZN/NAV = 35.00% — exactly at floor
        // ratio = 0.35/0.65 ≈ 0.53846; numerator ≈ 10000 - 0.53846×18571.43 ≈ 0
        let k = compute_k_max(100.0, 100.0, 18_571.43, 80.0, 0.35);
        assert!(k.abs() < 1e-9, "expected ~0.0, got {k}");
    }

    #[test]
    fn k_max_zero_when_amzn_below_floor() {
        // AMZN = $10 000, non_amzn = $50 000 → AMZN/NAV ≈ 16.7% < 35% floor
        // numerator is negative → clamped to 0
        let k = compute_k_max(100.0, 100.0, 50_000.0, 80.0, 0.35);
        assert_eq!(k, 0.0, "below floor: expected 0.0, got {k}");
    }

    #[test]
    fn k_max_positive_when_amzn_above_floor() {
        // AMZN = 1000 × $100 = $100 000, non_amzn = $10 000 → AMZN/NAV ≈ 90.9%
        let k = compute_k_max(1000.0, 100.0, 10_000.0, 80.0, 0.35);
        assert!(
            k > 0.0,
            "well above floor: expected positive K_max, got {k}"
        );
    }

    #[test]
    fn k_max_post_sale_satisfies_floor_exactly() {
        // Same inputs as above; verify the returned K drives AMZN/NAV to floor boundary
        let amzn_shares = 1000.0_f64;
        let price_eoy = 100.0_f64;
        let non_amzn_eoy = 10_000.0_f64;
        let net_per_share = 80.0_f64;
        let floor_pct = 0.35_f64;

        let k = compute_k_max(
            amzn_shares,
            price_eoy,
            non_amzn_eoy,
            net_per_share,
            floor_pct,
        );

        let amzn_eoy = (amzn_shares - k) * price_eoy;
        let nav_post = amzn_eoy + non_amzn_eoy + k * net_per_share;
        let ratio_post = amzn_eoy / nav_post;
        assert!(
            (ratio_post - floor_pct).abs() < 1e-6,
            "post-sale ratio {ratio_post} should equal floor {floor_pct}"
        );
    }

    #[test]
    fn k_max_handles_zero_net_per_share() {
        // net_per_share=0 → den = price_eoy; ratio = 0.35/0.65 ≈ 0.538462
        // k_max = (1000×100 - 0.538462×10000) / 100 = (100000 - 5384.62) / 100 ≈ 946.15
        let k = compute_k_max(1000.0, 100.0, 10_000.0, 0.0, 0.35);
        assert!((k - 946.15).abs() < 0.1, "expected ≈946.15, got {k}");
    }

    // ── Edge cases ───────────────────────────────────────────────────────────

    #[test]
    fn k_max_returns_zero_when_floor_is_one() {
        // floor=1.0 → ratio = 1/(1-1) = 1/0 — degenerate; guard returns 0.0
        let k = compute_k_max(1000.0, 100.0, 10_000.0, 80.0, 1.0);
        assert_eq!(k, 0.0, "floor=1.0 degenerate: expected 0.0, got {k}");
    }

    #[test]
    fn k_max_returns_zero_when_floor_is_zero() {
        // floor=0 → ratio=0 → K_max = amzn_shares (formula degenerates to "sell all")
        // guard: floor<=0 is degenerate → return 0.0
        let k = compute_k_max(1000.0, 100.0, 10_000.0, 80.0, 0.0);
        assert_eq!(k, 0.0, "floor=0.0 degenerate: expected 0.0, got {k}");
    }

    #[test]
    fn k_max_returns_zero_when_amzn_shares_is_zero() {
        let k = compute_k_max(0.0, 100.0, 10_000.0, 80.0, 0.35);
        assert_eq!(k, 0.0, "zero shares: expected 0.0, got {k}");
    }

    #[test]
    fn k_max_returns_zero_when_price_eoy_is_zero() {
        // den = 0 + ratio*net_per_share could be nonzero, but price_eoy guard fires first
        let k = compute_k_max(1000.0, 0.0, 10_000.0, 80.0, 0.35);
        assert_eq!(k, 0.0, "zero price: expected 0.0, got {k}");
    }

    #[test]
    fn k_max_returns_zero_when_price_eoy_is_negative() {
        let k = compute_k_max(1000.0, -50.0, 10_000.0, 80.0, 0.35);
        assert_eq!(k, 0.0, "negative price: expected 0.0, got {k}");
    }

    // ── Realistic-scale sanity (Gap 4 integration) ──────────────────────────

    #[test]
    fn k_max_for_example_portfolio_year_1() {
        // Year 1 estimates for a synthetic concentrated holding:
        //   concentrated_shares = 5000
        //   price_eoy           = 100.00
        //   non_concentrated_eoy = 300_000
        //   net_per_share       = 80.0 (after tax)
        //   floor = 0.35
        // Concentration at start is ~62% — well above floor, so K_max > 1000
        let k = compute_k_max(5000.0, 100.0, 300_000.0, 80.0, 0.35);
        assert!(
            k > 1000.0,
            "year-1 example portfolio: expected K_max > 1000, got {k}"
        );
    }
}
