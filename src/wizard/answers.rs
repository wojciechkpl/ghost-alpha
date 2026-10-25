//! Values collected by the wizard steps, before anything is written.

use std::fmt;
use std::str::FromStr;

use super::portfolio::PortfolioAnswer;

/// One wizard section, selectable with `ghost-alpha setup --section <name>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Section {
    /// Market-data provider and API keys (Finnhub, FRED)
    Data,
    /// SEC EDGAR User-Agent
    Edgar,
    /// Tax profile: state, filing status, other income
    Profile,
    /// Telegram / Discord channels and score thresholds
    Alerts,
    /// Database path and sentiment model directory
    Storage,
    /// Import positions and vesting CSVs into a data folder
    Portfolio,
}

impl Section {
    /// All sections in wizard order.
    pub const ALL: [Section; 6] = [
        Section::Data,
        Section::Edgar,
        Section::Profile,
        Section::Alerts,
        Section::Storage,
        Section::Portfolio,
    ];

    /// Lower-case CLI name (`"data"`, `"edgar"`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            Section::Data => "data",
            Section::Edgar => "edgar",
            Section::Profile => "profile",
            Section::Alerts => "alerts",
            Section::Storage => "storage",
            Section::Portfolio => "portfolio",
        }
    }
}

impl FromStr for Section {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let wanted = s.trim().to_ascii_lowercase();
        Section::ALL
            .into_iter()
            .find(|sec| sec.as_str() == wanted)
            .ok_or_else(|| {
                let names: Vec<&str> = Section::ALL.iter().map(|sec| sec.as_str()).collect();
                format!(
                    "unknown section {s:?} (possible values: {})",
                    names.join(", ")
                )
            })
    }
}

/// A secret value; `Debug` never prints it.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Secret(String);

impl Secret {
    pub(super) fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(super) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// `[profile]` answers.
#[derive(Debug, Clone)]
pub(super) struct ProfileAnswer {
    pub(super) state: String,
    pub(super) filing: String,
    pub(super) other_income: f64,
}

/// `[alerts]` answers plus the notification secrets.
#[derive(Debug, Clone)]
pub(super) struct AlertsAnswer {
    pub(super) critical: f64,
    pub(super) warning: f64,
    pub(super) info: f64,
    pub(super) telegram_token: Option<Secret>,
    pub(super) telegram_chat_id: Option<String>,
    pub(super) discord_webhook: Option<Secret>,
}

/// `[storage]` / `[sentiment]` answers.
#[derive(Debug, Clone)]
pub(super) struct StorageAnswer {
    pub(super) db_path: String,
    pub(super) model_dir: ModelDir,
}

/// What to do with `[sentiment] model_path`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ModelDir {
    /// Leave whatever is configured.
    Keep,
    /// Remove the setting (the user answered `-`).
    Clear,
    Set(String),
}

/// Opaque result of the steps; only [`super::plan_writes`] consumes it.
///
/// A `None` field means the step did not run, or the user kept the existing
/// value (secrets), so nothing is written for it.
#[derive(Debug, Clone, Default)]
pub struct SetupAnswers {
    pub(super) provider: Option<String>,
    pub(super) finnhub_key: Option<Secret>,
    pub(super) fred_key: Option<Secret>,
    pub(super) edgar_user_agent: Option<String>,
    pub(super) profile: Option<ProfileAnswer>,
    pub(super) alerts: Option<AlertsAnswer>,
    pub(super) storage: Option<StorageAnswer>,
    pub(super) portfolio: Option<PortfolioAnswer>,
}

impl SetupAnswers {
    /// True if any answer lives in `config.toml` (so the file must be planned).
    pub(super) fn touches_config(&self) -> bool {
        self.provider.is_some()
            || self.edgar_user_agent.is_some()
            || self.profile.is_some()
            || self.alerts.is_some()
            || self.storage.is_some()
    }

    pub(super) fn has_profile(&self) -> bool {
        self.profile.is_some()
    }

    pub(super) fn has_edgar(&self) -> bool {
        self.edgar_user_agent.is_some()
    }

    /// The data folder, if the portfolio step ran.
    pub(super) fn portfolio_dir(&self) -> Option<&std::path::Path> {
        self.portfolio.as_ref().map(|p| p.dir.as_path())
    }
}
