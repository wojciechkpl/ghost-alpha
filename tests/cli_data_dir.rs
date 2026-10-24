//! Binary-level tests for `ghost-alpha init-data` and `ghost-alpha plan --data-dir`
//! (spec §5/§6 of GhostAlpha_Import_API_Spec_2026-10-23).
//!
//! These drive the compiled CLI (`CARGO_BIN_EXE_ghost-alpha`) as a subprocess so
//! that flag parsing, env vars and CLI > plan.toml > default precedence are
//! exercised without assuming any internal function names.
//!
//! The runs are hermetic: no `--config` flag, no `config.toml`, no dotenv file
//! (cwd is a scratch dir outside the repo), and every `GHOST_ALPHA_*` /
//! `FINNHUB*` environment variable is removed. `plan` and `init-data` must work
//! without any app config or API key, and the default `--asset-classes` must fall
//! back to an embedded copy when `config/asset_classes.toml` is not under cwd.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Scratch working directory: empty, outside the repo, so no config.toml,
/// no dotenv file and no config/asset_classes.toml can be picked up from cwd.
fn scratch_cwd() -> PathBuf {
    let d = std::env::temp_dir().join(format!("ghost_alpha_cli_cwd_{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ghost_in(cwd: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ghost-alpha"));
    c.current_dir(cwd).env("RUST_LOG", "error");
    for (k, _) in std::env::vars_os() {
        let k_s = k.to_string_lossy().to_uppercase();
        if k_s.starts_with("GHOST_ALPHA_") || k_s.starts_with("FINNHUB") {
            c.env_remove(&k);
        }
    }
    c
}

fn ghost() -> Command {
    ghost_in(&scratch_cwd())
}

fn run(mut cmd: impl std::borrow::BorrowMut<Command>) -> Output {
    cmd.borrow_mut()
        .output()
        .expect("failed to spawn ghost-alpha")
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

fn write(root: &Path, rel: &str, content: &str) -> PathBuf {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, content).unwrap();
    p
}

fn audit(out_dir: &Path) -> serde_json::Value {
    let s = std::fs::read_to_string(out_dir.join("audit.json"))
        .unwrap_or_else(|e| panic!("audit.json missing in {}: {e}", out_dir.display()));
    serde_json::from_str(&s).unwrap()
}

fn find_key<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match v {
        serde_json::Value::Object(m) => m
            .get(key)
            .or_else(|| m.values().find_map(|c| find_key(c, key))),
        serde_json::Value::Array(a) => a.iter().find_map(|c| find_key(c, key)),
        _ => None,
    }
}

fn nav_rows(out_dir: &Path) -> usize {
    let s = std::fs::read_to_string(out_dir.join("nav_projection.csv")).unwrap();
    s.lines().count() - 1
}

const POSITIONS: &str = "symbol,quantity,price,cost_basis\n\
    MSFT,400,420,185\nAAPL,100,250,150\nVOO,100,600,420\nIEF,500,95,98\nSPAXX,20000,1,1\n";

const VESTING_MSFT: &str = "symbol,vesting_date,quantity\n\
    MSFT,2026-08-15,40\nMSFT,2026-11-15,40\nMSFT,2027-02-15,40\n";

const VESTING_AAPL: &str = "symbol,vesting_date,quantity\n\
    AAPL,2026-08-15,500\nAAPL,2026-11-15,500\n";

/// A hand-made data dir (simple-format positions + minimal vesting).
fn data_dir(tmp: &TempDir) -> PathBuf {
    let d = tmp.path().join("data");
    write(&d, "positions/p.csv", POSITIONS);
    write(&d, "vesting.csv", VESTING_MSFT);
    d
}

fn plan(data: &Path, out: &Path) -> Command {
    let mut c = ghost();
    c.args(["plan", "--data-dir"])
        .arg(data)
        .args(["--as-of", "2026-05-01", "--paths", "20", "--out-dir"])
        .arg(out);
    c
}

// ── init-data ────────────────────────────────────────────────────────────────

#[test]
fn init_data_creates_template_files_and_prints_next_step() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("mydata");
    let o = run(ghost().arg("init-data").arg(&dir));
    assert!(o.status.success(), "{}", describe(&o));
    for rel in [
        "positions/example_positions.csv",
        "vesting.csv",
        "symbol_map.toml",
        "plan.toml",
        "README.md",
    ] {
        assert!(dir.join(rel).is_file(), "missing {rel}\n{}", describe(&o));
    }
    let out = stdout(&o);
    assert!(
        out.contains("plan") && out.contains("--data-dir"),
        "next-step hint missing:\n{out}"
    );
    assert!(
        out.contains("mydata"),
        "next-step command should name the directory:\n{out}"
    );
}

#[test]
fn init_data_refuses_to_overwrite_without_force_but_succeeds_with_force() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("d");
    assert!(run(ghost().arg("init-data").arg(&dir)).status.success());
    let readme = dir.join("README.md");
    std::fs::write(&readme, "USER EDITED").unwrap();

    let o = run(ghost().arg("init-data").arg(&dir));
    assert!(
        !o.status.success(),
        "second init-data without --force must fail\n{}",
        describe(&o)
    );
    assert_eq!(std::fs::read_to_string(&readme).unwrap(), "USER EDITED");

    let o = run(ghost().arg("init-data").arg(&dir).arg("--force"));
    assert!(o.status.success(), "{}", describe(&o));
    assert_ne!(std::fs::read_to_string(&readme).unwrap(), "USER EDITED");
}

#[test]
fn init_data_then_plan_data_dir_runs_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("d");
    let out = tmp.path().join("out");
    assert!(run(ghost().arg("init-data").arg(&dir)).status.success());

    let o = run(plan(&dir, &out));
    assert!(
        o.status.success(),
        "plan --data-dir on fresh templates failed\n{}",
        describe(&o)
    );
    assert!(
        stdout(&o).contains("Plan pipeline complete"),
        "{}",
        describe(&o)
    );
    for f in ["nav_projection.csv", "nav_mc_paths.csv", "audit.json"] {
        assert!(out.join(f).is_file(), "missing artifact {f}");
    }
    let a = audit(&out);
    assert!(find_key(&a, "concentrated_symbol")
        .and_then(|v| v.as_str())
        .is_some());
    assert!(find_key(&a, "cost_basis_source")
        .and_then(|v| v.as_str())
        .is_some());
}

// ── plan: flags and defaults ─────────────────────────────────────────────────

#[test]
fn plan_help_lists_new_flags_and_new_withhold_default() {
    let o = run(ghost().args(["plan", "--help"]));
    assert!(o.status.success(), "{}", describe(&o));
    let h = stdout(&o);
    for flag in [
        "--data-dir",
        "--symbol",
        "--symbol-map",
        "--default-class",
        "--ltcg-rate",
        "--floor-pct",
        "--cost-basis",
        "--unwind",
    ] {
        assert!(h.contains(flag), "plan --help must list {flag}:\n{h}");
    }
    assert!(
        h.contains("GHOST_ALPHA_DATA_DIR"),
        "--data-dir must document its env var:\n{h}"
    );
    assert!(
        h.contains("0.22"),
        "withhold-rate default must be 0.22:\n{h}"
    );
    assert!(
        !h.contains("[default: 0.37]"),
        "old 0.37 default must be gone:\n{h}"
    );
}

#[test]
fn plan_help_documents_init_data_subcommand() {
    let o = run(ghost().arg("--help"));
    assert!(stdout(&o).contains("init-data"), "{}", describe(&o));
}

#[test]
fn plan_without_positions_or_data_dir_fails() {
    let o = run(ghost().args(["plan", "--as-of", "2026-05-01"]));
    assert!(!o.status.success(), "{}", describe(&o));
    assert!(
        stderr(&o).contains("--data-dir"),
        "the error should tell the user about --data-dir (not just demand --positions)\n{}",
        describe(&o)
    );
}

#[test]
fn plan_positions_flag_alone_still_works_without_data_dir() {
    let tmp = TempDir::new().unwrap();
    let pos = write(tmp.path(), "p.csv", POSITIONS);
    let out = tmp.path().join("out");
    let o = run(ghost()
        .args(["plan", "--positions"])
        .arg(&pos)
        .args([
            "--as-of",
            "2026-05-01",
            "--paths",
            "20",
            "--treatment",
            "hold",
            "--out-dir",
        ])
        .arg(&out));
    assert!(o.status.success(), "{}", describe(&o));
    assert!(out.join("audit.json").is_file());
}

#[test]
fn data_dir_can_come_from_environment_variable() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let out = tmp.path().join("out");
    let o = run(ghost()
        .env("GHOST_ALPHA_DATA_DIR", &dir)
        .args([
            "plan",
            "--as-of",
            "2026-05-01",
            "--paths",
            "20",
            "--out-dir",
        ])
        .arg(&out));
    assert!(o.status.success(), "{}", describe(&o));
    assert!(out.join("audit.json").is_file());
}

#[test]
fn data_dir_without_positions_fails_with_helpful_message() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("empty");
    std::fs::create_dir_all(&dir).unwrap();
    let o = run(plan(&dir, &tmp.path().join("out")));
    assert!(!o.status.success(), "{}", describe(&o));
    let err = stderr(&o);
    assert!(err.contains("positions"), "{}", describe(&o));
    assert!(
        !err.contains("unexpected argument")
            && !err.contains("required arguments were not provided"),
        "must fail inside discovery, not in argument parsing\n{}",
        describe(&o)
    );
}

// ── as_of: required unless in plan.toml ──────────────────────────────────────

#[test]
fn as_of_is_required_unless_set_in_plan_toml() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let out = tmp.path().join("out");

    let mut c = ghost();
    c.args(["plan", "--data-dir"])
        .arg(&dir)
        .args(["--paths", "20", "--out-dir"])
        .arg(&out);
    let o = run(&mut c);
    assert!(
        !o.status.success(),
        "no --as-of and no plan.toml as_of must fail\n{}",
        describe(&o)
    );
    let err = stderr(&o).to_lowercase();
    assert!(
        err.contains("as-of") || err.contains("as_of"),
        "{}",
        describe(&o)
    );

    write(&dir, "plan.toml", "as_of = \"2026-05-01\"\n");
    let mut c = ghost();
    c.args(["plan", "--data-dir"])
        .arg(&dir)
        .args(["--paths", "20", "--out-dir"])
        .arg(&out);
    let o = run(&mut c);
    assert!(
        o.status.success(),
        "plan.toml as_of must suffice\n{}",
        describe(&o)
    );
}

// ── precedence: CLI > plan.toml > default ────────────────────────────────────

#[test]
fn plan_toml_values_apply_when_cli_flags_absent() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    write(
        &dir,
        "plan.toml",
        "as_of = \"2026-05-01\"\nseed = 7\npaths = 25\nhorizon_years = 2\n",
    );
    let out = tmp.path().join("out");
    let o = run(ghost()
        .args(["plan", "--data-dir"])
        .arg(&dir)
        .arg("--out-dir")
        .arg(&out));
    assert!(o.status.success(), "{}", describe(&o));
    let a = audit(&out);
    assert_eq!(find_key(&a, "seed").and_then(|v| v.as_u64()), Some(7));
    assert_eq!(find_key(&a, "n_paths").and_then(|v| v.as_u64()), Some(25));
    assert_eq!(nav_rows(&out), 3, "horizon_years = 2 ⇒ years 0..=2");
}

#[test]
fn cli_flags_override_plan_toml() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    write(
        &dir,
        "plan.toml",
        "as_of = \"2020-01-01\"\nseed = 7\npaths = 25\nhorizon_years = 2\n",
    );
    let out = tmp.path().join("out");
    let o = run(ghost()
        .args(["plan", "--data-dir"])
        .arg(&dir)
        .args([
            "--as-of",
            "2026-05-01",
            "--seed",
            "9",
            "--paths",
            "30",
            "--horizon",
            "3",
            "--out-dir",
        ])
        .arg(&out));
    assert!(o.status.success(), "{}", describe(&o));
    let a = audit(&out);
    assert_eq!(find_key(&a, "seed").and_then(|v| v.as_u64()), Some(9));
    assert_eq!(find_key(&a, "n_paths").and_then(|v| v.as_u64()), Some(30));
    assert_eq!(
        nav_rows(&out),
        4,
        "CLI --horizon 3 beats plan.toml horizon_years = 2"
    );
    let csv = std::fs::read_to_string(out.join("nav_projection.csv")).unwrap();
    assert!(
        csv.contains("2026-05-01"),
        "CLI --as-of must beat plan.toml as_of:\n{csv}"
    );
    assert!(!csv.contains("2020-01-01"));
}

#[test]
fn builtin_defaults_apply_when_neither_cli_nor_plan_toml_set_a_value() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let out = tmp.path().join("out");
    let o = run(plan(&dir, &out)); // --paths 20 on CLI; seed/horizon unset anywhere
    assert!(o.status.success(), "{}", describe(&o));
    let a = audit(&out);
    assert_eq!(
        find_key(&a, "seed").and_then(|v| v.as_u64()),
        Some(42),
        "default seed"
    );
    assert_eq!(nav_rows(&out), 6, "default horizon 5 ⇒ years 0..=5");
}

#[test]
fn symbol_precedence_cli_over_plan_toml_over_auto_resolution() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);

    // auto: vesting is MSFT-only → MSFT
    let out1 = tmp.path().join("o1");
    assert!(run(plan(&dir, &out1)).status.success());
    assert_eq!(
        find_key(&audit(&out1), "concentrated_symbol").and_then(|v| v.as_str()),
        Some("MSFT")
    );

    // plan.toml beats auto
    write(&dir, "plan.toml", "symbol = \"AAPL\"\n");
    let out2 = tmp.path().join("o2");
    let o = run(plan(&dir, &out2));
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(
        find_key(&audit(&out2), "concentrated_symbol").and_then(|v| v.as_str()),
        Some("AAPL")
    );

    // CLI beats plan.toml
    let out3 = tmp.path().join("o3");
    let o = run(plan(&dir, &out3).args(["--symbol", "VOO"]));
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(
        find_key(&audit(&out3), "concentrated_symbol").and_then(|v| v.as_str()),
        Some("VOO")
    );
}

#[test]
fn explicit_positions_flag_takes_precedence_over_discovered_positions() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let other = write(
        tmp.path(),
        "other.csv",
        "symbol,quantity,price\nMSFT,10,100\nSPAXX,500,1\n",
    );
    let out = tmp.path().join("out");
    let o = run(plan(&dir, &out).arg("--positions").arg(&other));
    assert!(o.status.success(), "{}", describe(&o));
    let nav = find_key(&audit(&out), "starting_nav")
        .and_then(|v| v.as_f64())
        .unwrap();
    assert!(
        (nav - 1_500.0).abs() < 0.01,
        "explicit --positions must replace discovered files, NAV={nav}"
    );
}

#[test]
fn explicit_vesting_flag_takes_precedence_over_discovered_vesting() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let vest = write(tmp.path(), "other_vesting.csv", VESTING_AAPL);
    let out = tmp.path().join("out");
    let o = run(plan(&dir, &out).arg("--vesting").arg(&vest));
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(
        find_key(&audit(&out), "concentrated_symbol").and_then(|v| v.as_str()),
        Some("AAPL"),
        "auto-resolution must follow the explicit vesting file"
    );
}

// ── plan: cost basis / unwind / class flags ──────────────────────────────────

fn cost_source(out: &Path) -> String {
    find_key(&audit(out), "cost_basis_source")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string()
}

#[test]
fn cost_basis_source_follows_cli_plan_toml_and_positions() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);

    let o1 = tmp.path().join("o1");
    assert!(run(plan(&dir, &o1)).status.success());
    assert_eq!(cost_source(&o1), "positions");

    let o2 = tmp.path().join("o2");
    let r = run(plan(&dir, &o2).args(["--cost-basis", "100"]));
    assert!(r.status.success(), "{}", describe(&r));
    assert_eq!(cost_source(&o2), "override");

    // plan.toml explicit value also counts as an override of the derived basis.
    write(
        &dir,
        "plan.toml",
        "[unwind]\ncost_basis_per_share = 120.0\n",
    );
    let o3 = tmp.path().join("o3");
    let r = run(plan(&dir, &o3));
    assert!(r.status.success(), "{}", describe(&r));
    assert_eq!(cost_source(&o3), "override");
}

#[test]
fn cost_basis_source_is_none_without_any_basis() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("data");
    write(
        &dir,
        "positions/p.csv",
        "symbol,quantity,price\nMSFT,400,420\nSPAXX,1000,1\n",
    );
    let out = tmp.path().join("out");
    let o = run(plan(&dir, &out).args(["--symbol", "MSFT"]));
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(cost_source(&out), "none");
}

#[test]
fn unwind_flag_changes_option_b_outcome() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let o1 = tmp.path().join("o1");
    let o2 = tmp.path().join("o2");
    let r1 = run(plan(&dir, &o1).args(["--treatment", "option-b"]));
    let r2 = run(plan(&dir, &o2).args(["--treatment", "option-b", "--unwind", "1:100,2:50"]));
    assert!(r1.status.success(), "{}", describe(&r1));
    assert!(r2.status.success(), "{}", describe(&r2));
    let a = std::fs::read(o1.join("nav_projection.csv")).unwrap();
    let b = std::fs::read(o2.join("nav_projection.csv")).unwrap();
    assert_ne!(
        a, b,
        "--unwind \"1:100,2:50\" must alter the projection vs the empty default schedule"
    );
}

#[test]
fn ltcg_rate_and_floor_pct_flags_are_accepted_and_affect_results() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let base = tmp.path().join("base");
    let tuned = tmp.path().join("tuned");
    let common = ["--treatment", "option-b", "--unwind", "1:100"];
    assert!(run(plan(&dir, &base).args(common)).status.success());
    let r =
        run(plan(&dir, &tuned)
            .args(common)
            .args(["--ltcg-rate", "0.15", "--floor-pct", "0.30"]));
    assert!(r.status.success(), "{}", describe(&r));
    assert_ne!(
        std::fs::read(base.join("nav_projection.csv")).unwrap(),
        std::fs::read(tuned.join("nav_projection.csv")).unwrap()
    );
}

#[test]
fn default_class_and_symbol_map_flags_resolve_unmapped_tickers() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("data");
    write(
        &dir,
        "positions/p.csv",
        "symbol,quantity,price,cost_basis\nMSFT,400,420,185\nZZZQ,100,300,200\nSPAXX,1000,1,1\n",
    );

    // 1. unmapped ZZZQ ⇒ failure naming ZZZQ and pointing at symbol_map.toml
    let o =
        run(plan(&dir, &tmp.path().join("o1")).args(["--symbol", "MSFT", "--treatment", "hold"]));
    assert!(!o.status.success(), "{}", describe(&o));
    let err = stderr(&o);
    assert!(
        err.contains("ZZZQ") && err.contains("symbol_map.toml"),
        "{}",
        describe(&o)
    );

    // 2. --default-class resolves it
    let o = run(plan(&dir, &tmp.path().join("o2")).args([
        "--symbol",
        "MSFT",
        "--treatment",
        "hold",
        "--default-class",
        "us_large_cap_index",
    ]));
    assert!(o.status.success(), "{}", describe(&o));

    // 3. --symbol-map resolves it
    let map = write(
        tmp.path(),
        "my_map.toml",
        "[symbols]\nZZZQ = \"industrial\"\n",
    );
    let o = run(plan(&dir, &tmp.path().join("o3"))
        .args(["--symbol", "MSFT", "--treatment", "hold", "--symbol-map"])
        .arg(&map));
    assert!(o.status.success(), "{}", describe(&o));

    // 4. discovered <data-dir>/symbol_map.toml resolves it too
    write(
        &dir,
        "symbol_map.toml",
        "[symbols]\nZZZQ = \"industrial\"\n",
    );
    let o =
        run(plan(&dir, &tmp.path().join("o4")).args(["--symbol", "MSFT", "--treatment", "hold"]));
    assert!(o.status.success(), "{}", describe(&o));
}

#[test]
fn bad_default_class_name_fails_naming_the_value() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let o = run(plan(&dir, &tmp.path().join("out")).args(["--default-class", "not_a_class"]));
    assert!(!o.status.success(), "{}", describe(&o));
    assert!(stderr(&o).contains("not_a_class"), "{}", describe(&o));
}

#[test]
fn malformed_unwind_flag_fails() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    for bad in ["1-400", "x:400", "1:abc", "1:"] {
        let o = run(plan(&dir, &tmp.path().join("out")).args([
            "--treatment",
            "option-b",
            "--unwind",
            bad,
        ]));
        assert!(
            !o.status.success(),
            "--unwind {bad:?} must be rejected\n{}",
            describe(&o)
        );
        let err = stderr(&o);
        assert!(
            err.to_lowercase().contains("unwind") && !err.contains("unexpected argument"),
            "--unwind must exist and report its own parse error\n{}",
            describe(&o)
        );
    }
}

#[test]
fn withhold_rate_default_is_0_22() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let o_default = tmp.path().join("default");
    let o_022 = tmp.path().join("explicit22");
    let o_037 = tmp.path().join("explicit37");
    let t = ["--treatment", "hold"];
    assert!(run(plan(&dir, &o_default).args(t)).status.success());
    assert!(
        run(plan(&dir, &o_022).args(t).args(["--withhold-rate", "0.22"]))
            .status
            .success()
    );
    assert!(
        run(plan(&dir, &o_037).args(t).args(["--withhold-rate", "0.37"]))
            .status
            .success()
    );
    let read = |p: &Path| std::fs::read(p.join("nav_projection.csv")).unwrap();
    assert_eq!(
        read(&o_default),
        read(&o_022),
        "default must equal an explicit 0.22"
    );
    assert_ne!(
        read(&o_default),
        read(&o_037),
        "0.37 must give a different result"
    );
}

// ── H4: hermetic operation ───────────────────────────────────────────────────

#[test]
fn h4_plan_works_with_no_config_toml_no_env_and_no_asset_classes_file_in_cwd() {
    let cwd = scratch_cwd();
    assert!(!cwd.join("config.toml").exists());
    assert!(!cwd.join("config").join("asset_classes.toml").exists());
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let out = tmp.path().join("out");
    let o = run(plan(&dir, &out));
    assert!(
        o.status.success(),
        "plan must not need config.toml / Finnhub key\n{}",
        describe(&o)
    );
    assert!(out.join("audit.json").is_file());
}

#[test]
fn h4_init_data_works_with_no_config_toml() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("d");
    let o = run(ghost().arg("init-data").arg(&dir));
    assert!(o.status.success(), "{}", describe(&o));
}

#[test]
fn h4_plan_dot_data_dir_from_inside_the_data_dir_works() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("inside");
    assert!(run(ghost().arg("init-data").arg(&dir)).status.success());
    // cwd = the data dir itself; no config/ dir there => embedded asset-class fallback.
    let o = run(ghost_in(&dir).args([
        "plan",
        "--data-dir",
        ".",
        "--as-of",
        "2026-05-01",
        "--paths",
        "20",
        "--out-dir",
        "out",
    ]));
    assert!(
        o.status.success(),
        "plan --data-dir . failed\n{}",
        describe(&o)
    );
    assert!(dir.join("out").join("audit.json").is_file());
}

#[test]
fn h4_default_asset_classes_fall_back_to_embedded_copy_outside_the_repo() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let out = tmp.path().join("out");
    let o = run(ghost_in(tmp.path())
        .args(["plan", "--data-dir"])
        .arg(&dir)
        .args(["--as-of", "2026-05-01", "--paths", "20", "--out-dir"])
        .arg(&out));
    assert!(o.status.success(), "{}", describe(&o));
}

#[test]
fn h4_explicit_asset_classes_file_is_used_when_present() {
    // A real file wins over the embedded copy: use the repo's own file explicitly.
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let out = tmp.path().join("out");
    let o = run(plan(&dir, &out)
        .arg("--asset-classes")
        .arg(manifest().join("config/asset_classes.toml")));
    assert!(o.status.success(), "{}", describe(&o));
}

#[test]
fn h4_missing_explicit_asset_classes_path_is_reported_in_the_error() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let o = run(plan(&dir, &tmp.path().join("out")).args([
        "--asset-classes",
        "/nonexistent_dir_for_ghost_alpha/xyz_classes.toml",
    ]));
    assert!(!o.status.success(), "{}", describe(&o));
    assert!(
        stderr(&o).contains("xyz_classes.toml"),
        "an explicit missing --asset-classes path must NOT silently fall back and must be named\n{}",
        describe(&o)
    );
}

#[test]
fn m1_plan_toml_lowercase_symbol_is_upper_cased() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    write(&dir, "plan.toml", "symbol = \"aapl\"\n");
    let out = tmp.path().join("out");
    let o = run(plan(&dir, &out));
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(
        find_key(&audit(&out), "concentrated_symbol").and_then(|v| v.as_str()),
        Some("AAPL")
    );
}

#[test]
fn m1_cli_lowercase_symbol_is_upper_cased() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let out = tmp.path().join("out");
    let o = run(plan(&dir, &out).args(["--symbol", "msft"]));
    assert!(o.status.success(), "{}", describe(&o));
    assert_eq!(
        find_key(&audit(&out), "concentrated_symbol").and_then(|v| v.as_str()),
        Some("MSFT")
    );
}

#[test]
fn m4_cli_rejects_out_of_range_rates_with_an_error_not_a_panic() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    for args in [
        ["--withhold-rate", "1.5"],
        ["--ltcg-rate", "-0.2"],
        ["--floor-pct", "1.0"],
    ] {
        let o = run(plan(&dir, &tmp.path().join("out")).args(args));
        assert!(
            !o.status.success(),
            "{args:?} must be rejected\n{}",
            describe(&o)
        );
        assert!(
            !stderr(&o).contains("panicked"),
            "{args:?} must produce a clean error, not a panic\n{}",
            describe(&o)
        );
    }
}

#[test]
fn m4_cli_rejects_zero_paths() {
    let tmp = TempDir::new().unwrap();
    let dir = data_dir(&tmp);
    let o = run(plan(&dir, &tmp.path().join("out")).args(["--paths", "0"]));
    assert!(!o.status.success(), "{}", describe(&o));
    assert!(!stderr(&o).contains("panicked"), "{}", describe(&o));
}
