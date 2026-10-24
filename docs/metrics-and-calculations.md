# Metrics & Calculations Reference

This document covers every metric, formula, threshold, and algorithm used in GhostAlpha's screening, scoring, alerting, and exit-signal pipelines. All implementations are in Rust with zero external computation dependencies — every formula runs locally as pure math.

---

## Table of Contents

- [1. Technical Indicators](#1-technical-indicators)
  - [Simple Moving Average (SMA)](#simple-moving-average-sma)
  - [Average Directional Index (ADX)](#average-directional-index-adx)
  - [Average True Range (ATR)](#average-true-range-atr)
  - [Relative Strength Index (RSI)](#relative-strength-index-rsi)
  - [Volume Z-Score](#volume-z-score)
  - [Relative Strength Percentile](#relative-strength-percentile)
  - [Weighted RS (Multi-Timeframe)](#weighted-rs-multi-timeframe)
  - [RS Momentum](#rs-momentum)
  - [VCP (Volatility Contraction Pattern)](#vcp-volatility-contraction-pattern)
  - [Risk-Reward Ratio](#risk-reward-ratio)
  - [Liquidity Classification](#liquidity-classification)
- [2. Minervini 8-Point Trend Template](#2-minervini-8-point-trend-template)
- [3. Screening Strategies](#3-screening-strategies)
  - [Minervini (Default)](#minervini-default)
  - [Breakout](#breakout)
  - [Mean-Reversion](#mean-reversion)
  - [Trend-Strength](#trend-strength)
- [4. Composite Scoring](#4-composite-scoring)
  - [Technical Score (0-55)](#technical-score-0-55)
  - [Volume Score (0-30)](#volume-score-0-30)
  - [Sentiment Score (0-15)](#sentiment-score-0-15)
  - [Composite Score (0-100)](#composite-score-0-100)
  - [Regime-Adaptive Weights](#regime-adaptive-weights)
  - [Grade Assignment](#grade-assignment)
- [5. Market Regime Detection](#5-market-regime-detection)
- [6. Sentiment Analysis](#6-sentiment-analysis)
- [7. Alert Engine](#7-alert-engine)
  - [Severity Classification](#severity-classification)
  - [Deduplication](#deduplication)
  - [Quiet Hours](#quiet-hours)
  - [Daily Limit](#daily-limit)
  - [Sector Concentration](#sector-concentration)
  - [Bear Market Suppression](#bear-market-suppression)
- [8. Exit Signal Detection](#8-exit-signal-detection)
  - [Stage 2 Breakdown](#stage-2-breakdown)
  - [RS Deterioration](#rs-deterioration)
  - [MA Death Cross](#ma-death-cross)
  - [Conviction Decay](#conviction-decay)
- [9. Backtesting](#9-backtesting)
- [10. Data Pipeline](#10-data-pipeline)
  - [Cache Architecture](#cache-architecture)
  - [Rate Limiting](#rate-limiting)
- [11. Portfolio Projection (Phase 7)](#11-portfolio-projection-phase-7)
  - [Lognormal Monte Carlo](#lognormal-monte-carlo)
  - [Block-Correlation Covariance](#block-correlation-covariance)
  - [Concentration Floor Solver](#concentration-floor-solver)
  - [Cash Sequencing](#cash-sequencing)
  - [Percentile Aggregation](#percentile-aggregation)
- [Quick Reference: All Thresholds](#quick-reference-all-thresholds)

---

## 1. Technical Indicators

**Source:** `src/analysis/indicators.rs`

### Simple Moving Average (SMA)

The arithmetic mean of the last *N* closing prices.

```
SMA(N) = (P₁ + P₂ + ... + Pₙ) / N
```

| Period | Usage |
|--------|-------|
| 20 | Short-term trend, mean-reversion baseline, breadth calculation |
| 50 | Medium-term trend, Minervini conditions, death cross |
| 150 | Intermediate trend, Minervini conditions |
| 200 | Long-term trend, Minervini conditions, regime detection, death cross |

**Output:** Returns `prices.len() - period + 1` values, ordered oldest to newest.

### Average Directional Index (ADX)

Measures trend strength regardless of direction. Uses Wilder's smoothing method.

**Period:** 14 days (default)
**Minimum candles required:** `2 × period + 1 = 29`

**Step 1 — True Range (TR):**

```
TR = max(high - low, |high - prev_close|, |low - prev_close|)
```

**Step 2 — Directional Movement:**

```
up_move   = high - prev_high
down_move = prev_low - low

+DM = up_move   if (up_move > down_move AND up_move > 0)   else 0
-DM = down_move if (down_move > up_move AND down_move > 0) else 0
```

**Step 3 — Wilder's Smoothing** (applied to TR, +DM, -DM independently):

```
Seed:        sum of first N values
Subsequent:  smoothed = prev_smoothed - (prev_smoothed / N) + current_value
```

This is equivalent to an exponential moving average with `alpha = 1/N`.

**Step 4 — Directional Indicators:**

```
+DI = 100 × smoothed(+DM) / smoothed(TR)
-DI = 100 × smoothed(-DM) / smoothed(TR)
```

**Step 5 — DX and ADX:**

```
DX  = 100 × |+DI - -DI| / (+DI + -DI)
ADX = Wilder-smoothed DX series, clamped to [0, 100]
```

**Interpretation:**
- ADX < 20: Weak or absent trend
- ADX 20-25: Trend emerging
- ADX 25-50: Strong trend
- ADX > 50: Very strong trend

### Average True Range (ATR)

Wilder-smoothed True Range measuring volatility.

**Period:** 14 days (default)

```
Seed: SMA of first 14 TR values
Then: ATR = (prev_ATR × 13 + TR) / 14
```

Returns the latest single ATR value. Used for stop-loss calculation in the risk-reward ratio.

### Relative Strength Index (RSI)

Momentum oscillator measuring speed and magnitude of price changes.

**Period:** 14 days (default)

```
For each bar:
  gain = max(close - prev_close, 0)
  loss = max(prev_close - close, 0)

Seed (first 14 bars):
  avg_gain = mean(gains)
  avg_loss = mean(losses)

Subsequent bars (Wilder's smoothing):
  avg_gain = (prev_avg_gain × 13 + gain) / 14
  avg_loss = (prev_avg_loss × 13 + loss) / 14

RS  = avg_gain / avg_loss
RSI = 100 - (100 / (1 + RS))
```

**Special case:** If `avg_loss = 0`, RSI = 100.

**Interpretation:**
- RSI < 30: Oversold (used by mean-reversion screener)
- RSI > 70: Overbought
- RSI 30-70: Neutral zone

### Volume Z-Score

Directional volume anomaly detector with a 20-day rolling window. **The current bar is excluded from the baseline** to prevent look-ahead bias.

**Window:** 20 bars

```
baseline = volumes[i-20 .. i-1]  (20 bars before current)
mean     = average(baseline)
std      = standard_deviation(baseline)

If std < 1.0 (near-zero variance fallback):
    z = (volume[i] - mean) / mean    if mean >= 1.0
    z = 0                            otherwise

Normal case:
    z = (volume[i] - mean) / std

Direction adjustment:
    If close < open (bearish candle):
        signed_z = -|z|
    Else:
        signed_z = z
```

**Why directional?** A volume spike on a down-close signals distribution (selling pressure), while the same spike on an up-close signals accumulation. The sign encodes this.

**Output:** `candles.len() - 20` values.

### Relative Strength Percentile

Cross-sectional rank of a stock's return relative to the entire screening universe.

```
return_252d = (current_close - close_252d_ago) / close_252d_ago

count_below = number of stocks in universe with return < this stock's return

rs_percentile = 100 × count_below / universe_size
```

**Default (empty universe):** 50

This is a **relative** measure — a stock with RS 90 means it outperformed 90% of the universe over 252 trading days (~1 year).

### Weighted RS (Multi-Timeframe)

Combines four return periods with recency weighting.

```
weighted_rs = 0.40 × return_63d
            + 0.20 × return_126d
            + 0.20 × return_189d
            + 0.20 × return_252d
```

| Period | Trading Days | Calendar ~Months | Weight |
|--------|-------------|------------------|--------|
| 63d | 63 | ~3 months | 40% |
| 126d | 126 | ~6 months | 20% |
| 189d | 189 | ~9 months | 20% |
| 252d | 252 | ~12 months | 20% |

The 40% weight on the 63-day return gives more influence to recent momentum while retaining a full-year perspective.

**Minimum data:** 252 candles.

### RS Momentum

Rate of change in weighted RS over the past 21 trading days (~1 month).

```
rs_momentum = (weighted_rs_now - weighted_rs_21d_ago) / 21 × 100
```

Requires 273 candles minimum (252 + 21 lookback). A positive RS momentum means relative strength is accelerating.

### VCP (Volatility Contraction Pattern)

Detects Mark Minervini's "volatility contraction pattern" — a series of bases where each contraction is shallower than the last.

**Lookback:** 120 bars, divided into 3 equal segments of 40 bars.

```
For each segment:
    depth = (max_close - min_close) / max_close × 100%

base_count = count of segments where depth > 1% (noise filter)
contraction_ratio = depth_of_last_segment / depth_of_first_segment

VCP detected if:
    base_count >= 2  AND  contraction_ratio < 0.70
```

**Pivot:** Highest close in the final third (potential breakout level).
**Pivot proximity:** `(current_price - pivot) / pivot × 100%`

### Risk-Reward Ratio

Uses the 50-SMA and ATR to define a trailing stop and measures the upside potential to the 52-week high.

```
stop_loss = SMA(50) - 1.0 × ATR(14)
risk      = current_price - stop_loss
reward    = 52_week_high - current_price
ratio     = reward / risk
```

**Edge cases:**
- Risk <= 0 (price below stop): Returns capped at -10
- Reward <= 0 (at/above 52-week high): Returns 0
- Ratio clamped to [-10, 10]

### Liquidity Classification

Based on 20-day average dollar volume (`close × volume`).

| Class | Daily Dollar Volume |
|-------|-------------------|
| **High** | > $50M |
| **Medium** | $5M — $50M |
| **Low** | $1M — $5M |
| **Illiquid** | < $1M |

---

## 2. Minervini 8-Point Trend Template

**Source:** `src/analysis/indicators.rs` — `check_minervini_conditions()`
**Minimum candles:** 252

The 8-point filter is Mark Minervini's Stage 2 uptrend qualification template. A stock must satisfy **all 8 conditions** to be classified as `minervini_pass = true`.

| # | Condition | Formula | What it verifies |
|---|-----------|---------|-----------------|
| 1 | Price above both long-term MAs | `close > SMA(150) AND close > SMA(200)` | Sustained uptrend above key institutional levels |
| 2 | 150-SMA above 200-SMA | `SMA(150) > SMA(200)` | Moving average alignment (no crossover) |
| 3 | 200-SMA trending up | `SMA(200)_now > SMA(200)_21d_ago` | Long-term structural uptrend (minimum ~1 month) |
| 4 | 50-SMA above longer MAs | `SMA(50) > SMA(150) AND SMA(50) > SMA(200)` | Full moving average stack alignment |
| 5 | Price above 50-SMA | `close > SMA(50)` | Short-term momentum intact |
| 6 | Price well above 52-week low | `close >= 52w_low × 1.25` | At least 25% off the bottom |
| 7 | Price near 52-week high | `close >= 52w_high × 0.75` | Within 25% of highs, not extended |
| 8 | Relative strength above 70th percentile | `rs_percentile >= 70` | Outperforming 70%+ of universe |

**Output:** `(pass: bool, conditions_met: u8)` — a stock meeting 6/8 conditions gets `conditions_met = 6, pass = false`.

---

## 3. Screening Strategies

Each strategy produces a `ScreenResult` with the same structure but applies different selection criteria and scoring weights.

### Minervini (Default)

**Source:** `src/analysis/screener.rs`

The default screener runs every stock through the full indicator pipeline:

1. Calculate all indicators (SMA, ADX, RSI, ATR, volume Z-score)
2. Evaluate 8-point Minervini template
3. Compute RS percentile against the universe
4. Calculate weighted RS, RS momentum, VCP, risk-reward, liquidity
5. Score using composite formula with regime-adaptive weights
6. Return `ScreenResult` with all fields populated

**Scoring weights:** Technical 55%, Volume 30%, Sentiment 15%

### Breakout

**Source:** `src/analysis/breakout.rs`

Detects stocks breaking out of consolidation patterns on above-average volume.

**5 Selection Conditions:**

| # | Condition | Threshold |
|---|-----------|-----------|
| 1 | Near 52-week high | `close / 52w_high >= 0.95` (within 5%) |
| 2 | Above key SMAs | `close > SMA(50) AND close > SMA(200)` |
| 3 | Trending | `ADX > 20` |
| 4 | Volume surge | `volume_today >= 1.5 × SMA(50, volume)` |
| 5 | Consolidation base | >= 20 days with < 15% depth before breakout |

**Scoring weights:** Technical 45%, Volume 40%, Sentiment 15%

The higher volume weight reflects breakout strategy's emphasis on volume confirmation.

### Mean-Reversion

**Source:** `src/analysis/mean_reversion.rs`

Finds oversold stocks with strong fundamentals pulling back to buy.

**5 Selection Conditions:**

| # | Condition | Threshold |
|---|-----------|-----------|
| 1 | Oversold | `RSI(14) < 30` |
| 2 | Pullback not breakdown | `close < SMA(20) AND close > SMA(200)` |
| 3 | Not strong downtrend | `ADX(14) < 25` |
| 4 | Quality filter | `RS percentile > 50%` |
| 5 | Regime filter | Market = Bull OR Sideways (not Bear) |

**Scoring weights:** Technical 55%, Volume 20%, Sentiment 25%

The higher sentiment weight helps identify quality stocks with positive news despite temporary pullbacks.

### Trend-Strength

**Source:** `src/analysis/trend_strength.rs`

Ranks stocks by the absolute strength and alignment of their trend.

**5 Selection Conditions:**

| # | Condition | Threshold |
|---|-----------|-----------|
| 1 | Strong trend | `ADX > 30` |
| 2 | Bullish direction | `+DI > -DI` |
| 3 | Full MA alignment | `close > SMA(20) > SMA(50) > SMA(200)` |
| 4 | High relative strength | `RS percentile > 70%` |
| 5 | Above-average volume | `volume Z-score > 0` |

**Scoring weights:** Technical 75%, Volume 10%, Sentiment 15%

The heavy technical weight (75%) reflects this strategy's pure trend-following nature.

---

## 4. Composite Scoring

**Source:** `src/analysis/scorer.rs`

### Technical Score (0-55)

```
base = (conditions_met / 8) × 35

adx_bonus:
    ADX >= 50     →  +10 points (very strong trend)
    25 <= ADX < 50 →  +((ADX - 25) / 25) × 10  (linear interpolation)
    ADX < 25      →  +0  (no bonus for weak trends)

rs_bonus:
    RS >= 90      →  +10 points (top decile)
    70 <= RS < 90  →  +((RS - 70) / 20) × 10  (linear interpolation)
    RS < 70       →  +0  (no bonus)

technical_score = clamp(base + adx_bonus + rs_bonus, 0, 55)
```

**Example:** 7/8 conditions, ADX 35, RS 85:
- Base: (7/8) × 35 = 30.625
- ADX bonus: ((35-25)/25) × 10 = 4.0
- RS bonus: ((85-70)/20) × 10 = 7.5
- Total: 42.125

### Volume Score (0-30)

Maps the signed volume Z-score from [-3, +3] to [0, 30] linearly.

```
z_clamped = clamp(volume_zscore, -3, +3)
volume_score = ((z_clamped + 3) / 6) × 30
```

| Volume Z-Score | Volume Score | Interpretation |
|---------------|-------------|----------------|
| -3.0 (or lower) | 0 | Extreme selling volume |
| -1.0 | 10 | Below-average on down-close |
| 0.0 | 15 | Average volume |
| +1.0 | 20 | Above-average on up-close |
| +2.0 | 25 | Volume spike (accumulation) |
| +3.0 (or higher) | 30 | Extreme accumulation |

### Sentiment Score (0-15)

Maps the FinBERT compound score [-1, +1] to [0, 15], then applies a confidence weight based on article count.

```
raw = ((compound_score + 1) / 2) × 15

confidence_weight:
    0 articles  →  0.0  (no data, no score)
    1 article   →  0.33
    2 articles  →  0.67
    >= 3 articles →  1.0

sentiment_score = raw × confidence_weight
```

The confidence weight prevents single-article noise from dominating the score. Three or more articles are needed for full confidence.

### Composite Score (0-100)

```
composite = clamp(technical + volume + sentiment, 0, 100)
```

The maximum possible breakdown is 55 + 30 + 15 = 100.

### Regime-Adaptive Weights

The composite score's three sub-scores are weighted differently based on the current market regime:

| Regime | Technical | Volume | Sentiment | Rationale |
|--------|-----------|--------|-----------|-----------|
| **Bull** | 55% | 30% | 15% | Standard weights; volume confirmation matters |
| **Sideways** | 60% | 20% | 20% | Technical precision matters more; sentiment helps filter |
| **Bear** | 70% | 10% | 20% | Technical strength is paramount; volume less reliable |

These weights are applied within each screener's scoring step. The screener passes the current `RegimeState` to the scorer, which selects the appropriate weight set.

### Grade Assignment

Grades are assigned by **percentile rank within the current screening universe** — not by fixed score cutoffs.

```
sorted_scores = sort(all_composite_scores, ascending)

For each stock:
    rank = position in sorted_scores (0-indexed)
    percentile = rank / total_count
```

| Grade | Percentile Threshold | Meaning |
|-------|---------------------|---------|
| **A+** | >= 95th percentile | Top 5% — exceptional setup |
| **A** | >= 85th percentile | Top 15% — very strong |
| **B** | >= 65th percentile | Top 35% — above average |
| **C** | >= 40th percentile | Top 60% — average |
| **D** | >= 25th percentile | Top 75% — below average |
| **F** | < 25th percentile | Bottom 25% — weak / failing |

**Why percentile-based?** A "B" in a strong bull market with many qualifying stocks represents a different quality level than a "B" in a bear market with few qualifiers. Percentile grading adjusts automatically.

---

## 5. Market Regime Detection

**Source:** `src/analysis/regime.rs`

The market regime classifies the overall market environment using two signals.

### Input Signals

**SPY vs 200-SMA:** Is the S&P 500 ETF (SPY) trading above its 200-day simple moving average?

**Market breadth:** What percentage of the screening universe has its closing price above its own 50-day SMA?

```
breadth = (count of tickers where close > SMA(50)) / total_tickers × 100
```

### Classification Rules

| Regime | SPY Condition | Breadth Condition |
|--------|--------------|-------------------|
| **Bull** | SPY > SMA(200) | AND breadth > 60% |
| **Bear** | SPY < SMA(200) | AND breadth < 40% |
| **Sideways** | Mixed signals | (anything else) |

"Mixed signals" means: SPY above 200-SMA but breadth below 60%, or SPY below 200-SMA but breadth above 40%. This avoids false classification during transitions.

### Impact on Alerts

In **Bear** regime, only CRITICAL-severity alerts are sent. WARNING and INFO alerts are suppressed entirely. This is the primary noise-reduction mechanism — it prevents the system from flooding with alerts during broad market downturns where most setups are likely to fail.

---

## 6. Sentiment Analysis

**Source:** `src/analysis/sentiment.rs`

### FinBERT Model

Sentiment scoring uses FinBERT, a financial-domain fine-tuned BERT model running via ONNX Runtime. The model outputs three probabilities for each headline: positive, negative, neutral.

### Compound Score

```
compound = clamp(positive - negative, -1.0, +1.0)
```

### Time-Decay Weighting

Newer articles have more influence than older ones, using exponential half-life decay.

```
weight = exp(-ln(2) / half_life_days × age_in_days)
```

| Age | Weight (7-day half-life) |
|-----|------------------------|
| 0 days (today) | 1.00 |
| 3.5 days | 0.71 |
| 7 days | 0.50 |
| 14 days | 0.25 |
| 21 days | 0.13 |

### Aggregation

```
total_weight = sum(weight_i for all articles)
aggregate_compound = sum(weight_i × compound_i) / total_weight

Result:
    positive  = weighted average of positive scores
    negative  = weighted average of negative scores
    neutral   = weighted average of neutral scores
    compound  = weighted average of compound scores
    article_count = total articles processed
```

---

## 7. Alert Engine

**Source:** `src/alerts/engine.rs`

### Default Configuration

| Parameter | Default | Config Key |
|-----------|---------|------------|
| Critical threshold | 90.0 | `alerts.critical_score_threshold` |
| Warning threshold | 70.0 | `alerts.warning_score_threshold` |
| Info threshold | 50.0 | `alerts.info_score_threshold` |
| Volume spike threshold | 2.0 (Z-score) | `alerts.volume_spike_threshold` |
| Dedup cooldown | 7 days | `alerts.dedup_cooldown_days` |
| Max daily alerts | 50 | `alerts.max_daily_alerts` |
| Quiet start | 22:00 (10 PM) | `alerts.quiet_start_hour` |
| Quiet end | 07:00 (7 AM) | `alerts.quiet_end_hour` |
| Sector concentration | 3 alerts | `alerts.sector_concentration_threshold` |

### Severity Classification

```
if (minervini_pass AND volume_zscore >= 2.0) OR composite_score >= 90:
    → CRITICAL

elif composite_score >= 70:
    → WARNING

elif composite_score >= 50:
    → INFO

else:
    → NO ALERT (dropped)
```

There are two paths to CRITICAL:
1. **Score-based:** Composite score >= 90 (pure quantitative)
2. **Pattern-based:** Minervini pass + volume spike >= 2.0 Z-score (pattern recognition catch — a stock might score below 90 but have a textbook volume breakout with all 8 conditions met)

### Deduplication

```
key = (ticker, alert_type)

If the same key was sent within the cooldown window:
    → Skip (suppress duplicate)

Otherwise:
    → Send and record timestamp
```

The dedup store is backed by SQLite for persistence across process restarts. The cooldown window is 7 days by default.

### Quiet Hours

```
quiet_range = [22:00, 07:00) local time  (wraps midnight)

If current_hour is within quiet_range:
    CRITICAL alerts → ALWAYS sent (bypass quiet hours)
    WARNING/INFO   → SUPPRESSED until morning
```

### Daily Limit

```
If alerts_sent_today >= max_daily_alerts (default 50):
    → All further alerts suppressed for the day

max_daily_alerts = 0  →  unlimited
```

### Sector Concentration

After processing all alerts in a screening run:

```
For each GICS sector:
    count = number of alerts fired for tickers in this sector

If count >= sector_concentration_threshold (default 3):
    → Emit warning: "sector concentration: N alerts in sector 'X' — diversify!"
```

This helps traders avoid inadvertent sector overconcentration.

### Bear Market Suppression

When the market regime is **Bear**:
- Only **CRITICAL** alerts pass through
- **WARNING** and **INFO** are suppressed entirely
- This is checked in `AlertEngine::evaluate()` before severity classification

---

## 8. Exit Signal Detection

**Source:** `src/alerts/exit_signals.rs`

Stocks that pass the Minervini screen are **flagged** in the `flagged_stocks` table with `status = ACTIVE`. These are monitored daily for breakdown signals.

### Signal Priority

Exit signals are checked in priority order. The first triggered signal determines the exit:

```
1. Stage 2 Breakdown  (most severe — structural loss)
2. RS Deterioration    (moderate — relative weakening)
3. MA Death Cross      (long-term — trend reversal)
```

### Stage 2 Breakdown

**Trigger:** Stock has lost 3 or more Minervini conditions since entry.

```
conditions_lost = conditions_at_entry - conditions_now

if conditions_lost >= 3:
    → EXIT: Stage 2 Breakdown
```

This detects when a stock is losing its structural uptrend support — moving averages falling out of alignment, price dropping below key levels, etc.

### RS Deterioration

**Trigger:** RS percentile has dropped below 50 from a high starting point.

```
if entry_score >= 70 AND rs_percentile_now < 50:
    → EXIT: RS Deterioration
```

Only applies to stocks that entered with a strong score (>= 70). A stock that was outperforming 80%+ of the market and now underperforms the median is a significant deterioration signal.

### MA Death Cross

**Trigger:** The 50-day SMA crosses below the 200-day SMA.

```
if SMA(50) < SMA(200):
    → EXIT: MA Death Cross
```

The classic long-term trend reversal indicator. Once the 50-day drops below the 200-day, the stock's intermediate trend has turned negative.

### Conviction Decay

Monitors the composite score over time:

```
score_change = current_score - entry_score

if score_change <= -15:
    → Weakening (significant decay)

if previously_weakening AND score_change >= -5:
    → Recovered (bounced back)

if -15 < score_change < -5:
    → No alert (moderate, acceptable variation)
```

### Exit Alert Format

```
{TICKER} EXIT — {REASON}
Entry: ${entry_price} | Current: ${current_price} | P&L: {pnl_pct}%
Entry score: {entry_score}
```

P&L is calculated as: `(current_price - entry_price) / entry_price × 100`

After an exit signal fires:
1. The flagged stock's status changes to `EXITED`
2. The `exit_reason` and `exit_price` are recorded
3. An alert is sent through all configured channels

---

## 9. Backtesting

**Source:** `src/backtest/mod.rs`, `src/backtest/monte_carlo.rs`

### Walk-Forward Backtest

The backtesting engine simulates the screening pipeline on historical data:

1. Walk forward through historical dates
2. On each date, run the screening pipeline with data available up to that point
3. Record which stocks passed, their scores, and subsequent returns
4. Track portfolio performance assuming equal-weight positions

### Monte Carlo Simulation

Generates confidence intervals for performance estimates:

1. Resample daily returns with replacement (bootstrapping)
2. Run N simulations (default: 1000)
3. Calculate performance metrics for each simulation
4. Report percentile-based confidence intervals

**95% Confidence Interval:** Excludes the top 2.5% and bottom 2.5% of simulated outcomes.

Output includes an HTML report with embedded charts for visual analysis.

---

## 10. Data Pipeline

### Cache Architecture

**Source:** `src/data/cache.rs`

Three-level cache hierarchy minimizing API calls:

| Level | Storage | Speed | Persistence |
|-------|---------|-------|-------------|
| **L1** | In-memory HashMap | Fastest | Lost on restart |
| **L2** | File-based | Fast | Survives restart |
| **L3** | SQLite (`candles` table) | Moderate | Full durability |

Lookup order: L1 → L2 → L3 → API call. After a cache miss, the result is stored in all higher cache levels.

### Rate Limiting

**Source:** `src/data/fetcher.rs`

| Provider | Rate Limit | Retry Strategy |
|----------|-----------|----------------|
| Finnhub | 60 requests/minute (token bucket via `governor`) | Exponential backoff via `backoff` crate |
| Yahoo Finance | No hard limit (best-effort) | Exponential backoff |
| Sample (demo) | None | None (local data) |

---

## Quick Reference: All Thresholds

| Metric | Threshold | Where Used |
|--------|-----------|------------|
| **Minervini conditions (pass)** | 8/8 | Screener |
| **Price above 52w low** | >= 125% of low | Minervini condition 6 |
| **Price near 52w high** | >= 75% of high | Minervini condition 7 |
| **RS percentile (Minervini)** | >= 70% | Minervini condition 8 |
| **ADX (trending)** | > 20 | Breakout screener |
| **ADX (strong trend)** | > 25 | Scoring bonus starts |
| **ADX (very strong)** | > 30 | Trend-strength screener |
| **ADX (maximum bonus)** | >= 50 | Full 10-point ADX bonus |
| **RSI (oversold)** | < 30 | Mean-reversion screener |
| **Volume Z-score (spike)** | >= 2.0 | CRITICAL severity trigger |
| **Volume ratio (breakout)** | >= 1.5× 50-day avg | Breakout screener condition |
| **Consolidation depth** | < 15% | Breakout base |
| **Consolidation duration** | >= 20 days | Breakout base |
| **VCP contraction ratio** | < 0.70 | VCP detection |
| **VCP base count** | >= 2 | VCP detection |
| **Composite (CRITICAL)** | >= 90 | Alert severity |
| **Composite (WARNING)** | >= 70 | Alert severity |
| **Composite (INFO)** | >= 50 | Alert severity |
| **Breadth (bull)** | > 60% | Market regime |
| **Breadth (bear)** | < 40% | Market regime |
| **Dedup cooldown** | 7 days | Alert deduplication |
| **Daily alert limit** | 50 | Alert throttling |
| **Quiet hours** | 22:00-07:00 | Alert suppression |
| **Sector concentration** | >= 3 alerts | Diversification warning |
| **Stage 2 breakdown** | >= 3 conditions lost | Exit signal |
| **RS deterioration** | RS < 50 (from >= 80) | Exit signal |
| **Conviction decay** | Score drop >= 15 | Exit signal |
| **High liquidity** | > $50M daily | Liquidity class |
| **Medium liquidity** | $5M-$50M daily | Liquidity class |
| **Low liquidity** | $1M-$5M daily | Liquidity class |
| **Illiquid** | < $1M daily | Liquidity class |

---

---

## 11. Portfolio Projection (Phase 7)

**Source:** `src/projection/` — forward-NAV multi-year Monte Carlo with asset allocation, vesting, and concentration management.

### Lognormal Monte Carlo

Each simulated path assumes daily log-returns follow a multivariate normal distribution with hand-curated mean (μ) and volatility (σ) per asset class.

**Step 1 — Generate correlated standard normals:**

```
Z_t ~ MVN(0, Σ)  where Σ is block-correlation (see below)
```

**Step 2 — Map to log-returns:**

```
ln(S_{t+1} / S_t) = (μ - σ²/2) + σ × Z_t
```

**Step 3 — Update portfolio value per path:**

```
NAV_{t} = Σ(shares_i × price_i)  for each asset i
```

**Parameters:**

| Param | Source | Typical Value |
|-------|--------|---------------|
| **μ (annualized mean return)** | `config/asset_classes.toml` | 8.5% (equities), 3.5% (bonds), 5.2% (alternatives) |
| **σ (annualized volatility)** | `config/asset_classes.toml` | 18% (equities), 4.5% (bonds), 12% (alternatives) |
| **Paths** | `--paths` CLI flag | Default 5000 |
| **Seed** | `--seed` CLI flag | Default 42 (reproducible) |
| **Horizon** | `--horizon` CLI flag | Default 5 years |

**Code:** `src/projection/monte_carlo.rs` — `run_monte_carlo()`

### Block-Correlation Covariance

Σ is **hand-curated, PSD-enforced** to reflect realistic market structure. Three blocks:

| Block | Assets | Correlation |
|-------|--------|-------------|
| **Equities** | US Large, Mid, Small, Intl Developed, Emerging | 0.82–0.95 (high correlation) |
| **Bonds** | Treasuries, Investment Grade, High Yield | 0.45–0.70 (moderate correlation) |
| **Alternatives** | REITs, Commodities, Crypto | 0.25–0.55 (low correlation) |
| **Cross-block** | Equity ↔ Bonds: 0.15; Equity ↔ Alts: 0.30; Bonds ↔ Alts: 0.20 | Diversification benefit |

**Validation:** All eigenvalues > 0 (Cholesky decomposition succeeds) and **positive semi-definite** by construction.

**Code:** `src/projection/asset_class_library.rs` — `AssetClassLibrary::covariance_matrix()`

### Concentration Floor Solver

For portfolios with single-name concentration (e.g., AMZN RSU vesting), `compute_k_max()` determines the maximum fraction of vested shares that can be sold while maintaining a floor concentration (e.g., ≥35%).

**Algorithm:**

1. **Input:** Current holding, annual vesting, target floor (e.g., 35%), NAV projection
2. **For each year t:**
   - Remaining vested shares for vesting year t = S_t
   - Constrained unwind = max(0, (current_holding - floor×NAV) / price)
   - Cap annual sales to min(vested_t, constrained_unwind)
3. **Output:** Year-by-year sell schedule that respects the floor

**Code:** `src/portfolio/concentration.rs` — `compute_k_max()`

**Example:** Given a $600k concentrated position in a $1M portfolio and a 25% floor, the solver caps year-1 sales so the position is still ≥ 25% of year-end NAV after proceeds are reinvested.

### Cash Sequencing

When vested shares are sold, proceeds must be redeployed. Two allocation strategies:

| Strategy | Rule | Best For |
|----------|------|----------|
| **Greedy** | Deploy to highest-volatility bucket first (momentum) | Growth portfolios |
| **ProRata** | Deploy proportional to existing sleeve weights | Balanced/conservative |

**Code:** `src/portfolio/sequencer.rs` — `route_cash()`

### Percentile Aggregation

Across N paths, year-end NAVs are aggregated to percentile bands:

```
P_k = percentile(NAV_paths, k)  where k ∈ {5, 25, 50, 75, 95}
mean = Σ(NAV_paths) / N
```

**Percentile semantics:** NumPy linear interpolation (R=7 method).

**Output:** `nav_projection.csv` with columns `nav_p05, nav_p25, nav_p50, nav_p75, nav_p95, nav_mean` per (treatment, year).

**Code:** `src/projection/output.rs` — `write_nav_projection_csv()`

---

*All portfolio projection formulas are implemented in `src/projection/` with 282 unit + integration tests achieving >= 90% coverage. Cross-validated against an independent Python reference within 1% NAV. See [PLAN_PIPELINE.md](PLAN_PIPELINE.md) for end-to-end examples.*
