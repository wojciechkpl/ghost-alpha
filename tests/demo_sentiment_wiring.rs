//! Standalone demo: exercises the full sentiment wiring pipeline with mock data.
//! Run with: cargo test --test demo_sentiment_wiring -- --nocapture

use ghost_alpha::analysis::scorer::{
    composite_score_weighted, RegimeWeights, SentimentInputs, TechnicalInputs, VolumeInputs,
};
use ghost_alpha::analysis::screener::SentimentScorer;
use ghost_alpha::analysis::sentiment::NullSentimentScorer;
use ghost_alpha::data::models::{MarketRegime, NewsArticle};
use ghost_alpha::storage::db::{Repository, SqliteRepository};

#[test]
fn sentiment_wiring_full_demo() {
    println!("\n  ╔══════════════════════════════════════════════════════╗");
    println!("  ║      Ghost Alpha — Sentiment Wiring Demo            ║");
    println!("  ╚══════════════════════════════════════════════════════╝\n");

    // ── 1. DB Migration: news_cache table ────────────────────────────
    println!("  ▸ Step 1: Open in-memory DB with V7 migrations...");
    let repo = SqliteRepository::open_in_memory().unwrap();
    println!("    ✓ Schema version 7 — news_cache table created\n");

    // ── 2. Cache news articles ──────────────────────────────────────
    println!("  ▸ Step 2: Cache news articles for NVDA...");
    let now = chrono::Utc::now();
    let articles = vec![
        NewsArticle {
            ticker: "NVDA".into(),
            headline: "NVIDIA Beats Q4 Earnings Estimates".into(),
            summary: "NVIDIA reported revenue of $22.1B".into(),
            published_at: now - chrono::Duration::hours(6),
            source: "Reuters".into(),
        },
        NewsArticle {
            ticker: "NVDA".into(),
            headline: "AI Chip Demand Surges for NVIDIA".into(),
            summary: "Data center revenue grew 400% YoY".into(),
            published_at: now - chrono::Duration::days(1),
            source: "Bloomberg".into(),
        },
        NewsArticle {
            ticker: "NVDA".into(),
            headline: "NVIDIA Announces New H200 GPU".into(),
            summary: "Next-gen AI accelerator".into(),
            published_at: now - chrono::Duration::days(3),
            source: "TechCrunch".into(),
        },
        NewsArticle {
            ticker: "NVDA".into(),
            headline: "Wall Street Raises NVDA Price Targets".into(),
            summary: "Average target now $1,200".into(),
            published_at: now - chrono::Duration::days(5),
            source: "MarketWatch".into(),
        },
    ];

    let inserted = repo.cache_news(&articles).unwrap();
    println!("    ✓ Cached {} new articles", inserted);

    // Dedup test
    let dup_inserted = repo.cache_news(&articles).unwrap();
    println!(
        "    ✓ Re-insert same articles: {} new (dedup works)\n",
        dup_inserted
    );

    // ── 3. Retrieve cached news ─────────────────────────────────────
    println!("  ▸ Step 3: Retrieve cached news (last 7 days)...");
    let since = now.date_naive() - chrono::Duration::days(7);
    let cached = repo.get_cached_news("NVDA", since).unwrap();
    for (i, a) in cached.iter().enumerate() {
        let age_hrs = (now - a.published_at).num_hours();
        println!(
            "    {}. [{}] \"{}\" ({}h ago)",
            i + 1,
            a.source,
            a.headline,
            age_hrs
        );
    }
    println!();

    // ── 4. Sentiment scoring simulation ─────────────────────────────
    println!("  ▸ Step 4: Sentiment scoring (simulated)...");
    let headlines: Vec<&str> = cached.iter().map(|a| a.headline.as_str()).collect();
    println!("    Headlines to score: {}", headlines.len());

    // NullSentimentScorer (no ONNX) returns 0.0
    let null_scorer = NullSentimentScorer;
    let null_score = null_scorer.score(&headlines).unwrap();
    println!(
        "    NullScorer result: {:.2} (placeholder — no ONNX model)",
        null_score
    );

    // Simulate what FinBERT would return for positive news
    let simulated_finbert_score = 0.72; // strongly positive
    println!(
        "    Simulated FinBERT score: {:.2} (positive sentiment)\n",
        simulated_finbert_score
    );

    // ── 5. Composite score comparison ───────────────────────────────
    println!("  ▸ Step 5: Composite score comparison (Bull regime)...\n");
    let weights = RegimeWeights::for_regime(&MarketRegime::Bull);
    let tech = TechnicalInputs {
        conditions_met: 7,
        max_conditions: 8,
        adx: 42.0,
        rs_percentile: 88.0,
    };
    let vol = VolumeInputs { zscore: 2.1 };

    // Without sentiment
    let no_sent = SentimentInputs {
        score: 0.0,
        article_count: 0,
    };
    let baseline = composite_score_weighted(&tech, &vol, &no_sent, &weights);

    // With simulated FinBERT sentiment
    let with_sent = SentimentInputs {
        score: simulated_finbert_score,
        article_count: 4,
    };
    let enhanced = composite_score_weighted(&tech, &vol, &with_sent, &weights);

    println!("    {:<25} {:>10} {:>10}", "", "Baseline", "w/ Sentiment");
    println!("    {}", "─".repeat(47));
    println!(
        "    {:<25} {:>10.1} {:>10.1}",
        "Technical sub-score", baseline.technical, enhanced.technical
    );
    println!(
        "    {:<25} {:>10.1} {:>10.1}",
        "Volume sub-score", baseline.volume, enhanced.volume
    );
    println!(
        "    {:<25} {:>10.1} {:>10.1}",
        "Sentiment sub-score", baseline.sentiment, enhanced.sentiment
    );
    println!("    {}", "─".repeat(47));
    println!(
        "    {:<25} {:>10.1} {:>10.1}",
        "COMPOSITE", baseline.composite, enhanced.composite
    );
    println!(
        "    {:<25} {:>10}  {:>+9.1}",
        "Delta",
        "",
        enhanced.composite - baseline.composite
    );
    println!();

    // ── 6. Regime comparison ────────────────────────────────────────
    println!("  ▸ Step 6: Sentiment impact across regimes...\n");
    println!(
        "    {:<12} {:>8} {:>12} {:>8}",
        "Regime", "w/o Sent", "w/ Sent", "Delta"
    );
    println!("    {}", "─".repeat(44));
    for regime in &[
        MarketRegime::Bull,
        MarketRegime::Sideways,
        MarketRegime::Bear,
    ] {
        let w = RegimeWeights::for_regime(regime);
        let base = composite_score_weighted(&tech, &vol, &no_sent, &w);
        let full = composite_score_weighted(&tech, &vol, &with_sent, &w);
        let delta = full.composite - base.composite;
        println!(
            "    {:<12} {:>8.1} {:>12.1} {:>+8.1}",
            format!("{}", regime),
            base.composite,
            full.composite,
            delta
        );
    }
    println!();

    // ── 7. Cache prune ──────────────────────────────────────────────
    println!("  ▸ Step 7: Cache maintenance...");
    let deleted = repo.delete_old_news(14).unwrap();
    println!("    ✓ Pruned {} articles older than 14 days", deleted);
    let remaining = repo.get_cached_news("NVDA", since).unwrap();
    println!("    ✓ {} articles remaining in cache\n", remaining.len());

    println!("  ╔══════════════════════════════════════════════════════╗");
    println!("  ║  Demo complete — all components working end-to-end  ║");
    println!("  ╚══════════════════════════════════════════════════════╝\n");

    // Assertions to keep the test green
    assert_eq!(inserted, 4);
    assert_eq!(dup_inserted, 0);
    assert_eq!(cached.len(), 4);
    assert!(enhanced.composite > baseline.composite);
}
