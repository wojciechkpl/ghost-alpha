//! Pure N-PORT logic (phase 4b): fund ticker map, series feed, N-PORT XML, issuer matching,
//! exposure and point-in-time selection. No network.
//!
//! Contract pinned here (module `ghost_alpha::data::nport`):
//!   parse_fund_tickers(&str) -> Result<FundTickerMap, EdgarError>      (Parse on bad JSON/shape)
//!   FundTickerMap::{get(&str) -> Option<&FundId> (case-insensitive), len, is_empty, suggest(&str) -> Vec<String>}
//!   FundId { cik: Cik, series_id: String, class_id: String }
//!   parse_series_feed(&str) -> Result<Vec<NportFiling>, EdgarError>    (feed order kept; no entries = Ok(empty);
//!       not an Atom document = Parse)
//!   NportFiling { accession: String, filing_date: NaiveDate, form: String ("NPORT-P"|"NPORT-P/A"), cik: Cik }
//!       cik comes from the filing-href path (`/Archives/edgar/data/<cik>/...`)
//!   parse_nport(&str) -> Result<NportReport, EdgarError>               (Parse on malformed XML / missing
//!       repPdDate / not an N-PORT document)
//!   NportReport { series_id, series_name, registrant (regName), period_end (repPdDate, NOT repPdEnd),
//!       net_assets: f64, holdings: Vec<Holding> }
//!   Holding { name, title: Option<String>, cusip: Option<String> ("N/A"/missing => None), isin: Option<String>
//!       (identifiers/isin@value), ticker: Option<String> (identifiers/ticker@value), lei: Option<String>,
//!       value_usd: f64, pct: f64 (both as reported; shorts are negative in SEC data), asset_cat: String, short: bool
//!       (payoffProfile == "Short") }
//!   normalize_issuer_name(&str) -> String
//!   IssuerMatcher::new(ticker, Option<company_title>) .with_cusip(&str) .with_lei(&str) .match_rule(&Holding)
//!       -> Option<MatchRule>; MatchRule::{Cusip, Isin, Lei, Ticker, Name}; strongest rule wins.
//!   exposure(&NportReport, &IssuerMatcher) -> Exposure { pct, value_usd, matches: Vec<ExposureMatch{holding, rule}> }
//!       Totals sum ALL matches (equity, debt, derivatives); shorts contribute -abs(). Each match keeps its
//!       holding (so asset_cat stays visible).
//!   select_original(&[NportFiling], as_of) -> Option<&NportFiling>      newest "NPORT-P" with filing_date <= as_of
//!   amendment_candidates(&[NportFiling], &original, as_of) -> Vec<&NportFiling>
//!       "NPORT-P/A" with original.filing_date <= filing_date <= as_of, newest first, at most MAX_AMENDMENTS (3)
//!   same_period_amendment(original_period, &[amendment periods, newest first]) -> Option<usize>
//!       index of the first amendment whose period equals the original's (None => keep the original)

use chrono::NaiveDate;
use ghost_alpha::data::edgar::{Cik, EdgarError};
use ghost_alpha::data::nport::*;
use std::path::PathBuf;

fn fx(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/edgar/nport")
            .join(name),
    )
    .unwrap()
}

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

fn synthetic() -> NportReport {
    parse_nport(&fx("synthetic_nport.xml")).expect("synthetic N-PORT parses")
}

/// Holdings of synthetic_nport.xml by position (documented in the fixture order):
/// 0 Amazon long equity, 1 Amazon short swap, 2 Amazon bond, 3 Alphabet A, 4 Alphabet C,
/// 5 "Amazon.com, Inc." with cusip N/A, 6 "Amazonas Energia SA", 7 Microsoft (cusip),
/// 8 Microsoft (ISIN only), 9 Meta (ticker only), 10 LEI-only subsidiary, 11 bare name only.
fn h(r: &NportReport, i: usize) -> &Holding {
    &r.holdings[i]
}

fn holding(name: &str, pct: f64, value: f64, short: bool) -> Holding {
    Holding {
        name: name.into(),
        title: None,
        cusip: Some("023135106".into()),
        isin: None,
        ticker: None,
        lei: None,
        value_usd: value,
        pct,
        asset_cat: "EC".into(),
        short,
    }
}

// ── fund ticker map ──────────────────────────────────────────────────────────

#[test]
fn test_parse_fund_tickers_when_fixture_should_map_symbols_case_insensitively() {
    let m = parse_fund_tickers(&fx("company_tickers_mf.json")).unwrap();
    assert_eq!(m.len(), 5);
    assert!(!m.is_empty());
    let xly = m.get("xly").expect("XLY present");
    assert_eq!(xly.cik, Cik::new(1064641));
    assert_eq!(xly.series_id, "S000006408");
    assert_eq!(xly.class_id, "C000017594");
    assert_eq!(m.get("VTI").unwrap().series_id, "S000002848");
    assert_eq!(m.get("IVV").unwrap().cik, Cik::new(1100663));
    assert!(m.get("SPY").is_none());
}

#[test]
fn test_parse_fund_tickers_when_two_classes_share_series_should_keep_distinct_class_ids() {
    let m = parse_fund_tickers(&fx("company_tickers_mf.json")).unwrap();
    let (a, b) = (m.get("VFIAX").unwrap(), m.get("VOO").unwrap());
    assert_eq!(a.series_id, b.series_id);
    assert_ne!(a.class_id, b.class_id);
}

#[test]
fn test_parse_fund_tickers_when_fields_reordered_should_use_field_names() {
    let json = r#"{"fields":["symbol","classId","seriesId","cik"],"data":[["abcd","C000000001","S000000001",42]]}"#;
    let m = parse_fund_tickers(json).unwrap();
    let f = m.get("ABCD").unwrap();
    assert_eq!(f.cik, Cik::new(42));
    assert_eq!(f.series_id, "S000000001");
    assert_eq!(f.class_id, "C000000001");
}

#[test]
fn test_fund_ticker_map_suggest_when_query_is_near_miss_should_offer_real_symbols() {
    let m = parse_fund_tickers(&fx("company_tickers_mf.json")).unwrap();
    assert!(m.suggest("XLYY").contains(&"XLY".to_string()));
    assert!(m.suggest("ZZZZZ").is_empty());
}

#[test]
fn test_parse_fund_tickers_when_not_json_should_be_parse_error() {
    assert!(matches!(
        parse_fund_tickers("<html>"),
        Err(EdgarError::Parse(_))
    ));
}

#[test]
fn test_parse_fund_tickers_when_fields_missing_should_be_parse_error() {
    assert!(matches!(
        parse_fund_tickers(r#"{"data":[[1,"S1","C1","X"]]}"#),
        Err(EdgarError::Parse(_))
    ));
}

#[test]
fn test_parse_fund_tickers_when_no_rows_should_be_empty_map() {
    let m = parse_fund_tickers(r#"{"fields":["cik","seriesId","classId","symbol"],"data":[]}"#)
        .unwrap();
    assert!(m.is_empty());
    assert_eq!(m.len(), 0);
}

// ── series feed ──────────────────────────────────────────────────────────────

#[test]
fn test_parse_series_feed_when_fixture_should_list_filings_newest_first() {
    let f = parse_series_feed(&fx("xly_feed.atom")).unwrap();
    assert_eq!(f.len(), 4);
    assert_eq!(f[0].accession, "0001410368-26-089594");
    assert_eq!(f[0].filing_date, d("2026-08-28"));
    assert_eq!(f[0].form, "NPORT-P");
    assert_eq!(f[0].cik, Cik::new(1064641));
    assert_eq!(f[3].accession, "0001410368-25-031941");
    assert_eq!(f[3].filing_date, d("2025-11-26"));
}

#[test]
fn test_parse_series_feed_when_amendments_present_should_keep_form_and_order() {
    let f = parse_series_feed(&fx("synthetic_feed_amendments.atom")).unwrap();
    let got: Vec<(&str, &str)> = f
        .iter()
        .map(|x| (x.accession.as_str(), x.form.as_str()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("0001410368-26-100002", "NPORT-P/A"),
            ("0001410368-26-100001", "NPORT-P/A"),
            ("0001410368-26-089594", "NPORT-P"),
            ("0001410368-26-055328", "NPORT-P"),
        ]
    );
}

#[test]
fn test_parse_series_feed_when_no_entries_should_be_empty() {
    assert!(parse_series_feed(&fx("empty_feed.atom"))
        .unwrap()
        .is_empty());
}

#[test]
fn test_parse_series_feed_when_not_xml_should_be_parse_error() {
    assert!(matches!(
        parse_series_feed("definitely not xml"),
        Err(EdgarError::Parse(_))
    ));
}

// ── N-PORT XML ───────────────────────────────────────────────────────────────

#[test]
fn test_parse_nport_when_real_xly_should_read_header_and_holdings() {
    let r = parse_nport(&fx("xly_primary_doc.xml")).unwrap();
    assert_eq!(r.series_id, "S000006408");
    assert!(r.series_name.contains("Consumer Discretionary"));
    assert_eq!(r.registrant, "SELECT SECTOR SPDR TRUST");
    assert_eq!(r.period_end, d("2026-06-30")); // repPdDate, not repPdEnd (2026-09-30)
    approx::assert_relative_eq!(r.net_assets, 22_593_435_464.55, max_relative = 1e-9);
    assert_eq!(r.holdings.len(), 3);
    let amzn = r
        .holdings
        .iter()
        .find(|x| x.name == "Amazon.com Inc")
        .expect("Amazon present");
    assert_eq!(amzn.cusip.as_deref(), Some("023135106"));
    assert_eq!(amzn.isin.as_deref(), Some("US0231351067"));
    assert_eq!(amzn.lei.as_deref(), Some("ZXTILKJKG63JELOEG630"));
    assert_eq!(amzn.title.as_deref(), Some("Amazon.com Inc"));
    assert_eq!(amzn.asset_cat, "EC");
    assert!(!amzn.short);
    approx::assert_relative_eq!(amzn.pct, 22.22150378005, max_relative = 1e-12);
    approx::assert_relative_eq!(amzn.value_usd, 5_020_601_115.80, max_relative = 1e-12);
}

#[test]
fn test_parse_nport_when_synthetic_should_read_all_thirteen_holdings_in_order() {
    let r = synthetic();
    assert_eq!(r.series_id, "S999999001");
    assert_eq!(r.series_name, "Example Synthetic Overlap Fund");
    assert_eq!(r.registrant, "Example Funds Trust");
    assert_eq!(r.period_end, d("2026-09-30"));
    approx::assert_relative_eq!(r.net_assets, 100_000_000.0);
    assert_eq!(r.holdings.len(), 13);
    assert_eq!(h(&r, 4).title.as_deref(), Some("Alphabet Inc - Class C"));
}

#[test]
fn test_parse_nport_when_short_payoff_should_flag_short_and_keep_reported_sign() {
    let r = synthetic();
    let s = h(&r, 1); // swap booked under the counterparty's name
    assert_eq!(s.name, "Goldman Sachs International");
    assert_eq!(s.cusip, None);
    assert_eq!(s.isin, None);
    assert!(s.short);
    assert_eq!(s.asset_cat, "DE");
    assert!(s.pct < 0.0 && s.value_usd < 0.0);
    assert!(!h(&r, 0).short);
}

#[test]
fn test_parse_nport_when_debt_line_should_keep_asset_category() {
    let r = synthetic();
    assert_eq!(h(&r, 2).asset_cat, "DBT");
    assert_eq!(h(&r, 2).cusip.as_deref(), Some("023135BW5"));
}

#[test]
fn test_parse_nport_when_cusip_is_na_or_missing_should_be_none() {
    let r = synthetic();
    assert_eq!(h(&r, 5).cusip, None); // "N/A"
    assert_eq!(h(&r, 11).cusip, None); // element absent
}

#[test]
fn test_parse_nport_when_optional_fields_absent_should_be_none_not_error() {
    let r = synthetic();
    let bare = h(&r, 11);
    assert_eq!(bare.name, "Bare Minimum Name");
    assert_eq!(bare.title, None);
    assert_eq!(bare.isin, None);
    assert_eq!(bare.ticker, None);
    assert_eq!(bare.lei, None);
    approx::assert_relative_eq!(bare.pct, 0.05);
}

#[test]
fn test_parse_nport_when_identifiers_present_should_read_isin_and_ticker() {
    let r = synthetic();
    assert_eq!(h(&r, 3).ticker.as_deref(), Some("GOOGL"));
    assert_eq!(h(&r, 3).isin.as_deref(), Some("US02079K3059"));
    assert_eq!(h(&r, 8).isin.as_deref(), Some("US5949181045"));
    assert_eq!(h(&r, 9).ticker.as_deref(), Some("META"));
    assert_eq!(h(&r, 10).lei.as_deref(), Some("5493001KJTIIGC8Y1R12"));
}

#[test]
fn test_parse_nport_when_not_xml_should_be_parse_error() {
    assert!(matches!(parse_nport("{}"), Err(EdgarError::Parse(_))));
}

#[test]
fn test_parse_nport_when_period_date_missing_should_be_parse_error() {
    let x = fx("synthetic_nport.xml").replace("<repPdDate>2026-09-30</repPdDate>", "");
    assert!(matches!(parse_nport(&x), Err(EdgarError::Parse(_))));
}

#[test]
fn test_parse_nport_when_other_xml_document_should_be_parse_error() {
    assert!(matches!(
        parse_nport("<?xml version=\"1.0\"?><html><body>Not N-PORT</body></html>"),
        Err(EdgarError::Parse(_))
    ));
}

// ── name normalisation ───────────────────────────────────────────────────────

#[test]
fn test_normalize_issuer_name_should_strip_punctuation_legal_suffixes_and_class_tokens() {
    let cases = [
        ("Amazon.com Inc", "AMAZON"),
        ("AMAZON COM INC", "AMAZON"),
        ("Amazon.com, Inc.", "AMAZON"),
        ("Alphabet Inc. Class A", "ALPHABET"),
        ("Alphabet Inc - Class C", "ALPHABET"),
        ("Berkshire Hathaway Inc - Class B", "BERKSHIRE HATHAWAY"),
        ("Microsoft Corp", "MICROSOFT"),
        ("Amazonas Energia SA", "AMAZONAS ENERGIA"),
    ];
    for (input, want) in cases {
        assert_eq!(normalize_issuer_name(input), want, "input {input:?}");
    }
}

#[test]
fn test_normalize_issuer_name_when_different_issuers_should_stay_distinct() {
    assert_ne!(
        normalize_issuer_name("Amazon.com Services LLC"),
        normalize_issuer_name("Amazon.com Inc")
    );
    assert_ne!(
        normalize_issuer_name("Amazonas Energia SA"),
        normalize_issuer_name("AMAZON COM INC")
    );
}

// ── issuer matcher: one test per rule ────────────────────────────────────────

#[test]
fn test_matcher_when_cusip_given_should_match_issuer_code_across_securities() {
    let r = synthetic();
    let m = IssuerMatcher::new("AMZN", None).with_cusip("023135106");
    assert_eq!(m.match_rule(h(&r, 0)), Some(MatchRule::Cusip));
    assert_eq!(m.match_rule(h(&r, 1)), None); // counterparty swap: no CUSIP, name is the bank
    assert_eq!(m.match_rule(h(&r, 12)), Some(MatchRule::Cusip)); // DE line on the same CUSIP
    assert_eq!(m.match_rule(h(&r, 2)), Some(MatchRule::Cusip)); // bond, same first 6
    assert_eq!(m.match_rule(h(&r, 7)), None);
}

#[test]
fn test_matcher_when_share_classes_should_match_both_on_six_char_issuer_code() {
    let r = synthetic();
    let m = IssuerMatcher::new("GOOG", None).with_cusip("02079K107"); // class C key
    assert_eq!(m.match_rule(h(&r, 4)), Some(MatchRule::Cusip));
    assert_eq!(m.match_rule(h(&r, 3)), Some(MatchRule::Cusip)); // class A, other last digits
}

#[test]
fn test_matcher_when_cusip_absent_in_holding_but_isin_embeds_it_should_match_isin() {
    let r = synthetic();
    let m = IssuerMatcher::new("MSFT", None).with_cusip("594918104");
    assert_eq!(m.match_rule(h(&r, 8)), Some(MatchRule::Isin));
}

#[test]
fn test_matcher_when_cusip_and_isin_both_match_should_report_cusip_as_strongest() {
    let r = synthetic();
    let m = IssuerMatcher::new("MSFT", Some("MICROSOFT CORP")).with_cusip("594918104");
    assert_eq!(m.match_rule(h(&r, 7)), Some(MatchRule::Cusip));
}

#[test]
fn test_matcher_when_lei_given_should_match_lei_case_insensitively() {
    let r = synthetic();
    let m = IssuerMatcher::new("EXSUB", None).with_lei("5493001kjtiigc8y1r12");
    assert_eq!(m.match_rule(h(&r, 10)), Some(MatchRule::Lei));
    assert_eq!(m.match_rule(h(&r, 0)), None);
}

#[test]
fn test_matcher_when_ticker_identifier_equals_symbol_should_match_ticker() {
    let r = synthetic();
    let m = IssuerMatcher::new("meta", None);
    assert_eq!(m.match_rule(h(&r, 9)), Some(MatchRule::Ticker));
}

#[test]
fn test_matcher_when_ticker_only_shares_a_prefix_should_not_match() {
    let r = synthetic();
    let m = IssuerMatcher::new("GOOG", None);
    assert_eq!(m.match_rule(h(&r, 4)), Some(MatchRule::Ticker)); // GOOG == GOOG
    assert_eq!(m.match_rule(h(&r, 3)), None); // GOOGL is a different identifier
}

#[test]
fn test_matcher_when_company_title_given_should_match_normalised_name() {
    let r = synthetic();
    let m = IssuerMatcher::new("AMZN", Some("AMAZON COM INC"));
    assert_eq!(m.match_rule(h(&r, 5)), Some(MatchRule::Name)); // no cusip, "Amazon.com, Inc."
    assert_eq!(m.match_rule(h(&r, 0)), Some(MatchRule::Name));
    assert_eq!(m.match_rule(h(&r, 2)), Some(MatchRule::Name)); // bond is visible, not dropped
}

#[test]
fn test_matcher_when_near_miss_name_should_not_match_amazonas_for_amazon() {
    let r = synthetic();
    let by_name = IssuerMatcher::new("AMZN", Some("AMAZON COM INC"));
    let by_all = IssuerMatcher::new("AMZN", Some("AMAZON COM INC")).with_cusip("023135106");
    assert_eq!(by_name.match_rule(h(&r, 6)), None);
    assert_eq!(by_all.match_rule(h(&r, 6)), None);
}

#[test]
fn test_matcher_when_name_and_cusip_both_apply_should_prefer_cusip() {
    let r = synthetic();
    let m = IssuerMatcher::new("AMZN", Some("AMAZON COM INC")).with_cusip("023135106");
    assert_eq!(m.match_rule(h(&r, 0)), Some(MatchRule::Cusip));
    assert_eq!(m.match_rule(h(&r, 5)), Some(MatchRule::Name)); // cusip N/A: name still catches it
}

#[test]
fn test_matcher_when_holding_has_only_a_name_and_no_title_given_should_not_match() {
    let r = synthetic();
    let m = IssuerMatcher::new("AMZN", None);
    for i in 0..r.holdings.len() {
        assert_eq!(
            m.match_rule(h(&r, i)),
            None,
            "holding {i} ({}) must not match on ticker AMZN alone",
            h(&r, i).name
        );
    }
}

// ── exposure ─────────────────────────────────────────────────────────────────

#[test]
fn test_exposure_when_amazon_with_cusip_should_sum_equity_only_and_report_debt_separately() {
    let r = synthetic();
    let m = IssuerMatcher::new("AMZN", Some("AMAZON COM INC")).with_cusip("023135106");
    let e = exposure(&r, &m);
    // Equity: long 5.0 + name-only line 0.25. The Goldman swap does not match (counterparty name,
    // cusip N/A). The matching DE line (0.2) is listed but never summed; the bond (0.5) is debt.
    approx::assert_relative_eq!(e.pct, 5.25, epsilon = 1e-9);
    approx::assert_relative_eq!(e.value_usd, 5_250_000.0, epsilon = 1e-6);
    approx::assert_relative_eq!(e.debt_pct, 0.5, epsilon = 1e-9);
    approx::assert_relative_eq!(e.debt_value_usd, 500_000.0, epsilon = 1e-6);
    assert_eq!(e.matches.len(), 4);
    let by_cat = |cat: &str| {
        e.matches
            .iter()
            .filter(|x| x.holding.asset_cat == cat)
            .collect::<Vec<_>>()
    };
    assert_eq!(by_cat("DBT").len(), 1, "bond stays visible");
    assert_eq!(by_cat("DBT")[0].rule, MatchRule::Cusip);
    assert!(!by_cat("DBT")[0].counted, "debt is not part of equity pct");
    assert_eq!(by_cat("DE").len(), 1, "derivative stays visible");
    assert_eq!(by_cat("DE")[0].rule, MatchRule::Cusip);
    assert!(!by_cat("DE")[0].counted, "derivatives are never summed");
    let equity = by_cat("EC");
    assert_eq!(equity.len(), 2);
    assert!(equity.iter().all(|x| x.counted));
    assert!(e.matches.iter().any(|x| x.rule == MatchRule::Name));
    assert!(e
        .matches
        .iter()
        .all(|x| x.holding.name != "Amazonas Energia SA"
            && x.holding.name != "Goldman Sachs International"));
}

#[test]
fn test_exposure_when_name_only_matcher_should_find_same_totals_with_name_rule() {
    let r = synthetic();
    let e = exposure(&r, &IssuerMatcher::new("AMZN", Some("AMAZON COM INC")));
    approx::assert_relative_eq!(e.pct, 5.25, epsilon = 1e-9);
    approx::assert_relative_eq!(e.debt_pct, 0.5, epsilon = 1e-9);
    assert_eq!(e.matches.len(), 4);
    assert!(e.matches.iter().all(|x| x.rule == MatchRule::Name));
}

#[test]
fn test_exposure_when_no_debt_held_should_report_zero_debt() {
    let r = parse_nport(&fx("xly_primary_doc.xml")).unwrap();
    let e = exposure(&r, &IssuerMatcher::new("AMZN", Some("AMAZON COM INC")));
    assert_eq!(e.debt_pct, 0.0);
    assert_eq!(e.debt_value_usd, 0.0);
    assert!(e.matches.iter().all(|m| m.counted));
}

fn hand_report(holdings: Vec<Holding>) -> NportReport {
    NportReport {
        series_id: "S1".into(),
        series_name: "Hand Built".into(),
        registrant: "Example Trust".into(),
        period_end: d("2026-09-30"),
        net_assets: 1_000.0,
        holdings,
    }
}

#[test]
fn test_exposure_when_equity_and_preferred_categories_should_both_count() {
    let mut ep = holding("Amazon.com Inc", 2.0, 20.0, false);
    ep.asset_cat = "EP".into();
    let r = hand_report(vec![holding("Amazon.com Inc", 10.0, 100.0, false), ep]);
    let e = exposure(
        &r,
        &IssuerMatcher::new("AMZN", None).with_cusip("023135106"),
    );
    approx::assert_relative_eq!(e.pct, 12.0, epsilon = 1e-9);
    assert!(e.matches.iter().all(|m| m.counted));
}

#[test]
fn test_exposure_when_other_asset_categories_should_be_listed_but_not_summed() {
    let mut swap = holding("Amazon.com Inc", 4.0, 40.0, false);
    swap.asset_cat = "DE".into();
    let mut other = holding("Amazon.com Inc", 1.0, 10.0, false);
    other.asset_cat = "RA".into(); // repurchase agreement: not equity, not debt
    let r = hand_report(vec![
        holding("Amazon.com Inc", 10.0, 100.0, false),
        swap,
        other,
    ]);
    let e = exposure(
        &r,
        &IssuerMatcher::new("AMZN", None).with_cusip("023135106"),
    );
    approx::assert_relative_eq!(e.pct, 10.0, epsilon = 1e-9);
    approx::assert_relative_eq!(e.value_usd, 100.0, epsilon = 1e-9);
    assert_eq!(e.debt_pct, 0.0);
    assert_eq!(e.matches.len(), 3);
    assert_eq!(e.matches.iter().filter(|m| m.counted).count(), 1);
}

#[test]
fn test_exposure_when_two_share_classes_should_aggregate_them() {
    let r = synthetic();
    let m = IssuerMatcher::new("GOOG", Some("Alphabet Inc.")).with_cusip("02079K107");
    let e = exposure(&r, &m);
    approx::assert_relative_eq!(e.pct, 3.5, epsilon = 1e-9);
    approx::assert_relative_eq!(e.value_usd, 3_500_000.0, epsilon = 1e-6);
    assert_eq!(e.matches.len(), 2);
}

#[test]
fn test_exposure_when_issuer_not_held_should_be_zero_with_no_matches() {
    let r = synthetic();
    let e = exposure(&r, &IssuerMatcher::new("ZZZZ", Some("Zzzz Holdings Corp")));
    assert_eq!(e.pct, 0.0);
    assert_eq!(e.value_usd, 0.0);
    assert!(e.matches.is_empty());
}

#[test]
fn test_exposure_when_real_xly_and_amazon_should_report_22_percent() {
    let r = parse_nport(&fx("xly_primary_doc.xml")).unwrap();
    let e = exposure(&r, &IssuerMatcher::new("AMZN", Some("AMAZON COM INC")));
    approx::assert_relative_eq!(e.pct, 22.22150378005, max_relative = 1e-12);
    approx::assert_relative_eq!(e.value_usd, 5_020_601_115.80, max_relative = 1e-12);
    assert_eq!(e.matches.len(), 1);
    assert_eq!(e.matches[0].rule, MatchRule::Name);
}

#[test]
fn test_exposure_when_short_reported_as_positive_magnitude_should_still_count_negative() {
    let r = NportReport {
        series_id: "S1".into(),
        series_name: "Hand Built".into(),
        registrant: "Example Trust".into(),
        period_end: d("2026-09-30"),
        net_assets: 1_000.0,
        holdings: vec![
            holding("Amazon.com Inc", 10.0, 100.0, false),
            holding("Amazon.com Inc", 3.0, 30.0, true),
        ],
    };
    let e = exposure(
        &r,
        &IssuerMatcher::new("AMZN", None).with_cusip("023135106"),
    );
    approx::assert_relative_eq!(e.pct, 7.0, epsilon = 1e-9);
    approx::assert_relative_eq!(e.value_usd, 70.0, epsilon = 1e-9);
}

#[test]
fn test_exposure_when_report_has_no_holdings_should_be_zero() {
    let r = NportReport {
        series_id: "S1".into(),
        series_name: "Empty".into(),
        registrant: "Example Trust".into(),
        period_end: d("2026-09-30"),
        net_assets: 0.0,
        holdings: vec![],
    };
    let e = exposure(&r, &IssuerMatcher::new("AMZN", Some("AMAZON COM INC")));
    assert_eq!(e.pct, 0.0);
    assert!(e.matches.is_empty());
}

// ── point-in-time selection ──────────────────────────────────────────────────

fn feed() -> Vec<NportFiling> {
    parse_series_feed(&fx("synthetic_feed_amendments.atom")).unwrap()
}

#[test]
fn test_select_original_when_as_of_after_all_should_pick_newest_original_not_amendment() {
    let f = feed();
    let o = select_original(&f, d("2026-09-30")).unwrap();
    assert_eq!(o.accession, "0001410368-26-089594");
    assert_eq!(o.form, "NPORT-P");
}

#[test]
fn test_select_original_when_as_of_equals_filing_date_should_include_it() {
    let f = feed();
    assert_eq!(
        select_original(&f, d("2026-08-28")).unwrap().accession,
        "0001410368-26-089594"
    );
}

#[test]
fn test_select_original_when_as_of_before_newest_should_fall_back_to_older_original() {
    let f = feed();
    assert_eq!(
        select_original(&f, d("2026-08-27")).unwrap().accession,
        "0001410368-26-055328"
    );
}

#[test]
fn test_select_original_when_as_of_before_everything_should_be_none() {
    assert!(select_original(&feed(), d("2020-01-01")).is_none());
}

#[test]
fn test_select_original_when_empty_or_unordered_should_not_depend_on_feed_order() {
    assert!(select_original(&[], d("2026-09-30")).is_none());
    let mut f = feed();
    f.reverse();
    assert_eq!(
        select_original(&f, d("2026-09-30")).unwrap().accession,
        "0001410368-26-089594"
    );
}

#[test]
fn test_amendment_candidates_when_amendments_filed_after_original_should_list_newest_first() {
    let f = feed();
    let o = select_original(&f, d("2026-09-30")).unwrap();
    let got: Vec<&str> = amendment_candidates(&f, o, d("2026-09-30"))
        .iter()
        .map(|x| x.accession.as_str())
        .collect();
    assert_eq!(got, vec!["0001410368-26-100002", "0001410368-26-100001"]);
}

#[test]
fn test_amendment_candidates_when_amendment_filed_after_as_of_should_be_excluded() {
    let f = feed();
    let o = select_original(&f, d("2026-09-16")).unwrap();
    let got: Vec<&str> = amendment_candidates(&f, o, d("2026-09-16"))
        .iter()
        .map(|x| x.accession.as_str())
        .collect();
    assert_eq!(got, vec!["0001410368-26-100001"]);
    let o = select_original(&f, d("2026-08-31")).unwrap();
    assert!(amendment_candidates(&f, o, d("2026-08-31")).is_empty());
}

#[test]
fn test_amendment_candidates_when_more_than_three_should_cap_at_three_newest() {
    let cik = Cik::new(1064641);
    let mut f = vec![NportFiling {
        accession: "0001-26-000000".into(),
        filing_date: d("2026-08-28"),
        form: "NPORT-P".into(),
        cik: cik.clone(),
    }];
    for i in 1..=5 {
        f.insert(
            0,
            NportFiling {
                accession: format!("0001-26-00000{i}"),
                filing_date: d("2026-09-01") + chrono::Duration::days(i),
                form: "NPORT-P/A".into(),
                cik: cik.clone(),
            },
        );
    }
    assert_eq!(MAX_AMENDMENTS, 3);
    let o = select_original(&f, d("2026-12-31")).unwrap();
    let got: Vec<&str> = amendment_candidates(&f, o, d("2026-12-31"))
        .iter()
        .map(|x| x.accession.as_str())
        .collect();
    assert_eq!(
        got,
        vec!["0001-26-000005", "0001-26-000004", "0001-26-000003"]
    );
}

#[test]
fn test_amendment_candidates_when_amendment_predates_original_should_be_excluded() {
    let cik = Cik::new(1064641);
    let mk = |acc: &str, date: &str, form: &str| NportFiling {
        accession: acc.into(),
        filing_date: d(date),
        form: form.into(),
        cik: cik.clone(),
    };
    let f = vec![
        mk("O", "2026-08-28", "NPORT-P"),
        mk("A-old", "2026-07-01", "NPORT-P/A"),
    ];
    let o = select_original(&f, d("2026-12-31")).unwrap();
    assert!(amendment_candidates(&f, o, d("2026-12-31")).is_empty());
}

#[test]
fn test_same_period_amendment_when_amendment_is_for_older_period_should_not_replace() {
    assert_eq!(
        same_period_amendment(d("2026-06-30"), &[d("2026-03-31")]),
        None
    );
}

#[test]
fn test_same_period_amendment_when_period_matches_should_return_newest_matching_index() {
    let periods = [d("2026-03-31"), d("2026-06-30"), d("2026-06-30")];
    assert_eq!(same_period_amendment(d("2026-06-30"), &periods), Some(1));
    assert_eq!(same_period_amendment(d("2026-06-30"), &[]), None);
}

// ── round 2: ticker rule on foreign lines, name normalisation, accession shape ──

fn tickered(ticker: &str, isin: Option<&str>) -> Holding {
    let mut x = holding("Some Foreign Telecom", 1.0, 10.0, false);
    x.cusip = None;
    x.ticker = Some(ticker.into());
    x.isin = isin.map(str::to_string);
    x
}

#[test]
fn test_matcher_when_ticker_collides_with_foreign_isin_should_not_match() {
    let m = IssuerMatcher::new("T", None);
    assert_eq!(m.match_rule(&tickered("T", Some("CA87971M1032"))), None);
    assert_eq!(m.match_rule(&tickered("T", Some("GB0002634946"))), None);
}

#[test]
fn test_matcher_when_ticker_line_has_us_isin_or_no_isin_should_match_ticker() {
    let m = IssuerMatcher::new("T", None);
    assert_eq!(
        m.match_rule(&tickered("T", Some("US00206R1023"))),
        Some(MatchRule::Ticker)
    );
    assert_eq!(m.match_rule(&tickered("t", None)), Some(MatchRule::Ticker));
}

#[test]
fn test_normalize_issuer_name_should_drop_adr_ads_sponsored_capital_stock_and_shares() {
    assert_eq!(
        normalize_issuer_name("Alphabet Inc Class C Capital Stock"),
        "ALPHABET"
    );
    assert_eq!(
        normalize_issuer_name("Taiwan Semiconductor Manufacturing Co Ltd Sponsored ADR"),
        normalize_issuer_name("TAIWAN SEMICONDUCTOR MANUFACTURING CO LTD")
    );
    assert_eq!(
        normalize_issuer_name("Example Holdings ADS"),
        normalize_issuer_name("Example Holdings")
    );
    assert_eq!(
        normalize_issuer_name("Example Holdings Ordinary Shares"),
        normalize_issuer_name("Example Holdings Ordinary")
    );
    assert_eq!(
        normalize_issuer_name("Example Holdings Shares"),
        normalize_issuer_name("Example Holdings")
    );
}

#[test]
fn test_normalize_issuer_name_when_accented_should_fold_latin_accents() {
    assert_eq!(
        normalize_issuer_name("NESTL\u{c9}"),
        normalize_issuer_name("NESTLE")
    );
    assert_eq!(
        normalize_issuer_name("Nestl\u{e9} SA"),
        normalize_issuer_name("Nestle S.A.")
    );
    assert_eq!(
        normalize_issuer_name("Cr\u{e9}dit Agricole"),
        "CREDIT AGRICOLE"
    );
}

fn feed_with_accession(acc: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><feed xmlns=\"http://www.w3.org/2005/Atom\"><entry>\
         <content type=\"text/xml\"><accession-number>{acc}</accession-number>\
         <filing-date>2026-08-28</filing-date>\
         <filing-href>https://www.sec.gov/Archives/edgar/data/1064641/x/y-index.htm</filing-href>\
         <filing-type>NPORT-P</filing-type></content></entry></feed>"
    )
}

#[test]
fn test_parse_series_feed_when_accession_is_well_formed_should_be_accepted() {
    let f = parse_series_feed(&feed_with_accession("0001410368-26-089594")).unwrap();
    assert_eq!(f.len(), 1);
}

#[test]
fn test_parse_series_feed_when_accession_malformed_should_be_parse_error() {
    for bad in [
        "0001410368-26-89594",    // short sequence
        "001410368-26-089594",    // short filer id
        "0001410368-2026-089594", // long year
        "0001410368--26-089594",  // double dash
        "0001410368-26-089594-1", // extra part
        "0001410368-26-08959x",   // non-digit
        "../../etc/passwd",       // path traversal
        "000141036826089594",     // no dashes
    ] {
        assert!(
            matches!(
                parse_series_feed(&feed_with_accession(bad)),
                Err(EdgarError::Parse(_))
            ),
            "accession {bad:?} must be a Parse error"
        );
    }
}
