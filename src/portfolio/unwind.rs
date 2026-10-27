//! Unwind schedule and seeded Monte Carlo projection for the concentration hub.
//!
//! [`build_schedule`] splits a full sale of one concentrated position into
//! equal-share periods, picks lots highest-cost-first, taxes each sale with
//! [`compute_lot_tax`] and tracks the stock's weight afterwards. [`project`]
//! compares holding with following the schedule on two correlated lognormal
//! assets (the stock and "the rest"). Everything is deterministic: no clock,
//! no I/O, and the random stream is a seeded [`ChaCha12Rng`].

use chrono::{Datelike, Months, NaiveDate};
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;
use rand_distr::{Distribution, StandardNormal};

use crate::portfolio::types::TaxLot;
use crate::projection::monte_carlo::lognormal_params;
use crate::tax::brackets::{FederalCapGainsBrackets, StateTaxRate};
use crate::tax::engine::bracket_tax;
use crate::tax::hifo::hifo_select;
use crate::web::charts::FanChartYear;

// ─── Schedule ───────────────────────────────────────────────────────────────

/// How often a slice of the position is sold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cadence {
    Quarterly,
    Annual,
}

impl Cadence {
    fn step_months(self) -> u32 {
        match self {
            Cadence::Quarterly => 3,
            Cadence::Annual => 12,
        }
    }

    fn per_year(self) -> u32 {
        12 / self.step_months()
    }
}

/// Tax inputs from the user's profile.
#[derive(Debug, Clone)]
pub struct TaxSettings {
    pub state: StateTaxRate,
    pub brackets: FederalCapGainsBrackets,
    pub other_income: f64,
}

#[derive(Debug, Clone)]
pub struct UnwindInput {
    /// Lots of the concentrated symbol only; `shares` = `remaining_shares`.
    pub lots: Vec<TaxLot>,
    pub price: f64,
    /// Value of everything else, held flat in the schedule.
    pub other_value: f64,
    pub years: u32,
    pub cadence: Cadence,
    /// Price growth per year as a fraction.
    pub growth: f64,
    /// Date of the first sale.
    pub start: NaiveDate,
    pub tax: Option<TaxSettings>,
}

#[derive(Debug, Clone)]
pub struct UnwindPeriod {
    pub label: String,
    pub date: NaiveDate,
    pub shares: f64,
    pub proceeds: f64,
    pub cost_basis: f64,
    pub gain: f64,
    pub long_term_gain: f64,
    pub short_term_gain: f64,
    /// Federal + state + NIIT; `None` without tax settings.
    pub tax: Option<f64>,
    pub net: f64,
    /// Stock weight after this sale, net proceeds reinvested in the rest.
    pub weight_after: f64,
}

#[derive(Debug, Clone)]
pub struct UnwindTotals {
    pub shares: f64,
    pub proceeds: f64,
    pub cost_basis: f64,
    pub gain: f64,
    pub tax: Option<f64>,
    pub net: f64,
}

#[derive(Debug, Clone)]
pub struct UnwindSchedule {
    pub periods: Vec<UnwindPeriod>,
    pub totals: UnwindTotals,
}

/// Tax for one calendar year (see [`year_tax`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct YearTax {
    pub federal: f64,
    pub state: f64,
    pub niit: f64,
    pub total: f64,
}

/// Tax for ONE calendar year from its realised short- and long-term gain totals
/// (either may be negative; no carryforwards and no $3,000 offset).
///
/// Short and long term net against each other. Federal tax is the ordinary-bracket
/// tax on net short-term gain stacked on other income, plus the long-term bracket
/// tax on net long-term gain stacked on other income plus net short-term gain. NIIT
/// is `rate * min(net, max(0, other_income + net - threshold))`. States with
/// `long_term_only` (Washington) tax only the long-term total above the deduction;
/// other states tax the net gain above the deduction. A net loss owes nothing.
pub fn year_tax(st_gain: f64, lt_gain: f64, tax: &TaxSettings) -> YearTax {
    let net = st_gain + lt_gain;
    if net.is_nan() || net <= 0.0 {
        return YearTax {
            federal: 0.0,
            state: 0.0,
            niit: 0.0,
            total: 0.0,
        };
    }
    // IRS-style netting: a loss on one side offsets the other.
    let (st, lt) = if st_gain < 0.0 {
        (0.0, lt_gain + st_gain)
    } else if lt_gain < 0.0 {
        (st_gain + lt_gain, 0.0)
    } else {
        (st_gain, lt_gain)
    };
    let b = &tax.brackets;
    let income = tax.other_income;
    let federal = bracket_tax(st, &b.short_term_brackets, income)
        + bracket_tax(lt, &b.long_term_brackets, income + st);
    let niit = b.niit_rate * net.min((income + net - b.niit_threshold).max(0.0));
    let s = &tax.state;
    let taxable = if s.long_term_only {
        (lt_gain - s.annual_deduction).max(0.0)
    } else {
        (net - s.annual_deduction).max(0.0)
    };
    let state = if taxable <= 0.0 || (s.rate == 0.0 && s.brackets.is_empty()) {
        0.0
    } else if s.brackets.is_empty() {
        taxable * s.rate
    } else {
        bracket_tax(taxable, &s.brackets, 0.0)
    };
    YearTax {
        federal,
        state,
        niit,
        total: federal + state + niit,
    }
}

/// Build the unwind schedule. Empty when there is nothing to sell or no years.
pub fn build_schedule(input: &UnwindInput) -> UnwindSchedule {
    let mut lots = input.lots.clone();
    let total_shares: f64 = lots.iter().map(|l| l.remaining_shares.max(0.0)).sum();
    let n = (input.years * input.cadence.per_year()) as usize;
    let step = input.cadence.step_months();
    let prefix = match input.cadence {
        Cadence::Quarterly => "Q",
        Cadence::Annual => "Y",
    };

    let mut periods = Vec::with_capacity(n);
    if n > 0 && total_shares > 0.0 {
        let per = total_shares / n as f64;
        let mut sold = 0.0;
        let mut cum_net = 0.0;
        // (calendar year, short-term gains so far, long-term gains so far)
        let mut ytd: (i32, f64, f64) = (i32::MIN, 0.0, 0.0);
        for k in 0..n {
            let months = step * k as u32;
            let date = input
                .start
                .checked_add_months(Months::new(months))
                .unwrap_or(input.start);
            let price = input.price * (1.0 + input.growth).powf(months as f64 / 12.0);
            let shares = if k + 1 == n {
                total_shares - per * (n - 1) as f64
            } else {
                per
            };
            if date.year() != ytd.0 {
                ytd = (date.year(), 0.0, 0.0);
            }

            let (mut cost_basis, mut lt, mut st) = (0.0, 0.0, 0.0);
            for (idx, take) in hifo_select(&lots, shares) {
                let lot = &lots[idx];
                let g = take * (price - lot.cost_basis_per_share);
                cost_basis += take * lot.cost_basis_per_share;
                if lot.is_long_term(date) {
                    lt += g;
                } else {
                    st += g;
                }
                lots[idx].remaining_shares -= take;
            }
            // The period's tax is the change in the calendar year's tax.
            let tax = input.tax.as_ref().map(|t| {
                let before = year_tax(ytd.1, ytd.2, t).total;
                year_tax(ytd.1 + st, ytd.2 + lt, t).total - before
            });
            ytd.1 += st;
            ytd.2 += lt;
            let proceeds = shares * price;
            let gain = proceeds - cost_basis;
            let net = proceeds - tax.unwrap_or(0.0);
            cum_net += net;
            sold += shares;
            let remaining = if k + 1 == n {
                0.0
            } else {
                (total_shares - sold).max(0.0)
            };
            let stock_value = remaining * price;
            let denom = stock_value + input.other_value + cum_net;
            periods.push(UnwindPeriod {
                label: format!("{prefix}{}", k + 1),
                date,
                shares,
                proceeds,
                cost_basis,
                gain,
                long_term_gain: lt,
                short_term_gain: st,
                tax,
                net,
                weight_after: if denom > 0.0 {
                    stock_value / denom
                } else {
                    0.0
                },
            });
        }
    }

    let sum = |f: fn(&UnwindPeriod) -> f64| periods.iter().map(f).sum::<f64>();
    let totals = UnwindTotals {
        shares: sum(|p| p.shares),
        proceeds: sum(|p| p.proceeds),
        cost_basis: sum(|p| p.cost_basis),
        gain: sum(|p| p.gain),
        tax: input
            .tax
            .as_ref()
            .map(|_| periods.iter().filter_map(|p| p.tax).sum()),
        net: sum(|p| p.net),
    };
    UnwindSchedule { periods, totals }
}

// ─── Projection ─────────────────────────────────────────────────────────────

pub const DEFAULT_STOCK_MU: f64 = 0.08;
pub const DEFAULT_STOCK_SIGMA: f64 = 0.30;
pub const DEFAULT_REST_MU: f64 = 0.06;
pub const DEFAULT_REST_SIGMA: f64 = 0.15;
pub const DEFAULT_RHO: f64 = 0.6;
pub const DEFAULT_PATHS: usize = 2000;
/// Fixed seed so the same inputs always draw the same chart.
pub const PROJECTION_SEED: u64 = 20_261_025;

#[derive(Debug, Clone)]
pub struct ProjectionInput {
    pub stock_value: f64,
    pub other_value: f64,
    pub years: u32,
    pub stock_mu: f64,
    pub stock_sigma: f64,
    pub rest_mu: f64,
    pub rest_sigma: f64,
    pub rho: f64,
    pub paths: usize,
    /// One entry per year: fraction of the ORIGINAL shares sold at the start
    /// of that year, before its returns.
    pub sell_fractions: Vec<f64>,
    /// Effective tax rate on sale proceeds; `1 - rate` moves into the rest.
    pub tax_rate: f64,
}

#[derive(Debug, Clone)]
pub struct Projection {
    pub years: u32,
    pub hold: Vec<FanChartYear>,
    pub unwind: Vec<FanChartYear>,
    /// Median stock weight per year (len `years + 1`).
    pub weight_p50_hold: Vec<f64>,
    pub weight_p50_unwind: Vec<f64>,
}

/// Linear-interpolated quantile of an ascending slice.
fn quantile(sorted: &[f64], q: f64) -> f64 {
    match sorted.len() {
        0 => 0.0,
        1 => sorted[0],
        n => {
            let pos = q * (n - 1) as f64;
            let lo = pos.floor() as usize;
            let hi = (lo + 1).min(n - 1);
            let f = pos - lo as f64;
            sorted[lo] + (sorted[hi] - sorted[lo]) * f
        }
    }
}

fn fan_year(year: u32, values: &mut [f64]) -> FanChartYear {
    values.sort_by(f64::total_cmp);
    FanChartYear {
        year,
        p10: quantile(values, 0.10),
        p25: quantile(values, 0.25),
        p50: quantile(values, 0.50),
        p75: quantile(values, 0.75),
        p90: quantile(values, 0.90),
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    quantile(values, 0.5)
}

/// Seeded Monte Carlo of holding versus unwinding (see the module docs). Both
/// scenarios see the same random shocks.
pub fn project(input: &ProjectionInput, seed: u64) -> Projection {
    let years = input.years as usize;
    let paths = input.paths.max(1);
    let (s_mu, s_sig) = lognormal_params(input.stock_mu, input.stock_sigma);
    let (r_mu, r_sig) = lognormal_params(input.rest_mu, input.rest_sigma);
    let rho = input.rho.clamp(-1.0, 1.0);
    let ortho = (1.0 - rho * rho).sqrt();
    let keep = 1.0 - input.tax_rate.clamp(0.0, 1.0);
    let mut rng = ChaCha12Rng::seed_from_u64(seed);

    let mut hold_v = vec![Vec::with_capacity(paths); years + 1];
    let mut unw_v = vec![Vec::with_capacity(paths); years + 1];
    let mut hold_w = vec![Vec::with_capacity(paths); years + 1];
    let mut unw_w = vec![Vec::with_capacity(paths); years + 1];
    let weight = |stock: f64, rest: f64| {
        let t = stock + rest;
        if t > 0.0 {
            stock / t
        } else {
            0.0
        }
    };

    for _ in 0..paths {
        // `sv`: value of ALL original shares; the unwind holds `units` of them.
        let mut sv = input.stock_value;
        let mut rest_h = input.other_value;
        let mut units = 1.0_f64;
        let mut rest_u = input.other_value;
        hold_v[0].push(sv + rest_h);
        unw_v[0].push(sv + rest_u);
        hold_w[0].push(weight(sv, rest_h));
        unw_w[0].push(weight(units * sv, rest_u));
        for y in 0..years {
            let f = input.sell_fractions.get(y).copied().unwrap_or(0.0);
            let f = f.clamp(0.0, units);
            rest_u += keep * f * sv;
            units -= f;
            if units < 1e-12 {
                units = 0.0;
            }
            let z1: f64 = StandardNormal.sample(&mut rng);
            let z2: f64 = StandardNormal.sample(&mut rng);
            let gs = (s_mu + s_sig * z1).exp();
            let gr = (r_mu + r_sig * (rho * z1 + ortho * z2)).exp();
            sv *= gs;
            rest_h *= gr;
            rest_u *= gr;
            hold_v[y + 1].push(sv + rest_h);
            unw_v[y + 1].push(units * sv + rest_u);
            hold_w[y + 1].push(weight(sv, rest_h));
            unw_w[y + 1].push(weight(units * sv, rest_u));
        }
    }

    Projection {
        years: input.years,
        hold: hold_v
            .iter_mut()
            .enumerate()
            .map(|(y, v)| fan_year(y as u32, v))
            .collect(),
        unwind: unw_v
            .iter_mut()
            .enumerate()
            .map(|(y, v)| fan_year(y as u32, v))
            .collect(),
        weight_p50_hold: hold_w.iter_mut().map(|v| median(v)).collect(),
        weight_p50_unwind: unw_w.iter_mut().map(|v| median(v)).collect(),
    }
}
