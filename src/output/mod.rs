//! Unified output rendering for all tabular commands.
//!
//! Submodules:
//! - [`mermaid`] — Mermaid markdown emitter helpers (quote-if-needed rule).
//!
//! # Usage
//!
//! 1. Implement [`Renderable`] for the struct you want to display.
//! 2. Call [`render`] (writes to stdout) or [`render_to_writer`] (any `Write` sink — useful in tests).

pub mod mermaid;
pub mod plan_10b5_1_md;

use std::io::Write;

use clap::ValueEnum;
use serde::Serialize;

// ─── OutputFormat ─────────────────────────────────────────────────────────────

/// Selectable output format for every command that produces tabular data.
#[derive(Debug, Clone, PartialEq, Eq, ValueEnum, Default)]
pub enum OutputFormat {
    /// Coloured terminal table (default)
    #[default]
    Table,
    /// Pretty-printed JSON array
    Json,
    /// RFC 4180 CSV with a header row derived from serde field names
    Csv,
    /// JSON Lines — one compact JSON object per line (streaming-friendly)
    Jsonl,
    /// Self-contained HTML report (backtest command only)
    Html,
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OutputFormat::Table => write!(f, "table"),
            OutputFormat::Json => write!(f, "json"),
            OutputFormat::Csv => write!(f, "csv"),
            OutputFormat::Jsonl => write!(f, "jsonl"),
            OutputFormat::Html => write!(f, "html"),
        }
    }
}

// ─── Renderable trait ─────────────────────────────────────────────────────────

/// Implemented by any struct that can be rendered in multiple formats.
///
/// `Serialize` supplies the field names and values used for JSON/CSV.
/// The two extra methods supply the human-readable table representation.
pub trait Renderable: Serialize {
    /// Column headers for the table view, in display order.
    fn table_headers() -> Vec<&'static str>;

    /// One row of string values matching the order of [`table_headers`].
    fn table_row(&self) -> Vec<String>;
}

// ─── Public render entry points ───────────────────────────────────────────────

/// Render `items` in `format` to **stdout**.
///
/// For [`OutputFormat::Table`] the implementation delegates back to the caller
/// via the [`Renderable`] trait so each command can style the table however it
/// likes (colours, column widths, etc.).  The other three formats are handled
/// generically here.
pub fn render<T: Renderable>(items: &[T], format: &OutputFormat) {
    let stdout = std::io::stdout();
    let mut handle = stdout.lock();
    render_to_writer(items, format, &mut handle).expect("write to stdout failed");
}

/// Same as [`render`] but writes to an arbitrary [`Write`] sink.
///
/// This is the function to use in unit tests so output goes to a buffer rather
/// than stdout.
pub fn render_to_writer<T: Renderable, W: Write>(
    items: &[T],
    format: &OutputFormat,
    writer: &mut W,
) -> std::io::Result<()> {
    match format {
        OutputFormat::Table => render_table_to_writer(items, writer),
        OutputFormat::Json => render_json_to_writer(items, writer),
        OutputFormat::Csv => render_csv_to_writer(items, writer),
        OutputFormat::Jsonl => render_jsonl_to_writer(items, writer),
        // Html is handled directly by the backtest command; fall back to JSON
        // if this renderer is called with Html for any other command.
        OutputFormat::Html => render_json_to_writer(items, writer),
    }
}

// ─── Format implementations ──────────────────────────────────────────────────

fn render_table_to_writer<T: Renderable, W: Write>(
    items: &[T],
    writer: &mut W,
) -> std::io::Result<()> {
    if items.is_empty() {
        return writeln!(writer, "(no results)");
    }

    let headers = T::table_headers();

    // Compute column widths: max(header_len, max_cell_len)
    let rows: Vec<Vec<String>> = items.iter().map(|i| i.table_row()).collect();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if let Some(w) = widths.get_mut(i) {
                *w = (*w).max(cell.len());
            }
        }
    }

    // Header
    writeln!(writer)?;
    write!(writer, "  ")?;
    for (i, h) in headers.iter().enumerate() {
        write!(writer, "{:<width$}  ", h, width = widths[i])?;
    }
    writeln!(writer)?;
    let separator_len = widths.iter().sum::<usize>() + widths.len() * 2 + 2;
    writeln!(writer, "  {}", "─".repeat(separator_len.min(120)))?;

    // Rows
    for row in &rows {
        write!(writer, "  ")?;
        for (i, cell) in row.iter().enumerate() {
            write!(writer, "{:<width$}  ", cell, width = widths[i])?;
        }
        writeln!(writer)?;
    }
    writeln!(writer, "  {}", "─".repeat(separator_len.min(120)))?;

    Ok(())
}

fn render_json_to_writer<T: Serialize, W: Write>(
    items: &[T],
    writer: &mut W,
) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(items).expect("serialization to JSON failed");
    writeln!(writer, "{json}")
}

fn render_csv_to_writer<T: Serialize, W: Write>(
    items: &[T],
    writer: &mut W,
) -> std::io::Result<()> {
    // Build into a byte buffer first so the csv crate can own the writer, then
    // flush to the real writer in one shot.
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut wtr = csv::Writer::from_writer(&mut buf);
        for item in items {
            wtr.serialize(item).expect("CSV serialization failed");
        }
        wtr.flush().expect("CSV flush failed");
    }
    writer.write_all(&buf)
}

fn render_jsonl_to_writer<T: Serialize, W: Write>(
    items: &[T],
    writer: &mut W,
) -> std::io::Result<()> {
    for item in items {
        let line = serde_json::to_string(item).expect("serialization to JSON failed");
        writeln!(writer, "{line}")?;
    }
    Ok(())
}

// ─── Renderable implementations for core types ───────────────────────────────
//
// These live here (next to the trait definition) to satisfy the orphan rule:
// both the trait and the structs are defined in this crate.

use crate::analysis::screener::ScreenResult;
use crate::data::models::RegimeState;
use crate::storage::db::AlertLogEntry;

impl Renderable for ScreenResult {
    fn table_headers() -> Vec<&'static str> {
        vec![
            "Ticker",
            "Grade",
            "Score",
            "Minervini",
            "Cond",
            "ADX",
            "RS%",
            "52wH%",
            "Sentiment",
            "Regime",
        ]
    }

    fn table_row(&self) -> Vec<String> {
        vec![
            self.ticker.clone(),
            self.grade.to_string(),
            format!("{:.1}", self.composite_score),
            self.minervini_pass.to_string(),
            self.conditions_met.to_string(),
            format!("{:.1}", self.adx),
            format!("{:.1}", self.rs_percentile),
            format!("{:.1}", self.pct_from_52w_high),
            format!("{:.2}", self.sentiment_score),
            self.regime.to_string(),
        ]
    }
}

impl Renderable for AlertLogEntry {
    fn table_headers() -> Vec<&'static str> {
        vec!["Ticker", "Type", "Severity", "Message", "Sent At"]
    }

    fn table_row(&self) -> Vec<String> {
        let msg: String = if self.message.chars().count() > 60 {
            let t: String = self.message.chars().take(59).collect();
            format!("{t}…")
        } else {
            self.message.clone()
        };
        vec![
            self.ticker.clone(),
            self.alert_type.clone(),
            self.severity.clone(),
            msg,
            self.sent_at.clone(),
        ]
    }
}

impl Renderable for RegimeState {
    fn table_headers() -> Vec<&'static str> {
        vec!["Date", "Regime", "SPY Close", "SMA200", "Breadth%"]
    }

    fn table_row(&self) -> Vec<String> {
        vec![
            self.date.to_string(),
            self.regime.to_string(),
            format!("{:.2}", self.spy_close),
            format!("{:.2}", self.spy_sma200),
            format!("{:.1}", self.breadth_pct),
        ]
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    // Minimal test struct ─────────────────────────────────────────────────────

    #[derive(Debug, Clone, Serialize)]
    struct Item {
        ticker: String,
        score: f64,
        pass: bool,
    }

    impl Renderable for Item {
        fn table_headers() -> Vec<&'static str> {
            vec!["Ticker", "Score", "Pass"]
        }

        fn table_row(&self) -> Vec<String> {
            vec![
                self.ticker.clone(),
                format!("{:.1}", self.score),
                self.pass.to_string(),
            ]
        }
    }

    fn sample_items() -> Vec<Item> {
        vec![
            Item {
                ticker: "NVDA".into(),
                score: 94.2,
                pass: true,
            },
            Item {
                ticker: "AAPL".into(),
                score: 71.2,
                pass: true,
            },
        ]
    }

    fn render_to_string(items: &[Item], fmt: &OutputFormat) -> String {
        let mut buf: Vec<u8> = Vec::new();
        render_to_writer(items, fmt, &mut buf).unwrap();
        String::from_utf8(buf).unwrap()
    }

    // ── JSON ─────────────────────────────────────────────────────────────────

    #[test]
    fn json_is_valid_and_round_trips() {
        let items = sample_items();
        let out = render_to_string(&items, &OutputFormat::Json);

        let parsed: Vec<serde_json::Value> =
            serde_json::from_str(&out).expect("must be valid JSON");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["ticker"], "NVDA");
        assert!((parsed[0]["score"].as_f64().unwrap() - 94.2).abs() < 1e-6);
    }

    #[test]
    fn json_empty_produces_empty_array() {
        let out = render_to_string(&[], &OutputFormat::Json);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
        assert!(parsed.is_empty());
    }

    // ── CSV ──────────────────────────────────────────────────────────────────

    #[test]
    fn csv_has_header_and_correct_row_count() {
        let items = sample_items();
        let out = render_to_string(&items, &OutputFormat::Csv);
        let lines: Vec<&str> = out.lines().collect();

        // First line must be a header
        assert!(
            lines[0].contains("ticker"),
            "CSV must start with header row"
        );
        // One row per item after the header
        assert_eq!(lines.len() - 1, items.len());
    }

    #[test]
    fn csv_values_appear_in_correct_columns() {
        let items = sample_items();
        let out = render_to_string(&items, &OutputFormat::Csv);
        let mut rdr = csv::Reader::from_reader(out.as_bytes());

        let records: Vec<csv::StringRecord> = rdr.records().map(|r| r.unwrap()).collect();
        assert_eq!(records.len(), 2);
        // csv::Writer::serialize writes fields in the serde field order
        // ticker,score,pass
        assert_eq!(&records[0][0], "NVDA");
        assert_eq!(&records[1][0], "AAPL");
    }

    #[test]
    fn csv_empty_produces_no_rows() {
        // The csv crate only emits the header row when serialize() is called with
        // at least one record.  With an empty slice the output is empty.
        let empty: Vec<Item> = vec![];
        let out = render_to_string(&empty, &OutputFormat::Csv);
        // Empty output is valid RFC 4180 — no header, no rows.
        assert!(
            out.is_empty() || out.trim().is_empty(),
            "empty CSV should produce no output, got: {:?}",
            out
        );
    }

    // ── JSONL ────────────────────────────────────────────────────────────────

    #[test]
    fn jsonl_one_object_per_line() {
        let items = sample_items();
        let out = render_to_string(&items, &OutputFormat::Jsonl);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), items.len());

        for line in &lines {
            let v: serde_json::Value =
                serde_json::from_str(line).expect("each line must be valid JSON");
            assert!(v.is_object());
        }
    }

    #[test]
    fn jsonl_empty_produces_no_lines() {
        let empty: Vec<Item> = vec![];
        let out = render_to_string(&empty, &OutputFormat::Jsonl);
        assert!(out.is_empty() || out.trim().is_empty());
    }

    // ── Table ────────────────────────────────────────────────────────────────

    #[test]
    fn table_contains_headers_and_values() {
        let items = sample_items();
        let out = render_to_string(&items, &OutputFormat::Table);
        assert!(out.contains("Ticker"));
        assert!(out.contains("NVDA"));
        assert!(out.contains("AAPL"));
    }

    #[test]
    fn table_empty_shows_no_results_message() {
        let empty: Vec<Item> = vec![];
        let out = render_to_string(&empty, &OutputFormat::Table);
        assert!(out.contains("(no results)"));
    }

    // ── OutputFormat display ─────────────────────────────────────────────────

    #[test]
    fn output_format_display() {
        assert_eq!(OutputFormat::Table.to_string(), "table");
        assert_eq!(OutputFormat::Json.to_string(), "json");
        assert_eq!(OutputFormat::Csv.to_string(), "csv");
        assert_eq!(OutputFormat::Jsonl.to_string(), "jsonl");
        assert_eq!(OutputFormat::Html.to_string(), "html");
    }

    // ── Html fallback ────────────────────────────────────────────────────────

    #[test]
    fn html_format_falls_back_to_json() {
        let items = sample_items();
        let out = render_to_string(&items, &OutputFormat::Html);
        // Html falls back to JSON — the result must be parseable as a JSON array
        let parsed: Vec<serde_json::Value> =
            serde_json::from_str(&out).expect("Html fallback must produce valid JSON");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["ticker"], "NVDA");
    }

    // ── ScreenResult Renderable ──────────────────────────────────────────────

    #[test]
    fn screen_result_table_row_matches_header_count() {
        use crate::analysis::indicators::LiquidityClass;
        use crate::analysis::scorer::Grade;
        use crate::analysis::screener::ScreenResult;
        use crate::data::models::MarketRegime;

        let result = ScreenResult {
            ticker: "NVDA".into(),
            minervini_pass: true,
            conditions_met: 8,
            adx: 35.0,
            rs_percentile: 80.0,
            volume_zscore: 1.5,
            technical_score: 50.0,
            volume_score: 25.0,
            sentiment_score: 0.1,
            composite_score: 91.0,
            grade: Grade::A,
            close_price: 100.0,
            pct_from_52w_high: -3.0,
            atr_value: 2.5,
            risk_reward: 3.0,
            liquidity: LiquidityClass::High,
            weighted_rs: 0.5,
            rs_momentum: None,
            vcp: None,
            regime: MarketRegime::Bull,
        };
        assert_eq!(
            ScreenResult::table_headers().len(),
            result.table_row().len(),
            "Header count must equal row column count"
        );
    }

    // ── AlertLogEntry Renderable ─────────────────────────────────────────────

    #[test]
    fn alert_log_entry_long_message_truncated() {
        use crate::storage::db::AlertLogEntry;

        // Build a message that is exactly 100 characters long
        let long_message: String = "A".repeat(100);
        let entry = AlertLogEntry {
            ticker: "NVDA".into(),
            alert_type: "NewSignal".into(),
            severity: "CRITICAL".into(),
            message: long_message,
            sent_at: "2024-01-01 00:00:00".into(),
        };
        let row = entry.table_row();
        // The message column is index 3
        let rendered_msg = &row[3];
        assert!(
            rendered_msg.contains('…'),
            "Long message (100 chars) should be truncated with '…', got: {:?}",
            rendered_msg
        );
    }
}
