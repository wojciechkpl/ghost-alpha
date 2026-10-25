//! The wizard steps. Each asks through a [`Prompter`], pre-fills every answer
//! from the existing files, and validates before accepting.

use std::path::{Path, PathBuf};

use anyhow::{bail, Result};

use super::answers::{
    AlertsAnswer, ModelDir, ProfileAnswer, Secret, Section, SetupAnswers, StorageAnswer,
};
use super::existing::ExistingSetup;
use super::portfolio;
use super::prompter::Prompter;
use crate::config::{
    AlertsConfig, ENV_DISCORD_WEBHOOK_URL, ENV_FINNHUB_API_KEY, ENV_FRED_API_KEY,
    ENV_TELEGRAM_BOT_TOKEN, ENV_TELEGRAM_CHAT_ID,
};
use crate::data::edgar::EdgarConfig;
use crate::tax::brackets::state_tax_rate;

/// Attempts for a prompt that keeps being answered unusably before giving up.
const MAX_ATTEMPTS: usize = 10;

/// States with a tax-table entry (`tax::brackets::state_tax_rate`).
pub(super) const SUPPORTED_STATES: [&str; 10] =
    ["CA", "CO", "FL", "IL", "MA", "NJ", "NV", "NY", "TX", "WA"];

/// Run the steps (all, or one `section`) and collect answers.
pub fn run_steps(
    prompter: &mut dyn Prompter,
    existing: &ExistingSetup,
    section: Option<Section>,
) -> Result<SetupAnswers> {
    let wants = |s: Section| section.is_none_or(|only| only == s);
    let mut answers = SetupAnswers::default();
    if wants(Section::Data) {
        data_step(prompter, existing, &mut answers)?;
    }
    if wants(Section::Edgar) {
        edgar_step(prompter, existing, &mut answers)?;
    }
    if wants(Section::Profile) {
        profile_step(prompter, existing, &mut answers)?;
    }
    if wants(Section::Alerts) {
        alerts_step(prompter, existing, &mut answers)?;
    }
    if wants(Section::Storage) {
        storage_step(prompter, existing, &mut answers)?;
    }
    if wants(Section::Portfolio) {
        // `--section portfolio` is an explicit request; a full run asks first.
        let enabled = section == Some(Section::Portfolio)
            || prompter.confirm(
                "portfolio.enabled",
                "Import portfolio data (positions CSVs, vesting) into a data folder?",
                false,
            )?;
        if enabled {
            answers.portfolio = Some(portfolio::run(prompter)?);
        }
    }
    Ok(answers)
}

fn has_env(existing: &ExistingSetup, key: &str) -> bool {
    existing
        .env_value(key)
        .is_some_and(|v| !v.trim().is_empty())
}

/// Ask for a secret. `Ok(None)` = keep what is there. A `required` secret with
/// nothing on file is asked again until given.
fn ask_secret(
    prompter: &mut dyn Prompter,
    key: &str,
    label: &str,
    has_existing: bool,
    required: bool,
) -> Result<Option<Secret>> {
    for _ in 0..MAX_ATTEMPTS {
        match prompter.secret(key, label, has_existing)? {
            Some(v) if !v.trim().is_empty() => return Ok(Some(Secret::new(v.trim()))),
            _ if has_existing || !required => return Ok(None),
            _ => prompter.note("  A value is required."),
        }
    }
    bail!("no value given for `{key}`")
}

fn data_step(
    prompter: &mut dyn Prompter,
    existing: &ExistingSetup,
    answers: &mut SetupAnswers,
) -> Result<()> {
    prompter.note("\nData sources");
    let providers = ["finnhub", "yahoo"];
    let current = existing
        .doc_str(&["plugins", "active_provider"])
        .unwrap_or("finnhub");
    let default = providers.iter().position(|p| *p == current).unwrap_or(0);
    let idx = prompter.select("data.provider", "Market data provider", &providers, default)?;
    let provider = providers.get(idx).copied().unwrap_or("finnhub");
    answers.provider = Some(provider.to_string());

    if provider == "finnhub" {
        answers.finnhub_key = ask_secret(
            prompter,
            "data.finnhub_key",
            "Finnhub API key (free at https://finnhub.io)",
            has_env(existing, ENV_FINNHUB_API_KEY),
            true,
        )?;
    }
    answers.fred_key = ask_secret(
        prompter,
        "data.fred_key",
        "FRED API key (optional, enables macro-risk signals)",
        has_env(existing, ENV_FRED_API_KEY),
        false,
    )?;
    Ok(())
}

fn edgar_step(
    prompter: &mut dyn Prompter,
    existing: &ExistingSetup,
    answers: &mut SetupAnswers,
) -> Result<()> {
    prompter.note("\nSEC EDGAR");
    let validate = |s: &str| {
        EdgarConfig::new(s, PathBuf::new())
            .validate()
            .map_err(|e| e.to_string())
    };
    let ua = prompter.input(
        "edgar.user_agent",
        "SEC User-Agent: your name and contact email, e.g. Jane Doe jane.doe@example.com",
        existing.doc_str(&["edgar", "user_agent"]),
        &validate,
    )?;
    answers.edgar_user_agent = Some(ua.trim().to_string());
    Ok(())
}

/// Parse a dollar amount, tolerating `$` and thousands separators.
fn parse_money(s: &str) -> Option<f64> {
    s.trim()
        .replace(['$', ','], "")
        .parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
}

fn profile_step(
    prompter: &mut dyn Prompter,
    existing: &ExistingSetup,
    answers: &mut SetupAnswers,
) -> Result<()> {
    prompter.note("\nTax profile (defaults for --state, --filing, --other-income)");
    let check_state = |s: &str| {
        state_tax_rate(&s.trim().to_uppercase())
            .map(|_| ())
            .map_err(|_| {
                format!(
                    "unsupported state {:?} (supported: {})",
                    s.trim(),
                    SUPPORTED_STATES.join(", ")
                )
            })
    };
    let state = prompter.input(
        "profile.state",
        "State of residence (2-letter code)",
        existing.doc_str(&["profile", "state"]),
        &check_state,
    )?;

    let current_filing = existing.doc_str(&["profile", "filing"]);
    let mut filings = vec!["single", "mfj"];
    if let Some(other) = current_filing.filter(|f| !filings.contains(f)) {
        filings.push(other); // keep an existing mfs / hoh choice selectable
    }
    let default = filings
        .iter()
        .position(|f| Some(*f) == current_filing)
        .unwrap_or(0);
    let idx = prompter.select("profile.filing", "Filing status", &filings, default)?;
    let filing = filings.get(idx).copied().unwrap_or("single");

    let default_income = existing
        .doc_f64(&["profile", "other_income"])
        .map_or_else(|| "0".to_string(), |v| v.to_string());
    let check_income = |s: &str| match parse_money(s) {
        Some(v) if v >= 0.0 => Ok(()),
        Some(_) => Err("income cannot be negative".to_string()),
        None => Err("enter a number, e.g. 150000".to_string()),
    };
    let income = prompter.input(
        "profile.other_income",
        "Other annual income in dollars (salary etc.)",
        Some(&default_income),
        &check_income,
    )?;

    answers.profile = Some(ProfileAnswer {
        state: state.trim().to_uppercase(),
        filing: filing.to_string(),
        other_income: parse_money(&income).unwrap_or(0.0),
    });
    Ok(())
}

fn check_threshold(s: &str) -> Result<(), String> {
    match s.trim().parse::<f64>() {
        Ok(v) if v.is_finite() && (0.0..=100.0).contains(&v) => Ok(()),
        _ => Err("enter a score between 0 and 100".to_string()),
    }
}

fn alerts_step(
    prompter: &mut dyn Prompter,
    existing: &ExistingSetup,
    answers: &mut SetupAnswers,
) -> Result<()> {
    prompter.note("\nAlerts");
    let has_token = has_env(existing, ENV_TELEGRAM_BOT_TOKEN);
    let telegram_token = ask_secret(
        prompter,
        "alerts.telegram_token",
        "Telegram bot token (from @BotFather, optional)",
        has_token,
        false,
    )?;
    let mut telegram_chat_id = None;
    if telegram_token.is_some() || has_token {
        let need_chat = |s: &str| {
            if s.trim().is_empty() {
                Err("a chat ID is required with a bot token".to_string())
            } else {
                Ok(())
            }
        };
        let chat = prompter.input(
            "alerts.telegram_chat_id",
            "Telegram chat ID",
            existing.env_value(ENV_TELEGRAM_CHAT_ID),
            &need_chat,
        )?;
        telegram_chat_id = Some(chat.trim().to_string());
    }
    let discord_webhook = ask_secret(
        prompter,
        "alerts.discord_webhook",
        "Discord webhook URL (optional)",
        has_env(existing, ENV_DISCORD_WEBHOOK_URL),
        false,
    )?;

    let defaults = AlertsConfig::default();
    let current = |name: &str, fallback: f64| {
        existing
            .doc_f64(&["alerts", name])
            .unwrap_or(fallback)
            .to_string()
    };
    let mut values = [
        current(
            "critical_score_threshold",
            defaults.critical_score_threshold,
        ),
        current("warning_score_threshold", defaults.warning_score_threshold),
        current("info_score_threshold", defaults.info_score_threshold),
    ];
    let keys = ["alerts.critical", "alerts.warning", "alerts.info"];
    let labels = [
        "CRITICAL alert score threshold",
        "WARNING alert score threshold",
        "INFO alert score threshold",
    ];
    for _ in 0..MAX_ATTEMPTS {
        for i in 0..3 {
            let answer = prompter.input(keys[i], labels[i], Some(&values[i]), &check_threshold)?;
            values[i] = answer.trim().to_string();
        }
        let [critical, warning, info] = values.clone().map(|v| v.parse::<f64>().unwrap_or(0.0));
        if critical > warning && warning > info {
            answers.alerts = Some(AlertsAnswer {
                critical,
                warning,
                info,
                telegram_token: telegram_token.clone(),
                telegram_chat_id: telegram_chat_id.clone(),
                discord_webhook: discord_webhook.clone(),
            });
            return Ok(());
        }
        prompter.note("  Thresholds must satisfy critical > warning > info. Please re-enter them.");
    }
    bail!("alert thresholds are still out of order after {MAX_ATTEMPTS} attempts")
}

fn storage_step(
    prompter: &mut dyn Prompter,
    existing: &ExistingSetup,
    answers: &mut SetupAnswers,
) -> Result<()> {
    prompter.note("\nStorage and sentiment");
    let check_db = |s: &str| {
        let path = Path::new(s.trim());
        if s.trim().is_empty() {
            return Err("a path is required".to_string());
        }
        match path.parent() {
            Some(parent) if parent.is_file() => {
                Err(format!("{} is a file, not a directory", parent.display()))
            }
            _ => Ok(()),
        }
    };
    let db_path = prompter.input(
        "storage.db_path",
        "SQLite database path",
        Some(
            existing
                .doc_str(&["storage", "db_path"])
                .unwrap_or("ghost_alpha.db"),
        ),
        &check_db,
    )?;
    let model_dir = prompter.input(
        "storage.model_dir",
        "FinBERT model directory (optional; Enter keeps the current value, - removes it)",
        existing.doc_str(&["sentiment", "model_path"]),
        &|_| Ok(()),
    )?;
    let model_dir = match model_dir.trim() {
        "" => ModelDir::Keep,
        "-" => ModelDir::Clear,
        dir => ModelDir::Set(dir.to_string()),
    };
    answers.storage = Some(StorageAnswer {
        db_path: db_path.trim().to_string(),
        model_dir,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_states_should_all_have_a_tax_table_entry() {
        for s in SUPPORTED_STATES {
            assert!(state_tax_rate(s).is_ok(), "{s}");
        }
    }

    #[test]
    fn parse_money_when_decorated_should_accept_dollars_and_commas() {
        assert_eq!(parse_money("$150,000"), Some(150_000.0));
        assert_eq!(parse_money(" 0 "), Some(0.0));
        assert_eq!(parse_money("lots"), None);
        assert_eq!(parse_money("inf"), None);
    }
}
