# Plan 06: Narrative Alert Mode

**Effort:** Small (1-2 days)
**Impact:** Medium
**Priority:** 6

## Problem

Current alerts are data-dense tables optimized for technical users. A non-technical investor receiving a Telegram/Discord alert sees numbers but doesn't immediately know what to do. The current format:

```
🔴 CRITICAL NewSignal — NVDA
Score: 94.2 | RS: 97 | Vol-Z: 2.8 | Minervini: 8/8
```

This requires knowing what RS percentile, Z-scores, and Minervini conditions mean.

## Solution

Add a "narrative" alert format that translates data into plain-English actionable summaries. Configurable per-channel or globally.

## Example output

### Current format (data mode — default, unchanged):
```
🔴 CRITICAL NewSignal — NVDA
Score: 94.2 | Grade: A+ | Minervini: 8/8
RS: 97th | Vol-Z: 2.8 | Regime: Bull
```

### New format (narrative mode):
```
🔴 NVDA — Strong Buy Signal

NVDA is showing one of the strongest setups in the market right now.
It passes all 8 Minervini trend conditions, ranks in the top 3% for
relative strength, and just saw a significant volume surge (2.8x
normal). Market regime is bullish.

Consider entry near $142 with a stop at $131 based on ATR.
Risk/reward ratio: 3.2:1.

Score: 94.2/100 (A+) | Ghost Alpha
```

## Implementation

### 1. Configuration

Add to `AlertsConfig` in `src/config.rs`:

```rust
/// Alert format: "data" (default) or "narrative"
#[serde(default = "AlertsConfig::default_format")]
pub alert_format: AlertFormat,
```

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertFormat {
    Data,
    Narrative,
}
```

TOML:
```toml
[alerts]
alert_format = "narrative"
```

### 2. Narrative generator

Create `src/alerts/narrative.rs`:

```rust
pub fn generate_narrative(alert: &OutboundAlert, context: &ScreenResult) -> String {
    let mut parts = Vec::new();

    // Opening — what's happening
    parts.push(format!(
        "{} is showing {} in the market right now.",
        context.ticker,
        strength_description(alert.composite_score),
    ));

    // Minervini context
    parts.push(minervini_sentence(context));

    // RS context
    parts.push(rs_sentence(context));

    // Volume context
    parts.push(volume_sentence(context));

    // Regime context
    parts.push(regime_sentence(context));

    // Action line (if ATR/R:R available)
    if let Some(action) = action_sentence(context) {
        parts.push(action);
    }

    parts.join(" ")
}
```

Helper functions produce human-readable sentences:

```rust
fn strength_description(score: f64) -> &'static str {
    match score as u32 {
        90..=100 => "one of the strongest setups",
        80..=89  => "a very strong setup",
        70..=79  => "a solid setup",
        _        => "a notable setup",
    }
}

fn rs_sentence(ctx: &ScreenResult) -> String {
    let pct = ctx.rs_percentile;
    if pct >= 90.0 {
        format!("It ranks in the top {}% for relative strength.", (100.0 - pct) as u32)
    } else if pct >= 70.0 {
        format!("Relative strength is above average at the {}th percentile.", pct as u32)
    } else {
        format!("Relative strength is moderate at the {}th percentile.", pct as u32)
    }
}

fn volume_sentence(ctx: &ScreenResult) -> String {
    let z = ctx.volume_zscore;
    if z >= 2.0 {
        format!("Volume surged to {:.1}x normal levels — institutional interest likely.", z)
    } else if z >= 1.0 {
        format!("Volume is elevated at {:.1}x normal.", z)
    } else {
        "Volume is near average.".to_string()
    }
}
```

### 3. Channel integration

In `OutboundAlert`, add an optional `narrative` field:

```rust
pub struct OutboundAlert {
    // ... existing fields ...
    pub narrative: Option<String>,
}
```

Each channel's `send()` uses the narrative when `alert_format == Narrative`:
- **Telegram:** Use narrative as message body (plain text, no MarkdownV2 escaping issues)
- **Discord:** Use narrative as embed `description`
- **Webhook:** Include both `message` (data) and `narrative` fields in JSON

### 4. Exit signal narratives

```
⚠️ NVDA — Exit Signal: Weakening Trend

NVDA has lost 3 of the 8 Minervini conditions it held at entry.
The 50-day moving average is now approaching the 200-day from above
(potential death cross). Relative strength dropped from the 97th to
62nd percentile.

Consider reviewing your position. Original entry signal was on 2026-01-15
at a score of 94.2.
```

## Files to create/modify

| File | Action |
|------|--------|
| `src/alerts/narrative.rs` | Create — narrative text generation |
| `src/alerts/mod.rs` | Modify — add `pub mod narrative` |
| `src/alerts/channel.rs` | Modify — add `narrative` field to `OutboundAlert` |
| `src/alerts/engine.rs` | Modify — generate narrative when format is set |
| `src/config.rs` | Modify — add `AlertFormat` enum and config field |
| `src/alerts/telegram.rs` | Modify — use narrative in message body |
| `src/alerts/discord.rs` | Modify — use narrative in embed description |

## Testing

- Unit test: `generate_narrative` produces non-empty string for all severity levels
- Unit test: Strength descriptions map correctly to score ranges
- Unit test: Volume/RS/regime sentences handle edge cases (0 Z-score, 50th percentile)
- Snapshot test: Golden narrative output for a sample alert

## Acceptance criteria

- [ ] `alert_format = "narrative"` in config enables plain-English alerts
- [ ] Narratives cover: strength, Minervini status, RS, volume, regime, action
- [ ] Exit signals get their own narrative template
- [ ] Default remains "data" mode (no breaking change)
- [ ] Narratives work on Telegram, Discord, and webhook channels
- [ ] Score/grade still appears at the bottom for reference
