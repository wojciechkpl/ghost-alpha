//! Behavioural contract for the guided setup wizard (pure layer, no terminal).
//!
//! Spec: GhostAlpha_Guided_Setup_Plan_2026-10-25 sections 2-5. Every test drives
//! `ghost_alpha::wizard` through `ScriptedPrompter` inside a tempdir; the real
//! working directory and real config are never touched.
//!
//! ## Scripting semantics pinned here (the implementer's `ScriptedPrompter` must honour them)
//!
//! * `ScriptedPrompter::with(key, answers)` queues answers (as text) for prompts
//!   with that stable `key` ("<section>.<name>", e.g. "edgar.user_agent").
//! * Each ask first records an `Asked`, then takes the next queued answer for its
//!   key. With no queued answer the user "presses Enter": `input` returns the
//!   default (or "" when there is none), `select` the default item, `confirm`
//!   the default, `path` the default path, `secret` returns `Ok(None)` (keep).
//! * Answer text: `select` answers are the item text ("single", "yahoo");
//!   `confirm` answers are "y"/"n"; `secret` answer "" means keep.
//! * Like dialoguer, `input` re-asks while the validator rejects the answer, and
//!   `path(.., must_exist = true)` re-asks while the file is missing. Every
//!   attempt, rejected or not, is recorded in `asked()`.
//! * If the validator rejects and nothing is queued, the prompter returns `Err`
//!   (so a mis-scripted test fails instead of looping forever).
//!
//! ## Prompt keys (stable ids)
//!
//! data.provider (select finnhub|yahoo), data.finnhub_key (secret), data.fred_key
//! (secret); edgar.user_agent; profile.state, profile.filing (select single|mfj),
//! profile.other_income; alerts.telegram_token (secret), alerts.telegram_chat_id,
//! alerts.discord_webhook (secret), alerts.critical / alerts.warning / alerts.info;
//! storage.db_path, storage.model_dir ("" = none); portfolio.enabled (confirm,
//! default n; asked only on a full run, never with `--section portfolio`),
//! portfolio.dir (path), portfolio.positions (path, must exist, asked once per
//! file), portfolio.add_more (confirm, default n, asked after each accepted
//! positions file), portfolio.vesting (input, "" = skip), portfolio.symbol,
//! portfolio.as_of; review.confirm (asked by `run_setup` unless `yes`).
//! A step only asks keys prefixed with its own section name.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use ghost_alpha::config::{AppConfig, ProfileConfig};
use ghost_alpha::portfolio::import::read_positions_auto;
use ghost_alpha::wizard::{
    apply, plan_writes, run_setup, run_steps, summarize_positions, Asked, ChangeKind,
    ExistingSetup, FileChange, ScriptedPrompter, Section, SetupOptions,
};
use tempfile::TempDir;

// ── Example data (no real personal data) ─────────────────────────────────────

const UA: &str = "Jane Doe jane.doe@example.com";
const UA_NEW: &str = "Jane Q. Doe jane.doe@example.com";
const OLD_KEY: &str = "fh-EXAMPLE-OLD-KEY-111";
const NEW_KEY: &str = "fh-EXAMPLE-NEW-KEY-222";
const FRESH_KEY: &str = "fh-EXAMPLE-FRESH-KEY-333";
const TG_TOKEN: &str = "123456:EXAMPLE-TG-TOKEN";
const TG_CHAT: &str = "-1001234567890";
const DISCORD: &str = "https://discord.com/api/webhooks/111/EXAMPLE-WEBHOOK-SECRET";
const FRED_KEY: &str = "fred-EXAMPLE-KEY-444";

const FINNHUB_VAR: &str = "GHOST_ALPHA_FINNHUB_API_KEY";

/// A complete, hand-edited config: comments, an unknown top-level key, a custom
/// `[backtest]` section with an unknown key, trailing comments.
const FULL_CONFIG: &str = r#"# My hand-edited config -- keep this comment
unknown_top_key = "keep me"

[finnhub]
api_key = "FROM_ENV"   # the real key lives in the dotenv file
requests_per_minute = 60

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
warning_score_threshold  = 72.0   # tuned by hand
info_score_threshold     = 52.0

[storage]
db_path = "my_ghost.db"

[sentiment]
model_path = "models/finbert"

# custom backtest tuning
[backtest]
mc_iterations = 777
custom_note = "unknown key inside backtest"
"#;

const FULL_ENV: &str =
    "# my note\nGHOST_ALPHA_FINNHUB_API_KEY=fh-EXAMPLE-OLD-KEY-111\nCUSTOM_VAR=1\n";

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn fixture(rel: &str) -> PathBuf {
    manifest().join("tests/fixtures").join(rel)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn cfg_path(tmp: &TempDir) -> PathBuf {
    tmp.path().join("config.toml")
}
fn env_path(tmp: &TempDir) -> PathBuf {
    tmp.path().join(".env")
}

/// Write the given files (if any) and load an `ExistingSetup` from the tempdir.
fn existing(tmp: &TempDir, config: Option<&str>, env: Option<&str>) -> ExistingSetup {
    if let Some(c) = config {
        std::fs::write(cfg_path(tmp), c).unwrap();
    }
    if let Some(e) = env {
        std::fs::write(env_path(tmp), e).unwrap();
    }
    ExistingSetup::load(&cfg_path(tmp), &env_path(tmp))
}

fn full_existing(tmp: &TempDir) -> ExistingSetup {
    existing(tmp, Some(FULL_CONFIG), Some(FULL_ENV))
}

/// Run steps then plan; returns the prompter so tests can inspect what was asked.
fn plan(
    mut p: ScriptedPrompter,
    ex: &ExistingSetup,
    section: Option<Section>,
) -> (Vec<FileChange>, ScriptedPrompter) {
    let answers = run_steps(&mut p, ex, section).expect("steps should complete");
    (plan_writes(ex, &answers), p)
}

fn change<'a>(cs: &'a [FileChange], path: &Path) -> &'a FileChange {
    cs.iter()
        .find(|c| c.path == path)
        .unwrap_or_else(|| panic!("no planned change for {}: {cs:#?}", path.display()))
}

fn has_change(cs: &[FileChange], path: &Path) -> bool {
    cs.iter().any(|c| c.path == path)
}

fn count(p: &ScriptedPrompter, key: &str) -> usize {
    p.asked().iter().filter(|a| a.key == key).count()
}

fn first_ask<'a>(p: &'a ScriptedPrompter, key: &str) -> &'a Asked {
    p.asked()
        .iter()
        .find(|a| a.key == key)
        .unwrap_or_else(|| panic!("key {key} was never asked; asked = {:?}", p.asked()))
}

fn default_of(p: &ScriptedPrompter, key: &str) -> Option<String> {
    first_ask(p, key).default.clone()
}

fn fresh_script() -> ScriptedPrompter {
    ScriptedPrompter::new()
        .with("data.provider", ["finnhub"])
        .with("data.finnhub_key", [FRESH_KEY])
        .with("edgar.user_agent", [UA])
        .with("profile.state", ["CA"])
        .with("profile.filing", ["single"])
        .with("profile.other_income", ["150000"])
}

fn portfolio_script(data_dir: &Path) -> ScriptedPrompter {
    let main = fixture("fidelity/example_main.csv");
    let small = fixture("fidelity/example_small.csv");
    ScriptedPrompter::new()
        .with("portfolio.dir", [data_dir.to_str().unwrap()])
        .with(
            "portfolio.positions",
            [main.to_str().unwrap(), small.to_str().unwrap()],
        )
        .with("portfolio.add_more", ["y", "n"])
        .with(
            "portfolio.vesting",
            [fixture("vesting/sample.csv").to_str().unwrap()],
        )
        .with("portfolio.symbol", ["MSFT"])
        .with("portfolio.as_of", ["2026-05-01"])
}

fn parse_config(contents: &str) -> AppConfig {
    toml::from_str(contents)
        .unwrap_or_else(|e| panic!("config must parse as AppConfig: {e}\n{contents}"))
}

fn assert_all_unchanged(cs: &[FileChange]) {
    for c in cs {
        assert_eq!(c.kind, ChangeKind::Unchanged, "{}", c.path.display());
    }
}

// ═════════════════════════════════════════════════════════════════════════════
// Section
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_section_from_str_when_valid_name_should_parse_every_section() {
    for (name, expected) in [
        ("data", Section::Data),
        ("edgar", Section::Edgar),
        ("profile", Section::Profile),
        ("alerts", Section::Alerts),
        ("storage", Section::Storage),
        ("portfolio", Section::Portfolio),
    ] {
        assert_eq!(name.parse::<Section>(), Ok(expected), "{name}");
        assert_eq!(expected.as_str(), name);
    }
}

#[test]
fn test_section_from_str_when_unknown_name_should_error() {
    assert!("bogus".parse::<Section>().is_err());
    assert!("".parse::<Section>().is_err());
}

#[test]
fn test_section_all_should_list_six_sections_in_wizard_order() {
    let names: Vec<&str> = Section::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        names,
        ["data", "edgar", "profile", "alerts", "storage", "portfolio"]
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// ExistingSetup
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_existing_load_when_files_missing_should_have_no_warnings_or_env_values() {
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, None, None);
    assert!(ex.warnings().is_empty(), "{:?}", ex.warnings());
    assert_eq!(ex.env_value(FINNHUB_VAR), None);
    assert_eq!(ex.config_path(), cfg_path(&tmp));
    assert_eq!(ex.env_path(), env_path(&tmp));
}

#[test]
fn test_existing_load_when_env_present_should_expose_values() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    assert_eq!(ex.env_value(FINNHUB_VAR), Some(OLD_KEY));
    assert_eq!(ex.env_value("CUSTOM_VAR"), Some("1"));
    assert_eq!(ex.env_value("NOT_THERE"), None);
}

#[test]
fn test_existing_load_when_config_unparseable_should_warn_and_use_builtin_defaults() {
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, Some("this is [not valid toml"), None);
    assert!(
        !ex.warnings().is_empty(),
        "invalid config must produce a warning"
    );

    // Pre-fill falls back to built-in defaults rather than failing.
    let (cs, p) = plan(
        ScriptedPrompter::new().with("edgar.user_agent", [UA]),
        &ex,
        Some(Section::Edgar),
    );
    assert_eq!(default_of(&p, "edgar.user_agent"), None);
    // The broken file is replaced (it exists, so: Changed, hence backed up by apply).
    let c = change(&cs, &cfg_path(&tmp));
    assert_eq!(c.kind, ChangeKind::Changed);
    assert_eq!(
        parse_config(&c.contents)
            .edgar
            .unwrap()
            .user_agent
            .as_deref(),
        Some(UA)
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 1. Fresh install
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_fresh_install_when_no_files_exist_should_plan_new_config_and_env_that_parse() {
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, None, None);
    let (cs, _) = plan(fresh_script(), &ex, None);

    assert_eq!(cs.len(), 2, "{cs:#?}");
    assert!(cs.iter().all(|c| c.kind == ChangeKind::New), "{cs:#?}");

    let cfg = parse_config(&change(&cs, &cfg_path(&tmp)).contents);
    assert_eq!(cfg.edgar.unwrap().user_agent.as_deref(), Some(UA));
    assert_eq!(
        cfg.profile,
        Some(ProfileConfig {
            state: Some("CA".into()),
            filing: Some("single".into()),
            other_income: Some(150_000.0),
        })
    );
    assert_eq!(cfg.plugins.active_provider, "finnhub");
    assert_ne!(
        cfg.finnhub.api_key, FRESH_KEY,
        "secret must not be in the config"
    );
    assert_eq!(cfg.storage.db_path, "ghost_alpha.db");

    let env = &change(&cs, &env_path(&tmp)).contents;
    assert!(
        env.lines()
            .any(|l| l == format!("{FINNHUB_VAR}={FRESH_KEY}")),
        "{env}"
    );
}

#[test]
fn test_fresh_install_when_provider_is_yahoo_should_not_create_a_secrets_file() {
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, None, None);
    let script = ScriptedPrompter::new()
        .with("data.provider", ["yahoo"])
        .with("edgar.user_agent", [UA])
        .with("profile.state", ["TX"]);
    let (cs, _) = plan(script, &ex, None);

    let cfg = parse_config(&change(&cs, &cfg_path(&tmp)).contents);
    assert_eq!(cfg.plugins.active_provider, "yahoo");
    assert!(
        cs.iter()
            .all(|c| c.path != env_path(&tmp) || c.kind != ChangeKind::New),
        "no secrets -> no new secrets file: {cs:#?}"
    );
}

#[test]
fn test_fresh_install_when_finnhub_key_blank_should_reprompt_until_provided() {
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, None, None);
    let script = ScriptedPrompter::new()
        .with("data.provider", ["finnhub"])
        .with("data.finnhub_key", ["", FRESH_KEY])
        .with("edgar.user_agent", [UA])
        .with("profile.state", ["CA"]);
    let (cs, p) = plan(script, &ex, None);
    assert_eq!(count(&p, "data.finnhub_key"), 2);
    assert!(change(&cs, &env_path(&tmp)).contents.contains(FRESH_KEY));
}

// ═════════════════════════════════════════════════════════════════════════════
// 2. Preserve user edits
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_preserve_when_one_value_changes_should_keep_every_other_config_byte() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let script = ScriptedPrompter::new().with("edgar.user_agent", [UA_NEW]);
    let (cs, _) = plan(script, &ex, None);

    let c = change(&cs, &cfg_path(&tmp));
    assert_eq!(c.kind, ChangeKind::Changed);
    let expected = FULL_CONFIG.replace(
        &format!("user_agent = \"{UA}\""),
        &format!("user_agent = \"{UA_NEW}\""),
    );
    assert_eq!(c.contents, expected);
    assert_eq!(change(&cs, &env_path(&tmp)).kind, ChangeKind::Unchanged);
}

#[test]
fn test_preserve_when_secret_changes_should_keep_other_dotenv_lines_and_leave_config_alone() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let script = ScriptedPrompter::new().with("data.finnhub_key", [NEW_KEY]);
    let (cs, _) = plan(script, &ex, None);

    let env = change(&cs, &env_path(&tmp));
    assert_eq!(env.kind, ChangeKind::Changed);
    assert_eq!(env.contents, FULL_ENV.replace(OLD_KEY, NEW_KEY));
    assert_eq!(change(&cs, &cfg_path(&tmp)).kind, ChangeKind::Unchanged);
}

#[test]
fn test_preserve_when_section_missing_from_config_should_append_without_touching_existing_text() {
    let tmp = TempDir::new().unwrap();
    let original = "# only a comment and a custom section\n[backtest]\nmc_iterations = 777\n";
    let ex = existing(&tmp, Some(original), None);
    let script = ScriptedPrompter::new().with("edgar.user_agent", [UA]);
    let (cs, _) = plan(script, &ex, Some(Section::Edgar));

    let c = change(&cs, &cfg_path(&tmp));
    assert_eq!(c.kind, ChangeKind::Changed);
    assert!(
        c.contents.starts_with(original.trim_end()),
        "existing text must be preserved verbatim as a prefix:\n{}",
        c.contents
    );
    assert_eq!(
        parse_config(&c.contents)
            .edgar
            .unwrap()
            .user_agent
            .as_deref(),
        Some(UA)
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// 3. Pre-fill
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_prefill_when_setup_complete_should_default_every_prompt_to_current_value() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let (_, p) = plan(ScriptedPrompter::new(), &ex, None);

    assert_eq!(default_of(&p, "data.provider").as_deref(), Some("finnhub"));
    assert_eq!(default_of(&p, "edgar.user_agent").as_deref(), Some(UA));
    assert_eq!(default_of(&p, "profile.state").as_deref(), Some("CA"));
    assert_eq!(default_of(&p, "profile.filing").as_deref(), Some("single"));
    let income: f64 = default_of(&p, "profile.other_income")
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(income, 150_000.0);
    for (key, want) in [
        ("alerts.critical", 92.0),
        ("alerts.warning", 72.0),
        ("alerts.info", 52.0),
    ] {
        let got: f64 = default_of(&p, key).unwrap().parse().unwrap();
        assert_eq!(got, want, "{key}");
    }
    assert_eq!(
        default_of(&p, "storage.db_path").as_deref(),
        Some("my_ghost.db")
    );
    assert_eq!(
        default_of(&p, "storage.model_dir").as_deref(),
        Some("models/finbert")
    );
    assert_eq!(default_of(&p, "portfolio.enabled").as_deref(), Some("n"));
}

#[test]
fn test_prefill_when_secret_exists_should_say_so_without_revealing_it() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let (_, p) = plan(ScriptedPrompter::new(), &ex, None);
    let ask = first_ask(&p, "data.finnhub_key");
    assert!(ask.has_existing);
    assert_eq!(
        ask.default, None,
        "a secret must never be offered as a visible default"
    );
    let dump = format!("{:?}", p.asked());
    assert!(
        !dump.contains(OLD_KEY),
        "asked log leaks the secret: {dump}"
    );
}

#[test]
fn test_prefill_when_all_defaults_accepted_should_mark_every_change_unchanged() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let (cs, _) = plan(ScriptedPrompter::new(), &ex, None);

    assert!(!cs.is_empty());
    assert_all_unchanged(&cs);
    assert_eq!(change(&cs, &cfg_path(&tmp)).contents, FULL_CONFIG);
    assert_eq!(change(&cs, &env_path(&tmp)).contents, FULL_ENV);
    for c in &cs {
        assert_eq!(c.masked_diff(), "", "unchanged file has an empty diff");
    }
}

#[test]
fn test_prefill_when_all_defaults_accepted_should_write_nothing() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let (cs, _) = plan(ScriptedPrompter::new(), &ex, None);

    let written = apply(&cs, true).unwrap();
    assert!(written.is_empty(), "{written:?}");
    assert_eq!(
        std::fs::read_to_string(cfg_path(&tmp)).unwrap(),
        FULL_CONFIG
    );
    assert!(!tmp.path().join("config.toml.bak").exists());
    assert!(!tmp.path().join(".env.bak").exists());
}

// ═════════════════════════════════════════════════════════════════════════════
// 4. Secrets
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_secrets_when_user_presses_enter_should_keep_existing_key() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let script = ScriptedPrompter::new().with("data.finnhub_key", [""]);
    let (cs, _) = plan(script, &ex, None);
    let env = change(&cs, &env_path(&tmp));
    assert_eq!(env.kind, ChangeKind::Unchanged);
    assert!(env.contents.contains(OLD_KEY));
}

#[test]
fn test_secrets_when_key_changed_should_mask_old_and_new_values_in_diff() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let script = ScriptedPrompter::new()
        .with("data.finnhub_key", [NEW_KEY])
        .with("edgar.user_agent", [UA_NEW]);
    let (cs, _) = plan(script, &ex, None);

    let env_diff = change(&cs, &env_path(&tmp)).masked_diff();
    assert!(!env_diff.is_empty(), "a changed file must have a diff");
    assert!(
        !env_diff.contains(OLD_KEY),
        "old secret leaked:\n{env_diff}"
    );
    assert!(
        !env_diff.contains(NEW_KEY),
        "new secret leaked:\n{env_diff}"
    );
    assert!(
        env_diff.contains(FINNHUB_VAR),
        "key name stays visible:\n{env_diff}"
    );
    // Round-2 contract (M1): the dotenv diff lists key names only.
    assert!(
        env_diff
            .lines()
            .any(|l| l.trim() == format!("~ {FINNHUB_VAR} (changed)")),
        "dotenv diff should say which key changed:\n{env_diff}"
    );

    // Non-secret config diff shows the real values.
    let cfg_diff = change(&cs, &cfg_path(&tmp)).masked_diff();
    assert!(
        cfg_diff.contains(UA_NEW) && cfg_diff.contains(UA),
        "{cfg_diff}"
    );
}

#[test]
fn test_secrets_when_file_is_new_should_mask_values_in_its_diff_too() {
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, None, None);
    let (cs, _) = plan(fresh_script(), &ex, None);
    let diff = change(&cs, &env_path(&tmp)).masked_diff();
    assert!(diff.contains(FINNHUB_VAR));
    assert!(!diff.contains(FRESH_KEY), "{diff}");
}

#[test]
fn test_secrets_when_all_channels_configured_should_keep_them_out_of_config_and_diffs() {
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, None, None);
    let script = fresh_script()
        .with("data.fred_key", [FRED_KEY])
        .with("alerts.telegram_token", [TG_TOKEN])
        .with("alerts.telegram_chat_id", [TG_CHAT])
        .with("alerts.discord_webhook", [DISCORD]);
    let (cs, _) = plan(script, &ex, None);

    let cfg = change(&cs, &cfg_path(&tmp));
    let env = change(&cs, &env_path(&tmp));
    for secret in [FRESH_KEY, FRED_KEY, TG_TOKEN, DISCORD] {
        assert!(!cfg.contents.contains(secret), "config.toml leaks {secret}");
        assert!(
            env.contents.contains(secret),
            "secrets file is missing {secret}"
        );
        assert!(!cfg.masked_diff().contains(secret));
        assert!(!env.masked_diff().contains(secret), "diff leaks {secret}");
    }
    assert!(env.contents.contains(TG_CHAT));
}

#[test]
#[cfg(unix)]
fn test_secrets_when_planned_should_request_mode_0600_only_for_the_secrets_file() {
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, None, None);
    let (cs, _) = plan(fresh_script(), &ex, None);
    assert_eq!(change(&cs, &env_path(&tmp)).mode, Some(0o600));
    assert_ne!(change(&cs, &cfg_path(&tmp)).mode, Some(0o600));
}

#[test]
#[cfg(unix)]
fn test_secrets_when_applied_should_leave_file_with_0600_even_if_it_was_world_readable() {
    use std::os::unix::fs::PermissionsExt;
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;

    // New file.
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, None, None);
    let (cs, _) = plan(fresh_script(), &ex, None);
    apply(&cs, true).unwrap();
    assert_eq!(mode(&env_path(&tmp)), 0o600);

    // Existing 0644 file that gets replaced.
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    std::fs::set_permissions(env_path(&tmp), std::fs::Permissions::from_mode(0o644)).unwrap();
    let script = ScriptedPrompter::new().with("data.finnhub_key", [NEW_KEY]);
    let (cs, _) = plan(script, &ex, None);
    apply(&cs, true).unwrap();
    assert_eq!(mode(&env_path(&tmp)), 0o600);
}

// ═════════════════════════════════════════════════════════════════════════════
// 5. Backup and atomicity
// ═════════════════════════════════════════════════════════════════════════════

fn changed_both(tmp: &TempDir) -> Vec<FileChange> {
    let ex = full_existing(tmp);
    let script = ScriptedPrompter::new()
        .with("data.finnhub_key", [NEW_KEY])
        .with("edgar.user_agent", [UA_NEW]);
    plan(script, &ex, None).0
}

#[test]
fn test_apply_when_files_changed_and_backup_requested_should_save_originals_as_bak() {
    let tmp = TempDir::new().unwrap();
    let cs = changed_both(&tmp);
    let written = apply(&cs, true).unwrap();

    assert!(written.contains(&cfg_path(&tmp)) && written.contains(&env_path(&tmp)));
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("config.toml.bak")).unwrap(),
        FULL_CONFIG
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join(".env.bak")).unwrap(),
        FULL_ENV
    );
    assert!(std::fs::read_to_string(cfg_path(&tmp))
        .unwrap()
        .contains(UA_NEW));
    assert!(std::fs::read_to_string(env_path(&tmp))
        .unwrap()
        .contains(NEW_KEY));
}

#[test]
fn test_apply_when_backup_not_requested_should_not_create_bak_files() {
    let tmp = TempDir::new().unwrap();
    let cs = changed_both(&tmp);
    apply(&cs, false).unwrap();
    assert!(!tmp.path().join("config.toml.bak").exists());
    assert!(!tmp.path().join(".env.bak").exists());
    assert!(std::fs::read_to_string(cfg_path(&tmp))
        .unwrap()
        .contains(UA_NEW));
}

#[test]
fn test_apply_when_files_are_new_should_create_them_without_backups() {
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, None, None);
    let (cs, _) = plan(fresh_script(), &ex, None);
    let written = apply(&cs, true).unwrap();
    assert_eq!(written.len(), 2);
    assert!(cfg_path(&tmp).is_file() && env_path(&tmp).is_file());
    let baks: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
        .collect();
    assert!(baks.is_empty(), "{baks:?}");
}

#[test]
#[cfg(unix)]
fn test_apply_when_write_fails_should_return_error_and_leave_original_intact() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = TempDir::new().unwrap();
    let cs = changed_both(&tmp);

    // Make the directory read-only so neither a temp file nor a backup can be created.
    std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let probe = tmp.path().join("probe");
    if std::fs::write(&probe, b"x").is_ok() {
        // Running as a user that ignores permissions (e.g. root): cannot simulate.
        std::fs::remove_file(&probe).unwrap();
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("permissions are not enforced here; cannot simulate a failing write");
        return;
    }

    let result = apply(&cs, true);
    std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();

    assert!(result.is_err(), "a failed write must be reported");
    assert_eq!(
        std::fs::read_to_string(cfg_path(&tmp)).unwrap(),
        FULL_CONFIG
    );
    assert_eq!(std::fs::read_to_string(env_path(&tmp)).unwrap(), FULL_ENV);
}

// ── run_setup (load -> steps -> plan -> review -> apply) ─────────────────────

fn opts(tmp: &TempDir, section: Option<Section>, yes: bool) -> SetupOptions {
    SetupOptions {
        config_path: cfg_path(tmp),
        env_path: env_path(tmp),
        section,
        yes,
    }
}

#[test]
fn test_run_setup_when_yes_should_skip_confirmation_and_write_files() {
    let tmp = TempDir::new().unwrap();
    let mut p = fresh_script();
    let written = run_setup(&mut p, &opts(&tmp, None, true)).unwrap();
    assert_eq!(
        count(&p, "review.confirm"),
        0,
        "--yes skips only the final confirmation"
    );
    assert_eq!(written.len(), 2);
    assert!(cfg_path(&tmp).is_file() && env_path(&tmp).is_file());
    // Everything before the review is still asked.
    assert!(count(&p, "edgar.user_agent") >= 1);
}

#[test]
fn test_run_setup_when_user_declines_review_should_write_nothing() {
    let tmp = TempDir::new().unwrap();
    let mut p = fresh_script().with("review.confirm", ["n"]);
    let written = run_setup(&mut p, &opts(&tmp, None, false)).unwrap();
    assert!(written.is_empty());
    assert_eq!(count(&p, "review.confirm"), 1);
    assert!(!cfg_path(&tmp).exists() && !env_path(&tmp).exists());
}

#[test]
fn test_run_setup_when_user_confirms_review_should_write_and_back_up_existing_files() {
    let tmp = TempDir::new().unwrap();
    existing(&tmp, Some(FULL_CONFIG), Some(FULL_ENV));
    let mut p = ScriptedPrompter::new()
        .with("edgar.user_agent", [UA_NEW])
        .with("review.confirm", ["y"]);
    let written = run_setup(&mut p, &opts(&tmp, None, false)).unwrap();
    assert_eq!(
        written,
        vec![cfg_path(&tmp)],
        "only the changed file is written"
    );
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("config.toml.bak")).unwrap(),
        FULL_CONFIG
    );
    assert_eq!(std::fs::read_to_string(env_path(&tmp)).unwrap(), FULL_ENV);
}

#[test]
fn test_run_setup_when_nothing_changes_should_write_nothing() {
    let tmp = TempDir::new().unwrap();
    existing(&tmp, Some(FULL_CONFIG), Some(FULL_ENV));
    let mut p = ScriptedPrompter::new();
    let written = run_setup(&mut p, &opts(&tmp, None, true)).unwrap();
    assert!(written.is_empty());
    assert!(!tmp.path().join("config.toml.bak").exists());
}

// ═════════════════════════════════════════════════════════════════════════════
// 6. Sections
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_section_edgar_when_run_should_change_only_the_edgar_value() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let script = ScriptedPrompter::new().with("edgar.user_agent", [UA_NEW]);
    let (cs, p) = plan(script, &ex, Some(Section::Edgar));

    assert!(
        p.asked().iter().all(|a| a.key.starts_with("edgar.")),
        "{:?}",
        p.asked()
    );
    let cfg = change(&cs, &cfg_path(&tmp));
    assert_eq!(cfg.kind, ChangeKind::Changed);
    assert_eq!(cfg.contents, FULL_CONFIG.replace(UA, UA_NEW));
    for c in cs.iter().filter(|c| c.path != cfg_path(&tmp)) {
        assert_eq!(c.kind, ChangeKind::Unchanged, "{}", c.path.display());
    }
}

#[test]
fn test_section_when_run_alone_should_only_ask_its_own_keys() {
    let tmp = TempDir::new().unwrap();
    for section in Section::ALL {
        let ex = full_existing(&tmp);
        let data_dir = tmp.path().join(format!("data_{}", section.as_str()));
        let mut p = portfolio_script(&data_dir);
        run_steps(&mut p, &ex, Some(section)).expect("section run");
        let prefix = format!("{}.", section.as_str());
        assert!(!p.asked().is_empty(), "{section:?} asked nothing");
        for a in p.asked() {
            assert!(
                a.key.starts_with(&prefix),
                "{section:?} asked foreign key {}",
                a.key
            );
        }
    }
}

#[test]
fn test_section_profile_when_run_should_leave_every_byte_outside_profile_identical() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let script = ScriptedPrompter::new()
        .with("profile.state", ["NY"])
        .with("profile.filing", ["mfj"])
        .with("profile.other_income", ["200000"]);
    let (cs, _) = plan(script, &ex, Some(Section::Profile));
    let c = change(&cs, &cfg_path(&tmp));
    let cfg = parse_config(&c.contents);
    assert_eq!(cfg.profile.as_ref().unwrap().state.as_deref(), Some("NY"));
    assert_eq!(cfg.profile.as_ref().unwrap().filing.as_deref(), Some("mfj"));
    assert_eq!(cfg.profile.as_ref().unwrap().other_income, Some(200_000.0));

    // Everything before [profile] and from [alerts] on is untouched.
    let head = &FULL_CONFIG[..FULL_CONFIG.find("[profile]").unwrap()];
    let tail = &FULL_CONFIG[FULL_CONFIG.find("[alerts]").unwrap()..];
    assert!(c.contents.starts_with(head), "{}", c.contents);
    assert!(c.contents.ends_with(tail), "{}", c.contents);
}

// ═════════════════════════════════════════════════════════════════════════════
// 7. Validation loops
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_validation_when_edgar_user_agent_has_no_email_should_reprompt() {
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, None, None);
    let script = ScriptedPrompter::new().with("edgar.user_agent", ["just-a-name", UA]);
    let (cs, p) = plan(script, &ex, Some(Section::Edgar));
    assert_eq!(count(&p, "edgar.user_agent"), 2);
    assert_eq!(
        parse_config(&change(&cs, &cfg_path(&tmp)).contents)
            .edgar
            .unwrap()
            .user_agent
            .as_deref(),
        Some(UA)
    );
}

#[test]
fn test_validation_when_state_unknown_should_reprompt() {
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, None, None);
    let script = ScriptedPrompter::new().with("profile.state", ["ZZ", "TX"]);
    let (cs, p) = plan(script, &ex, Some(Section::Profile));
    assert_eq!(count(&p, "profile.state"), 2);
    let profile = parse_config(&change(&cs, &cfg_path(&tmp)).contents)
        .profile
        .unwrap();
    assert_eq!(profile.state.as_deref(), Some("TX"));
}

#[test]
fn test_validation_when_other_income_negative_or_not_a_number_should_reprompt() {
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, None, None);
    let script = ScriptedPrompter::new()
        .with("profile.state", ["CA"])
        .with("profile.other_income", ["-5", "lots", "150000"]);
    let (cs, p) = plan(script, &ex, Some(Section::Profile));
    assert_eq!(count(&p, "profile.other_income"), 3);
    let profile = parse_config(&change(&cs, &cfg_path(&tmp)).contents)
        .profile
        .unwrap();
    assert_eq!(profile.other_income, Some(150_000.0));
}

#[test]
fn test_validation_when_alert_thresholds_out_of_order_should_reprompt_and_end_valid() {
    let tmp = TempDir::new().unwrap();
    let ex = existing(&tmp, None, None);
    // First round: critical (60) < warning (70) -> invalid. Second-round values are
    // chosen so that re-asking any single threshold, or all three, ends valid.
    let script = ScriptedPrompter::new()
        .with("alerts.critical", ["60", "95"])
        .with("alerts.warning", ["70", "55"])
        .with("alerts.info", ["50", "40"]);
    let (cs, p) = plan(script, &ex, Some(Section::Alerts));

    let asks: usize = ["alerts.critical", "alerts.warning", "alerts.info"]
        .iter()
        .map(|k| count(&p, k))
        .sum();
    assert!(
        asks > 3,
        "an out-of-order set must trigger at least one re-ask"
    );

    let a = parse_config(&change(&cs, &cfg_path(&tmp)).contents).alerts;
    assert!(
        a.critical_score_threshold > a.warning_score_threshold
            && a.warning_score_threshold > a.info_score_threshold,
        "{a:?}"
    );
}

#[test]
fn test_validation_when_positions_csv_unparseable_should_reprompt_and_import_only_the_good_file() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let bad = tmp.path().join("bad.csv");
    std::fs::write(&bad, "foo,bar\n1,2\n").unwrap();
    let good = fixture("fidelity/example_main.csv");
    let data_dir = tmp.path().join("data");

    let script = ScriptedPrompter::new()
        .with("portfolio.dir", [data_dir.to_str().unwrap()])
        .with(
            "portfolio.positions",
            [bad.to_str().unwrap(), good.to_str().unwrap()],
        )
        .with("portfolio.symbol", ["MSFT"])
        .with("portfolio.as_of", ["2026-05-01"]);
    let (cs, p) = plan(script, &ex, Some(Section::Portfolio));

    assert_eq!(count(&p, "portfolio.positions"), 2);
    assert!(!has_change(&cs, &data_dir.join("positions/bad.csv")));
    assert!(has_change(
        &cs,
        &data_dir.join("positions/example_main.csv")
    ));
}

#[test]
fn test_validation_when_vesting_csv_unparseable_should_reprompt() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let bad = tmp.path().join("bad_vesting.csv");
    std::fs::write(&bad, "foo,bar\n1,2\n").unwrap();
    let data_dir = tmp.path().join("data");

    let script = ScriptedPrompter::new()
        .with("portfolio.dir", [data_dir.to_str().unwrap()])
        .with(
            "portfolio.positions",
            [fixture("fidelity/example_main.csv").to_str().unwrap()],
        )
        .with(
            "portfolio.vesting",
            [
                bad.to_str().unwrap(),
                fixture("vesting/sample.csv").to_str().unwrap(),
            ],
        )
        .with("portfolio.symbol", ["MSFT"])
        .with("portfolio.as_of", ["2026-05-01"]);
    let (cs, p) = plan(script, &ex, Some(Section::Portfolio));
    assert_eq!(count(&p, "portfolio.vesting"), 2);
    assert!(has_change(&cs, &data_dir.join("vesting.csv")));
}

// ═════════════════════════════════════════════════════════════════════════════
// 8. Portfolio import
// ═════════════════════════════════════════════════════════════════════════════

#[test]
fn test_summarize_positions_when_simple_csv_should_count_rows_accounts_symbols_and_value() {
    let tmp = TempDir::new().unwrap();
    let f = tmp.path().join("p.csv");
    std::fs::write(
        &f,
        "symbol,quantity,price,account\nXYZ,100,10,BROKERAGE\nABC,40,25,BROKERAGE\n",
    )
    .unwrap();
    let s = summarize_positions(&f).unwrap();
    assert_eq!(s.rows, 2);
    assert_eq!(s.symbols, 2);
    assert_eq!(s.accounts, 1);
    assert!((s.total_value - 2000.0).abs() < 1e-6, "{s:?}");
}

#[test]
fn test_summarize_positions_when_fidelity_export_should_match_parsed_rows() {
    let f = fixture("fidelity/example_main.csv");
    let rows = read_positions_auto(&f).unwrap();
    let s = summarize_positions(&f).unwrap();
    assert_eq!(s.rows, rows.len());
    assert_eq!(
        s.symbols,
        rows.iter()
            .map(|r| r.symbol.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    );
    assert_eq!(
        s.accounts,
        rows.iter()
            .map(|r| r.account_number.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    );
    let total: f64 = rows.iter().map(|r| r.current_value).sum();
    assert!((s.total_value - total).abs() < 0.01, "{s:?} vs {total}");
}

#[test]
fn test_summarize_positions_when_unparseable_or_missing_should_error() {
    let tmp = TempDir::new().unwrap();
    let bad = tmp.path().join("bad.csv");
    std::fs::write(&bad, "foo,bar\n1,2\n").unwrap();
    assert!(summarize_positions(&bad).is_err());
    assert!(summarize_positions(&tmp.path().join("missing.csv")).is_err());
}

#[test]
fn test_portfolio_import_when_files_supplied_should_copy_them_and_write_plan_toml() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let data_dir = tmp.path().join("data");
    let script = portfolio_script(&data_dir).with("portfolio.enabled", ["y"]);
    let main = fixture("fidelity/example_main.csv");
    let small = fixture("fidelity/example_small.csv");
    let vest = fixture("vesting/sample.csv");
    let before: Vec<Vec<u8>> = [&main, &small, &vest]
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();

    let (cs, p) = plan(script, &ex, None);

    for (src, rel) in [
        (&main, "positions/example_main.csv"),
        (&small, "positions/example_small.csv"),
        (&vest, "vesting.csv"),
    ] {
        let c = change(&cs, &data_dir.join(rel));
        assert_eq!(c.kind, ChangeKind::New, "{rel}");
        assert_eq!(
            c.contents,
            std::fs::read_to_string(src).unwrap(),
            "{rel} must be a verbatim copy"
        );
    }

    let plan_toml = change(&cs, &data_dir.join("plan.toml"));
    assert_eq!(plan_toml.kind, ChangeKind::New);
    let v: toml::Value = toml::from_str(&plan_toml.contents).expect("plan.toml parses");
    assert_eq!(v.get("symbol").and_then(|s| s.as_str()), Some("MSFT"));
    assert!(
        v.get("as_of")
            .map(|d| d.to_string())
            .unwrap_or_default()
            .contains("2026-05-01"),
        "{}",
        plan_toml.contents
    );

    // A summary is shown for each accepted file (row and symbol counts).
    let s = summarize_positions(&main).unwrap();
    assert!(
        p.notes()
            .iter()
            .any(|n| n.contains(&s.rows.to_string()) && n.contains(&s.symbols.to_string())),
        "no import summary note for example_main.csv: {:?}",
        p.notes()
    );

    // Existing config/secrets are untouched by the portfolio step.
    assert_eq!(change(&cs, &cfg_path(&tmp)).kind, ChangeKind::Unchanged);

    // Originals are never modified.
    let after: Vec<Vec<u8>> = [&main, &small, &vest]
        .iter()
        .map(|p| std::fs::read(p).unwrap())
        .collect();
    assert_eq!(before, after);
}

#[test]
fn test_portfolio_import_when_positions_supplied_should_not_add_example_data_that_would_pollute_the_plan(
) {
    // Deliberate reading of the spec ("templates for anything not supplied"): the
    // example positions file and the example MSFT vesting schedule carry made-up
    // numbers that `plan --data-dir` would silently include, so they must not be
    // written into a folder holding the user's real data.
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let data_dir = tmp.path().join("data");
    let script = ScriptedPrompter::new()
        .with("portfolio.dir", [data_dir.to_str().unwrap()])
        .with(
            "portfolio.positions",
            [fixture("fidelity/example_main.csv").to_str().unwrap()],
        )
        .with("portfolio.vesting", [""])
        .with("portfolio.symbol", ["MSFT"])
        .with("portfolio.as_of", ["2026-05-01"]);
    let (cs, _) = plan(script, &ex, Some(Section::Portfolio));
    assert!(!has_change(
        &cs,
        &data_dir.join("positions/example_positions.csv")
    ));
    assert!(!has_change(&cs, &data_dir.join("vesting.csv")));
    assert!(has_change(
        &cs,
        &data_dir.join("positions/example_main.csv")
    ));
    assert!(has_change(&cs, &data_dir.join("plan.toml")));
}

#[test]
fn test_portfolio_import_when_asked_for_symbol_should_suggest_the_largest_non_cash_position() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let data_dir = tmp.path().join("data");
    let (_, p) = plan(portfolio_script(&data_dir), &ex, Some(Section::Portfolio));
    // example_main.csv: MSFT is by far the largest position.
    assert_eq!(default_of(&p, "portfolio.symbol").as_deref(), Some("MSFT"));
}

#[test]
fn test_portfolio_import_when_full_run_declines_portfolio_should_ask_no_other_portfolio_key() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let (cs, p) = plan(ScriptedPrompter::new(), &ex, None);
    let portfolio_keys: Vec<&str> = p
        .asked()
        .iter()
        .map(|a| a.key.as_str())
        .filter(|k| k.starts_with("portfolio."))
        .collect();
    assert_eq!(portfolio_keys, ["portfolio.enabled"]);
    assert_all_unchanged(&cs);
}

#[test]
fn test_portfolio_import_when_applied_should_create_a_folder_that_plan_data_dir_runs_end_to_end() {
    let tmp = TempDir::new().unwrap();
    let ex = full_existing(&tmp);
    let data_dir = tmp.path().join("data");
    let script = portfolio_script(&data_dir).with("portfolio.enabled", ["y"]);
    let (cs, _) = plan(script, &ex, None);
    apply(&cs, true).unwrap();

    let out_dir = tmp.path().join("plan_out");
    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_ghost-alpha"));
    cmd.current_dir(tmp.path()).env("RUST_LOG", "error");
    for (k, _) in std::env::vars_os() {
        let upper = k.to_string_lossy().to_uppercase();
        if upper.starts_with("GHOST_ALPHA_") || upper.starts_with("FINNHUB") {
            cmd.env_remove(&k);
        }
    }
    // No --as-of / --symbol: both must come from the generated plan.toml.
    let out = cmd
        .args(["plan", "--data-dir"])
        .arg(&data_dir)
        .args(["--paths", "20", "--out-dir"])
        .arg(&out_dir)
        .output()
        .expect("spawn ghost-alpha");
    assert!(
        out.status.success(),
        "plan failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for f in ["nav_projection.csv", "nav_mc_paths.csv", "audit.json"] {
        assert!(out_dir.join(f).is_file(), "{f} missing");
    }
}
