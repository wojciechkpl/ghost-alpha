//! Current on-disk state: the config file and the dotenv file.
//!
//! Loading never fails. A missing file is simply empty; a file that exists but
//! cannot be read (permissions, not UTF-8) is recorded in [`ExistingSetup::unreadable`],
//! which makes `run_setup` stop before anything is written. A readable but
//! unparseable config produces a warning and the wizard falls back to built-in
//! defaults (the broken file is then replaced, with a backup).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item};

/// Current on-disk state (config file + dotenv file), never fails to load.
pub struct ExistingSetup {
    config_path: PathBuf,
    env_path: PathBuf,
    /// Raw text of the config file, if it exists and is readable.
    config_text: Option<String>,
    /// Parsed config, if `config_text` is valid TOML.
    doc: Option<DocumentMut>,
    /// Raw text of the dotenv file, if it exists and is readable.
    env_text: Option<String>,
    /// Parsed `KEY=value` pairs (first occurrence wins, like `dotenvy`).
    env: BTreeMap<String, String>,
    warnings: Vec<String>,
    unreadable: Vec<(PathBuf, String)>,
}

impl ExistingSetup {
    pub fn load(config_path: &Path, env_path: &Path) -> ExistingSetup {
        let mut warnings = Vec::new();
        let mut unreadable = Vec::new();
        let config_text = read_optional(config_path, &mut unreadable);
        let doc = config_text
            .as_deref()
            .and_then(|text| match text.parse::<DocumentMut>() {
                Ok(doc) => Some(doc),
                Err(e) => {
                    // `message()` is the short reason only; it never quotes the file.
                    warnings.push(format!(
                        "{} is not valid TOML ({}); using built-in defaults. \
                         The file will be replaced (a .bak copy is kept).",
                        config_path.display(),
                        e.message()
                    ));
                    None
                }
            });
        let env_text = read_optional(env_path, &mut unreadable);
        let mut env = BTreeMap::new();
        for entry in scan_env(env_text.as_deref().unwrap_or("")) {
            env.entry(entry.key).or_insert(entry.value);
        }
        ExistingSetup {
            config_path: config_path.to_path_buf(),
            env_path: env_path.to_path_buf(),
            config_text,
            doc,
            env_text,
            env,
            warnings,
            unreadable,
        }
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn env_path(&self) -> &Path {
        &self.env_path
    }

    /// Value of `key` in the dotenv file, with quotes removed.
    pub fn env_value(&self, key: &str) -> Option<&str> {
        self.env.get(key).map(String::as_str)
    }

    /// Human-readable warnings (e.g. unparseable config file was ignored).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Files that exist but cannot be read as UTF-8 text, with the reason.
    pub fn unreadable(&self) -> &[(PathBuf, String)] {
        &self.unreadable
    }

    pub(super) fn config_text(&self) -> Option<&str> {
        self.config_text.as_deref()
    }

    pub(super) fn env_text(&self) -> Option<&str> {
        self.env_text.as_deref()
    }

    /// The parsed config document, if the file exists and is valid TOML.
    pub(super) fn doc(&self) -> Option<&DocumentMut> {
        self.doc.as_ref()
    }

    /// String at `path` (e.g. `["edgar", "user_agent"]`) in the existing config.
    pub(super) fn doc_str(&self, path: &[&str]) -> Option<&str> {
        self.doc().and_then(|d| item_at(d, path))?.as_str()
    }

    /// Number at `path` in the existing config (integers are accepted too).
    pub(super) fn doc_f64(&self, path: &[&str]) -> Option<f64> {
        self.doc()
            .and_then(|d| item_at(d, path))
            .and_then(as_number)
    }
}

/// Look `path` up in `doc`.
pub(super) fn item_at<'d>(doc: &'d DocumentMut, path: &[&str]) -> Option<&'d Item> {
    let mut item = doc.as_item();
    for key in path {
        item = item.get(key)?;
    }
    Some(item)
}

/// A TOML float or integer as `f64`.
pub(super) fn as_number(item: &Item) -> Option<f64> {
    item.as_float()
        .or_else(|| item.as_integer().map(|i| i as f64))
}

/// `None` for a missing file; an unreadable or non-UTF-8 one is recorded in `unreadable`.
fn read_optional(path: &Path, unreadable: &mut Vec<(PathBuf, String)>) -> Option<String> {
    match std::fs::read(path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => Some(text),
            Err(_) => {
                unreadable.push((path.to_path_buf(), "it is not valid UTF-8 text".to_string()));
                None
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            unreadable.push((path.to_path_buf(), e.to_string()));
            None
        }
    }
}

// ─── Dotenv scanning ─────────────────────────────────────────────────────────

/// One logical `KEY=value` entry of a dotenv file (a quoted value may span lines).
#[derive(Debug, Clone, PartialEq)]
pub(super) struct EnvEntry {
    pub(super) key: String,
    /// The value with quotes removed.
    pub(super) value: String,
    /// Byte range of the whole entry in the text, including its line ending.
    pub(super) start: usize,
    pub(super) end: usize,
    /// The line began with `export `.
    pub(super) export: bool,
    /// A trailing ` # comment` after the value (with its leading whitespace).
    pub(super) comment: String,
}

/// Position of the closing `quote` in `s`, skipping `\x` escapes in double quotes.
fn closing_quote(s: &str, quote: char) -> Option<usize> {
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        if c == '\\' && quote == '"' {
            chars.next();
        } else if c == quote {
            return Some(i);
        }
    }
    None
}

/// All entries of dotenv `text`, in order. Comments, blank lines and lines
/// without `=` are skipped; quoted values spanning several lines are one entry.
pub(super) fn scan_env(text: &str) -> Vec<EnvEntry> {
    let mut entries = Vec::new();
    let mut offset = 0;
    let mut lines = text.split_inclusive('\n');
    while let Some(first) = lines.next() {
        let start = offset;
        offset += first.len();
        let trimmed = first.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (export, body) = match trimmed.strip_prefix("export ") {
            Some(rest) => (true, rest.trim_start()),
            None => (false, trimmed),
        };
        let Some((key, rest)) = body.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        let rest = rest.trim_start();
        let quote = rest.chars().next().filter(|c| *c == '\'' || *c == '"');
        let (value, comment) = match quote {
            Some(q) => {
                let mut acc = rest[1..].to_string();
                let mut tail = String::new();
                // Look ahead on a copy: an unterminated quote must not swallow
                // the lines after it, so they are only consumed once it closes.
                let mut ahead = lines.clone();
                let mut ahead_offset = offset;
                loop {
                    if let Some(i) = closing_quote(&acc, q) {
                        tail = acc[i + 1..].to_string();
                        acc.truncate(i);
                        lines = ahead;
                        offset = ahead_offset;
                        break;
                    }
                    match ahead.next() {
                        Some(next) => {
                            ahead_offset += next.len();
                            acc.push('\n');
                            acc.push_str(next.trim_end_matches(['\r', '\n']));
                        }
                        None => {
                            // Never closed: the value is the rest of its own line.
                            acc = rest[1..].trim_end_matches(['\r', '\n']).to_string();
                            break;
                        }
                    }
                }
                let value = if q == '"' {
                    acc.replace("\\\"", "\"").replace("\\\\", "\\")
                } else {
                    acc
                };
                let comment = if tail.trim_start().starts_with('#') {
                    tail.trim_end().to_string()
                } else {
                    String::new()
                };
                (value, comment)
            }
            None => match rest.find(" #") {
                Some(i) => (
                    rest[..i].trim_end().to_string(),
                    rest[i..].trim_end().to_string(),
                ),
                None => (rest.trim_end().to_string(), String::new()),
            },
        };
        entries.push(EnvEntry {
            key: key.to_string(),
            value,
            start,
            end: offset,
            export,
            comment,
        });
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> EnvEntry {
        let mut v = scan_env(text);
        assert_eq!(v.len(), 1, "{v:?}");
        v.remove(0)
    }

    #[test]
    fn scan_env_when_plain_should_split_key_and_value() {
        let e = one("A=1\n");
        assert_eq!((e.key.as_str(), e.value.as_str()), ("A", "1"));
        let e = one("export A = 1");
        assert_eq!(
            (e.key.as_str(), e.value.as_str(), e.export),
            ("A", "1", true)
        );
    }

    #[test]
    fn scan_env_when_comment_blank_or_no_equals_should_skip() {
        assert!(scan_env("# A=1\n   \ngarbage\n=x\n").is_empty());
    }

    #[test]
    fn scan_env_when_quoted_should_strip_quotes() {
        assert_eq!(one("A='x y'").value, "x y");
        assert_eq!(one(r#"A="say \"hi\"""#).value, "say \"hi\"");
    }

    #[test]
    fn scan_env_when_inline_comment_should_keep_it_apart_from_the_value() {
        let e = one("A=1 # note\n");
        assert_eq!((e.value.as_str(), e.comment.as_str()), ("1", " # note"));
        let e = one("A='x' # note\n");
        assert_eq!((e.value.as_str(), e.comment.as_str()), ("x", " # note"));
    }

    #[test]
    fn scan_env_when_quote_never_closes_should_end_at_its_own_line() {
        let keys: Vec<String> = scan_env("A=\"abc\nB=1\nC=2\n")
            .into_iter()
            .map(|e| e.key)
            .collect();
        assert_eq!(keys, ["A", "B", "C"]);
    }

    #[test]
    fn scan_env_when_quoted_value_spans_lines_should_be_one_entry() {
        let text = "A=1\nM=\"l1\nB=2\n\"\nC=3\n";
        let keys: Vec<String> = scan_env(text).into_iter().map(|e| e.key).collect();
        assert_eq!(keys, ["A", "M", "C"]);
    }
}
