//! RED-phase contract for the L2b unwind panel on `/portfolio/concentration`:
//!   GET /portfolio/concentration   gains an `#unwind` region (lazy htmx load)
//!   GET /partials/unwind           fragment: schedule table, fan chart, KPIs
//! Pure maths live in `tests/unwind.rs`. Synthetic data only (ticker XYZ).
//!
//! Query parameters (page AND partial; invalid => HTTP 400, never reflected, no DB access):
//!   `symbol`   as on the page (A-Z0-9.- ; default = largest position)
//!   `years`    integer 1..=10, default 3
//!   `cadence`  `quarterly` | `annual` (case-insensitive), default `quarterly`
//!   `growth`   percent per year, finite, -50..=50, default 0     (price growth)
//!   `mu`       percent per year, finite, -50..=50, default 8     (stock expected return)
//!   `vol`      percent per year, finite, 1..=150, default 30     (stock volatility)
//! Unknown parameters (limit, candidates, buy, ...) are ignored by the partial.
//!
//! Page hooks
//! * `<div id="unwind" hx-get="/partials/unwind?symbol=..&amp;years=..&amp;cadence=..&amp;growth=..&amp;mu=..&amp;vol=.."
//!   hx-trigger="load" aria-busy="true">`: normalised values, percent-encoded then HTML-escaped; a
//!   persistent `aria-live="polite"` ANCESTOR of `#unwind` (not the swapped element); when the default
//!   symbol is invalid the region has no `hx-get`/`hx-trigger` (same rule as `#etf-overlap`).
//! * A linked script (not htmx.min.js) handles `htmx:beforeSwap` + `shouldSwap` for `unwind` too.
//!
//! Partial hooks (the partial replaces `#unwind`; every response, 4xx/5xx included, is wrapped in
//! `<div id="unwind">`; errors carry `role="alert"`)
//! * schedule `<table>` with `<caption>`, `<th scope="col">`, one `<tr data-period="1">`.. per period
//!   plus a totals row `<tr data-period="total">`; cells `<td data-col="date|shares|proceeds|tax|net|weight-after">`
//!   (money as `$12,345`, weight as `50.0%`). The `tax` column is absent when no tax settings apply.
//! * Tax settings come from `config.profile`: state (required), filing (`single`|`mfj`|`mfs`|`hoh`,
//!   default single), other_income (default 0). No/unknown state => no tax column and a hint naming
//!   `ghost-alpha setup --section profile`.
//! * KPIs `data-kpi="total-tax|after-tax-proceeds|p10-hold|p10-unwind"` (first `$` amount in the text);
//!   total-tax only with tax settings. p10-* are the final-year p10 portfolio values.
//! * fan chart `<svg role="img">` with `<title>` and `<desc>` (mentions p10, p50, p90, hold, unwind),
//!   a dashed hold-median overlay (`stroke-dasharray`), "Start" as the first x label, no hex colours,
//!   plus a `<details>` holding a `<table>`.
//! * text: "Illustrative assumptions", "Not tax or investment advice", the seed, and rho 0.6.
//! * Not held / empty portfolio: HTTP 200 with an `empty-state` element inside `#unwind`.
//! * Same inputs => byte-identical body (seeded Monte Carlo).

mod common;

use axum::http::StatusCode;
use axum::Router;
use common::web::*;
use ghost_alpha::portfolio::types::{AcquisitionType, TaxLot};
use ghost_alpha::storage::db::SqliteRepository;
use ghost_alpha::tax::brackets::{state_tax_rate, FederalCapGainsBrackets};
use ghost_alpha::tax::engine::compute_lot_tax;

const PAGE: &str = "/portfolio/concentration";
const PARTIAL: &str = "/partials/unwind";

// XYZ 1000 sh @ 20 (acquired 2022-01-03, long term), close 100  = 100,000
// ABC  500 sh @ 100, close 100                                   =  50,000
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

// ───────────────────────────── html helpers ─────────────────────────────

fn inner_text(html: &str, open_tag: &str) -> String {
    let start = html.find(open_tag).expect("open tag in html") + open_tag.len();
    let name: String = open_tag[1..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    let (open, close) = (format!("<{name}"), format!("</{name}>"));
    let mut depth = 1;
    let mut i = start;
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
                    return strip_tags(&html[start..i + next_close]);
                }
                i += next_close + close.len();
            }
        }
    }
}

fn tag_with(html: &str, attr_name: &str, val: &str) -> Option<String> {
    all_tags(html)
        .into_iter()
        .find(|t| attr(t, attr_name).as_deref() == Some(val))
}

fn kpi(html: &str, name: &str) -> String {
    let tag = tag_with(html, "data-kpi", name)
        .unwrap_or_else(|| panic!("no data-kpi=\"{name}\" element in:\n{html}"));
    inner_text(html, &tag)
}

fn kpi_opt(html: &str, name: &str) -> Option<String> {
    tag_with(html, "data-kpi", name).map(|t| inner_text(html, &t))
}

/// First `$` amount (or, without a `$`, the first number) in `s`.
fn num(s: &str) -> f64 {
    let s = s.find('$').map(|i| &s[i + 1..]).unwrap_or(s);
    let t: String = s
        .chars()
        .skip_while(|c| !(c.is_ascii_digit() || *c == '-'))
        .take_while(|c| c.is_ascii_digit() || matches!(c, ',' | '.' | '-'))
        .filter(|c| *c != ',')
        .collect();
    t.parse().unwrap_or_else(|_| panic!("no number in {s:?}"))
}

fn period_row(html: &str, period: &str) -> Option<String> {
    let tag = open_tags(html, "tr")
        .into_iter()
        .find(|t| attr(t, "data-period").as_deref() == Some(period))?;
    let s = html.find(&tag)?;
    let e = html[s..].find("</tr>")? + s + 5;
    Some(html[s..e].to_string())
}

fn cell(row: &str, col: &str) -> Option<String> {
    let tag = open_tags(row, "td")
        .into_iter()
        .find(|t| attr(t, "data-col").as_deref() == Some(col))?;
    Some(inner_text(row, &tag))
}

fn period_count(html: &str) -> usize {
    open_tags(html, "tr")
        .iter()
        .filter_map(|t| attr(t, "data-period"))
        .filter(|p| p != "total")
        .count()
}

fn unwind_tag(body: &str) -> String {
    open_tags(body, "div")
        .into_iter()
        .find(|t| attr(t, "id").as_deref() == Some("unwind"))
        .unwrap_or_else(|| panic!("<div id=\"unwind\"> missing in:\n{body}"))
}

fn placeholder(body: &str) -> (String, Vec<(String, String)>) {
    let tag = unwind_tag(body);
    assert_eq!(attr(&tag, "hx-trigger").as_deref(), Some("load"), "{tag}");
    assert_eq!(attr(&tag, "aria-busy").as_deref(), Some("true"), "{tag}");
    let raw = attr(&tag, "hx-get").expect("hx-get");
    let (p, q) = raw.split_once('?').expect("query string");
    assert!(
        !q.replace("&amp;", "").contains('&'),
        "separators must be HTML-escaped: {raw}"
    );
    let params = q
        .replace("&amp;", "&")
        .split('&')
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k.to_string(), v.replace("%2C", ",").replace("%2c", ","))
        })
        .collect();
    (p.to_string(), params)
}

fn param<'a>(params: &'a [(String, String)], k: &str) -> &'a str {
    params
        .iter()
        .find(|(n, _)| n == k)
        .map(|(_, v)| v.as_str())
        .unwrap_or_else(|| panic!("param {k} missing in {params:?}"))
}

fn hex_literals(html: &str) -> Vec<String> {
    let b = html.as_bytes();
    let mut out = Vec::new();
    for i in 0..b.len() {
        if b[i] != b'#' || i == 0 {
            continue;
        }
        let prev = b[i - 1] as char;
        let ctx_ok = match prev {
            '"' | '\'' | ':' => true,
            ' ' => html[..i].trim_end().ends_with(':'),
            _ => false,
        };
        if !ctx_ok {
            continue;
        }
        let before = &html[i.saturating_sub(14)..i];
        if ["href=", "hx-", "for=", "url(", "aria-", "id="]
            .iter()
            .any(|k| before.contains(k))
        {
            continue;
        }
        let run: String = html[i + 1..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        let next = html[i + 1 + run.len()..].chars().next();
        let boundary = next.is_none_or(|c| !(c.is_alphanumeric() || c == '-' || c == '_'));
        if matches!(run.len(), 3 | 4 | 6 | 8) && boundary {
            out.push(format!("#{run}"));
        }
    }
    out
}

fn partial(q: &str) -> String {
    format!("{PARTIAL}?{q}")
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

// ═════════════════════════════ page: lazy region ═════════════════════════════

#[tokio::test]
async fn test_page_unwind_placeholder_when_no_params_should_carry_defaults() {
    let body = get(&app(), PAGE).await.text();
    let (p, params) = placeholder(&body);
    assert_eq!(p, PARTIAL);
    assert_eq!(param(&params, "symbol"), "XYZ");
    assert_eq!(num(param(&params, "years")), 3.0);
    assert_eq!(param(&params, "cadence"), "quarterly");
    assert_eq!(num(param(&params, "growth")), 0.0);
    assert_eq!(num(param(&params, "mu")), 8.0);
    assert_eq!(num(param(&params, "vol")), 30.0);
}

#[tokio::test]
async fn test_page_unwind_placeholder_when_params_given_should_carry_normalised_values() {
    let body = get(
        &app(),
        &format!("{PAGE}?symbol=xyz&years=5&cadence=ANNUAL&growth=-2.5&mu=7&vol=25"),
    )
    .await
    .text();
    let (_, params) = placeholder(&body);
    assert_eq!(param(&params, "symbol"), "XYZ");
    assert_eq!(num(param(&params, "years")), 5.0);
    assert_eq!(param(&params, "cadence"), "annual");
    assert_eq!(num(param(&params, "growth")), -2.5);
    assert_eq!(num(param(&params, "mu")), 7.0);
    assert_eq!(num(param(&params, "vol")), 25.0);
}

#[tokio::test]
async fn test_page_unwind_region_should_sit_inside_a_persistent_live_region() {
    let body = get(&app(), PAGE).await.text();
    let region = unwind_tag(&body);
    assert!(
        attr(&region, "aria-live").is_none(),
        "swapped element must not be the live region"
    );
    // Some aria-live="polite" element must contain #unwind.
    let found = all_tags(&body)
        .into_iter()
        .filter(|t| attr(t, "aria-live").as_deref() == Some("polite"))
        .any(|t| {
            let start = body.find(&t).unwrap() + t.len();
            let name: String = t[1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            let end = body[start..]
                .find(&format!("</{name}>"))
                .map_or(body.len(), |e| e + start);
            body[start..end].contains("id=\"unwind\"")
        });
    assert!(found, "no aria-live=\"polite\" ancestor of #unwind");
}

#[tokio::test]
async fn test_page_when_default_symbol_invalid_should_leave_unwind_without_hx_get() {
    let app = app_with_profile(
        |r| {
            seed(r);
            insert_lot_dated(r, "lot-bad", "BRK/B", 1000.0, 1000.0, "2022-01-03");
        },
        None,
    );
    let r = get(&app, PAGE).await;
    assert_eq!(r.status, StatusCode::OK);
    let body = r.text();
    let tag = unwind_tag(&body);
    assert!(
        attr(&tag, "hx-get").is_none() && attr(&tag, "hx-trigger").is_none(),
        "{tag}"
    );
    assert!(!body.contains("/partials/unwind"));
}

#[tokio::test]
async fn test_page_when_unwind_params_invalid_should_return_400_without_reflecting_input() {
    let cases = [
        "years=0",
        "years=11",
        "years=abc",
        "years=2.5",
        "years=-1",
        "cadence=monthly",
        "cadence=%3Cscript%3Ealert(1)%3C/script%3E",
        "growth=51",
        "growth=-51",
        "growth=abc",
        "growth=nan",
        "growth=inf",
        "mu=50.5",
        "mu=-60",
        "vol=0",
        "vol=0.5",
        "vol=151",
        "vol=abc",
    ];
    for q in cases {
        let r = get(&app(), &format!("{PAGE}?{q}")).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{q}");
        assert!(!r.text().contains("<script>alert"), "{q}");
    }
}

#[tokio::test]
async fn test_page_unwind_params_at_the_bounds_should_be_accepted() {
    for q in [
        "years=1&growth=-50&mu=-50&vol=1",
        "years=10&growth=50&mu=50&vol=150&cadence=annual",
    ] {
        let r = get(&app(), &format!("{PAGE}?{q}")).await;
        assert_eq!(r.status, StatusCode::OK, "{q}");
        let _ = placeholder(&r.text());
    }
}

#[tokio::test]
async fn test_static_js_should_let_htmx_swap_4xx_responses_for_the_unwind_region() {
    let body = get(&app(), PAGE).await.text();
    let mut found = false;
    for url in static_urls(&body) {
        let name = url
            .trim_start_matches("/static/")
            .split('?')
            .next()
            .unwrap()
            .to_string();
        if !name.ends_with(".js") || name.contains("htmx") {
            continue;
        }
        let f = ghost_alpha::web::assets::StaticAssets::get(&name).unwrap();
        let js = String::from_utf8_lossy(&f.data).into_owned();
        if js.contains("htmx:beforeSwap") && js.contains("shouldSwap") && js.contains("unwind") {
            found = true;
        }
    }
    assert!(
        found,
        "a linked script must handle htmx:beforeSwap for #unwind"
    );
}

// ═════════════════════════════ partial: schedule ═════════════════════════════

#[tokio::test]
async fn test_partial_without_profile_should_list_periods_without_tax_and_show_profile_hint() {
    let r = get(&app(), &partial("years=1&cadence=quarterly")).await;
    assert_eq!(r.status, StatusCode::OK);
    let body = r.text();
    unwind_tag(&body);
    assert_eq!(period_count(&body), 4);
    for k in 1..=4 {
        let row = period_row(&body, &k.to_string()).unwrap();
        assert_eq!(num(&cell(&row, "shares").unwrap()), 250.0);
        assert_eq!(num(&cell(&row, "proceeds").unwrap()), 25_000.0);
        assert!(
            cell(&row, "tax").is_none(),
            "no tax column without settings: {row}"
        );
    }
    assert!(body.contains("ghost-alpha setup --section profile"));
    assert!(kpi_opt(&body, "total-tax").is_none());
    let total = period_row(&body, "total").expect("totals row");
    assert_eq!(num(&cell(&total, "proceeds").unwrap()), 100_000.0);
}

#[tokio::test]
async fn test_partial_with_unknown_state_should_behave_like_no_profile() {
    let app = app_with_profile(seed, Some(profile("ZZ", "single", 0.0)));
    let body = get(&app, &partial("years=1")).await.text();
    assert!(body.contains("ghost-alpha setup --section profile"));
    assert!(period_row(&body, "1")
        .unwrap()
        .find("data-col=\"tax\"")
        .is_none());
}

#[tokio::test]
async fn test_partial_period_count_should_follow_years_and_cadence() {
    for (q, n) in [
        ("years=3&cadence=quarterly", 12),
        ("years=3&cadence=annual", 3),
        ("years=10&cadence=annual", 10),
        ("", 12),
    ] {
        let body = get(&app(), &partial(q)).await.text();
        assert_eq!(period_count(&body), n, "{q}");
    }
}

#[tokio::test]
async fn test_partial_with_profile_should_tax_each_sale_like_compute_lot_tax() {
    let body = get(&app_tx(), &partial("years=1&cadence=quarterly"))
        .await
        .text();
    assert!(!body.contains("ghost-alpha setup --section profile"));
    let lot = TaxLot {
        id: uuid::Uuid::from_u128(1),
        ticker: "XYZ".into(),
        shares: 1000.0,
        cost_basis_per_share: 20.0,
        acquired_date: chrono::NaiveDate::from_ymd_opt(2022, 1, 3).unwrap(),
        acquisition_type: AcquisitionType::Buy,
        remaining_shares: 1000.0,
    };
    let b = FederalCapGainsBrackets::single_2024();
    let st = state_tax_rate("TX").unwrap();
    let per_quarter = compute_lot_tax(
        &lot,
        250.0,
        100.0,
        chrono::Utc::now().date_naive(),
        80_000.0,
        &b,
        &st,
        0.0,
    )
    .total_tax;
    assert!(per_quarter > 0.0);
    for k in 1..=4 {
        let row = period_row(&body, &k.to_string()).unwrap();
        let tax = num(&cell(&row, "tax").unwrap());
        assert!(
            (tax - per_quarter).abs() <= 1.0,
            "q{k}: {tax} vs {per_quarter}"
        );
        let net = num(&cell(&row, "net").unwrap());
        assert!((net - (25_000.0 - tax)).abs() <= 1.5, "q{k}: net {net}");
    }
    let total = period_row(&body, "total").unwrap();
    assert!((num(&cell(&total, "tax").unwrap()) - 4.0 * per_quarter).abs() <= 3.0);
}

#[tokio::test]
async fn test_partial_totals_row_should_match_sum_of_period_rows() {
    let body = get(&app_tx(), &partial("years=2&cadence=quarterly"))
        .await
        .text();
    let n = period_count(&body);
    assert_eq!(n, 8);
    let total = period_row(&body, "total").unwrap();
    for col in ["shares", "proceeds", "tax", "net"] {
        let sum: f64 = (1..=n)
            .map(|k| num(&cell(&period_row(&body, &k.to_string()).unwrap(), col).unwrap()))
            .sum();
        let t = num(&cell(&total, col).unwrap());
        assert!(
            (sum - t).abs() <= n as f64,
            "{col}: rows {sum} vs total {t}"
        );
    }
}

#[tokio::test]
async fn test_partial_when_growth_given_should_compound_price_per_year() {
    let body = get(&app(), &partial("years=2&cadence=annual&growth=10"))
        .await
        .text();
    let p1 = num(&cell(&period_row(&body, "1").unwrap(), "proceeds").unwrap());
    let p2 = num(&cell(&period_row(&body, "2").unwrap(), "proceeds").unwrap());
    assert_eq!(p1, 50_000.0);
    assert_eq!(p2, 55_000.0);
}

#[tokio::test]
async fn test_partial_weight_after_should_fall_to_zero() {
    // 4 quarters, no tax: 750 sh * 100 = 75,000 vs rest 50,000 + 25,000 => 50.0% after Q1.
    let body = get(&app(), &partial("years=1&cadence=quarterly"))
        .await
        .text();
    let w = |k: &str| cell(&period_row(&body, k).unwrap(), "weight-after").unwrap();
    assert_eq!(w("1"), "50.0%");
    assert_eq!(w("4"), "0.0%");
}

#[tokio::test]
async fn test_partial_when_symbol_lowercase_should_work() {
    let body = get(&app(), &partial("symbol=xyz&years=1&cadence=annual"))
        .await
        .text();
    assert_eq!(period_count(&body), 1);
}

#[tokio::test]
async fn test_partial_table_should_have_caption_and_column_scopes() {
    let body = get(&app_tx(), &partial("years=1")).await.text();
    assert!(body.contains("<caption"));
    let ths = open_tags(&body, "th");
    assert!(!ths.is_empty());
    assert!(
        ths.iter()
            .all(|t| attr(t, "scope").as_deref() == Some("col")),
        "{ths:?}"
    );
}

// ═════════════════════════════ partial: KPIs and chart ═════════════════════════════

#[tokio::test]
async fn test_partial_kpis_should_report_tax_and_after_tax_proceeds() {
    let body = get(&app_tx(), &partial("years=1&cadence=quarterly"))
        .await
        .text();
    let tax = num(&kpi(&body, "total-tax"));
    let net = num(&kpi(&body, "after-tax-proceeds"));
    assert!(tax > 0.0);
    assert!((net - (100_000.0 - tax)).abs() <= 3.0, "{net} vs {tax}");
    let total = period_row(&body, "total").unwrap();
    assert!((num(&cell(&total, "tax").unwrap()) - tax).abs() <= 1.5);
}

#[tokio::test]
async fn test_partial_with_high_vol_unwind_p10_should_beat_hold_p10() {
    let body = get(&app(), &partial("years=5&cadence=annual&vol=100"))
        .await
        .text();
    let (hold, unwind) = (num(&kpi(&body, "p10-hold")), num(&kpi(&body, "p10-unwind")));
    assert!(unwind > hold, "unwind p10 {unwind} vs hold p10 {hold}");
}

#[tokio::test]
async fn test_partial_same_inputs_should_give_identical_output() {
    let q = partial("years=4&cadence=annual&vol=40&mu=6");
    let a = get(&app_tx(), &q).await.text();
    let b = get(&app_tx(), &q).await.text();
    assert!(period_count(&a) > 0, "{a}");
    assert_eq!(a, b);
}

#[tokio::test]
async fn test_partial_vol_param_should_change_the_projection() {
    let lo = get(&app(), &partial("years=5&cadence=annual&vol=10"))
        .await
        .text();
    let hi = get(&app(), &partial("years=5&cadence=annual&vol=100"))
        .await
        .text();
    assert_ne!(num(&kpi(&lo, "p10-hold")), num(&kpi(&hi, "p10-hold")));
}

#[tokio::test]
async fn test_partial_fan_chart_should_be_accessible_with_dashed_hold_overlay_and_start_label() {
    let body = get(&app_tx(), &partial("years=3&cadence=annual"))
        .await
        .text();
    let svg = svgs(&body).into_iter().next().expect("fan chart svg");
    assert!(svg.contains("role=\"img\""), "{svg}");
    assert!(svg.contains("<title"), "{svg}");
    let d = svg.find("<desc").expect("<desc>");
    let desc = &svg[d..svg[d..].find("</desc>").unwrap() + d].to_lowercase();
    for w in ["p10", "p50", "p90", "hold", "unwind"] {
        assert!(desc.contains(w), "desc lacks {w}: {desc}");
    }
    assert!(
        svg.contains("stroke-dasharray"),
        "hold-median overlay must be dashed"
    );
    assert!(svg.contains(">Start<") && !svg.contains("Q0"), "{svg}");
    let det = body.find("<details").expect("<details> table view");
    assert!(body[det..].contains("<table"));
    assert_eq!(hex_literals(&body), Vec::<String>::new());
}

#[tokio::test]
async fn test_partial_should_state_assumptions_and_disclaimer() {
    let body = get(&app_tx(), &partial("")).await.text();
    assert!(body.contains("Illustrative assumptions"));
    assert!(body.contains("Not tax or investment advice"));
    assert!(body.to_lowercase().contains("seed"));
    assert!(body.contains("0.6"), "correlation assumption");
}

// ═════════════════════════════ partial: errors and empty states ═════════════════════════════

#[tokio::test]
async fn test_partial_when_params_invalid_should_return_wrapped_400() {
    for q in [
        "years=0",
        "years=11",
        "cadence=%3Cscript%3Ealert(1)%3C/script%3E",
        "growth=99",
        "mu=abc",
        "vol=0",
        "symbol=%3Cb%3E",
    ] {
        let r = get(&app(), &partial(q)).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{q}");
        let body = r.text();
        unwind_tag(&body);
        assert!(body.contains("role=\"alert\""), "{q}: {body}");
        assert!(
            !body.contains("<script>alert") && !body.contains("<b>"),
            "{q}: {body}"
        );
    }
}

#[tokio::test]
async fn test_partial_validation_should_happen_before_any_database_access() {
    let r = get(&broken_db_app(), &partial("years=0")).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_partial_when_db_fails_should_return_wrapped_500_alert() {
    let r = get(&broken_db_app(), &partial("years=1")).await;
    assert_eq!(r.status, StatusCode::INTERNAL_SERVER_ERROR);
    let body = r.text();
    unwind_tag(&body);
    assert!(
        body.contains("role=\"alert\"") && !body.contains("empty-state"),
        "{body}"
    );
}

#[tokio::test]
async fn test_partial_when_portfolio_empty_should_show_empty_state_in_region() {
    let app = app_with_profile(|_| {}, None);
    let r = get(&app, &partial("")).await;
    assert_eq!(r.status, StatusCode::OK);
    let body = r.text();
    unwind_tag(&body);
    assert!(body.contains("empty-state"));
    assert_eq!(period_count(&body), 0);
}

#[tokio::test]
async fn test_partial_when_symbol_not_held_should_show_empty_state_in_region() {
    let r = get(&app(), &partial("symbol=NOPE")).await;
    assert_eq!(r.status, StatusCode::OK);
    let body = r.text();
    unwind_tag(&body);
    assert!(body.contains("empty-state"));
    assert_eq!(period_count(&body), 0);
}

// ═════════════════════════════ chart helper ═════════════════════════════
// `web::charts::fan_chart_svg_with_overlay(percentiles, overlay, width, height) -> String` is new;
// `fan_chart_svg(percentiles, width, height)` keeps its signature and delegates with `None`.
// `overlay` has one value per FanChartYear (the hold median) and is drawn as a dashed line.

fn years() -> Vec<ghost_alpha::web::charts::FanChartYear> {
    use ghost_alpha::web::charts::FanChartYear;
    (0..4)
        .map(|y| {
            let m = 100_000.0 * 1.07_f64.powi(y);
            FanChartYear {
                year: y as u32,
                p10: m * 0.7,
                p25: m * 0.85,
                p50: m,
                p75: m * 1.15,
                p90: m * 1.3,
            }
        })
        .collect()
}

#[test]
fn test_fan_chart_with_overlay_should_draw_a_dashed_line_and_label_the_start() {
    use ghost_alpha::web::charts::fan_chart_svg_with_overlay;
    let overlay = [100_000.0, 105_000.0, 108_000.0, 112_000.0];
    let svg = fan_chart_svg_with_overlay(&years(), Some(&overlay), 600, 300);
    assert!(svg.contains("stroke-dasharray"), "{svg}");
    assert!(svg.contains(">Start<") && !svg.contains("Q0"), "{svg}");
    assert_eq!(hex_literals(&svg), Vec::<String>::new());
}

#[test]
fn test_fan_chart_without_overlay_should_have_no_dashed_line() {
    use ghost_alpha::web::charts::{fan_chart_svg, fan_chart_svg_with_overlay};
    let a = fan_chart_svg_with_overlay(&years(), None, 600, 300);
    assert!(!a.contains("stroke-dasharray"));
    assert_eq!(a, fan_chart_svg(&years(), 600, 300));
    assert_eq!(hex_literals(&a), Vec::<String>::new());
}

// ═════════════════════════ review round: disclosures (m1, m3, m4) and nits ═════════════════════════

fn profile_with(state: Option<&str>, filing: Option<&str>) -> ghost_alpha::config::ProfileConfig {
    ghost_alpha::config::ProfileConfig {
        state: state.map(str::to_string),
        filing: filing.map(str::to_string),
        other_income: Some(80_000.0),
    }
}

async fn body_for(p: ghost_alpha::config::ProfileConfig) -> String {
    let r = get(&app_with_profile(seed, Some(p)), &partial("years=1")).await;
    assert_eq!(r.status, StatusCode::OK);
    r.text()
}

#[tokio::test]
async fn test_partial_should_disclose_bracket_year_and_filing_status() {
    for (filing, label) in [
        ("single", "single"),
        ("mfj", "married filing jointly"),
        ("mfs", "married filing separately"),
        ("hoh", "head of household"),
    ] {
        let low = body_for(profile_with(Some("TX"), Some(filing)))
            .await
            .to_lowercase();
        assert!(low.contains("2024"), "{filing}: bracket year");
        assert!(low.contains(label), "{filing}: status label {label:?}");
    }
}

#[tokio::test]
async fn test_partial_should_note_when_mfs_or_hoh_use_the_single_schedule() {
    for filing in ["mfs", "hoh"] {
        let low = body_for(profile_with(Some("TX"), Some(filing)))
            .await
            .to_lowercase();
        assert!(low.contains("single schedule"), "{filing}");
    }
    for filing in ["single", "mfj"] {
        let low = body_for(profile_with(Some("TX"), Some(filing)))
            .await
            .to_lowercase();
        assert!(
            !low.contains("single schedule"),
            "{filing}: no fallback note expected"
        );
    }
}

#[tokio::test]
async fn test_partial_when_filing_unparseable_should_say_so_and_escape_it() {
    let body = body_for(profile_with(Some("TX"), Some("<b>x</b>"))).await;
    let low = body.to_lowercase();
    assert!(
        low.contains("not recognised") && low.contains("single schedule"),
        "{body}"
    );
    assert!(!body.contains("<b>x</b>"), "raw filing value echoed");
}

#[tokio::test]
async fn test_partial_when_filing_missing_should_default_to_single_without_a_warning() {
    let low = body_for(profile_with(Some("TX"), None))
        .await
        .to_lowercase();
    assert!(!low.contains("not recognised"));
    assert!(low.contains("2024"));
}

#[tokio::test]
async fn test_partial_should_state_hold_values_are_before_tax_and_unwind_uses_average_tax_rate() {
    let low = get(&app_tx(), &partial("years=2"))
        .await
        .text()
        .to_lowercase();
    assert!(
        low.contains("before tax"),
        "hold values are before tax due on a later sale"
    );
    assert!(
        low.contains("average tax rate"),
        "unwind proceeds use the schedule's average rate"
    );
    let low = get(&app(), &partial("years=2")).await.text().to_lowercase();
    assert!(low.contains("before tax") && low.contains("average tax rate"));
}

#[tokio::test]
async fn test_partial_when_state_unsupported_should_say_not_supported_and_escape_it() {
    let body = body_for(profile_with(Some("ZZ"), Some("single"))).await;
    let low = body.to_lowercase();
    assert!(low.contains("not supported"), "{body}");
    assert!(!low.contains("no state is set"), "{body}");
    assert!(period_row(&body, "1")
        .unwrap()
        .find("data-col=\"tax\"")
        .is_none());
    let hostile = body_for(profile_with(Some("<b>"), Some("single"))).await;
    assert!(hostile.to_lowercase().contains("not supported"));
    assert!(!hostile.contains("<b>"), "raw state echoed");
}

#[tokio::test]
async fn test_partial_when_no_state_set_should_keep_the_no_state_hint() {
    let low = body_for(profile_with(None, Some("single")))
        .await
        .to_lowercase();
    assert!(low.contains("no state is set"));
    assert!(!low.contains("not supported"));
}

#[tokio::test]
async fn test_partial_assumptions_should_show_at_most_one_decimal() {
    let body = get(&app(), &partial("years=1&mu=7.25&vol=33.333&growth=2.55"))
        .await
        .text();
    for bad in ["7.25%", "33.33", "2.55%"] {
        assert!(
            !body.contains(bad),
            "{bad} should be rounded to one decimal"
        );
    }
    assert!(body.contains("33.3%"));
}
