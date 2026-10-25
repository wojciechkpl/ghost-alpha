//! Interactive I/O behind a trait, so every wizard step is testable without a terminal.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

/// All interactive I/O. `key` is a stable id (`"<section>.<name>"`) so scripted
/// tests do not depend on label wording.
pub trait Prompter {
    /// Free text. Re-asks while `validate` rejects the answer. `default` is
    /// used when the user just presses Enter.
    fn input(
        &mut self,
        key: &str,
        label: &str,
        default: Option<&str>,
        validate: &dyn Fn(&str) -> Result<(), String>,
    ) -> Result<String>;
    /// Masked input. `Ok(None)` means "keep the existing secret".
    fn secret(&mut self, key: &str, label: &str, has_existing: bool) -> Result<Option<String>>;
    /// Pick one of `items`; returns its index.
    fn select(&mut self, key: &str, label: &str, items: &[&str], default: usize) -> Result<usize>;
    /// Yes / no.
    fn confirm(&mut self, key: &str, label: &str, default: bool) -> Result<bool>;
    /// A filesystem path, exactly as typed (quotes, `~` and emptiness are the
    /// caller's business, so every prompter behaves the same).
    fn path(&mut self, key: &str, label: &str, default: Option<&Path>) -> Result<PathBuf>;
    /// Informational output (e.g. the portfolio import summary).
    fn note(&mut self, text: &str);
}

// ─── Scripted prompter (tests) ────────────────────────────────────────────────

/// One recorded prompt.
#[derive(Debug, Clone, PartialEq)]
pub struct Asked {
    pub key: String,
    /// Default shown to the user (select: the default item text; confirm: "y"/"n";
    /// path: the path as text). Always `None` for secrets.
    pub default: Option<String>,
    /// Only meaningful for `secret` prompts.
    pub has_existing: bool,
}

/// Non-interactive [`Prompter`] driven by per-key answer queues.
///
/// Each ask records an [`Asked`], then takes the next queued answer for its key.
/// With nothing queued the user "presses Enter": the default is used (secrets
/// keep the existing value). Like `dialoguer`, `input` re-asks while
/// the answer is rejected (`path` returns the answer as typed); if the rejected
/// answer was the implicit default the prompter fails instead of looping forever.
#[derive(Default)]
pub struct ScriptedPrompter {
    answers: HashMap<String, VecDeque<String>>,
    asked: Vec<Asked>,
    notes: Vec<String>,
}

impl ScriptedPrompter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue `answers` (as text) for prompts with this `key`.
    pub fn with<I, S>(mut self, key: &str, answers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.answers
            .entry(key.to_string())
            .or_default()
            .extend(answers.into_iter().map(Into::into));
        self
    }

    /// Every ask in order, including asks rejected by validation.
    pub fn asked(&self) -> &[Asked] {
        &self.asked
    }

    /// Everything passed to `note`.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    fn record(&mut self, key: &str, default: Option<String>, has_existing: bool) {
        self.asked.push(Asked {
            key: key.to_string(),
            default,
            has_existing,
        });
    }

    fn next_answer(&mut self, key: &str) -> Option<String> {
        self.answers.get_mut(key).and_then(VecDeque::pop_front)
    }
}

impl Prompter for ScriptedPrompter {
    fn input(
        &mut self,
        key: &str,
        _label: &str,
        default: Option<&str>,
        validate: &dyn Fn(&str) -> Result<(), String>,
    ) -> Result<String> {
        loop {
            self.record(key, default.map(str::to_string), false);
            let (answer, implicit) = match self.next_answer(key) {
                Some(a) => (a, false),
                None => (default.unwrap_or("").to_string(), true),
            };
            match validate(&answer) {
                Ok(()) => return Ok(answer),
                Err(why) if implicit => {
                    bail!("scripted prompter: default for `{key}` is rejected ({why}) and no answer is queued")
                }
                Err(_) => {}
            }
        }
    }

    fn secret(&mut self, key: &str, _label: &str, has_existing: bool) -> Result<Option<String>> {
        self.record(key, None, has_existing);
        Ok(self.next_answer(key).filter(|a| !a.is_empty()))
    }

    fn select(&mut self, key: &str, _label: &str, items: &[&str], default: usize) -> Result<usize> {
        self.record(key, items.get(default).map(|s| s.to_string()), false);
        match self.next_answer(key) {
            None => Ok(default),
            Some(a) => match items.iter().position(|i| *i == a) {
                Some(idx) => Ok(idx),
                None => bail!("scripted prompter: `{a}` is not one of {items:?} for `{key}`"),
            },
        }
    }

    fn confirm(&mut self, key: &str, _label: &str, default: bool) -> Result<bool> {
        self.record(
            key,
            Some(if default { "y" } else { "n" }.to_string()),
            false,
        );
        match self.next_answer(key).as_deref() {
            None => Ok(default),
            Some("y") | Some("yes") => Ok(true),
            Some("n") | Some("no") => Ok(false),
            Some(other) => bail!("scripted prompter: `{other}` is not y/n for `{key}`"),
        }
    }

    fn path(&mut self, key: &str, _label: &str, default: Option<&Path>) -> Result<PathBuf> {
        self.record(key, default.map(|p| p.display().to_string()), false);
        match self.next_answer(key) {
            Some(a) => Ok(PathBuf::from(a)),
            None => match default {
                Some(d) => Ok(d.to_path_buf()),
                None => bail!("scripted prompter: no answer queued for `{key}`"),
            },
        }
    }

    fn note(&mut self, text: &str) {
        self.notes.push(text.to_string());
    }
}

// ─── Terminal prompter ────────────────────────────────────────────────────────

/// `dialoguer` adapter for the real terminal. Deliberately logic-free: every
/// decision lives in the steps, which are tested through [`ScriptedPrompter`].
pub struct DialoguerPrompter {
    theme: dialoguer::theme::ColorfulTheme,
}

impl DialoguerPrompter {
    pub fn new() -> Self {
        Self {
            theme: dialoguer::theme::ColorfulTheme::default(),
        }
    }
}

impl Default for DialoguerPrompter {
    fn default() -> Self {
        Self::new()
    }
}

impl Prompter for DialoguerPrompter {
    fn input(
        &mut self,
        _key: &str,
        label: &str,
        default: Option<&str>,
        validate: &dyn Fn(&str) -> Result<(), String>,
    ) -> Result<String> {
        let mut prompt = dialoguer::Input::<String>::with_theme(&self.theme)
            .with_prompt(label)
            .allow_empty(true)
            .validate_with(|s: &String| validate(s));
        if let Some(d) = default {
            prompt = prompt.default(d.to_string());
        }
        Ok(prompt.interact_text()?)
    }

    fn secret(&mut self, _key: &str, label: &str, has_existing: bool) -> Result<Option<String>> {
        let prompt = if has_existing {
            format!("{label} (input hidden; Enter keeps the existing value)")
        } else {
            format!("{label} (input hidden)")
        };
        let value = dialoguer::Password::with_theme(&self.theme)
            .with_prompt(prompt)
            .allow_empty_password(true)
            .interact()?;
        Ok(Some(value).filter(|v| !v.is_empty()))
    }

    fn select(&mut self, _key: &str, label: &str, items: &[&str], default: usize) -> Result<usize> {
        Ok(dialoguer::Select::with_theme(&self.theme)
            .with_prompt(label)
            .items(items)
            .default(default)
            .interact()?)
    }

    fn confirm(&mut self, _key: &str, label: &str, default: bool) -> Result<bool> {
        Ok(dialoguer::Confirm::with_theme(&self.theme)
            .with_prompt(label)
            .default(default)
            .interact()?)
    }

    fn path(&mut self, _key: &str, label: &str, default: Option<&Path>) -> Result<PathBuf> {
        let mut prompt = dialoguer::Input::<String>::with_theme(&self.theme)
            .with_prompt(label)
            .allow_empty(true);
        if let Some(d) = default {
            prompt = prompt.default(d.display().to_string());
        }
        Ok(PathBuf::from(prompt.interact_text()?))
    }

    fn note(&mut self, text: &str) {
        println!("{text}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_input_when_rejected_and_nothing_queued_should_fail() {
        let mut p = ScriptedPrompter::new();
        let reject = |_: &str| Err("no".to_string());
        assert!(p.input("k", "l", None, &reject).is_err());
    }

    #[test]
    fn scripted_select_when_answer_not_an_item_should_fail() {
        let mut p = ScriptedPrompter::new().with("k", ["zzz"]);
        assert!(p.select("k", "l", &["a", "b"], 0).is_err());
    }

    #[test]
    fn scripted_secret_when_empty_answer_should_keep() {
        let mut p = ScriptedPrompter::new().with("k", [""]);
        assert_eq!(p.secret("k", "l", true).unwrap(), None);
    }
}
