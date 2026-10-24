# Plan 05: JSON/CSV Export

**Effort:** Small (1-2 days)
**Impact:** Medium
**Priority:** 5

## Problem

All output is currently formatted for terminal display. Users who want to pipe results into Excel, Python notebooks, Grafana, or automation tools (n8n, Zapier) have no structured output option.

## Solution

Add `--format json` and `--format csv` to all commands that produce tabular output: `screen`, `watch list`, `watch screen`, `alerts`, `regime`, `backtest`, `plugins`.

## Implementation

### 1. Unified OutputFormat enum

Already partially exists. Ensure it covers all formats:

```rust
#[derive(Clone, ValueEnum)]
pub enum OutputFormat {
    Table,   // Current default — colored terminal table
    Json,    // Structured JSON (single object or array)
    Csv,     // RFC 4180 CSV with header row
    Jsonl,   // JSON Lines — one JSON object per line (streaming-friendly)
}
```

### 2. Output trait

Create `src/output.rs`:

```rust
pub trait Renderable: Serialize {
    fn table_headers() -> Vec<&'static str>;
    fn table_row(&self) -> Vec<String>;
}

pub fn render<T: Renderable>(items: &[T], format: &OutputFormat) {
    match format {
        OutputFormat::Table => render_table(items),
        OutputFormat::Json  => println!("{}", serde_json::to_string_pretty(items).unwrap()),
        OutputFormat::Csv   => render_csv(items),
        OutputFormat::Jsonl => {
            for item in items {
                println!("{}", serde_json::to_string(item).unwrap());
            }
        }
    }
}
```

### 3. Add `Serialize` derives

Add `#[derive(Serialize)]` to output structs that don't have it yet:
- `ScreenResult` (likely already has it)
- `WatchlistEntry` (new from Plan 04)
- `AlertLogEntry`
- `RegimeState`
- `BacktestKpis`

### 4. CSV implementation

Use the `csv` crate (already common in Rust):

```toml
[dependencies]
csv = "1.3"
```

```rust
fn render_csv<T: Renderable + Serialize>(items: &[T]) {
    let mut wtr = csv::Writer::from_writer(std::io::stdout());
    for item in items {
        wtr.serialize(item).unwrap();
    }
    wtr.flush().unwrap();
}
```

### 5. Per-command integration

For each command, replace direct `println!` table formatting with:

```rust
// Before:
print_screen_table(&results);

// After:
render(&results, &args.format);
```

### 6. Example outputs

**JSON:**
```json
[
  {
    "ticker": "NVDA",
    "score": 94.2,
    "grade": "A+",
    "minervini_pass": true,
    "conditions_met": 8,
    "rs_percentile": 97.0,
    "volume_zscore": 2.8,
    "regime": "Bull"
  }
]
```

**CSV:**
```csv
ticker,score,grade,minervini_pass,conditions_met,rs_percentile,volume_zscore,regime
NVDA,94.2,A+,true,8,97.0,2.8,Bull
AAPL,71.2,B+,true,6,72.0,0.5,Bull
```

**JSONL (pipe-friendly):**
```
{"ticker":"NVDA","score":94.2,"grade":"A+","minervini_pass":true}
{"ticker":"AAPL","score":71.2,"grade":"B+","minervini_pass":true}
```

## Files to create/modify

| File | Action |
|------|--------|
| `src/output.rs` | Create — rendering logic |
| `src/lib.rs` | Modify — add `pub mod output` |
| `src/main.rs` | Modify — use `render()` in each subcommand |
| `src/data/models.rs` | Modify — add `Serialize` to output structs |
| `Cargo.toml` | Modify — add `csv = "1.3"` |

## Downstream use cases enabled

| Use case | Command |
|----------|---------|
| Excel import | `ghost-alpha screen --format csv > results.csv` |
| Python pandas | `ghost-alpha screen --format json \| python analyze.py` |
| Grafana/dashboard | `ghost-alpha screen --format jsonl` via file tail |
| n8n/Zapier webhook | `ghost-alpha screen --format json \| curl -X POST ...` |
| jq filtering | `ghost-alpha screen --format json \| jq '.[] \| select(.score > 80)'` |
| Daily cron + append | `ghost-alpha screen --format csv >> history.csv` |

## Testing

- Unit test: JSON output is valid JSON, parseable back into structs
- Unit test: CSV output has correct headers and row count
- Unit test: JSONL output has one valid JSON object per line
- Unit test: Table output unchanged (regression)

## Acceptance criteria

- [ ] `--format json`, `--format csv`, `--format jsonl` work on `screen`, `alerts`, `regime`, `backtest`
- [ ] JSON output is parseable by `jq`
- [ ] CSV output opens correctly in Excel
- [ ] Default format remains `table` (no breaking change)
- [ ] Empty results produce valid empty structures (`[]`, empty CSV with headers)
