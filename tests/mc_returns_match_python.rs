//! Step 8(c) of Gap 8a: cross-language statistical equivalence.
//!
//! Runs the Rust MC engine at the same (seed, n_paths) as the Python prototype
//! and asserts per-asset-class year-1 return moments (mean, std) are within
//! tolerance. RNG streams differ (Rust ChaCha vs numpy PCG64), so we test
//! statistical equivalence — NOT byte-equality.
//!
//! Fixture: tests/fixtures/python_mc/year1_moments.json (portfolio-independent;
//! depends only on the asset-class library, correlation rules, seed and n_paths).

use ghost_alpha::projection::asset_class_library::AssetClassLibrary;
use ghost_alpha::projection::monte_carlo::{
    build_block_corr, run_monte_carlo, AssetClass, CorrelationRules, McConfig,
};
use serde::Deserialize;
use std::collections::BTreeMap;

const PY_FIXTURE: &str = include_str!("fixtures/python_mc/year1_moments.json");

#[derive(Deserialize)]
struct PythonMomentsFile {
    n_paths: usize,
    seed: u64,
    year1_moments: BTreeMap<String, PythonMoment>,
}

#[derive(Deserialize)]
struct PythonMoment {
    mean: f64,
    std: f64,
    #[allow(dead_code)]
    p05: f64,
    #[allow(dead_code)]
    p50: f64,
    #[allow(dead_code)]
    p95: f64,
}

fn sample_mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

fn sample_std(xs: &[f64]) -> f64 {
    let m = sample_mean(xs);
    let var = xs.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (xs.len() - 1) as f64;
    var.sqrt()
}

#[test]
fn rust_mc_year1_moments_match_python_within_tolerance() {
    let py: PythonMomentsFile =
        serde_json::from_str(PY_FIXTURE).expect("fixture year1_moments.json parses as JSON");

    assert_eq!(py.seed, 42, "fixture seed must match this test's seed");
    assert_eq!(
        py.n_paths, 5000,
        "fixture n_paths must match this test's n_paths"
    );

    let library = AssetClassLibrary::builtin_defaults();
    let classes: Vec<AssetClass> = AssetClass::iter_all().collect();
    let corr = build_block_corr(&classes, &CorrelationRules::default());
    let cfg = McConfig {
        seed: py.seed,
        n_paths: py.n_paths,
        ..McConfig::default()
    };
    let mc = run_monte_carlo(&library, &corr, &cfg);

    // Tolerances: at N=5000 with two independent MC samples, expected
    // difference of sample means ≈ σ·√(2/N). For σ ≤ 0.32 that's ≤ 0.0064 at
    // 1σ. The thresholds below are ~3σ headroom — should never flake but will
    // catch any real distribution drift.
    let mean_tol_relative = 0.05; // 5% relative
    let mean_tol_absolute = 0.02; // 2 pp floor (matters for low-mean classes)
    let std_tol_relative = 0.05; // 5% relative
    let std_tol_absolute = 0.02;

    let mut failures: Vec<String> = Vec::new();
    for c in &classes {
        let pascal = format!("{:?}", c);
        let py_m = py
            .year1_moments
            .get(&pascal)
            .unwrap_or_else(|| panic!("python fixture missing class {}", pascal));

        let r_y1: Vec<f64> = mc.paths.iter().map(|p| p.yearly_returns[1][c]).collect();
        let r_mean = sample_mean(&r_y1);
        let r_std = sample_std(&r_y1);

        let mean_tol = (py_m.mean.abs() * mean_tol_relative).max(mean_tol_absolute);
        let std_tol = (py_m.std.abs() * std_tol_relative).max(std_tol_absolute);

        if (r_mean - py_m.mean).abs() > mean_tol {
            failures.push(format!(
                "{}: mean rust={:.6} py={:.6} diff={:.6} tol={:.6}",
                pascal,
                r_mean,
                py_m.mean,
                (r_mean - py_m.mean).abs(),
                mean_tol
            ));
        }
        if (r_std - py_m.std).abs() > std_tol {
            failures.push(format!(
                "{}: std rust={:.6} py={:.6} diff={:.6} tol={:.6}",
                pascal,
                r_std,
                py_m.std,
                (r_std - py_m.std).abs(),
                std_tol
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} class moment mismatches:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}
