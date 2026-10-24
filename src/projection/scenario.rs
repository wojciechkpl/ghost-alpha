//! Per-path return trajectory. Bootstrap stub for Gap 1 / Gap 8 — will be
//! extended with policy/state hooks later.

use crate::projection::monte_carlo::AssetClass;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One realised path of yearly returns per asset class.
///
/// `yearly_returns[0]` is a zero-filled placeholder (no compounding from t=0
/// to t=0). Years 1..=horizon contain the realised returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioPath {
    pub id: String,
    pub yearly_returns: Vec<HashMap<AssetClass, f64>>,
}

impl ScenarioPath {
    /// Per-class return realised between year-end (y-1) and year-end y.
    /// Year 0 returns the placeholder zero from `run_monte_carlo`.
    ///
    /// Panics if `year >= yearly_returns.len()` or `class` is absent from the
    /// year map — both indicate a mis-constructed `ScenarioPath`.
    pub fn class_return(&self, year: usize, class: AssetClass) -> f64 {
        self.yearly_returns[year][&class]
    }

    /// Walk a price along the scenario's per-year returns.
    ///
    /// `years_from_t0` may be fractional — the partial-year tail uses
    /// `(1 + r)^frac` compounding. Returns `t0_price` if `years_from_t0 <= 0`.
    pub fn walk_price(&self, t0_price: f64, class: AssetClass, years_from_t0: f64) -> f64 {
        if years_from_t0 <= 0.0 {
            return t0_price;
        }
        let mut p = t0_price;
        let full = years_from_t0.floor() as usize;
        let frac = years_from_t0 - full as f64;
        for y in 1..=full {
            p *= 1.0 + self.yearly_returns[y][&class];
        }
        if frac > 0.0 && full + 1 < self.yearly_returns.len() {
            p *= (1.0 + self.yearly_returns[full + 1][&class]).powf(frac);
        }
        p
    }
}
