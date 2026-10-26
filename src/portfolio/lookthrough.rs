//! Pure maths for the concentration hub: position valuation, concentration
//! summary, ETF look-through exposure and post-purchase weight.
//!
//! No I/O; the web layer feeds it lots, closes and N-PORT exposures.

use std::collections::{BTreeMap, HashMap};

use crate::storage::db::TaxLotRow;

/// One ticker's aggregated position.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionValue {
    pub ticker: String,
    pub shares: f64,
    /// Latest close, or the share-weighted average cost when `priced_at_cost`.
    pub price: f64,
    pub value: f64,
    pub priced_at_cost: bool,
}

/// Aggregate lots by ticker and value them at the latest close.
///
/// A ticker without a close is valued at its share-weighted average cost basis
/// and flagged `priced_at_cost`. Lots with non-positive shares are ignored.
/// Sorted by value descending, ties by ticker ascending.
pub fn value_positions(lots: &[TaxLotRow], closes: &HashMap<String, f64>) -> Vec<PositionValue> {
    // ticker -> (shares, total cost)
    let mut agg: BTreeMap<&str, (f64, f64)> = BTreeMap::new();
    for lot in lots.iter().filter(|l| l.shares > 0.0) {
        let e = agg.entry(lot.ticker.as_str()).or_insert((0.0, 0.0));
        e.0 += lot.shares;
        e.1 += lot.shares * lot.cost_basis_per_share;
    }
    let mut out: Vec<PositionValue> = agg
        .into_iter()
        .map(|(ticker, (shares, cost))| {
            let (price, priced_at_cost) = match closes.get(ticker) {
                Some(c) => (*c, false),
                None => (cost / shares, true),
            };
            PositionValue {
                ticker: ticker.to_string(),
                shares,
                price,
                value: shares * price,
                priced_at_cost,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.value
            .total_cmp(&a.value)
            .then_with(|| a.ticker.cmp(&b.ticker))
    });
    out
}

/// Concentration figures for one symbol within a portfolio.
#[derive(Debug, Clone, PartialEq)]
pub struct ConcentrationSummary {
    /// Upper-cased symbol the figures are about.
    pub symbol: String,
    pub total_value: f64,
    /// 0.0 when the symbol is not held.
    pub direct_value: f64,
    /// Fraction 0..1.
    pub direct_weight: f64,
    /// Ticker and weight fraction of the largest position.
    pub top1: (String, f64),
    /// `1 / sum(w^2)`; 0.0 when the total value is zero.
    pub effective_n: f64,
    /// Same order as the input positions, fractions.
    pub weights: Vec<(String, f64)>,
}

/// Summarise `positions` for `symbol` (case-insensitive) or, when `None`, the
/// largest position. An empty portfolio gives `None`; a zero total gives zero
/// weights (never NaN).
pub fn summarize(
    positions: &[PositionValue],
    symbol: Option<&str>,
) -> Option<ConcentrationSummary> {
    // First maximum wins, so ties follow input order.
    let largest = positions
        .iter()
        .reduce(|best, p| if p.value > best.value { p } else { best })?;
    let symbol = symbol
        .map(str::to_uppercase)
        .unwrap_or_else(|| largest.ticker.to_uppercase());
    let total_value: f64 = positions.iter().map(|p| p.value).sum();
    let weight = |v: f64| {
        if total_value > 0.0 {
            v / total_value
        } else {
            0.0
        }
    };
    let direct_value: f64 = positions
        .iter()
        .filter(|p| p.ticker.eq_ignore_ascii_case(&symbol))
        .map(|p| p.value)
        .sum();
    let weights: Vec<(String, f64)> = positions
        .iter()
        .map(|p| (p.ticker.clone(), weight(p.value)))
        .collect();
    let herfindahl: f64 = weights.iter().map(|(_, w)| w * w).sum();
    Some(ConcentrationSummary {
        symbol,
        total_value,
        direct_value,
        direct_weight: weight(direct_value),
        top1: (largest.ticker.clone(), weight(largest.value)),
        effective_n: if herfindahl > 0.0 {
            1.0 / herfindahl
        } else {
            0.0
        },
        weights,
    })
}

/// Direct plus through-ETF exposure to one symbol.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LookThrough {
    pub value: f64,
    pub weight: f64,
    pub via_etfs_value: f64,
}

/// Add each held ETF's `value * pct` (pct = fraction of fund net assets in the
/// symbol, may be negative) to the direct exposure.
pub fn look_through(direct_value: f64, total_value: f64, held_etfs: &[(f64, f64)]) -> LookThrough {
    let via_etfs_value: f64 = held_etfs.iter().map(|(v, p)| v * p).sum();
    let value = direct_value + via_etfs_value;
    LookThrough {
        value,
        weight: if total_value > 0.0 {
            value / total_value
        } else {
            0.0
        },
        via_etfs_value,
    }
}

/// `(exposure + buy * pct) / (total + buy)`; 0.0 when the denominator is 0.
pub fn weight_after_buy(exposure_value: f64, total_value: f64, buy: f64, pct: f64) -> f64 {
    let denom = total_value + buy;
    if denom == 0.0 {
        0.0
    } else {
        (exposure_value + buy * pct) / denom
    }
}
