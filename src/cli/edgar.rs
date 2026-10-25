//! `ghost-alpha edgar ...`: SEC EDGAR facts and filings from the command line.
//!
//! Needs no `config.toml` or API key, only a SEC-compliant User-Agent
//! (`GHOST_ALPHA_EDGAR_USER_AGENT`, or `[edgar] user_agent` in config.toml).
//!
//! Environment variables: `GHOST_ALPHA_EDGAR_USER_AGENT`,
//! `GHOST_ALPHA_EDGAR_CACHE_DIR`, and the test/mirror overrides
//! `GHOST_ALPHA_EDGAR_DATA_URL` / `GHOST_ALPHA_EDGAR_WWW_URL`.

use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::NaiveDate;
use clap::{Subcommand, ValueEnum};
use serde_json::json;

use crate::data::edgar::{
    Cik, CompanyFacts, EdgarClient, EdgarConfig, EdgarError, Fact, Filing, Periodicity,
    DEFAULT_DATA_BASE, DEFAULT_WWW_BASE,
};

/// Env var holding the SEC User-Agent (`"<app or name> <contact email>"`).
pub const ENV_USER_AGENT: &str = "GHOST_ALPHA_EDGAR_USER_AGENT";
/// Env var overriding the cache directory.
pub const ENV_CACHE_DIR: &str = "GHOST_ALPHA_EDGAR_CACHE_DIR";
/// Env var overriding the `data.sec.gov` base URL.
pub const ENV_DATA_URL: &str = "GHOST_ALPHA_EDGAR_DATA_URL";
/// Env var overriding the `www.sec.gov` base URL.
pub const ENV_WWW_URL: &str = "GHOST_ALPHA_EDGAR_WWW_URL";

/// Output format for `edgar` subcommands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum EdgarFormat {
    /// Human-readable text table.
    Table,
    /// Machine-readable JSON array on stdout.
    Json,
}

/// Fiscal period granularity for `edgar facts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PeriodicityArg {
    /// 10-K fiscal-year values.
    Annual,
    /// 10-Q Q1-Q3 values.
    Quarterly,
}

impl From<PeriodicityArg> for Periodicity {
    fn from(p: PeriodicityArg) -> Self {
        match p {
            PeriodicityArg::Annual => Periodicity::Annual,
            PeriodicityArg::Quarterly => Periodicity::Quarterly,
        }
    }
}

/// `ghost-alpha edgar` subcommands.
#[derive(Debug, Subcommand)]
pub enum EdgarCommand {
    /// Point-in-time XBRL facts for a concept (restatements appear only after they were filed)
    Facts {
        /// Ticker (e.g. AAPL) or all-digit CIK
        ticker: String,
        /// XBRL concept name
        #[arg(long, default_value = "Revenues")]
        concept: String,
        /// XBRL taxonomy
        #[arg(long, default_value = "us-gaap")]
        taxonomy: String,
        /// Unit of measure (e.g. USD, shares, USD/shares)
        #[arg(long, default_value = "USD")]
        unit: String,
        /// Annual (10-K) or quarterly (10-Q, Q1-Q3) values
        #[arg(long, value_enum, default_value = "annual")]
        periodicity: PeriodicityArg,
        /// Only use filings filed on or before this date (YYYY-MM-DD); default today
        #[arg(long)]
        as_of: Option<NaiveDate>,
        /// Output format
        #[arg(long, value_enum, default_value = "table")]
        format: EdgarFormat,
    },
    /// List recent filings, optionally filtered by form, 8-K item and date
    Filings {
        /// Ticker (e.g. AAPL) or all-digit CIK
        ticker: String,
        /// Exact form type (e.g. 8-K; 8-K/A is a different form)
        #[arg(long)]
        form: Option<String>,
        /// 8-K item number (e.g. 2.02)
        #[arg(long)]
        item: Option<String>,
        /// Only filings filed on or after this date (YYYY-MM-DD)
        #[arg(long)]
        since: Option<NaiveDate>,
        /// Output format
        #[arg(long, value_enum, default_value = "table")]
        format: EdgarFormat,
    },
}

/// Cache directory: `$XDG_CACHE_HOME`, else `~/Library/Caches` (macOS) or
/// `~/.cache`, followed by `ghost-alpha/edgar`.
pub fn default_cache_dir(
    xdg_cache_home: Option<PathBuf>,
    home: Option<PathBuf>,
    macos: bool,
) -> Result<PathBuf, EdgarError> {
    let base = match (
        xdg_cache_home.filter(|p| p.is_absolute()),
        home.filter(|p| p.is_absolute()),
    ) {
        (Some(xdg), _) => xdg,
        (None, Some(h)) if macos => h.join("Library").join("Caches"),
        (None, Some(h)) => h.join(".cache"),
        (None, None) => {
            return Err(EdgarError::InvalidConfig(format!(
                "cannot determine a cache directory; set {ENV_CACHE_DIR}"
            )))
        }
    };
    Ok(base.join("ghost-alpha").join("edgar"))
}

fn env_non_empty(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn cache_dir_from_env() -> Result<PathBuf, EdgarError> {
    if let Some(dir) = env_non_empty(ENV_CACHE_DIR) {
        return Ok(PathBuf::from(dir));
    }
    default_cache_dir(
        std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        cfg!(target_os = "macos"),
    )
}

/// Build a client from env vars plus the optional config-file User-Agent.
fn build_client(config_user_agent: Option<&str>) -> Result<EdgarClient, EdgarError> {
    let env_ua = env_non_empty(ENV_USER_AGENT);
    let cfg = EdgarConfig::resolve(config_user_agent, env_ua.as_deref(), cache_dir_from_env()?)?;
    let client = EdgarClient::new(cfg)?;
    Ok(
        match (env_non_empty(ENV_DATA_URL), env_non_empty(ENV_WWW_URL)) {
            (None, None) => client,
            (data, www) => client.with_base_urls(
                data.unwrap_or_else(|| DEFAULT_DATA_BASE.into()),
                www.unwrap_or_else(|| DEFAULT_WWW_BASE.into()),
            ),
        },
    )
}

/// Run an `edgar` subcommand, printing results to stdout.
///
/// `config_user_agent` is the `[edgar] user_agent` from config.toml, if any;
/// the env var takes precedence.
pub async fn run(cmd: EdgarCommand, config_user_agent: Option<&str>) -> Result<()> {
    let client = build_client(config_user_agent)?;
    match cmd {
        EdgarCommand::Facts {
            ticker,
            concept,
            taxonomy,
            unit,
            periodicity,
            as_of,
            format,
        } => {
            let as_of = as_of.unwrap_or_else(|| chrono::Local::now().date_naive());
            let facts = client
                .company_facts(&ticker)
                .await
                .with_context(|| format!("edgar facts failed for {ticker}"))?;
            let rows = facts.series(&taxonomy, &concept, &unit, periodicity.into(), as_of);
            let periodicity: Periodicity = periodicity.into();
            let hint = missing_hint(&facts, &taxonomy, &concept, &unit).or_else(|| {
                if rows.is_empty() {
                    form_hint(&facts, &taxonomy, &concept, &unit, periodicity)
                } else {
                    stale_hint(
                        &facts,
                        &taxonomy,
                        &concept,
                        &unit,
                        periodicity,
                        as_of,
                        &rows,
                    )
                }
            });
            if let Some(hint) = hint {
                eprintln!("{hint}");
            }
            match format {
                EdgarFormat::Json => println!("{}", serde_json::to_string_pretty(&rows)?),
                EdgarFormat::Table => {
                    print!(
                        "{}",
                        render_facts_table(&facts.entity_name, &concept, &unit, as_of, &rows)
                    )
                }
            }
        }
        EdgarCommand::Filings {
            ticker,
            form,
            item,
            since,
            format,
        } => {
            let cik = client
                .resolve_cik(&ticker)
                .await
                .with_context(|| format!("edgar filings failed for {ticker}"))?;
            let report = client
                .filings_report(cik.as_str(), form.as_deref(), item.as_deref(), since)
                .await
                .with_context(|| format!("edgar filings failed for {ticker}"))?;
            for w in &report.warnings {
                eprintln!("warning: {w}");
            }
            if since.is_none() && report.older_pages_exist {
                if let Some(oldest) = report.oldest_recent {
                    eprintln!(
                        "note: showing only the most recent filings (back to {oldest}); older filings exist. \
                         Use --since YYYY-MM-DD to search further back."
                    );
                }
            }
            match format {
                EdgarFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&filings_json(&cik, &report.filings))?
                    )
                }
                EdgarFormat::Table => print!("{}", render_filings_table(&cik, &report.filings)),
            }
        }
    }
    Ok(())
}

fn filings_json(cik: &Cik, filings: &[Filing]) -> serde_json::Value {
    filings
        .iter()
        .map(|f| {
            json!({
                "accession_number": f.accession_number,
                "filing_date": f.filing_date,
                "report_date": f.report_date,
                "form": f.form,
                "primary_document": f.primary_document,
                "items": f.items,
                "url": f.document_url(cik),
            })
        })
        .collect()
}

/// Stderr hint when the requested concept/unit does not exist for the company.
fn missing_hint(facts: &CompanyFacts, taxonomy: &str, concept: &str, unit: &str) -> Option<String> {
    let units = facts.units(taxonomy, concept);
    if units.is_empty() {
        let needle = concept.to_lowercase();
        let similar: Vec<&str> = facts
            .concepts(taxonomy)
            .into_iter()
            .filter(|c| c.to_lowercase().contains(&needle))
            .take(5)
            .collect();
        let mut msg = format!("concept {concept:?} not found in taxonomy {taxonomy:?}");
        if similar.is_empty() {
            msg.push_str("; no similar concept names");
        } else {
            msg.push_str(&format!("; similar concepts: {}", similar.join(", ")));
        }
        if needle.contains("revenue") || needle.contains("sales") {
            msg.push_str(
                "\nnote: since ASC 606 many filers report revenue as \
                 RevenueFromContractWithCustomerExcludingAssessedTax instead of Revenues",
            );
        }
        Some(msg)
    } else if !units.contains(&unit) {
        Some(format!(
            "unit {unit:?} not found for concept {concept:?}; available units: {}",
            units.join(", ")
        ))
    } else {
        None
    }
}

/// Window (in days) within which a concept counts as current for `periodicity`.
fn recency_days(periodicity: Periodicity) -> i64 {
    match periodicity {
        Periodicity::Annual => 2 * 365,
        Periodicity::Quarterly => 365,
    }
}

/// Stderr note when a non-empty series ends long before the filer's latest
/// periodic filing (the filer switched to another concept), listing active
/// alternatives in the same taxonomy and unit.
fn stale_hint(
    facts: &CompanyFacts,
    taxonomy: &str,
    concept: &str,
    unit: &str,
    periodicity: Periodicity,
    as_of: NaiveDate,
    rows: &[&Fact],
) -> Option<String> {
    let last_end = rows.last()?.end;
    let latest = facts.latest_periodic_filing(as_of)?;
    let window = chrono::Duration::days(recency_days(periodicity));
    if latest - last_end <= window {
        return None;
    }
    let revenue_like = concept.contains("Revenue") || concept.contains("Sales");
    let shared = |name: &str| {
        name.chars()
            .zip(concept.chars())
            .take_while(|(a, b)| a == b)
            .count()
    };
    let instant = rows.last()?.start.is_none();
    // Candidates must have the same shape (duration vs instant) and a series of
    // their own for this periodicity that is still current.
    let mut alternatives: Vec<&str> = facts
        .concepts(taxonomy)
        .into_iter()
        .filter(|c| *c != concept && (shared(c) >= 8 || (revenue_like && c.contains("Revenue"))))
        .filter(|c| {
            let alt = facts.series(taxonomy, c, unit, periodicity, as_of);
            alt.last()
                .is_some_and(|l| l.start.is_none() == instant && latest - l.end <= window)
        })
        .collect();
    alternatives.sort_by(|a, b| {
        let rev = |n: &str| revenue_like && n.starts_with("Revenue");
        shared(b)
            .cmp(&shared(a))
            .then_with(|| rev(b).cmp(&rev(a)))
            .then_with(|| a.cmp(b))
    });
    alternatives.truncate(5);
    let mut msg = format!(
        "note: {concept} stops at {last_end} but the filer's latest 10-K/10-Q was filed {latest}; \
         it probably switched concepts."
    );
    if !alternatives.is_empty() {
        msg.push_str(&format!(" Try: {}", alternatives.join(", ")));
    }
    Some(msg)
}

/// Stderr explanation when the concept has facts, but none from the forms the
/// periodicity accepts (e.g. a 20-F filer).
fn form_hint(
    facts: &CompanyFacts,
    taxonomy: &str,
    concept: &str,
    unit: &str,
    periodicity: Periodicity,
) -> Option<String> {
    let present = facts.forms(taxonomy, concept, unit);
    let (label, used): (&str, [&str; 2]) = match periodicity {
        Periodicity::Annual => ("annual", ["10-K", "10-K/A"]),
        Periodicity::Quarterly => ("quarterly", ["10-Q", "10-Q/A"]),
    };
    if present.is_empty() || present.iter().any(|f| used.contains(f)) {
        return None;
    }
    Some(format!(
        "note: {concept} ({unit}) is reported only on forms {}; the {label} series uses {}.",
        present.join(", "),
        used.join(" or ")
    ))
}

fn render_facts_table(
    entity: &str,
    concept: &str,
    unit: &str,
    as_of: NaiveDate,
    rows: &[&Fact],
) -> String {
    let mut out = format!("{entity} - {concept} ({unit}), as of {as_of}\n");
    if rows.is_empty() {
        out.push_str("(no matching facts)\n");
        return out;
    }
    out.push_str(&format!(
        "{:<10}  {:<10}  {:>18}  {:<12}  {:<8}  {:<10}  {}\n",
        "START", "END", "VALUE", "FILING FY/FP", "FORM", "FILED", "ACCN"
    ));
    for f in rows {
        let fyfp = match (f.fy, f.fp.as_deref()) {
            (Some(fy), Some(fp)) => format!("{fy}/{fp}"),
            (Some(fy), None) => fy.to_string(),
            (None, Some(fp)) => fp.to_string(),
            (None, None) => "-".to_string(),
        };
        let start = f.start.map_or_else(String::new, |d| d.to_string());
        out.push_str(&format!(
            "{:<10}  {:<10}  {:>18}  {:<12}  {:<8}  {:<10}  {}\n",
            start, f.end, f.val, fyfp, f.form, f.filed, f.accn
        ));
    }
    out
}

fn render_filings_table(cik: &Cik, filings: &[Filing]) -> String {
    if filings.is_empty() {
        return "(no matching filings)\n".to_string();
    }
    let mut out = format!(
        "{:<10}  {:<10}  {:<8}  {:<20}  {:<12}  {}\n",
        "FILED", "REPORT", "FORM", "ACCESSION", "ITEMS", "URL"
    );
    for f in filings {
        let report = f
            .report_date
            .map_or_else(|| "-".to_string(), |d| d.to_string());
        out.push_str(&format!(
            "{:<10}  {:<10}  {:<8}  {:<20}  {:<12}  {}\n",
            f.filing_date,
            report,
            f.form,
            f.accession_number,
            f.items.join(","),
            f.document_url(cik)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cache_dir_prefers_absolute_xdg() {
        let d = default_cache_dir(Some("/x/cache".into()), Some("/h".into()), true).unwrap();
        assert_eq!(d, PathBuf::from("/x/cache/ghost-alpha/edgar"));
    }

    #[test]
    fn default_cache_dir_falls_back_per_platform() {
        let mac = default_cache_dir(None, Some("/h".into()), true).unwrap();
        assert_eq!(mac, PathBuf::from("/h/Library/Caches/ghost-alpha/edgar"));
        let linux = default_cache_dir(Some("relative".into()), Some("/h".into()), false).unwrap();
        assert_eq!(linux, PathBuf::from("/h/.cache/ghost-alpha/edgar"));
    }

    #[test]
    fn default_cache_dir_rejects_empty_or_relative_home() {
        for home in ["", "relative/home"] {
            let e = default_cache_dir(None, Some(home.into()), false).unwrap_err();
            assert!(e.to_string().contains(ENV_CACHE_DIR), "{home:?}");
        }
    }

    #[test]
    fn default_cache_dir_without_home_is_actionable_error() {
        let e = default_cache_dir(None, None, false).unwrap_err();
        assert!(e.to_string().contains(ENV_CACHE_DIR));
    }
}
