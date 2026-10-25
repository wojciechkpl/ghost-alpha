//! RED-phase contract for dashboard batch 2: Q6 (chart accessibility), Q10 (chart labels and
//! axis titles) and the Q5 carry-over (no hex colour literals in rendered HTML).
//!
//! Pinned decisions
//! * Every `<svg` in any page or partial (with seeded data) is either decorative
//!   (`aria-hidden="true"`; e.g. the in-cell score bar/sparkline that sits next to its numeric
//!   value) or has `role="img"`, an `aria-labelledby` naming the id of a `<title>` inside it,
//!   and a non-empty `<desc>`; the `<title>` is non-empty.
//! * SVG text sizes (`font-size`) are >= 11 (px / unitless).
//! * Chart routes that must offer a `<details>` whose `<summary>` contains "Table view" and
//!   which holds a `<table>` (with `<caption>`, `<th scope>`) with at least one body row, placed
//!   after the chart: see `TABLE_VIEW_ROUTES`.
//! * Axis titles are `<text class="axis-title">` elements. Per chart, see `AXIS_CHARTS`.
//! * The concentration-decay start label is "Start" (never "<year> Q0").
//! * No hex colour literal (`#rgb`, `#rrggbb`, `#rrggbbaa`) in inline `style`, `fill`, `stroke`,
//!   `stop-color`, ... of any rendered page/partial; colours are `var(--ga-...)`.

mod common;

use common::web::*;

/// Routes that render a chart and must carry a table-view fallback.
const TABLE_VIEW_ROUTES: &[&str] = &[
    "/concentration-decay",
    "/partials/regime-history",
    "/partials/macro-gauges",
    "/partials/score-heatmap",
    "/partials/sentiment-heatmap",
    "/partials/grade-dist",
    "/partials/rs-scatter",
    "/partials/alert-calendar",
];

fn svg_tag_attr(svg: &str, a: &str) -> Option<String> {
    let end = svg.find('>').unwrap() + 1;
    attr(&svg[..end], a)
}

#[tokio::test]
async fn test_svg_when_rendered_anywhere_should_be_decorative_or_labelled_with_title_and_desc() {
    let app = seeded_app();
    let mut charts = 0;
    for p in all_get_paths() {
        let html = get(&app, &p).await.text();
        for svg in svgs(&html) {
            if svg_tag_attr(&svg, "aria-hidden").as_deref() == Some("true") {
                continue;
            }
            charts += 1;
            assert_eq!(
                svg_tag_attr(&svg, "role").as_deref(),
                Some("img"),
                "{p}: {svg:.120}"
            );
            let lb = svg_tag_attr(&svg, "aria-labelledby")
                .unwrap_or_else(|| panic!("{p}: svg lacks aria-labelledby: {svg:.120}"));
            let title = all_tags(&svg)
                .into_iter()
                .find(|t| {
                    t.starts_with("<title")
                        && lb
                            .split_whitespace()
                            .any(|id| attr(t, "id").as_deref() == Some(id))
                })
                .unwrap_or_else(|| panic!("{p}: no <title id> matching aria-labelledby={lb}"));
            assert!(
                !element_text(&svg, &title).trim().is_empty(),
                "{p}: empty title"
            );
            let desc = all_tags(&svg)
                .into_iter()
                .find(|t| t.starts_with("<desc"))
                .unwrap_or_else(|| panic!("{p}: svg lacks <desc>"));
            assert!(
                !element_text(&svg, &desc).trim().is_empty(),
                "{p}: empty desc"
            );
        }
    }
    assert!(charts >= 8, "expected many labelled charts, saw {charts}");
}

#[tokio::test]
async fn test_svg_when_rendered_should_use_unique_title_ids_per_response() {
    let app = seeded_app();
    for p in all_get_paths() {
        let html = get(&app, &p).await.text();
        let ids: Vec<String> = all_tags(&html)
            .iter()
            .filter(|t| t.starts_with("<title"))
            .filter_map(|t| attr(t, "id"))
            .collect();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            ids.len(),
            "{p}: duplicate <title id>s {ids:?}"
        );
    }
}

#[tokio::test]
async fn test_svg_text_when_rendered_should_be_at_least_11px() {
    let app = seeded_app();
    let mut seen = 0;
    for p in all_get_paths() {
        let html = get(&app, &p).await.text();
        for svg in svgs(&html) {
            let mut rest = svg.as_str();
            while let Some(i) = rest.find("font-size") {
                let a = &rest[i + "font-size".len()..];
                let a = a.trim_start_matches(['=', ':', '"', '\'', ' ']);
                let num: String = a
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                let unit: String = a[num.len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphabetic() || *c == '%')
                    .collect();
                if let Ok(n) = num.parse::<f64>() {
                    if unit.is_empty() || unit == "px" {
                        seen += 1;
                        assert!(n >= 11.0, "{p}: svg font-size {n}{unit} < 11");
                    }
                }
                rest = a;
            }
        }
    }
    assert!(seen > 0, "no svg font sizes found; fixture too small?");
}

#[tokio::test]
async fn test_charts_when_listed_should_offer_a_table_view_details_with_numbers() {
    let app = seeded_app();
    for p in TABLE_VIEW_ROUTES {
        let r = get(&app, p).await;
        assert_eq!(r.status, 200, "{p}");
        let html = r.text();
        let first_svg = html
            .find("<svg")
            .unwrap_or_else(|| panic!("{p}: fixture renders no chart"));
        let mut found = false;
        let mut rest = html.as_str();
        while let Some(i) = rest.find("<details") {
            let a = &rest[i..];
            let close = a.find("</details>").unwrap_or(a.len());
            let det = &a[..close];
            let summary = all_tags(det)
                .into_iter()
                .find(|t| t.starts_with("<summary"))
                .map(|t| element_text(det, &t).to_lowercase())
                .unwrap_or_default();
            if summary.contains("table view") {
                found = true;
                assert!(
                    html.find(det).unwrap() > first_svg,
                    "{p}: table view must follow the chart"
                );
                assert!(
                    det.contains("<table") && det.contains("<caption") && det.contains("<td"),
                    "{p}: table view needs a captioned table with rows"
                );
                assert!(det.contains("scope=\"col\""), "{p}");
            }
            rest = &a[close.min(a.len())..];
            if close == a.len() {
                break;
            }
        }
        assert!(found, "{p}: no <details> with summary \"Table view\"");
    }
}

/// (route, min number of axis titles, substrings that must each appear in some axis title)
const AXIS_CHARTS: &[(&str, usize, &[&str])] = &[
    ("/concentration-decay", 2, &["(years)", "(%)"]),
    ("/partials/score-heatmap", 2, &["Date", "Ticker"]),
    ("/partials/sentiment-heatmap", 2, &["Date", "Ticker"]),
    ("/partials/grade-dist", 2, &["Date", "(stocks)"]),
    ("/partials/rs-scatter", 2, &["RS", "Momentum"]),
    ("/optimize", 2, &["Risk (%)", "Return (%)"]),
];

fn axis_titles(html: &str) -> Vec<String> {
    all_tags(html)
        .into_iter()
        .filter(|t| {
            t.starts_with("<text")
                && attr(t, "class").is_some_and(|c| c.split_whitespace().any(|w| w == "axis-title"))
        })
        .map(|t| element_text(html, &t))
        .collect()
}

#[tokio::test]
async fn test_charts_with_axes_should_have_axis_titles_with_units() {
    let app = seeded_app();
    for (p, min, wanted) in AXIS_CHARTS {
        let html = get(&app, p).await.text();
        let titles = axis_titles(&html);
        assert!(titles.len() >= *min, "{p}: axis titles {titles:?}");
        for w in *wanted {
            assert!(
                titles.iter().any(|t| t.contains(w)),
                "{p}: no axis title containing {w:?}; got {titles:?}"
            );
        }
    }
}

#[tokio::test]
async fn test_concentration_decay_when_rendered_should_label_first_point_start_not_q0() {
    let app = app();
    let html = get(&app, "/concentration-decay").await.text();
    assert!(!html.contains(" Q0"), "found a 'Q0' label");
    let texts: Vec<String> = all_tags(&html)
        .into_iter()
        .filter(|t| t.starts_with("<text"))
        .map(|t| element_text(&html, &t))
        .collect();
    assert!(
        texts.iter().any(|t| t == "Start"),
        "svg needs a 'Start' tick: {texts:?}"
    );
    assert!(
        html.contains("<td>Start</td>"),
        "table view first row must be 'Start'"
    );
}

// ───────────────────────── Q5 carry-over: no hex in rendered HTML ─────────────────────────

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

#[test]
fn test_hex_scanner_should_flag_attribute_and_style_hex_but_not_fragments() {
    assert_eq!(hex_literals(r##"<rect fill="#fff"/>"##), vec!["#fff"]);
    assert_eq!(
        hex_literals(r##"<p style="color:#888;x:1">"##),
        vec!["#888"]
    );
    assert_eq!(
        hex_literals(r##"<p style="color: #1a2b3c">"##),
        vec!["#1a2b3c"]
    );
    assert!(hex_literals(r##"<a href="#main-content">&#39; &#10003;"##).is_empty());
    assert!(hex_literals(r##"<a hx-target="#add"> url(#abc)"##).is_empty());
}

#[tokio::test]
async fn test_pages_and_partials_when_rendered_should_contain_no_hex_colours() {
    let app = seeded_app();
    let mut paths = all_get_paths();
    paths.extend(
        [
            "/concentration-decay?ticker=AMZN&shares=500&price=210&other_value=90000",
            "/geo-risk?oil=130&vix=32&gold_3m=0.15&dxy_3m=-0.05",
            "/valuation?ticker=AAPL",
            "/allocation?brokerage=schwab&profile=aggressive&portfolio_value=1000000",
        ]
        .map(String::from),
    );
    let mut bad = Vec::new();
    for p in paths {
        let html = get(&app, &p).await.text();
        let hits = hex_literals(&html);
        if !hits.is_empty() {
            let mut u = hits.clone();
            u.sort();
            u.dedup();
            bad.push(format!(
                "{p}: {} hex literal(s), e.g. {:?}",
                hits.len(),
                &u[..u.len().min(4)]
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "inline hex colours remain:\n{}",
        bad.join("\n")
    );
}

#[tokio::test]
async fn test_svg_colours_when_rendered_should_come_from_ga_tokens() {
    let app = seeded_app();
    let mut tokens = 0;
    for p in [
        "/partials/score-heatmap",
        "/partials/grade-dist",
        "/concentration-decay",
        "/partials/regime-history",
    ] {
        let html = get(&app, p).await.text();
        tokens += count(&html, "var(--ga-");
    }
    assert!(
        tokens >= 20,
        "charts should use var(--ga-*) colours, saw {tokens}"
    );
}

// ───────────────────────── round 2 ─────────────────────────

#[tokio::test]
async fn test_svg_when_labelled_should_also_describe_via_desc_id_and_aria_describedby() {
    let app = seeded_app();
    let mut n = 0;
    for p in all_get_paths() {
        let html = get(&app, &p).await.text();
        for svg in svgs(&html) {
            if svg_tag_attr(&svg, "role").as_deref() != Some("img") {
                continue;
            }
            n += 1;
            let desc = all_tags(&svg)
                .into_iter()
                .find(|t| t.starts_with("<desc"))
                .unwrap_or_else(|| panic!("{p}: no <desc>"));
            let id = attr(&desc, "id").unwrap_or_else(|| panic!("{p}: <desc> needs an id"));
            let by = svg_tag_attr(&svg, "aria-describedby")
                .unwrap_or_else(|| panic!("{p}: svg needs aria-describedby"));
            assert!(by.split_whitespace().any(|x| x == id), "{p}: {by} vs {id}");
        }
    }
    assert!(n >= 8);
}

/// A matrix with |r| on both sides of 0.5 (0.50 itself counts as strong).
fn corr_app() -> axum::Router {
    app_with(|r| {
        use ghost_alpha::storage::db::Repository;
        let m = vec![
            vec![1.0, 0.9, -0.7, 0.5],
            vec![0.9, 1.0, 0.3, -0.2],
            vec![-0.7, 0.3, 1.0, 0.49],
            vec![0.5, -0.2, 0.49, 1.0],
        ];
        let t: Vec<String> = ["AAA", "BBB", "CCC", "DDD"].map(String::from).to_vec();
        r.save_correlation_snapshot(&today().to_string(), &t, &m, 90)
            .unwrap();
    })
}

#[tokio::test]
async fn test_correlation_matrix_when_rendered_should_mark_strong_cell_text_with_on_strong_class() {
    let html = get(&corr_app(), "/correlation").await.text();
    let svg = svgs(&html)
        .into_iter()
        .find(|s| s.contains("0.90"))
        .expect("matrix svg");
    let (mut strong, mut weak) = (0, 0);
    for tag in all_tags(&svg)
        .into_iter()
        .filter(|t| t.starts_with("<text"))
    {
        let text = element_text(&svg, &tag);
        let Ok(v) = text.parse::<f64>() else { continue };
        if !text.contains('.') {
            continue;
        }
        let class = attr(&tag, "class").unwrap_or_default();
        let has = class.split_whitespace().any(|c| c == "on-strong");
        assert!(
            !tag.contains(" fill="),
            "cell text colour must come from CSS classes: {tag}"
        );
        if v.abs() >= 0.5 {
            strong += 1;
            assert!(has, "|{v}| >= 0.5 needs class on-strong: {tag}");
        } else {
            weak += 1;
            assert!(!has, "|{v}| < 0.5 must not use on-strong: {tag}");
        }
    }
    assert!(strong >= 8 && weak >= 4, "strong={strong} weak={weak}");
}

// ───────────────────────── heatmap label geometry ─────────────────────────

/// Estimated vertical extent (min_y, max_y) of a rotated label in SVG user units.
/// Text length ~ 0.6 * font-size per character along the rotated baseline; `text-anchor`
/// decides on which side of (x, y) the text lies; rotation is about the transform centre.
fn rotated_label_extent(tag: &str, text: &str) -> Option<(f64, f64)> {
    let tr = attr(tag, "transform")?;
    let inner = tr.strip_prefix("rotate(")?.strip_suffix(')')?;
    let nums: Vec<f64> = inner
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect();
    if nums.len() != 3 {
        return None;
    }
    let (ang, cx, cy) = (nums[0], nums[1], nums[2]);
    if !(-80.0..=-10.0).contains(&ang) {
        return None; // only diagonal labels (axis titles at -90 are out of scope)
    }
    let x: f64 = attr(tag, "x")?.parse().ok()?;
    let y: f64 = attr(tag, "y")?.parse().ok()?;
    let fs: f64 = attr(tag, "font-size")
        .and_then(|f| f.parse().ok())
        .unwrap_or(11.0);
    let len = text.chars().count() as f64 * 0.6 * fs;
    let (x0, x1) = match attr(tag, "text-anchor").as_deref() {
        Some("end") => (x - len, x),
        Some("middle") => (x - len / 2.0, x + len / 2.0),
        _ => (x, x + len),
    };
    let rad = ang.to_radians();
    let rot_y = |px: f64| cy + (px - cx) * rad.sin() + (y - cy) * rad.cos();
    let (a, b) = (rot_y(x0), rot_y(x1));
    Some((a.min(b), a.max(b)))
}

#[tokio::test]
async fn test_heatmaps_when_date_labels_rotated_should_sit_above_the_grid_and_inside_the_svg() {
    let app = seeded_app();
    let mut checked = 0;
    for p in [
        "/partials/score-heatmap",
        "/partials/sentiment-heatmap",
        "/correlation",
    ] {
        let html = get(&app, p).await.text();
        let svg = svgs(&html)
            .into_iter()
            .find(|s| s.contains("<rect"))
            .expect("svg");
        let width: f64 = svg_tag_attr(&svg, "width").unwrap().parse().unwrap();
        let grid_top = all_tags(&svg)
            .iter()
            .filter(|t| t.starts_with("<rect"))
            .filter(|t| {
                attr(t, "width")
                    .and_then(|w| w.parse::<f64>().ok())
                    .is_some_and(|w| w < width * 0.9)
            })
            .filter_map(|t| attr(t, "y")?.parse::<f64>().ok())
            .fold(f64::INFINITY, f64::min);
        for tag in all_tags(&svg)
            .into_iter()
            .filter(|t| t.starts_with("<text"))
        {
            let text = element_text(&svg, &tag);
            if let Some((lo, hi)) = rotated_label_extent(&tag, &text) {
                checked += 1;
                assert!(
                    hi <= grid_top + 0.5,
                    "{p}: rotated label {text:?} reaches y={hi:.1} below grid top {grid_top:.1}"
                );
                assert!(
                    lo >= 0.0,
                    "{p}: rotated label {text:?} is clipped above the svg (y={lo:.1})"
                );
            }
        }
    }
    assert!(checked >= 6, "expected rotated labels, checked {checked}");
}
