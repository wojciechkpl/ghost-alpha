//! RED-phase contract for dashboard batch 2: Q3 (illustrative banner), status-region script,
//! Q7 (sortable/filterable tables + ticker links), and the nav.js loading change.
//!
//! Pinned decisions
//! * Demo-backed pages (hard-coded inputs, not the user's data): `/rotation`, `/sizing`,
//!   `/optimize` (always), `/concentration-decay` unless the query supplies BOTH `shares` and
//!   `price`. `/allocation` and `/geo-risk` are interactive calculators with query-driven
//!   defaults and do NOT show the banner. No other page shows it.
//! * Banner: `<div|aside|section class="banner banner-illustrative" role="note">` inside `<main>`,
//!   after the `<h1>` and before the first `<svg`/`<table`, text contains "Illustrative", and it
//!   holds a link to `/portfolio` (import instructions live there) or `/settings`.
//! * Status script: `/static/status.js` (versioned, deferred, on every page).
//! * Tables: `/static/table.js` (versioned, deferred, on every page). Sortable header:
//!   `<th scope="col" data-sort="text|number"><button type="button">Label</button></th>`.
//!   Filter: `<input type="search" data-filter-for="<table id>" aria-label="...">`.
//!   Applies to the screen table (/), watchlist table (/watchlist), alerts table (/alerts).
//! * Ticker cells link to `/valuation?ticker=SYM`.
//! * nav.js: not deferred/async, placed after the nav `</details>` inside `<body>`.

mod common;

use common::web::*;

fn script_tag(html: &str, name: &str) -> String {
    open_tags(html, "script")
        .into_iter()
        .find(|s| s.contains(&format!("/static/{name}")))
        .unwrap_or_else(|| panic!("no <script> for {name}"))
}

fn main_section(html: &str) -> &str {
    let s = html.find("<main").unwrap();
    let e = html.find("</main>").unwrap();
    &html[s..e]
}

fn banner(html: &str) -> Option<(String, String)> {
    let tag = all_tags(html).into_iter().find(|t| {
        attr(t, "class")
            .map(|c| c.split_whitespace().any(|w| w == "banner-illustrative"))
            .unwrap_or(false)
    })?;
    let start = html.find(&tag).unwrap();
    let name: String = tag[1..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    let end = html[start..].find(&format!("</{name}>")).unwrap() + start;
    Some((tag, html[start..end].to_string()))
}

// ───────────────────────── Q3 ─────────────────────────

const DEMO_PAGES: &[&str] = &["/rotation", "/sizing", "/optimize", "/concentration-decay"];

#[tokio::test]
async fn test_banner_when_demo_backed_page_should_show_illustrative_note_with_link() {
    for app in [app(), seeded_app()] {
        for p in DEMO_PAGES {
            let html = get(&app, p).await.text();
            let (tag, inner) = banner(&html).unwrap_or_else(|| panic!("{p}: no banner"));
            let class = attr(&tag, "class").unwrap();
            assert!(class.split_whitespace().any(|w| w == "banner"), "{p}");
            assert_eq!(attr(&tag, "role").as_deref(), Some("note"), "{p}");
            assert!(strip_tags(&inner).contains("Illustrative"), "{p}");
            let hrefs: Vec<_> = open_tags(&inner, "a")
                .iter()
                .filter_map(|a| attr(a, "href"))
                .collect();
            assert!(
                hrefs.iter().any(|h| h == "/portfolio" || h == "/settings"),
                "{p}: banner needs a link to /portfolio or /settings, got {hrefs:?}"
            );
            // placement: in <main>, after the h1, before any chart/table
            let m = main_section(&html);
            let b = m.find("banner-illustrative").expect("banner in main");
            assert!(b > m.find("</h1>").unwrap(), "{p}: banner after h1");
            for later in ["<svg", "<table"] {
                if let Some(i) = m.find(later) {
                    assert!(b < i, "{p}: banner must precede {later}");
                }
            }
            assert_eq!(count(&html, "banner-illustrative"), 1, "{p}");
        }
    }
}

#[tokio::test]
async fn test_banner_when_concentration_decay_has_user_inputs_should_not_show() {
    let app = app();
    let html = get(
        &app,
        "/concentration-decay?ticker=AMZN&shares=500&price=210&other_value=90000",
    )
    .await
    .text();
    assert!(banner(&html).is_none());
}

#[tokio::test]
async fn test_banner_when_page_uses_real_data_should_not_show() {
    for app in [app(), seeded_app()] {
        for p in page_routes() {
            if DEMO_PAGES.contains(&p.as_str()) {
                continue;
            }
            let html = get(&app, &p).await.text();
            assert!(banner(&html).is_none(), "{p} must not show the banner");
            assert!(!html.contains("Illustrative inputs"), "{p}");
        }
    }
}

#[test]
fn test_css_when_loaded_should_style_banner() {
    let css = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/web/static/ghost-alpha.css"
    ))
    .unwrap();
    assert!(css.contains(".banner-illustrative"));
}

// ───────────────────────── status regions ─────────────────────────

#[tokio::test]
async fn test_status_script_when_page_rendered_should_be_loaded_versioned_and_deferred() {
    let app = app();
    for p in ["/", "/analytics", "/regime"] {
        let html = get(&app, p).await.text();
        let s = script_tag(&html, "status.js");
        assert!(s.contains("defer"), "{p}");
        assert!(attr(&s, "src").unwrap().contains("?v="), "{p}");
    }
}

#[tokio::test]
async fn test_status_script_when_fetched_should_write_updated_text_on_after_swap() {
    let app = app();
    let r = get(&app, "/static/status.js").await;
    assert_eq!(r.status, 200);
    assert!(r.header("content-type").unwrap().contains("javascript"));
    let js = r.text();
    assert!(js.contains("htmx:afterSwap"));
    assert!(
        js.contains("[role=status]") || js.contains("[role=\"status\"]"),
        "must target [role=status]"
    );
    assert!(
        js.contains("textContent"),
        "write via textContent, not innerHTML"
    );
    assert!(js.contains("Updated"));
    assert!(!js.contains("eval(") && !js.contains("new Function"));
}

#[tokio::test]
async fn test_status_regions_when_home_and_analytics_rendered_should_exist_and_not_fetch() {
    let app = app();
    for p in ["/", "/analytics"] {
        let html = get(&app, p).await.text();
        let status: Vec<_> = all_tags(&html)
            .into_iter()
            .filter(|t| t.contains("role=\"status\""))
            .collect();
        assert!(!status.is_empty(), "{p}: needs a role=status region");
        assert!(status.iter().all(|t| !t.contains("hx-get")), "{p}");
    }
}

// ───────────────────────── nav.js loading ─────────────────────────

#[tokio::test]
async fn test_nav_js_when_shell_rendered_should_load_synchronously_right_after_the_menu() {
    let app = app();
    for p in ["/", "/tax"] {
        let html = get(&app, p).await.text();
        let tag = script_tag(&html, "nav.js");
        assert!(
            !tag.contains("defer") && !tag.contains("async"),
            "{p}: {tag}"
        );
        let body = html.find("<body").unwrap();
        let details_end = html.find("</details>").unwrap();
        let at = html.find(&tag).unwrap();
        assert!(at > body, "{p}: nav.js must not be in <head>");
        assert!(
            at > details_end,
            "{p}: nav.js must follow the nav <details>"
        );
        // still versioned
        assert!(attr(&tag, "src").unwrap().contains("?v="));
        // other scripts stay deferred
        assert!(script_tag(&html, "htmx.min.js").contains("defer"));
    }
}

// ───────────────────────── Q7 tables ─────────────────────────

#[tokio::test]
async fn test_table_script_when_page_rendered_should_be_loaded_versioned_and_deferred() {
    let app = app();
    let html = get(&app, "/").await.text();
    let s = script_tag(&html, "table.js");
    assert!(s.contains("defer"));
    assert!(attr(&s, "src").unwrap().contains("?v="));
}

#[tokio::test]
async fn test_table_script_when_fetched_should_implement_sort_and_filter_hooks() {
    let app = app();
    let r = get(&app, "/static/table.js").await;
    assert_eq!(r.status, 200);
    assert!(r.header("content-type").unwrap().contains("javascript"));
    let js = r.text();
    for hook in [
        "data-sort",
        "aria-sort",
        "ascending",
        "descending",
        "data-filter-for",
        "addEventListener",
        "htmx:afterSwap",
    ] {
        assert!(js.contains(hook), "table.js lacks `{hook}`");
    }
    assert!(!js.contains("eval(") && !js.contains("new Function"));
}

fn th_with_sort(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("<th") {
        let a = &rest[i..];
        let tag_end = a.find('>').unwrap() + 1;
        let tag = a[..tag_end].to_string();
        let close = a.find("</th>").unwrap_or(a.len());
        if tag.contains("data-sort=") {
            out.push((tag, a[tag_end..close].to_string()));
        }
        rest = &a[tag_end..];
    }
    out
}

#[tokio::test]
async fn test_tables_when_main_lists_rendered_should_have_button_headers_and_a_filter_input() {
    let app = seeded_app();
    for p in ["/", "/watchlist", "/alerts"] {
        let html = get(&app, p).await.text();
        let tables: Vec<String> = open_tags(&html, "table")
            .iter()
            .filter_map(|t| attr(t, "id"))
            .collect();
        assert!(!tables.is_empty(), "{p}: main table needs an id");
        let inputs = open_tags(&html, "input");
        for id in &tables {
            let f = inputs
                .iter()
                .find(|i| attr(i, "data-filter-for").as_deref() == Some(id))
                .unwrap_or_else(|| panic!("{p}: no filter input for #{id}"));
            assert_eq!(attr(f, "type").as_deref(), Some("search"), "{p}");
            assert!(
                !attr(f, "aria-label").unwrap_or_default().trim().is_empty(),
                "{p}: filter needs an aria-label"
            );
        }
        let sorts = th_with_sort(&html);
        assert!(sorts.len() >= 2, "{p}: sortable headers: {sorts:?}");
        for (tag, inner) in &sorts {
            assert_eq!(attr(tag, "scope").as_deref(), Some("col"), "{p}: {tag}");
            let v = attr(tag, "data-sort").unwrap();
            assert!(v == "text" || v == "number", "{p}: data-sort={v}");
            let b = open_tags(inner, "button");
            assert_eq!(b.len(), 1, "{p}: sortable th needs one <button>: {inner}");
            assert_eq!(attr(&b[0], "type").as_deref(), Some("button"));
        }
    }
}

#[tokio::test]
async fn test_tables_when_home_rendered_should_mark_score_columns_numeric_and_ticker_text() {
    let app = seeded_app();
    let html = get(&app, "/").await.text();
    let sorts = th_with_sort(&html);
    let kinds = |label: &str| {
        sorts
            .iter()
            .find(|(_, i)| strip_tags(i).contains(label))
            .and_then(|(t, _)| attr(t, "data-sort"))
    };
    assert_eq!(kinds("Ticker").as_deref(), Some("text"));
    assert_eq!(kinds("Score").as_deref(), Some("number"));
}

#[tokio::test]
async fn test_tickers_when_listed_should_link_to_valuation_page() {
    let app = seeded_app();
    let needle = "href=\"/valuation?ticker=AAPL\"";
    for p in [
        "/",
        "/partials/screen-table",
        "/watchlist",
        "/partials/watchlist-rows",
        "/alerts",
        "/partials/alert-list",
    ] {
        let html = get(&app, p).await.text();
        assert!(
            html.contains(needle),
            "{p}: ticker cell must link to /valuation"
        );
    }
    assert_eq!(
        get(&app, "/valuation?ticker=AAPL").await.status,
        200,
        "link target must exist"
    );
}

// ───────────────────────── round 2: alerts escaping, severity sorting, numeric hooks ─────────────────────────

#[tokio::test]
async fn test_alert_rows_when_stored_fields_contain_markup_should_be_escaped() {
    let app = app_with(|r| {
        let conn = r.connection().unwrap();
        conn.execute(
            "INSERT INTO alert_log (ticker, alert_type, severity, message, sent_at) \
             VALUES ('ZZ', '<i>kind</i>', '<img src=x onerror=alert(1)>', '<u>msg</u>', \
                     '<script>alert(2)</script>')",
            [],
        )
        .unwrap();
    });
    for p in ["/alerts", "/partials/alert-list"] {
        let html = get(&app, p).await.text();
        for raw in ["<img src=x", "<script>alert(2)", "<i>kind", "<u>msg"] {
            assert!(!html.contains(raw), "{p}: unescaped `{raw}`");
        }
        assert!(
            html.contains("&lt;img src=x onerror=alert(1)&gt;"),
            "{p}: severity"
        );
        assert!(
            html.contains("&lt;script&gt;alert(2)&lt;/script&gt;"),
            "{p}: sent_at"
        );
    }
}

#[tokio::test]
async fn test_alert_severity_cells_when_rendered_should_carry_ordinal_sort_values() {
    let app = seeded_app();
    for p in ["/alerts", "/partials/alert-list"] {
        let html = get(&app, p).await.text();
        let mut seen = std::collections::BTreeMap::new();
        for td in all_tags(&html)
            .into_iter()
            .filter(|t| t.starts_with("<td") && t.contains("data-sort-value"))
        {
            let text = element_text(&html, &td);
            seen.insert(text.to_uppercase(), attr(&td, "data-sort-value").unwrap());
        }
        assert_eq!(
            seen.get("CRITICAL").map(String::as_str),
            Some("3"),
            "{p}: {seen:?}"
        );
        assert_eq!(
            seen.get("WARNING").map(String::as_str),
            Some("2"),
            "{p}: {seen:?}"
        );
        assert_eq!(
            seen.get("INFO").map(String::as_str),
            Some("1"),
            "{p}: {seen:?}"
        );
    }
    let html = get(&app, "/alerts").await.text();
    let sev = th_with_sort(&html)
        .into_iter()
        .find(|(_, i)| strip_tags(i).contains("Severity"))
        .expect("Severity header is sortable");
    assert_eq!(attr(&sev.0, "data-sort").as_deref(), Some("number"));
}

#[tokio::test]
async fn test_table_script_when_fetched_should_support_sort_values_and_rich_numbers() {
    // Behavioural JS tests need node (unavailable/unreliable here), so pin the hooks.
    let js = get(&app(), "/static/table.js").await.text();
    assert!(
        js.contains("data-sort-value"),
        "cells may carry data-sort-value"
    );
    assert!(
        js.contains("\\u2212") || js.contains('\u{2212}'),
        "U+2212 minus must be normalised"
    );
    assert!(js.contains("KMBT"), "magnitude suffixes K/M/B/T");
    assert!(
        js.contains("\\d[\\d,]*") || js.contains("[\\d,]"),
        "first numeric token with thousands separators"
    );
}
