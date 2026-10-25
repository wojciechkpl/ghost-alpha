//! Binary-level tests for the review-round-2 CLI changes (hermetic, wiremock, no network).
//!
//! Contract:
//!   H2  A config.toml containing only `[edgar] user_agent = "..."` (no Finnhub key, no env UA) is enough
//!       (passed via the global `--config <path>` placed before `edgar`). A config.toml with a TOML syntax error
//!       must not block edgar when GHOST_ALPHA_EDGAR_USER_AGENT is set, and a warning mentioning "config"
//!       goes to stderr (written unconditionally, not via RUST_LOG-filtered tracing). No config file => no
//!       mention of "config" on stderr.
//!   H3  `edgar filings --since D` includes older paginated filings; without --since, when the company has
//!       older pages, stderr carries a note containing the oldest `recent` filing date and the text "--since".
//!       No note when there are no older pages. stdout stays pure JSON/table.
//!   M3  facts table header has columns START and "FILING FY/FP" (no bare "FY/FP" header); instants have a
//!       blank START cell, durations show their start date.
//!   M4  Unknown concept/unit: exit 0, JSON stdout `[]`, stderr says the concept/unit was "not found",
//!       lists the units available for the concept when the concept exists, and up to 5 similar
//!       concept names (case-insensitive substring) when it does not.
//!   L5  Default cache dir with HOME="" (or a relative HOME) and no cache/XDG env: non-zero exit, stderr names
//!       GHOST_ALPHA_EDGAR_CACHE_DIR, no request made.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
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
    let d = std::env::temp_dir().join(format!("ghost_alpha_edgar_r2_cwd_{}", std::process::id()));
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

fn ok(body: String) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(body, "application/json")
}

struct Fx {
    server: MockServer,
    cache: TempDir,
}

impl Fx {
    async fn new() -> Fx {
        Fx::with("companyfacts_exmp.json", "submissions_exmp.json").await
    }

    async fn with(facts: &str, subs: &str) -> Fx {
        let server = MockServer::start().await;
        for (p, body) in [
            ("/files/company_tickers.json", "company_tickers.json"),
            ("/api/xbrl/companyfacts/CIK0000999999.json", facts),
            ("/submissions/CIK0000999999.json", subs),
            (
                "/submissions/CIK0000999999-submissions-001.json",
                "submissions_page_001.json",
            ),
            (
                "/submissions/CIK0000999999-submissions-002.json",
                "submissions_page_002.json",
            ),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ok(fx(body)))
                .mount(&server)
                .await;
        }
        Fx {
            server,
            cache: TempDir::new().unwrap(),
        }
    }

    /// `pre` args go before `edgar` (global flags such as --config).
    fn cmd(&self, pre: &[&str], args: &[&str]) -> Command {
        let mut c = ghost();
        c.env("GHOST_ALPHA_EDGAR_USER_AGENT", UA)
            .env("GHOST_ALPHA_EDGAR_DATA_URL", self.server.uri())
            .env("GHOST_ALPHA_EDGAR_WWW_URL", self.server.uri())
            .env("GHOST_ALPHA_EDGAR_CACHE_DIR", self.cache.path())
            .args(pre)
            .arg("edgar")
            .args(args);
        c
    }

    async fn run(&self, args: &[&str]) -> Output {
        self.exec(self.cmd(&[], args)).await
    }

    async fn exec(&self, mut c: Command) -> Output {
        tokio::task::spawn_blocking(move || c.output().expect("spawn ghost-alpha"))
            .await
            .unwrap()
    }

    async fn path_hits(&self, p: &str) -> usize {
        self.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path() == p)
            .count()
    }
}

fn json_rows(o: &Output) -> Vec<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(&stdout(o))
        .unwrap_or_else(|e| panic!("stdout is not pure JSON ({e})\n{}", describe(o)));
    v.as_array().expect("array").clone()
}

fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, content).unwrap();
    p
}

const FACTS_JSON: [&str; 6] = ["facts", "EXMP", "--as-of", "2024-03-01", "--format", "json"];

// ── H2 ───────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_when_config_has_only_edgar_user_agent_should_work_without_env_or_finnhub_key() {
    let f = Fx::new().await;
    let tmp = TempDir::new().unwrap();
    let cfg = write(
        tmp.path(),
        "config.toml",
        &format!("[edgar]\nuser_agent = \"{UA}\"\n"),
    );
    let mut c = f.cmd(&["--config", cfg.to_str().unwrap()], &FACTS_JSON);
    c.env_remove("GHOST_ALPHA_EDGAR_USER_AGENT");
    let o = f.exec(c).await;
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(json_rows(&o).len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_when_env_user_agent_set_should_override_config_value() {
    let f = Fx::new().await;
    let tmp = TempDir::new().unwrap();
    let cfg = write(
        tmp.path(),
        "config.toml",
        "[edgar]\nuser_agent = \"not-an-email\"\n",
    );
    let o = f
        .exec(f.cmd(&["--config", cfg.to_str().unwrap()], &FACTS_JSON))
        .await;
    assert!(o.status.success(), "{}", describe(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_when_config_has_syntax_error_and_env_ua_set_should_work_and_warn_about_config()
{
    let f = Fx::new().await;
    let tmp = TempDir::new().unwrap();
    let cfg = write(tmp.path(), "config.toml", "[edgar\nuser_agent = = broken\n");
    let o = f
        .exec(f.cmd(&["--config", cfg.to_str().unwrap()], &FACTS_JSON))
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(json_rows(&o).len(), 3, "stdout must stay pure JSON");
    assert!(
        stderr(&o).to_lowercase().contains("config"),
        "a warning mentioning the config file is expected on stderr\n{}",
        describe(&o)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_when_no_config_file_exists_should_not_mention_config_on_stderr() {
    let f = Fx::new().await;
    let o = f.run(&FACTS_JSON).await;
    assert!(o.status.success(), "{}", describe(&o));
    assert!(
        !stderr(&o).to_lowercase().contains("config"),
        "{}",
        describe(&o)
    );
}

// ── H3 ───────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_when_since_given_should_include_older_paginated_filings() {
    let f = Fx::with("companyfacts_exmp.json", "submissions_paged.json").await;
    let o = f
        .run(&[
            "filings",
            "EXMP",
            "--form",
            "8-K",
            "--item",
            "2.02",
            "--since",
            "2021-01-01",
            "--format",
            "json",
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
            "0000999999-24-000011",
            "0000999999-23-000041",
            "0000999999-21-000040"
        ]
    );
    assert_eq!(
        f.path_hits("/submissions/CIK0000999999-submissions-001.json")
            .await,
        1
    );
    assert_eq!(
        f.path_hits("/submissions/CIK0000999999-submissions-002.json")
            .await,
        0
    );
    assert!(
        !stderr(&o).contains("--since"),
        "no coverage note when --since was given\n{}",
        describe(&o)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_when_no_since_and_older_pages_exist_should_note_coverage_on_stderr() {
    let f = Fx::with("companyfacts_exmp.json", "submissions_paged.json").await;
    let o = f
        .run(&["filings", "EXMP", "--form", "8-K", "--format", "json"])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(json_rows(&o).len(), 2, "recent only: {}", describe(&o));
    let err = stderr(&o);
    assert!(err.contains("2023-11-03"), "{}", describe(&o)); // oldest filing in `recent`
    assert!(err.contains("--since"), "{}", describe(&o));
    assert_eq!(
        f.path_hits("/submissions/CIK0000999999-submissions-001.json")
            .await,
        0
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_when_no_since_and_no_older_pages_should_not_print_coverage_note() {
    let f = Fx::new().await; // submissions_exmp.json has files: []
    let o = f
        .run(&["filings", "EXMP", "--form", "8-K", "--format", "json"])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let err = stderr(&o);
    assert!(
        !err.contains("--since") && !err.contains("2023-06-01"),
        "{}",
        describe(&o)
    );
}

// ── M3 ───────────────────────────────────────────────────────────────────────

fn is_date(t: &str) -> bool {
    let b = t.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
}

fn row_with<'a>(out: &'a str, end: &str) -> &'a str {
    out.lines()
        .find(|l| {
            let t: Vec<&str> = l.split_whitespace().collect();
            t.contains(&end) && t.iter().any(|x| x.starts_with("0000999999-"))
        })
        .unwrap_or_else(|| panic!("no row for {end}\n{out}"))
}

fn date_tokens(line: &str) -> usize {
    line.split_whitespace().filter(|t| is_date(t)).count()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_table_should_have_start_column_and_filing_fy_fp_header() {
    let f = Fx::new().await;
    let o = f.run(&["facts", "EXMP", "--as-of", "2024-03-01"]).await;
    assert!(o.status.success(), "{}", describe(&o));
    let out = stdout(&o);
    let header = out
        .lines()
        .find(|l| l.contains("END"))
        .unwrap_or_else(|| panic!("no header\n{out}"));
    assert!(header.contains("START"), "{header}");
    assert!(header.contains("FILING FY/FP"), "{header}");
    let without = header.replace("FILING FY/FP", "");
    assert!(!without.contains("FY/FP"), "{header}");
    // duration row shows its start date: START, END and FILED = 3 dates
    let r = row_with(&out, "2022-12-31");
    assert!(r.contains("2022-01-01"), "{r}");
    assert_eq!(date_tokens(r), 3, "{r}");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_table_when_instant_concept_should_leave_start_blank() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "Assets",
            "--as-of",
            "2024-03-01",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let out = stdout(&o);
    let r = row_with(&out, "2023-12-31");
    assert_eq!(date_tokens(r), 2, "END and FILED only: {r}");
}

// ── M4 ───────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_concept_unknown_should_exit_zero_and_suggest_similar_names() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "revenue",
            "--as-of",
            "2024-03-01",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert!(json_rows(&o).is_empty(), "{}", describe(&o));
    let err = stderr(&o);
    assert!(err.to_lowercase().contains("not found"), "{}", describe(&o));
    assert!(err.contains("revenue"), "{}", describe(&o));
    assert!(
        err.contains("Revenues"),
        "similar concept expected\n{}",
        describe(&o)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_unit_unknown_for_existing_concept_should_list_available_units() {
    let f = Fx::new().await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "Revenues",
            "--unit",
            "shares",
            "--as-of",
            "2024-03-01",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert!(json_rows(&o).is_empty());
    let err = stderr(&o);
    assert!(err.to_lowercase().contains("not found"), "{}", describe(&o));
    assert!(err.contains("shares"), "{}", describe(&o));
    assert!(
        err.contains("USD"),
        "available unit expected\n{}",
        describe(&o)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_many_similar_concepts_should_list_at_most_five() {
    let f = Fx::with("companyfacts_realistic.json", "submissions_exmp.json").await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "e",
            "--as-of",
            "2024-03-01",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let err = stderr(&o);
    let names = [
        "Revenues",
        "Assets",
        "NetIncomeLoss",
        "OperatingIncomeLoss",
        "SalesRevenueNet",
        "CostOfRevenue",
        "NetCashProvidedByUsedInOperatingActivities",
        "Wobble",
    ];
    let listed = names.iter().filter(|n| err.contains(*n)).count();
    assert!(
        (1..=5).contains(&listed),
        "listed {listed}\n{}",
        describe(&o)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_concept_exists_should_not_print_not_found() {
    let f = Fx::new().await;
    let o = f.run(&FACTS_JSON).await;
    assert!(o.status.success(), "{}", describe(&o));
    assert!(
        !stderr(&o).to_lowercase().contains("not found"),
        "{}",
        describe(&o)
    );
}

// ── L5 ───────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_when_home_empty_or_relative_and_no_cache_env_should_fail_naming_cache_env_var()
{
    for home in ["", "relative/home"] {
        let f = Fx::new().await;
        let mut c = f.cmd(&[], &FACTS_JSON);
        c.env_remove("GHOST_ALPHA_EDGAR_CACHE_DIR")
            .env_remove("XDG_CACHE_HOME")
            .env("HOME", home);
        let o = f.exec(c).await;
        assert!(!o.status.success(), "HOME={home:?}\n{}", describe(&o));
        assert!(
            stderr(&o).contains("GHOST_ALPHA_EDGAR_CACHE_DIR"),
            "HOME={home:?}\n{}",
            describe(&o)
        );
        assert!(!stderr(&o).contains("panicked"), "{}", describe(&o));
        assert!(
            f.server.received_requests().await.unwrap().is_empty(),
            "no request before the cache dir is known"
        );
        assert!(
            !scratch_cwd().join("relative").exists(),
            "must not create a cache dir relative to cwd"
        );
    }
}
