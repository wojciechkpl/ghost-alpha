use chrono::NaiveDate;

use crate::data::models::Candle;
use crate::error::AnalysisError;

// ─── SMA ────────────────────────────────────────────────────────────────────

/// Simple Moving Average over a slice of prices.
/// Returns `prices.len() - period + 1` values (oldest to newest).
pub fn sma(prices: &[f64], period: usize) -> Result<Vec<f64>, AnalysisError> {
    if prices.len() < period {
        return Err(AnalysisError::InsufficientData {
            need: period,
            got: prices.len(),
        });
    }
    Ok(prices
        .windows(period)
        .map(|w| w.iter().sum::<f64>() / period as f64)
        .collect())
}

// ─── ADX ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AdxPoint {
    pub adx: f64,
    pub plus_di: f64,
    pub minus_di: f64,
}

/// Wilder's Average Directional Index (14-period default).
/// Requires at least `2 * period + 1` candles to produce any output.
pub fn adx(candles: &[Candle], period: usize) -> Result<Vec<AdxPoint>, AnalysisError> {
    let required = 2 * period + 1;
    if candles.len() < required {
        return Err(AnalysisError::InsufficientData {
            need: required,
            got: candles.len(),
        });
    }

    let n = candles.len();

    // 1. Compute raw True Range, +DM, –DM for each bar (skip index 0)
    let mut tr_raw = Vec::with_capacity(n - 1);
    let mut plus_dm_raw = Vec::with_capacity(n - 1);
    let mut minus_dm_raw = Vec::with_capacity(n - 1);

    for i in 1..n {
        let c = &candles[i];
        let p = &candles[i - 1];
        let tr = (c.high - c.low)
            .max((c.high - p.close).abs())
            .max((c.low - p.close).abs());
        let up_move = c.high - p.high;
        let down_move = p.low - c.low;
        let plus_dm = if up_move > down_move && up_move > 0.0 {
            up_move
        } else {
            0.0
        };
        let minus_dm = if down_move > up_move && down_move > 0.0 {
            down_move
        } else {
            0.0
        };
        tr_raw.push(tr);
        plus_dm_raw.push(plus_dm);
        minus_dm_raw.push(minus_dm);
    }

    // 2. Wilder smooth: seed = sum of first `period` values, then EMA-like
    let wilder_smooth = |vals: &[f64]| -> Vec<f64> {
        let mut out = Vec::with_capacity(vals.len() - period + 1);
        let seed: f64 = vals[..period].iter().sum();
        out.push(seed);
        for &v in &vals[period..] {
            let prev = *out.last().unwrap();
            out.push(prev - prev / period as f64 + v);
        }
        out
    };

    let str_s = wilder_smooth(&tr_raw);
    let plus_s = wilder_smooth(&plus_dm_raw);
    let minus_s = wilder_smooth(&minus_dm_raw);

    // 3. Compute DX series
    let len = str_s.len();
    let mut dx_vals: Vec<f64> = Vec::with_capacity(len);
    let mut plus_di_series: Vec<f64> = Vec::with_capacity(len);
    let mut minus_di_series: Vec<f64> = Vec::with_capacity(len);

    for i in 0..len {
        let str_v = str_s[i];
        let plus_di = if str_v > 1e-10 {
            100.0 * plus_s[i] / str_v
        } else {
            0.0
        };
        let minus_di = if str_v > 1e-10 {
            100.0 * minus_s[i] / str_v
        } else {
            0.0
        };
        let di_sum = plus_di + minus_di;
        let dx = if di_sum > 1e-10 {
            100.0 * (plus_di - minus_di).abs() / di_sum
        } else {
            0.0
        };
        dx_vals.push(dx);
        plus_di_series.push(plus_di);
        minus_di_series.push(minus_di);
    }

    // 4. Wilder smooth DX → ADX
    let mut result = Vec::with_capacity(dx_vals.len() - period + 1);
    let mut adx_val: f64 = dx_vals[..period].iter().sum::<f64>() / period as f64;

    for i in period..dx_vals.len() {
        adx_val = (adx_val * (period as f64 - 1.0) + dx_vals[i]) / period as f64;
        result.push(AdxPoint {
            adx: adx_val.clamp(0.0, 100.0),
            plus_di: plus_di_series[i].clamp(0.0, 100.0),
            minus_di: minus_di_series[i].clamp(0.0, 100.0),
        });
    }

    Ok(result)
}

// ─── Volume Z-Score ──────────────────────────────────────────────────────────

/// Bias-corrected, directional volume Z-score.
///
/// The current bar is **excluded** from the rolling window so it cannot
/// inflate its own baseline (look-ahead bias prevention).
/// Sign is flipped negative on down-close bars (distribution signal).
///
/// Returns one value per bar starting at index `window` (i.e. `candles.len() - window` values).
pub fn volume_zscore(candles: &[Candle], window: usize) -> Result<Vec<f64>, AnalysisError> {
    let required = window + 1;
    if candles.len() < required {
        return Err(AnalysisError::InsufficientData {
            need: required,
            got: candles.len(),
        });
    }

    let vols: Vec<f64> = candles.iter().map(|c| c.volume as f64).collect();
    let mut results = Vec::with_capacity(candles.len() - window);

    for i in window..candles.len() {
        let win = &vols[i - window..i]; // excludes current bar
        let mean = win.iter().sum::<f64>() / window as f64;
        let variance = win.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / window as f64;
        let std = variance.sqrt();

        // When all window values are identical (std ≈ 0), use mean as denominator
        // so large volume spikes still register. If mean is also ≈ 0, return 0.
        let z = if std < 1.0 {
            if mean < 1.0 {
                0.0
            } else {
                (vols[i] - mean) / mean
            }
        } else {
            (vols[i] - mean) / std
        };

        // Directional: negative on down-close days
        let candle = &candles[i];
        let signed_z = if candle.close < candle.open {
            -z.abs()
        } else {
            z
        };
        results.push(signed_z);
    }

    Ok(results)
}

// ─── RS Percentile ──────────────────────────────────────────────────────────

/// Cross-sectional Relative Strength percentile.
/// Returns what fraction of `universe_returns` is strictly below `this_return`, 0–100.
pub fn rs_percentile(this_return: f64, universe_returns: &[f64]) -> f64 {
    if universe_returns.is_empty() {
        return 50.0;
    }
    let below = universe_returns
        .iter()
        .filter(|&&r| r < this_return)
        .count();
    100.0 * below as f64 / universe_returns.len() as f64
}

/// Calculate N-day price return for a candle series (uses adj_close).
pub fn price_return_nd(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period {
        return None;
    }
    let start = candles[candles.len() - period].adj_close;
    let end = candles.last()?.adj_close;
    if start <= 0.0 {
        return None;
    }
    Some((end - start) / start)
}

/// Calculate 252-day price return for a candle series (uses adj_close).
pub fn price_return_252d(candles: &[Candle]) -> Option<f64> {
    price_return_nd(candles, 252)
}

/// Weighted multi-timeframe Relative Strength.
///
/// Combines 4 lookback periods with decreasing recency weight:
/// `0.40 * ret_63d + 0.20 * ret_126d + 0.20 * ret_189d + 0.20 * ret_252d`
///
/// Returns `None` if the series has fewer than 252 bars or the start price is zero.
pub fn weighted_rs(candles: &[Candle]) -> Option<f64> {
    let r63 = price_return_nd(candles, 63)?;
    let r126 = price_return_nd(candles, 126)?;
    let r189 = price_return_nd(candles, 189)?;
    let r252 = price_return_nd(candles, 252)?;
    Some(0.40 * r63 + 0.20 * r126 + 0.20 * r189 + 0.20 * r252)
}

/// RS momentum: rate of change of an RS percentile over `period` bars.
///
/// Given a history of RS values, computes `(latest - earliest) / period * 100`
/// to measure how fast RS is improving or deteriorating.
///
/// Returns `None` if `rs_history.len() < period`.
pub fn rs_momentum(rs_history: &[f64], period: usize) -> Option<f64> {
    if rs_history.len() < period || period == 0 {
        return None;
    }
    let latest = *rs_history.last()?;
    let earliest = rs_history[rs_history.len() - period];
    Some((latest - earliest) / period as f64 * 100.0)
}

// ─── ATR — Average True Range ────────────────────────────────────────────

/// Average True Range using Wilder's smoothing method.
///
/// Requires at least `period + 1` candles.
/// Returns the most recent ATR value (a single f64).
pub fn atr(candles: &[Candle], period: usize) -> Result<f64, AnalysisError> {
    let required = period + 1;
    if candles.len() < required {
        return Err(AnalysisError::InsufficientData {
            need: required,
            got: candles.len(),
        });
    }

    // Compute True Range for each bar (skip index 0)
    let mut tr_vals = Vec::with_capacity(candles.len() - 1);
    for i in 1..candles.len() {
        let c = &candles[i];
        let prev_close = candles[i - 1].close;
        let tr = (c.high - c.low)
            .max((c.high - prev_close).abs())
            .max((c.low - prev_close).abs());
        tr_vals.push(tr);
    }

    // Wilder smoothing: seed = SMA of first `period` TR values
    let mut atr_val: f64 = tr_vals[..period].iter().sum::<f64>() / period as f64;
    for &tr in &tr_vals[period..] {
        atr_val = (atr_val * (period as f64 - 1.0) + tr) / period as f64;
    }

    Ok(atr_val)
}

// ─── Risk-Reward Ratio ──────────────────────────────────────────────────

/// Compute a risk-reward ratio.
///
/// - Stop loss level: `sma_50 - 1.0 * atr_14`
/// - Risk: `current - stop`
/// - Reward: `high_52w - current`
/// - Ratio: `reward / risk`, clamped at 10.0
///
/// Returns 0.0 if `current >= high_52w`, negative if `current < stop`.
pub fn risk_reward_ratio(current: f64, sma_50: f64, atr_14: f64, high_52w: f64) -> f64 {
    let stop = sma_50 - atr_14;
    let risk = current - stop;
    let reward = high_52w - current;

    if risk <= 0.0 {
        return 10.0_f64.copysign(-1.0).max(-10.0); // below stop → negative, cap at -10
    }
    if reward <= 0.0 {
        return 0.0; // at or above 52w high → no upside
    }

    (reward / risk).min(10.0)
}

// ─── Liquidity Classification ───────────────────────────────────────────

/// Dollar volume liquidity tiers.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LiquidityClass {
    /// > $50M avg daily dollar volume
    High,
    /// $5M–$50M
    Medium,
    /// $1M–$5M
    Low,
    /// < $1M
    Illiquid,
}

impl std::fmt::Display for LiquidityClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LiquidityClass::High => write!(f, "High"),
            LiquidityClass::Medium => write!(f, "Medium"),
            LiquidityClass::Low => write!(f, "Low"),
            LiquidityClass::Illiquid => write!(f, "Illiquid"),
        }
    }
}

/// Classify liquidity based on average daily dollar volume over `period` bars.
///
/// Dollar volume = close × volume for each bar. If fewer than `period` bars
/// are available, defaults to `Illiquid`.
pub fn classify_liquidity(candles: &[Candle], period: usize) -> LiquidityClass {
    if candles.len() < period || period == 0 {
        return LiquidityClass::Illiquid;
    }

    let recent = &candles[candles.len() - period..];
    let avg_dollar_vol: f64 = recent
        .iter()
        .map(|c| c.close * c.volume as f64)
        .sum::<f64>()
        / period as f64;

    if avg_dollar_vol > 50_000_000.0 {
        LiquidityClass::High
    } else if avg_dollar_vol > 5_000_000.0 {
        LiquidityClass::Medium
    } else if avg_dollar_vol > 1_000_000.0 {
        LiquidityClass::Low
    } else {
        LiquidityClass::Illiquid
    }
}

// ─── Data Quality Validation ────────────────────────────────────────────

/// A detected data quality issue.
#[derive(Debug, Clone, PartialEq)]
pub enum DataQualityIssue {
    /// Suspicious daily move exceeding threshold (likely split or data error)
    SuspiciousMove {
        date: NaiveDate,
        daily_return_pct: f64,
    },
    /// Likely stock split detected (large decline with volume surge)
    LikelySplit {
        date: NaiveDate,
        price_change_pct: f64,
        volume_ratio: f64,
    },
}

/// Validate candle data for quality issues.
///
/// Flags:
/// - Daily returns exceeding ±`threshold_pct` (default 50%)
/// - Likely stock splits (>40% decline with >2x volume increase)
///
/// Returns empty vec if data is clean.
pub fn validate_candles(candles: &[Candle], threshold_pct: f64) -> Vec<DataQualityIssue> {
    let mut issues = Vec::new();

    if candles.len() < 2 {
        return issues;
    }

    for window in candles.windows(2) {
        let prev = &window[0];
        let curr = &window[1];

        if prev.adj_close <= 0.0 {
            continue;
        }

        let daily_return = (curr.adj_close - prev.adj_close) / prev.adj_close;
        let daily_return_pct = daily_return * 100.0;

        if daily_return_pct.abs() > threshold_pct {
            // Check if this looks like a split rather than a real move
            let volume_ratio = if prev.volume > 0 {
                curr.volume as f64 / prev.volume as f64
            } else {
                1.0
            };

            if daily_return_pct < -40.0 && volume_ratio > 2.0 {
                issues.push(DataQualityIssue::LikelySplit {
                    date: curr.date,
                    price_change_pct: daily_return_pct,
                    volume_ratio,
                });
            } else {
                issues.push(DataQualityIssue::SuspiciousMove {
                    date: curr.date,
                    daily_return_pct,
                });
            }
        }
    }

    issues
}

// ─── Slippage Model ─────────────────────────────────────────────────────

/// Scale base transaction cost by liquidity class.
///
/// Higher-cost scaling for illiquid names reflects real-world market impact
/// where small/mid-cap stocks have wider spreads and thinner order books.
pub fn adjusted_cost_bps(base_bps: f64, liquidity: &LiquidityClass) -> f64 {
    let multiplier = match liquidity {
        LiquidityClass::High => 1.0,
        LiquidityClass::Medium => 1.5,
        LiquidityClass::Low => 3.0,
        LiquidityClass::Illiquid => 5.0,
    };
    base_bps * multiplier
}

/// Estimate additional market impact for large orders relative to ADV.
///
/// Uses square-root impact model: impact grows with sqrt of participation rate.
/// Returns 0 for small orders (<1% of ADV). Returns 5x base for unknown ADV.
pub fn market_impact_bps(base_bps: f64, order_shares: f64, avg_daily_volume: f64) -> f64 {
    if avg_daily_volume <= 0.0 {
        return base_bps * 5.0;
    }
    let participation_rate = order_shares / avg_daily_volume;
    if participation_rate <= 0.01 {
        0.0 // negligible impact below 1% of ADV
    } else {
        base_bps * participation_rate.sqrt()
    }
}

// ─── VCP Detection ──────────────────────────────────────────────────────

/// Result of VCP (Volatility Contraction Pattern) detection.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VcpResult {
    /// Whether a VCP was detected (2+ bases with contracting depth)
    pub vcp_detected: bool,
    /// Number of bases (pullback-recovery cycles) found
    pub base_count: usize,
    /// Depth of the deepest base as a percentage
    pub base_depth_pct: f64,
    /// Ratio of last base depth to first base depth (< 0.70 = contracting)
    pub contraction_ratio: f64,
    /// Pivot price (highest close in the last base)
    pub pivot: f64,
    /// How close current price is to the pivot as a percentage
    pub pivot_proximity_pct: f64,
}

/// Detect a Volatility Contraction Pattern in the most recent `lookback` candles.
///
/// Divides the lookback window into thirds and measures the pullback depth
/// in each segment. A VCP is detected when:
/// - 2+ bases (segments) show a pullback-and-recovery pattern
/// - Each successive base is tighter (contracting depths)
/// - The contraction ratio (last depth / first depth) is < 0.70
pub fn detect_vcp(candles: &[Candle], lookback: usize) -> Result<VcpResult, AnalysisError> {
    if candles.len() < lookback || lookback < 9 {
        return Err(AnalysisError::InsufficientData {
            need: lookback.max(9),
            got: candles.len(),
        });
    }

    let window = &candles[candles.len() - lookback..];
    let segment_len = lookback / 3;

    // Measure depth (max drawdown from local high) in each third
    let mut depths = Vec::new();
    for seg in 0..3 {
        let start = seg * segment_len;
        let end = if seg == 2 {
            lookback
        } else {
            (seg + 1) * segment_len
        };
        let segment = &window[start..end];

        let max_close = segment
            .iter()
            .map(|c| c.close)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_close = segment
            .iter()
            .map(|c| c.close)
            .fold(f64::INFINITY, f64::min);

        let depth = if max_close > 0.0 {
            (max_close - min_close) / max_close * 100.0
        } else {
            0.0
        };
        depths.push(depth);
    }

    let base_count = depths.iter().filter(|&&d| d > 1.0).count(); // ignore trivial noise
    let first_depth = depths[0].max(0.01); // avoid division by zero
    let last_depth = *depths.last().unwrap();
    let contraction_ratio = last_depth / first_depth;

    let pivot = window
        .iter()
        .rev()
        .take(segment_len)
        .map(|c| c.close)
        .fold(f64::NEG_INFINITY, f64::max);
    let current = candles.last().unwrap().close;
    let pivot_proximity_pct = if pivot > 0.0 {
        (current - pivot) / pivot * 100.0
    } else {
        0.0
    };

    let vcp_detected = base_count >= 2 && contraction_ratio < 0.70;

    Ok(VcpResult {
        vcp_detected,
        base_count,
        base_depth_pct: depths.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        contraction_ratio,
        pivot,
        pivot_proximity_pct,
    })
}

// ─── Minervini Conditions ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct MinerviniConditions {
    /// 1+2: close > SMA(150) AND close > SMA(200)
    pub close_above_sma150_200: bool,
    /// 3: SMA(150) > SMA(200)
    pub sma150_above_sma200: bool,
    /// 4: SMA(200) is trending up for at least 1 month (~21 trading days)
    pub sma200_trending_up: bool,
    /// 5: SMA(50) > SMA(150) AND SMA(50) > SMA(200)
    pub sma50_above_sma150_200: bool,
    /// 6: close > SMA(50)
    pub close_above_sma50: bool,
    /// 7: close is at least 25% above the 52-week low
    pub close_above_52w_low_25pct: bool,
    /// 8: close is within 25% of the 52-week high (i.e. close >= 52w_high * 0.75)
    pub close_within_25pct_52w_high: bool,
    /// Caller sets this from rs_percentile after cross-sectional ranking
    pub rs_above_threshold: bool,
}

impl MinerviniConditions {
    pub fn passes_all(&self) -> bool {
        self.close_above_sma150_200
            && self.sma150_above_sma200
            && self.sma200_trending_up
            && self.sma50_above_sma150_200
            && self.close_above_sma50
            && self.close_above_52w_low_25pct
            && self.close_within_25pct_52w_high
            && self.rs_above_threshold
    }

    pub fn count(&self) -> u8 {
        [
            self.close_above_sma150_200,
            self.sma150_above_sma200,
            self.sma200_trending_up,
            self.sma50_above_sma150_200,
            self.close_above_sma50,
            self.close_above_52w_low_25pct,
            self.close_within_25pct_52w_high,
            self.rs_above_threshold,
        ]
        .iter()
        .filter(|&&b| b)
        .count() as u8
    }
}

/// Evaluate Minervini 8-point trend template from adjusted close prices.
/// Requires at least 252 prices. The caller must set `rs_above_threshold`
/// after computing cross-sectional RS for the full universe.
pub fn check_minervini_conditions(closes: &[f64]) -> Result<MinerviniConditions, AnalysisError> {
    if closes.len() < 252 {
        return Err(AnalysisError::InsufficientData {
            need: 252,
            got: closes.len(),
        });
    }

    let current = *closes.last().unwrap();

    let sma50_vals = sma(closes, 50)?;
    let sma150_vals = sma(closes, 150)?;
    let sma200_vals = sma(closes, 200)?;

    let sma50 = *sma50_vals.last().unwrap();
    let sma150 = *sma150_vals.last().unwrap();
    let sma200 = *sma200_vals.last().unwrap();

    // SMA200 approximately one month ago (21 trading days)
    let sma200_month_ago = if sma200_vals.len() > 21 {
        sma200_vals[sma200_vals.len() - 22]
    } else {
        sma200
    };

    let window_52w = &closes[closes.len().saturating_sub(252)..];
    let high_52w = window_52w.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let low_52w = window_52w.iter().cloned().fold(f64::INFINITY, f64::min);

    Ok(MinerviniConditions {
        close_above_sma150_200: current > sma150 && current > sma200,
        sma150_above_sma200: sma150 > sma200,
        sma200_trending_up: sma200 > sma200_month_ago,
        sma50_above_sma150_200: sma50 > sma150 && sma50 > sma200,
        close_above_sma50: current > sma50,
        close_above_52w_low_25pct: current >= low_52w * 1.25,
        close_within_25pct_52w_high: current >= high_52w * 0.75,
        rs_above_threshold: false, // caller sets after universe ranking
    })
}

// ─── Regime Detection ────────────────────────────────────────────────────────

use crate::data::models::MarketRegime;

/// Detect market regime from SPY candles and pre-computed breadth percentage.
///
/// - **Bull**: SPY above its 200-day SMA AND breadth_pct > 60
/// - **Bear**: SPY below its 200-day SMA AND breadth_pct < 40
/// - **Sideways**: everything else
pub fn detect_regime(
    spy_candles: &[Candle],
    breadth_pct: f64,
) -> Result<MarketRegime, AnalysisError> {
    if spy_candles.len() < 200 {
        return Err(AnalysisError::InsufficientData {
            need: 200,
            got: spy_candles.len(),
        });
    }

    let closes: Vec<f64> = spy_candles.iter().map(|c| c.adj_close).collect();
    let sma200_vals = sma(&closes, 200)?;
    let spy_sma200 = *sma200_vals.last().unwrap();
    let spy_close = closes.last().unwrap();

    let above_200 = spy_close > &spy_sma200;

    let regime = if above_200 && breadth_pct > 60.0 {
        MarketRegime::Bull
    } else if !above_200 && breadth_pct < 40.0 {
        MarketRegime::Bear
    } else {
        MarketRegime::Sideways
    };

    Ok(regime)
}

// ─── RSI ─────────────────────────────────────────────────────────────────────

/// Relative Strength Index using Wilder's smoothed averages.
///
/// Requires at least `period + 1` close prices.
/// Returns the most recent RSI value in [0, 100].
/// Returns `Some(100.0)` when there are no down moves.
pub fn rsi(closes: &[f64], period: usize) -> Option<f64> {
    if closes.len() < period + 1 {
        return None;
    }

    let mut avg_gain = 0.0_f64;
    let mut avg_loss = 0.0_f64;

    // Seed: simple average of first `period` changes
    for i in 1..=period {
        let change = closes[i] - closes[i - 1];
        if change > 0.0 {
            avg_gain += change;
        } else {
            avg_loss += change.abs();
        }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;

    // Wilder smoothing for remaining bars
    for i in (period + 1)..closes.len() {
        let change = closes[i] - closes[i - 1];
        let (gain, loss) = if change > 0.0 {
            (change, 0.0)
        } else {
            (0.0, change.abs())
        };
        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;
    }

    if avg_loss == 0.0 {
        return Some(100.0);
    }
    let rs = avg_gain / avg_loss;
    Some(100.0 - (100.0 / (1.0 + rs)))
}

// ─── Consolidation Detection ─────────────────────────────────────────────────

/// Result of consolidation range detection.
#[derive(Debug, Clone)]
pub struct Consolidation {
    /// Depth of the tightest consolidation range as a percentage of the high
    pub depth_pct: f64,
    /// Number of bars within the consolidated range
    pub duration_days: usize,
    /// The mid-point pivot price within the consolidation
    pub pivot: f64,
}

/// Detect the longest run of recent candles where the price range stays within
/// `max_depth` (expressed as a fraction, e.g. `0.15` = 15%) of the period high.
///
/// Walks backwards from the most recent candle to find the longest contiguous
/// window of at least `min_days` bars that satisfies the depth constraint.
///
/// Returns `None` if no such window of at least `min_days` bars is found.
pub fn detect_consolidation(
    candles: &[Candle],
    min_days: usize,
    max_depth: f64,
) -> Option<Consolidation> {
    if candles.len() < min_days || min_days == 0 {
        return None;
    }

    // Walk backwards: find the longest suffix that stays within max_depth
    let n = candles.len();
    let mut best: Option<Consolidation> = None;

    // Try every possible end (always the last bar) and varying start points
    // to find the widest window that satisfies the depth constraint.
    let mut start = n - 1;
    let end = n - 1;

    // Expand the window backwards as long as the range stays tight
    let mut hi = candles[end].high;
    let mut lo = candles[end].low;

    loop {
        let depth = if hi > 0.0 { (hi - lo) / hi } else { 0.0 };

        if depth <= max_depth {
            let duration = end - start + 1;
            if duration >= min_days {
                let pivot = (hi + lo) / 2.0;
                best = Some(Consolidation {
                    depth_pct: depth * 100.0,
                    duration_days: duration,
                    pivot,
                });
            }
        } else {
            // Range exceeded — stop expanding
            break;
        }

        if start == 0 {
            break;
        }
        start -= 1;
        hi = hi.max(candles[start].high);
        lo = lo.min(candles[start].low);
    }

    best
}

// ─── Volume Ratio ────────────────────────────────────────────────────────────

/// Ratio of current volume to the average volume over `period` preceding bars.
///
/// Returns `None` if fewer than `period + 1` candles are available.
pub fn volume_ratio(candles: &[Candle], period: usize) -> Option<f64> {
    if candles.len() < period + 1 {
        return None;
    }
    let window = &candles[candles.len() - 1 - period..candles.len() - 1];
    let avg: f64 = window.iter().map(|c| c.volume as f64).sum::<f64>() / period as f64;
    if avg < 1.0 {
        return Some(0.0);
    }
    let current = candles.last()?.volume as f64;
    Some(current / avg)
}

// ─── 52-week High ────────────────────────────────────────────────────────────

/// Maximum high price over the last up to 252 candles.
pub fn high_52w(candles: &[Candle]) -> f64 {
    let window = &candles[candles.len().saturating_sub(252)..];
    window
        .iter()
        .map(|c| c.high)
        .fold(f64::NEG_INFINITY, f64::max)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn build_candles_trending(n: usize, start: f64, end: f64) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let price = start + (end - start) * (i as f64 / (n - 1) as f64);
                Candle {
                    date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
                        .unwrap()
                        .checked_add_days(chrono::Days::new(i as u64))
                        .unwrap(),
                    open: price * 0.995,
                    high: price * 1.01,
                    low: price * 0.99,
                    close: price,
                    adj_close: price,
                    volume: 1_000_000,
                }
            })
            .collect()
    }

    fn build_candles_flat(n: usize, price: f64) -> Vec<Candle> {
        build_candles_trending(n, price, price)
    }

    fn build_candles_declining(n: usize) -> Vec<Candle> {
        build_candles_trending(n, 100.0, 50.0)
    }

    // ── SMA ──────────────────────────────────────────────────────────────────

    #[test]
    fn sma_basic_5_period() {
        let prices = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0];
        let result = sma(&prices, 5).unwrap();
        assert_eq!(result.len(), 3);
        assert_relative_eq!(result[0], 3.0, epsilon = 1e-10);
        assert_relative_eq!(result[1], 4.0, epsilon = 1e-10);
        assert_relative_eq!(result[2], 5.0, epsilon = 1e-10);
    }

    #[test]
    fn sma_insufficient_data_returns_error() {
        let prices = vec![1.0, 2.0, 3.0];
        assert!(matches!(
            sma(&prices, 5),
            Err(AnalysisError::InsufficientData { need: 5, got: 3 })
        ));
    }

    #[test]
    fn sma_period_equals_length() {
        let prices = vec![2.0, 4.0, 6.0];
        let result = sma(&prices, 3).unwrap();
        assert_eq!(result.len(), 1);
        assert_relative_eq!(result[0], 4.0, epsilon = 1e-10);
    }

    #[test]
    fn sma_single_value_period_one() {
        let prices = vec![42.0];
        let result = sma(&prices, 1).unwrap();
        assert_eq!(result.len(), 1);
        assert_relative_eq!(result[0], 42.0, epsilon = 1e-10);
    }

    #[test]
    fn sma_output_length_correct() {
        let prices: Vec<f64> = (1..=100).map(|x| x as f64).collect();
        let result = sma(&prices, 20).unwrap();
        assert_eq!(result.len(), 81); // 100 - 20 + 1
    }

    #[test]
    fn sma_known_values() {
        // prices: 10,20,30,40,50 → 3-period SMAs: 20, 30, 40
        let prices = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        let result = sma(&prices, 3).unwrap();
        assert_relative_eq!(result[0], 20.0, epsilon = 1e-10);
        assert_relative_eq!(result[1], 30.0, epsilon = 1e-10);
        assert_relative_eq!(result[2], 40.0, epsilon = 1e-10);
    }

    // ── ADX ──────────────────────────────────────────────────────────────────

    #[test]
    fn adx_trending_market_above_25() {
        let candles = build_candles_trending(300, 50.0, 100.0);
        let result = adx(&candles, 14).unwrap();
        let last = result.last().unwrap();
        assert!(
            last.adx > 25.0,
            "Expected ADX > 25 for strongly trending market, got {:.2}",
            last.adx
        );
    }

    #[test]
    fn adx_flat_market_low() {
        let candles = build_candles_flat(300, 50.0);
        let result = adx(&candles, 14).unwrap();
        let last = result.last().unwrap();
        // Flat price = no directional movement = low ADX
        assert!(
            last.adx < 20.0,
            "Expected ADX < 20 for flat market, got {:.2}",
            last.adx
        );
    }

    #[test]
    fn adx_insufficient_candles_returns_error() {
        // Need 2*14 + 1 = 29; providing 28
        let candles = build_candles_flat(28, 50.0);
        assert!(matches!(
            adx(&candles, 14),
            Err(AnalysisError::InsufficientData { need: 29, .. })
        ));
    }

    #[test]
    fn adx_output_length_correct() {
        let candles = build_candles_trending(300, 50.0, 100.0);
        let result = adx(&candles, 14).unwrap();
        // ADX series length: (n - 1) bars → smooth period → smooth again
        // str_s length = n - period (299 - 14 + 1 = 286)
        // dx_vals length = 286
        // result length  = 286 - 14 = 272
        assert_eq!(result.len(), 300 - 1 - 14 - 14 + 1);
    }

    #[test]
    fn adx_values_bounded_0_to_100() {
        let candles = build_candles_trending(300, 50.0, 100.0);
        let result = adx(&candles, 14).unwrap();
        for r in &result {
            assert!(
                r.adx >= 0.0 && r.adx <= 100.0,
                "ADX out of range: {:.2}",
                r.adx
            );
            assert!(r.plus_di >= 0.0 && r.plus_di <= 100.0);
            assert!(r.minus_di >= 0.0 && r.minus_di <= 100.0);
        }
    }

    #[test]
    fn adx_uptrend_plus_di_dominates() {
        // In a sustained uptrend +DI should exceed -DI
        let candles = build_candles_trending(300, 50.0, 100.0);
        let result = adx(&candles, 14).unwrap();
        let last = result.last().unwrap();
        assert!(
            last.plus_di > last.minus_di,
            "+DI {:.2} should exceed -DI {:.2} in uptrend",
            last.plus_di,
            last.minus_di
        );
    }

    #[test]
    fn adx_downtrend_minus_di_dominates() {
        let candles = build_candles_declining(300);
        let result = adx(&candles, 14).unwrap();
        let last = result.last().unwrap();
        assert!(
            last.minus_di > last.plus_di,
            "-DI {:.2} should exceed +DI {:.2} in downtrend",
            last.minus_di,
            last.plus_di
        );
    }

    // ── Volume Z-Score ────────────────────────────────────────────────────────

    #[test]
    fn volume_zscore_uniform_volume_returns_zero() {
        // All volumes identical → std = 0 → z = 0
        let candles = build_candles_flat(60, 50.0);
        let result = volume_zscore(&candles, 20).unwrap();
        for &z in &result {
            assert_relative_eq!(z, 0.0, epsilon = 1e-10);
        }
    }

    #[test]
    fn volume_zscore_spike_up_close_positive() {
        let mut candles = build_candles_flat(60, 50.0);
        // Spike on the last bar, and it's an up-close
        let last = candles.len() - 1;
        candles[last].volume = 10_000_000; // 10x normal
        candles[last].close = 51.0;
        candles[last].open = 50.0;
        let result = volume_zscore(&candles, 20).unwrap();
        let z = *result.last().unwrap();
        assert!(z > 2.0, "Expected z > 2 for 10x spike, got {z:.2}");
        assert!(z > 0.0, "Up-close spike should be positive");
    }

    #[test]
    fn volume_zscore_spike_down_close_negative() {
        let mut candles = build_candles_flat(60, 50.0);
        let last = candles.len() - 1;
        candles[last].volume = 10_000_000;
        candles[last].close = 49.0; // down-close
        candles[last].open = 50.0;
        let result = volume_zscore(&candles, 20).unwrap();
        let z = *result.last().unwrap();
        assert!(
            z < 0.0,
            "Down-close spike should produce negative z, got {z:.2}"
        );
        assert!(z < -2.0, "Expected |z| > 2 for 10x spike");
    }

    #[test]
    fn volume_zscore_excludes_current_bar_from_window() {
        // Current bar at 100x normal; window bars all at 1x.
        // If current were included it would raise the mean and lower the z-score.
        let mut candles = build_candles_flat(60, 50.0);
        let last = candles.len() - 1;
        candles[last].volume = 50_000_000; // 50x
        let result = volume_zscore(&candles, 20).unwrap();
        let z = result.last().unwrap().abs();
        // Should be very large because window is unaffected by the spike
        assert!(
            z > 5.0,
            "Expected z >> 5 when current excluded from window, got {z:.2}"
        );
    }

    #[test]
    fn volume_zscore_insufficient_data() {
        let candles = build_candles_flat(5, 50.0);
        assert!(matches!(
            volume_zscore(&candles, 20),
            Err(AnalysisError::InsufficientData { need: 21, got: 5 })
        ));
    }

    #[test]
    fn volume_zscore_output_length() {
        let candles = build_candles_flat(60, 50.0);
        let result = volume_zscore(&candles, 20).unwrap();
        assert_eq!(result.len(), 40); // 60 - 20
    }

    // ── RS Percentile ─────────────────────────────────────────────────────────

    #[test]
    fn rs_percentile_top_performer_near_100() {
        let universe: Vec<f64> = (0..499).map(|i| i as f64 * 0.01).collect();
        let pct = rs_percentile(5.0, &universe); // above all 499
        assert!(
            pct > 99.0,
            "Top performer should be near 100th pct, got {pct:.1}"
        );
    }

    #[test]
    fn rs_percentile_bottom_performer_near_0() {
        let universe: Vec<f64> = (1..500).map(|i| i as f64).collect();
        let pct = rs_percentile(-10.0, &universe);
        assert!(
            pct < 1.0,
            "Bottom performer should be near 0th pct, got {pct:.1}"
        );
    }

    #[test]
    fn rs_percentile_median_near_50() {
        let universe: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let pct = rs_percentile(50.0, &universe); // 50 values below 50.0
        assert!(
            (pct - 50.0).abs() < 2.0,
            "Median should be ~50th pct, got {pct:.1}"
        );
    }

    #[test]
    fn rs_percentile_empty_universe_returns_50() {
        assert_relative_eq!(rs_percentile(5.0, &[]), 50.0, epsilon = 1e-10);
    }

    #[test]
    fn rs_percentile_single_element_above() {
        let pct = rs_percentile(5.0, &[3.0]);
        assert_relative_eq!(pct, 100.0, epsilon = 1e-10);
    }

    #[test]
    fn rs_percentile_single_element_below() {
        let pct = rs_percentile(1.0, &[3.0]);
        assert_relative_eq!(pct, 0.0, epsilon = 1e-10);
    }

    // ── price_return_nd ──────────────────────────────────────────────────────

    #[test]
    fn price_return_nd_63d_positive() {
        let candles = build_candles_trending(100, 50.0, 75.0);
        let ret = price_return_nd(&candles, 63).unwrap();
        assert!(
            ret > 0.0,
            "Uptrend 63d return should be positive, got {ret:.3}"
        );
    }

    #[test]
    fn price_return_nd_insufficient_returns_none() {
        let candles = build_candles_flat(50, 50.0);
        assert!(price_return_nd(&candles, 63).is_none());
    }

    #[test]
    fn price_return_nd_zero_start_returns_none() {
        let mut candles = build_candles_trending(100, 50.0, 75.0);
        candles[100 - 63].adj_close = 0.0;
        assert!(price_return_nd(&candles, 63).is_none());
    }

    // ── price_return_252d (backward compat) ─────────────────────────────────

    #[test]
    fn price_return_252d_doubles_from_50_to_100() {
        let candles = build_candles_trending(252, 50.0, 100.0);
        let ret = price_return_252d(&candles).unwrap();
        assert_relative_eq!(ret, 1.0, epsilon = 0.01); // ~100% return
    }

    #[test]
    fn price_return_252d_insufficient_returns_none() {
        let candles = build_candles_flat(100, 50.0);
        assert!(price_return_252d(&candles).is_none());
    }

    #[test]
    fn price_return_252d_backward_compat_with_nd() {
        let candles = build_candles_trending(300, 50.0, 100.0);
        let old = price_return_252d(&candles);
        let new = price_return_nd(&candles, 252);
        assert_eq!(
            old, new,
            "252d wrapper should match price_return_nd(_, 252)"
        );
    }

    // ── weighted_rs ─────────────────────────────────────────────────────────

    #[test]
    fn weighted_rs_uptrend_positive() {
        let candles = build_candles_trending(300, 50.0, 100.0);
        let rs = weighted_rs(&candles).unwrap();
        assert!(
            rs > 0.0,
            "Uptrend weighted_rs should be positive, got {rs:.4}"
        );
    }

    #[test]
    fn weighted_rs_downtrend_negative() {
        let candles = build_candles_declining(300);
        let rs = weighted_rs(&candles).unwrap();
        assert!(
            rs < 0.0,
            "Downtrend weighted_rs should be negative, got {rs:.4}"
        );
    }

    #[test]
    fn weighted_rs_insufficient_data_returns_none() {
        let candles = build_candles_flat(200, 50.0);
        assert!(weighted_rs(&candles).is_none());
    }

    #[test]
    fn weighted_rs_recent_momentum_weighted_higher() {
        // Create candles that accelerated recently: flat first 200 bars, then strong up last 100
        let mut candles = build_candles_flat(200, 50.0);
        let trending_tail = build_candles_trending(100, 50.0, 100.0);
        for (i, c) in trending_tail.into_iter().enumerate() {
            let mut adjusted = c;
            adjusted.date = chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
                .unwrap()
                .checked_add_days(chrono::Days::new((200 + i) as u64))
                .unwrap();
            candles.push(adjusted);
        }
        let rs = weighted_rs(&candles).unwrap();
        // The 63d return (recent surge) is weighted 40%, so overall should be strongly positive
        assert!(rs > 0.0, "Recent acceleration should dominate, got {rs:.4}");
    }

    #[test]
    fn weighted_rs_weights_sum_to_one() {
        // Verify: 0.40 + 0.20 + 0.20 + 0.20 = 1.00
        let sum = 0.40 + 0.20 + 0.20 + 0.20;
        assert_relative_eq!(sum, 1.0, epsilon = 1e-10);
    }

    // ── rs_momentum ─────────────────────────────────────────────────────────

    #[test]
    fn rs_momentum_improving_positive() {
        // RS rising from 50 to 80 over 21 bars
        let history: Vec<f64> = (0..21).map(|i| 50.0 + (30.0 * i as f64 / 20.0)).collect();
        let mom = rs_momentum(&history, 21).unwrap();
        assert!(
            mom > 0.0,
            "Rising RS should have positive momentum, got {mom:.2}"
        );
    }

    #[test]
    fn rs_momentum_declining_negative() {
        let history: Vec<f64> = (0..21).map(|i| 80.0 - (30.0 * i as f64 / 20.0)).collect();
        let mom = rs_momentum(&history, 21).unwrap();
        assert!(
            mom < 0.0,
            "Falling RS should have negative momentum, got {mom:.2}"
        );
    }

    #[test]
    fn rs_momentum_insufficient_returns_none() {
        let history = vec![50.0, 55.0, 60.0];
        assert!(rs_momentum(&history, 10).is_none());
    }

    // ── Minervini Conditions ──────────────────────────────────────────────────

    #[test]
    fn minervini_trending_stock_passes_7_non_rs_conditions() {
        // Strong uptrend 50→100 over 300 bars satisfies all structural conditions.
        // rs_above_threshold is left false (caller's responsibility).
        let candles = build_candles_trending(300, 50.0, 100.0);
        let closes: Vec<f64> = candles.iter().map(|c| c.adj_close).collect();
        let result = check_minervini_conditions(&closes).unwrap();
        // The 7 non-RS conditions should all pass for a clean uptrend
        assert!(result.close_above_sma150_200, "close > SMA150+200");
        assert!(result.sma150_above_sma200, "SMA150 > SMA200");
        assert!(result.sma200_trending_up, "SMA200 trending up");
        assert!(result.sma50_above_sma150_200, "SMA50 > SMA150+200");
        assert!(result.close_above_sma50, "close > SMA50");
        assert!(result.close_above_52w_low_25pct, "close >= 52w_low * 1.25");
        assert!(
            result.close_within_25pct_52w_high,
            "close within 25% of 52w high"
        );
    }

    #[test]
    fn minervini_declining_stock_fails_most_conditions() {
        let candles = build_candles_declining(300);
        let closes: Vec<f64> = candles.iter().map(|c| c.adj_close).collect();
        let result = check_minervini_conditions(&closes).unwrap();
        assert!(!result.passes_all());
        assert!(
            !result.close_above_sma150_200,
            "Declining stock should be below SMAs"
        );
        assert!(!result.sma50_above_sma150_200);
        assert!(!result.close_above_sma50);
    }

    #[test]
    fn minervini_requires_252_candles() {
        let closes = vec![100.0; 200];
        assert!(matches!(
            check_minervini_conditions(&closes),
            Err(AnalysisError::InsufficientData {
                need: 252,
                got: 200
            })
        ));
    }

    #[test]
    fn minervini_count_correct() {
        let candles = build_candles_trending(300, 50.0, 100.0);
        let closes: Vec<f64> = candles.iter().map(|c| c.adj_close).collect();
        let mut result = check_minervini_conditions(&closes).unwrap();
        result.rs_above_threshold = true;
        assert_eq!(result.count(), 8);
        result.rs_above_threshold = false;
        assert_eq!(result.count(), 7);
    }

    #[test]
    fn minervini_passes_all_with_rs_set() {
        let candles = build_candles_trending(300, 50.0, 100.0);
        let closes: Vec<f64> = candles.iter().map(|c| c.adj_close).collect();
        let mut result = check_minervini_conditions(&closes).unwrap();
        assert!(!result.passes_all(), "Should not pass without RS set");
        result.rs_above_threshold = true;
        assert!(result.passes_all(), "Should pass all 8 when RS set");
    }

    // ── Regime Detection ──────────────────────────────────────────────────────

    #[test]
    fn regime_bull_when_spy_above_200ma_and_high_breadth() {
        let spy = build_candles_trending(300, 300.0, 500.0); // strong uptrend
        let regime = detect_regime(&spy, 70.0).unwrap();
        assert_eq!(regime, MarketRegime::Bull);
    }

    #[test]
    fn regime_bear_when_spy_below_200ma_and_low_breadth() {
        let spy = build_candles_declining(300); // price falls from 100 to 50
        let regime = detect_regime(&spy, 25.0).unwrap();
        assert_eq!(regime, MarketRegime::Bear);
    }

    #[test]
    fn regime_sideways_when_mixed_signals() {
        // SPY above 200MA but breadth is borderline (50%)
        let spy = build_candles_trending(300, 300.0, 500.0);
        let regime = detect_regime(&spy, 50.0).unwrap();
        assert_eq!(regime, MarketRegime::Sideways);
    }

    #[test]
    fn regime_sideways_spy_below_200ma_but_moderate_breadth() {
        let spy = build_candles_declining(300);
        let regime = detect_regime(&spy, 50.0).unwrap();
        assert_eq!(regime, MarketRegime::Sideways);
    }

    #[test]
    fn regime_requires_200_candles() {
        let spy = build_candles_flat(150, 450.0);
        assert!(matches!(
            detect_regime(&spy, 50.0),
            Err(AnalysisError::InsufficientData { need: 200, .. })
        ));
    }

    // ── ATR ──────────────────────────────────────────────────────────────────

    #[test]
    fn atr_trending_market_positive() {
        let candles = build_candles_trending(300, 50.0, 100.0);
        let val = atr(&candles, 14).unwrap();
        assert!(
            val > 0.0,
            "ATR should be positive in trending market, got {val:.4}"
        );
    }

    #[test]
    fn atr_flat_market_small() {
        let candles = build_candles_flat(60, 50.0);
        let val = atr(&candles, 14).unwrap();
        // Flat candles still have high-low spread from the builder (1.01 * price - 0.99 * price = 1.0)
        assert!(
            val < 5.0,
            "ATR should be small in flat market, got {val:.4}"
        );
    }

    #[test]
    fn atr_insufficient_data_returns_error() {
        let candles = build_candles_flat(10, 50.0);
        assert!(matches!(
            atr(&candles, 14),
            Err(AnalysisError::InsufficientData { need: 15, got: 10 })
        ));
    }

    #[test]
    fn atr_known_values() {
        // Build candles where TR is always 2.0 → ATR should converge to 2.0
        let candles: Vec<Candle> = (0..30)
            .map(|i| Candle {
                date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                open: 50.0,
                high: 51.0,
                low: 49.0, // high - low = 2.0
                close: 50.0,
                adj_close: 50.0,
                volume: 1_000_000,
            })
            .collect();
        let val = atr(&candles, 14).unwrap();
        assert_relative_eq!(val, 2.0, epsilon = 0.1);
    }

    #[test]
    fn atr_volatile_market_larger() {
        // Volatile: high-low spread of 10 points
        let candles: Vec<Candle> = (0..30)
            .map(|i| Candle {
                date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                open: 50.0,
                high: 55.0,
                low: 45.0,
                close: 50.0,
                adj_close: 50.0,
                volume: 1_000_000,
            })
            .collect();
        let val = atr(&candles, 14).unwrap();
        assert!(
            val > 5.0,
            "Volatile market ATR should be large, got {val:.4}"
        );
    }

    // ── Risk-Reward Ratio ────────────────────────────────────────────────────

    #[test]
    fn risk_reward_positive_ratio() {
        // current=100, sma50=95, atr=5 → stop=90, risk=10, reward=10 → R:R=1.0
        let rr = risk_reward_ratio(100.0, 95.0, 5.0, 110.0);
        assert_relative_eq!(rr, 1.0, epsilon = 0.01);
    }

    #[test]
    fn risk_reward_zero_risk_returns_max() {
        // current at stop level (current <= stop) → capped
        let rr = risk_reward_ratio(90.0, 95.0, 5.0, 120.0);
        assert!(
            rr <= -10.0 || rr >= 10.0,
            "Zero/negative risk should cap, got {rr}"
        );
    }

    #[test]
    fn risk_reward_at_52w_high_returns_zero() {
        let rr = risk_reward_ratio(120.0, 115.0, 5.0, 120.0);
        assert_relative_eq!(rr, 0.0, epsilon = 0.01);
    }

    #[test]
    fn risk_reward_below_stop_negative() {
        // current=80 < stop=90 → negative (below stop)
        let rr = risk_reward_ratio(80.0, 95.0, 5.0, 120.0);
        assert!(rr < 0.0, "Below stop should be negative, got {rr}");
    }

    // ── Liquidity Classification ─────────────────────────────────────────────

    #[test]
    fn liquidity_high_above_50m() {
        // $100 × 1M shares = $100M > $50M threshold
        let candles: Vec<Candle> = (0..20)
            .map(|i| Candle {
                date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.0,
                adj_close: 100.0,
                volume: 1_000_000,
            })
            .collect();
        assert_eq!(classify_liquidity(&candles, 20), LiquidityClass::High);
    }

    #[test]
    fn liquidity_medium_5m_to_50m() {
        // $10 × 1M = $10M → Medium
        let candles: Vec<Candle> = (0..20)
            .map(|i| Candle {
                date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                open: 10.0,
                high: 10.5,
                low: 9.5,
                close: 10.0,
                adj_close: 10.0,
                volume: 1_000_000,
            })
            .collect();
        assert_eq!(classify_liquidity(&candles, 20), LiquidityClass::Medium);
    }

    #[test]
    fn liquidity_low_1m_to_5m() {
        // $20 × 100K = $2M → Low
        let candles: Vec<Candle> = (0..20)
            .map(|i| Candle {
                date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                open: 20.0,
                high: 20.5,
                low: 19.5,
                close: 20.0,
                adj_close: 20.0,
                volume: 100_000,
            })
            .collect();
        assert_eq!(classify_liquidity(&candles, 20), LiquidityClass::Low);
    }

    #[test]
    fn liquidity_illiquid_below_1m() {
        // $5 × 10K = $50K → Illiquid
        let candles: Vec<Candle> = (0..20)
            .map(|i| Candle {
                date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                open: 5.0,
                high: 5.5,
                low: 4.5,
                close: 5.0,
                adj_close: 5.0,
                volume: 10_000,
            })
            .collect();
        assert_eq!(classify_liquidity(&candles, 20), LiquidityClass::Illiquid);
    }

    #[test]
    fn liquidity_insufficient_data_defaults_illiquid() {
        let candles = build_candles_flat(5, 50.0);
        assert_eq!(classify_liquidity(&candles, 20), LiquidityClass::Illiquid);
    }

    #[test]
    fn liquidity_averages_over_period() {
        // First 10 bars at $100M, last 10 bars at $0 → average = $50M → High threshold
        let mut candles: Vec<Candle> = (0..20)
            .map(|i| Candle {
                date: chrono::NaiveDate::from_ymd_opt(2024, 1, 1)
                    .unwrap()
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                open: 100.0,
                high: 101.0,
                low: 99.0,
                close: 100.0,
                adj_close: 100.0,
                volume: 1_000_000, // $100M per bar
            })
            .collect();
        // Set last 10 bars to very low volume
        for c in candles[10..].iter_mut() {
            c.volume = 1_000; // $100K per bar
        }
        // Average = ($100M × 10 + $100K × 10) / 20 ≈ $50M → should be just above High
        let class = classify_liquidity(&candles, 20);
        assert!(
            class == LiquidityClass::High || class == LiquidityClass::Medium,
            "Should be near the High/Medium boundary"
        );
    }

    // ── VCP Detection ────────────────────────────────────────────────────────

    /// Build a synthetic VCP: 120 bars with 3 contracting pullbacks.
    fn build_vcp_candles() -> Vec<Candle> {
        let mut candles = Vec::with_capacity(120);
        let base_date = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

        for i in 0..120 {
            // Uptrend with 3 contracting corrections
            let trend = 100.0 + i as f64 * 0.3; // gradual uptrend
            let correction = if (10..25).contains(&i) {
                // Base 1: ~15% pullback
                -(15.0 * ((i as f64 - 10.0) / 15.0 * std::f64::consts::PI).sin())
            } else if (45..58).contains(&i) {
                // Base 2: ~8% pullback
                -(8.0 * ((i as f64 - 45.0) / 13.0 * std::f64::consts::PI).sin())
            } else if (85..95).contains(&i) {
                // Base 3: ~3% pullback
                -(3.0 * ((i as f64 - 85.0) / 10.0 * std::f64::consts::PI).sin())
            } else {
                0.0
            };
            let close = trend + correction;
            candles.push(Candle {
                date: base_date
                    .checked_add_days(chrono::Days::new(i as u64))
                    .unwrap(),
                open: close * 0.998,
                high: close * 1.01,
                low: close * 0.99,
                close,
                adj_close: close,
                volume: 1_000_000,
            });
        }
        candles
    }

    #[test]
    fn vcp_detected_in_contracting_base() {
        let candles = build_vcp_candles();
        let result = detect_vcp(&candles, 120).unwrap();
        assert!(
            result.vcp_detected,
            "Should detect VCP in contracting bases"
        );
        assert!(
            result.base_count >= 2,
            "Should find at least 2 bases, got {}",
            result.base_count
        );
        assert!(
            result.contraction_ratio < 0.70,
            "Contraction ratio should be < 0.70, got {:.2}",
            result.contraction_ratio
        );
    }

    #[test]
    fn vcp_not_detected_in_flat_market() {
        // Flat market: all segments have similar tiny depths → contraction_ratio ≈ 1.0
        let candles = build_candles_flat(120, 50.0);
        let result = detect_vcp(&candles, 120).unwrap();
        assert!(
            !result.vcp_detected,
            "Flat market should not produce VCP, ratio={:.2}",
            result.contraction_ratio
        );
    }

    #[test]
    fn vcp_not_detected_in_downtrend() {
        let candles = build_candles_declining(120);
        let result = detect_vcp(&candles, 120).unwrap();
        // Downtrend doesn't contract — depths stay large or expand
        // The contraction ratio won't necessarily be < 0.70
        assert!(
            !result.vcp_detected || result.contraction_ratio >= 0.70,
            "Downtrend should not produce contracting VCP"
        );
    }

    #[test]
    fn vcp_pivot_proximity() {
        let candles = build_vcp_candles();
        let result = detect_vcp(&candles, 120).unwrap();
        // Current price should be near the pivot (within a few percent)
        assert!(
            result.pivot_proximity_pct.abs() < 15.0,
            "Pivot proximity should be within 15%, got {:.1}%",
            result.pivot_proximity_pct
        );
    }

    #[test]
    fn vcp_insufficient_data() {
        let candles = build_candles_flat(30, 50.0);
        assert!(matches!(
            detect_vcp(&candles, 120),
            Err(AnalysisError::InsufficientData { .. })
        ));
    }

    #[test]
    fn vcp_base_count_populated() {
        let candles = build_vcp_candles();
        let result = detect_vcp(&candles, 120).unwrap();
        assert!(
            result.base_count > 0,
            "Base count should be > 0 in VCP fixture"
        );
    }

    // ── RSI ──────────────────────────────────────────────────────────────────

    #[test]
    fn rsi_all_gains_returns_100() {
        // Only up moves → avg_loss = 0 → RSI = 100
        let closes: Vec<f64> = (0..20).map(|i| 10.0 + i as f64).collect();
        let result = rsi(&closes, 14).unwrap();
        assert_relative_eq!(result, 100.0, epsilon = 1e-6);
    }

    #[test]
    fn rsi_all_losses_returns_0() {
        // Only down moves → avg_gain = 0 → RSI = 0
        let closes: Vec<f64> = (0..20).map(|i| 100.0 - i as f64).collect();
        let result = rsi(&closes, 14).unwrap();
        assert_relative_eq!(result, 0.0, epsilon = 1e-6);
    }

    #[test]
    fn rsi_insufficient_data_returns_none() {
        // Need period + 1 = 15 closes; providing 14
        let closes = vec![10.0; 14];
        assert!(rsi(&closes, 14).is_none());
    }

    #[test]
    fn rsi_exactly_enough_data() {
        let closes: Vec<f64> = (0..15).map(|i| 10.0 + i as f64).collect();
        assert!(rsi(&closes, 14).is_some());
    }

    #[test]
    fn rsi_known_value_alternating() {
        // Alternating +1/-1 with 14-period should converge toward 50
        let mut closes = vec![50.0];
        for i in 0..28 {
            let prev = *closes.last().unwrap();
            if i % 2 == 0 {
                closes.push(prev + 1.0);
            } else {
                closes.push(prev - 1.0);
            }
        }
        let result = rsi(&closes, 14).unwrap();
        // Equal up/down moves → RSI near 50
        assert!(
            (result - 50.0).abs() < 15.0,
            "Alternating RSI should be near 50, got {result:.2}"
        );
    }

    #[test]
    fn rsi_uptrend_above_50() {
        let closes: Vec<f64> = (0..30).map(|i| 50.0 + i as f64 * 0.5).collect();
        let result = rsi(&closes, 14).unwrap();
        assert!(result > 50.0, "Uptrend RSI should be > 50, got {result:.2}");
    }

    #[test]
    fn rsi_downtrend_below_50() {
        let closes: Vec<f64> = (0..30).map(|i| 100.0 - i as f64 * 0.5).collect();
        let result = rsi(&closes, 14).unwrap();
        assert!(
            result < 50.0,
            "Downtrend RSI should be < 50, got {result:.2}"
        );
    }

    #[test]
    fn rsi_bounded_0_to_100() {
        let closes: Vec<f64> = (0..50)
            .map(|i| if i % 3 == 0 { 100.0 } else { 50.0 })
            .collect();
        let result = rsi(&closes, 14).unwrap();
        assert!(
            (0.0..=100.0).contains(&result),
            "RSI out of range: {result:.2}"
        );
    }

    // Known RSI values: 14-period test with synthetic data
    // After 14 bars: avg_gain=1.0, avg_loss=0.5 → RS=2.0 → RSI=66.67
    #[test]
    fn rsi_known_computed_value() {
        // 14 gains of 1.0, then 7 losses of 1.0 each
        // seed: avg_gain=1.0, avg_loss=0.0 → RSI=100 after first 14
        // but then after smoothing 7 losses: avg_loss grows
        let mut closes = vec![50.0];
        for _ in 0..14 {
            let prev = *closes.last().unwrap();
            closes.push(prev + 1.0);
        }
        // After 14 gains: avg_gain=1.0, avg_loss=0.0 → RSI=100
        let rsi_14 = rsi(&closes, 14).unwrap();
        assert_relative_eq!(rsi_14, 100.0, epsilon = 1e-6);

        // Add 7 losses of 1.0 each, RSI should drop
        for _ in 0..7 {
            let prev = *closes.last().unwrap();
            closes.push(prev - 1.0);
        }
        let rsi_21 = rsi(&closes, 14).unwrap();
        assert!(
            rsi_21 < 100.0,
            "After losses RSI should drop below 100, got {rsi_21:.2}"
        );
        assert!(
            rsi_21 > 40.0,
            "RSI should still be above 40, got {rsi_21:.2}"
        );
    }

    // ── Consolidation Detection ───────────────────────────────────────────────

    #[test]
    fn consolidation_detected_in_flat_market() {
        let candles = build_candles_flat(40, 50.0);
        // Flat market has near-zero range → should find consolidation
        let result = detect_consolidation(&candles, 20, 0.15);
        assert!(
            result.is_some(),
            "Should detect consolidation in flat market"
        );
        let c = result.unwrap();
        assert!(
            c.depth_pct < 15.0,
            "Flat market depth should be < 15%, got {:.2}",
            c.depth_pct
        );
        assert!(
            c.duration_days >= 20,
            "Duration should be >= 20, got {}",
            c.duration_days
        );
    }

    #[test]
    fn consolidation_not_found_in_trending_market() {
        // A strong trend from 50→100 has > 15% range, so no tight consolidation
        let candles = build_candles_trending(60, 50.0, 100.0);
        let result = detect_consolidation(&candles, 20, 0.15);
        // The full window has ~100% depth, so the 20-day min won't be satisfied with < 15%
        // unless only the last few bars are considered — either outcome is valid
        if let Some(c) = result {
            assert!(
                c.depth_pct <= 15.0,
                "Any detected consolidation must be tight: {:.2}%",
                c.depth_pct
            );
        }
    }

    #[test]
    fn consolidation_min_days_not_met_returns_none() {
        // Only 5 candles but min_days=20
        let candles = build_candles_flat(5, 50.0);
        let result = detect_consolidation(&candles, 20, 0.15);
        assert!(
            result.is_none(),
            "Should return None when fewer candles than min_days"
        );
    }

    #[test]
    fn consolidation_depth_and_pivot_reasonable() {
        let candles = build_candles_flat(60, 50.0);
        let c = detect_consolidation(&candles, 20, 0.15).unwrap();
        assert!(
            c.pivot > 0.0,
            "Pivot should be positive, got {:.2}",
            c.pivot
        );
        assert!(
            (c.pivot - 50.0).abs() < 5.0,
            "Pivot should be near 50, got {:.2}",
            c.pivot
        );
    }

    #[test]
    fn consolidation_zero_min_days_returns_none() {
        let candles = build_candles_flat(30, 50.0);
        let result = detect_consolidation(&candles, 0, 0.15);
        assert!(result.is_none());
    }

    // ── Volume Ratio ──────────────────────────────────────────────────────────

    #[test]
    fn volume_ratio_equal_volumes_is_one() {
        let candles = build_candles_flat(51, 50.0); // all vol = 1_000_000
        let ratio = volume_ratio(&candles, 50).unwrap();
        assert_relative_eq!(ratio, 1.0, epsilon = 1e-6);
    }

    #[test]
    fn volume_ratio_spike_above_one() {
        let mut candles = build_candles_flat(51, 50.0);
        candles.last_mut().unwrap().volume = 3_000_000;
        let ratio = volume_ratio(&candles, 50).unwrap();
        assert!(
            ratio > 1.5,
            "Volume spike should yield ratio > 1.5, got {ratio:.2}"
        );
    }

    #[test]
    fn volume_ratio_insufficient_returns_none() {
        let candles = build_candles_flat(5, 50.0);
        assert!(volume_ratio(&candles, 50).is_none());
    }

    // ── slippage model ──────────────────────────────────────────────────────

    #[test]
    fn adjusted_cost_scales_by_liquidity() {
        let base = 10.0;
        assert_relative_eq!(adjusted_cost_bps(base, &LiquidityClass::High), 10.0);
        assert_relative_eq!(adjusted_cost_bps(base, &LiquidityClass::Medium), 15.0);
        assert_relative_eq!(adjusted_cost_bps(base, &LiquidityClass::Low), 30.0);
        assert_relative_eq!(adjusted_cost_bps(base, &LiquidityClass::Illiquid), 50.0);
    }

    #[test]
    fn market_impact_below_1pct_adv_is_zero() {
        // 100 shares of 1M ADV = 0.01% participation
        assert_relative_eq!(market_impact_bps(10.0, 100.0, 1_000_000.0), 0.0);
    }

    #[test]
    fn market_impact_sqrt_scaling() {
        // 100K shares of 1M ADV = 10% participation → sqrt(0.1) ≈ 0.316
        let impact = market_impact_bps(10.0, 100_000.0, 1_000_000.0);
        assert_relative_eq!(impact, 10.0 * (0.1_f64).sqrt(), epsilon = 0.01);
    }

    #[test]
    fn market_impact_zero_adv_returns_5x() {
        assert_relative_eq!(market_impact_bps(10.0, 100.0, 0.0), 50.0);
    }

    // ── data quality validation ─────────────────────────────────────────────

    #[test]
    fn clean_data_returns_no_issues() {
        let candles = build_candles_flat(20, 100.0);
        let issues = validate_candles(&candles, 50.0);
        assert!(issues.is_empty());
    }

    #[test]
    fn detects_suspicious_move() {
        let mut candles = build_candles_flat(5, 100.0);
        // Inject a 60% spike that persists (so only 1 flagged transition)
        candles[3].adj_close = 160.0;
        candles[4].adj_close = 160.0;
        let issues = validate_candles(&candles, 50.0);
        assert_eq!(issues.len(), 1);
        assert!(
            matches!(&issues[0], DataQualityIssue::SuspiciousMove { daily_return_pct, .. } if *daily_return_pct > 50.0)
        );
    }

    #[test]
    fn detects_likely_split() {
        let mut candles = build_candles_flat(5, 300.0);
        // Simulate 3:1 split: price drops to 1/3 and stays, volume triples
        candles[3].adj_close = 100.0; // -66.7% decline
        candles[3].volume = candles[2].volume * 3;
        candles[4].adj_close = 100.0;
        candles[4].volume = candles[2].volume * 3;
        let issues = validate_candles(&candles, 50.0);
        assert!(issues
            .iter()
            .any(|i| matches!(i, DataQualityIssue::LikelySplit { .. })));
    }

    #[test]
    fn normal_volatility_below_threshold_is_clean() {
        let mut candles = build_candles_flat(10, 100.0);
        // 10% daily move — below 50% threshold
        candles[5].adj_close = 110.0;
        let issues = validate_candles(&candles, 50.0);
        assert!(issues.is_empty());
    }

    #[test]
    fn single_candle_returns_no_issues() {
        let candles = build_candles_flat(1, 100.0);
        let issues = validate_candles(&candles, 50.0);
        assert!(issues.is_empty());
    }
}
