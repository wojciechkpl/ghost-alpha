//! Portfolio step: validate the user's positions / vesting CSVs with the same
//! loaders `plan` uses, then stage verbatim copies for the data folder.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;

use super::prompter::Prompter;
use crate::portfolio::import::{read_positions_auto, FidelityPositionRow};
use crate::portfolio::symbol_map::SymbolMap;
use crate::portfolio::vesting::DistributionSchedule;
use crate::projection::monte_carlo::AssetClass;

/// Attempts allowed for a path or CSV that keeps failing before the step gives up.
const MAX_BAD_ATTEMPTS: usize = 10;

/// A file to be copied into the data folder, relative to it.
#[derive(Clone)]
pub(super) struct ImportFile {
    pub(super) rel: PathBuf,
    pub(super) contents: String,
}

// Never print the contents: they are the user's holdings.
impl fmt::Debug for ImportFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImportFile")
            .field("rel", &self.rel)
            .field("bytes", &self.contents.len())
            .finish()
    }
}

/// Portfolio answers: the data folder, the staged copies and `plan.toml` values.
#[derive(Debug, Clone)]
pub(super) struct PortfolioAnswer {
    pub(super) dir: PathBuf,
    pub(super) files: Vec<ImportFile>,
    /// Old `positions/*.csv` files the user chose to replace; each is renamed
    /// to `<name>.bak` unless a new file takes over its name.
    pub(super) retire: Vec<PathBuf>,
    /// `None` = let `plan` auto-detect the concentrated symbol.
    pub(super) symbol: Option<String>,
    pub(super) as_of: NaiveDate,
}

/// Parsed-positions summary shown before continuing the portfolio step.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportSummary {
    pub rows: usize,
    pub accounts: usize,
    pub symbols: usize,
    pub total_value: f64,
}

fn summarize_rows(rows: &[FidelityPositionRow]) -> ImportSummary {
    ImportSummary {
        rows: rows.len(),
        accounts: rows
            .iter()
            .map(|r| r.account_number.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        symbols: rows
            .iter()
            .map(|r| r.symbol.as_str())
            .collect::<BTreeSet<_>>()
            .len(),
        total_value: rows.iter().map(|r| r.current_value).sum(),
    }
}

/// Parse `path` with `read_positions_auto` and summarise it.
pub fn summarize_positions(path: &Path) -> Result<ImportSummary> {
    Ok(summarize_rows(&read_positions_auto(path)?))
}

/// Turn text typed or dragged into a terminal into a path: surrounding quotes
/// and whitespace are dropped, `\ ` is unescaped (unix) and a leading `~/` is
/// expanded.
pub(super) fn clean_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    let unquoted = ["'", "\""]
        .iter()
        .find_map(|q| trimmed.strip_prefix(q)?.strip_suffix(q))
        .unwrap_or(trimmed);
    #[cfg(unix)]
    let unquoted = unquoted.replace("\\ ", " ");
    #[cfg(unix)]
    let unquoted = unquoted.as_str();
    if let (Some(rest), Some(home)) = (unquoted.strip_prefix("~/"), std::env::var_os("HOME")) {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(unquoted)
}

/// Ask for a path, clean it, and re-ask while it is empty (or, with `must_exist`, missing).
fn ask_path(
    prompter: &mut dyn Prompter,
    key: &str,
    label: &str,
    default: Option<&Path>,
    must_exist: bool,
) -> Result<PathBuf> {
    for _ in 0..MAX_BAD_ATTEMPTS {
        let raw = prompter.path(key, label, default)?;
        let path = clean_path(&raw.to_string_lossy());
        if path.as_os_str().is_empty() {
            prompter.note("  A path is required.");
        } else if must_exist && !path.exists() {
            prompter.note(&format!("  {} does not exist.", path.display()));
        } else {
            return Ok(path);
        }
    }
    bail!("no usable path given for `{key}`")
}

/// The concentrated-position suggestion, mirroring `plan`'s auto-detection:
/// the vesting symbol with the most shares, else the largest non-cash position.
fn suggest_symbol(
    rows: &[FidelityPositionRow],
    vesting: Option<&DistributionSchedule>,
) -> Option<String> {
    if let Some(schedule) = vesting {
        let best = schedule
            .symbols()
            .into_iter()
            .map(|s| (schedule.total_gross_quantity(&s), s))
            .max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, symbol)) = best {
            return Some(symbol);
        }
    }
    let map = SymbolMap::builtin();
    let mut value_by_symbol: BTreeMap<&str, f64> = BTreeMap::new();
    for r in rows {
        let cash = r.symbol.ends_with("**") || map.class_of(&r.symbol) == Some(AssetClass::CashMmf);
        if !cash && !r.symbol.is_empty() {
            *value_by_symbol.entry(&r.symbol).or_default() += r.current_value;
        }
    }
    value_by_symbol
        .into_iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(symbol, _)| symbol.to_string())
}

/// The copy's file name: the source stem with a `.csv` extension, so `plan`
/// (which reads only `positions/*.csv`) always sees it.
fn csv_name(src: &Path) -> String {
    let stem = src
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("positions");
    format!("{stem}.csv")
}

/// `name`, or `name_2.csv`, `name_3.csv`, ... if `taken` already holds it.
fn unique_name(name: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(name) {
        return name.to_string();
    }
    let stem = Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("positions");
    (2..)
        .map(|n| format!("{stem}_{n}.csv"))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| name.to_string())
}

/// `*.csv` files directly inside `dir`, sorted.
fn csv_files_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = read
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
        })
        .collect();
    files.sort();
    files
}

/// If any of `paths` is inside a git work tree and not ignored, a warning
/// that the user's data could be committed. Best effort: `None` without `git`,
/// or when anything about the lookup fails.
pub(super) fn git_ignore_warning(paths: &[PathBuf]) -> Option<String> {
    use std::process::{Command, Stdio};

    let cwd = std::env::current_dir().ok()?;
    let absolute = |p: &Path| {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        }
    };
    let first = absolute(paths.first()?);
    let base = first.ancestors().find(|p| p.is_dir())?.to_path_buf();
    let inside = Command::new("git")
        .arg("-C")
        .arg(&base)
        .args(["rev-parse", "--is-inside-work-tree"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !inside.status.success() || String::from_utf8_lossy(&inside.stdout).trim() != "true" {
        return None;
    }
    // Exit status 0 = ignored, 1 = not ignored, anything else = lookup failed.
    let exposed: Vec<&PathBuf> = paths
        .iter()
        .filter(|p| {
            Command::new("git")
                .arg("-C")
                .arg(&base)
                .args(["check-ignore", "-q", "--"])
                .arg(absolute(p))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .ok()
                .and_then(|s| s.code())
                == Some(1)
        })
        .collect();
    if exposed.is_empty() {
        return None;
    }
    let shown: Vec<String> = exposed
        .iter()
        .take(3)
        .map(|p| p.display().to_string())
        .collect();
    let more = exposed.len().saturating_sub(shown.len());
    Some(format!(
        "warning: {}{} would sit inside a git work tree and is not ignored, so your portfolio \
         data could be committed by accident. Add the data folder to .gitignore first.",
        shown.join(", "),
        if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        }
    ))
}

/// Fail if a file the step will write exists but cannot be read as UTF-8 text:
/// it would otherwise be overwritten without a backup.
fn ensure_readable_if_present(paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        match std::fs::read(path) {
            Ok(bytes) if std::str::from_utf8(&bytes).is_err() => {
                bail!(
                    "{} exists but is not valid UTF-8 text; refusing to overwrite it",
                    path.display()
                )
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => bail!(
                "cannot read {}: {e}; refusing to overwrite it",
                path.display()
            ),
        }
    }
    Ok(())
}

/// Ask for the data folder, positions, vesting, symbol and snapshot date.
pub(super) fn run(prompter: &mut dyn Prompter) -> Result<PortfolioAnswer> {
    prompter.note("\nPortfolio data");
    let dir = ask_path(
        prompter,
        "portfolio.dir",
        "Data folder for positions and vesting",
        Some(Path::new("data")),
        false,
    )?;

    // A re-import: old positions files would be summed with the new ones by `plan`.
    let positions_dir = dir.join("positions");
    let old_files = csv_files_in(&positions_dir);
    let mut replace_old = false;
    if !old_files.is_empty() {
        replace_old = prompter.confirm(
            "portfolio.replace_existing",
            &format!(
                "{} already holds {} positions file(s). Replace them? (they are kept as .bak)",
                positions_dir.display(),
                old_files.len()
            ),
            true,
        )?;
        if !replace_old {
            prompter.note(
                "  Keeping the old files: `plan` will sum every positions/*.csv, so old and \
                 new holdings will be added together.",
            );
        }
    }

    let mut files: Vec<ImportFile> = Vec::new();
    let mut taken: BTreeSet<String> = BTreeSet::new();
    if !replace_old {
        taken.extend(
            old_files
                .iter()
                .filter_map(|p| p.file_name()?.to_str().map(str::to_string)),
        );
    }
    let mut all_rows: Vec<FidelityPositionRow> = Vec::new();
    let mut bad_files = 0;
    loop {
        let src = ask_path(
            prompter,
            "portfolio.positions",
            "Positions CSV (Fidelity export or simple symbol,quantity,price)",
            None,
            true,
        )?;
        let parsed = read_positions_auto(&src)
            .map_err(anyhow::Error::from)
            .and_then(|rows| {
                let text = std::fs::read_to_string(&src)
                    .with_context(|| format!("reading {}", src.display()))?;
                Ok((rows, text))
            });
        let (rows, contents) = match parsed {
            Ok(ok) => ok,
            Err(e) => {
                bad_files += 1;
                if bad_files >= MAX_BAD_ATTEMPTS {
                    bail!("giving up after {bad_files} unreadable positions files: {e}");
                }
                prompter.note(&format!("  cannot use that file: {e:#}"));
                continue;
            }
        };
        let s = summarize_rows(&rows);
        let name = unique_name(&csv_name(&src), &taken);
        prompter.note(&format!(
            "  {name}: {} rows, {} accounts, {} symbols, total value ${:.2}",
            s.rows, s.accounts, s.symbols, s.total_value
        ));
        taken.insert(name.clone());
        files.push(ImportFile {
            rel: Path::new("positions").join(name),
            contents,
        });
        all_rows.extend(rows);
        if !prompter.confirm("portfolio.add_more", "Add another positions file?", false)? {
            break;
        }
    }

    let check_vesting = |s: &str| -> Result<(), String> {
        if s.trim().is_empty() {
            return Ok(());
        }
        let path = clean_path(s);
        DistributionSchedule::from_csv_file(&path)
            .map(|_| ())
            .map_err(|e| format!("{}: {e}", path.display()))
    };
    let vesting_answer = prompter.input(
        "portfolio.vesting",
        "Vesting schedule CSV (symbol,vesting_date,quantity; Enter to skip)",
        None,
        &check_vesting,
    )?;
    let mut schedule = None;
    if vesting_answer.trim().is_empty() {
        if dir.join("vesting.csv").is_file() {
            prompter.note(&format!(
                "  {} already exists and was kept; `plan` will keep using it.",
                dir.join("vesting.csv").display()
            ));
        }
    } else {
        let path = clean_path(&vesting_answer);
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        schedule = Some(
            DistributionSchedule::from_csv_str(&contents)
                .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?,
        );
        prompter.note(&format!(
            "  vesting: {} events",
            schedule.as_ref().map_or(0, |s| s.len())
        ));
        files.push(ImportFile {
            rel: PathBuf::from("vesting.csv"),
            contents,
        });
    }

    let suggestion = suggest_symbol(&all_rows, schedule.as_ref());
    let check_symbol = |s: &str| -> Result<(), String> {
        if s.trim()
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        {
            Ok(())
        } else {
            Err("use letters, digits, '.', '-' or '_' (e.g. MSFT, BRK.B)".to_string())
        }
    };
    let symbol = prompter.input(
        "portfolio.symbol",
        "Concentrated symbol (Enter keeps the suggestion, - means auto-detect)",
        suggestion.as_deref(),
        &check_symbol,
    )?;
    let symbol = match symbol.trim() {
        "" => suggestion,
        "-" => None,
        s => Some(s.to_uppercase()),
    };

    let today = chrono::Local::now().date_naive().to_string();
    let check_date = |s: &str| -> Result<(), String> {
        NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
            .map(|_| ())
            .map_err(|_| "use YYYY-MM-DD".to_string())
    };
    let as_of = prompter.input(
        "portfolio.as_of",
        "Snapshot date of the positions (YYYY-MM-DD)",
        Some(&today),
        &check_date,
    )?;
    let as_of = NaiveDate::parse_from_str(as_of.trim(), "%Y-%m-%d")?;

    let mut planned: Vec<PathBuf> = files.iter().map(|f| dir.join(&f.rel)).collect();
    planned.push(dir.join("plan.toml"));
    ensure_readable_if_present(&planned)?;
    if let Some(warning) = git_ignore_warning(&planned) {
        prompter.note(&warning);
    }

    let new_names: BTreeSet<&Path> = planned.iter().map(PathBuf::as_path).collect();
    let retire = if replace_old {
        old_files
            .into_iter()
            .filter(|p| !new_names.contains(p.as_path()))
            .collect()
    } else {
        Vec::new()
    };

    Ok(PortfolioAnswer {
        dir,
        files,
        retire,
        symbol,
        as_of,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(symbol: &str, value: f64) -> FidelityPositionRow {
        FidelityPositionRow {
            account_number: "A".into(),
            account_name: "Individual".into(),
            symbol: symbol.into(),
            description: String::new(),
            quantity: 1.0,
            last_price: value,
            current_value: value,
            cost_basis_total: None,
            average_cost_basis: None,
            percent_of_account: None,
            position_type: "Cash".into(),
        }
    }

    #[test]
    fn suggest_symbol_when_cash_is_largest_should_skip_it() {
        let rows = [row("FCASH**", 9e6), row("XYZ", 100.0), row("ABC", 50.0)];
        assert_eq!(suggest_symbol(&rows, None).as_deref(), Some("XYZ"));
    }

    #[test]
    fn suggest_symbol_when_symbol_spans_accounts_should_sum_values() {
        let rows = [row("ABC", 80.0), row("XYZ", 100.0), row("ABC", 80.0)];
        assert_eq!(suggest_symbol(&rows, None).as_deref(), Some("ABC"));
    }

    #[test]
    fn unique_name_when_taken_should_add_numeric_suffix() {
        let taken: BTreeSet<String> = ["a.csv".to_string(), "a_2.csv".to_string()].into();
        assert_eq!(unique_name("a.csv", &taken), "a_3.csv");
        assert_eq!(unique_name("b.csv", &taken), "b.csv");
    }

    #[test]
    fn csv_name_should_force_a_csv_extension() {
        assert_eq!(csv_name(Path::new("/x/holdings.txt")), "holdings.csv");
        assert_eq!(csv_name(Path::new("/x/noext")), "noext.csv");
        assert_eq!(csv_name(Path::new("/x/A.CSV")), "A.csv");
    }

    #[test]
    fn debug_of_import_file_should_not_print_contents() {
        let f = ImportFile {
            rel: PathBuf::from("positions/a.csv"),
            contents: "SECRET-HOLDING,1,2".into(),
        };
        assert!(!format!("{f:?}").contains("SECRET-HOLDING"));
    }

    fn git(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[test]
    fn git_ignore_warning_should_flag_only_unignored_paths_inside_a_work_tree() {
        let tmp = tempfile::TempDir::new().unwrap();
        if !git(tmp.path(), &["init", "-q"]) {
            eprintln!("git is not available; skipping");
            return;
        }
        let paths = vec![
            tmp.path().join("data/positions/a.csv"),
            tmp.path().join("data/plan.toml"),
        ];
        let warning = git_ignore_warning(&paths).expect("unignored paths in a repo must warn");
        assert!(warning.contains("not ignored"), "{warning}");

        std::fs::write(tmp.path().join(".gitignore"), "data/\n").unwrap();
        assert_eq!(git_ignore_warning(&paths), None);

        // Partially ignored still warns.
        std::fs::write(tmp.path().join(".gitignore"), "positions/\n").unwrap();
        assert!(git_ignore_warning(&paths).is_some());
    }

    #[test]
    fn git_ignore_warning_when_outside_any_work_tree_should_be_silent() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert_eq!(
            git_ignore_warning(&[tmp.path().join("data/plan.toml")]),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn clean_path_when_quoted_or_escaped_should_normalise() {
        assert_eq!(clean_path("  'a b/c.csv' "), PathBuf::from("a b/c.csv"));
        assert_eq!(clean_path("a\\ b/c.csv"), PathBuf::from("a b/c.csv"));
        assert_eq!(clean_path("\"x.csv\""), PathBuf::from("x.csv"));
    }
}
