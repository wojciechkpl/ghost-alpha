# Plan 09: Built-In Strategy Presets

**Effort:** Large (5-7 days)
**Impact:** High
**Priority:** 9

## Problem

Ghost Alpha only implements the Minervini trend template. Users who follow different investment methodologies (O'Neil CANSLIM, breakout trading, mean reversion) have no reason to use the tool. The `Screener` trait exists for extensibility, but nobody will write Rust plugins.

## Solution

Ship 4 built-in screening strategies selectable via `--strategy`. Each reuses existing indicators (SMA, ADX, RS, volume Z-score) with different selection logic and scoring weights.

## Strategies

### 1. Minervini (existing, default)

**Philosophy:** Stage 2 uptrend momentum — buy stocks in confirmed uptrends before acceleration.

Already implemented. No changes needed.

### 2. Breakout Scanner

**Philosophy:** Detect price/volume breakouts from consolidation bases. Optimized for swing traders.

**Selection criteria:**
- Price within 5% of 52-week high
- Volume today > 1.5x 50-day average
- ADX > 20 (trending)
- Price above 50-SMA and 200-SMA
- At least 20 days in a consolidation range (< 15% depth) before breakout

**Scoring weights:**
- Proximity to 52-week high: 30%
- Volume surge magnitude: 30%
- Consolidation quality (tightness + duration): 25%
- RS percentile: 15%

```rust
pub struct BreakoutScreener;

impl Screener for BreakoutScreener {
    fn screen(&self, candles: &[Candle]) -> ScreenResult {
        let high_52w = max_high(&candles[candles.len()-252..]);
        let current = candles.last().unwrap().close;
        let proximity = current / high_52w;  // 0.95+ = near high

        let vol_surge = volume_ratio(candles, 50);
        let consolidation = detect_consolidation(candles, 20, 0.15);

        // ...
    }

    fn name(&self) -> &'static str { "breakout" }
}
```

### 3. Mean Reversion (Oversold Bounce)

**Philosophy:** Buy quality stocks that are temporarily oversold in a bullish market. Contrarian approach.

**Selection criteria:**
- RSI(14) < 30 (oversold)
- Price below 20-SMA but above 200-SMA (pullback, not breakdown)
- RS percentile > 50 (still strong relative to peers on longer timeframe)
- Market regime = Bull or Sideways (don't catch knives in bears)
- ADX < 25 (not in a strong downtrend)

**Scoring weights:**
- Oversold depth (lower RSI = higher score): 30%
- Distance below 20-SMA (mean reversion potential): 25%
- Long-term RS (quality filter): 25%
- Volume contraction (selling exhaustion): 20%

```rust
pub struct MeanReversionScreener;

impl Screener for MeanReversionScreener {
    fn screen(&self, candles: &[Candle]) -> ScreenResult {
        let rsi = compute_rsi(candles, 14);
        let sma20 = sma(&closes(candles), 20);
        let sma200 = sma(&closes(candles), 200);
        let current = candles.last().unwrap().close;

        let oversold = rsi < 30.0 && current < sma20 && current > sma200;
        // ...
    }

    fn name(&self) -> &'static str { "mean-reversion" }
}
```

### 4. Trend Strength (ADX Momentum)

**Philosophy:** Ride strong established trends regardless of stage. Focused on trend strength rather than entry timing.

**Selection criteria:**
- ADX > 30 (strong trend)
- +DI > -DI (bullish directional)
- Price above 20-SMA, 50-SMA, and 200-SMA (all MAs aligned)
- RS percentile > 70
- Volume above 20-day average

**Scoring weights:**
- ADX strength: 35%
- DI separation (+DI - -DI): 20%
- MA alignment quality: 20%
- RS percentile: 15%
- Volume confirmation: 10%

```rust
pub struct TrendStrengthScreener;
```

## Implementation

### 1. Add RSI indicator

RSI is needed for mean reversion but not currently implemented. Add to `src/analysis/indicators.rs`:

```rust
/// Relative Strength Index (Wilder's smoothing).
pub fn rsi(closes: &[f64], period: usize) -> Option<f64> {
    if closes.len() < period + 1 { return None; }

    let mut avg_gain = 0.0;
    let mut avg_loss = 0.0;

    // Initial average
    for i in 1..=period {
        let change = closes[i] - closes[i - 1];
        if change > 0.0 { avg_gain += change; }
        else { avg_loss += change.abs(); }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;

    // Smoothed
    for i in (period + 1)..closes.len() {
        let change = closes[i] - closes[i - 1];
        let (gain, loss) = if change > 0.0 { (change, 0.0) } else { (0.0, change.abs()) };
        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;
    }

    if avg_loss == 0.0 { return Some(100.0); }
    let rs = avg_gain / avg_loss;
    Some(100.0 - (100.0 / (1.0 + rs)))
}
```

### 2. Add consolidation detection

For breakout scanner, add to `src/analysis/indicators.rs`:

```rust
/// Detect consolidation: at least `min_days` where range < `max_depth` percent.
pub fn detect_consolidation(candles: &[Candle], min_days: usize, max_depth: f64) -> Option<Consolidation> {
    // Walk backwards from current, find tightest range
    // Return depth, duration, pivot price
}
```

### 3. Strategy registry

Update plugin registry to handle multiple screeners:

```rust
// CLI
ghost-alpha screen --strategy breakout
ghost-alpha screen --strategy mean-reversion
ghost-alpha screen --strategy trend-strength
ghost-alpha screen --strategy minervini  # default

// List available strategies
ghost-alpha strategies
```

### 4. Composite scorer adaptation

Each strategy should be able to provide its own scoring weights:

```rust
pub trait Screener: Send + Sync {
    fn screen(&self, candles: &[Candle], context: &ScreenContext) -> ScreenResult;
    fn name(&self) -> &'static str;
    fn scoring_weights(&self) -> ScoringWeights {
        ScoringWeights::default()  // 55/30/15
    }
}
```

## Files to create/modify

| File | Action |
|------|--------|
| `src/analysis/indicators.rs` | Modify — add RSI, consolidation detection |
| `src/analysis/breakout.rs` | Create — breakout screener |
| `src/analysis/mean_reversion.rs` | Create — mean reversion screener |
| `src/analysis/trend_strength.rs` | Create — trend strength screener |
| `src/analysis/screener.rs` | Modify — add `scoring_weights()` to trait |
| `src/analysis/mod.rs` | Modify — add new modules |
| `src/plugins/registry.rs` | Modify — register all strategies |
| `src/main.rs` | Modify — add `--strategy` flag, `strategies` subcommand |

## Testing

Per strategy:
- Unit test: Passes with ideal fixture data
- Unit test: Fails with anti-pattern fixture data
- Unit test: Scoring weights sum to 100%
- Unit test: RSI calculation matches known values
- Integration test: Full pipeline with each strategy

## Acceptance criteria

- [ ] 4 strategies available: minervini, breakout, mean-reversion, trend-strength
- [ ] `--strategy` flag on `screen` command selects strategy
- [ ] `ghost-alpha strategies` lists all available strategies with descriptions
- [ ] Each strategy has its own scoring weights
- [ ] RSI indicator added with >=95% test coverage
- [ ] Default strategy remains minervini (no breaking change)
- [ ] Backtest engine works with all strategies
