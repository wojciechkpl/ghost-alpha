//! Integration tests for CSV emitters and audit-JSON round-trip (Step 7, Gap 8a).
//!
//! These tests verify schema compatibility with the Python prototype fixtures
//! in `tests/fixtures/python_mc/`. Statistical equivalence with the Python
//! output is deferred to Step 8.

use ghost_alpha::projection::monte_carlo::YearPercentiles;
use ghost_alpha::projection::output::{
    build_mc_audit_block, write_nav_projection_csv, McAuditBlock, NavProjectionMcRow,
};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use tempfile::NamedTempFile;

/// The Python reference fixture keeps its historical `amzn_*` headers (it is
/// generated for a single concentrated ticker, MSFT in the synthetic data).
const PYTHON_NAV_CSV_RAW: &str = include_str!("fixtures/python_mc/nav_projection.csv");

/// Rust emits the generalized `concentrated_pct_*` headers. Map the Python
/// header line onto them; data rows are untouched.
fn python_nav_csv_mapped() -> String {
    let (header, rest) = PYTHON_NAV_CSV_RAW
        .split_once('\n')
        .expect("fixture has a header");
    format!(
        "{}\n{}",
        header.replace("amzn_pct_", "concentrated_pct_"),
        rest
    )
}

// ---- Test 5: header of write_nav_projection_csv matches Python fixture ------

#[test]
fn nav_projection_csv_header_matches_python_fixture() {
    // Python header with amzn_pct_* mapped to concentrated_pct_*.
    let mapped = python_nav_csv_mapped();
    let python_header = mapped.lines().next().unwrap();
    assert!(
        PYTHON_NAV_CSV_RAW
            .lines()
            .next()
            .unwrap()
            .contains("amzn_pct_p05"),
        "fixture is expected to keep its amzn_* headers"
    );

    // Write an empty row slice — the csv writer still emits the header.
    let tmp = NamedTempFile::new().unwrap();
    write_nav_projection_csv(tmp.path(), &[]).unwrap();

    let file = std::fs::File::open(tmp.path()).unwrap();
    let rust_header = BufReader::new(file).lines().next().unwrap().unwrap();

    assert_eq!(rust_header, python_header,
        "Rust CSV header does not match Python fixture.\n  rust  : {rust_header}\n  python: {python_header}");
}

// ---- Test 6: full round-trip of Python fixture within 1e-2 tolerance -------

#[test]
fn nav_projection_csv_row_round_trip_matches_python_within_tolerance() {
    // Deserialize all rows from the Python fixture.
    let mapped = python_nav_csv_mapped();
    let mut rdr = csv::Reader::from_reader(mapped.as_bytes());
    let py_rows: Vec<NavProjectionMcRow> = rdr
        .deserialize()
        .map(|r| r.expect("deserialize Python fixture row"))
        .collect();
    assert!(
        !py_rows.is_empty(),
        "Python fixture must contain at least one data row"
    );

    // Write the rows back out with the Rust emitter.
    let tmp = NamedTempFile::new().unwrap();
    write_nav_projection_csv(tmp.path(), &py_rows).unwrap();

    // Re-read from the temp file.
    let mut rdr2 = csv::Reader::from_path(tmp.path()).unwrap();
    let rt_rows: Vec<NavProjectionMcRow> = rdr2
        .deserialize()
        .map(|r| r.expect("deserialize round-trip row"))
        .collect();

    assert_eq!(
        py_rows.len(),
        rt_rows.len(),
        "row count mismatch after round-trip"
    );

    const TOL: f64 = 1e-2;
    for (i, (py, rt)) in py_rows.iter().zip(rt_rows.iter()).enumerate() {
        assert_eq!(py.rsu_treatment, rt.rsu_treatment, "row {i} rsu_treatment");
        assert_eq!(py.year, rt.year, "row {i} year");
        assert_eq!(py.as_of, rt.as_of, "row {i} as_of");
        assert_eq!(py.n_paths, rt.n_paths, "row {i} n_paths");

        macro_rules! within_tol {
            ($field:ident) => {
                assert!(
                    (py.$field - rt.$field).abs() <= TOL,
                    "row {i} {} differs: py={} rt={}",
                    stringify!($field),
                    py.$field,
                    rt.$field
                );
            };
        }
        within_tol!(nav_p05);
        within_tol!(nav_p25);
        within_tol!(nav_p50);
        within_tol!(nav_p75);
        within_tol!(nav_p95);
        within_tol!(nav_mean);
        within_tol!(concentrated_pct_p05);
        within_tol!(concentrated_pct_p25);
        within_tol!(concentrated_pct_p50);
        within_tol!(concentrated_pct_p75);
        within_tol!(concentrated_pct_p95);
    }
}

// ---- Test 7: McAuditBlock JSON round-trip -----------------------------------

#[test]
fn mc_audit_block_round_trips_through_json() {
    fn yp(year: u32, mean: f64, qs: [f64; 5]) -> YearPercentiles {
        YearPercentiles {
            year,
            n_paths: 5000,
            mean,
            percentiles: vec![
                (5, qs[0]),
                (25, qs[1]),
                (50, qs[2]),
                (75, qs[3]),
                (95, qs[4]),
            ],
        }
    }
    let mut nav: BTreeMap<String, Vec<YearPercentiles>> = BTreeMap::new();
    nav.insert(
        "HOLD".into(),
        vec![yp(
            5,
            4_100_000.0 * (1.0 + 0.09519_f64).powi(5),
            [
                2_100_000.0,
                3_000_000.0,
                3_896_664.78,
                5_200_000.0,
                7_800_000.0,
            ],
        )],
    );
    let mut conc: BTreeMap<String, Vec<YearPercentiles>> = BTreeMap::new();
    conc.insert(
        "HOLD".into(),
        vec![yp(5, 65.0, [55.2, 61.1, 65.4, 69.8, 75.3])],
    );

    let original = build_mc_audit_block(
        42,
        5000,
        "mc_01523".into(),
        0, // horizon index into the single-element vectors above
        &["HOLD".to_string()],
        &nav,
        &conc,
    );

    let json_str = serde_json::to_string_pretty(&original).unwrap();
    let roundtripped: McAuditBlock = serde_json::from_str(&json_str).unwrap();
    assert_eq!(original, roundtripped);

    assert!(
        json_str.contains("year5_concentrated_pct_percentiles"),
        "audit JSON must use the generalized key:\n{json_str}"
    );
    assert!(
        !json_str.contains("amzn"),
        "no amzn-named keys:\n{json_str}"
    );

    // Spot-check that BTreeMap ordering is deterministic: p05 must precede p50 in the JSON.
    assert!(
        json_str.find("\"p05\"").unwrap() < json_str.find("\"p50\"").unwrap(),
        "BTreeMap must produce deterministic key ordering (p05 before p50)"
    );
}
