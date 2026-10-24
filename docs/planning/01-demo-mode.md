# Plan 01: Demo Mode — Zero-Config First Experience

**Effort:** Small (2-3 days)
**Impact:** Very High
**Priority:** 1

## Problem

A new user discovering Ghost Alpha must:
1. Sign up for a Finnhub API key
2. Create a `config.toml`
3. Run `ghost-alpha migrate`
4. Run `ghost-alpha screen`

Most people abandon at step 1. The tool has no way to show its value without external dependencies.

## Solution

Add a `ghost-alpha demo` subcommand that runs the full screening pipeline against bundled sample data. No API key, no config, no internet connection required.

## Implementation

### 1. Bundle sample data as a Rust constant

Create `src/data/sample.rs`:

```rust
/// 50 stocks x 300 trading days of OHLCV data, serialized as compressed JSON.
/// Snapshot date: 2026-03-01 (anonymized to avoid licensing issues).
pub const SAMPLE_CANDLES: &[u8] = include_bytes!("../../data/sample_candles.json.zst");
pub const SAMPLE_NEWS: &[u8] = include_bytes!("../../data/sample_news.json.zst");
```

- Use `zstd` compression to keep binary size impact under 500KB
- Include 50 S&P 500 tickers with realistic price histories
- Include 200 sample news headlines for sentiment demo

### 2. Add `demo` subcommand

In `src/main.rs`, add to the `Cli` enum:

```rust
/// Run a demo screening with bundled sample data (no API key required)
Demo {
    /// Number of top results to show
    #[arg(long, default_value = "10")]
    top: usize,
    /// Include sentiment scoring in demo
    #[arg(long)]
    with_sentiment: bool,
},
```

### 3. Implement `SampleProvider`

Create a `SampleProvider` that implements `DataProvider` by decompressing and returning the bundled data. Register it in the plugin registry when `demo` is invoked.

```rust
pub struct SampleProvider {
    candles: HashMap<String, Vec<Candle>>,
    news: HashMap<String, Vec<NewsArticle>>,
}

impl SampleProvider {
    pub fn load() -> Self {
        let raw = zstd::decode_all(SAMPLE_CANDLES).unwrap();
        // ... deserialize
    }
}
```

### 4. Demo output format

```
$ ghost-alpha demo

  Ghost Alpha Demo — screening 50 stocks (bundled sample data)
  ─────────────────────────────────────────────────────────────

  Rank  Ticker  Score  Grade  Minervini  RS%ile  Vol-Z  Regime
  ────  ──────  ─────  ─────  ─────────  ──────  ─────  ──────
   1    NVDA    94.2    A+      8/8       97      2.8   Bull
   2    PLTR    87.1    A       7/8       91      1.9   Bull
   3    META    82.5    A-      8/8       85      1.2   Bull
  ...

  This is sample data. To screen live markets:
    1. Get a free API key at https://finnhub.io
    2. Run: ghost-alpha setup
```

### 5. Sample data generation script

Add `scripts/generate_sample_data.py` (or Rust binary) that:
- Fetches real data from Finnhub
- Anonymizes timestamps (shift by random offset)
- Compresses with zstd
- Writes to `data/sample_candles.json.zst`

Run once manually; committed output goes into the repo.

## Files to create/modify

| File | Action |
|------|--------|
| `src/data/sample.rs` | Create — sample data loader |
| `src/data/mod.rs` | Modify — add `pub mod sample` |
| `src/main.rs` | Modify — add `Demo` subcommand + handler |
| `data/sample_candles.json.zst` | Create — bundled compressed data |
| `data/sample_news.json.zst` | Create — bundled compressed news |
| `scripts/generate_sample_data.py` | Create — one-time data generator |
| `Cargo.toml` | Modify — add `zstd = "0.13"` dependency |

## Dependencies

- `zstd = "0.13"` — decompression only, ~200KB added to binary

## Binary size impact

- Sample data: ~400KB compressed (50 stocks x 300 days)
- zstd decompressor: ~200KB
- **Total: ~600KB** (binary goes from 5.0MB to ~5.6MB)

## Testing

- Unit test: `SampleProvider` returns expected number of candles/tickers
- Unit test: `demo` subcommand produces non-empty output
- Integration test: Full demo pipeline runs without errors

## Acceptance criteria

- [ ] `ghost-alpha demo` works on a fresh machine with no config/API keys
- [ ] Output shows a ranked table with scores, grades, and regime
- [ ] Runs in under 2 seconds
- [ ] Binary size stays under 6MB
- [ ] Footer directs user to `ghost-alpha setup` for live data
