//! Projection engine: lognormal Monte Carlo simulation with block-correlation Σ.
//!
//! See `docs/PLAN_PIPELINE.md` for how the projection is used.
//!
//! # Reproducibility
//!
//! All MC runs are **deterministic given a seed**. The same `(seed, n_paths)`
//! produces byte-identical output. Default seed is 42 (`McConfig::default()`).
//! RNG is `rand::rngs::StdRng` (ChaCha-12); this is a different stream from
//! numpy's PCG64, so cross-language reproducibility is **statistical**, not
//! byte-exact — see `tests/mc_returns_match_python.rs`.
//!
//! # Quick start
//!
//! ```no_run
//! use ghost_alpha::projection::asset_class_library::AssetClassLibrary;
//! use ghost_alpha::projection::monte_carlo::{
//!     build_block_corr, run_monte_carlo, AssetClass, CorrelationRules, McConfig,
//! };
//!
//! let library = AssetClassLibrary::builtin_defaults();
//! let classes: Vec<AssetClass> = AssetClass::iter_all().collect();
//! let corr = build_block_corr(&classes, &CorrelationRules::default());
//! let result = run_monte_carlo(&library, &corr, &McConfig::default());
//! assert_eq!(result.n_paths, 5000);
//! ```

pub mod asset_class_library;
pub mod monte_carlo;
pub mod output;
pub mod portfolio_nav;
pub mod scenario;
