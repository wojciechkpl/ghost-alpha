//! RED-phase contract for dashboard quick wins Q1 (single shell + NAV),
//! Q2 (grouped, collapsible nav) and Q8 (landmarks / a11y).
//!
//! Public contract (module `ghost_alpha::web::shell`, new):
//!
//! ```ignore
//! pub struct NavItem  { pub href: &'static str, pub label: &'static str }
//! pub struct NavGroup { pub label: &'static str, pub items: &'static [NavItem] }
//! pub const NAV: &[NavGroup];
//! /// `active` is the route path of the current page (e.g. "/tax"); the item
//! /// whose href == active gets aria-current="page". Unknown => none.
//! /// `title` is HTML-escaped by the shell. The shell OWNS the single <h1>
//! /// (rendered from `title`); bodies passed in must not contain <h1>.
//! pub fn shell(active: &str, title: &str, body: &str) -> String;
//! ```
//!
//! `routes::layout()` and `dashboard_routes::dashboard_layout()` go away; every
//! full page is produced by `shell(...)`.

mod common;

use common::web::*;
use ghost_alpha::web::shell::{shell, NavGroup, NavItem, NAV};

/// Pinned route -> group mapping (group order is also pinned).
const EXPECTED: &[(&str, &[&str])] = &[
    (
        "Portfolio",
        &[
            "/portfolio",
            "/portfolio/concentration",
            "/allocation",
            "/concentration-decay",
            "/tax",
            "/selling",
            "/etf",
            "/correlation",
        ],
    ),
    ("Markets", &["/regime", "/geo-risk", "/rotation"]),
    (
        "Research",
        &[
            "/",
            "/watchlist",
            "/flagged",
            "/analytics",
            "/fundamentals",
            "/valuation",
        ],
    ),
    (
        "Tools",
        &["/sizing", "/optimize", "/strategies", "/decisions"],
    ),
    ("System", &["/alerts", "/settings"]),
];

fn all_items() -> Vec<&'static NavItem> {
    NAV.iter().flat_map(|g: &NavGroup| g.items.iter()).collect()
}

fn nav_section(html: &str) -> String {
    let s = html.find("<nav").expect("page has a <nav>");
    let e = html[s..].find("</nav>").expect("</nav>") + s;
    html[s..e].to_string()
}

// ───────────────────────── NAV data (Q1 + Q2) ─────────────────────────

#[test]
fn test_nav_when_inspected_should_have_five_groups_in_pinned_order_and_membership() {
    let got: Vec<(&str, Vec<&str>)> = NAV
        .iter()
        .map(|g| (g.label, g.items.iter().map(|i| i.href).collect()))
        .collect();
    assert_eq!(got.len(), EXPECTED.len());
    for (g, (label, hrefs)) in got.iter().zip(EXPECTED) {
        assert_eq!(g.0, *label);
        let mut a = g.1.clone();
        let mut b = hrefs.to_vec();
        a.sort();
        b.sort();
        assert_eq!(a, b, "membership of group {label}");
    }
}

#[test]
fn test_nav_when_inspected_should_list_each_href_and_label_exactly_once() {
    let items = all_items();
    let mut hrefs: Vec<_> = items.iter().map(|i| i.href).collect();
    let mut labels: Vec<_> = items.iter().map(|i| i.label).collect();
    assert!(items.iter().all(|i| !i.label.trim().is_empty()));
    hrefs.sort();
    labels.sort();
    let (h, l) = (hrefs.len(), labels.len());
    hrefs.dedup();
    labels.dedup();
    assert_eq!(hrefs.len(), h, "duplicate href in NAV");
    assert_eq!(labels.len(), l, "duplicate label in NAV");
}

#[test]
fn test_nav_when_compared_to_router_should_contain_every_page_route_and_nothing_else() {
    let pages = page_routes();
    assert!(pages.len() >= 22, "route parse found only {pages:?}");
    let mut nav: Vec<String> = all_items().iter().map(|i| i.href.to_string()).collect();
    let mut pages_sorted = pages.clone();
    nav.sort();
    pages_sorted.sort();
    assert_eq!(nav, pages_sorted, "NAV and router page routes must match");
    assert!(
        nav.contains(&"/concentration-decay".to_string()),
        "/concentration-decay must be reachable from NAV"
    );
}

// ───────────────────────── shell() unit contract ─────────────────────────

#[test]
fn test_shell_when_active_is_known_should_mark_only_that_link_aria_current_page() {
    let html = shell("/tax", "Tax", "<p>body</p>");
    assert_eq!(count(&html, "aria-current"), 1);
    let nav = nav_section(&html);
    let marked: Vec<_> = open_tags(&nav, "a")
        .into_iter()
        .filter(|t| t.contains("aria-current=\"page\""))
        .collect();
    assert_eq!(marked.len(), 1);
    assert_eq!(attr(&marked[0], "href").as_deref(), Some("/tax"));
}

#[test]
fn test_shell_when_active_is_unknown_should_mark_nothing() {
    let html = shell("/no-such-page", "X", "<p>b</p>");
    assert_eq!(count(&html, "aria-current"), 0);
}

#[test]
fn test_shell_when_active_is_home_should_mark_only_root_link() {
    let html = shell("/", "Home", "<p>b</p>");
    let nav = nav_section(&html);
    let marked: Vec<_> = open_tags(&nav, "a")
        .into_iter()
        .filter(|t| t.contains("aria-current=\"page\""))
        .collect();
    assert_eq!(marked.len(), 1);
    assert_eq!(attr(&marked[0], "href").as_deref(), Some("/"));
}

#[test]
fn test_shell_when_rendered_should_embed_body_inside_main_and_title_in_single_h1() {
    let html = shell("/tax", "Tax Overview", "<p id=\"probe\">hello</p>");
    assert_eq!(count(&html, "<h1"), 1);
    let h1s = html.find("<h1").unwrap();
    let h1e = html[h1s..].find("</h1>").unwrap() + h1s;
    assert!(html[h1s..h1e].contains("Tax Overview"));
    let ms = html.find("<main").unwrap();
    let me = html.find("</main>").unwrap();
    let probe = html.find("id=\"probe\"").unwrap();
    assert!(ms < h1s && h1s < me, "h1 belongs inside <main>");
    assert!(ms < probe && probe < me, "body belongs inside <main>");
    assert!(html.contains("<title>") && html.contains("Tax Overview"));
}

#[test]
fn test_shell_when_title_has_markup_should_escape_it() {
    let html = shell("/", "</title><script>alert(1)</script>", "<p>x</p>");
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(html.contains("&lt;script&gt;"));
}

#[test]
fn test_shell_when_rendered_should_have_skip_link_as_first_anchor_targeting_main() {
    let html = shell("/", "Home", "<p>b</p>");
    let first_a = open_tags(&html, "a").into_iter().next().expect("an <a>");
    assert_eq!(attr(&first_a, "href").as_deref(), Some("#main-content"));
    assert!(
        attr(&first_a, "class")
            .unwrap_or_default()
            .contains("skip-link"),
        "skip link needs class skip-link"
    );
    let main = open_tags(&html, "main");
    assert_eq!(main.len(), 1);
    assert_eq!(attr(&main[0], "id").as_deref(), Some("main-content"));
    // skip link precedes the header and nav
    assert!(html.find("href=\"#main-content\"").unwrap() < html.find("<header").unwrap());
}

#[test]
fn test_shell_when_rendered_should_render_all_nav_items_in_nav_order_grouped_with_labels() {
    let html = shell("/", "Home", "<p>b</p>");
    let nav = nav_section(&html);
    let mut cursor = 0usize;
    for g in NAV {
        let gpos = nav[cursor..]
            .find(g.label)
            .unwrap_or_else(|| panic!("group label {} missing/out of order", g.label))
            + cursor;
        cursor = gpos;
        for item in g.items {
            let needle = format!("href=\"{}\"", item.href);
            let p = nav[cursor..]
                .find(&needle)
                .unwrap_or_else(|| panic!("{} missing/out of order", item.href))
                + cursor;
            cursor = p;
        }
    }
    for item in all_items() {
        let needle = format!("href=\"{}\"", item.href);
        assert_eq!(count(&nav, &needle), 1, "{} appears once in nav", item.href);
    }
}

#[test]
fn test_shell_when_rendered_should_be_mobile_collapsible_without_js_via_details() {
    let html = shell("/", "Home", "<p>b</p>");
    let nav = nav_section(&html);
    assert!(nav.contains("<details"), "nav must use <details>");
    assert!(nav.contains("<summary"), "nav must have a <summary> toggle");
    let css = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/web/static/ghost-alpha.css"
    ))
    .unwrap();
    assert!(
        css.contains("@media") && css.contains("nav"),
        "ghost-alpha.css must contain responsive nav rules"
    );
}

#[test]
fn test_shell_when_rendered_should_have_header_then_labelled_nav_then_main_landmarks() {
    let html = shell("/", "Home", "<p>b</p>");
    assert_eq!(count(&html, "<header"), 1);
    assert_eq!(count(&html, "<nav"), 1);
    assert_eq!(count(&html, "<main"), 1);
    let nav_tag = open_tags(&html, "nav").remove(0);
    let label = attr(&nav_tag, "aria-label").expect("nav needs aria-label");
    assert!(!label.trim().is_empty());
    let (h, n, m) = (
        html.find("<header").unwrap(),
        html.find("<nav").unwrap(),
        html.find("<main").unwrap(),
    );
    assert!(h < n && n < m);
    assert!(html.contains("<html lang=\"en\""));
    assert!(html.contains("name=\"viewport\""));
}

#[test]
fn test_shell_when_rendered_should_link_assets_and_have_no_inline_script_or_style_block() {
    let html = shell("/", "Home", "<p>b</p>");
    let pico = html.find("/static/pico.min.css").expect("pico");
    let tokens = html.find("/static/tokens.css").expect("tokens.css linked");
    let ga = html
        .find("/static/ghost-alpha.css")
        .expect("ghost-alpha.css");
    assert!(
        pico < tokens && tokens < ga,
        "order: pico, tokens, ghost-alpha"
    );
    assert!(!html.contains("<style"), "no inline <style> in the shell");
    assert!(!html.contains("badge-critical"), "no inline badge CSS");
    for s in open_tags(&html, "script") {
        assert!(attr(&s, "src").is_some(), "inline script found: {s}");
    }
    let htmx = open_tags(&html, "script")
        .into_iter()
        .find(|s| s.contains("htmx.min.js"))
        .expect("htmx script");
    assert!(htmx.contains("defer"));
    assert!(!html.contains(" onclick=") && !html.contains(" onload="));
}

// ───────────────────────── every real page ─────────────────────────

#[tokio::test]
async fn test_pages_when_fetched_should_each_return_html_with_single_h1_main_and_skip_link() {
    let app = app();
    for item in all_items() {
        let r = get(&app, item.href).await;
        assert_eq!(r.status, 200, "{}", item.href);
        assert!(
            r.header("content-type").unwrap().starts_with("text/html"),
            "{}",
            item.href
        );
        let html = r.text();
        assert_eq!(count(&html, "<h1"), 1, "{} h1 count", item.href);
        assert_eq!(count(&html, "<main"), 1, "{} main count", item.href);
        assert_eq!(count(&html, "<header"), 1, "{} header count", item.href);
        assert_eq!(count(&html, "<nav"), 1, "{} nav count", item.href);
        let first_a = open_tags(&html, "a").into_iter().next().unwrap();
        assert_eq!(
            attr(&first_a, "href").as_deref(),
            Some("#main-content"),
            "{} skip link first",
            item.href
        );
        assert_eq!(
            attr(&open_tags(&html, "main")[0], "id").as_deref(),
            Some("main-content"),
            "{}",
            item.href
        );
    }
}

#[tokio::test]
async fn test_pages_when_fetched_should_mark_their_own_nav_item_aria_current_exactly_once() {
    let app = app();
    for item in all_items() {
        let html = get(&app, item.href).await.text();
        assert_eq!(count(&html, "aria-current"), 1, "{}", item.href);
        let nav = nav_section(&html);
        let marked: Vec<_> = open_tags(&nav, "a")
            .into_iter()
            .filter(|t| t.contains("aria-current=\"page\""))
            .collect();
        assert_eq!(marked.len(), 1, "{}", item.href);
        assert_eq!(
            attr(&marked[0], "href").as_deref(),
            Some(item.href),
            "wrong active item on {}",
            item.href
        );
    }
}

#[tokio::test]
async fn test_pages_when_fetched_should_share_one_nav_and_no_inline_style_blocks_or_scripts() {
    let app = app();
    let mut navs = std::collections::BTreeSet::new();
    for item in all_items() {
        let html = get(&app, item.href).await.text();
        // identical nav markup apart from the aria-current marker
        navs.insert(
            nav_section(&html)
                .replace(" aria-current=\"page\"", "")
                .replace("aria-current='page'", ""),
        );
        assert!(!html.contains("<style"), "{} has <style>", item.href);
        assert!(html.contains("/static/tokens.css"), "{}", item.href);
        for s in open_tags(&html, "script") {
            assert!(attr(&s, "src").is_some(), "{} inline script", item.href);
        }
        assert!(
            html.contains("<details") && html.contains("<summary"),
            "{}",
            item.href
        );
    }
    assert_eq!(navs.len(), 1, "all pages must render the same nav");
}

#[tokio::test]
async fn test_tables_when_rendered_should_have_caption_first_and_scoped_headers() {
    let app = app();
    let mut tables = 0;
    let mut paths: Vec<String> = all_items().iter().map(|i| i.href.to_string()).collect();
    paths.extend(partial_routes());
    for p in paths {
        let r = get(&app, &p).await;
        if r.status != 200 {
            continue;
        }
        let html = r.text();
        for t in html.match_indices("<table") {
            tables += 1;
            let after = &html[t.0..];
            let open_end = after.find('>').unwrap() + 1;
            assert!(
                after[open_end..].trim_start().starts_with("<caption"),
                "{p}: <table> must start with <caption>"
            );
        }
        for th in open_tags(&html, "th") {
            let scope = attr(&th, "scope");
            assert!(
                matches!(scope.as_deref(), Some("col") | Some("row")),
                "{p}: <th> without scope: {th}"
            );
        }
    }
    assert!(tables > 0, "expected at least one table across pages");
}

fn is_polled(tag: &str) -> bool {
    let trig = attr(tag, "hx-trigger").unwrap_or_default();
    trig.contains("every ") || trig.split([',', ' ']).next() == Some("load")
}

/// M1: polled / load-triggered containers must NOT be aria-live (they would re-announce
/// every poll). User-triggered swap targets are aria-live=polite, or, when the target is
/// itself polled, the page carries a dedicated status element (role=status or aria-live
/// on an element that does not itself fetch).
#[tokio::test]
async fn test_htmx_when_swap_targets_exist_should_announce_only_user_triggered_swaps() {
    let app = app();
    let (mut polled_seen, mut user_seen) = (0, 0);
    for item in all_items() {
        let html = get(&app, item.href).await.text();
        let tags = all_tags(&html);
        let has_status = tags.iter().any(|t| {
            !t.contains(" hx-get=")
                && (t.contains("role=\"status\"") || t.contains("aria-live=\"polite\""))
        });
        let find_id = |id: &str| {
            tags.iter()
                .find(|x| attr(x, "id").as_deref() == Some(id))
                .unwrap_or_else(|| panic!("{}: hx-target #{id} has no element", item.href))
                .clone()
        };
        for tag in &tags {
            if !(tag.contains(" hx-get=") || tag.contains(" hx-post=")) {
                continue;
            }
            let target = match attr(tag, "hx-target") {
                Some(t) if t.starts_with('#') => find_id(&t[1..]),
                Some(_) => continue,
                None => tag.clone(),
            };
            if is_polled(tag) || is_polled(&target) {
                polled_seen += 1;
                assert!(
                    !target.contains("aria-live"),
                    "{}: polled/load container must not be aria-live: {target}",
                    item.href
                );
                if !is_polled(tag) {
                    // user trigger writing into a polled container: needs a status element
                    user_seen += 1;
                    assert!(
                        has_status,
                        "{}: no role=status element for {tag}",
                        item.href
                    );
                }
            } else {
                user_seen += 1;
                assert!(
                    target.contains("aria-live=\"polite\"") || has_status,
                    "{}: user-triggered swap target not announced: {target}",
                    item.href
                );
            }
        }
    }
    assert!(
        polled_seen > 0 && user_seen > 0,
        "{polled_seen} {user_seen}"
    );
}

#[tokio::test]
async fn test_tables_when_rendered_should_not_use_role_grid() {
    let app = app();
    for item in all_items() {
        let html = get(&app, item.href).await.text();
        assert!(!html.contains("role=\"grid\""), "{}", item.href);
    }
}

#[tokio::test]
async fn test_captions_when_rendered_should_not_merely_repeat_the_page_h1() {
    let app = app();
    let mut seen = 0;
    for item in all_items() {
        let html = get(&app, item.href).await.text();
        let h1 = first_text(&html, "h1").unwrap().to_lowercase();
        let mut rest = html.as_str();
        while let Some(i) = rest.find("<caption") {
            seen += 1;
            let cap = first_text(&rest[i..], "caption").unwrap().to_lowercase();
            assert!(!cap.is_empty(), "{}: empty caption", item.href);
            assert_ne!(cap, h1, "{}: caption duplicates h1", item.href);
            rest = &rest[i + 8..];
        }
    }
    assert!(seen > 0);
}

#[tokio::test]
async fn test_headings_when_rendered_should_never_skip_a_level() {
    let app = app();
    for item in all_items() {
        let html = get(&app, item.href).await.text();
        let mut prev = 0u32;
        for t in all_tags(&html) {
            let b = t.as_bytes();
            if b.len() > 3 && b[1] == b'h' && (b'1'..=b'6').contains(&b[2]) && !t.starts_with("</")
            {
                let lvl = (b[2] - b'0') as u32;
                assert!(
                    lvl <= prev + 1,
                    "{}: heading jumps from h{prev} to h{lvl}",
                    item.href
                );
                prev = lvl;
            }
        }
        assert!(prev >= 1);
    }
}

#[tokio::test]
async fn test_home_when_polling_screen_table_should_include_the_date_input() {
    let app = app();
    let html = get(&app, "/").await.text();
    let tags = all_tags(&html);
    let date_input = tags
        .iter()
        .find(|t| t.starts_with("<input") && t.contains("name=\"date\""))
        .expect("date input");
    let id = attr(date_input, "id");
    let poller = tags
        .iter()
        .find(|t| t.contains("hx-get=\"/partials/screen-table\"") && t.contains("every "))
        .expect("polling tbody");
    let inc = attr(poller, "hx-include").expect("polling tbody needs hx-include");
    let refs_name = inc.contains("[name=date]") || inc.contains("[name=\"date\"]");
    let refs_id = id.as_ref().is_some_and(|i| inc.contains(&format!("#{i}")));
    assert!(
        refs_name || refs_id,
        "hx-include={inc} does not reference the date input"
    );
}

#[tokio::test]
async fn test_nav_js_when_shell_rendered_should_be_versioned_deferred_and_details_open_by_default()
{
    let app = app();
    let html = get(&app, "/").await.text();
    let script = open_tags(&html, "script")
        .into_iter()
        .find(|s| s.contains("/static/nav.js"))
        .expect("nav.js script tag");
    // batch 2: nav.js is loaded synchronously after the menu (see web_batch2.rs)
    assert!(!script.contains("defer"));
    assert!(attr(&script, "src")
        .unwrap()
        .starts_with("/static/nav.js?v="));
    // no-JS fallback: menu renders open
    let details = open_tags(&nav_section(&html), "details").remove(0);
    assert!(
        details
            .split_whitespace()
            .any(|w| w == "open" || w == "open>"),
        "nav <details> must render with `open`: {details}"
    );
    let js = get(&app, "/static/nav.js").await;
    assert_eq!(js.status, 200);
    assert!(js.header("content-type").unwrap().contains("javascript"));
    let body = js.text();
    assert!(body.contains("matchMedia") && body.contains("768"));
    assert!(
        body.contains("removeAttribute(\"open\")")
            || body.contains("removeAttribute('open')")
            || body.contains(".open = false")
            || body.contains(".open=false"),
        "nav.js must close the details on small screens"
    );
}

#[test]
fn test_css_when_loaded_should_define_focus_visible_style() {
    let css = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/web/static/ghost-alpha.css"
    ))
    .unwrap();
    assert!(css.contains(":focus-visible"));
    // skip link must exist as a styled class so it can be hidden until focused
    assert!(css.contains(".skip-link"));
}
