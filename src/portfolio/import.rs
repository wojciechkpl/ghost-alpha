use std::io::Read;
use std::path::Path;

use chrono::NaiveDate;
use csv::ReaderBuilder;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::PortfolioError;

use super::types::{AcquisitionType, Portfolio, Position, TaxLot};

// ─── Fidelity ETL Types ───────────────────────────────────────────────────────

/// One row from a Fidelity Portfolio_Positions CSV export.
///
/// Fidelity quirks handled by [`read_fidelity_positions_str`]:
/// - UTF-8 BOM at file start (`\u{feff}`)
/// - Trailing disclaimer rows (lines starting with `"` after a blank line)
/// - Blank lines between data and disclaimers
/// - Currency formatting: `$1,234.56` → 1234.56
/// - Percentage formatting: `12.34%` → 12.34 (NOT 0.1234)
/// - Sentinel `"--"` for missing data → `None`
/// - FCASH** rows with empty quantity → quantity=0.0, current_value preserved
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FidelityPositionRow {
    pub account_number: String,
    pub account_name: String,
    pub symbol: String,
    pub description: String,
    /// 0.0 if blank (FCASH** cash sleeve case)
    pub quantity: f64,
    /// 1.0 if blank (FCASH** cash sleeve case)
    pub last_price: f64,
    pub current_value: f64,
    /// None if "--" or blank
    pub cost_basis_total: Option<f64>,
    /// None if "--" or blank
    pub average_cost_basis: Option<f64>,
    /// Raw percent: 12.34 means 12.34%, NOT 0.1234.
    /// Matches Python prototype's convention.
    pub percent_of_account: Option<f64>,
    /// e.g. "Cash"
    pub position_type: String,
}

#[derive(Debug, thiserror::Error)]
pub enum FidelityImportError {
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("CSV parse: {0}")]
    Csv(#[from] csv::Error),
    #[error("Required column missing: {0}")]
    MissingColumn(&'static str),
    #[error(
        "unrecognized positions format: required column `{missing}` not found. Accepted formats: \
         (1) Fidelity Portfolio_Positions export (header has `Account Number` and `Symbol`); \
         (2) simple CSV with header `symbol,quantity,price[,cost_basis,account,description]`"
    )]
    UnrecognizedFormat { missing: &'static str },
    #[error("position {symbol}: negative quantity {value} (short positions are not supported)")]
    NegativeQuantity { symbol: String, value: f64 },
    #[error("{path}: {source}")]
    AtPath {
        path: String,
        #[source]
        source: Box<FidelityImportError>,
    },
    #[error("Cannot parse {column} value: {value}")]
    ParseValue { column: String, value: String },
}

// ─── Fidelity ETL Functions ───────────────────────────────────────────────────

/// Parse Fidelity Portfolio_Positions CSV from a string.
///
/// Handles BOM, trailing disclaimer rows, blank lines, currency/percent formatting,
/// "--" sentinel, and FCASH** blank-quantity rows.
///
/// # Precondition
/// All rows must belong to a single snapshot date.  Multi-snapshot roll-up is
/// a database-layer concern; this function does not validate or enforce dates.
pub fn read_fidelity_positions_str(
    s: &str,
) -> Result<Vec<FidelityPositionRow>, FidelityImportError> {
    // Strip UTF-8 BOM if present.
    let s = s.strip_prefix('\u{feff}').unwrap_or(s);

    // flexible(true) is critical: Fidelity data rows have a trailing empty column
    // that the header row may not have (or vice-versa depending on export version).
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(s.as_bytes());

    // Columns are looked up by (case-insensitive) header name, so reordered or
    // extra columns in the export do not matter.
    let headers: Vec<String> = rdr
        .headers()?
        .iter()
        .map(|h| h.trim().to_lowercase())
        .collect();
    let col = |name: &str| headers.iter().position(|h| h == name);
    let i_account =
        col("account number").ok_or(FidelityImportError::MissingColumn("Account Number"))?;
    let i_name = col("account name");
    let i_symbol = col("symbol").ok_or(FidelityImportError::MissingColumn("Symbol"))?;
    let i_desc = col("description");
    let i_qty = Some(col("quantity").ok_or(FidelityImportError::MissingColumn("Quantity"))?);
    let i_price = col("last price");
    let i_value = col("current value");
    let i_cost_total = col("cost basis total");
    let i_cost_avg = col("average cost basis");
    let i_pct = col("percent of account");
    let i_type = col("type");
    let get = |rec: &csv::StringRecord, idx: Option<usize>| -> String {
        idx.map(|i| fid_get(rec, i).to_string()).unwrap_or_default()
    };

    let mut rows = Vec::new();
    for result in rdr.records() {
        let rec = result?;
        let account = fid_get(&rec, i_account);

        // Skip blank rows and Fidelity disclaimer rows.
        if account.is_empty() {
            continue;
        }
        if account.starts_with("The data and information")
            || account.starts_with("Brokerage services")
            || account.starts_with("Date downloaded")
        {
            continue;
        }

        let symbol = fid_get(&rec, i_symbol).trim().to_uppercase();
        let qty_raw = get(&rec, i_qty);
        // Rows with a blank quantity are only valid for cash sleeves (`FCASH**`);
        // others (e.g. "Pending Activity", account totals) are not positions.
        if qty_raw.is_empty() && !symbol.ends_with("**") {
            continue;
        }

        rows.push(FidelityPositionRow {
            account_number: account.to_string(),
            account_name: get(&rec, i_name),
            symbol,
            description: get(&rec, i_desc),
            quantity: fid_parse_num(&qty_raw).unwrap_or(0.0),
            last_price: fid_parse_num(&get(&rec, i_price)).unwrap_or(1.0),
            current_value: fid_parse_num(&get(&rec, i_value)).unwrap_or(0.0),
            cost_basis_total: fid_parse_num_optional(&get(&rec, i_cost_total)),
            average_cost_basis: fid_parse_num_optional(&get(&rec, i_cost_avg)),
            percent_of_account: fid_parse_num_optional(&get(&rec, i_pct)),
            position_type: get(&rec, i_type),
        });
    }
    Ok(rows)
}

fn fid_get(rec: &csv::StringRecord, idx: usize) -> &str {
    rec.get(idx).unwrap_or("").trim()
}

/// Parse a Fidelity-formatted number. Returns `None` for empty / `"--"` / `"N/A"`.
/// Strips `$`, `,`, `%`, `+` before parsing, so both currency and percent strings
/// are handled. The raw numeric value is returned (e.g. `"3.87%"` → `3.87`).
fn fid_parse_num_optional(s: &str) -> Option<f64> {
    if s.is_empty() || s == "--" || s == "N/A" {
        return None;
    }
    let cleaned: String = s
        .chars()
        .filter(|c| !matches!(c, '$' | ',' | '%' | '+'))
        .collect();
    cleaned.parse::<f64>().ok()
}

fn fid_parse_num(s: &str) -> Option<f64> {
    fid_parse_num_optional(s)
}

/// Convenience: read from file path.
pub fn read_fidelity_positions_csv(
    path: &Path,
) -> Result<Vec<FidelityPositionRow>, FidelityImportError> {
    let s = std::fs::read_to_string(path)?;
    read_fidelity_positions_str(&s)
}

/// Parse a positions CSV, auto-detecting the format.
///
/// If the header has `Account Number` and `Symbol` it is parsed as a Fidelity
/// export. Otherwise it is parsed as the simple format: required columns
/// `symbol`, `quantity`, `price`; optional `cost_basis` (per share), `account`,
/// `description`. Headers are case-insensitive; blank lines and lines starting
/// with `#` are skipped; values may carry a `$` prefix and thousands separators.
pub fn read_positions_auto_str(s: &str) -> Result<Vec<FidelityPositionRow>, FidelityImportError> {
    let s = s.strip_prefix('\u{feff}').unwrap_or(s);
    let header_line = s
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("");
    let lower: Vec<String> = header_line
        .split(',')
        .map(|h| h.trim().trim_matches('"').trim().to_lowercase())
        .collect();
    let has = |name: &str| lower.iter().any(|h| h == name);
    if has("account number") && has("symbol") && has("current value") && has("last price") {
        return read_fidelity_positions_str(s);
    }
    read_simple_positions_str(s)
}

/// Like [`read_positions_auto_str`], reading from `path`; errors include the path.
pub fn read_positions_auto(path: &Path) -> Result<Vec<FidelityPositionRow>, FidelityImportError> {
    let wrap = |e: FidelityImportError| FidelityImportError::AtPath {
        path: path.display().to_string(),
        source: Box::new(e),
    };
    let s = std::fs::read_to_string(path).map_err(|e| wrap(e.into()))?;
    read_positions_auto_str(&s).map_err(wrap)
}

fn read_simple_positions_str(s: &str) -> Result<Vec<FidelityPositionRow>, FidelityImportError> {
    let filtered: String = s
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(filtered.as_bytes());
    let headers: Vec<String> = rdr
        .headers()?
        .iter()
        .map(|h| h.trim().to_lowercase())
        .collect();
    let col = |name: &str| headers.iter().position(|h| h == name);
    let require = |name: &'static str| {
        col(name).ok_or(FidelityImportError::UnrecognizedFormat { missing: name })
    };
    let (i_sym, i_qty, i_px) = (require("symbol")?, require("quantity")?, require("price")?);
    let (i_basis, i_acct, i_desc) = (col("cost_basis"), col("account"), col("description"));

    let num =
        |rec: &csv::StringRecord, idx: usize, column: &str| -> Result<f64, FidelityImportError> {
            let raw = rec.get(idx).unwrap_or("");
            fid_parse_num(raw).ok_or_else(|| FidelityImportError::ParseValue {
                column: column.to_string(),
                value: raw.to_string(),
            })
        };

    let mut rows = Vec::new();
    for result in rdr.records() {
        let rec = result?;
        let symbol = rec.get(i_sym).unwrap_or("").trim().to_uppercase();
        if symbol.is_empty() {
            continue;
        }
        let quantity = num(&rec, i_qty, "quantity")?;
        let price = num(&rec, i_px, "price")?;
        if quantity < 0.0 {
            return Err(FidelityImportError::NegativeQuantity {
                symbol,
                value: quantity,
            });
        }
        let basis = match i_basis {
            Some(i) => fid_parse_num_optional(rec.get(i).unwrap_or("").trim()),
            None => None,
        };
        let text = |idx: Option<usize>| {
            idx.and_then(|i| rec.get(i))
                .unwrap_or("")
                .trim()
                .to_string()
        };
        rows.push(FidelityPositionRow {
            account_number: text(i_acct),
            account_name: String::new(),
            symbol,
            description: text(i_desc),
            quantity,
            last_price: price,
            current_value: quantity * price,
            cost_basis_total: basis.map(|b| b * quantity),
            average_cost_basis: basis,
            percent_of_account: None,
            position_type: String::new(),
        });
    }
    Ok(rows)
}

/// Roll up Fidelity rows (potentially across multiple account CSVs) into a single
/// [`Portfolio`] by summing `quantity`, `current_value`, and `cost_basis_total`
/// per symbol.
///
/// `Position::current_price` is set to the weighted-average price
/// (`current_value / quantity`), falling back to `last_price` when quantity == 0.0
/// (handles FCASH** sleeves).
///
/// For FCASH** with `quantity == 0.0`, this function stores the position as
/// `quantity = current_value` shares at `current_price = 1.0` so that
/// `market_value() == current_value`.  This matches the Python prototype's
/// `mask_cash_no_price` convention.
///
/// Each `Position` gets a single synthetic `TaxLot` with `AcquisitionType::Buy`.
///
/// # Precondition
/// All rows must be from the **same** snapshot date.  Passing rows from
/// different dates is undefined behaviour from a financial correctness standpoint.
/// Multi-snapshot ETL is a database concern, not an in-memory roll-up concern.
pub fn roll_up_to_portfolio(rows: &[FidelityPositionRow], as_of: NaiveDate) -> Portfolio {
    use std::collections::BTreeMap;

    struct Acc {
        quantity: f64,
        current_value: f64,
        cost_basis_total: f64,
        last_price: f64,
    }

    let mut by_symbol: BTreeMap<String, Acc> = BTreeMap::new();
    for r in rows {
        let acc = by_symbol.entry(r.symbol.clone()).or_insert(Acc {
            quantity: 0.0,
            current_value: 0.0,
            cost_basis_total: 0.0,
            last_price: r.last_price,
        });
        acc.quantity += r.quantity;
        acc.current_value += r.current_value;
        acc.cost_basis_total += r.cost_basis_total.unwrap_or(0.0);
        acc.last_price = r.last_price;
    }

    let positions: Vec<Position> = by_symbol
        .into_iter()
        .map(|(symbol, acc)| {
            // FCASH** sleeves: quantity is zero but current_value > 0.
            // Represent as quantity=current_value shares at price=1.0 so that
            // market_value() == current_value. Matches the Python prototype convention.
            //
            // For normal positions use current_value / quantity as the price so that
            // market_value() (= shares * current_price) equals the summed current_value
            // from the CSV exactly, rather than recomputing from last_price which may
            // differ due to intraday moves or rounding in the Fidelity export.
            let (shares, price) = if acc.quantity == 0.0 && acc.current_value > 0.0 {
                (acc.current_value, 1.0)
            } else if acc.quantity > 0.0 {
                (acc.quantity, acc.current_value / acc.quantity)
            } else {
                (acc.quantity, acc.last_price)
            };

            let cost_basis_per_share = if shares > 0.0 {
                acc.cost_basis_total / shares
            } else {
                0.0
            };

            let lot = TaxLot {
                id: Uuid::new_v4(),
                ticker: symbol.clone(),
                shares,
                remaining_shares: shares,
                cost_basis_per_share,
                acquired_date: as_of,
                acquisition_type: AcquisitionType::Buy,
            };

            Position {
                ticker: symbol,
                lots: vec![lot],
                current_price: price,
                last_updated: as_of,
            }
        })
        .collect();

    Portfolio {
        positions,
        cash: 0.0,
        as_of,
    }
}

// ─── CSV Format Detection ────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum CsvFormat {
    /// ticker,shares,cost_basis,date,type
    Generic,
    /// Symbol,Quantity,Cost Basis,Date Acquired,Action
    Schwab,
}

fn detect_format(headers: &csv::StringRecord) -> Result<CsvFormat, PortfolioError> {
    let lower: Vec<String> = headers.iter().map(|h| h.trim().to_lowercase()).collect();

    if lower.contains(&"ticker".to_string()) && lower.contains(&"shares".to_string()) {
        return Ok(CsvFormat::Generic);
    }
    if lower.contains(&"symbol".to_string()) && lower.contains(&"quantity".to_string()) {
        return Ok(CsvFormat::Schwab);
    }

    Err(PortfolioError::UnknownFormat)
}

// ─── Column Index Helpers ────────────────────────────────────────────────────

struct ColIndex {
    ticker: usize,
    shares: usize,
    cost_basis: usize,
    date: usize,
    acq_type: usize,
}

fn find_col(headers: &csv::StringRecord, name: &str) -> Result<usize, PortfolioError> {
    let lower_name = name.to_lowercase();
    headers
        .iter()
        .position(|h| h.trim().to_lowercase() == lower_name)
        .ok_or_else(|| PortfolioError::ImportParse {
            line: 0,
            reason: format!("missing column: {name}"),
        })
}

fn resolve_columns(
    headers: &csv::StringRecord,
    format: &CsvFormat,
) -> Result<ColIndex, PortfolioError> {
    match format {
        CsvFormat::Generic => Ok(ColIndex {
            ticker: find_col(headers, "ticker")?,
            shares: find_col(headers, "shares")?,
            cost_basis: find_col(headers, "cost_basis")?,
            date: find_col(headers, "date")?,
            acq_type: find_col(headers, "type")?,
        }),
        CsvFormat::Schwab => Ok(ColIndex {
            ticker: find_col(headers, "symbol")?,
            shares: find_col(headers, "quantity")?,
            cost_basis: find_col(headers, "cost basis")?,
            date: find_col(headers, "date acquired")?,
            acq_type: find_col(headers, "action")?,
        }),
    }
}

// ─── Date Parsing ────────────────────────────────────────────────────────────

fn parse_date(s: &str, line: usize) -> Result<NaiveDate, PortfolioError> {
    let trimmed = s.trim();
    // Try ISO format first (YYYY-MM-DD)
    if let Ok(d) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return Ok(d);
    }
    // US format (MM/DD/YYYY)
    if let Ok(d) = NaiveDate::parse_from_str(trimmed, "%m/%d/%Y") {
        return Ok(d);
    }
    Err(PortfolioError::ImportParse {
        line,
        reason: format!("cannot parse date: {trimmed}"),
    })
}

fn parse_f64(s: &str, field: &str, line: usize) -> Result<f64, PortfolioError> {
    let cleaned = s.trim().replace(['$', ','], "");
    cleaned
        .parse::<f64>()
        .map_err(|_| PortfolioError::ImportParse {
            line,
            reason: format!("cannot parse {field} as number: {s}"),
        })
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Detect format from header row and parse all lots.
/// Each lot gets a fresh UUID and `remaining_shares` is set equal to `shares`.
pub fn import_lots_from_reader<R: Read>(reader: R) -> Result<Vec<TaxLot>, PortfolioError> {
    let mut rdr = ReaderBuilder::new()
        .flexible(true)
        .trim(csv::Trim::All)
        .from_reader(reader);

    let headers = rdr
        .headers()
        .map_err(|e| PortfolioError::ImportParse {
            line: 0,
            reason: e.to_string(),
        })?
        .clone();

    let format = detect_format(&headers)?;
    let cols = resolve_columns(&headers, &format)?;

    let mut lots = Vec::new();

    for (i, result) in rdr.records().enumerate() {
        let line = i + 2; // 1-indexed, +1 for header row
        let record = result.map_err(|e| PortfolioError::ImportParse {
            line,
            reason: e.to_string(),
        })?;

        let ticker = record
            .get(cols.ticker)
            .ok_or_else(|| PortfolioError::ImportParse {
                line,
                reason: "missing ticker".to_string(),
            })?
            .trim()
            .to_uppercase();

        if ticker.is_empty() {
            continue; // skip blank rows
        }

        let shares = parse_f64(record.get(cols.shares).unwrap_or(""), "shares", line)?;

        let cost_basis_per_share = parse_f64(
            record.get(cols.cost_basis).unwrap_or(""),
            "cost_basis",
            line,
        )?;

        let acquired_date = parse_date(record.get(cols.date).unwrap_or(""), line)?;

        let acquisition_type =
            AcquisitionType::from_str_flexible(record.get(cols.acq_type).unwrap_or("BUY"))?;

        lots.push(TaxLot {
            id: Uuid::new_v4(),
            ticker,
            shares,
            cost_basis_per_share,
            acquired_date,
            acquisition_type,
            remaining_shares: shares,
        });
    }

    Ok(lots)
}

/// Convenience wrapper for file path.
pub fn import_lots_from_file(path: &str) -> Result<Vec<TaxLot>, PortfolioError> {
    let file = std::fs::File::open(path).map_err(|e| PortfolioError::ImportParse {
        line: 0,
        reason: e.to_string(),
    })?;
    import_lots_from_reader(file)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const GENERIC_CSV: &str = "\
ticker,shares,cost_basis,date,type
AMZN,100,150.50,2022-03-15,BUY
AAPL,50,120.00,2023-06-01,RSU_VEST
GOOG,25,2800.00,2021-01-10,ESPP
";

    const SCHWAB_CSV: &str = "\
Symbol,Quantity,Cost Basis,Date Acquired,Action
AMZN,100,$150.50,03/15/2022,Bought
AAPL,50,$120.00,06/01/2023,RSU Vest
";

    #[test]
    fn import_generic_csv() {
        let lots = import_lots_from_reader(GENERIC_CSV.as_bytes()).unwrap();
        assert_eq!(lots.len(), 3);

        assert_eq!(lots[0].ticker, "AMZN");
        assert!((lots[0].shares - 100.0).abs() < f64::EPSILON);
        assert!((lots[0].cost_basis_per_share - 150.5).abs() < f64::EPSILON);
        assert_eq!(lots[0].acquired_date.to_string(), "2022-03-15");
        assert_eq!(lots[0].acquisition_type, AcquisitionType::Buy);
        assert!((lots[0].remaining_shares - 100.0).abs() < f64::EPSILON);

        assert_eq!(lots[1].ticker, "AAPL");
        assert_eq!(lots[1].acquisition_type, AcquisitionType::RsuVest);

        assert_eq!(lots[2].ticker, "GOOG");
        assert_eq!(lots[2].acquisition_type, AcquisitionType::Espp);
    }

    #[test]
    fn import_schwab_csv() {
        let lots = import_lots_from_reader(SCHWAB_CSV.as_bytes()).unwrap();
        assert_eq!(lots.len(), 2);

        assert_eq!(lots[0].ticker, "AMZN");
        assert!((lots[0].cost_basis_per_share - 150.5).abs() < f64::EPSILON);
        assert_eq!(lots[0].acquired_date.to_string(), "2022-03-15");
        assert_eq!(lots[0].acquisition_type, AcquisitionType::Buy);

        assert_eq!(lots[1].ticker, "AAPL");
        assert_eq!(lots[1].acquisition_type, AcquisitionType::RsuVest);
    }

    #[test]
    fn import_assigns_unique_uuids() {
        let lots = import_lots_from_reader(GENERIC_CSV.as_bytes()).unwrap();
        let ids: Vec<_> = lots.iter().map(|l| l.id).collect();
        for (i, id) in ids.iter().enumerate() {
            for (j, other) in ids.iter().enumerate() {
                if i != j {
                    assert_ne!(id, other, "UUIDs must be unique");
                }
            }
        }
    }

    #[test]
    fn import_remaining_shares_equals_shares() {
        let lots = import_lots_from_reader(GENERIC_CSV.as_bytes()).unwrap();
        for lot in &lots {
            assert!(
                (lot.remaining_shares - lot.shares).abs() < f64::EPSILON,
                "remaining_shares should equal shares on import"
            );
        }
    }

    #[test]
    fn import_unknown_format_returns_error() {
        let csv = "foo,bar,baz\n1,2,3\n";
        let result = import_lots_from_reader(csv.as_bytes());
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            matches!(err, PortfolioError::UnknownFormat),
            "expected UnknownFormat, got {err:?}"
        );
    }

    #[test]
    fn import_bad_date_returns_error() {
        let csv = "ticker,shares,cost_basis,date,type\nAMZN,100,150,not-a-date,BUY\n";
        let result = import_lots_from_reader(csv.as_bytes());
        assert!(result.is_err());
    }

    #[test]
    fn import_bad_number_returns_error() {
        let csv = "ticker,shares,cost_basis,date,type\nAMZN,abc,150,2023-01-01,BUY\n";
        let result = import_lots_from_reader(csv.as_bytes());
        assert!(result.is_err());
    }

    #[test]
    fn import_skips_blank_rows() {
        let csv = "ticker,shares,cost_basis,date,type\nAMZN,100,150,2023-01-01,BUY\n,,,, \n";
        let lots = import_lots_from_reader(csv.as_bytes()).unwrap();
        assert_eq!(lots.len(), 1);
    }

    #[test]
    fn import_ticker_uppercased() {
        let csv = "ticker,shares,cost_basis,date,type\namzn,100,150,2023-01-01,BUY\n";
        let lots = import_lots_from_reader(csv.as_bytes()).unwrap();
        assert_eq!(lots[0].ticker, "AMZN");
    }

    #[test]
    fn import_dollar_sign_stripped() {
        let csv =
            "Symbol,Quantity,Cost Basis,Date Acquired,Action\nAMZN,100,$150.50,03/15/2022,Bought\n";
        let lots = import_lots_from_reader(csv.as_bytes()).unwrap();
        assert!((lots[0].cost_basis_per_share - 150.5).abs() < f64::EPSILON);
    }

    #[test]
    fn import_us_date_format() {
        let csv = "Symbol,Quantity,Cost Basis,Date Acquired,Action\nAMZN,100,150,12/31/2023,BUY\n";
        let lots = import_lots_from_reader(csv.as_bytes()).unwrap();
        assert_eq!(lots[0].acquired_date.to_string(), "2023-12-31");
    }
}

// ─── Fidelity ETL Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod fidelity_tests {
    use super::*;
    use crate::portfolio::import::{
        read_fidelity_positions_csv, read_fidelity_positions_str, roll_up_to_portfolio,
    };
    use chrono::NaiveDate;
    use std::path::Path;

    fn as_of() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 5, 1).unwrap()
    }

    // Inline synthetic CSV identical to tests/fixtures/fidelity/synthetic.csv
    // (verified by `synthetic_str_matches_fixture_file`) but expressed as a Rust string literal (LF endings; crate handles CRLF too).
    // BOM is the first character: '\u{feff}'.
    const SYNTHETIC_STR: &str = "\u{feff}Account Number,Account Name,Symbol,Description,Quantity,Last Price,Last Price Change,Current Value,Today's Gain/Loss Dollar,Today's Gain/Loss Percent,Total Gain/Loss Dollar,Total Gain/Loss Percent,Percent Of Account,Cost Basis Total,Average Cost Basis,Type,\nEX0000003,Individual,FCASH**,HELD IN FCASH,,,,$100.00,,,,,0.28%,,,Cash,\nEX0000003,Individual,AAPL,APPLE INC,100,$250.00,+$0.00,$25000.00,+$0.00,+0.00%,+$10000.00,+66.67%,71.23%,$15000.00,$150.00,Cash,\nEX0000003,Individual,SPAXX,FIDELITY GOVERNMENT MONEY MARKET,10000,$1.00,$0.00,$10000.00,--,--,--,--,28.49%,--,--,Cash,\n\n\"The data and information in this spreadsheet is provided to you solely for your use ...\"\n\n\"Brokerage services are provided by Fidelity Brokerage Services LLC ...\"\n\n\"Date downloaded May-01-2026 4:00 p.m ET\"\n";

    // ── Quirk handling on synthetic input ────────────────────────────────────

    /// BOM byte sequence must be stripped so the first header field is exactly
    /// "Account Number", not "\u{feff}Account Number".
    #[test]
    fn bom_in_header_is_stripped() {
        let rows = read_fidelity_positions_str(SYNTHETIC_STR).unwrap();
        // If BOM were not stripped the parser would fail to identify the column.
        // A successful parse with a non-empty result proves the BOM was handled.
        assert!(!rows.is_empty(), "parse must succeed and return rows");
        // Additionally: first row's account_number must not start with the BOM char.
        assert!(
            !rows[0].account_number.starts_with('\u{feff}'),
            "account_number must not contain BOM character"
        );
    }

    /// Rows that begin with a double-quote after the data section are Fidelity
    /// disclaimer rows and must be excluded from the result.
    #[test]
    fn trailing_disclaimer_rows_skipped() {
        let rows = read_fidelity_positions_str(SYNTHETIC_STR).unwrap();
        for row in &rows {
            assert!(
                !row.account_number.starts_with("The data and information"),
                "disclaimer row leaked into result: {:?}",
                row.account_number
            );
            assert!(
                !row.account_number.starts_with("Brokerage services"),
                "disclaimer row leaked into result: {:?}",
                row.account_number
            );
            assert!(
                !row.account_number.starts_with("Date downloaded"),
                "disclaimer row leaked into result: {:?}",
                row.account_number
            );
        }
    }

    /// Blank lines between data and disclaimer section must not produce entries.
    #[test]
    fn blank_lines_between_data_and_disclaimer_skipped() {
        let rows = read_fidelity_positions_str(SYNTHETIC_STR).unwrap();
        // All rows must have a non-empty symbol.
        for row in &rows {
            assert!(
                !row.symbol.is_empty(),
                "blank row produced an entry: {:?}",
                row
            );
        }
    }

    /// Currency strings like "$25000.00" and quoted "$25,000.00" must parse to plain f64.
    #[test]
    fn currency_formatting_parsed() {
        let rows = read_fidelity_positions_str(SYNTHETIC_STR).unwrap();
        let aapl = rows.iter().find(|r| r.symbol == "AAPL").unwrap();
        assert!(
            (aapl.current_value - 25000.00).abs() < 0.01,
            "expected 25000.00, got {}",
            aapl.current_value
        );
        let cb = aapl
            .cost_basis_total
            .expect("AAPL cost_basis_total must be Some");
        assert!((cb - 15000.0).abs() < 0.01, "expected 15000.00, got {}", cb);

        // Exercise the comma-thousands separator via a quoted field.
        let with_commas = SYNTHETIC_STR.replace("$25000.00", "\"$25,000.00\"");
        let rows = read_fidelity_positions_str(&with_commas).unwrap();
        let aapl = rows.iter().find(|r| r.symbol == "AAPL").unwrap();
        assert!(
            (aapl.current_value - 25000.00).abs() < 0.01,
            "expected 25000.00 from \"$25,000.00\", got {}",
            aapl.current_value
        );
    }

    /// Percent fields like "71.23%" must parse to 71.23 (NOT 0.7123).
    /// This matches the Python prototype's convention (raw percent, not fraction).
    #[test]
    fn percentage_formatting_preserves_units() {
        let rows = read_fidelity_positions_str(SYNTHETIC_STR).unwrap();
        let aapl = rows.iter().find(|r| r.symbol == "AAPL").unwrap();
        let pct = aapl
            .percent_of_account
            .expect("AAPL percent_of_account must be Some");
        assert!(
            (pct - 71.23).abs() < 0.001,
            "expected 71.23 (raw percent), got {} — do not divide by 100",
            pct
        );
    }

    /// The "--" sentinel used by Fidelity for unavailable data must map to None.
    #[test]
    fn dash_dash_sentinel_parses_as_none() {
        let rows = read_fidelity_positions_str(SYNTHETIC_STR).unwrap();
        let spaxx = rows.iter().find(|r| r.symbol == "SPAXX").unwrap();
        assert!(
            spaxx.cost_basis_total.is_none(),
            "SPAXX cost_basis_total should be None for '--', got {:?}",
            spaxx.cost_basis_total
        );
        assert!(
            spaxx.average_cost_basis.is_none(),
            "SPAXX average_cost_basis should be None for '--', got {:?}",
            spaxx.average_cost_basis
        );
    }

    /// FCASH** rows have a blank Quantity field; the parser must yield 0.0,
    /// preserve current_value, and not emit NaN for last_price.
    #[test]
    fn fcash_with_blank_quantity_yields_zero_quantity() {
        let rows = read_fidelity_positions_str(SYNTHETIC_STR).unwrap();
        let fcash = rows.iter().find(|r| r.symbol == "FCASH**").unwrap();
        assert!(
            (fcash.quantity - 0.0).abs() < f64::EPSILON,
            "FCASH** quantity must be 0.0, got {}",
            fcash.quantity
        );
        assert!(
            (fcash.current_value - 100.0).abs() < 0.01,
            "FCASH** current_value must be 100.0, got {}",
            fcash.current_value
        );
        assert!(
            !fcash.last_price.is_nan(),
            "FCASH** last_price must not be NaN"
        );
    }

    // ── File I/O ──────────────────────────────────────────────────────────────

    /// Reading synthetic.csv from disk must return 3 data rows.
    #[test]
    fn read_fidelity_positions_csv_loads_synthetic_fixture() {
        let path = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/fidelity/synthetic.csv"
        ));
        let rows = read_fidelity_positions_csv(path).unwrap();
        assert_eq!(
            rows.len(),
            3,
            "expected 3 rows (FCASH**, AAPL, SPAXX), got {}",
            rows.len()
        );
    }

    /// CRLF line endings (Windows style) must parse identically to LF.
    #[test]
    fn read_fidelity_positions_str_handles_unix_and_windows_line_endings() {
        // Build CRLF variant from LF-based SYNTHETIC_STR.
        let crlf_version = SYNTHETIC_STR.replace('\n', "\r\n");
        let rows_lf = read_fidelity_positions_str(SYNTHETIC_STR).unwrap();
        let rows_crlf = read_fidelity_positions_str(&crlf_version).unwrap();
        assert_eq!(
            rows_lf.len(),
            rows_crlf.len(),
            "row count must be equal regardless of line ending"
        );
        for (lf, crlf) in rows_lf.iter().zip(rows_crlf.iter()) {
            assert_eq!(lf.symbol, crlf.symbol);
            assert_eq!(lf.quantity, crlf.quantity);
            assert_eq!(lf.current_value, crlf.current_value);
        }
    }

    // ── Roll-up to Portfolio ──────────────────────────────────────────────────

    /// Single-account input with 3 distinct symbols → 3 positions, no merging.
    #[test]
    fn roll_up_single_account_one_position_per_symbol() {
        let rows = read_fidelity_positions_str(SYNTHETIC_STR).unwrap();
        let portfolio = roll_up_to_portfolio(&rows, as_of());
        assert_eq!(
            portfolio.positions.len(),
            3,
            "expected 3 positions, got {}",
            portfolio.positions.len()
        );
    }

    /// The same symbol appearing in two accounts must be summed into one Position.
    /// Concrete: AAPL 100sh @ $282.46 in ACCT_A + 230sh @ $282.46 in ACCT_B = 330sh.
    #[test]
    fn roll_up_multi_account_sums_same_symbol() {
        // Build two rows for AAPL from different accounts.
        use crate::portfolio::import::FidelityPositionRow;
        let row_a = FidelityPositionRow {
            account_number: "ACCT_A".to_string(),
            account_name: "Account A".to_string(),
            symbol: "AAPL".to_string(),
            description: "APPLE INC".to_string(),
            quantity: 100.0,
            last_price: 282.46,
            current_value: 28246.0,
            cost_basis_total: Some(20000.0),
            average_cost_basis: Some(200.0),
            percent_of_account: Some(50.0),
            position_type: "Cash".to_string(),
        };
        let row_b = FidelityPositionRow {
            account_number: "ACCT_B".to_string(),
            account_name: "Account B".to_string(),
            symbol: "AAPL".to_string(),
            description: "APPLE INC".to_string(),
            quantity: 230.0,
            last_price: 282.46,
            current_value: 65965.8,
            cost_basis_total: Some(46000.0),
            average_cost_basis: Some(200.0),
            percent_of_account: Some(50.0),
            position_type: "Cash".to_string(),
        };
        let rows = vec![row_a, row_b];
        let portfolio = roll_up_to_portfolio(&rows, as_of());
        assert_eq!(
            portfolio.positions.len(),
            1,
            "must merge AAPL into single position"
        );
        let aapl = portfolio.position("AAPL").unwrap();
        assert!(
            (aapl.total_shares() - 330.0).abs() < 0.01,
            "expected 330.0 shares, got {}",
            aapl.total_shares()
        );
        assert!(
            (aapl.market_value() - (28246.0 + 65965.8)).abs() < 0.01,
            "expected summed market value, got {}",
            aapl.market_value()
        );
    }

    /// The Portfolio's `as_of` field must equal the argument passed to roll_up_to_portfolio.
    #[test]
    fn roll_up_sets_as_of_field() {
        let rows = read_fidelity_positions_str(SYNTHETIC_STR).unwrap();
        let target_date = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        let portfolio = roll_up_to_portfolio(&rows, target_date);
        assert_eq!(portfolio.as_of, target_date);
    }

    /// FCASH** with quantity=0 must be represented via Option (i):
    /// store quantity=current_value at last_price=1.0 so market_value() is preserved.
    ///
    /// Concretely: FCASH** has current_value=100.00.
    /// The implementer MUST store it so portfolio.position("FCASH**").market_value()
    /// is within $0.01 of 100.00.  The simplest correct representation is
    /// quantity=100.00 shares at price=1.0 (matching Python prototype).
    #[test]
    fn roll_up_fcash_position_quantity_zero_value_preserved() {
        let rows = read_fidelity_positions_str(SYNTHETIC_STR).unwrap();
        let portfolio = roll_up_to_portfolio(&rows, as_of());
        let fcash = portfolio
            .position("FCASH**")
            .expect("FCASH** must appear as a Position");
        assert!(
            (fcash.market_value() - 100.0).abs() < 0.01,
            "FCASH** market_value must be ~100.0, got {}",
            fcash.market_value()
        );
    }

    // ── Synthetic example fixtures ────────────────────────────────────────────

    fn fixture_path(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/fidelity")
            .join(name)
    }

    /// The inline string constant must stay byte-identical to the fixture file.
    #[test]
    fn synthetic_str_matches_fixture_file() {
        let on_disk = std::fs::read_to_string(fixture_path("synthetic.csv")).unwrap();
        assert_eq!(SYNTHETIC_STR, on_disk);
    }

    /// example_main.csv must parse without error and yield exactly 7 rows
    /// (6 positions + 1 FCASH**).
    #[test]
    fn parse_example_main_fidelity_csv_succeeds() {
        let rows = read_fidelity_positions_csv(&fixture_path("example_main.csv")).unwrap();
        assert_eq!(
            rows.len(),
            7,
            "expected 7 rows in example_main, got {}",
            rows.len()
        );
        assert!(rows.iter().all(|r| r.account_number == "EX0000001"));
    }

    /// Reading both example account CSVs and rolling up must produce exactly 10
    /// unique symbols and a total market value of $1,043,770.00.
    ///
    /// Unique symbols: AAPL FCASH** IAU IEF MSFT NVDA SPAXX VOO VWO VXUS.
    #[test]
    fn roll_up_example_two_accounts_yields_10_unique_symbols() {
        let mut rows = read_fidelity_positions_csv(&fixture_path("example_main.csv")).unwrap();
        rows.extend(read_fidelity_positions_csv(&fixture_path("example_small.csv")).unwrap());

        let portfolio = roll_up_to_portfolio(&rows, as_of());

        assert_eq!(
            portfolio.positions.len(),
            10,
            "expected 10 unique symbols after roll-up, got {}",
            portfolio.positions.len()
        );

        let total_mv = portfolio.total_market_value();
        assert!(
            (total_mv - 1_043_770.00_f64).abs() < 0.01,
            "total market value must be $1,043,770.00, got {:.2}",
            total_mv
        );
        let msft = portfolio.position("MSFT").unwrap();
        assert!((msft.total_shares() - 1300.0).abs() < 1e-9);
        assert!((msft.market_value() - 546_000.0).abs() < 0.01);
    }

    // ── Auto-detect (spec §1) ─────────────────────────────────────────────────

    const SIMPLE_CSV: &str = "symbol,quantity,price,cost_basis,account,description\n\
        MSFT,1300,420.00,185.00,ACCT1,MICROSOFT CORP\n\
        VOO,250,600.00,420.00,ACCT1,VANGUARD S&P 500 ETF\n";

    fn row<'a>(rows: &'a [FidelityPositionRow], sym: &str) -> &'a FidelityPositionRow {
        rows.iter()
            .find(|r| r.symbol == sym)
            .unwrap_or_else(|| panic!("no row {sym}"))
    }

    #[test]
    fn auto_detects_fidelity_export_and_matches_fidelity_parser() {
        let auto = read_positions_auto_str(SYNTHETIC_STR).unwrap();
        let direct = read_fidelity_positions_str(SYNTHETIC_STR).unwrap();
        assert_eq!(auto, direct);
        assert_eq!(auto.len(), 3);
    }

    #[test]
    fn auto_detects_fidelity_export_without_bom() {
        let no_bom = SYNTHETIC_STR.trim_start_matches('\u{feff}');
        let auto = read_positions_auto_str(no_bom).unwrap();
        assert_eq!(auto, read_fidelity_positions_str(no_bom).unwrap());
    }

    #[test]
    fn auto_detects_simple_format_and_maps_fields() {
        let rows = read_positions_auto_str(SIMPLE_CSV).unwrap();
        assert_eq!(rows.len(), 2);
        let msft = row(&rows, "MSFT");
        assert!((msft.quantity - 1300.0).abs() < 1e-9);
        assert!((msft.last_price - 420.0).abs() < 1e-9);
        assert!(
            (msft.current_value - 546_000.0).abs() < 1e-6,
            "current_value = quantity*price"
        );
        assert_eq!(msft.account_number, "ACCT1");
        assert_eq!(msft.description, "MICROSOFT CORP");
    }

    #[test]
    fn simple_format_cost_basis_is_per_share_and_total_is_quantity_times_it() {
        let rows = read_positions_auto_str(SIMPLE_CSV).unwrap();
        let msft = row(&rows, "MSFT");
        let total = msft
            .cost_basis_total
            .expect("cost_basis_total must be Some");
        assert!(
            (total - 240_500.0).abs() < 1e-6,
            "1300 * 185 = 240,500, got {total}"
        );
        let voo = row(&rows, "VOO");
        assert!((voo.cost_basis_total.unwrap() - 105_000.0).abs() < 1e-6);
    }

    #[test]
    fn simple_format_without_cost_basis_yields_none() {
        let rows = read_positions_auto_str("symbol,quantity,price\nMSFT,10,420\n").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].cost_basis_total, None);
    }

    #[test]
    fn simple_format_roll_up_preserves_value_and_per_share_basis() {
        let rows = read_positions_auto_str(SIMPLE_CSV).unwrap();
        let portfolio = roll_up_to_portfolio(&rows, as_of());
        assert!((portfolio.total_market_value() - (546_000.0 + 150_000.0)).abs() < 1e-6);
        let msft = portfolio.position("MSFT").unwrap();
        assert!((msft.lots[0].cost_basis_per_share - 185.0).abs() < 1e-9);
    }

    #[test]
    fn simple_format_handles_bom() {
        let with_bom = format!("\u{feff}{}", SIMPLE_CSV);
        let rows = read_positions_auto_str(&with_bom).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[0].symbol, "MSFT",
            "BOM must not leak into the first header/field"
        );
    }

    #[test]
    fn simple_format_headers_are_case_insensitive_and_trimmed() {
        let csv = " Symbol , QUANTITY ,Price , Cost_Basis \nMSFT,10,100,50\n";
        let rows = read_positions_auto_str(csv).unwrap();
        assert_eq!(rows.len(), 1);
        assert!((rows[0].current_value - 1000.0).abs() < 1e-9);
        assert!((rows[0].cost_basis_total.unwrap() - 500.0).abs() < 1e-9);
    }

    #[test]
    fn simple_format_accepts_dollar_prefix_and_thousands_separators() {
        let csv = "symbol,quantity,price,cost_basis\nMSFT,\"1,300\",\"$1,420.50\",\"$1,185.25\"\n";
        let rows = read_positions_auto_str(csv).unwrap();
        let r = &rows[0];
        assert!((r.quantity - 1300.0).abs() < 1e-9);
        assert!((r.last_price - 1420.50).abs() < 1e-9);
        assert!((r.current_value - 1300.0 * 1420.50).abs() < 1e-6);
        assert!((r.cost_basis_total.unwrap() - 1300.0 * 1185.25).abs() < 1e-6);
    }

    #[test]
    fn simple_format_skips_blank_and_comment_lines() {
        let csv = "symbol,quantity,price\n\n# my notes\nMSFT,10,100\n\n   \n# another\nVOO,5,600\n";
        let rows = read_positions_auto_str(csv).unwrap();
        let syms: Vec<&str> = rows.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(syms, vec!["MSFT", "VOO"]);
    }

    /// Assumption: a `#` comment line is also skipped before the header row.
    #[test]
    fn simple_format_skips_leading_comment_lines_before_header() {
        let csv = "# exported by hand\n# second note\nsymbol,quantity,price\nMSFT,10,100\n";
        let rows = read_positions_auto_str(csv).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].symbol, "MSFT");
    }

    #[test]
    fn simple_format_header_only_yields_empty_vec() {
        let rows = read_positions_auto_str("symbol,quantity,price\n").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn simple_format_missing_required_column_names_it_and_lists_accepted_formats() {
        for (csv, missing) in [
            ("symbol,quantity\nMSFT,10\n", "price"),
            ("symbol,price\nMSFT,10\n", "quantity"),
            ("quantity,price\n10,100\n", "symbol"),
        ] {
            let err = read_positions_auto_str(csv).expect_err("must fail");
            let msg = err.to_string();
            assert!(
                msg.contains(missing),
                "message should name `{missing}`: {msg}"
            );
            assert!(
                msg.to_lowercase().contains("fidelity"),
                "message should list the Fidelity format as accepted: {msg}"
            );
            assert!(
                msg.to_lowercase().contains("simple") || msg.contains("symbol,quantity,price"),
                "message should list the simple format as accepted: {msg}"
            );
        }
    }

    #[test]
    fn simple_format_unparseable_number_is_an_error() {
        let result = read_positions_auto_str("symbol,quantity,price\nMSFT,abc,100\n");
        assert!(result.is_err());
    }

    #[test]
    fn read_positions_auto_reads_fidelity_file() {
        let rows = read_positions_auto(&fixture_path("example_main.csv")).unwrap();
        assert_eq!(rows.len(), 7);
    }

    #[test]
    fn read_positions_auto_reads_simple_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("pos.csv");
        std::fs::write(&path, SIMPLE_CSV).unwrap();
        let rows = read_positions_auto(&path).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn read_positions_auto_error_includes_path_for_missing_column() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("bad_positions.csv");
        std::fs::write(&path, "symbol,quantity\nMSFT,10\n").unwrap();
        let msg = read_positions_auto(&path)
            .expect_err("must fail")
            .to_string();
        assert!(
            msg.contains("bad_positions.csv"),
            "error must include the path: {msg}"
        );
        assert!(
            msg.contains("price"),
            "error must still name the column: {msg}"
        );
    }

    #[test]
    fn read_positions_auto_error_includes_path_for_missing_file() {
        let path = Path::new("/nonexistent_dir_for_ghost_alpha/positions_xyz.csv");
        let msg = read_positions_auto(path)
            .expect_err("must fail")
            .to_string();
        assert!(
            msg.contains("positions_xyz.csv"),
            "error must include the path: {msg}"
        );
    }

    // ═════════════════════════════════════════════════════════════════════════
    // Round 2 regressions (M1, M2, M3)
    // ═════════════════════════════════════════════════════════════════════════

    // ── M1: tickers upper-cased by both parsers ───────────────────────────

    #[test]
    fn m1_simple_format_upper_cases_symbols() {
        let rows =
            read_positions_auto_str("symbol,quantity,price\nmsft,10,420\nVoo,5,600\n").unwrap();
        let syms: Vec<&str> = rows.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(syms, vec!["MSFT", "VOO"]);
    }

    #[test]
    fn m1_fidelity_parser_upper_cases_symbols() {
        let lower = SYNTHETIC_STR.replace(",AAPL,", ",aapl,");
        let rows = read_fidelity_positions_str(&lower).unwrap();
        assert!(
            rows.iter().any(|r| r.symbol == "AAPL"),
            "{:?}",
            rows.iter().map(|r| &r.symbol).collect::<Vec<_>>()
        );
        assert!(rows.iter().all(|r| r.symbol == r.symbol.to_uppercase()));
        let rows = read_positions_auto_str(&lower).unwrap();
        assert!(rows.iter().any(|r| r.symbol == "AAPL"));
    }

    #[test]
    fn m1_simple_format_symbols_are_trimmed_too() {
        let rows = read_positions_auto_str("symbol,quantity,price\n  msft ,10,420\n").unwrap();
        assert_eq!(rows[0].symbol, "MSFT");
    }

    // ── M2: Fidelity columns looked up by header name ─────────────────────

    #[test]
    fn m2_simple_csv_with_account_number_column_is_not_misread_as_fidelity() {
        let csv = "symbol,quantity,price,account number\nMSFT,10,420,ACCT9\nVOO,5,600,ACCT9\n";
        let rows = read_positions_auto_str(csv)
            .expect("no `current value`/`last price` columns ⇒ must parse as the simple format");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].symbol, "MSFT");
        assert!((rows[0].quantity - 10.0).abs() < 1e-9);
        assert!((rows[0].last_price - 420.0).abs() < 1e-9);
        assert!((rows[0].current_value - 4_200.0).abs() < 1e-9);
    }

    const REORDERED_FIDELITY: &str = "\u{feff}Symbol,Quantity,Account Number,Current Value,Last Price,\
Cost Basis Total,Average Cost Basis,Account Name,Description,Type,Percent Of Account\n\
FCASH**,,EX0000009,$100.00,,,,Individual,HELD IN FCASH,Cash,0.28%\n\
AAPL,100,EX0000009,$25000.00,$250.00,$15000.00,$150.00,Individual,APPLE INC,Cash,71.23%\n\
SPAXX,10000,EX0000009,$10000.00,$1.00,--,--,Individual,FIDELITY GOVERNMENT MONEY MARKET,Cash,28.49%\n";

    #[test]
    fn m2_reordered_fidelity_columns_parse_by_header_name() {
        for rows in [
            read_fidelity_positions_str(REORDERED_FIDELITY).unwrap(),
            read_positions_auto_str(REORDERED_FIDELITY).unwrap(),
        ] {
            assert_eq!(rows.len(), 3);
            let aapl = row(&rows, "AAPL");
            assert_eq!(aapl.account_number, "EX0000009");
            assert_eq!(aapl.account_name, "Individual");
            assert_eq!(aapl.description, "APPLE INC");
            assert!((aapl.quantity - 100.0).abs() < 1e-9);
            assert!((aapl.last_price - 250.0).abs() < 1e-9);
            assert!((aapl.current_value - 25_000.0).abs() < 1e-9);
            assert!((aapl.cost_basis_total.unwrap() - 15_000.0).abs() < 1e-9);
            assert!((aapl.average_cost_basis.unwrap() - 150.0).abs() < 1e-9);
            assert!((aapl.percent_of_account.unwrap() - 71.23).abs() < 1e-9);
            assert_eq!(aapl.position_type, "Cash");
            let fcash = row(&rows, "FCASH**");
            assert_eq!(fcash.quantity, 0.0);
            assert!((fcash.current_value - 100.0).abs() < 1e-9);
            let spaxx = row(&rows, "SPAXX");
            assert_eq!(spaxx.cost_basis_total, None);
            assert!((spaxx.current_value - 10_000.0).abs() < 1e-9);
        }
    }

    #[test]
    fn m2_reordered_fidelity_roll_up_nav_is_correct() {
        let rows = read_positions_auto_str(REORDERED_FIDELITY).unwrap();
        let p = roll_up_to_portfolio(&rows, as_of());
        assert!(
            (p.total_market_value() - 35_100.0).abs() < 0.01,
            "{}",
            p.total_market_value()
        );
    }

    // ── M3: non-cash rows with blank quantity are skipped ─────────────────

    #[test]
    fn m3_pending_activity_row_is_skipped() {
        let pending = "EX0000003,Individual,Pending Activity,,,,,$12.34,,,,,0.04%,,,Cash,\n";
        let csv = SYNTHETIC_STR.replacen(
            "EX0000003,Individual,AAPL",
            &format!("{pending}EX0000003,Individual,AAPL"),
            1,
        );
        assert!(csv.contains("Pending Activity"));
        for rows in [
            read_fidelity_positions_str(&csv).unwrap(),
            read_positions_auto_str(&csv).unwrap(),
        ] {
            assert_eq!(
                rows.len(),
                3,
                "{:?}",
                rows.iter().map(|r| &r.symbol).collect::<Vec<_>>()
            );
            assert!(rows
                .iter()
                .all(|r| !r.symbol.to_lowercase().contains("pending")));
            let p = roll_up_to_portfolio(&rows, as_of());
            assert!((p.total_market_value() - 35_100.0).abs() < 0.01);
        }
    }

    #[test]
    fn m3_fcash_double_star_rows_with_blank_quantity_are_still_kept() {
        let rows = read_fidelity_positions_str(SYNTHETIC_STR).unwrap();
        assert!(rows
            .iter()
            .any(|r| r.symbol == "FCASH**" && (r.current_value - 100.0).abs() < 0.01));
    }

    #[test]
    fn m3_other_blank_quantity_non_cash_symbol_is_skipped() {
        let odd = "EX0000003,Individual,Account Total,,,,,$999.00,,,,,,,,Cash,\n";
        let csv = SYNTHETIC_STR.replacen(
            "EX0000003,Individual,AAPL",
            &format!("{odd}EX0000003,Individual,AAPL"),
            1,
        );
        let rows = read_fidelity_positions_str(&csv).unwrap();
        assert_eq!(rows.len(), 3);
    }

    // ═══ Round 3 ═══

    #[test]
    fn r3_fidelity_export_without_quantity_column_is_an_error_naming_it() {
        let csv = "Account Number,Account Name,Symbol,Last Price,Current Value\n\
                   EX0000003,Individual,AAPL,$250.00,$25000.00\n";
        for res in [
            read_fidelity_positions_str(csv),
            read_positions_auto_str(csv),
        ] {
            let err = res.expect_err("must not silently return an empty/zero-quantity portfolio");
            assert!(err.to_string().to_lowercase().contains("quantity"), "{err}");
        }
    }

    #[test]
    fn r3_negative_quantity_error_mentions_shorts_not_supported() {
        let err = read_positions_auto_str("symbol,quantity,price\nMSFT,-5,420\n")
            .expect_err("negative quantity");
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("short") || msg.contains("not supported"),
            "{msg}"
        );
    }
}
