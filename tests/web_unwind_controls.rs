//! RED-phase contract for Track B on `/portfolio/concentration`: the what-if controls form, URL sync
//! through `HX-Replace-Url`, and the tax-by-year panel. Compiles against the current API; fails on
//! assertions. Pure maths: `tests/unwind_years.rs`; chart: `tests/charts_tax_by_year.rs`.
//! Synthetic data only (ticker XYZ).
//!
//! Controls form (page only, never part of the swapped `#unwind` region)
//! * `<form id="unwind-controls" action="/portfolio/concentration" method="get"
//!   hx-get="/partials/unwind" hx-target="#unwind" hx-swap="outerHTML"
//!   hx-trigger="change delay:400ms, submit" hx-sync="this:replace">`
//! * lives inside `<section aria-labelledby="unwind-h">`, outside the `.unwind-live` div.
//! * `<fieldset>` with `<legend>`; every visible input/select has an `id` and a `<label for=id>`.
//! * fields by `name`: `years` (number min 1 max 10 step 1), `cadence` (select: quarterly|annual,
//!   the current one carries `selected`), `growth` and `mu` (number min -50 max 50 step 0.5),
//!   `vol` (number min 1 max 150 step 0.5); a submit button.
//! * values are the VALIDATED params, normalised like the placeholder (`f64::to_string`: "0", "8",
//!   "-2.5", "10" for "1e1").
//! * hidden inputs: `symbol` (upper-cased) only when supplied; `limit`/`candidates`/`buy` only when
//!   present in the request (normalised: "20", "ABC,XYZ", "5000").
//! * no form when: invalid default symbol, empty portfolio, symbol not held, DB error, 400.
//!
//! `HX-Replace-Url` on GET /partials/unwind: only on a 200 AND request headers `HX-Request: true`
//! AND `HX-Trigger: unwind-controls` (so the placeholder's load request never rewrites the URL) AND a real schedule (not the empty states). Value, in this exact order, percent-encoded:
//! `/portfolio/concentration?[symbol=S&]years=Y&cadence=C&growth=G&mu=M&vol=V[&limit=L][&candidates=A%2CB][&buy=B]`
//! `symbol` only if supplied (upper-cased); limit/candidates/buy only if present AND valid under the
//! page's own validation (an invalid one is dropped, it does not fail the partial).
//!
//! Tax-by-year panel in the partial (profile with a supported state only): `<h3>Tax by calendar
//! year</h3>`, an accessible chart with id suffix `unwind-tax-years` whose `<desc>` lists each
//! year's total, and a `<details>` table with headers Year, Federal, State, NIIT, Total and a last
//! "... % of proceeds" column; one row per calendar year of the schedule.

mod common;

use axum::http::StatusCode;
use axum::Router;
use common::web::*;
use ghost_alpha::storage::db::SqliteRepository;

const PAGE: &str = "/portfolio/concentration";
const PARTIAL: &str = "/partials/unwind";
/// A request fired by the controls form (htmx sends the id of the element that triggered it).
const HX: &[(&str, &str)] = &[("HX-Request", "true"), ("HX-Trigger", "unwind-controls")];
/// The placeholder's own `hx-trigger="load"` request.
const HX_LOAD: &[(&str, &str)] = &[("HX-Request", "true"), ("HX-Trigger", "unwind")];
const REPLACE_URL: &str = "hx-replace-url";

// XYZ 1000 sh @ 20 (long term), close 100 = 100,000; ABC 500 sh @ 100 = 50,000.
fn seed(repo: &SqliteRepository) {
    insert_lot_dated(repo, "lot-1", "XYZ", 1000.0, 20.0, "2022-01-03");
    insert_lot_dated(repo, "lot-2", "ABC", 500.0, 100.0, "2022-01-03");
    insert_close(repo, "XYZ", "2026-10-24", 100.0);
    insert_close(repo, "ABC", "2026-10-24", 100.0);
}

fn app() -> Router {
    app_with_profile(seed, None)
}

fn app_ca() -> Router {
    app_with_profile(seed, Some(profile("CA", "single", 250_000.0)))
}

fn app_bad_symbol() -> Router {
    app_with_profile(
        |r| {
            seed(r);
            insert_lot_dated(r, "lot-bad", "BRK/B", 1000.0, 1000.0, "2022-01-03");
        },
        None,
    )
}

fn broken_db_app() -> Router {
    app_with_profile(
        |r| {
            r.connection()
                .unwrap()
                .execute_batch("DROP TABLE tax_lots;")
                .unwrap();
        },
        None,
    )
}

// ───────────────────────────── html helpers ─────────────────────────────

/// Raw html between the open tag (as found in `html`) and its matching close tag.
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

fn form_tag(body: &str) -> Option<String> {
    open_tags(body, "form")
        .into_iter()
        .find(|t| attr(t, "id").as_deref() == Some("unwind-controls"))
}

fn form_tag_req(body: &str) -> String {
    form_tag(body).unwrap_or_else(|| panic!("no <form id=\"unwind-controls\"> in:\n{body}"))
}

/// Inner html of the controls form.
fn form_html(body: &str) -> String {
    raw_inner(body, &form_tag_req(body)).to_string()
}

fn field(form: &str, tag: &str, name: &str) -> Option<String> {
    open_tags(form, tag)
        .into_iter()
        .find(|t| attr(t, "name").as_deref() == Some(name))
}

fn input(form: &str, name: &str) -> String {
    field(form, "input", name).unwrap_or_else(|| panic!("no input name={name} in {form}"))
}

fn hidden(form: &str, name: &str) -> Option<String> {
    open_tags(form, "input")
        .into_iter()
        .find(|t| {
            attr(t, "name").as_deref() == Some(name) && attr(t, "type").as_deref() == Some("hidden")
        })
        .map(|t| attr(&t, "value").unwrap_or_default())
}

fn hidden_names(form: &str) -> Vec<String> {
    open_tags(form, "input")
        .into_iter()
        .filter(|t| attr(t, "type").as_deref() == Some("hidden"))
        .filter_map(|t| attr(&t, "name"))
        .collect()
}

fn unwind_live_html(body: &str) -> String {
    let tag = open_tags(body, "div")
        .into_iter()
        .find(|t| {
            attr(t, "class").is_some_and(|c| c.split_whitespace().any(|w| w == "unwind-live"))
        })
        .expect("unwind-live div");
    raw_inner(body, &tag).to_string()
}

async fn page(app: &Router, q: &str) -> String {
    let r = get(app, &format!("{PAGE}?{q}")).await;
    assert_eq!(r.status, StatusCode::OK, "{q}");
    r.text()
}

// ═════════════════════════════ controls form ═════════════════════════════

#[tokio::test]
async fn test_controls_form_should_carry_id_action_method_and_htmx_attributes() {
    let body = page(&app(), "").await;
    let f = form_tag_req(&body);
    assert_eq!(
        attr(&f, "action").as_deref(),
        Some("/portfolio/concentration")
    );
    assert_eq!(attr(&f, "method").as_deref(), Some("get"));
    assert_eq!(attr(&f, "hx-get").as_deref(), Some("/partials/unwind"));
    assert_eq!(attr(&f, "hx-target").as_deref(), Some("#unwind"));
    assert_eq!(attr(&f, "hx-swap").as_deref(), Some("outerHTML"));
    assert_eq!(
        attr(&f, "hx-trigger").as_deref(),
        Some("change delay:400ms, submit")
    );
    assert_eq!(attr(&f, "hx-sync").as_deref(), Some("this:replace"));
}

#[tokio::test]
async fn test_controls_form_should_sit_in_the_unwind_section_outside_the_live_region() {
    let body = page(&app(), "").await;
    let sec_open = r#"<section aria-labelledby="unwind-h">"#;
    let section = raw_inner(&body, sec_open);
    assert!(
        section.contains("id=\"unwind-controls\""),
        "form not in section"
    );
    assert!(section.contains("id=\"unwind\""), "region not in section");
    let live = unwind_live_html(&body);
    assert!(live.contains("id=\"unwind\""));
    assert!(
        !live.contains("unwind-controls"),
        "form must not be inside .unwind-live (it would be re-announced and lost on swap)"
    );
    // And it is not part of the swapped region either.
    let region = open_tags(&body, "div")
        .into_iter()
        .find(|t| attr(t, "id").as_deref() == Some("unwind"))
        .unwrap();
    assert!(!raw_inner(&body, &region).contains("unwind-controls"));
}

#[tokio::test]
async fn test_controls_form_should_have_fieldset_legend_and_exactly_one_form() {
    let body = page(&app(), "").await;
    let form = form_html(&body);
    assert!(form.contains("<fieldset"), "{form}");
    let legend = first_text(&form, "legend").expect("legend");
    assert!(!legend.trim().is_empty());
    assert!(
        open_tags(&body, "form")
            .iter()
            .filter(|t| attr(t, "id").as_deref() == Some("unwind-controls"))
            .count()
            == 1
    );
}

#[tokio::test]
async fn test_controls_form_every_visible_field_should_have_a_label_for_it() {
    let body = page(&app(), "").await;
    let form = form_html(&body);
    let labels: Vec<String> = open_tags(&form, "label")
        .iter()
        .filter_map(|t| attr(t, "for"))
        .collect();
    let mut checked = 0;
    for name in ["years", "growth", "mu", "vol"] {
        let id = attr(&input(&form, name), "id").unwrap_or_else(|| panic!("{name} has no id"));
        assert!(
            labels.contains(&id),
            "no <label for=\"{id}\"> for {name}: {form}"
        );
        checked += 1;
    }
    let sel = field(&form, "select", "cadence").expect("select cadence");
    let id = attr(&sel, "id").expect("cadence id");
    assert!(labels.contains(&id), "no label for cadence");
    checked += 1;
    assert_eq!(checked, 5);
    // No other visible inputs without labels.
    for t in open_tags(&form, "input") {
        let ty = attr(&t, "type").unwrap_or_default();
        if ty == "hidden" || ty == "submit" {
            continue;
        }
        let id = attr(&t, "id").unwrap_or_else(|| panic!("input without id: {t}"));
        assert!(labels.contains(&id), "unlabelled input {t}");
    }
    assert!(form.contains("<button") || form.contains("type=\"submit\""));
}

#[tokio::test]
async fn test_controls_form_inputs_should_carry_type_min_max_and_step() {
    let body = page(&app(), "").await;
    let form = form_html(&body);
    let expect = [
        ("years", "1", "10", "1"),
        ("growth", "-50", "50", "0.5"),
        ("mu", "-50", "50", "0.5"),
        ("vol", "1", "150", "0.5"),
    ];
    for (name, min, max, step) in expect {
        let t = input(&form, name);
        assert_eq!(attr(&t, "type").as_deref(), Some("number"), "{t}");
        assert_eq!(attr(&t, "min").as_deref(), Some(min), "{t}");
        assert_eq!(attr(&t, "max").as_deref(), Some(max), "{t}");
        assert_eq!(attr(&t, "step").as_deref(), Some(step), "{t}");
    }
    let sel_tag = field(&form, "select", "cadence").expect("select cadence");
    let sel = raw_inner(&form, &sel_tag).to_string();
    let values: Vec<String> = open_tags(&sel, "option")
        .iter()
        .filter_map(|t| attr(t, "value"))
        .collect();
    assert_eq!(values, vec!["quarterly", "annual"]);
}

fn assert_values(form: &str, years: &str, cadence: &str, growth: &str, mu: &str, vol: &str) {
    for (name, want) in [
        ("years", years),
        ("growth", growth),
        ("mu", mu),
        ("vol", vol),
    ] {
        assert_eq!(
            attr(&input(form, name), "value").as_deref(),
            Some(want),
            "{name} in {form}"
        );
    }
    let selected: Vec<String> = open_tags(form, "option")
        .into_iter()
        .filter(|t| t.contains(" selected"))
        .filter_map(|t| attr(&t, "value"))
        .collect();
    assert_eq!(selected, vec![cadence.to_string()], "{form}");
}

#[tokio::test]
async fn test_controls_form_without_params_should_show_the_defaults() {
    let body = page(&app(), "").await;
    assert_values(&form_html(&body), "3", "quarterly", "0", "8", "30");
}

#[tokio::test]
async fn test_controls_form_with_custom_params_should_reflect_the_validated_values() {
    let body = page(&app(), "years=5&cadence=annual&growth=-2.5&mu=7.5&vol=45").await;
    assert_values(&form_html(&body), "5", "annual", "-2.5", "7.5", "45");
}

#[tokio::test]
async fn test_controls_form_should_show_normalised_not_raw_values() {
    // "1e1" and "+5" are valid numbers; "ANNUAL" is accepted case-insensitively.
    let body = page(&app(), "growth=1e1&mu=%2B5&cadence=ANNUAL").await;
    assert_values(&form_html(&body), "3", "annual", "10", "5", "30");
}

#[tokio::test]
async fn test_controls_form_without_symbol_or_overlap_params_should_have_no_hidden_inputs() {
    let body = page(&app(), "years=4").await;
    let names = hidden_names(&form_html(&body));
    assert!(names.is_empty(), "unexpected hidden inputs: {names:?}");
}

#[tokio::test]
async fn test_controls_form_when_symbol_supplied_should_carry_it_upper_cased_in_a_hidden_input() {
    let body = page(&app(), "symbol=xyz").await;
    let form = form_html(&body);
    assert_eq!(hidden(&form, "symbol").as_deref(), Some("XYZ"), "{form}");
    for n in ["limit", "candidates", "buy"] {
        assert!(hidden(&form, n).is_none(), "{n} must be absent: {form}");
    }
}

#[tokio::test]
async fn test_controls_form_should_carry_present_overlap_params_as_hidden_inputs() {
    let body = page(&app(), "symbol=ABC&limit=20&candidates=abc,xyz&buy=5000").await;
    let form = form_html(&body);
    assert_eq!(hidden(&form, "symbol").as_deref(), Some("ABC"));
    assert_eq!(hidden(&form, "limit").as_deref(), Some("20"));
    assert_eq!(hidden(&form, "candidates").as_deref(), Some("ABC,XYZ"));
    assert_eq!(hidden(&form, "buy").as_deref(), Some("5000"));
}

#[tokio::test]
async fn test_controls_form_should_carry_only_the_overlap_params_that_were_present() {
    let body = page(&app(), "buy=2500").await;
    let form = form_html(&body);
    assert_eq!(hidden(&form, "buy").as_deref(), Some("2500"));
    assert!(hidden(&form, "limit").is_none());
    assert!(hidden(&form, "candidates").is_none());
    assert!(hidden(&form, "symbol").is_none());
}

/// The validators only admit letters, digits, '.' and '-' (no quote or angle bracket can be valid),
/// so the hostile-but-valid value is the punctuation-heavy ticker list. It must come back verbatim
/// inside a well-formed attribute, and no unvalidated raw input may leak into the form.
#[tokio::test]
async fn test_controls_form_hidden_values_should_be_well_formed_attributes() {
    let punct = app_with_profile(
        |r| {
            seed(r);
            insert_lot_dated(r, "lot-p", "A.B-C", 10.0, 10.0, "2022-01-03");
            insert_close(r, "A.B-C", "2026-10-24", 10.0);
        },
        None,
    );
    let body = page(&punct, "symbol=a.b-c&candidates=a.b,C-D,e").await;
    let form = form_html(&body);
    assert_eq!(hidden(&form, "symbol").as_deref(), Some("A.B-C"), "{form}");
    assert_eq!(hidden(&form, "candidates").as_deref(), Some("A.B,C-D,E"));
    // Every hidden tag closes its value quote exactly once.
    for t in open_tags(&form, "input") {
        assert_eq!(t.matches('"').count() % 2, 0, "unbalanced quotes: {t}");
    }
}

#[tokio::test]
async fn test_controls_form_should_be_absent_when_default_symbol_is_invalid() {
    let r = get(&app_bad_symbol(), PAGE).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(form_tag(&r.text()).is_none());
}

#[tokio::test]
async fn test_controls_form_should_be_absent_for_empty_portfolio() {
    let r = get(&app_with_profile(|_| {}, None), PAGE).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(form_tag(&r.text()).is_none());
}

#[tokio::test]
async fn test_controls_form_should_be_absent_when_symbol_is_not_held() {
    let r = get(&app(), &format!("{PAGE}?symbol=NOPE")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(form_tag(&r.text()).is_none());
}

#[tokio::test]
async fn test_controls_form_should_be_absent_on_db_error_and_on_400() {
    let r = get(&broken_db_app(), PAGE).await;
    assert_eq!(r.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(form_tag(&r.text()).is_none());
    let r = get(&app(), &format!("{PAGE}?years=0")).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(form_tag(&r.text()).is_none());
}

#[tokio::test]
async fn test_partial_should_not_contain_the_controls_form() {
    let r = get(&app(), &format!("{PARTIAL}?years=2")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(form_tag(&r.text()).is_none());
}

// ═════════════════════════════ HX-Replace-Url ═════════════════════════════

async fn replace_url(
    app: &Router,
    q: &str,
    headers: &[(&str, &str)],
) -> (StatusCode, Option<String>) {
    let r = get_with(app, &format!("{PARTIAL}?{q}"), headers).await;
    (r.status, r.header(REPLACE_URL))
}

#[tokio::test]
async fn test_replace_url_with_defaults_should_list_the_five_unwind_params_without_symbol() {
    let (s, h) = replace_url(&app(), "", HX).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        h.as_deref(),
        Some("/portfolio/concentration?years=3&cadence=quarterly&growth=0&mu=8&vol=30")
    );
}

#[tokio::test]
async fn test_replace_url_with_everything_supplied_should_be_the_canonical_query() {
    let q = "symbol=xyz&years=5&cadence=ANNUAL&growth=-2.5&mu=7.5&vol=45&limit=20&candidates=abc,xyz,abc&buy=5000";
    let (s, h) = replace_url(&app(), q, HX).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        h.as_deref(),
        Some("/portfolio/concentration?symbol=XYZ&years=5&cadence=annual&growth=-2.5&mu=7.5&vol=45&limit=20&candidates=ABC%2CXYZ&buy=5000")
    );
}

#[tokio::test]
async fn test_replace_url_should_percent_encode_values() {
    let (_, h) = replace_url(&app(), "candidates=brk.b,xyz-1", HX).await;
    let h = h.expect("header");
    assert!(h.contains("candidates=BRK.B%2CXYZ-1"), "{h}");
    assert!(!h.contains(','), "{h}");
}

#[tokio::test]
async fn test_replace_url_should_include_only_the_overlap_params_that_are_present() {
    let (_, h) = replace_url(&app(), "buy=2500", HX).await;
    assert_eq!(
        h.as_deref(),
        Some("/portfolio/concentration?years=3&cadence=quarterly&growth=0&mu=8&vol=30&buy=2500")
    );
}

#[tokio::test]
async fn test_replace_url_should_drop_invalid_overlap_params_but_still_succeed() {
    for q in [
        "limit=abc",
        "limit=0",
        "limit=101",
        "buy=0",
        "buy=-5",
        "buy=nan",
        "candidates=A/B",
        "candidates=A,B,C,D,E,F,G,H,I,J,K",
    ] {
        let (s, h) = replace_url(&app(), q, HX).await;
        assert_eq!(s, StatusCode::OK, "{q}");
        assert_eq!(
            h.as_deref(),
            Some("/portfolio/concentration?years=3&cadence=quarterly&growth=0&mu=8&vol=30"),
            "{q}"
        );
    }
}

#[tokio::test]
async fn test_replace_url_should_keep_valid_overlap_params_next_to_invalid_ones() {
    let (s, h) = replace_url(&app(), "limit=25&buy=-1", HX).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        h.as_deref(),
        Some("/portfolio/concentration?years=3&cadence=quarterly&growth=0&mu=8&vol=30&limit=25")
    );
}

#[tokio::test]
async fn test_replace_url_without_hx_request_header_should_be_absent() {
    let (s, h) = replace_url(&app(), "years=2", &[]).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h.is_none(), "{h:?}");
}

#[tokio::test]
async fn test_replace_url_on_400_should_be_absent() {
    for q in ["years=0", "cadence=monthly", "symbol=A%2FB", "vol=0"] {
        let (s, h) = replace_url(&app(), q, HX).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{q}");
        assert!(h.is_none(), "{q}: {h:?}");
    }
}

#[tokio::test]
async fn test_replace_url_on_empty_states_should_be_absent() {
    let (s, h) = replace_url(&app_with_profile(|_| {}, None), "", HX).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h.is_none(), "empty portfolio: {h:?}");

    let (s, h) = replace_url(&app(), "symbol=NOPE", HX).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h.is_none(), "not held: {h:?}");

    let (s, h) = replace_url(&app_bad_symbol(), "", HX).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h.is_none(), "invalid default symbol: {h:?}");
}

#[tokio::test]
async fn test_replace_url_on_db_error_should_be_absent() {
    let (s, h) = replace_url(&broken_db_app(), "years=1", HX).await;
    assert_eq!(s, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(h.is_none());
}

#[tokio::test]
async fn test_replace_url_should_not_change_the_response_body() {
    let a = get(&app(), &format!("{PARTIAL}?years=2")).await.text();
    let b = get_with(&app(), &format!("{PARTIAL}?years=2"), HX)
        .await
        .text();
    assert_eq!(a, b);
}

// ═════════════════════════════ tax-by-year panel ═════════════════════════════

fn num(s: &str) -> f64 {
    let t: String = s
        .chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-'))
        .collect();
    t.parse().unwrap_or_else(|_| panic!("no number in {s:?}"))
}

fn rows_of(table_html: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut rest = table_html;
    while let Some(i) = rest.find("<tr") {
        let a = &rest[i..];
        let e = a.find("</tr>").unwrap_or(a.len());
        let row = &a[..e];
        let cells: Vec<String> = row
            .split("<td")
            .skip(1)
            .map(|c| strip_tags(&format!("<td{c}")))
            .collect();
        if !cells.is_empty() {
            out.push(cells);
        }
        rest = &a[e..];
    }
    out
}

/// (period date year, proceeds, tax) per schedule period row.
fn period_rows(body: &str) -> Vec<(i32, f64, f64)> {
    let mut out = Vec::new();
    for t in open_tags(body, "tr") {
        let Some(p) = attr(&t, "data-period") else {
            continue;
        };
        if p == "total" {
            continue;
        }
        let start = body.find(&t).unwrap();
        let row = &body[start..start + body[start..].find("</tr>").unwrap()];
        let cell = |col: &str| {
            let c = format!("data-col=\"{col}\">");
            let i = row.find(&c).unwrap_or_else(|| panic!("no {col} in {row}")) + c.len();
            row[i..row[i..].find("</td>").unwrap() + i].to_string()
        };
        let year: i32 = cell("date")[..4].parse().unwrap();
        out.push((year, num(&cell("proceeds")), num(&cell("tax"))));
    }
    out
}

/// Html from the "Tax by calendar year" heading to the end of its first `<details>`.
fn tax_panel(body: &str) -> String {
    let at = body
        .find("Tax by calendar year")
        .unwrap_or_else(|| panic!("no Tax by calendar year heading in:\n{body}"));
    let s = body[..at]
        .rfind("<h3")
        .expect("<h3> before the heading text");
    let rest = &body[s..];
    let d = rest.find("<details").expect("<details> after heading");
    let e = rest[d..].find("</details>").expect("</details>") + d + 10;
    rest[..e].to_string()
}

#[tokio::test]
async fn test_tax_panel_should_have_heading_chart_and_details_table() {
    let body = get(&app_ca(), &format!("{PARTIAL}?years=3")).await.text();
    let panel = tax_panel(&body);
    assert!(panel.contains("<details"), "{panel}");
    assert!(panel.contains("<table"), "{panel}");
    let svg = svgs(&panel).into_iter().next().expect("chart svg in panel");
    assert!(svg.contains("role=\"img\""), "{svg}");
    assert!(svg.contains("ga-svg-unwind-tax-years"), "{svg}");
    assert!(svg.contains("<title"), "{svg}");
    assert!(svg.contains("ga-desc-unwind-tax-years"), "{svg}");
    assert!(!svg.contains("=\"#"), "hex colour in chart");
    assert!(svg.contains("var(--ga-series-1)"));
}

#[tokio::test]
async fn test_tax_panel_table_should_have_the_specified_columns_and_one_row_per_calendar_year() {
    let body = get(&app_ca(), &format!("{PARTIAL}?years=3")).await.text();
    let panel = tax_panel(&body);
    let heads: Vec<String> = panel
        .split("<th ")
        .skip(1)
        .map(|c| strip_tags(&format!("<th {c}")))
        .map(|s| s.split("</th>").next().unwrap_or("").trim().to_string())
        .collect();
    assert_eq!(heads.len(), 6, "{heads:?}");
    assert_eq!(&heads[..5], ["Year", "Federal", "State", "NIIT", "Total"]);
    let last = heads[5].to_lowercase();
    assert!(
        last.contains("tax") && last.contains("of proceeds"),
        "{heads:?}"
    );
    for t in open_tags(&panel, "th") {
        assert_eq!(attr(&t, "scope").as_deref(), Some("col"), "{t}");
    }

    let rows = rows_of(&panel);
    let mut years: Vec<i32> = period_rows(&body).iter().map(|r| r.0).collect();
    years.dedup();
    assert!(
        years.len() >= 3,
        "3 years of quarterly sales span at least 3 calendar years"
    );
    assert_eq!(rows.len(), years.len(), "{rows:?}");
    for (row, y) in rows.iter().zip(&years) {
        assert_eq!(row.len(), 6, "{row:?}");
        assert_eq!(row[0], y.to_string());
    }
}

#[tokio::test]
async fn test_tax_panel_table_total_should_equal_the_schedule_total_tax() {
    let body = get(&app_ca(), &format!("{PARTIAL}?years=3")).await.text();
    let rows = rows_of(&tax_panel(&body));
    let table_total: f64 = rows.iter().map(|r| num(&r[4])).sum();
    let periods = period_rows(&body);
    let schedule_total: f64 = periods.iter().map(|p| p.2).sum();
    assert!(schedule_total > 0.0);
    // Cells are rounded to whole dollars.
    let slack = (rows.len() + periods.len()) as f64;
    assert!(
        (table_total - schedule_total).abs() <= slack,
        "table {table_total} vs schedule {schedule_total}"
    );
    // The headline KPI agrees too.
    let kpi_tag = open_tags(&body, "div")
        .into_iter()
        .find(|t| attr(t, "data-kpi").as_deref() == Some("total-tax"))
        .expect("total-tax kpi");
    let kpi = num(&strip_tags(raw_inner(&body, &kpi_tag)));
    assert!(
        (table_total - kpi).abs() <= slack,
        "table {table_total} vs kpi {kpi}"
    );
}

#[tokio::test]
async fn test_tax_panel_each_row_should_split_into_components_and_match_its_periods() {
    let body = get(&app_ca(), &format!("{PARTIAL}?years=3")).await.text();
    let rows = rows_of(&tax_panel(&body));
    let periods = period_rows(&body);
    for row in &rows {
        let year: i32 = row[0].parse().unwrap();
        let (fed, state, niit, total) = (num(&row[1]), num(&row[2]), num(&row[3]), num(&row[4]));
        assert!((fed + state + niit - total).abs() <= 2.0, "{row:?}");
        let mine: Vec<_> = periods.iter().filter(|p| p.0 == year).collect();
        let tax: f64 = mine.iter().map(|p| p.2).sum();
        let proceeds: f64 = mine.iter().map(|p| p.1).sum();
        assert!(
            (total - tax).abs() <= mine.len() as f64 + 1.0,
            "{row:?} vs {tax}"
        );
        let pct = num(&row[5]);
        assert!(
            (pct - tax / proceeds * 100.0).abs() <= 0.2,
            "{row:?}: expected about {:.1}",
            tax / proceeds * 100.0
        );
    }
    // The fixture is rich enough that every component shows up somewhere.
    assert!(rows.iter().any(|r| num(&r[1]) > 0.0));
    assert!(rows.iter().any(|r| num(&r[2]) > 0.0));
    assert!(rows.iter().any(|r| num(&r[3]) > 0.0));
}

#[tokio::test]
async fn test_tax_panel_chart_desc_should_list_each_years_total() {
    let body = get(&app_ca(), &format!("{PARTIAL}?years=3")).await.text();
    let panel = tax_panel(&body);
    let svg = svgs(&panel).into_iter().next().expect("svg");
    let d = svg.find("<desc").expect("desc");
    let desc = strip_tags(&svg[d..svg[d..].find("</desc>").unwrap() + d]);
    for row in rows_of(&panel) {
        assert!(
            desc.contains(&row[0]),
            "year {} missing from desc: {desc}",
            row[0]
        );
        assert!(
            desc.contains(&row[4]),
            "total {} missing from desc: {desc}",
            row[4]
        );
    }
}

#[tokio::test]
async fn test_tax_panel_should_be_omitted_without_a_profile_state() {
    for app in [
        app(),
        app_with_profile(seed, Some(profile("ZZ", "single", 0.0))),
    ] {
        let r = get(&app, &format!("{PARTIAL}?years=3")).await;
        assert_eq!(r.status, StatusCode::OK);
        let body = r.text();
        assert!(!body.contains("Tax by calendar year"), "{body}");
        assert!(!body.contains("unwind-tax-years"), "{body}");
        // The rest of the panel is unaffected.
        assert!(body.contains("unwind-fan"));
    }
}

#[tokio::test]
async fn test_tax_panel_should_be_omitted_on_empty_states() {
    let body = get(
        &app_with_profile(|_| {}, Some(profile("CA", "single", 0.0))),
        PARTIAL,
    )
    .await
    .text();
    assert!(!body.contains("Tax by calendar year"));
}

// ═════════════════════════════ round 2 (review fixes) ═════════════════════════════

#[tokio::test]
async fn test_replace_url_when_fired_by_the_placeholder_load_should_be_absent() {
    let (s, h) = replace_url(&app(), "years=2", HX_LOAD).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h.is_none(), "{h:?}");
}

#[tokio::test]
async fn test_replace_url_with_hx_request_but_no_hx_trigger_should_be_absent() {
    let (s, h) = replace_url(&app(), "years=2", &[("HX-Request", "true")]).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h.is_none(), "{h:?}");
}

#[tokio::test]
async fn test_replace_url_with_trigger_but_no_hx_request_should_be_absent() {
    let (s, h) = replace_url(&app(), "years=2", &[("HX-Trigger", "unwind-controls")]).await;
    assert_eq!(s, StatusCode::OK);
    assert!(h.is_none(), "{h:?}");
}

#[tokio::test]
async fn test_controls_form_should_disable_native_validation_so_the_server_validates() {
    let body = page(&app(), "").await;
    let f = form_tag_req(&body);
    assert!(
        f.split_whitespace()
            .any(|w| w == "novalidate" || w == "novalidate>"),
        "form needs the novalidate attribute: {f}"
    );
}

#[tokio::test]
async fn test_controls_form_should_opt_out_of_the_htmx_history_cache() {
    let body = page(&app(), "").await;
    assert_eq!(
        attr(&form_tag_req(&body), "hx-history").as_deref(),
        Some("false")
    );
}

#[tokio::test]
async fn test_controls_form_inputs_should_carry_default_placeholders_whatever_the_value() {
    for q in ["", "years=5&growth=-2.5&mu=7.5&vol=45"] {
        let body = page(&app(), q).await;
        let form = form_html(&body);
        for (name, want) in [("years", "3"), ("growth", "0"), ("mu", "8"), ("vol", "30")] {
            assert_eq!(
                attr(&input(&form, name), "placeholder").as_deref(),
                Some(want),
                "{name} ({q})"
            );
        }
    }
}

fn help_text(form: &str, id: &str) -> String {
    let tag = all_tags(form)
        .into_iter()
        .find(|t| !t.starts_with("</") && attr(t, "id").as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no element with id={id} in the form: {form}"));
    strip_tags(raw_inner(form, &tag))
}

#[tokio::test]
async fn test_controls_form_growth_and_mu_should_point_at_distinct_help_text() {
    let body = page(&app(), "").await;
    let form = form_html(&body);
    let g = attr(&input(&form, "growth"), "aria-describedby");
    let m = attr(&input(&form, "mu"), "aria-describedby");
    assert_eq!(g.as_deref(), Some("unwind-growth-help"));
    assert_eq!(m.as_deref(), Some("unwind-mu-help"));
    let (gt, mt) = (
        help_text(&form, "unwind-growth-help"),
        help_text(&form, "unwind-mu-help"),
    );
    assert!(!gt.trim().is_empty() && !mt.trim().is_empty());
    assert_ne!(gt, mt);
}

// Error order must match main: buy is validated before candidates.
#[tokio::test]
async fn test_page_with_invalid_buy_and_candidates_should_report_the_buy_error_first() {
    let r = get(&app(), &format!("{PAGE}?buy=0&candidates=A/B")).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.text().contains("Invalid buy amount"), "{}", r.text());
}

#[tokio::test]
async fn test_etf_overlap_partial_with_invalid_buy_and_candidates_should_report_the_buy_error_first(
) {
    let r = get(&app(), "/partials/etf-overlap?buy=0&candidates=A/B").await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.text().contains("Invalid buy amount"), "{}", r.text());
}
