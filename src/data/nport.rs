//! ETF overlap from SEC N-PORT filings: how much of one issuer an ETF holds.
//!
//! Free, key-less data from `www.sec.gov`, through the same rate-limited,
//! caching [`EdgarClient`] as the XBRL facts. The data path is:
//! fund ticker -> series ID (`company_tickers_mf.json`) -> filing list (Atom
//! feed of NPORT-P and NPORT-P/A filings) -> `primary_doc.xml` holdings.
//!
//! The module has three layers:
//! * pure parsers ([`parse_fund_tickers`], [`parse_series_feed`],
//!   [`parse_nport`]) with no I/O,
//! * issuer matching and arithmetic ([`IssuerMatcher`], [`exposure`]) plus
//!   point-in-time filing selection ([`select_original`],
//!   [`amendment_candidates`], [`same_period_amendment`]),
//! * [`EdgarClient`] additions ([`EdgarClient::fund_id`],
//!   [`EdgarClient::nport_as_of`], ...). N-PORT documents are immutable per
//!   accession, so their cache entries never expire.
//!
//! Unit investment trusts (e.g. SPY) file no N-PORT and are reported as
//! [`EdgarError::NotNportFiler`] rather than as an unknown ticker.

use std::cmp::Reverse;
use std::collections::HashMap;

use chrono::NaiveDate;
use roxmltree::{Document, Node};
use serde_json::Value;

use crate::data::edgar::{
    cache_namespace, cik_from_value, normalize_ticker, parse_date, parse_err, suggest_tickers, Cik,
    EdgarClient, EdgarError, TickerMap,
};

const FUND_TICKERS_CACHE_KEY: &str = "company_tickers_mf.json";

// ─── Fund ticker map ──────────────────────────────────────────────────────────

/// Fund identifiers from `company_tickers_mf.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FundId {
    /// Registrant CIK (the trust that files the N-PORT).
    pub cik: Cik,
    /// Series ID (`S000...`); one registrant can file many series.
    pub series_id: String,
    /// Share class ID (`C000...`) of the ticker.
    pub class_id: String,
}

/// Case-insensitive fund symbol -> [`FundId`].
#[derive(Debug, Clone, Default)]
pub struct FundTickerMap {
    map: HashMap<String, FundId>,
}

impl FundTickerMap {
    /// Fund identifiers for `ticker`, ignoring case (same normalisation as
    /// [`TickerMap::get`](crate::data::edgar::TickerMap::get)).
    pub fn get(&self, ticker: &str) -> Option<&FundId> {
        self.map.get(&normalize_ticker(ticker))
    }

    /// Number of fund symbols.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True when there are no fund symbols.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Up to five symbols close to `query`; same ranking as
    /// [`TickerMap::suggest`](crate::data::edgar::TickerMap::suggest).
    pub fn suggest(&self, query: &str) -> Vec<String> {
        suggest_tickers(self.map.keys(), query)
    }
}

/// Parse SEC's `company_tickers_mf.json`
/// (`{"fields":["cik","seriesId","classId","symbol"],"data":[[...]]}`).
///
/// Columns are located by name, so a reordered `fields` list still works.
/// Rows without a symbol are skipped; the first row wins for a duplicate symbol.
pub fn parse_fund_tickers(json: &str) -> Result<FundTickerMap, EdgarError> {
    let doc: Value = serde_json::from_str(json).map_err(parse_err)?;
    let fields = doc
        .get("fields")
        .and_then(Value::as_array)
        .ok_or_else(|| EdgarError::Parse("fund ticker map has no \"fields\" array".into()))?;
    let rows = doc
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| EdgarError::Parse("fund ticker map has no \"data\" array".into()))?;
    let column = |name: &str| {
        fields
            .iter()
            .position(|f| f.as_str() == Some(name))
            .ok_or_else(|| EdgarError::Parse(format!("fund ticker map has no {name:?} field")))
    };
    let (cik_i, series_i, class_i, symbol_i) = (
        column("cik")?,
        column("seriesId")?,
        column("classId")?,
        column("symbol")?,
    );
    let mut map = HashMap::with_capacity(rows.len());
    for row in rows {
        let cell = |i: usize| {
            row.get(i)
                .ok_or_else(|| EdgarError::Parse(format!("short fund ticker row {row}")))
        };
        let Some(symbol) = cell(symbol_i)?.as_str().filter(|s| !s.trim().is_empty()) else {
            continue;
        };
        let text = |i: usize| -> Result<String, EdgarError> {
            cell(i)?
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| EdgarError::Parse(format!("bad fund ticker row {row}")))
        };
        let id = FundId {
            cik: cik_from_value(cell(cik_i)?)?,
            series_id: text(series_i)?,
            class_id: text(class_i)?,
        };
        map.entry(normalize_ticker(symbol)).or_insert(id);
    }
    Ok(FundTickerMap { map })
}

// ─── XML helpers ──────────────────────────────────────────────────────────────

fn parse_xml(xml: &str) -> Result<Document<'_>, EdgarError> {
    Document::parse(xml).map_err(parse_err)
}

/// First child element with local name `name` (namespaces are ignored).
fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children()
        .find(|c| c.is_element() && c.tag_name().name() == name)
}

/// Trimmed, non-empty text of the child element `name`.
fn child_text<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(node, name)
        .and_then(|c| c.text())
        .map(str::trim)
        .filter(|t| !t.is_empty())
}

fn require_text<'a>(node: Node<'a, '_>, name: &str) -> Result<&'a str, EdgarError> {
    child_text(node, name)
        .ok_or_else(|| EdgarError::Parse(format!("N-PORT document has no <{name}>")))
}

fn parse_number(s: &str, what: &str) -> Result<f64, EdgarError> {
    s.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| EdgarError::Parse(format!("bad {what} number {s:?}")))
}

// ─── Series feed ──────────────────────────────────────────────────────────────

/// One N-PORT filing listed in a series feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NportFiling {
    /// Accession number with dashes (`0001410368-26-089594`).
    pub accession: String,
    /// Date the filing was submitted.
    pub filing_date: NaiveDate,
    /// `NPORT-P` (original) or `NPORT-P/A` (amendment).
    pub form: String,
    /// Registrant CIK from the filing path (the `Archives` URLs need it).
    pub cik: Cik,
}

/// Parse the Atom feed from `browse-edgar?...&output=atom`, keeping feed order
/// (newest first). A feed without entries is `Ok(empty)`.
pub fn parse_series_feed(atom: &str) -> Result<Vec<NportFiling>, EdgarError> {
    let doc = parse_xml(atom)?;
    let root = doc.root_element();
    if root.tag_name().name() != "feed" {
        return Err(EdgarError::Parse(format!(
            "not an Atom feed (root element <{}>)",
            root.tag_name().name()
        )));
    }
    let mut out = Vec::new();
    for entry in root
        .children()
        .filter(|c| c.is_element() && c.tag_name().name() == "entry")
    {
        let field = |name: &str| {
            entry
                .descendants()
                .find(|n| n.is_element() && n.tag_name().name() == name)
                .and_then(|n| n.text())
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .ok_or_else(|| EdgarError::Parse(format!("feed entry has no <{name}>")))
        };
        let accession = field("accession-number")?;
        if !is_valid_accession(accession) {
            return Err(EdgarError::Parse(format!("bad accession {accession:?}")));
        }
        out.push(NportFiling {
            accession: accession.to_string(),
            filing_date: parse_date(field("filing-date")?, "filing")?,
            form: field("filing-type")?.to_string(),
            cik: cik_from_href(field("filing-href")?)?,
        });
    }
    Ok(out)
}

/// Exactly `\d{10}-\d{2}-\d{6}`, so an accession is safe in URLs and cache paths.
fn is_valid_accession(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    matches!(parts.as_slice(), [a, b, c]
        if [(a, 10), (b, 2), (c, 6)]
            .iter()
            .all(|(p, n)| p.len() == *n && p.bytes().all(|x| x.is_ascii_digit())))
}

/// CIK from `.../Archives/edgar/data/<cik>/...`.
fn cik_from_href(href: &str) -> Result<Cik, EdgarError> {
    const MARKER: &str = "/Archives/edgar/data/";
    let rest = href
        .split_once(MARKER)
        .map(|(_, rest)| rest)
        .ok_or_else(|| EdgarError::Parse(format!("filing href has no CIK: {href:?}")))?;
    let digits = rest.split('/').next().unwrap_or_default();
    Cik::parse(digits).map_err(parse_err)
}

// ─── N-PORT document ──────────────────────────────────────────────────────────

/// One position of the fund. Value and percent are as reported, so a short
/// position is negative in SEC data.
#[derive(Debug, Clone, PartialEq)]
pub struct Holding {
    /// Issuer name (`name`).
    pub name: String,
    /// Security description (`title`).
    pub title: Option<String>,
    /// CUSIP; `None` when absent or `N/A`.
    pub cusip: Option<String>,
    /// ISIN from `identifiers/isin`.
    pub isin: Option<String>,
    /// Ticker from `identifiers/ticker`.
    pub ticker: Option<String>,
    /// Issuer LEI; `None` when absent or `N/A`.
    pub lei: Option<String>,
    /// Position value in USD (`valUSD`).
    pub value_usd: f64,
    /// Percent of net assets (`pctVal`).
    pub pct: f64,
    /// Asset category (`EC` equity, `DBT` debt, `DE` derivative, ...).
    pub asset_cat: String,
    /// `payoffProfile` is `Short`.
    pub short: bool,
}

/// A parsed N-PORT filing.
#[derive(Debug, Clone, PartialEq)]
pub struct NportReport {
    /// Series ID the report belongs to.
    pub series_id: String,
    /// Fund (series) name.
    pub series_name: String,
    /// Registrant (trust) name.
    pub registrant: String,
    /// Date of the holdings (`repPdDate`, not the fiscal `repPdEnd`).
    pub period_end: NaiveDate,
    /// Net assets in USD.
    pub net_assets: f64,
    /// All positions in document order.
    pub holdings: Vec<Holding>,
}

/// Parse an N-PORT `primary_doc.xml` (local element names, any namespace).
///
/// Dates and numbers are required and a malformed one is an error, so a
/// damaged document is never read as a zero exposure.
pub fn parse_nport(xml: &str) -> Result<NportReport, EdgarError> {
    let doc = parse_xml(xml)?;
    let root = doc.root_element();
    if root.tag_name().name() != "edgarSubmission" {
        return Err(EdgarError::Parse(format!(
            "not an N-PORT document (root element <{}>)",
            root.tag_name().name()
        )));
    }
    let form_data = child(root, "formData")
        .ok_or_else(|| EdgarError::Parse("N-PORT document has no <formData>".into()))?;
    let gen = child(form_data, "genInfo")
        .ok_or_else(|| EdgarError::Parse("N-PORT document has no <genInfo>".into()))?;
    let fund = child(form_data, "fundInfo")
        .ok_or_else(|| EdgarError::Parse("N-PORT document has no <fundInfo>".into()))?;
    let holdings = match child(form_data, "invstOrSecs") {
        Some(list) => list
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "invstOrSec")
            .map(parse_holding)
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    Ok(NportReport {
        series_id: require_text(gen, "seriesId")?.to_string(),
        series_name: child_text(gen, "seriesName").unwrap_or_default().into(),
        registrant: child_text(gen, "regName").unwrap_or_default().into(),
        period_end: parse_date(require_text(gen, "repPdDate")?, "repPdDate")?,
        net_assets: parse_number(require_text(fund, "netAssets")?, "netAssets")?,
        holdings,
    })
}

fn parse_holding(node: Node<'_, '_>) -> Result<Holding, EdgarError> {
    let title = child_text(node, "title");
    let name = child_text(node, "name").or(title).unwrap_or_default();
    let identifier = |kind: &str| {
        child(node, "identifiers")
            .and_then(|ids| child(ids, kind))
            .and_then(|n| n.attribute("value"))
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
    };
    let id_text = |tag: &str| {
        child_text(node, tag)
            .filter(|v| !v.eq_ignore_ascii_case("N/A"))
            .map(str::to_string)
    };
    let number = |tag: &str| {
        child_text(node, tag)
            .ok_or_else(|| EdgarError::Parse(format!("holding {name:?} has no <{tag}>")))
            .and_then(|v| parse_number(v, tag))
    };
    Ok(Holding {
        name: name.to_string(),
        title: title.map(str::to_string),
        cusip: id_text("cusip"),
        isin: identifier("isin"),
        ticker: identifier("ticker"),
        lei: id_text("lei"),
        value_usd: number("valUSD")?,
        pct: number("pctVal")?,
        asset_cat: child_text(node, "assetCat").unwrap_or_default().into(),
        short: child_text(node, "payoffProfile").is_some_and(|p| p.eq_ignore_ascii_case("Short")),
    })
}

// ─── Issuer matching ──────────────────────────────────────────────────────────

/// Legal-form words and filler dropped from issuer names. `COM` covers
/// "Amazon.com" once the dot is gone.
const NAME_NOISE: &[&str] = &[
    "INC",
    "INCORPORATED",
    "CORP",
    "CORPORATION",
    "CO",
    "COM",
    "COMPANY",
    "LTD",
    "LIMITED",
    "LLC",
    "LP",
    "PLC",
    "SA",
    "NV",
    "AG",
    "SE",
    "THE",
    "ADR",
    "ADS",
    "SPONSORED",
    "SHARES",
];

/// ASCII spelling of a Latin letter with a diacritic (`é` -> `e`); other
/// characters map to the empty string. Covers Latin-1 Supplement and the
/// common Latin Extended-A letters, which is enough for issuer names.
fn fold_latin(c: char) -> &'static str {
    match c {
        'À'..='Å' | 'Ā' | 'Ă' | 'Ą' => "A",
        'à'..='å' | 'ā' | 'ă' | 'ą' => "a",
        'Æ' => "AE",
        'æ' => "ae",
        'Ç' | 'Ć' | 'Č' => "C",
        'ç' | 'ć' | 'č' => "c",
        'Ď' | 'Đ' => "D",
        'ď' | 'đ' => "d",
        'È'..='Ë' | 'Ē' | 'Ę' | 'Ě' => "E",
        'è'..='ë' | 'ē' | 'ę' | 'ě' => "e",
        'Ğ' => "G",
        'ğ' => "g",
        'Ì'..='Ï' | 'Ī' | 'İ' => "I",
        'ì'..='ï' | 'ī' | 'ı' => "i",
        'Ł' => "L",
        'ł' => "l",
        'Ñ' | 'Ń' | 'Ň' => "N",
        'ñ' | 'ń' | 'ň' => "n",
        'Ò'..='Ö' | 'Ø' | 'Ō' | 'Ő' => "O",
        'ò'..='ö' | 'ø' | 'ō' | 'ő' => "o",
        'Œ' => "OE",
        'œ' => "oe",
        'Ř' => "R",
        'ř' => "r",
        'Ś' | 'Š' | 'Ş' => "S",
        'ś' | 'š' | 'ş' => "s",
        'ß' => "ss",
        'Ť' | 'Ţ' => "T",
        'ť' | 'ţ' => "t",
        'Ù'..='Ü' | 'Ū' | 'Ů' | 'Ű' => "U",
        'ù'..='ü' | 'ū' | 'ů' | 'ű' => "u",
        'Ý' | 'Ÿ' => "Y",
        'ý' | 'ÿ' => "y",
        'Ź' | 'Ż' | 'Ž' => "Z",
        'ź' | 'ż' | 'ž' => "z",
        _ => "",
    }
}

/// Canonical issuer name for matching: upper-case, Latin accents folded,
/// punctuation replaced by spaces, runs of single letters joined (`S.A.` ->
/// `SA`), legal suffixes (`INC`, `CORP`, `SA`, ...), depositary wording (`ADR`,
/// `SPONSORED`), `SHARES`, `CAPITAL STOCK` and share-class tokens (`CLASS A`)
/// dropped. "Amazon.com, Inc." and "AMAZON COM INC" both give `AMAZON`, while
/// "Amazonas Energia SA" stays `AMAZONAS ENERGIA`.
pub fn normalize_issuer_name(name: &str) -> String {
    let spaced: String = name
        .chars()
        .flat_map(|c| {
            let folded = fold_latin(c);
            if !folded.is_empty() {
                folded.to_uppercase().chars().collect::<Vec<_>>()
            } else if c.is_alphanumeric() {
                c.to_uppercase().collect::<Vec<_>>()
            } else {
                vec![' ']
            }
        })
        .collect();
    // "S.A." and "SA" must agree: join consecutive one-letter tokens.
    let mut joined: Vec<String> = Vec::new();
    let mut prev_single = false;
    for t in spaced.split_whitespace() {
        let single = t.chars().count() == 1 && t.chars().all(char::is_alphabetic);
        match joined.last_mut() {
            Some(last) if single && prev_single => last.push_str(t),
            _ => joined.push(t.to_string()),
        }
        prev_single = single;
    }
    let tokens: Vec<&str> = joined.iter().map(String::as_str).collect();
    let is_class_label = |t: &str| t.chars().count() == 1 || matches!(t, "II" | "III" | "IV");
    let mut kept = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i];
        let next = tokens.get(i + 1).copied();
        if (matches!(t, "CLASS" | "CL" | "SERIES") && next.is_some_and(is_class_label))
            || (t == "CAPITAL" && next == Some("STOCK"))
        {
            i += 2;
        } else {
            if !NAME_NOISE.contains(&t) {
                kept.push(t);
            }
            i += 1;
        }
    }
    // A name made only of noise words keeps its words rather than becoming empty.
    if kept.is_empty() {
        tokens.join(" ")
    } else {
        kept.join(" ")
    }
}

/// Which identifier tied a holding to the issuer, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchRule {
    /// First 6 CUSIP characters (the issuer code) are equal, so share classes
    /// and the issuer's bonds match together.
    Cusip,
    /// US/CA ISIN that embeds the issuer's CUSIP issuer code.
    Isin,
    /// Same legal entity identifier.
    Lei,
    /// The holding's ticker identifier equals the symbol.
    Ticker,
    /// Normalised issuer names are equal.
    Name,
}

impl MatchRule {
    /// Lower-case label used in CLI output (`cusip`, `isin`, `lei`, `ticker`, `name`).
    pub fn as_str(self) -> &'static str {
        match self {
            MatchRule::Cusip => "cusip",
            MatchRule::Isin => "isin",
            MatchRule::Lei => "lei",
            MatchRule::Ticker => "ticker",
            MatchRule::Name => "name",
        }
    }
}

/// Decides whether a [`Holding`] is the issuer being looked up.
///
/// Built from the ticker plus EDGAR's company title; [`with_cusip`](Self::with_cusip)
/// and [`with_lei`](Self::with_lei) add stronger keys. Ticker and name matches
/// are exact (never prefixes), so "AMAZON" does not match "AMAZONAS".
#[derive(Debug, Clone)]
pub struct IssuerMatcher {
    ticker: String,
    name: Option<String>,
    cusip_issuer: Option<String>,
    lei: Option<String>,
}

impl IssuerMatcher {
    /// Matcher for `ticker`; `company_title` (EDGAR's name for the company)
    /// enables the name rule.
    pub fn new(ticker: &str, company_title: Option<&str>) -> Self {
        Self {
            ticker: normalize_ticker(ticker),
            name: company_title
                .map(normalize_issuer_name)
                .filter(|n| !n.is_empty()),
            cusip_issuer: None,
            lei: None,
        }
    }

    /// Add a CUSIP; only its first 6 characters (the issuer code) are used.
    /// A value shorter than 6 alphanumeric characters is ignored.
    pub fn with_cusip(mut self, cusip: &str) -> Self {
        let cusip = cusip.trim();
        self.cusip_issuer = cusip
            .get(..6)
            .filter(|c| c.bytes().all(|b| b.is_ascii_alphanumeric()))
            .map(str::to_ascii_uppercase);
        self
    }

    /// Add the issuer's legal entity identifier (case-insensitive).
    pub fn with_lei(mut self, lei: &str) -> Self {
        let lei = lei.trim();
        self.lei = (!lei.is_empty()).then(|| lei.to_ascii_uppercase());
        self
    }

    /// The strongest rule that ties `holding` to the issuer, if any.
    pub fn match_rule(&self, holding: &Holding) -> Option<MatchRule> {
        let issuer = self.cusip_issuer.as_deref();
        if let (Some(code), Some(cusip)) = (issuer, holding.cusip.as_deref()) {
            if cusip.get(..6).is_some_and(|p| p.eq_ignore_ascii_case(code)) {
                return Some(MatchRule::Cusip);
            }
        }
        if let (Some(code), Some(isin)) = (issuer, holding.isin.as_deref()) {
            // Only CUSIP countries embed a CUSIP in the ISIN.
            let cusip_country = isin
                .get(..2)
                .is_some_and(|c| c.eq_ignore_ascii_case("US") || c.eq_ignore_ascii_case("CA"));
            if cusip_country && isin.get(2..8).is_some_and(|p| p.eq_ignore_ascii_case(code)) {
                return Some(MatchRule::Isin);
            }
        }
        if let (Some(lei), Some(h)) = (self.lei.as_deref(), holding.lei.as_deref()) {
            if h.eq_ignore_ascii_case(lei) {
                return Some(MatchRule::Lei);
            }
        }
        // A foreign line can reuse a US symbol, so a non-US ISIN rules the ticker out.
        let us_or_unlisted = holding
            .isin
            .as_deref()
            .is_none_or(|i| i.get(..2).is_some_and(|c| c.eq_ignore_ascii_case("US")));
        if !self.ticker.is_empty()
            && us_or_unlisted
            && holding
                .ticker
                .as_deref()
                .is_some_and(|t| normalize_ticker(t) == self.ticker)
        {
            return Some(MatchRule::Ticker);
        }
        if self
            .name
            .as_deref()
            .is_some_and(|n| normalize_issuer_name(&holding.name) == n)
        {
            return Some(MatchRule::Name);
        }
        None
    }
}

/// A holding that belongs to the issuer, with the rule that matched it.
#[derive(Debug, Clone, PartialEq)]
pub struct ExposureMatch {
    /// The matched position (its `asset_cat` keeps bond/derivative lines visible).
    pub holding: Holding,
    /// Why it matched.
    pub rule: MatchRule,
    /// True when the line is part of [`Exposure::pct`] and [`Exposure::value_usd`]
    /// (equity: asset category `EC` or `EP`). Debt is reported separately and
    /// derivative or other lines are listed only.
    pub counted: bool,
}

/// Exposure of a fund to one issuer.
#[derive(Debug, Clone, PartialEq)]
pub struct Exposure {
    /// Equity (`EC`/`EP`) percent of net assets, shorts negative.
    pub pct: f64,
    /// Equity USD value, shorts negative.
    pub value_usd: f64,
    /// Net percent of net assets held in the issuer's debt (`DBT`).
    pub debt_pct: f64,
    /// Net USD value of the issuer's debt (`DBT`).
    pub debt_value_usd: f64,
    /// Every matched position (equity, debt and others); see [`ExposureMatch::counted`].
    pub matches: Vec<ExposureMatch>,
}

/// Sum the issuer's equity positions in `report`, and its debt separately.
///
/// Only equity lines (`EC`, `EP`) go into `pct` and `value_usd`, so the figure
/// answers "how much of this stock does buying the fund buy". Debt (`DBT`)
/// is summed into `debt_pct`; derivatives and other categories are listed in
/// `matches` (with `counted == false`) but never summed. Short positions count
/// negative whichever sign the filing used.
pub fn exposure(report: &NportReport, matcher: &IssuerMatcher) -> Exposure {
    let matches: Vec<ExposureMatch> = report
        .holdings
        .iter()
        .filter_map(|h| {
            matcher.match_rule(h).map(|rule| ExposureMatch {
                holding: h.clone(),
                rule,
                counted: matches!(h.asset_cat.as_str(), "EC" | "EP"),
            })
        })
        .collect();
    let signed = |h: &Holding, v: f64| if h.short { -v.abs() } else { v };
    // Explicit 0.0 seeds: an empty float sum would otherwise be -0.0.
    let (mut pct, mut value_usd, mut debt_pct, mut debt_value_usd) = (0.0, 0.0, 0.0, 0.0);
    for m in &matches {
        let h = &m.holding;
        if m.counted {
            pct += signed(h, h.pct);
            value_usd += signed(h, h.value_usd);
        } else if h.asset_cat == "DBT" {
            debt_pct += signed(h, h.pct);
            debt_value_usd += signed(h, h.value_usd);
        }
    }
    Exposure {
        pct,
        value_usd,
        debt_pct,
        debt_value_usd,
        matches,
    }
}

// ─── Point-in-time selection ──────────────────────────────────────────────────

/// At most this many amendments are fetched per point-in-time lookup.
pub const MAX_AMENDMENTS: usize = 3;

/// Newest original `NPORT-P` filed on or before `as_of`, whatever the feed order.
pub fn select_original(filings: &[NportFiling], as_of: NaiveDate) -> Option<&NportFiling> {
    filings
        .iter()
        .filter(|f| f.form == "NPORT-P" && f.filing_date <= as_of)
        .min_by_key(|f| Reverse(f.filing_date))
}

/// `NPORT-P/A` filings made after `original` and on or before `as_of`, newest
/// first, capped at [`MAX_AMENDMENTS`]. They may amend other periods; see
/// [`same_period_amendment`].
pub fn amendment_candidates<'a>(
    filings: &'a [NportFiling],
    original: &NportFiling,
    as_of: NaiveDate,
) -> Vec<&'a NportFiling> {
    let mut v: Vec<&NportFiling> = filings
        .iter()
        .filter(|f| {
            f.form == "NPORT-P/A" && f.filing_date >= original.filing_date && f.filing_date <= as_of
        })
        .collect();
    v.sort_by_key(|f| Reverse(f.filing_date));
    v.truncate(MAX_AMENDMENTS);
    v
}

/// Index of the first (newest) amendment whose period equals the original's,
/// or `None` to keep the original. An amendment of an older period never
/// replaces it.
pub fn same_period_amendment(
    original_period: NaiveDate,
    amendment_periods: &[NaiveDate],
) -> Option<usize> {
    amendment_periods.iter().position(|p| *p == original_period)
}

// ─── Client ───────────────────────────────────────────────────────────────────

/// Resolve `ticker` against already loaded maps: its [`FundId`], else
/// [`EdgarError::NotNportFiler`] for a known non-fund SEC company, else
/// [`EdgarError::UnknownFund`] with suggestions.
pub fn resolve_fund_id(
    funds: &FundTickerMap,
    companies: &TickerMap,
    ticker: &str,
) -> Result<FundId, EdgarError> {
    if let Some(id) = funds.get(ticker) {
        return Ok(id.clone());
    }
    if companies.get(ticker).is_some() {
        return Err(EdgarError::NotNportFiler {
            ticker: ticker.to_string(),
        });
    }
    let mut suggestions = funds.suggest(ticker);
    if suggestions.is_empty() {
        suggestions = companies.suggest(ticker);
    }
    Err(EdgarError::UnknownFund {
        ticker: ticker.to_string(),
        suggestions,
    })
}

impl EdgarClient {
    /// Series and class IDs for a fund ticker (case-insensitive).
    ///
    /// A ticker that is a known SEC company but not a fund (a unit investment
    /// trust such as SPY) is [`EdgarError::NotNportFiler`]; a ticker unknown to
    /// both lists is [`EdgarError::UnknownFund`] with suggestions.
    pub async fn fund_id(&self, ticker: &str) -> Result<FundId, EdgarError> {
        let funds = self.fund_tickers().await?;
        if let Some(id) = funds.get(ticker) {
            return Ok(id.clone());
        }
        let companies = self.company_tickers().await?;
        resolve_fund_id(&funds, &companies, ticker)
    }

    /// The cached fund ticker map (`company_tickers_mf.json`). Load it once and
    /// share it when resolving many tickers.
    pub async fn fund_tickers(&self) -> Result<FundTickerMap, EdgarError> {
        let url = format!("{}/files/company_tickers_mf.json", self.www_base);
        let key = format!(
            "{}/{FUND_TICKERS_CACHE_KEY}",
            cache_namespace(&self.www_base)
        );
        self.get_cached_with(&url, &key, parse_fund_tickers, false)
            .await
    }

    /// The cached `company_tickers.json` map (ticker -> CIK and title).
    pub async fn company_ticker_map(&self) -> Result<TickerMap, EdgarError> {
        self.company_tickers().await
    }

    /// EDGAR's company title for `ticker` (e.g. `AMAZON COM INC`), `None` when
    /// the ticker is not in `company_tickers.json`.
    pub async fn issuer_title(&self, ticker: &str) -> Result<Option<String>, EdgarError> {
        let companies = self.company_tickers().await?;
        Ok(companies.title(ticker).map(str::to_string))
    }

    /// N-PORT filings (NPORT-P and NPORT-P/A, feed order) of a fund series.
    /// The feed is cached for `cache_ttl_hours`.
    pub async fn nport_filings(&self, series_id: &str) -> Result<Vec<NportFiling>, EdgarError> {
        if series_id.is_empty() || !series_id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(EdgarError::Parse(format!("bad series ID {series_id:?}")));
        }
        let url = format!(
            "{}/cgi-bin/browse-edgar?action=getcompany&CIK={series_id}&type=NPORT-P\
             &dateb=&owner=include&count=40&output=atom",
            self.www_base
        );
        let key = format!(
            "{}/nport_feed_{series_id}.atom",
            cache_namespace(&self.www_base)
        );
        self.get_cached_with(&url, &key, parse_series_feed, false)
            .await
    }

    /// The N-PORT report that was current at `as_of`.
    ///
    /// Uses the newest original NPORT-P filed on or before `as_of` (never a
    /// later one). Up to [`MAX_AMENDMENTS`] later NPORT-P/A filings (also filed
    /// by `as_of`) are then checked newest first, and the first one for the
    /// same period replaces the original. Returns the filing the report came from.
    pub async fn nport_as_of(
        &self,
        ticker: &str,
        as_of: NaiveDate,
    ) -> Result<(NportFiling, NportReport), EdgarError> {
        let fund = self.fund_id(ticker).await?;
        self.nport_as_of_for(&fund, ticker, as_of).await
    }

    /// [`EdgarClient::nport_as_of`] for an already resolved fund (no ticker-map lookups).
    pub async fn nport_as_of_for(
        &self,
        fund: &FundId,
        ticker: &str,
        as_of: NaiveDate,
    ) -> Result<(NportFiling, NportReport), EdgarError> {
        let filings = self.nport_filings(&fund.series_id).await?;
        let original =
            select_original(&filings, as_of).ok_or_else(|| EdgarError::NoNportFiling {
                ticker: ticker.to_string(),
                as_of,
            })?;
        let report = self.nport_report(original, ticker, &fund.series_id).await?;
        let mut periods = Vec::new();
        let mut amendments = Vec::new();
        for candidate in amendment_candidates(&filings, original, as_of) {
            // An amendment only refines the original, so a broken one is skipped.
            match self.nport_report(candidate, ticker, &fund.series_id).await {
                Ok(amended) => {
                    periods.push(amended.period_end);
                    amendments.push((candidate, amended));
                }
                Err(e) => {
                    tracing::warn!(
                        accession = %candidate.accession,
                        error = %e,
                        "Skipping unusable N-PORT amendment"
                    );
                    continue;
                }
            }
            if same_period_amendment(report.period_end, &periods).is_some() {
                break;
            }
        }
        match same_period_amendment(report.period_end, &periods) {
            Some(i) => {
                let (filing, amended) = amendments.swap_remove(i);
                Ok((filing.clone(), amended))
            }
            None => Ok((original.clone(), report)),
        }
    }

    /// Fetch and parse one filing's `primary_doc.xml`. Documents are immutable
    /// per accession, so the cache entry never expires.
    /// A document for another series than `series_id` is
    /// [`EdgarError::SeriesMismatch`].
    async fn nport_report(
        &self,
        filing: &NportFiling,
        ticker: &str,
        series_id: &str,
    ) -> Result<NportReport, EdgarError> {
        if !is_valid_accession(&filing.accession) {
            return Err(EdgarError::Parse(format!(
                "bad accession {:?}",
                filing.accession
            )));
        }
        let folder = filing.accession.replace('-', "");
        let url = format!(
            "{}/Archives/edgar/data/{}/{folder}/primary_doc.xml",
            self.www_base,
            filing.cik.numeric()
        );
        let key = format!(
            "{}/nport_{}_{folder}.xml",
            cache_namespace(&self.www_base),
            filing.cik.numeric()
        );
        let report = self.get_cached_with(&url, &key, parse_nport, true).await?;
        if report.series_id != series_id {
            return Err(EdgarError::SeriesMismatch {
                ticker: ticker.to_string(),
                expected: series_id.to_string(),
                found: report.series_id,
            });
        }
        Ok(report)
    }
}
