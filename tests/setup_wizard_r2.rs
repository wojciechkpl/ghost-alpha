//! Round-2 contract for the guided setup wizard (review findings C1, M1-M4, minors).
//!
//! Same conventions as `tests/setup_wizard.rs` (ScriptedPrompter keyed by prompt
//! id, tempdirs only, example data only). New or changed contract:
//!
//! * C1: `run_setup` returns `Err` naming the file when config/dotenv exists but
//!   cannot be read; nothing is written, originals stay byte-identical, no `.bak`.
//! * M1: the dotenv diff lists key names only (`~ KEY (changed)`, `+ KEY (added)`),
//!   never values, comments or context lines. The config diff masks values of keys
//!   matching `api_key|token|secret|webhook|password` (case-insensitive) as `****`.
//!   Replacing an unparseable config shows no old content, just a line saying it
//!   "will be replaced" and "backed up".
//! * M2: `Debug` of `FileChange` (dotenv) and `SetupAnswers` leaks no secrets/CSV text.
//! * M3: new confirm `portfolio.replace_existing` (default y) when `<dir>/positions/`
//!   already holds files: y renames each to `<name>.bak`, n keeps them and notes that
//!   plan will sum them. Copies always end in `.csv`. Skipping vesting while
//!   `vesting.csv` exists notes that it was kept.
//! * M4: `-` means "none" for `storage.model_dir` and `portfolio.symbol`; Enter (empty
//!   answer) keeps a non-empty default. Path cleaning and empty-path rejection happen in
//!   the steps, so they also apply under ScriptedPrompter.

use std::path::{Path, PathBuf};

use ghost_alpha::config::ENV_FRED_API_KEY;
use ghost_alpha::wizard::{
    apply, plan_writes, run_setup, run_steps, ChangeKind, ExistingSetup, FileChange,
    ScriptedPrompter, Section, SetupOptions,
};
use tempfile::TempDir;

const UA: &str = "Jane Doe jane.doe@example.com";
const UA_NEW: &str = "Jane Q. Doe jane.doe@example.com";
const NEW_KEY: &str = "fh-EXAMPLE-NEW-KEY-222";
const FRESH_KEY: &str = "fh-EXAMPLE-FRESH-KEY-333";
const FINNHUB_VAR: &str = "GHOST_ALPHA_FINNHUB_API_KEY";

const FULL_CONFIG: &str = r#"# My hand-edited config
[finnhub]
api_key = "FROM_ENV"

[plugins]
active_provider = "finnhub"

[edgar]
user_agent = "Jane Doe jane.doe@example.com"

[profile]
state = "CA"
filing = "single"
other_income = 150000.0

[alerts]
critical_score_threshold = 92.0
warning_score_threshold  = 72.0
info_score_threshold     = 52.0

[storage]
db_path = "my_ghost.db"

[sentiment]
model_path = "models/finbert"
"#;
const FULL_ENV: &str =
    "# my note\nGHOST_ALPHA_FINNHUB_API_KEY=fh-EXAMPLE-OLD-KEY-111\nCUSTOM_VAR=1\n";

const SIMPLE_OLD: &str = "symbol,quantity,price,account\nOLDCO,100,10,OLDACCT\n";

fn fixture(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel)
}
fn cfg_path(t: &TempDir) -> PathBuf {
    t.path().join("config.toml")
}
fn env_path(t: &TempDir) -> PathBuf {
    t.path().join(".env")
}
fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

fn existing(t: &TempDir, config: Option<&str>, env: Option<&str>) -> ExistingSetup {
    if let Some(c) = config {
        std::fs::write(cfg_path(t), c).unwrap();
    }
    if let Some(e) = env {
        std::fs::write(env_path(t), e).unwrap();
    }
    ExistingSetup::load(&cfg_path(t), &env_path(t))
}

fn plan(
    mut p: ScriptedPrompter,
    ex: &ExistingSetup,
    section: Option<Section>,
) -> (Vec<FileChange>, ScriptedPrompter) {
    let a = run_steps(&mut p, ex, section).expect("steps should complete");
    (plan_writes(ex, &a), p)
}

fn change<'a>(cs: &'a [FileChange], path: &Path) -> &'a FileChange {
    cs.iter()
        .find(|c| c.path == path)
        .unwrap_or_else(|| panic!("no change for {}: {cs:#?}", path.display()))
}
fn has_change(cs: &[FileChange], path: &Path) -> bool {
    cs.iter().any(|c| c.path == path)
}
fn count(p: &ScriptedPrompter, key: &str) -> usize {
    p.asked().iter().filter(|a| a.key == key).count()
}

fn opts(t: &TempDir, section: Option<Section>) -> SetupOptions {
    SetupOptions {
        config_path: cfg_path(t),
        env_path: env_path(t),
        section,
        yes: true,
    }
}

fn fresh_script() -> ScriptedPrompter {
    ScriptedPrompter::new()
        .with("data.provider", ["finnhub"])
        .with("data.finnhub_key", [FRESH_KEY])
        .with("edgar.user_agent", [UA])
        .with("profile.state", ["CA"])
}

fn no_baks(t: &TempDir) {
    for name in ["config.toml.bak", ".env.bak"] {
        assert!(!t.path().join(name).exists(), "{name} must not exist");
    }
}

/// Positions CSV (simple format) written into `dir`, returns its path.
fn write_csv(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).unwrap();
    p
}

// ═════════════════════════════════════════════════════════════════════════════
// C1. Unreadable files abort the run
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_unreadable_when_config_not_utf8_should_error_naming_file_and_write_nothing() {
    let t = TempDir::new().unwrap();
    let original = b"# Caf\xe9 settings\n[backtest]\nmy_custom = 1\n".to_vec();
    std::fs::write(cfg_path(&t), &original).unwrap();
    let mut p = ScriptedPrompter::new().with("edgar.user_agent", [UA]);

    let err = run_setup(&mut p, &opts(&t, Some(Section::Edgar))).expect_err("must refuse");

    assert!(
        format!("{err:#}").contains("config.toml"),
        "error should name the file: {err:#}"
    );
    assert_eq!(std::fs::read(cfg_path(&t)).unwrap(), original);
    no_baks(&t);
    assert!(!env_path(&t).exists());
}

#[test]
#[cfg(unix)]
fn test_unreadable_when_config_mode_000_should_error_naming_file_and_write_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let t = TempDir::new().unwrap();
    std::fs::write(cfg_path(&t), FULL_CONFIG).unwrap();
    std::fs::set_permissions(cfg_path(&t), std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read(cfg_path(&t)).is_ok() {
        eprintln!("permissions are not enforced here (root?); cannot simulate");
        return;
    }
    let mut p = ScriptedPrompter::new().with("edgar.user_agent", [UA_NEW]);
    let result = run_setup(&mut p, &opts(&t, Some(Section::Edgar)));
    std::fs::set_permissions(cfg_path(&t), std::fs::Permissions::from_mode(0o644)).unwrap();

    let err = result.expect_err("must refuse");
    assert!(format!("{err:#}").contains("config.toml"), "{err:#}");
    assert_eq!(std::fs::read_to_string(cfg_path(&t)).unwrap(), FULL_CONFIG);
    no_baks(&t);
}

#[test]
fn test_unreadable_when_dotenv_not_utf8_should_error_naming_file_and_keep_other_apps_token() {
    let t = TempDir::new().unwrap();
    let original =
        b"OTHER_APP_TOKEN=keepme\n# na\xefve\nGHOST_ALPHA_FINNHUB_API_KEY=old\n".to_vec();
    std::fs::write(env_path(&t), &original).unwrap();
    let mut p = ScriptedPrompter::new()
        .with("data.provider", ["finnhub"])
        .with("data.finnhub_key", [NEW_KEY]);

    let err = run_setup(&mut p, &opts(&t, Some(Section::Data))).expect_err("must refuse");

    assert!(
        format!("{err:#}").contains(".env"),
        "error should name the file: {err:#}"
    );
    assert_eq!(std::fs::read(env_path(&t)).unwrap(), original);
    no_baks(&t);
    assert!(!cfg_path(&t).exists(), "nothing at all may be written");
}

// ═════════════════════════════════════════════════════════════════════════════
// M1. No secrets on screen
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_diff_when_dotenv_has_commented_old_key_should_list_key_names_only() {
    let t = TempDir::new().unwrap();
    let env = "# old: GHOST_ALPHA_FINNHUB_API_KEY=sk-OLD-SECRET-123\n\
               GHOST_ALPHA_FINNHUB_API_KEY=sk-CUR-9f8e7d\n\
               CUSTOM_VAR=value-ctx-777\n";
    let ex = existing(&t, None, Some(env));
    let script = ScriptedPrompter::new()
        .with("data.provider", ["finnhub"])
        .with("data.finnhub_key", ["sk-NEW-4c3b2a"]);
    let (cs, p) = plan(script, &ex, Some(Section::Data));

    let diff = change(&cs, &env_path(&t)).masked_diff();
    for leak in [
        "OLD-SECRET",
        "9f8e7d",
        "4c3b2a",
        "sk-",
        "value-ctx-777",
        "# old",
    ] {
        assert!(!diff.contains(leak), "diff leaks `{leak}`:\n{diff}");
    }
    assert!(
        diff.lines()
            .any(|l| l.trim() == format!("~ {FINNHUB_VAR} (changed)")),
        "expected a key-name-only line:\n{diff}"
    );
    for n in p.notes() {
        assert!(!n.contains("OLD-SECRET") && !n.contains("4c3b2a"), "{n}");
    }
}

#[test]
fn test_diff_when_dotenv_has_multiline_quoted_value_should_not_show_any_of_it() {
    let t = TempDir::new().unwrap();
    let env = "GHOST_ALPHA_FINNHUB_API_KEY=old\nMULTI=\"line1\nAAAA=BBBBsecretval\n\"\n";
    let ex = existing(&t, None, Some(env));
    let script = ScriptedPrompter::new()
        .with("data.provider", ["finnhub"])
        .with("data.finnhub_key", [NEW_KEY]);
    let (cs, _) = plan(script, &ex, Some(Section::Data));
    let diff = change(&cs, &env_path(&t)).masked_diff();
    for leak in ["secretval", "BBBB", "AAAA", "line1", NEW_KEY] {
        assert!(!diff.contains(leak), "diff leaks `{leak}`:\n{diff}");
    }
}

#[test]
fn test_diff_when_dotenv_key_is_added_should_list_added_key_name_only() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, None, Some(FULL_ENV));
    let script = ScriptedPrompter::new()
        .with("data.provider", ["finnhub"])
        .with("data.fred_key", ["fred-EXAMPLE-KEY-444"]);
    let (cs, _) = plan(script, &ex, Some(Section::Data));
    let diff = change(&cs, &env_path(&t)).masked_diff();
    assert!(
        diff.lines()
            .any(|l| l.trim() == format!("+ {ENV_FRED_API_KEY} (added)")),
        "{diff}"
    );
    assert!(!diff.contains("fred-EXAMPLE"), "{diff}");
}

#[test]
fn test_diff_when_config_has_secret_looking_keys_should_mask_their_values() {
    let t = TempDir::new().unwrap();
    let config = "[edgar]\n\
        api_key = \"sk-CFG-111\"\n\
        Webhook_URL = \"https://hooks.example.com/CFGHOOK\"\n\
        user_agent = \"Jane Doe jane.doe@example.com\"\n\
        bot_token = \"tok-CFG-222\"\n\
        password = \"pw-CFG-333\"\n";
    let ex = existing(&t, Some(config), None);
    let script = ScriptedPrompter::new().with("edgar.user_agent", [UA_NEW]);
    let (cs, _) = plan(script, &ex, Some(Section::Edgar));

    let diff = change(&cs, &cfg_path(&t)).masked_diff();
    for leak in ["sk-CFG-111", "CFGHOOK", "tok-CFG-222", "pw-CFG-333"] {
        assert!(!diff.contains(leak), "config diff leaks `{leak}`:\n{diff}");
    }
    assert!(
        diff.contains("****"),
        "masked values appear as ****:\n{diff}"
    );
    assert!(
        diff.contains(UA_NEW),
        "non-secret values stay visible:\n{diff}"
    );
}

#[test]
fn test_diff_when_replacing_unparseable_config_should_show_no_old_content() {
    let t = TempDir::new().unwrap();
    let broken = "[finnhub]\napi_key = \"sk-INLINE-REAL-KEY\"\n[alerts\nbroken\n";
    let ex = existing(&t, Some(broken), None);
    let script = ScriptedPrompter::new().with("edgar.user_agent", [UA]);
    let (cs, p) = plan(script, &ex, Some(Section::Edgar));

    let c = change(&cs, &cfg_path(&t));
    assert_eq!(c.kind, ChangeKind::Changed);
    let diff = c.masked_diff();
    assert!(!diff.contains("sk-INLINE-REAL-KEY"), "{diff}");
    let lower = diff.to_lowercase();
    assert!(
        lower.contains("replaced") && lower.contains("backed up"),
        "diff should say the file will be replaced and backed up:\n{diff}"
    );
    for n in p.notes() {
        assert!(!n.contains("sk-INLINE-REAL-KEY"), "note leaks: {n}");
    }
}

#[test]
fn test_run_setup_when_replacing_unparseable_config_should_not_print_old_content_but_back_it_up() {
    let t = TempDir::new().unwrap();
    let broken = "[finnhub]\napi_key = \"sk-INLINE-REAL-KEY\"\n[alerts\nbroken\n";
    std::fs::write(cfg_path(&t), broken).unwrap();
    let mut p = ScriptedPrompter::new().with("edgar.user_agent", [UA]);
    run_setup(&mut p, &opts(&t, Some(Section::Edgar))).unwrap();
    for n in p.notes() {
        assert!(!n.contains("sk-INLINE-REAL-KEY"), "note leaks: {n}");
    }
    assert_eq!(
        std::fs::read_to_string(t.path().join("config.toml.bak")).unwrap(),
        broken
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// M2. Debug redaction
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_debug_when_dotenv_file_change_formatted_should_not_contain_secret_values() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, None, Some(FULL_ENV));
    let script = ScriptedPrompter::new()
        .with("data.provider", ["finnhub"])
        .with("data.finnhub_key", [NEW_KEY])
        .with("edgar.user_agent", [UA])
        .with("profile.state", ["CA"]);
    let (cs, _) = plan(script, &ex, None);
    let dump = format!("{:?}", change(&cs, &env_path(&t)));
    assert!(!dump.contains(NEW_KEY), "{dump}");
    assert!(!dump.contains("fh-EXAMPLE-OLD-KEY-111"), "{dump}");
}

#[test]
fn test_debug_when_answers_hold_secrets_and_portfolio_should_not_leak_them_or_csv_text() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, None, None);
    let data_dir = t.path().join("data");
    let script = fresh_script()
        .with("portfolio.enabled", ["y"])
        .with("portfolio.dir", [s(&data_dir)])
        .with(
            "portfolio.positions",
            [s(&fixture("fidelity/example_main.csv"))],
        )
        .with("portfolio.symbol", ["MSFT"])
        .with("portfolio.as_of", ["2026-05-01"]);
    let mut p = script;
    let answers = run_steps(&mut p, &ex, None).unwrap();
    let dump = format!("{answers:?}");
    for leak in [FRESH_KEY, "EX0000001", "MICROSOFT CORP", "Individual"] {
        assert!(!dump.contains(leak), "SetupAnswers Debug leaks `{leak}`");
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// M3. Re-importing a portfolio
// ═════════════════════════════════════════════════════════════════════════════

/// A data folder that already holds an old import.
fn old_data_dir(t: &TempDir) -> PathBuf {
    let d = t.path().join("data");
    std::fs::create_dir_all(d.join("positions")).unwrap();
    std::fs::write(d.join("positions/old.csv"), SIMPLE_OLD).unwrap();
    d
}

fn reimport_script(data_dir: &Path) -> ScriptedPrompter {
    ScriptedPrompter::new()
        .with("portfolio.dir", [s(data_dir)])
        .with(
            "portfolio.positions",
            [
                s(&fixture("fidelity/example_main.csv")),
                s(&fixture("fidelity/example_small.csv")),
            ],
        )
        .with("portfolio.add_more", ["y", "n"])
        .with("portfolio.vesting", [s(&fixture("vesting/sample.csv"))])
        .with("portfolio.symbol", ["MSFT"])
        .with("portfolio.as_of", ["2026-05-01"])
}

fn starting_nav(data: &Path, cwd: &Path, extra: &[&str]) -> String {
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_ghost-alpha"));
    cmd.current_dir(cwd).env("RUST_LOG", "error");
    for (k, _) in std::env::vars_os() {
        let u = k.to_string_lossy().to_uppercase();
        if u.starts_with("GHOST_ALPHA_") || u.starts_with("FINNHUB") {
            cmd.env_remove(&k);
        }
    }
    let out = cmd
        .args(["plan", "--data-dir"])
        .arg(data)
        .args(["--paths", "20", "--out-dir"])
        .arg(cwd.join("out"))
        .args(extra)
        .output()
        .unwrap();
    let so = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        out.status.success(),
        "plan failed:\n{so}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    so.lines()
        .find(|l| l.contains("Starting NAV"))
        .unwrap_or_else(|| panic!("no Starting NAV line:\n{so}"))
        .trim()
        .to_string()
}

#[test]
fn test_reimport_when_positions_dir_has_files_should_ask_replace_existing_defaulting_to_yes() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let data = old_data_dir(&t);
    let (_, p) = plan(reimport_script(&data), &ex, Some(Section::Portfolio));
    assert_eq!(count(&p, "portfolio.replace_existing"), 1);
    let ask = p
        .asked()
        .iter()
        .find(|a| a.key == "portfolio.replace_existing")
        .unwrap();
    assert_eq!(ask.default.as_deref(), Some("y"));
}

#[test]
fn test_reimport_when_positions_dir_is_new_or_empty_should_not_ask_replace_existing() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let (_, p) = plan(
        reimport_script(&t.path().join("fresh")),
        &ex,
        Some(Section::Portfolio),
    );
    assert_eq!(count(&p, "portfolio.replace_existing"), 0);

    let empty = t.path().join("empty");
    std::fs::create_dir_all(empty.join("positions")).unwrap();
    let (_, p) = plan(reimport_script(&empty), &ex, Some(Section::Portfolio));
    assert_eq!(count(&p, "portfolio.replace_existing"), 0);
}

#[test]
fn test_reimport_when_replace_is_yes_should_rename_old_files_and_plan_sees_only_new_holdings() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let data = old_data_dir(&t);
    let (cs, _) = plan(
        reimport_script(&data).with("portfolio.replace_existing", ["y"]),
        &ex,
        Some(Section::Portfolio),
    );
    apply(&cs, true).unwrap();

    assert!(
        !data.join("positions/old.csv").exists(),
        "old file must be renamed away"
    );
    assert_eq!(
        std::fs::read_to_string(data.join("positions/old.csv.bak")).unwrap(),
        SIMPLE_OLD
    );
    assert!(data.join("positions/example_main.csv").is_file());

    // `plan` reads only *.csv, so it must see exactly the new holdings: compare with a
    // hand-built folder that never had the old file.
    let reference = t.path().join("reference");
    std::fs::create_dir_all(reference.join("positions")).unwrap();
    for f in ["example_main.csv", "example_small.csv"] {
        std::fs::copy(
            fixture(&format!("fidelity/{f}")),
            reference.join("positions").join(f),
        )
        .unwrap();
    }
    std::fs::copy(fixture("vesting/sample.csv"), reference.join("vesting.csv")).unwrap();
    let flags = ["--as-of", "2026-05-01", "--symbol", "MSFT"];
    let got = starting_nav(&data, t.path(), &[]);
    let want = starting_nav(&reference, t.path(), &flags);
    assert_eq!(got, want, "old holdings must not leak into the plan");
}

#[test]
fn test_reimport_when_replace_is_no_should_keep_old_files_and_warn_that_plan_sums_them() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let data = old_data_dir(&t);
    let (cs, p) = plan(
        reimport_script(&data).with("portfolio.replace_existing", ["n"]),
        &ex,
        Some(Section::Portfolio),
    );
    apply(&cs, true).unwrap();
    assert_eq!(
        std::fs::read_to_string(data.join("positions/old.csv")).unwrap(),
        SIMPLE_OLD
    );
    assert!(!data.join("positions/old.csv.bak").exists());
    assert!(
        p.notes().iter().any(|n| n.to_lowercase().contains("sum")),
        "a note must warn that plan sums all positions files: {:?}",
        p.notes()
    );
}

#[test]
fn test_import_when_source_has_other_extension_should_copy_it_with_csv_extension() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let src = write_csv(
        t.path(),
        "holdings.txt",
        "symbol,quantity,price\nXYZ,10,5\n",
    );
    let data = t.path().join("data");
    let script = ScriptedPrompter::new()
        .with("portfolio.dir", [s(&data)])
        .with("portfolio.positions", [s(&src)])
        .with("portfolio.symbol", ["XYZ"])
        .with("portfolio.as_of", ["2026-05-01"]);
    let (cs, _) = plan(script, &ex, Some(Section::Portfolio));
    assert!(
        has_change(&cs, &data.join("positions/holdings.csv")),
        "{cs:#?}"
    );
    assert!(!has_change(&cs, &data.join("positions/holdings.txt")));
}

#[test]
fn test_reimport_when_vesting_skipped_and_vesting_csv_exists_should_note_it_was_kept() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let data = old_data_dir(&t);
    std::fs::write(
        data.join("vesting.csv"),
        "symbol,vesting_date,quantity\nXYZ,2027-01-01,5\n",
    )
    .unwrap();
    let script = ScriptedPrompter::new()
        .with("portfolio.dir", [s(&data)])
        .with(
            "portfolio.positions",
            [s(&fixture("fidelity/example_main.csv"))],
        )
        .with("portfolio.vesting", [""])
        .with("portfolio.symbol", ["MSFT"])
        .with("portfolio.as_of", ["2026-05-01"]);
    let (cs, p) = plan(script, &ex, Some(Section::Portfolio));
    assert!(
        p.notes().iter().any(|n| {
            let l = n.to_lowercase();
            l.contains("vesting") && l.contains("kept")
        }),
        "{:?}",
        p.notes()
    );
    assert!(cs
        .iter()
        .filter(|c| c.path == data.join("vesting.csv"))
        .all(|c| c.kind == ChangeKind::Unchanged));
}

// ═════════════════════════════════════════════════════════════════════════════
// M4. Terminal parity: "-" sentinel, Enter keeps defaults, path cleaning
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_sentinel_when_model_dir_is_dash_should_remove_the_existing_model_path() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let script = ScriptedPrompter::new().with("storage.model_dir", ["-"]);
    let (cs, _) = plan(script, &ex, Some(Section::Storage));
    let c = change(&cs, &cfg_path(&t));
    assert_eq!(c.kind, ChangeKind::Changed);
    assert!(!c.contents.contains("models/finbert"), "{}", c.contents);
    assert!(!c.contents.contains("model_path"), "{}", c.contents);
    assert!(
        c.contents.contains("my_ghost.db"),
        "rest of [storage] stays"
    );
}

#[test]
fn test_sentinel_when_model_dir_answer_is_empty_should_keep_the_default() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let script = ScriptedPrompter::new().with("storage.model_dir", [""]);
    let (cs, _) = plan(script, &ex, Some(Section::Storage));
    let c = change(&cs, &cfg_path(&t));
    assert_eq!(c.kind, ChangeKind::Unchanged);
    assert_eq!(c.contents, FULL_CONFIG);
}

fn symbol_script(t: &TempDir, symbol_answer: &str) -> (ScriptedPrompter, PathBuf) {
    let data = t.path().join("data");
    let p = ScriptedPrompter::new()
        .with("portfolio.dir", [s(&data)])
        .with(
            "portfolio.positions",
            [s(&fixture("fidelity/example_main.csv"))],
        )
        .with("portfolio.symbol", [symbol_answer])
        .with("portfolio.as_of", ["2026-05-01"]);
    (p, data)
}

#[test]
fn test_sentinel_when_symbol_is_dash_should_omit_symbol_from_plan_toml() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let (script, data) = symbol_script(&t, "-");
    let (cs, _) = plan(script, &ex, Some(Section::Portfolio));
    let v: toml::Value = toml::from_str(&change(&cs, &data.join("plan.toml")).contents).unwrap();
    assert!(v.get("symbol").is_none(), "{v:?}");
    assert!(v.get("as_of").is_some());
}

#[test]
fn test_sentinel_when_symbol_answer_is_empty_should_keep_the_suggested_default() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let (script, data) = symbol_script(&t, "");
    let (cs, _) = plan(script, &ex, Some(Section::Portfolio));
    let v: toml::Value = toml::from_str(&change(&cs, &data.join("plan.toml")).contents).unwrap();
    assert_eq!(v.get("symbol").and_then(|x| x.as_str()), Some("MSFT"));
}

#[test]
#[cfg(unix)]
fn test_paths_when_quoted_with_escaped_space_should_be_cleaned_inside_the_steps() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let src = write_csv(
        t.path(),
        "my positions.csv",
        "symbol,quantity,price\nXYZ,10,5\n",
    );
    let quoted = format!("'{}'", s(&src).replace(' ', "\\ "));
    let data = t.path().join("data dir");
    let quoted_dir = format!("\"{}\"", s(&data));
    let script = ScriptedPrompter::new()
        .with("portfolio.dir", [quoted_dir.as_str()])
        .with("portfolio.positions", [quoted.as_str()])
        .with("portfolio.symbol", ["XYZ"])
        .with("portfolio.as_of", ["2026-05-01"]);
    let (cs, p) = plan(script, &ex, Some(Section::Portfolio));
    assert_eq!(
        count(&p, "portfolio.positions"),
        1,
        "cleaned path accepted first time"
    );
    assert!(
        has_change(&cs, &data.join("positions/my positions.csv")),
        "{cs:#?}"
    );
}

#[test]
fn test_paths_when_answer_is_empty_should_reprompt() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let data = t.path().join("data");
    let script = ScriptedPrompter::new()
        .with("portfolio.dir", ["", s(&data)])
        .with(
            "portfolio.positions",
            ["", s(&fixture("fidelity/example_main.csv"))],
        )
        .with("portfolio.symbol", ["MSFT"])
        .with("portfolio.as_of", ["2026-05-01"]);
    let (cs, p) = plan(script, &ex, Some(Section::Portfolio));
    assert_eq!(count(&p, "portfolio.dir"), 2);
    assert_eq!(count(&p, "portfolio.positions"), 2);
    assert!(has_change(&cs, &data.join("positions/example_main.csv")));
}

#[test]
#[cfg(unix)]
fn test_paths_when_starting_with_tilde_should_expand_against_home() {
    let t = TempDir::new().unwrap();
    let home = t.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    write_csv(&home, "pos.csv", "symbol,quantity,price\nXYZ,10,5\n");
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let data = t.path().join("data");
    let script = ScriptedPrompter::new()
        .with("portfolio.dir", [s(&data)])
        .with("portfolio.positions", ["~/pos.csv"])
        .with("portfolio.symbol", ["XYZ"])
        .with("portfolio.as_of", ["2026-05-01"]);

    let saved = std::env::var_os("HOME");
    std::env::set_var("HOME", &home);
    let mut p = script;
    let result = run_steps(&mut p, &ex, Some(Section::Portfolio));
    match saved {
        Some(h) => std::env::set_var("HOME", h),
        None => std::env::remove_var("HOME"),
    }
    let answers = result.expect("`~/pos.csv` must resolve against HOME");
    let cs = plan_writes(&ex, &answers);
    assert!(has_change(&cs, &data.join("positions/pos.csv")), "{cs:#?}");
}

// ═════════════════════════════════════════════════════════════════════════════
// Minor fixes
// ═════════════════════════════════════════════════════════════════════════════

#[test]
#[cfg(unix)]
fn test_symlink_when_config_is_a_link_should_write_through_and_keep_the_link() {
    let t = TempDir::new().unwrap();
    let real = t.path().join("real.toml");
    std::fs::write(&real, FULL_CONFIG).unwrap();
    std::os::unix::fs::symlink(&real, cfg_path(&t)).unwrap();
    let mut p = ScriptedPrompter::new().with("edgar.user_agent", [UA_NEW]);
    run_setup(&mut p, &opts(&t, Some(Section::Edgar))).unwrap();

    assert!(std::fs::symlink_metadata(cfg_path(&t))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(std::fs::read_to_string(&real).unwrap().contains(UA_NEW));
}

#[test]
#[cfg(unix)]
fn test_symlink_when_bak_is_a_link_should_replace_it_and_leave_its_target_alone() {
    use std::os::unix::fs::PermissionsExt;
    let t = TempDir::new().unwrap();
    std::fs::write(env_path(&t), FULL_ENV).unwrap();
    let elsewhere = t.path().join("elsewhere.txt");
    std::fs::write(&elsewhere, "KEEP").unwrap();
    let bak = t.path().join(".env.bak");
    std::os::unix::fs::symlink(&elsewhere, &bak).unwrap();

    let mut p = ScriptedPrompter::new()
        .with("data.provider", ["finnhub"])
        .with("data.finnhub_key", [NEW_KEY]);
    run_setup(&mut p, &opts(&t, Some(Section::Data))).unwrap();

    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "KEEP");
    let md = std::fs::symlink_metadata(&bak).unwrap();
    assert!(md.file_type().is_file(), "backup must be a regular file");
    assert_eq!(md.permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::read_to_string(&bak).unwrap(), FULL_ENV);
}

#[test]
fn test_dotenv_when_file_uses_crlf_should_append_new_keys_with_crlf() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, None, Some("A=1\r\nGHOST_ALPHA_FINNHUB_API_KEY=x\r\n"));
    let script = ScriptedPrompter::new()
        .with("data.provider", ["finnhub"])
        .with("data.fred_key", ["fredkey"]);
    let (cs, _) = plan(script, &ex, Some(Section::Data));
    let text = &change(&cs, &env_path(&t)).contents;
    assert!(
        text.contains(&format!("{ENV_FRED_API_KEY}=fredkey\r\n")),
        "{text:?}"
    );
    assert!(
        text.matches('\n').count() == text.matches("\r\n").count(),
        "every line ending must be CRLF: {text:?}"
    );
}

#[test]
fn test_dotenv_when_line_has_export_and_trailing_comment_should_keep_both() {
    let t = TempDir::new().unwrap();
    let ex = existing(
        &t,
        None,
        Some("export GHOST_ALPHA_FINNHUB_API_KEY=old # note\n"),
    );
    let script = ScriptedPrompter::new()
        .with("data.provider", ["finnhub"])
        .with("data.finnhub_key", [NEW_KEY]);
    let (cs, _) = plan(script, &ex, Some(Section::Data));
    assert_eq!(
        change(&cs, &env_path(&t)).contents,
        format!("export {FINNHUB_VAR}={NEW_KEY} # note\n")
    );
}

#[test]
fn test_import_when_planned_should_request_mode_0600_for_csvs_and_plan_toml() {
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let data = t.path().join("data");
    let (cs, _) = plan(reimport_script(&data), &ex, Some(Section::Portfolio));
    for rel in [
        "positions/example_main.csv",
        "positions/example_small.csv",
        "vesting.csv",
        "plan.toml",
    ] {
        assert_eq!(change(&cs, &data.join(rel)).mode, Some(0o600), "{rel}");
    }
}

#[test]
#[cfg(unix)]
fn test_import_when_applied_should_leave_csvs_and_plan_toml_with_0600() {
    use std::os::unix::fs::PermissionsExt;
    let t = TempDir::new().unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let data = t.path().join("data");
    let (cs, _) = plan(reimport_script(&data), &ex, Some(Section::Portfolio));
    apply(&cs, true).unwrap();
    for rel in ["positions/example_main.csv", "vesting.csv", "plan.toml"] {
        let mode = std::fs::metadata(data.join(rel))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "{rel}");
    }
}

fn git(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .is_ok_and(|o| o.status.success())
}

fn not_ignored_note(p: &ScriptedPrompter) -> bool {
    p.notes().iter().any(|n| n.contains("not ignored"))
}

#[test]
fn test_git_warning_when_plan_toml_is_not_ignored_should_note_even_if_positions_are() {
    let t = TempDir::new().unwrap();
    if !git(t.path(), &["init", "-q"]) {
        eprintln!("git is not available; skipping");
        return;
    }
    // positions/ and vesting.csv are ignored, plan.toml is not.
    std::fs::write(t.path().join(".gitignore"), "positions/\nvesting.csv\n").unwrap();
    let ex = existing(&t, Some(FULL_CONFIG), Some(FULL_ENV));
    let data = t.path().join("data");
    let (_, p) = plan(reimport_script(&data), &ex, Some(Section::Portfolio));
    assert!(not_ignored_note(&p), "{:?}", p.notes());
}
