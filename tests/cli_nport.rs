//! Binary-level tests for `ghost-alpha edgar overlap` and `edgar holdings` (phase 4b), hermetic via wiremock.
//!
//! CLI contract pinned here (same env vars as the other `edgar` subcommands; one mock server plays www + data):
//!   ghost-alpha edgar overlap --symbol <TICKER> <ETF>... [--as-of YYYY-MM-DD =today] [--cusip C] [--lei L]
//!        [--format table|json =table]
//!     The matcher is built from the symbol plus EDGAR's company title (company_tickers.json; when the
//!     symbol is absent there, ticker-only), plus --cusip / --lei when given.
//!     json: stdout is ONLY a JSON array, one object per ETF in argument order, keys:
//!        etf (as given, string), series_name, period_end ("YYYY-MM-DD"), filed ("YYYY-MM-DD"),
//!        accession, pct (number, % of net assets), value_usd (number), per_10k (number = pct / 100 * 10000),
//!        matches: array of {name, asset_cat, pct, value_usd, rule} with rule one of
//!        "cusip" | "isin" | "lei" | "ticker" | "name".
//!     table: human text with, per ETF: the ETF symbol, series name, holdings date, filed date, the percent
//!        (to at least one decimal, e.g. "22.2"), and the dollars per $10,000 invested (rendered as "2222"
//!        or "2,222"), labelled "per $10,000".
//!   ghost-alpha edgar holdings <ETF> [--top N =25] [--format table|json =table]
//!     json: array of {name, cusip (string|null), asset_cat, pct, value_usd}, largest pct first, at most N.
//!     table: contains each listed holding's name and its percent.
//!   Failures (non-zero exit, message on stderr, no panic): unknown fund ticker (message shows the ticker
//!     and suggestions), non-N-PORT filer such as SPY (message contains "N-PORT"), no filing on/before
//!     --as-of (message contains "no N-PORT filing"), missing --symbol (usage error).

use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UA: &str = "Example Research jane.doe@example.com";
const ACC_NEW: &str = "0001410368-26-089594";
const ACC_OLD: &str = "0001410368-26-055328";

fn fx(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/edgar/nport")
            .join(name),
    )
    .unwrap()
}

fn scratch_cwd() -> PathBuf {
    let d = std::env::temp_dir().join(format!("ghost_alpha_nport_cli_cwd_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ghost() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ghost-alpha"));
    c.current_dir(scratch_cwd()).env("RUST_LOG", "error");
    for (k, _) in std::env::vars_os() {
        let k_s = k.to_string_lossy().to_uppercase();
        if k_s.starts_with("GHOST_ALPHA_") || k_s.starts_with("FINNHUB") {
            c.env_remove(&k);
        }
    }
    c
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}
fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}
fn describe(o: &Output) -> String {
    format!(
        "status={:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        o.status,
        stdout(o),
        stderr(o)
    )
}

fn doc_path(cik: u64, acc: &str) -> String {
    format!(
        "/Archives/edgar/data/{cik}/{}/primary_doc.xml",
        acc.replace('-', "")
    )
}

fn single_entry_feed(cik: u64, acc: &str, date: &str) -> String {
    format!(
        "<?xml version=\"1.0\"?><feed xmlns=\"http://www.w3.org/2005/Atom\"><entry>\
         <content type=\"text/xml\"><accession-number>{acc}</accession-number>\
         <filing-date>{date}</filing-date>\
         <filing-href>https://www.sec.gov/Archives/edgar/data/{cik}/{}/{acc}-index.htm</filing-href>\
         <filing-type>NPORT-P</filing-type></content></entry></feed>",
        acc.replace('-', "")
    )
}

struct Fx {
    server: MockServer,
    cache: TempDir,
}

impl Fx {
    /// XLY (real trimmed data, two quarters) and IVV (synthetic holdings) are served.
    async fn new() -> Fx {
        let server = MockServer::start().await;
        let get = |p: &str| {
            Mock::given(method("GET"))
                .and(path(p.to_string()))
                .and(header("user-agent", UA))
        };
        let json = |b: String| ResponseTemplate::new(200).set_body_raw(b, "application/json");
        let xml = |b: String| ResponseTemplate::new(200).set_body_raw(b, "application/xml");
        get("/files/company_tickers_mf.json")
            .respond_with(json(fx("company_tickers_mf.json")))
            .mount(&server)
            .await;
        get("/files/company_tickers.json")
            .respond_with(json(fx("company_tickers.json")))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/cgi-bin/browse-edgar"))
            .and(query_param("CIK", "S000006408"))
            .and(header("user-agent", UA))
            .respond_with(xml(fx("xly_feed.atom")))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/cgi-bin/browse-edgar"))
            .and(query_param("CIK", "S000004310"))
            .and(header("user-agent", UA))
            .respond_with(xml(single_entry_feed(
                1100663,
                "0001100663-26-000001",
                "2026-08-20",
            )))
            .mount(&server)
            .await;
        get(&doc_path(1064641, ACC_NEW))
            .respond_with(xml(fx("xly_primary_doc.xml")))
            .mount(&server)
            .await;
        get(&doc_path(1064641, ACC_OLD))
            .respond_with(xml(fx("xly_primary_doc.xml").replace(
                "<repPdDate>2026-06-30</repPdDate>",
                "<repPdDate>2026-03-31</repPdDate>",
            )))
            .mount(&server)
            .await;
        get(&doc_path(1100663, "0001100663-26-000001"))
            .respond_with(xml(
                fx("synthetic_nport.xml").replace("S999999001", "S000004310")
            ))
            .mount(&server)
            .await;
        Fx {
            server,
            cache: TempDir::new().unwrap(),
        }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = ghost();
        c.env("GHOST_ALPHA_EDGAR_USER_AGENT", UA)
            .env("GHOST_ALPHA_EDGAR_DATA_URL", self.server.uri())
            .env("GHOST_ALPHA_EDGAR_WWW_URL", self.server.uri())
            .env("GHOST_ALPHA_EDGAR_CACHE_DIR", self.cache.path())
            .arg("edgar")
            .args(args);
        c
    }

    /// Run off the async runtime thread so the mock server keeps serving.
    async fn run(&self, args: &[&str]) -> Output {
        let mut c = self.cmd(args);
        tokio::task::spawn_blocking(move || c.output().expect("spawn ghost-alpha"))
            .await
            .unwrap()
    }
}

fn json_rows(o: &Output) -> Vec<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(&stdout(o))
        .unwrap_or_else(|e| panic!("stdout is not pure JSON ({e})\n{}", describe(o)));
    v.as_array()
        .unwrap_or_else(|| panic!("not an array\n{}", describe(o)))
        .clone()
}

fn num(v: &serde_json::Value, k: &str) -> f64 {
    v[k].as_f64()
        .unwrap_or_else(|| panic!("{k} is not a number in {v}"))
}

fn assert_fails_cleanly(o: &Output) {
    assert!(!o.status.success(), "expected failure\n{}", describe(o));
    assert!(
        !stderr(o).contains("panicked"),
        "must not panic\n{}",
        describe(o)
    );
    assert!(
        !stderr(o).trim().is_empty(),
        "needs a message\n{}",
        describe(o)
    );
}

// ── overlap ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_overlap_json_when_amazon_in_xly_should_report_22_percent_and_per_10k() {
    let f = Fx::new().await;
    let o = f
        .run(&["overlap", "--symbol", "AMZN", "XLY", "--format", "json"])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let rows = json_rows(&o);
    assert_eq!(rows.len(), 1);
    let r = &rows[0];
    assert_eq!(r["etf"], "XLY");
    assert!(r["series_name"]
        .as_str()
        .unwrap()
        .contains("Consumer Discretionary"));
    assert_eq!(r["period_end"], "2026-06-30");
    assert_eq!(r["filed"], "2026-08-28");
    assert_eq!(r["accession"], ACC_NEW);
    approx::assert_relative_eq!(num(r, "pct"), 22.22150378005, max_relative = 1e-9);
    approx::assert_relative_eq!(num(r, "value_usd"), 5_020_601_115.80, max_relative = 1e-9);
    approx::assert_relative_eq!(num(r, "per_10k"), 2222.150378005, max_relative = 1e-9);
    let m = r["matches"].as_array().unwrap();
    assert_eq!(m.len(), 1);
    assert_eq!(m[0]["name"], "Amazon.com Inc");
    assert_eq!(m[0]["asset_cat"], "EC");
    assert_eq!(m[0]["rule"], "name");
}

#[tokio::test]
async fn test_overlap_json_when_cusip_flag_given_should_match_by_cusip() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "overlap",
            "--symbol",
            "AMZN",
            "--cusip",
            "023135106",
            "XLY",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let rows = json_rows(&o);
    assert_eq!(rows[0]["matches"][0]["rule"], "cusip");
    approx::assert_relative_eq!(num(&rows[0], "pct"), 22.22150378005, max_relative = 1e-9);
}

#[tokio::test]
async fn test_overlap_json_when_lei_flag_given_should_match_by_lei() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "overlap",
            "--symbol",
            "AMZN",
            "--lei",
            "ZXTILKJKG63JELOEG630",
            "XLY",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(json_rows(&o)[0]["matches"][0]["rule"], "lei");
}

#[tokio::test]
async fn test_overlap_json_when_as_of_earlier_should_use_older_filing() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "overlap",
            "--symbol",
            "AMZN",
            "XLY",
            "--as-of",
            "2026-06-01",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let r = &json_rows(&o)[0];
    assert_eq!(r["accession"], ACC_OLD);
    assert_eq!(r["filed"], "2026-05-28");
    assert_eq!(r["period_end"], "2026-03-31");
}

#[tokio::test]
async fn test_overlap_json_when_several_etfs_should_return_one_row_each_in_argument_order() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "overlap",
            "--symbol",
            "AMZN",
            "--cusip",
            "023135106",
            "XLY",
            "IVV",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let rows = json_rows(&o);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["etf"], "XLY");
    assert_eq!(rows[1]["etf"], "IVV");
    // synthetic fund, equity only: long 5.0 + name-only 0.25; bond 0.5 is reported as debt_pct
    approx::assert_relative_eq!(num(&rows[1], "pct"), 5.25, epsilon = 1e-9);
    approx::assert_relative_eq!(num(&rows[1], "per_10k"), 525.0, epsilon = 1e-6);
    approx::assert_relative_eq!(num(&rows[1], "debt_pct"), 0.5, epsilon = 1e-9);
    let cats: Vec<&str> = rows[1]["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["asset_cat"].as_str().unwrap())
        .collect();
    assert!(cats.contains(&"DBT") && cats.contains(&"DE"), "{cats:?}");
}

#[tokio::test]
async fn test_overlap_json_when_issuer_not_held_should_be_zero_with_no_matches() {
    let f = Fx::new().await;
    // Unknown to EDGAR is fine as long as an explicit key is supplied.
    let o = f
        .run(&[
            "overlap",
            "--symbol",
            "ZZZZ",
            "--cusip",
            "999999999",
            "XLY",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let r = &json_rows(&o)[0];
    assert_eq!(num(r, "pct"), 0.0);
    assert_eq!(num(r, "per_10k"), 0.0);
    assert_eq!(num(r, "debt_pct"), 0.0);
    assert!(r["matches"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_overlap_when_symbol_unknown_to_edgar_and_no_keys_should_fail_naming_cusip_flag() {
    let f = Fx::new().await;
    let o = f.run(&["overlap", "--symbol", "ZZZZ", "XLY"]).await;
    assert_fails_cleanly(&o);
    let e = stderr(&o);
    assert!(e.contains("ZZZZ") && e.contains("--cusip"), "{e}");
}

#[tokio::test]
async fn test_overlap_when_symbol_unknown_but_lei_given_should_proceed() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "overlap",
            "--symbol",
            "ZZZZ",
            "--lei",
            "ZXTILKJKG63JELOEG630",
            "XLY",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(json_rows(&o)[0]["matches"][0]["rule"], "lei");
}

#[tokio::test]
async fn test_overlap_json_should_report_debt_and_per_match_counted_flag() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "overlap",
            "--symbol",
            "AMZN",
            "--cusip",
            "023135106",
            "IVV",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let r = &json_rows(&o)[0];
    approx::assert_relative_eq!(num(r, "debt_pct"), 0.5, epsilon = 1e-9);
    let m = r["matches"].as_array().unwrap();
    assert_eq!(m.len(), 4);
    for x in m {
        let cat = x["asset_cat"].as_str().unwrap();
        let counted = x["counted"].as_bool().expect("counted is a bool");
        assert_eq!(counted, cat == "EC" || cat == "EP", "{x}");
    }
    assert!(m
        .iter()
        .any(|x| x["asset_cat"] == "DE" && x["counted"] == false));
}

#[tokio::test]
async fn test_overlap_table_when_debt_held_should_show_debt_separately() {
    let f = Fx::new().await;
    let o = f
        .run(&["overlap", "--symbol", "AMZN", "--cusip", "023135106", "IVV"])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let s = stdout(&o);
    assert!(s.contains("debt: 0.50%"), "{s}");
    assert!(s.contains("5.25"), "{s}");
}

#[tokio::test]
async fn test_overlap_table_when_no_debt_should_not_print_debt_line() {
    let f = Fx::new().await;
    let o = f.run(&["overlap", "--symbol", "AMZN", "XLY"]).await;
    assert!(o.status.success(), "{}", describe(&o));
    assert!(!stdout(&o).contains("debt:"), "{}", stdout(&o));
}

// ── one row per ETF ──────────────────────────────────────────────────────────

#[tokio::test]
async fn test_overlap_json_when_one_etf_fails_should_emit_error_row_and_exit_zero() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "overlap", "--symbol", "AMZN", "XLY", "SPY", "--format", "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let rows = json_rows(&o);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["etf"], "XLY");
    approx::assert_relative_eq!(num(&rows[0], "pct"), 22.22150378005, max_relative = 1e-9);
    assert_eq!(rows[1]["etf"], "SPY");
    assert!(rows[1].get("pct").is_none(), "{}", rows[1]);
    assert!(rows[1]["error"].as_str().unwrap().contains("N-PORT"));
}

#[tokio::test]
async fn test_overlap_json_when_failing_etf_is_listed_first_should_keep_argument_order() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "overlap", "--symbol", "AMZN", "XLYY", "XLY", "--format", "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let rows = json_rows(&o);
    assert_eq!(rows[0]["etf"], "XLYY");
    assert!(rows[0]["error"].as_str().unwrap().contains("XLYY"));
    assert_eq!(rows[1]["etf"], "XLY");
    assert!(rows[1].get("error").is_none());
}

#[tokio::test]
async fn test_overlap_table_when_one_etf_fails_should_show_message_in_its_row_and_exit_zero() {
    let f = Fx::new().await;
    let o = f.run(&["overlap", "--symbol", "AMZN", "XLY", "SPY"]).await;
    assert!(o.status.success(), "{}", describe(&o));
    let s = stdout(&o);
    assert!(
        s.contains("22.2") && s.contains("SPY") && s.contains("N-PORT"),
        "{s}"
    );
}

#[tokio::test]
async fn test_overlap_when_every_etf_fails_should_exit_nonzero_with_each_message() {
    let f = Fx::new().await;
    let o = f.run(&["overlap", "--symbol", "AMZN", "SPY", "XLYY"]).await;
    assert_fails_cleanly(&o);
    let e = stderr(&o);
    assert!(e.contains("SPY") && e.contains("XLYY"), "{e}");
}

#[tokio::test]
async fn test_overlap_table_when_default_format_should_show_dates_percent_and_per_10k() {
    let f = Fx::new().await;
    let o = f.run(&["overlap", "--symbol", "AMZN", "XLY"]).await;
    assert!(o.status.success(), "{}", describe(&o));
    let s = stdout(&o);
    for want in [
        "XLY",
        "Consumer Discretionary",
        "2026-06-30",
        "2026-08-28",
        "22.2",
    ] {
        assert!(s.contains(want), "missing {want:?}\n{s}");
    }
    assert!(s.contains("2,222") || s.contains("2222"), "per $10k\n{s}");
    assert!(s.contains("per $10,000"), "label\n{s}");
}

#[tokio::test]
async fn test_overlap_when_etf_is_unit_investment_trust_should_fail_with_nport_explanation() {
    let f = Fx::new().await;
    let o = f.run(&["overlap", "--symbol", "AMZN", "SPY"]).await;
    assert_fails_cleanly(&o);
    let e = stderr(&o);
    assert!(e.contains("SPY") && e.contains("N-PORT"), "{e}");
}

#[tokio::test]
async fn test_overlap_when_etf_unknown_should_fail_with_ticker_and_suggestion() {
    let f = Fx::new().await;
    let o = f.run(&["overlap", "--symbol", "AMZN", "XLYY"]).await;
    assert_fails_cleanly(&o);
    let e = stderr(&o);
    assert!(e.contains("XLYY") && e.contains("XLY"), "{e}");
}

#[tokio::test]
async fn test_overlap_when_as_of_before_first_filing_should_fail_with_no_filing_message() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "overlap",
            "--symbol",
            "AMZN",
            "XLY",
            "--as-of",
            "2020-01-01",
        ])
        .await;
    assert_fails_cleanly(&o);
    assert!(stderr(&o).contains("no N-PORT filing"), "{}", stderr(&o));
}

#[tokio::test]
async fn test_overlap_when_symbol_missing_should_be_usage_error() {
    let f = Fx::new().await;
    let o = f.run(&["overlap", "XLY"]).await;
    assert_fails_cleanly(&o);
    assert!(stderr(&o).contains("--symbol"), "{}", stderr(&o));
}

// ── holdings ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_holdings_json_when_top_given_should_list_largest_first_and_truncate() {
    let f = Fx::new().await;
    let o = f
        .run(&["holdings", "XLY", "--top", "2", "--format", "json"])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let rows = json_rows(&o);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["name"], "Amazon.com Inc");
    assert_eq!(rows[0]["cusip"], "023135106");
    assert_eq!(rows[0]["asset_cat"], "EC");
    approx::assert_relative_eq!(num(&rows[0], "pct"), 22.22150378005, max_relative = 1e-9);
    assert_eq!(rows[1]["name"], "Airbnb Inc");
    assert!(num(&rows[0], "pct") >= num(&rows[1], "pct"));
}

#[tokio::test]
async fn test_holdings_json_when_no_top_should_list_all_holdings() {
    let f = Fx::new().await;
    let o = f.run(&["holdings", "XLY", "--format", "json"]).await;
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(json_rows(&o).len(), 3);
}

#[tokio::test]
async fn test_holdings_table_should_show_names_and_percents() {
    let f = Fx::new().await;
    let o = f.run(&["holdings", "XLY"]).await;
    assert!(o.status.success(), "{}", describe(&o));
    let s = stdout(&o);
    assert!(s.contains("Amazon.com Inc") && s.contains("22.2"), "{s}");
    assert!(s.contains("AutoZone Inc"), "{s}");
}

#[tokio::test]
async fn test_holdings_when_unit_investment_trust_should_fail_with_nport_explanation() {
    let f = Fx::new().await;
    let o = f.run(&["holdings", "SPY"]).await;
    assert_fails_cleanly(&o);
    assert!(stderr(&o).contains("N-PORT"), "{}", stderr(&o));
}
