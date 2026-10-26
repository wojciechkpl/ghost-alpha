//! SEC EDGAR client: ticker -> CIK resolution, XBRL company facts and filing lists.
//!
//! Free, key-less data from `data.sec.gov` / `www.sec.gov`. SEC requires every
//! automated client to send a descriptive `User-Agent` containing a contact
//! email (see [`EdgarConfig::validate`]) and to stay under 10 requests/second.
//!
//! The module has three layers:
//! * pure parsers ([`parse_company_tickers`], [`parse_company_facts`],
//!   [`parse_submissions`]) with no I/O,
//! * point-in-time queries on [`CompanyFacts`] ([`CompanyFacts::value_as_of`],
//!   [`CompanyFacts::series`]) that only see facts already filed at `as_of`,
//! * [`EdgarClient`]: rate-limited HTTP with retry/backoff and an on-disk cache.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::NaiveDate;
use governor::clock::DefaultClock;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Default base URL for `data.sec.gov` (XBRL facts, submissions).
pub const DEFAULT_DATA_BASE: &str = "https://data.sec.gov";
/// Default base URL for `www.sec.gov` (ticker list, archives).
pub const DEFAULT_WWW_BASE: &str = "https://www.sec.gov";
const TICKERS_CACHE_KEY: &str = "company_tickers.json";
/// Upper bound for a server-supplied `Retry-After` so a bad header cannot hang the CLI.
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);
/// Upper bound for exponential backoff growth.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Archive documents (N-PORT files of bond funds reach 30-40 MB) get longer.
const DOCUMENT_TIMEOUT: Duration = Duration::from_secs(120);
/// How far in the future a cache file's mtime may be and still count as fresh.
const FUTURE_MTIME_TOLERANCE: Duration = Duration::from_secs(300);
const MAX_SUGGESTIONS: usize = 5;

/// How to fix a missing or malformed User-Agent; appended to the related errors.
const USER_AGENT_HELP: &str =
    "SEC requires a User-Agent of the form \"<app or name> <contact email>\"; \
set the GHOST_ALPHA_EDGAR_USER_AGENT env var (e.g. \"Jane Doe jane@example.com\") \
or `user_agent` under [edgar] in config.toml";

// ─── Errors ───────────────────────────────────────────────────────────────────

/// Errors produced by the EDGAR client and parsers.
#[derive(Debug, thiserror::Error)]
pub enum EdgarError {
    /// Not a valid CIK (1-10 ASCII digits).
    #[error("invalid CIK {0:?}: expected 1-10 digits")]
    InvalidCik(String),
    /// User-Agent missing, blank or without a contact email.
    #[error("invalid SEC User-Agent: {0}")]
    InvalidUserAgent(String),
    /// Other invalid configuration value.
    #[error("invalid EDGAR config: {0}")]
    InvalidConfig(String),
    /// Malformed JSON or unexpected document structure.
    #[error("failed to parse EDGAR response: {0}")]
    Parse(String),
    /// Ticker not present in SEC's `company_tickers.json`.
    #[error("unknown ticker {ticker:?}{}", fmt_suggestions(.suggestions))]
    UnknownTicker {
        /// Ticker as given by the caller.
        ticker: String,
        /// Similar tickers (see [`TickerMap::suggest`]).
        suggestions: Vec<String>,
    },
    /// HTTP 404 from SEC.
    #[error("not found on SEC EDGAR: {url}{}", not_found_hint(.url))]
    NotFound {
        /// Requested URL.
        url: String,
    },
    /// HTTP 403: SEC rejected the User-Agent or temporarily blocked the client. Never retried.
    #[error(
        "SEC EDGAR refused the request (HTTP 403) for {url}: check that your User-Agent \
         (\"<app or name> <contact email>\") is valid, and if you exceeded the rate limit \
         wait about 10 minutes before retrying"
    )]
    Forbidden {
        /// Requested URL.
        url: String,
    },
    /// Non-success HTTP status after retries were exhausted (or not retryable).
    #[error("SEC EDGAR returned HTTP {status} for {url}")]
    HttpStatus {
        /// HTTP status code.
        status: u16,
        /// Requested URL.
        url: String,
    },
    /// Transport-level failure (DNS, TLS, timeout, ...) after retries.
    #[error("network error talking to SEC EDGAR: {0}")]
    Network(String),
    /// Ticker not present in SEC's mutual-fund / ETF ticker map.
    #[error("unknown fund ticker {ticker:?}{}", fmt_suggestions(.suggestions))]
    UnknownFund {
        /// Ticker as given by the caller.
        ticker: String,
        /// Similar fund tickers.
        suggestions: Vec<String>,
    },
    /// Known to SEC but not a fund series, so it files no N-PORT (a unit investment
    /// trust such as SPY, or an operating company).
    #[error(
        "{ticker} is not a registered fund series, so it files no N-PORT: it is a unit \
         investment trust (e.g. SPY) or an operating company; no holdings data"
    )]
    NotNportFiler {
        /// Ticker as given by the caller.
        ticker: String,
    },
    /// The fetched N-PORT document belongs to a different series than the fund map says.
    #[error("N-PORT document is for series {found}, expected {expected} ({ticker})")]
    SeriesMismatch {
        /// Ticker as given by the caller.
        ticker: String,
        /// Series the fund map says the ticker belongs to.
        expected: String,
        /// Series found in the document.
        found: String,
    },
    /// The fund has no N-PORT filing on or before `as_of`.
    #[error("no N-PORT filing for {ticker} on or before {as_of}")]
    NoNportFiling {
        /// Ticker as given by the caller.
        ticker: String,
        /// Requested point-in-time date.
        as_of: NaiveDate,
    },
}

fn not_found_hint(url: &str) -> &'static str {
    if url.contains("/companyfacts/") {
        " (the company may not file XBRL financial data, e.g. funds, trusts or foreign filers)"
    } else {
        ""
    }
}

fn fmt_suggestions(s: &[String]) -> String {
    if s.is_empty() {
        String::new()
    } else {
        format!(" (did you mean: {}?)", s.join(", "))
    }
}

pub(super) fn parse_err(e: impl fmt::Display) -> EdgarError {
    EdgarError::Parse(e.to_string())
}

pub(super) fn parse_date(s: &str, what: &str) -> Result<NaiveDate, EdgarError> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .map_err(|e| EdgarError::Parse(format!("bad {what} date {s:?}: {e}")))
}

// ─── CIK ──────────────────────────────────────────────────────────────────────

/// SEC Central Index Key, normalised to 10 zero-padded digits.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Cik {
    padded: String,
    n: u64,
}

impl Cik {
    /// Build from a number (e.g. `320193`).
    pub fn new(n: u64) -> Self {
        Self {
            padded: format!("{n:010}"),
            n,
        }
    }

    /// Parse 1-10 ASCII digits, padded or not.
    pub fn parse(s: &str) -> Result<Self, EdgarError> {
        let s = s.trim();
        if s.is_empty() || s.len() > 10 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(EdgarError::InvalidCik(s.to_string()));
        }
        s.parse::<u64>()
            .map(Self::new)
            .map_err(|_| EdgarError::InvalidCik(s.to_string()))
    }

    /// The 10-digit zero-padded form used in SEC URLs.
    pub fn as_str(&self) -> &str {
        &self.padded
    }

    /// The numeric value (used in `Archives` URLs, which are not padded).
    pub fn numeric(&self) -> u64 {
        self.n
    }
}

impl fmt::Display for Cik {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.padded)
    }
}

/// Accepts the JSON number or string forms SEC uses for CIKs.
pub(super) fn cik_from_value(v: &Value) -> Result<Cik, EdgarError> {
    match v {
        Value::Number(n) => n
            .as_u64()
            .map(Cik::new)
            .ok_or_else(|| EdgarError::Parse(format!("bad CIK {n}"))),
        Value::String(s) => Cik::parse(s).map_err(parse_err),
        other => Err(EdgarError::Parse(format!("bad CIK {other}"))),
    }
}

// ─── Ticker map ───────────────────────────────────────────────────────────────

/// Normalise a ticker the way SEC spells class shares: `.`, `/`, `_` and spaces
/// become `-`, then upper-case (`brk.b` -> `BRK-B`).
pub(super) fn normalize_ticker(t: &str) -> String {
    t.trim()
        .chars()
        .map(|c| match c {
            '.' | '/' | '_' | ' ' => '-',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}

/// Ticker -> CIK lookup built from `company_tickers.json` (case-insensitive,
/// class-share separators normalised).
#[derive(Debug, Clone, Default)]
pub struct TickerMap {
    map: HashMap<String, Cik>,
    titles: HashMap<String, String>,
}

impl TickerMap {
    /// CIK for `ticker`, ignoring case and treating `.`, `/`, `_`, ` ` as `-`.
    pub fn get(&self, ticker: &str) -> Option<&Cik> {
        self.map.get(&normalize_ticker(ticker))
    }

    /// EDGAR company title for `ticker` (same normalisation as [`TickerMap::get`]).
    pub fn title(&self, ticker: &str) -> Option<&str> {
        self.titles
            .get(&normalize_ticker(ticker))
            .map(String::as_str)
    }

    /// Number of tickers.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when there are no tickers.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Up to five tickers that start with the (normalised) `query`, or that
    /// `query` starts with. Ranked by longest shared prefix with the query,
    /// ties alphabetical. One-character tickers are only offered for a
    /// one-character query.
    pub fn suggest(&self, query: &str) -> Vec<String> {
        suggest_tickers(self.map.keys(), query)
    }
}

/// Ranking behind [`TickerMap::suggest`], shared with the fund ticker map.
pub(super) fn suggest_tickers<'a>(
    keys: impl Iterator<Item = &'a String>,
    query: &str,
) -> Vec<String> {
    let q = normalize_ticker(query);
    if q.is_empty() {
        return Vec::new();
    }
    let shared = |t: &str| t.chars().zip(q.chars()).take_while(|(a, b)| a == b).count();
    let mut v: Vec<&String> = keys
        .filter(|t| t.starts_with(&q) || q.starts_with(t.as_str()))
        .filter(|t| t.len() > 1 || q.len() == 1)
        .collect();
    v.sort_by(|a, b| shared(b).cmp(&shared(a)).then_with(|| a.cmp(b)));
    v.into_iter().take(MAX_SUGGESTIONS).cloned().collect()
}

/// Parse SEC's `company_tickers.json` (an object of `{cik_str, ticker, title}`).
pub fn parse_company_tickers(json: &str) -> Result<TickerMap, EdgarError> {
    #[derive(Deserialize)]
    struct Row {
        cik_str: Value,
        ticker: String,
        #[serde(default)]
        title: Option<String>,
    }
    let rows: HashMap<String, Row> = serde_json::from_str(json).map_err(parse_err)?;
    let mut map = HashMap::with_capacity(rows.len());
    let mut titles = HashMap::new();
    // Numeric row keys ("0", "1", ...) give SEC's ranking order; keep the first
    // row if a ticker is duplicated.
    let mut ordered: Vec<(&String, &Row)> = rows.iter().collect();
    ordered.sort_by_key(|(k, _)| (k.len(), k.as_str()));
    for (_, row) in ordered {
        let cik = cik_from_value(&row.cik_str)?;
        let ticker = normalize_ticker(&row.ticker);
        if let Some(title) = &row.title {
            titles
                .entry(ticker.clone())
                .or_insert_with(|| title.clone());
        }
        map.entry(ticker).or_insert(cik);
    }
    Ok(TickerMap { map, titles })
}

// ─── Company facts ────────────────────────────────────────────────────────────

/// One reported XBRL value.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fact {
    /// Reported value.
    pub val: f64,
    /// Period start (absent for instant concepts such as balance-sheet items).
    pub start: Option<NaiveDate>,
    /// Period end (the instant for instant concepts).
    pub end: NaiveDate,
    /// Fiscal year of the filing that reported it.
    pub fy: Option<i32>,
    /// Fiscal period of the filing: `FY`, `Q1`..`Q4`.
    pub fp: Option<String>,
    /// Form that reported it, e.g. `10-K`, `10-Q/A`.
    pub form: String,
    /// Date the filing was filed with SEC (the point-in-time key).
    pub filed: NaiveDate,
    /// Accession number of the filing.
    pub accn: String,
    /// SEC calendar frame, e.g. `CY2023Q1`; only on the canonical fact.
    pub frame: Option<String>,
}

impl Fact {
    fn is_amendment(&self) -> bool {
        self.form.ends_with("/A")
    }

    /// Total order used to pick the winning fact: later filing, then an
    /// amendment (`/A`, usually a correction) over the original, then higher
    /// accession number, then the later `start` (`None` lowest). File order
    /// never matters.
    fn precedence(&self) -> (NaiveDate, bool, &str, Option<NaiveDate>) {
        (
            self.filed,
            self.is_amendment(),
            self.accn.as_str(),
            self.start,
        )
    }

    /// Length of the reporting period in days, `None` for instants.
    fn duration_days(&self) -> Option<i64> {
        self.start.map(|s| (self.end - s).num_days())
    }
}

/// Reporting period selector for [`CompanyFacts::value_as_of`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    /// Point in time (balance-sheet items): matches facts with no `start` and this `end`.
    Instant(NaiveDate),
    /// Span: matches facts with exactly this `start` and `end`.
    Duration {
        /// First day of the period.
        start: NaiveDate,
        /// Last day of the period.
        end: NaiveDate,
    },
}

impl Period {
    fn matches(self, f: &Fact) -> bool {
        match self {
            Period::Instant(end) => f.start.is_none() && f.end == end,
            Period::Duration { start, end } => f.start == Some(start) && f.end == end,
        }
    }
}

/// Granularity for [`CompanyFacts::series`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Periodicity {
    /// Fiscal-year values from `10-K` / `10-K/A`.
    Annual,
    /// Fiscal Q1-Q3 values from `10-Q` / `10-Q/A` (Q4 is only reported in the 10-K).
    Quarterly,
}

type ConceptUnits = HashMap<String, Vec<Fact>>;

/// Parsed `companyfacts` document.
#[derive(Debug, Clone)]
pub struct CompanyFacts {
    /// Company CIK.
    pub cik: Cik,
    /// Registrant name.
    pub entity_name: String,
    /// taxonomy -> concept -> unit -> facts (file order preserved per unit).
    facts: HashMap<String, HashMap<String, ConceptUnits>>,
}

impl CompanyFacts {
    /// All facts for a taxonomy/concept/unit in file order (empty when unknown).
    pub fn facts(&self, taxonomy: &str, concept: &str, unit: &str) -> &[Fact] {
        self.facts
            .get(taxonomy)
            .and_then(|c| c.get(concept))
            .and_then(|u| u.get(unit))
            .map_or(&[], Vec::as_slice)
    }

    /// Taxonomy names present (sorted).
    pub fn taxonomies(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.facts.keys().map(String::as_str).collect();
        v.sort_unstable();
        v
    }

    /// Concept names in a taxonomy (sorted; empty if the taxonomy is unknown).
    pub fn concepts(&self, taxonomy: &str) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .facts
            .get(taxonomy)
            .map(|c| c.keys().map(String::as_str).collect())
            .unwrap_or_default();
        v.sort_unstable();
        v
    }

    /// Units a concept is reported in (sorted; empty if the concept is unknown).
    pub fn units(&self, taxonomy: &str, concept: &str) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .facts
            .get(taxonomy)
            .and_then(|c| c.get(concept))
            .map(|u| u.keys().map(String::as_str).collect())
            .unwrap_or_default();
        v.sort_unstable();
        v
    }

    /// Distinct forms (sorted) that reported a concept/unit, on any date.
    pub fn forms(&self, taxonomy: &str, concept: &str, unit: &str) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .facts(taxonomy, concept, unit)
            .iter()
            .map(|f| f.form.as_str())
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Latest `filed` date <= `as_of` over every 10-K/10-Q (and amendment) fact of
    /// the company: a proxy for when the filer last reported periodic results.
    pub fn latest_periodic_filing(&self, as_of: NaiveDate) -> Option<NaiveDate> {
        self.facts
            .values()
            .flat_map(|concepts| concepts.values())
            .flat_map(|units| units.values())
            .flatten()
            .filter(|f| f.filed <= as_of && is_periodic_form(&f.form))
            .map(|f| f.filed)
            .max()
    }

    /// The value for `period` as known on `as_of`.
    ///
    /// `as_of` is inclusive: a fact filed on `as_of` is visible. For
    /// close-to-close backtests that must not see a filing made after the
    /// market close, pass the previous trading day. The winner is the latest
    /// filing; on the same day an amendment (`/A`) beats the original, then
    /// the higher accession number, then the later `start`.
    pub fn value_as_of(
        &self,
        taxonomy: &str,
        concept: &str,
        unit: &str,
        period: Period,
        as_of: NaiveDate,
    ) -> Option<&Fact> {
        self.facts(taxonomy, concept, unit)
            .iter()
            .filter(|f| f.filed <= as_of && period.matches(f))
            .max_by(|a, b| a.precedence().cmp(&b.precedence()))
    }

    /// Point-in-time time series: one fact per period end, ascending by `end`.
    ///
    /// `as_of` is inclusive of the filing date (see [`Self::value_as_of`]).
    /// Annual keeps `10-K`/`10-K/A` facts with `fp == "FY"` whose span is
    /// 335..=395 days; quarterly keeps `10-Q`/`10-Q/A` facts with `fp` in
    /// `Q1..Q3` and a span of 61..=121 days (so year-to-date and Q4 slices are
    /// dropped). Instant facts (no `start`) skip the span check; note that
    /// quarterly instants may therefore include prior fiscal-year-end points
    /// reported as comparatives in a 10-Q. Later filings (restatements)
    /// supersede earlier ones, using the same winner order as `value_as_of`.
    pub fn series(
        &self,
        taxonomy: &str,
        concept: &str,
        unit: &str,
        periodicity: Periodicity,
        as_of: NaiveDate,
    ) -> Vec<&Fact> {
        let mut by_end: BTreeMap<NaiveDate, &Fact> = BTreeMap::new();
        for f in self
            .facts(taxonomy, concept, unit)
            .iter()
            .filter(|f| f.filed <= as_of && matches_periodicity(f, periodicity))
        {
            by_end
                .entry(f.end)
                .and_modify(|cur| {
                    if f.precedence() > cur.precedence() {
                        *cur = f;
                    }
                })
                .or_insert(f);
        }
        by_end.into_values().collect()
    }
}

fn is_periodic_form(form: &str) -> bool {
    matches!(form, "10-K" | "10-K/A" | "10-Q" | "10-Q/A")
}

fn matches_periodicity(f: &Fact, p: Periodicity) -> bool {
    let fp = f.fp.as_deref();
    let (form_ok, days) = match p {
        Periodicity::Annual => (
            matches!(f.form.as_str(), "10-K" | "10-K/A") && fp == Some("FY"),
            335..=395,
        ),
        Periodicity::Quarterly => (
            matches!(f.form.as_str(), "10-Q" | "10-Q/A") && matches!(fp, Some("Q1" | "Q2" | "Q3")),
            61..=121,
        ),
    };
    form_ok && f.duration_days().is_none_or(|d| days.contains(&d))
}

/// Parse SEC's `api/xbrl/companyfacts/CIK##########.json`.
pub fn parse_company_facts(json: &str) -> Result<CompanyFacts, EdgarError> {
    #[derive(Deserialize)]
    struct RawFact {
        val: f64,
        start: Option<String>,
        end: String,
        fy: Option<i32>,
        fp: Option<String>,
        form: String,
        filed: String,
        accn: String,
        frame: Option<String>,
    }
    #[derive(Deserialize)]
    struct RawConcept {
        units: HashMap<String, Vec<RawFact>>,
    }
    #[derive(Deserialize)]
    struct Doc {
        cik: Value,
        #[serde(rename = "entityName")]
        entity_name: String,
        facts: HashMap<String, HashMap<String, RawConcept>>,
    }

    let doc: Doc = serde_json::from_str(json).map_err(parse_err)?;
    let cik = cik_from_value(&doc.cik)?;
    let mut facts: HashMap<String, HashMap<String, ConceptUnits>> =
        HashMap::with_capacity(doc.facts.len());
    for (taxonomy, concepts) in doc.facts {
        let mut out_concepts = HashMap::with_capacity(concepts.len());
        for (concept, raw) in concepts {
            let mut units = HashMap::with_capacity(raw.units.len());
            for (unit, rows) in raw.units {
                let parsed = rows
                    .into_iter()
                    .map(|r| {
                        Ok(Fact {
                            val: r.val,
                            start: r
                                .start
                                .as_deref()
                                .filter(|s| !s.is_empty())
                                .map(|s| parse_date(s, "start"))
                                .transpose()?,
                            end: parse_date(&r.end, "end")?,
                            fy: r.fy,
                            fp: r.fp,
                            form: r.form,
                            filed: parse_date(&r.filed, "filed")?,
                            accn: r.accn,
                            frame: r.frame,
                        })
                    })
                    .collect::<Result<Vec<_>, EdgarError>>()?;
                units.insert(unit, parsed);
            }
            out_concepts.insert(concept, units);
        }
        facts.insert(taxonomy, out_concepts);
    }
    Ok(CompanyFacts {
        cik,
        entity_name: doc.entity_name,
        facts,
    })
}

// ─── Submissions ──────────────────────────────────────────────────────────────

/// One entry of a company's filing index.
#[derive(Debug, Clone, PartialEq)]
pub struct Filing {
    /// e.g. `0000320193-24-000123`.
    pub accession_number: String,
    /// Date filed with SEC.
    pub filing_date: NaiveDate,
    /// Period of report, when SEC provides one.
    pub report_date: Option<NaiveDate>,
    /// Form type, e.g. `8-K`.
    pub form: String,
    /// Primary document file name.
    pub primary_document: String,
    /// 8-K item numbers (e.g. `2.02`); empty for other forms.
    pub items: Vec<String>,
}

impl Filing {
    /// URL of the primary document on `www.sec.gov/Archives`.
    pub fn document_url(&self, cik: &Cik) -> String {
        format!(
            "https://www.sec.gov/Archives/edgar/data/{}/{}/{}",
            cik.numeric(),
            self.accession_number.replace('-', ""),
            self.primary_document
        )
    }

    /// True if `item` is listed exactly (`"2.02"` does not match `"2.0"`).
    pub fn has_item(&self, item: &str) -> bool {
        self.items.iter().any(|i| i == item)
    }
}

/// Columnar filing arrays, as in `filings.recent` or at the top level of an older page.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Columns {
    accession_number: Vec<String>,
    filing_date: Vec<String>,
    report_date: Vec<String>,
    form: Vec<String>,
    items: Vec<String>,
    primary_document: Vec<String>,
}

/// Parse a submissions document into rows (file order, newest first).
///
/// Accepts the full `submissions/CIK##########.json` (uses `filings.recent`)
/// and the older-page documents (`CIK##########-submissions-NNN.json`) whose
/// columns are at the top level.
pub fn parse_submissions(json: &str) -> Result<Vec<Filing>, EdgarError> {
    let doc: Value = serde_json::from_str(json).map_err(parse_err)?;
    let cols = if let Some(filings) = doc.get("filings") {
        filings
            .get("recent")
            .ok_or_else(|| EdgarError::Parse("missing filings.recent".into()))?
    } else if doc.get("accessionNumber").is_some() {
        &doc
    } else {
        return Err(EdgarError::Parse("missing filings.recent".into()));
    };
    let r = Columns::deserialize(cols).map_err(parse_err)?;
    let n = r.accession_number.len();
    if [
        r.filing_date.len(),
        r.report_date.len(),
        r.form.len(),
        r.items.len(),
        r.primary_document.len(),
    ]
    .iter()
    .any(|&l| l != n)
    {
        return Err(EdgarError::Parse(
            "submissions columns have differing lengths".into(),
        ));
    }

    let mut out = Vec::with_capacity(n);
    let rows = r
        .accession_number
        .into_iter()
        .zip(r.filing_date)
        .zip(r.report_date)
        .zip(r.form)
        .zip(r.items)
        .zip(r.primary_document);
    for (((((accession_number, filing_date), report_date), form), items), primary_document) in rows
    {
        out.push(Filing {
            accession_number,
            filing_date: parse_date(&filing_date, "filing")?,
            report_date: if report_date.trim().is_empty() {
                None
            } else {
                Some(parse_date(&report_date, "report")?)
            },
            form,
            primary_document,
            items: items
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
        });
    }
    Ok(out)
}

/// An older page of a company's filing index, listed in `filings.files[]`.
#[derive(Debug, Clone, PartialEq)]
pub struct SubmissionPage {
    /// File name under `/submissions/`, e.g. `CIK0000320193-submissions-001.json`.
    pub name: String,
    /// Number of filings on the page.
    pub filing_count: u64,
    /// Oldest filing date on the page.
    pub filing_from: NaiveDate,
    /// Newest filing date on the page.
    pub filing_to: NaiveDate,
}

/// Parse the `filings.files[]` page list of a submissions document (empty if absent).
pub fn parse_submissions_files(json: &str) -> Result<Vec<SubmissionPage>, EdgarError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Raw {
        name: String,
        filing_count: u64,
        filing_from: String,
        filing_to: String,
    }
    let doc: Value = serde_json::from_str(json).map_err(parse_err)?;
    let Some(files) = doc.get("filings").and_then(|f| f.get("files")) else {
        return Ok(Vec::new());
    };
    let raw = Vec::<Raw>::deserialize(files).map_err(parse_err)?;
    raw.into_iter()
        .map(|r| {
            Ok(SubmissionPage {
                name: r.name,
                filing_count: r.filing_count,
                filing_from: parse_date(&r.filing_from, "filingFrom")?,
                filing_to: parse_date(&r.filing_to, "filingTo")?,
            })
        })
        .collect()
}

// ─── Config ───────────────────────────────────────────────────────────────────

/// Settings for [`EdgarClient`].
#[derive(Debug, Clone)]
pub struct EdgarConfig {
    /// `"<app or name> <contact email>"`, required by SEC.
    pub user_agent: String,
    /// Request rate cap (SEC allows 10/s; default 8).
    pub requests_per_second: u32,
    /// Directory for the on-disk response cache.
    pub cache_dir: PathBuf,
    /// A cache entry is fresh while its age is below this many hours. `0` disables
    /// cache reads (every call refetches); an expired entry is still used as a
    /// fallback when SEC is unreachable or returns 403/429/5xx.
    pub cache_ttl_hours: u64,
    /// Retries after the first attempt for 429/5xx/transport errors.
    pub max_retries: u32,
    /// Base for exponential backoff when the server gives no `Retry-After`.
    pub retry_base_delay_ms: u64,
}

impl EdgarConfig {
    /// Config with defaults: 8 req/s, 24h cache, 3 retries, 500ms backoff base.
    /// Does not validate; see [`EdgarConfig::validate`].
    pub fn new(user_agent: impl Into<String>, cache_dir: impl Into<PathBuf>) -> Self {
        Self {
            user_agent: user_agent.into(),
            requests_per_second: 8,
            cache_dir: cache_dir.into(),
            cache_ttl_hours: 24,
            max_retries: 3,
            retry_base_delay_ms: 500,
        }
    }

    /// Check the User-Agent (non-blank, contains a plausible email) and rate.
    pub fn validate(&self) -> Result<(), EdgarError> {
        if self.user_agent.trim().is_empty() {
            return Err(EdgarError::InvalidUserAgent(format!(
                "none configured. {USER_AGENT_HELP}"
            )));
        }
        let has_email = self.user_agent.split_whitespace().any(|tok| {
            tok.split_once('@')
                .is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty())
        });
        if !has_email {
            return Err(EdgarError::InvalidUserAgent(format!(
                "{:?} has no contact email. {USER_AGENT_HELP}",
                self.user_agent
            )));
        }
        if self.requests_per_second == 0 {
            return Err(EdgarError::InvalidConfig(
                "requests_per_second must be at least 1".into(),
            ));
        }
        Ok(())
    }

    /// Build a validated config from the config-file and env User-Agent.
    /// A non-blank env value wins over the config value.
    pub fn resolve(
        config_user_agent: Option<&str>,
        env_user_agent: Option<&str>,
        cache_dir: PathBuf,
    ) -> Result<Self, EdgarError> {
        fn non_blank(s: Option<&str>) -> Option<&str> {
            s.map(str::trim).filter(|s| !s.is_empty())
        }
        let ua = non_blank(env_user_agent)
            .or_else(|| non_blank(config_user_agent))
            .unwrap_or("");
        let cfg = Self::new(ua, cache_dir);
        cfg.validate()?;
        Ok(cfg)
    }
}

/// Result of [`EdgarClient::filings_report`].
#[derive(Debug, Clone, Default)]
pub struct FilingsReport {
    /// Matching filings, newest first.
    pub filings: Vec<Filing>,
    /// Human-readable notes about older pages that were skipped (bad name or fetch error).
    pub warnings: Vec<String>,
    /// True if SEC lists older pages beyond `recent` (fetched only when `since` is given).
    pub older_pages_exist: bool,
    /// Oldest filing date in the `recent` list, if any.
    pub oldest_recent: Option<NaiveDate>,
}

/// Parse `recent` rows and the older-page list from one submissions body.
fn parse_submissions_doc(json: &str) -> Result<(Vec<Filing>, Vec<SubmissionPage>), EdgarError> {
    Ok((parse_submissions(json)?, parse_submissions_files(json)?))
}

/// Only `CIK##########-submissions-###.json` is ever requested or cached: the
/// name comes from a remote document and is used in a URL and a file path.
fn is_valid_page_name(name: &str) -> bool {
    // Byte-wise on purpose: names are remote input and may contain multibyte characters.
    let b = name.as_bytes();
    b.len() == 34
        && b.starts_with(b"CIK")
        && b[3..13].iter().all(u8::is_ascii_digit)
        && &b[13..26] == b"-submissions-"
        && b[26..29].iter().all(u8::is_ascii_digit)
        && &b[29..] == b".json"
}

// ─── Client ───────────────────────────────────────────────────────────────────

type Limiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock>;

/// Rate-limited, caching SEC EDGAR HTTP client.
pub struct EdgarClient {
    cfg: EdgarConfig,
    http: reqwest::Client,
    limiter: Limiter,
    data_base: String,
    pub(super) www_base: String,
}

impl fmt::Debug for EdgarClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EdgarClient")
            .field("cfg", &self.cfg)
            .field("data_base", &self.data_base)
            .field("www_base", &self.www_base)
            .finish_non_exhaustive()
    }
}

/// Directory-safe `host_port` namespace for a base URL, so clients pointed at
/// different hosts never share cache entries.
pub(super) fn cache_namespace(base: &str) -> String {
    let raw = url::Url::parse(base)
        .ok()
        .and_then(|u| {
            u.host_str()
                .map(|h| u.port().map_or(h.to_string(), |p| format!("{h}_{p}")))
        })
        .unwrap_or_else(|| base.to_string());
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl EdgarClient {
    /// Validate `cfg` and build a client against the real SEC hosts.
    pub fn new(cfg: EdgarConfig) -> Result<Self, EdgarError> {
        cfg.validate()?;
        // gzip feature: reqwest sends `Accept-Encoding: gzip` and decodes for us.
        let http = reqwest::Client::builder()
            .user_agent(cfg.user_agent.trim())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| {
                EdgarError::InvalidUserAgent(format!("cannot be used as an HTTP header: {e}"))
            })?;
        let rps = std::num::NonZeroU32::new(cfg.requests_per_second)
            .ok_or_else(|| EdgarError::InvalidConfig("requests_per_second must be >= 1".into()))?;
        // Burst of 1 spaces requests evenly instead of firing `rps` at once.
        let limiter =
            RateLimiter::direct(Quota::per_second(rps).allow_burst(std::num::NonZeroU32::MIN));
        Ok(Self {
            cfg,
            http,
            limiter,
            data_base: DEFAULT_DATA_BASE.into(),
            www_base: DEFAULT_WWW_BASE.into(),
        })
    }

    /// Override the hosts (tests, mirrors). Trailing slashes are ignored.
    pub fn with_base_urls(
        mut self,
        data_base: impl Into<String>,
        www_base: impl Into<String>,
    ) -> Self {
        self.data_base = data_base.into().trim_end_matches('/').to_string();
        self.www_base = www_base.into().trim_end_matches('/').to_string();
        self
    }

    /// Resolve a ticker (case-insensitive) to its CIK.
    pub async fn cik_for(&self, ticker: &str) -> Result<Cik, EdgarError> {
        let map = self.company_tickers().await?;
        map.get(ticker)
            .cloned()
            .ok_or_else(|| EdgarError::UnknownTicker {
                ticker: ticker.to_string(),
                suggestions: map.suggest(ticker),
            })
    }

    /// The cached `company_tickers.json` map (ticker -> CIK and company title).
    pub(super) async fn company_tickers(&self) -> Result<TickerMap, EdgarError> {
        let url = format!("{}/files/company_tickers.json", self.www_base);
        let key = format!("{}/{TICKERS_CACHE_KEY}", cache_namespace(&self.www_base));
        self.get_cached(&url, &key, parse_company_tickers).await
    }

    /// XBRL company facts for a ticker or (all-digit) CIK.
    pub async fn company_facts(&self, ticker_or_cik: &str) -> Result<CompanyFacts, EdgarError> {
        let cik = self.resolve_cik(ticker_or_cik).await?;
        let url = format!("{}/api/xbrl/companyfacts/CIK{cik}.json", self.data_base);
        let key = self.data_key(&format!("companyfacts_CIK{cik}.json"));
        self.get_cached(&url, &key, parse_company_facts).await
    }

    /// The `recent` filings (SEC order, newest first) for a ticker or CIK.
    /// Never fetches older pages; see [`EdgarClient::older_pages`] and [`EdgarClient::filings`].
    pub async fn submissions(&self, ticker_or_cik: &str) -> Result<Vec<Filing>, EdgarError> {
        let cik = self.resolve_cik(ticker_or_cik).await?;
        let url = format!("{}/submissions/CIK{cik}.json", self.data_base);
        let key = self.data_key(&format!("submissions_CIK{cik}.json"));
        self.get_cached(&url, &key, parse_submissions).await
    }

    /// Filings filtered by exact `form`, 8-K `item`, and inclusive `since` filing date.
    ///
    /// Convenience wrapper over [`EdgarClient::filings_report`] that drops the
    /// report details; skipped-page warnings are logged via `tracing`.
    pub async fn filings(
        &self,
        ticker_or_cik: &str,
        form: Option<&str>,
        item: Option<&str>,
        since: Option<NaiveDate>,
    ) -> Result<Vec<Filing>, EdgarError> {
        let report = self
            .filings_report(ticker_or_cik, form, item, since)
            .await?;
        for w in &report.warnings {
            tracing::warn!("{w}");
        }
        Ok(report.filings)
    }

    /// Like [`EdgarClient::filings`], but also reports what was skipped and
    /// whether the result may be incomplete.
    ///
    /// SEC's index only embeds the ~1000 most recent filings (`recent`). With
    /// `since`, every older page that reaches back to `since` is fetched too
    /// (pages entirely older are skipped); without `since`, only `recent` is
    /// searched. The main submissions document is requested once per call.
    /// A failing main document is an error; a failing or invalidly named older
    /// page is skipped and described in [`FilingsReport::warnings`]. Results
    /// stay in SEC order (newest first).
    pub async fn filings_report(
        &self,
        ticker_or_cik: &str,
        form: Option<&str>,
        item: Option<&str>,
        since: Option<NaiveDate>,
    ) -> Result<FilingsReport, EdgarError> {
        let cik = self.resolve_cik(ticker_or_cik).await?;
        let url = format!("{}/submissions/CIK{cik}.json", self.data_base);
        let key = self.data_key(&format!("submissions_CIK{cik}.json"));
        let (mut all, pages) = self.get_cached(&url, &key, parse_submissions_doc).await?;
        let mut report = FilingsReport {
            older_pages_exist: !pages.is_empty(),
            oldest_recent: all.iter().map(|f| f.filing_date).min(),
            ..FilingsReport::default()
        };
        if let Some(since) = since {
            let wanted: Vec<SubmissionPage> =
                pages.into_iter().filter(|p| p.filing_to >= since).collect();
            for (i, page) in wanted.iter().enumerate() {
                if !is_valid_page_name(&page.name) {
                    report.warnings.push(format!(
                        "skipping submissions page with invalid name {:?}",
                        page.name
                    ));
                    continue;
                }
                let url = format!("{}/submissions/{}", self.data_base, page.name);
                let key = self.data_key(&page.name);
                match self.get_cached(&url, &key, parse_submissions).await {
                    Ok(rows) => all.extend(rows),
                    Err(EdgarError::Forbidden { .. }) => {
                        report.warnings.push(format!(
                            "SEC returned 403 for {}; remaining {} older pages skipped \
                             (wait about 10 minutes and retry)",
                            page.name,
                            wanted.len() - i
                        ));
                        break;
                    }
                    Err(e) => report.warnings.push(format!(
                        "skipping older submissions page {}: {e}",
                        page.name
                    )),
                }
            }
            // Stable: keeps SEC order within a day, restores newest-first across pages.
            all.sort_by_key(|a| std::cmp::Reverse(a.filing_date));
        }
        all.retain(|f| {
            form.is_none_or(|x| f.form == x)
                && item.is_none_or(|x| f.has_item(x))
                && since.is_none_or(|d| f.filing_date >= d)
        });
        report.filings = all;
        Ok(report)
    }

    /// Resolve a ticker or all-digit CIK string to a [`Cik`] (digits skip the ticker lookup).
    pub async fn resolve_cik(&self, id: &str) -> Result<Cik, EdgarError> {
        let id = id.trim();
        if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
            Cik::parse(id)
        } else {
            self.cik_for(id).await
        }
    }

    fn data_key(&self, name: &str) -> String {
        format!("{}/{name}", cache_namespace(&self.data_base))
    }

    /// Fetch `url` through the disk cache and parse it. Only bodies that parse
    /// successfully are written to the cache; error responses never are. If the
    /// entry is expired and the refetch fails with 403/429/5xx or a network
    /// error, the stale entry is returned instead.
    async fn get_cached<T>(
        &self,
        url: &str,
        key: &str,
        parse: fn(&str) -> Result<T, EdgarError>,
    ) -> Result<T, EdgarError>
    where
        T: Send + 'static,
    {
        self.get_cached_with(url, key, parse, false).await
    }

    /// [`EdgarClient::get_cached`] with an `immutable` switch: an immutable
    /// entry (e.g. an N-PORT document, fixed per accession) never expires, even
    /// with `cache_ttl_hours = 0`. Immutable entries are large archive documents,
    /// so they are also fetched with the longer [`DOCUMENT_TIMEOUT`].
    pub(super) async fn get_cached_with<T>(
        &self,
        url: &str,
        key: &str,
        parse: fn(&str) -> Result<T, EdgarError>,
        immutable: bool,
    ) -> Result<T, EdgarError>
    where
        T: Send + 'static,
    {
        let path = self.cfg.cache_dir.join(key);
        let ttl_hours = self.cfg.cache_ttl_hours;
        // Disk reads and parsing of (possibly multi-MB) documents run off the async runtime.
        let (cached, hit) = {
            let path = path.clone();
            blocking(move || {
                let cached = std::fs::read_to_string(&path).ok();
                let mut hit = None;
                if immutable || cache_is_fresh(&path, ttl_hours) {
                    if let Some(body) = &cached {
                        match parse(body) {
                            Ok(v) => hit = Some(v),
                            Err(e) => {
                                tracing::warn!(path = %path.display(), error = %e, "Ignoring corrupt EDGAR cache entry")
                            }
                        }
                    }
                }
                (cached, hit)
            })
            .await?
        };
        if let Some(v) = hit {
            return Ok(v);
        }
        let timeout = if immutable {
            DOCUMENT_TIMEOUT
        } else {
            REQUEST_TIMEOUT
        };
        match self.fetch(url, timeout).await {
            Ok(body) => {
                let cache_dir = self.cfg.cache_dir.clone();
                blocking(move || {
                    let parsed = parse(&body)?;
                    write_cache_file(&cache_dir, &path, &body);
                    Ok(parsed)
                })
                .await?
            }
            Err(e) if is_transient(&e) => {
                let stale = match cached {
                    Some(body) => blocking(move || parse(&body).ok()).await?,
                    None => None,
                };
                if let Some(v) = stale {
                    tracing::warn!(%url, error = %e, "EDGAR unavailable, using stale cached copy");
                    Ok(v)
                } else {
                    Err(e)
                }
            }
            Err(e) => Err(e),
        }
    }

    /// GET with rate limiting and retry. 429/5xx and transport errors are
    /// retried up to `max_retries` times, honouring a capped `Retry-After`
    /// and otherwise backing off exponentially (with jitter) from
    /// `retry_base_delay_ms`. 403, other 4xx and request-building errors
    /// (e.g. a base URL without a scheme) are not retried.
    async fn fetch(&self, url: &str, timeout: Duration) -> Result<String, EdgarError> {
        let mut attempt: u32 = 0;
        loop {
            self.limiter.until_ready().await;
            let outcome: Result<String, (EdgarError, Option<Duration>)> = match self
                .http
                .get(url)
                .timeout(timeout)
                .send()
                .await
            {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        match resp.text().await {
                            Ok(body) => return Ok(body),
                            Err(e) => Err((EdgarError::Network(e.to_string()), None)),
                        }
                    } else if status.as_u16() == 404 {
                        return Err(EdgarError::NotFound { url: url.into() });
                    } else if status.as_u16() == 403 {
                        return Err(EdgarError::Forbidden { url: url.into() });
                    } else {
                        let err = EdgarError::HttpStatus {
                            status: status.as_u16(),
                            url: url.into(),
                        };
                        if status.as_u16() == 429 || status.is_server_error() {
                            Err((err, retry_after(&resp)))
                        } else {
                            return Err(err);
                        }
                    }
                }
                Err(e) if e.is_builder() => {
                    return Err(EdgarError::InvalidConfig(format!(
                            "cannot request {url}: {e} (base URLs need a scheme, e.g. https://data.sec.gov)"
                        )));
                }
                Err(e) => Err((EdgarError::Network(e.to_string()), None)),
            };
            let Err((err, server_delay)) = outcome else {
                unreachable!("success paths return above");
            };
            if attempt >= self.cfg.max_retries {
                return Err(err);
            }
            let delay = server_delay.unwrap_or_else(|| self.backoff(attempt));
            if attempt == 0 {
                tracing::warn!(%url, ?delay, error = %err, "EDGAR request failed, retrying");
            } else {
                tracing::debug!(%url, attempt, ?delay, error = %err, "Retrying EDGAR request");
            }
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    /// Exponential backoff with up to +25% jitter.
    fn backoff(&self, attempt: u32) -> Duration {
        use rand::Rng;
        let factor = 1u64 << attempt.min(16);
        let base = self.cfg.retry_base_delay_ms.saturating_mul(factor);
        let jitter = rand::thread_rng().gen_range(0..=base / 4);
        Duration::from_millis(base.saturating_add(jitter)).min(MAX_BACKOFF)
    }
}

/// Run blocking file or parsing work on the blocking pool.
async fn blocking<R: Send + 'static>(
    f: impl FnOnce() -> R + Send + 'static,
) -> Result<R, EdgarError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| EdgarError::Parse(format!("background task failed: {e}")))
}

/// True when the cache file is younger than `ttl_hours`.
fn cache_is_fresh(path: &Path, ttl_hours: u64) -> bool {
    let ttl = Duration::from_secs(ttl_hours.saturating_mul(3600));
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        // An mtime up to 5 minutes in the future (clock skew, coarse timestamps)
        // counts as age zero; further ahead is untrustworthy, so treat as stale.
        .and_then(|t| match t.elapsed() {
            Ok(age) => Some(age),
            Err(e) if e.duration() <= FUTURE_MTIME_TOLERANCE => Some(Duration::ZERO),
            Err(_) => None,
        })
        .is_some_and(|age| age < ttl)
}

/// Best effort: a failing cache must not fail the request. Each write goes
/// through its own temp file in the target directory, then an atomic rename.
fn write_cache_file(cache_dir: &Path, path: &Path, body: &str) {
    use std::io::Write;
    let result = (|| -> std::io::Result<()> {
        let dir = path.parent().unwrap_or(cache_dir);
        std::fs::create_dir_all(dir)?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        tmp.write_all(body.as_bytes())?;
        // On failure the temp file is removed when the error is dropped.
        tmp.persist(path).map_err(|e| e.error)?;
        Ok(())
    })();
    if let Err(e) = result {
        tracing::warn!(path = %path.display(), error = %e, "Failed to write EDGAR cache entry");
    }
}

/// Errors for which a stale cached copy is better than failing.
fn is_transient(e: &EdgarError) -> bool {
    match e {
        EdgarError::Forbidden { .. } | EdgarError::Network(_) => true,
        EdgarError::HttpStatus { status, .. } => *status == 429 || *status >= 500,
        _ => false,
    }
}

/// Integer-seconds `Retry-After`, capped. HTTP-date form is ignored (falls back to backoff).
fn retry_after(resp: &reqwest::Response) -> Option<Duration> {
    let secs = resp
        .headers()
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(Duration::from_secs(secs).min(MAX_RETRY_AFTER))
}
