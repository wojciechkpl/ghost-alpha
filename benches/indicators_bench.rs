use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ghost_alpha::analysis::indicators::{adx, sma, volume_zscore};
use ghost_alpha::data::models::Candle;

// ─── Test data builders ───────────────────────────────────────────────────────

fn trending_candles(n: usize) -> Vec<Candle> {
    (0..n)
        .map(|i| {
            let c = 100.0 + i as f64 * 0.5;
            Candle::new(
                &format!("2024-{:02}-{:02}", 1 + i / 28, 1 + i % 28),
                c - 0.5,
                c + 1.0,
                c - 1.0,
                c,
                1_000_000u64 + (i as u64 * 1000),
            )
        })
        .collect()
}

// ─── Benchmarks ───────────────────────────────────────────────────────────────

fn bench_sma(c: &mut Criterion) {
    let candles = trending_candles(500);
    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();

    c.bench_function("sma_200_period_500_bars", |b| {
        b.iter(|| sma(black_box(&closes), black_box(200)).unwrap())
    });
}

fn bench_adx(c: &mut Criterion) {
    let candles = trending_candles(500);
    c.bench_function("adx_14_period_500_bars", |b| {
        b.iter(|| adx(black_box(&candles), black_box(14)).unwrap())
    });
}

fn bench_volume_zscore(c: &mut Criterion) {
    let candles = trending_candles(500);
    c.bench_function("volume_zscore_20_window_500_bars", |b| {
        b.iter(|| volume_zscore(black_box(&candles), black_box(20)).unwrap())
    });
}

criterion_group!(benches, bench_sma, bench_adx, bench_volume_zscore);
criterion_main!(benches);
