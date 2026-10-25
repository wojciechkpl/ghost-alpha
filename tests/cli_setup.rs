//! CLI surface of `ghost-alpha setup` and the `.gitignore` guard (spec section 4
//! and test area 10).
//!
//! There is no scripted-answers seam on the binary (prompts need a terminal), so
//! wizard behaviour is covered in `tests/setup_wizard.rs`; here we only pin flag
//! parsing and the repository hygiene rules.

use std::process::{Command, Output};

fn ghost(args: &[&str]) -> Output {
    let tmp = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_ghost-alpha"))
        .current_dir(tmp.path())
        .env("RUST_LOG", "error")
        .args(args)
        .output()
        .expect("spawn ghost-alpha")
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).to_string()
}

#[test]
fn test_setup_help_when_requested_should_document_section_yes_and_keep_models_only() {
    let o = ghost(&["setup", "--help"]);
    assert!(o.status.success());
    let h = text(&o.stdout);
    for flag in ["--section", "--yes", "--models-only"] {
        assert!(h.contains(flag), "setup --help is missing {flag}:\n{h}");
    }
    for name in ["data", "edgar", "profile", "alerts", "storage", "portfolio"] {
        assert!(
            h.contains(name),
            "setup --help should list section '{name}':\n{h}"
        );
    }
}

#[test]
fn test_setup_when_section_unknown_should_fail_listing_possible_values() {
    let o = ghost(&["setup", "--section", "bogus", "--yes"]);
    assert!(!o.status.success());
    let e = text(&o.stderr);
    assert!(e.contains("bogus"), "{e}");
    assert!(
        e.contains("possible values") && e.contains("edgar") && e.contains("profile"),
        "clap should list the valid sections:\n{e}"
    );
}

#[test]
fn test_global_config_flag_when_before_setup_should_be_accepted_by_the_parser() {
    // `--help` after the subcommand short-circuits before any prompting; this only
    // proves `--config <path>` followed by `setup --section ...` parses.
    let o = ghost(&[
        "--config",
        "custom.toml",
        "setup",
        "--section",
        "edgar",
        "--help",
    ]);
    assert!(o.status.success(), "{}", text(&o.stderr));
}

fn gitignore_lines() -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".gitignore");
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| l.trim().to_string())
        .collect()
}

#[test]
fn test_gitignore_when_read_should_ignore_user_data_folder_and_backups() {
    let lines = gitignore_lines();
    assert!(
        lines.iter().any(|l| l == "/data/"),
        "missing anchored `/data/` in .gitignore"
    );
    assert!(
        !lines.iter().any(|l| l == "data/"),
        "unanchored `data/` would also hide src/data and tests/**/data"
    );
    assert!(
        lines.iter().any(|l| l == "*.bak"),
        "missing `*.bak` in .gitignore"
    );
}
