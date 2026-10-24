//! Forward RSU vesting schedules as typed domain objects.
//!
//! `VestingEvent` mirrors the 12-column vesting CSV schema; only
//! `symbol,vesting_date,quantity` are required. CSV is the upstream boundary —
//! PDF parsing is deliberately out of scope (see docs/PLAN_PIPELINE.md).
//!
//! Tests are co-located below under `#[cfg(test)] mod tests`.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One scheduled future RSU vesting event.
///
/// The `quantity` CSV column maps to `gross_quantity` here for clarity; the
/// rename is handled by `#[serde(rename = "quantity")]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VestingEvent {
    pub snapshot_date: NaiveDate,
    pub plan_name: String,
    pub symbol: String,
    pub plan_account: String,
    pub grant_date: NaiveDate,
    pub grant_id: String,
    pub vesting_date: NaiveDate,
    pub distribution_date: NaiveDate,
    #[serde(rename = "quantity")]
    pub gross_quantity: f64,
    pub distribution_value: f64,
    pub distributed_in: String,
    pub source_file: String,
}

impl VestingEvent {
    /// Net shares retained after employer sell-to-cover withholding.
    ///
    /// `withhold_rate` is a tax assumption supplied by the caller (e.g. 0.22
    /// for the federal supplemental income rate).
    #[allow(dead_code)]
    pub fn net_quantity(&self, withhold_rate: f64) -> f64 {
        self.gross_quantity * (1.0 - withhold_rate)
    }

    /// True iff `vesting_date ∈ [start, end)` (inclusive start, exclusive end).
    #[allow(dead_code)]
    pub fn in_range(&self, start: NaiveDate, end: NaiveDate) -> bool {
        self.vesting_date >= start && self.vesting_date < end
    }
}

/// Collection of forward vesting events with query helpers.
///
/// Events must be sorted by `vesting_date` on construction so that
/// `next_vest_after` and `events_in_range` are O(log n) without re-sorting
/// on every call. The implementer should call
/// `events.sort_by_key(|e| e.vesting_date)` after parsing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DistributionSchedule {
    pub events: Vec<VestingEvent>,
}

impl DistributionSchedule {
    /// Parse a CSV string (with header) into a `DistributionSchedule`.
    ///
    /// Required columns: `symbol`, `vesting_date`, `quantity`. The other nine
    /// columns of the full 12-column format are optional and default to:
    /// `snapshot_date`/`grant_date`/`distribution_date` = `vesting_date`,
    /// `plan_name` = "RSU", `distribution_value` = 0, `distributed_in` =
    /// "Shares", and empty strings for `plan_account`, `grant_id`, `source_file`.
    pub fn from_csv_str(s: &str) -> Result<Self, VestingError> {
        let s = s.strip_prefix('\u{feff}').unwrap_or(s);
        // Skip `#` comment lines and blank lines.
        let filtered: String = s
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('#')
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut rdr = csv::ReaderBuilder::new()
            .trim(csv::Trim::All)
            .from_reader(filtered.as_bytes());
        let headers: Vec<String> = rdr.headers()?.iter().map(|h| h.to_lowercase()).collect();
        let col = |name: &str| headers.iter().position(|h| h == name);
        let require = |name: &'static str| col(name).ok_or(VestingError::MissingColumn(name));
        let (i_sym, i_vest, i_qty) = (
            require("symbol")?,
            require("vesting_date")?,
            require("quantity")?,
        );
        let (i_snap, i_plan, i_acct, i_grant_d, i_grant_id) = (
            col("snapshot_date"),
            col("plan_name"),
            col("plan_account"),
            col("grant_date"),
            col("grant_id"),
        );
        let (i_dist_d, i_dist_v, i_dist_in, i_src) = (
            col("distribution_date"),
            col("distribution_value"),
            col("distributed_in"),
            col("source_file"),
        );

        let mut events: Vec<VestingEvent> = Vec::new();
        for (n, rec) in rdr.records().enumerate() {
            let rec = rec?;
            let row = n + 1;
            if rec.iter().all(|f| f.trim().is_empty()) {
                continue; // blank spreadsheet row
            }
            let get = |i: Option<usize>| {
                i.and_then(|i| rec.get(i))
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
            };
            let date = |name: &str, v: &str| {
                NaiveDate::parse_from_str(v, "%Y-%m-%d").map_err(|_| VestingError::InvalidValue {
                    row,
                    column: name.to_string(),
                    value: v.to_string(),
                    expected: "a date as YYYY-MM-DD",
                })
            };
            let num = |name: &str, v: &str| {
                v.replace(['$', ','], "")
                    .parse::<f64>()
                    .map_err(|_| VestingError::InvalidValue {
                        row,
                        column: name.to_string(),
                        value: v.to_string(),
                        expected: "a number",
                    })
            };
            let sym = rec.get(i_sym).unwrap_or("").trim().to_uppercase();
            let vest_raw = rec.get(i_vest).unwrap_or("").trim();
            if sym.is_empty() {
                return Err(VestingError::EmptySymbol { row });
            }
            let vesting_date = date("vesting_date", vest_raw)?;
            let opt_date = |name: &str, i: Option<usize>| match get(i) {
                Some(v) => date(name, v),
                None => Ok(vesting_date),
            };
            events.push(VestingEvent {
                snapshot_date: opt_date("snapshot_date", i_snap)?,
                plan_name: get(i_plan).unwrap_or("RSU").to_string(),
                symbol: sym,
                plan_account: get(i_acct).unwrap_or("").to_string(),
                grant_date: opt_date("grant_date", i_grant_d)?,
                grant_id: get(i_grant_id).unwrap_or("").to_string(),
                vesting_date,
                distribution_date: opt_date("distribution_date", i_dist_d)?,
                gross_quantity: {
                    let q = num("quantity", rec.get(i_qty).unwrap_or(""))?;
                    if !q.is_finite() || q < 0.0 {
                        return Err(VestingError::NegativeQuantity { row });
                    }
                    q
                },
                distribution_value: match get(i_dist_v) {
                    Some(v) => num("distribution_value", v)?,
                    None => 0.0,
                },
                distributed_in: get(i_dist_in).unwrap_or("Shares").to_string(),
                source_file: get(i_src).unwrap_or("").to_string(),
            });
        }
        events.sort_by_key(|e| e.vesting_date);
        Ok(Self { events })
    }

    /// Read a CSV file from `path` and delegate to `from_csv_str`.
    #[allow(dead_code)]
    pub fn from_csv_file(path: &Path) -> Result<Self, VestingError> {
        let s = std::fs::read_to_string(path)?;
        Self::from_csv_str(&s)
    }

    /// All events whose `vesting_date ∈ [start, end)`.
    #[allow(dead_code)]
    pub fn events_in_range(&self, start: NaiveDate, end: NaiveDate) -> Vec<&VestingEvent> {
        self.events
            .iter()
            .filter(|e| e.in_range(start, end))
            .collect()
    }

    /// The chronologically first event with `vesting_date > after`.
    #[allow(dead_code)]
    pub fn next_vest_after(&self, after: NaiveDate) -> Option<&VestingEvent> {
        self.events.iter().find(|e| e.vesting_date > after)
    }

    /// Sum of `gross_quantity` for all events matching `symbol`.
    #[allow(dead_code)]
    pub fn total_gross_quantity(&self, symbol: &str) -> f64 {
        self.events
            .iter()
            .filter(|e| e.symbol == symbol)
            .map(|e| e.gross_quantity)
            .sum()
    }

    /// Sorted unique list of symbols present in the schedule.
    #[allow(dead_code)]
    pub fn symbols(&self) -> Vec<String> {
        let mut syms: Vec<String> = self.events.iter().map(|e| e.symbol.clone()).collect();
        syms.sort();
        syms.dedup();
        syms
    }

    /// Number of events in the schedule.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True iff the schedule contains no events.
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

/// Errors that can occur while loading a vesting schedule.
#[derive(Debug, thiserror::Error)]
pub enum VestingError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("CSV parse: {0}")]
    Csv(#[from] csv::Error),
    #[error(
        "vesting CSV is missing required column `{0}` (required: symbol,vesting_date,quantity)"
    )]
    MissingColumn(&'static str),
    #[error(
        "vesting CSV row {row}: invalid value `{value}` in column `{column}` (expected {expected})"
    )]
    InvalidValue {
        row: usize,
        column: String,
        value: String,
        expected: &'static str,
    },
    #[error("vesting CSV row {row}: symbol is empty")]
    EmptySymbol { row: usize },
    #[error("vesting CSV row {row}: quantity must be a non-negative number")]
    NegativeQuantity { row: usize },
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    // Fixture content embedded at compile time; Path::new(...) tests load the
    // same bytes from disk. Both must agree — verified by from_csv_file test.
    const SAMPLE_CSV: &str = include_str!("../../tests/fixtures/vesting/sample.csv");

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn example_event(vesting_date: &str, qty: f64) -> VestingEvent {
        VestingEvent {
            snapshot_date: d("2026-05-01"),
            plan_name: "EXAMPLE RSU".to_string(),
            symbol: "MSFT".to_string(),
            plan_account: "EX0000001".to_string(),
            grant_date: d("2025-03-01"),
            grant_id: "G-0001".to_string(),
            vesting_date: d(vesting_date),
            distribution_date: d(vesting_date),
            gross_quantity: qty,
            distribution_value: qty * 420.0,
            distributed_in: "Shares".to_string(),
            source_file: "example_vesting_schedule.pdf".to_string(),
        }
    }

    // ── VestingEvent ─────────────────────────────────────────────────────

    /// Round-trip a single event via csv::Writer + csv::Reader.
    /// Exercises #[serde(rename = "quantity")] on both serialize and deserialize
    /// so that any drift between struct field name and CSV column header fails.
    #[test]
    fn vesting_event_round_trips_through_csv() {
        let event = example_event("2026-08-15", 40.0);

        let mut buf = Vec::new();
        {
            let mut wtr = csv::Writer::from_writer(&mut buf);
            wtr.serialize(&event).unwrap();
            wtr.flush().unwrap();
        }
        let csv_str = String::from_utf8(buf).unwrap();
        let mut rdr = csv::Reader::from_reader(csv_str.as_bytes());
        let parsed: VestingEvent = rdr.deserialize().next().unwrap().unwrap();
        assert_eq!(parsed, event);
    }

    /// net_quantity applies withhold rate correctly.
    /// Three representative cases: normal rate, zero, and full withholding.
    #[test]
    fn vesting_event_net_quantity_applies_withhold_rate() {
        let event = example_event("2026-08-15", 1000.0);

        assert!((event.net_quantity(0.37) - 630.0).abs() < 1e-9);
        assert!((event.net_quantity(0.22) - 780.0).abs() < 1e-9);
        assert!((event.net_quantity(0.0) - 1000.0).abs() < 1e-9);
        assert!((event.net_quantity(1.0) - 0.0).abs() < 1e-9);
    }

    /// in_range uses [start, end) semantics: inclusive start, exclusive end.
    #[test]
    fn vesting_event_in_range_handles_inclusive_start_exclusive_end() {
        let mut event = example_event("2027-01-15", 100.0);

        let start = d("2027-01-15");
        let end = d("2027-06-30");

        // vest_date == start → TRUE (inclusive)
        assert!(event.in_range(start, end));
        // vest_date == end → FALSE (exclusive)
        event.vesting_date = end;
        assert!(!event.in_range(start, end));
        // one day before start → FALSE
        event.vesting_date = d("2027-01-14");
        assert!(!event.in_range(start, end));
        // one day before end → TRUE
        event.vesting_date = d("2027-06-29");
        assert!(event.in_range(start, end));
    }

    // ── DistributionSchedule::from_csv_* (12-column format) ──────────────

    /// Parse the embedded sample fixture; verify row count and spot-check
    /// field values on the first event (MSFT, 2026-05-15, qty=40, grant G-0001).
    #[test]
    fn from_csv_str_parses_sample_fixture() {
        let sched = DistributionSchedule::from_csv_str(SAMPLE_CSV).unwrap();
        // fixture has 12 data rows
        assert_eq!(sched.len(), 12);
        // First event after sort-by-vesting_date is MSFT 2026-05-15
        let first = &sched.events[0];
        assert_eq!(first.symbol, "MSFT");
        assert_eq!(first.plan_name, "EXAMPLE RSU");
        assert_eq!(first.plan_account, "EX0000001");
        assert_eq!(first.grant_id, "G-0001");
        assert_eq!(first.vesting_date, d("2026-05-15"));
        assert!((first.gross_quantity - 40.0).abs() < 1e-9);
        assert_eq!(first.distributed_in, "Shares");
        assert_eq!(first.source_file, "example_vesting_schedule.pdf");
        // Last event is the G-0002 tranche of 2028-03-01.
        let last = sched.events.last().unwrap();
        assert_eq!(last.vesting_date, d("2028-03-01"));
        assert_eq!(last.grant_id, "G-0002");
        assert!((last.gross_quantity - 25.0).abs() < 1e-9);
    }

    /// Header-only CSV (no data rows) produces an empty schedule, not an error.
    #[test]
    fn from_csv_str_handles_header_only() {
        let header_only = "snapshot_date,plan_name,symbol,plan_account,\
            grant_date,grant_id,vesting_date,distribution_date,\
            quantity,distribution_value,distributed_in,source_file\n";
        let sched = DistributionSchedule::from_csv_str(header_only).unwrap();
        assert!(sched.is_empty());
    }

    /// A row whose column count does not match a valid 12-column header
    /// returns Err(VestingError::Csv).
    #[test]
    fn from_csv_str_fails_on_malformed() {
        let bad = "snapshot_date,plan_name,symbol,plan_account,\
            grant_date,grant_id,vesting_date,distribution_date,\
            quantity,distribution_value,distributed_in,source_file\n1,2\n";
        let result = DistributionSchedule::from_csv_str(bad);
        assert!(matches!(result, Err(VestingError::Csv(_))));
    }

    /// from_csv_file loads the committed fixture file and produces the same
    /// schedule as from_csv_str on the same embedded bytes.
    #[test]
    fn from_csv_file_loads_committed_fixture() {
        let via_str = DistributionSchedule::from_csv_str(SAMPLE_CSV).unwrap();
        let via_file =
            DistributionSchedule::from_csv_file(Path::new("tests/fixtures/vesting/sample.csv"))
                .unwrap();
        assert_eq!(via_file, via_str);
    }

    // ── Query methods ─────────────────────────────────────────────────────

    /// events_in_range(2027-01-01, 2028-01-01) returns only events whose
    /// vesting_date falls in calendar year 2027 (6 rows in the fixture).
    #[test]
    fn events_in_range_filters_correctly() {
        let sched = DistributionSchedule::from_csv_str(SAMPLE_CSV).unwrap();
        let year_2027 = sched.events_in_range(d("2027-01-01"), d("2028-01-01"));
        // 2027-02-15, 03-01, 05-15, 08-15, 09-01, 11-15
        assert_eq!(year_2027.len(), 6);
        for ev in &year_2027 {
            assert!(ev.vesting_date >= d("2027-01-01"));
            assert!(ev.vesting_date < d("2028-01-01"));
        }
    }

    /// next_vest_after(2026-07-01) returns the chronologically earliest event
    /// with vesting_date strictly greater than 2026-07-01.
    #[test]
    fn next_vest_after_returns_chronologically_first() {
        let sched = DistributionSchedule::from_csv_str(SAMPLE_CSV).unwrap();
        let next = sched.next_vest_after(d("2026-07-01")).unwrap();
        assert_eq!(next.vesting_date, d("2026-08-15"));
    }

    /// next_vest_after a date beyond all events returns None.
    #[test]
    fn next_vest_after_returns_none_if_no_future_vests() {
        let sched = DistributionSchedule::from_csv_str(SAMPLE_CSV).unwrap();
        assert!(sched.next_vest_after(d("2099-01-01")).is_none());
    }

    /// total_gross_quantity sums only rows for the given symbol.
    /// MSFT: 8 × 40 + 4 × 25 = 420.
    #[test]
    fn total_gross_quantity_sums_per_symbol() {
        let sched = DistributionSchedule::from_csv_str(SAMPLE_CSV).unwrap();
        assert!((sched.total_gross_quantity("MSFT") - 420.0).abs() < 1e-9);
        assert_eq!(sched.total_gross_quantity("AAPL"), 0.0);
    }

    /// symbols() returns sorted unique symbols.
    #[test]
    fn symbols_returns_sorted_unique() {
        let sched = DistributionSchedule::from_csv_str(SAMPLE_CSV).unwrap();
        assert_eq!(sched.symbols(), vec!["MSFT".to_string()]);
    }

    // ── Minimal format (spec §2) ──────────────────────────────────────────

    /// Minimal CSV: only symbol,vesting_date,quantity. All other nine columns
    /// take their documented defaults.
    #[test]
    fn from_csv_str_minimal_format_applies_documented_defaults() {
        let csv = "symbol,vesting_date,quantity\nMSFT,2026-08-15,100\n";
        let sched = DistributionSchedule::from_csv_str(csv).unwrap();
        assert_eq!(sched.len(), 1);
        let e = &sched.events[0];
        assert_eq!(e.symbol, "MSFT");
        assert_eq!(e.vesting_date, d("2026-08-15"));
        assert!((e.gross_quantity - 100.0).abs() < 1e-9);
        assert_eq!(
            e.snapshot_date,
            d("2026-08-15"),
            "snapshot_date defaults to vesting_date"
        );
        assert_eq!(e.plan_name, "RSU");
        assert_eq!(e.plan_account, "");
        assert_eq!(
            e.grant_date,
            d("2026-08-15"),
            "grant_date defaults to vesting_date"
        );
        assert_eq!(e.grant_id, "");
        assert_eq!(
            e.distribution_date,
            d("2026-08-15"),
            "distribution_date defaults to vesting_date"
        );
        assert_eq!(e.distribution_value, 0.0);
        assert_eq!(e.distributed_in, "Shares");
        assert_eq!(e.source_file, "");
    }

    /// Column order is free in the minimal format.
    #[test]
    fn from_csv_str_minimal_format_accepts_any_column_order() {
        let csv = "quantity,symbol,vesting_date\n25,MSFT,2027-01-02\n";
        let sched = DistributionSchedule::from_csv_str(csv).unwrap();
        assert_eq!(sched.events[0].symbol, "MSFT");
        assert_eq!(sched.events[0].vesting_date, d("2027-01-02"));
        assert!((sched.events[0].gross_quantity - 25.0).abs() < 1e-9);
    }

    /// Optional columns, when present, override the defaults; absent ones
    /// still default. Mixed subset: plan_name, grant_id, distribution_value.
    #[test]
    fn from_csv_str_minimal_format_optional_columns_override_defaults() {
        let csv = "symbol,vesting_date,quantity,plan_name,grant_id,distribution_value,grant_date\n\
                   MSFT,2026-08-15,100,MY PLAN,G-9,4200.50,2025-01-15\n";
        let sched = DistributionSchedule::from_csv_str(csv).unwrap();
        let e = &sched.events[0];
        assert_eq!(e.plan_name, "MY PLAN");
        assert_eq!(e.grant_id, "G-9");
        assert!((e.distribution_value - 4200.50).abs() < 1e-9);
        assert_eq!(e.grant_date, d("2025-01-15"));
        // still defaulted
        assert_eq!(e.snapshot_date, d("2026-08-15"));
        assert_eq!(e.distribution_date, d("2026-08-15"));
        assert_eq!(e.distributed_in, "Shares");
        assert_eq!(e.plan_account, "");
        assert_eq!(e.source_file, "");
    }

    /// Minimal-format events are sorted by vesting_date, like the 12-column path.
    #[test]
    fn from_csv_str_minimal_format_sorts_by_vesting_date() {
        let csv = "symbol,vesting_date,quantity\n\
                   MSFT,2027-02-01,10\nMSFT,2026-02-01,20\nMSFT,2026-09-01,30\n";
        let sched = DistributionSchedule::from_csv_str(csv).unwrap();
        let dates: Vec<_> = sched.events.iter().map(|e| e.vesting_date).collect();
        assert_eq!(
            dates,
            vec![d("2026-02-01"), d("2026-09-01"), d("2027-02-01")]
        );
    }

    /// Minimal-format file loads through from_csv_file.
    #[test]
    fn from_csv_file_minimal_format_loads() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("vesting.csv");
        std::fs::write(&path, "symbol,vesting_date,quantity\nMSFT,2026-08-15,100\n").unwrap();
        let sched = DistributionSchedule::from_csv_file(&path).unwrap();
        assert_eq!(sched.len(), 1);
        assert_eq!(sched.events[0].plan_name, "RSU");
    }

    /// A header missing one of symbol/vesting_date/quantity is rejected, and the
    /// error message names the missing column.
    /// (Assumption: error variant is unspecified; only is_err + message are pinned.)
    #[test]
    fn from_csv_str_missing_required_column_errors_naming_the_column() {
        for (csv, missing) in [
            ("symbol,vesting_date\nMSFT,2026-08-15\n", "quantity"),
            ("symbol,quantity\nMSFT,100\n", "vesting_date"),
            ("vesting_date,quantity\n2026-08-15,100\n", "symbol"),
        ] {
            let err = DistributionSchedule::from_csv_str(csv)
                .expect_err("missing required column must be an error");
            assert!(
                err.to_string().contains(missing),
                "error {err:?} should name missing column {missing}"
            );
        }
    }

    /// Header-only minimal CSV yields an empty schedule, not an error.
    #[test]
    fn from_csv_str_minimal_format_header_only_is_empty() {
        let sched = DistributionSchedule::from_csv_str("symbol,vesting_date,quantity\n").unwrap();
        assert!(sched.is_empty());
    }

    // ═════════════════════════════════════════════════════════════════════
    // Round 2 regressions (M1, M4 vesting validation)
    // ═════════════════════════════════════════════════════════════════════

    #[test]
    fn m1_vesting_symbols_are_upper_cased() {
        let sched = DistributionSchedule::from_csv_str(
            "symbol,vesting_date,quantity\nmsft,2026-08-15,10\n Aapl ,2026-09-15,5\n",
        )
        .unwrap();
        assert_eq!(
            sched.symbols(),
            vec!["AAPL".to_string(), "MSFT".to_string()]
        );
        assert!((sched.total_gross_quantity("MSFT") - 10.0).abs() < 1e-9);
    }

    #[test]
    fn m1_vesting_symbols_upper_cased_in_twelve_column_format_too() {
        let lower = SAMPLE_CSV.replace(",MSFT,", ",msft,");
        let sched = DistributionSchedule::from_csv_str(&lower).unwrap();
        assert_eq!(sched.symbols(), vec!["MSFT".to_string()]);
    }

    #[test]
    fn m4_empty_symbol_is_an_error_naming_the_row() {
        let csv = "symbol,vesting_date,quantity\nMSFT,2026-08-15,10\n,2026-09-15,10\n";
        let err = DistributionSchedule::from_csv_str(csv).expect_err("empty symbol");
        let msg = err.to_string().to_lowercase();
        assert!(msg.contains("symbol"), "{msg}");
        assert!(
            msg.contains("row 2") || msg.contains("line 3"),
            "message must give the row number (data row 2 / file line 3): {msg}"
        );
    }

    #[test]
    fn m4_whitespace_only_symbol_is_an_error() {
        let csv = "symbol,vesting_date,quantity\n   ,2026-09-15,10\n";
        assert!(DistributionSchedule::from_csv_str(csv).is_err());
    }

    #[test]
    fn m4_negative_quantity_is_an_error() {
        let csv = "symbol,vesting_date,quantity\nMSFT,2026-08-15,-10\n";
        let err = DistributionSchedule::from_csv_str(csv).expect_err("negative quantity");
        assert!(err.to_string().to_lowercase().contains("quantity"), "{err}");
    }

    #[test]
    fn m4_zero_quantity_is_allowed() {
        let csv = "symbol,vesting_date,quantity\nMSFT,2026-08-15,0\n";
        assert!(DistributionSchedule::from_csv_str(csv).is_ok());
    }

    #[test]
    fn m4_negative_quantity_in_twelve_column_format_is_an_error() {
        let bad = SAMPLE_CSV.replacen(",40.000,", ",-40.000,", 1);
        assert!(DistributionSchedule::from_csv_str(&bad).is_err());
    }

    #[test]
    fn m4_bad_date_error_mentions_expected_format() {
        let csv = "symbol,vesting_date,quantity\nMSFT,15/08/2026,10\n";
        let err = DistributionSchedule::from_csv_str(csv).expect_err("bad date");
        assert!(err.to_string().contains("YYYY-MM-DD"), "{err}");
    }

    #[test]
    fn m4_bad_date_in_optional_column_mentions_expected_format() {
        let csv = "symbol,vesting_date,quantity,grant_date\nMSFT,2026-08-15,10,March 1\n";
        let err = DistributionSchedule::from_csv_str(csv).expect_err("bad grant_date");
        assert!(err.to_string().contains("YYYY-MM-DD"), "{err}");
    }

    #[test]
    fn m4_hash_comment_lines_are_skipped() {
        let csv = "# my vesting notes\nsymbol,vesting_date,quantity\n\
                   # first tranche\nMSFT,2026-08-15,10\n\n#  skipped\nMSFT,2026-11-15,20\n";
        let sched = DistributionSchedule::from_csv_str(csv).unwrap();
        assert_eq!(sched.len(), 2);
        assert!((sched.total_gross_quantity("MSFT") - 30.0).abs() < 1e-9);
    }

    #[test]
    fn m4_hash_comment_lines_are_skipped_in_twelve_column_format() {
        let with_comments = format!(
            "# header comment\n{}",
            SAMPLE_CSV.replacen('\n', "\n# mid comment\n", 1)
        );
        let sched = DistributionSchedule::from_csv_str(&with_comments).unwrap();
        assert_eq!(sched.len(), 12);
    }

    // ═══ Round 3 ═══

    #[test]
    fn r3_all_empty_row_is_skipped_not_an_error() {
        let csv = "symbol,vesting_date,quantity\nMSFT,2026-08-15,10\n,,\n";
        let sched = DistributionSchedule::from_csv_str(csv).expect("blank Excel row is skipped");
        assert_eq!(sched.len(), 1);
    }

    #[test]
    fn r3_all_empty_row_with_spaces_is_skipped_in_the_middle_too() {
        let csv = "symbol,vesting_date,quantity\nMSFT,2026-08-15,10\n , , \nMSFT,2026-09-15,5\n";
        assert_eq!(DistributionSchedule::from_csv_str(csv).unwrap().len(), 2);
    }

    #[test]
    fn r3_all_empty_row_is_skipped_in_twelve_column_format() {
        let csv = format!("{}{}\n", SAMPLE_CSV, ",".repeat(11));
        assert_eq!(DistributionSchedule::from_csv_str(&csv).unwrap().len(), 12);
    }

    #[test]
    fn r3_row_with_only_symbol_empty_still_errors() {
        let csv = "symbol,vesting_date,quantity\n,2026-08-15,10\n";
        assert!(DistributionSchedule::from_csv_str(csv).is_err());
    }

    #[test]
    fn r3_invalid_value_error_includes_row_number() {
        let csv = "symbol,vesting_date,quantity\nMSFT,2026-08-15,10\nMSFT,2026-09-15,abc\n";
        let msg = DistributionSchedule::from_csv_str(csv)
            .expect_err("bad quantity")
            .to_string()
            .to_lowercase();
        assert!(msg.contains("row 2"), "{msg}");
    }

    #[test]
    fn r3_bad_date_error_includes_row_number() {
        let csv = "symbol,vesting_date,quantity\nMSFT,2026-08-15,10\nMSFT,2026-09-15,5\nMSFT,15/10/2026,5\n";
        let msg = DistributionSchedule::from_csv_str(csv)
            .expect_err("bad date")
            .to_string()
            .to_lowercase();
        assert!(msg.contains("row 3"), "{msg}");
    }
}
