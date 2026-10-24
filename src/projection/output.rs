//! CSV / audit-JSON emitters for the MC projection. Bootstrap stub for Gap 1.
//!
//! Schema follows the Python prototype's output (cross-language fixtures in
//! `tests/fixtures/python_mc/`), except that the prototype's `amzn_*` columns
//! are generalized to `concentrated_*`. See `docs/PLAN_PIPELINE.md`.

use crate::projection::monte_carlo::YearPercentiles;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// One aggregated row of nav_projection.csv (MC mode).
/// The concentrated-position columns generalize the prototype's `amzn_*` names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NavProjectionMcRow {
    pub rsu_treatment: String,
    pub year: u32,
    pub as_of: String,
    pub n_paths: usize,
    pub nav_p05: f64,
    pub nav_p25: f64,
    pub nav_p50: f64,
    pub nav_p75: f64,
    pub nav_p95: f64,
    pub nav_mean: f64,
    pub concentrated_pct_p05: f64,
    pub concentrated_pct_p25: f64,
    pub concentrated_pct_p50: f64,
    pub concentrated_pct_p75: f64,
    pub concentrated_pct_p95: f64,
}

/// One row of nav_mc_paths.csv (raw long format).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NavMcPathRow {
    pub scenario: String,
    pub rsu_treatment: String,
    pub year: u32,
    pub as_of: String,
    pub nav: f64,
    pub concentrated_value: f64,
    pub concentrated_pct_of_nav: f64,
}

/// Audit-JSON `mc` block. Schema mirrors Python prototype exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McAuditBlock {
    pub n_paths: usize,
    pub seed: u64,
    pub representative_scenario_id: String,
    /// treatment -> {"p05": $, "p25": $, ..., "mean": $}
    pub year5_nav_percentiles: BTreeMap<String, BTreeMap<String, f64>>,
    /// treatment -> {"p05": %, "p25": %, ...}
    pub year5_concentrated_pct_percentiles: BTreeMap<String, BTreeMap<String, f64>>,
}

/// Build aggregated rows from nav + concentrated-pct YearPercentiles per treatment.
///
/// `treatments` is the canonical ordering (typically HOLD, SELL_REINVEST, OPTION_B_UNWIND).
/// `as_of_per_year[y]` is the ISO date label for year y (length = HORIZON_YEARS + 1).
pub fn build_nav_projection_rows(
    treatments: &[String],
    nav_pcts: &BTreeMap<String, Vec<YearPercentiles>>,
    concentrated_pcts: &BTreeMap<String, Vec<YearPercentiles>>,
    as_of_per_year: &[String],
) -> Vec<NavProjectionMcRow> {
    let mut rows = Vec::new();
    for tr in treatments {
        let nav_years = &nav_pcts[tr];
        let conc_years = &concentrated_pcts[tr];
        for (y, (nav_yp, conc_yp)) in nav_years.iter().zip(conc_years.iter()).enumerate() {
            rows.push(NavProjectionMcRow {
                rsu_treatment: tr.clone(),
                year: y as u32,
                as_of: as_of_per_year[y].clone(),
                n_paths: nav_yp.n_paths,
                nav_p05: pct(nav_yp, 5),
                nav_p25: pct(nav_yp, 25),
                nav_p50: pct(nav_yp, 50),
                nav_p75: pct(nav_yp, 75),
                nav_p95: pct(nav_yp, 95),
                nav_mean: nav_yp.mean,
                concentrated_pct_p05: pct(conc_yp, 5),
                concentrated_pct_p25: pct(conc_yp, 25),
                concentrated_pct_p50: pct(conc_yp, 50),
                concentrated_pct_p75: pct(conc_yp, 75),
                concentrated_pct_p95: pct(conc_yp, 95),
            });
        }
    }
    rows
}

/// Extract percentile `q` from a `YearPercentiles`, panicking if absent.
fn pct(yp: &YearPercentiles, q: u8) -> f64 {
    yp.percentiles
        .iter()
        .find(|(qq, _)| *qq == q)
        .map(|(_, v)| *v)
        .unwrap_or_else(|| panic!("YearPercentiles missing p{:02}", q))
}

/// Column names for `NavProjectionMcRow` in declaration order.
/// Used to force-emit a header even when the row slice is empty.
const NAV_PROJECTION_HEADERS: &[&str] = &[
    "rsu_treatment",
    "year",
    "as_of",
    "n_paths",
    "nav_p05",
    "nav_p25",
    "nav_p50",
    "nav_p75",
    "nav_p95",
    "nav_mean",
    "concentrated_pct_p05",
    "concentrated_pct_p25",
    "concentrated_pct_p50",
    "concentrated_pct_p75",
    "concentrated_pct_p95",
];

/// Write rows to `path` as CSV with header.
///
/// The header is always emitted, even when `rows` is empty, so that
/// downstream consumers can rely on the schema being present.
pub fn write_nav_projection_csv(
    path: &Path,
    rows: &[NavProjectionMcRow],
) -> Result<(), csv::Error> {
    let mut wtr = csv::WriterBuilder::new()
        .has_headers(false)
        .from_path(path)?;
    wtr.write_record(NAV_PROJECTION_HEADERS)?;
    for row in rows {
        wtr.serialize(row)?;
    }
    wtr.flush()?;
    Ok(())
}

pub fn write_nav_mc_paths_csv(path: &Path, rows: &[NavMcPathRow]) -> Result<(), csv::Error> {
    let mut wtr = csv::Writer::from_path(path)?;
    for row in rows {
        wtr.serialize(row)?;
    }
    wtr.flush()?;
    Ok(())
}

/// Build the audit block from final-year (`HORIZON_YEARS` index) percentiles.
///
/// `nav_pcts[tr][HORIZON_YEARS]` provides the percentiles for the treatment.
pub fn build_mc_audit_block(
    seed: u64,
    n_paths: usize,
    representative_scenario_id: String,
    horizon_years: u32,
    treatments: &[String],
    nav_pcts: &BTreeMap<String, Vec<YearPercentiles>>,
    concentrated_pcts: &BTreeMap<String, Vec<YearPercentiles>>,
) -> McAuditBlock {
    let h = horizon_years as usize;
    let mut nav_block = BTreeMap::new();
    let mut conc_block = BTreeMap::new();
    for tr in treatments {
        let nav_yp = &nav_pcts[tr][h];
        let conc_yp = &concentrated_pcts[tr][h];

        let mut nav_map = BTreeMap::new();
        for (q, v) in &nav_yp.percentiles {
            nav_map.insert(format!("p{:02}", q), *v);
        }
        nav_map.insert("mean".to_string(), nav_yp.mean);
        nav_block.insert(tr.clone(), nav_map);

        let mut conc_map = BTreeMap::new();
        for (q, v) in &conc_yp.percentiles {
            conc_map.insert(format!("p{:02}", q), *v);
        }
        conc_block.insert(tr.clone(), conc_map);
    }
    McAuditBlock {
        n_paths,
        seed,
        representative_scenario_id,
        year5_nav_percentiles: nav_block,
        year5_concentrated_pct_percentiles: conc_block,
    }
}

// ---------------------------------------------------------------------------
// Unit tests (RED phase — all call unimplemented functions and will panic)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projection::monte_carlo::YearPercentiles;
    use std::io::BufRead;
    use tempfile::NamedTempFile;

    /// Convenience constructor for a YearPercentiles with the standard quintet.
    #[allow(clippy::too_many_arguments)]
    fn make_yp(
        year: u32,
        n_paths: usize,
        mean: f64,
        p05: f64,
        p25: f64,
        p50: f64,
        p75: f64,
        p95: f64,
    ) -> YearPercentiles {
        YearPercentiles {
            year,
            n_paths,
            mean,
            percentiles: vec![(5, p05), (25, p25), (50, p50), (75, p75), (95, p95)],
        }
    }

    // ---- Test 1: NavProjectionMcRow column names (concentrated_* generalization of the Python amzn_* names) ---------

    #[test]
    fn nav_projection_row_serializes_with_generalized_column_names() {
        let tmp = NamedTempFile::new().unwrap();
        let mut wtr = csv::Writer::from_path(tmp.path()).unwrap();
        let row = NavProjectionMcRow {
            rsu_treatment: "HOLD".into(),
            year: 0,
            as_of: "2026-05-01".into(),
            n_paths: 5000,
            nav_p05: 1.0,
            nav_p25: 2.0,
            nav_p50: 3.0,
            nav_p75: 4.0,
            nav_p95: 5.0,
            nav_mean: 3.0,
            concentrated_pct_p05: 60.0,
            concentrated_pct_p25: 61.0,
            concentrated_pct_p50: 62.0,
            concentrated_pct_p75: 63.0,
            concentrated_pct_p95: 64.0,
        };
        wtr.serialize(&row).unwrap();
        wtr.flush().unwrap();
        drop(wtr);

        let file = std::fs::File::open(tmp.path()).unwrap();
        let header = std::io::BufReader::new(file)
            .lines()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(
            header,
            "rsu_treatment,year,as_of,n_paths,nav_p05,nav_p25,nav_p50,nav_p75,nav_p95,\
nav_mean,concentrated_pct_p05,concentrated_pct_p25,concentrated_pct_p50,concentrated_pct_p75,concentrated_pct_p95"
        );
    }

    // ---- Test 2: NavMcPathRow column names (concentrated_* generalization of the Python amzn_* names) ---------------

    #[test]
    fn nav_mc_paths_row_serializes_with_generalized_column_names() {
        let tmp = NamedTempFile::new().unwrap();
        let mut wtr = csv::Writer::from_path(tmp.path()).unwrap();
        let row = NavMcPathRow {
            scenario: "mc_00000".into(),
            rsu_treatment: "HOLD".into(),
            year: 1,
            as_of: "2027-05-01".into(),
            nav: 3_000_000.0,
            concentrated_value: 1_500_000.0,
            concentrated_pct_of_nav: 50.0,
        };
        wtr.serialize(&row).unwrap();
        wtr.flush().unwrap();
        drop(wtr);

        let file = std::fs::File::open(tmp.path()).unwrap();
        let header = std::io::BufReader::new(file)
            .lines()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(
            header,
            "scenario,rsu_treatment,year,as_of,nav,concentrated_value,concentrated_pct_of_nav"
        );
    }

    // ---- Test 3: McAuditBlock JSON keys match Python top-level schema ---------

    #[test]
    fn mc_audit_block_serializes_with_python_top_level_keys() {
        // Built through the public builder (structure-agnostic about field names).
        let block = build_mc_audit_block(
            42,
            5000,
            "mc_01523".into(),
            5,
            &[],
            &BTreeMap::new(),
            &BTreeMap::new(),
        );
        let json_str = serde_json::to_string(&block).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        for key in &[
            "n_paths",
            "seed",
            "representative_scenario_id",
            "year5_nav_percentiles",
            "year5_concentrated_pct_percentiles",
        ] {
            assert!(v.get(key).is_some(), "missing top-level key: {key}");
        }
        assert!(
            v.get("year5_amzn_pct_percentiles").is_none(),
            "old amzn-named audit key must be gone"
        );
    }

    // ---- Test 4: build_nav_projection_rows zips nav and concentrated-pct correctly --------

    #[test]
    fn build_nav_projection_rows_zips_nav_and_concentrated_correctly() {
        // Two years (y=0, y=1) for treatment "HOLD".
        // nav_p50 and concentrated_pct_p50 are deliberately different so a swap is detectable.
        let nav_y0 = make_yp(0, 5000, 1000.0, 800.0, 900.0, 1000.0, 1100.0, 1200.0);
        let nav_y1 = make_yp(1, 5000, 2000.0, 1600.0, 1800.0, 2000.0, 2200.0, 2400.0);
        let amzn_y0 = make_yp(0, 5000, 60.0, 55.0, 58.0, 61.0, 63.0, 66.0);
        let amzn_y1 = make_yp(1, 5000, 65.0, 59.0, 62.0, 65.0, 68.0, 72.0);

        let mut nav_pcts: BTreeMap<String, Vec<YearPercentiles>> = BTreeMap::new();
        nav_pcts.insert("HOLD".into(), vec![nav_y0, nav_y1]);
        let mut amzn_pcts: BTreeMap<String, Vec<YearPercentiles>> = BTreeMap::new();
        amzn_pcts.insert("HOLD".into(), vec![amzn_y0, amzn_y1]);

        let as_of = vec!["2026-05-01".to_string(), "2027-05-01".to_string()];
        let rows = build_nav_projection_rows(&["HOLD".to_string()], &nav_pcts, &amzn_pcts, &as_of);

        assert_eq!(rows.len(), 2);

        assert_eq!(rows[0].rsu_treatment, "HOLD");
        assert_eq!(rows[0].year, 0);
        assert_eq!(rows[0].as_of, "2026-05-01");
        assert!(
            (rows[0].nav_p50 - 1000.0).abs() < 1e-9,
            "year-0 nav_p50 mismatch"
        );
        assert!(
            (rows[0].concentrated_pct_p50 - 61.0).abs() < 1e-9,
            "year-0 concentrated_pct_p50 mismatch"
        );

        assert!(
            (rows[1].nav_p50 - 2000.0).abs() < 1e-9,
            "year-1 nav_p50 mismatch"
        );
        assert!(
            (rows[1].concentrated_pct_p50 - 65.0).abs() < 1e-9,
            "year-1 concentrated_pct_p50 mismatch"
        );
        assert!(
            (rows[1].nav_mean - 2000.0).abs() < 1e-9,
            "year-1 nav_mean mismatch"
        );
    }
}
