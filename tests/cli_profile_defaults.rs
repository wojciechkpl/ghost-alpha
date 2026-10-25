//! `[profile]` supplies defaults for `--state`, `--filing` and `--other-income`
//! on `sell-plan` / `sell-compare` (spec sections 4 and 5, test area 9).
//!
//! Resolution order: command-line flag, then `[profile]`, then (for `--state`)
//! an explicit error naming `ghost-alpha setup --section profile`; `--filing`
//! falls back to `single` and `--other-income` to `0` only when neither the flag
//! nor `[profile]` sets them.
//!
//! Assumption pinned here: the "which source was used" note goes to stderr so
//! stdout stays byte-identical to an explicit-flag run. That lets the tests
//! compare outputs instead of parsing numbers.
//!
//! Hermetic: tempdir cwd with its own `config.toml`, no dotenv, `GHOST_ALPHA_*`
//! and `FINNHUB*` env removed. Fixtures use example data (XYZ, CA/single/150000).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const BASE_CONFIG: &str = "[plugins]\nactive_provider = \"yahoo\"\n";

fn write_config(dir: &Path, profile_body: Option<&str>) -> PathBuf {
    let mut s = String::from(BASE_CONFIG);
    if let Some(body) = profile_body {
        s.push_str("\n[profile]\n");
        s.push_str(body);
    }
    let p = dir.join("config.toml");
    std::fs::write(&p, s).unwrap();
    p
}

/// `ghost-alpha --config <cfg> <sub> --ticker XYZ ... <extra>`
fn run(dir: &Path, cfg: &Path, sub: &str, extra: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ghost-alpha"));
    c.current_dir(dir)
        .env("RUST_LOG", "error")
        .arg("--config")
        .arg(cfg);
    for (k, _) in std::env::vars_os() {
        let upper = k.to_string_lossy().to_uppercase();
        if upper.starts_with("GHOST_ALPHA_") || upper.starts_with("FINNHUB") {
            c.env_remove(&k);
        }
    }
    c.arg(sub)
        .args([
            "--ticker",
            "XYZ",
            "--shares",
            "1000",
            "--cost-basis",
            "50",
            "--price",
            "100",
        ])
        .args(extra);
    c.output().expect("spawn ghost-alpha")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}
fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}
fn describe(o: &Output) -> String {
    format!(
        "status={:?}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        o.status,
        out(o),
        err(o)
    )
}

/// Run twice: with `[profile]` and no flag, vs. no profile and the explicit flags.
fn assert_profile_matches_explicit_flags(
    sub: &str,
    profile_body: &str,
    explicit_flags: &[&str],
    expect_note_mentions: &str,
) {
    let tmp = TempDir::new().unwrap();
    let with_profile = write_config(tmp.path(), Some(profile_body));
    let a = run(tmp.path(), &with_profile, sub, &[]);
    assert!(a.status.success(), "{}", describe(&a));

    let tmp2 = TempDir::new().unwrap();
    let without = write_config(tmp2.path(), None);
    let b = run(tmp2.path(), &without, sub, explicit_flags);
    assert!(b.status.success(), "{}", describe(&b));

    assert_eq!(
        out(&a),
        out(&b),
        "profile defaults must equal explicit flags"
    );
    assert!(
        err(&a).contains("profile") && err(&a).contains(expect_note_mentions),
        "stderr should say which value came from [profile]:\n{}",
        describe(&a)
    );
}

#[test]
fn test_sell_compare_when_profile_has_state_should_run_without_state_flag() {
    assert_profile_matches_explicit_flags(
        "sell-compare",
        "state = \"CA\"\n",
        &["--state", "CA"],
        "CA",
    );
}

#[test]
fn test_sell_plan_when_profile_has_state_should_run_without_state_flag() {
    assert_profile_matches_explicit_flags(
        "sell-plan",
        "state = \"CA\"\n",
        &["--state", "CA"],
        "CA",
    );
}

#[test]
fn test_sell_compare_when_state_flag_given_should_override_only_state_and_keep_other_profile_values(
) {
    let tmp = TempDir::new().unwrap();
    let with_profile = write_config(
        tmp.path(),
        Some("state = \"CA\"\nfiling = \"mfj\"\nother_income = 400000.0\n"),
    );
    let a = run(
        tmp.path(),
        &with_profile,
        "sell-compare",
        &["--state", "TX"],
    );
    assert!(a.status.success(), "{}", describe(&a));

    let tmp2 = TempDir::new().unwrap();
    let without = write_config(tmp2.path(), None);
    let b = run(
        tmp2.path(),
        &without,
        "sell-compare",
        &[
            "--state",
            "TX",
            "--filing",
            "mfj",
            "--other-income",
            "400000",
        ],
    );
    assert!(b.status.success(), "{}", describe(&b));

    assert_eq!(out(&a), out(&b));
    assert!(
        out(&a).contains("TX") && !out(&a).contains("CA"),
        "{}",
        out(&a)
    );
}

#[test]
fn test_sell_compare_when_no_profile_and_no_flag_should_fail_naming_setup_section_profile() {
    let tmp = TempDir::new().unwrap();
    let cfg = write_config(tmp.path(), None);
    let o = run(tmp.path(), &cfg, "sell-compare", &[]);
    assert!(!o.status.success(), "{}", describe(&o));
    assert!(
        err(&o).contains("setup --section profile"),
        "error must point at the wizard section:\n{}",
        describe(&o)
    );
}

#[test]
fn test_sell_plan_when_no_profile_and_no_flag_should_fail_naming_setup_section_profile() {
    let tmp = TempDir::new().unwrap();
    let cfg = write_config(tmp.path(), None);
    let o = run(tmp.path(), &cfg, "sell-plan", &[]);
    assert!(!o.status.success(), "{}", describe(&o));
    assert!(
        err(&o).contains("setup --section profile"),
        "{}",
        describe(&o)
    );
}

#[test]
fn test_sell_compare_when_profile_lacks_state_should_still_fail_naming_setup_section_profile() {
    let tmp = TempDir::new().unwrap();
    let cfg = write_config(tmp.path(), Some("filing = \"mfj\"\n"));
    let o = run(tmp.path(), &cfg, "sell-compare", &[]);
    assert!(!o.status.success(), "{}", describe(&o));
    assert!(
        err(&o).contains("setup --section profile"),
        "{}",
        describe(&o)
    );
}

#[test]
fn test_sell_compare_when_profile_has_filing_should_use_it_without_flag() {
    assert_profile_matches_explicit_flags(
        "sell-compare",
        "state = \"CA\"\nfiling = \"mfj\"\nother_income = 400000.0\n",
        &[
            "--state",
            "CA",
            "--filing",
            "mfj",
            "--other-income",
            "400000",
        ],
        "mfj",
    );
}

#[test]
fn test_sell_compare_when_filing_flag_given_should_override_profile_filing() {
    let tmp = TempDir::new().unwrap();
    let with_profile = write_config(
        tmp.path(),
        Some("state = \"CA\"\nfiling = \"mfj\"\nother_income = 400000.0\n"),
    );
    let a = run(
        tmp.path(),
        &with_profile,
        "sell-compare",
        &["--filing", "single"],
    );
    assert!(a.status.success(), "{}", describe(&a));

    let tmp2 = TempDir::new().unwrap();
    let without = write_config(tmp2.path(), None);
    let b = run(
        tmp2.path(),
        &without,
        "sell-compare",
        &[
            "--state",
            "CA",
            "--filing",
            "single",
            "--other-income",
            "400000",
        ],
    );
    assert!(b.status.success(), "{}", describe(&b));
    assert_eq!(out(&a), out(&b));
}

#[test]
fn test_sell_compare_when_neither_profile_nor_flag_sets_filing_should_default_to_single() {
    let tmp = TempDir::new().unwrap();
    let with_profile = write_config(tmp.path(), Some("state = \"CA\"\n"));
    let a = run(tmp.path(), &with_profile, "sell-compare", &[]);
    assert!(a.status.success(), "{}", describe(&a));

    let tmp2 = TempDir::new().unwrap();
    let without = write_config(tmp2.path(), None);
    let b = run(
        tmp2.path(),
        &without,
        "sell-compare",
        &["--state", "CA", "--filing", "single"],
    );
    assert!(b.status.success(), "{}", describe(&b));
    assert_eq!(out(&a), out(&b));
}

#[test]
fn test_sell_compare_when_other_income_flag_given_should_override_profile_other_income() {
    let tmp = TempDir::new().unwrap();
    let with_profile = write_config(
        tmp.path(),
        Some("state = \"CA\"\nother_income = 150000.0\n"),
    );
    let a = run(
        tmp.path(),
        &with_profile,
        "sell-compare",
        &["--other-income", "20000"],
    );
    assert!(a.status.success(), "{}", describe(&a));

    let tmp2 = TempDir::new().unwrap();
    let without = write_config(tmp2.path(), None);
    let b = run(
        tmp2.path(),
        &without,
        "sell-compare",
        &["--state", "CA", "--other-income", "20000"],
    );
    assert!(b.status.success(), "{}", describe(&b));
    assert_eq!(out(&a), out(&b));
}

// ── div-report (round 2, M5) ─────────────────────────────────────────────────
//
// Same precedence as sell-plan/sell-compare: flag, then [profile], then built-in.
// Built-ins for div-report: no state (error naming `setup --section profile`),
// filing `single`, other_income 200000 (its existing default). The report is a
// deterministic HTML file, so outputs are compared byte for byte.

fn run_div(dir: &Path, cfg: &Path, extra: &[&str]) -> (Output, Option<String>) {
    let mut args: Vec<&str> = vec!["--output", "report.html"];
    args.extend_from_slice(extra);
    let o = run(dir, cfg, "div-report", &args);
    let html = std::fs::read_to_string(dir.join("report.html")).ok();
    (o, html)
}

/// `[profile]` (no flags) must equal a profile-less config with explicit flags.
fn assert_div_profile_matches(profile_body: &str, explicit: &[&str], note_mentions: &str) {
    let tmp = TempDir::new().unwrap();
    let cfg = write_config(tmp.path(), Some(profile_body));
    let (a, html_a) = run_div(tmp.path(), &cfg, &[]);
    assert!(a.status.success(), "{}", describe(&a));

    let tmp2 = TempDir::new().unwrap();
    let cfg2 = write_config(tmp2.path(), None);
    let (b, html_b) = run_div(tmp2.path(), &cfg2, explicit);
    assert!(b.status.success(), "{}", describe(&b));

    assert!(html_a.is_some());
    assert_eq!(html_a, html_b, "profile defaults must equal explicit flags");
    assert!(
        err(&a).contains("profile") && err(&a).contains(note_mentions),
        "stderr should name the [profile] values used:\n{}",
        describe(&a)
    );
}

#[test]
fn test_div_report_when_profile_has_state_should_use_it_with_builtin_filing_and_income() {
    assert_div_profile_matches("state = \"CA\"\n", &["--state", "CA"], "CA");
}

#[test]
fn test_div_report_when_profile_has_all_values_should_use_them() {
    assert_div_profile_matches(
        "state = \"CA\"\nfiling = \"mfj\"\nother_income = 150000.0\n",
        &[
            "--state",
            "CA",
            "--filing",
            "mfj",
            "--other-income",
            "150000",
        ],
        "mfj",
    );
}

#[test]
fn test_div_report_when_flags_given_should_override_profile_value_by_value() {
    let tmp = TempDir::new().unwrap();
    let cfg = write_config(
        tmp.path(),
        Some("state = \"CA\"\nfiling = \"mfj\"\nother_income = 150000.0\n"),
    );
    let (a, html_a) = run_div(
        tmp.path(),
        &cfg,
        &["--state", "TX", "--other-income", "90000"],
    );
    assert!(a.status.success(), "{}", describe(&a));

    let tmp2 = TempDir::new().unwrap();
    let cfg2 = write_config(tmp2.path(), None);
    let (b, html_b) = run_div(
        tmp2.path(),
        &cfg2,
        &[
            "--state",
            "TX",
            "--filing",
            "mfj",
            "--other-income",
            "90000",
        ],
    );
    assert!(b.status.success(), "{}", describe(&b));
    assert!(html_a.is_some());
    assert_eq!(html_a, html_b);
}

#[test]
fn test_div_report_when_no_profile_and_no_state_flag_should_fail_naming_setup_section_profile() {
    let tmp = TempDir::new().unwrap();
    let cfg = write_config(tmp.path(), None);
    let (o, html) = run_div(tmp.path(), &cfg, &[]);
    assert!(!o.status.success(), "{}", describe(&o));
    assert!(
        err(&o).contains("setup --section profile"),
        "{}",
        describe(&o)
    );
    assert!(html.is_none(), "no report may be written on error");
}
