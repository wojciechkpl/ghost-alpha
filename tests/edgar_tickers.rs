//! Ticker normalisation and suggestion ranking (review finding M7).
//!
//! Contract (TickerMap, fixture tests/fixtures/edgar/company_tickers_ext.json):
//!   * `get` normalises class-share separators: "BRK.B", "brk/b", "BRK-B", "BRK_B" all resolve to SEC's "BRK-B"
//!     (each of '.', '/', '_' and ' ' in the query becomes '-', then upper-cased).
//!   * `suggest` candidates are tickers that start with the (normalised) query or that the query starts with;
//!     they are ranked by longest shared prefix with the query (descending), ties alphabetical;
//!     1-character tickers are excluded unless the query itself is 1 character; at most 5.

use ghost_alpha::data::edgar::*;

fn map() -> TickerMap {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/edgar/company_tickers_ext.json");
    parse_company_tickers(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn test_get_when_class_share_separator_variants_should_resolve_to_dashed_ticker() {
    let m = map();
    let want = m.get("BRK-B").expect("fixture has BRK-B").clone();
    assert_eq!(want.numeric(), 555001);
    for q in ["BRK.B", "brk.b", "brk/b", "BRK_B", "Brk-b", " brk.b "] {
        assert_eq!(m.get(q), Some(&want), "{q:?}");
    }
    assert_eq!(m.get("BRK.A").unwrap().numeric(), 555002);
}

#[test]
fn test_get_when_separator_variant_of_unknown_ticker_should_be_none() {
    let m = map();
    assert!(m.get("ZZZ.Q").is_none());
    assert!(m.get("BRK.C").is_none());
}

#[test]
fn test_suggest_when_candidates_differ_in_shared_prefix_should_rank_longest_first() {
    let m = map();
    // EXMPX: EXMP shares 4, EXM shares 3, EX shares 2; "E" (1 char) is excluded for a 5-char query.
    assert_eq!(m.suggest("EXMPX"), vec!["EXMP", "EXM", "EX"]);
}

#[test]
fn test_suggest_when_query_is_prefix_of_several_should_rank_exact_then_alphabetical() {
    let m = map();
    // query "EXM": EXM, EXMA and EXMP all share 3 chars (alphabetical among ties), EX shares only 2.
    assert_eq!(m.suggest("EXM"), vec!["EXM", "EXMA", "EXMP", "EX"]);
}

#[test]
fn test_suggest_when_query_is_multi_char_should_exclude_single_char_tickers() {
    let m = map();
    assert!(!m.suggest("EXMQ").contains(&"E".to_string()));
    assert!(!m.suggest("EX").contains(&"E".to_string()));
}

#[test]
fn test_suggest_when_query_is_single_char_should_include_single_char_tickers() {
    let m = map();
    let s = m.suggest("e");
    assert!(s.contains(&"E".to_string()), "{s:?}");
    assert_eq!(s[0], "E"); // all share 1 char; ties are alphabetical, so "E" first
    assert!(s.len() <= 5);
}

#[test]
fn test_suggest_when_separator_variant_query_should_normalise_before_matching() {
    let m = map();
    // "brk." normalises to "BRK-"
    assert_eq!(m.suggest("brk."), vec!["BRK-A", "BRK-B"]);
}

#[test]
fn test_suggest_when_more_than_five_matches_should_truncate_to_five() {
    let mut rows = serde_json::Map::new();
    for (i, t) in ["QAA", "QAB", "QAC", "QAD", "QAE", "QAF", "QAG"]
        .iter()
        .enumerate()
    {
        rows.insert(
            i.to_string(),
            serde_json::json!({"cik_str": 1000 + i, "ticker": t, "title": "Fictional"}),
        );
    }
    let m = parse_company_tickers(&serde_json::Value::Object(rows).to_string()).unwrap();
    assert_eq!(m.suggest("QA"), vec!["QAA", "QAB", "QAC", "QAD", "QAE"]);
}
