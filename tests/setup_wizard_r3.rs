//! Round-3 regressions for the guided setup wizard (example data only).

use std::path::{Path, PathBuf};

use ghost_alpha::wizard::{
    apply, plan_writes, run_setup, run_steps, ExistingSetup, ScriptedPrompter, Section,
    SetupOptions,
};
use tempfile::TempDir;

const NEW_KEY: &str = "fh-EXAMPLE-NEW-KEY-222";

#[test]
fn test_dotenv_when_quote_is_unterminated_should_end_value_at_its_own_line() {
    let t = TempDir::new().unwrap();
    let env_path = t.path().join(".env");
    std::fs::write(
        &env_path,
        "GHOST_ALPHA_FINNHUB_API_KEY=\"abc\nOTHER_SECRET=keepme\nANOTHER=1\n",
    )
    .unwrap();

    // Planned diff must not report the neighbouring keys as removed.
    let ex = ExistingSetup::load(&t.path().join("config.toml"), &env_path);
    let mut p = ScriptedPrompter::new()
        .with("data.provider", ["finnhub"])
        .with("data.finnhub_key", [NEW_KEY]);
    let answers = run_steps(&mut p, &ex, Some(Section::Data)).unwrap();
    let cs = plan_writes(&ex, &answers);
    let change = cs.iter().find(|c| c.path == env_path).expect("env change");
    let diff = change.masked_diff();
    for name in ["OTHER_SECRET", "ANOTHER"] {
        assert!(
            !diff.contains(name),
            "{name} must not appear in the diff:\n{diff}"
        );
    }

    // End to end.
    let mut p = ScriptedPrompter::new()
        .with("data.provider", ["finnhub"])
        .with("data.finnhub_key", [NEW_KEY]);
    let opts = SetupOptions {
        config_path: t.path().join("config.toml"),
        env_path: env_path.clone(),
        section: Some(Section::Data),
        yes: true,
    };
    run_setup(&mut p, &opts).unwrap();
    let after = std::fs::read_to_string(&env_path).unwrap();
    assert!(after.contains("OTHER_SECRET=keepme\n"), "{after:?}");
    assert!(after.contains("ANOTHER=1\n"), "{after:?}");
    assert!(
        after
            .lines()
            .any(|l| l == format!("GHOST_ALPHA_FINNHUB_API_KEY={NEW_KEY}")),
        "{after:?}"
    );
    assert_eq!(
        after.matches("GHOST_ALPHA_FINNHUB_API_KEY").count(),
        1,
        "{after:?}"
    );
}

fn read_dir_files(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect()
}

#[test]
fn test_replace_existing_when_case_variant_name_collides_should_retire_before_writing() {
    let t = TempDir::new().unwrap();
    let data = t.path().join("data");
    std::fs::create_dir_all(data.join("positions")).unwrap();
    let old = "symbol,quantity,price\nOLDCO,100,10\n";
    let new = "symbol,quantity,price\nXYZ,10,5\n";
    std::fs::write(data.join("positions/acct.CSV"), old).unwrap();
    let src_dir = t.path().join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    let src = src_dir.join("acct.csv");
    std::fs::write(&src, new).unwrap();

    let ex = ExistingSetup::load(&t.path().join("config.toml"), &t.path().join(".env"));
    let mut p = ScriptedPrompter::new()
        .with("portfolio.dir", [data.to_str().unwrap()])
        .with("portfolio.positions", [src.to_str().unwrap()])
        .with("portfolio.replace_existing", ["y"])
        .with("portfolio.symbol", ["XYZ"])
        .with("portfolio.as_of", ["2026-05-01"]);
    let answers = run_steps(&mut p, &ex, Some(Section::Portfolio)).unwrap();
    let cs = plan_writes(&ex, &answers);
    apply(&cs, true).unwrap();

    // Content checks only: directory listings differ on case-insensitive filesystems.
    let new_path = data.join("positions/acct.csv");
    assert_eq!(
        std::fs::read_to_string(&new_path).unwrap(),
        new,
        "positions/acct.csv must hold the NEW import"
    );
    let baks: Vec<PathBuf> = read_dir_files(&data.join("positions"))
        .into_iter()
        .filter(|p| p.to_string_lossy().to_lowercase().ends_with(".bak"))
        .collect();
    assert!(
        baks.iter()
            .any(|b| std::fs::read_to_string(b).is_ok_and(|c| c == old)),
        "old contents must survive in a .bak: {baks:?}"
    );
}
