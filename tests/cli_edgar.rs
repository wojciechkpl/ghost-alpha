//! Binary-level tests for `ghost-alpha edgar ...` against wiremock (hermetic, no network).
//!
//! CLI contract pinned here:
//!   ghost-alpha edgar facts <TICKER> [--concept <C>=Revenues] [--taxonomy <T>=us-gaap] [--unit <U>=USD]
//!        [--periodicity annual|quarterly =annual] [--as-of YYYY-MM-DD =today] [--format table|json =table]
//!     => point-in-time `CompanyFacts::series(...)` for the concept.
//!     json: a JSON array (ascending by period end) of objects with keys
//!        end, val (number), start (string|null), fy (number|null), fp (string|null), form, filed, accn, frame (string|null)
//!        and NOTHING else on stdout (logs go to stderr).
//!     table: human text on stdout containing each row's end date and value (value may be rendered as
//!        "1050", "1050.0" or "1,050" -- tests accept any), plus form/filed/accn.
//!   ghost-alpha edgar filings <TICKER> [--form F] [--item I] [--since YYYY-MM-DD] [--format table|json]
//!     json: array (SEC order, newest first) of objects with keys accession_number, filing_date,
//!        report_date (string|null), form, primary_document, items (array of strings), url (document_url).
//!     table: text containing every matching accession number and no non-matching one.
//!   Env vars (all read by the binary): GHOST_ALPHA_EDGAR_USER_AGENT, GHOST_ALPHA_EDGAR_DATA_URL,
//!     GHOST_ALPHA_EDGAR_WWW_URL, GHOST_ALPHA_EDGAR_CACHE_DIR.
//!   No config.toml / Finnhub key needed. Failures: non-zero exit and a message on stderr (no panic).

use std::path::PathBuf;
use std::process::{Command, Output};
use tempfile::TempDir;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UA: &str = "Example Research jane.doe@example.com";

fn fx(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/edgar")
            .join(name),
    )
    .unwrap()
}

fn scratch_cwd() -> PathBuf {
    let d = std::env::temp_dir().join(format!("ghost_alpha_edgar_cli_cwd_{}", std::process::id()));
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

fn json_ok(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

struct Fx {
    server: MockServer,
    cache: TempDir,
}

impl Fx {
    /// One server plays both data.sec.gov and www.sec.gov (paths differ). Mocks require the UA header.
    async fn new() -> Fx {
        let server = MockServer::start().await;
        for (p, body) in [
            ("/files/company_tickers.json", "company_tickers.json"),
            (
                "/api/xbrl/companyfacts/CIK0000999999.json",
                "companyfacts_exmp.json",
            ),
            ("/submissions/CIK0000999999.json", "submissions_exmp.json"),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .and(header("user-agent", UA))
                .respond_with(json_ok(fx(body)))
                .mount(&server)
                .await;
        }
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

    /// Run the binary off the async runtime thread so the mock server keeps serving.
    async fn run(&self, args: &[&str]) -> Output {
        let mut c = self.cmd(args);
        tokio::task::spawn_blocking(move || c.output().expect("spawn ghost-alpha"))
            .await
            .unwrap()
    }
}

fn has_num(s: &str, n: &str) -> bool {
    // "1050", "1050.0" or "1,050"
    let (head, tail) = n.split_at(n.len() - 3);
    s.contains(n) || s.contains(&format!("{head},{tail}"))
}

fn json_rows(o: &Output) -> Vec<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(&stdout(o))
        .unwrap_or_else(|e| panic!("stdout is not pure JSON ({e})\n{}", describe(o)));
    v.as_array()
        .unwrap_or_else(|| panic!("not an array\n{}", describe(o)))
        .clone()
}

fn end_val(rows: &[serde_json::Value]) -> Vec<(String, f64)> {
    rows.iter()
        .map(|r| {
            (
                r["end"].as_str().unwrap().to_string(),
                r["val"].as_f64().unwrap(),
            )
        })
        .collect()
}

// ── edgar facts ──────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_json_and_as_of_should_print_pit_annual_series() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "Revenues",
            "--as-of",
            "2024-03-01",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let rows = json_rows(&o);
    assert_eq!(
        end_val(&rows),
        vec![
            ("2021-12-31".to_string(), 800.0),
            ("2022-12-31".to_string(), 1050.0),
            ("2023-12-31".to_string(), 1300.0)
        ]
    );
    let r = &rows[1];
    assert_eq!(r["form"], "10-K");
    assert_eq!(r["filed"], "2024-02-20");
    assert_eq!(r["accn"], "0000999999-24-000012");
    assert_eq!(r["fy"], 2023);
    assert_eq!(r["fp"], "FY");
    assert_eq!(r["start"], "2022-01-01");
    assert_eq!(r["frame"], "CY2022"); // frame sits on the latest-filed (restated) fact
    assert_eq!(rows[0]["frame"], "CY2021");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_as_of_before_restatement_should_show_original_value() {
    let f = Fx::new().await;
    let o = f
        .run(&["facts", "EXMP", "--as-of", "2023-12-31", "--format", "json"])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let vals: Vec<f64> = json_rows(&o)
        .iter()
        .map(|r| r["val"].as_f64().unwrap())
        .collect();
    assert_eq!(vals, vec![800.0, 1000.0]); // default concept is Revenues
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_no_as_of_should_default_to_today_and_include_amendment() {
    let f = Fx::new().await;
    let o = f.run(&["facts", "EXMP", "--format", "json"]).await;
    assert!(o.status.success(), "{}", describe(&o));
    let rows = json_rows(&o);
    assert_eq!(rows.last().unwrap()["val"].as_f64(), Some(1310.0));
    assert_eq!(rows.last().unwrap()["form"], "10-K/A");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_table_format_should_render_rows_with_point_in_time_values() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "Revenues",
            "--as-of",
            "2024-03-01",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let out = stdout(&o);
    for end in ["2021-12-31", "2022-12-31", "2023-12-31"] {
        assert!(out.contains(end), "missing {end}\n{out}");
    }
    assert!(
        has_num(&out, "1050"),
        "restated FY2022 value missing\n{out}"
    );
    assert!(has_num(&out, "1300"), "{out}");
    assert!(
        !has_num(&out, "1310"),
        "amendment filed after --as-of must be hidden\n{out}"
    );
    assert!(out.contains("10-K"), "{out}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_table_format_explicit_should_equal_default() {
    let f = Fx::new().await;
    let a = f.run(&["facts", "EXMP", "--as-of", "2024-03-01"]).await;
    let b = f
        .run(&[
            "facts",
            "EXMP",
            "--as-of",
            "2024-03-01",
            "--format",
            "table",
        ])
        .await;
    assert!(a.status.success() && b.status.success(), "{}", describe(&a));
    assert_eq!(stdout(&a), stdout(&b));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_quarterly_periodicity_should_list_10q_periods() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--periodicity",
            "quarterly",
            "--as-of",
            "2024-06-01",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let got = end_val(&json_rows(&o));
    assert_eq!(got.len(), 4);
    assert_eq!(got[0], ("2023-03-31".to_string(), 285.0));
    assert_eq!(got[3], ("2024-03-31".to_string(), 330.0));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_other_concept_taxonomy_and_unit_should_select_them() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--taxonomy",
            "us-gaap",
            "--concept",
            "EarningsPerShareBasic",
            "--unit",
            "USD/shares",
            "--as-of",
            "2024-03-01",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    // the 2022 EPS fact has no fy/fp so it is not an "FY" annual fact; only 2023 qualifies
    let rows = json_rows(&o);
    assert_eq!(rows.len(), 1, "{}", describe(&o));
    assert_eq!(rows[0]["end"], "2023-12-31");
    assert_eq!(rows[0]["val"].as_f64(), Some(1.5));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_lowercase_ticker_should_resolve() {
    let f = Fx::new().await;
    let o = f
        .run(&["facts", "exmp", "--as-of", "2024-03-01", "--format", "json"])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_unknown_ticker_should_fail_naming_ticker_and_suggestions() {
    let f = Fx::new().await;
    let o = f.run(&["facts", "EXM"]).await;
    assert!(!o.status.success(), "{}", describe(&o));
    let err = stderr(&o);
    assert!(
        err.contains("EXM") && err.contains("EXMP"),
        "{}",
        describe(&o)
    );
    assert!(!err.contains("panicked"), "{}", describe(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_bad_as_of_or_format_should_fail_cleanly() {
    let f = Fx::new().await;
    for args in [
        vec!["facts", "EXMP", "--as-of", "not-a-date"],
        vec!["facts", "EXMP", "--format", "yaml"],
    ] {
        let o = f.run(&args).await;
        assert!(!o.status.success(), "{args:?}\n{}", describe(&o));
        let err = stderr(&o);
        assert!(!err.contains("panicked"), "{}", describe(&o));
        assert!(
            !err.contains("unrecognized subcommand"),
            "must fail on the bad value, not on a missing subcommand\n{}",
            describe(&o)
        );
        assert!(
            err.contains("--as-of")
                || err.contains("--format")
                || err.contains("not-a-date")
                || err.contains("yaml"),
            "error must name the offending flag or value\n{}",
            describe(&o)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_when_user_agent_env_missing_should_fail_mentioning_user_agent() {
    let f = Fx::new().await;
    let mut c = f.cmd(&["facts", "EXMP"]);
    c.env_remove("GHOST_ALPHA_EDGAR_USER_AGENT");
    let o = tokio::task::spawn_blocking(move || c.output().unwrap())
        .await
        .unwrap();
    assert!(!o.status.success(), "{}", describe(&o));
    let err = stderr(&o).to_lowercase();
    assert!(
        err.contains("user-agent") || err.contains("user agent"),
        "{}",
        describe(&o)
    );
    assert!(
        f.server.received_requests().await.unwrap().is_empty(),
        "no HTTP request may be made without a valid User-Agent"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_when_user_agent_env_has_no_email_should_fail() {
    let f = Fx::new().await;
    let mut c = f.cmd(&["facts", "EXMP"]);
    c.env("GHOST_ALPHA_EDGAR_USER_AGENT", "just-a-name");
    let o = tokio::task::spawn_blocking(move || c.output().unwrap())
        .await
        .unwrap();
    assert!(!o.status.success(), "{}", describe(&o));
    let err = stderr(&o).to_lowercase();
    assert!(
        err.contains("user-agent") || err.contains("user agent"),
        "{}",
        describe(&o)
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_run_twice_should_use_cache_dir_env_and_hit_network_once() {
    let f = Fx::new().await;
    for _ in 0..2 {
        let o = f
            .run(&["facts", "EXMP", "--as-of", "2024-03-01", "--format", "json"])
            .await;
        assert!(o.status.success(), "{}", describe(&o));
    }
    let reqs = f.server.received_requests().await.unwrap();
    let n = |p: &str| reqs.iter().filter(|r| r.url.path() == p).count();
    assert_eq!(n("/api/xbrl/companyfacts/CIK0000999999.json"), 1);
    assert_eq!(n("/files/company_tickers.json"), 1);
    assert!(
        std::fs::read_dir(f.cache.path()).unwrap().count() >= 1,
        "cache dir must be populated"
    );
}

// ── edgar filings ────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_when_form_and_item_filter_json_should_list_matching_8k() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "filings", "EXMP", "--form", "8-K", "--item", "2.02", "--format", "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let rows = json_rows(&o);
    let accns: Vec<&str> = rows
        .iter()
        .map(|r| r["accession_number"].as_str().unwrap())
        .collect();
    assert_eq!(
        accns,
        vec![
            "0000999999-24-000035",
            "0000999999-24-000011",
            "0000999999-23-000041"
        ]
    );
    let r = &rows[0];
    assert_eq!(r["filing_date"], "2024-04-25");
    assert_eq!(r["form"], "8-K");
    assert_eq!(r["primary_document"], "exmp-8k_20240425.htm");
    assert_eq!(r["items"], serde_json::json!(["2.02", "9.01"]));
    assert_eq!(r["report_date"], "2024-04-25");
    assert_eq!(
        r["url"],
        "https://www.sec.gov/Archives/edgar/data/999999/000099999924000035/exmp-8k_20240425.htm"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_when_table_format_should_show_matching_accessions_only() {
    let f = Fx::new().await;
    let o = f
        .run(&["filings", "EXMP", "--form", "8-K", "--item", "2.02"])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let out = stdout(&o);
    for a in [
        "0000999999-24-000035",
        "0000999999-24-000011",
        "0000999999-23-000041",
    ] {
        assert!(out.contains(a), "missing {a}\n{out}");
    }
    for a in [
        "0000999999-24-000014",
        "0000999999-23-000025",
        "0000999999-24-000012",
    ] {
        assert!(!out.contains(a), "non-matching {a} leaked\n{out}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_when_since_given_should_apply_inclusive_lower_bound() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "filings",
            "EXMP",
            "--form",
            "8-K",
            "--item",
            "2.02",
            "--since",
            "2024-02-20",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(json_rows(&o).len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_when_blank_report_date_should_serialize_as_null() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "filings", "EXMP", "--form", "8-K", "--item", "5.02", "--format", "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let rows = json_rows(&o);
    assert_eq!(rows.len(), 1);
    assert!(rows[0]["report_date"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_when_no_match_should_succeed_with_empty_json_array() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "filings", "EXMP", "--form", "8-K", "--item", "8.01", "--format", "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert!(json_rows(&o).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_when_unknown_ticker_should_fail() {
    let f = Fx::new().await;
    let o = f.run(&["filings", "NOPE"]).await;
    assert!(!o.status.success(), "{}", describe(&o));
    assert!(stderr(&o).contains("NOPE"), "{}", describe(&o));
}

#[test]
fn test_edgar_help_when_requested_should_list_subcommands_and_flags() {
    let o = ghost().args(["edgar", "--help"]).output().unwrap();
    assert!(o.status.success(), "{}", describe(&o));
    let h = stdout(&o);
    assert!(h.contains("facts") && h.contains("filings"), "{h}");
    let o = ghost().args(["edgar", "facts", "--help"]).output().unwrap();
    let h = stdout(&o);
    for flag in [
        "--concept",
        "--unit",
        "--as-of",
        "--format",
        "--taxonomy",
        "--periodicity",
    ] {
        assert!(h.contains(flag), "edgar facts --help must list {flag}\n{h}");
    }
    let o = ghost()
        .args(["edgar", "filings", "--help"])
        .output()
        .unwrap();
    let h = stdout(&o);
    for flag in ["--form", "--item", "--since", "--format"] {
        assert!(
            h.contains(flag),
            "edgar filings --help must list {flag}\n{h}"
        );
    }
}
