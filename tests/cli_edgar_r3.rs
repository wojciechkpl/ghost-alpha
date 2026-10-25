//! Binary-level tests for the review follow-ups N1, N3, N4, N5, N6 (hermetic, wiremock).
//!
//! Contract decisions (stderr text is free-form except for the substrings asserted here;
//! stdout, including `--format json`, must stay byte-for-byte what it was before):
//!
//!   N1  `edgar facts` stale-concept note. When the concept has facts and the series is non-empty but its
//!       last row's `end` is more than 2 years (annual) / 1 year (quarterly) before the filer's latest
//!       activity (max `filed` <= as_of over all 10-K/10-Q facts of the company), stderr names the concept,
//!       its latest end date, the filer's latest filing date and up to 5 alternative concepts (same taxonomy
//!       and unit) that have a fact filed within the same recency window and either (a) share a name stem
//!       (common prefix of at least 8 characters) or (b) when the queried concept contains "Revenue" or
//!       "Sales", contain "Revenue". Alternatives with only old facts are not listed. No note otherwise.
//!   N3  Series empty because of form filtering (concept has facts, none from the forms the periodicity
//!       accepts): stderr lists the forms present and says the annual series uses "10-K or 10-K/A" (quarterly:
//!       "10-Q or 10-Q/A"). No such message when facts exist in the right forms but are filtered by `--as-of`.
//!   N4  The `--since` coverage note is best-effort and must not cause extra submissions requests:
//!       `edgar filings` fetches the submissions document once per invocation (cache disabled by pointing
//!       GHOST_ALPHA_EDGAR_CACHE_DIR at a regular file) and a later failure never fails the command.
//!   N5  `filings.files[].name` must match ^CIK\d{10}-submissions-\d{3}\.json$ to be requested; others are
//!       skipped with a warning on stderr naming the bad entry (exit 0).
//!   N6  An older page that fails (404/5xx) is skipped with a stderr warning naming the page; results
//!       from the other sources are still printed, exit 0.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const UA: &str = "Example Research jane.doe@example.com";
const FACTS: &str = "/api/xbrl/companyfacts/CIK0000999999.json";
const MAIN: &str = "/submissions/CIK0000999999.json";
const P1: &str = "/submissions/CIK0000999999-submissions-001.json";
const P2: &str = "/submissions/CIK0000999999-submissions-002.json";

fn fx(name: &str) -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/edgar")
            .join(name),
    )
    .unwrap()
}

fn scratch_cwd() -> PathBuf {
    let d = std::env::temp_dir().join(format!("ghost_alpha_edgar_r3_cwd_{}", std::process::id()));
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
    /// Tickers + companyfacts (`facts`) + submissions main document (`main`, a fixture name or inline JSON
    /// produced by the caller) mounted at default priority; older pages are NOT mounted.
    async fn new(facts: &str, main: &str) -> Fx {
        let server = MockServer::start().await;
        for (p, body) in [
            ("/files/company_tickers.json", fx("company_tickers.json")),
            (FACTS, fx(facts)),
            (MAIN, main.to_string()),
        ] {
            Mock::given(method("GET"))
                .and(path(p))
                .respond_with(ok(body))
                .mount(&server)
                .await;
        }
        Fx {
            server,
            cache: TempDir::new().unwrap(),
        }
    }

    async fn page(&self, p: &str, fixture: &str) {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ok(fx(fixture)))
            .mount(&self.server)
            .await;
    }

    async fn page_status(&self, p: &str, status: u16) {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(status))
            .mount(&self.server)
            .await;
    }

    fn cmd(&self, cache: &Path, args: &[&str]) -> Command {
        let mut c = ghost();
        c.env("GHOST_ALPHA_EDGAR_USER_AGENT", UA)
            .env("GHOST_ALPHA_EDGAR_DATA_URL", self.server.uri())
            .env("GHOST_ALPHA_EDGAR_WWW_URL", self.server.uri())
            .env("GHOST_ALPHA_EDGAR_CACHE_DIR", cache)
            .arg("edgar")
            .args(args);
        c
    }

    async fn run(&self, args: &[&str]) -> Output {
        self.exec(self.cmd(self.cache.path(), args)).await
    }

    async fn exec(&self, mut c: Command) -> Output {
        tokio::task::spawn_blocking(move || c.output().expect("spawn ghost-alpha"))
            .await
            .unwrap()
    }

    async fn paths(&self) -> Vec<String> {
        self.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| r.url.path().to_string())
            .collect()
    }

    async fn hits(&self, p: &str) -> usize {
        self.paths().await.iter().filter(|x| *x == p).count()
    }
}

fn json_rows(o: &Output) -> Vec<serde_json::Value> {
    let v: serde_json::Value = serde_json::from_str(&stdout(o))
        .unwrap_or_else(|e| panic!("stdout is not pure JSON ({e})\n{}", describe(o)));
    v.as_array().expect("array").clone()
}

// ── N1 ───────────────────────────────────────────────────────────────────────

const AS_OF: &str = "2025-06-30";

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_concept_stopped_years_ago_should_name_active_alternatives_on_stderr()
{
    let f = Fx::new(
        "companyfacts_aapl_shaped.json",
        &fx("submissions_exmp.json"),
    )
    .await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "Revenues",
            "--as-of",
            AS_OF,
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(json_rows(&o).len(), 5, "stdout unchanged\n{}", describe(&o));
    let err = stderr(&o);
    for needle in [
        "Revenues",
        "2018-09-30", // concept's latest end
        "2025-05-02", // filer's latest filing
        "RevenueFromContractWithCustomerExcludingAssessedTax",
        "RevenueFromContractWithCustomerIncludingAssessedTax",
    ] {
        assert!(err.contains(needle), "missing {needle:?}\n{}", describe(&o));
    }
    for unwanted in ["SalesRevenueNet", "NetIncomeLoss", "OtherAssetsNoncurrent"] {
        assert!(
            !err.contains(unwanted),
            "{unwanted} is not an active revenue alternative\n{}",
            describe(&o)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_stale_concept_table_format_should_keep_stdout_and_still_note() {
    let f = Fx::new(
        "companyfacts_aapl_shaped.json",
        &fx("submissions_exmp.json"),
    )
    .await;
    let o = f
        .run(&["facts", "EXMP", "--concept", "Revenues", "--as-of", AS_OF])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert!(stdout(&o).contains("2018-09-30"));
    assert!(
        !stdout(&o).contains("RevenueFromContract"),
        "note belongs on stderr only\n{}",
        describe(&o)
    );
    assert!(stderr(&o).contains("RevenueFromContractWithCustomerExcludingAssessedTax"));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_quarterly_series_is_over_a_year_stale_should_note() {
    let f = Fx::new(
        "companyfacts_aapl_shaped.json",
        &fx("submissions_exmp.json"),
    )
    .await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "Revenues",
            "--periodicity",
            "quarterly",
            "--as-of",
            AS_OF,
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let err = stderr(&o);
    assert!(err.contains("2018-06-30"), "{}", describe(&o)); // last quarterly end
    assert!(err.contains("2025-05-02"), "{}", describe(&o));
    assert!(
        err.contains("RevenueFromContractWithCustomerExcludingAssessedTax"),
        "{}",
        describe(&o)
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_non_revenue_concept_is_stale_should_suggest_name_stem_siblings() {
    let f = Fx::new(
        "companyfacts_aapl_shaped.json",
        &fx("submissions_exmp.json"),
    )
    .await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "PropertyPlantAndEquipmentNet",
            "--as-of",
            AS_OF,
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let err = stderr(&o);
    assert!(err.contains("2018-09-30"), "{}", describe(&o));
    assert!(
        err.contains("PropertyPlantAndEquipmentNetIncludingFinanceLease"),
        "{}",
        describe(&o)
    );
    assert!(!err.contains("RevenueFromContract"), "{}", describe(&o));
    assert!(!err.contains("NetIncomeLoss"), "{}", describe(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_concept_is_current_or_only_mildly_behind_should_print_no_note() {
    let f = Fx::new(
        "companyfacts_aapl_shaped.json",
        &fx("submissions_exmp.json"),
    )
    .await;
    for concept in [
        "RevenueFromContractWithCustomerExcludingAssessedTax",
        "NetIncomeLoss",
        "OtherAssetsNoncurrent", // last annual end 2023-09-30: under two years behind
    ] {
        let o = f
            .run(&[
                "facts",
                "EXMP",
                "--concept",
                concept,
                "--as-of",
                AS_OF,
                "--format",
                "json",
            ])
            .await;
        assert!(o.status.success(), "{concept}\n{}", describe(&o));
        assert!(!json_rows(&o).is_empty());
        assert!(
            stderr(&o).trim().is_empty(),
            "{concept}: unexpected stderr\n{}",
            describe(&o)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_whole_filer_stopped_reporting_should_print_no_note() {
    let f = Fx::new(
        "companyfacts_delisted_shaped.json",
        &fx("submissions_exmp.json"),
    )
    .await;
    for periodicity in ["annual", "quarterly"] {
        let o = f
            .run(&[
                "facts",
                "EXMP",
                "--concept",
                "Revenues",
                "--periodicity",
                periodicity,
                "--as-of",
                AS_OF,
                "--format",
                "json",
            ])
            .await;
        assert!(o.status.success(), "{}", describe(&o));
        assert!(!json_rows(&o).is_empty());
        assert!(
            stderr(&o).trim().is_empty(),
            "{periodicity}: a delisted filer has no 'active' alternatives\n{}",
            describe(&o)
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_as_of_hides_recent_filings_should_judge_staleness_as_of_that_date() {
    let f = Fx::new(
        "companyfacts_aapl_shaped.json",
        &fx("submissions_exmp.json"),
    )
    .await;
    // As of 2019-06-30 the filer's latest filing is the FY2018 10-K (2018-11-15), so Revenues is current.
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "Revenues",
            "--as-of",
            "2019-06-30",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert!(stderr(&o).trim().is_empty(), "{}", describe(&o));
}

// ── N3 ───────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_only_foreign_forms_exist_should_explain_form_filtering_for_annual() {
    let f = Fx::new(
        "companyfacts_foreign_shaped.json",
        &fx("submissions_exmp.json"),
    )
    .await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "Revenues",
            "--as-of",
            AS_OF,
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert!(json_rows(&o).is_empty());
    let err = stderr(&o);
    for form in ["20-F", "40-F", "10-KT"] {
        assert!(
            err.contains(form),
            "forms present: {form}\n{}",
            describe(&o)
        );
    }
    assert!(err.contains("10-K or 10-K/A"), "{}", describe(&o));
    assert!(!err.contains("10-K/10-K/A"), "{}", describe(&o));
    assert!(err.to_lowercase().contains("annual"), "{}", describe(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_only_foreign_forms_exist_should_name_10q_forms_for_quarterly() {
    let f = Fx::new(
        "companyfacts_foreign_shaped.json",
        &fx("submissions_exmp.json"),
    )
    .await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "Revenues",
            "--periodicity",
            "quarterly",
            "--as-of",
            AS_OF,
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let err = stderr(&o);
    assert!(err.contains("20-F"), "{}", describe(&o));
    assert!(err.contains("10-Q or 10-Q/A"), "{}", describe(&o));
    assert!(!err.contains("10-Q/10-Q/A"), "{}", describe(&o));
    assert!(err.to_lowercase().contains("quarterly"), "{}", describe(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_series_empty_only_because_of_as_of_should_not_blame_form_filtering()
{
    let f = Fx::new("companyfacts_exmp.json", &fx("submissions_exmp.json")).await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "Revenues",
            "--as-of",
            "2020-01-01",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert!(json_rows(&o).is_empty());
    assert!(stderr(&o).trim().is_empty(), "{}", describe(&o));
}

// ── N4 ───────────────────────────────────────────────────────────────────────

/// A cache "directory" that is really a file: nothing can be cached, so every cache lookup is a miss.
fn uncacheable(tmp: &TempDir) -> PathBuf {
    let p = tmp.path().join("not-a-dir");
    std::fs::write(&p, "x").unwrap();
    p
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_when_nothing_is_cacheable_should_request_submissions_once() {
    let f = Fx::new("companyfacts_exmp.json", &fx("submissions_paged.json")).await;
    let tmp = TempDir::new().unwrap();
    let o = f
        .exec(f.cmd(
            &uncacheable(&tmp),
            &["filings", "999999", "--form", "8-K", "--format", "json"],
        ))
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(json_rows(&o).len(), 2);
    assert_eq!(
        f.hits(MAIN).await,
        1,
        "results and coverage note must share one submissions fetch"
    );
    // the coverage note itself is still produced from that single fetch
    assert!(stderr(&o).contains("--since"), "{}", describe(&o));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_since_when_nothing_is_cacheable_should_request_each_document_once() {
    let f = Fx::new("companyfacts_exmp.json", &fx("submissions_paged.json")).await;
    f.page(P1, "submissions_page_001.json").await;
    f.page(P2, "submissions_page_002.json").await;
    let tmp = TempDir::new().unwrap();
    let o = f
        .exec(f.cmd(
            &uncacheable(&tmp),
            &[
                "filings",
                "999999",
                "--form",
                "8-K",
                "--since",
                "2000-01-01",
                "--format",
                "json",
            ],
        ))
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(json_rows(&o).len(), 6);
    assert_eq!(f.hits(MAIN).await, 1);
    assert_eq!(f.hits(P1).await, 1);
    assert_eq!(f.hits(P2).await, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_when_submissions_fail_after_results_were_obtained_should_still_succeed()
{
    let server = MockServer::start().await;
    // first submissions request succeeds, every later one is 403 (not retried by the client)
    Mock::given(method("GET"))
        .and(path(MAIN))
        .respond_with(ok(fx("submissions_paged.json")))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(MAIN))
        .respond_with(ResponseTemplate::new(403))
        .with_priority(5)
        .mount(&server)
        .await;
    let f = Fx {
        server,
        cache: TempDir::new().unwrap(),
    };
    let tmp = TempDir::new().unwrap();
    let o = f
        .exec(f.cmd(
            &uncacheable(&tmp),
            &["filings", "999999", "--form", "8-K", "--format", "json"],
        ))
        .await;
    assert!(
        o.status.success(),
        "coverage note is best-effort\n{}",
        describe(&o)
    );
    assert_eq!(json_rows(&o).len(), 2, "{}", describe(&o));
}

// ── N5 ───────────────────────────────────────────────────────────────────────

const BAD_NAMES: [&str; 9] = [
    "../../evil.json",
    "../evil2.json",
    "CIK999999-submissions-001.json",         // not 10 digits
    "CIK0000999999-submissions-1.json",       // not 3 digits
    "CIK0000999999-submissions-002.json?x=1", // trailing junk
    "sub/CIK0000999999-submissions-002.json", // path separator
    "CIK0000999999-submissions\u{e9}01.json", // F1: multibyte at the slice boundary
    "CIK\u{ff10}\u{ff10}\u{ff10}\u{ff10}\u{ff10}\u{ff10}\u{ff10}\u{ff10}\u{ff10}\u{ff10}-submissions-001.json",
    "\u{1F600}.json",
];

fn hostile_main(extra: &[String]) -> String {
    let mut v: serde_json::Value = serde_json::from_str(&fx("submissions_paged.json")).unwrap();
    let files = v["filings"]["files"].as_array_mut().unwrap();
    files.retain(|f| f["name"] == "CIK0000999999-submissions-001.json");
    for n in BAD_NAMES
        .iter()
        .map(|s| s.to_string())
        .chain(extra.iter().cloned())
    {
        files.push(serde_json::json!({"name": n, "filingCount": 1,
            "filingFrom": "2001-01-01", "filingTo": "2020-01-01"}));
    }
    v.to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_since_when_page_names_are_hostile_should_skip_them_with_warning() {
    let outside = TempDir::new().unwrap();
    let abs = format!("{}/abs_evil.json", outside.path().display());
    let f = Fx::new(
        "companyfacts_exmp.json",
        &hostile_main(std::slice::from_ref(&abs)),
    )
    .await;
    f.page(P1, "submissions_page_001.json").await;
    // bait: any other GET returns a valid page body, so an unvalidated name would succeed and be cached
    Mock::given(method("GET"))
        .respond_with(ok(fx("submissions_page_002.json")))
        .with_priority(9)
        .mount(&f.server)
        .await;
    let tmp = TempDir::new().unwrap();
    let cache = tmp.path().join("cache");
    let o = f
        .exec(f.cmd(
            &cache,
            &[
                "filings",
                "999999",
                "--form",
                "8-K",
                "--since",
                "2000-01-01",
                "--format",
                "json",
            ],
        ))
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    // only the legitimate page contributed (3 recent 8-K rows incl. page 1 => 2 recent + 2 page-1 8-Ks)
    assert_eq!(json_rows(&o).len(), 4, "{}", describe(&o));

    let requested: Vec<String> = f.paths().await;
    for p in &requested {
        assert!(
            ["/files/company_tickers.json", MAIN, P1].contains(&p.as_str()) || p == FACTS,
            "unexpected request {p:?} in {requested:?}"
        );
    }
    // nothing written outside the cache dir
    assert!(!tmp.path().join("evil.json").exists());
    assert!(!tmp.path().join("evil2.json").exists());
    assert!(!Path::new(&abs).exists(), "absolute page name was written");
    for entry in walk(tmp.path()) {
        assert!(
            entry.starts_with(&cache),
            "{} is outside the cache",
            entry.display()
        );
    }
    // warning names the skipped entries
    let err = stderr(&o);
    assert!(err.contains("evil.json"), "{}", describe(&o));
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

// ── N6 ───────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_since_when_one_older_page_404s_should_print_other_results_and_warn() {
    let f = Fx::new("companyfacts_exmp.json", &fx("submissions_paged.json")).await;
    f.page_status(P1, 404).await;
    f.page(P2, "submissions_page_002.json").await;
    let o = f
        .run(&[
            "filings",
            "999999",
            "--form",
            "8-K",
            "--item",
            "2.02",
            "--since",
            "2000-01-01",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let accns: Vec<String> = json_rows(&o)
        .iter()
        .map(|r| r["accession_number"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        accns,
        vec![
            "0000999999-24-000011",
            "0000999999-23-000041",
            "0000999999-18-000041",
            "0000999999-12-000001"
        ]
    );
    let err = stderr(&o);
    assert!(
        err.contains("CIK0000999999-submissions-001.json"),
        "warning must name the failed page\n{}",
        describe(&o)
    );
    assert!(!err.contains("panicked"));
}

// ── F2: ranking and shape of stale-concept alternatives ──────────────────────

/// Contract for the alternatives named by the stale-concept note: same shape as the stale concept
/// (duration vs instant), a non-empty series for the same periodicity that ends inside the staleness
/// window, ranked by longest shared prefix with the requested concept, then names starting with
/// "Revenue" (revenue/sales queries), then alphabetically; at most 5.
#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_facts_when_many_revenue_distractors_exist_should_rank_asc606_concept_first() {
    let f = Fx::new(
        "companyfacts_aapl_shaped.json",
        &fx("submissions_exmp.json"),
    )
    .await;
    let o = f
        .run(&[
            "facts",
            "EXMP",
            "--concept",
            "Revenues",
            "--as-of",
            AS_OF,
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let err = stderr(&o);
    let first = "RevenueFromContractWithCustomerExcludingAssessedTax";
    let pos_first = err
        .find(first)
        .unwrap_or_else(|| panic!("{first} missing\n{}", describe(&o)));

    // every other candidate that the note names must come after it
    let eligible_others = [
        "RevenueFromContractWithCustomerIncludingAssessedTax",
        "ContractWithCustomerLiabilityRevenueRecognized",
        "IncreaseDecreaseInDeferredRevenue",
        "CostOfRevenue",
        "BusinessAcquisitionsProFormaRevenue",
        "DisposalGroupNotDiscontinuedOperationRevenue",
    ];
    let mut named = 1;
    for other in eligible_others {
        if let Some(p) = err.find(other) {
            named += 1;
            assert!(
                p > pos_first,
                "{other} listed before {first}\n{}",
                describe(&o)
            );
        }
    }
    assert!(
        named <= 5,
        "at most 5 alternatives, got {named}\n{}",
        describe(&o)
    );
    assert!(
        err.contains("RevenueFromContractWithCustomerIncludingAssessedTax"),
        "second-ranked sibling must survive the cap\n{}",
        describe(&o)
    );

    // instants never qualify for a duration concept
    for instant in ["DeferredRevenueCurrent", "DeferredRevenueNoncurrent"] {
        assert!(!err.contains(instant), "{instant}\n{}", describe(&o));
    }
    // no usable annual series ending inside the window
    for unusable in [
        "ForeignOnlyRevenue",
        "AdjustedRevenueLegacy",
        "SalesRevenueNet",
    ] {
        assert!(!err.contains(unusable), "{unusable}\n{}", describe(&o));
    }
}

// ── F3: stop fetching older pages after a 403 ────────────────────────────────

const P3: &str = "/submissions/CIK0000999999-submissions-003.json";

fn three_page_main() -> String {
    let mut v: serde_json::Value = serde_json::from_str(&fx("submissions_paged.json")).unwrap();
    v["filings"]["files"].as_array_mut().unwrap().push(
        serde_json::json!({"name": "CIK0000999999-submissions-003.json", "filingCount": 1,
            "filingFrom": "2009-05-01", "filingTo": "2009-05-01"}),
    );
    v.to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn test_edgar_filings_since_when_an_older_page_returns_403_should_stop_and_warn_once() {
    let f = Fx::new("companyfacts_exmp.json", &three_page_main()).await;
    f.page(P1, "submissions_page_001.json").await;
    f.page_status(P2, 403).await;
    f.page(P3, "submissions_page_003.json").await;
    let o = f
        .run(&[
            "filings",
            "999999",
            "--form",
            "8-K",
            "--item",
            "2.02",
            "--since",
            "2000-01-01",
            "--format",
            "json",
        ])
        .await;
    assert!(o.status.success(), "{}", describe(&o));
    let accns: Vec<String> = json_rows(&o)
        .iter()
        .map(|r| r["accession_number"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        accns,
        vec![
            "0000999999-24-000011",
            "0000999999-23-000041",
            "0000999999-21-000040",
            "0000999999-19-000020"
        ],
        "results from pages before the 403 are kept\n{}",
        describe(&o)
    );
    assert_eq!(f.hits(P1).await, 1);
    assert_eq!(f.hits(P2).await, 1, "403 is not retried");
    assert_eq!(
        f.hits(P3).await,
        0,
        "pages after a 403 must not be requested"
    );

    let err = stderr(&o);
    assert!(err.contains("403"), "{}", describe(&o));
    assert!(err.contains("skipped"), "{}", describe(&o));
    assert_eq!(
        err.lines().filter(|l| l.contains("403")).count(),
        1,
        "one summary warning, not one per page\n{}",
        describe(&o)
    );
}
