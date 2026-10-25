//! Planning and applying file changes.
//!
//! `plan_writes` computes the new contents of every file the wizard owns,
//! editing the existing text in place (`toml_edit` for TOML, entry by entry for
//! the dotenv file) so every byte outside the touched keys survives. `apply`
//! then writes the result atomically.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use toml_edit::{DocumentMut, Item, Table, TableLike, Value};

use super::answers::{ModelDir, SetupAnswers};
use super::existing::{item_at, scan_env, ExistingSetup};
use super::portfolio::PortfolioAnswer;
use super::{build_config_toml, WizardConfig};
use crate::config::{
    AlertsConfig, ENV_DISCORD_WEBHOOK_URL, ENV_FINNHUB_API_KEY, ENV_FRED_API_KEY,
    ENV_TELEGRAM_BOT_TOKEN, ENV_TELEGRAM_CHAT_ID,
};

/// Permission bits for files that hold secrets or the user's holdings.
const PRIVATE_MODE: u32 = 0o600;
/// Permission bits for new non-secret files.
const DEFAULT_MODE: u32 = 0o644;

/// What applying a [`FileChange`] does to the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    New,
    Changed,
    Unchanged,
    /// An old file that is renamed to `<name>.bak`.
    Removed,
}

/// How a file's diff is shown without leaking its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffStyle {
    /// Line diff with secret-looking values masked.
    Toml,
    /// Key names only: `~ KEY (changed)`.
    Env,
    /// Sizes only (the user's holdings).
    Opaque,
}

/// A planned file write.
#[derive(Clone)]
pub struct FileChange {
    pub path: PathBuf,
    pub kind: ChangeKind,
    pub contents: String,
    /// Unix permission bits to apply (`Some(0o600)` for private files).
    pub mode: Option<u32>,
    /// Current contents, if the file exists.
    previous: Option<String>,
    style: DiffStyle,
    /// The existing file is not valid TOML and is being replaced.
    replaces_unparseable: bool,
}

// Hand-written so a stray `{:?}` can never print secrets or holdings.
impl fmt::Debug for FileChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileChange")
            .field("path", &self.path)
            .field("kind", &self.kind)
            .field("mode", &self.mode)
            .field("bytes", &self.contents.len())
            .finish()
    }
}

impl FileChange {
    fn new(
        path: PathBuf,
        previous: Option<String>,
        contents: String,
        mode: Option<u32>,
        style: DiffStyle,
    ) -> Self {
        let kind = match previous.as_deref() {
            None => ChangeKind::New,
            Some(old) if old == contents => ChangeKind::Unchanged,
            Some(_) => ChangeKind::Changed,
        };
        Self {
            path,
            kind,
            contents,
            mode,
            previous,
            style,
            replaces_unparseable: false,
        }
    }

    fn retire(path: PathBuf) -> Self {
        Self {
            path,
            kind: ChangeKind::Removed,
            contents: String::new(),
            mode: None,
            previous: None,
            style: DiffStyle::Opaque,
            replaces_unparseable: false,
        }
    }

    /// Diff old -> new that never shows a secret: key names only for the
    /// dotenv file, masked values for secret-looking config keys, sizes only
    /// for imported CSVs. Empty if Unchanged.
    pub fn masked_diff(&self) -> String {
        let old = self.previous.as_deref().unwrap_or("");
        let name = self.path.display();
        match self.kind {
            ChangeKind::Unchanged => return String::new(),
            ChangeKind::Removed => {
                return format!(
                    "- {name} (renamed to {})\n",
                    backup_path(&self.path).display()
                )
            }
            _ => {}
        }
        if self.replaces_unparseable {
            return format!(
                "~ {name} is not valid TOML: it will be replaced and backed up to {}\n",
                backup_path(&self.path).display()
            );
        }
        match self.style {
            DiffStyle::Env => env_key_diff(self.previous.as_deref(), &self.contents),
            DiffStyle::Opaque => match self.kind {
                ChangeKind::New => {
                    format!("+ {name} (new, {} lines)\n", self.contents.lines().count())
                }
                _ => format!(
                    "~ {name} (changed, {} -> {} lines)\n",
                    old.lines().count(),
                    self.contents.lines().count()
                ),
            },
            DiffStyle::Toml => {
                let from = if self.kind == ChangeKind::New {
                    "/dev/null".to_string()
                } else {
                    name.to_string()
                };
                let body = unified_diff(old, &self.contents, 2, &mask_toml_line);
                format!("--- {from}\n+++ {name}\n{body}")
            }
        }
    }
}

// ─── Planning ────────────────────────────────────────────────────────────────

/// Compute the files to write. Reads existing files, never writes.
pub fn plan_writes(existing: &ExistingSetup, answers: &SetupAnswers) -> Vec<FileChange> {
    let mut out = Vec::new();

    // A broken config is only replaced when a config section was answered.
    if answers.touches_config() || existing.doc().is_some() {
        let mut doc = existing.doc().cloned().unwrap_or_else(fresh_config_doc);
        apply_config(&mut doc, answers);
        let mut change = FileChange::new(
            existing.config_path().to_path_buf(),
            existing.config_text().map(str::to_string),
            doc.to_string(),
            None,
            DiffStyle::Toml,
        );
        change.replaces_unparseable = existing.config_text().is_some() && existing.doc().is_none();
        out.push(change);
    }

    let secrets = env_updates(answers);
    if existing.env_text().is_some() || !secrets.is_empty() {
        let mut text = existing
            .env_text()
            .map_or_else(|| ENV_HEADER.to_string(), str::to_string);
        for (key, value) in &secrets {
            if existing.env_value(key) != Some(value.as_str()) {
                set_env_line(&mut text, key, value);
            }
        }
        out.push(FileChange::new(
            existing.env_path().to_path_buf(),
            existing.env_text().map(str::to_string),
            text,
            Some(PRIVATE_MODE),
            DiffStyle::Env,
        ));
    }

    if let Some(p) = &answers.portfolio {
        plan_portfolio(p, &mut out);
    }
    out
}

fn plan_portfolio(p: &PortfolioAnswer, out: &mut Vec<FileChange>) {
    for file in &p.files {
        let path = p.dir.join(&file.rel);
        let previous = std::fs::read_to_string(&path).ok();
        out.push(FileChange::new(
            path,
            previous,
            file.contents.clone(),
            Some(PRIVATE_MODE),
            DiffStyle::Opaque,
        ));
    }
    out.extend(p.retire.iter().cloned().map(FileChange::retire));
    let path = p.dir.join("plan.toml");
    let previous = std::fs::read_to_string(&path).ok();
    let contents = plan_toml_contents(previous.as_deref(), p);
    out.push(FileChange::new(
        path,
        previous,
        contents,
        Some(PRIVATE_MODE),
        DiffStyle::Toml,
    ));
}

const PLAN_TOML_HEADER: &str = "\
# Run settings for `ghost-alpha plan --data-dir <this folder>`.
# Written by `ghost-alpha setup`. Command-line flags override these values.
# Run `ghost-alpha init-data <dir>` to see every available option.

";

fn plan_toml_contents(previous: Option<&str>, p: &PortfolioAnswer) -> String {
    let (mut doc, header) = match previous.and_then(|t| t.parse::<DocumentMut>().ok()) {
        Some(doc) => (doc, ""),
        None => (DocumentMut::new(), PLAN_TOML_HEADER),
    };
    upsert_str(&mut doc, "", "as_of", &p.as_of.to_string(), None);
    if let Some(symbol) = &p.symbol {
        upsert_str(&mut doc, "", "symbol", symbol, None);
    }
    format!("{header}{doc}")
}

fn fresh_config_doc() -> DocumentMut {
    let text = build_config_toml(&WizardConfig {
        has_telegram: false,
        has_discord: false,
        db_path: "ghost_alpha.db".to_string(),
        model_path: None,
    });
    text.parse().unwrap_or_default()
}

fn apply_config(doc: &mut DocumentMut, a: &SetupAnswers) {
    let defaults = AlertsConfig::default();
    if let Some(provider) = &a.provider {
        upsert_str(doc, "plugins", "active_provider", provider, Some("finnhub"));
    }
    if let Some(ua) = &a.edgar_user_agent {
        upsert_str(doc, "edgar", "user_agent", ua, None);
    }
    if let Some(p) = &a.profile {
        upsert_str(doc, "profile", "state", &p.state, None);
        upsert_str(doc, "profile", "filing", &p.filing, None);
        upsert_f64(doc, "profile", "other_income", p.other_income, None);
    }
    if let Some(al) = &a.alerts {
        upsert_f64(
            doc,
            "alerts",
            "critical_score_threshold",
            al.critical,
            Some(defaults.critical_score_threshold),
        );
        upsert_f64(
            doc,
            "alerts",
            "warning_score_threshold",
            al.warning,
            Some(defaults.warning_score_threshold),
        );
        upsert_f64(
            doc,
            "alerts",
            "info_score_threshold",
            al.info,
            Some(defaults.info_score_threshold),
        );
    }
    if let Some(s) = &a.storage {
        upsert_str(
            doc,
            "storage",
            "db_path",
            &s.db_path,
            Some("ghost_alpha.db"),
        );
        match &s.model_dir {
            ModelDir::Keep => {}
            ModelDir::Clear => remove_key(doc, "sentiment", "model_path"),
            ModelDir::Set(dir) => upsert_str(doc, "sentiment", "model_path", dir, None),
        }
    }
}

// ─── TOML editing ────────────────────────────────────────────────────────────

/// The table called `name` (the document root for `""`), created if missing.
fn table_mut<'d>(doc: &'d mut DocumentMut, name: &str) -> Option<&'d mut dyn TableLike> {
    let root = doc.as_table_mut();
    if name.is_empty() {
        return Some(root);
    }
    if !root.contains_key(name) {
        root.insert(name, Item::Table(Table::new()));
    }
    let item = root.get_mut(name)?;
    if item.as_table_like().is_none() {
        *item = Item::Table(Table::new());
    }
    item.as_table_like_mut()
}

/// Set `table.key`, keeping the old value's surrounding whitespace and trailing
/// comment, and the key's own formatting.
fn set_value(doc: &mut DocumentMut, table: &str, key: &str, mut new: Value) {
    let Some(t) = table_mut(doc, table) else {
        return;
    };
    match t.get_mut(key) {
        Some(item) => {
            if let Some(old) = item.as_value() {
                *new.decor_mut() = old.decor().clone();
            }
            *item = Item::Value(new);
        }
        None => {
            t.insert(key, Item::Value(new));
        }
    }
}

/// Remove `table.key`, and the table itself if that leaves it empty.
fn remove_key(doc: &mut DocumentMut, table: &str, key: &str) {
    let root = doc.as_table_mut();
    let now_empty = match root.get_mut(table).and_then(Item::as_table_like_mut) {
        Some(t) => {
            t.remove(key);
            t.is_empty()
        }
        None => return,
    };
    if now_empty {
        root.remove(table);
    }
}

/// Write a string unless the effective value (`default` when absent) already equals it.
fn upsert_str(doc: &mut DocumentMut, table: &str, key: &str, new: &str, default: Option<&str>) {
    let path: Vec<&str> = [table, key].into_iter().filter(|s| !s.is_empty()).collect();
    let same = match item_at(doc, &path) {
        Some(item) => item.as_str() == Some(new),
        None => default == Some(new),
    };
    if !same {
        set_value(doc, table, key, Value::from(new));
    }
}

/// Write a number unless the effective value (`default` when absent) already equals it.
fn upsert_f64(doc: &mut DocumentMut, table: &str, key: &str, new: f64, default: Option<f64>) {
    let same = match item_at(doc, &[table, key]) {
        Some(item) => super::existing::as_number(item) == Some(new),
        None => default == Some(new),
    };
    if !same {
        set_value(doc, table, key, Value::from(new));
    }
}

/// Config key names whose values must not be shown on screen.
const SECRET_KEY_PARTS: [&str; 5] = ["api_key", "token", "secret", "webhook", "password"];

/// Hide the value of a secret-looking `key = value` config line (also in comments).
fn mask_toml_line(line: &str) -> String {
    let body = line.trim_start().trim_start_matches('#').trim_start();
    let key = body.split_once('=').map(|(k, _)| k.trim().to_lowercase());
    match (key, line.split_once('=')) {
        (Some(key), Some((before, _))) if SECRET_KEY_PARTS.iter().any(|p| key.contains(p)) => {
            format!("{before}= ****")
        }
        _ => line.to_string(),
    }
}

// ─── Dotenv editing ──────────────────────────────────────────────────────────

const ENV_HEADER: &str = "\
# Ghost Alpha secrets
# Generated by: ghost-alpha setup
# DO NOT commit this file. It is listed in .gitignore.

";

/// `(variable, value)` pairs for every dotenv value the user supplied.
fn env_updates(a: &SetupAnswers) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if let Some(k) = &a.finnhub_key {
        out.push((ENV_FINNHUB_API_KEY, k.expose().to_string()));
    }
    if let Some(k) = &a.fred_key {
        out.push((ENV_FRED_API_KEY, k.expose().to_string()));
    }
    if let Some(al) = &a.alerts {
        if let Some(t) = &al.telegram_token {
            out.push((ENV_TELEGRAM_BOT_TOKEN, t.expose().to_string()));
        }
        if let Some(chat) = &al.telegram_chat_id {
            out.push((ENV_TELEGRAM_CHAT_ID, chat.clone()));
        }
        if let Some(w) = &al.discord_webhook {
            out.push((ENV_DISCORD_WEBHOOK_URL, w.expose().to_string()));
        }
    }
    out
}

/// A dotenv-safe rendering of `value`: bare when it only has harmless
/// characters, else single-quoted (literal in `dotenvy`), else double-quoted.
fn env_quote(value: &str) -> String {
    let bare = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.:/@+,%=".contains(c));
    if bare {
        value.to_string()
    } else if !value.contains('\'') {
        format!("'{value}'")
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// Set `key` in dotenv `text`: the first existing entry for `key` is rewritten
/// in place (keeping `export `, a trailing comment and the line ending),
/// otherwise a line is appended using the file's line-ending style.
fn set_env_line(text: &mut String, key: &str, value: &str) {
    let rendered = env_quote(value);
    if let Some(e) = scan_env(text).into_iter().find(|e| e.key == key) {
        let raw = &text[e.start..e.end];
        let eol = if raw.ends_with("\r\n") {
            "\r\n"
        } else if raw.ends_with('\n') {
            "\n"
        } else {
            ""
        };
        let export = if e.export { "export " } else { "" };
        let line = format!("{export}{key}={rendered}{}{eol}", e.comment);
        text.replace_range(e.start..e.end, &line);
        return;
    }
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    if !text.is_empty() && !text.ends_with('\n') {
        text.push_str(eol);
    }
    text.push_str(&format!("{key}={rendered}{eol}"));
}

/// Key names only: `+ KEY (added)`, `~ KEY (changed)`, `- KEY (removed)`.
/// Values, comments and unrelated lines never appear.
fn env_key_diff(old: Option<&str>, new: &str) -> String {
    let first_values = |text: &str| -> Vec<(String, String)> {
        let mut seen = BTreeMap::new();
        let mut order = Vec::new();
        for e in scan_env(text) {
            if !seen.contains_key(&e.key) {
                seen.insert(e.key.clone(), e.value.clone());
                order.push((e.key, e.value));
            }
        }
        order
    };
    let old_entries = first_values(old.unwrap_or(""));
    let new_entries = first_values(new);
    let old_map: BTreeMap<&str, &str> = old_entries
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let mut out = String::new();
    for (key, value) in &new_entries {
        match old_map.get(key.as_str()) {
            None => out.push_str(&format!("+ {key} (added)\n")),
            Some(prev) if prev != value => out.push_str(&format!("~ {key} (changed)\n")),
            Some(_) => {}
        }
    }
    for (key, _) in &old_entries {
        if !new_entries.iter().any(|(k, _)| k == key) {
            out.push_str(&format!("- {key} (removed)\n"));
        }
    }
    if out.is_empty() {
        out.push_str("~ (comments or formatting changed; no keys differ)\n");
    }
    out
}

// ─── Line diff ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Op<'a> {
    Keep(&'a str),
    Del(&'a str),
    Add(&'a str),
}

/// Above this many cells the middle section is shown as delete-all / add-all
/// instead of computing an exact LCS.
const MAX_LCS_CELLS: usize = 4_000_000;

fn diff_ops<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<Op<'a>> {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let (mid_old, mid_new) = (
        &old[prefix..old.len() - suffix],
        &new[prefix..new.len() - suffix],
    );

    let mut ops: Vec<Op<'a>> = old[..prefix].iter().map(|l| Op::Keep(l)).collect();
    if mid_old.len().saturating_mul(mid_new.len()) > MAX_LCS_CELLS {
        ops.extend(mid_old.iter().map(|l| Op::Del(l)));
        ops.extend(mid_new.iter().map(|l| Op::Add(l)));
    } else {
        // lcs[i][j] = LCS length of mid_old[i..] and mid_new[j..]
        let (n, m) = (mid_old.len(), mid_new.len());
        let mut lcs = vec![0u32; (n + 1) * (m + 1)];
        let at = |i: usize, j: usize| i * (m + 1) + j;
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lcs[at(i, j)] = if mid_old[i] == mid_new[j] {
                    lcs[at(i + 1, j + 1)] + 1
                } else {
                    lcs[at(i + 1, j)].max(lcs[at(i, j + 1)])
                };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < n && j < m {
            if mid_old[i] == mid_new[j] {
                ops.push(Op::Keep(mid_old[i]));
                i += 1;
                j += 1;
            } else if lcs[at(i + 1, j)] >= lcs[at(i, j + 1)] {
                ops.push(Op::Del(mid_old[i]));
                i += 1;
            } else {
                ops.push(Op::Add(mid_new[j]));
                j += 1;
            }
        }
        ops.extend(mid_old[i..].iter().map(|l| Op::Del(l)));
        ops.extend(mid_new[j..].iter().map(|l| Op::Add(l)));
    }
    ops.extend(old[old.len() - suffix..].iter().map(|l| Op::Keep(l)));
    ops
}

/// Hunks of `old` -> `new` with `context` unchanged lines around each change.
/// `render` is applied to every line shown (used to mask secrets).
fn unified_diff(old: &str, new: &str, context: usize, render: &dyn Fn(&str) -> String) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let ops = diff_ops(&old_lines, &new_lines);

    // Line counts before each op, for hunk headers.
    let mut old_before = Vec::with_capacity(ops.len());
    let mut new_before = Vec::with_capacity(ops.len());
    let (mut o, mut n) = (0usize, 0usize);
    for op in &ops {
        old_before.push(o);
        new_before.push(n);
        match op {
            Op::Keep(_) => {
                o += 1;
                n += 1;
            }
            Op::Del(_) => o += 1,
            Op::Add(_) => n += 1,
        }
    }

    // Merge the windows around each change into hunks.
    let mut hunks: Vec<(usize, usize)> = Vec::new();
    for (idx, op) in ops.iter().enumerate() {
        if matches!(op, Op::Keep(_)) {
            continue;
        }
        let start = idx.saturating_sub(context);
        let end = (idx + context + 1).min(ops.len());
        match hunks.last_mut() {
            Some(last) if start <= last.1 => last.1 = end,
            _ => hunks.push((start, end)),
        }
    }

    let mut out = String::new();
    for (start, end) in hunks {
        let slice = &ops[start..end];
        let old_len = slice.iter().filter(|op| !matches!(op, Op::Add(_))).count();
        let new_len = slice.iter().filter(|op| !matches!(op, Op::Del(_))).count();
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_before[start] + 1,
            old_len,
            new_before[start] + 1,
            new_len
        ));
        for op in slice {
            let (sign, line) = match op {
                Op::Keep(l) => (' ', l),
                Op::Del(l) => ('-', l),
                Op::Add(l) => ('+', l),
            };
            out.push(sign);
            out.push_str(&render(line));
            out.push('\n');
        }
    }
    out
}

// ─── Applying ────────────────────────────────────────────────────────────────

/// `<path>.bak`
fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bak");
    path.with_file_name(name)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn mode_of(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .ok()
        .map(|m| m.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn mode_of(_path: &Path) -> Option<u32> {
    None
}

/// Copy `src` to `bak` as a new private (0600) regular file. A pre-existing
/// `bak` (including a symlink, which is never followed) is removed first, and
/// the new file is created exclusively.
fn write_backup(src: &Path, bak: &Path) -> Result<()> {
    let data = std::fs::read(src).with_context(|| format!("reading {}", src.display()))?;
    match std::fs::symlink_metadata(bak) {
        Ok(_) => std::fs::remove_file(bak)
            .with_context(|| format!("replacing the old backup {}", bak.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("inspecting {}", bak.display())),
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(PRIVATE_MODE);
    }
    let mut file = options
        .open(bak)
        .with_context(|| format!("creating backup {}", bak.display()))?;
    file.write_all(&data)
        .with_context(|| format!("writing backup {}", bak.display()))?;
    Ok(())
}

fn write_one(change: &FileChange, backup: bool) -> Result<()> {
    let link = &change.path;
    if change.kind == ChangeKind::Removed {
        let bak = backup_path(link);
        return std::fs::rename(link, &bak)
            .with_context(|| format!("renaming {} to {}", link.display(), bak.display()));
    }

    // A symlinked file is written through, so the link stays a link.
    let path: PathBuf = match std::fs::symlink_metadata(link) {
        Ok(m) if m.file_type().is_symlink() => std::fs::canonicalize(link)
            .with_context(|| format!("following the link {}", link.display()))?,
        _ => link.clone(),
    };
    let dir = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating directory {}", dir.display()))?;

    if backup && change.kind == ChangeKind::Changed && path.exists() {
        write_backup(&path, &backup_path(link))?;
    }

    // Temp file in the same directory, then rename: readers see the old or the
    // new file, never a partial one.
    let mut tmp = tempfile::NamedTempFile::new_in(dir)
        .with_context(|| format!("creating a temporary file in {}", dir.display()))?;
    tmp.write_all(change.contents.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    tmp.as_file()
        .sync_all()
        .with_context(|| format!("flushing {}", path.display()))?;
    let mode = change
        .mode
        .or_else(|| mode_of(&path))
        .unwrap_or(DEFAULT_MODE);
    set_mode(tmp.path(), mode)
        .with_context(|| format!("setting permissions on {}", path.display()))?;
    tmp.persist(&path)
        .map_err(|e| e.error)
        .with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Write `New`/`Changed` entries atomically (backing up `Changed` ones to
/// `<path>.bak` when `backup`) and rename `Removed` ones to `<path>.bak`;
/// returns the paths touched.
pub fn apply(changes: &[FileChange], backup: bool) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();
    // Retire renames go first: on a case-insensitive filesystem a new file may
    // share its name with an old one that is about to be renamed away.
    let active = changes.iter().filter(|c| c.kind != ChangeKind::Unchanged);
    let (retired, others): (Vec<_>, Vec<_>) = active.partition(|c| c.kind == ChangeKind::Removed);
    for change in retired.into_iter().chain(others) {
        write_one(change, backup)?;
        written.push(change.path.clone());
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_quote_when_harmless_should_stay_bare_else_be_quoted() {
        assert_eq!(env_quote("abc-123:/x"), "abc-123:/x");
        assert_eq!(env_quote("a b"), "'a b'");
        assert_eq!(env_quote("a$b#c"), "'a$b#c'");
        assert_eq!(env_quote("it's"), "\"it's\"");
        assert_eq!(env_quote(""), "''");
    }

    #[test]
    fn set_env_line_when_key_missing_should_append_with_newline() {
        let mut text = "A=1".to_string();
        set_env_line(&mut text, "B", "2");
        assert_eq!(text, "A=1\nB=2\n");
    }

    #[test]
    fn set_env_line_when_key_present_should_replace_in_place_keeping_export_and_crlf() {
        let mut text = "# c\r\nexport A=old\r\nB=1\r\n".to_string();
        set_env_line(&mut text, "A", "new");
        assert_eq!(text, "# c\r\nexport A=new\r\nB=1\r\n");
    }

    #[test]
    fn set_env_line_when_crlf_file_should_append_with_crlf() {
        let mut text = "A=1\r\n".to_string();
        set_env_line(&mut text, "B", "2");
        assert_eq!(text, "A=1\r\nB=2\r\n");
    }

    #[test]
    fn set_env_line_when_value_is_multiline_quoted_should_replace_whole_entry() {
        let mut text = "A=\"x\nB=1\n\"\nC=3\n".to_string();
        set_env_line(&mut text, "A", "new");
        assert_eq!(text, "A=new\nC=3\n");
    }

    #[test]
    fn env_key_diff_should_list_names_only() {
        let d = env_key_diff(Some("# c\nA=1\nB=2\n"), "A=9\nC=3\n");
        assert_eq!(d, "~ A (changed)\n+ C (added)\n- B (removed)\n");
        let d = env_key_diff(None, "A=1\n");
        assert_eq!(d, "+ A (added)\n");
    }

    #[test]
    fn mask_toml_line_should_hide_secret_looking_values_only() {
        assert_eq!(mask_toml_line("api_key = \"x\""), "api_key = ****");
        assert_eq!(mask_toml_line("Webhook_URL = \"u\""), "Webhook_URL = ****");
        assert_eq!(mask_toml_line("# bot_token = \"t\""), "# bot_token = ****");
        assert_eq!(mask_toml_line("user_agent = \"u\""), "user_agent = \"u\"");
        assert_eq!(mask_toml_line("[finnhub]"), "[finnhub]");
    }

    #[test]
    fn unified_diff_when_one_line_changes_should_show_hunk_with_context() {
        let d = unified_diff("a\nb\nc\nd\ne\n", "a\nb\nX\nd\ne\n", 1, &|l| l.to_string());
        assert_eq!(d, "@@ -2,3 +2,3 @@\n b\n-c\n+X\n d\n");
    }

    #[test]
    fn unified_diff_when_identical_should_be_empty() {
        assert_eq!(unified_diff("a\n", "a\n", 2, &|l| l.to_string()), "");
    }

    #[test]
    fn unified_diff_when_new_file_should_add_every_line() {
        let d = unified_diff("", "a\nb\n", 2, &|l| l.to_string());
        assert_eq!(d, "@@ -1,0 +1,2 @@\n+a\n+b\n");
    }

    #[test]
    fn set_value_when_replacing_should_keep_trailing_comment_and_key_spacing() {
        let mut doc: DocumentMut = "[a]\nk  = 1.0   # note\n".parse().unwrap();
        upsert_f64(&mut doc, "a", "k", 2.0, None);
        assert_eq!(doc.to_string(), "[a]\nk  = 2.0   # note\n");
    }

    #[test]
    fn upsert_str_when_value_equals_default_and_absent_should_not_write() {
        let mut doc = DocumentMut::new();
        upsert_str(
            &mut doc,
            "plugins",
            "active_provider",
            "finnhub",
            Some("finnhub"),
        );
        assert_eq!(doc.to_string(), "");
    }

    #[test]
    fn remove_key_should_drop_an_emptied_table_but_keep_a_shared_one() {
        let mut doc: DocumentMut = "[s]\nm = 1\n[t]\nm = 1\nn = 2\n".parse().unwrap();
        remove_key(&mut doc, "s", "m");
        remove_key(&mut doc, "t", "m");
        assert_eq!(doc.to_string(), "[t]\nn = 2\n");
    }

    #[test]
    fn debug_of_file_change_should_not_print_contents() {
        let c = FileChange::new(
            PathBuf::from(".env"),
            None,
            "K=SECRET-VALUE\n".into(),
            Some(PRIVATE_MODE),
            DiffStyle::Env,
        );
        assert!(!format!("{c:?}").contains("SECRET-VALUE"));
    }
}
