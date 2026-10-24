//! Performance benches for the Monte Carlo engine.
//!
//! Target: 5,000 paths × 5 years × 19 asset classes runs in ≤ 1.5 s on the dev laptop.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ghost_alpha::projection::asset_class_library::AssetClassLibrary;
use ghost_alpha::projection::monte_carlo::{
    build_block_corr, run_monte_carlo, AssetClass, CorrelationRules, McConfig,
};

fn bench_run_monte_carlo_5000_paths(c: &mut Criterion) {
    let library = AssetClassLibrary::builtin_defaults();
    let classes: Vec<AssetClass> = AssetClass::iter_all().collect();
    let corr = build_block_corr(&classes, &CorrelationRules::default());
    let cfg = McConfig::default(); // 5000 paths × 5 yrs × 19 classes, seed 42, lognormal

    c.bench_function("run_monte_carlo_5000_paths_5yr_19classes", |b| {
        b.iter(|| run_monte_carlo(black_box(&library), black_box(&corr), black_box(&cfg)))
    });
}

fn bench_run_monte_carlo_1000_paths(c: &mut Criterion) {
    let library = AssetClassLibrary::builtin_defaults();
    let classes: Vec<AssetClass> = AssetClass::iter_all().collect();
    let corr = build_block_corr(&classes, &CorrelationRules::default());
    let cfg = McConfig {
        n_paths: 1_000,
        ..McConfig::default()
    };

    c.bench_function("run_monte_carlo_1000_paths_5yr_19classes", |b| {
        b.iter(|| run_monte_carlo(black_box(&library), black_box(&corr), black_box(&cfg)))
    });
}

fn bench_build_block_corr(c: &mut Criterion) {
    let classes: Vec<AssetClass> = AssetClass::iter_all().collect();
    let rules = CorrelationRules::default();

    c.bench_function("build_block_corr_19classes", |b| {
        b.iter(|| build_block_corr(black_box(&classes), black_box(&rules)))
    });
}

criterion_group!(
    benches,
    bench_run_monte_carlo_5000_paths,
    bench_run_monte_carlo_1000_paths,
    bench_build_block_corr,
);
criterion_main!(benches);
