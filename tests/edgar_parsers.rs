//! Pure-parser contract for `ghost_alpha::data::edgar` (no HTTP).
//! Fixtures: tests/fixtures/edgar/* (fictional "Example Corp", EXMP, CIK 999999).
//!
//! Required public API (see also the other edgar_* test files):
//!   Cik::{new(u64), parse(&str) -> Result<Cik, EdgarError>, as_str() -> &str (10-digit zero padded),
//!         numeric() -> u64}; Cik: Debug + Clone + PartialEq + Eq + Display (padded)
//!   parse_company_tickers(&str) -> Result<TickerMap, EdgarError>
//!   TickerMap::{get(&str) -> Option<&Cik>, len(), is_empty(), suggest(&str) -> Vec<String>}
//!   parse_company_facts(&str) -> Result<CompanyFacts, EdgarError>
//!   CompanyFacts { pub cik: Cik, pub entity_name: String }, CompanyFacts::facts(tax, concept, unit) -> &[Fact]
//!   CompanyFacts and TickerMap must derive/impl Debug (tests print Results with {:?}); so must EdgarConfig.
//!   Fact { val: f64, start: Option<NaiveDate>, end: NaiveDate, fy: Option<i32>, fp: Option<String>,
//!          form: String, filed: NaiveDate, accn: String, frame: Option<String> }  (Debug+Clone+PartialEq)
//!   parse_submissions(&str) -> Result<Vec<Filing>, EdgarError>   (file order preserved)
//!   Filing { accession_number, filing_date: NaiveDate, report_date: Option<NaiveDate>, form,
//!            primary_document, items: Vec<String> } + document_url(&Cik) -> String + has_item(&str) -> bool
//!   EdgarError::Parse(String) for every malformed-input case, EdgarError::InvalidCik(String).

use chrono::NaiveDate;
use ghost_alpha::data::edgar::*;

fn fixture(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/edgar")
        .join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

// ── Cik ──────────────────────────────────────────────────────────────────────

#[test]
fn test_cik_new_when_small_number_should_zero_pad_to_ten_digits() {
    assert_eq!(Cik::new(42).as_str(), "0000000042");
    assert_eq!(Cik::new(999999).to_string(), "0000999999");
    assert_eq!(Cik::new(320193).numeric(), 320193);
}

#[test]
fn test_cik_parse_when_padded_or_unpadded_digits_should_normalise() {
    assert_eq!(Cik::parse("999999").unwrap(), Cik::new(999999));
    assert_eq!(Cik::parse("0000999999").unwrap(), Cik::new(999999));
}

#[test]
fn test_cik_parse_when_not_digits_or_too_long_should_error_invalid_cik() {
    for bad in ["", "abc", "12a4", "-5", "12345678901"] {
        assert!(
            matches!(Cik::parse(bad), Err(EdgarError::InvalidCik(_))),
            "{bad:?} must be rejected with InvalidCik"
        );
    }
}

// ── ticker map ───────────────────────────────────────────────────────────────

#[test]
fn test_parse_company_tickers_when_valid_should_map_ticker_to_padded_cik() {
    let m = parse_company_tickers(&fixture("company_tickers.json")).unwrap();
    assert_eq!(m.len(), 4);
    assert!(!m.is_empty());
    assert_eq!(m.get("EXMP").unwrap().as_str(), "0000999999");
    assert_eq!(m.get("TINY").unwrap().as_str(), "0000000042");
}

#[test]
fn test_parse_company_tickers_when_lookup_case_differs_should_still_match() {
    let m = parse_company_tickers(&fixture("company_tickers.json")).unwrap();
    assert_eq!(m.get("exmp"), m.get("EXMP"));
    assert!(m.get("eXmP").is_some());
}

#[test]
fn test_parse_company_tickers_when_unknown_ticker_should_return_none() {
    let m = parse_company_tickers(&fixture("company_tickers.json")).unwrap();
    assert!(m.get("NOPE").is_none());
    assert!(m.get("").is_none());
}

#[test]
fn test_parse_company_tickers_when_empty_object_should_give_empty_map() {
    let m = parse_company_tickers("{}").unwrap();
    assert!(m.is_empty());
    assert_eq!(m.len(), 0);
}

#[test]
fn test_parse_company_tickers_when_malformed_should_error_parse() {
    for bad in [
        "not json",
        "[]",
        r#"{"0":{"ticker":"X","title":"no cik"}}"#,
        r#"{"0":{"cik_str":"abc","ticker":"X","title":"bad cik"}}"#,
    ] {
        assert!(
            matches!(parse_company_tickers(bad), Err(EdgarError::Parse(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn test_ticker_map_suggest_when_prefix_related_should_list_sorted_matches() {
    let m = parse_company_tickers(&fixture("company_tickers.json")).unwrap();
    // "suggest" = tickers that start with the query or that the query starts with,
    // case-insensitive, sorted ascending, upper-case, at most 5.
    assert_eq!(
        m.suggest("EXM"),
        vec!["EXMA".to_string(), "EXMP".to_string()]
    );
    assert_eq!(
        m.suggest("exm"),
        vec!["EXMA".to_string(), "EXMP".to_string()]
    );
    assert!(m.suggest("QQQQ").is_empty());
}

// ── companyfacts ─────────────────────────────────────────────────────────────

#[test]
fn test_parse_company_facts_when_valid_should_expose_identity_and_multiple_taxonomies() {
    let f = parse_company_facts(&fixture("companyfacts_exmp.json")).unwrap();
    assert_eq!(f.cik.as_str(), "0000999999");
    assert_eq!(f.entity_name, "Example Corp");
    assert_eq!(f.facts("us-gaap", "Revenues", "USD").len(), 10);
    assert_eq!(
        f.facts("dei", "EntityCommonStockSharesOutstanding", "shares")
            .len(),
        1
    );
}

#[test]
fn test_parse_company_facts_when_multiple_units_should_key_by_unit() {
    let f = parse_company_facts(&fixture("companyfacts_exmp.json")).unwrap();
    assert_eq!(
        f.facts("us-gaap", "EarningsPerShareBasic", "USD/shares")
            .len(),
        2
    );
    assert!(f
        .facts("us-gaap", "EarningsPerShareBasic", "USD")
        .is_empty());
}

#[test]
fn test_parse_company_facts_when_concept_or_taxonomy_missing_should_return_empty_slice() {
    let f = parse_company_facts(&fixture("companyfacts_exmp.json")).unwrap();
    assert!(f.facts("us-gaap", "NoSuchConcept", "USD").is_empty());
    assert!(f.facts("ifrs-full", "Revenues", "USD").is_empty());
    assert!(f.facts("us-gaap", "Revenues", "shares").is_empty());
}

#[test]
fn test_parse_company_facts_when_duration_fact_should_fill_all_fields_in_file_order() {
    let f = parse_company_facts(&fixture("companyfacts_exmp.json")).unwrap();
    let first = &f.facts("us-gaap", "Revenues", "USD")[0];
    assert_eq!(first.val, 800.0);
    assert_eq!(first.start, Some(d("2021-01-01")));
    assert_eq!(first.end, d("2021-12-31"));
    assert_eq!(first.fy, Some(2021));
    assert_eq!(first.fp.as_deref(), Some("FY"));
    assert_eq!(first.form, "10-K");
    assert_eq!(first.filed, d("2022-02-14"));
    assert_eq!(first.accn, "0000999999-22-000008");
    assert_eq!(first.frame.as_deref(), Some("CY2021"));
}

#[test]
fn test_parse_company_facts_when_optional_fields_absent_should_be_none() {
    let f = parse_company_facts(&fixture("companyfacts_exmp.json")).unwrap();
    // instant fact: no start; frame present
    let a = &f.facts("us-gaap", "Assets", "USD")[0];
    assert_eq!(a.start, None);
    assert!(a.frame.is_some());
    // SEC puts the frame on the latest-filed fact of a period; the superseded original has none
    let original = &f.facts("us-gaap", "Revenues", "USD")[1];
    assert_eq!(original.val, 1000.0);
    assert_eq!(original.frame, None);
    let restated = &f.facts("us-gaap", "Revenues", "USD")[2];
    assert_eq!(restated.val, 1050.0);
    assert_eq!(restated.frame.as_deref(), Some("CY2022"));
    // fy/fp absent
    let eps = f
        .facts("us-gaap", "EarningsPerShareBasic", "USD/shares")
        .iter()
        .find(|x| x.end == d("2022-12-31"))
        .unwrap();
    assert_eq!(eps.fy, None);
    assert_eq!(eps.fp, None);
    assert!((eps.val - 1.25).abs() < 1e-12);
}

#[test]
fn test_parse_company_facts_when_malformed_should_error_parse() {
    let bad_date = r#"{"cik":1,"entityName":"X","facts":{"us-gaap":{"A":{"units":{"USD":[
        {"end":"nope","val":1,"accn":"a","form":"10-K","filed":"2024-01-01"}]}}}}"#;
    for bad in ["not json", "{}", r#"{"cik":1,"entityName":"X"}"#, bad_date] {
        assert!(
            matches!(parse_company_facts(bad), Err(EdgarError::Parse(_))),
            "{bad:?}"
        );
    }
}

// ── submissions ──────────────────────────────────────────────────────────────

#[test]
fn test_parse_submissions_when_valid_should_zip_columns_into_filings_in_file_order() {
    let v = parse_submissions(&fixture("submissions_exmp.json")).unwrap();
    assert_eq!(v.len(), 8);
    let q = &v[0];
    assert_eq!(q.accession_number, "0000999999-24-000040");
    assert_eq!(q.filing_date, d("2024-05-03"));
    assert_eq!(q.report_date, Some(d("2024-03-31")));
    assert_eq!(q.form, "10-Q");
    assert_eq!(q.primary_document, "exmp-20240331.htm");
    assert!(q.items.is_empty());
    assert_eq!(v[7].filing_date, d("2023-06-01"));
}

#[test]
fn test_parse_submissions_when_items_comma_separated_should_split_into_list() {
    let v = parse_submissions(&fixture("submissions_exmp.json")).unwrap();
    assert_eq!(v[1].form, "8-K");
    assert_eq!(v[1].items, vec!["2.02".to_string(), "9.01".to_string()]);
    assert_eq!(v[3].items, vec!["5.02".to_string()]);
}

#[test]
fn test_parse_submissions_when_report_date_or_items_blank_should_be_none_or_empty() {
    let v = parse_submissions(&fixture("submissions_exmp.json")).unwrap();
    assert_eq!(v[3].report_date, None); // reportDate ""
    assert!(v[2].items.is_empty()); // items ""
}

#[test]
fn test_filing_has_item_when_item_listed_should_be_true_only_for_exact_item() {
    let v = parse_submissions(&fixture("submissions_exmp.json")).unwrap();
    assert!(v[1].has_item("2.02"));
    assert!(v[1].has_item("9.01"));
    assert!(!v[1].has_item("2.0"));
    assert!(!v[1].has_item("5.02"));
    assert!(!v[0].has_item("2.02"));
}

#[test]
fn test_filing_document_url_when_cik_and_accession_should_strip_zeros_and_dashes() {
    let v = parse_submissions(&fixture("submissions_exmp.json")).unwrap();
    assert_eq!(
        v[0].document_url(&Cik::new(999999)),
        "https://www.sec.gov/Archives/edgar/data/999999/000099999924000040/exmp-20240331.htm"
    );
}

#[test]
fn test_parse_submissions_when_column_lengths_differ_should_error_parse() {
    let r = parse_submissions(&fixture("submissions_malformed.json"));
    assert!(matches!(r, Err(EdgarError::Parse(_))), "{r:?}");
}

#[test]
fn test_parse_submissions_when_structure_missing_or_invalid_should_error_parse() {
    for bad in [
        "not json",
        "{}",
        r#"{"filings":{}}"#,
        r#"{"filings":{"recent":{"accessionNumber":["a"]}}}"#,
        r#"{"filings":{"recent":{"accessionNumber":["a"],"filingDate":["notadate"],"reportDate":[""],
            "form":["8-K"],"items":[""],"primaryDocument":["x.htm"]}}}"#,
    ] {
        assert!(
            matches!(parse_submissions(bad), Err(EdgarError::Parse(_))),
            "{bad:?}"
        );
    }
}

#[test]
fn test_parse_submissions_when_recent_arrays_empty_should_return_empty_vec() {
    let j = r#"{"cik":"1","filings":{"recent":{"accessionNumber":[],"filingDate":[],"reportDate":[],
        "acceptanceDateTime":[],"form":[],"items":[],"primaryDocument":[],"primaryDocDescription":[]},"files":[]}}"#;
    assert!(parse_submissions(j).unwrap().is_empty());
}
