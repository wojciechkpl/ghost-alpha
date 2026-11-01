//! RED-phase contract for L2b slice 3 on `GET /partials/unwind`: the "Compare spreads" section.
//! Pure maths: `tests/unwind_spread.rs`. Synthetic data only (tickers XYZ, ABC).
//!
//! The handler starts the schedule today, so nothing here pins a dollar value that depends on the
//! run date. Instead the tests (a) compare the table with the main panel for the same request
//! (`years=N` partial vs row N), (b) use ANNUAL cadence on a long-term-only position, where every
//! sale lands in its own calendar year and each year's tax is `year_tax(0, gain / N)` whatever the
//! date, and (c) recompute the main projection from the public maths with the fixed seed.
//!
//! Markup pinned (everything else is left to the implementer):
//! * `<h3>Compare spreads</h3>` after the Tax by calendar year panel, then the table, then the chart.
//! * table inside `<div class="table-scroll" role="region" aria-label="Spread comparison"
//!   tabindex="0">`, with a `<caption>` that names the cadence and the year-5 horizon,
//!   `<th scope="col">` headers, one `<tr data-spread="N">` per N in 1..=5 (each with a
//!   `<th scope="row">` label) and a final `<tr data-spread="hold">`.
//! * header texts (matched case-insensitively by substring; the cells of a row line up with them):
//!   "Spread", "Calendar years taxed", "Total tax", "Tax % of gain", "Saved vs 1 year",
//!   "After-tax proceeds", "Year-5 p10", "Year-5 p50". The four tax columns (calendar years, total
//!   tax, tax %, saved) exist only with tax settings.
//! * the current row has `aria-current="true"` on its `<tr>` and the text "(current)"; its label
//!   is not a link, every other spread label is an `<a href>` to the canonical page URL with
//!   `years=N` (HTML-escaped `&amp;`, values percent-encoded).
//! * the lowest-RATE row carries the visible text "Lowest tax rate" (exactly once, only with tax);
//!   ranking is by tax rate with ties within 0.0005 going to the shorter spread (round 2).
//! * hold row: label contains "before tax"; total tax and tax % say "deferred"; "Saved vs 1 year"
//!   is an em dash. "Saved vs 1 year" of row N is `(rate_1 - rate_N) * proceeds_N`, signed.
//! * the note says the spread "postpones the tax" and "does not avoid it", explains the 1-year
//!   rate, and says tax follows the schedule price path while year-5 values are simulated.
//! * round 3: the rate column is "Tax % of gain" (tax / gain, `pct1`); ranking and "Saved vs 1
//!   year" use it: saved = (g_1 - g_N) * gain_N, shown signed with a leading "+" when positive.
//! * the chart has a `<details>` table view: Spread, Federal, State, NIIT, Total.
//! * chart (tax only): accessible svg with id suffix `unwind-spread-tax`, labels "1 yr".."5 yr".

mod common;

use axum::Router;
use common::web::*;
use ghost_alpha::portfolio::types::{AcquisitionType, TaxLot};
use ghost_alpha::portfolio::unwind::{
    build_schedule, project, year_tax, Cadence, ProjectionInput, TaxSettings, UnwindInput,
    DEFAULT_PATHS, DEFAULT_REST_MU, DEFAULT_REST_SIGMA, DEFAULT_RHO, PROJECTION_SEED,
};
use ghost_alpha::storage::db::SqliteRepository;
use ghost_alpha::tax::brackets::{state_tax_rate, FederalCapGainsBrackets, FilingStatus};

const PAGE: &str = "/portfolio/concentration";
const PARTIAL: &str = "/partials/unwind";
const HEADING: &str = "<h3>Compare spreads</h3>";

// XYZ 1000 sh @ 20 (acquired 2022-01-03, long term), close 100 = 100,000 (gain 80,000)
// ABC  500 sh @ 100, close 100                                  =  50,000
fn seed(repo: &SqliteRepository) {
    insert_lot_dated(repo, "lot-1", "XYZ", 1000.0, 20.0, "2022-01-03");
    insert_lot_dated(repo, "lot-2", "ABC", 500.0, 100.0, "2022-01-03");
    insert_close(repo, "XYZ", "2026-10-24", 100.0);
    insert_close(repo, "ABC", "2026-10-24", 100.0);
}

fn app() -> Router {
    app_with_profile(seed, None)
}

fn app_tx() -> Router {
    app_with_profile(seed, Some(profile("TX", "single", 80_000.0)))
}

fn app_ca() -> Router {
    app_with_profile(seed, Some(profile("CA", "single", 250_000.0)))
}

// ───────────────────────────── html helpers ─────────────────────────────

fn tag_end(html: &str, from: usize, close: &str) -> usize {
    html[from..]
        .find(close)
        .map(|e| e + from)
        .unwrap_or(html.len())
}

/// Raw html between `open_tag` (as found in `html`) and its matching close tag.
fn raw_inner<'a>(html: &'a str, open_tag: &str) -> &'a str {
    let start = html.find(open_tag).expect("open tag in html") + open_tag.len();
    let name: String = open_tag[1..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    let (open, close) = (format!("<{name}"), format!("</{name}>"));
    let (mut depth, mut i) = (1, start);
    loop {
        let rest = &html[i..];
        let next_close = rest.find(&close).expect("closing tag");
        match rest.find(&open) {
            Some(o)
                if o < next_close && rest[o + open.len()..].starts_with(['>', ' ', '/', '\n']) =>
            {
                depth += 1;
                i += o + open.len();
            }
            _ => {
                depth -= 1;
                if depth == 0 {
                    return &html[start..i + next_close];
                }
                i += next_close + close.len();
            }
        }
    }
}

fn kpi_text(html: &str, name: &str) -> String {
    let tag = all_tags(html)
        .into_iter()
        .find(|t| attr(t, "data-kpi").as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no data-kpi=\"{name}\" in:\n{html}"));
    strip_tags(raw_inner(html, &tag))
}

fn has_kpi(html: &str, name: &str) -> bool {
    all_tags(html)
        .iter()
        .any(|t| attr(t, "data-kpi").as_deref() == Some(name))
}

/// First `$` amount (or, without a `$`, the first number) in `s`.
fn num(s: &str) -> f64 {
    let neg = s.find('$').is_some_and(|i| s[..i].ends_with('-'));
    let s = s.find('$').map(|i| &s[i + 1..]).unwrap_or(s);
    let t: String = s
        .chars()
        .skip_while(|c| !(c.is_ascii_digit() || *c == '-'))
        .take_while(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | '-'))
        .filter(|c| *c != ',')
        .collect();
    let v: f64 = t.parse().unwrap_or_else(|_| panic!("no number in {s:?}"));
    if neg {
        -v.abs()
    } else {
        v
    }
}

/// A "Saved vs 1 year" cell: a signed amount, or a dash for nothing.
fn saved_num(text: &str) -> f64 {
    if text.contains('\u{2014}') || text.contains("&mdash;") {
        0.0
    } else {
        num(text)
    }
}

fn kpi(html: &str, name: &str) -> f64 {
    num(&kpi_text(html, name))
}

async fn partial(app: &Router, q: &str) -> String {
    let r = get(app, &format!("{PARTIAL}?{q}")).await;
    assert_eq!(r.status, axum::http::StatusCode::OK, "{q}");
    r.text()
}

/// Everything from the spread heading on (panics when the section is missing).
fn section(body: &str) -> &str {
    let i = body
        .find(HEADING)
        .unwrap_or_else(|| panic!("no {HEADING} in:\n{body}"));
    &body[i..]
}

/// Cell texts of one table row / header row, in order (`<th>` and `<td>` alike).
fn cells(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(p) = html[i..].find("<t") {
        let at = i + p;
        let b = html.as_bytes();
        let kind = b.get(at + 2).copied();
        let after = b.get(at + 3).copied();
        let is_cell =
            matches!(kind, Some(b'h') | Some(b'd')) && matches!(after, Some(b' ') | Some(b'>'));
        if !is_cell {
            i = at + 2;
            continue;
        }
        let close = if kind == Some(b'h') { "</th>" } else { "</td>" };
        let open_end = tag_end(html, at, ">") + 1;
        let end = tag_end(html, open_end, close);
        out.push(strip_tags(&html[open_end..end]).trim().to_string());
        i = end;
    }
    out
}

fn table_scroll_tag(body: &str) -> String {
    open_tags(section(body), "div")
        .into_iter()
        .find(|t| {
            attr(t, "class").is_some_and(|c| c.split_whitespace().any(|w| w == "table-scroll"))
        })
        .unwrap_or_else(|| panic!("no .table-scroll wrapper after the heading in:\n{body}"))
}

fn table_html(body: &str) -> String {
    raw_inner(section(body), &table_scroll_tag(body)).to_string()
}

fn headers(body: &str) -> Vec<String> {
    let t = table_html(body);
    let s = t.find("<thead").expect("<thead>");
    let e = tag_end(&t, s, "</thead>");
    cells(&t[s..e])
}

fn col(body: &str, needle: &str) -> usize {
    let h = headers(body);
    h.iter()
        .position(|x| x.to_lowercase().contains(&needle.to_lowercase()))
        .unwrap_or_else(|| panic!("no column containing {needle:?} in {h:?}"))
}

fn has_col(body: &str, needle: &str) -> bool {
    headers(body)
        .iter()
        .any(|x| x.to_lowercase().contains(&needle.to_lowercase()))
}

/// `(opening <tr> tag, html)` of the row with `data-spread=key`.
fn spread_row(body: &str, key: &str) -> (String, String) {
    let t = table_html(body);
    let tag = open_tags(&t, "tr")
        .into_iter()
        .find(|x| attr(x, "data-spread").as_deref() == Some(key))
        .unwrap_or_else(|| panic!("no data-spread=\"{key}\" row in:\n{t}"));
    let s = t.find(&tag).unwrap();
    let e = tag_end(&t, s, "</tr>") + 5;
    (tag, t[s..e.min(t.len())].to_string())
}

fn spread_keys(body: &str) -> Vec<String> {
    open_tags(&table_html(body), "tr")
        .iter()
        .filter_map(|t| attr(t, "data-spread"))
        .collect()
}

fn cell(body: &str, key: &str, needle: &str) -> String {
    let c = cells(&spread_row(body, key).1);
    let h = headers(body);
    assert_eq!(c.len(), h.len(), "row {key} cells {c:?} vs headers {h:?}");
    c[col(body, needle)].clone()
}

fn svg_with(body: &str, id: &str) -> Option<String> {
    svgs(section(body)).into_iter().find(|s| s.contains(id))
}

fn desc_of(svg: &str) -> String {
    let d = svg.find("<desc").expect("<desc>");
    svg[d..svg[d..].find("</desc>").unwrap() + d].to_string()
}

fn tax_settings(state: &str, other_income: f64) -> TaxSettings {
    TaxSettings {
        state: state_tax_rate(state).unwrap(),
        brackets: FederalCapGainsBrackets::for_filing(&FilingStatus::Single),
        other_income,
    }
}

/// Annual cadence, N years of equal sales of the long-term XYZ position: each lands in its own
/// calendar year, so the tax is N one-sale years whatever today is.
fn annual_tx_tax(n: u32) -> f64 {
    let t = tax_settings("TX", 80_000.0);
    n as f64 * year_tax(0.0, 80_000.0 / n as f64, &t).total
}

// ═════════════════════════════ presence and structure ═════════════════════════════

#[tokio::test]
async fn test_spread_section_should_render_after_tax_by_year_with_five_spreads_plus_hold() {
    let body = partial(&app_ca(), "years=3").await;
    let (tax_h, spread_h) = (
        body.find("<h3>Tax by calendar year</h3>"),
        body.find(HEADING),
    );
    assert!(tax_h.is_some(), "{body}");
    assert!(spread_h.is_some(), "no {HEADING} in:\n{body}");
    assert!(tax_h < spread_h, "spreads must come after tax by year");
    assert_eq!(
        spread_keys(&body),
        vec!["1", "2", "3", "4", "5", "hold"],
        "{body}"
    );
}

#[tokio::test]
async fn test_spread_table_should_be_a_labelled_scrollable_region_with_caption_and_scoped_headers()
{
    let body = partial(&app_ca(), "years=3&cadence=annual").await;
    let w = table_scroll_tag(&body);
    assert_eq!(attr(&w, "role").as_deref(), Some("region"), "{w}");
    assert_eq!(
        attr(&w, "aria-label").as_deref(),
        Some("Spread comparison"),
        "{w}"
    );
    assert_eq!(attr(&w, "tabindex").as_deref(), Some("0"), "{w}");
    let t = table_html(&body);
    let cap_s = t.find("<caption").expect("<caption>");
    let cap = strip_tags(&t[cap_s..tag_end(&t, cap_s, "</caption>")]).to_lowercase();
    assert!(cap.contains("annual"), "{cap}");
    assert!(
        cap.contains('5'),
        "caption must name the year-5 horizon: {cap}"
    );
    // every header is a column header; every row starts with a row header
    let head = &t[t.find("<thead").unwrap()..tag_end(&t, t.find("<thead").unwrap(), "</thead>")];
    let ths = open_tags(head, "th");
    assert!(!ths.is_empty());
    for th in &ths {
        assert_eq!(attr(th, "scope").as_deref(), Some("col"), "{th}");
    }
    for key in ["1", "2", "3", "4", "5", "hold"] {
        let (_, row) = spread_row(&body, key);
        let row_ths = open_tags(&row, "th");
        assert_eq!(row_ths.len(), 1, "row {key}: {row}");
        assert_eq!(attr(&row_ths[0], "scope").as_deref(), Some("row"), "{row}");
        assert!(row.trim_start().contains("<th"), "{row}");
    }
}

#[tokio::test]
async fn test_spread_table_with_tax_should_have_the_specified_columns_in_order() {
    let body = partial(&app_ca(), "years=3").await;
    let h: Vec<String> = headers(&body).iter().map(|s| s.to_lowercase()).collect();
    let needles = [
        "spread",
        "calendar years",
        "total tax",
        "tax % of gain",
        "saved",
        "after-tax",
        "p10",
        "p50",
    ];
    assert_eq!(h.len(), needles.len(), "{h:?}");
    for (cell, needle) in h.iter().zip(needles) {
        assert!(
            cell.contains(needle),
            "{cell:?} should contain {needle:?}: {h:?}"
        );
    }
}

#[tokio::test]
async fn test_spread_section_should_state_the_assumption_note() {
    let body = partial(&app_ca(), "years=3").await;
    let note = section(&body).to_lowercase();
    assert!(note.contains("random"), "{note}");
    assert!(note.contains("hold"), "{note}");
}

#[tokio::test]
async fn test_spread_note_should_say_spreading_postpones_tax_and_explain_the_numbers() {
    let body = partial(&app_ca(), "years=3").await;
    // Visible text of everything after the heading, lower-cased, whitespace collapsed.
    let text = strip_tags(section(&body))
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    for phrase in [
        "postpones the tax",
        "does not avoid it",
        "1-year spread",
        "price path",
        "simulation",
        "on the gain then",
    ] {
        assert!(text.contains(phrase), "note lacks {phrase:?}: {text}");
    }
    assert!(
        !text.contains("larger gain"),
        "the hold note must say tax on the gain then, not a larger gain: {text}"
    );
    assert!(
        !text.contains("tax % of proceeds"),
        "the rate column is now per gain: {text}"
    );
}

/// Headers and rows of the `<details>` table view that follows the spread chart.
fn chart_view(body: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let sec = section(body);
    let tag = open_tags(sec, "details")
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no <details> table view in the spread section:\n{sec}"));
    let inner = raw_inner(sec, &tag);
    let head_s = inner.find("<thead").expect("<thead>");
    let head = cells(&inner[head_s..tag_end(inner, head_s, "</thead>")]);
    let body_s = inner.find("<tbody").expect("<tbody>");
    let mut rows = Vec::new();
    let mut rest = &inner[body_s..];
    while let Some(i) = rest.find("<tr") {
        let e = tag_end(rest, i, "</tr>");
        rows.push(cells(&rest[i..e]));
        rest = &rest[(e + 5).min(rest.len())..];
    }
    (head, rows)
}

#[tokio::test]
async fn test_spread_chart_should_have_a_table_view_with_components_that_match_the_main_table() {
    let body = partial(&app_ca(), "years=3").await;
    let sec = section(&body);
    let chart_at = sec.find("<svg").expect("chart");
    let details_at = sec.find("<details").expect("<details> table view");
    assert!(chart_at < details_at, "table view follows the chart");
    let (head, rows) = chart_view(&body);
    let low: Vec<String> = head.iter().map(|h| h.to_lowercase()).collect();
    assert_eq!(low.len(), 5, "{head:?}");
    for (h, want) in low
        .iter()
        .zip(["spread", "federal", "state", "niit", "total"])
    {
        assert!(h.contains(want), "{h:?} vs {want}: {head:?}");
    }
    assert_eq!(rows.len(), 5, "one row per spread: {rows:?}");
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row.len(), 5, "{row:?}");
        let n = (i + 1).to_string();
        assert!(row[0].contains(&n), "row label names the spread: {row:?}");
        assert_eq!(
            row[4],
            cell(&body, &n, "total tax"),
            "view total must equal the main table's total tax for {n}"
        );
        let parts: f64 = row[1..4].iter().map(|c| num(c)).sum();
        assert!((parts - num(&row[4])).abs() <= 2.0, "{row:?}");
    }
}

#[tokio::test]
async fn test_spread_css_should_make_the_table_wrapper_scroll_horizontally() {
    let css = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/web/static/ghost-alpha.css"
    ))
    .unwrap();
    let i = css
        .find(".table-scroll")
        .expect(".table-scroll rule in ghost-alpha.css");
    let block = &css[i..tag_end(&css, i, "}")];
    let compact: String = block.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(compact.contains("overflow-x:auto"), "{block}");
}

// ═════════════════════════════ rows agree with the main panel ═════════════════════════════

#[tokio::test]
async fn test_spread_row_total_tax_and_net_should_equal_the_main_panel_for_the_same_years() {
    let app = app_ca();
    for cadence in ["quarterly", "annual"] {
        let body = partial(&app, &format!("years=3&cadence={cadence}")).await;
        for n in 1..=5u32 {
            let main = partial(&app, &format!("years={n}&cadence={cadence}")).await;
            let tax = num(&cell(&body, &n.to_string(), "total tax"));
            assert!(
                (tax - kpi(&main, "total-tax")).abs() <= 1.0,
                "{cadence} {n}: {tax} vs {}",
                kpi(&main, "total-tax")
            );
            let net = num(&cell(&body, &n.to_string(), "after-tax"));
            assert!(
                (net - kpi(&main, "after-tax-proceeds")).abs() <= 1.0,
                "{cadence} {n}"
            );
        }
    }
}

#[tokio::test]
async fn test_spread_current_row_for_five_years_should_match_the_main_panel_projection() {
    let app = app_ca();
    let body = partial(&app, "years=5&cadence=annual").await;
    let tax = num(&cell(&body, "5", "total tax"));
    assert!((tax - kpi(&body, "total-tax")).abs() <= 1.0);
    // Year-5 p10 of the unwind row is the main panel's year-5 unwind p10; hold likewise.
    let p10 = num(&cell(&body, "5", "p10"));
    assert!((p10 - kpi(&body, "p10-unwind")).abs() <= 1.0, "{p10}");
    let hold = num(&cell(&body, "hold", "p10"));
    assert!((hold - kpi(&body, "p10-hold")).abs() <= 1.0, "{hold}");
}

#[tokio::test]
async fn test_spread_p10_p50_cells_should_be_positive_and_ordered_for_every_row() {
    let body = partial(&app_ca(), "years=3").await;
    for key in ["1", "2", "3", "4", "5", "hold"] {
        let (p10, p50) = (num(&cell(&body, key, "p10")), num(&cell(&body, key, "p50")));
        assert!(p10 > 0.0 && p10 <= p50, "{key}: {p10} {p50}");
    }
}

#[tokio::test]
async fn test_spread_rows_annual_tx_should_equal_year_tax_for_each_spread() {
    // 100,000 proceeds, 80,000 long-term gain split over N separate calendar years.
    let body = partial(&app_tx(), "years=3&cadence=annual").await;
    for n in 1..=5u32 {
        let key = n.to_string();
        let tax = num(&cell(&body, &key, "total tax"));
        assert!(
            (tax - annual_tx_tax(n)).abs() <= 1.0,
            "N={n}: table {tax}, year_tax {}",
            annual_tx_tax(n)
        );
        let net = num(&cell(&body, &key, "after-tax"));
        assert!((net - (100_000.0 - annual_tx_tax(n))).abs() <= 1.0, "N={n}");
        assert_eq!(
            num(&cell(&body, &key, "calendar years")),
            n as f64,
            "annual sales: one calendar year per spread year"
        );
        // Tax as a share of the 80,000 gain.
        let rate = num(&cell(&body, &key, "tax %"));
        assert!(
            (rate - annual_tx_tax(n) / 800.0).abs() <= 0.06,
            "N={n}: {rate}% vs {}",
            annual_tx_tax(n) / 800.0
        );
        if n > 1 {
            let saved = saved_num(&cell(&body, &key, "saved"));
            assert!(
                (saved - (annual_tx_tax(1) - annual_tx_tax(n))).abs() <= 1.5,
                "N={n}: saved {saved}"
            );
        }
    }
}

#[tokio::test]
async fn test_spread_calendar_years_taxed_should_be_at_least_the_spread_years_for_quarterly() {
    // Quarterly sales from today span ceil((n*4 + offset)/4) calendar years: n or n + 1.
    let body = partial(&app_ca(), "years=3&cadence=quarterly").await;
    for n in 1..=5u32 {
        let c = num(&cell(&body, &n.to_string(), "calendar years")) as u32;
        assert!(c == n || c == n + 1, "N={n}: {c} calendar years");
    }
}

// ═════════════════════════════ hold row, current row, lowest tax ═════════════════════════════

#[tokio::test]
async fn test_spread_hold_row_should_defer_tax_instead_of_showing_zero() {
    let body = partial(&app_ca(), "years=3").await;
    let (_, row) = spread_row(&body, "hold");
    let th_s = row.find("<th").unwrap();
    let label = strip_tags(&row[th_s..tag_end(&row, th_s, "</th>")]).to_lowercase();
    assert!(label.contains("hold"), "{label}");
    assert!(
        label.contains("before tax"),
        "hold label must say its value is before tax: {label}"
    );
    for needle in ["total tax", "tax %"] {
        let c = cell(&body, "hold", needle).to_lowercase();
        assert!(c.contains("deferred"), "{needle}: {c}");
    }
    let saved = cell(&body, "hold", "saved").replace("&mdash;", "\u{2014}");
    assert_eq!(saved.trim(), "\u{2014}", "hold has nothing saved vs 1 year");
    assert!(num(&cell(&body, "hold", "p50")) > 0.0);
}

#[tokio::test]
async fn test_spread_current_row_should_follow_the_years_control() {
    let app = app_ca();
    for years in 1..=5u32 {
        let body = partial(&app, &format!("years={years}&cadence=annual")).await;
        let cur: Vec<String> = open_tags(&table_html(&body), "tr")
            .into_iter()
            .filter(|t| attr(t, "aria-current").as_deref() == Some("true"))
            .filter_map(|t| attr(&t, "data-spread"))
            .collect();
        assert_eq!(cur, vec![years.to_string()], "years={years}");
        for n in 1..=5u32 {
            let (_, row) = spread_row(&body, &n.to_string());
            assert_eq!(
                strip_tags(&row).contains("(current)"),
                n == years,
                "years={years} row {n}: {row}"
            );
        }
        let (_, hold) = spread_row(&body, "hold");
        assert!(!strip_tags(&hold).contains("(current)"));
    }
}

#[tokio::test]
async fn test_spread_when_years_exceed_five_should_have_no_current_row() {
    for years in ["7", "10"] {
        let body = partial(&app_ca(), &format!("years={years}")).await;
        assert_eq!(spread_keys(&body).len(), 6, "years={years}");
        let sec = section(&body);
        assert!(!sec.contains("aria-current"), "years={years}");
        assert!(!sec.contains("(current)"), "years={years}");
        assert!(
            sec.contains("<a "),
            "every row is a link when none is current"
        );
    }
}

#[tokio::test]
async fn test_spread_current_row_label_should_not_be_a_link_but_the_others_should() {
    let body = partial(&app_ca(), "years=2&cadence=annual").await;
    for n in 1..=5u32 {
        let (_, row) = spread_row(&body, &n.to_string());
        let th_s = row.find("<th").unwrap();
        let th = &row[th_s..tag_end(&row, th_s, "</th>")];
        assert_eq!(th.contains("<a "), n != 2, "row {n}: {th}");
    }
    let (_, hold) = spread_row(&body, "hold");
    assert!(!hold.contains("<a "), "hold row is not a spread link");
}

const LOW: &str = "Lowest tax rate";

#[tokio::test]
async fn test_spread_lowest_tax_rate_label_should_appear_exactly_once_on_the_lowest_rate_row() {
    let body = partial(&app_tx(), "years=3&cadence=annual").await;
    assert_eq!(body.matches(LOW).count(), 1, "{}", section(&body));
    // Lowest tax rate on the gain (80,000 here) by the library maths; ties within 0.0005 go to
    // the shorter spread. The rate is a flat 15% for every spread, so that is the 1-year row.
    let rates: Vec<f64> = (1..=5).map(|n| annual_tx_tax(n) / 80_000.0).collect();
    let min = rates.iter().cloned().fold(f64::INFINITY, f64::min);
    let want = rates.iter().position(|t| *t <= min + 0.0005).unwrap() + 1;
    for n in 1..=5usize {
        let (_, row) = spread_row(&body, &n.to_string());
        assert_eq!(row.contains(LOW), n == want, "row {n}: {row}");
        // The shown rate is tax / gain: a flat 15.0%, not tax / proceeds.
        assert_eq!(cell(&body, &n.to_string(), "tax %"), "15.0%", "row {n}");
    }
}

#[tokio::test]
async fn test_spread_lowest_tax_rate_label_should_sit_on_a_minimum_rate_row_for_quarterly_ca() {
    let body = partial(&app_ca(), "years=3").await;
    assert_eq!(body.matches(LOW).count(), 1);
    // Displayed rates are rounded to 0.1 percentage points, hence the slack.
    let rates: Vec<f64> = (1..=5)
        .map(|n| num(&cell(&body, &n.to_string(), "tax %")))
        .collect();
    let min = rates.iter().cloned().fold(f64::INFINITY, f64::min);
    let labelled: Vec<usize> = (1..=5usize)
        .filter(|n| spread_row(&body, &n.to_string()).1.contains(LOW))
        .collect();
    assert_eq!(labelled.len(), 1);
    assert!(
        rates[labelled[0] - 1] <= min + 0.05 + 0.1,
        "{rates:?} {labelled:?}"
    );
}

fn xyz_lots(shares: f64, basis: f64) -> Vec<TaxLot> {
    vec![TaxLot {
        id: uuid::Uuid::from_u128(1),
        ticker: "XYZ".to_string(),
        shares,
        cost_basis_per_share: basis,
        acquired_date: chrono::NaiveDate::from_ymd_opt(2022, 1, 3).unwrap(),
        acquisition_type: AcquisitionType::Buy,
        remaining_shares: shares,
    }]
}

/// `(tax, proceeds, gain)` of an annual unwind over N = 1..=5 years starting today, from the library.
fn lib_rows(
    state: &str,
    other_income: f64,
    shares: f64,
    basis: f64,
    growth: f64,
) -> Vec<(f64, f64, f64)> {
    (1..=5u32)
        .map(|n| {
            let s = build_schedule(&UnwindInput {
                lots: xyz_lots(shares, basis),
                price: 100.0,
                other_value: 50_000.0,
                years: n,
                cadence: Cadence::Annual,
                growth,
                start: chrono::Local::now().date_naive(),
                tax: Some(tax_settings(state, other_income)),
            });
            (s.totals.tax.unwrap(), s.totals.proceeds, s.totals.gain)
        })
        .collect()
}

/// CA, 120k other income, XYZ 10,000 sh @ 10 (1,000,000 at 100): with +8% a year the dollar tax
/// is lowest at 2 years but the tax RATE keeps falling, so the lowest-rate row is the 5-year one.
fn app_growth() -> Router {
    app_with_profile(
        |r| {
            insert_lot_dated(r, "lot-1", "XYZ", 10_000.0, 10.0, "2022-01-03");
            insert_lot_dated(r, "lot-2", "ABC", 500.0, 100.0, "2022-01-03");
            insert_close(r, "XYZ", "2026-10-24", 100.0);
            insert_close(r, "ABC", "2026-10-24", 100.0);
        },
        Some(profile("CA", "single", 120_000.0)),
    )
}

const GROWTH_Q: &str = "years=3&cadence=annual&growth=8";

/// Index (1-based) of the shortest spread whose gain tax rate is within 0.0005 of the lowest.
fn gain_rate_pick(rows: &[(f64, f64, f64)]) -> usize {
    let g: Vec<f64> = rows.iter().map(|(t, _, gain)| t / gain).collect();
    let min = g.iter().cloned().fold(f64::INFINITY, f64::min);
    g.iter().position(|x| *x <= min + 0.0005).unwrap() + 1
}

/// CA, 50k other income, XYZ 8,000 sh @ 50, +8% a year. Tax over gain: 30.7%, 29.5%, 28.6%,
/// 28.3%, 28.3% (4 and 5 years are within the tolerance); tax over proceeds would pick 1 year.
fn app_tolerance() -> Router {
    app_with_profile(
        |r| {
            insert_lot_dated(r, "lot-1", "XYZ", 8000.0, 50.0, "2022-01-03");
            insert_lot_dated(r, "lot-2", "ABC", 500.0, 100.0, "2022-01-03");
            insert_close(r, "XYZ", "2026-10-24", 100.0);
            insert_close(r, "ABC", "2026-10-24", 100.0);
        },
        Some(profile("CA", "single", 50_000.0)),
    )
}

#[tokio::test]
async fn test_spread_lowest_tax_rate_label_when_prices_grow_should_follow_the_gain_rate_not_dollars(
) {
    // (app, lots) fixtures where the dollar minimum and the gain-rate pick differ.
    let fixtures = [
        (
            app_growth(),
            lib_rows("CA", 120_000.0, 10_000.0, 10.0, 0.08),
            2,
            5,
        ),
        (
            app_tolerance(),
            lib_rows("CA", 50_000.0, 8000.0, 50.0, 0.08),
            1,
            4,
        ),
    ];
    for (app, rows, want_dollar, want_pick) in fixtures {
        let dollar_min = rows
            .iter()
            .position(|r| r.0 == rows.iter().map(|x| x.0).fold(f64::INFINITY, f64::min))
            .unwrap()
            + 1;
        let pick = gain_rate_pick(&rows);
        assert_eq!((dollar_min, pick), (want_dollar, want_pick), "{rows:?}");

        let body = partial(&app, GROWTH_Q).await;
        assert_eq!(body.matches(LOW).count(), 1, "{}", section(&body));
        for n in 1..=5usize {
            let (_, row) = spread_row(&body, &n.to_string());
            assert_eq!(row.contains(LOW), n == pick, "row {n} (pick {pick})");
        }
    }
}

#[tokio::test]
async fn test_spread_tax_percent_column_should_show_tax_over_gain() {
    let rows = lib_rows("CA", 120_000.0, 10_000.0, 10.0, 0.08);
    let body = partial(&app_growth(), GROWTH_Q).await;
    assert!(has_col(&body, "tax % of gain"), "{:?}", headers(&body));
    assert!(!has_col(&body, "of proceeds"), "{:?}", headers(&body));
    for (i, (tax, _, gain)) in rows.iter().enumerate() {
        let want = format!("{:.1}%", tax / gain * 100.0);
        assert_eq!(
            cell(&body, &(i + 1).to_string(), "tax %"),
            want,
            "N={}",
            i + 1
        );
    }
}

#[tokio::test]
async fn test_spread_lowest_tax_rate_label_on_a_flat_rate_should_stay_on_one_year_for_any_growth() {
    // TX single, 80k other income: a flat 15% on the gain whatever the price path.
    for growth in ["-10", "0", "8"] {
        let body = partial(
            &app_tx(),
            &format!("years=3&cadence=annual&growth={growth}"),
        )
        .await;
        assert_eq!(body.matches(LOW).count(), 1, "growth {growth}");
        for n in 1..=5u32 {
            let (_, row) = spread_row(&body, &n.to_string());
            assert_eq!(row.contains(LOW), n == 1, "growth {growth} row {n}: {row}");
            let saved = saved_num(&cell(&body, &n.to_string(), "saved"));
            assert!(saved.abs() <= 1.5, "growth {growth} N={n}: saved {saved}");
            let pct = cell(&body, &n.to_string(), "tax %");
            assert_eq!(pct, "15.0%", "growth {growth} N={n}");
        }
    }
}

// ═════════════════════════════ saved vs 1 year (signed) ═════════════════════════════

#[tokio::test]
async fn test_spread_saved_cell_when_prices_grow_should_apply_the_one_year_gain_rate_to_the_gain() {
    let rows = lib_rows("CA", 120_000.0, 10_000.0, 10.0, 0.08);
    let g1 = rows[0].0 / rows[0].2;
    let body = partial(&app_growth(), GROWTH_Q).await;
    for (i, (tax, _, gain)) in rows.iter().enumerate().skip(1) {
        let want = (g1 - tax / gain) * gain;
        let text = cell(&body, &(i + 1).to_string(), "saved");
        let got = num(&text);
        assert!((got - want).abs() <= 1.5, "N={}: {got} vs {want}", i + 1);
        assert_eq!(
            text.trim_start().starts_with('+'),
            want > 0.5,
            "positive saved values carry a leading +: {text:?}"
        );
    }
    // 5 years pays more dollars than 1 year, yet "saved" is positive (rate-based).
    assert!(rows[4].0 > rows[0].0);
    assert!(num(&cell(&body, "5", "saved")) > 30_000.0);
}

#[tokio::test]
async fn test_spread_saved_cell_when_a_longer_spread_pushes_gain_into_tax_should_be_negative() {
    // TX, no other income, XYZ 1,000 sh @ 60, +50% a year: the 1-year gain sits in the 0% band
    // and the 5-year spread's last sale does not.
    let app = app_with_profile(
        |r| {
            insert_lot_dated(r, "lot-1", "XYZ", 1000.0, 60.0, "2022-01-03");
            insert_lot_dated(r, "lot-2", "ABC", 500.0, 100.0, "2022-01-03");
            insert_close(r, "XYZ", "2026-10-24", 100.0);
            insert_close(r, "ABC", "2026-10-24", 100.0);
        },
        Some(profile("TX", "single", 0.0)),
    );
    let rows = lib_rows("TX", 0.0, 1000.0, 60.0, 0.5);
    let g1 = rows[0].0 / rows[0].2;
    let want = (g1 - rows[4].0 / rows[4].2) * rows[4].2;
    assert!(want < -5_000.0, "precondition: {want}");
    let body = partial(&app, "years=3&cadence=annual&growth=50").await;
    let text = cell(&body, "5", "saved");
    assert!(
        text.contains('-') && !text.trim_start().starts_with('+'),
        "negative saving must show a minus sign and no plus: {text:?}"
    );
    let got = num(&text);
    assert!(got < 0.0, "{text:?} parsed as {got}");
    assert!((got - want).abs() <= 1.5, "{got} vs {want}");
    // The 5-year rate shown is tax / gain.
    let g5 = rows[4].0 / rows[4].2;
    assert_eq!(cell(&body, "5", "tax %"), format!("{:.1}%", g5 * 100.0));
    // 1 and 2 years pay nothing, so the 1-year row is the lowest.
    let (_, row1) = spread_row(&body, "1");
    assert!(row1.contains(LOW), "{row1}");
}

// ═════════════════════════════ no tax settings ═════════════════════════════

#[tokio::test]
async fn test_spread_without_profile_should_drop_tax_columns_label_and_chart_but_keep_the_hint() {
    let body = partial(&app(), "years=3").await;
    assert_eq!(
        spread_keys(&body),
        vec!["1", "2", "3", "4", "5", "hold"],
        "section must still render without tax"
    );
    assert!(!body.contains("Lowest tax"));
    for needle in ["calendar years", "total tax", "tax %", "saved"] {
        assert!(!has_col(&body, needle), "{needle} column without tax");
    }
    assert!(has_col(&body, "p10") && has_col(&body, "p50"));
    assert!(svg_with(&body, "unwind-spread-tax").is_none());
    assert!(!body.contains("unwind-spread-tax"));
    assert!(body.contains("Tax is not shown because no state is set"));
    assert!(!has_kpi(&body, "total-tax"));
    // After-tax proceeds still reported (equal to proceeds) and p10 <= p50.
    for n in 1..=5u32 {
        assert!((num(&cell(&body, &n.to_string(), "after-tax")) - 100_000.0).abs() <= 1.0);
    }
}

#[tokio::test]
async fn test_spread_with_unsupported_state_should_behave_like_no_profile() {
    let app = app_with_profile(seed, Some(profile("ZZ", "single", 0.0)));
    let body = partial(&app, "years=3").await;
    assert_eq!(spread_keys(&body).len(), 6);
    assert!(!body.contains("Lowest tax"));
    assert!(!has_col(&body, "total tax"));
    assert!(!body.contains("unwind-spread-tax"));
}

// ═════════════════════════════ chart ═════════════════════════════

#[tokio::test]
async fn test_spread_chart_should_be_accessible_labelled_by_spread_and_free_of_hex_colours() {
    let body = partial(&app_ca(), "years=3").await;
    let svg = svg_with(&body, "unwind-spread-tax").expect("spread tax chart");
    assert!(svg.contains("ga-svg-unwind-spread-tax"), "{svg}");
    assert!(svg.contains("ga-desc-unwind-spread-tax"), "{svg}");
    assert!(svg.contains("role=\"img\""), "{svg}");
    assert!(svg.contains("<title"), "{svg}");
    assert!(svg.contains("var(--ga-series-1)"), "{svg}");
    assert!(
        !svg.contains("=\"#") && !svg.contains(":#"),
        "hex colour in {svg}"
    );
    for n in 1..=5 {
        assert!(svg.contains(&format!("{n} yr")), "label {n} yr in {svg}");
    }
}

#[tokio::test]
async fn test_spread_chart_desc_should_list_every_spreads_total_tax_and_year_five_p10() {
    let body = partial(&app_ca(), "years=3").await;
    let desc = desc_of(&svg_with(&body, "unwind-spread-tax").expect("spread tax chart"));
    for n in 1..=5u32 {
        let key = n.to_string();
        let tax = cell(&body, &key, "total tax");
        let p10 = cell(&body, &key, "p10");
        assert!(
            desc.contains(&tax),
            "total tax {tax} of {n} yr missing from {desc}"
        );
        assert!(
            desc.contains(&p10),
            "p10 {p10} of {n} yr missing from {desc}"
        );
    }
}

#[tokio::test]
async fn test_spread_chart_should_not_replace_the_tax_by_calendar_year_chart() {
    let body = partial(&app_ca(), "years=3").await;
    assert!(body.contains("ga-svg-unwind-tax-years"));
    assert!(body.contains("ga-svg-unwind-spread-tax"));
    assert!(body.contains("ga-svg-unwind-fan"));
}

// ═════════════════════════════ links ═════════════════════════════

fn hrefs(body: &str) -> Vec<(String, String)> {
    // (data-spread key of the enclosing row, raw href)
    let mut out = Vec::new();
    for n in 1..=5u32 {
        let (_, row) = spread_row(body, &n.to_string());
        for a in open_tags(&row, "a") {
            if let Some(h) = attr(&a, "href") {
                out.push((n.to_string(), h));
            }
        }
    }
    out
}

#[tokio::test]
async fn test_spread_links_should_be_the_canonical_page_url_with_years_n_and_every_param() {
    let q = "symbol=xyz&years=2&cadence=ANNUAL&growth=-2.5&mu=7.5&vol=45&limit=20&candidates=abc,xyz&buy=5000";
    let body = partial(&app_ca(), q).await;
    let links = hrefs(&body);
    let keys: Vec<&str> = links.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(
        keys,
        vec!["1", "3", "4", "5"],
        "current row 2 is not a link"
    );
    for (n, raw) in links {
        assert!(
            !raw.replace("&amp;", "").contains('&'),
            "separators must be HTML-escaped: {raw}"
        );
        assert_eq!(
            raw.replace("&amp;", "&"),
            format!(
                "{PAGE}?symbol=XYZ&years={n}&cadence=annual&growth=-2.5&mu=7.5&vol=45&limit=20&candidates=ABC%2CXYZ&buy=5000"
            )
        );
    }
}

#[tokio::test]
async fn test_spread_links_without_symbol_param_should_omit_it_and_keep_defaults() {
    let body = partial(&app_ca(), "").await;
    let links = hrefs(&body);
    // default years=3 is current
    assert_eq!(links.len(), 4, "{links:?}");
    for (n, raw) in links {
        assert_eq!(
            raw.replace("&amp;", "&"),
            format!("{PAGE}?years={n}&cadence=quarterly&growth=0&mu=8&vol=30")
        );
    }
}

#[tokio::test]
async fn test_spread_links_should_drop_invalid_overlap_params_like_the_replace_url() {
    let body = partial(&app_ca(), "years=2&limit=abc&candidates=A/B&buy=-5").await;
    for (n, raw) in hrefs(&body) {
        assert_eq!(
            raw.replace("&amp;", "&"),
            format!("{PAGE}?years={n}&cadence=quarterly&growth=0&mu=8&vol=30"),
            "{raw}"
        );
    }
}

// ═════════════════════════════ edge cases ═════════════════════════════

#[tokio::test]
async fn test_spread_section_should_be_absent_when_there_is_nothing_to_sell() {
    let app = app_ca();
    // Sanity: the held symbol has the section ...
    let held = partial(&app, "symbol=XYZ&years=3").await;
    assert!(held.contains(HEADING), "{held}");
    // ... and a symbol that is not held (empty state) has none.
    let none = partial(&app, "symbol=ZZZ&years=3").await;
    assert!(!none.contains("Compare spreads"), "{none}");
    assert!(!none.contains("data-spread"), "{none}");
}

#[tokio::test]
async fn test_spread_section_should_be_identical_for_identical_requests() {
    let app = app_ca();
    let a = partial(&app, "years=4&cadence=annual&vol=40").await;
    let b = partial(&app, "years=4&cadence=annual&vol=40").await;
    assert_eq!(table_html(&a), table_html(&b));
    assert!(a.contains(HEADING));
}

#[tokio::test]
async fn test_spread_vol_param_should_change_the_p10_columns() {
    let app = app_ca();
    let lo = partial(&app, "years=3&vol=10").await;
    let hi = partial(&app, "years=3&vol=60").await;
    assert_ne!(cell(&lo, "3", "p10"), cell(&hi, "3", "p10"));
    assert_ne!(cell(&lo, "hold", "p10"), cell(&hi, "hold", "p10"));
}

#[tokio::test]
async fn test_spread_links_for_a_punctuated_ticker_should_be_percent_encoded_and_html_escaped() {
    // Links built from a punctuated (but valid) ticker stay escaped and encoded.
    let app = app_with_profile(
        |r| {
            seed(r);
            insert_lot_dated(r, "lot-3", "A.B-C", 100.0, 10.0, "2022-01-03");
            insert_close(r, "A.B-C", "2026-10-24", 10.0);
        },
        Some(profile("CA", "single", 250_000.0)),
    );
    let body = partial(&app, "symbol=a.b-c&years=2").await;
    for (_, raw) in hrefs(&body) {
        assert!(raw.contains("symbol=A.B-C"), "{raw}");
        assert!(!raw.replace("&amp;", "").contains('&'), "{raw}");
        assert!(!raw.contains('<') && !raw.contains('"'), "{raw}");
    }
}

// ═════════════════════════════ the main panel is unchanged ═════════════════════════════

#[tokio::test]
async fn test_main_panel_should_render_as_before_with_the_spread_section_added() {
    // TX, annual, 3 years: three sales in three calendar years whatever today is.
    let body = partial(&app_tx(), "years=3&cadence=annual").await;

    // KPIs and the schedule table are the existing ones, with year_tax maths.
    let tax3 = annual_tx_tax(3);
    assert!((kpi(&body, "total-tax") - tax3).abs() <= 1.0);
    assert!((kpi(&body, "after-tax-proceeds") - (100_000.0 - tax3)).abs() <= 1.0);
    let rows = open_tags(&body, "tr")
        .iter()
        .filter_map(|t| attr(t, "data-period"))
        .collect::<Vec<_>>();
    assert_eq!(rows, vec!["1", "2", "3", "total"]);
    assert!(body.contains("<h3>Tax by calendar year</h3>"));
    assert!(body.contains("ga-svg-unwind-fan") && body.contains("ga-svg-unwind-tax-years"));

    // The average tax rate quoted in the assumptions is tax / proceeds.
    let rate = format!("average tax rate ({:.1}%)", tax3 / 100_000.0 * 100.0);
    assert!(body.contains(&rate), "missing {rate:?}");

    // The main projection equals the public maths with the fixed seed, the schedule's yearly
    // fractions (thirds) and its effective tax rate.
    let proj = ProjectionInput {
        stock_value: 100_000.0,
        other_value: 50_000.0,
        years: 3,
        stock_mu: 0.08,
        stock_sigma: 0.30,
        rest_mu: DEFAULT_REST_MU,
        rest_sigma: DEFAULT_REST_SIGMA,
        rho: DEFAULT_RHO,
        paths: DEFAULT_PATHS,
        sell_fractions: vec![1.0 / 3.0; 3],
        tax_rate: tax3 / 100_000.0,
    };
    let p = project(&proj, PROJECTION_SEED);
    assert!((kpi(&body, "p10-unwind") - p.unwind[3].p10).abs() <= 1.0);
    assert!((kpi(&body, "p10-hold") - p.hold[3].p10).abs() <= 1.0);

    // ... and the new section is there, after everything above it.
    let spread_at = body
        .find(HEADING)
        .unwrap_or_else(|| panic!("no {HEADING} in:\n{body}"));
    for earlier in [
        "data-kpi=\"total-tax\"",
        "<caption>",
        "ga-svg-unwind-fan",
        "<h3>Tax by calendar year</h3>",
    ] {
        assert!(body.find(earlier).unwrap() < spread_at, "{earlier}");
    }
}

#[tokio::test]
async fn test_main_panel_quarterly_with_the_section_should_keep_four_sales_a_year() {
    let body = partial(&app_ca(), "years=2&cadence=quarterly").await;
    let n = open_tags(&body, "tr")
        .iter()
        .filter_map(|t| attr(t, "data-period"))
        .filter(|p| p != "total")
        .count();
    assert_eq!(n, 8);
    assert!(body.contains("Illustrative assumptions"));
    assert!(body.contains(HEADING), "section missing from:\n{body}");
}

#[tokio::test]
async fn test_main_panel_replace_url_header_should_still_be_set_with_the_section() {
    let r = get_with(
        &app_ca(),
        &format!("{PARTIAL}?years=2&cadence=annual"),
        &[("HX-Request", "true"), ("HX-Trigger", "unwind-controls")],
    )
    .await;
    assert_eq!(
        r.header("hx-replace-url").as_deref(),
        Some("/portfolio/concentration?years=2&cadence=annual&growth=0&mu=8&vol=30")
    );
    assert!(r.text().contains(HEADING));
}

#[tokio::test]
async fn test_page_should_still_embed_only_the_placeholder_not_the_spread_section() {
    let r = get(&app_ca(), PAGE).await;
    assert_eq!(r.status, axum::http::StatusCode::OK);
    let body = r.text();
    assert!(body.contains("id=\"unwind\""));
    assert!(
        !body.contains(HEADING),
        "section is lazy-loaded via the partial"
    );
    // The partial is what carries it.
    assert!(partial(&app_ca(), "").await.contains(HEADING));
}
