//! Monte Carlo engine: correlated lognormal return sampling across asset
//! classes. See `docs/PLAN_PIPELINE.md` for how it feeds the plan pipeline.

use crate::projection::asset_class_library::AssetClassLibrary;
use crate::projection::scenario::ScenarioPath;
use nalgebra::{DMatrix, DVector, SymmetricEigen};
use rand::{rngs::StdRng, SeedableRng};
use rand_distr::{Distribution, StandardNormal};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for a Monte Carlo run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McConfig {
    pub n_paths: usize,
    pub seed: u64,
    pub horizon_years: u32,
    pub return_distribution: ReturnDistribution,
}

impl Default for McConfig {
    fn default() -> Self {
        Self {
            n_paths: 5_000,
            seed: 42,
            horizon_years: 5,
            return_distribution: ReturnDistribution::Lognormal,
        }
    }
}

/// Return distribution variant used when sampling path returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ReturnDistribution {
    /// Sample log-returns ~ N(μ_log, σ_log²), exponentiate → r > -1.
    Lognormal,
    /// Sample r ~ N(μ, σ²) and clip to ≥ floor (diagnostic use only).
    NormalClipped { floor: f64 },
}

/// Hand-curated block-correlation parameters for the asset-class universe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelationRules {
    pub us_core_within: f64,
    pub us_mega_within: f64,
    pub us_mega_to_core: f64,
    pub intl_within: f64,
    pub intl_to_us_core: f64,
    pub sectors_to_broad: f64,
    pub equity_default: f64,
    pub treasury_within: f64,
    pub equity_to_treasury: f64,
    pub gold_to_anything: f64,
    pub mf_to_anything: f64,
    pub cash_to_anything: f64,
}

impl Default for CorrelationRules {
    fn default() -> Self {
        Self {
            us_core_within: 0.92,
            us_mega_within: 0.75,
            us_mega_to_core: 0.80,
            intl_within: 0.75,
            intl_to_us_core: 0.65,
            sectors_to_broad: 0.55,
            equity_default: 0.65,
            treasury_within: 0.85,
            equity_to_treasury: -0.15,
            gold_to_anything: 0.05,
            mf_to_anything: 0.10,
            cash_to_anything: 0.0,
        }
    }
}

/// Convert arithmetic mean and std to lognormal (μ_log, σ_log) parameters.
pub fn lognormal_params(mu_arith: f64, sigma_arith: f64) -> (f64, f64) {
    let sigma_log_sq = (1.0 + (sigma_arith / (1.0 + mu_arith)).powi(2)).ln();
    let sigma_log = sigma_log_sq.sqrt();
    let mu_log = (1.0 + mu_arith).ln() - 0.5 * sigma_log_sq;
    (mu_log, sigma_log)
}

/// All 19 asset classes in the GhostAlpha universe, in alphabetical order.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AssetClass {
    AerospaceDefense,
    CashMmf,
    ConsumerMegaCap,
    EmergingAsia,
    EmergingMarketsEtf,
    EnergySector,
    Gold,
    Industrial,
    InternationalDeveloped,
    IntlDevelopedEtf,
    ManagedFutures,
    TechMegaCap,
    UsBlueChipGrowth,
    UsLargeCapIndex,
    UsQualityFactor,
    UsTips,
    UsTotalMarket,
    UsTreasuryIntermediate,
    UsTreasuryLong,
}

impl AssetClass {
    /// Iterate all 19 variants in alphabetical order (stable).
    pub fn iter_all() -> impl Iterator<Item = AssetClass> {
        const ALL: [AssetClass; 19] = [
            AssetClass::AerospaceDefense,
            AssetClass::CashMmf,
            AssetClass::ConsumerMegaCap,
            AssetClass::EmergingAsia,
            AssetClass::EmergingMarketsEtf,
            AssetClass::EnergySector,
            AssetClass::Gold,
            AssetClass::Industrial,
            AssetClass::InternationalDeveloped,
            AssetClass::IntlDevelopedEtf,
            AssetClass::ManagedFutures,
            AssetClass::TechMegaCap,
            AssetClass::UsBlueChipGrowth,
            AssetClass::UsLargeCapIndex,
            AssetClass::UsQualityFactor,
            AssetClass::UsTips,
            AssetClass::UsTotalMarket,
            AssetClass::UsTreasuryIntermediate,
            AssetClass::UsTreasuryLong,
        ];
        ALL.iter().copied()
    }

    /// True for US core broad-market equities.
    pub fn is_us_core(self) -> bool {
        matches!(
            self,
            Self::UsLargeCapIndex
                | Self::UsTotalMarket
                | Self::UsQualityFactor
                | Self::UsBlueChipGrowth
        )
    }

    /// True for US mega-cap equity ETFs / baskets.
    pub fn is_us_mega(self) -> bool {
        matches!(self, Self::TechMegaCap | Self::ConsumerMegaCap)
    }

    /// True for international / emerging-market equity exposures.
    pub fn is_intl(self) -> bool {
        matches!(
            self,
            Self::InternationalDeveloped
                | Self::IntlDevelopedEtf
                | Self::EmergingAsia
                | Self::EmergingMarketsEtf
        )
    }

    /// True for single-sector equity funds.
    pub fn is_sector(self) -> bool {
        matches!(
            self,
            Self::EnergySector | Self::AerospaceDefense | Self::Industrial
        )
    }

    /// True for any equity sub-class.
    pub fn is_equity(self) -> bool {
        self.is_us_core() || self.is_us_mega() || self.is_intl() || self.is_sector()
    }

    /// True for US Treasury / inflation-protected bond funds.
    pub fn is_treasury(self) -> bool {
        matches!(
            self,
            Self::UsTreasuryIntermediate | Self::UsTreasuryLong | Self::UsTips
        )
    }

    /// True for cash or money-market funds.
    pub fn is_cash(self) -> bool {
        matches!(self, Self::CashMmf)
    }

    /// True for gold or precious-metals funds.
    pub fn is_gold(self) -> bool {
        matches!(self, Self::Gold)
    }

    /// True for managed-futures / trend-following strategies.
    pub fn is_managed_futures(self) -> bool {
        matches!(self, Self::ManagedFutures)
    }
}

/// Block-correlation matrix for a given set of asset classes.
#[derive(Debug, Clone)]
pub struct CorrelationMatrix {
    /// Asset classes in the same order as the matrix rows/columns.
    pub classes: Vec<AssetClass>,
    /// Symmetric positive-semidefinite correlation matrix (unit diagonal).
    pub matrix: DMatrix<f64>,
}

/// Derive a pairwise correlation value from the block-correlation rules.
fn pair_corr(c1: AssetClass, c2: AssetClass, r: &CorrelationRules) -> f64 {
    if c1 == c2 {
        return 1.0;
    }
    if c1.is_cash() || c2.is_cash() {
        return r.cash_to_anything;
    }
    if c1.is_gold() || c2.is_gold() {
        return r.gold_to_anything;
    }
    if c1.is_managed_futures() || c2.is_managed_futures() {
        return r.mf_to_anything;
    }
    if c1.is_treasury() && c2.is_treasury() {
        return r.treasury_within;
    }
    let (e1, e2) = (c1.is_equity(), c2.is_equity());
    if e1 && e2 {
        if c1.is_us_core() && c2.is_us_core() {
            return r.us_core_within;
        }
        if c1.is_us_mega() && c2.is_us_mega() {
            return r.us_mega_within;
        }
        if (c1.is_us_mega() && c2.is_us_core()) || (c2.is_us_mega() && c1.is_us_core()) {
            return r.us_mega_to_core;
        }
        if c1.is_intl() && c2.is_intl() {
            return r.intl_within;
        }
        if (c1.is_intl() && c2.is_us_core()) || (c2.is_intl() && c1.is_us_core()) {
            return r.intl_to_us_core;
        }
        if c1.is_sector() || c2.is_sector() {
            return r.sectors_to_broad;
        }
        return r.equity_default;
    }
    if (e1 && c2.is_treasury()) || (e2 && c1.is_treasury()) {
        return r.equity_to_treasury;
    }
    0.30
}

/// Build a PSD block-correlation matrix for the given asset classes using the supplied rules.
///
/// Eigenvalues are clipped at 1e-6 and the diagonal is renormalized to ensure strict PSD
/// with unit diagonal.
pub fn build_block_corr(classes: &[AssetClass], rules: &CorrelationRules) -> CorrelationMatrix {
    let n = classes.len();
    let mut raw = DMatrix::<f64>::identity(n, n);
    for i in 0..n {
        for j in (i + 1)..n {
            let c = pair_corr(classes[i], classes[j], rules);
            raw[(i, j)] = c;
            raw[(j, i)] = c;
        }
    }

    // Clip eigenvalues at 1e-6 to enforce strict PSD; then renormalize to restore unit diagonal.
    let eigen = SymmetricEigen::new(raw);
    let clipped_evals = eigen.eigenvalues.map(|x| x.max(1e-6));
    // Reconstruct: M = V · diag(λ_clipped) · Vᵀ
    let v = &eigen.eigenvectors;
    let psd = v * DMatrix::from_diagonal(&clipped_evals) * v.transpose();
    // Renormalize to unit diagonal.
    let d = DVector::from_iterator(n, (0..n).map(|i| psd[(i, i)].sqrt()));
    let matrix = DMatrix::from_fn(n, n, |i, j| psd[(i, j)] / (d[i] * d[j]));

    CorrelationMatrix {
        classes: classes.to_vec(),
        matrix,
    }
}

/// Result of a Monte Carlo run.
#[derive(Debug)]
pub struct McResult {
    pub seed: u64,
    pub n_paths: usize,
    pub paths: Vec<ScenarioPath>,
}

/// Sample `cfg.n_paths` correlated lognormal return paths for the given asset
/// classes and write them to `McResult::paths`.
///
/// Pre-computes the Cholesky factor of the correlation matrix once, plus the
/// log-space μ and σ vectors. Each path draws `cfg.horizon_years × n_classes`
/// standard normals, applies L for correlation, exponentiates → arithmetic
/// returns guaranteed > -1.
pub fn run_monte_carlo(
    library: &AssetClassLibrary,
    correlations: &CorrelationMatrix,
    cfg: &McConfig,
) -> McResult {
    let classes = &correlations.classes;
    let n_classes = classes.len();
    let n_years = cfg.horizon_years as usize;
    let n_paths = cfg.n_paths;

    // Pre-compute log-space parameters for each asset class.
    let mu_log: DVector<f64> = DVector::from_iterator(
        n_classes,
        classes.iter().map(|&c| {
            let (mu, _) = lognormal_params(library.mean(c), library.vol(c));
            mu
        }),
    );
    let sigma_log: DVector<f64> = DVector::from_iterator(
        n_classes,
        classes.iter().map(|&c| {
            let (_, sigma) = lognormal_params(library.mean(c), library.vol(c));
            sigma
        }),
    );

    // Cholesky factor — computed once outside the sampling loop.
    let chol = correlations
        .matrix
        .clone()
        .cholesky()
        .expect("correlation matrix must be PSD");
    let l = chol.l();

    // Year-0 placeholder: all zeros, cloned per path.
    let year_zero: HashMap<AssetClass, f64> = classes.iter().map(|&c| (c, 0.0_f64)).collect();

    let mut rng = StdRng::seed_from_u64(cfg.seed);
    let mut paths = Vec::with_capacity(n_paths);

    for p in 0..n_paths {
        let mut yearly_returns = Vec::with_capacity(n_years + 1);
        yearly_returns.push(year_zero.clone());

        for _y in 0..n_years {
            // Draw n_classes independent standard normals.
            let z: DVector<f64> = DVector::from_iterator(
                n_classes,
                (0..n_classes).map(|_| StandardNormal.sample(&mut rng)),
            );
            // Apply Cholesky to introduce correlations.
            let eps = &l * z;

            let mut returns = HashMap::with_capacity(n_classes);
            for (i, &c) in classes.iter().enumerate() {
                let log_r = mu_log[i] + sigma_log[i] * eps[i];
                let r = log_r.exp() - 1.0;
                returns.insert(c, r);
            }
            yearly_returns.push(returns);
        }

        paths.push(ScenarioPath {
            id: format!("mc_{:05}", p),
            yearly_returns,
        });
    }

    McResult {
        seed: cfg.seed,
        n_paths,
        paths,
    }
}

/// Percentile bucket result for one year of a Monte Carlo run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YearPercentiles {
    pub year: u32,
    pub n_paths: usize,
    pub mean: f64,
    pub percentiles: Vec<(u8, f64)>,
}

/// Linear-interpolation percentile (numpy default behaviour).
///
/// Precondition: `sorted` is sorted ascending. q is clamped to [0, 100].
/// Caller must satisfy the sorted precondition; no runtime check.
pub fn percentile(sorted: &[f64], q: u8) -> f64 {
    let q = q.min(100) as f64;
    let n = sorted.len();
    let k = (q / 100.0) * (n - 1) as f64;
    let lo = k.floor() as usize;
    let hi = (k.ceil() as usize).min(n - 1);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (k - lo as f64)
}

/// Aggregate per-year statistics (mean + selected percentiles) across MC paths.
///
/// `states_by_path_year[p][y]` is path p's state at year y. The extractor
/// projects each state to a scalar (e.g. `|s| s.nav`). Years are taken from
/// the inner Vec length (assumed uniform across paths).
pub fn percentiles_per_year<T, F>(
    states_by_path_year: &[Vec<T>],
    extractor: F,
    pcts: &[u8],
) -> Vec<YearPercentiles>
where
    F: Fn(&T) -> f64,
{
    if states_by_path_year.is_empty() {
        return Vec::new();
    }
    let n_years = states_by_path_year[0].len();
    let n_paths = states_by_path_year.len();

    (0..n_years)
        .map(|y| {
            let mut values: Vec<f64> = states_by_path_year
                .iter()
                .map(|path| extractor(&path[y]))
                .collect();
            values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let mean = values.iter().sum::<f64>() / n_paths as f64;
            let percentiles = pcts.iter().map(|&q| (q, percentile(&values, q))).collect();
            YearPercentiles {
                year: y as u32,
                n_paths,
                mean,
                percentiles,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::SymmetricEigen;
    use rand::{rngs::StdRng, SeedableRng};
    use rand_distr::{Distribution, StandardNormal};
    use std::collections::HashMap;

    // ---- Step 3 helper -------------------------------------------------------

    /// Build a 2-class matrix and extract the off-diagonal correlation value.
    fn pair_value(a: AssetClass, b: AssetClass) -> f64 {
        let classes = vec![a, b];
        let m = build_block_corr(&classes, &CorrelationRules::default()).matrix;
        m[(0, 1)]
    }

    /// Checks the lognormal identity: E[exp(μ_log + σ_log·Z) - 1] = exp(μ_log + σ_log²/2) - 1
    #[test]
    fn lognormal_params_recovers_arith_mean() {
        let (mu_log, sigma_log) = lognormal_params(0.10, 0.32);
        // Analytical mean of a lognormal r.v. minus 1
        let analytical_mean = (mu_log + 0.5 * sigma_log * sigma_log).exp() - 1.0;
        assert!(
            (analytical_mean - 0.10).abs() < 1e-12,
            "got {} expected 0.10",
            analytical_mean
        );
    }

    /// Checks the lognormal variance identity: Var = exp(2μ_log + σ_log²)·(exp(σ_log²) - 1)
    #[test]
    fn lognormal_params_recovers_arith_std() {
        let cases = [(0.10_f64, 0.32_f64), (0.085, 0.16)];
        for (mu_arith, sigma_arith) in cases {
            let (mu_log, sigma_log) = lognormal_params(mu_arith, sigma_arith);
            let s2 = sigma_log * sigma_log;
            // Variance of lognormal r.v. minus 1 (shift doesn't change variance)
            let analytical_var = (2.0 * mu_log + s2).exp() * (s2.exp() - 1.0);
            let analytical_std = analytical_var.sqrt();
            assert!(
                (analytical_std - sigma_arith).abs() < 1e-12,
                "got {} expected {} (mu_arith={}, sigma_arith={})",
                analytical_std,
                sigma_arith,
                mu_arith,
                sigma_arith
            );
        }
    }

    /// Confirms sampled moments converge to targets at n=100_000 (seeded for reproducibility)
    #[test]
    fn lognormal_sampling_at_scale_matches_target_moments() {
        let (mu_log, sigma_log) = lognormal_params(0.10, 0.32);
        let mut rng = StdRng::seed_from_u64(42);
        let n = 100_000_usize;

        // Welford's online mean and M2 for numerically stable variance
        let mut mean = 0.0_f64;
        let mut m2 = 0.0_f64;
        for i in 1..=n {
            let z: f64 = StandardNormal.sample(&mut rng);
            let r = (mu_log + sigma_log * z).exp() - 1.0;
            let delta = r - mean;
            mean += delta / i as f64;
            m2 += delta * (r - mean);
        }
        let sample_std = (m2 / (n - 1) as f64).sqrt();

        assert!(
            (mean - 0.10).abs() < 0.005,
            "sample mean {} not within 0.005 of 0.10",
            mean
        );
        assert!(
            (sample_std - 0.32).abs() < 0.005,
            "sample std {} not within 0.005 of 0.32",
            sample_std
        );
    }

    // ---- Step 3: classifier method tests ------------------------------------

    #[test]
    fn iter_all_yields_19_distinct_variants() {
        let mut all: Vec<AssetClass> = AssetClass::iter_all().collect();
        assert_eq!(all.len(), 19, "expected 19 variants, got {}", all.len());
        all.sort_by_key(|a| *a as u8);
        all.dedup_by_key(|a| *a as u8);
        assert_eq!(all.len(), 19, "duplicates detected after dedup");
    }

    #[test]
    fn asset_class_classifiers_partition_correctly() {
        for c in AssetClass::iter_all() {
            let flags = [
                c.is_equity(),
                c.is_treasury(),
                c.is_cash(),
                c.is_gold(),
                c.is_managed_futures(),
            ];
            let true_count = flags.iter().filter(|&&f| f).count();
            assert_eq!(
                true_count, 1,
                "{:?} matched {} top-level classifiers (expected exactly 1)",
                c, true_count
            );
        }
    }

    #[test]
    fn asset_class_subgroups_are_disjoint_within_equity() {
        let equity: Vec<AssetClass> = AssetClass::iter_all().filter(|c| c.is_equity()).collect();
        for &c in &equity {
            let sub_count = [c.is_us_core(), c.is_us_mega(), c.is_intl(), c.is_sector()]
                .iter()
                .filter(|&&f| f)
                .count();
            assert_eq!(
                sub_count, 1,
                "equity variant {:?} matched {} subgroup classifiers (expected exactly 1)",
                c, sub_count
            );
        }
        // Union of subgroups must equal the equity set (same cardinality is sufficient given
        // the disjoint check above and that each equity item lands in exactly one subgroup).
        let subgroup_count: usize = equity
            .iter()
            .map(|c| {
                [c.is_us_core(), c.is_us_mega(), c.is_intl(), c.is_sector()]
                    .iter()
                    .filter(|&&f| f)
                    .count()
            })
            .sum();
        assert_eq!(subgroup_count, equity.len());
    }

    // ---- Step 3: structural matrix tests ------------------------------------

    #[test]
    fn correlation_matrix_is_symmetric() {
        let classes: Vec<AssetClass> = AssetClass::iter_all().collect();
        let m = build_block_corr(&classes, &CorrelationRules::default()).matrix;
        for i in 0..m.nrows() {
            for j in 0..m.ncols() {
                assert!(
                    (m[(i, j)] - m[(j, i)]).abs() < 1e-12,
                    "asymmetry at [{i},{j}]: {} vs {}",
                    m[(i, j)],
                    m[(j, i)]
                );
            }
        }
    }

    #[test]
    fn correlation_matrix_diagonal_is_one() {
        let classes: Vec<AssetClass> = AssetClass::iter_all().collect();
        let m = build_block_corr(&classes, &CorrelationRules::default()).matrix;
        for i in 0..m.nrows() {
            assert!(
                (m[(i, i)] - 1.0).abs() < 1e-9,
                "diagonal [{i},{i}] = {} (expected 1.0)",
                m[(i, i)]
            );
        }
    }

    #[test]
    fn correlation_matrix_is_psd() {
        let classes: Vec<AssetClass> = AssetClass::iter_all().collect();
        let m = build_block_corr(&classes, &CorrelationRules::default()).matrix;
        let eigen = SymmetricEigen::new(m);
        let min_ev = eigen.eigenvalues.min();
        assert!(min_ev > 0.0, "min eigenvalue {} <= 0", min_ev);
    }

    #[test]
    fn correlation_matrix_classes_field_matches_input() {
        let input: Vec<AssetClass> = AssetClass::iter_all().collect();
        let result = build_block_corr(&input, &CorrelationRules::default());
        assert_eq!(result.classes, input);
    }

    #[test]
    fn correlation_matrix_dimensions_match_input_count() {
        let classes = vec![
            AssetClass::UsLargeCapIndex,
            AssetClass::UsTotalMarket,
            AssetClass::TechMegaCap,
            AssetClass::Gold,
            AssetClass::CashMmf,
        ];
        let m = build_block_corr(&classes, &CorrelationRules::default()).matrix;
        assert_eq!(m.shape(), (5, 5));
    }

    // ---- Step 3: pair-correlation cell tests --------------------------------

    #[test]
    fn us_core_pair_uses_us_core_within() {
        let v = pair_value(AssetClass::UsLargeCapIndex, AssetClass::UsTotalMarket);
        assert!((v - 0.92).abs() < 1e-7, "got {v}");
    }

    #[test]
    fn us_mega_pair_uses_us_mega_within() {
        let v = pair_value(AssetClass::TechMegaCap, AssetClass::ConsumerMegaCap);
        assert!((v - 0.75).abs() < 1e-7, "got {v}");
    }

    #[test]
    fn us_mega_to_core_uses_us_mega_to_core() {
        let v = pair_value(AssetClass::TechMegaCap, AssetClass::UsLargeCapIndex);
        assert!((v - 0.80).abs() < 1e-7, "got {v}");
    }

    #[test]
    fn intl_pair_uses_intl_within() {
        let v = pair_value(AssetClass::InternationalDeveloped, AssetClass::EmergingAsia);
        assert!((v - 0.75).abs() < 1e-7, "got {v}");
    }

    #[test]
    fn intl_to_us_core_uses_intl_to_us_core() {
        let v = pair_value(AssetClass::IntlDevelopedEtf, AssetClass::UsLargeCapIndex);
        assert!((v - 0.65).abs() < 1e-7, "got {v}");
    }

    #[test]
    fn sector_to_broad_uses_sectors_to_broad() {
        let v = pair_value(AssetClass::EnergySector, AssetClass::UsLargeCapIndex);
        assert!((v - 0.55).abs() < 1e-7, "got {v}");
    }

    #[test]
    fn treasury_pair_uses_treasury_within() {
        let v = pair_value(
            AssetClass::UsTreasuryIntermediate,
            AssetClass::UsTreasuryLong,
        );
        assert!((v - 0.85).abs() < 1e-7, "got {v}");
    }

    #[test]
    fn equity_to_treasury_uses_equity_to_treasury() {
        let v = pair_value(
            AssetClass::UsLargeCapIndex,
            AssetClass::UsTreasuryIntermediate,
        );
        assert!((v - (-0.15)).abs() < 1e-7, "got {v}");
    }

    #[test]
    fn gold_to_anything_pair() {
        let v = pair_value(AssetClass::Gold, AssetClass::UsLargeCapIndex);
        assert!((v - 0.05).abs() < 1e-7, "got {v}");
    }

    #[test]
    fn managed_futures_to_anything_pair() {
        let v = pair_value(AssetClass::ManagedFutures, AssetClass::UsLargeCapIndex);
        assert!((v - 0.10).abs() < 1e-7, "got {v}");
    }

    #[test]
    fn cash_to_anything_is_zero() {
        let v = pair_value(AssetClass::CashMmf, AssetClass::UsLargeCapIndex);
        assert!((v - 0.0).abs() < 1e-9, "got {v}");
    }

    // ---- Step 5: helper -------------------------------------------------------

    fn run_with(seed: u64, n_paths: usize) -> McResult {
        let library = AssetClassLibrary::builtin_defaults();
        let classes: Vec<AssetClass> = AssetClass::iter_all().collect();
        let correlations = build_block_corr(&classes, &CorrelationRules::default());
        let cfg = McConfig {
            seed,
            n_paths,
            ..McConfig::default()
        };
        run_monte_carlo(&library, &correlations, &cfg)
    }

    // ---- Step 5: library tests ------------------------------------------------

    #[test]
    fn library_default_mean_for_tech_mega_cap_is_target() {
        let library = AssetClassLibrary::builtin_defaults();
        assert_eq!(library.mean(AssetClass::TechMegaCap), 0.10);
    }

    #[test]
    fn library_default_vol_for_us_large_cap_is_target() {
        let library = AssetClassLibrary::builtin_defaults();
        assert_eq!(library.vol(AssetClass::UsLargeCapIndex), 0.16);
    }

    #[test]
    fn library_covers_all_19_asset_classes() {
        let library = AssetClassLibrary::builtin_defaults();
        AssetClass::iter_all().for_each(|c| {
            let _ = library.mean(c);
            let _ = library.vol(c);
        });
    }

    // ---- Step 5: run shape tests ---------------------------------------------

    #[test]
    fn run_monte_carlo_produces_n_paths() {
        let result = run_with(42, 100);
        assert_eq!(result.paths.len(), 100);
    }

    #[test]
    fn run_monte_carlo_each_path_has_horizon_plus_one_years() {
        let result = run_with(42, 100);
        let expected = McConfig::default().horizon_years as usize + 1;
        for path in &result.paths {
            assert_eq!(
                path.yearly_returns.len(),
                expected,
                "path {} has {} years (expected {})",
                path.id,
                path.yearly_returns.len(),
                expected
            );
        }
    }

    #[test]
    fn run_monte_carlo_path_ids_are_distinct_and_indexed() {
        let result = run_with(42, 100);
        let expected = ["mc_00000", "mc_00001", "mc_00002", "mc_00003", "mc_00004"];
        for (i, id) in expected.iter().enumerate() {
            assert_eq!(&result.paths[i].id, id, "path {i} id mismatch");
        }
    }

    // ---- Step 5: year-0 placeholder test -------------------------------------

    #[test]
    fn run_monte_carlo_year_zero_returns_are_all_zero() {
        let result = run_with(42, 100);
        let year_zero: &HashMap<AssetClass, f64> = &result.paths[0].yearly_returns[0];
        for c in AssetClass::iter_all() {
            let r = year_zero[&c];
            assert_eq!(r, 0.0, "{:?} year-0 return is {} (expected 0.0)", c, r);
        }
    }

    // ---- Step 5: reproducibility tests ---------------------------------------

    #[test]
    fn run_monte_carlo_same_seed_produces_identical_paths() {
        let a = run_with(42, 10);
        let b = run_with(42, 10);
        for (pa, pb) in a.paths.iter().zip(b.paths.iter()) {
            for (ya, yb) in pa.yearly_returns.iter().zip(pb.yearly_returns.iter()) {
                for c in AssetClass::iter_all() {
                    assert_eq!(
                        ya[&c], yb[&c],
                        "divergence at {:?}: {} vs {}",
                        c, ya[&c], yb[&c]
                    );
                }
            }
        }
    }

    #[test]
    fn run_monte_carlo_different_seeds_produce_different_paths() {
        let a = run_with(42, 1);
        let b = run_with(43, 1);
        let ra = a.paths[0].yearly_returns[1][&AssetClass::TechMegaCap];
        let rb = b.paths[0].yearly_returns[1][&AssetClass::TechMegaCap];
        assert!(
            (ra - rb).abs() > 1e-9,
            "seed 42 and seed 43 produced identical TechMegaCap year-1 return: {}",
            ra
        );
    }

    // ---- Step 5: convergence tests -------------------------------------------

    #[test]
    fn run_monte_carlo_year_1_sample_mean_converges_to_target() {
        let result = run_with(42, 5000);
        let n = result.paths.len() as f64;
        let sample_mean = result
            .paths
            .iter()
            .map(|p| p.yearly_returns[1][&AssetClass::TechMegaCap])
            .sum::<f64>()
            / n;
        assert!(
            (sample_mean - 0.10).abs() < 0.015,
            "sample mean {} is not within 0.015 of target 0.10 (n=5000)",
            sample_mean
        );
    }

    #[test]
    fn run_monte_carlo_year_1_sample_returns_are_above_minus_one() {
        let result = run_with(42, 5000);
        for (i, path) in result.paths.iter().enumerate() {
            let r = path.yearly_returns[1][&AssetClass::TechMegaCap];
            assert!(
                r > -1.0,
                "path {i} year-1 TechMegaCap return {} <= -1.0 (lognormal guarantee violated)",
                r
            );
        }
    }

    // ---- Step 5: McResult metadata test --------------------------------------

    #[test]
    fn mc_result_records_seed_and_n_paths() {
        let result = run_with(99, 100);
        assert_eq!(result.seed, 99);
        assert_eq!(result.n_paths, 100);
    }

    // ---- Step 6: percentile math tests --------------------------------------

    #[test]
    fn percentile_p50_is_median() {
        let v = vec![1.0_f64, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&v, 50), 3.0);
    }

    #[test]
    fn percentile_p25_with_linear_interpolation() {
        // numpy: q=0.25 * (5-1) = 1.0 → exact index 1 → 2.0
        let v = vec![1.0_f64, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&v, 25), 2.0);
    }

    #[test]
    fn percentile_p95_uses_interpolation() {
        // numpy: 0.95 * 4 = 3.8 → between index 3 (4.0) and index 4 (5.0)
        // interpolated: 4.0 + 0.8 * (5.0 - 4.0) = 4.8
        let v = vec![1.0_f64, 2.0, 3.0, 4.0, 5.0];
        assert!((percentile(&v, 95) - 4.8).abs() < 1e-12);
    }

    #[test]
    fn percentile_endpoints() {
        let v = vec![1.0_f64, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(percentile(&v, 0), *v.first().unwrap());
        assert_eq!(percentile(&v, 100), *v.last().unwrap());
    }

    // ---- Step 6: percentiles_per_year shape tests ---------------------------

    #[test]
    fn percentiles_per_year_returns_one_row_per_year() {
        let data: Vec<Vec<f64>> = vec![vec![0.0_f64; 6]; 100];
        let result = percentiles_per_year(&data, |x: &f64| *x, &[50]);
        assert_eq!(result.len(), 6);
        for (y, row) in result.iter().enumerate() {
            assert_eq!(row.year, y as u32, "row {} has year field {}", y, row.year);
        }
    }

    #[test]
    fn percentiles_per_year_records_n_paths_per_year() {
        let data: Vec<Vec<f64>> = vec![vec![0.0_f64; 6]; 100];
        let result = percentiles_per_year(&data, |x: &f64| *x, &[50]);
        for row in &result {
            assert_eq!(
                row.n_paths, 100,
                "n_paths should be 100, got {}",
                row.n_paths
            );
        }
    }

    #[test]
    fn percentiles_per_year_preserves_pct_order() {
        // Deliberately unordered: output must mirror input order exactly.
        let data: Vec<Vec<f64>> = vec![vec![1.0_f64, 2.0, 3.0, 4.0, 5.0]; 5];
        let result = percentiles_per_year(&data, |x: &f64| *x, &[95, 5, 50]);
        let keys: Vec<u8> = result[0].percentiles.iter().map(|(q, _)| *q).collect();
        assert_eq!(keys, vec![95_u8, 5, 50]);
    }

    // ---- Step 6: percentiles_per_year correctness tests ---------------------

    #[test]
    fn percentiles_per_year_mean_is_arithmetic_mean() {
        // 5 paths × 1 year; values [1,2,3,4,5] → mean = 3.0 exactly
        let data: Vec<Vec<f64>> = vec![vec![1.0], vec![2.0], vec![3.0], vec![4.0], vec![5.0]];
        let result = percentiles_per_year(&data, |x: &f64| *x, &[50]);
        assert_eq!(result[0].mean, 3.0);
    }

    #[test]
    fn percentiles_per_year_p50_for_known_distribution() {
        // Same 5-path input; p50 must agree with percentile() directly.
        let data: Vec<Vec<f64>> = vec![vec![1.0], vec![2.0], vec![3.0], vec![4.0], vec![5.0]];
        let result = percentiles_per_year(&data, |x: &f64| *x, &[50]);
        let p50 = result[0]
            .percentiles
            .iter()
            .find(|(q, _)| *q == 50)
            .unwrap()
            .1;
        assert_eq!(p50, 3.0);
    }

    #[test]
    fn percentiles_per_year_extractor_applied_correctly() {
        // Values [10,20,30,40,50]; extractor divides by 10 → effective [1,2,3,4,5] → p50 = 3.0
        let data: Vec<Vec<f64>> = vec![vec![10.0], vec![20.0], vec![30.0], vec![40.0], vec![50.0]];
        let result = percentiles_per_year(&data, |x: &f64| x / 10.0, &[50]);
        let p50 = result[0]
            .percentiles
            .iter()
            .find(|(q, _)| *q == 50)
            .unwrap()
            .1;
        assert_eq!(p50, 3.0);
    }

    // ---- Step 6: integration bridge test ------------------------------------

    #[test]
    fn percentiles_per_year_year_1_mean_matches_target() {
        // Bridge test: build raw return series from run_monte_carlo, aggregate through
        // percentiles_per_year, verify year-1 mean for TechMegaCap is within 0.015 of 0.10.
        let mc = run_with(42, 5000);
        let raw: Vec<Vec<f64>> = mc
            .paths
            .iter()
            .map(|p| {
                p.yearly_returns
                    .iter()
                    .map(|hm| hm[&AssetClass::TechMegaCap])
                    .collect()
            })
            .collect();
        let agg = percentiles_per_year(&raw, |x: &f64| *x, &[5, 50, 95]);
        // Year-0 placeholder must be exactly zero for all paths.
        assert_eq!(
            agg[0].mean, 0.0,
            "year-0 mean should be 0.0, got {}",
            agg[0].mean
        );
        // Year-1 sample mean must converge to the 0.10 target.
        let y1_mean = agg[1].mean;
        assert!(
            (y1_mean - 0.10).abs() < 0.015,
            "year-1 mean {} is not within 0.015 of target 0.10",
            y1_mean,
        );
    }
}
